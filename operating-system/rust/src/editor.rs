//! A simple vim-like text editor.
//! 
//! Modes:
//! - Normal: Navigation and commands (default)
//! - Insert: Text input (Ctrl+I to enter, Escape to exit)
//! - Command: Ex-style commands after pressing ':'
//!
//! Commands:
//! - :w - Write file
//! - :q - Quit
//! - :wq - Write and quit

use crate::fs;
use crate::terminal::{print, println, clear, set_cursor, get_width, get_height};

/// Editor modes
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Normal,
    Insert,
    Command,
}

/// Maximum lines in the editor buffer
const MAX_LINES: usize = 100;
/// Maximum characters per line
const MAX_LINE_LEN: usize = 256;

/// The editor state
pub struct Editor {
    /// Current mode
    mode: Mode,
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
    /// Current filename (if any)
    filename: [u8; 64],
    /// Length of filename
    filename_len: usize,
    /// Command line buffer
    command_buffer: [u8; 64],
    /// Command buffer length
    command_len: usize,
    /// Status message
    status_message: [u8; 80],
    /// Status message length
    status_len: usize,
    /// Whether the editor should exit
    should_exit: bool,
    /// Whether the buffer has been modified
    modified: bool,
}

impl Editor {
    /// Creates a new editor instance
    pub fn new() -> Self {
        Self {
            mode: Mode::Normal,
            lines: [[0u8; MAX_LINE_LEN]; MAX_LINES],
            line_lengths: [0; MAX_LINES],
            num_lines: 1, // Start with one empty line
            cursor_x: 0,
            cursor_y: 0,
            scroll_offset: 0,
            filename: [0u8; 64],
            filename_len: 0,
            command_buffer: [0u8; 64],
            command_len: 0,
            status_message: [0u8; 80],
            status_len: 0,
            should_exit: false,
            modified: false,
        }
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
        // We need a static buffer for this since we can't use String
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
        unsafe {
            core::str::from_utf8_unchecked(&self.filename[..self.filename_len])
        }
    }
    
    /// Checks if the editor should exit
    pub fn should_exit(&self) -> bool {
        self.should_exit
    }
    
    /// Handles a line of input (for command mode or line-based input)
    pub fn handle_input(&mut self, input: &str) {
        match self.mode {
            Mode::Normal => self.handle_normal_input(input),
            Mode::Insert => self.handle_insert_input(input),
            Mode::Command => self.handle_command_input(input),
        }
        
        if !self.should_exit {
            self.render();
        }
    }
    
