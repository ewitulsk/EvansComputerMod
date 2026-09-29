//! Typed configuration and introspection API. The CLI is a thin layer over
//! these methods; tests and the kernel may call them directly.
//!
//! Setters validate, mutate configuration and mark the bridge dirty; the
//! resulting protocol work happens on the next `poll`/`handle_frame`/
//! `send_local`/`set_link` call (the CLI calls `poll` after every command).

use crate::bridge::{fmt_mac, Bridge, Lag, PortCounters};
use crate::frame::HashMode;
use crate::lacp::LacpPort;
use crate::log::{LogEntry, Severity, MAX_REMOTES};
use crate::stp::{InfoIs, StpPortState, StpRole};
use crate::types::*;
use ecm_net::types::{Ipv4Addr, MacAddr};

/// One FDB entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FdbEntryInfo {
    pub mac: [u8; 6],
    pub vlan: u16,
    pub port: PortRef,
    pub is_static: bool,
    pub last_seen_ms: i64,
    pub prev_port: Option<PortRef>,
    pub move_count: u32,
    /// -1 if the entry never moved.
    pub last_move_ms: i64,
}

impl FdbEntryInfo {
    pub fn mac_string(&self) -> String {
        fmt_mac(&self.mac)
    }
}

/// Filter for [`Bridge::clear_dynamic`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FdbFilter {
    pub vlan: Option<u16>,
    pub port: Option<PortRef>,
    pub mac: Option<[u8; 6]>,
}

/// Bridge-wide spanning tree status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StpStatus {
    pub enabled: bool,
    pub bridge_id: u64,
    pub root_id: u64,
    pub root_port: Option<PortRef>,
    pub root_path_cost: u32,
    pub is_root: bool,
    pub topology_changes: u64,
    pub last_tc_ms: Option<i64>,
}

/// Spanning tree status of one bridge port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StpPortStatus {
    pub port: PortRef,
    pub port_id: u16,
    pub role: StpRole,
    pub state: StpPortState,
    pub cost: u32,
    pub priority: u8,
    /// Designated bridge of the segment (our own ID when we are designated).
    pub designated_bridge: Option<u64>,
    pub designated_root: Option<u64>,
    pub oper_edge: bool,
    pub root_inconsistent: bool,
    pub err_disabled: bool,
    /// False when STP does not control the port (globally or per-port off).
    pub stp_active: bool,
    pub oper_up: bool,
    pub forwarding: bool,
    pub rx_bpdus: u64,
    pub tx_bpdus: u64,
}

fn check_vid(vid: u16) -> Result<(), BridgeError> {
    if (MIN_VID..=MAX_VID).contains(&vid) {
        Ok(())
    } else {
        Err(BridgeError::BadVlan)
    }
}

fn check_mode(m: &PortMode) -> Result<(), BridgeError> {
    match m {
        PortMode::Routed => Ok(()),
        PortMode::Access { vid } => check_vid(*vid),
        PortMode::Trunk { native, allowed, .. } => {
            check_vid(*native)?;
            if let AllowedList::Some(s) = allowed {
                for v in s {
                    check_vid(*v)?;
                }
            }
            Ok(())
        }
    }
}

fn range(what: &'static str, v: u32, min: u32, max: u32) -> Result<(), BridgeError> {
    if (min..=max).contains(&v) {
        Ok(())
    } else {
        Err(BridgeError::OutOfRange { what, min, max })
    }
}

impl Bridge {
    // ---------------- ports ----------------

    pub fn port_count(&self) -> usize {
        self.ports.len()
    }

    pub fn port_mac(&self, port: usize) -> Option<MacAddr> {
        self.ports.get(port).map(|p| MacAddr(p.mac))
    }

    pub fn link_up(&self, port: usize) -> bool {
        self.ports.get(port).map(|p| p.link_up).unwrap_or(false)
    }

    pub fn port_counters(&self, port: usize) -> Option<PortCounters> {
        self.ports.get(port).map(|p| p.stats)
    }

    pub fn eth_config(&self, port: usize) -> Option<&EthConfig> {
        self.ports.get(port).map(|p| &p.cfg)
    }

