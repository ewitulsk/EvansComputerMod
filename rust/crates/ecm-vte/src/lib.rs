//! `ecm-vte`: the VT100/ANSI terminal emulator shared by the kernel's console
//! (`terminal-os`, cells in the shared text framebuffer) and the GUI Terminal
//! app (cells in a [`VirtualScreen`]).
//!
//! Receives byte streams and writes cells to a [`Screen`].
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

/// Bytes per cell: char, attribute, flags, reserved.
pub const CELL_SIZE: usize = 4;
/// Bright green on black (low nibble foreground, high nibble background).
pub const DEFAULT_ATTR: u8 = 0x0A;

/// Re-lay the cells of a `w0 x h0` grid into a `w1 x h1` one: new row `y`
/// takes old row `y + shift`, each row keeps its first `min(w0, w1)` cells,
/// and everything else is blank in `attr`.
pub fn relayout(old: &[u8], w0: u16, h0: u16, w1: u16, h1: u16, shift: u16, attr: u8) -> Vec<u8> {
    let (w0, h0, w1, h1) = (w0 as usize, h0 as usize, w1 as usize, h1 as usize);
    let mut out = Vec::with_capacity(w1 * h1 * CELL_SIZE);
    for _ in 0..w1 * h1 {
        out.extend_from_slice(&[b' ', attr, 0, 0]);
    }
    let keep = w0.min(w1) * CELL_SIZE;
    for y in 0..h1 {
        let src_y = y + shift as usize;
        if src_y >= h0 {
            break;
        }
        let src = src_y * w0 * CELL_SIZE;
        let dst = y * w1 * CELL_SIZE;
        if let Some(s) = old.get(src..src + keep) {
            out[dst..dst + keep].copy_from_slice(s);
        }
    }
    out
}

/// Where a [`Vte`] keeps its cells: the kernel's shared text framebuffer
/// (the console) or a heap buffer ([`VirtualScreen`], the Terminal app and
/// remote sessions). Cells are `CELL_SIZE` bytes: char, attr, flags, 0.
pub trait Screen {
    fn put_cell(&mut self, x: u16, y: u16, ch: u8, attr: u8, flags: u8);
    /// `(char, attr, flags)`; out of range reads are blanks in `default_attr`.
    fn get_cell(&self, x: u16, y: u16, default_attr: u8) -> (u8, u8, u8);
    fn copy_row(&mut self, src_y: u16, dst_y: u16);
    /// One row's raw cells (zeros when out of range).
    fn read_row(&self, y: u16) -> Vec<u8>;
    /// The cursor moved (called after every write).
    fn flush_cursor(&mut self, _x: u16, _y: u16, _visible: bool) {}
    /// The largest grid this screen can hold for the asked size.
    fn clamp(&self, width: u16, height: u16) -> (u16, u16) {
        (width.max(1), height.max(1))
    }
    /// Change the grid from `w0 x h0` to `w1 x h1` (already clamped), keeping
    /// the text: new row `y` shows old row `y + shift`.
    fn resize(&mut self, w0: u16, h0: u16, w1: u16, h1: u16, shift: u16, default_attr: u8);
    /// The whole cell buffer, if the screen keeps one in memory.
    fn cells(&self) -> Option<&[u8]> {
        None
    }
}

/// A heap-allocated cell grid.
pub struct VirtualScreen {
    width: u16,
    buf: Vec<u8>,
}

impl VirtualScreen {
    pub fn new(width: u16, height: u16, attr: u8) -> VirtualScreen {
        let buf_size = width as usize * height as usize * CELL_SIZE;
        let mut buf = vec![0u8; buf_size];
        // Initialize with spaces + default attr
        for i in 0..(width as usize * height as usize) {
            let off = i * CELL_SIZE;
            buf[off] = b' ';
            buf[off + 1] = attr;
            buf[off + 2] = 0;
            buf[off + 3] = 0;
        }
        VirtualScreen { width, buf }
    }
}

impl Screen for VirtualScreen {
    fn put_cell(&mut self, x: u16, y: u16, ch: u8, attr: u8, flags: u8) {
        let off = (y as usize * self.width as usize + x as usize) * CELL_SIZE;
        if off + 3 < self.buf.len() {
            self.buf[off] = ch;
            self.buf[off + 1] = attr;
            self.buf[off + 2] = flags;
            self.buf[off + 3] = 0;
        }
    }

    fn get_cell(&self, x: u16, y: u16, default_attr: u8) -> (u8, u8, u8) {
        let off = (y as usize * self.width as usize + x as usize) * CELL_SIZE;
        if off + 2 < self.buf.len() {
            (self.buf[off], self.buf[off + 1], self.buf[off + 2])
        } else {
            (b' ', default_attr, 0)
        }
    }

    fn copy_row(&mut self, src_y: u16, dst_y: u16) {
        let w = self.width as usize;
        let row_bytes = w * CELL_SIZE;
        let src_off = src_y as usize * row_bytes;
        let dst_off = dst_y as usize * row_bytes;
        if src_off + row_bytes <= self.buf.len() && dst_off + row_bytes <= self.buf.len() {
            self.buf.copy_within(src_off..src_off + row_bytes, dst_off);
        }
    }

