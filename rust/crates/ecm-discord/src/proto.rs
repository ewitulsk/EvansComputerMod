//! The Discord line protocol (`DISCORD/1`), spoken between the Discord app and a
//! Discord backend (`discordd`) over TCP (default port [`DEFAULT_PORT`]).
//!
//! Every message is one UTF-8 line ending in `\n`. A connection starts in the
//! old chat protocol (`ecm_chat::proto`: `NICK`, `MSG`, `WHO`, `QUIT`), so the
//! console `chat` command still talks to a backend: it lands in the backend's
//! home server, channel `#general`. A Discord client upgrades the connection
//! with `HELLO <nick>` as its first line.
//!
//! A "backend" is the program (`discordd`); the "servers" a player sees in the
//! app are the backend's guilds (as in Discord's own API). One backend hosts
//! many servers, and the app talks to many backends at once.
//!
//! Client to backend (after `HELLO`):
//!
//! | Line | Meaning |
//! |---|---|
//! | `HELLO <nick>` | Upgrade to Discord, as `<nick>` |
//! | `GCREATE <name>` | Make a server (you own it; it gets `#general` and a `General` voice channel) |
//! | `GJOIN <gid>` / `GLEAVE <gid>` | Join / leave a server |
//! | `GDELETE <gid>` | Delete a server you own |
//! | `CCREATE <gid> text\|voice <name>` | Make a channel in a server you own |
//! | `CDELETE <cid>` | Delete a channel of a server you own |
//! | `HISTORY <cid>` | The channel's recent messages (`H` lines, then `HEND`) |
//! | `SAY <cid> <text>` | Post in a text channel |
//! | `VJOIN <cid>` / `VLEAVE` | Join a voice channel (answered by `VOICE`) / leave voice |
//! | `NICK <name>` | Change nickname |
//! | `PING` / `QUIT` | Keep-alive / leave |
//!
//! Backend to client (`<t>` is the backend's `HH:MM:SS` UTC time):
//!
//! | Line | Meaning |
//! |---|---|
//! | `READY <nick> <backend name>` | Upgraded, as `<nick>` (may differ from what you asked) |
//! | `GUILD <gid> <owner> <0\|1 joined> <members> <name>` | A server exists (or changed) |
//! | `GDEL <gid>` | A server was deleted |
//! | `CHANNEL <gid> <cid> text\|voice <name>` | A channel exists |
//! | `CDEL <cid>` | A channel was deleted |
//! | `MEMBER <gid> <nick> online\|offline\|left` | A member of a server you joined |
//! | `M <cid> <mid> <t> <nick> <text>` | A new message |
//! | `H <cid> <mid> <t> <nick> <text>` / `HEND <cid>` | History, oldest first / its end |
//! | `VOICE <cid> <udp port> <token> <sid>` | Your voice session: send audio to the UDP port with the token |
//! | `VSTATE <cid> <sid> <nick> join\|leave\|speaking\|quiet` | Who is in a voice channel and who talks |
//! | `RENAME <old> <new>` | Someone changed nickname |
//! | `SYNCED` | The first full picture after `READY` is complete |
//! | `PONG` / `ERR <text>` | Keep-alive answer / your last request was refused |
//!
//! Voice travels over UDP (see [`crate::voice`]).

/// TCP port a backend listens on by default (the old chat port, so `/etc/chat.conf` still works).
pub const DEFAULT_PORT: u16 = 7777;
/// Longest server name.
pub const MAX_GUILD_NAME: usize = 32;
/// Longest channel name.
pub const MAX_CHANNEL_NAME: usize = 24;
/// Servers one backend hosts.
pub const MAX_GUILDS: usize = 32;
/// Channels in one server.
pub const MAX_CHANNELS: usize = 16;
/// Messages a channel keeps.
pub const HISTORY: usize = 50;
/// Longest message.
pub const MAX_TEXT: usize = 300;

