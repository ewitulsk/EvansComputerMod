//! The two stream qualities' audio paths.
//!
//! **Data saver** (`GET /stream/<id>?q=saver`) is 22050 Hz mono IMA ADPCM,
//! about 11 KB/s against 16 KB/s for a 128 kbps MP3. `musicd` decodes each
//! MP3 once ([`transcode_to_saver`]) and caches the result in
//! `/var/music/cache`. The file is a 32-byte header then fixed-size blocks in
//! `ecm-audio`'s ADPCM block format (each block starts with its own predictor
//! and step index), so an HTTP `Range` read can start at any block boundary
//! and [`SaverHeader::offset_for_ms`] turns a seek into a byte offset:
//!
//! ```text
//! 0   4  magic "ECMS"
//! 4   1  version 1
//! 5   1  channels (1)
//! 6   2  reserved, 0
//! 8   4  sample rate, u32 LE (22050)
//! 12  4  samples per block, u32 LE (1024)
//! 16  4  total samples, u32 LE (the last block is padded with silence)
//! 20  4  duration in ms, u32 LE
//! 24  8  reserved, 0
//! 32  ... blocks of block_size() bytes: [i16 LE first sample][u8 step index][u8 0][nibbles]
//! ```
//!
//! **Original** streams the MP3 itself; the Music app decodes it with
//! [`Mp3Decoder`] (feature `decode`), which takes the bytes in whatever
//! chunks the network delivers and gives mono PCM at the speaker's rate.
//!
//! Both need a resampler; [`Resampler`] is a windowed-sinc low-pass with
//! polyphase interpolation, so 44.1 kHz music going to 22.05 kHz doesn't
//! alias and 22.05 kHz going to the speaker's 48 kHz doesn't image.

use ecm_audio::adpcm;

// ---------------------------------------------------------------------------
// Resampling

/// Taps on each side of the interpolation point.
const HALF: usize = 24;
const TAPS: usize = HALF * 2;
/// Fractional positions the kernel is tabulated at.
const PHASES: usize = 256;

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    for k in 1..30 {
        term *= (x / 2.0) * (x / 2.0) / (k as f64 * k as f64);
        sum += term;
    }
    sum
}

/// A streaming sample-rate converter for one channel.
#[derive(Clone, Debug)]
pub struct Resampler {
    in_rate: u32,
    out_rate: u32,
    /// Input samples per output sample.
    step: f64,
    /// Position of the next output sample in `buf`.
    pos: f64,
    buf: Vec<f32>,
    /// `PHASES + 1` rows of `TAPS` weights.
    table: Vec<f32>,
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Resampler {
        let in_rate = in_rate.max(1);
        let out_rate = out_rate.max(1);
        let mut table = Vec::new();
        if in_rate != out_rate {
            // Cut off a little below the lower Nyquist frequency, in cycles
            // per input sample; a Kaiser window keeps the stop band low.
            let fc = 0.45 * in_rate.min(out_rate) as f64 / in_rate as f64;
            let beta = 7.0;
            let i0b = bessel_i0(beta);
            table.reserve((PHASES + 1) * TAPS);
            for p in 0..=PHASES {
                let frac = p as f64 / PHASES as f64;
                let row: Vec<f64> = (0..TAPS)
                    .map(|k| {
                        // Distance from the output point to tap k.
                        let x = k as f64 - (HALF as f64 - 1.0) - frac;
                        let sinc = if x.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * 2.0 * fc * x).sin() / (std::f64::consts::PI * 2.0 * fc * x) };
                        let r = x / HALF as f64;
                        let w = if r.abs() >= 1.0 { 0.0 } else { bessel_i0(beta * (1.0 - r * r).sqrt()) / i0b };
                        2.0 * fc * sinc * w
                    })
                    .collect();
                let sum: f64 = row.iter().sum();
                table.extend(row.iter().map(|v| (v / sum) as f32));
            }
        }
        let mut r = Resampler { in_rate, out_rate, step: in_rate as f64 / out_rate as f64, pos: 0.0, buf: Vec::new(), table };
        r.reset();
        r
    }

    pub fn in_rate(&self) -> u32 {
        self.in_rate
    }

    pub fn out_rate(&self) -> u32 {
        self.out_rate
    }

    /// Forget the history (after a seek).
    pub fn reset(&mut self) {
        self.buf.clear();
        // Silence before the first sample, so the first output is centred on it.
        self.buf.resize(HALF - 1, 0.0);
        self.pos = (HALF - 1) as f64;
    }

    /// Convert `input`, appending what can be computed so far to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.in_rate == self.out_rate {
            out.extend_from_slice(input);
            return;
        }
        self.buf.extend_from_slice(input);
        loop {
            let base = self.pos as usize;
            if base + HALF >= self.buf.len() {
                break;
            }
            let frac = self.pos - base as f64;
            let p = ((frac * PHASES as f64) + 0.5) as usize;
            let row = &self.table[p * TAPS..(p + 1) * TAPS];
            let src = &self.buf[base + 1 - HALF..base + 1 + HALF];
            let mut acc = 0.0f32;
            for (a, b) in row.iter().zip(src) {
                acc += a * b;
            }
            out.push(acc);
            self.pos += self.step;
        }
        let keep_from = (self.pos as usize + 1).saturating_sub(HALF);
        if keep_from > 0 {
            self.buf.drain(..keep_from);
            self.pos -= keep_from as f64;
        }
    }

    /// Push the last input through (pads with silence).
    pub fn flush(&mut self, out: &mut Vec<f32>) {
        if self.in_rate != self.out_rate {
            // Only as many outputs as the real input accounts for.
            let (pos0, real_end) = (self.pos, self.buf.len() as f64);
            let mut tail = Vec::new();
            self.process(&[0.0; HALF + 1], &mut tail);
            let n = (0..tail.len()).take_while(|&k| pos0 + k as f64 * self.step < real_end).count();
            out.extend_from_slice(&tail[..n]);
        }
        self.reset();
    }
}

