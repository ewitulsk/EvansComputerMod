//! Phase 6 — Link Aggregation (static LAG + LACP, IEEE 802.1AX).
//!
//! Supports:
//!   - `interface lag <id>` logical LAG context (access/trunk modes)
//!   - Physical-port membership (`lag <id>` inside per-interface context)
//!   - LACP in active or passive mode; fast (1s) or slow (30s) PDUs
//!   - Basic actor/partner state machine with a simplified selection
//!     that accepts a member into the LAG when its partner-key matches
//!   - Egress hash (l2 / l3 / l4-src-dst) across member ports
//!   - `show lacp interfaces/aggregates/configuration`, `show interface lag <id>`
//!
//! Cut corners:
//!   - No churn detector, no marker protocol, no wait-while timer
//!   - Selection/mux logic is "partner key matches => collecting+distributing"
//!   - `lacp fallback` is honored only when zero partners are detected
//!     across all members after the startup window
//!   - The LAG can hold at most MAX_LAG_MEMBERS physical ports

use crate::shell::ShellInstance;
use crate::switch::{self, SwitchState, with_state_mut, AllowedList, PortMode, CurrentContext};
use crate::switch_log::{self, Severity};
use crate::net;
use crate::net::eth::{self, ETHERTYPE_8021Q};
use crate::net::types::{MacAddr, MAX_FRAME_SIZE};
use std::collections::BTreeSet;

pub const ETHERTYPE_SLOW: u16 = 0x8809;
pub const LACP_SUBTYPE: u8 = 0x01;
pub const LACP_VERSION: u8 = 0x01;

pub const LACP_MULTICAST: [u8; 6] = [0x01, 0x80, 0xc2, 0x00, 0x00, 0x02];

pub const MAX_LAG_MEMBERS: usize = 8;
pub const MAX_LAG_ID: u16 = 256;

pub const LACP_FAST_MS: i64 = 1_000;
pub const LACP_SLOW_MS: i64 = 30_000;
pub const LACP_FALLBACK_WINDOW_MS: i64 = 90_000;

// Actor state bitmap.
pub const STATE_ACTIVITY: u8       = 0x01;   // 1=active, 0=passive
pub const STATE_TIMEOUT: u8        = 0x02;   // 1=fast, 0=slow
pub const STATE_AGGREGATION: u8    = 0x04;
pub const STATE_SYNCHRONIZATION: u8 = 0x08;
pub const STATE_COLLECTING: u8     = 0x10;
pub const STATE_DISTRIBUTING: u8   = 0x20;
pub const STATE_DEFAULTED: u8      = 0x40;
pub const STATE_EXPIRED: u8        = 0x80;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LacpMode { Active, Passive, Off }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LacpRate { Fast, Slow }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HashMode { L2, L3, L4SrcDst }

#[derive(Clone)]
pub struct LagConfig {
    pub id: u16,
    pub members: Vec<usize>,
    pub mode: PortMode,             // access/trunk/routed of the LAG logical port
    pub lacp_mode: LacpMode,
    pub lacp_rate: LacpRate,
    pub hash: HashMode,
    pub fallback: bool,
}

impl LagConfig {
    pub fn new(id: u16) -> Self {
        Self {
            id,
            members: Vec::new(),
            mode: PortMode::Routed,
            lacp_mode: LacpMode::Off,
            lacp_rate: LacpRate::Slow,
            hash: HashMode::L3,
            fallback: false,
        }
    }
}

#[derive(Clone)]
pub struct LacpPort {
    pub lag_id: Option<u16>,
    pub actor_state: u8,
    pub partner_state: u8,
    pub partner_system: [u8; 6],
    pub partner_system_priority: u16,
    pub partner_key: u16,
    pub partner_port: u16,
    pub partner_port_priority: u16,
    pub last_rx_ms: i64,
    pub last_tx_ms: i64,
    pub first_up_ms: i64,
    pub tx_count: u64,
    pub rx_count: u64,
    pub selected: bool,
}

