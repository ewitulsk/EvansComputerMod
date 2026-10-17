//! Records the Browser's sessions that the Java golden-image test
//! (UiGoldenTest) replays through the compositor and rasterizer:
//!
//!   cargo run -p browser --example record_browser_sessions -- <out dir>
//!   (the checked-in copies live in src/test/resources/ui-sessions/)
//!
//! Same file format as ecm-ui's `record_ui_sessions`. A shell places the
//! window full screen; the app is the real code driven through a mock
//! connection and a scripted network (`browser_app::fake::Net`).
//!
//! * `browser_compact` (240x400, a phone's portrait display): the styled
//!   sample site (`browser_app::samples`) in its phone layout, the buttons in
//!   the bottom bar.
//! * `browser_medium` (640x400): the same site: the flex bar and the cards side by side.
//! * `browser_wide` (1024x640): a Tech Village website, unstyled, in the theme's colours.

use browser_app::app::BrowserApp;
use browser_app::fake::Net;
use browser_app::samples;
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

/// A Tech Village site as `WorldNetwork.website` writes it (shortened).
fn village() -> String {
    let mut others = String::new();
    for (v, style) in [(1, "desert"), (2, "taiga"), (3, "savanna"), (5, "snowy"), (6, "plains")] {
        others.push_str(&format!("<li><a href=\"http://100.{}.0.10/\">Tech Village {v}</a> ({style}, AS {})</li>\n", 64 + v, 65000 + v));
    }
    format!(
        "<!DOCTYPE html>\n<html><head><title>Tech Village 4 - AS 65004</title></head>\n<body>\n<h1>Tech Village 4</h1>\n\
         <p>A plains village on the Tech Village fiber ring. AS 65004, served from the village data center.</p>\n<h2>Network</h2>\n<ul>\n\
         <li>Village cable (ISP eth0): 100.68.1.0/24, gateway 100.68.1.1, DHCP .10-.200: plug any computer in and use 'iface ethN dhcp'</li>\n\
         <li>Data center LAN (ISP eth1): 100.68.0.0/24, gateway 100.68.0.1; this web server 100.68.0.10</li>\n</ul>\n\
         <h2>Chat</h2>\n<p>The ring's chat server runs in the data center of village 2: 100.66.0.20 port 7777.</p>\n\
         <p>Run <code>chat</code> (village computers read /etc/chat.conf) or <code>chat 100.66.0.20</code>.</p>\n\
         <h2>Other villages</h2>\n<ul>\n{others}</ul>\n</body></html>\n"
    )
}

fn record(name: &str, w: i32, h: i32, address: &str) -> Vec<u8> {
    let net = Net::default()
        .page("100.68.0.10:80 /", "200 OK", "text/html", &village())
        .page("10.0.0.5:80 /", "200 OK", "text/html", samples::INDEX_HTML)
        .page("10.0.0.5:80 /style.css", "200 OK", "text/css", samples::STYLE_CSS);
    let mut s = Session::new(w as u32, h as u32);
    shell(&mut s);
    let m = mock(w, h);
    s.open(2, 11, 0, "browser");
    let mut b = BrowserApp::new(Box::new(m.clone()), net, Some(address)).unwrap();
    b.app.core.flush().unwrap();
    s.flush_app(2, &m);
    place_full(&mut s, w, h);
    m.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w, h, states: proto::window_states::FULLSCREEN | proto::window_states::ACTIVATED })]);
    for _ in 0..4 {
        b.step(0);
    }
    s.flush_app(2, &m);
    s.advance(20);
    s.snap(name);
    s.out
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "../src/test/resources/ui-sessions".into());
    std::fs::create_dir_all(&dir).unwrap();
    let sessions: Vec<(&str, Vec<u8>)> = vec![
        ("browser_compact", record("browser_compact", 240, 400, "10.0.0.5")),
        ("browser_medium", record("browser_medium", 640, 400, "10.0.0.5")),
        ("browser_wide", record("browser_wide", 1024, 640, "100.68.0.10")),
    ];
    for (name, bytes) in sessions {
        let path = format!("{dir}/{name}.session");
        std::fs::write(&path, &bytes).unwrap();
        println!("wrote {path} ({} bytes)", bytes.len());
    }
}