/// f32 in -1..1 to i16.
pub fn to_i16(v: f32) -> i16 {
    (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16
}

// ---------------------------------------------------------------------------
// The Data saver file

pub const SAVER_MAGIC: &[u8; 4] = b"ECMS";
pub const SAVER_RATE: u32 = 22050;
/// 1024 samples (46 ms) a block: 516 bytes.
pub const SAVER_BLOCK_SAMPLES: u32 = 1024;
pub const SAVER_HEADER_BYTES: usize = 32;
/// The MIME type `musicd` serves it as.
pub const SAVER_MIME: &str = "audio/x-ecm-adpcm";

/// The header of a Data saver file.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SaverHeader {
    pub rate: u32,
    pub channels: u8,
    pub block_samples: u32,
    pub total_samples: u32,
    pub duration_ms: u32,
}

impl SaverHeader {
    /// The header for `total_samples` at [`SAVER_RATE`].
    pub fn new(total_samples: u32) -> SaverHeader {
        SaverHeader {
            rate: SAVER_RATE,
            channels: 1,
            block_samples: SAVER_BLOCK_SAMPLES,
            total_samples,
            duration_ms: (total_samples as u64 * 1000 / SAVER_RATE as u64) as u32,
        }
    }

    pub fn to_bytes(&self) -> [u8; SAVER_HEADER_BYTES] {
        let mut b = [0u8; SAVER_HEADER_BYTES];
        b[..4].copy_from_slice(SAVER_MAGIC);
        b[4] = 1;
        b[5] = self.channels;
        b[8..12].copy_from_slice(&self.rate.to_le_bytes());
        b[12..16].copy_from_slice(&self.block_samples.to_le_bytes());
        b[16..20].copy_from_slice(&self.total_samples.to_le_bytes());
        b[20..24].copy_from_slice(&self.duration_ms.to_le_bytes());
        b
    }

    /// Read a header; None if it isn't one this code can play.
    pub fn parse(b: &[u8]) -> Option<SaverHeader> {
        if b.len() < SAVER_HEADER_BYTES || &b[..4] != SAVER_MAGIC || b[4] != 1 || b[5] != 1 {
            return None;
        }
        let u = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let h = SaverHeader { rate: u(8), channels: b[5], block_samples: u(12), total_samples: u(16), duration_ms: u(20) };
        ((8000..=48000).contains(&h.rate) && (2..=65536).contains(&h.block_samples)).then_some(h)
    }

    /// Bytes of one block.
    pub fn block_size(&self) -> usize {
        adpcm::encoded_size(self.block_samples as usize)
    }

