//! The carrier core (`cored`), sans-IO: site registration, SIM challenges,
//! sessions, phone addresses, tunnels, handover path switches, usage and
//! quotas, revocation, paging, and the subscriber database file.
//!
//! The driver owns the sockets: a TCP listener for sites (each accepted
//! connection gets a [`ConnId`]), a UDP socket for tunnels, and a `tun`
//! interface towards the core's LAN (address = the pool's gateway, with IP
//! forwarding on so the LAN reaches the phones through it). It feeds bytes and
//! packets in, calls [`Core::tick`] every few tens of ms, and executes
//! [`CoreOut`]s.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::air::{Cause, Grant};
use crate::backhaul::{tunnel_split, AsContext, CellInfo, Ctl, Framer, IpConfig, TunnelJoin, TunnelPacket};
use crate::codec::{fmt_ip, hex};
use crate::crypto::{as_key, ct_eq, site_proof, smc_mac, sr_mac, ReplayWindow};
use crate::db::{Db, Subscriber};
use crate::ip::{ipv4_dst, ipv4_src};
use crate::simauth::{simauth_v1, SimResponse};

pub type ConnId = u32;

/// IPv4 socket address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SockAddr {
    pub ip: [u8; 4],
    pub port: u16,
}

/// Randomness for challenges, nonces and TMSIs.
pub trait Entropy {
    fn fill(&mut self, b: &mut [u8]);
}

#[derive(Debug, PartialEq)]
pub enum CoreOut {
    /// Send on a site connection (encode with [`Ctl::encode`]).
    Send(ConnId, Ctl),
    /// Close a site connection.
    Close(ConnId),
    /// A UDP tunnel datagram to a site.
    Tunnel(SockAddr, Vec<u8>),
    /// A phone's packet for the LAN: write it to the tun.
    ToLan(Vec<u8>),
    /// Write the database file (the whole text).
    SaveDb(String),
    /// A line for the log / Carrier Console.
    Log(String),
}

#[derive(Clone, Debug)]
pub struct CoreConfig {
    pub heartbeat_ms: u16,
    /// Pending challenges and handovers expire after this.
    pub procedure_timeout_ms: u64,
    /// Write usage to the database at most this often.
    pub save_every_ms: u64,
    /// Send `Usage` to the serving site this often while a phone has traffic.
    pub usage_every_ms: u64,
    /// Downlink packets held for an idle phone while it is paged.
    pub page_buffer: usize,
}

impl Default for CoreConfig {
    fn default() -> Self {
        CoreConfig { heartbeat_ms: 1000, procedure_timeout_ms: 5000, save_every_ms: 5000, usage_every_ms: 2000, page_buffer: 32 }
    }
}

struct Conn {
    peer: [u8; 4],
    framer: Framer,
    nonce: [u8; 16],
    site: Option<SiteReg>,
    last_rx_ms: u64,
    last_tx_ms: u64,
    hb_seq: u32,
}

#[derive(Clone)]
struct SiteReg {
    id: String,
    cells: Vec<CellInfo>,
    tunnel: SockAddr,
}

