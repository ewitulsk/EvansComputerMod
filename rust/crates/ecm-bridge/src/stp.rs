//! Rapid Spanning Tree (IEEE 802.1D-2004 clause 17) on the CIST.
//!
//! Implemented: spanning tree priority vectors with port-ID tie breaks
//! (17.5/17.6), rcvInfo superior/repeated/inferior classification
//! (17.21.8), message-age increment and max-age / 3xHello information
//! expiry (17.21.23), updtRolesTree role selection incl. Alternate/Backup
//! (17.21.25), Discarding/Learning/Forwarding with the forward-delay timer
//! fallback in place of proposal/agreement, topology change detection and
//! propagation with FDB flush (17.25/17.29), admin/oper edge, STP (v0)
//! compatibility incl. TCN/TCA, BPDU transmit hold count, root guard,
//! BPDU guard (in `bridge.rs`) and TCN guard.

use crate::bpdu::{self, Bpdu, BpduBody, BpduRole};
use crate::bridge::{Bridge, Deadline};
use crate::log::Severity;
use crate::types::{PortRef, StpConfig};

/// BPDUs a port may send per second (TxHoldCount).
const TX_HOLD_COUNT: u32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StpRole {
    Disabled,
    Root,
    Designated,
    Alternate,
    Backup,
}

impl StpRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            StpRole::Disabled => "Disabled",
            StpRole::Root => "Root",
            StpRole::Designated => "Designated",
            StpRole::Alternate => "Alternate",
            StpRole::Backup => "Backup",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StpPortState {
    Discarding,
    Learning,
    Forwarding,
}

impl StpPortState {
    pub fn as_str(&self) -> &'static str {
        match self {
            StpPortState::Discarding => "Discarding",
            StpPortState::Learning => "Learning",
            StpPortState::Forwarding => "Forwarding",
        }
    }
}

/// Spanning tree priority vector (17.5). Field order == comparison order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PrioVec {
    pub root: u64,
    pub cost: u32,
    pub dbridge: u64,
    pub dport: u16,
    pub rport: u16,
}

impl PrioVec {
    fn key4(&self) -> (u64, u32, u64, u16) {
        (self.root, self.cost, self.dbridge, self.dport)
    }
    /// 17.6: superior if better, or from the same designated bridge (MAC)
    /// and designated port (number), i.e. the same transmitter.
    fn superior_to(&self, port: &PrioVec) -> bool {
        self.key4() < port.key4()
            || (mac48(self.dbridge) == mac48(port.dbridge) && (self.dport & 0x0fff) == (port.dport & 0x0fff))
    }
}

/// Timer values carried in BPDUs, in 1/256 s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Times {
    pub msg_age: u16,
    pub max_age: u16,
    pub hello: u16,
    pub fwd_delay: u16,
}

impl Times {
    fn from_cfg(c: &StpConfig) -> Self {
        Times { msg_age: 0, max_age: secs256(c.max_age_secs), hello: secs256(c.hello_secs), fwd_delay: secs256(c.forward_delay_secs) }
    }
}

fn secs256(s: u32) -> u16 {
    (s.min(255) * 256) as u16
}

fn clamp256(v: u16, min_s: u16, max_s: u16) -> u16 {
    v.clamp(min_s * 256, max_s * 256)
}

