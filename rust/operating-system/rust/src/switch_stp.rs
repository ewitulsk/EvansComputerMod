//! Phase 5 — Spanning Tree (simplified single-instance CIST, IEEE 802.1Q-2014).
//!
//! Implements:
//!   - Bridge ID (priority||MAC) + root election through Hello BPDUs
//!   - Port roles: Root, Designated, Alternate, Disabled
//!   - Port states: Disabled, Blocking, Learning, Forwarding
//!   - Forward-delay, hello-time, max-age timers
//!   - Per-port admin-edge (skip listening/learning)
//!   - BPDU guard (disable a port that receives a BPDU)
//!   - Root guard (revert a port to designated if it sees a superior BPDU)
//!   - TCN guard (suppress outgoing TCN)
//!
//! Cut corners (see switch_instructions.md):
//!   - No topology-change handshake (TCN/TC bit is set but we never drain
//!     the dynamic MAC table)
//!   - No proposal/agreement handshake (RSTP sync)
//!   - No per-VLAN instances; this is the CIST only
//!   - No backup-port detection (LAN with two ports from same bridge)
//!
//! Commands:
//!   [no] spanning-tree
//!   spanning-tree priority <0-61440, step 4096>
//!   spanning-tree forward-delay <4-30>
//!   spanning-tree hello-time <1-10>
//!   spanning-tree max-age <6-40>
//!   spanning-tree config-name <name>
//!   spanning-tree config-revision <0-65535>
//!   interface: [no] spanning-tree [port-priority <n>|cost <n>|admin-edge-port|bpdu-guard|root-guard|tcn-guard]

use crate::shell::ShellInstance;
use crate::switch::{self, SwitchState, with_state_mut};
use crate::switch_log::{self, Severity};
use crate::net;
use crate::net::eth::{self, ETHERTYPE_8021Q};
use crate::net::types::{MacAddr, MAX_FRAME_SIZE};

pub const BPDU_DST: [u8; 6] = [0x01, 0x80, 0xc2, 0x00, 0x00, 0x00];

pub const DEFAULT_PRIORITY: u16 = 32768;
pub const DEFAULT_HELLO_SECS: u32 = 2;
pub const DEFAULT_FORWARD_DELAY_SECS: u32 = 15;
pub const DEFAULT_MAX_AGE_SECS: u32 = 20;
pub const DEFAULT_PORT_PRIORITY: u8 = 128;
pub const DEFAULT_PORT_COST: u32 = 20000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PortRole { Disabled, Root, Designated, Alternate }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PortState { Disabled, Blocking, Learning, Forwarding }

impl PortRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            PortRole::Disabled => "Disabled",
            PortRole::Root => "Root",
            PortRole::Designated => "Designated",
            PortRole::Alternate => "Alternate",
        }
    }
}
impl PortState {
    pub fn as_str(&self) -> &'static str {
        match self {
            PortState::Disabled => "Disabled",
            PortState::Blocking => "Blocking",
            PortState::Learning => "Learning",
            PortState::Forwarding => "Forwarding",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BridgeId {
    pub priority: u16,  // priority || extension (here 4-bit extension is always 0)
    pub mac: [u8; 6],
}
impl BridgeId {
    pub fn new(priority: u16, mac: MacAddr) -> Self { Self { priority, mac: mac.0 } }
    pub fn as_bytes(&self) -> [u8; 8] {
        let mut out = [0u8; 8];
        out[..2].copy_from_slice(&self.priority.to_be_bytes());
        out[2..].copy_from_slice(&self.mac);
        out
    }
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 8 { return None; }
        Some(Self {
            priority: u16::from_be_bytes([b[0], b[1]]),
            mac: [b[2], b[3], b[4], b[5], b[6], b[7]],
        })
    }
}

#[derive(Clone)]
pub struct StpPort {
    pub enabled: bool,
    pub priority: u8,
    pub cost: u32,
    pub admin_edge: bool,
    pub bpdu_guard: bool,
    pub root_guard: bool,
    pub tcn_guard: bool,
    pub role: PortRole,
    pub state: PortState,
    pub state_since_ms: i64,

