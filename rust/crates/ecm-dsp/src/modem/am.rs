//! AM (double sideband with carrier) and its envelope detector.

use crate::complex::C32;

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

/// Envelope detector: `|x|`, minus the tracked carrier level and divided by
/// it, so a signal from [`AmMod`] with
/// index `m` comes back as `audio` (when `index` matches) at any received level.
/// Works without carrier phase/frequency lock (tolerates small offsets).
///
/// The carrier level (the envelope's ~50 ms moving average, which also
/// removes the carrier: a ~3 Hz high-pass) starts as the plain running mean of
/// the envelope, so a receiver whose filters fade in from silence isn't
/// divided by a tiny first sample.
#[derive(Clone, Debug)]
pub struct AmDemod {
    level: f32,
    alpha: f32,
    index: f32,
    seen: u32,
}

impl AmDemod {
    /// `fs` sample rate; `index` the expected modulation depth (1.0 if unknown).
    pub fn new(fs: f32, index: f32) -> AmDemod {
        AmDemod {
            level: 0.0,
            alpha: (1.0 / (0.05 * fs)).min(1.0), // ~50 ms carrier-level average
            index,
            seen: 0,
        }
    }

    #[inline]
    pub fn demod_sample(&mut self, x: C32) -> f32 {
        let e = x.abs();
        self.seen = self.seen.saturating_add(1);
        // Running mean until ~50 ms have been seen, then the moving average.
        let alpha = self.alpha.max(1.0 / self.seen as f32);
        self.level += alpha * (e - self.level);
        if self.level > 1e-12 {
            (e - self.level) / (self.level * self.index)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A signal that fades in from (almost) nothing, as a receiver's FIR
    /// filter delivers at start-up: the carrier-level normaliser must not
    /// divide by that first tiny sample (it used to, giving outputs in the
    /// hundreds for the first ~100 ms).
    #[test]
    fn am_demod_has_no_startup_transient() {
        let fs = 48_000.0;
        let m = AmMod::new(0.8, 0.5);
        let mut x: Vec<C32> = (0..48_000)
            .map(|n| m.modulate_sample(0.9 * (std::f32::consts::TAU * 1000.0 * n as f32 / fs).sin()))
            .collect();
        // FIR-like ramp over the first 64 samples, starting at 1e-8.
        for (n, v) in x.iter_mut().take(64).enumerate() {
            *v = *v * (1e-8 + n as f32 / 64.0);
        }
        let y = AmDemod::new(fs, 0.8).demod_vec(&x);
        let start = y[..4800].iter().fold(0.0f32, |a, v| a.max(v.abs()));
        // (bounded by the ramp itself: ~(2 x 1.72 - 1) / 0.8; it was ~3000)
        assert!(start < 4.0, "start-up peak {start}");
        // Steady state: the audio comes back at its own level (0.9).
        let late = y[24_000..].iter().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!((late - 0.9).abs() < 0.1, "steady-state peak {late}");
    }
}
