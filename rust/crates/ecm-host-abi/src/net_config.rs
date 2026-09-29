//! High-level network configuration API for WASI programs.
//!
//! Wraps netlink/socket operations into simple function calls
//! for interface management, routing, DNS, and ping.
//! Currently returns stub values — networking from Python requires
//! reimplementation via the netlink socket API.

extern crate alloc;
use alloc::string::String;

use crate::socket::{self, SockAddrIn, AF_NETLINK, AF_INET, SOCK_DGRAM, SOCK_RAW, NETLINK_ROUTE, IPPROTO_ICMP};
use crate::netlink::*;

/// Return the number of network interfaces.
pub fn iface_count() -> i32 {
    let fd = socket::socket(AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE);
    if fd < 0 { return 0; }
    let links = nl_dump(fd, RTM_GETLINK, IFINFOMSG_SIZE);
    socket::close(fd);

    let mut count = 0i32;
    let mut off = 0;
    while off < links.len() {
        let hdr = match NlMsgHdr::parse(&links[off..]) {
            Some(h) => h,
            None => break,
        };
        if hdr.nlmsg_type == NLMSG_DONE { break; }
        if hdr.nlmsg_type == RTM_NEWLINK { count += 1; }
        off += nlmsg_align(hdr.nlmsg_len as usize);
    }
    count
}

/// Get interface info as a JSON string.
pub fn iface_info(idx: i32) -> Option<String> {
    let fd = socket::socket(AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE);
    if fd < 0 { return None; }
    let links = nl_dump(fd, RTM_GETLINK, IFINFOMSG_SIZE);
    let addrs = nl_dump(fd, RTM_GETADDR, IFADDRMSG_SIZE);
    socket::close(fd);

    let mut iface_num = 0i32;
    let mut off = 0;
    while off < links.len() {
        let hdr = match NlMsgHdr::parse(&links[off..]) {
            Some(h) => h,
            None => break,
        };
        if hdr.nlmsg_type == NLMSG_DONE { break; }
        if hdr.nlmsg_type == RTM_NEWLINK {
            if iface_num == idx {
                let payload = &links[off + NLMSG_HDR_SIZE..off + hdr.nlmsg_len as usize];
                let ifinfo = IfInfoMsg::parse(payload).unwrap_or_default();
                let mut name = String::new();
                let mut mac = String::new();
                let mut attr_off = IFINFOMSG_SIZE;
                while let Some((atype, adata, next)) = parse_attr(payload, attr_off) {
                    match atype {
                        IFLA_IFNAME => {
                            let end = adata.iter().position(|&b| b == 0).unwrap_or(adata.len());
                            if let Ok(s) = core::str::from_utf8(&adata[..end]) {
                                name = String::from(s);
                            }
                        }
                        IFLA_ADDRESS if adata.len() >= 6 => {
                            mac = alloc::format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                                adata[0], adata[1], adata[2], adata[3], adata[4], adata[5]);
                        }
                        _ => {}
                    }
                    attr_off = next;
                }

                let link_up = (ifinfo.ifi_flags & IFF_UP) != 0;

                // Find IP address for this interface
                let mut ip_str = String::from("0.0.0.0");
                let mut prefix_len: u8 = 0;
                let mut aoff = 0;
                while aoff < addrs.len() {
                    let ahdr = match NlMsgHdr::parse(&addrs[aoff..]) {
                        Some(h) => h,
                        None => break,
                    };
                    if ahdr.nlmsg_type == NLMSG_DONE { break; }
                    if ahdr.nlmsg_type == RTM_NEWADDR {
                        let apayload = &addrs[aoff + NLMSG_HDR_SIZE..aoff + ahdr.nlmsg_len as usize];
                        let ifa = IfAddrMsg::parse(apayload).unwrap_or_default();
                        if ifa.ifa_index == ifinfo.ifi_index as u32 {
                            prefix_len = ifa.ifa_prefixlen;
                            let mut aaoff = IFADDRMSG_SIZE;
                            while let Some((atype, adata, next)) = parse_attr(apayload, aaoff) {
                                if atype == IFA_LOCAL && adata.len() >= 4 {
                                    ip_str = alloc::format!("{}.{}.{}.{}", adata[0], adata[1], adata[2], adata[3]);
                                }
                                aaoff = next;
                            }
                        }
                    }
                    aoff += nlmsg_align(ahdr.nlmsg_len as usize);
                }

                let json = alloc::format!(
                    r#"{{"name":"{}","mac":"{}","ip":"{}","prefix_len":{},"link_up":{}}}"#,
                    name, mac, ip_str, prefix_len, link_up
                );
                return Some(json);
            }
            iface_num += 1;
        }
        off += nlmsg_align(hdr.nlmsg_len as usize);
    }
    None
}

