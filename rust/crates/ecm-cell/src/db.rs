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
//! by the core the first time it attaches), `label` (optional, the operator's
//! name for the SIM: 1-32 of `a-z A-Z 0-9 . _ -`). Parsing is strict: an unknown
//! line, key or bad value fails with its line number, so a typo never
//! silently drops a subscriber.
//!
//! Telephony (docs/TELEPHONY.md; all optional, so older files still load):
//!
//! ```text
//! cc 1                           the world's country code (default 1)
//! block 214-555                  a number block (NPA-NXX) the carrier holds, one line each
//! enum key=00112233...ff         the ENUM key IANA issued with the blocks (signs port-out codes)
//! ims pool=10.250.0.0/16 pcscf=100.65.2.1 domain=cell-red.ecm net=100.65.2.1/32
//! sub ... msisdn=+12145550142 ims_ip=10.250.0.2 cfu=+1... cfb=+1... cfnr=+1... cfnrc=+1...
//!         cw=1 clir=0 dnd=0 vm=1 block=+12145550199,+12145550198 port=3fa2c1-b843a26d ported=1
//! ```
//! `ims`: the IMS bearer's private pool (it routes only to `net`, the IMS core),
//! the P-CSCF the phones register with (default: the core's gateway, i.e. `imsd`
//! on the core's computer) and the carrier's SIP domain (default `<carrier>.ecm`).
//! Per subscriber: the number (`msisdn`, E.164), the sticky IMS address, call
//! forwarding (unconditional, busy, no answer, unreachable), call waiting,
//! caller-ID restriction, do not disturb, voicemail, the network block list, a
//! port-out code ([`port_code`]) and `ported=1` for a number ported in from another
//! carrier (outside the blocks). This is the HSS of a real network.

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
    /// The operator's name for the SIM (may be empty).
    pub label: String,
    /// IMS service profile and the number (HSS data).
    pub ims: ImsProfile,
}

/// What the HSS keeps for telephony per subscriber.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImsProfile {
    /// E.164 number, e.g. `+12145550142` (empty: none yet).
    pub msisdn: String,
    /// The sticky IMS bearer address.
    pub ims_ip: Option<[u8; 4]>,
    /// Call forwarding: unconditional, busy, no answer, not reachable (E.164 or empty).
    pub cfu: String,
    pub cfb: String,
    pub cfnr: String,
    pub cfnrc: String,
    /// Call waiting.
    pub cw: bool,
    /// Caller ID hidden by default (CLIR).
    pub clir: bool,
    /// Do not disturb (the network rejects calls).
    pub dnd: bool,
    /// Voicemail on.
    pub vm: bool,
    /// Numbers the network refuses calls and texts from.
    pub blocked: Vec<String>,
    /// Port-out code (empty: none issued).
    pub port: String,
    /// The number was ported in (it is outside the carrier's blocks).
    pub ported: bool,
}

impl Default for ImsProfile {
    fn default() -> Self {
        ImsProfile {
            msisdn: String::new(),
            ims_ip: None,
            cfu: String::new(),
            cfb: String::new(),
            cfnr: String::new(),
            cfnrc: String::new(),
            cw: true,
            clir: false,
            dnd: false,
            vm: true,
            blocked: Vec::new(),
            port: String::new(),
            ported: false,
        }
    }
}

/// `+` and 8-15 digits, not starting 0 (E.164).
pub fn valid_e164(s: &str) -> bool {
    s.strip_prefix('+').is_some_and(|d| (8..=15).contains(&d.len()) && d.bytes().all(|b| b.is_ascii_digit()) && !d.starts_with('0'))
}

/// A number block `NPA-NXX`: two 3-digit groups, each starting 2-9 and not `N11`.
pub fn valid_block(s: &str) -> bool {
    let ok3 = |g: &str| g.len() == 3 && g.bytes().all(|b| b.is_ascii_digit()) && g.as_bytes()[0] >= b'2' && &g[1..] != "11";
    s.split_once('-').is_some_and(|(a, b)| ok3(a) && ok3(b))
}

