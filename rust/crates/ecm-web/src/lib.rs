//! `ecm-web`: the engine of the in-game Browser (docs/gui/BROWSER.md).
//!
//! A page goes through five stages, each its own module:
//!
//! 1. [`fetch`]: `http://` (HTTP/1.0 with `Host:`, redirects, form posts,
//!    per-host session cookies) and `file://`, over a [`fetch::Transport`]
//!    the program supplies (the computer's sockets and disk; scripted ones in
//!    tests). [`url`] parses and resolves addresses.
//! 2. [`html`]: a forgiving tokenizer and tree builder into a small
//!    [`dom::Dom`]. Unknown tags keep their text; no input makes it fail.
//! 3. [`css`] and [`style`]: style sheets (`<style>`, `<link rel=stylesheet>`,
//!    `style=""`, `@media`), selectors with specificity, and the cascade into
//!    a computed [`style::Style`] per element. Unstyled pages take their
//!    colours from the shell theme ([`style::UaColors`]).
//! 4. [`layout`]: block flow, inline text word-wrapped with the ECM font
//!    metrics, flexbox, relative offsets.
//! 5. [`paint`]: a display list of rects and single-line texts in page
//!    coordinates, plus hit boxes that map a point back to its DOM node.
//!
//! [`page::Page`] strings the stages together.

pub mod css;
pub mod dom;
pub mod fetch;
pub mod html;
pub mod layout;
pub mod page;
pub mod paint;
pub mod style;
pub mod url;

pub use dom::{Dom, NodeId};
pub use page::{Page, Rendered};
pub use url::Url;
