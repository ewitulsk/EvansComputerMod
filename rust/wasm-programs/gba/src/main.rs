//! `gba` — a Game Boy Advance emulator for the computer.
//!
//! Usage:
//!   gba <rom.gba> [screen] [options]
//!
//! Options:
//!   screen               show the game on the attached Screen cluster instead of the terminal
//!   --controller N       use player N's controller (default: the first connected)
//!   --speaker SIDE       play sound on that speaker (default: the first one, if any)
//!   --no-sound           don't use a speaker
//!   --fps N              display refresh to ask for (default 30; the server caps it)
//!   --bios FILE          BIOS image (default: gba_bios.bin or bios/gba_bios.bin if present,
//!                        else the bundled open-source BIOS)
//!   --boot               run the BIOS start-up instead of jumping into the game
//!   --save FILE          save file (default: the ROM's path with .sav)
//!   --benchmark N        run N frames as fast as possible (no display or sound), print the speed
//!
//! Controls (Wireless Xbox Controller): A/B = A/B, LB/RB = L/R, Start = Start,
//! Back = Select, D-pad or left stick = D-pad, Guide = quit.
//! Without a controller, the keyboard: arrows, Z = A, X = B, A = L, S = R,
//! Enter = Start, Backspace = Select, Q = quit. Ctrl+T also quits.
//!
//! The emulated machine runs at its real speed (59.73 frames/s): paced by the
//! speaker's audio clock when there is one, by the wall clock otherwise. The
//! display shows the newest frame at each refresh. The game's save data is
//! written to the .sav file a moment after the game saves, and on quit.

use ecm_host_abi::gamepad::{self, Button, Gamepad};
use ecm_host_abi::gfx_child::{self, FLAG_DOUBLE_BUFFER, FORMAT_RGB565};
use ecm_host_abi::video::Target;
use gba::{keys, Emulator, FRAME_RATE, HEIGHT, SAMPLE_RATE, WIDTH};
use std::io::Read;
use std::time::{Duration, Instant};

struct Options {
    rom: String,
    target: Target,
    controller: Option<u8>,
    speaker: Option<String>,
    sound: bool,
    fps: u32,
    bios: Option<String>,
    boot: bool,
    save: Option<String>,
    benchmark: Option<u64>,
}

fn usage() -> ! {
    eprintln!("usage: gba <rom.gba> [screen] [--controller N] [--speaker SIDE] [--no-sound]");
    eprintln!("           [--fps N] [--bios FILE] [--boot] [--save FILE]");
    std::process::exit(1);
}

fn parse_args() -> Options {
    let mut o = Options {
        rom: String::new(),
        target: Target::Terminal,
        controller: None,
        speaker: None,
        sound: true,
        fps: 30,
        bios: None,
        boot: false,
        save: None,
        benchmark: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "screen" => o.target = Target::Screen,
            "--controller" => o.controller = Some(args.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| usage())),
            "--speaker" => o.speaker = Some(args.next().unwrap_or_else(|| usage())),
            "--no-sound" => o.sound = false,
            "--fps" => o.fps = args.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| usage()),
            "--bios" => o.bios = Some(args.next().unwrap_or_else(|| usage())),
            "--boot" => o.boot = true,
            "--save" => o.save = Some(args.next().unwrap_or_else(|| usage())),
            "--benchmark" => o.benchmark = Some(args.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| usage())),
            "-h" | "--help" => usage(),
            f if o.rom.is_empty() && !f.starts_with('-') => o.rom = f.to_string(),
            _ => usage(),
        }
    }
    if o.rom.is_empty() {
        usage();
    }
    o
}

#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn fd_fdstat_set_flags(fd: u32, flags: u32) -> u32;
}

/// Keyboard input from the terminal. Keys arrive as presses only (no
/// releases), so a pressed key is held for a few frames.
struct Keyboard {
    held: [u8; 10],
    quit: bool,
    esc: Vec<u8>,
}

impl Keyboard {
    const HOLD_FRAMES: u8 = 8;

    fn new() -> Keyboard {
        unsafe {
            fd_fdstat_set_flags(0, 4); // O_NONBLOCK
        }
        Keyboard { held: [0; 10], quit: false, esc: Vec::new() }
    }

    fn press(&mut self, key: u16) {
        let bit = key.trailing_zeros() as usize;
        if bit < self.held.len() {
            self.held[bit] = Self::HOLD_FRAMES;
        }
    }

