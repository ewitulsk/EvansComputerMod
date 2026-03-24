//! VIM editor implementation for the Terminal OS.
//!
//! A faithful VIM emulation with Normal, Insert, Visual, Command, and Search modes.
//! Supports motions, operators, text objects, registers, undo/redo, and dot repeat.

use crate::terminal::{self, print, println, clear};
use crate::fs;

// === Constants ===

const MAX_LINES: usize = 500;
const MAX_LINE_LEN: usize = 256;
const MAX_UNDO: usize = 50;
const MAX_REGISTERS: usize = 10;
const MAX_REG_LINES: usize = 20;
const TAB_WIDTH: usize = 4;

// === Enums ===

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    VisualLine,
    Command,
    Search,
    Replace, // single char replace (r)
}

#[derive(Clone, Copy, PartialEq)]
enum Operator {
    None,
    Delete,
    Yank,
    Change,
    Indent,
    Dedent,
}

#[derive(Clone, Copy, PartialEq)]
enum EscState {
    None,
    Escape,
    Csi,
}

// === Register ===

#[derive(Clone)]
struct Register {
    lines: Vec<Vec<u8>>,
    linewise: bool,
}

impl Register {
    fn new() -> Self {
        Register {
            lines: Vec::new(),
            linewise: false,
        }
    }

    fn set(&mut self, text: &[u8], linewise: bool) {
        self.lines.clear();
        if linewise {
            // Split by newlines
            for line in text.split(|&b| b == b'\n') {
                self.lines.push(line.to_vec());
            }
            // Remove trailing empty line from split
            if self.lines.last().map_or(false, |l| l.is_empty()) && self.lines.len() > 1 {
                self.lines.pop();
            }
        } else {
            self.lines.push(text.to_vec());
        }
        self.linewise = linewise;
    }

    fn set_lines(&mut self, lines: &[&[u8]], linewise: bool) {
        self.lines.clear();
        for line in lines {
            self.lines.push(line.to_vec());
        }
        self.linewise = linewise;
    }

    fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut result = Vec::new();
        for (i, line) in self.lines.iter().enumerate() {
            result.extend_from_slice(line);
            if i + 1 < self.lines.len() {
                result.push(b'\n');
            }
        }
        result
    }
}

// === Undo Snapshot ===

struct UndoSnapshot {
    lines: Vec<Vec<u8>>,
    num_lines: usize,
    cursor_x: usize,
    cursor_y: usize,
}

impl UndoSnapshot {
    fn capture(editor: &VimEditor) -> Self {
        let mut lines = Vec::with_capacity(editor.num_lines);
        for i in 0..editor.num_lines {
            lines.push(editor.lines[i][..editor.line_lengths[i]].to_vec());
        }
        UndoSnapshot {
            lines,
            num_lines: editor.num_lines,
            cursor_x: editor.cursor_x,
            cursor_y: editor.cursor_y,
        }
    }

    fn restore(&self, editor: &mut VimEditor) {
        editor.num_lines = self.num_lines;
        for i in 0..self.num_lines {
            let len = self.lines[i].len().min(MAX_LINE_LEN);
            editor.lines[i][..len].copy_from_slice(&self.lines[i][..len]);
            editor.line_lengths[i] = len;
        }
        // Clear remaining lines
        for i in self.num_lines..MAX_LINES {
            editor.line_lengths[i] = 0;
        }
        editor.cursor_x = self.cursor_x;
        editor.cursor_y = self.cursor_y;
        editor.clamp_cursor();
        editor.ensure_cursor_visible();
        editor.modified = true;
    }
}

// === Pending command state ===

struct PendingCommand {
    count: Option<usize>,
    operator: Operator,
    operator_count: Option<usize>,
    g_prefix: bool,       // received 'g' waiting for second char
    z_prefix: bool,       // received 'z' waiting for second char
    f_char: Option<u8>,   // 'f'/'F'/'t'/'T' waiting for target char
    f_type: u8,           // which find type: b'f', b'F', b't', b'T'
    text_obj_prefix: u8,  // b'i' or b'a' for text object (only when operator pending)
}

impl PendingCommand {
    fn new() -> Self {
        PendingCommand {
            count: None,
            operator: Operator::None,
            operator_count: None,
            g_prefix: false,
            z_prefix: false,
            f_char: None,
            f_type: 0,
            text_obj_prefix: 0,
        }
    }

    fn clear(&mut self) {
        self.count = None;
        self.operator = Operator::None;
        self.operator_count = None;
        self.g_prefix = false;
        self.z_prefix = false;
        self.f_char = None;
        self.f_type = 0;
        self.text_obj_prefix = 0;
    }

    fn effective_count(&self) -> usize {
        let c1 = self.operator_count.unwrap_or(1);
        let c2 = self.count.unwrap_or(1);
        c1 * c2
    }

    fn has_count(&self) -> bool {
        self.count.is_some() || self.operator_count.is_some()
    }
}

// === Motion result ===

#[derive(Clone, Copy)]
struct MotionResult {
    target_x: usize,
    target_y: usize,
    linewise: bool,
    inclusive: bool, // true = includes the target character
}

// === Main VimEditor struct ===

pub struct VimEditor {
    // Buffer
    lines: [[u8; MAX_LINE_LEN]; MAX_LINES],
    line_lengths: [usize; MAX_LINES],
    num_lines: usize,

    // Cursor
    cursor_x: usize,
    cursor_y: usize,
    desired_x: usize,
    scroll_offset: usize,

    // Mode
    mode: Mode,

    // File
    filename: [u8; 128],
    filename_len: usize,
    modified: bool,

    // Pending command
    pending: PendingCommand,

    // Dot repeat
    dot_command: Vec<u8>,
    recording_dot: bool,
    current_dot: Vec<u8>,
    replaying_dot: bool,

    // Command line
    cmdline: Vec<u8>,

    // Search
    search_query: Vec<u8>,
    search_forward: bool,
    last_search_query: Vec<u8>,
    last_search_forward: bool,

    // Visual mode
    visual_start_x: usize,
    visual_start_y: usize,

    // Registers: 0=unnamed, 1-9=delete history
    registers: Vec<Register>,

    // Undo/Redo
    undo_stack: Vec<UndoSnapshot>,
    redo_stack: Vec<UndoSnapshot>,

    // Escape sequence parsing
    esc_buf: [u8; 8],
    esc_len: usize,
    esc_state: EscState,

    // Display
    show_line_numbers: bool,

    // Status message
    status_msg: Vec<u8>,

    // Exit flag
    pub exit_requested: bool,

    // Last f/F/t/T find
    last_find_char: u8,
    last_find_type: u8, // b'f', b'F', b't', b'T'

    // Insert mode start position (for undo grouping)
    insert_start_x: usize,
    insert_start_y: usize,
}

impl VimEditor {
    pub fn new() -> Self {
        let mut editor = VimEditor {
            lines: [[0u8; MAX_LINE_LEN]; MAX_LINES],
            line_lengths: [0usize; MAX_LINES],
            num_lines: 1,
            cursor_x: 0,
            cursor_y: 0,
            desired_x: 0,
            scroll_offset: 0,
            mode: Mode::Normal,
            filename: [0u8; 128],
            filename_len: 0,
            modified: false,
            pending: PendingCommand::new(),
            dot_command: Vec::new(),
            recording_dot: false,
            current_dot: Vec::new(),
            replaying_dot: false,
            cmdline: Vec::new(),
            search_query: Vec::new(),
            search_forward: true,
            last_search_query: Vec::new(),
            last_search_forward: true,
            visual_start_x: 0,
            visual_start_y: 0,
            registers: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            esc_buf: [0u8; 8],
            esc_len: 0,
            esc_state: EscState::None,
            show_line_numbers: false,
            status_msg: Vec::new(),
            exit_requested: false,
            last_find_char: 0,
            last_find_type: 0,
            insert_start_x: 0,
            insert_start_y: 0,
        };
        for _ in 0..MAX_REGISTERS {
            editor.registers.push(Register::new());
        }
        editor
    }

    // === File Operations ===

    pub fn open(&mut self, filename: &str) {
        let len = filename.len().min(self.filename.len());
        self.filename[..len].copy_from_slice(&filename.as_bytes()[..len]);
        self.filename_len = len;

        if let Some(content) = fs::read_file(filename) {
            self.load_content(content);
        } else {
            // New file
            self.num_lines = 1;
            self.line_lengths[0] = 0;
        }
        self.modified = false;
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.scroll_offset = 0;
        self.set_status_msg(&format!("\"{}\" {}L", filename, self.num_lines));
    }

    fn load_content(&mut self, content: &str) {
        self.num_lines = 0;
        for line in content.split('\n') {
            if self.num_lines >= MAX_LINES {
                break;
            }
            let bytes = line.as_bytes();
            // Strip trailing \r
            let len = if bytes.last() == Some(&b'\r') {
                bytes.len() - 1
            } else {
                bytes.len()
            };
            let len = len.min(MAX_LINE_LEN);
            self.lines[self.num_lines][..len].copy_from_slice(&bytes[..len]);
            self.line_lengths[self.num_lines] = len;
            self.num_lines += 1;
        }
        if self.num_lines == 0 {
            self.num_lines = 1;
            self.line_lengths[0] = 0;
        }
        // If content ended with \n, the split produces an extra empty string at the end
        // which represents the final newline, not an extra line
        if content.ends_with('\n') && self.num_lines > 1 {
            // The last "line" from split is empty and represents the trailing newline
            // Check if it's truly empty
            if self.line_lengths[self.num_lines - 1] == 0 {
                self.num_lines -= 1;
            }
        }
    }

    fn get_content(&self) -> String {
        let mut result = String::new();
        for i in 0..self.num_lines {
            let line = &self.lines[i][..self.line_lengths[i]];
            if let Ok(s) = std::str::from_utf8(line) {
                result.push_str(s);
            }
            result.push('\n');
        }
        result
    }

    fn get_filename(&self) -> &str {
        std::str::from_utf8(&self.filename[..self.filename_len]).unwrap_or("")
    }

    fn save_file(&mut self) -> bool {
        let filename = std::str::from_utf8(&self.filename[..self.filename_len])
            .unwrap_or("")
            .to_string();
        if filename.is_empty() {
            self.set_status_msg("E32: No file name");
            return false;
        }
        let content = self.get_content();
        if fs::write_file(&filename, &content) {
            self.modified = false;
            self.set_status_msg(&format!("\"{}\" {}L written", filename, self.num_lines));
            true
        } else {
            self.set_status_msg("E212: Can't open file for writing");
            false
        }
    }

    fn save_file_as(&mut self, new_name: &str) -> bool {
        let len = new_name.len().min(self.filename.len());
        self.filename[..len].copy_from_slice(&new_name.as_bytes()[..len]);
        self.filename_len = len;
        self.save_file()
    }

    // === Status Message ===

    fn set_status_msg(&mut self, msg: &str) {
        self.status_msg = msg.as_bytes().to_vec();
    }

    // === Undo/Redo ===

    fn save_undo(&mut self) {
        if self.undo_stack.len() >= MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(UndoSnapshot::capture(self));
        self.redo_stack.clear();
    }

    fn undo(&mut self) {
        if let Some(snapshot) = self.undo_stack.pop() {
            self.redo_stack.push(UndoSnapshot::capture(self));
            snapshot.restore(self);
            self.set_status_msg("Undone");
        } else {
            self.set_status_msg("Already at oldest change");
        }
    }

    fn redo(&mut self) {
        if let Some(snapshot) = self.redo_stack.pop() {
            self.undo_stack.push(UndoSnapshot::capture(self));
            snapshot.restore(self);
            self.set_status_msg("Redone");
        } else {
            self.set_status_msg("Already at newest change");
        }
    }

    // === Register Operations ===

    fn yank_to_register(&mut self, text: &[u8], linewise: bool) {
        self.registers[0].set(text, linewise);
        // Shift delete history registers
        // (only for deletes, but we unify for simplicity)
    }

    fn yank_lines_to_register(&mut self, start_line: usize, end_line: usize) {
        let mut lines: Vec<&[u8]> = Vec::new();
        for i in start_line..=end_line.min(self.num_lines - 1) {
            lines.push(&self.lines[i][..self.line_lengths[i]]);
        }
        self.registers[0].set_lines(&lines, true);
    }

    fn delete_to_register(&mut self, text: &[u8], linewise: bool) {
        // Shift delete history: 9<-8<-...<-2<-1, 1<-unnamed
        for i in (2..MAX_REGISTERS).rev() {
            self.registers[i] = self.registers[i - 1].clone();
        }
        if MAX_REGISTERS > 1 {
            self.registers[1] = self.registers[0].clone();
        }
        self.registers[0].set(text, linewise);
    }

