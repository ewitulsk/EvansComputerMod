//! Everything Discord shows, as plain state: the backends it talks to (one
//! connection each), their servers, channels, members and messages, the voice
//! channel it is in, the sign-in state of each backend, and which page is open. Fed
//! `ecm_discord::proto::Event`s, read by the view. No I/O, so every rule is tested
//! with `cargo test -p discord`.
//!
//! Seamless backends: the server rail lists the servers the player joined on every
//! backend at once, so which computer hosts a server only matters when making one
//! (and is shown in its details).
//!
//! Accounts (DISCORD/2): each backend greets with the realm it trusts; the driver
//! (app.rs) signs in there with the app's own saved logins, or shows the shared
//! sign-in screen ([`ecm_apps_ui::signin`]) for that backend, and sends `HELLO` with
//! a token. People are user ids; [`Backend::name_of`] is what to show.

use ecm_apps_ui::signin::SignInScreen;
use ecm_discord::proto::{self, Cmd, Event, Greeting, Kind, Presence, VoiceEvent, DEFAULT_PORT};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// Messages kept per channel.
pub const MAX_MESSAGES: usize = 200;

/// Where a backend came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Found on the network (docs/DISCOVERY.md).
    Found,
    /// `/etc/chat.conf` (Tech Village computers).
    Config,
    /// Saved from an earlier session.
    Saved,
    /// Typed by the player on the Add page.
    Typed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// Not connected yet (the driver connects it).
    Idle,
    Connecting,
    /// Connected; `synced` once the first picture (`SYNCED`) arrived.
    Up { synced: bool },
    /// It failed or closed: why. The driver retries now and then.
    Down(String),
}

/// Signing in to one backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    /// Connected, waiting for the greeting (`AUTH`, `BACKEND`).
    Greeting,
    /// The greeting arrived: the driver signs in (a saved login, or the sign-in screen).
    Ready,
    /// The driver is talking to the realm.
    Working,
    /// Waiting for the player on the sign-in screen (also: a changed key, refused).
    NeedsSignIn,
    /// `HELLO` sent, waiting for `READY`.
    Hello,
    SignedIn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guild {
    pub name: String,
    /// The owner's user id (0: nobody, the home server).
    pub owner: u64,
    pub joined: bool,
    pub members: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: u32,
    pub time: String,
    pub uid: u64,
    pub text: String,
    pub mine: bool,
    /// From the history (sent before we looked).
    pub earlier: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    pub guild: u32,
    pub kind: Kind,
    pub name: String,
    pub messages: Vec<Message>,
    /// `HISTORY` was asked for (once per connection).
    pub history_asked: bool,
    pub unread: usize,
}

/// Someone in a voice channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InVoice {
    pub channel: u32,
    pub uid: u64,
    pub speaking: bool,
}

/// The account we use on a backend (from the app's saved login).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub realm: String,
    pub realm_fp: String,
    pub username: String,
    pub display: String,
}

/// One Discord backend (a computer running `discordd`).
#[derive(Clone, Debug)]
pub struct Backend {
    pub host: String,
    pub port: u16,
    /// Its name (from discovery, then from its greeting).
    pub name: String,
    pub origin: Origin,
    pub link: Link,
    pub auth: Auth,
    /// The greeting: the realm it trusts, its key fingerprint, this connection's nonce.
    pub greeting: Greeting,
    /// The sign-in screen for this backend (no login yet, expired, or a changed key).
    pub screen: Option<Rc<RefCell<SignInScreen>>>,
    /// Signed in as.
    pub account: Option<Account>,
    /// Our user id there.
    pub me: u64,
    /// Names by user id (`READY`, `MEMBER`, `USER`, `RENAME`).
    pub names: BTreeMap<u64, String>,
    pub guilds: BTreeMap<u32, Guild>,
    pub channels: BTreeMap<u32, Channel>,
    /// Members of the servers we joined: (user id, presence), online first.
    pub members: BTreeMap<u32, Vec<(u64, Presence)>>,
    /// Voice sessions by sid.
    pub voice: BTreeMap<u32, InVoice>,
    /// Lines for the backend (the driver sends them).
    pub outbox: Vec<String>,
    /// When the driver last tried to connect (ms), for retries.
    pub tried_ms: u64,
    /// Settings: the nickname field.
    pub nick_draft: String,
    /// The backend refused our `HELLO` (why): the driver shows the sign-in screen.
    pub refused: Option<String>,
}

