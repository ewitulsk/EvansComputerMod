//! Minimal Ethernet / 802.1Q parsing and rewriting, plus the header peeking
//! used for LAG flow hashing (ARP, IPv4, TCP/UDP).
//!
//! Every function here is total: malformed input yields `None` (or a
//! best-effort fallback for hashing), never a panic.

/// Ethernet header length without a tag.
pub const ETH_HDR_LEN: usize = 14;
/// 802.1Q C-tag TPID.
pub const TPID_8021Q: u16 = 0x8100;
pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_SLOW: u16 = 0x8809;
pub const ETHERTYPE_LLDP: u16 = 0x88cc;
/// Minimum frame length without FCS; frames we originate are padded to it.
pub const MIN_FRAME_LEN: usize = 60;
/// Largest frame accepted or produced (1500 payload + 14 header + 4 tag).
pub const MAX_FRAME_LEN: usize = 1518;

/// Destination of BPDUs (Nearest Customer Bridge / STP group address).
pub const STP_GROUP: [u8; 6] = [0x01, 0x80, 0xc2, 0x00, 0x00, 0x00];
/// Slow-protocols multicast address (LACP, Marker).
pub const SLOW_PROTOCOLS_GROUP: [u8; 6] = [0x01, 0x80, 0xc2, 0x00, 0x00, 0x02];
/// LLDP nearest-bridge group address.
pub const LLDP_NEAREST_BRIDGE: [u8; 6] = [0x01, 0x80, 0xc2, 0x00, 0x00, 0x0e];

/// Read a big-endian u16 at `off`.
pub fn be16(b: &[u8], off: usize) -> Option<u16> {
    let s = b.get(off..off.checked_add(2)?)?;
    Some(u16::from_be_bytes([*s.first()?, *s.get(1)?]))
}

/// Read a big-endian u32 at `off`.
pub fn be32(b: &[u8], off: usize) -> Option<u32> {
    let s = b.get(off..off.checked_add(4)?)?;
    let a: [u8; 4] = s.try_into().ok()?;
    Some(u32::from_be_bytes(a))
}

/// Copy 6 bytes at `off`.
pub fn mac_at(b: &[u8], off: usize) -> Option<[u8; 6]> {
    let s = b.get(off..off.checked_add(6)?)?;
    s.try_into().ok()
}

/// Parsed view of an Ethernet header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EthInfo {
    pub dst: [u8; 6],
    pub src: [u8; 6],
    /// 802.1Q TCI when the frame carries a C-tag.
    pub tci: Option<u16>,
    /// EtherType after the (optional) tag; values <= 1500 are an 802.3 length.
    pub ethertype: u16,
    /// Offset of the payload (after the optional tag).
    pub payload_off: usize,
}

impl EthInfo {
    /// VID of the tag, treating a priority tag (VID 0) as untagged.
    pub fn vid(&self) -> Option<u16> {
        match self.tci {
            Some(t) if t & 0x0fff != 0 => Some(t & 0x0fff),
            _ => None,
        }
    }
    /// PCP bits from the tag (0 when untagged).
    pub fn pcp(&self) -> u8 {
        self.tci.map(|t| (t >> 13) as u8).unwrap_or(0)
    }
}

/// Parse the Ethernet (+ optional single 802.1Q tag) header.
pub fn parse_eth(f: &[u8]) -> Option<EthInfo> {
    if f.len() < ETH_HDR_LEN || f.len() > MAX_FRAME_LEN + 4 {
        return None;
    }
    let dst = mac_at(f, 0)?;
    let src = mac_at(f, 6)?;
    let et = be16(f, 12)?;
    if et == TPID_8021Q {
        let tci = be16(f, 14)?;
        let et2 = be16(f, 16)?;
        Some(EthInfo { dst, src, tci: Some(tci), ethertype: et2, payload_off: 18 })
    } else {
        Some(EthInfo { dst, src, tci: None, ethertype: et, payload_off: 14 })
    }
}

