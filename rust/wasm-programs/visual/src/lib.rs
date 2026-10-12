//! The Visual Programmer (docs/gui/VISUAL_PROGRAMMER.md), rebuilt as an
//! `ecm-ui` app (ROADMAP P6).
//!
//! * [`blocks`] — block definitions (`blocks.json` plus the host's
//!   peripheral catalog);
//! * [`graph`] — the node graph and the `.vpl` format;
//! * [`codegen`] — graph → Python (a faithful port of the old Java
//!   generator, plus GUI apps);
//! * [`geometry`] — block, port and wire positions at a zoom level;
//! * [`designer`] — the `.ui.json` document the GUI designer edits;
//! * [`package`] — app folders with `app.toml`;
//! * [`editor`] — the editor's state and actions; [`view`] draws it;
//! * [`files`], [`runner`] — the file system and running programs, behind
//!   traits so everything runs in host tests.

pub mod blocks;
pub mod cli;
pub mod codegen;
pub mod designer;
pub mod editor;
pub mod files;
pub mod geometry;
pub mod graph;
pub mod package;
pub mod runner;
pub mod samples;
pub mod view;

/// What `visual` prints when no GUI is running.
pub const CONSOLE_MESSAGE: &str = "visual: the Visual Programmer is a GUI app. Start the GUI with `gui`, then run `visual` in the Terminal app (or `gui launch visual`).";

/// The app id the Visual Programmer runs as.
pub const APP_ID: &str = "dev.ecm.visual";
