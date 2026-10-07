//! The Game Boy Advance machine behind the `gba` program: a thin wrapper over
//! the vendored rustboyadvance-ng core (`rust/third_party/rustboyadvance-ng`)
//! with nothing host-specific in it, so it is tested natively with test ROMs.

use rustboyadvance_core::prelude::*;
use rustboyadvance_core::sound::interface::StereoSample;
use std::cell::RefCell;
use std::rc::Rc;

/// Screen size.
pub const WIDTH: usize = 240;
pub const HEIGHT: usize = 160;

/// The GBA's frame rate: 16.78 MHz / 280896 cycles per frame.
pub const FRAME_RATE: f64 = 16_777_216.0 / 280_896.0;

/// Audio sample rate (the GBA mixes at 32768 Hz).
pub const SAMPLE_RATE: u32 = 32_768;

/// The open-source replacement BIOS (Cult-of-GBA, MIT; see bios/).
pub const DEFAULT_BIOS: &[u8] = include_bytes!("../bios/cult-of-gba-bios.bin");

/// GBA buttons, as bits of [`Emulator::run_frame`]'s `keys` (1 = held).
pub mod keys {
    pub const A: u16 = 1 << 0;
    pub const B: u16 = 1 << 1;
    pub const SELECT: u16 = 1 << 2;
    pub const START: u16 = 1 << 3;
    pub const RIGHT: u16 = 1 << 4;
    pub const LEFT: u16 = 1 << 5;
    pub const UP: u16 = 1 << 6;
    pub const DOWN: u16 = 1 << 7;
    pub const R: u16 = 1 << 8;
    pub const L: u16 = 1 << 9;
    pub const ALL: u16 = 0x3FF;
}

/// Collects the core's stereo samples as mono.
struct MonoSink {
    samples: Rc<RefCell<Vec<i16>>>,
}

impl AudioInterface for MonoSink {
    fn get_sample_rate(&self) -> i32 {
        SAMPLE_RATE as i32
    }

    fn push_sample(&mut self, s: &StereoSample<i16>) {
        let mut v = self.samples.borrow_mut();
        // Bounded: if nobody drains for a few seconds, drop the backlog.
        if v.len() > SAMPLE_RATE as usize * 4 {
            v.clear();
        }
        v.push(((s[0] as i32 + s[1] as i32) / 2) as i16);
    }
}

/// One emulated Game Boy Advance with a cartridge in it.
pub struct Emulator {
    gba: GameBoyAdvance,
    audio: Rc<RefCell<Vec<i16>>>,
    title: String,
    code: String,
    save_kind: &'static str,
}

impl Emulator {
    /// Load `rom`, with `bios` (None = the bundled open-source BIOS) and the
    /// cartridge's saved data if any. `boot` runs the BIOS intro instead of
    /// jumping straight into the game.
    pub fn new(rom: Vec<u8>, bios: Option<Vec<u8>>, save: Option<&[u8]>, boot: bool) -> Result<Emulator, String> {
        if rom.len() < 0xC0 {
            return Err("not a GBA ROM (too small)".into());
        }
        if rom.len() > 32 * 1024 * 1024 {
            return Err("ROM is larger than 32 MiB".into());
        }
        let cart = GamepakBuilder::new()
            .take_buffer(rom.into_boxed_slice())
            .without_backup_to_file()
            .build()
            .map_err(|e| format!("bad ROM: {:?}", e))?;
        let save_kind = cart.backup_kind();
        let audio = Rc::new(RefCell::new(Vec::with_capacity(4096)));
        let bios = bios.unwrap_or_else(|| DEFAULT_BIOS.to_vec());
        let mut gba = GameBoyAdvance::new(bios.into_boxed_slice(), cart, Box::new(MonoSink { samples: audio.clone() }));
        if let Some(data) = save {
            if !data.is_empty() {
                gba.cartridge_mut().load_backup_bytes(data);
                // Loading isn't a change that needs writing back.
                gba.cartridge_mut().take_backup_dirty();
            }
        }
        if !boot {
            gba.skip_bios();
        }
        let title = gba.get_game_title().trim_end_matches('\0').trim().to_string();
        let code = gba.get_game_code().trim_end_matches('\0').to_string();
        Ok(Emulator { gba, audio, title, code, save_kind })
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    /// "sram", "flash", "eeprom" or "none".
    pub fn save_kind(&self) -> &'static str {
        self.save_kind
    }

    /// Run one frame with `held` buttons (see [`keys`]).
    ///
    /// Never inlined: on the JVM runtimes each wasm function becomes a Java
    /// method, and HotSpot doesn't JIT-compile methods over 8000 bytecodes.
    /// Inlined into a caller's loop, the emulator's hot path would end up in
    /// such a method and run interpreted (about 25% slower overall).
    #[inline(never)]
    pub fn run_frame(&mut self, held: u16) {
        // KEYINPUT is active-low.
        *self.gba.get_key_state_mut() = !held & keys::ALL;
        self.gba.key_poll();
        self.gba.frame();
    }

    /// The frame as `0x00RRGGBB` (8-bit channels, low 3 bits zero).
    pub fn framebuffer(&self) -> &[u32] {
        self.gba.get_frame_buffer()
    }

