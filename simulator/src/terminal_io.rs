use std::io::{self, Write};
use crossterm::{
    cursor,
    execute,
    style::Print,
    terminal::{self, ClearType},
};

pub const DEFAULT_WIDTH: usize = 80;
pub const DEFAULT_HEIGHT: usize = 24;

pub struct TerminalBuffer {
    pub buffer: Vec<Vec<char>>,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pub width: usize,
    pub height: usize,
    /// Whether we need to re-render
    dirty: bool,
    /// Headless mode: write directly to stdout, no crossterm positioning
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
            dirty: true,
            headless: false,
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
        self.dirty = true;
    }

    pub fn set_cursor(&mut self, x: i32, y: i32) {
        self.cursor_x = (x as usize).min(self.width.saturating_sub(1));
        self.cursor_y = (y as usize).min(self.height.saturating_sub(1));
        self.dirty = true;
    }

    pub fn write_text(&mut self, text: &str) {
        if self.headless {
            // In headless mode, just write directly to stdout
            let _ = io::stdout().write_all(text.as_bytes());
            let _ = io::stdout().flush();
        }
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
                    // Backspace
                    if self.cursor_x > 0 {
                        self.cursor_x -= 1;
                    }
                }
                c if c >= ' ' || c == '\t' => {
                    if c == '\t' {
                        // Tab: advance to next 8-column boundary
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
                _ => {
                    // Ignore other control chars
                }
            }
        }
        self.dirty = true;
    }

    fn scroll_up(&mut self) {
        self.buffer.remove(0);
        self.buffer.push(vec![' '; self.width]);
    }

    /// Render the terminal buffer to the real terminal
    pub fn render(&mut self) -> io::Result<()> {
        if self.headless || !self.dirty {
            return Ok(());
        }
        self.dirty = false;

        let mut stdout = io::stdout();
        // Move to top-left
        execute!(stdout, cursor::MoveTo(0, 0))?;

        for (y, row) in self.buffer.iter().enumerate() {
            let line: String = row.iter().collect();
            execute!(
                stdout,
                cursor::MoveTo(0, y as u16),
                Print(&line),
            )?;
        }

        // Move cursor to logical position
        execute!(
            stdout,
            cursor::MoveTo(self.cursor_x as u16, self.cursor_y as u16)
        )?;
        stdout.flush()?;
        Ok(())
    }

    /// Simple render: just flush what terminal_write outputs directly
    /// Used in Phase 1 before full buffer rendering
    pub fn render_simple(&self) -> io::Result<()> {
        io::stdout().flush()?;
        Ok(())
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
        terminal::LeaveAlternateScreen,
    )?;
    terminal::disable_raw_mode()?;
    Ok(())
}
