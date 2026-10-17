//! `ecm-man`: the computer's manual. The pages are plain text files in
//! `pages/` (format: `pages/FORMAT.md`) compiled in by `build.rs`; a computer
//! can add or override pages as `/usr/share/man/<name>.txt`. This crate
//! renders a page for a terminal width (headings in bold, paragraphs
//! re-wrapped), builds `help`'s index and `help <topic>` summaries, searches
//! (`man -k`), and has the pager `man`, `help` and `less` share.

use std::fmt::Write as _;

include!(concat!(env!("OUT_DIR"), "/pages.rs"));

/// Where a computer keeps pages of its own (same format, they win).
pub const LOCAL_DIR: &str = "/usr/share/man";

/// `help`'s index: headings and the pages under each.
pub const CATEGORIES: &[(&str, &[&str])] = &[
    ("Getting started", &["intro", "shell", "keys", "terminal", "filesystem", "help", "man", "less", "history"]),
    ("Files", &["ls", "cd", "pwd", "cat", "cp", "mv", "rm", "mkdir", "touch", "echo", "grep", "wc", "edit", "upload", "git"]),
    ("Programs and jobs", &["ps", "kill", "jobs", "fg", "bg", "sleep", "clear", "exit", "passwd"]),
    ("Scripting", &["scripting", "python", "lua", "peripheral", "peripherals"]),
    ("Desktop and apps", &["gui", "desktop", "terminal-app", "files", "editor", "notes", "settings", "store", "pkg", "picker", "chooser", "visual"]),
    ("ePhone", &["phone-guide", "phone"]),
    ("Networking", &["networking", "ifconfig", "ip", "ping", "traceroute", "dhclient", "dhcpd", "wifi", "iw", "wpa_cli", "wpa_supplicant", "switch", "router", "tcpdump"]),
    ("Names and the web", &["nslookup", "dig", "named", "resolvectl", "domain", "whois", "registrard", "registryd", "ianad", "httpd", "curl", "disco"]),
    ("Remote login and chat", &["ssh", "sshd", "ssh-keygen", "chat", "chatd", "chatapp"]),
    ("Radio", &["radio", "rx_fm", "rx_am", "rx_ssb", "waterfall", "scan", "tx_tone", "afsk1200", "radio_station", "iqrec", "iqplay", "radiod", "framed", "antenna"]),
    ("Cellular", &["cellular", "cell", "celld", "celld-site", "cored", "cellsite", "carrier", "simtool"]),
    ("Games, sound and video", &["gba", "gba-app", "controllertest", "mousetest", "gfxtest", "beep", "aplay", "player", "video", "videoplayer"]),
];

/// A built-in page's text.
pub fn builtin(name: &str) -> Option<&'static str> {
    PAGES.binary_search_by(|(n, _)| (*n).cmp(name)).ok().map(|i| PAGES[i].1)
}

/// A page by name: the computer's own (`/usr/share/man/<name>.txt`) first,
/// then the built-in one.
pub fn load(name: &str) -> Option<String> {
    if name.is_empty() || name.contains('/') || name.contains("..") {
        return None;
    }
    std::fs::read_to_string(format!("{LOCAL_DIR}/{name}.txt")).ok().or_else(|| builtin(name).map(str::to_string))
}

/// Pages only this computer has (in `/usr/share/man`), sorted.
pub fn local_names() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(LOCAL_DIR)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".txt")).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// The names of the built-in pages.
pub fn names() -> impl Iterator<Item = &'static str> {
    PAGES.iter().map(|(n, _)| *n)
}

/// The one-line summary from a page's NAME section (`name - summary`).
pub fn summary(text: &str) -> Option<String> {
    let mut lines = text.lines();
    lines.find(|l| l.trim_end() == "NAME")?;
    let first = lines.find(|l| !l.trim().is_empty())?;
    let (_, s) = first.trim().split_once(" - ")?;
    Some(s.trim().to_string())
}

