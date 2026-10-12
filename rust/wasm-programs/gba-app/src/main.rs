//! `gba-app` — the GBA emulator as a GUI app (`dev.ecm.gba`, the ePhone's
//! GBA app; docs/gui/EPHONE.md "GBA"). One window: the game in a `canvas`
//! node (the emulator's frames go straight in with `ui_put_pixels`) with
//! touch buttons drawn around it, so the same window works as a phone app and
//! on a Terminal desktop.
//!
//!   gba-app [rom.gba]        without a ROM: a list of the .gba files in /home
//!
//! Controls, all at once (they add up):
//!
//! * **Touch** (phone open, mouse as a finger): the on-screen D-pad, A, B, L,
//!   R, Start and Select.
//! * **Keyboard** (phone open, or a Terminal): arrows = D-pad, Z = A, X = B,
//!   A = L, S = R, Enter = Start, Backspace = Select.
//! * **In hand** (ePhone hotkey slots): 1 = A, 2 = B, 3 = Left, 4 = Right,
//!   5 = Up, 6 = Start. Down, L, R and Select need the phone open or a
//!   controller.
//! * **A paired Wireless Controller**: as in `gba` (A/B, LB/RB, Start, Back =
//!   Select, D-pad or left stick).
//!
//! It saves which ROM runs (`SaveState`) and comes back to it after a cold
//! start (`Restore`); the game's own save goes next to the ROM (`.sav`).

use ecm_host_abi::gamepad::{self, Button, Gamepad};
use ecm_ui::diff::{SceneNode, TreeState};
use ecm_ui::{ConnError, Connection, Json};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Font, Message, NodeKind, Prop};
use gba::{keys, Emulator, FRAME_RATE, HEIGHT, SAMPLE_RATE, WIDTH};
use std::time::{Duration, Instant};

const APP_ID: &str = "dev.ecm.gba";
const SURFACE: u32 = 1;
const CANVAS: u32 = 1;
/// Hit ids: GBA key bit + 1 for buttons, ROM_HIT + i for list rows.
const ROM_HIT: u32 = 100;

/// HID usages of the keyboard controls.
mod hid {
    pub const A: u16 = 0x04;
    pub const S: u16 = 0x16;
    pub const X: u16 = 0x1B;
    pub const Z: u16 = 0x1D;
    pub const ENTER: u16 = 0x28;
    pub const BACKSPACE: u16 = 0x2A;
    pub const RIGHT: u16 = 0x4F;
    pub const LEFT: u16 = 0x50;
    pub const DOWN: u16 = 0x51;
    pub const UP: u16 = 0x52;
}

/// In-hand hotkey slots 1-6 (the 6-slot limit leaves Down, L, R, Select to the open phone).
const HOTKEYS: [(u8, u16, &str); 6] = [
    (1, keys::A, "GBA A"),
    (2, keys::B, "GBA B"),
    (3, keys::LEFT, "GBA Left"),
    (4, keys::RIGHT, "GBA Right"),
    (5, keys::UP, "GBA Up"),
    (6, keys::START, "GBA Start"),
];

fn key_bit(hid: u16) -> u16 {
    match hid {
        hid::Z => keys::A,
        hid::X => keys::B,
        hid::A => keys::L,
        hid::S => keys::R,
        hid::ENTER => keys::START,
        hid::BACKSPACE => keys::SELECT,
        hid::RIGHT => keys::RIGHT,
        hid::LEFT => keys::LEFT,
        hid::DOWN => keys::DOWN,
        hid::UP => keys::UP,
        _ => 0,
    }
}