/// A channel's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Text,
    Voice,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Voice => "voice",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s.to_ascii_lowercase().as_str() {
            "text" => Some(Kind::Text),
            "voice" => Some(Kind::Voice),
            _ => None,
        }
    }
}

/// A request from a Discord client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    Hello(String),
    GuildCreate(String),
    GuildJoin(u32),
    GuildLeave(u32),
    GuildDelete(u32),
    ChannelCreate { guild: u32, kind: Kind, name: String },
    ChannelDelete(u32),
    History(u32),
    Say { channel: u32, text: String },
    VoiceJoin(u32),
    VoiceLeave,
    Nick(String),
    Ping,
    Quit,
}

fn id(s: &str) -> Result<u32, String> {
    s.trim().parse().map_err(|_| format!("bad id '{}'", clean_text(s)))
}

/// Split off the first word.
fn word(s: &str) -> (&str, &str) {
    match s.find(' ') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    }
}

impl Cmd {
    /// Parse one client line (without its newline).
    pub fn parse(line: &str) -> Result<Cmd, String> {
        let line = line.trim_end_matches('\r');
        let (w, rest) = word(line);
        Ok(match w.to_ascii_uppercase().as_str() {
            "HELLO" => Cmd::Hello(nick(rest)?),
            "GCREATE" => Cmd::GuildCreate(guild_name(rest)?),
            "GJOIN" => Cmd::GuildJoin(id(rest)?),
            "GLEAVE" => Cmd::GuildLeave(id(rest)?),
            "GDELETE" => Cmd::GuildDelete(id(rest)?),
            "CCREATE" => {
                let (g, rest) = word(rest);
                let (k, name) = word(rest);
                let kind = Kind::parse(k).ok_or_else(|| format!("channel kind '{}': text or voice", clean_text(k)))?;
                Cmd::ChannelCreate { guild: id(g)?, kind, name: channel_name(kind, name)? }
            }
            "CDELETE" => Cmd::ChannelDelete(id(rest)?),
            "HISTORY" => Cmd::History(id(rest)?),
            "SAY" => {
                let (c, text) = word(rest);
                let text = clean_text(text);
                if text.trim().is_empty() {
                    return Err("empty message".into());
                }
                Cmd::Say { channel: id(c)?, text }
            }
            "VJOIN" => Cmd::VoiceJoin(id(rest)?),
            "VLEAVE" => Cmd::VoiceLeave,
            "NICK" => Cmd::Nick(nick(rest)?),
            "PING" => Cmd::Ping,
            "QUIT" => Cmd::Quit,
            "" => return Err("empty line".into()),
            other => return Err(format!("unknown command {}", clean_text(other))),
        })
    }

    /// The wire line (without newline).
    pub fn line(&self) -> String {
        match self {
            Cmd::Hello(n) => format!("HELLO {n}"),
            Cmd::GuildCreate(n) => format!("GCREATE {n}"),
            Cmd::GuildJoin(g) => format!("GJOIN {g}"),
            Cmd::GuildLeave(g) => format!("GLEAVE {g}"),
            Cmd::GuildDelete(g) => format!("GDELETE {g}"),
            Cmd::ChannelCreate { guild, kind, name } => format!("CCREATE {guild} {} {name}", kind.word()),
            Cmd::ChannelDelete(c) => format!("CDELETE {c}"),
            Cmd::History(c) => format!("HISTORY {c}"),
            Cmd::Say { channel, text } => format!("SAY {channel} {text}"),
            Cmd::VoiceJoin(c) => format!("VJOIN {c}"),
            Cmd::VoiceLeave => "VLEAVE".into(),
            Cmd::Nick(n) => format!("NICK {n}"),
            Cmd::Ping => "PING".into(),
            Cmd::Quit => "QUIT".into(),
        }
    }
}

/// A member's state in a server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Online,
    Offline,
    /// No longer a member.
    Left,
}

