//! The conversation as plain state: fed `ecm_chat::proto::Event`s, read by the
//! view. No I/O, so every rule is tested with `cargo test -p chatapp`.

use ecm_chat::proto::{Event, DEFAULT_PORT};

/// Messages kept (older ones scroll away).
pub const MAX_LINES: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// Someone said something (`MSG`, or `HIST` replayed on joining).
    Message,
    /// Joins, leaves, nick changes, the welcome.
    Notice,
    /// The server refused something, or the connection failed.
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    pub time: String,
    pub nick: String,
    pub text: String,
    /// Said by this client.
    pub mine: bool,
    /// Replayed from before we joined.
    pub earlier: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conn {
    /// Not connected yet: the connect form.
    Idle,
    Connecting,
    /// Connected; `joined` once the server said WELCOME.
    Connected { joined: bool },
    /// It failed or the server closed the connection: why, with a Reconnect button.
    Failed(String),
}

/// What compact layouts show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Conversation,
    People,
}

/// A notification to post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug)]
pub struct Model {
    pub conn: Conn,
    /// The connect form (also what is saved).
    pub server: String,
    pub port: u16,
    pub nick: String,
    /// The nickname the server gave us.
    pub me: String,
    pub room: String,
    pub lines: Vec<Line>,
    /// Who is in the room (sorted).
    pub people: Vec<String>,
    /// Messages from others while the window wasn't on screen.
    pub unread: usize,
    pub on_screen: bool,
    pub pane: Pane,
    /// Bumped whenever a line is added (the view scrolls to the bottom).
    pub serial: u64,
    /// The driver connects when this is set.
    pub connect_wanted: bool,
    /// Lines for the server (the driver sends them).
    pub outbox: Vec<String>,
    /// The driver closes the connection when set.
    pub quit_wanted: bool,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            conn: Conn::Idle,
            server: String::new(),
            port: DEFAULT_PORT,
            nick: String::new(),
            me: String::new(),
            room: String::new(),
            lines: Vec::new(),
            people: Vec::new(),
            unread: 0,
            on_screen: true,
            pane: Pane::Conversation,
            serial: 0,
            connect_wanted: false,
            outbox: Vec::new(),
            quit_wanted: false,
        }
    }
}

impl Model {
    pub fn connected(&self) -> bool {
        matches!(self.conn, Conn::Connected { .. })
    }

    /// "10.0.30.1" or "10.0.30.1:7000".
    pub fn endpoint(&self) -> String {
        if self.port == DEFAULT_PORT {
            self.server.clone()
        } else {
            format!("{}:{}", self.server, self.port)
        }
    }

    /// The room's title for headers and the room list.
    pub fn room_title(&self) -> String {
        if self.room.is_empty() {
            "Chat".into()
        } else {
            self.room.clone()
        }
    }

    /// One line under the header.
    pub fn status(&self) -> String {
        match &self.conn {
            Conn::Idle => "Not connected".into(),
            Conn::Connecting => format!("Connecting to {} ...", self.endpoint()),
            Conn::Connected { joined: false } => format!("Connected to {}, joining ...", self.endpoint()),
            Conn::Connected { joined: true } => {
                format!("{} as {} · {} here", self.endpoint(), self.me, self.people.len().max(1))
            }
            Conn::Failed(why) => why.clone(),
        }
    }

    fn push(&mut self, line: Line) {
        self.lines.push(line);
        if self.lines.len() > MAX_LINES {
            let n = self.lines.len() - MAX_LINES;
            self.lines.drain(..n);
        }
        self.serial += 1;
    }

    /// A local note (a typed /help, a connection problem).
    pub fn notice(&mut self, text: &str, kind: LineKind) {
        self.push(Line { kind, time: String::new(), nick: String::new(), text: text.into(), mine: false, earlier: false });
    }

    /// The connect form's Connect: check it and ask the driver to connect.
    pub fn request_connect(&mut self) -> Result<(), String> {
        let server = self.server.trim().to_string();
        if server.is_empty() {
            return Err("Type the chat server's address (the computer running chatd)".into());
        }
        let (host, port) = ecm_chat::client::split_host_port(&server)?;
        if host.is_empty() {
            return Err("Type the chat server's address (the computer running chatd)".into());
        }
        let nick = self.nick.trim().to_string();
        // Empty: the app picks one from the computer's MAC once connected.
        if !nick.is_empty() && !ecm_chat::proto::valid_nick(&nick) {
            return Err(format!("Nickname: 1-{} letters, digits, '-', '_' or '.'", ecm_chat::proto::MAX_NICK));
        }
        self.server = host;
        if let Some(p) = port {
            self.port = p;
        }
        self.nick = nick;
        self.conn = Conn::Connecting;
        self.connect_wanted = true;
        Ok(())
    }

