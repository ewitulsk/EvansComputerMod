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

use crate::air::{Bearer, Cause, Grant, ImsInfo, QCI_VOICE};
use crate::backhaul::{tunnel_split, AsContext, CellInfo, Ctl, Framer, IpConfig, TunnelJoin, TunnelPacket};
use crate::codec::{fmt_ip, hex};
use crate::crypto::{as_key, ct_eq, site_proof, smc_mac, sr_mac, Backhaul, ReplayWindow};
use crate::db::{Db, Subscriber};
use crate::ip::{in_subnet, ipv4_dst, ipv4_src};
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
    /// Bytes for a site connection (a control message, sealed once the site registered).
    Send(ConnId, Vec<u8>),
    /// Close a site connection.
    Close(ConnId),
    /// A UDP tunnel datagram (sealed) to a site.
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
    /// The backhaul keys, from the registration on.
    bh: Option<Backhaul>,
    since_ms: u64,
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
    /// The IMS bearer's address (calls and texts), when the carrier offers them.
    ims_ip: Option<[u8; 4]>,
    /// Dedicated bearers (a call's voice flow) and whether the serving site admitted each.
    bearers: Vec<(Bearer, bool)>,
    next_bearer: u8,
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
    pub msisdn: String,
    pub ims_ip: Option<[u8; 4]>,
    /// Voice bearers (admitted by the site or best effort).
    pub bearers: Vec<(Bearer, bool)>,
}

