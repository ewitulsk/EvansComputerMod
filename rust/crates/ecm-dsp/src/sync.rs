//! Synchronisation blocks: AGC, squelch, PLL, Costas loop, symbol timing
//! recovery (Gardner / Mueller–Müller) and coarse frequency-offset estimation.

use crate::complex::C32;
use crate::fft::Fft;
use std::f64::consts::{PI, TAU};

/// Proportional/integral gains of a second-order loop with normalised noise
/// bandwidth `bw` (radians/sample, typical 0.005..0.05) and damping `zeta`
/// (0.707 is a good default). Returns `(alpha, beta)`.
pub fn loop_gains(bw: f32, zeta: f32) -> (f32, f32) {
    let (b, z) = (bw as f64, zeta as f64);
    let d = 1.0 + 2.0 * z * b + b * b;
    ((4.0 * z * b / d) as f32, (4.0 * b * b / d) as f32)
}

/// Automatic gain control with separate attack and decay rates.
///
/// Tracks the input envelope (`|x|`) with a fast `attack` rate when the level
/// rises and a slow `decay` rate when it falls, and scales the output so that
/// the envelope sits at `target`. Gain is clamped to `max_gain`.
#[derive(Clone, Debug)]
pub struct Agc {
    pub target: f32,
    pub attack: f32,
    pub decay: f32,
    pub max_gain: f32,
    env: f32,
}

impl Agc {
    /// `attack` / `decay` are per-sample smoothing factors in `(0, 1]`
    /// (e.g. 0.01 and 0.0005).
    pub fn new(target: f32, attack: f32, decay: f32) -> Agc {
        Agc { target, attack, decay, max_gain: 1e6, env: 0.0 }
    }

    /// Current gain.
    pub fn gain(&self) -> f32 {
        if self.env <= 0.0 {
            self.max_gain
        } else {
            (self.target / self.env).min(self.max_gain)
        }
    }

    #[inline]
    fn track(&mut self, mag: f32) -> f32 {
        let k = if mag > self.env { self.attack } else { self.decay };
        self.env += k * (mag - self.env);
        self.gain()
    }

    #[inline]
    pub fn process(&mut self, x: C32) -> C32 {
        let g = self.track(x.abs());
        x.scale(g)
    }

    #[inline]
    pub fn process_real(&mut self, x: f32) -> f32 {
        let g = self.track(x.abs());
        x * g
    }

    pub fn process_block(&mut self, buf: &mut [C32]) {
        for v in buf {
            *v = self.process(*v);
        }
    }

    pub fn process_block_real(&mut self, buf: &mut [f32]) {
        for v in buf {
            *v = self.process_real(*v);
        }
    }
}

/// Power squelch with hysteresis: passes samples while the smoothed power is
/// above `threshold_db` (dBFS of `|x|^2`), zeroes them otherwise.
#[derive(Clone, Debug)]
pub struct Squelch {
    open_db: f32,
    close_db: f32,
    alpha: f32,
    power: f32,
    open: bool,
}

