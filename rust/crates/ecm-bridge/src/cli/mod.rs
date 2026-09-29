//! AOS-CX style configuration CLI over the typed [`Bridge`] API.
//!
//! Contexts and prompts:
//!
//! | context              | prompt                      |
//! |----------------------|-----------------------------|
//! | top                  | `switch(config)# `          |
//! | `vlan N`             | `switch(config-vlan-N)# `   |
//! | `interface ethN`     | `switch(config-if-ethN)# `  |
//! | `interface lag N`    | `switch(config-lag-N)# `    |
//! | `interface vlan N`   | `switch(config-if-vlan-N)# `|
//!
//! `exit` pops a sub-context to top; at top it emits [`CliEffect::ExitCli`].
//! `end`/`quit` return to top from anywhere. `on` emits
//! [`CliEffect::Detach`] (keep the switch running after the CLI exits).
//! Blank lines and lines starting with `#` are ignored, so a saved
//! `switch.cfg` can be replayed line by line through one session.

mod config;
mod show;

use crate::bridge::{fmt_ip, Bridge};
use crate::frame::HashMode;
use crate::log::Severity;
use crate::types::*;
use ecm_net::types::Ipv4Addr;
use std::collections::BTreeSet;
use std::fmt;

pub use config::running_config;

/// Maximum accepted command line length (longer lines are rejected).
pub const MAX_LINE_LEN: usize = 1024;

/// The CLI's current configuration context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    Top,
    Vlan(u16),
    Eth(usize),
    Lag(u16),
    Svi(u16),
}

/// Per-session CLI state.
#[derive(Clone, Debug)]
pub struct CliSession {
    ctx: Context,
}

impl Default for CliSession {
    fn default() -> Self {
        Self::new()
    }
}

impl CliSession {
    /// A session at the top-level `switch(config)#` context.
    pub fn new() -> Self {
        CliSession { ctx: Context::Top }
    }
    pub fn context(&self) -> Context {
        self.ctx
    }
}

/// Side effects the caller must perform.
#[derive(Clone, PartialEq, Eq)]
pub enum CliEffect {
    /// Create/readdress the SVI interface for `vlan` in the host stack.
    SviAddress { vlan: u16, ip: Ipv4Addr, prefix: u8 },
    /// Remove the SVI interface for `vlan`.
    SviRemove { vlan: u16 },
    /// `write memory`: write this text to `/switch.cfg`.
    SaveConfig(String),
    /// `on`: keep the switch running after the CLI exits.
    Detach,
    /// `exit` at the top-level context: leave the CLI.
    ExitCli,
}

impl fmt::Debug for CliEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliEffect::SviAddress { vlan, ip, prefix } => write!(f, "SviAddress {{ vlan: {}, ip: {}, prefix: {} }}", vlan, fmt_ip(ip), prefix),
            CliEffect::SviRemove { vlan } => write!(f, "SviRemove {{ vlan: {} }}", vlan),
            CliEffect::SaveConfig(s) => write!(f, "SaveConfig({} bytes)", s.len()),
            CliEffect::Detach => write!(f, "Detach"),
            CliEffect::ExitCli => write!(f, "ExitCli"),
        }
    }
}

/// Result of one command: text to print (newline-terminated lines, may be
/// empty) and effects to apply.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CliResult {
    pub output: String,
    pub effects: Vec<CliEffect>,
}

pub(crate) struct Out {
    pub text: String,
    pub effects: Vec<CliEffect>,
}

impl Out {
    pub fn line(&mut self, s: impl AsRef<str>) {
        self.text.push_str(s.as_ref());
        self.text.push('\n');
    }
    pub fn err(&mut self, s: impl AsRef<str>) {
        self.text.push_str("% ");
        self.line(s);
    }
    fn result(&mut self, r: Result<(), BridgeError>, ok: impl AsRef<str>) {
        match r {
            Ok(()) => {
                if !ok.as_ref().is_empty() {
                    self.line(ok)
                }
            }
            Err(e) => self.err(e.to_string()),
        }
    }
}

/// The prompt for the session's current context.
pub fn prompt(s: &CliSession) -> String {
    match s.ctx {
        Context::Top => "switch(config)# ".to_string(),
        Context::Vlan(v) => format!("switch(config-vlan-{})# ", v),
        Context::Eth(p) => format!("switch(config-if-eth{})# ", p),
        Context::Lag(id) => format!("switch(config-lag-{})# ", id),
        Context::Svi(v) => format!("switch(config-if-vlan-{})# ", v),
    }
}

/// Execute one command line.
pub fn exec(bridge: &mut Bridge, s: &mut CliSession, line: &str, now_ms: i64) -> CliResult {
    let mut o = Out { text: String::new(), effects: Vec::new() };
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
        return CliResult::default();
    }
    if line.len() > MAX_LINE_LEN {
        o.err("Command line too long");
        return CliResult { output: o.text, effects: o.effects };
    }
    // Keep the bridge clock current for log timestamps.
    bridge.poll(now_ms);
    // A context may refer to an object deleted meanwhile.
    s.ctx = match s.ctx {
        Context::Vlan(v) if bridge.vlan(v).is_none() => Context::Top,
        Context::Lag(id) if bridge.lag_config(id).is_none() => Context::Top,
        Context::Eth(p) if p >= bridge.port_count() => Context::Top,
        c => c,
    };
    let t: Vec<&str> = line.split_whitespace().collect();
    match t.first().copied().unwrap_or("") {
        "end" | "quit" => s.ctx = Context::Top,
        "exit" => {
            if s.ctx == Context::Top {
                o.effects.push(CliEffect::ExitCli);
            } else {
                s.ctx = Context::Top;
            }
        }
        "help" | "?" => help(&mut o, s.ctx),
        "show" => show::show(bridge, &mut o, t.get(1..).unwrap_or(&[])),
        "write" => cmd_write(bridge, &mut o, t.get(1..).unwrap_or(&[])),
        _ => match s.ctx {
            Context::Top => exec_top(bridge, s, &mut o, &t, line),
            Context::Vlan(v) => exec_vlan(bridge, &mut o, &t, line, v),
            Context::Eth(p) => exec_iface(bridge, &mut o, &t, line, PortRef::Eth(p)),
            Context::Lag(id) => exec_iface(bridge, &mut o, &t, line, PortRef::Lag(id)),
            Context::Svi(v) => exec_svi(bridge, &mut o, &t, line, v),
        },
    }
    bridge.poll(now_ms);
    CliResult { output: o.text, effects: o.effects }
}

