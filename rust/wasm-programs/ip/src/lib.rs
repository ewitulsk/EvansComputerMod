use ecm_host_abi::netlink::*;
use ecm_net_tools::nl::{find_iface_index, ip_str, nl_dump, parse_cidr, parse_ip};
use ecm_net_tools::NetTools;

pub fn run(args: &[String], nt: &mut dyn NetTools) -> i32 {
    if args.is_empty() {
        nt.err("Usage: ip addr | ip route | ip link\n");
        return 1;
    }

    match args[0].as_str() {
        "addr" | "address" => cmd_ip_addr(nt, &args[1..]),
        "route" => cmd_ip_route(nt, &args[1..]),
        "link" => cmd_ip_link(nt, &args[1..]),
        _ => {
            nt.err("Usage: ip addr | ip route | ip link\n");
            1
        }
    }
}

fn cmd_ip_addr(nt: &mut dyn NetTools, args: &[String]) -> i32 {
    if args.is_empty() || args[0] == "show" {
        let links = nl_dump(nt, RTM_GETLINK, IFINFOMSG_SIZE);
        let addrs = nl_dump(nt, RTM_GETADDR, IFADDRMSG_SIZE);

        let mut off = 0;
        while off < links.len() {
            let hdr = match NlMsgHdr::parse(&links[off..]) {
                Some(h) => h,
                None => break,
            };
            if hdr.nlmsg_type == NLMSG_DONE {
                break;
            }
            if hdr.nlmsg_type != RTM_NEWLINK {
                off += nlmsg_align(hdr.nlmsg_len as usize);
                continue;
            }

            let payload = &links[off + NLMSG_HDR_SIZE..off + hdr.nlmsg_len as usize];
            let ifinfo = IfInfoMsg::parse(payload).unwrap_or_default();

            let mut name = String::new();
            let mut mac = String::new();
            let mut attr_off = IFINFOMSG_SIZE;
            while let Some((atype, adata, next)) = parse_attr(payload, attr_off) {
                match atype {
                    IFLA_IFNAME => {
                        let end = adata.iter().position(|&b| b == 0).unwrap_or(adata.len());
                        name = String::from_utf8_lossy(&adata[..end]).to_string();
                    }
                    IFLA_ADDRESS if adata.len() >= 6 => {
                        mac = format!(
                            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                            adata[0], adata[1], adata[2], adata[3], adata[4], adata[5]
                        );
                    }
                    _ => {}
                }
                attr_off = next;
            }

            let flags = if (ifinfo.ifi_flags & IFF_UP) != 0 {
                "UP"
            } else {
                "DOWN"
            };
            nt.out(&format!(
                "{}: {} flags=<{}>  mtu 1500\n",
                ifinfo.ifi_index, name, flags
            ));
            nt.out(&format!("    link/ether {}\n", mac));

            let mut addr_off = 0;
            while addr_off < addrs.len() {
                let ahdr = match NlMsgHdr::parse(&addrs[addr_off..]) {
                    Some(h) => h,
                    None => break,
                };
                if ahdr.nlmsg_type == NLMSG_DONE {
                    break;
                }
                if ahdr.nlmsg_type == RTM_NEWADDR {
                    let ap =
                        &addrs[addr_off + NLMSG_HDR_SIZE..addr_off + ahdr.nlmsg_len as usize];
                    let ifa = IfAddrMsg::parse(ap).unwrap_or_default();
                    if ifa.ifa_index == ifinfo.ifi_index as u32 {
                        let mut aattr_off = IFADDRMSG_SIZE;
                        while let Some((atype, adata, next)) = parse_attr(ap, aattr_off) {
                            if atype == IFA_LOCAL && adata.len() >= 4 {
                                nt.out(&format!(
                                    "    inet {}.{}.{}.{}/{}\n",
                                    adata[0], adata[1], adata[2], adata[3], ifa.ifa_prefixlen
                                ));
                            }
                            aattr_off = next;
                        }
                    }
                }
                addr_off += nlmsg_align(ahdr.nlmsg_len as usize);
            }
            off += nlmsg_align(hdr.nlmsg_len as usize);
        }
        0
    } else if args[0] == "add" {
        if args.len() < 4 || args[2] != "dev" {
            nt.err("Usage: ip addr add <ip>/<prefix> dev <iface>\n");
            return 1;
        }
        if let Some((ip, prefix)) = parse_cidr(&args[1]) {
            let idx = find_iface_index(nt, &args[3]);
            if idx < 0 {
                nt.err("Unknown interface.\n");
                return 1;
            }
            nl_add_addr(nt, idx, &ip, prefix);
            nt.out(&format!("Added {}/{} to {}\n", ip_str(&ip), prefix, args[3]));
            0
        } else {
            nt.err("Invalid CIDR address.\n");
            1
        }
    } else if args[0] == "del" {
        if args.len() < 4 || args[2] != "dev" {
            nt.err("Usage: ip addr del <ip>/<prefix> dev <iface>\n");
            return 1;
        }
        let idx = find_iface_index(nt, &args[3]);
        if idx < 0 {
            nt.err("Unknown interface.\n");
            return 1;
        }
        nl_del_addr(nt, idx);
        nt.out(&format!("Removed address from {}\n", args[3]));
        0
    } else {
        nt.err("Usage: ip addr [show|add|del]\n");
        1
    }
}

