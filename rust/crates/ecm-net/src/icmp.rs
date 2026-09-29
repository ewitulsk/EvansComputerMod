//! ICMP (Internet Control Message Protocol) — echo and error message formats.

use super::checksum::internet_checksum;

pub const ICMP_ECHO_REPLY: u8 = 0;
pub const ICMP_DEST_UNREACHABLE: u8 = 3;
pub const ICMP_ECHO_REQUEST: u8 = 8;
pub const ICMP_TIME_EXCEEDED: u8 = 11;

/// Parsed ICMP header. `id`/`seq` are the "rest of header" bytes (meaningful for echo).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IcmpPacket {
    pub icmp_type: u8,
    pub code: u8,
    pub checksum: u16,
    pub id: u16,
    pub seq: u16,
    pub payload_len: usize,
}

impl IcmpPacket {
    /// Minimum ICMP header size (type + code + checksum + id + seq).
    pub const HEADER_SIZE: usize = 8;

    /// Parse and verify the checksum over the whole message.
    pub fn parse(data: &[u8]) -> Option<(Self, &[u8])> {
        let (h, payload) = data.split_at_checked(8)?;
        if internet_checksum(data) != 0 {
            return None;
        }
        let pkt = IcmpPacket {
            icmp_type: h[0],
            code: h[1],
            checksum: u16::from_be_bytes([h[2], h[3]]),
            id: u16::from_be_bytes([h[4], h[5]]),
            seq: u16::from_be_bytes([h[6], h[7]]),
            payload_len: payload.len(),
        };
        Some((pkt, payload))
    }

    /// Serialize an echo request/reply into `buf`. Returns bytes written, 0 if `buf` is too small.
    pub fn serialize_echo(buf: &mut [u8], icmp_type: u8, id: u16, seq: u16, payload: &[u8]) -> usize {
        let total = 8 + payload.len();
        let Some(b) = buf.get_mut(..total) else { return 0 };
        b[0] = icmp_type;
        b[1] = 0;
        b[2] = 0;
        b[3] = 0;
        b[4..6].copy_from_slice(&id.to_be_bytes());
        b[6..8].copy_from_slice(&seq.to_be_bytes());
        b[8..].copy_from_slice(payload);
        let c = internet_checksum(b);
        b[2..4].copy_from_slice(&c.to_be_bytes());
        total
    }

    /// Build an echo request/reply as a new buffer.
    pub fn build_echo(icmp_type: u8, id: u16, seq: u16, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; 8 + payload.len()];
        Self::serialize_echo(&mut v, icmp_type, id, seq, payload);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let v = IcmpPacket::build_echo(ICMP_ECHO_REQUEST, 0x1234, 7, b"hello");
        let (p, pl) = IcmpPacket::parse(&v).unwrap();
        assert_eq!((p.icmp_type, p.code, p.id, p.seq, p.payload_len), (ICMP_ECHO_REQUEST, 0, 0x1234, 7, 5));
        assert_eq!(pl, b"hello");
        assert_eq!(IcmpPacket::serialize_echo(&mut [0u8; 9], 0, 0, 0, b"ab"), 0);
    }

    #[test]
    fn truncated_and_garbage() {
        let v = IcmpPacket::build_echo(ICMP_ECHO_REPLY, 1, 2, b"xyz");
        for n in 0..8 {
            assert!(IcmpPacket::parse(&v[..n]).is_none());
        }
        let mut g = v.clone();
        g[9] ^= 0xff;
        assert!(IcmpPacket::parse(&g).is_none());
        assert!(IcmpPacket::parse(&[1u8; 64]).is_none());
    }
}
