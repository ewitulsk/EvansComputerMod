//! The registrar (`registrard`, docs/DNS.md "The registrar"): registrant accounts (a handle
//! and a password; the account belongs to whoever knows the password), the domains each
//! account holds, the rules for them, and the plain-text API the `domain` program and the
//! web pages use. The domains themselves live at the registry, reached over EPP-lite
//! ([`crate::registry`]); this module only needs a function that sends one EPP command and
//! returns the reply, so it is tested against an in-memory registry.
//!
//! API (HTTP POST `/api`, form-encoded, `text/plain` reply `OK ...` or `ERR ...`):
//! `op=account handle= password=` (create), `op=register`, `op=set name= type= data= [ttl=]`,
//! `op=del name= type= [data=]`, `op=ns ns=ns1.x.ecm=1.2.3.4,ns2...` (empty: hosted), `op=show`,
//! `op=list`, `op=check`, `op=delete`; all but `check` need `handle` and `password`.

use crate::name::normalize;
use std::collections::BTreeMap;
use std::fmt::Write as _;

pub const MAX_DOMAINS: usize = 5;
pub const WHOIS_PORT: u16 = 43;

/// Salted SHA-256 of a password, hex.
pub fn hash(salt: &str, password: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update(b":");
    h.update(password.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub salt: String,
    pub hash: String,
    pub created: u64,
}

/// Accounts and which account holds which domain. File lines:
/// `account <handle> <salt> <hash> <created>` and `holds <handle> <domain>`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Accounts {
    pub accounts: BTreeMap<String, Account>,
    pub holds: BTreeMap<String, String>,
}

pub fn check_handle(h: &str) -> Result<(), String> {
    if h.len() < 2 || h.len() > 32 || !h.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') {
        return Err("a handle is 2 to 32 letters, digits, '-' or '_'".into());
    }
    Ok(())
}

impl Accounts {
    pub fn parse(text: &str) -> Accounts {
        let mut a = Accounts::default();
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            match w.as_slice() {
                ["account", h, salt, hash, created] => {
                    a.accounts.insert(
                        h.to_string(),
                        Account { salt: salt.to_string(), hash: hash.to_string(), created: created.parse().unwrap_or(0) },
                    );
                }
                ["holds", h, d] => {
                    a.holds.insert(d.to_string(), h.to_string());
                }
                _ => {}
            }
        }
        a
    }

    pub fn render(&self) -> String {
        let mut s = String::from("# registrard accounts (owned by registrard)\n");
        for (h, a) in &self.accounts {
            let _ = writeln!(s, "account {h} {} {} {}", a.salt, a.hash, a.created);
        }
        for (d, h) in &self.holds {
            let _ = writeln!(s, "holds {h} {d}");
        }
        s
    }

    pub fn create(&mut self, handle: &str, password: &str, salt: &str, now: u64) -> Result<(), String> {
        check_handle(handle)?;
        if password.len() < 4 {
            return Err("the password needs at least 4 characters".into());
        }
        if self.accounts.contains_key(handle) {
            return Err(format!("the handle '{handle}' is taken"));
        }
        self.accounts.insert(handle.to_string(), Account { salt: salt.to_string(), hash: hash(salt, password), created: now });
        Ok(())
    }

    pub fn check(&self, handle: &str, password: &str) -> bool {
        self.accounts.get(handle).is_some_and(|a| hash(&a.salt, password) == a.hash)
    }

    pub fn domains_of(&self, handle: &str) -> Vec<String> {
        self.holds.iter().filter(|(_, h)| *h == handle).map(|(d, _)| d.clone()).collect()
    }
}

/// Decode `application/x-www-form-urlencoded`.
pub fn form(body: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for pair in body.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        m.insert(url_decode(k), url_decode(v));
    }
    m
}

pub fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// Civil date of a Unix time (UTC), `2026-10-16`.
pub fn date(unix: u64) -> String {
    let days = (unix / 86400) as i64;
    // Howard Hinnant's civil_from_days
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

/// The registrar's state and its way to the registry.
pub struct Registrar<'a> {
    pub accounts: &'a mut Accounts,
    /// Send one EPP-lite command (after LOGIN), return the reply text.
    pub epp: &'a mut dyn FnMut(&str) -> String,
    pub tld: String,
}

