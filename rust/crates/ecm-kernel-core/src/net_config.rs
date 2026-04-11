//! Persistent network configuration stored at `network.cfg` in the
//! filesystem. Both `terminal-os` and `switch-os` call [`load`] during boot
//! to restore interface addresses, VLANs, DNS server, and static routes;
//! and the netlink handler calls [`save`] after every successful mutation so
//! changes survive kernel reloads, block break/place, and server restarts.
//!
//! File format (one directive per line, `#` comments and blanks allowed):
//!
//! ```text
//! iface eth0 10.0.0.1/24
//! iface eth1 192.168.1.1/24 vlan=100
//! dns 8.8.8.8
//! route default via 10.0.0.254 dev eth0
//! route 172.16.0.0/16 via 192.168.1.254 dev eth1
//! ```

use alloc::string::String;
use alloc::vec::Vec;
use ecm_net::types::Ipv4Addr;
use ecm_net::NetStack;

use crate::fs;

const CONFIG_PATH: &str = "network.cfg";

/// Restore persisted network config from `network.cfg` into `stack`. No-op if
/// the file doesn't exist. Unknown / malformed lines are silently skipped so
/// a partial file never wedges boot.
pub fn load(stack: &mut NetStack) -> bool {
    let raw = match fs::read_file_absolute(CONFIG_PATH) {
        Some(s) => s,
        None => return false,
    };

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        match parts[0] {
            "iface" if parts.len() >= 3 => {
                if let Some(idx) = stack.find_iface(parts[1]) {
                    if let Some((ip, prefix)) = Ipv4Addr::parse_cidr(parts[2]) {
                        stack.configure_iface(idx, ip, prefix);
                    }
                    for p in &parts[3..] {
                        if let Some(vid_str) = p.strip_prefix("vlan=") {
                            if let Ok(vid) = vid_str.parse::<u16>() {
                                stack.interfaces[idx].vlan = Some(vid);
                            }
                        }
                    }
                }
            }
            "dns" if parts.len() >= 2 => {
                if let Some(dns) = Ipv4Addr::parse(parts[1]) {
                    stack.dns_server = dns;
                }
            }
            "route" if parts.len() >= 5 => {
                let dest_str = parts[1];
                let mut gw = Ipv4Addr::ZERO;
                let mut dev = "";
                let mut i = 2;
                while i < parts.len() {
                    match parts[i] {
                        "via" if i + 1 < parts.len() => {
                            gw = Ipv4Addr::parse(parts[i + 1]).unwrap_or(Ipv4Addr::ZERO);
                            i += 2;
                        }
                        "dev" if i + 1 < parts.len() => {
                            dev = parts[i + 1];
                            i += 2;
                        }
                        _ => {
                            i += 1;
                        }
                    }
                }
                if let Some(iface_idx) = stack.find_iface(dev) {
                    if dest_str == "default" {
                        let _ = stack.routing.add_route(Ipv4Addr::ZERO, 0, gw, iface_idx);
                    } else if let Some((dest, prefix)) = Ipv4Addr::parse_cidr(dest_str) {
                        let _ = stack.routing.add_route(dest, prefix, gw, iface_idx);
                    }
                }
            }
            _ => {}
        }
    }
    true
}

/// Serialize `stack`'s current interface, DNS, and routing configuration to
/// `network.cfg`. Called from the netlink handler after mutative RTM_* ops.
///
/// Only persisted routes are the non-connected ones: connected routes are
/// rebuilt automatically by [`NetStack::configure_iface`] when the matching
/// `iface` line is replayed on boot, so emitting them would double-register.
pub fn save(stack: &NetStack) -> bool {
    let mut out = String::new();

    // Interfaces
    for i in 0..stack.iface_count {
        let iface = &stack.interfaces[i];
        if !iface.configured() && iface.vlan.is_none() {
            continue;
        }
        let name = iface.name_str();
        if iface.configured() {
            out.push_str(&alloc::format!(
                "iface {} {}/{}",
                name,
                iface.ip,
                iface.prefix_len
            ));
        } else {
            // Interface has a VLAN but no IP — still preserve the VLAN tag.
            out.push_str(&alloc::format!("iface {} 0.0.0.0/0", name));
        }
        if let Some(vid) = iface.vlan {
            out.push_str(&alloc::format!(" vlan={}", vid));
        }
        out.push('\n');
    }

    // DNS
    if stack.dns_server != Ipv4Addr::ZERO {
        out.push_str(&alloc::format!("dns {}\n", stack.dns_server));
    }

    // Routes (skip connected routes — those get reconstructed from iface lines)
    for e in stack.routing.entries.iter() {
        if !e.active {
            continue;
        }
        if e.gateway == Ipv4Addr::ZERO && e.prefix_len > 0 {
            continue; // connected route
        }
        if e.iface_index >= stack.iface_count {
            continue;
        }
        let dev = stack.interfaces[e.iface_index].name_str();
        if e.prefix_len == 0 {
            out.push_str(&alloc::format!("route default via {} dev {}\n", e.gateway, dev));
        } else {
            out.push_str(&alloc::format!(
                "route {}/{} via {} dev {}\n",
                e.destination,
                e.prefix_len,
                e.gateway,
                dev
            ));
        }
    }

    fs::write_file_absolute(CONFIG_PATH, &out)
}
