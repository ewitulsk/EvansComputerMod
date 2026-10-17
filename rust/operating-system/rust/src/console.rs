//! Terminals. `Term` is what the shell, line editor, job control and switch
//! CLI write to; `Console` is the physical one (the VTE rendering into the
//! shared framebuffer). Remote SSH sessions implement `Term` with a byte
//! buffer (see `sessions.rs`).

use crate::framebuffer;
use crate::vte::{self, Held, Vte};
pub use ecm_vte::Modes;

pub trait Term {
    /// Raw bytes (may contain ANSI escapes).
    fn write(&mut self, data: &[u8]);
    fn width(&self) -> u16;
    fn height(&self) -> u16;
    fn print(&mut self, s: &str) {
        self.write(s.as_bytes());
    }
    fn println(&mut self, s: &str) {
        self.write(s.as_bytes());
        self.write(b"\n");
    }
    fn clear(&mut self) {
        self.write(b"\x1b[2J\x1b[H");
    }
}

impl Term for Console {
    fn write(&mut self, data: &[u8]) {
        self.vte.write(data);
        if self.view_top.is_some() {
            // New output: the view stays on the same lines.
            self.draw_view();
        }
    }
    fn width(&self) -> u16 {
        self.vte.width()
    }
    fn height(&self) -> u16 {
        self.vte.height()
    }
}

pub struct Console {
    vte: Vte,
    held: Held,
    /// The absolute line at the top of a scrolled-back view (None: live).
    view_top: Option<u64>,
}

/// A scrolling request from the keyboard or the mouse wheel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroll {
    Lines(i64),
    Pages(i64),
    Top,
    Bottom,
}

impl Console {
    pub fn new() -> Self {
        framebuffer::init(framebuffer::DEFAULT_WIDTH, framebuffer::DEFAULT_HEIGHT);
        let (vte, held) = vte::new_physical(framebuffer::width(), framebuffer::height());
        Self { vte, held, view_top: None }
    }

    /// Resize the text grid (the host installed a different display panel).
    /// Returns the size actually set (clamped to the framebuffer region).
    pub fn resize(&mut self, width: u16, height: u16) -> (u16, u16) {
        self.live();
        self.vte.resize(width, height)
    }

    pub fn cursor(&self) -> (u16, u16) {
        self.vte.cursor()
    }

    /// The modes the running program switched (alternate screen, bracketed
    /// paste, its own Ctrl+C).
    pub fn modes(&self) -> Modes {
        self.vte.modes()
    }

    /// A program ended: forget the modes it left on.
    pub fn reset_modes(&mut self) {
        self.vte.reset_modes();
    }

    /// Replies to the program's terminal queries (cursor position, size).
    pub fn take_responses(&mut self) -> Vec<u8> {
        self.vte.take_responses()
    }

    /// Is the scrollback showing?
    pub fn scrolled(&self) -> bool {
        self.view_top.is_some()
    }

    fn first_line(&self) -> u64 {
        self.vte.scrolled_total() - self.vte.scrollback().len() as u64
    }

    /// Scroll the view back (negative) or forward through the scrollback.
    pub fn scroll(&mut self, by: Scroll) {
        let live_top = self.vte.scrolled_total();
        let first = self.first_line();
        let page = self.vte.height().saturating_sub(1).max(1) as i64;
        let top = self.view_top.unwrap_or(live_top) as i64;
        let new = match by {
            Scroll::Lines(n) => top + n,
            Scroll::Pages(n) => top + n * page,
            Scroll::Top => first as i64,
            Scroll::Bottom => live_top as i64,
        }
        .clamp(first as i64, live_top as i64) as u64;
        if new >= live_top {
            self.live();
            return;
        }
        if self.view_top.is_none() {
            // Hold the live screen; the VTE keeps drawing into it.
            let w = framebuffer::width() as usize;
            let h = framebuffer::height();
            let mut cells = Vec::with_capacity(w * h as usize * vte::CELL_SIZE);
            for y in 0..h {
                cells.extend_from_slice(&framebuffer::read_row(y));
            }
            *self.held.borrow_mut() = Some(cells);
        }
        self.view_top = Some(new);
        self.draw_view();
    }

    /// Back to the live screen.
    pub fn live(&mut self) {
        if self.view_top.take().is_none() {
            return;
        }
        if let Some(cells) = self.held.borrow_mut().take() {
            let row = framebuffer::width() as usize * vte::CELL_SIZE;
            for (y, r) in cells.chunks(row.max(1)).enumerate() {
                framebuffer::write_row(y as u16, r);
            }
        }
        let (x, y) = self.vte.cursor();
        framebuffer::set_cursor(x.min(self.vte.width() - 1), y, self.vte.cursor_visible());
        framebuffer::mark_dirty();
    }

    /// Draw the scrolled-back view: scrollback rows, then the held screen's,
    /// and how far back it is in the top-right corner.
    fn draw_view(&mut self) {
        let Some(top) = self.view_top else { return };
        let live_top = self.vte.scrolled_total();
        let first = self.first_line();
        let top = top.max(first);
        self.view_top = Some(top);
        let (w, h) = (self.vte.width(), self.vte.height());
        let def = self.vte.default_attr();
        for y in 0..h {
            let l = top + y as u64;
            for x in 0..w {
                let (c, a, f) = if l < live_top {
                    let row = &self.vte.scrollback()[(l - first) as usize];
                    let o = x as usize * vte::CELL_SIZE;
                    row.get(o..o + 3).map(|s| (s[0], s[1], s[2])).unwrap_or((b' ', def, 0))
                } else {
                    self.vte.cell(x, (l - live_top) as u16)
                };
                framebuffer::write_cell(x, y, c, a, f);
            }
        }
        let tag = format!(" -{} ", live_top - top);
        let x0 = w.saturating_sub(tag.len() as u16);
        for (i, ch) in tag.bytes().enumerate() {
            framebuffer::write_cell(x0 + i as u16, 0, ch, def, 0x08);
        }
        framebuffer::set_cursor(0, 0, false);
        framebuffer::mark_dirty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vte_survives_hostile_child_output() {
        let _g = crate::hal::test_lock();
        let mut c = Console::new();
        let mut x: u64 = 0x1234_5678_9abc_def0;
        let mut buf = Vec::with_capacity(4096);
        for _ in 0..200 {
            buf.clear();
            for _ in 0..4096 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                // Bias towards escape-sequence bytes.
                let b = match x % 8 {
                    0 => 0x1b,
                    1 => b'[',
                    2 => b'0' + (x >> 8) as u8 % 10,
                    3 => b';',
                    _ => (x >> 16) as u8,
                };
                buf.push(b);
            }
            c.write(&buf);
        }
        // Resizes in the middle of hostile output stay in bounds.
        c.resize(80, 25);
        c.write(&buf);
        c.resize(1, 1);
        c.write(b"abc\n\x1b[5;5H");
        c.resize(u16::MAX, u16::MAX);
        c.write(&buf);
        c.resize(160, 50);
        c.write(b"\x1b[65535;65535H\x1b[65535A\x1b[65535S\x1b[65535T\x1b[?1049h\x1b[?1049l");
    }
}