    fn read_row(&self, y: u16) -> Vec<u8> {
        let w = self.width as usize;
        let row_bytes = w * CELL_SIZE;
        let off = y as usize * row_bytes;
        if off + row_bytes <= self.buf.len() {
            self.buf[off..off + row_bytes].to_vec()
        } else {
            vec![0u8; row_bytes]
        }
    }

    fn resize(&mut self, w0: u16, h0: u16, w1: u16, h1: u16, shift: u16, default_attr: u8) {
        self.buf = relayout(&self.buf, w0, h0, w1, h1, shift, default_attr);
        self.width = w1;
    }

    fn cells(&self) -> Option<&[u8]> {
        Some(&self.buf)
    }
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
    /// Where the cells live.
    screen: Box<dyn Screen>,
    /// Scrollback buffer: saved rows that scrolled off the top.
    scrollback: Vec<Vec<u8>>,
    scrollback_max: usize,
    /// Saved main screen cells for alternate screen buffer (\x1b[?1049h/l).
    saved_main_screen: Option<Vec<u8>>,
    /// Saved main cursor position when entering alternate screen.
    saved_main_cursor: (u16, u16),
    /// Saved main attribute when entering alternate screen.
    saved_main_attr: u8,
    /// Rows pushed into the scrollback so far (never decreases).
    scrolled_total: u64,
}

