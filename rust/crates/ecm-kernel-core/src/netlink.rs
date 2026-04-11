//! Netlink message handler — processes RTM_GETLINK, RTM_GETADDR, RTM_NEWADDR,
//! RTM_DELADDR, RTM_GETROUTE, RTM_NEWROUTE, RTM_DELROUTE, RTM_NEWLINK messages
//! and returns response data by querying/modifying the kernel's NetStack.

extern crate alloc;
use alloc::vec::Vec;
use ecm_net::{NetStack, types::*};
use ecm_host_abi::netlink::*;

/// Process a netlink message and return response bytes.
pub fn handle_netlink_message(data: &[u8]) -> Vec<u8> {
    let hdr = match NlMsgHdr::parse(data) {
        Some(h) => h,
        None => return build_error(0, 0, -1),
    };

    let payload = &data[NLMSG_HDR_SIZE..];

    match hdr.nlmsg_type {
        RTM_GETLINK => handle_getlink(&hdr),
        RTM_NEWLINK => handle_newlink(&hdr, payload),
        RTM_GETADDR => handle_getaddr(&hdr),
        RTM_NEWADDR => handle_newaddr(&hdr, payload),
        RTM_DELADDR => handle_deladdr(&hdr, payload),
        RTM_GETROUTE => handle_getroute(&hdr),
        RTM_NEWROUTE => handle_newroute(&hdr, payload),
        RTM_DELROUTE => handle_delroute(&hdr, payload),
        _ => build_error(hdr.nlmsg_seq, hdr.nlmsg_pid, -95), // EOPNOTSUPP
    }
}

fn build_error(seq: u32, pid: u32, error: i32) -> Vec<u8> {
    let mut buf = vec![0u8; NLMSG_HDR_SIZE + 4];
    let hdr = NlMsgHdr {
        nlmsg_len: (NLMSG_HDR_SIZE + 4) as u32,
        nlmsg_type: NLMSG_ERROR,
        nlmsg_flags: 0,
        nlmsg_seq: seq,
        nlmsg_pid: pid,
    };
    hdr.serialize(&mut buf);
    buf[NLMSG_HDR_SIZE..NLMSG_HDR_SIZE+4].copy_from_slice(&error.to_le_bytes());
    buf
}

fn build_ack(seq: u32, pid: u32) -> Vec<u8> {
    build_error(seq, pid, 0)
}

fn build_done(seq: u32, pid: u32) -> Vec<u8> {
    let mut buf = vec![0u8; NLMSG_HDR_SIZE];
    let hdr = NlMsgHdr {
        nlmsg_len: NLMSG_HDR_SIZE as u32,
        nlmsg_type: NLMSG_DONE,
        nlmsg_flags: NLM_F_MULTI,
        nlmsg_seq: seq,
        nlmsg_pid: pid,
    };
    hdr.serialize(&mut buf);
    buf
}

// --- RTM_GETLINK: dump all interfaces ---
fn handle_getlink(req: &NlMsgHdr) -> Vec<u8> {
    let stack = match NetStack::get() {
        Some(s) => s,
        None => return build_error(req.nlmsg_seq, req.nlmsg_pid, -1),
    };

    let mut response = Vec::new();

    for i in 0..stack.iface_count {
        let iface = &stack.interfaces[i];
        let mut msg_buf = [0u8; 256];
        let mut off = NLMSG_HDR_SIZE;

        // ifinfomsg
        let flags = if iface.link_up { IFF_UP | IFF_RUNNING } else { 0 };
        let ifinfo = IfInfoMsg {
            ifi_family: 0,
            _pad: 0,
            ifi_type: 1, // ARPHRD_ETHER
            ifi_index: i as i32 + 1, // 1-based index (Linux convention)
            ifi_flags: flags,
            ifi_change: 0xFFFFFFFF,
        };
        off += ifinfo.serialize(&mut msg_buf[off..]);

        // IFLA_IFNAME
        let name = iface.name_str();
        let name_bytes = name.as_bytes();
        // Null-terminated
        let mut name_with_null = [0u8; 17];
        let name_len = name_bytes.len().min(16);
        name_with_null[..name_len].copy_from_slice(&name_bytes[..name_len]);
        off += write_attr(&mut msg_buf, off, IFLA_IFNAME, &name_with_null[..name_len + 1]);

        // IFLA_ADDRESS (MAC)
        off += write_attr(&mut msg_buf, off, IFLA_ADDRESS, &iface.mac.0);

        // IFLA_MTU
        off += write_attr_u32(&mut msg_buf, off, IFLA_MTU, 1500);

        // Write nlmsghdr
        let hdr = NlMsgHdr {
            nlmsg_len: off as u32,
            nlmsg_type: RTM_NEWLINK,
            nlmsg_flags: NLM_F_MULTI,
            nlmsg_seq: req.nlmsg_seq,
            nlmsg_pid: req.nlmsg_pid,
        };
        hdr.serialize(&mut msg_buf);

        response.extend_from_slice(&msg_buf[..off]);
    }

    // NLMSG_DONE
    response.extend_from_slice(&build_done(req.nlmsg_seq, req.nlmsg_pid));
    response
}

