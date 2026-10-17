//! musicd: the music backend (plan: `plans/background-apps/MUSIC_AND_AUTH.md`,
//! "Music backend musicd"). The server owner decides which songs exist by
//! putting MP3s in the library folder; musicd indexes them (tags, timing,
//! cover art), answers search and browsing, keeps playlists, likes and
//! recently played per account, and streams the songs with HTTP `Range`
//! (Original, or the 22 kHz ADPCM Data saver copy it makes once per song).
//!
//! The logic is here as a library so it is tested on the host: [`api::App`]
//! answers requests, [`conn::Conn`] is one HTTP connection, [`auth`] checks
//! tokens. `main.rs` has the sockets and the poll loop.

pub mod api;
pub mod auth;
pub mod config;
pub mod conn;
pub mod disk;

#[cfg(test)]
mod tests;
