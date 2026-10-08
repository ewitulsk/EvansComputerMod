//! LoRa-style chirp spread spectrum (CSS).
//!
//! A symbol of spreading factor `SF` is one of `N = 2^SF` cyclic shifts of a
//! linear up-chirp sweeping `-BW/2 .. +BW/2` in `N` chips. The receiver
//! multiplies by the conjugate base chirp ("dechirp"), which turns the symbol
//! into a tone, and picks the strongest FFT bin. The FFT's processing gain
//! (`10 log10 N`, 21 dB at SF7, 36 dB at SF12) is why CSS decodes far below
//! the noise floor.
//!
//! Frame: `preamble` up-chirps (symbol 0), 2 down-chirps (sync), then the
//! payload symbols. Payload bytes are framed as `[len, data.., crc16]`
//! (CRC-16/CCITT-FALSE, big-endian), packed MSB-first into `SF`-bit symbols
//! with Gray mapping (adjacent bins differ by one bit).
//!
//! The sample rate is `os * BW` with an integer oversampling factor `os`.

use crate::coding::crc::crc16_ccitt_false;
use crate::complex::C32;
use crate::fft::Fft;
use crate::fir::{lowpass_kaiser, FirRC};
use std::f64::consts::{PI, TAU};
use std::sync::Arc;

/// CSS parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChirpParams {
    /// Spreading factor, 7..=12.
    pub sf: u8,
    /// Chirp bandwidth (Hz).
    pub bw: f32,
    /// Oversampling: sample rate = `os * bw`.
    pub os: usize,
    /// Number of preamble up-chirps (>= 6).
    pub preamble: usize,
}

impl ChirpParams {
    pub fn new(sf: u8, bw: f32, os: usize) -> ChirpParams {
        assert!((6..=12).contains(&sf), "SF must be 6..=12");
        assert!(os >= 1);
        ChirpParams { sf, bw, os, preamble: 8 }
    }

    /// Chips per symbol (`2^SF`).
    pub fn chips(&self) -> usize {
        1 << self.sf
    }

    /// Samples per symbol.
    pub fn symbol_len(&self) -> usize {
        self.chips() * self.os
    }

    /// Sample rate (Hz).
    pub fn fs(&self) -> f32 {
        self.bw * self.os as f32
    }

    /// Raw bit rate (bits/s) before framing.
    pub fn bit_rate(&self) -> f32 {
        self.sf as f32 * self.bw / self.chips() as f32
    }

    fn header_symbols(&self) -> usize {
        8usize.div_ceil(self.sf as usize)
    }

    fn frame_symbols(&self, len: usize) -> usize {
        ((len + 3) * 8).div_ceil(self.sf as usize)
    }
}

fn gray_inverse(mut g: u16) -> u16 {
    let mut b = g;
    while g > 0 {
        g >>= 1;
        b ^= g;
    }
    b
}

#[inline]
fn gray(b: u16) -> u16 {
    b ^ (b >> 1)
}

/// Phase-continuous chirp generator.
#[derive(Clone, Debug)]
pub struct ChirpMod {
    p: ChirpParams,
    phase: f64,
}

impl ChirpMod {
    pub fn new(p: ChirpParams) -> ChirpMod {
        ChirpMod { p, phase: 0.0 }
    }

    /// Append one symbol: an up-chirp shifted by `sym` chips, or (if `down`) the base down-chirp.
    pub fn push_symbol(&mut self, sym: u16, down: bool, out: &mut Vec<C32>) {
        // Exact phase of the continuous-time chirp (integral of the
        // instantaneous frequency), so the waveform is identical at any
        // oversampling factor. x is time in chips; phase in cycles.
        let n = self.p.chips() as f64;
        let s = (sym as f64) % n;
        let cycles = |x: f64| -> f64 {
            if down {
                -(x * x / (2.0 * n) - x / 2.0)
            } else if x <= n - s {
                ((x + s) * (x + s) - s * s) / (2.0 * n) - x / 2.0
            } else {
                let w = n - s;
                let at_w = (n * n - s * s) / (2.0 * n) - w / 2.0;
                let y = x - w;
                at_w + y * y / (2.0 * n) - y / 2.0
            }
        };
        let os = self.p.os as f64;
        for m in 0..self.p.symbol_len() {
            let c = cycles(m as f64 / os);
            out.push(C32::expj64(self.phase + TAU * (c - c.floor())));
        }
        let end = cycles(n);
        self.phase = (self.phase + TAU * (end - end.floor()) + PI).rem_euclid(TAU) - PI;
    }

