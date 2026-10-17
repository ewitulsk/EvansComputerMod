//! The shell's line editor (`ecm-lineedit`, shared with the REPLs) bound to
//! the kernel: a [`Term`] to draw on, Tab completion of commands and paths,
//! the saved history, and how keyboard input is split between the editor,
//! the foreground program and the terminal itself (Ctrl+C, Ctrl+T, scrolling).

use crate::console::{Modes, Scroll, Term};
use crate::fs;

pub use ecm_lineedit::{Completer, Editor, Event, NoCompletion};

/// Where the shell keeps its history (shared by the console and every session).
pub const HISTORY_FILE: &str = ".history";

/// A [`Term`] as the editor's output.
pub struct TermOut<'a>(pub &'a mut dyn Term);

impl ecm_lineedit::Out for TermOut<'_> {
    fn write(&mut self, data: &[u8]) {
        self.0.write(data);
    }
    fn width(&self) -> u16 {
        self.0.width()
    }
}

/// Load the saved history into `ed`.
pub fn load_history(ed: &mut Editor) {
    if let Some(text) = fs::read_to_string(HISTORY_FILE) {
        ed.set_history(text.lines().map(str::to_string).collect());
    }
}

/// Save the history if lines were added since the last save.
pub fn save_history(ed: &mut Editor) {
    if ed.take_new_history() {
        write_history(ed);
    }
}

fn write_history(ed: &Editor) {
    let mut text = ed.history().join("\n");
    text.push('\n');
    fs::write(HISTORY_FILE, text.as_bytes());
}

/// The `history` builtin: `history`, `history N` (the last N), `history -c`.
pub fn history_cmd(ed: &mut Editor, args: &[String], con: &mut dyn Term) {
    match args.first().map(String::as_str) {
        Some("-c") => {
            ed.clear_history();
            write_history(ed);
        }
        Some(n) if n.parse::<usize>().is_err() => con.println("Usage: history [N] | history -c"),
        arg => {
            let h = ed.history();
            let n = arg.and_then(|a| a.parse::<usize>().ok()).unwrap_or(h.len()).min(h.len());
            for (i, line) in h.iter().enumerate().skip(h.len() - n) {
                con.println(&format!("{:>5}  {}", i + 1, line));
            }
        }
    }
}

/// Tab completion for the shell: the first word completes to builtins and
/// programs (`bin/`, `server-bin/`, `*.wasm` here), later words to paths.
pub struct ShellCompleter<'a> {
    pub cwd: &'a str,
}

impl Completer for ShellCompleter<'_> {
    fn complete(&mut self, line: &str, cursor: usize) -> (usize, Vec<String>) {
        let before = &line[..cursor];
        let start = before.rfind([' ', '|', '<', '>', '&']).map(|i| i + 1).unwrap_or(0);
        let word = &before[start..];
        let head = before[..start].trim_end();
        let first = head.is_empty() || head.ends_with(['|', '&']);
        if first && !word.contains('/') {
            return (start, commands(self.cwd, word));
        }
        (start, paths(self.cwd, word))
    }
}

fn commands(cwd: &str, prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = crate::shell::BUILTINS.iter().map(|s| s.to_string()).collect();
    for dir in [cwd, "bin", "server-bin"] {
        for e in fs::list_dir(dir) {
            if e.is_dir {
                continue;
            }
            // In the current directory only programs count.
            if dir == cwd && !e.name.ends_with(".wasm") {
                continue;
            }
            out.push(e.name.strip_suffix(".wasm").unwrap_or(&e.name).to_string());
        }
    }
    out.retain(|c| c.starts_with(prefix));
    out.sort();
    out.dedup();
    out
}

fn paths(cwd: &str, word: &str) -> Vec<String> {
    let (dir_part, name) = match word.rfind('/') {
        Some(i) => (&word[..=i], &word[i + 1..]),
        None => ("", word),
    };
    let dir = fs::resolve(cwd, dir_part);
    fs::list_dir(&dir)
        .into_iter()
        .filter(|e| e.name.starts_with(name))
        .map(|e| format!("{}{}{}", dir_part, e.name, if e.is_dir { "/" } else { "" }))
        .collect()
}

/// One piece of keyboard input, after the terminal took its own keys out.
#[derive(Debug, PartialEq, Eq)]
pub enum Piece {
    /// Bytes for the line editor or the program.
    Data(Vec<u8>),
    /// Ctrl+C that stops the foreground program.
    Interrupt,
    /// Ctrl+T.
    Terminate,
    /// Ctrl+Z that pauses the foreground program (job control).
    Suspend,
    /// Shift+PgUp/PgDn/Up/Down/Home/End or the wheel: scroll the terminal.
    Scroll(Scroll),
}

/// How [`split_input`] treats the keys the terminal can take.
#[derive(Clone, Copy)]
pub struct InputRules {
    /// Ctrl+C becomes [`Piece::Interrupt`] (a program is running and doesn't read it itself).
    pub interrupt: bool,
    /// Bracketed-paste markers are removed (the program didn't ask for them).
    pub strip_paste: bool,
    /// Shift+arrows and friends scroll (the console, when no full-screen program runs).
    pub scroll: bool,
    /// A full-screen program runs: the scroll keys go to it as plain keys.
    pub to_program: bool,
}

