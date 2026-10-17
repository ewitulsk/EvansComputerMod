//! RTP (RFC 3550): the fixed header, a minimal RTCP (one Sender Report or Receiver
//! Report with at most one report block) and [`RxStats`], the receiver's loss and
//! jitter bookkeeping (RFC 3550 appendix A.3 and A.8).
//!
//! ```text
//!  0                   1                   2                   3
//!  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |V=2|P|X|  CC   |M|     PT      |       sequence number         |
//! |                           timestamp                           |
//! |           synchronization source (SSRC) identifier            |
//! |            contributing source (CSRC) identifiers (CC x)      |
//! |   [extension: profile(16) length(16, words) data]  payload ... [padding, count]
//! ```
//!
//! Decoding is strict (version 2, every length checked) and never panics on garbage.
//! Header extensions are skipped; padding is removed from the payload.

/// The RTP version this module speaks.
pub const VERSION: u8 = 2;
/// Bytes of the fixed RTP header.
pub const HEADER_BYTES: usize = 12;

/// One RTP packet.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RtpPacket {
    pub marker: bool,
    /// 0..=127.
    pub payload_type: u8,
    pub seq: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    /// Up to 15 contributing sources.
    pub csrc: Vec<u32>,
    pub payload: Vec<u8>,
}

impl RtpPacket {
    pub fn new(
        payload_type: u8,
        seq: u16,
        timestamp: u32,
        ssrc: u32,
        payload: Vec<u8>,
    ) -> RtpPacket {
        RtpPacket {
            marker: false,
            payload_type,
            seq,
            timestamp,
            ssrc,
            csrc: Vec::new(),
            payload,
        }
    }

    /// The datagram (no padding, no extension). Extra CSRCs past 15 are dropped and the
    /// payload type is masked to 7 bits.
    pub fn encode(&self) -> Vec<u8> {
        let cc = self.csrc.len().min(15);
        let mut b = Vec::with_capacity(HEADER_BYTES + cc * 4 + self.payload.len());
        b.push((VERSION << 6) | cc as u8);
        b.push(((self.marker as u8) << 7) | (self.payload_type & 0x7f));
        b.extend_from_slice(&self.seq.to_be_bytes());
        b.extend_from_slice(&self.timestamp.to_be_bytes());
        b.extend_from_slice(&self.ssrc.to_be_bytes());
        for c in &self.csrc[..cc] {
            b.extend_from_slice(&c.to_be_bytes());
        }
        b.extend_from_slice(&self.payload);
        b
    }

    /// Parse a datagram; None unless it is a well-formed RTP version 2 packet.
    pub fn decode(b: &[u8]) -> Option<RtpPacket> {
        if b.len() < HEADER_BYTES || b[0] >> 6 != VERSION {
            return None;
        }
        let padding = b[0] & 0x20 != 0;
        let extension = b[0] & 0x10 != 0;
        let cc = (b[0] & 0x0f) as usize;
        let marker = b[1] & 0x80 != 0;
        let payload_type = b[1] & 0x7f;
        let seq = be16(b, 2)?;
        let timestamp = be32(b, 4)?;
        let ssrc = be32(b, 8)?;
        let mut pos = HEADER_BYTES;
        let mut csrc = Vec::with_capacity(cc);
        for _ in 0..cc {
            csrc.push(be32(b, pos)?);
            pos += 4;
        }
        if extension {
            let words = be16(b, pos + 2)? as usize;
            pos += 4 + words * 4;
            if pos > b.len() {
                return None;
            }
        }
        let mut end = b.len();
        if padding {
            let n = *b.last()? as usize;
            if n == 0 || n > end - pos {
                return None;
            }
            end -= n;
        }
        Some(RtpPacket {
            marker,
            payload_type,
            seq,
            timestamp,
            ssrc,
            csrc,
            payload: b[pos..end].to_vec(),
        })
    }
}

/// True if a datagram on a shared port looks like RTCP rather than RTP (RFC 5761:
/// the second byte is an RTCP packet type, 192..=223).
pub fn is_rtcp(b: &[u8]) -> bool {
    b.len() >= 8 && b[0] >> 6 == VERSION && (192..=223).contains(&b[1])
}

