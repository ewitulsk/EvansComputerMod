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

/// The title in a WAV file's `LIST`/`INFO` chunk (`INAM`, which ffmpeg and
/// most editors write from a "title" tag), if it has one.
pub fn wav_title(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let u32_at = |o: usize| -> Option<usize> { Some(u32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?) as usize) };
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let len = u32_at(pos + 4)?;
        let body = pos + 8;
        let end = body.saturating_add(len).min(bytes.len());
        if &bytes[pos..pos + 4] == b"LIST" && bytes.get(body..body + 4) == Some(b"INFO") {
            let mut p = body + 4;
            while p + 8 <= end {
                let n = u32_at(p + 4)?;
                let v = p + 8;
                if &bytes[p..p + 4] == b"INAM" {
                    let raw = bytes.get(v..(v + n).min(end))?;
                    let text = String::from_utf8_lossy(raw).trim_end_matches('\0').trim().to_string();
                    return if text.is_empty() { None } else { Some(text) };
                }
                p = v + n + (n & 1);
            }
        }
        pos = body.saturating_add(len + (len & 1));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_title_from_info_chunk() {
        let mut f = wav_file(8000, &[0.0; 10]);
        assert_eq!(wav_title(&f), None);
        // RIFF size is not checked; append LIST/INFO/INAM "Aria" (odd length 5 with NUL, padded).
        let mut list = b"INFO".to_vec();
        list.extend_from_slice(b"ISFT");
        list.extend_from_slice(&3u32.to_le_bytes());
        list.extend_from_slice(b"ab\0\0");
        list.extend_from_slice(b"INAM");
        list.extend_from_slice(&5u32.to_le_bytes());
        list.extend_from_slice(b"Aria\0\0");
        f.extend_from_slice(b"LIST");
        f.extend_from_slice(&(list.len() as u32).to_le_bytes());
        f.extend_from_slice(&list);
        assert_eq!(wav_title(&f).as_deref(), Some("Aria"));
        assert_eq!(read_audio(&f, 1).samples.len(), 10);
        assert_eq!(wav_title(b"not a wav"), None);
    }

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
