//! **Phone** (`dev.ecm.dialer`, program `dialer`): calls over telephonyd's control
//! socket ([`ecm_tel::ctl::PORT`]).
//!
//! Tabs: **Favourites** (the address book's favourites; tap = call), **Recents**
//! (the call log, missed calls in red, the tab shows how many; tap = call back),
//! **Contacts** (search, a contact's numbers, tap a number = call), **Keypad**
//! (digits, `*`, `#`, the number formatted as it grows with the matching
//! contact's name, Delete, Call) and **Voicemail** (visual voicemail: play,
//! mark heard, delete, call back; the tab shows the new ones). The header has
//! the phone's own number and its VoLTE registration.
//!
//! Over the tabs: the **incoming call** screen (Answer / Decline / Message, or
//! End & answer / Hold & answer / Decline for a call waiting; hotkey 1 answers,
//! hotkey 2 declines) and the **in-call** screen (name, timer, HD, codec and
//! loss, Mute, Keypad (tones), Speaker, Add call, Hold, Swap and Merge with two
//! calls, End). After the last call ends the tab that was open comes back.
//!
//! Calls are followed from `calls` every [`POLL_MS`] and from the `incoming` and
//! `call` events. Arguments: `--incoming <id>` (telephonyd starts the app for a
//! ringing call), `--number <n>` (the keypad with that number), `--call <n>`.
//!
//! * **compact** (< 400 lu) and **medium**: the tabs, one page at a time.
//! * **wide** (>= 800 lu): pages on the left, the page, and Recents (or the
//!   keypad, when Recents is the page) beside it.

use crate::book::Book;
use crate::widgets::{avatar, card, message, pill, AMBER, GREEN, GREY, RED};
use crate::{fmt_ago, fmt_duration, pretty, CallLine, Daemon, Event, Files, LogLine, Tel, DIALER_ID, MESSAGES_ID, POLL_MS};
use ecm_tel::ctl::{enc, PORT};
use ecm_tel::number::format_partial;
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::element::{button, column, grid, label, list, panel, row, scroll_view, spacer, stack, tabs, text_field, Element};
use ecm_ui::{Align, App, ViewCtx};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Message, SizeClass};
use std::cell::RefCell;
use std::rc::Rc;

/// How often the voicemail box is fetched without a `voicemail` event (it is an HTTP request), ms.
pub const VOICEMAIL_MS: u64 = 30_000;
/// Hotkey slots on the incoming call screen.
pub const ANSWER_SLOT: u8 = 1;
pub const DECLINE_SLOT: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Favourites,
    Recents,
    Contacts,
    Keypad,
    Voicemail,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Favourites, Tab::Recents, Tab::Contacts, Tab::Keypad, Tab::Voicemail];
    fn title(self, compact: bool) -> &'static str {
        match (self, compact) {
            (Tab::Favourites, true) => "Favs",
            (Tab::Favourites, false) => "Favourites",
            (Tab::Recents, true) => "Recent",
            (Tab::Recents, false) => "Recents",
            (Tab::Contacts, _) => "Contacts",
            (Tab::Keypad, _) => "Keypad",
            (Tab::Voicemail, true) => "VM",
            (Tab::Voicemail, false) => "Voicemail",
        }
    }
}

/// A voicemail as `voicemail` lists it: `vm <id> <from> <time> <ms> <new|heard>`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vm {
    pub id: u64,
    pub from: String,
    pub time_ms: u64,
    pub ms: u64,
    pub new: bool,
}

impl Vm {
    pub fn parse(l: &str) -> Option<Vm> {
        let w: Vec<&str> = l.split_whitespace().collect();
        if w.len() != 6 || w[0] != "vm" {
            return None;
        }
        Some(Vm { id: w[1].parse().ok()?, from: ecm_tel::ctl::dec(w[2]), time_ms: w[3].parse().unwrap_or(0), ms: w[4].parse().unwrap_or(0), new: w[5] == "new" })
    }
}

/// A ringing call.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Incoming {
    pub id: u32,
    pub from: String,
    pub hidden: bool,
    /// Another call is up (call waiting).
    pub waiting: bool,
}

/// What the program does next (handlers queue, [`DialerApp::process`] runs).
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// A request on [`PORT`].
    Req(String),
    /// Start another app.
    Launch(String, Vec<String>),
    /// Open a file with its default app.
    Open(String),
}

/// The program's arguments.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Args {
    pub incoming: Option<u32>,
    pub number: Option<String>,
    pub call: Option<String>,
}

impl Args {
    pub fn parse(args: &[String]) -> Args {
        let mut a = Args::default();
        let mut i = 0;
        while i < args.len() {
            let v = args.get(i + 1).cloned();
            match args[i].as_str() {
                "--incoming" => a.incoming = v.and_then(|v| v.parse().ok()),
                "--number" => a.number = v,
                "--call" => a.call = v,
                _ => {
                    i += 1;
                    continue;
                }
            }
            i += 2;
        }
        a
    }
}

