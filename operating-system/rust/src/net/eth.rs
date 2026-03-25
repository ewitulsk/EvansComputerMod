//! Ethernet frame parsing, construction, and host function wrappers.
//! Supports 802.1Q VLAN tagging.

use super::types::{MacAddr, MAX_FRAME_SIZE};

pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_8021Q: u16 = 0x8100;

/// 802.1Q VLAN tag (4 bytes on wire: 2-byte TPID 0x8100 + 2-byte TCI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VlanTag {
    pub pcp: u8,   // Priority Code Point (0-7)
    pub dei: bool,  // Drop Eligible Indicator
    pub vid: u16,   // VLAN Identifier (0-4094)
}

impl VlanTag {
    /// Create a VLAN tag with just a VID (PCP=0, DEI=false).
    pub fn new(vid: u16) -> Self {
        VlanTag { pcp: 0, dei: false, vid: vid & 0x0FFF }
    }

    /// Encode into 16-bit TCI field.
    pub fn to_tci(&self) -> u16 {
        ((self.pcp as u16 & 0x07) << 13)
            | (if self.dei { 1 << 12 } else { 0 })
            | (self.vid & 0x0FFF)
    }

    /// Parse from 2-byte TCI field.
    pub fn from_tci(tci: u16) -> Self {
        VlanTag {
            pcp: ((tci >> 13) & 0x07) as u8,
            dei: (tci >> 12) & 1 != 0,
            vid: tci & 0x0FFF,
        }
    }
}

/// Parsed ethernet frame header (14 bytes untagged, 18 bytes with 802.1Q tag).
pub struct EthHeader {
    pub dst: MacAddr,
    pub src: MacAddr,
    pub vlan_tag: Option<VlanTag>,
    pub ethertype: u16, // Always the "real" ethertype (IPv4, ARP, etc.)
}

impl EthHeader {
    pub const SIZE_UNTAGGED: usize = 14;
    pub const SIZE_TAGGED: usize = 18;
    // Keep SIZE for backward compat (minimum header size)
    pub const SIZE: usize = 14;

    /// Return the actual header size (14 or 18).
    pub fn header_size(&self) -> usize {
        if self.vlan_tag.is_some() { Self::SIZE_TAGGED } else { Self::SIZE_UNTAGGED }
    }

    /// Parse an ethernet header from raw bytes.
    /// Handles both untagged and 802.1Q tagged frames.
    /// Returns the header and a slice of the payload.
    pub fn parse(data: &[u8]) -> Option<(Self, &[u8])> {
        if data.len() < 14 {
            return None;
        }

        let dst = MacAddr::from_bytes(&data[0..6]);
        let src = MacAddr::from_bytes(&data[6..12]);
        let first_ethertype = u16::from_be_bytes([data[12], data[13]]);

        if first_ethertype == ETHERTYPE_8021Q {
            // 802.1Q tagged frame: need at least 18 bytes
            if data.len() < 18 {
                return None;
            }
            let tci = u16::from_be_bytes([data[14], data[15]]);
            let real_ethertype = u16::from_be_bytes([data[16], data[17]]);
            Some((
                EthHeader {
                    dst,
                    src,
                    vlan_tag: Some(VlanTag::from_tci(tci)),
                    ethertype: real_ethertype,
                },
                &data[18..],
            ))
        } else {
            // Untagged frame
            Some((
                EthHeader {
                    dst,
                    src,
                    vlan_tag: None,
                    ethertype: first_ethertype,
                },
                &data[14..],
            ))
        }
    }

    /// Write the ethernet header to a buffer.
    /// Returns the number of bytes written (14 or 18).
    pub fn write(&self, buf: &mut [u8]) -> usize {
        buf[0..6].copy_from_slice(&self.dst.0);
        buf[6..12].copy_from_slice(&self.src.0);

        if let Some(ref vlan) = self.vlan_tag {
            buf[12..14].copy_from_slice(&ETHERTYPE_8021Q.to_be_bytes());
            buf[14..16].copy_from_slice(&vlan.to_tci().to_be_bytes());
            buf[16..18].copy_from_slice(&self.ethertype.to_be_bytes());
            18
        } else {
            buf[12..14].copy_from_slice(&self.ethertype.to_be_bytes());
            14
        }
    }
}

// Host function declarations
extern "C" {
    fn net_get_mac(buf_ptr: *mut u8) -> i32;
    fn net_tx_frame(buf_ptr: *const u8, frame_len: i32) -> i32;
    fn net_rx_frame(buf_ptr: *mut u8, buf_len: i32) -> i32;
    fn net_rx_frame_blocking(buf_ptr: *mut u8, buf_len: i32, timeout_ms: i32) -> i32;
    fn net_set_promiscuous(enabled: i32) -> i32;
}

/// Get the local MAC address from the host.
pub fn get_local_mac() -> MacAddr {
    let mut buf = [0u8; 6];
    unsafe {
        net_get_mac(buf.as_mut_ptr());
    }
    MacAddr(buf)
}

/// Send a raw ethernet frame.
pub fn send_frame(frame: &[u8]) -> bool {
    unsafe { net_tx_frame(frame.as_ptr(), frame.len() as i32) == 0 }
}

/// Non-blocking receive of a raw ethernet frame.
/// Returns the number of bytes received, or None if no frame available.
pub fn recv_frame(buf: &mut [u8]) -> Option<usize> {
    let len = unsafe { net_rx_frame(buf.as_mut_ptr(), buf.len() as i32) };
    if len > 0 {
        Some(len as usize)
    } else {
        None
    }
}

/// Blocking receive with timeout.
pub fn recv_frame_blocking(buf: &mut [u8], timeout_ms: i32) -> Option<usize> {
    let len = unsafe { net_rx_frame_blocking(buf.as_mut_ptr(), buf.len() as i32, timeout_ms) };
    if len > 0 {
        Some(len as usize)
    } else {
        None
    }
}

/// Set promiscuous mode.
pub fn set_promiscuous(enabled: bool) -> bool {
    unsafe { net_set_promiscuous(if enabled { 1 } else { 0 }) == 0 }
}

/// Build and send an ethernet frame with the given payload.
/// Supports optional 802.1Q VLAN tagging.
pub fn send_eth_frame(
    tx_buf: &mut [u8],
    src: &MacAddr,
    dst: &MacAddr,
    ethertype: u16,
    vlan_tag: Option<&VlanTag>,
    payload: &[u8],
) -> bool {
    let hdr = EthHeader {
        dst: *dst,
        src: *src,
        vlan_tag: vlan_tag.copied(),
        ethertype,
    };
    let hdr_size = hdr.header_size();
    let frame_len = hdr_size + payload.len();
    if frame_len > MAX_FRAME_SIZE || frame_len > tx_buf.len() {
        return false;
    }

    hdr.write(&mut tx_buf[..hdr_size]);
    tx_buf[hdr_size..frame_len].copy_from_slice(payload);
    send_frame(&tx_buf[..frame_len])
}
