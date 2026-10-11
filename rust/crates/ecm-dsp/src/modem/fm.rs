//! Frequency modulation: NBFM and WBFM with optional pre-/de-emphasis.

use crate::complex::C32;
use crate::iir::{OnePole, PreEmphasis};
use std::f64::consts::{PI, TAU};

/// FM parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FmParams {
    /// Sample rate (Hz).
    pub fs: f32,
    /// Peak deviation (Hz) for audio at +-1.
    pub deviation: f32,
    /// Emphasis time constant in seconds (75e-6 US broadcast, 50e-6 EU), or 0 for none.
    pub tau: f32,
}

impl FmParams {
    /// Narrowband FM (voice/packet): 5 kHz deviation (2.5 kHz for 12.5 kHz
    /// channels), no emphasis.
    pub fn nbfm(fs: f32) -> FmParams {
        FmParams { fs, deviation: 5_000.0, tau: 0.0 }
    }

    /// Broadcast FM: 75 kHz deviation, 75 us emphasis. Needs `fs >= ~200 kHz`.
    pub fn wbfm(fs: f32) -> FmParams {
        FmParams { fs, deviation: 75_000.0, tau: 75e-6 }
    }
}

/// FM modulator (continuous phase), audio -> complex baseband of unit amplitude.
#[derive(Clone, Debug)]
pub struct FmMod {
    k: f64,
    phase: f64,
    pre: Option<PreEmphasis>,
}

impl FmMod {
    pub fn new(p: FmParams) -> FmMod {
        FmMod {
            k: TAU * p.deviation as f64 / p.fs as f64,
            phase: 0.0,
            pre: (p.tau > 0.0).then(|| PreEmphasis::from_tau(p.tau, p.fs)),
        }
    }

    #[inline]
    pub fn modulate_sample(&mut self, a: f32) -> C32 {
        let a = match &mut self.pre {
            Some(p) => p.process(a),
            None => a,
        };
        self.phase += self.k * a as f64;
        if !(-PI..PI).contains(&self.phase) {
            self.phase = (self.phase + PI).rem_euclid(TAU) - PI;
        }
        C32::expj64(self.phase)
    }

    pub fn modulate(&mut self, audio: &[f32], out: &mut [C32]) {
        for (a, o) in audio.iter().zip(out.iter_mut()) {
            *o = self.modulate_sample(*a);
        }
    }

    pub fn modulate_vec(&mut self, audio: &[f32]) -> Vec<C32> {
        audio.iter().map(|&a| self.modulate_sample(a)).collect()
    }
}

/// Quadrature (polar discriminator) FM demodulator:
/// `audio = arg(x[n] conj(x[n-1])) / (2 pi dev / fs)`, then optional de-emphasis.
/// Amplitude-independent; follow with an audio low-pass for best SNR.
#[derive(Clone, Debug)]
pub struct FmDemod {
    inv_k: f32,
    prev: C32,
    de: Option<OnePole>,
}

impl FmDemod {
    pub fn new(p: FmParams) -> FmDemod {
        FmDemod {
            inv_k: (p.fs as f64 / (TAU * p.deviation as f64)) as f32,
            prev: C32::ZERO,
            de: (p.tau > 0.0).then(|| OnePole::from_tau(p.tau, p.fs)),
        }
    }

    #[inline]
    pub fn demod_sample(&mut self, x: C32) -> f32 {
        let d = x.mul_conj(self.prev).arg() * self.inv_k;
        self.prev = x;
        match &mut self.de {
            Some(f) => f.process(d),
            None => d,
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
