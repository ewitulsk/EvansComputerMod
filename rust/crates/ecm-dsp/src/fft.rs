//! Fast Fourier transforms for any length.
//!
//! Powers of two use an iterative radix-2 decimation-in-time kernel with
//! precomputed twiddles. Every other length uses Bluestein's chirp-z algorithm
//! on top of a power-of-two plan, so any `N >= 1` works in `O(N log N)`.
//!
//! Conventions: `forward` computes `X[k] = sum x[n] e^{-j 2 pi k n / N}`
//! (unnormalised); `inverse` computes `x[n] = (1/N) sum X[k] e^{+j 2 pi k n / N}`,
//! so `inverse(forward(x)) == x`.

use crate::complex::C32;
use std::f64::consts::PI;
use std::sync::Arc;

#[derive(Debug)]
enum Kind {
    Trivial,
    Radix2 { twiddles: Vec<C32>, rev: Vec<u32> },
    Bluestein { m: usize, inner: Box<Fft>, chirp: Vec<C32>, b_fft: Vec<C32> },
}

/// A reusable FFT plan for one length.
#[derive(Debug)]
pub struct Fft {
    n: usize,
    kind: Kind,
}

impl Fft {
    /// Plan a transform of length `n` (`n >= 1`).
    pub fn new(n: usize) -> Fft {
        assert!(n >= 1, "FFT length must be >= 1");
        if n == 1 {
            return Fft { n, kind: Kind::Trivial };
        }
        if n.is_power_of_two() {
            let bits = n.trailing_zeros();
            let rev = (0..n as u32).map(|i| i.reverse_bits() >> (32 - bits)).collect();
            let twiddles = (0..n / 2)
                .map(|k| C32::expj64(-2.0 * PI * k as f64 / n as f64))
                .collect();
            return Fft { n, kind: Kind::Radix2 { twiddles, rev } };
        }
        // Bluestein: X_k = w_k * sum_n (x_n w_n) conj(w_{k-n}), w_k = e^{-j pi k^2 / N}.
        let m = (2 * n - 1).next_power_of_two();
        let inner = Box::new(Fft::new(m));
        let two_n = 2 * n as u64;
        let chirp: Vec<C32> = (0..n as u64)
            .map(|k| {
                let k2 = (k * k) % two_n; // exact phase reduction keeps precision for big N
                C32::expj64(-PI * k2 as f64 / n as f64)
            })
            .collect();
        let mut b = vec![C32::ZERO; m];
        b[0] = chirp[0].conj();
        for k in 1..n {
            b[k] = chirp[k].conj();
            b[m - k] = chirp[k].conj();
        }
        inner.forward(&mut b);
        Fft { n, kind: Kind::Bluestein { m, inner, chirp, b_fft: b } }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// In-place forward transform. `buf.len()` must equal the plan length.
    pub fn forward(&self, buf: &mut [C32]) {
        assert_eq!(buf.len(), self.n, "FFT buffer length mismatch");
        match &self.kind {
            Kind::Trivial => {}
            Kind::Radix2 { twiddles, rev } => radix2(buf, twiddles, rev),
            Kind::Bluestein { m, inner, chirp, b_fft } => {
                let mut a = vec![C32::ZERO; *m];
                for k in 0..self.n {
                    a[k] = buf[k] * chirp[k];
                }
                inner.forward(&mut a);
                for (x, b) in a.iter_mut().zip(b_fft.iter()) {
                    *x *= *b;
                }
                inner.inverse(&mut a);
                for k in 0..self.n {
                    buf[k] = a[k] * chirp[k];
                }
            }
        }
    }

    /// In-place inverse transform, normalised by `1/N`.
    pub fn inverse(&self, buf: &mut [C32]) {
        for v in buf.iter_mut() {
            *v = v.conj();
        }
        self.forward(buf);
        let k = 1.0 / self.n as f32;
        for v in buf.iter_mut() {
            *v = v.conj().scale(k);
        }
    }

    /// Forward transform of a real signal. Returns the `N/2 + 1` non-negative bins.
    pub fn forward_real(&self, input: &[f32]) -> Vec<C32> {
        assert_eq!(input.len(), self.n);
        let mut buf: Vec<C32> = input.iter().map(|&v| C32::new(v, 0.0)).collect();
        self.forward(&mut buf);
        buf.truncate(self.n / 2 + 1);
        buf
    }

    /// Inverse of [`Fft::forward_real`]: takes `N/2 + 1` bins (Hermitian symmetry
    /// is assumed) and returns `N` real samples.
    pub fn inverse_real(&self, half: &[C32]) -> Vec<f32> {
        assert_eq!(half.len(), self.n / 2 + 1);
        let mut buf = vec![C32::ZERO; self.n];
        buf[..half.len()].copy_from_slice(half);
        for k in 1..self.n - self.n / 2 {
            buf[self.n - k] = half[k].conj();
        }
        self.inverse(&mut buf);
        buf.iter().map(|z| z.re).collect()
    }
}

fn radix2(buf: &mut [C32], tw: &[C32], rev: &[u32]) {
    let n = buf.len();
    for i in 0..n {
        let j = rev[i] as usize;
        if i < j {
            buf.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let step = n / len;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let w = tw[k * step];
                let a = buf[start + k];
                let b = buf[start + k + half] * w;
                buf[start + k] = a + b;
                buf[start + k + half] = a - b;
            }
        }
        len <<= 1;
    }
}

