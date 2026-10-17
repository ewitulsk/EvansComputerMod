//! Records the Discord app's sessions that the Java golden-image test
//! (UiGoldenTest) replays through the compositor and rasterizer:
//!
//!   cargo run -p discord --example record_discord_sessions -- <out dir>
//!   (the checked-in copies live in src/test/resources/ui-sessions/)
//!
//! Same file format as ecm-ui's `record_ui_sessions`. A shell places the
//! window full screen; the app is the real code driven through a mock
//! connection and a scripted network of real backends
//! (`discord_app::app::fake::FakeNet`).
//!
//! * `discord_compact` (240x400, a phone's portrait display): the server list
//!   with the voice channel's talkers (`discord_compact_servers`), then
//!   `#general` with replies in it and a message being typed (`discord_compact`).
//! * `discord_medium` (640x400) and `discord_wide` (1024x640): the rail, the
//!   channels and the conversation side by side (wide: the members too).

use discord_app::app::fake::{FakeAudio, FakeNet};
use discord_app::app::DiscordApp;
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
        // Images (the logo, the voice-channel speaker) go up after the messages that made them.
        let uploads = std::mem::take(&mut mock.state.borrow_mut().uploads);
        for (res, kind, data) in uploads {
            self.out.push(4);
            self.u32(conn);
            self.u32(res);
            self.u32(kind as u32);
            self.bytes(&data);
        }
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
    let d = std::env::temp_dir().join(format!("ecm-record-discord-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().replace('\\', "/")
}

type D = DiscordApp<FakeNet, FakeAudio>;

fn steps(d: &mut D, n: usize) {
    for _ in 0..n {
        d.step(0);
    }
}

fn click(d: &mut D, m: &MockConnection, id: &str) {
    steps(d, 1);
    let hit = d.app.core.window(1).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no widget {id}"));
    m.push(&ui_samples::click(hit, 10, 10));
    steps(d, 4);
}

fn type_into(d: &mut D, m: &MockConnection, id: &str, text: &str) {
    click(d, m, id);
    m.push(&[Message::Text(proto::Text { surface: 1, text: text.into() })]);
    steps(d, 2);
}

/// Two backends on the network; bo and cy on the village one, bo talking in General.
fn network() -> FakeNet {
    let mut net = FakeNet::default();
    net.now = 36_000_000; // 10:00:00
    net.add("100.74.0.20", "Tech Village", true);
    net.add("10.0.8.1", "Lab", true);
    net.user(0, 9, "HELLO bo
SAY 1 morning! the tower is up
");
    net.now += 88_000;
    net.user(0, 10, "HELLO cy
SAY 1 nice, LTE reaches the farm
");
    net
}

/// The scripted conversation every size shows.
fn converse(d: &mut D, m: &MockConnection) {
    let out = d.net.user(0, 9, "VJOIN 2
");
    let token: u32 = out.lines().find(|l| l.starts_with("VOICE")).unwrap().split(' ').nth(3).unwrap().parse().unwrap();
    let tone: Vec<i16> = (0..640).map(|i| ((i as f64 * 0.2).sin() * 8000.0) as i16).collect();
    for p in ecm_discord::voice::Sender::new(token).push(&tone) {
        d.net.user_udp(0, &p.encode());
    }
    steps(d, 2);
    d.net.now += 31_000;
    type_into(d, m, "compose", "hi all, this is my phone");
    click(d, m, "send");
    d.net.now += 9_000;
    d.net.user(0, 9, "SAY 1 welcome! jump in General
");
    steps(d, 3);
    type_into(d, m, "compose", "on my way");
}

fn record(name: &str, w: i32, h: i32, compact: bool) -> Vec<u8> {
    let mut s = Session::new(w as u32, h as u32);
    shell(&mut s);
    let m = mock(w, h);
    s.open(2, 11, 0, "discord");
    let mut d = DiscordApp::new(Box::new(m.clone()), network(), FakeAudio::default(), Some(tmp(name)), None).unwrap();
    d.app.core.flush().unwrap();
    s.flush_app(2, &m);
    place_full(&mut s, w, h);
    m.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w, h, states: proto::window_states::FULLSCREEN | proto::window_states::ACTIVATED })]);
    steps(&mut d, 6);
    if compact {
        let out = d.net.user(0, 9, "VJOIN 2
");
        let token: u32 = out.lines().find(|l| l.starts_with("VOICE")).unwrap().split(' ').nth(3).unwrap().parse().unwrap();
        let tone: Vec<i16> = (0..640).map(|i| ((i as f64 * 0.2).sin() * 8000.0) as i16).collect();
        for p in ecm_discord::voice::Sender::new(token).push(&tone) {
            d.net.user_udp(0, &p.encode());
        }
        steps(&mut d, 3);
        s.flush_app(2, &m);
        s.advance(20);
        s.snap(&format!("{name}_servers"));
        click(&mut d, &m, "chan:1");
    }
    converse(&mut d, &m);
    s.flush_app(2, &m);
    s.advance(20);
    s.snap(name);
    s.out
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "../src/test/resources/ui-sessions".into());
    std::fs::create_dir_all(&dir).unwrap();
    let sessions: Vec<(&str, Vec<u8>)> = vec![
        ("discord_compact", record("discord_compact", 240, 400, true)),
        ("discord_medium", record("discord_medium", 640, 400, false)),
        ("discord_wide", record("discord_wide", 1024, 640, false)),
    ];
    for (name, bytes) in sessions {
        let path = format!("{dir}/{name}.session");
        std::fs::write(&path, &bytes).unwrap();
        println!("wrote {path} ({} bytes)", bytes.len());
    }
}
