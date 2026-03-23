//! ARP (Address Resolution Protocol) — resolve IPv4 addresses to MAC addresses.

use super::types::{MacAddr, Ipv4Addr};
use super::eth::{self, EthHeader, ETHERTYPE_ARP};

const ARP_TABLE_SIZE: usize = 32;
const ARP_TTL_MS: i64 = 300_000; // 5 minutes

// ARP operation codes
pub const ARP_REQUEST: u16 = 1;
pub const ARP_REPLY: u16 = 2;

// ARP hardware/protocol constants
const HW_ETHERNET: u16 = 1;
const PROTO_IPV4: u16 = 0x0800;

/// An entry in the ARP cache.
#[derive(Clone, Copy)]
struct ArpEntry {
    ip: Ipv4Addr,
    mac: MacAddr,
    expires_ms: i64,
    valid: bool,
}

impl ArpEntry {
    const EMPTY: Self = ArpEntry {
        ip: Ipv4Addr::ZERO,
        mac: MacAddr::ZERO,
        expires_ms: 0,
        valid: false,
    };
}

/// ARP cache table.
pub struct ArpTable {
    entries: [ArpEntry; ARP_TABLE_SIZE],
}

impl ArpTable {
    pub const fn new() -> Self {
        ArpTable {
            entries: [ArpEntry::EMPTY; ARP_TABLE_SIZE],
        }
    }

    /// Look up a MAC address for the given IP.
    pub fn lookup(&self, ip: &Ipv4Addr, now_ms: i64) -> Option<MacAddr> {
        for e in &self.entries {
            if e.valid && e.ip == *ip && now_ms < e.expires_ms {
                return Some(e.mac);
            }
        }
        None
    }

    /// Insert or update an ARP entry.
    pub fn insert(&mut self, ip: Ipv4Addr, mac: MacAddr, now_ms: i64) {
        // Update existing
        for e in self.entries.iter_mut() {
            if e.valid && e.ip == ip {
                e.mac = mac;
                e.expires_ms = now_ms + ARP_TTL_MS;
                return;
            }
        }
        // Find empty slot
        for e in self.entries.iter_mut() {
            if !e.valid {
                e.ip = ip;
                e.mac = mac;
                e.expires_ms = now_ms + ARP_TTL_MS;
                e.valid = true;
                return;
            }
        }
        // Evict oldest
        let mut oldest_idx = 0;
        let mut oldest_time = i64::MAX;
        for (i, e) in self.entries.iter().enumerate() {
            if e.expires_ms < oldest_time {
                oldest_time = e.expires_ms;
                oldest_idx = i;
            }
        }
        self.entries[oldest_idx] = ArpEntry {
            ip,
            mac,
            expires_ms: now_ms + ARP_TTL_MS,
            valid: true,
        };
    }

    /// Remove expired entries.
    pub fn evict_expired(&mut self, now_ms: i64) {
        for e in self.entries.iter_mut() {
            if e.valid && now_ms >= e.expires_ms {
                e.valid = false;
            }
        }
    }
}

/// Parsed ARP packet (28 bytes for IPv4-over-Ethernet).
pub struct ArpPacket {
    pub operation: u16,
    pub sender_mac: MacAddr,
    pub sender_ip: Ipv4Addr,
    pub target_mac: MacAddr,
    pub target_ip: Ipv4Addr,
}

impl ArpPacket {
    pub const SIZE: usize = 28;

    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 28 {
            return None;
        }
        let hw_type = u16::from_be_bytes([data[0], data[1]]);
        let proto_type = u16::from_be_bytes([data[2], data[3]]);
        let hw_len = data[4];
        let proto_len = data[5];
        if hw_type != HW_ETHERNET || proto_type != PROTO_IPV4 || hw_len != 6 || proto_len != 4 {
            return None;
        }
        Some(ArpPacket {
            operation: u16::from_be_bytes([data[6], data[7]]),
            sender_mac: MacAddr::from_bytes(&data[8..14]),
            sender_ip: Ipv4Addr::from_bytes(&data[14..18]),
            target_mac: MacAddr::from_bytes(&data[18..24]),
            target_ip: Ipv4Addr::from_bytes(&data[24..28]),
        })
    }

    pub fn serialize(&self, buf: &mut [u8]) -> usize {
        buf[0..2].copy_from_slice(&HW_ETHERNET.to_be_bytes());
        buf[2..4].copy_from_slice(&PROTO_IPV4.to_be_bytes());
        buf[4] = 6; // hw addr len
        buf[5] = 4; // proto addr len
        buf[6..8].copy_from_slice(&self.operation.to_be_bytes());
        buf[8..14].copy_from_slice(&self.sender_mac.0);
        buf[14..18].copy_from_slice(&self.sender_ip.0);
        buf[18..24].copy_from_slice(&self.target_mac.0);
        buf[24..28].copy_from_slice(&self.target_ip.0);
        28
    }
}

/// Send an ARP request (broadcast) asking who has `target_ip`.
pub fn send_arp_request(
    tx_buf: &mut [u8],
    our_mac: &MacAddr,
    our_ip: &Ipv4Addr,
    target_ip: &Ipv4Addr,
) {
    let pkt = ArpPacket {
        operation: ARP_REQUEST,
        sender_mac: *our_mac,
        sender_ip: *our_ip,
        target_mac: MacAddr::ZERO,
        target_ip: *target_ip,
    };
    let mut arp_buf = [0u8; 28];
    pkt.serialize(&mut arp_buf);
    eth::send_eth_frame(tx_buf, our_mac, &MacAddr::BROADCAST, ETHERTYPE_ARP, &arp_buf);
}

/// Send an ARP reply.
pub fn send_arp_reply(
    tx_buf: &mut [u8],
    our_mac: &MacAddr,
    our_ip: &Ipv4Addr,
    target_mac: &MacAddr,
    target_ip: &Ipv4Addr,
) {
    let pkt = ArpPacket {
        operation: ARP_REPLY,
        sender_mac: *our_mac,
        sender_ip: *our_ip,
        target_mac: *target_mac,
        target_ip: *target_ip,
    };
    let mut arp_buf = [0u8; 28];
    pkt.serialize(&mut arp_buf);
    eth::send_eth_frame(tx_buf, our_mac, target_mac, ETHERTYPE_ARP, &arp_buf);
}

/// Send a gratuitous ARP (announce our IP/MAC binding).
pub fn send_gratuitous_arp(
    tx_buf: &mut [u8],
    our_mac: &MacAddr,
    our_ip: &Ipv4Addr,
) {
    send_arp_request(tx_buf, our_mac, our_ip, our_ip);
}
