//! Layer 2 switch — Phases 1 + 2: MAC table + forwarding + 802.1Q VLANs.
//!
//! `switch` is a built-in sub-shell command. When entered, it enables
//! promiscuous mode on every interface and swaps the `IRQ_NETWORK` handler
//! so that incoming frames go through this module's forwarding path instead
//! of the normal local `NetStack::poll_rx`. Frames addressed to our own
//! MACs (or broadcast) are still re-injected into `NetStack::process_frame_on`
//! so management traffic (ARP for our IP, ping, SSH) continues to work while
//! the switch is running.
//!
//! Phase 1: Core L2 forwarding & MAC table
//!   - Dynamic MAC learning keyed by (mac, vlan_id)
//!   - Unicast forwarding to known ports
//!   - Broadcast / unknown-unicast flooding
//!   - Static MAC entries (never aged)
//!   - Configurable aging time (default 300 seconds)
//!   - Full Aruba-style `show` / `clear` / `mac-address-table` command set
//!   - MAC-move tracking for flap detection
//!
//! Phase 2: 802.1Q VLANs
//!   - Global VLAN database (vlan <id>, name, description, no shutdown)
//!   - Nested config sub-shells: `switch(config-vlan-N)#` and
//!     `switch(config-if-ethN)#`
//!   - Per-port modes: routed (default), access, trunk
//!   - Native VLAN with optional `tag` flag
//!   - Allowed-VLAN list on trunks (comma + range syntax)
//!   - Ingress filtering, egress tag/untag, broadcast-domain isolation
//!   - `show vlan` / `show vlan <id>` / `show vlan summary` /
//!     `show vlan port <n>`

use crate::net;
use crate::interrupt;
use crate::shell::{ShellInstance, OsState};
use crate::net::eth::{self, EthHeader, VlanTag, ETHERTYPE_8021Q};
use crate::net::types::{MacAddr, MAX_FRAME_SIZE};
use std::collections::{BTreeMap, BTreeSet};

const DEFAULT_AGE_TIME_SECS: u32 = 300;
const MIN_AGE_TIME_SECS: u32 = 15;
const MAX_AGE_TIME_SECS: u32 = 1_000_000;
const MAX_MAC_ENTRIES: usize = 512;
const DEFAULT_PVID: u16 = 1;
const SWITCH_INPUT_BUF_LEN: usize = 256;
const MIN_VID: u16 = 1;
const MAX_VID: u16 = 4094;
/// Maximum number of frames the switch IRQ handler will drain per invocation.
/// Paired with Java-side IRQ_NETWORK coalescing: if more frames are in flight
/// than this limit, the next arriving frame re-fires a fresh (coalesced)
/// IRQ_NETWORK and the handler picks up where it left off. This caps the
/// worst-case duration of a single `on_interrupt` call so the Java worker
/// thread can always return to input polling in a predictable amount of time.
const MAX_FRAMES_PER_IRQ: usize = 256;

/// The singleton switch state, present only while `OsState::Switch` is active.
static mut SWITCH_STATE: Option<SwitchState> = None;

// =====================================================================
// Phase 2: VLAN data model
// =====================================================================

/// A VLAN database entry.
#[derive(Clone)]
pub struct Vlan {
    pub vid: u16,
    pub name: Option<String>,
    pub description: Option<String>,
    /// VLANs are created in shutdown state and must be brought up
    /// explicitly with `no shutdown`. VLAN 1 is the only exception —
    /// it is created at switch entry with `active = true`.
    pub active: bool,
}

/// Allowed-VLAN list on a trunk port.
#[derive(Clone)]
pub enum AllowedList {
    /// Every VLAN, current and future. `vlan trunk allowed all`.
    All,
    /// An explicit set of VIDs.
    Some(BTreeSet<u16>),
}

impl AllowedList {
    /// Does this list permit VLAN `vid` to traverse the port?
    fn allows(&self, vid: u16) -> bool {
        match self {
            AllowedList::All => true,
            AllowedList::Some(s) => s.contains(&vid),
        }
    }
}

/// Per-port forwarding mode.
#[derive(Clone)]
pub enum PortMode {
    /// L3 (default per AOS-CX). No L2 switching on this port.
    Routed,
    /// Single untagged VLAN. All ingress frames must be untagged and
    /// are placed in `vid`; all egress frames are sent untagged.
    Access { vid: u16 },
    /// 802.1Q trunk port.
    Trunk {
        native: u16,
        /// If true, untagged ingress is dropped and the native VLAN's
        /// egress frames are tagged.
        native_tag: bool,
        allowed: AllowedList,
    },
}

#[derive(Clone)]
pub struct PortConfig {
    pub mode: PortMode,
}

/// Which nested config block the user is currently editing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CurrentContext {
    /// `switch(config)#`
    Top,
    /// `switch(config-vlan-N)#`
    Vlan(u16),
    /// `switch(config-if-ethN)#`
    Interface(usize),
}

/// A learned or statically configured MAC table entry.
#[derive(Clone)]
pub struct MacEntry {
    pub mac: MacAddr,
    pub vlan: u16,
    pub port: usize,
    pub is_static: bool,
    pub learned_ms: i64,
    pub prev_port: Option<usize>,
    pub move_count: u32,
    pub last_move_ms: i64,
}

pub struct SwitchState {
    pub entries: Vec<MacEntry>,
    pub age_time_secs: u32,
    pub own_macs: Vec<MacAddr>,
    pub iface_count: usize,
    pub last_age_ms: i64,
    pub input_buf: [u8; SWITCH_INPUT_BUF_LEN],
    pub input_len: usize,

    // Phase 2 — VLAN database, per-port config, current sub-shell context.
    pub vlans: BTreeMap<u16, Vlan>,
    pub ports: Vec<PortConfig>,
    pub context: CurrentContext,

    /// Whether exiting the CLI should preserve forwarding state. Set true
    /// by `start_detached` / the `on` command / `switch on`, false by
    /// plain `enter`. When false, `exit` tears the switch down (matching
    /// the original behavior of the switch command).
    pub persist_on_exit: bool,

    /// True only while `load_config` is replaying a saved `switch.cfg`.
    /// Guards the destructive top-level `exit` path so config files that
    /// end blocks with `exit` don't tear down the switch during the
    /// replay. Always false outside the loader.
    pub loading: bool,
}

impl SwitchState {
    fn new(own_macs: Vec<MacAddr>, iface_count: usize, now_ms: i64) -> Self {
        let mut vlans = BTreeMap::new();
        // VLAN 1 (default) is always present and active. Matches AOS-CX.
        vlans.insert(
            DEFAULT_PVID,
            Vlan {
                vid: DEFAULT_PVID,
                name: Some("default".to_string()),
                description: None,
                active: true,
            },
        );
        let ports = (0..iface_count)
            .map(|_| PortConfig { mode: PortMode::Routed })
            .collect();
        Self {
            entries: Vec::new(),
            age_time_secs: DEFAULT_AGE_TIME_SECS,
            own_macs,
            iface_count,
            last_age_ms: now_ms,
            input_buf: [0u8; SWITCH_INPUT_BUF_LEN],
            input_len: 0,
            vlans,
            ports,
            context: CurrentContext::Top,
            persist_on_exit: false,
            loading: false,
        }
    }

    /// True if the given VID is non-zero, in range, and `active`.
    fn vlan_active(&self, vid: u16) -> bool {
        self.vlans.get(&vid).map(|v| v.active).unwrap_or(false)
    }

    /// True if a VID exists in the database (regardless of active state).
    fn vlan_exists(&self, vid: u16) -> bool {
        self.vlans.contains_key(&vid)
    }

    /// Compute port membership for VLAN `vid`. Returns `Some(tagged)`
    /// where `tagged == true` means the port is a tagged member, or
    /// `None` if the port isn't a member at all.
    fn port_membership(&self, port: usize, vid: u16) -> Option<bool> {
        if port >= self.ports.len() {
            return None;
        }
        match &self.ports[port].mode {
            PortMode::Routed => None,
            PortMode::Access { vid: access } => {
                if *access == vid { Some(false) } else { None }
            }
            PortMode::Trunk { native, native_tag, allowed } => {
                if !allowed.allows(vid) {
                    return None;
                }
                // For AllowedList::All we only treat vids that exist
                // in the database as members, so growth of `vlans`
                // grows membership automatically.
                if matches!(allowed, AllowedList::All) && !self.vlan_exists(vid) {
                    return None;
                }
                if vid == *native && !*native_tag {
                    Some(false)
                } else {
                    Some(true)
                }
            }
        }
    }

    /// Is `vid` referenced by any port's config in a way that would
    /// break if the VLAN were deleted? Returns the port indexes.
    fn vlan_in_use_on(&self, vid: u16) -> Vec<usize> {
        let mut out = Vec::new();
        for (i, p) in self.ports.iter().enumerate() {
            let used = match &p.mode {
                PortMode::Routed => false,
                PortMode::Access { vid: a } => *a == vid,
                PortMode::Trunk { native, allowed, .. } => {
                    *native == vid
                        || matches!(allowed, AllowedList::Some(s) if s.contains(&vid))
                }
            };
            if used {
                out.push(i);
            }
        }
        out
    }

    fn is_own_mac(&self, mac: &MacAddr) -> bool {
        self.own_macs.iter().any(|m| m == mac)
    }

    fn find(&self, mac: &MacAddr, vlan: u16) -> Option<usize> {
        self.entries.iter().position(|e| e.mac == *mac && e.vlan == vlan)
    }

