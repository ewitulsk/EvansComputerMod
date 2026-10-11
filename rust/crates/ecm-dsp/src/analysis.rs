//! Spectrum analysis: Welch PSD, waterfall rows and SNR estimation.

use crate::complex::{to_db, C32};
use crate::fft::{fft_shift, Fft};
use crate::window::Window;
use std::sync::Arc;

/// Welch power spectrum estimator with persistent overlap state (streaming).
///
/// Scaling is **power per bin**: summing all bins of white noise of variance
/// `s^2` gives `s^2`; a complex tone of amplitude `A` peaks near `A^2 / ENBW`
/// (ENBW in bins, 1.5 for Hann).
#[derive(Clone, Debug)]
pub struct Welch {
    nfft: usize,
    hop: usize,
    window: Vec<f32>,
    fft: Arc<Fft>,
    buf: Vec<C32>,
    acc: Vec<f64>,
    segments: usize,
    norm: f64,
}

impl Welch {
    /// `overlap` is the fraction in `[0, 1)` (0.5 is standard).
    pub fn new(nfft: usize, overlap: f32, window: Window) -> Welch {
        let w = window.periodic(nfft);
        let s2: f64 = w.iter().map(|&v| (v as f64) * (v as f64)).sum();
        let hop = ((nfft as f32 * (1.0 - overlap)).round() as usize).clamp(1, nfft);
        Welch {
            nfft,
            hop,
            window: w,
            fft: Arc::new(Fft::new(nfft)),
            buf: Vec::new(),
            acc: vec![0.0; nfft],
            segments: 0,
            norm: 1.0 / (nfft as f64 * s2),
        }
    }

    /// Feed samples; full segments are accumulated.
    pub fn push(&mut self, x: &[C32]) {
        self.buf.extend_from_slice(x);
        let mut start = 0;
        let mut seg = vec![C32::ZERO; self.nfft];
        while start + self.nfft <= self.buf.len() {
            for i in 0..self.nfft {
                seg[i] = self.buf[start + i].scale(self.window[i]);
            }
            self.fft.forward(&mut seg);
            for (a, v) in self.acc.iter_mut().zip(&seg) {
                *a += v.norm_sqr() as f64;
            }
            self.segments += 1;
            start += self.hop;
        }
        self.buf.drain(..start.min(self.buf.len()));
    }

    pub fn segments(&self) -> usize {
        self.segments
    }

    /// Averaged linear power per bin in FFT order (DC first). Empty if no segment yet.
    pub fn psd(&self) -> Vec<f32> {
        if self.segments == 0 {
            return Vec::new();
        }
        let k = self.norm / self.segments as f64;
        self.acc.iter().map(|&a| (a * k) as f32).collect()
    }

    /// PSD in dB, centred (`-fs/2 .. fs/2`).
    pub fn psd_db_shifted(&self) -> Vec<f32> {
        let mut p: Vec<f32> = self.psd().into_iter().map(to_db).collect();
        fft_shift(&mut p);
        p
    }

    pub fn reset(&mut self) {
        self.acc.iter_mut().for_each(|a| *a = 0.0);
        self.segments = 0;
        self.buf.clear();
    }
}

/// One-shot Welch PSD of a complex block (FFT order, power per bin).
pub fn welch_psd(x: &[C32], nfft: usize, overlap: f32, window: Window) -> Vec<f32> {
    let mut w = Welch::new(nfft, overlap, window);
    w.push(x);
    w.psd()
}

/// Turns blocks of IQ into fixed-width waterfall rows.
#[derive(Clone, Debug)]
pub struct Waterfall {
    nfft: usize,
    width: usize,
    pub min_db: f32,
    pub max_db: f32,
    window: Vec<f32>,
    fft: Arc<Fft>,
    norm: f32,
}

impl Waterfall {
    /// `width` output columns spanning `-fs/2 .. fs/2`; dB range mapped to 0..=255.
    pub fn new(nfft: usize, width: usize, min_db: f32, max_db: f32) -> Waterfall {
        let window = Window::BlackmanHarris.periodic(nfft);
        let s2: f32 = window.iter().map(|v| v * v).sum();
        Waterfall {
            nfft,
            width,
            min_db,
            max_db,
            window,
            fft: Arc::new(Fft::new(nfft)),
            norm: 1.0 / (nfft as f32 * s2),
        }
    }

    pub fn nfft(&self) -> usize {
        self.nfft
    }

    /// dB values (power per bin) for one row, centred, `width` columns. Each
    /// column takes the maximum of the bins it covers (so narrow signals stay
    /// visible), or interpolates when columns outnumber bins. Uses the first
    /// `nfft` samples of `x` (zero-padded if shorter).
    pub fn row_db(&self, x: &[C32]) -> Vec<f32> {
        let mut buf = vec![C32::ZERO; self.nfft];
        for (i, v) in x.iter().take(self.nfft).enumerate() {
            buf[i] = v.scale(self.window[i]);
        }
        self.fft.forward(&mut buf);
        let mut p: Vec<f32> = buf.iter().map(|v| v.norm_sqr() * self.norm).collect();
        fft_shift(&mut p);
        let n = self.nfft as f32;
        (0..self.width)
            .map(|c| {
                let a = c as f32 * n / self.width as f32;
                let b = (c + 1) as f32 * n / self.width as f32;
                let (ia, ib) = (a.floor() as usize, (b.ceil() as usize).min(self.nfft));
                let m = if ib > ia + 1 {
                    p[ia..ib].iter().cloned().fold(0.0, f32::max)
                } else {
                    p[ia.min(self.nfft - 1)]
                };
                to_db(m)
            })
            .collect()
    }

