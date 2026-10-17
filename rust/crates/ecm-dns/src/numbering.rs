//! The numbering administrator (`ianad`, docs/DNS.md "ENUM"), as NANPA hands out
//! exchanges: a carrier applies for a block of 10,000 numbers (one NPA-NXX, written
//! `+1214555`) for its SIP domain, and IANA publishes the block's owner under
//! `e164.arpa`, a zone served by the same `named` as the root and delegated from it.
//!
//! * [`Numbering::apply`]: grant a free block to a domain whose SIP service resolves
//!   ([`evaluate`]). A domain's first block issues its key (32 hex), shown once; it signs
//!   the domain's port-out codes and authorizes ports into the domain.
//! * [`Numbering::port`]: move one number to another domain with a port-out code from
//!   the block's holder ([`port_code`]); it becomes a per-number NAPTR that overrides the
//!   block's. Porting back to the block's holder removes it. A code works once.
//! * [`Numbering::zone`]: the `e164.arpa` zone; [`delegate_root`], [`named_conf_with_zone`]:
//!   what the root zone and `named.conf` need to serve it.
//!
//! A port-out code is `<nonce>-<mac>`: 6 hex digits of nonce, then the first 8 hex digits of
//! HMAC-SHA256(block key, `ECM-PORT` || number || nonce), the number as `+12145550142`
//! and the nonce as its 6 characters (`cored port-code` makes them).
//!
//! The database lines (in `/etc/ianad.db`, after the TLD lines):
//! `enum-key <domain> <key> <created>`, `enum-block <+1NPANXX> <domain> <created>`,
//! `enum-port <+number> <domain> <created>`, `enum-used <+number> <nonce>`.

use crate::enum_dns::{enum_block_domain, enum_domain, E164_ARPA};
use crate::name::{fqdn, ip_str, is_ldh, is_sub, normalize};
use crate::wire::{Message, Naptr, RData, Record, Soa, A, NS, SOA, SRV};
use crate::zone::Zone;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// TTL of the NAPTR records (a port shows within minutes).
pub const ENUM_TTL: u32 = 300;
/// The SOA minimum (how long an unported number's NXDOMAIN is cached).
pub const NEGATIVE_TTL: u32 = 60;
pub const PORT_TAG: &[u8] = b"ECM-PORT";
/// Where `ianad` keeps the zone by default.
pub const ZONE_FILE: &str = "/etc/named/e164.arpa.zone";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Numbering {
    /// SIP domain -> (key, created).
    pub keys: BTreeMap<String, (String, u64)>,
    /// Block (`+1214555`) -> (domain, created).
    pub blocks: BTreeMap<String, (String, u64)>,
    /// Ported number -> (domain, created).
    pub ports: BTreeMap<String, (String, u64)>,
    /// Port-out codes spent: (number, nonce).
    pub used: BTreeSet<(String, String)>,
}

/// NPA and NXX: 3 digits, the first 2-9, not N11.
fn nanp3(g: &str) -> bool {
    g.len() == 3 && g.bytes().all(|b| b.is_ascii_digit()) && g.as_bytes()[0] >= b'2' && &g[1..] != "11"
}

fn digits_of(s: &str) -> Result<String, String> {
    let d: String = s.chars().filter(|c| !matches!(c, '+' | '-' | ' ' | '(' | ')' | '.')).collect();
    if d.is_empty() || !d.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("'{s}' is not a number"));
    }
    Ok(d)
}

/// A block, `+1214555` (also `214-555`, `1214555`): the country code `cc` and one NPA-NXX.
pub fn parse_block(s: &str, cc: &str) -> Result<String, String> {
    let d = digits_of(s).map_err(|_| format!("'{s}' is not a number block (+{cc}NPANXX, e.g. +{cc}214555)"))?;
    let d = if d.len() == 6 { format!("{cc}{d}") } else { d };
    let rest = d.strip_prefix(cc).filter(|r| r.len() == 6).ok_or_else(|| format!("'{s}' is not a number block (+{cc}NPANXX, e.g. +{cc}214555)"))?;
    if cc == "1" && !(nanp3(&rest[..3]) && nanp3(&rest[3..])) {
        return Err(format!("'{s}' is not a NANP block: NPA and NXX start 2-9 and are not N11"));
    }
    Ok(format!("+{d}"))
}

