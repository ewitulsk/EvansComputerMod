//! A Discord backend (`discordd`'s logic) as a state machine.
//!
//! `discordd` owns the sockets: it calls [`Backend::connect`], [`Backend::receive`],
//! [`Backend::disconnect`] for TCP clients, [`Backend::udp`] for voice datagrams and
//! [`Backend::tick`] now and then, and performs the returned [`Out`] actions in
//! order. Clients are identified by any number the caller chooses (the socket fd).
//!
//! The backend hosts servers ("guilds"), each with text and voice channels and
//! members. Server 1 is the backend's home server (named after the backend, owned
//! by nobody): every Discord user joins it on their first visit, and the old chat
//! protocol's room is its first text channel, so `chat` users and Discord users
//! talk together in `#general`.
//!
//! What must survive a restart (servers, channels, members, recent messages) is
//! written as text by [`Backend::save_text`] whenever [`Out::Save`] is returned.

use crate::proto::{self, Cmd, Event, Kind, Presence, VoiceEvent};
use crate::voice::{Packet, PacketKind, SPEAKING_HOLD_MS};
use ecm_chat::proto as chat;
use std::collections::{BTreeMap, VecDeque};

/// Most clients one backend accepts (the kernel allows 32 sockets per program; the
/// backend uses two itself).
pub const MAX_CLIENTS: usize = 28;
/// The home server's id.
pub const HOME: u32 = 1;
/// Old-protocol clients get this many messages of `#general` when they join.
pub const LEGACY_HISTORY: usize = 20;
/// The owner of the home server: nobody.
pub const NOBODY: &str = "-";

/// Something `discordd` must do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    /// Send this line (a newline is appended) to the client.
    Send(u32, String),
    /// Close the client's socket (after sending what came before).
    Close(u32),
    /// Print to `discordd`'s console.
    Log(String),
    /// Send a datagram from the voice port.
    Udp([u8; 4], u16, Vec<u8>),
    /// The saved state changed: write [`Backend::save_text`].
    Save,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The old chat protocol (the `chat` command).
    Legacy,
    Discord,
}

#[derive(Debug)]
struct Conn {
    peer: String,
    buf: Vec<u8>,
    nick: Option<String>,
    mode: Mode,
    /// Voice session, if in a voice channel.
    sid: Option<u32>,
}

#[derive(Debug, Clone)]
struct Guild {
    name: String,
    owner: String,
    members: Vec<String>,
}

#[derive(Debug, Clone)]
struct Message {
    id: u32,
    time: String,
    nick: String,
    text: String,
}

#[derive(Debug, Clone)]
struct Channel {
    guild: u32,
    kind: Kind,
    name: String,
    messages: VecDeque<Message>,
}

#[derive(Debug, Clone)]
struct Voice {
    conn: u32,
    channel: u32,
    token: u32,
    nick: String,
    addr: Option<([u8; 4], u16)>,
    last_audio: u64,
    speaking: bool,
}