#[derive(Debug)]
pub struct Model {
    pub daemon: Daemon,
    pub status: Vec<(String, String)>,
    pub calls: Vec<CallLine>,
    /// The call log, newest first.
    pub log: Vec<LogLine>,
    pub voicemail: Vec<Vm>,
    pub book: Book,
    pub tab: Tab,
    /// What the keypad holds.
    pub typed: String,
    pub search: String,
    /// The contact open on the Contacts tab (uid).
    pub contact: Option<String>,
    /// The voicemail chosen on the Voicemail tab.
    pub vm_selected: Option<u64>,
    pub incoming: Option<Incoming>,
    /// Ringing calls answered or declined here (not shown again).
    pub dismissed: Vec<u32>,
    /// The tabs over a call (Add call), and the tab to go back to.
    pub adding: bool,
    pub return_tab: Option<Tab>,
    /// The in-call keypad (tones) and what was sent with it.
    pub tones: bool,
    pub dtmf: String,
    /// The loudspeaker (status `speaker`).
    pub speaker: bool,
    pub last_dialled: String,
    pub message: Option<(bool, String)>,
    pub queue: Vec<Act>,
    pub refresh: bool,
    pub refresh_vm: bool,
    /// telephonyd's clock at the last `status` (`now`): the log's and voicemail's times are on it.
    pub now_ms: u64,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            daemon: Daemon::Unknown,
            status: Vec::new(),
            calls: Vec::new(),
            log: Vec::new(),
            voicemail: Vec::new(),
            book: Book::default(),
            tab: Tab::Favourites,
            typed: String::new(),
            search: String::new(),
            contact: None,
            vm_selected: None,
            incoming: None,
            dismissed: Vec::new(),
            adding: false,
            return_tab: None,
            tones: false,
            dtmf: String::new(),
            speaker: false,
            last_dialled: String::new(),
            message: None,
            queue: Vec::new(),
            refresh: true,
            refresh_vm: true,
            now_ms: 0,
        }
    }
}

fn digits(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit()).collect()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

