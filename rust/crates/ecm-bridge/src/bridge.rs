//! The bridge: port hierarchy, data plane, and the event loop that drives
//! the protocol state machines (`stp.rs`, `lacp.rs`, `lldp.rs`).
//!
//! Hierarchy: physical port -> (optional) LAG -> bridge port. STP, VLAN
//! membership, FDB learning and flooding operate on bridge ports
//! ([`PortRef::Eth`] for a standalone L2 port, [`PortRef::Lag`] for an
//! aggregate). [`PortRef::Cpu`] is the switch's own stack (the SVIs).

use crate::fdb::{Fdb, Learn};
use crate::frame::{self, EthInfo, ETHERTYPE_LLDP, ETHERTYPE_SLOW};
use crate::lacp::LacpPort;
use crate::lldp::LldpPort;
use crate::log::{Logger, Severity};
use crate::stp::{StpBridge, StpPort};
use crate::types::*;
use crate::{bpdu, lacpdu};
use ecm_net::types::MacAddr;
use std::collections::{BTreeMap, VecDeque};

/// Bound of the output queue. Drop policy: when full, new outputs are
/// discarded and counted (see [`Bridge::output_drops`]).
pub const OUTPUT_CAPACITY: usize = 8192;

/// Something the caller must do on the bridge's behalf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Transmit `frame` on physical port `port`.
    Tx { port: usize, frame: Vec<u8> },
    /// Deliver an untagged frame to the switch's own SVI on `vlan`.
    Local { vlan: u16, frame: Vec<u8> },
    /// A log event that passed the severity filter (also kept in the ring).
    Log { severity: Severity, msg: String },
}

/// Per-physical-port counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PortCounters {
    pub rx_frames: u64,
    pub rx_errors: u64,
    pub rx_drops: u64,
    pub tx_frames: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct EthPort {
    pub mac: [u8; 6],
    pub link_up: bool,
    pub cfg: EthConfig,
    pub err_disabled: bool,
    pub lacp: LacpPort,
    pub lldp: LldpPort,
    pub stats: PortCounters,
}

#[derive(Clone, Debug)]
pub(crate) struct Lag {
    pub cfg: LagConfig,
    pub err_disabled: bool,
    /// Member brought up alone by `lacp fallback` while no partner exists.
    pub fallback_member: Option<usize>,
}

/// Runtime state of a bridge port.
#[derive(Clone, Debug)]
pub(crate) struct BPort {
    pub oper: bool,
    pub stp: StpPort,
}

/// A pure, sans-IO L2 switch. See the crate docs for the driving model.
pub struct Bridge {
    pub(crate) now: i64,
    pub(crate) bridge_mac: [u8; 6],
    pub(crate) ports: Vec<EthPort>,
    pub(crate) lags: BTreeMap<u16, Lag>,
    pub(crate) vlans: BTreeMap<u16, VlanConfig>,
    pub(crate) fdb: Fdb,
    pub(crate) age_time_secs: u32,
    pub(crate) fast_age_until: i64,
    pub(crate) svis: BTreeMap<u16, ([u8; 4], u8)>,
    pub(crate) stp_cfg: StpConfig,
    pub(crate) stp: StpBridge,
    pub(crate) bports: BTreeMap<PortRef, BPort>,
    pub(crate) lldp_cfg: crate::types::LldpConfig,
    pub(crate) logger: Logger,
    pub(crate) out: VecDeque<Output>,
    pub(crate) out_drops: u64,
    pub(crate) dirty: bool,
}

