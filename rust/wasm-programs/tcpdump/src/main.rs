use ecm_host_abi::socket::{self, SockAddrIn, AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE};
use ecm_host_abi::netlink::*;

// Host functions for packet capture and time
#[link(wasm_import_module = "env")]
extern "C" {
    fn get_time_ms() -> i64;
    fn net_set_promiscuous_on(index: i32, enabled: i32) -> i32;
    fn net_pcap_enable(index: i32, enabled: i32) -> i32;
    fn net_pcap_rx(index: i32, buf_ptr: *mut u8, buf_len: i32) -> i32;
}

struct Config {
    verbosity: u32,
    hex_dump: bool,
}

fn now_ms() -> i64 {
    unsafe { get_time_ms() }
}

fn format_time(ms: i64) -> String {
    let total_secs = ms / 1000;
    let h = (total_secs / 3600) % 24;
    let m = (total_secs / 60) % 60;
    let s = total_secs % 60;
    let frac = (ms % 1000) * 1000; // microseconds (from ms precision)
    format!("{:02}:{:02}:{:02}.{:06}", h, m, s, frac)
}

fn parse_args() -> (Config, Option<String>) {
    let args: Vec<String> = std::env::args().collect();
    let mut config = Config {
        verbosity: 0,
        hex_dump: false,
    };
    let mut interface: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg.starts_with('-') {
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;
            while j < chars.len() {
                match chars[j] {
                    'v' => config.verbosity += 1,
                    'x' => config.hex_dump = true,
                    'i' => {
                        // Interface name: rest of this arg or next arg
                        if j + 1 < chars.len() {
                            interface = Some(chars[j + 1..].iter().collect());
                            j = chars.len();
                            continue;
                        } else if i + 1 < args.len() {
                            i += 1;
                            interface = Some(args[i].clone());
                        } else {
                            eprintln!("tcpdump: option requires an argument -- 'i'");
                            std::process::exit(1);
                        }
                    }
                    c => {
                        eprintln!("tcpdump: invalid option -- '{}'", c);
                        std::process::exit(1);
                    }
                }
                j += 1;
            }
        }
        i += 1;
    }

    (config, interface)
}

// ---------------------------------------------------------------------------
// Netlink helpers (same pattern as ifconfig)
// ---------------------------------------------------------------------------

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

    let nl_addr = SockAddrIn::default();
    socket::sendto(fd, &req, 0, &nl_addr);

    let mut response = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let mut from = SockAddrIn::default();
        let n = socket::recvfrom(fd, &mut buf, 0, &mut from);
        if n <= 0 {
            break;
        }
        response.extend_from_slice(&buf[..n as usize]);
        // Check for NLMSG_DONE
        if response
            .windows(4)
            .any(|w| w.len() >= 2 && u16::from_le_bytes([w[0], w[1]]) == NLMSG_DONE)
        {
            break;
        }
    }
    response
}

/// Walk netlink interface dump and return (0-based frame index, name) for each.
fn enumerate_interfaces() -> Vec<(i32, String)> {
    let fd = socket::socket(AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE);
    if fd < 0 {
        return Vec::new();
    }

    let links = nl_dump(fd, RTM_GETLINK, IFINFOMSG_SIZE);
    socket::close(fd);

    let mut result = Vec::new();
    let mut iface_num: i32 = 0;
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
            let mut name = String::new();
            let mut attr_off = IFINFOMSG_SIZE;
            while let Some((atype, adata, next)) = parse_attr(payload, attr_off) {
                if atype == IFLA_IFNAME {
                    let end = adata.iter().position(|&b| b == 0).unwrap_or(adata.len());
                    name = String::from_utf8_lossy(&adata[..end]).to_string();
                }
                attr_off = next;
            }
            result.push((iface_num, name));
            iface_num += 1;
        }
        off += nlmsg_align(hdr.nlmsg_len as usize);
    }
    result
}

fn resolve_interface(name: &str) -> Option<(i32, String)> {
    enumerate_interfaces()
        .into_iter()
        .find(|(_, n)| n == name)
}