/// Configure interface IP address via netlink.
pub fn iface_configure(idx: i32, ip: &str, prefix: u8) -> i32 {
    let ip_bytes = match parse_ip(ip) {
        Some(b) => b,
        None => return -1,
    };
    let iface_idx = match resolve_iface_index(idx) {
        Some(i) => i,
        None => return -1,
    };

    let fd = socket::socket(AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE);
    if fd < 0 { return -1; }

    let mut req = [0u8; 128];
    let mut off = NLMSG_HDR_SIZE;

    let ifa = IfAddrMsg {
        ifa_family: AF_INET as u8,
        ifa_prefixlen: prefix as u8,
        ifa_flags: 0,
        ifa_scope: 0,
        ifa_index: iface_idx as u32,
    };
    off += ifa.serialize(&mut req[off..]);
    off += write_attr(&mut req, off, IFA_LOCAL, &ip_bytes);

    let hdr = NlMsgHdr {
        nlmsg_len: off as u32,
        nlmsg_type: RTM_NEWADDR,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE,
        nlmsg_seq: 1,
        nlmsg_pid: 0,
    };
    hdr.serialize(&mut req);

    let nl_addr = SockAddrIn::default();
    socket::sendto(fd, &req[..off], 0, &nl_addr);

    let mut buf = [0u8; 64];
    let mut from = SockAddrIn::default();
    socket::recvfrom(fd, &mut buf, 0, &mut from);
    socket::close(fd);
    0
}

/// Set link up/down via netlink.
pub fn iface_set_link(idx: i32, up: bool) {
    let iface_idx = match resolve_iface_index(idx) {
        Some(i) => i,
        None => return,
    };

    let fd = socket::socket(AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE);
    if fd < 0 { return; }

    let msg_len = NLMSG_HDR_SIZE + IFINFOMSG_SIZE;
    let mut req = [0u8; 64];
    let hdr = NlMsgHdr {
        nlmsg_len: msg_len as u32,
        nlmsg_type: RTM_NEWLINK,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK,
        nlmsg_seq: 1,
        nlmsg_pid: 0,
    };
    hdr.serialize(&mut req);
    let ifinfo = IfInfoMsg {
        ifi_family: 0,
        _pad: 0,
        ifi_type: 0,
        ifi_index: iface_idx,
        ifi_flags: if up { IFF_UP } else { 0 },
        ifi_change: IFF_UP,
    };
    ifinfo.serialize(&mut req[NLMSG_HDR_SIZE..]);

    let nl_addr = SockAddrIn::default();
    socket::sendto(fd, &req[..msg_len], 0, &nl_addr);

    let mut buf = [0u8; 64];
    let mut from = SockAddrIn::default();
    socket::recvfrom(fd, &mut buf, 0, &mut from);
    socket::close(fd);
}

/// List routes as JSON string.
pub fn route_list() -> Option<String> {
    let fd = socket::socket(AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE);
    if fd < 0 { return None; }
    let routes = nl_dump(fd, RTM_GETROUTE, RTMSG_SIZE);
    socket::close(fd);

    let mut entries = alloc::vec::Vec::new();
    let mut off = 0;
    while off < routes.len() {
        let hdr = match NlMsgHdr::parse(&routes[off..]) {
            Some(h) => h,
            None => break,
        };
        if hdr.nlmsg_type == NLMSG_DONE { break; }
        if hdr.nlmsg_type == RTM_NEWROUTE {
            let payload = &routes[off + NLMSG_HDR_SIZE..off + hdr.nlmsg_len as usize];
            let rtm = RtMsg::parse(payload).unwrap_or_default();

            let mut dest = String::from("0.0.0.0");
            let mut gateway = String::from("0.0.0.0");
            let mut oif: i32 = -1;
            let mut attr_off = RTMSG_SIZE;
            while let Some((atype, adata, next)) = parse_attr(payload, attr_off) {
                match atype {
                    RTA_DST if adata.len() >= 4 => {
                        dest = alloc::format!("{}.{}.{}.{}", adata[0], adata[1], adata[2], adata[3]);
                    }
                    RTA_GATEWAY if adata.len() >= 4 => {
                        gateway = alloc::format!("{}.{}.{}.{}", adata[0], adata[1], adata[2], adata[3]);
                    }
                    RTA_OIF if adata.len() >= 4 => {
                        oif = i32::from_le_bytes([adata[0], adata[1], adata[2], adata[3]]);
                    }
                    _ => {}
                }
                attr_off = next;
            }
            entries.push(alloc::format!(
                r#"{{"dest":"{}","prefix_len":{},"gateway":"{}","iface_idx":{}}}"#,
                dest, rtm.rtm_dst_len, gateway, oif
            ));
        }
        off += nlmsg_align(hdr.nlmsg_len as usize);
    }

    let mut json = String::from("[");
    for (i, e) in entries.iter().enumerate() {
        if i > 0 { json.push(','); }
        json.push_str(e);
    }
    json.push(']');
    Some(json)
}

