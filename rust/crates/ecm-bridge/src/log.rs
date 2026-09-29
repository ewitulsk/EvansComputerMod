//! Severity-filtered event log with a bounded ring buffer.

use std::collections::VecDeque;

/// Ring capacity; the oldest entry is dropped when full.
pub const LOG_CAPACITY: usize = 256;
pub const DEFAULT_SEVERITY: Severity = Severity::Info;
/// Remote syslog collectors kept (configuration only).
pub const MAX_REMOTES: usize = 8;

/// RFC 5424 severities; lower value = more severe.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
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
    pub fn parse(s: &str) -> Option<Self> {
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

    /// True if `self` is at least as severe as `threshold`.
    pub fn passes(&self, threshold: Severity) -> bool {
        (*self as u8) <= (threshold as u8)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub ts_ms: i64,
    pub severity: Severity,
    pub message: String,
}

impl LogEntry {
    pub fn format(&self) -> String {
        format!("[{}] {:>9}  {}", format_time(self.ts_ms), self.severity.as_str(), self.message)
    }
}

/// Format a millisecond timestamp as `HH:MM:SS.mmm` (wrapping at 24 h).
pub fn format_time(ms: i64) -> String {
    if ms < 0 {
        return "-".to_string();
    }
    let secs = ms / 1_000;
    format!("{:02}:{:02}:{:02}.{:03}", (secs / 3600) % 24, (secs / 60) % 60, secs % 60, ms % 1_000)
}

#[derive(Clone, Debug)]
pub(crate) struct Logger {
    pub ring: VecDeque<LogEntry>,
    pub severity: Severity,
    pub console: bool,
    pub remotes: Vec<[u8; 4]>,
}

impl Logger {
    pub fn new() -> Self {
        Logger { ring: VecDeque::with_capacity(LOG_CAPACITY), severity: DEFAULT_SEVERITY, console: false, remotes: Vec::new() }
    }

    /// Record an entry if it passes the filter. Returns true when admitted.
    pub fn record(&mut self, ts_ms: i64, severity: Severity, message: &str) -> bool {
        if !severity.passes(self.severity) {
            return false;
        }
        if self.ring.len() >= LOG_CAPACITY {
            self.ring.pop_front();
        }
        self.ring.push_back(LogEntry { ts_ms, severity, message: message.to_string() });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_is_bounded_and_filtered() {
        let mut l = Logger::new();
        assert!(!l.record(0, Severity::Debug, "dropped"));
        for i in 0..300 {
            assert!(l.record(i, Severity::Info, &format!("m{}", i)));
        }
        assert_eq!(l.ring.len(), LOG_CAPACITY);
        assert_eq!(l.ring.front().map(|e| e.message.as_str()), Some("m44"));
        l.severity = Severity::Error;
        assert!(!l.record(1, Severity::Warning, "x"));
        assert!(l.record(1, Severity::Critical, "y"));
    }

    #[test]
    fn severity_parse_and_order() {
        for s in ["debug", "info", "notice", "warning", "error", "critical", "alert", "emergency"] {
            assert_eq!(Severity::parse(s).map(|x| x.as_str()), Some(s));
        }
        assert_eq!(Severity::parse("bogus"), None);
        assert!(Severity::Error.passes(Severity::Info));
        assert!(!Severity::Debug.passes(Severity::Info));
    }

    #[test]
    fn time_format() {
        assert_eq!(format_time(3_723_004), "01:02:03.004");
        assert_eq!(format_time(0), "00:00:00.000");
    }
}
