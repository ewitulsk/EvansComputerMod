//! `ecm-ui`: the ECM GUI toolkit (docs/gui/UI_PROTOCOL.md §13).
//!
//! * [`element`] — widgets (label, button, text field, list, scroll view,
//!   checkbox, toggle, slider, tabs, menu, dialog, image, canvas) and layout
//!   containers (row, column, stack, grid, spacer, responsive), described as
//!   [`Element`] trees.
//! * [`window`] — widget state, layout at the window's size class, text
//!   wrapping with the ECM font metrics, painting into display-list nodes,
//!   `hit_id` routing and keyboard focus.
//! * [`diff`] — the diff engine that turns scene changes into minimal TreeOps.
//! * [`theme`] — the theme as data.
//! * [`app`] — [`App`], [`Core`] and the event loop; [`conn`] — the
//!   connection trait with the host and mock implementations.
//! * [`layout_file`] — `.ui.json` layouts with named handlers.
//!
//! ```ignore
//! use ecm_ui::prelude::*;
//!
//! let mut app = App::new("dev.evan.hello");
//! app.window("Hello", |_| column().padding(12).child(label("Hello, world")));
//! app.run();
//! ```

#![forbid(unsafe_code)]

pub mod app;
pub mod conn;
pub mod diff;
pub mod element;
pub mod json;
pub mod layout_file;
pub mod samples;
pub mod shell;
pub mod text;
pub mod theme;
pub mod window;

pub use app::{App, AppEvent, Core, Ctx};
pub use conn::{ConnError, Connection, MockConnection};
pub use element::{Align, Element, EventKind, Handler, Kind};
pub use json::Json;
pub use theme::Theme;
pub use window::{EventValue, UiEvent, ViewCtx, Window};

/// Everything an app usually needs.
pub mod prelude {
    pub use crate::app::{App, Ctx};
    pub use crate::element::{
        button, canvas, checkbox, column, dialog, grid, image, label, list, menu, responsive, row, scroll_view, slider, spacer,
        stack, tabs, text_field, toggle, Align, Element, EventKind, Handler,
    };
    pub use crate::json::Json;
    pub use crate::window::{EventValue, UiEvent, ViewCtx};
    pub use ecm_ui_proto::SizeClass;
}
