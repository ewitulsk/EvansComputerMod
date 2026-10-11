//! Rational polyphase resampling (`L/M`), decimation and interpolation.

use crate::complex::C32;
use crate::fir::{lowpass, Tap};
use crate::window::{kaiser_beta, Window};

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Streaming rational resampler: output rate = input rate * `L / M`.
///
/// Implemented as polyphase decomposition of a prototype low-pass designed at
/// the upsampled rate. Generic over real (`f32`) or complex (`C32`) samples.
#[derive(Clone, Debug)]
pub struct Resampler<S> {
    l: usize,
    m: usize,
    phases: Vec<Vec<f32>>, // phases[p][k] = h[p + k L] (scaled by L)
    hist: Vec<S>,          // double-length ring, newest first from `pos`
    pos: usize,
    k: usize,
    phase: usize,
}

impl<S> Resampler<S>
where
    f32: Tap<S>,
    S: Copy + Default,
{
    /// Resample by `l / m` (reduced by their gcd) with a default Kaiser prototype
    /// (`taps_per_phase` = 24, 70 dB stopband, cutoff at 0.9 of the lower Nyquist).
    pub fn new(l: usize, m: usize) -> Self {
        Self::with_quality(l, m, 24, 70.0)
    }

    /// Resample with a chosen filter length per phase and stopband attenuation.
    pub fn with_quality(l: usize, m: usize, taps_per_phase: usize, atten_db: f32) -> Self {
        assert!(l > 0 && m > 0);
        let g = gcd(l, m);
        let (l, m) = (l / g, m / g);
        let n = taps_per_phase.max(2) * l;
        let cutoff = 0.5 / l.max(m) as f32 * 0.9;
        let proto = lowpass(n, cutoff, Window::Kaiser(kaiser_beta(atten_db)));
        Self::from_prototype(l, m, &proto)
    }

    /// Build from an explicit prototype (designed at `L` times the input rate,
    /// unity DC gain; it is scaled by `L` internally).
    pub fn from_prototype(l: usize, m: usize, proto: &[f32]) -> Self {
        let k = proto.len().div_ceil(l);
        let mut phases = vec![vec![0.0f32; k]; l];
        for (i, &v) in proto.iter().enumerate() {
            phases[i % l][i / l] = v * l as f32;
        }
        Resampler { l, m, phases, hist: vec![S::default(); 2 * k], pos: 0, k, phase: 0 }
    }

    /// Reduced interpolation factor.
    pub fn interp(&self) -> usize {
        self.l
    }

    /// Reduced decimation factor.
    pub fn decim(&self) -> usize {
        self.m
    }

    /// Process a block, appending outputs to `out`.
    pub fn process(&mut self, input: &[S], out: &mut Vec<S>) {
        for &x in input {
            self.pos = if self.pos == 0 { self.k - 1 } else { self.pos - 1 };
            self.hist[self.pos] = x;
            self.hist[self.pos + self.k] = x;
            while self.phase < self.l {
                let win = &self.hist[self.pos..self.pos + self.k];
                let mut acc = S::default();
                for (t, v) in self.phases[self.phase].iter().zip(win) {
                    acc = f32::mac(acc, *t, *v);
                }
                out.push(acc);
                self.phase += self.m;
            }
            self.phase -= self.l;
        }
    }

    pub fn process_vec(&mut self, input: &[S]) -> Vec<S> {
        let mut out = Vec::with_capacity(input.len() * self.l / self.m + 2);
        self.process(input, &mut out);
        out
    }
}

/// Streaming decimator: low-pass then keep every `m`-th sample. Only the kept
/// outputs are computed.
#[derive(Clone, Debug)]
pub struct Decimator<S> {
    fir: crate::fir::Fir<f32, S>,
    m: usize,
    count: usize,
}

impl<S> Decimator<S>
where
    f32: Tap<S>,
    S: Copy + Default,
{
    /// Decimate by `m` with a Kaiser anti-alias filter (cutoff 0.45/m, 70 dB).
    pub fn new(m: usize) -> Self {
        assert!(m >= 1);
        let taps = if m == 1 {
            vec![1.0]
        } else {
            crate::fir::lowpass_kaiser(0.45 / m as f32, 0.1 / m as f32, 70.0)
        };
        Self::with_taps(m, taps)
    }

    pub fn with_taps(m: usize, taps: Vec<f32>) -> Self {
        Decimator { fir: crate::fir::Fir::new(taps), m, count: 0 }
    }

    pub fn process(&mut self, input: &[S], out: &mut Vec<S>) {
        for &x in input {
            self.fir.push(x);
            self.count += 1;
            if self.count == self.m {
                self.count = 0;
                out.push(self.fir.output());
            }
        }
    }

    pub fn process_vec(&mut self, input: &[S]) -> Vec<S> {
        let mut out = Vec::with_capacity(input.len() / self.m + 1);
        self.process(input, &mut out);
        out
    }
}

