//! UDP header parsing and construction. Sockets live in the stack.

use super::checksum::pseudo_header_checksum;
use super::ipv4::PROTO_UDP;
use super::types::Ipv4Addr;

/// Largest UDP payload that fits one 1500-byte IPv4 packet.
pub const MAX_PAYLOAD: usize = 1472;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UdpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub length: u16,
    pub checksum: u16,
}

impl UdpHeader {
    pub const SIZE: usize = 8;

    /// Parse; validates `8 <= length <= data.len()`. Payload is trimmed to `length`.
    /// Does not verify the checksum (see [`verify_checksum`]).
    pub fn parse(data: &[u8]) -> Option<(Self, &[u8])> {
        let h = data.get(..8)?;
        let length = u16::from_be_bytes([h[4], h[5]]) as usize;
        if length < 8 {
            return None;
        }
        let payload = data.get(8..length)?;
        Some((
            UdpHeader {
                src_port: u16::from_be_bytes([h[0], h[1]]),
                dst_port: u16::from_be_bytes([h[2], h[3]]),
                length: length as u16,
                checksum: u16::from_be_bytes([h[6], h[7]]),
            },
            payload,
        ))
    }

    /// Serialize header + payload into `buf`. Returns total length, or 0 if `buf` is too small
    /// or the payload is larger than a UDP datagram can carry.
    pub fn serialize(buf: &mut [u8], src_port: u16, dst_port: u16, payload: &[u8], src_ip: &Ipv4Addr, dst_ip: &Ipv4Addr) -> usize {
        let total = 8 + payload.len();
        if total > u16::MAX as usize {
            return 0;
        }
        let Some(b) = buf.get_mut(..total) else { return 0 };
        b[0..2].copy_from_slice(&src_port.to_be_bytes());
        b[2..4].copy_from_slice(&dst_port.to_be_bytes());
        b[4..6].copy_from_slice(&(total as u16).to_be_bytes());
        b[6] = 0;
        b[7] = 0;
        b[8..].copy_from_slice(payload);
        let mut c = pseudo_header_checksum(src_ip, dst_ip, PROTO_UDP, b);
        if c == 0 {
            c = 0xffff;
        }
        b[6..8].copy_from_slice(&c.to_be_bytes());
        total
    }
}

/// Build a UDP datagram (header + payload) with checksum.
pub fn build(src_ip: Ipv4Addr, dst_ip: Ipv4Addr, src_port: u16, dst_port: u16, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 8 + payload.len()];
    let n = UdpHeader::serialize(&mut v, src_port, dst_port, payload, &src_ip, &dst_ip);
    v.truncate(n);
    v
}

/// Verify the checksum of `datagram` (header + payload, exactly `length` bytes).
/// A zero checksum field means "not computed" and is accepted.
pub fn verify_checksum(src_ip: &Ipv4Addr, dst_ip: &Ipv4Addr, datagram: &[u8]) -> bool {
    match datagram.get(6..8) {
        Some([0, 0]) => true,
        Some(_) => pseudo_header_checksum(src_ip, dst_ip, PROTO_UDP, datagram) == 0,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const D: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);

    #[test]
    fn roundtrip() {
        let v = build(S, D, 1000, 53, b"query");
        let (h, p) = UdpHeader::parse(&v).unwrap();
        assert_eq!((h.src_port, h.dst_port, h.length), (1000, 53, 13));
        assert_eq!(p, b"query");
        assert!(verify_checksum(&S, &D, &v));
        assert!(!verify_checksum(&S, &Ipv4Addr::new(10, 0, 0, 3), &v));
        let mut z = v.clone();
        z[6] = 0;
        z[7] = 0;
        assert!(verify_checksum(&S, &D, &z));
        let mut padded = v.clone();
        padded.extend_from_slice(&[0; 5]);
        assert_eq!(UdpHeader::parse(&padded).unwrap().1, b"query");
    }

    #[test]
    fn truncated_and_garbage() {
        let v = build(S, D, 1, 2, b"abcdef");
        for n in 0..v.len() {
            assert!(UdpHeader::parse(&v[..n]).is_none(), "len {n}");
        }
        for bad_len in [0u16, 7, 15, 0xffff] {
            let mut g = v.clone();
            g[4..6].copy_from_slice(&bad_len.to_be_bytes());
            assert!(UdpHeader::parse(&g).is_none(), "length {bad_len}");
        }
        let mut g = v.clone();
        g[9] ^= 1;
        assert!(!verify_checksum(&S, &D, &g));
        assert!(!verify_checksum(&S, &D, &[1, 2, 3]));
        assert_eq!(UdpHeader::serialize(&mut [0u8; 8], 1, 2, b"x", &S, &D), 0);
    }
}