fn be16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn be32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

// ---------------------------------------------------------------- RTCP

/// RTCP packet type: Sender Report.
pub const RTCP_SR: u8 = 200;
/// RTCP packet type: Receiver Report.
pub const RTCP_RR: u8 = 201;

/// One reception report block (24 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ReportBlock {
    /// The source this block reports on.
    pub ssrc: u32,
    /// Fraction lost since the last report, 1/256ths.
    pub fraction_lost: u8,
    /// Cumulative packets lost (24-bit signed on the wire; duplicates can make it negative).
    pub cumulative_lost: i32,
    /// Extended highest sequence number received (cycles << 16 | seq).
    pub highest_seq: u32,
    /// Interarrival jitter, timestamp units.
    pub jitter: u32,
    /// Middle 32 bits of the last SR's NTP timestamp (0 if none).
    pub lsr: u32,
    /// Delay since the last SR, 1/65536 s.
    pub dlsr: u32,
}

impl ReportBlock {
    /// Fraction lost as a percentage.
    pub fn loss_percent(&self) -> f32 {
        self.fraction_lost as f32 * 100.0 / 256.0
    }

    fn encode(&self, b: &mut Vec<u8>) {
        b.extend_from_slice(&self.ssrc.to_be_bytes());
        let lost = (self.cumulative_lost.clamp(-0x80_0000, 0x7f_ffff) as u32) & 0xff_ffff;
        b.extend_from_slice(&(((self.fraction_lost as u32) << 24) | lost).to_be_bytes());
        b.extend_from_slice(&self.highest_seq.to_be_bytes());
        b.extend_from_slice(&self.jitter.to_be_bytes());
        b.extend_from_slice(&self.lsr.to_be_bytes());
        b.extend_from_slice(&self.dlsr.to_be_bytes());
    }

    fn decode(b: &[u8], o: usize) -> Option<ReportBlock> {
        let w = be32(b, o + 4)?;
        let lost = (w & 0xff_ffff) as i32;
        Some(ReportBlock {
            ssrc: be32(b, o)?,
            fraction_lost: (w >> 24) as u8,
            cumulative_lost: if lost & 0x80_0000 != 0 {
                lost - 0x100_0000
            } else {
                lost
            },
            highest_seq: be32(b, o + 8)?,
            jitter: be32(b, o + 12)?,
            lsr: be32(b, o + 16)?,
            dlsr: be32(b, o + 20)?,
        })
    }
}

/// A Sender Report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SenderReport {
    pub ssrc: u32,
    /// NTP timestamp (32.32 fixed point seconds).
    pub ntp: u64,
    /// The same instant in RTP timestamp units.
    pub rtp_ts: u32,
    pub packet_count: u32,
    pub octet_count: u32,
    pub report: Option<ReportBlock>,
}

/// A Receiver Report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ReceiverReport {
    pub ssrc: u32,
    pub report: Option<ReportBlock>,
}

/// The RTCP packets this module understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rtcp {
    Sr(SenderReport),
    Rr(ReceiverReport),
}

impl Rtcp {
    pub fn encode(&self) -> Vec<u8> {
        let (pt, report) = match self {
            Rtcp::Sr(s) => (RTCP_SR, s.report),
            Rtcp::Rr(r) => (RTCP_RR, r.report),
        };
        let mut b = vec![(VERSION << 6) | report.is_some() as u8, pt, 0, 0];
        match self {
            Rtcp::Sr(s) => {
                b.extend_from_slice(&s.ssrc.to_be_bytes());
                b.extend_from_slice(&s.ntp.to_be_bytes());
                b.extend_from_slice(&s.rtp_ts.to_be_bytes());
                b.extend_from_slice(&s.packet_count.to_be_bytes());
                b.extend_from_slice(&s.octet_count.to_be_bytes());
            }
            Rtcp::Rr(r) => b.extend_from_slice(&r.ssrc.to_be_bytes()),
        }
        if let Some(rb) = report {
            rb.encode(&mut b);
        }
        let words = (b.len() / 4 - 1) as u16;
        b[2..4].copy_from_slice(&words.to_be_bytes());
        b
    }

