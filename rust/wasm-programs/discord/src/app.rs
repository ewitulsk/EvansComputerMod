//! [`DiscordApp`]: the window, one connection per Discord backend, the search for
//! backends on the network, voice, and notifications.
//!
//! Handlers only change the [`Model`] (queueing lines for a backend); between loop
//! turns the driver does the I/O: looks for backends (at start and every minute),
//! connects the ones not connected (retrying failed ones), sends the queued lines,
//! reads what arrived, posts a `Notify` for a message that arrives while the window
//! isn't on screen, and runs voice: the microphone through the voice gate to the
//! backend's UDP port, everyone else's audio through the jitter mixer to the
//! speaker (or the AirPods, when the phone's audio goes to headphones).
//!
//! On a loudspeaker the app is half duplex, like a speakerphone: while it plays
//! others it doesn't send its microphone, or they would hear themselves back.

use crate::model::{Link, Model, Origin};
use crate::view;
use ecm_discord::proto::Event;
use ecm_discord::voice::{Mixer, Packet, Sender, KEEPALIVE_MS, RATE};
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::App;
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use std::cell::RefCell;
use std::rc::Rc;

/// The network: TCP connections to backends (by handle), the search, and the voice socket.
pub trait Net {
    /// Connect (may block for the connect timeout); a handle for the connection. `Err` says why.
    fn connect(&mut self, host: &str, port: u16) -> Result<i32, String>;
    /// Send one protocol line (without the newline).
    fn send(&mut self, h: i32, line: &str) -> Result<(), String>;
    /// What has arrived since the last call (never blocks); `Err` once it closed.
    fn recv(&mut self, h: i32) -> Result<Vec<u8>, String>;
    fn close(&mut self, h: i32);
    /// Discord backends on the network (docs/DISCOVERY.md); blocks for the search.
    fn discover(&mut self) -> Vec<ecm_disco::Found> {
        Vec::new()
    }
    /// A nickname when none is set (the program: from the computer's MAC).
    fn default_nick(&mut self) -> String {
        "guest".into()
    }
    /// A clock in milliseconds.
    fn now_ms(&mut self) -> u64;
    /// Voice: a UDP socket that talks to `host:port`.
    fn voice_open(&mut self, host: &str, port: u16) -> Result<(), String>;
    fn voice_send(&mut self, data: &[u8]);
    /// Datagrams that arrived (never blocks).
    fn voice_recv(&mut self) -> Vec<Vec<u8>>;
    fn voice_close(&mut self);
}

/// The sound card: the microphone and the speaker at [`RATE`] mono.
pub trait Audio {
    /// Open both (`Err`: no microphone or speaker, or no permission; voice still listens if the speaker works).
    fn open(&mut self) -> Result<(), String>;
    /// What the microphone captured since the last call (empty when there is none).
    fn capture(&mut self) -> Vec<i16>;
    /// Queue samples for the speaker.
    fn play(&mut self, pcm: &[i16]);
    /// Milliseconds queued for the speaker.
    fn queued_ms(&mut self) -> i64;
    /// The audio goes to headphones (AirPods), not a loudspeaker.
    fn headphones(&mut self) -> bool {
        false
    }
    fn close(&mut self);
}

/// How often connections are polled while something is connected, ms.
pub const POLL_MS: i32 = 100;
/// ... and while in a voice channel.
pub const VOICE_POLL_MS: i32 = 20;
/// The network is searched again this often for new backends, ms.
pub const RESEARCH_MS: u64 = 60_000;
/// A backend that failed is tried again after this long, ms.
pub const RETRY_MS: u64 = 15_000;
/// Audio kept queued for the speaker, ms.
pub const PLAY_AHEAD_MS: i64 = 160;
/// On a loudspeaker the microphone stays closed this long after we played someone, ms.
pub const ECHO_HOLD_MS: u64 = 300;

/// The voice call's I/O, while in a voice channel.
struct Call {
    backend: usize,
    channel: u32,
    sender: Sender,
    mixer: Mixer,
    last_keepalive: u64,
    last_played: u64,
    audio_ok: bool,
}

