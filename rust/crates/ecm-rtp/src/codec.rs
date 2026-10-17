//! The voice codecs: IMA ADPCM (`ecm_audio::adpcm`, the same block format the
//! mod's Java speaker decodes) in 20 ms frames.
//!
//! * [`Codec::AdpcmWb`]: wideband, 16 kHz, 4 bits a sample: 164 bytes a frame, 8.2 KB/s.
//! * [`Codec::AdpcmNb`]: narrowband, 8 kHz: the encoder low-passes (a 15-tap windowed
//!   sinc at 3.6 kHz) and drops every other sample, the decoder interpolates back up,
//!   so the application always works at 16 kHz: 84 bytes a frame, 4.2 KB/s.
//!
//! An encoded frame is exactly one ADPCM block, whose 4-byte header carries the first
//! sample and the step index: every frame decodes on its own, so a lost or reordered
//! packet never desynchronises the decoder ([`Codec::decode_frame`] is stateless).
//! The encoder keeps the adapted step size and the filter history between frames.
//!
//! Frames should be [`crate::FRAME_SAMPLES`] long (any multiple of 4 round-trips
//! exactly in length).

use ecm_audio::adpcm;

/// Payload type used for [`Codec::AdpcmWb`] (dynamic range).
pub const PT_ADPCM_WB: u8 = 96;
/// Payload type used for [`Codec::AdpcmNb`].
pub const PT_ADPCM_NB: u8 = 97;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Codec {
    /// 16 kHz IMA ADPCM.
    AdpcmWb,
    /// 8 kHz IMA ADPCM (16 kHz at the application).
    AdpcmNb,
}

impl Codec {
    /// The codec's name in SDP/config ("adpcm-wb", "adpcm-nb").
    pub fn name(self) -> &'static str {
        match self {
            Codec::AdpcmWb => "adpcm-wb",
            Codec::AdpcmNb => "adpcm-nb",
        }
    }

    pub fn from_name(s: &str) -> Option<Codec> {
        match s.to_ascii_lowercase().as_str() {
            "adpcm-wb" | "wb" => Some(Codec::AdpcmWb),
            "adpcm-nb" | "nb" => Some(Codec::AdpcmNb),
            _ => None,
        }
    }

    pub fn payload_type(self) -> u8 {
        match self {
            Codec::AdpcmWb => PT_ADPCM_WB,
            Codec::AdpcmNb => PT_ADPCM_NB,
        }
    }

    pub fn from_payload_type(pt: u8) -> Option<Codec> {
        match pt {
            PT_ADPCM_WB => Some(Codec::AdpcmWb),
            PT_ADPCM_NB => Some(Codec::AdpcmNb),
            _ => None,
        }
    }

    /// The RTP clock rate (the coded sample rate), Hz.
    pub fn clock_rate(self) -> u32 {
        match self {
            Codec::AdpcmWb => 16_000,
            Codec::AdpcmNb => 8_000,
        }
    }

    /// RTP timestamp units per 20 ms frame.
    pub fn timestamp_step(self) -> u32 {
        self.clock_rate() * crate::FRAME_MS / 1000
    }

    /// Bytes of one encoded 20 ms frame.
    pub fn frame_bytes(self) -> usize {
        adpcm::encoded_size(self.timestamp_step() as usize)
    }

    /// Payload bytes per second while talking (8200 WB, 4200 NB).
    pub fn bytes_per_second(self) -> u32 {
        self.frame_bytes() as u32 * (1000 / crate::FRAME_MS)
    }

    /// Decode one frame to 16 kHz samples; None if it is malformed.
    pub fn decode_frame(self, frame: &[u8]) -> Option<Vec<i16>> {
        if frame.len() <= adpcm::HEADER_BYTES {
            return None;
        }
        let count = (frame.len() - adpcm::HEADER_BYTES) * 2;
        let pcm = adpcm::decode(frame, count)?;
        Some(match self {
            Codec::AdpcmWb => pcm,
            Codec::AdpcmNb => upsample2(&pcm),
        })
    }
}

