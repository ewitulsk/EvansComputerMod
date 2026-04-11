use ecm_host_abi::socket::{self, SockAddrIn, AF_INET, SOCK_RAW, IPPROTO_ICMP};

#[link(wasm_import_module = "env")]
extern "C" {
    fn get_time_ms() -> i64;
}

fn now_ms() -> i64 {
    unsafe { get_time_ms() }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: ping <ip_or_hostname> [count]");
        std::process::exit(1);
    }

    let target = &args[1];
    let count: u32 = if args.len() >= 3 {
        args[2].parse().unwrap_or(4)
    } else {
        4
    };

    // Resolve target to IP
    let mut addr = SockAddrIn::default();
    if socket::getaddrinfo(target, &mut addr) != 0 {
        eprintln!("ping: unknown host {}", target);
        std::process::exit(1);
    }

    let ip_str = format!("{}.{}.{}.{}",
        addr.sin_addr[0], addr.sin_addr[1], addr.sin_addr[2], addr.sin_addr[3]);

    // Create raw ICMP socket
    let fd = socket::socket(AF_INET, SOCK_RAW, IPPROTO_ICMP);
    if fd < 0 {
        eprintln!("ping: failed to create raw socket");
        std::process::exit(1);
    }

    println!("PING {} ({}) - {} packets", target, ip_str, count);

    let mut sent = 0u32;
    let mut received = 0u32;

    for seq in 0..count {
        sent += 1;

        // Build ICMP echo request
        let mut icmp_pkt = [0u8; 40]; // 8-byte header + 32-byte data
        icmp_pkt[0] = 8;  // type = echo request
        icmp_pkt[1] = 0;  // code = 0
        // checksum at [2..4] - filled below
        icmp_pkt[4] = 0;  // id high
        icmp_pkt[5] = 1;  // id low = 1
        icmp_pkt[6] = (seq >> 8) as u8;  // seq high
        icmp_pkt[7] = seq as u8;         // seq low
        // Fill data
        for i in 8..40 { icmp_pkt[i] = (i - 8) as u8; }
        // Calculate checksum
        let cksum = icmp_checksum(&icmp_pkt);
        icmp_pkt[2] = (cksum >> 8) as u8;
        icmp_pkt[3] = cksum as u8;

        let start_ms = now_ms();
        let n = socket::sendto(fd, &icmp_pkt, 0, &addr);
        if n < 0 {
            println!("Request timed out (send failed)");
            continue;
        }

        // Wait for reply
        let mut reply_buf = [0u8; 128];
        let mut from = SockAddrIn::default();
        let n = socket::recvfrom(fd, &mut reply_buf, 0, &mut from);
        if n > 0 {
            let elapsed = now_ms() - start_ms;
            received += 1;
            println!("Reply from {}: bytes={} time={}ms seq={}", ip_str, n, elapsed, seq);
        } else {
            println!("Request timed out");
        }

        if seq + 1 < count {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    socket::close(fd);

    println!("--- {} ping statistics ---", target);
    println!("{} packets sent, {} received", sent, received);
}

fn icmp_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += ((data[i] as u32) << 8) | (data[i + 1] as u32);
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !sum as u16
}
