//! The Music window at the three size classes.
//!
//! * **compact** (phones): one page at a time, the tabs along the bottom and a
//!   small player bar above them while a song is loaded.
//! * **medium**: a rail on the left (the backends, signed in or "Sign in", and
//!   the tabs) beside the page.
//! * **wide**: the same plus the queue on the right.
//!
//! Handlers only queue [`Action`]s on the model; the driver does them.

use crate::model::{clock, Action, Album, Artist, Model, Page, Playlist, QualitySetting, Repeat, Song, Tab};
use ecm_ui::element::{button, column, image, label, panel, row, scroll_view, slider, spacer, text_field, Element};
use ecm_ui::{Align, ViewCtx};
use ecm_ui_proto::SizeClass;
use std::cell::RefCell;
use std::rc::Rc;

pub type M = Rc<RefCell<Model>>;

pub const BG: u32 = 0x1212_12ff;
pub const SIDE: u32 = 0x0000_00ff;
pub const CARD: u32 = 0x1f1f_1fff;
pub const GREEN: u32 = 0x1db9_54ff;
pub const TEXT: u32 = 0xffff_ffff;
pub const MUTED: u32 = 0xb3b3_b3ff;
pub const RED: u32 = 0xe9_14_2fff;

pub fn view(m: &M, cx: &ViewCtx) -> Element {
    let st = m.borrow();
    if st.page == Page::SignIn {
        if let Some(s) = &st.signin {
            return panel().fill(BG).child(scroll_view(ecm_apps_ui::signin::view(s)).id("signin_scroll"));
        }
    }
    let page = page(m, &st, cx);
    let body = match cx.class {
        SizeClass::Compact => {
            let mut c = column().gap(4).grow(1).child(scroll_view(page).id("page").grow(1));
            if st.now.track.is_some() && st.tab != Tab::NowPlaying {
                c = c.child(mini_player(m, &st));
            }
            c.child(tab_bar(m, &st))
        }
        class => {
            let mut r = row().gap(8).grow(1).child(rail(m, &st)).child(scroll_view(page).id("page").grow(1));
            if class == SizeClass::Wide {
                r = r.child(column().width(220).child(queue_list(m, &st)));
            }
            column().grow(1).child(r).child(mini_player(m, &st))
        }
    };
    panel().fill(BG).padding(6).child(body)
}

fn caption(t: impl Into<String>) -> Element {
    label(t).style("caption").color(MUTED).wrap(false)
}

fn heading(t: impl Into<String>) -> Element {
    label(t).style("title").color(TEXT).wrap(false)
}

fn act(m: &M, a: Action) -> impl Fn(&mut ecm_ui::Ctx, &ecm_ui::UiEvent) + 'static {
    let m = m.clone();
    move |_, _| m.borrow_mut().act(a.clone())
}

const TABS: [(Tab, &str, &str); 5] =
    [(Tab::Home, "home", "Home"), (Tab::Search, "search", "Search"), (Tab::Library, "library", "Library"), (Tab::NowPlaying, "now", "Playing"), (Tab::Settings, "settings", "Settings")];

fn tab_bar(m: &M, st: &Model) -> Element {
    let mut r = row().gap(2);
    for (t, id, name) in TABS {
        let mut b = button(name).id(format!("tab:{id}")).grow(1).style("flat").on_click(act(m, Action::Tab(t)));
        if st.tab == t && st.page == Page::Main {
            b = b.style("primary");
        }
        r = r.child(b);
    }
    r
}

fn rail(m: &M, st: &Model) -> Element {
    let mut c = column().gap(4).width(150).padding(4);
    if st.logo != 0 {
        c = c.child(row().gap(6).align(Align::Center).child(image(st.logo, 24, 24)).child(heading("Music")));
    }
    c = c.child(caption("SERVERS"));
    c = c.children(backend_rows(m, st, ""));
    c = c.child(caption(""));
    for (t, id, name) in TABS {
        let mut b = button(name).id(format!("tab:{id}")).style("flat").on_click(act(m, Action::Tab(t)));
        if st.tab == t && st.page == Page::Main {
            b = b.style("primary");
        }
        c = c.child(b);
    }
    c
}