impl Presence {
    pub fn word(self) -> &'static str {
        match self {
            Presence::Online => "online",
            Presence::Offline => "offline",
            Presence::Left => "left",
        }
    }
    fn parse(s: &str) -> Option<Presence> {
        Some(match s {
            "online" => Presence::Online,
            "offline" => Presence::Offline,
            "left" => Presence::Left,
            _ => return None,
        })
    }
}

/// What happens to someone in a voice channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceEvent {
    Join,
    Leave,
    Speaking,
    Quiet,
}

impl VoiceEvent {
    pub fn word(self) -> &'static str {
        match self {
            VoiceEvent::Join => "join",
            VoiceEvent::Leave => "leave",
            VoiceEvent::Speaking => "speaking",
            VoiceEvent::Quiet => "quiet",
        }
    }
    fn parse(s: &str) -> Option<VoiceEvent> {
        Some(match s {
            "join" => VoiceEvent::Join,
            "leave" => VoiceEvent::Leave,
            "speaking" => VoiceEvent::Speaking,
            "quiet" => VoiceEvent::Quiet,
            _ => return None,
        })
    }
}

/// An event from a backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Ready { nick: String, backend: String },
    Guild { id: u32, owner: String, joined: bool, members: u32, name: String },
    GuildGone(u32),
    Channel { guild: u32, id: u32, kind: Kind, name: String },
    ChannelGone(u32),
    Member { guild: u32, nick: String, presence: Presence },
    /// A message; `earlier` for history (`H`), else new (`M`).
    Msg { channel: u32, id: u32, time: String, nick: String, text: String, earlier: bool },
    HistoryEnd(u32),
    Voice { channel: u32, port: u16, token: u32, sid: u32 },
    VoiceState { channel: u32, sid: u32, nick: String, event: VoiceEvent },
    Renamed { old: String, new: String },
    Synced,
    Pong,
    Err(String),
}

impl Event {
    /// The wire line (without newline).
    pub fn line(&self) -> String {
        match self {
            Event::Ready { nick, backend } => format!("READY {nick} {backend}"),
            Event::Guild { id, owner, joined, members, name } => {
                format!("GUILD {id} {owner} {} {members} {name}", *joined as u8)
            }
            Event::GuildGone(g) => format!("GDEL {g}"),
            Event::Channel { guild, id, kind, name } => format!("CHANNEL {guild} {id} {} {name}", kind.word()),
            Event::ChannelGone(c) => format!("CDEL {c}"),
            Event::Member { guild, nick, presence } => format!("MEMBER {guild} {nick} {}", presence.word()),
            Event::Msg { channel, id, time, nick, text, earlier } => {
                format!("{} {channel} {id} {time} {nick} {text}", if *earlier { "H" } else { "M" })
            }
            Event::HistoryEnd(c) => format!("HEND {c}"),
            Event::Voice { channel, port, token, sid } => format!("VOICE {channel} {port} {token} {sid}"),
            Event::VoiceState { channel, sid, nick, event } => format!("VSTATE {channel} {sid} {nick} {}", event.word()),
            Event::Renamed { old, new } => format!("RENAME {old} {new}"),
            Event::Synced => "SYNCED".into(),
            Event::Pong => "PONG".into(),
            Event::Err(t) => format!("ERR {t}"),
        }
    }