    fn check_iface(&self, i: PortRef) -> Result<(), BridgeError> {
        match i {
            PortRef::Eth(n) if n < self.ports.len() => Ok(()),
            PortRef::Lag(id) if self.lags.contains_key(&id) => Ok(()),
            PortRef::Lag(id) => Err(BridgeError::LagNotFound(id)),
            _ => Err(BridgeError::BadPort),
        }
    }

    /// Mode of an interface (`Eth` or `Lag`).
    pub fn port_mode(&self, iface: PortRef) -> Option<&PortMode> {
        self.bp_mode(iface)
    }

    /// Set the forwarding mode of an interface. VLANs need not exist (frames
    /// in a missing or shutdown VLAN are dropped).
    pub fn set_port_mode(&mut self, iface: PortRef, mode: PortMode) -> Result<(), BridgeError> {
        self.check_iface(iface)?;
        check_mode(&mode)?;
        match iface {
            PortRef::Eth(n) => {
                if let Some(p) = self.ports.get_mut(n) {
                    p.cfg.mode = mode;
                }
            }
            PortRef::Lag(id) => {
                if let Some(l) = self.lags.get_mut(&id) {
                    l.cfg.mode = mode;
                }
            }
            PortRef::Cpu => return Err(BridgeError::BadPort),
        }
        self.fdb.flush_port(iface);
        self.dirty = true;
        Ok(())
    }

    /// Administrative up/down. `up = true` also clears an err-disable.
    pub fn set_admin_up(&mut self, iface: PortRef, up: bool) -> Result<(), BridgeError> {
        self.check_iface(iface)?;
        let cleared = match iface {
            PortRef::Eth(n) => self.ports.get_mut(n).map(|p| {
                p.cfg.admin_up = up;
                up && std::mem::replace(&mut p.err_disabled, false)
            }),
            PortRef::Lag(id) => self.lags.get_mut(&id).map(|l| {
                l.cfg.admin_up = up;
                up && std::mem::replace(&mut l.err_disabled, false)
            }),
            PortRef::Cpu => None,
        };
        if cleared == Some(true) {
            self.log(Severity::Notice, format!("{} err-disable cleared", iface));
        }
        self.dirty = true;
        Ok(())
    }

    pub fn is_err_disabled(&self, iface: PortRef) -> bool {
        match iface {
            PortRef::Eth(n) => self.ports.get(n).map(|p| p.err_disabled).unwrap_or(false),
            PortRef::Lag(id) => self.lags.get(&id).map(|l| l.err_disabled).unwrap_or(false),
            PortRef::Cpu => false,
        }
    }

    /// Bridge ports currently in the bridge (standalone L2 ports and L2 LAGs).
    pub fn bridge_ports(&self) -> Vec<PortRef> {
        let mut v = Vec::new();
        for (i, p) in self.ports.iter().enumerate() {
            if p.cfg.lag.is_none() && !p.cfg.mode.is_routed() {
                v.push(PortRef::Eth(i));
            }
        }
        for (id, l) in &self.lags {
            if !l.cfg.mode.is_routed() {
                v.push(PortRef::Lag(*id));
            }
        }
        v
    }

    /// Operationally up (before STP)?
    pub fn bridge_port_oper_up(&self, bp: PortRef) -> bool {
        self.bport_oper(bp)
    }

    /// Would a data frame be forwarded out of `bp` (link, LACP and STP)?
    pub fn bridge_port_forwarding(&self, bp: PortRef) -> bool {
        self.bp_forwarding(bp)
    }

    // ---------------- VLANs ----------------

    /// Create a VLAN (inactive) if absent. Ok(true) when created.
    pub fn create_vlan(&mut self, vid: u16) -> Result<bool, BridgeError> {
        check_vid(vid)?;
        if self.vlans.contains_key(&vid) {
            return Ok(false);
        }
        self.vlans.insert(vid, VlanConfig { vid, name: None, description: None, active: false });
        self.log(Severity::Info, format!("VLAN {} created", vid));
        Ok(true)
    }

