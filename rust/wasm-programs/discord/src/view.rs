//! The Discord window, in Discord's colours, at the three size classes.
//!
//! * **compact** (phones, < 400 lu): one page at a time. Servers: the server
//!   rail (the Discord logo, one button per server, `+`) beside the open
//!   server's channels, the voice panel and your name. Chat: `<`, the channel,
//!   Members; the messages; the composer. Plus the Add / Create / Join / New
//!   channel / Members pages.
//! * **medium**: the rail, the channel list and the open channel side by side.
//! * **wide** (>= 800 lu): the same plus the member list on the right.
//!
//! Text channels show as `# name`; voice channels as a speaker and their name,
//! with the people in them underneath (green while they talk).

use crate::model::{Link, Model, Page};
use ecm_discord::proto::{Kind, Presence};
use ecm_ui::element::{button, column, image, label, panel, row, scroll_view, spacer, text_field, Element};
use ecm_ui::{Align, ViewCtx};
use ecm_ui_proto::SizeClass;
use std::cell::RefCell;
use std::rc::Rc;

pub type M = Rc<RefCell<Model>>;

/// Discord's dark theme.
pub const RAIL: u32 = 0x1e1f_22ff;
pub const SIDEBAR: u32 = 0x2b2d_31ff;
pub const MAIN: u32 = 0x3133_38ff;
pub const PANEL: u32 = 0x232428ff;
pub const BLURPLE: u32 = 0x5865_f2ff;
pub const GREEN: u32 = 0x23a5_5aff;
pub const TEXT: u32 = 0xdbde_e1ff;
pub const MUTED: u32 = 0x949b_a4ff;
pub const RED: u32 = 0xf23f_43ff;
pub const MENTION: u32 = 0xc9cd_fbff;

/// Body line, caption line, title line and button heights (default theme).
const LINE: i32 = 15;
const CAPTION: i32 = 11;
const TITLE: i32 = 20;
const BUTTON: i32 = LINE + 8;
const RAIL_W: i32 = 44;

pub fn view(m: &M, cx: &ViewCtx) -> Element {
    let st = m.borrow();
    let body = match cx.class {
        SizeClass::Compact => compact(m, &st, cx),
        SizeClass::Medium => side_by_side(m, &st, cx, false),
        SizeClass::Wide => side_by_side(m, &st, cx, true),
    };
    panel().fill(MAIN).child(body)
}

/// Up to two letters for a server's icon: "Tech Village" -> "TV".
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let s: String = if words.len() >= 2 {
        words.iter().take(2).filter_map(|w| w.chars().next()).collect()
    } else {
        name.chars().take(2).collect()
    };
    s.to_uppercase()
}

fn caption(text: impl Into<String>) -> Element {
    label(text).style("caption").color(MUTED).wrap(false)
}

fn heading(text: impl Into<String>) -> Element {
    label(text).style("title").color(TEXT).wrap(false)
}

// ------------------------------------------------------------------ the rail

fn rail(m: &M, st: &Model) -> Element {
    let mut c = column().gap(6).width(RAIL_W).padding(6);
    if st.logo != 0 {
        let home = m.clone();
        c = c.child(image(st.logo, 32, 32).id("logo").on_click(move |_, _| home.borrow_mut().back()));
    }
    for (i, (r, g)) in st.rail().into_iter().enumerate() {
        let p = m.clone();
        let open = st.guild == Some(r);
        let unread = st.backends[r.0].channels.values().any(|c| c.guild == r.1 && c.unread > 0);
        let text = if unread && !open { format!("{}*", initials(&g.name)) } else { initials(&g.name) };
        let mut b = button(text).id(format!("guild:{i}")).width(36).on_click(move |_, _| p.borrow_mut().open_guild(r));
        if open {
            b = b.style("primary");
        }
        c = c.child(b);
    }
    let add = m.clone();
    c.child(button("+").id("add").width(36).on_click(move |_, _| {
        let mut s = add.borrow_mut();
        s.page = Page::Add;
        s.notice = None;
    }))
}

// ------------------------------------------------------------------ the channel list

