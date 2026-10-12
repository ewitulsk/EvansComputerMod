//! dhcpd: a DHCPv4 server on packet sockets.
//!
//! ```text
//! dhcpd [-c <config>] [-l <leasefile>] [-t] [iface ...]
//! ```
//! Serves every pool in the config (default `/etc/dhcpd.conf`), or only the
//! named interfaces. Leases are kept in `/var/dhcpd.leases` across restarts.
//! `-t` checks the config and exits. Runs until killed (Ctrl+T); start it
//! with `dhcpd &` to keep the shell. See `lib.rs` for the config syntax.
//!
//! Without `/etc/dhcpd.conf` (and no `-c`) it writes a reasonable one there
//! first (`dhcpd::default_config`: the interface with an address, its subnet's
//! `.100`-`.199`, itself as router, itself as DNS only if a resolver runs here,
//! one-hour leases), prints it and serves it. With `-t` it only prints it.

use std::time::{SystemTime, UNIX_EPOCH};

use dhcpd::{default_config, Config, Daemon, Iface, LinkView, DEFAULT_CONFIG, DEFAULT_LEASES};
use ecm_host_abi::dhcp as ctl;
use ecm_host_abi::socket::{self, PollFd, SockAddrLl, ETH_P_IP, MSG_DONTWAIT, POLLERR, POLLIN};
use ecm_net::{Ipv4Addr, MacAddr};

const USAGE: &str = "Usage: dhcpd [-c config] [-l leasefile] [-t] [iface ...]";

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn save(path: &str, text: &str) {
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("dhcpd: cannot write {}: {}", path, e);
    }
}

/// True if something on this computer already answers DNS (UDP port 53 is taken).
fn resolver_here() -> bool {
    let fd = socket::socket(socket::AF_INET, socket::SOCK_DGRAM, 0);
    if fd < 0 {
        return false;
    }
    let taken = socket::bind(fd, &socket::SockAddrIn::from_ip_port([0, 0, 0, 0], 53)) != 0;
    socket::close(fd);
    taken
}

/// Write a default config to [`DEFAULT_CONFIG`] (only shown with `-t`), say what and where.
fn write_default(only: &[String], check: bool) -> Option<String> {
    let links: Vec<LinkView> = ctl::links()
        .into_iter()
        .map(|l| LinkView { name: l.name, running: l.running, ip: l.ip.map(Ipv4Addr), prefix: l.prefix })
        .collect();
    let want = if only.len() == 1 { Some(only[0].as_str()) } else { None };
    let g = match default_config(&links, want, resolver_here()) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("dhcpd: no {} and no default: {}", DEFAULT_CONFIG, e);
            return None;
        }
    };
    if check {
        println!("dhcpd: no {}; dhcpd would write:", DEFAULT_CONFIG);
    } else {
        save(DEFAULT_CONFIG, &g.text);
        println!("dhcpd: no {}; wrote a default config for {} there:", DEFAULT_CONFIG, g.iface);
    }
    for l in g.text.lines() {
        println!("  {}", l);
    }
    for n in &g.notes {
        println!("dhcpd: note: {}", n);
    }
    Some(g.text)
}

