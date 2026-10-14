//! Chat (`dev.ecm.chat`): a GUI messaging app for the Tech Village chat
//! service (docs/gui/APPS.md "Chat"). It speaks the existing line protocol
//! (`ecm_chat::proto`) to `chatd`, so it talks to the `chat` command and to
//! every other client of a chat server.
//!
//! * [`model`]: the conversation as plain state (who is here, the messages,
//!   unread count, the connection), fed protocol events. No I/O.
//! * [`view`]: the window at the three size classes: compact (phones: the
//!   conversation, with People one tap away), medium (a sidebar with the room
//!   and the people beside it) and wide (room list, conversation, people).
//! * [`app`]: [`app::ChatApp`] runs the window, a [`app::Net`] connection and
//!   the notifications (a message that arrives while the window isn't on
//!   screen posts one).
//! * [`source`]: the package folder (`chatapp --source <dir>`), for `pkg build`.

pub mod app;
pub mod model;
pub mod source;
pub mod view;

/// The app id.
pub const APP_ID: &str = "dev.ecm.chat";
/// The icon (32x32 pixel art, `assets/make_icon.py`).
pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");