    /// Read what was typed since the last frame; returns the held keys.
    fn poll(&mut self) -> u16 {
        let mut buf = [0u8; 64];
        if let Ok(n) = std::io::stdin().read(&mut buf) {
            for &b in &buf[..n] {
                if !self.esc.is_empty() || b == 0x1b {
                    self.esc.push(b);
                    if self.esc.len() == 3 {
                        match self.esc[2] {
                            b'A' => self.press(keys::UP),
                            b'B' => self.press(keys::DOWN),
                            b'C' => self.press(keys::RIGHT),
                            b'D' => self.press(keys::LEFT),
                            _ => {}
                        }
                        self.esc.clear();
                    }
                    continue;
                }
                match b {
                    b'z' | b'Z' => self.press(keys::A),
                    b'x' | b'X' => self.press(keys::B),
                    b'a' | b'A' => self.press(keys::L),
                    b's' | b'S' => self.press(keys::R),
                    b'\n' | b'\r' => self.press(keys::START),
                    0x08 | 0x7f => self.press(keys::SELECT),
                    b'q' | b'Q' => self.quit = true,
                    _ => {}
                }
            }
        }
        let mut mask = 0;
        for (bit, h) in self.held.iter_mut().enumerate() {
            if *h > 0 {
                mask |= 1 << bit;
                *h -= 1;
            }
        }
        mask
    }
}