fn mac48(id: u64) -> u64 {
    id & 0x0000_ffff_ffff_ffff
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InfoIs {
    Disabled,
    Aged,
    Mine,
    Received,
}

#[derive(Clone, Debug)]
pub(crate) struct StpPort {
    pub port_num: u16,
    pub role: StpRole,
    pub state: StpPortState,
    pub info: InfoIs,
    pub prio: PrioVec,
    pub times: Times,
    pub rcvd_until: i64,
    pub fd_until: Option<i64>,
    pub tc_until: i64,
    pub oper_edge: bool,
    pub send_rstp: bool,
    pub root_inconsistent: bool,
    pub next_hello: i64,
    pub new_info: bool,
    pub tx_window: i64,
    pub tx_in_window: u32,
    pub tc_ack: bool,
    pub rcvd_tc: bool,
    pub was_oper: bool,
    pub rx_bpdus: u64,
    pub tx_bpdus: u64,
}

impl StpPort {
    pub fn new(port_num: u16) -> Self {
        StpPort {
            port_num,
            role: StpRole::Disabled,
            state: StpPortState::Discarding,
            info: InfoIs::Disabled,
            prio: PrioVec { root: u64::MAX, cost: u32::MAX, dbridge: u64::MAX, dport: u16::MAX, rport: 0 },
            times: Times { msg_age: 0, max_age: 20 * 256, hello: 2 * 256, fwd_delay: 15 * 256 },
            rcvd_until: 0,
            fd_until: None,
            tc_until: i64::MIN,
            oper_edge: false,
            send_rstp: true,
            root_inconsistent: false,
            next_hello: i64::MIN,
            new_info: false,
            tx_window: i64::MIN,
            tx_in_window: 0,
            tc_ack: false,
            rcvd_tc: false,
            was_oper: false,
            rx_bpdus: 0,
            tx_bpdus: 0,
        }
    }

    fn reset(&mut self) {
        let n = self.port_num;
        let (rx, tx) = (self.rx_bpdus, self.tx_bpdus);
        *self = StpPort::new(n);
        self.rx_bpdus = rx;
        self.tx_bpdus = tx;
    }
}

/// Bridge-wide RSTP state.
#[derive(Clone, Debug)]
pub(crate) struct StpBridge {
    pub bridge_id: u64,
    pub root_prio: PrioVec,
    pub root_port: Option<PortRef>,
    pub root_times: Times,
    pub tc_count: u64,
    pub last_tc_ms: Option<i64>,
    pub was_enabled: bool,
}

impl StpBridge {
    pub fn new(cfg: &StpConfig, mac: [u8; 6]) -> Self {
        let bid = make_bid(cfg.priority, mac);
        StpBridge {
            bridge_id: bid,
            root_prio: PrioVec { root: bid, cost: 0, dbridge: bid, dport: 0, rport: 0 },
            root_port: None,
            root_times: Times::from_cfg(cfg),
            tc_count: 0,
            last_tc_ms: None,
            was_enabled: false,
        }
    }
}

pub(crate) fn make_bid(priority: u16, mac: [u8; 6]) -> u64 {
    let mut b = [0u8; 8];
    b[..2].copy_from_slice(&priority.to_be_bytes());
    b[2..].copy_from_slice(&mac);
    u64::from_be_bytes(b)
}

/// Port number used in the 12-bit port-number field of the port ID.
pub(crate) fn port_number(bp: PortRef) -> u16 {
    match bp {
        PortRef::Eth(n) => (n as u16).saturating_add(1) & 0x03ff,
        PortRef::Lag(id) => 0x400 | (id & 0x03ff),
        PortRef::Cpu => 0,
    }
}

/// Format a bridge ID as `pppp.mmmmmm.mmmmmm`.
pub fn format_bid(id: u64) -> String {
    let b = id.to_be_bytes();
    format!("{:02x}{:02x}.{:02x}{:02x}{:02x}.{:02x}{:02x}{:02x}", b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7])
}

impl Bridge {
    fn fd_ms(&self) -> i64 {
        self.stp_cfg.forward_delay_secs as i64 * 1000
    }
    fn hello_ms(&self) -> i64 {
        self.stp_cfg.hello_secs as i64 * 1000
    }

    /// Is STP controlling this bridge port?
    pub(crate) fn stp_active(&self, bp: PortRef) -> bool {
        self.stp_cfg.enabled && self.bp_stp_cfg(bp).map(|c| c.enabled).unwrap_or(false)
    }

    fn port_id(&self, bp: PortRef, num: u16) -> u16 {
        let prio = self.bp_stp_cfg(bp).map(|c| c.priority).unwrap_or(128) as u16;
        ((prio >> 4) << 12) | (num & 0x0fff)
    }

    fn path_cost(&self, bp: PortRef) -> u32 {
        self.bp_stp_cfg(bp).map(|c| c.cost.max(1)).unwrap_or(20_000)
    }

    /// Data-plane forwarding gate for a bridge port.
    pub(crate) fn bp_forwarding(&self, bp: PortRef) -> bool {
        match self.bports.get(&bp) {
            Some(b) if b.oper => !self.stp_active(bp) || b.stp.state == StpPortState::Forwarding,
            _ => false,
        }
    }