fn cmd_ip_route(nt: &mut dyn NetTools, args: &[String]) -> i32 {
    if args.is_empty() || args[0] == "show" {
        let routes = nl_dump(nt, RTM_GETROUTE, RTMSG_SIZE);
        let links = nl_dump(nt, RTM_GETLINK, IFINFOMSG_SIZE);
        let mut found = false;
        let mut off = 0;
        while off < routes.len() {
            let hdr = match NlMsgHdr::parse(&routes[off..]) {
                Some(h) => h,
                None => break,
            };
            if hdr.nlmsg_type == NLMSG_DONE {
                break;
            }
            if hdr.nlmsg_type != RTM_NEWROUTE {
                off += nlmsg_align(hdr.nlmsg_len as usize);
                continue;
            }
            found = true;

            let payload = &routes[off + NLMSG_HDR_SIZE..off + hdr.nlmsg_len as usize];
            let rtm = RtMsg::parse(payload).unwrap_or_default();

            let mut dest = [0u8; 4];
            let mut gw = [0u8; 4];
            let mut oif: u32 = 0;
            let mut attr_off = RTMSG_SIZE;
            while let Some((atype, adata, next)) = parse_attr(payload, attr_off) {
                match atype {
                    RTA_DST if adata.len() >= 4 => dest.copy_from_slice(&adata[..4]),
                    RTA_GATEWAY if adata.len() >= 4 => gw.copy_from_slice(&adata[..4]),
                    RTA_OIF if adata.len() >= 4 => {
                        oif = u32::from_le_bytes([adata[0], adata[1], adata[2], adata[3]])
                    }
                    _ => {}
                }
                attr_off = next;
            }

            let iface_name = get_iface_name_by_index(&links, oif as i32);
            if rtm.rtm_dst_len == 0 {
                nt.out(&format!("default via {} dev {}\n", ip_str(&gw), iface_name));
            } else if gw == [0, 0, 0, 0] {
                nt.out(&format!(
                    "{}/{} dev {} scope link\n",
                    ip_str(&dest),
                    rtm.rtm_dst_len,
                    iface_name
                ));
            } else {
                nt.out(&format!(
                    "{}/{} via {} dev {}\n",
                    ip_str(&dest),
                    rtm.rtm_dst_len,
                    ip_str(&gw),
                    iface_name
                ));
            }
            off += nlmsg_align(hdr.nlmsg_len as usize);
        }
        if !found {
            nt.out("No routes configured.\n");
        }
        0
    } else if args[0] == "add" {
        if args.len() < 2 {
            nt.err("Usage: ip route add <dest>/<prefix>|default via <gw> dev <iface>\n");
            return 1;
        }
        let dest_str = &args[1];
        let mut gw_str = "0.0.0.0".to_string();
        let mut dev = String::new();
        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "via" if i + 1 < args.len() => {
                    gw_str = args[i + 1].clone();
                    i += 2;
                }
                "dev" if i + 1 < args.len() => {
                    dev = args[i + 1].clone();
                    i += 2;
                }
                _ => {
                    i += 1;
                }
            }
        }
        let iface_idx = if !dev.is_empty() {
            find_iface_index(nt, &dev)
        } else {
            1
        };
        if iface_idx < 0 {
            nt.err("Unknown interface.\n");
            return 1;
        }
        let gw = parse_ip(&gw_str).unwrap_or([0, 0, 0, 0]);
        if dest_str == "default" {
            nl_add_route(nt, &[0, 0, 0, 0], 0, &gw, iface_idx);
            nt.out("Default route added.\n");
            0
        } else if let Some((dest, prefix)) = parse_cidr(dest_str) {
            nl_add_route(nt, &dest, prefix, &gw, iface_idx);
            nt.out(&format!("Route {}/{} added.\n", ip_str(&dest), prefix));
            0
        } else {
            nt.err("Invalid route destination.\n");
            1
        }
    } else if args[0] == "del" {
        if args.len() < 2 {
            nt.err("Usage: ip route del <dest>/<prefix>|default\n");
            return 1;
        }
        if args[1] == "default" {
            nl_del_route(nt, &[0, 0, 0, 0], 0);
            nt.out("Default route deleted.\n");
            0
        } else if let Some((dest, prefix)) = parse_cidr(&args[1]) {
            nl_del_route(nt, &dest, prefix);
            nt.out("Route deleted.\n");
            0
        } else {
            nt.err("Invalid route.\n");
            1
        }
    } else {
        nt.err("Usage: ip route [show|add|del]\n");
        1
    }
}

