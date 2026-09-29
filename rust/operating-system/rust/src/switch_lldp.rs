//! Phase 4 — Link Layer Discovery Protocol (LLDP, IEEE 802.1AB).
//!
//! Sends LLDPDUs at `msg-tx-interval` (default 30s) per link-up port with
//! transmit enabled. Maintains a neighbor database keyed by (port, chassis,
//! port_id) with TTL-based aging.
//!
//! Commands (top):
//!   [no] lldp                              enable/disable globally
//!   lldp timer <5-32768>                   msg-tx-interval seconds
//!   lldp holdtime <2-10>                   TTL multiplier
//!   lldp reinit <1-10>                     reinit delay seconds
//!   lldp txdelay <1-8192>                  msgTxDelay seconds
//!   lldp management-ipv4-address <ip>      override mgmt IP TLV
//!   lldp select-tlv <tlv>                  include optional TLV
//!   no lldp select-tlv <tlv>               exclude optional TLV
//!
//! Commands (interface):
//!   [no] lldp transmit
//!   [no] lldp receive
//!
//! Show:
//!   show lldp configuration
//!   show lldp neighbor-info [<port>]
//!   show lldp statistics
//!   show lldp tlv
//!   show lldp local-device
//!
//! Clear:
//!   clear lldp neighbors
//!   clear lldp statistics

use crate::shell::ShellInstance;
use crate::switch::{self, SwitchState};
use crate::switch_log::{self, Severity};
use crate::net;
use crate::net::eth::{self, ETHERTYPE_8021Q};
use crate::net::types::{MacAddr, MAX_FRAME_SIZE};

pub const ETHERTYPE_LLDP: u16 = 0x88cc;
pub const LLDP_MULTICAST: [u8; 6] = [0x01, 0x80, 0xc2, 0x00, 0x00, 0x0e];

pub const DEFAULT_TIMER_SECS: u32 = 30;
pub const DEFAULT_HOLDTIME: u32 = 4;
pub const DEFAULT_REINIT_SECS: u32 = 2;
pub const DEFAULT_TX_DELAY_SECS: u32 = 2;

/// TLV type codes (IEEE 802.1AB §8.5).
const TLV_END: u8 = 0;
const TLV_CHASSIS_ID: u8 = 1;
const TLV_PORT_ID: u8 = 2;
const TLV_TTL: u8 = 3;
const TLV_PORT_DESC: u8 = 4;
const TLV_SYS_NAME: u8 = 5;
const TLV_SYS_DESC: u8 = 6;
const TLV_SYS_CAPS: u8 = 7;
const TLV_MGMT_ADDR: u8 = 8;

/// Which optional TLVs to include. Mandatory (chassis/port/TTL/end) are always on.
#[derive(Clone)]
pub struct TlvSelection {
    pub port_desc: bool,
    pub sys_name: bool,
    pub sys_desc: bool,
    pub sys_caps: bool,
    pub mgmt_addr: bool,
}

impl TlvSelection {
    pub fn default() -> Self {
        Self { port_desc: true, sys_name: true, sys_desc: true, sys_caps: true, mgmt_addr: true }
    }
}

#[derive(Clone)]
pub struct LldpPort {
    pub transmit: bool,
    pub receive: bool,
    pub last_tx_ms: i64,
    pub tx_count: u64,
    pub rx_count: u64,
    pub rx_errors: u64,
    pub rx_discards: u64,
}

impl LldpPort {
    pub fn new() -> Self {
        Self {
            transmit: true,
            receive: true,
            last_tx_ms: 0,
            tx_count: 0,
            rx_count: 0,
            rx_errors: 0,
            rx_discards: 0,
        }
    }
}

#[derive(Clone)]
pub struct Neighbor {
    pub port: usize,
    pub chassis_id: Vec<u8>,
    pub port_id: Vec<u8>,
    pub ttl_secs: u16,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    pub port_desc: Option<String>,
    pub sys_name: Option<String>,
    pub sys_desc: Option<String>,
    pub mgmt_ipv4: Option<[u8; 4]>,
}

pub struct LldpState {
    pub enabled: bool,
    pub timer_secs: u32,
    pub holdtime: u32,
    pub reinit_secs: u32,
    pub tx_delay_secs: u32,
    pub mgmt_ipv4: Option<[u8; 4]>,
    pub tlvs: TlvSelection,
    pub ports: Vec<LldpPort>,
    pub neighbors: Vec<Neighbor>,
    pub bridge_mac: MacAddr,
    pub sys_name: String,
    pub sys_desc: String,
}