    /// Apply a learn event. Dynamic entries may be updated or moved; static
    /// entries are never overwritten by the learning path.
    fn learn(&mut self, src: MacAddr, vlan: u16, ingress: usize, now_ms: i64) {
        if let Some(idx) = self.find(&src, vlan) {
            let entry = &mut self.entries[idx];
            if entry.is_static {
                return;
            }
            if entry.port == ingress {
                entry.learned_ms = now_ms;
            } else {
                entry.prev_port = Some(entry.port);
                entry.port = ingress;
                entry.move_count += 1;
                entry.last_move_ms = now_ms;
                entry.learned_ms = now_ms;
            }
            return;
        }

        if self.entries.len() >= MAX_MAC_ENTRIES {
            // Evict the oldest dynamic entry.
            let mut oldest: Option<(usize, i64)> = None;
            for (i, e) in self.entries.iter().enumerate() {
                if e.is_static {
                    continue;
                }
                if oldest.map(|(_, t)| e.learned_ms < t).unwrap_or(true) {
                    oldest = Some((i, e.learned_ms));
                }
            }
            if let Some((idx, _)) = oldest {
                self.entries.remove(idx);
            } else {
                // All slots are static; cannot learn more.
                return;
            }
        }

        self.entries.push(MacEntry {
            mac: src,
            vlan,
            port: ingress,
            is_static: false,
            learned_ms: now_ms,
            prev_port: None,
            move_count: 0,
            last_move_ms: 0,
        });
    }

    /// Walk the table and drop expired dynamic entries, at most once per second.
    fn age_maybe(&mut self, now_ms: i64) {
        if now_ms - self.last_age_ms < 1_000 {
            return;
        }
        self.last_age_ms = now_ms;
        let age_ms = self.age_time_secs as i64 * 1_000;
        self.entries.retain(|e| e.is_static || (now_ms - e.learned_ms) <= age_ms);
    }

    /// Handle a single received frame on the given ingress port.
    ///
    /// Phase 2 dispatch:
    ///   1. Routed ports do no L2 switching, only local delivery.
    ///   2. Access ports drop tagged ingress; tag of the frame is the
    ///      access vid.
    ///   3. Trunk ports honor `native`/`native_tag` for untagged ingress
    ///      and the allowed-list for tagged ingress.
    ///   4. Frames in inactive or unknown VLANs are dropped.
    ///   5. Self-addressed/broadcast frames are re-injected into the
    ///      local NetStack as untagged (the kernel stack is VLAN-unaware).
    ///   6. Forwarding (unicast or flood) goes through `egress_send`
    ///      which enforces per-port egress mode.
    fn handle_frame(&mut self, stack: &mut net::NetStack, ingress: usize, frame: &[u8]) {
        if ingress >= self.iface_count {
            return;
        }
        if !stack.interfaces[ingress].link_up {
            return;
        }

        let (hdr, _payload) = match EthHeader::parse(frame) {
            Some(v) => v,
            None => return,
        };

        // ---- Routed ports: no L2 switching, only local delivery ----
        if matches!(self.ports[ingress].mode, PortMode::Routed) {
            let is_for_us = hdr.dst == MacAddr::BROADCAST || self.is_own_mac(&hdr.dst);
            if is_for_us {
                let mut buf = [0u8; MAX_FRAME_SIZE];
                let (untagged, ulen) = strip_tag(frame, &mut buf);
                stack.process_frame_on(ingress, &untagged[..ulen]);
            }
            return;
        }

        // ---- Determine ingress VLAN by port mode ----
        let frame_tag_vid = hdr.vlan_tag.map(|t| t.vid);
        let vlan = match (&self.ports[ingress].mode, frame_tag_vid) {
            (PortMode::Access { vid }, None) => *vid,
            (PortMode::Access { .. }, Some(_)) => {
                // Access ports do not accept tagged frames.
                return;
            }
            (PortMode::Trunk { native, native_tag, allowed }, None) => {
                if *native_tag {
                    // Trunk with tagged-native rejects untagged.
                    return;
                }
                if !allowed.allows(*native) {
                    // Native must be in the allowed list to be valid.
                    return;
                }
                *native
            }
            (PortMode::Trunk { allowed, .. }, Some(vid)) => {
                if !allowed.allows(vid) {
                    // Ingress filter — tagged frame for a disallowed VID.
                    return;
                }
                vid
            }
            (PortMode::Routed, _) => unreachable!("Routed handled above"),
        };

        // VLAN must be active and exist.
        if !self.vlan_active(vlan) {
            return;
        }

        let now_ms = stack.now_ms;

        // Learn source MAC (skip multicast/broadcast sources and our own MACs).
        let src_is_multicast = (hdr.src.0[0] & 0x01) != 0;
        if !src_is_multicast && !self.is_own_mac(&hdr.src) {
            self.learn(hdr.src, vlan, ingress, now_ms);
        }

        // Local delivery: any frame addressed to one of our own MACs, or a
        // broadcast, is injected back into the local network stack as
        // untagged so management traffic (ARP, ICMP, TCP, UDP, SSH) keeps
        // working while the switch is forwarding.
        let is_broadcast = hdr.dst == MacAddr::BROADCAST;
        let is_for_us = is_broadcast || self.is_own_mac(&hdr.dst);
        if is_for_us {
            let mut buf = [0u8; MAX_FRAME_SIZE];
            let (untagged, ulen) = strip_tag(frame, &mut buf);
            stack.process_frame_on(ingress, &untagged[..ulen]);
        }

        // ---- Forwarding decision ----
        if is_broadcast {
            self.flood_vlan(stack, ingress, vlan, frame);
            return;
        }

        // Pure self-traffic stops here.
        if self.is_own_mac(&hdr.dst) {
            return;
        }

        let lookup = self.find(&hdr.dst, vlan).map(|idx| self.entries[idx].port);
        match lookup {
            Some(egress) => {
                if egress == ingress {
                    // Hairpin drop.
                    return;
                }
                self.egress_send(stack, egress, vlan, frame);
            }
            None => {
                // Unknown unicast — flood VLAN.
                self.flood_vlan(stack, ingress, vlan, frame);
            }
        }
    }

    /// Send a frame out a single port, applying that port's egress mode
    /// (drops, strips tag for access/native-untagged, ensures tag for
    /// trunk-tagged or non-native-trunk).
    fn egress_send(&self, stack: &net::NetStack, egress: usize, vlan: u16, frame: &[u8]) {
        if egress >= self.iface_count {
            return;
        }
        if !stack.interfaces[egress].link_up {
            return;
        }
        let mut tx = [0u8; MAX_FRAME_SIZE];
        match &self.ports[egress].mode {
            PortMode::Routed => {
                // No L2 egress on routed ports.
            }
            PortMode::Access { vid } => {
                if *vid != vlan {
                    return;
                }
                let (out, len) = strip_tag(frame, &mut tx);
                eth::send_frame_on(egress, &out[..len]);
            }
            PortMode::Trunk { native, native_tag, allowed } => {
                if !allowed.allows(vlan) {
                    return;
                }
                if vlan == *native && !*native_tag {
                    let (out, len) = strip_tag(frame, &mut tx);
                    eth::send_frame_on(egress, &out[..len]);
                } else {
                    let len = ensure_tagged(frame, vlan, &mut tx);
                    eth::send_frame_on(egress, &tx[..len]);
                }
            }
        }
    }

    /// Flood a frame in `vlan` out every link-up port except the ingress.
    /// Per-port egress filtering happens inside `egress_send` so e.g. a
    /// trunk port that doesn't allow this VLAN is silently skipped.
    fn flood_vlan(&self, stack: &net::NetStack, ingress: usize, vlan: u16, frame: &[u8]) {
        for i in 0..self.iface_count {
            if i == ingress {
                continue;
            }
            self.egress_send(stack, i, vlan, frame);
        }
    }
}

// =====================================================================
// IRQ handler — replaces `NetStack::poll_rx` while the switch is active.
// =====================================================================

fn switch_irq_handler(_irq: i32, _data: &str) {
    let stack = match net::NetStack::get() {
        Some(s) => s,
        None => return,
    };
    let state = unsafe {
        match SWITCH_STATE.as_mut() {
            Some(s) => s,
            None => return,
        }
    };

    let mut rx = [0u8; MAX_FRAME_SIZE];
    let mut drained = 0usize;
    while drained < MAX_FRAMES_PER_IRQ {
        match eth::recv_frame_any(&mut rx) {
            Some((ingress, len)) => {
                // Copy the frame out of the shared rx buffer before handing it
                // to anything that might touch NetStack's own RX_BUF.
                let mut frame_copy = [0u8; MAX_FRAME_SIZE];
                frame_copy[..len].copy_from_slice(&rx[..len]);
                state.handle_frame(stack, ingress, &frame_copy[..len]);
                drained += 1;
            }
            None => break,
        }
    }

    state.age_maybe(stack.now_ms);
}

fn default_network_irq_handler(_irq: i32, _data: &str) {
    if let Some(stack) = net::NetStack::get() {
        stack.poll_rx();
    }
}

// =====================================================================
// Lifecycle: enter / exit / cleanup
// =====================================================================

/// Enter the config sub-shell. If the switch isn't already running,
/// initializes forwarding state with `persist_on_exit = false` so the
/// first `exit` tears down (matching the pre-detach behavior). If it
/// IS already running (started via `switch on` or via a prior detach),
/// just promotes the shell into `OsState::Switch` without re-initializing
/// — existing VLAN / MAC / port config is preserved.
pub fn enter(shell: &mut ShellInstance) {
    let already = unsafe { SWITCH_STATE.is_some() };
    if !already {
        if !init_state(shell, /* persist = */ false) {
            return;
        }
        shell.println("Entering switch configuration mode.");
        shell.println("Type 'help' for the command list, 'exit' to return to the shell.");
    } else {
        shell.println("Re-entering switch configuration mode (switch is running).");
    }
    unsafe {
        if let Some(s) = SWITCH_STATE.as_mut() {
            s.context = CurrentContext::Top;
        }
    }
    shell.state = OsState::Switch;
    print_prompt(shell);
}

/// Turn on switching without entering the CLI. Sets `persist_on_exit`
/// so any subsequent `switch` / `exit` round-trip won't tear it down.
/// Called from the main shell as `switch on`.
pub fn start_detached(shell: &mut ShellInstance) {
    let already = unsafe { SWITCH_STATE.is_some() };
    if already {
        unsafe {
            if let Some(s) = SWITCH_STATE.as_mut() {
                s.persist_on_exit = true;
            }
        }
        shell.println("Switch is already running.");
        return;
    }
    if !init_state(shell, /* persist = */ true) {
        return;
    }
    shell.println("Switch started. Forwarding in background. Use 'switch off' to stop.");
}