fn channel_list(m: &M, st: &Model, cx: &ViewCtx, h: i32) -> Element {
    let Some((b, g)) = st.current() else {
        let mut c = column().gap(8).padding(8).grow(1).child(heading("Discord"));
        c = c.child(label(no_server_text(st)).color(MUTED));
        return c;
    };
    let (bi, gid) = st.guild.unwrap();
    let owner = st.owns_current();
    let mut c = column().gap(4);
    c = c.child(heading(g.name.clone()));
    c = c.child(caption(format!("{} · {} member{}", b.name, g.members, if g.members == 1 { "" } else { "s" })));
    let mut text_head = row().gap(4).child(caption("TEXT CHANNELS")).child(spacer());
    let mut voice_head = row().gap(4).child(caption("VOICE CHANNELS")).child(spacer());
    if owner {
        let a = m.clone();
        text_head = text_head.child(button("+ Channel").id("new_text").style("flat").on_click(move |_, _| {
            let mut s = a.borrow_mut();
            s.new_channel_kind = Kind::Text;
            s.page = Page::NewChannel;
        }));
        let a = m.clone();
        voice_head = voice_head.child(button("+ Voice").id("new_voice").style("flat").on_click(move |_, _| {
            let mut s = a.borrow_mut();
            s.new_channel_kind = Kind::Voice;
            s.page = Page::NewChannel;
        }));
    }
    c = c.child(text_head);
    for (cid, ch) in b.channels_of(gid, Kind::Text) {
        let p = m.clone();
        let text = if ch.unread > 0 { format!("# {} ({})", ch.name, ch.unread) } else { format!("# {}", ch.name) };
        let mut btn = button(text).id(format!("chan:{cid}")).style("flat").grow(1).on_click(move |_, _| p.borrow_mut().open_channel(cid));
        if st.channel == Some(cid) && st.page != Page::Servers || st.channel == Some(cid) && cx.class != SizeClass::Compact {
            btn = btn.style("primary");
        }
        c = c.child(row().child(btn));
    }
    c = c.child(voice_head);
    for (cid, ch) in b.channels_of(gid, Kind::Voice) {
        let p = m.clone();
        let here = st.voice.as_ref().is_some_and(|v| v.backend == bi && v.channel == cid);
        let mut btn = button(ch.name.clone()).id(format!("voice:{cid}")).style(if here { "primary" } else { "flat" }).grow(1).on_click(move |_, _| {
            let r = p.borrow_mut().join_voice(cid);
            if let Err(e) = r {
                p.borrow_mut().notice = Some(e);
            }
        });
        if st.voice_wanted == Some((bi, cid)) {
            btn = btn.enabled(false);
        }
        let mut r = row().gap(4).align(Align::Center);
        if st.speaker != 0 {
            r = r.child(image(st.speaker, 10, 10));
        }
        c = c.child(r.child(btn));
        for (_, v) in b.in_voice(cid) {
            let me = v.nick.eq_ignore_ascii_case(&b.me);
            let speaking = v.speaking || me && st.talking;
            let name = if me && st.muted { format!("{} (muted)", v.nick) } else { v.nick.clone() };
            c = c.child(row().child(spacer().width(16)).child(label(name).color(if speaking { GREEN } else { MUTED }).wrap(false)));
        }
    }
    if let Some(n) = &st.notice {
        if cx.class == SizeClass::Compact {
            c = c.child(label(n.clone()).color(RED).id("notice_side"));
        }
    }
    let leave = m.clone();
    let leave_text = if owner { "Delete server" } else { "Leave server" };
    if !(owner && gid == ecm_discord::server::HOME) {
        c = c.child(row().child(button(leave_text).id("leave").style("flat").on_click(move |_, _| {
            let r = leave.borrow_mut().leave_server();
            if let Err(e) = r {
                leave.borrow_mut().notice = Some(e);
            }
        })));
    }
    let list_h = h - user_area_h(st);
    column().gap(6).grow(1).child(scroll_view(c).id("channels").height(list_h.max(40))).child(user_area(m, st))
}

fn no_server_text(st: &Model) -> String {
    if st.backends.is_empty() {
        if st.searching {
            "Looking for Discord backends on the network ...".into()
        } else {
            "No Discord backend found yet. Tap + to search again or add one by address.".into()
        }
    } else if st.backends.iter().any(|b| matches!(b.link, Link::Idle | Link::Connecting)) {
        "Connecting ...".into()
    } else if st.rail().is_empty() && st.backends.iter().any(|b| b.up()) {
        "You haven't joined a server yet. Tap + to make one or join one.".into()
    } else {
        "Can't reach a Discord backend. Tap + to see why.".into()
    }
}

