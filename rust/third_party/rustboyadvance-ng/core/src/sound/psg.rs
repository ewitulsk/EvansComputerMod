//! PATCH: the four Game Boy sound channels ("PSG") of the GBA: two square
//! waves (the first with a frequency sweep), a 4-bit wave channel with two
//! banks of wave RAM, and a noise channel. Upstream only mixed the DMA
//! (DirectSound) channels, so games lost every PSG instrument and effect.
//!
//! Timings follow GBATEK, converted to GBA cycles (16.78 MHz, 4x the Game
//! Boy's clock). Channels are advanced once per output sample; each yields a
//! signed level of -15..15 (0 when off), mixed by the sound controller.

use bit::BitIndex;
use serde::{Deserialize, Serialize};

/// Square duty patterns (12.5%, 25%, 50%, 75%), one step per 1/8 period.
const DUTY: [[bool; 8]; 4] = [
    [false, false, false, false, false, false, false, true],
    [true, false, false, false, false, false, false, true],
    [true, false, false, false, false, true, true, true],
    [false, true, true, true, true, true, true, false],
];

/// The frame sequencer runs at 512 Hz.
const FRAME_SEQUENCER_CYCLES: u32 = 16_777_216 / 512;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct Envelope {
    initial: u8,
    increase: bool,
    step: u8,
    volume: u8,
    timer: u8,
}

impl Envelope {
    /// Bits 8-15 of the channel's envelope register.
    fn write(&mut self, v: u16) {
        self.step = v.bit_range(8..11) as u8;
        self.increase = v.bit(11);
        self.initial = v.bit_range(12..16) as u8;
    }

    fn read(&self) -> u16 {
        (self.step as u16) << 8 | (self.increase as u16) << 11 | (self.initial as u16) << 12
    }

    /// The DAC is off when the envelope can only stay at 0.
    fn dac_on(&self) -> bool {
        self.initial != 0 || self.increase
    }

    fn trigger(&mut self) {
        self.volume = self.initial;
        self.timer = self.step;
    }