    /// Modulate raw symbols with preamble and sync (no length/CRC framing).
    pub fn modulate_symbols(&mut self, symbols: &[u16], out: &mut Vec<C32>) {
        for _ in 0..self.p.preamble {
            self.push_symbol(0, false, out);
        }
        self.push_symbol(0, true, out);
        self.push_symbol(0, true, out);
        for &s in symbols {
            self.push_symbol(s % self.p.chips() as u16, false, out);
        }
    }

    /// Frame and modulate a packet of up to 255 bytes.
    pub fn modulate_packet(&mut self, data: &[u8], out: &mut Vec<C32>) {
        let syms = packet_to_symbols(&self.p, data);
        self.modulate_symbols(&syms, out);
    }
}

/// Packet bytes -> Gray-mapped symbols (`[len, data.., crc16]`).
pub fn packet_to_symbols(p: &ChirpParams, data: &[u8]) -> Vec<u16> {
    assert!(data.len() <= 255, "CSS packets carry at most 255 bytes");
    let mut frame = Vec::with_capacity(data.len() + 3);
    frame.push(data.len() as u8);
    frame.extend_from_slice(data);
    frame.extend_from_slice(&crc16_ccitt_false(data).to_be_bytes());
    let bits = crate::coding::bytes_to_bits_msb(&frame);
    let sf = p.sf as usize;
    bits.chunks(sf)
        .map(|c| {
            let v = c.iter().enumerate().fold(0u16, |a, (i, &b)| a | ((b as u16) << (sf - 1 - i)));
            gray_inverse(v)
        })
        .collect()
}

fn symbols_to_bits(p: &ChirpParams, bins: &[u16]) -> Vec<u8> {
    let sf = p.sf as usize;
    let mut bits = Vec::with_capacity(bins.len() * sf);
    for &b in bins {
        let v = gray(b);
        for i in 0..sf {
            bits.push(((v >> (sf - 1 - i)) & 1) as u8);
        }
    }
    bits
}

/// Dechirp + FFT symbol detector.
#[derive(Clone, Debug)]
pub struct ChirpDemod {
    p: ChirpParams,
    up: Vec<C32>,
    fft: Arc<Fft>,
    buf: Vec<C32>,
}

/// Result of analysing one symbol window.
#[derive(Clone, Copy, Debug)]
pub struct Peak {
    pub bin: u16,
    /// Fractional bin offset of the true peak, in `[-0.5, 0.5]`.
    pub frac: f32,
    /// Peak power over the mean bin power.
    pub ratio: f32,
    pub power: f32,
}

impl ChirpDemod {
    pub fn new(p: ChirpParams) -> ChirpDemod {
        let mut up = Vec::with_capacity(p.symbol_len());
        ChirpMod::new(p).push_symbol(0, false, &mut up);
        ChirpDemod { p, up, fft: Arc::new(Fft::new(p.symbol_len())), buf: vec![C32::ZERO; p.symbol_len()] }
    }

    /// Analyse one window. `down = false` detects up-chirps (symbols), `true`
    /// detects the base down-chirp (peak at bin 0 when aligned).
    pub fn analyse(&mut self, w: &[C32], down: bool) -> Peak {
        self.analyse_shifted(w, down, 0.0)
    }

