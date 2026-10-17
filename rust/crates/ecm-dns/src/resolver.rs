//! The iterative resolver (what `named` does with `recursion yes`, and what `dig +trace`
//! does by itself): start at the root hints, follow referrals down the tree (at most
//! [`MAX_REFERRALS`]), chase CNAMEs (at most [`MAX_CNAMES`]), look up the addresses of
//! nameservers that came without glue, move on to the next server after a timeout or a
//! bad answer, and cache what it learns, positive and negative, by TTL. A delegation
//! where no server gives a usable answer ends in SERVFAIL (lame delegation).
//!
//! When the root itself says a top-level domain does not exist and `fallback` servers
//! are configured, the query is forwarded there with recursion desired (docs/DNS.md,
//! "Real internet names"): in-game TLDs resolve in game, `example.com` resolves for real.
//!
//! Sans-IO: the caller sends what [`Resolver::take_sends`] returns from one UDP socket,
//! feeds every datagram that arrives to [`Resolver::on_packet`], calls
//! [`Resolver::tick`] by [`Resolver::next_deadline`], and collects
//! [`Resolver::take_done`].

use crate::cache::{Cache, Hit};
use crate::name::{fqdn, ip_str, is_sub, parent, tld, Ip};
use crate::wire::*;

pub const MAX_REFERRALS: usize = 16;
pub const MAX_CNAMES: usize = 8;
/// Nested lookups of nameserver addresses.
pub const MAX_DEPTH: u8 = 4;
/// Datagrams one client query may cause (everything included).
pub const MAX_SENDS: usize = 48;
pub const DEFAULT_TIMEOUT_MS: i64 = 1000;

#[derive(Clone, Debug)]
pub struct Config {
    /// Root servers: name and address.
    pub hints: Vec<(String, Ip)>,
    /// Forwarders for names the in-game root says do not exist.
    pub fallback: Vec<Ip>,
    pub timeout_ms: i64,
}

impl Default for Config {
    fn default() -> Self {
        Config { hints: Vec::new(), fallback: Vec::new(), timeout_ms: DEFAULT_TIMEOUT_MS }
    }
}

/// Parse a root hints file (`. NS a.root-servers.ring.` and `a.root-servers.ring. A ...`).
pub fn parse_hints(text: &str) -> Result<Vec<(String, Ip)>, String> {
    let recs = crate::zone::parse_records(text, "", 3_600_000).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for ns in recs.iter().filter(|r| r.rtype == NS && r.name.is_empty()) {
        let t = ns.data.target().unwrap_or("");
        for a in recs.iter().filter(|r| r.rtype == A && r.name == t) {
            if let Some(ip) = a.a() {
                out.push((t.to_string(), ip));
            }
        }
    }
    if out.is_empty() {
        return Err("no root servers (need '. NS name' and 'name A address' lines)".into());
    }
    Ok(out)
}

/// One response (or timeout) on the way: what `dig +trace` prints.
#[derive(Clone, Debug)]
pub struct Step {
    /// The zone the server was asked as an authority for (`""` = root).
    pub zone: String,
    pub server_name: String,
    pub server: Ip,
    pub qname: String,
    pub qtype: u16,
    /// `None`: no answer before the timeout.
    pub response: Option<Message>,
    pub bytes: usize,
    pub ms: i64,
    /// This was the fallback forwarder.
    pub fallback: bool,
}

