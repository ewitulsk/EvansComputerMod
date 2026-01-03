//! A CC:Tweaked-style text editor.
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
//! - Ctrl+R: Save and exit with "run" flag
//! - Ctrl+A: Select all (for mass delete)
//! - Ctrl+X: Cut current line
//! - Ctrl+C: Copy current line
//! - Ctrl+V: Paste line below cursor
//! - Ctrl+D: Delete current line
//! - Ctrl+K: Clear current line
//! - Ctrl+F: Find (forward search)

use crate::fs;
use crate::terminal::{print, println, clear, set_cursor, get_width, get_height};

/// Maximum lines in the editor buffer
const MAX_LINES: usize = 500;
/// Maximum characters per line
const MAX_LINE_LEN: usize = 256;

/// Editor prompt modes
#[derive(Clone, Copy, PartialEq)]
pub enum PromptMode {
    /// Normal editing
    None,
    /// Asking "Save changes? (y/n)"
    SaveConfirm,
    /// Asking for search term
    Search,
}

/// Result of editor exit
#[derive(Clone, Copy, PartialEq)]
pub enum ExitResult {
    /// Editor should continue running
    Continue,
    /// Exit normally
    Exit,
    /// Exit and run the file
    ExitAndRun,
}

/// The editor state
pub struct Editor {
    /// Text buffer - array of lines
    lines: [[u8; MAX_LINE_LEN]; MAX_LINES],
    /// Length of each line
    line_lengths: [usize; MAX_LINES],
    /// Number of lines in the buffer
    num_lines: usize,
    /// Cursor X position (column)
    cursor_x: usize,
    /// Cursor Y position (row in buffer)
    cursor_y: usize,
    /// Scroll offset (first visible line)
    scroll_offset: usize,
    /// Current filename
    filename: [u8; 64],
    /// Length of filename
    filename_len: usize,
    /// Status message
    status_message: [u8; 80],
    /// Status message length
    status_len: usize,
    /// Whether the buffer has been modified
    modified: bool,
    /// Current prompt mode
    prompt_mode: PromptMode,
    /// Clipboard (single line)
    clipboard: [u8; MAX_LINE_LEN],
    /// Clipboard length
    clipboard_len: usize,
    /// Search query buffer
    search_query: [u8; 64],
    /// Search query length
    search_query_len: usize,
    /// Exit result (set when editor should exit)
    exit_result: ExitResult,
    /// Whether all text is selected (for Ctrl+A)
    all_selected: bool,
}

impl Editor {
    /// Creates a new editor instance
    pub fn new() -> Self {
        let mut editor = Self {
            lines: [[0u8; MAX_LINE_LEN]; MAX_LINES],
            line_lengths: [0; MAX_LINES],
            num_lines: 1,
            cursor_x: 0,
            cursor_y: 0,
            scroll_offset: 0,
            filename: [0u8; 64],
            filename_len: 0,
            status_message: [0u8; 80],
            status_len: 0,
            modified: false,
            prompt_mode: PromptMode::None,
            clipboard: [0u8; MAX_LINE_LEN],
            clipboard_len: 0,
            search_query: [0u8; 64],
            search_query_len: 0,
            exit_result: ExitResult::Continue,
            all_selected: false,
        };
        editor.line_lengths[0] = 0;
        editor
    }

    /// Opens a file for editing
    pub fn open(&mut self, filename: &str) {
        // Store filename
        let name_bytes = filename.as_bytes();
        let copy_len = name_bytes.len().min(self.filename.len());
        self.filename[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
        self.filename_len = copy_len;

        // Try to read the file
        if let Some(content) = fs::read_file(filename) {
            self.load_content(content);
            self.set_status("Opened file");
        } else {
            // New file
            self.num_lines = 1;
            self.line_lengths[0] = 0;
            self.set_status("New file");
        }

        self.cursor_x = 0;
        self.cursor_y = 0;
        self.scroll_offset = 0;
        self.modified = false;
    }

    /// Loads content into the editor buffer
    fn load_content(&mut self, content: &str) {
        self.num_lines = 0;

        for line in content.split('\n') {
            if self.num_lines >= MAX_LINES {
                break;
            }

            let bytes = line.as_bytes();
            let copy_len = bytes.len().min(MAX_LINE_LEN);
            self.lines[self.num_lines][..copy_len].copy_from_slice(&bytes[..copy_len]);
            self.line_lengths[self.num_lines] = copy_len;
            self.num_lines += 1;
        }

        if self.num_lines == 0 {
            self.num_lines = 1;
            self.line_lengths[0] = 0;
        }
    }

    /// Gets the buffer content as a string for saving
    fn get_content(&self) -> &str {
        static mut SAVE_BUFFER: [u8; 65536] = [0u8; 65536];

        unsafe {
            let mut offset = 0;
            for i in 0..self.num_lines {
                let line_len = self.line_lengths[i];
                if offset + line_len + 1 > SAVE_BUFFER.len() {
                    break;
                }

                SAVE_BUFFER[offset..offset + line_len].copy_from_slice(&self.lines[i][..line_len]);
                offset += line_len;

                if i < self.num_lines - 1 {
                    SAVE_BUFFER[offset] = b'\n';
                    offset += 1;
                }
            }

            core::str::from_utf8(&SAVE_BUFFER[..offset]).unwrap_or("")
        }
    }

    /// Sets the status message
    fn set_status(&mut self, msg: &str) {
        let bytes = msg.as_bytes();
        let copy_len = bytes.len().min(self.status_message.len());
        self.status_message[..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.status_len = copy_len;
    }

    /// Returns the current filename as a string
    fn get_filename(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.filename[..self.filename_len]) }
    }

