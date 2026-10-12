//! The admin socket of `celld-site` and `cored`, and their config files.
//!
//! **Admin socket.** Each daemon listens on TCP on the loopback address only
//! (`127.0.0.1`; `celld-site` port 7710, `cored` port 7711), so only programs on
//! the same computer reach it: the daemon's own CLI (`celld-site status`,
//! `cored subs`, ...) and its app (Cell Site, Carrier Console). The protocol is
//! line-based text:
//!
//! ```text
//! request:  one command line, '\n'-terminated     status
//! reply:    "ok" or "err <message>"               ok
//!           body lines                            site site-a
//!           "." alone ends the reply              ...
//!                                                 .
//! ```
//! A body line that starts with `.` is sent with one more `.` in front (SMTP's
//! dot-stuffing), so `.` alone always ends a reply. A connection can carry any
//! number of requests, answered in order. Bodies are `key value...` lines that
//! read well in a terminal and parse with [`fields`]; each daemon documents its
//! commands.
//!
//! **Config files** (`/etc/celld-site.toml`, `/etc/cored.toml`) are `key = value`
//! lines (a subset of TOML: strings in double quotes or bare words, numbers,
//! `#` comments). [`parse_conf`] is strict: an unknown key or a bad value is an
//! error naming the line.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Encode a successful reply.
pub fn reply_ok(body: &str) -> String {
    let mut s = String::from("ok\n");
    for l in body.lines() {
        if l.starts_with('.') {
            s.push('.');
        }
        s.push_str(l);
        s.push('\n');
    }
    s.push_str(".\n");
    s
}

/// Encode an error reply.
pub fn reply_err(msg: &str) -> String {
    format!("err {}\n.\n", msg.replace('\n', " "))
}

/// Encode either.
pub fn reply(r: &Result<String, String>) -> String {
    match r {
        Ok(b) => reply_ok(b),
        Err(e) => reply_err(e),
    }
}

/// Splits a byte stream into lines (requests on the daemon side).
#[derive(Default, Debug)]
pub struct Lines {
    buf: Vec<u8>,
}

impl Lines {
    pub const MAX_LINE: usize = 4096;

    /// Add bytes; returns the complete lines (without `\r\n`). A line longer than
    /// [`Lines::MAX_LINE`] is an error (the peer is not speaking the protocol).
    pub fn push(&mut self, b: &[u8]) -> Result<Vec<String>, String> {
        self.buf.extend_from_slice(b);
        let mut out = Vec::new();
        while let Some(i) = self.buf.iter().position(|&c| c == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            let s = String::from_utf8_lossy(&line[..line.len() - 1]).trim_end_matches('\r').to_string();
            out.push(s);
        }
        if self.buf.len() > Self::MAX_LINE {
            return Err("line too long".into());
        }
        Ok(out)
    }
}

/// Reassembles replies on the client side.
#[derive(Default, Debug)]
pub struct Replies {
    lines: Lines,
    cur: Option<(bool, String, Vec<String>)>,
}

impl Replies {
    /// Add bytes; returns complete replies: `Ok(body lines)` or `Err(message)`.
    pub fn push(&mut self, b: &[u8]) -> Result<Vec<Result<Vec<String>, String>>, String> {
        let mut out = Vec::new();
        for l in self.lines.push(b)? {
            match self.cur.as_mut() {
                None => {
                    if l == "ok" {
                        self.cur = Some((true, String::new(), Vec::new()));
                    } else if let Some(m) = l.strip_prefix("err ") {
                        self.cur = Some((false, m.to_string(), Vec::new()));
                    } else if l == "err" {
                        self.cur = Some((false, String::new(), Vec::new()));
                    } else {
                        return Err(format!("unexpected reply line '{}'", l));
                    }
                }
                Some((ok, msg, body)) => {
                    if l == "." {
                        out.push(if *ok { Ok(core::mem::take(body)) } else { Err(core::mem::take(msg)) });
                        self.cur = None;
                    } else {
                        body.push(l.strip_prefix('.').map(|x| x.to_string()).unwrap_or(l));
                    }
                }
            }
        }
        Ok(out)
    }
}

/// The words of a body line.
pub fn fields(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

/// The value after `key` in `key value...` lines (the rest of the line).
pub fn value<'a>(body: &'a [String], key: &str) -> Option<&'a str> {
    body.iter().find_map(|l| {
        let (k, v) = l.split_once(' ').unwrap_or((l.as_str(), ""));
        (k == key).then_some(v.trim())
    })
}

