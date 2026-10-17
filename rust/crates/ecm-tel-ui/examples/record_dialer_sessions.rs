//! Records the Phone (dev.ecm.dialer) sessions that the Java golden-image test
//! (UiGoldenTest) replays through the compositor and rasterizer:
//!
//!   cd rust; cargo run -p ecm-tel-ui --example record_dialer_sessions -- ../src/test/resources/ui-sessions
//!
//! Same file format as ecm-ui's `record_ui_sessions`. A shell places the window
//! full screen; the app is the real code over a scripted telephonyd
//! (`ecm_tel_ui::fake::FakeTel`, own number (214) 555-0100, registered) and an
//! address book of three contacts (Alex Smith a favourite, Bo Jones, Cy).

use ecm_tel::ctl::{CallLine, LogLine};
use ecm_tel::vcard::Contact;
use ecm_tel_ui::book::Book;
use ecm_tel_ui::dialer::{DialerApp, Tab};
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

fn book() -> MemFiles {
    let mut f = MemFiles::default();
    let mut b = Book::default();
    let mut a = Contact::new("Alex Smith", "+12145550142");
    a.favourite = true;
    b.put(a);
    let mut bo = Contact::new("Bo Jones", "+12145550143");
    bo.favourite = true;
    b.put(bo);
    b.put(Contact::new("Cy Park", "+12145550144"));
    b.save(&mut f).unwrap();
    f
}

/// The phone's state the sessions start from: a call log (one missed call) and a new voicemail.
fn tel() -> FakeTel {
    let mut t = FakeTel::new("+12145550100");
    // telephonyd's clock 6600 s in: the calls in the log are an hour or more old
    t.now_ms = 6_600_000;
    let l = |id, dir: &str, n: &str, start_ms, seconds| LogLine { id, dir: dir.into(), number: n.into(), start_ms, seconds, reason: "-".into() };
    t.log = vec![l(1, "out", "+12145550142", 0, 312), l(2, "in", "+12145550188", 1_200_000, 65), l(3, "missed", "+12145550143", 3_000_000, 0)];
    t.voicemail = vec!["vm 4 +12145550143 3000000 12400 new".into()];
    t
}

/// Open the app on the scripted telephonyd, run `setup`, take one picture.
fn record(name: &str, w: i32, h: i32, tel: FakeTel, setup: impl FnOnce(&mut DialerApp<FakeTel, MemFiles>, &MockConnection)) -> Vec<u8> {
    let mut s = Session::new(w as u32, h as u32);
    shell(&mut s);
    let m = mock(w, h);
    s.open(2, 11, 0, "dialer");
    let mut a = DialerApp::new(Box::new(m.clone()), tel, book()).unwrap();
    a.app.core.flush().unwrap();
    s.flush_app(2, &m);
    place_full(&mut s, w, h);
    configure(&m, w, h);
    let now = 1000;
    a.step(0, now);
    setup(&mut a, &m);
    a.step(0, now + 1);
    a.step(0, now + 2);
    s.flush_app(2, &m);
    s.advance(20);
    s.snap(name);
    s.out
}

fn press(a: &mut DialerApp<FakeTel, MemFiles>, m: &MockConnection, id: &str, t: u64) {
    m.push(&ui_samples::click(hit(&a.app, id), 10, 10));
    a.step(0, t);
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "../src/test/resources/ui-sessions".into());
    std::fs::create_dir_all(&dir).unwrap();
    let sessions: Vec<(&str, Vec<u8>)> = vec![
        // dialer_compact (320x480): the keypad with 214555014 typed on its keys,
        // shown as (214) 555-014 with the matching contact (Alex Smith) under it.
        (
            "dialer_compact",
            record("dialer_compact", 320, 480, tel(), |a, m| {
                a.model.borrow_mut().tab = Tab::Keypad;
                a.step(0, 1010);
                for (i, d) in "214555014".chars().enumerate() {
                    press(a, m, &format!("key_{d}"), 1020 + i as u64);
                }
            }),
        ),
        // dialer_medium (640x400): Recents, the missed call from Bo Jones in red and
        // the tab's missed count, an incoming call from an unknown number, an outgoing one.
        (
            "dialer_medium",
            record("dialer_medium", 640, 400, tel(), |a, _| {
                a.model.borrow_mut().tab = Tab::Recents;
            }),
        ),
        // dialer_wide (1024x640): the pages on the left (Recents 1, Voicemail 1),
        // Favourites (Alex Smith, Bo Jones) and Recents beside them.
        (
            "dialer_wide",
            record("dialer_wide", 1024, 640, tel(), |a, _| {
                a.model.borrow_mut().tab = Tab::Favourites;
            }),
        ),
        // dialer_incoming (320x480): Bo Jones calling: name, number, Decline,
        // Answer and Message (hotkeys 1 and 2 claimed).
        (
            "dialer_incoming",
            record(
                "dialer_incoming",
                320,
                480,
                {
                    let mut t = tel();
                    t.calls.push(CallLine { id: 7, state: "incoming".into(), number: "+12145550143".into(), ..Default::default() });
                    t
                },
                |_, _| {},
            ),
        ),
        // dialer_in_call (320x480): an HD call with Alex Smith at 0:42, muted with the Mute button:
        // the timer, HD and Muted tags, the codec line, Mute lit, End.
        (
            "dialer_in_call",
            record(
                "dialer_in_call",
                320,
                480,
                {
                    let mut t = tel();
                    t.calls.push(CallLine { id: 5, state: "active".into(), number: "+12145550142".into(), outgoing: true, seconds: 42, muted: false, hd: true, codec: "ADPCM-WB".into(), loss: 0.4, merged: false, hidden: false });
                    t
                },
                |a, m| press(a, m, "mute", 1010),
            ),
        ),
    ];
    for (name, bytes) in sessions {
        let path = format!("{dir}/{name}.session");
        std::fs::write(&path, &bytes).unwrap();
        println!("wrote {path} ({} bytes)", bytes.len());
    }
}
