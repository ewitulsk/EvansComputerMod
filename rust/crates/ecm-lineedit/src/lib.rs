//! `ecm-lineedit`: the line editor of the shell prompt (the Terminal block's
//! console, the GUI Terminal app and SSH sessions) and of the Python and Lua
//! REPLs. Readline-style keys:
//!
//! | Keys | Action |
//! |---|---|
//! | Left / Right, Ctrl+B / Ctrl+F | move one character |
//! | Ctrl+Left / Ctrl+Right, Alt+B / Alt+F | move one word |
//! | Home / End, Ctrl+A / Ctrl+E | start / end of the line |
//! | Backspace, Delete, Ctrl+D | delete before / at the cursor (Ctrl+D on an empty line: end of input) |
//! | Ctrl+W, Alt+Backspace / Alt+D | delete the word before / after the cursor |
//! | Ctrl+U / Ctrl+K | delete to the start / end of the line |
//! | Ctrl+Y | paste back what was deleted last |
//! | Up / Down, Ctrl+P / Ctrl+N | history |
//! | Ctrl+R | search the history backwards (again: older match; Esc / Ctrl+G cancels) |
//! | Tab | complete (a second Tab lists the choices) |
//! | Ctrl+L | clear the screen, keep the line |
//! | Ctrl+C | abandon the line |
//! | Ctrl+T | the mod's "terminate" key (reported to the caller) |
//!
//! Text pasted inside bracketed-paste markers (`ESC[200~` .. `ESC[201~`) is
//! inserted as typed text: a Tab in it never completes, control keys are
//! dropped, and each newline enters the line so far.
//!
//! Bytes arrive in any chunks (single keys, pastes); escape sequences keep
//! their parse state between calls. Output goes to an [`Out`]. The editor
//! redraws the prompt and the line from the prompt's first column, so the
//! prompt must start at column 0, and a line longer than the terminal wraps
//! over several rows.

use std::fmt::Write as _;

/// Longest line kept.
pub const MAX_LINE: usize = 4096;
/// History entries kept.
pub const HISTORY_LIMIT: usize = 500;

/// Where the editor draws.
pub trait Out {
    /// Raw bytes (ANSI escapes allowed).
    fn write(&mut self, data: &[u8]);
    /// Terminal width in columns.
    fn width(&self) -> u16;
}

/// Something that can complete the word before the cursor.
pub trait Completer {
    /// Candidates for `line[start..cursor]`, as `(start, words)`: each word
    /// replaces that range. A word ending in `/` is a directory (no space is
    /// added after it).
    fn complete(&mut self, line: &str, cursor: usize) -> (usize, Vec<String>);
}

/// No completion.
pub struct NoCompletion;

impl Completer for NoCompletion {
    fn complete(&mut self, _line: &str, cursor: usize) -> (usize, Vec<String>) {
        (cursor, Vec::new())
    }
}

/// What a key did that the caller must act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Enter: a complete line (history expansion already applied).
    Line(String),
    /// Ctrl+C: the line was abandoned (`^C` and a newline were printed).
    Interrupt,
    /// Ctrl+T (the line was cleared).
    Terminate,
    /// Ctrl+D on an empty line.
    Eof,
}

#[derive(Clone, PartialEq, Eq)]
enum Esc {
    None,
    /// After ESC.
    Esc,
    /// After `ESC [`, collecting parameters.
    Csi(String),
    /// After `ESC O`.
    Ss3,
}

struct Search {
    query: String,
    /// History index of the match shown (None: no match).
    hit: Option<usize>,
    /// The line and cursor before the search started (Esc restores them).
    saved: (Vec<u8>, usize),
    failed: bool,
}