impl Backend {
    pub fn new(host: &str, port: u16, name: &str, origin: Origin) -> Backend {
        Backend {
            host: host.into(),
            port,
            name: if name.is_empty() { host.to_string() } else { name.to_string() },
            origin,
            link: Link::Idle,
            auth: Auth::Greeting,
            greeting: Greeting::default(),
            screen: None,
            account: None,
            me: 0,
            names: BTreeMap::new(),
            guilds: BTreeMap::new(),
            channels: BTreeMap::new(),
            members: BTreeMap::new(),
            voice: BTreeMap::new(),
            outbox: Vec::new(),
            tried_ms: 0,
            nick_draft: String::new(),
            refused: None,
        }
    }

    /// "10.0.8.1" or "10.0.8.1:7000".
    pub fn endpoint(&self) -> String {
        if self.port == DEFAULT_PORT {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    pub fn up(&self) -> bool {
        matches!(self.link, Link::Up { .. })
    }

    /// Connected and signed in: requests can go.
    pub fn ready(&self) -> bool {
        self.up() && self.auth == Auth::SignedIn
    }

    /// The player has to sign in here (the rail shows it).
    pub fn needs_sign_in(&self) -> bool {
        self.up() && self.auth == Auth::NeedsSignIn
    }

    /// What to show for user `uid`.
    pub fn name_of(&self, uid: u64) -> String {
        self.names.get(&uid).cloned().unwrap_or_else(|| format!("user {uid}"))
    }

    /// Our name there.
    pub fn my_name(&self) -> String {
        if self.me == 0 {
            String::new()
        } else {
            self.name_of(self.me)
        }
    }

    /// The realm this backend trusts (from its greeting).
    pub fn realm_fp(&self) -> Option<String> {
        self.greeting.realm.as_ref().map(|r| r.fp.clone())
    }

    /// Channels of a server of one kind, by id.
    pub fn channels_of(&self, guild: u32, kind: Kind) -> Vec<(u32, &Channel)> {
        self.channels.iter().filter(|(_, c)| c.guild == guild && c.kind == kind).map(|(id, c)| (*id, c)).collect()
    }

    /// Who is in a voice channel.
    pub fn in_voice(&self, channel: u32) -> Vec<(u32, &InVoice)> {
        self.voice.iter().filter(|(_, v)| v.channel == channel).map(|(s, v)| (*s, v)).collect()
    }

    fn forget_connection(&mut self) {
        self.guilds.clear();
        self.channels.clear();
        self.members.clear();
        self.voice.clear();
        self.outbox.clear();
        self.greeting = Greeting::default();
        self.auth = Auth::Greeting;
    }
}

/// A server on the rail: which backend, which guild.
pub type GuildRef = (usize, u32);

/// The open page (compact layouts show one at a time; wider ones show Servers and Chat side by side).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    /// The rail and the open server's channels.
    Servers,
    /// A text channel's messages.
    Chat,
    /// The open server's members.
    Members,
    /// "Add a server": make one, join one, the backends.
    Add,
    Create,
    Join,
    /// Make a channel in the open server (its owner).
    NewChannel,
    /// Your accounts and nicknames, per backend.
    Settings,
    /// The sign-in screen of a backend.
    SignIn(usize),
}

/// The voice channel we are in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceSession {
    pub backend: usize,
    pub channel: u32,
    pub port: u16,
    pub token: u32,
    pub sid: u32,
}

/// A notification to post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub title: String,
    pub body: String,
}

/// Something in Settings that the driver does with the saved logins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountRequest {
    /// Sign out of a realm (revoke the refresh token, forget the login).
    SignOut(String),
    /// Remove a realm's pin by hand (and its login): the only way past a changed key.
    ForgetRealm(String),
}

