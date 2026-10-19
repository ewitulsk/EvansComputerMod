//! `ecm-vte`: the VT100/ANSI terminal emulator shared by the kernel's console
//! (`terminal-os`, cells in the shared text framebuffer) and the GUI Terminal
//! app (cells in a [`VirtualScreen`]).
//!
//! Receives byte streams and writes cells to a [`Screen`].
//!
//! Supported:
//! - Cursor: CSI A/B/C/D/E/F/G/H/f/d/`/a/e, CSI s/u, ESC 7/8, CSI I/Z (tabs)
//! - Erase and edit: CSI J (0-3), K, X, @ (insert chars), P (delete chars),
//!   L/M (insert/delete lines), b (repeat)
//! - Scrolling: CSI S/T, CSI r (scrolling region), ESC D/E/M
//! - SGR: bold, underline, blink, inverse, the 16 colours, and 256-colour /
//!   truecolor (`38;5;n`, `38;2;r;g;b`) drawn as the nearest of the 16
//! - Modes (CSI ? h/l, several at once): 25 cursor, 1049/47/1047 alternate
//!   screen (no scrollback there), 2004 bracketed paste, 1 cursor keys, and
//!   [`MODE_OWN_INTERRUPT`]; read them with [`Vte::modes`] or a [`ModeScanner`]
//! - Queries: CSI 6n / 5n / 18t, answered through [`Vte::take_responses`],
//!   and `?2048` in-band resize reports ([`MODE_RESIZE_REPORTS`])
//! - OSC/DCS/APC strings (window titles) and charset selection are skipped
//! - UTF-8: one cell per character ([`cell_byte`] picks its ASCII look-alike)
//! - \n, \r, \b, \t and the other C0 controls

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
    /// Inside an OSC / DCS / APC / PM string (ESC ] ... BEL or ESC \), skipped.
    Str,
    /// ESC seen inside a string: `\` ends it.
    StrEsc,
    /// ESC ( / ESC ) and friends: one more byte (the character set) follows.
    Charset,
}

/// Maximum CSI parameters we track.
const MAX_PARAMS: usize = 16;

/// DEC private mode a program sets (`ESC[?7727h`) while it wants Ctrl+C as a
/// key instead of being stopped by it (the REPLs, while reading a line).
/// Full-screen programs (the alternate screen) get Ctrl+C as a key too.
pub const MODE_OWN_INTERRUPT: u16 = 7727;

/// Modes a program can switch that the terminal's owner (the kernel) acts on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    /// `?1049`: the alternate screen (a full-screen program).
    pub alternate: bool,
    /// `?2004`: bracketed paste (pastes arrive between `ESC[200~` and `ESC[201~`).
    pub bracketed_paste: bool,
    /// `?7727`: the program reads Ctrl+C itself ([`MODE_OWN_INTERRUPT`]).
    pub own_interrupt: bool,
    /// `?1`: application cursor keys (arrows as `ESC O A`).
    pub app_cursor: bool,
    /// `?2048`: in-band resize reports. The terminal answers `ESC[48;rows;cols;0;0t`
    /// when the mode is set and whenever its size changes (vim follows the
    /// window size this way: there is no SIGWINCH).
    pub resize_reports: bool,
}

impl Modes {
    /// Does Ctrl+C go to the program as a key (instead of stopping it)?
    pub fn program_takes_ctrl_c(&self) -> bool {
        self.alternate || self.own_interrupt
    }

    fn set(&mut self, mode: u16, on: bool) {
        match mode {
            1 => self.app_cursor = on,
            1049 | 47 | 1047 => self.alternate = on,
            2004 => self.bracketed_paste = on,
            MODE_OWN_INTERRUPT => self.own_interrupt = on,
            MODE_RESIZE_REPORTS => self.resize_reports = on,
            _ => {}
        }
    }
}

/// DEC private mode for in-band resize reports ([`Modes::resize_reports`]).
pub const MODE_RESIZE_REPORTS: u16 = 2048;

/// The in-band resize report for a `cols x rows` text area (`ESC[48;rows;cols;0;0t`).
pub fn resize_report(cols: u16, rows: u16) -> Vec<u8> {
    format!("\x1b[48;{rows};{cols};0;0t").into_bytes()
}

/// Follows the DEC private modes in a program's output without drawing it
/// (for terminals that are only a byte stream, like SSH sessions).
#[derive(Default)]
pub struct ModeScanner {
    pub modes: Modes,
    state: u8,
    num: u16,
    nums: Vec<u16>,
}

impl ModeScanner {
    pub fn feed(&mut self, data: &[u8]) {
        for &b in data {
            self.state = match (self.state, b) {
                (_, 0x1b) => 1,
                (1, b'[') => 2,
                (2, b'?') => {
                    self.num = 0;
                    self.nums.clear();
                    3
                }
                (3, b'0'..=b'9') => {
                    self.num = self.num.saturating_mul(10).saturating_add((b - b'0') as u16);
                    3
                }
                (3, b';') => {
                    self.nums.push(self.num);
                    self.num = 0;
                    3
                }
                (3, b'h' | b'l') => {
                    self.nums.push(self.num);
                    for &m in &self.nums {
                        self.modes.set(m, b == b'h');
                    }
                    0
                }
                _ => 0,
            };
        }
    }
}