impl LacpPort {
    pub fn new() -> Self {
        Self {
            lag_id: None,
            actor_state: 0,
            partner_state: 0,
            partner_system: [0u8; 6],
            partner_system_priority: 0,
            partner_key: 0,
            partner_port: 0,
            partner_port_priority: 0,
            last_rx_ms: 0,
            last_tx_ms: 0,
            first_up_ms: 0,
            tx_count: 0,
            rx_count: 0,
            selected: false,
        }
    }
}

pub struct LacpState {
    pub system_priority: u16,
    pub system_mac: [u8; 6],
    pub ports: Vec<LacpPort>,
}

impl LacpState {
    pub fn new(iface_count: usize, bridge_mac: MacAddr) -> Self {
        Self {
            system_priority: 32768,
            system_mac: bridge_mac.0,
            ports: (0..iface_count).map(|_| LacpPort::new()).collect(),
        }
    }
}

// =====================================================================
// LAG membership / context management
// =====================================================================

pub fn enter_lag(shell: &mut ShellInstance, id: u16) {
    if id < 1 || id > MAX_LAG_ID {
        shell.println("% LAG id out of range (1-256)");
        return;
    }
    with_state_mut(|s| {
        s.lags.entry(id).or_insert_with(|| LagConfig::new(id));
        s.context = CurrentContext::Lag(id);
    });
}

pub fn delete_lag(shell: &mut ShellInstance, id: u16) {
    let removed = with_state_mut(|s| {
        if !s.lags.contains_key(&id) { return false; }
        // Strip membership from physical ports.
        for p in s.lacp.ports.iter_mut() {
            if p.lag_id == Some(id) {
                p.lag_id = None;
                p.actor_state = 0;
                p.selected = false;
            }
        }
        s.lags.remove(&id);
        true
    }).unwrap_or(false);
    if removed {
        shell.println(&format!("LAG {} deleted.", id));
        switch_log::log(Severity::Info, format!("LAG {} removed", id));
    } else {
        shell.println("% LAG does not exist");
    }
}

pub fn exec_lag_ctx(shell: &mut ShellInstance, tokens: &[&str], line: &str, id: u16) {
    match tokens[0] {
        "no" => exec_lag_no(shell, id, &tokens[1..]),
        "no routing" => set_lag_l2(shell, id),
        "routing" => {
            // same as `no routing` is L2 — keep for parity
            shell.println("% Use 'no routing' to make this LAG an L2 port");
        }
        "vlan" => cmd_lag_vlan(shell, id, &tokens[1..], false),
        "lacp" => cmd_lag_lacp(shell, id, &tokens[1..], false),
        "hash" => cmd_lag_hash(shell, id, &tokens[1..]),
        _ => {
            // The single-token "no routing" path is typical; handle it.
            if tokens == ["no", "routing"] || tokens == ["no","routing"] {
                set_lag_l2(shell, id);
                return;
            }
            let msg = format!("% Invalid input in lag context: {}", line);
            shell.println(&msg);
        }
    }
}

fn exec_lag_no(shell: &mut ShellInstance, id: u16, args: &[&str]) {
    if args.is_empty() { shell.println("% Incomplete"); return; }
    match args[0] {
        "routing" => set_lag_l2(shell, id),
        "vlan" => cmd_lag_vlan(shell, id, &args[1..], true),
        "lacp" => cmd_lag_lacp(shell, id, &args[1..], true),
        _ => shell.println("% Unknown no-command in lag context"),
    }
}

fn set_lag_l2(shell: &mut ShellInstance, id: u16) {
    with_state_mut(|s| {
        if let Some(l) = s.lags.get_mut(&id) {
            l.mode = PortMode::Access { vid: 1 };
        }
    });
    shell.println("LAG converted to L2 (access vlan 1).");
}