pub fn is_multicast(mac: &[u8; 6]) -> bool {
    mac[0] & 0x01 != 0
}

/// IEEE 802.1D reserved group addresses 01:80:c2:00:00:00..0f. Never forwarded.
pub fn is_reserved_group(mac: &[u8; 6]) -> bool {
    mac[..5] == [0x01, 0x80, 0xc2, 0x00, 0x00] && mac[5] <= 0x0f
}

/// Return the frame without its 802.1Q tag (a copy either way).
pub fn untag(f: &[u8]) -> Vec<u8> {
    match (be16(f, 12), f.len() >= 18) {
        (Some(TPID_8021Q), true) => {
            let mut out = Vec::with_capacity(f.len() - 4);
            out.extend_from_slice(f.get(..12).unwrap_or(&[]));
            out.extend_from_slice(f.get(16..).unwrap_or(&[]));
            out
        }
        _ => f.to_vec(),
    }
}

/// Return the frame carrying exactly one 802.1Q tag with `vid` and `pcp`
/// (an existing tag is rewritten). Returns `None` for runt frames or if
/// the result would exceed [`MAX_FRAME_LEN`].
pub fn tag(f: &[u8], vid: u16, pcp: u8) -> Option<Vec<u8>> {
    let base = untag(f);
    if base.len() < ETH_HDR_LEN || base.len() + 4 > MAX_FRAME_LEN + 4 {
        return None;
    }
    let tci = ((pcp as u16 & 0x7) << 13) | (vid & 0x0fff);
    let mut out = Vec::with_capacity(base.len() + 4);
    out.extend_from_slice(base.get(..12)?);
    out.extend_from_slice(&TPID_8021Q.to_be_bytes());
    out.extend_from_slice(&tci.to_be_bytes());
    out.extend_from_slice(base.get(12..)?);
    Some(out)
}

/// Pad a frame we originate to the Ethernet minimum.
pub fn pad_min(mut f: Vec<u8>) -> Vec<u8> {
    if f.len() < MIN_FRAME_LEN {
        f.resize(MIN_FRAME_LEN, 0);
    }
    f
}

/// Build an untagged frame header.
pub fn eth_header(dst: [u8; 6], src: [u8; 6], ethertype_or_len: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    out.extend_from_slice(&dst);
    out.extend_from_slice(&src);
    out.extend_from_slice(&ethertype_or_len.to_be_bytes());
    out
}

// ---------------------------------------------------------------------
// Flow hashing for LAG member selection
// ---------------------------------------------------------------------

/// LAG egress hashing policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HashMode {
    /// Source + destination MAC.
    L2,
    /// Source + destination IPv4 (ARP sender/target IP for ARP); falls back to L2.
    L3,
    /// L3 plus TCP/UDP ports for unfragmented IPv4; falls back to L3.
    L4,
}

impl HashMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            HashMode::L2 => "l2-src-dst",
            HashMode::L3 => "l3-src-dst",
            HashMode::L4 => "l4-src-dst",
        }
    }
}

fn fnv(h: &mut u32, bytes: &[u8]) {
    for &b in bytes {
        *h ^= b as u32;
        *h = h.wrapping_mul(0x0100_0193);
    }
}

fn finish(mut h: u32) -> u32 {
    // Final avalanche so that small input differences spread over the low bits.
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h
}

