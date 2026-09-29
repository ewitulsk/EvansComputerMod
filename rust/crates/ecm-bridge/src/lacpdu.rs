//! LACPDU codec (IEEE 802.1AX-2014 clause 6.4.2, version 1 layout).
//!
//! On the wire: 14-byte Ethernet header (to 01:80:c2:00:00:02, EtherType
//! 0x8809) followed by exactly 110 octets: subtype, version, Actor TLV (20),
//! Partner TLV (20), Collector TLV (16), Terminator TLV (2) and 50 reserved
//! octets. The frame is 124 octets, i.e. 128 with the FCS the host adds.

use crate::frame::{self, be16, mac_at, ETHERTYPE_SLOW, SLOW_PROTOCOLS_GROUP};

pub const SUBTYPE_LACP: u8 = 0x01;
pub const SUBTYPE_MARKER: u8 = 0x02;
pub const LACPDU_PAYLOAD_LEN: usize = 110;
/// Octets up to and including the Terminator TLV; the reserved tail is not
/// required on receive (tolerant of peers that omit it).
const MIN_RX_LEN: usize = 60;

pub const ST_ACTIVITY: u8 = 0x01;
pub const ST_TIMEOUT: u8 = 0x02;
pub const ST_AGGREGATION: u8 = 0x04;
pub const ST_SYNC: u8 = 0x08;
pub const ST_COLLECTING: u8 = 0x10;
pub const ST_DISTRIBUTING: u8 = 0x20;
pub const ST_DEFAULTED: u8 = 0x40;
pub const ST_EXPIRED: u8 = 0x80;

/// Actor or partner information block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LacpInfo {
    pub system_priority: u16,
    pub system: [u8; 6],
    pub key: u16,
    pub port_priority: u16,
    pub port: u16,
    pub state: u8,
}

