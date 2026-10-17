//! DTMF, both ways it travels:
//!
//! * Out of band, RFC 4733 telephone events: [`TelephoneEvent`] is the 4-byte payload,
//!   [`DtmfSender`] turns "123#" into the packet sequence (a marker on each event's
//!   first packet, one RTP timestamp per event, the duration growing each packet,
//!   then the end packet three times), [`DtmfReceiver`] yields each digit once
//!   however many packets of it arrive (or are lost).
//! * In band, as audio: [`tone`] / [`dtmf_tone`] make the two-sine keypad tones (the
//!   keypad's local feedback), [`DtmfDetector`] finds them again with the Goertzel
//!   algorithm (an IVR listening to a caller without telephone events).
//!
//! ```text
//!  0                   1                   2                   3
//! |     event     |E|R| volume    |          duration             |
//! ```
//!
//! Events 0-9 are the digits, 10 `*`, 11 `#`, 12-15 `A`-`D`.

use crate::tones::Tone;
use crate::RATE;

/// Keypad rows, Hz.
pub const ROWS: [f32; 4] = [697.0, 770.0, 852.0, 941.0];
/// Keypad columns, Hz.
pub const COLS: [f32; 4] = [1209.0, 1336.0, 1477.0, 1633.0];
const KEYPAD: [[char; 4]; 4] = [
    ['1', '2', '3', 'A'],
    ['4', '5', '6', 'B'],
    ['7', '8', '9', 'C'],
    ['*', '0', '#', 'D'],
];

/// The RFC 4733 event code of a keypad character.
pub fn digit_to_event(c: char) -> Option<u8> {
    match c.to_ascii_uppercase() {
        d @ '0'..='9' => Some(d as u8 - b'0'),
        '*' => Some(10),
        '#' => Some(11),
        d @ 'A'..='D' => Some(d as u8 - b'A' + 12),
        _ => None,
    }
}

/// The keypad character of an event code (0..=15).
pub fn event_to_digit(e: u8) -> Option<char> {
    match e {
        0..=9 => Some((b'0' + e) as char),
        10 => Some('*'),
        11 => Some('#'),
        12..=15 => Some((b'A' + e - 12) as char),
        _ => None,
    }
}

/// The (row, column) frequencies of a keypad character.
pub fn frequencies(c: char) -> Option<(f32, f32)> {
    let c = c.to_ascii_uppercase();
    for (r, row) in KEYPAD.iter().enumerate() {
        if let Some(col) = row.iter().position(|k| *k == c) {
            return Some((ROWS[r], COLS[col]));
        }
    }
    None
}

/// One telephone-event payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TelephoneEvent {
    pub event: u8,
    /// E: the last packet(s) of this event.
    pub end: bool,
    /// Power level, -dBm0 (0..=63).
    pub volume: u8,
    /// Duration so far, RTP timestamp units.
    pub duration: u16,
}

impl TelephoneEvent {
    pub fn encode(&self) -> [u8; 4] {
        let d = self.duration.to_be_bytes();
        [
            self.event,
            ((self.end as u8) << 7) | (self.volume & 0x3f),
            d[0],
            d[1],
        ]
    }

    /// The first 4 bytes of a payload (more is allowed by RFC 4733 for multiple events,
    /// only the first is read).
    pub fn decode(b: &[u8]) -> Option<TelephoneEvent> {
        if b.len() < 4 {
            return None;
        }
        Some(TelephoneEvent {
            event: b[0],
            end: b[1] & 0x80 != 0,
            volume: b[1] & 0x3f,
            duration: u16::from_be_bytes([b[2], b[3]]),
        })
    }

    pub fn digit(&self) -> Option<char> {
        event_to_digit(self.event)
    }
}

/// A telephone-event packet to send: put it in an RTP packet with the telephone-event
/// payload type, this marker and timestamp, the next sequence number, at `at_ms`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DtmfPacket {
    /// When to send it, ms after the [`DtmfSender::packets`] call.
    pub at_ms: u32,
    pub marker: bool,
    pub timestamp: u32,
    pub event: TelephoneEvent,
}

