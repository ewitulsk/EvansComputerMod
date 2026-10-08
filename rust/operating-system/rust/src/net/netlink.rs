//! rtnetlink subset used by `ifconfig` / `ip`: GET/NEW LINK, GET/NEW/DEL
//! ADDR, GET/NEW/DEL ROUTE, plus the private RTM_ECM_DHCP that drives the
//! kernel DHCP client (`dhclient`, `ifconfig <if> dhcp`). Message formats
//! are unchanged from the previous kernel; every index and length coming
//! from the program is validated.

use ecm_host_abi::netlink::*;
use ecm_net::types::Ipv4Addr;
use ecm_net::Stack;

pub struct Reply {
    pub bytes: Vec<u8>,
    /// True if the configuration changed (caller persists it).
    pub changed: bool,
    /// Administrative link changes the caller must apply to the host NIC
    /// (interface index, up).
    pub admin: Vec<(usize, bool)>,
}

fn reply(bytes: Vec<u8>) -> Reply {
    Reply {
        bytes,
        changed: false,
        admin: Vec::new(),
    }
}

pub fn handle(stack: &mut Stack, data: &[u8], now_ms: i64) -> Reply {
    let Some(hdr) = NlMsgHdr::parse(data) else {
        return reply(error(0, 0, -22));
    };
    let payload = data.get(NLMSG_HDR_SIZE..).unwrap_or(&[]);
    let (seq, pid) = (hdr.nlmsg_seq, hdr.nlmsg_pid);
    match hdr.nlmsg_type {
        RTM_GETLINK => reply(get_link(stack, seq, pid)),
        RTM_NEWLINK => match new_link(stack, payload, now_ms) {
            Ok(change) => Reply {
                bytes: error(seq, pid, 0),
                changed: true,
                admin: change.into_iter().collect(),
            },
            Err(e) => reply(error(seq, pid, e)),
        },
        RTM_GETADDR => reply(get_addr(stack, seq, pid)),
        RTM_NEWADDR => mutation(new_addr(stack, payload, now_ms), seq, pid),
        RTM_DELADDR => mutation(del_addr(stack, payload), seq, pid),
        RTM_GETROUTE => reply(get_route(stack, seq, pid)),
        RTM_NEWROUTE => mutation(new_route(stack, payload), seq, pid),
        RTM_DELROUTE => mutation(del_route(stack, payload), seq, pid),
        RTM_ECM_DHCP => dhcp(stack, payload, seq, pid, now_ms),
        _ => reply(error(seq, pid, -95)),
    }
}

fn mutation(result: Result<(), i32>, seq: u32, pid: u32) -> Reply {
    match result {
        Ok(()) => Reply {
            bytes: error(seq, pid, 0),
            changed: true,
            admin: Vec::new(),
        },
        Err(e) => reply(error(seq, pid, e)),
    }
}

fn error(seq: u32, pid: u32, err: i32) -> Vec<u8> {
    let mut buf = vec![0u8; NLMSG_HDR_SIZE + 4];
    NlMsgHdr {
        nlmsg_len: (NLMSG_HDR_SIZE + 4) as u32,
        nlmsg_type: NLMSG_ERROR,
        nlmsg_flags: 0,
        nlmsg_seq: seq,
        nlmsg_pid: pid,
    }
    .serialize(&mut buf);
    buf[NLMSG_HDR_SIZE..].copy_from_slice(&err.to_le_bytes());
    buf
}

fn done(seq: u32, pid: u32) -> Vec<u8> {
    let mut buf = vec![0u8; NLMSG_HDR_SIZE];
    NlMsgHdr {
        nlmsg_len: NLMSG_HDR_SIZE as u32,
        nlmsg_type: NLMSG_DONE,
        nlmsg_flags: NLM_F_MULTI,
        nlmsg_seq: seq,
        nlmsg_pid: pid,
    }
    .serialize(&mut buf);
    buf
}