    fn delete_lines_to_register(&mut self, start_line: usize, end_line: usize) {
        let mut lines: Vec<&[u8]> = Vec::new();
        for i in start_line..=end_line.min(self.num_lines - 1) {
            lines.push(&self.lines[i][..self.line_lengths[i]]);
        }
        // Shift delete history
        for i in (2..MAX_REGISTERS).rev() {
            self.registers[i] = self.registers[i - 1].clone();
        }
        if MAX_REGISTERS > 1 {
            self.registers[1] = self.registers[0].clone();
        }
        self.registers[0].set_lines(&lines, true);
    }

    // === Cursor Movement ===

    fn clamp_cursor(&mut self) {
        if self.cursor_y >= self.num_lines {
            self.cursor_y = if self.num_lines > 0 { self.num_lines - 1 } else { 0 };
        }
        let max_x = self.max_cursor_x();
        if self.cursor_x > max_x {
            self.cursor_x = max_x;
        }
    }

    fn max_cursor_x(&self) -> usize {
        let line_len = self.line_lengths[self.cursor_y];
        if self.mode == Mode::Insert {
            line_len // can be at end of line in insert mode
        } else if line_len > 0 {
            line_len - 1 // normal mode: cursor on last char, not past it
        } else {
            0
        }
    }

    fn ensure_cursor_visible(&mut self) {
        let text_height = self.text_height();
        if self.cursor_y < self.scroll_offset {
            self.scroll_offset = self.cursor_y;
        }
        if self.cursor_y >= self.scroll_offset + text_height {
            self.scroll_offset = self.cursor_y - text_height + 1;
        }
    }

    fn text_height(&self) -> usize {
        let h = terminal::get_height() as usize;
        if h > 2 { h - 2 } else { 1 }
    }

    fn text_width(&self) -> usize {
        let w = terminal::get_width() as usize;
        if self.show_line_numbers {
            let gutter = self.gutter_width();
            if w > gutter { w - gutter } else { 1 }
        } else {
            w
        }
    }

    fn gutter_width(&self) -> usize {
        if !self.show_line_numbers {
            return 0;
        }
        let mut digits = 1;
        let mut n = self.num_lines;
        while n >= 10 {
            n /= 10;
            digits += 1;
        }
        digits + 1 // digits + 1 space
    }

    fn move_left(&mut self) {
        if self.cursor_x > 0 {
            self.cursor_x -= 1;
            self.desired_x = self.cursor_x;
        }
    }

    fn move_right(&mut self) {
        let max = self.max_cursor_x();
        if self.cursor_x < max {
            self.cursor_x += 1;
            self.desired_x = self.cursor_x;
        }
    }

    fn move_up(&mut self) {
        if self.cursor_y > 0 {
            self.cursor_y -= 1;
            self.cursor_x = self.desired_x.min(self.max_cursor_x());
            self.ensure_cursor_visible();
        }
    }

    fn move_down(&mut self) {
        if self.cursor_y + 1 < self.num_lines {
            self.cursor_y += 1;
            self.cursor_x = self.desired_x.min(self.max_cursor_x());
            self.ensure_cursor_visible();
        }
    }

    fn move_to_line_start(&mut self) {
        self.cursor_x = 0;
        self.desired_x = 0;
    }

    fn move_to_first_non_blank(&mut self) {
        let line = &self.lines[self.cursor_y][..self.line_lengths[self.cursor_y]];
        let mut x = 0;
        while x < line.len() && (line[x] == b' ' || line[x] == b'\t') {
            x += 1;
        }
        if x >= self.line_lengths[self.cursor_y] && self.line_lengths[self.cursor_y] > 0 {
            x = self.line_lengths[self.cursor_y] - 1;
        }
        self.cursor_x = x;
        self.desired_x = x;
    }

    fn move_to_line_end(&mut self) {
        let max = self.max_cursor_x();
        self.cursor_x = max;
        self.desired_x = usize::MAX; // sticky end
    }

    // === Word Motions ===

    fn is_word_char(c: u8) -> bool {
        c.is_ascii_alphanumeric() || c == b'_'
    }

    fn is_big_word_char(c: u8) -> bool {
        c != b' ' && c != b'\t'
    }

    fn word_forward(&self, mut x: usize, mut y: usize, big: bool) -> (usize, usize) {
        let is_wc = if big { Self::is_big_word_char } else { Self::is_word_char };

        if y >= self.num_lines {
            return (x, y);
        }

        let line = &self.lines[y][..self.line_lengths[y]];

        if x < line.len() {
            // Skip current word
            let start_is_word = is_wc(line[x]);
            if start_is_word {
                while x < line.len() && is_wc(line[x]) {
                    x += 1;
                }
            } else if line[x] != b' ' && line[x] != b'\t' {
                // Punctuation word
                while x < line.len() && !is_wc(line[x]) && line[x] != b' ' && line[x] != b'\t' {
                    x += 1;
                }
            }
            // Skip whitespace
            while x < line.len() && (line[x] == b' ' || line[x] == b'\t') {
                x += 1;
            }
            if x < line.len() {
                return (x, y);
            }
        }

        // Move to next line
        y += 1;
        while y < self.num_lines {
            let line = &self.lines[y][..self.line_lengths[y]];
            let mut nx = 0;
            while nx < line.len() && (line[nx] == b' ' || line[nx] == b'\t') {
                nx += 1;
            }
            if nx < line.len() {
                return (nx, y);
            }
            // Empty line
            if line.is_empty() {
                return (0, y);
            }
            y += 1;
        }

        // End of file
        let last_y = if self.num_lines > 0 { self.num_lines - 1 } else { 0 };
        let last_x = if self.line_lengths[last_y] > 0 { self.line_lengths[last_y] - 1 } else { 0 };
        (last_x, last_y)
    }

    fn word_backward(&self, mut x: usize, mut y: usize, big: bool) -> (usize, usize) {
        let is_wc = if big { Self::is_big_word_char } else { Self::is_word_char };

        if y >= self.num_lines {
            return (0, 0);
        }

        // If at start of line, go to previous line
        if x == 0 {
            if y == 0 {
                return (0, 0);
            }
            y -= 1;
            x = if self.line_lengths[y] > 0 { self.line_lengths[y] - 1 } else { 0 };
            // Skip trailing whitespace
            let line = &self.lines[y][..self.line_lengths[y]];
            while x > 0 && (line[x] == b' ' || line[x] == b'\t') {
                x -= 1;
            }
        } else {
            x -= 1;
        }

        let line = &self.lines[y][..self.line_lengths[y]];
        if line.is_empty() {
            return (0, y);
        }

        // Skip whitespace
        while x > 0 && (line[x] == b' ' || line[x] == b'\t') {
            x -= 1;
        }

        if x == 0 && (line[0] == b' ' || line[0] == b'\t') {
            // All whitespace up to here, go to previous line
            if y > 0 {
                return self.word_backward(0, y, big);
            }
            return (0, 0);
        }

        // Go back to start of current word
        let cur_is_word = is_wc(line[x]);
        if cur_is_word {
            while x > 0 && is_wc(line[x - 1]) {
                x -= 1;
            }
        } else {
            while x > 0 && !is_wc(line[x - 1]) && line[x - 1] != b' ' && line[x - 1] != b'\t' {
                x -= 1;
            }
        }

        (x, y)
    }

    fn word_end(&self, mut x: usize, mut y: usize, big: bool) -> (usize, usize) {
        let is_wc = if big { Self::is_big_word_char } else { Self::is_word_char };

        if y >= self.num_lines {
            return (x, y);
        }

        // Move forward at least one character
        let line = &self.lines[y][..self.line_lengths[y]];
        if x + 1 < line.len() {
            x += 1;
        } else {
            y += 1;
            x = 0;
            if y >= self.num_lines {
                let last_y = self.num_lines - 1;
                let last_x = if self.line_lengths[last_y] > 0 { self.line_lengths[last_y] - 1 } else { 0 };
                return (last_x, last_y);
            }
        }

        // Skip whitespace and empty lines
        loop {
            if y >= self.num_lines {
                let last_y = self.num_lines - 1;
                let last_x = if self.line_lengths[last_y] > 0 { self.line_lengths[last_y] - 1 } else { 0 };
                return (last_x, last_y);
            }
            let line = &self.lines[y][..self.line_lengths[y]];
            if line.is_empty() {
                y += 1;
                x = 0;
                continue;
            }
            while x < line.len() && (line[x] == b' ' || line[x] == b'\t') {
                x += 1;
            }
            if x < line.len() {
                break;
            }
            y += 1;
            x = 0;
        }

        let line = &self.lines[y][..self.line_lengths[y]];
        // Move to end of word
        let cur_is_word = is_wc(line[x]);
        if cur_is_word {
            while x + 1 < line.len() && is_wc(line[x + 1]) {
                x += 1;
            }
        } else {
            while x + 1 < line.len() && !is_wc(line[x + 1]) && line[x + 1] != b' ' && line[x + 1] != b'\t' {
                x += 1;
            }
        }

        (x, y)
    }

    // === Paragraph Motions ===

    fn paragraph_forward(&self, y: usize) -> usize {
        let mut ny = y;
        // Skip current non-empty lines
        while ny < self.num_lines && self.line_lengths[ny] > 0 {
            ny += 1;
        }
        // Skip empty lines
        while ny < self.num_lines && self.line_lengths[ny] == 0 {
            ny += 1;
        }
        if ny >= self.num_lines {
            ny = self.num_lines - 1;
        }
        ny
    }

    fn paragraph_backward(&self, y: usize) -> usize {
        if y == 0 {
            return 0;
        }
        let mut ny = y - 1;
        // Skip current empty lines
        while ny > 0 && self.line_lengths[ny] == 0 {
            ny -= 1;
        }
        // Skip non-empty lines
        while ny > 0 && self.line_lengths[ny - 1] > 0 {
            ny -= 1;
        }
        ny
    }

    // === Character Find (f/F/t/T) ===

    fn find_char_forward(&self, ch: u8, cursor_x: usize, cursor_y: usize, stop_before: bool) -> Option<usize> {
        let line = &self.lines[cursor_y][..self.line_lengths[cursor_y]];
        for i in (cursor_x + 1)..line.len() {
            if line[i] == ch {
                return if stop_before && i > 0 { Some(i - 1) } else { Some(i) };
            }
        }
        None
    }

    fn find_char_backward(&self, ch: u8, cursor_x: usize, cursor_y: usize, stop_before: bool) -> Option<usize> {
        let line = &self.lines[cursor_y][..self.line_lengths[cursor_y]];
        if cursor_x == 0 {
            return None;
        }
        for i in (0..cursor_x).rev() {
            if line[i] == ch {
                return if stop_before { Some(i + 1) } else { Some(i) };
            }
        }
        None
    }

    // === Bracket Matching ===

    fn find_matching_bracket(&self) -> Option<(usize, usize)> {
        let line = &self.lines[self.cursor_y][..self.line_lengths[self.cursor_y]];
        if self.cursor_x >= line.len() {
            return None;
        }
        let ch = line[self.cursor_x];
        let (target, forward) = match ch {
            b'(' => (b')', true),
            b')' => (b'(', false),
            b'[' => (b']', true),
            b']' => (b'[', false),
            b'{' => (b'}', true),
            b'}' => (b'{', false),
            _ => return None,
        };

        let mut depth = 1i32;
        let mut x = self.cursor_x;
        let mut y = self.cursor_y;

        loop {
            if forward {
                x += 1;
                while y < self.num_lines {
                    let l = &self.lines[y][..self.line_lengths[y]];
                    while x < l.len() {
                        if l[x] == ch {
                            depth += 1;
                        } else if l[x] == target {
                            depth -= 1;
                            if depth == 0 {
                                return Some((x, y));
                            }
                        }
                        x += 1;
                    }
                    y += 1;
                    x = 0;
                }
            } else {
                loop {
                    if x == 0 {
                        if y == 0 {
                            return None;
                        }
                        y -= 1;
                        x = self.line_lengths[y];
                        if x == 0 {
                            continue;
                        }
                    }
                    x -= 1;
                    let l = &self.lines[y][..self.line_lengths[y]];
                    if l[x] == ch {
                        depth += 1;
                    } else if l[x] == target {
                        depth -= 1;
                        if depth == 0 {
                            return Some((x, y));
                        }
                    }
                }
            }
            break;
        }
        None
    }

    // === Text Objects ===