fn controller_keys(s: &gamepad::State) -> u16 {
    let mut k = 0;
    for (b, key) in [
        (Button::A, keys::A),
        (Button::B, keys::B),
        (Button::LB, keys::L),
        (Button::RB, keys::R),
        (Button::Start, keys::START),
        (Button::Back, keys::SELECT),
        (Button::DpadUp, keys::UP),
        (Button::DpadDown, keys::DOWN),
        (Button::DpadLeft, keys::LEFT),
        (Button::DpadRight, keys::RIGHT),
    ] {
        if s.is_down(b) {
            k |= key;
        }
    }
    const DEAD: i8 = 64;
    if s.ly > DEAD {
        k |= keys::UP;
    }
    if s.ly < -DEAD {
        k |= keys::DOWN;
    }
    if s.lx > DEAD {
        k |= keys::RIGHT;
    }
    if s.lx < -DEAD {
        k |= keys::LEFT;
    }
    k
}

/// A touch button: GBA key, label, rect (lu), round.
struct Pad {
    key: u16,
    label: &'static str,
    r: (i32, i32, i32, i32),
    round: bool,
}

/// Where the game and the buttons go in a `w` x `h` window. Returns the game
/// view's rect, the canvas scale (1, or 0 = half size), and the buttons.
fn layout(w: i32, h: i32) -> ((i32, i32, i32, i32), bool, Vec<Pad>) {
    let half = w < WIDTH as i32;
    let (gw, gh) = if half { (WIDTH as i32 / 2, HEIGHT as i32 / 2) } else { (WIDTH as i32, HEIGHT as i32) };
    let game = ((w - gw) / 2, 4, gw, gh);
    let top = game.1 + gh + 6;
    let room = (h - top).max(40);
    let u = (room / 6).clamp(10, 30); // button unit
    let mut pads = Vec::new();
    // L and R under the game view.
    pads.push(Pad { key: keys::L, label: "L", r: (4, top, u * 2, u), round: false });
    pads.push(Pad { key: keys::R, label: "R", r: (w - 4 - u * 2, top, u * 2, u), round: false });
    // D-pad on the left.
    let cx = 6 + u * 3 / 2;
    let cy = top + u + 6 + u * 3 / 2;
    pads.push(Pad { key: keys::UP, label: "^", r: (cx - u / 2, cy - u * 3 / 2, u, u), round: false });
    pads.push(Pad { key: keys::DOWN, label: "v", r: (cx - u / 2, cy + u / 2, u, u), round: false });
    pads.push(Pad { key: keys::LEFT, label: "<", r: (cx - u * 3 / 2, cy - u / 2, u, u), round: false });
    pads.push(Pad { key: keys::RIGHT, label: ">", r: (cx + u / 2, cy - u / 2, u, u), round: false });
    // A and B on the right.
    let ax = w - 6 - u - u / 2;
    pads.push(Pad { key: keys::A, label: "A", r: (ax, cy - u, u + u / 4, u + u / 4), round: true });
    pads.push(Pad { key: keys::B, label: "B", r: (ax - u - u / 2, cy - u / 4, u + u / 4, u + u / 4), round: true });
    // Select and Start at the bottom middle.
    let sy = (cy + u * 3 / 2 + 6).min(h - u / 2 - 4);
    pads.push(Pad { key: keys::SELECT, label: "SELECT", r: (w / 2 - u * 2 - 2, sy, u * 2, u / 2 + 2), round: true });
    pads.push(Pad { key: keys::START, label: "START", r: (w / 2 + 2, sy, u * 2, u / 2 + 2), round: true });
    (game, half, pads)
}

struct Client {
    conn: Box<dyn Connection>,
    out: Vec<u8>,
    pending: std::collections::VecDeque<Message>,
}

impl Client {
    fn send(&mut self, m: Message) {
        m.encode(&mut self.out);
    }

    fn flush(&mut self) -> Result<(), ConnError> {
        if self.out.is_empty() {
            return Ok(());
        }
        let b = std::mem::take(&mut self.out);
        self.conn.send(&b)
    }