    pub fn blocks(&self) -> u64 {
        (self.total_samples as u64).div_ceil(self.block_samples as u64)
    }

    /// File offset of block `n`.
    pub fn block_offset(&self, n: u64) -> u64 {
        SAVER_HEADER_BYTES as u64 + n * self.block_size() as u64
    }

    /// The block holding the moment `ms`.
    pub fn block_for_ms(&self, ms: u64) -> u64 {
        (ms * self.rate as u64 / 1000 / self.block_samples as u64).min(self.blocks().saturating_sub(1))
    }

    /// File offset to start a `Range` read at to play from `ms` (a block
    /// boundary at or before it).
    pub fn offset_for_ms(&self, ms: u64) -> u64 {
        self.block_offset(self.block_for_ms(ms))
    }

    /// The time block `n` starts at.
    pub fn ms_for_block(&self, n: u64) -> u64 {
        n * self.block_samples as u64 * 1000 / self.rate as u64
    }

    pub fn file_size(&self) -> u64 {
        self.block_offset(self.blocks())
    }
}

/// Encode mono PCM at [`SAVER_RATE`] as a Data saver file.
pub fn encode_saver(pcm: &[i16]) -> Vec<u8> {
    let h = SaverHeader::new(pcm.len() as u32);
    let mut out = Vec::with_capacity(h.file_size() as usize);
    out.extend_from_slice(&h.to_bytes());
    let mut enc = adpcm::Encoder::new();
    let bs = h.block_samples as usize;
    for chunk in pcm.chunks(bs) {
        if chunk.len() == bs {
            out.extend(enc.encode(chunk));
        } else {
            let mut last = chunk.to_vec();
            last.resize(bs, 0);
            out.extend(enc.encode(&last));
        }
    }
    out
}

/// Decodes a Data saver stream as it arrives: from the start of the file,
/// or (after a seek) from a block boundary with the header already known.
#[derive(Clone, Debug, Default)]
pub struct SaverDecoder {
    header: Option<SaverHeader>,
    buf: Vec<u8>,
    /// Index of the next block.
    block: u64,
}

impl SaverDecoder {
    /// For bytes from the start of the file (the header comes first).
    pub fn new() -> SaverDecoder {
        SaverDecoder::default()
    }

    /// For bytes from [`SaverHeader::block_offset`]`(block)` on.
    pub fn at_block(header: SaverHeader, block: u64) -> SaverDecoder {
        SaverDecoder { header: Some(header), buf: Vec::new(), block }
    }

    pub fn header(&self) -> Option<&SaverHeader> {
        self.header.as_ref()
    }

    /// The block the next decoded samples belong to.
    pub fn next_block(&self) -> u64 {
        self.block
    }