    /// Returns (start_x, end_x) on the current line for inner/around word
    fn text_object_word(&self, around: bool) -> Option<(usize, usize, usize, usize)> {
        let line = &self.lines[self.cursor_y][..self.line_lengths[self.cursor_y]];
        if line.is_empty() {
            return None;
        }
        let x = self.cursor_x.min(line.len() - 1);

        let mut start = x;
        let mut end = x;

        if Self::is_word_char(line[x]) {
            while start > 0 && Self::is_word_char(line[start - 1]) {
                start -= 1;
            }
            while end + 1 < line.len() && Self::is_word_char(line[end + 1]) {
                end += 1;
            }
            if around {
                // Include trailing whitespace
                while end + 1 < line.len() && (line[end + 1] == b' ' || line[end + 1] == b'\t') {
                    end += 1;
                }
            }
        } else if line[x] == b' ' || line[x] == b'\t' {
            while start > 0 && (line[start - 1] == b' ' || line[start - 1] == b'\t') {
                start -= 1;
            }
            while end + 1 < line.len() && (line[end + 1] == b' ' || line[end + 1] == b'\t') {
                end += 1;
            }
        } else {
            // Punctuation
            while start > 0 && !Self::is_word_char(line[start - 1]) && line[start - 1] != b' ' && line[start - 1] != b'\t' {
                start -= 1;
            }
            while end + 1 < line.len() && !Self::is_word_char(line[end + 1]) && line[end + 1] != b' ' && line[end + 1] != b'\t' {
                end += 1;
            }
            if around {
                while end + 1 < line.len() && (line[end + 1] == b' ' || line[end + 1] == b'\t') {
                    end += 1;
                }
            }
        }

        Some((start, self.cursor_y, end, self.cursor_y))
    }

    /// Returns (start_x, start_y, end_x, end_y) for inner/around delimited pair
    fn text_object_delimited(&self, open: u8, close: u8, around: bool) -> Option<(usize, usize, usize, usize)> {
        // Search backward for opening delimiter
        let mut depth = 0i32;
        let mut sx = self.cursor_x;
        let mut sy = self.cursor_y;
        let mut found_open = false;

        // Check if we're on the opening/closing delimiter
        let cur_line = &self.lines[self.cursor_y][..self.line_lengths[self.cursor_y]];
        if self.cursor_x < cur_line.len() && cur_line[self.cursor_x] == open {
            sx = self.cursor_x;
            sy = self.cursor_y;
            found_open = true;
        } else {
            // Search backward
            let mut x = self.cursor_x;
            let mut y = self.cursor_y;
            loop {
                let line = &self.lines[y][..self.line_lengths[y]];
                while x < line.len() {
                    if line[x] == close && !(y == self.cursor_y && x == self.cursor_x) {
                        depth += 1;
                    } else if line[x] == open {
                        if depth == 0 {
                            sx = x;
                            sy = y;
                            found_open = true;
                            break;
                        }
                        depth -= 1;
                    }
                    if x == 0 { break; }
                    x -= 1;
                }
                if found_open { break; }
                if y == 0 { break; }
                y -= 1;
                x = if self.line_lengths[y] > 0 { self.line_lengths[y] - 1 } else { 0 };
            }
        }

        if !found_open {
            return None;
        }

        // Search forward for closing delimiter
        depth = 1;
        let mut ex = sx + 1;
        let mut ey = sy;
        loop {
            if ey >= self.num_lines {
                return None;
            }
            let line = &self.lines[ey][..self.line_lengths[ey]];
            while ex < line.len() {
                if line[ex] == open {
                    depth += 1;
                } else if line[ex] == close {
                    depth -= 1;
                    if depth == 0 {
                        if around {
                            return Some((sx, sy, ex, ey));
                        } else {
                            // Inner: exclude delimiters
                            let isx = sx + 1;
                            let isy = sy;
                            let iex = if ex > 0 { ex - 1 } else { 0 };
                            let iey = ey;
                            if isy > iey || (isy == iey && isx > iex) {
                                // Empty inside
                                return Some((sx + 1, sy, sx, sy)); // empty range
                            }
                            return Some((isx, isy, iex, iey));
                        }
                    }
                }
                ex += 1;
            }
            ey += 1;
            ex = 0;
        }
    }

    /// Text object for quotes (i"/a", i'/a')
    fn text_object_quote(&self, quote: u8, around: bool) -> Option<(usize, usize, usize, usize)> {
        let line = &self.lines[self.cursor_y][..self.line_lengths[self.cursor_y]];
        let y = self.cursor_y;

        // Find quote boundaries on current line
        let mut positions: Vec<usize> = Vec::new();
        for (i, &c) in line.iter().enumerate() {
            if c == quote {
                // Check if escaped
                if i > 0 && line[i - 1] == b'\\' {
                    continue;
                }
                positions.push(i);
            }
        }

        // Find the pair that contains the cursor
        let cx = self.cursor_x;
        for pair in positions.chunks(2) {
            if pair.len() == 2 {
                let (start, end) = (pair[0], pair[1]);
                if cx >= start && cx <= end {
                    if around {
                        return Some((start, y, end, y));
                    } else {
                        if start + 1 <= end - 1 {
                            return Some((start + 1, y, end - 1, y));
                        } else {
                            return Some((start + 1, y, start, y)); // empty
                        }
                    }
                }
            }
        }

        // Try to find next quote pair after cursor
        let mut first_after = None;
        for &pos in &positions {
            if pos > cx {
                first_after = Some(pos);
                break;
            }
        }
        if let Some(start) = first_after {
            if let Some(&end) = positions.iter().find(|&&p| p > start) {
                if around {
                    return Some((start, y, end, y));
                } else {
                    if start + 1 <= end - 1 {
                        return Some((start + 1, y, end - 1, y));
                    }
                }
            }
        }

        None
    }

    // === Motion Computation ===

    fn compute_motion(&self, key: u8, count: usize) -> Option<MotionResult> {
        let mut cx = self.cursor_x;
        let mut cy = self.cursor_y;

        match key {
            b'h' => {
                for _ in 0..count {
                    if cx > 0 { cx -= 1; }
                }
                Some(MotionResult { target_x: cx, target_y: cy, linewise: false, inclusive: false })
            }
            b'l' => {
                let max = if self.line_lengths[cy] > 0 { self.line_lengths[cy] - 1 } else { 0 };
                for _ in 0..count {
                    if cx < max { cx += 1; }
                }
                Some(MotionResult { target_x: cx, target_y: cy, linewise: false, inclusive: false })
            }
            b'j' => {
                for _ in 0..count {
                    if cy + 1 < self.num_lines { cy += 1; }
                }
                Some(MotionResult { target_x: cx, target_y: cy, linewise: true, inclusive: false })
            }
            b'k' => {
                for _ in 0..count {
                    if cy > 0 { cy -= 1; }
                }
                Some(MotionResult { target_x: cx, target_y: cy, linewise: true, inclusive: false })
            }
            b'w' => {
                for _ in 0..count {
                    let (nx, ny) = self.word_forward(cx, cy, false);
                    cx = nx;
                    cy = ny;
                }
                Some(MotionResult { target_x: cx, target_y: cy, linewise: false, inclusive: false })
            }
            b'W' => {
                for _ in 0..count {
                    let (nx, ny) = self.word_forward(cx, cy, true);
                    cx = nx;
                    cy = ny;
                }
                Some(MotionResult { target_x: cx, target_y: cy, linewise: false, inclusive: false })
            }
            b'b' => {
                for _ in 0..count {
                    let (nx, ny) = self.word_backward(cx, cy, false);
                    cx = nx;
                    cy = ny;
                }
                Some(MotionResult { target_x: cx, target_y: cy, linewise: false, inclusive: false })
            }
            b'B' => {
                for _ in 0..count {
                    let (nx, ny) = self.word_backward(cx, cy, true);
                    cx = nx;
                    cy = ny;
                }
                Some(MotionResult { target_x: cx, target_y: cy, linewise: false, inclusive: false })
            }
            b'e' => {
                for _ in 0..count {
                    let (nx, ny) = self.word_end(cx, cy, false);
                    cx = nx;
                    cy = ny;
                }
                Some(MotionResult { target_x: cx, target_y: cy, linewise: false, inclusive: true })
            }
            b'E' => {
                for _ in 0..count {
                    let (nx, ny) = self.word_end(cx, cy, true);
                    cx = nx;
                    cy = ny;
                }
                Some(MotionResult { target_x: cx, target_y: cy, linewise: false, inclusive: true })
            }
            b'0' => {
                Some(MotionResult { target_x: 0, target_y: cy, linewise: false, inclusive: false })
            }
            b'$' => {
                // With count, go count-1 lines down and to end
                for _ in 1..count {
                    if cy + 1 < self.num_lines { cy += 1; }
                }
                let end = if self.line_lengths[cy] > 0 { self.line_lengths[cy] - 1 } else { 0 };
                Some(MotionResult { target_x: end, target_y: cy, linewise: false, inclusive: true })
            }
            b'^' => {
                let line = &self.lines[cy][..self.line_lengths[cy]];
                let mut x = 0;
                while x < line.len() && (line[x] == b' ' || line[x] == b'\t') {
                    x += 1;
                }
                if x >= line.len() && !line.is_empty() {
                    x = line.len() - 1;
                }
                Some(MotionResult { target_x: x, target_y: cy, linewise: false, inclusive: false })
            }
            b'G' => {
                // Go to line number (count) or last line
                if self.pending.has_count() {
                    cy = (count - 1).min(self.num_lines - 1);
                } else {
                    cy = self.num_lines - 1;
                }
                Some(MotionResult { target_x: 0, target_y: cy, linewise: true, inclusive: false })
            }
            b'{' => {
                for _ in 0..count {
                    cy = self.paragraph_backward(cy);
                }
                Some(MotionResult { target_x: 0, target_y: cy, linewise: true, inclusive: false })
            }
            b'}' => {
                for _ in 0..count {
                    cy = self.paragraph_forward(cy);
                }
                Some(MotionResult { target_x: 0, target_y: cy, linewise: true, inclusive: false })
            }
            b'%' => {
                if let Some((mx, my)) = self.find_matching_bracket() {
                    Some(MotionResult { target_x: mx, target_y: my, linewise: false, inclusive: true })
                } else {
                    None
                }
            }
            b'H' => {
                // Top of screen
                cy = self.scroll_offset;
                Some(MotionResult { target_x: 0, target_y: cy, linewise: true, inclusive: false })
            }
            b'M' => {
                // Middle of screen
                let th = self.text_height();
                cy = self.scroll_offset + th / 2;
                if cy >= self.num_lines { cy = self.num_lines - 1; }
                Some(MotionResult { target_x: 0, target_y: cy, linewise: true, inclusive: false })
            }
            b'L' => {
                // Bottom of screen
                let th = self.text_height();
                cy = (self.scroll_offset + th - 1).min(self.num_lines - 1);
                Some(MotionResult { target_x: 0, target_y: cy, linewise: true, inclusive: false })
            }
            _ => None,
        }
    }

    // === Text Modification Helpers ===

    fn insert_char_at(&mut self, x: usize, y: usize, c: u8) {
        if y >= self.num_lines || self.line_lengths[y] >= MAX_LINE_LEN - 1 {
            return;
        }
        let len = self.line_lengths[y];
        let x = x.min(len);
        // Shift right
        for i in (x..len).rev() {
            self.lines[y][i + 1] = self.lines[y][i];
        }
        self.lines[y][x] = c;
        self.line_lengths[y] += 1;
        self.modified = true;
    }

    fn delete_char_at(&mut self, x: usize, y: usize) -> Option<u8> {
        if y >= self.num_lines || x >= self.line_lengths[y] {
            return None;
        }
        let ch = self.lines[y][x];
        let len = self.line_lengths[y];
        for i in x..len - 1 {
            self.lines[y][i] = self.lines[y][i + 1];
        }
        self.line_lengths[y] -= 1;
        self.modified = true;
        Some(ch)
    }

    fn insert_line_below(&mut self, y: usize, content: &[u8]) {
        if self.num_lines >= MAX_LINES {
            return;
        }
        let insert_at = y + 1;
        // Shift lines down
        for i in (insert_at..self.num_lines).rev() {
            self.lines[i + 1] = self.lines[i];
            self.line_lengths[i + 1] = self.line_lengths[i];
        }
        let len = content.len().min(MAX_LINE_LEN);
        self.lines[insert_at][..len].copy_from_slice(&content[..len]);
        self.line_lengths[insert_at] = len;
        self.num_lines += 1;
        self.modified = true;
    }

    fn insert_line_at(&mut self, y: usize, content: &[u8]) {
        if self.num_lines >= MAX_LINES || y > self.num_lines {
            return;
        }
        // Shift lines down
        for i in (y..self.num_lines).rev() {
            self.lines[i + 1] = self.lines[i];
            self.line_lengths[i + 1] = self.line_lengths[i];
        }
        let len = content.len().min(MAX_LINE_LEN);
        self.lines[y][..len].copy_from_slice(&content[..len]);
        self.line_lengths[y] = len;
        self.num_lines += 1;
        self.modified = true;
    }

    fn delete_line(&mut self, y: usize) {
        if y >= self.num_lines {
            return;
        }
        for i in y..self.num_lines - 1 {
            self.lines[i] = self.lines[i + 1];
            self.line_lengths[i] = self.line_lengths[i + 1];
        }
        self.num_lines -= 1;
        if self.num_lines == 0 {
            self.num_lines = 1;
            self.line_lengths[0] = 0;
        }
        self.modified = true;
    }