/// One cell's byte for a Unicode character (cells hold one byte): ASCII as
/// is, common typography and box drawing as their ASCII look-alikes, the
/// rest as `?`.
pub fn cell_byte(c: char) -> u8 {
    let cp = c as u32;
    if cp < 0x80 {
        return cp as u8;
    }
    match c {
        '\u{a0}' | '\u{2000}'..='\u{200a}' => b' ',
        '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{2032}' | '\u{b4}' => b'\'',
        '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{2033}' | '\u{ab}' | '\u{bb}' => b'"',
        '\u{2010}'..='\u{2015}' | '\u{2212}' => b'-',
        '\u{2026}' => b'.',
        '\u{b7}' | '\u{2022}' | '\u{2219}' | '\u{25cf}' | '\u{25cb}' => b'*',
        '\u{d7}' => b'x',
        '\u{2190}' => b'<',
        '\u{2192}' => b'>',
        '\u{2191}' => b'^',
        '\u{2193}' => b'v',
        '\u{2500}' | '\u{2501}' | '\u{2504}' | '\u{2505}' | '\u{2508}' | '\u{2509}' | '\u{254c}' | '\u{254d}' | '\u{2550}' | '\u{2574}' | '\u{2576}' => b'-',
        '\u{2502}' | '\u{2503}' | '\u{2506}' | '\u{2507}' | '\u{250a}' | '\u{250b}' | '\u{254e}' | '\u{254f}' | '\u{2551}' | '\u{2575}' | '\u{2577}' => b'|',
        '\u{250c}'..='\u{254b}' | '\u{2552}'..='\u{256c}' => b'+',
        '\u{256d}' | '\u{256e}' | '\u{256f}' | '\u{2570}' => b'+',
        '\u{2571}' => b'/',
        '\u{2572}' => b'\\',
        '\u{2573}' => b'X',
        '\u{2580}'..='\u{259f}' | '\u{25a0}' | '\u{25ae}' => b'#',
        '\u{c0}'..='\u{c5}' => b'A',
        '\u{c7}' => b'C',
        '\u{c8}'..='\u{cb}' => b'E',
        '\u{cc}'..='\u{cf}' => b'I',
        '\u{d1}' => b'N',
        '\u{d2}'..='\u{d6}' | '\u{d8}' => b'O',
        '\u{d9}'..='\u{dc}' => b'U',
        '\u{dd}' => b'Y',
        '\u{df}' => b's',
        '\u{e0}'..='\u{e5}' => b'a',
        '\u{e7}' => b'c',
        '\u{e8}'..='\u{eb}' => b'e',
        '\u{ec}'..='\u{ef}' => b'i',
        '\u{f1}' => b'n',
        '\u{f2}'..='\u{f6}' | '\u{f8}' => b'o',
        '\u{f9}'..='\u{fc}' => b'u',
        '\u{fd}' | '\u{ff}' => b'y',
        '\u{b0}' => b'o',
        _ => b'?',
    }
}

/// The nearest of the 16 colours for a 256-colour index.
fn nearest16(n: u16) -> u8 {
    match n {
        0..=15 => n as u8,
        16..=231 => {
            let n = n - 16;
            let (r, g, b) = (n / 36, (n / 6) % 6, n % 6);
            let lvl = |v: u16| if v == 0 { 0 } else { 55 + v * 40 };
            nearest_rgb(lvl(r), lvl(g), lvl(b))
        }
        _ => {
            let v = 8 + (n.min(255) - 232) * 10;
            nearest_rgb(v, v, v)
        }
    }
}

