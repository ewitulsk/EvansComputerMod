use std::io::{self, Write};
use crossterm::{
    cursor,
    execute,
    style::{Print, Color, SetForegroundColor, SetBackgroundColor, ResetColor},
    terminal::{self, ClearType},
};

/// Framebuffer memory layout constants (must match operating-system/rust/src/framebuffer.rs)
pub const FB_BASE: usize = 0x20000;
pub const FB_HEADER_SIZE: usize = 64;
pub const FB_CELL_BASE: usize = FB_BASE + FB_HEADER_SIZE;
pub const CELL_SIZE: usize = 4;
pub const FB_MAGIC: u16 = 0xFB01;

// Header offsets from FB_BASE
const OFF_MAGIC: usize = 0x00;
const OFF_WIDTH: usize = 0x02;
const OFF_HEIGHT: usize = 0x04;
const OFF_CURSOR_X: usize = 0x06;
const OFF_CURSOR_Y: usize = 0x08;
const OFF_CURSOR_VISIBLE: usize = 0x0A;
const OFF_DIRTY_COUNTER: usize = 0x0C;

/// 16-color ANSI palette mapped to crossterm colors.
const PALETTE: [Color; 16] = [
    Color::Rgb { r: 0x00, g: 0x00, b: 0x00 }, // 0  Black
    Color::Rgb { r: 0xAA, g: 0x00, b: 0x00 }, // 1  Red
    Color::Rgb { r: 0x00, g: 0xAA, b: 0x00 }, // 2  Green
    Color::Rgb { r: 0xAA, g: 0x55, b: 0x00 }, // 3  Yellow/Brown
    Color::Rgb { r: 0x00, g: 0x00, b: 0xAA }, // 4  Blue
    Color::Rgb { r: 0xAA, g: 0x00, b: 0xAA }, // 5  Magenta
    Color::Rgb { r: 0x00, g: 0xAA, b: 0xAA }, // 6  Cyan
    Color::Rgb { r: 0xAA, g: 0xAA, b: 0xAA }, // 7  Light Gray
    Color::Rgb { r: 0x55, g: 0x55, b: 0x55 }, // 8  Dark Gray
    Color::Rgb { r: 0xFF, g: 0x55, b: 0x55 }, // 9  Light Red
    Color::Rgb { r: 0x55, g: 0xFF, b: 0x55 }, // 10 Light Green
    Color::Rgb { r: 0xFF, g: 0xFF, b: 0x55 }, // 11 Yellow
    Color::Rgb { r: 0x55, g: 0x55, b: 0xFF }, // 12 Light Blue
    Color::Rgb { r: 0xFF, g: 0x55, b: 0xFF }, // 13 Light Magenta
    Color::Rgb { r: 0x55, g: 0xFF, b: 0xFF }, // 14 Light Cyan
    Color::Rgb { r: 0xFF, g: 0xFF, b: 0xFF }, // 15 White
];

/// Framebuffer renderer — reads cell data from WASM memory and renders to the real terminal.
pub struct FramebufferRenderer {
    pub width: usize,
    pub height: usize,
    /// Last seen dirty counter from the framebuffer header.
    last_dirty_counter: u32,
    /// Shadow copy of cell data for diffing (optimization).
    prev_cells: Vec<u8>,
    /// Previous cursor position for diff rendering.
    prev_cursor: (u16, u16),
    /// Headless mode: write raw text to stdout.
    pub headless: bool,
}

impl FramebufferRenderer {
    pub fn new(width: usize, height: usize) -> Self {
        let cell_count = width * height;
        Self {
            width,
            height,
            last_dirty_counter: u32::MAX, // force first render
            prev_cells: vec![0u8; cell_count * CELL_SIZE],
            prev_cursor: (0, 0),
            headless: false,
        }
    }