pub struct Editor {
    buf: Vec<u8>,
    pos: usize,
    prompt: String,
    prompt_w: usize,
    /// Where the terminal's cursor is, counted in cells from the prompt's start.
    shown_at: usize,
    esc: Esc,
    history: Vec<String>,
    /// Index into `history` while browsing it.
    hist_idx: Option<usize>,
    draft: Vec<u8>,
    kill: Vec<u8>,
    search: Option<Search>,
    paste: bool,
    last_cr: bool,
    last_tab: bool,
    /// Lines entered since `take_new_history` (for saving the history).
    added: usize,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

/// Columns a prompt takes (ANSI escape sequences take none).
pub fn visible_width(s: &str) -> usize {
    let mut n = 0;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        if !c.is_control() {
            n += 1;
        }
    }
    n
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl Editor {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            pos: 0,
            prompt: String::new(),
            prompt_w: 0,
            shown_at: 0,
            esc: Esc::None,
            history: Vec::new(),
            hist_idx: None,
            draft: Vec::new(),
            kill: Vec::new(),
            search: None,
            paste: false,
            last_cr: false,
            last_tab: false,
            added: 0,
        }
    }

    // ------------------------------------------------------------ public API

    /// Print `prompt` (at column 0) and start a new, empty line.
    pub fn start(&mut self, prompt: &str, out: &mut dyn Out) {
        self.buf.clear();
        self.pos = 0;
        self.hist_idx = None;
        self.draft.clear();
        self.search = None;
        self.set_prompt(prompt);
        self.shown_at = 0;
        self.redraw(out);
    }

    /// Change the prompt without touching the screen.
    pub fn set_prompt(&mut self, prompt: &str) {
        self.prompt = prompt.to_string();
        self.prompt_w = visible_width(prompt);
    }

    /// Draw the prompt and the line again on a fresh row (after other output,
    /// or a resize). The cursor must be at column 0.
    pub fn reprint(&mut self, out: &mut dyn Out) {
        self.shown_at = 0;
        self.redraw(out);
    }

    /// The line typed so far.
    pub fn pending(&self) -> String {
        String::from_utf8_lossy(&self.buf).into_owned()
    }

    /// The cursor's index in the line.
    pub fn cursor(&self) -> usize {
        self.pos
    }

    /// Forget the line (and any half-read escape sequence) without drawing.
    pub fn clear(&mut self) {
        self.buf.clear();
        self.pos = 0;
        self.esc = Esc::None;
        self.hist_idx = None;
        self.draft.clear();
        self.search = None;
        self.paste = false;
        self.shown_at = 0;
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    /// Replace the history (loaded from a file).
    pub fn set_history(&mut self, lines: Vec<String>) {
        self.history = lines.into_iter().filter(|l| !l.trim().is_empty()).collect();
        let n = self.history.len();
        if n > HISTORY_LIMIT {
            self.history.drain(..n - HISTORY_LIMIT);
        }
        self.added = 0;
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
        self.added = 0;
    }

    /// Add a line to the history (also done for every line entered).
    pub fn add_history(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() || self.history.last().is_some_and(|l| l == line) {
            return;
        }
        self.history.push(line.to_string());
        if self.history.len() > HISTORY_LIMIT {
            self.history.remove(0);
        }
        self.added += 1;
    }

    /// True once per batch of new history lines (time to save the file).
    pub fn take_new_history(&mut self) -> bool {
        let any = self.added > 0;
        self.added = 0;
        any
    }

    /// Feed bytes; returns the events they produced, in order.
    pub fn feed(&mut self, data: &[u8], out: &mut dyn Out, comp: &mut dyn Completer) -> Vec<Event> {
        let mut events = Vec::new();
        for &b in data {
            if let Some(e) = self.feed_byte(b, out, comp) {
                events.push(e);
            }
        }
        events
    }

    /// Feed one byte.
    pub fn feed_byte(&mut self, b: u8, out: &mut dyn Out, comp: &mut dyn Completer) -> Option<Event> {
        let was_cr = std::mem::replace(&mut self.last_cr, false);
        let was_tab = std::mem::replace(&mut self.last_tab, false);
        match std::mem::replace(&mut self.esc, Esc::None) {
            Esc::Esc => return self.on_escape(b, out),
            Esc::Csi(mut p) => {
                if (0x20..=0x3f).contains(&b) {
                    if p.len() < 16 {
                        p.push(b as char);
                    }
                    self.esc = Esc::Csi(p);
                    return None;
                }
                return self.on_csi(&p, b, out);
            }
            Esc::Ss3 => {
                return self.on_csi("", b, out);
            }
            Esc::None => {}
        }
        if b == b'\n' && was_cr {
            return None; // CR LF is one Enter
        }
        if self.paste {
            return match b {
                0x1b => {
                    self.esc = Esc::Esc;
                    None
                }
                b'\r' | b'\n' => {
                    self.last_cr = b == b'\r';
                    self.enter(out)
                }
                b'\t' => {
                    self.insert(b" ", out);
                    None
                }
                32..=126 => {
                    self.insert(&[b], out);
                    None
                }
                _ => None,
            };
        }
        if self.search.is_some() {
            if let Some(r) = self.search_key(b, out) {
                return r;
            }
        }
        match b {
            0x1b => self.esc = Esc::Esc,
            b'\r' | b'\n' => {
                self.last_cr = b == b'\r';
                return self.enter(out);
            }
            0x01 => self.move_to(0, out),
            0x02 => self.move_to(self.pos.saturating_sub(1), out),
            0x03 => {
                self.pos = self.buf.len();
                self.redraw(out);
                out.write(b"^C\r\n");
                self.clear();
                return Some(Event::Interrupt);
            }
            0x04 => {
                if self.buf.is_empty() {
                    return Some(Event::Eof);
                }
                self.delete_range(self.pos, (self.pos + 1).min(self.buf.len()), false, out);
            }
            0x05 => self.move_to(self.buf.len(), out),
            0x06 => self.move_to((self.pos + 1).min(self.buf.len()), out),
            0x07 => {}
            0x08 | 0x7f => {
                if self.pos > 0 {
                    self.delete_range(self.pos - 1, self.pos, false, out);
                }
            }
            b'\t' => self.tab(was_tab, out, comp),
            0x0b => self.delete_range(self.pos, self.buf.len(), true, out),
            0x0c => {
                out.write(b"\x1b[2J\x1b[H");
                self.reprint(out);
            }
            0x0e => self.history_next(out),
            0x10 => self.history_prev(out),
            0x12 => self.search_start(out),
            0x14 => {
                self.clear();
                return Some(Event::Terminate);
            }
            0x15 => self.delete_range(0, self.pos, true, out),
            0x17 => {
                let mut a = self.pos;
                while a > 0 && self.buf[a - 1] == b' ' {
                    a -= 1;
                }
                while a > 0 && self.buf[a - 1] != b' ' {
                    a -= 1;
                }
                self.delete_range(a, self.pos, true, out);
            }
            0x19 => {
                let k = self.kill.clone();
                self.insert(&k, out);
            }
            32..=126 => self.insert(&[b], out),
            _ => {}
        }
        None
    }

    // ------------------------------------------------------------ escape sequences

    fn on_escape(&mut self, b: u8, out: &mut dyn Out) -> Option<Event> {
        match b {
            b'[' => self.esc = Esc::Csi(String::new()),
            b'O' => self.esc = Esc::Ss3,
            _ if self.search.is_some() => {
                // Esc ends a search, putting the line back; the next key is a key.
                self.search_cancel(out);
                return self.feed_byte(b, out, &mut NoCompletion);
            }
            b'b' | b'B' => self.move_to(self.word_left(), out),
            b'f' | b'F' => self.move_to(self.word_right(), out),
            b'd' | b'D' => {
                let e = self.word_right();
                self.delete_range(self.pos, e, true, out);
            }
            0x7f | 0x08 => {
                let a = self.word_left();
                self.delete_range(a, self.pos, true, out);
            }
            0x1b => self.esc = Esc::Esc,
            _ => {}
        }
        None
    }

    fn on_csi(&mut self, params: &str, fin: u8, out: &mut dyn Out) -> Option<Event> {
        match (params, fin) {
            ("200", b'~') => {
                self.paste = true;
                return None;
            }
            ("201", b'~') => {
                self.paste = false;
                return None;
            }
            _ => {}
        }
        if self.paste {
            return None;
        }
        if self.search.is_some() {
            // Any movement key leaves the search with the match to edit.
            self.search_accept(out);
        }
        let mods = params.split(';').nth(1).and_then(|m| m.parse::<u8>().ok()).unwrap_or(1);
        let word = mods == 3 || mods == 5 || mods == 7;
        match fin {
            b'A' => self.history_prev(out),
            b'B' => self.history_next(out),
            b'C' if word => self.move_to(self.word_right(), out),
            b'D' if word => self.move_to(self.word_left(), out),
            b'C' => self.move_to((self.pos + 1).min(self.buf.len()), out),
            b'D' => self.move_to(self.pos.saturating_sub(1), out),
            b'H' => self.move_to(0, out),
            b'F' => self.move_to(self.buf.len(), out),
            b'~' => match params.split(';').next().unwrap_or("") {
                "1" | "7" => self.move_to(0, out),
                "4" | "8" => self.move_to(self.buf.len(), out),
                "3" if word => {
                    let e = self.word_right();
                    self.delete_range(self.pos, e, true, out);
                }
                "3" => {
                    if self.pos < self.buf.len() {
                        self.delete_range(self.pos, self.pos + 1, false, out);
                    }
                }
                _ => {}
            },
            _ => {}
        }
        None
    }

    // ------------------------------------------------------------ editing

    fn enter(&mut self, out: &mut dyn Out) -> Option<Event> {
        if let Some(s) = self.search.take() {
            if let Some(i) = s.hit {
                self.buf = self.history[i].as_bytes().to_vec();
            }
            self.pos = self.buf.len();
        }
        self.pos = self.buf.len();
        self.redraw(out);
        out.write(b"\r\n");
        let line = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        self.pos = 0;
        self.shown_at = 0;
        self.hist_idx = None;
        self.draft.clear();
        let line = match expand_history(&line, &self.history) {
            Ok(Some(expanded)) => {
                out.write(expanded.as_bytes());
                out.write(b"\r\n");
                expanded
            }
            Ok(None) => line,
            Err(e) => {
                out.write(e.as_bytes());
                out.write(b"\r\n");
                String::new()
            }
        };
        self.add_history(&line);
        Some(Event::Line(line))
    }

    fn insert(&mut self, text: &[u8], out: &mut dyn Out) {
        let room = MAX_LINE.saturating_sub(self.buf.len());
        let text = &text[..text.len().min(room)];
        if text.is_empty() {
            return;
        }
        self.hist_idx = None;
        self.buf.splice(self.pos..self.pos, text.iter().copied());
        self.pos += text.len();
        if self.pos == self.buf.len() && self.search.is_none() {
            // Typing at the end: just echo it.
            out.write(text);
            self.shown_at = self.prompt_w + self.pos;
            self.settle(out);
        } else {
            self.redraw(out);
        }
    }

    fn delete_range(&mut self, a: usize, b: usize, to_kill: bool, out: &mut dyn Out) {
        if a >= b || b > self.buf.len() {
            return;
        }
        let gone: Vec<u8> = self.buf.drain(a..b).collect();
        if to_kill {
            self.kill = gone;
        }
        self.pos = a;
        self.hist_idx = None;
        self.redraw(out);
    }

    fn word_left(&self) -> usize {
        let mut i = self.pos;
        while i > 0 && !is_word(self.buf[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(self.buf[i - 1]) {
            i -= 1;
        }
        i
    }

    fn word_right(&self) -> usize {
        let n = self.buf.len();
        let mut i = self.pos;
        while i < n && !is_word(self.buf[i]) {
            i += 1;
        }
        while i < n && is_word(self.buf[i]) {
            i += 1;
        }
        i
    }

    fn move_to(&mut self, pos: usize, out: &mut dyn Out) {
        self.pos = pos.min(self.buf.len());
        let target = self.prompt_w + self.pos;
        self.cursor_to(target, out);
    }

    // ------------------------------------------------------------ history

    fn set_line(&mut self, line: Vec<u8>, out: &mut dyn Out) {
        self.buf = line;
        self.buf.truncate(MAX_LINE);
        self.pos = self.buf.len();
        self.redraw(out);
    }

    fn history_prev(&mut self, out: &mut dyn Out) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.hist_idx {
            None => {
                self.draft = self.buf.clone();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.hist_idx = Some(idx);
        let entry = self.history[idx].as_bytes().to_vec();
        self.set_line(entry, out);
        self.hist_idx = Some(idx);
    }

    fn history_next(&mut self, out: &mut dyn Out) {
        let Some(i) = self.hist_idx else { return };
        if i + 1 < self.history.len() {
            let entry = self.history[i + 1].as_bytes().to_vec();
            self.set_line(entry, out);
            self.hist_idx = Some(i + 1);
        } else {
            let draft = std::mem::take(&mut self.draft);
            self.set_line(draft, out);
            self.hist_idx = None;
        }
    }

    // ------------------------------------------------------------ reverse search

    fn search_start(&mut self, out: &mut dyn Out) {
        if let Some(s) = &self.search {
            // Ctrl+R again: the next older match.
            let from = s.hit.unwrap_or(self.history.len());
            let q = s.query.clone();
            self.search_find(&q, from, out);
            return;
        }
        self.search = Some(Search { query: String::new(), hit: None, saved: (self.buf.clone(), self.pos), failed: false });
        self.redraw(out);
    }

    /// Find `q` in history entries older than `before`.
    fn search_find(&mut self, q: &str, before: usize, out: &mut dyn Out) {
        let hit = (0..before.min(self.history.len())).rev().find(|&i| self.history[i].contains(q));
        if let Some(s) = &mut self.search {
            s.query = q.to_string();
            match hit {
                Some(i) => {
                    s.hit = Some(i);
                    s.failed = false;
                }
                None => s.failed = !q.is_empty(),
            }
        }
        if let Some(i) = hit {
            self.buf = self.history[i].as_bytes().to_vec();
            self.pos = self.history[i].find(q).unwrap_or(0);
        }
        self.redraw(out);
    }

    /// Keys while searching; None: not handled here (falls through).
    fn search_key(&mut self, b: u8, out: &mut dyn Out) -> Option<Option<Event>> {
        let q = self.search.as_ref()?.query.clone();
        match b {
            0x12 => {
                self.search_start(out);
                Some(None)
            }
            0x07 => {
                self.search_cancel(out);
                Some(None)
            }
            0x08 | 0x7f => {
                let mut q = q;
                q.pop();
                self.search_find(&q, self.history.len(), out);
                Some(None)
            }
            32..=126 => {
                let mut nq = q;
                nq.push(b as char);
                let from = self.search.as_ref().and_then(|s| s.hit).map(|i| i + 1).unwrap_or(self.history.len());
                self.search_find(&nq, from, out);
                Some(None)
            }
            0x1b | b'\r' | b'\n' => None,
            _ => {
                // Another control key: keep the match and handle the key on it.
                self.search_accept(out);
                None
            }
        }
    }

    fn search_accept(&mut self, out: &mut dyn Out) {
        if self.search.take().is_some() {
            self.redraw(out);
        }
    }

    fn search_cancel(&mut self, out: &mut dyn Out) {
        if let Some(s) = self.search.take() {
            self.buf = s.saved.0;
            self.pos = s.saved.1;
            self.redraw(out);
        }
    }

    // ------------------------------------------------------------ completion

    fn tab(&mut self, again: bool, out: &mut dyn Out, comp: &mut dyn Completer) {
        let line = String::from_utf8_lossy(&self.buf).into_owned();
        let (start, mut cands) = comp.complete(&line, self.pos);
        if start > self.pos {
            return;
        }
        cands.sort();
        cands.dedup();
        if cands.is_empty() {
            out.write(b"\x07");
            return;
        }
        let typed = &line[start..self.pos];
        if cands.len() == 1 {
            let mut w = cands[0].clone();
            if !w.ends_with('/') {
                w.push(' ');
            }
            self.replace_word(start, &w, out);
            return;
        }
        let common = common_prefix(&cands);
        if common.len() > typed.len() {
            self.replace_word(start, &common, out);
            return;
        }
        self.last_tab = true;
        if !again {
            out.write(b"\x07");
            return;
        }
        // Second Tab: list the choices below the line, then draw it again.
        self.move_to(self.buf.len(), out);
        out.write(b"\r\n");
        let names: Vec<&str> = cands.iter().map(|c| c.rsplit_once('/').map(|(_, n)| if n.is_empty() { c.as_str() } else { n }).unwrap_or(c)).collect();
        let names: Vec<String> = cands
            .iter()
            .zip(names)
            .map(|(c, n)| if c.ends_with('/') && !n.ends_with('/') { format!("{n}/") } else { n.to_string() })
            .collect();
        out.write(columns(&names, out.width() as usize).as_bytes());
        self.reprint(out);
    }

    fn replace_word(&mut self, start: usize, word: &str, out: &mut dyn Out) {
        let room = MAX_LINE.saturating_sub(self.buf.len() - (self.pos - start));
        let w = &word.as_bytes()[..word.len().min(room)];
        self.buf.splice(start..self.pos, w.iter().copied());
        self.pos = start + w.len();
        self.redraw(out);
    }

    // ------------------------------------------------------------ drawing

    fn width(out: &dyn Out) -> usize {
        (out.width() as usize).max(1)
    }

    /// Move the terminal's cursor to cell `target` (counted from the prompt's start).
    fn cursor_to(&mut self, target: usize, out: &mut dyn Out) {
        let w = Self::width(out);
        let (r0, c0) = (self.shown_at / w, self.shown_at % w);
        let (r1, c1) = (target / w, target % w);
        let mut s = String::new();
        if r1 < r0 {
            let _ = write!(s, "\x1b[{}A", r0 - r1);
        } else if r1 > r0 {
            let _ = write!(s, "\x1b[{}B", r1 - r0);
        }
        if c1 != c0 || r1 != r0 {
            s.push('\r');
            if c1 > 0 {
                let _ = write!(s, "\x1b[{}C", c1);
            }
        }
        out.write(s.as_bytes());
        self.shown_at = target;
    }

    /// After writing up to a row's last column the terminal's cursor waits at
    /// the edge; put it on the next row so the bookkeeping holds.
    fn settle(&mut self, out: &mut dyn Out) {
        let w = Self::width(out);
        if self.shown_at > 0 && self.shown_at % w == 0 {
            out.write(b" \r");
        }
    }

    /// Draw the prompt and the line from the prompt's start, clear what is
    /// left of an older, longer line, and put the cursor at `pos`.
    fn redraw(&mut self, out: &mut dyn Out) {
        self.cursor_to(0, out);
        let (prompt, prompt_w) = match &self.search {
            Some(s) => {
                let p = format!("({}reverse-i-search)`{}': ", if s.failed { "failed " } else { "" }, s.query);
                let w = p.len();
                (p, w)
            }
            None => (self.prompt.clone(), self.prompt_w),
        };
        let mut s = Vec::with_capacity(prompt.len() + self.buf.len() + 8);
        s.push(b'\r');
        s.extend_from_slice(prompt.as_bytes());
        s.extend_from_slice(&self.buf);
        out.write(&s);
        self.shown_at = prompt_w + self.buf.len();
        self.settle(out);
        out.write(b"\x1b[J");
        // The cursor's cell uses the real prompt width while not searching.
        let saved_w = self.prompt_w;
        self.prompt_w = prompt_w;
        let target = self.prompt_w + self.pos;
        self.cursor_to(target, out);
        self.prompt_w = saved_w;
    }
}

/// The longest common prefix of `words`.
pub fn common_prefix(words: &[String]) -> String {
    let Some(first) = words.first() else { return String::new() };
    let mut n = first.len();
    for w in &words[1..] {
        n = n.min(first.bytes().zip(w.bytes()).take_while(|(a, b)| a == b).count());
    }
    first[..n].to_string()
}

/// `names` in columns that fit `width`, one row per line (CRLF).
pub fn columns(names: &[String], width: usize) -> String {
    let col = names.iter().map(|n| n.len()).max().unwrap_or(0) + 2;
    let per_row = (width.max(1) / col.max(1)).max(1);
    let rows = names.len().div_ceil(per_row);
    let mut s = String::new();
    for r in 0..rows {
        let mut line = String::new();
        for c in 0..per_row {
            // Down the columns, like ls.
            let Some(n) = names.get(c * rows + r) else { continue };
            let _ = write!(line, "{:<w$}", n, w = col);
        }
        s.push_str(line.trim_end());
        s.push_str("\r\n");
    }
    s
}

/// Expand `!!`, `!N`, `!-N` and `!prefix` from `history` (bash's event
/// designators). Ok(None): nothing to expand; Err: no such event.
pub fn expand_history(line: &str, history: &[String]) -> Result<Option<String>, String> {
    if !line.contains('!') {
        return Ok(None);
    }
    let b = line.as_bytes();
    let mut out = String::new();
    let mut changed = false;
    let mut i = 0;
    let mut quote: Option<u8> = None;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            out.push(c as char);
            i += 1;
            continue;
        }
        if c == b'\'' {
            quote = Some(c);
            out.push('\'');
            i += 1;
            continue;
        }
        if c != b'!' || i + 1 >= b.len() {
            out.push(c as char);
            i += 1;
            continue;
        }
        let rest = &line[i + 1..];
        let (entry, used) = if rest.starts_with('!') {
            (history.last().cloned().ok_or("!!: event not found")?, 1)
        } else if let Some(num) = rest.strip_prefix('-').map(|r| r.bytes().take_while(u8::is_ascii_digit).count()).filter(|&n| n > 0) {
            let n: usize = rest[1..1 + num].parse().unwrap_or(0);
            let e = history.len().checked_sub(n).and_then(|k| history.get(k)).cloned();
            (e.ok_or_else(|| format!("!-{n}: event not found"))?, 1 + num)
        } else if rest.as_bytes()[0].is_ascii_digit() {
            let num = rest.bytes().take_while(u8::is_ascii_digit).count();
            let n: usize = rest[..num].parse().unwrap_or(0);
            let e = n.checked_sub(1).and_then(|k| history.get(k)).cloned();
            (e.ok_or_else(|| format!("!{n}: event not found"))?, num)
        } else if rest.as_bytes()[0].is_ascii_alphabetic() {
            let len = rest.bytes().take_while(|c| !c.is_ascii_whitespace()).count();
            let pre = &rest[..len];
            let e = history.iter().rev().find(|h| h.starts_with(pre)).cloned();
            (e.ok_or_else(|| format!("!{pre}: event not found"))?, len)
        } else {
            out.push('!');
            i += 1;
            continue;
        };
        out.push_str(&entry);
        changed = true;
        i += 1 + used;
    }
    Ok(if changed { Some(out) } else { None })
}

#[cfg(test)]
mod tests;