impl LldpState {
    pub fn new(iface_count: usize, bridge_mac: MacAddr) -> Self {
        Self {
            enabled: true,
            timer_secs: DEFAULT_TIMER_SECS,
            holdtime: DEFAULT_HOLDTIME,
            reinit_secs: DEFAULT_REINIT_SECS,
            tx_delay_secs: DEFAULT_TX_DELAY_SECS,
            mgmt_ipv4: None,
            tlvs: TlvSelection::default(),
            ports: (0..iface_count).map(|_| LldpPort::new()).collect(),
            neighbors: Vec::new(),
            bridge_mac,
            sys_name: "ecm-switch".to_string(),
            sys_desc: "Evan's Computer Mod virtual switch".to_string(),
        }
    }
}

// =====================================================================
// Periodic tick: transmit + age neighbors
// =====================================================================

pub fn tick(state: &mut SwitchState, stack: &mut net::NetStack, now_ms: i64) {
    if !state.lldp.enabled {
        return;
    }

    // Build each outgoing frame by reading state first, then writing.
    let iface_count = state.iface_count.min(stack.iface_count);
    let timer_ms = state.lldp.timer_secs as i64 * 1_000;
    let ttl = (state.lldp.timer_secs as u32).saturating_mul(state.lldp.holdtime).min(65535) as u16;

    for i in 0..iface_count {
        if !stack.interfaces[i].link_up {
            continue;
        }
        let port = match state.lldp.ports.get(i) {
            Some(p) => p,
            None => continue,
        };
        if !port.transmit {
            continue;
        }
        if port.last_tx_ms != 0 && (now_ms - port.last_tx_ms) < timer_ms {
            continue;
        }
        let src = stack.interfaces[i].mac;
        let pdu = build_lldpdu(state, i, src, ttl);
        let mut tx = [0u8; MAX_FRAME_SIZE];
        if 14 + pdu.len() > tx.len() {
            continue;
        }
        tx[..6].copy_from_slice(&LLDP_MULTICAST);
        tx[6..12].copy_from_slice(&src.0);
        tx[12..14].copy_from_slice(&ETHERTYPE_LLDP.to_be_bytes());
        tx[14..14 + pdu.len()].copy_from_slice(&pdu);
        eth::send_frame_on(i, &tx[..14 + pdu.len()]);
        if let Some(p) = state.lldp.ports.get_mut(i) {
            p.last_tx_ms = now_ms;
            p.tx_count += 1;
        }
    }

    // Age neighbors.
    let before = state.lldp.neighbors.len();
    let mut removed = Vec::new();
    state.lldp.neighbors.retain(|n| {
        let expiry = n.last_seen_ms + (n.ttl_secs as i64) * 1_000;
        if now_ms >= expiry {
            removed.push((n.port, n.chassis_id.clone(), n.port_id.clone()));
            false
        } else {
            true
        }
    });
    if state.lldp.neighbors.len() != before {
        for (port, cid, pid) in removed {
            let m = format!(
                "LLDP neighbor expired on eth{} (chassis={}, port={})",
                port,
                hex_or_ascii(&cid),
                hex_or_ascii(&pid),
            );
            switch_log::log(Severity::Notice, m);
        }
    }
}