    /// Delete a VLAN; refused while any interface or an SVI references it.
    pub fn delete_vlan(&mut self, vid: u16) -> Result<(), BridgeError> {
        if vid == DEFAULT_VID {
            return Err(BridgeError::DefaultVlan);
        }
        if !self.vlans.contains_key(&vid) {
            return Err(BridgeError::VlanNotFound(vid));
        }
        if let Some(port) = self.vlan_in_use_on(vid) {
            return Err(BridgeError::VlanInUse { vid, port });
        }
        if self.svis.contains_key(&vid) {
            return Err(BridgeError::VlanHasSvi(vid));
        }
        self.vlans.remove(&vid);
        self.fdb.remove_all_where(|v, _, _| v == vid);
        self.log(Severity::Info, format!("VLAN {} deleted", vid));
        Ok(())
    }

    fn vlan_in_use_on(&self, vid: u16) -> Option<PortRef> {
        let uses = |m: &PortMode| match m {
            PortMode::Routed => false,
            PortMode::Access { vid: a } => *a == vid,
            PortMode::Trunk { native, allowed, .. } => *native == vid || matches!(allowed, AllowedList::Some(s) if s.contains(&vid)),
        };
        for (i, p) in self.ports.iter().enumerate() {
            if uses(&p.cfg.mode) {
                return Some(PortRef::Eth(i));
            }
        }
        self.lags.iter().find(|(_, l)| uses(&l.cfg.mode)).map(|(id, _)| PortRef::Lag(*id))
    }

    pub fn set_vlan_name(&mut self, vid: u16, name: Option<String>) -> Result<(), BridgeError> {
        let v = self.vlans.get_mut(&vid).ok_or(BridgeError::VlanNotFound(vid))?;
        v.name = name.map(|s| s.chars().take(64).collect());
        Ok(())
    }

    pub fn set_vlan_description(&mut self, vid: u16, d: Option<String>) -> Result<(), BridgeError> {
        let v = self.vlans.get_mut(&vid).ok_or(BridgeError::VlanNotFound(vid))?;
        v.description = d.map(|s| s.chars().take(128).collect());
        Ok(())
    }

    pub fn set_vlan_active(&mut self, vid: u16, active: bool) -> Result<(), BridgeError> {
        if vid == DEFAULT_VID && !active {
            return Err(BridgeError::DefaultVlan);
        }
        let v = self.vlans.get_mut(&vid).ok_or(BridgeError::VlanNotFound(vid))?;
        v.active = active;
        if !active {
            self.fdb.remove_dynamic_where(|v, _, _| v == vid);
        }
        Ok(())
    }

    pub fn vlan(&self, vid: u16) -> Option<&VlanConfig> {
        self.vlans.get(&vid)
    }

    pub fn vlans(&self) -> impl Iterator<Item = &VlanConfig> {
        self.vlans.values()
    }

    /// (untagged members, tagged members) of `vid` among bridge ports.
    pub fn vlan_members(&self, vid: u16) -> (Vec<PortRef>, Vec<PortRef>) {
        let mut u = Vec::new();
        let mut t = Vec::new();
        for bp in self.bridge_ports() {
            match self.membership(bp, vid) {
                Some(false) => u.push(bp),
                Some(true) => t.push(bp),
                None => {}
            }
        }
        (u, t)
    }

    // ---------------- FDB ----------------

    pub fn age_time_secs(&self) -> u32 {
        self.age_time_secs
    }

    pub fn set_age_time_secs(&mut self, secs: u32) -> Result<(), BridgeError> {
        range("age-time", secs, MIN_AGE_TIME_SECS, MAX_AGE_TIME_SECS)?;
        self.age_time_secs = secs;
        Ok(())
    }

    fn check_static_port(&self, port: PortRef) -> Result<(), BridgeError> {
        match port {
            PortRef::Eth(n) if n < self.ports.len() => Ok(()),
            PortRef::Lag(id) if (MIN_LAG_ID..=MAX_LAG_ID).contains(&id) => Ok(()),
            _ => Err(BridgeError::BadPort),
        }
    }

    /// Add (or convert an entry to) a static entry. Ok(true) if an existing
    /// entry was replaced.
    pub fn add_static_mac(&mut self, mac: MacAddr, vlan: u16, port: PortRef) -> Result<bool, BridgeError> {
        check_vid(vlan)?;
        self.check_static_port(port)?;
        self.fdb.add_static(mac.0, vlan, port, self.now).map_err(|_| BridgeError::FdbFull)
    }