impl Bridge {
    /// Create a bridge with one port per MAC in `port_macs` (at most
    /// [`MAX_PORTS`]). Every port starts routed (not a bridge member), link
    /// down; call [`Bridge::set_link`] for ports with carrier.
    pub fn new(port_macs: &[MacAddr], bridge_mac: MacAddr, now_ms: i64) -> Self {
        let mut vlans = BTreeMap::new();
        vlans.insert(
            DEFAULT_VID,
            VlanConfig { vid: DEFAULT_VID, name: Some("default".to_string()), description: None, active: true },
        );
        let ports = port_macs
            .iter()
            .take(MAX_PORTS)
            .map(|m| EthPort {
                mac: m.0,
                link_up: false,
                cfg: EthConfig::default(),
                err_disabled: false,
                lacp: LacpPort::new(),
                lldp: LldpPort::new(),
                stats: PortCounters::default(),
            })
            .collect();
        let stp_cfg = StpConfig::default();
        let stp = StpBridge::new(&stp_cfg, bridge_mac.0);
        Bridge {
            now: now_ms,
            bridge_mac: bridge_mac.0,
            ports,
            lags: BTreeMap::new(),
            vlans,
            fdb: Fdb::default(),
            age_time_secs: DEFAULT_AGE_TIME_SECS,
            fast_age_until: i64::MIN,
            svis: BTreeMap::new(),
            stp_cfg,
            stp,
            bports: BTreeMap::new(),
            lldp_cfg: crate::types::LldpConfig::default(),
            logger: Logger::new(),
            out: VecDeque::new(),
            out_drops: 0,
            dirty: true,
        }
    }

    // ------------------------------------------------------------------
    // Driving API
    // ------------------------------------------------------------------

    /// A frame received on physical port `port`. Never panics.
    pub fn handle_frame(&mut self, port: usize, frame: &[u8], now_ms: i64) {
        self.now = now_ms;
        if self.dirty {
            self.run(now_ms);
        }
        let e = {
            let ep = match self.ports.get_mut(port) {
                Some(p) => p,
                None => return,
            };
            if !ep.link_up {
                ep.stats.rx_drops += 1;
                return;
            }
            ep.stats.rx_frames += 1;
            match frame::parse_eth(frame) {
                Some(e) => e,
                None => {
                    ep.stats.rx_errors += 1;
                    return;
                }
            }
        };
        if frame::is_reserved_group(&e.dst) {
            // Link-local control traffic: consumed, never forwarded.
            if self.handle_control(port, &e, frame, now_ms) {
                self.run(now_ms);
            }
            return;
        }
        if e.ethertype == ETHERTYPE_SLOW {
            return; // slow protocols are never forwarded, whatever the DA
        }
        self.forward_data(port, &e, frame);
    }

    /// A frame from the switch's own stack via the SVI on `vlan`. Forwarded
    /// as if it had arrived on an internal CPU port that is an untagged
    /// member of `vlan`.
    pub fn send_local(&mut self, vlan: u16, frame: &[u8], now_ms: i64) {
        self.now = now_ms;
        if self.dirty {
            self.run(now_ms);
        }
        let e = match frame::parse_eth(frame) {
            Some(e) => e,
            None => return,
        };
        if !self.svis.contains_key(&vlan) || !self.vlan_active(vlan) || frame::is_reserved_group(&e.dst) {
            return;
        }
        let owned;
        let frame = if e.tci.is_some() {
            owned = frame::untag(frame);
            &owned[..]
        } else {
            frame
        };
        if !frame::is_multicast(&e.src) && e.src != [0; 6] {
            self.fdb.learn(e.src, vlan, PortRef::Cpu, now_ms);
        }
        if self.is_own_mac(&e.dst) {
            return;
        }
        if frame::is_multicast(&e.dst) {
            self.flood(vlan, PortRef::Cpu, frame, 0);
            return;
        }
        match self.fdb.lookup(&e.dst, vlan).map(|p| self.resolve(p)) {
            Some(PortRef::Cpu) => {}
            Some(out) => self.egress(out, vlan, frame, 0),
            None => self.flood(vlan, PortRef::Cpu, frame, 0),
        }
    }

    /// Carrier change on physical port `port`.
    pub fn set_link(&mut self, port: usize, up: bool, now_ms: i64) {
        self.now = now_ms;
        let (changed, lag) = match self.ports.get_mut(port) {
            Some(ep) if ep.link_up != up => {
                ep.link_up = up;
                if up {
                    // A link flap clears an err-disable (BPDU guard).
                    ep.err_disabled = false;
                }
                (true, ep.cfg.lag)
            }
            _ => (false, None),
        };
        if !changed {
            return;
        }
        if up {
            if let Some(l) = lag.and_then(|id| self.lags.get_mut(&id)) {
                l.err_disabled = false;
            }
        }
        self.log(Severity::Notice, format!("Port eth{} link {}", port, if up { "up" } else { "down" }));
        if !up {
            self.lldp_purge_port(port);
        }
        self.run(now_ms);
    }