/// Replay a saved configuration through one session. Returns the number of
/// commands applied and the combined result: output contains only error
/// lines (`% ...`) plus a summary; effects (e.g. `SviAddress`) must be
/// applied by the caller. `ExitCli` is never returned.
pub fn load_config(bridge: &mut Bridge, text: &str, now_ms: i64) -> (usize, CliResult) {
    let mut s = CliSession::new();
    let mut out = CliResult::default();
    let mut applied = 0;
    for raw in text.lines() {
        let l = raw.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with('!') {
            continue;
        }
        let r = exec(bridge, &mut s, l, now_ms);
        applied += 1;
        for line in r.output.lines().filter(|x| x.starts_with('%')) {
            out.output.push_str(&format!("{}  (line: {})\n", line, l));
        }
        out.effects.extend(r.effects.into_iter().filter(|e| *e != CliEffect::ExitCli && *e != CliEffect::Detach));
    }
    out.output.push_str(&format!("Loaded switch.cfg ({} commands applied).\n", applied));
    (applied, out)
}

// =====================================================================
// Parsing helpers
// =====================================================================

pub(crate) fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let sep = if s.contains(':') {
        ':'
    } else if s.contains('-') {
        '-'
    } else if s.contains('.') {
        // Cisco-style aabb.ccdd.eeff
        let hex: String = s.split('.').collect();
        if hex.len() != 12 || s.split('.').count() != 3 {
            return None;
        }
        let mut out = [0u8; 6];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        return Some(out);
    } else {
        return None;
    };
    let parts: Vec<&str> = s.split(sep).collect();
    if parts.len() != 6 {
        return None;
    }
    let mut out = [0u8; 6];
    for (b, p) in out.iter_mut().zip(parts.iter()) {
        if p.len() != 2 {
            return None;
        }
        *b = u8::from_str_radix(p, 16).ok()?;
    }
    Some(out)
}

pub(crate) fn parse_vid(s: &str) -> Result<u16, &'static str> {
    match s.parse::<u16>() {
        Ok(v) if (MIN_VID..=MAX_VID).contains(&v) => Ok(v),
        Ok(_) => Err("VLAN id out of range (1-4094)"),
        Err(_) => Err("Invalid VLAN id"),
    }
}

/// `10,20,30-40`
pub(crate) fn parse_vlan_list(s: &str) -> Result<BTreeSet<u16>, String> {
    let mut out = BTreeSet::new();
    for raw in s.split(',') {
        let tok = raw.trim();
        if tok.is_empty() {
            continue;
        }
        let bad = || format!("Invalid VLAN list: {}", tok);
        if let Some((a, b)) = tok.split_once('-') {
            let a: u16 = a.trim().parse().map_err(|_| bad())?;
            let b: u16 = b.trim().parse().map_err(|_| bad())?;
            if a < MIN_VID || b > MAX_VID || a > b {
                return Err(bad());
            }
            out.extend(a..=b);
        } else {
            let v: u16 = tok.parse().map_err(|_| bad())?;
            if !(MIN_VID..=MAX_VID).contains(&v) {
                return Err(format!("Invalid VLAN id: {}", tok));
            }
            out.insert(v);
        }
    }
    if out.is_empty() {
        return Err("Empty VLAN list".to_string());
    }
    Ok(out)
}

/// Render a VLAN set compactly (`1,10-12,20`).
pub(crate) fn format_vlan_list(s: &BTreeSet<u16>) -> String {
    let mut parts = Vec::new();
    let mut it = s.iter().copied().peekable();
    while let Some(start) = it.next() {
        let mut end = start;
        while it.peek() == Some(&(end.wrapping_add(1))) {
            end += 1;
            it.next();
        }
        parts.push(if start == end { start.to_string() } else { format!("{}-{}", start, end) });
    }
    parts.join(",")
}

/// `N`, `ethN`, `lagN` (static-mac and show filters).
pub(crate) fn parse_port_ref(s: &str, nports: usize) -> Option<PortRef> {
    if let Some(r) = s.strip_prefix("lag") {
        let id: u16 = r.parse().ok()?;
        return (MIN_LAG_ID..=MAX_LAG_ID).contains(&id).then_some(PortRef::Lag(id));
    }
    let n: usize = s.strip_prefix("eth").unwrap_or(s).parse().ok()?;
    (n < nports).then_some(PortRef::Eth(n))
}

pub(crate) fn parse_eth(s: &str, nports: usize) -> Option<usize> {
    let n: usize = s.strip_prefix("eth").unwrap_or(s).parse().ok()?;
    (n < nports).then_some(n)
}