/// Add a route via netlink.
pub fn route_add(dest: &str, prefix: u8, gw: &str, iface_idx: i32) -> i32 {
    let dest_bytes = match parse_ip(dest) {
        Some(b) => b,
        None => return -1,
    };
    let gw_bytes = match parse_ip(gw) {
        Some(b) => b,
        None => return -1,
    };
    let nl_iface = match resolve_iface_index(iface_idx) {
        Some(i) => i,
        None => return -1,
    };

    let fd = socket::socket(AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE);
    if fd < 0 { return -1; }

    let mut req = [0u8; 128];
    let mut off = NLMSG_HDR_SIZE;

    let rtm = RtMsg {
        rtm_family: AF_INET as u8,
        rtm_dst_len: prefix as u8,
        rtm_src_len: 0,
        rtm_tos: 0,
        rtm_table: RT_TABLE_MAIN,
        rtm_protocol: RTPROT_STATIC,
        rtm_scope: if gw == "0.0.0.0" { RT_SCOPE_LINK } else { RT_SCOPE_UNIVERSE },
        rtm_type: RTN_UNICAST,
        rtm_flags: 0,
    };
    off += rtm.serialize(&mut req[off..]);
    if prefix > 0 {
        off += write_attr(&mut req, off, RTA_DST, &dest_bytes);
    }
    if gw != "0.0.0.0" {
        off += write_attr(&mut req, off, RTA_GATEWAY, &gw_bytes);
    }
    off += write_attr_u32(&mut req, off, RTA_OIF, nl_iface as u32);

    let hdr = NlMsgHdr {
        nlmsg_len: off as u32,
        nlmsg_type: RTM_NEWROUTE,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE,
        nlmsg_seq: 1,
        nlmsg_pid: 0,
    };
    hdr.serialize(&mut req);

    let nl_addr = SockAddrIn::default();
    socket::sendto(fd, &req[..off], 0, &nl_addr);

    let mut buf = [0u8; 64];
    let mut from = SockAddrIn::default();
    socket::recvfrom(fd, &mut buf, 0, &mut from);
    socket::close(fd);
    0
}

/// Delete a route via netlink.
pub fn route_del(dest: &str, prefix: u8) -> i32 {
    let dest_bytes = match parse_ip(dest) {
        Some(b) => b,
        None => return -1,
    };

    let fd = socket::socket(AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE);
    if fd < 0 { return -1; }

    let mut req = [0u8; 128];
    let mut off = NLMSG_HDR_SIZE;

    let rtm = RtMsg {
        rtm_family: AF_INET as u8,
        rtm_dst_len: prefix as u8,
        rtm_src_len: 0,
        rtm_tos: 0,
        rtm_table: RT_TABLE_MAIN,
        rtm_protocol: RTPROT_STATIC,
        rtm_scope: RT_SCOPE_UNIVERSE,
        rtm_type: RTN_UNICAST,
        rtm_flags: 0,
    };
    off += rtm.serialize(&mut req[off..]);
    if prefix > 0 {
        off += write_attr(&mut req, off, RTA_DST, &dest_bytes);
    }

    let hdr = NlMsgHdr {
        nlmsg_len: off as u32,
        nlmsg_type: RTM_DELROUTE,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK,
        nlmsg_seq: 1,
        nlmsg_pid: 0,
    };
    hdr.serialize(&mut req);

    let nl_addr = SockAddrIn::default();
    socket::sendto(fd, &req[..off], 0, &nl_addr);

    let mut buf = [0u8; 64];
    let mut from = SockAddrIn::default();
    socket::recvfrom(fd, &mut buf, 0, &mut from);
    socket::close(fd);
    0
}

/// Set DNS server (stub — uses IPC).
pub fn dns_set(_ip: &str) -> i32 {
    // DNS configuration not yet available via netlink in WASI
    -1
}

/// Get DNS server (stub).
pub fn dns_get() -> Option<String> {
    None
}

