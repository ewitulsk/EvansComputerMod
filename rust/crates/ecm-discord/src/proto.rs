//! The Discord line protocol (`DISCORD/2`), spoken between the Discord app and a
//! Discord backend (`discordd`) over TCP (default port [`DEFAULT_PORT`]).
//!
//! Every message is one UTF-8 line ending in `\n`. Accounts are required: a backend
//! trusts one realm (an Auth server, `authd`, or its own with `--auth local`) and every
//! client signs in with a token from that realm ([`ecm_auth::token`]).
//!
//! On connect the backend greets every client with two lines:
//!
//! | Line | Meaning |
//! |---|---|
//! | `AUTH <realm fp> <host:port\|-:port> <realm name...>` | The realm it trusts and where it is ([`ecm_auth::RealmRef`]); `-:port` is the backend's own address at that port (`--auth local`) |
//! | `BACKEND <backend fp> <nonce hex> <backend name...>` | The backend's own key fingerprint (the token's audience) and this connection's nonce |
//!
//! The client gets a token for `<backend fp>` from the realm and answers
//! `HELLO <token> <proof>` (the Discord app) or `LOGIN <token> <proof>` (the console
//! `chat` command, which then speaks the old chat protocol, `ecm_chat::proto`, in the
//! home server's `#general`). `<proof>` is `ecm_auth::token::prove(session key, nonce,
//! backend fp)` in hex. A refused sign-in is `ERR <why>`; the third one closes the
//! connection. Nothing else is accepted before signing in. [`Greeting`] collects the two
//! lines on the client side and [`realm_host`] says where to get the token.
//!
//! A "backend" is the program (`discordd`); the "servers" a player sees in the app
//! are the backend's guilds (as in Discord's own API). Users are named by their realm
//! **user id** (`<uid>`, stable); what the app shows is their **name**: the backend
//! nickname they set with `NICK`, else their account's display name.
//!
//! Client to backend (after `HELLO`):
//!
//! | Line | Meaning |
//! |---|---|
//! | `HELLO <token> <proof>` | Sign in (first line after the greeting) |
//! | `GCREATE <name>` | Make a server (you own it; it gets `#general` and a `General` voice channel) |
//! | `GJOIN <gid>` / `GLEAVE <gid>` | Join / leave a server |
//! | `GDELETE <gid>` | Delete a server you own |
//! | `CCREATE <gid> text\|voice <name>` | Make a channel in a server you own |
//! | `CDELETE <cid>` | Delete a channel of a server you own |
//! | `HISTORY <cid>` | The channel's recent messages (`USER` lines for their authors, `H` lines, then `HEND`) |
//! | `SAY <cid> <text>` | Post in a text channel |
//! | `VJOIN <cid>` / `VLEAVE` | Join a voice channel (answered by `VOICE`) / leave voice |
//! | `NICK [<name...>]` | Set your nickname on this backend (1-32 printable characters); none: back to your display name |
//! | `PING` / `QUIT` | Keep-alive / leave |
//!
//! Backend to client (`<t>` is the backend's `HH:MM:SS` UTC time, `<uid>` a user id, 0 = nobody):
//!
//! | Line | Meaning |
//! |---|---|
//! | `READY <uid> <name...>` | Signed in as user `<uid>`, shown as `<name>` |
//! | `USER <uid> <username> <name...>` | Who a user id is (before history from people the client may not know) |
//! | `GUILD <gid> <owner uid> <0\|1 joined> <members> <name...>` | A server exists (or changed) |
//! | `GDEL <gid>` | A server was deleted |
//! | `CHANNEL <gid> <cid> text\|voice <name...>` | A channel exists |
//! | `CDEL <cid>` | A channel was deleted |
//! | `MEMBER <gid> <uid> online\|offline\|left <name...>` | A member of a server you joined |
//! | `M <cid> <mid> <t> <uid> <text...>` | A new message |
//! | `H <cid> <mid> <t> <uid> <text...>` / `HEND <cid>` | History, oldest first / its end |
//! | `VOICE <cid> <udp port> <token> <sid>` | Your voice session: send audio to the UDP port with the token |
//! | `VSTATE <cid> <sid> <uid> join\|leave\|speaking\|quiet` | Who is in a voice channel and who talks |
//! | `RENAME <uid> <name...>` | Someone's name changed (nickname or display name) |
//! | `SYNCED` | The first full picture after `READY` is complete |
//! | `PONG` / `ERR <text>` | Keep-alive answer / your last request was refused |
//!
//! Voice travels over UDP (see [`crate::voice`]).

use ecm_auth::RealmRef;

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
    /// Sign in to the Discord protocol: a token for this backend and the proof (hex).
    Hello { token: String, proof: String },
    /// Sign in to the old chat protocol (the `chat` command).
    Login { token: String, proof: String },
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
    /// A backend nickname; None goes back to the display name.
    Nick(Option<String>),
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

