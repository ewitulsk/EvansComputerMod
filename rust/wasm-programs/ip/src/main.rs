use ecm_host_abi::socket::{self, SockAddrIn, AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE};
use ecm_host_abi::netlink::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: ip addr | ip route | ip link");
        std::process::exit(1);
    }

    let fd = socket::socket(AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE);
    if fd < 0 {
        eprintln!("ip: failed to create netlink socket");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "addr" | "address" => cmd_ip_addr(fd, &args[2..]),
        "route" => cmd_ip_route(fd, &args[2..]),
        "link" => cmd_ip_link(fd, &args[2..]),
        _ => eprintln!("Usage: ip addr | ip route | ip link"),
    }

    socket::close(fd);
}

fn cmd_ip_addr(fd: i32, args: &[String]) {
    if args.is_empty() || args[0] == "show" {
        let links = nl_dump(fd, RTM_GETLINK, IFINFOMSG_SIZE);
        let addrs = nl_dump(fd, RTM_GETADDR, IFADDRMSG_SIZE);

        let mut off = 0;
        while off < links.len() {
            let hdr = match NlMsgHdr::parse(&links[off..]) { Some(h) => h, None => break };
            if hdr.nlmsg_type == NLMSG_DONE { break; }
            if hdr.nlmsg_type != RTM_NEWLINK { off += nlmsg_align(hdr.nlmsg_len as usize); continue; }

            let payload = &links[off + NLMSG_HDR_SIZE..off + hdr.nlmsg_len as usize];
            let ifinfo = IfInfoMsg::parse(payload).unwrap_or_default();

            let mut name = String::new();
            let mut mac = String::new();
            let mut attr_off = IFINFOMSG_SIZE;
            while let Some((atype, adata, next)) = parse_attr(payload, attr_off) {
                match atype {
                    IFLA_IFNAME => { let end = adata.iter().position(|&b| b == 0).unwrap_or(adata.len()); name = String::from_utf8_lossy(&adata[..end]).to_string(); }
                    IFLA_ADDRESS if adata.len() >= 6 => { mac = format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", adata[0], adata[1], adata[2], adata[3], adata[4], adata[5]); }
                    _ => {}
                }
                attr_off = next;
            }

            let flags = if (ifinfo.ifi_flags & IFF_UP) != 0 { "UP" } else { "DOWN" };
            println!("{}: {} flags=<{}>  mtu 1500", ifinfo.ifi_index, name, flags);
            println!("    link/ether {}", mac);

            // Find addresses
            let mut addr_off = 0;
            while addr_off < addrs.len() {
                let ahdr = match NlMsgHdr::parse(&addrs[addr_off..]) { Some(h) => h, None => break };
                if ahdr.nlmsg_type == NLMSG_DONE { break; }
                if ahdr.nlmsg_type == RTM_NEWADDR {
                    let ap = &addrs[addr_off + NLMSG_HDR_SIZE..addr_off + ahdr.nlmsg_len as usize];
                    let ifa = IfAddrMsg::parse(ap).unwrap_or_default();
                    if ifa.ifa_index == ifinfo.ifi_index as u32 {
                        let mut aattr_off = IFADDRMSG_SIZE;
                        while let Some((atype, adata, next)) = parse_attr(ap, aattr_off) {
                            if atype == IFA_LOCAL && adata.len() >= 4 {
                                println!("    inet {}.{}.{}.{}/{}", adata[0], adata[1], adata[2], adata[3], ifa.ifa_prefixlen);
                            }
                            aattr_off = next;
                        }
                    }
                }
                addr_off += nlmsg_align(ahdr.nlmsg_len as usize);
            }
            off += nlmsg_align(hdr.nlmsg_len as usize);
        }
    } else if args[0] == "add" {
        // ip addr add 10.0.0.1/24 dev eth0
        if args.len() < 4 || args[2] != "dev" {
            eprintln!("Usage: ip addr add <ip>/<prefix> dev <iface>");
            return;
        }
        if let Some((ip, prefix)) = parse_cidr(&args[1]) {
            let idx = find_iface_index(fd, &args[3]);
            if idx < 0 { eprintln!("Unknown interface."); return; }
            nl_add_addr(fd, idx, &ip, prefix);
            println!("Added {}/{} to {}", ip_str(&ip), prefix, args[3]);
        } else {
            eprintln!("Invalid CIDR address.");
        }
    } else if args[0] == "del" {
        if args.len() < 4 || args[2] != "dev" {
            eprintln!("Usage: ip addr del <ip>/<prefix> dev <iface>");
            return;
        }
        let idx = find_iface_index(fd, &args[3]);
        if idx < 0 { eprintln!("Unknown interface."); return; }
        nl_del_addr(fd, idx);
        println!("Removed address from {}", args[3]);
    }
}

fn cmd_ip_route(fd: i32, args: &[String]) {
    if args.is_empty() || args[0] == "show" {
        let routes = nl_dump(fd, RTM_GETROUTE, RTMSG_SIZE);
        let links = nl_dump(fd, RTM_GETLINK, IFINFOMSG_SIZE);
        let mut found = false;
        let mut off = 0;
        while off < routes.len() {
            let hdr = match NlMsgHdr::parse(&routes[off..]) { Some(h) => h, None => break };
            if hdr.nlmsg_type == NLMSG_DONE { break; }
            if hdr.nlmsg_type != RTM_NEWROUTE { off += nlmsg_align(hdr.nlmsg_len as usize); continue; }
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
                    RTA_OIF if adata.len() >= 4 => oif = u32::from_le_bytes([adata[0], adata[1], adata[2], adata[3]]),
                    _ => {}
                }
                attr_off = next;
            }

            let iface_name = get_iface_name_by_index(&links, oif as i32);
            if rtm.rtm_dst_len == 0 {
                println!("default via {} dev {}", ip_str(&gw), iface_name);
            } else if gw == [0, 0, 0, 0] {
                println!("{}/{} dev {} scope link", ip_str(&dest), rtm.rtm_dst_len, iface_name);
            } else {
                println!("{}/{} via {} dev {}", ip_str(&dest), rtm.rtm_dst_len, ip_str(&gw), iface_name);
            }
            off += nlmsg_align(hdr.nlmsg_len as usize);
        }
        if !found { println!("No routes configured."); }
    } else if args[0] == "add" {
        if args.len() < 2 { eprintln!("Usage: ip route add <dest>/<prefix>|default via <gw> dev <iface>"); return; }
        let dest_str = &args[1];
        let mut gw_str = "0.0.0.0".to_string();
        let mut dev = String::new();
        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "via" if i + 1 < args.len() => { gw_str = args[i + 1].clone(); i += 2; }
                "dev" if i + 1 < args.len() => { dev = args[i + 1].clone(); i += 2; }
                _ => { i += 1; }
            }
        }
        let iface_idx = if !dev.is_empty() { find_iface_index(fd, &dev) } else { 1 };
        if iface_idx < 0 { eprintln!("Unknown interface."); return; }
        let gw = parse_ip(&gw_str).unwrap_or([0, 0, 0, 0]);
        if dest_str == "default" {
            nl_add_route(fd, &[0, 0, 0, 0], 0, &gw, iface_idx);
            println!("Default route added.");
        } else if let Some((dest, prefix)) = parse_cidr(dest_str) {
            nl_add_route(fd, &dest, prefix, &gw, iface_idx);
            println!("Route {}/{} added.", ip_str(&dest), prefix);
        } else { eprintln!("Invalid route destination."); }
    } else if args[0] == "del" {
        if args.len() < 2 { eprintln!("Usage: ip route del <dest>/<prefix>|default"); return; }
        if args[1] == "default" {
            nl_del_route(fd, &[0, 0, 0, 0], 0);
            println!("Default route deleted.");
        } else if let Some((dest, prefix)) = parse_cidr(&args[1]) {
            nl_del_route(fd, &dest, prefix);
            println!("Route deleted.");
        } else { eprintln!("Invalid route."); }
    }
}

