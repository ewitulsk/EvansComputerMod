//! The P3 apps against a mock compositor (`MockConnection`) and a mock
//! computer (`MockSys` over a temp directory): the desktop's window
//! management (frames, placement, stacking, focus, minimize, drags, Alt+Tab,
//! the launcher, toasts, permission prompts, the theme from /etc/ui.toml,
//! --restart), the Terminal's session I/O and clipboard, Files, Settings'
//! Wi-Fi join and theme switch, and the Editor.

use ecm_apps::desktop::{self, Desktop, BORDER, TITLE_H};
use ecm_apps::sys::{CellState, DisplayState, MockSys, Network};
use ecm_apps::{editor, files, settings, terminal};
use ecm_ui::samples::welcome;
use ecm_ui::{App, MockConnection, TEXT_MIME};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Message, Prop, TreeOp};
use std::collections::HashMap;
use std::path::PathBuf;

// ------------------------------------------------------------------ helpers

fn temp_root(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("ecm-apps-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn mock(app_id: &str, w: i32, h: i32) -> MockConnection {
    let m = MockConnection::new();
    m.state.borrow_mut().idle_timeouts = true;
    let mut wl = welcome(false);
    if let Message::Welcome(ref mut x) = wl {
        x.app_id = app_id.into();
        x.display.w = w;
        x.display.h = h;
        x.display.size_class = proto::SizeClass::for_width(w);
    }
    m.push(&[wl]);
    m
}

/// Texts by surface, from the TreeOps sent.
#[derive(Default)]
struct Texts {
    nodes: HashMap<(u32, u32), String>,
}

impl Texts {
    fn apply(&mut self, msgs: &[Message]) {
        for m in msgs {
            if let Message::TreeOps(t) = m {
                for op in &t.ops {
                    match op {
                        TreeOp::Create { id, props, .. } | TreeOp::Set { id, props } => {
                            for p in props {
                                if let Prop::Text(s) = p {
                                    self.nodes.insert((t.surface, *id), s.clone());
                                }
                            }
                        }
                        TreeOp::Remove { id } => {
                            self.nodes.remove(&(t.surface, *id));
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    fn of(&self, surface: u32) -> Vec<String> {
        let mut v: Vec<(&u32, &String)> = self.nodes.iter().filter(|((s, _), _)| *s == surface).map(|((_, id), t)| (id, t)).collect();
        v.sort();
        v.into_iter().map(|(_, t)| t.clone()).collect()
    }
    fn has(&self, surface: u32, text: &str) -> bool {
        self.of(surface).iter().any(|t| t.contains(text))
    }
}

fn sent(m: &MockConnection) -> Vec<Message> {
    let s = m.sent_messages();
    m.clear_sent();
    s
}

fn configure(surface: u32, serial: u32, w: i32, h: i32, states: u32) -> Message {
    Message::Configure(proto::Configure { surface, serial, w, h, states })
}

fn pointer(surface: u32, kind: proto::PointerKind, x: i32, y: i32, hit: u32, button: proto::Button) -> Message {
    let held = match (kind, button) {
        (proto::PointerKind::Down | proto::PointerKind::Move, proto::Button::Left) => proto::buttons::LEFT,
        (proto::PointerKind::Down | proto::PointerKind::Move, proto::Button::Right) => proto::buttons::RIGHT,
        _ => 0,
    };
    Message::Pointer(proto::Pointer {
        surface,
        kind,
        x,
        y,
        buttons: held,
        button: if matches!(kind, proto::PointerKind::Down | proto::PointerKind::Up) { button } else { proto::Button::None },
        scroll_x: 0,
        scroll_y: 0,
        hit_id: hit,
    })
}

fn click(surface: u32, hit: u32) -> Vec<Message> {
    vec![pointer(surface, proto::PointerKind::Down, 5, 5, hit, proto::Button::Left), pointer(surface, proto::PointerKind::Up, 5, 5, hit, proto::Button::Left)]
}

fn key(surface: u32, key: u16, mods: u16) -> Message {
    Message::Key(proto::Key { surface, key, mods, down: true, repeat: false })
}

fn text(surface: u32, s: &str) -> Message {
    Message::Text(proto::Text { surface, text: s.into() })
}

/// Drive an App: deliver messages, run two loop turns, return what it sent.
fn run(app: &mut App, m: &MockConnection, t: &mut Texts, msgs: Vec<Message>) -> Vec<Message> {
    m.push(&msgs);
    app.step(0);
    app.step(0);
    let s = sent(m);
    t.apply(&s);
    s
}

fn hit(app: &App, surface: u32, id: &str) -> u32 {
    app.core.window(surface).unwrap_or_else(|| panic!("no surface {surface}")).hit_id_of(id).unwrap_or_else(|| panic!("no hit id for {id}"))
}

// ------------------------------------------------------------------ desktop

struct Desk {
    d: Desktop,
    m: MockConnection,
    sys: MockSys,
    t: Texts,
}

impl Desk {
    fn new(args: &[&str], root: &PathBuf, w: i32, h: i32) -> (Desk, Vec<Message>) {
        let m = mock("desktop", w, h);
        let sys = MockSys::new(root);
        sys.state.borrow_mut().clock = Some((9, 41));
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let d = Desktop::new(Box::new(m.clone()), Box::new(sys.clone()), &args).unwrap();
        let mut desk = Desk { d, m, sys, t: Texts::default() };
        // The compositor configures layer surfaces when they are made.
        let (tb, wp) = {
            let s = desk.d.state.borrow();
            (s.taskbar, s.wallpaper)
        };
        let first = desk.step(vec![configure(wp, 1, w, h, 0), configure(tb, 1, w, desktop::TASKBAR_H, 0)]);
        (desk, first)
    }

    fn step(&mut self, msgs: Vec<Message>) -> Vec<Message> {
        self.m.push(&msgs);
        self.d.step(0);
        self.d.step(0);
        let s = sent(&self.m);
        self.t.apply(&s);
        s
    }

    fn surface_titled(&self, title: &str) -> u32 {
        self.d.app.core.windows.iter().find(|w| w.title == title).map(|w| w.surface).unwrap_or_else(|| panic!("no surface {title}"))
    }

    /// Open app window `wid`, then report its frame as compositor window `frame_wid`.
    fn open(&mut self, wid: u32, frame_wid: u32, app: &str, title: &str) -> Vec<Message> {
        let mut out = self.step(vec![Message::WindowOpened(proto::WindowOpened {
            window: wid,
            pid: 40 + wid,
            app_id: app.into(),
            title: title.into(),
            icon: String::new(),
            min_w: 0,
            min_h: 0,
            max_w: 0,
            max_h: 0,
            parent: 0,
        })]);
        let frame_title = format!("frame:{wid}");
        let surface = self.surface_titled(&frame_title);
        assert!(out.iter().any(|m| matches!(m, Message::SurfaceCreate(s) if s.surface == surface && s.role == proto::SurfaceRole::Window)));
        out.extend(self.step(vec![Message::WindowOpened(proto::WindowOpened {
            window: frame_wid,
            pid: 1,
            app_id: "desktop".into(),
            title: frame_title,
            icon: String::new(),
            min_w: 0,
            min_h: 0,
            max_w: 0,
            max_h: 0,
            parent: 0,
        })]));
        // The compositor configures the frame once it is placed.
        let fr = places(&out).get(&frame_wid).copied();
        if let Some(f) = fr {
            out.extend(self.step(vec![configure(surface, 1, f.2, f.3, 0)]));
        }
        out
    }
}

fn places(msgs: &[Message]) -> HashMap<u32, (i32, i32, i32, i32, u32)> {
    msgs.iter().filter_map(|m| if let Message::Place(p) = m { Some((p.window, (p.x, p.y, p.w, p.h, p.states))) } else { None }).collect()
}

fn last_stack(msgs: &[Message]) -> Option<Vec<u32>> {
    msgs.iter().rev().find_map(|m| if let Message::Stack(s) = m { Some(s.windows.clone()) } else { None })
}

#[test]
fn desktop_frames_places_stacks_and_focuses_windows() {
    let root = temp_root("desk1");
    std::fs::create_dir_all(root.join("etc")).unwrap();
    std::fs::write(root.join("etc/ui.toml"), "theme = \"ecm-ocean\"\n").unwrap();
    let (mut desk, first) = Desk::new(&["--launch", "terminal", "--launch", "files:/etc"], &root, 640, 400);
    let _ = first;
    let all = desk.m.state.borrow().sent.clone();
    let boot = Message::decode_all(&all).unwrap_or_default();
    let _ = boot;
    // Hello, SetTheme from /etc/ui.toml, layers, chords and the session's apps went out at start.
    let st = desk.d.state.borrow();
    assert_eq!(st.theme_name, "ecm-ocean");
    assert!(st.taskbar != 0 && st.wallpaper != 0);
    drop(st);

    // Terminal opens as window 1; its frame is the compositor's window 2.
    let out = desk.open(1, 2, "terminal", "Terminal");
    let p = places(&out);
    let app = p[&1];
    let frame = p[&2];
    assert_eq!((frame.0, frame.1, frame.2, frame.3), (app.0 - BORDER, app.1 - TITLE_H, app.2 + 2 * BORDER, app.3 + TITLE_H + BORDER), "the frame goes around the window");
    assert!(app.4 & proto::window_states::ACTIVATED != 0, "the new window is focused");
    assert_eq!(last_stack(&out), Some(vec![2, 1]), "frame right below its window");
    assert!(out.iter().any(|m| matches!(m, Message::FocusWindow(f) if f.window == 1)));
    // The frame shows the title; the taskbar has a button for it and the clock.
    let fs = desk.surface_titled("frame:1");
    assert!(desk.t.has(fs, "Terminal"));
    let tb = desk.d.state.borrow().taskbar;
    assert!(desk.t.has(tb, "Terminal") && desk.t.has(tb, "09:41") && desk.t.has(tb, "Apps"));

    // A second window cascades on top.
    let out = desk.open(3, 4, "files", "Files");
    assert_eq!(last_stack(&out), Some(vec![2, 1, 4, 3]));
    assert_ne!(places(&out)[&3].0, app.0, "cascaded");
    // A click on the first window's frame (the compositor asks to activate the frame) raises window 1.
    let out = desk.step(vec![Message::WindowRequest(proto::WindowRequest { window: 2, action: proto::WindowAction::Activate, edges: 0 })]);
    assert_eq!(last_stack(&out), Some(vec![4, 3, 2, 1]));
    assert!(out.iter().any(|m| matches!(m, Message::FocusWindow(f) if f.window == 1)));

    // Minimize from the frame's button: both hidden, focus moves to Files.
    let mb = hit(&desk.d.app, fs, "min");
    let out = desk.step(click(fs, mb));
    assert!(out.iter().any(|m| matches!(m, Message::SetVisible(v) if v.window == 1 && !v.visible)));
    assert!(out.iter().any(|m| matches!(m, Message::SetVisible(v) if v.window == 2 && !v.visible)));
    assert!(out.iter().any(|m| matches!(m, Message::FocusWindow(f) if f.window == 3)));
    // Its taskbar button brings it back.
    let tbtn = hit(&desk.d.app, tb, "task1");
    let out = desk.step(click(tb, tbtn));
    assert!(out.iter().any(|m| matches!(m, Message::SetVisible(v) if v.window == 1 && v.visible)));
    assert!(out.iter().any(|m| matches!(m, Message::FocusWindow(f) if f.window == 1)));

    // Maximize: the window fills the work area under its title bar.
    let xb = hit(&desk.d.app, fs, "max");
    let out = desk.step(click(fs, xb));
    let p = places(&out);
    assert_eq!(p[&1], (0, TITLE_H, 640, 400 - desktop::TASKBAR_H - TITLE_H, proto::window_states::ACTIVATED | proto::window_states::MAXIMIZED));
    assert_eq!((p[&2].0, p[&2].1, p[&2].2, p[&2].3), (0, 0, 640, 400 - desktop::TASKBAR_H));
    let out = desk.step(click(fs, xb));
    assert_eq!((places(&out)[&1].0, places(&out)[&1].2), (app.0, app.2), "restored");

    // Close: CloseWindow for the app; WindowClosed drops its frame.
    let cb = hit(&desk.d.app, fs, "close");
    let out = desk.step(click(fs, cb));
    assert!(out.iter().any(|m| matches!(m, Message::CloseWindow(c) if c.window == 1)));
    let out = desk.step(vec![Message::WindowClosed(proto::WindowClosed { window: 1 })]);
    assert!(out.iter().any(|m| matches!(m, Message::SurfaceDestroy(d) if d.surface == fs)));
    assert!(out.iter().any(|m| matches!(m, Message::FocusWindow(f) if f.window == 3)));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn desktop_drag_moves_with_an_outline_and_alt_tab_switches() {
    let root = temp_root("desk2");
    let (mut desk, _) = Desk::new(&[], &root, 640, 400);
    desk.open(1, 2, "terminal", "Terminal");
    desk.open(3, 4, "editor", "Text Editor");
    let fs = desk.surface_titled("frame:1");
    let fr = desk.d.state.borrow().win(1).unwrap().rect;
    // Configure the frame so it renders and has hit ids.
    let (w, h) = (fr.2 + 2 * BORDER, fr.3 + TITLE_H + BORDER);
    let area = hit(&desk.d.app, fs, "frame");
    let out = desk.step(vec![pointer(fs, proto::PointerKind::Down, 100, 8, area, proto::Button::Left)]);
    let outline = out.iter().find_map(|m| if let Message::SurfaceCreate(s) = m { Some(s.clone()) } else { None }).expect("outline layer");
    assert_eq!((outline.role, outline.layer), (proto::SurfaceRole::Layer, proto::Layer::Overlay));
    // Moving draws only the outline; nothing is placed until the release.
    let out = desk.step(vec![pointer(fs, proto::PointerKind::Move, 150, 48, area, proto::Button::Left)]);
    assert!(places(&out).is_empty());
    let out = desk.step(vec![pointer(fs, proto::PointerKind::Up, 150, 48, area, proto::Button::Left)]);
    let p = places(&out);
    assert_eq!((p[&1].0, p[&1].1), (fr.0 + 50, fr.1 + 40));
    assert!(out.iter().any(|m| matches!(m, Message::SurfaceDestroy(d) if d.surface == outline.surface)));
    // Resize from the bottom-right corner.
    let out = desk.step(vec![pointer(fs, proto::PointerKind::Down, w - 1, h - 1, area, proto::Button::Left)]);
    assert!(out.iter().any(|m| matches!(m, Message::SurfaceCreate(_))));
    desk.step(vec![pointer(fs, proto::PointerKind::Move, w + 29, h + 19, area, proto::Button::Left)]);
    let out = desk.step(vec![pointer(fs, proto::PointerKind::Up, w + 29, h + 19, area, proto::Button::Left)]);
    let p = places(&out);
    assert_eq!((p[&1].2, p[&1].3), (fr.2 + 30, fr.3 + 20));

    // Alt+Tab twice: a switcher appears; after a pause the choice is focused.
    let st = desk.d.state.clone();
    assert_eq!(st.borrow().focused, Some(1));
    let out = desk.step(vec![Message::Chord(proto::Chord { id: desktop::CHORD_SWITCH })]);
    let sw = out.iter().find_map(|m| if let Message::SurfaceCreate(s) = m { Some(s.clone()) } else { None }).expect("switcher");
    assert_eq!(sw.title, "switcher");
    desk.step(vec![configure(sw.surface, 1, sw.w, sw.h, 0)]);
    assert!(desk.t.has(sw.surface, "Text Editor"));
    desk.sys.state.borrow_mut().now += 2000;
    let out = desk.step(vec![]);
    assert!(out.iter().any(|m| matches!(m, Message::FocusWindow(f) if f.window == 3)), "{out:?}");
    assert!(out.iter().any(|m| matches!(m, Message::SurfaceDestroy(d) if d.surface == sw.surface)));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn desktop_launcher_toasts_prompts_theme_and_restart() {
    let root = temp_root("desk3");
    std::fs::create_dir_all(root.join("apps/dev.pat.game")).unwrap();
    std::fs::write(root.join("apps/dev.pat.game/app.toml"), "name = \"Pat's Game\"\n").unwrap();
    let (mut desk, _) = Desk::new(&["--restart", "--launch", "terminal"], &root, 640, 400);
    let all = Message::decode_all(&desk.m.state.borrow().sent).unwrap_or_default();
    assert!(!all.iter().any(|m| matches!(m, Message::Launch(_))), "a restarted shell doesn't relaunch the session");
    let tb = desk.d.state.borrow().taskbar;

    // The Apps button opens the launcher popup above the taskbar.
    let apps = hit(&desk.d.app, tb, "apps");
    let out = desk.step(click(tb, apps));
    let pop = out.iter().find_map(|m| if let Message::SurfaceCreate(s) = m { Some(s.clone()) } else { None }).expect("launcher popup");
    assert_eq!((pop.role, pop.parent), (proto::SurfaceRole::Popup, tb));
    assert!(pop.y < 0, "above the taskbar");
    desk.step(vec![configure(pop.surface, 1, pop.w, pop.h, 0)]);
    for name in ["Terminal", "Files", "Text Editor", "Settings", "Notes", "Visual Programmer", "Pat's Game"] {
        assert!(desk.t.has(pop.surface, name), "launcher lists {name}");
    }
    let row = desk.d.app.core.window(pop.surface).unwrap().hit_id_of_row("apps", 3).unwrap();
    let out = desk.step(click(pop.surface, row));
    assert!(out.iter().any(|m| matches!(m, Message::Launch(l) if l.app_id == "settings")));
    assert!(out.iter().any(|m| matches!(m, Message::SurfaceDestroy(d) if d.surface == pop.surface)));
    // Apps installed later (the App Store) appear the next time the launcher opens; the Text Editor
    // system app (open with) is the core Text Editor's twin and isn't listed twice.
    for (id, name) in [("dev.new.app", "New App"), ("dev.ecm.editor", "Text Editor")] {
        std::fs::create_dir_all(root.join("apps").join(id)).unwrap();
        std::fs::write(root.join("apps").join(id).join("app.toml"), format!("name = \"{name}\"
")).unwrap();
    }
    let out = desk.step(click(tb, apps));
    let pop = out.iter().find_map(|m| if let Message::SurfaceCreate(s) = m { Some(s.clone()) } else { None }).expect("launcher popup again");
    desk.step(vec![configure(pop.surface, 1, pop.w, pop.h, 0)]);
    assert!(desk.t.has(pop.surface, "New App"), "a newly installed app is listed");
    let items = desk.d.state.borrow().launch_items.clone();
    assert!(items.iter().any(|i| i.program == "dev.new.app" && i.label == "New App"));
    assert!(!items.iter().any(|i| i.program == "dev.ecm.editor"), "{items:?}");
    let row = desk.d.app.core.window(pop.surface).unwrap().hit_id_of_row("apps", items.iter().position(|i| i.program == "dev.new.app").unwrap()).unwrap();
    let out = desk.step(click(pop.surface, row));
    assert!(out.iter().any(|m| matches!(m, Message::Launch(l) if l.app_id == "dev.new.app")), "Launch by app id (the app manager starts it)");

    // A notification becomes a toast; clicking it invokes it.
    let out = desk.step(vec![Message::NotificationPosted(proto::NotificationPosted {
        nid: 7,
        app_id: "dev.ecm.editor".into(),
        tag: "saved".into(),
        title: "Saved".into(),
        body: "notes.txt".into(),
        icon: String::new(),
        actions: vec!["Open".into()],
    })]);
    let toast = out.iter().find_map(|m| if let Message::SurfaceCreate(s) = m { Some(s.clone()) } else { None }).expect("toast");
    assert_eq!((toast.layer, toast.title.as_str()), (proto::Layer::Overlay, "toast"));
    desk.step(vec![configure(toast.surface, 1, toast.w, toast.h, 0)]);
    assert!(desk.t.has(toast.surface, "Saved") && desk.t.has(toast.surface, "notes.txt"));
    let act = hit(&desk.d.app, toast.surface, "act0");
    let out = desk.step(click(toast.surface, act));
    assert!(out.iter().any(|m| matches!(m, Message::NotificationInvoke(n) if n.nid == 7 && n.action == "Open")));
    assert!(out.iter().any(|m| matches!(m, Message::SurfaceDestroy(d) if d.surface == toast.surface)));

    // A permission prompt is a dialog; Allow with "remember" answers it.
    let out = desk.step(vec![Message::PermissionPrompt(proto::PermissionPrompt { req: 5, app_id: "dev.pat.game".into(), permission: "network".into() })]);
    let pr = out.iter().find_map(|m| if let Message::SurfaceCreate(s) = m { Some(s.clone()) } else { None }).expect("prompt");
    desk.step(vec![configure(pr.surface, 1, pr.w, pr.h, 0)]);
    // The question names the app by its app.toml name, not its id.
    assert!(desk.t.has(pr.surface, "Allow Pat's Game to use network?"));
    let rem = hit(&desk.d.app, pr.surface, "remember");
    desk.step(click(pr.surface, rem));
    let allow = hit(&desk.d.app, pr.surface, "allow");
    let out = desk.step(click(pr.surface, allow));
    assert!(out.iter().any(|m| matches!(m, Message::AnswerPermission(a) if a.req == 5 && a.allow && a.remember)));

    // Settings writes a new theme: the desktop sends it within a poll.
    std::fs::create_dir_all(root.join("etc")).unwrap();
    std::fs::write(root.join("etc/ui.toml"), "theme = \"ecm-light\"\n").unwrap();
    desk.sys.state.borrow_mut().now += 1500;
    let out = desk.step(vec![]);
    let theme = out.iter().find_map(|m| if let Message::SetTheme(t) = m { Some(t.theme.clone()) } else { None }).expect("SetTheme");
    assert_eq!(theme.name, "ecm-light");
    assert_eq!(theme.get("bg"), Some(ecm_ui::Theme::named("ecm-light").unwrap().bg));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn compact_displays_maximize_windows() {
    let root = temp_root("desk4");
    let (mut desk, _) = Desk::new(&[], &root, 320, 200);
    let out = desk.open(1, 2, "settings", "Settings");
    let p = places(&out);
    assert_eq!((p[&1].0, p[&1].1, p[&1].2), (0, TITLE_H, 320));
    assert!(p[&1].4 & proto::window_states::MAXIMIZED != 0);
    let _ = std::fs::remove_dir_all(root);
}

// ------------------------------------------------------------------ terminal

#[test]
fn terminal_shows_the_session_types_into_it_and_pastes() {
    let m = mock("terminal", 640, 400);
    let sys = MockSys::new(temp_root("term"));
    sys.state.borrow_mut().session_out = b"Terminal OS - logged in as user.\r\n/ > ".to_vec();
    let mut app = terminal::app(Box::new(m.clone()), Box::new(sys.clone())).unwrap();
    let mut t = Texts::default();
    run(&mut app, &m, &mut t, vec![configure(1, 1, 644, 388, proto::window_states::ACTIVATED)]);
    assert_eq!(sys.state.borrow().session_size, Some((80, 24)), "the window size is the session's size");
    run(&mut app, &m, &mut t, vec![]);
    assert!(t.has(1, "logged in as user."), "{:?}", t.of(1));
    assert!(t.has(1, "/ >"));
    // Typed text and Enter go to the session.
    run(&mut app, &m, &mut t, vec![text(1, "ls"), key(1, 0x28, 0)]);
    assert_eq!(sys.state.borrow().session_in, b"ls\r");
    // Ctrl+C is the interrupt byte; arrows are escape sequences.
    run(&mut app, &m, &mut t, vec![key(1, 0x06, proto::mods::CTRL), key(1, 0x52, 0)]);
    assert!(sys.state.borrow().session_in.ends_with(b"\x03\x1b[A"));
    // Ctrl+Shift+V asks for the clipboard; the answer is typed, newlines as Enter.
    let out = run(&mut app, &m, &mut t, vec![key(1, 0x19, proto::mods::CTRL | proto::mods::SHIFT)]);
    assert!(out.iter().any(|x| matches!(x, Message::ClipboardRequest(r) if r.mime == TEXT_MIME)));
    run(&mut app, &m, &mut t, vec![Message::ClipboardData(proto::ClipboardData { mime: TEXT_MIME.into(), data: b"echo pasted\n".to_vec() })]);
    assert!(sys.state.borrow().session_in.ends_with(b"echo pasted\r"));
    // Output arrives and is drawn; selecting a line and Ctrl+Shift+C copies it.
    sys.state.borrow_mut().session_out = b"\r\npasted\r\n/ > ".to_vec();
    run(&mut app, &m, &mut t, vec![]);
    assert!(t.has(1, "pasted"));
    let h = hit(&app, 1, "term");
    run(&mut app, &m, &mut t, vec![pointer(1, proto::PointerKind::Down, 2, 2 + 16, h, proto::Button::Left)]);
    run(&mut app, &m, &mut t, vec![pointer(1, proto::PointerKind::Move, 2 + 6 * 8, 2 + 16, h, proto::Button::Left)]);
    run(&mut app, &m, &mut t, vec![pointer(1, proto::PointerKind::Up, 2 + 6 * 8, 2 + 16, h, proto::Button::Left)]);
    let out = run(&mut app, &m, &mut t, vec![key(1, 0x06, proto::mods::CTRL | proto::mods::SHIFT)]);
    let copied = out.iter().find_map(|x| if let Message::ClipboardSet(c) = x { Some(String::from_utf8_lossy(&c.data).into_owned()) } else { None });
    assert!(copied.is_some(), "Ctrl+Shift+C copies the selection");
    // The session ends: the window says so.
    sys.state.borrow_mut().session_exited = true;
    run(&mut app, &m, &mut t, vec![]);
    assert!(t.has(1, "session ended"));
}

// ------------------------------------------------------------------ editor

#[test]
fn editor_opens_copies_finds_and_saves() {
    let root = temp_root("edit");
    std::fs::write(root.join("hello.py"), "print('hello')\nprint('world')\n").unwrap();
    let m = mock("editor", 640, 400);
    let sys = MockSys::new(&root);
    let mut app = editor::app(Box::new(m.clone()), Box::new(sys), Some("/hello.py")).unwrap();
    let mut t = Texts::default();
    let out = run(&mut app, &m, &mut t, vec![configure(1, 1, 600, 360, proto::window_states::ACTIVATED)]);
    assert!(t.has(1, "print('hello')") && t.has(1, "print('world')"));
    assert!(t.has(1, "Opened /hello.py"));
    let _ = out;
    // Select all and copy: the clipboard gets the whole file.
    let out = run(&mut app, &m, &mut t, vec![key(1, 0x04, proto::mods::CTRL), key(1, 0x06, proto::mods::CTRL)]);
    let set = out.iter().find_map(|x| if let Message::ClipboardSet(c) = x { Some(c.clone()) } else { None }).expect("ClipboardSet");
    assert_eq!(String::from_utf8(set.data).unwrap(), "print('hello')\nprint('world')\n");
    // Edit: the title gets a star; Ctrl+S saves and the star goes.
    let out = run(&mut app, &m, &mut t, vec![key(1, 0x4D, proto::mods::CTRL), text(1, "# end")]);
    assert!(out.iter().any(|x| matches!(x, Message::SurfaceSetTitle(s) if s.title == "*hello.py - Text Editor")));
    let out = run(&mut app, &m, &mut t, vec![key(1, 0x16, proto::mods::CTRL)]);
    assert!(out.iter().any(|x| matches!(x, Message::SurfaceSetTitle(s) if s.title == "hello.py - Text Editor")));
    assert!(std::fs::read_to_string(root.join("hello.py")).unwrap().ends_with("# end"));
    // Find: Ctrl+F, type, Enter selects the match.
    run(&mut app, &m, &mut t, vec![key(1, 0x09, proto::mods::CTRL)]);
    run(&mut app, &m, &mut t, vec![text(1, "world"), key(1, 0x28, 0)]);
    let doc = app.core.window(1).unwrap().state("doc").unwrap().clone();
    assert_eq!(doc.selected_text(), "world");
    assert!(t.has(1, "Found at line 2"));
    let _ = std::fs::remove_dir_all(root);
}

// ------------------------------------------------------------------ files

#[test]
fn files_lists_opens_and_manages_files() {
    let root = temp_root("files");
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(root.join("docs/todo.txt"), "milk").unwrap();
    std::fs::write(root.join("run.py"), "print(1)").unwrap();
    let m = mock("files", 640, 400);
    let sys = MockSys::new(&root);
    let mut app = files::app(Box::new(m.clone()), Box::new(sys), None).unwrap();
    let mut t = Texts::default();
    run(&mut app, &m, &mut t, vec![configure(1, 1, 600, 360, proto::window_states::ACTIVATED)]);
    assert!(t.has(1, "docs") && t.has(1, "run.py") && t.has(1, "2 items"));
    // Enter on run.py (row 1: folders first) asks the compositor to open it ("open with": the app manager picks the app).
    run(&mut app, &m, &mut t, vec![key(1, 0x51, 0), key(1, 0x51, 0)]);
    let out = run(&mut app, &m, &mut t, vec![key(1, 0x28, 0)]);
    assert!(out.iter().any(|x| matches!(x, Message::Open(o) if o.path == "/run.py")), "{out:?}");
    assert!(!out.iter().any(|x| matches!(x, Message::Launch(_))), "Files no longer picks the program itself: {out:?}");
    // Enter on the folder opens it.
    run(&mut app, &m, &mut t, vec![key(1, 0x4A, 0), key(1, 0x28, 0)]);
    assert!(t.has(1, "todo.txt"));
    // Copy todo.txt, go up, paste: a copy appears.
    run(&mut app, &m, &mut t, vec![key(1, 0x4A, 0)]);
    let out = run(&mut app, &m, &mut t, vec![key(1, 0x06, proto::mods::CTRL)]);
    assert!(out.iter().any(|x| matches!(x, Message::ClipboardSet(c) if c.data == b"/docs/todo.txt")));
    run(&mut app, &m, &mut t, vec![key(1, 0x2A, 0)]);
    run(&mut app, &m, &mut t, vec![key(1, 0x19, proto::mods::CTRL)]);
    assert_eq!(std::fs::read_to_string(root.join("todo.txt")).unwrap(), "milk");
    // New folder through the dialog.
    let mk = hit(&app, 1, "mkdir");
    run(&mut app, &m, &mut t, click(1, mk));
    assert!(t.has(1, "New folder"));
    run(&mut app, &m, &mut t, vec![text(1, "projects"), key(1, 0x28, 0)]);
    assert!(root.join("projects").is_dir());
    // Rename (F2) and delete (Delete, confirmed) the copy.
    let i = app.core.window(1).unwrap().state("list").and_then(|s| s.selected);
    let _ = i;
    let rows: Vec<String> = ecm_apps::sys::list_dir(&MockSys::new(&root), "/").unwrap().into_iter().map(|e| e.name).collect();
    let idx = rows.iter().position(|n| n == "todo.txt").unwrap();
    app.core.window_mut(1).unwrap().state_mut("list").selected = Some(idx);
    run(&mut app, &m, &mut t, vec![key(1, 0x3B, 0)]);
    run(&mut app, &m, &mut t, vec![key(1, 0x04, proto::mods::CTRL), text(1, "shopping.txt"), key(1, 0x28, 0)]);
    assert!(root.join("shopping.txt").exists() && !root.join("todo.txt").exists());
    let rows: Vec<String> = ecm_apps::sys::list_dir(&MockSys::new(&root), "/").unwrap().into_iter().map(|e| e.name).collect();
    let idx = rows.iter().position(|n| n == "shopping.txt").unwrap();
    app.core.window_mut(1).unwrap().state_mut("list").selected = Some(idx);
    run(&mut app, &m, &mut t, vec![key(1, 0x4C, 0)]);
    assert!(t.has(1, "Delete shopping.txt?"));
    let ok = hit(&app, 1, "ok");
    run(&mut app, &m, &mut t, click(1, ok));
    assert!(!root.join("shopping.txt").exists());
    let _ = std::fs::remove_dir_all(root);
}

// ------------------------------------------------------------------ settings

fn settings_sys(root: &PathBuf) -> MockSys {
    let sys = MockSys::new(root);
    {
        let mut s = sys.state.borrow_mut();
        s.wifi.present = true;
        s.networks = vec![
            Network { ssid: "ecm-cafe".into(), signal_dbm: -52, channel: 6, security: "wpa2-psk".into() },
            Network { ssid: "library".into(), signal_dbm: -78, channel: 11, security: "open".into() },
        ];
        s.join_ok = vec![("ecm-cafe".into(), Some("letmein123".into()))];
        s.display = Some(DisplayState { tier: 2, width: 640, height: 400, scale: 1, refresh_hz: 30, text_cols: 80, text_rows: 25, colors: "RGBA".into(), touch: false });
        s.cells = vec![CellState { name: "lte_modem_0".into(), tech: "lte".into(), state: "connected".into(), message: "on ecm-mobile cell 1".into(), sim: "ecm-mobile".into(), carrier: "ecm-mobile".into(), bars: 3, signal: "-81 dBm".into(), ..Default::default() }];
    }
    sys
}

#[test]
fn settings_joins_wifi_without_a_command_and_switches_the_theme() {
    let root = temp_root("settings");
    let m = mock("settings", 640, 400);
    let sys = settings_sys(&root);
    let mut app = settings::app(Box::new(m.clone()), Box::new(sys.clone()), None).unwrap();
    let mut t = Texts::default();
    run(&mut app, &m, &mut t, vec![configure(1, 1, 600, 360, proto::window_states::ACTIVATED)]);
    assert!(t.has(1, "Not connected"));
    let scan = hit(&app, 1, "scan");
    run(&mut app, &m, &mut t, click(1, scan));
    assert!(t.has(1, "ecm-cafe") && t.has(1, "library"));
    // A wrong password: the fix-it message, no connection.
    let row0 = app.core.window(1).unwrap().hit_id_of_row("nets", 0).unwrap();
    run(&mut app, &m, &mut t, click(1, row0));
    run(&mut app, &m, &mut t, vec![text(1, "wrongpass")]);
    let join = hit(&app, 1, "join");
    run(&mut app, &m, &mut t, click(1, join));
    assert!(t.has(1, "the password for 'ecm-cafe' is wrong"));
    // The right one.
    let pass = hit(&app, 1, "pass");
    run(&mut app, &m, &mut t, click(1, pass));
    run(&mut app, &m, &mut t, vec![key(1, 0x04, proto::mods::CTRL), text(1, "letmein123"), key(1, 0x28, 0)]);
    assert!(t.has(1, "Connected. Address 192.168.60.23/24"), "{:?}", t.of(1));
    assert!(t.has(1, "Connected to 'ecm-cafe'"));
    assert!(sys.state.borrow().wifi.connected);

    // Appearance: picking Light writes /etc/ui.toml.
    let sec = app.core.window(1).unwrap().hit_id_of_row("sections", 3).unwrap();
    run(&mut app, &m, &mut t, click(1, sec));
    assert!(t.has(1, "Appearance") && t.has(1, "Light"));
    let light = app.core.window(1).unwrap().hit_id_of_row("themes", 1).unwrap();
    run(&mut app, &m, &mut t, click(1, light));
    let toml = std::fs::read_to_string(root.join("etc/ui.toml")).unwrap();
    assert_eq!(ecm_apps::uitoml::get(&toml, "theme").as_deref(), Some("ecm-light"));
    // The compositor's ThemeChanged restyles this app's window at once.
    let light_bg = ecm_ui::Theme::named("ecm-light").unwrap();
    let out = run(&mut app, &m, &mut t, vec![Message::ThemeChanged(proto::ThemeChanged { theme: light_bg.to_wire() })]);
    let fills: Vec<u32> = out
        .iter()
        .filter_map(|x| if let Message::TreeOps(o) = x { Some(o.ops.clone()) } else { None })
        .flatten()
        .filter_map(|op| match op {
            TreeOp::Set { props, .. } | TreeOp::Create { props, .. } => props.iter().find_map(|p| if let Prop::Fill(c) = p { Some(*c) } else { None }),
            _ => None,
        })
        .collect();
    assert!(fills.contains(&light_bg.bg), "the window background restyled");
    // Display and cellular pages.
    let sec = app.core.window(1).unwrap().hit_id_of_row("sections", 2).unwrap();
    run(&mut app, &m, &mut t, click(1, sec));
    assert!(t.has(1, "Color LCD (Tier 2)") && t.has(1, "640 x 400 pixels"));
    let sec = app.core.window(1).unwrap().hit_id_of_row("sections", 1).unwrap();
    run(&mut app, &m, &mut t, click(1, sec));
    assert!(t.has(1, "lte_modem_0 (LTE modem)") && t.has(1, "on ecm-mobile cell 1") && t.has(1, "|||."));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn settings_adapts_to_compact_medium_and_wide() {
    for (w, sidebar) in [(300, false), (600, true), (900, true)] {
        let root = temp_root(&format!("sz{w}"));
        let m = mock("settings", w, 400);
        let sys = settings_sys(&root);
        let mut app = settings::app(Box::new(m.clone()), Box::new(sys), None).unwrap();
        let mut t = Texts::default();
        run(&mut app, &m, &mut t, vec![configure(1, 1, w, 300, proto::window_states::ACTIVATED)]);
        let win = app.core.window(1).unwrap();
        assert_eq!(win.hit_id_of_row("sections", 0).is_some(), sidebar, "sidebar at {w}");
        assert_eq!(win.hit_id_of("sections_menu").is_some(), !sidebar, "Sections menu at {w}");
        if w == 900 {
            assert_eq!(win.rect_of("sections").unwrap().w, 170);
        }
        // Everything fits the window, with the network list and the Join row too.
        let scan = hit(&app, 1, "scan");
        run(&mut app, &m, &mut t, click(1, scan));
        let row0 = app.core.window(1).unwrap().hit_id_of_row("nets", 0).unwrap();
        run(&mut app, &m, &mut t, click(1, row0));
        let win = app.core.window(1).unwrap();
        for id in ["page", "nets", "join", "pass"] {
            let r = win.rect_of(id).unwrap_or_else(|| panic!("{id} at {w}"));
            assert!(r.x >= 0 && r.x + r.w <= w, "{id} at {w} wide: {r:?}");
        }
        let _ = std::fs::remove_dir_all(root);
    }
}

/// A FakeCtl the test can still look at after Settings took it.
struct SharedCtl(std::rc::Rc<std::cell::RefCell<ecm_apps_ui::appctl::FakeCtl>>);

impl ecm_apps_ui::appctl::AppCtl for SharedCtl {
    fn ctl(&mut self, words: &[&str]) -> Option<(i32, String)> {
        self.0.borrow_mut().ctl(words)
    }
}

/// Settings -> Apps (P4's AppsSection in the Settings app): listed in the
/// sidebar and the Sections menu, it shows the installed apps and the chosen
/// one's permissions, and a toggle goes through app_ctl; it fits at compact,
/// medium and wide. Without an app manager it says so.
#[test]
fn settings_apps_section_at_every_size() {
    use ecm_apps_ui::appctl::FakeCtl;
    for w in [300, 600, 900] {
        let root = temp_root(&format!("apps{w}"));
        let mut f = FakeCtl::default();
        f.add("dev.ecm.netcheck", "Net Check", "1.0.0", &["network"], &[".txt"]);
        f.add("dev.ecm.store", "App Store", "1.0.0", &["packages", "network"], &[]);
        let ctl = std::rc::Rc::new(std::cell::RefCell::new(f));
        let m = mock("settings", w, 400);
        let mut app = settings::app_with(Box::new(m.clone()), Box::new(settings_sys(&root)), Some("apps"), Box::new(SharedCtl(ctl.clone()))).unwrap();
        let mut t = Texts::default();
        run(&mut app, &m, &mut t, vec![configure(1, 1, w, 400, proto::window_states::ACTIVATED)]);
        assert!(t.has(1, "Installed apps") && t.has(1, "Net Check") && t.has(1, "App Store"), "at {w}: {:?}", t.of(1));
        let row = app.core.window(1).unwrap().hit_id_of_row("apps_list", 1).unwrap();
        run(&mut app, &m, &mut t, click(1, row));
        assert!(t.has(1, "Use the network"), "Net Check's permission in plain words at {w}");
        let tg = hit(&app, 1, "perm:network");
        run(&mut app, &m, &mut t, click(1, tg));
        assert!(ctl.borrow().log.iter().any(|l| l == "perm grant dev.ecm.netcheck network"), "{:?}", ctl.borrow().log);
        let win = app.core.window(1).unwrap();
        for id in ["page", "apps_list", "perm:network"] {
            let r = win.rect_of(id).unwrap_or_else(|| panic!("{id} at {w}"));
            assert!(r.x >= 0 && r.x + r.w <= w, "{id} at {w} wide: {r:?}");
        }
        let _ = std::fs::remove_dir_all(root);
    }
    // The section is in the sidebar; without an app manager (host default) it says so.
    let root = temp_root("apps-none");
    let m = mock("settings", 600, 400);
    let mut app = settings::app(Box::new(m.clone()), Box::new(settings_sys(&root)), None).unwrap();
    let mut t = Texts::default();
    run(&mut app, &m, &mut t, vec![configure(1, 1, 600, 360, proto::window_states::ACTIVATED)]);
    let sec = app.core.window(1).unwrap().hit_id_of_row("sections", settings::APPS).unwrap();
    run(&mut app, &m, &mut t, click(1, sec));
    assert!(t.has(1, "no app manager on this computer"), "{:?}", t.of(1));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn settings_sound_picks_default_devices_and_sets_the_gain() {
    use ecm_apps::sys::SoundDevice;
    let root = temp_root("sound");
    let m = mock("settings", 600, 400);
    let sys = settings_sys(&root);
    {
        let mut s = sys.state.borrow_mut();
        s.sound = vec![
            SoundDevice { name: "left".into(), input: false, volume: 80 },
            SoundDevice { name: "back".into(), input: true, volume: 50 },
            SoundDevice { name: "top".into(), input: true, volume: 40 },
        ];
        s.mic_levels = vec![("back".into(), -23), ("top".into(), -40)];
    }
    let mut app = settings::app(Box::new(m.clone()), Box::new(sys.clone()), Some("sound")).unwrap();
    let mut t = Texts::default();
    run(&mut app, &m, &mut t, vec![configure(1, 1, 600, 380, proto::window_states::ACTIVATED)]);
    for want in ["Sound", "Output", "left", "Input", "back", "top", "Input volume", "Input level"] {
        assert!(t.has(1, want), "{want}: {:?}", t.of(1));
    }
    // Pick the second microphone: it becomes the default in /etc/sound.conf.
    let first = app.core.window(1).unwrap().hit_id_of_row("inputs", 1).unwrap();
    run(&mut app, &m, &mut t, click(1, first));
    let conf = std::fs::read_to_string(root.join("etc/sound.conf")).unwrap();
    assert_eq!(ecm_audio::parse_sound_conf(&conf, "input").as_deref(), Some("top"), "{conf}");
    assert!(t.has(1, "Default input: top"), "{:?}", t.of(1));
    // Drag the input volume to the far right: the microphone's gain is 100.
    let vol = hit(&app, 1, "input_volume");
    run(&mut app, &m, &mut t, vec![pointer(1, proto::PointerKind::Down, 10_000, 5, vol, proto::Button::Left)]);
    run(&mut app, &m, &mut t, vec![pointer(1, proto::PointerKind::Up, 10_000, 5, vol, proto::Button::Left)]);
    assert!(sys.state.borrow().log.contains(&"gain top 100".to_string()), "{:?}", sys.state.borrow().log);
    assert_eq!(settings::meter(-30), "||||||||||..........");
    assert_eq!(settings::meter(-99), "....................");
    let _ = std::fs::remove_dir_all(root);
}

/// What celld says on a phone attached to cell 1 of cell-red (two cells heard).
fn celld_cell() -> CellState {
    let st = celld::ctl::Status::parse(
        "state connected\nmessage on cell-red cell 1\nmodem module_1\nprofile lte\nsim_carrier cell-red\nimsi 310150000000001\ncarrier cell-red\ncell 1\nchannel 1\nrssi -62.5\nsinr 18.4\ncqi 12\nbars 4\nip 100.65.2.2/24\ngateway 100.65.2.1\ndns 100.65.0.10\nhandovers 1\ntx_bytes 2048\nrx_bytes 1300\nroaming off\n",
    );
    let cells = celld::ctl::parse_cells(
        "cell 1 carrier cell-red profile lte channel 1 rssi -62.5 sinr 18.4 age 40 serving\ncell 7 carrier cell-blue profile lte channel 3 rssi -91.0 sinr 2.0 age 900 note other_carrier\n",
    );
    CellState::from_celld(&st, &cells)
}

#[test]
fn settings_cellular_shows_celld_and_sends_roaming_and_power() {
    for w in [300, 600] {
        let root = temp_root(&format!("cell{w}"));
        let m = mock("settings", w, 400);
        let sys = settings_sys(&root);
        sys.state.borrow_mut().cells = vec![celld_cell()];
        let mut app = settings::app(Box::new(m.clone()), Box::new(sys.clone()), Some("cellular")).unwrap();
        let mut t = Texts::default();
        run(&mut app, &m, &mut t, vec![configure(1, 1, w, 380, proto::window_states::ACTIVATED)]);
        for want in [
            "module_1 (LTE modem)",
            "on cell-red cell 1",
            "cell-red (IMSI 310150000000001)",
            "cell 1, channel 1",
            "-62.5 dBm, SINR 18.4 dB, CQI 12",
            "||||",
            "100.65.2.2/24 via 100.65.2.1",
            "sent 2.0 KB, received 1.3 KB, 1 handover",
            "Cells heard",
            "cell-blue cell 7, ch 3, -91 dBm (other carrier)",
            "Data roaming",
            "Turn off",
        ] {
            assert!(t.has(1, want), "{want} at {w}: {:?}", t.of(1));
        }
        // Everything fits the window.
        let win = app.core.window(1).unwrap();
        for id in ["page", "roaming", "cells", "cell_power"] {
            let r = win.rect_of(id).unwrap_or_else(|| panic!("{id} at {w}"));
            assert!(r.x >= 0 && r.x + r.w <= w, "{id} at {w} wide: {r:?}");
        }
        // The roaming switch asks celld (which saves it in /etc/celld.toml).
        let roaming = hit(&app, 1, "roaming");
        run(&mut app, &m, &mut t, click(1, roaming));
        assert!(sys.state.borrow().log.contains(&"cell roaming on".to_string()), "{:?}", sys.state.borrow().log);
        assert!(sys.state.borrow().cells[0].roaming);
        assert!(t.has(1, "celld: roaming on"), "{:?}", t.of(1));
        // Turn off: celld detaches; the button offers Turn on.
        let power = hit(&app, 1, "cell_power");
        run(&mut app, &m, &mut t, click(1, power));
        assert!(sys.state.borrow().log.contains(&"cell down".to_string()));
        assert!(t.has(1, "Turn on") && t.has(1, "switched off"), "{:?}", t.of(1));
        let _ = std::fs::remove_dir_all(root);
    }
}
