//! VT100/ANSI terminal emulator for the console.
//!
//! The parser lives in the shared `ecm-vte` crate (the GUI Terminal app uses
//! the same code). This module gives it the physical screen: the shared text
//! framebuffer (`hal::FB`) that the host renders.
//!
//! The escape sequences it understands are listed in `ecm_vte`.

use crate::framebuffer;
use std::cell::RefCell;
use std::rc::Rc;

pub use ecm_vte::{Screen, VirtualScreen, Vte, CELL_SIZE};

/// While the console shows its scrollback, the live screen's cells are kept
/// here and the VTE draws into them; the framebuffer shows the scrolled view.
pub type Held = Rc<RefCell<Option<Vec<u8>>>>;

/// The shared text framebuffer as a [`Screen`].
pub struct Physical {
    held: Held,
}

impl Physical {
    fn off(x: u16, y: u16) -> Option<usize> {
        let (w, h) = (framebuffer::width(), framebuffer::height());
        (x < w && y < h).then(|| (y as usize * w as usize + x as usize) * CELL_SIZE)
    }
}

impl Screen for Physical {
    fn put_cell(&mut self, x: u16, y: u16, ch: u8, attr: u8, flags: u8) {
        match self.held.borrow_mut().as_mut() {
            Some(cells) => {
                if let Some(o) = Self::off(x, y).filter(|o| o + CELL_SIZE <= cells.len()) {
                    cells[o..o + CELL_SIZE].copy_from_slice(&[ch, attr, flags, 0]);
                }
            }
            None => framebuffer::write_cell(x, y, ch, attr, flags),
        }
    }

    fn get_cell(&self, x: u16, y: u16, default_attr: u8) -> (u8, u8, u8) {
        match self.held.borrow().as_ref() {
            Some(cells) => match Self::off(x, y).filter(|o| o + CELL_SIZE <= cells.len()) {
                Some(o) => (cells[o], cells[o + 1], cells[o + 2]),
                None => (b' ', default_attr, 0),
            },
            None => framebuffer::read_cell(x, y),
        }
    }

    fn copy_row(&mut self, src_y: u16, dst_y: u16) {
        match self.held.borrow_mut().as_mut() {
            Some(cells) => {
                let row = framebuffer::width() as usize * CELL_SIZE;
                let (s, d) = (src_y as usize * row, dst_y as usize * row);
                if s + row <= cells.len() && d + row <= cells.len() {
                    cells.copy_within(s..s + row, d);
                }
            }
            None => framebuffer::copy_row(src_y, dst_y),
        }
    }

    fn read_row(&self, y: u16) -> Vec<u8> {
        match self.held.borrow().as_ref() {
            Some(cells) => {
                let row = framebuffer::width() as usize * CELL_SIZE;
                let s = y as usize * row;
                cells.get(s..s + row).map(|r| r.to_vec()).unwrap_or_else(|| vec![0u8; row])
            }
            None => framebuffer::read_row(y),
        }
    }

    fn flush_cursor(&mut self, x: u16, y: u16, visible: bool) {
        if self.held.borrow().is_none() {
            framebuffer::set_cursor(x, y, visible);
        }
        framebuffer::mark_dirty();
    }

    fn clamp(&self, width: u16, height: u16) -> (u16, u16) {
        framebuffer::clamp_dims(width, height)
    }

    fn resize(&mut self, _w0: u16, _h0: u16, w1: u16, h1: u16, shift: u16, _default_attr: u8) {
        // The console returns to the live view before resizing.
        framebuffer::resize(w1, h1, shift);
    }
}

/// Create a VTE that writes to the shared text framebuffer (`hal::FB`), and
/// the handle the console uses to hold the live screen while scrolled back.
pub fn new_physical(width: u16, height: u16) -> (Vte, Held) {
    let held: Held = Rc::new(RefCell::new(None));
    (Vte::with_screen(width, height, Box::new(Physical { held: held.clone() })), held)
}
