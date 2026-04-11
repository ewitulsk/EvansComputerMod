//! Netlink helpers shared by `ifconfig`, `ip`, and any other CLI that talks
//! rtnetlink. These were previously duplicated between the two programs.

use crate::NetTools;
use ecm_host_abi::netlink::*;

/// Send an `NLM_F_REQUEST | NLM_F_DUMP` request for `msg_type` and return the
/// full multi-part response collected by the backend.
pub fn nl_dump(nt: &mut dyn NetTools, msg_type: u16, payload_size: usize) -> Vec<u8> {
    let msg_len = NLMSG_HDR_SIZE + payload_size;
    let mut req = vec![0u8; msg_len];
    NlMsgHdr {
        nlmsg_len: msg_len as u32,
        nlmsg_type: msg_type,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_DUMP,
        nlmsg_seq: 1,
        nlmsg_pid: 0,
    }
    .serialize(&mut req);
    nt.netlink_request(&req)
}

/// Walk the response of an `RTM_GETLINK` dump and return the `ifi_index` of the
/// interface whose `IFLA_IFNAME` matches `name`, or -1 if not found.
pub fn find_iface_index(nt: &mut dyn NetTools, name: &str) -> i32 {
    let links = nl_dump(nt, RTM_GETLINK, IFINFOMSG_SIZE);
    let mut off = 0;
    while off < links.len() {
        let hdr = match NlMsgHdr::parse(&links[off..]) {
            Some(h) => h,
            None => break,
        };
        if hdr.nlmsg_type == NLMSG_DONE {
            break;
        }
        if hdr.nlmsg_type == RTM_NEWLINK {
            let payload = &links[off + NLMSG_HDR_SIZE..off + hdr.nlmsg_len as usize];
            let ifinfo = IfInfoMsg::parse(payload).unwrap_or_default();
            let mut attr_off = IFINFOMSG_SIZE;
            while let Some((atype, adata, next)) = parse_attr(payload, attr_off) {
                if atype == IFLA_IFNAME {
                    let end = adata.iter().position(|&b| b == 0).unwrap_or(adata.len());
                    if core::str::from_utf8(&adata[..end]).unwrap_or("") == name {
                        return ifinfo.ifi_index;
                    }
                }
                attr_off = next;
            }
        }
        off += nlmsg_align(hdr.nlmsg_len as usize);
    }
    -1
}

/// Parse a "a.b.c.d/prefix" string into (octets, prefix).
pub fn parse_cidr(s: &str) -> Option<([u8; 4], u8)> {
    let (ip_str, prefix_str) = s.split_once('/')?;
    let prefix: u8 = prefix_str.parse().ok()?;
    let ip = parse_ip(ip_str)?;
    Some((ip, prefix))
}

/// Parse a "a.b.c.d" string into octets.
pub fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    Some([
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
        parts[3].parse().ok()?,
    ])
}

/// Format an IPv4 octet array as "a.b.c.d".
pub fn ip_str(ip: &[u8; 4]) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}
