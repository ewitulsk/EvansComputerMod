//! The carrier core's subscriber database: a text file `cored` owns on its
//! computer (default `/etc/cored.db`), rewritten whole on every change.
//!
//! ```text
//! # anything after '#' is a comment
//! version 1
//! carrier cell-red
//! pool 100.65.2.0/24             phones' addresses (the core's tun is `gateway`)
//! gateway 100.65.2.1
//! dns 100.65.0.53                optional
//! mtu 1500                       optional (default 1500)
//! site site-a 000102...0f        site id and its 16-byte credential (hex)
//! sub 310150000000001 key=00112233445566778899aabbccddeeff quota=0 used=0 revoked=0 ip=100.65.2.10
//! ```
//! `sub` keys: `key` (required), `quota` (bytes, 0 = unlimited), `used`
//! (bytes so far), `revoked` (0/1), `ip` (the phone's sticky address, written
//! by the core the first time it attaches). Parsing is strict: an unknown
//! line, key or bad value fails with its line number, so a typo never
//! silently drops a subscriber.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;

use crate::codec::{fmt_ip, hex, parse_cidr, parse_ip, unhex};
use crate::ip::in_subnet;
use crate::simauth::{valid_carrier, valid_imsi};

#[derive(Clone, PartialEq, Eq)]
pub struct Subscriber {
    pub key: [u8; 16],
    pub quota: u64,
    pub used: u64,
    pub revoked: bool,
    pub ip: Option<[u8; 4]>,
}

impl core::fmt::Debug for Subscriber {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Subscriber {{ key: <redacted>, quota: {}, used: {}, revoked: {}, ip: {:?} }}", self.quota, self.used, self.revoked, self.ip)
    }
}

impl Subscriber {
    pub fn new(key: [u8; 16]) -> Subscriber {
        Subscriber { key, quota: 0, used: 0, revoked: false, ip: None }
    }
    pub fn over_quota(&self) -> bool {
        self.quota > 0 && self.used >= self.quota
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Db {
    pub carrier: String,
    pub pool: [u8; 4],
    pub prefix: u8,
    pub gateway: [u8; 4],
    pub dns: [u8; 4],
    pub mtu: u16,
    pub sites: BTreeMap<String, [u8; 16]>,
    pub subs: BTreeMap<String, Subscriber>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbError {
    pub line: usize,
    pub msg: String,
}

impl core::fmt::Display for DbError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
    }
}

impl Db {
    pub fn new(carrier: &str, pool: [u8; 4], prefix: u8, gateway: [u8; 4]) -> Db {
        Db { carrier: carrier.into(), pool, prefix, gateway, dns: [0; 4], mtu: 1500, sites: BTreeMap::new(), subs: BTreeMap::new() }
    }

