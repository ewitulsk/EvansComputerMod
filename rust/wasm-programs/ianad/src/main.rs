//! ianad: IANA's TLD desk (docs/DNS.md "Adding TLDs"), on the computer that keeps the root
//! zone. Applications come over HTTP (`POST /api`, used by `domain tld ...`; a page at `/`):
//! the label is checked, every nameserver must answer authoritatively for the TLD (the
//! technical evaluation), then the root zone gets the delegation, its serial goes up and the
//! local `named` reloads it and NOTIFYs the root letters (`ecm_dns::iana`).
//!
//! It is also the numbering administrator (docs/DNS.md "ENUM", `ecm_dns::numbering`): it
//! keeps the `e164.arpa` zone in the same `named` (adding the zone to `named.conf` and its
//! delegation to the root zone when they are missing), grants number blocks
//! (`domain enum apply`) and ports numbers (`domain enum port`).
//!
//! `ianad [-c FILE]` runs it (`ianad &`). `/etc/ianad.conf`: `zone /etc/named/root.zone`,
//! `port 80`, `db /etc/ianad.db`, `enum-zone /etc/named/e164.arpa.zone`, `enum-ns iana.ring`
//! (the zone's nameserver: this computer's name), `named-conf /etc/named.conf`, `cc 1`.
//! At the IANA console: `ianad list`, `ianad revoke TLD` (what an operator does;
//! `/ecm dns tld remove TLD` types it here), `ianad enum`, `ianad enum-revoke BLOCK`.

use ecm_dns::iana::{self, Iana};
use ecm_dns::numbering::{self, Numbering};
use ecm_dns::name::{fqdn, ip_str, normalize};
use ecm_dns::registrar::form;
use ecm_dns::sys::{self, esc, now_ms};
use ecm_dns::wire::{Message, A, SOA, SRV};
use ecm_dns::zone::Zone;
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, POLLIN};
use std::collections::BTreeMap;
use std::io::Write;

struct Conf {
    zone: String,
    port: u16,
    db: String,
    enum_zone: String,
    enum_ns: String,
    named_conf: String,
    cc: String,
}

fn log(text: &str) {
    let t = now_ms() / 1000;
    println!("[{:02}:{:02}:{:02}] ianad: {}", (t / 3600) % 24, (t / 60) % 60, t % 60, text);
    let _ = std::io::stdout().flush();
}