/// Controller -> GBA buttons. Also returns whether Guide (quit) is held.
fn controller_keys(s: &gamepad::State) -> (u16, bool) {
    let mut k = 0;
    let map = [
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
    ];
    for (b, key) in map {
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
    // Opposite directions at once confuse some games.
    if k & (keys::UP | keys::DOWN) == keys::UP | keys::DOWN {
        k &= !(keys::UP | keys::DOWN);
    }
    if k & (keys::LEFT | keys::RIGHT) == keys::LEFT | keys::RIGHT {
        k &= !(keys::LEFT | keys::RIGHT);
    }
    (k, s.is_down(Button::Guide))
}

fn find_controller(player: Option<u8>) -> Option<Gamepad> {
    match player {
        Some(n) => gamepad::player(n),
        None => gamepad::first(),
    }
}

fn load_bios(path: &Option<String>) -> Option<Vec<u8>> {
    let candidates: Vec<String> = match path {
        Some(p) => vec![p.clone()],
        None => vec!["gba_bios.bin".into(), "bios/gba_bios.bin".into()],
    };
    for c in &candidates {
        if let Ok(b) = std::fs::read(c) {
            if b.len() == 16 * 1024 {
                println!("gba: BIOS {}", c);
                return Some(b);
            }
            eprintln!("gba: {} is not a 16 KiB BIOS image, ignoring it", c);
        } else if path.is_some() {
            eprintln!("gba: can't read BIOS {}", c);
            std::process::exit(1);
        }
    }
    println!("gba: using the bundled open-source BIOS (Cult-of-GBA)");
    None
}

fn write_save(path: &str, data: &[u8]) {
    if let Err(e) = std::fs::write(path, data) {
        eprintln!("gba: can't write {}: {}", path, e);
    }
}

#[inline(never)]
fn benchmark(emu: &mut Emulator, n: u64) {
    let t = Instant::now();
    for _ in 0..n {
        emu.run_frame(0);
        emu.take_audio();
    }
    let secs = t.elapsed().as_secs_f64();
    println!("gba: benchmark: {} frames in {:.2} s = {:.1} fps ({:.2}x real time)",
             n, secs, n as f64 / secs, n as f64 / secs / FRAME_RATE);
}

fn main() {
    let o = parse_args();
    let rom = match std::fs::read(&o.rom) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("gba: {}: {}", o.rom, e);
            eprintln!("  ROM files go in this computer's folder on the server (computer-data/<id>/).");
            std::process::exit(1);
        }
    };
    let save_path = o.save.clone().unwrap_or_else(|| gba::save_path_for(&o.rom));
    let save = std::fs::read(&save_path).ok();
    let bios = load_bios(&o.bios);

    let mut emu = match Emulator::new(rom, bios, save.as_deref(), o.boot) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("gba: {}: {}", o.rom, e);
            std::process::exit(1);
        }
    };
    println!("gba: \"{}\" ({}), save: {}{}", emu.title(), emu.code(), emu.save_kind(),
             if save.is_some() { format!(" (loaded {})", save_path) } else { String::new() });

    if let Some(n) = o.benchmark {
        benchmark(&mut emu, n);
        return;
    }

    // Display.
    if o.target == Target::Screen {
        if gfx_child::screen_dims().is_none() {
            eprintln!("gba: no Screen attached to this computer");
            std::process::exit(1);
        }
        gfx_child::set_screen_power(true);
    }
    if let Err(e) = gfx_child::init2(o.target, WIDTH as i32, HEIGHT as i32, FORMAT_RGB565, FLAG_DOUBLE_BUFFER) {
        eprintln!("gba: can't open the display: {:?}", e);
        std::process::exit(1);
    }
    let refresh = gfx_child::set_refresh(o.target, o.fps.max(1)).unwrap_or(30).max(1);

    // Sound.
    let mut speaker = if o.sound {
        match ecm_audio::Device::open(o.speaker.as_deref()) {
            Ok(mut dev) => match dev.set_format(SAMPLE_RATE, 16, 1).and_then(|_| dev.set_latency_ms(100)) {
                Ok(()) => Some(dev),
                Err(e) => {
                    eprintln!("gba: {}: {}", dev.name(), e);
                    None
                }
            },
            Err(_) if o.speaker.is_some() => {
                eprintln!("gba: no speaker on {}", o.speaker.as_deref().unwrap_or(""));
                None
            }
            Err(_) => None,
        }
    } else {
        None
    };

    let mut pad = find_controller(o.controller);
    let mut keyboard = Keyboard::new();
    println!(
        "gba: {} at {} Hz, {}, {}. {}",
        if o.target == Target::Screen { "Screen" } else { "terminal" },
        refresh,
        speaker.as_ref().map(|s| format!("sound on {}", s.name())).unwrap_or_else(|| "no sound".into()),
        pad.as_ref().map(|p| format!("controller {}", p.player())).unwrap_or_else(|| "keyboard".into()),
        if pad.is_some() { "Guide quits." } else { "Q quits." },
    );

    let frame_time = Duration::from_secs_f64(1.0 / FRAME_RATE);
    let present_every = Duration::from_secs_f64(1.0 / refresh as f64);
    let mut rgb565 = vec![0u8; WIDTH * HEIGHT * 2];
    let start = Instant::now();
    let mut next_frame = start;
    let mut next_present = start;
    let mut frames: u64 = 0;
    let mut last_save_check = start;

    loop {
        // Input: the controller if there is one, else the keyboard.
        if pad.is_none() && frames % 60 == 0 {
            pad = find_controller(o.controller);
        }
        let mut held = keyboard.poll();
        let mut quit = keyboard.quit;
        if let Some(p) = &pad {
            match p.poll() {
                Ok(s) => {
                    let (k, guide) = controller_keys(&s);
                    held |= k;
                    quit |= guide;
                }
                Err(_) => pad = None, // disconnected; look again later
            }
        }
        if quit {
            break;
        }

        emu.run_frame(held);
        frames += 1;
        let samples = emu.take_audio();

        // Show the newest frame once per display refresh.
        let now = Instant::now();
        if now >= next_present {
            emu.framebuffer_rgb565(&mut rgb565);
            if gfx_child::blit_rect(o.target, 0, 0, WIDTH as i32, HEIGHT as i32, &rgb565, FORMAT_RGB565).is_err()
                || gfx_child::present(o.target, 0).is_err()
            {
                break; // display taken away (Ctrl+T)
            }
            next_present += present_every;
            if next_present < now {
                next_present = now + present_every;
            }
        }

        // Pace: the speaker's clock when there is sound (the write waits for
        // room), the wall clock otherwise.
        let paced_by_audio = match &mut speaker {
            Some(dev) => match dev.write_samples(&samples) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("gba: speaker: {}", e);
                    speaker = None;
                    false
                }
            },
            None => false,
        };
        next_frame += frame_time;
        let now = Instant::now();
        if !paced_by_audio {
            if next_frame > now {
                std::thread::sleep(next_frame - now);
            } else if now - next_frame > Duration::from_millis(250) {
                // Too slow to keep up: don't try to catch up in a burst.
                next_frame = now;
            }
        } else {
            next_frame = now;
        }

        // Save shortly after the game writes its save memory.
        if now.duration_since(last_save_check) >= Duration::from_secs(1) {
            last_save_check = now;
            if let Some(data) = emu.take_save_if_changed() {
                write_save(&save_path, &data);
            }
        }
    }

    if let Some(data) = emu.take_save_if_changed() {
        write_save(&save_path, &data);
    }
    let _ = gfx_child::set_mode(o.target, 0);
    let secs = start.elapsed().as_secs_f64();
    println!("gba: {} frames in {:.1} s ({:.1} fps).", frames, secs, frames as f64 / secs.max(0.001));
}