impl Squelch {
    /// `alpha` is the power smoother (e.g. 0.01), `hysteresis_db` the gap
    /// between opening and closing (e.g. 3 dB).
    pub fn new(threshold_db: f32, hysteresis_db: f32, alpha: f32) -> Squelch {
        Squelch {
            open_db: threshold_db,
            close_db: threshold_db - hysteresis_db,
            alpha,
            power: 0.0,
            open: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Smoothed power in dB.
    pub fn power_db(&self) -> f32 {
        crate::complex::to_db(self.power)
    }

    #[inline]
    pub fn process(&mut self, x: C32) -> C32 {
        self.power += self.alpha * (x.norm_sqr() - self.power);
        let db = self.power_db();
        if self.open && db < self.close_db {
            self.open = false;
        } else if !self.open && db >= self.open_db {
            self.open = true;
        }
        if self.open {
            x
        } else {
            C32::ZERO
        }
    }

    pub fn process_block(&mut self, buf: &mut [C32]) {
        for v in buf {
            *v = self.process(*v);
        }
    }
}

/// Carrier-tracking phase-locked loop. Locks an internal NCO onto the phase of
/// a (complex) carrier.
#[derive(Clone, Debug)]
pub struct Pll {
    phase: f64,
    freq: f64,
    alpha: f64,
    beta: f64,
    max_freq: f64,
    lock: f32,
}

impl Pll {
    /// `bw` loop bandwidth (rad/sample), `max_freq_hz` pull-in limit.
    pub fn new(bw: f32, max_freq_hz: f64, fs: f64) -> Pll {
        let (a, b) = loop_gains(bw, 0.707);
        Pll { phase: 0.0, freq: 0.0, alpha: a as f64, beta: b as f64, max_freq: TAU * max_freq_hz / fs, lock: 0.0 }
    }

    /// Frequency estimate in Hz.
    pub fn freq_hz(&self, fs: f64) -> f64 {
        self.freq * fs / TAU
    }

    pub fn phase(&self) -> f64 {
        self.phase
    }

    /// Lock indicator in `[0, 1]` (smoothed cos of the phase error).
    pub fn lock(&self) -> f32 {
        self.lock
    }

    /// Track one sample. Returns the input derotated by the NCO (`x e^{-j phase}`).
    #[inline]
    pub fn process(&mut self, x: C32) -> C32 {
        let y = x.mul_conj(C32::expj64(self.phase));
        let err = y.arg() as f64;
        self.lock += 0.001 * ((err.cos() as f32) - self.lock);
        self.advance(err);
        y
    }

    #[inline]
    fn advance(&mut self, err: f64) {
        self.freq = (self.freq + self.beta * err).clamp(-self.max_freq, self.max_freq);
        self.phase += self.freq + self.alpha * err;
        if !(-PI..PI).contains(&self.phase) {
            self.phase = (self.phase + PI).rem_euclid(TAU) - PI;
        }
    }

    /// The NCO's current phasor (the regenerated carrier).
    pub fn carrier(&self) -> C32 {
        C32::expj64(self.phase)
    }
}

/// Costas loop for BPSK (`order = 2`) or QPSK (`order = 4`) carrier recovery.
/// Runs at one sample per symbol (after timing recovery).
#[derive(Clone, Debug)]
pub struct Costas {
    order: u8,
    phase: f64,
    freq: f64,
    alpha: f64,
    beta: f64,
    power: f32,
}

impl Costas {
    pub fn new(order: u8, bw: f32) -> Costas {
        assert!(order == 2 || order == 4, "Costas order must be 2 or 4");
        let (a, b) = loop_gains(bw, 0.707);
        Costas { order, phase: 0.0, freq: 0.0, alpha: a as f64, beta: b as f64, power: 1.0 }
    }

    /// Frequency estimate in radians/symbol.
    pub fn freq(&self) -> f64 {
        self.freq
    }

    #[inline]
    pub fn process(&mut self, x: C32) -> C32 {
        let y = x.mul_conj(C32::expj64(self.phase));
        self.power += 0.01 * (y.norm_sqr() - self.power);
        let p = self.power.max(1e-12);
        let err = match self.order {
            2 => y.re * y.im / p,
            _ => {
                let s = |v: f32| if v >= 0.0 { 1.0 } else { -1.0 };
                (s(y.re) * y.im - s(y.im) * y.re) / p.sqrt()
            }
        };
        let err = (err as f64).clamp(-1.0, 1.0);
        self.freq = (self.freq + self.beta * err).clamp(-1.0, 1.0);
        self.phase += self.freq + self.alpha * err;
        if !(-PI..PI).contains(&self.phase) {
            self.phase = (self.phase + PI).rem_euclid(TAU) - PI;
        }
        y
    }

    pub fn process_block(&mut self, buf: &mut [C32]) {
        for v in buf {
            *v = self.process(*v);
        }
    }
}

/// Timing-error detector used by [`SymbolSync`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ted {
    /// Gardner: needs 2+ samples/symbol, data-aided-free, carrier-phase independent.
    Gardner,
    /// Mueller–Müller: decision-directed, one interpolant per symbol.
    MuellerMuller,
}

/// Decision rule for the decision-directed parts (Mueller–Müller).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Bpsk,
    Qpsk,
}

/// Symbol timing recovery with cubic (Lagrange) interpolation.
///
/// Feed matched-filtered samples at `sps` samples/symbol (fractional allowed);
/// outputs one interpolated sample per symbol at the estimated optimum
/// sampling instant.
#[derive(Clone, Debug)]
pub struct SymbolSync {
    ted: Ted,
    decision: Decision,
    sps: f64,
    period: f64,
    max_dev: f64,
    alpha: f64,
    beta: f64,
    buf: Vec<C32>,
    next: f64,
    prev: C32,
    prev_dec: C32,
    power: f32,
}

impl SymbolSync {
    /// `bw` loop bandwidth (rad/symbol, e.g. 0.01), `max_dev` maximum relative
    /// clock deviation (e.g. 0.01 = 1%).
    pub fn new(ted: Ted, decision: Decision, sps: f32, bw: f32, max_dev: f32) -> SymbolSync {
        let (a, b) = loop_gains(bw, 1.0);
        SymbolSync {
            ted,
            decision,
            sps: sps as f64,
            period: sps as f64,
            max_dev: max_dev as f64 * sps as f64,
            alpha: a as f64,
            beta: b as f64,
            buf: Vec::new(),
            next: 1.0 + sps as f64,
            prev: C32::ZERO,
            prev_dec: C32::ZERO,
            power: 1.0,
        }
    }