    /// Run timers. Returns the next absolute deadline (ms), if any.
    pub fn poll(&mut self, now_ms: i64) -> Option<i64> {
        self.run(now_ms);
        self.next_deadline(now_ms)
    }

    /// Next queued output.
    pub fn pop_output(&mut self) -> Option<Output> {
        self.out.pop_front()
    }

    /// Outputs discarded because the queue was full.
    pub fn output_drops(&self) -> u64 {
        self.out_drops
    }

    /// False for routed ports (the default) and for members of a routed
    /// LAG: the kernel hands those frames to the host stack directly.
    pub fn is_l2_port(&self, port: usize) -> bool {
        let ep = match self.ports.get(port) {
            Some(p) => p,
            None => return false,
        };
        match ep.cfg.lag {
            Some(id) => self.lags.get(&id).map(|l| !l.cfg.mode.is_routed()).unwrap_or(false),
            None => !ep.cfg.mode.is_routed(),
        }
    }

    pub fn bridge_mac(&self) -> MacAddr {
        MacAddr(self.bridge_mac)
    }

    // ------------------------------------------------------------------
    // Internals
    // ------------------------------------------------------------------

    pub(crate) fn run(&mut self, now: i64) {
        self.now = now;
        self.dirty = false;
        self.refresh_bridge_ports();
        self.lacp_run(now);
        self.update_oper(now);
        self.stp_run(now);
        self.lldp_run(now);
        self.fdb_age(now);
    }

    fn next_deadline(&self, now: i64) -> Option<i64> {
        let mut dl = Deadline(None);
        self.stp_deadline(&mut dl);
        self.lacp_deadline(&mut dl);
        self.lldp_deadline(&mut dl);
        if let Some(t) = self.fdb.oldest_dynamic() {
            dl.at(t.saturating_add(self.current_age_ms()).saturating_add(1));
        }
        if self.fast_age_until > now {
            dl.at(self.fast_age_until);
        }
        dl.0.map(|t| t.max(now))
    }

    pub(crate) fn emit(&mut self, o: Output) {
        if self.out.len() >= OUTPUT_CAPACITY {
            self.out_drops += 1;
            return;
        }
        self.out.push_back(o);
    }

    pub(crate) fn tx(&mut self, port: usize, frame: Vec<u8>) {
        if let Some(ep) = self.ports.get_mut(port) {
            if !ep.link_up {
                return;
            }
            ep.stats.tx_frames += 1;
        } else {
            return;
        }
        self.emit(Output::Tx { port, frame });
    }

    pub(crate) fn log(&mut self, severity: Severity, msg: String) {
        if self.logger.record(self.now, severity, &msg) {
            self.emit(Output::Log { severity, msg });
        }
    }

    /// Log with lazy formatting (for hot paths).
    pub(crate) fn log_with(&mut self, severity: Severity, f: impl FnOnce() -> String) {
        if severity.passes(self.logger.severity) {
            self.log(severity, f());
        }
    }

    pub(crate) fn is_own_mac(&self, mac: &[u8; 6]) -> bool {
        *mac == self.bridge_mac || self.ports.iter().any(|p| p.mac == *mac)
    }

    pub(crate) fn vlan_active(&self, vid: u16) -> bool {
        self.vlans.get(&vid).map(|v| v.active).unwrap_or(false)
    }

    pub(crate) fn current_age_ms(&self) -> i64 {
        if self.now < self.fast_age_until {
            self.stp_cfg.forward_delay_secs as i64 * 1000
        } else {
            self.age_time_secs as i64 * 1000
        }
    }

    fn fdb_age(&mut self, now: i64) {
        let age = self.current_age_ms();
        self.fdb.age(now, age);
    }

