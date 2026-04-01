//! A CC:Tweaked-style text editor (WASI port).
//!
//! This is a simple, mode-less text editor inspired by ComputerCraft:Tweaked.
//! All commands use Ctrl+key shortcuts, and navigation uses arrow keys.
//!
//! Key Bindings:
//! - Arrow keys: Move cursor
//! - Typing: Insert at cursor
//! - Enter: Split line
//! - Backspace: Delete char left / merge with previous line
//! - Delete: Delete char under cursor / merge with next line
//! - Ctrl+E: Exit (prompts if unsaved)
//! - Ctrl+S: Save file
//! - Ctrl+R: Save and exit
//! - Ctrl+A: Select all (for mass delete)
//! - Ctrl+X: Cut current line
//! - Ctrl+C: Copy current line
//! - Ctrl+V: Paste line below cursor
//! - Ctrl+D: Delete current line
//! - Ctrl+K: Clear current line
//! - Ctrl+F: Find (forward search)
//! - Page Up/Down: Scroll by page
//! - Home/End: Move to start/end of line

use std::io::{self, Read, Write};

/// Default terminal dimensions (the host may provide actual size)
const DEFAULT_WIDTH: usize = 80;
const DEFAULT_HEIGHT: usize = 25;

/// Maximum lines in the editor buffer
const MAX_LINES: usize = 500;
/// Maximum characters per line
const MAX_LINE_LEN: usize = 256;

// ---------------------------------------------------------------------------
// ANSI helpers
// ---------------------------------------------------------------------------

fn term_clear() {
    print!("\x1b[2J");
}

fn term_cursor(row: usize, col: usize) {
    // ANSI is 1-based
    print!("\x1b[{};{}H", row + 1, col + 1);
}

fn flush() {
    let _ = io::stdout().flush();
}

// ---------------------------------------------------------------------------
// Prompt modes & exit result
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum PromptMode {
    None,
    SaveConfirm,
    Search,
}

#[derive(Clone, Copy, PartialEq)]
enum ExitResult {
    Continue,
    Exit,
}

// ---------------------------------------------------------------------------
// Editor state
// ---------------------------------------------------------------------------

struct Editor {
    lines: Vec<Vec<u8>>,
    num_lines: usize,
    cursor_x: usize,
    cursor_y: usize,
    scroll_offset: usize,
    filename: String,
    status_message: String,
    modified: bool,
    prompt_mode: PromptMode,
    clipboard: Vec<u8>,
    search_query: Vec<u8>,
    exit_result: ExitResult,
    all_selected: bool,
    term_width: usize,
    term_height: usize,
}

impl Editor {
    fn new() -> Self {
        Self {
            lines: vec![Vec::new()],
            num_lines: 1,
            cursor_x: 0,
            cursor_y: 0,
            scroll_offset: 0,
            filename: String::new(),
            status_message: String::new(),
            modified: false,
            prompt_mode: PromptMode::None,
            clipboard: Vec::new(),
            search_query: Vec::new(),
            exit_result: ExitResult::Continue,
            all_selected: false,
            term_width: DEFAULT_WIDTH,
            term_height: DEFAULT_HEIGHT,
        }
    }

    fn open(&mut self, filename: &str) {
        self.filename = filename.to_string();

        if let Ok(content) = std::fs::read_to_string(filename) {
            self.load_content(&content);
            self.set_status("Opened file");
        } else {
            self.lines = vec![Vec::new()];
            self.num_lines = 1;
            self.set_status("New file");
        }

        self.cursor_x = 0;
        self.cursor_y = 0;
        self.scroll_offset = 0;
        self.modified = false;
    }

    fn load_content(&mut self, content: &str) {
        self.lines.clear();
        for line in content.split('\n') {
            if self.lines.len() >= MAX_LINES {
                break;
            }
            let bytes = line.as_bytes();
            let take = bytes.len().min(MAX_LINE_LEN);
            self.lines.push(bytes[..take].to_vec());
        }
        if self.lines.is_empty() {
            self.lines.push(Vec::new());
        }
        self.num_lines = self.lines.len();
    }