    fn split_line(&mut self, x: usize, y: usize) {
        if y >= self.num_lines || self.num_lines >= MAX_LINES {
            return;
        }
        let x = x.min(self.line_lengths[y]);
        let remainder_len = self.line_lengths[y] - x;
        let mut remainder = [0u8; MAX_LINE_LEN];
        remainder[..remainder_len].copy_from_slice(&self.lines[y][x..self.line_lengths[y]]);
        self.line_lengths[y] = x;

        // Insert new line below
        for i in (y + 1..self.num_lines).rev() {
            self.lines[i + 1] = self.lines[i];
            self.line_lengths[i + 1] = self.line_lengths[i];
        }
        self.lines[y + 1][..remainder_len].copy_from_slice(&remainder[..remainder_len]);
        self.line_lengths[y + 1] = remainder_len;
        self.num_lines += 1;
        self.modified = true;
    }

    fn join_line_with_next(&mut self, y: usize) {
        if y + 1 >= self.num_lines {
            return;
        }
        let len1 = self.line_lengths[y];
        let len2 = self.line_lengths[y + 1];

        // Add a space between if line doesn't end with space and next doesn't start with space
        let need_space = len1 > 0 && len2 > 0
            && self.lines[y][len1 - 1] != b' '
            && self.lines[y + 1][0] != b' ';

        let total = len1 + if need_space { 1 } else { 0 } + len2;
        if total > MAX_LINE_LEN {
            return;
        }

        if need_space {
            self.lines[y][len1] = b' ';
            self.line_lengths[y] = len1 + 1;
        }
        let new_len = self.line_lengths[y];
        // Copy via temp buffer to satisfy borrow checker
        let mut temp = [0u8; MAX_LINE_LEN];
        temp[..len2].copy_from_slice(&self.lines[y + 1][..len2]);
        self.lines[y][new_len..new_len + len2].copy_from_slice(&temp[..len2]);
        self.line_lengths[y] = new_len + len2;

        // Remove line y+1
        self.delete_line(y + 1);
    }

    // === Operator Execution ===

    fn execute_operator_motion(&mut self, op: Operator, motion: MotionResult) {
        let (start_y, start_x, end_y, end_x);

        if motion.linewise {
            start_y = self.cursor_y.min(motion.target_y);
            end_y = self.cursor_y.max(motion.target_y);
            start_x = 0;
            end_x = 0;
            self.execute_operator_linewise(op, start_y, end_y);
        } else {
            // Charwise
            if motion.target_y < self.cursor_y || (motion.target_y == self.cursor_y && motion.target_x < self.cursor_x) {
                start_x = motion.target_x;
                start_y = motion.target_y;
                end_x = self.cursor_x;
                end_y = self.cursor_y;
            } else {
                start_x = self.cursor_x;
                start_y = self.cursor_y;
                end_x = motion.target_x;
                end_y = motion.target_y;
            }

            let final_end_x = if motion.inclusive { end_x + 1 } else { end_x };
            self.execute_operator_charwise(op, start_x, start_y, final_end_x, end_y);
        }
    }

    fn execute_operator_linewise(&mut self, op: Operator, start_y: usize, end_y: usize) {
        match op {
            Operator::Delete => {
                self.save_undo();
                self.delete_lines_to_register(start_y, end_y);
                let count = end_y - start_y + 1;
                for _ in 0..count {
                    self.delete_line(start_y);
                }
                self.cursor_y = start_y.min(if self.num_lines > 0 { self.num_lines - 1 } else { 0 });
                self.move_to_first_non_blank();
                if count > 1 {
                    self.set_status_msg(&format!("{} fewer lines", count));
                }
            }
            Operator::Yank => {
                self.yank_lines_to_register(start_y, end_y);
                let count = end_y - start_y + 1;
                self.cursor_y = start_y;
                self.move_to_first_non_blank();
                if count > 1 {
                    self.set_status_msg(&format!("{} lines yanked", count));
                }
            }
            Operator::Change => {
                self.save_undo();
                self.delete_lines_to_register(start_y, end_y);
                let count = end_y - start_y + 1;
                for _ in 0..count {
                    self.delete_line(start_y);
                }
                // Insert an empty line and enter insert mode
                if start_y >= self.num_lines {
                    self.insert_line_at(self.num_lines, &[]);
                    self.cursor_y = self.num_lines - 1;
                } else {
                    self.insert_line_at(start_y, &[]);
                    self.cursor_y = start_y;
                }
                self.cursor_x = 0;
                self.enter_insert_mode();
            }
            Operator::Indent => {
                self.save_undo();
                for y in start_y..=end_y.min(self.num_lines - 1) {
                    if self.line_lengths[y] > 0 && self.line_lengths[y] + TAB_WIDTH <= MAX_LINE_LEN {
                        // Shift content right
                        for i in (0..self.line_lengths[y]).rev() {
                            self.lines[y][i + TAB_WIDTH] = self.lines[y][i];
                        }
                        for i in 0..TAB_WIDTH {
                            self.lines[y][i] = b' ';
                        }
                        self.line_lengths[y] += TAB_WIDTH;
                    }
                }
                self.modified = true;
                self.cursor_y = start_y;
                self.move_to_first_non_blank();
            }
            Operator::Dedent => {
                self.save_undo();
                for y in start_y..=end_y.min(self.num_lines - 1) {
                    let line = &self.lines[y][..self.line_lengths[y]];
                    let mut spaces = 0;
                    while spaces < line.len() && spaces < TAB_WIDTH && line[spaces] == b' ' {
                        spaces += 1;
                    }
                    if spaces > 0 {
                        for i in spaces..self.line_lengths[y] {
                            self.lines[y][i - spaces] = self.lines[y][i];
                        }
                        self.line_lengths[y] -= spaces;
                    }
                }
                self.modified = true;
                self.cursor_y = start_y;
                self.move_to_first_non_blank();
            }
            Operator::None => {}
        }
    }

    fn execute_operator_charwise(&mut self, op: Operator, sx: usize, sy: usize, ex: usize, ey: usize) {
        match op {
            Operator::Delete | Operator::Change => {
                self.save_undo();
                let deleted = self.extract_range(sx, sy, ex, ey);
                self.delete_to_register(&deleted, false);
                self.delete_range(sx, sy, ex, ey);
                self.cursor_x = sx;
                self.cursor_y = sy;
                self.clamp_cursor();
                if op == Operator::Change {
                    self.enter_insert_mode();
                }
            }
            Operator::Yank => {
                let yanked = self.extract_range(sx, sy, ex, ey);
                self.yank_to_register(&yanked, false);
                self.cursor_x = sx;
                self.cursor_y = sy;
            }
            Operator::Indent | Operator::Dedent => {
                // For charwise indent, treat as linewise
                self.execute_operator_linewise(op, sy, ey);
            }
            Operator::None => {}
        }
    }

    fn extract_range(&self, sx: usize, sy: usize, ex: usize, ey: usize) -> Vec<u8> {
        let mut result = Vec::new();
        if sy == ey {
            let end = ex.min(self.line_lengths[sy]);
            let start = sx.min(end);
            result.extend_from_slice(&self.lines[sy][start..end]);
        } else {
            // First line from sx to end
            result.extend_from_slice(&self.lines[sy][sx..self.line_lengths[sy]]);
            result.push(b'\n');
            // Middle lines
            for y in (sy + 1)..ey {
                result.extend_from_slice(&self.lines[y][..self.line_lengths[y]]);
                result.push(b'\n');
            }
            // Last line from start to ex
            let end = ex.min(self.line_lengths[ey]);
            result.extend_from_slice(&self.lines[ey][..end]);
        }
        result
    }

    fn delete_range(&mut self, sx: usize, sy: usize, ex: usize, ey: usize) {
        if sy == ey {
            // Same line
            let end = ex.min(self.line_lengths[sy]);
            let start = sx.min(end);
            let remaining = self.line_lengths[sy] - end;
            for i in 0..remaining {
                self.lines[sy][start + i] = self.lines[sy][end + i];
            }
            self.line_lengths[sy] = start + remaining;
        } else {
            // Keep beginning of first line + end of last line
            let end_remaining = self.line_lengths[ey] - ex.min(self.line_lengths[ey]);
            let new_len = sx + end_remaining;
            if new_len <= MAX_LINE_LEN {
                for i in 0..end_remaining {
                    self.lines[sy][sx + i] = self.lines[ey][ex.min(self.line_lengths[ey]) + i];
                }
                self.line_lengths[sy] = new_len;
            }
            // Delete lines sy+1 through ey
            let lines_to_delete = ey - sy;
            for _ in 0..lines_to_delete {
                self.delete_line(sy + 1);
            }
        }
        self.modified = true;
    }

    // === Put (Paste) ===

    fn put_after(&mut self) {
        if self.registers[0].is_empty() {
            return;
        }
        self.save_undo();
        let reg = self.registers[0].clone();
        if reg.linewise {
            for (i, line_data) in reg.lines.iter().enumerate() {
                self.insert_line_below(self.cursor_y + i, line_data);
            }
            self.cursor_y += 1;
            self.move_to_first_non_blank();
        } else {
            let data = reg.to_bytes();
            if data.contains(&b'\n') {
                // Multi-line charwise paste
                let parts: Vec<&[u8]> = data.split(|&b| b == b'\n').collect();
                // Insert first part after cursor
                let cx = (self.cursor_x + 1).min(self.line_lengths[self.cursor_y]);
                for (i, &part) in parts.iter().enumerate() {
                    if i == 0 {
                        // Insert into current line after cursor
                        let line_len = self.line_lengths[self.cursor_y];
                        let after = line_len - cx;
                        // Move rest of line to a temp buffer
                        let mut rest = [0u8; MAX_LINE_LEN];
                        rest[..after].copy_from_slice(&self.lines[self.cursor_y][cx..line_len]);
                        // Place first part
                        let plen = part.len().min(MAX_LINE_LEN - cx);
                        self.lines[self.cursor_y][cx..cx + plen].copy_from_slice(&part[..plen]);
                        self.line_lengths[self.cursor_y] = cx + plen;
                        if parts.len() > 1 {
                            // Rest goes to new last line - but we need to add it after all middle parts
                            // Store it temporarily
                            let _ = rest; // will handle below
                            // Split: current line ends here, rest goes to new line after all parts
                            let last_idx = parts.len() - 1;
                            // Insert middle lines
                            for j in 1..last_idx {
                                self.insert_line_below(self.cursor_y + j - 1, parts[j]);
                            }
                            // Last part + rest of original line
                            let last_part = parts[last_idx];
                            let mut final_line = Vec::new();
                            final_line.extend_from_slice(last_part);
                            final_line.extend_from_slice(&rest[..after]);
                            self.insert_line_below(self.cursor_y + last_idx - 1, &final_line);
                            self.cursor_y += last_idx;
                            self.cursor_x = last_part.len();
                        }
                    }
                }
            } else {
                // Single line charwise paste: insert after cursor
                let cx = (self.cursor_x + 1).min(self.line_lengths[self.cursor_y]);
                for (i, &byte) in data.iter().enumerate() {
                    if cx + i < MAX_LINE_LEN && self.line_lengths[self.cursor_y] < MAX_LINE_LEN {
                        self.insert_char_at(cx + i, self.cursor_y, byte);
                    }
                }
                self.cursor_x = cx + data.len() - 1;
            }
        }
        self.modified = true;
    }

    fn put_before(&mut self) {
        if self.registers[0].is_empty() {
            return;
        }
        self.save_undo();
        let reg = self.registers[0].clone();
        if reg.linewise {
            for (i, line_data) in reg.lines.iter().enumerate() {
                self.insert_line_at(self.cursor_y + i, line_data);
            }
            self.move_to_first_non_blank();
        } else {
            let data = reg.to_bytes();
            let cx = self.cursor_x;
            for (i, &byte) in data.iter().enumerate() {
                if cx + i < MAX_LINE_LEN && self.line_lengths[self.cursor_y] < MAX_LINE_LEN {
                    self.insert_char_at(cx + i, self.cursor_y, byte);
                }
            }
            if !data.is_empty() {
                self.cursor_x = cx + data.len() - 1;
            }
        }
        self.modified = true;
    }

    // === Search ===

    fn search_forward_from(&self, query: &[u8], start_x: usize, start_y: usize) -> Option<(usize, usize)> {
        if query.is_empty() {
            return None;
        }
        // Search from start_x+1 on start_y, then subsequent lines, then wrap
        let mut y = start_y;
        let mut x = start_x + 1;
        let total_lines = self.num_lines;

        for _ in 0..total_lines + 1 {
            let line = &self.lines[y][..self.line_lengths[y]];
            if let Some(pos) = Self::find_in_bytes(&line[x..], query) {
                return Some((x + pos, y));
            }
            y = (y + 1) % self.num_lines;
            x = 0;
        }
        None
    }