pub struct DiscordApp<N: Net, A: Audio> {
    pub app: App,
    pub model: Rc<RefCell<Model>>,
    pub net: N,
    pub audio: A,
    pub window: u32,
    /// Where the settings live (`ECM_APP_DATA`); None = not saved.
    data_dir: Option<String>,
    saved: String,
    /// Connection handle per backend (same index as `model.backends`).
    handles: Vec<Option<i32>>,
    inbufs: Vec<Vec<u8>>,
    call: Option<Call>,
    last_search: u64,
    shown_serial: u64,
    /// Notifications posted (for tests and the log).
    pub posted: Vec<(String, String)>,
    /// Voice packets sent and received (tests, the log).
    pub voice_sent: u64,
    pub voice_heard: u64,
}

impl<N: Net, A: Audio> DiscordApp<N, A> {
    /// Connect to the compositor and open the window. `data_dir` holds the saved settings;
    /// `chat_conf` is `/etc/chat.conf`'s text, if the computer has one (its backend is used).
    pub fn new(conn: Box<dyn Connection>, net: N, audio: A, data_dir: Option<String>, chat_conf: Option<&str>) -> Result<Self, ConnError> {
        let mut app = App::with_connection(crate::APP_ID, conn)?;
        let saved = data_dir.as_ref().and_then(|dir| std::fs::read_to_string(format!("{dir}/settings")).ok());
        let mut m = Model::default();
        m.load(saved.as_deref(), chat_conf);
        m.search();
        if let Ok(id) = app.core.image_png(32, 32, crate::ICON_PNG) {
            m.logo = id;
        }
        let spk = ecm_ui::icons::from_ascii(&SPEAKER, &[('.', 0), ('w', view::MUTED)]);
        if let Ok(id) = app.core.image_rgba(spk.w, spk.h, &spk.rgba) {
            m.speaker = id;
        }
        let model = Rc::new(RefCell::new(m));
        let v = model.clone();
        let window = app.window("Discord", move |cx| view::view(&v, cx));
        let hot = model.clone();
        app.hotkey(1, "Send", move |cx| {
            let text = cx.take_text("compose");
            hot.borrow_mut().compose(&text);
        });
        let mute = model.clone();
        app.hotkey(2, "Mute", move |_| {
            let mut s = mute.borrow_mut();
            s.muted = !s.muted;
        });
        let hang = model.clone();
        app.hotkey(3, "Hang up", move |_| hang.borrow_mut().leave_voice());
        let saved_text = model.borrow().settings_text();
        Ok(DiscordApp {
            app,
            model,
            net,
            audio,
            window,
            data_dir,
            saved: saved_text,
            handles: Vec::new(),
            inbufs: Vec::new(),
            call: None,
            last_search: 0,
            shown_serial: 0,
            posted: Vec::new(),
            voice_sent: 0,
            voice_heard: 0,
        })
    }

