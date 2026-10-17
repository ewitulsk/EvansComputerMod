//! Voice channels: the UDP packets between the Discord app and a backend, and the
//! app's side of the audio (who is talking, the jitter buffer that mixes everyone
//! for the speaker, the voice-activity gate for the microphone).
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

use std::collections::BTreeMap;

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
/// voice in it (a level gate with a hang time, so words aren't clipped), and encodes it.
pub struct Sender {
    pub token: u32,
    seq: u16,
    pending: Vec<i16>,
    encoder: ecm_audio::adpcm::Encoder,
    /// Frames above this peak (dBFS) open the gate.
    pub threshold_db: i32,
    /// Frames still sent after the last loud one.
    hang: u32,
    pub hang_frames: u32,
    pub muted: bool,
}

impl Sender {
    pub fn new(token: u32) -> Sender {
        Sender {
            token,
            seq: 0,
            pending: Vec::new(),
            encoder: ecm_audio::adpcm::Encoder::new(),
            threshold_db: -45,
            hang: 0,
            hang_frames: 8,
            muted: false,
        }
    }

    /// Captured samples in; the packets to send out (possibly none).
    pub fn push(&mut self, samples: &[i16]) -> Vec<Packet> {
        self.pending.extend_from_slice(samples);
        let mut out = Vec::new();
        while self.pending.len() >= FRAME {
            let frame: Vec<i16> = self.pending.drain(..FRAME).collect();
            if self.muted {
                self.hang = 0;
                continue;
            }
            if ecm_audio::peak_dbfs(&frame) >= self.threshold_db {
                self.hang = self.hang_frames;
            } else if self.hang > 0 {
                self.hang -= 1;
            } else {
                continue;
            }
            self.seq = self.seq.wrapping_add(1);
            out.push(Packet { kind: PacketKind::Audio, id: self.token, seq: self.seq, samples: FRAME as u16, adpcm: self.encoder.encode(&frame) });
        }
        out
    }

    /// True while the gate is open (we are being heard).
    pub fn talking(&self) -> bool {
        self.hang > 0 && !self.muted
    }

    pub fn keepalive(&self) -> Packet {
        Packet { kind: PacketKind::KeepAlive, id: self.token, seq: self.seq, samples: 0, adpcm: Vec::new() }
    }
}

/// One talker's queue in the [`Mixer`].
#[derive(Default)]
struct Talker {
    queue: std::collections::VecDeque<i16>,
    last_seq: Option<u16>,
    /// Waiting for [`Mixer::prebuffer`] samples before playing (after silence or a start).
    priming: bool,
    received: u64,
    lost: u64,
}

/// The speaker side: a small jitter buffer per talker, mixed into one stream.
pub struct Mixer {
    talkers: BTreeMap<u32, Talker>,
    /// Samples a talker queues before it is heard (absorbs network jitter).
    pub prebuffer: usize,
    /// Most samples a talker may queue (older ones are dropped: we are late).
    pub max_queue: usize,
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer { talkers: BTreeMap::new(), prebuffer: FRAME * 2, max_queue: FRAME * 10 }
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
        let t = self.talkers.entry(p.id).or_insert_with(|| Talker { priming: true, ..Default::default() });
        if let Some(last) = t.last_seq {
            let gap = p.seq.wrapping_sub(last);
            if gap == 0 || gap > 0x8000 {
                return; // a duplicate or a late, reordered packet
            }
            t.lost += (gap - 1) as u64;
        }
        t.last_seq = Some(p.seq);
        t.received += 1;
        t.queue.extend(pcm);
        while t.queue.len() > self.max_queue {
            t.queue.pop_front();
        }
    }

    /// Up to `n` samples of everyone mixed, or fewer (none when nobody is talking).
    /// A talker whose queue ran dry primes again.
    pub fn pull(&mut self, n: usize) -> Vec<i16> {
        let mut len = 0;
        for t in self.talkers.values_mut() {
            if t.priming && t.queue.len() >= self.prebuffer {
                t.priming = false;
            }
            if !t.priming {
                len = len.max(t.queue.len().min(n));
            }
        }
        if len == 0 {
            return Vec::new();
        }
        let mut mix = vec![0i32; len];
        for t in self.talkers.values_mut() {
            if t.priming {
                continue;
            }
            for (k, s) in t.queue.drain(..t.queue.len().min(len)).enumerate() {
                mix[k] += s as i32;
            }
            if t.queue.is_empty() {
                t.priming = true;
            }
        }
        mix.into_iter().map(|s| s.clamp(-32768, 32767) as i16).collect()
    }

    /// A talker left: drop their queue.
    pub fn forget(&mut self, sid: u32) {
        self.talkers.remove(&sid);
    }

    pub fn clear(&mut self) {
        self.talkers.clear();
    }

    /// Packets received and lost (sequence gaps) from a talker.
    pub fn stats(&self, sid: u32) -> (u64, u64) {
        self.talkers.get(&sid).map(|t| (t.received, t.lost)).unwrap_or((0, 0))
    }

    /// Samples waiting across all talkers (for tests and the status line).
    pub fn queued(&self) -> usize {
        self.talkers.values().map(|t| t.queue.len()).max().unwrap_or(0)
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