impl Model {
    pub fn get(&self, k: &str) -> &str {
        self.status.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str()).unwrap_or("")
    }

    pub fn missed(&self) -> usize {
        self.log.iter().filter(|l| l.dir == "missed").count()
    }

    pub fn new_voicemail(&self) -> usize {
        self.voicemail.iter().filter(|v| v.new).count()
    }

    /// Calls that are up (not ringing in): active, dialing, alerting, held.
    pub fn live(&self) -> Vec<&CallLine> {
        self.calls.iter().filter(|c| c.state != "incoming" && c.state != "ended").collect()
    }

    pub fn in_call(&self) -> bool {
        !self.live().is_empty()
    }

    /// The call the in-call screen is about: the active one, else one being set up, else a held one.
    pub fn current(&self) -> Option<&CallLine> {
        let live = self.live();
        ["active", "alerting", "dialing", "held"].iter().find_map(|s| live.iter().find(|c| c.state == *s).copied())
    }

    /// The other call (on hold, or the one merged with the current one).
    pub fn other(&self) -> Option<&CallLine> {
        let cur = self.current()?.id;
        self.live().into_iter().find(|c| c.id != cur)
    }

    /// The name to show for a call.
    pub fn who(&self, number: &str, hidden: bool) -> String {
        if hidden {
            "No Caller ID".into()
        } else {
            self.book.name_of(number)
        }
    }

    /// The contact the keypad's digits belong to.
    pub fn keypad_match(&self) -> Option<String> {
        let d = digits(&self.typed);
        if d.len() < 3 {
            return None;
        }
        self.book.find(&self.typed).or_else(|| self.book.search(&d).into_iter().next()).map(|c| c.name.clone())
    }

    /// Call a number.
    pub fn dial(&mut self, number: &str) {
        let n = number.trim();
        if n.is_empty() || n == "anonymous" {
            self.message = Some((false, "There is no number to call".into()));
            return;
        }
        self.queue.push(Act::Req(format!("dial {}", enc(n))));
        self.last_dialled = n.to_string();
        self.adding = false;
        self.message = None;
    }

    /// The keypad's Call button: the typed number, or with nothing typed the last number called.
    pub fn call_typed(&mut self) {
        if self.typed.is_empty() {
            let last = if self.last_dialled.is_empty() {
                self.log.iter().find(|l| l.dir == "out").map(|l| l.number.clone()).unwrap_or_default()
            } else {
                self.last_dialled.clone()
            };
            if last.is_empty() {
                self.message = Some((false, "Type a number first".into()));
            } else {
                self.typed = digits(&last);
                if last.starts_with('+') && self.typed.starts_with('1') {
                    self.typed.remove(0);
                }
            }
            return;
        }
        let t = self.typed.clone();
        self.dial(&t);
        self.typed.clear();
    }

    pub fn key(&mut self, k: char) {
        if self.tones {
            if let Some(c) = self.current() {
                let id = c.id;
                self.queue.push(Act::Req(format!("dtmf {id} {k}")));
                self.dtmf.push(k);
            }
            return;
        }
        if self.typed.chars().count() < 32 {
            self.typed.push(k);
        }
    }

    pub fn backspace(&mut self) {
        self.typed.pop();
    }

    pub fn answer(&mut self, how: &str) {
        let Some(inc) = self.incoming.take() else { return };
        self.dismissed.push(inc.id);
        let up = self.current().map(|c| c.id);
        match (how, up) {
            ("end", Some(a)) => self.queue.push(Act::Req(format!("hangup {a}"))),
            ("hold", Some(a)) => self.queue.push(Act::Req(format!("hold {a} on"))),
            _ => {}
        }
        self.queue.push(Act::Req(format!("answer {}", inc.id)));
        self.adding = false;
        self.tones = false;
    }

    /// Decline; `text`: then open Messages to the caller.
    pub fn decline(&mut self, text: bool) {
        let Some(inc) = self.incoming.take() else { return };
        self.dismissed.push(inc.id);
        self.queue.push(Act::Req(format!("decline {}", inc.id)));
        if text && !inc.hidden && !inc.from.is_empty() && inc.from != "anonymous" {
            self.queue.push(Act::Launch(MESSAGES_ID.into(), vec!["--to".into(), inc.from.clone()]));
        }
    }

    /// An in-call button.
    pub fn on_call(&mut self, what: &str) {
        let Some(c) = self.current().cloned() else { return };
        match what {
            "mute" => self.queue.push(Act::Req(format!("mute {} {}", c.id, if c.muted { "off" } else { "on" }))),
            "hold" => self.queue.push(Act::Req(format!("hold {} {}", c.id, if c.state == "held" { "off" } else { "on" }))),
            "speaker" => {
                // shown at once; the next status confirms it
                self.speaker = !self.speaker;
                self.queue.push(Act::Req(format!("speaker {}", if self.speaker { "on" } else { "off" })));
            }
            "keypad" => {
                self.tones = !self.tones;
                self.dtmf.clear();
            }
            "add" => {
                self.adding = true;
                self.tones = false;
                self.return_tab.get_or_insert(self.tab);
                self.tab = Tab::Keypad;
                self.typed.clear();
            }
            "swap" => self.queue.push(Act::Req("swap".into())),
            "merge" => self.queue.push(Act::Req("merge".into())),
            "end" => self.queue.push(Act::Req(format!("hangup {}", c.id))),
            _ => {}
        }
    }

    pub fn play_voicemail(&mut self, id: u64) {
        self.queue.push(Act::Req(format!("voicemail-fetch {id} /tmp/vm-{id}.wav")));
    }

    /// A notification was tapped.
    pub fn on_notification(&mut self, tag: &str) {
        match tag {
            "call:missed" => {
                self.tab = Tab::Recents;
                self.refresh = true;
            }
            "vm" => {
                self.tab = Tab::Voicemail;
                self.refresh_vm = true;
            }
            _ => {}
        }
        self.contact = None;
    }

    /// Program arguments.
    pub fn start(&mut self, a: &Args) {
        if let Some(id) = a.incoming {
            self.incoming = Some(Incoming { id, ..Default::default() });
        }
        if let Some(n) = &a.number {
            self.tab = Tab::Keypad;
            self.typed = n.chars().filter(|c| c.is_ascii_digit() || *c == '*' || *c == '#' || *c == '+').collect();
        }
        if let Some(n) = &a.call {
            let n = n.clone();
            self.dial(&n);
        }
    }

    /// An event from the watch.
    pub fn on_event(&mut self, e: &Event) {
        match e.name.as_str() {
            "incoming" => {
                if let Ok(id) = e.get("call").parse::<u32>() {
                    if !self.dismissed.contains(&id) {
                        self.incoming = Some(Incoming { id, from: e.get("from").into(), hidden: e.get("hidden") == "1", waiting: e.get("waiting") == "1" });
                    }
                }
                self.refresh = true;
            }
            "call" => {
                if e.get("state") == "ended" {
                    let r = e.get("reason");
                    self.message = Some(if r.is_empty() || r == "call ended" || r == "cancelled" { (true, "Call ended".into()) } else { (false, capitalize(r)) });
                }
                self.refresh = true;
            }
            "voicemail" => {
                self.refresh_vm = true;
                self.refresh = true;
            }
            "registration" => self.refresh = true,
            _ => {}
        }
    }

    /// `calls` came back: follow the ringing call and the end of the last one.
    pub fn set_calls(&mut self, calls: Vec<CallLine>) {
        let was = self.in_call();
        self.calls = calls;
        let ringing: Vec<CallLine> = self.calls.iter().filter(|c| c.state == "incoming").cloned().collect();
        let waiting = self.in_call();
        let keep = self.incoming.as_ref().and_then(|i| ringing.iter().find(|c| c.id == i.id)).or_else(|| ringing.iter().find(|c| !self.dismissed.contains(&c.id)));
        self.incoming = keep.map(|c| Incoming { id: c.id, from: c.number.clone(), hidden: c.hidden, waiting });
        let ids: Vec<u32> = self.calls.iter().map(|c| c.id).collect();
        self.dismissed.retain(|d| ids.contains(d));
        if was && !self.in_call() {
            self.adding = false;
            self.tones = false;
            self.dtmf.clear();
            if let Some(t) = self.return_tab.take() {
                self.tab = t;
            }
        }
    }
}

pub type M = Rc<RefCell<Model>>;

/// The Phone app over a [`Tel`] and the address book's [`Files`].
pub struct DialerApp<T: Tel, F: Files> {
    pub app: App,
    pub model: M,
    pub tel: T,
    pub files: F,
    pub window: u32,
    last_poll: Option<u64>,
    last_vm: Option<u64>,
    hotkeys: bool,
}