#[derive(Clone, Debug)]
pub struct Resolution {
    pub qname: String,
    pub qtype: u16,
    pub rcode: u8,
    /// CNAMEs followed, then the records asked for.
    pub answers: Vec<Record>,
    /// The SOA of a negative answer.
    pub authority: Vec<Record>,
    pub steps: Vec<Step>,
    /// Answered (in part) from the cache.
    pub cached: bool,
    /// Answered by the fallback forwarder.
    pub fallback: bool,
    /// Why it failed, for logs (`lame delegation for v4.ring`).
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QueryId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
enum SrvState {
    Untried,
    LookingUp(u32),
    /// Timed out once: asked again after every other server.
    TimedOut,
    Failed,
}

#[derive(Clone, Debug)]
struct Srv {
    name: String,
    addrs: Vec<Ip>,
    /// Index of the next address to try.
    next: usize,
    state: SrvState,
    timeouts: u8,
}

#[derive(Clone, Debug)]
struct Inflight {
    txid: u16,
    to: Ip,
    srv: usize,
    sent: i64,
    deadline: i64,
}

#[derive(Clone, Debug)]
struct Task {
    id: u32,
    parent: Option<u32>,
    depth: u8,
    qname: String,
    qtype: u16,
    orig: String,
    chain: Vec<Record>,
    referrals: usize,
    zone: Option<String>,
    servers: Vec<Srv>,
    inflight: Option<Inflight>,
    waiting: bool,
    steps: Vec<Step>,
    fallback: bool,
    cached: bool,
    no_cache: bool,
}

pub struct Resolver {
    pub cfg: Config,
    pub cache: Cache,
    tasks: Vec<Task>,
    next_id: u32,
    rng: u32,
    sends: Vec<(Ip, Vec<u8>)>,
    done: Vec<(QueryId, Resolution)>,
    /// Datagrams sent per top-level query id.
    budget: std::collections::HashMap<u32, usize>,
    last_now: i64,
}

fn has_rrset(records: &[Record], name: &str, rtype: u16) -> Vec<Record> {
    records.iter().filter(|r| r.name == name && (r.rtype == rtype || rtype == ANY)).cloned().collect()
}

impl Resolver {
    pub fn new(cfg: Config, seed: u32) -> Resolver {
        Resolver {
            cfg,
            cache: Cache::new(),
            tasks: Vec::new(),
            next_id: 1,
            rng: seed | 1,
            sends: Vec::new(),
            done: Vec::new(),
            budget: Default::default(),
            last_now: 0,
        }
    }

    fn rand16(&mut self) -> u16 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as u16
    }

    /// Start resolving. The result may already be in [`take_done`](Self::take_done) when
    /// the cache answers it.
    pub fn resolve(&mut self, qname: &str, qtype: u16, now: i64) -> QueryId {
        self.start(qname, qtype, None, 0, false, now)
    }

    /// Resolve without reading the cache (what `dig +trace` wants: every step on the wire).
    pub fn resolve_fresh(&mut self, qname: &str, qtype: u16, now: i64) -> QueryId {
        self.start(qname, qtype, None, 0, true, now)
    }

    fn start(&mut self, qname: &str, qtype: u16, parent: Option<u32>, depth: u8, no_cache: bool, now: i64) -> QueryId {
        self.last_now = now;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.tasks.push(Task {
            id,
            parent,
            depth,
            qname: qname.to_string(),
            qtype,
            orig: qname.to_string(),
            chain: Vec::new(),
            referrals: 0,
            zone: None,
            servers: Vec::new(),
            inflight: None,
            waiting: false,
            steps: Vec::new(),
            fallback: false,
            cached: false,
            no_cache,
        });
        self.advance(id, now);
        QueryId(id)
    }

    pub fn take_sends(&mut self) -> Vec<(Ip, Vec<u8>)> {
        std::mem::take(&mut self.sends)
    }

    pub fn take_done(&mut self) -> Vec<(QueryId, Resolution)> {
        std::mem::take(&mut self.done)
    }

    pub fn pending(&self) -> usize {
        self.tasks.iter().filter(|t| t.parent.is_none()).count()
    }

    pub fn cancel(&mut self, q: QueryId) {
        let mut kill = vec![q.0];
        while let Some(id) = kill.pop() {
            self.tasks.retain(|t| t.id != id);
            kill.extend(self.tasks.iter().filter(|t| t.parent == Some(id)).map(|t| t.id));
        }
    }

    pub fn next_deadline(&self) -> Option<i64> {
        self.tasks.iter().filter_map(|t| t.inflight.as_ref().map(|i| i.deadline)).min()
    }

    fn idx(&self, id: u32) -> Option<usize> {
        self.tasks.iter().position(|t| t.id == id)
    }

    fn root_of(&self, mut id: u32) -> u32 {
        while let Some(p) = self.idx(id).and_then(|i| self.tasks[i].parent) {
            id = p;
        }
        id
    }