pub(crate) fn parse_ipv4(s: &str) -> Option<Ipv4Addr> {
    let mut o = [0u8; 4];
    let mut n = 0;
    for part in s.split('.') {
        if n >= 4 || part.is_empty() || part.len() > 3 {
            return None;
        }
        *o.get_mut(n)? = part.parse().ok()?;
        n += 1;
    }
    (n == 4).then_some(Ipv4Addr(o))
}

fn parse_u32(s: Option<&&str>) -> Option<u32> {
    s.and_then(|x| x.parse().ok())
}

// =====================================================================
// Top-level context
// =====================================================================

fn exec_top(b: &mut Bridge, s: &mut CliSession, o: &mut Out, t: &[&str], line: &str) {
    let rest = t.get(1..).unwrap_or(&[]);
    match t.first().copied().unwrap_or("") {
        "mac-address-table" => match rest {
            ["age-time", v] => match v.parse::<u32>() {
                Ok(n) => o.result(b.set_age_time_secs(n), format!("MAC age-time set to {} seconds.", n)),
                Err(_) => o.err("Invalid age-time value"),
            },
            _ => o.err("Usage: mac-address-table age-time <15-1000000>"),
        },
        "static-mac" => cmd_static_mac(b, o, rest, false),
        "vlan" => cmd_vlan(b, s, o, rest),
        "interface" => cmd_interface(b, s, o, rest),
        "no" => exec_top_no(b, o, rest, line),
        "clear" => show::clear(b, o, rest),
        "logging" => cmd_logging(b, o, rest, false),
        "lldp" => cmd_lldp(b, o, rest, false),
        "spanning-tree" => cmd_spanning_tree(b, o, rest, false),
        "on" => {
            o.effects.push(CliEffect::Detach);
            o.line("Switching will persist after exit. Use 'switch off' to stop it.");
        }
        "off" => o.err("Use 'exit' and then 'switch off' to stop the switch"),
        _ => o.err(format!("Invalid input: {}", line)),
    }
}

fn exec_top_no(b: &mut Bridge, o: &mut Out, rest: &[&str], line: &str) {
    let args = rest.get(1..).unwrap_or(&[]);
    match rest.first().copied().unwrap_or("") {
        "mac-address-table" if args == ["age-time"] => {
            o.result(b.set_age_time_secs(DEFAULT_AGE_TIME_SECS), format!("MAC age-time set to {} seconds.", DEFAULT_AGE_TIME_SECS))
        }
        "static-mac" => cmd_static_mac(b, o, args, true),
        "vlan" => match args {
            [v] => match parse_vid(v) {
                Ok(vid) => o.result(b.delete_vlan(vid), format!("VLAN {} deleted.", vid)),
                Err(e) => o.err(e),
            },
            _ => o.err("Usage: no vlan <vlan-id>"),
        },
        "interface" => match args {
            ["lag", id] => match id.parse::<u16>() {
                Ok(id) => o.result(b.delete_lag(id), format!("LAG {} deleted.", id)),
                Err(_) => o.err("Invalid LAG id"),
            },
            ["vlan", v] => match parse_vid(v) {
                Ok(vid) => {
                    if b.has_svi(vid) {
                        let _ = b.set_svi(vid, None);
                        o.effects.push(CliEffect::SviRemove { vlan: vid });
                        o.line(format!("Interface vlan {} removed.", vid));
                    } else {
                        o.err(format!("Interface vlan {} does not exist", vid));
                    }
                }
                Err(e) => o.err(e),
            },
            _ => o.err("Usage: no interface lag <id> | no interface vlan <id>"),
        },
        "logging" => cmd_logging(b, o, args, true),
        "lldp" => cmd_lldp(b, o, args, true),
        "spanning-tree" => cmd_spanning_tree(b, o, args, true),
        "" => o.err("Incomplete command."),
        _ => o.err(format!("Invalid input: {}", line)),
    }
}

fn cmd_write(b: &mut Bridge, o: &mut Out, args: &[&str]) {
    if args != ["memory"] && args != ["mem"] {
        o.err("Usage: write memory");
        return;
    }
    o.effects.push(CliEffect::SaveConfig(running_config(b, &[])));
}

fn cmd_static_mac(b: &mut Bridge, o: &mut Out, args: &[&str], remove: bool) {
    let (mac, vlan, port) = match args {
        [m, "vlan", v, "port", p] => (*m, *v, *p),
        _ => return o.err("Usage: static-mac <mac> vlan <vlan-id> port <port-num|lagN>"),
    };
    let mac = match parse_mac(mac) {
        Some(m) => m,
        None => return o.err("Invalid MAC address"),
    };
    let vlan = match parse_vid(vlan) {
        Ok(v) => v,
        Err(e) => return o.err(e),
    };
    let n = b.port_count();
    let port = match parse_port_ref(port, n) {
        Some(p) => p,
        None => return o.err(format!("Port {} out of range (0-{})", port, n.saturating_sub(1))),
    };
    let mac = ecm_net::types::MacAddr(mac);
    if remove {
        match b.remove_static_mac(mac, vlan, port) {
            Ok(()) => o.line("Static MAC entry removed."),
            Err(_) => o.err("No matching static entry"),
        }
    } else {
        match b.add_static_mac(mac, vlan, port) {
            Ok(true) => o.line("Static MAC entry updated."),
            Ok(false) => o.line("Static MAC entry added."),
            Err(e) => o.err(e.to_string()),
        }
    }
}

