//! The program itself: sockets and `/etc/resolv.conf` (wasm32 only).

use crate::format;
use ecm_dns::name::{fqdn, ip_str, normalize, Ip};
use ecm_dns::resolver::{Config, Resolver};
use ecm_dns::sys;
use ecm_dns::wire::*;

const TIMEOUT_MS: i64 = 3000;

struct Opts {
    server: Option<String>,
    name: Option<String>,
    qtype: u16,
    trace: bool,
    recurse: bool,
    short: bool,
    tcp: bool,
    port: u16,
}

fn usage() {
    println!("{}", format::USAGE);
}

pub fn main() {
    let mut o = Opts { server: None, name: None, qtype: A, trace: false, recurse: true, short: false, tcp: false, port: 53 };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-h" | "--help" => return usage(),
            "+trace" => o.trace = true,
            "+norecurse" | "+norec" => o.recurse = false,
            "+recurse" => o.recurse = true,
            "+short" => o.short = true,
            "+tcp" | "+vc" => o.tcp = true,
            "-p" => {
                i += 1;
                o.port = args.get(i).and_then(|p| p.parse().ok()).unwrap_or(53);
            }
            "-t" => {
                i += 1;
                o.qtype = args.get(i).and_then(|t| type_from_name(t)).unwrap_or(A);
            }
            _ if a.starts_with('@') => o.server = Some(a[1..].to_string()),
            _ if a.starts_with('+') => eprintln!("dig: ignoring unknown option {a}"),
            _ => {
                if o.name.is_some() {
                    match type_from_name(a) {
                        Some(t) => o.qtype = t,
                        None => {
                            eprintln!("dig: unknown type '{a}'");
                            std::process::exit(1);
                        }
                    }
                } else if type_from_name(a).is_some_and(|t| t != A) && a.chars().all(|c| c.is_ascii_uppercase()) && i + 1 < args.len() {
                    // `dig MX name`
                    o.qtype = type_from_name(a).unwrap();
                } else {
                    o.name = Some(a.to_string());
                }
            }
        }
        i += 1;
    }
    let Some(raw) = o.name.clone() else {
        usage();
        std::process::exit(1);
    };
    let Some(name) = normalize(&raw) else {
        eprintln!("dig: '{raw}' is not a valid name");
        std::process::exit(1);
    };
    let server = match &o.server {
        Some(s) => match sys::server_addr(s) {
            Ok(ip) => Some(ip),
            Err(e) => {
                eprintln!("dig: {e}");
                std::process::exit(1);
            }
        },
        None => sys::default_server(),
    };
    if o.trace {
        std::process::exit(trace(&o, &name, server));
    }
    let Some(server) = server else {
        eprintln!("dig: no DNS server: give @SERVER or set a nameserver in /etc/resolv.conf");
        std::process::exit(1);
    };
    if o.qtype == AXFR {
        std::process::exit(axfr(server, &name));
    }
    let q = Message::query(sys::random_id(), &name, o.qtype, o.recurse);
    let answer = if o.tcp {
        let start = std::time::Instant::now();
        sys::tcp_query(server, o.port, &q, TIMEOUT_MS).and_then(|mut v| {
            v.pop().map(|m| sys::Answer { bytes: m.encode().len(), msg: m, ms: start.elapsed().as_millis() as i64, tcp: true }).ok_or("no answer".into())
        })
    } else {
        sys::query(server, o.port, &q, TIMEOUT_MS)
    };
    match answer {
        Ok(a) => {
            if o.short {
                for l in format::short(&a.msg) {
                    println!("{l}");
                }
            } else {
                print_full(&a.msg, server, o.port, &a);
            }
            if a.msg.header.rcode == SERVFAIL || a.msg.header.rcode == REFUSED {
                std::process::exit(1);
            }
        }
        Err(e) => {
            println!(";; connection timed out; {e}");
            std::process::exit(9);
        }
    }
}

fn print_full(m: &Message, server: Ip, port: u16, a: &sys::Answer) {
    for l in format::message(m) {
        println!("{l}");
    }
    println!();
    println!(";; Query time: {} msec", a.ms);
    println!(";; SERVER: {}#{}({}){}", ip_str(server), port, ip_str(server), if a.tcp { " (TCP)" } else { "" });
    println!(";; MSG SIZE  rcvd: {}", a.bytes);
}

