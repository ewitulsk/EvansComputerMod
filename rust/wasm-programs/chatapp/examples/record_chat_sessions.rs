//! Records the Chat app's sessions that the Java golden-image test
//! (UiGoldenTest) replays through the compositor and rasterizer:
//!
//!   cargo run -p chatapp --example record_chat_sessions -- <out dir>
//!   (the checked-in copies live in src/test/resources/ui-sessions/)
//!
//! Same file format as ecm-ui's `record_ui_sessions`. A shell places the
//! window full screen; the app is the real code driven through a mock
//! connection and a scripted chat server (`chat_app::app::fake::FakeNet`).
//!
//! * `chat_compact` (240x400, a phone's portrait display): the connect form
//!   with the server typed (`chat_compact_connect`), then the conversation
//!   with a reply in it and a message being typed (`chat_compact`).
//! * `chat_medium` (640x400) and `chat_wide` (1024x640): the conversation
//!   with the room and the people beside it.

use chat_app::app::fake::FakeNet;
use chat_app::app::ChatApp;
use chat_app::model::Model;
use ecm_ui::{samples as ui_samples, MockConnection};
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
    fn advance(&mut self, ms: u32) {
        self.out.push(5);
        self.u32(ms);
    }
    fn snap(&mut self, name: &str) {
        self.out.push(6);
        self.out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.out.extend_from_slice(name.as_bytes());
    }
    /// What the app sent since the last call.
    fn flush_app(&mut self, conn: u32, mock: &MockConnection) {
        let sent = mock.sent();
        mock.clear_sent();
        self.send(conn, &sent);
    }
}

fn mock(w: i32, h: i32) -> MockConnection {
    let m = MockConnection::new();
    m.state.borrow_mut().idle_timeouts = true;
    let mut wl = ui_samples::welcome(false);
    if let Message::Welcome(ref mut x) = wl {
        x.display.w = w;
        x.display.h = h;
        x.display.size_class = proto::SizeClass::for_width(w);
    }
    m.push(&[wl]);
    m
}

fn shell(s: &mut Session) {
    s.open(1, 10, 1, "shell");
    s.send(1, &proto::encode_all(&[Message::Hello(proto::Hello { version: proto::VERSION, role: proto::Role::Shell, capabilities: vec![] })]));
}

/// The shell places window 1 over the whole display (after the app created it).
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

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("ecm-record-chat-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().replace('\\', "/")
}

fn steps(ca: &mut ChatApp<FakeNet>, n: usize) {
    for _ in 0..n {
        ca.step(0);
    }
}

fn click(ca: &mut ChatApp<FakeNet>, m: &MockConnection, id: &str) {
    steps(ca, 1);
    let hit = ca.app.core.window(1).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no widget {id}"));
    m.push(&ui_samples::click(hit, 10, 10));
    steps(ca, 2);
}

fn type_into(ca: &mut ChatApp<FakeNet>, m: &MockConnection, id: &str, text: &str) {
    click(ca, m, id);
    m.push(&[Message::Text(proto::Text { surface: 1, text: text.into() })]);
    steps(ca, 2);
}

/// The scripted conversation every size shows.
fn converse(ca: &mut ChatApp<FakeNet>, m: &MockConnection) {
    type_into(ca, m, "nick", "alex");
    click(ca, m, "connect");
    ca.net.server_says(&[
        "WELCOME alex Village chat",
        "HIST 09:58:12 bo morning! the tower is up",
        "HIST 09:59:40 cy nice, LTE reaches the farm",
    ]);
    steps(ca, 1);
    ca.net.server_says(&["WHO alex bo cy", "JOIN 10:00:02 dee"]);
    steps(ca, 1);
    type_into(ca, m, "compose", "hi all, this is my phone");
    click(ca, m, "send");
    ca.net.server_says(&["MSG 10:00:31 alex hi all, this is my phone", "MSG 10:00:40 bo welcome alex! got the SIM from the tower?"]);
    steps(ca, 2);
    type_into(ca, m, "compose", "yes, wrote it there");
}

fn record(name: &str, w: i32, h: i32, connect_snap: bool) -> Vec<u8> {
    let mut s = Session::new(w as u32, h as u32);
    shell(&mut s);
    let m = mock(w, h);
    s.open(2, 11, 0, "chatapp");
    let mut ca = ChatApp::new(Box::new(m.clone()), FakeNet::default(), Some(tmp(name)), Model::default()).unwrap();
    ca.app.core.flush().unwrap();
    s.flush_app(2, &m);
    place_full(&mut s, w, h);
    m.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w, h, states: proto::window_states::FULLSCREEN | proto::window_states::ACTIVATED })]);
    steps(&mut ca, 2);
    type_into(&mut ca, &m, "server", "10.0.30.1");
    if connect_snap {
        s.flush_app(2, &m);
        s.advance(20);
        s.snap(&format!("{name}_connect"));
    }
    converse(&mut ca, &m);
    s.flush_app(2, &m);
    s.advance(20);
    s.snap(name);
    s.out
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "../src/test/resources/ui-sessions".into());
    std::fs::create_dir_all(&dir).unwrap();
    let sessions: Vec<(&str, Vec<u8>)> = vec![
        ("chat_compact", record("chat_compact", 240, 400, true)),
        ("chat_medium", record("chat_medium", 640, 400, false)),
        ("chat_wide", record("chat_wide", 1024, 640, false)),
    ];
    for (name, bytes) in sessions {
        let path = format!("{dir}/{name}.session");
        std::fs::write(&path, &bytes).unwrap();
        println!("wrote {path} ({} bytes)", bytes.len());
    }
}
