//! MPEG-1/2/2.5 Layer III frame headers: what `musicd scan` records about an
//! MP3 (sample rate, channels, bit rate, VBR, duration) and the byte offset
//! of a moment in the song, so the Music app can seek with an HTTP `Range`
//! read instead of downloading up to that point.
//!
//! Duration and seeking come from a Xing/Info header (what LAME writes) or a
//! VBRI header (Fraunhofer) when there is one, else from walking every frame.
//! A `Range` read lands anywhere, usually inside a frame, so [`frame_sync`]
//! finds where the next frame starts.

use crate::id3;

/// MPEG audio version.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Version {
    V1,
    V2,
    V25,
}

/// One Layer III frame header.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    pub version: Version,
    pub bitrate_kbps: u32,
    pub sample_rate: u32,
    pub padding: bool,
    /// A 16-bit CRC follows the header.
    pub crc: bool,
    /// 0 stereo, 1 joint stereo, 2 dual channel, 3 mono.
    pub channel_mode: u8,
    pub channels: u8,
    /// Whole frame in bytes, header included.
    pub frame_len: usize,
    /// PCM samples per channel the frame decodes to (1152 or 576).
    pub samples: u32,
}

const BITRATES_V1: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
const BITRATES_V2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

impl FrameHeader {
    /// Bytes of side information after the header (and CRC).
    pub fn side_info_len(&self) -> usize {
        match (self.version, self.channels) {
            (Version::V1, 1) => 17,
            (Version::V1, _) => 32,
            (_, 1) => 9,
            _ => 17,
        }
    }

    /// Frame duration in milliseconds (fractional).
    pub fn duration_ms(&self) -> f64 {
        self.samples as f64 * 1000.0 / self.sample_rate as f64
    }

    /// Whether `other` can follow this frame in the same stream.
    pub fn compatible(&self, other: &FrameHeader) -> bool {
        self.version == other.version && self.sample_rate == other.sample_rate && (self.channels == 1) == (other.channels == 1)
    }
}

/// Parse a Layer III frame header from the first 4 bytes of `b`. Free-format
/// bit rates and Layer I/II are refused.
pub fn parse_header(b: &[u8]) -> Option<FrameHeader> {
    if b.len() < 4 || b[0] != 0xff || b[1] & 0xe0 != 0xe0 {
        return None;
    }
    let version = match (b[1] >> 3) & 3 {
        0 => Version::V25,
        2 => Version::V2,
        3 => Version::V1,
        _ => return None,
    };
    if (b[1] >> 1) & 3 != 1 {
        return None; // not Layer III
    }
    let crc = b[1] & 1 == 0;
    let bi = (b[2] >> 4) as usize;
    let si = ((b[2] >> 2) & 3) as usize;
    if bi == 0 || bi == 15 || si == 3 || b[3] & 3 == 2 {
        return None;
    }
    let bitrate_kbps = if version == Version::V1 { BITRATES_V1[bi] } else { BITRATES_V2[bi] };
    let sample_rate = match version {
        Version::V1 => [44100, 48000, 32000][si],
        Version::V2 => [22050, 24000, 16000][si],
        Version::V25 => [11025, 12000, 8000][si],
    };
    let padding = (b[2] >> 1) & 1 == 1;
    let channel_mode = b[3] >> 6;
    let (samples, coef) = if version == Version::V1 { (1152, 144) } else { (576, 72) };
    let frame_len = (coef * bitrate_kbps * 1000 / sample_rate) as usize + padding as usize;
    Some(FrameHeader {
        version,
        bitrate_kbps,
        sample_rate,
        padding,
        crc,
        channel_mode,
        channels: if channel_mode == 3 { 1 } else { 2 },
        frame_len,
        samples,
    })
}

