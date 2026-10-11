//! Modulators and demodulators. Every modem has a streaming modulator and a
//! matching streaming demodulator; state persists across calls.
//!
//! Complex baseband is centred on the tuned frequency (carrier at 0 Hz)
//! unless a modem says otherwise.

pub mod am;
pub mod chirp;
pub mod cw;
pub mod fm;
pub mod fsk;
pub mod psk;
pub mod ssb;

/// Digital PLL bit-clock recovery for 2-level soft streams (Direwolf-style).
///
/// Feed one soft value per sample (`> 0` means 1). Transitions pull the clock
/// phase towards the bit boundary; a bit is emitted at each bit centre.
#[derive(Clone, Debug)]
pub struct BitClock {
    step: f32,
    phase: f32,
    prev: f32,
    gain: f32,
}

impl BitClock {
    /// `sps` samples per bit; `gain` in `(0, 1)` is how strongly transitions
    /// re-time the clock (0.3 when locked onto clean data is typical).
    pub fn new(sps: f32, gain: f32) -> BitClock {
        BitClock { step: 1.0 / sps, phase: 0.0, prev: 0.0, gain }
    }

    /// Push one soft sample. Returns `Some((bit, soft))` at bit centres.
    #[inline]
    pub fn push(&mut self, v: f32) -> Option<(u8, f32)> {
        if (v > 0.0) != (self.prev > 0.0) {
            // Transition: ideal phase here is 0 (== 1). Pull towards it.
            if self.phase < 0.5 {
                self.phase -= self.phase * self.gain;
            } else {
                self.phase += (1.0 - self.phase) * self.gain;
            }
        }
        self.prev = v;
        let before = self.phase;
        self.phase += self.step;
        let mut out = None;
        if before < 0.5 && self.phase >= 0.5 {
            out = Some(((v > 0.0) as u8, v));
        }
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            if self.phase >= 0.5 {
                // step larger than half a bit (very low sps): emit anyway
                out = Some(((v > 0.0) as u8, v));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_clock_recovers_bits_with_offset_clock() {
        // 40 samples/bit nominal, transmitted at 40.2 samples/bit, phase-offset
        let bits: Vec<u8> = (0..500).map(|i| ((i * 7 + i / 3) % 5 < 2) as u8).collect();
        let mut wave = Vec::new();
        let mut t = 13.0f32;
        for &b in &bits {
            t += 40.2;
            while (wave.len() as f32) < t {
                wave.push(if b == 1 { 1.0 } else { -1.0 });
            }
        }
        let mut bc = BitClock::new(40.0, 0.3);
        let out: Vec<u8> = wave.iter().filter_map(|&v| bc.push(v)).map(|(b, _)| b).collect();
        // find alignment and compare
        let pos = (0..5).find(|&o| out[o..o + 100] == bits[..100]).expect("aligned");
        let n = bits.len().min(out.len() - pos) - 1;
        assert_eq!(&out[pos..pos + n], &bits[..n]);
    }
}
