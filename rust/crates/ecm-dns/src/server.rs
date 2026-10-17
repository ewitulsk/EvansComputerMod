//! `named` without the sockets: authoritative zones (primary from a file, secondary by
//! transfer), recursion through the [`Resolver`] for allowed clients, NOTIFY out when a
//! primary zone changes, SOA checks and AXFR requests for secondaries, and the control
//! commands (`reload`, `cache`, `flush`, `status`, `retransfer`).
//!
//! The program (`wasm-programs/named`) owns a UDP socket on port 53 (queries, NOTIFY,
//! SOA checks), a second UDP socket for the resolver, a TCP listener on 53 (AXFR and
//! TCP queries), TCP connections for its own transfers, and the control socket; it moves
//! bytes between them and this state machine.

use crate::auth::{answer_serial, Authority};
use crate::conf::{NamedConf, ZoneKind};
use crate::name::{fqdn, ip_str, Ip};
use crate::resolver::{self, QueryId, Resolver};
use crate::wire::*;
use crate::xfr::{self, serial_newer};
use crate::zone::Zone;

pub const NOTIFY_RETRY_MS: i64 = 2000;
pub const NOTIFY_TRIES: u8 = 4;
/// Bounds on the SOA refresh / retry timers of secondaries (seconds).
pub const MIN_REFRESH: u32 = 15;
pub const MAX_REFRESH: u32 = 3600;
/// A secondary without a copy of its zone checks the primary again this soon.
pub const UNLOADED_RETRY_MS: i64 = 3000;
/// Client queries waiting for the resolver at most.
pub const MAX_RECURSIVE_CLIENTS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    /// A datagram from the port-53 socket.
    Udp { to: Ip, port: u16, data: Vec<u8> },
    /// A datagram from the resolver's socket (to port 53).
    Resolver { to: Ip, data: Vec<u8> },
    /// Fetch `zone` from `primary` by AXFR over TCP and call [`Server::on_transfer`].
    Transfer { zone: String, primary: Ip },
    /// Write a secondary's copy of a zone.
    Save { file: String, text: String },
    Log(String),
}

#[derive(Clone, Debug)]
struct Secondary {
    zone: String,
    primary: Ip,
    file: String,
    next_check: i64,
    soa_query: Option<(u16, i64)>,
    transferring: bool,
}

#[derive(Clone, Debug)]
struct Notify {
    zone: String,
    to: Ip,
    id: u16,
    tries: u8,
    next: i64,
}

#[derive(Clone, Debug)]
struct Client {
    q: QueryId,
    from: Ip,
    port: u16,
    query: Message,
}

pub struct Server {
    pub conf: NamedConf,
    pub auth: Authority,
    pub resolver: Option<Resolver>,
    /// This computer's addresses (NOTIFY is not sent to them).
    pub own: Vec<Ip>,
    secondaries: Vec<Secondary>,
    notifies: Vec<Notify>,
    clients: Vec<Client>,
    out: Vec<Out>,
    rng: u32,
    pub stats: Stats,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub queries: u64,
    pub recursive: u64,
    pub refused: u64,
    pub servfail: u64,
    pub transfers_out: u64,
}

/// File reader passed to [`Server::load`]: absolute path -> contents.
pub type ReadFn<'a> = &'a dyn Fn(&str) -> Option<String>;

impl Server {
    /// Build from a configuration, reading zone files and hints through `read`.
    /// Problems are logged and skipped (a broken zone is not served), as BIND does.
    pub fn load(conf: NamedConf, read: ReadFn, own: Vec<Ip>, seed: u32, now: i64) -> Server {
        let mut s = Server {
            conf: conf.clone(),
            auth: Authority::default(),
            resolver: None,
            own,
            secondaries: Vec::new(),
            notifies: Vec::new(),
            clients: Vec::new(),
            out: Vec::new(),
            rng: seed | 1,
            stats: Stats::default(),
        };
        s.apply(conf, read, now, None);
        s
    }

    /// `named reload`: re-read the configuration and the zones, keep the cache.
    pub fn reload(&mut self, conf: NamedConf, read: ReadFn, now: i64) {
        let cache = self.resolver.take().map(|r| r.cache);
        let old: Vec<Zone> = std::mem::take(&mut self.auth.zones);
        self.secondaries.clear();
        self.conf = conf.clone();
        self.apply(conf, read, now, Some(old));
        if let (Some(r), Some(c)) = (self.resolver.as_mut(), cache) {
            r.cache = c;
        }
    }

