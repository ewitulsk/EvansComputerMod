//! The `chat` client's logic: arguments and `/etc/chat.conf`, the line editor that keeps
//! what you are typing intact while messages arrive, how events are shown, and the
//! fix-it hints printed when the server cannot be reached.

use crate::proto::{self, Event, DEFAULT_PORT};

pub const USAGE: &str = "Usage: chat [server[:port]] [--nick NAME]\n\
  With no server, uses the one in /etc/chat.conf (Tech Village computers have it).\n\
  While connected: type a line and press Enter to send it.\n\
  /nick NAME  change nickname    /who  list users\n\
  /help       this help          /quit leave (Ctrl+T also quits)";

/// Command-line options.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Args {
    pub server: Option<String>,
    pub port: Option<u16>,
    pub nick: Option<String>,
    pub help: bool,
}

pub fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "-h" | "--help" => a.help = true,
            "--nick" | "-n" => {
                i += 1;
                let nick = args.get(i).ok_or("--nick needs a name")?;
                if !proto::valid_nick(nick) {
                    return Err(format!("invalid nickname '{}': 1-{} letters, digits, '-', '_' or '.'", nick, proto::MAX_NICK));
                }
                a.nick = Some(nick.clone());
            }
            "--port" | "-p" => {
                i += 1;
                let p = args.get(i).ok_or("--port needs a number")?;
                a.port = Some(p.parse().map_err(|_| format!("bad port '{}'", p))?);
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option {}", arg)),
            _ => {
                if a.server.is_some() {
                    return Err(format!("unexpected argument {}", arg));
                }
                let (host, port) = split_host_port(arg)?;
                a.server = Some(host);
                if port.is_some() {
                    a.port = port;
                }
            }
        }
        i += 1;
    }
    Ok(a)
}

/// `host` or `host:port`.
pub fn split_host_port(s: &str) -> Result<(String, Option<u16>), String> {
    match s.rfind(':') {
        Some(i) => {
            let port = s[i + 1..].parse().map_err(|_| format!("bad port in '{}'", s))?;
            Ok((s[..i].to_string(), Some(port)))
        }
        None => Ok((s.to_string(), None)),
    }
}

/// `/etc/chat.conf`: `server ADDRESS`, `port N`, `nick NAME` (or `key=value`); `#` comments.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Config {
    pub server: Option<String>,
    pub port: Option<u16>,
    pub nick: Option<String>,
}

pub fn parse_config(text: &str) -> Config {
    let mut c = Config::default();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = match line.find(|ch: char| ch == '=' || ch.is_whitespace()) {
            Some(i) => (line[..i].trim(), line[i + 1..].trim().trim_start_matches('=').trim()),
            None => continue,
        };
        match key.to_ascii_lowercase().as_str() {
            "server" => {
                if let Ok((host, port)) = split_host_port(value) {
                    c.server = Some(host);
                    if port.is_some() {
                        c.port = port;
                    }
                }
            }
            "port" => c.port = value.parse().ok(),
            "nick" if proto::valid_nick(value) => c.nick = Some(value.to_string()),
            _ => {}
        }
    }
    c
}

/// Where to connect and as whom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub nick: String,
}

/// Arguments win over the config file; the nick falls back to `fallback_nick`.
pub fn resolve(args: &Args, config: &Config, fallback_nick: &str) -> Result<Target, Problem> {
    let host = args.server.clone().or_else(|| config.server.clone()).ok_or(Problem::NoServer)?;
    let port = args.port.or(config.port).unwrap_or(DEFAULT_PORT);
    let nick = args.nick.clone().or_else(|| config.nick.clone()).unwrap_or_else(|| fallback_nick.to_string());
    Ok(Target { host, port, nick })
}

/// A default nickname from a MAC address: `pc-` and its last two bytes.
pub fn nick_from_mac(mac: &str) -> String {
    let hex: String = mac.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() >= 4 {
        format!("pc-{}", &hex[hex.len() - 4..])
    } else {
        "guest".into()
    }
}

/// Why the chat could not start or ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// No server given and none in /etc/chat.conf.
    NoServer,
    /// The host name did not resolve.
    Unresolved(String),
    /// This computer has no IPv4 address.
    NoAddress,
    /// No route (not even a default) covers the server address.
    NoRoute(String),
    /// The server answered with a reset: reachable, but nothing listens on the port.
    Refused(String),
    /// No answer at all within the connect timeout.
    Timeout(String),
    /// The server closed the connection.
    Lost(String),
}

