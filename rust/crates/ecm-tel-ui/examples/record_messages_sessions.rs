//! Records the Messages (dev.ecm.messages) sessions that the Java golden-image
//! test (UiGoldenTest) replays through the compositor and rasterizer:
//!
//!   cd rust; cargo run -p ecm-tel-ui --example record_messages_sessions -- ../src/test/resources/ui-sessions
//!
//! Same file format as ecm-ui's `record_ui_sessions` (records 1 open, 2 send,
//! 4 upload, 5 advance, 6 snap). A shell places the window full screen; the app
//! is the real code over a scripted telephonyd (`ecm_tel_ui::fake::FakeTel`, own
//! number (214) 555-0100) and an address book in memory: Alex Smith and Bo Jones
//! are contacts, Cy and a short code are not.
//!
//! * `messages_compact` (320x480): the conversations, with unread counts.
//! * `messages_medium` (640x400): a 1:1 thread with Delivered, Sent and Failed (Retry) bubbles.
//! * `messages_wide` (1024x640): the conversations beside that thread.
//! * `messages_group` (320x480): a group thread, sender names above received bubbles, an inline picture.
//! * `messages_compose` (320x480): compose to a contact picked from the suggestions; the counter shows 2 parts.

use ecm_tel::ctl::{MsgLine, ThreadLine};
use ecm_tel::vcard::Contact;
use ecm_tel_ui::book::Book;
use ecm_tel_ui::fake::{FakeTel, MemFiles};
use ecm_tel_ui::messages::MessagesApp;
use ecm_tel_ui::testkit::{hit, mock};
use ecm_ui::{samples as ui_samples, MockConnection};
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;

