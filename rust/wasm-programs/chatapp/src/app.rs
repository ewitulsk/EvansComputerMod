//! [`ChatApp`]: the window, the connection to `chatd` and the notifications.
//!
//! Handlers only change the [`Model`] (and queue lines or a connect); between
//! loop turns the driver does the I/O: connects, sends the queued lines, reads
//! what arrived, applies it, posts a `Notify` for a message that arrives while
//! the window isn't on screen (not activated: another window has focus on a
//! Terminal; on a phone, the app is in the background, which it only sees
//! during a background refresh wake), scrolls the conversation to its end, and
//! saves the server and nickname in the app's data folder.

use crate::model::{LineKind, Model};
use crate::view;
use ecm_chat::proto::{take_lines, Event};
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::json::Json;
use ecm_ui::App;
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use std::cell::RefCell;
use std::rc::Rc;

/// A connection to a chat server: the host's TCP socket in the program, a
/// scripted one in tests.
pub trait Net {
    /// Connect (may block for the connect timeout). `Err` says why, for the player.
    fn connect(&mut self, host: &str, port: u16) -> Result<(), String>;
    /// Send one protocol line (without the newline).
    fn send(&mut self, line: &str) -> Result<(), String>;
    /// What has arrived since the last call (never blocks); `Err` once it closed.
    fn recv(&mut self) -> Result<Vec<u8>, String>;
    fn close(&mut self);
    /// A nickname when the player left it empty (the program: from the computer's MAC).
    fn default_nick(&mut self) -> String {
        "guest".into()
    }
    /// Chat servers on the network (docs/DISCOVERY.md); blocks for the search.
    fn discover(&mut self) -> Vec<ecm_disco::Found> {
        Vec::new()
    }
}

/// How often the connection is polled while it is up, ms.
pub const POLL_MS: i32 = 100;

pub struct ChatApp<N: Net> {
    pub app: App,
    pub model: Rc<RefCell<Model>>,
    pub net: N,
    pub window: u32,
    /// Where the settings live (`ECM_APP_DATA`); None = not saved.
    data_dir: Option<String>,
    /// The saved settings named a server: [`ChatApp::auto_connect`] connects at once.
    saved_server: bool,
    inbuf: Vec<u8>,
    shown_serial: u64,
    /// Notifications posted (for tests and the log).
    pub posted: Vec<(String, String)>,
}

impl<N: Net> ChatApp<N> {
    /// Connect to the compositor and open the window. `data_dir` holds the
    /// saved server and nickname; `defaults` is the model they are loaded into.
    pub fn new(conn: Box<dyn Connection>, net: N, data_dir: Option<String>, defaults: Model) -> Result<Self, ConnError> {
        Self::with_chat_conf(conn, net, data_dir, defaults, None)
    }

    /// [`ChatApp::new`] with the text of `/etc/chat.conf`, if the computer has one: it
    /// prefills the connect form while no server is saved ([`crate::model::starting`]).
    pub fn with_chat_conf(conn: Box<dyn Connection>, net: N, data_dir: Option<String>, defaults: Model, chat_conf: Option<&str>)
        -> Result<Self, ConnError> {
        let mut app = App::with_connection(crate::APP_ID, conn)?;
        let saved = data_dir.as_ref().and_then(|dir| std::fs::read_to_string(format!("{dir}/settings")).ok());
        let (mut m, saved_server) = crate::model::starting(defaults, saved.as_deref(), chat_conf);
        // No saved server to connect to: the connect form looks on the network at once.
        if !saved_server {
            m.search();
        }
        // A cold start after a suspend: the conversation comes back too.
        restore(&mut m, &app.restore_json());
        let model = Rc::new(RefCell::new(m));
        let v = model.clone();
        let window = app.window("Chat", move |cx| view::view(&v, cx));
        let saved = model.clone();
        app.on_suspend_json(move || save_state(&saved.borrow()));
        let hot = model.clone();
        app.hotkey(1, "Send", move |cx| {
            let text = cx.take_text("compose");
            hot.borrow_mut().compose(&text);
        });
        Ok(ChatApp { app, model, net, window, data_dir, saved_server, inbuf: Vec::new(), shown_serial: 0, posted: Vec::new() })
    }

    /// Connect at once if a server was saved, as the form's Connect would (a form prefilled
    /// from `/etc/chat.conf` waits for Connect).
    pub fn auto_connect(&mut self) {
        let ready = self.saved_server && !self.model.borrow().server.is_empty();
        if ready {
            let _ = self.model.borrow_mut().request_connect();
            self.process();
        }
    }

    /// One loop turn: draw, wait for input (up to [`POLL_MS`] while connected), then the I/O.
    pub fn step(&mut self, timeout_ms: i32) -> bool {
        let wait = {
            let m = self.model.borrow();
            if m.connect_wanted || !m.outbox.is_empty() {
                0
            } else if m.connected() {
                if timeout_ms < 0 { POLL_MS } else { timeout_ms.min(POLL_MS) }
            } else {
                timeout_ms
            }
        };
        if !self.app.step(wait) {
            return false;
        }
        self.process();
        !self.app.core.is_closed()
    }

    pub fn run(&mut self) {
        self.auto_connect();
        while self.step(-1) {}
        self.net.close();
    }

    fn on_screen(&self) -> bool {
        self.app.core.window(self.window).is_none_or(|w| w.activated)
    }