/// The backends; `p` prefixes the ids ("" on the rail and Home, "s" in Settings).
fn backend_rows(m: &M, st: &Model, p: &str) -> Vec<Element> {
    let mut v = Vec::new();
    if st.backends.is_empty() {
        v.push(caption(if st.searching { "Looking for music servers..." } else { "No music server found yet" }).id(format!("{p}no_backends")));
    }
    for (i, b) in st.backends.iter().enumerate() {
        let name = if b.name.is_empty() { b.endpoint() } else { b.name.clone() };
        let mut pick = button(name).id(format!("{p}backend:{i}")).style("flat").grow(1).on_click(act(m, Action::Select(i)));
        if st.current == Some(i) {
            pick = pick.style("primary");
        }
        let who: Element = match (&b.user, &b.error) {
            (Some(u), _) => caption(u.clone()).id(format!("{p}user:{i}")),
            (None, Some(e)) => caption(e.clone()).color(RED).id(format!("{p}berror:{i}")),
            (None, None) => button("Sign in").id(format!("{p}signin:{i}")).style("flat").on_click(act(m, Action::SignIn(i))),
        };
        v.push(column().gap(1).child(row().child(pick)).child(row().child(spacer().width(8)).child(who)));
    }
    v
}

fn header(m: &M, st: &Model, title: &str) -> Element {
    let mut r = row().gap(6).align(Align::Center);
    if st.page != Page::Main {
        r = r.child(button("<").id("back").style("flat").on_click(act(m, Action::Back)));
    }
    r = r.child(heading(title.to_string())).child(spacer());
    if let Some(b) = st.backend() {
        r = r.child(caption(if b.name.is_empty() { b.endpoint() } else { b.name.clone() }));
    }
    let mut c = column().gap(4).child(r);
    if let Some(n) = &st.notice {
        c = c.child(label(n.clone()).color(MUTED).id("notice"));
    }
    c
}

/// A list of songs; `ctx` names the list in the ids (`song:<ctx>:<i>`, ...).
fn song_list(m: &M, songs: &[Song], ctx: &str, playlist: Option<&Playlist>) -> Element {
    let mut c = column().gap(2);
    for (i, s) in songs.iter().enumerate() {
        let play = Action::Play { songs: songs.to_vec(), index: i };
        let title = if s.missing { "(no longer on this server)".to_string() } else { s.title.clone() };
        let mut left = column().grow(1).child(button(title).id(format!("song:{ctx}:{i}")).style("flat").enabled(!s.missing).on_click(act(m, play)));
        left = left.child(caption(format!("{}  {}", s.artist, clock(s.duration_ms))));
        let mut r = row().gap(2).align(Align::Center).child(left);
        if !s.missing {
            r = r
                .child(button(if s.liked { "Liked" } else { "Like" }).id(format!("like:{ctx}:{i}")).style("flat").on_click(act(m, Action::Like(s.id.clone(), !s.liked))))
                .child(button("Next").id(format!("next:{ctx}:{i}")).style("flat").on_click(act(m, Action::PlayNext(s.clone()))))
                .child(button("Queue").id(format!("queue:{ctx}:{i}")).style("flat").on_click(act(m, Action::Enqueue(s.clone()))))
                .child(button("+").id(format!("add:{ctx}:{i}")).style("flat").on_click(act(m, Action::Open(Page::AddTo(s.clone())))));
        }
        if let Some(p) = playlist.filter(|p| p.mine) {
            let pid = p.id.clone();
            if i > 0 {
                r = r.child(button("^").id(format!("up:{i}")).style("flat").on_click(act(m, Action::MoveInPlaylist { playlist: pid.clone(), from: i, to: i - 1 })));
            }
            if i + 1 < songs.len() {
                r = r.child(button("v").id(format!("down:{i}")).style("flat").on_click(act(m, Action::MoveInPlaylist { playlist: pid.clone(), from: i, to: i + 1 })));
            }
            r = r.child(button("x").id(format!("remove:{i}")).style("flat").on_click(act(m, Action::RemoveFromPlaylist { playlist: pid, index: i })));
        }
        c = c.child(r);
    }
    if songs.is_empty() {
        c = c.child(caption("Nothing here yet"));
    }
    c
}

fn albums_list(m: &M, albums: &[Album]) -> Element {
    let mut c = column().gap(2);
    for a in albums {
        c = c.child(
            column()
                .child(button(a.name.clone()).id(format!("album:{}", a.id)).style("flat").on_click(act(m, Action::Open(Page::Album(a.id.clone())))))
                .child(caption(format!("{} · {} songs", a.artist, a.song_count))),
        );
    }
    c
}

fn artists_list(m: &M, artists: &[Artist]) -> Element {
    let mut c = column().gap(2);
    for a in artists {
        c = c.child(button(a.name.clone()).id(format!("artist:{}", a.id)).style("flat").on_click(act(m, Action::Open(Page::Artist(a.id.clone())))));
    }
    c
}

