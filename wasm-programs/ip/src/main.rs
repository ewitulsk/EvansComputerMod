extern crate ecm_host_abi;
use ecm_host_abi::net_config;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: ip addr | ip route | ip link");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "addr" | "address" => cmd_ip_addr(&args[2..]),
        "route" => cmd_ip_route(&args[2..]),
        "link" => cmd_ip_link(&args[2..]),
        _ => {
            eprintln!("Usage: ip addr | ip route | ip link");
            std::process::exit(1);
        }
    }
}

fn cmd_ip_addr(args: &[String]) {
    if args.is_empty() || args[0] == "show" {
        let count = net_config::iface_count();
        for i in 0..count {
            show_interface(i);
        }
    } else if args[0] == "add" {
        // ip addr add 10.0.0.1/24 dev eth0
        if args.len() < 4 || args[2] != "dev" {
            eprintln!("Usage: ip addr add <ip>/<prefix> dev <iface>");
            return;
        }
        let cidr = &args[1];
        let dev = &args[3];
        if let Some((ip, prefix)) = parse_cidr(cidr) {
            let idx = find_iface(dev);
            if idx < 0 {
                eprintln!("Unknown interface.");
                return;
            }
            net_config::iface_configure(idx, &ip, prefix);
            println!("Added {}/{} to {}", ip, prefix, dev);
            net_config::save_config();
        } else {
            eprintln!("Invalid CIDR address.");
        }
    } else if args[0] == "del" {
        if args.len() < 4 || args[2] != "dev" {
            eprintln!("Usage: ip addr del <ip>/<prefix> dev <iface>");
            return;
        }
        let dev = &args[3];
        let idx = find_iface(dev);
        if idx < 0 {
            eprintln!("Unknown interface.");
            return;
        }
        net_config::iface_deconfigure(idx);
        println!("Removed address from {}", dev);
        net_config::save_config();
    } else {
        eprintln!("Usage: ip addr [show|add|del]");
    }
}

fn cmd_ip_route(args: &[String]) {
    if args.is_empty() || args[0] == "show" {
        if let Some(routes_json) = net_config::route_list() {
            // Parse JSON array of routes
            let mut found = false;
            for route in routes_json.split('{').skip(1) {
                found = true;
                let dest = extract_json_str(route, "dest").unwrap_or_default();
                let prefix = extract_json_int(route, "prefix").unwrap_or(0);
                let gateway = extract_json_str(route, "gateway").unwrap_or_default();
                let iface_idx = extract_json_int(route, "iface_idx").unwrap_or(0);
                let iface_name = get_iface_name(iface_idx);

                if prefix == 0 && (dest == "0.0.0.0" || dest.is_empty()) {
                    println!("default via {} dev {}", gateway, iface_name);
                } else if gateway == "0.0.0.0" || gateway.is_empty() {
                    println!("{}/{} dev {} scope link", dest, prefix, iface_name);
                } else {
                    println!("{}/{} via {} dev {}", dest, prefix, gateway, iface_name);
                }
            }
            if !found {
                println!("No routes configured.");
            }
        } else {
            println!("No routes configured.");
        }
    } else if args[0] == "add" {
        // ip route add default via 10.0.0.1 dev eth0
        // ip route add 192.168.1.0/24 via 10.0.0.1 dev eth0
        if args.len() < 2 {
            eprintln!("Usage: ip route add <dest>/<prefix>|default via <gw> dev <iface>");
            return;
        }
        let dest_str = &args[1];
        let mut gw = "0.0.0.0".to_string();
        let mut dev = String::new();

        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "via" if i + 1 < args.len() => { gw = args[i + 1].clone(); i += 2; }
                "dev" if i + 1 < args.len() => { dev = args[i + 1].clone(); i += 2; }
                _ => { i += 1; }
            }
        }

        let iface_idx = if !dev.is_empty() { find_iface(&dev) } else { 0 };
        if iface_idx < 0 {
            eprintln!("Unknown interface.");
            return;
        }

        if dest_str == "default" {
            net_config::route_add("0.0.0.0", 0, &gw, iface_idx);
            println!("Default route added.");
        } else if let Some((dest, prefix)) = parse_cidr(dest_str) {
            net_config::route_add(&dest, prefix, &gw, iface_idx);
            println!("Route {}/{} added.", dest, prefix);
        } else {
            eprintln!("Invalid route destination.");
        }
        net_config::save_config();
    } else if args[0] == "del" {
        if args.len() < 2 {
            eprintln!("Usage: ip route del <dest>/<prefix>|default");
            return;
        }
        if args[1] == "default" {
            net_config::route_del("0.0.0.0", 0);
            println!("Default route deleted.");
        } else if let Some((dest, prefix)) = parse_cidr(&args[1]) {
            net_config::route_del(&dest, prefix);
            println!("Route deleted.");
        } else {
            eprintln!("Invalid route.");
        }
        net_config::save_config();
    } else {
        eprintln!("Usage: ip route [show|add|del]");
    }
}

fn cmd_ip_link(args: &[String]) {
    if args.is_empty() || args[0] == "show" {
        let count = net_config::iface_count();
        for i in 0..count {
            if let Some(info) = net_config::iface_info(i) {
                let name = extract_json_str(&info, "name").unwrap_or("?".to_string());
                let mac = extract_json_str(&info, "mac").unwrap_or("??".to_string());
                let link_up = info.contains("\"link_up\":true");
                let flags = if link_up { "UP" } else { "DOWN" };
                println!("{}: <{}> mtu 1500", name, flags);
                println!("    link/ether {}", mac);
            }
        }
    } else if args[0] == "set" {
        // ip link set eth0 up/down
        if args.len() < 3 {
            eprintln!("Usage: ip link set <iface> up|down");
            return;
        }
        let dev = &args[1];
        let idx = find_iface(dev);
        if idx < 0 {
            eprintln!("Unknown interface.");
            return;
        }
        match args[2].as_str() {
            "up" => { net_config::iface_set_link(idx, true); println!("Link up."); }
            "down" => { net_config::iface_set_link(idx, false); println!("Link down."); }
            _ => eprintln!("Usage: ip link set <iface> up|down"),
        }
        net_config::save_config();
    } else {
        eprintln!("Usage: ip link [show|set]");
    }
}

fn show_interface(index: i32) {
    if let Some(info) = net_config::iface_info(index) {
        let name = extract_json_str(&info, "name").unwrap_or("?".to_string());
        let mac = extract_json_str(&info, "mac").unwrap_or("??".to_string());
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
        println!();
    }
}

fn find_iface(name: &str) -> i32 {
    let count = net_config::iface_count();
    for i in 0..count {
        if let Some(info) = net_config::iface_info(i) {
            if let Some(n) = extract_json_str(&info, "name") {
                if n == name { return i; }
            }
        }
    }
    -1
}

fn get_iface_name(index: i32) -> String {
    net_config::iface_info(index)
        .and_then(|info| extract_json_str(&info, "name"))
        .unwrap_or_else(|| "?".to_string())
}

fn parse_cidr(s: &str) -> Option<(String, u8)> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 { return None; }
    let prefix: u8 = parts[1].parse().ok()?;
    let octets: Vec<&str> = parts[0].split('.').collect();
    if octets.len() != 4 { return None; }
    for o in &octets { let _: u8 = o.parse().ok()?; }
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