    /// Learning allowed in Learning and Forwarding.
    pub(crate) fn bp_learning(&self, bp: PortRef) -> bool {
        match self.bports.get(&bp) {
            Some(b) if b.oper => !self.stp_active(bp) || b.stp.state >= StpPortState::Learning,
            _ => false,
        }
    }

    /// Process a received BPDU on bridge port `bp` (already validated as
    /// operational and STP-controlled). Returns true if state machines must run.
    pub(crate) fn stp_rx(&mut self, bp: PortRef, b: Bpdu, now: i64) -> bool {
        let bid = self.stp.bridge_id;
        let tcn_guard = self.bp_stp_cfg(bp).map(|c| c.tcn_guard).unwrap_or(false);
        let port_id = {
            let num = match self.bports.get(&bp) {
                Some(x) => x.stp.port_num,
                None => return false,
            };
            self.port_id(bp, num)
        };
        let p = match self.bports.get_mut(&bp) {
            Some(x) => &mut x.stp,
            None => return false,
        };
        p.rx_bpdus += 1;
        let was_edge = p.oper_edge;
        p.oper_edge = false; // a BPDU means there is a bridge on this segment
        let mut changed = was_edge;
        match b {
            Bpdu::Tcn => {
                p.send_rstp = false;
                if p.role == StpRole::Designated && !tcn_guard {
                    p.tc_ack = true;
                    p.rcvd_tc = true;
                    changed = true;
                }
            }
            Bpdu::Config(body) | Bpdu::Rst(body) => {
                let is_rst = matches!(b, Bpdu::Rst(_));
                if p.send_rstp != is_rst {
                    p.send_rstp = is_rst;
                    changed = true;
                }
                // Our own BPDU looped back to the port that sent it.
                if body.bridge_id == bid && (body.port_id & 0x0fff) == (port_id & 0x0fff) {
                    return changed;
                }
                let role = if is_rst { body.role() } else { BpduRole::Designated };
                if role == BpduRole::Designated {
                    let msg = PrioVec {
                        root: body.root_id,
                        cost: body.root_path_cost,
                        dbridge: body.bridge_id,
                        dport: body.port_id,
                        rport: port_id,
                    };
                    let times = Times {
                        msg_age: body.message_age,
                        max_age: clamp256(body.max_age, 6, 40),
                        hello: clamp256(body.hello_time, 1, 10),
                        fwd_delay: clamp256(body.forward_delay, 4, 30),
                    };
                    let repeated = p.info == InfoIs::Received && msg == p.prio && times == p.times;
                    if repeated {
                        p.rcvd_until = now.saturating_add(3 * (times.hello as i64 * 1000 / 256));
                    } else if msg.superior_to(&p.prio) || p.info != InfoIs::Received && p.info != InfoIs::Mine {
                        p.prio = msg;
                        p.times = times;
                        if times.msg_age as u32 + 256 > times.max_age as u32 {
                            // Information too old to use (17.21.23): discard.
                            p.info = InfoIs::Aged;
                        } else {
                            p.info = InfoIs::Received;
                            p.rcvd_until = now.saturating_add(3 * (times.hello as i64 * 1000 / 256));
                        }
                        changed = true;
                    }
                    // Inferior designated info: ignored (no dispute mechanism).
                }
                if body.flags & bpdu::FLAG_TC != 0 && !tcn_guard {
                    p.rcvd_tc = true;
                    changed = true;
                }
                if !is_rst && body.flags & bpdu::FLAG_TC_ACK != 0 && p.role == StpRole::Root {
                    p.tc_until = i64::MIN; // TCN acknowledged
                }
            }
        }
        changed
    }

