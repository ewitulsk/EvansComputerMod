//! **Messages** (`dev.ecm.messages`, program `messages`): texts and picture
//! messages over telephonyd's texts-only port ([`ecm_tel::ctl::SMS_PORT`], the
//! `sms` permission).
//!
//! * **Conversations**: every thread, newest first, with the other side's name
//!   (Contacts, else the number; groups `Alex, Bo +1`), the last text, its age and
//!   the unread count. New message. The header shows this phone's number.
//! * **Thread**: bubbles (sent on the right in the accent colour, received on
//!   the left), each with its time and, for sent ones, Sending / Sent /
//!   Delivered / Failed (Retry sends it again). Group threads name the sender
//!   above each received bubble. Pictures show inline (View opens them full
//!   window, Save to Files copies them to `/home/Pictures`), voice notes play,
//!   contact cards go into Contacts. Opening a thread marks it read. Delete
//!   removes the whole conversation (after a confirmation; ecm-ui has no swipe).
//!   A thread opened from a notification has a reply box at the top
//!   (notifications can't take text).
//! * **Compose**: recipients (Contacts suggestions as you type; several make a
//!   group), the text with a counter (characters and SMS parts: GSM 7-bit
//!   160 / 153 per part, UCS-2 70 / 67), attachments (a picture or a `.wav` from
//!   Files, a voice note recorded here, a contact card). One recipient and text
//!   only goes as an SMS (`sms`), anything else as an MMS (`mms`).
//!
//! * **compact** (< 400 lu) and **medium**: one screen at a time.
//! * **wide** (>= 800 lu): the conversations beside the open thread or compose.
//!
//! Program args: `--to <number[,number]>` (compose to them), `--thread <key>`,
//! `--text <text>` (prefill), `--vcard <path>` (compose with that contact card attached). Notification tags: `sms:<thread key>` opens that
//! thread, `sms:failed` the conversations.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::book::Book;
use crate::widgets::{avatar, card, message, pill, GREY, RED};
use crate::{fmt_ago, pretty, Daemon, Event, Files, MsgLine, Tel, ThreadLine, POLL_MS};
use ecm_tel::ctl::{self, dec, enc, SMS_PORT};
use ecm_tel::sms::{parts_needed, Encoding};
use ecm_tel::vcard;
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::element::{button, column, image, label, list, panel, row, scroll_view, spacer, text_field, Element};
use ecm_ui::{Align, App, Ctx, UiEvent, ViewCtx};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Message, SizeClass};

/// Where pictures are saved with Save to Files.
pub const PICTURES: &str = "/home/Pictures";
/// Where attachments made here (voice notes, contact cards) are written.
pub const OUTBOX: &str = "/home/Messages/out";
/// The longest voice note, s.
pub const MAX_VOICE_S: u64 = 60;

/// The microphone, for voice notes.
pub trait Recorder {
    /// Start capturing.
    fn start(&mut self) -> Result<(), String>;
    /// Take what was captured since the last call (called every step while recording).
    fn pump(&mut self);
    /// Stop: the recording as a WAV file.
    fn stop(&mut self) -> Vec<u8>;
}

/// No microphone (tests, a computer without one).
pub struct NoMic;

impl Recorder for NoMic {
    fn start(&mut self) -> Result<(), String> {
        Err("there is no microphone".into())
    }
    fn pump(&mut self) {}
    fn stop(&mut self) -> Vec<u8> {
        Vec::new()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Screen {
    List,
    Thread,
    Compose,
    /// A picture full window.
    Viewer(String),
    /// Choosing a contact to send as a card.
    ShareContact,
}

/// An MMS part the app has looked at.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    /// A picture uploaded as an image resource.
    Picture { res: u32, w: i32, h: i32 },
    /// A contact card: the name on it.
    Card(String),
    /// Unreadable.
    Broken,
}

/// Work the app does outside the view (telephonyd, files, the microphone).
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// A request on the texts port.
    Tel(String),
    /// Send `text` and `files` to `to`; `compose`: from the compose screen;
    /// `retry`: the failed message this replaces.
    Send { to: Vec<String>, text: String, files: Vec<String>, compose: bool, retry: Option<u64> },
    SavePicture(String),
    ImportCard(String),
    ShareContact(String),
    RecordStart,
    RecordStop,
}

#[derive(Debug)]
pub struct Model {
    pub daemon: Daemon,
    pub msisdn: String,
    pub registered: bool,
    pub threads: Vec<ThreadLine>,
    pub book: Book,
    pub screen: Screen,
    /// The open thread.
    pub key: Option<String>,
    pub msgs: Vec<MsgLine>,
    /// The reply being typed in the open thread.
    pub draft: String,
    /// The thread was opened from a notification: a reply box on top.
    pub quick_reply: bool,
    pub confirm_delete: bool,
    /// Compose: the recipients chosen, the recipient field, the text, the attachments.
    pub to: Vec<String>,
    pub to_field: String,
    pub text: String,
    pub attach: Vec<String>,
    pub share_query: String,
    pub parts: BTreeMap<String, Part>,
    /// A PickFile waiting for its answer.
    pub picking: Option<u32>,
    pub next_req: u32,
    /// Recording a voice note since (app ms).
    pub recording: Option<u64>,
    pub message: Option<(bool, String)>,
    pub queue: Vec<Act>,
    pub refresh: bool,
    pub scroll_end: bool,
    /// telephonyd's clock (from `status now`, when it says) against ours: (theirs, ours).
    pub clock: Option<(u64, u64)>,
    /// Now on telephonyd's clock (message times are on it).
    pub tel_now: u64,
    pub now_ms: u64,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            daemon: Daemon::Unknown,
            msisdn: String::new(),
            registered: false,
            threads: Vec::new(),
            book: Book::default(),
            screen: Screen::List,
            key: None,
            msgs: Vec::new(),
            draft: String::new(),
            quick_reply: false,
            confirm_delete: false,
            to: Vec::new(),
            to_field: String::new(),
            text: String::new(),
            attach: Vec::new(),
            share_query: String::new(),
            parts: BTreeMap::new(),
            picking: None,
            next_req: 1,
            recording: None,
            message: None,
            queue: Vec::new(),
            refresh: true,
            scroll_end: false,
            clock: None,
            tel_now: 0,
            now_ms: 0,
        }
    }
}

