//! BPSK / QPSK with root-raised-cosine pulse shaping.
//!
//! Mapping: BPSK bit 0 -> +1, bit 1 -> -1. QPSK (Gray) bits `(b0, b1)` ->
//! `((1 - 2 b0) + j (1 - 2 b1)) / sqrt(2)`. Symbols have unit energy and the
//! RRC pulse has unit energy, so after the matched filter a symbol has
//! amplitude 1 and `Eb/N0` follows directly from the per-sample noise power:
//! `N0 = sigma^2 (complex)`, `Eb = 1 / bits_per_symbol`.
//!
//! The receiver chain is matched filter -> Gardner timing recovery -> Costas
//! loop. A coherent receiver has an inherent phase ambiguity (180 deg for
//! BPSK, 90 deg for QPSK); enable `differential` on both ends to remove it, or
//! resolve it with a known preamble.

use crate::complex::C32;
use crate::fir::{rrc, FirRC};
use crate::sync::{Costas, Decision, SymbolSync, Ted};
use std::f32::consts::FRAC_1_SQRT_2;

/// PSK modem parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PskParams {
    /// 2 (BPSK) or 4 (QPSK).
    pub order: u8,
    /// Samples per symbol (integer, >= 2).
    pub sps: usize,
    /// RRC roll-off (0.2..0.5).
    pub beta: f32,
    /// RRC length in symbols.
    pub span: usize,
    /// Differential encoding (DBPSK / DQPSK).
    pub differential: bool,
}

impl PskParams {
    pub fn bpsk(sps: usize) -> PskParams {
        PskParams { order: 2, sps, beta: 0.35, span: 10, differential: false }
    }

    pub fn qpsk(sps: usize) -> PskParams {
        PskParams { order: 4, sps, beta: 0.35, span: 10, differential: false }
    }

    pub fn bits_per_symbol(&self) -> usize {
        if self.order == 4 {
            2
        } else {
            1
        }
    }
}

/// Map a symbol index (BPSK: 0..2, QPSK: 0..4 in bit order b0 b1) to a point.
fn map(order: u8, idx: u8) -> C32 {
    if order == 2 {
        C32::new(if idx & 1 == 0 { 1.0 } else { -1.0 }, 0.0)
    } else {
        let b0 = idx & 1;
        let b1 = (idx >> 1) & 1;
        C32::new(1.0 - 2.0 * b0 as f32, 1.0 - 2.0 * b1 as f32).scale(FRAC_1_SQRT_2)
    }
}

/// Phase index (multiples of 90 deg) for QPSK Gray points, and back.
fn qpsk_quadrant(idx: u8) -> u8 {
    // idx bits (b0,b1): 00 -> 45deg (q0), 10(b1=1)-> -45 (q3), 11 -> -135 (q2), 01(b0=1) -> 135 (q1)
    match idx & 3 {
        0 => 0,
        1 => 1,
        3 => 2,
        _ => 3,
    }
}

fn quadrant_to_idx(q: u8) -> u8 {
    match q & 3 {
        0 => 0,
        1 => 1,
        2 => 3,
        _ => 2,
    }
}

/// PSK modulator: bits -> RRC-shaped complex baseband.
#[derive(Clone, Debug)]
pub struct PskMod {
    p: PskParams,
    fir: FirRC,
    pending: Vec<u8>,
    diff: u8,
}

impl PskMod {
    pub fn new(p: PskParams) -> PskMod {
        assert!(p.order == 2 || p.order == 4);
        assert!(p.sps >= 2);
        PskMod { p, fir: FirRC::new(rrc(p.sps, p.span, p.beta)), pending: Vec::new(), diff: 0 }
    }

    /// Filter delay in samples (pulse centre).
    pub fn delay(&self) -> usize {
        self.p.sps * self.p.span / 2
    }

    fn symbol(&mut self, idx: u8) -> C32 {
        if !self.p.differential {
            return map(self.p.order, idx);
        }
        if self.p.order == 2 {
            self.diff ^= idx & 1;
            map(2, self.diff)
        } else {
            self.diff = (self.diff + qpsk_quadrant(idx)) & 3;
            map(4, quadrant_to_idx(self.diff))
        }
    }

    /// Emit one symbol's worth of samples for a symbol point.
    pub fn push_symbol(&mut self, s: C32, out: &mut Vec<C32>) {
        out.push(self.fir.filter(s));
        for _ in 1..self.p.sps {
            out.push(self.fir.filter(C32::ZERO));
        }
    }

