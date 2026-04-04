//! VT100/ANSI terminal emulator.
//!
//! Receives byte streams and writes cells to the framebuffer (physical mode)
//! or to a heap-allocated virtual buffer (for SSH sessions).
//!
//! Supported escape sequences:
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
use crate::framebuffer::{CELL_SIZE, DEFAULT_ATTR};

extern "C" {
    /// Hint to the host to read the framebuffer now (for low-latency sync).
    fn fb_sync();
}

/// Parser state machine states.
#[derive(Clone, Copy, PartialEq)]
enum ParseState {
    /// Normal text — print characters directly.
    Normal,
    /// Received ESC (0x1B), waiting for next byte.
    Escape,
    /// Inside a CSI sequence (ESC [), collecting params.
    Csi,
    /// Inside a CSI ? sequence (ESC [ ?), collecting params.
    CsiQuestion,
}

/// Maximum CSI parameters we track.
const MAX_PARAMS: usize = 8;

/// VT100 terminal emulator.
pub struct Vte {
    width: u16,
    height: u16,
    cursor_x: u16,
    cursor_y: u16,
    saved_cursor: (u16, u16),
    current_attr: u8,
    current_flags: u8,
    default_attr: u8,
    state: ParseState,
    params: [u16; MAX_PARAMS],
    param_count: usize,
    cursor_visible: bool,
    /// If true, writes to the physical framebuffer at FB_BASE.
    /// If false, writes to `virtual_buf`.
    physical: bool,
    /// Virtual cell buffer for non-physical mode (SSH sessions).
    virtual_buf: Option<Vec<u8>>,
    /// Scrollback buffer: saved rows that scrolled off the top.
    scrollback: Vec<Vec<u8>>,
    scrollback_max: usize,
}