// --- RTM_NEWLINK: set interface flags (up/down) ---
fn handle_newlink(req: &NlMsgHdr, payload: &[u8]) -> Vec<u8> {
    let ifinfo = match IfInfoMsg::parse(payload) {
        Some(m) => m,
        None => return build_error(req.nlmsg_seq, req.nlmsg_pid, -22),
    };

    let stack = match NetStack::get() {
        Some(s) => s,
        None => return build_error(req.nlmsg_seq, req.nlmsg_pid, -1),
    };

    let idx = (ifinfo.ifi_index - 1) as usize; // Convert to 0-based
    if idx >= stack.iface_count {
        return build_error(req.nlmsg_seq, req.nlmsg_pid, -19); // ENODEV
    }

    let up = (ifinfo.ifi_flags & IFF_UP) != 0;
    stack.set_link_state(idx, up);

    build_ack(req.nlmsg_seq, req.nlmsg_pid)
}

// --- RTM_GETADDR: dump all addresses ---
fn handle_getaddr(req: &NlMsgHdr) -> Vec<u8> {
    let stack = match NetStack::get() {
        Some(s) => s,
        None => return build_error(req.nlmsg_seq, req.nlmsg_pid, -1),
    };

    let mut response = Vec::new();

    for i in 0..stack.iface_count {
        let iface = &stack.interfaces[i];
        if !iface.configured() { continue; }

        let mut msg_buf = [0u8; 128];
        let mut off = NLMSG_HDR_SIZE;

        // ifaddrmsg
        let ifa = IfAddrMsg {
            ifa_family: AF_INET,
            ifa_prefixlen: iface.prefix_len,
            ifa_flags: 0,
            ifa_scope: 0,
            ifa_index: (i + 1) as u32,
        };
        off += ifa.serialize(&mut msg_buf[off..]);

        // IFA_ADDRESS
        off += write_attr(&mut msg_buf, off, IFA_ADDRESS, &iface.ip.0);

        // IFA_LOCAL (same as address for point-to-point)
        off += write_attr(&mut msg_buf, off, IFA_LOCAL, &iface.ip.0);

        // IFA_LABEL (interface name)
        let name = iface.name_str();
        let name_bytes = name.as_bytes();
        let mut name_with_null = [0u8; 17];
        let name_len = name_bytes.len().min(16);
        name_with_null[..name_len].copy_from_slice(&name_bytes[..name_len]);
        off += write_attr(&mut msg_buf, off, IFA_LABEL, &name_with_null[..name_len + 1]);

        // Write nlmsghdr
        let hdr = NlMsgHdr {
            nlmsg_len: off as u32,
            nlmsg_type: RTM_NEWADDR,
            nlmsg_flags: NLM_F_MULTI,
            nlmsg_seq: req.nlmsg_seq,
            nlmsg_pid: req.nlmsg_pid,
        };
        hdr.serialize(&mut msg_buf);

        response.extend_from_slice(&msg_buf[..off]);
    }

    response.extend_from_slice(&build_done(req.nlmsg_seq, req.nlmsg_pid));
    response
}

