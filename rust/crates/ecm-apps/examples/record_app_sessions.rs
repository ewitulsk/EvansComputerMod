//! Records the P3 golden sessions that the Java `UiGoldenTest` replays
//! through the compositor and rasterizer: the real desktop shell and the real
//! apps, driven through recording connections against a mock computer, with
//! the compositor's answers (window ids, configure serials, pointer events)
//! predicted here.
//!
//!   cargo run -p ecm-apps --example record_app_sessions -- <out dir>
//!   (the checked-in copies live in src/test/resources/ui-sessions/)
//!
//! * `settings_compact` (320x200): the desktop maximizes Settings; compact layout.
//! * `settings_medium` (640x400): Settings in a cascaded frame; sidebar layout.
//! * `settings_wide` (960x480): Settings maximized by a click on its frame's
//!   maximize button (hit-tested by the compositor); wide layout.
//! * `settings_apps` (640x400), `settings_apps_compact` (320x200) and
//!   `settings_apps_wide` (960x480, list and app side by side):
//!   Settings -> Apps (P4's `AppsSection`) with Net Check selected: its
//!   permissions, the file it was given and the types it opens.
//! * `settings_cellular` (640x400) and `settings_cellular_compact` (320x200):
//!   Settings -> Cellular with celld's status (Cellular v2): the SIM's identity,
//!   the serving cell, signal, address, data, the roaming switch, cells heard.
//! * `desktop_apps` (640x400): Files and the Text Editor in frames, a toast,
//!   and the launcher opened from the taskbar's Apps button (the core apps
//!   with the Visual Programmer, then the installed App Store).
//!
//! The file format is the one `record_ui_sessions` (ecm-ui) documents.

use ecm_apps::desktop::{self, frame_rect, Desktop};
use ecm_apps::sys::{CellState, DisplayState, MockSys, Network};
use ecm_apps::{editor, files, settings};
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::App;
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;

struct Session {
    out: Vec<u8>,
}

impl Session {
    fn new(w: u32, h: u32, scale: u32) -> Session {
        let mut out = b"ECMUISS1".to_vec();
        for v in [w, h, scale] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        Session { out }
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.out.extend_from_slice(b);
    }
    fn open(&mut self, conn: u32, pid: u32, role: u8, name: &str) {
        self.out.push(1);
        self.u32(conn);
        self.u32(pid);
        self.out.push(role);
        self.out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.out.extend_from_slice(name.as_bytes());
    }
    fn send(&mut self, conn: u32, b: &[u8]) {
        if b.is_empty() {
            return;
        }
        Message::decode_all(b).expect("only valid messages are recorded");
        self.out.push(2);
        self.u32(conn);
        self.bytes(b);
    }
    fn pixels(&mut self, conn: u32, target: u32, rects: &[[i32; 4]], b: &[u8]) {
        self.out.push(3);
        self.u32(conn);
        self.u32(target);
        self.u32(rects.len() as u32);
        for r in rects {
            for v in r {
                self.i32(*v);
            }
        }
        self.bytes(b);
    }
    fn upload(&mut self, conn: u32, res: u32, kind: u32, b: &[u8]) {
        self.out.push(4);
        self.u32(conn);
        self.u32(res);
        self.u32(kind);
        self.bytes(b);
    }
    fn advance(&mut self, ms: u32) {
        self.out.push(5);
        self.u32(ms);
    }
    fn snap(&mut self, name: &str) {
        self.out.push(6);
        self.out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.out.extend_from_slice(name.as_bytes());
    }
    fn pointer(&mut self, kind: proto::PointerKind, x: i32, y: i32, buttons: u8, button: proto::Button, conn: u32, hit: u32) {
        self.out.push(7);
        self.out.push(kind as u8);
        self.i32(x);
        self.i32(y);
        self.out.push(buttons);
        self.out.push(button as u8);
        self.i32(0);
        self.u32(conn);
        self.u32(hit);
    }
}

type Shared = Rc<RefCell<Session>>;

/// A connection that writes what the program sends into the session as it
/// happens (sends, uploads and pixels keep their order) and returns the
/// compositor's predicted answers.
struct RecConn {
    s: Shared,
    conn: u32,
    incoming: Rc<RefCell<VecDeque<Vec<u8>>>>,
}