#[derive(Clone, Debug)]
pub struct Model {
    pub backends: Vec<Backend>,
    /// The server open on the rail.
    pub guild: Option<GuildRef>,
    /// The text channel open in it.
    pub channel: Option<u32>,
    pub page: Page,
    /// The voice channel we are in, once the backend answered `VOICE`.
    pub voice: Option<VoiceSession>,
    /// We asked to join this voice channel (backend, channel) and wait for `VOICE`.
    pub voice_wanted: Option<(usize, u32)>,
    pub muted: bool,
    pub deafened: bool,
    /// Our microphone is being heard (the gate is open).
    pub talking: bool,
    /// Audio goes to headphones (AirPods), not the loudspeaker.
    pub headphones: bool,
    /// Messages from others while the window wasn't on screen.
    pub unread: usize,
    pub on_screen: bool,
    /// Bumped when a message is added to the open channel (the view scrolls to the end).
    pub serial: u64,
    /// The driver searches the network when set.
    pub search_wanted: bool,
    pub searching: bool,
    pub searched: bool,
    /// A line for the player (errors, progress).
    pub notice: Option<String>,
    /// Forms: Create (name, which backend), New channel (name, kind), Add (address).
    pub new_server: String,
    pub new_server_backend: usize,
    pub new_channel: String,
    pub new_channel_kind: Kind,
    pub add_address: String,
    /// Settings' sign out / forget, for the driver.
    pub account_requests: Vec<AccountRequest>,
    /// A server we made or joined, to open as soon as the backend shows it ours: (backend, name).
    pub open_when_made: Option<(usize, String)>,
    /// The saved settings' open server, opened once its backend shows it.
    pub reopen: Option<(String, u16, u32)>,
    /// Image resources the app uploaded (0 = none): the Discord logo, the voice-channel speaker.
    pub logo: u32,
    pub speaker: u32,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            backends: Vec::new(),
            guild: None,
            channel: None,
            page: Page::Servers,
            voice: None,
            voice_wanted: None,
            muted: false,
            deafened: false,
            talking: false,
            headphones: false,
            unread: 0,
            on_screen: true,
            serial: 0,
            search_wanted: false,
            searching: false,
            searched: false,
            notice: None,
            new_server: String::new(),
            new_server_backend: 0,
            new_channel: String::new(),
            new_channel_kind: Kind::Text,
            add_address: String::new(),
            account_requests: Vec::new(),
            open_when_made: None,
            reopen: None,
            logo: 0,
            speaker: 0,
        }
    }
}

/// `/etc/chat.conf` (Tech Village computers): the village's backend.
pub const CHAT_CONF: &str = "/etc/chat.conf";

impl Model {
    // ------------------------------------------------------------------ backends

    /// Add a backend unless it is already listed (same address and port); its index.
    pub fn add_backend(&mut self, host: &str, port: u16, name: &str, origin: Origin) -> usize {
        if let Some(i) = self.backends.iter().position(|b| b.host == host && b.port == port) {
            let b = &mut self.backends[i];
            if !name.is_empty() && !b.up() {
                b.name = name.to_string();
            }
            return i;
        }
        self.backends.push(Backend::new(host, port, name, origin));
        self.backends.len() - 1
    }

    /// The Add page's "Add backend": `host` or `host:port`.
    pub fn add_typed_backend(&mut self) -> Result<usize, String> {
        let text = self.add_address.trim().to_string();
        if text.is_empty() {
            return Err("Type the address of a computer running discordd".into());
        }
        let (host, port) = ecm_chat::client::split_host_port(&text)?;
        if host.is_empty() {
            return Err("Type the address of a computer running discordd".into());
        }
        let i = self.add_backend(&host, port.unwrap_or(DEFAULT_PORT), "", Origin::Typed);
        self.add_address.clear();
        self.notice = Some(format!("Connecting to {} ...", self.backends[i].endpoint()));
        Ok(i)
    }

    /// Settings saved between sessions: the backends we reached, the open server. (The
    /// logins are the app's accounts folder, not here.)
    pub fn settings_text(&self) -> String {
        let mut s = String::new();
        for b in &self.backends {
            if matches!(b.origin, Origin::Typed | Origin::Saved) || b.up() {
                s.push_str(&format!("backend {}:{} {}\n", b.host, b.port, b.name));
            }
        }
        if let Some((bi, g)) = self.guild {
            if let Some(b) = self.backends.get(bi) {
                s.push_str(&format!("open {}:{} {}\n", b.host, b.port, g));
            }
        }
        s
    }