fn cmd_lag_vlan(shell: &mut ShellInstance, id: u16, args: &[&str], remove: bool) {
    if args.is_empty() { shell.println("% Incomplete"); return; }
    match args[0] {
        "access" => {
            if remove {
                with_state_mut(|s| {
                    if let Some(l) = s.lags.get_mut(&id) { l.mode = PortMode::Access { vid: 1 }; }
                });
                shell.println("LAG access VLAN reset to 1.");
                return;
            }
            if args.len() != 2 { shell.println("% Usage: vlan access <id>"); return; }
            let vid: u16 = match args[1].parse() { Ok(v) => v, _ => { shell.println("% Invalid"); return; } };
            with_state_mut(|s| {
                if let Some(l) = s.lags.get_mut(&id) { l.mode = PortMode::Access { vid }; }
            });
            shell.println(&format!("LAG access VLAN set to {}.", vid));
        }
        "trunk" => {
            if args.len() < 2 { shell.println("% Incomplete"); return; }
            match args[1] {
                "native" => {
                    if args.len() < 3 { shell.println("% Usage: vlan trunk native <id> [tag]"); return; }
                    let vid: u16 = match args[2].parse() { Ok(v) => v, _ => { shell.println("% Invalid"); return; } };
                    let tag = args.len() == 4 && args[3] == "tag";
                    with_state_mut(|s| {
                        if let Some(l) = s.lags.get_mut(&id) {
                            l.mode = PortMode::Trunk { native: vid, native_tag: tag, allowed: AllowedList::All };
                        }
                    });
                    shell.println("LAG trunk native set.");
                }
                "allowed" => {
                    if args.len() < 3 { shell.println("% Usage: vlan trunk allowed <list|all>"); return; }
                    let rest = args[2..].join("");
                    let allowed = if rest == "all" {
                        AllowedList::All
                    } else {
                        match switch::parse_vlan_list_pub(&rest) {
                            Ok(s) => AllowedList::Some(s),
                            Err(e) => { shell.println(&format!("% {}", e)); return; }
                        }
                    };
                    with_state_mut(|s| {
                        if let Some(l) = s.lags.get_mut(&id) {
                            if let PortMode::Trunk { native, native_tag, .. } = l.mode.clone() {
                                l.mode = PortMode::Trunk { native, native_tag, allowed };
                            } else {
                                l.mode = PortMode::Trunk { native: 1, native_tag: false, allowed };
                            }
                        }
                    });
                    shell.println("LAG trunk allowed updated.");
                }
                _ => shell.println("% Unknown trunk subcommand"),
            }
        }
        _ => shell.println("% Unknown vlan subcommand"),
    }
}

fn cmd_lag_lacp(shell: &mut ShellInstance, id: u16, args: &[&str], remove: bool) {
    if args.is_empty() { shell.println("% Incomplete"); return; }
    match args[0] {
        "mode" => {
            if remove {
                with_state_mut(|s| { if let Some(l) = s.lags.get_mut(&id) { l.lacp_mode = LacpMode::Off; } });
                shell.println("LACP mode cleared (static LAG).");
                return;
            }
            if args.len() != 2 { shell.println("% Usage: lacp mode [active|passive]"); return; }
            let mode = match args[1] {
                "active" => LacpMode::Active,
                "passive" => LacpMode::Passive,
                _ => { shell.println("% Mode must be active or passive"); return; }
            };
            with_state_mut(|s| { if let Some(l) = s.lags.get_mut(&id) { l.lacp_mode = mode; } });
            shell.println(&format!("LACP mode set to {:?}.", mode));
        }
        "rate" => {
            if args.len() != 2 { shell.println("% Usage: lacp rate [fast|slow]"); return; }
            let rate = match args[1] {
                "fast" => LacpRate::Fast,
                "slow" => LacpRate::Slow,
                _ => { shell.println("% Rate must be fast or slow"); return; }
            };
            with_state_mut(|s| { if let Some(l) = s.lags.get_mut(&id) { l.lacp_rate = rate; } });
            shell.println(&format!("LACP rate set to {:?}.", rate));
        }
        "fallback" => {
            with_state_mut(|s| { if let Some(l) = s.lags.get_mut(&id) { l.fallback = !remove; } });
            shell.println(if remove { "LACP fallback disabled." } else { "LACP fallback enabled." });
        }
        _ => shell.println("% Unknown lacp subcommand"),
    }
}

