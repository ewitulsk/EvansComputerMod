//! The running core without its I/O: the `ecm_cell` core machine, the log and the
//! handover log, SIM provisioning through a Card Reader/Writer, and the admin
//! commands. `main.rs` owns the sockets, the tun and the database file.
//!
//! Admin commands (one per line on 127.0.0.1:7711; replies framed as in `ecm_cell::admin`):
//!
//! | Command | Reply body |
//! |---|---|
//! | `status` | `key value` lines, then `site ...` per connected site and `session ...` per phone |
//! | `subs` | `sub <imsi> <ip or -> <used> <quota> <revoked 0/1> <label or -> <online 0/1>` per subscriber |
//! | `sites` | `site <id> <connected 0/1> <peer or -> <cells or -> <last heartbeat ms ago or -> <up s or -> <rejected>` per configured site |
//! | `sessions` | `session <id> <imsi> <ip> <state> <site or -> <cell or -> <used> <quota>` |
//! | `log [n]`, `handovers [n]` | the last n lines of the log / of the handover log |
//! | `sim-new [label] [quota]` | write the blank SIM in the Card Reader/Writer (with the next free number when the carrier holds a block): `imsi`, `number`, `label`, `ejected` lines |
//! | `blocks`, `block-add <NPA-NXX>`, `block-remove <NPA-NXX>` | number blocks (docs/TELEPHONY.md) |
//! | `numbers`, `number <imsi\|label> [+number [ported]]`, `number-release <who>`, `number-move <+number> <who>` | who has which number: `num <number> <imsi> <label>` lines; `ported` for a number ported in from another carrier |
//! | `enum-key <hex>` | the ENUM key IANA issued with the blocks (`domain enum apply`) |
//! | `ims pool=<cidr> [pcscf=<ip>] [domain=<d>] [net=<cidr>] [mmsc=<ip>]`, `ims-off` | the IMS bearer (VoLTE, SMS over IMS, MMS) |
//! | `feature <who> <cfu\|cfb\|cfnr\|cfnrc\|cw\|clir\|dnd\|vm\|block\|unblock> <value>`, `port-code <who> [code]` | the TAS profile in the HSS; `port-code <who>` makes a port-out code signed with the enum key |
//! | `site-new <id>` | a new site with a random credential: `site`, `secret` lines |
//! | `add <imsi> <key> [quota] [label]`, `revoke <imsi>`, `unrevoke <imsi>`, `quota <imsi> <bytes>`, `reset-usage <imsi>`, `remove <imsi>`, `label <imsi> <label>`, `release <imsi>`, `site-add <id> <secret>`, `site-remove <id>` | the database operations (`ecm_cell::core::Core::admin`) |
//! | `stop` | `stopping` |

use crate::config::Conf;
use ecm_cell::codec::{fmt_ip, hex, parse_cidr, parse_ip};
use ecm_cell::core::{Core, CoreConfig, CoreOut, Entropy};
use ecm_cell::db::{valid_label, valid_site_id, Db};
use ecm_cell::simauth::valid_carrier;
use std::collections::VecDeque;

pub const LOG_LINES: usize = 300;
pub const HANDOVER_LINES: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Stop,
}

/// The card in the Card Reader/Writer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Card {
    Empty,
    Blank,
    Sim { imsi: String, carrier: String },
    Other(String),
}

/// The `card_reader` peripheral as provisioning needs it (a fake in tests).
pub trait CardWriter {
    fn card(&mut self) -> Result<Card, String>;
    /// Write the blank SIM in the reader (`msisdn`: its number, may be empty);
    /// `Ok(true)` when Auto Eject Sim pushed it out.
    fn write(&mut self, imsi: &str, carrier: &str, key_hex: &str, msisdn: &str) -> Result<bool, String>;
}