// --- RTM_NEWADDR: add IP address to interface ---
fn handle_newaddr(req: &NlMsgHdr, payload: &[u8]) -> Vec<u8> {
    let ifa = match IfAddrMsg::parse(payload) {
        Some(m) => m,
        None => return build_error(req.nlmsg_seq, req.nlmsg_pid, -22),
    };

    let stack = match NetStack::get() {
        Some(s) => s,
        None => return build_error(req.nlmsg_seq, req.nlmsg_pid, -1),
    };

    let idx = ifa.ifa_index as usize - 1;
    if idx >= stack.iface_count {
        return build_error(req.nlmsg_seq, req.nlmsg_pid, -19);
    }

    // Parse attributes to find IFA_LOCAL or IFA_ADDRESS
    let attrs_start = IFADDRMSG_SIZE;
    let mut off = attrs_start;
    let mut addr: Option<Ipv4Addr> = None;
    while let Some((attr_type, attr_data, next)) = parse_attr(payload, off) {
        match attr_type {
            IFA_LOCAL | IFA_ADDRESS => {
                if attr_data.len() >= 4 {
                    addr = Some(Ipv4Addr::from_bytes(attr_data));
                }
            }
            _ => {}
        }
        off = next;
    }

    let ip = match addr {
        Some(ip) => ip,
        None => return build_error(req.nlmsg_seq, req.nlmsg_pid, -22),
    };

    stack.interfaces[idx].ip = ip;
    stack.interfaces[idx].prefix_len = ifa.ifa_prefixlen;
    // Add connected route
    let mask = prefix_to_mask(ifa.ifa_prefixlen);
    let network = Ipv4Addr::new(
        ip.0[0] & mask.0[0], ip.0[1] & mask.0[1],
        ip.0[2] & mask.0[2], ip.0[3] & mask.0[3],
    );
    let _ = stack.routing.add_route(network, ifa.ifa_prefixlen, Ipv4Addr::ZERO, idx);

    build_ack(req.nlmsg_seq, req.nlmsg_pid)
}

// --- RTM_DELADDR: remove IP from interface ---
fn handle_deladdr(req: &NlMsgHdr, payload: &[u8]) -> Vec<u8> {
    let ifa = match IfAddrMsg::parse(payload) {
        Some(m) => m,
        None => return build_error(req.nlmsg_seq, req.nlmsg_pid, -22),
    };

    let stack = match NetStack::get() {
        Some(s) => s,
        None => return build_error(req.nlmsg_seq, req.nlmsg_pid, -1),
    };

    let idx = ifa.ifa_index as usize - 1;
    if idx >= stack.iface_count {
        return build_error(req.nlmsg_seq, req.nlmsg_pid, -19);
    }

    stack.interfaces[idx].ip = Ipv4Addr::ZERO;
    stack.interfaces[idx].prefix_len = 0;

    build_ack(req.nlmsg_seq, req.nlmsg_pid)
}

