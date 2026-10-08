//! Window functions for FIR design and spectral analysis.

use std::f64::consts::PI;

/// A window shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Window {
    Rectangular,
    Hann,
    Hamming,
    Blackman,
    /// 4-term Blackman–Harris (-92 dB sidelobes).
    BlackmanHarris,
    /// Kaiser window with shape parameter `beta`.
    Kaiser(f32),
}

impl Window {
    /// Symmetric window of length `n` (for FIR design): `w[0] == w[n-1]`.
    pub fn symmetric(self, n: usize) -> Vec<f32> {
        self.generate(n, n.saturating_sub(1))
    }

    /// Periodic window of length `n` (for FFT analysis; DFT-even).
    pub fn periodic(self, n: usize) -> Vec<f32> {
        self.generate(n, n)
    }

    fn generate(self, n: usize, denom: usize) -> Vec<f32> {
        if n == 0 {
            return Vec::new();
        }
        if n == 1 || denom == 0 {
            return vec![1.0; n];
        }
        let d = denom as f64;
        (0..n)
            .map(|i| {
                let x = i as f64 / d; // 0..1
                let c = |k: f64| (2.0 * PI * k * x).cos();
                let w = match self {
                    Window::Rectangular => 1.0,
                    Window::Hann => 0.5 - 0.5 * c(1.0),
                    Window::Hamming => 0.54 - 0.46 * c(1.0),
                    Window::Blackman => 0.42 - 0.5 * c(1.0) + 0.08 * c(2.0),
                    Window::BlackmanHarris => {
                        0.35875 - 0.48829 * c(1.0) + 0.14128 * c(2.0) - 0.01168 * c(3.0)
                    }
                    Window::Kaiser(beta) => {
                        let r = 2.0 * x - 1.0;
                        bessel_i0(beta as f64 * (1.0 - r * r).max(0.0).sqrt())
                            / bessel_i0(beta as f64)
                    }
                };
                w as f32
            })
            .collect()
    }
}

/// Modified Bessel function of the first kind, order 0 (power series).
pub fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    let mut k = 1.0;
    loop {
        term *= q / (k * k);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
        k += 1.0;
    }
    sum
}

/// Kaiser `beta` for a desired stopband attenuation in dB (Kaiser's formula).
pub fn kaiser_beta(atten_db: f32) -> f32 {
    let a = atten_db as f64;
    let b = if a > 50.0 {
        0.1102 * (a - 8.7)
    } else if a >= 21.0 {
        0.5842 * (a - 21.0).powf(0.4) + 0.07886 * (a - 21.0)
    } else {
        0.0
    };
    b as f32
}

/// Coherent gain (mean of the window): the amplitude scaling of a bin-centred tone.
pub fn coherent_gain(w: &[f32]) -> f32 {
    w.iter().sum::<f32>() / w.len() as f32
}

/// Equivalent noise bandwidth in bins: `N * sum(w^2) / sum(w)^2`.
pub fn enbw(w: &[f32]) -> f32 {
    let s1: f32 = w.iter().sum();
    let s2: f32 = w.iter().map(|v| v * v).sum();
    w.len() as f32 * s2 / (s1 * s1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes() {
        let h = Window::Hann.symmetric(5);
        assert!((h[0]).abs() < 1e-7 && (h[2] - 1.0).abs() < 1e-7 && (h[4]).abs() < 1e-7);
        let hm = Window::Hamming.symmetric(3);
        assert!((hm[0] - 0.08).abs() < 1e-6);
        let k = Window::Kaiser(0.0).symmetric(7);
        assert!(k.iter().all(|&v| (v - 1.0).abs() < 1e-6));
        assert!((enbw(&Window::Hann.periodic(1024)) - 1.5).abs() < 1e-3);
        assert!((enbw(&Window::Rectangular.periodic(64)) - 1.0).abs() < 1e-6);
        let bh = Window::BlackmanHarris.periodic(1024);
        assert!((enbw(&bh) - 2.0044).abs() < 1e-2);
        // I0(1) = 1.2660658777520082
        assert!((bessel_i0(1.0) - 1.2660658777520082).abs() < 1e-14);
    }
}