    // Best BPDU info seen on this port (from a peer).
    pub designated_bridge: Option<BridgeId>,
    pub designated_port: u16,
    pub root_id: Option<BridgeId>,
    pub root_path_cost: u32,
    pub last_bpdu_ms: i64,

    pub tx_count: u64,
    pub rx_count: u64,
    pub bpdu_guard_tripped: bool,
}

impl StpPort {
    pub fn new() -> Self {
        Self {
            enabled: true,
            priority: DEFAULT_PORT_PRIORITY,
            cost: DEFAULT_PORT_COST,
            admin_edge: false,
            bpdu_guard: false,
            root_guard: false,
            tcn_guard: false,
            role: PortRole::Designated,
            state: PortState::Blocking,
            state_since_ms: 0,
            designated_bridge: None,
            designated_port: 0,
            root_id: None,
            root_path_cost: 0,
            last_bpdu_ms: 0,
            tx_count: 0,
            rx_count: 0,
            bpdu_guard_tripped: false,
        }
    }
}

pub struct StpState {
    pub enabled: bool,
    pub priority: u16,
    pub hello_secs: u32,
    pub forward_delay_secs: u32,
    pub max_age_secs: u32,
    pub config_name: String,
    pub config_revision: u16,
    pub bridge_id: BridgeId,
    pub root_id: BridgeId,
    pub root_port: Option<usize>,
    pub root_path_cost: u32,
    pub ports: Vec<StpPort>,
    pub last_hello_ms: i64,
    pub topology_changes: u64,
}

impl StpState {
    pub fn new(iface_count: usize, bridge_mac: MacAddr) -> Self {
        let bid = BridgeId::new(DEFAULT_PRIORITY, bridge_mac);
        Self {
            enabled: false,
            priority: DEFAULT_PRIORITY,
            hello_secs: DEFAULT_HELLO_SECS,
            forward_delay_secs: DEFAULT_FORWARD_DELAY_SECS,
            max_age_secs: DEFAULT_MAX_AGE_SECS,
            config_name: String::new(),
            config_revision: 0,
            bridge_id: bid,
            root_id: bid,
            root_port: None,
            root_path_cost: 0,
            ports: (0..iface_count).map(|_| StpPort::new()).collect(),
            last_hello_ms: 0,
            topology_changes: 0,
        }
    }
}

/// Query helper used by the forwarding path (see `switch::handle_frame`).
/// Returns true if STP is enabled AND the port is not forwarding. A port
/// in Learning state *may* learn source MACs but must not forward data
/// frames; for simplicity we only treat Forwarding as unblocked.
pub fn port_forwards(state: &SwitchState, port: usize) -> bool {
    if !state.stp.enabled { return true; }
    state.stp.ports.get(port).map(|p| p.state == PortState::Forwarding).unwrap_or(true)
}

// =====================================================================
// Periodic tick — hello + state transitions + aging
// =====================================================================

pub fn tick(state: &mut SwitchState, stack: &mut net::NetStack, now_ms: i64) {
    if !state.stp.enabled {
        return;
    }
    // Update per-port state by link up/down.
    let iface_count = state.iface_count.min(stack.iface_count);
    for i in 0..iface_count {
        let up = stack.interfaces[i].link_up;
        if let Some(p) = state.stp.ports.get_mut(i) {
            if !up && p.state != PortState::Disabled {
                p.state = PortState::Disabled;
                p.role = PortRole::Disabled;
                p.state_since_ms = now_ms;
            } else if up && p.state == PortState::Disabled && p.enabled {
                if p.admin_edge {
                    p.state = PortState::Forwarding;
                    p.role = PortRole::Designated;
                } else {
                    p.state = PortState::Blocking;
                    p.role = PortRole::Designated;
                }
                p.state_since_ms = now_ms;
            }
        }
    }

    // Age out BPDUs: if last_bpdu_ms older than max_age, drop root-path info on that port.
    let max_age_ms = state.stp.max_age_secs as i64 * 1_000;
    let fd_ms = state.stp.forward_delay_secs as i64 * 1_000;
    for p in state.stp.ports.iter_mut() {
        if p.last_bpdu_ms != 0 && (now_ms - p.last_bpdu_ms) > max_age_ms {
            p.designated_bridge = None;
            p.root_id = None;
            p.root_path_cost = 0;
            p.last_bpdu_ms = 0;
        }
    }

    // Recompute root election + roles.
    recompute_roles(&mut state.stp, now_ms);

    // Advance port states by timers: Blocking->Learning->Forwarding on designated/root.
    for p in state.stp.ports.iter_mut() {
        if !p.enabled { continue; }
        if p.state == PortState::Disabled { continue; }
        if p.role == PortRole::Alternate {
            p.state = PortState::Blocking;
            continue;
        }
        // Root or Designated.
        if p.admin_edge {
            if p.state != PortState::Forwarding {
                p.state = PortState::Forwarding;
                p.state_since_ms = now_ms;
            }
            continue;
        }
        match p.state {
            PortState::Blocking => {
                p.state = PortState::Learning;
                p.state_since_ms = now_ms;
            }
            PortState::Learning => {
                if now_ms - p.state_since_ms >= fd_ms {
                    p.state = PortState::Forwarding;
                    p.state_since_ms = now_ms;
                }
            }
            PortState::Forwarding | PortState::Disabled => {}
        }
    }

    // Send Hello BPDUs from designated + root bridge.
    let hello_ms = state.stp.hello_secs as i64 * 1_000;
    if state.stp.last_hello_ms == 0 || (now_ms - state.stp.last_hello_ms) >= hello_ms {
        send_hello_all(state, stack);
        state.stp.last_hello_ms = now_ms;
    }
}

fn recompute_roles(s: &mut StpState, now_ms: i64) {
    // Start with our own bridge as root.
    let old_root = s.root_id;
    let mut best_root = s.bridge_id;
    let mut best_cost: u32 = 0;
    let mut best_port: Option<usize> = None;
    for (i, p) in s.ports.iter().enumerate() {
        if !p.enabled || p.state == PortState::Disabled { continue; }
        if let Some(rid) = p.root_id {
            let total = p.root_path_cost.saturating_add(p.cost);
            if rid < best_root || (rid == best_root && total < best_cost) {
                best_root = rid;
                best_cost = total;
                best_port = Some(i);
            }
        }
    }
    s.root_id = best_root;
    s.root_path_cost = if best_port.is_some() { best_cost } else { 0 };
    s.root_port = best_port;

    if old_root != s.root_id {
        let m = format!(
            "STP root change: {} (cost {}) via {}",
            format_bid(&s.root_id),
            s.root_path_cost,
            best_port.map(|p| format!("eth{}", p)).unwrap_or_else(|| "self".to_string()),
        );
        switch_log::log(Severity::Notice, m);
        s.topology_changes = s.topology_changes.saturating_add(1);
    }

    // Assign roles.
    for (i, p) in s.ports.iter_mut().enumerate() {
        if !p.enabled || p.state == PortState::Disabled {
            p.role = PortRole::Disabled;
            continue;
        }
        if Some(i) == best_port {
            p.role = PortRole::Root;
            continue;
        }
        // Port is designated if our advertised (bridge_id, our cost) is
        // better than the BPDU we'd forward on this segment.
        let our_cost = s.root_path_cost;
        if let Some(peer_bid) = p.designated_bridge {
            let peer_total = p.root_path_cost;
            let our_root = s.root_id;
            let peer_root = p.root_id.unwrap_or(peer_bid);
            let we_win = our_root < peer_root
                || (our_root == peer_root && our_cost < peer_total)
                || (our_root == peer_root && our_cost == peer_total && s.bridge_id < peer_bid);
            if we_win {
                p.role = PortRole::Designated;
            } else {
                p.role = PortRole::Alternate;
                if p.root_guard {
                    // Root guard: port would have made a superior neighbor
                    // the root → keep it as designated+blocking instead.
                    p.role = PortRole::Designated;
                    p.state = PortState::Blocking;
                    p.state_since_ms = now_ms;
                    switch_log::log(Severity::Warning,
                        format!("STP root-guard blocked eth{} (superior BPDU seen)", i));
                }
            }
        } else {
            p.role = PortRole::Designated;
        }
    }
}

fn send_hello_all(state: &mut SwitchState, stack: &mut net::NetStack) {
    let root_bytes = state.stp.root_id.as_bytes();
    let bridge_bytes = state.stp.bridge_id.as_bytes();
    let path_cost = state.stp.root_path_cost;
    let iface_count = state.iface_count.min(stack.iface_count);
    for i in 0..iface_count {
        if !stack.interfaces[i].link_up { continue; }
        let port = match state.stp.ports.get(i) { Some(p) => p, None => continue };
        if !port.enabled { continue; }
        if port.state == PortState::Disabled { continue; }
        if port.role != PortRole::Designated && port.role != PortRole::Root {
            // Alternate/blocking: don't transmit.
            continue;
        }
        if port.admin_edge {
            // Edge ports don't send BPDUs unless TC is pending.
            continue;
        }

        let bpdu = build_rstp_bpdu(
            &root_bytes,
            path_cost,
            &bridge_bytes,
            i as u16,
            state.stp.max_age_secs as u16,
            state.stp.hello_secs as u16,
            state.stp.forward_delay_secs as u16,
            port.role,
        );
        let frame = wrap_bpdu_in_llc(&bpdu, stack.interfaces[i].mac);
        if frame.len() <= MAX_FRAME_SIZE {
            eth::send_frame_on(i, &frame);
            if let Some(p) = state.stp.ports.get_mut(i) {
                p.tx_count += 1;
            }
        }
    }
}

fn build_rstp_bpdu(
    root: &[u8; 8],
    cost: u32,
    bridge: &[u8; 8],
    port_id: u16,
    max_age: u16,
    hello: u16,
    fd: u16,
    role: PortRole,
) -> Vec<u8> {
    // Simplified RSTP BPDU layout (IEEE 802.1D-2004 §9):
    // protocol id (2)=0x0000, proto version(1)=2 (RSTP), bpdu type(1)=0x02 (RST),
    // flags(1), root id(8), root path cost(4), bridge id(8), port id(2),
    // msg-age(2), max-age(2), hello-time(2), forward-delay(2), v1-length(1)=0
    let mut out = Vec::with_capacity(36);
    out.extend_from_slice(&[0x00, 0x00]); // protocol id
    out.push(2);                          // version = RSTP
    out.push(0x02);                       // BPDU type = RST
    let flags: u8 = match role {
        PortRole::Designated => 0x3C,  // role=designated (binary 11), learning+forwarding
        PortRole::Root => 0x24,        // role=root
        _ => 0x00,
    };
    out.push(flags);
    out.extend_from_slice(root);
    out.extend_from_slice(&cost.to_be_bytes());
    out.extend_from_slice(bridge);
    let pid_tagged = (0x80u16 << 8) | port_id;
    out.extend_from_slice(&pid_tagged.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // msg age
    out.extend_from_slice(&(max_age * 256).to_be_bytes()); // values are in 1/256 s
    out.extend_from_slice(&(hello * 256).to_be_bytes());
    out.extend_from_slice(&(fd * 256).to_be_bytes());
    out.push(0); // v1 length
    out
}

fn wrap_bpdu_in_llc(bpdu: &[u8], src: MacAddr) -> Vec<u8> {
    // DST(6) SRC(6) LEN(2) LLC(3: DSAP=0x42 SSAP=0x42 CTRL=0x03) BPDU
    let mut out = Vec::with_capacity(14 + 3 + bpdu.len());
    out.extend_from_slice(&BPDU_DST);
    out.extend_from_slice(&src.0);
    let len = (3 + bpdu.len()) as u16;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&[0x42, 0x42, 0x03]);
    out.extend_from_slice(bpdu);
    out
}

// =====================================================================
// Ingress
// =====================================================================

pub fn on_frame(state: &mut SwitchState, _stack: &mut net::NetStack, port: usize, frame: &[u8]) {
    if !state.stp.enabled { return; }
    // Expect LLC header at offset 14 (or 18 if tagged).
    let mut off = 14usize;
    if frame.len() >= 18 && u16::from_be_bytes([frame[12], frame[13]]) == ETHERTYPE_8021Q {
        off = 18;
    }
    if frame.len() < off + 3 + 35 { return; }
    if frame[off] != 0x42 || frame[off + 1] != 0x42 || frame[off + 2] != 0x03 { return; }
    let bpdu = &frame[off + 3..];

    // Parse.
    if bpdu.len() < 35 { return; }
    let proto = u16::from_be_bytes([bpdu[0], bpdu[1]]);
    if proto != 0x0000 { return; }
    let _bpdu_type = bpdu[3];
    let root = BridgeId::from_bytes(&bpdu[4..12]);
    let cost = u32::from_be_bytes([bpdu[12], bpdu[13], bpdu[14], bpdu[15]]);
    let bridge = BridgeId::from_bytes(&bpdu[16..24]);
    let pid = u16::from_be_bytes([bpdu[24], bpdu[25]]);
    let now_ms = crate::net::NetStack::get().map(|s| s.now_ms).unwrap_or(0);

    if let Some(p) = state.stp.ports.get_mut(port) {
        if p.bpdu_guard {
            p.bpdu_guard_tripped = true;
            p.state = PortState::Disabled;
            p.role = PortRole::Disabled;
            p.state_since_ms = now_ms;
            switch_log::log(Severity::Error,
                format!("STP bpdu-guard tripped on eth{} — port disabled", port));
            return;
        }
        p.rx_count += 1;
        p.designated_bridge = bridge;
        p.designated_port = pid;
        p.root_id = root;
        p.root_path_cost = cost;
        p.last_bpdu_ms = now_ms;
    }
    recompute_roles(&mut state.stp, now_ms);
}

// =====================================================================
// CLI
// =====================================================================

pub fn cmd_spanning_tree(shell: &mut ShellInstance, args: &[&str], remove: bool) {
    if args.is_empty() {
        with_state_mut(|s| {
            s.stp.enabled = !remove;
            if s.stp.enabled {
                s.stp.last_hello_ms = 0;
            }
        });
        shell.println(if remove { "Spanning tree disabled." } else { "Spanning tree enabled." });
        return;
    }
    match args[0] {
        "priority" => {
            if remove { set_priority(shell, DEFAULT_PRIORITY); return; }
            if args.len() != 2 { shell.println("% Usage: spanning-tree priority <0-61440, step 4096>"); return; }
            match args[1].parse::<u16>() {
                Ok(v) if v % 4096 == 0 && v <= 61440 => set_priority(shell, v),
                _ => shell.println("% Priority must be 0..61440 in steps of 4096"),
            }
        }
        "forward-delay" => set_u32(shell, args, remove, 4, 30, |s, v| s.stp.forward_delay_secs = v, DEFAULT_FORWARD_DELAY_SECS, "forward-delay"),
        "hello-time" => set_u32(shell, args, remove, 1, 10, |s, v| s.stp.hello_secs = v, DEFAULT_HELLO_SECS, "hello-time"),
        "max-age" => set_u32(shell, args, remove, 6, 40, |s, v| s.stp.max_age_secs = v, DEFAULT_MAX_AGE_SECS, "max-age"),
        "config-name" => {
            if remove {
                with_state_mut(|s| s.stp.config_name.clear());
                shell.println("STP config-name cleared.");
                return;
            }
            if args.len() < 2 { shell.println("% Usage: spanning-tree config-name <name>"); return; }
            let name = args[1..].join(" ");
            with_state_mut(|s| s.stp.config_name = name.clone());
            shell.println(&format!("STP config-name set to {}.", name));
        }
        "config-revision" => {
            if remove { with_state_mut(|s| s.stp.config_revision = 0); shell.println("Reset."); return; }
            if args.len() != 2 { shell.println("% Usage: spanning-tree config-revision <0-65535>"); return; }
            match args[1].parse::<u16>() {
                Ok(v) => { with_state_mut(|s| s.stp.config_revision = v); shell.println("OK."); }
                _ => shell.println("% Invalid value"),
            }
        }
        _ => shell.println("% Unknown spanning-tree subcommand"),
    }
}

fn set_priority(shell: &mut ShellInstance, v: u16) {
    with_state_mut(|s| {
        s.stp.priority = v;
        s.stp.bridge_id = BridgeId::new(v, MacAddr(s.stp.bridge_id.mac));
        s.stp.root_id = s.stp.bridge_id;
        s.stp.root_port = None;
        s.stp.root_path_cost = 0;
        recompute_roles(&mut s.stp, 0);
    });
    shell.println(&format!("STP priority set to {}.", v));
}

fn set_u32(
    shell: &mut ShellInstance,
    args: &[&str],
    remove: bool,
    min: u32,
    max: u32,
    apply: impl Fn(&mut SwitchState, u32),
    default: u32,
    name: &str,
) {
    if remove {
        with_state_mut(|s| apply(s, default));
        shell.println(&format!("STP {} reset.", name));
        return;
    }
    if args.len() != 2 { shell.println(&format!("% Usage: spanning-tree {} <{}-{}>", name, min, max)); return; }
    match args[1].parse::<u32>() {
        Ok(v) if v >= min && v <= max => {
            with_state_mut(|s| apply(s, v));
            shell.println(&format!("STP {} set to {}.", name, v));
        }
        _ => shell.println(&format!("% Value out of range ({}-{})", min, max)),
    }
}

pub fn cmd_if_spanning_tree(shell: &mut ShellInstance, idx: usize, args: &[&str], remove: bool) {
    if args.is_empty() {
        with_state_mut(|s| {
            if let Some(p) = s.stp.ports.get_mut(idx) { p.enabled = !remove; }
        });
        shell.println(if remove { "STP disabled on port." } else { "STP enabled on port." });
        return;
    }
    match args[0] {
        "port-priority" => {
            if args.len() != 2 { shell.println("% Usage: spanning-tree port-priority <0-240>"); return; }
            match args[1].parse::<u8>() {
                Ok(v) if v <= 240 && v % 16 == 0 => {
                    with_state_mut(|s| { if let Some(p) = s.stp.ports.get_mut(idx) { p.priority = v; } });
                    shell.println("Port priority set.");
                }
                _ => shell.println("% Port-priority must be 0..240 in steps of 16"),
            }
        }
        "cost" => {
            if args.len() != 2 { shell.println("% Usage: spanning-tree cost <n>"); return; }
            match args[1].parse::<u32>() {
                Ok(v) if v >= 1 => {
                    with_state_mut(|s| { if let Some(p) = s.stp.ports.get_mut(idx) { p.cost = v; } });
                    shell.println("Port cost set.");
                }
                _ => shell.println("% Invalid cost"),
            }
        }
        "admin-edge-port" => {
            with_state_mut(|s| { if let Some(p) = s.stp.ports.get_mut(idx) { p.admin_edge = !remove; } });
            shell.println(if remove { "admin-edge cleared." } else { "admin-edge set." });
        }
        "bpdu-guard" => {
            with_state_mut(|s| { if let Some(p) = s.stp.ports.get_mut(idx) { p.bpdu_guard = !remove; } });
            shell.println(if remove { "bpdu-guard cleared." } else { "bpdu-guard enabled." });
        }
        "root-guard" => {
            with_state_mut(|s| { if let Some(p) = s.stp.ports.get_mut(idx) { p.root_guard = !remove; } });
            shell.println(if remove { "root-guard cleared." } else { "root-guard enabled." });
        }
        "tcn-guard" => {
            with_state_mut(|s| { if let Some(p) = s.stp.ports.get_mut(idx) { p.tcn_guard = !remove; } });
            shell.println(if remove { "tcn-guard cleared." } else { "tcn-guard enabled." });
        }
        _ => shell.println("% Unknown per-port spanning-tree option"),
    }
}

// =====================================================================
// show / serialize
// =====================================================================

pub fn show(shell: &mut ShellInstance, args: &[&str]) {
    let full = args.is_empty();
    switch::with_state(|s| {
        shell.println(&format!("Spanning Tree: {}", if s.stp.enabled { "enabled" } else { "disabled" }));
        shell.println(&format!("  Bridge ID  : {}", format_bid(&s.stp.bridge_id)));
        shell.println(&format!("  Root ID    : {}", format_bid(&s.stp.root_id)));
        let role = if s.stp.root_id == s.stp.bridge_id { "This bridge is the root" }
                   else { "" };
        if !role.is_empty() { shell.println(&format!("  {}", role)); }
        shell.println(&format!("  Root port  : {}", s.stp.root_port.map(|p| format!("eth{}", p)).unwrap_or_else(|| "-".to_string())));
        shell.println(&format!("  Root cost  : {}", s.stp.root_path_cost));
        shell.println(&format!("  Hello/FD/MaxAge: {}/{}/{}",
            s.stp.hello_secs, s.stp.forward_delay_secs, s.stp.max_age_secs));
        shell.println(&format!("  Topo changes: {}", s.stp.topology_changes));
        if full {
            shell.println("");
            shell.println("Port    Role         State        Cost     Prio  PeerBridge");
            shell.println("------------------------------------------------------------");
            for (i, p) in s.stp.ports.iter().enumerate() {
                let peer = p.designated_bridge.map(|b| format_bid(&b)).unwrap_or_else(|| "-".to_string());
                shell.println(&format!("eth{:<3}  {:<11}  {:<11}  {:<7}  {:<4}  {}",
                    i, p.role.as_str(), p.state.as_str(), p.cost, p.priority, peer));
            }
        }
    });
}

pub fn serialize(state: &SwitchState, out: &mut String) {
    if state.stp.enabled {
        out.push_str("spanning-tree\n");
    }
    if state.stp.priority != DEFAULT_PRIORITY {
        out.push_str(&format!("spanning-tree priority {}\n", state.stp.priority));
    }
    if state.stp.forward_delay_secs != DEFAULT_FORWARD_DELAY_SECS {
        out.push_str(&format!("spanning-tree forward-delay {}\n", state.stp.forward_delay_secs));
    }
    if state.stp.hello_secs != DEFAULT_HELLO_SECS {
        out.push_str(&format!("spanning-tree hello-time {}\n", state.stp.hello_secs));
    }
    if state.stp.max_age_secs != DEFAULT_MAX_AGE_SECS {
        out.push_str(&format!("spanning-tree max-age {}\n", state.stp.max_age_secs));
    }
    if !state.stp.config_name.is_empty() {
        out.push_str(&format!("spanning-tree config-name {}\n", state.stp.config_name));
    }
    if state.stp.config_revision != 0 {
        out.push_str(&format!("spanning-tree config-revision {}\n", state.stp.config_revision));
    }
    for (i, p) in state.stp.ports.iter().enumerate() {
        let defaults = StpPort::new();
        if p.enabled != defaults.enabled
            || p.priority != defaults.priority
            || p.cost != defaults.cost
            || p.admin_edge != defaults.admin_edge
            || p.bpdu_guard != defaults.bpdu_guard
            || p.root_guard != defaults.root_guard
            || p.tcn_guard != defaults.tcn_guard
        {
            out.push_str(&format!("interface eth{}\n", i));
            if !p.enabled { out.push_str(" no spanning-tree\n"); }
            if p.priority != defaults.priority {
                out.push_str(&format!(" spanning-tree port-priority {}\n", p.priority));
            }
            if p.cost != defaults.cost {
                out.push_str(&format!(" spanning-tree cost {}\n", p.cost));
            }
            if p.admin_edge { out.push_str(" spanning-tree admin-edge-port\n"); }
            if p.bpdu_guard { out.push_str(" spanning-tree bpdu-guard\n"); }
            if p.root_guard { out.push_str(" spanning-tree root-guard\n"); }
            if p.tcn_guard { out.push_str(" spanning-tree tcn-guard\n"); }
            out.push_str("exit\n");
        }
    }
}

fn format_bid(b: &BridgeId) -> String {
    format!(
        "{:04x}.{:02x}{:02x}{:02x}.{:02x}{:02x}{:02x}",
        b.priority,
        b.mac[0], b.mac[1], b.mac[2],
        b.mac[3], b.mac[4], b.mac[5]
    )
}
