//! `gba-app` — the GBA emulator as a GUI app (`dev.ecm.gba`, the ePhone's
//! GBA app; docs/gui/EPHONE.md "GBA"). One window: the game in a `canvas`
//! node (the emulator's frames go straight in with `ui_put_pixels`) with
//! touch buttons drawn around it, so the same window works as a phone app and
//! on a Terminal desktop.
//!
//!   gba-app [rom.gba]        without a ROM: a list of the .gba files in /home
//!
//! The list has a **Settings** page to choose the keyboard keys; while a game
//! runs, the **GAMES** button goes back to the list (screens and bindings:
//! `lib.rs`).
//!
//! Controls, all at once (they add up):
//!
//! * **Touch** (phone open, mouse as a finger): the on-screen D-pad, A, B, L,
//!   R, Start and Select.
//! * **Keyboard** (phone open, or a Terminal): by default arrows = D-pad,
//!   Z = A, X = B, A = L, S = R, Enter = Start, Backspace = Select; Settings
//!   changes them (saved in `/var/apps/dev.ecm.gba/keys.json`).
//! * **In hand** (ePhone hotkey slots): 1 = A, 2 = B, 3 = Left, 4 = Right,
//!   5 = Up, 6 = Start. Down, L, R and Select need the phone open or a
//!   controller.
//! * **A paired Wireless Controller**: as in `gba` (A/B, LB/RB, Start, Back =
//!   Select, D-pad or left stick).
//!
//! It saves which ROM runs (`SaveState`) and comes back to it after a cold
//! start (`Restore`); the game's own save goes next to the ROM (`.sav`).

use ecm_host_abi::gamepad::{self, Button, Gamepad};
use ecm_ui::diff::TreeState;
use ecm_ui::{ConnError, Connection, Json};
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use gba::{keys, Emulator, FRAME_RATE, HEIGHT, SAMPLE_RATE, WIDTH};
use gba_app::{layout, Action, Screen, Ui, APP_ID, KEYS_FILE};
use std::time::{Duration, Instant};

const SURFACE: u32 = 1;
const CANVAS: u32 = 1;

/// In-hand hotkey slots 1-6 (the 6-slot limit leaves Down, L, R, Select to the open phone).
const HOTKEYS: [(u8, u16, &str); 6] = [
    (1, keys::A, "GBA A"),
    (2, keys::B, "GBA B"),
    (3, keys::LEFT, "GBA Left"),
    (4, keys::RIGHT, "GBA Right"),
    (5, keys::UP, "GBA Up"),
    (6, keys::START, "GBA Start"),
];

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
    let mut ui = Ui::new(KEYS_FILE);
    ui.roms = find_roms();
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
            Ok(g) => {
                game = Some(g);
                ui.screen = Screen::Playing;
            }
            Err(e) => ui.error = e,
        }
    }

    while !quit {
        // Draw the window when something changed.
        if size.0 > 0 && (dirty || serial.is_some()) {
            let (w, h) = size;
            let (_, hv, _, _) = layout(w, h);
            half = hv;
            if game.is_some() && canvas_made != Some(half) {
                if canvas_made.is_some() {
                    c.send(Message::ResourceDestroy(proto::ResourceDestroy { id: CANVAS }));
                }
                let (cw, ch) = if half { (WIDTH as i32 / 2, HEIGHT as i32 / 2) } else { (WIDTH as i32, HEIGHT as i32) };
                c.send(Message::ResourceCreate(proto::ResourceCreate { id: CANVAS, kind: proto::ResourceKind::Canvas, w: cw, h: ch, format: proto::PixelFormat::Rgb565 }));
                canvas_made = Some(half);
                next_present = Instant::now();
            }
            let nodes = ui.nodes(w, h, held_keys | held_touch | held_hot);
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
                    if ui.settings_key(k.key, k.down) {
                        dirty = true;
                        continue;
                    }
                    let bit = if ui.screen == Screen::Playing { ui.keys.bit_for(k.key) } else { 0 };
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
                        if ui.screen == Screen::Playing && (1..=10).contains(&p.hit_id) {
                            held_touch = 1 << (p.hit_id - 1);
                            continue;
                        }
                        match ui.pointer_down(p.hit_id) {
                            Action::Play(r) => match start_game(&r) {
                                Ok(g) => {
                                    game = Some(g);
                                    ui.screen = Screen::Playing;
                                    ui.error.clear();
                                    next_frame = Instant::now();
                                }
                                Err(e) => ui.error = e,
                            },
                            Action::BackToGames => {
                                // Keep the cartridge save, then back to the list.
                                if let Some(mut g) = game.take() {
                                    if let Some(data) = g.emu.take_save_if_changed() {
                                        let _ = std::fs::write(&g.save_path, data);
                                    }
                                }
                                held_keys = 0;
                                held_touch = 0;
                                held_hot = 0;
                                touch_hit = 0;
                                ui.screen = Screen::Games;
                                ui.roms = find_roms();
                            }
                            Action::None => {}
                        }
                        dirty = true;
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
        if game.is_none() && ui.roms.is_empty() && pad_looked.elapsed() > Duration::from_secs(3) {
            ui.roms = find_roms();
            pad_looked = Instant::now();
            if !ui.roms.is_empty() {
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