/// The request that sends `text` and `files` to `to`: an SMS for one number and
/// text only, else an MMS (several numbers make a group). None: nothing to send.
pub fn send_request(to: &[String], text: &str, files: &[String]) -> Option<String> {
    if to.is_empty() || (text.trim().is_empty() && files.is_empty()) {
        return None;
    }
    if to.len() == 1 && files.is_empty() {
        return Some(format!("sms {} {}", enc(&to[0]), enc(text)));
    }
    let nums: Vec<String> = to.iter().map(|n| enc(n)).collect();
    let fs = if files.is_empty() { "-".to_string() } else { files.iter().map(|f| enc(f)).collect::<Vec<_>>().join(",") };
    Some(format!("mms {} {} {}", nums.join(","), enc(text), fs))
}

/// The compose counter: `172 characters · 2 parts`, or what an MMS carries.
pub fn counter(text: &str, recipients: usize, attachments: usize) -> String {
    let n = text.chars().count();
    if recipients > 1 || attachments > 0 {
        let a = match attachments {
            0 => String::new(),
            1 => " · 1 attachment".into(),
            k => format!(" · {k} attachments"),
        };
        return format!("{n} characters · MMS{a}");
    }
    let (e, parts, left) = parts_needed(text);
    let parts = parts.max(1);
    let uni = if e == Encoding::Ucs2 { " · Unicode" } else { "" };
    format!("{n} characters · {parts} {}{uni} · {left} left", if parts == 1 { "part" } else { "parts" })
}

/// A picture's size from its PNG header.
pub fn png_size(b: &[u8]) -> Option<(i32, i32)> {
    if b.len() < 24 || &b[..8] != b"\x89PNG\r\n\x1a\n" || &b[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]) as i32;
    let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]) as i32;
    (w > 0 && h > 0 && w <= 4096 && h <= 4096).then_some((w, h))
}

/// `w x h` scaled to fit in `mw x mh` (never enlarged).
pub fn fit(w: i32, h: i32, mw: i32, mh: i32) -> (i32, i32) {
    let s = (mw as f32 / w as f32).min(mh as f32 / h as f32).min(1.0);
    (((w as f32 * s) as i32).max(1), ((h as f32 * s) as i32).max(1))
}

/// The name of a thread: the contact or the number; groups `Alex, Bo +1`.
pub fn thread_name(book: &Book, key: &str) -> String {
    let ms: Vec<&str> = key.split(',').filter(|n| !n.is_empty()).collect();
    if ms.len() <= 1 {
        return book.name_of(key);
    }
    let names: Vec<String> = ms.iter().map(|n| book.find(n).map(|c| c.name.split_whitespace().next().unwrap_or(&c.name).to_string()).unwrap_or_else(|| pretty(n))).collect();
    let shown = names.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
    if names.len() > 2 {
        format!("{shown} +{}", names.len() - 2)
    } else {
        shown
    }
}

/// The avatar's letters: the contact's initials (a group: its first two members'), else `#`.
pub fn thread_initials(book: &Book, key: &str) -> String {
    let ms: Vec<&str> = key.split(',').filter(|n| !n.is_empty()).collect();
    if ms.len() == 1 {
        return book.find(ms[0]).map(|c| c.initials()).unwrap_or_else(|| "#".into());
    }
    let s: String = ms.iter().filter_map(|n| book.find(n)).filter_map(|c| c.name.chars().next()).take(2).collect();
    if s.is_empty() {
        "#".into()
    } else {
        s.to_uppercase()
    }
}

/// What a sent message's status says (None for received ones).
pub fn status_text(m: &MsgLine) -> Option<String> {
    if m.incoming {
        return None;
    }
    Some(match m.status.as_str() {
        "sending" => "Sending".into(),
        "sent" => "Sent".into(),
        "delivered" => "Delivered".into(),
        s if s.starts_with("failed") => match s.split_once(':') {
            Some((_, why)) if !why.is_empty() => format!("Failed: {why}"),
            _ => "Failed".into(),
        },
        s => s.to_string(),
    })
}

fn failed(m: &MsgLine) -> bool {
    !m.incoming && m.status.starts_with("failed")
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn safe_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if n.is_empty() {
        "contact".into()
    } else {
        n
    }
}