    /// Parse a backend line; None for anything unrecognised (old chat lines included).
    pub fn parse(line: &str) -> Option<Event> {
        let line = line.trim_end_matches('\r');
        let (w, rest) = word(line);
        let p: Vec<&str> = rest.splitn(6, ' ').collect();
        let n = |i: usize| -> Option<u32> { p.get(i)?.parse().ok() };
        let s = |i: usize| -> Option<String> { p.get(i).map(|x| x.to_string()) };
        // The rest of the line from word `i` on (names and texts contain spaces).
        let tail = |i: usize| -> String {
            let mut it = rest.splitn(i + 1, ' ');
            it.nth(i).unwrap_or("").to_string()
        };
        Some(match w {
            "READY" => Event::Ready { nick: s(0)?, backend: tail(1) },
            "GUILD" => Event::Guild { id: n(0)?, owner: s(1)?, joined: p.get(2)? == &"1", members: n(3)?, name: tail(4) },
            "GDEL" => Event::GuildGone(n(0)?),
            "CHANNEL" => Event::Channel { guild: n(0)?, id: n(1)?, kind: Kind::parse(p.get(2)?)?, name: tail(3) },
            "CDEL" => Event::ChannelGone(n(0)?),
            "MEMBER" => Event::Member { guild: n(0)?, nick: s(1)?, presence: Presence::parse(p.get(2)?)? },
            "M" | "H" => Event::Msg {
                channel: n(0)?,
                id: n(1)?,
                time: s(2)?,
                nick: s(3)?,
                text: tail(4),
                earlier: w == "H",
            },
            "HEND" => Event::HistoryEnd(n(0)?),
            "VOICE" => Event::Voice { channel: n(0)?, port: p.get(1)?.parse().ok()?, token: n(2)?, sid: n(3)? },
            "VSTATE" => Event::VoiceState { channel: n(0)?, sid: n(1)?, nick: s(2)?, event: VoiceEvent::parse(p.get(3)?)? },
            "RENAME" => Event::Renamed { old: s(0)?, new: s(1)? },
            "SYNCED" => Event::Synced,
            "PONG" => Event::Pong,
            "ERR" => Event::Err(rest.to_string()),
            _ => return None,
        })
    }
}

/// A nickname: the chat protocol's rule (1-16 of letters, digits, `-`, `_`, `.`).
pub fn nick(s: &str) -> Result<String, String> {
    let s = s.trim();
    if ecm_chat::proto::valid_nick(s) {
        Ok(s.to_string())
    } else {
        Err(format!(
            "invalid nickname '{}': 1-{} letters, digits, '-', '_' or '.'",
            clean_text(s),
            ecm_chat::proto::MAX_NICK
        ))
    }
}

/// A server name: 1-32 printable characters, spaces squeezed.
pub fn guild_name(s: &str) -> Result<String, String> {
    let n = squeeze(s);
    if n.is_empty() {
        return Err("a server needs a name".into());
    }
    if n.chars().count() > MAX_GUILD_NAME {
        return Err(format!("server names are at most {MAX_GUILD_NAME} characters"));
    }
    Ok(n)
}

/// A channel name. Text channels follow Discord's rule: lower case, spaces become `-`
/// ("Build Plans" -> "build-plans"); voice channels keep what was typed.
pub fn channel_name(kind: Kind, s: &str) -> Result<String, String> {
    let mut n = squeeze(s.trim_start_matches('#'));
    if kind == Kind::Text {
        n = n.to_lowercase().replace(' ', "-");
    }
    if n.is_empty() {
        return Err("a channel needs a name".into());
    }
    if n.chars().count() > MAX_CHANNEL_NAME {
        return Err(format!("channel names are at most {MAX_CHANNEL_NAME} characters"));
    }
    Ok(n)
}

