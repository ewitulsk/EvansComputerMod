//! Link aggregation with LACP (IEEE 802.1AX-2014 clause 6.4), simplified:
//!
//! * Receive machine: CURRENT / EXPIRED / DEFAULTED with `current_while`
//!   of 3 s (fast) or 90 s (slow) chosen by the *actor's* timeout bit.
//! * Periodic transmission at 1 s / 30 s chosen by the *partner's* timeout
//!   bit, suppressed when both ends are passive; NTT with a 3 PDU/s limit.
//! * Selection: members of a LAG aggregate only with the same partner
//!   (system priority, system, key). One partner group is chosen per LAG
//!   (sticky; else the largest, ties to the lowest port); members to other
//!   partners stay unselected. Loopback to our own system never aggregates.
//! * Coupled-control mux: DETACHED -> WAITING (2 s) -> ATTACHED (Sync) ->
//!   COLLECTING_DISTRIBUTING once the partner signals Sync for us.
//! * Static LAGs (`lacp mode` off) distribute on every link-up member.
//! * `fallback`: with no partner on any member, the lowest link-up member
//!   distributes alone.

use crate::bridge::{Bridge, Deadline};
use crate::frame;
use crate::lacpdu::{self, LacpInfo, Lacpdu, ST_ACTIVITY, ST_AGGREGATION, ST_COLLECTING, ST_DEFAULTED, ST_DISTRIBUTING, ST_EXPIRED, ST_SYNC, ST_TIMEOUT};
use crate::log::Severity;
use crate::types::{LacpMode, LacpRate, PortRef};

pub const FAST_PERIODIC_MS: i64 = 1_000;
pub const SLOW_PERIODIC_MS: i64 = 30_000;
pub const SHORT_TIMEOUT_MS: i64 = 3_000;
pub const LONG_TIMEOUT_MS: i64 = 90_000;
pub const AGGREGATE_WAIT_MS: i64 = 2_000;
const TX_LIMIT_PER_SEC: u32 = 3;
pub const SYSTEM_PRIORITY: u16 = 32768;
pub const PORT_PRIORITY: u16 = 32768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LacpRx {
    Disabled,
    Expired,
    Defaulted,
    Current,
}

