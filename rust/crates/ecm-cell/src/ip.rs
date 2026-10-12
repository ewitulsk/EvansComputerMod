//! Small IPv4 / Ethernet helpers for the programs' `tun` interfaces: the
//! phone's `wwan0` and the core's interface towards the phones are Ethernet
//! tun devices, so the daemon answers ARP for every address on them (proxy
//! ARP, like a point-to-point link) and moves bare IP packets.

use alloc::vec::Vec;

pub const ETH_HEADER: usize = 14;
pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;

/// The MAC the daemon answers ARP with (locally administered).
pub const PEER_MAC: [u8; 6] = [0x02, 0xce, 0x11, 0xff, 0xff, 0x01];

/// A minimally valid IPv4 packet: version 4, header length, total length within the buffer.
pub fn ipv4_ok(p: &[u8]) -> bool {
    if p.len() < 20 || p[0] >> 4 != 4 {
        return false;
    }
    let ihl = (p[0] & 15) as usize * 4;
    let total = u16::from_be_bytes([p[2], p[3]]) as usize;
    ihl >= 20 && total >= ihl && total <= p.len()
}

pub fn ipv4_src(p: &[u8]) -> Option<[u8; 4]> {
    ipv4_ok(p).then(|| [p[12], p[13], p[14], p[15]])
}

pub fn ipv4_dst(p: &[u8]) -> Option<[u8; 4]> {
    ipv4_ok(p).then(|| [p[16], p[17], p[18], p[19]])
}

/// The packet cut to its IPv4 total length (Ethernet padding removed).
pub fn ipv4_trim(p: &[u8]) -> Option<&[u8]> {
    if !ipv4_ok(p) {
        return None;
    }
    Some(&p[..u16::from_be_bytes([p[2], p[3]]) as usize])
}

/// What a frame the kernel sent on the tun means.
#[derive(Debug, PartialEq, Eq)]
pub enum TunFrame<'a> {
    Ipv4(&'a [u8]),
    /// Answer this to the kernel.
    ArpReply(Vec<u8>),
    Other,
}

/// Classify a frame from the kernel. ARP requests for any address except
/// `own_ip` are answered with [`PEER_MAC`].
pub fn from_kernel<'a>(frame: &'a [u8], own_ip: [u8; 4]) -> TunFrame<'a> {
    if frame.len() < ETH_HEADER {
        return TunFrame::Other;
    }
    let et = u16::from_be_bytes([frame[12], frame[13]]);
    let body = &frame[ETH_HEADER..];
    match et {
        ETHERTYPE_IPV4 => ipv4_trim(body).map(TunFrame::Ipv4).unwrap_or(TunFrame::Other),
        ETHERTYPE_ARP if body.len() >= 28 => {
            // htype 1, ptype 0x0800, hlen 6, plen 4, op 1 (request)
            if body[0..2] != [0, 1] || body[2..4] != [8, 0] || body[4] != 6 || body[5] != 4 || body[6..8] != [0, 1] {
                return TunFrame::Other;
            }
            let sha = &body[8..14];
            let spa = &body[14..18];
            let tpa = &body[24..28];
            if tpa == own_ip {
                return TunFrame::Other;
            }
            let mut r = Vec::with_capacity(42);
            r.extend_from_slice(sha);
            r.extend_from_slice(&PEER_MAC);
            r.extend_from_slice(&ETHERTYPE_ARP.to_be_bytes());
            r.extend_from_slice(&[0, 1, 8, 0, 6, 4, 0, 2]);
            r.extend_from_slice(&PEER_MAC);
            r.extend_from_slice(tpa);
            r.extend_from_slice(sha);
            r.extend_from_slice(spa);
            TunFrame::ArpReply(r)
        }
        _ => TunFrame::Other,
    }
}

/// An IPv4 packet as a frame for the kernel's tun interface with MAC `tun_mac`.
pub fn to_kernel(ip: &[u8], tun_mac: [u8; 6]) -> Vec<u8> {
    let mut f = Vec::with_capacity(ETH_HEADER + ip.len());
    f.extend_from_slice(&tun_mac);
    f.extend_from_slice(&PEER_MAC);
    f.extend_from_slice(&ETHERTYPE_IPV4.to_be_bytes());
    f.extend_from_slice(ip);
    f
}

/// Is `ip` inside `net/prefix`?
pub fn in_subnet(ip: [u8; 4], net: [u8; 4], prefix: u8) -> bool {
    let m = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix.min(32) as u32) };
    u32::from_be_bytes(ip) & m == u32::from_be_bytes(net) & m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip_packet(src: [u8; 4], dst: [u8; 4], len: usize) -> Vec<u8> {
        let mut p = alloc::vec![0u8; len];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&(len as u16).to_be_bytes());
        p[9] = 17;
        p[12..16].copy_from_slice(&src);
        p[16..20].copy_from_slice(&dst);
        p
    }

    #[test]
    fn arp_is_answered_except_for_ourselves() {
        let own = [100, 65, 2, 10];
        let mut f = alloc::vec![0xffu8; 6];
        f.extend_from_slice(&[2, 0, 0, 0, 0, 1]);
        f.extend_from_slice(&ETHERTYPE_ARP.to_be_bytes());
        f.extend_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1]);
        f.extend_from_slice(&[2, 0, 0, 0, 0, 1]);
        f.extend_from_slice(&own);
        f.extend_from_slice(&[0; 6]);
        f.extend_from_slice(&[100, 65, 2, 1]);
        match from_kernel(&f, own) {
            TunFrame::ArpReply(r) => {
                assert_eq!(&r[0..6], &[2, 0, 0, 0, 0, 1]);
                assert_eq!(&r[22..28], &PEER_MAC);
                assert_eq!(&r[28..32], &[100, 65, 2, 1]);
                assert_eq!(&r[38..42], &own);
            }
            x => panic!("{:?}", x),
        }
        let n = f.len();
        f[n - 4..].copy_from_slice(&own);
        assert_eq!(from_kernel(&f, own), TunFrame::Other);
    }

    #[test]
    fn ipv4_frames_and_padding() {
        let p = ip_packet([1, 2, 3, 4], [5, 6, 7, 8], 28);
        let mut f = to_kernel(&p, [2, 0, 0, 0, 0, 9]);
        f.extend_from_slice(&[0; 18]); // Ethernet padding
        assert_eq!(from_kernel(&f, [0; 4]), TunFrame::Ipv4(&p));
        assert_eq!(ipv4_dst(&p), Some([5, 6, 7, 8]));
        assert_eq!(ipv4_src(&p[..10]), None);
        assert!(in_subnet([100, 65, 2, 77], [100, 65, 2, 0], 24));
        assert!(!in_subnet([100, 65, 3, 77], [100, 65, 2, 0], 24));
        assert!(in_subnet([9, 9, 9, 9], [0; 4], 0));
    }
}