/// No reader (offline commands that don't write cards).
pub struct NoCard;
impl CardWriter for NoCard {
    fn card(&mut self) -> Result<Card, String> {
        Err("no Card Reader/Writer next to this computer".into())
    }
    fn write(&mut self, _: &str, _: &str, _: &str, _: &str) -> Result<bool, String> {
        Err("no Card Reader/Writer next to this computer".into())
    }
}

/// A new database: `carrier`, the phones' `pool` (CIDR), the core's address on its tun
/// (default: the pool's first address) and optional DNS server.
pub fn init_db(carrier: &str, pool: &str, gateway: Option<&str>, dns: Option<&str>) -> Result<String, String> {
    if !valid_carrier(carrier) {
        return Err(format!("bad carrier '{carrier}' (1-24 of a-z 0-9 _ -)"));
    }
    let (net, len) = parse_cidr(pool).ok_or_else(|| format!("bad pool '{pool}' (like 100.65.2.0/24)"))?;
    if !(8..=30).contains(&len) {
        return Err("the pool's prefix must be /8 to /30".into());
    }
    let base = u32::from_be_bytes(net) & (u32::MAX << (32 - len as u32));
    let gw = match gateway {
        Some(g) => parse_ip(g).ok_or_else(|| format!("bad gateway '{g}'"))?,
        None => (base + 1).to_be_bytes(),
    };
    let mut db = Db::new(carrier, base.to_be_bytes(), len, gw);
    if let Some(d) = dns {
        db.dns = parse_ip(d).ok_or_else(|| format!("bad dns '{d}'"))?;
    }
    let text = db.to_text();
    Db::parse(&text).map_err(|e| e.msg)?;
    Ok(text)
}

pub struct Daemon {
    pub core: Core,
    pub conf: Conf,
    pub log: VecDeque<String>,
    pub handovers: VecDeque<String>,
    pub started_ms: u64,
    rng: Box<dyn Entropy + Send>,
}

impl Daemon {
    pub fn new(db: Db, conf: Conf, core_entropy: Box<dyn Entropy + Send>, rng: Box<dyn Entropy + Send>, now_ms: u64) -> Daemon {
        Daemon { core: Core::new(db, CoreConfig::default(), core_entropy), conf, log: VecDeque::new(), handovers: VecDeque::new(), started_ms: now_ms, rng }
    }

    fn stamp(&self, now_ms: u64) -> String {
        let t = now_ms.saturating_sub(self.started_ms);
        format!("[{:>6}.{:01}]", t / 1000, (t % 1000) / 100)
    }

    pub fn log(&mut self, now_ms: u64, line: &str) {
        let l = format!("{} {}", self.stamp(now_ms), line);
        if line.starts_with("handover") || line.contains("handover ") {
            if self.handovers.len() >= HANDOVER_LINES {
                self.handovers.pop_front();
            }
            self.handovers.push_back(l.clone());
        }
        if self.log.len() >= LOG_LINES {
            self.log.pop_front();
        }
        self.log.push_back(l);
    }

    /// Take the core's outputs, logging its log lines; the rest go to the caller.
    pub fn take_out(&mut self, now_ms: u64) -> Vec<CoreOut> {
        let mut rest = Vec::new();
        for o in self.core.take_out() {
            match o {
                CoreOut::Log(l) => self.log(now_ms, &l),
                o => rest.push(o),
            }
        }
        rest
    }

