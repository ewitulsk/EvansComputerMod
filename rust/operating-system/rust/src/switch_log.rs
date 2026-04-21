//! Phase 3 — Switch logging subsystem.
//!
//! Rolling event buffer that captures switch-internal events (MAC
//! learn/move, port up/down, STP root/topology changes, LACP
//! transitions, LLDP neighbor add/remove, VLAN create/delete). Severity
//! filter gates buffer admission.
//!
//! Commands:
//!   logging <ipv4>                     Register a remote collector (see shortcut note).
//!   no logging <ipv4>                  Remove a collector.
//!   logging severity <level>           Filter threshold (debug..emergency).
//!   logging console                    Enable synchronous console mirror for CLI-path events.
//!   no logging console                 Disable console mirror.
//!   show logging [-r] [severity <lvl>] Dump buffer (newest first with -r).
//!   show events  [-r] [severity <lvl>] Alias.
//!   clear logging                      Wipe the buffer.
//!
//! Shortcut: remote syslog forwarding is *configured* but currently just
//! logged at debug severity and not actually transmitted. See
//! switch_instructions.md "Cut Corners" for the reason.

use crate::shell::ShellInstance;
use crate::switch::{self, SwitchState};

pub const MAX_LOG_ENTRIES: usize = 256;
pub const DEFAULT_SEVERITY: Severity = Severity::Info;

/// RFC-5424-style severities, ordered most to least severe.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    Emergency = 0,
    Alert = 1,
    Critical = 2,
    Error = 3,
    Warning = 4,
    Notice = 5,
    Info = 6,
    Debug = 7,
}

impl Severity {
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "debug" => Severity::Debug,
            "info" | "informational" => Severity::Info,
            "notice" => Severity::Notice,
            "warning" | "warn" => Severity::Warning,
            "error" | "err" => Severity::Error,
            "critical" | "crit" => Severity::Critical,
            "alert" => Severity::Alert,
            "emergency" | "emerg" => Severity::Emergency,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Debug => "debug",
            Severity::Info => "info",
            Severity::Notice => "notice",
            Severity::Warning => "warning",
            Severity::Error => "error",
            Severity::Critical => "critical",
            Severity::Alert => "alert",
            Severity::Emergency => "emergency",
        }
    }

    /// Is `self` at least as severe as `threshold`? (Lower numeric = more severe.)
    pub fn passes(&self, threshold: Severity) -> bool {
        (*self as u8) <= (threshold as u8)
    }
}

#[derive(Clone)]
pub struct LogEntry {
    pub ts_ms: i64,
    pub severity: Severity,
    pub message: String,
}

#[derive(Clone)]
pub struct RemoteCollector {
    pub ip: [u8; 4],
}

/// Per-switch logging state.
pub struct LogState {
    pub entries: Vec<LogEntry>,
    pub severity: Severity,
    pub console: bool,
    pub remotes: Vec<RemoteCollector>,
}

impl LogState {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            severity: DEFAULT_SEVERITY,
            console: false,
            remotes: Vec::new(),
        }
    }

    fn push(&mut self, entry: LogEntry) {
        if self.entries.len() >= MAX_LOG_ENTRIES {
            self.entries.remove(0);
        }
        self.entries.push(entry);
    }
}

// =====================================================================
// Log emission — callable from any phase
// =====================================================================

/// Record an event in the buffer if its severity is at or above the
/// configured threshold. Safe to call from any IRQ path; never touches
/// the shell (use `log_cli` from a CLI command handler to also print).
pub fn log(severity: Severity, msg: impl Into<String>) {
    let msg = msg.into();
    let now_ms = crate::net::NetStack::get().map(|s| s.now_ms).unwrap_or(0);
    switch::with_state_mut(|state| {
        if !severity.passes(state.log.severity) {
            return;
        }
        state.log.push(LogEntry { ts_ms: now_ms, severity, message: msg });
    });
}

/// Variant that also prints to the shell when `logging console` is set.
/// Used from exec_command paths where a shell reference is available.
pub fn log_cli(shell: &mut ShellInstance, severity: Severity, msg: impl Into<String>) {
    let msg = msg.into();
    let now_ms = crate::net::NetStack::get().map(|s| s.now_ms).unwrap_or(0);
    let (mirror, entry_snapshot) = switch::with_state_mut(|state| {
        if !severity.passes(state.log.severity) {
            return (false, None);
        }
        let e = LogEntry { ts_ms: now_ms, severity, message: msg.clone() };
        let should = state.log.console;
        state.log.push(e.clone());
        (should, Some(e))
    }).unwrap_or((false, None));
    if mirror {
        if let Some(e) = entry_snapshot {
            shell.println(&format_entry(&e));
        }
    }
}

pub fn format_entry(e: &LogEntry) -> String {
    format!(
        "[{}] {:>9}  {}",
        format_time(e.ts_ms),
        e.severity.as_str(),
        e.message,
    )
}

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

// =====================================================================
// Convenience emitters
// =====================================================================

pub fn info(msg: impl Into<String>)     { log(Severity::Info, msg) }
pub fn notice(msg: impl Into<String>)   { log(Severity::Notice, msg) }
pub fn warning(msg: impl Into<String>)  { log(Severity::Warning, msg) }
pub fn error(msg: impl Into<String>)    { log(Severity::Error, msg) }
pub fn debug(msg: impl Into<String>)    { log(Severity::Debug, msg) }

