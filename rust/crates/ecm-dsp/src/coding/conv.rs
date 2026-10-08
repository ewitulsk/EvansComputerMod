//! Rate-1/2, constraint-length-7 convolutional code (the NASA/CCSDS "Voyager"
//! code, generators 171 and 133 octal) with hard- and soft-decision Viterbi.
//!
//! Bits are `u8` values 0/1. Soft values are `f32` where **positive means 1**
//! and the magnitude is the confidence (e.g. a BPSK matched-filter output
//! mapped with bit 1 -> +1).

/// Generator polynomials (octal 171, 133).
pub const G1: u8 = 0o171;
pub const G2: u8 = 0o133;
/// Constraint length.
pub const K: usize = 7;
const STATES: usize = 1 << (K - 1);

#[inline]
fn parity(x: u8) -> u8 {
    (x.count_ones() & 1) as u8
}

#[inline]
fn outputs(sr: u8) -> (u8, u8) {
    (parity(sr & G1), parity(sr & G2))
}

/// Streaming convolutional encoder.
#[derive(Clone, Debug, Default)]
pub struct ConvEncoder {
    sr: u8,
}

impl ConvEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Encode bits, appending two coded bits per input bit.
    pub fn encode_into(&mut self, bits: &[u8], out: &mut Vec<u8>) {
        for &b in bits {
            self.sr = ((self.sr << 1) | (b & 1)) & 0x7F;
            let (a, c) = outputs(self.sr);
            out.push(a);
            out.push(c);
        }
    }

    /// Flush with `K-1` zero tail bits, returning the encoder to state 0.
    pub fn flush_into(&mut self, out: &mut Vec<u8>) {
        self.encode_into(&[0; K - 1], out);
    }

    pub fn reset(&mut self) {
        self.sr = 0;
    }
}

/// Encode a block and terminate it (output length `2 * (bits.len() + 6)`).
pub fn encode_terminated(bits: &[u8]) -> Vec<u8> {
    let mut e = ConvEncoder::new();
    let mut out = Vec::with_capacity(2 * (bits.len() + K - 1));
    e.encode_into(bits, &mut out);
    e.flush_into(&mut out);
    out
}

/// Soft-decision Viterbi decode. `soft.len()` must be even. If `terminated`,
/// the trellis is forced to end in state 0 and the `K-1` tail bits are removed.
pub fn viterbi_decode_soft(soft: &[f32], terminated: bool) -> Vec<u8> {
    assert!(soft.len() % 2 == 0, "need pairs of coded bits");
    let steps = soft.len() / 2;
    // Precompute expected outputs (+-1) for every 7-bit register value.
    let mut exp = [(0f32, 0f32); 128];
    for (sr, e) in exp.iter_mut().enumerate() {
        let (a, b) = outputs(sr as u8);
        *e = (2.0 * a as f32 - 1.0, 2.0 * b as f32 - 1.0);
    }
    let mut metric = [f32::NEG_INFINITY; STATES];
    metric[0] = 0.0;
    let mut decisions: Vec<u64> = Vec::with_capacity(steps);
    let mut next = [0f32; STATES];
    for t in 0..steps {
        let (r0, r1) = (soft[2 * t], soft[2 * t + 1]);
        let mut dec = 0u64;
        for ns in 0..STATES {
            // predecessors: s = (ns >> 1) | (top << 5); register = (s << 1) | (ns & 1)
            let s0 = ns >> 1;
            let s1 = s0 | (1 << (K - 2));
            let sr0 = ((s0 << 1) | (ns & 1)) as usize & 0x7F;
            let sr1 = ((s1 << 1) | (ns & 1)) as usize & 0x7F;
            let m0 = metric[s0] + r0 * exp[sr0].0 + r1 * exp[sr0].1;
            // sr1 has bit 6 set (the bit shifted out of the 6-bit state)
            let sr1 = sr1 | 0x40;
            let m1 = metric[s1] + r0 * exp[sr1].0 + r1 * exp[sr1].1;
            if m1 > m0 {
                next[ns] = m1;
                dec |= 1 << ns;
            } else {
                next[ns] = m0;
            }
        }
        decisions.push(dec);
        // renormalise
        let best = next.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        for (m, n) in metric.iter_mut().zip(next.iter()) {
            *m = *n - best;
        }
    }
    let mut state = if terminated {
        0
    } else {
        (0..STATES).max_by(|&a, &b| metric[a].total_cmp(&metric[b])).unwrap_or(0)
    };
    let mut bits = vec![0u8; steps];
    for t in (0..steps).rev() {
        bits[t] = (state & 1) as u8;
        let top = ((decisions[t] >> state) & 1) as usize;
        state = (state >> 1) | (top << (K - 2));
    }
    if terminated {
        bits.truncate(steps.saturating_sub(K - 1));
    }
    bits
}

