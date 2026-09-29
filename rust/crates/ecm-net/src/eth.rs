//! Ethernet frame parsing and construction, with 802.1Q VLAN tags.
//!
//! Pure helpers only; the stack owns all I/O (see [`crate::Stack::pop_tx`]).

use super::types::{MacAddr, MAX_FRAME_SIZE};

pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_8021Q: u16 = 0x8100;
/// Minimum ethernet frame length without FCS; shorter frames are zero-padded on TX.
pub const MIN_FRAME_SIZE: usize = 60;

/// 802.1Q VLAN tag (4 bytes on wire: 2-byte TPID 0x8100 + 2-byte TCI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VlanTag {
    pub pcp: u8,
    pub dei: bool,
    pub vid: u16,
}

impl VlanTag {
    pub fn new(vid: u16) -> Self {
        VlanTag { pcp: 0, dei: false, vid: vid & 0x0FFF }
    }

    pub fn to_tci(&self) -> u16 {
        ((self.pcp as u16 & 0x07) << 13) | (if self.dei { 1 << 12 } else { 0 }) | (self.vid & 0x0FFF)
    }

    pub fn from_tci(tci: u16) -> Self {
        VlanTag { pcp: ((tci >> 13) & 0x07) as u8, dei: (tci >> 12) & 1 != 0, vid: tci & 0x0FFF }
    }
}

/// Parsed ethernet frame header (14 bytes untagged, 18 bytes with 802.1Q tag).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EthHeader {
    pub dst: MacAddr,
    pub src: MacAddr,
    pub vlan_tag: Option<VlanTag>,
    pub ethertype: u16,
}

impl EthHeader {
    pub const SIZE_UNTAGGED: usize = 14;
    pub const SIZE_TAGGED: usize = 18;
    pub const SIZE: usize = 14;

    pub fn header_size(&self) -> usize {
        if self.vlan_tag.is_some() {
            Self::SIZE_TAGGED
        } else {
            Self::SIZE_UNTAGGED
        }
    }

    /// Parse the header; returns the header and the payload after it.
    /// Only one 802.1Q tag is decoded; a second tag shows up as `ethertype == ETHERTYPE_8021Q`.
    pub fn parse(data: &[u8]) -> Option<(Self, &[u8])> {
        let (hdr, rest) = data.split_at_checked(14)?;
        let dst = MacAddr::from_bytes(&hdr[0..6]);
        let src = MacAddr::from_bytes(&hdr[6..12]);
        let first_ethertype = u16::from_be_bytes([hdr[12], hdr[13]]);
        if first_ethertype == ETHERTYPE_8021Q {
            let (tag, rest) = rest.split_at_checked(4)?;
            let tci = u16::from_be_bytes([tag[0], tag[1]]);
            let ethertype = u16::from_be_bytes([tag[2], tag[3]]);
            Some((EthHeader { dst, src, vlan_tag: Some(VlanTag::from_tci(tci)), ethertype }, rest))
        } else {
            Some((EthHeader { dst, src, vlan_tag: None, ethertype: first_ethertype }, rest))
        }
    }

    /// Write the header into `buf`. Returns bytes written, or 0 if `buf` is too small.
    pub fn write(&self, buf: &mut [u8]) -> usize {
        let n = self.header_size();
        let Some(buf) = buf.get_mut(..n) else { return 0 };
        buf[0..6].copy_from_slice(&self.dst.0);
        buf[6..12].copy_from_slice(&self.src.0);
        if let Some(ref vlan) = self.vlan_tag {
            buf[12..14].copy_from_slice(&ETHERTYPE_8021Q.to_be_bytes());
            buf[14..16].copy_from_slice(&vlan.to_tci().to_be_bytes());
            buf[16..18].copy_from_slice(&self.ethertype.to_be_bytes());
        } else {
            buf[12..14].copy_from_slice(&self.ethertype.to_be_bytes());
        }
        n
    }
}

/// Build a complete frame (zero-padded to [`MIN_FRAME_SIZE`]).
/// Returns `None` if the frame would exceed [`MAX_FRAME_SIZE`].
pub fn build_frame(dst: MacAddr, src: MacAddr, vlan: Option<VlanTag>, ethertype: u16, payload: &[u8]) -> Option<Vec<u8>> {
    let hdr = EthHeader { dst, src, vlan_tag: vlan, ethertype };
    let hl = hdr.header_size();
    if hl + payload.len() > MAX_FRAME_SIZE {
        return None;
    }
    let mut f = vec![0u8; (hl + payload.len()).max(MIN_FRAME_SIZE)];
    hdr.write(&mut f);
    f[hl..hl + payload.len()].copy_from_slice(payload);
    Some(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: MacAddr = MacAddr([2, 0, 0, 0, 0, 1]);
    const B: MacAddr = MacAddr([2, 0, 0, 0, 0, 2]);

    #[test]
    fn roundtrip_untagged_and_tagged() {
        let f = build_frame(A, B, None, ETHERTYPE_ARP, &[1, 2, 3]).unwrap();
        assert_eq!(f.len(), MIN_FRAME_SIZE);
        let (h, p) = EthHeader::parse(&f).unwrap();
        assert_eq!((h.dst, h.src, h.vlan_tag, h.ethertype), (A, B, None, ETHERTYPE_ARP));
        assert_eq!(&p[..3], &[1, 2, 3]);

        let tag = VlanTag { pcp: 5, dei: true, vid: 100 };
        let f = build_frame(A, B, Some(tag), ETHERTYPE_IPV4, &[9; 100]).unwrap();
        let (h, p) = EthHeader::parse(&f).unwrap();
        assert_eq!(h.vlan_tag, Some(tag));
        assert_eq!(h.ethertype, ETHERTYPE_IPV4);
        assert_eq!(p.len(), 100);
        assert_eq!(VlanTag::from_tci(tag.to_tci()), tag);
    }

    #[test]
    fn truncated_and_garbage() {
        for n in 0..14 {
            assert!(EthHeader::parse(&vec![0xAB; n]).is_none());
        }
        let mut f = vec![0u8; 17];
        f[12] = 0x81;
        f[13] = 0x00;
        assert!(EthHeader::parse(&f).is_none());
        f.push(0);
        assert!(EthHeader::parse(&f).is_some());
        assert!(build_frame(A, B, None, 0, &[0; 1505]).is_none());
        assert!(build_frame(A, B, Some(VlanTag::new(1)), 0, &[0; 1500]).is_some());
        let h = EthHeader { dst: A, src: B, vlan_tag: Some(VlanTag::new(3)), ethertype: 1 };
        assert_eq!(h.write(&mut [0u8; 17]), 0);
    }
}
