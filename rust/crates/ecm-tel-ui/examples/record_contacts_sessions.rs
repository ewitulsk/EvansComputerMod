//! Records the Contacts sessions that the Java golden-image test (UiGoldenTest)
//! replays through the compositor and rasterizer:
//!
//!   cd rust; cargo run -p ecm-tel-ui --example record_contacts_sessions -- ../src/test/resources/ui-sessions
//!
//! Same file format as ecm-ui's `record_ui_sessions`. A shell places the window
//! full screen; the app is the real code over a scripted telephonyd (this phone
//! is +1 214 555 0199) and an in-memory address book
//! (`ecm_tel_ui::contacts::sample_files`: five contacts, Alex Smith a favourite
//! with a mobile and a work number, an email and a note).
//!
//! * `contacts_compact` (320x480): the list: My card with the phone's number,
//!   the five contacts with their avatars, Alex's Fav tag, the A-Z index.
//! * `contacts_medium` (640x400): Alex's page: two labelled numbers with Call and
//!   Text, the email, the note, Edit / Unfavourite / Share / Delete.
//! * `contacts_wide` (1024x640): the list beside Alex's page.
//! * `contacts_edit` (320x480): the Edit form for Alex, after Add number.

use ecm_tel_ui::contacts::{sample_files, ContactsApp, Screen};
use ecm_tel_ui::fake::{FakeTel, MemFiles};
use ecm_tel_ui::testkit::{hit, mock};
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

type A = ContactsApp<FakeTel, MemFiles>;

fn click(a: &mut A, m: &MockConnection, id: &str, t: u64) {
    a.step(0, t);
    m.push(&ui_samples::click(hit(&a.app, id), 10, 10));
    a.step(0, t + 1);
    a.step(0, t + 2);
}

/// What the session shows once the list is up.
#[derive(Clone, Copy)]
enum Shot {
    List,
    Alex,
    Edit,
}

fn record(name: &str, w: i32, h: i32, shot: Shot) -> Vec<u8> {
    let mut s = Session::new(w as u32, h as u32);
    shell(&mut s);
    let m = mock(w, h);
    s.open(2, 11, 0, "contacts");
    let mut a = ContactsApp::new(Box::new(m.clone()), FakeTel::new("+12145550199"), sample_files()).unwrap();
    a.app.core.flush().unwrap();
    s.flush_app(2, &m);
    place_full(&mut s, w, h);
    configure(&m, w, h);
    a.step(0, 0);
    a.step(0, 1);
    match shot {
        Shot::List => {}
        Shot::Alex => click(&mut a, &m, "contacts", 10), // the first row
        Shot::Edit => {
            click(&mut a, &m, "contacts", 10);
            click(&mut a, &m, "edit", 20);
            click(&mut a, &m, "e_add_num", 30);
        }
    }
    a.step(0, 40);
    a.step(0, 41);
    match shot {
        Shot::Alex => assert!(matches!(a.model.borrow().screen, Screen::Contact(_))),
        Shot::Edit => assert_eq!(a.model.borrow().screen, Screen::Edit),
        Shot::List => {}
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
        ("contacts_compact", record("contacts_compact", 320, 480, Shot::List)),
        ("contacts_medium", record("contacts_medium", 640, 400, Shot::Alex)),
        ("contacts_wide", record("contacts_wide", 1024, 640, Shot::Alex)),
        ("contacts_edit", record("contacts_edit", 320, 480, Shot::Edit)),
    ];
    for (name, bytes) in sessions {
        let path = format!("{dir}/{name}.session");
        std::fs::write(&path, &bytes).unwrap();
        println!("wrote {path} ({} bytes)", bytes.len());
    }
}