impl LacpRx {
    pub fn as_str(&self) -> &'static str {
        match self {
            LacpRx::Disabled => "disabled",
            LacpRx::Expired => "expired",
            LacpRx::Defaulted => "defaulted",
            LacpRx::Current => "current",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LacpMux {
    Detached,
    Waiting,
    Attached,
    CollectingDistributing,
}

impl LacpMux {
    pub fn as_str(&self) -> &'static str {
        match self {
            LacpMux::Detached => "detached",
            LacpMux::Waiting => "waiting",
            LacpMux::Attached => "attached",
            LacpMux::CollectingDistributing => "coll-dist",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LacpPort {
    pub rx: LacpRx,
    pub current_until: i64,
    pub partner: LacpInfo,
    pub partner_sync: bool,
    pub selected: bool,
    pub mux: LacpMux,
    pub wait_until: i64,
    pub ntt: bool,
    pub next_periodic: i64,
    pub tx_window: i64,
    pub tx_in_window: u32,
    pub tx_count: u64,
    pub rx_count: u64,
}

impl LacpPort {
    pub fn new() -> Self {
        LacpPort {
            rx: LacpRx::Disabled,
            current_until: 0,
            partner: LacpInfo::default(),
            partner_sync: false,
            selected: false,
            mux: LacpMux::Detached,
            wait_until: 0,
            ntt: false,
            next_periodic: i64::MIN,
            tx_window: i64::MIN,
            tx_in_window: 0,
            tx_count: 0,
            rx_count: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        let (t, r) = (self.tx_count, self.rx_count);
        *self = LacpPort::new();
        self.tx_count = t;
        self.rx_count = r;
    }
}

impl Bridge {
    /// LAG id and LACP mode for a member port running LACP right now.
    fn lacp_enabled_on(&self, p: usize) -> Option<(u16, LacpMode, LacpRate)> {
        let ep = self.ports.get(p)?;
        let id = ep.cfg.lag?;
        let lag = self.lags.get(&id)?;
        if lag.cfg.mode.is_routed() || lag.cfg.lacp_mode == LacpMode::Off || !ep.link_up || !ep.cfg.admin_up {
            return None;
        }
        Some((id, lag.cfg.lacp_mode, lag.cfg.lacp_rate))
    }

    pub(crate) fn actor_info(&self, p: usize) -> LacpInfo {
        let ep = match self.ports.get(p) {
            Some(e) => e,
            None => return LacpInfo::default(),
        };
        let (key, mode, rate) = match ep.cfg.lag.and_then(|id| self.lags.get(&id).map(|l| (id, l.cfg.lacp_mode, l.cfg.lacp_rate))) {
            Some(x) => x,
            None => (0, LacpMode::Off, LacpRate::Slow),
        };
        let lp = &ep.lacp;
        let mut s = ST_AGGREGATION;
        if mode == LacpMode::Active {
            s |= ST_ACTIVITY;
        }
        if rate == LacpRate::Fast || lp.rx == LacpRx::Expired {
            s |= ST_TIMEOUT;
        }
        match lp.mux {
            LacpMux::Attached => s |= ST_SYNC,
            LacpMux::CollectingDistributing => s |= ST_SYNC | ST_COLLECTING | ST_DISTRIBUTING,
            _ => {}
        }
        if lp.rx == LacpRx::Defaulted {
            s |= ST_DEFAULTED;
        }
        if lp.rx == LacpRx::Expired {
            s |= ST_EXPIRED;
        }
        LacpInfo {
            system_priority: SYSTEM_PRIORITY,
            system: self.bridge_mac,
            key,
            port_priority: PORT_PRIORITY,
            port: (p as u16).saturating_add(1),
            state: s,
        }
    }

    /// Is physical member `p` collecting and distributing for its LAG?
    pub(crate) fn member_distributing(&self, p: usize) -> bool {
        let ep = match self.ports.get(p) {
            Some(e) => e,
            None => return false,
        };
        let lag = match ep.cfg.lag.and_then(|id| self.lags.get(&id)) {
            Some(l) => l,
            None => return false,
        };
        if !ep.link_up || !ep.cfg.admin_up {
            return false;
        }
        match lag.cfg.lacp_mode {
            LacpMode::Off => true,
            _ => ep.lacp.mux == LacpMux::CollectingDistributing || lag.fallback_member == Some(p),
        }
    }

    /// Pick the egress member for `frame` on LAG `id`.
    pub(crate) fn lag_select_member(&self, id: u16, f: &[u8]) -> Option<usize> {
        let hash = self.lags.get(&id)?.cfg.hash;
        let members: Vec<usize> =
            (0..self.ports.len()).filter(|&m| self.ports.get(m).map(|p| p.cfg.lag == Some(id)).unwrap_or(false) && self.member_distributing(m)).collect();
        if members.is_empty() {
            return None;
        }
        let h = frame::flow_hash(f, hash) as usize;
        members.get(h % members.len()).copied()
    }

    /// An LACPDU arrived on physical port `p`.
    pub(crate) fn lacp_rx(&mut self, p: usize, pdu: Lacpdu, now: i64) -> bool {
        let (_, _, rate) = match self.lacp_enabled_on(p) {
            Some(x) => x,
            None => return false,
        };
        let actor = self.actor_info(p);
        let lp = match self.ports.get_mut(p) {
            Some(e) => &mut e.lacp,
            None => return false,
        };
        if lp.rx == LacpRx::Disabled {
            return false; // not yet initialised; the next run enables it
        }
        lp.rx_count += 1;
        let np = pdu.actor;
        if lp.rx == LacpRx::Defaulted || !lp.partner.same_identity(&np) || (lp.partner.state ^ np.state) & ST_AGGREGATION != 0 {
            lp.selected = false;
        }
        lp.partner = np;
        let view_ok = pdu.partner.same_identity(&actor) && (pdu.partner.state & ST_AGGREGATION) == (actor.state & ST_AGGREGATION);
        lp.partner_sync = np.state & ST_SYNC != 0 && view_ok;
        let mask = ST_ACTIVITY | ST_TIMEOUT | ST_AGGREGATION | ST_SYNC | ST_COLLECTING | ST_DISTRIBUTING;
        if !view_ok || (pdu.partner.state & mask) != (actor.state & mask) {
            lp.ntt = true;
        }
        lp.rx = LacpRx::Current;
        lp.current_until = now + if rate == LacpRate::Fast { SHORT_TIMEOUT_MS } else { LONG_TIMEOUT_MS };
        true
    }

    pub(crate) fn lacp_run(&mut self, now: i64) {
        let n = self.ports.len();
        // Receive machine timers / enable / disable.
        for p in 0..n {
            let enabled = self.lacp_enabled_on(p).is_some();
            let mut msg = None;
            let lp = match self.ports.get_mut(p) {
                Some(e) => &mut e.lacp,
                None => continue,
            };
            if !enabled {
                if lp.rx != LacpRx::Disabled || lp.mux != LacpMux::Detached {
                    if lp.mux == LacpMux::CollectingDistributing {
                        msg = Some(format!("LACP eth{} stopped distributing (port disabled)", p));
                    }
                    lp.reset();
                }
            } else {
                if lp.rx == LacpRx::Disabled {
                    lp.rx = LacpRx::Expired;
                    lp.partner.state |= ST_TIMEOUT;
                    lp.partner_sync = false;
                    lp.current_until = now + SHORT_TIMEOUT_MS;
                    lp.ntt = true;
                } else if now >= lp.current_until {
                    match lp.rx {
                        LacpRx::Current => {
                            lp.rx = LacpRx::Expired;
                            lp.partner_sync = false;
                            lp.partner.state |= ST_TIMEOUT;
                            lp.current_until = now + SHORT_TIMEOUT_MS;
                            lp.ntt = true;
                            msg = Some(format!("LACP eth{} partner timed out (expired)", p));
                        }
                        LacpRx::Expired => {
                            lp.rx = LacpRx::Defaulted;
                            lp.partner = LacpInfo::default();
                            lp.partner_sync = false;
                            lp.selected = false;
                            lp.ntt = true;
                        }
                        _ => {}
                    }
                }
            }
            if let Some(m) = msg {
                self.log(Severity::Warning, m);
            }
        }

        // Selection per LAG.
        let lag_ids: Vec<u16> = self.lags.keys().copied().collect();
        for id in lag_ids {
            let (mode, fallback) = match self.lags.get(&id) {
                Some(l) => (l.cfg.lacp_mode, l.cfg.fallback),
                None => continue,
            };
            let members: Vec<usize> = (0..n).filter(|&m| self.ports.get(m).map(|e| e.cfg.lag == Some(id)).unwrap_or(false)).collect();
            if mode == LacpMode::Off {
                if let Some(l) = self.lags.get_mut(&id) {
                    l.fallback_member = None;
                }
                continue;
            }
            let key_of = |lp: &LacpPort| (lp.partner.system_priority, lp.partner.system, lp.partner.key);
            let eligible: Vec<(usize, (u16, [u8; 6], u16))> = members
                .iter()
                .filter_map(|&m| {
                    let ok = self.lacp_enabled_on(m).is_some();
                    let lp = &self.ports.get(m)?.lacp;
                    let valid = matches!(lp.rx, LacpRx::Current | LacpRx::Expired)
                        && lp.partner.state & ST_AGGREGATION != 0
                        && lp.partner.system != self.bridge_mac
                        && lp.partner.system != [0; 6];
                    (ok && valid).then(|| (m, key_of(lp)))
                })
                .collect();
            // Sticky: keep the group that already has selected members.
            let sticky = eligible.iter().find(|(m, _)| self.ports.get(*m).map(|e| e.lacp.selected).unwrap_or(false)).map(|(_, k)| *k);
            let chosen = sticky.or_else(|| {
                let mut best: Option<((u16, [u8; 6], u16), usize)> = None;
                for (_, k) in &eligible {
                    let count = eligible.iter().filter(|(_, k2)| k2 == k).count();
                    if best.map(|(_, c)| count > c).unwrap_or(true) {
                        best = Some((*k, count));
                    }
                }
                best.map(|(k, _)| k)
            });
            let mut logs = Vec::new();
            for &m in &members {
                let sel = eligible.iter().any(|(e, k)| *e == m && Some(*k) == chosen);
                if let Some(ep) = self.ports.get_mut(m) {
                    if ep.lacp.selected != sel {
                        ep.lacp.selected = sel;
                        if !sel {
                            logs.push(format!("LACP eth{} unselected from lag{}", m, id));
                        }
                    }
                }
            }
            let fb = if fallback && eligible.is_empty() {
                members.iter().copied().find(|&m| self.lacp_enabled_on(m).is_some())
            } else {
                None
            };
            if let Some(l) = self.lags.get_mut(&id) {
                if l.fallback_member != fb {
                    l.fallback_member = fb;
                    logs.push(match fb {
                        Some(m) => format!("LACP lag{} fallback: eth{} forwarding without a partner", id, m),
                        None => format!("LACP lag{} fallback ended", id),
                    });
                }
            }
            for m in logs {
                self.log(Severity::Notice, m);
            }
        }

        // Mux (coupled control) and transmit.
        for p in 0..n {
            let cfg = match self.lacp_enabled_on(p) {
                Some(x) => x,
                None => continue,
            };
            let mut msg = None;
            if let Some(ep) = self.ports.get_mut(p) {
                let lp = &mut ep.lacp;
                match lp.mux {
                    LacpMux::Detached => {
                        if lp.selected {
                            lp.mux = LacpMux::Waiting;
                            lp.wait_until = now + AGGREGATE_WAIT_MS;
                        }
                    }
                    LacpMux::Waiting => {
                        if !lp.selected {
                            lp.mux = LacpMux::Detached;
                        } else if now >= lp.wait_until {
                            lp.mux = LacpMux::Attached;
                            lp.ntt = true;
                        }
                    }
                    LacpMux::Attached => {
                        if !lp.selected {
                            lp.mux = LacpMux::Detached;
                            lp.ntt = true;
                        } else if lp.partner_sync {
                            lp.mux = LacpMux::CollectingDistributing;
                            lp.ntt = true;
                            msg = Some((Severity::Notice, format!("LACP eth{} in lag{} collecting/distributing", p, cfg.0)));
                        }
                    }
                    LacpMux::CollectingDistributing => {
                        if !lp.selected || !lp.partner_sync {
                            lp.mux = if lp.selected { LacpMux::Attached } else { LacpMux::Detached };
                            lp.ntt = true;
                            msg = Some((Severity::Notice, format!("LACP eth{} in lag{} stopped distributing", p, cfg.0)));
                        }
                    }
                }
            }
            if let Some((s, m)) = msg {
                self.log(s, m);
            }
            self.lacp_tx(p, cfg.1, now);
        }
    }

    fn lacp_tx(&mut self, p: usize, mode: LacpMode, now: i64) {
        let actor = self.actor_info(p);
        let (mac, pdu) = {
            let ep = match self.ports.get_mut(p) {
                Some(e) => e,
                None => return,
            };
            let lp = &mut ep.lacp;
            let periodic = mode == LacpMode::Active || lp.partner.state & ST_ACTIVITY != 0;
            if periodic {
                let interval = if lp.partner.state & ST_TIMEOUT != 0 { FAST_PERIODIC_MS } else { SLOW_PERIODIC_MS };
                if now >= lp.next_periodic {
                    lp.ntt = true;
                    lp.next_periodic = now + interval;
                } else if lp.next_periodic > now + interval {
                    lp.next_periodic = now + interval;
                }
            }
            if !lp.ntt {
                return;
            }
            if now - lp.tx_window >= 1000 {
                lp.tx_window = now;
                lp.tx_in_window = 0;
            }
            if lp.tx_in_window >= TX_LIMIT_PER_SEC {
                return;
            }
            lp.tx_in_window += 1;
            lp.ntt = false;
            lp.tx_count += 1;
            let partner = if lp.rx == LacpRx::Defaulted { LacpInfo::default() } else { lp.partner };
            (ep.mac, Lacpdu { actor, partner, collector_max_delay: 0 })
        };
        self.tx(p, lacpdu::build_frame(mac, &pdu));
    }

    pub(crate) fn lacp_deadline(&self, dl: &mut Deadline) {
        for (p, ep) in self.ports.iter().enumerate() {
            if self.lacp_enabled_on(p).is_none() {
                continue;
            }
            let lp = &ep.lacp;
            if matches!(lp.rx, LacpRx::Current | LacpRx::Expired) {
                dl.at(lp.current_until);
            }
            if lp.mux == LacpMux::Waiting {
                dl.at(lp.wait_until);
            }
            if lp.ntt {
                dl.at(lp.tx_window + 1000);
            }
            let mode = ep.cfg.lag.and_then(|id| self.lags.get(&id)).map(|l| l.cfg.lacp_mode);
            if mode == Some(LacpMode::Active) || lp.partner.state & ST_ACTIVITY != 0 {
                dl.at(lp.next_periodic);
            }
        }
    }

    /// Status of a LAG member for show commands / tests.
    pub fn lacp_member_status(&self, p: usize) -> Option<LacpMemberStatus> {
        let ep = self.ports.get(p)?;
        let lag = ep.cfg.lag?;
        let lp = &ep.lacp;
        Some(LacpMemberStatus {
            port: p,
            lag,
            actor: self.actor_info(p),
            partner: lp.partner,
            rx_state: lp.rx,
            mux: lp.mux,
            selected: lp.selected,
            distributing: self.member_distributing(p),
            tx_count: lp.tx_count,
            rx_count: lp.rx_count,
        })
    }

    /// Physical members currently distributing for LAG `id`.
    pub fn lag_distributing_members(&self, id: u16) -> Vec<usize> {
        (0..self.ports.len()).filter(|&m| self.ports.get(m).map(|p| p.cfg.lag == Some(id)).unwrap_or(false) && self.member_distributing(m)).collect()
    }

    /// Is the LAG bridge port operationally up (>= 1 distributing member)?
    pub fn lag_oper_up(&self, id: u16) -> bool {
        self.bport_oper(PortRef::Lag(id))
    }
}

/// LACP status of one member port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LacpMemberStatus {
    pub port: usize,
    pub lag: u16,
    pub actor: LacpInfo,
    pub partner: LacpInfo,
    pub rx_state: LacpRx,
    pub mux: LacpMux,
    pub selected: bool,
    pub distributing: bool,
    pub tx_count: u64,
    pub rx_count: u64,
}
