//! IANA (`ianad`, docs/DNS.md "Adding TLDs"): who may add a top-level domain to the root
//! zone. An applicant runs the TLD's nameservers (a `named` with the TLD's zone) and applies
//! with an account (a handle and a password, one TLD per account). IANA checks the label,
//! then the technical evaluation: every nameserver must answer authoritatively with the
//! TLD's SOA (done by the program over the network, see [`evaluate`]). Accepted, the root
//! zone gets the TLD's NS records and their glue, the serial goes up and `named` NOTIFYs the
//! root letters. The owner (or an operator) can revoke it again.
//!
//! The database (`/etc/ianad.db`): `account <handle> <salt> <hash> <created>` and
//! `tld <label> <handle> <created>`.

use crate::name::{fqdn, ip_str, is_ldh, is_sub, normalize, Ip};
use crate::registrar::{hash, Account};
use crate::wire::{Message, RData, Record, NS, SOA};
use crate::zone::Zone;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Never delegated (RFC 6761 special-use names, mDNS, ICANN's private-use TLD, the root's
/// own reverse tree) and the ring's own TLDs.
pub const NEVER: &[&str] = &["test", "example", "invalid", "localhost", "local", "internal", "arpa", "ring", "ecm"];
pub const TLD_TTL: u32 = 3600;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Iana {
    pub accounts: BTreeMap<String, Account>,
    /// TLD -> (owner handle, created).
    pub tlds: BTreeMap<String, (String, u64)>,
}

impl Iana {
    pub fn parse(text: &str) -> Iana {
        let mut i = Iana::default();
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            match w.as_slice() {
                ["account", h, salt, hash, created] => {
                    i.accounts.insert(
                        h.to_string(),
                        Account { salt: salt.to_string(), hash: hash.to_string(), created: created.parse().unwrap_or(0) },
                    );
                }
                ["tld", t, h, created] => {
                    i.tlds.insert(t.to_string(), (h.to_string(), created.parse().unwrap_or(0)));
                }
                _ => {}
            }
        }
        i
    }

    pub fn render(&self) -> String {
        let mut s = String::from("# ianad database (owned by ianad)\n");
        for (h, a) in &self.accounts {
            let _ = writeln!(s, "account {h} {} {} {}", a.salt, a.hash, a.created);
        }
        for (t, (h, c)) in &self.tlds {
            let _ = writeln!(s, "tld {t} {h} {c}");
        }
        s
    }

    /// Log in, or create the account on its first application.
    pub fn login(&mut self, handle: &str, password: &str, salt: &str, now: u64) -> Result<(), String> {
        match self.accounts.get(handle) {
            Some(a) if hash(&a.salt, password) == a.hash => Ok(()),
            Some(_) => Err("wrong handle or password".into()),
            None => {
                crate::registrar::check_handle(handle)?;
                if password.len() < 4 {
                    return Err("the password needs at least 4 characters".into());
                }
                self.accounts.insert(handle.to_string(), Account { salt: salt.to_string(), hash: hash(salt, password), created: now });
                Ok(())
            }
        }
    }
}

/// The label checks: 2-63 LDH, not all digits, never delegated, not in the root zone yet.
pub fn check_label(label: &str, root: &Zone) -> Result<String, String> {
    let l = normalize(label).ok_or_else(|| format!("'{label}' is not a label"))?;
    if l.contains('.') {
        return Err("a TLD is a single label (base, not shop.base)".into());
    }
    if l.len() < 2 || !is_ldh(&l) {
        return Err("a TLD is 2 to 63 letters, digits and hyphens, not at the start or the end".into());
    }
    if l.bytes().all(|c| c.is_ascii_digit()) {
        return Err("a TLD cannot be all digits".into());
    }
    if NEVER.contains(&l.as_str()) {
        return Err(format!(".{l} is reserved and never delegated"));
    }
    if root.records.iter().any(|r| r.rtype == NS && r.name == l) {
        return Err(format!(".{l} is already in the root zone"));
    }
    Ok(l)
}

/// A nameserver of an application: `ns1.base=10.200.0.53` (glue needed inside the TLD).
pub fn parse_ns(tld: &str, items: &[&str]) -> Result<Vec<(String, Option<Ip>)>, String> {
    if items.is_empty() {
        return Err("at least one nameserver (ns1.base=10.200.0.53), two in different networks is better".into());
    }
    let mut out = Vec::new();
    for it in items {
        let (n, g) = it.split_once('=').map_or((*it, None), |(n, g)| (n, Some(g)));
        let n = normalize(n).ok_or_else(|| format!("bad nameserver '{n}'"))?;
        let glue = match g {
            Some(g) => Some(crate::name::parse_ip(g).ok_or_else(|| format!("bad address '{g}'"))?),
            None => None,
        };
        if is_sub(&n, tld) && glue.is_none() {
            return Err(format!("{} is inside .{tld}: give its address ({n}=ADDRESS)", fqdn(&n)));
        }
        out.push((n, glue));
    }
    Ok(out)
}

/// The technical evaluation of one nameserver's answer to `SOA tld.`: authoritative, with
/// the SOA of exactly that zone.
pub fn evaluate(tld: &str, server: &str, answer: Result<Message, String>) -> Result<(), String> {
    let m = answer.map_err(|e| format!("{server} did not answer ({e}): the nameserver is lame"))?;
    if m.header.rcode != 0 {
        return Err(format!("{server} answered {} for {}: lame", crate::wire::rcode_name(m.header.rcode), fqdn(tld)));
    }
    if !m.header.aa || !m.answers.iter().any(|r| r.rtype == SOA && r.name == tld) {
        return Err(format!("{server} is not authoritative for {} (no SOA with the AA flag): lame", fqdn(tld)));
    }
    Ok(())
}

