//! ARP packet format (IPv4 over Ethernet). The neighbour cache lives in the stack.

use super::types::{Ipv4Addr, MacAddr};

pub const ARP_REQUEST: u16 = 1;
pub const ARP_REPLY: u16 = 2;

const HW_ETHERNET: u16 = 1;
const PROTO_IPV4: u16 = 0x0800;

/// Parsed ARP packet (28 bytes for IPv4-over-Ethernet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
        let d = data.get(..28)?;
        let hw_type = u16::from_be_bytes([d[0], d[1]]);
        let proto_type = u16::from_be_bytes([d[2], d[3]]);
        if hw_type != HW_ETHERNET || proto_type != PROTO_IPV4 || d[4] != 6 || d[5] != 4 {
            return None;
        }
        Some(ArpPacket {
            operation: u16::from_be_bytes([d[6], d[7]]),
            sender_mac: MacAddr::from_bytes(&d[8..14]),
            sender_ip: Ipv4Addr::from_bytes(&d[14..18]),
            target_mac: MacAddr::from_bytes(&d[18..24]),
            target_ip: Ipv4Addr::from_bytes(&d[24..28]),
        })
    }

    pub fn to_bytes(&self) -> [u8; 28] {
        let mut b = [0u8; 28];
        b[0..2].copy_from_slice(&HW_ETHERNET.to_be_bytes());
        b[2..4].copy_from_slice(&PROTO_IPV4.to_be_bytes());
        b[4] = 6;
        b[5] = 4;
        b[6..8].copy_from_slice(&self.operation.to_be_bytes());
        b[8..14].copy_from_slice(&self.sender_mac.0);
        b[14..18].copy_from_slice(&self.sender_ip.0);
        b[18..24].copy_from_slice(&self.target_mac.0);
        b[24..28].copy_from_slice(&self.target_ip.0);
        b
    }

    /// Serialize into `buf`. Returns 28, or 0 if `buf` is too small.
    pub fn serialize(&self, buf: &mut [u8]) -> usize {
        match buf.get_mut(..28) {
            Some(b) => {
                b.copy_from_slice(&self.to_bytes());
                28
            }
            None => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let p = ArpPacket {
            operation: ARP_REPLY,
            sender_mac: MacAddr([2, 1, 2, 3, 4, 5]),
            sender_ip: Ipv4Addr::new(10, 0, 0, 1),
            target_mac: MacAddr([2, 9, 9, 9, 9, 9]),
            target_ip: Ipv4Addr::new(10, 0, 0, 2),
        };
        let b = p.to_bytes();
        assert_eq!(ArpPacket::parse(&b), Some(p));
        let mut padded = b.to_vec();
        padded.extend_from_slice(&[0; 18]);
        assert_eq!(ArpPacket::parse(&padded), Some(p));
        assert_eq!(p.serialize(&mut [0u8; 27]), 0);
    }

    #[test]
    fn truncated_and_garbage() {
        let b = ArpPacket {
            operation: 1,
            sender_mac: MacAddr::ZERO,
            sender_ip: Ipv4Addr::ZERO,
            target_mac: MacAddr::ZERO,
            target_ip: Ipv4Addr::ZERO,
        }
        .to_bytes();
        for n in 0..28 {
            assert!(ArpPacket::parse(&b[..n]).is_none());
        }
        for i in 0..6 {
            let mut g = b;
            g[i] ^= 0x40;
            assert!(ArpPacket::parse(&g).is_none(), "byte {i}");
        }
    }
}
