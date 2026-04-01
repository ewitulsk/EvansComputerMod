extern crate ecm_host_abi;
use ecm_host_abi::net_config;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: resolvectl status | dns [iface] <server> | query <hostname>");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "status" => {
            let dns = net_config::dns_get().unwrap_or_else(|| "(none)".to_string());
            println!("Global DNS: {}", dns);
            println!();

            let count = net_config::iface_count();
            for i in 0..count {
                if let Some(info) = net_config::iface_info(i) {
                    let name = extract_json_str(&info, "name").unwrap_or("?".to_string());
                    let link_up = info.contains("\"link_up\":true");
                    let flags = if link_up { "UP" } else { "DOWN" };
                    println!("Link {} ({}):", name, flags);

                    if let Some(ip) = extract_json_str(&info, "ip") {
                        if !ip.is_empty() && ip != "0.0.0.0" {
                            let prefix = extract_json_int(&info, "prefix").unwrap_or(0);
                            println!("    Address: {}/{}", ip, prefix);
                        }
                    }

                    println!("    DNS: {}", dns);
                }
            }
        }
        "dns" => {
            // resolvectl dns [iface] <server>
            let mut server_ip = None;
            for arg in args.iter().skip(2) {
                // Try to parse as IP
                let octets: Vec<&str> = arg.split('.').collect();
                if octets.len() == 4 && octets.iter().all(|o| o.parse::<u8>().is_ok()) {
                    server_ip = Some(arg.clone());
                    break;
                }
            }
            match server_ip {
                Some(ip) => {
                    net_config::dns_set(&ip);
                    net_config::save_config();
                    println!("DNS server set to {}", ip);
                }
                None => {
                    let dns = net_config::dns_get().unwrap_or_else(|| "(none)".to_string());
                    println!("Global DNS: {}", dns);
                }
            }
        }
        "query" => {
            if args.len() < 3 {
                eprintln!("Usage: resolvectl query <hostname>");
                return;
            }
            let name = &args[2];
            let dns = net_config::dns_get().unwrap_or_else(|| "(none)".to_string());
            if dns == "(none)" || dns == "0.0.0.0" {
                eprintln!("No DNS server configured. Use: resolvectl dns <iface> <server>");
                return;
            }
            println!("Resolving {} via {}...", name, dns);
            let mut ip_out = [0u8; 4];
            let result = ecm_host_abi::net_ipc::dns_resolve(name, &mut ip_out);
            if result == 0 {
                println!("{} -> {}.{}.{}.{}", name, ip_out[0], ip_out[1], ip_out[2], ip_out[3]);
            } else {
                eprintln!("Resolution failed");
            }
        }
        _ => {
            eprintln!("Usage: resolvectl status | dns [iface] <server> | query <hostname>");
        }
    }
}

fn extract_json_str(json: &str, key: &str) -> Option<String> {
    let search = format!("\"{}\":\"", key);
    let start = json.find(&search)? + search.len();
    let rest = &json[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_json_int(json: &str, key: &str) -> Option<i32> {
    let search = format!("\"{}\":", key);
    let start = json.find(&search)? + search.len();
    let rest = &json[start..];
    let end = rest.find(|c: char| !c.is_ascii_digit() && c != '-').unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}
