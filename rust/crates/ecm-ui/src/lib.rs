//! `ecm-ui`: an immediate-mode, sans-IO UI toolkit for programs that render
//! into an RGBA8888 framebuffer (EvansComputerMod screens / terminal gfx).
//!
//! * [`Canvas`] — pixels, clipping, built-in 5x9 bitmap font, dirty rects.
//! * [`Ui`] — persistent widget state; [`Ui::begin_frame`] returns a
//!   [`Frame`] on which widgets are called; [`Frame::end`] yields a
//!   [`FrameOutput`] with the [`ItemPlacement`]s the client draws on top.
//!
//! No host calls, no dependencies: the program feeds [`InputEvent`]s in and
//! blits `Canvas::take_dirty_rects` / `Canvas::pixels_of` out.

mod canvas;
mod font;
mod geom;
mod input;
mod theme;
mod ui;

pub use canvas::{fit_text, text_height, text_width, Align, Canvas, CAP_HEIGHT, CHAR_ADVANCE, GLYPH_HEIGHT, LINE_HEIGHT, MAX_CANVAS_EDGE, MAX_DIRTY_RECTS};
pub use geom::{Color, Rect};
pub use input::{InputEvent, Key, KeyOrChar, Modifiers, MouseButton};
pub use theme::Theme;
pub use ui::{
    id, label_width, Frame, FrameOutput, GridResponse, ItemPlacement, ModalOptions, Response, Ui, WidgetId, ITEM_FLAG_DIMMED,
    ITEM_FLAG_HOVERED, ITEM_FLAG_SELECTED,
};