    /// Like [`ChirpDemod::analyse`], but first removes a frequency offset of
    /// `shift` bins (fractional timing and carrier offsets both appear as a
    /// bin offset after dechirping).
    pub fn analyse_shifted(&mut self, w: &[C32], down: bool, shift: f32) -> Peak {
        let l = self.p.symbol_len();
        let n = self.p.chips();
        let step = -TAU * shift as f64 / l as f64;
        for i in 0..l {
            let v = if down { w[i] * self.up[i] } else { w[i].mul_conj(self.up[i]) };
            self.buf[i] = if shift != 0.0 { v * C32::expj64(step * i as f64) } else { v };
        }
        self.fft.forward(&mut self.buf);
        let (mut best, mut bp, mut total) = (0usize, 0f32, 0f64);
        for k in 0..n {
            let mut pk = self.buf[k].norm_sqr();
            if self.p.os > 1 {
                pk += self.buf[l - n + k].norm_sqr();
            }
            total += pk as f64;
            if pk > bp {
                bp = pk;
                best = k;
            }
        }
        let mean = (total / n as f64) as f32;
        // Parabolic interpolation on magnitude for a fractional bin offset.
        let mag = |k: usize| {
            let mut pk = self.buf[k].norm_sqr();
            if self.p.os > 1 {
                pk += self.buf[l - n + k].norm_sqr();
            }
            pk.sqrt()
        };
        let (a, b, c) = (mag((best + n - 1) % n), mag(best), mag((best + 1) % n));
        let den = a - 2.0 * b + c;
        let frac = if den.abs() > 1e-20 { (0.5 * (a - c) / den).clamp(-0.5, 0.5) } else { 0.0 };
        Peak { bin: best as u16, frac, ratio: bp / mean.max(1e-30), power: bp }
    }

    /// Demodulate aligned symbol windows to bins (no framing).
    pub fn demod_symbols(&mut self, x: &[C32]) -> Vec<u16> {
        let l = self.p.symbol_len();
        x.chunks_exact(l).map(|w| self.analyse(w, false).bin).collect()
    }
}

#[derive(Clone, Debug)]
enum State {
    Search { pos: usize, run: usize, last: u16 },
    Sync { b0: usize, j: usize },
    Payload { start: f64, offset: f64 },
}

const INTERP_HALF: i64 = 6;

/// Streaming CSS packet receiver: preamble detection, timing and carrier
/// offset estimation from the up/down-chirp peaks, payload decode and CRC
/// check. Carrier offsets up to about +-N/4 bins are tolerated.
///
/// Oversampled input (`os > 1`) is low-passed to the chirp bandwidth and each
/// payload symbol is resampled at the exact (fractional) chip instants with a
/// windowed-sinc interpolator, so arbitrary timing offsets decode cleanly.
/// With `os = 1` timing is resolved to whole samples and the remaining
/// fraction is removed as a frequency offset (good to about a quarter chip).
#[derive(Clone, Debug)]
pub struct ChirpRx {
    p: ChirpParams,
    lpf: Option<FirRC>,
    demod: ChirpDemod,
    buf: Vec<C32>,
    win: Vec<C32>,
    state: State,
    thresh: f32,
    min_run: usize,
    /// Packets that failed the CRC (diagnostics).
    pub crc_errors: u64,
}

impl ChirpRx {
    pub fn new(p: ChirpParams) -> ChirpRx {
        let n = p.chips() as f32;
        let lpf = (p.os > 1).then(|| {
            let os = p.os as f32;
            FirRC::new(lowpass_kaiser(0.55 / os, 0.2 / os, 50.0))
        });
        let chip_rate = ChirpParams { os: 1, ..p };
        ChirpRx {
            p,
            lpf,
            demod: ChirpDemod::new(chip_rate),
            buf: Vec::new(),
            win: vec![C32::ZERO; p.chips()],
            state: State::Search { pos: 0, run: 0, last: 0 },
            thresh: 2.0 * n.ln() + 4.0,
            min_run: (p.preamble.saturating_sub(4)).max(3),
            crc_errors: 0,
        }
    }

    fn signed_bin(&self, b: i64) -> i64 {
        let n = self.p.chips() as i64;
        let b = b.rem_euclid(n);
        if b >= n / 2 {
            b - n
        } else {
            b
        }
    }