const ME: &str = "+12145550100";
const ALEX: &str = "+12145550142";
const BO: &str = "+12145550143";
const CY: &str = "+12145550144";
/// telephonyd's clock (`status now`) and the app's when the sessions start.
const NOW: u64 = 9_000_000;

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
    /// What the app sent, each image's data right after its ResourceCreate.
    fn flush_app(&mut self, conn: u32, m: &MockConnection) {
        let sent = m.sent();
        m.clear_sent();
        let ups = std::mem::take(&mut m.state.borrow_mut().uploads);
        let mut batch = Vec::new();
        for msg in Message::decode_all(&sent).expect("valid messages") {
            let created = if let Message::ResourceCreate(r) = &msg { Some(r.id) } else { None };
            batch.push(msg);
            if let Some(id) = created {
                self.send(conn, &proto::encode_all(&batch));
                batch.clear();
                for (res, kind, data) in ups.iter().filter(|u| u.0 == id) {
                    self.upload(conn, *res, *kind as u32, data);
                }
            }
        }
        self.send(conn, &proto::encode_all(&batch));
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

/// A PNG (stored deflate) of a sunset: sky, sun, sea.
fn picture(w: u32, h: u32) -> Vec<u8> {
    fn crc(d: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in d {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc(&c).to_be_bytes());
    }
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let (dx, dy) = (x as i32 - w as i32 / 2, y as i32 - h as i32 * 3 / 5);
            let px = if dx * dx + dy * dy < (h as i32 / 5).pow(2) && dy < 0 {
                [255, 214, 90]
            } else if y < h * 3 / 5 {
                [240 - (y * 120 / h) as u8, 120 + (y * 60 / h) as u8, 90 + (y * 40 / h) as u8]
            } else {
                [30, 70 + (y * 40 / h) as u8, 120 + ((x + y) % 7) as u8 * 6]
            };
            raw.extend_from_slice(&px);
        }
    }
    let mut z = vec![0x78, 0x01];
    let n = raw.chunks(65535).count();
    for (i, blk) in raw.chunks(65535).enumerate() {
        z.push((i + 1 == n) as u8);
        z.extend_from_slice(&(blk.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(blk.len() as u16)).to_le_bytes());
        z.extend_from_slice(blk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn msg(id: u64, incoming: bool, from: &str, ago_s: u64, status: &str, text: &str) -> MsgLine {
    MsgLine { id, incoming, from: from.into(), time_ms: NOW - ago_s * 1000, status: status.into(), text: text.into(), ..Default::default() }
}

fn group_key() -> String {
    format!("{ALEX},{BO},{CY}")
}

/// Four conversations (a 1:1 with two unread, a group, someone not in Contacts, a short code) and two threads' messages.
fn tel() -> FakeTel {
    let mut t = FakeTel::new(ME);
    t.now_ms = NOW;
    t.threads = vec![
        ThreadLine { key: ALEX.into(), unread: 2, time_ms: NOW - 40_000, preview: "are you coming? we're all at the station already".into() },
        ThreadLine { key: group_key(), unread: 0, time_ms: NOW - 1_400_000, preview: "wow".into() },
        ThreadLine { key: CY.into(), unread: 1, time_ms: NOW - 7_300_000, preview: "thanks for the ride!".into() },
        ThreadLine { key: "72000".into(), unread: 0, time_ms: NOW - 3_900_000, preview: "Your code is 4411".into() },
    ];
    t.messages.insert(
        ALEX.into(),
        vec![
            msg(1, true, ALEX, 3_000, "read", "hey, still on for tonight?"),
            msg(2, false, ME, 2_900, "delivered", "yes! the 8 pm train"),
            msg(3, false, ME, 1_200, "sent", "bringing snacks for everyone"),
            msg(4, false, ME, 300, "failed:the phone is off", "see you there"),
            msg(5, true, ALEX, 40, "received", "are you coming? we're all at the station already"),
        ],
    );
    t.messages.insert(
        group_key(),
        vec![
            msg(10, true, BO, 1_900, "read", "who's around this weekend?"),
            msg(11, false, ME, 1_800, "delivered", "me! beach?"),
            MsgLine { id: 12, incoming: true, from: ALEX.into(), time_ms: NOW - 1_500_000, status: "read".into(), mms: true, text: "the view from here".into(), files: vec!["/home/Messages/3900-12/sunset.png".into()] },
            msg(13, true, CY, 1_400, "read", "wow"),
        ],
    );
    t
}

fn files() -> MemFiles {
    let mut f = MemFiles::default();
    let mut b = Book::default();
    b.put(Contact::new("Alex Smith", ALEX));
    b.put(Contact::new("Bo Jones", BO));
    b.save(&mut f).unwrap();
    f.files.insert("/home/Messages/3900-12/sunset.png".into(), picture(96, 64));
    f
}

type A = MessagesApp<FakeTel, MemFiles>;

fn click(a: &mut A, m: &MockConnection, id: &str, t: u64) {
    m.push(&ui_samples::click(hit(&a.app, id), 10, 10));
    a.step(0, t);
    a.step(0, t + 1);
}

fn type_into(a: &mut A, m: &MockConnection, id: &str, text: &str, t: u64) {
    click(a, m, id, t);
    m.push(&[Message::Text(proto::Text { surface: 1, text: text.into() })]);
    a.step(0, t + 3);
    a.step(0, t + 4);
}

#[derive(Clone, Copy)]
enum Shot {
    List,
    Thread,
    Group,
    Compose,
}

fn record(name: &str, w: i32, h: i32, shot: Shot) -> Vec<u8> {
    let mut s = Session::new(w as u32, h as u32);
    shell(&mut s);
    let m = mock(w, h);
    s.open(2, 11, 0, "messages");
    let mut a = MessagesApp::new(Box::new(m.clone()), tel(), files()).unwrap();
    a.app.core.flush().unwrap();
    s.flush_app(2, &m);
    place_full(&mut s, w, h);
    configure(&m, w, h);
    a.step(0, NOW);
    a.step(0, NOW + 1);
    match shot {
        Shot::List => {}
        Shot::Thread => a.model.borrow_mut().open_thread(ALEX, false),
        Shot::Group => a.model.borrow_mut().open_thread(&group_key(), false),
        Shot::Compose => {
            click(&mut a, &m, "new", NOW + 10);
            type_into(&mut a, &m, "to_field", "Bo", NOW + 20);
            click(&mut a, &m, "suggest", NOW + 30);
            type_into(&mut a, &m, "compose_text", "Running about ten minutes late, the train stopped outside the station for a signal problem. Save me a seat and order me the usual, I'll be there soon! Text me when you get there.", NOW + 40);
        }
    }
    a.step(0, NOW + 100);
    a.step(0, NOW + 101);
    a.step(0, NOW + 102);
    s.flush_app(2, &m);
    s.advance(20);
    s.snap(name);
    s.out
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "../src/test/resources/ui-sessions".into());
    std::fs::create_dir_all(&dir).unwrap();
    let sessions: Vec<(&str, Vec<u8>)> = vec![
        ("messages_compact", record("messages_compact", 320, 480, Shot::List)),
        ("messages_medium", record("messages_medium", 640, 400, Shot::Thread)),
        ("messages_wide", record("messages_wide", 1024, 640, Shot::Thread)),
        ("messages_group", record("messages_group", 320, 480, Shot::Group)),
        ("messages_compose", record("messages_compose", 320, 480, Shot::Compose)),
    ];
    for (name, bytes) in sessions {
        let path = format!("{dir}/{name}.session");
        std::fs::write(&path, &bytes).unwrap();
        println!("wrote {path} ({} bytes)", bytes.len());
    }
}
