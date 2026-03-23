//! Ethernet frame parsing, construction, and host function wrappers.

use super::types::{MacAddr, MAX_FRAME_SIZE};

pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;

/// Parsed ethernet frame header (14 bytes).
pub struct EthHeader {
    pub dst: MacAddr,
    pub src: MacAddr,
    pub ethertype: u16,
}

impl EthHeader {
    pub const SIZE: usize = 14;

    /// Parse an ethernet header from raw bytes.
    /// Returns the header and a slice of the payload.
    pub fn parse(data: &[u8]) -> Option<(Self, &[u8])> {
        if data.len() < 14 {
            return None;
        }
        Some((
            EthHeader {
                dst: MacAddr::from_bytes(&data[0..6]),
                src: MacAddr::from_bytes(&data[6..12]),
                ethertype: u16::from_be_bytes([data[12], data[13]]),
            },
            &data[14..],
        ))
    }

    /// Write the ethernet header to a buffer. Buffer must be >= 14 bytes.
    pub fn write(&self, buf: &mut [u8]) {
        buf[0..6].copy_from_slice(&self.dst.0);
        buf[6..12].copy_from_slice(&self.src.0);
        buf[12..14].copy_from_slice(&self.ethertype.to_be_bytes());
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
/// Uses a static TX buffer.
pub fn send_eth_frame(
    tx_buf: &mut [u8],
    src: &MacAddr,
    dst: &MacAddr,
    ethertype: u16,
    payload: &[u8],
) -> bool {
    let frame_len = EthHeader::SIZE + payload.len();
    if frame_len > MAX_FRAME_SIZE || frame_len > tx_buf.len() {
        return false;
    }

    let hdr = EthHeader {
        dst: *dst,
        src: *src,
        ethertype,
    };
    hdr.write(&mut tx_buf[..EthHeader::SIZE]);
    tx_buf[EthHeader::SIZE..frame_len].copy_from_slice(payload);
    send_frame(&tx_buf[..frame_len])
}
