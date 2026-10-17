//! ianad: IANA's TLD desk (docs/DNS.md "Adding TLDs"), on the computer that keeps the root
//! zone. Applications come over HTTP (`POST /api`, used by `domain tld ...`; a page at `/`):
//! the label is checked, every nameserver must answer authoritatively for the TLD (the
//! technical evaluation), then the root zone gets the delegation, its serial goes up and the
//! local `named` reloads it and NOTIFYs the root letters (`ecm_dns::iana`).
//!
//! `ianad [-c FILE]` runs it (`ianad &`). `/etc/ianad.conf`: `zone /etc/named/root.zone`,
//! `port 80`, `db /etc/ianad.db`. At the IANA console: `ianad list`, `ianad revoke TLD`
//! (what an operator does; `/ecm dns tld remove TLD` types it here).

use ecm_dns::iana::{self, Iana};
use ecm_dns::name::{fqdn, ip_str};
use ecm_dns::registrar::form;
use ecm_dns::sys::{self, esc, now_ms};
use ecm_dns::wire::{Message, SOA};
use ecm_dns::zone::Zone;
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, POLLIN};
use std::collections::BTreeMap;
use std::io::Write;

struct Conf {
    zone: String,
    port: u16,
    db: String,
}

fn log(text: &str) {
    let t = now_ms() / 1000;
    println!("[{:02}:{:02}:{:02}] ianad: {}", (t / 3600) % 24, (t / 60) % 60, t % 60, text);
    let _ = std::io::stdout().flush();
}

fn conf(path: &str) -> Result<Conf, String> {
    let text = std::fs::read_to_string(path).map_err(|_| format!("cannot read {path}"))?;
    let mut c = Conf { zone: "/etc/named/root.zone".into(), port: 80, db: "/etc/ianad.db".into() };
    for line in text.lines() {
        let w: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
        match w.as_slice() {
            [] => {}
            ["zone", f] => c.zone = f.to_string(),
            ["port", p] => c.port = p.parse().map_err(|_| format!("bad port '{p}'"))?,
            ["db", f] => c.db = f.to_string(),
            _ => return Err(format!("{path}: cannot read '{line}'")),
        }
    }
    Ok(c)
}

fn root(c: &Conf) -> Result<Zone, String> {
    let text = std::fs::read_to_string(&c.zone).map_err(|_| format!("cannot read {}", c.zone))?;
    Zone::parse(&text, "").map_err(|e| format!("{}: {e}", c.zone))
}

fn db(c: &Conf) -> Iana {
    Iana::parse(&std::fs::read_to_string(&c.db).unwrap_or_default())
}

/// Write the root zone and the database, have named reload (it NOTIFYs the letters).
fn publish(c: &Conf, z: &Zone, i: &Iana) -> Result<(), String> {
    let header = "; The root zone of the Tech Village ring, kept by IANA (ianad edits it).\n";
    if !sys::write_file(&c.zone, &(header.to_string() + &z.to_text())) || !sys::write_file(&c.db, &i.render()) {
        return Err("cannot write the root zone".into());
    }
    if let Err(e) = sys::control("reload", ecm_dns::conf::CONTROL_PORT) {
        log(&format!("named reload: {e}"));
    }
    Ok(())
}

