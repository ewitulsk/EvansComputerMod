//! Line editor shared by the shell prompt and the switch CLI.
//!
//! Bytes arrive in arbitrary chunks (single keystrokes, pastes). Escape
//! sequences are parsed with persistent state, so an arrow key split across
//! two `on_input` calls still works. Echo goes to the console.

use crate::console::Console;

const MAX_LINE: usize = 1024;
const HISTORY_LIMIT: usize = 100;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Esc {
    None,
    Esc,
    Csi,
}

pub struct LineEditor {
    buf: String,
    esc: Esc,
    history: Vec<String>,
    cursor: Option<usize>,
    draft: String,
}

pub enum Key {
    /// A complete line was entered (Enter pressed).
    Line(String),
    /// Ctrl+T.
    Terminate,
}

impl LineEditor {
    pub fn new() -> Self {
        Self { buf: String::new(), esc: Esc::None, history: Vec::new(), cursor: None, draft: String::new() }
    }

    /// The line typed so far (for redrawing after asynchronous output).
    pub fn pending(&self) -> &str {
        &self.buf
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.esc = Esc::None;
        self.cursor = None;
        self.draft.clear();
    }

    /// Feed one byte. Returns a completed line or Ctrl+T.
    pub fn feed(&mut self, b: u8, con: &mut Console) -> Option<Key> {
        match self.esc {
            Esc::Esc => {
                self.esc = if b == b'[' { Esc::Csi } else { Esc::None };
                return None;
            }
            Esc::Csi => {
                // Parameters/intermediates keep the sequence open.
                if (0x20..=0x3f).contains(&b) {
                    return None;
                }
                self.esc = Esc::None;
                match b {
                    b'A' => self.history_prev(con),
                    b'B' => self.history_next(con),
                    _ => {}
                }
                return None;
            }
            Esc::None => {}
        }
        match b {
            0x1b => self.esc = Esc::Esc,
            0x14 => {
                self.clear();
                return Some(Key::Terminate);
            }
            b'\r' | b'\n' => {
                con.println("");
                let line = core::mem::take(&mut self.buf);
                self.cursor = None;
                self.draft.clear();
                self.push_history(&line);
                return Some(Key::Line(line));
            }
            8 | 127 => {
                if self.buf.pop().is_some() {
                    con.print("\x08 \x08");
                }
                self.cursor = None;
            }
            32..=126 => {
                if self.buf.len() < MAX_LINE {
                    self.buf.push(b as char);
                    let s = [b];
                    con.write(&s);
                }
                self.cursor = None;
            }
            _ => {}
        }
        None
    }

    fn push_history(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() || self.history.last().map(|l| l == line).unwrap_or(false) {
            return;
        }
        self.history.push(line.to_string());
        if self.history.len() > HISTORY_LIMIT {
            self.history.remove(0);
        }
    }

    fn replace(&mut self, new: &str, con: &mut Console) {
        for _ in 0..self.buf.len() {
            con.print("\x08 \x08");
        }
        self.buf.clear();
        self.buf.push_str(&new[..new.len().min(MAX_LINE)]);
        con.print(&self.buf.clone());
    }

    fn history_prev(&mut self, con: &mut Console) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.cursor {
            None => {
                self.draft = self.buf.clone();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.cursor = Some(idx);
        let entry = self.history[idx].clone();
        self.replace(&entry, con);
    }

    fn history_next(&mut self, con: &mut Console) {
        let Some(i) = self.cursor else { return };
        if i + 1 < self.history.len() {
            self.cursor = Some(i + 1);
            let entry = self.history[i + 1].clone();
            self.replace(&entry, con);
        } else {
            self.cursor = None;
            let draft = core::mem::take(&mut self.draft);
            self.replace(&draft, con);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(ed: &mut LineEditor, con: &mut Console, s: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        for &b in s {
            if let Some(Key::Line(l)) = ed.feed(b, con) {
                out.push(l);
            }
        }
        out
    }

    #[test]
    fn lines_backspace_and_split_escape_history() {
        let _g = crate::hal::test_lock();
        let mut con = Console::new();
        let mut ed = LineEditor::new();
        assert_eq!(feed_all(&mut ed, &mut con, b"lx\x08s\n"), vec!["ls"]);
        assert_eq!(feed_all(&mut ed, &mut con, b"pwd\r"), vec!["pwd"]);
        // Up arrow split across chunks recalls "pwd", second Up recalls "ls".
        feed_all(&mut ed, &mut con, b"\x1b");
        feed_all(&mut ed, &mut con, b"[A");
        feed_all(&mut ed, &mut con, b"\x1b[A");
        assert_eq!(feed_all(&mut ed, &mut con, b"\n"), vec!["ls"]);
        // Ctrl+T clears the buffer.
        feed_all(&mut ed, &mut con, b"abc");
        assert!(matches!(ed.feed(0x14, &mut con), Some(Key::Terminate)));
        assert_eq!(feed_all(&mut ed, &mut con, b"\n"), vec![""]);
    }
}
