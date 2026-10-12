//! Records the App Store, file picker and chooser sessions that the Java
//! golden-image test (UiGoldenTest) replays through the compositor and
//! rasterizer, at the three size classes:
//!
//!   cargo run -p ecm-apps-ui --example record_store_sessions -- <out dir>
//!   (the checked-in copies live in src/test/resources/ui-sessions/)
//!
//! Same file format as ecm-ui's `record_ui_sessions` (see there). A shell
//! places the window full screen; the app is the real code driven through a
//! mock connection, with the sample repository in memory and an in-memory
//! app manager.

use ecm_apps_ui::appctl::FakeCtl;
use ecm_apps_ui::chooser::{ChooseRequest, ChooserApp};
use ecm_apps_ui::picker::{HostFs, Mode, PickRequest, PickerApp};
use ecm_apps_ui::samples;
use ecm_apps_ui::store::StoreApp;
use ecm_apps_ui::uictl::Recorded;
use ecm_pkg::client::{Client, MemoryNet};
use ecm_ui::{samples as ui_samples, App, MockConnection};
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;

struct Session {
    out: Vec<u8>,
}

impl Session {
    fn new(w: u32, h: u32) -> Session {
        let mut out = b"ECMUISS1".to_vec();
        for v in [w, h, 1] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        Session { out }
    }
    fn u32(&mut self, v: u32) {
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

    /// What the app sent since the last call: messages, then image uploads.
    fn flush_app(&mut self, conn: u32, mock: &MockConnection) {
        let sent = mock.sent();
        mock.clear_sent();
        self.send(conn, &sent);
        let ups: Vec<(u32, i32, Vec<u8>)> = std::mem::take(&mut mock.state.borrow_mut().uploads);
        for (res, kind, data) in ups {
            self.upload(conn, res, kind as u32, &data);
        }
    }
}

fn welcome(w: i32, h: i32) -> Message {
    let mut m = ui_samples::welcome(false);
    if let Message::Welcome(ref mut wl) = m {
        wl.display.w = w;
        wl.display.h = h;
        wl.display.size_class = proto::SizeClass::for_width(w);
    }
    m
}

fn mock(w: i32, h: i32) -> MockConnection {
    let m = MockConnection::new();
    m.state.borrow_mut().idle_timeouts = true;
    m.push(&[welcome(w, h)]);
    m
}

/// A shell that places window 1 over the whole display.
fn shell(s: &mut Session, w: i32, h: i32) {
    s.open(1, 10, 1, "shell");
    s.send(
        1,
        &proto::encode_all(&[
            Message::Hello(proto::Hello { version: proto::VERSION, role: proto::Role::Shell, capabilities: vec![] }),
        ]),
    );
    let _ = (w, h);
}

fn place_full(s: &mut Session, w: i32, h: i32) {
    s.send(
        1,
        &proto::encode_all(&[Message::Place(proto::Place {
            window: 1,
            x: 0,
            y: 0,
            w,
            h,
            states: proto::window_states::FULLSCREEN,
            transition: proto::Transition { ms: 0, easing: proto::Easing::Linear },
        })]),
    );
}

fn configure(m: &MockConnection, w: i32, h: i32) {
    m.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w, h, states: proto::window_states::FULLSCREEN })]);
}

fn click(app: &mut App, m: &MockConnection, id: &str) {
    let hit = app.core.window(1).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no widget {id}"));
    m.push(&ui_samples::click(hit, 10, 10));
}

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("ecm-record-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().replace('\\', "/")
}

const URL: &str = "http://10.0.7.1:8080/";