fn conf(path: &str) -> Result<Conf, String> {
    let text = std::fs::read_to_string(path).map_err(|_| format!("cannot read {path}"))?;
    let mut c = Conf {
        zone: "/etc/named/root.zone".into(),
        port: 80,
        db: "/etc/ianad.db".into(),
        enum_zone: numbering::ZONE_FILE.into(),
        enum_ns: "iana.ring".into(),
        named_conf: ecm_dns::conf::PATH.into(),
        cc: "1".into(),
    };
    for line in text.lines() {
        let w: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
        match w.as_slice() {
            [] => {}
            ["zone", f] => c.zone = f.to_string(),
            ["port", p] => c.port = p.parse().map_err(|_| format!("bad port '{p}'"))?,
            ["db", f] => c.db = f.to_string(),
            ["enum-zone", f] => c.enum_zone = f.to_string(),
            ["enum-ns", n] => c.enum_ns = normalize(n).ok_or_else(|| format!("bad enum-ns '{n}'"))?,
            ["named-conf", f] => c.named_conf = f.to_string(),
            ["cc", d] if (1..=3).contains(&d.len()) && d.bytes().all(|b| b.is_ascii_digit()) && !d.starts_with('0') => c.cc = d.to_string(),
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

fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    ecm_host_abi::random::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn salt() -> String {
    random_hex(8)
}

/// Write `e164.arpa` (serial one past the file's) and the database, make sure `named`
/// serves the zone and the root delegates it, then have named reload. Returns the serial.
fn publish_enum(c: &Conf, i: &Iana) -> Result<u32, String> {
    let old = sys::read_file(&c.enum_zone).and_then(|t| Zone::parse(&t, ecm_dns::enum_dns::E164_ARPA).ok()).map_or(0, |z| z.serial());
    let z = i.numbering.zone(old.wrapping_add(1), &c.enum_ns);
    let header = "; e164.arpa: telephone numbers (ENUM), kept by IANA as numbering administrator (ianad edits it).\n";
    if !sys::write_file(&c.enum_zone, &(header.to_string() + &z.to_text())) || !sys::write_file(&c.db, &i.render()) {
        return Err("cannot write the e164.arpa zone".into());
    }
    let conf = sys::read_file(&c.named_conf).unwrap_or_default();
    if let Some(nc) = numbering::named_conf_with_zone(&conf, &c.enum_zone) {
        if !sys::write_file(&c.named_conf, &nc) {
            return Err(format!("cannot write {}", c.named_conf));
        }
        log(&format!("{}: added zone e164.arpa", c.named_conf));
    }
    if let Some(nr) = numbering::delegate_root(&root(c)?, &c.enum_ns) {
        let header = "; The root zone of the Tech Village ring, kept by IANA (ianad edits it).\n";
        if !sys::write_file(&c.zone, &(header.to_string() + &nr.to_text())) {
            return Err("cannot write the root zone".into());
        }
        log(&format!("root zone: e164.arpa delegated to {} (serial {})", fqdn(&c.enum_ns), nr.serial()));
    }
    if let Err(e) = sys::control("reload", ecm_dns::conf::CONTROL_PORT) {
        log(&format!("named reload: {e}"));
    }
    Ok(z.serial())
}

/// The technical check of a block application: the domain's SIP border (or its address)
/// resolves.
fn sip_check(domain: &str) -> Result<String, String> {
    let server = sys::default_server().ok_or("no DNS server (/etc/resolv.conf) to check the domain with")?;
    let ask = |name: &str, t: u16| sys::query(server, 53, &Message::query(sys::random_id(), name, t, true), 2500).map(|a| a.msg);
    numbering::evaluate(domain, ask(&format!("_sip._udp.{domain}"), SRV), ask(domain, A))
}

fn enum_api(c: &Conf, i: &mut Iana, op: &str, f: &BTreeMap<String, String>, now: u64) -> Result<String, String> {
    let arg = |k: &str| f.get(k).map(|s| s.trim().to_string()).unwrap_or_default();
    match op {
        "enum-list" => Ok(i.numbering.list()),
        "enum-apply" => {
            let mut n: Numbering = i.numbering.clone();
            let (msg, _) = n.apply(&c.cc, &arg("block"), &arg("domain"), &arg("key"), &random_hex(16), now)?;
            let found = sip_check(&numbering::check_domain(&arg("domain"))?)?;
            i.numbering = n;
            let serial = publish_enum(c, i)?;
            Ok(format!("{msg}. Checked {found}; e164.arpa serial {serial}"))
        }
        "enum-port" => {
            let msg = i.numbering.port(&c.cc, &arg("number"), &arg("domain"), &arg("code"), &arg("key"), now)?;
            let serial = publish_enum(c, i)?;
            Ok(format!("{msg}; e164.arpa serial {serial}"))
        }
        o => Err(format!("unknown op '{o}'")),
    }
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
            "enum-apply" | "enum-port" | "enum-list" => enum_api(c, &mut i, op, f, now),
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
            o => Err(format!("unknown op '{o}' (apply, show, list, revoke, enum-apply, enum-port, enum-list)")),
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
     authoritatively. One TLD per account. <code>domain tld show base</code>, <code>domain tld revoke base</code>.</p>\n\
     <h2>Telephone numbers (ENUM)</h2>\n\
     <p>IANA is also the numbering administrator: a carrier applies for a block of 10,000 numbers (one NPA-NXX) for its SIP \
     domain, once <code>_sip._udp.</code> SRV or the domain's address resolves, and gets a key (shown once) for <code>cored enum-key</code>:</p>\n\
     <pre>domain enum apply +1214555 cell-red.ecm</pre>\n\
     <p>A subscriber moving to another carrier gets a port-out code from the old one (<code>cored port-code</code>); the new \
     carrier repoints the number: <code>domain enum port +12145550142 cell-blue.ecm CODE KEY</code>. <code>domain enum list</code>. \
     IANA publishes it all under <code>e164.arpa</code> (<code>dig -t NAPTR 5.5.5.4.1.2.1.e164.arpa</code>).</p>\n</body></html>\n"
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
        ["enum"] => {
            println!("{}", db(&c).numbering.list());
            return;
        }
        ["enum-revoke", block] => {
            // The operator at the IANA console.
            let mut i = db(&c);
            let r = i.numbering.revoke(&c.cc, block).and_then(|m| publish_enum(&c, &i).map(|s| format!("{m}; e164.arpa serial {s}")));
            match r {
                Ok(m) => println!("ianad: {m}"),
                Err(e) => {
                    eprintln!("ianad: {e}");
                    std::process::exit(1);
                }
            }
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
            println!("       ianad enum             number blocks and ported numbers (e164.arpa)");
            println!("       ianad enum-revoke BLOCK  take a number block back (operator)");
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
    match publish_enum(&c, &db(&c)) {
        Ok(s) => log(&format!("e164.arpa serial {s} ({})", c.enum_zone)),
        Err(e) => log(&format!("e164.arpa: {e}")),
    }
    log(&format!("TLD and number block applications on port {} (root zone {}). Ctrl+T to stop.", c.port, c.zone));
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
                let what = ["tld", "block", "number"].iter().find_map(|k| f.get(*k)).map(String::as_str).unwrap_or("");
                // never log a key (the reply to a domain's first block carries one)
                let shown = match out.split_once("; its ENUM key is") {
                    Some((head, _)) => format!("{head} (key issued)"),
                    None => out.clone(),
                };
                log(&format!("{} {} from {}: {}", f.get("op").map(String::as_str).unwrap_or("?"), what, peer.to_string(), shown));
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
