//! IQ sample formats for `/dev/sdr*` and SigMF files, and conversions to `C32`.
//!
//! All multi-byte formats are little-endian and interleaved `I, Q, I, Q, ...`.

use crate::complex::C32;
use std::fmt;
use std::str::FromStr;

/// Wire format of IQ samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    /// Unsigned 8-bit, offset 127.5 (RTL-SDR native).
    Cu8,
    /// Signed 8-bit (HackRF native).
    Cs8,
    /// Signed 16-bit, full scale 32767.
    Cs16,
    /// 32-bit float, full scale 1.0.
    Cf32,
}

impl SampleFormat {
    /// Bytes per complex sample.
    pub fn bytes_per_sample(self) -> usize {
        match self {
            SampleFormat::Cu8 | SampleFormat::Cs8 => 2,
            SampleFormat::Cs16 => 4,
            SampleFormat::Cf32 => 8,
        }
    }

    /// Name as used by `/dev/sdrctl` and file extensions (`cs16`, `cf32`, ...).
    pub fn name(self) -> &'static str {
        match self {
            SampleFormat::Cu8 => "cu8",
            SampleFormat::Cs8 => "cs8",
            SampleFormat::Cs16 => "cs16",
            SampleFormat::Cf32 => "cf32",
        }
    }

    /// SigMF `core:datatype` string.
    pub fn sigmf_datatype(self) -> &'static str {
        match self {
            SampleFormat::Cu8 => "cu8",
            SampleFormat::Cs8 => "ci8",
            SampleFormat::Cs16 => "ci16_le",
            SampleFormat::Cf32 => "cf32_le",
        }
    }

    /// Decode raw bytes (length must be a multiple of the sample size; any
    /// trailing partial sample is ignored), appending to `out`.
    pub fn decode(self, bytes: &[u8], out: &mut Vec<C32>) {
        let bps = self.bytes_per_sample();
        out.reserve(bytes.len() / bps);
        for c in bytes.chunks_exact(bps) {
            out.push(match self {
                SampleFormat::Cu8 => C32::new((c[0] as f32 - 127.5) / 127.5, (c[1] as f32 - 127.5) / 127.5),
                SampleFormat::Cs8 => C32::new(c[0] as i8 as f32 / 127.0, c[1] as i8 as f32 / 127.0),
                SampleFormat::Cs16 => C32::new(
                    i16::from_le_bytes([c[0], c[1]]) as f32 / 32767.0,
                    i16::from_le_bytes([c[2], c[3]]) as f32 / 32767.0,
                ),
                SampleFormat::Cf32 => C32::new(
                    f32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                    f32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                ),
            });
        }
    }

    /// Encode samples (clipped to full scale for integer formats), appending to `out`.
    pub fn encode(self, samples: &[C32], out: &mut Vec<u8>) {
        out.reserve(samples.len() * self.bytes_per_sample());
        for s in samples {
            match self {
                SampleFormat::Cu8 => {
                    let q = |v: f32| (v.clamp(-1.0, 1.0) * 127.5 + 127.5).round().clamp(0.0, 255.0) as u8;
                    out.extend_from_slice(&[q(s.re), q(s.im)]);
                }
                SampleFormat::Cs8 => {
                    let q = |v: f32| (v.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8;
                    out.extend_from_slice(&[q(s.re), q(s.im)]);
                }
                SampleFormat::Cs16 => {
                    out.extend_from_slice(&f32_to_i16(s.re).to_le_bytes());
                    out.extend_from_slice(&f32_to_i16(s.im).to_le_bytes());
                }
                SampleFormat::Cf32 => {
                    out.extend_from_slice(&s.re.to_le_bytes());
                    out.extend_from_slice(&s.im.to_le_bytes());
                }
            }
        }
    }
}

impl fmt::Display for SampleFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for SampleFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "cu8" => Ok(SampleFormat::Cu8),
            "cs8" | "ci8" => Ok(SampleFormat::Cs8),
            "cs16" | "ci16" | "ci16_le" => Ok(SampleFormat::Cs16),
            "cf32" | "cf32_le" | "fc32" => Ok(SampleFormat::Cf32),
            other => Err(format!("unknown sample format '{other}'")),
        }
    }
}

#[inline]
fn f32_to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// Interleaved `cs16` (i16 pairs) -> complex.
pub fn cs16_to_c32(input: &[i16], out: &mut [C32]) {
    for (c, o) in input.as_chunks::<2>().0.iter().zip(out.iter_mut()) {
        *o = C32::new(c[0] as f32 / 32767.0, c[1] as f32 / 32767.0);
    }
}

/// Complex -> interleaved `cs16` (clipped).
pub fn c32_to_cs16(input: &[C32], out: &mut [i16]) {
    for (s, o) in input.iter().zip(out.as_chunks_mut::<2>().0.iter_mut()) {
        o[0] = f32_to_i16(s.re);
        o[1] = f32_to_i16(s.im);
    }
}

/// Interleaved `cf32` (f32 pairs) -> complex.
pub fn cf32_to_c32(input: &[f32], out: &mut [C32]) {
    for (c, o) in input.as_chunks::<2>().0.iter().zip(out.iter_mut()) {
        *o = C32::new(c[0], c[1]);
    }
}

/// Complex -> interleaved `cf32`.
pub fn c32_to_cf32(input: &[C32], out: &mut [f32]) {
    for (s, o) in input.iter().zip(out.as_chunks_mut::<2>().0.iter_mut()) {
        o[0] = s.re;
        o[1] = s.im;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_round_trip() {
        let x = vec![C32::new(0.5, -0.25), C32::new(-1.0, 1.0), C32::new(2.0, -3.0)];
        for f in [SampleFormat::Cu8, SampleFormat::Cs8, SampleFormat::Cs16, SampleFormat::Cf32] {
            let mut b = Vec::new();
            f.encode(&x, &mut b);
            assert_eq!(b.len(), 3 * f.bytes_per_sample());
            let mut y = Vec::new();
            f.decode(&b, &mut y);
            let tol = match f {
                SampleFormat::Cf32 => 0.0,
                SampleFormat::Cs16 => 1e-4,
                _ => 1e-2,
            };
            assert!((y[0] - x[0]).abs() <= tol, "{f}");
            assert!((y[1] - x[1]).abs() <= tol, "{f}");
            if f != SampleFormat::Cf32 {
                assert!((y[2] - C32::new(1.0, -1.0)).abs() <= tol, "{f} clip");
            } else {
                assert_eq!(y[2], x[2]);
            }
            assert_eq!(f.name().parse::<SampleFormat>().unwrap(), f);
        }
        let mut i = [0i16; 6];
        c32_to_cs16(&x, &mut i);
        assert_eq!(&i[..2], &[16384, -8192]);
        let mut back = [C32::ZERO; 3];
        cs16_to_c32(&i, &mut back);
        assert!((back[1] - C32::new(-1.0, 1.0)).abs() < 1e-6);
        let mut f = [0f32; 6];
        c32_to_cf32(&x, &mut f);
        let mut bc = [C32::ZERO; 3];
        cf32_to_c32(&f, &mut bc);
        assert_eq!(bc.to_vec(), x);
    }
}
