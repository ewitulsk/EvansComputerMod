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

const PCM_ONLY: &str = "only 8- or 16-bit PCM WAVs play (ffmpeg -i in -ac 1 -ar 8000 -c:a pcm_s16le out.wav)";

/// Where a WAV's PCM data is: (rate, bits, channels, data offset, data length).
struct PcmWav {
    rate: u32,
    bits: u8,
    channels: u8,
    offset: usize,
    len: usize,
}

/// Parse a RIFF/WAVE file's `fmt `/`data` chunks: `Ok(None)` if `bytes`
/// isn't RIFF/WAVE at all, an error naming the format if it is a WAV but not
/// 8/16-bit PCM in one or two channels (24/32-bit, float, compressed, or a
/// WAVE_FORMAT_EXTENSIBLE whose sub-format isn't such PCM).
fn parse_pcm_wav(bytes: &[u8]) -> Result<Option<PcmWav>, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Ok(None);
    }
    let u16_at = |o: usize| bytes.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| bytes.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let bad = || "not a valid WAV file (broken fmt chunk)".to_string();
    let mut pos = 12;
    let mut fmt: Option<(u32, u8, u8)> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32_at(pos + 4).ok_or_else(bad)? as usize;
        let body = pos + 8;
        if id == b"fmt " {
            let mut tag = u16_at(body).ok_or_else(bad)?;
            let channels = u16_at(body + 2).ok_or_else(bad)?;
            let rate = u32_at(body + 4).ok_or_else(bad)?;
            let bits = u16_at(body + 14).ok_or_else(bad)?;
            if tag == 0xFFFE {
                // WAVE_FORMAT_EXTENSIBLE: the real format is the first two
                // bytes of the sub-format GUID (after cbSize, valid bits, mask).
                tag = u16_at(body + 24).ok_or_else(bad)?;
            }
            match tag {
                1 => {}
                3 => return Err(format!("{bits}-bit float WAV; {PCM_ONLY}")),
                t => return Err(format!("WAV format 0x{t:04x} (compressed or not PCM); {PCM_ONLY}")),
            }
            if bits != 8 && bits != 16 {
                return Err(format!("{bits}-bit PCM WAV; {PCM_ONLY}"));
            }
            if channels != 1 && channels != 2 {
                return Err(format!("{channels}-channel WAV; only mono or stereo play"));
            }
            if rate == 0 {
                return Err(bad());
            }
            fmt = Some((rate, bits as u8, channels as u8));
        } else if id == b"data" {
            let (rate, bits, channels) = fmt.ok_or_else(|| "not a valid WAV file (data before fmt)".to_string())?;
            let len = len.min(bytes.len() - body);
            return Ok(Some(PcmWav { rate, bits, channels, offset: body, len }));
        }
        pos = body.saturating_add(len + (len & 1));
    }
    Err("not a valid WAV file (no fmt/data chunk)".into())
}

/// Read a WAV file (PCM 8/16-bit, mono or stereo mixed down; a
/// WAVE_FORMAT_EXTENSIBLE header is fine if it holds such PCM). Other WAVs
/// (24-bit, float, compressed) are an error rather than noise. Input that
/// isn't RIFF/WAVE is taken as headerless 16-bit mono PCM at `raw_rate`.
pub fn read_audio(bytes: &[u8], raw_rate: u32) -> Result<Audio, String> {
    match parse_pcm_wav(bytes)? {
        Some(w) => Ok(Audio { rate: w.rate, samples: from_pcm(&bytes[w.offset..w.offset + w.len], w.bits, w.channels) }),
        None => Ok(Audio { rate: raw_rate, samples: from_pcm(bytes, 16, 1) }),
    }
}

/// [`read_audio`] for a file named `path`: headerless PCM is accepted only
/// from a `.pcm` / `.raw` file, so anything else that isn't a WAV (an MP3, a
/// text file in a playlist) is an error instead of noise.
pub fn read_audio_file(path: &str, bytes: &[u8], raw_rate: u32) -> Result<Audio, String> {
    let l = path.to_ascii_lowercase();
    let raw_ok = l.ends_with(".pcm") || l.ends_with(".raw");
    if !raw_ok && parse_pcm_wav(bytes)?.is_none() {
        return Err("not a WAV file (no RIFF/WAVE header); headerless 16-bit PCM must be named .pcm or .raw".into());
    }
    read_audio(bytes, raw_rate)
}

/// A 16-bit mono WAV written as audio arrives: every [`WavWriter::append`]
/// adds the samples and rewrites the header's sizes, so the file on disk is
/// always a complete, valid WAV (a program killed mid-recording leaves
/// everything up to its last append).
pub struct WavWriter {
    file: std::fs::File,
    rate: u32,
    samples: u32,
}

impl WavWriter {
    pub fn create(path: &str, rate: u32) -> std::io::Result<WavWriter> {
        use std::io::Write;
        let mut file = std::fs::File::create(path)?;
        file.write_all(&wav_header(rate, 0))?;
        Ok(WavWriter { file, rate, samples: 0 })
    }

