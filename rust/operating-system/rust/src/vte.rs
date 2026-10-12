//! VT100/ANSI terminal emulator for the console.
//!
//! The parser lives in the shared `ecm-vte` crate (the GUI Terminal app uses
//! the same code). This module gives it the physical screen: the shared text
//! framebuffer (`hal::FB`) that the host renders.
//!
//! Supported escape sequences (see `ecm_vte`):
//! - CSI n A/B/C/D — cursor up/down/forward/back
//! - CSI n;m H/f   — cursor position
//! - CSI 2J / CSI J / CSI 1J — clear screen variants
//! - CSI K / CSI 0K / CSI 1K / CSI 2K — erase in line
//! - CSI n m — SGR (colors, bold, underline, inverse, reset)
//! - CSI ?25 h/l — show/hide cursor
//! - CSI s / CSI u — save/restore cursor
//! - CSI n S / CSI n T — scroll up/down
//! - \n, \r, \b, \t — standard control characters

use crate::framebuffer;

pub use ecm_vte::{Screen, VirtualScreen, Vte};

/// The shared text framebuffer as a [`Screen`].
pub struct Physical;

impl Screen for Physical {
    fn put_cell(&mut self, x: u16, y: u16, ch: u8, attr: u8, flags: u8) {
        framebuffer::write_cell(x, y, ch, attr, flags);
    }

    fn get_cell(&self, x: u16, y: u16, _default_attr: u8) -> (u8, u8, u8) {
        framebuffer::read_cell(x, y)
    }

    fn copy_row(&mut self, src_y: u16, dst_y: u16) {
        framebuffer::copy_row(src_y, dst_y);
    }

    fn read_row(&self, y: u16) -> Vec<u8> {
        framebuffer::read_row(y)
    }

    fn flush_cursor(&mut self, x: u16, y: u16, visible: bool) {
        framebuffer::set_cursor(x, y, visible);
        framebuffer::mark_dirty();
    }

    fn clamp(&self, width: u16, height: u16) -> (u16, u16) {
        framebuffer::clamp_dims(width, height)
    }

    fn resize(&mut self, _w0: u16, _h0: u16, w1: u16, h1: u16, shift: u16, _default_attr: u8) {
        framebuffer::resize(w1, h1, shift);
    }
}

/// Create a VTE that writes to the shared text framebuffer (`hal::FB`).
pub fn new_physical(width: u16, height: u16) -> Vte {
    Vte::with_screen(width, height, Box::new(Physical))
}