fn cmd_lag_hash(shell: &mut ShellInstance, id: u16, args: &[&str]) {
    if args.len() != 1 {
        shell.println("% Usage: hash [l2|l3|l4-src-dst]");
        return;
    }
    let h = match args[0] {
        "l2" => HashMode::L2,
        "l3" => HashMode::L3,
        "l4-src-dst" | "l4" => HashMode::L4SrcDst,
        _ => { shell.println("% Unknown hash mode"); return; }
    };
    with_state_mut(|s| { if let Some(l) = s.lags.get_mut(&id) { l.hash = h; } });
    shell.println("Hash mode set.");
}

// Per-interface commands: `lag <id>` to join, `no lag <id>` to leave,
//   `lacp port-priority <n>` etc. (port-priority omitted — CLI stub).
pub fn cmd_if_lag(shell: &mut ShellInstance, idx: usize, args: &[&str], remove: bool) {
    if args.len() != 1 {
        shell.println("% Usage: [no] lag <id>");
        return;
    }
    let id: u16 = match args[0].parse() {
        Ok(v) => v,
        _ => { shell.println("% Invalid LAG id"); return; }
    };
    if remove {
        with_state_mut(|s| {
            if let Some(l) = s.lags.get_mut(&id) {
                l.members.retain(|&p| p != idx);
            }
            if let Some(p) = s.lacp.ports.get_mut(idx) {
                p.lag_id = None;
                p.actor_state = 0;
                p.selected = false;
            }
        });
        shell.println(&format!("eth{} removed from LAG {}.", idx, id));
        return;
    }
    let ok = with_state_mut(|s| {
        let lag = match s.lags.get_mut(&id) {
            Some(l) => l,
            None => return false,
        };
        if lag.members.contains(&idx) { return true; }
        if lag.members.len() >= MAX_LAG_MEMBERS { return false; }
        lag.members.push(idx);
        if let Some(p) = s.lacp.ports.get_mut(idx) {
            p.lag_id = Some(id);
        }
        true
    }).unwrap_or(false);
    if ok {
        shell.println(&format!("eth{} joined LAG {}.", idx, id));
        switch_log::log(Severity::Info, format!("LAG {}: eth{} joined", id, idx));
    } else {
        shell.println("% LAG not configured or member list full");
    }
}

pub fn cmd_if_lacp(shell: &mut ShellInstance, _idx: usize, args: &[&str], _remove: bool) {
    if args.is_empty() {
        shell.println("% Usage: lacp port-priority <n>");
        return;
    }
    // Accept but ignore port-priority tuning (documented as a stub).
    shell.println("LACP per-port knob acknowledged (no-op in this build).");
}

// =====================================================================
// Periodic tick — LACPDU TX, partner aging, selection
// =====================================================================

