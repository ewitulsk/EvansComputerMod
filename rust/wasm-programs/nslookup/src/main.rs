extern crate ecm_host_abi;
use ecm_host_abi::net_ipc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: nslookup <hostname>");
        std::process::exit(1);
    }

    let name = &args[1];

    // Get DNS server info
    if let Some(dns) = ecm_host_abi::net_config::dns_get() {
        println!("Server: {}", dns);
    }

    let mut ip_out = [0u8; 4];
    let result = net_ipc::dns_resolve(name, &mut ip_out);
    if result == 0 {
        println!("Name:    {}", name);
        println!("Address: {}.{}.{}.{}", ip_out[0], ip_out[1], ip_out[2], ip_out[3]);
    } else {
        eprintln!("DNS lookup failed");
        std::process::exit(1);
    }
}