/// IPv4 addresses (and, for `want_ports`, the L4 ports) of a frame.
fn l3_fields(f: &[u8], e: &EthInfo, want_ports: bool) -> Option<([u8; 8], Option<[u8; 4]>)> {
    let off = e.payload_off;
    match e.ethertype {
        ETHERTYPE_IPV4 => {
            let vihl = *f.get(off)?;
            if vihl >> 4 != 4 {
                return None;
            }
            let ihl = ((vihl & 0x0f) as usize) * 4;
            if ihl < 20 {
                return None;
            }
            let total = be16(f, off + 2)? as usize;
            if total < ihl || off + ihl > f.len() {
                return None;
            }
            let mut ips = [0u8; 8];
            ips.copy_from_slice(f.get(off + 12..off + 20)?);
            let mut ports = None;
            if want_ports {
                let frag = be16(f, off + 6)?;
                let unfragmented = frag & 0x3fff == 0; // MF clear, offset 0
                let proto = *f.get(off + 9)?;
                if unfragmented && (proto == 6 || proto == 17) {
                    let p = f.get(off + ihl..off + ihl + 4);
                    if let Some(p) = p {
                        let mut a = [0u8; 4];
                        a.copy_from_slice(p);
                        ports = Some(a);
                    }
                }
            }
            Some((ips, ports))
        }
        ETHERTYPE_ARP => {
            // htype(2) ptype(2) hlen(1) plen(1) op(2) sha(6) spa(4) tha(6) tpa(4)
            if be16(f, off + 2)? != ETHERTYPE_IPV4 || *f.get(off + 4)? != 6 || *f.get(off + 5)? != 4 {
                return None;
            }
            let mut ips = [0u8; 8];
            ips[..4].copy_from_slice(f.get(off + 14..off + 18)?);
            ips[4..].copy_from_slice(f.get(off + 24..off + 28)?);
            Some((ips, None))
        }
        _ => None,
    }
}

