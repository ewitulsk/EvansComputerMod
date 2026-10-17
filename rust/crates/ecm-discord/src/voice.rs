//! Voice channels: the UDP packets between the Discord app and a backend, and the
//! app's side of the audio (who is talking, the jitter buffer that mixes everyone
//! for the speaker, the voice-activity gate for the microphone). The gate, the mixer
//! and the loudspeaker echo guard are shared with cellular calls in `ecm-rtp`; this
//! module keeps Discord's own "DV" packets and wraps them.
//!
//! Like Discord, a backend is a forwarding unit: each client sends its microphone
//! to the backend's voice port (the TCP port + 1), and the backend forwards every
//! packet to the other people in that voice channel. Clients mix what they hear.
//!
//! Audio is 16 kHz mono, [`FRAME`] samples (40 ms) per packet, IMA ADPCM
//! (`ecm_audio::adpcm`): 324 bytes of audio, about 8 KB/s per talker.
//!
//! ```text
//!  0  2  magic "DV"
//!  2  1  version (1)
//!  3  1  kind: 1 = audio to the backend, 2 = keep-alive to the backend,
//!              3 = audio from the backend (forwarded)
//!  4  4  token (to the backend, from VOICE) or the talker's sid (from it), big endian
//!  8  2  sequence number
//! 10  2  samples
//! 12  .. IMA ADPCM block (kind 1 and 3)
//! ```

pub use ecm_rtp::echo::EchoGuard;
pub use ecm_rtp::gate::VoiceGate;

/// Voice sample rate.
pub const RATE: u32 = 16_000;
/// Samples per packet (40 ms).
pub const FRAME: usize = 640;
/// A client sends a keep-alive this often while in a voice channel (so the backend knows
/// where to send others' audio before it talks).
pub const KEEPALIVE_MS: u64 = 1000;
/// The backend says a talker went quiet after this long without audio from them.
pub const SPEAKING_HOLD_MS: u64 = 400;

const MAGIC: [u8; 2] = *b"DV";
const VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketKind {
    /// Client to backend: microphone audio.
    Audio = 1,
    /// Client to backend: "I'm here" (no audio).
    KeepAlive = 2,
    /// Backend to client: another talker's audio.
    Forwarded = 3,
}

/// One voice datagram.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub kind: PacketKind,
    /// Token (to the backend) or the talker's sid (from it).
    pub id: u32,
    pub seq: u16,
    pub samples: u16,
    pub adpcm: Vec<u8>,
}

impl Packet {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(12 + self.adpcm.len());
        b.extend_from_slice(&MAGIC);
        b.push(VERSION);
        b.push(self.kind as u8);
        b.extend_from_slice(&self.id.to_be_bytes());
        b.extend_from_slice(&self.seq.to_be_bytes());
        b.extend_from_slice(&self.samples.to_be_bytes());
        b.extend_from_slice(&self.adpcm);
        b
    }

    pub fn decode(b: &[u8]) -> Option<Packet> {
        if b.len() < 12 || b[0..2] != MAGIC || b[2] != VERSION {
            return None;
        }
        let kind = match b[3] {
            1 => PacketKind::Audio,
            2 => PacketKind::KeepAlive,
            3 => PacketKind::Forwarded,
            _ => return None,
        };
        let id = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
        let seq = u16::from_be_bytes([b[8], b[9]]);
        let samples = u16::from_be_bytes([b[10], b[11]]);
        let adpcm = b[12..].to_vec();
        if kind != PacketKind::KeepAlive && (samples == 0 || adpcm.len() < ecm_audio::adpcm::encoded_size(samples as usize)) {
            return None;
        }
        Some(Packet { kind, id, seq, samples, adpcm })
    }

    /// The same audio as the backend forwards it: from talker `sid`.
    pub fn forwarded(&self, sid: u32) -> Packet {
        Packet { kind: PacketKind::Forwarded, id: sid, ..self.clone() }
    }
}

/// The microphone side: cuts captured samples into frames, keeps only what has a
/// voice in it (the [`VoiceGate`]: a level gate with a hang time, so words aren't
/// clipped), and encodes it. The gate's settings (`threshold_db`, `hang_frames`,
/// `muted`) are reached through the sender (`Deref` to [`VoiceGate`]).
pub struct Sender {
    pub token: u32,
    seq: u16,
    pending: Vec<i16>,
    encoder: ecm_audio::adpcm::Encoder,
    pub gate: VoiceGate,
}

impl std::ops::Deref for Sender {
    type Target = VoiceGate;
    fn deref(&self) -> &VoiceGate {
        &self.gate
    }
}

impl std::ops::DerefMut for Sender {
    fn deref_mut(&mut self) -> &mut VoiceGate {
        &mut self.gate
    }
}

impl Sender {
    pub fn new(token: u32) -> Sender {
        Sender { token, seq: 0, pending: Vec::new(), encoder: ecm_audio::adpcm::Encoder::new(), gate: VoiceGate::with(-45, 8) }
    }

    /// Captured samples in; the packets to send out (possibly none).
    pub fn push(&mut self, samples: &[i16]) -> Vec<Packet> {
        self.pending.extend_from_slice(samples);
        let mut out = Vec::new();
        while self.pending.len() >= FRAME {
            let frame: Vec<i16> = self.pending.drain(..FRAME).collect();
            if !self.gate.admit(&frame) {
                continue;
            }
            self.seq = self.seq.wrapping_add(1);
            out.push(Packet { kind: PacketKind::Audio, id: self.token, seq: self.seq, samples: FRAME as u16, adpcm: self.encoder.encode(&frame) });
        }
        out
    }

