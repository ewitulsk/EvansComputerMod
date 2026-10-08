//! PCM and WAV conversion for audio sources and sinks.

/// Decoded audio: mono samples in -1..1 at `rate`.
#[derive(Clone, Debug, PartialEq)]
pub struct Audio {
    pub rate: u32,
    pub samples: Vec<f32>,
}

/// f32 (-1..1, clipped) -> 16-bit signed little-endian PCM.
pub fn to_pcm16(x: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(x.len() * 2);
    for &v in x {
        let s = (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// PCM (8-bit unsigned or 16-bit signed LE, interleaved channels) -> mono f32.
pub fn from_pcm(data: &[u8], bits: u8, channels: u8) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    let bps = if bits == 8 { 1 } else { 2 };
    let frame = bps * ch;
    data.chunks_exact(frame)
        .map(|f| {
            let mut sum = 0.0;
            for c in 0..ch {
                sum += if bits == 8 {
                    (f[c] as f32 - 128.0) / 128.0
                } else {
                    i16::from_le_bytes([f[2 * c], f[2 * c + 1]]) as f32 / 32768.0
                };
            }
            sum / ch as f32
        })
        .collect()
}

/// RIFF/WAVE header for 16-bit PCM mono with `samples` samples.
pub fn wav_header(rate: u32, samples: u32) -> Vec<u8> {
    let data_len = samples * 2;
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data_len).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes()); // PCM
    h.extend_from_slice(&1u16.to_le_bytes()); // mono
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * 2).to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data_len.to_le_bytes());
    h
}

/// A complete 16-bit mono WAV file.
pub fn wav_file(rate: u32, x: &[f32]) -> Vec<u8> {
    let mut v = wav_header(rate, x.len() as u32);
    v.extend(to_pcm16(x));
    v
}

/// Read a WAV file (PCM 8/16-bit, mono or stereo mixed down). Headerless
/// input is treated as 16-bit mono PCM at `raw_rate`.
pub fn read_audio(bytes: &[u8], raw_rate: u32) -> Audio {
    match ecm_audio::parse_wav(bytes) {
        Some(w) => {
            let end = (w.data_offset + w.data_len).min(bytes.len());
            Audio { rate: w.rate, samples: from_pcm(&bytes[w.data_offset..end], w.bits, w.channels) }
        }
        None => Audio { rate: raw_rate, samples: from_pcm(bytes, 16, 1) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_round_trip() {
        let x: Vec<f32> = (0..100).map(|i| ((i as f32) * 0.1).sin() * 0.5).collect();
        let f = wav_file(8000, &x);
        assert_eq!(f.len(), 44 + 200);
        let a = read_audio(&f, 1);
        assert_eq!(a.rate, 8000);
        assert_eq!(a.samples.len(), 100);
        for (p, q) in a.samples.iter().zip(&x) {
            assert!((p - q).abs() < 1e-4);
        }
        let raw = read_audio(&to_pcm16(&x), 22050);
        assert_eq!(raw.rate, 22050);
        // 8-bit stereo mixes down
        assert_eq!(from_pcm(&[255, 1, 128, 128], 8, 2), vec![0.0, 0.0]);
    }
}
