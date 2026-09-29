//! Shared network types: addresses, errors, constants.

use core::fmt;

/// Maximum Transmission Unit (ethernet payload, excludes ethernet header).
pub const MTU: usize = 1500;
/// Maximum ethernet frame size (14 header + 4 VLAN tag + 1500 payload).
pub const MAX_FRAME_SIZE: usize = 1518;

/// 6-byte MAC address.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    pub const BROADCAST: MacAddr = MacAddr([0xff; 6]);
    pub const ZERO: MacAddr = MacAddr([0; 6]);

    /// Build from the first 6 bytes of `b`. Missing bytes are zero (never panics).
    pub fn from_bytes(b: &[u8]) -> Self {
        let mut a = [0u8; 6];
        let n = b.len().min(6);
        a[..n].copy_from_slice(&b[..n]);
        MacAddr(a)
    }

    pub fn is_broadcast(&self) -> bool {
        *self == Self::BROADCAST
    }

    /// Group bit set (includes broadcast).
    pub fn is_multicast(&self) -> bool {
        self.0[0] & 1 != 0
    }

    pub fn is_unicast(&self) -> bool {
        !self.is_multicast() && *self != Self::ZERO
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

impl fmt::Debug for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// 4-byte IPv4 address.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    pub const ZERO: Ipv4Addr = Ipv4Addr([0; 4]);
    pub const BROADCAST: Ipv4Addr = Ipv4Addr([255, 255, 255, 255]);
    pub const LOCALHOST: Ipv4Addr = Ipv4Addr([127, 0, 0, 1]);

    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Ipv4Addr([a, b, c, d])
    }

    /// Build from the first 4 bytes of `b`. Missing bytes are zero (never panics).
    pub fn from_bytes(b: &[u8]) -> Self {
        let mut a = [0u8; 4];
        let n = b.len().min(4);
        a[..n].copy_from_slice(&b[..n]);
        Ipv4Addr(a)
    }

    pub fn to_u32(&self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    pub fn from_u32(v: u32) -> Self {
        Ipv4Addr(v.to_be_bytes())
    }

    pub fn is_unspecified(&self) -> bool {
        *self == Self::ZERO
    }

    pub fn is_broadcast(&self) -> bool {
        *self == Self::BROADCAST
    }

    /// 127.0.0.0/8
    pub fn is_loopback(&self) -> bool {
        self.0[0] == 127
    }

    /// 224.0.0.0/4
    pub fn is_multicast(&self) -> bool {
        self.0[0] & 0xf0 == 0xe0
    }

    /// Parse an IPv4 address from a dotted-decimal string like "10.0.0.1".
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = [0u8; 4];
        let mut idx = 0;
        for part in s.split('.') {
            if idx >= 4 || part.is_empty() || part.len() > 3 || !part.bytes().all(|c| c.is_ascii_digit()) {
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
        (self.to_u32() & mask.to_u32()) == (other.to_u32() & mask.to_u32())
    }

    /// Check if this IP is on the same subnet as another, given a CIDR prefix length.
    pub fn same_subnet_prefix(&self, other: &Ipv4Addr, prefix_len: u8) -> bool {
        let mask = Ipv4Addr::mask_from_prefix(prefix_len);
        self.same_subnet(other, &mask)
    }

    /// Convert a CIDR prefix length (0-32) to a subnet mask.
    /// e.g. 24 → 255.255.255.0, 16 → 255.255.0.0
    pub fn mask_from_prefix(prefix_len: u8) -> Ipv4Addr {
        if prefix_len == 0 {
            return Ipv4Addr::ZERO;
        }
        if prefix_len >= 32 {
            return Ipv4Addr([255, 255, 255, 255]);
        }
        let mask = !0u32 << (32 - prefix_len);
        Ipv4Addr::from_u32(mask)
    }

    /// Convert a subnet mask to CIDR prefix length.
    /// e.g. 255.255.255.0 → 24
    pub fn prefix_from_mask(mask: &Ipv4Addr) -> u8 {
        let v = mask.to_u32();
        v.leading_ones() as u8
    }

    /// Apply subnet mask from prefix length to get the network address.
    /// e.g. 10.0.0.5 with prefix 24 → 10.0.0.0
    pub fn network_addr(&self, prefix_len: u8) -> Ipv4Addr {
        let mask = Ipv4Addr::mask_from_prefix(prefix_len);
        Ipv4Addr::from_u32(self.to_u32() & mask.to_u32())
    }

    /// Subnet-directed broadcast address for this IP/prefix.
    pub fn broadcast_addr(&self, prefix_len: u8) -> Ipv4Addr {
        let mask = Ipv4Addr::mask_from_prefix(prefix_len);
        Ipv4Addr::from_u32(self.to_u32() | !mask.to_u32())
    }

    /// Parse CIDR notation like "10.0.0.1/24". Returns (ip, prefix_len).
    pub fn parse_cidr(s: &str) -> Option<(Ipv4Addr, u8)> {
        let (ip_s, prefix_s) = s.split_once('/')?;
        let ip = Ipv4Addr::parse(ip_s)?;
        if prefix_s.is_empty() || prefix_s.len() > 2 || !prefix_s.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let prefix: u8 = prefix_s.parse().ok()?;
        if prefix > 32 {
            return None;
        }
        Some((ip, prefix))
    }
}

impl fmt::Display for Ipv4Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

impl fmt::Debug for Ipv4Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Socket address (IP + port).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SocketAddr {
    pub ip: Ipv4Addr,
    pub port: u16,
}

impl SocketAddr {
    pub const fn new(ip: Ipv4Addr, port: u16) -> Self {
        SocketAddr { ip, port }
    }
}

impl fmt::Display for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.ip, self.port)
    }
}