    /// Handles input in normal mode
    fn handle_normal_input(&mut self, input: &str) {
        let bytes = input.as_bytes();
        if bytes.is_empty() {
            return;
        }
        
        // Check for Ctrl+I (ASCII 9, which is Tab)
        if bytes[0] == 9 || input == "i" {
            self.mode = Mode::Insert;
            self.set_status("-- INSERT --");
            return;
        }
        
        // Check for escape (shouldn't happen in normal mode, but just in case)
        if bytes[0] == 27 {
            return;
        }
        
        // Check for colon to enter command mode
        if bytes[0] == b':' {
            self.mode = Mode::Command;
            self.command_len = 0;
            self.set_status(":");
            return;
        }
        
        // Navigation keys
        match bytes[0] {
            b'h' | 0x1B => { // left or escape sequence
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                }
            }
            b'j' => { // down
                if self.cursor_y < self.num_lines - 1 {
                    self.cursor_y += 1;
                    self.adjust_cursor_x();
                    self.ensure_cursor_visible();
                }
            }
            b'k' => { // up
                if self.cursor_y > 0 {
                    self.cursor_y -= 1;
                    self.adjust_cursor_x();
                    self.ensure_cursor_visible();
                }
            }
            b'l' => { // right
                if self.cursor_x < self.line_lengths[self.cursor_y] {
                    self.cursor_x += 1;
                }
            }
            b'0' => { // beginning of line
                self.cursor_x = 0;
            }
            b'$' => { // end of line
                self.cursor_x = self.line_lengths[self.cursor_y];
            }
            b'G' => { // go to end of file
                self.cursor_y = self.num_lines - 1;
                self.adjust_cursor_x();
                self.ensure_cursor_visible();
            }
            b'g' => { // go to beginning of file (simplified, vim uses gg)
                self.cursor_y = 0;
                self.cursor_x = 0;
                self.scroll_offset = 0;
            }
            b'x' => { // delete character
                self.delete_char_at_cursor();
            }
            b'd' => { // delete line (simplified, vim uses dd)
                self.delete_current_line();
            }
            b'o' => { // open new line below
                self.insert_line_below();
                self.mode = Mode::Insert;
                self.set_status("-- INSERT --");
            }
            b'O' => { // open new line above
                self.insert_line_above();
                self.mode = Mode::Insert;
                self.set_status("-- INSERT --");
            }
            _ => {}
        }
    }
    
    /// Handles input in insert mode
    fn handle_insert_input(&mut self, input: &str) {
        let bytes = input.as_bytes();
        
        for &byte in bytes {
            // Check for escape (ASCII 27)
            if byte == 27 {
                self.mode = Mode::Normal;
                self.set_status("");
                return;
            }
            
            // Backspace
            if byte == 8 || byte == 127 {
                self.backspace();
                continue;
            }
            
            // Enter - insert new line
            if byte == b'\n' || byte == b'\r' {
                self.insert_newline();
                continue;
            }
            
            // Regular character - insert it
            if byte >= 32 && byte < 127 {
                self.insert_char(byte as char);
            }
        }
        
        self.modified = true;
    }
    
    /// Handles input in command mode
    fn handle_command_input(&mut self, input: &str) {
        let bytes = input.as_bytes();
        
        // Check for escape
        if !bytes.is_empty() && bytes[0] == 27 {
            self.mode = Mode::Normal;
            self.set_status("");
            return;
        }
        
        // Enter - execute command
        if !bytes.is_empty() && (bytes[0] == b'\n' || bytes[0] == b'\r') {
            self.execute_command();
            return;
        }
        
        // Backspace
        if !bytes.is_empty() && (bytes[0] == 8 || bytes[0] == 127) {
            if self.command_len > 0 {
                self.command_len -= 1;
            } else {
                // Exit command mode if buffer is empty
                self.mode = Mode::Normal;
                self.set_status("");
            }
            return;
        }
        
        // Add to command buffer
        for &byte in bytes {
            if byte >= 32 && byte < 127 && self.command_len < self.command_buffer.len() {
                self.command_buffer[self.command_len] = byte;
                self.command_len += 1;
            }
        }
        
        // Update status to show command
        self.status_message[0] = b':';
        let copy_len = self.command_len.min(self.status_message.len() - 1);
        self.status_message[1..1 + copy_len].copy_from_slice(&self.command_buffer[..copy_len]);
        self.status_len = 1 + copy_len;
    }
    
    /// Executes the current command
    fn execute_command(&mut self) {
        // Copy command buffer to avoid borrow issues
        let mut cmd_copy = [0u8; 64];
        let cmd_len = self.command_len;
        cmd_copy[..cmd_len].copy_from_slice(&self.command_buffer[..cmd_len]);
        
        // Process commands sequentially (w, q, wq, etc.)
        let mut i = 0;
        let mut has_write = false;
        let mut has_quit = false;
        let mut force_quit = false;
        
        while i < cmd_len {
            match cmd_copy[i] {
                b'w' => {
                    has_write = true;
                    i += 1;
                }
                b'q' => {
                    has_quit = true;
                    i += 1;
                }
                b'!' => {
                    force_quit = true;
                    i += 1;
                }
                b' ' => {
                    // Skip spaces
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            }
        }
        
        // Handle write command
        if has_write {
            if self.filename_len == 0 {
                self.set_status("No filename");
                self.mode = Mode::Normal;
                return;
            }
            
            let content = self.get_content();
            let filename = self.get_filename();
            
            if fs::write_file(filename, content) {
                self.set_status("Written");
                self.modified = false;
            } else {
                self.set_status("Error writing file");
                self.mode = Mode::Normal;
                return;
            }
        }
        
        // Handle quit command
        if has_quit {
            if self.modified && !force_quit {
                self.set_status("Unsaved changes! Use :q! or :wq");
                self.mode = Mode::Normal;
                return;
            }
            self.should_exit = true;
            return;
        }
        
        // Handle force quit without q (just !)
        if force_quit && !has_quit {
            self.should_exit = true;
            return;
        }
        
        self.mode = Mode::Normal;
    }
    
    /// Inserts a character at the cursor position
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
    }
    
    /// Handles backspace
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
        } else if self.cursor_y > 0 {
            // Join with previous line
            let prev_len = self.line_lengths[self.cursor_y - 1];
            let curr_len = self.line_lengths[self.cursor_y];
            
            if prev_len + curr_len <= MAX_LINE_LEN {
                // Copy current line content to end of previous line
                let (prev_lines, curr_lines) = self.lines.split_at_mut(self.cursor_y);
                prev_lines[self.cursor_y - 1][prev_len..prev_len + curr_len]
                    .copy_from_slice(&curr_lines[0][..curr_len]);
                self.line_lengths[self.cursor_y - 1] += curr_len;
                
                // Remove current line
                self.remove_line(self.cursor_y);
                
                // Move cursor
                self.cursor_y -= 1;
                self.cursor_x = prev_len;
            }
        }
    }
    
    /// Inserts a newline at the cursor position
    fn insert_newline(&mut self) {
        if self.num_lines >= MAX_LINES {
            return;
        }
        
        let curr_len = self.line_lengths[self.cursor_y];
        let remaining = curr_len - self.cursor_x;
        let cursor_x = self.cursor_x;
        let cursor_y = self.cursor_y;
        
        // Make room for new line by shifting lines down
        for i in (cursor_y + 1..self.num_lines).rev() {
            self.lines[i + 1] = self.lines[i];
            self.line_lengths[i + 1] = self.line_lengths[i];
        }
        
        // Clear the new line first
        self.line_lengths[cursor_y + 1] = 0;
        
        // Copy content after cursor to new line using split_at_mut
        let (first_part, second_part) = self.lines.split_at_mut(cursor_y + 1);
        second_part[0][..remaining].copy_from_slice(&first_part[cursor_y][cursor_x..curr_len]);
        self.line_lengths[cursor_y + 1] = remaining;
        
        // Truncate current line
        self.line_lengths[cursor_y] = cursor_x;
        
        // Move cursor to start of new line
        self.cursor_y += 1;
        self.cursor_x = 0;
        self.num_lines += 1;
        
        self.ensure_cursor_visible();
    }
    
    /// Deletes the character at the cursor position
    fn delete_char_at_cursor(&mut self) {
        let line_len = self.line_lengths[self.cursor_y];
        if self.cursor_x >= line_len {
            return;
        }
        
        let line = &mut self.lines[self.cursor_y];
        for i in self.cursor_x..line_len - 1 {
            line[i] = line[i + 1];
        }
        
        self.line_lengths[self.cursor_y] -= 1;
        self.modified = true;
    }
    
    /// Deletes the current line
    fn delete_current_line(&mut self) {
        if self.num_lines <= 1 {
            // Don't delete the last line, just clear it
            self.line_lengths[0] = 0;
            self.cursor_x = 0;
            self.modified = true;
            return;
        }
        
        self.remove_line(self.cursor_y);
        
        if self.cursor_y >= self.num_lines {
            self.cursor_y = self.num_lines - 1;
        }
        
        self.adjust_cursor_x();
        self.modified = true;
    }
    
    /// Removes a line from the buffer
    fn remove_line(&mut self, line_idx: usize) {
        for i in line_idx..self.num_lines - 1 {
            self.lines[i] = self.lines[i + 1];
            self.line_lengths[i] = self.line_lengths[i + 1];
        }
        self.num_lines -= 1;
    }
    
    /// Inserts a new line below the current line
    fn insert_line_below(&mut self) {
        if self.num_lines >= MAX_LINES {
            return;
        }
        
        // Make room for new line
        for i in (self.cursor_y + 1..self.num_lines).rev() {
            self.lines[i + 1] = self.lines[i];
            self.line_lengths[i + 1] = self.line_lengths[i];
        }
        
        // Clear new line
        self.line_lengths[self.cursor_y + 1] = 0;
        self.num_lines += 1;
        
        // Move cursor
        self.cursor_y += 1;
        self.cursor_x = 0;
        
        self.ensure_cursor_visible();
        self.modified = true;
    }
    
    /// Inserts a new line above the current line
    fn insert_line_above(&mut self) {
        if self.num_lines >= MAX_LINES {
            return;
        }
        
        // Make room for new line
        for i in (self.cursor_y..self.num_lines).rev() {
            self.lines[i + 1] = self.lines[i];
            self.line_lengths[i + 1] = self.line_lengths[i];
        }
        
        // Clear new line
        self.line_lengths[self.cursor_y] = 0;
        self.num_lines += 1;
        
        self.cursor_x = 0;
        self.ensure_cursor_visible();
        self.modified = true;
    }
    
    /// Adjusts cursor_x to be within the current line
    fn adjust_cursor_x(&mut self) {
        let line_len = self.line_lengths[self.cursor_y];
        if self.cursor_x > line_len {
            self.cursor_x = line_len;
        }
    }
    
    /// Ensures the cursor is visible by adjusting scroll offset
    fn ensure_cursor_visible(&mut self) {
        let visible_lines = get_height() as usize - 2; // Leave room for status lines
        
        if self.cursor_y < self.scroll_offset {
            self.scroll_offset = self.cursor_y;
        } else if self.cursor_y >= self.scroll_offset + visible_lines {
            self.scroll_offset = self.cursor_y - visible_lines + 1;
        }
    }
    
    /// Renders the editor to the terminal
    pub fn render(&self) {
        clear();
        
        let width = get_width() as usize;
        let height = get_height() as usize;
        let visible_lines = height - 2; // Status bar + command line
        
        // Render visible lines
        for screen_y in 0..visible_lines {
            let buffer_y = self.scroll_offset + screen_y;
            
            if buffer_y < self.num_lines {
                let line_len = self.line_lengths[buffer_y];
                let line = &self.lines[buffer_y][..line_len];
                
                // Convert to string and print
                if let Ok(s) = core::str::from_utf8(line) {
                    // Truncate to terminal width
                    let display_len = s.len().min(width);
                    print(&s[..display_len]);
                }
            } else {
                // Empty line indicator (like vim's ~)
                print("~");
            }
            
            println("");
        }
        
        // Status bar (second to last line)
        set_cursor(0, (height - 2) as i32);
        
        // Show filename and status
        let filename = self.get_filename();
        if filename.is_empty() {
            print("[No Name]");
        } else {
            print(filename);
        }
        
        if self.modified {
            print(" [+]");
        }
        
        // Show line/column info
        print(" - Line ");
        print_int((self.cursor_y + 1) as i32);
        print("/");
        print_int(self.num_lines as i32);
        print(", Col ");
        print_int((self.cursor_x + 1) as i32);
        
        // Command/status line (last line)
        println("");
        
        if self.status_len > 0 {
            let status = unsafe {
                core::str::from_utf8_unchecked(&self.status_message[..self.status_len])
            };
            print(status);
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