    /// Map a configured port (e.g. a static entry on a LAG member) to the
    /// bridge port that carries it.
    pub(crate) fn resolve(&self, p: PortRef) -> PortRef {
        match p {
            PortRef::Eth(n) => match self.ports.get(n).and_then(|e| e.cfg.lag) {
                Some(id) => PortRef::Lag(id),
                None => p,
            },
            _ => p,
        }
    }

    pub(crate) fn bp_mode(&self, bp: PortRef) -> Option<&PortMode> {
        match bp {
            PortRef::Eth(n) => self.ports.get(n).map(|p| &p.cfg.mode),
            PortRef::Lag(id) => self.lags.get(&id).map(|l| &l.cfg.mode),
            PortRef::Cpu => None,
        }
    }

    pub(crate) fn bp_stp_cfg(&self, bp: PortRef) -> Option<&StpPortConfig> {
        match bp {
            PortRef::Eth(n) => self.ports.get(n).map(|p| &p.cfg.stp),
            PortRef::Lag(id) => self.lags.get(&id).map(|l| &l.cfg.stp),
            PortRef::Cpu => None,
        }
    }

    /// Operational state (before STP) of a bridge port.
    fn compute_oper(&self, bp: PortRef) -> bool {
        match bp {
            PortRef::Eth(n) => self.ports.get(n).map(|p| p.link_up && p.cfg.admin_up && !p.err_disabled).unwrap_or(false),
            PortRef::Lag(id) => match self.lags.get(&id) {
                Some(l) if l.cfg.admin_up && !l.err_disabled => {
                    (0..self.ports.len()).any(|m| self.ports.get(m).map(|p| p.cfg.lag == Some(id)).unwrap_or(false) && self.member_distributing(m))
                }
                _ => false,
            },
            PortRef::Cpu => true,
        }
    }

    pub(crate) fn bport_oper(&self, bp: PortRef) -> bool {
        self.bports.get(&bp).map(|b| b.oper).unwrap_or(false)
    }

    /// Bridge port that carries data frames arriving on physical `port`.
    pub(crate) fn ingress_bport(&self, port: usize) -> Option<PortRef> {
        let ep = self.ports.get(port)?;
        match ep.cfg.lag {
            Some(id) => {
                let lag = self.lags.get(&id)?;
                if lag.cfg.mode.is_routed() || !self.member_distributing(port) {
                    None
                } else {
                    Some(PortRef::Lag(id))
                }
            }
            None if ep.cfg.mode.is_routed() => None,
            None => Some(PortRef::Eth(port)),
        }
    }

    /// Keep `bports` in sync with the configuration.
    fn refresh_bridge_ports(&mut self) {
        let mut want: Vec<PortRef> = Vec::new();
        for (i, p) in self.ports.iter().enumerate() {
            if p.cfg.lag.is_none() && !p.cfg.mode.is_routed() {
                want.push(PortRef::Eth(i));
            }
        }
        for (id, l) in &self.lags {
            if !l.cfg.mode.is_routed() {
                want.push(PortRef::Lag(*id));
            }
        }
        let stale: Vec<PortRef> = self.bports.keys().filter(|k| !want.contains(k)).copied().collect();
        for bp in stale {
            self.bports.remove(&bp);
            self.fdb.flush_port(bp);
        }
        for bp in want {
            if !self.bports.contains_key(&bp) {
                let id = crate::stp::port_number(bp);
                self.bports.insert(bp, BPort { oper: false, stp: StpPort::new(id) });
            }
        }
    }

    fn update_oper(&mut self, _now: i64) {
        let keys: Vec<PortRef> = self.bports.keys().copied().collect();
        for bp in keys {
            let new = self.compute_oper(bp);
            let old = match self.bports.get_mut(&bp) {
                Some(b) => std::mem::replace(&mut b.oper, new),
                None => continue,
            };
            if old != new {
                if !new {
                    self.fdb.flush_port(bp);
                }
                if let PortRef::Lag(_) = bp {
                    self.log(Severity::Notice, format!("Interface {} {}", bp, if new { "up" } else { "down" }));
                }
            }
        }
    }