    fn get_content(&self) -> String {
        let mut out = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            if let Ok(s) = std::str::from_utf8(line) {
                out.push_str(s);
            }
            if i < self.num_lines - 1 {
                out.push('\n');
            }
        }
        out
    }

    fn set_status(&mut self, msg: &str) {
        self.status_message = msg.to_string();
    }

    fn should_exit(&self) -> bool {
        self.exit_result != ExitResult::Continue
    }

    // ------------------------------------------------------------------
    // Line number gutter width
    // ------------------------------------------------------------------

    fn gutter_width(&self) -> usize {
        // digits for largest line number + 1 space
        let mut digits = 1usize;
        let mut n = self.num_lines;
        while n >= 10 {
            n /= 10;
            digits += 1;
        }
        digits + 1 // +1 for the space separator
    }

    // ------------------------------------------------------------------
    // Input handling
    // ------------------------------------------------------------------

    fn handle_byte_stream(&mut self, bytes: &[u8]) {
        let was_selected = self.all_selected;

        match self.prompt_mode {
            PromptMode::SaveConfirm => self.handle_save_confirm(bytes),
            PromptMode::Search => self.handle_search_input(bytes),
            PromptMode::None => self.handle_normal_input(bytes, was_selected),
        }
    }

    fn handle_save_confirm(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        match bytes[0] {
            b'y' | b'Y' => {
                self.save_file();
                self.exit_result = ExitResult::Exit;
            }
            b'n' | b'N' => {
                self.exit_result = ExitResult::Exit;
            }
            27 => {
                self.prompt_mode = PromptMode::None;
                self.set_status("");
            }
            _ => {}
        }
    }

    fn handle_search_input(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            match byte {
                27 => {
                    self.prompt_mode = PromptMode::None;
                    self.search_query.clear();
                    self.set_status("");
                    return;
                }
                b'\n' | b'\r' => {
                    self.prompt_mode = PromptMode::None;
                    self.find_next();
                    return;
                }
                8 | 127 => {
                    self.search_query.pop();
                }
                c if c >= 32 && c < 127 => {
                    if self.search_query.len() < 64 {
                        self.search_query.push(c);
                    }
                }
                _ => {}
            }
        }
    }

    fn handle_normal_input(&mut self, bytes: &[u8], was_selected: bool) {
        let mut i = 0;
        while i < bytes.len() {
            let byte = bytes[i];

            // Check for escape sequences
            if byte == 27 && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                if i + 2 < bytes.len() {
                    match bytes[i + 2] {
                        b'A' => { self.move_up(); i += 3; continue; }
                        b'B' => { self.move_down(); i += 3; continue; }
                        b'C' => { self.move_right(); i += 3; continue; }
                        b'D' => { self.move_left(); i += 3; continue; }
                        b'H' => { self.move_home(); i += 3; continue; }
                        b'F' => { self.move_end(); i += 3; continue; }
                        b'3' if i + 3 < bytes.len() && bytes[i + 3] == b'~' => {
                            self.delete_at_cursor();
                            i += 4;
                            continue;
                        }
                        b'5' if i + 3 < bytes.len() && bytes[i + 3] == b'~' => {
                            self.page_up();
                            i += 4;
                            continue;
                        }
                        b'6' if i + 3 < bytes.len() && bytes[i + 3] == b'~' => {
                            self.page_down();
                            i += 4;
                            continue;
                        }
                        b'1' if i + 3 < bytes.len() && bytes[i + 3] == b'~' => {
                            self.move_home();
                            i += 4;
                            continue;
                        }
                        b'4' if i + 3 < bytes.len() && bytes[i + 3] == b'~' => {
                            self.move_end();
                            i += 4;
                            continue;
                        }
                        // Cursor position report: ESC [ row ; col R
                        b'0'..=b'9' => {
                            if let Some((_row, _col, consumed)) = parse_cursor_position(&bytes[i+2..]) {
                                i += 2 + consumed;
                                continue;
                            }
                        }
                        _ => {}
                    }
                }
                // Unknown escape -- skip ESC
                i += 1;
                continue;
            }

            // Control characters / printable
            match byte {
                1 => {
                    // Ctrl+A
                    self.all_selected = true;
                    self.set_status("All text selected. Press Backspace/Delete to clear.");
                }
                3 => { self.copy_line(); }
                4 => { self.delete_line(); }
                5 => { self.try_exit(); }
                6 => { self.start_search(); }
                11 => { self.clear_line(); }
                18 => {
                    // Ctrl+R - save and exit
                    self.save_file();
                    self.exit_result = ExitResult::Exit;
                }
                19 => { self.save_file(); }
                22 => { self.paste_line(); }
                24 => { self.cut_line(); }
                b'\n' | b'\r' => {
                    if was_selected { self.delete_all(); }
                    self.insert_newline();
                }
                8 | 127 => {
                    if was_selected { self.delete_all(); } else { self.backspace(); }
                }
                b'\t' => {
                    for _ in 0..4 { self.insert_char(' '); }
                }
                c if c >= 32 && c < 127 => {
                    if was_selected { self.delete_all(); }
                    self.all_selected = false;
                    self.insert_char(c as char);
                }
                _ => {}
            }
            i += 1;
        }
    }

    // ------------------------------------------------------------------
    // Cursor movement
    // ------------------------------------------------------------------

    fn move_up(&mut self) {
        self.all_selected = false;
        if self.cursor_y > 0 {
            self.cursor_y -= 1;
            self.clamp_cursor_x();
            self.ensure_cursor_visible();
        }
    }

    fn move_down(&mut self) {
        self.all_selected = false;
        if self.cursor_y < self.num_lines - 1 {
            self.cursor_y += 1;
            self.clamp_cursor_x();
            self.ensure_cursor_visible();
        }
    }

    fn move_left(&mut self) {
        self.all_selected = false;
        if self.cursor_x > 0 {
            self.cursor_x -= 1;
        }
    }

    fn move_right(&mut self) {
        self.all_selected = false;
        let line_len = self.lines[self.cursor_y].len();
        if self.cursor_x < line_len {
            self.cursor_x += 1;
        }
    }

    fn move_home(&mut self) {
        self.all_selected = false;
        self.cursor_x = 0;
    }

    fn move_end(&mut self) {
        self.all_selected = false;
        self.cursor_x = self.lines[self.cursor_y].len();
    }

    fn page_up(&mut self) {
        self.all_selected = false;
        let visible = self.visible_lines();
        if self.cursor_y >= visible {
            self.cursor_y -= visible;
        } else {
            self.cursor_y = 0;
        }
        self.clamp_cursor_x();
        self.ensure_cursor_visible();
    }

    fn page_down(&mut self) {
        self.all_selected = false;
        let visible = self.visible_lines();
        self.cursor_y = (self.cursor_y + visible).min(self.num_lines - 1);
        self.clamp_cursor_x();
        self.ensure_cursor_visible();
    }

    fn clamp_cursor_x(&mut self) {
        let line_len = self.lines[self.cursor_y].len();
        if self.cursor_x > line_len {
            self.cursor_x = line_len;
        }
    }

    fn visible_lines(&self) -> usize {
        self.term_height.saturating_sub(2) // status bar + command line
    }

    fn ensure_cursor_visible(&mut self) {
        let visible = self.visible_lines();
        if self.cursor_y < self.scroll_offset {
            self.scroll_offset = self.cursor_y;
        } else if self.cursor_y >= self.scroll_offset + visible {
            self.scroll_offset = self.cursor_y - visible + 1;
        }
    }

    // ------------------------------------------------------------------
    // Text editing
    // ------------------------------------------------------------------

    fn insert_char(&mut self, c: char) {
        let line = &mut self.lines[self.cursor_y];
        if line.len() >= MAX_LINE_LEN - 1 {
            return;
        }
        line.insert(self.cursor_x, c as u8);
        self.cursor_x += 1;
        self.modified = true;
    }

    fn backspace(&mut self) {
        if self.cursor_x > 0 {
            self.lines[self.cursor_y].remove(self.cursor_x - 1);
            self.cursor_x -= 1;
            self.modified = true;
        } else if self.cursor_y > 0 {
            let curr_line = self.lines.remove(self.cursor_y);
            self.num_lines -= 1;
            self.cursor_y -= 1;
            let prev_len = self.lines[self.cursor_y].len();
            if prev_len + curr_line.len() <= MAX_LINE_LEN {
                self.lines[self.cursor_y].extend_from_slice(&curr_line);
                self.cursor_x = prev_len;
                self.ensure_cursor_visible();
                self.modified = true;
            } else {
                // Re-insert if it would exceed limit
                self.cursor_y += 1;
                self.lines.insert(self.cursor_y, curr_line);
                self.num_lines += 1;
            }
        }
    }

    fn delete_at_cursor(&mut self) {
        let line_len = self.lines[self.cursor_y].len();
        if self.cursor_x < line_len {
            self.lines[self.cursor_y].remove(self.cursor_x);
            self.modified = true;
        } else if self.cursor_y < self.num_lines - 1 {
            let next_line = self.lines.remove(self.cursor_y + 1);
            self.num_lines -= 1;
            let curr_len = self.lines[self.cursor_y].len();
            if curr_len + next_line.len() <= MAX_LINE_LEN {
                self.lines[self.cursor_y].extend_from_slice(&next_line);
                self.modified = true;
            } else {
                // Re-insert if too long
                self.lines.insert(self.cursor_y + 1, next_line);
                self.num_lines += 1;
            }
        }
    }

    fn insert_newline(&mut self) {
        if self.num_lines >= MAX_LINES {
            return;
        }
        let remaining = self.lines[self.cursor_y].split_off(self.cursor_x);
        self.lines.insert(self.cursor_y + 1, remaining);
        self.num_lines += 1;
        self.cursor_y += 1;
        self.cursor_x = 0;
        self.ensure_cursor_visible();
        self.modified = true;
    }

    // ------------------------------------------------------------------
    // Line operations
    // ------------------------------------------------------------------

    fn copy_line(&mut self) {
        self.clipboard = self.lines[self.cursor_y].clone();
        self.set_status("Line copied");
    }

    fn cut_line(&mut self) {
        self.copy_line();
        self.delete_line();
        self.set_status("Line cut");
    }

    fn paste_line(&mut self) {
        if self.clipboard.is_empty() {
            self.set_status("Clipboard empty");
            return;
        }
        if self.num_lines >= MAX_LINES {
            return;
        }
        self.lines.insert(self.cursor_y + 1, self.clipboard.clone());
        self.num_lines += 1;
        self.cursor_y += 1;
        self.cursor_x = 0;
        self.ensure_cursor_visible();
        self.modified = true;
        self.set_status("Line pasted");
    }

    fn delete_line(&mut self) {
        if self.num_lines <= 1 {
            self.lines[0].clear();
            self.cursor_x = 0;
        } else {
            self.lines.remove(self.cursor_y);
            self.num_lines -= 1;
            if self.cursor_y >= self.num_lines {
                self.cursor_y = self.num_lines - 1;
            }
            self.clamp_cursor_x();
        }
        self.modified = true;
        self.set_status("Line deleted");
    }

    fn clear_line(&mut self) {
        self.lines[self.cursor_y].clear();
        self.cursor_x = 0;
        self.modified = true;
        self.set_status("Line cleared");
    }

    fn delete_all(&mut self) {
        self.lines = vec![Vec::new()];
        self.num_lines = 1;
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.scroll_offset = 0;
        self.all_selected = false;
        self.modified = true;
        self.set_status("All text deleted");
    }

    // ------------------------------------------------------------------
    // File operations
    // ------------------------------------------------------------------

    fn save_file(&mut self) {
        if self.filename.is_empty() {
            self.set_status("No filename");
            return;
        }
        let content = self.get_content();
        match std::fs::write(&self.filename, &content) {
            Ok(_) => {
                self.set_status("Saved");
                self.modified = false;
            }
            Err(_) => {
                self.set_status("Error saving file");
            }
        }
    }

    fn try_exit(&mut self) {
        if self.modified {
            self.prompt_mode = PromptMode::SaveConfirm;
            self.set_status("Save changes? (y/n)");
        } else {
            self.exit_result = ExitResult::Exit;
        }
    }

    // ------------------------------------------------------------------
    // Search
    // ------------------------------------------------------------------

    fn start_search(&mut self) {
        self.prompt_mode = PromptMode::Search;
        self.search_query.clear();
        self.set_status("Find: ");
    }

    fn find_next(&mut self) {
        if self.search_query.is_empty() {
            self.set_status("No search term");
            return;
        }

        let query = self.search_query.clone();
        let start_y = self.cursor_y;
        let start_x = self.cursor_x + 1;

        // Search current line after cursor
        if let Some(pos) = find_in_slice(&self.lines[start_y], start_x, &query) {
            self.cursor_x = pos;
            self.set_status("Found");
            return;
        }

        // Search subsequent lines
        for y in start_y + 1..self.num_lines {
            if let Some(pos) = find_in_slice(&self.lines[y], 0, &query) {
                self.cursor_y = y;
                self.cursor_x = pos;
                self.ensure_cursor_visible();
                self.set_status("Found");
                return;
            }
        }

        // Wrap around
        for y in 0..=start_y {
            let end = if y == start_y { start_x } else { self.lines[y].len() };
            if let Some(pos) = find_in_slice_range(&self.lines[y], 0, end, &query) {
                self.cursor_y = y;
                self.cursor_x = pos;
                self.ensure_cursor_visible();
                self.set_status("Found (wrapped)");
                return;
            }
        }

        self.set_status("Not found");
    }

    // ------------------------------------------------------------------
    // Rendering
    // ------------------------------------------------------------------

    fn render(&self) {
        let mut out = String::with_capacity(4096);
        let width = self.term_width;
        let visible = self.visible_lines();
        let gutter_w = self.gutter_width();

        // Clear screen and go home
        out.push_str("\x1b[2J\x1b[1;1H");

        // Render visible lines
        for screen_y in 0..visible {
            let buffer_y = self.scroll_offset + screen_y;

            // Position cursor at start of this screen row
            out.push_str(&format!("\x1b[{};1H", screen_y + 1));

            if buffer_y < self.num_lines {
                // Line number (right-aligned in gutter)
                let line_num = format!("{:>width$} ", buffer_y + 1, width = gutter_w - 1);
                // Dim color for line numbers
                out.push_str("\x1b[90m");
                out.push_str(&line_num);
                out.push_str("\x1b[0m");

                let line = &self.lines[buffer_y];
                if let Ok(s) = std::str::from_utf8(line) {
                    let avail = width.saturating_sub(gutter_w);
                    let display_len = s.len().min(avail);
                    out.push_str(&s[..display_len]);
                }
            } else {
                // Dim tilde for lines past end of file
                out.push_str("\x1b[90m");
                out.push('~');
                out.push_str("\x1b[0m");
            }

            // Clear rest of line
            out.push_str("\x1b[K");
        }

        // Status bar (reverse video)
        out.push_str(&format!("\x1b[{};1H", visible + 1));
        out.push_str("\x1b[7m"); // reverse video

        let mut status = String::new();
        if self.filename.is_empty() {
            status.push_str("[No Name]");
        } else {
            status.push_str(&self.filename);
        }
        if self.modified {
            status.push_str(" [+]");
        }
        if self.all_selected {
            status.push_str(" [ALL]");
        }
        status.push_str(&format!(
            " - L{}/{} C{}",
            self.cursor_y + 1,
            self.num_lines,
            self.cursor_x + 1
        ));

        // Pad to full width
        let pad = if status.len() < width { width - status.len() } else { 0 };
        out.push_str(&status);
        for _ in 0..pad {
            out.push(' ');
        }
        out.push_str("\x1b[0m"); // reset

        // Command / status line
        out.push_str(&format!("\x1b[{};1H\x1b[K", visible + 2));

        match self.prompt_mode {
            PromptMode::SaveConfirm => {
                out.push_str("Save changes? (y/n)");
            }
            PromptMode::Search => {
                out.push_str("Find: ");
                if let Ok(s) = std::str::from_utf8(&self.search_query) {
                    out.push_str(s);
                }
            }
            PromptMode::None => {
                if !self.status_message.is_empty() {
                    out.push_str(&self.status_message);
                } else {
                    out.push_str("Ctrl+E:Exit  Ctrl+S:Save  Ctrl+F:Find");
                }
            }
        }

        // Position cursor
        let screen_cursor_y = self.cursor_y - self.scroll_offset;
        let screen_cursor_x = self.cursor_x + gutter_w;
        out.push_str(&format!(
            "\x1b[{};{}H",
            screen_cursor_y + 1,
            screen_cursor_x + 1
        ));

        print!("{}", out);
        flush();
    }
}