pub fn tick(state: &mut SwitchState, stack: &mut net::NetStack, now_ms: i64) {
    if state.lags.is_empty() { return; }

    let iface_count = state.iface_count.min(stack.iface_count);
    // Collect configuration snapshot per LAG to minimize reborrows.
    let lag_cfgs: Vec<LagConfig> = state.lags.values().cloned().collect();

    // Per-port TX.
    for cfg in &lag_cfgs {
        if cfg.lacp_mode == LacpMode::Off { continue; }
        let interval = match cfg.lacp_rate { LacpRate::Fast => LACP_FAST_MS, LacpRate::Slow => LACP_SLOW_MS };
        for &mem in &cfg.members {
            if mem >= iface_count { continue; }
            if !stack.interfaces[mem].link_up { continue; }
            let port = match state.lacp.ports.get(mem) { Some(p) => p.clone(), None => continue };

            // Passive ports should only transmit after seeing a partner.
            if cfg.lacp_mode == LacpMode::Passive && port.last_rx_ms == 0 {
                continue;
            }
            if port.last_tx_ms != 0 && (now_ms - port.last_tx_ms) < interval {
                continue;
            }
            let frame = build_lacpdu(state, cfg, mem, &port);
            eth::send_frame_on(mem, &frame);
            if let Some(p) = state.lacp.ports.get_mut(mem) {
                p.last_tx_ms = now_ms;
                p.tx_count += 1;
                if p.first_up_ms == 0 { p.first_up_ms = now_ms; }
            }
        }
    }

    // Partner aging.
    for i in 0..state.lacp.ports.len() {
        let (expire, was_selected, lag_id_opt) = {
            let p = &state.lacp.ports[i];
            let expire = p.lag_id.is_some() && p.last_rx_ms != 0
                && now_ms - p.last_rx_ms > 3 * LACP_SLOW_MS;
            (expire, p.selected, p.lag_id)
        };
        if expire {
            let p = &mut state.lacp.ports[i];
            p.selected = false;
            p.actor_state &= !(STATE_COLLECTING | STATE_DISTRIBUTING | STATE_SYNCHRONIZATION);
            if was_selected {
                switch_log::log(Severity::Warning, format!(
                    "LACP partner lost on eth{} LAG {}",
                    i,
                    lag_id_opt.unwrap_or(0)
                ));
            }
        }
    }

    // Selection: a member is selected if its partner key matches and partner is in-sync.
    for cfg in &lag_cfgs {
        let lag_id = cfg.id;
        let lag_key = lag_id;
        let mut any_partner = false;
        for &mem in &cfg.members {
            if mem >= state.lacp.ports.len() { continue; }
            let p = &state.lacp.ports[mem];
            if p.last_rx_ms != 0 && p.partner_key == lag_key {
                any_partner = true;
            }
        }
        for &mem in &cfg.members {
            if mem >= state.lacp.ports.len() { continue; }
            let p = &mut state.lacp.ports[mem];
            let ok = p.last_rx_ms != 0 && p.partner_key == lag_key;
            let fallback_ok = cfg.fallback && !any_partner
                && p.first_up_ms != 0
                && (now_ms - p.first_up_ms) > LACP_FALLBACK_WINDOW_MS;
            let was_selected = p.selected;
            let now_selected = ok || fallback_ok;
            p.selected = now_selected;
            if now_selected {
                p.actor_state |= STATE_SYNCHRONIZATION | STATE_COLLECTING | STATE_DISTRIBUTING;
            } else {
                p.actor_state &= !(STATE_COLLECTING | STATE_DISTRIBUTING);
            }
            if was_selected != now_selected {
                let msg = format!("LACP eth{} in LAG {} {} (partner={})",
                    mem, lag_id, if now_selected { "UP" } else { "down" }, ok);
                switch_log::log(Severity::Notice, msg);
            }
        }
    }
}