    fn recv(&mut self, timeout_ms: i32) -> Result<Vec<Message>, ConnError> {
        self.flush()?;
        if self.pending.is_empty() {
            let b = self.conn.recv(timeout_ms)?;
            self.pending.extend(Message::decode_all(&b).map_err(|_| ConnError::Protocol)?);
        }
        Ok(self.pending.drain(..).collect())
    }
}

/// Every `.gba` file in the documents folder (and its `roms/`).
fn find_roms() -> Vec<String> {
    let mut out = Vec::new();
    for dir in ["/home", "/home/roms"] {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.to_lowercase().ends_with(".gba") {
                    out.push(format!("{dir}/{name}"));
                }
            }
        }
    }
    out.sort();
    out
}

struct Game {
    emu: Emulator,
    rom: String,
    save_path: String,
    speaker: Option<ecm_audio::Device>,
}

fn start_game(rom: &str) -> Result<Game, String> {
    let bytes = std::fs::read(rom).map_err(|e| format!("{rom}: {e}"))?;
    let save_path = gba::save_path_for(rom);
    let save = std::fs::read(&save_path).ok();
    let emu = Emulator::new(bytes, None, save.as_deref(), false)?;
    let speaker = ecm_audio::Device::open(None).ok().and_then(|mut d| {
        d.set_format(SAMPLE_RATE, 16, 1).and_then(|_| d.set_latency_ms(100)).ok().map(|_| d)
    });
    println!("{APP_ID}: \"{}\" ({}), {}", emu.title(), emu.code(), if speaker.is_some() { "sound on" } else { "no sound" });
    Ok(Game { emu, rom: rom.to_string(), save_path, speaker })
}

fn text_w(s: &str, size: u8) -> i32 {
    proto::fonts::face(Font::Ui, size).map(|f| f.text_width(s)).unwrap_or(0)
}