/// `<token> <proof>` of HELLO / LOGIN.
fn credentials(rest: &str) -> Result<(String, String), String> {
    let (token, proof) = word(rest.trim());
    let proof = proof.trim();
    if !token.starts_with(ecm_auth::token::PREFIX) || proof.len() != 128 || ecm_auth::unhex(proof).is_none() {
        return Err("sign in with a token and its proof (HELLO <token> <proof>)".into());
    }
    Ok((token.to_string(), proof.to_ascii_lowercase()))
}

impl Cmd {
    /// Parse one client line (without its newline).
    pub fn parse(line: &str) -> Result<Cmd, String> {
        let line = line.trim_end_matches('\r');
        let (w, rest) = word(line);
        Ok(match w.to_ascii_uppercase().as_str() {
            "HELLO" => {
                let (token, proof) = credentials(rest)?;
                Cmd::Hello { token, proof }
            }
            "LOGIN" => {
                let (token, proof) = credentials(rest)?;
                Cmd::Login { token, proof }
            }
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
            "NICK" if rest.trim().is_empty() => Cmd::Nick(None),
            "NICK" => Cmd::Nick(Some(nick(rest)?)),
            "PING" => Cmd::Ping,
            "QUIT" => Cmd::Quit,
            "" => return Err("empty line".into()),
            other => return Err(format!("unknown command {}", clean_text(other))),
        })
    }