    /// Modulate bits (a partial QPSK symbol is held until the next call).
    pub fn modulate(&mut self, bits: &[u8], out: &mut Vec<C32>) {
        let k = self.p.bits_per_symbol();
        self.pending.extend_from_slice(bits);
        let full = self.pending.len() / k * k;
        let take: Vec<u8> = self.pending.drain(..full).collect();
        for c in take.chunks(k) {
            let idx = if k == 2 { (c[0] & 1) | ((c[1] & 1) << 1) } else { c[0] & 1 };
            let s = self.symbol(idx);
            self.push_symbol(s, out);
        }
    }

    /// Push zeros to drain the pulse-shaping filter tail.
    pub fn flush(&mut self, out: &mut Vec<C32>) {
        for _ in 0..self.p.span {
            self.push_symbol(C32::ZERO, out);
        }
    }

    pub fn modulate_vec(&mut self, bits: &[u8]) -> Vec<C32> {
        let mut v = Vec::new();
        self.modulate(bits, &mut v);
        v
    }
}

/// Hard decision of a symbol to its index.
pub fn slice(order: u8, y: C32) -> u8 {
    if order == 2 {
        (y.re < 0.0) as u8
    } else {
        ((y.re < 0.0) as u8) | (((y.im < 0.0) as u8) << 1)
    }
}

/// PSK receiver: matched filter, Gardner timing recovery, Costas loop.
#[derive(Clone, Debug)]
pub struct PskDemod {
    p: PskParams,
    mf: FirRC,
    sync: SymbolSync,
    costas: Costas,
    prev: C32,
    tmp: Vec<C32>,
    syms: Vec<C32>,
}

impl PskDemod {
    /// Default loop bandwidths: timing 0.005, carrier 0.01 (rad/symbol).
    pub fn new(p: PskParams) -> PskDemod {
        Self::with_loops(p, 0.001, 0.005)
    }

    pub fn with_loops(p: PskParams, timing_bw: f32, carrier_bw: f32) -> PskDemod {
        let dec = if p.order == 2 { Decision::Bpsk } else { Decision::Qpsk };
        PskDemod {
            p,
            mf: FirRC::new(rrc(p.sps, p.span, p.beta)),
            sync: SymbolSync::new(Ted::Gardner, dec, p.sps as f32, timing_bw, 0.01),
            costas: Costas::new(p.order, carrier_bw),
            prev: C32::ONE,
            tmp: Vec::new(),
            syms: Vec::new(),
        }
    }

    /// Demodulate samples into carrier-corrected symbols (one per symbol).
    pub fn demod_symbols(&mut self, input: &[C32], out: &mut Vec<C32>) {
        self.tmp.clear();
        self.tmp.extend(input.iter().map(|&x| self.mf.filter(x)));
        self.syms.clear();
        self.sync.process(&self.tmp, &mut self.syms);
        for &s in &self.syms {
            out.push(self.costas.process(s));
        }
    }

    /// Symbols -> bits (handles differential decoding).
    pub fn symbols_to_bits(&mut self, syms: &[C32], out: &mut Vec<u8>) {
        for &y in syms {
            let idx = if self.p.differential {
                let d = y.mul_conj(self.prev);
                self.prev = y;
                if self.p.order == 2 {
                    slice(2, d)
                } else {
                    // rotate by 45 deg so quadrants map onto the Gray points
                    let r = d * C32::expj(std::f32::consts::FRAC_PI_4);
                    let q = qpsk_quadrant(slice(4, r));
                    quadrant_to_idx(q)
                }
            } else {
                slice(self.p.order, y)
            };
            out.push(idx & 1);
            if self.p.order == 4 {
                out.push((idx >> 1) & 1);
            }
        }
    }

    /// Demodulate straight to hard bits.
    pub fn demod_bits(&mut self, input: &[C32], out: &mut Vec<u8>) {
        let mut syms = Vec::new();
        self.demod_symbols(input, &mut syms);
        self.symbols_to_bits(&syms, out);
    }
}

/// Soft bits from (non-differential) symbols, positive = bit 1, for the soft Viterbi.
pub fn soft_bits(order: u8, syms: &[C32]) -> Vec<f32> {
    let mut v = Vec::with_capacity(syms.len() * if order == 4 { 2 } else { 1 });
    for y in syms {
        v.push(-y.re);
        if order == 4 {
            v.push(-y.im);
        }
    }
    v
}
