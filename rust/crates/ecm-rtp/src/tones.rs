//! Call progress tones at 16 kHz ([`crate::RATE`]), as generators:
//! `Tone::busy().next_samples(320)`.
//!
//! | tone | frequencies (Hz) | cadence |
//! |------|------------------|---------|
//! | [`Tone::dial`] | 350 + 440 | continuous |
//! | [`Tone::ringback`] | 440 + 480 | 2 s on, 4 s off |
//! | [`Tone::busy`] | 480 + 620 | 0.5 s on, 0.5 s off |
//! | [`Tone::reorder`] (fast busy) | 480 + 620 | 0.25 s on, 0.25 s off |
//! | [`Tone::call_waiting`] | 440 | one 300 ms beep |
//! | [`Tone::sit`] (intercept) | 913.8, 1370.6, 1776.7 | 330 ms each, once |
//! | [`Tone::ringtone`] | 523.25 + 659.25 | 0.4 on, 0.2 off, 0.4 on, 2 s off |
//! | [`Tone::message`] | 880 then 1320 | 100 ms, 50 ms gap, 100 ms, once |
//!
//! Each step starts and ends with a 4 ms ramp so cadences don't click. One-shot tones
//! give silence (and [`Tone::is_done`]) after their last step.

use crate::RATE;

/// Peak level of a tone (all its frequencies together), 16-bit.
pub const DEFAULT_LEVEL: f32 = 8000.0;
const RAMP: usize = (RATE as usize) * 4 / 1000;

/// One step of a cadence: these frequencies (none = silence) for this long.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub freqs: Vec<f32>,
    pub samples: usize,
}

impl Step {
    pub fn new(freqs: &[f32], ms: u32) -> Step {
        Step {
            freqs: freqs.to_vec(),
            samples: (ms as usize) * RATE as usize / 1000,
        }
    }
}

/// A tone generator: a cadence of steps, repeating or once.
#[derive(Clone, Debug)]
pub struct Tone {
    steps: Vec<Step>,
    repeat: bool,
    level: f32,
    step: usize,
    pos: usize,
    done: bool,
}

impl Tone {
    /// A tone from steps (`(frequencies, ms)`), repeating or once.
    pub fn custom(steps: &[(&[f32], u32)], repeat: bool) -> Tone {
        let steps: Vec<Step> = steps
            .iter()
            .map(|(f, ms)| Step::new(f, *ms))
            .filter(|s| s.samples > 0)
            .collect();
        let done = steps.is_empty();
        Tone {
            steps,
            repeat,
            level: DEFAULT_LEVEL,
            step: 0,
            pos: 0,
            done,
        }
    }

    /// A continuous tone of these frequencies.
    pub fn continuous(freqs: &[f32]) -> Tone {
        Tone::custom(&[(freqs, 1000)], true)
    }

    pub fn dial() -> Tone {
        Tone::continuous(&[350.0, 440.0])
    }

    pub fn ringback() -> Tone {
        Tone::custom(&[(&[440.0, 480.0], 2000), (&[], 4000)], true)
    }

    pub fn busy() -> Tone {
        Tone::custom(&[(&[480.0, 620.0], 500), (&[], 500)], true)
    }

    /// Reorder ("fast busy"): all circuits busy / the call can't be completed.
    pub fn reorder() -> Tone {
        Tone::custom(&[(&[480.0, 620.0], 250), (&[], 250)], true)
    }

    /// One call-waiting beep (the app repeats it, typically every 10 s).
    pub fn call_waiting() -> Tone {
        Tone::custom(&[(&[440.0], 300)], false)
    }

    /// Special Information Tone, played once before an intercept announcement.
    pub fn sit() -> Tone {
        Tone::custom(
            &[(&[913.8], 330), (&[1370.6], 330), (&[1776.7], 330)],
            false,
        )
    }

    /// The phone's own ring for an incoming call.
    pub fn ringtone() -> Tone {
        let chord: &[f32] = &[523.25, 659.25];
        Tone::custom(&[(chord, 400), (&[], 200), (chord, 400), (&[], 2000)], true)
    }

    /// A new text message / voicemail notification, once.
    pub fn message() -> Tone {
        Tone::custom(&[(&[880.0], 100), (&[], 50), (&[1320.0], 100)], false)
    }

    /// The same tone at another peak level.
    pub fn with_level(mut self, peak: f32) -> Tone {
        self.level = peak.clamp(0.0, 32767.0);
        self
    }

    /// A one-shot tone has played all its steps.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Length of one cadence cycle, ms.
    pub fn cycle_ms(&self) -> u32 {
        (self.steps.iter().map(|s| s.samples).sum::<usize>() * 1000 / RATE as usize) as u32
    }

    /// Back to the start of the cadence.
    pub fn reset(&mut self) {
        self.step = 0;
        self.pos = 0;
        self.done = self.steps.is_empty();
    }