fn ok(s: impl Into<String>) -> String {
    format!("OK {}", s.into())
}

fn err(s: impl Into<String>) -> String {
    format!("ERR {}", s.into())
}

/// An EPP reply as the API reports it: `OK x` stays, `ERR <code> text` loses the code.
fn epp_result(r: &str) -> Result<String, String> {
    if let Some(rest) = r.strip_prefix("OK ") {
        Ok(rest.to_string())
    } else if let Some(rest) = r.strip_prefix("ERR ") {
        Err(rest.split_once(' ').map_or(rest, |(_, t)| t).to_string())
    } else {
        Err(format!("the registry did not answer ({})", r.trim()))
    }
}

impl Registrar<'_> {
    fn domain(&self, f: &BTreeMap<String, String>) -> Result<String, String> {
        let d = f.get("domain").map(|s| s.trim()).filter(|s| !s.is_empty()).ok_or("which domain? (domain=NAME.ecm)")?;
        let n = normalize(d).ok_or_else(|| format!("'{d}' is not a domain name"))?;
        if !n.ends_with(&format!(".{}", self.tld)) {
            return Err(format!("this registrar sells .{} names", self.tld));
        }
        Ok(n)
    }

    fn auth(&self, f: &BTreeMap<String, String>) -> Result<String, String> {
        let h = f.get("handle").cloned().unwrap_or_default();
        let p = f.get("password").cloned().unwrap_or_default();
        if h.is_empty() {
            return Err("log in first: domain login HANDLE PASSWORD (or create an account: domain account create HANDLE PASSWORD)".into());
        }
        if !self.accounts.check(&h, &p) {
            return Err("wrong handle or password".into());
        }
        Ok(h)
    }

    /// The domain must be this account's.
    fn mine(&self, f: &BTreeMap<String, String>) -> Result<(String, String), String> {
        let h = self.auth(f)?;
        let d = self.domain(f)?;
        match self.accounts.holds.get(&d) {
            Some(owner) if *owner == h => Ok((h, d)),
            Some(_) => Err(format!("{d} belongs to another account")),
            None => Err(format!("{d} is not registered here")),
        }
    }

    /// One API call. `salt` is fresh randomness for a new account.
    pub fn api(&mut self, f: &BTreeMap<String, String>, salt: &str, now: u64) -> String {
        let op = f.get("op").map(String::as_str).unwrap_or("");
        let r: Result<String, String> = (|| match op {
            "account" => {
                let h = f.get("handle").cloned().unwrap_or_default();
                let p = f.get("password").cloned().unwrap_or_default();
                self.accounts.create(&h, &p, salt, now)?;
                Ok(format!("account {h} created"))
            }
            "login" => self.auth(f).map(|h| format!("logged in as {h}")),
            "check" => {
                let d = self.domain(f)?;
                epp_result(&(self.epp)(&format!("CHECK {d}"))).map(|s| format!("{d} {s}"))
            }
            "register" => {
                let h = self.auth(f)?;
                let d = self.domain(f)?;
                let held = self.accounts.domains_of(&h).len();
                if held >= MAX_DOMAINS {
                    return Err(format!("an account holds at most {MAX_DOMAINS} domains"));
                }
                epp_result(&(self.epp)(&format!("CREATE {d} {h}")))?;
                self.accounts.holds.insert(d.clone(), h.clone());
                Ok(format!("registered {d} to {h}; its records are served by the registry until you set nameservers"))
            }
            "set" => {
                let (_, d) = self.mine(f)?;
                let name = f.get("name").cloned().unwrap_or_else(|| "@".into());
                let t = f.get("type").cloned().unwrap_or_default();
                let data = f.get("data").cloned().unwrap_or_default();
                let ttl = f.get("ttl").cloned().filter(|t| !t.is_empty()).unwrap_or_else(|| crate::registry::DEFAULT_TTL.to_string());
                if t.is_empty() || data.is_empty() {
                    return Err("set needs a name, a type and data (domain set NAME.ecm www A 100.70.0.120)".into());
                }
                epp_result(&(self.epp)(&format!("UPDATE {d} RR ADD {name} {ttl} {t} {data}")))
            }
            "del" => {
                let (_, d) = self.mine(f)?;
                let name = f.get("name").cloned().unwrap_or_else(|| "@".into());
                let t = f.get("type").cloned().unwrap_or_default();
                let data = f.get("data").cloned().unwrap_or_default();
                epp_result(&(self.epp)(&format!("UPDATE {d} RR DEL {name} {t} {data}").trim_end().to_string()))
            }
            "ns" => {
                let (_, d) = self.mine(f)?;
                let list: Vec<String> =
                    f.get("ns").map(|s| s.split([',', ' ']).filter(|x| !x.is_empty()).map(String::from).collect()).unwrap_or_default();
                if list.is_empty() {
                    epp_result(&(self.epp)(&format!("UPDATE {d} NS HOSTED")))
                } else {
                    epp_result(&(self.epp)(&format!("UPDATE {d} NS SET {}", list.join(" "))))
                }
            }
            "show" => {
                let (_, d) = self.mine(f)?;
                let info = epp_result(&(self.epp)(&format!("INFO {d}")))?;
                Ok(info.trim_end_matches('.').trim_end().to_string())
            }
            "delete" => {
                let (_, d) = self.mine(f)?;
                epp_result(&(self.epp)(&format!("DELETE {d}")))?;
                self.accounts.holds.remove(&d);
                Ok(format!("deleted {d}"))
            }
            "list" => {
                let h = self.auth(f)?;
                let ds = self.accounts.domains_of(&h);
                Ok(format!("{} domain(s){}{}", ds.len(), if ds.is_empty() { "" } else { "\n" }, ds.join("\n")))
            }
            "" => Err("no op".into()),
            o => Err(format!("unknown op '{o}'")),
        })();
        match r {
            Ok(s) => ok(s),
            Err(e) => err(e),
        }
    }

    /// The whois answer for a query line.
    pub fn whois(&mut self, query: &str) -> String {
        let q = query.trim();
        let Some(d) = normalize(q) else { return format!("% '{q}' is not a domain name\n") };
        if !d.ends_with(&format!(".{}", self.tld)) {
            return format!("% This server answers for .{} names (whois.nic.ring).\n", self.tld);
        }
        let info = match epp_result(&(self.epp)(&format!("INFO {d}"))) {
            Ok(i) => i,
            Err(e) if e.contains("not registered") => return format!("No match for \"{}\".\n", d.to_ascii_uppercase()),
            Err(e) => return format!("% the registry did not answer: {e}\n"),
        };
        let mut s = String::new();
        let _ = writeln!(s, "Domain Name: {}", d.to_ascii_uppercase());
        let mut hosted = false;
        for line in info.lines() {
            let (k, v) = line.split_once(' ').unwrap_or((line, ""));
            match k {
                "registrar" => {
                    let _ = writeln!(s, "Registrar: nic.ring ({v})");
                }
                "registrant" => {
                    let _ = writeln!(s, "Registrant: {v}");
                }
                "created" => {
                    let _ = writeln!(s, "Creation Date: {}", date(v.parse().unwrap_or(0)));
                }
                "updated" => {
                    let _ = writeln!(s, "Updated Date: {}", date(v.parse().unwrap_or(0)));
                }
                "nameserver" => {
                    let _ = writeln!(s, "Name Server: {}", v.to_ascii_uppercase());
                }
                "dns" => hosted = true,
                _ => {}
            }
        }
        if hosted {
            let _ = writeln!(s, "Name Server: NS1.ECM.RING (hosted by the registry)\nName Server: NS2.ECM.RING (hosted by the registry)");
        }
        s.push_str(">>> whois.nic.ring: no passwords or contact details are published <<<\n");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Registry, Session};

    fn setup() -> (Registry, Accounts) {
        let mut reg = Registry::new("ecm");
        reg.registrars.insert("nic-ring".into(), "k".into());
        (reg, Accounts::default())
    }

    fn call(reg: &mut Registry, acc: &mut Accounts, q: &str) -> String {
        let mut sess = Session::default();
        reg.command(&mut sess, "LOGIN nic-ring k", 0);
        let mut epp = |c: &str| reg.command(&mut sess, c, 1_760_000_000).text;
        let mut r = Registrar { accounts: acc, epp: &mut epp, tld: "ecm".into() };
        r.api(&form(q), "salty", 1_760_000_000)
    }

    #[test]
    fn accounts_register_and_rules() {
        let (mut reg, mut acc) = setup();
        assert!(call(&mut reg, &mut acc, "op=register&handle=alice&password=pw12&domain=mybase.ecm").starts_with("ERR wrong handle"));
        assert_eq!(call(&mut reg, &mut acc, "op=account&handle=alice&password=pw12"), "OK account alice created");
        assert!(call(&mut reg, &mut acc, "op=account&handle=alice&password=zzzz").starts_with("ERR the handle"));
        assert!(call(&mut reg, &mut acc, "op=register&handle=alice&password=pw12&domain=mybase.ecm").starts_with("OK registered mybase.ecm"));
        assert!(call(&mut reg, &mut acc, "op=register&handle=alice&password=pw12&domain=mybase.ecm").contains("already registered"));
        assert!(call(&mut reg, &mut acc, "op=register&handle=alice&password=pw12&domain=nic.ecm").contains("reserved"));
        assert!(call(&mut reg, &mut acc, "op=set&handle=alice&password=wrong&domain=mybase.ecm&name=www&type=A&data=1.2.3.4").starts_with("ERR wrong"));
        assert!(call(&mut reg, &mut acc, "op=set&handle=alice&password=pw12&domain=mybase.ecm&name=www&type=A&data=100.70.0.120").starts_with("OK www.mybase.ecm. A"));
        let show = call(&mut reg, &mut acc, "op=show&handle=alice&password=pw12&domain=mybase.ecm");
        assert!(show.contains("record www 300 A 100.70.0.120"), "{show}");
        // bob may not touch alice's domain
        call(&mut reg, &mut acc, "op=account&handle=bob&password=pw34");
        assert!(call(&mut reg, &mut acc, "op=set&handle=bob&password=pw34&domain=mybase.ecm&name=x&type=A&data=1.1.1.1").contains("another account"));
        // five per account
        for i in 0..4 {
            assert!(call(&mut reg, &mut acc, &format!("op=register&handle=alice&password=pw12&domain=base{i}.ecm")).starts_with("OK"));
        }
        assert!(call(&mut reg, &mut acc, "op=register&handle=alice&password=pw12&domain=base9.ecm").contains("at most 5"));
        assert_eq!(Accounts::parse(&acc.render()), acc);
        let ns = call(&mut reg, &mut acc, "op=ns&handle=alice&password=pw12&domain=mybase.ecm&ns=ns1.mybase.ecm%3D10.200.0.53");
        assert!(ns.starts_with("OK mybase.ecm delegated"), "{ns}");
    }

    #[test]
    fn whois_and_helpers() {
        let (mut reg, mut acc) = setup();
        call(&mut reg, &mut acc, "op=account&handle=alice&password=pw12");
        call(&mut reg, &mut acc, "op=register&handle=alice&password=pw12&domain=mybase.ecm");
        let mut sess = Session::default();
        reg.command(&mut sess, "LOGIN nic-ring k", 0);
        let mut epp = |c: &str| reg.command(&mut sess, c, 0).text;
        let mut r = Registrar { accounts: &mut acc, epp: &mut epp, tld: "ecm".into() };
        let w = r.whois("mybase.ecm\r\n");
        assert!(w.contains("Domain Name: MYBASE.ECM") && w.contains("Registrant: alice") && w.contains("Creation Date: 2025-10-09"), "{w}");
        assert!(!w.contains("pw12"));
        assert!(r.whois("nope.ecm").starts_with("No match"));
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(url_decode("a%3Db+c"), "a=b c");
        assert_eq!(url_decode(&url_encode("x=1 \"q\"")), "x=1 \"q\"");
        assert_eq!(url_decode("%"), "%");
        assert_eq!(url_decode("%4"), "%4");
    }
}
