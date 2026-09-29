//! Framebuffer text capture with scrollback.
//!
//! The kernel renders into its text framebuffer (header + 4-byte cells,
//! see `framebuffer.rs` in the kernel). After every kernel call the host
//! snapshots it. Lines that scroll off the top are detected by diffing
//! consecutive snapshots and appended to a scrollback, so scenario
//! `expect`s can match output that has already left the screen. Every line
//! has an absolute index (scrollback + screen), which is how `expect`
//! restricts matching to text produced after a command's echo.

use std::collections::VecDeque;

pub const FB_MAGIC: u16 = 0xFB01;
pub const FB_HEADER: usize = 64;
const SCROLLBACK_CAP: usize = 20_000;

pub struct Screen {
    pub w: usize,
    pub h: usize,
    pub rows: Vec<String>,
    /// Raw cells (char, attr, flags) for rendering.
    pub cells: Vec<[u8; 3]>,
    pub cursor: (usize, usize),
    pub cursor_visible: bool,
    pub scrollback: VecDeque<String>,
    /// Absolute line index of `rows[0]`.
    pub base: u64,
    /// Absolute index of the first line still held in `scrollback`.
    pub first_abs: u64,
    last_dirty: Option<u32>,
    /// Times a screenful or more scrolled by between two snapshots.
    pub lost_events: u64,
    pub version: u64,
}