    pub fn status_text(&self, now_ms: u64) -> String {
        let db = self.core.db();
        let sites = self.core.sites();
        let sessions = self.core.sessions();
        let mut s = String::new();
        let mut kv = |k: &str, v: String| s.push_str(&format!("{k} {v}\n"));
        kv("carrier", db.carrier.clone());
        kv("pool", format!("{}/{}", fmt_ip(db.pool), db.prefix));
        kv("gateway", fmt_ip(db.gateway));
        kv("db", self.conf.get("db"));
        kv("listen", self.conf.get("listen"));
        kv("tunnel_port", self.conf.get("tunnel_port"));
        kv("tun", self.conf.get("tun"));
        kv("uptime_s", (now_ms.saturating_sub(self.started_ms) / 1000).to_string());
        kv("subscribers", db.subs.len().to_string());
        kv("sites_configured", db.sites.len().to_string());
        kv("sites_connected", sites.len().to_string());
        kv("sessions", sessions.len().to_string());
        kv("tunnel_rejects", self.core.tunnel_rejects.to_string());
        // telephony (docs/TELEPHONY.md)
        kv("cc", db.cc.clone());
        kv("blocks", if db.blocks.is_empty() { "-".into() } else { db.blocks.join(",") });
        kv("enum_key", if db.enum_key.is_some() { "set" } else { "-" }.into());
        kv("numbers", db.subs.values().filter(|x| !x.ims.msisdn.is_empty()).count().to_string());
        match &db.ims {
            Some(i) => {
                kv("ims", "on".into());
                kv("ims_pool", format!("{}/{}", fmt_ip(i.pool), i.prefix));
                kv("pcscf", fmt_ip(i.pcscf));
                kv("ims_domain", i.domain.clone());
                kv("mmsc", i.mmsc.map(fmt_ip).unwrap_or_else(|| "-".into()));
            }
            None => kv("ims", "off".into()),
        }
        kv("hss_port", self.conf.get("hss_port"));
        kv("ims_policy_drops", self.core.ims_policy_drops.to_string());
        for x in &sites {
            s.push_str(&format!(
                "site {} {} {} {} {} {}\n",
                x.id,
                fmt_ip(x.peer),
                x.cells.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(","),
                now_ms.saturating_sub(x.since_ms) / 1000,
                now_ms.saturating_sub(x.last_rx_ms),
                x.rejected
            ));
        }
        s.push_str(&self.sessions_text());
        s
    }

    fn sessions_text(&self) -> String {
        let mut s = String::new();
        for x in self.core.sessions() {
            s.push_str(&format!(
                "session {} {} {} {} {} {} {} {}\n",
                x.session,
                x.imsi,
                fmt_ip(x.ip),
                x.state,
                x.site.clone().unwrap_or_else(|| "-".into()),
                x.cell.map(|c| c.to_string()).unwrap_or_else(|| "-".into()),
                x.used,
                x.quota
            ));
        }
        s
    }

    fn subs_text(&self) -> String {
        let online: Vec<String> = self.core.sessions().into_iter().map(|s| s.imsi).collect();
        let mut s = String::new();
        for (imsi, u) in &self.core.db().subs {
            s.push_str(&format!(
                "sub {} {} {} {} {} {} {}\n",
                imsi,
                u.ip.map(fmt_ip).unwrap_or_else(|| "-".into()),
                u.used,
                u.quota,
                u.revoked as u8,
                if u.label.is_empty() { "-" } else { &u.label },
                online.contains(imsi) as u8
            ));
        }
        s
    }

    fn sites_text(&self, now_ms: u64) -> String {
        let live = self.core.sites();
        let mut s = String::new();
        for id in self.core.db().sites.keys() {
            match live.iter().find(|x| &x.id == id) {
                Some(x) => s.push_str(&format!(
                    "site {} 1 {} {} {} {} {}\n",
                    id,
                    fmt_ip(x.peer),
                    x.cells.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(","),
                    now_ms.saturating_sub(x.last_rx_ms),
                    now_ms.saturating_sub(x.since_ms) / 1000,
                    x.rejected
                )),
                None => s.push_str(&format!("site {id} 0 - - - - 0\n")),
            }
        }
        s
    }

    fn digits(&mut self, n: usize) -> String {
        let mut out = String::new();
        while out.len() < n {
            let mut b = [0u8; 16];
            self.rng.fill(&mut b);
            for x in b {
                if x < 250 && out.len() < n {
                    out.push((b'0' + x % 10) as char);
                }
            }
        }
        out
    }