/// What went wrong and how to fix it (several lines).
pub fn hint(p: &Problem) -> String {
    match p {
        Problem::NoServer => "chat: no server given and /etc/chat.conf names none.\n\
  Fix: chat SERVER[:PORT], or put a line 'server ADDRESS' in /etc/chat.conf\n\
  (on a Tech Village computer: cat /etc/chat.conf; the village website names the chat server)."
            .into(),
        Problem::Unresolved(h) => format!(
            "chat: cannot resolve '{}'.\n  Fix: use the server's IPv4 address (e.g. 100.65.0.20), or check DNS with nslookup {}.",
            h, h
        ),
        Problem::NoAddress => "chat: this computer has no IPv4 address.\n\
  Fix: cable a network face to the village cable (or your home router's LAN) and put\n\
  'iface ethN dhcp' in network.cfg for that face, then restart; check with ifconfig."
            .into(),
        Problem::NoRoute(ip) => format!(
            "chat: no route to {}.\n  Fix: the computer needs a default route (DHCP gives one): run ip route;\n  with a static address add 'route default via GATEWAY dev ethN' to network.cfg.",
            ip
        ),
        Problem::Refused(addr) => format!(
            "chat: connection to {} refused: the computer answered but no chat server listens there.\n  Fix: on the chat server run 'chatd &' (it is in its services.cfg), or check the port.",
            addr
        ),
        Problem::Timeout(addr) => format!(
            "chat: no answer from {} (timed out).\n  Fix: ping and traceroute it; on your ISP router check 'show bgp ipv4 unicast summary'\n  and 'show ip route' (a cut fiber or cable takes the long way round the ring, or none).",
            addr
        ),
        Problem::Lost(addr) => format!(
            "chat: connection to {} closed. Run chat again to reconnect.",
            addr
        ),
    }
}

/// Does a route in `routes` ((destination, prefix length)) cover `ip`?
pub fn routed(routes: &[([u8; 4], u8)], ip: [u8; 4]) -> bool {
    let ip = u32::from_be_bytes(ip);
    routes.iter().any(|(dest, len)| {
        let len = (*len).min(32);
        let mask = if len == 0 { 0 } else { u32::MAX << (32 - len) };
        (u32::from_be_bytes(*dest) & mask) == (ip & mask)
    })
}

pub fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = s.trim().split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut ip = [0u8; 4];
    for (o, p) in ip.iter_mut().zip(parts) {
        *o = p.parse().ok()?;
    }
    Some(ip)
}

/// A string or number field of a flat JSON object (as the network config API returns).
pub fn json_field<'a>(obj: &'a str, key: &str) -> Option<&'a str> {
    let at = obj.find(&format!("\"{}\":", key))? + key.len() + 3;
    let rest = obj[at..].trim_start();
    if let Some(s) = rest.strip_prefix('"') {
        Some(&s[..s.find('"')?])
    } else {
        let end = rest.find(|c: char| c == ',' || c == '}').unwrap_or(rest.len());
        Some(rest[..end].trim())
    }
}

/// Routes from the JSON array `[{"dest":..,"prefix_len":..},...]`.
pub fn parse_routes(json: &str) -> Vec<([u8; 4], u8)> {
    json.split('{')
        .skip(1)
        .filter_map(|o| {
            let dest = parse_ipv4(json_field(o, "dest")?)?;
            let len = json_field(o, "prefix_len")?.parse().ok()?;
            Some((dest, len))
        })
        .collect()
}

/// What a keystroke did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// Bytes to print (the echo, or a backspace).
    Echo(String),
    /// Enter: the finished line (the editor is empty again).
    Line(String),
    /// Ctrl+C / Ctrl+D on an empty line.
    Quit,
    Nothing,
}

/// One line of input with its own echo (programs get raw keys; the terminal does not echo).
#[derive(Debug, Default)]
pub struct Editor {
    buf: String,
    escape: u8,
    last_cr: bool,
}

pub const PROMPT: &str = "> ";

impl Editor {
    pub fn text(&self) -> &str {
        &self.buf
    }

