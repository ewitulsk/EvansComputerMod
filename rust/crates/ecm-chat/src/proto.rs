//! The chat line protocol.
//!
//! Every message is one UTF-8 line ending in `\n` (a preceding `\r` is ignored).
//!
//! Client to server:
//!
//! | Line | Meaning |
//! |---|---|
//! | `NICK <name>` | Join with (or change to) a nickname |
//! | `MSG <text>` | Say something to everyone |
//! | `WHO` | List who is connected |
//! | `QUIT` | Leave |
//!
//! Server to client (`<t>` is the server's `HH:MM:SS` UTC time):
//!
//! | Line | Meaning |
//! |---|---|
//! | `WELCOME <nick> <room>` | You joined as `<nick>` (may differ from what you asked for) |
//! | `HIST <t> <nick> <text>` | A recent message, replayed when you join |
//! | `MSG <t> <nick> <text>` | A message (your own come back too) |
//! | `JOIN <t> <nick>` / `LEAVE <t> <nick> <reason>` | Someone arrived / left |
//! | `NICK <t> <old> <new>` | Someone changed nickname |
//! | `WHO <nick> <nick> ...` | Answer to `WHO` |
//! | `ERR <text>` | Your last request was refused |

/// TCP port `chatd` listens on by default.
pub const DEFAULT_PORT: u16 = 7777;
/// Longest accepted line (bytes, without the newline).
pub const MAX_LINE: usize = 400;
/// Longest nickname.
pub const MAX_NICK: usize = 16;

/// A request from a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Nick(String),
    Msg(String),
    Who,
    Quit,
}

/// Parse one client line (without its newline).
pub fn parse_request(line: &str) -> Result<Request, String> {
    let line = line.trim_end_matches('\r');
    let (word, rest) = match line.find(' ') {
        Some(i) => (&line[..i], &line[i + 1..]),
        None => (line, ""),
    };
    match word.to_ascii_uppercase().as_str() {
        "NICK" => {
            let nick = rest.trim();
            if valid_nick(nick) {
                Ok(Request::Nick(nick.to_string()))
            } else {
                Err(format!(
                    "invalid nickname '{}': 1-{} letters, digits, '-', '_' or '.'",
                    clean_text(nick),
                    MAX_NICK
                ))
            }
        }
        "MSG" => {
            let text = clean_text(rest);
            if text.trim().is_empty() {
                Err("empty message".into())
            } else {
                Ok(Request::Msg(text))
            }
        }
        "WHO" => Ok(Request::Who),
        "QUIT" => Ok(Request::Quit),
        "" => Err("empty line".into()),
        other => Err(format!("unknown command {}", clean_text(other))),
    }
}

/// An event sent by the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Welcome { nick: String, room: String },
    History { time: String, nick: String, text: String },
    Msg { time: String, nick: String, text: String },
    Join { time: String, nick: String },
    Leave { time: String, nick: String, reason: String },
    Nick { time: String, old: String, new: String },
    Who(Vec<String>),
    Err(String),
}

impl Event {
    /// The wire line (without newline).
    pub fn line(&self) -> String {
        match self {
            Event::Welcome { nick, room } => format!("WELCOME {} {}", nick, room),
            Event::History { time, nick, text } => format!("HIST {} {} {}", time, nick, text),
            Event::Msg { time, nick, text } => format!("MSG {} {} {}", time, nick, text),
            Event::Join { time, nick } => format!("JOIN {} {}", time, nick),
            Event::Leave { time, nick, reason } => format!("LEAVE {} {} {}", time, nick, reason),
            Event::Nick { time, old, new } => format!("NICK {} {} {}", time, old, new),
            Event::Who(nicks) => format!("WHO {}", nicks.join(" ")).trim_end().to_string(),
            Event::Err(text) => format!("ERR {}", text),
        }
    }

    /// Parse a server line; None for anything unrecognised.
    pub fn parse(line: &str) -> Option<Event> {
        let line = line.trim_end_matches('\r');
        let mut it = line.splitn(2, ' ');
        let word = it.next()?;
        let rest = it.next().unwrap_or("");
        let words = |n: usize| -> Option<Vec<String>> {
            let parts: Vec<&str> = rest.splitn(n, ' ').collect();
            if parts.len() < n - 1 {
                return None;
            }
            Some(parts.into_iter().map(str::to_string).collect())
        };
        Some(match word {
            "WELCOME" => {
                let p = words(2)?;
                Event::Welcome { nick: p[0].clone(), room: p.get(1).cloned().unwrap_or_default() }
            }
            "HIST" | "MSG" => {
                let p = words(3)?;
                let (time, nick, text) = (p[0].clone(), p.get(1)?.clone(), p.get(2).cloned().unwrap_or_default());
                if word == "HIST" {
                    Event::History { time, nick, text }
                } else {
                    Event::Msg { time, nick, text }
                }
            }
            "JOIN" => {
                let p = words(2)?;
                Event::Join { time: p[0].clone(), nick: p.get(1)?.clone() }
            }
            "LEAVE" => {
                let p = words(3)?;
                Event::Leave { time: p[0].clone(), nick: p.get(1)?.clone(), reason: p.get(2).cloned().unwrap_or_default() }
            }
            "NICK" => {
                let p = words(3)?;
                Event::Nick { time: p[0].clone(), old: p.get(1)?.clone(), new: p.get(2)?.clone() }
            }
            "WHO" => Event::Who(rest.split_whitespace().map(str::to_string).collect()),
            "ERR" => Event::Err(rest.to_string()),
            _ => return None,
        })
    }
}

