//! Zone files in RFC 1035 master format, the way a player writes them with `edit`:
//!
//! ```text
//! $ORIGIN lab.test.
//! $TTL 300
//! @     IN SOA ns1 admin ( 1 60 30 600 30 )
//!       IN NS  ns1
//! ns1      A   10.0.0.3
//! www  60  A   10.0.0.4
//! mail     MX  10 www
//! info     TXT "hello world"
//! ```
//!
//! Supported: `$ORIGIN`, `$TTL`, `@`, relative and absolute names, a blank owner
//! repeating the previous one, optional TTL and class in either order, parentheses
//! spanning lines, `;` comments, quoted strings, TTL units (`1h`, `2d`, `30m`).

use crate::name::{absolute, fqdn, relative};
use crate::wire::{rdata_from_text, type_from_name, RData, Record, Soa, NS, SOA};
use std::fmt::Write as _;

/// `300`, `5m`, `1h30m`, `2d`, `1w`.
pub fn parse_ttl(s: &str) -> Option<u32> {
    if let Ok(n) = s.parse::<u32>() {
        return Some(n);
    }
    let mut total: u64 = 0;
    let mut num: u64 = 0;
    let mut have = false;
    for c in s.chars() {
        if let Some(d) = c.to_digit(10) {
            num = num * 10 + d as u64;
            have = true;
            if num > u32::MAX as u64 {
                return None;
            }
        } else {
            let mult = match c.to_ascii_lowercase() {
                's' => 1,
                'm' => 60,
                'h' => 3600,
                'd' => 86400,
                'w' => 604800,
                _ => return None,
            };
            if !have {
                return None;
            }
            total += num * mult;
            num = 0;
            have = false;
        }
    }
    if have {
        return None; // trailing digits without a unit after a unit
    }
    u32::try_from(total).ok()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneError {
    pub line: usize,
    pub msg: String,
}

impl std::fmt::Display for ZoneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
    }
}

/// Split one logical entry into tokens: quoted strings keep spaces (and lose quotes),
/// `;` starts a comment, `(`/`)` are dropped (the caller joins lines).
fn tokenize(line: &str) -> Result<(Vec<String>, i32), String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut chars = line.chars().peekable();
    let mut cur = String::new();
    let mut in_token = false;
    while let Some(c) = chars.next() {
        match c {
            ';' => break,
            '"' => {
                let mut s = String::new();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(n) = chars.next() {
                                s.push(n);
                            }
                        }
                        '"' => {
                            closed = true;
                            break;
                        }
                        c => s.push(c),
                    }
                }
                if !closed {
                    return Err("unterminated quoted string".into());
                }
                if in_token {
                    out.push(std::mem::take(&mut cur));
                    in_token = false;
                }
                // Mark quoted tokens so "@" in a TXT stays text: a leading \u{1}.
                out.push(format!("\u{1}{s}"));
            }
            '(' | ')' => {
                depth += if c == '(' { 1 } else { -1 };
                if in_token {
                    out.push(std::mem::take(&mut cur));
                    in_token = false;
                }
            }
            c if c.is_whitespace() => {
                if in_token {
                    out.push(std::mem::take(&mut cur));
                    in_token = false;
                }
            }
            c => {
                cur.push(c);
                in_token = true;
            }
        }
    }
    if in_token {
        out.push(cur);
    }
    Ok((out, depth))
}

fn unquote(t: &str) -> &str {
    t.strip_prefix('\u{1}').unwrap_or(t)
}

