//! Real-time voice shared by the mod's voice programs: Discord voice channels
//! (`ecm-discord`) and cellular VoLTE calls.
//!
//! Everything works at [`RATE`] (16 kHz mono, 16-bit) on the application side.
//!
//! * [`rtp`]: RTP headers (RFC 3550), minimal RTCP Sender/Receiver Reports and the
//!   receive statistics (loss and interarrival jitter) a call screen shows.
//! * [`codec`]: IMA ADPCM voice codecs, wideband (16 kHz) and narrowband (8 kHz),
//!   in 20 ms frames that each carry their own decoder state.
//! * [`dtmf`]: RFC 4733 telephone events (sender and de-duplicating receiver), the
//!   in-band DTMF tone generator and a Goertzel detector.
//! * [`jitter`]: an adaptive RTP jitter buffer with loss concealment, and the
//!   multi-talker [`jitter::Mixer`] (from Discord).
//! * [`gate`]: the microphone's voice-activity gate (threshold and hang time).
//! * [`echo`]: the loudspeaker echo guard (half duplex like a speakerphone).
//! * [`tones`]: call progress tones (dial, ringback, busy, reorder, SIT, ...).
//! * [`wav`]: minimal RIFF WAV read/write for voicemail and voice notes.
//!
//! Nothing here calls the host, so everything is tested with `cargo test -p ecm-rtp`.

pub mod codec;
pub mod dtmf;
pub mod echo;
pub mod gate;
pub mod jitter;
pub mod rtp;
pub mod tones;
pub mod wav;

/// The application-side sample rate, Hz.
pub const RATE: u32 = 16_000;
/// Frame length, ms.
pub const FRAME_MS: u32 = 20;
/// Samples per frame at [`RATE`] (20 ms).
pub const FRAME_SAMPLES: usize = (RATE * FRAME_MS / 1000) as usize;

pub use codec::Codec;
pub use echo::EchoGuard;
pub use gate::VoiceGate;
pub use jitter::{JitterBuffer, Mixer};
pub use tones::Tone;

#[cfg(test)]
pub(crate) mod test_util {
    use crate::RATE;

    /// A sine at `hz`, `amp` peak, at `RATE`.
    pub fn sine(n: usize, hz: f64, amp: f64) -> Vec<i16> {
        (0..n)
            .map(|i| ((i as f64 * hz * std::f64::consts::TAU / RATE as f64).sin() * amp) as i16)
            .collect()
    }

    /// RMS of a signal.
    pub fn rms(v: &[i16]) -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        (v.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
    }

    /// Power of `hz` in `v` (Goertzel, normalised so a full-scale-amplitude A sine gives ~A^2/4).
    pub fn power_at(v: &[i16], hz: f64) -> f64 {
        let w = std::f64::consts::TAU * hz / RATE as f64;
        let c = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &x in v {
            let s0 = x as f64 + c * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let n = v.len() as f64;
        (s1 * s1 + s2 * s2 - c * s1 * s2) / (n * n)
    }
}