fn build_lldpdu(state: &SwitchState, port: usize, src: MacAddr, ttl: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);

    // Chassis ID (subtype 4: MAC).
    let mut chassis = Vec::new();
    chassis.push(4);
    chassis.extend_from_slice(&state.lldp.bridge_mac.0);
    push_tlv(&mut out, TLV_CHASSIS_ID, &chassis);

    // Port ID (subtype 3: MAC address = interface MAC; subtype 5: interface name).
    let port_name = format!("eth{}", port);
    let mut pid = Vec::new();
    pid.push(5); // interface name
    pid.extend_from_slice(port_name.as_bytes());
    push_tlv(&mut out, TLV_PORT_ID, &pid);

    // TTL
    push_tlv(&mut out, TLV_TTL, &ttl.to_be_bytes());

    if state.lldp.tlvs.port_desc {
        let desc = format!("Port {}", port);
        push_tlv(&mut out, TLV_PORT_DESC, desc.as_bytes());
    }
    if state.lldp.tlvs.sys_name {
        push_tlv(&mut out, TLV_SYS_NAME, state.lldp.sys_name.as_bytes());
    }
    if state.lldp.tlvs.sys_desc {
        push_tlv(&mut out, TLV_SYS_DESC, state.lldp.sys_desc.as_bytes());
    }
    if state.lldp.tlvs.sys_caps {
        // cap = bridge (0x04), enabled = bridge
        let mut caps = Vec::new();
        caps.extend_from_slice(&0x04u16.to_be_bytes());
        caps.extend_from_slice(&0x04u16.to_be_bytes());
        push_tlv(&mut out, TLV_SYS_CAPS, &caps);
    }
    if state.lldp.tlvs.mgmt_addr {
        if let Some(ip) = state.lldp.mgmt_ipv4.or_else(|| {
            crate::net::NetStack::get().and_then(|s| {
                (0..s.iface_count).find_map(|i| {
                    let ip = s.interfaces[i].ip;
                    if ip != crate::net::types::Ipv4Addr::ZERO { Some(ip.0) } else { None }
                })
            })
        }) {
            // addr-string-len(1) subtype(1)=IPv4 addr(4) iface-subtype(1) iface-num(4) OID-len(1)
            let mut mgmt = Vec::new();
            mgmt.push(1 + 4); // address-string-length
            mgmt.push(1);     // subtype IPv4
            mgmt.extend_from_slice(&ip);
            mgmt.push(2);     // interface numbering: ifIndex
            mgmt.extend_from_slice(&(port as u32).to_be_bytes());
            mgmt.push(0);     // no OID
            push_tlv(&mut out, TLV_MGMT_ADDR, &mgmt);
        }
    }
    // Pad a hint of system name via chassis; not strictly needed.
    let _ = src; // reserved for future MAC-based port-id

    // END TLV (type=0, len=0).
    push_tlv(&mut out, TLV_END, &[]);
    out
}

fn push_tlv(out: &mut Vec<u8>, ty: u8, value: &[u8]) {
    let len = value.len().min(511);
    let hdr = ((ty as u16) << 9) | (len as u16);
    out.push((hdr >> 8) as u8);
    out.push(hdr as u8);
    out.extend_from_slice(&value[..len]);
}

// =====================================================================
// Ingress handler
// =====================================================================

pub fn on_frame(state: &mut SwitchState, _stack: &mut net::NetStack, port: usize, frame: &[u8]) {
    if port < state.lldp.ports.len() {
        let rx_enabled = state.lldp.ports[port].receive && state.lldp.enabled;
        if !rx_enabled {
            state.lldp.ports[port].rx_discards += 1;
            return;
        }
        state.lldp.ports[port].rx_count += 1;
    }
    // Skip L2 header / optional tag.
    let mut off = 14usize;
    if frame.len() >= 18 && u16::from_be_bytes([frame[12], frame[13]]) == ETHERTYPE_8021Q {
        off = 18;
    }
    let payload = &frame[off..];

    let mut chassis_id: Vec<u8> = Vec::new();
    let mut port_id: Vec<u8> = Vec::new();
    let mut ttl: u16 = 0;
    let mut port_desc: Option<String> = None;
    let mut sys_name: Option<String> = None;
    let mut sys_desc: Option<String> = None;
    let mut mgmt_ipv4: Option<[u8; 4]> = None;

    let mut i = 0;
    while i + 2 <= payload.len() {
        let hdr = u16::from_be_bytes([payload[i], payload[i + 1]]);
        let ty = (hdr >> 9) as u8 & 0x7f;
        let len = (hdr & 0x1ff) as usize;
        i += 2;
        if i + len > payload.len() { break; }
        let value = &payload[i..i + len];
        match ty {
            TLV_END => break,
            TLV_CHASSIS_ID => chassis_id = value.to_vec(),
            TLV_PORT_ID => port_id = value.to_vec(),
            TLV_TTL => {
                if value.len() == 2 { ttl = u16::from_be_bytes([value[0], value[1]]); }
            }
            TLV_PORT_DESC => port_desc = Some(sanitize(value)),
            TLV_SYS_NAME => sys_name = Some(sanitize(value)),
            TLV_SYS_DESC => sys_desc = Some(sanitize(value)),
            TLV_MGMT_ADDR => {
                if value.len() >= 7 && value[1] == 1 {
                    let mut ip = [0u8; 4];
                    ip.copy_from_slice(&value[2..6]);
                    mgmt_ipv4 = Some(ip);
                }
            }
            _ => {}
        }
        i += len;
    }

    if chassis_id.is_empty() || port_id.is_empty() || ttl == 0 {
        if port < state.lldp.ports.len() {
            state.lldp.ports[port].rx_errors += 1;
        }
        return;
    }

    let now_ms = crate::net::NetStack::get().map(|s| s.now_ms).unwrap_or(0);
    let existing = state.lldp.neighbors.iter().position(|n| {
        n.port == port && n.chassis_id == chassis_id && n.port_id == port_id
    });
    match existing {
        Some(idx) => {
            let n = &mut state.lldp.neighbors[idx];
            n.ttl_secs = ttl;
            n.last_seen_ms = now_ms;
            n.port_desc = port_desc;
            n.sys_name = sys_name;
            n.sys_desc = sys_desc;
            n.mgmt_ipv4 = mgmt_ipv4;
        }
        None => {
            let m = format!(
                "LLDP neighbor added on eth{} (chassis={}, port={}, sys={:?})",
                port,
                hex_or_ascii(&chassis_id),
                hex_or_ascii(&port_id),
                sys_name.clone().unwrap_or_default(),
            );
            switch_log::log(Severity::Info, m);
            state.lldp.neighbors.push(Neighbor {
                port,
                chassis_id,
                port_id,
                ttl_secs: ttl,
                first_seen_ms: now_ms,
                last_seen_ms: now_ms,
                port_desc,
                sys_name,
                sys_desc,
                mgmt_ipv4,
            });
        }
    }
}