    /// The first packet of a (possibly compound) RTCP datagram, if it is an SR or RR.
    /// Of several report blocks only the first is kept.
    pub fn decode(b: &[u8]) -> Option<Rtcp> {
        if b.len() < 8 || b[0] >> 6 != VERSION {
            return None;
        }
        let rc = (b[0] & 0x1f) as usize;
        let len = (be16(b, 2)? as usize + 1) * 4;
        if len > b.len() {
            return None;
        }
        let b = &b[..len];
        let ssrc = be32(b, 4)?;
        match b[1] {
            RTCP_SR => {
                if len < 28 + rc * 24 {
                    return None;
                }
                let report = if rc > 0 {
                    Some(ReportBlock::decode(b, 28)?)
                } else {
                    None
                };
                Some(Rtcp::Sr(SenderReport {
                    ssrc,
                    ntp: ((be32(b, 8)? as u64) << 32) | be32(b, 12)? as u64,
                    rtp_ts: be32(b, 16)?,
                    packet_count: be32(b, 20)?,
                    octet_count: be32(b, 24)?,
                    report,
                }))
            }
            RTCP_RR => {
                if len < 8 + rc * 24 {
                    return None;
                }
                let report = if rc > 0 {
                    Some(ReportBlock::decode(b, 8)?)
                } else {
                    None
                };
                Some(Rtcp::Rr(ReceiverReport { ssrc, report }))
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- receive statistics

const RTP_SEQ_MOD: u32 = 1 << 16;
const MAX_DROPOUT: u16 = 3000;
const MAX_MISORDER: u16 = 100;
/// Sequential packets needed before a source is believed (RFC 3550 A.1).
pub const MIN_SEQUENTIAL: u32 = 2;

/// A receiver's statistics for one source: RFC 3550 A.3 sequence tracking (with
/// probation, wrap-around, resync after a big jump) and the A.8 jitter estimate.
#[derive(Clone, Debug)]
pub struct RxStats {
    clock_rate: u32,
    started: bool,
    max_seq: u16,
    cycles: u32,
    base_seq: u32,
    bad_seq: u32,
    probation: u32,
    received: u32,
    expected_prior: u32,
    received_prior: u32,
    /// Last relative transit time (timestamp units), None before the first valid packet.
    transit: Option<u32>,
    /// Jitter, scaled by 16 (A.8's integer form).
    jitter_q4: u32,
}

impl RxStats {
    /// Statistics for a source whose RTP clock runs at `clock_rate` Hz.
    pub fn new(clock_rate: u32) -> RxStats {
        RxStats {
            clock_rate: clock_rate.max(1),
            started: false,
            max_seq: 0,
            cycles: 0,
            base_seq: 0,
            bad_seq: RTP_SEQ_MOD + 1,
            probation: 0,
            received: 0,
            expected_prior: 0,
            received_prior: 0,
            transit: None,
            jitter_q4: 0,
        }
    }

    fn init_seq(&mut self, seq: u16) {
        self.base_seq = seq as u32;
        self.max_seq = seq;
        self.bad_seq = RTP_SEQ_MOD + 1;
        self.cycles = 0;
        self.received = 0;
        self.received_prior = 0;
        self.expected_prior = 0;
    }

    /// A packet arrived at `arrival_ms` (any monotonic clock). Returns false while the
    /// source is on probation or for a packet A.3 discards (a far jump, until confirmed).
    pub fn on_packet(&mut self, seq: u16, timestamp: u32, arrival_ms: u64) -> bool {
        if !self.started {
            self.started = true;
            self.init_seq(seq);
            self.max_seq = seq.wrapping_sub(1);
            self.probation = MIN_SEQUENTIAL;
        }
        if !self.update_seq(seq) {
            return false;
        }
        // A.8: interarrival jitter.
        let arrival = (arrival_ms as u128 * self.clock_rate as u128 / 1000) as u32;
        let transit = arrival.wrapping_sub(timestamp);
        if let Some(last) = self.transit {
            let d = (transit.wrapping_sub(last) as i32).unsigned_abs();
            let j = self.jitter_q4 as i64 + d as i64 - ((self.jitter_q4 as i64 + 8) >> 4);
            self.jitter_q4 = j.clamp(0, u32::MAX as i64) as u32;
        }
        self.transit = Some(transit);
        true
    }

    fn update_seq(&mut self, seq: u16) -> bool {
        let udelta = seq.wrapping_sub(self.max_seq);
        if self.probation > 0 {
            if seq == self.max_seq.wrapping_add(1) {
                self.probation -= 1;
                self.max_seq = seq;
                if self.probation == 0 {
                    self.init_seq(seq);
                    self.received += 1;
                    return true;
                }
            } else {
                self.probation = MIN_SEQUENTIAL - 1;
                self.max_seq = seq;
            }
            return false;
        } else if udelta < MAX_DROPOUT {
            if seq < self.max_seq {
                self.cycles += RTP_SEQ_MOD;
            }
            self.max_seq = seq;
        } else if udelta as u32 <= RTP_SEQ_MOD - MAX_MISORDER as u32 {
            if seq as u32 == self.bad_seq {
                // Two sequential packets after a big jump: the other side restarted.
                self.init_seq(seq);
                self.transit = None;
            } else {
                self.bad_seq = (seq as u32 + 1) & (RTP_SEQ_MOD - 1);
                return false;
            }
        }
        // else: a duplicate or a reordered packet, counted but max_seq stays.
        self.received += 1;
        true
    }

    /// Packets counted as received (after probation).
    pub fn received(&self) -> u32 {
        self.received
    }

    /// Extended highest sequence number.
    pub fn extended_max(&self) -> u32 {
        self.cycles.wrapping_add(self.max_seq as u32)
    }

    /// Packets expected since the source was believed.
    pub fn expected(&self) -> u32 {
        if !self.started || self.probation > 0 {
            return 0;
        }
        self.extended_max()
            .wrapping_sub(self.base_seq)
            .wrapping_add(1)
    }

    /// Cumulative packets lost (negative with duplicates).
    pub fn lost(&self) -> i64 {
        self.expected() as i64 - self.received as i64
    }

    /// Percentage of expected packets lost over the whole call.
    pub fn loss_percent(&self) -> f32 {
        let e = self.expected();
        if e == 0 {
            0.0
        } else {
            (self.lost().max(0) as f32 * 100.0 / e as f32).min(100.0)
        }
    }

    /// Interarrival jitter, timestamp units.
    pub fn jitter(&self) -> u32 {
        self.jitter_q4 >> 4
    }

    /// Interarrival jitter, ms.
    pub fn jitter_ms(&self) -> f32 {
        (self.jitter_q4 as f32 / 16.0) * 1000.0 / self.clock_rate as f32
    }

    /// The report block for this source, starting a new interval for the fraction lost.
    pub fn report_block(&mut self, ssrc: u32) -> ReportBlock {
        let expected = self.expected();
        let expected_interval = expected.wrapping_sub(self.expected_prior);
        self.expected_prior = expected;
        let received_interval = self.received.wrapping_sub(self.received_prior);
        self.received_prior = self.received;
        let lost_interval = expected_interval as i64 - received_interval as i64;
        let fraction_lost = if expected_interval == 0 || lost_interval <= 0 {
            0
        } else {
            ((lost_interval << 8) / expected_interval as i64).min(255) as u8
        };
        ReportBlock {
            ssrc,
            fraction_lost,
            cumulative_lost: self.lost().clamp(-0x80_0000, 0x7f_ffff) as i32,
            highest_seq: self.extended_max(),
            jitter: self.jitter(),
            lsr: 0,
            dlsr: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtp_round_trips() {
        let mut p = RtpPacket::new(96, 65535, 0xdead_beef, 0x1234_5678, vec![1, 2, 3, 4, 5]);
        p.marker = true;
        p.csrc = vec![7, 8];
        let b = p.encode();
        assert_eq!(b.len(), 12 + 8 + 5);
        assert_eq!(b[0], 0x82);
        assert_eq!(b[1], 0x80 | 96);
        assert_eq!(RtpPacket::decode(&b), Some(p));
        let empty = RtpPacket::new(0, 1, 2, 3, vec![]);
        assert_eq!(RtpPacket::decode(&empty.encode()), Some(empty));
    }

    #[test]
    fn rtp_skips_extensions_and_strips_padding() {
        let p = RtpPacket::new(8, 10, 20, 30, vec![9, 9, 9]);
        let mut b = p.encode();
        b[0] |= 0x10; // X: insert a one-word extension after the header
        b.splice(12..12, [0xbe, 0xde, 0, 1, 1, 2, 3, 4]);
        b[0] |= 0x20; // P: three bytes of padding
        b.extend_from_slice(&[0, 0, 3]);
        assert_eq!(RtpPacket::decode(&b), Some(p));
    }

    #[test]
    fn rtp_refuses_garbage() {
        assert_eq!(RtpPacket::decode(&[]), None);
        assert_eq!(RtpPacket::decode(&[0x80; 11]), None, "short");
        let good = RtpPacket::new(0, 0, 0, 0, vec![1]).encode();
        let mut v1 = good.clone();
        v1[0] = 0x40;
        assert_eq!(RtpPacket::decode(&v1), None, "version 1");
        let mut cc = good.clone();
        cc[0] |= 0x03;
        assert_eq!(RtpPacket::decode(&cc), None, "CSRCs past the end");
        let mut ext = good.clone();
        ext[0] |= 0x10;
        assert_eq!(RtpPacket::decode(&ext), None, "extension past the end");
        let mut pad = good.clone();
        pad[0] |= 0x20;
        *pad.last_mut().unwrap() = 9;
        assert_eq!(
            RtpPacket::decode(&pad),
            None,
            "padding longer than the payload"
        );
        *pad.last_mut().unwrap() = 0;
        assert_eq!(RtpPacket::decode(&pad), None, "zero padding count");
        // Every truncation and every random-ish byte string: no panic.
        let mut long = RtpPacket {
            csrc: vec![1, 2, 3],
            ..RtpPacket::new(1, 2, 3, 4, vec![5; 40])
        }
        .encode();
        long[0] |= 0x30;
        for n in 0..long.len() {
            let _ = RtpPacket::decode(&long[..n]);
        }
        let mut x: u32 = 1;
        for _ in 0..2000 {
            let v: Vec<u8> = (0..(x % 64) as usize)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    x as u8
                })
                .collect();
            let _ = RtpPacket::decode(&v);
            let _ = Rtcp::decode(&v);
        }
    }

    #[test]
    fn rtcp_round_trips() {
        let rb = ReportBlock {
            ssrc: 5,
            fraction_lost: 64,
            cumulative_lost: -3,
            highest_seq: 0x0001_0010,
            jitter: 77,
            lsr: 9,
            dlsr: 10,
        };
        let sr = Rtcp::Sr(SenderReport {
            ssrc: 1,
            ntp: 0x0102_0304_0506_0708,
            rtp_ts: 160,
            packet_count: 50,
            octet_count: 8200,
            report: Some(rb),
        });
        let b = sr.encode();
        assert_eq!(b.len(), 28 + 24);
        assert_eq!(b[0], 0x81);
        assert_eq!(b[1], 200);
        assert_eq!(u16::from_be_bytes([b[2], b[3]]) as usize, b.len() / 4 - 1);
        assert_eq!(Rtcp::decode(&b), Some(sr));
        assert!(is_rtcp(&b));
        let rr = Rtcp::Rr(ReceiverReport {
            ssrc: 2,
            report: Some(rb),
        });
        assert_eq!(Rtcp::decode(&rr.encode()), Some(rr));
        let bare = Rtcp::Rr(ReceiverReport {
            ssrc: 2,
            report: None,
        });
        assert_eq!(bare.encode().len(), 8);
        assert_eq!(Rtcp::decode(&bare.encode()), Some(bare));
        assert_eq!(rb.loss_percent(), 25.0);
        // A compound datagram: the first packet is decoded.
        let mut compound = rr.encode();
        compound.extend_from_slice(&bare.encode());
        assert_eq!(Rtcp::decode(&compound), Some(rr));
        assert!(!is_rtcp(&RtpPacket::new(96, 0, 0, 0, vec![0; 4]).encode()));
    }

    #[test]
    fn rtcp_refuses_garbage() {
        let b = Rtcp::Rr(ReceiverReport {
            ssrc: 2,
            report: Some(ReportBlock::default()),
        })
        .encode();
        assert_eq!(Rtcp::decode(&b[..b.len() - 1]), None, "length past the end");
        let mut lie = b.clone();
        lie[3] = 1; // says 8 bytes but RC = 1
        assert_eq!(Rtcp::decode(&lie), None);
        let mut other = b.clone();
        other[1] = 202; // SDES
        assert_eq!(Rtcp::decode(&other), None);
    }

    /// Feed `seqs` 20 ms apart (16 kHz clock), each arriving `delay(i)` ms late.
    fn feed(s: &mut RxStats, seqs: &[u16], delay: impl Fn(usize) -> u64) {
        for (i, &q) in seqs.iter().enumerate() {
            s.on_packet(q, q as u32 * 320, 1000 + q as u64 * 20 + delay(i));
        }
    }

    #[test]
    fn rx_stats_count_loss_and_fraction() {
        let mut s = RxStats::new(16000);
        // Probation: the first packet is not believed yet.
        assert!(!s.on_packet(100, 0, 0));
        assert_eq!(s.expected(), 0);
        let seqs: Vec<u16> = (101..=200).filter(|q| q % 10 != 5).collect();
        feed(&mut s, &seqs, |_| 0);
        // 101..=200 believed from 101: 100 expected, 10 lost.
        assert_eq!(s.expected(), 100);
        assert_eq!(s.received(), 90);
        assert_eq!(s.lost(), 10);
        assert!((s.loss_percent() - 10.0).abs() < 0.01);
        assert_eq!(s.jitter(), 0, "perfectly paced");
        let rb = s.report_block(42);
        assert_eq!(rb.ssrc, 42);
        assert_eq!(rb.fraction_lost, 25); // 10/100*256
        assert_eq!(rb.cumulative_lost, 10);
        assert_eq!(rb.highest_seq, 200);
        // Next interval without loss.
        feed(&mut s, &(201..=250).collect::<Vec<_>>(), |_| 0);
        assert_eq!(s.report_block(42).fraction_lost, 0);
    }

    #[test]
    fn rx_stats_wrap_reorder_duplicates_and_restart() {
        let mut s = RxStats::new(8000);
        let seqs: Vec<u16> = (65530..=65535).chain(0..=5).collect();
        for q in &seqs {
            s.on_packet(*q, 0, 0);
        }
        assert_eq!(s.extended_max(), 65536 + 5);
        assert_eq!(s.lost(), 0);
        // A reordered old packet and a duplicate count as received, not lost.
        assert!(s.on_packet(3, 0, 0));
        assert_eq!(s.lost(), -1);
        // A far jump is refused once, believed when followed by its successor.
        assert!(!s.on_packet(30000, 0, 0));
        assert!(s.on_packet(30001, 0, 0));
        assert_eq!(s.expected(), 1);
        assert_eq!(s.received(), 1);
    }

    #[test]
    fn rx_stats_jitter_follows_a8() {
        let mut s = RxStats::new(16000);
        // Alternating 0 / 10 ms late: |D| = 10 ms = 160 units each packet after the first.
        let seqs: Vec<u16> = (0..400).collect();
        feed(&mut s, &seqs, |i| if i % 2 == 0 { 0 } else { 10 });
        assert!((s.jitter() as i64 - 160).abs() <= 2, "{}", s.jitter());
        assert!((s.jitter_ms() - 10.0).abs() < 0.2, "{}", s.jitter_ms());
    }
}
