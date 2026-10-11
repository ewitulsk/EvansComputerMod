//! Instant messaging for the Tech Villages.
//!
//! * [`proto`]: the line protocol between `chat` and `chatd` (UTF-8 lines over TCP,
//!   default port [`proto::DEFAULT_PORT`]).
//! * [`server`]: the chat room as a state machine (nicknames, broadcast, join/leave
//!   notices, history replay). `chatd` feeds it socket events and writes what it returns.
//! * [`client`]: argument/config parsing, the line editor that keeps the typed line
//!   intact while messages arrive, how events are displayed, and fix-it hints.
//!
//! Nothing here calls the host, so everything is unit-tested with `cargo test -p ecm-chat`.

pub mod client;
pub mod proto;
pub mod server;
