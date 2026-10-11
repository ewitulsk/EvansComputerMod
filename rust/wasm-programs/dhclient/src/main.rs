//! dhclient: drive the kernel's DHCP client.
//!
//! ```text
//! dhclient [-t <secs>] <iface>   start the client and wait for a lease (default 15 s)
//! dhclient -r <iface>            release the lease (DHCPRELEASE) and stop
//! dhclient -x <iface>            stop without releasing
//! dhclient -s <iface>            show state, lease, router and DNS
//! ```
//! The kernel keeps the client running (renewing, rebinding) after
//! `dhclient` exits, and `ifconfig <iface> dhcp` uses the same client. The
//! leased address, default route and DNS server are applied by the kernel;
//! `resolvectl`, `nslookup` and every program resolving names use that DNS
//! server. The setting is saved in network.cfg (`iface <iface> dhcp`).

use std::thread::sleep;
use std::time::{Duration, Instant};

use ecm_host_abi::dhcp::{self, DhcpError, DhcpInfo};
use ecm_host_abi::netlink::{dhcp_state_name, DHCP_STATE_BOUND, DHCP_STATE_OFF};

const USAGE: &str = "Usage: dhclient [-t secs] <iface> | -r <iface> | -x <iface> | -s <iface>";
const POLL: Duration = Duration::from_millis(250);

fn ip(a: Option<[u8; 4]>) -> String {
    match a {
        Some(b) => format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]),
        None => "-".to_string(),
    }
}

fn fail(iface: &str, e: DhcpError) -> ! {
    match e {
        DhcpError::NoSuchInterface => eprintln!("dhclient: no interface {}", iface),
        DhcpError::NotRunning => eprintln!("dhclient: no DHCP client running on {}", iface),
        DhcpError::Io => eprintln!("dhclient: netlink request failed"),
        DhcpError::Errno(n) => eprintln!("dhclient: {}: error {}", iface, n),
    }
    std::process::exit(1);
}

fn show(iface: &str, s: &DhcpInfo) {
    println!("{}: state {}", iface, dhcp_state_name(s.state));
    if s.state == DHCP_STATE_OFF {
        println!("  DHCP client not running");
        return;
    }
    if let Some(a) = s.address {
        println!("  lease   {}/{}  expires in {}s", ip(Some(a)), s.prefix, s.expires_in);
        println!("  renew   in {}s, rebind in {}s", s.renew_in, s.rebind_in);
        println!("  router  {}", ip(s.router));
        println!("  dns     {}", ip(s.dns));
    } else {
        println!("  no lease");
    }
    println!("  server  {}", ip(s.server));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut wait_secs: u64 = 15;
    let mut mode = "start";
    let mut iface: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-r" => mode = "release",
            "-x" => mode = "stop",
            "-s" => mode = "status",
            "-t" => {
                i += 1;
                match args.get(i).and_then(|v| v.parse().ok()) {
                    Some(v) => wait_secs = v,
                    None => {
                        eprintln!("{}", USAGE);
                        std::process::exit(2);
                    }
                }
            }
            "-h" | "--help" => {
                println!("{}", USAGE);
                return;
            }
            s if s.starts_with('-') => {
                eprintln!("{}", USAGE);
                std::process::exit(2);
            }
            s => iface = Some(s.to_string()),
        }
        i += 1;
    }
    let Some(iface) = iface else {
        eprintln!("{}", USAGE);
        std::process::exit(2);
    };

    match mode {
        "status" => match dhcp::status(&iface) {
            Ok(s) => show(&iface, &s),
            Err(e) => fail(&iface, e),
        },
        "release" => {
            let before = dhcp::status(&iface).unwrap_or_else(|e| fail(&iface, e));
            dhcp::release(&iface).unwrap_or_else(|e| fail(&iface, e));
            match before.address {
                Some(a) => println!("{}: released {} to {}", iface, ip(Some(a)), ip(before.server)),
                None => println!("{}: DHCP client stopped (no lease to release)", iface),
            }
        }
        "stop" => {
            dhcp::stop(&iface).unwrap_or_else(|e| fail(&iface, e));
            println!("{}: DHCP client stopped", iface);
        }
        _ => {
            if let Ok(s) = dhcp::status(&iface) {
                if s.state == DHCP_STATE_BOUND {
                    println!("{}: already bound", iface);
                    show(&iface, &s);
                    return;
                }
            }
            dhcp::start(&iface).unwrap_or_else(|e| fail(&iface, e));
            println!("{}: DHCP client started, waiting up to {}s for a lease", iface, wait_secs);
            let t0 = Instant::now();
            let mut last = u32::MAX;
            loop {
                let s = dhcp::status(&iface).unwrap_or_else(|e| fail(&iface, e));
                if s.state != last {
                    last = s.state;
                    println!("{}: {}", iface, dhcp_state_name(s.state));
                }
                if s.state == DHCP_STATE_OFF {
                    eprintln!("dhclient: the client on {} was stopped", iface);
                    std::process::exit(1);
                }
                if s.state == DHCP_STATE_BOUND && s.address.is_some() {
                    println!(
                        "bound to {}/{} -- renewal in {} seconds.",
                        ip(s.address),
                        s.prefix,
                        s.renew_in
                    );
                    println!("  router {}, dns {}, server {}", ip(s.router), ip(s.dns), ip(s.server));
                    return;
                }
                if t0.elapsed() >= Duration::from_secs(wait_secs) {
                    eprintln!(
                        "dhclient: no lease on {} after {}s (state {}); still trying in the background",
                        iface,
                        wait_secs,
                        dhcp_state_name(s.state)
                    );
                    std::process::exit(1);
                }
                sleep(POLL);
            }
        }
    }
}