    /// The next `n` samples.
    pub fn next_samples(&mut self, n: usize) -> Vec<i16> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            if self.done {
                out.resize(n, 0);
                break;
            }
            let st = &self.steps[self.step];
            let take = (st.samples - self.pos).min(n - out.len());
            if st.freqs.is_empty() {
                out.resize(out.len() + take, 0);
            } else {
                let amp = self.level / st.freqs.len() as f32;
                for k in self.pos..self.pos + take {
                    let t = k as f32 / RATE as f32;
                    let edge = k.min(st.samples - 1 - k);
                    let ramp = if edge < RAMP {
                        edge as f32 / RAMP as f32
                    } else {
                        1.0
                    };
                    let v: f32 = st
                        .freqs
                        .iter()
                        .map(|f| (std::f32::consts::TAU * f * t).sin())
                        .sum();
                    out.push((v * amp * ramp).round().clamp(-32768.0, 32767.0) as i16);
                }
            }
            self.pos += take;
            if self.pos >= st.samples {
                self.pos = 0;
                self.step += 1;
                if self.step >= self.steps.len() {
                    self.step = 0;
                    if !self.repeat {
                        self.done = true;
                    }
                }
            }
        }
        out
    }
}

impl Iterator for Tone {
    type Item = i16;
    /// Sample by sample; a one-shot tone ends.
    fn next(&mut self) -> Option<i16> {
        if self.done {
            return None;
        }
        self.next_samples(1).pop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{power_at, rms};

    fn ms(n: u32) -> usize {
        n as usize * RATE as usize / 1000
    }

    /// RMS of each `win_ms` window.
    fn envelope(v: &[i16], win_ms: u32) -> Vec<bool> {
        v.chunks(ms(win_ms)).map(|c| rms(c) > 500.0).collect()
    }

    #[test]
    fn busy_and_reorder_cadences() {
        let b = Tone::busy().next_samples(ms(2000));
        assert_eq!(
            envelope(&b, 250),
            vec![true, true, false, false, true, true, false, false]
        );
        let r = Tone::reorder().next_samples(ms(1000));
        assert_eq!(
            envelope(&r, 125),
            vec![true, true, false, false, true, true, false, false]
        );
        let on = &b[ms(100)..ms(400)];
        assert!(power_at(on, 480.0) > 1e6 && power_at(on, 620.0) > 1e6);
        assert!(power_at(on, 440.0) < power_at(on, 480.0) / 20.0);
    }

    #[test]
    fn ringback_is_2_on_4_off() {
        let mut t = Tone::ringback();
        assert_eq!(t.cycle_ms(), 6000);
        let v = t.next_samples(ms(12000));
        assert_eq!(
            envelope(&v, 1000),
            vec![true, true, false, false, false, false, true, true, false, false, false, false]
        );
        assert!(
            power_at(&v[ms(500)..ms(1500)], 440.0) > 1e6
                && power_at(&v[ms(500)..ms(1500)], 480.0) > 1e6
        );
    }

    #[test]
    fn dial_tone_is_continuous_350_440() {
        let v = Tone::dial().next_samples(ms(3000));
        assert!(envelope(&v, 100).into_iter().all(|on| on));
        assert!(power_at(&v, 350.0) > 1e6 && power_at(&v, 440.0) > 1e6);
        let peak = v.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak as f32 <= DEFAULT_LEVEL + 1.0 && peak > 7000, "{peak}");
    }

    #[test]
    fn sit_plays_three_rising_tones_once() {
        let mut t = Tone::sit();
        let v = t.next_samples(ms(1200));
        for (i, f) in [913.8, 1370.6, 1776.7].iter().enumerate() {
            let seg = &v[ms(330 * i as u32 + 30)..ms(330 * (i as u32 + 1) - 30)];
            assert!(power_at(seg, *f) > 1e6, "tone {i}");
        }
        assert!(t.is_done());
        assert!(rms(&v[ms(1000)..]) == 0.0);
        assert!(t.next_samples(10).iter().all(|s| *s == 0));
    }

    #[test]
    fn one_shot_beeps_and_the_iterator_ends() {
        let cw: Vec<i16> = Tone::call_waiting().collect();
        assert_eq!(cw.len(), ms(300));
        assert!(power_at(&cw, 440.0) > 1e6);
        let m: Vec<i16> = Tone::message().collect();
        assert_eq!(m.len(), ms(250));
        assert!(power_at(&m[..ms(100)], 880.0) > power_at(&m[..ms(100)], 1320.0) * 10.0);
        assert!(power_at(&m[ms(150)..], 1320.0) > power_at(&m[ms(150)..], 880.0) * 10.0);
    }

    #[test]
    fn ringtone_double_ring_and_steps_ramp() {
        let v = Tone::ringtone().next_samples(ms(3000));
        assert_eq!(
            envelope(&v, 200),
            vec![
                true, true, false, true, true, false, false, false, false, false, false, false,
                false, false, false
            ]
        );
        // Steps start from silence (no click) and calls in pieces equal one call.
        assert_eq!(v[0], 0);
        let mut t = Tone::ringtone();
        let mut pieces = Vec::new();
        for n in [1, 7, 333, 5000, 42659] {
            pieces.extend(t.next_samples(n));
        }
        assert_eq!(pieces, v);
        let quiet = Tone::ringtone().with_level(1000.0).next_samples(ms(300));
        assert!(quiet.iter().all(|s| s.unsigned_abs() <= 1001));
    }
}