    /// Row quantised to 0..=255 between `min_db` and `max_db`.
    pub fn row(&self, x: &[C32]) -> Vec<u8> {
        let span = (self.max_db - self.min_db).max(1e-6);
        self.row_db(x)
            .into_iter()
            .map(|d| (((d - self.min_db) / span).clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect()
    }
}

/// M2M4 SNR estimator for constant-modulus signals (PSK, FM) in complex AWGN.
/// Returns the SNR in dB, or `None` if it cannot be estimated (e.g. pure noise).
pub fn snr_m2m4(x: &[C32]) -> Option<f32> {
    if x.is_empty() {
        return None;
    }
    let n = x.len() as f64;
    let m2 = x.iter().map(|v| v.norm_sqr() as f64).sum::<f64>() / n;
    let m4 = x.iter().map(|v| (v.norm_sqr() as f64).powi(2)).sum::<f64>() / n;
    let d = 2.0 * m2 * m2 - m4;
    if d <= 0.0 {
        return None;
    }
    let s = d.sqrt();
    let nn = m2 - s;
    if nn <= 0.0 {
        return Some(100.0);
    }
    Some((10.0 * (s / nn).log10()) as f32)
}

/// SNR from a PSD (FFT order, power per bin): signal power in bins `lo..hi`
/// (normalised frequency band, may be negative) over the noise power in the
/// same bandwidth, using the median of the out-of-band bins as the noise floor.
/// Returns dB.
pub fn snr_from_psd(psd: &[f32], lo: f32, hi: f32) -> f32 {
    let n = psd.len();
    let in_band = |k: usize| {
        let f = crate::fft::bin_freq(k, n);
        f >= lo && f <= hi
    };
    let mut noise: Vec<f32> = (0..n).filter(|&k| !in_band(k)).map(|k| psd[k]).collect();
    noise.sort_by(|a, b| a.total_cmp(b));
    let floor = if noise.is_empty() { 0.0 } else { noise[noise.len() / 2] };
    let band: Vec<f32> = (0..n).filter(|&k| in_band(k)).map(|k| psd[k]).collect();
    let total: f32 = band.iter().sum();
    let nb = floor * band.len() as f32;
    to_db((total - nb).max(1e-30) / nb.max(1e-30))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::{awgn, Rng};

    #[test]
    fn welch_scaling_and_tone_location() {
        let mut rng = Rng::new(1);
        let mut x = vec![C32::ZERO; 64 * 1024];
        awgn(&mut x, 0.5, &mut rng);
        let p = welch_psd(&x, 1024, 0.5, Window::Hann);
        let total: f32 = p.iter().sum();
        assert!((total - 0.5).abs() < 0.02, "total {total}");
        let tone: Vec<C32> = (0..16_384).map(|i| C32::expj(2.0 * std::f32::consts::PI * 0.125 * i as f32)).collect();
        let p = welch_psd(&tone, 1024, 0.5, Window::Hann);
        let (k, &v) = p.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
        assert_eq!(k, 128);
        assert!((v - 1.0 / 1.5).abs() < 0.01);
    }

    #[test]
    fn waterfall_row_shape() {
        let wf = Waterfall::new(512, 128, -100.0, 0.0);
        let x: Vec<C32> = (0..512).map(|i| C32::expj(2.0 * std::f32::consts::PI * -0.25 * i as f32)).collect();
        let row = wf.row(&x);
        assert_eq!(row.len(), 128);
        let (c, _) = row.iter().enumerate().max_by_key(|(_, &v)| v).unwrap();
        // -0.25 fs -> a quarter of the way across
        assert!((c as i32 - 32).abs() <= 1, "col {c}");
        let wide = Waterfall::new(64, 200, -100.0, 0.0);
        assert_eq!(wide.row(&x).len(), 200);
    }

    #[test]
    fn snr_estimators() {
        let mut rng = Rng::new(2);
        for &snr in &[0.0f32, 10.0, 20.0] {
            let mut x: Vec<C32> = (0..50_000)
                .map(|_| C32::new(if rng.bit() == 1 { 1.0 } else { -1.0 }, 0.0))
                .collect();
            crate::rng::awgn_snr(&mut x, 1.0, snr, &mut rng);
            let est = snr_m2m4(&x).unwrap();
            assert!((est - snr).abs() < 0.5, "m2m4 {snr}: {est}");
        }
        // tone in noise: band-limited SNR
        let n = 64 * 1024;
        let mut x: Vec<C32> = (0..n).map(|i| C32::expj(2.0 * std::f32::consts::PI * 0.1 * i as f32)).collect();
        awgn(&mut x, 1.0, &mut rng); // 0 dB over full band
        let p = welch_psd(&x, 1024, 0.5, Window::Hann);
        let est = snr_from_psd(&p, 0.095, 0.105);
        // band 0.01 of fs -> noise in band is -20 dB -> SNR ~ 20 dB
        assert!((est - 20.0).abs() < 1.5, "psd snr {est}");
    }
}