/// Stop forwarding unconditionally and tear down state. Safe from both
/// the main shell and from inside the CLI; if called from the CLI,
/// also leaves back to the main shell.
pub fn stop(shell: &mut ShellInstance) {
    if unsafe { SWITCH_STATE.is_none() } {
        shell.println("Switch is not running.");
        return;
    }
    tear_down();
    if shell.state == OsState::Switch {
        shell.state = OsState::Shell;
    }
    shell.println("Switch stopped.");
    crate::print_prompt(shell);
}

/// Shared helper: promiscuous on, IRQ handler registered, state
/// allocated, saved config loaded. Returns false if the network stack
/// isn't up yet (rare — only during early boot). Callers set the
/// `persist` flag on the newly created state.
fn init_state(shell: &mut ShellInstance, persist: bool) -> bool {
    let stack = match net::NetStack::get() {
        Some(s) => s,
        None => {
            shell.println("switch: network stack not initialized");
            return false;
        }
    };

    let mut own = Vec::with_capacity(stack.iface_count);
    for i in 0..stack.iface_count {
        own.push(stack.interfaces[i].mac);
        eth::set_promiscuous_on(i, true);
    }

    let iface_count = stack.iface_count;
    let now_ms = stack.now_ms;

    unsafe {
        let mut state = SwitchState::new(own, iface_count, now_ms);
        state.persist_on_exit = persist;
        SWITCH_STATE = Some(state);
    }

    interrupt::register(interrupt::IRQ_NETWORK, switch_irq_handler);

    // Replay any saved switch.cfg. Safe to call before the shell is
    // promoted to OsState::Switch — the loader runs commands through
    // exec_command directly and the `loading` flag guards destructive
    // exit paths during replay.
    load_config(shell);

    true
}

/// Leave the config sub-shell. Behavior depends on `persist_on_exit`:
///
/// - `persist_on_exit == false` (session started via bare `switch`):
///   tear down forwarding state and print "Exited switch mode."
///   Matches the original pre-detach behavior exactly.
///
/// - `persist_on_exit == true` (session started via `switch on`, or
///   promoted via the `on` CLI command): leave the CLI but keep
///   forwarding alive. Tell the user what's going on so they know
///   how to stop it.
///
/// Idempotent: if already torn down, returns without printing.
pub fn exit_switch(shell: &mut ShellInstance) {
    let persist = unsafe {
        match SWITCH_STATE.as_ref() {
            Some(s) => s.persist_on_exit,
            None => return,
        }
    };
    shell.state = OsState::Shell;
    if persist {
        shell.println("Exited switch mode. Switch continues running in background.");
        shell.println("Use 'switch off' to stop it.");
    } else {
        tear_down();
        shell.println("Exited switch mode.");
    }
    crate::print_prompt(shell);
}

/// Called from `reset_to_shell` (Ctrl+T). Must be safe to invoke even if
/// switch mode is not currently active.
pub fn cleanup_on_reset() {
    let active = unsafe { SWITCH_STATE.is_some() };
    if active {
        tear_down();
    }
}

fn tear_down() {
    if let Some(stack) = net::NetStack::get() {
        for i in 0..stack.iface_count {
            eth::set_promiscuous_on(i, false);
        }
    }
    interrupt::register(interrupt::IRQ_NETWORK, default_network_irq_handler);
    unsafe {
        SWITCH_STATE = None;
    }
}

/// Replay `/switch.cfg` through `exec_command`. Called from `init_state`
/// after a fresh `SwitchState` is installed, so both `switch` and
/// `switch on` automatically pick up the saved config. Does nothing if
/// the file doesn't exist (first run, or after the user manually
/// deleted it).
///
/// Sets `SwitchState::loading = true` while replaying so that the
/// `exit` commands that the config file uses to end each block don't
/// tear down the switch we're trying to load. Clears the flag and
/// resets the context to `Top` before returning.
fn load_config(shell: &mut ShellInstance) {
    let content = match crate::fs::read_file_absolute("switch.cfg") {
        Some(c) => c,
        None => return,
    };
    unsafe {
        if let Some(s) = SWITCH_STATE.as_mut() {
            s.loading = true;
        }
    }
    // Copy into an owned String so we can release the borrow on the
    // static file buffer — exec_command may re-use that buffer via
    // other fs reads mid-replay.
    let owned = content.to_string();
    let mut applied = 0usize;
    for raw in owned.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        exec_command(shell, line);
        applied += 1;
    }
    unsafe {
        if let Some(s) = SWITCH_STATE.as_mut() {
            s.loading = false;
            s.context = CurrentContext::Top;
        }
    }
    let msg = format!("Loaded switch.cfg ({} commands applied).", applied);
    shell.println(&msg);
}

/// Handler for the top-level `write memory` command. Serializes the
/// current running config and writes it to `/switch.cfg`.
fn cmd_write(shell: &mut ShellInstance, args: &[&str]) {
    if args.len() != 1 || args[0] != "memory" {
        shell.println("% Usage: write memory");
        return;
    }
    let cfg = serialize_running_config();
    if crate::fs::write_file_absolute("switch.cfg", &cfg) {
        shell.println("Configuration saved to /switch.cfg.");
    } else {
        shell.println("% Failed to write /switch.cfg");
    }
}

/// Serialize the current `SwitchState` as a replayable `switch.cfg` file.
/// Each line (or block) in the output is a command the CLI can parse
/// via `exec_command`; replaying the whole file through `load_config`
/// rebuilds the state. VLAN 1 is treated as implicit — it's always
/// present and active, so we only emit it if the user gave it a
/// non-default name or description. Routed ports are also implicit
/// (the default on switch start), so we only emit blocks for ports
/// converted to access or trunk mode.
fn serialize_running_config() -> String {
    let mut out = String::new();
    out.push_str("# switch.cfg - generated by 'write memory'\n");
    let state = unsafe {
        match SWITCH_STATE.as_ref() {
            Some(s) => s,
            None => return out,
        }
    };

    if state.age_time_secs != DEFAULT_AGE_TIME_SECS {
        out.push_str(&format!(
            "mac-address-table age-time {}\n",
            state.age_time_secs
        ));
    }

    for (vid, v) in state.vlans.iter() {
        let is_trivial_default = *vid == DEFAULT_PVID
            && v.name.as_deref() == Some("default")
            && v.description.is_none();
        if is_trivial_default {
            continue;
        }
        out.push_str(&format!("vlan {}\n", vid));
        if let Some(n) = &v.name {
            out.push_str(&format!(" name {}\n", n));
        }
        if let Some(d) = &v.description {
            out.push_str(&format!(" description {}\n", d));
        }
        if *vid != DEFAULT_PVID {
            if v.active {
                out.push_str(" no shutdown\n");
            } else {
                out.push_str(" shutdown\n");
            }
        }
        out.push_str("exit\n");
    }

    for (i, p) in state.ports.iter().enumerate() {
        match &p.mode {
            PortMode::Routed => continue,
            PortMode::Access { vid } => {
                out.push_str(&format!("interface eth{}\n", i));
                out.push_str(" no routing\n");
                if *vid != DEFAULT_PVID {
                    out.push_str(&format!(" vlan access {}\n", vid));
                }
                out.push_str("exit\n");
            }
            PortMode::Trunk {
                native,
                native_tag,
                allowed,
            } => {
                out.push_str(&format!("interface eth{}\n", i));
                out.push_str(" no routing\n");
                if *native_tag {
                    out.push_str(&format!(" vlan trunk native {} tag\n", native));
                } else {
                    out.push_str(&format!(" vlan trunk native {}\n", native));
                }
                match allowed {
                    // `vlan trunk native` already sets allowed=all, so
                    // this state is implicit and needs nothing emitted.
                    AllowedList::All => {}
                    AllowedList::Some(s) => {
                        let list: Vec<String> = s.iter().map(|v| v.to_string()).collect();
                        out.push_str(&format!(" vlan trunk allowed {}\n", list.join(",")));
                    }
                }
                out.push_str("exit\n");
            }
        }
    }

    for e in &state.entries {
        if !e.is_static {
            continue;
        }
        out.push_str(&format!(
            "static-mac {} vlan {} port {}\n",
            format_mac(&e.mac),
            e.vlan,
            e.port
        ));
    }

    out
}

fn print_prompt(shell: &mut ShellInstance) {
    let prompt = unsafe {
        match SWITCH_STATE.as_ref() {
            Some(s) => match s.context {
                CurrentContext::Top => "switch(config)# ".to_string(),
                CurrentContext::Vlan(vid) => format!("switch(config-vlan-{})# ", vid),
                CurrentContext::Interface(idx) => {
                    format!("switch(config-if-eth{})# ", idx)
                }
            },
            None => "switch(config)# ".to_string(),
        }
    };
    shell.print(&prompt);
}

// =====================================================================
// Input handling — character-at-a-time sub-shell loop.
// =====================================================================

/// Dispatch for `OsState::Switch` — called from `on_input`.
pub fn handle_input(shell: &mut ShellInstance, input: &str) {
    for &b in input.as_bytes() {
        // Ctrl+T (0x14) is handled upstream by `dispatch_interrupt` →
        // `reset_to_shell` before getting here, but we defensively break.
        if b == 0x14 {
            return;
        }
        match b {
            b'\n' | b'\r' => {
                let line = unsafe {
                    let state = match SWITCH_STATE.as_mut() {
                        Some(s) => s,
                        None => return,
                    };
                    let text = core::str::from_utf8(&state.input_buf[..state.input_len])
                        .unwrap_or("");
                    let owned = text.to_string();
                    state.input_len = 0;
                    owned
                };
                // Advance to the next line so any command output lands
                // below the echoed command instead of next to it.
                shell.println("");
                exec_command(shell, line.trim());
                if shell.state != OsState::Switch {
                    // `exit`/`end`/`quit` already printed the exit message
                    // and a fresh main-shell prompt inside `exit_switch`.
                    // Do NOT print a second prompt here. Also stop
                    // consuming further bytes in this on_input call so
                    // leftover input from a paste is re-dispatched by
                    // on_input's outer loop against the new shell state.
                    return;
                }
                print_prompt(shell);
            }
            8 | 127 => {
                let should_erase = unsafe {
                    match SWITCH_STATE.as_mut() {
                        Some(state) if state.input_len > 0 => {
                            state.input_len -= 1;
                            true
                        }
                        _ => false,
                    }
                };
                if should_erase {
                    shell.print("\x08 \x08");
                }
            }
            _ if b >= 32 && b < 127 => {
                let accepted = unsafe {
                    match SWITCH_STATE.as_mut() {
                        Some(state) if state.input_len < state.input_buf.len() => {
                            state.input_buf[state.input_len] = b;
                            state.input_len += 1;
                            true
                        }
                        _ => false,
                    }
                };
                if accepted {
                    let ch = [b];
                    if let Ok(s) = core::str::from_utf8(&ch) {
                        shell.print(s);
                    }
                }
            }
            _ => {}
        }
    }
}

