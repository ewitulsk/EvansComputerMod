# rustboyadvance-ng (vendored)

The Game Boy Advance core of [rustboyadvance-ng](https://github.com/michelhe/rustboyadvance-ng)
by Michel Heily, MIT-licensed (see `LICENSE`), at upstream commit
`1d7ff23a6adb42d803ebd393c3d27d94e4fea15e` (2026-08-27). Only the `core`,
`arm7tdmi` and `utils` crates are included; the front-ends are not. Used by the
`gba` program (`rust/wasm-programs/gba`).

## Local changes

1. **Undefined behaviour fix (required on wasm32 and with LTO).** Upstream read
   the CPU's PC through a raw pointer back into the CPU while the CPU held
   `&mut self` (BIOS read protection in `bios.rs`, open-bus reads in
   `sysbus.rs`). The compiler relies on `noalias` there: with LTO natively the
   emulator panicked at the first BIOS call, and on wasm32-wasip1 games ran
   with a blank screen. `MemoryInterface` gains `fetch_hint(pc)`, which the CPU
   calls before every fetch; the bus keeps the PC and the fetched words itself.
2. **Save data survives savestates.** `BackupFile` serialized only its size and
   path, and `Clone` made an empty buffer, so a save state dropped SRAM / Flash /
   EEPROM contents when no backing file was used. The buffer is now serialized
   and cloned, with a dirty flag.
3. **Save-data access.** `GameBoyAdvance::cartridge[_mut]()`,
   `Cartridge::backup_bytes()`, `load_backup_bytes()`, `take_backup_dirty()` and
   `backup_kind()`, so the host program owns the `.sav` file.
4. **Per-instance frame overshoot.** `GameBoyAdvance::frame()` kept the cycles a
   frame ran over in a function-local `static mut`, shared and raced on by every
   emulator in the process; it is a field now.
5. **Keypad interrupt.** KEYCNT wasn't stored and `key_poll()` was an empty
   debugger-only stub; it now raises the keypad IRQ when KEYCNT asks for it.
6. **Build trimming.** Removed `clap` (only used for an attribute), the
   criterion benchmark and its ROMs, and the arm7tdmi examples.
7. **Direct-call instruction dispatch.** Upstream dispatched decoded ARM/THUMB
   instructions through a table of function pointers. Compiled to wasm that is
   one `call_indirect` site whose JVM translation (Chicory) is too large for
   HotSpot to JIT. `arm7tdmi/build.rs` now also writes `dispatch.rs`: handler
   ids per opcode and a two-level `match` of direct calls, used by
   `step_arm_exec` / `step_thumb_exec`. Same instructions; about 1.2x
   faster under Chicory.
8. **Game Boy sound channels.** Upstream only had the two DMA (Direct Sound)
   channels; the four PSG channels (two squares, wave, noise) are implemented
   in `core/src/sound/psg.rs` with frame sequencer, length, envelope and sweep.
   Also fixed: SOUNDCNT_L master volumes are 3 bits, the SOUNDCNT_H PSG ratio is
   2 bits, SOUNDCNT_X reports channel status, and clearing master enable resets
   the PSG registers.
9. **64-bit cycle timestamps.** The scheduler and timers kept absolute cycle
   counts in `usize`, which is 32 bits on wasm32: after 2^32 cycles (4 min
   16 s of play, frame 15290) the count wrapped, no event (vblank, timers)
   ever came due again and the game hung in HALT. They are `u64`
   (`sched::Timestamp`) now, with a compile-time check of the width.
10. **Tests.** The core's ROM tests needed the upstream `external/gba-tests`
   submodule; they were removed. The same ROMs are run by `rust/wasm-programs/gba`
   (`cargo test --release -p gba`).