fn u16le(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn cell_char(c: u8) -> char {
    match c {
        0x20..=0x7e => c as char,
        _ => ' ',
    }
}

impl Screen {
    pub fn new() -> Self {
        Screen {
            w: 0,
            h: 0,
            rows: Vec::new(),
            cells: Vec::new(),
            cursor: (0, 0),
            cursor_visible: true,
            scrollback: VecDeque::new(),
            base: 0,
            first_abs: 0,
            last_dirty: None,
            lost_events: 0,
            version: 0,
        }
    }

    fn push_scrollback(&mut self, line: String) {
        self.scrollback.push_back(line);
        if self.scrollback.len() > SCROLLBACK_CAP {
            self.scrollback.pop_front();
            self.first_abs += 1;
        }
    }

    /// Update from the framebuffer region bytes. Returns true if the text
    /// or cursor changed.
    pub fn update(&mut self, fb: &[u8]) -> bool {
        if fb.len() < FB_HEADER || u16le(fb, 0) != FB_MAGIC {
            return false;
        }
        let w = u16le(fb, 2) as usize;
        let h = u16le(fb, 4) as usize;
        let cx = u16le(fb, 6) as usize;
        let cy = u16le(fb, 8) as usize;
        let vis = fb[10] != 0;
        let dirty = u32::from_le_bytes([fb[12], fb[13], fb[14], fb[15]]);
        if w == 0 || h == 0 || FB_HEADER + w * h * 4 > fb.len() {
            return false;
        }
        let cursor_changed = (cx, cy) != self.cursor || vis != self.cursor_visible;
        let old_cursor_bottom = self.h > 0 && self.cursor.1 + 1 >= self.h;
        self.cursor = (cx.min(w - 1), cy.min(h - 1));
        self.cursor_visible = vis;
        // The kernel bumps the dirty counter on clears; plain writes don't
        // always, so compare content whenever anything might have changed.
        let mut cells = Vec::with_capacity(w * h);
        let mut rows = Vec::with_capacity(h);
        for y in 0..h {
            let mut s = String::with_capacity(w);
            for x in 0..w {
                let o = FB_HEADER + (y * w + x) * 4;
                let c = [fb[o], fb[o + 1], fb[o + 2]];
                s.push(cell_char(c[0]));
                cells.push(c);
            }
            rows.push(s.trim_end().to_string());
        }
        if self.last_dirty == Some(dirty) && rows == self.rows && w == self.w && h == self.h {
            if cursor_changed {
                self.version += 1;
            }
            return cursor_changed;
        }
        self.last_dirty = Some(dirty);
        if w != self.w || h != self.h || self.rows.is_empty() {
            for r in std::mem::take(&mut self.rows) {
                self.push_scrollback(r);
            }
            self.base = self.first_abs + self.scrollback.len() as u64;
        } else {
            let k = scroll_offset(&self.rows, &rows, old_cursor_bottom);
            if k > 0 {
                if k >= h {
                    self.lost_events += 1;
                }
                let old = std::mem::take(&mut self.rows);
                for r in old.into_iter().take(k) {
                    self.push_scrollback(r);
                }
                self.base += k as u64;
            }
        }
        self.w = w;
        self.h = h;
        self.rows = rows;
        self.cells = cells;
        self.version += 1;
        true
    }

    /// Absolute line index of the cursor row.
    pub fn cursor_abs(&self) -> u64 {
        self.base + self.cursor.1 as u64
    }

    /// Lines with absolute index >= `from` (scrollback then screen),
    /// trailing blank screen rows removed.
    pub fn lines_from(&self, from: u64) -> Vec<String> {
        let mut out = Vec::new();
        for (i, l) in self.scrollback.iter().enumerate() {
            if self.first_abs + i as u64 >= from {
                out.push(l.clone());
            }
        }
        let mut screen: Vec<String> = Vec::new();
        for (i, r) in self.rows.iter().enumerate() {
            if self.base + i as u64 >= from {
                screen.push(r.clone());
            }
        }
        while screen.last().is_some_and(|l| l.is_empty()) {
            screen.pop();
        }
        out.extend(screen);
        out
    }

    pub fn text_from(&self, from: u64) -> String {
        self.lines_from(from).join("\n")
    }

    /// Text of the cursor line up to the cursor.
    pub fn cursor_line_prefix(&self) -> String {
        let row = self.rows.get(self.cursor.1).map(|s| s.as_str()).unwrap_or("");
        let x = self.cursor.0.min(row.len());
        row[..x].to_string()
    }

    pub fn dump(&self) -> String {
        let mut rows: Vec<&str> = self.rows.iter().map(|s| s.as_str()).collect();
        while rows.last().is_some_and(|l| l.is_empty()) {
            rows.pop();
        }
        rows.join("\n")
    }
}

/// How many rows the content moved up between two snapshots. The VTE only
/// scrolls when output runs past the bottom row, so a snapshot taken with
/// the cursor elsewhere is an in-place update.
fn scroll_offset(old: &[String], new: &[String], old_cursor_bottom: bool) -> usize {
    let h = old.len();
    if h < 2 || !old_cursor_bottom || old == new {
        return 0;
    }
    // Only the old cursor row (bottom) may have been edited before a scroll,
    // so compare everything above it.
    if old[..h - 1] == new[..h - 1] {
        return 0;
    }
    for k in 1..h - 1 {
        let a = &old[k..h - 1];
        if a == &new[..h - 1 - k] && a.iter().any(|l| !l.is_empty()) {
            return k;
        }
    }
    // Only the (possibly extended) old bottom row survived, now at the top.
    if !old[h - 1].is_empty() && new[0].starts_with(old[h - 1].as_str()) {
        return h - 1;
    }
    // A screenful or more went by (or the screen was cleared): keep all of
    // the old screen as history.
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fb(w: usize, h: usize, lines: &[&str], cy: usize, dirty: u32) -> Vec<u8> {
        let mut b = vec![0u8; FB_HEADER + w * h * 4];
        b[0..2].copy_from_slice(&FB_MAGIC.to_le_bytes());
        b[2..4].copy_from_slice(&(w as u16).to_le_bytes());
        b[4..6].copy_from_slice(&(h as u16).to_le_bytes());
        b[8..10].copy_from_slice(&(cy as u16).to_le_bytes());
        b[10] = 1;
        b[12..16].copy_from_slice(&dirty.to_le_bytes());
        for y in 0..h {
            let l = lines.get(y).copied().unwrap_or("").as_bytes();
            for x in 0..w {
                let o = FB_HEADER + (y * w + x) * 4;
                b[o] = *l.get(x).unwrap_or(&b' ');
                b[o + 1] = 0x0a;
            }
        }
        b
    }

    #[test]
    fn detects_scroll_into_scrollback() {
        let mut s = Screen::new();
        s.update(&fb(10, 3, &["a", "b", "c"], 2, 1));
        s.update(&fb(10, 3, &["b", "c", "d"], 2, 2));
        assert_eq!(s.scrollback, vec!["a".to_string()]);
        assert_eq!(s.base, 1);
        assert_eq!(s.text_from(0), "a\nb\nc\nd");
        assert_eq!(s.text_from(2), "c\nd");
        // Edited bottom line, then scrolled by two.
        s.update(&fb(10, 3, &["b", "c", "d!"], 2, 3));
        s.update(&fb(10, 3, &["d!", "e", "f"], 2, 4));
        assert_eq!(s.text_from(0), "a\nb\nc\nd!\ne\nf");
        assert_eq!(s.cursor_abs(), 5);
    }

    #[test]
    fn in_place_edit_is_not_a_scroll() {
        let mut s = Screen::new();
        s.update(&fb(10, 3, &["x", "y", "> "], 2, 1));
        s.update(&fb(10, 3, &["x", "y", "> ls"], 2, 1));
        assert!(s.scrollback.is_empty());
        assert_eq!(s.rows[2], "> ls");
        // Screen not full yet: new lines appear below, nothing scrolls.
        let mut t = Screen::new();
        t.update(&fb(10, 4, &["a", "> "], 1, 1));
        t.update(&fb(10, 4, &["a", "> x", "out", "> "], 3, 1));
        assert!(t.scrollback.is_empty());
        assert_eq!(t.text_from(0), "a
> x
out
>");
    }
}