// =====================================================================
// Command dispatch
// =====================================================================

fn exec_command(shell: &mut ShellInstance, line: &str) {
    if line.is_empty() {
        return;
    }
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.is_empty() {
        return;
    }

    // Snapshot the current context so we don't have to thread it
    // through every command. Read-only show commands work in every
    // context. `end`/`quit` always leave switch mode entirely.
    // `exit` pops one level (vlan/if -> top, top -> leave switch).
    let context = unsafe {
        SWITCH_STATE.as_ref().map(|s| s.context).unwrap_or(CurrentContext::Top)
    };

    // Universal commands available in any context.
    match tokens[0] {
        "end" | "quit" => {
            exit_switch(shell);
            return;
        }
        "exit" => {
            cmd_exit(shell);
            return;
        }
        "help" | "?" => {
            cmd_help(shell, context);
            return;
        }
        "show" => {
            cmd_show(shell, &tokens[1..]);
            return;
        }
        _ => {}
    }

    match context {
        CurrentContext::Top => exec_top(shell, &tokens, line),
        CurrentContext::Vlan(vid) => exec_vlan_ctx(shell, &tokens, line, vid),
        CurrentContext::Interface(idx) => exec_if_ctx(shell, &tokens, line, idx),
    }
}

fn exec_top(shell: &mut ShellInstance, tokens: &[&str], line: &str) {
    match tokens[0] {
        "mac-address-table" => cmd_mac_address_table(shell, &tokens[1..]),
        "no" => cmd_no(shell, &tokens[1..]),
        "static-mac" => cmd_static_mac(shell, &tokens[1..], false),
        "clear" => cmd_clear(shell, &tokens[1..]),
        "vlan" => cmd_vlan(shell, &tokens[1..]),
        "interface" => cmd_interface(shell, &tokens[1..]),
        "on" => cmd_cli_on(shell),
        "off" => cmd_cli_off(shell),
        "write" => cmd_write(shell, &tokens[1..]),
        _ => {
            let msg = format!("% Invalid input: {}", line);
            shell.println(&msg);
        }
    }
}

/// In-CLI `on` — promotes the current session to persistent so that
/// `exit` will leave the CLI without tearing down the switch.
fn cmd_cli_on(shell: &mut ShellInstance) {
    unsafe {
        if let Some(s) = SWITCH_STATE.as_mut() {
            s.persist_on_exit = true;
        }
    }
    shell.println("Switching will persist after exit. Use 'off' or 'switch off' to stop.");
}

/// In-CLI `off` — identical to running `switch off` from the main shell.
fn cmd_cli_off(shell: &mut ShellInstance) {
    stop(shell);
}

fn exec_vlan_ctx(shell: &mut ShellInstance, tokens: &[&str], line: &str, vid: u16) {
    match tokens[0] {
        "name" => cmd_vlan_name(shell, vid, &tokens[1..]),
        "description" => cmd_vlan_description(shell, vid, &tokens[1..]),
        "no" => cmd_vlan_no(shell, vid, &tokens[1..]),
        "shutdown" => cmd_vlan_set_active(shell, vid, false),
        _ => {
            let msg = format!("% Invalid input in vlan context: {}", line);
            shell.println(&msg);
        }
    }
}

fn exec_if_ctx(shell: &mut ShellInstance, tokens: &[&str], line: &str, idx: usize) {
    match tokens[0] {
        "no" => cmd_if_no(shell, idx, &tokens[1..]),
        "vlan" => cmd_if_vlan(shell, idx, &tokens[1..]),
        _ => {
            let msg = format!("% Invalid input in interface context: {}", line);
            shell.println(&msg);
        }
    }
}

/// `exit` semantics: pop one nested context, or leave the CLI from Top.
/// During config-file replay (`loading == true`) the top-level exit is
/// intentionally a no-op — a saved config file ends each block with
/// `exit`, and without this guard the first such `exit` would call
/// `exit_switch` and tear down the switch we just loaded.
fn cmd_exit(shell: &mut ShellInstance) {
    let (at_top, loading) = unsafe {
        match SWITCH_STATE.as_ref() {
            Some(s) => (s.context == CurrentContext::Top, s.loading),
            None => return,
        }
    };
    if !at_top {
        unsafe {
            if let Some(s) = SWITCH_STATE.as_mut() {
                s.context = CurrentContext::Top;
            }
        }
        return;
    }
    if loading {
        return;
    }
    exit_switch(shell);
}

fn cmd_help(shell: &mut ShellInstance, context: CurrentContext) {
    match context {
        CurrentContext::Top => {
            shell.println("Switch configuration commands:");
            shell.println("  mac-address-table age-time <15-1000000>");
            shell.println("  no mac-address-table age-time");
            shell.println("  static-mac <mac> vlan <vlan-id> port <port-num>");
            shell.println("  no static-mac <mac> vlan <vlan-id> port <port-num>");
            shell.println("  vlan <vlan-id>           Enter VLAN config (creates if absent)");
            shell.println("  vlan <start>-<end>       Bulk-create a range of VLANs");
            shell.println("  no vlan <vlan-id>        Delete a VLAN");
            shell.println("  interface <port|ethN>    Enter interface config");
            shell.println("  on                       Mark session persistent (exit keeps forwarding)");
            shell.println("  off                      Stop switching and leave the CLI");
            shell.println("  write memory             Save running config to /switch.cfg");
            shell.println("  show mac-address-table [dynamic|static|vlan|port|address|count|mac-move ...]");
            shell.println("  show vlan");
            shell.println("  show vlan <vlan-id>");
            shell.println("  show vlan summary");
            shell.println("  show vlan port <port-num>");
            shell.println("  clear mac-address-table dynamic [vlan <id> | port <n> | address <mac>]");
            shell.println("  exit / end / quit        Leave CLI (preserves forwarding if persistent)");
        }
        CurrentContext::Vlan(_) => {
            shell.println("VLAN configuration commands:");
            shell.println("  name <name>");
            shell.println("  description <text>");
            shell.println("  no shutdown              Activate the VLAN");
            shell.println("  shutdown                 Deactivate the VLAN");
            shell.println("  exit                     Return to the parent context");
            shell.println("  end                      Leave switch mode");
        }
        CurrentContext::Interface(_) => {
            shell.println("Interface configuration commands:");
            shell.println("  no routing               Convert to L2 (required first)");
            shell.println("  vlan access <vlan-id>");
            shell.println("  no vlan access <vlan-id>");
            shell.println("  vlan trunk native <vlan-id> [tag]");
            shell.println("  no vlan trunk native <vlan-id>");
            shell.println("  vlan trunk allowed <vlan-list|all>");
            shell.println("  no vlan trunk allowed <vlan-list>");
            shell.println("  exit                     Return to the parent context");
            shell.println("  end                      Leave switch mode");
        }
    }
}

fn cmd_mac_address_table(shell: &mut ShellInstance, args: &[&str]) {
    // `mac-address-table age-time <secs>` is the only form; everything else
    // falls under `show mac-address-table` or `clear mac-address-table`.
    if args.len() == 2 && args[0] == "age-time" {
        match args[1].parse::<u32>() {
            Ok(n) if (MIN_AGE_TIME_SECS..=MAX_AGE_TIME_SECS).contains(&n) => {
                set_age_time(shell, n);
            }
            Ok(_) => {
                shell.println("% age-time out of range (15-1000000)");
            }
            Err(_) => {
                shell.println("% Invalid age-time value");
            }
        }
        return;
    }
    shell.println("% Incomplete command. Try 'help'.");
}

fn set_age_time(shell: &mut ShellInstance, secs: u32) {
    unsafe {
        if let Some(state) = SWITCH_STATE.as_mut() {
            state.age_time_secs = secs;
            let msg = format!("MAC age-time set to {} seconds", secs);
            shell.println(&msg);
        }
    }
}

fn cmd_no(shell: &mut ShellInstance, args: &[&str]) {
    if args.is_empty() {
        shell.println("% Incomplete command.");
        return;
    }
    match args[0] {
        "mac-address-table" => {
            if args.len() == 2 && args[1] == "age-time" {
                set_age_time(shell, DEFAULT_AGE_TIME_SECS);
                return;
            }
            shell.println("% Incomplete command.");
        }
        "static-mac" => cmd_static_mac(shell, &args[1..], true),
        "vlan" => cmd_no_vlan(shell, &args[1..]),
        _ => {
            let msg = format!("% Invalid input: no {}", args.join(" "));
            shell.println(&msg);
        }
    }
}