fn build_lacpdu(state: &SwitchState, cfg: &LagConfig, port_idx: usize, port: &LacpPort) -> Vec<u8> {
    // Preamble of eth frame: DST=LACP_MULTICAST, SRC=our MAC, EType=0x8809
    let src_mac = crate::net::NetStack::get().map(|s| s.interfaces[port_idx].mac.0).unwrap_or([0u8; 6]);
    let mut out = Vec::with_capacity(128);
    out.extend_from_slice(&LACP_MULTICAST);
    out.extend_from_slice(&src_mac);
    out.extend_from_slice(&ETHERTYPE_SLOW.to_be_bytes());
    out.push(LACP_SUBTYPE);
    out.push(LACP_VERSION);

    // Actor TLV (type=1, len=20): sys prio(2), sys(6), key(2), port prio(2), port(2), state(1), reserved(3)
    let actor_state = compute_actor_state(cfg, port);
    out.push(0x01);
    out.push(20);
    out.extend_from_slice(&state.lacp.system_priority.to_be_bytes());
    out.extend_from_slice(&state.lacp.system_mac);
    out.extend_from_slice(&cfg.id.to_be_bytes());
    out.extend_from_slice(&255u16.to_be_bytes()); // port priority
    out.extend_from_slice(&((port_idx as u16) + 1).to_be_bytes());
    out.push(actor_state);
    out.extend_from_slice(&[0, 0, 0]);

    // Partner TLV (type=2, len=20).
    out.push(0x02);
    out.push(20);
    out.extend_from_slice(&port.partner_system_priority.to_be_bytes());
    out.extend_from_slice(&port.partner_system);
    out.extend_from_slice(&port.partner_key.to_be_bytes());
    out.extend_from_slice(&port.partner_port_priority.to_be_bytes());
    out.extend_from_slice(&port.partner_port.to_be_bytes());
    out.push(port.partner_state);
    out.extend_from_slice(&[0, 0, 0]);

    // Collector TLV (type=3, len=16): max delay + 12 reserved.
    out.push(0x03);
    out.push(16);
    out.extend_from_slice(&0u16.to_be_bytes()); // collector max delay
    out.extend_from_slice(&[0u8; 12]);

    // Terminator TLV (type=0, len=0) + padding to at least 60 bytes minus FCS.
    out.push(0x00);
    out.push(0x00);
    while out.len() < 60 { out.push(0); }
    if out.len() > MAX_FRAME_SIZE { out.truncate(MAX_FRAME_SIZE); }
    out
}

fn compute_actor_state(cfg: &LagConfig, port: &LacpPort) -> u8 {
    let mut s = STATE_AGGREGATION;
    if cfg.lacp_mode == LacpMode::Active { s |= STATE_ACTIVITY; }
    if cfg.lacp_rate == LacpRate::Fast { s |= STATE_TIMEOUT; }
    s |= port.actor_state & (STATE_SYNCHRONIZATION | STATE_COLLECTING | STATE_DISTRIBUTING);
    s
}

// =====================================================================
// Ingress
// =====================================================================

pub fn on_frame(state: &mut SwitchState, _stack: &mut net::NetStack, port: usize, frame: &[u8]) {
    if port >= state.lacp.ports.len() { return; }
    let mut off = 14usize;
    if frame.len() >= 18 && u16::from_be_bytes([frame[12], frame[13]]) == ETHERTYPE_8021Q {
        off = 18;
    }
    if frame.len() < off + 2 { return; }
    if frame[off] != LACP_SUBTYPE { return; }

    // Minimum LACPDU is 109+2 bytes; we just read fixed offsets.
    if frame.len() < off + 110 { return; }
    // Actor: type(1)=1 len(1)=20 sys_prio(2) sys(6) key(2) port_prio(2) port(2) state(1) res(3)
    let actor = off + 2;
    if frame[actor] != 1 { return; }
    let actor_sys_prio = u16::from_be_bytes([frame[actor+2], frame[actor+3]]);
    let actor_sys: [u8; 6] = [frame[actor+4], frame[actor+5], frame[actor+6], frame[actor+7], frame[actor+8], frame[actor+9]];
    let actor_key = u16::from_be_bytes([frame[actor+10], frame[actor+11]]);
    let actor_port_prio = u16::from_be_bytes([frame[actor+12], frame[actor+13]]);
    let actor_port = u16::from_be_bytes([frame[actor+14], frame[actor+15]]);
    let actor_state = frame[actor+16];

    let now_ms = crate::net::NetStack::get().map(|s| s.now_ms).unwrap_or(0);
    let p = &mut state.lacp.ports[port];
    p.rx_count += 1;
    p.last_rx_ms = now_ms;
    p.partner_system_priority = actor_sys_prio;
    p.partner_system = actor_sys;
    p.partner_key = actor_key;
    p.partner_port_priority = actor_port_prio;
    p.partner_port = actor_port;
    p.partner_state = actor_state;
    if p.first_up_ms == 0 { p.first_up_ms = now_ms; }
}