    /// Samples needed in the buffer to read a window starting at `t0`.
    fn window_end(&self, t0: f64) -> usize {
        (t0.floor() as i64 + ((self.p.chips() - 1) * self.p.os) as i64 + INTERP_HALF + 1).max(0) as usize
    }

    /// Fill `self.win` with one sample per chip starting at full-rate time `t0`.
    fn load_window(&mut self, t0: f64) {
        let n = self.p.chips();
        let os = self.p.os;
        let base = t0.floor();
        let mu = t0 - base;
        let base = base as i64;
        let len = self.buf.len() as i64;
        let buf = &self.buf;
        let at = |i: i64| if i >= 0 && i < len { buf[i as usize] } else { C32::ZERO };
        if mu < 1e-9 {
            for m in 0..n {
                self.win[m] = at(base + (m * os) as i64);
            }
            return;
        }
        // Blackman-windowed sinc fractional-delay weights (shared by the whole window).
        let mut w = [0f32; 2 * INTERP_HALF as usize];
        let mut sum = 0.0;
        for (j, k) in (-INTERP_HALF + 1..=INTERP_HALF).enumerate() {
            let x = k as f64 - mu;
            let sinc = if x.abs() < 1e-12 { 1.0 } else { (PI * x).sin() / (PI * x) };
            let r = (x + INTERP_HALF as f64) / (2 * INTERP_HALF) as f64;
            let win = 0.42 - 0.5 * (TAU * r).cos() + 0.08 * (2.0 * TAU * r).cos();
            w[j] = (sinc * win) as f32;
            sum += w[j];
        }
        for v in &mut w {
            *v /= sum;
        }
        for m in 0..n {
            let c = base + (m * os) as i64;
            let mut acc = C32::ZERO;
            for (j, k) in (-INTERP_HALF + 1..=INTERP_HALF).enumerate() {
                acc += at(c + k).scale(w[j]);
            }
            self.win[m] = acc;
        }
    }

    fn analyse_at(&mut self, t0: f64, down: bool, shift: f32) -> Peak {
        self.load_window(t0);
        let w = std::mem::take(&mut self.win);
        let pk = self.demod.analyse_shifted(&w, down, shift);
        self.win = w;
        pk
    }

