use ecm_host_abi::netlink::*;
use ecm_net_tools::nl::{find_iface_index, ip_str, nl_dump, parse_cidr};
use ecm_net_tools::NetTools;

pub fn run(args: &[String], nt: &mut dyn NetTools) -> i32 {
    if args.is_empty() {
        show_all_interfaces(nt);
        return 0;
    }

    let iface_name = &args[0];
    if args.len() == 1 {
        // Legacy behavior: show all (same as the pre-refactor program).
        show_all_interfaces(nt);
        return 0;
    }

    configure_interface(nt, iface_name, &args[1..])
}

fn show_all_interfaces(nt: &mut dyn NetTools) {
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
        nt.out(&format!("{}: flags=<{}>  mtu 1500\n", name, flags));
        if !mac.is_empty() {
            nt.out(&format!("      ether {}\n", mac));
        }

        // Find matching addresses in the RTM_GETADDR dump.
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
                let apayload =
                    &addrs[addr_off + NLMSG_HDR_SIZE..addr_off + ahdr.nlmsg_len as usize];
                let ifa = IfAddrMsg::parse(apayload).unwrap_or_default();
                if ifa.ifa_index == ifinfo.ifi_index as u32 {
                    let mut aattr_off = IFADDRMSG_SIZE;
                    while let Some((atype, adata, next)) = parse_attr(apayload, aattr_off) {
                        if atype == IFA_LOCAL && adata.len() >= 4 {
                            nt.out(&format!(
                                "      inet {}.{}.{}.{}/{}\n",
                                adata[0], adata[1], adata[2], adata[3], ifa.ifa_prefixlen
                            ));
                        }
                        aattr_off = next;
                    }
                }
            }
            addr_off += nlmsg_align(ahdr.nlmsg_len as usize);
        }
        nt.out("\n");

        off += nlmsg_align(hdr.nlmsg_len as usize);
    }
}

fn configure_interface(nt: &mut dyn NetTools, iface_name: &str, config_args: &[String]) -> i32 {
    let iface_idx = find_iface_index(nt, iface_name);
    if iface_idx < 0 {
        nt.err(&format!("Unknown interface: {}\n", iface_name));
        return 1;
    }

    match config_args[0].as_str() {
        "up" => {
            set_link_state(nt, iface_idx, true);
            nt.out("Link up.\n");
            0
        }
        "down" => {
            set_link_state(nt, iface_idx, false);
            nt.out("Link down.\n");
            0
        }
        cidr if cidr.contains('/') => {
            if let Some((ip, prefix)) = parse_cidr(cidr) {
                add_address(nt, iface_idx, &ip, prefix);
                nt.out(&format!("{}: inet {}/{}\n", iface_name, ip_str(&ip), prefix));
                0
            } else {
                nt.err("Invalid CIDR address.\n");
                1
            }
        }
        _ => {
            nt.err("Usage: ifconfig <iface> [<ip>/<prefix> | up | down]\n");
            1
        }
    }
}

fn set_link_state(nt: &mut dyn NetTools, iface_idx: i32, up: bool) {
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
        ifi_family: 0,
        _pad: 0,
        ifi_type: 0,
        ifi_index: iface_idx,
        ifi_flags: if up { IFF_UP } else { 0 },
        ifi_change: IFF_UP,
    }
    .serialize(&mut req[NLMSG_HDR_SIZE..]);
    let _ = nt.netlink_request(&req);
}

fn add_address(nt: &mut dyn NetTools, iface_idx: i32, ip: &[u8; 4], prefix: u8) {
    let mut req = vec![0u8; 128];
    let mut off = NLMSG_HDR_SIZE;
    off += IfAddrMsg {
        ifa_family: AF_INET,
        ifa_prefixlen: prefix,
        ifa_flags: 0,
        ifa_scope: 0,
        ifa_index: iface_idx as u32,
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