/// The result of looking for a frame start.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FrameSearch {
    /// A header at this offset, confirmed by a matching header right after
    /// the frame.
    Found(usize),
    /// A header at this offset whose frame runs past the end of the data, so
    /// it can't be confirmed yet.
    Unconfirmed(usize),
    /// No header at or after `from`.
    NotFound,
}

/// Look for the next frame start at or after `from`. A lone 0xFFE pattern
/// inside audio data is common, so a header only counts as `Found` when the
/// frame it describes is followed by another compatible header.
pub fn find_frame(bytes: &[u8], from: usize) -> FrameSearch {
    let mut i = from;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xff {
            i += 1;
            continue;
        }
        if let Some(h) = parse_header(&bytes[i..]) {
            let next = i + h.frame_len;
            if next + 4 > bytes.len() {
                return FrameSearch::Unconfirmed(i);
            }
            if parse_header(&bytes[next..]).is_some_and(|n| h.compatible(&n)) {
                return FrameSearch::Found(i);
            }
        }
        i += 1;
    }
    FrameSearch::NotFound
}

/// Offset of the next frame start at or after `from`: used when a `Range`
/// read lands mid-frame. A header near the end of `bytes` that can't be
/// confirmed by the one after it is still returned.
pub fn frame_sync(bytes: &[u8], from: usize) -> Option<usize> {
    match find_frame(bytes, from) {
        FrameSearch::Found(i) | FrameSearch::Unconfirmed(i) => Some(i),
        FrameSearch::NotFound => None,
    }
}

/// Where the duration and seek table came from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Timing {
    /// A Xing header (LAME's VBR files).
    Xing,
    /// An Info header (LAME's CBR files; same layout as Xing).
    Info,
    /// A Fraunhofer VBRI header.
    Vbri,
    /// No header: every frame was counted.
    Counted,
}

/// What a scan records about an MP3.
#[derive(Clone, Debug, PartialEq)]
pub struct Mp3Info {
    pub version: Version,
    pub sample_rate: u32,
    pub channels: u8,
    /// Average bit rate.
    pub bitrate_kbps: u32,
    pub vbr: bool,
    pub duration_ms: u64,
    /// Audio frames (not counting a Xing/Info/VBRI frame).
    pub frames: u64,
    pub samples_per_frame: u32,
    /// Offset of the first frame (which may be the Xing/Info frame).
    pub first_frame: u64,
    /// Offset of the first frame with audio.
    pub audio_start: u64,
    /// End of the audio (before an ID3v1 or APE tag).
    pub audio_end: u64,
    pub timing: Timing,
    /// Byte offsets at 0%, 1%, ... 100% of the duration.
    pub seek: Vec<u64>,
}

impl Mp3Info {
    /// Byte offset to start reading at to play from `ms`. It is on (or just
    /// before) a frame boundary for counted files and Xing tables; the
    /// decoder resyncs with [`frame_sync`] anyway.
    pub fn offset_for_ms(&self, ms: u64) -> u64 {
        if self.duration_ms == 0 || self.seek.len() < 2 {
            return self.audio_start;
        }
        let p = (ms as f64 * 100.0 / self.duration_ms as f64).clamp(0.0, 100.0);
        let i = (p as usize).min(self.seek.len() - 2);
        let (a, b) = (self.seek[i] as f64, self.seek[i + 1] as f64);
        let off = a + (b - a) * (p - i as f64);
        (off as u64).clamp(self.audio_start, self.audio_end.max(self.audio_start))
    }

    /// Bytes per second on average (for buffer sizing).
    pub fn bytes_per_second(&self) -> u64 {
        self.bitrate_kbps as u64 * 1000 / 8
    }
}

/// Offset after every ID3v2 tag at the start of `file`.
pub fn skip_id3v2(file: &[u8]) -> usize {
    let mut pos = 0;
    loop {
        let n = id3::v2_size(&file[pos..]);
        if n == 0 {
            return pos.min(file.len());
        }
        pos = pos.saturating_add(n).min(file.len());
    }
}

