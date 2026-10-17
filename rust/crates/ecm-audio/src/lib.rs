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
//!
//! A Microphone (or an ePhone's built-in one) is the capture side: programs
//! read PCM from `/dev/mic` (or `/dev/mic.<side>`) and set it up through
//! `/dev/micctl` (`rate`, `bits`, `channels`, `gain 0..100`, `flush`). A read
//! waits until the samples asked for have been captured; a program that reads
//! too rarely loses audio and counts an `overrun`.
//!
//! ```ignore
//! let mut mic = ecm_audio::Mic::open(None)?;
//! mic.set_format(16000, 16, 1)?;
//! let samples = mic.read_samples(1600)?; // 100 ms
//! ```
//!
//! `Device::open(None)` and `Mic::open(None)` use the default devices chosen
//! in Settings > Sound ([`SOUND_CONF`]: `output <side>` / `input <side>`
//! lines), else the first speaker / microphone.

pub mod adpcm;
pub mod mixer;

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};

/// Where Settings > Sound keeps the default devices: `output <side>` and `input <side>` lines.
pub const SOUND_CONF: &str = "/etc/sound.conf";

/// The default device named in [`SOUND_CONF`] for `key` ("output" or "input"), if any.
pub fn default_device(key: &str) -> Option<String> {
    let text = std::fs::read_to_string(SOUND_CONF).ok()?;
    parse_sound_conf(&text, key)
}

/// The value of `key` in sound.conf text.
pub fn parse_sound_conf(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        (it.next()? == key).then(|| it.next().map(str::to_string)).flatten()
    })
}

/// sound.conf text with `key` set to `value` (or removed when `value` is None), other lines kept.
pub fn set_sound_conf(text: &str, key: &str, value: Option<&str>) -> String {
    let mut out: Vec<String> = text
        .lines()
        .filter(|l| l.split_whitespace().next() != Some(key))
        .map(str::to_string)
        .collect();
    if let Some(v) = value {
        out.push(format!("{key} {v}"));
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// `side` if given, else the default from sound.conf: the device suffix (".left") or "" for the first.
fn suffix_for(side: Option<&str>, key: &str) -> String {
    match side.map(str::to_string).or_else(|| default_device(key)) {
        Some(s) if !s.is_empty() => format!(".{}", s),
        _ => String::new(),
    }
}

/// Parse `key value` status lines.
fn parse_status(text: &str) -> Vec<(String, i64)> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let k = it.next()?;
            let v = it.next()?.parse().ok()?;
            Some((k.to_string(), v))
        })
        .collect()
}

/// Who a speaker stream is for (`/dev/audioctl role`): while a stream carries sound,
/// every stream with a lower role is ducked (`Media < Voice < Phone`). A Discord call is
/// `Voice`; cellular calls and ringtones are `Phone`; music and everything else `Media`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Role {
    #[default]
    Media,
    Voice,
    Phone,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Media => "media",
            Role::Voice => "voice",
            Role::Phone => "phone",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "media" => Some(Role::Media),
            "voice" => Some(Role::Voice),
            "phone" => Some(Role::Phone),
            _ => None,
        }
    }
}

/// The name PLATFORM_API.md uses for an open speaker stream.
pub type Speaker = Device;

/// An open speaker: one stream on its mixer (each open is its own stream, with its own
/// format, clock and role).
pub struct Device {
    pcm: File,
    ctl: File,
    name: String,
}