fn finish(msg: &mut [u8], len: usize, kind: u16, seq: u32, pid: u32, out: &mut Vec<u8>) {
    NlMsgHdr {
        nlmsg_len: len as u32,
        nlmsg_type: kind,
        nlmsg_flags: NLM_F_MULTI,
        nlmsg_seq: seq,
        nlmsg_pid: pid,
    }
    .serialize(msg);
    out.extend_from_slice(&msg[..len]);
}

fn name_attr(name: &str) -> Vec<u8> {
    let b = name.as_bytes();
    let mut v = b[..b.len().min(15)].to_vec();
    v.push(0);
    v
}

/// Netlink interface indices are 1-based; validate against the stack.
fn iface_index(stack: &Stack, one_based: i64) -> Result<usize, i32> {
    if one_based < 1 {
        return Err(-19);
    }
    let idx = (one_based - 1) as usize;
    if stack.iface(idx).is_some() {
        Ok(idx)
    } else {
        Err(-19)
    }
}

fn get_link(stack: &Stack, seq: u32, pid: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for idx in 0..stack.iface_count() {
        let Some(iface) = stack.iface(idx) else {
            continue;
        };
        let mut msg = [0u8; 256];
        let mut off = NLMSG_HDR_SIZE;
        let mut flags = 0;
        if iface.admin_up {
            flags |= IFF_UP;
        }
        if iface.admin_up && iface.link_up {
            flags |= IFF_RUNNING;
        }
        let info = IfInfoMsg {
            ifi_family: 0,
            _pad: 0,
            ifi_type: 1,
            ifi_index: idx as i32 + 1,
            ifi_flags: flags,
            ifi_change: 0xFFFF_FFFF,
        };
        off += info.serialize(&mut msg[off..]);
        off += write_attr(&mut msg, off, IFLA_IFNAME, &name_attr(&iface.name));
        off += write_attr(&mut msg, off, IFLA_ADDRESS, &iface.mac.0);
        off += write_attr_u32(&mut msg, off, IFLA_MTU, 1500);
        if let Some(vid) = iface.vlan {
            off += write_attr_u32(&mut msg, off, IFLA_ECM_VLAN, vid as u32);
        }
        finish(&mut msg, off, RTM_NEWLINK, seq, pid, &mut out);
    }
    out.extend_from_slice(&done(seq, pid));
    out
}

/// RTM_NEWLINK: admin up/down (only if IFF_UP is in `ifi_change`) and the
/// private IFLA_ECM_VLAN attribute. Returns the admin change, if any.
fn new_link(stack: &mut Stack, payload: &[u8], now_ms: i64) -> Result<Option<(usize, bool)>, i32> {
    let info = IfInfoMsg::parse(payload).ok_or(-22)?;
    let idx = iface_index(stack, info.ifi_index as i64)?;
    for (kind, data) in attrs(payload, IFINFOMSG_SIZE) {
        if kind == IFLA_ECM_VLAN && data.len() >= 4 {
            let vid = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            match vid {
                0 => stack.set_vlan(idx, None),
                1..=4094 => stack.set_vlan(idx, Some(vid as u16)),
                _ => return Err(-22),
            }
        }
    }
    if info.ifi_change & IFF_UP != 0 {
        let up = info.ifi_flags & IFF_UP != 0;
        stack.set_admin_up(idx, up, now_ms);
        return Ok(Some((idx, up)));
    }
    Ok(None)
}

