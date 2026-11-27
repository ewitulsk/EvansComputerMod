//! The core GUI apps of ROADMAP P3 (see the crate description).
//!
//! * [`desktop`] — the desktop shell `dev.ecm.desktop`.
//! * [`terminal`] — a terminal emulator over the kernel's shell sessions.
//! * [`files`] — browse, open, rename, delete, copy and paste files.
//! * [`settings`] — Wi-Fi, cellular, display, appearance and about.
//! * [`editor`] — the Text Editor.
//! * [`sys`] — the computer as the apps see it; [`uitoml`] — `/etc/ui.toml`.

pub mod app_catalog;
pub mod desktop;
pub mod desktop_icons;
pub mod editor;
pub mod files;
pub mod settings;
pub mod start_menu;
pub mod sys;
pub mod terminal;
pub mod uitoml;