fn user_area_h(st: &Model) -> i32 {
    let voice = if st.voice.is_some() || st.voice_wanted.is_some() { LINE + CAPTION + BUTTON + 12 } else { 0 };
    voice + 2 * BUTTON + 16
}

/// The voice panel (while in a voice channel) and your name with Mute / Deafen.
fn user_area(m: &M, st: &Model) -> Element {
    let mut c = column().gap(4);
    if let Some((guild, channel)) = st.voice_place() {
        let hang = m.clone();
        let mut v = column().gap(2);
        v = v.child(label("Voice Connected").color(GREEN).wrap(false).id("voice_status"));
        v = v.child(caption(format!("{channel} / {guild}")));
        let route = if st.headphones { "Headphones" } else { "Speaker" };
        v = v.child(row().gap(4).child(caption(route)).child(spacer()).child(button("Disconnect").id("hangup").style("danger").on_click(move |_, _| hang.borrow_mut().leave_voice())));
        c = c.child(panel().fill(PANEL).radius(4).padding(4).child(v));
    } else if st.voice_wanted.is_some() {
        c = c.child(panel().fill(PANEL).radius(4).padding(4).child(label("Connecting to voice ...").color(MUTED)));
    }
    let mute = m.clone();
    let deaf = m.clone();
    let gear = m.clone();
    let me = if st.nick.is_empty() { "you".to_string() } else { st.nick.clone() };
    c.child(
        // Your name and Settings, then Mute / Deafen under them (one row is too wide for the sidebar).
        panel().fill(PANEL).radius(4).padding(4).child(
            column()
                .gap(4)
                .child(
                    row()
                        .gap(4)
                        .align(Align::Center)
                        .child(label(me).color(TEXT).wrap(false).id("me"))
                        .child(spacer())
                        .child(button("Settings").id("settings").style("flat").on_click(move |_, _| gear.borrow_mut().open_settings())),
                )
                .child(
                    row()
                        .gap(4)
                        .child(button(if st.muted { "Unmute" } else { "Mute" }).id("mute").style(if st.muted { "danger" } else { "flat" }).on_click(move |_, _| {
                            let mut s = mute.borrow_mut();
                            s.muted = !s.muted;
                        }))
                        .child(button(if st.deafened { "Undeafen" } else { "Deafen" }).id("deafen").style(if st.deafened { "danger" } else { "flat" }).on_click(
                            move |_, _| {
                                let mut s = deaf.borrow_mut();
                                s.deafened = !s.deafened;
                            },
                        )),
                ),
        ),
    )
}

// ------------------------------------------------------------------ messages

fn send_now(m: &M, cx: &mut ecm_ui::Ctx) {
    let text = cx.take_text("compose");
    m.borrow_mut().compose(&text);
}

fn composer(m: &M, st: &Model) -> Element {
    let a = m.clone();
    let b = m.clone();
    let place = match st.current_channel() {
        Some(c) => format!("Message #{}", c.name),
        None => "No channel open".into(),
    };
    let ok = st.current_channel().is_some() && st.current().is_some_and(|(b, _)| b.up());
    row()
        .gap(6)
        .child(text_field().id("compose").placeholder(place).grow(1).on_submit(move |cx, _| send_now(&a, cx)))
        .child(button("Send").style("primary").id("send").enabled(ok).on_click(move |cx, _| send_now(&b, cx)))
}

fn messages(cx: &ViewCtx, st: &Model, h: i32) -> Element {
    let mut c = column().gap(8);
    match st.current_channel() {
        None => c = c.child(label("Pick a text channel.").color(MUTED)),
        Some(ch) => {
            if ch.messages.is_empty() {
                c = c.child(heading(format!("Welcome to #{}!", ch.name))).child(label(format!("This is the start of the #{} channel.", ch.name)).color(MUTED));
            }
            let me = st.current().map(|(b, _)| b.me.clone()).unwrap_or_default();
            let mut last: Option<&str> = None;
            for msg in &ch.messages {
                // Discord groups a run of messages by one person under one name.
                if last != Some(msg.nick.as_str()) {
                    let when = if msg.earlier { format!("{} · earlier", msg.time) } else { msg.time.clone() };
                    c = c.child(row().gap(6).child(label(msg.nick.clone()).color(if msg.mine { BLURPLE } else { TEXT }).wrap(false)).child(caption(when)));
                }
                last = Some(msg.nick.as_str());
                let mentioned = !me.is_empty() && msg.text.contains(&format!("@{me}"));
                c = c.child(label(msg.text.clone()).color(if mentioned { MENTION } else { TEXT }));
            }
        }
    }
    let _ = cx;
    scroll_view(c).id("history").height(h.max(40))
}

