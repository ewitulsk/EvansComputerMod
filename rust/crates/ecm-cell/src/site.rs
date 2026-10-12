//! The cell site (`celld-site`), sans-IO: system information beacons, random
//! access, attach relay to the core, air security, uplink scheduling,
//! measurement-driven handover (source and target side), the user-plane relay
//! between the air and the core's UDP tunnel, idle release and paging.
//!
//! The driver owns the framed radio (an Advanced SDR in framed mode), one TCP
//! connection to the core (it connects while [`Site::wants_core`] says so and
//! reports [`Site::core_connected`] / [`Site::core_disconnected`]) and a UDP
//! socket bound to `tunnel_port` that sends to the core's tunnel port.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::air::{self, decode_items, encode_items, Cause, Grant, Item, Measurement, Neighbour, Plain, SysInfo};
use crate::backhaul::{tunnel_split, CellInfo, Ctl, IpConfig, TunnelJoin, TunnelPacket};
use crate::codec::fmt_ip;
use crate::crypto::{site_proof, AirCipher, PROTECTED, PROTECTED_OVERHEAD};
use crate::phy::{PhyCmd, PhyConfig, PhyEvent, Profile, Role, RxFrame, TxReq, TxResult, BROADCAST};
use crate::sched::{plan, sub_preference, SchedUe};
use crate::seg::{segment, Reassembly};
use crate::Now;

/// Scheduler / housekeeping batch period.
pub const SCHED_PERIOD_MS: u64 = 100;
/// Plaintext budget of one downlink frame.
pub const FRAME_BUDGET: usize = crate::phy::MAX_PAYLOAD - PROTECTED_OVERHEAD - 16;
/// HARQ frames in flight per phone.
const DL_INFLIGHT: u32 = 4;
/// Consecutive failed downlink frames that count as a lost phone.
const DL_FAIL_LIMIT: u32 = 8;
const DL_QUEUE: usize = 128;

#[derive(Clone, Debug)]
pub struct SiteConfig {
    pub site_id: String,
    pub secret: [u8; 16],
    pub carrier: String,
    pub cell_id: u32,
    pub area: u16,
    pub profile: Profile,
    pub channel: u8,
    /// The radio's frame address.
    pub address: u32,
    pub power_dbm: Option<f64>,
    pub tunnel_port: u16,
    pub beacon_period_ms: u32,
    /// Configured neighbours (more are learned from phones' reports).
    pub neighbours: Vec<Neighbour>,
    /// Release a phone to idle after this long without data (0 = never).
    pub idle_after_ms: u64,
    /// Hand over when a neighbour is this much stronger, dB.
    pub ho_hysteresis_db: f64,
    pub rach_sub: u8,
    /// A phone silent this long is lost.
    pub ue_timeout_ms: u64,
}

