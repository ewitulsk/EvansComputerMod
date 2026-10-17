//! Discord (`dev.ecm.discord`): servers with text and voice channels, on every
//! Discord backend (`discordd`) the computer can reach (docs/DISCORD.md).
//!
//! * [`model`]: the backends, their servers, channels, members, messages and the
//!   voice channel we are in, as plain state fed protocol events. No I/O.
//! * [`view`]: the window in Discord's colours at the three size classes.
//! * [`app`]: [`app::DiscordApp`] runs the window, a connection per backend, the
//!   search for backends on the network, voice and notifications.
//! * [`source`]: the package folder (`discord --source <dir>`), for `pkg build`.

pub mod app;
pub mod model;
pub mod source;
pub mod view;

/// The app id.
pub const APP_ID: &str = "dev.ecm.discord";
/// The icon (32x32: the purple Discord logo, painted in Blockbench: assets/discord-logo.bbmodel).
pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");
