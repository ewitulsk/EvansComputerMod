use ecm_host_abi::socket::{self, SockAddrIn};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: resolvectl status | query <hostname>");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "status" => {
            println!("DNS resolver status:");
            println!("  (Use 'resolvectl query <hostname>' to test resolution)");
        }
        "query" => {
            if args.len() < 3 {
                eprintln!("Usage: resolvectl query <hostname>");
                return;
            }
            let name = &args[2];
            println!("Resolving {}...", name);
            let mut addr = SockAddrIn::default();
            if socket::getaddrinfo(name, &mut addr) == 0 {
                let ip = addr.sin_addr;
                println!("{} -> {}.{}.{}.{}", name, ip[0], ip[1], ip[2], ip[3]);
            } else {
                eprintln!("Resolution failed for {}", name);
            }
        }
        _ => {
            eprintln!("Usage: resolvectl status | query <hostname>");
        }
    }
}
