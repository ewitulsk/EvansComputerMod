//! The editor for a program reading lines from its terminal (the Python and
//! Lua REPLs and their `input()`): keys come in on stdin, the echo goes out
//! on stdout (the terminal doesn't echo a running program's input).

use crate::{Completer, Editor, Event, NoCompletion, Out};
use std::io::{Read, Write};

/// Keys read past the end of the last line (the rest of a pasted block).
static PENDING: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

/// What [`read_line`] got.
#[derive(Debug, PartialEq, Eq)]
pub enum Input {
    Line(String),
    /// Ctrl+C (only when the caller reads Ctrl+C itself).
    Interrupt,
    /// End of input (Ctrl+D on an empty line, or stdin closed).
    Eof,
}

/// stdout as the editor's output, `COLUMNS` wide.
pub struct Stdout {
    width: u16,
}

impl Default for Stdout {
    fn default() -> Self {
        let width = std::env::var("COLUMNS").ok().and_then(|v| v.parse().ok()).filter(|&w| w > 0).unwrap_or(80);
        Stdout { width }
    }
}

impl Out for Stdout {
    fn write(&mut self, data: &[u8]) {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(data);
        let _ = o.flush();
    }
    fn width(&self) -> u16 {
        self.width
    }
}

/// Read one line with `prompt`. With `own_interrupt` the program takes Ctrl+C
/// while it waits (the terminal's mode 7727) and gets [`Input::Interrupt`];
/// otherwise Ctrl+C stops the program as usual.
pub fn read_line(ed: &mut Editor, prompt: &str, own_interrupt: bool) -> Input {
    read_line_with(ed, prompt, own_interrupt, &mut NoCompletion)
}

/// [`read_line`] with Tab completion.
pub fn read_line_with(ed: &mut Editor, prompt: &str, own_interrupt: bool, comp: &mut dyn Completer) -> Input {
    let mut out = Stdout::default();
    if own_interrupt {
        out.write(b"\x1b[?7727h");
    }
    ed.start(prompt, &mut out);
    let mut data = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()));
    let result = feed_until_line(ed, &mut out, comp, &mut data, &mut std::io::stdin().lock());
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = data;
    if own_interrupt {
        out.write(b"\x1b[?7727l");
    }
    result
}

/// Feed `data` (refilled from `input` when empty) until a line, Ctrl+C or the
/// end; `data` keeps what follows.
fn feed_until_line(ed: &mut Editor, out: &mut dyn Out, comp: &mut dyn Completer, data: &mut Vec<u8>, input: &mut dyn Read) -> Input {
    loop {
        if data.is_empty() {
            let mut buf = [0u8; 256];
            match input.read(&mut buf) {
                Ok(0) | Err(_) => return Input::Eof,
                Ok(n) => data.extend_from_slice(&buf[..n]),
            }
        }
        let mut i = 0;
        while i < data.len() {
            let ev = ed.feed_byte(data[i], out, comp);
            i += 1;
            let done = match ev {
                Some(Event::Line(l)) => Some(Input::Line(l)),
                Some(Event::Interrupt) => Some(Input::Interrupt),
                Some(Event::Eof) => {
                    out.write(b"\r\n");
                    Some(Input::Eof)
                }
                Some(Event::Terminate) | None => None,
            };
            if let Some(r) = done {
                data.drain(..i);
                return r;
            }
        }
        data.clear();
    }
}

/// Load a history file (one line per entry) into `ed`.
pub fn load_history(ed: &mut Editor, path: &str) {
    if let Ok(text) = std::fs::read_to_string(path) {
        ed.set_history(text.lines().map(str::to_string).collect());
    }
}

/// Save `ed`'s history to `path` if lines were added.
pub fn save_history(ed: &mut Editor, path: &str) {
    if ed.take_new_history() {
        let mut text = ed.history().join("\n");
        text.push('\n');
        let _ = std::fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sink;
    impl Out for Sink {
        fn write(&mut self, _: &[u8]) {}
        fn width(&self) -> u16 {
            80
        }
    }

    #[test]
    fn a_pasted_block_is_read_one_line_at_a_time() {
        let mut ed = Editor::new();
        let mut data = Vec::new();
        let mut input: &[u8] = b"a = 1\rprint(a)\r\x1b[Dx";
        let mut c = NoCompletion;
        assert_eq!(feed_until_line(&mut ed, &mut Sink, &mut c, &mut data, &mut input), Input::Line("a = 1".into()));
        assert_eq!(feed_until_line(&mut ed, &mut Sink, &mut c, &mut data, &mut input), Input::Line("print(a)".into()));
        assert_eq!(feed_until_line(&mut ed, &mut Sink, &mut c, &mut data, &mut input), Input::Eof);
        assert_eq!(ed.pending(), "x");
        let mut input: &[u8] = b"\x03";
        assert_eq!(feed_until_line(&mut ed, &mut Sink, &mut c, &mut data, &mut input), Input::Interrupt);
    }
}