/// Real-sample resampler.
pub type ResamplerR = Resampler<f32>;
/// Complex-sample resampler.
pub type ResamplerC = Resampler<C32>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::Fft;
    use crate::window::Window;

    fn peak_freq(x: &[f32], fs: f32) -> (f32, f32) {
        let n = x.len();
        let w = Window::BlackmanHarris.periodic(n);
        let xs: Vec<f32> = x.iter().zip(&w).map(|(a, b)| a * b).collect();
        let spec = Fft::new(n).forward_real(&xs);
        let (k, _) = spec
            .iter()
            .enumerate()
            .skip(1)
            .max_by(|a, b| a.1.norm_sqr().partial_cmp(&b.1.norm_sqr()).unwrap())
            .unwrap();
        // parabolic interpolation on log magnitude
        let l = |i: usize| spec[i].abs().max(1e-20).ln();
        let d = if k > 0 && k + 1 < spec.len() {
            let (a, b, c) = (l(k - 1), l(k), l(k + 1));
            0.5 * (a - c) / (a - 2.0 * b + c)
        } else {
            0.0
        };
        let amp = spec[k].abs() * 2.0 / w.iter().sum::<f32>();
        ((k as f32 + d) * fs / n as f32, amp)
    }

    #[test]
    fn rational_resampler_preserves_tone() {
        for &(l, m) in &[(3usize, 2usize), (2, 3), (160, 147), (1, 4), (5, 1)] {
            let fs_in = 48_000.0f32;
            let f0 = 1234.5f32;
            let x: Vec<f32> = (0..24_000)
                .map(|i| (2.0 * std::f32::consts::PI * f0 * i as f32 / fs_in).sin())
                .collect();
            let mut r = ResamplerR::new(l, m);
            // stream in odd-sized chunks
            let mut y = Vec::new();
            for c in x.chunks(997) {
                r.process(c, &mut y);
            }
            let fs_out = fs_in * r.interp() as f32 / r.decim() as f32;
            let expect = (x.len() * r.interp()).div_ceil(r.decim());
            assert!((y.len() as isize - expect as isize).abs() <= 1, "len {} vs {}", y.len(), expect);
            let tail = &y[y.len() / 4..];
            let (f, a) = peak_freq(tail, fs_out);
            assert!((f - f0).abs() < 1.0, "L/M {l}/{m}: {f} Hz");
            assert!((a - 1.0).abs() < 0.02, "L/M {l}/{m}: amp {a}");
        }
    }

    #[test]
    fn decimator_rejects_alias() {
        let fs = 48_000.0f32;
        let mut d: Decimator<C32> = Decimator::new(6); // out 8 kHz
        // 1 kHz wanted, 7 kHz would alias to -1 kHz after decimation
        let x: Vec<C32> = (0..48_000)
            .map(|i| {
                let t = i as f32 / fs;
                C32::expj(2.0 * std::f32::consts::PI * 1000.0 * t)
                    + C32::expj(2.0 * std::f32::consts::PI * 7000.0 * t)
            })
            .collect();
        let y = d.process_vec(&x);
        assert_eq!(y.len(), 8000);
        let mut buf = y[1000..1000 + 4096].to_vec();
        let w = Window::BlackmanHarris.periodic(4096);
        for (v, w) in buf.iter_mut().zip(&w) {
            *v = v.scale(*w);
        }
        Fft::new(4096).forward(&mut buf);
        let bin = |f: f32| ((f / 8000.0 * 4096.0).round() as isize).rem_euclid(4096) as usize;
        let p = |k: usize| (k.saturating_sub(3)..k + 4).map(|i| buf[i % 4096].norm_sqr()).fold(0.0, f32::max);
        let want = p(bin(1000.0));
        let alias = p(bin(-1000.0));
        assert!(10.0 * (want / alias).log10() > 60.0, "alias rejection");
    }
}