/// A number, `+12145550142` (also `214-555-0142`): the country code and 10 digits.
pub fn parse_number(s: &str, cc: &str) -> Result<String, String> {
    let d = digits_of(s).map_err(|_| format!("'{s}' is not a number (+{cc} and 10 digits)"))?;
    let d = if d.len() == 10 { format!("{cc}{d}") } else { d };
    if !d.strip_prefix(cc).is_some_and(|r| r.len() == 10) {
        return Err(format!("'{s}' is not a number (+{cc} and 10 digits, e.g. +{cc}2145550142)"));
    }
    parse_block(&d[..cc.len() + 6], cc)?;
    Ok(format!("+{d}"))
}

/// The block a number is in (`+12145550142` -> `+1214555`).
pub fn block_of(number: &str) -> &str {
    &number[..number.len() - 4]
}

/// A SIP domain: a name of two or more LDH labels, outside `e164.arpa`.
pub fn check_domain(s: &str) -> Result<String, String> {
    let d = normalize(s).ok_or_else(|| format!("'{s}' is not a domain"))?;
    if !d.contains('.') || !d.split('.').all(is_ldh) {
        return Err(format!("'{s}' is not a SIP domain (like cell-red.ecm)"));
    }
    if is_sub(&d, "arpa") {
        return Err("a SIP domain is not under .arpa".into());
    }
    Ok(d)
}

/// The technical check of an application: `_sip._udp.<domain>` SRV (a SIP border) or the
/// domain's A record must resolve. The answers come from the program (recursive queries).
pub fn evaluate(domain: &str, srv: Result<Message, String>, a: Result<Message, String>) -> Result<String, String> {
    let sip = format!("_sip._udp.{domain}");
    if let Ok(m) = &srv {
        for r in m.answers.iter().filter(|r| r.rtype == SRV) {
            if let RData::Srv(s) = &r.data {
                return Ok(format!("{} SRV {}:{}", fqdn(&sip), fqdn(&s.target), s.port));
            }
        }
    }
    if let Ok(m) = &a {
        if let Some(ip) = m.answers.iter().filter(|r| r.rtype == A).find_map(|r| r.a()) {
            return Ok(format!("{} A {}", fqdn(domain), ip_str(ip)));
        }
    }
    let why = |r: &Result<Message, String>| match r {
        Ok(m) if m.header.rcode != 0 => crate::wire::rcode_name(m.header.rcode),
        Ok(_) => "no records".to_string(),
        Err(e) => e.clone(),
    };
    Err(format!(
        "neither {} SRV ({}) nor {} A ({}) resolves: publish the carrier's SIP border first",
        fqdn(&sip),
        why(&srv),
        fqdn(domain),
        why(&a)
    ))
}

fn mac(key_hex: &str, number: &str, nonce: &str) -> Option<String> {
    use hmac::{Hmac, Mac};
    let key = unhex16(key_hex)?;
    let mut m = <Hmac<sha2::Sha256> as Mac>::new_from_slice(&key).ok()?;
    m.update(PORT_TAG);
    m.update(number.as_bytes());
    m.update(nonce.as_bytes());
    Some(m.finalize().into_bytes()[..4].iter().map(|b| format!("{b:02x}")).collect())
}

