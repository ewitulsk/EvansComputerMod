//! nslookup: look a name up (docs/DNS.md).
//!
//! `nslookup [-type=TYPE] NAME [SERVER]`. Without a server it asks the first
//! `nameserver` of `/etc/resolv.conf` and applies its `search` domains, like the stub
//! resolver; without any configured server it falls back to the kernel's resolver.

use ecm_dns::name::{fqdn, ip_str, normalize};
use ecm_dns::sys;
use ecm_dns::wire::*;
use ecm_host_abi::socket;

fn usage() {
    eprintln!("Usage: nslookup [-type=A|NS|CNAME|SOA|MX|TXT|PTR|SRV|NAPTR|ANY] <name> [server]");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut qtype = A;
    let mut rest = Vec::new();
    for a in &args {
        if let Some(t) = a.strip_prefix("-type=").or_else(|| a.strip_prefix("-query=")).or_else(|| a.strip_prefix("-q=")) {
            match type_from_name(t) {
                Some(t) => qtype = t,
                None => {
                    eprintln!("nslookup: unknown type '{t}'");
                    std::process::exit(1);
                }
            }
        } else if a == "-h" || a == "--help" {
            usage();
            return;
        } else {
            rest.push(a.clone());
        }
    }
    let Some(name) = rest.first().cloned() else {
        usage();
        std::process::exit(1);
    };
    let conf = sys::resolv_conf();
    let server = match rest.get(1) {
        Some(s) => match sys::server_addr(s) {
            Ok(ip) => Some(ip),
            Err(e) => {
                eprintln!("nslookup: {e}");
                std::process::exit(1);
            }
        },
        None => conf.nameservers.first().copied(),
    };
    let Some(server) = server else {
        // No resolv.conf server: the kernel knows (network.cfg `dns`, DHCP).
        match socket::resolve(&name) {
            Ok(ip) => {
                println!("Name:    {}", name);
                println!("Address: {}", ip_str(ip));
            }
            Err(e) => {
                eprintln!("** server can't find {}: {}", name, e.describe());
                std::process::exit(1);
            }
        }
        return;
    };
    println!("Server:         {}", ip_str(server));
    println!("Address:        {}#53", ip_str(server));
    println!();
    let candidates = if rest.get(1).is_some() { vec![name.trim_end_matches('.').to_string()] } else { conf.candidates(&name) };
    let mut last_rcode = NXDOMAIN;
    for cand in candidates {
        let Some(n) = normalize(&cand) else { continue };
        let q = Message::query(sys::random_id(), &n, qtype, true);
        let a = match sys::query(server, 53, &q, 3000) {
            Ok(a) => a,
            Err(e) => {
                eprintln!(";; connection timed out; {e}");
                std::process::exit(1);
            }
        };
        let m = a.msg;
        if last_rcode != 255 {
            last_rcode = m.header.rcode;
        }
        if m.header.rcode == NXDOMAIN || (m.header.rcode == NOERROR && m.answers.is_empty()) {
            if m.header.rcode == NOERROR {
                last_rcode = 255; // NODATA: the name exists, the type does not
            }
            continue;
        }
        if m.header.rcode != NOERROR {
            break;
        }
        if !m.header.aa {
            println!("Non-authoritative answer:");
        }
        for r in &m.answers {
            match &r.data {
                RData::A(ip) => {
                    println!("Name:    {}", r.name);
                    println!("Address: {}", ip_str(*ip));
                }
                RData::Cname(t) => println!("{}  canonical name = {}", r.name, fqdn(t)),
                RData::Ns(t) => println!("{}  nameserver = {}", r.name, fqdn(t)),
                RData::Mx(p, t) => println!("{}  mail exchanger = {} {}", r.name, p, fqdn(t)),
                RData::Ptr(t) => println!("{}  name = {}", r.name, fqdn(t)),
                RData::Txt(_) => println!("{}  text = {}", r.name, r.data.to_text()),
                RData::Srv(_) => println!("{}  service = {}", r.name, r.data.to_text()),
                RData::Naptr(_) => println!("{}  naptr = {}", r.name, r.data.to_text()),
                RData::Soa(s) => {
                    println!("{}", r.name);
                    println!("  origin = {}", fqdn(&s.mname));
                    println!("  mail addr = {}", fqdn(&s.rname));
                    println!("  serial = {}", s.serial);
                    println!("  refresh = {}, retry = {}, expire = {}, minimum = {}", s.refresh, s.retry, s.expire, s.minimum);
                }
                _ => println!("{}", r.to_columns()),
            }
        }
        return;
    }
    match last_rcode {
        255 => println!("*** No {} records for {}", type_name(qtype), name),
        NXDOMAIN => println!("** server can't find {}: NXDOMAIN", name),
        rc => println!("** server can't find {}: {}", name, rcode_name(rc)),
    }
    std::process::exit(1);
}