/// Anti-alias filter taps for 16 kHz -> 8 kHz.
const TAPS: usize = 15;

fn lowpass_taps() -> [f32; TAPS] {
    let fc = 3600.0f32 / 16000.0;
    let m = (TAPS - 1) as f32 / 2.0;
    let mut h = [0f32; TAPS];
    for (i, t) in h.iter_mut().enumerate() {
        let x = i as f32 - m;
        let sinc = if x == 0.0 {
            2.0 * fc
        } else {
            (std::f32::consts::TAU * fc * x).sin() / (std::f32::consts::PI * x)
        };
        let window = 0.54 - 0.46 * (std::f32::consts::TAU * i as f32 / (TAPS - 1) as f32).cos();
        *t = sinc * window;
    }
    let sum: f32 = h.iter().sum();
    h.iter_mut().for_each(|t| *t /= sum);
    h
}

/// Encodes 16 kHz frames with one codec, keeping state between frames.
#[derive(Clone, Debug)]
pub struct FrameEncoder {
    codec: Codec,
    adpcm: adpcm::Encoder,
    taps: [f32; TAPS],
    /// The last `TAPS - 1` input samples (the filter's memory).
    history: Vec<f32>,
}

impl FrameEncoder {
    pub fn new(codec: Codec) -> FrameEncoder {
        FrameEncoder {
            codec,
            adpcm: adpcm::Encoder::new(),
            taps: lowpass_taps(),
            history: vec![0.0; TAPS - 1],
        }
    }

    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// One frame of 16 kHz samples in, one payload out.
    pub fn encode_frame(&mut self, pcm: &[i16]) -> Vec<u8> {
        match self.codec {
            Codec::AdpcmWb => self.adpcm.encode(pcm),
            Codec::AdpcmNb => {
                let narrow = self.decimate(pcm);
                self.adpcm.encode(&narrow)
            }
        }
    }

    fn decimate(&mut self, pcm: &[i16]) -> Vec<i16> {
        let mut buf = std::mem::take(&mut self.history);
        let start = buf.len();
        buf.extend(pcm.iter().map(|s| *s as f32));
        let mut out = Vec::with_capacity(pcm.len() / 2);
        let mut i = 0;
        while i < pcm.len() {
            // Output aligned with input sample i: taps over buf[start + i - (TAPS-1) ..= start + i].
            let end = start + i;
            let acc: f32 = (0..TAPS)
                .map(|k| self.taps[k] * buf[end + 1 - TAPS + k])
                .sum();
            out.push(acc.round().clamp(-32768.0, 32767.0) as i16);
            i += 2;
        }
        self.history = buf[buf.len() - (TAPS - 1)..].to_vec();
        out
    }
}