fn cmd_ip_link(nt: &mut dyn NetTools, args: &[String]) -> i32 {
    if args.is_empty() || args[0] == "show" {
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
            if hdr.nlmsg_type != RTM_NEWLINK {
                off += nlmsg_align(hdr.nlmsg_len as usize);
                continue;
            }
            let payload = &links[off + NLMSG_HDR_SIZE..off + hdr.nlmsg_len as usize];
            let ifinfo = IfInfoMsg::parse(payload).unwrap_or_default();
            let mut name = String::new();
            let mut mac = String::new();
            let mut attr_off = IFINFOMSG_SIZE;
            while let Some((atype, adata, next)) = parse_attr(payload, attr_off) {
                match atype {
                    IFLA_IFNAME => {
                        let end = adata.iter().position(|&b| b == 0).unwrap_or(adata.len());
                        name = String::from_utf8_lossy(&adata[..end]).to_string();
                    }
                    IFLA_ADDRESS if adata.len() >= 6 => {
                        mac = format!(
                            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                            adata[0], adata[1], adata[2], adata[3], adata[4], adata[5]
                        );
                    }
                    _ => {}
                }
                attr_off = next;
            }
            let flags = if (ifinfo.ifi_flags & IFF_UP) != 0 {
                "UP"
            } else {
                "DOWN"
            };
            nt.out(&format!("{}: <{}> mtu 1500\n", name, flags));
            nt.out(&format!("    link/ether {}\n", mac));
            off += nlmsg_align(hdr.nlmsg_len as usize);
        }
        0
    } else if args[0] == "set" {
        if args.len() < 3 {
            nt.err("Usage: ip link set <iface> up|down\n");
            return 1;
        }
        let idx = find_iface_index(nt, &args[1]);
        if idx < 0 {
            nt.err("Unknown interface.\n");
            return 1;
        }
        match args[2].as_str() {
            "up" => {
                nl_set_link(nt, idx, true);
                nt.out("Link up.\n");
                0
            }
            "down" => {
                nl_set_link(nt, idx, false);
                nt.out("Link down.\n");
                0
            }
            _ => {
                nt.err("Usage: ip link set <iface> up|down\n");
                1
            }
        }
    } else {
        nt.err("Usage: ip link [show|set]\n");
        1
    }
}

fn get_iface_name_by_index(links: &[u8], target_idx: i32) -> String {
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
            if ifinfo.ifi_index == target_idx {
                let mut attr_off = IFINFOMSG_SIZE;
                while let Some((atype, adata, next)) = parse_attr(payload, attr_off) {
                    if atype == IFLA_IFNAME {
                        let end = adata.iter().position(|&b| b == 0).unwrap_or(adata.len());
                        return String::from_utf8_lossy(&adata[..end]).to_string();
                    }
                    attr_off = next;
                }
            }
        }
        off += nlmsg_align(hdr.nlmsg_len as usize);
    }
    "?".to_string()
}