    /// One loop turn: draw, wait for input, then the I/O.
    pub fn step(&mut self, timeout_ms: i32) -> bool {
        let wait = {
            let m = self.model.borrow();
            let pending = m.search_wanted
                || m.backends.iter().any(|b| !b.outbox.is_empty() || b.link == Link::Idle)
                || m.voice.is_some() != self.call.is_some();
            if pending {
                0
            } else if m.voice.is_some() {
                VOICE_POLL_MS
            } else if m.backends.iter().any(|b| b.up() || matches!(b.link, Link::Down(_))) {
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
        while self.step(-1) {}
        self.hang_up();
        for h in self.handles.iter().flatten() {
            self.net.close(*h);
        }
    }

    fn on_screen(&self) -> bool {
        self.app.core.window(self.window).is_none_or(|w| w.activated)
    }

    /// The I/O between loop turns.
    pub fn process(&mut self) {
        let now = self.net.now_ms();
        let mut changed = false;
        let on = self.on_screen();
        if on != self.model.borrow().on_screen {
            self.model.borrow_mut().set_on_screen(on);
            changed = true;
        }
        // Look for backends once the window is on screen (the search may wait for the network
        // permission dialog, and the player should see Discord behind it), then every minute.
        let shown = self.app.core.window(self.window).is_some_and(|w| w.configured);
        if shown && !self.model.borrow().searching && now.saturating_sub(self.last_search) >= RESEARCH_MS && self.last_search != 0 {
            self.model.borrow_mut().search();
        }
        // A nickname from the computer's MAC when none is set (once the window is up and drawn: reading
        // the network interfaces may wait for the network permission dialog, and the player should see
        // Discord behind it).
        if shown && self.model.borrow().nick.is_empty() {
            self.redraw();
            let n = self.net.default_nick();
            self.model.borrow_mut().nick = n;
        }
        if self.model.borrow().search_wanted && shown {
            self.model.borrow_mut().search_wanted = false;
            self.redraw();
            let found = self.net.discover();
            self.last_search = now.max(1);
            self.model.borrow_mut().on_found(found);
            changed = true;
        }
        if shown {
            changed |= self.connect_one(now);
        }
        changed |= self.exchange();
        changed |= self.voice(now);
        self.save_settings();
        if changed {
            self.redraw();
            if !self.model.borrow().on_screen {
                self.app.core.send(Message::BackgroundDone(proto::BackgroundDone));
            }
        }
    }

    /// Connect the first backend that needs it (one per turn: a connect may block).
    fn connect_one(&mut self, now: u64) -> bool {
        let n = self.model.borrow().backends.len();
        self.handles.resize(n, None);
        self.inbufs.resize(n, Vec::new());
        let next = {
            let m = self.model.borrow();
            (0..n).find(|&i| match &m.backends[i].link {
                Link::Idle => true,
                Link::Down(_) => now.saturating_sub(m.backends[i].tried_ms) >= RETRY_MS,
                _ => false,
            })
        };
        let Some(i) = next else { return false };
        self.model.borrow_mut().on_connecting(i, now);
        self.redraw();
        let (host, port) = {
            let m = self.model.borrow();
            (m.backends[i].host.clone(), m.backends[i].port)
        };
        match self.net.connect(&host, port) {
            Ok(h) => {
                self.handles[i] = Some(h);
                self.inbufs[i].clear();
                self.model.borrow_mut().on_connected(i);
            }
            Err(e) => self.model.borrow_mut().on_down(i, &e),
        }
        true
    }

    /// Send what is queued and read what arrived, for every connected backend.
    fn exchange(&mut self) -> bool {
        let mut changed = false;
        for i in 0..self.handles.len() {
            let Some(h) = self.handles[i] else { continue };
            let out: Vec<String> = std::mem::take(&mut self.model.borrow_mut().backends[i].outbox);
            let mut lost = None;
            for line in out {
                if let Err(e) = self.net.send(h, &line) {
                    lost = Some(e);
                    break;
                }
            }
            if lost.is_none() {
                loop {
                    match self.net.recv(h) {
                        Ok(b) if b.is_empty() => break,
                        Ok(b) => {
                            for l in ecm_chat::proto::take_lines(&mut self.inbufs[i], &b) {
                                let Some(e) = Event::parse(&l) else { continue };
                                let note = self.model.borrow_mut().apply(i, &e);
                                changed = true;
                                if let Some(n) = note {
                                    self.notify(&n.title, &n.body);
                                }
                            }
                        }
                        Err(e) => {
                            lost = Some(e);
                            break;
                        }
                    }
                }
            }
            if let Some(e) = lost {
                self.net.close(h);
                self.handles[i] = None;
                let name = self.model.borrow().backends[i].endpoint();
                let why = if e.is_empty() { format!("the connection to {name} closed") } else { e };
                self.model.borrow_mut().on_down(i, &why);
                changed = true;
            }
            // Lines queued by what arrived (HISTORY for a server just opened).
            let more = !self.model.borrow().backends[i].outbox.is_empty();
            if more && self.handles[i].is_some() {
                let out: Vec<String> = std::mem::take(&mut self.model.borrow_mut().backends[i].outbox);
                for line in out {
                    if self.net.send(h, &line).is_err() {
                        break;
                    }
                }
            }
        }
        changed
    }

    fn hang_up(&mut self) {
        if self.call.take().is_some() {
            self.net.voice_close();
            self.audio.close();
            self.app.core.background_mode("call", false);
        }
    }

    /// Voice: start or stop the call to match the model, then move audio both ways.
    fn voice(&mut self, now: u64) -> bool {
        let want = self.model.borrow().voice.clone();
        let mut changed = false;
        match (&want, &self.call) {
            (None, Some(_)) => {
                self.hang_up();
                self.model.borrow_mut().talking = false;
                return true;
            }
            (Some(v), Some(c)) if c.backend != v.backend || c.channel != v.channel => {
                self.hang_up();
            }
            _ => {}
        }
        let Some(v) = want else { return changed };
        if self.call.is_none() {
            let host = self.model.borrow().backends[v.backend].host.clone();
            if let Err(e) = self.net.voice_open(&host, v.port) {
                let mut m = self.model.borrow_mut();
                m.notice = Some(format!("Voice: {e}"));
                m.leave_voice();
                return true;
            }
            // A missing microphone or speaker only costs that half of the call.
            if let Err(e) = self.audio.open() {
                self.model.borrow_mut().notice = Some(format!("Voice: {e}"));
            }
            let audio_ok = true;
            let sender = Sender::new(v.token);
            self.net.voice_send(&sender.keepalive().encode());
            // The call keeps running when Discord is hidden or the phone is pocketed (the player is asked once).
            self.app.core.background_mode("call", true);
            self.call = Some(Call { backend: v.backend, channel: v.channel, sender, mixer: Mixer::new(), last_keepalive: now, last_played: 0, audio_ok });
            let hp = audio_ok && self.audio.headphones();
            self.model.borrow_mut().headphones = hp;
            changed = true;
        }
        let (muted, deafened, headphones) = {
            let m = self.model.borrow();
            (m.muted, m.deafened, m.headphones)
        };
        let call = self.call.as_mut().unwrap();
        // Everyone else, into the mixer.
        for d in self.net.voice_recv() {
            if let Some(p) = Packet::decode(&d) {
                self.voice_heard += 1;
                if !deafened {
                    call.mixer.push(&p);
                }
            }
        }
        if call.audio_ok {
            // The microphone (dropped while a loudspeaker plays others: no echo).
            let mic = self.audio.capture();
            call.sender.muted = muted || deafened;
            let echo = !headphones && now.saturating_sub(call.last_played) < ECHO_HOLD_MS && call.last_played != 0;
            if !echo {
                for p in call.sender.push(&mic) {
                    self.net.voice_send(&p.encode());
                    self.voice_sent += 1;
                }
            }
            // The speaker, kept a little ahead.
            let room = PLAY_AHEAD_MS - self.audio.queued_ms();
            if room > 0 {
                let pcm = call.mixer.pull((room as usize) * RATE as usize / 1000);
                if !pcm.is_empty() && ecm_audio::peak_dbfs(&pcm) > -60 {
                    call.last_played = now;
                }
                if !pcm.is_empty() {
                    self.audio.play(&pcm);
                }
            }
        }
        if now.saturating_sub(call.last_keepalive) >= KEEPALIVE_MS {
            call.last_keepalive = now;
            self.net.voice_send(&call.sender.keepalive().encode());
            let hp = call.audio_ok && self.audio.headphones();
            if hp != headphones {
                self.model.borrow_mut().headphones = hp;
                changed = true;
            }
        }
        let talking = self.call.as_ref().is_some_and(|c| c.sender.talking());
        if talking != self.model.borrow().talking {
            self.model.borrow_mut().talking = talking;
            changed = true;
        }
        changed
    }

    fn notify(&mut self, title: &str, body: &str) {
        self.posted.push((title.to_string(), body.to_string()));
        self.app.core.send(Message::Notify(proto::Notify {
            tag: "discord".into(),
            title: title.chars().take(256).collect(),
            body: body.chars().take(1024).collect(),
            icon: String::new(),
            actions: Vec::new(),
        }));
    }

    fn save_settings(&mut self) {
        let text = self.model.borrow().settings_text();
        if text == self.saved {
            return;
        }
        self.saved = text.clone();
        if let Some(dir) = &self.data_dir {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(format!("{dir}/settings"), text);
        }
    }

    /// Redraw now; when a message arrived in the open channel, scroll it to its end.
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

    /// Backends this app knows (tests, the log): (name, endpoint, origin, connected).
    pub fn backends(&self) -> Vec<(String, String, Origin, bool)> {
        self.model.borrow().backends.iter().map(|b| (b.name.clone(), b.endpoint(), b.origin.clone(), b.up())).collect()
    }
}

/// The voice-channel speaker (10x10).
const SPEAKER: [&str; 10] = [
    "..........",
    ".....w....",
    "....ww..w.",
    "wwwwww...w",
    "wwwwww.w.w",
    "wwwwww.w.w",
    "wwwwww...w",
    "....ww..w.",
    ".....w....",
    "..........",
];

pub mod fake;

#[cfg(test)]
mod tests;
