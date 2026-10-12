//! The Cellular v2 operator apps (docs/gui/CELLULAR_V2.md, "As built: phase 3
//! network side"), on `ecm-ui`:
//!
//! * [`site`]: **Cell Site** (`dev.ecm.cellsite`, program `cellsite`): the site's
//!   status, radio, core link, phones with their signal and CQI, neighbours,
//!   traffic; its settings, saved through `celld-site` and applied with Restart.
//! * [`carrier`]: **Carrier Console** (`dev.ecm.carrier`, program `carrier`):
//!   subscribers (write a SIM through the Card Reader/Writer, revoke, quota,
//!   usage), sites (connected, last heartbeat, add one with a new credential),
//!   live phones, the handover log, the core's settings.
//!
//! Both talk to their daemon's admin socket (127.0.0.1, `ecm_cell::admin`)
//! through [`Admin`]: the program's is a TCP connection (`ecm-cell-net`), the
//! tests' and the golden recorder's are [`fake`]s. Handlers only change the
//! model and queue commands; between loop turns the app runs them, polls the
//! daemon once a second and redraws.

pub mod carrier;
pub mod fake;
pub mod site;
pub mod testkit;
pub mod widgets;

/// A daemon's admin socket as the apps use it.
pub trait Admin {
    /// One command. Outer `Err`: no daemon (not running, the connection failed);
    /// inner `Err`: the daemon's `err` reply.
    fn call(&mut self, cmd: &str) -> Result<Result<Vec<String>, String>, String>;
}

/// How often the apps poll their daemon, ms.
pub const POLL_MS: u64 = 1000;

/// Whether the daemon answered the last poll.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Daemon {
    Unknown,
    Running,
    /// Not running or unreachable, and why.
    Down(String),
}

/// `12.3 KB`.
pub fn fmt_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else if n < 1024 * 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", n as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

/// `3 s`, `4 min`, `2 h 5 min`.
pub fn fmt_secs(s: u64) -> String {
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h {} min", s / 3600, (s % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(12_600), "12.3 KB");
        assert_eq!(fmt_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(fmt_secs(42), "42 s");
        assert_eq!(fmt_secs(7500), "2 h 5 min");
    }
}