fn playlists_list(m: &M, pls: &[Playlist]) -> Element {
    let mut c = column().gap(2);
    for p in pls {
        let sub = if p.mine { format!("{} songs{}", p.song_count, if p.public { " · public" } else { "" }) } else { format!("by {} · {} songs", p.owner, p.song_count) };
        c = c.child(
            column().child(button(p.name.clone()).id(format!("playlist:{}", p.id)).style("flat").on_click(act(m, Action::Open(Page::Playlist(p.id.clone()))))).child(caption(sub)),
        );
    }
    c
}

fn page(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    match &st.page {
        Page::Main => match st.tab {
            Tab::Home => home(m, st, cx),
            Tab::Search => search(m, st),
            Tab::Library => library(m, st),
            Tab::NowPlaying => now_playing(m, st),
            Tab::Settings => settings(m, st),
        },
        Page::Album(_) | Page::Artist(_) => {
            let d = &st.detail;
            let mut c = column().gap(6).child(header(m, st, &d.title)).child(caption(d.subtitle.clone()));
            if !d.songs.is_empty() {
                c = c.child(button("Play").id("play_all").style("primary").on_click(act(m, Action::Play { songs: d.songs.clone(), index: 0 })));
            }
            if !d.albums.is_empty() {
                c = c.child(caption("ALBUMS")).child(albums_list(m, &d.albums));
            }
            c.child(caption("SONGS")).child(song_list(m, &d.songs, "detail", None))
        }
        Page::Playlist(_) => {
            let Some(p) = &st.detail.playlist else { return header(m, st, "Playlist") };
            let mut c = column().gap(6).child(header(m, st, &p.name));
            c = c.child(caption(if p.mine { format!("Your playlist · {}", if p.public { "public" } else { "private" }) } else { format!("by {}", p.owner) }));
            let mut r = row().gap(4);
            if !p.songs.is_empty() {
                r = r.child(button("Play").id("play_all").style("primary").on_click(act(m, Action::Play { songs: p.songs.iter().filter(|s| !s.missing).cloned().collect(), index: 0 })));
            }
            if p.mine {
                r = r
                    .child(button("Rename").id("rename").on_click(act(m, Action::Open(Page::Rename(p.id.clone())))))
                    .child(button(if p.public { "Make private" } else { "Make public" }).id("public").on_click(act(m, Action::SetPublic { playlist: p.id.clone(), public: !p.public })))
                    .child(button("Delete").id("delete").style("danger").on_click(act(m, Action::DeletePlaylist(p.id.clone()))));
            }
            c.child(r).child(song_list(m, &p.songs, "detail", Some(p)))
        }
        Page::Liked => column().gap(6).child(header(m, st, "Liked Songs")).child(song_list(m, &st.liked, "liked", None)),
        Page::AddTo(song) => {
            let mut c = column().gap(6).child(header(m, st, "Add to playlist")).child(caption(song.title.clone()));
            for p in st.my_playlists.iter() {
                c = c.child(button(p.name.clone()).id(format!("addto:{}", p.id)).on_click(act(m, Action::AddToPlaylist { playlist: p.id.clone(), song: song.clone() })));
            }
            c.child(button("New playlist").id("addto_new").style("primary").on_click(act(m, Action::Open(Page::NewPlaylist(Some(song.clone()))))))
        }
        Page::NewPlaylist(song) => name_page(m, st, None, song.clone()),
        Page::Rename(id) => name_page(m, st, Some(id.clone()), None),
        Page::AddBackend => {
            let mm = m.clone();
            let add = move |cx: &mut ecm_ui::Ctx, _: &ecm_ui::UiEvent| {
                let a = cx.text("address");
                mm.borrow_mut().act(Action::AddBackend(a));
            };
            column()
                .gap(6)
                .child(header(m, st, "Add a server"))
                .child(caption("The address of a computer running musicd (host or host:port; port 7800 by default)."))
                .child(text_field().id("address").placeholder("10.0.0.5 or radio.village:7800").on_submit(add.clone()))
                .child(row().child(button("Search the network").id("discover").on_click(act(m, Action::Discover))).child(spacer()).child(button("Add").id("add_backend").style("primary").on_click(add)))
        }
        _ => header(m, st, "Music"),
    }
}