impl Connection for RecConn {
    fn send(&mut self, bytes: &[u8]) -> Result<(), ConnError> {
        self.s.borrow_mut().send(self.conn, bytes);
        Ok(())
    }
    fn recv(&mut self, _timeout_ms: i32) -> Result<Vec<u8>, ConnError> {
        Ok(self.incoming.borrow_mut().pop_front().unwrap_or_default())
    }
    fn put_pixels(&mut self, target: u32, pixels: &[u8], damage: &[[i32; 4]]) -> Result<(), ConnError> {
        self.s.borrow_mut().pixels(self.conn, target, damage, pixels);
        Ok(())
    }
    fn upload(&mut self, resource: u32, kind: i32, data: &[u8]) -> Result<(), ConnError> {
        self.s.borrow_mut().upload(self.conn, resource, kind as u32, data);
        Ok(())
    }
}

struct Peer {
    incoming: Rc<RefCell<VecDeque<Vec<u8>>>>,
}

impl Peer {
    fn push(&self, msgs: &[Message]) {
        self.incoming.borrow_mut().push_back(proto::encode_all(msgs));
    }
}

fn connect(s: &Shared, conn: u32, pid: u32, role: u8, name: &str, w: i32, h: i32) -> (Box<dyn Connection>, Peer) {
    s.borrow_mut().open(conn, pid, role, name);
    let incoming = Rc::new(RefCell::new(VecDeque::new()));
    let peer = Peer { incoming: incoming.clone() };
    let mut wl = ecm_ui::samples::welcome(false);
    if let Message::Welcome(ref mut x) = wl {
        x.app_id = name.into();
        x.display = proto::DisplayInfo { w, h, scale: 1, size_class: proto::SizeClass::for_width(w), tier: 2, device: proto::Device::Terminal };
    }
    peer.push(&[wl]);
    (Box::new(RecConn { s: s.clone(), conn, incoming }), peer)
}

fn configure(surface: u32, serial: u32, w: i32, h: i32, states: u32) -> Message {
    Message::Configure(proto::Configure { surface, serial, w, h, states })
}

fn opened(window: u32, pid: u32, app: &str, title: &str) -> Message {
    Message::WindowOpened(proto::WindowOpened { window, pid, app_id: app.into(), title: title.into(), icon: String::new(), min_w: 0, min_h: 0, max_w: 0, max_h: 0, parent: 0 })
}

