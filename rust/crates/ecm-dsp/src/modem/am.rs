//! AM (double sideband with carrier) and its envelope detector.

use crate::complex::C32;
use crate::iir::DcBlocker;

/// AM modulator: `s = carrier * (1 + m * audio)` at complex baseband.
///
/// - `index` (m): modulation depth, 0..1 (audio is expected in -1..1; values
///   driving `1 + m a` below 0 over-modulate and are clipped at 0).
/// - `carrier`: carrier amplitude.
#[derive(Clone, Debug)]
pub struct AmMod {
    pub index: f32,
    pub carrier: f32,
}

impl AmMod {
    pub fn new(index: f32, carrier: f32) -> AmMod {
        AmMod { index, carrier }
    }

    #[inline]
    pub fn modulate_sample(&self, a: f32) -> C32 {
        C32::new(self.carrier * (1.0 + self.index * a).max(0.0), 0.0)
    }

    pub fn modulate(&self, audio: &[f32], out: &mut [C32]) {
        for (a, o) in audio.iter().zip(out.iter_mut()) {
            *o = self.modulate_sample(*a);
        }
    }

    pub fn modulate_vec(&self, audio: &[f32]) -> Vec<C32> {
        audio.iter().map(|&a| self.modulate_sample(a)).collect()
    }
}

/// Envelope detector: `|x|`, carrier removed with a DC blocker and the output
/// normalised by the tracked carrier level, so a signal from [`AmMod`] with
/// index `m` comes back as `audio` (when `index` matches) at any received level.
/// Works without carrier phase/frequency lock (tolerates small offsets).
#[derive(Clone, Debug)]
pub struct AmDemod {
    dc: DcBlocker<f32>,
    level: f32,
    alpha: f32,
    index: f32,
}

impl AmDemod {
    /// `fs` sample rate; `index` the expected modulation depth (1.0 if unknown).
    pub fn new(fs: f32, index: f32) -> AmDemod {
        AmDemod {
            dc: DcBlocker::with_corner(30.0, fs),
            level: 0.0,
            alpha: (1.0 / (0.05 * fs)).min(1.0), // ~50 ms carrier-level average
            index,
        }
    }

    #[inline]
    pub fn demod_sample(&mut self, x: C32) -> f32 {
        let e = x.abs();
        if self.level == 0.0 {
            self.level = e;
        }
        self.level += self.alpha * (e - self.level);
        let a = self.dc.process(e);
        if self.level > 1e-12 {
            a / (self.level * self.index)
        } else {
            0.0
        }
    }

    pub fn demod(&mut self, input: &[C32], out: &mut [f32]) {
        for (x, o) in input.iter().zip(out.iter_mut()) {
            *o = self.demod_sample(*x);
        }
    }

    pub fn demod_vec(&mut self, input: &[C32]) -> Vec<f32> {
        input.iter().map(|&x| self.demod_sample(x)).collect()
    }
}
