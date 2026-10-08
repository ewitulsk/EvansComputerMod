//! Spectrum display (waterfall) and band scanning.

use ecm_dsp::analysis::{Waterfall, Welch};
use ecm_dsp::window::Window;
use ecm_dsp::C32;
use std::collections::VecDeque;

/// 256-entry heat palette (black -> blue -> cyan -> yellow -> red -> white),
/// as 768 RGB bytes for an indexed8 display.
pub fn heat_palette() -> Vec<u8> {
    let stops: [(f32, [f32; 3]); 6] = [
        (0.0, [0.0, 0.0, 0.0]),
        (0.2, [0.0, 0.0, 0.6]),
        (0.45, [0.0, 0.7, 0.9]),
        (0.65, [1.0, 0.9, 0.0]),
        (0.85, [1.0, 0.1, 0.0]),
        (1.0, [1.0, 1.0, 1.0]),
    ];
    let mut out = Vec::with_capacity(768);
    for i in 0..256 {
        let t = i as f32 / 255.0;
        let k = stops.iter().rposition(|s| s.0 <= t).unwrap_or(0).min(stops.len() - 2);
        let (t0, c0) = stops[k];
        let (t1, c1) = stops[k + 1];
        let u = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
        for j in 0..3 {
            out.push(((c0[j] + (c1[j] - c0[j]) * u) * 255.0).round() as u8);
        }
    }
    out
}

/// Palette index used for the spectrum trace and grid.
pub const TRACE: u8 = 255;
pub const GRID: u8 = 60;

/// A spectrum trace on top and a scrolling waterfall below, rendered into an
/// indexed8 framebuffer (`width x height`).
pub struct WaterfallView {
    pub width: usize,
    pub height: usize,
    pub spectrum_height: usize,
    wf: Waterfall,
    rows: VecDeque<Vec<u8>>,
    last_db: Vec<f32>,
}

impl WaterfallView {
    pub fn new(width: usize, height: usize, nfft: usize, min_db: f32, max_db: f32) -> WaterfallView {
        let spectrum_height = (height / 3).max(8).min(height.saturating_sub(4));
        WaterfallView {
            width,
            height,
            spectrum_height,
            wf: Waterfall::new(nfft, width, min_db, max_db),
            rows: VecDeque::new(),
            last_db: vec![min_db; width],
        }
    }

    pub fn nfft(&self) -> usize {
        self.wf.nfft()
    }

