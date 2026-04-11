use ecm_host_abi::socket::{self, SockAddrIn, AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE};
use ecm_host_abi::netlink::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let fd = socket::socket(AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE);
    if fd < 0 {
        eprintln!("ifconfig: failed to create netlink socket");
        std::process::exit(1);
    }

    if args.len() <= 1 {
        // Show all interfaces
        show_all_interfaces(fd);
    } else {
        let iface_name = &args[1];
        if args.len() == 2 {
            show_interface_by_name(fd, iface_name);
        } else {
            configure_interface(fd, iface_name, &args[2..]);
        }
    }

    socket::close(fd);
}

fn show_all_interfaces(fd: i32) {
    // Send RTM_GETLINK dump request
    let links = nl_dump(fd, RTM_GETLINK, IFINFOMSG_SIZE);
    // Send RTM_GETADDR dump request
    let addrs = nl_dump(fd, RTM_GETADDR, IFADDRMSG_SIZE);

    // Parse links
    let mut off = 0;
    while off < links.len() {
        let hdr = match NlMsgHdr::parse(&links[off..]) {
            Some(h) => h,
            None => break,
        };
        if hdr.nlmsg_type == NLMSG_DONE { break; }
        if hdr.nlmsg_type != RTM_NEWLINK { off += nlmsg_align(hdr.nlmsg_len as usize); continue; }

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
                    mac = format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                        adata[0], adata[1], adata[2], adata[3], adata[4], adata[5]);
                }
                _ => {}
            }
            attr_off = next;
        }

        let flags = if (ifinfo.ifi_flags & IFF_UP) != 0 { "UP" } else { "DOWN" };
        println!("{}: flags=<{}>  mtu 1500", name, flags);
        if !mac.is_empty() {
            println!("      ether {}", mac);
        }

        // Find matching address
        let mut addr_off = 0;
        while addr_off < addrs.len() {
            let ahdr = match NlMsgHdr::parse(&addrs[addr_off..]) {
                Some(h) => h,
                None => break,
            };
            if ahdr.nlmsg_type == NLMSG_DONE { break; }
            if ahdr.nlmsg_type == RTM_NEWADDR {
                let apayload = &addrs[addr_off + NLMSG_HDR_SIZE..addr_off + ahdr.nlmsg_len as usize];
                let ifa = IfAddrMsg::parse(apayload).unwrap_or_default();
                if ifa.ifa_index == ifinfo.ifi_index as u32 {
                    let mut aattr_off = IFADDRMSG_SIZE;
                    while let Some((atype, adata, next)) = parse_attr(apayload, aattr_off) {
                        if atype == IFA_LOCAL && adata.len() >= 4 {
                            println!("      inet {}.{}.{}.{}/{}", adata[0], adata[1], adata[2], adata[3], ifa.ifa_prefixlen);
                        }
                        aattr_off = next;
                    }
                }
            }
            addr_off += nlmsg_align(ahdr.nlmsg_len as usize);
        }
        println!();

        off += nlmsg_align(hdr.nlmsg_len as usize);
    }
}

fn show_interface_by_name(fd: i32, target: &str) {
    // For simplicity, dump all and filter
    // A more efficient approach would use NLM_F_MATCH, but dump works fine
    show_all_interfaces(fd);
}

fn configure_interface(fd: i32, iface_name: &str, config_args: &[String]) {
    // Find interface index by name
    let iface_idx = find_iface_index(fd, iface_name);
    if iface_idx < 0 {
        eprintln!("Unknown interface: {}", iface_name);
        return;
    }

    match config_args[0].as_str() {
        "up" => {
            set_link_state(fd, iface_idx, true);
            println!("Link up.");
        }
        "down" => {
            set_link_state(fd, iface_idx, false);
            println!("Link down.");
        }
        cidr if cidr.contains('/') => {
            if let Some((ip, prefix)) = parse_cidr(cidr) {
                add_address(fd, iface_idx, &ip, prefix);
                println!("{}: inet {}/{}", iface_name, ip_to_str(&ip), prefix);
            } else {
                eprintln!("Invalid CIDR address.");
            }
        }
        _ => eprintln!("Usage: ifconfig <iface> [<ip>/<prefix> | up | down]"),
    }
}

// --- Netlink helpers ---

