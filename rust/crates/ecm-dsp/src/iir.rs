//! IIR filters: RBJ "Audio EQ Cookbook" biquads, cascades (incl. Butterworth),
//! DC blockers and single-pole (de-)emphasis.

use crate::complex::C32;
use std::f64::consts::PI;

/// Biquad coefficients, normalised so `a0 == 1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiquadCoeffs {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl BiquadCoeffs {
    fn norm(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> BiquadCoeffs {
        BiquadCoeffs {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
        }
    }

    fn common(fs: f32, f0: f32, q: f32) -> (f64, f64, f64) {
        let w0 = 2.0 * PI * f0 as f64 / fs as f64;
        let alpha = w0.sin() / (2.0 * q as f64);
        (w0, w0.cos(), alpha)
    }

    pub fn lowpass(fs: f32, f0: f32, q: f32) -> Self {
        let (_, c, a) = Self::common(fs, f0, q);
        Self::norm((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    pub fn highpass(fs: f32, f0: f32, q: f32) -> Self {
        let (_, c, a) = Self::common(fs, f0, q);
        Self::norm((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    /// Band-pass with 0 dB peak gain at `f0`.
    pub fn bandpass(fs: f32, f0: f32, q: f32) -> Self {
        let (_, c, a) = Self::common(fs, f0, q);
        Self::norm(a, 0.0, -a, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    pub fn notch(fs: f32, f0: f32, q: f32) -> Self {
        let (_, c, a) = Self::common(fs, f0, q);
        Self::norm(1.0, -2.0 * c, 1.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    pub fn allpass(fs: f32, f0: f32, q: f32) -> Self {
        let (_, c, a) = Self::common(fs, f0, q);
        Self::norm(1.0 - a, -2.0 * c, 1.0 + a, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    pub fn peaking(fs: f32, f0: f32, q: f32, gain_db: f32) -> Self {
        let (_, c, a) = Self::common(fs, f0, q);
        let ag = 10f64.powf(gain_db as f64 / 40.0);
        Self::norm(1.0 + a * ag, -2.0 * c, 1.0 - a * ag, 1.0 + a / ag, -2.0 * c, 1.0 - a / ag)
    }

    pub fn lowshelf(fs: f32, f0: f32, q: f32, gain_db: f32) -> Self {
        let (_, c, a) = Self::common(fs, f0, q);
        let ag = 10f64.powf(gain_db as f64 / 40.0);
        let s = 2.0 * ag.sqrt() * a;
        Self::norm(
            ag * ((ag + 1.0) - (ag - 1.0) * c + s),
            2.0 * ag * ((ag - 1.0) - (ag + 1.0) * c),
            ag * ((ag + 1.0) - (ag - 1.0) * c - s),
            (ag + 1.0) + (ag - 1.0) * c + s,
            -2.0 * ((ag - 1.0) + (ag + 1.0) * c),
            (ag + 1.0) + (ag - 1.0) * c - s,
        )
    }

    pub fn highshelf(fs: f32, f0: f32, q: f32, gain_db: f32) -> Self {
        let (_, c, a) = Self::common(fs, f0, q);
        let ag = 10f64.powf(gain_db as f64 / 40.0);
        let s = 2.0 * ag.sqrt() * a;
        Self::norm(
            ag * ((ag + 1.0) + (ag - 1.0) * c + s),
            -2.0 * ag * ((ag - 1.0) + (ag + 1.0) * c),
            ag * ((ag + 1.0) + (ag - 1.0) * c - s),
            (ag + 1.0) - (ag - 1.0) * c + s,
            2.0 * ((ag - 1.0) - (ag + 1.0) * c),
            (ag + 1.0) - (ag - 1.0) * c - s,
        )
    }

    /// Complex response at frequency `f` (Hz) for sample rate `fs`.
    pub fn response(&self, f: f32, fs: f32) -> C32 {
        let w = 2.0 * PI * f as f64 / fs as f64;
        let z1 = C32::expj64(-w);
        let z2 = C32::expj64(-2.0 * w);
        let num = C32::from(self.b0) + z1 * self.b1 + z2 * self.b2;
        let den = C32::ONE + z1 * self.a1 + z2 * self.a2;
        num / den
    }
}

/// A biquad section (transposed direct form II) with persistent state.
#[derive(Clone, Debug)]
pub struct Biquad {
    pub c: BiquadCoeffs,
    s1: f32,
    s2: f32,
}

impl Biquad {
    pub fn new(c: BiquadCoeffs) -> Biquad {
        Biquad { c, s1: 0.0, s2: 0.0 }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.c.b0 * x + self.s1;
        self.s1 = self.c.b1 * x - self.c.a1 * y + self.s2;
        self.s2 = self.c.b2 * x - self.c.a2 * y;
        y
    }

    pub fn process_block(&mut self, buf: &mut [f32]) {
        for v in buf {
            *v = self.process(*v);
        }
    }

    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

/// A chain of biquads.
#[derive(Clone, Debug, Default)]
pub struct BiquadCascade {
    pub stages: Vec<Biquad>,
}

impl BiquadCascade {
    pub fn new(coeffs: &[BiquadCoeffs]) -> BiquadCascade {
        BiquadCascade { stages: coeffs.iter().map(|&c| Biquad::new(c)).collect() }
    }

    /// Butterworth low-pass of even `order` (2, 4, 6, ...) as a biquad cascade.
    pub fn butterworth_lowpass(order: usize, fs: f32, fc: f32) -> BiquadCascade {
        Self::new(&butterworth_qs(order).map(|q| BiquadCoeffs::lowpass(fs, fc, q)).collect::<Vec<_>>())
    }

    /// Butterworth high-pass of even `order`.
    pub fn butterworth_highpass(order: usize, fs: f32, fc: f32) -> BiquadCascade {
        Self::new(&butterworth_qs(order).map(|q| BiquadCoeffs::highpass(fs, fc, q)).collect::<Vec<_>>())
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.stages.iter_mut().fold(x, |v, s| s.process(v))
    }

    pub fn process_block(&mut self, buf: &mut [f32]) {
        for v in buf {
            *v = self.process(*v);
        }
    }

    pub fn response(&self, f: f32, fs: f32) -> C32 {
        self.stages.iter().fold(C32::ONE, |a, s| a * s.c.response(f, fs))
    }

    pub fn reset(&mut self) {
        self.stages.iter_mut().for_each(Biquad::reset);
    }
}

fn butterworth_qs(order: usize) -> impl Iterator<Item = f32> {
    assert!(order >= 2 && order % 2 == 0, "Butterworth order must be even");
    (0..order / 2).map(move |k| {
        let theta = PI * (2 * k + 1) as f64 / (2 * order) as f64;
        (1.0 / (2.0 * theta.sin())) as f32
    })
}

/// A biquad cascade applied to complex samples (real coefficients, I and Q independently).
#[derive(Clone, Debug)]
pub struct ComplexIir {
    i: BiquadCascade,
    q: BiquadCascade,
}

impl ComplexIir {
    pub fn new(cascade: BiquadCascade) -> ComplexIir {
        ComplexIir { i: cascade.clone(), q: cascade }
    }

    #[inline]
    pub fn process(&mut self, x: C32) -> C32 {
        C32::new(self.i.process(x.re), self.q.process(x.im))
    }

    pub fn process_block(&mut self, buf: &mut [C32]) {
        for v in buf {
            *v = self.process(*v);
        }
    }
}

/// DC blocker `y[n] = x[n] - x[n-1] + r y[n-1]`, for real or complex samples.
#[derive(Clone, Debug)]
pub struct DcBlocker<S> {
    r: f32,
    x1: S,
    y1: S,
}

impl<S> DcBlocker<S>
where
    S: Copy + Default + core::ops::Sub<Output = S> + core::ops::Add<Output = S> + core::ops::Mul<f32, Output = S>,
{
    /// `r` close to 1 (e.g. 0.995) gives a narrow notch at DC.
    pub fn new(r: f32) -> Self {
        DcBlocker { r, x1: S::default(), y1: S::default() }
    }

    /// Choose `r` for a -3 dB corner of `fc` Hz at sample rate `fs`.
    pub fn with_corner(fc: f32, fs: f32) -> Self {
        Self::new((1.0 - 2.0 * std::f32::consts::PI * fc / fs).clamp(0.0, 0.99999))
    }

    #[inline]
    pub fn process(&mut self, x: S) -> S {
        let y = x - self.x1 + self.y1 * self.r;
        self.x1 = x;
        self.y1 = y;
        y
    }

    pub fn process_block(&mut self, buf: &mut [S]) {
        for v in buf {
            *v = self.process(*v);
        }
    }
}

/// Single-pole low-pass `y = (1-a) x + a y[n-1]`, e.g. FM de-emphasis with
/// time constant `tau` (75 us US / 50 us EU).
#[derive(Clone, Debug)]
pub struct OnePole {
    a: f32,
    y: f32,
}

impl OnePole {
    pub fn from_tau(tau: f32, fs: f32) -> OnePole {
        OnePole { a: (-1.0 / (fs * tau)).exp(), y: 0.0 }
    }

    pub fn from_alpha(a: f32) -> OnePole {
        OnePole { a, y: 0.0 }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.y = (1.0 - self.a) * x + self.a * self.y;
        self.y
    }
}

/// Exact inverse of [`OnePole`] (pre-emphasis): `y = (x - a x[n-1]) / (1 - a)`.
#[derive(Clone, Debug)]
pub struct PreEmphasis {
    a: f32,
    x1: f32,
}

impl PreEmphasis {
    pub fn from_tau(tau: f32, fs: f32) -> PreEmphasis {
        PreEmphasis { a: (-1.0 / (fs * tau)).exp(), x1: 0.0 }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = (x - self.a * self.x1) / (1.0 - self.a);
        self.x1 = x;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(c: C32) -> f32 {
        20.0 * c.abs().log10()
    }

    #[test]
    fn cookbook_responses() {
        let fs = 48_000.0;
        let lp = BiquadCoeffs::lowpass(fs, 1000.0, std::f32::consts::FRAC_1_SQRT_2);
        assert!(db(lp.response(10.0, fs)).abs() < 0.01);
        assert!((db(lp.response(1000.0, fs)) + 3.01).abs() < 0.05);
        assert!(db(lp.response(10_000.0, fs)) < -38.0);
        let hp = BiquadCoeffs::highpass(fs, 1000.0, std::f32::consts::FRAC_1_SQRT_2);
        assert!(db(hp.response(10.0, fs)) < -70.0);
        let bp = BiquadCoeffs::bandpass(fs, 2000.0, 5.0);
        assert!(db(bp.response(2000.0, fs)).abs() < 0.01);
        let n = BiquadCoeffs::notch(fs, 1000.0, 10.0);
        assert!(db(n.response(1000.0, fs)) < -50.0);
        assert!(db(n.response(3000.0, fs)).abs() < 0.1);
        let pk = BiquadCoeffs::peaking(fs, 3000.0, 1.0, 6.0);
        assert!((db(pk.response(3000.0, fs)) - 6.0).abs() < 0.01);
        let ap = BiquadCoeffs::allpass(fs, 3000.0, 1.0);
        assert!(db(ap.response(1234.0, fs)).abs() < 1e-3);
        let ls = BiquadCoeffs::lowshelf(fs, 200.0, 0.707, 10.0);
        assert!((db(ls.response(5.0, fs)) - 10.0).abs() < 0.1);
        let hs = BiquadCoeffs::highshelf(fs, 5000.0, 0.707, -8.0);
        assert!((db(hs.response(23_000.0, fs)) + 8.0).abs() < 0.2);
    }

    #[test]
    fn butterworth_cascade_and_time_domain() {
        let fs = 8000.0;
        let mut c = BiquadCascade::butterworth_lowpass(4, fs, 500.0);
        assert!((db(c.response(500.0, fs)) + 3.01).abs() < 0.05);
        // 4th order: -24 dB/octave far above corner -> 2 kHz is two octaves: about -48 dB
        assert!(db(c.response(2000.0, fs)) < -45.0);
        // A 100 Hz tone passes with unit amplitude after settling.
        let mut peak: f32 = 0.0;
        for i in 0..8000 {
            let y = c.process((2.0 * std::f32::consts::PI * 100.0 * i as f32 / fs).sin());
            if i > 4000 {
                peak = peak.max(y.abs());
            }
        }
        assert!((peak - 1.0).abs() < 0.01, "peak {peak}");
    }

    #[test]
    fn dc_blocker_and_emphasis() {
        let mut d: DcBlocker<f32> = DcBlocker::new(0.995);
        let mut last = 0.0;
        for _ in 0..5000 {
            last = d.process(1.0);
        }
        assert!(last.abs() < 1e-3);
        let mut dc: DcBlocker<C32> = DcBlocker::new(0.99);
        let mut lc = C32::ZERO;
        for _ in 0..3000 {
            lc = dc.process(C32::new(0.5, -0.3));
        }
        assert!(lc.abs() < 1e-3);
        let mut pre = PreEmphasis::from_tau(75e-6, 48_000.0);
        let mut de = OnePole::from_tau(75e-6, 48_000.0);
        for i in 0..100 {
            let x = (i as f32 * 0.37).sin();
            let y = de.process(pre.process(x));
            assert!((x - y).abs() < 1e-4);
        }
    }
}