    /// Run all RSTP machines once.
    pub(crate) fn stp_run(&mut self, now: i64) {
        if !self.stp_cfg.enabled {
            if self.stp.was_enabled {
                self.stp.was_enabled = false;
                for b in self.bports.values_mut() {
                    b.stp.reset();
                }
            }
            return;
        }
        if !self.stp.was_enabled {
            self.stp.was_enabled = true;
            for b in self.bports.values_mut() {
                b.stp.reset();
            }
        }
        let bid = make_bid(self.stp_cfg.priority, self.bridge_mac);
        self.stp.bridge_id = bid;
        let keys: Vec<PortRef> = self.bports.keys().copied().collect();

        // Port enable/disable and information expiry.
        for &bp in &keys {
            let active = self.stp_active(bp);
            let admin_edge = self.bp_stp_cfg(bp).map(|c| c.admin_edge).unwrap_or(false);
            let pid = self.bports.get(&bp).map(|x| self.port_id(bp, x.stp.port_num)).unwrap_or(0);
            let b = match self.bports.get_mut(&bp) {
                Some(b) => b,
                None => continue,
            };
            let up = b.oper && active;
            let p = &mut b.stp;
            if !up {
                if p.was_oper || p.info != InfoIs::Disabled {
                    p.reset();
                }
                continue;
            }
            if !p.was_oper {
                p.was_oper = true;
                p.info = InfoIs::Aged;
                p.oper_edge = admin_edge;
                p.next_hello = now;
                p.new_info = true;
            }
            if !admin_edge {
                p.oper_edge = false;
            }
            p.prio.rport = pid;
            if p.info == InfoIs::Received && now >= p.rcvd_until {
                p.info = InfoIs::Aged;
            }
        }

        self.stp_reselect(&keys);
        self.stp_update_states(&keys, now);

        // Received topology changes (after roles are known).
        for &bp in &keys {
            let tc = match self.bports.get_mut(&bp) {
                Some(b) => std::mem::replace(&mut b.stp.rcvd_tc, false),
                None => false,
            };
            if tc {
                let role = self.bports.get(&bp).map(|b| b.stp.role);
                if matches!(role, Some(StpRole::Root) | Some(StpRole::Designated)) {
                    self.stp_topology_change(bp, false, now);
                }
            }
        }
        self.stp_tx(&keys, now);
    }

    /// updtRolesTree (17.21.25) + updtRoleDisabledTree.
    fn stp_reselect(&mut self, keys: &[PortRef]) {
        let bid = self.stp.bridge_id;
        let own_mac = mac48(bid);
        let mut best = PrioVec { root: bid, cost: 0, dbridge: bid, dport: 0, rport: 0 };
        let mut root_port = None;
        let mut root_times = Times::from_cfg(&self.stp_cfg);
        for &bp in keys {
            let root_guard = self.bp_stp_cfg(bp).map(|c| c.root_guard).unwrap_or(false);
            let cost = self.path_cost(bp);
            let b = match self.bports.get(&bp) {
                Some(b) => b,
                None => continue,
            };
            let p = &b.stp;
            if !p.was_oper || p.info != InfoIs::Received || root_guard || mac48(p.prio.dbridge) == own_mac {
                continue;
            }
            let cand = PrioVec { cost: p.prio.cost.saturating_add(cost), ..p.prio };
            if cand < best {
                best = cand;
                root_port = Some(bp);
                root_times = Times { msg_age: p.times.msg_age.saturating_add(256), ..p.times };
            }
        }
        let old_root = self.stp.root_prio.root;
        let old_port = self.stp.root_port;
        self.stp.root_prio = best;
        self.stp.root_port = root_port;
        self.stp.root_times = root_times;
        if old_root != best.root || old_port != root_port {
            let via = root_port.map(|p| p.to_string()).unwrap_or_else(|| "self".to_string());
            let sev = if old_root != best.root { Severity::Warning } else { Severity::Notice };
            self.log(sev, format!("STP root {} (cost {}) via {}", format_bid(best.root), best.cost, via));
        }

        for &bp in keys {
            let root_guard = self.bp_stp_cfg(bp).map(|c| c.root_guard).unwrap_or(false);
            let pid = match self.bports.get(&bp) {
                Some(b) => self.port_id(bp, b.stp.port_num),
                None => continue,
            };
            let designated = PrioVec { root: best.root, cost: best.cost, dbridge: bid, dport: pid, rport: pid };
            let mut msg = None;
            let b = match self.bports.get_mut(&bp) {
                Some(b) => b,
                None => continue,
            };
            let p = &mut b.stp;
            if !p.was_oper {
                p.role = StpRole::Disabled;
                continue;
            }
            let mut inconsistent = false;
            let new_role = match p.info {
                InfoIs::Disabled | InfoIs::Aged | InfoIs::Mine => {
                    if p.info != InfoIs::Mine || p.prio != designated || p.times != root_times {
                        p.new_info = true;
                    }
                    p.info = InfoIs::Mine;
                    p.prio = designated;
                    p.times = root_times;
                    StpRole::Designated
                }
                InfoIs::Received => {
                    if Some(bp) == root_port {
                        StpRole::Root
                    } else if designated.key4() < p.prio.key4() {
                        p.info = InfoIs::Mine;
                        p.prio = designated;
                        p.times = root_times;
                        p.new_info = true;
                        StpRole::Designated
                    } else if root_guard {
                        inconsistent = true;
                        StpRole::Alternate
                    } else if mac48(p.prio.dbridge) == own_mac {
                        StpRole::Backup
                    } else {
                        StpRole::Alternate
                    }
                }
            };
            if new_role != p.role {
                if new_role == StpRole::Designated || new_role == StpRole::Root {
                    p.new_info = true;
                }
                p.role = new_role;
            }
            if inconsistent != p.root_inconsistent {
                p.root_inconsistent = inconsistent;
                msg = Some(if inconsistent {
                    (Severity::Warning, format!("STP root-guard: superior BPDU on {}, port root-inconsistent", bp))
                } else {
                    (Severity::Notice, format!("STP root-guard: {} no longer root-inconsistent", bp))
                });
            }
            if let Some((s, m)) = msg {
                self.log(s, m);
            }
        }
    }

