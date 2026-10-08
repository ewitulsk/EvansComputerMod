//! Binary FSK at complex baseband, and AFSK (audio tones; Bell 202 1200/2200 Hz
//! at 1200 baud for APRS-style packet), including a complete AX.25 packet
//! transmitter/receiver chain (HDLC + NRZI over AFSK).

use super::BitClock;
use crate::coding::hdlc::{encode_frame, HdlcDecoder, Nrzi};
use crate::complex::C32;
use crate::fir::{bandpass, FirRR};
use crate::window::Window;
use std::f64::consts::{PI, TAU};

/// Continuous-phase binary FSK modulator at complex baseband: bit 1 -> +dev Hz,
/// bit 0 -> -dev Hz, unit amplitude.
#[derive(Clone, Debug)]
pub struct FskMod {
    sps: f64,
    w: f64,
    phase: f64,
    t: f64,
    emitted: u64,
}

impl FskMod {
    pub fn new(fs: f32, baud: f32, deviation: f32) -> FskMod {
        FskMod { sps: fs as f64 / baud as f64, w: TAU * deviation as f64 / fs as f64, phase: 0.0, t: 0.0, emitted: 0 }
    }

    pub fn modulate(&mut self, bits: &[u8], out: &mut Vec<C32>) {
        for &b in bits {
            self.t += self.sps;
            let w = if b & 1 == 1 { self.w } else { -self.w };
            while (self.emitted as f64) < self.t {
                out.push(C32::expj64(self.phase));
                self.phase = (self.phase + w + PI).rem_euclid(TAU) - PI;
                self.emitted += 1;
            }
        }
    }

    pub fn modulate_vec(&mut self, bits: &[u8]) -> Vec<C32> {
        let mut v = Vec::new();
        self.modulate(bits, &mut v);
        v
    }
}

/// Binary FSK demodulator: polar discriminator, one-bit moving average, then
/// bit-clock recovery.
#[derive(Clone, Debug)]
pub struct FskDemod {
    prev: C32,
    avg: Vec<f32>,
    pos: usize,
    sum: f32,
    clock: BitClock,
}

impl FskDemod {
    pub fn new(fs: f32, baud: f32) -> FskDemod {
        let sps = fs / baud;
        FskDemod {
            prev: C32::ZERO,
            avg: vec![0.0; (sps.round() as usize).max(1)],
            pos: 0,
            sum: 0.0,
            clock: BitClock::new(sps, 0.25),
        }
    }

    pub fn demod(&mut self, input: &[C32], out: &mut Vec<u8>) {
        for &x in input {
            let d = x.mul_conj(self.prev).arg();
            self.prev = x;
            self.sum += d - self.avg[self.pos];
            self.avg[self.pos] = d;
            self.pos = (self.pos + 1) % self.avg.len();
            if let Some((b, _)) = self.clock.push(self.sum) {
                out.push(b);
            }
        }
    }
}

/// AFSK parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AfskParams {
    pub fs: f32,
    pub baud: f32,
    pub mark: f32,
    pub space: f32,
}

impl AfskParams {
    /// Bell 202: 1200 baud, mark 1200 Hz, space 2200 Hz.
    pub fn bell202(fs: f32) -> AfskParams {
        AfskParams { fs, baud: 1200.0, mark: 1200.0, space: 2200.0 }
    }
}

/// AFSK modulator: line bits (1 = mark) -> phase-continuous audio tones.
#[derive(Clone, Debug)]
pub struct AfskMod {
    p: AfskParams,
    amp: f32,
    phase: f64,
    t: f64,
    emitted: u64,
}

impl AfskMod {
    pub fn new(p: AfskParams, amplitude: f32) -> AfskMod {
        AfskMod { p, amp: amplitude, phase: 0.0, t: 0.0, emitted: 0 }
    }

    pub fn modulate(&mut self, bits: &[u8], out: &mut Vec<f32>) {
        let sps = self.p.fs as f64 / self.p.baud as f64;
        for &b in bits {
            self.t += sps;
            let f = if b & 1 == 1 { self.p.mark } else { self.p.space };
            let w = TAU * f as f64 / self.p.fs as f64;
            while (self.emitted as f64) < self.t {
                out.push(self.amp * self.phase.sin() as f32);
                self.phase = (self.phase + w) % TAU;
                self.emitted += 1;
            }
        }
    }
}

/// AFSK demodulator: band-pass, mark/space quadrature correlators over one
/// bit, normalised difference, bit-clock recovery. Outputs line bits.
#[derive(Clone, Debug)]
pub struct AfskDemod {
    bpf: FirRR,
    wm: f64,
    ws: f64,
    pm: f64,
    ps: f64,
    hist_m: Vec<C32>,
    hist_s: Vec<C32>,
    pos: usize,
    sum_m: C32,
    sum_s: C32,
    clock: BitClock,
}