    /// True while the gate is open (we are being heard).
    pub fn talking(&self) -> bool {
        self.gate.talking()
    }

    pub fn keepalive(&self) -> Packet {
        Packet { kind: PacketKind::KeepAlive, id: self.token, seq: self.seq, samples: 0, adpcm: Vec::new() }
    }
}

/// The speaker side: a small jitter buffer per talker, mixed into one stream
/// ([`ecm_rtp::jitter::Mixer`], fed with forwarded packets). Its settings and queries
/// (`prebuffer`, `max_queue`, `pull`, `forget`, `clear`, `stats`, `queued`) are reached
/// through `Deref`.
pub struct Mixer {
    inner: ecm_rtp::jitter::Mixer,
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer { inner: ecm_rtp::jitter::Mixer::with_limits(FRAME * 2, FRAME * 10) }
    }
}

impl std::ops::Deref for Mixer {
    type Target = ecm_rtp::jitter::Mixer;
    fn deref(&self) -> &ecm_rtp::jitter::Mixer {
        &self.inner
    }
}

impl std::ops::DerefMut for Mixer {
    fn deref_mut(&mut self) -> &mut ecm_rtp::jitter::Mixer {
        &mut self.inner
    }
}

impl Mixer {
    pub fn new() -> Mixer {
        Mixer::default()
    }

    /// A forwarded packet arrived (other kinds are ignored).
    pub fn push(&mut self, p: &Packet) {
        if p.kind != PacketKind::Forwarded {
            return;
        }
        let Some(pcm) = ecm_audio::adpcm::decode(&p.adpcm, p.samples as usize) else { return };
        self.inner.push_frame(p.id, p.seq, &pcm);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, amp: f64) -> Vec<i16> {
        (0..n).map(|i| ((i as f64 * 440.0 * std::f64::consts::TAU / RATE as f64).sin() * amp) as i16).collect()
    }

    #[test]
    fn packets_round_trip_and_reject_garbage() {
        let mut s = Sender::new(77);
        let p = s.push(&tone(FRAME, 8000.0)).pop().unwrap();
        assert_eq!(p.kind, PacketKind::Audio);
        assert_eq!(p.id, 77);
        let bytes = p.encode();
        assert_eq!(bytes.len(), 12 + 324);
        assert_eq!(Packet::decode(&bytes), Some(p.clone()));
        let f = p.forwarded(3);
        assert_eq!(Packet::decode(&f.encode()).unwrap().kind, PacketKind::Forwarded);
        assert_eq!(Packet::decode(&s.keepalive().encode()).unwrap().kind, PacketKind::KeepAlive);
        assert!(Packet::decode(b"XX\x01\x01aaaaaaaa").is_none());
        assert!(Packet::decode(&bytes[..20]).is_none(), "truncated audio");
    }

    #[test]
    fn the_gate_sends_voice_not_hiss_and_holds_over_pauses() {
        let mut s = Sender::new(1);
        // Hiss (-75 dBFS): nothing.
        assert!(s.push(&tone(FRAME * 3, 6.0)).is_empty());
        assert!(!s.talking());
        // A word: sent, then the hang time keeps a few quiet frames (word endings).
        assert_eq!(s.push(&tone(FRAME * 2, 8000.0)).len(), 2);
        let tail = s.push(&tone(FRAME * 20, 6.0));
        assert_eq!(tail.len(), s.hang_frames as usize);
        assert!(!s.talking());
        // Partial frames wait for the rest.
        assert!(s.push(&tone(FRAME / 2, 8000.0)).is_empty());
        assert_eq!(s.push(&tone(FRAME / 2, 8000.0)).len(), 1);
        // Muted: nothing at all.
        s.muted = true;
        assert!(s.push(&tone(FRAME * 3, 8000.0)).is_empty());
    }

    #[test]
    fn the_mixer_primes_mixes_and_counts_losses() {
        let mut tx_a = Sender::new(1);
        let mut tx_b = Sender::new(2);
        let mut m = Mixer::new();
        let a = tx_a.push(&tone(FRAME * 3, 4000.0));
        let b = tx_b.push(&tone(FRAME * 3, 4000.0));
        m.push(&a[0].forwarded(10));
        // One frame is less than the prebuffer: silence for now.
        assert!(m.pull(FRAME).is_empty());
        m.push(&a[1].forwarded(10));
        let first = m.pull(FRAME);
        assert_eq!(first.len(), FRAME);
        // Two talkers at once add up (the same words from both: twice as loud).
        let mut both_m = Mixer::new();
        for p in a.iter().take(2) {
            both_m.push(&p.forwarded(1));
        }
        for p in b.iter().take(2) {
            both_m.push(&p.forwarded(2));
        }
        let mut one_m = Mixer::new();
        for p in a.iter().take(2) {
            one_m.push(&p.forwarded(1));
        }
        let both = both_m.pull(FRAME);
        let one = one_m.pull(FRAME);
        assert_eq!(both.len(), FRAME);
        let peak = |v: &[i16]| v.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak(&both) > peak(&one) * 3 / 2, "{} vs {}", peak(&both), peak(&one));
        assert_eq!(first, one, "the same talker decodes the same");
        // A gap in the sequence counts a loss; a duplicate is dropped.
        m.push(&a[2].forwarded(10).clone());
        m.push(&a[2].forwarded(10));
        assert_eq!(m.stats(10), (3, 0));
        let mut skip = a[2].forwarded(10);
        skip.seq = skip.seq.wrapping_add(3);
        m.push(&skip);
        assert_eq!(m.stats(10).1, 2);
        m.forget(10);
        assert_eq!(m.stats(10), (0, 0));
    }
}