fn axfr(server: Ip, zone: &str) -> i32 {
    match sys::axfr(server, zone, 10_000) {
        Ok(z) => {
            for l in format::axfr(&z) {
                println!("{l}");
            }
            println!(";; XFR size: {} records, serial {}", z.records.len() + 1, z.serial());
            println!(";; SERVER: {}#53({})", ip_str(server), ip_str(server));
            0
        }
        Err(e) => {
            println!("; Transfer failed: {e}");
            1
        }
    }
}

/// Root servers, the way dig finds them: ask the resolver for `. NS` and the
/// addresses of those names; without a resolver, `/etc/named/root.hints`.
fn root_servers(server: Option<Ip>) -> (Vec<(String, Ip)>, Option<(Message, sys::Answer, Ip)>) {
    if let Some(s) = server {
        let q = Message::query(sys::random_id(), "", NS, true);
        if let Ok(a) = sys::query(s, 53, &q, TIMEOUT_MS) {
            let mut out = Vec::new();
            for ns in a.msg.answers.iter().filter(|r| r.rtype == NS) {
                let Some(t) = ns.data.target() else { continue };
                let glue: Vec<Ip> = a.msg.additional.iter().filter(|r| r.name == t).filter_map(|r| r.a()).collect();
                let addrs = if glue.is_empty() {
                    let q = Message::query(sys::random_id(), t, A, true);
                    sys::query(s, 53, &q, TIMEOUT_MS).map(|r| r.msg.answers.iter().filter_map(|r| r.a()).collect()).unwrap_or_default()
                } else {
                    glue
                };
                for ip in addrs {
                    out.push((t.to_string(), ip));
                }
            }
            if !out.is_empty() {
                let m = a.msg.clone();
                return (out, Some((m, a, s)));
            }
        }
    }
    let hints = std::fs::read_to_string("/etc/named/root.hints")
        .ok()
        .and_then(|t| ecm_dns::resolver::parse_hints(&t).ok())
        .unwrap_or_default();
    (hints, None)
}

fn trace(o: &Opts, name: &str, server: Option<Ip>) -> i32 {
    let (hints, first) = root_servers(server);
    if hints.is_empty() {
        eprintln!("dig: cannot find the root servers (no resolver answered '. NS' and there is no /etc/named/root.hints)");
        return 9;
    }
    match &first {
        Some((m, a, s)) => {
            for r in m.answers.iter().filter(|r| r.rtype == NS) {
                println!("{}", r.to_columns());
            }
            println!(";; Received {} bytes from {}#53({}) in {} ms", a.bytes, ip_str(*s), ip_str(*s), a.ms);
        }
        None => {
            for (n, ip) in &hints {
                println!("{:<23} {:>6} IN {:<5} {}   ; {}", ".", 3600000, "NS", fqdn(n), ip_str(*ip));
            }
            println!(";; from /etc/named/root.hints");
        }
    }
    println!();
    let mut res = Resolver::new(Config { hints, timeout_ms: 1500, ..Default::default() }, sys::random_id() as u32 | 0x10000);
    let q = res.resolve_fresh(name, o.qtype, sys::now_ms());
    let r = match sys::drive(&mut res, q, 20_000) {
        Ok(r) => r,
        Err(e) => {
            println!(";; {e}");
            return 9;
        }
    };
    for s in &r.steps {
        let who = format!("{}#53({})", ip_str(s.server), if s.fallback { "fallback".to_string() } else { s.server_name.clone() });
        let Some(m) = &s.response else {
            println!(";; no answer from {} in {} ms", who, s.ms);
            println!();
            continue;
        };
        let mut shown = 0;
        for rec in m.answers.iter().chain(m.authority.iter().filter(|a| a.rtype == NS || a.rtype == SOA)) {
            println!("{}", rec.to_columns());
            shown += 1;
        }
        if shown == 0 || m.header.rcode != NOERROR {
            println!(";; status: {}{}", rcode_name(m.header.rcode), if m.header.aa { ", authoritative" } else { "" });
        }
        println!(";; Received {} bytes from {} in {} ms", s.bytes, who, s.ms);
        println!();
    }
    match r.rcode {
        NOERROR if !r.answers.is_empty() => 0,
        NOERROR => {
            println!(";; {} has no {} records (NODATA)", fqdn(name), type_name(o.qtype));
            0
        }
        NXDOMAIN => {
            println!(";; {} does not exist (NXDOMAIN)", fqdn(name));
            0
        }
        rc => {
            println!(";; {}: {}", rcode_name(rc), if r.reason.is_empty() { "no usable answer" } else { &r.reason });
            1
        }
    }
}