/// Hash a frame for LAG member selection. Total: any input hashes.
pub fn flow_hash(f: &[u8], mode: HashMode) -> u32 {
    let e = match parse_eth(f) {
        Some(e) => e,
        None => return 0,
    };
    let mut h: u32 = 0x811c_9dc5;
    let l2 = |h: &mut u32| {
        fnv(h, &e.dst);
        fnv(h, &e.src);
    };
    match mode {
        HashMode::L2 => l2(&mut h),
        HashMode::L3 | HashMode::L4 => match l3_fields(f, &e, mode == HashMode::L4) {
            Some((ips, ports)) => {
                fnv(&mut h, &ips);
                if let Some(p) = ports {
                    fnv(&mut h, &p);
                }
            }
            None => l2(&mut h),
        },
    }
    finish(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(et: u16, payload: &[u8]) -> Vec<u8> {
        let mut f = eth_header([2, 0, 0, 0, 0, 2], [2, 0, 0, 0, 0, 1], et);
        f.extend_from_slice(payload);
        f
    }

    #[test]
    fn parse_untagged_and_tagged() {
        let f = frame(0x0800, &[0u8; 46]);
        let e = parse_eth(&f).unwrap();
        assert_eq!(e.tci, None);
        assert_eq!(e.ethertype, 0x0800);
        assert_eq!(e.payload_off, 14);
        let t = tag(&f, 10, 5).unwrap();
        assert_eq!(t.len(), f.len() + 4);
        let e = parse_eth(&t).unwrap();
        assert_eq!(e.vid(), Some(10));
        assert_eq!(e.pcp(), 5);
        assert_eq!(e.ethertype, 0x0800);
        assert_eq!(untag(&t), f);
        // Retag rewrites rather than stacking.
        let t2 = tag(&t, 20, 0).unwrap();
        assert_eq!(t2.len(), t.len());
        assert_eq!(parse_eth(&t2).unwrap().vid(), Some(20));
    }

    #[test]
    fn priority_tag_is_untagged() {
        let f = frame(0x0800, &[0u8; 46]);
        let t = tag(&f, 0, 3).unwrap();
        assert_eq!(parse_eth(&t).unwrap().vid(), None);
    }

    #[test]
    fn truncated_and_garbage() {
        for n in 0..20 {
            let f = vec![0x81u8; n];
            let _ = parse_eth(&f);
            let _ = untag(&f);
            let _ = tag(&f, 1, 0);
            let _ = flow_hash(&f, HashMode::L4);
        }
        assert!(parse_eth(&[0u8; 13]).is_none());
        // Tagged header truncated inside the tag.
        let mut f = vec![0u8; 16];
        f[12] = 0x81;
        assert!(parse_eth(&f).is_none());
        assert!(parse_eth(&vec![0u8; 3000]).is_none());
    }

    #[test]
    fn reserved_groups() {
        assert!(is_reserved_group(&STP_GROUP));
        assert!(is_reserved_group(&LLDP_NEAREST_BRIDGE));
        assert!(is_reserved_group(&[0x01, 0x80, 0xc2, 0, 0, 0x0f]));
        assert!(!is_reserved_group(&[0x01, 0x80, 0xc2, 0, 0, 0x10]));
        assert!(!is_reserved_group(&[0xff; 6]));
    }

    fn ipv4(src: [u8; 4], dst: [u8; 4], proto: u8, sport: u16, dport: u16, frag: u16) -> Vec<u8> {
        let mut ip = vec![0x45, 0, 0, 40, 0, 0];
        ip.extend_from_slice(&frag.to_be_bytes());
        ip.extend_from_slice(&[64, proto, 0, 0]);
        ip.extend_from_slice(&src);
        ip.extend_from_slice(&dst);
        ip.extend_from_slice(&sport.to_be_bytes());
        ip.extend_from_slice(&dport.to_be_bytes());
        ip.extend_from_slice(&[0u8; 16]);
        frame(ETHERTYPE_IPV4, &ip)
    }

    #[test]
    fn hash_modes_use_the_right_fields() {
        let a = ipv4([10, 0, 0, 1], [10, 0, 0, 2], 17, 1000, 53, 0);
        let b = ipv4([10, 0, 0, 1], [10, 0, 0, 2], 17, 1001, 53, 0);
        let c = ipv4([10, 0, 0, 1], [10, 0, 0, 3], 17, 1000, 53, 0);
        // Same MACs everywhere: L2 identical.
        assert_eq!(flow_hash(&a, HashMode::L2), flow_hash(&c, HashMode::L2));
        // L3 ignores ports, sees IPs.
        assert_eq!(flow_hash(&a, HashMode::L3), flow_hash(&b, HashMode::L3));
        assert_ne!(flow_hash(&a, HashMode::L3), flow_hash(&c, HashMode::L3));
        // L4 sees ports.
        assert_ne!(flow_hash(&a, HashMode::L4), flow_hash(&b, HashMode::L4));
        // Fragments fall back to L3 fields under L4.
        let fa = ipv4([10, 0, 0, 1], [10, 0, 0, 2], 17, 1000, 53, 0x2000);
        let fb = ipv4([10, 0, 0, 1], [10, 0, 0, 2], 17, 1001, 53, 0x2000);
        assert_eq!(flow_hash(&fa, HashMode::L4), flow_hash(&fb, HashMode::L4));
        // Tagged frames hash on the inner IP header.
        let ta = tag(&a, 10, 0).unwrap();
        assert_eq!(flow_hash(&a, HashMode::L4), flow_hash(&ta, HashMode::L4));
    }

    #[test]
    fn hash_arp_uses_ips() {
        let arp = |spa: [u8; 4], tpa: [u8; 4]| {
            let mut p = vec![0, 1, 8, 0, 6, 4, 0, 1];
            p.extend_from_slice(&[2, 0, 0, 0, 0, 1]);
            p.extend_from_slice(&spa);
            p.extend_from_slice(&[0; 6]);
            p.extend_from_slice(&tpa);
            p.extend_from_slice(&[0; 18]);
            frame(ETHERTYPE_ARP, &p)
        };
        let a = arp([10, 0, 0, 1], [10, 0, 0, 2]);
        let b = arp([10, 0, 0, 1], [10, 0, 0, 3]);
        assert_ne!(flow_hash(&a, HashMode::L3), flow_hash(&b, HashMode::L3));
        assert_eq!(flow_hash(&a, HashMode::L2), flow_hash(&b, HashMode::L2));
    }
}