impl Model {
    /// Program arguments: `--to a[,b]`, `--thread <key>`, `--text <t>`, `--vcard <path>`.
    pub fn apply_args(&mut self, args: &[String]) {
        let mut to: Option<String> = None;
        let mut thread: Option<String> = None;
        let mut text: Option<String> = None;
        let mut card: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            let v = args.get(i + 1).cloned();
            match args[i].as_str() {
                "--to" => to = v,
                "--thread" => thread = v,
                "--text" => text = v,
                "--vcard" => card = v,
                _ => {
                    i += 1;
                    continue;
                }
            }
            i += 2;
        }
        if let Some(p) = card {
            // Contacts' "Send in Messages": a new message carrying the card
            self.new_message();
            if let Some(t) = to {
                self.to = t.split(',').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
            }
            self.attach.push(p);
            self.text = text.unwrap_or_default();
        } else if let Some(k) = thread {
            self.open_thread(&k, false);
            if let Some(t) = text {
                self.draft = t;
            }
        } else if let Some(t) = to {
            self.new_message();
            self.to = t.split(',').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
            if let Some(t) = text {
                self.text = t;
            }
        } else if let Some(t) = text {
            self.new_message();
            self.text = t;
        }
    }

    /// The home area code (for numbers typed without one).
    fn home_npa(&self) -> Option<String> {
        let d: String = self.msisdn.chars().filter(|c| c.is_ascii_digit()).collect();
        (self.msisdn.starts_with("+1") && d.len() == 11).then(|| d[1..4].to_string())
    }

    /// A number as telephonyd keys threads by it.
    pub fn normalize(&self, n: &str) -> String {
        let t = n.trim();
        if t.len() <= 6 && t.bytes().all(|b| b.is_ascii_digit()) {
            return t.to_string();
        }
        ecm_tel::number::normalize(t, "1", self.home_npa().as_deref()).map(|e| e.to_string()).unwrap_or_else(|| t.to_string())
    }

    /// The thread key telephonyd gives these recipients.
    pub fn key_for(&self, to: &[String]) -> String {
        let mut v: Vec<String> = to.iter().map(|n| self.normalize(n)).filter(|n| !n.is_empty() && *n != self.msisdn).collect();
        v.sort();
        v.dedup();
        v.join(",")
    }

    pub fn open_thread(&mut self, key: &str, from_notification: bool) {
        if self.key.as_deref() != Some(key) {
            self.msgs.clear();
            self.draft.clear();
        }
        self.key = Some(key.to_string());
        self.screen = Screen::Thread;
        self.quick_reply = from_notification;
        self.confirm_delete = false;
        self.queue.push(Act::Tel(format!("read {key}")));
        self.refresh = true;
        self.scroll_end = true;
    }

    pub fn back(&mut self) {
        match self.screen {
            Screen::Viewer(_) if self.key.is_some() => self.screen = Screen::Thread,
            Screen::ShareContact => self.screen = Screen::Compose,
            _ => {
                self.screen = Screen::List;
                self.key = None;
                self.msgs.clear();
                self.quick_reply = false;
                self.confirm_delete = false;
            }
        }
    }

    pub fn new_message(&mut self) {
        self.screen = Screen::Compose;
        self.key = None;
        self.msgs.clear();
        self.to.clear();
        self.to_field.clear();
        self.text.clear();
        self.attach.clear();
    }

    /// Compose, starting from the open thread (its people and the draft): to attach something.
    pub fn compose_from_thread(&mut self) {
        let Some(k) = self.key.clone() else { return };
        let draft = std::mem::take(&mut self.draft);
        self.new_message();
        self.to = k.split(',').map(String::from).collect();
        self.text = draft;
    }

    /// Add what's in the recipient field (a number, or the only contact it matches).
    pub fn add_typed_recipient(&mut self) {
        let t = self.to_field.trim().to_string();
        if t.is_empty() {
            return;
        }
        let hits = self.book.search(&t);
        let n = if t.chars().any(|c| c.is_ascii_digit()) && !t.chars().any(|c| c.is_alphabetic()) {
            t.clone()
        } else if hits.len() == 1 && hits[0].primary().is_some() {
            hits[0].primary().unwrap().to_string()
        } else {
            self.message = Some((false, format!("No contact or number '{t}'")));
            return;
        };
        self.add_recipient(&n);
    }

    pub fn add_recipient(&mut self, n: &str) {
        if !self.to.iter().any(|x| crate::book::same_number(x, n)) {
            self.to.push(n.to_string());
        }
        self.to_field.clear();
    }

    pub fn send_compose(&mut self) {
        if !self.to_field.trim().is_empty() {
            self.add_typed_recipient();
        }
        if self.to.is_empty() {
            self.message = Some((false, "Choose who the message is for".into()));
            return;
        }
        if self.text.trim().is_empty() && self.attach.is_empty() {
            self.message = Some((false, "Type a message or attach something".into()));
            return;
        }
        self.queue.push(Act::Send { to: self.to.clone(), text: self.text.clone(), files: self.attach.clone(), compose: true, retry: None });
    }

    pub fn send_reply(&mut self) {
        let Some(k) = self.key.clone() else { return };
        if self.draft.trim().is_empty() {
            return;
        }
        self.queue.push(Act::Send { to: k.split(',').map(String::from).collect(), text: self.draft.clone(), files: Vec::new(), compose: false, retry: None });
    }

    /// Send a failed message again (the failed copy goes).
    pub fn retry(&mut self, id: u64) {
        let (Some(k), Some(m)) = (self.key.clone(), self.msgs.iter().find(|m| m.id == id).cloned()) else { return };
        self.queue.push(Act::Send { to: k.split(',').map(String::from).collect(), text: m.text, files: m.files, compose: false, retry: Some(id) });
    }

    pub fn delete_thread(&mut self) {
        if let Some(k) = self.key.clone() {
            self.queue.push(Act::Tel(format!("delete-thread {k}")));
            self.threads.retain(|t| t.key != k);
            self.back();
        }
    }

    /// Ask the system file picker for a file of these types (`png`, `wav`).
    pub fn pick(&mut self, cx: &mut Ctx, types: &[&str]) {
        let req = self.next_req;
        self.next_req += 1;
        self.picking = Some(req);
        cx.send(Message::PickFile(proto::PickFile { req, mode: proto::PickMode::Open, types: types.iter().map(|t| t.to_string()).collect(), suggested: String::new() }));
    }

    /// A message the toolkit didn't handle: notification taps, picked files, files opened with Messages.
    pub fn on_message(&mut self, msg: &Message) {
        match msg {
            Message::NotificationAction(n) => {
                if let Some(rest) = n.tag.strip_prefix("sms:") {
                    if rest == "failed" {
                        self.back();
                        self.screen = Screen::List;
                        self.refresh = true;
                    } else if !rest.is_empty() {
                        self.open_thread(&dec(rest), true);
                    }
                }
            }
            Message::FilePicked(f) => {
                if self.picking == Some(f.req) {
                    self.picking = None;
                    if !f.path.is_empty() && !self.attach.contains(&f.path) {
                        self.attach.push(f.path.clone());
                    }
                }
            }
            Message::OpenFile(o) => {
                // "Open with Messages": a new message carrying the file
                self.new_message();
                self.attach.push(o.path.clone());
            }
            _ => {}
        }
    }

    fn on_event(&mut self, e: &Event) {
        match e.name.as_str() {
            "sms" | "mms" => {
                self.refresh = true;
                if self.key.as_deref() == Some(e.get("thread")) {
                    self.scroll_end = true;
                }
            }
            "sent" | "delivered" | "failed" => {
                let id: u64 = e.get("id").parse().unwrap_or(0);
                let st = match e.name.as_str() {
                    "failed" => format!("failed:{}", e.get("why")),
                    s => s.to_string(),
                };
                if let Some(m) = self.msgs.iter_mut().find(|m| m.id == id && !m.incoming) {
                    m.status = st;
                }
                self.refresh = true;
            }
            "registration" => self.refresh = true,
            _ => {}
        }
    }

    fn thread_open(&self) -> bool {
        self.key.is_some() && matches!(self.screen, Screen::Thread | Screen::Viewer(_))
    }
}

pub type M = Rc<RefCell<Model>>;

pub struct MessagesApp<T: Tel, F: Files, R: Recorder = NoMic> {
    pub app: App,
    pub model: M,
    pub tel: T,
    pub files: F,
    pub mic: R,
    pub window: u32,
    last_poll: Option<u64>,
    last_tick: u64,
}