    /// Checks if the editor should exit
    pub fn should_exit(&self) -> bool {
        self.exit_result != ExitResult::Continue
    }

    /// Gets the exit result
    pub fn get_exit_result(&self) -> ExitResult {
        self.exit_result
    }

    /// Gets the filename for running
    pub fn get_run_filename(&self) -> &str {
        self.get_filename()
    }

    /// Handles input from the terminal
    pub fn handle_input(&mut self, input: &str) {
        // Clear selection on any input (except if we're handling the delete)
        let was_selected = self.all_selected;
        
        match self.prompt_mode {
            PromptMode::SaveConfirm => self.handle_save_confirm(input),
            PromptMode::Search => self.handle_search_input(input),
            PromptMode::None => self.handle_normal_input(input, was_selected),
        }

        if !self.should_exit() {
            self.render();
        }
    }

    /// Handles input during save confirmation prompt
    fn handle_save_confirm(&mut self, input: &str) {
        let bytes = input.as_bytes();
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
                // Escape - cancel
                self.prompt_mode = PromptMode::None;
                self.set_status("");
            }
            _ => {}
        }
    }

    /// Handles input during search prompt
    fn handle_search_input(&mut self, input: &str) {
        let bytes = input.as_bytes();
        
        for &byte in bytes {
            match byte {
                27 => {
                    // Escape - cancel search
                    self.prompt_mode = PromptMode::None;
                    self.search_query_len = 0;
                    self.set_status("");
                    return;
                }
                b'\n' | b'\r' => {
                    // Enter - perform search
                    self.prompt_mode = PromptMode::None;
                    self.find_next();
                    return;
                }
                8 | 127 => {
                    // Backspace
                    if self.search_query_len > 0 {
                        self.search_query_len -= 1;
                    }
                }
                _ if byte >= 32 && byte < 127 => {
                    // Printable character
                    if self.search_query_len < self.search_query.len() {
                        self.search_query[self.search_query_len] = byte;
                        self.search_query_len += 1;
                    }
                }
                _ => {}
            }
        }
    }

    /// Handles normal editing input
    fn handle_normal_input(&mut self, input: &str, was_selected: bool) {
        let bytes = input.as_bytes();
        let mut i = 0;

        while i < bytes.len() {
            let byte = bytes[i];

            // Check for escape sequences
            if byte == 27 && i + 2 < bytes.len() && bytes[i + 1] == b'[' {
                // ANSI escape sequence
                match bytes[i + 2] {
                    b'A' => {
                        // Arrow Up
                        self.move_up();
                        i += 3;
                        continue;
                    }
                    b'B' => {
                        // Arrow Down
                        self.move_down();
                        i += 3;
                        continue;
                    }
                    b'C' => {
                        // Arrow Right
                        self.move_right();
                        i += 3;
                        continue;
                    }
                    b'D' => {
                        // Arrow Left
                        self.move_left();
                        i += 3;
                        continue;
                    }
                    b'3' if i + 3 < bytes.len() && bytes[i + 3] == b'~' => {
                        // Delete key
                        self.delete_at_cursor();
                        i += 4;
                        continue;
                    }
                    _ => {}
                }
            }

            // Handle control characters
            match byte {
                1 => {
                    // Ctrl+A - Select All
                    self.all_selected = true;
                    self.set_status("All text selected. Press Backspace/Delete to clear.");
                }
                3 => {
                    // Ctrl+C - Copy line
                    self.copy_line();
                }
                4 => {
                    // Ctrl+D - Delete line
                    self.delete_line();
                }
                5 => {
                    // Ctrl+E - Exit
                    self.try_exit();
                }
                6 => {
                    // Ctrl+F - Find
                    self.start_search();
                }
                11 => {
                    // Ctrl+K - Clear line
                    self.clear_line();
                }
                18 => {
                    // Ctrl+R - Save and Run
                    self.save_file();
                    self.exit_result = ExitResult::ExitAndRun;
                }
                19 => {
                    // Ctrl+S - Save
                    self.save_file();
                }
                22 => {
                    // Ctrl+V - Paste
                    self.paste_line();
                }
                24 => {
                    // Ctrl+X - Cut line
                    self.cut_line();
                }
                b'\n' | b'\r' => {
                    // Enter - insert newline
                    if was_selected {
                        self.delete_all();
                    }
                    self.insert_newline();
                }
                8 | 127 => {
                    // Backspace
                    if was_selected {
                        self.delete_all();
                    } else {
                        self.backspace();
                    }
                }
                b'\t' => {
                    // Tab - insert spaces
                    for _ in 0..4 {
                        self.insert_char(' ');
                    }
                }
                _ if byte >= 32 && byte < 127 => {
                    // Printable character
                    if was_selected {
                        self.delete_all();
                    }
                    self.all_selected = false;
                    self.insert_char(byte as char);
                }
                _ => {}
            }

            i += 1;
        }
    }

    // === Cursor Movement ===

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
        let line_len = self.line_lengths[self.cursor_y];
        if self.cursor_x < line_len {
            self.cursor_x += 1;
        }
    }

    fn clamp_cursor_x(&mut self) {
        let line_len = self.line_lengths[self.cursor_y];
        if self.cursor_x > line_len {
            self.cursor_x = line_len;
        }
    }

    fn ensure_cursor_visible(&mut self) {
        let visible_lines = (get_height() as usize).saturating_sub(2);

        if self.cursor_y < self.scroll_offset {
            self.scroll_offset = self.cursor_y;
        } else if self.cursor_y >= self.scroll_offset + visible_lines {
            self.scroll_offset = self.cursor_y - visible_lines + 1;
        }
    }

    // === Text Editing ===

    fn insert_char(&mut self, c: char) {
        let line_len = self.line_lengths[self.cursor_y];
        if line_len >= MAX_LINE_LEN - 1 {
            return;
        }

        // Shift characters to the right
        let line = &mut self.lines[self.cursor_y];
        for i in (self.cursor_x..line_len).rev() {
            line[i + 1] = line[i];
        }

        // Insert the new character
        line[self.cursor_x] = c as u8;
        self.line_lengths[self.cursor_y] += 1;
        self.cursor_x += 1;
        self.modified = true;
    }

    fn backspace(&mut self) {
        if self.cursor_x > 0 {
            // Delete character before cursor
            let line = &mut self.lines[self.cursor_y];
            let line_len = self.line_lengths[self.cursor_y];

            for i in self.cursor_x - 1..line_len - 1 {
                line[i] = line[i + 1];
            }

            self.line_lengths[self.cursor_y] -= 1;
            self.cursor_x -= 1;
            self.modified = true;
        } else if self.cursor_y > 0 {
            // Merge with previous line
            let prev_len = self.line_lengths[self.cursor_y - 1];
            let curr_len = self.line_lengths[self.cursor_y];

            if prev_len + curr_len <= MAX_LINE_LEN {
                // Copy current line to end of previous
                let (prev, curr) = self.lines.split_at_mut(self.cursor_y);
                prev[self.cursor_y - 1][prev_len..prev_len + curr_len]
                    .copy_from_slice(&curr[0][..curr_len]);
                self.line_lengths[self.cursor_y - 1] += curr_len;

                // Remove current line
                self.remove_line(self.cursor_y);

                // Move cursor
                self.cursor_y -= 1;
                self.cursor_x = prev_len;
                self.ensure_cursor_visible();
                self.modified = true;
            }
        }
    }

    fn delete_at_cursor(&mut self) {
        let line_len = self.line_lengths[self.cursor_y];

        if self.cursor_x < line_len {
            // Delete character at cursor
            let line = &mut self.lines[self.cursor_y];
            for i in self.cursor_x..line_len - 1 {
                line[i] = line[i + 1];
            }
            self.line_lengths[self.cursor_y] -= 1;
            self.modified = true;
        } else if self.cursor_y < self.num_lines - 1 {
            // Merge with next line
            let curr_len = self.line_lengths[self.cursor_y];
            let next_len = self.line_lengths[self.cursor_y + 1];

            if curr_len + next_len <= MAX_LINE_LEN {
                // Copy next line to end of current
                let (curr, next) = self.lines.split_at_mut(self.cursor_y + 1);
                curr[self.cursor_y][curr_len..curr_len + next_len]
                    .copy_from_slice(&next[0][..next_len]);
                self.line_lengths[self.cursor_y] += next_len;

                // Remove next line
                self.remove_line(self.cursor_y + 1);
                self.modified = true;
            }
        }
    }

    fn insert_newline(&mut self) {
        if self.num_lines >= MAX_LINES {
            return;
        }

        let curr_len = self.line_lengths[self.cursor_y];
        let remaining = curr_len - self.cursor_x;

        // Shift lines down
        for i in (self.cursor_y + 1..self.num_lines).rev() {
            self.lines[i + 1] = self.lines[i];
            self.line_lengths[i + 1] = self.line_lengths[i];
        }

        // Copy content after cursor to new line
        let (first, second) = self.lines.split_at_mut(self.cursor_y + 1);
        second[0][..remaining].copy_from_slice(&first[self.cursor_y][self.cursor_x..curr_len]);
        self.line_lengths[self.cursor_y + 1] = remaining;

        // Truncate current line
        self.line_lengths[self.cursor_y] = self.cursor_x;

        // Move cursor
        self.cursor_y += 1;
        self.cursor_x = 0;
        self.num_lines += 1;

        self.ensure_cursor_visible();
        self.modified = true;
    }

    fn remove_line(&mut self, line_idx: usize) {
        if self.num_lines <= 1 {
            // Keep at least one line
            self.line_lengths[0] = 0;
            return;
        }

        for i in line_idx..self.num_lines - 1 {
            self.lines[i] = self.lines[i + 1];
            self.line_lengths[i] = self.line_lengths[i + 1];
        }
        self.num_lines -= 1;
    }

    // === Line Operations ===

    fn copy_line(&mut self) {
        let line_len = self.line_lengths[self.cursor_y];
        self.clipboard[..line_len].copy_from_slice(&self.lines[self.cursor_y][..line_len]);
        self.clipboard_len = line_len;
        self.set_status("Line copied");
    }

    fn cut_line(&mut self) {
        self.copy_line();
        self.delete_line();
        self.set_status("Line cut");
    }

    fn paste_line(&mut self) {
        if self.clipboard_len == 0 {
            self.set_status("Clipboard empty");
            return;
        }

        if self.num_lines >= MAX_LINES {
            return;
        }

        // Insert new line below cursor
        for i in (self.cursor_y + 1..self.num_lines).rev() {
            self.lines[i + 1] = self.lines[i];
            self.line_lengths[i + 1] = self.line_lengths[i];
        }

        // Copy clipboard to new line
        self.lines[self.cursor_y + 1][..self.clipboard_len]
            .copy_from_slice(&self.clipboard[..self.clipboard_len]);
        self.line_lengths[self.cursor_y + 1] = self.clipboard_len;
        self.num_lines += 1;

        // Move cursor to pasted line
        self.cursor_y += 1;
        self.cursor_x = 0;
        self.ensure_cursor_visible();
        self.modified = true;
        self.set_status("Line pasted");
    }

    fn delete_line(&mut self) {
        if self.num_lines <= 1 {
            // Clear the only line
            self.line_lengths[0] = 0;
            self.cursor_x = 0;
        } else {
            self.remove_line(self.cursor_y);
            if self.cursor_y >= self.num_lines {
                self.cursor_y = self.num_lines - 1;
            }
            self.clamp_cursor_x();
        }
        self.modified = true;
        self.set_status("Line deleted");
    }

    fn clear_line(&mut self) {
        self.line_lengths[self.cursor_y] = 0;
        self.cursor_x = 0;
        self.modified = true;
        self.set_status("Line cleared");
    }

    fn delete_all(&mut self) {
        self.num_lines = 1;
        self.line_lengths[0] = 0;
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.scroll_offset = 0;
        self.all_selected = false;
        self.modified = true;
        self.set_status("All text deleted");
    }

    // === File Operations ===

    fn save_file(&mut self) {
        if self.filename_len == 0 {
            self.set_status("No filename");
            return;
        }

        let content = self.get_content();
        let filename = self.get_filename();

        if fs::write_file(filename, content) {
            self.set_status("Saved");
            self.modified = false;
        } else {
            self.set_status("Error saving file");
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

    // === Search ===

    fn start_search(&mut self) {
        self.prompt_mode = PromptMode::Search;
        self.search_query_len = 0;
        self.set_status("Find: ");
    }

    fn find_next(&mut self) {
        if self.search_query_len == 0 {
            self.set_status("No search term");
            return;
        }

        let query = &self.search_query[..self.search_query_len];

        // Search from current position forward
        let start_y = self.cursor_y;
        let start_x = self.cursor_x + 1;

        // Search in current line first (after cursor)
        if let Some(pos) = self.find_in_line(start_y, start_x, query) {
            self.cursor_x = pos;
            self.set_status("Found");
            return;
        }

        // Search in subsequent lines
        for y in start_y + 1..self.num_lines {
            if let Some(pos) = self.find_in_line(y, 0, query) {
                self.cursor_y = y;
                self.cursor_x = pos;
                self.ensure_cursor_visible();
                self.set_status("Found");
                return;
            }
        }

        // Wrap around to beginning
        for y in 0..=start_y {
            let search_start = if y == start_y { 0 } else { 0 };
            let search_end = if y == start_y { start_x } else { self.line_lengths[y] };
            
            if let Some(pos) = self.find_in_line_range(y, search_start, search_end, query) {
                self.cursor_y = y;
                self.cursor_x = pos;
                self.ensure_cursor_visible();
                self.set_status("Found (wrapped)");
                return;
            }
        }

        self.set_status("Not found");
    }

    fn find_in_line(&self, line_idx: usize, start_x: usize, query: &[u8]) -> Option<usize> {
        let line_len = self.line_lengths[line_idx];
        self.find_in_line_range(line_idx, start_x, line_len, query)
    }

    fn find_in_line_range(&self, line_idx: usize, start_x: usize, end_x: usize, query: &[u8]) -> Option<usize> {
        let line = &self.lines[line_idx];
        let line_len = end_x.min(self.line_lengths[line_idx]);
        let query_len = query.len();

        if query_len == 0 || line_len < query_len {
            return None;
        }

        for x in start_x..=line_len.saturating_sub(query_len) {
            if &line[x..x + query_len] == query {
                return Some(x);
            }
        }

        None
    }

    // === Rendering ===

    pub fn render(&self) {
        clear();

        let width = get_width() as usize;
        let height = get_height() as usize;
        let visible_lines = height.saturating_sub(2);

        // Render visible lines
        for screen_y in 0..visible_lines {
            let buffer_y = self.scroll_offset + screen_y;

            if buffer_y < self.num_lines {
                let line_len = self.line_lengths[buffer_y];
                let line = &self.lines[buffer_y][..line_len];

                if let Ok(s) = core::str::from_utf8(line) {
                    let display_len = s.len().min(width);
                    print(&s[..display_len]);
                }
            } else {
                print("~");
            }

            println("");
        }

        // Status bar
        set_cursor(0, (height - 2) as i32);

        let filename = self.get_filename();
        if filename.is_empty() {
            print("[No Name]");
        } else {
            print(filename);
        }

        if self.modified {
            print(" [+]");
        }

        if self.all_selected {
            print(" [ALL]");
        }

        print(" - L");
        print_int((self.cursor_y + 1) as i32);
        print("/");
        print_int(self.num_lines as i32);
        print(" C");
        print_int((self.cursor_x + 1) as i32);

        // Command/status line
        println("");

        match self.prompt_mode {
            PromptMode::SaveConfirm => {
                print("Save changes? (y/n)");
            }
            PromptMode::Search => {
                print("Find: ");
                if let Ok(s) = core::str::from_utf8(&self.search_query[..self.search_query_len]) {
                    print(s);
                }
            }
            PromptMode::None => {
                if self.status_len > 0 {
                    let status = unsafe {
                        core::str::from_utf8_unchecked(&self.status_message[..self.status_len])
                    };
                    print(status);
                } else {
                    print("Ctrl+E:Exit  Ctrl+S:Save  Ctrl+F:Find");
                }
            }
        }

        // Position the cursor
        let screen_cursor_y = (self.cursor_y - self.scroll_offset) as i32;
        let screen_cursor_x = self.cursor_x as i32;
        set_cursor(screen_cursor_x, screen_cursor_y);
    }
}

/// Helper function to print an integer
fn print_int(mut n: i32) {
    if n < 0 {
        print("-");
        n = -n;
    }
    if n == 0 {
        print("0");
        return;
    }

    let mut digits = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }

    while i > 0 {
        i -= 1;
        let c = [digits[i]];
        if let Ok(s) = core::str::from_utf8(&c) {
            print(s);
        }
    }
}
