//! IPv4 packet parsing, construction, and routing.

use super::types::{Ipv4Addr, MacAddr, NetError};
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

// ===== Routing Table =====

pub const MAX_ROUTES: usize = 16;

/// A single route entry.
#[derive(Clone, Copy)]
pub struct RouteEntry {
    pub destination: Ipv4Addr,  // network address (e.g. 10.0.0.0)
    pub prefix_len: u8,         // CIDR prefix (e.g. 24)
    pub gateway: Ipv4Addr,      // next-hop; ZERO = on-link/connected
    pub iface_index: usize,     // which interface to send from
    pub active: bool,
}

impl RouteEntry {
    const EMPTY: Self = RouteEntry {
        destination: Ipv4Addr::ZERO,
        prefix_len: 0,
        gateway: Ipv4Addr::ZERO,
        iface_index: 0,
        active: false,
    };

    /// Check if a destination IP matches this route entry.
    pub fn matches(&self, dst: &Ipv4Addr) -> bool {
        if !self.active {
            return false;
        }
        if self.prefix_len == 0 {
            return true; // default route matches everything
        }
        self.destination.same_subnet_prefix(dst, self.prefix_len)
    }
}

/// Routing table with longest-prefix-match lookup.
pub struct RoutingTable {
    pub entries: [RouteEntry; MAX_ROUTES],
}

impl RoutingTable {
    pub const fn new() -> Self {
        RoutingTable {
            entries: [RouteEntry::EMPTY; MAX_ROUTES],
        }
    }

    /// Add a route. Returns Ok(()) or Err if table is full.
    pub fn add_route(
        &mut self,
        destination: Ipv4Addr,
        prefix_len: u8,
        gateway: Ipv4Addr,
        iface_index: usize,
    ) -> Result<(), NetError> {
        // Check for duplicate
        for e in self.entries.iter_mut() {
            if e.active && e.destination == destination && e.prefix_len == prefix_len {
                // Update existing route
                e.gateway = gateway;
                e.iface_index = iface_index;
                return Ok(());
            }
        }
        // Find empty slot
        for e in self.entries.iter_mut() {
            if !e.active {
                *e = RouteEntry {
                    destination,
                    prefix_len,
                    gateway,
                    iface_index,
                    active: true,
                };
                return Ok(());
            }
        }
        Err(NetError::BufferFull)
    }

    /// Delete a route matching destination/prefix. Returns true if found.
    pub fn del_route(&mut self, destination: Ipv4Addr, prefix_len: u8) -> bool {
        for e in self.entries.iter_mut() {
            if e.active && e.destination == destination && e.prefix_len == prefix_len {
                e.active = false;
                return true;
            }
        }
        false
    }

    /// Delete all routes that use a specific interface.
    pub fn del_routes_for_iface(&mut self, iface_index: usize) {
        for e in self.entries.iter_mut() {
            if e.active && e.iface_index == iface_index {
                e.active = false;
            }
        }
    }

    /// Longest-prefix-match lookup.
    /// Returns (next_hop_ip, iface_index). next_hop is ZERO for connected routes (meaning dst itself).
    pub fn lookup(&self, dst: &Ipv4Addr) -> Option<(Ipv4Addr, usize)> {
        if *dst == Ipv4Addr::BROADCAST {
            // For broadcast, find any configured interface (prefer most specific)
            for e in &self.entries {
                if e.active && e.prefix_len > 0 {
                    return Some((Ipv4Addr::ZERO, e.iface_index));
                }
            }
            return None;
        }

        let mut best: Option<&RouteEntry> = None;
        for e in &self.entries {
            if !e.active {
                continue;
            }
            if !e.matches(dst) {
                continue;
            }
            match best {
                None => best = Some(e),
                Some(b) if e.prefix_len > b.prefix_len => best = Some(e),
                _ => {}
            }
        }
        best.map(|e| (e.gateway, e.iface_index))
    }

    /// Clear all routes.
    pub fn clear(&mut self) {
        for e in self.entries.iter_mut() {
            e.active = false;
        }
    }

    /// Count active routes.
    pub fn count(&self) -> usize {
        self.entries.iter().filter(|e| e.active).count()
    }
}