impl<T: Tel, F: Files> DialerApp<T, F> {
    pub fn new(conn: Box<dyn Connection>, tel: T, files: F) -> Result<Self, ConnError> {
        let mut app = App::with_connection(DIALER_ID, conn)?;
        let model: M = Rc::new(RefCell::new(Model::default()));
        let v = model.clone();
        let window = app.window("Phone", move |cx| view(&v, cx));
        let n = model.clone();
        app.on_message(move |cx, msg| {
            if let Message::NotificationAction(a) = msg {
                n.borrow_mut().on_notification(&a.tag);
                cx.invalidate();
            }
        });
        let k = model.clone();
        app.on_key(move |cx, key, mods| {
            let shift = mods & 0x22 != 0;
            let ch = match key {
                0x1e..=0x26 if shift && key == 0x20 => Some('#'),
                0x1e..=0x26 if shift && key == 0x25 => Some('*'),
                0x1e..=0x26 if !shift => Some((b'1' + (key - 0x1e) as u8) as char),
                0x27 if !shift => Some('0'),
                0x59..=0x61 => Some((b'1' + (key - 0x59) as u8) as char),
                0x62 => Some('0'),
                0x55 => Some('*'),
                _ => None,
            };
            let mut m = k.borrow_mut();
            let keypad = m.incoming.is_none() && (m.tones || (m.tab == Tab::Keypad && (!m.in_call() || m.adding)));
            if !keypad {
                return false;
            }
            match (ch, key) {
                (Some(c), _) => m.key(c),
                (None, 0x2a) if !m.tones => m.backspace(),
                (None, 0x28 | 0x58) if !m.tones => m.call_typed(),
                _ => return false,
            }
            cx.invalidate();
            true
        });
        Ok(DialerApp { app, model, tel, files, window, last_poll: None, last_vm: None, hotkeys: false })
    }

    /// Apply the program's arguments.
    pub fn start(&mut self, args: &Args) {
        self.model.borrow_mut().start(args);
    }

    pub fn step(&mut self, timeout_ms: i32, now_ms: u64) -> bool {
        let wait = if self.model.borrow().queue.is_empty() { if timeout_ms < 0 { 250 } else { timeout_ms.min(250) } } else { 0 };
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
        for e in self.tel.events() {
            self.model.borrow_mut().on_event(&e);
            changed = true;
        }
        let acts: Vec<Act> = std::mem::take(&mut self.model.borrow_mut().queue);
        for a in acts {
            changed = true;
            match a {
                Act::Req(r) => {
                    let res = self.tel.call(PORT, &r);
                    self.after(&r, res);
                }
                Act::Launch(app, args) => self.app.core.send(Message::Launch(proto::Launch { app_id: app, args })),
                Act::Open(path) => self.app.core.send(Message::Open(proto::Open { path })),
            }
        }
        let due = self.model.borrow().refresh || self.last_poll.is_none_or(|t| now_ms.saturating_sub(t) >= POLL_MS);
        if due {
            self.last_poll = Some(now_ms);
            changed |= self.poll(now_ms);
        }
        changed |= self.sync_hotkeys();
        if changed {
            self.redraw();
        }
    }

    fn after(&mut self, req: &str, res: Result<Result<Vec<String>, String>, String>) {
        let mut m = self.model.borrow_mut();
        m.refresh = true;
        let w: Vec<&str> = req.split_whitespace().collect();
        match res {
            Err(e) => m.daemon = Daemon::Down(e),
            Ok(Err(e)) => m.message = Some((false, capitalize(&e))),
            Ok(Ok(_)) => match w.as_slice() {
                ["voicemail-fetch", id, path] => {
                    let from = id.parse::<u64>().ok().and_then(|i| m.voicemail.iter().find(|v| v.id == i)).map(|v| m.book.name_of(&v.from)).unwrap_or_default();
                    m.queue.push(Act::Req(format!("voicemail-heard {id}")));
                    m.queue.push(Act::Open(path.to_string()));
                    m.message = Some((true, format!("Playing the voicemail from {from} (saved as {path})")));
                }
                ["voicemail-heard", id] | ["voicemail-delete", id] => {
                    let id: u64 = id.parse().unwrap_or(0);
                    if w[0] == "voicemail-delete" {
                        m.voicemail.retain(|v| v.id != id);
                        m.vm_selected = None;
                        m.message = Some((true, "Voicemail deleted".into()));
                    } else if let Some(v) = m.voicemail.iter_mut().find(|v| v.id == id) {
                        v.new = false;
                    }
                    m.refresh_vm = true;
                }
                _ => {}
            },
        }
    }

    fn poll(&mut self, now_ms: u64) -> bool {
        let mut m = self.model.borrow_mut();
        m.refresh = false;
        let before = (m.now_ms / 60_000, m.daemon.clone(), m.status.clone(), m.calls.clone(), m.log.clone(), m.voicemail.clone(), m.book.clone(), m.incoming.clone());
        match self.tel.call(PORT, "status") {
            Ok(Ok(body)) => {
                m.daemon = Daemon::Running;
                m.status = body
                    .iter()
                    .map(|l| {
                        let (k, v) = l.split_once(' ').unwrap_or((l, ""));
                        (k.to_string(), v.to_string())
                    })
                    .collect();
                m.now_ms = m.get("now").parse().unwrap_or(m.now_ms);
                m.speaker = m.get("speaker") == "1";
                // the clock alone redraws once a minute (the ages), not every poll
                m.status.retain(|(k, _)| k != "now");
                if let Ok(Ok(b)) = self.tel.call(PORT, "calls") {
                    m.set_calls(b.iter().filter_map(|l| CallLine::parse(l)).collect());
                }
                if let Ok(Ok(b)) = self.tel.call(PORT, "log 100") {
                    m.log = b.iter().filter_map(|l| LogLine::parse(l)).rev().collect();
                }
                let vm_due = m.refresh_vm || self.last_vm.is_none_or(|t| now_ms.saturating_sub(t) >= VOICEMAIL_MS);
                if vm_due {
                    m.refresh_vm = false;
                    self.last_vm = Some(now_ms);
                    if let Ok(Ok(b)) = self.tel.call(PORT, "voicemail") {
                        m.voicemail = b.iter().filter_map(|l| Vm::parse(l)).collect();
                    }
                }
            }
            Ok(Err(e)) | Err(e) => m.daemon = Daemon::Down(e),
        }
        m.book = Book::load(&mut self.files);
        if m.contact.as_ref().is_some_and(|u| m.book.by_uid(u).is_none()) {
            m.contact = None;
        }
        before != (m.now_ms / 60_000, m.daemon.clone(), m.status.clone(), m.calls.clone(), m.log.clone(), m.voicemail.clone(), m.book.clone(), m.incoming.clone())
    }