/// New playlist (optionally with a song in it), or rename one.
fn name_page(m: &M, st: &Model, rename: Option<String>, song: Option<Song>) -> Element {
    let mm = m.clone();
    let r2 = rename.clone();
    let save = move |cx: &mut ecm_ui::Ctx, _: &ecm_ui::UiEvent| {
        let name = cx.text("pl_name");
        let a = match &r2 {
            Some(id) => Action::RenamePlaylist { playlist: id.clone(), name },
            None => Action::NewPlaylist { name, song: song.clone() },
        };
        mm.borrow_mut().act(a);
    };
    column()
        .gap(6)
        .child(header(m, st, if rename.is_some() { "Rename playlist" } else { "New playlist" }))
        .child(text_field().id("pl_name").placeholder("Playlist name").on_submit(save.clone()))
        .child(row().child(spacer()).child(button(if rename.is_some() { "Save" } else { "Create" }).id("create_playlist").style("primary").on_click(save)))
}
fn home(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    let mut c = column().gap(8).child(header(m, st, "Home"));
    if cx.class == SizeClass::Compact {
        c = c.children(backend_rows(m, st, ""));
    }
    if st.backend().is_none() {
        return c.child(button("Add a server").id("open_add").on_click(act(m, Action::Open(Page::AddBackend))));
    }
    if !st.recent.is_empty() {
        c = c.child(caption("RECENTLY PLAYED")).child(song_list(m, &st.recent, "recent", None));
    }
    if !st.my_playlists.is_empty() {
        c = c.child(caption("YOUR PLAYLISTS")).child(playlists_list(m, &st.my_playlists));
    }
    c.child(caption("NEW ON THIS SERVER")).child(song_list(m, &st.new_songs, "new", None))
}

fn search(m: &M, st: &Model) -> Element {
    let mm = m.clone();
    let field = text_field().id("q").placeholder("Songs, artists, albums, playlists").text(st.query.clone()).on_change(move |cx, _| {
        let q = cx.text("q");
        mm.borrow_mut().act(Action::Search(q));
    });
    let mut c = column().gap(8).child(header(m, st, "Search")).child(field);
    let r = &st.results;
    if let Some((kind, name, sub, id, song)) = &r.top {
        let open = match (kind.as_str(), song) {
            ("song", Some(s)) => Action::Play { songs: vec![s.clone()], index: 0 },
            ("album", _) => Action::Open(Page::Album(id.clone())),
            ("playlist", _) => Action::Open(Page::Playlist(id.clone())),
            _ => Action::Open(Page::Artist(id.clone())),
        };
        c = c.child(caption("TOP RESULT")).child(
            panel().fill(CARD).radius(6).padding(8).child(column().child(button(name.clone()).id("top").style("flat").on_click(act(m, open))).child(caption(format!("{kind} · {sub}")))),
        );
    }
    if !r.songs.is_empty() {
        c = c.child(caption("SONGS")).child(song_list(m, &r.songs, "search", None));
    }
    if !r.artists.is_empty() {
        c = c.child(caption("ARTISTS")).child(artists_list(m, &r.artists));
    }
    if !r.albums.is_empty() {
        c = c.child(caption("ALBUMS")).child(albums_list(m, &r.albums));
    }
    if !r.playlists.is_empty() {
        c = c.child(caption("PLAYLISTS")).child(playlists_list(m, &r.playlists));
    }
    if !st.query.trim().is_empty() && r.top.is_none() {
        c = c.child(caption(format!("Nothing found for \"{}\"", st.query.trim())).id("nothing"));
    }
    c
}

fn library(m: &M, st: &Model) -> Element {
    column()
        .gap(8)
        .child(header(m, st, "Library"))
        .child(row().gap(4).child(button("Liked Songs").id("liked").on_click(act(m, Action::Open(Page::Liked)))).child(button("New playlist").id("new_playlist").on_click(act(m, Action::Open(Page::NewPlaylist(None))))))
        .child(caption("PLAYLISTS"))
        .child(playlists_list(m, &st.my_playlists))
        .child(caption("ALBUMS"))
        .child(albums_list(m, &st.albums))
        .child(caption("ARTISTS"))
        .child(artists_list(m, &st.artists))
}

fn controls(m: &M, st: &Model) -> Element {
    row()
        .gap(4)
        .child(button(if st.shuffle { "Shuffle on" } else { "Shuffle" }).id("shuffle").style("flat").on_click(act(m, Action::Shuffle)))
        .child(button("|<").id("prev").on_click(act(m, Action::Previous)))
        .child(button(if st.now.playing { "Pause" } else { "Play" }).id("toggle").style("primary").on_click(act(m, Action::Toggle)))
        .child(button(">|").id("next").on_click(act(m, Action::Next)))
        .child(
            button(match st.repeat {
                Repeat::Off => "Repeat",
                Repeat::All => "Repeat all",
                Repeat::One => "Repeat one",
            })
            .id("repeat")
            .style("flat")
            .on_click(act(m, Action::Repeat)),
        )
}