    /// Feed bytes; whole blocks are decoded into `out`. Err if the stream
    /// doesn't start with a Data saver header or a block is broken.
    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<i16>) -> Result<(), String> {
        self.buf.extend_from_slice(bytes);
        let mut at = 0;
        if self.header.is_none() {
            if self.buf.len() < SAVER_HEADER_BYTES {
                return Ok(());
            }
            self.header = Some(SaverHeader::parse(&self.buf).ok_or("not a Data saver stream")?);
            at = SAVER_HEADER_BYTES;
        }
        let h = self.header.unwrap();
        let size = h.block_size();
        while at + size <= self.buf.len() {
            let start = self.block * h.block_samples as u64;
            if start >= h.total_samples as u64 {
                at = self.buf.len(); // past the end: padding or junk
                break;
            }
            let pcm = adpcm::decode(&self.buf[at..at + size], h.block_samples as usize).ok_or("broken Data saver block")?;
            let n = (h.total_samples as u64 - start).min(h.block_samples as u64) as usize;
            out.extend_from_slice(&pcm[..n]);
            self.block += 1;
            at += size;
        }
        self.buf.drain(..at);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MP3 decoding (Symphonia)

#[cfg(feature = "decode")]
pub use self::decode::{decode_mp3_mono, transcode_to_saver, Mp3Decoder};

#[cfg(feature = "decode")]
mod decode {
    use super::{encode_saver, to_i16, Resampler, SAVER_RATE};
    use crate::id3;
    use crate::mp3::{self, FrameSearch};
    use symphonia_bundle_mp3::MpaDecoder;
    use symphonia_core::audio::{AudioBufferRef, Signal};
    use symphonia_core::codecs::{CodecParameters, Decoder, DecoderOptions, CODEC_TYPE_MP3};
    use symphonia_core::formats::Packet;

    /// Most bytes kept waiting for a frame header before old ones are dropped.
    const MAX_PENDING: usize = 256 * 1024;

    /// An MP3 decoder for a stream that arrives in arbitrary chunks: it keeps
    /// the bytes that don't yet make a whole frame, finds frames by their
    /// headers (so it resyncs after junk or a mid-frame start), skips an
    /// ID3v2 tag and the Xing/Info frame, and gives mono i16 PCM at
    /// `out_rate`.
    pub struct Mp3Decoder {
        out_rate: u32,
        buf: Vec<u8>,
        /// Bytes of an ID3v2 tag still to drop.
        skip: usize,
        /// The next bytes are the start of the file (an ID3v2 tag may follow).
        at_start: bool,
        decoder: Option<MpaDecoder>,
        spec: Option<(u32, u8)>,
        resampler: Option<Resampler>,
        mono: Vec<f32>,
        resampled: Vec<f32>,
        frames: u64,
        errors: u64,
    }

    impl Mp3Decoder {
        pub fn new(out_rate: u32) -> Mp3Decoder {
            Mp3Decoder {
                out_rate,
                buf: Vec::new(),
                skip: 0,
                at_start: true,
                decoder: None,
                spec: None,
                resampler: None,
                mono: Vec::new(),
                resampled: Vec::new(),
                frames: 0,
                errors: 0,
            }
        }

        /// The source sample rate and channel count, once a frame was seen.
        pub fn source(&self) -> Option<(u32, u8)> {
            self.spec
        }

        /// Frames decoded so far.
        pub fn frames(&self) -> u64 {
            self.frames
        }

        /// Frames that failed to decode (skipped).
        pub fn errors(&self) -> u64 {
            self.errors
        }

        pub fn out_rate(&self) -> u32 {
            self.out_rate
        }

        /// Feed the next bytes of the stream; decoded audio is appended to `out`.
        pub fn push(&mut self, bytes: &[u8], out: &mut Vec<i16>) {
            self.buf.extend_from_slice(bytes);
            self.run(out, false);
        }

        /// The stream ended: decode what is left and drain the resampler.
        pub fn finish(&mut self, out: &mut Vec<i16>) {
            self.run(out, true);
            self.buf.clear();
            if let Some(r) = &mut self.resampler {
                self.resampled.clear();
                r.flush(&mut self.resampled);
                out.extend(self.resampled.iter().map(|&v| to_i16(v)));
            }
        }

        /// Seek: forget buffered bytes and decoder state. The bytes pushed
        /// next come from a new offset (anywhere: the decoder resyncs on the
        /// next frame header; the first frame or two after a seek may be
        /// skipped while the bit reservoir refills).
        pub fn seek(&mut self) {
            self.buf.clear();
            self.skip = 0;
            self.at_start = false;
            if let Some(d) = &mut self.decoder {
                d.reset();
            }
            if let Some(r) = &mut self.resampler {
                r.reset();
            }
        }

        /// Start again at the beginning of a (new) file.
        pub fn restart(&mut self) {
            self.seek();
            self.at_start = true;
        }

        fn run(&mut self, out: &mut Vec<i16>, last: bool) {
            let mut at = 0usize;
            loop {
                if self.skip > 0 {
                    let n = self.skip.min(self.buf.len() - at);
                    at += n;
                    self.skip -= n;
                    if self.skip > 0 {
                        break;
                    }
                }
                if self.at_start {
                    let rest = &self.buf[at..];
                    if rest.len() < 10 && !last && b"ID3".starts_with(&rest[..rest.len().min(3)]) {
                        break; // can't tell yet
                    }
                    self.at_start = false;
                    let n = id3::v2_size(rest);
                    if n > 0 {
                        self.skip = n;
                        self.at_start = true; // tags can be stacked
                        continue;
                    }
                }
                match mp3::find_frame(&self.buf, at) {
                    FrameSearch::Found(i) => {
                        let h = mp3::parse_header(&self.buf[i..]).expect("found a header");
                        let end = i + h.frame_len;
                        let frame = self.buf[i..end].to_vec();
                        self.decode_frame(&frame, &h, out);
                        at = end;
                    }
                    FrameSearch::Unconfirmed(i) => {
                        let h = mp3::parse_header(&self.buf[i..]).expect("found a header");
                        if last && i + h.frame_len <= self.buf.len() {
                            let frame = self.buf[i..i + h.frame_len].to_vec();
                            self.decode_frame(&frame, &h, out);
                            at = i + h.frame_len;
                            continue;
                        }
                        at = i;
                        break;
                    }
                    FrameSearch::NotFound => {
                        // Keep a possible partial header at the end.
                        at = at.max(self.buf.len().saturating_sub(3));
                        break;
                    }
                }
            }
            self.buf.drain(..at.min(self.buf.len()));
            if self.buf.len() > MAX_PENDING {
                let cut = self.buf.len() - MAX_PENDING;
                self.buf.drain(..cut);
            }
        }

        fn decode_frame(&mut self, frame: &[u8], h: &mp3::FrameHeader, out: &mut Vec<i16>) {
            if is_info_frame(frame, h) {
                return;
            }
            let spec = (h.sample_rate, h.channels);
            if self.spec != Some(spec) || self.decoder.is_none() {
                let params = {
                    let mut p = CodecParameters::new();
                    p.for_codec(CODEC_TYPE_MP3).with_sample_rate(h.sample_rate);
                    p
                };
                match MpaDecoder::try_new(&params, &DecoderOptions::default()) {
                    Ok(d) => self.decoder = Some(d),
                    Err(_) => return,
                }
                if self.resampler.as_ref().map(|r| r.in_rate()) != Some(h.sample_rate) {
                    self.resampler = Some(Resampler::new(h.sample_rate, self.out_rate));
                }
                self.spec = Some(spec);
            }
            let dec = self.decoder.as_mut().unwrap();
            let packet = Packet::new_from_slice(0, self.frames * h.samples as u64, h.samples as u64, frame);
            self.mono.clear();
            match dec.decode(&packet) {
                Ok(AudioBufferRef::F32(b)) => {
                    let chans = b.spec().channels.count().max(1);
                    let n = b.frames();
                    let scale = 1.0 / chans as f32;
                    self.mono.resize(n, 0.0);
                    for c in 0..chans {
                        for (m, v) in self.mono.iter_mut().zip(b.chan(c)) {
                            *m += v * scale;
                        }
                    }
                    self.frames += 1;
                }
                _ => {
                    self.errors += 1;
                    return;
                }
            }
            self.resampled.clear();
            self.resampler.as_mut().unwrap().process(&self.mono, &mut self.resampled);
            out.extend(self.resampled.iter().map(|&v| to_i16(v)));
        }
    }

    /// The Xing/Info/VBRI frame LAME and others put first: no audio.
    fn is_info_frame(frame: &[u8], h: &mp3::FrameHeader) -> bool {
        let off = 4 + if h.crc { 2 } else { 0 } + h.side_info_len();
        matches!(frame.get(off..off + 4), Some(b"Xing") | Some(b"Info")) || frame.get(36..40) == Some(b"VBRI")
    }

    /// Decode a whole MP3 file to mono PCM at `rate`.
    pub fn decode_mp3_mono(mp3: &[u8], rate: u32) -> Vec<i16> {
        let mut d = Mp3Decoder::new(rate);
        let mut out = Vec::new();
        d.push(mp3, &mut out);
        d.finish(&mut out);
        out
    }

    /// The Data saver file for an MP3 (what `musicd` caches).
    pub fn transcode_to_saver(mp3: &[u8]) -> Result<Vec<u8>, String> {
        let pcm = decode_mp3_mono(mp3, SAVER_RATE);
        if pcm.is_empty() {
            return Err("no MP3 audio".into());
        }
        Ok(encode_saver(&pcm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, rate: f64, hz: f64, amp: f64) -> Vec<f32> {
        (0..n).map(|i| ((i as f64 * hz * std::f64::consts::TAU / rate).sin() * amp) as f32).collect()
    }

    /// Goertzel power of `hz` relative to the signal power (0..1).
    pub(crate) fn tone_share(pcm: &[f32], rate: f64, hz: f64) -> f64 {
        let w = std::f64::consts::TAU * hz / rate;
        let c = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &x in pcm {
            let s = x as f64 + c * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        let p = s1 * s1 + s2 * s2 - c * s1 * s2;
        let e: f64 = pcm.iter().map(|&x| (x as f64) * (x as f64)).sum();
        if e == 0.0 {
            0.0
        } else {
            2.0 * p / (e * pcm.len() as f64)
        }
    }

    fn rms(v: &[f32]) -> f64 {
        (v.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / v.len().max(1) as f64).sqrt()
    }

    #[test]
    fn resampler_keeps_the_passband_and_removes_what_would_alias() {
        // 1 kHz at 44.1 kHz to 22.05 kHz: kept at the same level.
        let mut r = Resampler::new(44100, 22050);
        let mut out = Vec::new();
        let input = tone(44100, 44100.0, 1000.0, 0.5);
        for chunk in input.chunks(1000) {
            r.process(chunk, &mut out);
        }
        r.flush(&mut out);
        assert!((out.len() as i64 - 22050).abs() <= 2, "{}", out.len());
        let mid = &out[1000..21000];
        assert!((rms(mid) - 0.5 / 2f64.sqrt()).abs() < 0.01, "rms {}", rms(mid));
        assert!(tone_share(mid, 22050.0, 1000.0) > 0.98);
        // 15 kHz can't be represented at 22.05 kHz: it must not fold to 7.05 kHz.
        let mut r = Resampler::new(44100, 22050);
        let mut out = Vec::new();
        r.process(&tone(44100, 44100.0, 15000.0, 0.5), &mut out);
        assert!(rms(&out[1000..20000]) < 0.005, "aliased rms {}", rms(&out[1000..20000]));
        // Upsampling 22.05 kHz to 48 kHz keeps a 3 kHz tone clean.
        let mut r = Resampler::new(22050, 48000);
        let mut out = Vec::new();
        r.process(&tone(22050, 22050.0, 3000.0, 0.5), &mut out);
        assert!(tone_share(&out[2000..40000], 48000.0, 3000.0) > 0.98);
        // Same rate: passthrough.
        let mut r = Resampler::new(8000, 8000);
        let mut out = Vec::new();
        r.process(&[0.25, -0.5], &mut out);
        assert_eq!(out, vec![0.25, -0.5]);
    }

    #[test]
    fn saver_format_round_trip_and_seek_offsets() {
        let pcm: Vec<i16> = tone(5000, 22050.0, 440.0, 10000.0).iter().map(|&v| v as i16).collect();
        let file = encode_saver(&pcm);
        let h = SaverHeader::parse(&file).unwrap();
        assert_eq!(h, SaverHeader { rate: 22050, channels: 1, block_samples: 1024, total_samples: 5000, duration_ms: 226 });
        assert_eq!(h.block_size(), 516);
        assert_eq!(h.blocks(), 5);
        assert_eq!(file.len() as u64, h.file_size());
        assert_eq!(file.len(), 32 + 5 * 516);
        assert_eq!(h.offset_for_ms(0), 32);
        assert_eq!(h.offset_for_ms(50), 32 + 516, "50 ms is in the second block");
        assert_eq!(h.offset_for_ms(10_000), 32 + 4 * 516, "clamped to the last block");
        assert_eq!(h.ms_for_block(1), 46);
        // Decode in awkward chunks.
        let mut d = SaverDecoder::new();
        let mut out = Vec::new();
        for c in file.chunks(77) {
            d.push(c, &mut out).unwrap();
        }
        assert_eq!(out.len(), 5000);
        let err: f64 = pcm.iter().zip(&out).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / 5000.0;
        assert!(err < 400.0, "mean error {err}");
        // A Range read from block 2 decodes the same samples.
        let mut d2 = SaverDecoder::at_block(h, 2);
        let mut out2 = Vec::new();
        d2.push(&file[h.block_offset(2) as usize..], &mut out2).unwrap();
        assert_eq!(out2, out[2048..]);
        assert!(SaverDecoder::new().push(&[0u8; 40], &mut Vec::new()).is_err());
        assert!(SaverHeader::parse(b"ECMS").is_none());
        // About 11 KB/s.
        let per_second = 22050.0 / 1024.0 * 516.0;
        assert!((10_500.0..11_500.0).contains(&per_second));
    }

    #[cfg(feature = "decode")]
    mod decode {
        use super::super::*;
        use super::tone_share;

        fn data(name: &str) -> Vec<u8> {
            std::fs::read(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
        }

        fn f(pcm: &[i16]) -> Vec<f32> {
            pcm.iter().map(|&v| v as f32 / 32768.0).collect()
        }

        #[test]
        fn decodes_the_fixture_tones() {
            // tone_cbr: 440 Hz stereo 44.1 kHz, 2 s; tone_vbr: 1 kHz mono;
            // tone_mpeg2: 440 Hz, 22.05 kHz mono with an ID3v1 tag.
            for (name, hz) in [("tone_cbr.mp3", 440.0), ("tone_vbr.mp3", 1000.0), ("tone_mpeg2.mp3", 440.0)] {
                let pcm = decode_mp3_mono(&data(name), 22050);
                let secs = pcm.len() as f64 / 22050.0;
                assert!((1.95..2.2).contains(&secs), "{name}: {secs} s");
                let mid = f(&pcm[4000..pcm.len() - 4000]);
                let share = tone_share(&mid, 22050.0, hz);
                assert!(share > 0.9, "{name}: {hz} Hz share {share}");
                let peak = pcm.iter().map(|v| v.unsigned_abs()).max().unwrap();
                // ffmpeg reads the same peaks (-21.5, -17.4 and -18.5 dBFS).
                assert!(peak > 2000, "{name}: peak {peak}");
            }
        }

        #[test]
        fn chunked_input_gives_the_same_audio_and_seek_resyncs() {
            let file = data("tone_cbr.mp3");
            let whole = decode_mp3_mono(&file, 48000);
            let mut d = Mp3Decoder::new(48000);
            let mut chunked = Vec::new();
            let mut seed = 7u32;
            let mut pos = 0;
            while pos < file.len() {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let n = (1 + (seed >> 16) as usize % 700).min(file.len() - pos);
                d.push(&file[pos..pos + n], &mut chunked);
                pos += n;
            }
            d.finish(&mut chunked);
            assert_eq!(chunked, whole);
            assert_eq!(d.source(), Some((44100, 2)));
            // Seek to the middle (mid-frame) and keep decoding.
            let info = crate::mp3::probe(&file).unwrap();
            let off = info.offset_for_ms(1000) as usize + 37;
            d.seek();
            let mut after = Vec::new();
            d.push(&file[off..], &mut after);
            d.finish(&mut after);
            let secs = after.len() as f64 / 48000.0;
            assert!((0.85..1.1).contains(&secs), "{secs} s after seeking to 1 s");
            let mid = f(&after[4000..after.len() - 4000]);
            assert!(tone_share(&mid, 48000.0, 440.0) > 0.9);
        }

        #[test]
        fn transcode_to_saver_file() {
            let saver = transcode_to_saver(&data("tone_vbr.mp3")).unwrap();
            let h = SaverHeader::parse(&saver).unwrap();
            assert!((1950..2200).contains(&h.duration_ms), "{}", h.duration_ms);
            assert_eq!(saver.len() as u64, h.file_size());
            let mut d = SaverDecoder::new();
            let mut pcm = Vec::new();
            d.push(&saver, &mut pcm).unwrap();
            let mid = f(&pcm[4000..pcm.len() - 4000]);
            assert!(tone_share(&mid, 22050.0, 1000.0) > 0.9);
            assert!(transcode_to_saver(b"not an mp3").is_err());
        }

        #[test]
        fn garbage_and_junk_between_frames() {
            let mut d = Mp3Decoder::new(22050);
            let mut out = Vec::new();
            let mut seed = 1u64;
            let junk: Vec<u8> = (0..20000)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed as u8
                })
                .collect();
            d.push(&junk, &mut out);
            d.finish(&mut out);
            // Junk, then a real file: the decoder finds the frames.
            let file = data("tone_mpeg2.mp3");
            let mut d = Mp3Decoder::new(22050);
            let mut out = Vec::new();
            d.push(&junk[..3000], &mut out);
            d.push(&file, &mut out);
            d.finish(&mut out);
            assert!(out.len() as f64 / 22050.0 > 1.9);
        }
    }
}