    /// Hotkeys 1 and 2 answer and decline while a call rings in.
    fn sync_hotkeys(&mut self) -> bool {
        let ringing = self.model.borrow().incoming.is_some();
        if ringing == self.hotkeys {
            return false;
        }
        self.hotkeys = ringing;
        if ringing {
            let (a, d) = (self.model.clone(), self.model.clone());
            self.app.hotkey(ANSWER_SLOT, "Answer", move |cx| {
                a.borrow_mut().answer("");
                cx.invalidate();
            });
            self.app.hotkey(DECLINE_SLOT, "Decline", move |cx| {
                d.borrow_mut().decline(false);
                cx.invalidate();
            });
        } else {
            self.app.core.claim_hotkey(ANSWER_SLOT, "");
            self.app.core.claim_hotkey(DECLINE_SLOT, "");
        }
        true
    }

    fn redraw(&mut self) {
        self.app.core.mark_all_dirty();
        let _ = self.app.core.render();
    }
}

// ------------------------------------------------------------ the window

const LINE: i32 = 15;
const TITLE: i32 = 20;
const TABS_H: i32 = LINE + 8 + 1;
const HEADER_H: i32 = TITLE + 4 + 16;

pub fn view(m: &M, cx: &ViewCtx) -> Element {
    let st = m.borrow();
    let pad = if cx.class == SizeClass::Compact { 6 } else { 10 };
    if let Daemon::Down(why) = &st.daemon {
        return column().gap(8).padding(pad).child(header(&st, cx)).child(not_running(cx, why));
    }
    if let Some(inc) = &st.incoming {
        return incoming(m, &st, inc, cx);
    }
    if st.in_call() && !st.adding {
        return in_call(m, &st, cx);
    }
    let mut body_h = cx.height - 2 * pad - HEADER_H - 8;
    let mut c = column().gap(8).padding(pad).child(header(&st, cx));
    if st.in_call() {
        let r = m.clone();
        let who = st.current().map(|x| st.who(&x.number, x.hidden)).unwrap_or_default();
        c = c.child(button(format!("Return to call with {who}")).style("primary").id("return").on_click(move |_, _| r.borrow_mut().adding = false));
        body_h -= 32;
    }
    if st.message.is_some() {
        c = c.child(message(cx, &st.message));
        body_h -= LINE + 8;
    }
    c.child(if cx.class == SizeClass::Wide { wide(m, &st, cx, body_h) } else { tabbed(m, &st, cx, body_h) })
}

fn header(st: &Model, cx: &ViewCtx) -> Element {
    let me = st.get("msisdn");
    let mut pills = row().gap(6);
    match &st.daemon {
        Daemon::Down(_) => pills = pills.child(pill("telephonyd not running", GREY)),
        Daemon::Unknown => pills = pills.child(pill("Connecting", GREY)),
        Daemon::Running => {
            pills = pills.child(match st.get("registration") {
                "registered" => pill("VoLTE", GREEN),
                "registering" | "authenticating" => pill("Registering", AMBER),
                "failed" => pill("Registration failed", RED),
                _ => pill("No service", GREY),
            });
            if cx.class != SizeClass::Compact && st.get("domain") != "-" && !st.get("domain").is_empty() {
                pills = pills.child(label(st.get("domain").to_string()).style("caption").wrap(false));
            }
        }
    }
    let mine = if me.is_empty() || me == "-" { "Phone".to_string() } else { format!("My number {}", pretty(me)) };
    column().gap(4).child(label(mine).style("title").wrap(false)).child(pills)
}

fn not_running(cx: &ViewCtx, why: &str) -> Element {
    card(
        cx,
        "telephonyd is not running",
        column()
            .gap(6)
            .child(label("The phone's calling service isn't answering. It starts with the phone; on a Terminal with a modem, type:"))
            .child(label("telephonyd &").color(cx.theme.accent))
            .child(label(format!("({why})")).style("caption")),
    )
}

/// A green button with white text (Answer, Call): a green panel, the text, and a
/// clickable label over the whole of it with the id (ecm-ui buttons take only theme colours).
fn green_button(text: &str, id: &str, h: i32, grow: u16, f: impl Fn() + 'static) -> Element {
    stack()
        .height(h)
        .grow(grow)
        .child(panel().fill(GREEN).radius(4))
        .child(label(text).color(0xffff_ffff).wrap(false).align(Align::Center))
        .child(label("").id(id).on_click(move |_, _| f()))
}

fn big_button(text: &str, id: &str, style: &str, f: impl Fn() + 'static) -> Element {
    button(text).id(id).style(style).height(36).grow(1).on_click(move |_, _| f())
}

