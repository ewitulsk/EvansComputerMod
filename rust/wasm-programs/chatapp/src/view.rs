//! The Chat window at the three size classes.
//!
//! * **compact** (phones, < 400 lu): the room's name, the connection line and
//!   a People button on top; the conversation (or the people, after People);
//!   the compose box and Send at the bottom.
//! * **medium**: a sidebar with the room (and its unread count) and the people
//!   in it, the conversation and the compose box beside it.
//! * **wide** (>= 800 lu): the room list, the conversation, and the people
//!   with the server's address in a panel on the right.
//!
//! Not connected (or after a failure) the conversation's place shows the
//! connect form: server address, nickname, Connect.

use crate::model::{Conn, Line, LineKind, Model, Pane};
use ecm_ui::element::{button, column, label, list, panel, row, scroll_view, spacer, text_field, Element};
use ecm_ui::{Align, ViewCtx};
use ecm_ui_proto::SizeClass;
use std::cell::RefCell;
use std::rc::Rc;

pub type M = Rc<RefCell<Model>>;

/// Body line, caption line, title line and button heights (default theme).
const LINE: i32 = 15;
const CAPTION: i32 = 11;
const TITLE: i32 = 20;
const BUTTON: i32 = LINE + 8;

pub fn view(m: &M, cx: &ViewCtx) -> Element {
    let st = m.borrow();
    match cx.class {
        SizeClass::Compact => compact(m, &st, cx),
        SizeClass::Medium => medium(m, &st, cx),
        SizeClass::Wide => wide(m, &st, cx),
    }
}

fn send_now(m: &M, cx: &mut ecm_ui::Ctx) {
    let text = cx.take_text("compose");
    m.borrow_mut().compose(&text);
}

/// The compose box and Send.
fn compose(m: &M, st: &Model) -> Element {
    let a = m.clone();
    let b = m.clone();
    row()
        .gap(6)
        .child(
            text_field()
                .id("compose")
                .placeholder(if st.connected() { format!("Message {}", st.room_title()) } else { "Not connected".to_string() })
                .grow(1)
                .on_submit(move |cx, _| send_now(&a, cx)),
        )
        .child(button("Send").style("primary").id("send").enabled(st.connected()).on_click(move |cx, _| send_now(&b, cx)))
}

/// One message (or notice) in the conversation.
fn line(cx: &ViewCtx, l: &Line) -> Element {
    match l.kind {
        LineKind::Message => {
            let who = if l.mine { format!("{} (you)", l.nick) } else { l.nick.clone() };
            let mut head = row().gap(6).child(label(who).color(if l.mine { cx.theme.accent } else { cx.theme.text }).wrap(false));
            let when = if l.earlier { format!("{} · earlier", l.time) } else { l.time.clone() };
            head = head.child(label(when).style("caption").wrap(false));
            column().gap(1).child(head).child(label(l.text.clone()))
        }
        LineKind::Notice => label(if l.time.is_empty() { l.text.clone() } else { format!("{}  {}", l.text, l.time) }).style("caption"),
        LineKind::Error => label(l.text.clone()).color(cx.theme.danger),
    }
}

/// The conversation, scrolled to its end by the app when a line arrives.
fn history(cx: &ViewCtx, st: &Model, h: i32) -> Element {
    let mut c = column().gap(8);
    if st.lines.is_empty() {
        c = c.child(label("No messages yet. Say hello!").style("caption"));
    }
    for l in &st.lines {
        c = c.child(line(cx, l));
    }
    scroll_view(c).id("history").height(h.max(40))
}

/// The connect form (shown instead of the conversation when not connected).
fn connect_form(m: &M, st: &Model, h: i32) -> Element {
    let s1 = m.clone();
    let s2 = m.clone();
    let go = m.clone();
    let go2 = m.clone();
    let connect = move |m: &M| {
        let r = m.borrow_mut().request_connect();
        if let Err(e) = r {
            m.borrow_mut().conn = Conn::Failed(e);
        }
    };
    let connect2 = connect.clone();
    let busy = st.conn == Conn::Connecting;
    let mut c = column().gap(8).child(label("Connect to a chat server").style("title"));
    c = c.child(on_the_network(m, st));
    c = c
        .child(label("The computer running chatd: its address (port 7777 unless you add :PORT).").style("caption"))
        .child(label(match &st.prefilled_from {
            Some(f) => format!("Filled in from {f}: this village's chat server."),
            None => "In a Tech Village: the chat server's address is on every village's web page (curl http://100.65.0.10/, section Chat)."
                .to_string(),
        }).style("caption"))
        .child(text_field().id("server").placeholder("10.0.30.1").text(st.server.clone()).on_change(move |cx, _| {
            let t = cx.text("server");
            s1.borrow_mut().server = t;
        }))
        .child(text_field().id("nick").placeholder("Nickname (optional)").text(st.nick.clone()).on_change(move |cx, _| {
            let t = cx.text("nick");
            s2.borrow_mut().nick = t;
        }).on_submit(move |_, _| connect2(&go2)))
        .child(
            row()
                .child(spacer())
                .child(button(if busy { "Connecting..." } else if matches!(st.conn, Conn::Failed(_)) { "Reconnect" } else { "Connect" })
                    .style("primary")
                    .id("connect")
                    .enabled(!busy)
                    .on_click(move |_, _| connect(&go))),
        );
    if let Conn::Failed(why) = &st.conn {
        c = c.child(label(why.clone()).color(0xe553_4bff));
    }
    scroll_view(c).id("connect_form").height(h.max(40))
}