/// The root zone with `tld` delegated to `ns` (glue added), the serial bumped.
pub fn delegate(root: &Zone, tld: &str, ns: &[(String, Option<Ip>)]) -> Zone {
    let mut z = remove(root, tld);
    for (n, g) in ns {
        z.records.push(Record::new(tld, TLD_TTL, RData::Ns(n.clone())));
        if let Some(ip) = g {
            z.records.push(Record::new(n, TLD_TTL, RData::A(*ip)));
        }
    }
    z
}

/// The root zone without `tld` (its NS records and any glue below it), the serial bumped.
pub fn remove(root: &Zone, tld: &str) -> Zone {
    let mut z = root.clone();
    z.records.retain(|r| r.rtype == SOA || !is_sub(&r.name, tld));
    z.bump_serial(0);
    z
}

/// What `domain tld show` prints.
pub fn show(root: &Zone, iana: &Iana, tld: &str) -> String {
    let mut s = String::new();
    match iana.tlds.get(tld) {
        Some((h, c)) => {
            let _ = writeln!(s, "TLD .{tld}: delegated {} to {h}", crate::registrar::date(*c));
        }
        None if root.records.iter().any(|r| r.rtype == NS && r.name == tld) => {
            let _ = writeln!(s, "TLD .{tld}: part of the ring (run by the ring's registries)");
        }
        None => {
            let _ = writeln!(s, "TLD .{tld}: not in the root zone");
            return s;
        }
    }
    for r in root.records.iter().filter(|r| r.name == tld && r.rtype == NS) {
        let t = r.data.target().unwrap_or("");
        let glue: Vec<String> = root.get(t, crate::wire::A).iter().filter_map(|a| a.a()).map(ip_str).collect();
        let _ = writeln!(s, "  nameserver {}{}", fqdn(t), if glue.is_empty() { String::new() } else { format!(" {}", glue.join(" ")) });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> Zone {
        Zone::parse("$TTL 86400\n@ SOA a.root-servers.ring. h.nic.ring. 5 300 60 600 300\n@ NS a.root-servers.ring.\nring. NS ns1.ring.ring.\nns1.ring.ring. A 100.68.0.55\n", "").unwrap()
    }

    #[test]
    fn labels() {
        let r = root();
        assert_eq!(check_label("Base", &r), Ok("base".into()));
        for bad in ["test", "ring", "localhost", "a", "123", "-ab", "shop.base", "arpa"] {
            assert!(check_label(bad, &r).is_err(), "{bad}");
        }
    }

    #[test]
    fn delegate_and_remove() {
        let r = root();
        let ns = parse_ns("base", &["ns1.base=10.200.0.53", "ns2.base=10.201.0.53"]).unwrap();
        assert!(parse_ns("base", &["ns1.base"]).is_err(), "glue required");
        assert!(parse_ns("base", &[]).is_err());
        let z = delegate(&r, "base", &ns);
        assert_eq!(z.serial(), 6);
        assert_eq!(z.get("base", NS).len(), 2);
        assert_eq!(z.get("ns2.base", crate::wire::A)[0].data, RData::A([10, 201, 0, 53]));
        assert!(check_label("base", &z).is_err(), "taken now");
        assert!(Zone::from_records("", z.records.clone()).is_ok());
        let back = remove(&z, "base");
        assert!(back.get("base", NS).is_empty() && back.get("ns1.base", crate::wire::A).is_empty());
        assert_eq!(back.get("ring", NS).len(), 1, "other TLDs untouched");
        assert_eq!(back.serial(), 7);
    }

    #[test]
    fn evaluation() {
        let q = Message::query(1, "base", SOA, false);
        let mut good = Message::response_to(&q, 0);
        good.header.aa = true;
        good.answers.push(Record::new(
            "base",
            60,
            RData::Soa(crate::wire::Soa { mname: "ns1.base".into(), rname: "h.base".into(), serial: 1, refresh: 1, retry: 1, expire: 1, minimum: 1 }),
        ));
        assert!(evaluate("base", "ns1.base", Ok(good.clone())).is_ok());
        let mut not_aa = good.clone();
        not_aa.header.aa = false;
        assert!(evaluate("base", "ns1.base", Ok(not_aa)).unwrap_err().contains("lame"));
        assert!(evaluate("base", "ns2.base", Err("timeout".into())).unwrap_err().contains("lame"));
        let refused = Message::response_to(&q, crate::wire::REFUSED);
        assert!(evaluate("base", "x", Ok(refused)).is_err());
    }

    #[test]
    fn accounts() {
        let mut i = Iana::default();
        assert!(i.login("alice", "pw12", "s", 1).is_ok(), "first application creates it");
        assert!(i.login("alice", "pw12", "s", 1).is_ok());
        assert!(i.login("alice", "nope", "s", 1).is_err());
        i.tlds.insert("base".into(), ("alice".into(), 1));
        assert_eq!(Iana::parse(&i.render()), i);
        let z = delegate(&root(), "base", &parse_ns("base", &["ns1.base=10.200.0.53"]).unwrap());
        assert!(show(&z, &i, "base").contains("nameserver ns1.base. 10.200.0.53"));
        assert!(show(&z, &i, "ring").contains("part of the ring"));
    }
}