fn dhcp(stack: &mut Stack, payload: &[u8], seq: u32, pid: u32, now: i64) -> Reply {
    let Some(info) = IfInfoMsg::parse(payload) else {
        return reply(error(seq, pid, -22));
    };
    let idx = match iface_index(stack, info.ifi_index as i64) {
        Ok(i) => i,
        Err(e) => return reply(error(seq, pid, e)),
    };
    let op = attrs(payload, IFINFOMSG_SIZE)
        .into_iter()
        .find(|(k, d)| *k == IFLA_ECM_DHCP_OP && d.len() >= 4)
        .map(|(_, d)| u32::from_le_bytes([d[0], d[1], d[2], d[3]]));
    match op {
        Some(DHCP_OP_START) => {
            if !stack.dhcp_enabled(idx) {
                if stack.start_dhcp(idx, now).is_err() {
                    return reply(error(seq, pid, -22));
                }
            }
            mutation(Ok(()), seq, pid)
        }
        Some(DHCP_OP_RELEASE) | Some(DHCP_OP_STOP) => {
            if !stack.dhcp_enabled(idx) {
                return reply(error(seq, pid, -3));
            }
            if op == Some(DHCP_OP_RELEASE) {
                stack.release_dhcp(idx, now);
            } else {
                stack.stop_dhcp(idx);
            }
            mutation(Ok(()), seq, pid)
        }
        Some(DHCP_OP_STATUS) => reply(dhcp_status(stack, idx, seq, pid, now)),
        _ => reply(error(seq, pid, -22)),
    }
}

fn dhcp_status(stack: &Stack, idx: usize, seq: u32, pid: u32, now: i64) -> Vec<u8> {
    use ecm_net::dhcp::State;
    let mut msg = [0u8; 256];
    let mut off = NLMSG_HDR_SIZE;
    let info = IfInfoMsg {
        ifi_family: 0,
        _pad: 0,
        ifi_type: 1,
        ifi_index: idx as i32 + 1,
        ifi_flags: 0,
        ifi_change: 0,
    };
    off += info.serialize(&mut msg[off..]);
    let secs = |at: i64| ((at - now).max(0) / 1000).min(u32::MAX as i64) as u32;
    match stack.dhcp_status(idx) {
        None => off += write_attr_u32(&mut msg, off, DHCPA_STATE, DHCP_STATE_OFF),
        Some(st) => {
            let state = match st.state {
                State::Init => DHCP_STATE_INIT,
                State::Selecting => DHCP_STATE_SELECTING,
                State::Requesting => DHCP_STATE_REQUESTING,
                State::Bound => DHCP_STATE_BOUND,
                State::Renewing => DHCP_STATE_RENEWING,
                State::Rebinding => DHCP_STATE_REBINDING,
            };
            off += write_attr_u32(&mut msg, off, DHCPA_STATE, state);
            if let Some(s) = st.server {
                off += write_attr(&mut msg, off, DHCPA_SERVER, &s.0);
            }
            if let Some(l) = &st.lease {
                off += write_attr(&mut msg, off, DHCPA_ADDRESS, &l.address.0);
                off += write_attr_u32(&mut msg, off, DHCPA_PREFIX, l.prefix as u32);
                if let Some(r) = l.router {
                    off += write_attr(&mut msg, off, DHCPA_ROUTER, &r.0);
                }
                if let Some(d) = l.dns {
                    off += write_attr(&mut msg, off, DHCPA_DNS, &d.0);
                }
                off += write_attr_u32(&mut msg, off, DHCPA_EXPIRES_IN, secs(l.expires));
                off += write_attr_u32(&mut msg, off, DHCPA_RENEW_IN, secs(st.renew_at));
                off += write_attr_u32(&mut msg, off, DHCPA_REBIND_IN, secs(st.rebind_at));
            }
        }
    }
    NlMsgHdr {
        nlmsg_len: off as u32,
        nlmsg_type: RTM_ECM_DHCP,
        nlmsg_flags: 0,
        nlmsg_seq: seq,
        nlmsg_pid: pid,
    }
    .serialize(&mut msg);
    msg[..off].to_vec()
}

fn get_addr(stack: &Stack, seq: u32, pid: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for idx in 0..stack.iface_count() {
        let Some(iface) = stack.iface(idx) else {
            continue;
        };
        if iface.ip == Ipv4Addr::ZERO {
            continue;
        }
        let mut msg = [0u8; 128];
        let mut off = NLMSG_HDR_SIZE;
        let ifa = IfAddrMsg {
            ifa_family: AF_INET,
            ifa_prefixlen: iface.prefix,
            ifa_flags: 0,
            ifa_scope: 0,
            ifa_index: idx as u32 + 1,
        };
        off += ifa.serialize(&mut msg[off..]);
        off += write_attr(&mut msg, off, IFA_ADDRESS, &iface.ip.0);
        off += write_attr(&mut msg, off, IFA_LOCAL, &iface.ip.0);
        off += write_attr(&mut msg, off, IFA_LABEL, &name_attr(&iface.name));
        finish(&mut msg, off, RTM_NEWADDR, seq, pid, &mut out);
    }
    out.extend_from_slice(&done(seq, pid));
    out
}