    pub fn key(&mut self, b: u8) -> Key {
        // Swallow ANSI escape sequences (arrow keys etc.): ESC [ ... final byte.
        if self.escape == 1 {
            self.escape = if b == b'[' || b == b'O' { 2 } else { 0 };
            return Key::Nothing;
        }
        if self.escape == 2 {
            if (0x40..=0x7e).contains(&b) {
                self.escape = 0;
            }
            return Key::Nothing;
        }
        let after_cr = std::mem::replace(&mut self.last_cr, false);
        match b {
            0x1b => {
                self.escape = 1;
                Key::Nothing
            }
            b'\n' if after_cr => Key::Nothing,
            b'\r' | b'\n' => {
                self.last_cr = b == b'\r';
                Key::Line(std::mem::take(&mut self.buf))
            }
            3 => Key::Quit,
            4 if self.buf.is_empty() => Key::Quit,
            8 | 127 => {
                if self.buf.pop().is_some() {
                    Key::Echo("\x08 \x08".into())
                } else {
                    Key::Nothing
                }
            }
            b if (32..127).contains(&b) && self.buf.len() < proto::MAX_LINE - 40 => {
                self.buf.push(b as char);
                Key::Echo((b as char).to_string())
            }
            _ => Key::Nothing,
        }
    }

    /// Print `text` above the line being typed: clear the line, print, redraw prompt + input.
    pub fn above(&self, text: &str) -> String {
        let mut s = String::from("\r\x1b[K");
        for line in text.lines() {
            s.push_str(line);
            s.push('\n');
        }
        s.push_str(PROMPT);
        s.push_str(&self.buf);
        s
    }
}

/// What a typed line means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typed {
    /// Send this protocol line.
    Send(String),
    /// Show this locally.
    Show(String),
    Quit,
    Nothing,
}

pub fn typed(line: &str) -> Typed {
    let line = line.trim();
    if line.is_empty() {
        return Typed::Nothing;
    }
    if let Some(cmd) = line.strip_prefix('/') {
        let (word, rest) = match cmd.find(' ') {
            Some(i) => (&cmd[..i], cmd[i + 1..].trim()),
            None => (cmd, ""),
        };
        return match word.to_ascii_lowercase().as_str() {
            "quit" | "exit" | "q" => Typed::Quit,
            "who" | "names" => Typed::Send("WHO".into()),
            "nick" if proto::valid_nick(rest) => Typed::Send(format!("NICK {}", rest)),
            "nick" => Typed::Show(format!("usage: /nick NAME (1-{} of A-Z a-z 0-9 - _ .)", proto::MAX_NICK)),
            "help" | "?" => Typed::Show(USAGE.into()),
            _ => Typed::Show(format!("unknown command /{} (try /help)", word)),
        };
    }
    Typed::Send(format!("MSG {}", proto::clean_text(line)))
}