    /// Render from a framebuffer slice (starting at offset 0 = the header).
    /// This is the primary rendering method. The slice should be exactly:
    /// header (64 bytes) + cells (width * height * 4 bytes).
    pub fn render_from_fb_slice(&mut self, fb: &[u8]) -> io::Result<()> {
        if fb.len() < FB_HEADER_SIZE {
            return Ok(());
        }

        // Read header (offsets are relative to start of slice)
        let magic = read_u16(fb, OFF_MAGIC);
        if magic != FB_MAGIC {
            return Ok(()); // Not initialized yet
        }

        let width = read_u16(fb, OFF_WIDTH) as usize;
        let height = read_u16(fb, OFF_HEIGHT) as usize;
        let cursor_x = read_u16(fb, OFF_CURSOR_X);
        let cursor_y = read_u16(fb, OFF_CURSOR_Y);
        let cursor_visible = fb[OFF_CURSOR_VISIBLE] != 0;
        let dirty_counter = read_u32(fb, OFF_DIRTY_COUNTER);

        if dirty_counter == self.last_dirty_counter {
            return Ok(()); // No changes
        }
        self.last_dirty_counter = dirty_counter;

        // Update our dimensions if they changed
        self.width = width;
        self.height = height;

        let cell_count = width * height;
        let cells_start = FB_HEADER_SIZE;
        let cells_end = cells_start + cell_count * CELL_SIZE;
        if fb.len() < cells_end {
            return Ok(());
        }

        let cells = &fb[cells_start..cells_end];

        if self.headless {
            // Headless mode: just dump the text content
            self.render_headless(cells, width, height)?;
        } else {
            // Interactive mode: render with colors to real terminal
            self.render_interactive(cells, width, height, cursor_x, cursor_y, cursor_visible)?;
        }

        // Update shadow copy
        if self.prev_cells.len() != cells.len() {
            self.prev_cells = cells.to_vec();
        } else {
            self.prev_cells.copy_from_slice(cells);
        }
        self.prev_cursor = (cursor_x, cursor_y);

        Ok(())
    }

    fn render_headless(&self, cells: &[u8], width: usize, height: usize) -> io::Result<()> {
        let mut stdout = io::stdout();
        // In headless mode, print only non-empty lines (trimmed of trailing spaces)
        for y in 0..height {
            let mut line = String::with_capacity(width);
            for x in 0..width {
                let off = (y * width + x) * CELL_SIZE;
                let ch = cells[off];
                if ch >= 0x20 && ch < 0x7F {
                    line.push(ch as char);
                } else {
                    line.push(' ');
                }
            }
            let trimmed = line.trim_end();
            if !trimmed.is_empty() {
                writeln!(stdout, "{}", trimmed)?;
            }
        }
        stdout.flush()?;
        Ok(())
    }

    fn render_interactive(
        &mut self,
        cells: &[u8],
        width: usize,
        height: usize,
        cursor_x: u16,
        cursor_y: u16,
        cursor_visible: bool,
    ) -> io::Result<()> {
        let mut stdout = io::stdout();

        // Get real terminal size and limit rendering to what fits
        let (term_cols, term_rows) = terminal::size().unwrap_or((80, 24));
        let render_width = width.min(term_cols as usize);
        let render_height = height.min(term_rows as usize);

        let mut last_fg: Option<Color> = None;
        let mut last_bg: Option<Color> = None;

        for y in 0..render_height {
            execute!(stdout, cursor::MoveTo(0, y as u16))?;

            for x in 0..render_width {
                let off = (y * width + x) * CELL_SIZE;
                let prev_off = if off + 3 < self.prev_cells.len() { off } else { usize::MAX };

                // Check if this cell changed
                let changed = prev_off == usize::MAX
                    || cells[off] != self.prev_cells[prev_off]
                    || cells[off + 1] != self.prev_cells[prev_off + 1]
                    || cells[off + 2] != self.prev_cells[prev_off + 2];

                if !changed && (cursor_x, cursor_y) == self.prev_cursor {
                    continue; // Skip unchanged cells (unless cursor moved)
                }

                let ch = cells[off];
                let attr = cells[off + 1];
                let _flags = cells[off + 2];

                let fg_idx = (attr & 0x0F) as usize;
                let bg_idx = ((attr >> 4) & 0x0F) as usize;
                let fg = PALETTE[fg_idx];
                let bg = PALETTE[bg_idx];

                // Only emit color changes when they differ
                if last_fg != Some(fg) {
                    execute!(stdout, SetForegroundColor(fg))?;
                    last_fg = Some(fg);
                }
                if last_bg != Some(bg) {
                    execute!(stdout, SetBackgroundColor(bg))?;
                    last_bg = Some(bg);
                }

                execute!(stdout, cursor::MoveTo(x as u16, y as u16))?;

                let display_char = if ch >= 0x20 && ch < 0x7F {
                    ch as char
                } else {
                    ' '
                };
                execute!(stdout, Print(display_char))?;
            }
        }

        // Reset colors and position cursor
        execute!(stdout, ResetColor)?;

        if cursor_visible {
            execute!(
                stdout,
                cursor::MoveTo(cursor_x.min(render_width as u16 - 1), cursor_y.min(render_height as u16 - 1)),
                cursor::Show,
            )?;
        } else {
            execute!(stdout, cursor::Hide)?;
        }

        stdout.flush()?;
        Ok(())
    }
}

