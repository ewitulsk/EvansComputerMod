//! The Browser (`dev.ecm.browser`, docs/gui/BROWSER.md): one app for the
//! Terminal desktop and the ePhone. The engine is `ecm-web`; this crate is the
//! window around it.
//!
//! * [`model`]: the page, history, scrolling, hover and what to load next;
//! * [`view`]: the window: an address bar with Back / Forward / Reload /
//!   Home (on a phone, compact: the address on top and the buttons in a bar at
//!   the bottom), the page in one custom area, a status line;
//! * [`scene`]: the engine's display list as scene nodes, only what is on
//!   screen, scrolled by one group;
//! * [`app`]: [`app::BrowserApp`], the event loop and the fetching between
//!   loop turns (through an `ecm_web::fetch::Transport`: the computer's
//!   sockets and disk in the program, scripted ones in tests);
//! * [`pages`]: the browser's own pages (start page, errors).

pub mod app;
pub mod fake;
pub mod model;
pub mod pages;
pub mod samples;
pub mod scene;
pub mod view;

pub const APP_ID: &str = "dev.ecm.browser";

#[cfg(test)]
mod tests;