/// `static-mac <mac> vlan <id> port <n>` / `no static-mac <mac> vlan <id> port <n>`
fn cmd_static_mac(shell: &mut ShellInstance, args: &[&str], remove: bool) {
    if args.len() != 5 || args[1] != "vlan" || args[3] != "port" {
        shell.println("% Usage: static-mac <mac> vlan <vlan-id> port <port-num>");
        return;
    }
    let mac = match parse_mac(args[0]) {
        Some(m) => m,
        None => {
            shell.println("% Invalid MAC address");
            return;
        }
    };
    let vlan: u16 = match args[2].parse() {
        Ok(v) if v <= 4094 => v,
        _ => {
            shell.println("% Invalid VLAN id (1-4094)");
            return;
        }
    };
    let port: usize = match args[4].parse() {
        Ok(p) => p,
        Err(_) => {
            shell.println("% Invalid port number");
            return;
        }
    };

    let iface_count = unsafe {
        match SWITCH_STATE.as_ref() {
            Some(s) => s.iface_count,
            None => return,
        }
    };
    if port >= iface_count {
        let msg = format!("% Port {} out of range (0-{})", port, iface_count.saturating_sub(1));
        shell.println(&msg);
        return;
    }

    unsafe {
        let state = match SWITCH_STATE.as_mut() {
            Some(s) => s,
            None => return,
        };
        if remove {
            let before = state.entries.len();
            state.entries.retain(|e| !(e.is_static && e.mac == mac && e.vlan == vlan && e.port == port));
            if state.entries.len() == before {
                shell.println("% No matching static entry");
            } else {
                shell.println("Static MAC entry removed.");
            }
            return;
        }

        let now_ms = net::NetStack::get().map(|s| s.now_ms).unwrap_or(0);

        if let Some(idx) = state.find(&mac, vlan) {
            let e = &mut state.entries[idx];
            e.is_static = true;
            e.port = port;
            e.learned_ms = now_ms;
            e.prev_port = None;
            e.move_count = 0;
            e.last_move_ms = 0;
            shell.println("Static MAC entry updated.");
            return;
        }

        if state.entries.len() >= MAX_MAC_ENTRIES {
            // Evict oldest dynamic to make room for the static entry.
            let mut oldest: Option<(usize, i64)> = None;
            for (i, e) in state.entries.iter().enumerate() {
                if e.is_static { continue; }
                if oldest.map(|(_, t)| e.learned_ms < t).unwrap_or(true) {
                    oldest = Some((i, e.learned_ms));
                }
            }
            if let Some((idx, _)) = oldest {
                state.entries.remove(idx);
            } else {
                shell.println("% MAC address table full (all entries static)");
                return;
            }
        }

        state.entries.push(MacEntry {
            mac,
            vlan,
            port,
            is_static: true,
            learned_ms: now_ms,
            prev_port: None,
            move_count: 0,
            last_move_ms: 0,
        });
        shell.println("Static MAC entry added.");
    }
}

// =====================================================================
// show mac-address-table ...
// =====================================================================

fn cmd_show(shell: &mut ShellInstance, args: &[&str]) {
    if args.is_empty() {
        shell.println("% Unknown show command. Try 'help'.");
        return;
    }
    if args[0] == "vlan" {
        cmd_show_vlan(shell, &args[1..]);
        return;
    }
    if args[0] != "mac-address-table" {
        shell.println("% Unknown show command. Try 'help'.");
        return;
    }
    let rest = &args[1..];
    if rest.is_empty() {
        show_full_table(shell);
        return;
    }
    match rest[0] {
        "dynamic" => show_filtered(shell, Filter::dynamic(&rest[1..])),
        "static" => show_filtered(shell, Filter::only_static()),
        "vlan" => {
            if rest.len() != 2 {
                shell.println("% Usage: show mac-address-table vlan <vlan-id>");
                return;
            }
            match rest[1].parse::<u16>() {
                Ok(v) => show_filtered(shell, Filter::by_vlan(v)),
                Err(_) => shell.println("% Invalid VLAN id"),
            }
        }
        "port" => {
            if rest.len() != 2 {
                shell.println("% Usage: show mac-address-table port <port-num>");
                return;
            }
            match rest[1].parse::<usize>() {
                Ok(p) => show_filtered(shell, Filter::by_port(p)),
                Err(_) => shell.println("% Invalid port number"),
            }
        }
        "address" => {
            if rest.len() != 2 {
                shell.println("% Usage: show mac-address-table address <mac>");
                return;
            }
            match parse_mac(rest[1]) {
                Some(m) => show_filtered(shell, Filter::by_address(m)),
                None => shell.println("% Invalid MAC address"),
            }
        }
        "count" => show_count(shell, &rest[1..]),
        "mac-move" => show_mac_move(shell, &rest[1..]),
        _ => shell.println("% Unknown show mac-address-table option. Try 'help'."),
    }
}

#[derive(Clone, Copy)]
enum EntryKind {
    Any,
    Dynamic,
    Static,
}

#[derive(Clone, Copy, Default)]
struct Filter {
    kind: Option<EntryKind>,
    port: Option<usize>,
    vlan: Option<u16>,
    address: Option<MacAddr>,
}

impl Filter {
    fn dynamic(rest: &[&str]) -> Self {
        let mut f = Filter {
            kind: Some(EntryKind::Dynamic),
            ..Filter::default()
        };
        if rest.is_empty() {
            return f;
        }
        if rest.len() == 2 && rest[0] == "port" {
            if let Ok(p) = rest[1].parse::<usize>() {
                f.port = Some(p);
            }
        } else if rest.len() == 2 && rest[0] == "vlan" {
            if let Ok(v) = rest[1].parse::<u16>() {
                f.vlan = Some(v);
            }
        }
        f
    }

    fn only_static() -> Self {
        Filter { kind: Some(EntryKind::Static), ..Filter::default() }
    }

    fn by_vlan(v: u16) -> Self {
        Filter { vlan: Some(v), ..Filter::default() }
    }

    fn by_port(p: usize) -> Self {
        Filter { port: Some(p), ..Filter::default() }
    }

    fn by_address(m: MacAddr) -> Self {
        Filter { address: Some(m), ..Filter::default() }
    }

    fn matches(&self, e: &MacEntry) -> bool {
        match self.kind.unwrap_or(EntryKind::Any) {
            EntryKind::Any => {}
            EntryKind::Dynamic => if e.is_static { return false; },
            EntryKind::Static => if !e.is_static { return false; },
        }
        if let Some(p) = self.port { if e.port != p { return false; } }
        if let Some(v) = self.vlan { if e.vlan != v { return false; } }
        if let Some(m) = self.address { if e.mac != m { return false; } }
        true
    }
}

fn show_full_table(shell: &mut ShellInstance) {
    unsafe {
        let state = match SWITCH_STATE.as_ref() {
            Some(s) => s,
            None => return,
        };
        let header = format!("MAC age-time            : {} seconds", state.age_time_secs);
        shell.println(&header);
        let count = format!("Number of MAC addresses : {}", state.entries.len());
        shell.println(&count);
        shell.println("");
        print_table_header(shell);
        for e in &state.entries {
            print_entry(shell, e);
        }
    }
}

fn show_filtered(shell: &mut ShellInstance, filter: Filter) {
    unsafe {
        let state = match SWITCH_STATE.as_ref() {
            Some(s) => s,
            None => return,
        };
        print_table_header(shell);
        let mut count = 0usize;
        for e in &state.entries {
            if filter.matches(e) {
                print_entry(shell, e);
                count += 1;
            }
        }
        if count == 0 {
            shell.println("(no matching entries)");
        }
    }
}

fn show_count(shell: &mut ShellInstance, rest: &[&str]) {
    let mut filter = Filter::default();
    if !rest.is_empty() {
        match rest[0] {
            "dynamic" => filter.kind = Some(EntryKind::Dynamic),
            "port" => {
                if rest.len() != 2 {
                    shell.println("% Usage: show mac-address-table count port <port-num>");
                    return;
                }
                match rest[1].parse::<usize>() {
                    Ok(p) => filter.port = Some(p),
                    Err(_) => {
                        shell.println("% Invalid port number");
                        return;
                    }
                }
            }
            "vlan" => {
                if rest.len() != 2 {
                    shell.println("% Usage: show mac-address-table count vlan <vlan-id>");
                    return;
                }
                match rest[1].parse::<u16>() {
                    Ok(v) => filter.vlan = Some(v),
                    Err(_) => {
                        shell.println("% Invalid VLAN id");
                        return;
                    }
                }
            }
            _ => {
                shell.println("% Usage: show mac-address-table count [dynamic | port <n> | vlan <id>]");
                return;
            }
        }
    }
    unsafe {
        let state = match SWITCH_STATE.as_ref() {
            Some(s) => s,
            None => return,
        };
        let count = state.entries.iter().filter(|e| filter.matches(e)).count();
        let msg = format!("Number of entries: {}", count);
        shell.println(&msg);
    }
}

fn show_mac_move(shell: &mut ShellInstance, rest: &[&str]) {
    // Optional filter: `address <mac> vlan <id>` or `vlan <id>`.
    let mut addr_filter: Option<MacAddr> = None;
    let mut vlan_filter: Option<u16> = None;

    if !rest.is_empty() {
        let mut i = 0;
        while i < rest.len() {
            match rest[i] {
                "address" => {
                    if i + 1 >= rest.len() {
                        shell.println("% Missing MAC after 'address'");
                        return;
                    }
                    match parse_mac(rest[i + 1]) {
                        Some(m) => addr_filter = Some(m),
                        None => {
                            shell.println("% Invalid MAC address");
                            return;
                        }
                    }
                    i += 2;
                }
                "vlan" => {
                    if i + 1 >= rest.len() {
                        shell.println("% Missing VLAN id after 'vlan'");
                        return;
                    }
                    match rest[i + 1].parse::<u16>() {
                        Ok(v) => vlan_filter = Some(v),
                        Err(_) => {
                            shell.println("% Invalid VLAN id");
                            return;
                        }
                    }
                    i += 2;
                }
                _ => {
                    shell.println("% Unknown option. Try 'show mac-address-table mac-move [address <mac> vlan <id> | vlan <id>]'");
                    return;
                }
            }
        }
    }

    unsafe {
        let state = match SWITCH_STATE.as_ref() {
            Some(s) => s,
            None => return,
        };
        shell.println("MAC Address         VLAN    Curr  Prev  Moves  Last-Move");
        shell.println("-----------------------------------------------------------");
        let mut any = false;
        for e in &state.entries {
            if e.move_count == 0 {
                continue;
            }
            if let Some(m) = addr_filter { if e.mac != m { continue; } }
            if let Some(v) = vlan_filter { if e.vlan != v { continue; } }
            let prev = e.prev_port.map(|p| format!("{:<4}", p)).unwrap_or_else(|| "-   ".to_string());
            let line = format!(
                "{}   {:<6}  {:<4}  {}  {:<5}  {}",
                format_mac(&e.mac),
                e.vlan,
                e.port,
                prev,
                e.move_count,
                format_time(e.last_move_ms),
            );
            shell.println(&line);
            any = true;
        }
        if !any {
            shell.println("(no MAC moves recorded)");
        }
    }
}