/// Turns digit strings into RFC 4733 packet sequences.
#[derive(Clone, Debug)]
pub struct DtmfSender {
    /// RTP clock rate of the stream, Hz (the telephone-event clock is the audio's).
    pub clock_rate: u32,
    /// How long each digit lasts, ms.
    pub digit_ms: u32,
    /// Silence between digits, ms.
    pub gap_ms: u32,
    /// Packet interval, ms.
    pub ptime_ms: u32,
    /// Volume, -dBm0.
    pub volume: u8,
    /// Times the end packet is sent.
    pub end_repeats: u32,
    /// RTP timestamp at which the next event starts.
    pub next_timestamp: u32,
}

impl DtmfSender {
    pub fn new(clock_rate: u32, start_timestamp: u32) -> DtmfSender {
        DtmfSender {
            clock_rate,
            digit_ms: 100,
            gap_ms: 60,
            ptime_ms: 20,
            volume: 10,
            end_repeats: 3,
            next_timestamp: start_timestamp,
        }
    }

    fn units(&self, ms: u32) -> u32 {
        (ms as u64 * self.clock_rate as u64 / 1000) as u32
    }

    /// The packets for `digits` (characters that aren't keys are skipped). The stream's
    /// timestamp moves on past the last digit and its gap.
    pub fn packets(&mut self, digits: &str) -> Vec<DtmfPacket> {
        let mut out = Vec::new();
        let ptime = self.ptime_ms.max(1);
        let updates = (self.digit_ms / ptime).max(1);
        let duration = self.units(updates * ptime).min(u16::MAX as u32) as u16;
        let mut t0 = 0u32;
        for c in digits.chars() {
            let Some(event) = digit_to_event(c) else {
                continue;
            };
            let ts = self.next_timestamp;
            for k in 1..updates {
                let d = self.units(k * ptime) as u16;
                out.push(DtmfPacket {
                    at_ms: t0 + k * ptime,
                    marker: k == 1,
                    timestamp: ts,
                    event: TelephoneEvent {
                        event,
                        end: false,
                        volume: self.volume,
                        duration: d,
                    },
                });
            }
            for r in 0..self.end_repeats.max(1) {
                out.push(DtmfPacket {
                    at_ms: t0 + updates * ptime + r * ptime,
                    marker: updates == 1 && r == 0,
                    timestamp: ts,
                    event: TelephoneEvent {
                        event,
                        end: true,
                        volume: self.volume,
                        duration,
                    },
                });
            }
            let span = (updates + self.end_repeats.max(1) - 1) * ptime;
            t0 += span.max(updates * ptime + self.gap_ms);
            self.next_timestamp = ts
                .wrapping_add(duration as u32)
                .wrapping_add(self.units(self.gap_ms));
        }
        out
    }
}

/// Yields each received telephone event's digit once. Events are told apart by their
/// RTP timestamp; the last few are remembered so late retransmissions of an earlier
/// event's end packet are not taken for a new key press.
#[derive(Clone, Debug, Default)]
pub struct DtmfReceiver {
    recent: std::collections::VecDeque<u32>,
}

impl DtmfReceiver {
    pub fn new() -> DtmfReceiver {
        DtmfReceiver::default()
    }

    /// A telephone-event packet with RTP timestamp `timestamp` arrived: its digit the first
    /// time this event (timestamp) is seen, None for its repeats.
    pub fn push(&mut self, timestamp: u32, payload: &[u8]) -> Option<char> {
        let ev = TelephoneEvent::decode(payload)?;
        if self.recent.contains(&timestamp) {
            return None;
        }
        if self.recent.len() >= 8 {
            self.recent.pop_front();
        }
        self.recent.push_back(timestamp);
        ev.digit()
    }
}

/// An endless in-band tone for a key (local keypad feedback), None if `c` isn't a key.
pub fn tone(c: char) -> Option<Tone> {
    let (r, col) = frequencies(c)?;
    Some(Tone::continuous(&[r, col]))
}

/// `ms` of a key's in-band tone at 16 kHz, None if `c` isn't a key.
pub fn dtmf_tone(c: char, ms: u32) -> Option<Vec<i16>> {
    Some(tone(c)?.next_samples(ms as usize * RATE as usize / 1000))
}

/// In-band DTMF at 16 kHz with the Goertzel algorithm: blocks of [`DtmfDetector::BLOCK`]
/// samples (25.6 ms, the classic 205 at 8 kHz), a key is a strong row tone plus a strong
/// column tone holding most of the block's energy with limited twist; it must hold for two
/// blocks and is reported once, until a block without it.
#[derive(Clone, Debug)]
pub struct DtmfDetector {
    pending: Vec<i16>,
    candidate: Option<char>,
    count: u32,
    reported: bool,
    coeffs: [f32; 8],
}