    /// Current estimate of samples per symbol.
    pub fn period(&self) -> f64 {
        self.period
    }

    fn interp(&self, t: f64) -> C32 {
        // 4-point Lagrange around floor(t): uses buf[i-1..=i+2]
        let i = t.floor() as usize;
        let mu = (t - i as f64) as f32;
        let (y0, y1, y2, y3) = (self.buf[i - 1], self.buf[i], self.buf[i + 1], self.buf[i + 2]);
        let c0 = -mu * (mu - 1.0) * (mu - 2.0) / 6.0;
        let c1 = (mu + 1.0) * (mu - 1.0) * (mu - 2.0) / 2.0;
        let c2 = -(mu + 1.0) * mu * (mu - 2.0) / 2.0;
        let c3 = (mu + 1.0) * mu * (mu - 1.0) / 6.0;
        y0 * c0 + y1 * c1 + y2 * c2 + y3 * c3
    }

    fn decide(&self, y: C32) -> C32 {
        let s = |v: f32| if v >= 0.0 { 1.0 } else { -1.0 };
        let a = self.power.max(1e-12).sqrt();
        match self.decision {
            Decision::Bpsk => C32::new(s(y.re) * a, 0.0),
            Decision::Qpsk => C32::new(s(y.re), s(y.im)).scale(a * std::f32::consts::FRAC_1_SQRT_2),
        }
    }

    /// Process a block of samples, appending symbol-rate outputs to `out`.
    pub fn process(&mut self, input: &[C32], out: &mut Vec<C32>) {
        self.buf.extend_from_slice(input);
        let half = self.sps / 2.0;
        while self.next + 3.0 < self.buf.len() as f64 {
            if self.next - half.max(self.period / 2.0) < 1.0 {
                self.next += self.period;
                continue;
            }
            let y = self.interp(self.next);
            self.power += 0.02 * (y.norm_sqr() - self.power);
            let p = self.power.max(1e-12);
            let err = match self.ted {
                Ted::Gardner => {
                    let mid = self.interp(self.next - self.period / 2.0);
                    ((self.prev - y).mul_conj(mid)).re / p
                }
                Ted::MuellerMuller => {
                    let d = self.decide(y);
                    (y.mul_conj(self.prev_dec).re - self.prev.mul_conj(d).re) / p
                }
            };
            let err = (err as f64).clamp(-1.0, 1.0);
            self.prev_dec = self.decide(y);
            self.prev = y;
            out.push(y);
            let dev = (self.period - self.sps + self.beta * err * self.sps).clamp(-self.max_dev, self.max_dev);
            self.period = self.sps + dev;
            self.next += self.period + self.alpha * err * self.sps;
        }
        // Drop consumed samples, keep enough history for interpolation / midpoints.
        let keep_from = (self.next - self.period - 3.0).floor().max(0.0) as usize;
        if keep_from > 0 {
            self.buf.drain(..keep_from);
            self.next -= keep_from as f64;
        }
    }

    pub fn process_vec(&mut self, input: &[C32]) -> Vec<C32> {
        let mut out = Vec::new();
        self.process(input, &mut out);
        out
    }
}

/// Coarse frequency offset (Hz) from the FFT peak of `x^power` (use `power = 1`
/// for a carrier, 2 for BPSK, 4 for QPSK). Uses parabolic peak interpolation.
pub fn fft_peak_offset(x: &[C32], fs: f32, power: u32) -> f32 {
    let n = x.len().next_power_of_two().max(2);
    let mut buf = vec![C32::ZERO; n];
    for (d, s) in buf.iter_mut().zip(x) {
        let mut v = C32::ONE;
        for _ in 0..power {
            v *= *s;
        }
        *d = v;
    }
    Fft::new(n).forward(&mut buf);
    let (k, _) = buf
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.norm_sqr().total_cmp(&b.1.norm_sqr()))
        .unwrap();
    let m = |i: usize| buf[i % n].abs();
    let (a, b, c) = (m(k + n - 1), m(k), m(k + 1));
    let den = a - 2.0 * b + c;
    let d = if den.abs() > 1e-20 { 0.5 * (a - c) / den } else { 0.0 };
    let f = crate::fft::bin_freq(k, n) + d / n as f32;
    f * fs / power as f32
}

