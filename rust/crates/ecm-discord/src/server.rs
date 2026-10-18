//! A Discord backend (`discordd`'s logic) as a state machine.
//!
//! `discordd` owns the sockets: it calls [`Backend::connect`], [`Backend::receive`],
//! [`Backend::disconnect`] for TCP clients, [`Backend::udp`] for voice datagrams and
//! [`Backend::tick`] now and then, and performs the returned [`Out`] actions in
//! order. Clients are identified by any number the caller chooses (the socket fd).
//!
//! Accounts are required. The backend trusts one realm ([`AuthSetup`]): every
//! connection is greeted with `AUTH` and `BACKEND` lines (the realm, the backend's own
//! key fingerprint and a fresh nonce) and must sign in with a token for this backend
//! and the session key's proof for the nonce ([`ecm_auth::BackendGate`]) before
//! anything else. People are their realm user ids; what others see is their name
//! (a nickname set here with `NICK`, else the account's display name).
//!
//! The backend hosts servers ("guilds"), each with text and voice channels and
//! members. Server 1 is the backend's home server (named after the backend, owned
//! by nobody): every user joins it when they first sign in, and the old chat
//! protocol's room is its first text channel, so `chat` users (`LOGIN` instead of
//! `HELLO`, then `MSG`/`WHO`, shown by username) and Discord users talk together in
//! `#general`.
//!
//! What must survive a restart (users' names, servers, channels, members, recent
//! messages) is written as text by [`Backend::save_text`] whenever [`Out::Save`] is
//! returned.