fn chat_header(m: &M, st: &Model, compact: bool) -> Element {
    let mut r = row().gap(6).align(Align::Center);
    if compact {
        let back = m.clone();
        r = r.child(button("<").id("back").style("flat").on_click(move |_, _| back.borrow_mut().back()));
    }
    let title = st.current_channel().map(|c| format!("# {}", c.name)).unwrap_or_else(|| "Discord".into());
    r = r.child(heading(title)).child(spacer());
    if compact && st.current().is_some() {
        let p = m.clone();
        r = r.child(button("Members").id("members").style("flat").on_click(move |_, _| p.borrow_mut().page = Page::Members));
    }
    r
}

fn chat(m: &M, st: &Model, cx: &ViewCtx, h: i32, compact: bool) -> Element {
    let mut c = column().gap(8).grow(1).child(chat_header(m, st, compact));
    let mut used = TITLE + 8 + BUTTON + 8;
    if let Some(n) = &st.notice {
        c = c.child(label(n.clone()).color(RED).id("notice"));
        used += LINE + 8;
    }
    c.child(messages(cx, st, h - used)).child(composer(m, st))
}

fn members(st: &Model) -> Element {
    let mut c = column().gap(4);
    let Some((b, _)) = st.current() else { return c };
    let (_, gid) = st.guild.unwrap();
    let list = b.members.get(&gid).cloned().unwrap_or_default();
    let online: Vec<&(String, Presence)> = list.iter().filter(|(_, p)| *p == Presence::Online).collect();
    let offline: Vec<&(String, Presence)> = list.iter().filter(|(_, p)| *p != Presence::Online).collect();
    c = c.child(caption(format!("ONLINE - {}", online.len())));
    for (n, _) in online {
        c = c.child(label(n.clone()).color(TEXT).wrap(false));
    }
    if !offline.is_empty() {
        c = c.child(caption(format!("OFFLINE - {}", offline.len())));
        for (n, _) in offline {
            c = c.child(label(n.clone()).color(MUTED).wrap(false));
        }
    }
    c
}

// ------------------------------------------------------------------ Add / Create / Join / New channel

fn back_button(m: &M) -> Element {
    let b = m.clone();
    button("Back").id("back").style("flat").on_click(move |_, _| b.borrow_mut().back())
}

fn add_page(m: &M, st: &Model) -> Element {
    let create = m.clone();
    let join = m.clone();
    let mut c = column()
        .gap(8)
        .child(heading("Add a server"))
        .child(label("Your server is where you and your friends hang out. Make yours and start talking.").color(MUTED))
        .child(row().child(button("Create My Own").id("create").style("primary").grow(1).on_click(move |_, _| {
            let mut s = create.borrow_mut();
            s.page = Page::Create;
            s.notice = None;
        })))
        .child(row().child(button("Join a Server").id("join").grow(1).on_click(move |_, _| {
            let mut s = join.borrow_mut();
            s.page = Page::Join;
            s.notice = None;
        })));
    c = c.child(caption("DISCORD BACKENDS"));
    if st.backends.is_empty() && !st.searching {
        c = c.child(label("None yet: a backend is a computer running discordd (and disco serve, to be found).").color(MUTED));
    }
    for b in &st.backends {
        let state = match &b.link {
            Link::Idle | Link::Connecting => "connecting".to_string(),
            Link::Up { .. } => format!("{} server{}", b.guilds.len(), if b.guilds.len() == 1 { "" } else { "s" }),
            Link::Down(why) => why.clone(),
        };
        c = c.child(label(format!("{} · {}", b.name, b.endpoint())).color(TEXT)).child(caption(state));
    }
    if st.searching {
        c = c.child(label("Looking for Discord backends ...").color(MUTED));
    } else {
        let again = m.clone();
        c = c.child(row().child(button("Search the network").id("search").on_click(move |_, _| again.borrow_mut().search())));
    }
    let typed = m.clone();
    let add = m.clone();
    let add2 = m.clone();
    let go = move |m: &M| {
        let r = m.borrow_mut().add_typed_backend();
        if let Err(e) = r {
            m.borrow_mut().notice = Some(e);
        }
    };
    let go2 = go.clone();
    c = c
        .child(text_field().id("address").placeholder("Backend address (10.0.8.1)").text(st.add_address.clone()).on_change(move |cx, _| {
            let t = cx.text("address");
            typed.borrow_mut().add_address = t;
        }).on_submit(move |_, _| go2(&add2)))
        .child(row().child(spacer()).child(button("Add backend").id("add_backend").on_click(move |_, _| go(&add))));
    if let Some(n) = &st.notice {
        c = c.child(label(n.clone()).color(MUTED).id("notice"));
    }
    c.child(row().child(back_button(m)))
}

