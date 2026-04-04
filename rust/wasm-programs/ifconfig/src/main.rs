extern crate ecm_host_abi;
use ecm_host_abi::net_config;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() <= 1 {
        // Show all interfaces
        let count = net_config::iface_count();
        for i in 0..count {
            show_interface(i);
        }
        return;
    }

    let iface_name = &args[1];
    let iface_idx = find_iface(iface_name);
    if iface_idx < 0 {
        eprintln!("Unknown interface: {}", iface_name);
        std::process::exit(1);
    }

    if args.len() == 2 {
        // Show specific interface
        show_interface(iface_idx);
        return;
    }

    match args[2].as_str() {
        "up" => {
            net_config::iface_set_link(iface_idx, true);
            println!("Link up.");
            net_config::save_config();
        }
        "down" => {
            net_config::iface_set_link(iface_idx, false);
            println!("Link down.");
            net_config::save_config();
        }
        "vlan" if args.len() >= 4 => {
            let vlan_arg = &args[3];
            if vlan_arg == "off" || vlan_arg == "none" {
                net_config::iface_set_vlan(iface_idx, -1);
                println!("VLAN disabled.");
            } else if let Ok(vid) = vlan_arg.parse::<i32>() {
                if vid >= 0 && vid <= 4094 {
                    net_config::iface_set_vlan(iface_idx, vid);
                    println!("VLAN set to {}.", vid);
                } else {
                    eprintln!("VLAN ID must be 0-4094.");
                }
            } else {
                eprintln!("Invalid VLAN ID.");
            }
            net_config::save_config();
        }
        cidr if cidr.contains('/') => {
            // Parse IP/prefix
            if let Some((ip, prefix)) = parse_cidr(cidr) {
                net_config::iface_configure(iface_idx, &ip, prefix);
                println!("{}: inet {}/{}", iface_name, ip, prefix);
                net_config::save_config();
            } else {
                eprintln!("Invalid CIDR address (e.g. 10.0.0.1/24).");
            }
        }
        _ => {
            eprintln!("Usage: ifconfig <iface> [<ip>/<prefix> | up | down | vlan <id|off>]");
        }
    }
}

fn find_iface(name: &str) -> i32 {
    let count = net_config::iface_count();
    for i in 0..count {
        if let Some(info) = net_config::iface_info(i) {
            if let Some(n) = extract_json_str(&info, "name") {
                if n == name {
                    return i;
                }
            }
        }
    }
    -1
}

fn show_interface(index: i32) {
    if let Some(info) = net_config::iface_info(index) {
        let name = extract_json_str(&info, "name").unwrap_or("?".to_string());
        let mac = extract_json_str(&info, "mac").unwrap_or("??:??:??:??:??:??".to_string());
        let link_up = info.contains("\"link_up\":true");
        let flags = if link_up { "UP" } else { "DOWN" };

        println!("{}: flags=<{}>  mtu 1500", name, flags);
        println!("      ether {}", mac);

        if let Some(ip) = extract_json_str(&info, "ip") {
            if !ip.is_empty() && ip != "0.0.0.0" {
                let prefix = extract_json_int(&info, "prefix").unwrap_or(0);
                println!("      inet {}/{}", ip, prefix);
            }
        }

        if let Some(vlan) = extract_json_int(&info, "vlan") {
            if vlan >= 0 {
                println!("      vlan {}", vlan);
            }
        }

        println!();
    }
}

fn parse_cidr(s: &str) -> Option<(String, u8)> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 { return None; }
    let prefix: u8 = parts[1].parse().ok()?;
    // Validate IP has 4 octets
    let octets: Vec<&str> = parts[0].split('.').collect();
    if octets.len() != 4 { return None; }
    for o in &octets {
        let _: u8 = o.parse().ok()?;
    }
    Some((parts[0].to_string(), prefix))
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