// --- RTM_GETROUTE: dump routing table ---
fn handle_getroute(req: &NlMsgHdr) -> Vec<u8> {
    let stack = match NetStack::get() {
        Some(s) => s,
        None => return build_error(req.nlmsg_seq, req.nlmsg_pid, -1),
    };

    let mut response = Vec::new();

    for entry in stack.routing.entries.iter() {
        if !entry.active { continue; }

        let mut msg_buf = [0u8; 128];
        let mut off = NLMSG_HDR_SIZE;

        let rtm = RtMsg {
            rtm_family: AF_INET,
            rtm_dst_len: entry.prefix_len,
            rtm_src_len: 0,
            rtm_tos: 0,
            rtm_table: RT_TABLE_MAIN,
            rtm_protocol: RTPROT_STATIC,
            rtm_scope: if entry.gateway == Ipv4Addr::ZERO { RT_SCOPE_LINK } else { RT_SCOPE_UNIVERSE },
            rtm_type: RTN_UNICAST,
            rtm_flags: 0,
        };
        off += rtm.serialize(&mut msg_buf[off..]);

        // RTA_DST
        if entry.prefix_len > 0 {
            off += write_attr(&mut msg_buf, off, RTA_DST, &entry.destination.0);
        }

        // RTA_GATEWAY (if not directly connected)
        if entry.gateway != Ipv4Addr::ZERO {
            off += write_attr(&mut msg_buf, off, RTA_GATEWAY, &entry.gateway.0);
        }

        // RTA_OIF (output interface, 1-based)
        off += write_attr_u32(&mut msg_buf, off, RTA_OIF, (entry.iface_index + 1) as u32);

        let hdr = NlMsgHdr {
            nlmsg_len: off as u32,
            nlmsg_type: RTM_NEWROUTE,
            nlmsg_flags: NLM_F_MULTI,
            nlmsg_seq: req.nlmsg_seq,
            nlmsg_pid: req.nlmsg_pid,
        };
        hdr.serialize(&mut msg_buf);

        response.extend_from_slice(&msg_buf[..off]);
    }

    response.extend_from_slice(&build_done(req.nlmsg_seq, req.nlmsg_pid));
    response
}

// --- RTM_NEWROUTE: add route ---
fn handle_newroute(req: &NlMsgHdr, payload: &[u8]) -> Vec<u8> {
    let rtm = match RtMsg::parse(payload) {
        Some(m) => m,
        None => return build_error(req.nlmsg_seq, req.nlmsg_pid, -22),
    };

    let stack = match NetStack::get() {
        Some(s) => s,
        None => return build_error(req.nlmsg_seq, req.nlmsg_pid, -1),
    };

    let mut dest = Ipv4Addr::ZERO;
    let mut gateway = Ipv4Addr::ZERO;
    let mut oif: usize = 0;

    let attrs_start = RTMSG_SIZE;
    let mut off = attrs_start;
    while let Some((attr_type, attr_data, next)) = parse_attr(payload, off) {
        match attr_type {
            RTA_DST => {
                if attr_data.len() >= 4 { dest = Ipv4Addr::from_bytes(attr_data); }
            }
            RTA_GATEWAY => {
                if attr_data.len() >= 4 { gateway = Ipv4Addr::from_bytes(attr_data); }
            }
            RTA_OIF => {
                if attr_data.len() >= 4 {
                    oif = u32::from_le_bytes([attr_data[0], attr_data[1], attr_data[2], attr_data[3]]) as usize;
                    if oif > 0 { oif -= 1; } // Convert from 1-based to 0-based
                }
            }
            _ => {}
        }
        off = next;
    }

    let _ = stack.routing.add_route(dest, rtm.rtm_dst_len, gateway, oif);

    build_ack(req.nlmsg_seq, req.nlmsg_pid)
}

// --- RTM_DELROUTE: delete route ---
fn handle_delroute(req: &NlMsgHdr, payload: &[u8]) -> Vec<u8> {
    let rtm = match RtMsg::parse(payload) {
        Some(m) => m,
        None => return build_error(req.nlmsg_seq, req.nlmsg_pid, -22),
    };

    let stack = match NetStack::get() {
        Some(s) => s,
        None => return build_error(req.nlmsg_seq, req.nlmsg_pid, -1),
    };

    let mut dest = Ipv4Addr::ZERO;
    let attrs_start = RTMSG_SIZE;
    let mut off = attrs_start;
    while let Some((attr_type, attr_data, next)) = parse_attr(payload, off) {
        if attr_type == RTA_DST && attr_data.len() >= 4 {
            dest = Ipv4Addr::from_bytes(attr_data);
        }
        off = next;
    }

    stack.routing.del_route(dest, rtm.rtm_dst_len);

    build_ack(req.nlmsg_seq, req.nlmsg_pid)
}

fn prefix_to_mask(prefix: u8) -> Ipv4Addr {
    if prefix == 0 { return Ipv4Addr::ZERO; }
    if prefix >= 32 { return Ipv4Addr::new(255, 255, 255, 255); }
    let mask = !((1u32 << (32 - prefix)) - 1);
    Ipv4Addr::from_u32(mask)
}