fn main() {
    let conn = match ecm_ui::conn::default_connection(proto::host::ROLE_APP) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gba-app: {e} (start the GUI, or use `gba` on the console)");
            std::process::exit(1);
        }
    };
    let mut c = Client { conn, out: Vec::new(), pending: Default::default() };
    // Hello -> Welcome (-> Restore).
    c.send(Message::Hello(proto::Hello { version: proto::VERSION, role: proto::Role::App, capabilities: vec![] }));
    let mut restore: Option<Vec<u8>> = None;
    let mut welcomed = false;
    let mut wants_restore = false;
    let t0 = Instant::now();
    while (!welcomed || (wants_restore && restore.is_none())) && t0.elapsed() < Duration::from_secs(5) {
        for m in c.recv(5000).unwrap_or_default() {
            match m {
                Message::Welcome(w) => {
                    welcomed = true;
                    wants_restore = w.restore_follows;
                }
                Message::Restore(r) => restore = Some(r.data),
                other => c.pending.push_back(other),
            }
        }
    }
    if !welcomed {
        eprintln!("gba-app: no answer from the compositor");
        std::process::exit(1);
    }
    let restored_rom = restore
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| Json::parse(&s).ok())
        .and_then(|j| j.get("rom").and_then(|r| r.as_str().map(String::from)));
    let mut rom_arg = std::env::args().nth(1).or(restored_rom);

    c.send(Message::SurfaceCreate(proto::SurfaceCreate {
        surface: SURFACE,
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
    }));
    for (slot, _, label) in HOTKEYS {
        c.send(Message::HotkeyClaim(proto::HotkeyClaim { slot, label: label.into() }));
    }

    let mut tree = TreeState::new();
    let mut size = (0, 0);
    let mut serial: Option<u32> = None;
    let mut half = false;
    let mut canvas_made: Option<bool> = None; // Some(half) once created
    let mut game: Option<Game> = None;
    let mut error = String::new();
    let mut roms = find_roms();
    let mut held_keys: u16 = 0; // keyboard
    let mut held_touch: u16 = 0;
    let mut held_hot: u16 = 0;
    let mut touch_hit = 0u32;
    let mut pad: Option<Gamepad> = None;
    let mut pad_looked = Instant::now() - Duration::from_secs(10);
    let mut dirty = true;
    let frame_time = Duration::from_secs_f64(1.0 / FRAME_RATE);
    let present_every = Duration::from_millis(33);
    let mut next_frame = Instant::now();
    let mut next_present = Instant::now();
    let mut last_save_check = Instant::now();
    let mut rgb565 = vec![0u8; WIDTH * HEIGHT * 2];
    let mut quit = false;

    if let Some(r) = rom_arg.take() {
        match start_game(&r) {
            Ok(g) => game = Some(g),
            Err(e) => error = e,
        }
    }

    while !quit {
        // Draw the window when something changed.
        if size.0 > 0 && (dirty || serial.is_some()) {
            let (w, h) = size;
            let (g, hv, pads) = layout(w, h);
            half = hv;
            let mut nodes = Vec::new();
            nodes.push(SceneNode::new("bg", NodeKind::Rect, vec![Prop::X(0), Prop::Y(0), Prop::W(w), Prop::H(h), Prop::Fill(0x2b2d42ff)]));
            if game.is_some() {
                if canvas_made != Some(half) {
                    if canvas_made.is_some() {
                        c.send(Message::ResourceDestroy(proto::ResourceDestroy { id: CANVAS }));
                    }
                    let (cw, ch) = if half { (WIDTH as i32 / 2, HEIGHT as i32 / 2) } else { (WIDTH as i32, HEIGHT as i32) };
                    c.send(Message::ResourceCreate(proto::ResourceCreate { id: CANVAS, kind: proto::ResourceKind::Canvas, w: cw, h: ch, format: proto::PixelFormat::Rgb565 }));
                    canvas_made = Some(half);
                    next_present = Instant::now();
                }
                nodes.push(SceneNode::new("bezel", NodeKind::Rect, vec![Prop::X(g.0 - 2), Prop::Y(g.1 - 2), Prop::W(g.2 + 4), Prop::H(g.3 + 4), Prop::Fill(0x111111ff), Prop::Radius(2)]));
                nodes.push(SceneNode::new("game", NodeKind::Canvas, vec![Prop::X(g.0), Prop::Y(g.1), Prop::W(g.2), Prop::H(g.3), Prop::Resource(CANVAS), Prop::Scale(1)]));
                let held = held_keys | held_touch | held_hot;
                for p in &pads {
                    let bit = p.key.trailing_zeros();
                    let down = held & p.key != 0;
                    let fill = if down { 0xef8354ff } else if p.round && (p.key == keys::A || p.key == keys::B) { 0x8d99aeff } else { 0x4a4e69ff };
                    let k = format!("pad{bit}");
                    nodes.push(SceneNode::new(&k, NodeKind::Rect, vec![
                        Prop::X(p.r.0), Prop::Y(p.r.1), Prop::W(p.r.2), Prop::H(p.r.3), Prop::Fill(fill),
                        Prop::Radius(if p.round { (p.r.3 / 2) as u16 } else { 3 }), Prop::HitId(bit + 1),
                    ]));
                    let size = if p.label.len() > 1 { 8 } else if p.r.3 >= 16 { 12 } else { 8 };
                    let tw = text_w(p.label, size);
                    nodes.push(SceneNode::new(&format!("{k}:t"), NodeKind::Text, vec![
                        Prop::X(p.r.0 + (p.r.2 - tw) / 2), Prop::Y(p.r.1 + (p.r.3 - size as i32) / 2), Prop::Text(p.label.into()),
                        Prop::Font(Font::Ui), Prop::Size(size), Prop::Color(0xedf2f4ff), Prop::HitId(bit + 1),
                    ]));
                }
            } else {
                nodes.push(SceneNode::new("title", NodeKind::Text, vec![Prop::X(8), Prop::Y(8), Prop::Text("Choose a game".into()), Prop::Font(Font::Ui), Prop::Size(12), Prop::Color(0xedf2f4ff)]));
                let hint = if roms.is_empty() { "No .gba files in /home" } else { "Tap a game to play" };
                nodes.push(SceneNode::new("hint", NodeKind::Text, vec![Prop::X(8), Prop::Y(24), Prop::Text(hint.into()), Prop::Font(Font::Ui), Prop::Size(8), Prop::Color(0x8d99aeff)]));
                for (i, r) in roms.iter().enumerate().take(12) {
                    let y = 40 + i as i32 * 22;
                    let name = r.rsplit('/').next().unwrap_or(r).to_string();
                    nodes.push(SceneNode::new(&format!("rom{i}"), NodeKind::Rect, vec![Prop::X(6), Prop::Y(y), Prop::W(w - 12), Prop::H(18), Prop::Fill(0x4a4e69ff), Prop::Radius(3), Prop::HitId(ROM_HIT + i as u32)]));
                    nodes.push(SceneNode::new(&format!("rom{i}:t"), NodeKind::Text, vec![Prop::X(12), Prop::Y(y + 3), Prop::Text(name), Prop::Font(Font::Ui), Prop::Size(12), Prop::Color(0xedf2f4ff), Prop::HitId(ROM_HIT + i as u32)]));
                }
                if !error.is_empty() {
                    nodes.push(SceneNode::new("err", NodeKind::Text, vec![Prop::X(8), Prop::Y(h - 14), Prop::Text(error.chars().take(60).collect()), Prop::Font(Font::Ui), Prop::Size(8), Prop::Color(0xef8354ff)]));
                }
            }
            let ops = tree.update(&nodes);
            if !ops.is_empty() {
                c.send(Message::TreeOps(proto::TreeOps { surface: SURFACE, ops }));
            }
            if let Some(s) = serial.take() {
                c.send(Message::AckConfigure(proto::AckConfigure { surface: SURFACE, serial: s }));
            }
            c.send(Message::Commit(proto::Commit { surface: SURFACE }));
            dirty = false;
        }

        // Wait for input until the next emulated frame.
        let now = Instant::now();
        let wait = if game.is_some() { next_frame.saturating_duration_since(now).as_millis() as i32 } else { 500 };
        let msgs = match c.recv(wait) {
            Ok(m) => m,
            Err(_) => break,
        };
        let before = held_keys | held_touch | held_hot;
        for m in msgs {
            match m {
                Message::Configure(cf) if cf.surface == SURFACE => {
                    size = (cf.w, cf.h);
                    serial = Some(cf.serial);
                }
                Message::Key(k) => {
                    let bit = key_bit(k.key);
                    if k.down {
                        held_keys |= bit;
                    } else {
                        held_keys &= !bit;
                    }
                }
                Message::Hotkey(hk) => {
                    if let Some(&(_, bit, _)) = HOTKEYS.iter().find(|(s, _, _)| *s == hk.slot) {
                        if hk.down {
                            held_hot |= bit;
                        } else {
                            held_hot &= !bit;
                        }
                    }
                }
                Message::Focus(f) if !f.gained => {
                    held_keys = 0;
                    held_touch = 0;
                }
                Message::Pointer(p) => match p.kind {
                    proto::PointerKind::Down => {
                        touch_hit = p.hit_id;
                        if game.is_none() && p.hit_id >= ROM_HIT {
                            if let Some(r) = roms.get((p.hit_id - ROM_HIT) as usize).cloned() {
                                match start_game(&r) {
                                    Ok(g) => {
                                        game = Some(g);
                                        next_frame = Instant::now();
                                    }
                                    Err(e) => error = e,
                                }
                                dirty = true;
                            }
                        } else if (1..=10).contains(&p.hit_id) {
                            held_touch = 1 << (p.hit_id - 1);
                        }
                    }
                    proto::PointerKind::Up => {
                        if touch_hit != 0 {
                            held_touch = 0;
                            touch_hit = 0;
                        }
                    }
                    _ => {}
                },
                Message::WillSuspend(_) => {
                    let state = match &game {
                        Some(g) => Json::Obj(vec![("rom".into(), Json::Str(g.rom.clone()))]),
                        None => Json::Obj(vec![]),
                    };
                    if let Some(g) = game.as_mut() {
                        if let Some(data) = g.emu.take_save_if_changed() {
                            let _ = std::fs::write(&g.save_path, data);
                        }
                    }
                    held_keys = 0;
                    held_touch = 0;
                    held_hot = 0;
                    c.send(Message::SaveState(proto::SaveState { data: state.to_string().into_bytes() }));
                    let _ = c.flush();
                    // Resumed later: don't try to catch up the frames we missed.
                    next_frame = Instant::now();
                }
                Message::CloseRequested(_) => quit = true,
                Message::Error(e) => {
                    eprintln!("gba-app: {}", e.message);
                    quit = true;
                }
                _ => {}
            }
        }
        if (held_keys | held_touch | held_hot) != before {
            dirty = true;
        }
        if game.is_none() && roms.is_empty() && pad_looked.elapsed() > Duration::from_secs(3) {
            roms = find_roms();
            pad_looked = Instant::now();
            if !roms.is_empty() {
                dirty = true;
            }
        }
        let Some(g) = game.as_mut() else { continue };

        // A controller: looked for every few seconds until one is there.
        if pad.is_none() && pad_looked.elapsed() > Duration::from_secs(3) {
            pad_looked = Instant::now();
            pad = gamepad::first();
        }
        let mut held = held_keys | held_touch | held_hot;
        if let Some(p) = &pad {
            match p.poll() {
                Ok(s) => held |= controller_keys(&s),
                Err(_) => pad = None,
            }
        }
        if held & (keys::UP | keys::DOWN) == keys::UP | keys::DOWN {
            held &= !(keys::UP | keys::DOWN);
        }
        if held & (keys::LEFT | keys::RIGHT) == keys::LEFT | keys::RIGHT {
            held &= !(keys::LEFT | keys::RIGHT);
        }

        let now = Instant::now();
        if now < next_frame {
            continue;
        }
        g.emu.run_frame(held);
        let samples = g.emu.take_audio();
        let paced = match &mut g.speaker {
            Some(dev) => dev.write_samples(&samples).is_ok(),
            None => false,
        };
        next_frame += frame_time;
        let now = Instant::now();
        if paced || now.duration_since(next_frame) > Duration::from_millis(250) {
            if now > next_frame {
                next_frame = now;
            }
        }
        if now >= next_present && canvas_made.is_some() {
            g.emu.framebuffer_rgb565(&mut rgb565);
            let pixels: Vec<u8> = if half {
                let mut out = Vec::with_capacity(WIDTH * HEIGHT / 2);
                for y in (0..HEIGHT).step_by(2) {
                    for x in (0..WIDTH).step_by(2) {
                        let i = (y * WIDTH + x) * 2;
                        out.extend_from_slice(&rgb565[i..i + 2]);
                    }
                }
                out
            } else {
                rgb565.clone()
            };
            if c.flush().is_err() || c.conn.put_pixels(CANVAS, &pixels, &[]).is_err() {
                break;
            }
            c.send(Message::Commit(proto::Commit { surface: SURFACE }));
            next_present = now + present_every;
        }
        if now.duration_since(last_save_check) >= Duration::from_secs(1) {
            last_save_check = now;
            if let Some(data) = g.emu.take_save_if_changed() {
                let _ = std::fs::write(&g.save_path, data);
            }
        }
    }
    if let Some(g) = game.as_mut() {
        if let Some(data) = g.emu.take_save_if_changed() {
            let _ = std::fs::write(&g.save_path, data);
        }
    }
    let _ = c.flush();
}
