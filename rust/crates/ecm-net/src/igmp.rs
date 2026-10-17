//! IGMPv2 (RFC 2236): the membership query, report and leave messages a host
//! exchanges with routers and snooping switches, and the group → MAC mapping
//! (RFC 1112 §6.4). Group membership itself lives in the stack.

use super::checksum::internet_checksum;
use super::types::{Ipv4Addr, MacAddr};

pub const PROTO_IGMP: u8 = 2;

pub const MEMBERSHIP_QUERY: u8 = 0x11;
pub const V1_MEMBERSHIP_REPORT: u8 = 0x12;
pub const V2_MEMBERSHIP_REPORT: u8 = 0x16;
pub const LEAVE_GROUP: u8 = 0x17;

/// 224.0.0.1: every multicast host on the link (always joined, never reported).
pub const ALL_HOSTS: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 1);
/// 224.0.0.2: every multicast router on the link (where leaves go).
pub const ALL_ROUTERS: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 2);

/// One IGMPv2 message (8 bytes on the wire).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IgmpMessage {
    pub kind: u8,
    /// Max response time in 1/10 s (queries only).
    pub max_resp: u8,
    /// 0.0.0.0 in a general query.
    pub group: Ipv4Addr,
}

impl IgmpMessage {
    pub fn to_bytes(&self) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[0] = self.kind;
        b[1] = self.max_resp;
        b[4..8].copy_from_slice(&self.group.0);
        let sum = internet_checksum(&b);
        b[2..4].copy_from_slice(&sum.to_be_bytes());
        b
    }

    /// Parse and verify the checksum (longer IGMPv3 queries are read as v2 ones).
    pub fn parse(data: &[u8]) -> Option<IgmpMessage> {
        if data.len() < 8 || internet_checksum(data) != 0 {
            return None;
        }
        Some(IgmpMessage { kind: data[0], max_resp: data[1], group: Ipv4Addr::from_bytes(&data[4..8]) })
    }
}

/// The Ethernet address a group's datagrams are sent to: 01:00:5e and the group's low 23 bits.
pub fn group_mac(group: Ipv4Addr) -> MacAddr {
    let g = group.0;
    MacAddr([0x01, 0x00, 0x5e, g[1] & 0x7f, g[2], g[3]])
}

/// Link-local groups (224.0.0.0/24): never routed, and only 224.0.0.1 is not reported.
pub fn is_link_local(group: Ipv4Addr) -> bool {
    group.0[..3] == [224, 0, 0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_with_checksums() {
        let m = IgmpMessage { kind: V2_MEMBERSHIP_REPORT, max_resp: 0, group: Ipv4Addr::new(224, 0, 0, 254) };
        let b = m.to_bytes();
        assert_eq!(IgmpMessage::parse(&b), Some(m));
        let mut bad = b;
        bad[7] ^= 1;
        assert_eq!(IgmpMessage::parse(&bad), None);
        assert_eq!(IgmpMessage::parse(&b[..7]), None);
    }

    #[test]
    fn groups_map_to_their_mac() {
        assert_eq!(group_mac(Ipv4Addr::new(224, 0, 0, 251)).0, [0x01, 0x00, 0x5e, 0x00, 0x00, 0xfb]);
        // The 24th bit is lost: 224.128.1.2 and 224.0.1.2 share a MAC.
        assert_eq!(group_mac(Ipv4Addr::new(239, 128, 1, 2)), group_mac(Ipv4Addr::new(224, 0, 1, 2)));
        assert!(group_mac(ALL_HOSTS).is_multicast());
        assert!(is_link_local(ALL_ROUTERS) && !is_link_local(Ipv4Addr::new(239, 1, 2, 3)));
    }
}