/// A challenge in flight (no session yet).
struct PendingAuth {
    imsi: String,
    cell: u32,
    rand: [u8; 16],
    resp: SimResponse,
    started_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Serving {
    conn: ConnId,
    ue: u32,
    cell: u32,
}

struct Ho {
    id: u32,
    target: Serving,
    acked: bool,
    started_ms: u64,
}

struct Session {
    id: u32,
    imsi: String,
    ip: [u8; 4],
    tmsi: u32,
    rand: [u8; 16],
    resp: SimResponse,
    epoch: u32,
    serving: Option<Serving>,
    active: bool,
    idle: bool,
    ho: Option<Ho>,
    dl_seq: u32,
    /// Uplink tunnel sequence numbers seen, per sending site (each site counts its own).
    ul_seen: BTreeMap<ConnId, ReplayWindow>,
    sr_count: u32,
    page: Option<(u64, VecDeque<Vec<u8>>)>,
    used_since_report: u64,
    last_usage_ms: u64,
}

/// What the Carrier Console shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    pub session: u32,
    pub imsi: String,
    pub ip: [u8; 4],
    pub site: Option<String>,
    pub cell: Option<u32>,
    pub state: &'static str,
    pub used: u64,
    pub quota: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteInfo {
    pub conn: ConnId,
    pub id: String,
    pub peer: [u8; 4],
    pub cells: Vec<u32>,
}

pub struct Core {
    pub cfg: CoreConfig,
    db: Db,
    conns: BTreeMap<ConnId, Conn>,
    sessions: BTreeMap<u32, Session>,
    by_ip: BTreeMap<[u8; 4], u32>,
    by_tmsi: BTreeMap<u32, u32>,
    pending: BTreeMap<(ConnId, u32), PendingAuth>,
    next_session: u32,
    next_ho: u32,
    join: TunnelJoin,
    dirty: bool,
    last_save_ms: u64,
    out: Vec<CoreOut>,
    entropy: alloc::boxed::Box<dyn Entropy + Send>,
}

impl Core {
    pub fn new(db: Db, cfg: CoreConfig, mut entropy: alloc::boxed::Box<dyn Entropy + Send>) -> Core {
        // Session ids start somewhere random, so a restarted core never
        // reuses the ids its sites and phones still remember.
        let mut b = [0u8; 4];
        entropy.fill(&mut b);
        let first = (u32::from_be_bytes(b) & 0x7fff_ffff).max(1);
        Core {
            cfg,
            db,
            conns: BTreeMap::new(),
            sessions: BTreeMap::new(),
            by_ip: BTreeMap::new(),
            by_tmsi: BTreeMap::new(),
            pending: BTreeMap::new(),
            next_session: first,
            next_ho: 1,
            join: TunnelJoin::default(),
            dirty: false,
            last_save_ms: 0,
            out: Vec::new(),
            entropy,
        }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn take_out(&mut self) -> Vec<CoreOut> {
        core::mem::take(&mut self.out)
    }

    fn log(&mut self, s: String) {
        self.out.push(CoreOut::Log(s));
    }

    fn send(&mut self, c: ConnId, m: Ctl) {
        if self.conns.contains_key(&c) {
            self.out.push(CoreOut::Send(c, m));
        }
    }

    fn random<const N: usize>(&mut self) -> [u8; N] {
        let mut b = [0u8; N];
        self.entropy.fill(&mut b);
        b
    }

    fn save(&mut self, now: u64) {
        self.dirty = false;
        self.last_save_ms = now;
        self.out.push(CoreOut::SaveDb(self.db.to_text()));
    }

    // ------------------------------------------------------------ connections

    /// A site connected (TCP accepted) from `peer`.
    pub fn on_connect(&mut self, c: ConnId, peer: [u8; 4], now: u64) {
        let nonce = self.random::<16>();
        self.conns.insert(c, Conn { peer, framer: Framer::new(), nonce, site: None, last_rx_ms: now, last_tx_ms: now, hb_seq: 0 });
        let carrier = self.db.carrier.clone();
        self.send(c, Ctl::CoreHello { version: crate::backhaul::VERSION, carrier, nonce });
    }

    /// The connection closed (or the driver gave up on it).
    pub fn on_disconnect(&mut self, c: ConnId, now: u64) {
        let Some(conn) = self.conns.remove(&c) else { return };
        if let Some(s) = conn.site {
            self.log(format!("site {} disconnected", s.id));
        }
        self.pending.retain(|(pc, _), _| *pc != c);
        let ids: Vec<u32> = self.sessions.iter().filter(|(_, s)| s.serving.map(|v| v.conn) == Some(c)).map(|(k, _)| *k).collect();
        for id in ids {
            self.end_session(id, Cause::RadioLost, false, now);
        }
        // handovers towards that site fail
        for s in self.sessions.values_mut() {
            if s.ho.as_ref().is_some_and(|h| h.target.conn == c) {
                s.ho = None;
            }
        }
    }

    /// Bytes from a site connection.
    pub fn on_bytes(&mut self, c: ConnId, bytes: &[u8], now: u64) {
        let Some(conn) = self.conns.get_mut(&c) else { return };
        conn.framer.push(bytes);
        conn.last_rx_ms = now;
        loop {
            let Some(conn) = self.conns.get_mut(&c) else { return };
            let m = conn.framer.pop();
            if conn.framer.broken {
                self.log(format!("conn {}: framing error, closing", c));
                self.out.push(CoreOut::Close(c));
                self.on_disconnect(c, now);
                return;
            }
            match m {
                Some(m) => self.on_ctl(c, m, now),
                None => return,
            }
        }
    }

    fn site_of(&self, c: ConnId) -> Option<&SiteReg> {
        self.conns.get(&c)?.site.as_ref()
    }

    fn conn_for_cell(&self, cell: u32) -> Option<(ConnId, CellInfo)> {
        for (id, c) in &self.conns {
            if let Some(s) = &c.site {
                if let Some(ci) = s.cells.iter().find(|ci| ci.cell_id == cell) {
                    return Some((*id, ci.clone()));
                }
            }
        }
        None
    }

    fn on_ctl(&mut self, c: ConnId, m: Ctl, now: u64) {
        let registered = self.site_of(c).is_some();
        if !registered {
            match m {
                Ctl::SiteAuth { version, site_id, cells, tunnel_port, proof } => self.site_auth(c, version, site_id, cells, tunnel_port, proof, now),
                other => {
                    self.log(format!("conn {}: {} before registration, closing", c, other.name()));
                    self.out.push(CoreOut::Close(c));
                    self.on_disconnect(c, now);
                }
            }
            return;
        }
        match m {
            Ctl::Heartbeat { .. } => {}
            Ctl::UeAttach { ue, cell_id, imsi, carrier } => self.ue_attach(c, ue, cell_id, imsi, carrier, now),
            Ctl::UeAuthResponse { ue, res } => self.ue_auth_response(c, ue, res, now),
            Ctl::UeService { ue, cell_id, tmsi, count, mac } => self.ue_service(c, ue, cell_id, tmsi, count, mac, now),
            Ctl::UeActive { session } => {
                let mut flush = None;
                if let Some(s) = self.sessions.get_mut(&session) {
                    if s.serving.map(|v| v.conn) == Some(c) {
                        s.active = true;
                        s.idle = false;
                        flush = s.page.take();
                    }
                }
                if let Some((_, pkts)) = flush {
                    for p in pkts {
                        self.downlink(session, p, now);
                    }
                }
            }
            Ctl::UeRelease { session, cause } => {
                if self.sessions.get(&session).and_then(|s| s.serving).map(|v| v.conn) == Some(c) {
                    self.end_session(session, cause, false, now);
                }
            }
            Ctl::UeIdle { session } => {
                if let Some(s) = self.sessions.get_mut(&session) {
                    if s.serving.map(|v| v.conn) == Some(c) {
                        s.serving = None;
                        s.active = false;
                        s.idle = true;
                        s.ho = None;
                        let imsi = s.imsi.clone();
                        self.log(format!("{} idle", imsi));
                    }
                }
            }
            Ctl::HoRequired { session, target_cell } => self.ho_required(c, session, target_cell, now),
            Ctl::HoRequestAck { ho, ue_id, grant } => self.ho_ack(c, ho, ue_id, grant),
            Ctl::HoFailure { ho, session, cause } => {
                let mut src = None;
                if let Some(s) = self.sessions.get_mut(&session) {
                    if s.ho.as_ref().is_some_and(|h| h.id == ho && h.target.conn == c) {
                        s.ho = None;
                        src = s.serving.map(|v| v.conn);
                    }
                }
                if let Some(src) = src {
                    self.log(format!("handover {} failed: {}", ho, cause));
                    self.send(src, Ctl::HoFailure { ho, session, cause });
                }
            }
            Ctl::PathSwitch { ho, session } => self.path_switch(c, ho, session),
            other => self.log(format!("conn {}: unexpected {}", c, other.name())),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn site_auth(&mut self, c: ConnId, version: u8, site_id: String, cells: Vec<CellInfo>, tunnel_port: u16, proof: [u8; 32], now: u64) {
        let nonce = self.conns[&c].nonce;
        let ok = version == crate::backhaul::VERSION
            && self.db.sites.get(&site_id).is_some_and(|secret| ct_eq(&site_proof(secret, &nonce, &site_id), &proof))
            && !cells.is_empty();
        if !ok {
            self.log(format!("site '{}' refused: bad credential", site_id));
            self.send(c, Ctl::SiteReject { cause: Cause::AuthFailed });
            self.out.push(CoreOut::Close(c));
            self.on_disconnect(c, now);
            return;
        }
        // A site that reconnects (or another holding one of its cells) replaces the old connection.
        let stale: Vec<ConnId> = self
            .conns
            .iter()
            .filter(|(id, k)| {
                **id != c && k.site.as_ref().is_some_and(|s| s.id == site_id || s.cells.iter().any(|x| cells.iter().any(|y| y.cell_id == x.cell_id)))
            })
            .map(|(id, _)| *id)
            .collect();
        for s in stale {
            self.out.push(CoreOut::Close(s));
            self.on_disconnect(s, now);
        }
        let peer = self.conns[&c].peer;
        let cell_list: Vec<String> = cells.iter().map(|x| format!("{}", x.cell_id)).collect();
        self.log(format!("site {} registered from {} (cells {})", site_id, fmt_ip(peer), cell_list.join(",")));
        if let Some(conn) = self.conns.get_mut(&c) {
            conn.site = Some(SiteReg { id: site_id, cells, tunnel: SockAddr { ip: peer, port: tunnel_port } });
        }
        let revoked: Vec<String> = self.db.subs.iter().filter(|(_, s)| s.revoked).map(|(i, _)| i.clone()).collect();
        let hb = self.cfg.heartbeat_ms;
        self.send(c, Ctl::SiteAccept { heartbeat_ms: hb, revoked });
    }

    // ------------------------------------------------------------ attach

    fn reject(&mut self, c: ConnId, ue: u32, imsi: &str, cause: Cause) {
        self.log(format!("attach {} rejected: {}", imsi, cause));
        self.send(c, Ctl::UeReject { ue, cause });
    }

    fn ue_attach(&mut self, c: ConnId, ue: u32, cell: u32, imsi: String, carrier: String, now: u64) {
        if carrier != self.db.carrier {
            return self.reject(c, ue, &imsi, Cause::WrongCarrier);
        }
        let Some(sub) = self.db.subs.get(&imsi).cloned() else {
            return self.reject(c, ue, &imsi, Cause::UnknownSubscriber);
        };
        if sub.revoked {
            return self.reject(c, ue, &imsi, Cause::Revoked);
        }
        if sub.over_quota() {
            return self.reject(c, ue, &imsi, Cause::QuotaExhausted);
        }
        let rand = self.random::<16>();
        let resp = simauth_v1(&sub.key, &rand);
        self.pending.insert((c, ue), PendingAuth { imsi, cell, rand, resp, started_ms: now });
        self.send(c, Ctl::UeChallenge { ue, rand });
    }

    fn ue_auth_response(&mut self, c: ConnId, ue: u32, res: [u8; 16], now: u64) {
        let Some(p) = self.pending.remove(&(c, ue)) else { return };
        if !ct_eq(&p.resp.res, &res) {
            return self.reject(c, ue, &p.imsi, Cause::AuthFailed);
        }
        // A phone attaching again ends its old session (another site, or lost).
        let old: Vec<u32> = self.sessions.iter().filter(|(_, s)| s.imsi == p.imsi).map(|(k, _)| *k).collect();
        for id in old {
            self.end_session(id, Cause::SessionLost, true, now);
        }
        let Some(ip) = self.db.address_for(&p.imsi) else {
            return self.reject(c, ue, &p.imsi, Cause::Congestion);
        };
        let id = self.next_session;
        self.next_session = self.next_session.wrapping_add(1).max(1);
        let tmsi = self.new_tmsi();
        let epoch = 1;
        let s = Session {
            id,
            imsi: p.imsi.clone(),
            ip,
            tmsi,
            rand: p.rand,
            resp: p.resp,
            epoch,
            serving: Some(Serving { conn: c, ue, cell: p.cell }),
            active: false,
            idle: false,
            ho: None,
            dl_seq: 0,
            ul_seen: BTreeMap::new(),
            sr_count: 0,
            page: None,
            used_since_report: 0,
            last_usage_ms: now,
        };
        self.by_ip.insert(ip, id);
        self.by_tmsi.insert(tmsi, id);
        self.sessions.insert(id, s);
        let site = self.site_of(c).map(|s| s.id.clone()).unwrap_or_default();
        self.log(format!("attach {} at {} cell {}: ip {} session {}", p.imsi, site, p.cell, fmt_ip(ip), id));
        self.save(now);
        self.send_accept(id, c, ue, p.cell);
    }

    fn new_tmsi(&mut self) -> u32 {
        loop {
            let t = u32::from_be_bytes(self.random::<4>());
            if t != 0 && !self.by_tmsi.contains_key(&t) {
                return t;
            }
        }
    }

    fn ip_config(&self, ip: [u8; 4]) -> IpConfig {
        IpConfig { ip, prefix: self.db.prefix, gateway: self.db.gateway, dns: self.db.dns, mtu: self.db.mtu }
    }

    fn send_accept(&mut self, id: u32, c: ConnId, ue: u32, cell: u32) {
        // a new radio context at that site: its uplink sequence numbers start over
        self.sessions.get_mut(&id).expect("live session").ul_seen.remove(&c);
        let s = &self.sessions[&id];
        let ctx = AsContext { epoch: s.epoch, k_as: as_key(&s.resp.ks_enc(), cell, s.epoch) };
        let mac = smc_mac(&s.resp.ks_int(), &s.rand, cell, s.epoch, ue);
        let m = Ctl::UeAccept { ue, session: id, ip: self.ip_config(s.ip), tmsi: s.tmsi, ctx, smc_mac: mac };
        self.send(c, m);
    }

    #[allow(clippy::too_many_arguments)]
    fn ue_service(&mut self, c: ConnId, ue: u32, cell: u32, tmsi: u32, count: u32, mac: [u8; 16], _now: u64) {
        let Some(&id) = self.by_tmsi.get(&tmsi) else {
            self.send(c, Ctl::UeReject { ue, cause: Cause::SessionLost });
            return;
        };
        let s = self.sessions.get_mut(&id).expect("by_tmsi in sync");
        if count <= s.sr_count || !ct_eq(&sr_mac(&s.resp.ks_int(), tmsi, count), &mac) {
            let imsi = s.imsi.clone();
            self.log(format!("service request for {} failed verification", imsi));
            self.send(c, Ctl::UeReject { ue, cause: Cause::SessionLost });
            return;
        }
        s.sr_count = count;
        s.epoch = s.epoch.wrapping_add(1);
        s.serving = Some(Serving { conn: c, ue, cell });
        s.active = false;
        s.idle = false;
        let imsi = s.imsi.clone();
        self.log(format!("{} resumed at cell {}", imsi, cell));
        self.send_accept(id, c, ue, cell);
    }

    /// End a session: tell its serving site (unless it told us), forget the tunnel.
    fn end_session(&mut self, id: u32, cause: Cause, notify_site: bool, now: u64) {
        let Some(s) = self.sessions.remove(&id) else { return };
        self.by_ip.remove(&s.ip);
        self.by_tmsi.remove(&s.tmsi);
        self.join.forget(id);
        if notify_site {
            if let Some(v) = s.serving {
                self.send(v.conn, Ctl::UeRelease { session: id, cause });
            }
        }
        if let Some(h) = s.ho {
            self.send(h.target.conn, Ctl::UeRelease { session: id, cause: Cause::HandoverFailed });
        }
        self.log(format!("session {} ({}) ended: {}", id, s.imsi, cause));
        if self.dirty {
            self.save(now);
        }
    }

    // ------------------------------------------------------------ handover

    fn ho_required(&mut self, c: ConnId, session: u32, target_cell: u32, now: u64) {
        let target = self.conn_for_cell(target_cell);
        let ipc;
        let fail = {
            let Some(s) = self.sessions.get(&session) else { return };
            if s.serving.map(|v| v.conn) != Some(c) || !s.active {
                return;
            }
            ipc = self.ip_config(s.ip);
            if s.ho.is_some() {
                return;
            }
            target.is_none()
        };
        if fail {
            self.log(format!("handover of session {} to unknown cell {}", session, target_cell));
            self.send(c, Ctl::HoFailure { ho: 0, session, cause: Cause::HandoverFailed });
            return;
        }
        let (tconn, _cell) = target.expect("checked");
        let ho = self.next_ho;
        self.next_ho = self.next_ho.wrapping_add(1).max(1);
        let s = self.sessions.get_mut(&session).expect("checked");
        let ue = s.serving.expect("checked").ue;
        s.epoch = s.epoch.wrapping_add(1);
        let ctx = AsContext { epoch: s.epoch, k_as: as_key(&s.resp.ks_enc(), target_cell, s.epoch) };
        s.ho = Some(Ho { id: ho, target: Serving { conn: tconn, ue, cell: target_cell }, acked: false, started_ms: now });
        s.ul_seen.remove(&tconn);
        let tmsi = s.tmsi;
        self.send(tconn, Ctl::HoRequest { ho, session, ue, target_cell, ip: ipc, tmsi, ctx });
    }

    fn ho_ack(&mut self, c: ConnId, ho: u32, ue_id: u16, grant: Grant) {
        let mut cmd = None;
        for s in self.sessions.values_mut() {
            if let Some(h) = s.ho.as_mut() {
                if h.id == ho && h.target.conn == c {
                    h.acked = true;
                    cmd = Some((s.id, s.serving.map(|v| v.conn), h.target.cell, s.epoch));
                }
            }
        }
        let Some((session, Some(src), cell, epoch)) = cmd else { return };
        let Some((_, ci)) = self.conn_for_cell(cell) else { return };
        self.send(src, Ctl::HoCommand { ho, session, cell_id: cell, addr: ci.addr, profile: ci.profile, channel: ci.channel, epoch, ue_id, grant });
    }

    fn path_switch(&mut self, c: ConnId, ho: u32, session: u32) {
        let Some(s) = self.sessions.get_mut(&session) else { return };
        let Some(h) = s.ho.take() else { return };
        if h.id != ho || h.target.conn != c {
            s.ho = Some(h);
            return;
        }
        let old = s.serving.replace(h.target);
        s.active = true;
        s.ul_seen.retain(|c, _| *c == h.target.conn);
        let imsi = s.imsi.clone();
        let ip = s.ip;
        if let Some(o) = old {
            self.send(o.conn, Ctl::UeRelease { session, cause: Cause::HandoverDone });
        }
        self.send(c, Ctl::PathSwitchAck { session });
        let from = old.map(|o| o.cell).unwrap_or(0);
        self.log(format!("handover {} cell {} -> {} (ip {} kept)", imsi, from, h.target.cell, fmt_ip(ip)));
    }

    // ------------------------------------------------------------ user plane

    /// A UDP datagram on the tunnel port.
    pub fn on_tunnel(&mut self, from: SockAddr, bytes: &[u8], now: u64) {
        let Ok(p) = TunnelPacket::decode(bytes) else { return };
        let Some(s) = self.sessions.get(&p.session) else { return };
        // Only the serving site (or the handover target) may send for a session.
        let src_conn = |v: Option<Serving>| v.filter(|v| self.site_of(v.conn).is_some_and(|r| r.tunnel == from)).map(|v| v.conn);
        let Some(conn) = src_conn(s.serving).or_else(|| src_conn(s.ho.as_ref().map(|h| h.target))) else { return };
        let Some((id, seq, pkt)) = self.join.push(p) else { return };
        let s = self.sessions.get_mut(&id).expect("checked");
        let w = s.ul_seen.entry(conn).or_default();
        if !w.check(seq as u64) {
            return;
        }
        w.accept(seq as u64);
        if ipv4_src(&pkt) != Some(s.ip) {
            return; // spoofed source
        }
        if !self.account(id, pkt.len(), now) {
            return;
        }
        self.out.push(CoreOut::ToLan(pkt));
    }

    /// Count `n` bytes; false (and the session released) once over quota.
    fn account(&mut self, id: u32, n: usize, now: u64) -> bool {
        let s = self.sessions.get_mut(&id).expect("live session");
        let Some(sub) = self.db.subs.get_mut(&s.imsi) else { return false };
        if sub.over_quota() {
            return false;
        }
        sub.used += n as u64;
        s.used_since_report += n as u64;
        self.dirty = true;
        if sub.over_quota() {
            let (imsi, used, quota) = (s.imsi.clone(), sub.used, sub.quota);
            self.log(format!("{} quota exhausted ({} of {} bytes)", imsi, used, quota));
            self.save(now);
            self.end_session(id, Cause::QuotaExhausted, true, now);
            return false;
        }
        true
    }

    /// An IP packet from the LAN (the tun) for a phone.
    pub fn on_lan_packet(&mut self, pkt: &[u8], now: u64) {
        let Some(dst) = ipv4_dst(pkt) else { return };
        let Some(&id) = self.by_ip.get(&dst) else { return };
        self.downlink(id, pkt.to_vec(), now);
    }

    fn downlink(&mut self, id: u32, pkt: Vec<u8>, now: u64) {
        let Some(s) = self.sessions.get_mut(&id) else { return };
        if s.idle {
            let cap = self.cfg.page_buffer;
            let first = s.page.is_none();
            let (_, q) = s.page.get_or_insert_with(|| (now, VecDeque::new()));
            if q.len() >= cap {
                q.pop_front();
            }
            q.push_back(pkt);
            if first {
                let tmsi = s.tmsi;
                self.page_all(tmsi);
            }
            return;
        }
        if !s.active && s.ho.is_none() {
            return;
        }
        if !self.account(id, pkt.len(), now) {
            return;
        }
        let s = self.sessions.get_mut(&id).expect("still live");
        s.dl_seq = s.dl_seq.wrapping_add(1).max(1);
        let seq = s.dl_seq;
        let mut targets: Vec<Serving> = s.serving.into_iter().collect();
        if let Some(h) = s.ho.as_ref().filter(|h| h.acked) {
            targets.push(h.target); // make-before-break: bicast until the path switch
        }
        for t in targets {
            if let Some(addr) = self.site_of(t.conn).map(|r| r.tunnel) {
                for d in tunnel_split(id, seq, &pkt) {
                    self.out.push(CoreOut::Tunnel(addr, d));
                }
            }
        }
    }

    fn page_all(&mut self, tmsi: u32) {
        let conns: Vec<ConnId> = self.conns.iter().filter(|(_, c)| c.site.is_some()).map(|(k, _)| *k).collect();
        for c in conns {
            self.send(c, Ctl::Page { tmsi });
        }
    }

    // ------------------------------------------------------------ time

    pub fn tick(&mut self, now: u64) {
        let hb = self.cfg.heartbeat_ms as u64;
        let dead: Vec<ConnId> = self.conns.iter().filter(|(_, c)| now.saturating_sub(c.last_rx_ms) > 3 * hb + hb / 2).map(|(k, _)| *k).collect();
        for c in dead {
            self.log(format!("conn {}: heartbeat lost", c));
            self.out.push(CoreOut::Close(c));
            self.on_disconnect(c, now);
        }
        let due: Vec<ConnId> = self.conns.iter().filter(|(_, c)| c.site.is_some() && now.saturating_sub(c.last_tx_ms) >= hb).map(|(k, _)| *k).collect();
        for c in due {
            let conn = self.conns.get_mut(&c).expect("listed");
            conn.hb_seq += 1;
            conn.last_tx_ms = now;
            let seq = conn.hb_seq;
            self.out.push(CoreOut::Send(c, Ctl::Heartbeat { seq }));
        }
        let to = self.cfg.procedure_timeout_ms;
        self.pending.retain(|_, p| now.saturating_sub(p.started_ms) < to);
        let mut expired = Vec::new();
        let mut usage = Vec::new();
        let mut repage = Vec::new();
        for s in self.sessions.values_mut() {
            if let Some(h) = &s.ho {
                if now.saturating_sub(h.started_ms) > to {
                    expired.push((s.id, h.target.conn));
                    s.ho = None;
                }
            }
            if s.used_since_report > 0 && now.saturating_sub(s.last_usage_ms) >= self.cfg.usage_every_ms {
                s.used_since_report = 0;
                s.last_usage_ms = now;
                if let (Some(v), Some(sub)) = (s.serving, self.db.subs.get(&s.imsi)) {
                    usage.push((v.conn, Ctl::Usage { session: s.id, used: sub.used, quota: sub.quota }));
                }
            }
            if let Some((t, _)) = &mut s.page {
                if now.saturating_sub(*t) >= 1000 {
                    *t = now;
                    repage.push(s.tmsi);
                }
            }
        }
        for (id, tc) in expired {
            self.log(format!("handover of session {} timed out", id));
            self.send(tc, Ctl::UeRelease { session: id, cause: Cause::HandoverFailed });
        }
        for (c, m) in usage {
            self.send(c, m);
        }
        for t in repage {
            self.page_all(t);
        }
        if self.dirty && now.saturating_sub(self.last_save_ms) >= self.cfg.save_every_ms {
            self.save(now);
        }
    }

    // ------------------------------------------------------------ admin + status

    /// Operator commands (the Carrier Console and `cored`'s admin socket):
    /// `list`, `sites`, `sessions`, `add <imsi> <key-hex> [quota]`, `revoke <imsi>`,
    /// `unrevoke <imsi>`, `quota <imsi> <bytes>`, `reset-usage <imsi>`,
    /// `site-add <id> <secret-hex>`, `release <imsi>`.
    pub fn admin(&mut self, line: &str, now: u64) -> Result<String, String> {
        let w: Vec<&str> = line.split_whitespace().collect();
        let need_sub = |db: &Db, imsi: &str| -> Result<(), String> {
            if db.subs.contains_key(imsi) {
                Ok(())
            } else {
                Err(format!("no subscriber {}", imsi))
            }
        };
        let r = match w.as_slice() {
            ["list"] => {
                let mut s = String::new();
                for (imsi, u) in &self.db.subs {
                    let ip = u.ip.map(fmt_ip).unwrap_or_else(|| "-".into());
                    s.push_str(&format!("{} ip {} used {} quota {}{}\n", imsi, ip, u.used, u.quota, if u.revoked { " REVOKED" } else { "" }));
                }
                return Ok(s);
            }
            ["sites"] => {
                let s: Vec<String> = self.sites().iter().map(|x| format!("{} {} cells {:?}", x.id, fmt_ip(x.peer), x.cells)).collect();
                return Ok(s.join("\n"));
            }
            ["sessions"] => {
                let s: Vec<String> =
                    self.sessions().iter().map(|x| format!("{} {} {} {} cell {:?} {}", x.session, x.imsi, fmt_ip(x.ip), x.state, x.cell, x.used)).collect();
                return Ok(s.join("\n"));
            }
            ["add", imsi, key] | ["add", imsi, key, _] => {
                if !crate::simauth::valid_imsi(imsi) {
                    return Err(format!("bad IMSI {}", imsi));
                }
                let k = crate::codec::unhex::<16>(key).ok_or("key must be 32 hex digits")?;
                let mut s = Subscriber::new(k);
                if let Some(q) = w.get(3) {
                    s.quota = q.parse().map_err(|_| "bad quota")?;
                }
                if self.db.subs.contains_key(*imsi) {
                    return Err(format!("{} exists", imsi));
                }
                self.db.subs.insert((*imsi).into(), s);
                format!("added {}", imsi)
            }
            ["site-add", id, secret] => {
                let k = crate::codec::unhex::<16>(secret).ok_or("secret must be 32 hex digits")?;
                self.db.sites.insert((*id).into(), k);
                format!("site {} added", id)
            }
            ["revoke", imsi] | ["unrevoke", imsi] => {
                need_sub(&self.db, imsi)?;
                let revoke = w[0] == "revoke";
                self.db.subs.get_mut(*imsi).expect("checked").revoked = revoke;
                if revoke {
                    let conns: Vec<ConnId> = self.conns.iter().filter(|(_, c)| c.site.is_some()).map(|(k, _)| *k).collect();
                    for c in conns {
                        self.send(c, Ctl::Revoked { imsi: (*imsi).into() });
                    }
                    let ids: Vec<u32> = self.sessions.iter().filter(|(_, s)| s.imsi == *imsi).map(|(k, _)| *k).collect();
                    for id in ids {
                        self.end_session(id, Cause::Revoked, true, now);
                    }
                }
                format!("{} {}", imsi, if revoke { "revoked" } else { "reinstated" })
            }
            ["quota", imsi, q] => {
                need_sub(&self.db, imsi)?;
                self.db.subs.get_mut(*imsi).expect("checked").quota = q.parse().map_err(|_| "bad quota")?;
                format!("{} quota {}", imsi, q)
            }
            ["reset-usage", imsi] => {
                need_sub(&self.db, imsi)?;
                self.db.subs.get_mut(*imsi).expect("checked").used = 0;
                format!("{} usage reset", imsi)
            }
            ["release", imsi] => {
                let ids: Vec<u32> = self.sessions.iter().filter(|(_, s)| s.imsi == *imsi).map(|(k, _)| *k).collect();
                if ids.is_empty() {
                    return Err(format!("{} has no session", imsi));
                }
                for id in ids {
                    self.end_session(id, Cause::Admin, true, now);
                }
                format!("{} released", imsi)
            }
            _ => return Err(format!("unknown command '{}'", line.trim())),
        };
        self.log(format!("admin: {}", r));
        self.save(now);
        Ok(r)
    }

    pub fn sites(&self) -> Vec<SiteInfo> {
        self.conns
            .iter()
            .filter_map(|(k, c)| c.site.as_ref().map(|s| SiteInfo { conn: *k, id: s.id.clone(), peer: c.peer, cells: s.cells.iter().map(|x| x.cell_id).collect() }))
            .collect()
    }

    pub fn sessions(&self) -> Vec<SessionInfo> {
        self.sessions
            .values()
            .map(|s| {
                let sub = self.db.subs.get(&s.imsi);
                SessionInfo {
                    session: s.id,
                    imsi: s.imsi.clone(),
                    ip: s.ip,
                    site: s.serving.and_then(|v| self.site_of(v.conn)).map(|r| r.id.clone()),
                    cell: s.serving.map(|v| v.cell),
                    state: if s.idle {
                        "idle"
                    } else if s.ho.is_some() {
                        "handover"
                    } else if s.active {
                        "active"
                    } else {
                        "attaching"
                    },
                    used: sub.map(|u| u.used).unwrap_or(0),
                    quota: sub.map(|u| u.quota).unwrap_or(0),
                }
            })
            .collect()
    }

    /// Connected-and-registered site ids (for tests and the console).
    pub fn site_ids(&self) -> BTreeSet<String> {
        self.sites().into_iter().map(|s| s.id).collect()
    }

    /// The key of `imsi` as hex, for writing a SIM through the Card Reader/Writer.
    pub fn key_hex(&self, imsi: &str) -> Option<String> {
        self.db.subs.get(imsi).map(|s| hex(&s.key))
    }
}