fn cmd_vlan(b: &mut Bridge, s: &mut CliSession, o: &mut Out, args: &[&str]) {
    let arg = match args {
        [a] => *a,
        _ => return o.err("Usage: vlan <vlan-id> | vlan <start>-<end>"),
    };
    if let Some((a, z)) = arg.split_once('-') {
        match (a.parse::<u16>(), z.parse::<u16>()) {
            (Ok(a), Ok(z)) if a >= MIN_VID && z <= MAX_VID && a <= z => {
                let mut created = 0;
                for v in a..=z {
                    if b.create_vlan(v) == Ok(true) {
                        created += 1;
                    }
                }
                o.line(format!("Created {} VLANs (range {}-{}).", created, a, z));
            }
            _ => o.err("Invalid VLAN range (1-4094, start <= end)"),
        }
        return;
    }
    match parse_vid(arg) {
        Ok(v) => {
            let _ = b.create_vlan(v);
            s.ctx = Context::Vlan(v);
        }
        Err(e) => o.err(e),
    }
}

fn cmd_interface(b: &mut Bridge, s: &mut CliSession, o: &mut Out, args: &[&str]) {
    match args {
        ["lag", id] => match id.parse::<u16>() {
            Ok(id) => match b.create_lag(id) {
                Ok(_) => s.ctx = Context::Lag(id),
                Err(e) => o.err(e.to_string()),
            },
            Err(_) => o.err("Invalid LAG id"),
        },
        ["vlan", v] => match parse_vid(v) {
            Ok(vid) => {
                if b.vlan(vid).is_none() {
                    o.err(format!("VLAN {} does not exist (create it with 'vlan {}')", vid, vid));
                } else {
                    s.ctx = Context::Svi(vid);
                }
            }
            Err(e) => o.err(e),
        },
        [p] => {
            // Also accept "lag1" / "vlan10" as one token.
            if let Some(id) = p.strip_prefix("lag").filter(|r| !r.is_empty()) {
                return cmd_interface(b, s, o, &["lag", id]);
            }
            if let Some(v) = p.strip_prefix("vlan").filter(|r| !r.is_empty()) {
                return cmd_interface(b, s, o, &["vlan", v]);
            }
            let n = b.port_count();
            match parse_eth(p, n) {
                Some(i) => s.ctx = Context::Eth(i),
                None => o.err(format!("Invalid port (must be 0..{} or eth0..eth{})", n.saturating_sub(1), n.saturating_sub(1))),
            }
        }
        _ => o.err("Usage: interface <port-num | ethN> | interface lag <id> | interface vlan <id>"),
    }
}

// =====================================================================
// VLAN context
// =====================================================================

fn exec_vlan(b: &mut Bridge, o: &mut Out, t: &[&str], line: &str, vid: u16) {
    let rest = t.get(1..).unwrap_or(&[]);
    match t.first().copied().unwrap_or("") {
        "name" if !rest.is_empty() => o.result(b.set_vlan_name(vid, Some(rest.join(" "))), "VLAN name set."),
        "description" if !rest.is_empty() => o.result(b.set_vlan_description(vid, Some(rest.join(" "))), "VLAN description set."),
        "shutdown" => o.result(b.set_vlan_active(vid, false), "VLAN deactivated."),
        "no" => match rest {
            ["shutdown"] => o.result(b.set_vlan_active(vid, true), "VLAN activated."),
            ["name", ..] => o.result(b.set_vlan_name(vid, None), "VLAN name cleared."),
            ["description", ..] => o.result(b.set_vlan_description(vid, None), "VLAN description cleared."),
            _ => o.err(format!("Invalid input in vlan context: {}", line)),
        },
        _ => o.err(format!("Invalid input in vlan context: {}", line)),
    }
}

// =====================================================================
// Interface (ethN / lag N) context
// =====================================================================

fn exec_iface(b: &mut Bridge, o: &mut Out, t: &[&str], line: &str, i: PortRef) {
    let rest = t.get(1..).unwrap_or(&[]);
    let is_lag = matches!(i, PortRef::Lag(_));
    match (t.first().copied().unwrap_or(""), rest) {
        ("routing", []) => o.result(b.set_port_mode(i, PortMode::Routed), "Port converted to L3 (routed)."),
        ("vlan", _) => cmd_if_vlan(b, o, i, rest, false),
        ("shutdown", []) => o.result(b.set_admin_up(i, false), format!("{} administratively down.", i)),
        ("spanning-tree", _) => cmd_if_stp(b, o, i, rest, false),
        ("lldp", _) if !is_lag => cmd_if_lldp(b, o, i, rest, false),
        ("lag", [id]) if !is_lag => cmd_if_lag(b, o, i, Some(id)),
        ("lacp", _) if !is_lag => o.line("LACP per-port knob acknowledged (no-op in this build)."),
        ("lacp", _) if is_lag => cmd_lag_lacp(b, o, i, rest, false),
        ("fallback", []) if is_lag => cmd_lag_lacp(b, o, i, &["fallback"], false),
        ("hash", [h]) if is_lag => cmd_lag_hash(b, o, i, Some(h)),
        ("no", _) => {
            let args = rest.get(1..).unwrap_or(&[]);
            match (rest.first().copied().unwrap_or(""), args) {
                ("routing", []) => {
                    let r = if b.port_mode(i).map(|m| m.is_routed()).unwrap_or(false) {
                        b.set_port_mode(i, PortMode::Access { vid: DEFAULT_VID })
                    } else {
                        Ok(())
                    };
                    o.result(r, "Port converted to L2 (access vlan 1).");
                    if let PortRef::Eth(n) = i {
                        if let Some(lag) = b.eth_config(n).and_then(|c| c.lag) {
                            o.line(format!("Note: eth{} is a member of lag{}; the LAG's VLAN config applies.", n, lag));
                        }
                    }
                }
                ("vlan", _) => cmd_if_vlan(b, o, i, args, true),
                ("shutdown", []) => o.result(b.set_admin_up(i, true), format!("{} administratively up.", i)),
                ("spanning-tree", _) => cmd_if_stp(b, o, i, args, true),
                ("lldp", _) if !is_lag => cmd_if_lldp(b, o, i, args, true),
                ("lag", _) if !is_lag => cmd_if_lag(b, o, i, None),
                ("lacp", _) if is_lag => cmd_lag_lacp(b, o, i, args, true),
                ("fallback", []) if is_lag => cmd_lag_lacp(b, o, i, &["fallback"], true),
                ("hash", _) if is_lag => cmd_lag_hash(b, o, i, None),
                ("lacp", _) => o.line("LACP per-port knob acknowledged (no-op in this build)."),
                _ => o.err(format!("Invalid input in interface context: {}", line)),
            }
        }
        _ => o.err(format!("Invalid input in interface context: {}", line)),
    }
}