fn create_page(m: &M, st: &Model) -> Element {
    let name = m.clone();
    let go = m.clone();
    let go2 = m.clone();
    let make = move |m: &M| {
        let r = m.borrow_mut().create_server();
        if let Err(e) = r {
            m.borrow_mut().notice = Some(e);
        }
    };
    let make2 = make.clone();
    let mut c = column()
        .gap(8)
        .child(heading("Create a server"))
        .child(label("Give your new server a name. You can always change it later.").color(MUTED))
        .child(caption("SERVER NAME"))
        .child(text_field().id("server_name").placeholder("My Server").text(st.new_server.clone()).on_change(move |cx, _| {
            let t = cx.text("server_name");
            name.borrow_mut().new_server = t;
        }).on_submit(move |_, _| make2(&go2)));
    let hosts = st.hosts();
    if hosts.len() > 1 {
        c = c.child(caption("HOST IT ON"));
        for (i, n) in hosts {
            let p = m.clone();
            let chosen = st.new_server_backend == i;
            c = c.child(row().child(button(format!("Host on {n}")).id(format!("host:{i}")).style(if chosen { "primary" } else { "flat" }).grow(1).on_click(move |_, _| p.borrow_mut().new_server_backend = i)));
        }
    }
    if let Some(n) = &st.notice {
        c = c.child(label(n.clone()).color(MUTED).id("notice"));
    }
    c.child(row().gap(6).child(back_button(m)).child(spacer()).child(button("Create").id("create_server").style("primary").on_click(move |_, _| make(&go))))
}

fn join_page(m: &M, st: &Model) -> Element {
    let mut c = column().gap(8).child(heading("Join a Server")).child(label("Servers on the Discord backends around you.").color(MUTED));
    let list = st.joinable();
    if list.is_empty() {
        c = c.child(label("Nothing new to join: you are in every server.").color(MUTED));
    }
    for (r, g) in list {
        let p = m.clone();
        let host = st.backends[r.0].name.clone();
        c = c.child(
            row()
                .gap(6)
                .align(Align::Center)
                .child(column().grow(1).child(label(g.name.clone()).color(TEXT).wrap(false)).child(caption(format!("{} member{} · {}", g.members, if g.members == 1 { "" } else { "s" }, host))))
                .child(button("Join").id(format!("join:{}:{}", r.0, r.1)).style("primary").on_click(move |_, _| {
                    let res = p.borrow_mut().join_server(r);
                    if let Err(e) = res {
                        p.borrow_mut().notice = Some(e);
                    }
                })),
        );
    }
    if let Some(n) = &st.notice {
        c = c.child(label(n.clone()).color(RED).id("notice"));
    }
    c.child(row().child(back_button(m)))
}

fn new_channel_page(m: &M, st: &Model) -> Element {
    let t = m.clone();
    let v = m.clone();
    let name = m.clone();
    let go = m.clone();
    let go2 = m.clone();
    let make = move |m: &M| {
        let r = m.borrow_mut().create_channel();
        if let Err(e) = r {
            m.borrow_mut().notice = Some(e);
        }
    };
    let make2 = make.clone();
    let text = st.new_channel_kind == Kind::Text;
    let mut c = column()
        .gap(8)
        .child(heading("New Channel"))
        .child(caption("CHANNEL TYPE"))
        .child(
            row()
                .gap(6)
                .child(button("Text").id("kind_text").style(if text { "primary" } else { "flat" }).grow(1).on_click(move |_, _| t.borrow_mut().new_channel_kind = Kind::Text))
                .child(button("Voice").id("kind_voice").style(if text { "flat" } else { "primary" }).grow(1).on_click(move |_, _| v.borrow_mut().new_channel_kind = Kind::Voice)),
        )
        .child(caption("CHANNEL NAME"))
        .child(text_field().id("channel_name").placeholder(if text { "new-channel" } else { "New Voice" }).text(st.new_channel.clone()).on_change(move |cx, _| {
            let t = cx.text("channel_name");
            name.borrow_mut().new_channel = t;
        }).on_submit(move |_, _| make2(&go2)));
    if let Some(n) = &st.notice {
        c = c.child(label(n.clone()).color(RED).id("notice"));
    }
    c.child(row().gap(6).child(back_button(m)).child(spacer()).child(button("Create Channel").id("create_channel").style("primary").on_click(move |_, _| make(&go))))
}

