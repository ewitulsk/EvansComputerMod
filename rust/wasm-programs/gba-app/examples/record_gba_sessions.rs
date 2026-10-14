//! Records the GBA app sessions that the Java golden-image test
//! (UiGoldenTest) replays through the compositor and rasterizer:
//!
//!   cargo run -p gba-app --example record_gba_sessions -- <out dir>
//!   (the checked-in copies live in src/test/resources/ui-sessions/)
//!
//! Same file format as ecm-ui's `record_ui_sessions` (see there). A shell
//! places the window full screen; the screens are the app's own (`lib.rs`),
//! sent the way `main.rs` sends them.
//!
//! * `gba_games` (240x360, a phone-shaped display): the ROM list with its
//!   Settings button (`gba_games`), then Settings with Start rebound to
//!   Space and A refused the same key (`gba_settings`).
//! * `gba_playing` (640x400, a Terminal): the test ROM running, with the
//!   GAMES button between L and R.

use ecm_ui::diff::TreeState;
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use gba::{Emulator, HEIGHT, WIDTH};
use gba_app::{Screen, Ui, BIND_HIT, SETTINGS_HIT};

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
    fn send(&mut self, conn: u32, msgs: &[Message]) {
        let b = proto::encode_all(msgs);
        self.out.push(2);
        self.u32(conn);
        self.bytes(&b);
    }
    fn pixels(&mut self, conn: u32, target: u32, b: &[u8]) {
        self.out.push(3);
        self.u32(conn);
        self.u32(target);
        self.u32(0);
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
}

/// A shell placing window 1 full screen; the app's hello and window.
fn start(w: i32, h: i32) -> Session {
    let mut s = Session::new(w as u32, h as u32);
    s.open(1, 10, 1, "shell");
    s.send(1, &[Message::Hello(proto::Hello { version: proto::VERSION, role: proto::Role::Shell, capabilities: vec![] })]);
    s.open(2, 11, 0, "gba-app");
    s.send(2, &[
        Message::Hello(proto::Hello { version: proto::VERSION, role: proto::Role::App, capabilities: vec![] }),
        Message::SurfaceCreate(proto::SurfaceCreate {
            surface: 1,
            kind: proto::SurfaceKind::Tree,
            role: proto::SurfaceRole::Window,
            layer: proto::Layer::None,
            parent: 0,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            format: proto::PixelFormat::None,
            scale: 1,
            title: "GBA".into(),
        }),
    ]);
    s.send(1, &[Message::Place(proto::Place {
        window: 1,
        x: 0,
        y: 0,
        w,
        h,
        states: proto::window_states::FULLSCREEN,
        transition: proto::Transition { ms: 0, easing: proto::Easing::Linear },
    })]);
    s
}

/// Draw the app's screen as main.rs does (the first draw acks the configure).
fn draw(s: &mut Session, tree: &mut TreeState, ui: &Ui, w: i32, h: i32, first: bool) {
    let ops = tree.update(&ui.nodes(w, h, 0));
    let mut msgs = vec![Message::TreeOps(proto::TreeOps { surface: 1, ops })];
    if first {
        msgs.push(Message::AckConfigure(proto::AckConfigure { surface: 1, serial: 1 }));
    }
    msgs.push(Message::Commit(proto::Commit { surface: 1 }));
    s.send(2, &msgs);
}

fn keys_path(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ecm-record-gba-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d.join("keys.json")
}

fn games() -> Vec<u8> {
    let (w, h) = (240, 360);
    let mut s = start(w, h);
    let mut tree = TreeState::new();
    let mut ui = Ui::new(keys_path("games"));
    ui.roms = vec!["/home/arm.gba".into(), "/home/roms/pinball.gba".into(), "/home/roms/racer.gba".into()];
    draw(&mut s, &mut tree, &ui, w, h, true);
    s.advance(20);
    s.snap("gba_games");
    ui.pointer_down(SETTINGS_HIT);
    ui.pointer_down(BIND_HIT + 4); // Start
    ui.settings_key(0x2C, true); // Space
    ui.pointer_down(BIND_HIT); // A
    ui.settings_key(0x2C, true); // Space again: refused, A still waits
    assert_eq!(ui.screen, Screen::Settings);
    draw(&mut s, &mut tree, &ui, w, h, false);
    s.advance(20);
    s.snap("gba_settings");
    s.out
}

fn playing() -> Vec<u8> {
    let (w, h) = (640, 400);
    let mut s = start(w, h);
    let mut tree = TreeState::new();
    let mut ui = Ui::new(keys_path("playing"));
    ui.screen = Screen::Playing;
    let rom = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../gba/tests/roms/arm.gba")).expect("the gba test ROM");
    let mut emu = Emulator::new(rom, None, None, false).expect("the test ROM starts");
    for _ in 0..90 {
        emu.run_frame(0);
    }
    let mut fb = vec![0u8; WIDTH * HEIGHT * 2];
    emu.framebuffer_rgb565(&mut fb);
    s.send(2, &[Message::ResourceCreate(proto::ResourceCreate { id: 1, kind: proto::ResourceKind::Canvas, w: WIDTH as i32, h: HEIGHT as i32, format: proto::PixelFormat::Rgb565 })]);
    draw(&mut s, &mut tree, &ui, w, h, true);
    s.pixels(2, 1, &fb);
    s.send(2, &[Message::Commit(proto::Commit { surface: 1 })]);
    s.advance(20);
    s.snap("gba_playing");
    s.out
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "../src/test/resources/ui-sessions".into());
    std::fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in [("gba_games", games()), ("gba_playing", playing())] {
        let path = format!("{dir}/{name}.session");
        std::fs::write(&path, &bytes).unwrap();
        println!("wrote {path} ({} bytes)", bytes.len());
    }
}
