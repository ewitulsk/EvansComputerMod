//! IPv4 packet parsing, construction, and routing.

use super::types::{Ipv4Addr, MacAddr};
use super::checksum::internet_checksum;

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;

/// IPv4 header (20 bytes, no options).
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

    /// Parse an IPv4 header from raw bytes.
    /// Returns the header and a slice of the payload.
    pub fn parse(data: &[u8]) -> Option<(Self, &[u8])> {
        if data.len() < 20 {
            return None;
        }
        let version_ihl = data[0];
        let version = version_ihl >> 4;
        let ihl = (version_ihl & 0x0f) as usize;
        if version != 4 || ihl < 5 {
            return None;
        }
        let header_len = ihl * 4;
        if data.len() < header_len {
            return None;
        }

        let total_length = u16::from_be_bytes([data[2], data[3]]) as usize;
        if data.len() < total_length {
            return None;
        }

        // Verify checksum
        let computed = internet_checksum(&data[..header_len]);
        if computed != 0 {
            return None; // bad checksum
        }

        let hdr = Ipv4Header {
            version_ihl,
            dscp_ecn: data[1],
            total_length: total_length as u16,
            identification: u16::from_be_bytes([data[4], data[5]]),
            flags_fragment: u16::from_be_bytes([data[6], data[7]]),
            ttl: data[8],
            protocol: data[9],
            checksum: u16::from_be_bytes([data[10], data[11]]),
            src: Ipv4Addr::from_bytes(&data[12..16]),
            dst: Ipv4Addr::from_bytes(&data[16..20]),
        };

        let payload_start = header_len;
        let payload_end = total_length;
        Some((hdr, &data[payload_start..payload_end]))
    }

    /// Serialize the IPv4 header into a buffer (20 bytes).
    /// Computes the checksum automatically.
    pub fn serialize(&self, buf: &mut [u8]) {
        buf[0] = self.version_ihl;
        buf[1] = self.dscp_ecn;
        buf[2..4].copy_from_slice(&self.total_length.to_be_bytes());
        buf[4..6].copy_from_slice(&self.identification.to_be_bytes());
        buf[6..8].copy_from_slice(&self.flags_fragment.to_be_bytes());
        buf[8] = self.ttl;
        buf[9] = self.protocol;
        buf[10] = 0; // checksum = 0 for computation
        buf[11] = 0;
        buf[12..16].copy_from_slice(&self.src.0);
        buf[16..20].copy_from_slice(&self.dst.0);

        // Compute and write checksum
        let cksum = internet_checksum(&buf[..20]);
        buf[10..12].copy_from_slice(&cksum.to_be_bytes());
    }

    /// Create a standard IPv4 header for outgoing packets.
    pub fn new_outgoing(
        src: Ipv4Addr,
        dst: Ipv4Addr,
        protocol: u8,
        payload_len: usize,
        id: u16,
    ) -> Self {
        Ipv4Header {
            version_ihl: 0x45, // v4, 5 words (no options)
            dscp_ecn: 0,
            total_length: (20 + payload_len) as u16,
            identification: id,
            flags_fragment: 0x4000, // Don't Fragment
            ttl: 64,
            protocol,
            checksum: 0, // computed in serialize
            src,
            dst,
        }
    }
}

/// Simple routing table: local subnet + default gateway.
pub struct RoutingTable {
    pub local_ip: Ipv4Addr,
    pub subnet_mask: Ipv4Addr,
    pub gateway: Ipv4Addr,
}

impl RoutingTable {
    pub const fn new() -> Self {
        RoutingTable {
            local_ip: Ipv4Addr::ZERO,
            subnet_mask: Ipv4Addr::ZERO,
            gateway: Ipv4Addr::ZERO,
        }
    }

    /// Determine the next-hop IP for a destination.
    /// If dst is on the local subnet, return dst. Otherwise return gateway.
    pub fn next_hop(&self, dst: &Ipv4Addr) -> Ipv4Addr {
        if self.is_local(dst) || *dst == Ipv4Addr::BROADCAST {
            *dst
        } else {
            self.gateway
        }
    }

    /// Check if dst is on the local subnet.
    pub fn is_local(&self, dst: &Ipv4Addr) -> bool {
        self.local_ip.same_subnet(dst, &self.subnet_mask)
    }
}