fn require_l2(b: &Bridge, o: &mut Out, i: PortRef) -> bool {
    if b.port_mode(i).map(|m| m.is_routed()).unwrap_or(true) {
        o.err(BridgeError::NotL2.to_string());
        false
    } else {
        true
    }
}

fn cmd_if_vlan(b: &mut Bridge, o: &mut Out, i: PortRef, args: &[&str], remove: bool) {
    if !require_l2(b, o, i) {
        return;
    }
    let mode = b.port_mode(i).cloned().unwrap_or(PortMode::Routed);
    match (args.first().copied().unwrap_or(""), args.get(1..).unwrap_or(&[])) {
        ("access", rest) if remove && rest.len() <= 1 => {
            o.result(b.set_port_mode(i, PortMode::Access { vid: DEFAULT_VID }), "Access VLAN reset to default (1).")
        }
        ("access", [v]) => {
            let vid = match parse_vid(v) {
                Ok(v) => v,
                Err(e) => return o.err(e),
            };
            match b.vlan(vid) {
                None => {
                    o.err(format!("Warning: VLAN {} does not exist (creating as inactive)", vid));
                    let _ = b.create_vlan(vid);
                }
                Some(v) if !v.active => o.err(format!("Warning: VLAN {} is shutdown", vid)),
                _ => {}
            }
            o.result(b.set_port_mode(i, PortMode::Access { vid }), format!("Access VLAN set to {}.", vid));
        }
        ("trunk", rest) => match (rest.first().copied().unwrap_or(""), rest.get(1..).unwrap_or(&[])) {
            ("native", r) if remove && r.len() <= 1 => match mode {
                PortMode::Trunk { native_tag, allowed, .. } => {
                    o.result(b.set_port_mode(i, PortMode::Trunk { native: DEFAULT_VID, native_tag, allowed }), "Native VLAN reset to 1.")
                }
                _ => o.err("Port is not in trunk mode"),
            },
            ("native", r) if !remove && (r.len() == 1 || r.get(1) == Some(&"tag") && r.len() == 2) => {
                let vid = match r.first().map(|v| parse_vid(v)) {
                    Some(Ok(v)) => v,
                    Some(Err(e)) => return o.err(e),
                    None => return o.err("Usage: vlan trunk native <vlan-id> [tag]"),
                };
                let tag = r.len() == 2;
                let msg = if tag { format!("Trunk native VLAN set to {} (tagged).", vid) } else { format!("Trunk native VLAN set to {}.", vid) };
                o.result(b.set_port_mode(i, PortMode::Trunk { native: vid, native_tag: tag, allowed: AllowedList::All }), msg);
            }
            ("allowed", r) if !r.is_empty() => {
                let (native, native_tag, allowed) = match mode {
                    PortMode::Trunk { native, native_tag, allowed } => (native, native_tag, allowed),
                    _ => return o.err(BridgeError::NotTrunk.to_string()),
                };
                if r == ["all"] {
                    if remove {
                        return o.err("Usage: no vlan trunk allowed <vlan-list>");
                    }
                    return o.result(b.set_port_mode(i, PortMode::Trunk { native, native_tag, allowed: AllowedList::All }), "Trunk allowed VLANs set to all.");
                }
                let list = match parse_vlan_list(&r.join("")) {
                    Ok(l) => l,
                    Err(e) => return o.err(e),
                };
                let (new, msg) = if remove {
                    let mut cur: BTreeSet<u16> = match allowed {
                        AllowedList::All => b.vlans().map(|v| v.vid).collect(),
                        AllowedList::Some(s) => s,
                    };
                    for v in &list {
                        cur.remove(v);
                    }
                    (cur, "Removed VLANs from trunk allowed list.")
                } else {
                    (list, "Trunk allowed VLAN list set.")
                };
                o.result(b.set_port_mode(i, PortMode::Trunk { native, native_tag, allowed: AllowedList::Some(new) }), msg);
            }
            _ => o.err("Usage: vlan trunk native <id> [tag] | vlan trunk allowed <list|all>"),
        },
        _ => o.err("Usage: vlan access <id> | vlan trunk native <id> [tag] | vlan trunk allowed <list|all>"),
    }
}