fn settings_page(m: &M, st: &Model) -> Element {
    let draft = m.clone();
    let go = m.clone();
    let go2 = m.clone();
    let save = move |m: &M| {
        let r = m.borrow_mut().save_username();
        if let Err(e) = r {
            m.borrow_mut().notice = Some(e);
        }
    };
    let save2 = save.clone();
    let mut c = column()
        .gap(8)
        .child(heading("Settings"))
        .child(caption("MY ACCOUNT"))
        .child(label("Username").color(TEXT))
        .child(text_field().id("username").placeholder("Username").text(st.name_draft.clone()).on_change(move |cx, _| {
            let t = cx.text("username");
            draft.borrow_mut().name_draft = t;
        }).on_submit(move |_, _| save2(&go2)))
        .child(label("1-16 letters, digits, '-', '_' or '.'. Everyone on every backend sees the new name.").color(MUTED));
    // What each connected backend calls you now (it may have added a number if the name was taken there).
    for b in st.backends.iter().filter(|b| b.up() && !b.me.is_empty()) {
        c = c.child(caption(format!("On {}: {}", b.name, b.me)));
    }
    if let Some(n) = &st.notice {
        c = c.child(label(n.clone()).color(MUTED).id("notice"));
    }
    c.child(row().gap(6).child(back_button(m)).child(spacer()).child(button("Save").id("save_username").style("primary").on_click(move |_, _| save(&go))))
}

/// A page that isn't Servers/Chat, in a scroll view `h` tall; None for those two.
fn other_page(m: &M, st: &Model, h: i32) -> Option<Element> {
    let page = match st.page {
        Page::Add => add_page(m, st),
        Page::Create => create_page(m, st),
        Page::Join => join_page(m, st),
        Page::NewChannel => new_channel_page(m, st),
        Page::Settings => settings_page(m, st),
        Page::Members => column().gap(8).child(row().child(back_button(m)).child(heading("Members"))).child(members(st)),
        Page::Servers | Page::Chat => return None,
    };
    Some(scroll_view(page).id("page").height(h.max(40)))
}

// ------------------------------------------------------------------ layouts

fn compact(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    let pad = 6;
    let h = cx.height - 2 * pad;
    if let Some(p) = other_page(m, st, h) {
        return column().padding(pad).child(p);
    }
    if st.page == Page::Chat && st.current_channel().is_some() {
        return column().padding(pad).child(chat(m, st, cx, h, true));
    }
    row()
        .align(Align::Stretch)
        .child(panel().fill(RAIL).child(rail(m, st)))
        .child(panel().fill(SIDEBAR).padding(pad).grow(1).child(channel_list(m, st, cx, h)))
}

fn side_by_side(m: &M, st: &Model, cx: &ViewCtx, wide: bool) -> Element {
    let pad = 10;
    let h = cx.height - 2 * pad;
    let main = match other_page(m, st, h) {
        Some(p) => column().grow(1).padding(pad).child(p),
        None => column().grow(1).padding(pad).child(chat(m, st, cx, h, false)),
    };
    let mut r = row()
        .align(Align::Stretch)
        .child(panel().fill(RAIL).child(rail(m, st)))
        .child(panel().fill(SIDEBAR).padding(pad).width(if wide { 200 } else { 170 }).child(channel_list(m, st, cx, h)))
        .child(main);
    if wide {
        r = r.child(panel().fill(SIDEBAR).padding(pad).width(170).child(scroll_view(members(st)).id("member_list").height(h.max(40))));
    }
    r
}

#[cfg(test)]
mod tests {
    #[test]
    fn initials() {
        assert_eq!(super::initials("Tech Village"), "TV");
        assert_eq!(super::initials("builders"), "BU");
        assert_eq!(super::initials("A Big Long Name"), "AB");
        assert_eq!(super::initials(""), "");
    }
}