impl SiteConfig {
    pub fn new(site_id: &str, secret: [u8; 16], carrier: &str, cell_id: u32, address: u32) -> SiteConfig {
        SiteConfig {
            site_id: site_id.into(),
            secret,
            carrier: carrier.into(),
            cell_id,
            area: 1,
            profile: Profile::Lte,
            channel: 1,
            address,
            power_dbm: None,
            tunnel_port: crate::backhaul::TUNNEL_PORT,
            beacon_period_ms: 100,
            neighbours: Vec::new(),
            idle_after_ms: 0,
            ho_hysteresis_db: 3.0,
            rach_sub: 4,
            ue_timeout_ms: 5000,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum SiteOut {
    Phy(PhyCmd),
    /// Send to the core over the control connection.
    Ctl(Ctl),
    /// Close the control connection (the core stopped answering).
    CloseCore,
    /// A datagram for the core's tunnel port.
    Tunnel(Vec<u8>),
    Log(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    Down,
    Hello,
    Registering,
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum St {
    Rach,
    AttachWait,
    AuthWait,
    CoreAuth,
    Smc,
    Accepting,
    Connected,
    HoIn(u32),
    HoOut,
}

impl St {
    fn name(self) -> &'static str {
        match self {
            St::Rach => "random-access",
            St::AttachWait => "attach",
            St::AuthWait | St::CoreAuth => "authenticating",
            St::Smc => "security",
            St::Accepting => "accepting",
            St::Connected => "connected",
            St::HoIn(_) => "handover-in",
            St::HoOut => "handover-out",
        }
    }
    fn secured(self) -> bool {
        matches!(self, St::Accepting | St::Connected | St::HoIn(_) | St::HoOut)
    }
}

struct UeCtx {
    addr: u32,
    ue_id: u16,
    st: St,
    imsi: String,
    session: u32,
    ip: Option<IpConfig>,
    tmsi: u32,
    cipher: Option<AirCipher>,
    grant: Option<Grant>,
    cqi_ul: u8,
    bsr: u32,
    ctl_q: VecDeque<Item>,
    dl_q: VecDeque<(u32, Vec<u8>)>,
    dl_segs: VecDeque<Item>,
    reasm: Reassembly,
    ul_seq: u32,
    gap: Option<(u64, u64)>,
    inflight: u32,
    dl_fail: u32,
    last_rx_ms: u64,
    last_activity_ms: u64,
    ho_pending_ms: Option<u64>,
    rach_nonce: u32,
    meas: Vec<Measurement>,
    release_after_send: bool,
    rssi_dbm: f64,
    used: u64,
}

/// One phone as the Cell Site app shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct UeLine {
    pub addr: u32,
    pub imsi: String,
    pub state: &'static str,
    pub ip: Option<[u8; 4]>,
    pub rssi_dbm: f64,
    pub cqi_ul: u8,
    pub subchannel: u8,
    pub used: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SiteStatus {
    pub core: &'static str,
    pub ues: Vec<UeLine>,
    pub neighbours: Vec<Neighbour>,
    pub ul_mask: u8,
}

pub struct Site {
    pub cfg: SiteConfig,
    link: Link,
    reject_until_ms: u64,
    ues: BTreeMap<u32, UeCtx>,
    next_ue_id: u16,
    revoked: BTreeSet<String>,
    learned: BTreeMap<u32, (Neighbour, u64)>,
    sib: Vec<u8>,
    ul_mask: u8,
    last_sched_ms: u64,
    last_hb_tx_ms: u64,
    last_core_rx_ms: u64,
    hb_ms: u64,
    hb_seq: u32,
    pages: Vec<(u32, u64)>,
    last_page_ms: u64,
    next_tag: u16,
    tags: BTreeMap<u16, u32>,
    join: TunnelJoin,
    out: Vec<SiteOut>,
}

impl Site {
    pub fn new(cfg: SiteConfig) -> Site {
        Site {
            cfg,
            link: Link::Down,
            reject_until_ms: 0,
            ues: BTreeMap::new(),
            next_ue_id: 1,
            revoked: BTreeSet::new(),
            learned: BTreeMap::new(),
            sib: Vec::new(),
            ul_mask: 0,
            last_sched_ms: 0,
            last_hb_tx_ms: 0,
            last_core_rx_ms: 0,
            hb_ms: 1000,
            hb_seq: 0,
            pages: Vec::new(),
            last_page_ms: 0,
            next_tag: 1,
            tags: BTreeMap::new(),
            join: TunnelJoin::default(),
            out: Vec::new(),
        }
    }

    pub fn take_out(&mut self) -> Vec<SiteOut> {
        core::mem::take(&mut self.out)
    }

    fn log(&mut self, s: String) {
        self.out.push(SiteOut::Log(s));
    }

    /// Configure the radio and start beaconing.
    pub fn start(&mut self, now: Now) {
        let c = &self.cfg;
        self.out.push(SiteOut::Phy(PhyCmd::Configure(PhyConfig {
            profile: c.profile,
            role: Role::Bs,
            channel: c.channel,
            power_dbm: c.power_dbm,
            address: c.address,
        })));
        let line = format!("cell {} on {} channel {} (carrier {})", c.cell_id, c.profile.name(), c.channel, c.carrier);
        self.sib.clear();
        self.update_sib(now);
        self.log(line);
    }

    /// Stop beaconing and release every phone (the program is exiting).
    pub fn stop(&mut self) {
        let addrs: Vec<u32> = self.ues.keys().copied().collect();
        for a in addrs {
            self.release_ue(a, Cause::SiteShutdown, true);
        }
        self.out.push(SiteOut::Phy(PhyCmd::Beacon { index: 0, period_ms: 0, payload: Vec::new() }));
    }

    /// Should the driver open a connection to the core now?
    pub fn wants_core(&self, now: Now) -> bool {
        self.link == Link::Down && now.ms >= self.reject_until_ms
    }

    pub fn core_up(&self) -> bool {
        self.link == Link::Up
    }

    pub fn core_connected(&mut self, now: Now) {
        self.link = Link::Hello;
        self.last_core_rx_ms = now.ms;
    }

    pub fn core_disconnected(&mut self, now: Now) {
        if self.link == Link::Down {
            return;
        }
        self.link = Link::Down;
        self.log("core link down".into());
        // Without the core no session works: release everyone so they find a
        // cell whose core is reachable (or wait for this one).
        let addrs: Vec<u32> = self.ues.keys().copied().collect();
        for a in addrs {
            self.release_ue(a, Cause::CoreUnavailable, false);
        }
        self.update_sib(now);
    }

    fn ctl(&mut self, m: Ctl) {
        if self.link != Link::Down {
            self.out.push(SiteOut::Ctl(m));
        }
    }

    // ------------------------------------------------------------ system information

    fn neighbours(&self) -> Vec<Neighbour> {
        let mut v = self.cfg.neighbours.clone();
        for (n, _) in self.learned.values() {
            if let Some(e) = v.iter_mut().find(|x| x.cell_id == n.cell_id) {
                e.ul_mask = n.ul_mask;
            } else {
                v.push(n.clone());
            }
        }
        v.retain(|n| n.cell_id != self.cfg.cell_id);
        v.truncate(16);
        v
    }

    fn update_sib(&mut self, _now: Now) {
        let c = &self.cfg;
        let sib = Plain::SysInfo(SysInfo {
            version: air::VERSION,
            carrier: c.carrier.clone(),
            cell_id: c.cell_id,
            area: c.area,
            flags: if self.link == Link::Up { air::SIB_CORE_UP } else { 0 },
            profile: c.profile.code(),
            channel: c.channel,
            tx_power_dbm: c.power_dbm.unwrap_or(c.profile.default_power_dbm(Role::Bs)).clamp(-128.0, 127.0) as i8,
            ul_mask: self.ul_mask,
            rach_sub: c.rach_sub,
            neighbours: self.neighbours(),
        })
        .encode();
        if sib != self.sib {
            self.sib = sib.clone();
            self.out.push(SiteOut::Phy(PhyCmd::Beacon { index: 0, period_ms: self.cfg.beacon_period_ms, payload: sib }));
        }
    }

    // ------------------------------------------------------------ radio out

    fn tx(&mut self, dst: u32, payload: Vec<u8>) {
        let tag = self.next_tag;
        self.next_tag = self.next_tag.wrapping_add(1).max(1);
        self.tags.insert(tag, dst);
        if let Some(u) = self.ues.get_mut(&dst) {
            u.inflight += 1;
        }
        self.out.push(SiteOut::Phy(PhyCmd::Tx(TxReq { dst, harq: dst != BROADCAST, slot: 0, cqi: 0, subchannel: 0, tag, payload })));
    }

    fn tx_plain(&mut self, dst: u32, m: Plain) {
        self.tx(dst, m.encode());
    }

    /// Send queued control items and data to every phone that can take them.
    fn pump_dl(&mut self, now: Now) {
        let addrs: Vec<u32> = self.ues.keys().copied().collect();
        for a in addrs {
            while let Some(u) = self.ues.get_mut(&a) {
                if u.inflight >= DL_INFLIGHT || u.cipher.is_none() {
                    break;
                }
                if let Some((from, to)) = u.gap {
                    if now.slot + 2 >= from && now.slot < to {
                        break; // the phone is listening elsewhere
                    }
                    if now.slot >= to {
                        u.gap = None;
                    }
                }
                let data_ok = matches!(u.st, St::Connected | St::HoOut);
                let mut items: Vec<Item> = Vec::new();
                let mut size = 0;
                while let Some(i) = u.ctl_q.front() {
                    let n = i.encoded_len();
                    if size + n > FRAME_BUDGET && !items.is_empty() {
                        break;
                    }
                    size += n;
                    items.push(u.ctl_q.pop_front().expect("front"));
                }
                if data_ok {
                    loop {
                        if u.dl_segs.is_empty() {
                            let Some((seq, pkt)) = u.dl_q.pop_front() else { break };
                            u.dl_segs.extend(segment(seq, &pkt, FRAME_BUDGET));
                            u.last_activity_ms = now.ms;
                        }
                        let n = u.dl_segs.front().map(|i| i.encoded_len()).unwrap_or(0);
                        if size + n > FRAME_BUDGET {
                            break;
                        }
                        size += n;
                        items.push(u.dl_segs.pop_front().expect("front"));
                    }
                }
                if items.is_empty() {
                    break;
                }
                let plain = encode_items(&items);
                let me = self.cfg.address;
                let frame = u.cipher.as_mut().expect("checked").seal(me, a, &plain);
                self.tx(a, frame);
            }
        }
    }

    /// End a phone's context here; `tell_ue` sends it a Release first.
    fn release_ue(&mut self, a: u32, cause: Cause, tell_core: bool) {
        let Some(u) = self.ues.get_mut(&a) else { return };
        let session = u.session;
        let secured = u.cipher.is_some() && u.st.secured();
        if tell_core && session != 0 {
            self.ctl(Ctl::UeRelease { session, cause });
        }
        if secured && cause != Cause::HandoverDone {
            let u = self.ues.get_mut(&a).expect("present");
            u.ctl_q.clear();
            u.dl_q.clear();
            u.dl_segs.clear();
            u.ctl_q.push_back(Item::Release { cause });
            u.release_after_send = true;
            u.st = St::Connected;
            u.session = 0;
        } else {
            self.ues.remove(&a);
        }
    }

    // ------------------------------------------------------------ inputs

    pub fn on_phy(&mut self, ev: PhyEvent, now: Now) {
        match ev {
            PhyEvent::Status(s) => {
                let Some(a) = self.tags.remove(&s.tag) else { return };
                let Some(u) = self.ues.get_mut(&a) else { return };
                u.inflight = u.inflight.saturating_sub(1);
                match s.result {
                    TxResult::Acked => u.dl_fail = 0,
                    TxResult::Failed => u.dl_fail += 1,
                    _ => {}
                }
                if u.release_after_send && u.inflight == 0 && u.ctl_q.is_empty() {
                    self.ues.remove(&a);
                } else if u.dl_fail >= DL_FAIL_LIMIT && u.st != St::HoOut {
                    let imsi = u.imsi.clone();
                    self.log(format!("{} lost (downlink failures)", imsi));
                    self.release_ue(a, Cause::RadioLost, true);
                    self.ues.remove(&a);
                }
            }
            PhyEvent::Rx(rx) => self.on_rx(rx, now),
        }
    }

    fn on_rx(&mut self, rx: RxFrame, now: Now) {
        if rx.dst != self.cfg.address || rx.payload.is_empty() {
            return;
        }
        if let Some(u) = self.ues.get_mut(&rx.src) {
            u.last_rx_ms = now.ms;
            u.cqi_ul = rx.cqi.max(1);
            u.rssi_dbm = rx.rssi_dbm;
        }
        if rx.payload[0] == PROTECTED {
            let me = self.cfg.address;
            let Some(u) = self.ues.get_mut(&rx.src) else { return };
            let Some(c) = u.cipher.as_mut() else { return };
            let Ok(plain) = c.open(rx.src, me, &rx.payload) else { return };
            let Ok(items) = decode_items(&plain) else { return };
            for i in items {
                self.on_item(rx.src, i, now);
            }
            return;
        }
        let Ok(m) = Plain::decode(&rx.payload) else { return };
        self.on_plain(rx.src, rx.cqi, m, now);
    }

    fn on_plain(&mut self, src: u32, cqi: u8, m: Plain, now: Now) {
        match m {
            Plain::RachRequest { nonce, cause: _ } => {
                if self.link != Link::Up {
                    self.tx_plain(src, Plain::AttachReject { cause: Cause::CoreUnavailable });
                    return;
                }
                if let Some(u) = self.ues.get(&src) {
                    if u.st == St::Rach && u.rach_nonce == nonce {
                        let (ue_id, g) = (u.ue_id, u.grant.expect("rach grant"));
                        self.tx_plain(src, Plain::RachResponse { nonce, ue_id, grant: g });
                        return;
                    }
                    // The phone starts over: its old context here is gone.
                    self.release_ue(src, Cause::RadioLost, true);
                    self.ues.remove(&src);
                }
                let ue_id = self.next_ue_id;
                self.next_ue_id = self.next_ue_id.wrapping_add(1).max(1);
                let spt = 1000 / (self.cfg.profile.slot_us().max(1)) * 1000;
                let g = Grant { from_slot: now.slot, to_slot: now.slot + 10 * spt, period: 1, offset: 0, len: 1, subchannel: self.cfg.rach_sub, cqi: 1 };
                self.ues.insert(src, UeCtx::new(src, ue_id, St::Rach, nonce, cqi, now.ms, Some(g)));
                self.tx_plain(src, Plain::RachResponse { nonce, ue_id, grant: g });
            }
            Plain::AttachRequest { imsi, carrier } => {
                if self.ues.get(&src).map(|u| u.st) != Some(St::Rach) {
                    return;
                }
                let cause = if carrier != self.cfg.carrier {
                    Some(Cause::WrongCarrier)
                } else if self.revoked.contains(&imsi) {
                    Some(Cause::Revoked)
                } else {
                    None
                };
                if let Some(c) = cause {
                    self.log(format!("attach {} refused here: {}", imsi, c));
                    self.tx_plain(src, Plain::AttachReject { cause: c });
                    self.ues.remove(&src);
                    return;
                }
                let u = self.ues.get_mut(&src).expect("checked");
                u.imsi = imsi.clone();
                u.st = St::AttachWait;
                let cell_id = self.cfg.cell_id;
                self.ctl(Ctl::UeAttach { ue: src, cell_id, imsi, carrier });
            }
            Plain::ServiceRequest { tmsi, count, mac } => {
                if self.ues.get(&src).map(|u| u.st) != Some(St::Rach) {
                    return;
                }
                let u = self.ues.get_mut(&src).expect("checked");
                u.st = St::AttachWait;
                u.tmsi = tmsi;
                let cell_id = self.cfg.cell_id;
                self.ctl(Ctl::UeService { ue: src, cell_id, tmsi, count, mac });
            }
            Plain::AuthResponse { res } => {
                if self.ues.get(&src).map(|u| u.st) != Some(St::AuthWait) {
                    return;
                }
                self.ues.get_mut(&src).expect("checked").st = St::CoreAuth;
                self.ctl(Ctl::UeAuthResponse { ue: src, res });
            }
            _ => {}
        }
    }

    fn on_item(&mut self, a: u32, i: Item, now: Now) {
        let Some(u) = self.ues.get_mut(&a) else { return };
        match (i, u.st) {
            (Item::SecurityModeComplete, St::Smc) => {
                u.st = St::Accepting;
                let ip = u.ip.unwrap_or_default();
                let accept = Item::AttachAccept { ip: ip.ip, prefix: ip.prefix, gateway: ip.gateway, dns: ip.dns, mtu: ip.mtu, tmsi: u.tmsi, session: u.session };
                u.ctl_q.push_back(accept);
                self.reschedule(now, true);
            }
            (Item::AttachComplete, St::Accepting) => {
                u.st = St::Connected;
                u.last_activity_ms = now.ms;
                let (session, imsi, ip) = (u.session, u.imsi.clone(), u.ip.map(|c| c.ip).unwrap_or_default());
                self.ctl(Ctl::UeActive { session });
                self.log(format!("{} attached here (ip {})", if imsi.is_empty() { "phone".into() } else { imsi }, fmt_ip(ip)));
            }
            (Item::HandoverConfirm, St::HoIn(ho)) => {
                u.st = St::Connected;
                u.last_activity_ms = now.ms;
                let session = u.session;
                self.ctl(Ctl::PathSwitch { ho, session });
                self.log(format!("phone {:08x} arrived by handover", a));
                self.reschedule(now, true);
            }
            (Item::Data { seq, idx, count, bytes }, St::Connected | St::HoOut) => {
                u.last_activity_ms = now.ms;
                if let Some(pkt) = u.reasm.push(seq, idx, count, &bytes, now.ms) {
                    u.ul_seq = u.ul_seq.wrapping_add(1).max(1);
                    u.used += pkt.len() as u64;
                    let (session, seq) = (u.session, u.ul_seq);
                    if self.link == Link::Up && session != 0 {
                        for d in tunnel_split(session, seq, &pkt) {
                            self.out.push(SiteOut::Tunnel(d));
                        }
                    }
                }
            }
            (Item::BufferStatus { bytes }, _) => u.bsr = bytes,
            (Item::MeasurementReport { cells }, St::Connected) => {
                u.meas = cells.clone();
                self.on_report(a, cells, now);
            }
            (Item::Detach { cause: _ }, _) => {
                let session = u.session;
                u.ctl_q.clear();
                u.dl_q.clear();
                u.ctl_q.push_back(Item::DetachAccept);
                u.release_after_send = true;
                u.session = 0;
                let imsi = u.imsi.clone();
                if session != 0 {
                    self.ctl(Ctl::UeRelease { session, cause: Cause::UserDetach });
                }
                self.log(format!("{} detached", imsi));
            }
            (Item::MeasGap { from_slot, to_slot }, _) => u.gap = Some((from_slot, to_slot)),
            _ => {}
        }
    }

    fn on_report(&mut self, a: u32, cells: Vec<Measurement>, now: Now) {
        // Automatic neighbour relations: remember every cell the phones hear.
        for m in cells.iter().filter(|m| m.cell_id != self.cfg.cell_id) {
            self.learned.insert(m.cell_id, (Neighbour { cell_id: m.cell_id, profile: m.profile, channel: m.channel, ul_mask: m.ul_mask }, now.ms));
        }
        let Some(serving) = cells.iter().find(|m| m.cell_id == self.cfg.cell_id) else { return };
        let hyst = (self.cfg.ho_hysteresis_db * 10.0) as i16;
        let Some(best) = cells.iter().filter(|m| m.cell_id != self.cfg.cell_id).max_by_key(|m| m.rssi_x10) else { return };
        if best.rssi_x10 <= serving.rssi_x10.saturating_add(hyst) || self.link != Link::Up {
            return;
        }
        let u = self.ues.get_mut(&a).expect("caller checked");
        if u.ho_pending_ms.is_some_and(|t| now.ms.saturating_sub(t) < 3000) {
            return;
        }
        u.ho_pending_ms = Some(now.ms);
        let (session, target_cell) = (u.session, best.cell_id);
        let imsi = u.imsi.clone();
        self.log(format!("{}: cell {} is {:.1} dB stronger, handover", imsi, target_cell, (best.rssi_x10 - serving.rssi_x10) as f64 / 10.0));
        self.ctl(Ctl::HoRequired { session, target_cell });
    }

    pub fn on_ctl(&mut self, m: Ctl, now: Now) {
        self.last_core_rx_ms = now.ms;
        match m {
            Ctl::CoreHello { version: _, carrier, nonce } if self.link == Link::Hello => {
                if carrier != self.cfg.carrier {
                    self.log(format!("core serves carrier '{}', this site is '{}'", carrier, self.cfg.carrier));
                }
                let c = &self.cfg;
                let proof = site_proof(&c.secret, &nonce, &c.site_id);
                let cells = alloc::vec![CellInfo { cell_id: c.cell_id, profile: c.profile.code(), channel: c.channel, area: c.area, addr: c.address }];
                let m = Ctl::SiteAuth { version: crate::backhaul::VERSION, site_id: c.site_id.clone(), cells, tunnel_port: c.tunnel_port, proof };
                self.link = Link::Registering;
                self.ctl(m);
            }
            Ctl::SiteAccept { heartbeat_ms, revoked } if self.link == Link::Registering => {
                self.link = Link::Up;
                self.hb_ms = (heartbeat_ms as u64).max(100);
                self.revoked = revoked.into_iter().collect();
                self.log("registered with the core".into());
                self.update_sib(now);
            }
            Ctl::SiteReject { cause } => {
                self.log(format!("core refused this site: {}", cause));
                self.reject_until_ms = now.ms + 10_000;
                self.out.push(SiteOut::CloseCore);
                self.core_disconnected(now);
            }
            Ctl::Heartbeat { .. } => {}
            Ctl::UeChallenge { ue, rand } => {
                if self.ues.get(&ue).map(|u| u.st) == Some(St::AttachWait) {
                    self.ues.get_mut(&ue).expect("checked").st = St::AuthWait;
                    self.tx_plain(ue, Plain::AuthChallenge { rand });
                }
            }
            Ctl::UeAccept { ue, session, ip, tmsi, ctx, smc_mac } => {
                let Some(u) = self.ues.get_mut(&ue) else {
                    self.ctl(Ctl::UeRelease { session, cause: Cause::RadioLost });
                    return;
                };
                if !matches!(u.st, St::CoreAuth | St::AttachWait) {
                    return;
                }
                u.st = St::Smc;
                u.session = session;
                u.ip = Some(ip);
                u.tmsi = tmsi;
                u.cipher = Some(AirCipher::new(&ctx.k_as, ctx.epoch, false));
                self.tx_plain(ue, Plain::SecurityModeCommand { epoch: ctx.epoch, mac: smc_mac });
            }
            Ctl::UeReject { ue, cause } => {
                if self.ues.remove(&ue).is_some() {
                    self.tx_plain(ue, Plain::AttachReject { cause });
                }
            }
            Ctl::UeRelease { session, cause } => {
                if let Some(a) = self.addr_of(session) {
                    if cause == Cause::HandoverDone {
                        self.ues.remove(&a);
                    } else {
                        self.release_ue(a, cause, false);
                    }
                }
            }
            Ctl::HoCommand { ho: _, session, cell_id, addr, profile, channel, epoch, ue_id, grant } => {
                if let Some(a) = self.addr_of(session) {
                    let u = self.ues.get_mut(&a).expect("found");
                    if u.st == St::Connected {
                        u.st = St::HoOut;
                        u.ctl_q.push_back(Item::HandoverCommand { cell_id, addr, profile, channel, epoch, ue_id, grant });
                    }
                }
            }
            Ctl::HoFailure { session, .. } => {
                if let Some(a) = self.addr_of(session) {
                    let u = self.ues.get_mut(&a).expect("found");
                    if u.st == St::HoOut {
                        u.st = St::Connected;
                    }
                }
            }
            Ctl::HoRequest { ho, session, ue, target_cell, ip, tmsi, ctx } => {
                if target_cell != self.cfg.cell_id {
                    self.ctl(Ctl::HoFailure { ho, session, cause: Cause::HandoverFailed });
                    return;
                }
                if self.ues.contains_key(&ue) {
                    self.ues.remove(&ue);
                }
                let ue_id = self.next_ue_id;
                self.next_ue_id = self.next_ue_id.wrapping_add(1).max(1);
                let mut c = UeCtx::new(ue, ue_id, St::HoIn(ho), 0, 7, now.ms, None);
                c.session = session;
                c.ip = Some(ip);
                c.tmsi = tmsi;
                c.cipher = Some(AirCipher::new(&ctx.k_as, ctx.epoch, false));
                self.ues.insert(ue, c);
                self.reschedule(now, false);
                let grant = self.ues[&ue].grant.expect("scheduled");
                self.ctl(Ctl::HoRequestAck { ho, ue_id, grant });
            }
            Ctl::PathSwitchAck { .. } => {}
            Ctl::Page { tmsi } => {
                self.pages.retain(|(t, _)| *t != tmsi);
                self.pages.push((tmsi, now.ms + 1500));
                self.last_page_ms = 0;
            }
            Ctl::Usage { session, used, .. } => {
                if let Some(a) = self.addr_of(session) {
                    self.ues.get_mut(&a).expect("found").used = used;
                }
            }
            Ctl::Revoked { imsi } => {
                self.revoked.insert(imsi);
            }
            other => self.log(format!("unexpected {} from the core", other.name())),
        }
    }

    fn addr_of(&self, session: u32) -> Option<u32> {
        if session == 0 {
            return None;
        }
        self.ues.iter().find(|(_, u)| u.session == session).map(|(a, _)| *a)
    }

    /// A datagram from the core's tunnel.
    pub fn on_tunnel(&mut self, bytes: &[u8], _now: Now) {
        let Ok(p) = TunnelPacket::decode(bytes) else { return };
        let Some((session, seq, pkt)) = self.join.push(p) else { return };
        let Some(a) = self.addr_of(session) else { return };
        let u = self.ues.get_mut(&a).expect("found");
        if u.dl_q.len() >= DL_QUEUE {
            u.dl_q.pop_front();
        }
        u.dl_q.push_back((seq, pkt));
    }

    // ------------------------------------------------------------ time

    fn reschedule(&mut self, now: Now, notify: bool) {
        let nb_mask = self
            .neighbours()
            .iter()
            .filter(|n| n.profile == self.cfg.profile.code() && n.channel == self.cfg.channel)
            .fold(0u8, |m, n| m | n.ul_mask);
        let subs = sub_preference(self.cfg.cell_id, self.cfg.rach_sub, nb_mask);
        let ues: Vec<SchedUe> =
            self.ues.values().filter(|u| u.st.secured() && !u.release_after_send).map(|u| SchedUe { addr: u.addr, bsr: u.bsr, cqi: u.cqi_ul }).collect();
        let spw = SCHED_PERIOD_MS * 1000 / self.cfg.profile.slot_us();
        let run = match self.cfg.profile {
            Profile::Lte => 4,
            Profile::Mmwave => 16,
        };
        let (grants, mask) = plan(&ues, &subs, now.slot + 1, now.slot + 1 + 4 * spw, run);
        self.ul_mask = mask;
        let settle = 10 * 1000 / self.cfg.profile.slot_us();
        for (a, mut g) in grants {
            let u = self.ues.get_mut(&a).expect("planned");
            let same = u.grant.is_some_and(|o| (o.period, o.offset, o.len, o.subchannel) == (g.period, g.offset, g.len, g.subchannel));
            if !same && u.grant.is_some() {
                // a new pattern starts a little later, so it never meets frames
                // other phones already queued under their old grants
                g.from_slot += settle;
            }
            u.grant = Some(g);
            if notify && matches!(u.st, St::Accepting | St::Connected | St::HoOut) {
                u.ctl_q.retain(|i| !matches!(i, Item::Grant(_)));
                u.ctl_q.push_back(Item::Grant(g));
            }
        }
    }

    pub fn tick(&mut self, now: Now) {
        // core link
        if matches!(self.link, Link::Hello | Link::Registering | Link::Up) {
            if now.ms.saturating_sub(self.last_core_rx_ms) > self.hb_ms * 7 / 2 {
                self.log("core stopped answering".into());
                self.out.push(SiteOut::CloseCore);
                self.core_disconnected(now);
            } else if self.link == Link::Up && now.ms.saturating_sub(self.last_hb_tx_ms) >= self.hb_ms {
                self.last_hb_tx_ms = now.ms;
                self.hb_seq += 1;
                let seq = self.hb_seq;
                self.ctl(Ctl::Heartbeat { seq });
            }
        }
        // phones that went quiet
        let quiet: Vec<(u32, bool)> = self
            .ues
            .iter()
            .filter(|(_, u)| {
                let limit = if u.st.secured() { self.cfg.ue_timeout_ms } else { 4000 };
                now.ms.saturating_sub(u.last_rx_ms) > limit && !u.release_after_send
            })
            .map(|(a, u)| (*a, u.session != 0))
            .collect();
        for (a, has) in quiet {
            if has {
                let imsi = self.ues[&a].imsi.clone();
                self.log(format!("{} lost (silent)", imsi));
                self.release_ue(a, Cause::RadioLost, true);
            }
            self.ues.remove(&a);
        }
        if self.cfg.idle_after_ms > 0 {
            let idle: Vec<u32> = self
                .ues
                .iter()
                .filter(|(_, u)| u.st == St::Connected && !u.release_after_send && now.ms.saturating_sub(u.last_activity_ms) > self.cfg.idle_after_ms)
                .map(|(a, _)| *a)
                .collect();
            for a in idle {
                let session = self.ues[&a].session;
                self.ctl(Ctl::UeIdle { session });
                self.release_ue(a, Cause::Inactivity, false);
            }
        }
        if now.ms.saturating_sub(self.last_sched_ms) >= SCHED_PERIOD_MS {
            self.last_sched_ms = now.ms;
            self.reschedule(now, true);
            self.learned.retain(|_, (_, t)| now.ms.saturating_sub(*t) < 60_000);
            self.update_sib(now);
        }
        // paging
        self.pages.retain(|(_, until)| *until > now.ms);
        if !self.pages.is_empty() && now.ms.saturating_sub(self.last_page_ms) >= SCHED_PERIOD_MS {
            self.last_page_ms = now.ms;
            let tmsis = self.pages.iter().map(|(t, _)| *t).collect();
            self.tx_plain(BROADCAST, Plain::Paging { tmsis });
        }
        self.pump_dl(now);
    }

    pub fn status(&self) -> SiteStatus {
        SiteStatus {
            core: match self.link {
                Link::Down => "down",
                Link::Hello | Link::Registering => "registering",
                Link::Up => "up",
            },
            ues: self
                .ues
                .values()
                .map(|u| UeLine {
                    addr: u.addr,
                    imsi: u.imsi.clone(),
                    state: u.st.name(),
                    ip: u.ip.map(|c| c.ip),
                    rssi_dbm: u.rssi_dbm,
                    cqi_ul: u.cqi_ul,
                    subchannel: u.grant.map(|g| g.subchannel).unwrap_or(0),
                    used: u.used,
                })
                .collect(),
            neighbours: self.neighbours(),
            ul_mask: self.ul_mask,
        }
    }
}

impl UeCtx {
    fn new(addr: u32, ue_id: u16, st: St, nonce: u32, cqi: u8, now_ms: u64, grant: Option<Grant>) -> UeCtx {
        UeCtx {
            addr,
            ue_id,
            st,
            imsi: String::new(),
            session: 0,
            ip: None,
            tmsi: 0,
            cipher: None,
            grant,
            cqi_ul: cqi.max(1),
            bsr: 0,
            ctl_q: VecDeque::new(),
            dl_q: VecDeque::new(),
            dl_segs: VecDeque::new(),
            reasm: Reassembly::default(),
            ul_seq: 0,
            gap: None,
            inflight: 0,
            dl_fail: 0,
            last_rx_ms: now_ms,
            last_activity_ms: now_ms,
            ho_pending_ms: None,
            rach_nonce: nonce,
            meas: Vec::new(),
            release_after_send: false,
            rssi_dbm: 0.0,
            used: 0,
        }
    }
}