/// End of the audio data: before an ID3v1 tag and an APEv2 tag at the end.
pub fn audio_end(file: &[u8]) -> usize {
    let mut end = file.len();
    if end >= 128 && &file[end - 128..end - 125] == b"TAG" {
        end -= 128;
    }
    if end >= 32 && &file[end - 32..end - 24] == b"APETAGEX" {
        let size = u32::from_le_bytes([file[end - 20], file[end - 19], file[end - 18], file[end - 17]]) as usize;
        let header = if file[end - 9] & 0x80 != 0 { 32 } else { 0 };
        end = end.saturating_sub(size + header);
    }
    end
}

struct XingInfo {
    tag: Timing,
    frames: Option<u32>,
    bytes: Option<u32>,
    toc: Option<[u8; 100]>,
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn be16(b: &[u8]) -> u32 {
    u16::from_be_bytes([b[0], b[1]]) as u32
}

fn xing(frame: &[u8], h: &FrameHeader) -> Option<XingInfo> {
    let off = 4 + if h.crc { 2 } else { 0 } + h.side_info_len();
    let b = frame.get(off..)?;
    let tag = match b.get(..4)? {
        b"Xing" => Timing::Xing,
        b"Info" => Timing::Info,
        _ => return None,
    };
    let flags = be32(b.get(4..8)?);
    let mut p = 8;
    let mut take = |n: usize| -> Option<&[u8]> {
        let s = b.get(p..p + n)?;
        p += n;
        Some(s)
    };
    let frames = if flags & 1 != 0 { Some(be32(take(4)?)) } else { None };
    let bytes = if flags & 2 != 0 { Some(be32(take(4)?)) } else { None };
    let toc = if flags & 4 != 0 { take(100)?.try_into().ok() } else { None };
    Some(XingInfo { tag, frames, bytes, toc })
}

struct Vbri {
    bytes: u32,
    frames: u32,
    /// Cumulative byte offsets (from the first frame) at equal steps of time.
    table: Vec<u64>,
}

fn vbri(frame: &[u8]) -> Option<Vbri> {
    let b = frame.get(36..)?;
    if b.get(..4)? != b"VBRI" {
        return None;
    }
    let bytes = be32(b.get(10..14)?);
    let frames = be32(b.get(14..18)?);
    let entries = be16(b.get(18..20)?) as usize;
    let scale = be16(b.get(20..22)?) as u64;
    let size = be16(b.get(22..24)?) as usize;
    let mut table = vec![0u64];
    if (1..=4).contains(&size) {
        let mut acc = 0u64;
        for i in 0..entries {
            let e = b.get(26 + i * size..26 + (i + 1) * size)?;
            let v = e.iter().fold(0u64, |a, &x| (a << 8) | x as u64);
            acc += v * scale.max(1);
            table.push(acc);
        }
    }
    Some(Vbri { bytes, frames, table })
}

/// Read the timing of an MP3 file (all of it, as `musicd scan` has it).
/// None when no Layer III frame is found.
pub fn probe(file: &[u8]) -> Option<Mp3Info> {
    let start = skip_id3v2(file);
    let end = audio_end(file).max(start);
    let audio = &file[..end];
    let first = match find_frame(audio, start) {
        FrameSearch::Found(i) | FrameSearch::Unconfirmed(i) => i,
        FrameSearch::NotFound => return None,
    };
    let h = parse_header(&audio[first..])?;
    let frame = &audio[first..(first + h.frame_len).min(end)];
    let spf = h.samples as u64;
    let rate = h.sample_rate as u64;
    let mut info = Mp3Info {
        version: h.version,
        sample_rate: h.sample_rate,
        channels: h.channels,
        bitrate_kbps: h.bitrate_kbps,
        vbr: false,
        duration_ms: 0,
        frames: 0,
        samples_per_frame: h.samples,
        first_frame: first as u64,
        audio_start: first as u64,
        audio_end: end as u64,
        timing: Timing::Counted,
        seek: Vec::new(),
    };
    let header = xing(frame, &h);
    if let Some(x) = header.as_ref().filter(|x| x.frames.is_some()) {
        let frames = x.frames.unwrap_or(0) as u64;
        let total = x.bytes.map(|b| b as u64).filter(|b| *b > 0).unwrap_or((end - first) as u64);
        info.timing = x.tag;
        info.vbr = x.tag == Timing::Xing;
        info.frames = frames;
        info.audio_start = (first + h.frame_len).min(end) as u64;
        info.duration_ms = frames * spf * 1000 / rate;
        info.seek = match &x.toc {
            Some(toc) => {
                let mut s: Vec<u64> = toc.iter().map(|&t| first as u64 + t as u64 * total / 256).collect();
                s.push(first as u64 + total);
                s
            }
            None => proportional(info.audio_start, end as u64),
        };
    } else if let Some(v) = vbri(frame).filter(|v| v.frames > 0) {
        info.timing = Timing::Vbri;
        info.vbr = true;
        info.frames = v.frames as u64;
        info.audio_start = (first + h.frame_len).min(end) as u64;
        info.duration_ms = info.frames * spf * 1000 / rate;
        let total = if v.bytes > 0 { v.bytes as u64 } else { (end - first) as u64 };
        info.seek = if v.table.len() >= 2 {
            // Resample the VBRI table (equal time steps) to 101 points.
            let n = (v.table.len() - 1) as f64;
            (0..=100)
                .map(|i| {
                    let p = i as f64 / 100.0 * n;
                    let k = (p as usize).min(v.table.len() - 2);
                    let (a, b) = (v.table[k] as f64, v.table[k + 1] as f64);
                    first as u64 + (a + (b - a) * (p - k as f64)) as u64
                })
                .collect()
        } else {
            proportional(first as u64, first as u64 + total)
        };
    } else {
        // Count every frame, remembering where each starts. An Info/Xing
        // frame without a frame count is skipped (it holds no audio).
        let mut pos = first;
        if header.is_some() {
            pos += h.frame_len;
            info.audio_start = pos as u64;
        }
        let mut offsets: Vec<u64> = Vec::new();
        let mut bitrate: Option<u32> = None;
        let mut bytes = 0u64;
        while pos + 4 <= end {
            match parse_header(&audio[pos..]).filter(|f| h.compatible(f)) {
                Some(f) if pos + f.frame_len <= end || offsets.is_empty() => {
                    offsets.push(pos as u64);
                    if bitrate.is_some_and(|b| b != f.bitrate_kbps) {
                        info.vbr = true;
                    }
                    bitrate = Some(f.bitrate_kbps);
                    bytes += f.frame_len as u64;
                    pos += f.frame_len;
                }
                Some(_) => break, // a truncated last frame
                None => match find_frame(audio, pos + 1) {
                    FrameSearch::Found(i) => pos = i,
                    _ => break,
                },
            }
        }
        info.frames = offsets.len() as u64;
        info.duration_ms = info.frames * spf * 1000 / rate;
        info.seek = (0..=100)
            .map(|i| {
                if offsets.is_empty() {
                    info.audio_start
                } else if i == 100 {
                    end as u64
                } else {
                    offsets[(i * offsets.len() / 100).min(offsets.len() - 1)]
                }
            })
            .collect();
        if info.duration_ms > 0 {
            info.bitrate_kbps = (bytes * 8 / info.duration_ms) as u32;
        }
        return Some(info);
    }
    if info.duration_ms > 0 {
        let bytes = info.audio_end.saturating_sub(info.audio_start);
        info.bitrate_kbps = (bytes * 8 / info.duration_ms) as u32;
    }
    Some(info)
}

fn proportional(a: u64, b: u64) -> Vec<u64> {
    (0..=100).map(|i| a + (b.saturating_sub(a)) * i / 100).collect()
}

#[cfg(test)]
pub(crate) mod build {
    //! Hand-made frames for tests: valid headers with zeroed side info and
    //! main data, which decode to silence.