impl Device {
    /// Open the speaker attached as `side` (`"left"`, `"left_bay_1"`, ...), or the first one.
    pub fn open(side: Option<&str>) -> io::Result<Device> {
        let mut suffix = suffix_for(side, "output");
        if side.is_none() && !suffix.is_empty() && std::fs::metadata(format!("/dev/audio{}", suffix)).is_err() {
            suffix.clear(); // the default speaker is gone: use the first one
        }
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

    /// This stream's role (ducking order `Media < Voice < Phone`).
    pub fn set_role(&mut self, role: Role) -> io::Result<()> {
        self.control(&format!("role {}", role.as_str()))
    }

    /// A louder, higher-role stream lowers this one right now.
    pub fn ducked(&mut self) -> bool {
        self.stat("ducked").ok().flatten() == Some(1)
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

/// An open microphone: the capture side.
pub struct Mic {
    pcm: File,
    ctl: File,
    name: String,
    bits: u8,
    channels: u8,
}

impl Mic {
    /// Open the microphone attached as `side`, or the default (Settings > Sound), or the first one.
    pub fn open(side: Option<&str>) -> io::Result<Mic> {
        let mut suffix = suffix_for(side, "input");
        if side.is_none() && !suffix.is_empty() && std::fs::metadata(format!("/dev/mic{}", suffix)).is_err() {
            suffix.clear();
        }
        let pcm = OpenOptions::new().read(true).open(format!("/dev/mic{}", suffix))?;
        let ctl = OpenOptions::new().read(true).write(true).open(format!("/dev/micctl{}", suffix))?;
        let mut m = Mic { pcm, ctl, name: format!("/dev/mic{}", suffix), bits: 16, channels: 1 };
        if let Ok(st) = m.status() {
            for (k, v) in st {
                match k.as_str() {
                    "bits" => m.bits = v as u8,
                    "channels" => m.channels = v as u8,
                    _ => {}
                }
            }
        }
        Ok(m)
    }

    /// Device path, e.g. `/dev/mic.top`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Send one `/dev/micctl` command, e.g. `"gain 60"`.
    pub fn control(&mut self, cmd: &str) -> io::Result<()> {
        self.ctl.write_all(format!("{}\n", cmd).as_bytes())
    }

    /// Sample rate (Hz), bits per sample (8 = unsigned, 16 = signed LE) and channels (1, 2).
    pub fn set_format(&mut self, rate: u32, bits: u8, channels: u8) -> io::Result<()> {
        self.control(&format!("rate {}", rate))?;
        self.control(&format!("bits {}", bits))?;
        self.control(&format!("channels {}", channels))?;
        self.bits = bits;
        self.channels = channels;
        Ok(())
    }

    /// Input volume 0..100 (50 = the capsule's own level, 0 = mute).
    pub fn set_gain(&mut self, gain: u8) -> io::Result<()> {
        self.control(&format!("gain {}", gain))
    }

    /// Drop what was captured and not read.
    pub fn flush_queue(&mut self) -> io::Result<()> {
        self.control("flush")
    }

    /// Raw PCM in the current format: waits until `buf` (whole frames, at most half a second) is captured.
    pub fn read_bytes(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.pcm.read(buf)
    }

    /// `n` mono 16-bit samples (format must be 16-bit mono); waits for them.
    pub fn read_samples(&mut self, n: usize) -> io::Result<Vec<i16>> {
        let frame = (self.bits as usize / 8) * self.channels as usize;
        let mut buf = vec![0u8; n * frame];
        let mut got = 0;
        while got < buf.len() {
            let k = self.pcm.read(&mut buf[got..])?;
            if k == 0 {
                break;
            }
            got += k;
        }
        buf.truncate(got);
        Ok(match (self.bits, self.channels) {
            (16, 1) => buf.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect(),
            (16, _) => buf.chunks_exact(2 * self.channels as usize).map(|b| i16::from_le_bytes([b[0], b[1]])).collect(),
            (_, c) => buf.chunks_exact(c as usize).map(|b| ((b[0] as i16) - 128) << 8).collect(),
        })
    }

    /// The settings and counters (`rate`, `gain`, `buffered_ms`, `overruns`, `level_db`, `listening`...).
    pub fn status(&mut self) -> io::Result<Vec<(String, i64)>> {
        let mut text = String::new();
        self.ctl.read_to_string(&mut text)?;
        Ok(parse_status(&text))
    }

    /// One value from [`status`](Self::status).
    pub fn stat(&mut self, key: &str) -> io::Result<Option<i64>> {
        Ok(self.status()?.into_iter().find(|(k, _)| k == key).map(|(_, v)| v))
    }
}

/// Peak level of 16-bit samples in dBFS (-99 for silence).
pub fn peak_dbfs(samples: &[i16]) -> i32 {
    let peak = samples.iter().map(|s| (*s as i32).unsigned_abs()).max().unwrap_or(0);
    if peak == 0 {
        -99
    } else {
        ((20.0 * (peak as f64 / 32768.0).log10()).round() as i32).max(-99)
    }
}

/// A 44-byte RIFF/WAVE header for `data_len` bytes of PCM.
pub fn wav_header(rate: u32, bits: u8, channels: u8, data_len: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(44);
    let block = channels as u32 * bits as u32 / 8;
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&(channels as u16).to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * block).to_le_bytes());
    v.extend_from_slice(&(block as u16).to_le_bytes());
    v.extend_from_slice(&(bits as u16).to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    v
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
    fn header_round_trips() {
        let mut w = wav_header(16000, 16, 1, 4);
        w.extend_from_slice(&[1, 2, 3, 4]);
        assert_eq!(parse_wav(&w), Some(WavInfo { rate: 16000, bits: 16, channels: 1, data_offset: 44, data_len: 4 }));
    }

    #[test]
    fn sound_conf_defaults() {
        let t = "# defaults\noutput left\n";
        assert_eq!(parse_sound_conf(t, "output").as_deref(), Some("left"));
        assert_eq!(parse_sound_conf(t, "input"), None);
        let t2 = set_sound_conf(t, "input", Some("top"));
        assert_eq!(parse_sound_conf(&t2, "input").as_deref(), Some("top"));
        assert_eq!(parse_sound_conf(&t2, "output").as_deref(), Some("left"));
        let t3 = set_sound_conf(&t2, "output", None);
        assert_eq!(parse_sound_conf(&t3, "output"), None);
    }

    #[test]
    fn peak_level() {
        assert_eq!(peak_dbfs(&[]), -99);
        assert_eq!(peak_dbfs(&[0, 0]), -99);
        assert_eq!(peak_dbfs(&[16384, -100]), -6);
        assert_eq!(peak_dbfs(&[-32768]), 0);
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