// =====================================================================
// Egress helper — pick a member port for a hashed flow.
// =====================================================================

pub fn select_member(state: &SwitchState, lag_id: u16, frame: &[u8]) -> Option<usize> {
    let cfg = state.lags.get(&lag_id)?;
    if cfg.members.is_empty() { return None; }
    let live: Vec<usize> = cfg.members.iter().copied()
        .filter(|&m| {
            if cfg.lacp_mode == LacpMode::Off { return true; }
            state.lacp.ports.get(m).map(|p| p.selected).unwrap_or(false)
        }).collect();
    if live.is_empty() { return None; }
    let h = hash_frame(cfg.hash, frame) as usize;
    Some(live[h % live.len()])
}

fn hash_frame(mode: HashMode, frame: &[u8]) -> u32 {
    if frame.len() < 14 { return 0; }
    let mut h: u32 = 0x811c9dc5;
    let bytes: &[u8] = match mode {
        HashMode::L2 => &frame[0..12],
        HashMode::L3 if frame.len() >= 34 => &frame[26..34],
        HashMode::L4SrcDst if frame.len() >= 38 => &frame[34..38],
        _ => &frame[0..12],
    };
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

// =====================================================================
// show
// =====================================================================

pub fn show_lacp(shell: &mut ShellInstance, args: &[&str]) {
    if args.is_empty() {
        shell.println("% Usage: show lacp [interfaces|aggregates|configuration]");
        return;
    }
    match args[0] {
        "configuration" => show_lacp_config(shell),
        "aggregates" => show_lacp_aggregates(shell),
        "interfaces" => show_lacp_interfaces(shell),
        _ => shell.println("% Unknown show lacp option"),
    }
}

fn show_lacp_config(shell: &mut ShellInstance) {
    switch::with_state(|s| {
        shell.println(&format!("System Priority : {}", s.lacp.system_priority));
        shell.println(&format!("System MAC      : {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            s.lacp.system_mac[0], s.lacp.system_mac[1], s.lacp.system_mac[2],
            s.lacp.system_mac[3], s.lacp.system_mac[4], s.lacp.system_mac[5]));
        for (id, cfg) in s.lags.iter() {
            shell.println(&format!("LAG {}: mode={:?} rate={:?} hash={:?} fallback={} members={:?}",
                id, cfg.lacp_mode, cfg.lacp_rate, cfg.hash, cfg.fallback, cfg.members));
        }
    });
}

fn show_lacp_aggregates(shell: &mut ShellInstance) {
    switch::with_state(|s| {
        shell.println("LAG   Members                Mode      Rate   UpMembers");
        for (id, cfg) in s.lags.iter() {
            let up: Vec<usize> = cfg.members.iter().copied()
                .filter(|&m| s.lacp.ports.get(m).map(|p| p.selected).unwrap_or(false))
                .collect();
            let members_str: Vec<String> = cfg.members.iter().map(|m| format!("eth{}", m)).collect();
            shell.println(&format!("{:<4}  {:<22}  {:<8}  {:<5}  {}",
                id, members_str.join(","),
                format!("{:?}", cfg.lacp_mode),
                format!("{:?}", cfg.lacp_rate),
                up.len()));
        }
    });
}

fn show_lacp_interfaces(shell: &mut ShellInstance) {
    switch::with_state(|s| {
        shell.println("Port    LAG   ActorState  PartnerKey  Selected  Tx      Rx");
        shell.println("---------------------------------------------------------------");
        for (i, p) in s.lacp.ports.iter().enumerate() {
            let lag = p.lag_id.map(|x| x.to_string()).unwrap_or_else(|| "-".to_string());
            shell.println(&format!(
                "eth{:<3}  {:<4}  {:08b}    {:<10}  {:<8}  {:<6}  {}",
                i, lag, p.actor_state, p.partner_key,
                if p.selected { "yes" } else { "no" },
                p.tx_count, p.rx_count));
        }
    });
}

pub fn show_interface_lag(shell: &mut ShellInstance, args: &[&str]) {
    if args.len() != 1 {
        shell.println("% Usage: show interface lag <id>");
        return;
    }
    let id: u16 = match args[0].parse() {
        Ok(v) => v,
        _ => { shell.println("% Invalid LAG id"); return; }
    };
    switch::with_state(|s| {
        let cfg = match s.lags.get(&id) { Some(c) => c, None => { shell.println("% LAG not configured"); return; } };
        shell.println(&format!("LAG {}:", id));
        let mode_str = match &cfg.mode {
            PortMode::Routed => "routed".to_string(),
            PortMode::Access { vid } => format!("access vlan {}", vid),
            PortMode::Trunk { native, native_tag, allowed } => {
                let tag = if *native_tag { " tagged-native" } else { "" };
                let al = match allowed {
                    AllowedList::All => "all".to_string(),
                    AllowedList::Some(v) => v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","),
                };
                format!("trunk native {}{} allowed {}", native, tag, al)
            }
        };
        shell.println(&format!("  Mode       : {}", mode_str));
        shell.println(&format!("  LACP Mode  : {:?}", cfg.lacp_mode));
        shell.println(&format!("  LACP Rate  : {:?}", cfg.lacp_rate));
        shell.println(&format!("  Hash       : {:?}", cfg.hash));
        shell.println(&format!("  Fallback   : {}", cfg.fallback));
        shell.println(&format!("  Members    : {:?}", cfg.members));
        let up: Vec<usize> = cfg.members.iter().copied()
            .filter(|&m| s.lacp.ports.get(m).map(|p| p.selected).unwrap_or(false))
            .collect();
        shell.println(&format!("  Up members : {:?}", up));
    });
}

