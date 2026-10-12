//! `celld`, the phone's cellular daemon (Cellular v2, docs/gui/CELLULAR_V2.md
//! "As built: phase 3 phone side"), like `wpa_supplicant` for Wi-Fi.
//!
//! | Module | What |
//! |---|---|
//! | [`daemon`] | the driver loop around the `ecm-cell` UE machine, over an [`daemon::Io`] |
//! | [`ctl`] | the control socket protocol (UDP 127.0.0.1:7710), status and cells, the `cell` command's pages |
//! | [`config`] | `/etc/celld.toml` |
//! | `host` (WASI) | the computer: framed radio, `sim` peripheral, tun, netlink, `/dev/uictl`, `main` |
//!
//! The simulator's phone agent runs [`daemon::Daemon`] over the simulated
//! medium and its node's kernel, so its scenarios exercise this code.

pub mod config;
pub mod ctl;
pub mod daemon;
#[cfg(target_os = "wasi")]
pub mod host;

pub use daemon::{describe_event, Daemon, Io, Options};