    pub(crate) fn err_disable(&mut self, bp: PortRef, why: &str) {
        match bp {
            PortRef::Eth(n) => {
                if let Some(p) = self.ports.get_mut(n) {
                    p.err_disabled = true;
                }
            }
            PortRef::Lag(id) => {
                if let Some(l) = self.lags.get_mut(&id) {
                    l.err_disabled = true;
                }
            }
            PortRef::Cpu => return,
        }
        self.log(Severity::Error, format!("{} on {} - port err-disabled", why, bp));
        self.dirty = true;
    }

    /// Consume a frame addressed to 01:80:c2:00:00:0x. Returns true when
    /// protocol state changed and the state machines must run.
    fn handle_control(&mut self, port: usize, e: &EthInfo, f: &[u8], now: i64) -> bool {
        match e.dst[5] {
            0x00 => match bpdu::parse_frame(f) {
                Some(b) => self.rx_bpdu(port, b, now),
                None => {
                    if let Some(p) = self.ports.get_mut(port) {
                        p.stats.rx_errors += 1;
                    }
                    false
                }
            },
            0x02 if e.ethertype == ETHERTYPE_SLOW => match lacpdu::parse_frame(f) {
                Some(pdu) => self.lacp_rx(port, pdu, now),
                None => false, // marker PDUs and garbage are dropped
            },
            0x0e if e.ethertype == ETHERTYPE_LLDP => {
                let payload = f.get(e.payload_off..).unwrap_or(&[]);
                self.lldp_rx(port, payload, now);
                false
            }
            _ => false,
        }
    }

    fn rx_bpdu(&mut self, port: usize, b: bpdu::Bpdu, now: i64) -> bool {
        let bp = match self.ingress_bport(port) {
            Some(bp) => bp,
            None => return false,
        };
        let (guard, enabled) = match self.bp_stp_cfg(bp) {
            Some(c) => (c.bpdu_guard, c.enabled),
            None => return false,
        };
        if guard {
            if self.bport_oper(bp) {
                self.err_disable(bp, "STP bpdu-guard: BPDU received");
                return true;
            }
            return false;
        }
        if !self.stp_cfg.enabled || !enabled || !self.bport_oper(bp) {
            return false;
        }
        self.stp_rx(bp, b, now)
    }

    fn forward_data(&mut self, port: usize, e: &EthInfo, f: &[u8]) {
        let bp = match self.ingress_bport(port) {
            Some(bp) if self.bport_oper(bp) => bp,
            _ => return self.drop_rx(port),
        };
        let vlan = match self.ingress_vlan(bp, e) {
            Some(v) if self.vlan_active(v) => v,
            _ => return self.drop_rx(port),
        };
        if self.is_own_mac(&e.src) {
            return self.drop_rx(port); // our own frame looped back
        }
        let now = self.now;
        if self.bp_learning(bp) && !frame::is_multicast(&e.src) && e.src != [0; 6] {
            match self.fdb.learn(e.src, vlan, bp, now) {
                Learn::New => {
                    let src = e.src;
                    self.log_with(Severity::Debug, || format!("MAC learn {} VLAN {} port {}", fmt_mac(&src), vlan, bp));
                }
                Learn::Moved { from } => {
                    let src = e.src;
                    self.log_with(Severity::Notice, || format!("MAC move {} VLAN {} {} -> {}", fmt_mac(&src), vlan, from, bp));
                }
                _ => {}
            }
        }
        if !self.bp_forwarding(bp) {
            return;
        }
        let pcp = e.pcp();
        let has_svi = self.svis.contains_key(&vlan);
        if self.is_own_mac(&e.dst) {
            if has_svi {
                self.emit(Output::Local { vlan, frame: frame::untag(f) });
            }
            return;
        }
        if frame::is_multicast(&e.dst) {
            if has_svi {
                self.emit(Output::Local { vlan, frame: frame::untag(f) });
            }
            self.flood(vlan, bp, f, pcp);
            return;
        }
        match self.fdb.lookup(&e.dst, vlan).map(|p| self.resolve(p)) {
            Some(PortRef::Cpu) => {
                if has_svi {
                    self.emit(Output::Local { vlan, frame: frame::untag(f) });
                }
            }
            Some(out) if out == bp => {} // destination is on the ingress segment
            Some(out) => self.egress(out, vlan, f, pcp),
            None => self.flood(vlan, bp, f, pcp),
        }
    }

