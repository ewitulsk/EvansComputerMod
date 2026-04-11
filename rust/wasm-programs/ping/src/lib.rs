use ecm_net_tools::NetTools;

const PER_PACKET_TIMEOUT_MS: u32 = 2000;

pub fn run(args: &[String], nt: &mut dyn NetTools) -> i32 {
    if args.is_empty() {
        nt.err("Usage: ping <ip_or_hostname> [-n <count>]\n");
        return 1;
    }

    let target = &args[0];

    // Parse `-n <count>`. Without it, ping runs until interrupted (Ctrl+T) —
    // matches the upstream WASI ping CLI's "continuous by default" behaviour.
    let mut count: Option<u32> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-n" => {
                i += 1;
                if i >= args.len() {
                    nt.err("ping: -n requires a packet count\n");
                    return 1;
                }
                let parsed = match args[i].parse::<u32>() {
                    Ok(n) if n > 0 => n,
                    _ => {
                        nt.err(&format!("ping: invalid packet count '{}'\n", args[i]));
                        return 1;
                    }
                };
                count = Some(parsed);
            }
            other => {
                nt.err(&format!("ping: unknown argument '{}'\n", other));
                nt.err("Usage: ping <ip_or_hostname> [-n <count>]\n");
                return 1;
            }
        }
        i += 1;
    }

    let ip = match nt.getaddrinfo(target) {
        Some(ip) => ip,
        None => {
            nt.err(&format!("ping: unknown host {}\n", target));
            return 1;
        }
    };

    let ip_str = format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
    match count {
        Some(n) => nt.out(&format!("PING {} ({}) - {} packets\n", target, ip_str, n)),
        None => nt.out(&format!("PING {} ({}) - continuous\n", target, ip_str)),
    }

    let mut sent = 0u32;
    let mut received = 0u32;
    let mut seq: u32 = 0;

    loop {
        if let Some(limit) = count {
            if seq >= limit {
                break;
            }
        }
        sent += 1;
        match nt.icmp_echo(ip, 1, seq as u16, PER_PACKET_TIMEOUT_MS) {
            Some(elapsed) => {
                received += 1;
                nt.out(&format!(
                    "Reply from {}: time={}ms seq={}\n",
                    ip_str, elapsed, seq
                ));
            }
            None => {
                nt.out("Request timed out\n");
            }
        }

        let has_more = match count {
            Some(limit) => seq + 1 < limit,
            None => true,
        };
        if has_more {
            nt.sleep_ms(1000);
        }
        seq += 1;
    }

    nt.out(&format!("--- {} ping statistics ---\n", target));
    nt.out(&format!("{} packets sent, {} received\n", sent, received));
    0
}