    pub fn parse(text: &str) -> Result<Db, DbError> {
        let mut carrier = None;
        let mut pool = None;
        let mut gateway = None;
        let mut db = Db::new("", [0; 4], 0, [0; 4]);
        let mut version_seen = false;
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let err = |msg: String| DbError { line, msg };
            let l = raw.split('#').next().unwrap_or("").trim();
            if l.is_empty() {
                continue;
            }
            let w: Vec<&str> = l.split_whitespace().collect();
            match (w[0], w.len()) {
                ("version", 2) => {
                    if w[1] != "1" {
                        return Err(err(format!("unsupported version {}", w[1])));
                    }
                    version_seen = true;
                }
                ("carrier", 2) => {
                    if !valid_carrier(w[1]) {
                        return Err(err(format!("bad carrier '{}'", w[1])));
                    }
                    carrier = Some(w[1].to_string());
                }
                ("pool", 2) => {
                    let (net, len) = parse_cidr(w[1]).ok_or_else(|| err(format!("bad pool '{}'", w[1])))?;
                    if !(8..=30).contains(&len) {
                        return Err(err("pool prefix must be /8../30".into()));
                    }
                    pool = Some((net, len));
                }
                ("gateway", 2) => gateway = Some(parse_ip(w[1]).ok_or_else(|| err(format!("bad gateway '{}'", w[1])))?),
                ("dns", 2) => db.dns = parse_ip(w[1]).ok_or_else(|| err(format!("bad dns '{}'", w[1])))?,
                ("mtu", 2) => {
                    db.mtu = w[1].parse().ok().filter(|m| (576..=1500).contains(m)).ok_or_else(|| err("mtu must be 576..1500".into()))?;
                }
                ("site", 3) => {
                    let secret = unhex::<16>(w[2]).ok_or_else(|| err("site secret must be 32 hex digits".into()))?;
                    if w[1].is_empty() || w[1].len() > 32 {
                        return Err(err("site id must be 1..32 characters".into()));
                    }
                    if db.sites.insert(w[1].to_string(), secret).is_some() {
                        return Err(err(format!("site '{}' twice", w[1])));
                    }
                }
                ("sub", n) if n >= 3 => {
                    let imsi = w[1];
                    if !valid_imsi(imsi) {
                        return Err(err(format!("bad IMSI '{}'", imsi)));
                    }
                    let mut s = Subscriber::new([0; 16]);
                    let mut have_key = false;
                    for kv in &w[2..] {
                        let (k, v) = kv.split_once('=').ok_or_else(|| err(format!("expected key=value, got '{}'", kv)))?;
                        match k {
                            "key" => {
                                s.key = unhex::<16>(v).ok_or_else(|| err("key must be 32 hex digits".into()))?;
                                have_key = true;
                            }
                            "quota" => s.quota = v.parse().map_err(|_| err(format!("bad quota '{}'", v)))?,
                            "used" => s.used = v.parse().map_err(|_| err(format!("bad used '{}'", v)))?,
                            "revoked" => {
                                s.revoked = match v {
                                    "0" => false,
                                    "1" => true,
                                    _ => return Err(err("revoked must be 0 or 1".into())),
                                }
                            }
                            "ip" => s.ip = Some(parse_ip(v).ok_or_else(|| err(format!("bad ip '{}'", v)))?),
                            _ => return Err(err(format!("unknown key '{}'", k))),
                        }
                    }
                    if !have_key {
                        return Err(err(format!("subscriber {} has no key", imsi)));
                    }
                    if db.subs.insert(imsi.to_string(), s).is_some() {
                        return Err(err(format!("subscriber {} twice", imsi)));
                    }
                }
                _ => return Err(err(format!("unknown line '{}'", l))),
            }
        }
        let end = text.lines().count();
        let err = |msg: &str| DbError { line: end, msg: msg.into() };
        if !version_seen {
            return Err(err("missing 'version 1'"));
        }
        db.carrier = carrier.ok_or_else(|| err("missing 'carrier'"))?;
        let (net, len) = pool.ok_or_else(|| err("missing 'pool'"))?;
        db.pool = net;
        db.prefix = len;
        db.gateway = gateway.ok_or_else(|| err("missing 'gateway'"))?;
        if !in_subnet(db.gateway, db.pool, db.prefix) {
            return Err(err("gateway is outside the pool"));
        }
        for (imsi, s) in &db.subs {
            if let Some(ip) = s.ip {
                if !in_subnet(ip, db.pool, db.prefix) || ip == db.gateway {
                    return Err(DbError { line: end, msg: format!("subscriber {} has ip {} outside the pool", imsi, fmt_ip(ip)) });
                }
            }
        }
        Ok(db)
    }

