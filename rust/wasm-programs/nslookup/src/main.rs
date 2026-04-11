use ecm_host_abi::socket::{self, SockAddrIn};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: nslookup <hostname>");
        std::process::exit(1);
    }

    let name = &args[1];

    let mut addr = SockAddrIn::default();
    let result = socket::getaddrinfo(name, &mut addr);
    if result == 0 {
        let ip = addr.sin_addr;
        println!("Name:    {}", name);
        println!("Address: {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
    } else {
        eprintln!("DNS lookup failed for {}", name);
        std::process::exit(1);
    }
}