    /// Timeouts: the server in flight is given up and the next one tried.
    pub fn tick(&mut self, now: i64) {
        self.last_now = now;
        let late: Vec<u32> = self
            .tasks
            .iter()
            .filter(|t| t.inflight.as_ref().is_some_and(|i| now >= i.deadline))
            .map(|t| t.id)
            .collect();
        for id in late {
            let Some(i) = self.idx(id) else { continue };
            let t = &mut self.tasks[i];
            let inf = t.inflight.take().unwrap();
            let zone = t.zone.clone().unwrap_or_default();
            let fallback = t.fallback;
            if let Some(s) = t.servers.get_mut(inf.srv) {
                s.timeouts += 1;
                if s.next >= s.addrs.len() {
                    s.state = if s.timeouts >= 2 { SrvState::Failed } else { SrvState::TimedOut };
                }
            }
            let (qname, qtype) = (t.qname.clone(), t.qtype);
            let name = t.servers.get(inf.srv).map(|s| s.name.clone()).unwrap_or_default();
            t.steps.push(Step {
                zone,
                server_name: name,
                server: inf.to,
                qname,
                qtype,
                response: None,
                bytes: 0,
                ms: now - inf.sent,
                fallback,
            });
            self.advance(id, now);
        }
    }

    /// A datagram from `from` (port 53) on the resolver's socket.
    pub fn on_packet(&mut self, from: Ip, data: &[u8], now: i64) {
        self.last_now = now;
        let Ok(msg) = Message::decode(data) else { return };
        if !msg.header.qr {
            return;
        }
        let Some(i) = self.tasks.iter().position(|t| {
            t.inflight.as_ref().is_some_and(|f| f.txid == msg.header.id && f.to == from)
                && msg.question().is_some_and(|q| q.name == t.qname && q.qtype == t.qtype)
        }) else {
            return;
        };
        let id = self.tasks[i].id;
        let inf = self.tasks[i].inflight.take().unwrap();
        let t = &mut self.tasks[i];
        let zone = t.zone.clone().unwrap_or_default();
        let name = t.servers.get(inf.srv).map(|s| s.name.clone()).unwrap_or_default();
        t.steps.push(Step {
            zone: zone.clone(),
            server_name: name,
            server: from,
            qname: t.qname.clone(),
            qtype: t.qtype,
            response: Some(msg.clone()),
            bytes: data.len(),
            ms: now - inf.sent,
            fallback: t.fallback,
        });
        self.handle(id, inf.srv, &zone, msg, now);
    }

    fn mark_failed(&mut self, id: u32, srv: usize) {
        if let Some(i) = self.idx(id) {
            if let Some(s) = self.tasks[i].servers.get_mut(srv) {
                if s.next >= s.addrs.len() {
                    s.state = SrvState::Failed;
                }
            }
        }
    }