    /// The I/O between loop turns.
    pub fn process(&mut self) {
        let on = self.on_screen();
        if on != self.model.borrow().on_screen {
            self.model.borrow_mut().set_on_screen(on);
            self.redraw();
        }
        if self.model.borrow().quit_wanted {
            self.model.borrow_mut().quit_wanted = false;
            let _ = self.net.send("QUIT");
            self.net.close();
            self.model.borrow_mut().on_failed("You left the chat. Connect to join again.");
            self.redraw();
        }
        // Search once the window is on screen: the search may wait for the network permission
        // dialog, and the player should see Chat behind it.
        let shown = self.app.core.window(self.window).is_some_and(|w| w.configured);
        if self.model.borrow().search_wanted && shown {
            self.model.borrow_mut().search_wanted = false;
            // Show "Looking ..." before the (blocking) search.
            self.redraw();
            let found = self.net.discover();
            self.model.borrow_mut().on_found(found);
            self.redraw();
        }
        if self.model.borrow().connect_wanted {
            self.model.borrow_mut().connect_wanted = false;
            // Show "Connecting..." before the (blocking) connect.
            self.redraw();
            let (host, port) = {
                let m = self.model.borrow();
                (m.server.clone(), m.port)
            };
            self.inbuf.clear();
            match self.net.connect(&host, port) {
                Ok(()) => {
                    if self.model.borrow().nick.is_empty() {
                        let n = self.net.default_nick();
                        self.model.borrow_mut().nick = n;
                    }
                    self.model.borrow_mut().on_connected();
                    self.save_settings();
                }
                Err(e) => self.model.borrow_mut().on_failed(&e),
            }
            self.redraw();
        }
        let out: Vec<String> = std::mem::take(&mut self.model.borrow_mut().outbox);
        for line in out {
            if let Err(e) = self.net.send(&line) {
                self.lost(&e);
                return;
            }
        }
        if !self.model.borrow().connected() {
            return;
        }
        let mut changed = false;
        loop {
            match self.net.recv() {
                Ok(b) if b.is_empty() => break,
                Ok(b) => {
                    for l in take_lines(&mut self.inbuf, &b) {
                        let Some(e) = Event::parse(&l) else { continue };
                        let note = self.model.borrow_mut().apply(&e);
                        changed = true;
                        if let Some(n) = note {
                            self.notify(&n.title, &n.body);
                        }
                    }
                }
                Err(e) => {
                    self.lost(&e);
                    return;
                }
            }
        }
        // WHO and other answers queued by what arrived.
        let out: Vec<String> = std::mem::take(&mut self.model.borrow_mut().outbox);
        for line in out {
            if let Err(e) = self.net.send(&line) {
                self.lost(&e);
                return;
            }
        }
        if changed {
            self.redraw();
            // In the background (a refresh wake on a phone): done for now.
            if !self.model.borrow().on_screen {
                self.app.core.send(Message::BackgroundDone(proto::BackgroundDone));
            }
        }
    }

    fn lost(&mut self, why: &str) {
        self.net.close();
        let endpoint = self.model.borrow().endpoint();
        let text = if why.is_empty() { format!("The connection to {endpoint} closed. Reconnect to join again.") } else { why.to_string() };
        self.model.borrow_mut().on_failed(&text);
        self.redraw();
    }

    fn notify(&mut self, title: &str, body: &str) {
        self.posted.push((title.to_string(), body.to_string()));
        self.app.core.send(Message::Notify(proto::Notify {
            tag: "chat".into(),
            title: title.chars().take(256).collect(),
            body: body.chars().take(1024).collect(),
            icon: String::new(),
            actions: Vec::new(),
        }));
    }

    fn save_settings(&self) {
        if let Some(dir) = &self.data_dir {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(format!("{dir}/settings"), self.model.borrow().settings_text());
        }
    }

    /// Redraw now; when a line arrived, scroll the conversation to its end.
    fn redraw(&mut self) {
        let serial = self.model.borrow().serial;
        if serial != self.shown_serial {
            self.shown_serial = serial;
            if let Some(w) = self.app.core.window_mut(self.window) {
                w.scroll_to_end("history");
            }
        }
        self.app.core.mark_all_dirty();
        let _ = self.app.core.render();
    }
}

/// What a suspend saves: the form and the last 50 lines.
pub fn save_state(m: &Model) -> Json {
    let lines: Vec<Json> = m
        .lines
        .iter()
        .rev()
        .take(50)
        .rev()
        .filter(|l| l.kind == LineKind::Message)
        .map(|l| Json::Arr(vec![Json::Str(l.time.clone()), Json::Str(l.nick.clone()), Json::Str(l.text.clone()), Json::Bool(l.mine)]))
        .collect();
    Json::Obj(vec![
        ("server".into(), Json::Str(m.server.clone())),
        ("nick".into(), Json::Str(m.nick.clone())),
        ("room".into(), Json::Str(m.room.clone())),
        ("lines".into(), Json::Arr(lines)),
    ])
}

fn restore(m: &mut Model, j: &Json) {
    if let Some(s) = j.get("server").and_then(Json::as_str) {
        if !s.is_empty() {
            m.server = s.to_string();
        }
    }
    if let Some(s) = j.get("nick").and_then(Json::as_str) {
        if !s.is_empty() {
            m.nick = s.to_string();
        }
    }
    if let Some(s) = j.get("room").and_then(Json::as_str) {
        m.room = s.to_string();
    }
    for l in j.get("lines").and_then(Json::as_array).into_iter().flatten() {
        let Some(a) = l.as_array() else { continue };
        let s = |i: usize| a.get(i).and_then(Json::as_str).unwrap_or("").to_string();
        m.lines.push(crate::model::Line {
            kind: LineKind::Message,
            time: s(0),
            nick: s(1),
            text: s(2),
            mine: matches!(a.get(3), Some(Json::Bool(true))),
            earlier: true,
        });
    }
    if !m.lines.is_empty() {
        m.serial += 1;
    }
}

pub mod fake;

#[cfg(test)]
mod tests;