impl fmt::Debug for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Network errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    /// Stale or wrong-kind socket/query handle.
    BadHandle,
    /// Next hop did not answer ARP.
    HostUnreachable,
    /// Payload too large for one datagram (UDP > 1472, ICMP > 1480).
    MessageTooLong,
    /// Malformed argument from the caller (bad name, port 0, unknown iface, ...).
    InvalidInput,
    /// Lookup miss: no such route, or DNS NXDOMAIN / no A record.
    NotFound,
    /// Connection torn down locally (interface removed, ...).
    ConnectionAborted,
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            NetError::NotConfigured => "network not configured",
            NetError::NoRoute => "no route to host",
            NetError::ArpTimeout => "ARP timeout",
            NetError::ConnectionRefused => "connection refused",
            NetError::ConnectionReset => "connection reset",
            NetError::TimedOut => "timed out",
            NetError::WouldBlock => "would block",
            NetError::BufferFull => "buffer full",
            NetError::NotConnected => "not connected",
            NetError::AddrInUse => "address in use",
            NetError::InvalidPacket => "invalid packet",
            NetError::NoSockets => "no sockets available",
            NetError::BadHandle => "bad handle",
            NetError::HostUnreachable => "host unreachable",
            NetError::MessageTooLong => "message too long",
            NetError::InvalidInput => "invalid argument",
            NetError::NotFound => "not found",
            NetError::ConnectionAborted => "connection aborted",
        };
        f.write_str(s)
    }
}

impl std::error::Error for NetError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_bytes_short_input_does_not_panic() {
        assert_eq!(MacAddr::from_bytes(&[1, 2]), MacAddr([1, 2, 0, 0, 0, 0]));
        assert_eq!(Ipv4Addr::from_bytes(&[]), Ipv4Addr::ZERO);
        assert_eq!(Ipv4Addr::from_bytes(&[1, 2, 3, 4, 5]), Ipv4Addr::new(1, 2, 3, 4));
    }

    #[test]
    fn parse_ip() {
        assert_eq!(Ipv4Addr::parse("10.0.0.1"), Some(Ipv4Addr::new(10, 0, 0, 1)));
        for bad in [
            "", "1.2.3", "1.2.3.4.5", "256.1.1.1", "a.b.c.d", "1..2.3", "+1.2.3.4", "1.2.3.4 ",
            "0001.2.3.4", "\u{e9}.1.1.1", ".", "1.2.3.",
        ] {
            assert_eq!(Ipv4Addr::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn parse_cidr() {
        assert_eq!(Ipv4Addr::parse_cidr("10.0.0.1/24"), Some((Ipv4Addr::new(10, 0, 0, 1), 24)));
        assert_eq!(Ipv4Addr::parse_cidr("10.0.0.1/0"), Some((Ipv4Addr::new(10, 0, 0, 1), 0)));
        for bad in ["10.0.0.1", "10.0.0.1/", "10.0.0.1/33", "/24", "10.0.0.1/+4", "10.0.0.1/2/3", "x/1", "1.1.1.1/\u{e9}"] {
            assert_eq!(Ipv4Addr::parse_cidr(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn masks() {
        assert_eq!(Ipv4Addr::mask_from_prefix(24), Ipv4Addr::new(255, 255, 255, 0));
        assert_eq!(Ipv4Addr::mask_from_prefix(0), Ipv4Addr::ZERO);
        assert_eq!(Ipv4Addr::mask_from_prefix(200), Ipv4Addr::BROADCAST);
        assert_eq!(Ipv4Addr::prefix_from_mask(&Ipv4Addr::new(255, 255, 0, 0)), 16);
        assert_eq!(Ipv4Addr::new(10, 1, 2, 3).network_addr(16), Ipv4Addr::new(10, 1, 0, 0));
        assert_eq!(Ipv4Addr::new(10, 1, 2, 3).broadcast_addr(24), Ipv4Addr::new(10, 1, 2, 255));
        assert!(Ipv4Addr::new(10, 1, 2, 3).same_subnet_prefix(&Ipv4Addr::new(10, 1, 9, 9), 16));
        assert!(!Ipv4Addr::new(10, 1, 2, 3).same_subnet_prefix(&Ipv4Addr::new(10, 2, 9, 9), 16));
    }

    #[test]
    fn mac_classes() {
        assert!(MacAddr::BROADCAST.is_multicast());
        assert!(MacAddr([0x01, 0x80, 0xc2, 0, 0, 0]).is_multicast());
        assert!(MacAddr([0x02, 0, 0, 0, 0, 1]).is_unicast());
        assert!(!MacAddr::ZERO.is_unicast());
        assert_eq!(format!("{}", MacAddr([0xde, 0xad, 0xbe, 0xef, 0, 1])), "de:ad:be:ef:00:01");
    }
}
