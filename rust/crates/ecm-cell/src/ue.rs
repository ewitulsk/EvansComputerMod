//! The phone (`celld`, the UE daemon), sans-IO: cell search over channels,
//! random access and attach with the SIM's answer, network authentication
//! (the security mode MAC), protected data, measurements (with gaps for other
//! channels) and reports, handover execution, radio-link-failure recovery
//! (re-attach keeps the address: it lives at the core), idle and paging,
//! detach.
//!
//! The driver owns the modem's framed radio, the SIM (the phone's SIM slot,
//! or a `card_reader`), and the `wwan0` tun interface: [`UeOut::Ip`] goes to
//! the tun, packets the kernel routes out of `wwan0` go to [`Ue::send_ip`],
//! and [`UeEvent::Attached`] says which address and gateway to configure.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::air::{decode_items, encode_items, Cause, Grant, Item, Measurement, Plain, RachCause, SysInfo};
use crate::backhaul::IpConfig;
use crate::crypto::{as_key, ct_eq, smc_mac, sr_mac, AirCipher, PROTECTED, PROTECTED_OVERHEAD};
use crate::phy::{bytes_in, PhyCmd, PhyConfig, PhyEvent, Profile, Role, RxFrame, TxReq, TxResult};
use crate::seg::{segment, Dedup, Reassembly};
use crate::simauth::{Sim, SimResponse};
use crate::Now;

#[derive(Clone, Debug)]
pub struct UeConfig {
    pub profile: Profile,
    pub address: u32,
    /// Channels to search (empty = every channel of the profile).
    pub channels: Vec<u8>,
    pub power_dbm: Option<f64>,
    /// Time on each channel while searching (at least two beacon periods).
    pub dwell_ms: u64,
    pub report_ms: u64,
    /// Report at once when a neighbour is this much stronger, dB.
    pub a3_offset_db: f64,
    /// Measure other channels (with gaps) while the serving cell is weaker than this, dBm.
    pub gap_below_dbm: f64,
    pub gap_every_ms: u64,
    pub gap_ms: u64,
    pub keepalive_ms: u64,
    /// No beacon from the serving cell for this long = radio link failure.
    pub rlf_ms: u64,
    /// Cells weaker than this are not used, dBm.
    pub min_rssi_dbm: f64,
    /// How far ahead of its own slot clock the phone schedules a transmission, µs (at least
    /// the next slot). A driver that reads the radio's clock now and then and counts forward
    /// lags it a little; frames named for a slot the radio has already passed are refused.
    pub tx_lead_us: u64,
}

