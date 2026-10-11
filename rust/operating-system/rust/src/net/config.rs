//! Persistent network configuration (`/network.cfg`).
//!
//! ```text
//! iface eth0 10.0.0.1/24
//! vlan eth0 100
//! down eth2
//! route default via 10.0.0.254 dev eth0
//! route 192.168.1.0/24 via 10.0.0.1 dev eth0
//! dns 10.0.0.53
//! ```
//! Loaded at boot; rewritten by the kernel whenever netlink changes the
//! configuration, so `ifconfig`/`ip` changes survive a reboot. Connected
//! routes are implied by `iface` lines and are not stored.

use ecm_net::types::Ipv4Addr;
use ecm_net::Stack;

use crate::fs;

pub const PATH: &str = "network.cfg";

pub fn load(stack: &mut Stack, now_ms: i64) -> bool {
    let Some(text) = fs::read_to_string(PATH) else {
        return false;
    };
    apply(stack, &text, now_ms);
    true
}

/// Apply only the lines that concern one interface (`wlan0` appearing after boot).
pub fn load_iface(stack: &mut Stack, name: &str, now_ms: i64) -> bool {
    let Some(text) = fs::read_to_string(PATH) else {
        return false;
    };
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| {
            let t: Vec<&str> = l.split_whitespace().collect();
            t.get(1) == Some(&name) || t.windows(2).any(|w| w[0] == "dev" && w[1] == name)
        })
        .collect();
    apply(stack, &lines.join("\n"), now_ms);
    true
}

pub fn apply(stack: &mut Stack, text: &str, now_ms: i64) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["iface", name, "dhcp"] => {
                if let Some(idx) = stack.find_iface(name) {
                    let _ = stack.start_dhcp(idx, now_ms);
                }
            }
            ["iface", name, cidr, ..] => {
                if let (Some(idx), Some((ip, prefix))) =
                    (stack.find_iface(name), Ipv4Addr::parse_cidr(cidr))
                {
                    stack.configure_addr(idx, ip, prefix, now_ms);
                }
            }
            ["vlan", name, vid, ..] => {
                if let (Some(idx), Ok(vid)) = (stack.find_iface(name), vid.parse::<u16>()) {
                    if (1..=4094).contains(&vid) {
                        stack.set_vlan(idx, Some(vid));
                    }
                }
            }
            ["down", name, ..] => {
                if let Some(idx) = stack.find_iface(name) {
                    stack.set_admin_up(idx, false, now_ms);
                }
            }
            ["dns", ip, ..] => {
                if let Some(ip) = Ipv4Addr::parse(ip) {
                    stack.set_dns_server(ip);
                }
            }
            ["route", dest, rest @ ..] => {
                let mut gw = Ipv4Addr::ZERO;
                let mut dev = None;
                let mut i = 0;
                while i + 1 < rest.len() {
                    match rest[i] {
                        "via" => gw = Ipv4Addr::parse(rest[i + 1]).unwrap_or(Ipv4Addr::ZERO),
                        "dev" => dev = stack.find_iface(rest[i + 1]),
                        _ => {
                            i += 1;
                            continue;
                        }
                    }
                    i += 2;
                }
                let Some(dev) = dev else { continue };
                let target = if *dest == "default" {
                    Some((Ipv4Addr::ZERO, 0))
                } else {
                    Ipv4Addr::parse_cidr(dest)
                };
                if let Some((d, p)) = target {
                    let _ = stack.add_route(d, p, gw, dev);
                }
            }
            _ => {}
        }
    }
}

fn fmt_ip(ip: &Ipv4Addr) -> String {
    format!("{}.{}.{}.{}", ip.0[0], ip.0[1], ip.0[2], ip.0[3])
}

/// Serialise the stack's physical-interface configuration. SVI interfaces
/// (`vlanN`) belong to the switch config and are skipped.
pub fn render(stack: &Stack) -> String {
    let mut out = String::from("# Written by the kernel. Edited via ifconfig / ip.\n");
    let mut names = Vec::new();
    for idx in 0..stack.iface_count() {
        let Some(iface) = stack.iface(idx) else {
            continue;
        };
        if !iface.name.starts_with("eth") {
            continue;
        }
        names.push((idx, iface.name.clone()));
        if stack.dhcp_enabled(idx) {
            out.push_str(&format!("iface {} dhcp\n", iface.name));
        } else if iface.ip != Ipv4Addr::ZERO {
            out.push_str(&format!(
                "iface {} {}/{}\n",
                iface.name,
                fmt_ip(&iface.ip),
                iface.prefix
            ));
        }
        if let Some(vid) = iface.vlan {
            out.push_str(&format!("vlan {} {}\n", iface.name, vid));
        }
        if !iface.admin_up {
            out.push_str(&format!("down {}\n", iface.name));
        }
    }
    for r in stack.routes() {
        // Connected routes are recreated from `iface` lines.
        if r.source != ecm_net::RouteSource::Static {
            continue;
        }
        let Some((_, dev)) = names.iter().find(|(i, _)| *i == r.iface) else {
            continue;
        };
        let dest = if r.prefix == 0 {
            "default".to_string()
        } else {
            format!("{}/{}", fmt_ip(&r.dst), r.prefix)
        };
        out.push_str(&format!(
            "route {} via {} dev {}\n",
            dest,
            fmt_ip(&r.gateway),
            dev
        ));
    }
    if stack.dns_server() != Ipv4Addr::ZERO {
        out.push_str(&format!("dns {}\n", fmt_ip(&stack.dns_server())));
    }
    out
}

pub fn save(stack: &Stack) {
    fs::write(PATH, render(stack).as_bytes());
}