    /// A MPEG-1 Layer III frame header (no CRC, 44.1 kHz) at `kbps`.
    pub fn header_v1(kbps: u32, mono: bool) -> [u8; 4] {
        let idx = super::BITRATES_V1.iter().position(|&b| b == kbps).unwrap() as u8;
        [0xff, 0xfb, idx << 4, if mono { 0xc0 } else { 0x00 }]
    }

    pub fn frame_v1(kbps: u32, mono: bool) -> Vec<u8> {
        let h = header_v1(kbps, mono);
        let len = super::parse_header(&h).unwrap().frame_len;
        let mut f = h.to_vec();
        f.resize(len, 0);
        f
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    fn data(name: &str) -> Vec<u8> {
        std::fs::read(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn header_fields() {
        let h = parse_header(&[0xff, 0xfb, 0x90, 0x64]).unwrap();
        assert_eq!((h.version, h.bitrate_kbps, h.sample_rate, h.channels, h.frame_len, h.samples), (Version::V1, 128, 44100, 2, 417, 1152));
        let h2 = parse_header(&[0xff, 0xf3, 0x40, 0xc4]).unwrap(); // MPEG-2, 32 kbps, 22.05 kHz, mono
        assert_eq!((h2.version, h2.bitrate_kbps, h2.sample_rate, h2.channels, h2.samples), (Version::V2, 32, 22050, 1, 576));
        assert_eq!(h2.frame_len, 72 * 32000 / 22050);
        let h25 = parse_header(&[0xff, 0xe3, 0x40, 0xc4]).unwrap();
        assert_eq!((h25.version, h25.sample_rate), (Version::V25, 11025));
        assert!(parse_header(&[0xff, 0xfd, 0x90, 0x64]).is_none(), "layer II");
        assert!(parse_header(&[0xff, 0xfb, 0x00, 0x64]).is_none(), "free format");
        assert!(parse_header(&[0xff, 0xfb, 0x9c, 0x64]).is_none(), "reserved rate");
        assert!(parse_header(&[0xff, 0xeb, 0x90, 0x64]).is_none(), "reserved version");
    }

    #[test]
    fn frame_sync_skips_false_syncs_and_lands_on_frames() {
        let mut s = vec![0x12, 0xff, 0xfb, 0x90, 0x64, 0x00, 0x00]; // a false header with no frame after it
        let f = frame_v1(128, false);
        s.resize(500, 0x55);
        let at = s.len();
        for _ in 0..3 {
            s.extend_from_slice(&f);
        }
        assert_eq!(find_frame(&s, 0), FrameSearch::Found(at));
        assert_eq!(frame_sync(&s, at + 1), Some(at + f.len()));
        assert_eq!(find_frame(&s, at + 2 * f.len() + 1), FrameSearch::NotFound);
        assert_eq!(find_frame(&s, at + 2 * f.len()), FrameSearch::Unconfirmed(at + 2 * f.len()));
    }

    #[test]
    fn counted_cbr_vbr_and_proportional_seek() {
        let mut file = vec![];
        for _ in 0..100 {
            file.extend(frame_v1(128, false));
        }
        let i = probe(&file).unwrap();
        assert_eq!(i.timing, Timing::Counted);
        assert_eq!(i.frames, 100);
        assert!(!i.vbr);
        assert_eq!(i.duration_ms, 100 * 1152 * 1000 / 44100);
        assert!((127..=129).contains(&i.bitrate_kbps), "{}", i.bitrate_kbps);
        let half = i.offset_for_ms(i.duration_ms / 2);
        assert_eq!(half, 50 * 417, "on the 50th frame");
        let mut vbr = vec![];
        for k in 0..60 {
            vbr.extend(frame_v1(if k % 2 == 0 { 64 } else { 320 }, true));
        }
        let v = probe(&vbr).unwrap();
        assert!(v.vbr);
        assert_eq!(v.channels, 1);
        assert_eq!(v.frames, 60);
    }

    #[test]
    fn fixtures_with_xing_info_and_none() {
        let cbr = probe(&data("tone_cbr.mp3")).unwrap();
        assert_eq!((cbr.sample_rate, cbr.channels, cbr.timing, cbr.vbr), (44100, 2, Timing::Info, false));
        assert!((1950..=2150).contains(&cbr.duration_ms), "{}", cbr.duration_ms);
        assert!((60..=68).contains(&cbr.bitrate_kbps), "{}", cbr.bitrate_kbps);
        let vbr = probe(&data("tone_vbr.mp3")).unwrap();
        assert_eq!((vbr.channels, vbr.timing, vbr.vbr), (1, Timing::Xing, true));
        assert!((1950..=2150).contains(&vbr.duration_ms), "{}", vbr.duration_ms);
        let m2 = probe(&data("tone_mpeg2.mp3")).unwrap();
        assert_eq!((m2.version, m2.sample_rate, m2.channels, m2.timing), (Version::V2, 22050, 1, Timing::Counted));
        assert!((1950..=2150).contains(&m2.duration_ms), "{}", m2.duration_ms);
        assert!(m2.audio_end < data("tone_mpeg2.mp3").len() as u64, "the ID3v1 tag is not audio");
        for info in [&cbr, &vbr, &m2] {
            let mut last = 0;
            for ms in (0..=info.duration_ms).step_by(100) {
                let off = info.offset_for_ms(ms);
                assert!(off >= last && off >= info.audio_start && off <= info.audio_end);
                last = off;
            }
        }
        // The seek offset lands in or on a frame; the next one starts within a frame length.
        let file = data("tone_mpeg2.mp3");
        let off = m2.offset_for_ms(1000) as usize;
        let at = frame_sync(&file, off).unwrap();
        assert!(at - off < 200, "{off} -> {at}");
        assert!(parse_header(&file[at..]).is_some());
    }

    #[test]
    fn vbri_header() {
        let mut first = frame_v1(128, false);
        let mut v = b"VBRI".to_vec();
        v.extend_from_slice(&[0, 1, 0, 0, 0, 50]);
        v.extend_from_slice(&(417u32 * 10).to_be_bytes());
        v.extend_from_slice(&10u32.to_be_bytes());
        v.extend_from_slice(&[0, 2, 0, 1, 0, 2, 0, 5]); // 2 entries, scale 1, 2 bytes each, 5 frames per entry
        v.extend_from_slice(&(417u16 * 5).to_be_bytes());
        v.extend_from_slice(&(417u16 * 5).to_be_bytes());
        first[36..36 + v.len()].copy_from_slice(&v);
        let mut file = first;
        for _ in 0..10 {
            file.extend(frame_v1(128, false));
        }
        let i = probe(&file).unwrap();
        assert_eq!(i.timing, Timing::Vbri);
        assert_eq!(i.frames, 10);
        assert_eq!(i.audio_start, 417);
        assert!(i.offset_for_ms(i.duration_ms / 2).abs_diff(417 * 5) < 30);
    }

    #[test]
    fn probe_never_panics() {
        let mut seed = 99u64;
        let file = data("tone_vbr.mp3");
        for cut in (0..file.len()).step_by(7) {
            let _ = probe(&file[..cut]);
            let _ = probe(&file[cut..]);
        }
        for _ in 0..500 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let len = (seed % 2000) as usize;
            let junk: Vec<u8> = (0..len).map(|k| (seed >> (k % 56)) as u8 | if k % 5 == 0 { 0xe0 } else { 0 }).collect();
            if let Some(i) = probe(&junk) {
                let _ = i.offset_for_ms(12345);
            }
        }
    }
}