fn incoming(m: &M, st: &Model, inc: &Incoming, cx: &ViewCtx) -> Element {
    let name = st.who(&inc.from, inc.hidden);
    let contact = if inc.hidden { None } else { st.book.find(&inc.from) };
    let initials = contact.map(|c| c.initials()).unwrap_or_else(|| "?".into());
    let mut who = column().gap(6).align(Align::Center).child(avatar(&initials, 72)).child(label(name).size(22).wrap(false));
    if contact.is_some() {
        who = who.child(label(pretty(&inc.from)).style("caption").wrap(false));
    }
    who = who.child(label(if inc.waiting { "Call waiting" } else { "Incoming call" }).style("caption"));
    let (a, b, c) = (m.clone(), m.clone(), m.clone());
    let buttons = if inc.waiting {
        column()
            .gap(8)
            .child(green_button("End & answer", "end_answer", 36, 1, move || a.borrow_mut().answer("end")))
            .child(green_button("Hold & answer", "hold_answer", 36, 1, move || b.borrow_mut().answer("hold")))
            .child(big_button("Decline", "decline", "danger", move || c.borrow_mut().decline(false)))
    } else {
        column()
            .gap(8)
            .child(row().gap(8).child(big_button("Decline", "decline", "danger", move || a.borrow_mut().decline(false))).child(green_button("Answer", "answer", 36, 1, move || b.borrow_mut().answer(""))))
            .child(big_button("Message", "message_reply", "", move || c.borrow_mut().decline(true)))
            .child(label("Hotkey 1 answers, hotkey 2 declines").style("caption").align(Align::Center))
    };
    let w = (cx.width - 24).min(360);
    column()
        .padding(12)
        .gap(24)
        .align(Align::Center)
        .child(spacer().height((cx.height / 10).max(4)))
        .child(who)
        .child(spacer().grow(1))
        .child(column().width(w).child(buttons))
}

fn in_call(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    let Some(c) = st.current() else { return column() };
    let other = st.other();
    let merged = c.merged && other.is_some_and(|o| o.merged);
    let name = if merged { format!("{} & {}", st.who(&c.number, c.hidden), other.map(|o| st.who(&o.number, o.hidden)).unwrap_or_default()) } else { st.who(&c.number, c.hidden) };
    let state = match c.state.as_str() {
        "dialing" => "Calling...".to_string(),
        "alerting" => "Ringing...".to_string(),
        "held" => format!("On hold  {}", fmt_duration(c.seconds)),
        _ => fmt_duration(c.seconds),
    };
    let mut tags = row().gap(6).align(Align::Center).child(label(state).size(18).wrap(false).id("timer"));
    if c.hd {
        tags = tags.child(pill("HD", GREEN));
    }
    if c.muted {
        tags = tags.child(pill("Muted", AMBER));
    }
    let mut top = column().gap(6).align(Align::Center).child(label(name).size(22).wrap(false));
    if !merged && !c.hidden && st.book.find(&c.number).is_some() {
        top = top.child(label(pretty(&c.number)).style("caption").wrap(false));
    }
    top = top.child(tags);
    if c.state == "active" && !c.codec.is_empty() {
        top = top.child(label(format!("{} · {:.1}% loss", c.codec, c.loss)).style("caption").wrap(false));
    }
    if let (Some(o), false) = (other, merged) {
        top = top.child(label(format!("On hold: {} {}", st.who(&o.number, o.hidden), fmt_duration(o.seconds))).style("caption").wrap(false));
    }
    let b = |text: &str, id: &str, on: bool, what: &'static str| {
        let mm = m.clone();
        button(text).id(id).style(if on { "primary" } else { "" }).height(36).grow(1).on_click(move |_, _| mm.borrow_mut().on_call(what))
    };
    let mut pad = column().gap(8);
    if st.tones {
        pad = pad.child(label(if st.dtmf.is_empty() { " ".to_string() } else { st.dtmf.clone() }).size(18).align(Align::Center).wrap(false).id("dtmf"));
        pad = pad.child(keys(m, "dtmf"));
        pad = pad.child(row().gap(8).child(b("Hide keypad", "keypad", false, "keypad")));
    } else {
        pad = pad
            .child(row().gap(8).child(b("Mute", "mute", c.muted, "mute")).child(b("Keypad", "keypad", false, "keypad")).child(b("Speaker", "speaker", st.speaker, "speaker")))
            .child(row().gap(8).child(b("Add call", "add", false, "add")).child(b(if c.state == "held" { "Resume" } else { "Hold" }, "hold", c.state == "held", "hold")));
        if other.is_some() {
            pad = pad.child(row().gap(8).child(b("Swap", "swap", false, "swap")).child(b("Merge", "merge", merged, "merge")));
        }
    }
    let e = m.clone();
    pad = pad.child(button("End").id("end").style("danger").height(36).on_click(move |_, _| e.borrow_mut().on_call("end")));
    let w = (cx.width - 24).min(360);
    column()
        .padding(12)
        .gap(16)
        .align(Align::Center)
        .child(spacer().height((cx.height / 14).max(4)))
        .child(top)
        .child(message(cx, &st.message))
        .child(spacer().grow(1))
        .child(column().width(w).child(pad))
}

/// The 12 keys; ids `<prefix>_1` .. `<prefix>_0`, `<prefix>_star`, `<prefix>_hash`.
fn keys(m: &M, prefix: &str) -> Element {
    let mut g = grid(3).gap(6);
    for k in ['1', '2', '3', '4', '5', '6', '7', '8', '9', '*', '0', '#'] {
        let id = match k {
            '*' => format!("{prefix}_star"),
            '#' => format!("{prefix}_hash"),
            d => format!("{prefix}_{d}"),
        };
        let mm = m.clone();
        g = g.child(button(k.to_string()).id(id).height(32).on_click(move |_, _| mm.borrow_mut().key(k)));
    }
    g
}