fn salt() -> String {
    let mut b = [0u8; 8];
    ecm_host_abi::random::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The technical evaluation: ask every nameserver for the TLD's SOA.
fn evaluate(tld: &str, ns: &[(String, Option<[u8; 4]>)]) -> Result<(), String> {
    for (name, glue) in ns {
        let addr = match glue {
            Some(ip) => *ip,
            None => sys::server_addr(name).map_err(|e| format!("{}: {e}: the nameserver is lame", fqdn(name)))?,
        };
        let q = Message::query(sys::random_id(), tld, SOA, false);
        let answer = sys::query(addr, 53, &q, 2000).map(|a| a.msg);
        iana::evaluate(tld, &format!("{} ({})", fqdn(name), ip_str(addr)), answer)?;
    }
    Ok(())
}

fn api(c: &Conf, f: &BTreeMap<String, String>) -> String {
    let op = f.get("op").map(String::as_str).unwrap_or("");
    let now = (now_ms() / 1000) as u64;
    let r: Result<String, String> = (|| {
        let mut i = db(c);
        let z = root(c)?;
        let tld_arg = f.get("tld").cloned().unwrap_or_default().trim_start_matches('.').to_ascii_lowercase();
        match op {
            "show" => Ok(iana::show(&z, &i, &tld_arg).trim_end().to_string()),
            "list" => {
                let mut s = format!("{} TLD(s) delegated by IANA", i.tlds.len());
                for (t, (h, _)) in &i.tlds {
                    s.push_str(&format!("\n.{t} ({h})"));
                }
                Ok(s)
            }
            "apply" => {
                let h = f.get("handle").cloned().unwrap_or_default();
                let p = f.get("password").cloned().unwrap_or_default();
                i.login(&h, &p, &salt(), now)?;
                if let Some((t, _)) = i.tlds.iter().find(|(_, (owner, _))| *owner == h) {
                    return Err(format!("{h} already runs .{t}: one TLD per account"));
                }
                let tld = iana::check_label(&tld_arg, &z)?;
                let items: Vec<&str> = f.get("ns").map(|s| s.split([',', ' ']).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
                let ns = iana::parse_ns(&tld, &items)?;
                log(&format!("application for .{tld} from {h}: checking {} nameserver(s)", ns.len()));
                evaluate(&tld, &ns)?;
                let nz = iana::delegate(&z, &tld, &ns);
                i.tlds.insert(tld.clone(), (h.clone(), now));
                publish(c, &nz, &i)?;
                Ok(format!(".{tld} is delegated to {}; root zone serial {} (the root servers have it in a moment)", ns.iter().map(|(n, _)| fqdn(n)).collect::<Vec<_>>().join(" "), nz.serial()))
            }
            "revoke" => {
                let h = f.get("handle").cloned().unwrap_or_default();
                let p = f.get("password").cloned().unwrap_or_default();
                i.login(&h, &p, &salt(), now)?;
                match i.tlds.get(&tld_arg) {
                    Some((owner, _)) if *owner == h => {}
                    Some(_) => return Err(format!(".{tld_arg} belongs to another account")),
                    None => return Err(format!(".{tld_arg} was not delegated by IANA")),
                }
                i.tlds.remove(&tld_arg);
                let nz = iana::remove(&z, &tld_arg);
                publish(c, &nz, &i)?;
                Ok(format!(".{tld_arg} removed from the root zone (serial {})", nz.serial()))
            }
            o => Err(format!("unknown op '{o}' (apply, show, list, revoke)")),
        }
    })();
    match r {
        Ok(s) => format!("OK {s}"),
        Err(e) => format!("ERR {e}"),
    }
}

fn home() -> String {
    "<!DOCTYPE html>\n<html><head><title>IANA</title></head>\n<body>\n<h1>IANA: the root zone of the Tech Village ring</h1>\n\
     <p>A new top-level domain exists when IANA puts its nameservers in the root zone. Run the TLD's nameservers yourself\
     (a <code>named</code> with the TLD's zone, at least one reachable address, two in different networks is better), then apply:</p>\n\
     <pre>domain tld apply base ns1.base=10.200.0.53 ns2.base=10.201.0.53</pre>\n\
     <p>IANA checks the name (2-63 letters, digits and hyphens, not all digits, not reserved: test, example, invalid, localhost,\
     local, internal, arpa, ring, ecm; not taken) and asks every nameserver for the TLD's SOA: each must answer\
     authoritatively. One TLD per account. <code>domain tld show base</code>, <code>domain tld revoke base</code>.</p>\n</body></html>\n"
        .to_string()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut path = "/etc/ianad.conf".to_string();
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-c" {
            i += 1;
            path = args.get(i).cloned().unwrap_or_default();
        } else {
            rest.push(args[i].clone());
        }
        i += 1;
    }
    let c = match conf(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ianad: {e}");
            std::process::exit(1);
        }
    };
    match rest.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => {}
        ["list"] => {
            println!("{}", api(&c, &BTreeMap::from([("op".to_string(), "list".to_string())])).trim_start_matches("OK "));
            return;
        }
        ["revoke", tld] => {
            // The operator at the IANA console: no account needed.
            let tld = tld.trim_start_matches('.').to_ascii_lowercase();
            let r = root(&c).and_then(|z| {
                if !z.records.iter().any(|r| r.name == tld && r.rtype == ecm_dns::wire::NS) {
                    return Err(format!(".{tld} is not in the root zone"));
                }
                let mut i = db(&c);
                i.tlds.remove(&tld);
                let nz = iana::remove(&z, &tld);
                publish(&c, &nz, &i).map(|_| nz.serial())
            });
            match r {
                Ok(s) => println!("ianad: .{tld} removed from the root zone (serial {s})"),
                Err(e) => {
                    eprintln!("ianad: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        _ => {
            println!("Usage: ianad [-c FILE]        IANA's TLD desk (ianad &)");
            println!("       ianad list             TLDs IANA delegated");
            println!("       ianad revoke TLD       take a TLD out of the root zone (operator)");
            return;
        }
    }
    let web = match sys::listen(c.port) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("ianad: {e}");
            std::process::exit(1);
        }
    };
    log(&format!("TLD applications on port {} (root zone {}). Ctrl+T to stop.", c.port, c.zone));
    loop {
        let mut fds = [PollFd { fd: web, events: POLLIN, revents: 0 }];
        if socket::poll(&mut fds, 1000) <= 0 || fds[0].revents & POLLIN == 0 {
            continue;
        }
        let mut peer = SockAddrIn::default();
        let fd = socket::accept(web, &mut peer);
        if fd < 0 {
            continue;
        }
        let Some(req) = sys::read_request(fd, 3000) else {
            socket::close(fd);
            continue;
        };
        match (req.method.as_str(), req.path.as_str()) {
            ("POST", "/api") => {
                let f = form(&req.body);
                let out = api(&c, &f);
                log(&format!("{} .{} from {}: {}", f.get("op").map(String::as_str).unwrap_or("?"), f.get("tld").map(String::as_str).unwrap_or(""), peer.to_string(), out));
                sys::reply(fd, "200 OK", "text/plain", &format!("{out}\n"));
            }
            ("GET", "/") | ("GET", "/index.html") => sys::reply(fd, "200 OK", "text/html", &home()),
            ("GET", "/tlds") => {
                let out = api(&c, &BTreeMap::from([("op".to_string(), "list".to_string())]));
                sys::reply(fd, "200 OK", "text/html", &format!("<html><body><pre>{}</pre></body></html>\n", esc(&out)));
            }
            _ => sys::reply(fd, "404 Not Found", "text/plain", "not found\n"),
        }
        socket::close(fd);
    }
}