    fn rand16(&mut self) -> u16 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as u16
    }

    fn log(&mut self, s: String) {
        self.out.push(Out::Log(s));
    }

    /// Load the zones of `conf`. On a reload, `old` holds the zones served so far: a zone
    /// file that does not parse keeps its old copy (as BIND does), and only zones whose
    /// serial changed are NOTIFYed.
    fn apply(&mut self, conf: NamedConf, read: ReadFn, now: i64, old: Option<Vec<Zone>>) {
        let old_zone = |name: &str| old.as_ref().and_then(|o| o.iter().find(|z| z.origin == name).cloned());
        for zc in &conf.zones {
            match &zc.kind {
                ZoneKind::Primary { file, notify } => {
                    let Some(text) = read(file) else {
                        self.log(format!("zone {}: cannot read {}", fqdn(&zc.name), file));
                        continue;
                    };
                    match Zone::parse(&text, &zc.name) {
                        Ok(z) => {
                            let serial = z.serial();
                            let changed = old.is_none() || old_zone(&zc.name).map(|z| z.serial()) != Some(serial);
                            self.log(format!("zone {}: loaded serial {} ({} records)", fqdn(&zc.name), serial, z.records.len()));
                            let mut targets: Vec<Ip> = notify.clone();
                            for ns in z.get(&z.origin, NS) {
                                if let Some(t) = ns.data.target() {
                                    for a in z.get(t, A).iter().filter_map(|r| r.a()) {
                                        targets.push(a);
                                    }
                                }
                            }
                            targets.retain(|t| !self.own.contains(t));
                            targets.sort();
                            targets.dedup();
                            self.auth.put(z);
                            if changed {
                                for to in targets {
                                    let id = self.rand16();
                                    self.notifies.retain(|n| !(n.zone == zc.name && n.to == to));
                                    self.notifies.push(Notify { zone: zc.name.clone(), to, id, tries: 0, next: now });
                                }
                            }
                        }
                        Err(e) => match old_zone(&zc.name) {
                            Some(z) => {
                                self.log(format!("zone {}: {} {} (still serving serial {})", fqdn(&zc.name), file, e, z.serial()));
                                self.auth.put(z);
                            }
                            None => self.log(format!("zone {}: {} {} (not loaded)", fqdn(&zc.name), file, e)),
                        },
                    }
                }
                ZoneKind::Secondary { primary, file } => {
                    // The copy in memory (a reload) is at least as new as the one on disk.
                    let copy = old_zone(&zc.name).or_else(|| read(file).and_then(|t| Zone::parse(&t, &zc.name).ok()));
                    if let Some(z) = copy {
                        self.log(format!("zone {}: serial {} (secondary of {})", fqdn(&zc.name), z.serial(), ip_str(*primary)));
                        self.auth.put(z);
                    }
                    self.secondaries.push(Secondary {
                        zone: zc.name.clone(),
                        primary: *primary,
                        file: file.clone(),
                        next_check: now,
                        soa_query: None,
                        transferring: false,
                    });
                }
            }
        }
        self.resolver = None;
        if conf.recursion {
            let hints = conf.root_hints.as_deref().and_then(read).map(|t| resolver::parse_hints(&t));
            match hints {
                Some(Ok(h)) => {
                    let cfg = resolver::Config { hints: h, fallback: conf.fallback.clone(), ..Default::default() };
                    self.resolver = Some(Resolver::new(cfg, self.rng ^ 0x5eed));
                }
                Some(Err(e)) => self.log(format!("root hints: {e}; recursion is off")),
                None => self.log(format!("cannot read root hints {}; recursion is off", conf.root_hints.clone().unwrap_or_default())),
            }
        }
    }

    /// Put an output back (the program took it out early).
    pub fn push_out(&mut self, o: Out) {
        self.out.push(o);
    }

    pub fn take_out(&mut self) -> Vec<Out> {
        std::mem::take(&mut self.out)
    }

    /// The next time [`tick`](Self::tick) has something to do.
    pub fn next_deadline(&self) -> Option<i64> {
        let mut d: Vec<i64> = Vec::new();
        d.extend(self.resolver.as_ref().and_then(|r| r.next_deadline()));
        d.extend(self.notifies.iter().map(|n| n.next));
        d.extend(self.secondaries.iter().filter(|s| !s.transferring).map(|s| s.soa_query.map_or(s.next_check, |(_, t)| t)));
        d.into_iter().min()
    }

    fn refresh_ms(&self, zone: &str, retry: bool) -> i64 {
        let secs = match self.auth.zones.iter().find(|z| z.origin == zone) {
            Some(z) if retry => z.soa().retry,
            Some(z) => z.soa().refresh,
            // Nothing to serve yet (first start, the primary still booting): ask again soon.
            None => return UNLOADED_RETRY_MS,
        };
        secs.clamp(MIN_REFRESH, MAX_REFRESH) as i64 * 1000
    }

    pub fn tick(&mut self, now: i64) {
        if let Some(r) = self.resolver.as_mut() {
            r.tick(now);
        }
        self.pump_resolver();
        // NOTIFY retries
        let mut sends = Vec::new();
        for n in self.notifies.iter_mut() {
            if now >= n.next {
                n.tries += 1;
                n.next = now + NOTIFY_RETRY_MS;
                sends.push((n.zone.clone(), n.to, n.id, n.tries));
            }
        }
        for (zone, to, id, tries) in sends {
            if tries > NOTIFY_TRIES {
                self.log(format!("zone {}: no answer to NOTIFY from {}", fqdn(&zone), ip_str(to)));
                continue;
            }
            if let Some(z) = self.auth.zones.iter().find(|z| z.origin == zone) {
                let data = xfr::notify(z, id).encode();
                if tries == 1 {
                    let serial = z.serial();
                    self.log(format!("zone {}: sending NOTIFY (serial {}) to {}", fqdn(&zone), serial, ip_str(to)));
                }
                self.out.push(Out::Udp { to, port: 53, data });
            }
        }
        self.notifies.retain(|n| n.tries <= NOTIFY_TRIES);
        // Secondaries: SOA checks.
        for i in 0..self.secondaries.len() {
            let s = &self.secondaries[i];
            if s.transferring {
                continue;
            }
            match s.soa_query {
                Some((_, deadline)) if now >= deadline => {
                    let zone = s.zone.clone();
                    let primary = s.primary;
                    let retry = self.refresh_ms(&zone, true);
                    let s = &mut self.secondaries[i];
                    s.soa_query = None;
                    s.next_check = now + retry;
                    self.log(format!("zone {}: primary {} did not answer the SOA check", fqdn(&zone), ip_str(primary)));
                }
                None if now >= s.next_check => {
                    let (zone, primary) = (s.zone.clone(), s.primary);
                    let id = self.rand16();
                    self.secondaries[i].soa_query = Some((id, now + 3000));
                    let q = Message::query(id, &zone, SOA, false);
                    self.out.push(Out::Udp { to: primary, port: 53, data: q.encode() });
                }
                _ => {}
            }
        }
    }

    fn pump_resolver(&mut self) {
        let Some(r) = self.resolver.as_mut() else { return };
        let sends = r.take_sends();
        let done = r.take_done();
        for (to, data) in sends {
            self.out.push(Out::Resolver { to, data });
        }
        for (q, res) in done {
            let Some(pos) = self.clients.iter().position(|c| c.q == q) else { continue };
            let c = self.clients.remove(pos);
            let mut m = Message::response_to(&c.query, res.rcode);
            m.header.ra = true;
            m.answers = res.answers;
            m.authority = res.authority;
            if res.rcode == SERVFAIL {
                self.stats.servfail += 1;
                if !res.reason.is_empty() {
                    let qn = c.query.question().map(|q| fqdn(&q.name)).unwrap_or_default();
                    self.out.push(Out::Log(format!("SERVFAIL {} for {}: {}", qn, ip_str(c.from), res.reason)));
                }
            }
            self.out.push(Out::Udp { to: c.from, port: c.port, data: m.encode_limited(UDP_MAX) });
        }
    }

    /// A datagram on the port-53 socket.
    pub fn on_udp(&mut self, from: Ip, port: u16, data: &[u8], now: i64) {
        let Ok(m) = Message::decode(data) else {
            // FORMERR needs at least an id
            if data.len() >= 12 && data[2] & 0x80 == 0 {
                let mut r = vec![0u8; 12];
                r[..2].copy_from_slice(&data[..2]);
                r[2] = 0x80;
                r[3] = FORMERR;
                self.out.push(Out::Udp { to: from, port, data: r });
            }
            return;
        };
        if m.header.qr {
            return self.on_reply(from, m, now);
        }
        match m.header.opcode {
            OP_NOTIFY => self.on_notify(from, port, m, now),
            OP_QUERY => {
                if let Some(r) = self.answer(from, port, &m, now, true) {
                    self.out.push(Out::Udp { to: from, port, data: r.encode_limited(UDP_MAX) });
                }
            }
            _ => {
                let r = Message::response_to(&m, NOTIMP);
                self.out.push(Out::Udp { to: from, port, data: r.encode() });
            }
        }
    }

    /// A datagram on the resolver's socket.
    pub fn on_resolver_udp(&mut self, from: Ip, data: &[u8], now: i64) {
        if let Some(r) = self.resolver.as_mut() {
            r.on_packet(from, data, now);
        }
        self.pump_resolver();
    }

    /// Answer a query (`udp`: recursion allowed). `None` when it went to the resolver.
    fn answer(&mut self, from: Ip, port: u16, m: &Message, now: i64, udp: bool) -> Option<Message> {
        self.stats.queries += 1;
        let Some(q) = m.question().cloned() else {
            return Some(Message::response_to(m, FORMERR));
        };
        if m.questions.len() != 1 || q.qclass != IN && q.qclass != 255 {
            return Some(Message::response_to(m, FORMERR));
        }
        if q.qtype == AXFR {
            return Some(Message::response_to(m, if udp { FORMERR } else { REFUSED }));
        }
        let may_recurse = udp && m.header.rd && self.resolver.is_some() && self.conf.may_recurse(from);
        if let Some(mut r) = self.auth.respond(m) {
            let referral = !r.header.aa && r.answers.is_empty() && r.authority.iter().any(|a| a.rtype == NS);
            if !(referral && may_recurse) {
                r.header.ra = self.resolver.is_some() && self.conf.may_recurse(from);
                return Some(r);
            }
        }
        if may_recurse {
            if self.clients.len() >= MAX_RECURSIVE_CLIENTS {
                return Some(Message::response_to(m, SERVFAIL));
            }
            self.stats.recursive += 1;
            let r = self.resolver.as_mut().unwrap();
            let qid = r.resolve(&q.name, q.qtype, now);
            self.clients.push(Client { q: qid, from, port, query: m.clone() });
            self.pump_resolver();
            return None;
        }
        self.stats.refused += 1;
        let mut r = Message::response_to(m, REFUSED);
        r.header.ra = false;
        Some(r)
    }

    fn on_notify(&mut self, from: Ip, port: u16, m: Message, now: i64) {
        let zone = m.question().map(|q| q.name.clone()).unwrap_or_default();
        let Some(s) = self.secondaries.iter_mut().find(|s| s.zone == zone && s.primary == from) else {
            let r = Message::response_to(&m, NOTAUTH);
            self.out.push(Out::Udp { to: from, port, data: r.encode() });
            return;
        };
        s.next_check = now;
        s.soa_query = None;
        let serial = answer_serial(&m);
        self.out.push(Out::Udp { to: from, port, data: xfr::notify_ack(&m).encode() });
        self.log(format!(
            "zone {}: NOTIFY from {}{}",
            fqdn(&zone),
            ip_str(from),
            serial.map(|s| format!(" (serial {s})")).unwrap_or_default()
        ));
        self.tick(now);
    }

    fn on_reply(&mut self, from: Ip, m: Message, now: i64) {
        // NOTIFY acknowledgements
        if m.header.opcode == OP_NOTIFY {
            self.notifies.retain(|n| !(n.to == from && n.id == m.header.id));
            return;
        }
        // SOA checks of secondaries
        let Some(i) = self.secondaries.iter().position(|s| s.primary == from && s.soa_query.is_some_and(|(id, _)| id == m.header.id))
        else {
            return;
        };
        let zone = self.secondaries[i].zone.clone();
        self.secondaries[i].soa_query = None;
        let theirs = answer_serial(&m);
        let ours = self.auth.zones.iter().find(|z| z.origin == zone).map(|z| z.serial());
        match (theirs, ours) {
            (Some(t), Some(o)) if !serial_newer(t, o) => {
                let refresh = self.refresh_ms(&zone, false);
                self.secondaries[i].next_check = now + refresh;
            }
            (Some(_), _) => {
                self.secondaries[i].transferring = true;
                self.out.push(Out::Transfer { zone, primary: from });
            }
            (None, _) => {
                let retry = self.refresh_ms(&zone, true);
                self.secondaries[i].next_check = now + retry;
                self.log(format!("zone {}: {} is not authoritative for it ({})", fqdn(&zone), ip_str(from), rcode_name(m.header.rcode)));
            }
        }
    }

    /// The result of a transfer asked for with [`Out::Transfer`].
    pub fn on_transfer(&mut self, zone: &str, result: Result<Zone, String>, now: i64) {
        let Some(i) = self.secondaries.iter().position(|s| s.zone == zone) else { return };
        self.secondaries[i].transferring = false;
        match result {
            Ok(z) => {
                let file = self.secondaries[i].file.clone();
                let primary = self.secondaries[i].primary;
                self.log(format!("zone {}: transferred serial {} from {} ({} records)", fqdn(zone), z.serial(), ip_str(primary), z.records.len()));
                self.out.push(Out::Save { file, text: z.to_text() });
                self.auth.put(z);
                let refresh = self.refresh_ms(zone, false);
                self.secondaries[i].next_check = now + refresh;
            }
            Err(e) => {
                let retry = self.refresh_ms(zone, true);
                self.secondaries[i].next_check = now + retry;
                self.log(format!("zone {}: transfer failed: {}", fqdn(zone), e));
            }
        }
    }

    /// One message on a TCP connection to port 53: AXFR or an authoritative query.
    pub fn on_tcp(&mut self, from: Ip, data: &[u8], now: i64) -> Vec<Message> {
        let Ok(m) = Message::decode(data) else { return Vec::new() };
        if m.header.qr {
            return Vec::new();
        }
        let Some(q) = m.question().cloned() else { return vec![Message::response_to(&m, FORMERR)] };
        if q.qtype == AXFR {
            let zone = self.auth.zones.iter().find(|z| z.origin == q.name).map(|z| (xfr::axfr_messages(&m, z), z.serial()));
            return match zone {
                Some((msgs, serial)) if self.conf.may_transfer(from) => {
                    self.stats.transfers_out += 1;
                    self.log(format!("zone {}: AXFR to {} (serial {})", fqdn(&q.name), ip_str(from), serial));
                    msgs
                }
                _ => {
                    self.log(format!("zone {}: AXFR from {} refused", fqdn(&q.name), ip_str(from)));
                    vec![Message::response_to(&m, if zone.is_some() { REFUSED } else { NOTAUTH })]
                }
            };
        }
        self.answer(from, 0, &m, now, false).into_iter().collect()
    }

    /// `named retransfer ZONE`: check the primary now.
    pub fn retransfer(&mut self, zone: &str, now: i64) -> bool {
        match self.secondaries.iter_mut().find(|s| s.zone == zone) {
            Some(s) => {
                s.next_check = now;
                s.soa_query = None;
                true
            }
            None => false,
        }
    }

    /// `named status`.
    pub fn status(&self) -> Vec<String> {
        let mut v = Vec::new();
        for z in &self.auth.zones {
            let kind = match self.secondaries.iter().find(|s| s.zone == z.origin) {
                Some(s) => format!("secondary of {}", ip_str(s.primary)),
                None => "primary".into(),
            };
            v.push(format!("zone {} serial {} {} ({} records)", fqdn(&z.origin), z.serial(), kind, z.records.len()));
        }
        for s in &self.secondaries {
            if !self.auth.zones.iter().any(|z| z.origin == s.zone) {
                v.push(format!("zone {} secondary of {} (not loaded yet)", fqdn(&s.zone), ip_str(s.primary)));
            }
        }
        v.push(match &self.resolver {
            Some(r) => format!(
                "recursion yes ({} cached, {} in progress){}",
                r.cache.len(),
                r.pending(),
                if self.conf.fallback.is_empty() {
                    String::new()
                } else {
                    format!(", fallback {}", self.conf.fallback.iter().map(|i| ip_str(*i)).collect::<Vec<_>>().join(" "))
                }
            ),
            None => "recursion no".into(),
        });
        v.push(format!(
            "queries {} recursive {} refused {} servfail {} transfers {}",
            self.stats.queries, self.stats.recursive, self.stats.refused, self.stats.servfail, self.stats.transfers_out
        ));
        v
    }

    /// `named cache`.
    pub fn cache_dump(&self, now: i64) -> Vec<String> {
        match &self.resolver {
            Some(r) => r.cache.dump(now),
            None => vec!["recursion is off: no cache".into()],
        }
    }

    /// `named flush [NAME]`.
    pub fn flush(&mut self, name: Option<&str>) {
        if let Some(r) = self.resolver.as_mut() {
            match name {
                Some(n) => r.cache.flush_name(n),
                None => r.cache.flush(),
            }
        }
    }
}