/// Where a subscriber can be reached (the HSS answer for routing a call or a text).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reach {
    pub imsi: String,
    pub msisdn: String,
    /// The IMS address, when the phone has a session (attached or idle).
    pub ims_ip: Option<[u8; 4]>,
    /// The session is idle (a page wakes it).
    pub idle: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteInfo {
    pub conn: ConnId,
    pub id: String,
    pub peer: [u8; 4],
    pub cells: Vec<u32>,
    /// When it registered, ms (the core's clock).
    pub since_ms: u64,
    /// When the last bytes (a heartbeat at least) arrived from it, ms.
    pub last_rx_ms: u64,
    /// Sealed messages and datagrams from it that failed to open.
    pub rejected: u64,
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
    /// Tunnel datagrams that matched no site's keys (forged, tampered, replayed, or from no site).
    pub tunnel_rejects: u64,
    /// IMS-bearer packets dropped for leaving the IMS core network.
    pub ims_policy_drops: u64,
    /// IMS registrations the HSS holds: IMSI -> serving CSCF (from `SAR`).
    pub(crate) ims_regs: BTreeMap<String, String>,
    last_reject_log_ms: Option<u64>,
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
        let mut db = db;
        // Subscribers without a number get one as soon as the carrier holds a block.
        let filled = db.fill_numbers();
        let mut core = Core {
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
            tunnel_rejects: 0,
            ims_policy_drops: 0,
            ims_regs: BTreeMap::new(),
            last_reject_log_ms: None,
            out: Vec::new(),
            entropy,
        };
        if filled > 0 {
            core.log(format!("gave {} subscriber(s) a number", filled));
            core.save(0);
        }
        core
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

    /// Queue a control message, sealed once the connection has its keys.
    fn send(&mut self, c: ConnId, m: Ctl) {
        let Some(conn) = self.conns.get_mut(&c) else { return };
        let bytes = match conn.bh.as_mut() {
            Some(bh) => bh.ctl.seal_ctl(&m.encode()[4..]),
            None => m.encode(),
        };
        self.out.push(CoreOut::Send(c, bytes));
    }

    pub(crate) fn random16(&mut self) -> [u8; 16] {
        self.random::<16>()
    }

    pub(crate) fn log_line(&mut self, s: String) {
        self.log(s);
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
        self.conns.insert(c, Conn { peer, framer: Framer::new(), nonce, site: None, last_rx_ms: now, last_tx_ms: now, hb_seq: 0, bh: None, since_ms: now });
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
            let body = conn.framer.pop_body();
            if conn.framer.broken {
                let who = conn.site.as_ref().map(|s| format!("site {}", s.id)).unwrap_or_else(|| format!("conn {}", c));
                self.log(format!("{}: control stream broken (framing error), closing", who));
                self.out.push(CoreOut::Close(c));
                self.on_disconnect(c, now);
                return;
            }
            let Some(body) = body else { return };
            let m = match conn.bh.as_mut() {
                // Registered: only sealed messages, in order.
                Some(bh) => match bh.ctl.open_ctl(&body) {
                    Ok(plain) => Ctl::decode(&plain).map_err(|_| "undecodable after opening"),
                    Err(_) => Err("failed authentication"),
                },
                None => Ctl::decode(&body).map_err(|_| "undecodable"),
            };
            match m {
                Ok(m) => self.on_ctl(c, m, now),
                Err(why) => {
                    let who = conn.site.as_ref().map(|s| format!("site {}", s.id)).unwrap_or_else(|| format!("conn {}", c));
                    self.log(format!("{}: control message {}, closing", who, why));
                    self.out.push(CoreOut::Close(c));
                    self.on_disconnect(c, now);
                    return;
                }
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
                Ctl::SiteAuth { version, site_id, cells, tunnel_port, nonce, proof } => self.site_auth(c, version, site_id, cells, tunnel_port, nonce, proof, now),
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
                        s.bearers.clear();
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
            Ctl::BearerResult { session, id, ok } => {
                if let Some(s) = self.sessions.get_mut(&session) {
                    if let Some(e) = s.bearers.iter_mut().find(|(b, _)| b.id == id) {
                        e.1 = ok;
                    }
                    if !ok {
                        let imsi = s.imsi.clone();
                        self.log(format!("{}: voice bearer {} not admitted, the call runs best effort", imsi, id));
                    }
                }
            }
            other => self.log(format!("conn {}: unexpected {}", c, other.name())),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn site_auth(&mut self, c: ConnId, version: u8, site_id: String, cells: Vec<CellInfo>, tunnel_port: u16, site_nonce: [u8; 16], proof: [u8; 32], now: u64) {
        let nonce = self.conns[&c].nonce;
        let secret = self.db.sites.get(&site_id).copied();
        let ok = version == crate::backhaul::VERSION
            && secret.is_some_and(|secret| {
                let hello = Ctl::CoreHello { version: crate::backhaul::VERSION, carrier: self.db.carrier.clone(), nonce };
                let auth = Ctl::SiteAuth { version, site_id: site_id.clone(), cells: cells.clone(), tunnel_port, nonce: site_nonce, proof };
                ct_eq(&site_proof(&secret, &crate::backhaul::auth_transcript(&hello, &auth)), &proof)
            })
            && !cells.is_empty();
        if !ok {
            let why = if version != crate::backhaul::VERSION {
                format!("protocol version {} (this core speaks {})", version, crate::backhaul::VERSION)
            } else if secret.is_none() {
                "unknown site".into()
            } else {
                "bad credential".into()
            };
            self.log(format!("site '{}' refused: {}", site_id, why));
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
            conn.bh = Some(Backhaul::new(&secret.expect("checked"), &nonce, &site_nonce, &site_id, false));
            conn.site = Some(SiteReg { id: site_id, cells, tunnel: SockAddr { ip: peer, port: tunnel_port } });
            conn.since_ms = now;
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
        // The IMS bearer: only for a carrier with IMS and a SIM with a number.
        let has_number = self.db.subs.get(&p.imsi).is_some_and(|s| !s.ims.msisdn.is_empty());
        let ims_ip = if has_number { self.db.ims_address_for(&p.imsi) } else { None };
        let id = self.next_session;
        self.next_session = self.next_session.wrapping_add(1).max(1);
        let tmsi = self.new_tmsi();
        let epoch = 1;
        let s = Session {
            id,
            imsi: p.imsi.clone(),
            ip,
            ims_ip,
            bearers: Vec::new(),
            next_bearer: 1,
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
        if let Some(i) = ims_ip {
            self.by_ip.insert(i, id);
        }
        self.by_tmsi.insert(tmsi, id);
        self.sessions.insert(id, s);
        let site = self.site_of(c).map(|s| s.id.clone()).unwrap_or_default();
        let ims = ims_ip.map(|i| format!(" ims {}", fmt_ip(i))).unwrap_or_default();
        self.log(format!("attach {} at {} cell {}: ip {}{} session {}", p.imsi, site, p.cell, fmt_ip(ip), ims, id));
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

    fn ims_info(&self, s: &Session) -> Option<ImsInfo> {
        let conf = self.db.ims.as_ref()?;
        let ip = s.ims_ip?;
        let msisdn = self.db.subs.get(&s.imsi).map(|u| u.ims.msisdn.clone()).unwrap_or_default();
        Some(ImsInfo { ip, prefix: conf.prefix, pcscf: conf.pcscf, domain: conf.domain.clone(), msisdn })
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
        let ims = self.ims_info(s);
        let m = Ctl::UeAccept { ue, session: id, ip: self.ip_config(s.ip), tmsi: s.tmsi, ctx, smc_mac: mac, ims };
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
        s.bearers.clear();
        let imsi = s.imsi.clone();
        self.log(format!("{} resumed at cell {}", imsi, cell));
        self.send_accept(id, c, ue, cell);
    }

    /// End a session: tell its serving site (unless it told us), forget the tunnel.
    fn end_session(&mut self, id: u32, cause: Cause, notify_site: bool, now: u64) {
        let Some(s) = self.sessions.remove(&id) else { return };
        self.by_ip.remove(&s.ip);
        if let Some(i) = s.ims_ip {
            self.by_ip.remove(&i);
        }
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
        let bearers = s.bearers.iter().map(|(b, _)| *b).collect();
        self.send(tconn, Ctl::HoRequest { ho, session, ue, target_cell, ip: ipc, tmsi, ctx, bearers });
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
        // The site connection whose tunnel key opens it (normally the one site at that address).
        let cands: Vec<ConnId> =
            self.conns.iter().filter(|(_, k)| k.site.as_ref().is_some_and(|s| s.tunnel == from) && k.bh.is_some()).map(|(id, _)| *id).collect();
        let pick = if cands.len() == 1 {
            Some(cands[0])
        } else {
            cands.iter().copied().find(|id| self.conns[id].bh.as_ref().is_some_and(|b| b.tun.tunnel_opens(bytes)))
        };
        let opened = pick.and_then(|id| self.conns.get_mut(&id).and_then(|k| k.bh.as_mut()).and_then(|b| b.tun.open_tunnel(bytes).ok()).map(|p| (id, p)));
        let Some((conn, inner)) = opened else {
            self.tunnel_rejects += 1;
            if self.last_reject_log_ms.is_none_or(|t| now.saturating_sub(t) >= 2000) {
                self.last_reject_log_ms = Some(now);
                let n = self.tunnel_rejects;
                self.log(format!("tunnel datagram from {}:{} failed authentication, dropped ({} so far)", fmt_ip(from.ip), from.port, n));
            }
            return;
        };
        let Ok(p) = TunnelPacket::decode(&inner) else { return };
        let Some(s) = self.sessions.get(&p.session) else { return };
        // Only the serving site (or the handover target) may send for a session.
        let allowed = s.serving.map(|v| v.conn) == Some(conn) || s.ho.as_ref().map(|h| h.target.conn) == Some(conn);
        if !allowed {
            return;
        }
        let Some((id, seq, pkt)) = self.join.push(p) else { return };
        let s = self.sessions.get_mut(&id).expect("checked");
        let w = s.ul_seen.entry(conn).or_default();
        if !w.check(seq as u64) {
            return;
        }
        w.accept(seq as u64);
        let src = ipv4_src(&pkt);
        if src.is_some() && src == s.ims_ip {
            // The IMS bearer reaches only the IMS core (P-CSCF, media), never the internet; not counted.
            let dst = ipv4_dst(&pkt).unwrap_or_default();
            let ok = self.db.ims.as_ref().is_some_and(|c| in_subnet(dst, c.net, c.net_prefix) || dst == c.pcscf);
            if !ok {
                self.ims_policy_drops += 1;
                return;
            }
            self.out.push(CoreOut::ToLan(pkt));
            return;
        }
        if src != Some(s.ip) {
            return; // spoofed source
        }
        if !self.zero_rated(&pkt) && !self.account(id, pkt.len(), now) {
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

    /// MMS traffic to or from the MMSC is not counted against the quota.
    fn zero_rated(&self, pkt: &[u8]) -> bool {
        let Some(m) = self.db.ims.as_ref().and_then(|c| c.mmsc) else { return false };
        ipv4_src(pkt) == Some(m) || ipv4_dst(pkt) == Some(m)
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
        let free = s.ims_ip.is_some() && ipv4_dst(&pkt) == s.ims_ip;
        if !free && !self.zero_rated(&pkt) && !self.account(id, pkt.len(), now) {
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
            let Some(conn) = self.conns.get_mut(&t.conn) else { continue };
            let (Some(addr), Some(bh)) = (conn.site.as_ref().map(|r| r.tunnel), conn.bh.as_mut()) else { continue };
            for d in tunnel_split(id, seq, &pkt) {
                self.out.push(CoreOut::Tunnel(addr, bh.tun.seal_tunnel(&d)));
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
            self.send(c, Ctl::Heartbeat { seq });
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
    /// `site-add <id> <secret-hex>`, `site-remove <id>`, `release <imsi>`, `remove <imsi>`,
    /// `label <imsi> <label>`; `add` takes an optional label after the quota.
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
                    s.push_str(&format!(
                        "{} ip {} used {} quota {}{}{}\n",
                        imsi,
                        ip,
                        u.used,
                        u.quota,
                        if u.revoked { " REVOKED" } else { "" },
                        if u.label.is_empty() { String::new() } else { format!(" label {}", u.label) }
                    ));
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
            ["add", imsi, key] | ["add", imsi, key, _] | ["add", imsi, key, _, _] => {
                if !crate::simauth::valid_imsi(imsi) {
                    return Err(format!("bad IMSI {}", imsi));
                }
                let k = crate::codec::unhex::<16>(key).ok_or("key must be 32 hex digits")?;
                let mut s = Subscriber::new(k);
                if let Some(q) = w.get(3) {
                    s.quota = q.parse().map_err(|_| "bad quota")?;
                }
                if let Some(l) = w.get(4) {
                    if !crate::db::valid_label(l) {
                        return Err(format!("bad label '{}' (1-32 of a-z A-Z 0-9 . _ -)", l));
                    }
                    s.label = (*l).into();
                }
                if self.db.subs.contains_key(*imsi) {
                    return Err(format!("{} exists", imsi));
                }
                self.db.subs.insert((*imsi).into(), s);
                format!("added {}", imsi)
            }
            ["site-add", id, secret] => {
                let k = crate::codec::unhex::<16>(secret).ok_or("secret must be 32 hex digits")?;
                if !crate::db::valid_site_id(id) {
                    return Err(format!("bad site id '{}' (1-32 of a-z A-Z 0-9 . _ -)", id));
                }
                self.db.sites.insert((*id).into(), k);
                format!("site {} added", id)
            }
            ["site-remove", id] => {
                if self.db.sites.remove(*id).is_none() {
                    return Err(format!("no site {}", id));
                }
                let conns: Vec<ConnId> = self.conns.iter().filter(|(_, c)| c.site.as_ref().is_some_and(|s| s.id == *id)).map(|(k, _)| *k).collect();
                for c in conns {
                    self.out.push(CoreOut::Close(c));
                    self.on_disconnect(c, now);
                }
                format!("site {} removed", id)
            }
            ["remove", imsi] => {
                need_sub(&self.db, imsi)?;
                let ids: Vec<u32> = self.sessions.iter().filter(|(_, s)| s.imsi == *imsi).map(|(k, _)| *k).collect();
                for id in ids {
                    self.end_session(id, Cause::Admin, true, now);
                }
                self.db.subs.remove(*imsi);
                format!("{} removed", imsi)
            }
            ["label", imsi, l] => {
                need_sub(&self.db, imsi)?;
                if !crate::db::valid_label(l) {
                    return Err(format!("bad label '{}' (1-32 of a-z A-Z 0-9 . _ -)", l));
                }
                self.db.subs.get_mut(*imsi).expect("checked").label = (*l).into();
                format!("{} label {}", imsi, l)
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
            ["block-add", b] => {
                if !crate::db::valid_block(b) {
                    return Err(format!("bad number block '{}' (NPA-NXX, e.g. 214-555)", b));
                }
                if self.db.blocks.iter().any(|x| x == b) {
                    return Err(format!("block {} is already held", b));
                }
                self.db.blocks.push((*b).into());
                let n = self.db.fill_numbers();
                format!("block {} added ({} subscriber(s) given a number)", b, n)
            }
            ["block-remove", b] => {
                let used = self.db.subs.values().filter(|s| self.db.block_of(&s.ims.msisdn) == Some(*b)).count();
                if used > 0 {
                    return Err(format!("{} numbers of {} are in use: release or move them first", used, b));
                }
                let had = self.db.blocks.len();
                self.db.blocks.retain(|x| x != b);
                if had == self.db.blocks.len() {
                    return Err(format!("no block {}", b));
                }
                format!("block {} removed", b)
            }
            ["blocks"] => {
                let mut s = String::new();
                for b in &self.db.blocks {
                    let used = self.db.subs.values().filter(|x| self.db.block_of(&x.ims.msisdn) == Some(b.as_str())).count();
                    s.push_str(&format!("block {} {} used {} free\n", b, used, 9999 + 1 - crate::db::FIRST_LINE as usize - used));
                }
                return Ok(s);
            }
            ["numbers"] => {
                let mut s = String::new();
                for (imsi, u) in &self.db.subs {
                    if !u.ims.msisdn.is_empty() {
                        s.push_str(&format!("num {} {} {}\n", u.ims.msisdn, imsi, if u.label.is_empty() { "-" } else { &u.label }));
                    }
                }
                return Ok(s);
            }
            ["number", who] | ["number", who, _] => {
                let imsi = self.imsi_of(who).ok_or_else(|| format!("no subscriber {}", who))?;
                let n = self.db.assign_number(&imsi, w.get(2).copied())?;
                format!("{} number {}", imsi, n)
            }
            ["number-release", who] => {
                let imsi = self.imsi_of(who).ok_or_else(|| format!("no subscriber {}", who))?;
                let u = self.db.subs.get_mut(&imsi).expect("found");
                let old = core::mem::take(&mut u.ims.msisdn);
                if old.is_empty() {
                    return Err(format!("{} has no number", imsi));
                }
                format!("{} released {}", imsi, old)
            }
            ["number-move", num, to] => {
                let from = self.db.by_number(num).map(String::from).ok_or_else(|| format!("nobody has {}", num))?;
                let to = self.imsi_of(to).ok_or_else(|| format!("no subscriber {}", to))?;
                if !self.db.subs[&to].ims.msisdn.is_empty() {
                    return Err(format!("{} already has {}", to, self.db.subs[&to].ims.msisdn));
                }
                self.db.subs.get_mut(&from).expect("found").ims.msisdn.clear();
                self.db.subs.get_mut(&to).expect("found").ims.msisdn = (*num).into();
                format!("{} moved from {} to {}", num, from, to)
            }
            ["feature", who, key, value] => {
                let imsi = self.imsi_of(who).ok_or_else(|| format!("no subscriber {}", who))?;
                let p = &mut self.db.subs.get_mut(&imsi).expect("found").ims;
                match *key {
                    "cfu" | "cfb" | "cfnr" | "cfnrc" => {
                        let v = if *value == "off" || *value == "-" { "" } else { *value };
                        if !v.is_empty() && !crate::db::valid_e164(v) {
                            return Err(format!("bad number '{}'", v));
                        }
                        let slot = match *key {
                            "cfu" => &mut p.cfu,
                            "cfb" => &mut p.cfb,
                            "cfnr" => &mut p.cfnr,
                            _ => &mut p.cfnrc,
                        };
                        *slot = v.into();
                    }
                    "cw" | "clir" | "dnd" | "vm" => {
                        let b = match *value {
                            "on" | "1" => true,
                            "off" | "0" => false,
                            _ => return Err("value must be on or off".into()),
                        };
                        match *key {
                            "cw" => p.cw = b,
                            "clir" => p.clir = b,
                            "dnd" => p.dnd = b,
                            _ => p.vm = b,
                        }
                    }
                    "block" | "unblock" => {
                        if !crate::db::valid_e164(value) {
                            return Err(format!("bad number '{}'", value));
                        }
                        p.blocked.retain(|n| n != value);
                        if *key == "block" {
                            p.blocked.push((*value).into());
                        }
                    }
                    _ => return Err(format!("unknown feature '{}' (cfu cfb cfnr cfnrc cw clir dnd vm block unblock)", key)),
                }
                format!("{} {} {}", imsi, key, value)
            }
            ["ims", rest @ ..] if !rest.is_empty() => {
                // validated by the database parser: rebuild the file with the new `ims` line
                let text: String = self.db.to_text().lines().filter(|l| !l.starts_with("ims ")).map(|l| format!("{}\n", l)).collect();
                let text = format!("{}ims {}\n", text, rest.join(" "));
                let db = Db::parse(&text).map_err(|e| e.msg)?;
                self.db.ims = db.ims;
                let i = self.db.ims.as_ref().expect("parsed");
                format!("ims pool {}/{} pcscf {} domain {}", fmt_ip(i.pool), i.prefix, fmt_ip(i.pcscf), i.domain)
            }
            ["ims-off"] => {
                self.db.ims = None;
                "ims off (phones get no IMS bearer)".to_string()
            }
            ["port-code", who, code] => {
                let imsi = self.imsi_of(who).ok_or_else(|| format!("no subscriber {}", who))?;
                if code.len() < 4 || code.len() > 16 || !code.bytes().all(|b| b.is_ascii_alphanumeric()) {
                    return Err("a port-out code is 4-16 letters and digits".into());
                }
                self.db.subs.get_mut(&imsi).expect("found").ims.port = (*code).into();
                format!("{} port-out code set", imsi)
            }
            _ => return Err(format!("unknown command '{}'", line.trim())),
        };
        self.log(format!("admin: {}", r));
        self.save(now);
        Ok(r)
    }

    // ------------------------------------------------------------ IMS (HSS + PCRF)

    /// The subscriber behind an IMSI or a number (`+1...`).
    pub fn imsi_of(&self, who: &str) -> Option<String> {
        if who.starts_with('+') {
            self.db.by_number(who).map(|s| s.into())
        } else if self.db.subs.contains_key(who) {
            Some(who.into())
        } else {
            None
        }
    }

    /// HSS: where `who` (IMSI or number) is.
    pub fn reach(&self, who: &str) -> Option<Reach> {
        let imsi = self.imsi_of(who)?;
        let sub = self.db.subs.get(&imsi)?;
        let s = self.sessions.values().find(|s| s.imsi == imsi);
        Some(Reach { imsi: imsi.clone(), msisdn: sub.ims.msisdn.clone(), ims_ip: s.and_then(|s| s.ims_ip), idle: s.is_some_and(|s| s.idle) })
    }

    /// The subscriber whose session holds IMS address `ip`.
    pub fn imsi_at(&self, ip: [u8; 4]) -> Option<&str> {
        let id = self.by_ip.get(&ip)?;
        let s = self.sessions.get(id)?;
        (s.ims_ip == Some(ip)).then_some(s.imsi.as_str())
    }

    /// The subscriber whose session holds default-bearer (internet) address `ip`.
    pub fn imsi_at_data(&self, ip: [u8; 4]) -> Option<&str> {
        let id = self.by_ip.get(&ip)?;
        let s = self.sessions.get(id)?;
        (s.ip == ip).then_some(s.imsi.as_str())
    }

    /// PCRF (Rx): a voice bearer for the phone at IMS address `ue_ip`, flow
    /// `ue_port <-> remote:remote_port`. Returns the bearer id, and whether it
    /// went to a serving site (an idle phone gets it on its next resume: no).
    pub fn bearer_setup(&mut self, ue_ip: [u8; 4], ue_port: u16, remote: [u8; 4], remote_port: u16) -> Result<(u8, bool), String> {
        let id = *self.by_ip.get(&ue_ip).ok_or("no session at that IMS address")?;
        let s = self.sessions.get_mut(&id).expect("by_ip in sync");
        if s.ims_ip != Some(ue_ip) {
            return Err("not an IMS address".into());
        }
        if let Some((b, _)) = s.bearers.iter().find(|(b, _)| b.ue_port == ue_port && b.remote == remote && b.remote_port == remote_port) {
            return Ok((b.id, true));
        }
        if s.bearers.len() >= 4 {
            return Err("too many bearers".into());
        }
        let bid = s.next_bearer;
        s.next_bearer = s.next_bearer.wrapping_add(1).max(1);
        let b = Bearer { id: bid, qci: QCI_VOICE, ue_port, remote, remote_port };
        s.bearers.push((b, false));
        let serving = s.serving.map(|v| v.conn);
        let imsi = s.imsi.clone();
        self.log(format!("{}: voice bearer {} udp {} <-> {}:{}", imsi, bid, ue_port, fmt_ip(remote), remote_port));
        if let Some(c) = serving {
            self.send(c, Ctl::BearerSetup { session: id, bearer: b });
            if let Some(h) = self.sessions.get(&id).and_then(|s| s.ho.as_ref()).map(|h| h.target.conn) {
                self.send(h, Ctl::BearerSetup { session: id, bearer: b });
            }
        }
        Ok((bid, serving.is_some()))
    }

    /// PCRF: the call ended; release its bearer.
    pub fn bearer_release(&mut self, ue_ip: [u8; 4], bid: u8) -> Result<(), String> {
        let id = *self.by_ip.get(&ue_ip).ok_or("no session at that IMS address")?;
        let s = self.sessions.get_mut(&id).expect("by_ip in sync");
        let had = s.bearers.len();
        s.bearers.retain(|(b, _)| b.id != bid);
        if s.bearers.len() == had {
            return Err(format!("no bearer {}", bid));
        }
        if let Some(c) = s.serving.map(|v| v.conn) {
            self.send(c, Ctl::BearerRelease { session: id, id: bid });
        }
        Ok(())
    }

    pub fn sites(&self) -> Vec<SiteInfo> {
        self.conns
            .iter()
            .filter_map(|(k, c)| {
                c.site.as_ref().map(|s| SiteInfo {
                    conn: *k,
                    id: s.id.clone(),
                    peer: c.peer,
                    cells: s.cells.iter().map(|x| x.cell_id).collect(),
                    since_ms: c.since_ms,
                    last_rx_ms: c.last_rx_ms,
                    rejected: c.bh.as_ref().map(|b| b.ctl.rejected + b.tun.rejected).unwrap_or(0),
                })
            })
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
                    msisdn: sub.map(|u| u.ims.msisdn.clone()).unwrap_or_default(),
                    ims_ip: s.ims_ip,
                    bearers: s.bearers.clone(),
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
