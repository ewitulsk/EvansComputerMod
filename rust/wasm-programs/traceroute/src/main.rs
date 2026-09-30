use ecm_host_abi::socket::{self, *};
use ecm_net::{icmp::IcmpPacket, ipv4::Ipv4Header};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let Some(target) = args.get(1) else {
        eprintln!("Usage: traceroute <host> [max-hops]");
        return;
    };
    let max = args
        .get(2)
        .and_then(|s| s.parse::<u8>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(30);
    let mut dest = SockAddrIn::default();
    if socket::getaddrinfo(target, &mut dest) != 0 {
        eprintln!("Unknown host {}", target);
        return;
    }
    let fd = socket::socket(AF_INET, SOCK_RAW, IPPROTO_ICMP);
    if fd < 0 {
        eprintln!("Cannot open ICMP socket");
        return;
    }
    socket::setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &1000i32.to_le_bytes());
    let id = std::process::id() as u16;
    println!(
        "traceroute to {} ({}.{}.{}.{}), {} hops",
        target, dest.sin_addr[0], dest.sin_addr[1], dest.sin_addr[2], dest.sin_addr[3], max
    );
    for ttl in 1..=max {
        if socket::setsockopt(fd, IPPROTO_IP, IP_TTL, &(ttl as i32).to_le_bytes()) != 0 {
            eprintln!("IP_TTL unavailable");
            break;
        }
        let probe = IcmpPacket::build_echo(8, id, ttl as u16, b"ECM traceroute");
        if socket::sendto(fd, &probe, 0, &dest) < 0 {
            println!("{}  *", ttl);
            continue;
        }
        let mut buf = [0u8; 1500];
        let mut from = SockAddrIn::default();
        let mut matched = false;
        let mut done = false;
        // Ignore a bounded number of unrelated raw-socket messages.
        for _ in 0..16 {
            let n = socket::recvfrom(fd, &mut buf, 0, &mut from);
            if n < 8 {
                break;
            }
            let Some((h, p)) = IcmpPacket::parse(&buf[..n as usize]) else {
                continue;
            };
            if h.icmp_type == 0 && h.id == id && h.seq == ttl as u16 {
                matched = true;
                done = true;
                break;
            }
            if matches!(h.icmp_type, 3 | 11) {
                // Quoted packets are shorter than their declared IP length.
                if p.len() >= 28 && p[9] == 1 {
                    let hl = ((p[0] & 15) as usize) * 4;
                    if hl >= Ipv4Header::SIZE
                        && p.len() >= hl + 8
                        && p[hl + 4..hl + 6] == id.to_be_bytes()
                        && p[hl + 6..hl + 8] == (ttl as u16).to_be_bytes()
                    {
                        matched = true;
                        done = h.icmp_type == 3;
                        break;
                    }
                }
            }
        }
        if matched {
            println!(
                "{}  {}.{}.{}.{}",
                ttl, from.sin_addr[0], from.sin_addr[1], from.sin_addr[2], from.sin_addr[3]
            );
        } else {
            println!("{}  *", ttl);
        }
        if done {
            break;
        }
    }
    socket::close(fd);
}