/// 1-16 of `[A-Za-z0-9_.-]`.
pub fn valid_nick(nick: &str) -> bool {
    !nick.is_empty()
        && nick.len() <= MAX_NICK
        && nick.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// Drop control characters (so nobody can move another user's cursor) and cap the length.
pub fn clean_text(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_control() {
            continue;
        }
        if out.len() + c.len_utf8() > MAX_LINE - 32 {
            break;
        }
        out.push(c);
    }
    out
}

/// `HH:MM:SS` (UTC) of a Unix time in seconds.
pub fn hms(unix_secs: u64) -> String {
    let s = unix_secs % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

/// Split a byte stream into complete lines; keeps the unfinished tail in `buf`.
/// A line longer than [`MAX_LINE`] is cut there (the rest of it is dropped).
pub fn take_lines(buf: &mut Vec<u8>, data: &[u8]) -> Vec<String> {
    let mut lines = Vec::new();
    for &b in data {
        if b == b'\n' {
            let line = String::from_utf8_lossy(buf).trim_end_matches('\r').to_string();
            lines.push(line);
            buf.clear();
        } else if buf.len() < MAX_LINE {
            buf.push(b);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_parse_and_reject() {
        assert_eq!(parse_request("NICK alice"), Ok(Request::Nick("alice".into())));
        assert_eq!(parse_request("nick bob\r"), Ok(Request::Nick("bob".into())));
        assert_eq!(parse_request("MSG hello there"), Ok(Request::Msg("hello there".into())));
        assert_eq!(parse_request("WHO"), Ok(Request::Who));
        assert_eq!(parse_request("QUIT"), Ok(Request::Quit));
        assert!(parse_request("NICK bad nick").is_err());
        assert!(parse_request("NICK way-too-long-nickname-here").is_err());
        assert!(parse_request("MSG    ").is_err());
        assert!(parse_request("FOO").is_err());
        // Control characters are stripped from messages.
        assert_eq!(parse_request("MSG hi\x1b[2Jthere"), Ok(Request::Msg("hi[2Jthere".into())));
    }

    #[test]
    fn events_round_trip() {
        let events = vec![
            Event::Welcome { nick: "alice".into(), room: "Tech Village chat".into() },
            Event::History { time: "01:02:03".into(), nick: "bob".into(), text: "earlier words".into() },
            Event::Msg { time: "12:00:00".into(), nick: "alice".into(), text: "hello world".into() },
            Event::Join { time: "12:00:01".into(), nick: "carol".into() },
            Event::Leave { time: "12:00:02".into(), nick: "carol".into(), reason: "quit".into() },
            Event::Nick { time: "12:00:03".into(), old: "bob".into(), new: "robert".into() },
            Event::Who(vec!["alice".into(), "robert".into()]),
            Event::Err("nickname alice is taken".into()),
        ];
        for e in events {
            assert_eq!(Event::parse(&e.line()), Some(e.clone()), "{}", e.line());
        }
        assert_eq!(Event::parse("BOGUS x"), None);
        assert_eq!(Event::parse("MSG 12:00:00"), None, "a message needs a nick");
    }

    #[test]
    fn line_splitting_keeps_partial_tail_and_caps_length() {
        let mut buf = Vec::new();
        assert_eq!(take_lines(&mut buf, b"NICK a"), Vec::<String>::new());
        assert_eq!(take_lines(&mut buf, b"lice\r\nMSG hi\nWH"), vec!["NICK alice", "MSG hi"]);
        assert_eq!(buf, b"WH");
        let long = vec![b'x'; MAX_LINE * 3];
        let mut buf = Vec::new();
        let lines = take_lines(&mut buf, &[&long[..], b"\n"].concat());
        assert_eq!(lines[0].len(), MAX_LINE);
    }

    #[test]
    fn time_format() {
        assert_eq!(hms(0), "00:00:00");
        assert_eq!(hms(86_400 + 3600 * 13 + 60 * 5 + 9), "13:05:09");
    }
}