    fn drop_rx(&mut self, port: usize) {
        if let Some(p) = self.ports.get_mut(port) {
            p.stats.rx_drops += 1;
        }
    }

    /// Classify an ingress frame into a VLAN per the bridge port's mode.
    pub(crate) fn ingress_vlan(&self, bp: PortRef, e: &EthInfo) -> Option<u16> {
        match self.bp_mode(bp)? {
            PortMode::Routed => None,
            PortMode::Access { vid } => match e.vid() {
                None => Some(*vid),
                Some(_) => None,
            },
            PortMode::Trunk { native, native_tag, allowed } => match e.vid() {
                None if *native_tag => None,
                None => allowed.allows(*native).then_some(*native),
                Some(v) => allowed.allows(v).then_some(v),
            },
        }
    }

    /// Membership of bridge port `bp` in `vid`: Some(tagged) or None.
    pub(crate) fn membership(&self, bp: PortRef, vid: u16) -> Option<bool> {
        match self.bp_mode(bp)? {
            PortMode::Routed => None,
            PortMode::Access { vid: a } => (*a == vid).then_some(false),
            PortMode::Trunk { native, native_tag, allowed } => {
                if !allowed.allows(vid) {
                    return None;
                }
                if matches!(allowed, AllowedList::All) && !self.vlans.contains_key(&vid) {
                    return None;
                }
                Some(!(vid == *native && !*native_tag))
            }
        }
    }

    fn egress(&mut self, bp: PortRef, vlan: u16, f: &[u8], pcp: u8) {
        let tagged = match self.membership(bp, vlan) {
            Some(t) => t,
            None => return,
        };
        if !self.bp_forwarding(bp) {
            return;
        }
        let out = if tagged {
            match frame::tag(f, vlan, pcp) {
                Some(x) => x,
                None => return,
            }
        } else {
            frame::untag(f)
        };
        match bp {
            PortRef::Eth(p) => self.tx(p, out),
            PortRef::Lag(id) => {
                if let Some(m) = self.lag_select_member(id, &out) {
                    self.tx(m, out);
                }
            }
            PortRef::Cpu => {}
        }
    }

    fn flood(&mut self, vlan: u16, except: PortRef, f: &[u8], pcp: u8) {
        let targets: Vec<PortRef> = self.bports.iter().filter(|(k, b)| **k != except && b.oper).map(|(k, _)| *k).collect();
        for bp in targets {
            self.egress(bp, vlan, f, pcp);
        }
    }

    /// Transmit a link-local control frame on a bridge port (on one
    /// distributing member for a LAG).
    pub(crate) fn tx_control(&mut self, bp: PortRef, build: impl FnOnce([u8; 6]) -> Vec<u8>) -> bool {
        let phys = match bp {
            PortRef::Eth(p) => Some(p),
            PortRef::Lag(id) => (0..self.ports.len()).find(|&m| self.ports.get(m).map(|p| p.cfg.lag == Some(id)).unwrap_or(false) && self.member_distributing(m)),
            PortRef::Cpu => None,
        };
        match phys.and_then(|p| self.ports.get(p).map(|e| (p, e.mac))) {
            Some((p, mac)) => {
                let f = build(mac);
                self.tx(p, f);
                true
            }
            None => false,
        }
    }
}

/// Earliest-deadline accumulator.
pub(crate) struct Deadline(pub Option<i64>);

impl Deadline {
    pub fn at(&mut self, t: i64) {
        self.0 = Some(match self.0 {
            Some(x) => x.min(t),
            None => t,
        });
    }
}

/// Dotted-quad rendering (kept local: ecm-net is used for its types only).
pub fn fmt_ip(ip: &ecm_net::types::Ipv4Addr) -> String {
    let o = ip.0;
    format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3])
}

pub fn fmt_mac(m: &[u8; 6]) -> String {
    format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", m[0], m[1], m[2], m[3], m[4], m[5])
}
