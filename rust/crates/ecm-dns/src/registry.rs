//! A TLD registry (`registryd`, docs/DNS.md "The registrar"): the domains under one TLD,
//! which registrar sponsors each, their nameservers or hosted records, and the zone file
//! made from them. Registrars talk to it over **EPP-lite**: one command per line on TCP 700,
//! one reply per command.
//!
//! ```text
//! LOGIN <registrar> <secret>                  OK login nic-ring
//! CHECK <domain>                              OK available | OK taken
//! CREATE <domain> <registrant>                OK created mybase.ecm serial 7
//! INFO <domain>                               OK info  (then key/value lines, then ".")
//! UPDATE <domain> RR ADD <name> <ttl> <type> <data...>
//! UPDATE <domain> RR DEL <name> <type> [data...]
//! UPDATE <domain> NS SET <ns>[=<glue>] ...    delegate to the registrant's own servers
//! UPDATE <domain> NS HOSTED                   back to records served by the registry
//! DELETE <domain>
//! LIST                                        OK list (domains of this registrar, then ".")
//! LOGOUT
//! ```
//! Errors are `ERR <code> <text>` with EPP's result codes (2001 syntax, 2200 authentication,
//! 2201 authorization, 2302 object exists, 2303 does not exist, 2306 policy, 2308 data
//! management policy).
//!
//! Policy: second-level names only (`label.tld`), labels LDH 3-63 without leading or
//! trailing hyphen, reserved names refused, records only inside the domain, TTL at least
//! [`MIN_TTL`], glue required when a nameserver is inside the domain. Every change bumps the
//! zone's serial; the program writes the zone and has `named` reload (which NOTIFYs the
//! secondaries).

use crate::name::{fqdn, ip_str, is_ldh, is_sub, normalize, parse_ip, relative, Ip};
use crate::wire::{rdata_from_text, type_from_name, type_name, RData, Record, A, CNAME, MX, TXT};
use crate::zone::{parse_ttl, Zone};
use std::collections::BTreeMap;
use std::fmt::Write as _;

