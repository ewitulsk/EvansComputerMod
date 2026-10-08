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
        let n = self.p.chips() as f64;
        let os = self.p.os as f64;
        let fs = self.p.fs() as f64;
        let bw = self.p.bw as f64;
        for m in 0..self.p.symbol_len() {
            let chip = m as f64 / os;
            let f = if down {
                -(chip / n - 0.5) * bw
            } else {
                (((chip + sym as f64) % n) / n - 0.5) * bw
            };
            out.push(C32::expj64(self.phase));
            self.phase = (self.phase + TAU * f / fs + PI).rem_euclid(TAU) - PI;
        }
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
        let l = self.p.symbol_len();
        let n = self.p.chips();
        for i in 0..l {
            self.buf[i] = if down { w[i] * self.up[i] } else { w[i].mul_conj(self.up[i]) };
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
        Peak { bin: best as u16, ratio: bp / mean.max(1e-30), power: bp }
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
    Payload { start: usize, cfo: i32 },
}

/// Streaming CSS packet receiver: preamble detection, timing and integer
/// carrier-offset sync from the down-chirps, payload decode and CRC check.
#[derive(Clone, Debug)]
pub struct ChirpRx {
    p: ChirpParams,
    demod: ChirpDemod,
    buf: Vec<C32>,
    state: State,
    thresh: f32,
    min_run: usize,
    /// Packets that failed the CRC (diagnostics).
    pub crc_errors: u64,
}

impl ChirpRx {
    pub fn new(p: ChirpParams) -> ChirpRx {
        let n = p.chips() as f32;
        ChirpRx {
            p,
            demod: ChirpDemod::new(p),
            buf: Vec::new(),
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

    /// Feed samples; returns decoded packets.
    pub fn push(&mut self, x: &[C32]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(x);
        let l = self.p.symbol_len();
        let n = self.p.chips();
        let os = self.p.os;
        let mut packets = Vec::new();
        loop {
            match self.state.clone() {
                State::Search { pos, run, last } => {
                    if pos + l > self.buf.len() {
                        break;
                    }
                    let pk = self.demod.analyse(&self.buf[pos..pos + l], false);
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
                        let b0 = pos + ((n - pk.bin as usize) % n) * os;
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
                    if b + l > self.buf.len() {
                        break;
                    }
                    let w = &self.buf[b..b + l];
                    let dn = self.demod.analyse(w, true);
                    let up = self.demod.analyse(w, false);
                    if dn.ratio > self.thresh && dn.power > up.power {
                        let delta = self.signed_bin(dn.bin as i64) as f64 / 2.0;
                        let shift = (delta * os as f64).round() as i64;
                        let start = (b as i64 + shift + 2 * l as i64).max(0) as usize;
                        self.state = State::Payload { start, cfo: delta.round() as i32 };
                    } else {
                        self.state = State::Sync { b0, j: j + 1 };
                    }
                }
                State::Payload { start, cfo } => {
                    let hs = self.p.header_symbols();
                    if start + hs * l > self.buf.len() {
                        break;
                    }
                    let mut bins = Vec::new();
                    let decode = |me: &mut ChirpRx, i: usize| {
                        let s = start + i * l;
                        let pk = me.demod.analyse(&me.buf[s..s + l], false);
                        (pk.bin as i64 - cfo as i64).rem_euclid(n as i64) as u16
                    };
                    for i in 0..hs {
                        bins.push(decode(self, i));
                    }
                    let len = crate::coding::bits_to_bytes_msb(&symbols_to_bits(&self.p, &bins))[0] as usize;
                    let total = self.p.frame_symbols(len);
                    if start + total * l > self.buf.len() {
                        break;
                    }
                    for i in hs..total {
                        bins.push(decode(self, i));
                    }
                    let bytes = crate::coding::bits_to_bytes_msb(&symbols_to_bits(&self.p, &bins));
                    let data = &bytes[1..1 + len];
                    let crc = u16::from_be_bytes([bytes[1 + len], bytes[2 + len]]);
                    if crc == crc16_ccitt_false(data) {
                        packets.push(data.to_vec());
                        self.state = State::Search { pos: start + total * l, run: 0, last: 0 };
                    } else {
                        self.crc_errors += 1;
                        self.state = State::Search { pos: start, run: 0, last: 0 };
                    }
                }
            }
        }
        // Trim consumed samples.
        let keep = match &self.state {
            State::Search { pos, .. } => *pos,
            State::Sync { b0, .. } => b0.saturating_sub(l),
            State::Payload { start, .. } => *start,
        }
        .min(self.buf.len());
        if keep > 0 {
            self.buf.drain(..keep);
            match &mut self.state {
                State::Search { pos, .. } => *pos -= keep,
                State::Sync { b0, .. } => *b0 -= keep,
                State::Payload { start, .. } => *start -= keep,
            }
        }
        packets
    }
}