    /// Load saved settings and `/etc/chat.conf` (its backend).
    pub fn load(&mut self, saved: Option<&str>, chat_conf: Option<&str>) {
        if let Some(conf) = chat_conf {
            let c = ecm_chat::client::parse_config(conf);
            if let Some(server) = c.server {
                self.add_backend(&server, c.port.unwrap_or(DEFAULT_PORT), "", Origin::Config);
            }
        }
        for line in saved.unwrap_or("").lines() {
            let mut it = line.splitn(2, ' ');
            match (it.next(), it.next()) {
                (Some("backend"), Some(rest)) => {
                    let (addr, name) = rest.split_once(' ').unwrap_or((rest, ""));
                    if let Ok((host, Some(port))) = ecm_chat::client::split_host_port(addr) {
                        self.add_backend(&host, port, name, Origin::Saved);
                    }
                }
                (Some("open"), Some(rest)) => {
                    if let Some((addr, g)) = rest.split_once(' ') {
                        if let (Ok((host, Some(port))), Ok(g)) = (ecm_chat::client::split_host_port(addr), g.trim().parse()) {
                            self.reopen = Some((host, port, g));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Look for backends on the network (the driver does it between turns).
    pub fn search(&mut self) {
        if !self.searching {
            self.search_wanted = true;
            self.searching = true;
        }
    }

    /// The search finished: new backends are added (and connected by the driver).
    pub fn on_found(&mut self, found: Vec<ecm_disco::Found>) {
        self.searching = false;
        self.searched = true;
        for f in found {
            self.add_backend(&f.address(), f.port, &f.name, Origin::Found);
        }
    }

    /// The driver is connecting backend `i`.
    pub fn on_connecting(&mut self, i: usize, now: u64) {
        if let Some(b) = self.backends.get_mut(i) {
            b.link = Link::Connecting;
            b.tried_ms = now;
        }
    }

    /// Backend `i` is connected: wait for its greeting.
    pub fn on_connected(&mut self, i: usize) {
        if let Some(b) = self.backends.get_mut(i) {
            b.forget_connection();
            b.link = Link::Up { synced: false };
        }
    }

    /// Backend `i` failed or closed.
    pub fn on_down(&mut self, i: usize, why: &str) {
        if self.voice.as_ref().is_some_and(|v| v.backend == i) || self.voice_wanted.is_some_and(|(b, _)| b == i) {
            self.voice = None;
            self.voice_wanted = None;
        }
        if let Some(b) = self.backends.get_mut(i) {
            b.forget_connection();
            b.link = Link::Down(why.to_string());
        }
        if self.page == Page::SignIn(i) {
            self.page = Page::Servers;
        }
        if self.guild.is_some_and(|(b, _)| b == i) {
            self.guild = None;
            self.channel = None;
            if matches!(self.page, Page::Chat | Page::Members | Page::NewChannel) {
                self.page = Page::Servers;
            }
            self.pick_default();
        }
    }

    // ------------------------------------------------------------------ signing in

    /// The driver made a sign-in screen for backend `i` (no login, expired, changed key).
    pub fn show_sign_in(&mut self, i: usize, screen: SignInScreen) {
        if let Some(b) = self.backends.get_mut(i) {
            b.auth = Auth::NeedsSignIn;
            b.screen = Some(Rc::new(RefCell::new(screen)));
        }
    }

    /// Open backend `i`'s sign-in screen (the rail's Sign in).
    pub fn open_sign_in(&mut self, i: usize) {
        if self.backends.get(i).is_some_and(|b| b.screen.is_some()) {
            self.page = Page::SignIn(i);
            self.notice = None;
        }
    }

    /// The driver has a token for backend `i`: say hello.
    pub fn hello(&mut self, i: usize, line: String, account: Account) {
        if let Some(b) = self.backends.get_mut(i) {
            b.outbox.push(line);
            b.auth = Auth::Hello;
            b.account = Some(account);
        }
    }

    /// Backends that need the player (the rail's Sign in buttons).
    pub fn sign_ins(&self) -> Vec<usize> {
        (0..self.backends.len()).filter(|&i| self.backends[i].needs_sign_in()).collect()
    }

    // ------------------------------------------------------------------ the rail

    /// The servers we joined, every backend's, in rail order.
    pub fn rail(&self) -> Vec<(GuildRef, &Guild)> {
        let mut out = Vec::new();
        for (bi, b) in self.backends.iter().enumerate() {
            for (g, guild) in &b.guilds {
                if guild.joined {
                    out.push(((bi, *g), guild));
                }
            }
        }
        out
    }

    /// Servers we could join, on every connected backend.
    pub fn joinable(&self) -> Vec<(GuildRef, &Guild)> {
        let mut out = Vec::new();
        for (bi, b) in self.backends.iter().enumerate() {
            for (g, guild) in &b.guilds {
                if !guild.joined {
                    out.push(((bi, *g), guild));
                }
            }
        }
        out
    }

    pub fn guild_of(&self, r: GuildRef) -> Option<&Guild> {
        self.backends.get(r.0)?.guilds.get(&r.1)
    }

    /// The open server and its backend.
    pub fn current(&self) -> Option<(&Backend, &Guild)> {
        let (bi, g) = self.guild?;
        let b = self.backends.get(bi)?;
        Some((b, b.guilds.get(&g)?))
    }

    /// The open text channel.
    pub fn current_channel(&self) -> Option<&Channel> {
        let (bi, _) = self.guild?;
        self.backends.get(bi)?.channels.get(&self.channel?)
    }

    /// We own the open server.
    pub fn owns_current(&self) -> bool {
        self.current().is_some_and(|(b, g)| g.owner != 0 && g.owner == b.me)
    }

    /// Open a server from the rail and its first text channel (the page stays on the server).
    pub fn open_guild(&mut self, r: GuildRef) {
        if self.guild_of(r).is_none_or(|g| !g.joined) {
            return;
        }
        self.guild = Some(r);
        self.channel = None;
        let first = self.backends[r.0].channels_of(r.1, Kind::Text).first().map(|(id, _)| *id);
        if let Some(c) = first {
            self.select_channel(c);
        }
        self.page = Page::Servers;
        self.notice = None;
    }

    /// Open a text channel of the open server (its messages page).
    pub fn open_channel(&mut self, c: u32) {
        if self.select_channel(c) {
            self.page = Page::Chat;
        }
    }

    /// Make `c` the open text channel (asks for its history the first time).
    fn select_channel(&mut self, c: u32) -> bool {
        let Some((bi, g)) = self.guild else { return false };
        let b = &mut self.backends[bi];
        let Some(ch) = b.channels.get_mut(&c) else { return false };
        if ch.guild != g || ch.kind != Kind::Text {
            return false;
        }
        ch.unread = 0;
        if !ch.history_asked {
            ch.history_asked = true;
            b.outbox.push(Cmd::History(c).line());
        }
        self.channel = Some(c);
        self.serial += 1;
        true
    }

    /// Nothing open (a first start, or the open server went away): the saved one, else the first on the rail.
    pub fn pick_default(&mut self) {
        if self.guild.is_some() {
            return;
        }
        if let Some((host, port, g)) = self.reopen.clone() {
            if let Some(bi) = self.backends.iter().position(|b| b.host == host && b.port == port && b.up()) {
                if self.guild_of((bi, g)).is_some_and(|x| x.joined) {
                    self.reopen = None;
                    self.open_guild((bi, g));
                    return;
                }
            }
        }
        let first = self.rail().first().map(|(r, _)| *r);
        if let Some(r) = first {
            let page = self.page;
            self.open_guild(r);
            if page != Page::Servers {
                self.page = page;
            }
        }
    }

    pub fn back(&mut self) {
        self.page = Page::Servers;
        self.notice = None;
    }

    // ------------------------------------------------------------------ requests

    fn send(&mut self, bi: usize, cmd: Cmd) -> Result<(), String> {
        let b = self.backends.get_mut(bi).ok_or("That backend is gone")?;
        if !b.up() {
            return Err(format!("Not connected to {}", b.name));
        }
        if b.auth != Auth::SignedIn {
            return Err(format!("Sign in to {} first", b.name));
        }
        b.outbox.push(cmd.line());
        Ok(())
    }

    /// The composer: post in the open channel (`/nick NAME` sets your nickname on its backend).
    pub fn compose(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if let Some(n) = text.strip_prefix("/nick").filter(|r| r.is_empty() || r.starts_with(' ')) {
            let Some((bi, _)) = self.guild else {
                self.notice = Some("Open a server first".into());
                return;
            };
            if let Err(e) = self.set_nick(bi, n) {
                self.notice = Some(e);
            }
            return;
        }
        let (Some((bi, _)), Some(c)) = (self.guild, self.channel) else {
            self.notice = Some("Open a text channel first".into());
            return;
        };
        match self.send(bi, Cmd::Say { channel: c, text: proto::clean_text(text) }) {
            Ok(()) => self.notice = None,
            Err(e) => self.notice = Some(e),
        }
    }

    /// Your nickname on backend `bi` (empty: back to your account's display name).
    pub fn set_nick(&mut self, bi: usize, n: &str) -> Result<(), String> {
        let n = n.trim();
        let cmd = if n.is_empty() { Cmd::Nick(None) } else { Cmd::Nick(Some(proto::nick(n)?)) };
        self.send(bi, cmd)?;
        let name = self.backends[bi].name.clone();
        self.notice = Some(if n.is_empty() { format!("{name}: your display name again") } else { format!("{name}: your nickname is now {n}") });
        Ok(())
    }

    /// Open Settings with each backend's nickname field filled in.
    pub fn open_settings(&mut self) {
        for b in &mut self.backends {
            b.nick_draft = b.my_name();
        }
        self.notice = None;
        self.page = Page::Settings;
    }

    /// Settings' Save beside a backend's nickname.
    pub fn save_nick(&mut self, bi: usize) -> Result<(), String> {
        let Some(b) = self.backends.get(bi) else { return Err("That backend is gone".into()) };
        let draft = b.nick_draft.trim().to_string();
        if draft == b.my_name() {
            return Ok(());
        }
        let display = b.account.as_ref().map(|a| a.display.clone()).unwrap_or_default();
        self.set_nick(bi, if draft == display { "" } else { &draft })
    }

    /// Backends a new server can go on: (index, name), signed-in ones.
    pub fn hosts(&self) -> Vec<(usize, String)> {
        self.backends.iter().enumerate().filter(|(_, b)| b.ready()).map(|(i, b)| (i, b.name.clone())).collect()
    }

    /// The Create page's Create.
    pub fn create_server(&mut self) -> Result<(), String> {
        let name = proto::guild_name(&self.new_server)?;
        let up: Vec<usize> = self.hosts().into_iter().map(|(i, _)| i).collect();
        if up.is_empty() {
            return Err("No Discord backend is connected yet".into());
        }
        let bi = if up.contains(&self.new_server_backend) { self.new_server_backend } else { up[0] };
        self.send(bi, Cmd::GuildCreate(name.clone()))?;
        self.open_when_made = Some((bi, name.clone()));
        self.new_server.clear();
        self.notice = Some(format!("Making {} on {} ...", name, self.backends[bi].name));
        Ok(())
    }

    /// The Join page: join a server.
    pub fn join_server(&mut self, r: GuildRef) -> Result<(), String> {
        let name = self.guild_of(r).map(|g| g.name.clone()).ok_or("That server is gone")?;
        self.send(r.0, Cmd::GuildJoin(r.1))?;
        self.open_when_made = Some((r.0, name));
        Ok(())
    }

    /// Leave (or, owning it, delete) the open server.
    pub fn leave_server(&mut self) -> Result<(), String> {
        let (bi, g) = self.guild.ok_or("No server open")?;
        if g == ecm_discord::server::HOME && self.owns_current() {
            return Err("This backend's home server can't be deleted".into());
        }
        let cmd = if self.owns_current() { Cmd::GuildDelete(g) } else { Cmd::GuildLeave(g) };
        self.send(bi, cmd)
    }

    /// The New channel page's Create.
    pub fn create_channel(&mut self) -> Result<(), String> {
        let (bi, g) = self.guild.ok_or("No server open")?;
        if !self.owns_current() {
            return Err("Only the server's owner makes channels".into());
        }
        let name = proto::channel_name(self.new_channel_kind, &self.new_channel)?;
        self.send(bi, Cmd::ChannelCreate { guild: g, kind: self.new_channel_kind, name })?;
        self.new_channel.clear();
        self.page = Page::Servers;
        Ok(())
    }

    /// Join a voice channel of the open server (leaving any other voice channel first).
    pub fn join_voice(&mut self, c: u32) -> Result<(), String> {
        let (bi, _) = self.guild.ok_or("No server open")?;
        if self.voice.as_ref().is_some_and(|v| v.backend == bi && v.channel == c) {
            return Ok(());
        }
        self.leave_voice();
        self.send(bi, Cmd::VoiceJoin(c))?;
        self.voice_wanted = Some((bi, c));
        Ok(())
    }

    /// Hang up.
    pub fn leave_voice(&mut self) {
        if let Some(v) = self.voice.take() {
            let _ = self.send(v.backend, Cmd::VoiceLeave);
        } else if let Some((bi, _)) = self.voice_wanted {
            let _ = self.send(bi, Cmd::VoiceLeave);
        }
        self.voice_wanted = None;
        self.talking = false;
    }

    /// The voice channel we are in: (server name, channel name).
    pub fn voice_place(&self) -> Option<(String, String)> {
        let v = self.voice.as_ref()?;
        let b = self.backends.get(v.backend)?;
        let c = b.channels.get(&v.channel)?;
        let g = b.guilds.get(&c.guild)?;
        Some((g.name.clone(), c.name.clone()))
    }

    // ------------------------------------------------------------------ events

    pub fn set_on_screen(&mut self, on: bool) {
        self.on_screen = on;
        if on {
            self.unread = 0;
        }
    }

    /// Apply one event from backend `bi`; returns a notification to post for a message
    /// from someone else that arrives while the window isn't on screen.
    pub fn apply(&mut self, bi: usize, e: &Event) -> Option<Note> {
        let open = self.guild;
        let open_channel = self.channel;
        let on_screen = self.on_screen;
        let page = self.page;
        let b = self.backends.get_mut(bi)?;
        let mut note = None;
        match e {
            Event::Auth(_) | Event::Backend { .. } => {
                if b.greeting.feed(&e.line()) && b.auth == Auth::Greeting {
                    b.auth = Auth::Ready;
                    if let Event::Backend { name, .. } = e {
                        b.name = name.clone();
                    }
                }
            }
            Event::Ready { uid, name } => {
                b.me = *uid;
                b.names.insert(*uid, name.clone());
                b.auth = Auth::SignedIn;
                b.screen = None;
                if page == Page::SignIn(bi) {
                    self.page = Page::Servers;
                }
            }
            Event::User { uid, name, .. } | Event::Renamed { uid, name } => {
                b.names.insert(*uid, name.clone());
            }
            Event::Guild { id, owner, joined, members, name } => {
                let was_joined = b.guilds.get(id).is_some_and(|g| g.joined);
                b.guilds.insert(*id, Guild { name: name.clone(), owner: *owner, joined: *joined, members: *members });
                if was_joined && !joined {
                    b.channels.retain(|_, c| c.guild != *id);
                    b.members.remove(id);
                    if open == Some((bi, *id)) {
                        self.guild = None;
                        self.channel = None;
                        self.page = Page::Servers;
                        self.pick_default();
                    }
                }
            }
            Event::GuildGone(id) => {
                b.guilds.remove(id);
                let gone: Vec<u32> = b.channels.iter().filter(|(_, c)| c.guild == *id).map(|(c, _)| *c).collect();
                b.channels.retain(|_, c| c.guild != *id);
                b.members.remove(id);
                if self.voice.as_ref().is_some_and(|v| v.backend == bi && gone.contains(&v.channel)) {
                    self.voice = None;
                }
                if open == Some((bi, *id)) {
                    self.guild = None;
                    self.channel = None;
                    self.page = Page::Servers;
                    self.pick_default();
                }
            }
            Event::Channel { guild, id, kind, name } => {
                let ch = b.channels.entry(*id).or_insert_with(|| Channel {
                    guild: *guild,
                    kind: *kind,
                    name: String::new(),
                    messages: Vec::new(),
                    history_asked: false,
                    unread: 0,
                });
                ch.name = name.clone();
                // The open server got its first text channel: open it.
                if open == Some((bi, *guild)) && open_channel.is_none() && *kind == Kind::Text {
                    self.select_channel(*id);
                }
            }
            Event::ChannelGone(id) => {
                b.channels.remove(id);
                b.voice.retain(|_, v| v.channel != *id);
                if self.voice.as_ref().is_some_and(|v| v.backend == bi && v.channel == *id) {
                    self.voice = None;
                }
                if open.is_some_and(|(o, _)| o == bi) && open_channel == Some(*id) {
                    self.channel = None;
                    self.page = Page::Servers;
                }
            }
            Event::Member { guild, uid, presence, name } => {
                b.names.insert(*uid, name.clone());
                let list = b.members.entry(*guild).or_default();
                list.retain(|(u, _)| u != uid);
                if *presence != Presence::Left {
                    list.push((*uid, *presence));
                }
                let names = b.names.clone();
                let list = b.members.get_mut(guild).unwrap();
                list.sort_by_key(|(u, p)| (*p != Presence::Online, names.get(u).map(|n| n.to_lowercase()).unwrap_or_default()));
            }
            Event::Msg { channel, id, time, uid, text, earlier } => {
                let me = b.me;
                let bname = b.name.clone();
                let who = b.name_of(*uid);
                let my_name = b.my_name();
                let Some(ch) = b.channels.get_mut(channel) else { return None };
                if ch.messages.iter().any(|m| m.id == *id) {
                    return None;
                }
                let mine = *uid == me;
                let msg = Message { id: *id, time: time.clone(), uid: *uid, text: text.clone(), mine, earlier: *earlier };
                // Ids grow with time: history slots in before anything newer.
                let at = ch.messages.iter().position(|m| m.id > *id).unwrap_or(ch.messages.len());
                ch.messages.insert(at, msg);
                if ch.messages.len() > MAX_MESSAGES {
                    let n = ch.messages.len() - MAX_MESSAGES;
                    ch.messages.drain(..n);
                }
                let looking = open.is_some_and(|(o, g)| o == bi && g == ch.guild) && open_channel == Some(*channel);
                let _ = my_name;
                if !*earlier && !mine {
                    if !looking || !on_screen {
                        ch.unread += 1;
                    }
                    if !on_screen {
                        self.unread += 1;
                        let gname = b.guilds.get(&ch.guild).map(|g| g.name.clone()).unwrap_or(bname);
                        let body = if self.unread > 1 { format!("{} (+{} more)", text, self.unread - 1) } else { text.clone() };
                        note = Some(Note { title: format!("{} in #{} ({})", who, ch.name, gname), body });
                    }
                }
                if looking {
                    self.serial += 1;
                }
            }
            Event::HistoryEnd(_) => {}
            Event::Voice { channel, port, token, sid } => {
                if self.voice_wanted == Some((bi, *channel)) {
                    self.voice_wanted = None;
                    self.voice = Some(VoiceSession { backend: bi, channel: *channel, port: *port, token: *token, sid: *sid });
                }
            }
            Event::VoiceState { channel, sid, uid, event } => match event {
                VoiceEvent::Join => {
                    b.voice.insert(*sid, InVoice { channel: *channel, uid: *uid, speaking: false });
                }
                VoiceEvent::Leave => {
                    b.voice.remove(sid);
                    if self.voice.as_ref().is_some_and(|v| v.backend == bi && v.sid == *sid) {
                        self.voice = None;
                    }
                }
                VoiceEvent::Speaking | VoiceEvent::Quiet => {
                    if let Some(v) = b.voice.get_mut(sid) {
                        v.speaking = *event == VoiceEvent::Speaking;
                    }
                }
            },
            Event::Synced => {
                b.link = Link::Up { synced: true };
                self.pick_default();
            }
            Event::Pong => {}
            Event::Err(text) => {
                let name = b.name.clone();
                if b.auth == Auth::Hello {
                    // The backend refused our token: sign in again.
                    b.auth = Auth::Ready;
                    b.account = None;
                    b.refused = Some(text.clone());
                }
                self.notice = Some(format!("{name}: {text}"));
                if self.voice_wanted.is_some_and(|(w, _)| w == bi) {
                    self.voice_wanted = None;
                }
            }
        }
        // A server we made or joined: open it once it is ours.
        if let Some((wb, name)) = self.open_when_made.clone() {
            if wb == bi {
                let made = self.backends[bi].guilds.iter().find(|(_, g)| g.joined && g.name == name).map(|(id, _)| *id);
                if let Some(g) = made {
                    self.open_when_made = None;
                    self.open_guild((bi, g));
                }
            }
        }
        note
    }
}

#[cfg(test)]
mod tests;