    fn search_backward_from(&self, query: &[u8], start_x: usize, start_y: usize) -> Option<(usize, usize)> {
        if query.is_empty() {
            return None;
        }
        // Search from start_x-1 on start_y backward, then previous lines, then wrap
        let mut y = start_y;
        let mut search_end = start_x;

        for _ in 0..self.num_lines + 1 {
            let line = &self.lines[y][..self.line_lengths[y]];
            // Search in line[0..search_end]
            if let Some(pos) = Self::rfind_in_bytes(&line[..search_end], query) {
                return Some((pos, y));
            }
            if y == 0 {
                y = self.num_lines - 1;
            } else {
                y -= 1;
            }
            search_end = self.line_lengths[y];
        }
        None
    }

    fn find_in_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        if needle.is_empty() || needle.len() > haystack.len() {
            return None;
        }
        for i in 0..=haystack.len() - needle.len() {
            if &haystack[i..i + needle.len()] == needle {
                return Some(i);
            }
        }
        None
    }

    fn rfind_in_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        if needle.is_empty() || needle.len() > haystack.len() {
            return None;
        }
        for i in (0..=haystack.len() - needle.len()).rev() {
            if &haystack[i..i + needle.len()] == needle {
                return Some(i);
            }
        }
        None
    }

    fn execute_search(&mut self) {
        let query = self.search_query.clone();
        self.last_search_query = query.clone();
        self.last_search_forward = self.search_forward;

        let result = if self.search_forward {
            self.search_forward_from(&query, self.cursor_x, self.cursor_y)
        } else {
            self.search_backward_from(&query, self.cursor_x, self.cursor_y)
        };

        if let Some((x, y)) = result {
            self.cursor_x = x;
            self.cursor_y = y;
            self.desired_x = x;
            self.ensure_cursor_visible();
        } else {
            let q = String::from_utf8_lossy(&query).to_string();
            self.set_status_msg(&format!("E486: Pattern not found: {}", q));
        }
    }

    fn search_next(&mut self) {
        if self.last_search_query.is_empty() {
            self.set_status_msg("E486: No previous search pattern");
            return;
        }
        let query = self.last_search_query.clone();
        let result = if self.last_search_forward {
            self.search_forward_from(&query, self.cursor_x, self.cursor_y)
        } else {
            self.search_backward_from(&query, self.cursor_x, self.cursor_y)
        };
        if let Some((x, y)) = result {
            self.cursor_x = x;
            self.cursor_y = y;
            self.desired_x = x;
            self.ensure_cursor_visible();
        } else {
            let q = String::from_utf8_lossy(&query).to_string();
            self.set_status_msg(&format!("E486: Pattern not found: {}", q));
        }
    }

    fn search_prev(&mut self) {
        if self.last_search_query.is_empty() {
            self.set_status_msg("E486: No previous search pattern");
            return;
        }
        let query = self.last_search_query.clone();
        let result = if self.last_search_forward {
            self.search_backward_from(&query, self.cursor_x, self.cursor_y)
        } else {
            self.search_forward_from(&query, self.cursor_x, self.cursor_y)
        };
        if let Some((x, y)) = result {
            self.cursor_x = x;
            self.cursor_y = y;
            self.desired_x = x;
            self.ensure_cursor_visible();
        } else {
            let q = String::from_utf8_lossy(&query).to_string();
            self.set_status_msg(&format!("E486: Pattern not found: {}", q));
        }
    }

    fn search_word_under_cursor(&mut self, forward: bool) {
        let line = &self.lines[self.cursor_y][..self.line_lengths[self.cursor_y]];
        if line.is_empty() || self.cursor_x >= line.len() {
            return;
        }
        let x = self.cursor_x;
        if !Self::is_word_char(line[x]) {
            return;
        }
        let mut start = x;
        let mut end = x;
        while start > 0 && Self::is_word_char(line[start - 1]) {
            start -= 1;
        }
        while end + 1 < line.len() && Self::is_word_char(line[end + 1]) {
            end += 1;
        }
        self.last_search_query = line[start..=end].to_vec();
        self.last_search_forward = forward;
        self.search_forward = forward;
        self.search_next();
    }

    // === Substitution ===

    fn substitute(&mut self, range_all: bool, pattern: &str, replacement: &str, global: bool) {
        self.save_undo();
        let pat = pattern.as_bytes();
        let rep = replacement.as_bytes();
        let (start_y, end_y) = if range_all {
            (0, self.num_lines - 1)
        } else {
            (self.cursor_y, self.cursor_y)
        };

        let mut total_subs = 0;
        for y in start_y..=end_y {
            loop {
                let line = &self.lines[y][..self.line_lengths[y]];
                if let Some(pos) = Self::find_in_bytes(line, pat) {
                    // Replace
                    let after_pat = pos + pat.len();
                    let remaining = self.line_lengths[y] - after_pat;
                    let new_len = pos + rep.len() + remaining;
                    if new_len > MAX_LINE_LEN {
                        break;
                    }
                    // Shift content after pattern
                    if rep.len() != pat.len() {
                        let mut temp = [0u8; MAX_LINE_LEN];
                        temp[..remaining].copy_from_slice(&self.lines[y][after_pat..self.line_lengths[y]]);
                        self.lines[y][pos + rep.len()..pos + rep.len() + remaining].copy_from_slice(&temp[..remaining]);
                    }
                    self.lines[y][pos..pos + rep.len()].copy_from_slice(rep);
                    self.line_lengths[y] = new_len;
                    total_subs += 1;
                    if !global {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
        self.modified = true;
        self.set_status_msg(&format!("{} substitution(s)", total_subs));
    }

    // === Mode Transitions ===

    fn enter_insert_mode(&mut self) {
        self.mode = Mode::Insert;
        self.insert_start_x = self.cursor_x;
        self.insert_start_y = self.cursor_y;
        if !self.replaying_dot {
            self.recording_dot = true;
            self.current_dot.clear();
        }
        self.status_msg.clear();
    }

    fn exit_insert_mode(&mut self) {
        self.mode = Mode::Normal;
        // Move cursor left 1 (VIM behavior)
        if self.cursor_x > 0 {
            self.cursor_x -= 1;
        }
        self.clamp_cursor();
        self.recording_dot = false;
        if !self.replaying_dot {
            // Finalize dot command
            self.dot_command = self.current_dot.clone();
        }
        self.status_msg.clear();
    }

    fn enter_visual_mode(&mut self, line_mode: bool) {
        if line_mode {
            self.mode = Mode::VisualLine;
        } else {
            self.mode = Mode::Visual;
        }
        self.visual_start_x = self.cursor_x;
        self.visual_start_y = self.cursor_y;
        self.status_msg.clear();
    }

    fn exit_visual_mode(&mut self) {
        self.mode = Mode::Normal;
        self.status_msg.clear();
    }

    fn enter_command_mode(&mut self) {
        self.mode = Mode::Command;
        self.cmdline.clear();
    }

    fn enter_search_mode(&mut self, forward: bool) {
        self.mode = Mode::Search;
        self.search_query.clear();
        self.search_forward = forward;
    }

    // === Visual Mode Helpers ===

    fn visual_range(&self) -> (usize, usize, usize, usize) {
        if self.mode == Mode::VisualLine {
            let start_y = self.visual_start_y.min(self.cursor_y);
            let end_y = self.visual_start_y.max(self.cursor_y);
            (0, start_y, self.line_lengths[end_y], end_y)
        } else {
            let (sy, sx, ey, ex);
            if self.visual_start_y < self.cursor_y
                || (self.visual_start_y == self.cursor_y && self.visual_start_x <= self.cursor_x)
            {
                sy = self.visual_start_y;
                sx = self.visual_start_x;
                ey = self.cursor_y;
                ex = self.cursor_x;
            } else {
                sy = self.cursor_y;
                sx = self.cursor_x;
                ey = self.visual_start_y;
                ex = self.visual_start_x;
            }
            (sx, sy, ex + 1, ey) // +1 because visual includes cursor char
        }
    }

    fn is_in_visual_selection(&self, x: usize, y: usize) -> bool {
        let (sx, sy, ex, ey) = self.visual_range();
        if self.mode == Mode::VisualLine {
            y >= sy && y <= ey
        } else {
            if y < sy || y > ey {
                return false;
            }
            if sy == ey {
                x >= sx && x < ex
            } else if y == sy {
                x >= sx
            } else if y == ey {
                x < ex
            } else {
                true
            }
        }
    }

    // === Command Execution ===

    fn execute_command(&mut self) {
        let cmd = String::from_utf8_lossy(&self.cmdline).to_string();
        let cmd = cmd.trim();

        if cmd.is_empty() {
            self.mode = Mode::Normal;
            return;
        }

        // :w - save
        if cmd == "w" {
            self.save_file();
        } else if cmd.starts_with("w ") {
            let name = cmd[2..].trim();
            self.save_file_as(name);
        }
        // :q - quit
        else if cmd == "q" {
            if self.modified {
                self.set_status_msg("E37: No write since last change (add ! to override)");
            } else {
                self.exit_requested = true;
            }
        } else if cmd == "q!" {
            self.exit_requested = true;
        }
        // :wq or :x - save and quit
        else if cmd == "wq" || cmd == "x" {
            if self.save_file() {
                self.exit_requested = true;
            }
        }
        // :e file - edit file
        else if cmd.starts_with("e ") {
            let name = cmd[2..].trim();
            if self.modified {
                self.set_status_msg("E37: No write since last change (add ! to override)");
            } else {
                self.open(name);
            }
        } else if cmd.starts_with("e! ") {
            let name = cmd[3..].trim();
            self.open(name);
        }
        // :set options
        else if cmd == "set nu" || cmd == "set number" {
            self.show_line_numbers = true;
        } else if cmd == "set nonu" || cmd == "set nonumber" {
            self.show_line_numbers = false;
        }
        // :N - go to line N
        else if cmd.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(n) = cmd.parse::<usize>() {
                if n > 0 && n <= self.num_lines {
                    self.cursor_y = n - 1;
                    self.move_to_first_non_blank();
                    self.ensure_cursor_visible();
                }
            }
        }
        // :%s/old/new/g - substitution
        else if cmd.starts_with("%s/") || cmd.starts_with("s/") {
            let range_all = cmd.starts_with("%");
            let s = if range_all { &cmd[3..] } else { &cmd[2..] };
            // Parse /pattern/replacement/flags
            let parts: Vec<&str> = s.splitn(3, '/').collect();
            if parts.len() >= 2 {
                let pattern = parts[0];
                let replacement = parts[1];
                let flags = if parts.len() > 2 { parts[2] } else { "" };
                let global = flags.contains('g');
                self.substitute(range_all, pattern, replacement, global);
            } else {
                self.set_status_msg("E146: Invalid substitution");
            }
        }
        else {
            self.set_status_msg(&format!("E492: Not an editor command: {}", cmd));
        }

        if !self.exit_requested {
            self.mode = Mode::Normal;
        }
    }

    // === Dot Repeat ===

    fn record_dot_byte(&mut self, byte: u8) {
        if self.recording_dot && !self.replaying_dot {
            self.current_dot.push(byte);
        }
    }

    fn replay_dot(&mut self) {
        if self.dot_command.is_empty() {
            return;
        }
        self.replaying_dot = true;
        let cmd = self.dot_command.clone();
        for &byte in &cmd {
            self.process_byte(byte);
        }
        self.replaying_dot = false;
    }

    // === Main Input Handler ===

    pub fn handle_input(&mut self, input: &str) {
        let bytes = input.as_bytes();

        for &byte in bytes {
            // Check for Ctrl+T termination — handled by lib.rs, but double check
            if byte == 0x14 {
                return;
            }

            match self.esc_state {
                EscState::None => {
                    if byte == 27 {
                        // ESC byte received
                        self.esc_state = EscState::Escape;
                        self.esc_len = 0;
                    } else {
                        self.process_byte(byte);
                    }
                }
                EscState::Escape => {
                    if byte == b'[' {
                        self.esc_state = EscState::Csi;
                        self.esc_len = 0;
                    } else {
                        // Bare ESC — process as escape keypress
                        self.process_byte(27);
                        // Then process this byte as a new input
                        self.esc_state = EscState::None;
                        if byte == 27 {
                            self.esc_state = EscState::Escape;
                        } else {
                            self.process_byte(byte);
                        }
                    }
                }
                EscState::Csi => {
                    if byte.is_ascii_digit() || byte == b';' {
                        if self.esc_len < self.esc_buf.len() {
                            self.esc_buf[self.esc_len] = byte;
                            self.esc_len += 1;
                        }
                    } else {
                        // CSI sequence complete
                        self.esc_state = EscState::None;
                        match byte {
                            b'A' => self.process_special_key(SpecialKey::Up),
                            b'B' => self.process_special_key(SpecialKey::Down),
                            b'C' => self.process_special_key(SpecialKey::Right),
                            b'D' => self.process_special_key(SpecialKey::Left),
                            b'H' => self.process_special_key(SpecialKey::Home),
                            b'F' => self.process_special_key(SpecialKey::End),
                            b'~' => {
                                // Check which ~ sequence
                                if self.esc_len > 0 && self.esc_buf[0] == b'3' {
                                    self.process_special_key(SpecialKey::Delete);
                                }
                            }
                            _ => {} // Unknown CSI sequence, ignore
                        }
                    }
                }
            }
        }

        // If we ended in Escape state and there are no more bytes in this input,
        // we need to handle the possibility that this is a bare ESC.
        // Since input arrives per-keystroke, if we got ESC alone in one call,
        // the next call will have the next key. We use a heuristic:
        // if input was exactly 1 byte (ESC), treat it as bare ESC.
        if self.esc_state == EscState::Escape && bytes.len() == 1 && bytes[0] == 27 {
            self.esc_state = EscState::None;
            self.process_byte(27);
        }
    }

    fn process_special_key(&mut self, key: SpecialKey) {
        match self.mode {
            Mode::Insert => {
                match key {
                    SpecialKey::Up => self.move_up(),
                    SpecialKey::Down => self.move_down(),
                    SpecialKey::Left => self.move_left(),
                    SpecialKey::Right => {
                        if self.cursor_x < self.line_lengths[self.cursor_y] {
                            self.cursor_x += 1;
                            self.desired_x = self.cursor_x;
                        }
                    }
                    SpecialKey::Home => self.move_to_line_start(),
                    SpecialKey::End => {
                        self.cursor_x = self.line_lengths[self.cursor_y];
                        self.desired_x = self.cursor_x;
                    }
                    SpecialKey::Delete => {
                        self.delete_char_at(self.cursor_x, self.cursor_y);
                    }
                }
                self.ensure_cursor_visible();
                self.render();
            }
            Mode::Normal | Mode::Visual | Mode::VisualLine => {
                match key {
                    SpecialKey::Up => self.move_up(),
                    SpecialKey::Down => self.move_down(),
                    SpecialKey::Left => self.move_left(),
                    SpecialKey::Right => self.move_right(),
                    SpecialKey::Home => self.move_to_line_start(),
                    SpecialKey::End => self.move_to_line_end(),
                    SpecialKey::Delete => {
                        if self.mode == Mode::Normal {
                            self.save_undo();
                            if let Some(ch) = self.delete_char_at(self.cursor_x, self.cursor_y) {
                                self.delete_to_register(&[ch], false);
                            }
                            self.clamp_cursor();
                        }
                    }
                }
                self.ensure_cursor_visible();
                self.render();
            }
            Mode::Command | Mode::Search => {
                // Arrow keys in command/search mode: ignore for now
                self.render();
            }
            Mode::Replace => {
                self.mode = Mode::Normal;
                self.render();
            }
        }
    }

    fn process_byte(&mut self, byte: u8) {
        match self.mode {
            Mode::Normal => self.process_normal(byte),
            Mode::Insert => self.process_insert(byte),
            Mode::Visual | Mode::VisualLine => self.process_visual(byte),
            Mode::Command => self.process_command_mode(byte),
            Mode::Search => self.process_search_mode(byte),
            Mode::Replace => self.process_replace(byte),
        }
    }

    // === Normal Mode ===

    fn process_normal(&mut self, byte: u8) {
        // Handle pending text object prefix (i/a after operator)
        if self.pending.text_obj_prefix != 0 {
            let around = self.pending.text_obj_prefix == b'a';
            self.pending.text_obj_prefix = 0;
            let op = self.pending.operator;

            let range = match byte {
                b'w' => self.text_object_word(around),
                b'"' => self.text_object_quote(b'"', around),
                b'\'' => self.text_object_quote(b'\'', around),
                b'(' | b')' => self.text_object_delimited(b'(', b')', around),
                b'[' | b']' => self.text_object_delimited(b'[', b']', around),
                b'{' | b'}' => self.text_object_delimited(b'{', b'}', around),
                b'<' | b'>' => self.text_object_delimited(b'<', b'>', around),
                _ => None,
            };

            if let Some((sx, sy, ex, ey)) = range {
                self.pending.clear();
                if sy == ey && sx > ex {
                    // Empty range
                } else {
                    self.execute_operator_charwise(op, sx, sy, ex + 1, ey);
                }
            } else {
                self.pending.clear();
            }
            self.render();
            return;
        }

        // Handle pending f/F/t/T
        if self.pending.f_char.is_some() {
            let f_type = self.pending.f_type;
            let count = self.pending.effective_count();
            self.pending.f_char = None;

            let result = match f_type {
                b'f' => self.find_char_forward(byte, self.cursor_x, self.cursor_y, false),
                b'F' => self.find_char_backward(byte, self.cursor_x, self.cursor_y, false),
                b't' => self.find_char_forward(byte, self.cursor_x, self.cursor_y, true),
                b'T' => self.find_char_backward(byte, self.cursor_x, self.cursor_y, true),
                _ => None,
            };

            if let Some(target_x) = result {
                self.last_find_char = byte;
                self.last_find_type = f_type;

                if self.pending.operator != Operator::None {
                    let inclusive = f_type == b'f' || f_type == b'F';
                    let motion = MotionResult {
                        target_x,
                        target_y: self.cursor_y,
                        linewise: false,
                        inclusive,
                    };
                    let op = self.pending.operator;
                    self.pending.clear();
                    self.execute_operator_motion(op, motion);
                } else {
                    self.cursor_x = target_x;
                    self.desired_x = target_x;
                    self.pending.clear();
                }
            } else {
                self.pending.clear();
            }
            self.render();
            return;
        }

        // Handle pending g prefix
        if self.pending.g_prefix {
            self.pending.g_prefix = false;
            match byte {
                b'g' => {
                    // gg - go to first line (or line N with count)
                    let target = if self.pending.has_count() {
                        (self.pending.effective_count() - 1).min(self.num_lines - 1)
                    } else {
                        0
                    };
                    if self.pending.operator != Operator::None {
                        let op = self.pending.operator;
                        self.pending.clear();
                        let motion = MotionResult {
                            target_x: 0,
                            target_y: target,
                            linewise: true,
                            inclusive: false,
                        };
                        self.execute_operator_motion(op, motion);
                    } else {
                        self.cursor_y = target;
                        self.move_to_first_non_blank();
                        self.ensure_cursor_visible();
                        self.pending.clear();
                    }
                    self.render();
                    return;
                }
                _ => {
                    self.pending.clear();
                    self.render();
                    return;
                }
            }
        }

        // Handle pending z prefix
        if self.pending.z_prefix {
            self.pending.z_prefix = false;
            match byte {
                b'z' => {
                    // zz - center cursor line on screen
                    let th = self.text_height();
                    if self.cursor_y >= th / 2 {
                        self.scroll_offset = self.cursor_y - th / 2;
                    } else {
                        self.scroll_offset = 0;
                    }
                }
                b't' => {
                    // zt - cursor line to top of screen
                    self.scroll_offset = self.cursor_y;
                }
                b'b' => {
                    // zb - cursor line to bottom of screen
                    let th = self.text_height();
                    if self.cursor_y >= th - 1 {
                        self.scroll_offset = self.cursor_y - th + 1;
                    } else {
                        self.scroll_offset = 0;
                    }
                }
                _ => {}
            }
            self.pending.clear();
            self.render();
            return;
        }

        // Count prefix
        if byte >= b'1' && byte <= b'9' && self.pending.f_char.is_none() {
            let digit = (byte - b'0') as usize;
            if self.pending.operator != Operator::None {
                // Count after operator
                self.pending.count = Some(self.pending.count.unwrap_or(0) * 10 + digit);
            } else {
                self.pending.count = Some(self.pending.count.unwrap_or(0) * 10 + digit);
            }
            return; // Don't render yet
        }
        if byte == b'0' && self.pending.count.is_some() {
            self.pending.count = Some(self.pending.count.unwrap_or(0) * 10);
            return;
        }

        let count = self.pending.effective_count();

        match byte {
            // === Mode entries / text objects ===
            b'i' if self.pending.operator != Operator::None => {
                // Text object prefix: inner
                self.pending.text_obj_prefix = b'i';
                return; // Wait for object type
            }
            b'a' if self.pending.operator != Operator::None => {
                // Text object prefix: around
                self.pending.text_obj_prefix = b'a';
                return; // Wait for object type
            }
            b'i' => {
                if !self.replaying_dot {
                    self.current_dot.clear();
                    self.current_dot.push(b'i');
                }
                self.save_undo();
                self.enter_insert_mode();
                self.pending.clear();
            }
            b'a' => {
                if !self.replaying_dot {
                    self.current_dot.clear();
                    self.current_dot.push(b'a');
                }
                self.save_undo();
                if self.line_lengths[self.cursor_y] > 0 {
                    self.cursor_x += 1;
                }
                self.enter_insert_mode();
                self.pending.clear();
            }
            b'I' => {
                if !self.replaying_dot {
                    self.current_dot.clear();
                    self.current_dot.push(b'I');
                }
                self.save_undo();
                self.move_to_first_non_blank();
                self.enter_insert_mode();
                self.pending.clear();
            }
            b'A' => {
                if !self.replaying_dot {
                    self.current_dot.clear();
                    self.current_dot.push(b'A');
                }
                self.save_undo();
                self.cursor_x = self.line_lengths[self.cursor_y];
                self.enter_insert_mode();
                self.pending.clear();
            }
            b'o' => {
                if !self.replaying_dot {
                    self.current_dot.clear();
                    self.current_dot.push(b'o');
                }
                self.save_undo();
                self.insert_line_below(self.cursor_y, &[]);
                self.cursor_y += 1;
                self.cursor_x = 0;
                self.ensure_cursor_visible();
                self.enter_insert_mode();
                self.pending.clear();
            }
            b'O' => {
                if !self.replaying_dot {
                    self.current_dot.clear();
                    self.current_dot.push(b'O');
                }
                self.save_undo();
                self.insert_line_at(self.cursor_y, &[]);
                self.cursor_x = 0;
                self.enter_insert_mode();
                self.pending.clear();
            }

            // === Visual mode ===
            b'v' => {
                self.enter_visual_mode(false);
                self.pending.clear();
            }
            b'V' => {
                self.enter_visual_mode(true);
                self.pending.clear();
            }

            // === Command/Search mode ===
            b':' => {
                self.enter_command_mode();
                self.pending.clear();
            }
            b'/' => {
                self.enter_search_mode(true);
                self.pending.clear();
            }
            b'?' => {
                self.enter_search_mode(false);
                self.pending.clear();
            }

            // === Motions ===
            b'h' | b'l' | b'j' | b'k' | b'w' | b'W' | b'b' | b'B' | b'e' | b'E'
            | b'$' | b'^' | b'G' | b'{' | b'}' | b'%' | b'H' | b'M' | b'L' => {
                if let Some(motion) = self.compute_motion(byte, count) {
                    if self.pending.operator != Operator::None {
                        let op = self.pending.operator;
                        self.pending.clear();
                        self.execute_operator_motion(op, motion);
                    } else {
                        // Just move
                        self.cursor_x = motion.target_x;
                        self.cursor_y = motion.target_y;
                        if byte != b'j' && byte != b'k' {
                            self.desired_x = self.cursor_x;
                        } else {
                            self.cursor_x = self.desired_x.min(self.max_cursor_x());
                        }
                        self.clamp_cursor();
                        self.ensure_cursor_visible();
                        self.pending.clear();
                    }
                } else {
                    self.pending.clear();
                }
            }

            // 0 can be a motion (start of line) when no count is building
            b'0' => {
                if self.pending.operator != Operator::None {
                    if let Some(motion) = self.compute_motion(b'0', 1) {
                        let op = self.pending.operator;
                        self.pending.clear();
                        self.execute_operator_motion(op, motion);
                    }
                } else {
                    self.move_to_line_start();
                    self.pending.clear();
                }
            }

            // g prefix
            b'g' => {
                self.pending.g_prefix = true;
                return; // Don't render yet
            }

            // z prefix
            b'z' => {
                self.pending.z_prefix = true;
                return;
            }

            // === Operators ===
            b'd' => {
                if self.pending.operator == Operator::Delete {
                    // dd - delete line(s)
                    let c = count;
                    let end = (self.cursor_y + c - 1).min(self.num_lines - 1);
                    self.execute_operator_linewise(Operator::Delete, self.cursor_y, end);
                    self.pending.clear();
                } else if self.pending.operator == Operator::None {
                    self.pending.operator_count = self.pending.count;
                    self.pending.count = None;
                    self.pending.operator = Operator::Delete;
                    return; // Wait for motion
                } else {
                    self.pending.clear();
                }
            }
            b'y' => {
                if self.pending.operator == Operator::Yank {
                    // yy - yank line(s)
                    let c = count;
                    let end = (self.cursor_y + c - 1).min(self.num_lines - 1);
                    self.execute_operator_linewise(Operator::Yank, self.cursor_y, end);
                    self.pending.clear();
                } else if self.pending.operator == Operator::None {
                    self.pending.operator_count = self.pending.count;
                    self.pending.count = None;
                    self.pending.operator = Operator::Yank;
                    return;
                } else {
                    self.pending.clear();
                }
            }
            b'c' => {
                if self.pending.operator == Operator::Change {
                    // cc - change line(s)
                    let c = count;
                    let end = (self.cursor_y + c - 1).min(self.num_lines - 1);
                    self.execute_operator_linewise(Operator::Change, self.cursor_y, end);
                    self.pending.clear();
                } else if self.pending.operator == Operator::None {
                    self.pending.operator_count = self.pending.count;
                    self.pending.count = None;
                    self.pending.operator = Operator::Change;
                    return;
                } else {
                    self.pending.clear();
                }
            }
            b'>' => {
                if self.pending.operator == Operator::Indent {
                    // >> - indent line(s)
                    let c = count;
                    let end = (self.cursor_y + c - 1).min(self.num_lines - 1);
                    self.execute_operator_linewise(Operator::Indent, self.cursor_y, end);
                    self.pending.clear();
                } else if self.pending.operator == Operator::None {
                    self.pending.operator_count = self.pending.count;
                    self.pending.count = None;
                    self.pending.operator = Operator::Indent;
                    return;
                } else {
                    self.pending.clear();
                }
            }
            b'<' => {
                if self.pending.operator == Operator::Dedent {
                    // << - dedent line(s)
                    let c = count;
                    let end = (self.cursor_y + c - 1).min(self.num_lines - 1);
                    self.execute_operator_linewise(Operator::Dedent, self.cursor_y, end);
                    self.pending.clear();
                } else if self.pending.operator == Operator::None {
                    self.pending.operator_count = self.pending.count;
                    self.pending.count = None;
                    self.pending.operator = Operator::Dedent;
                    return;
                } else {
                    self.pending.clear();
                }
            }

            // === Simple commands ===
            b'x' => {
                self.save_undo();
                for _ in 0..count {
                    if self.cursor_x < self.line_lengths[self.cursor_y] {
                        if let Some(ch) = self.delete_char_at(self.cursor_x, self.cursor_y) {
                            self.delete_to_register(&[ch], false);
                        }
                    }
                }
                self.clamp_cursor();
                self.pending.clear();
            }
            b'X' => {
                self.save_undo();
                for _ in 0..count {
                    if self.cursor_x > 0 {
                        self.cursor_x -= 1;
                        if let Some(ch) = self.delete_char_at(self.cursor_x, self.cursor_y) {
                            self.delete_to_register(&[ch], false);
                        }
                    }
                }
                self.pending.clear();
            }
            b'r' => {
                self.mode = Mode::Replace;
                self.pending.clear();
                return; // Wait for replacement char
            }
            b'J' => {
                self.save_undo();
                for _ in 0..count {
                    self.join_line_with_next(self.cursor_y);
                }
                self.pending.clear();
            }
            b'~' => {
                self.save_undo();
                for _ in 0..count {
                    if self.cursor_x < self.line_lengths[self.cursor_y] {
                        let ch = self.lines[self.cursor_y][self.cursor_x];
                        if ch.is_ascii_lowercase() {
                            self.lines[self.cursor_y][self.cursor_x] = ch.to_ascii_uppercase();
                        } else if ch.is_ascii_uppercase() {
                            self.lines[self.cursor_y][self.cursor_x] = ch.to_ascii_lowercase();
                        }
                        self.modified = true;
                        if self.cursor_x < self.max_cursor_x() {
                            self.cursor_x += 1;
                        }
                    }
                }
                self.pending.clear();
            }
            b'D' => {
                // Delete to end of line
                self.save_undo();
                let deleted = self.lines[self.cursor_y][self.cursor_x..self.line_lengths[self.cursor_y]].to_vec();
                self.delete_to_register(&deleted, false);
                self.line_lengths[self.cursor_y] = self.cursor_x;
                self.modified = true;
                self.clamp_cursor();
                self.pending.clear();
            }
            b'C' => {
                // Change to end of line
                self.save_undo();
                let deleted = self.lines[self.cursor_y][self.cursor_x..self.line_lengths[self.cursor_y]].to_vec();
                self.delete_to_register(&deleted, false);
                self.line_lengths[self.cursor_y] = self.cursor_x;
                self.modified = true;
                if !self.replaying_dot {
                    self.current_dot.clear();
                    self.current_dot.push(b'C');
                }
                self.enter_insert_mode();
                self.pending.clear();
            }
            b'Y' => {
                // Yank line (like yy)
                let end = (self.cursor_y + count - 1).min(self.num_lines - 1);
                self.yank_lines_to_register(self.cursor_y, end);
                self.pending.clear();
            }
            b'S' => {
                // Substitute line (like cc)
                self.save_undo();
                let y = self.cursor_y;
                self.delete_lines_to_register(y, y);
                self.line_lengths[y] = 0;
                self.cursor_x = 0;
                self.modified = true;
                if !self.replaying_dot {
                    self.current_dot.clear();
                    self.current_dot.push(b'S');
                }
                self.enter_insert_mode();
                self.pending.clear();
            }
            b's' => {
                // Substitute char: delete char then enter insert mode
                self.save_undo();
                if self.cursor_x < self.line_lengths[self.cursor_y] {
                    if let Some(ch) = self.delete_char_at(self.cursor_x, self.cursor_y) {
                        self.delete_to_register(&[ch], false);
                    }
                }
                if !self.replaying_dot {
                    self.current_dot.clear();
                    self.current_dot.push(b's');
                }
                self.enter_insert_mode();
                self.pending.clear();
            }

            // === Put ===
            b'p' => {
                for _ in 0..count {
                    self.put_after();
                }
                self.pending.clear();
            }
            b'P' => {
                for _ in 0..count {
                    self.put_before();
                }
                self.pending.clear();
            }

            // === Undo/Redo ===
            b'u' => {
                for _ in 0..count {
                    self.undo();
                }
                self.pending.clear();
            }
            18 => {
                // Ctrl+R - redo
                for _ in 0..count {
                    self.redo();
                }
                self.pending.clear();
            }

            // === Dot repeat ===
            b'.' => {
                for _ in 0..count {
                    self.replay_dot();
                }
                self.pending.clear();
            }

            // === Search repeat ===
            b'n' => {
                for _ in 0..count {
                    self.search_next();
                }
                self.pending.clear();
            }
            b'N' => {
                for _ in 0..count {
                    self.search_prev();
                }
                self.pending.clear();
            }
            b'*' => {
                self.search_word_under_cursor(true);
                self.pending.clear();
            }
            b'#' => {
                self.search_word_under_cursor(false);
                self.pending.clear();
            }

            // === f/F/t/T character find ===
            b'f' | b'F' | b't' | b'T' => {
                if self.pending.operator == Operator::None && self.pending.count.is_none() && self.pending.operator_count.is_none() {
                    self.pending.f_char = Some(0); // placeholder
                    self.pending.f_type = byte;
                } else {
                    // Operator + f/F/t/T: wait for target char
                    self.pending.f_char = Some(0);
                    self.pending.f_type = byte;
                }
                return; // Wait for target char
            }
            b';' => {
                // Repeat last f/F/t/T
                if self.last_find_char != 0 {
                    let result = match self.last_find_type {
                        b'f' => self.find_char_forward(self.last_find_char, self.cursor_x, self.cursor_y, false),
                        b'F' => self.find_char_backward(self.last_find_char, self.cursor_x, self.cursor_y, false),
                        b't' => self.find_char_forward(self.last_find_char, self.cursor_x, self.cursor_y, true),
                        b'T' => self.find_char_backward(self.last_find_char, self.cursor_x, self.cursor_y, true),
                        _ => None,
                    };
                    if let Some(x) = result {
                        self.cursor_x = x;
                        self.desired_x = x;
                    }
                }
                self.pending.clear();
            }
            b',' => {
                // Repeat last f/F/t/T in reverse direction
                if self.last_find_char != 0 {
                    let rev_type = match self.last_find_type {
                        b'f' => b'F',
                        b'F' => b'f',
                        b't' => b'T',
                        b'T' => b't',
                        _ => 0,
                    };
                    let result = match rev_type {
                        b'f' => self.find_char_forward(self.last_find_char, self.cursor_x, self.cursor_y, false),
                        b'F' => self.find_char_backward(self.last_find_char, self.cursor_x, self.cursor_y, false),
                        b't' => self.find_char_forward(self.last_find_char, self.cursor_x, self.cursor_y, true),
                        b'T' => self.find_char_backward(self.last_find_char, self.cursor_x, self.cursor_y, true),
                        _ => None,
                    };
                    if let Some(x) = result {
                        self.cursor_x = x;
                        self.desired_x = x;
                    }
                }
                self.pending.clear();
            }

            // === Scroll commands ===
            4 => {
                // Ctrl+D - half page down
                let half = self.text_height() / 2;
                for _ in 0..half {
                    if self.cursor_y + 1 < self.num_lines {
                        self.cursor_y += 1;
                    }
                }
                self.cursor_x = self.desired_x.min(self.max_cursor_x());
                self.ensure_cursor_visible();
                self.pending.clear();
            }
            21 => {
                // Ctrl+U - half page up
                let half = self.text_height() / 2;
                for _ in 0..half {
                    if self.cursor_y > 0 {
                        self.cursor_y -= 1;
                    }
                }
                self.cursor_x = self.desired_x.min(self.max_cursor_x());
                self.ensure_cursor_visible();
                self.pending.clear();
            }
            6 => {
                // Ctrl+F - full page down
                let page = self.text_height();
                for _ in 0..page {
                    if self.cursor_y + 1 < self.num_lines {
                        self.cursor_y += 1;
                    }
                }
                self.cursor_x = self.desired_x.min(self.max_cursor_x());
                self.ensure_cursor_visible();
                self.pending.clear();
            }
            2 => {
                // Ctrl+B - full page up
                let page = self.text_height();
                for _ in 0..page {
                    if self.cursor_y > 0 {
                        self.cursor_y -= 1;
                    }
                }
                self.cursor_x = self.desired_x.min(self.max_cursor_x());
                self.ensure_cursor_visible();
                self.pending.clear();
            }

            // === Text objects (only valid after operator) ===
            // Handled via 'i' and 'a' when operator is pending
            // But 'i' and 'a' are handled above as insert mode entries...
            // Need to check if operator is pending

            // === ESC / Ctrl+C ===
            27 | 3 => {
                self.pending.clear();
                self.status_msg.clear();
            }

            _ => {
                // Unknown key, clear pending
                self.pending.clear();
            }
        }

        self.render();
    }

    // Override 'i' and 'a' when operator is pending (text objects)
    // This is handled by intercepting in the match above
    // Actually we need to fix this - when operator is pending, 'i' should be text object prefix
    // Let me restructure: check operator before matching insert commands

    // === Insert Mode ===

    fn process_insert(&mut self, byte: u8) {
        self.record_dot_byte(byte);

        match byte {
            27 | 3 => {
                // ESC or Ctrl+C - exit insert mode
                self.exit_insert_mode();
            }
            b'\n' | b'\r' => {
                self.split_line(self.cursor_x, self.cursor_y);
                self.cursor_y += 1;
                self.cursor_x = 0;
                self.ensure_cursor_visible();
            }
            8 | 127 => {
                // Backspace
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                    self.delete_char_at(self.cursor_x, self.cursor_y);
                } else if self.cursor_y > 0 {
                    // Join with previous line
                    let prev_len = self.line_lengths[self.cursor_y - 1];
                    let cur_len = self.line_lengths[self.cursor_y];
                    if prev_len + cur_len <= MAX_LINE_LEN {
                        // Append current line to previous (via temp to satisfy borrow checker)
                        let mut temp = [0u8; MAX_LINE_LEN];
                        temp[..cur_len].copy_from_slice(&self.lines[self.cursor_y][..cur_len]);
                        self.lines[self.cursor_y - 1][prev_len..prev_len + cur_len]
                            .copy_from_slice(&temp[..cur_len]);
                        self.line_lengths[self.cursor_y - 1] = prev_len + cur_len;
                        self.delete_line(self.cursor_y);
                        self.cursor_y -= 1;
                        self.cursor_x = prev_len;
                    }
                }
            }
            b'\t' => {
                // Insert spaces for tab
                for _ in 0..TAB_WIDTH {
                    if self.line_lengths[self.cursor_y] < MAX_LINE_LEN - 1 {
                        self.insert_char_at(self.cursor_x, self.cursor_y, b' ');
                        self.cursor_x += 1;
                    }
                }
            }
            _ if byte >= 32 && byte < 127 => {
                // Printable character
                if self.line_lengths[self.cursor_y] < MAX_LINE_LEN - 1 {
                    self.insert_char_at(self.cursor_x, self.cursor_y, byte);
                    self.cursor_x += 1;
                }
            }
            _ => {}
        }

        self.ensure_cursor_visible();
        self.render();
    }

    // === Replace Mode (single char) ===

    fn process_replace(&mut self, byte: u8) {
        if byte == 27 || byte == 3 {
            // ESC or Ctrl+C - cancel
            self.mode = Mode::Normal;
            self.render();
            return;
        }
        if byte >= 32 && byte < 127 {
            self.save_undo();
            if self.cursor_x < self.line_lengths[self.cursor_y] {
                self.lines[self.cursor_y][self.cursor_x] = byte;
                self.modified = true;
            }
        }
        self.mode = Mode::Normal;
        self.render();
    }

    // === Visual Mode ===

    fn process_visual(&mut self, byte: u8) {
        match byte {
            27 | 3 => {
                // ESC or Ctrl+C
                self.exit_visual_mode();
            }
            b'v' => {
                if self.mode == Mode::Visual {
                    self.exit_visual_mode();
                } else {
                    self.mode = Mode::Visual;
                    self.visual_start_x = self.cursor_x;
                    self.visual_start_y = self.cursor_y;
                }
            }
            b'V' => {
                if self.mode == Mode::VisualLine {
                    self.exit_visual_mode();
                } else {
                    self.mode = Mode::VisualLine;
                    self.visual_start_x = self.cursor_x;
                    self.visual_start_y = self.cursor_y;
                }
            }

            // Motions in visual mode
            b'h' => self.move_left(),
            b'l' => self.move_right(),
            b'j' => self.move_down(),
            b'k' => self.move_up(),
            b'w' => {
                let (nx, ny) = self.word_forward(self.cursor_x, self.cursor_y, false);
                self.cursor_x = nx;
                self.cursor_y = ny;
                self.desired_x = nx;
                self.ensure_cursor_visible();
            }
            b'b' => {
                let (nx, ny) = self.word_backward(self.cursor_x, self.cursor_y, false);
                self.cursor_x = nx;
                self.cursor_y = ny;
                self.desired_x = nx;
                self.ensure_cursor_visible();
            }
            b'e' => {
                let (nx, ny) = self.word_end(self.cursor_x, self.cursor_y, false);
                self.cursor_x = nx;
                self.cursor_y = ny;
                self.desired_x = nx;
                self.ensure_cursor_visible();
            }
            b'0' => self.move_to_line_start(),
            b'$' => self.move_to_line_end(),
            b'^' => self.move_to_first_non_blank(),
            b'G' => {
                self.cursor_y = self.num_lines - 1;
                self.cursor_x = 0;
                self.ensure_cursor_visible();
            }
            b'{' => {
                self.cursor_y = self.paragraph_backward(self.cursor_y);
                self.cursor_x = 0;
                self.ensure_cursor_visible();
            }
            b'}' => {
                self.cursor_y = self.paragraph_forward(self.cursor_y);
                self.cursor_x = 0;
                self.ensure_cursor_visible();
            }

            // Operators on selection
            b'd' | b'x' => {
                let (sx, sy, ex, ey) = self.visual_range();
                if self.mode == Mode::VisualLine {
                    self.execute_operator_linewise(Operator::Delete, sy, ey);
                } else {
                    self.execute_operator_charwise(Operator::Delete, sx, sy, ex, ey);
                }
                self.exit_visual_mode();
            }
            b'y' => {
                let (sx, sy, ex, ey) = self.visual_range();
                if self.mode == Mode::VisualLine {
                    self.execute_operator_linewise(Operator::Yank, sy, ey);
                } else {
                    self.execute_operator_charwise(Operator::Yank, sx, sy, ex, ey);
                }
                self.exit_visual_mode();
            }
            b'c' => {
                let (sx, sy, ex, ey) = self.visual_range();
                if self.mode == Mode::VisualLine {
                    self.execute_operator_linewise(Operator::Change, sy, ey);
                } else {
                    self.execute_operator_charwise(Operator::Change, sx, sy, ex, ey);
                }
                // Change exits visual but stays in insert (handled by operator)
                if self.mode != Mode::Insert {
                    self.exit_visual_mode();
                }
            }
            b'>' => {
                let (_sx, sy, _ex, ey) = self.visual_range();
                self.execute_operator_linewise(Operator::Indent, sy, ey);
                self.exit_visual_mode();
            }
            b'<' => {
                let (_sx, sy, _ex, ey) = self.visual_range();
                self.execute_operator_linewise(Operator::Dedent, sy, ey);
                self.exit_visual_mode();
            }
            b'J' => {
                let (_sx, sy, _ex, ey) = self.visual_range();
                self.save_undo();
                self.cursor_y = sy;
                for _ in sy..ey {
                    self.join_line_with_next(self.cursor_y);
                }
                self.exit_visual_mode();
            }
            b'~' => {
                // Toggle case in selection
                let (sx, sy, ex, ey) = self.visual_range();
                self.save_undo();
                for y in sy..=ey {
                    let start = if y == sy { sx } else { 0 };
                    let end = if y == ey { ex.min(self.line_lengths[y]) } else { self.line_lengths[y] };
                    for x in start..end {
                        let ch = self.lines[y][x];
                        if ch.is_ascii_lowercase() {
                            self.lines[y][x] = ch.to_ascii_uppercase();
                        } else if ch.is_ascii_uppercase() {
                            self.lines[y][x] = ch.to_ascii_lowercase();
                        }
                    }
                }
                self.modified = true;
                self.exit_visual_mode();
            }
            b':' => {
                self.enter_command_mode();
            }

            _ => {}
        }
        self.render();
    }

    // === Command Mode ===

    fn process_command_mode(&mut self, byte: u8) {
        match byte {
            27 | 3 => {
                // ESC or Ctrl+C - cancel
                self.mode = Mode::Normal;
                self.status_msg.clear();
            }
            b'\n' | b'\r' => {
                // Execute command
                self.execute_command();
            }
            8 | 127 => {
                // Backspace
                if !self.cmdline.is_empty() {
                    self.cmdline.pop();
                } else {
                    self.mode = Mode::Normal;
                }
            }
            _ if byte >= 32 && byte < 127 => {
                self.cmdline.push(byte);
            }
            _ => {}
        }
        self.render();
    }

    // === Search Mode ===

    fn process_search_mode(&mut self, byte: u8) {
        match byte {
            27 | 3 => {
                // ESC or Ctrl+C - cancel
                self.mode = Mode::Normal;
                self.status_msg.clear();
            }
            b'\n' | b'\r' => {
                // Execute search
                self.mode = Mode::Normal;
                self.execute_search();
            }
            8 | 127 => {
                // Backspace
                if !self.search_query.is_empty() {
                    self.search_query.pop();
                } else {
                    self.mode = Mode::Normal;
                }
            }
            _ if byte >= 32 && byte < 127 => {
                self.search_query.push(byte);
            }
            _ => {}
        }
        self.render();
    }

    // === Rendering ===

    pub fn render(&self) {
        clear();
        let width = terminal::get_width() as usize;
        let height = terminal::get_height() as usize;
        let text_height = if height > 2 { height - 2 } else { 1 };
        let gutter = self.gutter_width();

        for screen_y in 0..text_height {
            let buf_y = self.scroll_offset + screen_y;
            terminal::set_cursor(0, screen_y as i32);

            if buf_y < self.num_lines {
                let mut line_buf = [0u8; 256];
                let mut pos = 0;

                // Line number gutter
                if self.show_line_numbers {
                    let num_str = format!("{:>width$} ", buf_y + 1, width = gutter - 1);
                    let num_bytes = num_str.as_bytes();
                    let copy_len = num_bytes.len().min(line_buf.len());
                    line_buf[..copy_len].copy_from_slice(&num_bytes[..copy_len]);
                    pos = copy_len;
                }

                // Visual selection marker
                if (self.mode == Mode::Visual || self.mode == Mode::VisualLine) && self.is_line_in_visual(buf_y) {
                    // We'll render char by char for highlighting
                    let line = &self.lines[buf_y][..self.line_lengths[buf_y]];
                    let avail = width - pos;
                    for (i, &ch) in line.iter().enumerate() {
                        if i >= avail { break; }
                        if self.is_in_visual_selection(i, buf_y) {
                            // Invert case as a visual indicator (crude but effective without colors)
                            if ch.is_ascii_lowercase() {
                                line_buf[pos] = ch.to_ascii_uppercase();
                            } else if ch.is_ascii_uppercase() {
                                line_buf[pos] = ch.to_ascii_lowercase();
                            } else if ch == b' ' {
                                line_buf[pos] = b'_'; // show selected spaces
                            } else {
                                line_buf[pos] = ch;
                            }
                        } else {
                            line_buf[pos] = ch;
                        }
                        pos += 1;
                    }
                    // If visual selection extends past line content (VisualLine mode)
                    if self.mode == Mode::VisualLine && self.is_line_in_visual(buf_y) {
                        // Show selection indicator for empty space
                        while pos < width {
                            line_buf[pos] = b' ';
                            pos += 1;
                        }
                    }
                } else {
                    // Normal line rendering
                    let line = &self.lines[buf_y][..self.line_lengths[buf_y]];
                    let avail = width - pos;
                    let copy_len = line.len().min(avail);
                    line_buf[pos..pos + copy_len].copy_from_slice(&line[..copy_len]);
                    pos += copy_len;
                }

                if pos > 0 {
                    if let Ok(s) = std::str::from_utf8(&line_buf[..pos]) {
                        print(s);
                    }
                }
            } else {
                // Past end of file
                print("~");
            }
        }

        // Status line (row height-2)
        terminal::set_cursor(0, (height - 2) as i32);
        self.render_status_line(width);

        // Command/message line (row height-1)
        terminal::set_cursor(0, (height - 1) as i32);
        self.render_command_line(width);

        // Position cursor
        let screen_x = self.cursor_x + gutter;
        let screen_y = if self.cursor_y >= self.scroll_offset {
            self.cursor_y - self.scroll_offset
        } else {
            0
        };

        // In command/search mode, cursor is on the command line
        if self.mode == Mode::Command {
            terminal::set_cursor((1 + self.cmdline.len()) as i32, (height - 1) as i32);
        } else if self.mode == Mode::Search {
            terminal::set_cursor((1 + self.search_query.len()) as i32, (height - 1) as i32);
        } else {
            terminal::set_cursor(screen_x as i32, screen_y as i32);
        }
    }

    fn is_line_in_visual(&self, y: usize) -> bool {
        let (_, sy, _, ey) = self.visual_range();
        y >= sy && y <= ey
    }

    fn render_status_line(&self, width: usize) {
        let mut buf = Vec::with_capacity(width);

        // Left side: mode indicator
        let mode_str = match self.mode {
            Mode::Insert => "-- INSERT --",
            Mode::Visual => "-- VISUAL --",
            Mode::VisualLine => "-- VISUAL LINE --",
            Mode::Replace => "-- REPLACE --",
            _ => "",
        };
        buf.extend_from_slice(mode_str.as_bytes());

        // Right side: filename [+] line,col percentage
        let filename = self.get_filename();
        let modified_marker = if self.modified { "[+]" } else { "" };
        let percentage = if self.num_lines <= 1 {
            "All".to_string()
        } else if self.cursor_y == 0 {
            "Top".to_string()
        } else if self.cursor_y >= self.num_lines - 1 {
            "Bot".to_string()
        } else {
            format!("{}%", self.cursor_y * 100 / (self.num_lines - 1))
        };

        let right = format!("{} {} {},{} {}",
            filename, modified_marker,
            self.cursor_y + 1, self.cursor_x + 1, percentage
        );

        let right_bytes = right.as_bytes();
        let left_len = buf.len();
        let right_len = right_bytes.len();

        if left_len + right_len < width {
            // Pad with spaces
            let padding = width - left_len - right_len;
            for _ in 0..padding {
                buf.push(b' ');
            }
            buf.extend_from_slice(right_bytes);
        } else {
            // Truncate - just show what fits
            while buf.len() < width {
                buf.push(b' ');
            }
        }

        // Truncate to width
        buf.truncate(width);
        if let Ok(s) = std::str::from_utf8(&buf) {
            print(s);
        }
    }

    fn render_command_line(&self, width: usize) {
        match self.mode {
            Mode::Command => {
                let mut buf = Vec::with_capacity(width);
                buf.push(b':');
                buf.extend_from_slice(&self.cmdline);
                buf.truncate(width);
                if let Ok(s) = std::str::from_utf8(&buf) {
                    print(s);
                }
            }
            Mode::Search => {
                let mut buf = Vec::with_capacity(width);
                if self.search_forward {
                    buf.push(b'/');
                } else {
                    buf.push(b'?');
                }
                buf.extend_from_slice(&self.search_query);
                buf.truncate(width);
                if let Ok(s) = std::str::from_utf8(&buf) {
                    print(s);
                }
            }
            _ => {
                // Show status message if any
                if !self.status_msg.is_empty() {
                    let msg = &self.status_msg[..self.status_msg.len().min(width)];
                    if let Ok(s) = std::str::from_utf8(msg) {
                        print(s);
                    }
                }
            }
        }
    }

    pub fn should_exit(&self) -> bool {
        self.exit_requested
    }
}

enum SpecialKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Delete,
}