impl<T: Tel, F: Files> MessagesApp<T, F, NoMic> {
    pub fn new(conn: Box<dyn Connection>, tel: T, files: F) -> Result<Self, ConnError> {
        MessagesApp::with_recorder(conn, tel, files, NoMic)
    }
}

impl<T: Tel, F: Files, R: Recorder> MessagesApp<T, F, R> {
    pub fn with_recorder(conn: Box<dyn Connection>, tel: T, mut files: F, mic: R) -> Result<Self, ConnError> {
        let mut app = App::with_connection(crate::MESSAGES_ID, conn)?;
        let model: M = Rc::new(RefCell::new(Model { book: Book::load(&mut files), ..Default::default() }));
        let v = model.clone();
        let window = app.window("Messages", move |cx| view(&v, cx));
        let mm = model.clone();
        app.on_message(move |_, msg| mm.borrow_mut().on_message(msg));
        Ok(MessagesApp { app, model, tel, files, mic, window, last_poll: None, last_tick: 0 })
    }

    pub fn step(&mut self, timeout_ms: i32, now_ms: u64) -> bool {
        let busy = !self.model.borrow().queue.is_empty() || self.model.borrow().recording.is_some();
        let wait = if busy { 0 } else if timeout_ms < 0 { 250 } else { timeout_ms.min(250) };
        if !self.app.step(wait) {
            return false;
        }
        self.process(now_ms);
        !self.app.core.is_closed()
    }

    pub fn run(&mut self) {
        let start = std::time::Instant::now();
        while self.step(-1, start.elapsed().as_millis() as u64) {}
    }

    pub fn process(&mut self, now_ms: u64) {
        let mut changed = false;
        {
            let mut m = self.model.borrow_mut();
            m.now_ms = now_ms;
            m.tel_now = match m.clock {
                Some((theirs, ours)) => theirs + now_ms.saturating_sub(ours),
                None => now_ms,
            };
        }
        for e in self.tel.events() {
            self.model.borrow_mut().on_event(&e);
            changed = true;
        }
        let acts: Vec<Act> = std::mem::take(&mut self.model.borrow_mut().queue);
        if acts.iter().any(|a| matches!(a, Act::Send { .. })) {
            // "Sending..." while an MMS goes up (telephonyd answers when the MMSC has it)
            self.model.borrow_mut().message = Some((true, "Sending...".into()));
            self.redraw();
        }
        for a in acts {
            self.act(a, now_ms);
            changed = true;
        }
        // a voice note being recorded
        let rec = self.model.borrow().recording;
        if let Some(t0) = rec {
            self.mic.pump();
            if now_ms.saturating_sub(t0) >= MAX_VOICE_S * 1000 {
                self.act(Act::RecordStop, now_ms);
                changed = true;
            } else if now_ms / 1000 != self.last_tick / 1000 {
                changed = true;
            }
        }
        self.last_tick = now_ms;
        let due = self.model.borrow().refresh || self.last_poll.is_none_or(|t| now_ms.saturating_sub(t) >= POLL_MS);
        if due {
            self.last_poll = Some(now_ms);
            changed |= self.poll(now_ms);
        }
        let end = std::mem::take(&mut self.model.borrow_mut().scroll_end);
        if end {
            if let Some(w) = self.app.core.window_mut(self.window) {
                w.scroll_to_end("bubbles");
            }
            changed = true;
        }
        if changed {
            self.redraw();
        }
    }

    fn call(&mut self, req: &str) -> Option<Result<Vec<String>, String>> {
        match self.tel.call(SMS_PORT, req) {
            Ok(r) => Some(r),
            Err(e) => {
                self.model.borrow_mut().daemon = Daemon::Down(e);
                None
            }
        }
    }