/// The App Store at `w x h`: the list (Counter already installed, so its card
/// says Open), then an app's page.
fn store(name: &str, w: i32, h: i32, detail: &str) -> Vec<u8> {
    let mut s = Session::new(w as u32, h as u32);
    shell(&mut s, w, h);
    let m = mock(w, h);
    s.open(2, 11, 0, "store");
    let client = Client::new(&tmp(name), samples::memory_repo(URL, None));
    client.add_repo("samples", URL, None).unwrap();
    let mut pre = client;
    let a = pre.find("dev.visual.counter", false).unwrap();
    pre.install_available(&a, &mut |_, _| {}).unwrap();
    let mut ctl = FakeCtl::default();
    ctl.add(ecm_apps_ui::STORE_ID, "App Store", "1.0.0", &["packages", "network"], &[]);
    let mut sa: StoreApp<MemoryNet, FakeCtl> = StoreApp::new(Box::new(m.clone()), pre, ctl, true).unwrap();
    sa.app.core.flush().unwrap();
    s.flush_app(2, &m);
    place_full(&mut s, w, h);
    configure(&m, w, h);
    sa.step(0);
    sa.step(0);
    s.flush_app(2, &m);
    s.advance(20);
    s.snap(name);
    click(&mut sa.app, &m, &format!("name:{detail}"));
    sa.step(0);
    sa.step(0);
    s.flush_app(2, &m);
    s.advance(20);
    s.snap(&format!("{name}_detail"));
    s.out
}

fn picker() -> Vec<u8> {
    let (w, h) = (640, 400);
    let mut s = Session::new(w as u32, h as u32);
    shell(&mut s, w, h);
    let root = tmp("picker");
    for (p, d) in [("/home/todo.txt", "x"), ("/home/shopping.md", "x"), ("/home/photo.png", "x"), ("/home/letters/mum.txt", "x")] {
        let f = format!("{root}{p}");
        std::fs::create_dir_all(std::path::Path::new(&f).parent().unwrap()).unwrap();
        std::fs::write(&f, d.repeat(1200)).unwrap();
    }
    let m = mock(w, h);
    s.open(2, 12, 0, "picker");
    let req = PickRequest { token: "0123456789abcdef".into(), app: "dev.ecm.notes".into(), mode: Mode::Save, types: vec![".txt".into(), ".md".into()], suggested: "notes.txt".into() };
    let mut p = PickerApp::new(Box::new(m.clone()), req, Box::new(HostFs { root }), Recorded::default()).unwrap();
    p.app.core.flush().unwrap();
    s.flush_app(2, &m);
    place_full(&mut s, w, h);
    configure(&m, w, h);
    p.step(0);
    p.step(0);
    s.flush_app(2, &m);
    s.advance(20);
    s.snap("picker_save");
    s.out
}

fn chooser() -> Vec<u8> {
    let (w, h) = (640, 400);
    let mut s = Session::new(w as u32, h as u32);
    shell(&mut s, w, h);
    let m = mock(w, h);
    s.open(2, 13, 0, "chooser");
    let mut ctl = FakeCtl::default();
    ctl.add("dev.ecm.editor", "Text Editor", "1.0.0", &[], &[".txt", ".md"]);
    ctl.add("dev.ecm.notes", "Notes", "1.0.0", &[], &[".txt"]);
    ctl.defaults.insert(".md".into(), "dev.ecm.editor".into());
    let req = ChooseRequest { token: "fedcba9876543210".into(), path: "/home/todo.txt".into() };
    let mut c = ChooserApp::new(Box::new(m.clone()), req, &mut ctl, Recorded::default()).unwrap();
    c.app.core.flush().unwrap();
    s.flush_app(2, &m);
    place_full(&mut s, w, h);
    configure(&m, w, h);
    c.step(0);
    c.step(0);
    s.flush_app(2, &m);
    s.advance(20);
    s.snap("chooser");
    s.out
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "../src/test/resources/ui-sessions".into());
    std::fs::create_dir_all(&dir).unwrap();
    let sessions: Vec<(&str, Vec<u8>)> = vec![
        ("store_compact", store("store_compact", 360, 640, "dev.ecm.notes")),
        ("store_medium", store("store_medium", 640, 400, "dev.ecm.netcheck")),
        ("store_wide", store("store_wide", 1024, 640, "dev.ecm.notes")),
        ("picker_save", picker()),
        ("chooser", chooser()),
    ];
    for (name, bytes) in sessions {
        let path = format!("{dir}/{name}.session");
        std::fs::write(&path, &bytes).unwrap();
        println!("wrote {path} ({} bytes)", bytes.len());
    }
}