fn now_playing(m: &M, st: &Model) -> Element {
    let mut c = column().gap(8).child(header(m, st, "Now Playing"));
    let Some(t) = &st.now.track else { return c.child(caption("Nothing is playing. Pick a song in Home, Search or Library.")) };
    if st.now.art != 0 {
        c = c.child(image(st.now.art, 64, 64));
    }
    c = c.child(heading(t.title.clone()).id("now_title")).child(caption(format!("{} · {}", t.artist, t.album)));
    let secs = (t.duration_ms / 1000) as i32;
    let mm = m.clone();
    c = c.child(slider(0, secs.max(1)).id("seek").value((st.now.position_ms / 1000) as i32).on_change(move |cx, _| {
        let v = cx.value("seek").max(0) as u64;
        mm.borrow_mut().act(Action::Seek(v * 1000));
    }));
    c = c.child(row().child(caption(clock(st.now.position_ms)).id("position")).child(spacer()).child(caption(clock(t.duration_ms))));
    if !st.now.status.is_empty() {
        c = c.child(caption(st.now.status.clone()).id("status"));
    }
    c.child(controls(m, st)).child(caption("QUEUE")).child(queue_list(m, st))
}

fn queue_list(m: &M, st: &Model) -> Element {
    let mut c = column().gap(2);
    for (i, t) in st.queue.iter().enumerate() {
        let here = st.pos == Some(i);
        let mut b = button(t.title.clone()).id(format!("qitem:{i}")).style("flat").grow(1).on_click(act(m, Action::PlayQueued(i)));
        if here {
            b = b.style("primary");
        }
        c = c.child(row().child(b).child(button("x").id(format!("qremove:{i}")).style("flat").on_click(act(m, Action::Unqueue(i)))));
    }
    if st.queue.is_empty() {
        c = c.child(caption("The queue is empty"));
    }
    c
}

fn mini_player(m: &M, st: &Model) -> Element {
    let Some(t) = &st.now.track else { return row() };
    let status = if st.now.buffering { "Buffering...".to_string() } else { format!("{} / {}", clock(st.now.position_ms), clock(t.duration_ms)) };
    panel().fill(CARD).radius(6).padding(4).child(
        row()
            .gap(6)
            .align(Align::Center)
            .child(column().grow(1).child(button(t.title.clone()).id("mini_title").style("flat").on_click(act(m, Action::Tab(Tab::NowPlaying)))).child(caption(status).id("mini_status")))
            .child(button(if st.now.playing { "Pause" } else { "Play" }).id("mini_toggle").on_click(act(m, Action::Toggle)))
            .child(button(">|").id("mini_next").on_click(act(m, Action::Next))),
    )
}

fn settings(m: &M, st: &Model) -> Element {
    let mut c = column().gap(8).child(header(m, st, "Settings")).child(caption("STREAMING QUALITY"));
    for (q, name, text) in [
        (QualitySetting::Auto, "auto", "Auto: Data saver on cellular, Original on Wi-Fi or a cable"),
        (QualitySetting::Original, "original", "Original: the MP3 as it is"),
        (QualitySetting::Saver, "saver", "Data saver: 22 kHz mono, about 11 KB/s"),
    ] {
        let mut b = button(text).id(format!("quality:{name}")).style("flat").on_click(act(m, Action::Quality(q)));
        if st.quality == q {
            b = b.style("primary");
        }
        c = c.child(b);
    }
    c = c.child(caption("A change applies from the next song.")).child(caption("SERVERS"));
    c = c.children(backend_rows(m, st, "s"));
    c = c.child(row().gap(4).child(button("Add a server").id("open_add").on_click(act(m, Action::Open(Page::AddBackend)))).child(button("Search again").id("discover").on_click(act(m, Action::Discover))));
    if let Some(i) = st.current.filter(|i| st.backends.get(*i).is_some_and(|b| b.user.is_some())) {
        c = c.child(button("Sign out of this server").id("signout").on_click(act(m, Action::SignOut(i))));
    }
    c = c.child(caption("ACCOUNTS (remembered server keys)"));
    if st.pins.is_empty() {
        c = c.child(caption("None yet"));
    }
    for (fp, host, name) in &st.pins {
        c = c.child(
            row()
                .gap(4)
                .align(Align::Center)
                .child(column().grow(1).child(label(name.clone()).wrap(false)).child(caption(format!("{host} · {}", ecm_auth::short_fingerprint(fp)))))
                .child(button("Forget").id(format!("forget:{fp}")).style("danger").on_click(act(m, Action::ForgetPin(fp.clone())))),
        );
    }
    c
}
