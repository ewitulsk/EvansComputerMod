//! Spanning-tree BPDU codec (IEEE 802.1D-2004 clause 9).
//!
//! Supports Configuration BPDUs (protocol version 0, type 0x00), TCN BPDUs
//! (type 0x80) and RST BPDUs (version 2, type 0x02). BPDUs travel in
//! 802.3/LLC frames (DSAP = SSAP = 0x42, control 0x03) to 01:80:c2:00:00:00.

use crate::frame::{self, be16, be32, STP_GROUP};

pub const FLAG_TC: u8 = 0x01;
pub const FLAG_PROPOSAL: u8 = 0x02;
pub const FLAG_LEARNING: u8 = 0x10;
pub const FLAG_FORWARDING: u8 = 0x20;
pub const FLAG_AGREEMENT: u8 = 0x40;
pub const FLAG_TC_ACK: u8 = 0x80;

const LLC: [u8; 3] = [0x42, 0x42, 0x03];
const CONFIG_LEN: usize = 35;
const RST_LEN: usize = 36;
const TCN_LEN: usize = 4;

/// Port role as encoded in the RST BPDU flags (bits 2-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BpduRole {
    Unknown,
    AlternateOrBackup,
    Root,
    Designated,
}

impl BpduRole {
    fn bits(self) -> u8 {
        match self {
            BpduRole::Unknown => 0,
            BpduRole::AlternateOrBackup => 1,
            BpduRole::Root => 2,
            BpduRole::Designated => 3,
        }
    }
    fn from_bits(b: u8) -> Self {
        match b & 3 {
            1 => BpduRole::AlternateOrBackup,
            2 => BpduRole::Root,
            3 => BpduRole::Designated,
            _ => BpduRole::Unknown,
        }
    }
}

/// Body shared by Configuration and RST BPDUs. Times are in 1/256 s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BpduBody {
    pub flags: u8,
    pub root_id: u64,
    pub root_path_cost: u32,
    pub bridge_id: u64,
    pub port_id: u16,
    pub message_age: u16,
    pub max_age: u16,
    pub hello_time: u16,
    pub forward_delay: u16,
}