/// Frequency offset (Hz) by delay-and-multiply: `angle(sum x[n] conj(x[n-lag])) / (2 pi lag)`.
/// Unambiguous for `|f| < fs / (2 lag)`.
pub fn delay_multiply_offset(x: &[C32], fs: f32, lag: usize) -> f32 {
    let mut acc = C32::ZERO;
    for n in lag..x.len() {
        acc += x[n].mul_conj(x[n - lag]);
    }
    (acc.arg() as f64 / (TAU * lag as f64) * fs as f64) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::{awgn, Rng};

    #[test]
    fn agc_tracks_level_changes() {
        let mut agc = Agc::new(1.0, 0.05, 0.005);
        let mut last = 0.0;
        for i in 0..20_000 {
            let amp = if i < 10_000 { 0.01 } else { 5.0 };
            last = agc.process(C32::expj(i as f32 * 0.1).scale(amp)).abs();
            if i == 9_999 {
                assert!((last - 1.0).abs() < 0.01);
            }
        }
        assert!((last - 1.0).abs() < 0.01);
    }

    #[test]
    fn squelch_opens_on_signal_only() {
        let mut rng = Rng::new(3);
        let mut sq = Squelch::new(-20.0, 3.0, 0.01);
        let mut noise = vec![C32::ZERO; 5000];
        awgn(&mut noise, 1e-4, &mut rng);
        sq.process_block(&mut noise);
        assert!(!sq.is_open());
        let mut sig: Vec<C32> = (0..2000).map(|i| C32::expj(i as f32 * 0.2).scale(0.5)).collect();
        sq.process_block(&mut sig);
        assert!(sq.is_open());
        assert!(sig[1999].abs() > 0.4);
    }

    #[test]
    fn pll_locks_to_offset_carrier() {
        let fs = 48_000.0;
        let mut pll = Pll::new(0.02, 2000.0, fs);
        let f = 330.0;
        let mut rng = Rng::new(7);
        let mut x: Vec<C32> = (0..20_000)
            .map(|i| C32::expj64(TAU * f * i as f64 / fs + 1.0))
            .collect();
        awgn(&mut x, 0.01, &mut rng);
        let mut last = C32::ZERO;
        for v in &x {
            last = pll.process(*v);
        }
        assert!((pll.freq_hz(fs) - f).abs() < 5.0, "freq {}", pll.freq_hz(fs));
        assert!(last.arg().abs() < 0.3);
        assert!(pll.lock() > 0.95);
    }

    #[test]
    fn costas_removes_phase_and_frequency_offset() {
        let mut rng = Rng::new(11);
        for order in [2u8, 4] {
            let mut c = Costas::new(order, 0.02);
            let mut errs = 0;
            for i in 0..6000 {
                let sym = if order == 2 {
                    C32::new(if rng.bit() == 1 { 1.0 } else { -1.0 }, 0.0)
                } else {
                    C32::new(if rng.bit() == 1 { 1.0 } else { -1.0 }, if rng.bit() == 1 { 1.0 } else { -1.0 })
                        .scale(std::f32::consts::FRAC_1_SQRT_2)
                };
                let rx = sym * C32::expj64(0.7 + 0.002 * i as f64);
                let y = c.process(rx);
                if i > 3000 {
                    // up to the inherent phase ambiguity, y should lie on the constellation
                    let ang = y.arg() as f64;
                    let q = if order == 2 { PI } else { PI / 2.0 };
                    let off = if order == 2 { 0.0 } else { PI / 4.0 };
                    let r = ((ang - off) / q).round() * q + off;
                    if (ang - r).abs() > 0.1 {
                        errs += 1;
                    }
                }
            }
            assert_eq!(errs, 0, "order {order}");
        }
    }

    #[test]
    fn frequency_offset_estimators() {
        let fs = 10_000.0f32;
        let mut rng = Rng::new(5);
        let f = 123.4f32;
        let mut x: Vec<C32> = (0..4096)
            .map(|i| {
                let b = if rng.bit() == 1 { 1.0 } else { -1.0 };
                C32::expj64(TAU * f as f64 * i as f64 / fs as f64).scale(b)
            })
            .collect();
        awgn(&mut x, 0.1, &mut rng);
        let est = fft_peak_offset(&x, fs, 2);
        assert!((est - f).abs() < 2.0, "fft {est}");
        let tone: Vec<C32> = (0..4096).map(|i| C32::expj64(TAU * f as f64 * i as f64 / fs as f64)).collect();
        let est2 = delay_multiply_offset(&tone, fs, 4);
        assert!((est2 - f).abs() < 0.5, "dm {est2}");
    }
}