/// Parse records from master-format text. `origin` is the starting `$ORIGIN`;
/// `default_ttl` applies until a `$TTL`.
pub fn parse_records(text: &str, origin: &str, default_ttl: u32) -> Result<Vec<Record>, ZoneError> {
    let mut origin = origin.to_string();
    let mut ttl_default = default_ttl;
    let mut last_owner: Option<String> = None;
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let start = i;
        let err = |msg: String| ZoneError { line: start + 1, msg };
        let first = lines[i];
        let (mut tokens, mut depth) = tokenize(first).map_err(err)?;
        i += 1;
        while depth > 0 {
            let Some(next) = lines.get(i) else {
                return Err(err("unclosed '('".into()));
            };
            let (t, d) = tokenize(next).map_err(|m| ZoneError { line: i + 1, msg: m })?;
            tokens.extend(t);
            depth += d;
            i += 1;
        }
        if depth < 0 {
            return Err(err("unbalanced ')'".into()));
        }
        if tokens.is_empty() {
            continue;
        }
        let blank_owner = first.starts_with(' ') || first.starts_with('\t');
        if !blank_owner && tokens[0].starts_with('$') {
            match tokens[0].to_ascii_uppercase().as_str() {
                "$ORIGIN" => {
                    let o = tokens.get(1).ok_or_else(|| err("$ORIGIN needs a name".into()))?;
                    origin = absolute(o, &origin).ok_or_else(|| err(format!("bad origin '{o}'")))?;
                }
                "$TTL" => {
                    let t = tokens.get(1).ok_or_else(|| err("$TTL needs a value".into()))?;
                    ttl_default = parse_ttl(t).ok_or_else(|| err(format!("bad TTL '{t}'")))?;
                }
                d => return Err(err(format!("unknown directive {d}"))),
            }
            continue;
        }
        let mut rest: &[String] = &tokens;
        let owner = if blank_owner {
            last_owner.clone().ok_or_else(|| err("record without an owner name".into()))?
        } else {
            let o = &rest[0];
            rest = &rest[1..];
            absolute(unquote(o), &origin).ok_or_else(|| err(format!("bad name '{o}'")))?
        };
        last_owner = Some(owner.clone());
        let mut ttl = None;
        let mut rtype = None;
        while let Some(t) = rest.first() {
            if t.eq_ignore_ascii_case("IN") {
                rest = &rest[1..];
            } else if ttl.is_none() && t.starts_with(|c: char| c.is_ascii_digit()) {
                ttl = Some(parse_ttl(t).ok_or_else(|| err(format!("bad TTL '{t}'")))?);
                rest = &rest[1..];
            } else {
                rtype = Some(type_from_name(t).ok_or_else(|| err(format!("unknown record type '{t}'")))?);
                rest = &rest[1..];
                break;
            }
        }
        let rtype = rtype.ok_or_else(|| err("missing record type".into()))?;
        let values: Vec<String> = rest.iter().map(|t| unquote(t).to_string()).collect();
        let data = rdata_from_text(rtype, &values, &origin).map_err(err)?;
        // RFC 2308: without $TTL the SOA's minimum is the default from there on.
        if let (RData::Soa(s), true) = (&data, ttl_default == u32::MAX) {
            ttl_default = s.minimum;
        }
        let ttl = ttl.unwrap_or(ttl_default);
        if ttl == u32::MAX {
            return Err(err("no TTL: add $TTL at the top".into()));
        }
        out.push(Record::new(&owner, ttl, data));
    }
    Ok(out)
}

/// A zone: its origin and records, the SOA first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Zone {
    pub origin: String,
    pub records: Vec<Record>,
}

impl Zone {
    /// Parse and check a zone: exactly one SOA, at the apex; at least one apex NS; every
    /// record at or below the origin; no CNAME next to other data.
    pub fn parse(text: &str, origin: &str) -> Result<Zone, ZoneError> {
        let records = parse_records(text, origin, u32::MAX)?;
        Zone::from_records(origin, records)
    }

    pub fn from_records(origin: &str, records: Vec<Record>) -> Result<Zone, ZoneError> {
        let e = |msg: String| ZoneError { line: 0, msg };
        let soas: Vec<&Record> = records.iter().filter(|r| r.rtype == SOA).collect();
        if soas.len() != 1 || soas[0].name != origin {
            return Err(e(format!("the zone needs exactly one SOA record at {}", fqdn(origin))));
        }
        if !records.iter().any(|r| r.rtype == NS && r.name == origin) {
            return Err(e(format!("the zone needs an NS record at {}", fqdn(origin))));
        }
        for r in &records {
            if !crate::name::is_sub(&r.name, origin) {
                return Err(e(format!("{} is outside the zone {}", fqdn(&r.name), fqdn(origin))));
            }
            if r.rtype == crate::wire::CNAME
                && records.iter().any(|o| o.name == r.name && o.rtype != crate::wire::CNAME)
            {
                return Err(e(format!("{} has a CNAME and other records", fqdn(&r.name))));
            }
        }
        let mut records = records;
        // SOA first, then in file order (stable).
        records.sort_by_key(|r| r.rtype != SOA);
        Ok(Zone { origin: origin.to_string(), records })
    }

    pub fn soa(&self) -> &Soa {
        self.records[0].soa().expect("SOA first")
    }

    pub fn soa_record(&self) -> &Record {
        &self.records[0]
    }

    pub fn serial(&self) -> u32 {
        self.soa().serial
    }

    /// The zone written back as a master file (relative names, one record per line).
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "$ORIGIN {}", fqdn(&self.origin));
        for r in &self.records {
            let _ = writeln!(
                s,
                "{}\t{}\tIN\t{}\t{}",
                relative(&r.name, &self.origin),
                r.ttl,
                crate::wire::type_name(r.rtype),
                rdata_rel(&r.data, &self.origin)
            );
        }
        s
    }

    /// Records with this owner name and type.
    pub fn get(&self, name: &str, rtype: u16) -> Vec<&Record> {
        self.records.iter().filter(|r| r.name == name && r.rtype == rtype).collect()
    }

    /// Set the SOA serial to `max(serial + 1, at_least)` (RFC 1982 arithmetic is not
    /// needed for our small counters).
    pub fn bump_serial(&mut self, at_least: u32) -> u32 {
        if let RData::Soa(s) = &mut self.records[0].data {
            s.serial = s.serial.wrapping_add(1).max(at_least);
            s.serial
        } else {
            0
        }
    }
}