// =====================================================================
// Persistence
// =====================================================================

pub fn serialize(state: &SwitchState, out: &mut String) {
    for (id, cfg) in state.lags.iter() {
        out.push_str(&format!("interface lag {}\n", id));
        match &cfg.mode {
            PortMode::Routed => {}
            PortMode::Access { vid } => {
                out.push_str(" no routing\n");
                if *vid != 1 { out.push_str(&format!(" vlan access {}\n", vid)); }
            }
            PortMode::Trunk { native, native_tag, allowed } => {
                out.push_str(" no routing\n");
                if *native_tag {
                    out.push_str(&format!(" vlan trunk native {} tag\n", native));
                } else {
                    out.push_str(&format!(" vlan trunk native {}\n", native));
                }
                if let AllowedList::Some(s) = allowed {
                    let v: Vec<String> = s.iter().map(|x| x.to_string()).collect();
                    out.push_str(&format!(" vlan trunk allowed {}\n", v.join(",")));
                }
            }
        }
        match cfg.lacp_mode {
            LacpMode::Off => {}
            LacpMode::Active => out.push_str(" lacp mode active\n"),
            LacpMode::Passive => out.push_str(" lacp mode passive\n"),
        }
        if cfg.lacp_rate == LacpRate::Fast { out.push_str(" lacp rate fast\n"); }
        if cfg.fallback { out.push_str(" lacp fallback\n"); }
        match cfg.hash {
            HashMode::L2 => out.push_str(" hash l2\n"),
            HashMode::L4SrcDst => out.push_str(" hash l4-src-dst\n"),
            HashMode::L3 => {}
        }
        out.push_str("exit\n");
    }
    for (i, p) in state.lacp.ports.iter().enumerate() {
        if let Some(id) = p.lag_id {
            out.push_str(&format!("interface eth{}\n", i));
            out.push_str(&format!(" lag {}\n", id));
            out.push_str("exit\n");
        }
    }
    let _ = BTreeSet::<u16>::new();
}
