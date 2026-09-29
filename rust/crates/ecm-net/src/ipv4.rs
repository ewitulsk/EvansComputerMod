//! IPv4 header parsing and construction. Routing lives in the stack.

use super::checksum::internet_checksum;
use super::types::Ipv4Addr;

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;

/// Largest IPv4 payload we emit (MTU 1500 - 20 byte header).
pub const MAX_PAYLOAD: usize = 1480;

/// Why a packet was rejected by [`Ipv4Header::parse_checked`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ipv4Error {
    Truncated,
    BadVersion,
    BadHeaderLength,
    BadTotalLength,
    BadChecksum,
}

/// IPv4 header. Options (if any) are skipped, not interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4Header {
    pub version_ihl: u8,
    pub dscp_ecn: u8,
    pub total_length: u16,
    pub identification: u16,
    pub flags_fragment: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
}

impl Ipv4Header {
    pub const SIZE: usize = 20;

    /// Parse and validate (version, IHL, total length, header checksum).
    /// Returns the header and the payload, trimmed to `total_length` (link padding removed).
    pub fn parse(data: &[u8]) -> Option<(Self, &[u8])> {
        Self::parse_checked(data).ok()
    }

    pub fn parse_checked(data: &[u8]) -> Result<(Self, &[u8]), Ipv4Error> {
        let d = data.get(..20).ok_or(Ipv4Error::Truncated)?;
        let version_ihl = d[0];
        if version_ihl >> 4 != 4 {
            return Err(Ipv4Error::BadVersion);
        }
        let header_len = ((version_ihl & 0x0f) as usize) * 4;
        if header_len < 20 {
            return Err(Ipv4Error::BadHeaderLength);
        }
        let header = data.get(..header_len).ok_or(Ipv4Error::Truncated)?;
        let total_length = u16::from_be_bytes([d[2], d[3]]) as usize;
        if total_length < header_len || total_length > data.len() {
            return Err(Ipv4Error::BadTotalLength);
        }
        if internet_checksum(header) != 0 {
            return Err(Ipv4Error::BadChecksum);
        }
        let hdr = Ipv4Header {
            version_ihl,
            dscp_ecn: d[1],
            total_length: total_length as u16,
            identification: u16::from_be_bytes([d[4], d[5]]),
            flags_fragment: u16::from_be_bytes([d[6], d[7]]),
            ttl: d[8],
            protocol: d[9],
            checksum: u16::from_be_bytes([d[10], d[11]]),
            src: Ipv4Addr::from_bytes(&d[12..16]),
            dst: Ipv4Addr::from_bytes(&d[16..20]),
        };
        let payload = data.get(header_len..total_length).ok_or(Ipv4Error::BadTotalLength)?;
        Ok((hdr, payload))
    }

    pub fn header_len(&self) -> usize {
        ((self.version_ihl & 0x0f) as usize) * 4
    }

    /// More-fragments set or non-zero fragment offset.
    pub fn is_fragment(&self) -> bool {
        self.flags_fragment & 0x2000 != 0 || self.flags_fragment & 0x1fff != 0
    }

    /// Serialize a 20-byte header (options are not written) and fill in the checksum.
    /// Returns 20, or 0 if `buf` is too small.
    pub fn serialize(&self, buf: &mut [u8]) -> usize {
        let Some(b) = buf.get_mut(..20) else { return 0 };
        b[0] = 0x45;
        b[1] = self.dscp_ecn;
        b[2..4].copy_from_slice(&self.total_length.to_be_bytes());
        b[4..6].copy_from_slice(&self.identification.to_be_bytes());
        b[6..8].copy_from_slice(&self.flags_fragment.to_be_bytes());
        b[8] = self.ttl;
        b[9] = self.protocol;
        b[10] = 0;
        b[11] = 0;
        b[12..16].copy_from_slice(&self.src.0);
        b[16..20].copy_from_slice(&self.dst.0);
        let c = internet_checksum(b);
        b[10..12].copy_from_slice(&c.to_be_bytes());
        20
    }