/// Printable, one line, spaces squeezed.
fn squeeze(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Drop control characters (nobody moves another client's cursor) and cap the length.
pub fn clean_text(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_control() {
            continue;
        }
        if out.len() + c.len_utf8() > MAX_TEXT {
            break;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_parse_and_round_trip() {
        let cmds = vec![
            Cmd::Hello("alex".into()),
            Cmd::GuildCreate("Builders Guild".into()),
            Cmd::GuildJoin(3),
            Cmd::GuildLeave(3),
            Cmd::GuildDelete(3),
            Cmd::ChannelCreate { guild: 3, kind: Kind::Text, name: "build-plans".into() },
            Cmd::ChannelCreate { guild: 3, kind: Kind::Voice, name: "The Lounge".into() },
            Cmd::ChannelDelete(9),
            Cmd::History(9),
            Cmd::Say { channel: 9, text: "hello there".into() },
            Cmd::VoiceJoin(4),
            Cmd::VoiceLeave,
            Cmd::Nick("al".into()),
            Cmd::Ping,
            Cmd::Quit,
        ];
        for c in cmds {
            assert_eq!(Cmd::parse(&c.line()), Ok(c.clone()), "{}", c.line());
        }
        assert_eq!(
            Cmd::parse("CCREATE 3 text Build Plans"),
            Ok(Cmd::ChannelCreate { guild: 3, kind: Kind::Text, name: "build-plans".into() })
        );
        assert_eq!(Cmd::parse("ccreate 3 TEXT #News"), Ok(Cmd::ChannelCreate { guild: 3, kind: Kind::Text, name: "news".into() }));
        assert!(Cmd::parse("CCREATE 3 video x").unwrap_err().contains("text or voice"));
        assert!(Cmd::parse("GCREATE   ").is_err());
        assert!(Cmd::parse(&format!("GCREATE {}", "x".repeat(40))).is_err());
        assert!(Cmd::parse("GJOIN abc").is_err());
        assert!(Cmd::parse("SAY 4   ").is_err());
        assert!(Cmd::parse("HELLO bad nick").is_err());
        assert!(Cmd::parse("BOGUS").is_err());
        assert_eq!(Cmd::parse("SAY 4 hi\x1b[2Jthere"), Ok(Cmd::Say { channel: 4, text: "hi[2Jthere".into() }));
    }

    #[test]
    fn events_round_trip() {
        let events = vec![
            Event::Ready { nick: "alex".into(), backend: "Tech Village".into() },
            Event::Guild { id: 1, owner: "-".into(), joined: true, members: 4, name: "Tech Village".into() },
            Event::Guild { id: 2, owner: "bo".into(), joined: false, members: 1, name: "Bo's Builds".into() },
            Event::GuildGone(2),
            Event::Channel { guild: 1, id: 1, kind: Kind::Text, name: "general".into() },
            Event::Channel { guild: 1, id: 2, kind: Kind::Voice, name: "General Voice".into() },
            Event::ChannelGone(2),
            Event::Member { guild: 1, nick: "bo".into(), presence: Presence::Online },
            Event::Member { guild: 1, nick: "bo".into(), presence: Presence::Left },
            Event::Msg { channel: 1, id: 7, time: "12:00:00".into(), nick: "bo".into(), text: "hi all".into(), earlier: false },
            Event::Msg { channel: 1, id: 6, time: "11:00:00".into(), nick: "cy".into(), text: "older".into(), earlier: true },
            Event::HistoryEnd(1),
            Event::Voice { channel: 2, port: 7778, token: 123456, sid: 3 },
            Event::VoiceState { channel: 2, sid: 3, nick: "bo".into(), event: VoiceEvent::Speaking },
            Event::Renamed { old: "bo".into(), new: "bob".into() },
            Event::Synced,
            Event::Pong,
            Event::Err("you don't own that server".into()),
        ];
        for e in events {
            assert_eq!(Event::parse(&e.line()), Some(e.clone()), "{}", e.line());
        }
        assert_eq!(Event::parse("WELCOME alex room"), None, "old chat lines are not Discord events");
        assert_eq!(Event::parse("GUILD x"), None);
    }

    #[test]
    fn names() {
        assert_eq!(channel_name(Kind::Text, " Build  Plans "), Ok("build-plans".into()));
        assert_eq!(channel_name(Kind::Voice, " The  Lounge "), Ok("The Lounge".into()));
        assert!(channel_name(Kind::Text, "#").is_err());
        assert_eq!(guild_name("  My\tServer "), Ok("My Server".into()));
    }
}