    pub fn to_text(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "# ecm-cell subscriber database (owned by cored; edit with the Carrier Console or cored's admin commands)");
        let _ = writeln!(s, "version 1");
        let _ = writeln!(s, "carrier {}", self.carrier);
        let _ = writeln!(s, "pool {}/{}", fmt_ip(self.pool), self.prefix);
        let _ = writeln!(s, "gateway {}", fmt_ip(self.gateway));
        if self.dns != [0; 4] {
            let _ = writeln!(s, "dns {}", fmt_ip(self.dns));
        }
        let _ = writeln!(s, "mtu {}", self.mtu);
        for (id, k) in &self.sites {
            let _ = writeln!(s, "site {} {}", id, hex(k));
        }
        for (imsi, u) in &self.subs {
            let _ = write!(s, "sub {} key={} quota={} used={} revoked={}", imsi, hex(&u.key), u.quota, u.used, u.revoked as u8);
            if let Some(ip) = u.ip {
                let _ = write!(s, " ip={}", fmt_ip(ip));
            }
            s.push('\n');
        }
        s
    }

    /// The subscriber's sticky address, assigning the lowest free one the first time.
    pub fn address_for(&mut self, imsi: &str) -> Option<[u8; 4]> {
        if let Some(ip) = self.subs.get(imsi)?.ip {
            return Some(ip);
        }
        let used: alloc::collections::BTreeSet<[u8; 4]> = self.subs.values().filter_map(|s| s.ip).collect();
        let base = u32::from_be_bytes(self.pool) & (u32::MAX << (32 - self.prefix as u32));
        let size = 1u32 << (32 - self.prefix as u32);
        for host in 1..size - 1 {
            let ip = (base + host).to_be_bytes();
            if ip != self.gateway && !used.contains(&ip) {
                self.subs.get_mut(imsi)?.ip = Some(ip);
                return Some(ip);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# test\nversion 1\ncarrier cell-red\npool 100.65.2.0/24\ngateway 100.65.2.1\ndns 100.65.0.53\n\
        site site-a 000102030405060708090a0b0c0d0e0f\n\
        sub 310150000000001 key=00112233445566778899aabbccddeeff quota=5000 used=10 revoked=0 ip=100.65.2.10\n\
        sub 310150000000002 key=00112233445566778899aabbccddeeff revoked=1\n";

    #[test]
    fn parse_write_round_trip() {
        let db = Db::parse(SAMPLE).unwrap();
        assert_eq!(db.carrier, "cell-red");
        assert_eq!(db.subs.len(), 2);
        assert_eq!(db.subs["310150000000001"].quota, 5000);
        assert!(db.subs["310150000000002"].revoked);
        let again = Db::parse(&db.to_text()).unwrap();
        assert_eq!(again, db);
        assert!(!alloc::format!("{:?}", db).contains("00112233"), "Debug hides keys");
    }

    #[test]
    fn errors_name_the_line() {
        for (bad, line) in [
            ("version 1\ncarrier cell-red\npool 100.65.2.0/24\ngateway 100.65.2.1\nsub 31015 key=00\n", 5),
            ("version 1\ncarrier cell-red\nfrobnicate\n", 3),
            ("version 1\ncarrier Cell\n", 2),
            ("version 1\ncarrier c\npool 100.65.2.0/24\ngateway 100.65.2.1\nsub 310150000000001 key=00112233445566778899aabbccddeeff colour=red\n", 5),
            ("version 1\ncarrier c\npool 100.65.2.0/24\ngateway 100.65.3.1\n", 4),
        ] {
            let e = Db::parse(bad).unwrap_err();
            assert_eq!(e.line, line, "{}: {}", bad, e);
        }
        assert!(Db::parse("carrier c\npool 10.0.0.0/24\ngateway 10.0.0.1\n").is_err(), "no version");
    }

    #[test]
    fn sticky_addresses() {
        let mut db = Db::parse(SAMPLE).unwrap();
        assert_eq!(db.address_for("310150000000001"), Some([100, 65, 2, 10]));
        // .1 is the gateway, .10 is taken
        assert_eq!(db.address_for("310150000000002"), Some([100, 65, 2, 2]));
        assert_eq!(db.address_for("310150000000002"), Some([100, 65, 2, 2]));
        assert_eq!(db.address_for("999999"), None);
        let mut small = Db::parse("version 1\ncarrier c\npool 10.0.0.0/30\ngateway 10.0.0.1\nsub 111111 key=00000000000000000000000000000000\nsub 222222 key=00000000000000000000000000000000\n").unwrap();
        assert_eq!(small.address_for("111111"), Some([10, 0, 0, 2]));
        assert_eq!(small.address_for("222222"), None, "pool exhausted");
    }
}
