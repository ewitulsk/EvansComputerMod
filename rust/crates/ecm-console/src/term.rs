//! A ComputerCraft-style terminal drawn on the console: a character grid
//! with a cursor (1-based), a text and a background colour (see [`colors`])
//! and a cursor that blinks or not. Each call appends the console escape
//! sequences that draw it (ecm-vte: CUP, CHA, SGR, ED, EL, SU/SD, ?25) to
//! [`Term::out`]; the interpreter writes them to stdout.
//!
//! Text is bytes, one cell each. `write` doesn't wrap: what falls outside
//! the screen is cut off and the cursor still moves past it. `write_wrapped`
//! (the `write`/`print` globals) wraps words and scrolls.
//!
//! Until the program sets the cursor or clears the screen, output only moves
//! within the line and down (column moves and newlines), so it follows on
//! from what is already shown, whatever else wrote to the console meanwhile
//! (a REPL's prompt, echoed input). The console's cursor when the program was
//! spawned (`ECM_CURSOR`) is where get_cursor_pos starts counting.
//!
//! Such a program also keeps the console's cursor; one that positions the
//! cursor gets ComputerCraft's: hidden unless `set_blink(true)`.
//!
//! The console has fixed colours: a palette change is remembered (and read
//! back) but doesn't change how colours look yet.

use crate::colors;

pub struct Term {
    w: i32,
    h: i32,
    x: i32,
    y: i32,
    /// Whether `y` is the console's real cursor row.
    known: bool,
    fg: usize,
    bg: usize,
    blink: bool,
    palette: [u32; 16],
    /// Where the console's cursor is (when we know), to skip needless moves.
    at: Option<(i32, i32)>,
    /// The colours the console draws with now (None: not set by us yet).
    sgr: Option<(usize, usize)>,
    /// Whether the cursor's visibility has been set to `blink` yet.
    shown: Option<bool>,
    used: bool,
    pub out: Vec<u8>,
}

impl Term {
    /// A `w` x `h` terminal with the cursor at column `x`, row `y` (1-based,
    /// None if the row isn't known).
    pub fn new(w: i32, h: i32, x: i32, y: Option<i32>) -> Term {
        let (w, h) = (w.max(1), h.max(1));
        Term {
            w,
            h,
            x: x.clamp(1, w),
            y: y.unwrap_or(h).clamp(1, h),
            known: y.is_some(),
            fg: 0,
            bg: 15,
            blink: false,
            palette: colors::DEFAULT_PALETTE,
            at: None,
            sgr: None,
            shown: None,
            used: false,
            out: Vec::new(),
        }
    }