fn print_table_header(shell: &mut ShellInstance) {
    shell.println("MAC Address         VLAN    Type     Port");
    shell.println("-----------------------------------------");
}

fn print_entry(shell: &mut ShellInstance, e: &MacEntry) {
    let kind = if e.is_static { "static" } else { "dynamic" };
    let line = format!(
        "{}   {:<6}  {:<7}  {}",
        format_mac(&e.mac),
        e.vlan,
        kind,
        e.port,
    );
    shell.println(&line);
}

// =====================================================================
// clear mac-address-table ...
// =====================================================================

fn cmd_clear(shell: &mut ShellInstance, args: &[&str]) {
    if args.len() < 2 || args[0] != "mac-address-table" || args[1] != "dynamic" {
        shell.println("% Usage: clear mac-address-table dynamic [vlan <id> | port <n> | address <mac>]");
        return;
    }
    let rest = &args[2..];
    unsafe {
        let state = match SWITCH_STATE.as_mut() {
            Some(s) => s,
            None => return,
        };
        let before = state.entries.len();
        if rest.is_empty() {
            state.entries.retain(|e| e.is_static);
            let removed = before - state.entries.len();
            let msg = format!("Cleared {} dynamic entries.", removed);
            shell.println(&msg);
            return;
        }
        match rest[0] {
            "vlan" => {
                if rest.len() != 2 {
                    shell.println("% Usage: clear mac-address-table dynamic vlan <vlan-id>");
                    return;
                }
                match rest[1].parse::<u16>() {
                    Ok(v) => {
                        state.entries.retain(|e| e.is_static || e.vlan != v);
                        let removed = before - state.entries.len();
                        let msg = format!("Cleared {} dynamic entries in VLAN {}.", removed, v);
                        shell.println(&msg);
                    }
                    Err(_) => shell.println("% Invalid VLAN id"),
                }
            }
            "port" => {
                if rest.len() != 2 {
                    shell.println("% Usage: clear mac-address-table dynamic port <port-num>");
                    return;
                }
                match rest[1].parse::<usize>() {
                    Ok(p) => {
                        state.entries.retain(|e| e.is_static || e.port != p);
                        let removed = before - state.entries.len();
                        let msg = format!("Cleared {} dynamic entries on port {}.", removed, p);
                        shell.println(&msg);
                    }
                    Err(_) => shell.println("% Invalid port number"),
                }
            }
            "address" => {
                if rest.len() != 2 {
                    shell.println("% Usage: clear mac-address-table dynamic address <mac>");
                    return;
                }
                match parse_mac(rest[1]) {
                    Some(m) => {
                        state.entries.retain(|e| e.is_static || e.mac != m);
                        let removed = before - state.entries.len();
                        let msg = format!("Cleared {} dynamic entries matching {}.", removed, format_mac(&m));
                        shell.println(&msg);
                    }
                    None => shell.println("% Invalid MAC address"),
                }
            }
            _ => shell.println("% Usage: clear mac-address-table dynamic [vlan <id> | port <n> | address <mac>]"),
        }
    }
}

// =====================================================================
// Phase 2 — VLAN database + interface/vlan context commands
// =====================================================================

/// `vlan <id>` or `vlan <start>-<end>` at top-level.
fn cmd_vlan(shell: &mut ShellInstance, args: &[&str]) {
    if args.len() != 1 {
        shell.println("% Usage: vlan <vlan-id> | vlan <start>-<end>");
        return;
    }
    let arg = args[0];
    if let Some((a, b)) = arg.split_once('-') {
        // Range form — bulk create, no context entered.
        let start: u16 = match a.parse() {
            Ok(v) => v,
            Err(_) => { shell.println("% Invalid VLAN range"); return; }
        };
        let end: u16 = match b.parse() {
            Ok(v) => v,
            Err(_) => { shell.println("% Invalid VLAN range"); return; }
        };
        if start < MIN_VID || end > MAX_VID || start > end {
            shell.println("% Invalid VLAN range (1-4094, start <= end)");
            return;
        }
        unsafe {
            if let Some(state) = SWITCH_STATE.as_mut() {
                let mut created = 0u32;
                for v in start..=end {
                    if state.vlans.contains_key(&v) {
                        continue;
                    }
                    state.vlans.insert(v, Vlan {
                        vid: v,
                        name: None,
                        description: None,
                        active: false,
                    });
                    created += 1;
                }
                let msg = format!("Created {} VLANs (range {}-{}).", created, start, end);
                shell.println(&msg);
            }
        }
        return;
    }

    // Single VLAN form — create if absent + enter context.
    let vid: u16 = match arg.parse() {
        Ok(v) => v,
        Err(_) => { shell.println("% Invalid VLAN id"); return; }
    };
    if vid < MIN_VID || vid > MAX_VID {
        shell.println("% VLAN id out of range (1-4094)");
        return;
    }
    unsafe {
        if let Some(state) = SWITCH_STATE.as_mut() {
            state.vlans.entry(vid).or_insert(Vlan {
                vid,
                name: None,
                description: None,
                active: false,
            });
            state.context = CurrentContext::Vlan(vid);
        }
    }
}

/// `no vlan <id>` at top-level.
fn cmd_no_vlan(shell: &mut ShellInstance, args: &[&str]) {
    if args.len() != 1 {
        shell.println("% Usage: no vlan <vlan-id>");
        return;
    }
    let vid: u16 = match args[0].parse() {
        Ok(v) => v,
        Err(_) => { shell.println("% Invalid VLAN id"); return; }
    };
    if vid == DEFAULT_PVID {
        shell.println("% Cannot delete VLAN 1 (default)");
        return;
    }
    unsafe {
        if let Some(state) = SWITCH_STATE.as_mut() {
            if !state.vlans.contains_key(&vid) {
                let msg = format!("% VLAN {} does not exist", vid);
                shell.println(&msg);
                return;
            }
            let in_use = state.vlan_in_use_on(vid);
            if let Some(&p) = in_use.first() {
                let msg = format!("% VLAN {} is in use on port eth{}", vid, p);
                shell.println(&msg);
                return;
            }
            state.vlans.remove(&vid);
            // Also drop any MAC entries learned in that VLAN.
            state.entries.retain(|e| e.vlan != vid);
            let msg = format!("VLAN {} deleted.", vid);
            shell.println(&msg);
        }
    }
}

/// `interface <port>` at top-level.
fn cmd_interface(shell: &mut ShellInstance, args: &[&str]) {
    if args.len() != 1 {
        shell.println("% Usage: interface <port-num | ethN>");
        return;
    }
    let iface_count = unsafe {
        SWITCH_STATE.as_ref().map(|s| s.iface_count).unwrap_or(0)
    };
    let idx = match parse_port_id(args[0], iface_count) {
        Some(i) => i,
        None => {
            let msg = format!("% Invalid port (must be 0..{} or eth0..eth{})",
                iface_count.saturating_sub(1), iface_count.saturating_sub(1));
            shell.println(&msg);
            return;
        }
    };
    unsafe {
        if let Some(state) = SWITCH_STATE.as_mut() {
            state.context = CurrentContext::Interface(idx);
        }
    }
}

// ----- VLAN context commands -----

fn cmd_vlan_name(shell: &mut ShellInstance, vid: u16, args: &[&str]) {
    if args.is_empty() {
        shell.println("% Usage: name <name>");
        return;
    }
    let name = args.join(" ");
    unsafe {
        if let Some(state) = SWITCH_STATE.as_mut() {
            if let Some(v) = state.vlans.get_mut(&vid) {
                v.name = Some(name);
                shell.println("VLAN name set.");
            }
        }
    }
}

fn cmd_vlan_description(shell: &mut ShellInstance, vid: u16, args: &[&str]) {
    if args.is_empty() {
        shell.println("% Usage: description <text>");
        return;
    }
    let desc = args.join(" ");
    unsafe {
        if let Some(state) = SWITCH_STATE.as_mut() {
            if let Some(v) = state.vlans.get_mut(&vid) {
                v.description = Some(desc);
                shell.println("VLAN description set.");
            }
        }
    }
}

fn cmd_vlan_set_active(shell: &mut ShellInstance, vid: u16, active: bool) {
    if !active && vid == DEFAULT_PVID {
        shell.println("% Cannot shutdown VLAN 1 (default)");
        return;
    }
    unsafe {
        if let Some(state) = SWITCH_STATE.as_mut() {
            if let Some(v) = state.vlans.get_mut(&vid) {
                v.active = active;
                let msg = if active { "VLAN activated." } else { "VLAN deactivated." };
                shell.println(msg);
            }
        }
    }
}

fn cmd_vlan_no(shell: &mut ShellInstance, vid: u16, args: &[&str]) {
    if args.len() == 1 && args[0] == "shutdown" {
        cmd_vlan_set_active(shell, vid, true);
        return;
    }
    let msg = format!("% Invalid input in vlan context: no {}", args.join(" "));
    shell.println(&msg);
}

// ----- Interface context commands -----

fn cmd_if_no(shell: &mut ShellInstance, idx: usize, args: &[&str]) {
    if args.is_empty() {
        shell.println("% Incomplete command.");
        return;
    }
    match args[0] {
        "routing" => {
            // `no routing` — convert L3 -> L2 access vlan 1.
            if args.len() != 1 {
                shell.println("% Usage: no routing");
                return;
            }
            handle_no_routing(shell, idx);
        }
        "vlan" => cmd_if_vlan_no(shell, idx, &args[1..]),
        _ => {
            let msg = format!("% Invalid input in interface context: no {}", args.join(" "));
            shell.println(&msg);
        }
    }
}