/// Caches FFT plans by length.
#[derive(Default)]
pub struct FftPlanner {
    plans: Vec<Arc<Fft>>,
}

impl FftPlanner {
    pub fn new() -> FftPlanner {
        FftPlanner::default()
    }

    /// Return the cached plan for `n`, creating it on first use.
    pub fn plan(&mut self, n: usize) -> Arc<Fft> {
        if let Some(p) = self.plans.iter().find(|p| p.len() == n) {
            return p.clone();
        }
        let p = Arc::new(Fft::new(n));
        self.plans.push(p.clone());
        p
    }
}

/// Convenience: out-of-place forward FFT of any length.
pub fn fft(x: &[C32]) -> Vec<C32> {
    let mut v = x.to_vec();
    Fft::new(x.len()).forward(&mut v);
    v
}

/// Convenience: out-of-place inverse FFT (normalised) of any length.
pub fn ifft(x: &[C32]) -> Vec<C32> {
    let mut v = x.to_vec();
    Fft::new(x.len()).inverse(&mut v);
    v
}

/// Rotate so that DC sits in the middle (`-fs/2 .. +fs/2` order).
pub fn fft_shift<T: Copy>(x: &mut [T]) {
    let n = x.len();
    x.rotate_left(n.div_ceil(2));
}

/// Frequency (as a fraction of the sample rate, in `[-0.5, 0.5)`) of FFT bin `k` of `n`.
pub fn bin_freq(k: usize, n: usize) -> f32 {
    let k = k % n;
    if k < n.div_ceil(2) {
        k as f32 / n as f32
    } else {
        k as f32 / n as f32 - 1.0
    }
}

/// Reference O(N^2) DFT (double precision), for tests.
pub fn naive_dft(x: &[C32]) -> Vec<C32> {
    let n = x.len();
    (0..n)
        .map(|k| {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, v) in x.iter().enumerate() {
                let ph = -2.0 * PI * ((k * i) % n) as f64 / n as f64;
                let (s, c) = ph.sin_cos();
                re += v.re as f64 * c - v.im as f64 * s;
                im += v.re as f64 * s + v.im as f64 * c;
            }
            C32::new(re as f32, im as f32)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    fn rand_vec(n: usize, seed: u64) -> Vec<C32> {
        let mut r = Rng::new(seed);
        (0..n).map(|_| r.cgaussian(2.0)).collect()
    }

    fn max_err(a: &[C32], b: &[C32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| (*x - *y).abs()).fold(0.0, f32::max)
    }

    #[test]
    fn matches_naive_dft_for_many_lengths() {
        for &n in &[1usize, 2, 3, 4, 5, 7, 8, 12, 16, 31, 64, 100, 127, 256, 360, 1000, 1024] {
            let x = rand_vec(n, n as u64);
            let want = naive_dft(&x);
            let got = fft(&x);
            let scale = (n as f32).sqrt();
            let e = max_err(&got, &want) / scale;
            assert!(e < 2e-5, "n={n} err={e}");
        }
    }

    #[test]
    fn inverse_round_trip() {
        for &n in &[16usize, 48, 1023, 4096] {
            let x = rand_vec(n, 99);
            let y = ifft(&fft(&x));
            assert!(max_err(&x, &y) < 1e-4, "n={n}");
        }
    }

    #[test]
    fn real_fft_round_trip() {
        let n = 30;
        let mut r = Rng::new(5);
        let x: Vec<f32> = (0..n).map(|_| r.gaussian() as f32).collect();
        let p = Fft::new(n);
        let h = p.forward_real(&x);
        assert_eq!(h.len(), 16);
        let y = p.inverse_real(&h);
        for (a, b) in x.iter().zip(&y) {
            assert!((a - b).abs() < 1e-5);
        }
        let full = naive_dft(&x.iter().map(|&v| C32::new(v, 0.0)).collect::<Vec<_>>());
        assert!(max_err(&h, &full[..16]) < 1e-4);
    }

    #[test]
    fn planner_caches() {
        let mut p = FftPlanner::new();
        let a = p.plan(64);
        let b = p.plan(64);
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn shift_and_bins() {
        let mut v = [0, 1, 2, 3, 4, 5, 6, 7];
        fft_shift(&mut v);
        assert_eq!(v, [4, 5, 6, 7, 0, 1, 2, 3]);
        assert_eq!(bin_freq(6, 8), -0.25);
        assert_eq!(bin_freq(2, 8), 0.25);
    }
}