/// Send ICMP ping and return RTT in ms.
pub fn ping(target: &str, timeout_ms: i32) -> i32 {
    let dst = match parse_ip(target) {
        Some(b) => b,
        None => return -1,
    };

    let fd = socket::socket(AF_INET, SOCK_RAW, IPPROTO_ICMP);
    if fd < 0 { return -1; }

    // Set receive timeout
    let timeout_val = [timeout_ms, 0i32];
    let timeout_bytes = unsafe {
        core::slice::from_raw_parts(timeout_val.as_ptr() as *const u8, 8)
    };
    socket::setsockopt(fd, crate::socket::SOL_SOCKET, crate::socket::SO_RCVTIMEO, timeout_bytes);

    // Build ICMP echo request
    let mut pkt = [0u8; 64];
    pkt[0] = 8; // ICMP echo request
    pkt[1] = 0; // code
    // id = 1, seq = 1
    pkt[4] = 0; pkt[5] = 1; // id
    pkt[6] = 0; pkt[7] = 1; // seq
    // Compute checksum
    let cksum = icmp_checksum(&pkt[..8]);
    pkt[2] = (cksum >> 8) as u8;
    pkt[3] = (cksum & 0xff) as u8;

    let addr = SockAddrIn::new(dst[0], dst[1], dst[2], dst[3], 0);

    let start = unsafe { get_time_ms() };
    socket::sendto(fd, &pkt[..8], 0, &addr);

    let mut buf = [0u8; 128];
    let mut from = SockAddrIn::default();
    let n = socket::recvfrom(fd, &mut buf, 0, &mut from);
    socket::close(fd);

    if n > 0 {
        let rtt = (unsafe { get_time_ms() } - start) as i32;
        rtt
    } else {
        -1
    }
}

// --- Internal helpers ---

fn nl_dump(fd: i32, msg_type: u16, payload_size: usize) -> alloc::vec::Vec<u8> {
    let msg_len = NLMSG_HDR_SIZE + payload_size;
    let mut req = alloc::vec![0u8; msg_len];
    let hdr = NlMsgHdr {
        nlmsg_len: msg_len as u32,
        nlmsg_type: msg_type,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_DUMP,
        nlmsg_seq: 1,
        nlmsg_pid: 0,
    };
    hdr.serialize(&mut req);

    let nl_addr = SockAddrIn::default();
    socket::sendto(fd, &req, 0, &nl_addr);

    let mut response = alloc::vec::Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let mut from = SockAddrIn::default();
        let n = socket::recvfrom(fd, &mut buf, 0, &mut from);
        if n <= 0 { break; }
        response.extend_from_slice(&buf[..n as usize]);
        // Check for NLMSG_DONE
        let mut off = 0;
        while off + NLMSG_HDR_SIZE <= response.len() {
            if let Some(h) = NlMsgHdr::parse(&response[off..]) {
                if h.nlmsg_type == NLMSG_DONE { return response; }
                off += nlmsg_align(h.nlmsg_len as usize);
            } else {
                break;
            }
        }
    }
    response
}

/// Resolve sequential index (0, 1, 2...) to netlink interface index.
fn resolve_iface_index(idx: i32) -> Option<i32> {
    let fd = socket::socket(AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE);
    if fd < 0 { return None; }
    let links = nl_dump(fd, RTM_GETLINK, IFINFOMSG_SIZE);
    socket::close(fd);

    let mut num = 0i32;
    let mut off = 0;
    while off < links.len() {
        let hdr = match NlMsgHdr::parse(&links[off..]) {
            Some(h) => h,
            None => break,
        };
        if hdr.nlmsg_type == NLMSG_DONE { break; }
        if hdr.nlmsg_type == RTM_NEWLINK {
            if num == idx {
                let payload = &links[off + NLMSG_HDR_SIZE..off + hdr.nlmsg_len as usize];
                let ifinfo = IfInfoMsg::parse(payload).unwrap_or_default();
                return Some(ifinfo.ifi_index);
            }
            num += 1;
        }
        off += nlmsg_align(hdr.nlmsg_len as usize);
    }
    None
}

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let parts: alloc::vec::Vec<&str> = s.split('.').collect();
    if parts.len() != 4 { return None; }
    let a: u8 = parts[0].parse().ok()?;
    let b: u8 = parts[1].parse().ok()?;
    let c: u8 = parts[2].parse().ok()?;
    let d: u8 = parts[3].parse().ok()?;
    Some([a, b, c, d])
}

fn icmp_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

extern "C" {
    fn get_time_ms() -> i64;
}