fn nl_set_link(nt: &mut dyn NetTools, iface_idx: i32, up: bool) {
    let msg_len = NLMSG_HDR_SIZE + IFINFOMSG_SIZE;
    let mut req = vec![0u8; msg_len];
    NlMsgHdr {
        nlmsg_len: msg_len as u32,
        nlmsg_type: RTM_NEWLINK,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK,
        nlmsg_seq: 2,
        nlmsg_pid: 0,
    }
    .serialize(&mut req);
    IfInfoMsg {
        ifi_index: iface_idx,
        ifi_flags: if up { IFF_UP } else { 0 },
        ifi_change: IFF_UP,
        ..Default::default()
    }
    .serialize(&mut req[NLMSG_HDR_SIZE..]);
    let _ = nt.netlink_request(&req);
}

fn nl_add_addr(nt: &mut dyn NetTools, iface_idx: i32, ip: &[u8; 4], prefix: u8) {
    let mut req = vec![0u8; 128];
    let mut off = NLMSG_HDR_SIZE;
    off += IfAddrMsg {
        ifa_family: AF_INET,
        ifa_prefixlen: prefix,
        ifa_index: iface_idx as u32,
        ..Default::default()
    }
    .serialize(&mut req[off..]);
    off += write_attr(&mut req, off, IFA_LOCAL, ip);
    NlMsgHdr {
        nlmsg_len: off as u32,
        nlmsg_type: RTM_NEWADDR,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE,
        nlmsg_seq: 3,
        nlmsg_pid: 0,
    }
    .serialize(&mut req);
    let _ = nt.netlink_request(&req[..off]);
}

fn nl_del_addr(nt: &mut dyn NetTools, iface_idx: i32) {
    let mut req = vec![0u8; 64];
    let mut off = NLMSG_HDR_SIZE;
    off += IfAddrMsg {
        ifa_family: AF_INET,
        ifa_index: iface_idx as u32,
        ..Default::default()
    }
    .serialize(&mut req[off..]);
    NlMsgHdr {
        nlmsg_len: off as u32,
        nlmsg_type: RTM_DELADDR,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK,
        nlmsg_seq: 4,
        nlmsg_pid: 0,
    }
    .serialize(&mut req);
    let _ = nt.netlink_request(&req[..off]);
}

fn nl_add_route(
    nt: &mut dyn NetTools,
    dest: &[u8; 4],
    prefix: u8,
    gw: &[u8; 4],
    iface_idx: i32,
) {
    let mut req = vec![0u8; 128];
    let mut off = NLMSG_HDR_SIZE;
    off += RtMsg {
        rtm_family: AF_INET,
        rtm_dst_len: prefix,
        rtm_table: RT_TABLE_MAIN,
        rtm_protocol: RTPROT_STATIC,
        rtm_scope: if *gw == [0, 0, 0, 0] {
            RT_SCOPE_LINK
        } else {
            RT_SCOPE_UNIVERSE
        },
        rtm_type: RTN_UNICAST,
        ..Default::default()
    }
    .serialize(&mut req[off..]);
    if prefix > 0 {
        off += write_attr(&mut req, off, RTA_DST, dest);
    }
    if *gw != [0, 0, 0, 0] {
        off += write_attr(&mut req, off, RTA_GATEWAY, gw);
    }
    off += write_attr_u32(&mut req, off, RTA_OIF, iface_idx as u32);
    NlMsgHdr {
        nlmsg_len: off as u32,
        nlmsg_type: RTM_NEWROUTE,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE,
        nlmsg_seq: 5,
        nlmsg_pid: 0,
    }
    .serialize(&mut req);
    let _ = nt.netlink_request(&req[..off]);
}

fn nl_del_route(nt: &mut dyn NetTools, dest: &[u8; 4], prefix: u8) {
    let mut req = vec![0u8; 64];
    let mut off = NLMSG_HDR_SIZE;
    off += RtMsg {
        rtm_family: AF_INET,
        rtm_dst_len: prefix,
        ..Default::default()
    }
    .serialize(&mut req[off..]);
    if prefix > 0 {
        off += write_attr(&mut req, off, RTA_DST, dest);
    }
    NlMsgHdr {
        nlmsg_len: off as u32,
        nlmsg_type: RTM_DELROUTE,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK,
        nlmsg_seq: 6,
        nlmsg_pid: 0,
    }
    .serialize(&mut req);
    let _ = nt.netlink_request(&req[..off]);
}