impl UeConfig {
    pub fn new(profile: Profile, address: u32) -> UeConfig {
        UeConfig {
            profile,
            address,
            channels: Vec::new(),
            power_dbm: None,
            dwell_ms: 250,
            report_ms: 1000,
            a3_offset_db: 3.0,
            gap_below_dbm: -75.0,
            gap_every_ms: 2000,
            gap_ms: 160,
            keepalive_ms: 1000,
            rlf_ms: 1500,
            min_rssi_dbm: -115.0,
            tx_lead_us: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UeEvent {
    Searching,
    /// A full sweep found no usable cell of our carrier; `heard` lists the carriers heard.
    NoService { heard: Vec<String> },
    Attached { cell: u32, ip: IpConfig },
    /// Back from idle on `cell` (same address).
    Resumed { cell: u32 },
    Handover { from: u32, to: u32, channel: u8 },
    /// The network ended the session (we re-attach unless the cause is permanent).
    Released { cause: Cause },
    Rejected { cause: Cause },
    /// The network failed authentication (security mode MAC wrong): not our network.
    NetworkAuthFailed { cell: u32 },
    SimError,
    Idle,
    Detached,
}

#[derive(Debug, PartialEq)]
pub enum UeOut {
    Phy(PhyCmd),
    /// A downlink IP packet for the tun.
    Ip(Vec<u8>),
    Event(UeEvent),
    Log(String),
}

#[derive(Clone)]
struct CellSeen {
    sib: SysInfo,
    addr: u32,
    rssi: f64,
    sinr: f64,
    last_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum St {
    Off,
    Search { idx: usize, until: u64, sweep_from: u64, swept: usize },
    Backoff { until: u64 },
    Rach { cell: u32, nonce: u32, tries: u8, deadline: u64, cause: RachCause },
    Attach { cell: u32, deadline: u64 },
    Auth { cell: u32, deadline: u64 },
    Secure { cell: u32, deadline: u64, resume: bool },
    Connected { cell: u32 },
    HandingOver { from: u32, to: u32, deadline: u64, tag: u16, tries: u8 },
    Idle { cell: u32 },
    Detaching { deadline: u64 },
    Rejected,
}

/// What the phone's status bar and `cell` command show.
#[derive(Clone, Debug, PartialEq)]
pub struct UeStatus {
    pub state: &'static str,
    pub carrier: String,
    pub cell: Option<u32>,
    pub channel: u8,
    pub rssi_dbm: Option<f64>,
    pub sinr_db: Option<f64>,
    pub ip: Option<IpConfig>,
    pub last_cause: Option<Cause>,
}

pub struct Ue {
    pub cfg: UeConfig,
    st: St,
    imsi: String,
    carrier: String,
    cells: BTreeMap<u32, CellSeen>,
    barred: BTreeMap<u32, u64>,
    cur_channel: u8,
    ue_id: u16,
    grant: Option<Grant>,
    cipher: Option<AirCipher>,
    /// The source cell's cipher during a handover (make-before-break downlink).
    prev: Option<(u32, AirCipher)>,
    auth: Option<([u8; 16], SimResponse)>,
    tmsi: u32,
    sr_count: u32,
    ip: Option<IpConfig>,
    session: u32,
    ul_q: VecDeque<Vec<u8>>,
    ul_segs: VecDeque<Item>,
    ul_seq: u32,
    ctl_q: VecDeque<Item>,
    reasm: Reassembly,
    dedup: Dedup,
    /// Data frames in flight by tag (resent once if the radio refuses the slot).
    inflight: BTreeMap<u16, (u32, Vec<u8>, u8, u8)>,
    /// Plain attach messages sent on a grant, by tag: resent at the next slot if the radio
    /// refuses theirs as already past (a driver whose slot clock lags the radio's).
    plain_inflight: BTreeMap<u16, (u32, Vec<u8>, u8, u8)>,
    next_tx_slot: u64,
    last_tx_ms: u64,
    last_report_ms: u64,
    last_gap_ms: u64,
    gap: Option<(u64, u64, u8)>,
    gap_rr: usize,
    paged: bool,
    last_cause: Option<Cause>,
    rng: u64,
    next_tag: u16,
    out: Vec<UeOut>,
}

const UL_INFLIGHT: u32 = 4;
const UL_QUEUE: usize = 64;

impl Ue {
    pub fn new(cfg: UeConfig) -> Ue {
        let rng = (cfg.address as u64) << 1 | 1;
        Ue {
            cur_channel: cfg.channels.first().copied().unwrap_or(1),
            cfg,
            st: St::Off,
            imsi: String::new(),
            carrier: String::new(),
            cells: BTreeMap::new(),
            barred: BTreeMap::new(),
            ue_id: 0,
            grant: None,
            cipher: None,
            prev: None,
            auth: None,
            tmsi: 0,
            sr_count: 0,
            ip: None,
            session: 0,
            ul_q: VecDeque::new(),
            ul_segs: VecDeque::new(),
            ul_seq: 0,
            ctl_q: VecDeque::new(),
            reasm: Reassembly::default(),
            dedup: Dedup::default(),
            inflight: BTreeMap::new(),
            plain_inflight: BTreeMap::new(),
            next_tx_slot: 0,
            last_tx_ms: 0,
            last_report_ms: 0,
            last_gap_ms: 0,
            gap: None,
            gap_rr: 0,
            paged: false,
            last_cause: None,
            rng: rng ^ 0x9E37_79B9_7F4A_7C15,
            next_tag: 1,
            out: Vec::new(),
        }
    }

    pub fn take_out(&mut self) -> Vec<UeOut> {
        core::mem::take(&mut self.out)
    }

    fn ev(&mut self, e: UeEvent) {
        self.out.push(UeOut::Event(e));
    }

    fn log(&mut self, s: String) {
        self.out.push(UeOut::Log(s));
    }

    fn rand(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    fn channels(&self) -> Vec<u8> {
        if self.cfg.channels.is_empty() {
            (1..=self.cfg.profile.channels()).collect()
        } else {
            self.cfg.channels.clone()
        }
    }

    fn tune(&mut self, ch: u8) {
        self.cur_channel = ch;
        self.out.push(UeOut::Phy(PhyCmd::Configure(PhyConfig {
            profile: self.cfg.profile,
            role: Role::Ue,
            channel: ch,
            power_dbm: self.cfg.power_dbm,
            address: self.cfg.address,
        })));
    }

    /// The first slot a transmission may name: the next one, plus the configured lead.
    fn first_slot(&self, now: Now) -> u64 {
        now.slot + 1 + self.cfg.tx_lead_us / self.cfg.profile.slot_us().max(1)
    }

    fn tx(&mut self, dst: u32, payload: Vec<u8>, slot: u64, cqi: u8, sub: u8) -> u16 {
        let tag = self.next_tag;
        self.next_tag = self.next_tag.wrapping_add(1).max(1);
        self.out.push(UeOut::Phy(PhyCmd::Tx(TxReq { dst, harq: true, slot, cqi, subchannel: sub, tag, payload })));
        tag
    }

    /// See [`UeConfig::tx_lead_us`].
    pub fn set_tx_lead_us(&mut self, us: u64) {
        self.cfg.tx_lead_us = us;
    }

    /// Power on with the SIM in the slot: search for a cell of its carrier.
    pub fn start(&mut self, sim: &mut dyn Sim, now: Now) {
        match sim.identity() {
            Ok(id) => {
                self.imsi = id.imsi;
                self.carrier = id.carrier;
            }
            Err(_) => {
                self.ev(UeEvent::SimError);
                self.st = St::Off;
                return;
            }
        }
        self.auth = None;
        self.tmsi = 0;
        self.search(now);
    }

    /// Detach (or just stop when not attached).
    pub fn detach(&mut self, now: Now) {
        match self.st {
            St::Connected { .. } => {
                self.ctl_q.clear();
                self.ul_q.clear();
                self.ul_segs.clear();
                self.ctl_q.push_back(Item::Detach { cause: Cause::UserDetach });
                self.st = St::Detaching { deadline: now.ms + 1000 };
                self.pump_ul(now);
            }
            _ => {
                self.reset_session();
                self.st = St::Off;
                self.ev(UeEvent::Detached);
            }
        }
    }

    /// Queue an uplink IP packet. False when there is no session to carry it.
    pub fn send_ip(&mut self, pkt: Vec<u8>, now: Now) -> bool {
        match self.st {
            St::Connected { .. } | St::HandingOver { .. } => {}
            St::Idle { .. } => self.paged = true, // uplink data resumes the session
            _ => return false,
        }
        if self.ul_q.len() >= UL_QUEUE {
            self.ul_q.pop_front();
        }
        self.ul_q.push_back(pkt);
        if matches!(self.st, St::Connected { .. }) {
            self.pump_ul(now);
        }
        true
    }

    fn reset_session(&mut self) {
        self.cipher = None;
        self.prev = None;
        self.grant = None;
        self.ctl_q.clear();
        self.ul_segs.clear();
        self.inflight.clear();
        self.plain_inflight.clear();
        self.gap = None;
    }

    fn search(&mut self, now: Now) {
        self.reset_session();
        let chans = self.channels();
        let idx = chans.iter().position(|c| *c == self.cur_channel).unwrap_or(0);
        self.tune(chans[idx]);
        self.st = St::Search { idx, until: now.ms + self.cfg.dwell_ms, sweep_from: now.ms, swept: 0 };
        self.ev(UeEvent::Searching);
    }

    fn suitable(&self, c: &CellSeen, now: Now, since: u64) -> bool {
        c.sib.carrier == self.carrier
            && c.sib.core_up()
            && !c.sib.barred()
            && c.sib.profile == self.cfg.profile.code()
            && c.last_ms >= since
            && c.rssi >= self.cfg.min_rssi_dbm
            && self.barred.get(&c.sib.cell_id).is_none_or(|t| *t <= now.ms)
    }

    fn best_cell(&self, now: Now, since: u64) -> Option<u32> {
        self.cells
            .values()
            .filter(|c| self.suitable(c, now, since))
            .max_by(|a, b| a.rssi.partial_cmp(&b.rssi).unwrap_or(core::cmp::Ordering::Equal))
            .map(|c| c.sib.cell_id)
    }

    fn camp(&mut self, cell: u32, now: Now) {
        let ch = self.cells[&cell].sib.channel;
        if ch != self.cur_channel {
            self.tune(ch);
        }
        if self.auth.is_some() && self.tmsi != 0 && self.ip.is_some() && !self.paged && self.ul_q.is_empty() {
            // an idle phone just camps
            self.st = St::Idle { cell };
            return;
        }
        let cause = if self.auth.is_some() && self.tmsi != 0 { RachCause::Service } else { RachCause::Attach };
        self.rach(cell, 0, cause, now);
    }

    fn rach(&mut self, cell: u32, tries: u8, cause: RachCause, now: Now) {
        let nonce = self.rand() as u32;
        let (addr, sub) = {
            let c = &self.cells[&cell];
            (c.addr, c.sib.rach_sub)
        };
        let jitter = self.rand() % 20;
        let slot = self.first_slot(now) + jitter;
        self.tx(addr, Plain::RachRequest { nonce, cause }.encode(), slot, 1, sub);
        self.st = St::Rach { cell, nonce, tries, deadline: now.ms + 300, cause };
    }

    fn bar(&mut self, cell: u32, ms: u64, now: Now) {
        self.barred.insert(cell, now.ms + ms);
    }

    // ------------------------------------------------------------ radio in

    pub fn on_phy(&mut self, ev: PhyEvent, sim: &mut dyn Sim, now: Now) {
        match ev {
            PhyEvent::Status(s) => {
                if let Some((dst, payload, cqi, sub)) = self.inflight.remove(&s.tag) {
                    if s.result == TxResult::Rejected && s.attempts == 0 {
                        // a slot already past (a slow driver): next slot instead
                        let tag = self.tx(dst, payload.clone(), 0, cqi, sub);
                        self.inflight.insert(tag, (dst, payload, cqi, sub));
                        return;
                    }
                } else if let Some((dst, payload, cqi, sub)) = self.plain_inflight.remove(&s.tag) {
                    if s.result == TxResult::Rejected && s.attempts == 0 {
                        // the same for an attach message on its grant (else the attach times out)
                        let tag = self.tx(dst, payload.clone(), 0, cqi, sub);
                        self.plain_inflight.insert(tag, (dst, payload, cqi, sub));
                        return;
                    }
                }
                if let St::HandingOver { from, to, tag, .. } = self.st {
                    if s.tag == tag {
                        match s.result {
                            TxResult::Acked => self.handover_done(from, to, now),
                            TxResult::Failed => self.send_ho_confirm(now),
                            _ => {}
                        }
                    }
                }
            }
            PhyEvent::Rx(rx) => self.on_rx(rx, sim, now),
        }
    }

    fn serving_cell(&self) -> Option<u32> {
        match self.st {
            St::Rach { cell, .. } | St::Attach { cell, .. } | St::Auth { cell, .. } | St::Secure { cell, .. } | St::Connected { cell } | St::Idle { cell } => Some(cell),
            St::HandingOver { to, .. } => Some(to),
            _ => None,
        }
    }

    fn serving_addr(&self) -> Option<u32> {
        self.serving_cell().and_then(|c| self.cells.get(&c)).map(|c| c.addr)
    }

    fn on_rx(&mut self, rx: RxFrame, sim: &mut dyn Sim, now: Now) {
        if rx.payload.is_empty() {
            return;
        }
        if rx.is_beacon() || rx.dst == crate::phy::BROADCAST {
            if let Ok(m) = Plain::decode(&rx.payload) {
                match m {
                    Plain::SysInfo(sib) => self.on_sib(sib, &rx, now),
                    Plain::Paging { tmsis } => {
                        if let St::Idle { cell } = self.st {
                            if self.tmsi != 0 && tmsis.contains(&self.tmsi) && self.serving_addr() == Some(rx.src) {
                                self.paged = true;
                                self.camp(cell, now);
                            }
                        }
                    }
                    _ => {}
                }
            }
            return;
        }
        if rx.dst != self.cfg.address {
            return;
        }
        if rx.payload[0] == PROTECTED {
            let me = self.cfg.address;
            let opened = if let Some((pa, pc)) = self.prev.as_mut().filter(|(pa, _)| *pa == rx.src) {
                pc.open(*pa, me, &rx.payload).ok().map(|p| (p, true))
            } else if let Some(c) = self.cipher.as_mut() {
                c.open(rx.src, me, &rx.payload).ok().map(|p| (p, false))
            } else {
                None
            };
            let Some((plain, from_prev)) = opened else { return };
            let Ok(items) = decode_items(&plain) else { return };
            if !from_prev {
                if let St::HandingOver { from, to, .. } = self.st {
                    self.handover_done(from, to, now);
                }
            }
            for i in items {
                self.on_item(i, from_prev, now);
            }
            return;
        }
        if Some(rx.src) != self.serving_addr() {
            return;
        }
        let Ok(m) = Plain::decode(&rx.payload) else { return };
        self.on_plain(m, sim, now);
    }

    fn on_sib(&mut self, sib: SysInfo, rx: &RxFrame, now: Now) {
        let id = sib.cell_id;
        let e = self.cells.entry(id).or_insert(CellSeen { sib: sib.clone(), addr: rx.src, rssi: rx.rssi_dbm, sinr: rx.sinr_db, last_ms: now.ms });
        e.rssi = 0.6 * e.rssi + 0.4 * rx.rssi_dbm;
        e.sinr = 0.6 * e.sinr + 0.4 * rx.sinr_db;
        e.sib = sib;
        e.addr = rx.src;
        e.last_ms = now.ms;
    }

    fn on_plain(&mut self, m: Plain, sim: &mut dyn Sim, now: Now) {
        match (m, self.st) {
            (Plain::RachResponse { nonce, ue_id, grant }, St::Rach { cell, nonce: n, cause, .. }) if nonce == n => {
                self.ue_id = ue_id;
                self.grant = Some(grant);
                self.next_tx_slot = 0;
                let msg = match cause {
                    RachCause::Attach => Plain::AttachRequest { imsi: self.imsi.clone(), carrier: self.carrier.clone() },
                    RachCause::Service => {
                        self.sr_count += 1;
                        let ks_int = self.auth.as_ref().map(|(_, r)| r.ks_int()).unwrap_or_default();
                        Plain::ServiceRequest { tmsi: self.tmsi, count: self.sr_count, mac: sr_mac(&ks_int, self.tmsi, self.sr_count) }
                    }
                };
                self.send_plain_on_grant(msg, now);
                self.st = St::Attach { cell, deadline: now.ms + 3000 };
            }
            (Plain::AuthChallenge { rand }, St::Attach { cell, .. }) => match sim.respond(&rand) {
                Ok(r) => {
                    self.auth = Some((rand, r));
                    self.tmsi = 0;
                    self.send_plain_on_grant(Plain::AuthResponse { res: r.res }, now);
                    self.st = St::Auth { cell, deadline: now.ms + 3000 };
                }
                Err(_) => {
                    self.ev(UeEvent::SimError);
                    self.st = St::Rejected;
                }
            },
            (Plain::SecurityModeCommand { epoch, mac }, St::Auth { cell, .. } | St::Attach { cell, .. }) => {
                let resume = matches!(self.st, St::Attach { .. });
                let Some((rand, r)) = self.auth else { return };
                let want = smc_mac(&r.ks_int(), &rand, cell, epoch, self.cfg.address);
                if !ct_eq(&want, &mac) {
                    self.log(format!("cell {}: security mode MAC wrong, not our network", cell));
                    self.ev(UeEvent::NetworkAuthFailed { cell });
                    self.bar(cell, 60_000, now);
                    self.st = St::Backoff { until: now.ms + 2000 };
                    return;
                }
                self.cipher = Some(AirCipher::new(&as_key(&r.ks_enc(), cell, epoch), epoch, true));
                self.ctl_q.push_back(Item::SecurityModeComplete);
                self.st = St::Secure { cell, deadline: now.ms + 3000, resume };
                self.pump_ul(now);
            }
            (Plain::AttachReject { cause }, St::Rach { cell, .. } | St::Attach { cell, .. } | St::Auth { cell, .. } | St::Secure { cell, .. }) => {
                self.reject(cell, cause, now);
            }
            _ => {}
        }
    }

    fn reject(&mut self, cell: u32, cause: Cause, now: Now) {
        self.reset_session();
        self.last_cause = Some(cause);
        if cause == Cause::SessionLost {
            // the core forgot our idle session: attach afresh, same SIM
            self.auth = None;
            self.tmsi = 0;
            self.paged = false;
            self.rach(cell, 0, RachCause::Attach, now);
            return;
        }
        self.ev(UeEvent::Rejected { cause });
        self.log(format!("attach rejected at cell {}: {}", cell, cause));
        if cause.is_permanent() {
            self.st = St::Rejected;
            return;
        }
        let wait = match cause {
            Cause::CoreUnavailable => {
                self.bar(cell, 3000, now);
                500
            }
            Cause::QuotaExhausted => 60_000,
            _ => 5000,
        };
        self.st = St::Backoff { until: now.ms + wait };
    }

    fn send_plain_on_grant(&mut self, m: Plain, now: Now) {
        let (Some(addr), Some(g)) = (self.serving_addr(), self.grant) else { return };
        let slot = g.next_run(self.first_slot(now)).map(|(s, _)| s).unwrap_or(0);
        let payload = m.encode();
        let tag = self.tx(addr, payload.clone(), slot, 1, g.subchannel);
        if self.plain_inflight.len() >= 8 {
            self.plain_inflight.pop_first();
        }
        self.plain_inflight.insert(tag, (addr, payload, 1, g.subchannel));
    }

    fn on_item(&mut self, i: Item, from_prev: bool, now: Now) {
        match i {
            Item::AttachAccept { ip, prefix, gateway, dns, mtu, tmsi, session } => {
                if let St::Secure { cell, resume, .. } = self.st {
                    if session != self.session || !resume {
                        // a new session: its downlink sequence numbers start over
                        self.session = session;
                        self.dedup = Dedup::default();
                        self.reasm = Reassembly::default();
                    }
                    let cfg = IpConfig { ip, prefix, gateway, dns, mtu };
                    let same = self.ip == Some(cfg);
                    self.ip = Some(cfg);
                    self.tmsi = tmsi;
                    self.paged = false;
                    self.st = St::Connected { cell };
                    self.ctl_q.push_back(Item::AttachComplete);
                    self.last_report_ms = now.ms;
                    self.last_gap_ms = now.ms;
                    if resume && same {
                        self.ev(UeEvent::Resumed { cell });
                    } else {
                        self.ev(UeEvent::Attached { cell, ip: cfg });
                    }
                    self.pump_ul(now);
                }
            }
            Item::Grant(g) => {
                if !from_prev {
                    self.grant = Some(g);
                }
            }
            Item::Data { seq, idx, count, bytes } => {
                if let Some(pkt) = self.reasm.push(seq, idx, count, &bytes, now.ms) {
                    if self.dedup.fresh(seq) {
                        self.out.push(UeOut::Ip(pkt));
                    }
                }
            }
            Item::Release { cause } => {
                if from_prev {
                    return;
                }
                self.last_cause = Some(cause);
                let cell = self.serving_cell();
                self.reset_session();
                self.ev(UeEvent::Released { cause });
                if cause == Cause::Inactivity {
                    if let Some(c) = cell {
                        self.st = St::Idle { cell: c };
                        self.ev(UeEvent::Idle);
                        return;
                    }
                }
                self.auth = None;
                self.tmsi = 0;
                if cause.is_permanent() {
                    self.st = St::Rejected;
                } else if cause == Cause::QuotaExhausted {
                    self.st = St::Backoff { until: now.ms + 60_000 };
                } else {
                    if cause == Cause::CoreUnavailable {
                        if let Some(c) = cell {
                            self.bar(c, 2000, now);
                        }
                    }
                    self.search(now);
                }
            }
            Item::HandoverCommand { cell_id, addr, profile, channel, epoch, ue_id, grant } => {
                let St::Connected { cell: from } = self.st else { return };
                let Some((_, r)) = self.auth else { return };
                if Profile::from_code(profile) != Some(self.cfg.profile) {
                    return;
                }
                let old = self.cipher.take();
                if let (Some(old), Some(a)) = (old, self.serving_addr()) {
                    self.prev = Some((a, old));
                }
                self.cipher = Some(AirCipher::new(&as_key(&r.ks_enc(), cell_id, epoch), epoch, true));
                // the target's address may be new to us (another channel): make sure it is known
                let known = self.cells.get(&cell_id).map(|c| c.addr);
                if known != Some(addr) {
                    let Some(mut sib) = self.cells.get(&from).map(|c| c.sib.clone()) else { return };
                    sib.cell_id = cell_id;
                    sib.channel = channel;
                    self.cells.insert(cell_id, CellSeen { sib, addr, rssi: -200.0, sinr: 0.0, last_ms: now.ms });
                }
                if channel != self.cur_channel {
                    self.tune(channel);
                }
                self.gap = None;
                self.ue_id = ue_id;
                self.grant = Some(grant);
                self.next_tx_slot = 0;
                self.inflight.clear();
                self.st = St::HandingOver { from, to: cell_id, deadline: now.ms + 1500, tag: 0, tries: 0 };
                self.send_ho_confirm(now);
            }
            Item::DetachAccept => {
                if let St::Detaching { .. } = self.st {
                    self.finish_detach();
                }
            }
            _ => {}
        }
    }

    fn finish_detach(&mut self) {
        self.reset_session();
        self.auth = None;
        self.tmsi = 0;
        self.st = St::Off;
        self.ev(UeEvent::Detached);
    }

    fn send_ho_confirm(&mut self, now: Now) {
        let St::HandingOver { from, to, deadline, tries, .. } = self.st else { return };
        if tries >= 4 {
            return;
        }
        let (Some(c), Some(g)) = (self.cipher.as_mut(), self.grant) else { return };
        let Some(addr) = self.cells.get(&to).map(|c| c.addr) else { return };
        let items = encode_items(&[Item::HandoverConfirm, Item::BufferStatus { bytes: self.ul_q.iter().map(|p| p.len() as u32).sum() }]);
        let frame = c.seal(self.cfg.address, addr, &items);
        let slot = g.next_run(self.first_slot(now)).map(|(s, _)| s).unwrap_or(0);
        let tag = self.tx(addr, frame, slot, g.cqi, g.subchannel);
        self.st = St::HandingOver { from, to, deadline, tag, tries: tries + 1 };
    }

    fn handover_done(&mut self, from: u32, to: u32, now: Now) {
        self.st = St::Connected { cell: to };
        self.last_report_ms = now.ms;
        self.last_gap_ms = now.ms;
        let ch = self.cells.get(&to).map(|c| c.sib.channel).unwrap_or(self.cur_channel);
        if let Some(c) = self.cells.get_mut(&to) {
            c.last_ms = now.ms; // the target's beacons count from here
        }
        self.ev(UeEvent::Handover { from, to, channel: ch });
        self.pump_ul(now);
    }

    // ------------------------------------------------------------ uplink

    fn pump_ul(&mut self, now: Now) {
        let (cell, ok) = match self.st {
            St::Connected { cell } | St::Secure { cell, .. } => (cell, true),
            St::Detaching { .. } => (self.serving_cell().unwrap_or(0), true),
            _ => (0, false),
        };
        if !ok {
            return;
        }
        let _ = cell;
        let Some(addr) = self.serving_addr().or_else(|| self.cells.values().find(|c| c.last_ms + 2000 > now.ms).map(|c| c.addr)) else { return };
        let Some(g) = self.grant else { return };
        let slot_us = self.cfg.profile.slot_us();
        let data_ok = matches!(self.st, St::Connected { .. });
        while (self.inflight.len() as u32) < UL_INFLIGHT {
            let start = self.first_slot(now).max(self.next_tx_slot);
            if let Some((gf, gt, _)) = self.gap {
                if start + 4 >= gf && start < gt {
                    return;
                }
            }
            let Some((slot, run)) = g.next_run(start) else { return };
            let airtime = if g.period == 1 && g.len == 1 { 4000.min(1000 * slot_us) } else { run * slot_us };
            // the frame (radio header and protection included) must end inside the run
            let cap = bytes_in(self.cfg.profile, g.cqi, g.subchannel, airtime.saturating_sub(slot_us / 10)).saturating_sub(PROTECTED_OVERHEAD + 16);
            let budget = cap.clamp(24, crate::site::FRAME_BUDGET);
            let mut items = Vec::new();
            let mut size = 0;
            while let Some(i) = self.ctl_q.front() {
                let n = i.encoded_len();
                if size + n > budget && !items.is_empty() {
                    break;
                }
                size += n;
                items.push(self.ctl_q.pop_front().expect("front"));
            }
            if data_ok {
                loop {
                    if self.ul_segs.is_empty() {
                        let Some(pkt) = self.ul_q.pop_front() else { break };
                        self.ul_seq = self.ul_seq.wrapping_add(1).max(1);
                        self.ul_segs.extend(segment(self.ul_seq, &pkt, budget.max(64)));
                    }
                    let n = self.ul_segs.front().map(|i| i.encoded_len()).unwrap_or(0);
                    if size + n > budget {
                        break;
                    }
                    size += n;
                    items.push(self.ul_segs.pop_front().expect("front"));
                }
                let waiting: usize = self.ul_q.iter().map(|p| p.len()).sum::<usize>() + self.ul_segs.iter().map(|i| i.encoded_len()).sum::<usize>();
                if waiting > 0 {
                    items.push(Item::BufferStatus { bytes: waiting as u32 });
                }
            }
            if items.is_empty() {
                return;
            }
            let plain = encode_items(&items);
            let me = self.cfg.address;
            let Some(c) = self.cipher.as_mut() else { return };
            let frame = c.seal(me, addr, &plain);
            let used = crate::phy::airtime_us(self.cfg.profile, g.cqi, g.subchannel, frame.len()).div_ceil(slot_us).max(1);
            let tag = self.tx(addr, frame.clone(), slot, g.cqi, g.subchannel);
            self.inflight.insert(tag, (addr, frame, g.cqi, g.subchannel));
            self.next_tx_slot = slot + used;
            self.last_tx_ms = now.ms;
        }
    }

    // ------------------------------------------------------------ time

    fn measurement_report(&self, now: Now) -> Option<Item> {
        let serving = self.serving_cell()?;
        let mut cells: Vec<Measurement> = Vec::new();
        let m = |c: &CellSeen| Measurement {
            cell_id: c.sib.cell_id,
            profile: c.sib.profile,
            channel: c.sib.channel,
            rssi_x10: (c.rssi * 10.0).clamp(-32000.0, 32000.0) as i16,
            sinr_x10: (c.sinr * 10.0).clamp(-32000.0, 32000.0) as i16,
            ul_mask: c.sib.ul_mask,
        };
        cells.push(m(self.cells.get(&serving)?));
        for c in self.cells.values() {
            if c.sib.cell_id != serving && c.sib.carrier == self.carrier && now.ms.saturating_sub(c.last_ms) < 2500 {
                cells.push(m(c));
            }
        }
        Some(Item::MeasurementReport { cells })
    }

    pub fn tick(&mut self, now: Now) {
        self.cells.retain(|_, c| now.ms.saturating_sub(c.last_ms) < 10_000);
        self.barred.retain(|_, t| *t > now.ms);
        match self.st {
            St::Off | St::Rejected => {}
            St::Search { idx, until, sweep_from, swept } => {
                if now.ms < until {
                    return;
                }
                if let Some(c) = self.best_cell(now, sweep_from) {
                    self.camp(c, now);
                    return;
                }
                let chans = self.channels();
                let swept = swept + 1;
                if swept >= chans.len() {
                    let mut heard: Vec<String> = self.cells.values().filter(|c| c.last_ms >= sweep_from).map(|c| c.sib.carrier.clone()).collect();
                    heard.sort();
                    heard.dedup();
                    self.ev(UeEvent::NoService { heard });
                    let ni = (idx + 1) % chans.len();
                    self.tune(chans[ni]);
                    self.st = St::Search { idx: ni, until: now.ms + self.cfg.dwell_ms, sweep_from: now.ms, swept: 0 };
                    return;
                }
                let ni = (idx + 1) % chans.len();
                self.tune(chans[ni]);
                self.st = St::Search { idx: ni, until: now.ms + self.cfg.dwell_ms, sweep_from, swept };
            }
            St::Backoff { until } => {
                if now.ms >= until {
                    self.search(now);
                }
            }
            St::Rach { cell, tries, deadline, cause, .. } => {
                if now.ms >= deadline {
                    if tries >= 5 || !self.cells.contains_key(&cell) {
                        self.bar(cell, 5000, now);
                        self.search(now);
                    } else {
                        self.rach(cell, tries + 1, cause, now);
                    }
                }
            }
            St::Attach { deadline, .. } | St::Auth { deadline, .. } | St::Secure { deadline, .. } => {
                if now.ms >= deadline {
                    self.log("attach timed out".into());
                    self.search(now);
                } else {
                    self.pump_ul(now);
                }
            }
            St::Connected { cell } => {
                let last = self.cells.get(&cell).map(|c| c.last_ms).unwrap_or(0);
                let in_gap = self.gap.is_some();
                if !in_gap && now.ms.saturating_sub(last) > self.cfg.rlf_ms {
                    self.log(format!("radio link to cell {} lost", cell));
                    self.reset_session();
                    self.ev(UeEvent::Released { cause: Cause::RadioLost });
                    self.last_cause = Some(Cause::RadioLost);
                    self.auth = None;
                    self.tmsi = 0;
                    self.bar(cell, 3000, now);
                    self.search(now);
                    return;
                }
                self.gaps(cell, now);
                // measurement reports: periodic, and at once on a stronger neighbour
                let serving_rssi = self.cells.get(&cell).map(|c| c.rssi).unwrap_or(-200.0);
                let a3 = self
                    .cells
                    .values()
                    .any(|c| c.sib.cell_id != cell && c.sib.carrier == self.carrier && now.ms.saturating_sub(c.last_ms) < 1000 && c.rssi > serving_rssi + self.cfg.a3_offset_db);
                let due = now.ms.saturating_sub(self.last_report_ms) >= if a3 { 300 } else { self.cfg.report_ms };
                if due {
                    if let Some(r) = self.measurement_report(now) {
                        self.ctl_q.retain(|i| !matches!(i, Item::MeasurementReport { .. }));
                        self.ctl_q.push_back(r);
                        self.last_report_ms = now.ms;
                    }
                }
                if now.ms.saturating_sub(self.last_tx_ms) >= self.cfg.keepalive_ms && self.ctl_q.is_empty() && self.ul_q.is_empty() {
                    self.ctl_q.push_back(Item::Keepalive);
                }
                self.pump_ul(now);
            }
            St::HandingOver { deadline, .. } => {
                if now.ms >= deadline {
                    self.log("handover failed".into());
                    self.reset_session();
                    self.ev(UeEvent::Released { cause: Cause::HandoverFailed });
                    self.auth = None;
                    self.tmsi = 0;
                    self.search(now);
                }
            }
            St::Idle { cell } => {
                let last = self.cells.get(&cell).map(|c| c.last_ms).unwrap_or(0);
                if now.ms.saturating_sub(last) > self.cfg.rlf_ms {
                    // reselect: find another cell, stay idle there
                    self.search_keep_idle(now);
                    return;
                }
                if self.paged || !self.ul_q.is_empty() {
                    self.paged = true;
                    self.rach(cell, 0, RachCause::Service, now);
                }
            }
            St::Detaching { deadline } => {
                if now.ms >= deadline {
                    self.finish_detach();
                } else {
                    self.pump_ul(now);
                }
            }
        }
    }

    fn search_keep_idle(&mut self, now: Now) {
        let chans = self.channels();
        let idx = chans.iter().position(|c| *c == self.cur_channel).unwrap_or(0);
        self.cipher = None;
        self.grant = None;
        self.st = St::Search { idx, until: now.ms + self.cfg.dwell_ms, sweep_from: now.ms, swept: 0 };
    }

    /// Inter-frequency measurement: while the serving cell is weak, visit
    /// neighbour channels (from its system information) in announced gaps.
    fn gaps(&mut self, cell: u32, now: Now) {
        let slot_ms = 1000 / self.cfg.profile.slot_us().max(1);
        let spm = slot_ms.max(1); // slots per ms
        if let Some((from, to, ch)) = self.gap {
            if now.slot >= to {
                let back = self.cells.get(&cell).map(|c| c.sib.channel).unwrap_or(self.cur_channel);
                if self.cur_channel != back {
                    self.tune(back);
                }
                self.gap = None;
            } else if now.slot >= from && self.cur_channel != ch {
                self.tune(ch);
            }
            return;
        }
        let Some(serving) = self.cells.get(&cell) else { return };
        if serving.rssi >= self.cfg.gap_below_dbm || now.ms.saturating_sub(self.last_gap_ms) < self.cfg.gap_every_ms {
            return;
        }
        let mut chans: Vec<u8> = serving
            .sib
            .neighbours
            .iter()
            .filter(|n| n.profile == self.cfg.profile.code() && n.channel != serving.sib.channel)
            .map(|n| n.channel)
            .collect();
        chans.sort();
        chans.dedup();
        if chans.is_empty() {
            return;
        }
        let ch = chans[self.gap_rr % chans.len()];
        self.gap_rr += 1;
        self.last_gap_ms = now.ms;
        let from = now.slot + 30 * spm;
        let to = from + self.cfg.gap_ms * spm;
        self.gap = Some((from, to, ch));
        self.ctl_q.push_front(Item::MeasGap { from_slot: from, to_slot: to });
    }

    pub fn status(&self) -> UeStatus {
        let cell = self.serving_cell();
        let seen = cell.and_then(|c| self.cells.get(&c));
        UeStatus {
            state: match self.st {
                St::Off => "off",
                St::Search { .. } => "searching",
                St::Backoff { .. } => "waiting",
                St::Rach { .. } | St::Attach { .. } | St::Auth { .. } | St::Secure { .. } => "attaching",
                St::Connected { .. } => "connected",
                St::HandingOver { .. } => "handover",
                St::Idle { .. } => "idle",
                St::Detaching { .. } => "detaching",
                St::Rejected => "rejected",
            },
            carrier: self.carrier.clone(),
            cell,
            channel: self.cur_channel,
            rssi_dbm: seen.map(|c| c.rssi),
            sinr_db: seen.map(|c| c.sinr),
            ip: self.ip,
            last_cause: self.last_cause,
        }
    }
}