impl BpduBody {
    /// Role bits of an RST BPDU.
    pub fn role(&self) -> BpduRole {
        BpduRole::from_bits(self.flags >> 2)
    }
    pub fn with_role(mut self, r: BpduRole) -> Self {
        self.flags = (self.flags & !0x0c) | (r.bits() << 2);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bpdu {
    /// Protocol version 0 configuration BPDU.
    Config(BpduBody),
    /// Topology change notification.
    Tcn,
    /// Rapid spanning tree BPDU (version >= 2).
    Rst(BpduBody),
}

/// Decode the BPDU octets that follow the LLC header.
pub fn decode(b: &[u8]) -> Option<Bpdu> {
    if be16(b, 0)? != 0x0000 {
        return None;
    }
    let version = *b.get(2)?;
    let ty = *b.get(3)?;
    match ty {
        0x80 if b.len() >= TCN_LEN => Some(Bpdu::Tcn),
        0x00 if b.len() >= CONFIG_LEN => Some(Bpdu::Config(body(b)?)),
        // Accept RST BPDUs from future versions too (802.1D 9.3.4 c).
        0x02 if version >= 2 && b.len() >= RST_LEN => Some(Bpdu::Rst(body(b)?)),
        _ => None,
    }
}

fn body(b: &[u8]) -> Option<BpduBody> {
    let id = |off: usize| -> Option<u64> {
        let s = b.get(off..off + 8)?;
        let a: [u8; 8] = s.try_into().ok()?;
        Some(u64::from_be_bytes(a))
    };
    Some(BpduBody {
        flags: *b.get(4)?,
        root_id: id(5)?,
        root_path_cost: be32(b, 13)?,
        bridge_id: id(17)?,
        port_id: be16(b, 25)?,
        message_age: be16(b, 27)?,
        max_age: be16(b, 29)?,
        hello_time: be16(b, 31)?,
        forward_delay: be16(b, 33)?,
    })
}

/// Encode a BPDU (the octets that follow the LLC header).
pub fn encode(bpdu: &Bpdu) -> Vec<u8> {
    let mut out = Vec::with_capacity(RST_LEN);
    match bpdu {
        Bpdu::Tcn => {
            out.extend_from_slice(&[0, 0, 0, 0x80]);
        }
        Bpdu::Config(b) | Bpdu::Rst(b) => {
            let (version, ty) = if matches!(bpdu, Bpdu::Rst(_)) { (2u8, 0x02u8) } else { (0, 0) };
            out.extend_from_slice(&[0, 0, version, ty]);
            let flags = if matches!(bpdu, Bpdu::Config(_)) { b.flags & (FLAG_TC | FLAG_TC_ACK) } else { b.flags };
            out.push(flags);
            out.extend_from_slice(&b.root_id.to_be_bytes());
            out.extend_from_slice(&b.root_path_cost.to_be_bytes());
            out.extend_from_slice(&b.bridge_id.to_be_bytes());
            out.extend_from_slice(&b.port_id.to_be_bytes());
            out.extend_from_slice(&b.message_age.to_be_bytes());
            out.extend_from_slice(&b.max_age.to_be_bytes());
            out.extend_from_slice(&b.hello_time.to_be_bytes());
            out.extend_from_slice(&b.forward_delay.to_be_bytes());
            if matches!(bpdu, Bpdu::Rst(_)) {
                out.push(0); // Version 1 Length
            }
        }
    }
    out
}

/// Build a complete (padded) BPDU frame from `src`.
pub fn build_frame(src: [u8; 6], bpdu: &Bpdu) -> Vec<u8> {
    let body = encode(bpdu);
    let mut f = frame::eth_header(STP_GROUP, src, (LLC.len() + body.len()) as u16);
    f.extend_from_slice(&LLC);
    f.extend_from_slice(&body);
    frame::pad_min(f)
}

/// Parse a frame as a BPDU. Requires the STP group address, an 802.3
/// length field and the STP LLC header. Tagged BPDUs are accepted.
pub fn parse_frame(f: &[u8]) -> Option<Bpdu> {
    let e = frame::parse_eth(f)?;
    if e.dst != STP_GROUP || e.ethertype > 1500 {
        return None;
    }
    let len = e.ethertype as usize;
    let llc = f.get(e.payload_off..e.payload_off.checked_add(len)?)?;
    if llc.get(..3)? != LLC {
        return None;
    }
    decode(llc.get(3..)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BpduBody {
        BpduBody {
            flags: FLAG_TC | FLAG_LEARNING | FLAG_FORWARDING,
            root_id: 0x8000_0200_0000_0001,
            root_path_cost: 20000,
            bridge_id: 0x8000_0200_0000_0002,
            port_id: 0x8003,
            message_age: 256,
            max_age: 20 * 256,
            hello_time: 2 * 256,
            forward_delay: 15 * 256,
        }
        .with_role(BpduRole::Designated)
    }

    #[test]
    fn rst_round_trip() {
        let b = Bpdu::Rst(sample());
        let enc = encode(&b);
        assert_eq!(enc.len(), 36);
        assert_eq!(decode(&enc), Some(b));
        let f = build_frame([2, 0, 0, 0, 0, 9], &b);
        assert_eq!(f.len(), 60);
        assert_eq!(parse_frame(&f), Some(b));
        if let Some(Bpdu::Rst(x)) = parse_frame(&f) {
            assert_eq!(x.role(), BpduRole::Designated);
        }
    }

    #[test]
    fn config_and_tcn_round_trip() {
        let mut s = sample();
        s.flags = FLAG_TC | FLAG_TC_ACK;
        let c = Bpdu::Config(s);
        assert_eq!(encode(&c).len(), 35);
        assert_eq!(parse_frame(&build_frame([2; 6], &c)), Some(c));
        assert_eq!(parse_frame(&build_frame([2; 6], &Bpdu::Tcn)), Some(Bpdu::Tcn));
    }

    #[test]
    fn config_encoding_drops_rstp_only_flags() {
        let c = Bpdu::Config(sample());
        match decode(&encode(&c)) {
            Some(Bpdu::Config(b)) => assert_eq!(b.flags, FLAG_TC),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn truncated_and_garbage_rejected() {
        let enc = encode(&Bpdu::Rst(sample()));
        for n in 0..enc.len() {
            assert_eq!(decode(&enc[..n]), None, "len {}", n);
        }
        let f = build_frame([2; 6], &Bpdu::Rst(sample()));
        for n in 0..f.len() {
            let _ = parse_frame(&f[..n]);
        }
        // Wrong protocol id / wrong LLC / wrong dst.
        let mut bad = enc.clone();
        bad[0] = 1;
        assert_eq!(decode(&bad), None);
        let mut bad = f.clone();
        bad[14] = 0xaa;
        assert_eq!(parse_frame(&bad), None);
        let mut bad = f.clone();
        bad[5] = 0x01;
        assert_eq!(parse_frame(&bad), None);
        // Version 0 with type 2 is not an RST BPDU.
        let mut bad = enc.clone();
        bad[2] = 0;
        assert_eq!(decode(&bad), None);
        // Length field larger than the frame.
        let mut bad = f.clone();
        bad[12] = 0x05;
        bad[13] = 0xdc;
        assert_eq!(parse_frame(&bad), None);
        // Pure garbage.
        let mut x: u32 = 1;
        for _ in 0..2000 {
            let n = (x % 80) as usize;
            let v: Vec<u8> = (0..n).map(|i| { x = x.wrapping_mul(1_103_515_245).wrapping_add(12345); (x >> 16) as u8 ^ i as u8 }).collect();
            let _ = decode(&v);
            let _ = parse_frame(&v);
        }
    }

    #[test]
    fn future_version_rst_accepted() {
        let mut enc = encode(&Bpdu::Rst(sample()));
        enc[2] = 3;
        assert!(matches!(decode(&enc), Some(Bpdu::Rst(_))));
    }
}
