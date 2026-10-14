//! Records the Cell Site and Carrier Console sessions that the Java golden-image
//! test (UiGoldenTest) replays through the compositor and rasterizer:
//!
//!   cd rust; cargo run -p ecm-cell-ui --example record_cell_sessions -- ../src/test/resources/ui-sessions
//!
//! Same file format as ecm-ui's `record_ui_sessions`. A shell places the window
//! full screen; the apps are the real code over scripted admin sockets
//! (`ecm_cell_ui::fake`): a site with two phones and a neighbour, a core with
//! three subscribers, two sites (one connected) and two phones.
//!
//! * `cellsite_compact` (320x480): the overview. `cellsite_medium` (640x400): the
//!   phones. `cellsite_wide` (1024x640): overview, phones and settings.
//! * `carrier_compact` (320x480): subscribers. `carrier_medium` (640x400): sites,
//!   after Add site made a credential. `carrier_wide` (1024x640): subscribers with
//!   one chosen, after Write SIM.

use ecm_cell_ui::carrier::{self, ConsoleApp};
use ecm_cell_ui::fake::{busy_core, busy_site, Scripted};
use ecm_cell_ui::site::{self, SiteApp};
use ecm_cell_ui::testkit::{hit, mock};
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
    fn flush_app(&mut self, conn: u32, m: &MockConnection) {
        let sent = m.sent();
        m.clear_sent();
        self.send(conn, &sent);
    }
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

fn configure(m: &MockConnection, w: i32, h: i32) {
    m.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w, h, states: proto::window_states::FULLSCREEN | proto::window_states::ACTIVATED })]);
}

fn record_site(name: &str, w: i32, h: i32, page: site::Page) -> Vec<u8> {
    let mut s = Session::new(w as u32, h as u32);
    shell(&mut s);
    let m = mock(w, h);
    s.open(2, 11, 0, "cellsite");
    let mut a = SiteApp::new(Box::new(m.clone()), busy_site()).unwrap();
    a.app.core.flush().unwrap();
    s.flush_app(2, &m);
    place_full(&mut s, w, h);
    configure(&m, w, h);
    a.step(0, 0);
    a.model.borrow_mut().page = page;
    a.step(0, 1);
    a.step(0, 2);
    s.flush_app(2, &m);
    s.advance(20);
    s.snap(name);
    s.out
}

fn click(a: &mut ConsoleApp<Scripted>, m: &MockConnection, id: &str, t: u64) {
    a.step(0, t);
    m.push(&ui_samples::click(hit(&a.app, id), 10, 10));
    a.step(0, t + 1);
    a.step(0, t + 2);
}

fn type_into(a: &mut ConsoleApp<Scripted>, m: &MockConnection, id: &str, text: &str, t: u64) {
    click(a, m, id, t);
    m.push(&[Message::Text(proto::Text { surface: 1, text: text.into() })]);
    a.step(0, t + 3);
    a.step(0, t + 4);
}

fn record_carrier(name: &str, w: i32, h: i32, page: carrier::Page) -> Vec<u8> {
    let mut s = Session::new(w as u32, h as u32);
    shell(&mut s);
    let m = mock(w, h);
    s.open(2, 11, 0, "carrier");
    let mut admin = busy_core();
    admin.on("site-new", "site site-c\nsecret 3f9a1c0e5b7d2468ace013579bdf2468");
    admin.on("sim-new", "imsi 001010000000763\nlabel carol\nejected 0");
    let mut a = ConsoleApp::new(Box::new(m.clone()), admin).unwrap();
    a.app.core.flush().unwrap();
    s.flush_app(2, &m);
    place_full(&mut s, w, h);
    configure(&m, w, h);
    a.step(0, 0);
    a.model.borrow_mut().page = page;
    a.step(0, 1);
    a.step(0, 2);
    match (page, w >= 800) {
        (carrier::Page::Sites, _) => {
            type_into(&mut a, &m, "new_site", "site-c", 10);
            a.admin.on("sites", "site site-a 1 10.0.1.21 1 412 1830 0
site site-b 0 - - - - 0
site site-c 0 - - - - 0");
            click(&mut a, &m, "add_site", 20);
        }
        (carrier::Page::Subscribers, true) => {
            type_into(&mut a, &m, "new_label", "carol", 10);
            click(&mut a, &m, "write_sim", 20);
            a.model.borrow_mut().selected = Some("001010000000932".into());
            a.model.borrow_mut().quota = "5000000".into();
            a.step(0, 30);
            a.step(0, 31);
        }
        _ => {}
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
        ("cellsite_compact", record_site("cellsite_compact", 320, 480, site::Page::Overview)),
        ("cellsite_medium", record_site("cellsite_medium", 640, 400, site::Page::Phones)),
        ("cellsite_wide", record_site("cellsite_wide", 1024, 640, site::Page::Overview)),
        ("carrier_compact", record_carrier("carrier_compact", 320, 480, carrier::Page::Subscribers)),
        ("carrier_medium", record_carrier("carrier_medium", 640, 400, carrier::Page::Sites)),
        ("carrier_wide", record_carrier("carrier_wide", 1024, 640, carrier::Page::Subscribers)),
    ];
    for (name, bytes) in sessions {
        let path = format!("{dir}/{name}.session");
        std::fs::write(&path, &bytes).unwrap();
        println!("wrote {path} ({} bytes)", bytes.len());
    }
}
