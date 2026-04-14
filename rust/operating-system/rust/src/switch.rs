//! Layer 2 switch — Phase 1: Core L2 Forwarding & MAC Address Table.
//!
//! `switch` is a built-in sub-shell command. When entered, it enables
//! promiscuous mode on every interface and swaps the `IRQ_NETWORK` handler
//! so that incoming frames go through this module's forwarding path instead
//! of the normal local `NetStack::poll_rx`. Frames addressed to our own
//! MACs (or broadcast) are still re-injected into `NetStack::process_frame_on`
//! so management traffic (ARP for our IP, ping, SSH) continues to work while
//! the switch is running.
//!
//! Phase 1 implements:
//!   - Dynamic MAC learning keyed by (mac, vlan_id)
//!   - Unicast forwarding to known ports
//!   - Broadcast / unknown-unicast flooding
//!   - Static MAC entries (never aged)
//!   - Configurable aging time (default 300 seconds)
//!   - Full Aruba-style `show` / `clear` / `mac-address-table` command set
//!   - MAC-move tracking for flap detection

use crate::net;
use crate::interrupt;
use crate::shell::{ShellInstance, OsState};
use crate::net::eth::{self, EthHeader};
use crate::net::types::{MacAddr, MAX_FRAME_SIZE};

const DEFAULT_AGE_TIME_SECS: u32 = 300;
const MIN_AGE_TIME_SECS: u32 = 15;
const MAX_AGE_TIME_SECS: u32 = 1_000_000;
const MAX_MAC_ENTRIES: usize = 512;
const DEFAULT_PVID: u16 = 1;
const SWITCH_INPUT_BUF_LEN: usize = 256;
/// Maximum number of frames the switch IRQ handler will drain per invocation.
/// Paired with Java-side IRQ_NETWORK coalescing: if more frames are in flight
/// than this limit, the next arriving frame re-fires a fresh (coalesced)
/// IRQ_NETWORK and the handler picks up where it left off. This caps the
/// worst-case duration of a single `on_interrupt` call so the Java worker
/// thread can always return to input polling in a predictable amount of time.
const MAX_FRAMES_PER_IRQ: usize = 256;

/// The singleton switch state, present only while `OsState::Switch` is active.
static mut SWITCH_STATE: Option<SwitchState> = None;

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
}

impl SwitchState {
    fn new(own_macs: Vec<MacAddr>, iface_count: usize, now_ms: i64) -> Self {
        Self {
            entries: Vec::new(),
            age_time_secs: DEFAULT_AGE_TIME_SECS,
            own_macs,
            iface_count,
            last_age_ms: now_ms,
            input_buf: [0u8; SWITCH_INPUT_BUF_LEN],
            input_len: 0,
        }
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

        let vlan = hdr.vlan_tag.map(|t| t.vid).unwrap_or(DEFAULT_PVID);
        let now_ms = stack.now_ms;

        // Learn source MAC (skip multicast / broadcast sources and our own MACs).
        let src_is_multicast = (hdr.src.0[0] & 0x01) != 0;
        if !src_is_multicast && !self.is_own_mac(&hdr.src) {
            self.learn(hdr.src, vlan, ingress, now_ms);
        }

        // Local delivery: any frame addressed to one of our own MACs, or a
        // broadcast, is injected back into the local network stack so that
        // ARP, ICMP, TCP, UDP, SSH, etc. all continue to work while the
        // switch is forwarding.
        let is_broadcast = hdr.dst == MacAddr::BROADCAST;
        let is_for_us = is_broadcast || self.is_own_mac(&hdr.dst);
        if is_for_us {
            stack.process_frame_on(ingress, frame);
        }

        // Forwarding decision.
        if is_broadcast {
            self.flood(stack, ingress, frame);
            return;
        }

        // If the destination matches one of our own MACs, this is purely
        // management traffic and has already been delivered locally above.
        if self.is_own_mac(&hdr.dst) {
            return;
        }

        let lookup = self.find(&hdr.dst, vlan).map(|idx| self.entries[idx].port);
        if let Some(egress) = lookup {
            if egress == ingress {
                // Hairpin — dst is learned on the same port it came from.
                // Drop to avoid looping the frame back onto its source segment.
                return;
            }
            if egress >= self.iface_count {
                return;
            }
            if !stack.interfaces[egress].link_up {
                return;
            }
            eth::send_frame_on(egress, frame);
        } else {
            // Unknown unicast — flood.
            self.flood(stack, ingress, frame);
        }
    }