/// The lines of one section (without its heading), or None.
pub fn section<'a>(text: &'a str, heading: &str) -> Option<Vec<&'a str>> {
    let mut out = None;
    for l in text.lines() {
        if is_heading(l) {
            if out.is_some() {
                break;
            }
            if l.trim_end() == heading {
                out = Some(Vec::new());
            }
            continue;
        }
        if let Some(v) = &mut out {
            v.push(l);
        }
    }
    out.map(|mut v: Vec<&str>| {
        while v.last().is_some_and(|l| l.trim().is_empty()) {
            v.pop();
        }
        v
    })
}

fn is_heading(l: &str) -> bool {
    let t = l.trim_end();
    !t.is_empty()
        && !l.starts_with(' ')
        && t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == ' ' || c == '-')
        && t.chars().any(|c| c.is_ascii_uppercase())
}

fn indent_of(l: &str) -> usize {
    l.len() - l.trim_start_matches(' ').len()
}

/// Wrap `text`'s words to `width` columns with `indent` spaces before each line.
fn wrap_into(out: &mut Vec<String>, text: &str, indent: usize, width: usize) {
    let indent = indent.min(width.saturating_sub(12));
    let room = width.saturating_sub(indent).max(1);
    let pad = " ".repeat(indent);
    let mut line = String::new();
    for w in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + w.len() > room {
            out.push(format!("{pad}{line}"));
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(w);
        // A word longer than the room is cut.
        while line.len() > room {
            let rest = line.split_off(room);
            out.push(format!("{pad}{line}"));
            line = rest;
        }
    }
    if !line.is_empty() {
        out.push(format!("{pad}{line}"));
    }
}

/// Render a page for a terminal `width` columns wide: headings bold (if
/// `ansi`), paragraphs (indent 4 or 8) re-wrapped, SYNOPSIS lines and other
/// lines as written, cut into rows that fit.
pub fn render(text: &str, width: usize, ansi: bool) -> Vec<String> {
    let width = width.max(16);
    let mut out: Vec<String> = Vec::new();
    let mut para = String::new();
    let mut para_indent = 0;
    let mut synopsis = false;
    let flush = |out: &mut Vec<String>, para: &mut String, indent: usize| {
        if !para.is_empty() {
            wrap_into(out, para, indent, width);
            para.clear();
        }
    };
    for raw in text.lines() {
        let l = raw.trim_end();
        if l.is_empty() {
            flush(&mut out, &mut para, para_indent);
            if out.last().is_some_and(|x| !x.is_empty()) {
                out.push(String::new());
            }
            continue;
        }
        if is_heading(l) {
            flush(&mut out, &mut para, para_indent);
            synopsis = l == "SYNOPSIS";
            out.push(if ansi { format!("\x1b[1m{l}\x1b[0m") } else { l.to_string() });
            continue;
        }
        let ind = indent_of(l);
        if (ind == 4 || ind == 8) && !synopsis {
            if !para.is_empty() && ind != para_indent {
                flush(&mut out, &mut para, para_indent);
            }
            para_indent = ind;
            if !para.is_empty() {
                para.push(' ');
            }
            para.push_str(l.trim());
            continue;
        }
        flush(&mut out, &mut para, para_indent);
        if synopsis {
            // Command lines: wrapped, the continuation indented further.
            let mut rows = Vec::new();
            wrap_into(&mut rows, l.trim(), ind.min(4), width.saturating_sub(4));
            for (i, r) in rows.into_iter().enumerate() {
                out.push(if i == 0 { r } else { format!("    {r}") });
            }
            continue;
        }
        let mut s = l.to_string();
        while s.len() > width {
            let rest = s.split_off(width);
            out.push(s);
            s = format!("{}{}", " ".repeat(ind.min(8)), rest.trim_start());
        }
        out.push(s);
    }
    flush(&mut out, &mut para, para_indent);
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}

/// Text without ANSI escape sequences.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            if it.peek() == Some(&'[') {
                it.next();
                for c in it.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Pages whose name or summary contains `word` (case-insensitive), as