    /// The wire line (without newline).
    pub fn line(&self) -> String {
        match self {
            Cmd::Hello { token, proof } => format!("HELLO {token} {proof}"),
            Cmd::Login { token, proof } => format!("LOGIN {token} {proof}"),
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
            Cmd::Nick(Some(n)) => format!("NICK {n}"),
            Cmd::Nick(None) => "NICK".into(),
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
    /// Greeting, line 1: the realm the backend trusts.
    Auth(RealmRef),
    /// Greeting, line 2: the backend's key fingerprint, this connection's nonce (hex), its name.
    Backend { fp: String, nonce: String, name: String },
    Ready { uid: u64, name: String },
    User { uid: u64, username: String, name: String },
    /// `owner` 0: nobody (the home server).
    Guild { id: u32, owner: u64, joined: bool, members: u32, name: String },
    GuildGone(u32),
    Channel { guild: u32, id: u32, kind: Kind, name: String },
    ChannelGone(u32),
    Member { guild: u32, uid: u64, presence: Presence, name: String },
    /// A message; `earlier` for history (`H`), else new (`M`).
    Msg { channel: u32, id: u32, time: String, uid: u64, text: String, earlier: bool },
    HistoryEnd(u32),
    Voice { channel: u32, port: u16, token: u32, sid: u32 },
    VoiceState { channel: u32, sid: u32, uid: u64, event: VoiceEvent },
    Renamed { uid: u64, name: String },
    Synced,
    Pong,
    Err(String),
}

impl Event {
    /// The wire line (without newline).
    pub fn line(&self) -> String {
        match self {
            Event::Auth(r) => r.to_line(),
            Event::Backend { fp, nonce, name } => format!("BACKEND {fp} {nonce} {name}"),
            Event::Ready { uid, name } => format!("READY {uid} {name}"),
            Event::User { uid, username, name } => format!("USER {uid} {username} {name}"),
            Event::Guild { id, owner, joined, members, name } => {
                format!("GUILD {id} {owner} {} {members} {name}", *joined as u8)
            }
            Event::GuildGone(g) => format!("GDEL {g}"),
            Event::Channel { guild, id, kind, name } => format!("CHANNEL {guild} {id} {} {name}", kind.word()),
            Event::ChannelGone(c) => format!("CDEL {c}"),
            Event::Member { guild, uid, presence, name } => format!("MEMBER {guild} {uid} {} {name}", presence.word()),
            Event::Msg { channel, id, time, uid, text, earlier } => {
                format!("{} {channel} {id} {time} {uid} {text}", if *earlier { "H" } else { "M" })
            }
            Event::HistoryEnd(c) => format!("HEND {c}"),
            Event::Voice { channel, port, token, sid } => format!("VOICE {channel} {port} {token} {sid}"),
            Event::VoiceState { channel, sid, uid, event } => format!("VSTATE {channel} {sid} {uid} {}", event.word()),
            Event::Renamed { uid, name } => format!("RENAME {uid} {name}"),
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
        let u = |i: usize| -> Option<u64> { p.get(i)?.parse().ok() };
        let s = |i: usize| -> Option<String> { p.get(i).map(|x| x.to_string()) };
        // The rest of the line from word `i` on (names and texts contain spaces).
        let tail = |i: usize| -> String {
            let mut it = rest.splitn(i + 1, ' ');
            it.nth(i).unwrap_or("").to_string()
        };
        Some(match w {
            "AUTH" => Event::Auth(RealmRef::parse(line).ok()?),
            "BACKEND" => {
                let fp = s(0).filter(|f| ecm_auth::is_fingerprint(f))?;
                let nonce = s(1).filter(|x| x.len() == 64 && ecm_auth::unhex(x).is_some())?;
                Event::Backend { fp, nonce, name: tail(2) }
            }
            "READY" => Event::Ready { uid: u(0)?, name: tail(1) },
            "USER" => Event::User { uid: u(0)?, username: s(1)?, name: tail(2) },
            "GUILD" => Event::Guild { id: n(0)?, owner: u(1)?, joined: p.get(2)? == &"1", members: n(3)?, name: tail(4) },
            "GDEL" => Event::GuildGone(n(0)?),
            "CHANNEL" => Event::Channel { guild: n(0)?, id: n(1)?, kind: Kind::parse(p.get(2)?)?, name: tail(3) },
            "CDEL" => Event::ChannelGone(n(0)?),
            "MEMBER" => Event::Member { guild: n(0)?, uid: u(1)?, presence: Presence::parse(p.get(2)?)?, name: tail(3) },
            "M" | "H" => Event::Msg { channel: n(0)?, id: n(1)?, time: s(2)?, uid: u(3)?, text: tail(4), earlier: w == "H" },
            "HEND" => Event::HistoryEnd(n(0)?),
            "VOICE" => Event::Voice { channel: n(0)?, port: p.get(1)?.parse().ok()?, token: n(2)?, sid: n(3)? },
            "VSTATE" => Event::VoiceState { channel: n(0)?, sid: n(1)?, uid: u(2)?, event: VoiceEvent::parse(p.get(3)?)? },
            "RENAME" => Event::Renamed { uid: u(0)?, name: tail(1) },
            "SYNCED" => Event::Synced,
            "PONG" => Event::Pong,
            "ERR" => Event::Err(rest.to_string()),
            _ => return None,
        })
    }
}

/// The backend's greeting, collected by a client from the first lines it receives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Greeting {
    pub realm: Option<RealmRef>,
    /// The backend's key fingerprint: the audience to ask the realm a token for.
    pub backend_fp: String,
    pub nonce: [u8; 32],
    pub backend_name: String,
}

impl Greeting {
    /// Feed a line; true once both greeting lines have arrived.
    pub fn feed(&mut self, line: &str) -> bool {
        match Event::parse(line) {
            Some(Event::Auth(r)) => self.realm = Some(r),
            Some(Event::Backend { fp, nonce, name }) => {
                self.backend_fp = fp;
                self.nonce = ecm_auth::unhex_n(&nonce).unwrap_or([0; 32]);
                self.backend_name = name;
            }
            _ => {}
        }
        self.complete()
    }

    pub fn complete(&self) -> bool {
        self.realm.is_some() && !self.backend_fp.is_empty()
    }

    /// The sign-in line: `HELLO` (Discord) or, with `chat`, `LOGIN`.
    pub fn sign_in_line(&self, token: &str, session_key: &ecm_auth::SigningKey, chat: bool) -> String {
        let proof = ecm_auth::hex(&ecm_auth::prove(session_key, &self.nonce, &self.backend_fp));
        let (token, proof) = (token.to_string(), proof);
        if chat { Cmd::Login { token, proof } } else { Cmd::Hello { token, proof } }.line()
    }
}

/// Where to reach the realm a backend named in its `AUTH` line, for a client that
/// reached the backend at `backend_host` (an address without port): `-:port` means the
/// backend's own address. None if the line gave no address.
pub fn realm_host(realm: &RealmRef, backend_host: &str) -> Option<String> {
    let ep = realm.endpoint.as_deref()?;
    Some(match ep.strip_prefix("-:") {
        Some(port) => format!("{backend_host}:{port}"),
        None => ep.to_string(),
    })
}

/// A backend nickname: the display-name rule (1-32 printable characters).
pub fn nick(s: &str) -> Result<String, String> {
    ecm_auth::display_name(s).map_err(|e| e.to_string())
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

    fn tok() -> String {
        "ecmtok1.cGF5bG9hZA.c2ln".into()
    }

    #[test]
    fn commands_parse_and_round_trip() {
        let proof = "ab".repeat(64);
        let cmds = vec![
            Cmd::Hello { token: tok(), proof: proof.clone() },
            Cmd::Login { token: tok(), proof: proof.clone() },
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
            Cmd::Nick(Some("Al the Builder".into())),
            Cmd::Nick(None),
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
        // The nickname-only HELLO is gone: a token and a proof are required.
        assert!(Cmd::parse("HELLO alex").is_err());
        assert!(Cmd::parse(&format!("HELLO {} short", tok())).is_err());
        assert!(Cmd::parse(&format!("HELLO notatoken {proof}")).is_err());
        assert!(Cmd::parse(&format!("NICK {}", "x".repeat(33))).is_err());
        assert!(Cmd::parse("BOGUS").is_err());
        assert_eq!(Cmd::parse("SAY 4 hi\x1b[2Jthere"), Ok(Cmd::Say { channel: 4, text: "hi[2Jthere".into() }));
    }

    #[test]
    fn events_round_trip() {
        let fp = "3fa19c0d00000000000000000000beef".to_string();
        let events = vec![
            Event::Auth(RealmRef { name: "Tech Village".into(), fp: fp.clone(), endpoint: Some("10.0.8.1:7600".into()) }),
            Event::Auth(RealmRef { name: "Lab".into(), fp: fp.clone(), endpoint: Some("-:7779".into()) }),
            Event::Backend { fp: fp.clone(), nonce: "0f".repeat(32), name: "Tech Village Discord".into() },
            Event::Ready { uid: 7, name: "Alex A.".into() },
            Event::User { uid: 9, username: "cy".into(), name: "Cy Gone".into() },
            Event::Guild { id: 1, owner: 0, joined: true, members: 4, name: "Tech Village".into() },
            Event::Guild { id: 2, owner: 9, joined: false, members: 1, name: "Bo's Builds".into() },
            Event::GuildGone(2),
            Event::Channel { guild: 1, id: 1, kind: Kind::Text, name: "general".into() },
            Event::Channel { guild: 1, id: 2, kind: Kind::Voice, name: "General Voice".into() },
            Event::ChannelGone(2),
            Event::Member { guild: 1, uid: 9, presence: Presence::Online, name: "Bo B.".into() },
            Event::Member { guild: 1, uid: 9, presence: Presence::Left, name: "Bo".into() },
            Event::Msg { channel: 1, id: 7, time: "12:00:00".into(), uid: 9, text: "hi all".into(), earlier: false },
            Event::Msg { channel: 1, id: 6, time: "11:00:00".into(), uid: 3, text: "older".into(), earlier: true },
            Event::HistoryEnd(1),
            Event::Voice { channel: 2, port: 7778, token: 123456, sid: 3 },
            Event::VoiceState { channel: 2, sid: 3, uid: 9, event: VoiceEvent::Speaking },
            Event::Renamed { uid: 9, name: "Bob the Builder".into() },
            Event::Synced,
            Event::Pong,
            Event::Err("you don't own that server".into()),
        ];
        for e in events {
            assert_eq!(Event::parse(&e.line()), Some(e.clone()), "{}", e.line());
        }
        assert_eq!(Event::parse("WELCOME alex room"), None, "old chat lines are not Discord events");
        assert_eq!(Event::parse("GUILD x"), None);
        assert_eq!(Event::parse("BACKEND nothex 00 x"), None);
    }

    #[test]
    fn the_greeting_tells_where_to_sign_in() {
        let fp = "3fa19c0d00000000000000000000beef";
        let mut g = Greeting::default();
        assert!(!g.feed(&format!("AUTH {fp} -:7779 Lab")));
        assert!(g.feed(&format!("BACKEND {fp} {} Lab", "01".repeat(32))));
        assert_eq!(g.nonce, [1; 32]);
        assert_eq!(realm_host(g.realm.as_ref().unwrap(), "10.0.0.5").as_deref(), Some("10.0.0.5:7779"));
        let shared = RealmRef::parse(&format!("AUTH {fp} 10.0.8.1:7600 Tech Village")).unwrap();
        assert_eq!(realm_host(&shared, "10.0.0.5").as_deref(), Some("10.0.8.1:7600"));
        let key = ecm_auth::signing_key(&[3; 32]);
        let line = g.sign_in_line("ecmtok1.a.b", &key, false);
        let Ok(Cmd::Hello { proof, .. }) = Cmd::parse(&line) else { panic!("{line}") };
        let want = ecm_auth::prove(&key, &[1; 32], fp);
        assert_eq!(ecm_auth::unhex(&proof).unwrap(), want.to_vec());
        assert!(g.sign_in_line("ecmtok1.a.b", &key, true).starts_with("LOGIN "));
    }

    #[test]
    fn names() {
        assert_eq!(channel_name(Kind::Text, " Build  Plans "), Ok("build-plans".into()));
        assert_eq!(channel_name(Kind::Voice, " The  Lounge "), Ok("The Lounge".into()));
        assert!(channel_name(Kind::Text, "#").is_err());
        assert_eq!(guild_name("  My\tServer "), Ok("My Server".into()));
        assert_eq!(nick(" Al  B. "), Ok("Al  B.".into()));
    }
}