/// Hard-decision Viterbi decode of 0/1 coded bits.
pub fn viterbi_decode_hard(coded: &[u8], terminated: bool) -> Vec<u8> {
    let soft: Vec<f32> = coded.iter().map(|&b| if b & 1 == 1 { 1.0 } else { -1.0 }).collect();
    viterbi_decode_soft(&soft, terminated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    fn rand_bits(n: usize, seed: u64) -> Vec<u8> {
        let mut r = Rng::new(seed);
        (0..n).map(|_| r.bit()).collect()
    }

    #[test]
    fn clean_round_trip() {
        let bits = rand_bits(500, 1);
        let c = encode_terminated(&bits);
        assert_eq!(c.len(), 2 * 506);
        assert_eq!(viterbi_decode_hard(&c, true), bits);
        // Known first outputs for a single 1 bit: impulse response = generator taps
        let imp = encode_terminated(&[1]);
        // register 0000001: G1=1111001 lsb=1 ->1, G2=1011011 lsb=1 ->1
        assert_eq!(&imp[..2], &[1, 1]);
    }

    #[test]
    fn hard_viterbi_corrects_scattered_errors() {
        let bits = rand_bits(1000, 2);
        let mut c = encode_terminated(&bits);
        // flip one coded bit every 20 (free distance 10 handles isolated errors)
        for i in (5..c.len()).step_by(20) {
            c[i] ^= 1;
        }
        assert_eq!(viterbi_decode_hard(&c, true), bits);
    }

    #[test]
    fn soft_viterbi_beats_uncoded_on_awgn() {
        // BPSK over AWGN at Eb/N0 = 4 dB: uncoded BER ~ 1.25e-2, coded ~ 1e-5.
        let mut rng = Rng::new(3);
        let n = 20_000;
        let bits = rand_bits(n, 4);
        let c = encode_terminated(&bits);
        let ebn0 = 10f64.powf(4.0 / 10.0);
        // rate 1/2: Es = Eb/2 -> sigma^2 = N0/2 with Es = 1 => N0 = 2/ebn0... per coded bit
        let sigma = (1.0 / (2.0 * 0.5 * ebn0)).sqrt();
        let soft: Vec<f32> = c
            .iter()
            .map(|&b| ((2.0 * b as f64 - 1.0) + sigma * rng.gaussian()) as f32)
            .collect();
        let hard_errs = soft.iter().zip(&c).filter(|(s, &b)| ((**s > 0.0) as u8) != b).count();
        let dec = viterbi_decode_soft(&soft, true);
        let errs = dec.iter().zip(&bits).filter(|(a, b)| a != b).count();
        assert!(hard_errs > 200, "channel errors {hard_errs}");
        assert!(errs <= 5, "decoded errors {errs} of {n}");
        // unterminated decoding of a clean stream also works
        let clean: Vec<f32> = c.iter().map(|&b| 2.0 * b as f32 - 1.0).collect();
        let dec2 = viterbi_decode_soft(&clean, false);
        assert_eq!(&dec2[..n], &bits[..]);
    }
}