/// `(name, summary)`; `pages` is every page available.
pub fn apropos<'a>(word: &str, pages: impl Iterator<Item = (String, &'a str)>) -> Vec<(String, String)> {
    let w = word.to_ascii_lowercase();
    let mut out: Vec<(String, String)> = pages
        .filter_map(|(n, t)| {
            let s = summary(t).unwrap_or_default();
            (n.to_ascii_lowercase().contains(&w) || s.to_ascii_lowercase().contains(&w)).then_some((n, s))
        })
        .collect();
    out.sort();
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn bold(s: &str, ansi: bool) -> String {
    if ansi {
        format!("\x1b[1m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// `help` with no topic: every page name under its heading (`extra`: pages
/// only this computer has).
pub fn help_index(width: usize, ansi: bool, extra: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    wrap_into(&mut out, "Commands and topics by heading. help <heading> (help files, help radio, ...) lists a heading's commands with what each does; help <name> sums one up; man <name> shows its manual page; man -k <word> searches; man keys lists the editing keys.", 0, width);
    let mut list = |title: &str, names: &[&str]| {
        out.push(String::new());
        out.push(bold(&title.to_ascii_uppercase(), ansi));
        wrap_into(&mut out, &names.join("  "), 2, width);
    };
    for (title, names) in CATEGORIES {
        list(title, names);
    }
    if !extra.is_empty() {
        let e: Vec<&str> = extra.iter().map(String::as_str).collect();
        list("On this computer", &e);
    }
    out
}

/// The `help` heading `name` stands for: its title ("Names and the web"),
/// or the title's first word ("names"), any case.
pub fn category(name: &str) -> Option<(&'static str, &'static [&'static str])> {
    let n = name.trim().replace(['-', '_'], " ").to_ascii_lowercase();
    CATEGORIES.iter().copied().find(|(title, _)| {
        let t = title.to_ascii_lowercase();
        t == n || t.split_whitespace().next() == Some(n.as_str())
    })
}

/// `help <heading>`: every page under the heading with its summary.
/// `pages` gives a page's text by name.
pub fn help_category(title: &str, names: &[&str], width: usize, ansi: bool, pages: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let mut out = vec![bold(&title.to_ascii_uppercase(), ansi)];
    let col = names.iter().map(|n| n.len()).max().unwrap_or(0) + 2;
    for n in names {
        let s = pages(n).and_then(|t| summary(&t)).unwrap_or_default();
        if 2 + col + 24 > width {
            // Too narrow for two columns: the summary under the name.
            out.push(format!("  {n}"));
            wrap_into(&mut out, &s, 4, width);
        } else {
            let mut rows = Vec::new();
            wrap_into(&mut rows, &s, 0, width - 2 - col);
            for (i, r) in rows.iter().enumerate() {
                let name = if i == 0 { *n } else { "" };
                out.push(format!("  {name:<col$}{r}"));
            }
            if rows.is_empty() {
                out.push(format!("  {n}"));
            }
        }
    }
    out.push(String::new());
    wrap_into(&mut out, "help <name> sums one up; man <name> has the whole page.", 0, width);
    out
}

/// `help <topic>`: the summary, the synopsis, the first paragraph of the
/// description (or of a topic page's first section) and where to read more.
pub fn help_topic(name: &str, text: &str, width: usize, ansi: bool) -> Vec<String> {
    let mut out = Vec::new();
    out.push(bold(&format!("{} - {}", name, summary(text).unwrap_or_default()), ansi));
    if let Some(syn) = section(text, "SYNOPSIS") {
        out.push(String::new());
        out.push(bold("USAGE", ansi));
        for l in syn.iter().filter(|l| !l.trim().is_empty()) {
            wrap_into(&mut out, l.trim(), 2, width);
        }
    }
    let body = section(text, "DESCRIPTION").or_else(|| {
        let first = text.lines().find(|l| is_heading(l) && l.trim() != "NAME" && l.trim() != "SYNOPSIS")?;
        section(text, first.trim())
    });
    if let Some(body) = body {
        let para: Vec<&str> = body.iter().skip_while(|l| l.trim().is_empty()).take_while(|l| !l.trim().is_empty()).map(|l| l.trim()).collect();
        if !para.is_empty() {
            out.push(String::new());
            wrap_into(&mut out, &para.join(" "), 0, width);
        }
    }
    out.push(String::new());
    out.push(format!("More: man {name}"));
    out
}

// ---------------------------------------------------------------- pager

/// A key the pager understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(u8),
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Enter,
    Backspace,
    Escape,
    CtrlC,
}

/// Turns terminal input bytes into [`Key`]s (escape sequences may be split;
/// an Esc that arrives alone is the Esc key).
#[derive(Default)]
pub struct KeyReader {
    pending: Vec<u8>,
}

impl KeyReader {
    pub fn feed(&mut self, data: &[u8]) -> Vec<Key> {
        self.pending.extend_from_slice(data);
        let mut keys = Vec::new();
        let mut i = 0;
        let p = self.pending.clone();
        while i < p.len() {
            let b = p[i];
            if b == 0x1b {
                if i + 1 >= p.len() {
                    if data == [0x1b] {
                        keys.push(Key::Escape);
                        i += 1;
                    }
                    break;
                }
                if p[i + 1] == b'[' || p[i + 1] == b'O' {
                    let Some(end) = p[i + 2..].iter().position(|&c| (0x40..=0x7e).contains(&c)) else { break };
                    let seq = &p[i + 2..i + 3 + end];
                    keys.extend(match seq {
                        b"A" => Some(Key::Up),
                        b"B" => Some(Key::Down),
                        b"5~" => Some(Key::PageUp),
                        b"6~" => Some(Key::PageDown),
                        b"H" | b"1~" | b"7~" => Some(Key::Home),
                        b"F" | b"4~" | b"8~" => Some(Key::End),
                        _ => None,
                    });
                    i += 3 + end;
                    continue;
                }
                keys.push(Key::Escape);
                i += 1;
                continue;
            }
            keys.push(match b {
                b'\r' | b'\n' => Key::Enter,
                0x08 | 0x7f => Key::Backspace,
                0x03 => Key::CtrlC,
                _ => Key::Char(b),
            });
            i += 1;
        }
        self.pending.drain(..i);
        keys
    }
}

/// A full-screen pager over rendered lines.
pub struct Pager {
    pub title: String,
    lines: Vec<String>,
    plain: Vec<String>,
    pub top: usize,
    /// The search being typed after `/`.
    typing: Option<String>,
    pattern: String,
    message: String,
    help: bool,
}

/// What the program should do after a key.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Redraw,
    Quit,
}

impl Pager {
    pub fn new(title: &str, lines: Vec<String>) -> Pager {
        let plain = lines.iter().map(|l| strip_ansi(l).to_ascii_lowercase()).collect();
        Pager { title: title.to_string(), lines, plain, top: 0, typing: None, pattern: String::new(), message: String::new(), help: false }
    }

    fn max_top(&self, rows: usize) -> usize {
        self.lines.len().saturating_sub(rows)
    }

    /// Handle a key on a screen of `height` rows.
    pub fn key(&mut self, k: Key, height: usize) -> Action {
        let rows = height.saturating_sub(1).max(1);
        if let Some(q) = &mut self.typing {
            match k {
                Key::Enter => {
                    self.pattern = q.to_ascii_lowercase();
                    self.typing = None;
                    self.find(true, self.top + 1, rows);
                }
                Key::Escape | Key::CtrlC => self.typing = None,
                Key::Backspace => {
                    if q.pop().is_none() {
                        self.typing = None;
                    }
                }
                Key::Char(c) if (32..127).contains(&c) => q.push(c as char),
                _ => {}
            }
            return Action::Redraw;
        }
        self.message.clear();
        if self.help {
            // Any key closes the help screen.
            self.help = false;
            return Action::Redraw;
        }
        let max = self.max_top(rows);
        match k {
            Key::Char(b'q') | Key::Char(b'Q') | Key::CtrlC | Key::Escape => return Action::Quit,
            Key::Char(b' ') | Key::Char(b'f') | Key::PageDown => self.top = (self.top + rows).min(max),
            Key::Char(b'b') | Key::PageUp => self.top = self.top.saturating_sub(rows),
            Key::Char(b'd') => self.top = (self.top + rows / 2).min(max),
            Key::Char(b'u') => self.top = self.top.saturating_sub(rows / 2),
            Key::Enter | Key::Char(b'j') | Key::Char(b'e') | Key::Down => self.top = (self.top + 1).min(max),
            Key::Char(b'k') | Key::Char(b'y') | Key::Up => self.top = self.top.saturating_sub(1),
            Key::Char(b'g') | Key::Char(b'<') | Key::Home => self.top = 0,
            Key::Char(b'G') | Key::Char(b'>') | Key::End => self.top = max,
            Key::Char(b'/') => self.typing = Some(String::new()),
            Key::Char(b'n') => self.find(true, self.top + 1, rows),
            Key::Char(b'N') => self.find(false, self.top, rows),
            Key::Char(b'h') | Key::Char(b'?') => self.help = true,
            _ => {}
        }
        Action::Redraw
    }

    fn find(&mut self, forward: bool, from: usize, rows: usize) {
        if self.pattern.is_empty() {
            return;
        }
        let hit = if forward {
            (from..self.plain.len()).find(|&i| self.plain[i].contains(&self.pattern))
        } else {
            (0..from.min(self.plain.len())).rev().find(|&i| self.plain[i].contains(&self.pattern))
        };
        match hit {
            Some(i) => self.top = i.min(self.max_top(rows)),
            None => self.message = format!("Pattern not found: {}", self.pattern),
        }
    }

    /// The whole screen (`width` x `height`) as terminal output.
    pub fn draw(&self, width: usize, height: usize) -> String {
        let rows = height.saturating_sub(1).max(1);
        let mut s = String::from("\x1b[H\x1b[2J");
        if self.help {
            for l in PAGER_HELP.lines().take(rows) {
                let _ = write!(s, "{}\r\n", &l[..l.len().min(width)]);
            }
        } else {
            for i in self.top..(self.top + rows).min(self.lines.len()) {
                if !self.pattern.is_empty() && self.plain[i].contains(&self.pattern) {
                    let _ = write!(s, "\x1b[7m{}\x1b[0m\r\n", strip_ansi(&self.lines[i]));
                } else {
                    let _ = write!(s, "{}\r\n", self.lines[i]);
                }
            }
        }
        let _ = write!(s, "\x1b[{};1H\x1b[7m", height.max(1));
        let status = if let Some(q) = &self.typing {
            format!("/{q}")
        } else if !self.message.is_empty() {
            self.message.clone()
        } else {
            let end = (self.top + rows).min(self.lines.len());
            let tail = if end >= self.lines.len() { "(END)".to_string() } else { format!("{}%", end * 100 / self.lines.len().max(1)) };
            format!(" {} {}-{}/{} {}  q quit  / find  h keys", self.title, self.top + 1, end, self.lines.len(), tail)
        };
        let status: String = status.chars().take(width.saturating_sub(1)).collect();
        let _ = write!(s, "{status}\x1b[0m\x1b[K");
        s
    }
}

/// The pager's own help screen (h).
pub const PAGER_HELP: &str = "\
Pager keys (any key returns)
  q  Esc  Ctrl+C       quit
  Space  f  PgDn       next page
  b  PgUp              previous page
  Enter  j  Down       one line down
  k  y  Up             one line up
  d / u                half a page down / up
  g  Home / G  End     top / bottom
  /text Enter          find text (matching lines are highlighted)
  n / N                next / previous match";

pub mod tty;

#[cfg(test)]
mod tests;