    /// 64 Hz.
    fn clock(&mut self) {
        if self.step == 0 {
            return;
        }
        if self.timer > 1 {
            self.timer -= 1;
            return;
        }
        self.timer = self.step;
        if self.increase && self.volume < 15 {
            self.volume += 1;
        } else if !self.increase && self.volume > 0 {
            self.volume -= 1;
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct Length {
    /// Stop the channel when the counter runs out ("timed" mode).
    enabled: bool,
    counter: u16,
}

impl Length {
    /// 256 Hz. Returns true when the channel must stop.
    fn clock(&mut self) -> bool {
        if self.enabled && self.counter > 0 {
            self.counter -= 1;
            return self.counter == 0;
        }
        false
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct Square {
    on: bool,
    duty: u8,
    env: Envelope,
    length: Length,
    freq: u16,
    timer: i32,
    step: u8,
    // Sweep (channel 1 only).
    sweep_shift: u8,
    sweep_decrease: bool,
    sweep_time: u8,
    sweep_timer: u8,
    sweep_enabled: bool,
    shadow_freq: u16,
}

impl Square {
    fn period(&self) -> i32 {
        (2048 - self.freq as i32) * 16
    }

    fn advance(&mut self, cycles: i32) {
        if !self.on {
            return;
        }
        self.timer -= cycles;
        if self.timer <= 0 {
            let p = self.period();
            let n = (-self.timer) / p + 1;
            self.timer += n * p;
            self.step = ((self.step as i32 + n) & 7) as u8;
        }
    }

    fn output(&self) -> i32 {
        if !self.on {
            return 0;
        }
        let v = self.env.volume as i32;
        if DUTY[self.duty as usize][self.step as usize] { v } else { -v }
    }

    fn trigger(&mut self) {
        self.on = self.env.dac_on();
        if self.length.counter == 0 {
            self.length.counter = 64;
        }
        self.timer = self.period();
        self.env.trigger();
        self.shadow_freq = self.freq;
        self.sweep_timer = if self.sweep_time == 0 { 8 } else { self.sweep_time };
        self.sweep_enabled = self.sweep_time != 0 || self.sweep_shift != 0;
        if self.sweep_shift != 0 {
            self.sweep_calc();
        }
    }

    /// The next swept frequency; stops the channel on overflow.
    fn sweep_calc(&mut self) -> u16 {
        let delta = self.shadow_freq >> self.sweep_shift;
        let f = if self.sweep_decrease {
            self.shadow_freq.saturating_sub(delta)
        } else {
            self.shadow_freq + delta
        };
        if f > 2047 {
            self.on = false;
        }
        f
    }

    /// 128 Hz.
    fn clock_sweep(&mut self) {
        if self.sweep_timer > 1 {
            self.sweep_timer -= 1;
            return;
        }
        self.sweep_timer = if self.sweep_time == 0 { 8 } else { self.sweep_time };
        if self.sweep_enabled && self.sweep_time != 0 {
            let f = self.sweep_calc();
            if f <= 2047 && self.sweep_shift != 0 {
                self.freq = f;
                self.shadow_freq = f;
                self.sweep_calc();
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct Wave {
    on: bool,
    dac_on: bool,
    /// 64 samples (both banks) instead of 32.
    two_banks: bool,
    /// The bank being played; the CPU sees the other one.
    bank: u8,
    volume_code: u8,
    force_75: bool,
    length: Length,
    freq: u16,
    timer: i32,
    position: u8,
    ram: [u8; 32],
}

impl Wave {
    fn period(&self) -> i32 {
        (2048 - self.freq as i32) * 8
    }

    fn samples(&self) -> u8 {
        if self.two_banks { 64 } else { 32 }
    }

    fn advance(&mut self, cycles: i32) {
        if !self.on {
            return;
        }
        self.timer -= cycles;
        if self.timer <= 0 {
            let p = self.period();
            let n = (-self.timer) / p + 1;
            self.timer += n * p;
            self.position = ((self.position as i32 + n) % self.samples() as i32) as u8;
        }
    }

    fn sample(&self) -> u8 {
        // Bytes hold two samples, high nibble first; playback starts in the selected bank.
        let index = (self.bank as usize * 32 + self.position as usize) % 64;
        let byte = self.ram[index / 2];
        if index % 2 == 0 { byte >> 4 } else { byte & 0x0F }
    }

    fn output(&self) -> i32 {
        if !self.on || !self.dac_on {
            return 0;
        }
        let s = self.sample() as i32 * 2 - 15;
        if self.force_75 {
            return s * 3 / 4;
        }
        match self.volume_code {
            0 => 0,
            1 => s,
            2 => s / 2,
            _ => s / 4,
        }
    }

    fn trigger(&mut self) {
        self.on = self.dac_on;
        if self.length.counter == 0 {
            self.length.counter = 256;
        }
        self.timer = self.period();
        self.position = 0;
    }

    /// The bank the CPU sees (the one not being played).
    fn cpu_bank(&self) -> usize {
        (self.bank as usize ^ 1) * 16
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct Noise {
    on: bool,
    env: Envelope,
    length: Length,
    divisor: u8,
    width7: bool,
    shift: u8,
    lfsr: u16,
    timer: i32,
}

impl Noise {
    fn period(&self) -> i32 {
        let base = if self.divisor == 0 { 32 } else { 64 * self.divisor as i32 };
        base << self.shift
    }

    fn advance(&mut self, cycles: i32) {
        if !self.on || self.shift >= 14 {
            return;
        }
        self.timer -= cycles;
        while self.timer <= 0 {
            self.timer += self.period();
            let bit = (self.lfsr ^ (self.lfsr >> 1)) & 1;
            self.lfsr = (self.lfsr >> 1) | (bit << 14);
            if self.width7 {
                self.lfsr = (self.lfsr & !(1 << 6)) | (bit << 6);
            }
        }
    }

    fn output(&self) -> i32 {
        if !self.on {
            return 0;
        }
        let v = self.env.volume as i32;
        if self.lfsr & 1 == 0 { v } else { -v }
    }

    fn trigger(&mut self) {
        self.on = self.env.dac_on();
        if self.length.counter == 0 {
            self.length.counter = 64;
        }
        self.timer = self.period();
        self.lfsr = 0x7FFF;
        self.env.trigger();
    }
}

/// The four PSG channels and their frame sequencer.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Psg {
    sq1: Square,
    sq2: Square,
    wave: Wave,
    noise: Noise,
    sequencer_timer: u32,
    sequencer_step: u8,
}

impl Psg {
    pub fn new() -> Psg {
        Psg { noise: Noise { lfsr: 0x7FFF, ..Default::default() }, ..Default::default() }
    }

    /// Channel outputs (-15..15 each): square 1, square 2, wave, noise.
    pub fn outputs(&self) -> [i32; 4] {
        [self.sq1.output(), self.sq2.output(), self.wave.output(), self.noise.output()]
    }

    /// SOUNDCNT_X bits 0-3: which channels are playing.
    pub fn status(&self) -> u16 {
        (self.sq1.on as u16) | (self.sq2.on as u16) << 1 | (self.wave.on as u16) << 2 | (self.noise.on as u16) << 3
    }

    /// Advance by `cycles` (one output sample).
    pub fn advance(&mut self, cycles: u32) {
        let c = cycles as i32;
        self.sq1.advance(c);
        self.sq2.advance(c);
        self.wave.advance(c);
        self.noise.advance(c);
        self.sequencer_timer += cycles;
        while self.sequencer_timer >= FRAME_SEQUENCER_CYCLES {
            self.sequencer_timer -= FRAME_SEQUENCER_CYCLES;
            self.clock_sequencer();
        }
    }

    fn clock_sequencer(&mut self) {
        let step = self.sequencer_step;
        self.sequencer_step = (step + 1) & 7;
        if step % 2 == 0 {
            if self.sq1.length.clock() {
                self.sq1.on = false;
            }
            if self.sq2.length.clock() {
                self.sq2.on = false;
            }
            if self.wave.length.clock() {
                self.wave.on = false;
            }
            if self.noise.length.clock() {
                self.noise.on = false;
            }
        }
        if step == 2 || step == 6 {
            self.sq1.clock_sweep();
        }
        if step == 7 {
            self.sq1.env.clock();
            self.sq2.env.clock();
            self.noise.env.clock();
        }
    }

    /// Master enable cleared: every PSG register is reset (wave RAM kept).
    pub fn reset(&mut self) {
        let ram = self.wave.ram;
        *self = Psg::new();
        self.wave.ram = ram;
    }

    /// Whether `io_addr` is a PSG register this module handles.
    pub fn handles(io_addr: u32) -> bool {
        matches!(io_addr, 0x0400_0060..=0x0400_007F | 0x0400_0090..=0x0400_009F)
    }

    pub fn read(&self, io_addr: u32) -> u16 {
        match io_addr {
            0x0400_0060 => {
                self.sq1.sweep_shift as u16 | (self.sq1.sweep_decrease as u16) << 3 | (self.sq1.sweep_time as u16) << 4
            }
            0x0400_0062 => (self.sq1.duty as u16) << 6 | self.sq1.env.read(),
            0x0400_0064 => (self.sq1.length.enabled as u16) << 14,
            0x0400_0068 => (self.sq2.duty as u16) << 6 | self.sq2.env.read(),
            0x0400_006C => (self.sq2.length.enabled as u16) << 14,
            0x0400_0070 => (self.wave.two_banks as u16) << 5 | (self.wave.bank as u16) << 6 | (self.wave.dac_on as u16) << 7,
            0x0400_0072 => (self.wave.volume_code as u16) << 13 | (self.wave.force_75 as u16) << 15,
            0x0400_0074 => (self.wave.length.enabled as u16) << 14,
            0x0400_0078 => self.noise.env.read(),
            0x0400_007C => {
                self.noise.divisor as u16
                    | (self.noise.width7 as u16) << 3
                    | (self.noise.shift as u16) << 4
                    | (self.noise.length.enabled as u16) << 14
            }
            0x0400_0090..=0x0400_009F => {
                let i = self.wave.cpu_bank() + (io_addr as usize - 0x0400_0090);
                self.wave.ram[i] as u16 | (self.wave.ram[(i + 1) % 32] as u16) << 8
            }
            _ => 0,
        }
    }

    pub fn write(&mut self, io_addr: u32, v: u16) {
        match io_addr {
            0x0400_0060 => {
                self.sq1.sweep_shift = v.bit_range(0..3) as u8;
                self.sq1.sweep_decrease = v.bit(3);
                self.sq1.sweep_time = v.bit_range(4..7) as u8;
            }
            0x0400_0062 => write_duty_length_env(&mut self.sq1, v),
            0x0400_0064 => write_square_control(&mut self.sq1, v),
            0x0400_0068 => write_duty_length_env(&mut self.sq2, v),
            0x0400_006C => write_square_control(&mut self.sq2, v),
            0x0400_0070 => {
                self.wave.two_banks = v.bit(5);
                self.wave.bank = v.bit(6) as u8;
                self.wave.dac_on = v.bit(7);
                if !self.wave.dac_on {
                    self.wave.on = false;
                }
            }
            0x0400_0072 => {
                self.wave.length.counter = 256 - v.bit_range(0..8);
                self.wave.volume_code = v.bit_range(13..15) as u8;
                self.wave.force_75 = v.bit(15);
            }
            0x0400_0074 => {
                self.wave.freq = v.bit_range(0..11);
                self.wave.length.enabled = v.bit(14);
                if v.bit(15) {
                    self.wave.trigger();
                }
            }
            0x0400_0078 => {
                self.noise.length.counter = 64 - v.bit_range(0..6);
                self.noise.env.write(v);
                if !self.noise.env.dac_on() {
                    self.noise.on = false;
                }
            }
            0x0400_007C => {
                self.noise.divisor = v.bit_range(0..3) as u8;
                self.noise.width7 = v.bit(3);
                self.noise.shift = v.bit_range(4..8) as u8;
                self.noise.length.enabled = v.bit(14);
                if v.bit(15) {
                    self.noise.trigger();
                }
            }
            0x0400_0090..=0x0400_009F => {
                let i = self.wave.cpu_bank() + (io_addr as usize - 0x0400_0090);
                self.wave.ram[i] = v as u8;
                self.wave.ram[(i + 1) % 32] = (v >> 8) as u8;
            }
            _ => {}
        }
    }
}

fn write_duty_length_env(sq: &mut Square, v: u16) {
    sq.length.counter = 64 - v.bit_range(0..6);
    sq.duty = v.bit_range(6..8) as u8;
    sq.env.write(v);
    if !sq.env.dac_on() {
        sq.on = false;
    }
}

fn write_square_control(sq: &mut Square, v: u16) {
    sq.freq = v.bit_range(0..11);
    sq.length.enabled = v.bit(14);
    if v.bit(15) {
        sq.trigger();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(psg: &mut Psg, samples: usize) -> Vec<[i32; 4]> {
        (0..samples)
            .map(|_| {
                psg.advance(512);
                psg.outputs()
            })
            .collect()
    }

    #[test]
    fn square_plays_at_its_frequency() {
        let mut psg = Psg::new();
        psg.write(0x0400_0062, 2 << 6 | 15 << 12); // 50% duty, volume 15
        // freq 1750: 131072 / (2048 - 1750) = 439.8 Hz
        psg.write(0x0400_0064, 1750 | 1 << 15);
        let out = run(&mut psg, 32768);
        let edges = out.windows(2).filter(|w| w[0][0] < 0 && w[1][0] > 0).count();
        assert!((edges as i32 - 440).abs() <= 2, "{} rising edges in 1 s", edges);
        assert!(out.iter().all(|o| o[0].abs() == 15));
    }

    #[test]
    fn length_stops_a_timed_channel() {
        let mut psg = Psg::new();
        psg.write(0x0400_0068, 15 << 12 | 0); // length 64 -> 64/256 s
        psg.write(0x0400_006C, 1000 | 1 << 14 | 1 << 15);
        assert_eq!(psg.status() & 2, 2);
        run(&mut psg, 32768 / 4 + 600);
        assert_eq!(psg.status() & 2, 0, "stopped after 1/4 s");
    }

    #[test]
    fn envelope_fades_out() {
        let mut psg = Psg::new();
        psg.write(0x0400_0078, 15 << 12 | 1 << 8); // vol 15, decrease every 1/64 s
        psg.write(0x0400_007C, 1 << 15);
        let out = run(&mut psg, 32768 / 2);
        assert_eq!(out.last().unwrap()[3], 0, "faded to silence in 15/64 s");
    }

    #[test]
    fn wave_ram_is_read_from_the_bank_not_playing() {
        let mut psg = Psg::new();
        psg.write(0x0400_0070, 0); // play bank 0, CPU sees bank 1
        psg.write(0x0400_0090, 0x1234);
        assert_eq!(psg.read(0x0400_0090), 0x1234);
        psg.write(0x0400_0070, 1 << 6); // now play bank 1, CPU sees bank 0
        assert_eq!(psg.read(0x0400_0090), 0);
    }

    #[test]
    fn wave_channel_outputs_its_samples() {
        let mut psg = Psg::new();
        psg.write(0x0400_0070, 1 << 6); // CPU writes bank 0, plays bank 1...
        for a in (0..16).step_by(2) {
            psg.write(0x0400_0090 + a, 0xFFFF);
        }
        psg.write(0x0400_0070, 1 << 7); // ...then play bank 0, DAC on
        psg.write(0x0400_0072, 1 << 13); // 100%
        psg.write(0x0400_0074, 2000 | 1 << 15);
        let out = run(&mut psg, 100);
        assert!(out.iter().all(|o| o[2] == 15));
    }

    #[test]
    fn sweep_overflow_silences_channel_1() {
        let mut psg = Psg::new();
        psg.write(0x0400_0060, 1 | 1 << 4); // shift 1, increase, every 1/128 s
        psg.write(0x0400_0062, 15 << 12);
        psg.write(0x0400_0064, 1900 | 1 << 15);
        run(&mut psg, 32768);
        assert_eq!(psg.status() & 1, 0);
    }
}