fn keypad(m: &M, st: &Model, w: i32) -> Element {
    let shown = format_partial(&st.typed);
    let name = st.keypad_match().unwrap_or_default();
    let (d, c) = (m.clone(), m.clone());
    column()
        .gap(8)
        .align(Align::Center)
        .child(label(if shown.is_empty() { " ".to_string() } else { shown }).size(22).wrap(false).id("typed"))
        .child(label(if name.is_empty() { " ".to_string() } else { name }).style("caption").wrap(false).id("match"))
        .child(column().width(w).gap(8).child(keys(m, "key")).child(
            row()
                .gap(8)
                .child(button("Delete").id("del").height(32).grow(1).enabled(!st.typed.is_empty()).on_click(move |_, _| d.borrow_mut().backspace()))
                .child(green_button("Call", "call", 32, 2, move || c.borrow_mut().call_typed())),
        ))
}

fn contact_row(name: &str, initials: &str, detail: String) -> Element {
    row().gap(8).child(avatar(initials, 28)).child(column().gap(1).grow(1).child(label(name.to_string()).wrap(false)).child(label(detail).style("caption").wrap(false)))
}

fn tel_label(l: &str) -> String {
    match l.to_ascii_lowercase().as_str() {
        "" => "Phone".into(),
        x => capitalize(x),
    }
}

fn favourites(m: &M, st: &Model) -> Element {
    let favs = st.book.favourites();
    if favs.is_empty() {
        return label("No favourites yet. Mark a contact as a favourite in Contacts.").style("caption");
    }
    let numbers: Vec<String> = favs.iter().map(|c| c.primary().unwrap_or("").to_string()).collect();
    let mm = m.clone();
    list()
        .id("favourites")
        .children(favs.iter().map(|c| contact_row(&c.name, &c.initials(), c.tels.first().map(|t| format!("{} {}", tel_label(&t.label), pretty(&t.number))).unwrap_or_default())))
        .on_select(move |cx, _| {
            if let Some(n) = cx.selected("favourites").and_then(|i| numbers.get(i)) {
                mm.borrow_mut().dial(n);
            }
        })
}

fn recents(m: &M, st: &Model) -> Element {
    if st.log.is_empty() {
        return label("No calls yet.").style("caption");
    }
    let numbers: Vec<String> = st.log.iter().map(|l| l.number.clone()).collect();
    let mm = m.clone();
    let rows = st.log.iter().map(|l| {
        let missed = l.dir == "missed";
        let name = st.book.name_of(&l.number);
        let what = match l.dir.as_str() {
            "missed" => "Missed".to_string(),
            "out" => "Outgoing".to_string(),
            _ => "Incoming".to_string(),
        };
        let detail = if l.seconds > 0 { format!("{what} · {}", fmt_duration(l.seconds)) } else { what };
        let n = label(name).wrap(false);
        let n = if missed { n.color(RED) } else { n };
        let left = column().gap(1).grow(1).child(n).child(label(detail).style("caption").wrap(false));
        row().gap(6).child(left).child(label(fmt_ago(l.start_ms, st.now_ms)).style("caption").wrap(false))
    });
    list().id("recents").children(rows).on_select(move |cx, _| {
        if let Some(n) = cx.selected("recents").and_then(|i| numbers.get(i)) {
            mm.borrow_mut().dial(n);
        }
    })
}

fn contacts(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    if let Some(c) = st.contact.as_ref().and_then(|u| st.book.by_uid(u)) {
        let back = m.clone();
        let mut col = column()
            .gap(8)
            .child(row().gap(8).child(button("Back").id("back").style("flat").on_click(move |_, _| back.borrow_mut().contact = None)).child(spacer()))
            .child(row().gap(10).child(avatar(&c.initials(), 48)).child(label(c.name.clone()).style("title").wrap(false)));
        if c.tels.is_empty() {
            col = col.child(label("No phone numbers.").style("caption"));
        } else {
            let numbers: Vec<String> = c.tels.iter().map(|t| t.number.clone()).collect();
            let mm = m.clone();
            col = col.child(
                list()
                    .id("numbers")
                    .children(c.tels.iter().map(|t| column().gap(1).child(label(tel_label(&t.label)).style("caption").wrap(false)).child(label(pretty(&t.number)).color(cx.theme.accent).wrap(false))))
                    .on_select(move |cx, _| {
                        if let Some(n) = cx.selected("numbers").and_then(|i| numbers.get(i)) {
                            mm.borrow_mut().dial(n);
                        }
                    }),
            );
        }
        return col;
    }
    let s = m.clone();
    let found = st.book.search(&st.search);
    let uids: Vec<String> = found.iter().map(|c| c.uid.clone()).collect();
    let mut col = column().gap(6).child(text_field().id("search").placeholder("Search").text(st.search.clone()).on_change(move |cx, _| {
        let t = cx.text("search");
        s.borrow_mut().search = t;
    }));
    if found.is_empty() {
        col = col.child(label(if st.book.contacts.is_empty() { "No contacts yet. Add them in Contacts." } else { "No matches." }).style("caption"));
    } else {
        let mm = m.clone();
        col = col.child(list().id("contacts").children(found.iter().map(|c| contact_row(&c.name, &c.initials(), c.primary().map(pretty).unwrap_or_default()))).on_select(move |cx, _| {
            if let Some(u) = cx.selected("contacts").and_then(|i| uids.get(i)) {
                mm.borrow_mut().contact = Some(u.clone());
            }
        }));
    }
    col
}