    /// Add one row from a block of samples (the first `nfft` are used).
    pub fn push(&mut self, x: &[C32]) {
        self.last_db = self.wf.row_db(x);
        let span = (self.wf.max_db - self.wf.min_db).max(1e-6);
        let row = self
            .last_db
            .iter()
            .map(|d| (((d - self.wf.min_db) / span).clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect();
        self.rows.push_front(row);
        let keep = self.height - self.spectrum_height;
        self.rows.truncate(keep);
    }

    /// Column of the strongest bin in the last row and its level (dB).
    pub fn peak(&self) -> (usize, f32) {
        self.last_db
            .iter()
            .cloned()
            .enumerate()
            .fold((0, f32::NEG_INFINITY), |a, (i, d)| if d > a.1 { (i, d) } else { a })
    }

    /// Render the whole frame.
    pub fn render(&self) -> Vec<u8> {
        let (w, h, sh) = (self.width, self.height, self.spectrum_height);
        let mut fb = vec![0u8; w * h];
        // grid: 4 horizontal lines, centre line
        for k in 1..4 {
            let y = k * sh / 4;
            fb[y * w..y * w + w].iter_mut().step_by(2).for_each(|p| *p = GRID);
        }
        for y in 0..sh {
            fb[y * w + w / 2] = GRID;
        }
        let span = (self.wf.max_db - self.wf.min_db).max(1e-6);
        let mut prev: Option<usize> = None;
        for x in 0..w {
            let v = ((self.last_db[x] - self.wf.min_db) / span).clamp(0.0, 1.0);
            let y = sh - 1 - ((v * (sh - 1) as f32).round() as usize);
            let (a, b) = match prev {
                Some(p) => (p.min(y), p.max(y)),
                None => (y, y),
            };
            for yy in a..=b {
                fb[yy * w + x] = TRACE;
            }
            prev = Some(y);
        }
        for (i, row) in self.rows.iter().enumerate() {
            let y = sh + i;
            if y >= h {
                break;
            }
            fb[y * w..y * w + w].copy_from_slice(row);
        }
        fb
    }

    /// Text-mode row for a terminal without graphics: one character per column.
    pub fn text_row(&self) -> String {
        const SHADES: &[u8] = b" .:-=+*#%@";
        let span = (self.wf.max_db - self.wf.min_db).max(1e-6);
        self.last_db
            .iter()
            .map(|d| {
                let v = ((d - self.wf.min_db) / span).clamp(0.0, 1.0);
                SHADES[((v * (SHADES.len() - 1) as f32).round() as usize).min(SHADES.len() - 1)] as char
            })
            .collect()
    }
}

/// A signal found by [`find_signals`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Absolute frequency of the strongest bin (Hz).
    pub freq: f64,
    /// Peak power per bin (dB).
    pub power_db: f32,
    /// Peak above the noise floor (dB).
    pub snr_db: f32,
}

/// Find signals in a block received at `center` / `rate`: Welch PSD, noise
/// floor = median bin, report each run of bins at least `threshold_db` above
/// it by its strongest bin. The outer 10% of the band (filter roll-off) is
/// ignored.
pub fn find_signals(x: &[C32], rate: f64, center: f64, nfft: usize, threshold_db: f32) -> Vec<Hit> {
    let mut w = Welch::new(nfft, 0.5, Window::Hann);
    w.push(x);
    let p = w.psd_db_shifted();
    if p.is_empty() {
        return Vec::new();
    }
    let mut sorted = p.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let floor = sorted[sorted.len() / 2];
    let lo = nfft / 20;
    let hi = nfft - nfft / 20;
    let mut hits = Vec::new();
    let mut i = lo;
    while i < hi {
        if p[i] - floor >= threshold_db {
            let mut best = i;
            while i < hi && p[i] - floor >= threshold_db - 6.0 {
                if p[i] > p[best] {
                    best = i;
                }
                i += 1;
            }
            let f = center + (best as f64 - nfft as f64 / 2.0) * rate / nfft as f64;
            hits.push(Hit { freq: f, power_db: p[best], snr_db: p[best] - floor });
        } else {
            i += 1;
        }
    }
    hits
}

/// The strongest audio tone in `x` (real samples at `rate`) above 100 Hz:
/// `(frequency, dB above the median bin)`. `None` for silence.
pub fn strongest_tone(x: &[f32], rate: f64) -> Option<(f64, f32)> {
    let nfft = 2048;
    if x.len() < nfft {
        return None;
    }
    let c: Vec<C32> = x.iter().map(|&v| C32::new(v, 0.0)).collect();
    let mut w = Welch::new(nfft, 0.5, Window::Hann);
    w.push(&c);
    let p = w.psd();
    let half = &p[..nfft / 2];
    let lo = ((100.0 / rate) * nfft as f64).ceil() as usize;
    let (k, peak) = half.iter().enumerate().skip(lo).fold((0, 0.0f32), |a, (i, &v)| if v > a.1 { (i, v) } else { a });
    if peak <= 0.0 {
        return None;
    }
    let mut sorted = half[lo..].to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted[sorted.len() / 2].max(1e-30);
    Some((k as f64 * rate / nfft as f64, ecm_dsp::complex::to_db(peak / median)))
}

/// Centre frequencies to visit when scanning `start..=stop` with a receiver
/// of `rate` samples/s (80% of each window is used).
pub fn scan_plan(start: f64, stop: f64, rate: f64) -> Vec<f64> {
    let step = rate * 0.8;
    let (a, b) = if start <= stop { (start, stop) } else { (stop, start) };
    let mut out = Vec::new();
    let mut c = a + step / 2.0;
    loop {
        out.push(c);
        if c + step / 2.0 >= b || out.len() >= 100_000 {
            break;
        }
        c += step;
    }
    out
}

/// Merge hits from overlapping windows that are within `tolerance` Hz,
/// keeping the strongest.
pub fn merge_hits(mut hits: Vec<Hit>, tolerance: f64) -> Vec<Hit> {
    hits.sort_by(|a, b| a.freq.partial_cmp(&b.freq).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<Hit> = Vec::new();
    for h in hits {
        match out.last_mut() {
            Some(l) if h.freq - l.freq <= tolerance => {
                if h.power_db > l.power_db {
                    *l = h;
                }
            }
            _ => out.push(h),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Tone;

    fn noisy_tone(hz: f64, amp: f32, n: usize) -> Vec<C32> {
        let mut x = Tone::new(hz, 48_000.0, amp).complex(n);
        let mut rng = ecm_dsp::rng::Rng::new(7);
        ecm_dsp::rng::awgn(&mut x, 1e-4, &mut rng);
        x
    }

    #[test]
    fn signals_are_found_at_their_frequency() {
        let mut x = noisy_tone(5_000.0, 0.3, 16_384);
        let y = Tone::new(-12_000.0, 48_000.0, 0.1).complex(16_384);
        for (a, b) in x.iter_mut().zip(y) {
            *a += b;
        }
        let hits = find_signals(&x, 48_000.0, 146.0e6, 1024, 15.0);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!((hits[0].freq - (146.0e6 - 12_000.0)).abs() < 100.0, "{hits:?}");
        assert!((hits[1].freq - (146.0e6 + 5_000.0)).abs() < 100.0, "{hits:?}");
        assert!(hits[1].power_db > hits[0].power_db);
        // noise only: nothing
        let mut n = vec![C32::ZERO; 16_384];
        ecm_dsp::rng::awgn(&mut n, 1e-4, &mut ecm_dsp::rng::Rng::new(1));
        assert!(find_signals(&n, 48_000.0, 0.0, 1024, 15.0).is_empty());
    }

    #[test]
    fn strongest_tone_finds_audio_tone() {
        let mut x = Tone::new(1000.0, 24_000.0, 0.5).real(24_000);
        let mut rng = ecm_dsp::rng::Rng::new(3);
        ecm_dsp::rng::awgn_real(&mut x, 1e-3, &mut rng);
        let (f, snr) = strongest_tone(&x, 24_000.0).unwrap();
        assert!((f - 1000.0).abs() < 15.0 && snr > 30.0, "{f} {snr}");
        let mut n = vec![0.0f32; 24_000];
        ecm_dsp::rng::awgn_real(&mut n, 1e-3, &mut rng);
        assert!(strongest_tone(&n, 24_000.0).unwrap().1 < 20.0);
        assert!(strongest_tone(&[0.0; 100], 8000.0).is_none());
    }

    #[test]
    fn scan_plan_covers_band() {
        let p = scan_plan(144e6, 144.2e6, 48_000.0);
        assert!((p[0] - 144.0192e6).abs() < 1.0);
        assert!(p.last().unwrap() + 19_200.0 >= 144.2e6);
        assert_eq!(p.len(), 6);
        let m = merge_hits(
            vec![
                Hit { freq: 100.0, power_db: -10.0, snr_db: 30.0 },
                Hit { freq: 150.0, power_db: -5.0, snr_db: 35.0 },
                Hit { freq: 5000.0, power_db: -20.0, snr_db: 20.0 },
            ],
            500.0,
        );
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].freq, 150.0);
    }

    #[test]
    fn waterfall_renders_trace_and_rows() {
        let mut v = WaterfallView::new(64, 48, 256, -100.0, 0.0);
        for _ in 0..5 {
            v.push(&noisy_tone(12_000.0, 0.5, 256));
        }
        let (col, db) = v.peak();
        assert_eq!(col, 48, "+12 kHz at 48 kS/s is 3/4 across");
        assert!(db > -20.0);
        let fb = v.render();
        assert_eq!(fb.len(), 64 * 48);
        let wf_top = v.spectrum_height;
        assert!(fb[wf_top * 64 + 48] > 200, "hot pixel at the tone");
        assert!(fb[wf_top * 64 + 10] < 100, "cool pixel elsewhere");
        assert!(fb[..wf_top * 64].contains(&TRACE));
        assert!(fb[(wf_top + 5) * 64..].iter().all(|&p| p == 0), "only 5 rows so far");
        let t = v.text_row();
        assert_eq!(t.chars().count(), 64);
        assert!("#%@".contains(t.chars().nth(48).unwrap()), "{t}");
        assert!(" .:-".contains(t.chars().nth(10).unwrap()), "{t}");
        assert_eq!(heat_palette().len(), 768);
    }
}