    fn act(&mut self, a: Act, now_ms: u64) {
        match a {
            Act::Tel(req) => {
                if let Some(Err(e)) = self.call(&req) {
                    self.model.borrow_mut().message = Some((false, e));
                }
                self.model.borrow_mut().refresh = true;
            }
            Act::Send { to, text, files, compose, retry } => {
                if let Some(id) = retry {
                    let _ = self.call(&format!("delete {id}"));
                }
                // numbers as telephonyd keys threads by them (typed ones too)
                let to: Vec<String> = to.iter().map(|n| self.model.borrow().normalize(n)).collect();
                let Some(req) = send_request(&to, &text, &files) else { return };
                let r = self.call(&req);
                let mut m = self.model.borrow_mut();
                m.refresh = true;
                match r {
                    Some(Ok(_)) => {
                        m.message = None;
                        if compose {
                            let k = m.key_for(&to);
                            m.new_message();
                            m.open_thread(&k, false);
                        } else {
                            m.draft.clear();
                            m.quick_reply = false;
                            m.scroll_end = true;
                        }
                    }
                    Some(Err(e)) => m.message = Some((false, format!("Not sent: {e}"))),
                    None => m.message = None,
                }
            }
            Act::SavePicture(path) => {
                let r = match self.files.read(&path) {
                    Some(data) => {
                        let have = self.files.list(PICTURES);
                        let name = file_name(&path).to_string();
                        let (stem, ext) = name.rsplit_once('.').map(|(a, b)| (a.to_string(), format!(".{b}"))).unwrap_or((name.clone(), String::new()));
                        let mut dest = name.clone();
                        let mut n = 1;
                        while have.contains(&dest) {
                            dest = format!("{stem}-{n}{ext}");
                            n += 1;
                        }
                        let p = format!("{PICTURES}/{dest}");
                        self.files.write(&p, &data).map(|_| format!("Saved to {p}"))
                    }
                    None => Err(format!("can't read {path}")),
                };
                self.model.borrow_mut().message = Some(match r {
                    Ok(t) => (true, t),
                    Err(e) => (false, e),
                });
            }
            Act::ImportCard(path) => {
                let text = self.files.read(&path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
                let mut book = Book::load(&mut self.files);
                let names: Vec<String> = vcard::parse(&text).map(|cs| cs.into_iter().map(|c| c.name).collect()).unwrap_or_default();
                let r = book.import(&text).and_then(|n| if n == 0 { Err("no contact in the card".into()) } else { Ok(n) }).and_then(|_| book.save(&mut self.files));
                let mut m = self.model.borrow_mut();
                match r {
                    Ok(()) => {
                        m.message = Some((true, format!("Added {} to Contacts", names.join(", "))));
                        m.book = book;
                    }
                    Err(e) => m.message = Some((false, format!("Not added: {e}"))),
                }
            }
            Act::ShareContact(uid) => {
                let mut m = self.model.borrow_mut();
                let Some(c) = m.book.by_uid(&uid).cloned() else { return };
                let p = format!("{OUTBOX}/{}.vcf", safe_name(&c.name));
                match self.files.write(&p, c.to_vcard().as_bytes()) {
                    Ok(()) => {
                        if !m.attach.contains(&p) {
                            m.attach.push(p.clone());
                        }
                        m.parts.insert(p, Part::Card(c.name.clone()));
                        m.screen = Screen::Compose;
                    }
                    Err(e) => m.message = Some((false, e)),
                }
            }
            Act::RecordStart => {
                let r = self.mic.start();
                let mut m = self.model.borrow_mut();
                match r {
                    Ok(()) => {
                        m.recording = Some(now_ms);
                        m.message = None;
                    }
                    Err(e) => m.message = Some((false, format!("Can't record: {e}. Attach a .wav from Files instead."))),
                }
            }
            Act::RecordStop => {
                if self.model.borrow_mut().recording.take().is_none() {
                    return;
                }
                let wav = self.mic.stop();
                let p = format!("{OUTBOX}/voice-{now_ms}.wav");
                let r = if wav.len() <= 44 { Err("nothing was recorded".to_string()) } else { self.files.write(&p, &wav) };
                let mut m = self.model.borrow_mut();
                match r {
                    Ok(()) => m.attach.push(p),
                    Err(e) => m.message = Some((false, format!("Voice note: {e}"))),
                }
            }
        }
    }

    fn poll(&mut self, now_ms: u64) -> bool {
        let book = Book::load(&mut self.files);
        let key = {
            let mut m = self.model.borrow_mut();
            m.refresh = false;
            m.book = book;
            m.key.clone()
        };
        let before = {
            let m = self.model.borrow();
            (m.daemon.clone(), m.msisdn.clone(), m.registered, m.threads.clone(), m.msgs.clone(), m.book.clone())
        };
        let Some(r) = self.call("status") else { return before.0 != self.model.borrow().daemon };
        {
            let mut m = self.model.borrow_mut();
            match r {
                Ok(body) => {
                    m.daemon = Daemon::Running;
                    m.msisdn = ctl::value(&body, "msisdn").trim_start_matches('-').to_string();
                    m.registered = ctl::value(&body, "registration") == "registered";
                    if let Ok(t) = ctl::value(&body, "now").parse::<u64>() {
                        m.clock = Some((t, now_ms));
                        m.tel_now = t;
                    }
                }
                Err(e) => m.daemon = Daemon::Down(e),
            }
        }
        if let Some(Ok(b)) = self.call("threads") {
            self.model.borrow_mut().threads = b.iter().filter_map(|l| ThreadLine::parse(l)).collect();
        }
        if let Some(k) = key {
            if let Some(Ok(b)) = self.call(&format!("messages {k} 200")) {
                let msgs: Vec<MsgLine> = b.iter().filter_map(|l| MsgLine::parse(l)).collect();
                let unread = msgs.iter().any(|m| m.incoming && m.status == "received");
                let mut m = self.model.borrow_mut();
                if msgs.len() != m.msgs.len() {
                    m.scroll_end = true;
                }
                m.msgs = msgs;
                // read while it is on screen
                if unread && m.thread_open() {
                    m.queue.push(Act::Tel(format!("read {k}")));
                }
            }
        }
        self.load_parts();
        let m = self.model.borrow();
        before != (m.daemon.clone(), m.msisdn.clone(), m.registered, m.threads.clone(), m.msgs.clone(), m.book.clone())
    }

    /// Look at the open thread's pictures and cards once (pictures become image resources).
    fn load_parts(&mut self) {
        let files: Vec<String> = {
            let m = self.model.borrow();
            m.msgs.iter().flat_map(|x| x.files.clone()).chain(m.attach.iter().cloned()).filter(|f| !m.parts.contains_key(f)).collect()
        };
        if files.is_empty() {
            return;
        }
        // keep the resources few: forget the pictures of other threads
        let pics = self.model.borrow().parts.values().filter(|p| matches!(p, Part::Picture { .. })).count();
        if pics > 32 {
            let old: Vec<u32> = self.model.borrow().parts.values().filter_map(|p| if let Part::Picture { res, .. } = p { Some(*res) } else { None }).collect();
            for r in old {
                self.app.core.destroy_resource(r);
            }
            self.model.borrow_mut().parts.retain(|_, p| !matches!(p, Part::Picture { .. }));
        }
        for f in files {
            let lower = f.to_ascii_lowercase();
            let part = if lower.ends_with(".png") {
                match self.files.read(&f).and_then(|d| png_size(&d).map(|s| (d, s))) {
                    Some((d, (w, h))) => match self.app.core.image_png(w, h, &d) {
                        Ok(res) => Part::Picture { res, w, h },
                        Err(_) => Part::Broken,
                    },
                    None => Part::Broken,
                }
            } else if lower.ends_with(".vcf") {
                let t = self.files.read(&f).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
                match vcard::parse(&t).ok().and_then(|cs| cs.into_iter().next()) {
                    Some(c) => Part::Card(c.name),
                    None => Part::Broken,
                }
            } else {
                continue;
            };
            self.model.borrow_mut().parts.insert(f, part);
        }
    }

    fn redraw(&mut self) {
        self.app.core.mark_all_dirty();
        let _ = self.app.core.render();
    }
}

// ------------------------------------------------------------ the window

const ROW_H: i32 = 26;

fn act(m: &M, f: impl Fn(&mut Model) + 'static) -> impl Fn(&mut Ctx, &UiEvent) + 'static {
    let m = m.clone();
    move |_, _| f(&mut m.borrow_mut())
}

pub fn view(m: &M, cx: &ViewCtx) -> Element {
    let st = m.borrow();
    let pad = if cx.class == SizeClass::Compact { 6 } else { 10 };
    let h = cx.height - 2 * pad;
    let w = cx.width - 2 * pad;
    if let Daemon::Down(why) = &st.daemon {
        return column().gap(8).padding(pad).child(header(m, &st, cx)).child(not_running(cx, why));
    }
    if let Screen::Viewer(p) = &st.screen {
        return column().padding(pad).child(viewer(m, &st, cx, p, w, h));
    }
    if cx.class == SizeClass::Wide {
        let lw = 300;
        let left = column().gap(8).width(lw).child(header(m, &st, cx)).child(conversations(m, &st, cx, h - ROW_H - 8, lw));
        let rw = w - lw - 12;
        let right = match st.screen {
            Screen::Thread => thread(m, &st, cx, rw, h),
            Screen::Compose => compose(m, &st, cx, rw, h),
            Screen::ShareContact => share_contact(m, &st, cx, h),
            _ => panel().fill(cx.theme.surface).radius(6).padding(16).grow(1).child(label("Choose a conversation, or New for a new message.").style("caption")),
        };
        return row().gap(12).padding(pad).align(Align::Start).child(left).child(column().grow(1).child(right));
    }
    let body = match st.screen {
        Screen::Thread => thread(m, &st, cx, w, h),
        Screen::Compose => compose(m, &st, cx, w, h),
        Screen::ShareContact => share_contact(m, &st, cx, h),
        _ => column().gap(8).child(header(m, &st, cx)).child(conversations(m, &st, cx, h - ROW_H - 8, w)),
    };
    column().padding(pad).child(body)
}

fn header(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    let me = if st.msisdn.is_empty() { String::new() } else { format!("My number {}", pretty(&st.msisdn)) };
    let mut r = row().gap(6).child(label("Messages").style("title").wrap(false));
    if cx.class != SizeClass::Compact || st.msisdn.len() < 14 {
        r = r.child(label(me).style("caption").wrap(false));
    }
    if st.daemon == Daemon::Running && !st.registered {
        r = r.child(pill("No service", GREY));
    }
    r.child(spacer()).child(button("New").id("new").style("primary").on_click(act(m, |m| m.new_message())))
}

fn not_running(cx: &ViewCtx, why: &str) -> Element {
    card(
        cx,
        "telephonyd is not running",
        column()
            .gap(6)
            .child(label("Texts need the phone's telephony service. In a terminal:"))
            .child(label("telephonyd --phone &").color(cx.theme.accent))
            .child(label(format!("({why})")).style("caption")),
    )
}

fn thread_row(st: &Model, t: &ThreadLine, w: i32) -> Element {
    let name = thread_name(&st.book, &t.key);
    let mut top = row().gap(4).child(label(name).wrap(false).grow(1)).child(label(fmt_ago(t.time_ms, st.tel_now)).style("caption").wrap(false));
    if t.unread > 0 {
        top = top.child(pill(&t.unread.to_string(), 0x3d8b_fdff));
    }
    let tw = (w - 32 - 8 - 16).max(60);
    let face = ecm_ui_proto::fonts::face(proto::Font::Ui, 8);
    let preview = match &face {
        Some(f) => ecm_ui::text::ellipsize(f, &t.preview.replace('\n', " "), tw),
        None => t.preview.clone(),
    };
    row()
        .gap(8)
        .child(avatar(&thread_initials(&st.book, &t.key), 30))
        .child(column().gap(1).grow(1).child(top).child(label(preview).style("caption").wrap(false)))
}

fn conversations(m: &M, st: &Model, cx: &ViewCtx, h: i32, w: i32) -> Element {
    let mut c = column().gap(6);
    if st.threads.is_empty() {
        c = c.child(label("No messages yet. New starts a conversation.").style("caption"));
    } else {
        let keys: Vec<String> = st.threads.iter().map(|t| t.key.clone()).collect();
        let mm = m.clone();
        let mut l = list().id("threads").children(st.threads.iter().map(|t| thread_row(st, t, w))).on_select(move |cx, _| {
            if let Some(k) = cx.selected("threads").and_then(|i| keys.get(i).cloned()) {
                mm.borrow_mut().open_thread(&k, false);
            }
        });
        if let Some(i) = st.key.as_ref().and_then(|k| st.threads.iter().position(|t| &t.key == k)) {
            l = l.selected(i);
        }
        c = c.child(l);
    }
    if matches!(st.screen, Screen::List | Screen::ShareContact) {
        c = c.child(message(cx, &st.message));
    }
    scroll_view(c).id("list").height(h.max(60))
}

/// The width a bubble needs for `text` (at most `max`).
fn text_width(text: &str, max: i32) -> i32 {
    match ecm_ui_proto::fonts::face(proto::Font::Ui, 12) {
        Some(f) => ecm_ui::text::wrap(&f, text, Some(max)).iter().map(|l| f.text_width(l)).max().unwrap_or(0).min(max),
        None => max,
    }
}

fn part_view(m: &M, st: &Model, cx: &ViewCtx, path: &str, out: bool, tc: u32) -> (Element, i32) {
    let lower = path.to_ascii_lowercase();
    let p = path.to_string();
    if lower.ends_with(".png") {
        return match st.parts.get(path) {
            Some(Part::Picture { res, w, h }) => {
                let (tw, th) = fit(*w, *h, 160, 120);
                let v = act(m, move |m| m.screen = Screen::Viewer(p.clone()));
                (column().gap(4).child(image(*res, tw, th)).child(row().child(button("View").id(format!("view_{}", file_name(path))).on_click(v)).child(spacer())), tw.max(60))
            }
            _ => (label("Picture (can't be shown)").color(tc), 140),
        };
    }
    if lower.ends_with(".wav") {
        let pp = p.clone();
        let el = row().gap(6).child(label("Voice note").color(tc).wrap(false)).child(button("Play").id(format!("play_{}", file_name(path))).on_click(move |cx, _| cx.open(&pp)));
        return (el, 140);
    }
    if lower.ends_with(".vcf") {
        let name = match st.parts.get(path) {
            Some(Part::Card(n)) => n.clone(),
            _ => file_name(path).trim_end_matches(".vcf").to_string(),
        };
        let mut c = column().gap(4).child(label(format!("Contact card: {name}")).color(tc));
        if !out {
            c = c.child(row().child(button("Add to Contacts").id(format!("add_{}", file_name(path))).on_click(act(m, move |m| m.queue.push(Act::ImportCard(p.clone()))))).child(spacer()));
        }
        return (c, 170);
    }
    let pp = p.clone();
    let _ = cx;
    (row().gap(6).child(label(file_name(path).to_string()).color(tc).wrap(false)).child(button("Open").on_click(move |cx, _| cx.open(&pp))), 160)
}

fn bubble(m: &M, st: &Model, cx: &ViewCtx, msg: &MsgLine, maxw: i32, group: bool) -> Element {
    let out = !msg.incoming;
    let (fill, tc) = if out { (cx.theme.accent, cx.theme.accent_text) } else { (cx.theme.surface, cx.theme.text) };
    let mut inner = column().gap(4);
    let mut need = 0;
    for f in &msg.files {
        let (e, w) = part_view(m, st, cx, f, out, tc);
        inner = inner.child(e);
        need = need.max(w);
    }
    if !msg.text.is_empty() {
        need = need.max(text_width(&msg.text, maxw - 16));
        inner = inner.child(label(msg.text.clone()).color(tc).wrap(true));
    }
    let bw = (need + 16).min(maxw).max(40);
    let b = panel().fill(fill).radius(8).padding(8).width(bw).child(inner);
    let mut meta = fmt_ago(msg.time_ms, st.tel_now);
    if let Some(s) = status_text(msg) {
        meta = format!("{meta} · {s}");
    }
    let mut metar = row().gap(6);
    if failed(msg) {
        let id = msg.id;
        metar = metar.child(label(meta).size(8).color(RED).wrap(false)).child(button("Retry").id(format!("retry_{id}")).on_click(act(m, move |m| m.retry(id))));
    } else {
        metar = metar.child(label(meta).style("caption").wrap(false));
    }
    let mut col = column().gap(2);
    if group && msg.incoming {
        col = col.child(label(st.book.name_of(&msg.from)).style("caption").wrap(false));
    }
    // the bubble and its time / status line each on a full-width row, so the line never clips
    if out {
        col.child(row().child(spacer()).child(b)).child(row().child(spacer()).child(metar))
    } else {
        col.child(row().child(b).child(spacer())).child(row().child(metar).child(spacer()))
    }
}

fn thread(m: &M, st: &Model, cx: &ViewCtx, w: i32, h: i32) -> Element {
    let key = st.key.clone().unwrap_or_default();
    let group = key.contains(',');
    let name = thread_name(&st.book, &key);
    let mut head = row().gap(6);
    if cx.class != SizeClass::Wide {
        head = head.child(button("Back").id("back").on_click(act(m, |m| m.back())));
    }
    head = head.child(avatar(&thread_initials(&st.book, &key), 24)).child(column().grow(1).child(label(name.clone()).wrap(false)).child(
        label(if group { format!("{} people", key.split(',').count() + 1) } else { pretty(&key) }).style("caption").wrap(false),
    ));
    head = head.child(button("Delete").id("delete").style("danger").on_click(act(m, |m| m.confirm_delete = true)));
    let mut c = column().gap(6).child(head);
    let mut used = 34 + 6;
    if st.confirm_delete {
        c = c.child(
            panel().fill(cx.theme.surface).radius(6).padding(8).child(
                row()
                    .gap(6)
                    .child(label("Delete this conversation?").grow(1))
                    .child(button("Delete").id("confirm_delete").style("danger").on_click(act(m, |m| m.delete_thread())))
                    .child(button("Cancel").id("cancel_delete").on_click(act(m, |m| m.confirm_delete = false))),
            ),
        );
        used += 40 + 6;
    }
    if st.quick_reply {
        let (q1, q2) = (m.clone(), m.clone());
        c = c.child(
            panel().fill(cx.theme.surface_alt).radius(6).padding(8).child(
                column().gap(4).child(label(format!("Reply to {name}")).style("caption")).child(
                    row()
                        .gap(6)
                        .child(text_field().id("quick_reply").placeholder("Reply").text(st.draft.clone()).grow(1).on_change(move |cx, _| {
                            let t = cx.text("quick_reply");
                            q1.borrow_mut().draft = t;
                        }).on_submit(move |_, _| q2.borrow_mut().send_reply()))
                        .child(button("Send").id("quick_send").style("primary").on_click(act(m, |m| m.send_reply()))),
                ),
            ),
        );
        used += 58 + 6;
    }
    let maxw = (w * 3 / 4).max(120);
    let mut bubbles = column().gap(8);
    if st.msgs.is_empty() {
        bubbles = bubbles.child(label("No messages.").style("caption"));
    }
    for msg in &st.msgs {
        bubbles = bubbles.child(bubble(m, st, cx, msg, maxw, group));
    }
    let msg_h = if st.message.is_some() { 18 } else { 0 };
    let compose_h = ROW_H + 6;
    c = c.child(scroll_view(bubbles).id("bubbles").height((h - used - compose_h - msg_h).max(60)));
    if st.message.is_some() {
        c = c.child(message(cx, &st.message));
    }
    let (d1, d2) = (m.clone(), m.clone());
    c.child(
        row()
            .gap(6)
            .child(button("+").id("attach").on_click(act(m, |m| m.compose_from_thread())))
            .child(text_field().id("draft").placeholder("Text message").text(st.draft.clone()).grow(1).on_change(move |cx, _| {
                let t = cx.text("draft");
                d1.borrow_mut().draft = t;
            }).on_submit(move |_, _| d2.borrow_mut().send_reply()))
            .child(button("Send").id("send").style("primary").on_click(act(m, |m| m.send_reply()))),
    )
}

fn compose(m: &M, st: &Model, cx: &ViewCtx, w: i32, h: i32) -> Element {
    let mut head = row().gap(6);
    if cx.class != SizeClass::Wide {
        head = head.child(button("Back").id("back").on_click(act(m, |m| m.back())));
    }
    head = head.child(label("New message").style("title").wrap(false)).child(spacer());
    let mut c = column().gap(6).child(head);
    // recipients
    let mut chips = row().gap(4);
    for (i, n) in st.to.iter().enumerate() {
        chips = chips.child(button(format!("{} x", st.book.name_of(n))).id(format!("to_{i}")).on_click(act(m, move |m| {
            if i < m.to.len() {
                m.to.remove(i);
            }
        })));
    }
    let (t1, t2) = (m.clone(), m.clone());
    let to_row = row()
        .gap(6)
        .child(label("To").width(24).wrap(false))
        .child(text_field().id("to_field").placeholder("Name or number").text(st.to_field.clone()).grow(1).on_change(move |cx, _| {
            let t = cx.text("to_field");
            t1.borrow_mut().to_field = t;
        }).on_submit(move |_, _| t2.borrow_mut().add_typed_recipient()))
        .child(button("Add").id("add_to").on_click(act(m, |m| m.add_typed_recipient())));
    c = c.child(to_row);
    if !st.to.is_empty() {
        c = c.child(chips);
    }
    let q = st.to_field.trim();
    if !q.is_empty() {
        let hits: Vec<(String, String)> = st.book.search(q).iter().filter_map(|c| c.primary().map(|p| (c.name.clone(), p.to_string()))).take(4).collect();
        if !hits.is_empty() {
            let nums: Vec<String> = hits.iter().map(|(_, n)| n.clone()).collect();
            let mm = m.clone();
            c = c.child(list().id("suggest").children(hits.iter().map(|(name, n)| row().gap(6).child(label(name.clone()).wrap(false)).child(spacer()).child(label(pretty(n)).style("caption").wrap(false)))).on_select(move |cx, _| {
                if let Some(n) = cx.selected("suggest").and_then(|i| nums.get(i).cloned()) {
                    mm.borrow_mut().add_recipient(&n);
                }
            }));
        }
    }
    // attachments
    if !st.attach.is_empty() {
        let mut a = column().gap(3);
        for (i, f) in st.attach.iter().enumerate() {
            let lower = f.to_ascii_lowercase();
            let what = if lower.ends_with(".png") {
                format!("Picture {}", file_name(f))
            } else if lower.ends_with(".wav") {
                "Voice note".to_string()
            } else if lower.ends_with(".vcf") {
                match st.parts.get(f) {
                    Some(Part::Card(n)) => format!("Contact card: {n}"),
                    _ => format!("Contact card {}", file_name(f)),
                }
            } else {
                file_name(f).to_string()
            };
            a = a.child(row().gap(6).child(label(what).grow(1).wrap(false)).child(button("Remove").id(format!("unattach_{i}")).on_click(act(m, move |m| {
                if i < m.attach.len() {
                    m.attach.remove(i);
                }
            }))));
        }
        c = c.child(panel().fill(cx.theme.surface).radius(6).padding(6).child(a));
    }
    // the text and its counter
    let x1 = m.clone();
    // ecm-ui's text fields don't wrap: the whole text, wrapped, above a one-line field
    if !st.text.is_empty() {
        let lines = (h / 40).clamp(3, 10) as usize;
        let face = ecm_ui_proto::fonts::face(proto::Font::Ui, 12);
        let mut shown = st.text.clone();
        if let Some(f) = &face {
            let wrapped = ecm_ui::text::wrap(f, &st.text, Some(w - 24));
            if wrapped.len() > lines {
                shown = format!("...{}", wrapped[wrapped.len() - lines..].join(" "));
            }
        }
        c = c.child(panel().fill(cx.theme.surface).radius(6).padding(6).child(label(shown).wrap(true).id("compose_preview")));
    }
    c = c.child(text_field().id("compose_text").placeholder("Message").text(st.text.clone()).on_change(move |cx, _| {
        let t = cx.text("compose_text");
        x1.borrow_mut().text = t;
    }));
    c = c.child(label(counter(&st.text, st.to.len(), st.attach.len())).style("caption").id("counter").wrap(false));
    // attach
    let (p1, p2) = (m.clone(), m.clone());
    let mut tools = row().gap(4).child(button("Picture").id("attach_picture").on_click(move |cx, _| p1.borrow_mut().pick(cx, &["png"])));
    tools = match st.recording {
        Some(t0) => tools.child(button(format!("Stop {}", crate::fmt_duration(st.now_ms.saturating_sub(t0) / 1000))).id("record").style("danger").on_click(act(m, |m| m.queue.push(Act::RecordStop)))),
        None => tools.child(button("Record").id("record").on_click(act(m, |m| m.queue.push(Act::RecordStart)))),
    };
    tools = tools
        .child(button(".wav").id("attach_wav").on_click(move |cx, _| p2.borrow_mut().pick(cx, &["wav"])))
        .child(button("Contact").id("attach_contact").on_click(act(m, |m| {
            m.share_query.clear();
            m.screen = Screen::ShareContact;
        })));
    c = c.child(tools);
    c = c.child(row().gap(6).child(message(cx, &st.message).grow(1)).child(button("Send").id("send").style("primary").on_click(act(m, |m| m.send_compose()))));
    c
}

fn share_contact(m: &M, st: &Model, cx: &ViewCtx, h: i32) -> Element {
    let q1 = m.clone();
    let mut c = column().gap(6).child(row().gap(6).child(button("Back").id("back").on_click(act(m, |m| m.back()))).child(label("Share a contact").style("title").wrap(false)));
    c = c.child(text_field().id("share_query").placeholder("Search").text(st.share_query.clone()).on_change(move |cx, _| {
        let t = cx.text("share_query");
        q1.borrow_mut().share_query = t;
    }));
    let hits: Vec<(String, String, String)> = st.book.search(&st.share_query).iter().map(|c| (c.uid.clone(), c.name.clone(), c.initials())).collect();
    if hits.is_empty() {
        c = c.child(label("No contacts.").style("caption"));
    } else {
        let uids: Vec<String> = hits.iter().map(|h| h.0.clone()).collect();
        let mm = m.clone();
        let l = list().id("share").children(hits.iter().map(|(_, n, i)| row().gap(8).child(avatar(i, 24)).child(label(n.clone()).wrap(false)))).on_select(move |cx, _| {
            if let Some(u) = cx.selected("share").and_then(|i| uids.get(i).cloned()) {
                mm.borrow_mut().queue.push(Act::ShareContact(u));
            }
        });
        c = c.child(scroll_view(l).id("share_scroll").height((h - 80).max(60)));
    }
    let _ = cx;
    c
}

fn viewer(m: &M, st: &Model, cx: &ViewCtx, path: &str, w: i32, h: i32) -> Element {
    let p = path.to_string();
    let bar = row()
        .gap(6)
        .child(button("Back").id("back").on_click(act(m, |m| m.back())))
        .child(label(file_name(path).to_string()).wrap(false).grow(1))
        .child(button("Save to Files").id("save_picture").style("primary").on_click(act(m, move |m| m.queue.push(Act::SavePicture(p.clone())))));
    let pic = match st.parts.get(path) {
        Some(Part::Picture { res, w: iw, h: ih }) => {
            let (tw, th) = fit(*iw, *ih, w, h - 60);
            row().child(spacer()).child(image(*res, tw, th)).child(spacer())
        }
        _ => label("This picture can't be shown.").style("caption"),
    };
    column().gap(8).child(bar).child(pic).child(message(cx, &st.message))
}

#[cfg(test)]
mod tests;