    pub fn append(&mut self, x: &[f32]) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        if x.is_empty() {
            return Ok(());
        }
        // Stop growing before the 4 GiB RIFF limit.
        let room = (u32::MAX - 44) / 2 - self.samples;
        let x = &x[..x.len().min(room as usize)];
        self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&to_pcm16(x))?;
        self.samples += x.len() as u32;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&wav_header(self.rate, self.samples))?;
        self.file.flush()
    }

    /// Samples written so far.
    pub fn samples(&self) -> u32 {
        self.samples
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
        assert_eq!(read_audio(&f, 1).unwrap().samples.len(), 10);
        assert_eq!(wav_title(b"not a wav"), None);
    }

    #[test]
    fn wav_round_trip() {
        let x: Vec<f32> = (0..100).map(|i| ((i as f32) * 0.1).sin() * 0.5).collect();
        let f = wav_file(8000, &x);
        assert_eq!(f.len(), 44 + 200);
        let a = read_audio(&f, 1).unwrap();
        assert_eq!(a.rate, 8000);
        assert_eq!(a.samples.len(), 100);
        for (p, q) in a.samples.iter().zip(&x) {
            assert!((p - q).abs() < 1e-4);
        }
        let raw = read_audio(&to_pcm16(&x), 22050).unwrap();
        assert_eq!(raw.rate, 22050);
        // 8-bit stereo mixes down
        assert_eq!(from_pcm(&[255, 1, 128, 128], 8, 2), vec![0.0, 0.0]);
    }

    /// A WAVE_FORMAT_EXTENSIBLE header around 16-bit PCM plays; extensible
    /// float, plain float, 24-bit and non-WAV files are errors, not noise.
    #[test]
    fn unsupported_wavs_are_rejected() {
        let x: Vec<f32> = (0..64).map(|i| ((i as f32) * 0.3).sin() * 0.5).collect();
        let pcm = to_pcm16(&x);
        let ext = |sub: u16, bits: u16| {
            let mut v = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
            v.extend_from_slice(&40u32.to_le_bytes());
            v.extend_from_slice(&0xFFFEu16.to_le_bytes());
            v.extend_from_slice(&1u16.to_le_bytes());
            v.extend_from_slice(&8000u32.to_le_bytes());
            v.extend_from_slice(&(8000 * bits as u32 / 8).to_le_bytes());
            v.extend_from_slice(&(bits / 8).to_le_bytes());
            v.extend_from_slice(&bits.to_le_bytes());
            v.extend_from_slice(&22u16.to_le_bytes()); // cbSize
            v.extend_from_slice(&bits.to_le_bytes()); // valid bits
            v.extend_from_slice(&4u32.to_le_bytes()); // channel mask
            v.extend_from_slice(&sub.to_le_bytes());
            v.extend_from_slice(&[0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71]);
            v.extend_from_slice(b"data");
            v.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
            v.extend_from_slice(&pcm);
            v
        };
        let a = read_audio_file("x.wav", &ext(1, 16), 1).unwrap();
        assert_eq!((a.rate, a.samples.len()), (8000, 64));
        assert!((a.samples[10] - x[10]).abs() < 1e-3);
        let e = read_audio_file("x.wav", &ext(3, 32), 1).unwrap_err();
        assert!(e.contains("32-bit float"), "{e}");
        let mut f = wav_file(8000, &x);
        f[20..22].copy_from_slice(&3u16.to_le_bytes());
        assert!(read_audio(&f, 1).unwrap_err().contains("float"));
        let mut d = wav_file(8000, &x);
        d[34..36].copy_from_slice(&24u16.to_le_bytes());
        assert!(read_audio(&d, 1).unwrap_err().contains("24-bit PCM"));
        let mut c = wav_file(8000, &x);
        c[20..22].copy_from_slice(&0x55u16.to_le_bytes()); // MP3-in-WAV
        assert!(read_audio(&c, 1).unwrap_err().contains("0x0055"));
        // Headerless data: fine from a .pcm/.raw file, an error from anything else.
        assert_eq!(read_audio_file("song.PCM", &pcm, 22050).unwrap().rate, 22050);
        assert!(read_audio_file("song.raw", &pcm, 8000).is_ok());
        let e = read_audio_file("song.mp3", b"ID3\x04 not a wav", 8000).unwrap_err();
        assert!(e.contains("not a WAV"), "{e}");
        assert!(read_audio_file("song.wav", &pcm, 8000).is_err());
    }

    /// The streaming writer keeps a valid WAV on disk after every append.
    #[test]
    fn wav_writer_is_valid_after_every_append() {
        let dir = std::env::temp_dir().join(format!("ecm-radio-wavw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rec.wav").to_string_lossy().into_owned();
        let mut w = WavWriter::create(&path, 24_000).unwrap();
        let empty = read_audio(&std::fs::read(&path).unwrap(), 1).unwrap();
        assert_eq!((empty.rate, empty.samples.len()), (24_000, 0));
        let x: Vec<f32> = (0..1000).map(|i| ((i as f32) * 0.05).sin() * 0.5).collect();
        for k in 1..=3 {
            w.append(&x).unwrap();
            let b = std::fs::read(&path).unwrap();
            assert_eq!(b.len(), 44 + 2000 * k);
            assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize, b.len() - 8);
            assert_eq!(read_audio(&b, 1).unwrap().samples.len(), 1000 * k);
        }
        assert_eq!(w.samples(), 3000);
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
