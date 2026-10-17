//! The console side of the Python and Lua interpreters: [`term`] (drawing),
//! [`events`] (one event queue), [`clock`] (in-game and real time),
//! [`colors`], [`keys`], [`fs`] (CC-style paths over the computer's files) and
//! [`textutils`] (values as Lua literals and JSON). Behaviour follows ComputerCraft's documented APIs
//! (tweaked.cc) so its programs run unchanged; nothing here talks to the host
//! directly, so all of it is tested with `cargo test -p ecm-console`.

pub mod clock;
pub mod colors;
pub mod events;
pub mod fs;
pub mod keys;
pub mod redstone;
pub mod session;
pub mod term;
pub mod textutils;