pub const PORT: u16 = 700;
pub const MIN_TTL: u32 = 60;
pub const DEFAULT_TTL: u32 = 300;
pub const MAX_RECORDS: usize = 64;
pub const MAX_NS: usize = 8;
/// Labels never for sale under the public TLD.
pub const RESERVED: &[&str] = &["nic", "www", "root", "whois", "iana", "registry", "registrar", "dns", "ns1", "ns2", "example", "test", "localhost"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Domain {
    pub name: String,
    pub registrar: String,
    pub registrant: String,
    pub created: u64,
    pub updated: u64,
    /// Delegation: nameserver names and their glue (needed inside the domain).
    pub ns: Vec<(String, Option<Ip>)>,
    /// Hosted records (owner names absolute), used when `ns` is empty.
    pub records: Vec<Record>,
}

impl Domain {
    pub fn hosted(&self) -> bool {
        self.ns.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Registry {
    pub tld: String,
    pub domains: BTreeMap<String, Domain>,
    /// Registrar id -> secret.
    pub registrars: BTreeMap<String, String>,
}

/// Is `label` a name a registrant may have?
pub fn check_label(label: &str) -> Result<(), String> {
    if label.len() < 3 || label.len() > 63 {
        return Err("names are 3 to 63 characters".into());
    }
    if !is_ldh(label) {
        return Err("names use letters, digits and hyphens, not at the start or the end".into());
    }
    if RESERVED.contains(&label) || label.starts_with("v") && label[1..].parse::<u32>().is_ok_and(|n| (1..=10).contains(&n)) {
        return Err(format!("'{label}' is reserved"));
    }
    Ok(())
}

impl Registry {
    pub fn new(tld: &str) -> Registry {
        Registry { tld: tld.to_string(), ..Default::default() }
    }

    /// `label.tld` from what the registrar sent, checked.
    pub fn domain_name(&self, s: &str) -> Result<String, (u16, String)> {
        let n = normalize(s).ok_or((2005, format!("'{s}' is not a domain name")))?;
        let Some(label) = n.strip_suffix(&format!(".{}", self.tld)) else {
            return Err((2306, format!("this registry is for .{} only", self.tld)));
        };
        if label.contains('.') {
            return Err((2306, format!("register the name itself ({}.{}), not a name under it", label.rsplit('.').next().unwrap_or(label), self.tld)));
        }
        check_label(label).map_err(|e| (2306, e))?;
        Ok(n)
    }

    // ------------------------------------------------------------ persistence

    /// The database file: `tld`, `registrar`, `domain`, `ns`, `rr` lines.
    pub fn render(&self) -> String {
        let mut s = format!("# registryd database (owned by registryd)\ntld {}\n", self.tld);
        for (id, secret) in &self.registrars {
            let _ = writeln!(s, "registrar {id} {secret}");
        }
        for d in self.domains.values() {
            let _ = writeln!(s, "domain {} {} {} {} {}", d.name, d.registrar, d.registrant, d.created, d.updated);
            for (n, g) in &d.ns {
                let _ = writeln!(s, "ns {} {} {}", d.name, n, g.map(ip_str).unwrap_or_else(|| "-".into()));
            }
            for r in &d.records {
                let _ = writeln!(s, "rr {} {} {} {} {}", d.name, r.name, r.ttl, type_name(r.rtype), r.data.to_text());
            }
        }
        s
    }

    pub fn parse(text: &str) -> Result<Registry, String> {
        let mut reg = Registry::default();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let err = |m: &str| format!("registry database line {}: {m}", n + 1);
            let w: Vec<&str> = line.split_whitespace().collect();
            match w.as_slice() {
                ["tld", t] => reg.tld = t.to_string(),
                ["registrar", id, secret] => {
                    reg.registrars.insert(id.to_string(), secret.to_string());
                }
                ["domain", name, registrar, registrant, created, updated] => {
                    reg.domains.insert(
                        name.to_string(),
                        Domain {
                            name: name.to_string(),
                            registrar: registrar.to_string(),
                            registrant: registrant.to_string(),
                            created: created.parse().map_err(|_| err("bad date"))?,
                            updated: updated.parse().map_err(|_| err("bad date"))?,
                            ns: Vec::new(),
                            records: Vec::new(),
                        },
                    );
                }
                ["ns", name, ns, glue] => {
                    let d = reg.domains.get_mut(*name).ok_or_else(|| err("ns before its domain"))?;
                    d.ns.push((ns.to_string(), parse_ip(glue)));
                }
                ["rr", name, owner, ttl, t, data @ ..] => {
                    let rtype = type_from_name(t).ok_or_else(|| err("bad type"))?;
                    let ttl = ttl.parse().map_err(|_| err("bad ttl"))?;
                    let tokens = tokens(&data.join(" ")).map_err(|e| err(&e))?;
                    let rdata = rdata_from_text(rtype, &tokens, "").map_err(|e| err(&e))?;
                    let d = reg.domains.get_mut(*name).ok_or_else(|| err("rr before its domain"))?;
                    d.records.push(Record::new(owner, ttl, rdata));
                }
                _ => return Err(err("unknown line")),
            }
        }
        if reg.tld.is_empty() {
            return Err("registry database: no tld line".into());
        }
        Ok(reg)
    }

    /// The zone: the base file's SOA and apex records (serial kept from `base`, bumped by
    /// the caller), then every domain.
    pub fn zone(&self, base: &Zone) -> Zone {
        let mut records: Vec<Record> =
            base.records.iter().filter(|r| r.name == base.origin && r.rtype != crate::wire::SOA).cloned().collect();
        records.insert(0, base.soa_record().clone());
        for d in self.domains.values() {
            if d.hosted() {
                records.extend(d.records.iter().cloned());
            } else {
                for (n, glue) in &d.ns {
                    records.push(Record::new(&d.name, DEFAULT_TTL, RData::Ns(n.clone())));
                    if let Some(ip) = glue {
                        records.push(Record::new(n, DEFAULT_TTL, RData::A(*ip)));
                    }
                }
            }
        }
        Zone { origin: base.origin.clone(), records }
    }
}

/// Split record data into tokens: quoted strings keep their spaces (for TXT).
pub fn tokens(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let mut t = String::new();
            let mut closed = false;
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        if let Some(n) = chars.next() {
                            t.push(n)
                        }
                    }
                    '"' => {
                        closed = true;
                        break;
                    }
                    c => t.push(c),
                }
            }
            if !closed {
                return Err("unterminated quote".into());
            }
            out.push(t);
        } else {
            let mut t = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                t.push(c);
                chars.next();
            }
            out.push(t);
        }
    }
    Ok(out)
}