    /// The frame as little-endian RGB565, `WIDTH * HEIGHT * 2` bytes.
    #[inline(never)]
    pub fn framebuffer_rgb565(&self, out: &mut [u8]) {
        for (px, o) in self.framebuffer().iter().zip(out.chunks_exact_mut(2)) {
            let r = (px >> 16) & 0xFF;
            let g = (px >> 8) & 0xFF;
            let b = px & 0xFF;
            let v = (((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)) as u16;
            o.copy_from_slice(&v.to_le_bytes());
        }
    }

    /// Mono samples at [`SAMPLE_RATE`] produced since the last call.
    pub fn take_audio(&mut self) -> Vec<i16> {
        std::mem::take(&mut *self.audio.borrow_mut())
    }

    /// The cartridge's save data if the game changed it since the last call.
    pub fn take_save_if_changed(&mut self) -> Option<Vec<u8>> {
        if self.gba.cartridge_mut().take_backup_dirty() {
            self.gba.cartridge().backup_bytes()
        } else {
            None
        }
    }

    /// The cartridge's save data (None if it has no save chip).
    pub fn save_data(&self) -> Option<Vec<u8>> {
        self.gba.cartridge().backup_bytes()
    }

    /// A CPU register (tests read results from them).
    pub fn register(&self, r: usize) -> u32 {
        self.gba.cpu.gpr[r]
    }

    /// FNV-1a hash of the frame (regression checks).
    pub fn frame_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &p in self.framebuffer() {
            h = (h ^ p as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }
}

/// Where the saved game for `rom_path` goes: the same path with `.sav`.
pub fn save_path_for(rom_path: &str) -> String {
    match rom_path.rfind('.') {
        Some(dot) if !rom_path[dot..].contains('/') => format!("{}.sav", &rom_path[..dot]),
        _ => format!("{}.sav", rom_path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARM: &[u8] = include_bytes!("../tests/roms/arm.gba");
    const THUMB: &[u8] = include_bytes!("../tests/roms/thumb.gba");
    const MEMORY: &[u8] = include_bytes!("../tests/roms/memory.gba");
    const NES: &[u8] = include_bytes!("../tests/roms/nes.gba");
    const UNSAFE: &[u8] = include_bytes!("../tests/roms/unsafe.gba");
    const SRAM: &[u8] = include_bytes!("../tests/roms/sram.gba");

    /// The "All tests passed" screen of jsmolka's gba-tests.
    const ALL_PASSED: u64 = 0x2b81_1c8e_b3b0_32a5;

    fn run(rom: &[u8], frames: usize) -> Emulator {
        let mut emu = Emulator::new(rom.to_vec(), None, None, false).expect("load");
        for _ in 0..frames {
            emu.run_frame(0);
        }
        emu
    }

    #[test]
    fn arm_instruction_tests_pass() {
        let emu = run(ARM, 300);
        assert_eq!(emu.register(12), 0, "failed test number in r12");
        assert_eq!(emu.frame_hash(), ALL_PASSED);
    }

    #[test]
    fn thumb_instruction_tests_pass() {
        let emu = run(THUMB, 300);
        assert_eq!(emu.register(7), 0, "failed test number in r7");
        assert_eq!(emu.frame_hash(), ALL_PASSED);
    }

    #[test]
    fn memory_nes_and_unsafe_tests_pass() {
        for (name, rom) in [("memory", MEMORY), ("nes", NES), ("unsafe", UNSAFE)] {
            assert_eq!(run(rom, 300).frame_hash(), ALL_PASSED, "{}", name);
        }
    }

    #[test]
    fn rgb565_conversion_matches_the_framebuffer() {
        let emu = run(ARM, 60);
        let mut out = vec![0u8; WIDTH * HEIGHT * 2];
        emu.framebuffer_rgb565(&mut out);
        let px = emu.framebuffer()[0];
        let v = u16::from_le_bytes([out[0], out[1]]) as u32;
        assert_eq!((v >> 11) << 3, (px >> 16) & 0xF8);
        assert_eq!(v & 0x1F, (px & 0xFF) >> 3);
    }

    #[test]
    fn save_data_round_trips_through_a_new_machine() {
        let mut emu = run(SRAM, 120);
        assert_eq!(emu.save_kind(), "sram");
        let saved = emu.take_save_if_changed().expect("the test writes SRAM");
        assert!(saved.iter().any(|&b| b != 0xFF));
        assert!(emu.take_save_if_changed().is_none(), "nothing new after taking it");
        let reloaded = Emulator::new(SRAM.to_vec(), None, Some(&saved), false).unwrap();
        assert_eq!(reloaded.save_data().unwrap(), saved);
    }

    #[test]
    fn audio_runs_at_32_khz() {
        let mut emu = run(ARM, 1);
        emu.take_audio();
        for _ in 0..60 {
            emu.run_frame(0);
        }
        let n = emu.take_audio().len() as f64;
        let expected = SAMPLE_RATE as f64 * 60.0 / FRAME_RATE;
        assert!((n - expected).abs() < expected * 0.02, "{} samples for 60 frames", n);
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(Emulator::new(vec![0; 16], None, None, false).is_err());
    }

    #[test]
    fn save_path_replaces_the_extension() {
        assert_eq!(save_path_for("roms/zelda.gba"), "roms/zelda.sav");
        assert_eq!(save_path_for("game"), "game.sav");
        assert_eq!(save_path_for("dir.v2/game"), "dir.v2/game.sav");
    }
}
