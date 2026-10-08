//! CW / on-off keying, plus Morse code keying helpers.

use super::BitClock;
use crate::complex::C32;
use crate::nco::Nco;

/// OOK / CW modulator: a carrier at `tone_hz` (0 = on the tuned frequency)
/// keyed by bits at `baud`, with raised-cosine edges of `ramp` seconds to keep
/// key clicks down.
#[derive(Clone, Debug)]
pub struct OokMod {
    sps: f64,
    ramp_step: f32,
    nco: Nco,
    level: f32,
    t: f64,
    emitted: u64,
}

impl OokMod {
    pub fn new(fs: f32, baud: f32, tone_hz: f32, ramp: f32) -> OokMod {
        let ramp_samples = (ramp * fs).max(1.0);
        OokMod {
            sps: fs as f64 / baud as f64,
            ramp_step: 1.0 / ramp_samples,
            nco: Nco::new(tone_hz as f64, fs as f64),
            level: 0.0,
            t: 0.0,
            emitted: 0,
        }
    }

    /// Modulate bits (1 = key down), appending samples to `out`.
    pub fn modulate(&mut self, bits: &[u8], out: &mut Vec<C32>) {
        for &b in bits {
            self.t += self.sps;
            let target = if b & 1 == 1 { 1.0 } else { 0.0 };
            while (self.emitted as f64) < self.t {
                if self.level < target {
                    self.level = (self.level + self.ramp_step).min(1.0);
                } else if self.level > target {
                    self.level = (self.level - self.ramp_step).max(0.0);
                }
                let shaped = 0.5 - 0.5 * (std::f32::consts::PI * self.level).cos();
                out.push(self.nco.next().scale(shaped));
                self.emitted += 1;
            }
        }
    }

    pub fn modulate_vec(&mut self, bits: &[u8]) -> Vec<C32> {
        let mut v = Vec::new();
        self.modulate(bits, &mut v);
        v
    }
}

/// Non-coherent OOK demodulator: mix the tone to 0 Hz, average over half a
/// bit, take the envelope, slice against an adaptive mid-level threshold and
/// recover the bit clock from transitions.
#[derive(Clone, Debug)]
pub struct OokDemod {
    nco: Nco,
    avg: Vec<C32>,
    pos: usize,
    sum: C32,
    hi: f32,
    lo: f32,
    track: f32,
    clock: BitClock,
}

impl OokDemod {
    pub fn new(fs: f32, baud: f32, tone_hz: f32) -> OokDemod {
        let sps = fs / baud;
        let n = ((sps / 2.0).round() as usize).max(1);
        OokDemod {
            nco: Nco::new(tone_hz as f64, fs as f64),
            avg: vec![C32::ZERO; n],
            pos: 0,
            sum: C32::ZERO,
            hi: 0.0,
            lo: 0.0,
            track: 1.0 / (sps * 16.0),
            clock: BitClock::new(sps, 0.25),
        }
    }

    /// Envelope + threshold slicer; returns the soft value (`> 0` = key down).
    #[inline]
    pub fn soft_sample(&mut self, x: C32) -> f32 {
        let v = x.mul_conj(self.nco.next());
        self.sum += v - self.avg[self.pos];
        self.avg[self.pos] = v;
        self.pos = (self.pos + 1) % self.avg.len();
        let env = self.sum.abs() / self.avg.len() as f32;
        if env > self.hi {
            self.hi += 0.5 * (env - self.hi);
        } else {
            self.hi += self.track * (env - self.hi);
        }
        if env < self.lo {
            self.lo += 0.5 * (env - self.lo);
        } else {
            self.lo += self.track * (env - self.lo);
        }
        env - 0.5 * (self.hi + self.lo)
    }

    /// Demodulate samples, appending recovered bits to `out`.
    pub fn demod(&mut self, input: &[C32], out: &mut Vec<u8>) {
        for &x in input {
            let s = self.soft_sample(x);
            if let Some((b, _)) = self.clock.push(s) {
                out.push(b);
            }
        }
    }
}

const MORSE: &[(char, &str)] = &[
    ('A', ".-"), ('B', "-..."), ('C', "-.-."), ('D', "-.."), ('E', "."), ('F', "..-."),
    ('G', "--."), ('H', "...."), ('I', ".."), ('J', ".---"), ('K', "-.-"), ('L', ".-.."),
    ('M', "--"), ('N', "-."), ('O', "---"), ('P', ".--."), ('Q', "--.-"), ('R', ".-."),
    ('S', "..."), ('T', "-"), ('U', "..-"), ('V', "...-"), ('W', ".--"), ('X', "-..-"),
    ('Y', "-.--"), ('Z', "--.."), ('0', "-----"), ('1', ".----"), ('2', "..---"),
    ('3', "...--"), ('4', "....-"), ('5', "....."), ('6', "-...."), ('7', "--..."),
    ('8', "---.."), ('9', "----."), ('.', ".-.-.-"), (',', "--..--"), ('?', "..--.."),
    ('/', "-..-."), ('=', "-...-"), ('+', ".-.-."), ('-', "-....-"), ('@', ".--.-."),
];

/// Text -> keying units (1 = key down for one dit length). Unknown characters
/// are skipped. Dit = 1, dah = 3, element gap 1, letter gap 3, word gap 7.
pub fn morse_keying(text: &str) -> Vec<u8> {
    let mut v = Vec::new();
    for word in text.split_whitespace() {
        if !v.is_empty() {
            v.extend([0; 4]); // letter gap (3) already there -> 7
        }
        for ch in word.chars() {
            let up = ch.to_ascii_uppercase();
            if let Some((_, code)) = MORSE.iter().find(|(c, _)| *c == up) {
                for (i, e) in code.chars().enumerate() {
                    if i > 0 {
                        v.push(0);
                    }
                    v.extend(if e == '.' { &[1u8][..] } else { &[1u8, 1, 1][..] });
                }
                v.extend([0; 3]);
            }
        }
    }
    v
}

/// Keying units -> text (tolerant to run-length jitter: runs of 1s of length
/// >= 2 are dahs; gaps of >= 2 end a letter, >= 5 end a word).
pub fn morse_decode(units: &[u8]) -> String {
    let mut out = String::new();
    let mut sym = String::new();
    let mut i = 0;
    let flush = |sym: &mut String, out: &mut String| {
        if !sym.is_empty() {
            out.push(MORSE.iter().find(|(_, c)| *c == sym.as_str()).map(|(ch, _)| *ch).unwrap_or('?'));
            sym.clear();
        }
    };
    while i < units.len() {
        let b = units[i];
        let mut j = i;
        while j < units.len() && units[j] == b {
            j += 1;
        }
        let run = j - i;
        if b == 1 {
            sym.push(if run >= 2 { '-' } else { '.' });
        } else if j < units.len() {
            if run >= 2 {
                flush(&mut sym, &mut out);
            }
            if run >= 5 && !out.is_empty() {
                out.push(' ');
            }
        }
        i = j;
    }
    flush(&mut sym, &mut out);
    out
}

/// CW keying rate in units/second for a speed in words per minute (PARIS).
pub fn wpm_to_baud(wpm: f32) -> f32 {
    wpm / 1.2
}