fn main() {
    let mut conf = DEFAULT_CONFIG.to_string();
    let mut leases = DEFAULT_LEASES.to_string();
    let mut check = false;
    let mut only: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-c" | "-l" => {
                let Some(v) = args.next() else {
                    eprintln!("{}", USAGE);
                    std::process::exit(2);
                };
                if a == "-c" {
                    conf = v;
                } else {
                    leases = v;
                }
            }
            "-t" => check = true,
            "-h" | "--help" => {
                println!("{}", USAGE);
                return;
            }
            s if s.starts_with('-') => {
                eprintln!("{}", USAGE);
                std::process::exit(2);
            }
            s => only.push(s.to_string()),
        }
    }

    let text = match std::fs::read_to_string(&conf) {
        Ok(t) => t,
        // No config at the standard path: write a reasonable one and serve it.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && conf == DEFAULT_CONFIG => match write_default(&only, check) {
            Some(t) => t,
            None => std::process::exit(1),
        },
        Err(e) => {
            eprintln!("dhcpd: cannot read {}: {}", conf, e);
            std::process::exit(1);
        }
    };
    let cfg = match Config::parse(&text) {
        Ok(c) => c,
        Err(errs) => {
            for e in errs {
                eprintln!("dhcpd: {}: {}", conf, e);
            }
            std::process::exit(1);
        }
    };
    if check {
        println!("dhcpd: {} OK ({} pool(s), {} reservation(s))", conf, cfg.pools.len(), cfg.reservations.len());
        return;
    }

    // Open one IPv4 packet socket per served interface.
    let links = ctl::links();
    let mut ifaces: Vec<(Iface, i32)> = Vec::new();
    for p in &cfg.pools {
        if !only.is_empty() && !only.contains(&p.name) {
            continue;
        }
        let Some(link) = links.iter().find(|l| l.name == p.name) else {
            eprintln!("dhcpd: no interface {} (pool skipped)", p.name);
            continue;
        };
        let Some(server) = cfg.server_id(&p.name, link.ip.map(Ipv4Addr)) else {
            eprintln!(
                "dhcpd: {} has no address in {}/{}; add 'server {} <ip>' or a router option",
                p.name, p.start, p.prefix, p.name
            );
            continue;
        };
        let fd = socket::packet_socket(&p.name, ETH_P_IP);
        if fd < 0 {
            eprintln!("dhcpd: cannot open a packet socket on {}", p.name);
            continue;
        }
        println!(
            "dhcpd: serving {} {}-{}/{} as {} (lease {}s)",
            p.name, p.start, p.end, p.prefix, server, p.lease_secs
        );
        ifaces.push((
            Iface {
                name: p.name.clone(),
                mac: MacAddr(link.mac),
                server,
            },
            fd,
        ));
    }
    for o in &only {
        if !cfg.pools.iter().any(|p| &p.name == o) {
            eprintln!("dhcpd: no pool for {} in {}", o, conf);
        }
    }
    if ifaces.is_empty() {
        eprintln!("dhcpd: nothing to serve");
        std::process::exit(1);
    }

    let mut d = Daemon::new(cfg);
    if let Ok(t) = std::fs::read_to_string(&leases) {
        d.db.restore(&t, now_ms());
        let n = d.db.leases.len();
        if n > 0 {
            println!("dhcpd: {} lease(s) restored from {}", n, leases);
        }
    }

    let mut fds: Vec<PollFd> = ifaces
        .iter()
        .map(|(_, fd)| PollFd {
            fd: *fd,
            events: POLLIN,
            revents: 0,
        })
        .collect();
    let mut buf = [0u8; 1600];
    loop {
        for f in fds.iter_mut() {
            f.revents = 0;
        }
        let n = socket::poll(&mut fds, 1000);
        if n < 0 {
            eprintln!("dhcpd: poll failed");
            std::process::exit(1);
        }
        for (i, f) in fds.iter().enumerate() {
            if f.revents & POLLERR != 0 {
                eprintln!("dhcpd: {} went away", ifaces[i].0.name);
                std::process::exit(1);
            }
            if f.revents & POLLIN == 0 {
                continue;
            }
            loop {
                let mut from = SockAddrLl::default();
                let len = socket::recvfrom_ll(f.fd, &mut buf, MSG_DONTWAIT, &mut from);
                if len <= 0 {
                    break;
                }
                let iface = &ifaces[i].0;
                let (reply, log) = d.on_frame(iface, &buf[..len as usize], now_ms());
                for l in log {
                    println!("{}", l);
                }
                if let Some(frame) = reply {
                    if socket::send(f.fd, &frame, 0) < 0 {
                        eprintln!("dhcpd: send on {} failed", iface.name);
                    }
                }
            }
        }
        d.db.expire(now_ms());
        if d.db.dirty {
            d.db.dirty = false;
            save(&leases, &d.db.render());
        }
    }
}