    fn handle(&mut self, id: u32, srv: usize, zone: &str, msg: Message, now: i64) {
        let Some(i) = self.idx(id) else { return };
        let (qname, qtype, fallback) = (self.tasks[i].qname.clone(), self.tasks[i].qtype, self.tasks[i].fallback);
        let rcode = msg.header.rcode;
        if fallback {
            // The forwarder recursed for us: take its answer as it is.
            if !matches!(rcode, NOERROR | NXDOMAIN) {
                self.mark_failed(id, srv);
                return self.advance(id, now);
            }
            let ans: Vec<Record> = msg.answers.clone();
            self.cache.put_all(&ans, now);
            let soa: Vec<Record> = msg.authority.iter().filter(|r| r.rtype == SOA).cloned().collect();
            let t = &mut self.tasks[i];
            t.chain.extend(ans);
            return self.finish(id, rcode, soa, String::new());
        }
        if !matches!(rcode, NOERROR | NXDOMAIN) || msg.header.tc && msg.answers.is_empty() && msg.authority.is_empty() {
            self.mark_failed(id, srv);
            return self.advance(id, now);
        }
        let soa: Option<Record> =
            msg.authority.iter().find(|r| r.rtype == SOA && is_sub(&qname, &r.name) && is_sub(&r.name, zone)).cloned();
        // Data for the name (only what the server is an authority for).
        let mut name = qname.clone();
        let mut moved = false;
        let mut got_final = false;
        let mut chain_add: Vec<Record> = Vec::new();
        for _ in 0..=MAX_CNAMES {
            if !is_sub(&name, zone) {
                break;
            }
            let hits = has_rrset(&msg.answers, &name, qtype);
            if !hits.is_empty() {
                self.cache.put_all(&hits, now);
                if qtype == NS {
                    // Addresses of the nameservers, from inside the zone asked.
                    let glue: Vec<Record> =
                        msg.additional.iter().filter(|r| r.rtype == A && is_sub(&r.name, zone)).cloned().collect();
                    self.cache.put_all(&glue, now);
                }
                chain_add.extend(hits);
                got_final = true;
                break;
            }
            let c = has_rrset(&msg.answers, &name, CNAME);
            match c.first() {
                Some(c) if qtype != CNAME => {
                    self.cache.put(vec![c.clone()], now);
                    chain_add.push(c.clone());
                    name = c.data.target().unwrap_or("").to_string();
                    moved = true;
                }
                _ => break,
            }
        }
        if got_final || moved {
            let t = &mut self.tasks[i];
            t.chain.extend(chain_add);
            if t.chain.iter().filter(|r| r.rtype == CNAME).count() > MAX_CNAMES {
                return self.finish(id, SERVFAIL, vec![], "too many CNAMEs".into());
            }
            if got_final {
                return self.finish(id, NOERROR, vec![], String::new());
            }
            // Continue with the CNAME's target from the top of what we know.
            if rcode == NXDOMAIN && is_sub(&name, zone) && msg.header.aa {
                self.cache.put_nxdomain(&name, soa.clone(), now);
                return self.finish(id, NXDOMAIN, soa.into_iter().collect(), String::new());
            }
            t.qname = name;
            t.zone = None;
            return self.advance(id, now);
        }
        if rcode == NXDOMAIN {
            // Fall back to the real internet when the root denies a top-level domain.
            if zone.is_empty() && !self.cfg.fallback.is_empty() && !never_forward(&qname) {
                let fb = self.cfg.fallback.clone();
                let t = &mut self.tasks[i];
                t.fallback = true;
                t.zone = Some(String::new());
                t.servers = vec![Srv { name: "fallback".into(), addrs: fb, next: 0, state: SrvState::Untried, timeouts: 0 }];
                return self.advance(id, now);
            }
            self.cache.put_nxdomain(&qname, soa.clone(), now);
            return self.finish(id, NXDOMAIN, soa.into_iter().collect(), String::new());
        }
        // A referral to a zone closer to the name?
        let cut = msg
            .authority
            .iter()
            .filter(|r| r.rtype == NS && is_sub(&qname, &r.name) && is_sub(&r.name, zone) && r.name != zone)
            .map(|r| r.name.clone())
            .max_by_key(|n| n.len());
        if let Some(cut) = cut {
            if !msg.header.aa || msg.answers.is_empty() {
                let ns: Vec<Record> = msg.authority.iter().filter(|r| r.rtype == NS && r.name == cut).cloned().collect();
                self.cache.put(ns, now);
                // Glue only from inside the zone the server answers for.
                let glue: Vec<Record> =
                    msg.additional.iter().filter(|r| r.rtype == A && is_sub(&r.name, zone)).cloned().collect();
                self.cache.put_all(&glue, now);
                let t = &mut self.tasks[i];
                t.referrals += 1;
                if t.referrals > MAX_REFERRALS {
                    return self.finish(id, SERVFAIL, vec![], "too many referrals".into());
                }
                t.zone = None;
                return self.advance(id, now);
            }
        }
        if msg.header.aa || soa.is_some() {
            // NODATA
            self.cache.put_nodata(&qname, qtype, soa.clone(), now);
            return self.finish(id, NOERROR, soa.into_iter().collect(), String::new());
        }
        // Lame: an upward or sideways referral, or a non-authoritative empty answer.
        self.mark_failed(id, srv);
        self.advance(id, now)
    }

