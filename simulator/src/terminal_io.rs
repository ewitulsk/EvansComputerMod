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

/// Graphics framebuffer constants (must match operating-system/rust/src/gfx.rs)
pub const GFX_BASE: usize = 0x30000;
pub const GFX_HEADER_SIZE: usize = 64;
pub const GFX_PALETTE_OFF: usize = 0x40;
pub const GFX_PIXEL_OFF: usize = 0x400;
pub const GFX_MAGIC: u16 = 0xFB02;

// GFX header offsets from GFX_BASE
const GFX_OFF_MAGIC: usize = 0x00;
const GFX_OFF_MODE: usize = 0x02;
const GFX_OFF_WIDTH: usize = 0x04;
const GFX_OFF_HEIGHT: usize = 0x06;
const GFX_OFF_PALETTE_DIRTY: usize = 0x08;
const GFX_OFF_PIXEL_DIRTY: usize = 0x0C;

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
    pub last_dirty_counter: u32,
    /// Shadow copy of cell data for diffing (optimization).
    prev_cells: Vec<u8>,
    /// Previous cursor position for diff rendering.
    prev_cursor: (u16, u16),
    /// Headless mode: write raw text to stdout.
    pub headless: bool,
    // Graphics state
    pub display_mode: u8,
    pub gfx_width: usize,
    pub gfx_height: usize,
    pub last_palette_dirty: u32,
    pub last_pixel_dirty: u32,
    prev_palette: Vec<u8>,
    prev_pixels: Vec<u8>,
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
            display_mode: 0,
            gfx_width: 0,
            gfx_height: 0,
            last_palette_dirty: u32::MAX,
            last_pixel_dirty: u32::MAX,
            prev_palette: vec![0u8; 768],
            prev_pixels: Vec::new(),
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

        // Check if graphics dirty counters changed (even if text didn't)
        let gfx_changed = self.display_mode > 0 && (
            self.last_palette_dirty != self.last_palette_dirty ||
            self.last_pixel_dirty != self.last_pixel_dirty
        );

        if dirty_counter == self.last_dirty_counter && !gfx_changed && self.display_mode == 0 {
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
            // Headless mode: just dump the text content (skip graphics)
            if self.display_mode != 1 {
                self.render_headless(cells, width, height)?;
            }
        } else {
            match self.display_mode {
                0 => {
                    // Text-only mode
                    self.render_interactive(cells, width, height, cursor_x, cursor_y, cursor_visible)?;
                }
                1 => {
                    // Graphics-only mode: render pixels using half-block characters
                    self.render_gfx_halfblock()?;
                }
                2 => {
                    // Overlay mode: render graphics first, then text on top
                    self.render_gfx_halfblock()?;
                    // Re-render non-empty text cells on top
                    self.render_text_overlay(cells, width, height, cursor_x, cursor_y, cursor_visible)?;
                }
                _ => {
                    self.render_interactive(cells, width, height, cursor_x, cursor_y, cursor_visible)?;
                }
            }
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

    /// Read the graphics framebuffer from a WASM memory slice.
    /// Call this before render_from_fb_slice to update display_mode and pixel data.
    pub fn update_gfx_state(&mut self, gfx_data: &[u8]) {
        if gfx_data.len() < GFX_HEADER_SIZE {
            return;
        }

        let magic = read_u16(gfx_data, GFX_OFF_MAGIC);
        if magic != GFX_MAGIC {
            self.display_mode = 0;
            return;
        }

        let mode = gfx_data[GFX_OFF_MODE];
        let gfx_w = read_u16(gfx_data, GFX_OFF_WIDTH) as usize;
        let gfx_h = read_u16(gfx_data, GFX_OFF_HEIGHT) as usize;
        let palette_dirty = read_u32(gfx_data, GFX_OFF_PALETTE_DIRTY);
        let pixel_dirty = read_u32(gfx_data, GFX_OFF_PIXEL_DIRTY);

        self.display_mode = mode;
        self.gfx_width = gfx_w;
        self.gfx_height = gfx_h;
        self.last_palette_dirty = palette_dirty;
        self.last_pixel_dirty = pixel_dirty;

        // Read palette (768 bytes at offset 0x40)
        let palette_start = GFX_PALETTE_OFF;
        let palette_end = palette_start + 768;
        if gfx_data.len() >= palette_end {
            self.prev_palette[..768].copy_from_slice(&gfx_data[palette_start..palette_end]);
        }

        // Read pixel data
        let pixel_start = GFX_PIXEL_OFF;
        let pixel_count = gfx_w * gfx_h;
        let pixel_end = pixel_start + pixel_count;
        if gfx_data.len() >= pixel_end {
            self.prev_pixels = gfx_data[pixel_start..pixel_end].to_vec();
        }
    }

    /// Render graphics using half-block Unicode characters (▀).
    /// Each terminal cell represents 2 vertical pixels:
    /// - foreground color = top pixel
    /// - background color = bottom pixel
    fn render_gfx_halfblock(&self) -> io::Result<()> {
        if self.prev_pixels.is_empty() || self.gfx_width == 0 || self.gfx_height == 0 {
            return Ok(());
        }

        let mut stdout = io::stdout();
        let (term_cols, term_rows) = terminal::size().unwrap_or((80, 24));

        // Scale down if graphics don't fit in terminal
        // Each terminal cell = 1 pixel wide, 2 pixels tall (using half-blocks)
        let render_cols = self.gfx_width.min(term_cols as usize);
        let render_rows = (self.gfx_height / 2).min(term_rows as usize);

        for ty in 0..render_rows {
            execute!(stdout, cursor::MoveTo(0, ty as u16))?;

            let top_y = ty * 2;
            let bot_y = top_y + 1;

            for tx in 0..render_cols {
                // Get top and bottom pixel colors
                let top_idx = if top_y < self.gfx_height {
                    self.prev_pixels[top_y * self.gfx_width + tx] as usize
                } else {
                    0
                };
                let bot_idx = if bot_y < self.gfx_height {
                    self.prev_pixels[bot_y * self.gfx_width + tx] as usize
                } else {
                    0
                };

                let top_color = palette_to_crossterm(&self.prev_palette, top_idx);
                let bot_color = palette_to_crossterm(&self.prev_palette, bot_idx);

                execute!(
                    stdout,
                    SetForegroundColor(top_color),
                    SetBackgroundColor(bot_color),
                    Print('▀')
                )?;
            }
        }

        execute!(stdout, ResetColor, cursor::Hide)?;
        stdout.flush()?;
        Ok(())
    }

    /// Render text overlay on top of graphics (for mode 2).
    /// Only renders non-empty cells (character != space or bg != 0).
    fn render_text_overlay(
        &mut self,
        cells: &[u8],
        width: usize,
        height: usize,
        cursor_x: u16,
        cursor_y: u16,
        cursor_visible: bool,
    ) -> io::Result<()> {
        let mut stdout = io::stdout();
        let (term_cols, term_rows) = terminal::size().unwrap_or((80, 24));
        let render_width = width.min(term_cols as usize);
        let render_height = height.min(term_rows as usize);

        for y in 0..render_height {
            for x in 0..render_width {
                let off = (y * width + x) * CELL_SIZE;
                let ch = cells[off];
                let attr = cells[off + 1];
                let bg_idx = ((attr >> 4) & 0x0F) as usize;

                // Only draw if character is visible (not space) or has a non-black background
                if (ch > 0x20 && ch < 0x7F) || bg_idx != 0 {
                    let fg_idx = (attr & 0x0F) as usize;
                    let fg = PALETTE[fg_idx];
                    let bg = PALETTE[bg_idx];
                    let display_char = if ch >= 0x20 && ch < 0x7F { ch as char } else { ' ' };

                    execute!(
                        stdout,
                        cursor::MoveTo(x as u16, y as u16),
                        SetForegroundColor(fg),
                        SetBackgroundColor(bg),
                        Print(display_char)
                    )?;
                }
            }
        }

        execute!(stdout, ResetColor)?;
        if cursor_visible {
            execute!(
                stdout,
                cursor::MoveTo(
                    cursor_x.min(render_width as u16 - 1),
                    cursor_y.min(render_height as u16 - 1)
                ),
                cursor::Show,
            )?;
        }
        stdout.flush()?;
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

/// Convert a 256-color palette entry to a crossterm Color.
fn palette_to_crossterm(palette: &[u8], idx: usize) -> Color {
    let base = idx * 3;
    if base + 2 < palette.len() {
        Color::Rgb {
            r: palette[base],
            g: palette[base + 1],
            b: palette[base + 2],
        }
    } else {
        Color::Rgb { r: 0, g: 0, b: 0 }
    }
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