/// One Discord backend.
#[derive(Debug)]
pub struct Backend {
    pub name: String,
    pub voice_port: u16,
    guilds: BTreeMap<u32, Guild>,
    channels: BTreeMap<u32, Channel>,
    next_guild: u32,
    next_channel: u32,
    next_msg: u32,
    conns: BTreeMap<u32, Conn>,
    voice: BTreeMap<u32, Voice>,
    next_sid: u32,
    rng: u64,
    /// Voice packets forwarded (tests, the status line).
    pub forwarded: u64,
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl Backend {
    /// A new backend with only its home server (`#general` and a `General` voice channel).
    /// `seed` makes the voice tokens unpredictable (the program passes random bits).
    pub fn new(name: &str, voice_port: u16, seed: u64) -> Backend {
        let mut b = Backend {
            name: name.to_string(),
            voice_port,
            guilds: BTreeMap::new(),
            channels: BTreeMap::new(),
            next_guild: HOME,
            next_channel: 1,
            next_msg: 1,
            conns: BTreeMap::new(),
            voice: BTreeMap::new(),
            next_sid: 1,
            rng: seed | 1,
            forwarded: 0,
        };
        b.ensure_home();
        b
    }

    /// A backend restored from [`Backend::save_text`] output (unreadable lines are skipped).
    pub fn load(name: &str, voice_port: u16, seed: u64, text: &str) -> Backend {
        let mut b = Backend::new(name, voice_port, seed);
        b.guilds.clear();
        b.channels.clear();
        for line in text.lines() {
            let mut it = line.splitn(2, ' ');
            let (w, rest) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
            let p: Vec<&str> = rest.splitn(5, ' ').collect();
            let n = |i: usize| p.get(i).and_then(|s| s.parse::<u32>().ok());
            let tail = |i: usize| rest.splitn(i + 1, ' ').nth(i).unwrap_or("").to_string();
            match w {
                "next" => {
                    if let (Some(g), Some(c), Some(m)) = (n(0), n(1), n(2)) {
                        b.next_guild = g;
                        b.next_channel = c;
                        b.next_msg = m;
                    }
                }
                "guild" => {
                    if let (Some(id), Some(owner)) = (n(0), p.get(1)) {
                        b.guilds.insert(id, Guild { name: tail(2), owner: owner.to_string(), members: Vec::new() });
                    }
                }
                "member" => {
                    if let (Some(g), Some(nick)) = (n(0), p.get(1)) {
                        if let Some(guild) = b.guilds.get_mut(&g) {
                            if !guild.members.iter().any(|m| same(m, nick)) {
                                guild.members.push(nick.to_string());
                            }
                        }
                    }
                }
                "channel" => {
                    if let (Some(id), Some(g), Some(kind)) = (n(0), n(1), p.get(2).and_then(|k| Kind::parse(k))) {
                        if b.guilds.contains_key(&g) {
                            b.channels.insert(id, Channel { guild: g, kind, name: tail(3), messages: VecDeque::new() });
                        }
                    }
                }
                "msg" => {
                    if let (Some(c), Some(id), Some(time), Some(nick)) = (n(0), n(1), p.get(2), p.get(3)) {
                        if let Some(ch) = b.channels.get_mut(&c) {
                            ch.messages.push_back(Message { id, time: time.to_string(), nick: nick.to_string(), text: tail(4) });
                            while ch.messages.len() > proto::HISTORY {
                                ch.messages.pop_front();
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // Counters never go backwards past what was loaded.
        b.next_guild = b.next_guild.max(b.guilds.keys().max().map_or(HOME, |g| g + 1));
        b.next_channel = b.next_channel.max(b.channels.keys().max().map_or(1, |c| c + 1));
        let top_msg = b.channels.values().flat_map(|c| c.messages.iter().map(|m| m.id)).max().unwrap_or(0);
        b.next_msg = b.next_msg.max(top_msg + 1);
        b.ensure_home();
        if let Some(home) = b.guilds.get_mut(&HOME) {
            home.name = name.to_string();
        }
        b
    }

    /// The state to keep across restarts.
    pub fn save_text(&self) -> String {
        let mut s = format!("discordd 1\nnext {} {} {}\n", self.next_guild, self.next_channel, self.next_msg);
        for (id, g) in &self.guilds {
            s.push_str(&format!("guild {} {} {}\n", id, g.owner, g.name));
            for m in &g.members {
                s.push_str(&format!("member {} {}\n", id, m));
            }
        }
        for (id, c) in &self.channels {
            s.push_str(&format!("channel {} {} {} {}\n", id, c.guild, c.kind.word(), c.name));
            for m in &c.messages {
                s.push_str(&format!("msg {} {} {} {} {}\n", id, m.id, m.time, m.nick, m.text));
            }
        }
        s
    }

    fn ensure_home(&mut self) {
        if self.guilds.contains_key(&HOME) {
            return;
        }
        self.guilds.insert(HOME, Guild { name: self.name.clone(), owner: NOBODY.into(), members: Vec::new() });
        self.next_guild = self.next_guild.max(HOME + 1);
        self.add_channel(HOME, Kind::Text, "general");
        self.add_channel(HOME, Kind::Voice, "General");
    }

    fn add_channel(&mut self, guild: u32, kind: Kind, name: &str) -> u32 {
        let id = self.next_channel;
        self.next_channel += 1;
        self.channels.insert(id, Channel { guild, kind, name: name.into(), messages: VecDeque::new() });
        id
    }

    fn token(&mut self) -> u32 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 32) as u32 | 1
    }

    // ------------------------------------------------------------------ queries

    pub fn len(&self) -> usize {
        self.conns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.conns.is_empty()
    }

    /// Server names, by id.
    pub fn guild_names(&self) -> Vec<(u32, String)> {
        self.guilds.iter().map(|(id, g)| (*id, g.name.clone())).collect()
    }

    /// Channels of a server: (id, kind, name).
    pub fn channels_of(&self, guild: u32) -> Vec<(u32, Kind, String)> {
        self.channels.iter().filter(|(_, c)| c.guild == guild).map(|(id, c)| (*id, c.kind, c.name.clone())).collect()
    }

    /// The members of a server.
    pub fn members_of(&self, guild: u32) -> Vec<String> {
        self.guilds.get(&guild).map(|g| g.members.clone()).unwrap_or_default()
    }

    /// Texts in a channel, oldest first.
    pub fn messages_of(&self, channel: u32) -> Vec<String> {
        self.channels.get(&channel).map(|c| c.messages.iter().map(|m| format!("<{}> {}", m.nick, m.text)).collect()).unwrap_or_default()
    }

    /// Who is in a voice channel.
    pub fn voice_in(&self, channel: u32) -> Vec<String> {
        self.voice.values().filter(|v| v.channel == channel).map(|v| v.nick.clone()).collect()
    }

    /// The old chat room: the home server's first text channel.
    pub fn home_general(&self) -> Option<u32> {
        self.channels.iter().find(|(_, c)| c.guild == HOME && c.kind == Kind::Text).map(|(id, _)| *id)
    }

    /// Nicknames connected now, sorted.
    pub fn nicks(&self) -> Vec<String> {
        let mut n: Vec<String> = self.conns.values().filter_map(|c| c.nick.clone()).collect();
        n.sort_by_key(|s| s.to_lowercase());
        n
    }

    fn online(&self, nick: &str) -> bool {
        self.conns.values().any(|c| c.nick.as_deref().is_some_and(|n| same(n, nick)))
    }

    fn is_member(&self, guild: u32, nick: &str) -> bool {
        self.guilds.get(&guild).is_some_and(|g| g.members.iter().any(|m| same(m, nick)))
    }

    fn taken(&self, nick: &str, except: u32) -> bool {
        self.conns.iter().any(|(id, c)| *id != except && c.nick.as_deref().is_some_and(|n| same(n, nick)))
    }

    /// A free variant of `wanted`: `wanted`, `wanted2`, `wanted3`, ...
    fn free_nick(&self, wanted: &str, id: u32) -> String {
        if !self.taken(wanted, id) {
            return wanted.to_string();
        }
        for n in 2.. {
            let suffix = n.to_string();
            let base: String = wanted.chars().take(chat::MAX_NICK - suffix.len()).collect();
            let candidate = format!("{}{}", base, suffix);
            if !self.taken(&candidate, id) {
                return candidate;
            }
        }
        unreachable!()
    }

    // ------------------------------------------------------------------ event lines

    fn guild_event(&self, id: u32, for_nick: &str) -> Option<Event> {
        let g = self.guilds.get(&id)?;
        Some(Event::Guild {
            id,
            owner: g.owner.clone(),
            joined: g.members.iter().any(|m| same(m, for_nick)),
            members: g.members.len() as u32,
            name: g.name.clone(),
        })
    }

    fn channel_event(&self, id: u32) -> Option<Event> {
        let c = self.channels.get(&id)?;
        Some(Event::Channel { guild: c.guild, id, kind: c.kind, name: c.name.clone() })
    }

    /// Discord connections (id, nick).
    fn discord_conns(&self) -> Vec<(u32, String)> {
        self.conns
            .iter()
            .filter(|(_, c)| c.mode == Mode::Discord)
            .filter_map(|(id, c)| c.nick.clone().map(|n| (*id, n)))
            .collect()
    }

    /// Send `e` to every Discord connection that is a member of `guild`.
    fn to_members(&self, guild: u32, e: &Event) -> Vec<Out> {
        let line = e.line();
        self.discord_conns()
            .into_iter()
            .filter(|(_, n)| self.is_member(guild, n))
            .map(|(id, _)| Out::Send(id, line.clone()))
            .collect()
    }

    /// Every Discord connection gets the server's line, with its own `joined` flag.
    fn guild_to_all(&self, guild: u32) -> Vec<Out> {
        self.discord_conns()
            .into_iter()
            .filter_map(|(id, n)| self.guild_event(guild, &n).map(|e| Out::Send(id, e.line())))
            .collect()
    }

    /// Old-protocol connections that joined.
    fn to_legacy(&self, line: &str) -> Vec<Out> {
        self.conns
            .iter()
            .filter(|(_, c)| c.mode == Mode::Legacy && c.nick.is_some())
            .map(|(id, _)| Out::Send(*id, line.to_string()))
            .collect()
    }

    // ------------------------------------------------------------------ connections

    /// A client connected from `peer` ("a.b.c.d:port").
    pub fn connect(&mut self, id: u32, peer: &str) -> Vec<Out> {
        if self.conns.len() >= MAX_CLIENTS {
            return vec![
                Out::Send(id, chat::Event::Err(format!("server full ({} users)", MAX_CLIENTS)).line()),
                Out::Close(id),
                Out::Log(format!("refused {}: full", peer)),
            ];
        }
        self.conns.insert(id, Conn { peer: peer.into(), buf: Vec::new(), nick: None, mode: Mode::Legacy, sid: None });
        vec![Out::Log(format!("connection from {}", peer))]
    }

    /// Bytes arrived from a client. `now` is Unix time in milliseconds.
    pub fn receive(&mut self, id: u32, data: &[u8], now: u64) -> Vec<Out> {
        let lines = match self.conns.get_mut(&id) {
            Some(c) => chat::take_lines(&mut c.buf, data),
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let mode = self.conns[&id].mode;
            out.extend(match mode {
                Mode::Legacy => self.legacy(id, &line, now),
                Mode::Discord => self.discord(id, &line, now),
            });
            if !self.conns.contains_key(&id) {
                break;
            }
        }
        out
    }

    /// The client's connection ended (EOF or error).
    pub fn disconnect(&mut self, id: u32, now: u64) -> Vec<Out> {
        self.leave(id, "connection closed", now)
    }

    fn leave(&mut self, id: u32, reason: &str, now: u64) -> Vec<Out> {
        let mut out = self.voice_leave(id);
        let Some(c) = self.conns.remove(&id) else { return out };
        let Some(nick) = c.nick else {
            out.push(Out::Log(format!("{} disconnected", c.peer)));
            return out;
        };
        out.extend(self.to_legacy(&chat::Event::Leave { time: chat::hms(now / 1000), nick: nick.clone(), reason: reason.into() }.line()));
        if !self.online(&nick) {
            let guilds: Vec<u32> = self.guilds.keys().copied().collect();
            for g in guilds {
                if self.is_member(g, &nick) {
                    out.extend(self.to_members(g, &Event::Member { guild: g, nick: nick.clone(), presence: Presence::Offline }));
                } else if g == HOME && c.mode == Mode::Legacy {
                    out.extend(self.to_members(g, &Event::Member { guild: g, nick: nick.clone(), presence: Presence::Left }));
                }
            }
        }
        out.push(Out::Log(format!("{} left ({})", nick, reason)));
        out
    }

    /// Announce that `nick` came online (to old-protocol rooms and to the members of its servers).
    fn arrived(&self, id: u32, nick: &str, now: u64, legacy: bool) -> Vec<Out> {
        let mut out: Vec<Out> = self
            .to_legacy(&chat::Event::Join { time: chat::hms(now / 1000), nick: nick.into() }.line())
            .into_iter()
            .filter(|o| !matches!(o, Out::Send(to, _) if *to == id && !legacy))
            .collect();
        for g in self.guilds.keys() {
            if self.is_member(*g, nick) || (*g == HOME && legacy) {
                let e = Event::Member { guild: *g, nick: nick.into(), presence: Presence::Online };
                out.extend(self.to_members(*g, &e).into_iter().filter(|o| !matches!(o, Out::Send(to, _) if *to == id)));
            }
        }
        out
    }

    // ------------------------------------------------------------------ the old chat protocol

    fn legacy(&mut self, id: u32, line: &str, now: u64) -> Vec<Out> {
        let first = line.trim_start().get(..6).map(|w| w.eq_ignore_ascii_case("HELLO ")).unwrap_or(false);
        if first {
            if self.conns[&id].nick.is_some() {
                return vec![Out::Send(id, chat::Event::Err("HELLO must be the first line".into()).line())];
            }
            return match Cmd::parse(line) {
                Ok(Cmd::Hello(nick)) => self.hello(id, &nick, now),
                Ok(_) => Vec::new(),
                Err(e) => vec![Out::Send(id, Event::Err(e).line())],
            };
        }
        let req = match chat::parse_request(line) {
            Ok(r) => r,
            Err(e) => return vec![Out::Send(id, chat::Event::Err(e).line())],
        };
        let joined = self.conns.get(&id).and_then(|c| c.nick.clone());
        match (req, joined) {
            (chat::Request::Quit, _) => {
                let mut out = self.leave(id, "quit", now);
                out.push(Out::Close(id));
                out
            }
            (chat::Request::Nick(wanted), None) => self.legacy_join(id, &wanted, now),
            (chat::Request::Nick(wanted), Some(old)) => self.rename(id, &old, &wanted, now),
            (chat::Request::Msg(text), joined) => {
                let mut out = Vec::new();
                let nick = match joined {
                    Some(n) => n,
                    None => {
                        out.extend(self.legacy_join(id, &format!("guest{}", id), now));
                        self.conns[&id].nick.clone().unwrap_or_default()
                    }
                };
                match self.home_general() {
                    Some(cid) => out.extend(self.post(cid, &nick, &proto::clean_text(&text), now)),
                    None => out.push(Out::Send(id, chat::Event::Err("this server has no text channel".into()).line())),
                }
                out
            }
            (chat::Request::Who, _) => vec![Out::Send(id, chat::Event::Who(self.nicks()).line())],
        }
    }

    fn legacy_join(&mut self, id: u32, wanted: &str, now: u64) -> Vec<Out> {
        let nick = self.free_nick(wanted, id);
        let peer = match self.conns.get_mut(&id) {
            Some(c) => {
                c.nick = Some(nick.clone());
                c.peer.clone()
            }
            None => return Vec::new(),
        };
        let mut out = vec![Out::Send(id, chat::Event::Welcome { nick: nick.clone(), room: self.name.clone() }.line())];
        if let Some(cid) = self.home_general() {
            let ch = &self.channels[&cid];
            let skip = ch.messages.len().saturating_sub(LEGACY_HISTORY);
            for m in ch.messages.iter().skip(skip) {
                out.push(Out::Send(id, chat::Event::History { time: m.time.clone(), nick: m.nick.clone(), text: m.text.clone() }.line()));
            }
        }
        out.extend(self.arrived(id, &nick, now, true));
        out.push(Out::Log(format!("{} joined from {} (chat)", nick, peer)));
        out
    }

    // ------------------------------------------------------------------ Discord

    fn hello(&mut self, id: u32, wanted: &str, now: u64) -> Vec<Out> {
        let nick = self.free_nick(wanted, id);
        let peer = match self.conns.get_mut(&id) {
            Some(c) => {
                c.nick = Some(nick.clone());
                c.mode = Mode::Discord;
                c.peer.clone()
            }
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        // First visit: a member of the home server.
        if !self.is_member(HOME, &nick) {
            if let Some(home) = self.guilds.get_mut(&HOME) {
                home.members.push(nick.clone());
                out.push(Out::Save);
                out.extend(self.guild_to_all(HOME).into_iter().filter(|o| !matches!(o, Out::Send(to, _) if *to == id)));
            }
        }
        out.push(Out::Send(id, Event::Ready { nick: nick.clone(), backend: self.name.clone() }.line()));
        let guilds: Vec<u32> = self.guilds.keys().copied().collect();
        for g in &guilds {
            out.extend(self.guild_event(*g, &nick).map(|e| Out::Send(id, e.line())));
        }
        for g in &guilds {
            if self.is_member(*g, &nick) {
                out.extend(self.joined_picture(id, *g));
            }
        }
        // Old-protocol people in the home room are online there too.
        let legacy: Vec<String> =
            self.conns.values().filter(|c| c.mode == Mode::Legacy).filter_map(|c| c.nick.clone()).collect();
        for n in legacy {
            out.push(Out::Send(id, Event::Member { guild: HOME, nick: n, presence: Presence::Online }.line()));
        }
        out.push(Out::Send(id, Event::Synced.line()));
        out.extend(self.arrived(id, &nick, now, false));
        out.push(Out::Log(format!("{} joined from {}", nick, peer)));
        out
    }

    /// What a member of `guild` sees: its channels, its members, who is in voice.
    fn joined_picture(&self, id: u32, guild: u32) -> Vec<Out> {
        let mut out = Vec::new();
        for (cid, c) in &self.channels {
            if c.guild == guild {
                out.extend(self.channel_event(*cid).map(|e| Out::Send(id, e.line())));
            }
        }
        if let Some(g) = self.guilds.get(&guild) {
            for m in &g.members {
                let presence = if self.online(m) { Presence::Online } else { Presence::Offline };
                out.push(Out::Send(id, Event::Member { guild, nick: m.clone(), presence }.line()));
            }
        }
        for (sid, v) in &self.voice {
            if self.channels.get(&v.channel).is_some_and(|c| c.guild == guild) {
                out.push(Out::Send(id, Event::VoiceState { channel: v.channel, sid: *sid, nick: v.nick.clone(), event: VoiceEvent::Join }.line()));
                if v.speaking {
                    out.push(Out::Send(
                        id,
                        Event::VoiceState { channel: v.channel, sid: *sid, nick: v.nick.clone(), event: VoiceEvent::Speaking }.line(),
                    ));
                }
            }
        }
        out
    }

    fn err(id: u32, text: impl Into<String>) -> Vec<Out> {
        vec![Out::Send(id, Event::Err(text.into()).line())]
    }

    fn discord(&mut self, id: u32, line: &str, now: u64) -> Vec<Out> {
        let cmd = match Cmd::parse(line) {
            Ok(c) => c,
            Err(e) => return Self::err(id, e),
        };
        let nick = self.conns.get(&id).and_then(|c| c.nick.clone()).unwrap_or_default();
        match cmd {
            Cmd::Hello(_) => Self::err(id, "already connected"),
            Cmd::Ping => vec![Out::Send(id, Event::Pong.line())],
            Cmd::Quit => {
                let mut out = self.leave(id, "quit", now);
                out.push(Out::Close(id));
                out
            }
            Cmd::Nick(wanted) => self.rename(id, &nick, &wanted, now),
            Cmd::GuildCreate(name) => {
                if self.guilds.len() >= proto::MAX_GUILDS {
                    return Self::err(id, format!("this backend already hosts {} servers", proto::MAX_GUILDS));
                }
                let gid = self.next_guild;
                self.next_guild += 1;
                self.guilds.insert(gid, Guild { name: name.clone(), owner: nick.clone(), members: vec![nick.clone()] });
                self.add_channel(gid, Kind::Text, "general");
                self.add_channel(gid, Kind::Voice, "General");
                let mut out = vec![Out::Save];
                out.extend(self.guild_to_all(gid));
                out.extend(self.joined_picture(id, gid));
                out.push(Out::Log(format!("{} made the server \"{}\" ({})", nick, name, gid)));
                out
            }
            Cmd::GuildJoin(gid) => {
                if !self.guilds.contains_key(&gid) {
                    return Self::err(id, "no such server");
                }
                if self.is_member(gid, &nick) {
                    return Vec::new();
                }
                self.guilds.get_mut(&gid).unwrap().members.push(nick.clone());
                let mut out = vec![Out::Save];
                out.extend(self.guild_to_all(gid));
                out.extend(self.joined_picture(id, gid));
                let e = Event::Member { guild: gid, nick: nick.clone(), presence: Presence::Online };
                out.extend(self.to_members(gid, &e).into_iter().filter(|o| !matches!(o, Out::Send(to, _) if *to == id)));
                out.push(Out::Log(format!("{} joined the server \"{}\"", nick, self.guilds[&gid].name)));
                out
            }
            Cmd::GuildLeave(gid) => {
                let Some(g) = self.guilds.get(&gid) else { return Self::err(id, "no such server") };
                if same(&g.owner, &nick) {
                    return Self::err(id, "you own this server: delete it instead");
                }
                if !self.is_member(gid, &nick) {
                    return Vec::new();
                }
                let mut out = Vec::new();
                if self.conns[&id].sid.is_some_and(|s| self.voice.get(&s).is_some_and(|v| self.channels.get(&v.channel).is_some_and(|c| c.guild == gid)))
                {
                    out.extend(self.voice_leave(id));
                }
                out.extend(self.to_members(gid, &Event::Member { guild: gid, nick: nick.clone(), presence: Presence::Left }));
                self.guilds.get_mut(&gid).unwrap().members.retain(|m| !same(m, &nick));
                out.push(Out::Save);
                out.extend(self.guild_to_all(gid));
                out
            }
            Cmd::GuildDelete(gid) => {
                let Some(g) = self.guilds.get(&gid) else { return Self::err(id, "no such server") };
                if !same(&g.owner, &nick) {
                    return Self::err(id, "only the server's owner can delete it");
                }
                let mut out = Vec::new();
                let gone: Vec<u32> = self.channels.iter().filter(|(_, c)| c.guild == gid).map(|(c, _)| *c).collect();
                out.extend(self.drop_voice_in(&gone));
                for c in gone {
                    self.channels.remove(&c);
                }
                let name = self.guilds.remove(&gid).map(|g| g.name).unwrap_or_default();
                out.push(Out::Save);
                let line = Event::GuildGone(gid).line();
                out.extend(self.discord_conns().into_iter().map(|(c, _)| Out::Send(c, line.clone())));
                out.push(Out::Log(format!("{} deleted the server \"{}\"", nick, name)));
                out
            }
            Cmd::ChannelCreate { guild, kind, name } => {
                let Some(g) = self.guilds.get(&guild) else { return Self::err(id, "no such server") };
                if !same(&g.owner, &nick) {
                    return Self::err(id, "only the server's owner can make channels");
                }
                if self.channels.values().filter(|c| c.guild == guild).count() >= proto::MAX_CHANNELS {
                    return Self::err(id, format!("a server has at most {} channels", proto::MAX_CHANNELS));
                }
                if self.channels.values().any(|c| c.guild == guild && c.kind == kind && same(&c.name, &name)) {
                    return Self::err(id, format!("there already is a {} channel called {}", kind.word(), name));
                }
                let cid = self.add_channel(guild, kind, &name);
                let mut out = vec![Out::Save];
                out.extend(self.to_members(guild, &self.channel_event(cid).unwrap()));
                out.push(Out::Log(format!("{} made the {} channel {} in \"{}\"", nick, kind.word(), name, self.guilds[&guild].name)));
                out
            }
            Cmd::ChannelDelete(cid) => {
                let Some(c) = self.channels.get(&cid) else { return Self::err(id, "no such channel") };
                let guild = c.guild;
                if !same(&self.guilds[&guild].owner, &nick) {
                    return Self::err(id, "only the server's owner can delete channels");
                }
                let mut out = self.drop_voice_in(&[cid]);
                self.channels.remove(&cid);
                out.push(Out::Save);
                out.extend(self.to_members(guild, &Event::ChannelGone(cid)));
                out
            }
            Cmd::History(cid) => {
                let Some(c) = self.channels.get(&cid) else { return Self::err(id, "no such channel") };
                if !self.is_member(c.guild, &nick) {
                    return Self::err(id, "join the server first");
                }
                let mut out: Vec<Out> = c
                    .messages
                    .iter()
                    .map(|m| {
                        Out::Send(
                            id,
                            Event::Msg { channel: cid, id: m.id, time: m.time.clone(), nick: m.nick.clone(), text: m.text.clone(), earlier: true }
                                .line(),
                        )
                    })
                    .collect();
                out.push(Out::Send(id, Event::HistoryEnd(cid).line()));
                out
            }
            Cmd::Say { channel, text } => {
                let Some(c) = self.channels.get(&channel) else { return Self::err(id, "no such channel") };
                if c.kind != Kind::Text {
                    return Self::err(id, "that is a voice channel");
                }
                if !self.is_member(c.guild, &nick) {
                    return Self::err(id, "join the server first");
                }
                self.post(channel, &nick, &text, now)
            }
            Cmd::VoiceJoin(cid) => {
                let Some(c) = self.channels.get(&cid) else { return Self::err(id, "no such channel") };
                if c.kind != Kind::Voice {
                    return Self::err(id, "that is a text channel");
                }
                if !self.is_member(c.guild, &nick) {
                    return Self::err(id, "join the server first");
                }
                let guild = c.guild;
                let mut out = self.voice_leave(id);
                let sid = self.next_sid;
                self.next_sid += 1;
                let token = self.token();
                self.voice.insert(sid, Voice { conn: id, channel: cid, token, nick: nick.clone(), addr: None, last_audio: 0, speaking: false });
                if let Some(c) = self.conns.get_mut(&id) {
                    c.sid = Some(sid);
                }
                out.push(Out::Send(id, Event::Voice { channel: cid, port: self.voice_port, token, sid }.line()));
                out.extend(self.to_members(guild, &Event::VoiceState { channel: cid, sid, nick: nick.clone(), event: VoiceEvent::Join }));
                out.push(Out::Log(format!("{} joined the voice channel {}", nick, self.channels[&cid].name)));
                out
            }
            Cmd::VoiceLeave => self.voice_leave(id),
        }
    }

    /// Post a message in a text channel: kept, sent to the server's members (and to the
    /// old-protocol room when it is the home server's `#general`).
    fn post(&mut self, cid: u32, nick: &str, text: &str, now: u64) -> Vec<Out> {
        let time = chat::hms(now / 1000);
        let mid = self.next_msg;
        self.next_msg += 1;
        let Some(c) = self.channels.get_mut(&cid) else { return Vec::new() };
        c.messages.push_back(Message { id: mid, time: time.clone(), nick: nick.into(), text: text.into() });
        while c.messages.len() > proto::HISTORY {
            c.messages.pop_front();
        }
        let guild = c.guild;
        let name = c.name.clone();
        let mut out = vec![Out::Save];
        out.extend(self.to_members(
            guild,
            &Event::Msg { channel: cid, id: mid, time: time.clone(), nick: nick.into(), text: text.into(), earlier: false },
        ));
        if Some(cid) == self.home_general() {
            out.extend(self.to_legacy(&chat::Event::Msg { time, nick: nick.into(), text: text.into() }.line()));
        }
        out.push(Out::Log(format!("#{} <{}> {}", name, nick, text)));
        out
    }

    fn rename(&mut self, id: u32, old: &str, wanted: &str, now: u64) -> Vec<Out> {
        if wanted == old {
            return Vec::new();
        }
        if self.taken(wanted, id) {
            return vec![Out::Send(id, chat::Event::Err(format!("nickname {} is taken", wanted)).line())];
        }
        if let Some(c) = self.conns.get_mut(&id) {
            c.nick = Some(wanted.to_string());
        }
        for g in self.guilds.values_mut() {
            for m in g.members.iter_mut() {
                if same(m, old) {
                    *m = wanted.to_string();
                }
            }
            if same(&g.owner, old) {
                g.owner = wanted.to_string();
            }
        }
        for v in self.voice.values_mut() {
            if v.conn == id {
                v.nick = wanted.to_string();
            }
        }
        let mut out = vec![Out::Save];
        let line = Event::Renamed { old: old.into(), new: wanted.into() }.line();
        out.extend(self.discord_conns().into_iter().map(|(c, _)| Out::Send(c, line.clone())));
        out.extend(self.to_legacy(&chat::Event::Nick { time: chat::hms(now / 1000), old: old.into(), new: wanted.into() }.line()));
        out.push(Out::Log(format!("{} is now {}", old, wanted)));
        out
    }

    // ------------------------------------------------------------------ voice

    fn voice_leave(&mut self, id: u32) -> Vec<Out> {
        let Some(sid) = self.conns.get_mut(&id).and_then(|c| c.sid.take()) else { return Vec::new() };
        let Some(v) = self.voice.remove(&sid) else { return Vec::new() };
        let Some(guild) = self.channels.get(&v.channel).map(|c| c.guild) else { return Vec::new() };
        let mut out = self.to_members(guild, &Event::VoiceState { channel: v.channel, sid, nick: v.nick.clone(), event: VoiceEvent::Leave });
        // The leaver hears about it too (it may no longer be a member, after GLEAVE).
        if !out.iter().any(|o| matches!(o, Out::Send(to, _) if *to == id)) {
            out.push(Out::Send(id, Event::VoiceState { channel: v.channel, sid, nick: v.nick.clone(), event: VoiceEvent::Leave }.line()));
        }
        out.push(Out::Log(format!("{} left voice", v.nick)));
        out
    }

    fn drop_voice_in(&mut self, channels: &[u32]) -> Vec<Out> {
        let conns: Vec<u32> = self.voice.values().filter(|v| channels.contains(&v.channel)).map(|v| v.conn).collect();
        conns.into_iter().flat_map(|c| self.voice_leave(c)).collect()
    }

    /// A datagram arrived on the voice port from `ip:port`. `now` in milliseconds.
    pub fn udp(&mut self, ip: [u8; 4], port: u16, data: &[u8], now: u64) -> Vec<Out> {
        let Some(p) = Packet::decode(data) else { return Vec::new() };
        if p.kind == PacketKind::Forwarded {
            return Vec::new();
        }
        let Some((sid, channel)) = self.voice.iter().find(|(_, v)| v.token == p.id).map(|(s, v)| (*s, v.channel)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        {
            let v = self.voice.get_mut(&sid).unwrap();
            v.addr = Some((ip, port));
            if p.kind == PacketKind::KeepAlive {
                return out;
            }
            v.last_audio = now;
        }
        let bytes = p.forwarded(sid).encode();
        for (other, v) in &self.voice {
            if *other != sid && v.channel == channel {
                if let Some((ip, port)) = v.addr {
                    out.push(Out::Udp(ip, port, bytes.clone()));
                    self.forwarded += 1;
                }
            }
        }
        if !self.voice[&sid].speaking {
            self.voice.get_mut(&sid).unwrap().speaking = true;
            let v = &self.voice[&sid];
            if let Some(guild) = self.channels.get(&channel).map(|c| c.guild) {
                out.extend(self.to_members(guild, &Event::VoiceState { channel, sid, nick: v.nick.clone(), event: VoiceEvent::Speaking }));
            }
        }
        out
    }

    /// Time passes: talkers who stopped sending go quiet. `now` in milliseconds.
    pub fn tick(&mut self, now: u64) -> Vec<Out> {
        let quiet: Vec<u32> = self
            .voice
            .iter()
            .filter(|(_, v)| v.speaking && now.saturating_sub(v.last_audio) > SPEAKING_HOLD_MS)
            .map(|(s, _)| *s)
            .collect();
        let mut out = Vec::new();
        for sid in quiet {
            let v = self.voice.get_mut(&sid).unwrap();
            v.speaking = false;
            let (channel, nick) = (v.channel, v.nick.clone());
            if let Some(guild) = self.channels.get(&channel).map(|c| c.guild) {
                out.extend(self.to_members(guild, &Event::VoiceState { channel, sid, nick, event: VoiceEvent::Quiet }));
            }
        }
        out
    }

    /// Someone is talking (the program polls faster then).
    pub fn anyone_speaking(&self) -> bool {
        self.voice.values().any(|v| v.speaking)
    }
}

#[cfg(test)]
mod tests;