use crate::proto::{self, Cmd, Event, Kind, Presence, VoiceEvent};
use crate::voice::{Packet, PacketKind, SPEAKING_HOLD_MS};
use ecm_auth::{BackendGate, RealmRef};
use ecm_chat::proto as chat;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Most clients one backend accepts (the kernel allows 32 sockets per program; the
/// backend uses a few itself).
pub const MAX_CLIENTS: usize = 26;
/// The home server's id.
pub const HOME: u32 = 1;
/// Old-protocol clients get this many messages of `#general` when they join.
pub const LEGACY_HISTORY: usize = 20;
/// The owner of the home server: nobody.
pub const NOBODY: u64 = 0;
/// Failed sign-ins on one connection before the backend hangs up.
pub const SIGN_IN_TRIES: u8 = 3;

/// Longest line a client may send (a `HELLO` with its token is about 600 bytes, more
/// than the old chat protocol's 400).
pub const MAX_LINE: usize = 2048;

/// Cut received bytes into lines (`\r` dropped); a longer line is cut at [`MAX_LINE`].
pub fn take_lines(buf: &mut Vec<u8>, data: &[u8]) -> Vec<String> {
    let mut lines = Vec::new();
    for &b in data {
        if b == b'\n' {
            lines.push(String::from_utf8_lossy(buf).trim_end_matches('\r').to_string());
            buf.clear();
        } else if buf.len() < MAX_LINE {
            buf.push(b);
        }
    }
    lines
}

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

/// Which realm the backend trusts, and who the backend is.
#[derive(Debug, Clone)]
pub struct AuthSetup {
    /// The realm, as the `AUTH` line names it (`-:port` for `--auth local`).
    pub realm: RealmRef,
    /// The realm's public key (its fingerprint is `realm.fp`).
    pub realm_pub: [u8; 32],
    /// The backend's own public key: tokens must name its fingerprint.
    pub backend_pub: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Greeted, not signed in yet.
    Pending,
    /// The old chat protocol (the `chat` command).
    Legacy,
    Discord,
}

#[derive(Debug)]
struct Conn {
    peer: String,
    buf: Vec<u8>,
    user: Option<u64>,
    mode: Mode,
    nonce: [u8; 32],
    tries: u8,
    /// Voice session, if in a voice channel.
    sid: Option<u32>,
}

/// A user this backend has seen.
#[derive(Debug, Clone)]
struct Account {
    username: String,
    display: String,
    nick: Option<String>,
}

#[derive(Debug, Clone)]
struct Guild {
    name: String,
    owner: u64,
    members: Vec<u64>,
}

#[derive(Debug, Clone)]
struct Message {
    id: u32,
    time: String,
    uid: u64,
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
    uid: u64,
    addr: Option<([u8; 4], u16)>,
    last_audio: u64,
    speaking: bool,
    /// Audio packets from this talker, and copies forwarded to the others, since it started talking.
    packets: u64,
    copies: u64,
}

/// One Discord backend.
pub struct Backend {
    pub name: String,
    pub voice_port: u16,
    auth: AuthSetup,
    gate: BackendGate,
    users: BTreeMap<u64, Account>,
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

impl Backend {
    /// A new backend with only its home server (`#general` and a `General` voice channel).
    /// `seed` makes the voice tokens unpredictable (the program passes random bits).
    pub fn new(name: &str, voice_port: u16, seed: u64, auth: AuthSetup) -> Backend {
        let gate = BackendGate::new(auth.realm_pub, auth.backend_pub);
        let mut b = Backend {
            name: name.to_string(),
            voice_port,
            auth,
            gate,
            users: BTreeMap::new(),
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
    pub fn load(name: &str, voice_port: u16, seed: u64, auth: AuthSetup, text: &str) -> Backend {
        let mut b = Backend::new(name, voice_port, seed, auth);
        b.guilds.clear();
        b.channels.clear();
        for line in text.lines() {
            let mut it = line.splitn(2, ' ');
            let (w, rest) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
            let p: Vec<&str> = rest.splitn(5, ' ').collect();
            let n = |i: usize| p.get(i).and_then(|s| s.parse::<u32>().ok());
            let u = |i: usize| p.get(i).and_then(|s| s.parse::<u64>().ok());
            let tail = |i: usize| rest.splitn(i + 1, ' ').nth(i).unwrap_or("").to_string();
            match w {
                "next" => {
                    if let (Some(g), Some(c), Some(m)) = (n(0), n(1), n(2)) {
                        b.next_guild = g;
                        b.next_channel = c;
                        b.next_msg = m;
                    }
                }
                "user" => {
                    if let (Some(uid), Some(username)) = (u(0), p.get(1)) {
                        b.users.insert(uid, Account { username: username.to_string(), display: tail(2), nick: None });
                    }
                }
                "nick" => {
                    if let Some(a) = u(0).and_then(|uid| b.users.get_mut(&uid)) {
                        a.nick = Some(tail(1)).filter(|s| !s.is_empty());
                    }
                }
                "guild" => {
                    if let (Some(id), Some(owner)) = (n(0), u(1)) {
                        b.guilds.insert(id, Guild { name: tail(2), owner, members: Vec::new() });
                    }
                }
                "member" => {
                    if let (Some(g), Some(uid)) = (n(0), u(1)) {
                        if let Some(guild) = b.guilds.get_mut(&g) {
                            if !guild.members.contains(&uid) {
                                guild.members.push(uid);
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
                    if let (Some(c), Some(id), Some(time), Some(uid)) = (n(0), n(1), p.get(2), u(3)) {
                        if let Some(ch) = b.channels.get_mut(&c) {
                            ch.messages.push_back(Message { id, time: time.to_string(), uid, text: tail(4) });
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
        let mut s = format!("discordd 2\nnext {} {} {}\n", self.next_guild, self.next_channel, self.next_msg);
        for (uid, a) in &self.users {
            s.push_str(&format!("user {} {} {}\n", uid, a.username, a.display));
            if let Some(n) = &a.nick {
                s.push_str(&format!("nick {} {}\n", uid, n));
            }
        }
        for (id, g) in &self.guilds {
            s.push_str(&format!("guild {} {} {}\n", id, g.owner, g.name));
            for m in &g.members {
                s.push_str(&format!("member {} {}\n", id, m));
            }
        }
        for (id, c) in &self.channels {
            s.push_str(&format!("channel {} {} {} {}\n", id, c.guild, c.kind.word(), c.name));
            for m in &c.messages {
                s.push_str(&format!("msg {} {} {} {} {}\n", id, m.id, m.time, m.uid, m.text));
            }
        }
        s
    }

    fn ensure_home(&mut self) {
        if self.guilds.contains_key(&HOME) {
            return;
        }
        self.guilds.insert(HOME, Guild { name: self.name.clone(), owner: NOBODY, members: Vec::new() });
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

    /// This backend's key fingerprint (the audience of its tokens).
    pub fn audience(&self) -> String {
        self.gate.audience()
    }

    /// The realm it trusts.
    pub fn realm(&self) -> &RealmRef {
        &self.auth.realm
    }

    /// Server names, by id.
    pub fn guild_names(&self) -> Vec<(u32, String)> {
        self.guilds.iter().map(|(id, g)| (*id, g.name.clone())).collect()
    }

    /// Channels of a server: (id, kind, name).
    pub fn channels_of(&self, guild: u32) -> Vec<(u32, Kind, String)> {
        self.channels.iter().filter(|(_, c)| c.guild == guild).map(|(id, c)| (*id, c.kind, c.name.clone())).collect()
    }

    /// The names of a server's members.
    pub fn members_of(&self, guild: u32) -> Vec<String> {
        self.guilds.get(&guild).map(|g| g.members.iter().map(|u| self.name_of(*u)).collect()).unwrap_or_default()
    }

    /// Texts in a channel, oldest first, as `<name> text`.
    pub fn messages_of(&self, channel: u32) -> Vec<String> {
        self.channels
            .get(&channel)
            .map(|c| c.messages.iter().map(|m| format!("<{}> {}", self.name_of(m.uid), m.text)).collect())
            .unwrap_or_default()
    }

    /// Who is in a voice channel (names).
    pub fn voice_in(&self, channel: u32) -> Vec<String> {
        self.voice.values().filter(|v| v.channel == channel).map(|v| self.name_of(v.uid)).collect()
    }

    /// The old chat room: the home server's first text channel.
    pub fn home_general(&self) -> Option<u32> {
        self.channels.iter().find(|(_, c)| c.guild == HOME && c.kind == Kind::Text).map(|(id, _)| *id)
    }

    /// Usernames signed in now, sorted, once each.
    pub fn online_users(&self) -> Vec<String> {
        let set: BTreeSet<String> = self.conns.values().filter_map(|c| c.user).map(|u| self.username(u)).collect();
        set.into_iter().collect()
    }

    /// What everyone sees for a user: the nickname, else the display name.
    pub fn name_of(&self, uid: u64) -> String {
        match self.users.get(&uid) {
            Some(a) => a.nick.clone().unwrap_or_else(|| a.display.clone()),
            None => format!("user {uid}"),
        }
    }

    fn username(&self, uid: u64) -> String {
        self.users.get(&uid).map(|a| a.username.clone()).unwrap_or_else(|| format!("user{uid}"))
    }

    fn online(&self, uid: u64) -> bool {
        self.conns.values().any(|c| c.user == Some(uid))
    }

    fn is_member(&self, guild: u32, uid: u64) -> bool {
        self.guilds.get(&guild).is_some_and(|g| g.members.contains(&uid))
    }

    // ------------------------------------------------------------------ event lines

    fn guild_event(&self, id: u32, for_uid: u64) -> Option<Event> {
        let g = self.guilds.get(&id)?;
        Some(Event::Guild { id, owner: g.owner, joined: g.members.contains(&for_uid), members: g.members.len() as u32, name: g.name.clone() })
    }

    fn channel_event(&self, id: u32) -> Option<Event> {
        let c = self.channels.get(&id)?;
        Some(Event::Channel { guild: c.guild, id, kind: c.kind, name: c.name.clone() })
    }

    fn member_event(&self, guild: u32, uid: u64, presence: Presence) -> Event {
        Event::Member { guild, uid, presence, name: self.name_of(uid) }
    }

    /// Discord connections (id, uid).
    fn discord_conns(&self) -> Vec<(u32, u64)> {
        self.conns.iter().filter(|(_, c)| c.mode == Mode::Discord).filter_map(|(id, c)| c.user.map(|u| (*id, u))).collect()
    }

    /// Send `e` to every Discord connection that is a member of `guild`.
    fn to_members(&self, guild: u32, e: &Event) -> Vec<Out> {
        let line = e.line();
        self.discord_conns().into_iter().filter(|(_, u)| self.is_member(guild, *u)).map(|(id, _)| Out::Send(id, line.clone())).collect()
    }

    /// Every Discord connection gets the server's line, with its own `joined` flag.
    fn guild_to_all(&self, guild: u32) -> Vec<Out> {
        self.discord_conns().into_iter().filter_map(|(id, u)| self.guild_event(guild, u).map(|e| Out::Send(id, e.line()))).collect()
    }

    /// Old-protocol connections that signed in.
    fn to_legacy(&self, line: &str) -> Vec<Out> {
        self.conns
            .iter()
            .filter(|(_, c)| c.mode == Mode::Legacy && c.user.is_some())
            .map(|(id, _)| Out::Send(*id, line.to_string()))
            .collect()
    }

    // ------------------------------------------------------------------ connections

    /// A client connected from `peer` ("a.b.c.d:port"): greet it with the realm and a nonce.
    pub fn connect(&mut self, id: u32, peer: &str) -> Vec<Out> {
        if self.conns.len() >= MAX_CLIENTS {
            return vec![
                Out::Send(id, chat::Event::Err(format!("server full ({} users)", MAX_CLIENTS)).line()),
                Out::Close(id),
                Out::Log(format!("refused {}: full", peer)),
            ];
        }
        let nonce = self.gate.challenge();
        self.conns.insert(id, Conn { peer: peer.into(), buf: Vec::new(), user: None, mode: Mode::Pending, nonce, tries: 0, sid: None });
        vec![
            Out::Send(id, Event::Auth(self.auth.realm.clone()).line()),
            Out::Send(id, Event::Backend { fp: self.audience(), nonce: ecm_auth::hex(&nonce), name: self.name.clone() }.line()),
            Out::Log(format!("connection from {}", peer)),
        ]
    }

    /// Bytes arrived from a client. `now` is Unix time in milliseconds.
    pub fn receive(&mut self, id: u32, data: &[u8], now: u64) -> Vec<Out> {
        let lines = match self.conns.get_mut(&id) {
            Some(c) => take_lines(&mut c.buf, data),
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let mode = self.conns[&id].mode;
            out.extend(match mode {
                Mode::Pending => self.sign_in(id, &line, now),
                Mode::Legacy => self.legacy(id, &line, now),
                Mode::Discord => self.discord(id, &line, now),
            });
            if !self.conns.contains_key(&id) || out.contains(&Out::Close(id)) {
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
        let Some(uid) = c.user else {
            out.push(Out::Log(format!("{} disconnected", c.peer)));
            return out;
        };
        let username = self.username(uid);
        if c.mode == Mode::Legacy {
            out.extend(self.to_legacy(&chat::Event::Leave { time: chat::hms(now / 1000), nick: username.clone(), reason: reason.into() }.line()));
        }
        if !self.online(uid) {
            let guilds: Vec<u32> = self.guilds.keys().copied().collect();
            for g in guilds {
                if self.is_member(g, uid) {
                    out.extend(self.to_members(g, &self.member_event(g, uid, Presence::Offline)));
                }
            }
        }
        out.push(Out::Log(format!("{} left ({})", username, reason)));
        out
    }

    /// Announce that `uid` came online (to old-protocol rooms and to the members of its servers).
    fn arrived(&self, id: u32, uid: u64, now: u64, legacy: bool) -> Vec<Out> {
        let mut out = Vec::new();
        if legacy {
            out.extend(self.to_legacy(&chat::Event::Join { time: chat::hms(now / 1000), nick: self.username(uid) }.line()));
        }
        for g in self.guilds.keys() {
            if self.is_member(*g, uid) {
                let e = self.member_event(*g, uid, Presence::Online);
                out.extend(self.to_members(*g, &e).into_iter().filter(|o| !matches!(o, Out::Send(to, _) if *to == id)));
            }
        }
        out
    }

    // ------------------------------------------------------------------ signing in

    fn sign_in(&mut self, id: u32, line: &str, now: u64) -> Vec<Out> {
        let (token, proof, chat_mode) = match Cmd::parse(line) {
            Ok(Cmd::Hello { token, proof }) => (token, proof, false),
            Ok(Cmd::Login { token, proof }) => (token, proof, true),
            Ok(Cmd::Quit) => {
                let mut out = self.leave(id, "quit", now);
                out.push(Out::Close(id));
                return out;
            }
            Ok(_) => return Self::err(id, "sign in first (HELLO <token> <proof>)"),
            Err(e) => return Self::err(id, e),
        };
        let nonce = self.conns[&id].nonce;
        let proof = ecm_auth::unhex(&proof).unwrap_or_default();
        let claims = match self.gate.check(&token, &proof, &nonce, now) {
            Ok(c) => c,
            Err(e) => {
                let c = self.conns.get_mut(&id).unwrap();
                c.tries += 1;
                let peer = c.peer.clone();
                let mut out = Self::err(id, format!("sign-in refused: {e}"));
                out.push(Out::Log(format!("refused a sign-in from {peer}: {e}")));
                if self.conns[&id].tries >= SIGN_IN_TRIES {
                    out.push(Out::Close(id));
                    self.conns.remove(&id);
                }
                return out;
            }
        };
        let uid = claims.user_id;
        let mut out = Vec::new();
        // The account's names as the realm says now.
        let renamed = match self.users.get_mut(&uid) {
            Some(a) => {
                let before = a.nick.clone().unwrap_or_else(|| a.display.clone());
                let changed = a.username != claims.username || a.display != claims.display_name;
                a.username = claims.username.clone();
                a.display = claims.display_name.clone();
                if changed {
                    out.push(Out::Save);
                }
                a.nick.is_none() && before != a.display
            }
            None => {
                self.users.insert(uid, Account { username: claims.username.clone(), display: claims.display_name.clone(), nick: None });
                out.push(Out::Save);
                false
            }
        };
        if renamed {
            let line = Event::Renamed { uid, name: self.name_of(uid) }.line();
            out.extend(self.discord_conns().into_iter().map(|(c, _)| Out::Send(c, line.clone())));
        }
        if let Some(c) = self.conns.get_mut(&id) {
            c.user = Some(uid);
            c.mode = if chat_mode { Mode::Legacy } else { Mode::Discord };
        }
        // First visit: a member of the home server.
        if !self.is_member(HOME, uid) {
            if let Some(home) = self.guilds.get_mut(&HOME) {
                home.members.push(uid);
                out.push(Out::Save);
                out.extend(self.guild_to_all(HOME).into_iter().filter(|o| !matches!(o, Out::Send(to, _) if *to == id)));
            }
        }
        out.extend(if chat_mode { self.legacy_join(id, uid, now) } else { self.hello(id, uid, now) });
        out
    }

    // ------------------------------------------------------------------ the old chat protocol

    fn legacy(&mut self, id: u32, line: &str, now: u64) -> Vec<Out> {
        let first = line.trim_start().get(..6).map(|w| w.eq_ignore_ascii_case("HELLO ") || w.eq_ignore_ascii_case("LOGIN ")).unwrap_or(false);
        if first {
            return vec![Out::Send(id, chat::Event::Err("already signed in".into()).line())];
        }
        let req = match chat::parse_request(line) {
            Ok(r) => r,
            Err(e) => return vec![Out::Send(id, chat::Event::Err(e).line())],
        };
        let uid = self.conns.get(&id).and_then(|c| c.user).unwrap_or(NOBODY);
        match req {
            chat::Request::Quit => {
                let mut out = self.leave(id, "quit", now);
                out.push(Out::Close(id));
                out
            }
            chat::Request::Nick(_) => vec![Out::Send(
                id,
                chat::Event::Err("chat shows your account's username; set a nickname in the Discord app".into()).line(),
            )],
            chat::Request::Msg(text) => match self.home_general() {
                Some(cid) => self.post(cid, uid, &proto::clean_text(&text), now),
                None => vec![Out::Send(id, chat::Event::Err("this server has no text channel".into()).line())],
            },
            chat::Request::Who => vec![Out::Send(id, chat::Event::Who(self.online_users()).line())],
        }
    }

    fn legacy_join(&mut self, id: u32, uid: u64, now: u64) -> Vec<Out> {
        let username = self.username(uid);
        let peer = self.conns.get(&id).map(|c| c.peer.clone()).unwrap_or_default();
        let mut out = vec![Out::Send(id, chat::Event::Welcome { nick: username.clone(), room: self.name.clone() }.line())];
        if let Some(cid) = self.home_general() {
            let ch = &self.channels[&cid];
            let skip = ch.messages.len().saturating_sub(LEGACY_HISTORY);
            for m in ch.messages.iter().skip(skip) {
                out.push(Out::Send(id, chat::Event::History { time: m.time.clone(), nick: self.username(m.uid), text: m.text.clone() }.line()));
            }
        }
        out.extend(self.arrived(id, uid, now, true));
        out.push(Out::Log(format!("{} signed in from {} (chat)", username, peer)));
        out
    }

    // ------------------------------------------------------------------ Discord

    fn hello(&mut self, id: u32, uid: u64, now: u64) -> Vec<Out> {
        let peer = self.conns.get(&id).map(|c| c.peer.clone()).unwrap_or_default();
        let mut out = vec![Out::Send(id, Event::Ready { uid, name: self.name_of(uid) }.line())];
        let guilds: Vec<u32> = self.guilds.keys().copied().collect();
        for g in &guilds {
            out.extend(self.guild_event(*g, uid).map(|e| Out::Send(id, e.line())));
        }
        for g in &guilds {
            if self.is_member(*g, uid) {
                out.extend(self.joined_picture(id, *g));
            }
        }
        out.push(Out::Send(id, Event::Synced.line()));
        out.extend(self.arrived(id, uid, now, false));
        out.push(Out::Log(format!("{} signed in from {}", self.username(uid), peer)));
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
                let presence = if self.online(*m) { Presence::Online } else { Presence::Offline };
                out.push(Out::Send(id, self.member_event(guild, *m, presence).line()));
            }
        }
        for (sid, v) in &self.voice {
            if self.channels.get(&v.channel).is_some_and(|c| c.guild == guild) {
                out.push(Out::Send(id, Event::VoiceState { channel: v.channel, sid: *sid, uid: v.uid, event: VoiceEvent::Join }.line()));
                if v.speaking {
                    out.push(Out::Send(id, Event::VoiceState { channel: v.channel, sid: *sid, uid: v.uid, event: VoiceEvent::Speaking }.line()));
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
        let uid = self.conns.get(&id).and_then(|c| c.user).unwrap_or(NOBODY);
        let who = self.name_of(uid);
        match cmd {
            Cmd::Hello { .. } | Cmd::Login { .. } => Self::err(id, "already signed in"),
            Cmd::Ping => vec![Out::Send(id, Event::Pong.line())],
            Cmd::Quit => {
                let mut out = self.leave(id, "quit", now);
                out.push(Out::Close(id));
                out
            }
            Cmd::Nick(n) => self.set_nick(uid, n),
            Cmd::GuildCreate(name) => {
                if self.guilds.len() >= proto::MAX_GUILDS {
                    return Self::err(id, format!("this backend already hosts {} servers", proto::MAX_GUILDS));
                }
                let gid = self.next_guild;
                self.next_guild += 1;
                self.guilds.insert(gid, Guild { name: name.clone(), owner: uid, members: vec![uid] });
                self.add_channel(gid, Kind::Text, "general");
                self.add_channel(gid, Kind::Voice, "General");
                let mut out = vec![Out::Save];
                out.extend(self.guild_to_all(gid));
                out.extend(self.joined_picture(id, gid));
                out.push(Out::Log(format!("{} made the server \"{}\" ({})", who, name, gid)));
                out
            }
            Cmd::GuildJoin(gid) => {
                if !self.guilds.contains_key(&gid) {
                    return Self::err(id, "no such server");
                }
                if self.is_member(gid, uid) {
                    return Vec::new();
                }
                self.guilds.get_mut(&gid).unwrap().members.push(uid);
                let mut out = vec![Out::Save];
                out.extend(self.guild_to_all(gid));
                out.extend(self.joined_picture(id, gid));
                let e = self.member_event(gid, uid, Presence::Online);
                out.extend(self.to_members(gid, &e).into_iter().filter(|o| !matches!(o, Out::Send(to, _) if *to == id)));
                out.push(Out::Log(format!("{} joined the server \"{}\"", who, self.guilds[&gid].name)));
                out
            }
            Cmd::GuildLeave(gid) => {
                let Some(g) = self.guilds.get(&gid) else { return Self::err(id, "no such server") };
                if g.owner == uid {
                    return Self::err(id, "you own this server: delete it instead");
                }
                if !self.is_member(gid, uid) {
                    return Vec::new();
                }
                let mut out = Vec::new();
                if self.conns[&id].sid.is_some_and(|s| self.voice.get(&s).is_some_and(|v| self.channels.get(&v.channel).is_some_and(|c| c.guild == gid)))
                {
                    out.extend(self.voice_leave(id));
                }
                out.extend(self.to_members(gid, &self.member_event(gid, uid, Presence::Left)));
                self.guilds.get_mut(&gid).unwrap().members.retain(|m| *m != uid);
                out.push(Out::Save);
                out.extend(self.guild_to_all(gid));
                out
            }
            Cmd::GuildDelete(gid) => {
                let Some(g) = self.guilds.get(&gid) else { return Self::err(id, "no such server") };
                if g.owner != uid || g.owner == NOBODY {
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
                out.push(Out::Log(format!("{} deleted the server \"{}\"", who, name)));
                out
            }
            Cmd::ChannelCreate { guild, kind, name } => {
                let Some(g) = self.guilds.get(&guild) else { return Self::err(id, "no such server") };
                if g.owner != uid {
                    return Self::err(id, "only the server's owner can make channels");
                }
                if self.channels.values().filter(|c| c.guild == guild).count() >= proto::MAX_CHANNELS {
                    return Self::err(id, format!("a server has at most {} channels", proto::MAX_CHANNELS));
                }
                if self.channels.values().any(|c| c.guild == guild && c.kind == kind && c.name.eq_ignore_ascii_case(&name)) {
                    return Self::err(id, format!("there already is a {} channel called {}", kind.word(), name));
                }
                let cid = self.add_channel(guild, kind, &name);
                let mut out = vec![Out::Save];
                out.extend(self.to_members(guild, &self.channel_event(cid).unwrap()));
                out.push(Out::Log(format!("{} made the {} channel {} in \"{}\"", who, kind.word(), name, self.guilds[&guild].name)));
                out
            }
            Cmd::ChannelDelete(cid) => {
                let Some(c) = self.channels.get(&cid) else { return Self::err(id, "no such channel") };
                let guild = c.guild;
                if self.guilds[&guild].owner != uid {
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
                if !self.is_member(c.guild, uid) {
                    return Self::err(id, "join the server first");
                }
                // Who wrote them first (some may have left the server since).
                let authors: BTreeSet<u64> = c.messages.iter().map(|m| m.uid).collect();
                let mut out: Vec<Out> = authors
                    .into_iter()
                    .map(|u| Out::Send(id, Event::User { uid: u, username: self.username(u), name: self.name_of(u) }.line()))
                    .collect();
                out.extend(c.messages.iter().map(|m| {
                    Out::Send(id, Event::Msg { channel: cid, id: m.id, time: m.time.clone(), uid: m.uid, text: m.text.clone(), earlier: true }.line())
                }));
                out.push(Out::Send(id, Event::HistoryEnd(cid).line()));
                out
            }
            Cmd::Say { channel, text } => {
                let Some(c) = self.channels.get(&channel) else { return Self::err(id, "no such channel") };
                if c.kind != Kind::Text {
                    return Self::err(id, "that is a voice channel");
                }
                if !self.is_member(c.guild, uid) {
                    return Self::err(id, "join the server first");
                }
                self.post(channel, uid, &text, now)
            }
            Cmd::VoiceJoin(cid) => {
                let Some(c) = self.channels.get(&cid) else { return Self::err(id, "no such channel") };
                if c.kind != Kind::Voice {
                    return Self::err(id, "that is a text channel");
                }
                if !self.is_member(c.guild, uid) {
                    return Self::err(id, "join the server first");
                }
                let guild = c.guild;
                let mut out = self.voice_leave(id);
                let sid = self.next_sid;
                self.next_sid += 1;
                let token = self.token();
                self.voice.insert(sid, Voice { conn: id, channel: cid, token, uid, addr: None, last_audio: 0, speaking: false, packets: 0, copies: 0 });
                if let Some(c) = self.conns.get_mut(&id) {
                    c.sid = Some(sid);
                }
                out.push(Out::Send(id, Event::Voice { channel: cid, port: self.voice_port, token, sid }.line()));
                out.extend(self.to_members(guild, &Event::VoiceState { channel: cid, sid, uid, event: VoiceEvent::Join }));
                out.push(Out::Log(format!("{} joined the voice channel {}", who, self.channels[&cid].name)));
                out
            }
            Cmd::VoiceLeave => self.voice_leave(id),
        }
    }

    /// Post a message in a text channel: kept, sent to the server's members (and to the
    /// old-protocol room when it is the home server's `#general`).
    fn post(&mut self, cid: u32, uid: u64, text: &str, now: u64) -> Vec<Out> {
        let time = chat::hms(now / 1000);
        let mid = self.next_msg;
        self.next_msg += 1;
        let Some(c) = self.channels.get_mut(&cid) else { return Vec::new() };
        c.messages.push_back(Message { id: mid, time: time.clone(), uid, text: text.into() });
        while c.messages.len() > proto::HISTORY {
            c.messages.pop_front();
        }
        let guild = c.guild;
        let name = c.name.clone();
        let mut out = vec![Out::Save];
        out.extend(self.to_members(guild, &Event::Msg { channel: cid, id: mid, time: time.clone(), uid, text: text.into(), earlier: false }));
        if Some(cid) == self.home_general() {
            out.extend(self.to_legacy(&chat::Event::Msg { time, nick: self.username(uid), text: text.into() }.line()));
        }
        out.push(Out::Log(format!("#{} <{}> {}", name, self.username(uid), text)));
        out
    }

    /// `NICK`: the name this backend shows for the user (None: the display name again).
    fn set_nick(&mut self, uid: u64, nick: Option<String>) -> Vec<Out> {
        let Some(a) = self.users.get_mut(&uid) else { return Vec::new() };
        if a.nick == nick {
            return Vec::new();
        }
        let old = a.nick.clone().unwrap_or_else(|| a.display.clone());
        a.nick = nick;
        let name = self.name_of(uid);
        let mut out = vec![Out::Save];
        let line = Event::Renamed { uid, name: name.clone() }.line();
        out.extend(self.discord_conns().into_iter().map(|(c, _)| Out::Send(c, line.clone())));
        out.push(Out::Log(format!("{} is now {}", old, name)));
        out
    }

    // ------------------------------------------------------------------ voice

    fn voice_leave(&mut self, id: u32) -> Vec<Out> {
        let Some(sid) = self.conns.get_mut(&id).and_then(|c| c.sid.take()) else { return Vec::new() };
        let Some(v) = self.voice.remove(&sid) else { return Vec::new() };
        let Some(guild) = self.channels.get(&v.channel).map(|c| c.guild) else { return Vec::new() };
        let e = Event::VoiceState { channel: v.channel, sid, uid: v.uid, event: VoiceEvent::Leave };
        let mut out = self.to_members(guild, &e);
        // The leaver hears about it too (it may no longer be a member, after GLEAVE).
        if !out.iter().any(|o| matches!(o, Out::Send(to, _) if *to == id)) {
            out.push(Out::Send(id, e.line()));
        }
        out.push(Out::Log(format!("{} left voice", self.name_of(v.uid))));
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
        let mut copies = 0;
        for (other, v) in &self.voice {
            if *other != sid && v.channel == channel {
                if let Some((ip, port)) = v.addr {
                    out.push(Out::Udp(ip, port, bytes.clone()));
                    copies += 1;
                }
            }
        }
        self.forwarded += copies;
        {
            let v = self.voice.get_mut(&sid).unwrap();
            v.packets += 1;
            v.copies += copies;
        }
        if !self.voice[&sid].speaking {
            self.voice.get_mut(&sid).unwrap().speaking = true;
            let uid = self.voice[&sid].uid;
            let cname = self.channels.get(&channel).map(|c| c.name.clone()).unwrap_or_default();
            out.push(Out::Log(format!("{} is talking in {}", self.name_of(uid), cname)));
            if let Some(guild) = self.channels.get(&channel).map(|c| c.guild) {
                out.extend(self.to_members(guild, &Event::VoiceState { channel, sid, uid, event: VoiceEvent::Speaking }));
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
            let (channel, uid, packets, copies) = (v.channel, v.uid, v.packets, v.copies);
            v.packets = 0;
            v.copies = 0;
            out.push(Out::Log(format!("{} stopped talking ({} packets, {} forwarded)", self.name_of(uid), packets, copies)));
            if let Some(guild) = self.channels.get(&channel).map(|c| c.guild) {
                out.extend(self.to_members(guild, &Event::VoiceState { channel, sid, uid, event: VoiceEvent::Quiet }));
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