/// Chat servers found on the network: a tap connects; Search again.
fn on_the_network(m: &M, st: &Model) -> Element {
    let mut c = column().gap(4).child(label("On the network").style("caption"));
    if st.searching {
        return c.child(label("Looking for chat servers ...").style("caption"));
    }
    for (i, f) in st.found.iter().enumerate() {
        let p = m.clone();
        let text = if f.name.is_empty() { f.endpoint() } else { format!("{} · {}", f.name, f.endpoint()) };
        c = c.child(button(text).id(format!("found:{i}")).on_click(move |_, _| {
            let r = p.borrow_mut().pick_found(i);
            if let Err(e) = r {
                p.borrow_mut().conn = Conn::Failed(e);
            }
        }));
    }
    if st.searched && st.found.is_empty() {
        c = c.child(label("None found (a server is found when its computer runs disco serve, and routers relay).").style("caption"));
    }
    let again = m.clone();
    c.child(row().child(button("Search again").id("search").on_click(move |_, _| again.borrow_mut().search())))
}

/// The people in the room (a tap puts "@nick " in the compose box).
fn people(st: &Model) -> Element {
    let mut c = column().gap(4).child(label(format!("People ({})", st.people.len())).style("caption"));
    if st.people.is_empty() {
        c = c.child(label(if st.connected() { "Nobody yet" } else { "Not connected" }).style("caption"));
    }
    let names: Vec<String> = st.people.iter().map(|p| if *p == st.me { format!("{p} (you)") } else { p.clone() }).collect();
    let nicks = st.people.clone();
    c.child(list().id("people").children(names.into_iter().map(label)).on_select(move |cx, _| {
        if let Some(n) = cx.selected("people").and_then(|i| nicks.get(i).cloned()) {
            let cur = cx.text("compose");
            cx.set_text("compose", &format!("{cur}@{n} "));
            cx.focus("compose");
        }
    }))
}

/// The room list (one room per server today) with its unread count.
fn rooms(st: &Model) -> Element {
    let title = if st.unread > 0 { format!("# {}  ({})", st.room_title(), st.unread) } else { format!("# {}", st.room_title()) };
    column()
        .gap(4)
        .child(label("Rooms").style("caption"))
        .child(list().id("rooms").selected(0).children([label(title).wrap(false)]))
}

fn header(m: &M, st: &Model, compact: bool) -> Element {
    let mut r = row().gap(6).child(label(st.room_title()).style("title").wrap(false)).child(spacer());
    if compact && st.connected() {
        let p = m.clone();
        let text = match st.pane {
            Pane::Conversation => format!("People ({})", st.people.len()),
            Pane::People => "Chat".to_string(),
        };
        r = r.child(button(text).id("people_toggle").on_click(move |_, _| {
            let mut s = p.borrow_mut();
            s.pane = if s.pane == Pane::People { Pane::Conversation } else { Pane::People };
        }));
    }
    column().gap(2).child(r).child(label(st.status()).style("caption").id("status"))
}

/// The conversation (or the connect form) and the compose box, `h` lu tall.
fn main_column(m: &M, st: &Model, cx: &ViewCtx, h: i32) -> Element {
    let body_h = h - BUTTON - 8;
    let body = if matches!(st.conn, Conn::Idle | Conn::Connecting | Conn::Failed(_)) && !st.connected() {
        connect_form(m, st, body_h)
    } else {
        history(cx, st, body_h)
    };
    column().gap(8).grow(1).child(body).child(compose(m, st))
}

fn compact(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    let pad = 6;
    let top = pad + TITLE + 2 + CAPTION + 8;
    let h = cx.height - top - pad;
    let mut c = column().gap(8).padding(pad).child(header(m, st, true));
    if st.pane == Pane::People && st.connected() {
        c = c.child(scroll_view(people(st)).id("people_page").height(h.max(40)));
    } else {
        c = c.child(main_column(m, st, cx, h));
    }
    c
}

fn medium(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    let pad = 10;
    let side = column().gap(12).width(150).child(rooms(st)).child(people(st));
    let h = cx.height - 2 * pad - (TITLE + 2 + CAPTION + 8);
    let main = column().gap(8).grow(1).child(header(m, st, false)).child(main_column(m, st, cx, h));
    row().gap(12).padding(pad).align(Align::Stretch).child(panel().fill(cx.theme.surface).radius(6).padding(8).child(side)).child(main)
}

fn wide(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    let pad = 12;
    let left = column()
        .gap(12)
        .width(170)
        .child(label("Chat").style("title"))
        .child(rooms(st))
        .child(label(if st.server.is_empty() { "No server yet".to_string() } else { format!("Server {}", st.endpoint()) }).style("caption"));
    let h = cx.height - 2 * pad - (TITLE + 2 + CAPTION + 8);
    let main = column().gap(8).grow(1).child(header(m, st, false)).child(main_column(m, st, cx, h));
    let right = column().gap(8).width(170).child(people(st));
    row()
        .gap(12)
        .padding(pad)
        .align(Align::Stretch)
        .child(panel().fill(cx.theme.surface).radius(6).padding(8).child(left))
        .child(main)
        .child(panel().fill(cx.theme.surface).radius(6).padding(8).child(right))
}
