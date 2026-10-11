//! FIR filter design (windowed sinc, Kaiser, root-raised-cosine) and a
//! streaming FIR filter for real or complex samples and taps.
//!
//! Frequencies in design functions are **normalised to the sample rate**
//! (`0.0 ..= 0.5`, where 0.5 is Nyquist).

use crate::complex::C32;
use crate::window::{kaiser_beta, Window};
use std::f64::consts::PI;

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

/// Kaiser's estimate of the number of taps (odd) for stopband attenuation
/// `atten_db` and transition width `transition` (normalised to the sample rate).
pub fn kaiser_order(atten_db: f32, transition: f32) -> usize {
    let n = ((atten_db as f64 - 7.95) / (14.36 * transition as f64)).ceil().max(1.0) as usize + 1;
    n | 1
}

/// Windowed-sinc low-pass, unity gain at DC. `cutoff` is the -6 dB point.
pub fn lowpass(num_taps: usize, cutoff: f32, window: Window) -> Vec<f32> {
    assert!(num_taps >= 1);
    let w = window.symmetric(num_taps);
    let m = (num_taps - 1) as f64 / 2.0;
    let fc = cutoff as f64;
    let mut h: Vec<f64> = (0..num_taps)
        .map(|i| 2.0 * fc * sinc(2.0 * fc * (i as f64 - m)) * w[i] as f64)
        .collect();
    let s: f64 = h.iter().sum();
    for v in &mut h {
        *v /= s;
    }
    h.into_iter().map(|v| v as f32).collect()
}

/// Windowed-sinc high-pass by spectral inversion. `num_taps` is forced odd.
pub fn highpass(num_taps: usize, cutoff: f32, window: Window) -> Vec<f32> {
    let n = num_taps | 1;
    let mut h = lowpass(n, cutoff, window);
    for v in &mut h {
        *v = -*v;
    }
    h[n / 2] += 1.0;
    h
}

/// Windowed-sinc band-pass between `low` and `high`, unity gain at the centre.
pub fn bandpass(num_taps: usize, low: f32, high: f32, window: Window) -> Vec<f32> {
    assert!(low < high);
    let w = window.symmetric(num_taps);
    let m = (num_taps - 1) as f64 / 2.0;
    let (f1, f2) = (low as f64, high as f64);
    let mut h: Vec<f64> = (0..num_taps)
        .map(|i| {
            let t = i as f64 - m;
            (2.0 * f2 * sinc(2.0 * f2 * t) - 2.0 * f1 * sinc(2.0 * f1 * t)) * w[i] as f64
        })
        .collect();
    let fc = (f1 + f2) / 2.0;
    let g = response_f64(&h, fc).0.hypot(response_f64(&h, fc).1);
    for v in &mut h {
        *v /= g;
    }
    h.into_iter().map(|v| v as f32).collect()
}

/// Kaiser-window low-pass meeting `atten_db` with the given transition width.
pub fn lowpass_kaiser(cutoff: f32, transition: f32, atten_db: f32) -> Vec<f32> {
    lowpass(kaiser_order(atten_db, transition), cutoff, Window::Kaiser(kaiser_beta(atten_db)))
}

/// Complex band-pass: a real low-pass of half-width `bw/2` shifted to `center`
/// (normalised, may be negative). Passes only `center - bw/2 .. center + bw/2`.
pub fn complex_bandpass(num_taps: usize, center: f32, bw: f32, window: Window) -> Vec<C32> {
    let lp = lowpass(num_taps, bw / 2.0, window);
    let m = (num_taps - 1) as f64 / 2.0;
    lp.iter()
        .enumerate()
        .map(|(i, &v)| C32::expj64(2.0 * PI * center as f64 * (i as f64 - m)).scale(v))
        .collect()
}