    /// Port state transitions (timer fallback instead of proposal/agreement).
    fn stp_update_states(&mut self, keys: &[PortRef], now: i64) {
        let fd = self.fd_ms();
        for &bp in keys {
            let b = match self.bports.get_mut(&bp) {
                Some(b) => b,
                None => continue,
            };
            let p = &mut b.stp;
            let old = p.state;
            match p.role {
                StpRole::Disabled | StpRole::Alternate | StpRole::Backup => {
                    p.state = StpPortState::Discarding;
                    p.fd_until = None;
                }
                StpRole::Root | StpRole::Designated => {
                    if p.role == StpRole::Designated && p.oper_edge {
                        p.state = StpPortState::Forwarding;
                        p.fd_until = None;
                    } else {
                        match p.state {
                            StpPortState::Discarding => match p.fd_until {
                                None => p.fd_until = Some(now.saturating_add(fd)),
                                Some(t) if now >= t => {
                                    p.state = StpPortState::Learning;
                                    p.fd_until = Some(now.saturating_add(fd));
                                    p.new_info = true;
                                }
                                _ => {}
                            },
                            StpPortState::Learning => {
                                if p.fd_until.map(|t| now >= t).unwrap_or(true) {
                                    p.state = StpPortState::Forwarding;
                                    p.fd_until = None;
                                    p.new_info = true;
                                }
                            }
                            StpPortState::Forwarding => {}
                        }
                    }
                }
            }
            let new = p.state;
            let edge = p.oper_edge;
            if old != new {
                if new == StpPortState::Discarding {
                    self.fdb.flush_port(bp);
                }
                self.log(Severity::Info, format!("STP {} {} -> {}", bp, old.as_str(), new.as_str()));
                if new == StpPortState::Forwarding && !edge {
                    self.stp_topology_change(bp, true, now);
                }
            }
        }
    }

    /// Topology change detected on (`include_origin`) or received on `origin`.
    pub(crate) fn stp_topology_change(&mut self, origin: PortRef, include_origin: bool, now: i64) {
        let tc_ms = 2 * self.hello_ms();
        let was_active = self.bports.values().any(|b| b.stp.tc_until > now);
        for (bp, b) in self.bports.iter_mut() {
            let p = &mut b.stp;
            if !p.was_oper || p.oper_edge || !matches!(p.role, StpRole::Root | StpRole::Designated) {
                continue;
            }
            if *bp == origin && !include_origin {
                continue;
            }
            if p.tc_until <= now {
                p.tc_until = now.saturating_add(tc_ms);
                p.new_info = true;
            }
        }
        self.fdb.flush_except(origin);
        self.fast_age_until = now.saturating_add(self.fd_ms());
        if !was_active {
            self.stp.tc_count += 1;
            self.stp.last_tc_ms = Some(now);
            self.log(
                Severity::Notice,
                format!("STP topology change {} {}", if include_origin { "detected on" } else { "received on" }, origin),
            );
        }
    }