// ---------------------------------------------------------------------------
// Search helpers
// ---------------------------------------------------------------------------

fn find_in_slice(line: &[u8], start: usize, query: &[u8]) -> Option<usize> {
    find_in_slice_range(line, start, line.len(), query)
}

fn find_in_slice_range(line: &[u8], start: usize, end: usize, query: &[u8]) -> Option<usize> {
    let end = end.min(line.len());
    if query.is_empty() || end < query.len() {
        return None;
    }
    for x in start..=end.saturating_sub(query.len()) {
        if &line[x..x + query.len()] == query {
            return Some(x);
        }
    }
    None
}

/// Parses a cursor position report: digits ; digits R
/// Returns (row, col, bytes_consumed) or None.
fn parse_cursor_position(bytes: &[u8]) -> Option<(usize, usize, usize)> {
    let mut i = 0;
    let mut row: usize = 0;
    let mut col: usize = 0;

    while i < bytes.len() && bytes[i].is_ascii_digit() {
        row = row * 10 + (bytes[i] - b'0') as usize;
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b';' {
        return None;
    }
    i += 1;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        col = col * 10 + (bytes[i] - b'0') as usize;
        i += 1;
    }
    // Accept both 'R' (cursor position report) and 'H' (set cursor)
    if i >= bytes.len() || (bytes[i] != b'R' && bytes[i] != b'H') {
        return None;
    }
    i += 1;
    Some((row, col, i))
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: edit <filename>");
        std::process::exit(1);
    }

    let mut editor = Editor::new();

    // Check for terminal size from environment (host may set these)
    if let Ok(val) = std::env::var("COLUMNS") {
        if let Ok(w) = val.parse::<usize>() {
            if w > 0 { editor.term_width = w; }
        }
    }
    if let Ok(val) = std::env::var("LINES") {
        if let Ok(h) = val.parse::<usize>() {
            if h > 0 { editor.term_height = h; }
        }
    }

    editor.open(&args[1]);
    editor.render();

    let stdin = io::stdin();
    let mut buf = [0u8; 64];

    loop {
        match stdin.lock().read(&mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => {
                editor.handle_byte_stream(&buf[..n]);
                if editor.should_exit() {
                    break;
                }
                editor.render();
            }
            Err(_) => break,
        }
    }

    // Clear screen on exit
    term_clear();
    term_cursor(0, 0);
    flush();
}
