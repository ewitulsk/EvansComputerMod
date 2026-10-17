//! Discord for the mod's computers (docs/DISCORD.md).
//!
//! * [`proto`]: the `DISCORD/1` line protocol between the Discord app and a backend
//!   (`discordd`), a superset of the old chat protocol (so `chat` still works).
//! * [`server`]: the backend as a state machine: servers ("guilds"), text and voice
//!   channels, members, history, voice forwarding and the saved state.
//! * [`voice`]: voice channels' UDP packets, the microphone gate and the speaker mixer.
//!
//! Nothing here calls the host, so everything is unit-tested with `cargo test -p ecm-discord`.

pub mod proto;
pub mod server;
pub mod voice;