    /// Write the blank SIM in the reader for a new subscriber (random IMSI under the
    /// configured prefix, random key). The key goes into the database and onto the card,
    /// never anywhere else.
    pub fn sim_new(&mut self, label: Option<&str>, quota: u64, card: &mut dyn CardWriter, now_ms: u64) -> Result<String, String> {
        if let Some(l) = label {
            if !valid_label(l) {
                return Err(format!("bad label '{l}' (1-32 of a-z A-Z 0-9 . _ -)"));
            }
        }
        match card.card()? {
            Card::Blank => {}
            Card::Empty => return Err("no card in the Card Reader/Writer: right-click it with a blank SIM card".into()),
            Card::Sim { imsi, carrier } => return Err(format!("the card in the reader is already written (IMSI {imsi}, carrier {carrier}); insert a blank SIM")),
            Card::Other(t) => return Err(format!("the card in the reader is not a SIM ({t})")),
        }
        let prefix = self.conf.get("imsi_prefix");
        let imsi = loop {
            let i = format!("{prefix}{}", self.digits(15 - prefix.len()));
            if !self.core.db().subs.contains_key(&i) {
                break i;
            }
        };
        let mut k = [0u8; 16];
        self.rng.fill(&mut k);
        let key = hex(&k);
        let label_arg = label.map(|l| format!(" {l}")).unwrap_or_default();
        self.core.admin(&format!("add {imsi} {key} {quota}{label_arg}"), now_ms)?;
        // the next free number of the carrier's blocks, when it holds one
        let number = if self.core.db().blocks.is_empty() {
            String::new()
        } else {
            match self.core.admin(&format!("number {imsi}"), now_ms) {
                Ok(_) => self.core.db().subs[&imsi].ims.msisdn.clone(),
                Err(e) => {
                    let _ = self.core.admin(&format!("remove {imsi}"), now_ms);
                    return Err(format!("no number for the new SIM: {e}"));
                }
            }
        };
        let carrier = self.core.db().carrier.clone();
        match card.write(&imsi, &carrier, &key, &number) {
            Ok(ejected) => {
                let n = if number.is_empty() { String::new() } else { format!(", number {number}") };
                self.log(now_ms, &format!("SIM {imsi} written{}{n}", label.map(|l| format!(" ({l})")).unwrap_or_default()));
                Ok(format!("imsi {imsi}\nnumber {}\nlabel {}\nejected {}", if number.is_empty() { "-" } else { &number }, label.unwrap_or("-"), ejected as u8))
            }
            Err(e) => {
                let _ = self.core.admin(&format!("remove {imsi}"), now_ms);
                Err(format!("writing the SIM failed: {e}"))
            }
        }
    }