    /// Choose the zone to ask, from the cache or the hints.
    fn find_zone(&self, qname: &str, now: i64) -> (String, Vec<Srv>) {
        let mut n = Some(qname);
        while let Some(z) = n {
            if let Some(ns) = self.cache.records(z, NS, now) {
                let servers = ns
                    .iter()
                    .filter_map(|r| r.data.target())
                    .map(|t| Srv {
                        name: t.to_string(),
                        addrs: self.cache.records(t, A, now).unwrap_or_default().iter().filter_map(|r| r.a()).collect(),
                        next: 0,
                        state: SrvState::Untried,
                        timeouts: 0,
                    })
                    .collect();
                let mut servers: Vec<Srv> = servers;
                if z.is_empty() {
                    // The root's addresses always come from the hints when not cached.
                    for s in servers.iter_mut().filter(|s| s.addrs.is_empty()) {
                        s.addrs = self.cfg.hints.iter().filter(|(n, _)| *n == s.name).map(|(_, ip)| *ip).collect();
                    }
                    if servers.iter().all(|s| s.addrs.is_empty()) {
                        break;
                    }
                }
                return (z.to_string(), servers);
            }
            n = parent(z);
        }
        let mut servers: Vec<Srv> = Vec::new();
        for (name, ip) in &self.cfg.hints {
            match servers.iter_mut().find(|s| &s.name == name) {
                Some(s) => s.addrs.push(*ip),
                None => servers.push(Srv { name: name.clone(), addrs: vec![*ip], next: 0, state: SrvState::Untried, timeouts: 0 }),
            }
        }
        (String::new(), servers)
    }

    fn advance(&mut self, id: u32, now: i64) {
        let Some(i) = self.idx(id) else { return };
        if self.tasks[i].inflight.is_some() || self.tasks[i].waiting {
            return;
        }
        // The cache first (every turn: CNAMEs learned on the way count too).
        for _ in 0..=MAX_CNAMES {
            let t = &self.tasks[i];
            if t.fallback || t.no_cache && t.parent.is_none() {
                break;
            }
            let (qn, qt) = (t.qname.clone(), t.qtype);
            if qt != CNAME && qt != ANY {
                if let Some(c) = self.cache.records(&qn, CNAME, now) {
                    let t = &mut self.tasks[i];
                    t.cached = true;
                    t.qname = c[0].data.target().unwrap_or("").to_string();
                    t.chain.extend(c);
                    t.zone = None;
                    if t.chain.len() > MAX_CNAMES {
                        return self.finish(id, SERVFAIL, vec![], "too many CNAMEs".into());
                    }
                    continue;
                }
            }
            match self.cache.get(&qn, qt, now) {
                Some(Hit::Records(r)) => {
                    let t = &mut self.tasks[i];
                    t.cached = true;
                    t.chain.extend(r);
                    return self.finish(id, NOERROR, vec![], String::new());
                }
                Some(Hit::NoData(soa)) => {
                    self.tasks[i].cached = true;
                    return self.finish(id, NOERROR, soa.into_iter().collect(), String::new());
                }
                Some(Hit::NxDomain(soa)) => {
                    self.tasks[i].cached = true;
                    return self.finish(id, NXDOMAIN, soa.into_iter().collect(), String::new());
                }
                None => break,
            }
        }
        if self.tasks[i].zone.is_none() {
            let (zone, servers) = self.find_zone(&self.tasks[i].qname, now);
            if servers.is_empty() {
                return self.finish(id, SERVFAIL, vec![], "no root hints".into());
            }
            let t = &mut self.tasks[i];
            t.zone = Some(zone);
            t.servers = servers;
        }
        let root = self.root_of(id);
        let used = *self.budget.get(&root).unwrap_or(&0);
        if used >= MAX_SENDS {
            return self.finish(id, SERVFAIL, vec![], "query budget exhausted".into());
        }
        // A server with an address left to try.
        let mut pick = self.tasks[i]
            .servers
            .iter()
            .position(|s| s.state == SrvState::Untried && s.next < s.addrs.len());
        if pick.is_none() && !self.tasks[i].servers.iter().any(|s| matches!(s.state, SrvState::Untried | SrvState::LookingUp(_))) {
            // Everyone was asked: one more round for the servers that only timed out.
            pick = self.tasks[i].servers.iter().position(|s| s.state == SrvState::TimedOut);
            if let Some(si) = pick {
                let s = &mut self.tasks[i].servers[si];
                s.next = 0;
                s.state = SrvState::Untried;
            }
        }
        if let Some(si) = pick {
            let txid = self.rand16();
            let t = &mut self.tasks[i];
            let s = &mut t.servers[si];
            let to = s.addrs[s.next];
            s.next += 1;
            let q = Message::query(txid, &t.qname, t.qtype, t.fallback);
            t.inflight = Some(Inflight { txid, to, srv: si, sent: now, deadline: now + self.cfg.timeout_ms });
            self.sends.push((to, q.encode()));
            *self.budget.entry(root).or_insert(0) += 1;
            return;
        }
        // Otherwise look up the address of a nameserver that came without glue.
        let depth = self.tasks[i].depth;
        let zone = self.tasks[i].zone.clone().unwrap_or_default();
        let lookup = self.tasks[i].servers.iter().position(|s| {
            s.state == SrvState::Untried && s.addrs.is_empty() && !(is_sub(&s.name, &zone) && !zone.is_empty())
        });
        if let (Some(si), true) = (lookup, depth < MAX_DEPTH) {
            let name = self.tasks[i].servers[si].name.clone();
            // Not while an ancestor is already looking for the same name (loops).
            let mut a = self.tasks[i].parent;
            let mut looping = false;
            while let Some(p) = a {
                let Some(pi) = self.idx(p) else { break };
                if self.tasks[pi].qname == name {
                    looping = true;
                }
                a = self.tasks[pi].parent;
            }
            if !looping {
                self.tasks[i].waiting = true;
                let sub = self.next_id;
                self.tasks[i].servers[si].state = SrvState::LookingUp(sub);
                self.start(&name, A, Some(id), depth + 1, false, now);
                return;
            }
            self.tasks[i].servers[si].state = SrvState::Failed;
            return self.advance(id, now);
        }
        if let Some(si) = lookup {
            self.tasks[i].servers[si].state = SrvState::Failed;
        }
        let t = &self.tasks[i];
        let reason = if t.fallback {
            "no answer from the fallback servers".to_string()
        } else {
            format!("no nameserver of {} answered (lame delegation)", fqdn(&zone))
        };
        self.finish(id, SERVFAIL, vec![], reason)
    }