fn sanitize(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| if (32..127).contains(&b) { b as char } else { '.' }).collect()
}

fn hex_or_ascii(bytes: &[u8]) -> String {
    if bytes.iter().all(|&b| (32..127).contains(&b)) {
        String::from_utf8_lossy(bytes).to_string()
    } else {
        bytes.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(":")
    }
}

// =====================================================================
// CLI — top-level
// =====================================================================

pub fn cmd_lldp(shell: &mut ShellInstance, args: &[&str], remove: bool) {
    if args.is_empty() {
        switch::with_state_mut(|s| {
            s.lldp.enabled = !remove;
        });
        shell.println(if remove { "LLDP disabled." } else { "LLDP enabled." });
        return;
    }
    match args[0] {
        "timer" => set_u32(shell, args, remove, 5, 32768, |s, v| s.lldp.timer_secs = v, DEFAULT_TIMER_SECS, "timer"),
        "holdtime" => set_u32(shell, args, remove, 2, 10, |s, v| s.lldp.holdtime = v, DEFAULT_HOLDTIME, "holdtime"),
        "reinit" => set_u32(shell, args, remove, 1, 10, |s, v| s.lldp.reinit_secs = v, DEFAULT_REINIT_SECS, "reinit"),
        "txdelay" => set_u32(shell, args, remove, 1, 8192, |s, v| s.lldp.tx_delay_secs = v, DEFAULT_TX_DELAY_SECS, "txdelay"),
        "management-ipv4-address" => {
            if remove {
                switch::with_state_mut(|s| { s.lldp.mgmt_ipv4 = None; });
                shell.println("LLDP management-ipv4-address cleared.");
                return;
            }
            if args.len() != 2 {
                shell.println("% Usage: lldp management-ipv4-address <ip>");
                return;
            }
            match parse_ipv4(args[1]) {
                Some(ip) => {
                    switch::with_state_mut(|s| { s.lldp.mgmt_ipv4 = Some(ip); });
                    shell.println("LLDP management-ipv4-address set.");
                }
                None => shell.println("% Invalid IPv4 address"),
            }
        }
        "select-tlv" => {
            if args.len() != 2 {
                shell.println("% Usage: [no] lldp select-tlv <port-desc|sys-name|sys-desc|sys-caps|mgmt-addr>");
                return;
            }
            let v = !remove;
            switch::with_state_mut(|s| {
                match args[1] {
                    "port-desc" | "port-description" => s.lldp.tlvs.port_desc = v,
                    "sys-name" | "system-name" => s.lldp.tlvs.sys_name = v,
                    "sys-desc" | "system-description" => s.lldp.tlvs.sys_desc = v,
                    "sys-caps" | "system-capabilities" => s.lldp.tlvs.sys_caps = v,
                    "mgmt-addr" | "management-address" => s.lldp.tlvs.mgmt_addr = v,
                    _ => { }
                }
            });
            let msg = format!("LLDP TLV {} {}.", args[1], if v { "enabled" } else { "disabled" });
            shell.println(&msg);
        }
        _ => shell.println("% Unknown lldp subcommand"),
    }
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
        switch::with_state_mut(|s| apply(s, default));
        shell.println(&format!("LLDP {} reset to default.", name));
        return;
    }
    if args.len() != 2 {
        shell.println(&format!("% Usage: lldp {} <{}-{}>", name, min, max));
        return;
    }
    match args[1].parse::<u32>() {
        Ok(v) if v >= min && v <= max => {
            switch::with_state_mut(|s| apply(s, v));
            shell.println(&format!("LLDP {} set to {}.", name, v));
        }
        _ => shell.println(&format!("% Value out of range ({}-{})", min, max)),
    }
}