fn attrs(payload: &[u8], start: usize) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    let mut off = start;
    while let Some((kind, data, next)) = parse_attr(payload, off) {
        out.push((kind, data));
        if next <= off || out.len() > 64 {
            break;
        }
        off = next;
    }
    out
}

fn ip_attr(data: &[u8]) -> Option<Ipv4Addr> {
    (data.len() >= 4).then(|| Ipv4Addr::from_bytes(&data[..4]))
}

fn new_addr(stack: &mut Stack, payload: &[u8], now_ms: i64) -> Result<(), i32> {
    let ifa = IfAddrMsg::parse(payload).ok_or(-22)?;
    let idx = iface_index(stack, ifa.ifa_index as i64)?;
    if ifa.ifa_prefixlen > 32 {
        return Err(-22);
    }
    let ip = attrs(payload, IFADDRMSG_SIZE)
        .into_iter()
        .filter(|(k, _)| *k == IFA_LOCAL || *k == IFA_ADDRESS)
        .find_map(|(_, d)| ip_attr(d))
        .ok_or(-22)?;
    stack.configure_addr(idx, ip, ifa.ifa_prefixlen, now_ms);
    Ok(())
}

fn del_addr(stack: &mut Stack, payload: &[u8]) -> Result<(), i32> {
    let ifa = IfAddrMsg::parse(payload).ok_or(-22)?;
    let idx = iface_index(stack, ifa.ifa_index as i64)?;
    stack.clear_addr(idx);
    Ok(())
}

fn get_route(stack: &Stack, seq: u32, pid: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for r in stack.routes() {
        let mut msg = [0u8; 128];
        let mut off = NLMSG_HDR_SIZE;
        let rtm = RtMsg {
            rtm_family: AF_INET,
            rtm_dst_len: r.prefix,
            rtm_src_len: 0,
            rtm_tos: 0,
            rtm_table: RT_TABLE_MAIN,
            rtm_protocol: r.source as u8,
            rtm_scope: if r.gateway == Ipv4Addr::ZERO {
                RT_SCOPE_LINK
            } else {
                RT_SCOPE_UNIVERSE
            },
            rtm_type: RTN_UNICAST,
            rtm_flags: 0,
        };
        off += rtm.serialize(&mut msg[off..]);
        if r.prefix > 0 {
            off += write_attr(&mut msg, off, RTA_DST, &r.dst.0);
        }
        if r.gateway != Ipv4Addr::ZERO {
            off += write_attr(&mut msg, off, RTA_GATEWAY, &r.gateway.0);
        }
        off += write_attr_u32(&mut msg, off, RTA_OIF, r.iface as u32 + 1);
        finish(&mut msg, off, RTM_NEWROUTE, seq, pid, &mut out);
    }
    out.extend_from_slice(&done(seq, pid));
    out
}

fn new_route(stack: &mut Stack, payload: &[u8]) -> Result<(), i32> {
    let rtm = RtMsg::parse(payload).ok_or(-22)?;
    if rtm.rtm_dst_len > 32 {
        return Err(-22);
    }
    let (mut dst, mut gw, mut oif) = (Ipv4Addr::ZERO, Ipv4Addr::ZERO, None);
    for (kind, data) in attrs(payload, RTMSG_SIZE) {
        match kind {
            RTA_DST => dst = ip_attr(data).unwrap_or(dst),
            RTA_GATEWAY => gw = ip_attr(data).unwrap_or(gw),
            RTA_OIF if data.len() >= 4 => {
                oif = Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as i64)
            }
            _ => {}
        }
    }
    let idx = match oif {
        Some(o) if o > 0 => iface_index(stack, o)?,
        // No (or zero) output interface: use the interface whose connected
        // subnet contains the gateway.
        _ => (0..stack.iface_count())
            .find(|&i| {
                stack.iface(i).is_some_and(|f| {
                    f.ip != Ipv4Addr::ZERO && gw.same_subnet_prefix(&f.ip, f.prefix)
                })
            })
            .ok_or(-101)?, // ENETUNREACH
    };
    stack
        .add_route(dst, rtm.rtm_dst_len, gw, idx)
        .map_err(|_| -22)
}