impl Vte {
    /// Create a VTE that writes to the physical framebuffer at 0x20000.
    pub fn new_physical(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            cursor_x: 0,
            cursor_y: 0,
            saved_cursor: (0, 0),
            current_attr: DEFAULT_ATTR,
            current_flags: 0,
            default_attr: DEFAULT_ATTR,
            state: ParseState::Normal,
            params: [0; MAX_PARAMS],
            param_count: 0,
            cursor_visible: true,
            physical: true,
            virtual_buf: None,
            scrollback: Vec::new(),
            scrollback_max: 500,
        }
    }

    /// Create a VTE that writes to a heap-allocated virtual buffer.
    pub fn new_virtual(width: u16, height: u16) -> Self {
        let buf_size = width as usize * height as usize * CELL_SIZE;
        let mut buf = vec![0u8; buf_size];
        // Initialize with spaces + default attr
        for i in 0..(width as usize * height as usize) {
            let off = i * CELL_SIZE;
            buf[off] = b' ';
            buf[off + 1] = DEFAULT_ATTR;
            buf[off + 2] = 0;
            buf[off + 3] = 0;
        }
        Self {
            width,
            height,
            cursor_x: 0,
            cursor_y: 0,
            saved_cursor: (0, 0),
            current_attr: DEFAULT_ATTR,
            current_flags: 0,
            default_attr: DEFAULT_ATTR,
            state: ParseState::Normal,
            params: [0; MAX_PARAMS],
            param_count: 0,
            cursor_visible: true,
            physical: false,
            virtual_buf: Some(buf),
            scrollback: Vec::new(),
            scrollback_max: 500,
        }
    }

    /// Feed a byte slice through the terminal emulator.
    pub fn write(&mut self, data: &[u8]) {
        for &b in data {
            self.process_byte(b);
        }
        self.flush_cursor();
    }

    /// Feed a string through the terminal emulator.
    pub fn write_str(&mut self, s: &str) {
        self.write(s.as_bytes());
    }

    /// Get the current width.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Get the current height.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Get cursor position.
    pub fn cursor(&self) -> (u16, u16) {
        (self.cursor_x, self.cursor_y)
    }

    /// Resync cursor position from the framebuffer header.
    /// Called after a child process writes directly to the framebuffer,
    /// bypassing this VTE (e.g., via process_wait's drain loop).
    pub fn resync_cursor_from_header(&mut self) {
        if self.physical {
            let (x, y) = crate::framebuffer::cursor();
            self.cursor_x = x.min(self.width.saturating_sub(1));
            self.cursor_y = y.min(self.height.saturating_sub(1));
        }
    }

    /// Get cursor visibility.
    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    /// Access the virtual buffer (for SSH output extraction). Returns None for physical VTE.
    pub fn virtual_buf(&self) -> Option<&[u8]> {
        self.virtual_buf.as_deref()
    }

    // --- Cell I/O ---

    #[inline]
    fn put_cell(&mut self, x: u16, y: u16, ch: u8, attr: u8, flags: u8) {
        if self.physical {
            framebuffer::write_cell(x, y, ch, attr, flags);
        } else if let Some(ref mut buf) = self.virtual_buf {
            let off = (y as usize * self.width as usize + x as usize) * CELL_SIZE;
            if off + 3 < buf.len() {
                buf[off] = ch;
                buf[off + 1] = attr;
                buf[off + 2] = flags;
                buf[off + 3] = 0;
            }
        }
    }

    #[inline]
    fn get_cell(&self, x: u16, y: u16) -> (u8, u8, u8) {
        if self.physical {
            framebuffer::read_cell(x, y)
        } else if let Some(ref buf) = self.virtual_buf {
            let off = (y as usize * self.width as usize + x as usize) * CELL_SIZE;
            if off + 2 < buf.len() {
                (buf[off], buf[off + 1], buf[off + 2])
            } else {
                (b' ', self.default_attr, 0)
            }
        } else {
            (b' ', self.default_attr, 0)
        }
    }

    fn clear_row_range(&mut self, y: u16, start_x: u16, end_x: u16) {
        let end = end_x.min(self.width);
        for x in start_x..end {
            self.put_cell(x, y, b' ', self.current_attr, 0);
        }
    }

    fn clear_row(&mut self, y: u16) {
        self.clear_row_range(y, 0, self.width);
    }

    fn copy_row(&mut self, src_y: u16, dst_y: u16) {
        if self.physical {
            framebuffer::copy_row(src_y, dst_y);
        } else {
            let w = self.width as usize;
            let row_bytes = w * CELL_SIZE;
            let src_off = src_y as usize * row_bytes;
            let dst_off = dst_y as usize * row_bytes;
            if let Some(ref mut buf) = self.virtual_buf {
                if src_off + row_bytes <= buf.len() && dst_off + row_bytes <= buf.len() {
                    unsafe {
                        let ptr = buf.as_mut_ptr();
                        core::ptr::copy(ptr.add(src_off), ptr.add(dst_off), row_bytes);
                    }
                }
            }
        }
    }

    fn read_row_bytes(&self, y: u16) -> Vec<u8> {
        if self.physical {
            framebuffer::read_row(y)
        } else if let Some(ref buf) = self.virtual_buf {
            let w = self.width as usize;
            let row_bytes = w * CELL_SIZE;
            let off = y as usize * row_bytes;
            if off + row_bytes <= buf.len() {
                buf[off..off + row_bytes].to_vec()
            } else {
                vec![0u8; row_bytes]
            }
        } else {
            vec![0u8; self.width as usize * CELL_SIZE]
        }
    }

    // --- Cursor management ---

    fn flush_cursor(&mut self) {
        if self.physical {
            framebuffer::set_cursor(self.cursor_x, self.cursor_y, self.cursor_visible);
            framebuffer::mark_dirty();
        }
    }

    // --- Scrolling ---

    fn scroll_up(&mut self, n: u16) {
        let n = n.min(self.height);
        // Save scrolled-off rows to scrollback
        for i in 0..n {
            let row = self.read_row_bytes(i);
            self.scrollback.push(row);
            if self.scrollback.len() > self.scrollback_max {
                self.scrollback.remove(0);
            }
        }
        // Shift rows up
        for y in n..self.height {
            self.copy_row(y, y - n);
        }
        // Clear bottom rows
        for y in (self.height - n)..self.height {
            self.clear_row(y);
        }
    }

    fn scroll_down(&mut self, n: u16) {
        let n = n.min(self.height);
        // Shift rows down (bottom to top to avoid overwrite)
        for y in (0..self.height - n).rev() {
            self.copy_row(y, y + n);
        }
        // Clear top rows
        for y in 0..n {
            self.clear_row(y);
        }
    }

    // --- Byte processing ---

    fn process_byte(&mut self, b: u8) {
        match self.state {
            ParseState::Normal => self.process_normal(b),
            ParseState::Escape => self.process_escape(b),
            ParseState::Csi => self.process_csi(b),
            ParseState::CsiQuestion => self.process_csi_question(b),
        }
    }

    fn process_normal(&mut self, b: u8) {
        match b {
            0x1B => {
                // ESC
                self.state = ParseState::Escape;
            }
            b'\n' => {
                // Line feed — move down, scroll if at bottom
                self.cursor_x = 0;
                if self.cursor_y + 1 >= self.height {
                    self.scroll_up(1);
                } else {
                    self.cursor_y += 1;
                }
                // Line-buffered sync: tell host to read framebuffer on each newline.
                // This ensures output is visible during long-running WASM calls
                // (e.g., Python infinite loops with print) where the worker thread
                // can't poll the dirty counter.
                if self.physical {
                    unsafe { fb_sync(); }
                }
            }
            b'\r' => {
                // Carriage return
                self.cursor_x = 0;
            }
            0x08 => {
                // Backspace
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                    // Erase the character at the new cursor position
                    self.put_cell(self.cursor_x, self.cursor_y, b' ', self.current_attr, 0);
                }
            }
            b'\t' => {
                // Tab — advance to next multiple of 8
                let next = (self.cursor_x + 8) & !7;
                let next = next.min(self.width - 1);
                self.cursor_x = next;
            }
            0x07 => {
                // BEL — ignore (could trigger a sound)
            }
            0x00..=0x1F => {
                // Other control characters — ignore
            }
            _ => {
                // Printable character
                self.put_char(b);
            }
        }
    }

    fn put_char(&mut self, ch: u8) {
        if self.cursor_x >= self.width {
            // Line wrap
            self.cursor_x = 0;
            if self.cursor_y + 1 >= self.height {
                self.scroll_up(1);
            } else {
                self.cursor_y += 1;
            }
        }
        self.put_cell(self.cursor_x, self.cursor_y, ch, self.current_attr, self.current_flags);
        self.cursor_x += 1;
    }

    fn process_escape(&mut self, b: u8) {
        match b {
            b'[' => {
                // CSI introducer
                self.state = ParseState::Csi;
                self.params = [0; MAX_PARAMS];
                self.param_count = 0;
            }
            b'c' => {
                // RIS — full reset
                self.reset();
                self.state = ParseState::Normal;
            }
            b'D' => {
                // IND — index (move cursor down, scroll if needed)
                if self.cursor_y + 1 >= self.height {
                    self.scroll_up(1);
                } else {
                    self.cursor_y += 1;
                }
                self.state = ParseState::Normal;
            }
            b'M' => {
                // RI — reverse index (move cursor up, scroll if needed)
                if self.cursor_y == 0 {
                    self.scroll_down(1);
                } else {
                    self.cursor_y -= 1;
                }
                self.state = ParseState::Normal;
            }
            b'7' => {
                // DECSC — save cursor
                self.saved_cursor = (self.cursor_x, self.cursor_y);
                self.state = ParseState::Normal;
            }
            b'8' => {
                // DECRC — restore cursor
                self.cursor_x = self.saved_cursor.0;
                self.cursor_y = self.saved_cursor.1;
                self.state = ParseState::Normal;
            }
            _ => {
                // Unknown escape — discard and return to normal
                self.state = ParseState::Normal;
            }
        }
    }

    fn process_csi(&mut self, b: u8) {
        match b {
            b'?' => {
                self.state = ParseState::CsiQuestion;
            }
            b'0'..=b'9' => {
                // Accumulate numeric parameter
                if self.param_count == 0 {
                    self.param_count = 1;
                }
                let idx = self.param_count - 1;
                if idx < MAX_PARAMS {
                    self.params[idx] = self.params[idx].saturating_mul(10).saturating_add((b - b'0') as u16);
                }
            }
            b';' => {
                // Parameter separator
                if self.param_count < MAX_PARAMS {
                    self.param_count += 1;
                }
            }
            b'A'..=b'z' => {
                // Final byte — dispatch
                self.handle_csi(b);
                self.state = ParseState::Normal;
            }
            _ => {
                // Unknown — abort CSI
                self.state = ParseState::Normal;
            }
        }
    }

    fn process_csi_question(&mut self, b: u8) {
        match b {
            b'0'..=b'9' => {
                if self.param_count == 0 {
                    self.param_count = 1;
                }
                let idx = self.param_count - 1;
                if idx < MAX_PARAMS {
                    self.params[idx] = self.params[idx].saturating_mul(10).saturating_add((b - b'0') as u16);
                }
            }
            b';' => {
                if self.param_count < MAX_PARAMS {
                    self.param_count += 1;
                }
            }
            b'h' => {
                // CSI ? ... h — set mode
                let param = if self.param_count > 0 { self.params[0] } else { 0 };
                if param == 25 {
                    self.cursor_visible = true;
                }
                self.state = ParseState::Normal;
            }
            b'l' => {
                // CSI ? ... l — reset mode
                let param = if self.param_count > 0 { self.params[0] } else { 0 };
                if param == 25 {
                    self.cursor_visible = false;
                }
                self.state = ParseState::Normal;
            }
            _ => {
                self.state = ParseState::Normal;
            }
        }
    }

    fn param(&self, idx: usize, default: u16) -> u16 {
        if idx < self.param_count && self.params[idx] > 0 {
            self.params[idx]
        } else {
            default
        }
    }

    fn handle_csi(&mut self, final_byte: u8) {
        match final_byte {
            b'A' => {
                // Cursor Up
                let n = self.param(0, 1);
                self.cursor_y = self.cursor_y.saturating_sub(n);
            }
            b'B' => {
                // Cursor Down
                let n = self.param(0, 1);
                self.cursor_y = (self.cursor_y + n).min(self.height - 1);
            }
            b'C' => {
                // Cursor Forward
                let n = self.param(0, 1);
                self.cursor_x = (self.cursor_x + n).min(self.width - 1);
            }
            b'D' => {
                // Cursor Back
                let n = self.param(0, 1);
                self.cursor_x = self.cursor_x.saturating_sub(n);
            }
            b'E' => {
                // Cursor Next Line
                let n = self.param(0, 1);
                self.cursor_y = (self.cursor_y + n).min(self.height - 1);
                self.cursor_x = 0;
            }
            b'F' => {
                // Cursor Previous Line
                let n = self.param(0, 1);
                self.cursor_y = self.cursor_y.saturating_sub(n);
                self.cursor_x = 0;
            }
            b'G' => {
                // Cursor Horizontal Absolute
                let n = self.param(0, 1);
                self.cursor_x = (n - 1).min(self.width - 1);
            }
            b'H' | b'f' => {
                // Cursor Position (CUP) — CSI row ; col H
                let row = self.param(0, 1);
                let col = self.param(1, 1);
                self.cursor_y = (row - 1).min(self.height - 1);
                self.cursor_x = (col - 1).min(self.width - 1);
            }
            b'J' => {
                // Erase in Display
                let mode = self.param(0, 0);
                match mode {
                    0 => {
                        // Clear from cursor to end
                        self.clear_row_range(self.cursor_y, self.cursor_x, self.width);
                        for y in (self.cursor_y + 1)..self.height {
                            self.clear_row(y);
                        }
                    }
                    1 => {
                        // Clear from start to cursor
                        for y in 0..self.cursor_y {
                            self.clear_row(y);
                        }
                        self.clear_row_range(self.cursor_y, 0, self.cursor_x + 1);
                    }
                    2 | 3 => {
                        // Clear entire screen
                        for y in 0..self.height {
                            self.clear_row(y);
                        }
                    }
                    _ => {}
                }
            }
            b'K' => {
                // Erase in Line
                let mode = self.param(0, 0);
                match mode {
                    0 => {
                        // Clear from cursor to end of line
                        self.clear_row_range(self.cursor_y, self.cursor_x, self.width);
                    }
                    1 => {
                        // Clear from start of line to cursor
                        self.clear_row_range(self.cursor_y, 0, self.cursor_x + 1);
                    }
                    2 => {
                        // Clear entire line
                        self.clear_row(self.cursor_y);
                    }
                    _ => {}
                }
            }
            b'S' => {
                // Scroll Up
                let n = self.param(0, 1);
                self.scroll_up(n);
            }
            b'T' => {
                // Scroll Down
                let n = self.param(0, 1);
                self.scroll_down(n);
            }
            b'm' => {
                // SGR — Select Graphic Rendition
                self.handle_sgr();
            }
            b's' => {
                // Save cursor position
                self.saved_cursor = (self.cursor_x, self.cursor_y);
            }
            b'u' => {
                // Restore cursor position
                self.cursor_x = self.saved_cursor.0.min(self.width - 1);
                self.cursor_y = self.saved_cursor.1.min(self.height - 1);
            }
            b'n' => {
                // Device Status Report — we ignore this (would need to send response)
            }
            _ => {
                // Unknown CSI — ignore
            }
        }
    }

    fn handle_sgr(&mut self) {
        if self.param_count == 0 {
            // ESC[m with no params = reset
            self.current_attr = self.default_attr;
            self.current_flags = 0;
            return;
        }

        let mut i = 0;
        while i < self.param_count {
            let p = self.params[i];
            match p {
                0 => {
                    // Reset
                    self.current_attr = self.default_attr;
                    self.current_flags = 0;
                }
                1 => {
                    // Bold
                    self.current_flags |= 0x01;
                }
                4 => {
                    // Underline
                    self.current_flags |= 0x02;
                }
                5 => {
                    // Blink
                    self.current_flags |= 0x04;
                }
                7 => {
                    // Inverse
                    self.current_flags |= 0x08;
                }
                22 => {
                    // Normal intensity (unbold)
                    self.current_flags &= !0x01;
                }
                24 => {
                    // No underline
                    self.current_flags &= !0x02;
                }
                25 => {
                    // No blink
                    self.current_flags &= !0x04;
                }
                27 => {
                    // No inverse
                    self.current_flags &= !0x08;
                }
                // Standard foreground colors (30-37)
                30..=37 => {
                    let color = (p - 30) as u8;
                    self.current_attr = (self.current_attr & 0xF0) | color;
                }
                // Default foreground
                39 => {
                    let default_fg = self.default_attr & 0x0F;
                    self.current_attr = (self.current_attr & 0xF0) | default_fg;
                }
                // Standard background colors (40-47)
                40..=47 => {
                    let color = (p - 40) as u8;
                    self.current_attr = (self.current_attr & 0x0F) | (color << 4);
                }
                // Default background
                49 => {
                    let default_bg = self.default_attr & 0xF0;
                    self.current_attr = (self.current_attr & 0x0F) | default_bg;
                }
                // Bright foreground colors (90-97)
                90..=97 => {
                    let color = (p - 90 + 8) as u8;
                    self.current_attr = (self.current_attr & 0xF0) | color;
                }
                // Bright background colors (100-107)
                100..=107 => {
                    let color = (p - 100 + 8) as u8;
                    self.current_attr = (self.current_attr & 0x0F) | (color << 4);
                }
                _ => {
                    // Unknown SGR param — ignore
                }
            }
            i += 1;
        }
    }

    /// Full reset — clear screen, reset attributes, cursor to (0,0).
    pub fn reset(&mut self) {
        self.current_attr = self.default_attr;
        self.current_flags = 0;
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.cursor_visible = true;
        self.saved_cursor = (0, 0);
        self.state = ParseState::Normal;
        self.params = [0; MAX_PARAMS];
        self.param_count = 0;
        for y in 0..self.height {
            self.clear_row(y);
        }
        self.flush_cursor();
    }
}