    /// The compose box's Send (or Enter): a message, or a /command.
    pub fn compose(&mut self, text: &str) {
        match ecm_chat::client::typed(text) {
            ecm_chat::client::Typed::Send(line) => {
                if self.connected() {
                    self.outbox.push(line);
                } else {
                    self.notice("Not connected: Reconnect first", LineKind::Error);
                }
            }
            ecm_chat::client::Typed::Show(t) => {
                for l in t.lines() {
                    self.notice(l, LineKind::Notice);
                }
            }
            ecm_chat::client::Typed::Quit => self.quit_wanted = true,
            ecm_chat::client::Typed::Nothing => {}
        }
    }

    /// The connection is up: join with our nickname.
    pub fn on_connected(&mut self) {
        self.conn = Conn::Connected { joined: false };
        self.lines.retain(|l| l.kind != LineKind::Error);
        self.outbox.push(format!("NICK {}", self.nick));
    }

    /// It failed or closed.
    pub fn on_failed(&mut self, why: &str) {
        self.conn = Conn::Failed(why.into());
        self.people.clear();
        self.notice(why, LineKind::Error);
    }

    /// The window went on or off screen (focus / the phone showing it).
    pub fn set_on_screen(&mut self, on: bool) {
        self.on_screen = on;
        if on {
            self.unread = 0;
        }
    }

    fn add_person(&mut self, nick: &str) {
        if !self.people.iter().any(|p| p == nick) {
            self.people.push(nick.into());
            self.people.sort_by_key(|p| p.to_lowercase());
        }
    }

    /// Apply one server event; returns a notification to post when a message
    /// from someone else arrives while the window isn't on screen.
    pub fn apply(&mut self, e: &Event) -> Option<Note> {
        match e {
            Event::Welcome { nick, room } => {
                self.me = nick.clone();
                self.nick = nick.clone();
                self.room = room.clone();
                self.conn = Conn::Connected { joined: true };
                self.add_person(nick);
                self.notice(&format!("You joined {} as {}", room, nick), LineKind::Notice);
                // Who else is here.
                self.outbox.push("WHO".into());
            }
            Event::History { time, nick, text } => {
                self.push(Line { kind: LineKind::Message, time: time.clone(), nick: nick.clone(), text: text.clone(), mine: *nick == self.me, earlier: true });
            }
            Event::Msg { time, nick, text } => {
                let mine = *nick == self.me;
                self.push(Line { kind: LineKind::Message, time: time.clone(), nick: nick.clone(), text: text.clone(), mine, earlier: false });
                if !mine && !self.on_screen {
                    self.unread += 1;
                    let body = if self.unread > 1 { format!("{} (+{} more)", text, self.unread - 1) } else { text.clone() };
                    return Some(Note { title: format!("{} in {}", nick, self.room_title()), body });
                }
            }
            Event::Join { time, nick } => {
                self.add_person(nick);
                if *nick != self.me {
                    self.push(Line { kind: LineKind::Notice, time: time.clone(), nick: nick.clone(), text: format!("{} joined", nick), mine: false, earlier: false });
                }
            }
            Event::Leave { time, nick, reason } => {
                self.people.retain(|p| p != nick);
                self.push(Line { kind: LineKind::Notice, time: time.clone(), nick: nick.clone(), text: format!("{} left ({})", nick, reason), mine: false, earlier: false });
            }
            Event::Nick { time, old, new } => {
                if *old == self.me {
                    self.me = new.clone();
                    self.nick = new.clone();
                }
                self.people.retain(|p| p != old);
                self.add_person(new);
                self.push(Line { kind: LineKind::Notice, time: time.clone(), nick: new.clone(), text: format!("{} is now {}", old, new), mine: false, earlier: false });
            }
            Event::Who(nicks) => {
                self.people.clear();
                for n in nicks {
                    self.add_person(n);
                }
            }
            Event::Err(text) => self.notice(&format!("Server: {}", text), LineKind::Error),
        }
        None
    }

    /// Saved settings (the `chat.conf` format, so `ecm_chat::client::parse_config` reads it).
    pub fn settings_text(&self) -> String {
        format!("server {}\nport {}\nnick {}\n", self.server, self.port, self.nick)
    }