/// How a server event is shown to `me`; None for events that print nothing.
pub fn show(e: &Event, me: &str) -> Option<String> {
    Some(match e {
        Event::Welcome { nick, room } => format!("*** Joined {} as {} (/help for commands)", room, nick),
        Event::History { time, nick, text } => format!("[{}] <{}> {}  (earlier)", time, nick, text),
        Event::Msg { time, nick, text } => format!("[{}] <{}> {}", time, nick, text),
        Event::Join { time, nick } if nick == me => format!("[{}] * you joined", time),
        Event::Join { time, nick } => format!("[{}] * {} joined", time, nick),
        Event::Leave { time, nick, reason } => format!("[{}] * {} left ({})", time, nick, reason),
        Event::Nick { time, old, new } => format!("[{}] * {} is now {}", time, old, new),
        Event::Who(n) => format!("*** {} online: {}", n.len(), n.join(", ")),
        Event::Err(t) => format!("*** server: {}", t),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn arguments_and_config_resolve() {
        let a = parse_args(&s(&["100.65.0.20:9000", "--nick", "alice"])).unwrap();
        assert_eq!(a.server.as_deref(), Some("100.65.0.20"));
        assert_eq!(a.port, Some(9000));
        assert_eq!(a.nick.as_deref(), Some("alice"));
        assert!(parse_args(&s(&["--nick", "bad name"])).is_err());
        assert!(parse_args(&s(&["--bogus"])).is_err());
        assert!(parse_args(&s(&["a", "b"])).is_err());
        assert!(parse_args(&s(&["-h"])).unwrap().help);

        let conf = parse_config("# Tech Village chat\nserver 100.68.0.20\nport=7777\nnick v3-house1\n");
        assert_eq!(conf, Config { server: Some("100.68.0.20".into()), port: Some(7777), nick: Some("v3-house1".into()) });
        let t = resolve(&Args::default(), &conf, "pc-0001").unwrap();
        assert_eq!(t, Target { host: "100.68.0.20".into(), port: 7777, nick: "v3-house1".into() });
        // Arguments win; defaults fill in.
        let t = resolve(&parse_args(&s(&["10.0.0.5"])).unwrap(), &Config::default(), "pc-0001").unwrap();
        assert_eq!(t, Target { host: "10.0.0.5".into(), port: DEFAULT_PORT, nick: "pc-0001".into() });
        assert_eq!(resolve(&Args::default(), &Config::default(), "x"), Err(Problem::NoServer));
        assert_eq!(nick_from_mac("02:00:a1:b2:c3:d4"), "pc-c3d4");
    }

    #[test]
    fn routes_cover_addresses() {
        let routes = parse_routes(
            r#"[{"dest":"100.65.1.0","prefix_len":24,"gateway":"0.0.0.0","iface_idx":1},{"dest":"0.0.0.0","prefix_len":0,"gateway":"100.65.1.1","iface_idx":1}]"#,
        );
        assert_eq!(routes, vec![([100, 65, 1, 0], 24), ([0, 0, 0, 0], 0)]);
        assert!(routed(&routes, [100, 70, 0, 20]), "the default route covers everything");
        assert!(!routed(&routes[..1], [100, 70, 0, 20]), "control: only the LAN prefix");
        assert!(routed(&routes[..1], [100, 65, 1, 7]));
        assert_eq!(json_field(r#"{"name":"eth1","ip":"100.65.1.10","prefix_len":24}"#, "ip"), Some("100.65.1.10"));
        assert_eq!(json_field(r#"{"prefix_len":24}"#, "prefix_len"), Some("24"));
    }

    #[test]
    fn editor_echoes_edits_and_keeps_the_line_while_messages_arrive() {
        let mut e = Editor::default();
        for b in b"helo" {
            e.key(*b);
        }
        assert_eq!(e.key(8), Key::Echo("\x08 \x08".into()));
        e.key(b'l');
        e.key(b'o');
        // Arrow keys are ignored.
        for b in b"\x1b[D" {
            assert_eq!(e.key(*b), Key::Nothing);
        }
        assert_eq!(e.text(), "hello");
        let shown = e.above("[12:00:00] <bob> hi");
        assert_eq!(shown, "\r\x1b[K[12:00:00] <bob> hi\n> hello");
        assert_eq!(e.key(b'\r'), Key::Line("hello".into()));
        assert_eq!(e.key(b'\n'), Key::Nothing, "CR LF is one Enter");
        assert_eq!(e.key(b'\n'), Key::Line(String::new()));
        assert_eq!(e.key(4), Key::Quit, "Ctrl+D on an empty line");
        assert_eq!(e.key(3), Key::Quit);
        assert_eq!(e.key(7), Key::Nothing);
    }

    #[test]
    fn typed_lines_become_requests() {
        assert_eq!(typed("hello all"), Typed::Send("MSG hello all".into()));
        assert_eq!(typed("/who"), Typed::Send("WHO".into()));
        assert_eq!(typed("/nick carol"), Typed::Send("NICK carol".into()));
        assert!(matches!(typed("/nick two words"), Typed::Show(_)));
        assert_eq!(typed("/quit"), Typed::Quit);
        assert!(matches!(typed("/frobnicate"), Typed::Show(t) if t.contains("/help")));
        assert_eq!(typed("   "), Typed::Nothing);
    }

    #[test]
    fn events_display_and_hints_explain() {
        let msg = Event::Msg { time: "10:00:00".into(), nick: "bob".into(), text: "hi".into() };
        assert_eq!(show(&msg, "alice").unwrap(), "[10:00:00] <bob> hi");
        let join = Event::Join { time: "10:00:01".into(), nick: "alice".into() };
        assert_eq!(show(&join, "alice").unwrap(), "[10:00:01] * you joined");
        assert_eq!(show(&join, "bob").unwrap(), "[10:00:01] * alice joined");
        assert!(hint(&Problem::NoServer).contains("/etc/chat.conf"));
        assert!(hint(&Problem::NoAddress).contains("dhcp"));
        assert!(hint(&Problem::NoRoute("100.70.0.20".into())).contains("route default"));
        assert!(hint(&Problem::Refused("1.2.3.4:7777".into())).contains("chatd"));
        assert!(hint(&Problem::Timeout("1.2.3.4:7777".into())).contains("traceroute"));
    }
}