impl InputRules {
    /// The rules for a program running with `modes` (`scroll`: this terminal has a scrollback view).
    pub fn for_program(modes: Modes, scroll: bool) -> Self {
        Self {
            interrupt: !modes.program_takes_ctrl_c(),
            strip_paste: !modes.bracketed_paste,
            scroll: scroll && !modes.alternate,
            to_program: scroll && modes.alternate,
        }
    }

    /// The rules at the shell prompt (the editor reads Ctrl+C and pastes itself).
    pub fn for_prompt(scroll: bool) -> Self {
        Self { interrupt: false, strip_paste: false, scroll, to_program: false }
    }
}

const SCROLL_KEYS: &[(&[u8], Scroll, &[u8])] = &[
    (b"\x1b[5;2~", Scroll::Pages(-1), b"\x1b[5~"),
    (b"\x1b[6;2~", Scroll::Pages(1), b"\x1b[6~"),
    (b"\x1b[1;2A", Scroll::Lines(-1), b"\x1b[A"),
    (b"\x1b[1;2B", Scroll::Lines(1), b"\x1b[B"),
    (b"\x1b[1;2H", Scroll::Top, b"\x1b[H"),
    (b"\x1b[1;2F", Scroll::Bottom, b"\x1b[F"),
];

/// Split keyboard input into data and the terminal's own keys.
pub fn split_input(bytes: &[u8], rules: InputRules) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut data = Vec::new();
    let mut i = 0;
    fn flush(data: &mut Vec<u8>, out: &mut Vec<Piece>) {
        if !data.is_empty() {
            out.push(Piece::Data(std::mem::take(data)));
        }
    }
    'outer: while i < bytes.len() {
        let b = bytes[i];
        if b == 0x14 {
            flush(&mut data, &mut out);
            out.push(Piece::Terminate);
            i += 1;
            continue;
        }
        if (b == 0x03 || b == 0x1a) && rules.interrupt {
            flush(&mut data, &mut out);
            out.push(if b == 0x03 { Piece::Interrupt } else { Piece::Suspend });
            i += 1;
            continue;
        }
        if b == 0x1b {
            let rest = &bytes[i..];
            if rules.scroll || rules.to_program {
                for (seq, s, plain) in SCROLL_KEYS {
                    if rest.starts_with(seq) {
                        if rules.scroll {
                            flush(&mut data, &mut out);
                            out.push(Piece::Scroll(*s));
                        } else {
                            data.extend_from_slice(plain);
                        }
                        i += seq.len();
                        continue 'outer;
                    }
                }
            }
            if rules.strip_paste && (rest.starts_with(b"\x1b[200~") || rest.starts_with(b"\x1b[201~")) {
                i += 6;
                continue;
            }
        }
        data.push(b);
        i += 1;
    }
    flush(&mut data, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_splits_into_data_and_terminal_keys() {
        let prog = InputRules::for_program(Modes::default(), true);
        assert_eq!(
            split_input(b"ab\x03cd\x1b[5;2~\x14\x1b[200~x\x1b[201~", prog),
            vec![
                Piece::Data(b"ab".to_vec()),
                Piece::Interrupt,
                Piece::Data(b"cd".to_vec()),
                Piece::Scroll(Scroll::Pages(-1)),
                Piece::Terminate,
                Piece::Data(b"x".to_vec()),
            ]
        );
        // A full-screen program: Ctrl+C and the wheel (as arrows) are its keys.
        let full = InputRules::for_program(Modes { alternate: true, bracketed_paste: true, ..Modes::default() }, true);
        assert_eq!(split_input(b"\x03\x1b[1;2A\x1b[200~", full), vec![Piece::Data(b"\x03\x1b[A\x1b[200~".to_vec())]);
        // At the prompt Ctrl+C goes to the editor.
        assert_eq!(split_input(b"\x03", InputRules::for_prompt(true)), vec![Piece::Data(vec![3])]);
        // SSH sessions don't scroll.
        let ssh = InputRules::for_prompt(false);
        assert_eq!(split_input(b"\x1b[1;2A", ssh), vec![Piece::Data(b"\x1b[1;2A".to_vec())]);
    }

    #[test]
    fn completes_commands_and_paths() {
        let _g = crate::hal::test_lock();
        use crate::hal::ffi;
        ffi::FILES.with(|f| {
            let mut f = f.borrow_mut();
            f.insert("bin/hello.wasm".into(), vec![0]);
            f.insert("server-bin/help.wasm".into(), vec![0]);
            f.insert("home/notes.txt".into(), vec![0]);
            f.insert("home/new/x".into(), vec![0]);
        });
        let mut c = ShellCompleter { cwd: "" };
        let (s, w) = c.complete("he", 2);
        assert_eq!((s, w), (0, vec!["hello".to_string(), "help".to_string()]));
        assert_eq!(c.complete("his", 3).1, vec!["history".to_string()]);
        let (s, w) = c.complete("cat home/n", 10);
        assert_eq!(s, 4);
        assert_eq!(w, vec!["home/new/".to_string(), "home/notes.txt".to_string()]);
        let mut c = ShellCompleter { cwd: "home" };
        assert_eq!(c.complete("cat no", 6).1, vec!["notes.txt".to_string()]);
        assert_eq!(c.complete("ls | he", 7).1.len(), 2, "after a pipe: a command");
    }
}
