//! ICMP (Internet Control Message Protocol) — ping and error messages.

use super::checksum::internet_checksum;

pub const ICMP_ECHO_REPLY: u8 = 0;
pub const ICMP_DEST_UNREACHABLE: u8 = 3;
pub const ICMP_ECHO_REQUEST: u8 = 8;

/// Parsed ICMP packet.
pub struct IcmpPacket {
    pub icmp_type: u8,
    pub code: u8,
    pub checksum: u16,
    pub id: u16,
    pub seq: u16,
    pub payload_len: usize,
}

impl IcmpPacket {
    /// Minimum ICMP header size (type + code + checksum + id + seq).
    pub const HEADER_SIZE: usize = 8;

    /// Parse an ICMP packet from raw bytes.
    pub fn parse(data: &[u8]) -> Option<(Self, &[u8])> {
        if data.len() < 8 {
            return None;
        }

        // Verify checksum over entire ICMP message
        let computed = internet_checksum(data);
        if computed != 0 {
            return None;
        }

        let pkt = IcmpPacket {
            icmp_type: data[0],
            code: data[1],
            checksum: u16::from_be_bytes([data[2], data[3]]),
            id: u16::from_be_bytes([data[4], data[5]]),
            seq: u16::from_be_bytes([data[6], data[7]]),
            payload_len: data.len() - 8,
        };

        Some((pkt, &data[8..]))
    }

    /// Serialize an ICMP echo request/reply into a buffer.
    /// Returns total length written. Computes checksum automatically.
    pub fn serialize_echo(
        buf: &mut [u8],
        icmp_type: u8,
        id: u16,
        seq: u16,
        payload: &[u8],
    ) -> usize {
        let total_len = 8 + payload.len();
        if buf.len() < total_len {
            return 0;
        }

        buf[0] = icmp_type;
        buf[1] = 0; // code
        buf[2] = 0; // checksum placeholder
        buf[3] = 0;
        buf[4..6].copy_from_slice(&id.to_be_bytes());
        buf[6..8].copy_from_slice(&seq.to_be_bytes());
        buf[8..total_len].copy_from_slice(payload);

        // Compute checksum
        let cksum = internet_checksum(&buf[..total_len]);
        buf[2..4].copy_from_slice(&cksum.to_be_bytes());

        total_len
    }
}