    fn stp_tx(&mut self, keys: &[PortRef], now: i64) {
        let hello = self.hello_ms().max(1000);
        let bid = self.stp.bridge_id;
        let root = self.stp.root_prio;
        let times = self.stp.root_times;
        for &bp in keys {
            let pid = match self.bports.get(&bp) {
                Some(b) => self.port_id(bp, b.stp.port_num),
                None => continue,
            };
            let bpdu = {
                let b = match self.bports.get_mut(&bp) {
                    Some(b) => b,
                    None => continue,
                };
                let p = &mut b.stp;
                if !p.was_oper {
                    continue;
                }
                let tc_active = p.tc_until > now;
                let periodic = p.role == StpRole::Designated || (p.role == StpRole::Root && tc_active);
                if periodic && now >= p.next_hello {
                    p.new_info = true;
                    p.next_hello = now.saturating_add(hello);
                }
                if !p.new_info {
                    continue;
                }
                if !periodic {
                    // Root (without TC), Alternate, Backup: nothing to say.
                    p.new_info = false;
                    continue;
                }
                if now.saturating_sub(p.tx_window) >= 1000 {
                    p.tx_window = now;
                    p.tx_in_window = 0;
                }
                if p.tx_in_window >= TX_HOLD_COUNT {
                    continue;
                }
                p.tx_in_window += 1;
                p.new_info = false;
                p.tx_bpdus += 1;
                let role_bits = if p.role == StpRole::Root { BpduRole::Root } else { BpduRole::Designated };
                let mut flags = 0u8;
                if tc_active {
                    flags |= bpdu::FLAG_TC;
                }
                if p.state >= StpPortState::Learning {
                    flags |= bpdu::FLAG_LEARNING;
                }
                if p.state == StpPortState::Forwarding {
                    flags |= bpdu::FLAG_FORWARDING;
                }
                let body = BpduBody {
                    flags,
                    root_id: root.root,
                    root_path_cost: root.cost,
                    bridge_id: bid,
                    port_id: pid,
                    message_age: times.msg_age,
                    max_age: times.max_age,
                    hello_time: times.hello,
                    forward_delay: times.fwd_delay,
                };
                if p.send_rstp {
                    Bpdu::Rst(body.with_role(role_bits))
                } else if p.role == StpRole::Root {
                    Bpdu::Tcn
                } else {
                    let mut f = flags & bpdu::FLAG_TC;
                    if std::mem::replace(&mut p.tc_ack, false) {
                        f |= bpdu::FLAG_TC_ACK;
                    }
                    Bpdu::Config(BpduBody { flags: f, ..body })
                }
            };
            self.tx_control(bp, |mac| bpdu::build_frame(mac, &bpdu));
        }
    }

    pub(crate) fn stp_deadline(&self, dl: &mut Deadline) {
        if !self.stp_cfg.enabled {
            return;
        }
        for b in self.bports.values() {
            let p = &b.stp;
            if !p.was_oper {
                continue;
            }
            if p.role == StpRole::Designated || p.role == StpRole::Root && p.tc_until > self.now {
                dl.at(p.next_hello);
                if p.new_info {
                    dl.at(p.tx_window.saturating_add(1000));
                }
            }
            if let Some(t) = p.fd_until {
                dl.at(t);
            }
            if p.info == InfoIs::Received {
                dl.at(p.rcvd_until);
            }
            if p.tc_until > self.now {
                dl.at(p.tc_until);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_vector_ordering() {
        let a = PrioVec { root: 1, cost: 10, dbridge: 5, dport: 0x8001, rport: 0x8002 };
        let b = PrioVec { cost: 20, ..a };
        assert!(a < b);
        let c = PrioVec { dport: 0x8002, ..a };
        assert!(a < c); // port-id tie break
        // Same transmitter with worse info is still "superior" (replacement).
        let worse_same = PrioVec { root: 9, ..a };
        assert!(worse_same.superior_to(&a));
        let worse_other = PrioVec { root: 9, dbridge: 6, ..a };
        assert!(!worse_other.superior_to(&a));
    }

    #[test]
    fn bid_and_port_numbers() {
        let id = make_bid(4096, [2, 0, 0, 0, 0, 1]);
        assert_eq!(format_bid(id), "1000.020000.000001");
        assert_eq!(port_number(PortRef::Eth(0)), 1);
        assert_eq!(port_number(PortRef::Lag(3)), 0x403);
    }
}
