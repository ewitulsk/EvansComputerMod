extern crate ecm_host_abi;
use ecm_host_abi::net_config;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: ping <ip> [count]");
        std::process::exit(1);
    }

    let target = &args[1];
    let count: u32 = if args.len() >= 3 {
        args[2].parse().unwrap_or(4)
    } else {
        4
    };

    println!("PING {} - {} packets", target, count);

    let mut sent = 0u32;
    let mut received = 0u32;

    for seq in 0..count {
        sent += 1;
        let rtt = net_config::ping(target, 2000);
        if rtt >= 0 {
            received += 1;
            println!("Reply from {}: time={}ms seq={}", target, rtt, seq);
        } else {
            println!("Request timed out");
        }
        if seq + 1 < count {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    println!("--- {} ping statistics ---", target);
    println!("{} packets sent, {} received", sent, received);
}