fn cmd_if_stp(b: &mut Bridge, o: &mut Out, i: PortRef, args: &[&str], remove: bool) {
    let mut c = match b.port_stp_config(i) {
        Some(c) => c.clone(),
        None => return o.err("Invalid interface"),
    };
    let msg: String = match args {
        [] => {
            c.enabled = !remove;
            (if remove { "STP disabled on port." } else { "STP enabled on port." }).into()
        }
        ["port-priority", v] if !remove => match v.parse::<u8>() {
            Ok(p) => {
                c.priority = p;
                "Port priority set.".into()
            }
            Err(_) => return o.err("Port-priority must be 0..240 in steps of 16"),
        },
        ["port-priority", ..] if remove => {
            c.priority = DEFAULT_PORT_PRIORITY;
            "Port priority reset.".into()
        }
        ["cost", v] if !remove => match v.parse::<u32>() {
            Ok(x) => {
                c.cost = x;
                "Port cost set.".into()
            }
            Err(_) => return o.err("Invalid cost"),
        },
        ["cost", ..] if remove => {
            c.cost = DEFAULT_PORT_COST;
            "Port cost reset.".into()
        }
        ["admin-edge-port"] => {
            c.admin_edge = !remove;
            (if remove { "admin-edge cleared." } else { "admin-edge set." }).into()
        }
        ["bpdu-guard"] => {
            c.bpdu_guard = !remove;
            (if remove { "bpdu-guard cleared." } else { "bpdu-guard enabled." }).into()
        }
        ["root-guard"] => {
            c.root_guard = !remove;
            (if remove { "root-guard cleared." } else { "root-guard enabled." }).into()
        }
        ["tcn-guard"] => {
            c.tcn_guard = !remove;
            (if remove { "tcn-guard cleared." } else { "tcn-guard enabled." }).into()
        }
        _ => return o.err("Usage: [no] spanning-tree [port-priority <0-240>|cost <n>|admin-edge-port|bpdu-guard|root-guard|tcn-guard]"),
    };
    o.result(b.set_port_stp(i, c), msg);
}

fn cmd_if_lldp(b: &mut Bridge, o: &mut Out, i: PortRef, args: &[&str], remove: bool) {
    let n = match i {
        PortRef::Eth(n) => n,
        _ => return,
    };
    let (tx, rx) = match args {
        ["transmit"] => (Some(!remove), None),
        ["receive"] => (None, Some(!remove)),
        _ => return o.err("Usage: [no] lldp [transmit|receive]"),
    };
    let what = args.first().copied().unwrap_or("");
    o.result(b.set_lldp_port(n, tx, rx), format!("LLDP {} {} on eth{}.", what, if remove { "disabled" } else { "enabled" }, n));
}

fn cmd_if_lag(b: &mut Bridge, o: &mut Out, i: PortRef, id: Option<&&str>) {
    let n = match i {
        PortRef::Eth(n) => n,
        _ => return,
    };
    match id {
        Some(s) => match s.parse::<u16>() {
            Ok(id) => o.result(b.set_lag_membership(n, Some(id)), format!("eth{} joined LAG {}.", n, id)),
            Err(_) => o.err("Invalid LAG id"),
        },
        None => match b.eth_config(n).and_then(|c| c.lag) {
            Some(old) => o.result(b.set_lag_membership(n, None), format!("eth{} removed from LAG {}.", n, old)),
            None => o.err(format!("eth{} is not a LAG member", n)),
        },
    }
}

fn cmd_lag_lacp(b: &mut Bridge, o: &mut Out, i: PortRef, args: &[&str], remove: bool) {
    let id = match i {
        PortRef::Lag(id) => id,
        _ => return,
    };
    match args {
        ["mode", ..] if remove => o.result(b.set_lacp_mode(id, LacpMode::Off), "LACP mode cleared (static LAG)."),
        ["mode", "active"] => o.result(b.set_lacp_mode(id, LacpMode::Active), "LACP mode set to active."),
        ["mode", "passive"] => o.result(b.set_lacp_mode(id, LacpMode::Passive), "LACP mode set to passive."),
        ["rate", ..] if remove => o.result(b.set_lacp_rate(id, LacpRate::Slow), "LACP rate set to slow."),
        ["rate", "fast"] => o.result(b.set_lacp_rate(id, LacpRate::Fast), "LACP rate set to fast."),
        ["rate", "slow"] => o.result(b.set_lacp_rate(id, LacpRate::Slow), "LACP rate set to slow."),
        ["fallback"] => o.result(b.set_lag_fallback(id, !remove), if remove { "LACP fallback disabled." } else { "LACP fallback enabled." }),
        _ => o.err("Usage: lacp mode [active|passive] | lacp rate [fast|slow] | [no] lacp fallback"),
    }
}

fn cmd_lag_hash(b: &mut Bridge, o: &mut Out, i: PortRef, h: Option<&&str>) {
    let id = match i {
        PortRef::Lag(id) => id,
        _ => return,
    };
    let mode = match h.copied() {
        None => HashMode::L3,
        Some("l2" | "l2-src-dst") => HashMode::L2,
        Some("l3" | "l3-src-dst") => HashMode::L3,
        Some("l4" | "l4-src-dst") => HashMode::L4,
        Some(_) => return o.err("Usage: hash [l2-src-dst|l3-src-dst|l4-src-dst]"),
    };
    o.result(b.set_lag_hash(id, mode), format!("Hash mode set to {}.", mode.as_str()));
}

// =====================================================================
// SVI context
// =====================================================================

fn exec_svi(b: &mut Bridge, o: &mut Out, t: &[&str], line: &str, vid: u16) {
    match t {
        ["ip", "address", a] | ["ip", "address", a, _] => {
            let parsed = match t.get(3) {
                Some(mask) => parse_ipv4(a).zip(parse_ipv4(mask)).and_then(|(ip, m)| {
                    let v = u32::from_be_bytes(m.0);
                    (v.leading_ones() + v.trailing_zeros() == 32).then(|| (ip, v.leading_ones() as u8))
                }),
                None => a.split_once('/').and_then(|(ip, p)| parse_ipv4(ip).zip(p.parse::<u8>().ok().filter(|p| *p <= 32))),
            };
            match parsed {
                Some((ip, prefix)) => match b.set_svi(vid, Some((ip, prefix))) {
                    Ok(()) => {
                        o.effects.push(CliEffect::SviAddress { vlan: vid, ip, prefix });
                        o.line(format!("Interface vlan {} address {}/{}.", vid, fmt_ip(&ip), prefix));
                    }
                    Err(e) => o.err(e.to_string()),
                },
                None => o.err("Usage: ip address <a.b.c.d/nn>"),
            }
        }
        ["no", "ip", "address", ..] => {
            if b.has_svi(vid) {
                let _ = b.set_svi(vid, None);
                o.effects.push(CliEffect::SviRemove { vlan: vid });
                o.line(format!("Interface vlan {} address removed.", vid));
            } else {
                o.err("No IP address configured");
            }
        }
        _ => o.err(format!("Invalid input in interface vlan context: {}", line)),
    }
}