/// 8 kHz -> 16 kHz by linear interpolation (the last sample is held).
fn upsample2(x: &[i16]) -> Vec<i16> {
    let mut out = Vec::with_capacity(x.len() * 2);
    for (i, &s) in x.iter().enumerate() {
        let next = *x.get(i + 1).unwrap_or(&s);
        out.push(s);
        out.push(((s as i32 + next as i32) / 2) as i16);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{power_at, rms, sine};
    use crate::FRAME_SAMPLES;

    fn round_trip(codec: Codec, pcm: &[i16]) -> Vec<i16> {
        let mut enc = FrameEncoder::new(codec);
        let mut out = Vec::new();
        for f in pcm.chunks(FRAME_SAMPLES) {
            let bytes = enc.encode_frame(f);
            assert_eq!(bytes.len(), codec.frame_bytes());
            out.extend(codec.decode_frame(&bytes).unwrap());
        }
        out
    }

    #[test]
    fn rates_and_sizes() {
        assert_eq!(Codec::AdpcmWb.frame_bytes(), 164);
        assert_eq!(Codec::AdpcmNb.frame_bytes(), 84);
        assert_eq!(Codec::AdpcmWb.bytes_per_second(), 8200);
        assert_eq!(Codec::AdpcmNb.bytes_per_second(), 4200);
        assert_eq!(Codec::AdpcmWb.timestamp_step(), 320);
        assert_eq!(Codec::AdpcmNb.timestamp_step(), 160);
        for c in [Codec::AdpcmWb, Codec::AdpcmNb] {
            assert_eq!(Codec::from_payload_type(c.payload_type()), Some(c));
            assert_eq!(Codec::from_name(c.name()), Some(c));
        }
        assert_eq!(Codec::from_payload_type(0), None);
    }

    #[test]
    fn wideband_round_trips_closely() {
        let pcm = sine(FRAME_SAMPLES * 10, 440.0, 10000.0);
        let out = round_trip(Codec::AdpcmWb, &pcm);
        assert_eq!(out.len(), pcm.len());
        let err: Vec<i16> = pcm
            .iter()
            .zip(&out)
            .skip(FRAME_SAMPLES)
            .map(|(a, b)| a - b)
            .collect();
        assert!(
            rms(&err) < rms(&pcm) / 10.0,
            "err {} sig {}",
            rms(&err),
            rms(&pcm)
        );
    }

    #[test]
    fn narrowband_keeps_voice_and_filters_out_high_tones() {
        let n = FRAME_SAMPLES * 10;
        let voice = round_trip(Codec::AdpcmNb, &sine(n, 1000.0, 10000.0));
        assert_eq!(voice.len(), n);
        let p1k = power_at(&voice[FRAME_SAMPLES..], 1000.0);
        let input_p = power_at(&sine(n, 1000.0, 10000.0)[FRAME_SAMPLES..], 1000.0);
        assert!(p1k > input_p * 0.5, "1 kHz passes: {p1k} vs {input_p}");
        // 6.5 kHz is above the narrowband limit: it must not come back (nor alias to 1.5 kHz).
        let hi = round_trip(Codec::AdpcmNb, &sine(n, 6500.0, 10000.0));
        assert!(
            rms(&hi[FRAME_SAMPLES..]) < 10000.0 * 0.707 * 0.1,
            "rms {}",
            rms(&hi[FRAME_SAMPLES..])
        );
        assert!(power_at(&hi[FRAME_SAMPLES..], 1500.0) < input_p * 0.01);
    }

    #[test]
    fn a_lost_frame_does_not_desync_the_decoder() {
        for codec in [Codec::AdpcmWb, Codec::AdpcmNb] {
            let pcm = sine(FRAME_SAMPLES * 4, 700.0, 9000.0);
            let mut enc = FrameEncoder::new(codec);
            let frames: Vec<Vec<u8>> = pcm
                .chunks(FRAME_SAMPLES)
                .map(|f| enc.encode_frame(f))
                .collect();
            let in_order = codec.decode_frame(&frames[3]).unwrap();
            // Decoding frame 3 alone (0..2 lost) gives the same samples.
            let _ = codec.decode_frame(&frames[0]);
            assert_eq!(codec.decode_frame(&frames[3]).unwrap(), in_order);
            let want = power_at(&pcm[FRAME_SAMPLES * 3..], 700.0);
            assert!(power_at(&in_order, 700.0) > want * 0.5, "{codec:?}");
        }
    }

    #[test]
    fn malformed_frames_are_refused() {
        for codec in [Codec::AdpcmWb, Codec::AdpcmNb] {
            assert_eq!(codec.decode_frame(&[]), None);
            assert_eq!(codec.decode_frame(&[0, 0, 0, 0]), None);
            assert_eq!(
                codec.decode_frame(&[0, 0, 200, 0, 0, 0]),
                None,
                "bad step index"
            );
        }
        assert!(FrameEncoder::new(Codec::AdpcmNb)
            .encode_frame(&[])
            .is_empty());
    }

    #[test]
    fn upsampling_interpolates() {
        assert_eq!(upsample2(&[0, 100, 200]), vec![0, 50, 100, 150, 200, 200]);
    }
}
