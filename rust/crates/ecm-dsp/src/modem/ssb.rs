//! Single sideband (USB/LSB), phasing method using a Hilbert transformer, and
//! a Weaver-method demodulator for signals offset in the passband.

use crate::complex::C32;
use crate::fir::{lowpass, FirRC};
use crate::nco::{Hilbert, Nco};
use crate::window::Window;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sideband {
    Upper,
    Lower,
}

/// SSB modulator: audio -> `a + j H{a}` (USB) or `a - j H{a}` (LSB) at complex
/// baseband (suppressed carrier at 0 Hz). Delay = `(taps-1)/2` samples.
#[derive(Clone, Debug)]
pub struct SsbMod {
    sb: Sideband,
    h: Hilbert,
}

impl SsbMod {
    /// `taps`: Hilbert length (odd, e.g. 127 at 8–48 kHz).
    pub fn new(sb: Sideband, taps: usize) -> SsbMod {
        SsbMod { sb, h: Hilbert::new(taps) }
    }

    #[inline]
    pub fn modulate_sample(&mut self, a: f32) -> C32 {
        let (d, h) = self.h.step(a);
        match self.sb {
            Sideband::Upper => C32::new(d, h),
            Sideband::Lower => C32::new(d, -h),
        }
    }

    pub fn modulate_vec(&mut self, audio: &[f32]) -> Vec<C32> {
        audio.iter().map(|&a| self.modulate_sample(a)).collect()
    }
}

/// Phasing SSB demodulator: `audio = (Re{x}[n-D] -/+ H{Im{x}}[n]) / 2`, which keeps
/// the chosen sideband and cancels the other one.
#[derive(Clone, Debug)]
pub struct SsbDemod {
    sb: Sideband,
    hi: Hilbert,
    hq: Hilbert,
}

impl SsbDemod {
    pub fn new(sb: Sideband, taps: usize) -> SsbDemod {
        SsbDemod { sb, hi: Hilbert::new(taps), hq: Hilbert::new(taps) }
    }

    #[inline]
    pub fn demod_sample(&mut self, x: C32) -> f32 {
        let (re_d, _) = self.hi.step(x.re);
        let (_, h_im) = self.hq.step(x.im);
        match self.sb {
            Sideband::Upper => 0.5 * (re_d - h_im),
            Sideband::Lower => 0.5 * (re_d + h_im),
        }
    }

    pub fn demod_vec(&mut self, input: &[C32]) -> Vec<f32> {
        input.iter().map(|&x| self.demod_sample(x)).collect()
    }
}

/// Weaver SSB demodulator: shift the centre of the audio passband
/// (`low..high` Hz above/below the carrier) to 0 Hz, low-pass to half the
/// bandwidth, then shift back up by the passband centre and take the real part.
/// Rejects the opposite sideband by the low-pass stopband (> 60 dB).
#[derive(Clone, Debug)]
pub struct WeaverDemod {
    sb: Sideband,
    nco1: Nco,
    lpf: FirRC,
    nco2: Nco,
}

impl WeaverDemod {
    /// e.g. `low = 300`, `high = 2700` for voice.
    pub fn new(sb: Sideband, fs: f32, low: f32, high: f32) -> WeaverDemod {
        let centre = (low + high) / 2.0;
        let half_bw = (high - low) / 2.0;
        let shift = match sb {
            Sideband::Upper => centre,
            Sideband::Lower => -centre,
        };
        let taps = crate::fir::kaiser_order(65.0, (low.min(half_bw * 0.5)) / fs).min(1023);
        WeaverDemod {
            sb,
            nco1: Nco::new(shift as f64, fs as f64),
            lpf: FirRC::new(lowpass(taps, half_bw / fs, Window::Kaiser(crate::window::kaiser_beta(65.0)))),
            nco2: Nco::new(centre as f64, fs as f64),
        }
    }

    #[inline]
    pub fn demod_sample(&mut self, x: C32) -> f32 {
        let v = self.lpf.filter(x.mul_conj(self.nco1.next()));
        let v = match self.sb {
            Sideband::Upper => v,
            Sideband::Lower => v.conj(),
        };
        (v * self.nco2.next()).re
    }

    pub fn demod_vec(&mut self, input: &[C32]) -> Vec<f32> {
        input.iter().map(|&x| self.demod_sample(x)).collect()
    }
}