fn voicemail(m: &M, st: &Model) -> Element {
    if st.voicemail.is_empty() {
        return label("No voicemail.").style("caption");
    }
    let ids: Vec<u64> = st.voicemail.iter().map(|v| v.id).collect();
    let mm = m.clone();
    let sel = st.vm_selected.and_then(|s| ids.iter().position(|i| *i == s));
    let mut l = list()
        .id("voicemail")
        .children(st.voicemail.iter().map(|v| {
            let mut top = row().gap(6).child(label(st.book.name_of(&v.from)).wrap(false).grow(1));
            if v.new {
                top = top.child(pill("new", RED));
            }
            column().gap(1).child(top).child(label(format!("{} · {}", fmt_duration(v.ms.div_ceil(1000)), fmt_ago(v.time_ms, st.now_ms))).style("caption").wrap(false))
        }))
        .on_select(move |cx, _| {
            let id = cx.selected("voicemail").and_then(|i| ids.get(i).copied());
            mm.borrow_mut().vm_selected = id;
        });
    if let Some(i) = sel {
        l = l.selected(i);
    }
    let mut col = column().gap(8).child(l);
    if let Some(v) = st.vm_selected.and_then(|s| st.voicemail.iter().find(|v| v.id == s)) {
        let (id, from) = (v.id, v.from.clone());
        let (p, h, d, c) = (m.clone(), m.clone(), m.clone(), m.clone());
        col = col.child(
            row()
                .gap(6)
                .child(button("Play").id("vm_play").style("primary").on_click(move |_, _| p.borrow_mut().play_voicemail(id)))
                .child(button("Mark heard").id("vm_heard").enabled(v.new).on_click(move |_, _| h.borrow_mut().queue.push(Act::Req(format!("voicemail-heard {id}")))))
                .child(button("Call back").id("vm_call").on_click(move |_, _| c.borrow_mut().dial(&from)))
                .child(button("Delete").id("vm_delete").style("danger").on_click(move |_, _| d.borrow_mut().queue.push(Act::Req(format!("voicemail-delete {id}"))))),
        );
    }
    col
}

fn page(m: &M, st: &Model, cx: &ViewCtx, t: Tab, w: i32) -> Element {
    match t {
        Tab::Favourites => favourites(m, st),
        Tab::Recents => recents(m, st),
        Tab::Contacts => contacts(m, st, cx),
        Tab::Keypad => keypad(m, st, w.min(300)),
        Tab::Voicemail => voicemail(m, st),
    }
}

fn tab_title(st: &Model, t: Tab, compact: bool) -> String {
    let n = match t {
        Tab::Recents => st.missed(),
        Tab::Voicemail => st.new_voicemail(),
        _ => 0,
    };
    if n > 0 {
        format!("{} ({n})", t.title(compact))
    } else {
        t.title(compact).to_string()
    }
}

fn select_tab(m: &M, t: Tab) {
    let mut md = m.borrow_mut();
    md.tab = t;
    if t == Tab::Voicemail {
        md.refresh_vm = true;
        md.refresh = true;
    }
}

fn tabbed(m: &M, st: &Model, cx: &ViewCtx, h: i32) -> Element {
    let compact = cx.class == SizeClass::Compact;
    let sel = Tab::ALL.iter().position(|p| *p == st.tab).unwrap_or(0);
    let mut pages: Vec<Element> = (0..Tab::ALL.len()).map(|_| spacer()).collect();
    pages[sel] = scroll_view(page(m, st, cx, st.tab, cx.width - 40)).id(format!("page_{sel}")).height((h - TABS_H).max(60));
    let t = m.clone();
    tabs(Tab::ALL.iter().map(|p| tab_title(st, *p, compact))).id("tabs").selected(sel).grow(1).children(pages).on_select(move |cx, _| {
        let i = cx.selected("tabs").unwrap_or(0);
        select_tab(&t, Tab::ALL[i.min(Tab::ALL.len() - 1)]);
    })
}

fn wide(m: &M, st: &Model, cx: &ViewCtx, h: i32) -> Element {
    let sel = Tab::ALL.iter().position(|p| *p == st.tab).unwrap_or(0);
    let t = m.clone();
    let nav = list()
        .id("nav")
        .selected(sel)
        .children(Tab::ALL.iter().map(|p| {
            let n = match p {
                Tab::Recents if st.missed() > 0 => label(st.missed().to_string()).color(RED).wrap(false),
                Tab::Voicemail if st.new_voicemail() > 0 => label(st.new_voicemail().to_string()).color(RED).wrap(false),
                _ => label(""),
            };
            row().gap(6).child(label(p.title(false)).wrap(false)).child(spacer()).child(n)
        }))
        .on_select(move |cx, _| {
            let i = cx.selected("nav").unwrap_or(0);
            select_tab(&t, Tab::ALL[i.min(Tab::ALL.len() - 1)]);
        });
    let side = panel().fill(cx.theme.surface).radius(6).padding(8).child(column().gap(12).width(150).child(nav));
    let main = panel().fill(cx.theme.surface).radius(6).padding(8).grow(1).child(column().gap(6).child(label(st.tab.title(false)).style("title")).child(scroll_view(page(m, st, cx, st.tab, cx.width - 520)).id(format!("page_{sel}")).height(h - 40)));
    let (extra_tab, extra_title) = if st.tab == Tab::Recents { (Tab::Keypad, "Keypad") } else { (Tab::Recents, "Recents") };
    let extra = panel().fill(cx.theme.surface).radius(6).padding(8).width(300).child(column().gap(6).child(label(extra_title).style("title")).child(scroll_view(page(m, st, cx, extra_tab, 270)).id("extra").height(h - 40)));
    row().gap(12).align(Align::Start).child(side).child(main).child(extra)
}

#[cfg(test)]
mod tests;