/// One EPP-lite session (per TCP connection).
#[derive(Debug, Default)]
pub struct Session {
    pub registrar: Option<String>,
}

/// What a command did.
#[derive(Debug, PartialEq, Eq)]
pub struct Reply {
    pub text: String,
    /// The zone changed: write it, bump the serial, reload named.
    pub changed: bool,
    pub close: bool,
}

fn ok(s: String) -> Reply {
    Reply { text: format!("OK {s}"), changed: false, close: false }
}

fn err(code: u16, s: impl Into<String>) -> Reply {
    Reply { text: format!("ERR {code} {}", s.into()), changed: false, close: false }
}

fn changed(s: String) -> Reply {
    Reply { text: format!("OK {s}"), changed: true, close: false }
}

impl Registry {
    /// Run one EPP-lite command.
    pub fn command(&mut self, sess: &mut Session, line: &str, now: u64) -> Reply {
        let w: Vec<&str> = line.split_whitespace().collect();
        let Some(cmd) = w.first() else { return err(2001, "empty command") };
        let cmd = cmd.to_ascii_uppercase();
        if cmd == "LOGIN" {
            return match w.as_slice() {
                [_, id, secret] if self.registrars.get(*id).is_some_and(|s| s == secret) => {
                    sess.registrar = Some(id.to_string());
                    ok(format!("login {id}"))
                }
                [_, _, _] => err(2200, "authentication failed"),
                _ => err(2001, "LOGIN <registrar> <secret>"),
            };
        }
        if cmd == "LOGOUT" {
            return Reply { text: "OK bye".into(), changed: false, close: true };
        }
        let Some(me) = sess.registrar.clone() else { return err(2002, "LOGIN first") };
        let domain = |reg: &Registry, i: usize| -> Result<String, Reply> {
            let s = w.get(i).ok_or_else(|| err(2001, format!("{cmd} needs a domain")))?;
            reg.domain_name(s).map_err(|(c, m)| err(c, m))
        };
        match cmd.as_str() {
            "CHECK" => match domain(self, 1) {
                Ok(d) => ok(if self.domains.contains_key(&d) { "taken".into() } else { "available".into() }),
                Err(e) => e,
            },
            "CREATE" => {
                let d = match domain(self, 1) {
                    Ok(d) => d,
                    Err(e) => return e,
                };
                let Some(registrant) = w.get(2) else { return err(2001, "CREATE <domain> <registrant>") };
                if self.domains.contains_key(&d) {
                    return err(2302, format!("{d} is already registered"));
                }
                self.domains.insert(
                    d.clone(),
                    Domain {
                        name: d.clone(),
                        registrar: me,
                        registrant: registrant.to_string(),
                        created: now,
                        updated: now,
                        ns: Vec::new(),
                        records: Vec::new(),
                    },
                );
                changed(format!("created {d}"))
            }
            "LIST" => {
                let mut s = String::from("list\n");
                for d in self.domains.values().filter(|d| d.registrar == me) {
                    let _ = writeln!(s, "{} {}", d.name, d.registrant);
                }
                s.push('.');
                ok(s)
            }
            "INFO" => {
                let d = match domain(self, 1) {
                    Ok(d) => d,
                    Err(e) => return e,
                };
                let Some(dom) = self.domains.get(&d) else { return err(2303, format!("{d} is not registered")) };
                ok(info(dom))
            }
            "DELETE" | "UPDATE" => {
                let d = match domain(self, 1) {
                    Ok(d) => d,
                    Err(e) => return e,
                };
                let Some(dom) = self.domains.get(&d) else { return err(2303, format!("{d} is not registered")) };
                if dom.registrar != me {
                    return err(2201, format!("{d} is sponsored by another registrar"));
                }
                if cmd == "DELETE" {
                    self.domains.remove(&d);
                    return changed(format!("deleted {d}"));
                }
                self.update(&d, &w[2..], now)
            }
            _ => err(2000, format!("unknown command {cmd}")),
        }
    }