    pub fn remove_static_mac(&mut self, mac: MacAddr, vlan: u16, port: PortRef) -> Result<(), BridgeError> {
        if self.fdb.remove_static(mac.0, vlan, port) {
            Ok(())
        } else {
            Err(BridgeError::NoSuchEntry)
        }
    }

    pub fn fdb_entries(&self) -> Vec<FdbEntryInfo> {
        self.fdb
            .iter()
            .map(|(vlan, mac, e)| FdbEntryInfo {
                mac,
                vlan,
                port: e.port,
                is_static: e.is_static,
                last_seen_ms: e.last_seen_ms,
                prev_port: e.prev_port,
                move_count: e.move_count,
                last_move_ms: e.last_move_ms,
            })
            .collect()
    }

    pub fn fdb_lookup(&self, mac: MacAddr, vlan: u16) -> Option<PortRef> {
        self.fdb.lookup(&mac.0, vlan)
    }

    pub fn fdb_len(&self) -> usize {
        self.fdb.len()
    }

    /// Remove dynamic entries matching the filter; returns how many.
    pub fn clear_dynamic(&mut self, f: FdbFilter) -> usize {
        self.fdb.remove_dynamic_where(|v, m, e| {
            f.vlan.map(|x| x == v).unwrap_or(true) && f.port.map(|x| x == e.port).unwrap_or(true) && f.mac.map(|x| x == *m).unwrap_or(true)
        })
    }

    // ---------------- LAGs ----------------

    /// Create a LAG if absent. Ok(true) when created.
    pub fn create_lag(&mut self, id: u16) -> Result<bool, BridgeError> {
        if !(MIN_LAG_ID..=MAX_LAG_ID).contains(&id) {
            return Err(BridgeError::BadLagId);
        }
        if self.lags.contains_key(&id) {
            return Ok(false);
        }
        self.lags.insert(id, Lag { cfg: LagConfig::new(id), err_disabled: false, fallback_member: None });
        self.log(Severity::Info, format!("LAG {} created", id));
        self.dirty = true;
        Ok(true)
    }

    /// Delete a LAG, releasing its members (they keep their own config).
    pub fn delete_lag(&mut self, id: u16) -> Result<(), BridgeError> {
        if self.lags.remove(&id).is_none() {
            return Err(BridgeError::LagNotFound(id));
        }
        for p in self.ports.iter_mut() {
            if p.cfg.lag == Some(id) {
                p.cfg.lag = None;
                p.lacp.reset();
            }
        }
        self.fdb.remove_all_where(|_, _, e| e.port == PortRef::Lag(id));
        self.log(Severity::Info, format!("LAG {} removed", id));
        self.dirty = true;
        Ok(())
    }

    /// Join `port` to LAG `lag` (leaving any previous LAG), or leave (None).
    pub fn set_lag_membership(&mut self, port: usize, lag: Option<u16>) -> Result<(), BridgeError> {
        if port >= self.ports.len() {
            return Err(BridgeError::BadPort);
        }
        if let Some(id) = lag {
            if !self.lags.contains_key(&id) {
                return Err(BridgeError::LagNotFound(id));
            }
            let already = self.ports.get(port).map(|p| p.cfg.lag == Some(id)).unwrap_or(false);
            let count = self.ports.iter().filter(|p| p.cfg.lag == Some(id)).count();
            if !already && count >= MAX_LAG_MEMBERS {
                return Err(BridgeError::LagFull);
            }
            if already {
                return Ok(());
            }
        }
        let old = self.ports.get(port).and_then(|p| p.cfg.lag);
        if let Some(p) = self.ports.get_mut(port) {
            p.cfg.lag = lag;
            p.lacp = LacpPort::new();
        }
        self.fdb.flush_port(PortRef::Eth(port));
        match (old, lag) {
            (_, Some(id)) => self.log(Severity::Info, format!("LAG {}: eth{} joined", id, port)),
            (Some(id), None) => self.log(Severity::Info, format!("LAG {}: eth{} left", id, port)),
            _ => {}
        }
        self.dirty = true;
        Ok(())
    }

    fn lag_mut(&mut self, id: u16) -> Result<&mut Lag, BridgeError> {
        self.dirty = true;
        self.lags.get_mut(&id).ok_or(BridgeError::LagNotFound(id))
    }

