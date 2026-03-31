//! RFC 1071 internet checksum, used by IPv4, ICMP, TCP, and UDP.

use super::types::Ipv4Addr;

/// Compute the one's complement checksum over the given data.
pub fn internet_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Compute the TCP/UDP checksum including the IPv4 pseudo-header.
pub fn pseudo_header_checksum(
    src: &Ipv4Addr,
    dst: &Ipv4Addr,
    protocol: u8,
    segment: &[u8],
) -> u16 {
    let mut sum: u32 = 0;

    // Pseudo-header: src IP (4) + dst IP (4) + zero (1) + protocol (1) + length (2)
    sum += u16::from_be_bytes([src.0[0], src.0[1]]) as u32;
    sum += u16::from_be_bytes([src.0[2], src.0[3]]) as u32;
    sum += u16::from_be_bytes([dst.0[0], dst.0[1]]) as u32;
    sum += u16::from_be_bytes([dst.0[2], dst.0[3]]) as u32;
    sum += protocol as u32;
    sum += segment.len() as u32;

    // Segment data
    let mut i = 0;
    while i + 1 < segment.len() {
        sum += u16::from_be_bytes([segment[i], segment[i + 1]]) as u32;
        i += 2;
    }
    if i < segment.len() {
        sum += (segment[i] as u32) << 8;
    }

    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}
