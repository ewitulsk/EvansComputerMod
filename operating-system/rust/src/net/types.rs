//! Shared network types: addresses, errors, constants.

use core::fmt;

/// Maximum Transmission Unit (ethernet payload, excludes ethernet header).
pub const MTU: usize = 1500;
/// Maximum ethernet frame size (14 header + 4 VLAN tag + 1500 payload).
pub const MAX_FRAME_SIZE: usize = 1518;

/// 6-byte MAC address.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    pub const BROADCAST: MacAddr = MacAddr([0xff; 6]);
    pub const ZERO: MacAddr = MacAddr([0; 6]);

    pub fn from_bytes(b: &[u8]) -> Self {
        let mut a = [0u8; 6];
        a.copy_from_slice(&b[..6]);
        MacAddr(a)
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

/// 4-byte IPv4 address.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    pub const ZERO: Ipv4Addr = Ipv4Addr([0; 4]);
    pub const BROADCAST: Ipv4Addr = Ipv4Addr([255, 255, 255, 255]);

    pub fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Ipv4Addr([a, b, c, d])
    }

    pub fn from_bytes(b: &[u8]) -> Self {
        let mut a = [0u8; 4];
        a.copy_from_slice(&b[..4]);
        Ipv4Addr(a)
    }

    pub fn to_u32(&self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    pub fn from_u32(v: u32) -> Self {
        Ipv4Addr(v.to_be_bytes())
    }

    /// Parse an IPv4 address from a dotted-decimal string like "10.0.0.1".
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = [0u8; 4];
        let mut idx = 0;
        for part in s.split('.') {
            if idx >= 4 {
                return None;
            }
            parts[idx] = part.parse::<u8>().ok()?;
            idx += 1;
        }
        if idx != 4 {
            return None;
        }
        Some(Ipv4Addr(parts))
    }

    /// Check if this IP is on the same subnet as another, given a mask.
    pub fn same_subnet(&self, other: &Ipv4Addr, mask: &Ipv4Addr) -> bool {
        for i in 0..4 {
            if (self.0[i] & mask.0[i]) != (other.0[i] & mask.0[i]) {
                return false;
            }
        }
        true
    }
}

impl fmt::Display for Ipv4Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

/// Socket address (IP + port).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SocketAddr {
    pub ip: Ipv4Addr,
    pub port: u16,
}

impl fmt::Display for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.ip, self.port)
    }
}

/// Network errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetError {
    NotConfigured,
    NoRoute,
    ArpTimeout,
    ConnectionRefused,
    ConnectionReset,
    TimedOut,
    WouldBlock,
    BufferFull,
    NotConnected,
    AddrInUse,
    InvalidPacket,
    NoSockets,
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::NotConfigured => write!(f, "network not configured"),
            NetError::NoRoute => write!(f, "no route to host"),
            NetError::ArpTimeout => write!(f, "ARP timeout"),
            NetError::ConnectionRefused => write!(f, "connection refused"),
            NetError::ConnectionReset => write!(f, "connection reset"),
            NetError::TimedOut => write!(f, "timed out"),
            NetError::WouldBlock => write!(f, "would block"),
            NetError::BufferFull => write!(f, "buffer full"),
            NetError::NotConnected => write!(f, "not connected"),
            NetError::AddrInUse => write!(f, "address in use"),
            NetError::InvalidPacket => write!(f, "invalid packet"),
            NetError::NoSockets => write!(f, "no sockets available"),
        }
    }
}