impl Default for DtmfDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl DtmfDetector {
    pub const BLOCK: usize = 410;

    pub fn new() -> DtmfDetector {
        let mut coeffs = [0f32; 8];
        for (i, f) in ROWS.iter().chain(COLS.iter()).enumerate() {
            coeffs[i] = 2.0 * (std::f32::consts::TAU * f / RATE as f32).cos();
        }
        DtmfDetector {
            pending: Vec::new(),
            candidate: None,
            count: 0,
            reported: false,
            coeffs,
        }
    }

    /// Samples in; keys newly detected out.
    pub fn push(&mut self, samples: &[i16]) -> Vec<char> {
        self.pending.extend_from_slice(samples);
        let mut out = Vec::new();
        while self.pending.len() >= Self::BLOCK {
            let block: Vec<i16> = self.pending.drain(..Self::BLOCK).collect();
            let key = self.detect_block(&block);
            if key.is_some() && key == self.candidate {
                self.count += 1;
            } else {
                self.candidate = key;
                self.count = 1;
                self.reported = false;
            }
            if let Some(k) = self.candidate {
                if self.count >= 2 && !self.reported {
                    self.reported = true;
                    out.push(k);
                }
            }
        }
        out
    }

    /// The key in one block, if any.
    pub fn detect_block(&self, x: &[i16]) -> Option<char> {
        let n = x.len() as f32;
        let energy: f32 = x.iter().map(|s| (*s as f32).powi(2)).sum();
        if n == 0.0 || energy / n < 100.0 * 100.0 {
            return None; // too quiet (below about -50 dBFS)
        }
        let mut p = [0f32; 8];
        for (i, c) in self.coeffs.iter().enumerate() {
            let (mut s1, mut s2) = (0f32, 0f32);
            for &v in x {
                let s0 = v as f32 + c * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            // Normalised so a sine holding all the energy gives 0.5.
            p[i] = (s1 * s1 + s2 * s2 - c * s1 * s2) / (n * energy);
        }
        let best = |g: &[f32]| {
            let (i, &v) = g
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap();
            let second = g
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, v)| *v)
                .fold(0.0, f32::max);
            (i, v, second)
        };
        let (r, rp, r2) = best(&p[..4]);
        let (c, cp, c2) = best(&p[4..]);
        let twist = rp.max(cp) / rp.min(cp).max(1e-9);
        if rp < 0.1 || cp < 0.1 || rp + cp < 0.35 || twist > 6.3 || r2 * 6.0 > rp || c2 * 6.0 > cp {
            return None;
        }
        Some(KEYPAD[r][c])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::sine;

    #[test]
    fn events_map_to_keys() {
        for (i, c) in "0123456789*#ABCD".chars().enumerate() {
            assert_eq!(digit_to_event(c), Some(i as u8));
            assert_eq!(event_to_digit(i as u8), Some(c));
        }
        assert_eq!(digit_to_event('a'), Some(12));
        assert_eq!(digit_to_event('x'), None);
        assert_eq!(event_to_digit(16), None);
        assert_eq!(frequencies('5'), Some((770.0, 1336.0)));
        assert_eq!(frequencies('#'), Some((941.0, 1477.0)));
        assert_eq!(frequencies('D'), Some((941.0, 1633.0)));
    }

    #[test]
    fn telephone_event_round_trips() {
        let e = TelephoneEvent {
            event: 11,
            end: true,
            volume: 10,
            duration: 800,
        };
        let b = e.encode();
        assert_eq!(b, [11, 0x80 | 10, 0x03, 0x20]);
        assert_eq!(TelephoneEvent::decode(&b), Some(e));
        assert_eq!(TelephoneEvent::decode(&b[..3]), None);
        // R is ignored, volume is 6 bits.
        assert_eq!(
            TelephoneEvent::decode(&[1, 0x7f, 0, 0]).unwrap(),
            TelephoneEvent {
                event: 1,
                end: false,
                volume: 63,
                duration: 0
            }
        );
    }