// =====================================================================
// logging / lldp / spanning-tree (top level)
// =====================================================================

fn cmd_logging(b: &mut Bridge, o: &mut Out, args: &[&str], remove: bool) {
    match args {
        ["severity", ..] if remove => {
            b.set_log_severity(crate::log::DEFAULT_SEVERITY);
            o.line("Logging severity set to info.");
        }
        ["severity", lvl] => match Severity::parse(lvl) {
            Some(s) => {
                b.set_log_severity(s);
                o.line(format!("Logging severity set to {}.", s.as_str()));
            }
            None => o.err("Unknown severity (debug..emergency)"),
        },
        ["console"] => {
            b.set_log_console(!remove);
            o.line(if remove { "Console logging disabled." } else { "Console logging enabled." });
        }
        [ip] => match parse_ipv4(ip) {
            Some(ip) if remove => match b.remove_log_remote(ip) {
                Ok(()) => o.line(format!("Removed logging collector {}.", fmt_ip(&ip))),
                Err(_) => o.err("No matching collector"),
            },
            Some(ip) => o.result(b.add_log_remote(ip), format!("Added logging collector {}.", fmt_ip(&ip))),
            None => o.err("Unknown logging argument (expected ip, severity, or console)"),
        },
        _ => o.err("Usage: [no] logging <ip> | logging severity <lvl> | [no] logging console"),
    }
}

fn cmd_lldp(b: &mut Bridge, o: &mut Out, args: &[&str], remove: bool) {
    let num = |o: &mut Out, name: &str, r: Result<(), BridgeError>, v: u32| {
        o.result(r, if remove { format!("LLDP {} reset to default.", name) } else { format!("LLDP {} set to {}.", name, v) })
    };
    match args {
        [] => {
            b.set_lldp_enabled(!remove);
            o.line(if remove { "LLDP disabled." } else { "LLDP enabled." });
        }
        ["timer", ..] | ["holdtime", ..] | ["reinit", ..] | ["txdelay", ..] => {
            let name = args.first().copied().unwrap_or("");
            let (def, min, max) = match name {
                "timer" => (DEFAULT_LLDP_TIMER, 5, 32768),
                "holdtime" => (DEFAULT_LLDP_HOLDTIME, 2, 10),
                "reinit" => (DEFAULT_LLDP_REINIT, 1, 10),
                _ => (DEFAULT_LLDP_TXDELAY, 1, 8192),
            };
            let v = if remove {
                def
            } else {
                match (args.len(), parse_u32(args.get(1))) {
                    (2, Some(v)) => v,
                    _ => return o.err(format!("Usage: lldp {} <{}-{}>", name, min, max)),
                }
            };
            let r = match name {
                "timer" => b.set_lldp_timer(v),
                "holdtime" => b.set_lldp_holdtime(v),
                "reinit" => b.set_lldp_reinit(v),
                _ => b.set_lldp_txdelay(v),
            };
            num(o, name, r, v);
        }
        ["management-ipv4-address", ..] if remove => {
            b.set_lldp_mgmt_ipv4(None);
            o.line("LLDP management-ipv4-address cleared.");
        }
        ["management-ipv4-address", ip] => match parse_ipv4(ip) {
            Some(ip) => {
                b.set_lldp_mgmt_ipv4(Some(ip));
                o.line("LLDP management-ipv4-address set.");
            }
            None => o.err("Invalid IPv4 address"),
        },
        ["select-tlv", name] => match LldpTlv::parse(name) {
            Some(t) => {
                b.set_lldp_tlv(t, !remove);
                o.line(format!("LLDP TLV {} {}.", t.name(), if remove { "disabled" } else { "enabled" }));
            }
            None => o.err("Usage: [no] lldp select-tlv <port-desc|sys-name|sys-desc|sys-caps|mgmt-addr>"),
        },
        _ => o.err("Unknown lldp subcommand"),
    }
}