impl LacpInfo {
    /// Everything except the state byte: identifies the (system, key, port).
    pub fn same_identity(&self, o: &LacpInfo) -> bool {
        self.system_priority == o.system_priority
            && self.system == o.system
            && self.key == o.key
            && self.port_priority == o.port_priority
            && self.port == o.port
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lacpdu {
    pub actor: LacpInfo,
    pub partner: LacpInfo,
    pub collector_max_delay: u16,
}

fn put_info(out: &mut Vec<u8>, ty: u8, i: &LacpInfo) {
    out.push(ty);
    out.push(20);
    out.extend_from_slice(&i.system_priority.to_be_bytes());
    out.extend_from_slice(&i.system);
    out.extend_from_slice(&i.key.to_be_bytes());
    out.extend_from_slice(&i.port_priority.to_be_bytes());
    out.extend_from_slice(&i.port.to_be_bytes());
    out.push(i.state);
    out.extend_from_slice(&[0, 0, 0]);
}

fn get_info(b: &[u8], off: usize, ty: u8) -> Option<LacpInfo> {
    if *b.get(off)? != ty || *b.get(off + 1)? != 20 {
        return None;
    }
    Some(LacpInfo {
        system_priority: be16(b, off + 2)?,
        system: mac_at(b, off + 4)?,
        key: be16(b, off + 10)?,
        port_priority: be16(b, off + 12)?,
        port: be16(b, off + 14)?,
        state: *b.get(off + 16)?,
    })
}

/// Encode the 110-octet LACPDU payload (after the EtherType).
pub fn encode(p: &Lacpdu) -> Vec<u8> {
    let mut out = Vec::with_capacity(LACPDU_PAYLOAD_LEN);
    out.push(SUBTYPE_LACP);
    out.push(0x01); // version
    put_info(&mut out, 0x01, &p.actor);
    put_info(&mut out, 0x02, &p.partner);
    out.push(0x03);
    out.push(16);
    out.extend_from_slice(&p.collector_max_delay.to_be_bytes());
    out.extend_from_slice(&[0u8; 12]);
    out.push(0x00);
    out.push(0x00);
    out.resize(LACPDU_PAYLOAD_LEN, 0);
    out
}

/// Decode an LACPDU payload (starting at the subtype octet).
pub fn decode(b: &[u8]) -> Option<Lacpdu> {
    if b.len() < MIN_RX_LEN || *b.first()? != SUBTYPE_LACP || *b.get(1)? == 0 {
        return None;
    }
    let actor = get_info(b, 2, 0x01)?;
    let partner = get_info(b, 22, 0x02)?;
    if *b.get(42)? != 0x03 || *b.get(43)? != 16 {
        return None;
    }
    let collector_max_delay = be16(b, 44)?;
    if *b.get(58)? != 0x00 || *b.get(59)? != 0x00 {
        return None;
    }
    Some(Lacpdu { actor, partner, collector_max_delay })
}

/// Build the complete 124-octet frame.
pub fn build_frame(src: [u8; 6], p: &Lacpdu) -> Vec<u8> {
    let mut f = frame::eth_header(SLOW_PROTOCOLS_GROUP, src, ETHERTYPE_SLOW);
    f.extend_from_slice(&encode(p));
    f
}

/// Parse a slow-protocols frame as an LACPDU (untagged or tagged).
pub fn parse_frame(f: &[u8]) -> Option<Lacpdu> {
    let e = frame::parse_eth(f)?;
    if e.ethertype != ETHERTYPE_SLOW || e.dst != SLOW_PROTOCOLS_GROUP {
        return None;
    }
    decode(f.get(e.payload_off..)?)
}

/// Render a state byte as the conventional letter string (e.g. "ASAGSCDx").
pub fn state_string(s: u8) -> String {
    let bit = |m: u8, on: char| if s & m != 0 { on } else { '-' };
    [
        bit(ST_ACTIVITY, 'A'),
        bit(ST_TIMEOUT, 'F'),
        bit(ST_AGGREGATION, 'G'),
        bit(ST_SYNC, 'S'),
        bit(ST_COLLECTING, 'C'),
        bit(ST_DISTRIBUTING, 'D'),
        bit(ST_DEFAULTED, 'd'),
        bit(ST_EXPIRED, 'e'),
    ]
    .iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Lacpdu {
        Lacpdu {
            actor: LacpInfo {
                system_priority: 32768,
                system: [2, 0, 0, 0, 0, 1],
                key: 7,
                port_priority: 32768,
                port: 3,
                state: ST_ACTIVITY | ST_TIMEOUT | ST_AGGREGATION | ST_SYNC,
            },
            partner: LacpInfo {
                system_priority: 100,
                system: [2, 0, 0, 0, 0, 2],
                key: 9,
                port_priority: 1,
                port: 4,
                state: ST_AGGREGATION | ST_COLLECTING | ST_DISTRIBUTING,
            },
            collector_max_delay: 5,
        }
    }

    #[test]
    fn exact_sizes() {
        assert_eq!(encode(&sample()).len(), 110);
        let f = build_frame([2, 0, 0, 0, 0, 1], &sample());
        assert_eq!(f.len(), 124);
        assert!(f.len() >= crate::frame::MIN_FRAME_LEN);
    }

    #[test]
    fn round_trip() {
        let p = sample();
        assert_eq!(decode(&encode(&p)), Some(p));
        let f = build_frame([2, 0, 0, 0, 0, 1], &p);
        assert_eq!(parse_frame(&f), Some(p));
        let t = crate::frame::tag(&f, 5, 0).unwrap();
        assert_eq!(parse_frame(&t), Some(p));
    }

    #[test]
    fn short_reserved_tail_accepted() {
        let enc = encode(&sample());
        assert_eq!(decode(&enc[..60]), Some(sample()));
        for n in 0..60 {
            assert_eq!(decode(&enc[..n]), None, "len {}", n);
        }
    }

    #[test]
    fn garbage_rejected() {
        let enc = encode(&sample());
        for i in [0usize, 2, 3, 22, 23, 42, 43, 58] {
            let mut b = enc.clone();
            b[i] ^= 0x5a;
            assert_eq!(decode(&b), None, "byte {}", i);
        }
        let f = build_frame([2; 6], &sample());
        let mut wrong_dst = f.clone();
        wrong_dst[5] = 0x0e;
        assert_eq!(parse_frame(&wrong_dst), None);
        for n in 0..f.len() {
            let _ = parse_frame(&f[..n]);
        }
        let mut x: u32 = 7;
        for _ in 0..2000 {
            let n = (x % 130) as usize;
            let v: Vec<u8> = (0..n).map(|_| { x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223); (x >> 24) as u8 }).collect();
            let _ = decode(&v);
            let _ = parse_frame(&v);
        }
    }

    #[test]
    fn state_strings() {
        assert_eq!(state_string(0), "--------");
        assert_eq!(state_string(0x3d), "A-GSCD--");
    }
}