pub fn cmd_if_lldp(shell: &mut ShellInstance, idx: usize, args: &[&str], remove: bool) {
    if args.is_empty() {
        shell.println("% Usage: [no] lldp [transmit|receive]");
        return;
    }
    switch::with_state_mut(|s| {
        if let Some(p) = s.lldp.ports.get_mut(idx) {
            match args[0] {
                "transmit" => p.transmit = !remove,
                "receive" => p.receive = !remove,
                _ => { }
            }
        }
    });
    shell.println(&format!("LLDP {} {} on eth{}.", args[0], if remove { "disabled" } else { "enabled" }, idx));
}

// =====================================================================
// show / clear
// =====================================================================

pub fn show(shell: &mut ShellInstance, args: &[&str]) {
    if args.is_empty() {
        shell.println("% Usage: show lldp [configuration|neighbor-info|statistics|tlv|local-device]");
        return;
    }
    match args[0] {
        "configuration" => show_config(shell),
        "neighbor-info" => show_neighbors(shell, &args[1..]),
        "statistics" => show_stats(shell),
        "tlv" => show_tlv(shell),
        "local-device" => show_local(shell),
        _ => shell.println("% Unknown show lldp option"),
    }
}

fn show_config(shell: &mut ShellInstance) {
    switch::with_state(|s| {
        shell.println(&format!("LLDP Global:"));
        shell.println(&format!("  Enabled   : {}", s.lldp.enabled));
        shell.println(&format!("  Timer     : {} s", s.lldp.timer_secs));
        shell.println(&format!("  Holdtime  : {} (TTL = {} s)", s.lldp.holdtime, s.lldp.timer_secs * s.lldp.holdtime));
        shell.println(&format!("  Reinit    : {} s", s.lldp.reinit_secs));
        shell.println(&format!("  Tx-delay  : {} s", s.lldp.tx_delay_secs));
        if let Some(ip) = s.lldp.mgmt_ipv4 {
            shell.println(&format!("  Mgmt IPv4 : {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]));
        }
        shell.println("");
        shell.println("Per-Port:");
        shell.println("  Port    Tx    Rx");
        for (i, p) in s.lldp.ports.iter().enumerate() {
            shell.println(&format!("  eth{:<3}  {}  {}",
                i,
                if p.transmit { "on " } else { "off" },
                if p.receive { "on " } else { "off" }));
        }
    });
}

fn show_neighbors(shell: &mut ShellInstance, args: &[&str]) {
    let filter: Option<usize> = if args.is_empty() { None } else {
        match crate::switch::parse_port_id_pub(args[0]) {
            Some(p) => Some(p),
            None => { shell.println("% Invalid port"); return; }
        }
    };
    switch::with_state(|s| {
        if s.lldp.neighbors.is_empty() {
            shell.println("(no LLDP neighbors)");
            return;
        }
        shell.println("Port    Chassis             Remote-Port        Sys-Name          TTL");
        shell.println("-----------------------------------------------------------------------");
        for n in &s.lldp.neighbors {
            if let Some(f) = filter {
                if n.port != f { continue; }
            }
            let sys = n.sys_name.clone().unwrap_or_default();
            shell.println(&format!(
                "eth{:<3}  {:<18}  {:<16}  {:<16}  {}",
                n.port,
                hex_or_ascii(&n.chassis_id),
                hex_or_ascii(&n.port_id),
                truncate(&sys, 16),
                n.ttl_secs,
            ));
        }
    });
}

fn show_stats(shell: &mut ShellInstance) {
    switch::with_state(|s| {
        shell.println("Port    Tx      Rx      Rx-Err  Discard");
        for (i, p) in s.lldp.ports.iter().enumerate() {
            shell.println(&format!("eth{:<3}  {:<6}  {:<6}  {:<6}  {}",
                i, p.tx_count, p.rx_count, p.rx_errors, p.rx_discards));
        }
    });
}

fn show_tlv(shell: &mut ShellInstance) {
    switch::with_state(|s| {
        shell.println("Optional TLVs:");
        shell.println(&format!("  port-desc : {}", s.lldp.tlvs.port_desc));
        shell.println(&format!("  sys-name  : {}", s.lldp.tlvs.sys_name));
        shell.println(&format!("  sys-desc  : {}", s.lldp.tlvs.sys_desc));
        shell.println(&format!("  sys-caps  : {}", s.lldp.tlvs.sys_caps));
        shell.println(&format!("  mgmt-addr : {}", s.lldp.tlvs.mgmt_addr));
    });
}

fn show_local(shell: &mut ShellInstance) {
    switch::with_state(|s| {
        shell.println(&format!("Chassis ID (MAC) : {}", format_mac(&s.lldp.bridge_mac)));
        shell.println(&format!("System Name      : {}", s.lldp.sys_name));
        shell.println(&format!("System Desc      : {}", s.lldp.sys_desc));
        if let Some(ip) = s.lldp.mgmt_ipv4 {
            shell.println(&format!("Mgmt IPv4        : {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]));
        }
    });
}

pub fn clear(shell: &mut ShellInstance, args: &[&str]) {
    match args[0] {
        "neighbors" => {
            switch::with_state_mut(|s| s.lldp.neighbors.clear());
            shell.println("LLDP neighbors cleared.");
        }
        "statistics" => {
            switch::with_state_mut(|s| {
                for p in s.lldp.ports.iter_mut() {
                    p.tx_count = 0; p.rx_count = 0; p.rx_errors = 0; p.rx_discards = 0;
                }
            });
            shell.println("LLDP statistics cleared.");
        }
        _ => shell.println("% Usage: clear lldp [neighbors|statistics]"),
    }
}

// =====================================================================
// Persistence
// =====================================================================

pub fn serialize(state: &SwitchState, out: &mut String) {
    if !state.lldp.enabled {
        out.push_str("no lldp\n");
    }
    if state.lldp.timer_secs != DEFAULT_TIMER_SECS {
        out.push_str(&format!("lldp timer {}\n", state.lldp.timer_secs));
    }
    if state.lldp.holdtime != DEFAULT_HOLDTIME {
        out.push_str(&format!("lldp holdtime {}\n", state.lldp.holdtime));
    }
    if state.lldp.reinit_secs != DEFAULT_REINIT_SECS {
        out.push_str(&format!("lldp reinit {}\n", state.lldp.reinit_secs));
    }
    if state.lldp.tx_delay_secs != DEFAULT_TX_DELAY_SECS {
        out.push_str(&format!("lldp txdelay {}\n", state.lldp.tx_delay_secs));
    }
    if let Some(ip) = state.lldp.mgmt_ipv4 {
        out.push_str(&format!("lldp management-ipv4-address {}.{}.{}.{}\n", ip[0], ip[1], ip[2], ip[3]));
    }
    // Default TLV selection is all-on; only emit `no` lines for disabled ones.
    if !state.lldp.tlvs.port_desc { out.push_str("no lldp select-tlv port-desc\n"); }
    if !state.lldp.tlvs.sys_name  { out.push_str("no lldp select-tlv sys-name\n"); }
    if !state.lldp.tlvs.sys_desc  { out.push_str("no lldp select-tlv sys-desc\n"); }
    if !state.lldp.tlvs.sys_caps  { out.push_str("no lldp select-tlv sys-caps\n"); }
    if !state.lldp.tlvs.mgmt_addr { out.push_str("no lldp select-tlv mgmt-addr\n"); }
    for (i, p) in state.lldp.ports.iter().enumerate() {
        if !p.transmit || !p.receive {
            out.push_str(&format!("interface eth{}\n", i));
            if !p.transmit { out.push_str(" no lldp transmit\n"); }
            if !p.receive  { out.push_str(" no lldp receive\n"); }
            out.push_str("exit\n");
        }
    }
}

// =====================================================================
// helpers
// =====================================================================

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 { return None; }
    let mut out = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p.parse().ok()?;
    }
    Some(out)
}

fn format_mac(m: &MacAddr) -> String {
    format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        m.0[0], m.0[1], m.0[2], m.0[3], m.0[4], m.0[5])
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n { s.to_string() } else { s.chars().take(n).collect() }
}