fn cmd_spanning_tree(b: &mut Bridge, o: &mut Out, args: &[&str], remove: bool) {
    let timer = |o: &mut Out, name: &str, r: Result<(), BridgeError>, v: u32| {
        o.result(r, if remove { format!("STP {} reset.", name) } else { format!("STP {} set to {}.", name, v) })
    };
    match args {
        [] => {
            b.set_stp_enabled(!remove);
            o.line(if remove { "Spanning tree disabled." } else { "Spanning tree enabled." });
        }
        ["priority", ..] if remove => o.result(b.set_stp_priority(DEFAULT_BRIDGE_PRIORITY), format!("STP priority set to {}.", DEFAULT_BRIDGE_PRIORITY)),
        ["priority", v] => match v.parse::<u16>() {
            Ok(p) => o.result(b.set_stp_priority(p), format!("STP priority set to {}.", p)),
            Err(_) => o.err("Priority must be 0..61440 in steps of 4096"),
        },
        ["forward-delay" | "hello-time" | "max-age", ..] => {
            let name = args.first().copied().unwrap_or("");
            let (def, min, max) = match name {
                "forward-delay" => (DEFAULT_FORWARD_DELAY_SECS, 4, 30),
                "hello-time" => (DEFAULT_HELLO_SECS, 1, 10),
                _ => (DEFAULT_MAX_AGE_SECS, 6, 40),
            };
            let v = if remove {
                def
            } else {
                match (args.len(), parse_u32(args.get(1))) {
                    (2, Some(v)) => v,
                    _ => return o.err(format!("Usage: spanning-tree {} <{}-{}>", name, min, max)),
                }
            };
            let r = match name {
                "forward-delay" => b.set_stp_forward_delay(v),
                "hello-time" => b.set_stp_hello(v),
                _ => b.set_stp_max_age(v),
            };
            timer(o, name, r, v);
        }
        ["config-name", ..] if remove => {
            b.set_stp_config_name(String::new());
            o.line("STP config-name cleared.");
        }
        ["config-name", rest @ ..] if !rest.is_empty() => {
            let name = rest.join(" ");
            b.set_stp_config_name(name.clone());
            o.line(format!("STP config-name set to {}.", name));
        }
        ["config-revision", ..] if remove => {
            b.set_stp_config_revision(0);
            o.line("STP config-revision reset.");
        }
        ["config-revision", v] => match v.parse::<u16>() {
            Ok(r) => {
                b.set_stp_config_revision(r);
                o.line(format!("STP config-revision set to {}.", r));
            }
            Err(_) => o.err("Invalid value (0-65535)"),
        },
        _ => o.err("Unknown spanning-tree subcommand"),
    }
}

// =====================================================================
// help
// =====================================================================

fn help(o: &mut Out, ctx: Context) {
    let lines: &[&str] = match ctx {
        Context::Top => &[
            "Switch configuration commands:",
            "  mac-address-table age-time <15-1000000>",
            "  no mac-address-table age-time",
            "  [no] static-mac <mac> vlan <vlan-id> port <port-num|lagN>",
            "  vlan <vlan-id>           Enter VLAN config (creates if absent)",
            "  vlan <start>-<end>       Bulk-create a range of VLANs",
            "  no vlan <vlan-id>        Delete a VLAN",
            "  interface <port|ethN>    Enter interface config",
            "  interface lag <id>       Enter (create) a LAG",
            "  interface vlan <id>      Enter SVI config (ip address)",
            "  no interface lag <id> | no interface vlan <id>",
            "  [no] logging <ip> | logging severity <lvl> | [no] logging console",
            "  [no] lldp | lldp timer|holdtime|reinit|txdelay <n> | [no] lldp select-tlv <tlv>",
            "  [no] lldp management-ipv4-address <ip>",
            "  [no] spanning-tree | spanning-tree priority|forward-delay|hello-time|max-age <n>",
            "  spanning-tree config-name <name> | config-revision <n>",
            "  on                       Keep the switch running after exit",
            "  write memory             Save running config to /switch.cfg",
            "  show mac-address-table [dynamic|static|vlan|port|address|count|mac-move ...]",
            "  show vlan [<id> | summary | port <n>]",
            "  show interface [brief] | show interface lag <id> | show interface vlan",
            "  show spanning-tree | show lacp [interfaces|aggregates|configuration]",
            "  show lldp [configuration|neighbor-info [port]|statistics|tlv|local-device]",
            "  show logging [-r] [severity <lvl>] | show running-config",
            "  clear mac-address-table dynamic [vlan <id> | port <n> | address <mac>]",
            "  clear logging | clear lldp [neighbors|statistics]",
            "  exit                     Leave the CLI",
        ],
        Context::Vlan(_) => &[
            "VLAN configuration commands:",
            "  name <name> | no name",
            "  description <text> | no description",
            "  no shutdown              Activate the VLAN",
            "  shutdown                 Deactivate the VLAN",
            "  exit / end               Return to the top-level context",
        ],
        Context::Eth(_) => &[
            "Interface configuration commands:",
            "  no routing               Convert to L2 (required first)",
            "  routing                  Convert back to L3",
            "  [no] shutdown            Admin down/up (no shutdown clears err-disable)",
            "  vlan access <vlan-id> | no vlan access",
            "  vlan trunk native <vlan-id> [tag] | no vlan trunk native",
            "  vlan trunk allowed <vlan-list|all> | no vlan trunk allowed <vlan-list>",
            "  [no] lldp [transmit|receive]",
            "  [no] spanning-tree [port-priority|cost|admin-edge-port|bpdu-guard|root-guard|tcn-guard]",
            "  lag <id> | no lag        Join/leave a LAG",
            "  exit / end               Return to the top-level context",
        ],
        Context::Lag(_) => &[
            "LAG configuration commands:",
            "  no routing | routing | [no] shutdown",
            "  vlan access <id> | vlan trunk native <id> [tag] | vlan trunk allowed <list|all>",
            "  lacp mode [active|passive] | no lacp mode (static)",
            "  lacp rate [fast|slow]",
            "  [no] lacp fallback",
            "  hash [l2-src-dst|l3-src-dst|l4-src-dst]",
            "  [no] spanning-tree [port-priority|cost|admin-edge-port|bpdu-guard|root-guard|tcn-guard]",
            "  exit / end               Return to the top-level context",
        ],
        Context::Svi(_) => &[
            "Interface VLAN (SVI) commands:",
            "  ip address <a.b.c.d/nn>",
            "  no ip address",
            "  exit / end               Return to the top-level context",
        ],
    };
    for l in lines {
        o.line(l);
    }
}

#[cfg(test)]
mod tests;