    pub fn set_lacp_mode(&mut self, id: u16, mode: LacpMode) -> Result<(), BridgeError> {
        self.lag_mut(id)?.cfg.lacp_mode = mode;
        for p in self.ports.iter_mut().filter(|p| p.cfg.lag == Some(id)) {
            p.lacp.reset();
        }
        Ok(())
    }

    pub fn set_lacp_rate(&mut self, id: u16, rate: LacpRate) -> Result<(), BridgeError> {
        self.lag_mut(id)?.cfg.lacp_rate = rate;
        for p in self.ports.iter_mut().filter(|p| p.cfg.lag == Some(id)) {
            p.lacp.ntt = true;
        }
        Ok(())
    }

    pub fn set_lag_hash(&mut self, id: u16, hash: HashMode) -> Result<(), BridgeError> {
        self.lag_mut(id)?.cfg.hash = hash;
        Ok(())
    }

    pub fn set_lag_fallback(&mut self, id: u16, on: bool) -> Result<(), BridgeError> {
        self.lag_mut(id)?.cfg.fallback = on;
        Ok(())
    }

    pub fn lag_config(&self, id: u16) -> Option<&LagConfig> {
        self.lags.get(&id).map(|l| &l.cfg)
    }

    pub fn lag_ids(&self) -> Vec<u16> {
        self.lags.keys().copied().collect()
    }

    pub fn lag_members(&self, id: u16) -> Vec<usize> {
        self.ports.iter().enumerate().filter(|(_, p)| p.cfg.lag == Some(id)).map(|(i, _)| i).collect()
    }

    // ---------------- STP ----------------

    pub fn stp_config(&self) -> &StpConfig {
        &self.stp_cfg
    }

    pub fn set_stp_enabled(&mut self, on: bool) {
        self.stp_cfg.enabled = on;
        self.dirty = true;
    }

    pub fn set_stp_priority(&mut self, prio: u16) -> Result<(), BridgeError> {
        if prio % 4096 != 0 || prio > 61440 {
            return Err(BridgeError::Invalid("Priority must be 0..61440 in steps of 4096"));
        }
        self.stp_cfg.priority = prio;
        self.dirty = true;
        Ok(())
    }

    pub fn set_stp_hello(&mut self, s: u32) -> Result<(), BridgeError> {
        range("hello-time", s, 1, 10)?;
        self.stp_cfg.hello_secs = s;
        self.dirty = true;
        Ok(())
    }

    pub fn set_stp_forward_delay(&mut self, s: u32) -> Result<(), BridgeError> {
        range("forward-delay", s, 4, 30)?;
        self.stp_cfg.forward_delay_secs = s;
        self.dirty = true;
        Ok(())
    }

    pub fn set_stp_max_age(&mut self, s: u32) -> Result<(), BridgeError> {
        range("max-age", s, 6, 40)?;
        self.stp_cfg.max_age_secs = s;
        self.dirty = true;
        Ok(())
    }

    pub fn set_stp_config_name(&mut self, name: String) {
        self.stp_cfg.config_name = name.chars().take(32).collect();
    }

    pub fn set_stp_config_revision(&mut self, rev: u16) {
        self.stp_cfg.config_revision = rev;
    }

    pub fn port_stp_config(&self, iface: PortRef) -> Option<&StpPortConfig> {
        self.bp_stp_cfg(iface)
    }

    /// Replace the STP configuration of an interface.
    pub fn set_port_stp(&mut self, iface: PortRef, c: StpPortConfig) -> Result<(), BridgeError> {
        self.check_iface(iface)?;
        if c.priority % 16 != 0 || c.priority > 240 {
            return Err(BridgeError::Invalid("Port-priority must be 0..240 in steps of 16"));
        }
        if c.cost == 0 || c.cost > 200_000_000 {
            return Err(BridgeError::OutOfRange { what: "cost", min: 1, max: 200_000_000 });
        }
        match iface {
            PortRef::Eth(n) => {
                if let Some(p) = self.ports.get_mut(n) {
                    p.cfg.stp = c;
                }
            }
            PortRef::Lag(id) => {
                if let Some(l) = self.lags.get_mut(&id) {
                    l.cfg.stp = c;
                }
            }
            PortRef::Cpu => {}
        }
        self.dirty = true;
        Ok(())
    }