fn first_interface() -> Option<(i32, String)> {
    enumerate_interfaces().into_iter().next()
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn format_ip(ip: &[u8]) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

fn format_mac(mac: &[u8]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

fn tcp_flags_str(flags: u8) -> String {
    let mut s = String::new();
    if flags & 0x02 != 0 {
        s.push('S');
    } // SYN
    if flags & 0x01 != 0 {
        s.push('F');
    } // FIN
    if flags & 0x04 != 0 {
        s.push('R');
    } // RST
    if flags & 0x08 != 0 {
        s.push('P');
    } // PSH
    if flags & 0x10 != 0 {
        s.push('.');
    } // ACK
    if flags & 0x20 != 0 {
        s.push('U');
    } // URG
    if s.is_empty() {
        s.push_str("none");
    }
    s
}

fn hex_dump(data: &[u8]) {
    for (i, chunk) in data.chunks(16).enumerate() {
        let offset = i * 16;
        print!("\t0x{:04x}:  ", offset);
        for (j, pair) in chunk.chunks(2).enumerate() {
            if j > 0 {
                print!(" ");
            }
            for b in pair {
                print!("{:02x}", b);
            }
        }
        println!();
    }
}

// ---------------------------------------------------------------------------
// Packet processing
// ---------------------------------------------------------------------------

fn process_packet(frame: &[u8], frame_len: usize, config: &Config) {
    if frame_len < 14 {
        return;
    }

    let ts = format_time(now_ms());

    // Parse Ethernet header (14 bytes)
    let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    let eth_payload = &frame[14..frame_len];

    match ethertype {
        0x0800 => process_ipv4(&ts, eth_payload, config),
        0x0806 => process_arp(&ts, eth_payload, config),
        _ => {
            println!(
                "{} Unknown EtherType 0x{:04x}, length {}",
                ts, ethertype, frame_len
            );
        }
    }

    // -x: hex dump of packet data after link-layer header
    if config.hex_dump && !eth_payload.is_empty() {
        hex_dump(eth_payload);
    }
}

fn process_ipv4(ts: &str, data: &[u8], config: &Config) {
    if data.len() < 20 {
        return;
    }

    let version_ihl = data[0];
    let ihl = (version_ihl & 0x0f) as usize;
    let header_len = ihl * 4;
    if data.len() < header_len {
        return;
    }

    let tos = data[1];
    let total_length = u16::from_be_bytes([data[2], data[3]]);
    let identification = u16::from_be_bytes([data[4], data[5]]);
    let flags_frag = u16::from_be_bytes([data[6], data[7]]);
    let flags = (flags_frag >> 13) as u8;
    let frag_offset = flags_frag & 0x1fff;
    let ttl = data[8];
    let protocol = data[9];
    let src_ip = &data[12..16];
    let dst_ip = &data[16..20];

    let ip_payload = if (total_length as usize) <= data.len() {
        &data[header_len..total_length as usize]
    } else {
        &data[header_len..]
    };

    // IP flags string
    let mut flag_parts = Vec::new();
    if flags & 0x02 != 0 {
        flag_parts.push("DF");
    }
    if flags & 0x01 != 0 {
        flag_parts.push("+");
    }
    let flags_str = if flag_parts.is_empty() {
        "[none]".to_string()
    } else {
        format!("[{}]", flag_parts.join(","))
    };

    let ip_meta = IpMeta {
        tos,
        ttl,
        id: identification,
        frag_offset,
        flags_str: flags_str.clone(),
        total_len: total_length,
    };

    match protocol {
        6 => process_tcp(ts, src_ip, dst_ip, ip_payload, config, &ip_meta),
        17 => process_udp(ts, src_ip, dst_ip, ip_payload, config, &ip_meta),
        1 => process_icmp(ts, src_ip, dst_ip, ip_payload, config, &ip_meta),
        _ => {
            if config.verbosity >= 1 {
                println!(
                    "{} IP (tos 0x{:x}, ttl {}, id {}, offset {}, flags {}, proto {} ({}), length {})",
                    ts, tos, ttl, identification, frag_offset, flags_str, protocol, protocol,
                    total_length
                );
                println!(
                    "    {} > {}: protocol {}, length {}",
                    format_ip(src_ip),
                    format_ip(dst_ip),
                    protocol,
                    ip_payload.len()
                );
            } else {
                println!(
                    "{} IP {} > {}: protocol {}, length {}",
                    ts,
                    format_ip(src_ip),
                    format_ip(dst_ip),
                    protocol,
                    ip_payload.len()
                );
            }
        }
    }
}

struct IpMeta {
    tos: u8,
    ttl: u8,
    id: u16,
    frag_offset: u16,
    flags_str: String,
    total_len: u16,
}

fn process_tcp(
    ts: &str,
    src_ip: &[u8],
    dst_ip: &[u8],
    data: &[u8],
    config: &Config,
    ip: &IpMeta,
) {
    if data.len() < 20 {
        return;
    }

    let src_port = u16::from_be_bytes([data[0], data[1]]);
    let dst_port = u16::from_be_bytes([data[2], data[3]]);
    let seq = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let ack = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
    let data_offset = ((data[12] >> 4) as usize) * 4;
    let flags = data[13];
    let window = u16::from_be_bytes([data[14], data[15]]);
    let checksum = u16::from_be_bytes([data[16], data[17]]);

    let payload_len = if data.len() > data_offset {
        data.len() - data_offset
    } else {
        0
    };
    let flags_s = tcp_flags_str(flags);

    let seq_str = if payload_len > 0 {
        format!("seq {}:{}", seq, seq.wrapping_add(payload_len as u32))
    } else {
        format!("seq {}", seq)
    };

    let ack_str = if flags & 0x10 != 0 {
        format!(", ack {}", ack)
    } else {
        String::new()
    };

    if config.verbosity >= 1 {
        println!(
            "{} IP (tos 0x{:x}, ttl {}, id {}, offset {}, flags {}, proto TCP (6), length {})",
            ts, ip.tos, ip.ttl, ip.id, ip.frag_offset, ip.flags_str, ip.total_len
        );
        let cksum_str = if config.verbosity >= 2 {
            format!(", cksum 0x{:04x}", checksum)
        } else {
            String::new()
        };
        println!(
            "    {}.{} > {}.{}: Flags [{}]{}, {}{}, win {}, length {}",
            format_ip(src_ip),
            src_port,
            format_ip(dst_ip),
            dst_port,
            flags_s,
            cksum_str,
            seq_str,
            ack_str,
            window,
            payload_len
        );
    } else {
        println!(
            "{} IP {}.{} > {}.{}: Flags [{}], {}{}, win {}, length {}",
            ts,
            format_ip(src_ip),
            src_port,
            format_ip(dst_ip),
            dst_port,
            flags_s,
            seq_str,
            ack_str,
            window,
            payload_len
        );
    }
}

fn process_udp(
    ts: &str,
    src_ip: &[u8],
    dst_ip: &[u8],
    data: &[u8],
    config: &Config,
    ip: &IpMeta,
) {
    if data.len() < 8 {
        return;
    }

    let src_port = u16::from_be_bytes([data[0], data[1]]);
    let dst_port = u16::from_be_bytes([data[2], data[3]]);
    let udp_length = u16::from_be_bytes([data[4], data[5]]);
    let payload_len = if udp_length >= 8 {
        udp_length - 8
    } else {
        0
    };

    if config.verbosity >= 1 {
        println!(
            "{} IP (tos 0x{:x}, ttl {}, id {}, offset {}, flags {}, proto UDP (17), length {})",
            ts, ip.tos, ip.ttl, ip.id, ip.frag_offset, ip.flags_str, ip.total_len
        );
        println!(
            "    {}.{} > {}.{}: UDP, length {}",
            format_ip(src_ip),
            src_port,
            format_ip(dst_ip),
            dst_port,
            payload_len
        );
    } else {
        println!(
            "{} IP {}.{} > {}.{}: UDP, length {}",
            ts,
            format_ip(src_ip),
            src_port,
            format_ip(dst_ip),
            dst_port,
            payload_len
        );
    }
}

fn process_icmp(
    ts: &str,
    src_ip: &[u8],
    dst_ip: &[u8],
    data: &[u8],
    config: &Config,
    ip: &IpMeta,
) {
    if data.len() < 8 {
        return;
    }

    let icmp_type = data[0];
    let code = data[1];
    let icmp_id = u16::from_be_bytes([data[4], data[5]]);
    let icmp_seq = u16::from_be_bytes([data[6], data[7]]);

    let type_str = match (icmp_type, code) {
        (8, _) => "echo request",
        (0, _) => "echo reply",
        (3, 0) => "net unreachable",
        (3, 1) => "host unreachable",
        (3, 3) => "port unreachable",
        (11, 0) => "time exceeded in-transit",
        _ => "ICMP type unknown",
    };

    if config.verbosity >= 1 {
        println!(
            "{} IP (tos 0x{:x}, ttl {}, id {}, offset {}, flags {}, proto ICMP (1), length {})",
            ts, ip.tos, ip.ttl, ip.id, ip.frag_offset, ip.flags_str, ip.total_len
        );
        println!(
            "    {} > {}: ICMP {}, id {}, seq {}, length {}",
            format_ip(src_ip),
            format_ip(dst_ip),
            type_str,
            icmp_id,
            icmp_seq,
            data.len()
        );
    } else {
        println!(
            "{} IP {} > {}: ICMP {}, id {}, seq {}, length {}",
            ts,
            format_ip(src_ip),
            format_ip(dst_ip),
            type_str,
            icmp_id,
            icmp_seq,
            data.len()
        );
    }
}

fn process_arp(ts: &str, data: &[u8], config: &Config) {
    if data.len() < 28 {
        return;
    }

    let operation = u16::from_be_bytes([data[6], data[7]]);
    let sender_mac = &data[8..14];
    let sender_ip = &data[14..18];
    let target_ip = &data[24..28];

    match operation {
        1 => {
            // ARP Request
            if config.verbosity >= 1 {
                println!(
                    "{} ARP, Ethernet (len 6), IPv4 (len 4), Request who-has {} tell {} ({}), length {}",
                    ts,
                    format_ip(target_ip),
                    format_ip(sender_ip),
                    format_mac(sender_mac),
                    data.len()
                );
            } else {
                println!(
                    "{} ARP, Request who-has {} tell {}, length {}",
                    ts,
                    format_ip(target_ip),
                    format_ip(sender_ip),
                    data.len()
                );
            }
        }
        2 => {
            // ARP Reply
            if config.verbosity >= 1 {
                println!(
                    "{} ARP, Ethernet (len 6), IPv4 (len 4), Reply {} is-at {}, length {}",
                    ts,
                    format_ip(sender_ip),
                    format_mac(sender_mac),
                    data.len()
                );
            } else {
                println!(
                    "{} ARP, Reply {} is-at {}, length {}",
                    ts,
                    format_ip(sender_ip),
                    format_mac(sender_mac),
                    data.len()
                );
            }
        }
        _ => {
            println!(
                "{} ARP, unknown op {}, length {}",
                ts,
                operation,
                data.len()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let (config, iface_arg) = parse_args();

    // Resolve interface
    let (iface_idx, iface_name) = if let Some(ref name) = iface_arg {
        match resolve_interface(name) {
            Some(v) => v,
            None => {
                eprintln!("tcpdump: {}: No such device exists", name);
                std::process::exit(1);
            }
        }
    } else {
        match first_interface() {
            Some(v) => v,
            None => {
                eprintln!("tcpdump: no suitable device found");
                std::process::exit(1);
            }
        }
    };

    // Enable promiscuous mode and packet capture on the interface.
    // pcap uses a separate mirror queue so the kernel still processes frames normally.
    let (promisc_rc, pcap_rc) = unsafe {
        (
            net_set_promiscuous_on(iface_idx, 1),
            net_pcap_enable(iface_idx, 1),
        )
    };
    if promisc_rc < 0 {
        eprintln!("tcpdump: failed to enable promiscuous mode on {}", iface_name);
        std::process::exit(1);
    }
    if pcap_rc < 0 {
        eprintln!("tcpdump: failed to enable packet capture on {}", iface_name);
        std::process::exit(1);
    }

    if config.verbosity == 0 {
        eprintln!(
            "tcpdump: verbose output suppressed, use -v[v]... for full protocol decode"
        );
    }
    eprintln!(
        "tcpdump: listening on {}, link-type EN10MB (Ethernet), snapshot length 65535 bytes",
        iface_name
    );

    let mut frame_buf = [0u8; 2048];
    let mut _pkt_count: u64 = 0;

    loop {
        let len = unsafe {
            net_pcap_rx(iface_idx, frame_buf.as_mut_ptr(), frame_buf.len() as i32)
        };

        if len > 0 {
            _pkt_count += 1;
            process_packet(&frame_buf, len as usize, &config);
        } else {
            // No frame available — yield briefly to avoid busy-waiting
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}