fn cmd_ip_link(fd: i32, args: &[String]) {
    if args.is_empty() || args[0] == "show" {
        let links = nl_dump(fd, RTM_GETLINK, IFINFOMSG_SIZE);
        let mut off = 0;
        while off < links.len() {
            let hdr = match NlMsgHdr::parse(&links[off..]) { Some(h) => h, None => break };
            if hdr.nlmsg_type == NLMSG_DONE { break; }
            if hdr.nlmsg_type != RTM_NEWLINK { off += nlmsg_align(hdr.nlmsg_len as usize); continue; }
            let payload = &links[off + NLMSG_HDR_SIZE..off + hdr.nlmsg_len as usize];
            let ifinfo = IfInfoMsg::parse(payload).unwrap_or_default();
            let mut name = String::new();
            let mut mac = String::new();
            let mut attr_off = IFINFOMSG_SIZE;
            while let Some((atype, adata, next)) = parse_attr(payload, attr_off) {
                match atype {
                    IFLA_IFNAME => { let end = adata.iter().position(|&b| b == 0).unwrap_or(adata.len()); name = String::from_utf8_lossy(&adata[..end]).to_string(); }
                    IFLA_ADDRESS if adata.len() >= 6 => { mac = format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", adata[0], adata[1], adata[2], adata[3], adata[4], adata[5]); }
                    _ => {}
                }
                attr_off = next;
            }
            let flags = if (ifinfo.ifi_flags & IFF_UP) != 0 { "UP" } else { "DOWN" };
            println!("{}: <{}> mtu 1500", name, flags);
            println!("    link/ether {}", mac);
            off += nlmsg_align(hdr.nlmsg_len as usize);
        }
    } else if args[0] == "set" {
        if args.len() < 3 { eprintln!("Usage: ip link set <iface> up|down"); return; }
        let idx = find_iface_index(fd, &args[1]);
        if idx < 0 { eprintln!("Unknown interface."); return; }
        match args[2].as_str() {
            "up" => { nl_set_link(fd, idx, true); println!("Link up."); }
            "down" => { nl_set_link(fd, idx, false); println!("Link down."); }
            _ => eprintln!("Usage: ip link set <iface> up|down"),
        }
    }
}

// --- Netlink helpers ---

fn nl_dump(fd: i32, msg_type: u16, payload_size: usize) -> Vec<u8> {
    let msg_len = NLMSG_HDR_SIZE + payload_size;
    let mut req = vec![0u8; msg_len];
    NlMsgHdr {
        nlmsg_len: msg_len as u32, nlmsg_type: msg_type,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_DUMP, nlmsg_seq: 1, nlmsg_pid: 0,
    }.serialize(&mut req);
    let nl_addr = SockAddrIn::default();
    socket::sendto(fd, &req, 0, &nl_addr);
    let mut response = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let mut from = SockAddrIn::default();
        let n = socket::recvfrom(fd, &mut buf, 0, &mut from);
        if n <= 0 { break; }
        response.extend_from_slice(&buf[..n as usize]);
        // Check for NLMSG_DONE in the response
        let mut check = 0;
        while check + NLMSG_HDR_SIZE <= response.len() {
            if let Some(h) = NlMsgHdr::parse(&response[check..]) {
                if h.nlmsg_type == NLMSG_DONE { return response; }
                check += nlmsg_align(h.nlmsg_len as usize).max(NLMSG_HDR_SIZE);
            } else { break; }
        }
    }
    response
}