fn del_route(stack: &mut Stack, payload: &[u8]) -> Result<(), i32> {
    let rtm = RtMsg::parse(payload).ok_or(-22)?;
    let dst = attrs(payload, RTMSG_SIZE)
        .into_iter()
        .find(|(k, _)| *k == RTA_DST)
        .and_then(|(_, d)| ip_attr(d))
        .unwrap_or(Ipv4Addr::ZERO);
    stack.del_route(dst, rtm.rtm_dst_len).map_err(|_| -3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecm_net::types::{Ipv4Addr, MacAddr};
    use ecm_net::StackConfig;

    fn newlink(index: i32, flags: u32, change: u32, vlan: Option<u32>) -> Vec<u8> {
        let mut req = vec![0u8; 64];
        let mut off = NLMSG_HDR_SIZE;
        let info = IfInfoMsg {
            ifi_family: 0,
            _pad: 0,
            ifi_type: 0,
            ifi_index: index,
            ifi_flags: flags,
            ifi_change: change,
        };
        off += info.serialize(&mut req[off..]);
        if let Some(v) = vlan {
            off += write_attr_u32(&mut req, off, IFLA_ECM_VLAN, v);
        }
        NlMsgHdr {
            nlmsg_len: off as u32,
            nlmsg_type: RTM_NEWLINK,
            nlmsg_flags: NLM_F_REQUEST,
            nlmsg_seq: 1,
            nlmsg_pid: 0,
        }
        .serialize(&mut req);
        req.truncate(off);
        req
    }

    fn status(r: &Reply) -> i32 {
        i32::from_le_bytes([r.bytes[16], r.bytes[17], r.bytes[18], r.bytes[19]])
    }

    #[test]
    fn vlan_attribute_sets_tagging_without_touching_admin_state() {
        let mut s = Stack::new(StackConfig { seed: 1 });
        s.add_interface("eth0", MacAddr([2, 0, 0, 0, 0, 1]));
        let r = handle(&mut s, &newlink(1, 0, 0, Some(100)), 0);
        assert_eq!(status(&r), 0);
        assert!(
            r.admin.is_empty(),
            "VLAN-only change must not report an admin change"
        );
        assert_eq!(s.iface(0).unwrap().vlan, Some(100));
        assert!(s.iface(0).unwrap().admin_up);
        // The dump reports it back.
        let dump = get_link(&s, 1, 0);
        assert!(dump.windows(2).any(|w| w == IFLA_ECM_VLAN.to_le_bytes()));
        // 0 turns tagging off; out-of-range ids are rejected.
        handle(&mut s, &newlink(1, 0, 0, Some(0)), 0);
        assert_eq!(s.iface(0).unwrap().vlan, None);
        assert_eq!(
            status(&handle(&mut s, &newlink(1, 0, 0, Some(5000)), 0)),
            -22
        );
        // Admin down only when IFF_UP is in ifi_change.
        let r = handle(&mut s, &newlink(1, 0, IFF_UP, None), 0);
        assert_eq!(r.admin, vec![(0, false)]);
        assert!(!s.iface(0).unwrap().admin_up);
    }

    fn dhcp_req(index: i32, op: u32) -> Vec<u8> {
        let mut req = vec![0u8; 64];
        let mut off = NLMSG_HDR_SIZE;
        let info = IfInfoMsg {
            ifi_family: 0,
            _pad: 0,
            ifi_type: 0,
            ifi_index: index,
            ifi_flags: 0,
            ifi_change: 0,
        };
        off += info.serialize(&mut req[off..]);
        off += write_attr_u32(&mut req, off, IFLA_ECM_DHCP_OP, op);
        NlMsgHdr {
            nlmsg_len: off as u32,
            nlmsg_type: RTM_ECM_DHCP,
            nlmsg_flags: NLM_F_REQUEST,
            nlmsg_seq: 1,
            nlmsg_pid: 0,
        }
        .serialize(&mut req);
        req.truncate(off);
        req
    }

    fn dhcp_state(r: &Reply) -> u32 {
        let hdr = NlMsgHdr::parse(&r.bytes).unwrap();
        assert_eq!(hdr.nlmsg_type, RTM_ECM_DHCP);
        let payload = &r.bytes[NLMSG_HDR_SIZE..hdr.nlmsg_len as usize];
        let (_, d) = attrs(payload, IFINFOMSG_SIZE)
            .into_iter()
            .find(|(k, _)| *k == DHCPA_STATE)
            .unwrap();
        u32::from_le_bytes([d[0], d[1], d[2], d[3]])
    }

    #[test]
    fn dhcp_control_starts_reports_and_stops_the_client() {
        let mut s = Stack::new(StackConfig { seed: 1 });
        s.add_interface("eth0", MacAddr([2, 0, 0, 0, 0, 1]));
        s.configure_addr(0, Ipv4Addr::new(10, 0, 0, 5), 24, 0);
        assert_eq!(dhcp_state(&handle(&mut s, &dhcp_req(1, DHCP_OP_STATUS), 0)), DHCP_STATE_OFF);
        // Releasing with no client running is an error (ESRCH).
        assert_eq!(status(&handle(&mut s, &dhcp_req(1, DHCP_OP_RELEASE), 0)), -3);
        let r = handle(&mut s, &dhcp_req(1, DHCP_OP_START), 0);
        assert_eq!(status(&r), 0);
        assert!(r.changed, "dhcp is persisted to network.cfg");
        assert!(s.dhcp_enabled(0));
        assert_eq!(s.iface(0).unwrap().ip, Ipv4Addr::ZERO, "static address replaced");
        assert_eq!(dhcp_state(&handle(&mut s, &dhcp_req(1, DHCP_OP_STATUS), 0)), DHCP_STATE_INIT);
        s.poll(0);
        assert_eq!(dhcp_state(&handle(&mut s, &dhcp_req(1, DHCP_OP_STATUS), 0)), DHCP_STATE_SELECTING);
        // Starting again keeps the running client.
        assert_eq!(status(&handle(&mut s, &dhcp_req(1, DHCP_OP_START), 0)), 0);
        assert_eq!(dhcp_state(&handle(&mut s, &dhcp_req(1, DHCP_OP_STATUS), 0)), DHCP_STATE_SELECTING);
        assert_eq!(status(&handle(&mut s, &dhcp_req(1, DHCP_OP_STOP), 0)), 0);
        assert!(!s.dhcp_enabled(0));
        // Bad interface / missing op.
        assert_eq!(status(&handle(&mut s, &dhcp_req(9, DHCP_OP_START), 0)), -19);
        assert_eq!(status(&handle(&mut s, &dhcp_req(1, 77), 0)), -22);
    }

    #[test]
    fn bad_indices_and_garbage_are_rejected() {
        let mut s = Stack::new(StackConfig { seed: 1 });
        s.add_interface("eth0", MacAddr([2, 0, 0, 0, 0, 1]));
        assert_eq!(
            status(&handle(&mut s, &newlink(0, 0, IFF_UP, None), 0)),
            -19
        );
        assert_eq!(
            status(&handle(&mut s, &newlink(99, 0, IFF_UP, None), 0)),
            -19
        );
        assert_eq!(
            status(&handle(&mut s, &newlink(-5, 0, IFF_UP, None), 0)),
            -19
        );
        for len in 0..40 {
            let junk: Vec<u8> = (0..len).map(|i| (i * 37) as u8).collect();
            handle(&mut s, &junk, 0);
        }
    }
}