/// The IMS bearer configuration (`ims` line).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImsConf {
    pub pool: [u8; 4],
    pub prefix: u8,
    /// The P-CSCF (`imsd`) the phones register with.
    pub pcscf: [u8; 4],
    /// The carrier's SIP domain.
    pub domain: String,
    /// Where IMS-bearer traffic may go: the IMS core network.
    pub net: [u8; 4],
    pub net_prefix: u8,
    /// The MMSC (`mmscd`): traffic to and from it on the default bearer is zero-rated.
    pub mmsc: Option<[u8; 4]>,
}

/// A port-out code: 4-16 of letters, digits and `-`.
pub fn valid_port_code(s: &str) -> bool {
    (4..=16).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The port-out code for `number` with `nonce` (6 hex digits): `<nonce>-<mac>`, the mac the
/// first 8 hex digits of HMAC-SHA256(enum key, `ECM-PORT` || number || nonce). `ianad`
/// checks it against the key it issued with the number's block (`ecm_dns::numbering`).
pub fn port_code(key: &[u8; 16], number: &str, nonce: &str) -> String {
    let m = crate::crypto::hmac_sha256_parts(key, &[b"ECM-PORT", number.as_bytes(), nonce.as_bytes()]);
    format!("{}-{}", nonce, hex(&m[..4]))
}

/// A subscriber label: 1-32 of `a-z A-Z 0-9 . _ -`.
pub fn valid_label(s: &str) -> bool {
    (1..=32).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// A site id: same characters as a label.
pub fn valid_site_id(s: &str) -> bool {
    valid_label(s)
}

impl core::fmt::Debug for Subscriber {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Subscriber {{ key: <redacted>, quota: {}, used: {}, revoked: {}, ip: {:?}, label: {:?}, ims: {:?} }}", self.quota, self.used, self.revoked, self.ip, self.label, self.ims)
    }
}

impl Subscriber {
    pub fn new(key: [u8; 16]) -> Subscriber {
        Subscriber { key, quota: 0, used: 0, revoked: false, ip: None, label: String::new(), ims: ImsProfile::default() }
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
    /// Country code (`cc`).
    pub cc: String,
    /// Number blocks held, `NPA-NXX`.
    pub blocks: Vec<String>,
    /// The IMS bearer, when the carrier offers calls.
    pub ims: Option<ImsConf>,
    /// The ENUM key IANA issued with the carrier's blocks (`enum key=`).
    pub enum_key: Option<[u8; 16]>,
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
        Db { carrier: carrier.into(), pool, prefix, gateway, dns: [0; 4], mtu: 1500, sites: BTreeMap::new(), subs: BTreeMap::new(), cc: "1".into(), blocks: Vec::new(), ims: None, enum_key: None }
    }

    pub fn parse(text: &str) -> Result<Db, DbError> {
        let mut carrier = None;
        let mut pool = None;
        let mut gateway = None;
        let mut db = Db::new("", [0; 4], 0, [0; 4]);
        let mut version_seen = false;
        #[allow(clippy::type_complexity)]
        let mut ims_seen: Option<([u8; 4], u8, Option<[u8; 4]>, Option<String>, Option<([u8; 4], u8)>, Option<[u8; 4]>)> = None;
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
                ("cc", 2) => {
                    if !(1..=3).contains(&w[1].len()) || !w[1].bytes().all(|b| b.is_ascii_digit()) || w[1].starts_with('0') {
                        return Err(err(format!("bad country code '{}'", w[1])));
                    }
                    db.cc = w[1].to_string();
                }
                ("block", 2) => {
                    if !valid_block(w[1]) {
                        return Err(err(format!("bad number block '{}' (NPA-NXX, e.g. 214-555)", w[1])));
                    }
                    if db.blocks.iter().any(|b| b == w[1]) {
                        return Err(err(format!("block {} twice", w[1])));
                    }
                    db.blocks.push(w[1].to_string());
                }
                ("enum", 2) => {
                    let k = w[1].strip_prefix("key=").ok_or_else(|| err(format!("expected key=<32 hex>, got '{}'", w[1])))?;
                    db.enum_key = Some(unhex::<16>(k).ok_or_else(|| err("the enum key must be 32 hex digits".into()))?);
                }
                ("ims", n) if n >= 2 => {
                    let mut pool = None;
                    let mut pcscf = None;
                    let mut domain = None;
                    let mut net = None;
                    let mut mmsc = None;
                    for kv in &w[1..] {
                        let (k, v) = kv.split_once('=').ok_or_else(|| err(format!("expected key=value, got '{}'", kv)))?;
                        match k {
                            "pool" => pool = Some(parse_cidr(v).filter(|(_, l)| (8..=30).contains(l)).ok_or_else(|| err(format!("bad ims pool '{}'", v)))?),
                            "pcscf" => pcscf = Some(parse_ip(v).ok_or_else(|| err(format!("bad pcscf '{}'", v)))?),
                            "domain" => {
                                if v.is_empty() || v.len() > 63 || !v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') {
                                    return Err(err(format!("bad domain '{}'", v)));
                                }
                                domain = Some(v.to_ascii_lowercase());
                            }
                            "net" => net = Some(parse_cidr(v).ok_or_else(|| err(format!("bad ims net '{}'", v)))?),
                            "mmsc" => mmsc = Some(parse_ip(v).ok_or_else(|| err(format!("bad mmsc '{}'", v)))?),
                            _ => return Err(err(format!("unknown ims key '{}'", k))),
                        }
                    }
                    let (p, l) = pool.ok_or_else(|| err("ims needs pool=".into()))?;
                    ims_seen = Some((p, l, pcscf, domain, net, mmsc));
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
                            "label" => {
                                if !valid_label(v) {
                                    return Err(err(format!("bad label '{}'", v)));
                                }
                                s.label = v.to_string();
                            }
                            "msisdn" => {
                                if !valid_e164(v) {
                                    return Err(err(format!("bad msisdn '{}' (E.164, e.g. +12145550142)", v)));
                                }
                                s.ims.msisdn = v.to_string();
                            }
                            "ims_ip" => s.ims.ims_ip = Some(parse_ip(v).ok_or_else(|| err(format!("bad ims_ip '{}'", v)))?),
                            "cfu" | "cfb" | "cfnr" | "cfnrc" => {
                                if !v.is_empty() && !valid_e164(v) {
                                    return Err(err(format!("bad {} number '{}'", k, v)));
                                }
                                let slot = match k {
                                    "cfu" => &mut s.ims.cfu,
                                    "cfb" => &mut s.ims.cfb,
                                    "cfnr" => &mut s.ims.cfnr,
                                    _ => &mut s.ims.cfnrc,
                                };
                                *slot = v.to_string();
                            }
                            "cw" | "clir" | "dnd" | "vm" => {
                                let b = match v {
                                    "0" => false,
                                    "1" => true,
                                    _ => return Err(err(format!("{} must be 0 or 1", k))),
                                };
                                match k {
                                    "cw" => s.ims.cw = b,
                                    "clir" => s.ims.clir = b,
                                    "dnd" => s.ims.dnd = b,
                                    _ => s.ims.vm = b,
                                }
                            }
                            "block" => {
                                for n in v.split(',').filter(|n| !n.is_empty()) {
                                    if !valid_e164(n) {
                                        return Err(err(format!("bad blocked number '{}'", n)));
                                    }
                                    s.ims.blocked.push(n.to_string());
                                }
                            }
                            "port" => {
                                if !valid_port_code(v) {
                                    return Err(err(format!("bad port-out code '{}'", v)));
                                }
                                s.ims.port = v.to_string();
                            }
                            "ported" => {
                                s.ims.ported = match v {
                                    "0" => false,
                                    "1" => true,
                                    _ => return Err(err("ported must be 0 or 1".into())),
                                }
                            }
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
        if let Some((p, l, pcscf, domain, net, mmsc)) = ims_seen {
            let pcscf = pcscf.unwrap_or(db.gateway);
            let (net, net_prefix) = net.unwrap_or((pcscf, 32));
            db.ims = Some(ImsConf { pool: p, prefix: l, pcscf, domain: domain.unwrap_or_else(|| format!("{}.ecm", db.carrier)), net, net_prefix, mmsc });
        }
        let mut numbers = alloc::collections::BTreeSet::new();
        for (imsi, s) in &db.subs {
            if !s.ims.msisdn.is_empty() && !numbers.insert(s.ims.msisdn.clone()) {
                return Err(DbError { line: end, msg: format!("number {} is given to two subscribers (one is {})", s.ims.msisdn, imsi) });
            }
            if let (Some(ip), Some(ims)) = (s.ims.ims_ip, &db.ims) {
                if !in_subnet(ip, ims.pool, ims.prefix) {
                    return Err(DbError { line: end, msg: format!("subscriber {} has ims_ip {} outside the ims pool", imsi, fmt_ip(ip)) });
                }
            }
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
        if self.cc != "1" {
            let _ = writeln!(s, "cc {}", self.cc);
        }
        for b in &self.blocks {
            let _ = writeln!(s, "block {}", b);
        }
        if let Some(k) = &self.enum_key {
            let _ = writeln!(s, "enum key={}", hex(k));
        }
        if let Some(i) = &self.ims {
            let _ = write!(s, "ims pool={}/{} pcscf={} domain={} net={}/{}", fmt_ip(i.pool), i.prefix, fmt_ip(i.pcscf), i.domain, fmt_ip(i.net), i.net_prefix);
            if let Some(m) = i.mmsc {
                let _ = write!(s, " mmsc={}", fmt_ip(m));
            }
            s.push('\n');
        }
        for (id, k) in &self.sites {
            let _ = writeln!(s, "site {} {}", id, hex(k));
        }
        for (imsi, u) in &self.subs {
            let _ = write!(s, "sub {} key={} quota={} used={} revoked={}", imsi, hex(&u.key), u.quota, u.used, u.revoked as u8);
            if let Some(ip) = u.ip {
                let _ = write!(s, " ip={}", fmt_ip(ip));
            }
            if !u.label.is_empty() {
                let _ = write!(s, " label={}", u.label);
            }
            let p = &u.ims;
            if !p.msisdn.is_empty() {
                let _ = write!(s, " msisdn={}", p.msisdn);
            }
            if let Some(ip) = p.ims_ip {
                let _ = write!(s, " ims_ip={}", fmt_ip(ip));
            }
            for (k, v) in [("cfu", &p.cfu), ("cfb", &p.cfb), ("cfnr", &p.cfnr), ("cfnrc", &p.cfnrc)] {
                if !v.is_empty() {
                    let _ = write!(s, " {}={}", k, v);
                }
            }
            let d = ImsProfile::default();
            for (k, v, dv) in [("cw", p.cw, d.cw), ("clir", p.clir, d.clir), ("dnd", p.dnd, d.dnd), ("vm", p.vm, d.vm)] {
                if v != dv {
                    let _ = write!(s, " {}={}", k, v as u8);
                }
            }
            if !p.blocked.is_empty() {
                let _ = write!(s, " block={}", p.blocked.join(","));
            }
            if !p.port.is_empty() {
                let _ = write!(s, " port={}", p.port);
            }
            if p.ported {
                let _ = write!(s, " ported=1");
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

/// The first line handed out in a block (0000-0099 are kept for the carrier's own services).
pub const FIRST_LINE: u16 = 100;

impl Db {
    /// The subscriber with number `msisdn`.
    pub fn by_number(&self, msisdn: &str) -> Option<&str> {
        self.subs.iter().find(|(_, s)| s.ims.msisdn == msisdn).map(|(i, _)| i.as_str())
    }

    /// The E.164 number of line `line` in `block` (`NPA-NXX`).
    pub fn block_number(&self, block: &str, line: u16) -> String {
        let (a, b) = block.split_once('-').unwrap_or((block, ""));
        format!("+{}{}{}{:04}", self.cc, a, b, line)
    }

    /// The block (`NPA-NXX`) a number of this carrier belongs to.
    pub fn block_of(&self, msisdn: &str) -> Option<&str> {
        self.blocks.iter().find(|b| msisdn.starts_with(&self.block_number(b, 0)[..2 + self.cc.len() + 5])).map(|b| b.as_str())
    }

    /// The next free number in the carrier's blocks (lowest block, lowest line from [`FIRST_LINE`]).
    pub fn next_free_number(&self) -> Option<String> {
        let used: alloc::collections::BTreeSet<&str> = self.subs.values().map(|s| s.ims.msisdn.as_str()).collect();
        for b in &self.blocks {
            for line in FIRST_LINE..=9999 {
                let n = self.block_number(b, line);
                if !used.contains(n.as_str()) {
                    return Some(n);
                }
            }
        }
        None
    }

    /// Give `imsi` a number: `requested` (which must be free and, once the carrier holds
    /// blocks, in one of them unless `ported` in from another carrier) or the next free one.
    /// Returns the number.
    pub fn assign_number(&mut self, imsi: &str, requested: Option<&str>, ported: bool) -> Result<String, String> {
        if !self.subs.contains_key(imsi) {
            return Err(format!("no subscriber {}", imsi));
        }
        let n = match requested {
            Some(r) => {
                if !valid_e164(r) {
                    return Err(format!("bad number '{}' (E.164, e.g. +12145550142)", r));
                }
                if let Some(other) = self.by_number(r) {
                    if other != imsi {
                        return Err(format!("{} already belongs to {}", r, other));
                    }
                }
                let ours = self.block_of(r).is_some();
                if ported && ours {
                    return Err(format!("{} is in this carrier's block {}: it is not ported in", r, self.block_of(r).unwrap_or("")));
                }
                if !ported && !ours && !self.blocks.is_empty() {
                    return Err(format!("{} is outside this carrier's blocks (add 'ported' for a number ported in)", r));
                }
                r.to_string()
            }
            None if ported => return Err("a ported number must be given".into()),
            None => self.next_free_number().ok_or_else(|| if self.blocks.is_empty() { "no number block configured (cored block add NPA-NXX)".to_string() } else { "every number in the blocks is taken".to_string() })?,
        };
        let p = &mut self.subs.get_mut(imsi).expect("checked").ims;
        p.msisdn = n.clone();
        p.ported = ported;
        Ok(n)
    }

    /// Give every subscriber without a number one (once a block exists). Returns how many.
    pub fn fill_numbers(&mut self) -> usize {
        if self.blocks.is_empty() {
            return 0;
        }
        let need: Vec<String> = self.subs.iter().filter(|(_, s)| s.ims.msisdn.is_empty()).map(|(i, _)| i.clone()).collect();
        let mut n = 0;
        for imsi in need {
            if self.assign_number(&imsi, None, false).is_ok() {
                n += 1;
            }
        }
        n
    }

    /// The subscriber's sticky IMS bearer address (lowest free in the IMS pool the first time).
    pub fn ims_address_for(&mut self, imsi: &str) -> Option<[u8; 4]> {
        let ims = self.ims.clone()?;
        if let Some(ip) = self.subs.get(imsi)?.ims.ims_ip {
            return Some(ip);
        }
        let used: alloc::collections::BTreeSet<[u8; 4]> = self.subs.values().filter_map(|s| s.ims.ims_ip).collect();
        let base = u32::from_be_bytes(ims.pool) & (u32::MAX << (32 - ims.prefix as u32));
        let size = 1u32 << (32 - ims.prefix as u32);
        for host in 2..size - 1 {
            let ip = (base + host).to_be_bytes();
            if !used.contains(&ip) {
                self.subs.get_mut(imsi)?.ims.ims_ip = Some(ip);
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
        sub 310150000000002 key=00112233445566778899aabbccddeeff revoked=1 label=alice-phone\n";

    #[test]
    fn parse_write_round_trip() {
        let db = Db::parse(SAMPLE).unwrap();
        assert_eq!(db.carrier, "cell-red");
        assert_eq!(db.subs.len(), 2);
        assert_eq!(db.subs["310150000000001"].quota, 5000);
        assert!(db.subs["310150000000002"].revoked);
        assert_eq!(db.subs["310150000000002"].label, "alice-phone");
        assert!(Db::parse(&SAMPLE.replace("label=alice-phone", "label=a+b")).is_err());
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

    #[test]
    fn numbers_and_ims() {
        let head = SAMPLE.split("site site-a").next().unwrap();
        let text = format!(
            "{}cc 1\nblock 214-555\nims pool=10.250.0.0/24 domain=cell-red.ecm\nsite site-a 000102030405060708090a0b0c0d0e0f\n\
             sub 310150000000001 key=00112233445566778899aabbccddeeff msisdn=+12145550142 cfnr=+12145550100 dnd=1 block=+12145550199\n\
             sub 310150000000002 key=00112233445566778899aabbccddeeff\n",
            head
        );
        let mut db = Db::parse(&text).unwrap();
        let ims = db.ims.clone().unwrap();
        assert_eq!(ims.pcscf, [100, 65, 2, 1], "pcscf defaults to the gateway");
        assert_eq!((ims.net, ims.net_prefix), ([100, 65, 2, 1], 32));
        let a = &db.subs["310150000000001"].ims;
        assert_eq!(a.msisdn, "+12145550142");
        assert!(a.dnd && a.cw && a.vm && !a.clir);
        assert_eq!(a.blocked, alloc::vec!["+12145550199".to_string()]);
        assert_eq!(db.by_number("+12145550142"), Some("310150000000001"));
        assert_eq!(db.block_of("+12145550142"), Some("214-555"));
        assert_eq!(db.block_of("+12145560142"), None);
        // the next free number starts at line 0100
        assert_eq!(db.fill_numbers(), 1);
        assert_eq!(db.subs["310150000000002"].ims.msisdn, "+12145550100");
        assert!(db.assign_number("310150000000002", Some("+12145550142"), false).is_err(), "taken");
        assert_eq!(db.ims_address_for("310150000000001"), Some([10, 250, 0, 2]));
        assert_eq!(db.ims_address_for("310150000000002"), Some([10, 250, 0, 3]));
        let again = Db::parse(&db.to_text()).unwrap();
        assert_eq!(again, db);
        assert!(db.to_text().contains("dnd=1"));
        assert!(!db.to_text().contains("cw="), "defaults are not written");
        for bad in ["block 211-555", "block 214-5555", "ims pcscf=1.2.3.4", "cc 01"] {
            assert!(Db::parse(&format!("{}{}\n", text, bad)).is_err(), "{}", bad);
        }
        let dup = text.replace(
            "sub 310150000000002 key=00112233445566778899aabbccddeeff",
            "sub 310150000000002 key=00112233445566778899aabbccddeeff msisdn=+12145550142",
        );
        assert!(Db::parse(&dup).unwrap_err().msg.contains("two subscribers"));
        let mut none = Db::parse(SAMPLE).unwrap();
        assert_eq!(none.fill_numbers(), 0);
        assert!(none.assign_number("310150000000001", None, false).unwrap_err().contains("no number block"));
        assert_eq!(none.ims_address_for("310150000000001"), None);
    }

    #[test]
    fn enum_key_and_ported_numbers() {
        let head = SAMPLE.split("site site-a").next().unwrap();
        let text = format!(
            "{}block 214-555\nenum key=000102030405060708090a0b0c0d0e0f\n\
             sub 310150000000001 key=00112233445566778899aabbccddeeff msisdn=+19725550123 ported=1 port=3fa2c1-b843a26d\n\
             sub 310150000000002 key=00112233445566778899aabbccddeeff\n",
            head
        );
        let mut db = Db::parse(&text).unwrap();
        assert_eq!(db.enum_key, Some([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]));
        assert!(db.subs["310150000000001"].ims.ported);
        assert_eq!(Db::parse(&db.to_text()).unwrap(), db, "round trip");
        assert!(db.to_text().contains("enum key=000102030405060708090a0b0c0d0e0f\n"));
        assert!(db.to_text().contains(" ported=1"));
        for bad in ["enum key=00", "enum 000102030405060708090a0b0c0d0e0f"] {
            assert!(Db::parse(&format!("{}{}\n", text, bad)).is_err(), "{}", bad);
        }
        // a number outside the blocks only when ported in, one inside never
        let b = "310150000000002";
        assert!(db.assign_number(b, Some("+13125550100"), false).unwrap_err().contains("outside"));
        assert!(db.assign_number(b, Some("+12145550100"), true).unwrap_err().contains("not ported in"));
        assert!(db.assign_number(b, None, true).is_err());
        assert_eq!(db.assign_number(b, Some("+13125550100"), true).as_deref(), Ok("+13125550100"));
        assert!(db.subs[b].ims.ported);
        assert_eq!(db.assign_number(b, Some("+12145550100"), false).as_deref(), Ok("+12145550100"));
        assert!(!db.subs[b].ims.ported);
        // the code ianad checks (ecm_dns::numbering shares this vector)
        assert_eq!(port_code(&db.enum_key.unwrap(), "+12145550142", "3fa2c1"), "3fa2c1-b843a26d");
    }
}