impl AfskDemod {
    pub fn new(p: AfskParams) -> AfskDemod {
        let sps = p.fs / p.baud;
        let lo = (p.mark.min(p.space) - p.baud * 0.5).max(50.0) / p.fs;
        let hi = (p.mark.max(p.space) + p.baud * 0.5).min(p.fs * 0.45) / p.fs;
        let taps = ((sps * 4.0) as usize) | 1;
        let n = (sps.round() as usize).max(1);
        AfskDemod {
            bpf: FirRR::new(bandpass(taps.max(15), lo, hi, Window::Hamming)),
            wm: TAU * p.mark as f64 / p.fs as f64,
            ws: TAU * p.space as f64 / p.fs as f64,
            pm: 0.0,
            ps: 0.0,
            hist_m: vec![C32::ZERO; n],
            hist_s: vec![C32::ZERO; n],
            pos: 0,
            sum_m: C32::ZERO,
            sum_s: C32::ZERO,
            clock: BitClock::new(sps, 0.3),
        }
    }

    /// Soft value per sample in `[-1, 1]` (`> 0` = mark).
    #[inline]
    pub fn soft_sample(&mut self, x: f32) -> f32 {
        let y = self.bpf.filter(x);
        let m = C32::expj64(-self.pm).scale(y);
        let s = C32::expj64(-self.ps).scale(y);
        self.pm = (self.pm + self.wm) % TAU;
        self.ps = (self.ps + self.ws) % TAU;
        self.sum_m += m - self.hist_m[self.pos];
        self.sum_s += s - self.hist_s[self.pos];
        self.hist_m[self.pos] = m;
        self.hist_s[self.pos] = s;
        self.pos = (self.pos + 1) % self.hist_m.len();
        let (am, as_) = (self.sum_m.abs(), self.sum_s.abs());
        (am - as_) / (am + as_ + 1e-12)
    }

    /// Demodulate audio, appending line bits (1 = mark) to `out`.
    pub fn demod(&mut self, audio: &[f32], out: &mut Vec<u8>) {
        for &x in audio {
            let s = self.soft_sample(x);
            if let Some((b, _)) = self.clock.push(s) {
                out.push(b);
            }
        }
    }
}

/// AX.25-over-AFSK transmitter: frame body -> HDLC bits -> NRZI -> audio.
#[derive(Clone, Debug)]
pub struct PacketTx {
    afsk: AfskMod,
    nrzi: Nrzi,
    /// Number of leading flags (TX delay); 1200 baud * 8 bits ~ 6.7 ms per flag.
    pub preamble_flags: usize,
    pub postamble_flags: usize,
}

impl PacketTx {
    pub fn new(p: AfskParams, amplitude: f32) -> PacketTx {
        PacketTx { afsk: AfskMod::new(p, amplitude), nrzi: Nrzi::new(), preamble_flags: 32, postamble_flags: 4 }
    }

    /// Append the audio for one frame (FCS added here) to `out`.
    pub fn send(&mut self, frame: &[u8], out: &mut Vec<f32>) {
        let bits = encode_frame(frame, self.preamble_flags, self.postamble_flags);
        let line = self.nrzi.encode_block(&bits);
        self.afsk.modulate(&line, out);
    }
}

/// AX.25-over-AFSK receiver: audio -> frames with a valid FCS.
#[derive(Clone, Debug)]
pub struct PacketRx {
    demod: AfskDemod,
    nrzi: Nrzi,
    hdlc: HdlcDecoder,
    bits: Vec<u8>,
}

impl PacketRx {
    pub fn new(p: AfskParams) -> PacketRx {
        PacketRx { demod: AfskDemod::new(p), nrzi: Nrzi::new(), hdlc: HdlcDecoder::new(15, 330), bits: Vec::new() }
    }

    /// Feed audio; returns decoded frame bodies (FCS stripped).
    pub fn push(&mut self, audio: &[f32]) -> Vec<Vec<u8>> {
        self.bits.clear();
        self.demod.demod(audio, &mut self.bits);
        let mut frames = Vec::new();
        for &l in &self.bits {
            let b = self.nrzi.decode(l);
            if let Some(f) = self.hdlc.push_bit(b) {
                frames.push(f);
            }
        }
        frames
    }

    /// Frames rejected with a bad FCS so far.
    pub fn bad_fcs(&self) -> u64 {
        self.hdlc.bad_fcs
    }
}