impl Vte {
    /// A VTE drawing into `screen` (which already holds a `width x height` grid).
    pub fn with_screen(width: u16, height: u16, screen: Box<dyn Screen>) -> Self {
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
            screen,
            scrollback: Vec::new(),
            scrollback_max: 500,
            saved_main_screen: None,
            saved_main_cursor: (0, 0),
            saved_main_attr: DEFAULT_ATTR,
            scrolled_total: 0,
        }
    }

    /// Create a VTE that writes to a heap-allocated virtual buffer.
    pub fn new_virtual(width: u16, height: u16) -> Self {
        Self::with_screen(width, height, Box::new(VirtualScreen::new(width, height, DEFAULT_ATTR)))
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

    /// Get cursor visibility.
    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    /// Access the virtual buffer (for SSH output extraction). Returns None for physical VTE.
    pub fn virtual_buf(&self) -> Option<&[u8]> {
        self.screen.cells()
    }

    /// The cell at `(x, y)` of the visible grid: `(char, attr, flags)`.
    pub fn cell(&self, x: u16, y: u16) -> (u8, u8, u8) {
        self.get_cell(x, y)
    }

    /// Rows that scrolled off the top, oldest first (raw cells, `CELL_SIZE` each).
    pub fn scrollback(&self) -> &[Vec<u8>] {
        &self.scrollback
    }

    /// How many rows the scrollback keeps (500 by default).
    pub fn set_scrollback_max(&mut self, n: usize) {
        self.scrollback_max = n;
        while self.scrollback.len() > self.scrollback_max {
            self.scrollback.remove(0);
        }
    }

    /// Rows pushed into the scrollback since the start (to keep a scrolled-back view in place).
    pub fn scrolled_total(&self) -> u64 {
        self.scrolled_total
    }

    /// Is the alternate screen (full-screen programs) shown?
    pub fn alternate_screen(&self) -> bool {
        self.saved_main_screen.is_some()
    }

    /// The default attribute (fg low nibble, bg high nibble).
    pub fn default_attr(&self) -> u8 {
        self.default_attr
    }

    // --- Cell I/O ---

    #[inline]
    fn put_cell(&mut self, x: u16, y: u16, ch: u8, attr: u8, flags: u8) {
        self.screen.put_cell(x, y, ch, attr, flags);
    }

    #[inline]
    fn get_cell(&self, x: u16, y: u16) -> (u8, u8, u8) {
        self.screen.get_cell(x, y, self.default_attr)
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
        self.screen.copy_row(src_y, dst_y);
    }

    fn read_row_bytes(&self, y: u16) -> Vec<u8> {
        self.screen.read_row(y)
    }

    // --- Cursor management ---

    fn flush_cursor(&mut self) {
        self.screen.flush_cursor(self.cursor_x, self.cursor_y, self.cursor_visible);
    }

    // --- Scrolling ---

    fn scroll_up(&mut self, n: u16) {
        let n = n.min(self.height);
        // Save scrolled-off rows to scrollback
        for i in 0..n {
            let row = self.read_row_bytes(i);
            self.scrollback.push(row);
            self.scrolled_total += 1;
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

    // --- Alternate screen buffer ---

    /// Save current screen and switch to alternate buffer (ESC[?1049h).
    fn enter_alternate_screen(&mut self) {
        let total = self.width as usize * self.height as usize;
        let mut saved = Vec::with_capacity(total * CELL_SIZE);
        for y in 0..self.height {
            for x in 0..self.width {
                let (ch, attr, flags) = self.get_cell(x, y);
                saved.push(ch);
                saved.push(attr);
                saved.push(flags);
                saved.push(0);
            }
        }
        self.saved_main_screen = Some(saved);
        self.saved_main_cursor = (self.cursor_x, self.cursor_y);
        self.saved_main_attr = self.current_attr;
        // Clear screen for alternate buffer
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.current_attr = self.default_attr;
        self.current_flags = 0;
        for y in 0..self.height {
            self.clear_row(y);
        }
    }

    /// Restore main screen from saved buffer (ESC[?1049l).
    fn leave_alternate_screen(&mut self) {
        if let Some(saved) = self.saved_main_screen.take() {
            let total = self.width as usize * self.height as usize;
            for i in 0..total {
                let off = i * CELL_SIZE;
                if off + 3 < saved.len() {
                    let x = (i % self.width as usize) as u16;
                    let y = (i / self.width as usize) as u16;
                    self.put_cell(x, y, saved[off], saved[off + 1], saved[off + 2]);
                }
            }
            self.cursor_x = self.saved_main_cursor.0;
            self.cursor_y = self.saved_main_cursor.1;
            self.current_attr = self.saved_main_attr;
            self.current_flags = 0;
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
                match param {
                    25 => self.cursor_visible = true,
                    1049 => self.enter_alternate_screen(),
                    _ => {}
                }
                self.state = ParseState::Normal;
            }
            b'l' => {
                // CSI ? ... l — reset mode
                let param = if self.param_count > 0 { self.params[0] } else { 0 };
                match param {
                    25 => self.cursor_visible = false,
                    1049 => self.leave_alternate_screen(),
                    _ => {}
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

    /// Change the grid to `width x height` (a display panel was swapped).
    /// Text is kept: rows are cut or padded on the right, and if the cursor
    /// would fall off the bottom the top rows scroll away so the cursor's row
    /// stays the last one. The physical VTE also resizes the shared
    /// framebuffer (clamped to its region). Returns the size actually set.
    pub fn resize(&mut self, width: u16, height: u16) -> (u16, u16) {
        let (w0, h0) = (self.width, self.height);
        let (w1, h1) = self.screen.clamp(width, height);
        let shift = if self.cursor_y >= h1 { self.cursor_y + 1 - h1 } else { 0 };
        self.screen.resize(w0, h0, w1, h1, shift, self.default_attr);
        if let Some(saved) = self.saved_main_screen.take() {
            let s = self.saved_main_cursor.1;
            let sshift = if s >= h1 { s + 1 - h1 } else { 0 };
            self.saved_main_screen = Some(relayout(&saved, w0, h0, w1, h1, sshift, self.default_attr));
            self.saved_main_cursor = (self.saved_main_cursor.0.min(w1 - 1), s - sshift);
        }
        self.width = w1;
        self.height = h1;
        self.cursor_y -= shift;
        self.cursor_x = self.cursor_x.min(w1 - 1);
        self.saved_cursor = (self.saved_cursor.0.min(w1 - 1), self.saved_cursor.1.min(h1 - 1));
        self.flush_cursor();
        (w1, h1)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(v: &Vte, y: u16) -> String {
        (0..v.width()).map(|x| v.cell(x, y).0 as char).collect::<String>().trim_end().to_string()
    }

    #[test]
    fn text_newlines_and_scrollback() {
        let mut v = Vte::new_virtual(10, 3);
        v.write(b"one\r\ntwo\r\nthree\r\nfour");
        assert_eq!(row(&v, 0), "two");
        assert_eq!(row(&v, 2), "four");
        assert_eq!(v.scrollback().len(), 1);
        assert_eq!(v.scrolled_total(), 1);
        assert_eq!(v.cursor(), (4, 2));
    }

    #[test]
    fn sgr_colours_cursor_moves_and_erase() {
        let mut v = Vte::new_virtual(20, 4);
        v.write(b"\x1b[31;44mred\x1b[0m ok");
        assert_eq!(v.cell(0, 0), (b'r', 0x41, 0));
        assert_eq!(v.cell(4, 0).1, DEFAULT_ATTR);
        v.write(b"\x1b[2;5HX\x1b[1;1H\x1b[K");
        assert_eq!(v.cell(4, 1).0, b'X');
        assert_eq!(row(&v, 0), "");
        v.write(b"\x1b[7mi\x1b[27m");
        assert_eq!(v.cell(0, 0).2 & 0x08, 0x08);
    }

    #[test]
    fn alternate_screen_restores_and_resize_keeps_text() {
        let mut v = Vte::new_virtual(10, 3);
        v.write(b"main");
        v.write(b"\x1b[?1049hfull");
        assert!(v.alternate_screen());
        assert_eq!(row(&v, 0), "full");
        v.write(b"\x1b[?1049l");
        assert_eq!(row(&v, 0), "main");
        assert_eq!(v.resize(4, 2), (4, 2));
        assert_eq!(row(&v, 0), "main");
        assert_eq!(v.virtual_buf().unwrap().len(), 4 * 2 * CELL_SIZE);
    }
}