/// Root-raised-cosine pulse, `sps` samples per symbol, `span` symbols long,
/// roll-off `beta`. Normalised to unit energy (`sum h^2 = 1`).
pub fn rrc(sps: usize, span: usize, beta: f32) -> Vec<f32> {
    let n = sps * span + 1;
    let m = (n - 1) as f64 / 2.0;
    let b = beta as f64;
    let mut h: Vec<f64> = (0..n)
        .map(|i| {
            let t = (i as f64 - m) / sps as f64;
            if t.abs() < 1e-9 {
                1.0 - b + 4.0 * b / PI
            } else if b > 0.0 && (t.abs() - 1.0 / (4.0 * b)).abs() < 1e-9 {
                b / 2f64.sqrt()
                    * ((1.0 + 2.0 / PI) * (PI / (4.0 * b)).sin()
                        + (1.0 - 2.0 / PI) * (PI / (4.0 * b)).cos())
            } else {
                ((PI * t * (1.0 - b)).sin() + 4.0 * b * t * (PI * t * (1.0 + b)).cos())
                    / (PI * t * (1.0 - (4.0 * b * t).powi(2)))
            }
        })
        .collect();
    let e: f64 = h.iter().map(|v| v * v).sum::<f64>().sqrt();
    for v in &mut h {
        *v /= e;
    }
    h.into_iter().map(|v| v as f32).collect()
}

fn response_f64(h: &[f64], f: f64) -> (f64, f64) {
    let (mut re, mut im) = (0.0, 0.0);
    for (i, &v) in h.iter().enumerate() {
        let ph = -2.0 * PI * f * i as f64;
        re += v * ph.cos();
        im += v * ph.sin();
    }
    (re, im)
}

/// Frequency response of real taps at normalised frequency `f`.
pub fn freq_response(h: &[f32], f: f32) -> C32 {
    let hd: Vec<f64> = h.iter().map(|&v| v as f64).collect();
    let (re, im) = response_f64(&hd, f as f64);
    C32::new(re as f32, im as f32)
}

/// Frequency response of complex taps at normalised frequency `f`.
pub fn freq_response_complex(h: &[C32], f: f32) -> C32 {
    h.iter()
        .enumerate()
        .map(|(i, &v)| v * C32::expj64(-2.0 * PI * f as f64 * i as f64))
        .sum()
}

/// Gain in dB of real taps at normalised frequency `f`.
pub fn gain_db(h: &[f32], f: f32) -> f32 {
    20.0 * freq_response(h, f).abs().max(1e-15).log10()
}

/// Multiply-accumulate between a tap type and a sample type.
pub trait Tap<S>: Copy {
    fn mac(acc: S, tap: Self, x: S) -> S;
}
impl Tap<f32> for f32 {
    #[inline]
    fn mac(acc: f32, t: f32, x: f32) -> f32 {
        acc + t * x
    }
}
impl Tap<C32> for f32 {
    #[inline]
    fn mac(acc: C32, t: f32, x: C32) -> C32 {
        C32::new(acc.re + t * x.re, acc.im + t * x.im)
    }
}
impl Tap<C32> for C32 {
    #[inline]
    fn mac(acc: C32, t: C32, x: C32) -> C32 {
        acc + t * x
    }
}

/// Streaming FIR filter. `T` is the tap type, `S` the sample type.
/// State (the delay line) persists across calls.
#[derive(Clone, Debug)]
pub struct Fir<T, S> {
    taps: Vec<T>,
    hist: Vec<S>,
    pos: usize,
}

/// Real taps, real samples.
pub type FirRR = Fir<f32, f32>;
/// Real taps, complex samples.
pub type FirRC = Fir<f32, C32>;
/// Complex taps, complex samples.
pub type FirCC = Fir<C32, C32>;

impl<T: Tap<S>, S: Copy + Default> Fir<T, S> {
    pub fn new(taps: Vec<T>) -> Self {
        assert!(!taps.is_empty());
        let n = taps.len();
        Fir { taps, hist: vec![S::default(); 2 * n], pos: 0 }
    }

    pub fn taps(&self) -> &[T] {
        &self.taps
    }

