//! Numerically controlled oscillator, mixers and the Hilbert transformer.

use crate::complex::C32;
use crate::fir::FirRR;
use crate::window::Window;
use std::f64::consts::{PI, TAU};

/// NCO with a double-precision phase accumulator (no drift over long runs).
#[derive(Clone, Debug)]
pub struct Nco {
    phase: f64, // radians, kept in [-pi, pi)
    freq: f64,  // radians per sample
}

impl Nco {
    /// Oscillator at `freq_hz` for sample rate `fs` (negative frequencies allowed).
    pub fn new(freq_hz: f64, fs: f64) -> Nco {
        Nco { phase: 0.0, freq: TAU * freq_hz / fs }
    }

    /// Oscillator from a frequency in radians per sample.
    pub fn from_rad(freq: f64) -> Nco {
        Nco { phase: 0.0, freq }
    }

    pub fn set_freq(&mut self, freq_hz: f64, fs: f64) {
        self.freq = TAU * freq_hz / fs;
    }

    pub fn set_freq_rad(&mut self, w: f64) {
        self.freq = w;
    }

    pub fn freq_rad(&self) -> f64 {
        self.freq
    }

    pub fn phase(&self) -> f64 {
        self.phase
    }

    pub fn set_phase(&mut self, p: f64) {
        self.phase = wrap(p);
    }

    /// Nudge phase (radians) and frequency (radians/sample), for loops.
    pub fn adjust(&mut self, dphase: f64, dfreq: f64) {
        self.phase = wrap(self.phase + dphase);
        self.freq += dfreq;
    }

    /// Current phasor `e^{j phase}`, then advance one sample.
    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> C32 {
        let v = C32::expj64(self.phase);
        self.step();
        v
    }

    /// Current phasor without advancing.
    #[inline]
    pub fn current(&self) -> C32 {
        C32::expj64(self.phase)
    }

    #[inline]
    pub fn step(&mut self) {
        self.phase = wrap(self.phase + self.freq);
    }

    /// Real cosine output, advancing one sample.
    #[inline]
    pub fn next_cos(&mut self) -> f32 {
        let v = self.phase.cos() as f32;
        self.step();
        v
    }

    /// Multiply by `e^{+j phase}` (shift up by the NCO frequency).
    pub fn mix_up(&mut self, input: &[C32], out: &mut [C32]) {
        for (x, y) in input.iter().zip(out.iter_mut()) {
            *y = *x * self.next();
        }
    }

    /// Multiply by `e^{-j phase}` (shift down by the NCO frequency).
    pub fn mix_down(&mut self, input: &[C32], out: &mut [C32]) {
        for (x, y) in input.iter().zip(out.iter_mut()) {
            *y = x.mul_conj(self.next());
        }
    }

    /// In-place frequency shift down.
    pub fn mix_down_inplace(&mut self, buf: &mut [C32]) {
        for v in buf {
            *v = v.mul_conj(self.next());
        }
    }

    /// In-place frequency shift up.
    pub fn mix_up_inplace(&mut self, buf: &mut [C32]) {
        for v in buf {
            *v *= self.next();
        }
    }
}

#[inline]
fn wrap(p: f64) -> f64 {
    let mut p = p;
    if !(-PI..PI).contains(&p) {
        p = (p + PI).rem_euclid(TAU) - PI;
    }
    p
}

/// Sample-by-sample product of two complex streams.
pub fn mix(a: &[C32], b: &[C32], out: &mut [C32]) {
    for ((x, y), o) in a.iter().zip(b).zip(out.iter_mut()) {
        *o = *x * *y;
    }
}

/// Shift a complex block by `shift_hz` (stateful through the given NCO-free
/// helper; for streaming use an [`Nco`]).
pub fn frequency_shift(input: &[C32], shift_hz: f64, fs: f64) -> Vec<C32> {
    let mut n = Nco::new(shift_hz, fs);
    let mut out = vec![C32::ZERO; input.len()];
    n.mix_up(input, &mut out);
    out
}

