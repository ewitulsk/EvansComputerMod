//! Records the phone shell's golden-image sessions (ROADMAP P5): the shell's
//! own policy (`phone_shell::PhoneShell`) with a fixed clock, status and app
//! list, driven through a mock connection; the Java `UiGoldenTest` replays
//! them through the compositor and rasterizer at 240x400.
//!
//!   cargo run -p phone --example record_phone_sessions -- <out dir>
//!   (the checked-in copies live in src/test/resources/ui-sessions/)
//!
//! Sessions: `phone_home` (home grid, status bar with Wi-Fi, cellular,
//! battery and an audio indicator, nav bar; a tap on Notes hits its icon),
//! `phone_sheet` (the notification sheet pulled down over home),
//! `phone_dialog` (the permission dialog). Same file format as ecm-ui's
//! `record_ui_sessions`.

use ecm_ui::shell::ShellEvent;
use ecm_ui::MockConnection;
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use phone_shell::{Action, AppEntry, Env, PhoneShell, Status};

const NOTES_ICON: &[u8] = include_bytes!("../../../../src/main/resources/assets/evanscomputermod/ecm_apps/dev.ecm.notes/icon.png");
const GBA_ICON: &[u8] = include_bytes!("../../../../src/main/resources/assets/evanscomputermod/ecm_apps/dev.ecm.gba/icon.png");

struct Fixed;

impl Env for Fixed {
    fn clock(&mut self) -> String {
        "09:41".into()
    }

    fn status(&mut self) -> Status {
        Status::parse("state awake\nbattery 76.5\nwifi 3\ncell 2\nforeground -\nmode dev.ecm.music audio\nhotkey dev.ecm.notes 1 - Add note\n")
    }

    fn apps(&mut self) -> Vec<AppEntry> {
        vec![
            AppEntry { id: "dev.ecm.notes".into(), name: "Notes".into(), icon: Some(NOTES_ICON.to_vec()) },
            AppEntry { id: "dev.ecm.gba".into(), name: "GBA".into(), icon: Some(GBA_ICON.to_vec()) },
            AppEntry { id: "dev.ecm.music".into(), name: "Music".into(), icon: None },
            AppEntry { id: "dev.evan.weather".into(), name: "Weather Station".into(), icon: None },
        ]
    }

    fn session(&mut self) -> Vec<String> {
        Vec::new()
    }
}

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
    /// What the shell sent since the last call, then its uploads.
    fn flush(&mut self, mock: &MockConnection) {
        let sent = mock.sent();
        mock.clear_sent();
        if !sent.is_empty() {
            Message::decode_all(&sent).expect("only valid messages are recorded");
            self.out.push(2);
            self.u32(1);
            self.bytes(&sent);
        }
        let uploads = std::mem::take(&mut mock.state.borrow_mut().uploads);
        for (res, kind, data) in uploads {
            self.out.push(4);
            self.u32(1);
            self.u32(res);
            self.u32(kind as u32);
            self.bytes(&data);
        }
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
    fn tap(&mut self, x: i32, y: i32, hit: u32) {
        for (kind, buttons, button) in [(proto::PointerKind::Down, 1u8, proto::Button::Left), (proto::PointerKind::Up, 0u8, proto::Button::Left)] {
            self.out.push(7);
            self.out.push(kind as u8);
            self.i32(x);
            self.i32(y);
            self.out.push(buttons);
            self.out.push(button as u8);
            self.i32(0);
            self.u32(1);
            self.u32(hit);
        }
    }
}

fn welcome() -> Message {
    let mut m = ecm_ui::samples::welcome(false);
    if let Message::Welcome(ref mut w) = m {
        w.display.w = 240;
        w.display.h = 400;
        w.display.scale = 1;
        w.display.size_class = proto::SizeClass::Compact;
        w.display.device = proto::Device::Ephone;
    }
    m
}

fn configure(ps: &mut PhoneShell<Fixed>, surface: u32, w: i32, h: i32) {
    ps.handle(vec![ShellEvent::Configure(proto::Configure { surface, serial: 1, w, h, states: 0 })]);
}

/// A shell on the home screen, recorded up to its first frame.
fn start(s: &mut Session) -> (PhoneShell<Fixed>, MockConnection) {
    let mock = MockConnection::new();
    mock.push(&[welcome()]);
    s.open(1, 1, 1, "phone");
    let mut ps = PhoneShell::new(Box::new(mock.clone()), Fixed, true).expect("connect");
    configure(&mut ps, 1, 240, 400);
    configure(&mut ps, 2, 240, 16);
    configure(&mut ps, 3, 240, 20);
    ps.render().unwrap();
    s.flush(&mock);
    s.advance(40);
    (ps, mock)
}

fn notifications(ps: &mut PhoneShell<Fixed>) {
    for (nid, app, title, body) in [
        (1, "dev.ecm.music", "Now playing", "Creeper Shuffle - C418"),
        (2, "dev.evan.weather", "Rain at 14:00", "Take an umbrella, or a roof"),
    ] {
        ps.handle(vec![ShellEvent::Message(Message::NotificationPosted(proto::NotificationPosted {
            nid,
            app_id: app.into(),
            tag: format!("t{nid}"),
            title: title.into(),
            body: body.into(),
            icon: String::new(),
            actions: Vec::new(),
        }))]);
    }
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "src/test/resources/ui-sessions".into());
    std::fs::create_dir_all(&out).unwrap();

    // Home, with an unread notification; a tap on the Notes icon hits it.
    let mut s = Session::new(240, 400, 1);
    let (mut ps, mock) = start(&mut s);
    notifications(&mut ps);
    ps.render().unwrap();
    s.flush(&mock);
    s.advance(40);
    s.snap("phone_home");
    let hit = ps.hit_of(&Action::Launch("dev.ecm.notes".into())).expect("Notes drawn");
    // Notes is the third app by name (GBA, Music, Notes, ...): x 150 +- 16, y 16 + 10 + 16.
    s.tap(150, 42, hit);
    std::fs::write(format!("{out}/phone_home.session"), &s.out).unwrap();

    // The notification sheet, pulled down over home.
    let mut s = Session::new(240, 400, 1);
    let (mut ps, mock) = start(&mut s);
    notifications(&mut ps);
    ps.act(Action::PullSheet);
    ps.render().unwrap();
    s.flush(&mock);
    configure(&mut ps, 4, 240, 400);
    ps.render().unwrap();
    s.flush(&mock);
    s.advance(40);
    s.snap("phone_sheet");
    std::fs::write(format!("{out}/phone_sheet.session"), &s.out).unwrap();

    // The permission dialog.
    let mut s = Session::new(240, 400, 1);
    let (mut ps, mock) = start(&mut s);
    ps.handle(vec![ShellEvent::Message(Message::PermissionPrompt(proto::PermissionPrompt {
        req: 1,
        app_id: "dev.ecm.gba".into(),
        permission: "files:documents".into(),
    }))]);
    ps.render().unwrap();
    s.flush(&mock);
    configure(&mut ps, 4, 240, 400);
    ps.render().unwrap();
    s.flush(&mock);
    s.advance(40);
    s.snap("phone_dialog");
    std::fs::write(format!("{out}/phone_dialog.session"), &s.out).unwrap();
    println!("wrote phone_home, phone_sheet, phone_dialog to {out}");
}