    pub fn len(&self) -> usize {
        self.taps.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Group delay in samples for a linear-phase design.
    pub fn delay(&self) -> usize {
        (self.taps.len() - 1) / 2
    }

    pub fn reset(&mut self) {
        self.hist.iter_mut().for_each(|v| *v = S::default());
        self.pos = 0;
    }

    /// Shift one sample into the delay line without computing an output.
    #[inline]
    pub fn push(&mut self, x: S) {
        let n = self.taps.len();
        self.pos = if self.pos == 0 { n - 1 } else { self.pos - 1 };
        self.hist[self.pos] = x;
        self.hist[self.pos + n] = x;
    }

    /// Output for the current delay-line contents.
    #[inline]
    pub fn output(&self) -> S {
        let n = self.taps.len();
        let win = &self.hist[self.pos..self.pos + n];
        let mut acc = S::default();
        for (t, x) in self.taps.iter().zip(win) {
            acc = T::mac(acc, *t, *x);
        }
        acc
    }

    /// Push one sample and return one output.
    #[inline]
    pub fn filter(&mut self, x: S) -> S {
        self.push(x);
        self.output()
    }

    /// Filter a block; `output.len()` must equal `input.len()`.
    pub fn process(&mut self, input: &[S], output: &mut [S]) {
        assert_eq!(input.len(), output.len());
        for (x, y) in input.iter().zip(output.iter_mut()) {
            *y = self.filter(*x);
        }
    }

    /// Filter a block in place.
    pub fn process_inplace(&mut self, buf: &mut [S]) {
        for v in buf.iter_mut() {
            *v = self.filter(*v);
        }
    }

    /// Filter a block into a new vector.
    pub fn process_vec(&mut self, input: &[S]) -> Vec<S> {
        input.iter().map(|&x| self.filter(x)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowpass_response() {
        let h = lowpass_kaiser(0.1, 0.05, 60.0);
        assert!(gain_db(&h, 0.0).abs() < 0.05);
        assert!(gain_db(&h, 0.05).abs() < 0.1, "passband {}", gain_db(&h, 0.05));
        for f in [0.13, 0.2, 0.3, 0.45] {
            assert!(gain_db(&h, f) < -55.0, "stopband at {f}: {}", gain_db(&h, f));
        }
    }

    #[test]
    fn highpass_and_bandpass_response() {
        let hp = highpass(101, 0.2, Window::Blackman);
        assert!(gain_db(&hp, 0.0) < -60.0);
        assert!(gain_db(&hp, 0.05) < -60.0);
        assert!(gain_db(&hp, 0.4).abs() < 0.1);
        let bp = bandpass(129, 0.1, 0.2, Window::Hamming);
        assert!(gain_db(&bp, 0.15).abs() < 0.1);
        assert!(gain_db(&bp, 0.0) < -45.0);
        assert!(gain_db(&bp, 0.35) < -45.0);
    }

    #[test]
    fn complex_bandpass_is_one_sided() {
        let h = complex_bandpass(127, 0.2, 0.1, Window::Blackman);
        assert!(20.0 * freq_response_complex(&h, 0.2).abs().log10() > -0.1);
        assert!(20.0 * freq_response_complex(&h, -0.2).abs().log10() < -60.0);
    }

    #[test]
    fn rrc_squared_is_nyquist() {
        let sps = 8;
        let h = rrc(sps, 10, 0.35);
        // Convolve with itself: raised cosine must be ~0 at nonzero symbol offsets.
        let n = h.len();
        let mut rc = vec![0.0f32; 2 * n - 1];
        for i in 0..n {
            for j in 0..n {
                rc[i + j] += h[i] * h[j];
            }
        }
        let c = n - 1;
        assert!((rc[c] - 1.0).abs() < 1e-4);
        for k in 1..8 {
            assert!(rc[c + k * sps].abs() < 0.01, "isi at {k}: {}", rc[c + k * sps]);
        }
    }

    #[test]
    fn streaming_matches_block() {
        let h = lowpass(31, 0.2, Window::Hann);
        let x: Vec<f32> = (0..200).map(|i| ((i * 7919) % 101) as f32 / 50.0 - 1.0).collect();
        let mut a = FirRR::new(h.clone());
        let full = a.process_vec(&x);
        let mut b = FirRR::new(h.clone());
        let mut parts = b.process_vec(&x[..77]);
        parts.extend(b.process_vec(&x[77..]));
        assert_eq!(full, parts);
        // direct convolution check
        for n in 0..x.len() {
            let mut s = 0.0;
            for k in 0..h.len() {
                if n >= k {
                    s += h[k] * x[n - k];
                }
            }
            assert!((s - full[n]).abs() < 1e-5);
        }
    }

    #[test]
    fn kaiser_order_estimate() {
        // A=60 dB, df=0.05 -> (60-7.95)/(14.36*0.05) = 72.5 -> 73+1 -> odd 75
        assert_eq!(kaiser_order(60.0, 0.05), 75);
    }
}