fn cmd_if_vlan(shell: &mut ShellInstance, idx: usize, args: &[&str]) {
    if args.is_empty() {
        shell.println("% Usage: vlan access <id> | vlan trunk native <id> [tag] | vlan trunk allowed ...");
        return;
    }
    match args[0] {
        "access" => cmd_if_vlan_access(shell, idx, &args[1..]),
        "trunk" => cmd_if_vlan_trunk(shell, idx, &args[1..]),
        _ => {
            let msg = format!("% Invalid input: vlan {}", args.join(" "));
            shell.println(&msg);
        }
    }
}

fn ensure_l2(shell: &mut ShellInstance, idx: usize) -> bool {
    let routed = unsafe {
        SWITCH_STATE.as_ref()
            .map(|s| matches!(s.ports[idx].mode, PortMode::Routed))
            .unwrap_or(true)
    };
    if routed {
        shell.println("% Run 'no routing' first to convert this port to L2");
        false
    } else {
        true
    }
}

fn handle_no_routing(shell: &mut ShellInstance, idx: usize) {
    unsafe {
        if let Some(state) = SWITCH_STATE.as_mut() {
            state.ports[idx].mode = PortMode::Access { vid: DEFAULT_PVID };
            shell.println("Port converted to L2 (access vlan 1).");
        }
    }
}

fn cmd_if_vlan_access(shell: &mut ShellInstance, idx: usize, args: &[&str]) {
    if args.len() != 1 {
        shell.println("% Usage: vlan access <vlan-id>");
        return;
    }
    if !ensure_l2(shell, idx) {
        return;
    }
    let vid: u16 = match args[0].parse() {
        Ok(v) => v,
        Err(_) => { shell.println("% Invalid VLAN id"); return; }
    };
    if vid < MIN_VID || vid > MAX_VID {
        shell.println("% VLAN id out of range (1-4094)");
        return;
    }
    unsafe {
        if let Some(state) = SWITCH_STATE.as_mut() {
            if !state.vlans.contains_key(&vid) {
                let msg = format!("% Warning: VLAN {} does not exist (creating as inactive)", vid);
                shell.println(&msg);
                state.vlans.insert(vid, Vlan {
                    vid,
                    name: None,
                    description: None,
                    active: false,
                });
            } else if !state.vlan_active(vid) {
                let msg = format!("% Warning: VLAN {} is shutdown", vid);
                shell.println(&msg);
            }
            state.ports[idx].mode = PortMode::Access { vid };
            let msg = format!("Access VLAN set to {}.", vid);
            shell.println(&msg);
        }
    }
}

fn cmd_if_vlan_trunk(shell: &mut ShellInstance, idx: usize, args: &[&str]) {
    if args.is_empty() {
        shell.println("% Usage: vlan trunk native <id> [tag] | vlan trunk allowed <list|all>");
        return;
    }
    match args[0] {
        "native" => cmd_if_trunk_native(shell, idx, &args[1..], false),
        "allowed" => cmd_if_trunk_allowed(shell, idx, &args[1..], false),
        _ => {
            shell.println("% Usage: vlan trunk native <id> [tag] | vlan trunk allowed <list|all>");
        }
    }
}

fn cmd_if_vlan_no(shell: &mut ShellInstance, idx: usize, args: &[&str]) {
    if args.is_empty() || args[0] != "vlan" {
        shell.println("% Invalid input.");
        return;
    }
    let rest = &args[1..];
    if rest.is_empty() {
        shell.println("% Incomplete command.");
        return;
    }
    match rest[0] {
        "access" => {
            // `no vlan access <id>` resets to default-after-no-routing.
            if !ensure_l2(shell, idx) {
                return;
            }
            unsafe {
                if let Some(state) = SWITCH_STATE.as_mut() {
                    state.ports[idx].mode = PortMode::Access { vid: DEFAULT_PVID };
                    shell.println("Access VLAN reset to default (1).");
                }
            }
        }
        "trunk" => {
            if rest.len() < 2 {
                shell.println("% Incomplete command.");
                return;
            }
            match rest[1] {
                "native" => cmd_if_trunk_native(shell, idx, &rest[2..], true),
                "allowed" => cmd_if_trunk_allowed(shell, idx, &rest[2..], true),
                _ => shell.println("% Invalid input."),
            }
        }
        _ => shell.println("% Invalid input."),
    }
}

fn cmd_if_trunk_native(shell: &mut ShellInstance, idx: usize, args: &[&str], remove: bool) {
    if !ensure_l2(shell, idx) {
        return;
    }
    if args.is_empty() {
        shell.println("% Usage: vlan trunk native <vlan-id> [tag]");
        return;
    }
    let vid: u16 = match args[0].parse() {
        Ok(v) => v,
        Err(_) => { shell.println("% Invalid VLAN id"); return; }
    };
    if vid < MIN_VID || vid > MAX_VID {
        shell.println("% VLAN id out of range (1-4094)");
        return;
    }
    let tag = args.len() == 2 && args[1] == "tag";
    if args.len() > 1 && !tag {
        shell.println("% Usage: vlan trunk native <vlan-id> [tag]");
        return;
    }
    unsafe {
        if let Some(state) = SWITCH_STATE.as_mut() {
            if remove {
                // `no vlan trunk native <id>` resets native to 1.
                if let PortMode::Trunk { native, native_tag, allowed } =
                    state.ports[idx].mode.clone()
                {
                    let _ = native;
                    state.ports[idx].mode = PortMode::Trunk {
                        native: DEFAULT_PVID,
                        native_tag,
                        allowed,
                    };
                    shell.println("Native VLAN reset to 1.");
                } else {
                    shell.println("% Port is not in trunk mode");
                }
                return;
            }
            // Setting native implicitly puts the port in trunk mode and
            // sets allowed=All (per AOS-CX semantics).
            state.ports[idx].mode = PortMode::Trunk {
                native: vid,
                native_tag: tag,
                allowed: AllowedList::All,
            };
            let msg = if tag {
                format!("Trunk native VLAN set to {} (tagged).", vid)
            } else {
                format!("Trunk native VLAN set to {}.", vid)
            };
            shell.println(&msg);
        }
    }
}

fn cmd_if_trunk_allowed(shell: &mut ShellInstance, idx: usize, args: &[&str], remove: bool) {
    if !ensure_l2(shell, idx) {
        return;
    }
    if args.is_empty() {
        shell.println("% Usage: vlan trunk allowed <vlan-list> | all");
        return;
    }
    // Allowed-list ops require the port to already be in trunk mode.
    let in_trunk = unsafe {
        SWITCH_STATE.as_ref()
            .map(|s| matches!(s.ports[idx].mode, PortMode::Trunk { .. }))
            .unwrap_or(false)
    };
    if !in_trunk {
        shell.println("% Port is not in trunk mode (set 'vlan trunk native <id>' first)");
        return;
    }
    if args.len() == 1 && args[0] == "all" {
        if remove {
            shell.println("% Usage: no vlan trunk allowed <vlan-list>");
            return;
        }
        unsafe {
            if let Some(state) = SWITCH_STATE.as_mut() {
                if let PortMode::Trunk { native, native_tag, .. } = state.ports[idx].mode.clone() {
                    state.ports[idx].mode = PortMode::Trunk {
                        native,
                        native_tag,
                        allowed: AllowedList::All,
                    };
                    shell.println("Trunk allowed VLANs set to all.");
                }
            }
        }
        return;
    }
    let list_str = args.join("");
    let parsed = match parse_vlan_list(&list_str) {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("% {}", e);
            shell.println(&msg);
            return;
        }
    };
    unsafe {
        if let Some(state) = SWITCH_STATE.as_mut() {
            if let PortMode::Trunk { native, native_tag, allowed } = state.ports[idx].mode.clone() {
                let new_allowed = if remove {
                    // Materialize current set, then subtract.
                    let mut current: BTreeSet<u16> = match allowed {
                        AllowedList::All => state.vlans.keys().copied().collect(),
                        AllowedList::Some(s) => s,
                    };
                    for v in &parsed {
                        current.remove(v);
                    }
                    AllowedList::Some(current)
                } else {
                    AllowedList::Some(parsed)
                };
                state.ports[idx].mode = PortMode::Trunk {
                    native,
                    native_tag,
                    allowed: new_allowed,
                };
                let msg = if remove {
                    "Removed VLANs from trunk allowed list.".to_string()
                } else {
                    "Trunk allowed VLAN list set.".to_string()
                };
                shell.println(&msg);
            }
        }
    }
}

// =====================================================================
// `show vlan` command family
// =====================================================================

fn cmd_show_vlan(shell: &mut ShellInstance, args: &[&str]) {
    if args.is_empty() {
        show_vlan_full(shell);
        return;
    }
    if args[0] == "summary" && args.len() == 1 {
        show_vlan_summary(shell);
        return;
    }
    if args[0] == "port" {
        if args.len() != 2 {
            shell.println("% Usage: show vlan port <port-num>");
            return;
        }
        let iface_count = unsafe {
            SWITCH_STATE.as_ref().map(|s| s.iface_count).unwrap_or(0)
        };
        match parse_port_id(args[1], iface_count) {
            Some(idx) => show_vlan_port(shell, idx),
            None => shell.println("% Invalid port"),
        }
        return;
    }
    // `show vlan <id>`
    if args.len() == 1 {
        match args[0].parse::<u16>() {
            Ok(v) => show_vlan_one(shell, v),
            Err(_) => shell.println("% Usage: show vlan [<id> | summary | port <n>]"),
        }
    } else {
        shell.println("% Usage: show vlan [<id> | summary | port <n>]");
    }
}

fn show_vlan_full(shell: &mut ShellInstance) {
    print_vlan_header(shell);
    unsafe {
        if let Some(state) = SWITCH_STATE.as_ref() {
            for (vid, _) in state.vlans.iter() {
                print_vlan_row(shell, state, *vid);
            }
        }
    }
}