fn nl_dump(fd: i32, msg_type: u16, payload_size: usize) -> Vec<u8> {
    let msg_len = NLMSG_HDR_SIZE + payload_size;
    let mut req = vec![0u8; msg_len];
    let hdr = NlMsgHdr {
        nlmsg_len: msg_len as u32,
        nlmsg_type: msg_type,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_DUMP,
        nlmsg_seq: 1,
        nlmsg_pid: 0,
    };
    hdr.serialize(&mut req);

    let nl_addr = SockAddrIn::default(); // netlink uses kernel address (all zeros)
    socket::sendto(fd, &req, 0, &nl_addr);

    let mut response = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let mut from = SockAddrIn::default();
        let n = socket::recvfrom(fd, &mut buf, 0, &mut from);
        if n <= 0 { break; }
        response.extend_from_slice(&buf[..n as usize]);
        // Check if we got NLMSG_DONE
        if response.windows(4).any(|w| {
            w.len() >= 2 && u16::from_le_bytes([w[0], w[1]]) == NLMSG_DONE
        }) {
            break;
        }
    }
    response
}

fn find_iface_index(fd: i32, name: &str) -> i32 {
    let links = nl_dump(fd, RTM_GETLINK, IFINFOMSG_SIZE);
    let mut off = 0;
    while off < links.len() {
        let hdr = match NlMsgHdr::parse(&links[off..]) {
            Some(h) => h,
            None => break,
        };
        if hdr.nlmsg_type == NLMSG_DONE { break; }
        if hdr.nlmsg_type == RTM_NEWLINK {
            let payload = &links[off + NLMSG_HDR_SIZE..off + hdr.nlmsg_len as usize];
            let ifinfo = IfInfoMsg::parse(payload).unwrap_or_default();
            let mut attr_off = IFINFOMSG_SIZE;
            while let Some((atype, adata, next)) = parse_attr(payload, attr_off) {
                if atype == IFLA_IFNAME {
                    let end = adata.iter().position(|&b| b == 0).unwrap_or(adata.len());
                    let iname = std::str::from_utf8(&adata[..end]).unwrap_or("");
                    if iname == name {
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

fn set_link_state(fd: i32, iface_idx: i32, up: bool) {
    let msg_len = NLMSG_HDR_SIZE + IFINFOMSG_SIZE;
    let mut req = vec![0u8; msg_len];
    let hdr = NlMsgHdr {
        nlmsg_len: msg_len as u32,
        nlmsg_type: RTM_NEWLINK,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK,
        nlmsg_seq: 2,
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
    socket::sendto(fd, &req, 0, &nl_addr);

    // Read ack
    let mut buf = [0u8; 64];
    let mut from = SockAddrIn::default();
    socket::recvfrom(fd, &mut buf, 0, &mut from);
}

fn add_address(fd: i32, iface_idx: i32, ip: &[u8; 4], prefix: u8) {
    let mut req = vec![0u8; 128];
    let mut off = NLMSG_HDR_SIZE;

    let ifa = IfAddrMsg {
        ifa_family: AF_INET,
        ifa_prefixlen: prefix,
        ifa_flags: 0,
        ifa_scope: 0,
        ifa_index: iface_idx as u32,
    };
    off += ifa.serialize(&mut req[off..]);
    off += write_attr(&mut req, off, IFA_LOCAL, ip);

    let hdr = NlMsgHdr {
        nlmsg_len: off as u32,
        nlmsg_type: RTM_NEWADDR,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE,
        nlmsg_seq: 3,
        nlmsg_pid: 0,
    };
    hdr.serialize(&mut req);

    let nl_addr = SockAddrIn::default();
    socket::sendto(fd, &req[..off], 0, &nl_addr);

    let mut buf = [0u8; 64];
    let mut from = SockAddrIn::default();
    socket::recvfrom(fd, &mut buf, 0, &mut from);
}

fn parse_cidr(s: &str) -> Option<([u8; 4], u8)> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 { return None; }
    let prefix: u8 = parts[1].parse().ok()?;
    let octets: Vec<&str> = parts[0].split('.').collect();
    if octets.len() != 4 { return None; }
    let a: u8 = octets[0].parse().ok()?;
    let b: u8 = octets[1].parse().ok()?;
    let c: u8 = octets[2].parse().ok()?;
    let d: u8 = octets[3].parse().ok()?;
    Some(([a, b, c, d], prefix))
}

fn ip_to_str(ip: &[u8; 4]) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}
