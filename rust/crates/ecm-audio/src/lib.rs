//! Speaker audio for WASI programs: the `/dev/audio` device.
//!
//! A Speaker next to the computer is a sound card. Programs write PCM to
//! `/dev/audio` (the first speaker) or `/dev/audio.<side>` (e.g.
//! `/dev/audio.left`), and set it up by writing text commands to the matching
//! `/dev/audioctl`:
//!
//! ```text
//! rate 8000..48000   bits 8|16   channels 1|2   volume 0..100   latency 20..1000   flush
//! ```
//!
//! Reading `/dev/audioctl` gives the settings plus `buffered_ms` and
//! `underruns`. A write waits while more than `latency` ms of audio is queued,
//! so a program that just writes is paced by the audio clock (non-blocking
//! mode returns what fitted instead).
//!
//! ```ignore
//! let mut dev = ecm_audio::Device::open(None)?;
//! dev.set_format(32768, 16, 1)?;
//! dev.write_samples(&samples)?;          // i16, waits for room
//! ```

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};

/// An open speaker.
pub struct Device {
    pcm: File,
    ctl: File,
    name: String,
}

impl Device {
    /// Open the speaker attached as `side` (`"left"`, `"left_bay_1"`, ...), or the first one.
    pub fn open(side: Option<&str>) -> io::Result<Device> {
        let suffix = side.map(|s| format!(".{}", s)).unwrap_or_default();
        let pcm = OpenOptions::new().write(true).open(format!("/dev/audio{}", suffix))?;
        let ctl = OpenOptions::new().read(true).write(true).open(format!("/dev/audioctl{}", suffix))?;
        Ok(Device { pcm, ctl, name: format!("/dev/audio{}", suffix) })
    }

    /// Device path, e.g. `/dev/audio.left`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Send one `/dev/audioctl` command, e.g. `"volume 50"`.
    pub fn control(&mut self, cmd: &str) -> io::Result<()> {
        self.ctl.write_all(format!("{}\n", cmd).as_bytes())
    }

    /// Sample rate (Hz), bits per sample (8 = unsigned, 16 = signed LE) and channels (1, 2).
    pub fn set_format(&mut self, rate: u32, bits: u8, channels: u8) -> io::Result<()> {
        self.control(&format!("rate {}", rate))?;
        self.control(&format!("bits {}", bits))?;
        self.control(&format!("channels {}", channels))
    }

    pub fn set_volume(&mut self, volume: u8) -> io::Result<()> {
        self.control(&format!("volume {}", volume))
    }

    /// How much audio may queue before writes wait (ms).
    pub fn set_latency_ms(&mut self, ms: u32) -> io::Result<()> {
        self.control(&format!("latency {}", ms))
    }

    /// Drop whatever is queued.
    pub fn flush_queue(&mut self) -> io::Result<()> {
        self.control("flush")
    }

    /// Raw PCM bytes in the current format; waits for room.
    pub fn write_bytes(&mut self, pcm: &[u8]) -> io::Result<()> {
        self.pcm.write_all(pcm)
    }

    /// 16-bit samples (set `bits 16` first); waits for room.
    pub fn write_samples(&mut self, samples: &[i16]) -> io::Result<()> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        self.pcm.write_all(&bytes)
    }

    /// The settings and counters, e.g. `[("rate", 48000), ("buffered_ms", 37), ...]`.
    pub fn status(&mut self) -> io::Result<Vec<(String, i64)>> {
        let mut text = String::new();
        self.ctl.read_to_string(&mut text)?;
        Ok(text
            .lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                let k = it.next()?;
                let v = it.next()?.parse().ok()?;
                Some((k.to_string(), v))
            })
            .collect())
    }

    /// One counter from [`status`](Self::status), e.g. `"buffered_ms"`.
    pub fn stat(&mut self, key: &str) -> io::Result<Option<i64>> {
        Ok(self.status()?.into_iter().find(|(k, _)| k == key).map(|(_, v)| v))
    }
}

/// A WAV file's PCM format and where its samples start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavInfo {
    pub rate: u32,
    pub bits: u8,
    pub channels: u8,
    pub data_offset: usize,
    pub data_len: usize,
}

/// Parse a RIFF/WAVE header (PCM, 8 or 16 bits, 1 or 2 channels).
pub fn parse_wav(bytes: &[u8]) -> Option<WavInfo> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let u16_at = |o: usize| -> Option<u16> { Some(u16::from_le_bytes(bytes.get(o..o + 2)?.try_into().ok()?)) };
    let u32_at = |o: usize| -> Option<u32> { Some(u32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?)) };
    let mut pos = 12;
    let mut fmt: Option<(u32, u8, u8)> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32_at(pos + 4)? as usize;
        let body = pos + 8;
        if id == b"fmt " {
            if u16_at(body)? != 1 {
                return None; // not plain PCM
            }
            let channels = u16_at(body + 2)?;
            let rate = u32_at(body + 4)?;
            let bits = u16_at(body + 14)?;
            if !(channels == 1 || channels == 2) || !(bits == 8 || bits == 16) {
                return None;
            }
            fmt = Some((rate, bits as u8, channels as u8));
        } else if id == b"data" {
            let (rate, bits, channels) = fmt?;
            let data_len = len.min(bytes.len().saturating_sub(body));
            return Some(WavInfo { rate, bits, channels, data_offset: body, data_len });
        }
        pos = body + len + (len & 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(rate: u32, bits: u16, channels: u16, data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * channels as u32 * bits as u32 / 8).to_le_bytes());
        v.extend_from_slice(&(channels * bits / 8).to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn parses_a_pcm_wav() {
        let w = wav(22050, 16, 2, &[1, 2, 3, 4]);
        assert_eq!(
            parse_wav(&w),
            Some(WavInfo { rate: 22050, bits: 16, channels: 2, data_offset: 44, data_len: 4 })
        );
    }

    #[test]
    fn rejects_garbage_and_truncation() {
        assert_eq!(parse_wav(b"not a wav"), None);
        let w = wav(8000, 8, 1, &[0; 10]);
        assert_eq!(parse_wav(&w[..30]), None);
        let mut bad = wav(8000, 24, 1, &[0; 3]);
        assert_eq!(parse_wav(&bad), None);
        bad[20] = 3; // float format
        assert_eq!(parse_wav(&bad), None);
    }
}