// Helper functions to read little-endian values from byte slices
fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
}

// ---------------------------------------------------------------------------
// Legacy TerminalBuffer — kept for internal use by the TTY and process systems.
// This is NOT used for the physical display (that's FramebufferRenderer above).
// ---------------------------------------------------------------------------

pub const DEFAULT_WIDTH: usize = 80;
pub const DEFAULT_HEIGHT: usize = 24;

pub struct TerminalBuffer {
    pub buffer: Vec<Vec<char>>,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pub width: usize,
    pub height: usize,
    pub headless: bool,
}

impl TerminalBuffer {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            buffer: vec![vec![' '; width]; height],
            cursor_x: 0,
            cursor_y: 0,
            width,
            height,
            headless: true,
        }
    }

    pub fn clear(&mut self) {
        for row in &mut self.buffer {
            for ch in row.iter_mut() {
                *ch = ' ';
            }
        }
        self.cursor_x = 0;
        self.cursor_y = 0;
    }

    pub fn set_cursor(&mut self, x: i32, y: i32) {
        self.cursor_x = (x as usize).min(self.width.saturating_sub(1));
        self.cursor_y = (y as usize).min(self.height.saturating_sub(1));
    }

    pub fn write_text(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                '\n' => {
                    self.cursor_x = 0;
                    self.cursor_y += 1;
                    if self.cursor_y >= self.height {
                        self.scroll_up();
                        self.cursor_y = self.height - 1;
                    }
                }
                '\r' => {
                    self.cursor_x = 0;
                }
                '\x08' => {
                    if self.cursor_x > 0 {
                        self.cursor_x -= 1;
                    }
                }
                c if c >= ' ' || c == '\t' => {
                    if c == '\t' {
                        let target = (self.cursor_x + 8) & !7;
                        while self.cursor_x < target && self.cursor_x < self.width {
                            self.buffer[self.cursor_y][self.cursor_x] = ' ';
                            self.cursor_x += 1;
                        }
                        if self.cursor_x >= self.width {
                            self.cursor_x = 0;
                            self.cursor_y += 1;
                            if self.cursor_y >= self.height {
                                self.scroll_up();
                                self.cursor_y = self.height - 1;
                            }
                        }
                    } else {
                        if self.cursor_x >= self.width {
                            self.cursor_x = 0;
                            self.cursor_y += 1;
                            if self.cursor_y >= self.height {
                                self.scroll_up();
                                self.cursor_y = self.height - 1;
                            }
                        }
                        self.buffer[self.cursor_y][self.cursor_x] = c;
                        self.cursor_x += 1;
                    }
                }
                _ => {}
            }
        }
    }

    fn scroll_up(&mut self) {
        self.buffer.remove(0);
        self.buffer.push(vec![' '; self.width]);
    }
}

/// Enter raw terminal mode for the simulator
pub fn enter_raw_mode() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        terminal::EnterAlternateScreen,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0),
        cursor::Show,
    )?;
    Ok(())
}

/// Restore terminal to normal mode
pub fn exit_raw_mode() -> io::Result<()> {
    let mut stdout = io::stdout();
    execute!(
        stdout,
        cursor::Show,
        ResetColor,
        terminal::LeaveAlternateScreen,
    )?;
    terminal::disable_raw_mode()?;
    Ok(())
}