    pub fn stp_status(&self) -> StpStatus {
        StpStatus {
            enabled: self.stp_cfg.enabled,
            bridge_id: crate::stp::make_bid(self.stp_cfg.priority, self.bridge_mac),
            root_id: if self.stp_cfg.enabled { self.stp.root_prio.root } else { crate::stp::make_bid(self.stp_cfg.priority, self.bridge_mac) },
            root_port: if self.stp_cfg.enabled { self.stp.root_port } else { None },
            root_path_cost: if self.stp_cfg.enabled { self.stp.root_prio.cost } else { 0 },
            is_root: !self.stp_cfg.enabled || self.stp.root_port.is_none(),
            topology_changes: self.stp.tc_count,
            last_tc_ms: self.stp.last_tc_ms,
        }
    }

    pub fn stp_port_status(&self, bp: PortRef) -> Option<StpPortStatus> {
        let b = self.bports.get(&bp)?;
        let cfg = self.bp_stp_cfg(bp)?;
        let p = &b.stp;
        let known = matches!(p.info, InfoIs::Mine | InfoIs::Received);
        Some(StpPortStatus {
            port: bp,
            port_id: ((cfg.priority as u16 >> 4) << 12) | (p.port_num & 0x0fff),
            role: p.role,
            state: p.state,
            cost: cfg.cost,
            priority: cfg.priority,
            designated_bridge: known.then_some(p.prio.dbridge),
            designated_root: known.then_some(p.prio.root),
            oper_edge: p.oper_edge,
            root_inconsistent: p.root_inconsistent,
            err_disabled: self.is_err_disabled(bp),
            stp_active: self.stp_active(bp),
            oper_up: b.oper,
            forwarding: self.bp_forwarding(bp),
            rx_bpdus: p.rx_bpdus,
            tx_bpdus: p.tx_bpdus,
        })
    }

    // ---------------- LLDP ----------------

    pub fn lldp_config(&self) -> &LldpConfig {
        &self.lldp_cfg
    }

    pub fn set_lldp_enabled(&mut self, on: bool) {
        self.lldp_cfg.enabled = on;
    }

    pub fn set_lldp_timer(&mut self, s: u32) -> Result<(), BridgeError> {
        range("timer", s, 5, 32768)?;
        self.lldp_cfg.timer_secs = s;
        self.lldp_local_change();
        Ok(())
    }

    pub fn set_lldp_holdtime(&mut self, m: u32) -> Result<(), BridgeError> {
        range("holdtime", m, 2, 10)?;
        self.lldp_cfg.holdtime = m;
        self.lldp_local_change();
        Ok(())
    }

    pub fn set_lldp_reinit(&mut self, s: u32) -> Result<(), BridgeError> {
        range("reinit", s, 1, 10)?;
        self.lldp_cfg.reinit_secs = s;
        Ok(())
    }

    pub fn set_lldp_txdelay(&mut self, s: u32) -> Result<(), BridgeError> {
        range("txdelay", s, 1, 8192)?;
        self.lldp_cfg.txdelay_secs = s;
        Ok(())
    }

    pub fn set_lldp_mgmt_ipv4(&mut self, ip: Option<Ipv4Addr>) {
        self.lldp_cfg.mgmt_ipv4 = ip.map(|i| i.0);
        self.lldp_local_change();
    }

    pub fn set_lldp_tlv(&mut self, t: LldpTlv, on: bool) {
        if let Some(x) = self.lldp_cfg.tlvs.get_mut(t as usize) {
            *x = on;
        }
        self.lldp_local_change();
    }

    /// Advertised system name (not part of the CLI config).
    pub fn set_system_name(&mut self, name: &str) {
        self.lldp_cfg.sys_name = name.chars().take(64).collect();
        self.lldp_local_change();
    }

    pub fn set_lldp_port(&mut self, port: usize, tx: Option<bool>, rx: Option<bool>) -> Result<(), BridgeError> {
        let p = self.ports.get_mut(port).ok_or(BridgeError::BadPort)?;
        if let Some(t) = tx {
            p.cfg.lldp_tx = t;
        }
        if let Some(r) = rx {
            p.cfg.lldp_rx = r;
        }
        Ok(())
    }

    // ---------------- logging ----------------

