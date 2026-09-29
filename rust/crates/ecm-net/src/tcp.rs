//! TCP wire format, states and sequence-number helpers. The connection state
//! machine lives in `tcb.rs`.

use super::checksum::pseudo_header_checksum;
use super::ipv4::PROTO_TCP;
use super::types::Ipv4Addr;

pub const FIN: u8 = 0x01;
pub const SYN: u8 = 0x02;
pub const RST: u8 = 0x04;
pub const PSH: u8 = 0x08;
pub const ACK: u8 = 0x10;
pub const URG: u8 = 0x20;

/// Our MSS for a 1500-byte MTU.
pub const TCP_MSS: u16 = 1460;
/// MSS assumed when the peer sends no MSS option (RFC 9293).
pub const DEFAULT_MSS: u16 = 536;

/// TCP connection state (RFC 9293).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}

impl TcpState {
    /// netstat-style name.
    pub fn as_str(&self) -> &'static str {
        match self {
            TcpState::Closed => "CLOSED",
            TcpState::Listen => "LISTEN",
            TcpState::SynSent => "SYN_SENT",
            TcpState::SynReceived => "SYN_RECV",
            TcpState::Established => "ESTABLISHED",
            TcpState::FinWait1 => "FIN_WAIT1",
            TcpState::FinWait2 => "FIN_WAIT2",
            TcpState::CloseWait => "CLOSE_WAIT",
            TcpState::Closing => "CLOSING",
            TcpState::LastAck => "LAST_ACK",
            TcpState::TimeWait => "TIME_WAIT",
        }
    }
}

/// Parsed TCP header, including the MSS option if present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq_num: u32,
    pub ack_num: u32,
    /// Header length in 32-bit words.
    pub data_offset: u8,
    pub flags: u8,
    pub window: u16,
    pub checksum: u16,
    pub urgent_ptr: u16,
    pub mss: Option<u16>,
}

impl TcpHeader {
    pub const MIN_SIZE: usize = 20;

    /// Parse; validates the data offset. Does not verify the checksum (see [`verify_checksum`]).
    /// Malformed options end option parsing but do not reject the segment.
    pub fn parse(data: &[u8]) -> Option<(Self, &[u8])> {
        let h = data.get(..20)?;
        let data_offset = h[12] >> 4;
        let header_len = data_offset as usize * 4;
        if header_len < 20 {
            return None;
        }
        let options = data.get(20..header_len)?;
        let payload = data.get(header_len..)?;
        let hdr = TcpHeader {
            src_port: u16::from_be_bytes([h[0], h[1]]),
            dst_port: u16::from_be_bytes([h[2], h[3]]),
            seq_num: u32::from_be_bytes([h[4], h[5], h[6], h[7]]),
            ack_num: u32::from_be_bytes([h[8], h[9], h[10], h[11]]),
            data_offset,
            flags: h[13],
            window: u16::from_be_bytes([h[14], h[15]]),
            checksum: u16::from_be_bytes([h[16], h[17]]),
            urgent_ptr: u16::from_be_bytes([h[18], h[19]]),
            mss: parse_mss(options),
        };
        Some((hdr, payload))
    }

    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }
}

fn parse_mss(mut opts: &[u8]) -> Option<u16> {
    let mut mss = None;
    while let Some((&kind, rest)) = opts.split_first() {
        match kind {
            0 => break,
            1 => opts = rest,
            _ => {
                let &len = rest.first()?;
                let len = len as usize;
                if len < 2 {
                    break;
                }
                let body = opts.get(2..len)?;
                if kind == 2 && body.len() == 2 {
                    mss = Some(u16::from_be_bytes([body[0], body[1]]));
                }
                opts = opts.get(len..)?;
            }
        }
    }
    mss
}

