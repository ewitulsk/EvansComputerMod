//! The Now Playing visualizer: a small spectrum (16 bars) painted from the
//! samples Music hands to the speaker, one frame per 1/15 s of audio, each
//! committed for the moment its audio is heard (`CommitAt` at the speaker's
//! `pts_us`), so the bars follow the music however far ahead it is queued.
//!
//! It is cheap on purpose (the phone gives background audio a small CPU
//! share): a Goertzel filter per bar over each frame's samples, and nothing at
//! all while the window is hidden or another tab is open.

use crate::engine::{Written, RATE};

pub const W: usize = 96;
pub const H: usize = 24;
pub const BANDS: usize = 16;
/// Samples per frame (15 frames a second).
pub const FRAME_SAMPLES: usize = RATE as usize / 15;

const BAR: [u8; 4] = [0x1d, 0xb9, 0x54, 0xff];
const BACK: [u8; 4] = [0x18, 0x18, 0x18, 0xff];

pub struct Visualizer {
    pending: Vec<i16>,
    /// When the first sample since the last reset plays (µs, the display clock), and how many
    /// samples were framed since: frame times come from these (no rounding drift).
    origin: Option<u64>,
    framed: u64,
    coeffs: [f32; BANDS],
    levels: [f32; BANDS],
}

impl Default for Visualizer {
    fn default() -> Self {
        Visualizer::new()
    }
}

impl Visualizer {
    pub fn new() -> Visualizer {
        // Bands spaced evenly in log frequency from 80 Hz to 8 kHz.
        let mut coeffs = [0f32; BANDS];
        for (i, c) in coeffs.iter_mut().enumerate() {
            let f = 80.0 * (8000.0f32 / 80.0).powf(i as f32 / (BANDS - 1) as f32);
            *c = 2.0 * (std::f32::consts::TAU * f / RATE as f32).cos();
        }
        Visualizer { pending: Vec::new(), origin: None, framed: 0, coeffs, levels: [0.0; BANDS] }
    }

    /// Forget what was pending (pause, seek, a new song).
    pub fn reset(&mut self) {
        self.pending.clear();
        self.origin = None;
        self.framed = 0;
        self.levels = [0.0; BANDS];
    }

    /// The frames that `w` completes: (pts µs, RGBA pixels). `now_us` is the
    /// fallback clock when the speaker gave no pts (then `queued_ms` was ahead).
    pub fn push(&mut self, w: &Written, now_us: u64) -> Vec<(u64, Vec<u8>)> {
        let pts = w.pts_us.unwrap_or_else(|| now_us + w.queued_ms.max(0) as u64 * 1000);
        let us = |n: u64| n * 1_000_000 / RATE as u64;
        let pending = self.pending.len() as u64;
        match self.origin {
            None => {
                self.origin = Some(pts);
                self.framed = 0;
            }
            // The speaker's clock moved (an underrun, a stall): follow it, not our count.
            Some(o) if w.pts_us.is_some() && (o + us(self.framed + pending)).abs_diff(pts) > 50_000 => {
                self.origin = Some(pts.saturating_sub(us(pending)));
                self.framed = 0;
            }
            Some(_) => {}
        }
        self.pending.extend_from_slice(&w.samples);
        let mut out = Vec::new();
        while self.pending.len() >= FRAME_SAMPLES {
            let at = self.origin.unwrap_or(pts) + self.framed * 1_000_000 / RATE as u64;
            let frame: Vec<i16> = self.pending.drain(..FRAME_SAMPLES).collect();
            self.analyse(&frame);
            out.push((at, self.paint()));
            self.framed += FRAME_SAMPLES as u64;
        }
        out
    }

    fn analyse(&mut self, x: &[i16]) {
        let n = x.len() as f32;
        for (b, &c) in self.coeffs.iter().enumerate() {
            let (mut s1, mut s2) = (0f32, 0f32);
            for &v in x {
                let s = v as f32 / 32768.0 + c * s1 - s2;
                s2 = s1;
                s1 = s;
            }
            let power = (s1 * s1 + s2 * s2 - c * s1 * s2).max(0.0);
            let amp = power.sqrt() * 2.0 / n;
            // dB over 48 dB of range, 0..1.
            let level = ((20.0 * (amp + 1e-6).log10() + 48.0) / 48.0).clamp(0.0, 1.0);
            let l = &mut self.levels[b];
            *l = if level > *l { level } else { *l * 0.75 + level * 0.25 };
        }
    }

    /// The bars as RGBA, `W` x `H`.
    pub fn paint(&self) -> Vec<u8> {
        let mut px = Vec::with_capacity(W * H * 4);
        let bar_w = W / BANDS;
        for y in 0..H {
            for x in 0..W {
                let b = x / bar_w;
                let height = (self.levels[b] * H as f32).round() as usize;
                let on = x % bar_w != bar_w - 1 && H - y <= height;
                px.extend_from_slice(if on { &BAR } else { &BACK });
            }
        }
        px
    }

    pub fn levels(&self) -> &[f32; BANDS] {
        &self.levels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, n: usize) -> Vec<i16> {
        (0..n).map(|i| ((i as f32 * hz * std::f32::consts::TAU / RATE as f32).sin() * 12000.0) as i16).collect()
    }

    #[test]
    fn frames_carry_the_time_their_audio_plays_and_show_the_tone() {
        let mut v = Visualizer::new();
        let w = Written { pts_us: Some(5_000_000), queued_ms: 0, samples: tone(1000.0, FRAME_SAMPLES * 2 + 100) };
        let frames = v.push(&w, 0);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].0, 5_000_000);
        assert_eq!(frames[1].0, 5_000_000 + FRAME_SAMPLES as u64 * 1_000_000 / RATE as u64);
        assert_eq!(frames[0].1.len(), W * H * 4);
        let l = v.levels();
        let loud = (0..BANDS).max_by(|a, b| l[*a].total_cmp(&l[*b])).unwrap();
        let f = 80.0 * (8000.0f32 / 80.0).powf(loud as f32 / (BANDS - 1) as f32);
        assert!((600.0..1700.0).contains(&f), "the 1 kHz tone lights the bar near 1 kHz ({f} Hz)");
        // The rest of the chunk waits for the next one; without a pts the fallback clock is used.
        let next = v.push(&Written { pts_us: None, queued_ms: 300, samples: tone(1000.0, FRAME_SAMPLES) }, 9_000_000);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].0, 5_000_000 + 2 * FRAME_SAMPLES as u64 * 1_000_000 / RATE as u64, "continues the earlier clock");
        v.reset();
        let after = v.push(&Written { pts_us: None, queued_ms: 300, samples: tone(1000.0, FRAME_SAMPLES) }, 9_000_000);
        assert_eq!(after[0].0, 9_300_000, "after a reset: now + what is queued");
    }
}