fn rdata_rel(d: &RData, origin: &str) -> String {
    match d {
        RData::Ns(n) | RData::Cname(n) | RData::Ptr(n) => relative(n, origin),
        RData::Mx(p, n) => format!("{p} {}", relative(n, origin)),
        RData::Soa(s) => format!(
            "{} {} ( {} {} {} {} {} )",
            relative(&s.mname, origin),
            relative(&s.rname, origin),
            s.serial,
            s.refresh,
            s.retry,
            s.expire,
            s.minimum
        ),
        other => other.to_text(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::*;

    const LAB: &str = "$ORIGIN lab.test.\n$TTL 300\n@ IN SOA ns1 admin ( 1 ; serial\n 60 30 600 30 )\n  IN NS ns1\nns1 A 10.0.0.3\nwww 60 IN A 10.0.0.4\nweb CNAME www\nmail MX 10 www\ninfo TXT \"hello world\" \"two\"\nabs.lab.test. 1h A 10.0.0.9\n";

    #[test]
    fn parses_master_format() {
        assert!(Zone::parse(LAB, "").is_err(), "SOA owner is not the origin");
        let z = Zone::parse(LAB, "lab.test").unwrap();
        assert_eq!(z.serial(), 1);
        assert_eq!(z.soa().mname, "ns1.lab.test");
        assert_eq!(z.soa().rname, "admin.lab.test");
        assert_eq!(z.get("lab.test", NS)[0].data, RData::Ns("ns1.lab.test".into()));
        assert_eq!(z.get("www.lab.test", A)[0].ttl, 60);
        assert_eq!(z.get("ns1.lab.test", A)[0].ttl, 300);
        assert_eq!(z.get("web.lab.test", CNAME)[0].data, RData::Cname("www.lab.test".into()));
        assert_eq!(z.get("mail.lab.test", MX)[0].data, RData::Mx(10, "www.lab.test".into()));
        assert_eq!(z.get("info.lab.test", TXT)[0].data, RData::Txt(vec!["hello world".into(), "two".into()]));
        assert_eq!(z.get("abs.lab.test", A)[0].ttl, 3600);
        // written back and parsed again: the same zone
        let again = Zone::parse(&z.to_text(), "lab.test").unwrap();
        assert_eq!(again, z);
    }

    #[test]
    fn errors_have_line_numbers() {
        let e = Zone::parse("$TTL 60\n@ SOA a b 1 2 3 4 5\n@ NS a\nx A 1.2.3\n", "t").unwrap_err();
        assert_eq!(e.line, 4);
        assert!(Zone::parse("$TTL 60\n@ NS a\n", "t").is_err(), "no SOA");
        assert!(Zone::parse("$TTL 60\n@ SOA a b 1 2 3 4 5\n", "t").is_err(), "no NS");
        assert!(Zone::parse("$TTL 60\n@ SOA a b 1 2 3 4 5\n@ NS a\nx.other. A 1.1.1.1\n", "t").is_err());
        assert!(Zone::parse("$TTL 60\n@ SOA a b 1 2 3 4 5\n@ NS a\nx CNAME y\nx A 1.1.1.1\n", "t").is_err());
        assert!(Zone::parse("$TTL 60\n@ SOA a b ( 1 2 3 4 5\n", "t").is_err());
        assert!(Zone::parse("$TTL 60\nx BOGUS 1\n", "t").is_err());
        assert!(Zone::parse("@ NS a\n", "t").is_err(), "no TTL anywhere");
    }

    #[test]
    fn ttls() {
        assert_eq!(parse_ttl("300"), Some(300));
        assert_eq!(parse_ttl("1h30m"), Some(5400));
        assert_eq!(parse_ttl("2d"), Some(172800));
        assert_eq!(parse_ttl("h"), None);
        assert_eq!(parse_ttl("1h5"), None);
    }

    #[test]
    fn bump() {
        let mut z = Zone::parse(LAB, "lab.test").unwrap();
        assert_eq!(z.bump_serial(0), 2);
        assert_eq!(z.bump_serial(2026101601), 2026101601);
    }

    #[test]
    fn garbage_never_panics() {
        let mut rng = 99u32;
        let chars: Vec<char> = "@$();\" \n\tAINSOAabc.0123456789hm".chars().collect();
        for _ in 0..3000 {
            let mut s = String::new();
            for _ in 0..(rng % 80) {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                s.push(chars[rng as usize % chars.len()]);
            }
            let _ = Zone::parse(&s, "t");
        }
    }
}
