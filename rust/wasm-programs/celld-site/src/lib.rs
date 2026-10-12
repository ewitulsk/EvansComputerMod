//! `celld-site`, the Cellular v2 cell-site daemon (docs/gui/CELLULAR_V2.md, "As built:
//! phase 3 network side"): the program side that is tested on the host.
//!
//! * [`config`]: `/etc/celld-site.toml`, strict, every key checked.
//! * [`daemon`]: the running site without its I/O: the `ecm_cell` site machine, the
//!   log, the admin commands and the `status` format (and its parser for the Cell Site app).
//!
//! `main.rs` is the program: the CLI and the loop over the framed radio, the
//! control connection to the core, the tunnel socket and the admin socket.

pub mod config;
pub mod daemon;

/// The Cell Site app's id.
pub const APP_ID: &str = "dev.ecm.cellsite";
