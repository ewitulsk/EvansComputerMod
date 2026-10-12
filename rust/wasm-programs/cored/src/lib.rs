//! `cored`, the Cellular v2 carrier core (docs/gui/CELLULAR_V2.md, "As built: phase 3
//! network side"): the program side that is tested on the host.
//!
//! * [`config`]: `/etc/cored.toml` (program settings; the network is the database).
//! * [`daemon`]: the running core without its I/O: the `ecm_cell` core machine, the
//!   logs, SIM provisioning through a Card Reader/Writer, the admin commands and
//!   their reply formats (with parsers for the Carrier Console).
//!
//! `main.rs` is the program: the CLI and the loop over the sites' control
//! connections, the tunnel socket, the `cell0` tun and the admin socket.

pub mod config;
pub mod daemon;

/// The Carrier Console app's id.
pub const APP_ID: &str = "dev.ecm.carrier";
