//! The console `chat` command's side of instant messaging.
//!
//! * [`proto`]: the line protocol `chat` speaks (UTF-8 lines over TCP, default port
//!   [`proto::DEFAULT_PORT`]). Discord backends (`discordd`, crate `ecm-discord`)
//!   accept it: `chat` lands in the backend's home server, channel `#general`.
//! * [`client`]: argument/config parsing, the line editor that keeps the typed line
//!   intact while messages arrive, how events are displayed, and fix-it hints.
//!
//! Nothing here calls the host, so everything is unit-tested with `cargo test -p ecm-chat`.

pub mod client;
pub mod proto;