    /// Run one admin command.
    pub fn handle(&mut self, line: &str, now_ms: u64, card: &mut dyn CardWriter) -> (Result<String, String>, Action) {
        let w: Vec<&str> = line.split_whitespace().collect();
        let tail = |n: usize, q: &VecDeque<String>| {
            let skip = q.len().saturating_sub(n);
            q.iter().skip(skip).cloned().collect::<Vec<_>>().join("\n")
        };
        let r = match w.as_slice() {
            ["status"] => Ok(self.status_text(now_ms)),
            ["subs"] => Ok(self.subs_text()),
            ["sites"] => Ok(self.sites_text(now_ms)),
            ["sessions"] => Ok(self.sessions_text()),
            ["log"] | ["log", _] => Ok(tail(w.get(1).and_then(|n| n.parse().ok()).unwrap_or(50), &self.log)),
            ["handovers"] | ["handovers", _] => Ok(tail(w.get(1).and_then(|n| n.parse().ok()).unwrap_or(50), &self.handovers)),
            ["stop"] => return (Ok("stopping".into()), Action::Stop),
            ["sim-new"] => self.sim_new(None, 0, card, now_ms),
            ["sim-new", a] => match a.parse::<u64>() {
                Ok(q) => self.sim_new(None, q, card, now_ms),
                Err(_) => self.sim_new(Some(a), 0, card, now_ms),
            },
            ["sim-new", l, q] => match q.parse::<u64>() {
                Ok(q) => self.sim_new(Some(l), q, card, now_ms),
                Err(_) => Err("usage: sim-new [label] [quota bytes]".into()),
            },
            ["site-new", id] => {
                if !valid_site_id(id) {
                    Err(format!("bad site id '{id}' (1-32 of a-z A-Z 0-9 . _ -)"))
                } else if self.core.db().sites.contains_key(*id) {
                    Err(format!("site {id} exists (site-remove it first)"))
                } else {
                    let mut k = [0u8; 16];
                    self.rng.fill(&mut k);
                    let secret = hex(&k);
                    self.core.admin(&format!("site-add {id} {secret}"), now_ms).map(|_| format!("site {id}\nsecret {secret}"))
                }
            }
            [cmd, ..]
                if [
                    "add", "revoke", "unrevoke", "quota", "reset-usage", "remove", "label", "release", "site-add", "site-remove", "block-add", "block-remove", "blocks", "numbers", "number", "number-release",
                    "number-move", "feature", "port-code", "enum-key", "ims", "ims-off",
                ]
                .contains(cmd) =>
            {
                self.core.admin(line, now_ms)
            }
            [] => Err("empty command".into()),
            _ => Err(format!(
                "unknown command '{}' (status, subs, sites, sessions, log, handovers, sim-new, site-new, add, revoke, unrevoke, quota, reset-usage, remove, label, release, site-add, site-remove, blocks, block-add, block-remove, numbers, number, number-release, number-move, ims, ims-off, feature, port-code, enum-key, stop)",
                line.trim()
            )),
        };
        (r, Action::None)
    }
}

// ------------------------------------------------------------ replies, parsed (the Carrier Console)

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubView {
    pub imsi: String,
    pub ip: String,
    pub used: u64,
    pub quota: u64,
    pub revoked: bool,
    pub label: String,
    pub online: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteView {
    pub id: String,
    pub connected: bool,
    pub peer: String,
    pub cells: String,
    /// ms since the last heartbeat (None when not connected).
    pub heartbeat_ms: Option<u64>,
    pub up_s: Option<u64>,
    pub rejected: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionView {
    pub session: u32,
    pub imsi: String,
    pub ip: String,
    pub state: String,
    pub site: String,
    pub cell: String,
    pub used: u64,
    pub quota: u64,
}

pub fn parse_subs(body: &[String]) -> Vec<SubView> {
    body.iter()
        .filter_map(|l| match l.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["sub", imsi, ip, used, quota, revoked, label, online] => Some(SubView {
                imsi: imsi.to_string(),
                ip: ip.to_string(),
                used: used.parse().unwrap_or(0),
                quota: quota.parse().unwrap_or(0),
                revoked: *revoked == "1",
                label: if *label == "-" { String::new() } else { label.to_string() },
                online: *online == "1",
            }),
            _ => None,
        })
        .collect()
}

pub fn parse_sites(body: &[String]) -> Vec<SiteView> {
    body.iter()
        .filter_map(|l| match l.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["site", id, conn, peer, cells, hb, up, rej] => Some(SiteView {
                id: id.to_string(),
                connected: *conn == "1",
                peer: peer.to_string(),
                cells: cells.to_string(),
                heartbeat_ms: hb.parse().ok(),
                up_s: up.parse().ok(),
                rejected: rej.parse().unwrap_or(0),
            }),
            _ => None,
        })
        .collect()
}