/// Type-III FIR Hilbert transformer taps (odd length, Blackman window).
pub fn hilbert_taps(num_taps: usize) -> Vec<f32> {
    let n = num_taps | 1;
    let w = Window::Blackman.symmetric(n);
    let m = (n / 2) as i64;
    (0..n as i64)
        .map(|i| {
            let k = i - m;
            if k % 2 == 0 {
                0.0
            } else {
                (2.0 / (PI * k as f64)) as f32 * w[i as usize]
            }
        })
        .collect()
}

/// Real -> analytic signal: output `x[n-D] + j H{x}[n]`, with `D = (taps-1)/2`.
/// Negative frequencies are suppressed.
#[derive(Clone, Debug)]
pub struct Hilbert {
    fir: FirRR,
    delay: Vec<f32>,
    dpos: usize,
}

impl Hilbert {
    /// `num_taps` odd, e.g. 65 (more taps = flatter response near DC/Nyquist).
    pub fn new(num_taps: usize) -> Hilbert {
        let taps = hilbert_taps(num_taps);
        let d = taps.len() / 2;
        Hilbert { fir: FirRR::new(taps), delay: vec![0.0; d.max(1)], dpos: 0 }
    }

    /// Group delay in samples.
    pub fn delay(&self) -> usize {
        self.fir.len() / 2
    }

    /// Hilbert-filter a single sample and return the matching delayed input
    /// (`(x[n-D], H{x}[n])`).
    #[inline]
    pub fn step(&mut self, x: f32) -> (f32, f32) {
        let h = self.fir.filter(x);
        let d = if self.delay() == 0 {
            x
        } else {
            let old = self.delay[self.dpos];
            self.delay[self.dpos] = x;
            self.dpos = (self.dpos + 1) % self.delay.len();
            old
        };
        (d, h)
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> C32 {
        let (d, h) = self.step(x);
        C32::new(d, h)
    }

    pub fn process_block(&mut self, input: &[f32], out: &mut [C32]) {
        for (x, y) in input.iter().zip(out.iter_mut()) {
            *y = self.process(*x);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fir::freq_response;

    #[test]
    fn nco_phase_continuity_and_mixing() {
        let fs = 48_000.0;
        let mut n = Nco::new(1000.0, fs);
        let a: Vec<C32> = (0..480).map(|_| n.next()).collect();
        // 480 samples of 1 kHz at 48 kHz = 10 full cycles -> back to phase 0
        assert!((n.current() - C32::ONE).abs() < 1e-9 + 1e-6);
        let mut down = Nco::new(1000.0, fs);
        let mut out = vec![C32::ZERO; a.len()];
        down.mix_down(&a, &mut out);
        assert!(out.iter().all(|v| (*v - C32::ONE).abs() < 1e-5));
        let shifted = frequency_shift(&out, -250.0, fs);
        let d = shifted[100].mul_conj(shifted[99]).arg();
        assert!((d as f64 + TAU * 250.0 / fs).abs() < 1e-5);
    }

    #[test]
    fn hilbert_analytic_suppresses_negative_frequencies() {
        let taps = hilbert_taps(101);
        // |H| ~ 1 in midband
        for f in [0.1f32, 0.15, 0.25, 0.35, 0.4] {
            assert!((freq_response(&taps, f).abs() - 1.0).abs() < 0.01, "f={f}");
        }
        let mut h = Hilbert::new(101);
        let fs = 8000.0f32;
        let x: Vec<f32> = (0..4000).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / fs).cos()).collect();
        let mut y = vec![C32::ZERO; x.len()];
        h.process_block(&x, &mut y);
        // analytic signal of cos is e^{j w t}: constant magnitude, positive rotation
        for v in &y[200..] {
            assert!((v.abs() - 1.0).abs() < 0.01);
        }
        let rot = y[1000].mul_conj(y[999]).arg();
        assert!((rot - 2.0 * std::f32::consts::PI * 1000.0 / fs).abs() < 1e-3);
    }
}