    fn update(&mut self, d: &str, w: &[&str], now: u64) -> Reply {
        let dom = self.domains.get_mut(d).unwrap();
        match w {
            ["RR", "ADD", name, ttl, t, data @ ..] | ["rr", "add", name, ttl, t, data @ ..] => {
                let Some(owner) = owner(name, d) else { return err(2005, format!("'{name}' is not a name in {d}")) };
                let Some(ttl) = parse_ttl(ttl) else { return err(2005, format!("bad TTL '{ttl}'")) };
                if ttl < MIN_TTL {
                    return err(2306, format!("the TTL must be at least {MIN_TTL} seconds"));
                }
                let Some(rtype) = type_from_name(t).filter(|t| [A, CNAME, MX, TXT].contains(t)) else {
                    return err(2306, format!("record type '{t}' (A, CNAME, MX, TXT; nameservers with NS SET)"));
                };
                let toks = match tokens(&data.join(" ")) {
                    Ok(t) => t,
                    Err(e) => return err(2005, e),
                };
                let rdata = match rdata_from_text(rtype, &toks, d) {
                    Ok(r) => r,
                    Err(e) => return err(2005, e),
                };
                let rec = Record::new(&owner, ttl, rdata);
                if rtype == CNAME && (owner == d || dom.records.iter().any(|r| r.name == owner && r.rtype != CNAME)) {
                    return err(2306, format!("{} cannot be a CNAME (it has other records)", fqdn(&owner)));
                }
                if rtype != CNAME && dom.records.iter().any(|r| r.name == owner && r.rtype == CNAME) {
                    return err(2306, format!("{} is a CNAME", fqdn(&owner)));
                }
                if dom.records.len() >= MAX_RECORDS {
                    return err(2308, format!("at most {MAX_RECORDS} records"));
                }
                // Same name and type: a new value replaces an equal one.
                dom.records.retain(|r| !(r.name == rec.name && r.rtype == rec.rtype && r.data == rec.data));
                dom.records.push(rec.clone());
                dom.updated = now;
                changed(format!("{} {} {}", fqdn(&rec.name), type_name(rec.rtype), rec.data.to_text()))
            }
            ["RR", "DEL", name, t, data @ ..] | ["rr", "del", name, t, data @ ..] => {
                let Some(owner) = owner(name, d) else { return err(2005, format!("'{name}' is not a name in {d}")) };
                let Some(rtype) = type_from_name(t) else { return err(2005, format!("bad type '{t}'")) };
                let want = if data.is_empty() {
                    None
                } else {
                    match tokens(&data.join(" ")).map_err(|e| e).and_then(|t| rdata_from_text(rtype, &t, d)) {
                        Ok(r) => Some(r),
                        Err(e) => return err(2005, e),
                    }
                };
                let before = dom.records.len();
                dom.records.retain(|r| !(r.name == owner && r.rtype == rtype && want.as_ref().is_none_or(|w| *w == r.data)));
                if dom.records.len() == before {
                    return err(2303, format!("no {} record at {}", type_name(rtype), fqdn(&owner)));
                }
                dom.updated = now;
                changed(format!("removed {} {}", fqdn(&owner), type_name(rtype)))
            }
            ["NS", "HOSTED"] | ["ns", "hosted"] => {
                dom.ns.clear();
                dom.updated = now;
                changed(format!("{d} is hosted by the registry"))
            }
            ["NS", "SET", list @ ..] | ["ns", "set", list @ ..] => {
                if list.is_empty() || list.len() > MAX_NS {
                    return err(2306, format!("1 to {MAX_NS} nameservers"));
                }
                let mut ns = Vec::new();
                for item in list {
                    let (n, g) = item.split_once('=').map_or((*item, None), |(n, g)| (n, Some(g)));
                    let Some(n) = normalize(n) else { return err(2005, format!("bad nameserver '{n}'")) };
                    let glue = match g {
                        Some(g) if !g.is_empty() => match parse_ip(g) {
                            Some(ip) => Some(ip),
                            None => return err(2005, format!("bad glue address '{g}'")),
                        },
                        _ => None,
                    };
                    if is_sub(&n, d) && glue.is_none() {
                        return err(2306, format!("{} is inside {d}: give its address (glue), {}=ADDRESS", fqdn(&n), n));
                    }
                    if !is_sub(&n, d) && glue.is_some() {
                        return err(2306, format!("glue only for nameservers inside {d}"));
                    }
                    ns.push((n, glue));
                }
                dom.ns = ns;
                dom.updated = now;
                let list: Vec<String> = dom.ns.iter().map(|(n, g)| format!("{}{}", fqdn(n), g.map(|g| format!(" ({})", ip_str(g))).unwrap_or_default())).collect();
                changed(format!("{d} delegated to {}", list.join(", ")))
            }
            _ => err(2001, "UPDATE <domain> RR ADD <name> <ttl> <type> <data> | RR DEL <name> <type> [data] | NS SET <ns>[=<ip>]... | NS HOSTED"),
        }
    }
}

