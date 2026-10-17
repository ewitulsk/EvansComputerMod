//! The editor's output is fed through the real terminal emulator (`ecm-vte`)
//! and the tests read the screen, so redraws (wrapping, clearing) are checked
//! the way the player sees them.

use super::*;
use ecm_vte::Vte;

struct Screen(Vte);

impl Out for Screen {
    fn write(&mut self, data: &[u8]) {
        self.0.write(data);
    }
    fn width(&self) -> u16 {
        self.0.width()
    }
}

impl Screen {
    fn new(w: u16, h: u16) -> Self {
        Screen(Vte::new_virtual(w, h))
    }
    fn row(&self, y: u16) -> String {
        (0..self.0.width()).map(|x| self.0.cell(x, y).0 as char).collect::<String>().trim_end().to_string()
    }
    fn rows(&self) -> Vec<String> {
        (0..self.0.height()).map(|y| self.row(y)).collect()
    }
    fn cursor(&self) -> (u16, u16) {
        self.0.cursor()
    }
}

struct Words(Vec<&'static str>);

impl Completer for Words {
    fn complete(&mut self, line: &str, cursor: usize) -> (usize, Vec<String>) {
        let start = line[..cursor].rfind(' ').map(|i| i + 1).unwrap_or(0);
        let w = &line[start..cursor];
        (start, self.0.iter().filter(|c| c.starts_with(w)).map(|c| c.to_string()).collect())
    }
}

fn setup(w: u16) -> (Editor, Screen) {
    let mut ed = Editor::new();
    let mut s = Screen::new(w, 6);
    ed.start("/ > ", &mut s);
    (ed, s)
}

fn feed(ed: &mut Editor, s: &mut Screen, data: &[u8]) -> Vec<Event> {
    ed.feed(data, s, &mut NoCompletion)
}

fn line(ev: &[Event]) -> String {
    match ev.last() {
        Some(Event::Line(l)) => l.clone(),
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn left_right_insert_and_delete_in_the_middle() {
    let (mut ed, mut s) = setup(40);
    feed(&mut ed, &mut s, b"ecoh hi");
    // Left x5 to between "ec" and "oh", fix the typo.
    feed(&mut ed, &mut s, b"\x1b[D\x1b[D\x1b[D\x1b[D\x1b[D");
    assert_eq!(s.cursor(), (6, 0));
    feed(&mut ed, &mut s, b"h");
    feed(&mut ed, &mut s, b"\x1b[C\x1b[C\x7f");
    assert_eq!(s.row(0), "/ > echo hi");
    assert_eq!(s.cursor(), (8, 0));
    // Delete at the cursor; Home/End; Ctrl+A/Ctrl+E.
    feed(&mut ed, &mut s, b"\x1b[H");
    assert_eq!(s.cursor(), (4, 0));
    feed(&mut ed, &mut s, b"\x1b[3~");
    assert_eq!(s.row(0), "/ > cho hi");
    feed(&mut ed, &mut s, b"e\x05!");
    assert_eq!(s.row(0), "/ > echo hi!");
    feed(&mut ed, &mut s, b"\x01\x1b[F");
    assert_eq!(s.cursor(), (12, 0));
    assert_eq!(line(&feed(&mut ed, &mut s, b"\r")), "echo hi!");
}

#[test]
fn word_jumps_and_kill_yank() {
    let (mut ed, mut s) = setup(60);
    feed(&mut ed, &mut s, b"ping -c 3 10.0.0.2");
    feed(&mut ed, &mut s, b"\x1b[1;5D"); // Ctrl+Left: start of "2"
    assert_eq!(ed.cursor(), 17);
    feed(&mut ed, &mut s, b"\x1bb\x1bb"); // Alt+B twice: "0" then "0" of 10.0
    assert_eq!(ed.cursor(), 13);
    feed(&mut ed, &mut s, b"\x1b[1;5C");
    assert_eq!(ed.cursor(), 14);
    // Ctrl+W deletes the space-separated word before the cursor.
    feed(&mut ed, &mut s, b"\x05\x17");
    assert_eq!(s.row(0), "/ > ping -c 3");
    // Ctrl+Y puts it back.
    feed(&mut ed, &mut s, b"\x19");
    assert_eq!(s.row(0), "/ > ping -c 3 10.0.0.2");
    // Ctrl+U kills to the start, Ctrl+K to the end.
    feed(&mut ed, &mut s, b"\x1b[1;5D\x1b[1;5D\x1b[1;5D\x1b[1;5D\x1b[1;5D");
    assert_eq!(ed.cursor(), 8);
    feed(&mut ed, &mut s, b"\x0b");
    assert_eq!(s.row(0), "/ > ping -c");
    feed(&mut ed, &mut s, b"\x15");
    assert_eq!(s.row(0), "/ >");
    feed(&mut ed, &mut s, b"\x19");
    assert_eq!(s.row(0), "/ > ping -c");
    // Alt+D deletes the next word, Alt+Backspace the previous one.
    feed(&mut ed, &mut s, b"\x01\x1bd");
    assert_eq!(s.row(0), "/ >  -c");
    feed(&mut ed, &mut s, b"\x05\x1b\x7f");
    assert_eq!(s.row(0), "/ >  -");
}

#[test]
fn long_lines_wrap_and_redraw_across_rows() {
    let (mut ed, mut s) = setup(10);
    // "/ > " + 14 chars = 18 cells: two rows.
    feed(&mut ed, &mut s, b"abcdefghijklmn");
    assert_eq!(s.row(0), "/ > abcdef");
    assert_eq!(s.row(1), "ghijklmn");
    // Exactly at the edge: the cursor moves to the next row.
    feed(&mut ed, &mut s, b"op");
    assert_eq!(s.cursor(), (0, 2));
    // Home goes back up a row; insert pushes text over the wrap.
    feed(&mut ed, &mut s, b"\x01XY");
    assert_eq!(s.row(0), "/ > XYabcd");
    assert_eq!(s.row(1), "efghijklmn");
    assert_eq!(s.row(2), "op");
    assert_eq!(s.cursor(), (6, 0));
    // Ctrl+K from there clears the rows below.
    feed(&mut ed, &mut s, b"\x0b");
    assert_eq!(s.rows()[..3], ["/ > XY".to_string(), String::new(), String::new()]);
    assert_eq!(line(&feed(&mut ed, &mut s, b"\n")), "XY");
}

#[test]
fn history_ctrl_p_n_and_draft() {
    let (mut ed, mut s) = setup(40);
    feed(&mut ed, &mut s, b"ls\r");
    ed.start("/ > ", &mut s);
    feed(&mut ed, &mut s, b"pwd\r");
    ed.start("/ > ", &mut s);
    feed(&mut ed, &mut s, b"dra");
    feed(&mut ed, &mut s, b"\x10\x10");
    assert_eq!(ed.pending(), "ls");
    feed(&mut ed, &mut s, b"\x0e");
    assert_eq!(ed.pending(), "pwd");
    feed(&mut ed, &mut s, b"\x1b[B");
    assert_eq!(ed.pending(), "dra");
    // CR LF counts as one Enter.
    let ev = feed(&mut ed, &mut s, b"\r\n");
    assert_eq!(ev, vec![Event::Line("dra".into())]);
    assert_eq!(ed.history(), ["ls", "pwd", "dra"]);
}

#[test]
fn reverse_search() {
    let (mut ed, mut s) = setup(60);
    for l in ["ping 10.0.0.1", "ls /bin", "ping 10.0.0.2", "cat x"] {
        ed.add_history(l);
    }
    feed(&mut ed, &mut s, b"\x12pi");
    assert_eq!(s.row(0), "(reverse-i-search)`pi': ping 10.0.0.2");
    feed(&mut ed, &mut s, b"\x12");
    assert_eq!(s.row(0), "(reverse-i-search)`pi': ping 10.0.0.1");
    feed(&mut ed, &mut s, b"\x12");
    assert_eq!(s.row(0), "(failed reverse-i-search)`pi': ping 10.0.0.1", "no older match");
    // Esc cancels back to the empty line.
    feed(&mut ed, &mut s, b"\x1b");
    feed(&mut ed, &mut s, b"x");
    assert_eq!(s.row(0), "/ > x");
    feed(&mut ed, &mut s, b"\x15");
    // A movement key keeps the match for editing; Enter runs the match.
    feed(&mut ed, &mut s, b"\x12cat");
    feed(&mut ed, &mut s, b"\x1b[F");
    assert_eq!(s.row(0), "/ > cat x");
    feed(&mut ed, &mut s, b"y");
    assert_eq!(line(&feed(&mut ed, &mut s, b"\r")), "cat xy");
    ed.start("/ > ", &mut s);
    feed(&mut ed, &mut s, b"\x12ls");
    assert_eq!(line(&feed(&mut ed, &mut s, b"\r")), "ls /bin");
    // No match.
    ed.start("/ > ", &mut s);
    feed(&mut ed, &mut s, b"\x12zzz");
    assert!(s.row(s.cursor().1).starts_with("(failed reverse-i-search)`zzz'"));
}

#[test]
fn bang_expansion() {
    let h: Vec<String> = ["ls", "ping 1.2.3.4", "cat a"].iter().map(|s| s.to_string()).collect();
    assert_eq!(expand_history("!!", &h).unwrap(), Some("cat a".into()));
    assert_eq!(expand_history("sudo !!", &h).unwrap(), Some("sudo cat a".into()));
    assert_eq!(expand_history("!2", &h).unwrap(), Some("ping 1.2.3.4".into()));
    assert_eq!(expand_history("!-3", &h).unwrap(), Some("ls".into()));
    assert_eq!(expand_history("!pi", &h).unwrap(), Some("ping 1.2.3.4".into()));
    assert_eq!(expand_history("echo hi!", &h).unwrap(), None);
    assert_eq!(expand_history("echo '!!'", &h).unwrap(), None);
    assert_eq!(expand_history("a ! b", &h).unwrap(), None);
    assert!(expand_history("!9", &h).is_err());
    assert!(expand_history("!zz", &h).is_err());
    // Through the editor: the expansion is echoed and stored.
    let (mut ed, mut s) = setup(40);
    feed(&mut ed, &mut s, b"echo one\r");
    ed.start("/ > ", &mut s);
    assert_eq!(line(&feed(&mut ed, &mut s, b"!!\r")), "echo one");
    assert_eq!(s.row(2), "echo one");
    ed.start("/ > ", &mut s);
    assert_eq!(line(&feed(&mut ed, &mut s, b"!nope\r")), "");
    assert_eq!(s.row(4), "!nope: event not found");
}

#[test]
fn tab_completion_unique_common_and_list() {
    let (mut ed, mut s) = setup(40);
    let mut c = Words(vec!["ifconfig", "ip", "iqplay", "iqrec", "bin/"]);
    ed.feed(b"ifc\t", &mut s, &mut c);
    assert_eq!(s.row(0), "/ > ifconfig");
    ed.feed(b"\x15iq\t", &mut s, &mut c);
    assert_eq!(s.row(0), "/ > iq"); // "iqplay"/"iqrec": no longer prefix
    ed.feed(b"\t", &mut s, &mut c);
    assert_eq!(s.row(1), "iqplay  iqrec");
    assert_eq!(s.row(2), "/ > iq");
    // A directory gets no space after it.
    ed.feed(b"\x15b\t", &mut s, &mut c);
    assert_eq!(ed.pending(), "bin/");
    ed.feed(b"\x15i\t", &mut s, &mut c);
    assert_eq!(ed.pending(), "i");
}

#[test]
fn bracketed_paste_inserts_text_and_newlines_enter() {
    let (mut ed, mut s) = setup(40);
    let mut c = Words(vec!["ifconfig"]);
    let ev = ed.feed(b"\x1b[200~echo a\tb\x01\recho c\x1b[201~", &mut s, &mut c);
    assert_eq!(ev, vec![Event::Line("echo a b".into())]);
    assert_eq!(ed.pending(), "echo c");
    // After the paste, keys work again.
    ed.feed(b"\x01", &mut s, &mut c);
    assert_eq!(ed.cursor(), 0);
}

#[test]
fn ctrl_c_ctrl_d_ctrl_t_ctrl_l() {
    let (mut ed, mut s) = setup(40);
    feed(&mut ed, &mut s, b"half typed");
    assert_eq!(feed(&mut ed, &mut s, b"\x03"), vec![Event::Interrupt]);
    assert_eq!(s.row(0), "/ > half typed^C");
    assert_eq!(ed.pending(), "");
    ed.start("/ > ", &mut s);
    assert_eq!(feed(&mut ed, &mut s, b"\x04"), vec![Event::Eof]);
    feed(&mut ed, &mut s, b"ab\x01\x04");
    assert_eq!(ed.pending(), "b");
    assert_eq!(feed(&mut ed, &mut s, b"\x14"), vec![Event::Terminate]);
    assert_eq!(ed.pending(), "");
    feed(&mut ed, &mut s, b"keep");
    feed(&mut ed, &mut s, b"\x0c");
    assert_eq!(s.row(0), "/ > keep");
    assert_eq!(s.row(1), "");
}

#[test]
fn split_escape_sequences_and_ss3_keys() {
    let (mut ed, mut s) = setup(40);
    feed(&mut ed, &mut s, b"abc");
    feed(&mut ed, &mut s, b"\x1b");
    feed(&mut ed, &mut s, b"[");
    feed(&mut ed, &mut s, b"D");
    assert_eq!(ed.cursor(), 2);
    feed(&mut ed, &mut s, b"\x1bOH");
    assert_eq!(ed.cursor(), 0);
    feed(&mut ed, &mut s, b"\x1b[4~");
    assert_eq!(ed.cursor(), 3);
}

#[test]
fn columns_and_prompt_width() {
    let n: Vec<String> = ["a", "bb", "ccc", "d", "e"].iter().map(|s| s.to_string()).collect();
    assert_eq!(columns(&n, 12), "a    d\r\nbb   e\r\nccc\r\n");
    assert_eq!(visible_width("\x1b[32m/ >\x1b[0m "), 4);
}
