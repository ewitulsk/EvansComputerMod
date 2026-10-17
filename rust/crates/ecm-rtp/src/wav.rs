//! Minimal RIFF WAV for voicemail and voice notes: write 16-bit mono PCM at any
//! rate, read PCM WAVs (16-bit or 8-bit, mono or stereo, any rate; the header parser
//! is `ecm_audio::parse_wav`) and bring them to 16 kHz mono by linear interpolation.

/// Decoded WAV audio: mono samples at `rate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wav {
    pub rate: u32,
    pub samples: Vec<i16>,
}

/// A 16-bit mono WAV file of `samples` at `rate`.
pub fn write(rate: u32, samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut v = ecm_audio::wav_header(rate, 16, 1, data_len);
    v.reserve(samples.len() * 2);
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

/// Read a PCM WAV at its own rate (stereo is mixed down, 8-bit widened); None if it isn't one.
pub fn read(bytes: &[u8]) -> Option<Wav> {
    let info = ecm_audio::parse_wav(bytes)?;
    if info.rate == 0 {
        return None;
    }
    let data = bytes.get(info.data_offset..info.data_offset + info.data_len)?;
    let width = info.bits as usize / 8 * info.channels as usize;
    let sample = |b: &[u8]| -> i32 {
        if info.bits == 16 {
            i16::from_le_bytes([b[0], b[1]]) as i32
        } else {
            (b[0] as i32 - 128) << 8
        }
    };
    let step = info.bits as usize / 8;
    let samples = data
        .chunks_exact(width)
        .map(|f| {
            let sum: i32 = (0..info.channels as usize)
                .map(|c| sample(&f[c * step..]))
                .sum();
            (sum / info.channels as i32) as i16
        })
        .collect();
    Some(Wav {
        rate: info.rate,
        samples,
    })
}

/// Read a PCM WAV as 16 kHz mono ([`crate::RATE`]).
pub fn read_16k(bytes: &[u8]) -> Option<Vec<i16>> {
    let w = read(bytes)?;
    Some(resample(&w.samples, w.rate, crate::RATE))
}

/// Resample by linear interpolation.
pub fn resample(x: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to || x.is_empty() || from == 0 || to == 0 {
        return x.to_vec();
    }
    let n = (x.len() as u64 * to as u64 / from as u64) as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * from as f64 / to as f64;
            let k = pos.floor() as usize;
            let frac = pos - k as f64;
            let a = x[k.min(x.len() - 1)] as f64;
            let b = x[(k + 1).min(x.len() - 1)] as f64;
            (a + (b - a) * frac).round() as i16
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{power_at, sine};

    #[test]
    fn write_then_read_round_trips() {
        let pcm: Vec<i16> = vec![0, 1, -1, 32767, -32768, 1234];
        let f = write(16000, &pcm);
        assert_eq!(f.len(), 44 + 12);
        assert_eq!(&f[..4], b"RIFF");
        assert_eq!(
            read(&f),
            Some(Wav {
                rate: 16000,
                samples: pcm.clone()
            })
        );
        assert_eq!(read_16k(&f), Some(pcm));
        assert_eq!(
            read(&write(8000, &[])),
            Some(Wav {
                rate: 8000,
                samples: vec![]
            })
        );
    }

    #[test]
    fn other_rates_come_to_16k() {
        // 8 kHz, 1 s of 400 Hz: 16000 samples at 16 kHz, still 400 Hz.
        let tone8: Vec<i16> = (0..8000)
            .map(|i| ((i as f64 * 400.0 * std::f64::consts::TAU / 8000.0).sin() * 10000.0) as i16)
            .collect();
        let up = read_16k(&write(8000, &tone8)).unwrap();
        assert_eq!(up.len(), 16000);
        assert!(power_at(&up, 400.0) > power_at(&sine(16000, 400.0, 10000.0), 400.0) * 0.8);
        // 48 kHz down to 16 kHz.
        let tone48: Vec<i16> = (0..4800)
            .map(|i| ((i as f64 * 300.0 * std::f64::consts::TAU / 48000.0).sin() * 10000.0) as i16)
            .collect();
        let down = read_16k(&write(48000, &tone48)).unwrap();
        assert_eq!(down.len(), 1600);
        assert_eq!(resample(&[0, 100], 1, 2), vec![0, 50, 100, 100]);
    }

    #[test]
    fn stereo_and_8_bit_are_read() {
        // Stereo 16-bit: (100, 300) -> 200.
        let mut st = ecm_audio::wav_header(22050, 16, 2, 4);
        st.extend_from_slice(&100i16.to_le_bytes());
        st.extend_from_slice(&300i16.to_le_bytes());
        assert_eq!(
            read(&st).unwrap(),
            Wav {
                rate: 22050,
                samples: vec![200]
            }
        );
        // 8-bit unsigned: 128 is silence, 255 near full scale.
        let mut b8 = ecm_audio::wav_header(8000, 8, 1, 2);
        b8.extend_from_slice(&[128, 255]);
        assert_eq!(read(&b8).unwrap().samples, vec![0, 127 << 8]);
    }

    #[test]
    fn garbage_is_refused() {
        assert_eq!(read(b""), None);
        assert_eq!(read(b"RIFF\0\0\0\0WAVE"), None);
        let f = write(16000, &[1, 2, 3]);
        assert_eq!(read(&f[..20]), None);
        let mut bad = f.clone();
        bad[24..28].copy_from_slice(&0u32.to_le_bytes()); // rate 0
        assert_eq!(read(&bad), None);
        // A truncated data chunk reads what is there.
        assert_eq!(read(&f[..f.len() - 2]).unwrap().samples, vec![1, 2]);
    }
}