/// One `key = value` line of a config file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfLine {
    pub line: usize,
    pub key: String,
    pub value: String,
}

/// Parse `key = value` lines: `#` comments, blank lines, values in double quotes
/// (`\"` and `\\` escapes) or bare (up to a `#`). Errors name the line.
pub fn parse_conf(text: &str) -> Result<Vec<ConfLine>, String> {
    let mut out: Vec<ConfLine> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let (k, v) = t.split_once('=').ok_or_else(|| format!("line {}: expected key = value", line))?;
        let key = k.trim();
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
            return Err(format!("line {}: bad key '{}'", line, key));
        }
        let v = v.trim();
        let value = if let Some(rest) = v.strip_prefix('"') {
            let mut s = String::new();
            let mut chars = rest.chars();
            let mut closed = false;
            while let Some(c) = chars.next() {
                match c {
                    '\\' => match chars.next() {
                        Some('"') => s.push('"'),
                        Some('\\') => s.push('\\'),
                        _ => return Err(format!("line {}: bad escape in string", line)),
                    },
                    '"' => {
                        closed = true;
                        break;
                    }
                    c => s.push(c),
                }
            }
            let tail = chars.as_str().trim();
            if !closed || !(tail.is_empty() || tail.starts_with('#')) {
                return Err(format!("line {}: bad string", line));
            }
            s
        } else {
            v.split('#').next().unwrap_or("").trim().to_string()
        };
        if out.iter().any(|c| c.key == key) {
            return Err(format!("line {}: '{}' twice", line, key));
        }
        out.push(ConfLine { line, key: key.to_string(), value });
    }
    Ok(out)
}

/// A value as a config line writes it: bare when it is a number or a plain word, else quoted.
pub fn conf_value(v: &str) -> String {
    let bare = !v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric() || b".:-_/,".contains(&b));
    if bare {
        v.to_string()
    } else {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn replies_round_trip_in_any_pieces() {
        let wire = alloc::format!("{}{}{}", reply_ok("a 1\n.dot\nb"), reply_err("no such thing"), reply_ok(""));
        for step in [1, 3, 7, 1000] {
            let mut r = Replies::default();
            let mut got = Vec::new();
            for c in wire.as_bytes().chunks(step) {
                got.extend(r.push(c).unwrap());
            }
            assert_eq!(got, vec![Ok(vec!["a 1".to_string(), ".dot".into(), "b".into()]), Err("no such thing".into()), Ok(vec![])]);
        }
        let mut r = Replies::default();
        assert!(r.push(b"hello\n").is_err());
        assert_eq!(value(&["site a b".into(), "link up".into()], "link"), Some("up"));
        assert_eq!(value(&["site a b".into()], "site"), Some("a b"));
    }

    #[test]
    fn requests_are_lines() {
        let mut l = Lines::default();
        assert_eq!(l.push(b"sta").unwrap(), Vec::<String>::new());
        assert_eq!(l.push(b"tus\r\nlog 5\n").unwrap(), vec!["status".to_string(), "log 5".into()]);
        assert!(l.push(&[b'x'; 5000]).is_err());
    }

    #[test]
    fn config_lines() {
        let c = parse_conf("# site\nsite_id = \"site-a\"\ncell_id = 3   # the cell\n\nname = \"a \\\"b\\\" # c\"\ncore = 10.0.1.1\n").unwrap();
        assert_eq!(c.iter().map(|l| (l.key.as_str(), l.value.as_str())).collect::<Vec<_>>(), vec![
            ("site_id", "site-a"),
            ("cell_id", "3"),
            ("name", "a \"b\" # c"),
            ("core", "10.0.1.1")
        ]);
        assert_eq!(c[1].line, 3);
        for (bad, line) in [("x\n", 1), ("a = 1\nA = 2\n", 2), ("a = \"x\n", 1), ("a = 1\na = 2\n", 2), ("a = \"x\" y\n", 1)] {
            let e = parse_conf(bad).unwrap_err();
            assert!(e.starts_with(&alloc::format!("line {}:", line)), "{}: {}", bad, e);
        }
        assert_eq!(conf_value("10.0.1.1"), "10.0.1.1");
        assert_eq!(conf_value("a b"), "\"a b\"");
        assert_eq!(parse_conf(&alloc::format!("k = {}\n", conf_value("q\"\\ x"))).unwrap()[0].value, "q\"\\ x");
    }
}