/// The nearest of the 16 colours (xterm's values) for an RGB colour.
fn nearest_rgb(r: u16, g: u16, b: u16) -> u8 {
    const RGB: [(u16, u16, u16); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    let d = |c: &(u16, u16, u16)| {
        let (dr, dg, db) = (c.0 as i32 - r as i32, c.1 as i32 - g as i32, c.2 as i32 - b as i32);
        dr * dr + dg * dg + db * db
    };
    (0..16).min_by_key(|&i| d(&RGB[i])).unwrap_or(7) as u8
}

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
    /// The CSI had a private marker (`?`, `>`, `=`, `<`).
    private: u8,
    /// The CSI had an intermediate byte (` `, `!`, `"`, ...): ignored.
    intermediate: bool,
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
    /// The scrolling region (DECSTBM), inclusive.
    top_margin: u16,
    bottom_margin: u16,
    /// A UTF-8 character being assembled: (code point so far, bytes still expected).
    utf8: (u32, u8),
    /// The last character printed (for REP).
    last_char: u8,
    modes: Modes,
    /// Replies to the program's queries (cursor position, status), in order.
    responses: Vec<u8>,
    /// BEL characters since the last [`Vte::take_bells`].
    bells: u32,
    /// Per screen row: the text runs on into the next row (it wrapped at the
    /// edge), so a resize can re-join and re-wrap it.
    wrapped: Vec<bool>,
    /// The same for the scrollback rows.
    sb_wrapped: Vec<bool>,
    saved_main_wrapped: Option<Vec<bool>>,
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
            private: 0,
            intermediate: false,
            cursor_visible: true,
            screen,
            scrollback: Vec::new(),
            scrollback_max: 500,
            saved_main_screen: None,
            saved_main_cursor: (0, 0),
            saved_main_attr: DEFAULT_ATTR,
            scrolled_total: 0,
            top_margin: 0,
            bottom_margin: height.saturating_sub(1),
            utf8: (0, 0),
            last_char: b' ',
            modes: Modes::default(),
            responses: Vec::new(),
            bells: 0,
            wrapped: vec![false; height as usize],
            sb_wrapped: Vec::new(),
            saved_main_wrapped: None,
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
            self.sb_wrapped.remove(0);
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

    /// The modes programs switched (alternate screen, bracketed paste, Ctrl+C handling).
    pub fn modes(&self) -> Modes {
        let mut m = self.modes;
        m.alternate = self.alternate_screen();
        m
    }

    /// A program ended: drop the modes it left on (bracketed paste, its own
    /// Ctrl+C) and leave the alternate screen if it never did.
    pub fn reset_modes(&mut self) {
        self.modes = Modes::default();
        self.leave_alternate_screen();
        self.top_margin = 0;
        self.bottom_margin = self.height - 1;
        self.cursor_visible = true;
        self.current_attr = self.default_attr;
        self.current_flags = 0;
        self.flush_cursor();
    }

    /// How many times the bell (BEL, Ctrl+G) rang since the last call.
    pub fn take_bells(&mut self) -> u32 {
        std::mem::take(&mut self.bells)
    }

    /// Replies the terminal owes the program (to its `ESC[6n` and `ESC[5n`
    /// queries), to be written to the program's input.
    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.responses)
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
        self.set_wrapped(y, false);
    }

    fn copy_row(&mut self, src_y: u16, dst_y: u16) {
        self.screen.copy_row(src_y, dst_y);
        let w = self.wrapped.get(src_y as usize).copied().unwrap_or(false);
        self.set_wrapped(dst_y, w);
    }

    fn set_wrapped(&mut self, y: u16, on: bool) {
        if let Some(f) = self.wrapped.get_mut(y as usize) {
            *f = on;
        }
    }

    fn read_row_bytes(&self, y: u16) -> Vec<u8> {
        self.screen.read_row(y)
    }

    // --- Cursor management ---

    fn flush_cursor(&mut self) {
        self.screen.flush_cursor(self.cursor_x.min(self.width - 1), self.cursor_y, self.cursor_visible);
    }

    // --- Scrolling ---

    /// Scroll rows `top..=bottom` up by `n`. Rows leaving the top of the whole
    /// screen go into the scrollback (not on the alternate screen).
    fn scroll_region_up(&mut self, top: u16, bottom: u16, n: u16) {
        if top > bottom || bottom >= self.height {
            return;
        }
        let n = n.min(bottom - top + 1);
        if top == 0 && !self.alternate_screen() {
            for i in 0..n {
                let row = self.read_row_bytes(i);
                self.scrollback.push(row);
                self.sb_wrapped.push(self.wrapped.get(i as usize).copied().unwrap_or(false));
                self.scrolled_total += 1;
                if self.scrollback.len() > self.scrollback_max {
                    self.scrollback.remove(0);
                    self.sb_wrapped.remove(0);
                }
            }
        }
        for y in top + n..=bottom {
            self.copy_row(y, y - n);
        }
        for y in (bottom + 1 - n)..=bottom {
            self.clear_row(y);
        }
    }

    fn scroll_region_down(&mut self, top: u16, bottom: u16, n: u16) {
        if top > bottom || bottom >= self.height {
            return;
        }
        let n = n.min(bottom - top + 1);
        for y in (top..=bottom).rev() {
            if y >= top + n {
                self.copy_row(y - n, y);
            } else {
                self.clear_row(y);
            }
        }
    }

    /// Rows `top..=bottom` move up by `n` without feeding the scrollback (DL).
    fn scroll_region_up_in_place(&mut self, top: u16, bottom: u16, n: u16) {
        if top > bottom || bottom >= self.height {
            return;
        }
        let n = n.min(bottom - top + 1);
        for y in top..=bottom {
            if y + n <= bottom {
                self.copy_row(y + n, y);
            } else {
                self.clear_row(y);
            }
        }
    }

    fn scroll_up(&mut self, n: u16) {
        self.scroll_region_up(self.top_margin, self.bottom_margin, n);
    }

    fn scroll_down(&mut self, n: u16) {
        self.scroll_region_down(self.top_margin, self.bottom_margin, n);
    }

    /// Move down a row, scrolling the region at its bottom margin.
    fn line_feed(&mut self) {
        if self.cursor_y == self.bottom_margin {
            self.scroll_up(1);
        } else if self.cursor_y + 1 < self.height {
            self.cursor_y += 1;
        }
    }

    // --- Alternate screen buffer ---

    /// Save current screen and switch to alternate buffer (ESC[?1049h).
    fn enter_alternate_screen(&mut self) {
        if self.saved_main_screen.is_some() {
            return;
        }
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
        self.saved_main_wrapped = Some(std::mem::replace(&mut self.wrapped, vec![false; self.height as usize]));
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
            if let Some(mut w) = self.saved_main_wrapped.take() {
                w.resize(self.height as usize, false);
                self.wrapped = w;
            }
        }
    }

    // --- Byte processing ---

    fn process_byte(&mut self, b: u8) {
        // C0 controls act even in the middle of a CSI sequence (as on a VT100).
        if self.state == ParseState::Csi && b < 0x20 && b != 0x1b {
            self.control(b);
            return;
        }
        match self.state {
            ParseState::Normal => self.process_normal(b),
            ParseState::Escape => self.process_escape(b),
            ParseState::Csi => self.process_csi(b),
            ParseState::Str => match b {
                0x07 => self.state = ParseState::Normal,
                0x1b => self.state = ParseState::StrEsc,
                _ => {}
            },
            ParseState::StrEsc => {
                self.state = if b == b'\\' { ParseState::Normal } else { ParseState::Str };
            }
            ParseState::Charset => self.state = ParseState::Normal,
        }
    }

    fn control(&mut self, b: u8) {
        match b {
            b'\n' | 0x0b | 0x0c => {
                // Line feed — move down (column 0, as this terminal always has), scroll at the margin
                self.cursor_x = 0;
                self.line_feed();
            }
            b'\r' => self.cursor_x = 0,
            0x08 => {
                // Backspace
                if self.cursor_x >= self.width {
                    self.cursor_x = self.width - 1;
                }
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                    // Erase the character at the new cursor position
                    self.put_cell(self.cursor_x, self.cursor_y, b' ', self.current_attr, 0);
                }
            }
            b'\t' => {
                // Tab — advance to next multiple of 8
                let next = (self.cursor_x.min(self.width - 1) + 8) & !7;
                self.cursor_x = next.min(self.width - 1);
            }
            0x07 => self.bells = self.bells.saturating_add(1),
            _ => {}
        }
    }

    fn process_normal(&mut self, b: u8) {
        if self.utf8.1 > 0 {
            if b & 0xC0 == 0x80 {
                self.utf8.0 = (self.utf8.0 << 6) | (b & 0x3F) as u32;
                self.utf8.1 -= 1;
                if self.utf8.1 == 0 {
                    let c = char::from_u32(self.utf8.0).unwrap_or('?');
                    self.put_char(cell_byte(c));
                }
                return;
            }
            // A broken sequence: show it, then handle this byte afresh.
            self.utf8 = (0, 0);
            self.put_char(b'?');
        }
        match b {
            0x1B => self.state = ParseState::Escape,
            0x00..=0x1F | 0x7F => self.control(b),
            0x20..=0x7E => self.put_char(b),
            0xC2..=0xDF => self.utf8 = ((b & 0x1F) as u32, 1),
            0xE0..=0xEF => self.utf8 = ((b & 0x0F) as u32, 2),
            0xF0..=0xF4 => self.utf8 = ((b & 0x07) as u32, 3),
            _ => self.put_char(b'?'),
        }
    }

    fn put_char(&mut self, ch: u8) {
        if self.cursor_x >= self.width {
            // Line wrap: the row runs on into the next one.
            self.set_wrapped(self.cursor_y, true);
            self.cursor_x = 0;
            self.line_feed();
        }
        self.put_cell(self.cursor_x, self.cursor_y, ch, self.current_attr, self.current_flags);
        self.cursor_x += 1;
        self.last_char = ch;
    }

    fn process_escape(&mut self, b: u8) {
        self.state = ParseState::Normal;
        match b {
            b'[' => {
                // CSI introducer
                self.state = ParseState::Csi;
                self.params = [0; MAX_PARAMS];
                self.param_count = 0;
                self.private = 0;
                self.intermediate = false;
            }
            b']' | b'P' | b'_' | b'^' | b'X' => self.state = ParseState::Str,
            b'(' | b')' | b'*' | b'+' | b'#' | b'%' => self.state = ParseState::Charset,
            b'c' => self.reset(),
            b'D' => self.line_feed(),
            b'E' => {
                self.cursor_x = 0;
                self.line_feed();
            }
            b'M' => {
                // RI — reverse index (move cursor up, scroll at the top margin)
                if self.cursor_y == self.top_margin {
                    self.scroll_down(1);
                } else {
                    self.cursor_y = self.cursor_y.saturating_sub(1);
                }
            }
            b'7' => self.saved_cursor = (self.cursor_x, self.cursor_y),
            b'8' => {
                self.cursor_x = self.saved_cursor.0.min(self.width - 1);
                self.cursor_y = self.saved_cursor.1.min(self.height - 1);
            }
            0x1b => self.state = ParseState::Escape,
            _ => {}
        }
    }

    fn process_csi(&mut self, b: u8) {
        match b {
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
            b';' | b':' => {
                // Parameter separator
                if self.param_count == 0 {
                    self.param_count = 1;
                }
                if self.param_count < MAX_PARAMS {
                    self.param_count += 1;
                }
            }
            b'<'..=b'?' => self.private = b,
            0x20..=0x2F => self.intermediate = true,
            0x40..=0x7E => {
                // Final byte — dispatch
                self.state = ParseState::Normal;
                if self.intermediate {
                    return;
                }
                match self.private {
                    0 => self.handle_csi(b),
                    b'?' => self.handle_private(b),
                    _ => {}
                }
            }
            0x1b => self.state = ParseState::Escape,
            _ => self.state = ParseState::Normal,
        }
    }

    fn handle_private(&mut self, fin: u8) {
        if fin != b'h' && fin != b'l' {
            return;
        }
        let on = fin == b'h';
        for i in 0..self.param_count.max(1).min(MAX_PARAMS) {
            let p = self.params[i];
            match p {
                25 => self.cursor_visible = on,
                1049 | 47 | 1047 => {
                    if on {
                        self.enter_alternate_screen();
                    } else {
                        self.leave_alternate_screen();
                    }
                }
                MODE_RESIZE_REPORTS if on => {
                    let r = resize_report(self.width, self.height);
                    self.responses.extend_from_slice(&r);
                }
                _ => {}
            }
            self.modes.set(p, on);
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
        let (w, h) = (self.width, self.height);
        if self.cursor_x >= w && !matches!(final_byte, b'm' | b'n' | b's' | b'r' | b'h' | b'l') {
            // Any cursor or editing sequence ends a pending wrap.
            self.cursor_x = w - 1;
        }
        match final_byte {
            b'A' => self.cursor_y = self.cursor_y.saturating_sub(self.param(0, 1)),
            b'B' | b'e' => self.cursor_y = (self.cursor_y + self.param(0, 1)).min(h - 1),
            b'C' | b'a' => self.cursor_x = (self.cursor_x + self.param(0, 1)).min(w - 1),
            b'D' => self.cursor_x = self.cursor_x.saturating_sub(self.param(0, 1)),
            b'E' => {
                self.cursor_y = (self.cursor_y + self.param(0, 1)).min(h - 1);
                self.cursor_x = 0;
            }
            b'F' => {
                self.cursor_y = self.cursor_y.saturating_sub(self.param(0, 1));
                self.cursor_x = 0;
            }
            b'G' | b'`' => self.cursor_x = (self.param(0, 1) - 1).min(w - 1),
            b'd' => self.cursor_y = (self.param(0, 1) - 1).min(h - 1),
            b'H' | b'f' => {
                // Cursor Position (CUP) — CSI row ; col H
                self.cursor_y = (self.param(0, 1) - 1).min(h - 1);
                self.cursor_x = (self.param(1, 1) - 1).min(w - 1);
            }
            b'I' => {
                for _ in 0..self.param(0, 1) {
                    self.cursor_x = ((self.cursor_x + 8) & !7).min(w - 1);
                }
            }
            b'Z' => {
                for _ in 0..self.param(0, 1) {
                    self.cursor_x = if self.cursor_x == 0 { 0 } else { (self.cursor_x - 1) & !7 };
                }
            }
            b'J' => match self.param(0, 0) {
                0 => {
                    self.clear_row_range(self.cursor_y, self.cursor_x, w);
                    for y in (self.cursor_y + 1)..h {
                        self.clear_row(y);
                    }
                }
                1 => {
                    for y in 0..self.cursor_y {
                        self.clear_row(y);
                    }
                    self.clear_row_range(self.cursor_y, 0, self.cursor_x + 1);
                }
                2 => {
                    for y in 0..h {
                        self.clear_row(y);
                    }
                }
                3 => {
                    // Clear the scrollback too.
                    self.scrollback.clear();
                    self.sb_wrapped.clear();
                    for y in 0..h {
                        self.clear_row(y);
                    }
                }
                _ => {}
            },
            b'K' => match self.param(0, 0) {
                0 => self.clear_row_range(self.cursor_y, self.cursor_x, w),
                1 => self.clear_row_range(self.cursor_y, 0, self.cursor_x + 1),
                2 => self.clear_row(self.cursor_y),
                _ => {}
            },
            b'X' => {
                // ECH — erase characters
                let n = self.param(0, 1);
                let x = self.cursor_x;
                self.clear_row_range(self.cursor_y, x, x.saturating_add(n));
            }
            b'@' => {
                // ICH — insert blank characters, shifting the rest right
                let n = self.param(0, 1).min(w - self.cursor_x);
                let y = self.cursor_y;
                for x in (self.cursor_x + n..w).rev() {
                    let (c, a, f) = self.get_cell(x - n, y);
                    self.put_cell(x, y, c, a, f);
                }
                self.clear_row_range(y, self.cursor_x, self.cursor_x + n);
            }
            b'P' => {
                // DCH — delete characters, pulling the rest left
                let n = self.param(0, 1).min(w - self.cursor_x);
                let y = self.cursor_y;
                for x in self.cursor_x..w - n {
                    let (c, a, f) = self.get_cell(x + n, y);
                    self.put_cell(x, y, c, a, f);
                }
                self.clear_row_range(y, w - n, w);
            }
            b'L' => {
                // IL — insert lines at the cursor (inside the scrolling region)
                if (self.top_margin..=self.bottom_margin).contains(&self.cursor_y) {
                    self.scroll_region_down(self.cursor_y, self.bottom_margin, self.param(0, 1));
                    self.cursor_x = 0;
                }
            }
            b'M' => {
                // DL — delete lines at the cursor
                if (self.top_margin..=self.bottom_margin).contains(&self.cursor_y) {
                    let (y, b) = (self.cursor_y, self.bottom_margin);
                    self.scroll_region_up_in_place(y, b, self.param(0, 1));
                    self.cursor_x = 0;
                }
            }
            b'S' => self.scroll_up(self.param(0, 1)),
            b'T' => self.scroll_down(self.param(0, 1)),
            b'b' => {
                // REP — repeat the last character
                let c = self.last_char;
                for _ in 0..self.param(0, 1).min(w * h) {
                    self.put_char(c);
                }
            }
            b'r' => {
                // DECSTBM — scrolling region (top;bottom), cursor home
                let top = self.param(0, 1) - 1;
                let bottom = self.param(1, h).min(h) - 1;
                if top < bottom {
                    self.top_margin = top;
                    self.bottom_margin = bottom;
                } else {
                    self.top_margin = 0;
                    self.bottom_margin = h - 1;
                }
                self.cursor_x = 0;
                self.cursor_y = 0;
            }
            b'm' => self.handle_sgr(),
            b's' => self.saved_cursor = (self.cursor_x, self.cursor_y),
            b'u' => {
                self.cursor_x = self.saved_cursor.0.min(w - 1);
                self.cursor_y = self.saved_cursor.1.min(h - 1);
            }
            b'n' => match self.param(0, 0) {
                // DSR: 5 = status (OK), 6 = cursor position.
                5 => self.responses.extend_from_slice(b"\x1b[0n"),
                6 => {
                    let r = format!("\x1b[{};{}R", self.cursor_y + 1, self.cursor_x.min(w - 1) + 1);
                    self.responses.extend_from_slice(r.as_bytes());
                }
                _ => {}
            },
            b't' => {
                // Window ops: 18 = report the text area size.
                if self.param(0, 0) == 18 {
                    let r = format!("\x1b[8;{};{}t", h, w);
                    self.responses.extend_from_slice(r.as_bytes());
                }
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
        while i < self.param_count.min(MAX_PARAMS) {
            let p = self.params[i];
            match p {
                0 => {
                    self.current_attr = self.default_attr;
                    self.current_flags = 0;
                }
                1 => self.current_flags |= 0x01,
                4 => self.current_flags |= 0x02,
                5 | 6 => self.current_flags |= 0x04,
                7 => self.current_flags |= 0x08,
                22 => self.current_flags &= !0x01,
                24 => self.current_flags &= !0x02,
                25 => self.current_flags &= !0x04,
                27 => self.current_flags &= !0x08,
                30..=37 => self.current_attr = (self.current_attr & 0xF0) | (p - 30) as u8,
                39 => self.current_attr = (self.current_attr & 0xF0) | (self.default_attr & 0x0F),
                40..=47 => self.current_attr = (self.current_attr & 0x0F) | (((p - 40) as u8) << 4),
                49 => self.current_attr = (self.current_attr & 0x0F) | (self.default_attr & 0xF0),
                90..=97 => self.current_attr = (self.current_attr & 0xF0) | (p - 90 + 8) as u8,
                100..=107 => self.current_attr = (self.current_attr & 0x0F) | (((p - 100 + 8) as u8) << 4),
                38 | 48 => {
                    // Extended colour: 5;n (256 colours) or 2;r;g;b (truecolor), drawn as the nearest of 16.
                    let colour = match self.params.get(i + 1) {
                        Some(5) if i + 2 < self.param_count => {
                            let c = nearest16(self.params[i + 2]);
                            i += 2;
                            Some(c)
                        }
                        Some(2) if i + 4 < self.param_count => {
                            let c = nearest_rgb(self.params[i + 2].min(255), self.params[i + 3].min(255), self.params[i + 4].min(255));
                            i += 4;
                            Some(c)
                        }
                        _ => {
                            i = self.param_count;
                            None
                        }
                    };
                    if let Some(c) = colour {
                        if p == 38 {
                            self.current_attr = (self.current_attr & 0xF0) | c;
                        } else {
                            self.current_attr = (self.current_attr & 0x0F) | (c << 4);
                        }
                    }
                }
                _ => {
                    // Dim, italic, hidden, strike-through and the rest: not drawn.
                }
            }
            i += 1;
        }
    }

    /// Change the grid to `width x height` (a display panel was swapped).
    /// On the main screen a width change re-wraps the text, as real
    /// terminals do: lines that wrapped at the old edge are joined and wrapped
    /// at the new one, the scrollback included, and the cursor keeps its place
    /// in its line. Otherwise (the alternate screen, or only the height
    /// changes) rows are cut or padded on the right, and if the cursor would
    /// fall off the bottom the top rows scroll away so the cursor's row stays
    /// the last one. The physical VTE also resizes the shared framebuffer
    /// (clamped to its region). Returns the size actually set.
    pub fn resize(&mut self, width: u16, height: u16) -> (u16, u16) {
        let (w0, h0) = (self.width, self.height);
        let (w1, h1) = self.screen.clamp(width, height);
        if w1 != w0 && !self.alternate_screen() {
            self.reflow(w0, h0, w1, h1);
        } else {
            let shift = if self.cursor_y >= h1 { self.cursor_y + 1 - h1 } else { 0 };
            self.screen.resize(w0, h0, w1, h1, shift, self.default_attr);
            if let Some(saved) = self.saved_main_screen.take() {
                let s = self.saved_main_cursor.1;
                let sshift = if s >= h1 { s + 1 - h1 } else { 0 };
                self.saved_main_screen = Some(relayout(&saved, w0, h0, w1, h1, sshift, self.default_attr));
                self.saved_main_cursor = (self.saved_main_cursor.0.min(w1 - 1), s - sshift);
                self.saved_main_wrapped = Some(vec![false; h1 as usize]);
            }
            let mut w: Vec<bool> = self.wrapped.iter().skip(shift as usize).copied().collect();
            w.resize(h1 as usize, false);
            if w1 != w0 {
                w.iter_mut().for_each(|f| *f = false);
            }
            self.wrapped = w;
            self.cursor_y -= shift;
        }
        self.width = w1;
        self.height = h1;
        self.top_margin = 0;
        self.bottom_margin = h1 - 1;
        self.cursor_x = self.cursor_x.min(w1);
        self.saved_cursor = (self.saved_cursor.0.min(w1 - 1), self.saved_cursor.1.min(h1 - 1));
        self.flush_cursor();
        if self.modes.resize_reports && (w1, h1) != (w0, h0) {
            let r = resize_report(w1, h1);
            self.responses.extend_from_slice(&r);
        }
        (w1, h1)
    }

    /// Re-wrap the scrollback and the screen from width `w0` to `w1`.
    fn reflow(&mut self, w0: u16, h0: u16, w1: u16, h1: u16) {
        let def = self.default_attr;
        let (w0u, w1u) = (w0 as usize, w1 as usize);
        let blank_cell = |c: &[u8]| c[0] == b' ' && c[2] == 0 && (c[1] & 0xF0) == (def & 0xF0);
        // Every row, oldest first, with its wrap flag.
        let old_sb = self.scrollback.len();
        let mut rows: Vec<(Vec<u8>, bool)> = std::mem::take(&mut self.scrollback).into_iter().zip(std::mem::take(&mut self.sb_wrapped)).collect();
        for y in 0..h0 {
            let mut r = self.read_row_bytes(y);
            r.resize(w0u * CELL_SIZE, 0);
            rows.push((r, self.wrapped.get(y as usize).copied().unwrap_or(false)));
        }
        let cur_row = old_sb + self.cursor_y as usize;
        let last = (old_sb..rows.len()).rev().find(|&i| !rows[i].0.chunks(CELL_SIZE).all(blank_cell)).unwrap_or(0).max(cur_row);
        rows.truncate(last + 1);
        // Logical lines (wrapped rows joined, trailing blanks dropped).
        let mut lines: Vec<Vec<u8>> = Vec::new();
        let (mut cur_line, mut cur_off) = (0, 0);
        let mut line: Vec<u8> = Vec::new();
        for (i, (cells, wrapped)) in rows.into_iter().enumerate() {
            let at_cursor = i == cur_row;
            if at_cursor {
                cur_line = lines.len();
                cur_off = line.len() / CELL_SIZE + (self.cursor_x as usize).min(w0u);
            }
            line.extend_from_slice(&cells);
            if !wrapped {
                let mut n = line.len() / CELL_SIZE;
                let keep = if lines.len() == cur_line && (at_cursor || cur_line == lines.len() && i >= cur_row) { cur_off } else { 0 };
                while n > keep && blank_cell(&line[(n - 1) * CELL_SIZE..n * CELL_SIZE]) {
                    n -= 1;
                }
                line.truncate(n * CELL_SIZE);
                lines.push(std::mem::take(&mut line));
            }
        }
        if !line.is_empty() || lines.len() <= cur_line {
            lines.push(line);
        }
        // Wrap them at the new width.
        let mut new_rows: Vec<(Vec<u8>, bool)> = Vec::new();
        let (mut cy, mut cx) = (0usize, 0usize);
        for (li, l) in lines.into_iter().enumerate() {
            let n = l.len() / CELL_SIZE;
            let mut chunks = n.div_ceil(w1u).max(1);
            if li == cur_line {
                chunks = chunks.max(cur_off / w1u + 1);
                cy = new_rows.len() + cur_off / w1u;
                cx = cur_off % w1u;
            }
            for k in 0..chunks {
                let a = (k * w1u * CELL_SIZE).min(l.len());
                let b = ((k + 1) * w1u * CELL_SIZE).min(l.len());
                let mut r = l[a..b].to_vec();
                while r.len() < w1u * CELL_SIZE {
                    r.extend_from_slice(&[b' ', def, 0, 0]);
                }
                new_rows.push((r, k + 1 < chunks));
            }
        }
        // The screen shows the last rows (the cursor's row on it).
        let start = new_rows.len().saturating_sub(h1 as usize).min(cy);
        let screen_rows = new_rows.split_off(start);
        let drop = new_rows.len().saturating_sub(self.scrollback_max);
        for (cells, w) in new_rows.into_iter().skip(drop) {
            self.scrollback.push(cells);
            self.sb_wrapped.push(w);
        }
        if self.scrollback.len() > old_sb {
            self.scrolled_total += (self.scrollback.len() - old_sb) as u64;
        }
        self.screen.resize(w0, h0, w1, h1, 0, def);
        self.width = w1;
        self.height = h1;
        self.wrapped = vec![false; h1 as usize];
        for y in 0..h1 as usize {
            let (cells, w) = screen_rows.get(y).cloned().unwrap_or_else(|| (Vec::new(), false));
            for x in 0..w1u {
                let c = cells.get(x * CELL_SIZE..x * CELL_SIZE + 3).map(|c| (c[0], c[1], c[2])).unwrap_or((b' ', def, 0));
                self.put_cell(x as u16, y as u16, c.0, c.1, c.2);
            }
            self.wrapped[y] = w;
        }
        self.cursor_y = (cy - start) as u16;
        self.cursor_x = cx as u16;
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
        self.top_margin = 0;
        self.bottom_margin = self.height - 1;
        self.utf8 = (0, 0);
        for y in 0..self.height {
            self.clear_row(y);
        }
        self.flush_cursor();
    }

    /// Does screen row `y` run on into the next one (it wrapped)?
    pub fn row_wrapped(&self, y: u16) -> bool {
        self.wrapped.get(y as usize).copied().unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(v: &Vte, y: u16) -> String {
        (0..v.width()).map(|x| v.cell(x, y).0 as char).collect::<String>().trim_end().to_string()
    }

    #[test]
    fn in_band_resize_reports() {
        let mut v = Vte::new_virtual(80, 25);
        v.resize(70, 20);
        assert!(v.take_responses().is_empty(), "no reports until a program asks");
        v.write(b"\x1b[?2048h");
        assert!(v.modes().resize_reports);
        assert_eq!(v.take_responses(), b"\x1b[48;20;70;0;0t", "set: the size now");
        v.resize(100, 30);
        assert_eq!(v.take_responses(), b"\x1b[48;30;100;0;0t", "every change");
        v.resize(100, 30);
        assert!(v.take_responses().is_empty(), "the same size again is no change");
        v.reset_modes();
        v.resize(80, 25);
        assert!(v.take_responses().is_empty(), "a program's exit ends the reports");
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

    #[test]
    fn osc_titles_and_private_csis_print_nothing() {
        let mut v = Vte::new_virtual(20, 3);
        v.write(b"a\x1b]0;my title\x07b\x1b]2;t2\x1b\\c\x1b[>0cd\x1b[?1;2ce\x1b(Bf");
        assert_eq!(row(&v, 0), "abcdef");
    }

    #[test]
    fn extended_colours_map_to_the_16() {
        let mut v = Vte::new_virtual(20, 3);
        v.write(b"\x1b[38;5;196mr\x1b[48;5;21mb\x1b[0m\x1b[38;2;0;205;0mg\x1b[38;5;244mx");
        assert_eq!(v.cell(0, 0).1 & 0x0F, 9, "256-colour red is bright red");
        assert_eq!(v.cell(0, 0).2 & 0x04, 0, "not blink");
        assert_eq!(v.cell(1, 0).1 >> 4, 4, "blue background");
        assert_eq!(v.cell(2, 0).1 & 0x0F, 2, "truecolor green");
        assert_eq!(v.cell(3, 0).1 & 0x0F, 8, "grey ramp");
    }

    #[test]
    fn insert_delete_chars_and_lines_and_scroll_region() {
        let mut v = Vte::new_virtual(10, 5);
        v.write(b"abcdef\x1b[1;3H\x1b[2@XY");
        assert_eq!(row(&v, 0), "abXYcdef");
        v.write(b"\x1b[1;3H\x1b[2P");
        assert_eq!(row(&v, 0), "abcdef");
        v.write(b"\x1b[1;2H\x1b[3X");
        assert_eq!(row(&v, 0), "a   ef");
        v.write(b"\x1b[2;1H1\r\n2\r\n3\r\n4");
        v.write(b"\x1b[2;1H\x1b[L");
        assert_eq!((row(&v, 1), row(&v, 2), row(&v, 4)), (String::new(), "1".into(), "3".into()));
        v.write(b"\x1b[M");
        assert_eq!((row(&v, 1), row(&v, 3), row(&v, 4)), ("1".into(), "3".into(), String::new()));
        // A scroll region rows 2-4: a line feed at its bottom scrolls only it.
        let total = v.scrolled_total();
        v.write(b"\x1b[2;4r\x1b[4;1H\nnew");
        assert_eq!(row(&v, 0), "a   ef", "row above the region stays");
        assert_eq!(row(&v, 3), "new");
        assert_eq!(row(&v, 4), "", "row below the region stays");
        assert_eq!(v.scrolled_total(), total, "region scrolls don't feed the scrollback");
        v.write(b"\x1b[r");
    }

    #[test]
    fn utf8_takes_one_cell() {
        let mut v = Vte::new_virtual(20, 3);
        v.write("caf\u{e9} \u{2500}\u{2502}\u{250c} \u{2018}x\u{2019} \u{1f600}!".as_bytes());
        assert_eq!(row(&v, 0), "cafe -|+ 'x' ?!");
        // A sequence split across writes.
        v.write(b"\r\n\xc3");
        v.write(b"\xa9");
        assert_eq!(row(&v, 1), "e");
    }

    #[test]
    fn modes_responses_and_no_scrollback_on_the_alternate_screen() {
        let mut v = Vte::new_virtual(10, 3);
        v.write(b"\x1b[?2004;7727h");
        assert!(v.modes().bracketed_paste && v.modes().own_interrupt);
        assert!(v.modes().program_takes_ctrl_c());
        v.write(b"\x1b[?7727l");
        assert!(!v.modes().program_takes_ctrl_c());
        v.write(b"\x1b[?1049h");
        assert!(v.modes().program_takes_ctrl_c());
        let before = v.scrolled_total();
        v.write(b"1\n2\n3\n4\n5\n");
        assert_eq!(v.scrolled_total(), before, "full-screen programs don't fill the scrollback");
        v.reset_modes();
        assert!(!v.alternate_screen() && !v.modes().bracketed_paste);
        v.write(b"\x1b[2;3H\x1b[6n\x1b[5n");
        assert_eq!(v.take_responses(), b"\x1b[2;3R\x1b[0n");
        let mut m = ModeScanner::default();
        m.feed(b"text\x1b[?10");
        m.feed(b"49h");
        assert!(m.modes.alternate);
        m.feed(b"\x1b[?1049l\x1b[?7727h");
        assert!(!m.modes.alternate && m.modes.own_interrupt);
    }

    #[test]
    fn a_width_change_rewraps_lines_and_keeps_the_cursor_in_its_line() {
        let mut v = Vte::new_virtual(10, 4);
        // A 15-character line wraps; then a short line and a prompt.
        v.write(b"abcdefghijklmno\r\nshort\r\n> ");
        assert_eq!((row(&v, 0), row(&v, 1)), ("abcdefghij".into(), "klmno".into()));
        assert!(v.row_wrapped(0) && !v.row_wrapped(1));
        v.resize(20, 4);
        assert_eq!(row(&v, 0), "abcdefghijklmno", "re-joined");
        assert_eq!(row(&v, 1), "short");
        assert_eq!(row(&v, 2), ">");
        assert_eq!(v.cursor(), (2, 2), "after the prompt's space");
        v.resize(5, 4);
        // 15 chars -> 3 rows, short -> 1, prompt -> 1: the top row goes to the scrollback.
        assert_eq!(v.scrollback().len(), 1);
        assert_eq!((row(&v, 0), row(&v, 1), row(&v, 2), row(&v, 3)), ("fghij".into(), "klmno".into(), "short".into(), ">".into()));
        assert_eq!(v.cursor(), (2, 3));
        // Back to 10: the scrollback row re-joins its line.
        v.resize(10, 4);
        assert_eq!((row(&v, 0), row(&v, 1)), ("abcdefghij".into(), "klmno".into()));
        assert!(v.scrollback().is_empty());
    }
}