fn show_vlan_one(shell: &mut ShellInstance, vid: u16) {
    unsafe {
        let state = match SWITCH_STATE.as_ref() {
            Some(s) => s,
            None => return,
        };
        if !state.vlans.contains_key(&vid) {
            let msg = format!("% VLAN {} does not exist", vid);
            shell.println(&msg);
            return;
        }
        print_vlan_header(shell);
        print_vlan_row(shell, state, vid);

        // Member detail.
        let mut untagged = Vec::new();
        let mut tagged = Vec::new();
        for p in 0..state.iface_count {
            match state.port_membership(p, vid) {
                Some(false) => untagged.push(p),
                Some(true) => tagged.push(p),
                None => {}
            }
        }
        shell.println("");
        let utxt = format_port_list(&untagged);
        let ttxt = format_port_list(&tagged);
        let msg = format!("Untagged ports: {}", utxt);
        shell.println(&msg);
        let msg = format!("Tagged ports:   {}", ttxt);
        shell.println(&msg);
    }
}

fn show_vlan_summary(shell: &mut ShellInstance) {
    unsafe {
        let n = SWITCH_STATE.as_ref().map(|s| s.vlans.len()).unwrap_or(0);
        let msg = format!("Number of VLANs: {}", n);
        shell.println(&msg);
    }
}

fn show_vlan_port(shell: &mut ShellInstance, idx: usize) {
    unsafe {
        let state = match SWITCH_STATE.as_ref() {
            Some(s) => s,
            None => return,
        };
        let port_str = format!("Port: eth{}", idx);
        shell.println(&port_str);
        match &state.ports[idx].mode {
            PortMode::Routed => shell.println("Mode: routed (L3)"),
            PortMode::Access { vid } => {
                shell.println("Mode: access");
                let msg = format!("Untagged VLAN: {}", vid);
                shell.println(&msg);
                shell.println("Tagged VLANs: -");
            }
            PortMode::Trunk { native, native_tag, allowed } => {
                shell.println("Mode: trunk");
                let nt = if *native_tag { "tagged" } else { "untagged" };
                let msg = format!("Native VLAN: {} ({})", native, nt);
                shell.println(&msg);
                let allowed_str = match allowed {
                    AllowedList::All => "all".to_string(),
                    AllowedList::Some(s) => {
                        let mut v: Vec<String> = s.iter().map(|x| x.to_string()).collect();
                        v.sort();
                        if v.is_empty() { "-".to_string() } else { v.join(",") }
                    }
                };
                let msg = format!("Allowed VLANs: {}", allowed_str);
                shell.println(&msg);
            }
        }
    }
}

fn print_vlan_header(shell: &mut ShellInstance) {
    shell.println("VLAN  Name             Status   Reason       Ports (untagged / tagged)");
    shell.println("----  ---------------  -------  -----------  -----------------------------");
}

fn print_vlan_row(shell: &mut ShellInstance, state: &SwitchState, vid: u16) {
    let v = match state.vlans.get(&vid) {
        Some(v) => v,
        None => return,
    };
    let name = v.name.as_deref().unwrap_or("-");
    let mut untagged = Vec::new();
    let mut tagged = Vec::new();
    for p in 0..state.iface_count {
        match state.port_membership(p, vid) {
            Some(false) => untagged.push(p),
            Some(true) => tagged.push(p),
            None => {}
        }
    }
    // Simple status: admin-down if !active, no-member-up if active but
    // every member is link-down, else ok.
    let (status, reason) = if !v.active {
        ("down", "admin-down")
    } else {
        // Need stack to check link_up; without it we just say "ok".
        let any_member = !untagged.is_empty() || !tagged.is_empty();
        let any_up = if let Some(stack) = net::NetStack::get() {
            untagged.iter().chain(tagged.iter())
                .any(|&p| p < stack.iface_count && stack.interfaces[p].link_up)
        } else {
            false
        };
        if !any_member {
            ("down", "no-members")
        } else if !any_up {
            ("down", "no-member-up")
        } else {
            ("up", "ok")
        }
    };
    let utxt = format_port_list(&untagged);
    let ttxt = format_port_list(&tagged);
    let line = format!(
        "{:<4}  {:<15}  {:<7}  {:<11}  {} / {}",
        vid,
        truncate(name, 15),
        status,
        reason,
        utxt,
        ttxt,
    );
    shell.println(&line);
}

fn format_port_list(ports: &[usize]) -> String {
    if ports.is_empty() {
        return "-".to_string();
    }
    let parts: Vec<String> = ports.iter().map(|p| format!("eth{}", p)).collect();
    parts.join(",")
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect()
    }
}

// =====================================================================
// Formatting / parsing helpers
// =====================================================================

fn format_mac(mac: &MacAddr) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac.0[0], mac.0[1], mac.0[2], mac.0[3], mac.0[4], mac.0[5]
    )
}

/// Format an absolute `ms` timestamp as `HH:MM:SS.mmm`.
fn format_time(ms: i64) -> String {
    if ms <= 0 {
        return "-".to_string();
    }
    let secs = ms / 1_000;
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    let frac = ms % 1_000;
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, frac)
}

/// Parse a MAC address from `aa:bb:cc:dd:ee:ff` or `aa-bb-cc-dd-ee-ff`.
fn parse_mac(s: &str) -> Option<MacAddr> {
    let sep = if s.contains(':') {
        ':'
    } else if s.contains('-') {
        '-'
    } else {
        return None;
    };
    let parts: Vec<&str> = s.split(sep).collect();
    if parts.len() != 6 {
        return None;
    }
    let mut out = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        if p.len() != 2 {
            return None;
        }
        out[i] = u8::from_str_radix(p, 16).ok()?;
    }
    Some(MacAddr(out))
}

// =====================================================================
// Phase 2: VLAN tag mutation + parser helpers
// =====================================================================

/// Strip an 802.1Q tag if present. Returns `(buf_slice_to_use, len)`.
/// If the frame is already untagged, the *original* slice is returned
/// unchanged via the returned buf reference (still pointing at `out_buf`
/// after a copy — we always copy to make the lifetime convenient).
fn strip_tag<'a>(frame: &[u8], out_buf: &'a mut [u8; MAX_FRAME_SIZE]) -> (&'a [u8], usize) {
    if frame.len() < 14 {
        let n = frame.len().min(out_buf.len());
        out_buf[..n].copy_from_slice(&frame[..n]);
        return (&out_buf[..n], n);
    }
    let first_et = u16::from_be_bytes([frame[12], frame[13]]);
    if first_et != ETHERTYPE_8021Q || frame.len() < 18 {
        let n = frame.len().min(out_buf.len());
        out_buf[..n].copy_from_slice(&frame[..n]);
        return (&out_buf[..n], n);
    }
    // Drop the 4 bytes of [TPID 0x8100][TCI] sitting at offsets 12..16.
    let payload_len = frame.len() - 18;
    let total = 14 + payload_len;
    if total > out_buf.len() {
        return (&out_buf[..0], 0);
    }
    out_buf[..12].copy_from_slice(&frame[..12]);
    out_buf[12..14].copy_from_slice(&frame[16..18]);
    out_buf[14..14 + payload_len].copy_from_slice(&frame[18..]);
    (&out_buf[..total], total)
}

/// Ensure a frame carries an 802.1Q tag with the given VID. If already
/// tagged, the TCI is rewritten in place (PCP/DEI cleared). Writes the
/// resulting frame into `out_buf` and returns the new length.
fn ensure_tagged(frame: &[u8], vid: u16, out_buf: &mut [u8; MAX_FRAME_SIZE]) -> usize {
    if frame.len() < 14 {
        return 0;
    }
    let new_tci = VlanTag::new(vid).to_tci();
    let first_et = u16::from_be_bytes([frame[12], frame[13]]);
    if first_et == ETHERTYPE_8021Q && frame.len() >= 18 {
        // Already tagged — copy and overwrite TCI.
        let n = frame.len();
        if n > out_buf.len() {
            return 0;
        }
        out_buf[..n].copy_from_slice(&frame[..n]);
        out_buf[12..14].copy_from_slice(&ETHERTYPE_8021Q.to_be_bytes());
        out_buf[14..16].copy_from_slice(&new_tci.to_be_bytes());
        return n;
    }
    // Untagged — insert [TPID 0x8100][TCI] between offset 12 and 14.
    let payload_len = frame.len() - 14;
    let total = 18 + payload_len;
    if total > out_buf.len() {
        return 0;
    }
    out_buf[..12].copy_from_slice(&frame[..12]);
    out_buf[12..14].copy_from_slice(&ETHERTYPE_8021Q.to_be_bytes());
    out_buf[14..16].copy_from_slice(&new_tci.to_be_bytes());
    out_buf[16..18].copy_from_slice(&frame[12..14]);
    out_buf[18..18 + payload_len].copy_from_slice(&frame[14..]);
    total
}

/// Parse a VLAN allowed list like `10,20,30-40,100`.
fn parse_vlan_list(s: &str) -> Result<BTreeSet<u16>, String> {
    let mut out = BTreeSet::new();
    for raw in s.split(',') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        if let Some((a, b)) = token.split_once('-') {
            let start: u16 = a.trim().parse().map_err(|_| format!("Invalid VLAN list: {}", token))?;
            let end: u16 = b.trim().parse().map_err(|_| format!("Invalid VLAN list: {}", token))?;
            if start < MIN_VID || end > MAX_VID || start > end {
                return Err(format!("Invalid VLAN list: {}", token));
            }
            for v in start..=end {
                out.insert(v);
            }
        } else {
            let v: u16 = token.parse().map_err(|_| format!("Invalid VLAN list: {}", token))?;
            if v < MIN_VID || v > MAX_VID {
                return Err(format!("Invalid VLAN id: {}", token));
            }
            out.insert(v);
        }
    }
    if out.is_empty() {
        return Err("Empty VLAN list".to_string());
    }
    Ok(out)
}

/// Parse `ethN` or `N` into a port index. Returns None if out of range
/// or the syntax is unrecognized.
fn parse_port_id(s: &str, iface_count: usize) -> Option<usize> {
    let trimmed = s.trim();
    let n: usize = if let Some(rest) = trimmed.strip_prefix("eth") {
        rest.parse().ok()?
    } else {
        trimmed.parse().ok()?
    };
    if n < iface_count { Some(n) } else { None }
}