    #[test]
    fn the_sender_follows_rfc_4733() {
        let mut s = DtmfSender::new(8000, 1000);
        let p = s.packets("1#");
        // 100 ms at 20 ms: 4 updates (160..640) then 3 end packets (800) per digit.
        assert_eq!(p.len(), 14);
        let one: Vec<_> = p.iter().filter(|x| x.event.event == 1).collect();
        assert!(one.iter().all(|x| x.timestamp == 1000));
        assert_eq!(
            one.iter().map(|x| x.event.duration).collect::<Vec<_>>(),
            vec![160, 320, 480, 640, 800, 800, 800]
        );
        assert_eq!(
            one.iter().map(|x| x.event.end).collect::<Vec<_>>(),
            vec![false, false, false, false, true, true, true]
        );
        assert_eq!(one.iter().filter(|x| x.marker).count(), 1);
        assert!(one[0].marker);
        let hash: Vec<_> = p.iter().filter(|x| x.event.event == 11).collect();
        // The next event starts after the duration and the gap: 1000 + 800 + 480.
        assert!(hash.iter().all(|x| x.timestamp == 2280));
        assert!(hash[0].marker);
        // Send times never go backwards and the second digit starts after the first's end packets.
        assert!(p.windows(2).all(|w| w[0].at_ms <= w[1].at_ms));
        assert!(hash[0].at_ms > one[6].at_ms);
        assert_eq!(s.next_timestamp, 2280 + 800 + 480);
        // Non-keys are skipped.
        assert!(s.packets(" ,x").is_empty());
    }

    #[test]
    fn the_receiver_yields_each_digit_once() {
        let mut s = DtmfSender::new(16000, 0);
        let packets = s.packets("1223");
        let mut r = DtmfReceiver::new();
        let got: String = packets
            .iter()
            .filter_map(|p| r.push(p.timestamp, &p.event.encode()))
            .collect();
        assert_eq!(got, "1223", "repeated keys are separate events");
        // Losing the first packets (and the marker) of an event still yields it, once.
        let mut r = DtmfReceiver::new();
        let got: String = packets
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 7 > 2)
            .filter_map(|(_, p)| r.push(p.timestamp, &p.event.encode()))
            .collect();
        assert_eq!(got, "1223");
        // A late end packet of an earlier event doesn't press its key again.
        assert_eq!(
            r.push(packets[6].timestamp, &packets[6].event.encode()),
            None
        );
        assert_eq!(r.push(99, &[0; 2]), None, "garbage payload");
    }

    #[test]
    fn keypad_tones_are_the_two_sines() {
        let v = dtmf_tone('9', 100).unwrap();
        assert_eq!(v.len(), 1600);
        let p = |hz| crate::test_util::power_at(&v, hz);
        assert!(p(852.0) > 1e6 && p(1477.0) > 1e6);
        assert!(p(941.0) < p(852.0) / 50.0 && p(1336.0) < p(1477.0) / 50.0);
        assert!(dtmf_tone('x', 100).is_none());
    }

    #[test]
    fn goertzel_detects_in_band_digits() {
        let keys = "0123456789*#ABCD55";
        let mut audio = Vec::new();
        for k in keys.chars() {
            audio.extend(dtmf_tone(k, 80).unwrap());
            audio.extend(vec![0i16; 1000]); // 62 ms silence
        }
        // A little noise on top.
        let mut x: u32 = 7;
        for s in audio.iter_mut() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *s = s.saturating_add((x % 200) as i16 - 100);
        }
        let mut d = DtmfDetector::new();
        let mut got = String::new();
        for chunk in audio.chunks(333) {
            got.extend(d.push(chunk));
        }
        assert_eq!(got, keys);
    }

    #[test]
    fn goertzel_ignores_other_sounds() {
        let mut d = DtmfDetector::new();
        assert!(
            d.push(&crate::tones::Tone::ringback().next_samples(16000))
                .is_empty(),
            "440+480"
        );
        assert!(d
            .push(&crate::tones::Tone::dial().next_samples(16000))
            .is_empty());
        assert!(
            d.push(&sine(16000, 852.0, 8000.0)).is_empty(),
            "a row tone alone"
        );
        assert!(d.push(&vec![0; 16000]).is_empty());
        // A key with a strong third tone on top is not a key.
        let mut v = dtmf_tone('1', 200).unwrap();
        for (s, t) in v.iter_mut().zip(sine(3200, 2500.0, 8000.0)) {
            *s = s.saturating_add(t);
        }
        assert!(d.push(&v).is_empty());
        // A quiet key: still found.
        let q = crate::tones::Tone::continuous(&[697.0, 1209.0])
            .with_level(1500.0)
            .next_samples(3200);
        assert_eq!(d.push(&q), vec!['1']);
    }
}