    pub fn log_severity(&self) -> Severity {
        self.logger.severity
    }

    pub fn set_log_severity(&mut self, s: Severity) {
        self.logger.severity = s;
    }

    /// `logging console`: the kernel should print `Output::Log` events.
    pub fn log_console(&self) -> bool {
        self.logger.console
    }

    pub fn set_log_console(&mut self, on: bool) {
        self.logger.console = on;
    }

    /// Remote syslog collectors (configuration only; the kernel may send).
    pub fn log_remotes(&self) -> Vec<Ipv4Addr> {
        self.logger.remotes.iter().map(|r| Ipv4Addr(*r)).collect()
    }

    pub fn add_log_remote(&mut self, ip: Ipv4Addr) -> Result<(), BridgeError> {
        if self.logger.remotes.contains(&ip.0) {
            return Ok(());
        }
        if self.logger.remotes.len() >= MAX_REMOTES {
            return Err(BridgeError::OutOfRange { what: "collectors", min: 0, max: MAX_REMOTES as u32 });
        }
        self.logger.remotes.push(ip.0);
        Ok(())
    }

    pub fn remove_log_remote(&mut self, ip: Ipv4Addr) -> Result<(), BridgeError> {
        let before = self.logger.remotes.len();
        self.logger.remotes.retain(|r| *r != ip.0);
        if self.logger.remotes.len() == before {
            Err(BridgeError::NoSuchEntry)
        } else {
            Ok(())
        }
    }

    /// Buffered log entries, oldest first.
    pub fn log_entries(&self) -> impl DoubleEndedIterator<Item = &LogEntry> {
        self.logger.ring.iter()
    }

    pub fn clear_log(&mut self) {
        self.logger.ring.clear();
    }

    // ---------------- SVIs ----------------

    /// Configure (Some) or remove (None) the SVI address of `vlan`. The VLAN
    /// must exist to add an SVI. Only VLANs with an SVI receive
    /// `Output::Local` and accept `send_local`.
    pub fn set_svi(&mut self, vlan: u16, addr: Option<(Ipv4Addr, u8)>) -> Result<(), BridgeError> {
        check_vid(vlan)?;
        match addr {
            Some((ip, prefix)) => {
                if !self.vlans.contains_key(&vlan) {
                    return Err(BridgeError::VlanNotFound(vlan));
                }
                if prefix > 32 {
                    return Err(BridgeError::OutOfRange { what: "prefix", min: 0, max: 32 });
                }
                self.svis.insert(vlan, (ip.0, prefix));
            }
            None => {
                self.svis.remove(&vlan);
                self.fdb.remove_dynamic_where(|v, _, e| v == vlan && e.port == PortRef::Cpu);
            }
        }
        self.lldp_local_change();
        Ok(())
    }

    /// Configured SVIs: (vlan, ip, prefix), ascending by VLAN.
    pub fn svis(&self) -> Vec<(u16, Ipv4Addr, u8)> {
        self.svis.iter().map(|(v, (ip, p))| (*v, Ipv4Addr(*ip), *p)).collect()
    }

    pub fn has_svi(&self, vlan: u16) -> bool {
        self.svis.contains_key(&vlan)
    }

    // ---------------- snapshot ----------------

    /// Complete configuration snapshot (comparable).
    pub fn config_snapshot(&self) -> BridgeConfig {
        let mut statics: Vec<StaticMac> = self.fdb.iter().filter(|(_, _, e)| e.is_static).map(|(vlan, mac, e)| StaticMac { mac, vlan, port: e.port }).collect();
        statics.sort_by_key(|s| (s.vlan, s.mac));
        BridgeConfig {
            age_time_secs: self.age_time_secs,
            vlans: self.vlans.values().cloned().collect(),
            eth: self.ports.iter().map(|p| p.cfg.clone()).collect(),
            lags: self.lags.values().map(|l| l.cfg.clone()).collect(),
            stp: self.stp_cfg.clone(),
            lldp: self.lldp_cfg.clone(),
            log: LogConfig { severity: self.logger.severity, console: self.logger.console, remotes: self.logger.remotes.clone() },
            statics,
            svis: self.svis.iter().map(|(v, (ip, p))| (*v, *ip, *p)).collect(),
        }
    }
}