    /// Standard outgoing header: no options, DF set, TTL 64.
    pub fn new_outgoing(src: Ipv4Addr, dst: Ipv4Addr, protocol: u8, payload_len: usize, id: u16) -> Self {
        Ipv4Header {
            version_ihl: 0x45,
            dscp_ecn: 0,
            total_length: (20 + payload_len).min(u16::MAX as usize) as u16,
            identification: id,
            flags_fragment: 0x4000,
            ttl: 64,
            protocol,
            checksum: 0,
            src,
            dst,
        }
    }
}

/// Build a full IPv4 packet (header + payload). `None` if the payload exceeds [`MAX_PAYLOAD`].
pub fn build_packet(src: Ipv4Addr, dst: Ipv4Addr, protocol: u8, id: u16, payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() > MAX_PAYLOAD {
        return None;
    }
    let mut p = vec![0u8; 20 + payload.len()];
    Ipv4Header::new_outgoing(src, dst, protocol, payload.len(), id).serialize(&mut p);
    p[20..].copy_from_slice(payload);
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        build_packet(Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2), PROTO_UDP, 7, &[1, 2, 3, 4]).unwrap()
    }

    #[test]
    fn roundtrip_and_padding() {
        let mut p = sample();
        p.extend_from_slice(&[0xEE; 10]); // link padding
        let (h, pl) = Ipv4Header::parse(&p).unwrap();
        assert_eq!(h.src, Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(h.protocol, PROTO_UDP);
        assert_eq!(pl, &[1, 2, 3, 4]);
        assert!(!h.is_fragment());
        assert!(build_packet(Ipv4Addr::ZERO, Ipv4Addr::ZERO, 1, 0, &[0; 1481]).is_none());
    }

    fn fix_cksum(p: &mut [u8]) {
        let hl = ((p[0] & 0xf) as usize * 4).min(p.len());
        p[10] = 0;
        p[11] = 0;
        let c = internet_checksum(&p[..hl]);
        p[10..12].copy_from_slice(&c.to_be_bytes());
    }

    #[test]
    fn truncated_and_garbage() {
        let p = sample();
        for n in 0..p.len() {
            assert!(Ipv4Header::parse(&p[..n]).is_none(), "len {n}");
        }
        let mut g = p.clone();
        g[0] = 0x65;
        fix_cksum(&mut g);
        assert_eq!(Ipv4Header::parse_checked(&g).err(), Some(Ipv4Error::BadVersion));
        let mut g = p.clone();
        g[0] = 0x44;
        assert_eq!(Ipv4Header::parse_checked(&g).err(), Some(Ipv4Error::BadHeaderLength));
        // total_length smaller than header (the old remote-panic bug)
        let mut g = p.clone();
        g[2] = 0;
        g[3] = 10;
        fix_cksum(&mut g);
        assert_eq!(Ipv4Header::parse_checked(&g).err(), Some(Ipv4Error::BadTotalLength));
        // IHL larger than the packet
        let mut g = p.clone();
        g[0] = 0x4f;
        assert!(Ipv4Header::parse(&g).is_none());
        let mut g = p.clone();
        g[8] ^= 1;
        assert_eq!(Ipv4Header::parse_checked(&g).err(), Some(Ipv4Error::BadChecksum));
    }

    #[test]
    fn options_and_fragments() {
        let mut p = sample();
        // insert 4 bytes of options (NOP x4) -> IHL 6
        p.splice(20..20, [1u8, 1, 1, 1]);
        p[0] = 0x46;
        let tl = p.len() as u16;
        p[2..4].copy_from_slice(&tl.to_be_bytes());
        fix_cksum(&mut p);
        let (h, pl) = Ipv4Header::parse(&p).unwrap();
        assert_eq!(h.header_len(), 24);
        assert_eq!(pl, &[1, 2, 3, 4]);
        let mut f = sample();
        f[6] = 0x20; // MF
        fix_cksum(&mut f);
        assert!(Ipv4Header::parse(&f).unwrap().0.is_fragment());
        let mut f = sample();
        f[7] = 0x01; // offset 1
        fix_cksum(&mut f);
        assert!(Ipv4Header::parse(&f).unwrap().0.is_fragment());
    }
}