/// Build a segment (header [+ MSS option] + payload) with checksum.
#[allow(clippy::too_many_arguments)]
pub fn build_segment(
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
    src_port: u16,
    dst_port: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    window: u16,
    mss: Option<u16>,
    payload: &[u8],
) -> Vec<u8> {
    let hl = if mss.is_some() { 24 } else { 20 };
    let mut b = vec![0u8; hl + payload.len()];
    b[0..2].copy_from_slice(&src_port.to_be_bytes());
    b[2..4].copy_from_slice(&dst_port.to_be_bytes());
    b[4..8].copy_from_slice(&seq.to_be_bytes());
    b[8..12].copy_from_slice(&ack.to_be_bytes());
    b[12] = ((hl / 4) as u8) << 4;
    b[13] = flags;
    b[14..16].copy_from_slice(&window.to_be_bytes());
    if let Some(m) = mss {
        b[20] = 2;
        b[21] = 4;
        b[22..24].copy_from_slice(&m.to_be_bytes());
    }
    b[hl..].copy_from_slice(payload);
    let c = pseudo_header_checksum(&src_ip, &dst_ip, PROTO_TCP, &b);
    b[16..18].copy_from_slice(&c.to_be_bytes());
    b
}

/// Verify the checksum of a full segment.
pub fn verify_checksum(src_ip: &Ipv4Addr, dst_ip: &Ipv4Addr, segment: &[u8]) -> bool {
    segment.len() >= 20 && pseudo_header_checksum(src_ip, dst_ip, PROTO_TCP, segment) == 0
}

// ---- sequence number arithmetic (mod 2^32) ----

#[inline]
pub fn seq_lt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}
#[inline]
pub fn seq_le(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) <= 0
}
#[inline]
pub fn seq_gt(a: u32, b: u32) -> bool {
    seq_lt(b, a)
}
#[inline]
pub fn seq_ge(a: u32, b: u32) -> bool {
    seq_le(b, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const D: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);

    #[test]
    fn roundtrip_with_mss() {
        let s = build_segment(S, D, 1234, 80, 0xfffffff0, 5, SYN | ACK, 8192, Some(1400), b"");
        assert_eq!(s.len(), 24);
        assert!(verify_checksum(&S, &D, &s));
        let (h, p) = TcpHeader::parse(&s).unwrap();
        assert_eq!((h.src_port, h.dst_port, h.seq_num, h.ack_num), (1234, 80, 0xfffffff0, 5));
        assert!(h.has(SYN) && h.has(ACK) && !h.has(FIN));
        assert_eq!(h.mss, Some(1400));
        assert!(p.is_empty());
        let s = build_segment(S, D, 1, 2, 3, 4, ACK, 5, None, b"data");
        let (h, p) = TcpHeader::parse(&s).unwrap();
        assert_eq!(h.mss, None);
        assert_eq!(p, b"data");
        let mut bad = s.clone();
        bad[21] ^= 1;
        assert!(!verify_checksum(&S, &D, &bad));
    }

    #[test]
    fn truncated_and_garbage() {
        let s = build_segment(S, D, 1, 2, 3, 4, SYN, 5, Some(1460), b"");
        for n in 0..24 {
            assert!(TcpHeader::parse(&s[..n]).is_none(), "len {n}");
        }
        let mut g = s.clone();
        g[12] = 0x40; // offset 4 words < 5
        assert!(TcpHeader::parse(&g).is_none());
        let mut g = s.clone();
        g[12] = 0xf0; // offset 60 > len
        assert!(TcpHeader::parse(&g).is_none());
        // malformed options: zero length, truncated length, NOPs
        for opts in [[2u8, 0, 0, 0], [2, 9, 0, 0], [1, 1, 1, 1], [8, 3, 1, 1], [2, 4, 0xff, 0xff]] {
            let mut g = s.clone();
            g[20..24].copy_from_slice(&opts);
            let (h, _) = TcpHeader::parse(&g).unwrap();
            if opts == [2, 4, 0xff, 0xff] {
                assert_eq!(h.mss, Some(0xffff));
            } else {
                assert_eq!(h.mss, None, "{opts:?}");
            }
        }
        assert!(!verify_checksum(&S, &D, &[0; 10]));
    }

    #[test]
    fn seq_wraparound() {
        assert!(seq_lt(0xffff_fff0, 5));
        assert!(seq_gt(5, 0xffff_fff0));
        assert!(seq_le(7, 7) && seq_ge(7, 7));
        assert!(!seq_lt(7, 7));
    }
}
