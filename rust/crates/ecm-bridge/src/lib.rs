//! `ecm-bridge`: a pure, sans-IO, panic-free L2 switch.
//!
//! # Driving model
//!
//! The bridge performs no I/O and never reads a clock. The caller:
//!
//! * feeds every frame received on an L2 port with [`Bridge::handle_frame`]
//!   (frames on ports where [`Bridge::is_l2_port`] is false belong to the
//!   host stack);
//! * feeds frames the switch's own stack sends on an SVI with
//!   [`Bridge::send_local`];
//! * reports carrier changes with [`Bridge::set_link`];
//! * calls [`Bridge::poll`] when the returned deadline passes (calling it
//!   early is always safe);
//! * drains [`Bridge::pop_output`] after each of the above:
//!   [`Output::Tx`] goes to a physical port, [`Output::Local`] to the SVI of
//!   that VLAN, [`Output::Log`] to the console/syslog if desired.
//!
//! Configuration is changed through the typed API (`set_*`, `create_*`) or
//! through the AOS-CX style CLI in [`cli`].

pub mod bpdu;
pub mod cli;
pub mod frame;
pub mod lacpdu;
pub mod lldpdu;
pub mod log;
pub mod types;

mod api;
mod bridge;
mod fdb;
mod lacp;
mod lldp;
mod stp;

pub use api::{FdbEntryInfo, FdbFilter, StpPortStatus, StpStatus};
pub use bridge::{fmt_ip, fmt_mac, Bridge, Output, PortCounters, OUTPUT_CAPACITY};
pub use fdb::FDB_CAPACITY;
pub use frame::HashMode;
pub use lacp::{LacpMemberStatus, LacpMux, LacpRx};
pub use lldp::{LldpNeighbor, LldpPortStats, MAX_NEIGHBORS_PER_PORT};
pub use log::{LogEntry, Severity, LOG_CAPACITY};
pub use stp::{format_bid, StpPortState, StpRole};
pub use types::*;