    pub fn load_settings(&mut self, text: &str) {
        let c = ecm_chat::client::parse_config(text);
        if let Some(s) = c.server {
            self.server = s;
        }
        if let Some(p) = c.port {
            self.port = p;
        }
        if let Some(n) = c.nick {
            self.nick = n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined() -> Model {
        let mut m = Model { server: "10.0.30.1".into(), nick: "alex".into(), ..Default::default() };
        m.request_connect().unwrap();
        m.on_connected();
        assert_eq!(m.outbox, vec!["NICK alex"]);
        m.outbox.clear();
        m.apply(&Event::Welcome { nick: "alex".into(), room: "Tech Village chat".into() });
        m
    }

    #[test]
    fn connect_form_checks_its_fields() {
        let mut m = Model::default();
        assert!(m.request_connect().unwrap_err().contains("address"));
        m.server = "10.0.30.1:7000".into();
        m.nick = "".into();
        m.request_connect().unwrap();
        m.nick = "bad nick".into();
        assert!(m.request_connect().unwrap_err().starts_with("Nickname"));
        m.nick = "alex".into();
        m.request_connect().unwrap();
        assert_eq!((m.server.as_str(), m.port), ("10.0.30.1", 7000));
        assert!(m.connect_wanted);
        assert_eq!(m.conn, Conn::Connecting);
        assert_eq!(m.endpoint(), "10.0.30.1:7000");
    }

    #[test]
    fn welcome_history_messages_and_people() {
        let mut m = joined();
        assert_eq!(m.conn, Conn::Connected { joined: true });
        assert_eq!(m.outbox, vec!["WHO"], "asks who is here");
        assert_eq!(m.room_title(), "Tech Village chat");
        m.apply(&Event::History { time: "10:00:00".into(), nick: "bo".into(), text: "earlier words".into() });
        m.apply(&Event::Who(vec!["bo".into(), "alex".into(), "Cy".into()]));
        assert_eq!(m.people, vec!["alex", "bo", "Cy"]);
        m.apply(&Event::Join { time: "10:00:01".into(), nick: "dee".into() });
        m.apply(&Event::Nick { time: "10:00:02".into(), old: "bo".into(), new: "bob".into() });
        m.apply(&Event::Leave { time: "10:00:03".into(), nick: "Cy".into(), reason: "quit".into() });
        assert_eq!(m.people, vec!["alex", "bob", "dee"]);
        assert!(m.apply(&Event::Msg { time: "10:00:04".into(), nick: "alex".into(), text: "hi all".into() }).is_none());
        let last = m.lines.last().unwrap();
        assert!(last.mine && last.kind == LineKind::Message);
        assert!(m.lines.iter().any(|l| l.earlier && l.text == "earlier words"));
        assert!(m.status().contains("as alex · 3 here"), "{}", m.status());
        // Our own nick change follows.
        m.apply(&Event::Nick { time: "10:00:05".into(), old: "alex".into(), new: "al".into() });
        assert_eq!(m.me, "al");
    }

    #[test]
    fn messages_off_screen_notify_and_count() {
        let mut m = joined();
        // On screen: no notification.
        assert!(m.apply(&Event::Msg { time: "t".into(), nick: "bo".into(), text: "one".into() }).is_none());
        m.set_on_screen(false);
        // Our own message never notifies.
        assert!(m.apply(&Event::Msg { time: "t".into(), nick: "alex".into(), text: "me".into() }).is_none());
        let n = m.apply(&Event::Msg { time: "t".into(), nick: "bo".into(), text: "two".into() }).unwrap();
        assert_eq!(n, Note { title: "bo in Tech Village chat".into(), body: "two".into() });
        let n = m.apply(&Event::Msg { time: "t".into(), nick: "bo".into(), text: "three".into() }).unwrap();
        assert_eq!(n.body, "three (+1 more)");
        assert_eq!(m.unread, 2);
        m.set_on_screen(true);
        assert_eq!(m.unread, 0);
    }

    #[test]
    fn compose_sends_messages_and_commands() {
        let mut m = joined();
        m.outbox.clear();
        m.compose("hello there");
        m.compose("/who");
        m.compose("/nick al");
        m.compose("   ");
        assert_eq!(m.outbox, vec!["MSG hello there", "WHO", "NICK al"]);
        m.compose("/bogus");
        assert!(m.lines.last().unwrap().text.contains("unknown command"));
        m.compose("/quit");
        assert!(m.quit_wanted);
        // Not connected: nothing is queued, it says so.
        let mut off = Model::default();
        off.compose("hi");
        assert!(off.outbox.is_empty());
        assert_eq!(off.lines.last().unwrap().kind, LineKind::Error);
    }

    #[test]
    fn failures_settings_and_the_line_cap() {
        let mut m = joined();
        m.on_failed("chat: connection to 10.0.30.1:7777 closed.");
        assert!(matches!(m.conn, Conn::Failed(_)));
        assert!(m.people.is_empty());
        let saved = m.settings_text();
        let mut n = Model::default();
        n.load_settings(&saved);
        assert_eq!((n.server.as_str(), n.port, n.nick.as_str()), ("10.0.30.1", DEFAULT_PORT, "alex"));
        for i in 0..(MAX_LINES + 30) {
            n.notice(&format!("{i}"), LineKind::Notice);
        }
        assert_eq!(n.lines.len(), MAX_LINES);
        assert_eq!(n.lines[0].text, "30");
        // A reconnect clears the old error lines.
        m.on_connected();
        assert!(m.lines.iter().all(|l| l.kind != LineKind::Error));
    }
}