    fn finish(&mut self, id: u32, rcode: u8, authority: Vec<Record>, reason: String) {
        let Some(i) = self.idx(id) else { return };
        let t = self.tasks.remove(i);
        // Sub-lookups of this task are moot now.
        let orphans: Vec<u32> = self.tasks.iter().filter(|s| s.parent == Some(id)).map(|s| s.id).collect();
        for o in orphans {
            self.cancel(QueryId(o));
        }
        let res = Resolution {
            qname: t.orig.clone(),
            qtype: t.qtype,
            rcode,
            answers: t.chain,
            authority,
            steps: t.steps,
            cached: t.cached,
            fallback: t.fallback,
            reason,
        };
        match t.parent {
            None => {
                self.budget.remove(&id);
                self.done.push((QueryId(id), res));
            }
            Some(p) => {
                let Some(pi) = self.idx(p) else { return };
                let pt = &mut self.tasks[pi];
                pt.waiting = false;
                let addrs: Vec<Ip> = res.answers.iter().filter_map(|r| r.a()).collect();
                if let Some(s) = pt.servers.iter_mut().find(|s| s.state == SrvState::LookingUp(id)) {
                    if addrs.is_empty() {
                        s.state = SrvState::Failed;
                    } else {
                        s.addrs = addrs;
                        s.next = 0;
                        s.state = SrvState::Untried;
                    }
                }
                // The parent goes on at the time of the event that finished this one.
                let now = self.last_now;
                self.advance(p, now);
            }
        }
    }
}

impl Resolver {
    /// Pretty name of a server for logs: `a.root-servers.ring (100.65.0.54)`.
    pub fn describe(step: &Step) -> String {
        format!("{} ({})", fqdn(&step.server_name), ip_str(step.server))
    }
}

/// Is `name`'s top-level domain one the fallback should never be asked about
/// (RFC 6761 special-use names and the in-game infrastructure TLD)?
pub fn never_forward(name: &str) -> bool {
    matches!(tld(name), "test" | "example" | "invalid" | "localhost" | "local" | "internal" | "arpa" | "ring" | "ecm")
}