    /// Flood a frame out every link-up port except the ingress port.
    fn flood(&self, stack: &net::NetStack, ingress: usize, frame: &[u8]) {
        for i in 0..self.iface_count {
            if i == ingress {
                continue;
            }
            if !stack.interfaces[i].link_up {
                continue;
            }
            eth::send_frame_on(i, frame);
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

/// Enter switch configuration mode. Swaps IRQ_NETWORK handler, enables
/// promiscuous mode on every interface, and switches the shell into
/// `OsState::Switch`.
pub fn enter(shell: &mut ShellInstance) {
    let stack = match net::NetStack::get() {
        Some(s) => s,
        None => {
            shell.println("switch: network stack not initialized");
            return;
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
        SWITCH_STATE = Some(SwitchState::new(own, iface_count, now_ms));
    }

    interrupt::register(interrupt::IRQ_NETWORK, switch_irq_handler);

    shell.state = OsState::Switch;
    shell.println("Entering switch configuration mode.");
    shell.println("Type 'help' for the command list, 'exit' to return to the shell.");
    print_prompt(shell);
}

/// Leave switch mode via the `exit` or `end` command. Cleans up state,
/// disables promiscuous mode, restores the default IRQ handler, wipes any
/// stale framebuffer content that accumulated during the switch session,
/// and prints a fresh main-shell prompt.
///
/// Idempotent: if called while already torn down, returns immediately. This
/// protects against any code path that might end up invoking `exit_switch`
/// twice (e.g., a stray `\n` replay) so the user never sees duplicated
/// "Exited switch mode." / prompt pairs.
pub fn exit_switch(shell: &mut ShellInstance) {
    if unsafe { SWITCH_STATE.is_none() } {
        return;
    }
    tear_down();
    shell.state = OsState::Shell;
    shell.println("Exited switch mode.");
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

fn print_prompt(shell: &mut ShellInstance) {
    shell.print("switch(config)# ");
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

    match tokens[0] {
        "exit" | "end" | "quit" => {
            exit_switch(shell);
        }
        "help" | "?" => cmd_help(shell),
        "mac-address-table" => cmd_mac_address_table(shell, &tokens[1..]),
        "no" => cmd_no(shell, &tokens[1..]),
        "static-mac" => cmd_static_mac(shell, &tokens[1..], false),
        "show" => cmd_show(shell, &tokens[1..]),
        "clear" => cmd_clear(shell, &tokens[1..]),
        _ => {
            let msg = format!("% Invalid input: {}", line);
            shell.println(&msg);
        }
    }
}

fn cmd_help(shell: &mut ShellInstance) {
    shell.println("Switch configuration commands:");
    shell.println("  mac-address-table age-time <15-1000000>");
    shell.println("  no mac-address-table age-time");
    shell.println("  static-mac <mac> vlan <vlan-id> port <port-num>");
    shell.println("  no static-mac <mac> vlan <vlan-id> port <port-num>");
    shell.println("  show mac-address-table");
    shell.println("  show mac-address-table dynamic [port <n> | vlan <id>]");
    shell.println("  show mac-address-table static");
    shell.println("  show mac-address-table vlan <vlan-id>");
    shell.println("  show mac-address-table port <port-num>");
    shell.println("  show mac-address-table address <mac>");
    shell.println("  show mac-address-table count [dynamic | port <n> | vlan <id>]");
    shell.println("  show mac-address-table mac-move [address <mac> vlan <id> | vlan <id>]");
    shell.println("  clear mac-address-table dynamic");
    shell.println("  clear mac-address-table dynamic vlan <vlan-id>");
    shell.println("  clear mac-address-table dynamic port <port-num>");
    shell.println("  clear mac-address-table dynamic address <mac>");
    shell.println("  exit    Return to the main shell");
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
    if args.is_empty() || args[0] != "mac-address-table" {
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