fn unhex16(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// The port-out code for `number` (`+12145550142`) with `nonce` (6 hex digits), signed with
/// the key of the number's block.
pub fn port_code(key_hex: &str, number: &str, nonce: &str) -> Option<String> {
    Some(format!("{nonce}-{}", mac(key_hex, number, nonce)?))
}

/// A key as `ianad` hands it out: 32 lowercase hex digits.
pub fn check_key(k: &str) -> Result<String, String> {
    let k = k.trim().to_ascii_lowercase();
    if unhex16(&k).is_none() {
        return Err("an ENUM key is 32 hex digits".into());
    }
    Ok(k)
}

impl Numbering {
    /// One database line, if it is a numbering line.
    pub fn parse_line(&mut self, w: &[&str]) -> bool {
        let t = |s: &str| s.parse().unwrap_or(0);
        match w {
            ["enum-key", d, k, c] => {
                self.keys.insert(d.to_string(), (k.to_string(), t(c)));
            }
            ["enum-block", b, d, c] => {
                self.blocks.insert(b.to_string(), (d.to_string(), t(c)));
            }
            ["enum-port", n, d, c] => {
                self.ports.insert(n.to_string(), (d.to_string(), t(c)));
            }
            ["enum-used", n, nonce] => {
                self.used.insert((n.to_string(), nonce.to_string()));
            }
            _ => return false,
        }
        true
    }

    pub fn render(&self) -> String {
        let mut s = String::new();
        for (d, (k, c)) in &self.keys {
            let _ = writeln!(s, "enum-key {d} {k} {c}");
        }
        for (b, (d, c)) in &self.blocks {
            let _ = writeln!(s, "enum-block {b} {d} {c}");
        }
        for (n, (d, c)) in &self.ports {
            let _ = writeln!(s, "enum-port {n} {d} {c}");
        }
        for (n, nonce) in &self.used {
            let _ = writeln!(s, "enum-used {n} {nonce}");
        }
        s
    }

    /// Grant `block` to `domain`. A domain that holds a key must give it (`key`); one that
    /// does not gets `fresh_key`, returned once. Returns the reply and the key issued.
    pub fn apply(&mut self, cc: &str, block: &str, domain: &str, key: &str, fresh_key: &str, now: u64) -> Result<(String, Option<String>), String> {
        let b = parse_block(block, cc)?;
        let d = check_domain(domain)?;
        if let Some((owner, _)) = self.blocks.get(&b) {
            return Err(format!("{b} is already held by {owner}"));
        }
        let issued = match self.keys.get(&d) {
            Some((k, _)) => {
                if key.is_empty() {
                    return Err(format!("{d} already has an ENUM key: give it to apply for another block"));
                }
                if check_key(key)? != *k {
                    return Err(format!("that is not the ENUM key IANA issued to {d}"));
                }
                None
            }
            None => {
                let k = check_key(fresh_key)?;
                self.keys.insert(d.clone(), (k.clone(), now));
                Some(k)
            }
        };
        self.blocks.insert(b.clone(), (d.clone(), now));
        let msg = match &issued {
            Some(k) => format!("{b} (10,000 numbers) is held by {d}; its ENUM key is {k} (shown once: put it in cored with enum-key)"),
            None => format!("{b} (10,000 numbers) is held by {d} (signed with its ENUM key)"),
        };
        Ok((msg, issued))
    }

    /// Port `number` to `domain` (the gaining carrier, proving itself with `key`) with the
    /// port-out `code` its subscriber got from the block's holder.
    pub fn port(&mut self, cc: &str, number: &str, domain: &str, code: &str, key: &str, now: u64) -> Result<String, String> {
        let n = parse_number(number, cc)?;
        let d = check_domain(domain)?;
        let b = block_of(&n).to_string();
        let (owner, _) = self.blocks.get(&b).cloned().ok_or_else(|| format!("no carrier holds {b}: {n} is not in service"))?;
        match self.keys.get(&d) {
            Some((k, _)) if check_key(key).ok().as_deref() == Some(k.as_str()) => {}
            Some(_) => return Err(format!("that is not the ENUM key IANA issued to {d}")),
            None => return Err(format!("{d} holds no ENUM key: a carrier gets one with its first block")),
        }
        let code = code.trim().to_ascii_lowercase();
        let (nonce, m) = code.split_once('-').filter(|(a, b)| a.len() == 6 && b.len() == 8).ok_or("a port-out code is NONCE-MAC (6 and 8 hex digits, e.g. 3fa2c1-9b04e7d2)")?;
        if !code.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-') {
            return Err("a port-out code is hex digits".into());
        }
        let owner_key = &self.keys.get(&owner).ok_or_else(|| format!("{owner} has no ENUM key"))?.0;
        if mac(owner_key, &n, nonce).as_deref() != Some(m) {
            return Err(format!("wrong port-out code for {n} (ask {owner} for a new one)"));
        }
        if self.used.contains(&(n.clone(), nonce.to_string())) {
            return Err(format!("that port-out code for {n} was used already"));
        }
        self.used.insert((n.clone(), nonce.to_string()));
        let from = self.ports.get(&n).map(|(x, _)| x.clone()).unwrap_or_else(|| owner.clone());
        if d == owner {
            self.ports.remove(&n);
            Ok(format!("{n} is back with {d}, the holder of {b} (from {from})"))
        } else {
            self.ports.insert(n.clone(), (d.clone(), now));
            Ok(format!("{n} is ported to {d} (from {from})"))
        }
    }

    /// Take a block back: its numbers and their ports go out of service.
    pub fn revoke(&mut self, cc: &str, block: &str) -> Result<String, String> {
        let b = parse_block(block, cc)?;
        let (owner, _) = self.blocks.remove(&b).ok_or_else(|| format!("no carrier holds {b}"))?;
        let before = self.ports.len();
        self.ports.retain(|n, _| block_of(n) != b);
        self.used.retain(|(n, _)| block_of(n) != b);
        Ok(format!("{b} taken back from {owner} ({} ported number(s) dropped)", before - self.ports.len()))
    }

    /// What `domain enum list` and `ianad enum` print.
    pub fn list(&self) -> String {
        let mut s = format!("{} block(s), {} ported number(s)", self.blocks.len(), self.ports.len());
        for (b, (d, _)) in &self.blocks {
            let _ = write!(s, "\nblock {b} {d}");
        }
        for (n, (d, _)) in &self.ports {
            let _ = write!(s, "\nport {n} {d}");
        }
        s
    }

    /// The `e164.arpa` zone: a NAPTR per block and per ported number, the nameserver `ns`.
    pub fn zone(&self, serial: u32, ns: &str) -> Zone {
        let soa = Soa { mname: ns.into(), rname: format!("hostmaster.{ns}"), serial, refresh: 300, retry: 60, expire: 604800, minimum: NEGATIVE_TTL };
        let mut recs = vec![Record::new(E164_ARPA, 3600, RData::Soa(soa)), Record::new(E164_ARPA, 3600, RData::Ns(ns.into()))];
        for (b, (d, _)) in &self.blocks {
            recs.push(Record::new(&enum_block_domain(b, E164_ARPA), ENUM_TTL, RData::Naptr(sip_naptr(d))));
        }
        for (n, (d, _)) in &self.ports {
            recs.push(Record::new(&enum_domain(n, E164_ARPA), ENUM_TTL, RData::Naptr(sip_naptr(d))));
        }
        Zone::from_records(E164_ARPA, recs).expect("the records are inside e164.arpa")
    }
}