// =====================================================================
// CLI — top-level commands
// =====================================================================

pub fn cmd_logging(shell: &mut ShellInstance, args: &[&str], remove: bool) {
    if args.is_empty() {
        shell.println("% Usage: [no] logging <ip> | logging severity <lvl> | logging console");
        return;
    }
    match args[0] {
        "severity" => {
            if remove {
                set_severity(shell, DEFAULT_SEVERITY);
                return;
            }
            if args.len() != 2 {
                shell.println("% Usage: logging severity <level>");
                return;
            }
            match Severity::from_str(args[1]) {
                Some(s) => set_severity(shell, s),
                None => shell.println("% Unknown severity (debug..emergency)"),
            }
        }
        "console" => {
            if args.len() != 1 {
                shell.println("% Usage: [no] logging console");
                return;
            }
            set_console(shell, !remove);
        }
        other => {
            match parse_ipv4(other) {
                Some(ip) => {
                    if remove { remove_remote(shell, ip); } else { add_remote(shell, ip); }
                }
                None => {
                    shell.println("% Unknown logging argument (expected ip, severity, or console)");
                }
            }
        }
    }
}

fn set_severity(shell: &mut ShellInstance, s: Severity) {
    switch::with_state_mut(|state| { state.log.severity = s; });
    let msg = format!("Logging severity set to {}.", s.as_str());
    shell.println(&msg);
}

fn set_console(shell: &mut ShellInstance, on: bool) {
    switch::with_state_mut(|state| { state.log.console = on; });
    shell.println(if on { "Console logging enabled." } else { "Console logging disabled." });
}

fn add_remote(shell: &mut ShellInstance, ip: [u8; 4]) {
    switch::with_state_mut(|state| {
        if !state.log.remotes.iter().any(|r| r.ip == ip) {
            state.log.remotes.push(RemoteCollector { ip });
        }
    });
    let msg = format!("Added logging collector {}.{}.{}.{}.", ip[0], ip[1], ip[2], ip[3]);
    shell.println(&msg);
}

fn remove_remote(shell: &mut ShellInstance, ip: [u8; 4]) {
    let removed = switch::with_state_mut(|state| {
        let before = state.log.remotes.len();
        state.log.remotes.retain(|r| r.ip != ip);
        before != state.log.remotes.len()
    }).unwrap_or(false);
    if removed {
        let msg = format!("Removed logging collector {}.{}.{}.{}.", ip[0], ip[1], ip[2], ip[3]);
        shell.println(&msg);
    } else {
        shell.println("% No matching collector");
    }
}

// =====================================================================
// show logging / show events / clear logging
// =====================================================================

pub fn show(shell: &mut ShellInstance, args: &[&str]) {
    let mut reverse = false;
    let mut min_sev: Option<Severity> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "-r" | "reverse" => { reverse = true; i += 1; }
            "severity" => {
                if i + 1 >= args.len() {
                    shell.println("% Missing severity");
                    return;
                }
                match Severity::from_str(args[i + 1]) {
                    Some(s) => { min_sev = Some(s); i += 2; }
                    None => { shell.println("% Unknown severity"); return; }
                }
            }
            _ => { shell.println("% Usage: show logging [-r] [severity <lvl>]"); return; }
        }
    }

    // Snapshot state in a single closure to avoid multiple borrows.
    let snapshot: Option<(Severity, bool, Vec<RemoteCollector>, Vec<LogEntry>)> =
        switch::with_state(|state| {
            (
                state.log.severity,
                state.log.console,
                state.log.remotes.clone(),
                state.log.entries.clone(),
            )
        });

    let (severity, console, remotes, entries) = match snapshot {
        Some(s) => s,
        None => return,
    };

    shell.println(&format!("Logging severity: {}", severity.as_str()));
    shell.println(&format!("Console: {}", if console { "on" } else { "off" }));
    if !remotes.is_empty() {
        let list: Vec<String> = remotes.iter()
            .map(|r| format!("{}.{}.{}.{}", r.ip[0], r.ip[1], r.ip[2], r.ip[3]))
            .collect();
        shell.println(&format!("Collectors: {}", list.join(", ")));
    }
    shell.println(&format!("Entries: {}", entries.len()));
    shell.println("");
    let iter: Box<dyn Iterator<Item = &LogEntry>> = if reverse {
        Box::new(entries.iter().rev())
    } else {
        Box::new(entries.iter())
    };
    for e in iter {
        if let Some(thr) = min_sev {
            if !e.severity.passes(thr) { continue; }
        }
        shell.println(&format_entry(e));
    }
}

pub fn clear(shell: &mut ShellInstance) {
    switch::with_state_mut(|state| { state.log.entries.clear(); });
    shell.println("Log buffer cleared.");
}

// =====================================================================
// Persistence helpers — called from serialize_running_config.
// =====================================================================

pub fn serialize(state: &SwitchState, out: &mut String) {
    if state.log.severity != DEFAULT_SEVERITY {
        out.push_str(&format!("logging severity {}\n", state.log.severity.as_str()));
    }
    if state.log.console {
        out.push_str("logging console\n");
    }
    for r in &state.log.remotes {
        out.push_str(&format!("logging {}.{}.{}.{}\n", r.ip[0], r.ip[1], r.ip[2], r.ip[3]));
    }
}

// =====================================================================
// Parsing helpers
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