    /// Size and cursor from the program's environment: COLUMNS, LINES and
    /// ECM_CURSOR ("x,y", 0-based), as the computer sets them at spawn.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Term {
        let num = |k: &str| get(k).and_then(|v| v.trim().parse::<i32>().ok()).filter(|v| *v > 0);
        let (w, h) = (num("COLUMNS").unwrap_or(51), num("LINES").unwrap_or(19));
        let cur = get("ECM_CURSOR").and_then(|v| {
            let (a, b) = v.split_once(',')?;
            Some((a.trim().parse::<i32>().ok()?, b.trim().parse::<i32>().ok()?))
        });
        // The row is only a guess for get_cursor_pos: other output (a prompt, echoed
        // input, shell.write) can move the console's cursor without us.
        let mut t = Term::new(w, h, 1, None);
        if let Some((x, y)) = cur {
            t.x = (x + 1).clamp(1, w);
            t.y = (y + 1).clamp(1, h);
        }
        t
    }

    /// Take the bytes to send to the console.
    pub fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    /// Whether the program drew anything through the term (so [`Term::finish`] must tidy up).
    pub fn used(&self) -> bool {
        self.used
    }

    /// Leave the console as programs expect it: default colours, cursor shown.
    pub fn finish(&mut self) {
        if self.used {
            self.out.extend_from_slice(b"\x1b[0m\x1b[?25h");
            self.sgr = None;
            self.shown = Some(true);
        }
    }

    pub fn size(&self) -> (i32, i32) {
        (self.w, self.h)
    }

    /// The console was resized.
    pub fn resize(&mut self, w: i32, h: i32) {
        self.w = w.max(1);
        self.h = h.max(1);
        self.at = None;
    }

    pub fn cursor(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    pub fn set_cursor(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
        self.known = true;
        self.used = true;
        self.show_cursor();
    }

    pub fn blink(&self) -> bool {
        self.blink
    }

    pub fn set_blink(&mut self, on: bool) {
        self.blink = on;
        self.used = true;
        self.show_cursor();
    }

    pub fn text_colour(&self) -> u32 {
        colors::from_index(self.fg)
    }

    pub fn background_colour(&self) -> u32 {
        colors::from_index(self.bg)
    }

    pub fn set_text_colour(&mut self, colour: f64) -> Result<(), String> {
        self.fg = colors::index(colour)?;
        Ok(())
    }

    pub fn set_background_colour(&mut self, colour: f64) -> Result<(), String> {
        self.bg = colors::index(colour)?;
        Ok(())
    }

    pub fn palette(&self, colour: f64) -> Result<u32, String> {
        Ok(self.palette[colors::index(colour)?])
    }

    pub fn set_palette(&mut self, colour: f64, rgb: u32) -> Result<(), String> {
        self.palette[colors::index(colour)?] = rgb & 0xFF_FFFF;
        Ok(())
    }

    fn colours(&mut self, fg: usize, bg: usize) {
        if self.sgr != Some((fg, bg)) {
            let s = format!("\x1b[0;{};{}m", colors::sgr(fg, false), colors::sgr(bg, true));
            self.out.extend_from_slice(s.as_bytes());
            self.sgr = Some((fg, bg));
        }
    }

    /// Put the console's cursor at our cursor (when it's on screen).
    fn go(&mut self, x: i32) {
        if self.known {
            if self.at != Some((x, self.y)) {
                self.out.extend_from_slice(format!("\x1b[{};{}H", self.y, x).as_bytes());
            }
        } else {
            self.out.extend_from_slice(format!("\x1b[{x}G").as_bytes());
        }
        self.at = Some((x, self.y));
    }

    fn show_cursor(&mut self) {
        // A program that only prints (a REPL, a script) keeps the console's cursor.
        if !self.known && self.shown.is_none() {
            return;
        }
        let visible = self.blink && self.x >= 1 && self.x <= self.w && self.y >= 1 && self.y <= self.h;
        if visible {
            self.go(self.x);
        }
        if self.shown != Some(visible) {
            self.out.extend_from_slice(if visible { b"\x1b[?25h" } else { b"\x1b[?25l" });
            self.shown = Some(visible);
        }
    }

    /// Draw `cells` (byte, fg, bg) at the cursor and move the cursor past them.
    fn put(&mut self, cells: &[(u8, usize, usize)]) {
        self.used = true;
        let start = self.x;
        self.x += cells.len() as i32;
        if self.y < 1 || self.y > self.h {
            return;
        }
        let from = (1 - start).max(0) as usize;
        let to = ((self.w - start + 1).max(0) as usize).min(cells.len());
        if from < to {
            self.go(start + from as i32);
            for &(b, fg, bg) in &cells[from..to] {
                self.colours(fg, bg);
                // Control bytes would be read as console commands.
                self.out.push(if b < 32 || b == 127 { b'?' } else { b });
            }
            let end = start + to as i32;
            self.at = if end <= self.w { Some((end, self.y)) } else { None };
        }
        self.show_cursor();
    }

    /// `term.write`: the text at the cursor, in the current colours.
    pub fn write(&mut self, text: &[u8]) {
        let (fg, bg) = (self.fg, self.bg);
        let cells: Vec<_> = text.iter().map(|&b| (b, fg, bg)).collect();
        self.put(&cells);
    }

    /// `term.blit`: text with a text and a background colour (blit digit) per character.
    pub fn blit(&mut self, text: &[u8], fg: &[u8], bg: &[u8]) -> Result<(), String> {
        if fg.len() != text.len() || bg.len() != text.len() {
            return Err("Arguments must be the same length".into());
        }
        let digit = |c: u8, dflt: usize| (c as char).to_digit(16).map(|d| d as usize).unwrap_or(dflt);
        let cells: Vec<_> = (0..text.len()).map(|i| (text[i], digit(fg[i], self.fg), digit(bg[i], self.bg))).collect();
        self.put(&cells);
        Ok(())
    }

    /// `term.clear`: the whole screen in the background colour; the cursor stays.
    pub fn clear(&mut self) {
        self.used = true;
        self.known = true;
        self.colours(self.fg, self.bg);
        self.out.extend_from_slice(b"\x1b[2J");
        self.at = None;
        self.show_cursor();
    }

    /// `term.clearLine`: the cursor's row in the background colour.
    pub fn clear_line(&mut self) {
        self.used = true;
        if self.y < 1 || self.y > self.h {
            return;
        }
        self.colours(self.fg, self.bg);
        let x = self.x;
        self.go(1);
        self.out.extend_from_slice(b"\x1b[2K");
        self.x = x;
        self.show_cursor();
    }

    /// `term.scroll`: move every row up `n` rows (down if negative); new rows
    /// are in the background colour. The cursor stays.
    pub fn scroll(&mut self, n: i32) {
        self.used = true;
        if n == 0 {
            return;
        }
        self.colours(self.fg, self.bg);
        let n2 = n.unsigned_abs().min(self.h as u32);
        self.out.extend_from_slice(format!("\x1b[{n2}{}", if n > 0 { 'S' } else { 'T' }).as_bytes());
        self.at = None;
        self.show_cursor();
    }

    /// Start of the next line, scrolling at the bottom.
    fn newline(&mut self) {
        if !self.known {
            // The console scrolls by itself at its bottom row.
            self.colours(self.fg, self.bg);
            self.out.push(b'\n');
            self.x = 1;
            self.y = (self.y + 1).min(self.h);
            // Others may write to the console between lines: set both again.
            self.at = None;
            self.sgr = None;
            self.show_cursor();
            return;
        }
        if self.y + 1 <= self.h {
            self.set_cursor(1, self.y + 1);
        } else {
            self.scroll(1);
            self.set_cursor(1, self.h);
        }
    }

    /// The `write` global: text wrapped at word boundaries ("\n" starts a
    /// line), scrolling at the bottom. Returns the number of line breaks.
    pub fn write_wrapped(&mut self, text: &[u8]) -> u32 {
        self.used = true;
        let mut lines = 0;
        let mut i = 0;
        while i < text.len() {
            let b = text[i];
            if b == b'\n' {
                self.newline();
                lines += 1;
                i += 1;
            } else if b == b' ' || b == b'\t' {
                let end = text[i..].iter().position(|&c| c != b' ' && c != b'\t').map_or(text.len(), |p| i + p);
                let spaces = vec![b' '; end - i];
                self.write(&spaces);
                i = end;
            } else {
                let end = text[i..].iter().position(|&c| c == b' ' || c == b'\t' || c == b'\n').map_or(text.len(), |p| i + p);
                let mut word = &text[i..end];
                if self.x + word.len() as i32 - 1 > self.w && self.x > 1 {
                    self.newline();
                    lines += 1;
                }
                // A word longer than a line is broken.
                while word.len() as i32 > self.w - self.x + 1 {
                    let room = (self.w - self.x + 1).max(1) as usize;
                    self.write(&word[..room]);
                    self.newline();
                    lines += 1;
                    word = &word[room..];
                }
                self.write(word);
                i = end;
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t: &mut Term) -> String {
        String::from_utf8_lossy(&t.take()).into_owned()
    }

    #[test]
    fn write_clips_and_moves_the_cursor() {
        let mut t = Term::new(10, 4, 1, Some(1));
        t.set_cursor(8, 2);
        let _ = s(&mut t);
        t.write(b"hello");
        assert_eq!(s(&mut t), "\x1b[2;8H\x1b[0;97;40mhel");
        assert_eq!(t.cursor(), (13, 2), "the cursor moves past the cut-off text");
        t.set_cursor(-1, 3);
        t.write(b"abcd");
        assert_eq!(s(&mut t), "\x1b[3;1Hcd");
        t.set_cursor(1, 9);
        t.write(b"off screen");
        assert_eq!(s(&mut t), "");
        assert_eq!(t.cursor(), (11, 9));
    }

    #[test]
    fn colours_blit_and_clearing() {
        let mut t = Term::new(10, 4, 1, Some(1));
        t.set_text_colour(16384.0).unwrap(); // red
        t.set_background_colour(2048.0).unwrap(); // blue
        assert!(t.set_text_colour(0.0).is_err());
        t.clear();
        assert_eq!(s(&mut t), "\x1b[0;31;44m\x1b[2J\x1b[?25l");
        t.set_cursor(1, 1);
        t.blit(b"ab", b"0e", b"ff").unwrap();
        assert_eq!(s(&mut t), "\x1b[1;1H\x1b[0;97;40ma\x1b[0;31;40mb");
        assert!(t.blit(b"ab", b"0", b"ff").is_err());
        assert_eq!(t.text_colour(), 16384);
        t.set_blink(true);
        assert_eq!(s(&mut t), "\x1b[?25h", "the console cursor is already at 3,1");
        t.scroll(-2);
        assert!(s(&mut t).contains("\x1b[2T"));
        t.finish();
        assert!(s(&mut t).ends_with("\x1b[0m\x1b[?25h"));
    }

    #[test]
    fn write_wrapped_breaks_words_and_scrolls() {
        let mut t = Term::new(10, 3, 1, Some(2));
        assert_eq!(t.write_wrapped(b"one two three"), 1);
        assert_eq!(t.cursor(), (6, 3));
        // " four" ends exactly at the last column.
        assert_eq!(t.write_wrapped(b" four"), 0);
        assert_eq!(t.write_wrapped(b"\nabcdefghijklmno"), 2);
        let out = s(&mut t);
        assert_eq!(out.matches("\x1b[1S").count(), 2, "scrolled twice at the bottom: {out:?}");
        assert_eq!(t.cursor(), (6, 3));
    }

    #[test]
    fn an_unknown_row_follows_on_from_the_console() {
        let mut t = Term::from_env(|k| match k {
            "COLUMNS" => Some("80".into()),
            "LINES" => Some("25".into()),
            _ => None,
        });
        assert_eq!(t.size(), (80, 25));
        t.write_wrapped(b"hi\n");
        assert_eq!(s(&mut t), "\x1b[1G\x1b[0;97;40mhi\n", "the cursor stays as it was");
        t.write_wrapped(b"there");
        assert_eq!(s(&mut t), "\x1b[1G\x1b[0;97;40mthere", "a column move, as other output may have come first");
        let mut t2 = Term::from_env(|k| (k == "ECM_CURSOR").then(|| "4,7".into()));
        assert_eq!((t2.size(), t2.cursor()), ((51, 19), (5, 8)));
        t2.write(b"x");
        assert_eq!(s(&mut t2), "\x1b[5G\x1b[0;97;40mx", "the row is only a guess");
    }
}