    /// Feed samples; returns decoded packets.
    pub fn push(&mut self, x: &[C32]) -> Vec<Vec<u8>> {
        match &mut self.lpf {
            Some(f) => self.buf.extend(x.iter().map(|&v| f.filter(v))),
            None => self.buf.extend_from_slice(x),
        }
        let n = self.p.chips();
        let os = self.p.os;
        let l = n * os;
        let mut packets = Vec::new();
        loop {
            match self.state.clone() {
                State::Search { pos, run, last } => {
                    if self.window_end(pos as f64) > self.buf.len() {
                        break;
                    }
                    // Oversampled: also try half a chip later and keep the better
                    // alignment (a half-chip offset splits the energy at the wrap).
                    let mut pk = self.analyse_at(pos as f64, false, 0.0);
                    let mut at = pos;
                    if os > 1 && self.window_end((pos + os / 2) as f64) <= self.buf.len() {
                        let pk2 = self.analyse_at((pos + os / 2) as f64, false, 0.0);
                        if pk2.ratio > pk.ratio {
                            pk = pk2;
                            at = pos + os / 2;
                        }
                    }
                    let near = |a: u16, b: u16| {
                        let d = (a as i64 - b as i64).rem_euclid(n as i64);
                        d <= 1 || d >= n as i64 - 1
                    };
                    let (run, last) = if pk.ratio > self.thresh {
                        if run > 0 && near(pk.bin, last) {
                            (run + 1, pk.bin)
                        } else {
                            (1, pk.bin)
                        }
                    } else {
                        (0, 0)
                    };
                    if run >= self.min_run {
                        // Next up-chirp boundary at or after this window start.
                        let b0 = at + ((n - pk.bin as usize) % n) * os;
                        self.state = State::Sync { b0, j: 0 };
                    } else {
                        self.state = State::Search { pos: pos + l, run, last };
                    }
                }
                State::Sync { b0, j } => {
                    if j > self.p.preamble + 3 {
                        self.state = State::Search { pos: b0 + j * l, run: 0, last: 0 };
                        continue;
                    }
                    let b = b0 + j * l;
                    if self.window_end(b as f64) > self.buf.len() {
                        break;
                    }
                    let dn = self.analyse_at(b as f64, true, 0.0);
                    let up = self.analyse_at(b as f64, false, 0.0);
                    if dn.ratio > self.thresh && dn.power > up.power {
                        // With the window late by tau chips and a carrier offset of
                        // delta bins, an up-chirp peaks at tau + delta and the
                        // down-chirp at delta - tau. Measure the up-chirp in the
                        // window just before (the last preamble symbol).
                        let u = if b >= l {
                            let pu = self.analyse_at((b - l) as f64, false, 0.0);
                            self.signed_bin(pu.bin as i64) as f64 + pu.frac as f64
                        } else {
                            0.0
                        };
                        let d = self.signed_bin(dn.bin as i64) as f64 + dn.frac as f64;
                        let delta = (u + d) / 2.0;
                        let tau = (u - d) / 2.0;
                        let exact = (b as f64 - tau * os as f64 + 2.0 * l as f64).max(0.0);
                        self.state = if os == 1 {
                            // whole-sample timing; the residual shows up as a bin offset
                            let start = exact.round();
                            State::Payload { start, offset: delta + (start - exact) }
                        } else {
                            State::Payload { start: exact, offset: delta }
                        };
                    } else {
                        self.state = State::Sync { b0, j: j + 1 };
                    }
                }
                State::Payload { start, offset } => {
                    let cfo = offset.round() as i64;
                    let frac = (offset - cfo as f64) as f32;
                    let hs = self.p.header_symbols();
                    let t = |i: usize| start + (i * l) as f64;
                    if self.window_end(t(hs - 1)) > self.buf.len() {
                        break;
                    }
                    let mut bins = Vec::new();
                    let decode = |me: &mut ChirpRx, i: usize| {
                        let pk = me.analyse_at(t(i), false, frac);
                        (pk.bin as i64 - cfo).rem_euclid(n as i64) as u16
                    };
                    for i in 0..hs {
                        bins.push(decode(self, i));
                    }
                    let len = crate::coding::bits_to_bytes_msb(&symbols_to_bits(&self.p, &bins))[0] as usize;
                    let total = self.p.frame_symbols(len);
                    if self.window_end(t(total - 1)) > self.buf.len() {
                        break;
                    }
                    for i in hs..total {
                        bins.push(decode(self, i));
                    }
                    let bytes = crate::coding::bits_to_bytes_msb(&symbols_to_bits(&self.p, &bins));
                    let data = &bytes[1..1 + len];
                    let crc = u16::from_be_bytes([bytes[1 + len], bytes[2 + len]]);
                    let after = t(total).floor() as usize;
                    if crc == crc16_ccitt_false(data) {
                        packets.push(data.to_vec());
                        self.state = State::Search { pos: after, run: 0, last: 0 };
                    } else {
                        self.crc_errors += 1;
                        self.state = State::Search { pos: start.floor() as usize, run: 0, last: 0 };
                    }
                }
            }
        }
        // Trim consumed samples, keeping a margin for interpolation and the
        // up-chirp measured one symbol before a sync candidate.
        let margin = INTERP_HALF as usize + 1;
        let keep = match &self.state {
            State::Search { pos, .. } => pos.saturating_sub(margin),
            State::Sync { b0, .. } => b0.saturating_sub(l + margin),
            State::Payload { start, .. } => (start.floor() as usize).saturating_sub(margin),
        }
        .min(self.buf.len());
        if keep > 0 {
            self.buf.drain(..keep);
            match &mut self.state {
                State::Search { pos, .. } => *pos -= keep,
                State::Sync { b0, .. } => *b0 -= keep,
                State::Payload { start, .. } => *start -= keep as f64,
            }
        }
        packets
    }
}