fn find_iface_index(fd: i32, name: &str) -> i32 {
    let links = nl_dump(fd, RTM_GETLINK, IFINFOMSG_SIZE);
    let mut off = 0;
    while off < links.len() {
        let hdr = match NlMsgHdr::parse(&links[off..]) { Some(h) => h, None => break };
        if hdr.nlmsg_type == NLMSG_DONE { break; }
        if hdr.nlmsg_type == RTM_NEWLINK {
            let payload = &links[off + NLMSG_HDR_SIZE..off + hdr.nlmsg_len as usize];
            let ifinfo = IfInfoMsg::parse(payload).unwrap_or_default();
            let mut attr_off = IFINFOMSG_SIZE;
            while let Some((atype, adata, next)) = parse_attr(payload, attr_off) {
                if atype == IFLA_IFNAME {
                    let end = adata.iter().position(|&b| b == 0).unwrap_or(adata.len());
                    if std::str::from_utf8(&adata[..end]).unwrap_or("") == name {
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

fn get_iface_name_by_index(links: &[u8], target_idx: i32) -> String {
    let mut off = 0;
    while off < links.len() {
        let hdr = match NlMsgHdr::parse(&links[off..]) { Some(h) => h, None => break };
        if hdr.nlmsg_type == NLMSG_DONE { break; }
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

fn nl_set_link(fd: i32, iface_idx: i32, up: bool) {
    let msg_len = NLMSG_HDR_SIZE + IFINFOMSG_SIZE;
    let mut req = vec![0u8; msg_len];
    NlMsgHdr { nlmsg_len: msg_len as u32, nlmsg_type: RTM_NEWLINK, nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK, nlmsg_seq: 2, nlmsg_pid: 0 }.serialize(&mut req);
    IfInfoMsg { ifi_index: iface_idx, ifi_flags: if up { IFF_UP } else { 0 }, ifi_change: IFF_UP, ..Default::default() }.serialize(&mut req[NLMSG_HDR_SIZE..]);
    let nl_addr = SockAddrIn::default();
    socket::sendto(fd, &req, 0, &nl_addr);
    let mut buf = [0u8; 64]; let mut from = SockAddrIn::default();
    socket::recvfrom(fd, &mut buf, 0, &mut from);
}

fn nl_add_addr(fd: i32, iface_idx: i32, ip: &[u8; 4], prefix: u8) {
    let mut req = vec![0u8; 128];
    let mut off = NLMSG_HDR_SIZE;
    off += IfAddrMsg { ifa_family: AF_INET, ifa_prefixlen: prefix, ifa_index: iface_idx as u32, ..Default::default() }.serialize(&mut req[off..]);
    off += write_attr(&mut req, off, IFA_LOCAL, ip);
    NlMsgHdr { nlmsg_len: off as u32, nlmsg_type: RTM_NEWADDR, nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE, nlmsg_seq: 3, nlmsg_pid: 0 }.serialize(&mut req);
    let nl_addr = SockAddrIn::default();
    socket::sendto(fd, &req[..off], 0, &nl_addr);
    let mut buf = [0u8; 64]; let mut from = SockAddrIn::default();
    socket::recvfrom(fd, &mut buf, 0, &mut from);
}

fn nl_del_addr(fd: i32, iface_idx: i32) {
    let mut req = vec![0u8; 64];
    let mut off = NLMSG_HDR_SIZE;
    off += IfAddrMsg { ifa_family: AF_INET, ifa_index: iface_idx as u32, ..Default::default() }.serialize(&mut req[off..]);
    NlMsgHdr { nlmsg_len: off as u32, nlmsg_type: RTM_DELADDR, nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK, nlmsg_seq: 4, nlmsg_pid: 0 }.serialize(&mut req);
    let nl_addr = SockAddrIn::default();
    socket::sendto(fd, &req[..off], 0, &nl_addr);
    let mut buf = [0u8; 64]; let mut from = SockAddrIn::default();
    socket::recvfrom(fd, &mut buf, 0, &mut from);
}

fn nl_add_route(fd: i32, dest: &[u8; 4], prefix: u8, gw: &[u8; 4], iface_idx: i32) {
    let mut req = vec![0u8; 128];
    let mut off = NLMSG_HDR_SIZE;
    off += RtMsg { rtm_family: AF_INET, rtm_dst_len: prefix, rtm_table: RT_TABLE_MAIN, rtm_protocol: RTPROT_STATIC, rtm_scope: if *gw == [0,0,0,0] { RT_SCOPE_LINK } else { RT_SCOPE_UNIVERSE }, rtm_type: RTN_UNICAST, ..Default::default() }.serialize(&mut req[off..]);
    if prefix > 0 { off += write_attr(&mut req, off, RTA_DST, dest); }
    if *gw != [0, 0, 0, 0] { off += write_attr(&mut req, off, RTA_GATEWAY, gw); }
    off += write_attr_u32(&mut req, off, RTA_OIF, iface_idx as u32);
    NlMsgHdr { nlmsg_len: off as u32, nlmsg_type: RTM_NEWROUTE, nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE, nlmsg_seq: 5, nlmsg_pid: 0 }.serialize(&mut req);
    let nl_addr = SockAddrIn::default();
    socket::sendto(fd, &req[..off], 0, &nl_addr);
    let mut buf = [0u8; 64]; let mut from = SockAddrIn::default();
    socket::recvfrom(fd, &mut buf, 0, &mut from);
}

fn nl_del_route(fd: i32, dest: &[u8; 4], prefix: u8) {
    let mut req = vec![0u8; 64];
    let mut off = NLMSG_HDR_SIZE;
    off += RtMsg { rtm_family: AF_INET, rtm_dst_len: prefix, ..Default::default() }.serialize(&mut req[off..]);
    if prefix > 0 { off += write_attr(&mut req, off, RTA_DST, dest); }
    NlMsgHdr { nlmsg_len: off as u32, nlmsg_type: RTM_DELROUTE, nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK, nlmsg_seq: 6, nlmsg_pid: 0 }.serialize(&mut req);
    let nl_addr = SockAddrIn::default();
    socket::sendto(fd, &req[..off], 0, &nl_addr);
    let mut buf = [0u8; 64]; let mut from = SockAddrIn::default();
    socket::recvfrom(fd, &mut buf, 0, &mut from);
}

fn parse_cidr(s: &str) -> Option<([u8; 4], u8)> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 { return None; }
    let prefix: u8 = parts[1].parse().ok()?;
    let ip = parse_ip(parts[0])?;
    Some((ip, prefix))
}

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let o: Vec<&str> = s.split('.').collect();
    if o.len() != 4 { return None; }
    Some([o[0].parse().ok()?, o[1].parse().ok()?, o[2].parse().ok()?, o[3].parse().ok()?])
}

fn ip_str(ip: &[u8; 4]) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}