/// `100 10 "u" "E2U+sip" "!^.*$!sip:\1@domain!" .`: the number at `domain`.
pub fn sip_naptr(domain: &str) -> Naptr {
    Naptr {
        order: 100,
        preference: 10,
        flags: "u".into(),
        services: "E2U+sip".into(),
        regexp: format!("!^.*$!sip:\\1@{domain}!"),
        replacement: String::new(),
    }
}

/// The root zone with `e164.arpa` delegated to `ns` (the serial bumped), or `None` when it
/// already is.
pub fn delegate_root(root: &Zone, ns: &str) -> Option<Zone> {
    let cut: Vec<&Record> = root.get(E164_ARPA, NS);
    if cut.len() == 1 && cut[0].data.target() == Some(ns) {
        return None;
    }
    let mut z = root.clone();
    z.records.retain(|r| !(r.name == E164_ARPA && r.rtype != SOA));
    z.records.push(Record::new(E164_ARPA, crate::iana::TLD_TTL, RData::Ns(ns.into())));
    z.bump_serial(0);
    Some(z)
}

/// `named.conf` with `zone e164.arpa primary FILE` added, or `None` when it serves the zone.
pub fn named_conf_with_zone(conf: &str, file: &str) -> Option<String> {
    let has = conf.lines().any(|l| {
        let w: Vec<&str> = l.split('#').next().unwrap_or("").split_whitespace().collect();
        w.len() >= 2 && w[0] == "zone" && normalize(w[1]).as_deref() == Some(E164_ARPA)
    });
    if has {
        return None;
    }
    let mut s = conf.to_string();
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    let _ = writeln!(s, "# ENUM: phone numbers (ianad keeps this zone)\nzone {E164_ARPA} primary {file}");
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enum_dns::{enum_sip_uri, naptrs};
    use crate::wire::{NAPTR, NXDOMAIN};

    const KA: &str = "000102030405060708090a0b0c0d0e0f";
    const KB: &str = "f0e0d0c0b0a090807060504030201000";

    fn two_carriers() -> Numbering {
        let mut n = Numbering::default();
        let (msg, k) = n.apply("1", "+1214555", "cell-red.ecm", "", KA, 1).unwrap();
        assert_eq!(k.as_deref(), Some(KA));
        assert!(msg.contains(KA), "the key is shown once: {msg}");
        n.apply("1", "214-556", "Cell-Blue.ecm", "", KB, 2).unwrap();
        n
    }

    #[test]
    fn blocks_and_numbers() {
        assert_eq!(parse_block("+1214555", "1").as_deref(), Ok("+1214555"));
        assert_eq!(parse_block("214-555", "1").as_deref(), Ok("+1214555"));
        for bad in ["+1211555", "+1214155", "+1114555", "+121455", "+44214555", "abc", ""] {
            assert!(parse_block(bad, "1").is_err(), "{bad}");
        }
        assert_eq!(parse_number("(214) 555-0142", "1").as_deref(), Ok("+12145550142"));
        assert!(parse_number("+1214555014", "1").is_err());
        assert!(parse_number("+12115550142", "1").is_err(), "N11 area code");
        assert_eq!(block_of("+12145550142"), "+1214555");
        assert_eq!(check_domain("Cell-Red.ecm.").as_deref(), Ok("cell-red.ecm"));
        for bad in ["ecm", "a b.ecm", "x.e164.arpa", "-a.ecm"] {
            assert!(check_domain(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn apply_and_duplicates() {
        let mut n = two_carriers();
        assert_eq!(n.blocks["+1214556"].0, "cell-blue.ecm");
        assert!(n.apply("1", "+1214555", "cell-blue.ecm", KB, KB, 3).unwrap_err().contains("already held by cell-red.ecm"));
        // a second block for a domain needs its key, and issues none
        assert!(n.apply("1", "+1972555", "cell-red.ecm", "", KB, 3).unwrap_err().contains("already has an ENUM key"));
        assert!(n.apply("1", "+1972555", "cell-red.ecm", KB, KB, 3).unwrap_err().contains("not the ENUM key"));
        let (msg, k) = n.apply("1", "+1972555", "cell-red.ecm", &KA.to_uppercase(), KB, 3).unwrap();
        assert!(k.is_none() && !msg.contains(KA));
        assert!(n.apply("1", "+1972556", "x.ecm", "", "short", 3).is_err());
        assert_eq!(n.blocks.len(), 3);
        assert!(n.list().contains("block +1972555 cell-red.ecm"));
        // the database round-trips through the IANA file
        let mut i = crate::iana::Iana::default();
        i.numbering = n.clone();
        i.tlds.insert("base".into(), ("alice".into(), 1));
        assert_eq!(crate::iana::Iana::parse(&i.render()), i);
        assert!(n.revoke("1", "+1972555").is_ok());
        assert!(n.revoke("1", "+1972555").is_err());
    }

    #[test]
    fn port_codes_and_overrides() {
        let mut n = two_carriers();
        let num = "+12145550142";
        let code = port_code(KA, num, "3fa2c1").unwrap();
        assert_eq!(code, "3fa2c1-b843a26d", "the vector ecm-cell's cored shares");
        // bad codes
        let wrong = port_code(KB, num, "3fa2c1").unwrap();
        assert!(n.port("1", num, "cell-blue.ecm", &wrong, KB, 5).unwrap_err().contains("wrong port-out code"));
        assert!(n.port("1", "+12145550143", "cell-blue.ecm", &code, KB, 5).unwrap_err().contains("wrong"), "another number");
        assert!(n.port("1", num, "cell-blue.ecm", "3fa2c1", KB, 5).is_err());
        assert!(n.port("1", num, "cell-blue.ecm", &code, KA, 5).unwrap_err().contains("not the ENUM key"));
        assert!(n.port("1", num, "nobody.ecm", &code, KA, 5).unwrap_err().contains("holds no ENUM key"));
        assert!(n.port("1", "+19725550142", "cell-blue.ecm", &code, KB, 5).unwrap_err().contains("no carrier holds"));
        assert!(n.ports.is_empty());
        // the good one, once
        assert!(n.port("1", num, "cell-blue.ecm", &code.to_uppercase(), KB, 5).unwrap().contains("ported to cell-blue.ecm"));
        assert_eq!(n.ports[num].0, "cell-blue.ecm");
        assert!(n.port("1", num, "cell-blue.ecm", &code, KB, 6).unwrap_err().contains("used already"));
        // back to the block's holder: the override goes
        let back = port_code(KA, num, "000001").unwrap();
        assert!(n.port("1", num, "cell-red.ecm", &back, KA, 7).unwrap().contains("back with cell-red.ecm"));
        assert!(n.ports.is_empty());
        // revoking a block drops its ports
        n.port("1", num, "cell-blue.ecm", &port_code(KA, num, "000002").unwrap(), KB, 8).unwrap();
        assert!(n.revoke("1", "+1214555").unwrap().contains("1 ported"));
        assert!(n.ports.is_empty() && n.used.is_empty());
    }

    #[test]
    fn zone_text() {
        let mut n = two_carriers();
        let num = "+12145550142";
        n.port("1", num, "cell-blue.ecm", &port_code(KA, num, "abcdef").unwrap(), KB, 5).unwrap();
        let z = n.zone(7, "iana.ring");
        assert_eq!(z.serial(), 7);
        let text = z.to_text();
        assert!(text.contains("5.5.5.4.1.2.1\t300\tIN\tNAPTR\t100 10 \"u\" \"E2U+sip\" \"!^.*$!sip:\\\\1@cell-red.ecm!\" ."), "{text}");
        // the file reads back the same, and the records rewrite to SIP as imsd does
        let again = Zone::parse(&text, E164_ARPA).unwrap();
        assert_eq!(again, z);
        let at = |name: &str| match z.lookup(name, NAPTR) {
            crate::auth::Lookup::Answer(r) => enum_sip_uri(naptrs(&r), num),
            _ => None,
        };
        assert_eq!(at("2.4.1.0.5.5.5.4.1.2.1.e164.arpa").as_deref(), Some("sip:+12145550142@cell-blue.ecm"));
        assert_eq!(at("5.5.5.4.1.2.1.e164.arpa").as_deref(), Some("sip:+12145550142@cell-red.ecm"));
        assert_eq!(at("6.5.5.4.1.2.1.e164.arpa").as_deref(), Some("sip:+12145550142@cell-blue.ecm"));
        let q = Message::query(1, "3.4.1.0.5.5.5.4.1.2.1.e164.arpa", NAPTR, false);
        let mut a = crate::auth::Authority::default();
        a.put(z);
        assert_eq!(a.respond(&q).unwrap().header.rcode, NXDOMAIN, "an unported number falls back to its block");
    }

    #[test]
    fn root_and_named_conf() {
        let root = Zone::parse("$TTL 86400\n@ SOA a.root-servers.ring. h.nic.ring. 5 300 60 600 300\n@ NS a.root-servers.ring.\nring. NS ns1.ring.ring.\n", "").unwrap();
        let z = delegate_root(&root, "iana.ring").unwrap();
        assert_eq!(z.serial(), 6);
        assert_eq!(z.get("e164.arpa", NS)[0].data, RData::Ns("iana.ring".into()));
        assert!(delegate_root(&z, "iana.ring").is_none(), "once");
        assert_eq!(delegate_root(&z, "ns.other").unwrap().get("e164.arpa", NS).len(), 1);
        let conf = "zone . primary /etc/named/root.zone";
        let c = named_conf_with_zone(conf, ZONE_FILE).unwrap();
        assert!(crate::conf::NamedConf::parse(&c).unwrap().zones.iter().any(|z| z.name == "e164.arpa"));
        assert!(named_conf_with_zone(&c, ZONE_FILE).is_none());
    }

    #[test]
    fn technical_check() {
        let q = Message::query(1, "_sip._udp.cell-red.ecm", SRV, true);
        let mut srv = Message::response_to(&q, 0);
        srv.answers.push(Record::new(
            "_sip._udp.cell-red.ecm",
            60,
            RData::Srv(crate::wire::Srv { priority: 10, weight: 10, port: 5060, target: "border.cell-red.ecm".into() }),
        ));
        let nx = Message::response_to(&q, NXDOMAIN);
        let mut a = Message::response_to(&Message::query(2, "cell-red.ecm", A, true), 0);
        a.answers.push(Record::new("cell-red.ecm", 60, RData::A([100, 65, 2, 1])));
        assert!(evaluate("cell-red.ecm", Ok(srv), Err("timeout".into())).unwrap().contains("border.cell-red.ecm.:5060"));
        assert!(evaluate("cell-red.ecm", Ok(nx.clone()), Ok(a)).unwrap().contains("100.65.2.1"));
        let e = evaluate("cell-red.ecm", Ok(nx), Err("timeout".into())).unwrap_err();
        assert!(e.contains("NXDOMAIN") && e.contains("timeout"), "{e}");
    }
}