fn root(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("ecm-golden-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// The computer the sessions show: Wi-Fi joined, two networks around, a Tier 2 panel, an LTE modem.
fn computer(dir: &PathBuf) -> MockSys {
    let sys = MockSys::new(dir);
    {
        let mut s = sys.state.borrow_mut();
        s.clock = Some((9, 41));
        s.wifi.present = true;
        s.wifi.connected = true;
        s.wifi.ssid = "ecm-cafe".into();
        s.wifi.signal_dbm = Some(-58);
        s.wifi.address = Some("192.168.60.23/24".into());
        s.networks = vec![
            Network { ssid: "ecm-cafe".into(), signal_dbm: -58, channel: 6, security: "wpa2-psk".into() },
            Network { ssid: "library".into(), signal_dbm: -77, channel: 11, security: "open".into() },
        ];
        s.display = Some(DisplayState { tier: 2, width: 640, height: 400, scale: 1, refresh_hz: 30, text_cols: 80, text_rows: 25, colors: "RGBA".into(), touch: false });
        s.cells = vec![CellState { name: "lte_modem_0".into(), tech: "lte".into(), state: "connected".into(), message: "on ecm-mobile cell 1".into(), sim: "ecm-mobile".into(), carrier: "ecm-mobile".into(), bars: 3, signal: "-81 dBm, SINR 12 dB".into(), ..Default::default() }];
    }
    sys
}

fn turn_desk(d: &mut Desktop) {
    d.step(0);
    d.step(0);
}

fn turn(a: &mut App) {
    a.step(0);
    a.step(0);
}

/// The desktop as shell (connection 1, pid 10), its layers configured.
fn start_desktop(s: &Shared, sys: MockSys, w: i32, h: i32) -> (Desktop, Peer) {
    let (conn, peer) = connect(s, 1, 10, 1, "desktop", w, h);
    let mut d = Desktop::new(conn, Box::new(sys), &[]).unwrap();
    turn_desk(&mut d);
    let (tb, wp) = {
        let st = d.state.borrow();
        (st.taskbar, st.wallpaper)
    };
    peer.push(&[configure(tb, 1, w, desktop::TASKBAR_H, 0), configure(wp, 1, w, h, 0)]);
    turn_desk(&mut d);
    (d, peer)
}

/// An app window `wid` (whose frame becomes `frame_wid`) opened and drawn by the desktop and the app.
#[allow(clippy::too_many_arguments)]
fn open_window(d: &mut Desktop, shell: &Peer, app: &mut App, ap: &Peer, wid: u32, frame_wid: u32, pid: u32, app_id: &str, title: &str) {
    turn(app);
    shell.push(&[opened(wid, pid, app_id, title)]);
    turn_desk(d);
    shell.push(&[opened(frame_wid, 10, "desktop", &format!("frame:{wid}"))]);
    turn_desk(d);
    let (rect, maxed, frame_surface) = {
        let st = d.state.borrow();
        let w = st.win(wid).unwrap();
        (w.rect, w.maximized, w.frame_surface)
    };
    let mut states = proto::window_states::ACTIVATED;
    if maxed {
        states |= proto::window_states::MAXIMIZED;
    }
    ap.push(&[Message::Focus(proto::Focus { surface: 1, gained: true }), configure(1, 1, rect.2, rect.3, states)]);
    turn(app);
    let fr = frame_rect(rect, maxed);
    shell.push(&[configure(frame_surface, 1, fr.2, fr.3, 0)]);
    turn_desk(d);
}

fn settings_session(name: &str, w: i32, h: i32, maximize_by_click: bool) -> Vec<u8> {
    let s: Shared = Rc::new(RefCell::new(Session::new(w as u32, h as u32, 1)));
    let dir = root(name);
    let (mut d, shell) = start_desktop(&s, computer(&dir), w, h);
    let (conn, ap) = connect(&s, 2, 11, 0, "settings", w, h);
    let mut app = settings::app(conn, Box::new(computer(&dir)), None).unwrap();
    open_window(&mut d, &shell, &mut app, &ap, 1, 2, 11, "settings", "Settings");
    // Scan and pick the first network (as the compositor would deliver the clicks), so the
    // page shows the network list and the password row too.
    let app_click = |app: &mut App, ap: &Peer, hit: u32| {
        let p = |kind, buttons, button| Message::Pointer(proto::Pointer { surface: 1, kind, x: 4, y: 4, buttons, button, scroll_x: 0, scroll_y: 0, hit_id: hit });
        ap.push(&[p(proto::PointerKind::Down, proto::buttons::LEFT, proto::Button::Left), p(proto::PointerKind::Up, 0, proto::Button::Left)]);
        turn(app);
    };
    let scan = app.core.window(1).unwrap().hit_id_of("scan").unwrap();
    app_click(&mut app, &ap, scan);
    let row = app.core.window(1).unwrap().hit_id_of_row("nets", 0).unwrap();
    app_click(&mut app, &ap, row);
    if maximize_by_click {
        // Click the frame's maximize button where the compositor draws it.
        let (rect, fs) = {
            let st = d.state.borrow();
            let w = st.win(1).unwrap();
            (w.rect, w.frame_surface)
        };
        let fr = frame_rect(rect, false);
        let fw = d.app.core.window(fs).unwrap();
        let b = fw.rect_of("max").unwrap();
        let hit = fw.hit_id_of("max").unwrap();
        let (lx, ly) = (b.x + b.w / 2, b.y + b.h / 2);
        let (x, y) = (fr.0 + lx, fr.1 + ly);
        s.borrow_mut().pointer(proto::PointerKind::Down, x, y, proto::buttons::LEFT, proto::Button::Left, 1, hit);
        s.borrow_mut().pointer(proto::PointerKind::Up, x, y, 0, proto::Button::Left, 1, hit);
        let p = |kind, buttons, button| {
            Message::Pointer(proto::Pointer { surface: fs, kind, x: lx, y: ly, buttons, button, scroll_x: 0, scroll_y: 0, hit_id: hit })
        };
        shell.push(&[
            Message::WindowRequest(proto::WindowRequest { window: 2, action: proto::WindowAction::Activate, edges: 0 }),
            p(proto::PointerKind::Enter, 0, proto::Button::None),
            p(proto::PointerKind::Down, proto::buttons::LEFT, proto::Button::Left),
            p(proto::PointerKind::Up, 0, proto::Button::Left),
        ]);
        turn_desk(&mut d);
        let rect = d.state.borrow().win(1).unwrap().rect;
        ap.push(&[configure(1, 2, rect.2, rect.3, proto::window_states::ACTIVATED | proto::window_states::MAXIMIZED)]);
        turn(&mut app);
        let fr = frame_rect(rect, true);
        shell.push(&[configure(fs, 2, fr.2, fr.3, 0)]);
        turn_desk(&mut d);
    }
    s.borrow_mut().advance(0);
    s.borrow_mut().snap(name);
    let _ = std::fs::remove_dir_all(dir);
    let out = std::mem::take(&mut s.borrow_mut().out);
    out
}

/// The app manager Settings -> Apps shows in the sessions.
fn fake_apps() -> ecm_apps_ui::appctl::FakeCtl {
    use ecm_apps_ui::appctl::{FakeCtl, FileGrant};
    let mut f = FakeCtl::default();
    f.add("dev.ecm.store", "App Store", "1.0.0", &["packages", "network"], &[]);
    f.add("dev.ecm.editor", "Text Editor", "1.0.0", &[], &[".txt", ".md", ".py"]);
    f.add("dev.ecm.netcheck", "Net Check", "1.0.0", &["network", "notifications"], &[".txt"]);
    f.add("dev.ecm.notes", "Notes", "1.0.0", &["files:documents"], &[".md"]);
    for p in ["packages", "network"] {
        f.decisions.insert(("dev.ecm.store".into(), p.into()), true);
    }
    f.decisions.insert(("dev.ecm.netcheck".into(), "network".into()), true);
    f.decisions.insert(("dev.ecm.netcheck".into(), "notifications".into()), false);
    f.grants.push(FileGrant { app: "dev.ecm.netcheck".into(), path: "/home/hosts.txt".into(), write: true });
    f
}

/// Settings opened on Apps with Net Check selected (a click on its row, as the compositor delivers it).
fn settings_apps_session(name: &str, w: i32, h: i32) -> Vec<u8> {
    let s: Shared = Rc::new(RefCell::new(Session::new(w as u32, h as u32, 1)));
    let dir = root(name);
    let (mut d, shell) = start_desktop(&s, computer(&dir), w, h);
    let (conn, ap) = connect(&s, 2, 11, 0, "settings", w, h);
    let mut app = settings::app_with(conn, Box::new(computer(&dir)), Some("apps"), Box::new(fake_apps())).unwrap();
    open_window(&mut d, &shell, &mut app, &ap, 1, 2, 11, "settings", "Settings");
    let row = app.core.window(1).unwrap().hit_id_of_row("apps_list", 1).unwrap();
    let p = |kind, buttons, button| Message::Pointer(proto::Pointer { surface: 1, kind, x: 4, y: 4, buttons, button, scroll_x: 0, scroll_y: 0, hit_id: row });
    ap.push(&[p(proto::PointerKind::Down, proto::buttons::LEFT, proto::Button::Left), p(proto::PointerKind::Up, 0, proto::Button::Left)]);
    turn(&mut app);
    s.borrow_mut().advance(0);
    s.borrow_mut().snap(name);
    let _ = std::fs::remove_dir_all(dir);
    let out = std::mem::take(&mut s.borrow_mut().out);
    out
}

/// What celld reports on a phone attached to cell 1 of cell-red, two cells heard.
fn celld_cell() -> CellState {
    let st = celld::ctl::Status::parse(
        "state connected\nmessage on cell-red cell 1\nmodem module_1\nprofile lte\nsim_carrier cell-red\nimsi 310150000000001\ncarrier cell-red\ncell 1\nchannel 1\nrssi -62.5\nsinr 18.4\ncqi 12\nbars 4\nip 100.65.2.2/24\ngateway 100.65.2.1\ndns 100.65.0.10\nhandovers 1\ntx_bytes 2048\nrx_bytes 1300\nroaming off\n",
    );
    let cells = celld::ctl::parse_cells(
        "cell 1 carrier cell-red profile lte channel 1 rssi -62.5 sinr 18.4 age 40 serving\ncell 7 carrier cell-blue profile lte channel 3 rssi -91.0 sinr 2.0 age 900 note other_carrier\n",
    );
    CellState::from_celld(&st, &cells)
}

/// Settings opened on Cellular (`settings cellular`) with celld's status.
fn settings_cellular_session(name: &str, w: i32, h: i32) -> Vec<u8> {
    let s: Shared = Rc::new(RefCell::new(Session::new(w as u32, h as u32, 1)));
    let dir = root(name);
    let (mut d, shell) = start_desktop(&s, computer(&dir), w, h);
    let (conn, ap) = connect(&s, 2, 11, 0, "settings", w, h);
    let sys = computer(&dir);
    sys.state.borrow_mut().cells = vec![celld_cell()];
    let mut app = settings::app(conn, Box::new(sys), Some("cellular")).unwrap();
    open_window(&mut d, &shell, &mut app, &ap, 1, 2, 11, "settings", "Settings");
    s.borrow_mut().advance(0);
    s.borrow_mut().snap(name);
    let _ = std::fs::remove_dir_all(dir);
    let out = std::mem::take(&mut s.borrow_mut().out);
    out
}

fn desktop_apps() -> Vec<u8> {
    let (w, h) = (640, 400);
    let s: Shared = Rc::new(RefCell::new(Session::new(w as u32, h as u32, 1)));
    let dir = root("desktop");
    std::fs::create_dir_all(dir.join("home/projects")).unwrap();
    std::fs::write(dir.join("home/notes.txt"), "Buy redstone\nCall the village\n").unwrap();
    std::fs::write(dir.join("home/hello.py"), "import ui\n\napp = ui.App(\"dev.you.hello\")\napp.window(\"Hello\", ui.Label(\"Hello, world\"))\napp.run()\n").unwrap();
    std::fs::write(dir.join("home/game.gba"), [0u8; 64]).unwrap();
    // The system apps every computer has: the App Store is listed, the Text Editor app is the core one's twin.
    for (id, name) in [("dev.ecm.store", "App Store"), ("dev.ecm.editor", "Text Editor")] {
        std::fs::create_dir_all(dir.join("apps").join(id)).unwrap();
        std::fs::write(dir.join("apps").join(id).join("app.toml"), format!("id = \"{id}\"\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"main.wasm\"\n")).unwrap();
    }
    let (mut d, shell) = start_desktop(&s, computer(&dir), w, h);
    // Files, then the Editor on top.
    let (c1, p1) = connect(&s, 2, 11, 0, "files", w, h);
    let mut files = files::app(c1, Box::new(computer(&dir)), Some("/home")).unwrap();
    open_window(&mut d, &shell, &mut files, &p1, 1, 2, 11, "files", "Files");
    let (c2, p2) = connect(&s, 3, 12, 0, "editor", w, h);
    let mut ed = editor::app(c2, Box::new(computer(&dir)), Some("/home/hello.py")).unwrap();
    // Window 1 loses focus to the new one.
    p1.push(&[Message::Focus(proto::Focus { surface: 1, gained: false })]);
    open_window(&mut d, &shell, &mut ed, &p2, 3, 4, 12, "editor", "hello.py - Text Editor");
    let rect1 = d.state.borrow().win(1).unwrap().rect;
    p1.push(&[configure(1, 2, rect1.2, rect1.3, 0)]);
    turn(&mut files);
    // A toast.
    shell.push(&[Message::NotificationPosted(proto::NotificationPosted {
        nid: 1,
        app_id: "dev.ecm.editor".into(),
        tag: "saved".into(),
        title: "Saved hello.py".into(),
        body: "/home/hello.py".into(),
        icon: String::new(),
        actions: vec!["Show".into()],
    })]);
    turn_desk(&mut d);
    let toast = d.state.borrow().toasts[0].clone();
    shell.push(&[configure(toast.surface, 1, 240, toast.height, 0)]);
    turn_desk(&mut d);
    // The Apps button, clicked where the compositor draws the taskbar.
    let tb = d.state.borrow().taskbar;
    let (b, hit) = {
        let win = d.app.core.window(tb).unwrap();
        (win.rect_of("apps").unwrap(), win.hit_id_of("apps").unwrap())
    };
    let (lx, ly) = (b.x + b.w / 2, b.y + b.h / 2);
    let (x, y) = (lx, h - desktop::TASKBAR_H + ly);
    s.borrow_mut().pointer(proto::PointerKind::Down, x, y, proto::buttons::LEFT, proto::Button::Left, 1, hit);
    s.borrow_mut().pointer(proto::PointerKind::Up, x, y, 0, proto::Button::Left, 1, hit);
    let p = |kind, buttons, button| Message::Pointer(proto::Pointer { surface: tb, kind, x: lx, y: ly, buttons, button, scroll_x: 0, scroll_y: 0, hit_id: hit });
    shell.push(&[p(proto::PointerKind::Enter, 0, proto::Button::None), p(proto::PointerKind::Down, proto::buttons::LEFT, proto::Button::Left), p(proto::PointerKind::Up, 0, proto::Button::Left)]);
    turn_desk(&mut d);
    let pop = d.state.borrow().launcher.unwrap();
    let (pw, ph) = {
        // The popup asked for its size in SurfaceCreate: the compositor configures it with that.
        let st = s.borrow();
        let msgs = scan_creates(&st.out);
        msgs.into_iter().rev().find(|c| c.surface == pop && c.role == proto::SurfaceRole::Popup).map(|c| (c.w, c.h)).unwrap()
    };
    shell.push(&[configure(pop, 1, pw, ph, 0)]);
    turn_desk(&mut d);
    s.borrow_mut().advance(0);
    s.borrow_mut().snap("desktop_apps");
    let _ = std::fs::remove_dir_all(dir);
    let out = std::mem::take(&mut s.borrow_mut().out);
    out
}

/// Every SurfaceCreate in a session's send records so far.
fn scan_creates(session: &[u8]) -> Vec<proto::SurfaceCreate> {
    let mut out = Vec::new();
    let mut i = 20;
    while i < session.len() {
        let t = session[i];
        i += 1;
        let rd = |i: usize| u32::from_le_bytes([session[i], session[i + 1], session[i + 2], session[i + 3]]) as usize;
        match t {
            1 => {
                let n = u16::from_le_bytes([session[i + 9], session[i + 10]]) as usize;
                i += 9 + 2 + n;
            }
            2 => {
                let len = rd(i + 4);
                let b = &session[i + 8..i + 8 + len];
                for m in Message::decode_all(b).unwrap() {
                    if let Message::SurfaceCreate(c) = m {
                        out.push(c);
                    }
                }
                i += 8 + len;
            }
            3 => {
                let n = rd(i + 8);
                let off = i + 12 + n * 16;
                let len = rd(off);
                i = off + 4 + len;
            }
            4 => {
                let len = rd(i + 12);
                i += 16 + len;
            }
            5 => i += 4,
            6 => {
                let n = u16::from_le_bytes([session[i], session[i + 1]]) as usize;
                i += 2 + n;
            }
            7 => i += 1 + 4 + 4 + 1 + 1 + 4 + 4 + 4,
            9 => i += 4,
            _ => panic!("unknown record {t}"),
        }
    }
    out
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "../src/test/resources/ui-sessions".into());
    std::fs::create_dir_all(&dir).unwrap();
    let sessions: Vec<(&str, Vec<u8>)> = vec![
        ("settings_compact", settings_session("settings_compact", 320, 200, false)),
        ("settings_medium", settings_session("settings_medium", 640, 400, false)),
        ("settings_wide", settings_session("settings_wide", 960, 480, true)),
        ("settings_apps", settings_apps_session("settings_apps", 640, 400)),
        ("settings_apps_compact", settings_apps_session("settings_apps_compact", 320, 200)),
        ("settings_apps_wide", settings_apps_session("settings_apps_wide", 960, 480)),
        ("settings_cellular", settings_cellular_session("settings_cellular", 640, 400)),
        ("settings_cellular_compact", settings_cellular_session("settings_cellular_compact", 320, 200)),
        ("desktop_apps", desktop_apps()),
    ];
    for (name, bytes) in sessions {
        let path = format!("{dir}/{name}.session");
        std::fs::write(&path, &bytes).unwrap();
        println!("wrote {path} ({} bytes)", bytes.len());
    }
}