pub fn parse_sessions(body: &[String]) -> Vec<SessionView> {
    body.iter()
        .filter_map(|l| match l.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["session", id, imsi, ip, state, site, cell, used, quota] => Some(SessionView {
                session: id.parse().unwrap_or(0),
                imsi: imsi.to_string(),
                ip: ip.to_string(),
                state: state.to_string(),
                site: site.to_string(),
                cell: cell.to_string(),
                used: used.parse().unwrap_or(0),
                quota: quota.parse().unwrap_or(0),
            }),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecm_cell::admin::value;

    struct Counter(u8);
    impl Entropy for Counter {
        fn fill(&mut self, b: &mut [u8]) {
            for x in b {
                self.0 = self.0.wrapping_add(37);
                *x = self.0;
            }
        }
    }

    /// A Card Reader/Writer in a test.
    #[derive(Default)]
    pub struct FakeReader {
        pub card: Option<Card>,
        pub written: Vec<(String, String, String)>,
        pub fail: Option<String>,
    }
    impl CardWriter for FakeReader {
        fn card(&mut self) -> Result<Card, String> {
            Ok(self.card.clone().unwrap_or(Card::Empty))
        }
        fn write(&mut self, imsi: &str, carrier: &str, key: &str, _msisdn: &str) -> Result<bool, String> {
            if let Some(e) = &self.fail {
                return Err(e.clone());
            }
            self.written.push((imsi.into(), carrier.into(), key.into()));
            self.card = Some(Card::Sim { imsi: imsi.into(), carrier: carrier.into() });
            Ok(false)
        }
    }

    fn daemon() -> Daemon {
        let db = Db::parse(&init_db("cell-red", "100.65.2.0/24", None, None).unwrap()).unwrap();
        Daemon::new(db, Conf::default(), Box::new(Counter(1)), Box::new(Counter(9)), 0)
    }

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(String::from).collect()
    }

    #[test]
    fn init_checks_its_input() {
        let t = init_db("cell-red", "100.65.2.0/24", None, Some("100.65.0.53")).unwrap();
        let db = Db::parse(&t).unwrap();
        assert_eq!(db.gateway, [100, 65, 2, 1]);
        assert_eq!(db.dns, [100, 65, 0, 53]);
        assert!(init_db("Cell", "100.65.2.0/24", None, None).is_err());
        assert!(init_db("cell-red", "100.65.2.0/31", None, None).is_err());
        assert!(init_db("cell-red", "100.65.2.0/24", Some("10.0.0.1"), None).unwrap_err().contains("outside the pool"));
    }

    #[test]
    fn sim_new_writes_a_blank_card_and_the_database() {
        let mut d = daemon();
        let mut r = FakeReader::default();
        assert!(d.handle("sim-new alice", 0, &mut r).0.unwrap_err().contains("no card in the Card Reader/Writer"));
        r.card = Some(Card::Blank);
        let body = d.handle("sim-new alice 5000", 0, &mut r).0.unwrap();
        let b = lines(&body);
        let imsi = value(&b, "imsi").unwrap().to_string();
        assert!(imsi.starts_with("00101") && imsi.len() == 15, "{imsi}");
        assert_eq!(value(&b, "label"), Some("alice"));
        assert_eq!(r.written.len(), 1);
        let (wi, wc, wk) = &r.written[0];
        assert_eq!((wi.as_str(), wc.as_str()), (imsi.as_str(), "cell-red"));
        let sub = &d.core.db().subs[&imsi];
        assert_eq!(&hex(&sub.key), wk, "the card and the database hold the same key");
        assert_eq!((sub.quota, sub.label.as_str()), (5000, "alice"));
        assert!(!body.contains(wk.as_str()), "the key is never shown");
        // control: the written card can't be written again
        assert!(d.handle("sim-new bob", 0, &mut r).0.unwrap_err().contains("already written"));
        // a failed write leaves no subscriber behind
        r.card = Some(Card::Blank);
        r.fail = Some("not blank".into());
        assert!(d.handle("sim-new", 0, &mut r).0.unwrap_err().contains("writing the SIM failed"));
        assert_eq!(d.core.db().subs.len(), 1);
        let subs = parse_subs(&lines(&d.handle("subs", 0, &mut r).0.unwrap()));
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].label, "alice");
        assert!(!subs[0].online);
        assert!(d.handle("sim-new two words here", 0, &mut r).0.is_err());
    }

    #[test]
    fn sites_and_database_commands() {
        let mut d = daemon();
        let mut r = FakeReader::default();
        let b = lines(&d.handle("site-new site-a", 0, &mut r).0.unwrap());
        let secret = value(&b, "secret").unwrap().to_string();
        assert_eq!(secret.len(), 32);
        assert!(d.handle("site-new site-a", 0, &mut r).0.unwrap_err().contains("exists"));
        let sites = parse_sites(&lines(&d.handle("sites", 0, &mut r).0.unwrap()));
        assert_eq!(sites, vec![SiteView { id: "site-a".into(), connected: false, peer: "-".into(), cells: "-".into(), heartbeat_ms: None, up_s: None, rejected: 0 }]);
        d.handle("add 001010000000042 00112233445566778899aabbccddeeff", 0, &mut r).0.unwrap();
        d.handle("quota 001010000000042 100", 0, &mut r).0.unwrap();
        d.handle("revoke 001010000000042", 0, &mut r).0.unwrap();
        let subs = parse_subs(&lines(&d.handle("subs", 0, &mut r).0.unwrap()));
        assert!(subs[0].revoked && subs[0].quota == 100);
        let saves = d.take_out(0).into_iter().filter(|o| matches!(o, CoreOut::SaveDb(_))).count();
        assert!(saves >= 3, "every change is saved");
        let st = lines(&d.handle("status", 2500, &mut r).0.unwrap());
        assert_eq!(value(&st, "carrier"), Some("cell-red"));
        assert_eq!(value(&st, "sites_configured"), Some("1"));
        assert_eq!(value(&st, "uptime_s"), Some("2"));
        assert!(d.handle("frobnicate", 0, &mut r).0.unwrap_err().contains("unknown command"));
        assert_eq!(d.handle("stop", 0, &mut r).1, Action::Stop);
        let log = d.handle("log", 0, &mut r).0.unwrap();
        assert!(log.contains("admin: 001010000000042 revoked"), "{log}");
    }

    #[test]
    fn enum_key_and_port_codes() {
        let mut d = daemon();
        let mut r = FakeReader::default();
        d.handle("block-add 214-555", 0, &mut r).0.unwrap();
        d.handle("add 001010000000042 00112233445566778899aabbccddeeff", 0, &mut r).0.unwrap();
        d.handle("number 001010000000042", 0, &mut r).0.unwrap();
        assert!(d.handle("port-code +12145550100", 0, &mut r).0.unwrap_err().contains("no enum key"));
        d.handle("enum-key 000102030405060708090a0b0c0d0e0f", 0, &mut r).0.unwrap();
        let st = lines(&d.handle("status", 0, &mut r).0.unwrap());
        assert_eq!(value(&st, "enum_key"), Some("set"));
        let code = d.handle("port-code +12145550100", 0, &mut r).0.unwrap();
        let code = code.rsplit(' ').next().unwrap();
        assert_eq!(code.len(), 15, "{code}");
        assert_eq!(*code, ecm_cell::db::port_code(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15], "+12145550100", &code[..6]));
        d.handle("add 001010000000043 00112233445566778899aabbccddeeff", 0, &mut r).0.unwrap();
        assert!(d.handle("number 001010000000043 +19725550123 ported", 0, &mut r).0.is_ok());
        let saved = d.take_out(0).into_iter().filter_map(|o| if let CoreOut::SaveDb(t) = o { Some(t) } else { None }).last().unwrap();
        assert!(saved.contains("enum key=000102030405060708090a0b0c0d0e0f") && saved.contains("ported=1"), "{saved}");
    }
}