/// An owner name typed relative to the domain (`www`, `@`) or absolute (`www.mybase.ecm.`).
pub fn owner(name: &str, domain: &str) -> Option<String> {
    let n = if name == "@" {
        domain.to_string()
    } else if name.ends_with('.') {
        normalize(name)?
    } else if is_sub(&normalize(name)?, domain) {
        normalize(name)?
    } else {
        normalize(&format!("{name}.{domain}"))?
    };
    is_sub(&n, domain).then_some(n)
}

/// INFO text: key/value lines and the records, ended by ".".
pub fn info(d: &Domain) -> String {
    let mut s = String::from("info\n");
    let _ = writeln!(s, "domain {}", d.name);
    let _ = writeln!(s, "registrar {}", d.registrar);
    let _ = writeln!(s, "registrant {}", d.registrant);
    let _ = writeln!(s, "created {}", d.created);
    let _ = writeln!(s, "updated {}", d.updated);
    if d.hosted() {
        let _ = writeln!(s, "dns hosted by the registry");
        for r in &d.records {
            let _ = writeln!(s, "record {} {} {} {}", relative(&r.name, &d.name), r.ttl, type_name(r.rtype), r.data.to_text());
        }
    } else {
        for (n, g) in &d.ns {
            let _ = writeln!(s, "nameserver {}{}", fqdn(n), g.map(|g| format!(" {}", ip_str(g))).unwrap_or_default());
        }
    }
    s.push('.');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::NS;

    fn reg() -> (Registry, Session) {
        let mut r = Registry::new("ecm");
        r.registrars.insert("nic-ring".into(), "s3cret".into());
        r.registrars.insert("other".into(), "x".into());
        let mut s = Session::default();
        assert_eq!(r.command(&mut s, "LOGIN nic-ring s3cret", 0).text, "OK login nic-ring");
        (r, s)
    }

    #[test]
    fn login_and_create() {
        let mut r = Registry::new("ecm");
        r.registrars.insert("nic-ring".into(), "s3cret".into());
        let mut s = Session::default();
        assert!(r.command(&mut s, "CHECK mybase.ecm", 0).text.starts_with("ERR 2002"));
        assert!(r.command(&mut s, "LOGIN nic-ring nope", 0).text.starts_with("ERR 2200"));
        let (mut r, mut s) = reg();
        assert_eq!(r.command(&mut s, "CHECK mybase.ecm", 0).text, "OK available");
        let c = r.command(&mut s, "CREATE MyBase.ecm alice", 100);
        assert!(c.changed && c.text == "OK created mybase.ecm", "{c:?}");
        assert_eq!(r.command(&mut s, "CHECK mybase.ecm", 0).text, "OK taken");
        assert!(r.command(&mut s, "CREATE mybase.ecm bob", 0).text.starts_with("ERR 2302"));
        for bad in ["nic.ecm", "ab.ecm", "-ab.ecm", "a_b.ecm", "v3.ecm", "x.mybase.ecm", "mybase.ring"] {
            let t = r.command(&mut s, &format!("CREATE {bad} bob"), 0).text;
            assert!(t.starts_with("ERR 2306") || t.starts_with("ERR 2005"), "{bad}: {t}");
        }
    }

    #[test]
    fn records_delegation_and_zone() {
        let (mut r, mut s) = reg();
        r.command(&mut s, "CREATE mybase.ecm alice", 1);
        assert!(r.command(&mut s, "UPDATE mybase.ecm RR ADD www 300 A 100.70.0.120", 2).changed);
        assert!(r.command(&mut s, "UPDATE mybase.ecm RR ADD @ 300 TXT \"hello there\"", 2).changed);
        assert!(r.command(&mut s, "UPDATE mybase.ecm RR ADD www 30 A 1.2.3.4", 2).text.starts_with("ERR 2306"), "TTL floor");
        assert!(r.command(&mut s, "UPDATE mybase.ecm RR ADD x.other. 300 A 1.2.3.4", 2).text.starts_with("ERR 2005"));
        assert!(r.command(&mut s, "UPDATE mybase.ecm RR ADD www 300 CNAME x.ecm.", 2).text.starts_with("ERR 2306"));
        assert!(r.command(&mut s, "UPDATE mybase.ecm RR ADD sub 300 NS x.", 2).text.starts_with("ERR 2306"));
        let base = Zone::parse("$TTL 3600\n@ SOA ns1.ecm.ring. h.nic.ring. 1 300 60 600 60\n@ NS ns1.ecm.ring.\n@ NS ns2.ecm.ring.\n", "ecm").unwrap();
        let z = r.zone(&base);
        assert!(z.records.iter().any(|x| x.name == "www.mybase.ecm" && x.data == RData::A([100, 70, 0, 120])));
        assert!(Zone::from_records("ecm", z.records.clone()).is_ok());
        // delegation needs glue inside the domain
        assert!(r.command(&mut s, "UPDATE mybase.ecm NS SET ns1.mybase.ecm", 3).text.starts_with("ERR 2306"));
        let t = r.command(&mut s, "UPDATE mybase.ecm NS SET ns1.mybase.ecm=10.200.0.53 ns.elsewhere.ecm", 3);
        assert!(t.changed, "{t:?}");
        let z = r.zone(&base);
        assert!(z.records.iter().any(|x| x.name == "mybase.ecm" && x.rtype == NS));
        assert!(z.records.iter().any(|x| x.name == "ns1.mybase.ecm" && x.data == RData::A([10, 200, 0, 53])));
        assert!(!z.records.iter().any(|x| x.name == "www.mybase.ecm"), "hosted records hidden while delegated");
        // the database round-trips
        assert_eq!(Registry::parse(&r.render()).unwrap(), r);
        assert!(r.command(&mut s, "INFO mybase.ecm", 0).text.contains("nameserver ns1.mybase.ecm. 10.200.0.53"));
        assert!(r.command(&mut s, "UPDATE mybase.ecm NS HOSTED", 4).changed);
        assert!(r.command(&mut s, "UPDATE mybase.ecm RR DEL www A", 5).changed);
        assert!(r.command(&mut s, "UPDATE mybase.ecm RR DEL www A", 5).text.starts_with("ERR 2303"));
        // another registrar may not touch it
        let mut s2 = Session::default();
        r.command(&mut s2, "LOGIN other x", 0);
        assert!(r.command(&mut s2, "DELETE mybase.ecm", 0).text.starts_with("ERR 2201"));
        assert!(r.command(&mut s, "DELETE mybase.ecm", 0).changed);
        assert!(r.command(&mut s, "LOGOUT", 0).close);
    }
}
