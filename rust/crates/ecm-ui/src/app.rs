//! The app framework: connecting, windows, the event pump and the lifecycle.
//!
//! [`Core`] is the language-neutral engine: it speaks the protocol, owns the
//! windows and turns incoming messages into [`AppEvent`]s. [`App`] is the
//! Rust front end that dispatches those events to closures. The Python `ui`
//! module drives the same `Core`, so both produce the same bytes for the same
//! app.
//!
//! One iteration of the loop, in both front ends:
//!
//! 1. render every configured window (TreeOps, then `AckConfigure` for a
//!    pending configure, then `Commit`) and flush;
//! 2. [`Core::step`]: wait for messages and turn them into events (it stops
//!    early after `WillSuspend`, so the app answers it before anything else);
//! 3. dispatch the events to handlers (which may change state and send
//!    messages).

use crate::conn::{default_connection, ConnError, Connection};
use crate::element::{Element, Handler};
use crate::json::Json;
use crate::theme::Theme;
use crate::window::{Content, UiEvent, ViewCtx, Window, WidgetState};
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

/// What happened, for the front end to dispatch.
#[derive(Clone, Debug)]
pub enum AppEvent {
    /// A widget event with the handler to call.
    Widget(UiEvent),
    /// An ePhone hotkey slot (1-6) went down or up.
    Hotkey { slot: u8, down: bool },
    /// Save state now: answer with `SaveState` (§11).
    Suspend { deadline_ms: u32 },
    /// The surface may draw its next animation frame.
    Frame { window: u32, time_ms: u64 },
    /// The player asked to close this window.
    CloseRequested { window: u32 },
    /// The theme changed (windows re-render with it).
    ThemeChanged,
    /// Any other message (clipboard data, picked files, notification
    /// actions, pointer lock changes, errors, …).
    Message(Message),
}

/// The protocol engine shared by the Rust and Python front ends.
pub struct Core {
    conn: Box<dyn Connection>,
    pub app_id: String,
    pub welcome: proto::Welcome,
    pub theme: Theme,
    pub windows: Vec<Window>,
    out: Vec<u8>,
    pending: VecDeque<Message>,
    restore: Option<Vec<u8>>,
    next_surface: u32,
    closed: bool,
    quit: bool,
}

impl Core {
    /// Open a connection in `role`, say Hello and wait for Welcome (and the
    /// saved state, if the compositor has some).
    pub fn connect(app_id: &str, mut conn: Box<dyn Connection>, role: proto::Role) -> Result<Core, ConnError> {
        let hello = Message::Hello(proto::Hello { version: proto::VERSION, role, capabilities: Vec::new() });
        conn.send(&hello.to_bytes())?;
        let mut pending = VecDeque::new();
        let welcome = loop {
            match pending.pop_front() {
                Some(Message::Welcome(w)) => break w,
                Some(_) => return Err(ConnError::Protocol),
                None => {
                    let bytes = conn.recv(-1)?;
                    pending.extend(Message::decode_all(&bytes).map_err(|_| ConnError::Protocol)?);
                }
            }
        };
        let mut restore = None;
        if welcome.restore_follows {
            loop {
                match pending.pop_front() {
                    Some(Message::Restore(r)) => {
                        restore = Some(r.data);
                        break;
                    }
                    Some(_) => return Err(ConnError::Protocol),
                    None => {
                        let bytes = conn.recv(-1)?;
                        pending.extend(Message::decode_all(&bytes).map_err(|_| ConnError::Protocol)?);
                    }
                }
            }
        }
        let theme = Theme::from_wire(&welcome.theme);
        Ok(Core {
            conn,
            app_id: app_id.to_string(),
            welcome,
            theme,
            windows: Vec::new(),
            out: Vec::new(),
            pending,
            restore,
            next_surface: 1,
            closed: false,
            quit: false,
        })
    }

    /// Queue a message (sent at the next flush).
    pub fn send(&mut self, m: Message) {
        m.encode(&mut self.out);
    }

    pub fn flush(&mut self) -> Result<(), ConnError> {
        if self.out.is_empty() {
            return Ok(());
        }
        let bytes = std::mem::take(&mut self.out);
        self.conn.send(&bytes)
    }

    pub fn connection(&mut self) -> &mut dyn Connection {
        &mut *self.conn
    }

    /// The saved state from a previous run (`Restore`), if any.
    pub fn restore(&self) -> Option<&[u8]> {
        self.restore.as_deref()
    }

    pub fn is_closed(&self) -> bool {
        self.closed || self.quit
    }

    /// Stop the loop after this iteration.
    pub fn quit(&mut self) {
        self.quit = true;
    }

    /// Create a tree window (sent as `SurfaceCreate`; it is drawn once the
    /// compositor configures it).
    pub fn create_window(&mut self, title: &str, content: Content) -> u32 {
        let surface = self.next_surface;
        self.next_surface += 1;
        self.send(Message::SurfaceCreate(proto::SurfaceCreate {
            surface,
            kind: proto::SurfaceKind::Tree,
            role: proto::SurfaceRole::Window,
            layer: proto::Layer::None,
            parent: 0,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            format: proto::PixelFormat::None,
            scale: 1,
            title: title.chars().take(64).collect(),
        }));
        self.windows.push(Window::new(surface, title, content));
        surface
    }

    pub fn close_window(&mut self, surface: u32) {
        if let Some(i) = self.windows.iter().position(|w| w.surface == surface) {
            self.windows.remove(i);
            self.send(Message::SurfaceDestroy(proto::SurfaceDestroy { surface }));
        }
    }

    pub fn window(&self, surface: u32) -> Option<&Window> {
        self.windows.iter().find(|w| w.surface == surface)
    }

    pub fn window_mut(&mut self, surface: u32) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.surface == surface)
    }

    pub fn set_root(&mut self, surface: u32, root: Element) {
        if let Some(w) = self.window_mut(surface) {
            w.set_content(Content::Static(root));
        }
    }

    pub fn mark_all_dirty(&mut self) {
        for w in &mut self.windows {
            w.dirty = true;
        }
    }

    /// Claim (or with an empty label release) an ePhone hotkey slot.
    pub fn claim_hotkey(&mut self, slot: u8, label: &str) {
        self.send(Message::HotkeyClaim(proto::HotkeyClaim { slot, label: label.chars().take(32).collect() }));
    }

    /// Render dirty, configured windows and flush.
    pub fn render(&mut self) -> Result<(), ConnError> {
        let mut out = Vec::new();
        for w in &mut self.windows {
            if !w.configured || !(w.dirty || w.pending_ack.is_some()) {
                continue;
            }
            let ops = w.render(&self.theme);
            let ack = w.pending_ack.take();
            let commit = !ops.is_empty() || ack.is_some();
            if !ops.is_empty() {
                out.push(Message::TreeOps(proto::TreeOps { surface: w.surface, ops }));
            }
            if let Some(serial) = ack {
                out.push(Message::AckConfigure(proto::AckConfigure { surface: w.surface, serial }));
            }
            if commit {
                out.push(Message::Commit(proto::Commit { surface: w.surface }));
            }
        }
        for m in out {
            self.send(m);
        }
        self.flush()
    }

    /// Wait up to `timeout_ms` for messages and turn them into events.
    /// `Err` once the connection is closed.
    pub fn step(&mut self, timeout_ms: i32) -> Result<Vec<AppEvent>, ConnError> {
        if self.is_closed() {
            return Err(ConnError::Closed);
        }
        self.flush()?;
        if self.pending.is_empty() {
            let bytes = match self.conn.recv(timeout_ms) {
                Ok(b) => b,
                Err(e) => {
                    self.closed = true;
                    return Err(e);
                }
            };
            match Message::decode_all(&bytes) {
                Ok(ms) => self.pending.extend(ms),
                Err(_) => {
                    self.closed = true;
                    return Err(ConnError::Protocol);
                }
            }
        }
        let mut events = Vec::new();
        while let Some(m) = self.pending.pop_front() {
            match m {
                Message::Configure(c) => {
                    if let Some(w) = self.window_mut(c.surface) {
                        w.configure(c.serial, c.w, c.h, c.states);
                    }
                }
                Message::Pointer(p) => {
                    let theme = self.theme.clone();
                    if let Some(w) = self.window_mut(p.surface) {
                        let evs = w.pointer(p.kind, p.x, p.y, p.scroll_y as i32, p.hit_id, &theme);
                        events.extend(evs.into_iter().map(AppEvent::Widget));
                    }
                }
                Message::Key(k) => {
                    if let Some(w) = self.window_mut(k.surface) {
                        events.extend(w.key(k.key, k.mods, k.down).into_iter().map(AppEvent::Widget));
                    }
                }
                Message::Text(t) => {
                    if let Some(w) = self.window_mut(t.surface) {
                        events.extend(w.text_input(&t.text).into_iter().map(AppEvent::Widget));
                    }
                }
                Message::Focus(f) => {
                    if let Some(w) = self.window_mut(f.surface) {
                        w.activated = f.gained;
                        w.dirty = true;
                    }
                }
                Message::Hotkey(h) => events.push(AppEvent::Hotkey { slot: h.slot, down: h.down }),
                Message::Frame(f) => events.push(AppEvent::Frame { window: f.surface, time_ms: f.time_ms }),
                Message::CloseRequested(c) => events.push(AppEvent::CloseRequested { window: c.surface }),
                Message::ThemeChanged(t) => {
                    self.theme = Theme::from_wire(&t.theme);
                    self.mark_all_dirty();
                    events.push(AppEvent::ThemeChanged);
                }
                Message::Restore(r) => self.restore = Some(r.data),
                Message::WillSuspend(s) => {
                    events.push(AppEvent::Suspend { deadline_ms: s.deadline_ms });
                    break;
                }
                Message::Error(e) => {
                    self.closed = true;
                    events.push(AppEvent::Message(Message::Error(e)));
                    break;
                }
                other => events.push(AppEvent::Message(other)),
            }
        }
        Ok(events)
    }

    /// Answer `WillSuspend` with the bytes to keep (§11).
    pub fn save_state(&mut self, data: Vec<u8>) {
        self.send(Message::SaveState(proto::SaveState { data }));
    }
}

/// What a handler can do: read and change widgets of its window, send
/// messages, request frames, quit.
pub struct Ctx<'a> {
    pub(crate) core: &'a mut Core,
    pub(crate) window: u32,
}

impl Ctx<'_> {
    pub fn new(core: &mut Core, window: u32) -> Ctx<'_> {
        Ctx { core, window }
    }

    /// The window this handler runs for (0 for hotkeys without windows).
    pub fn window(&self) -> u32 {
        self.window
    }

    fn state(&self, id: &str) -> Option<&WidgetState> {
        self.core.window(self.window).and_then(|w| w.state(id))
    }

    fn state_mut(&mut self, id: &str) -> Option<&mut WidgetState> {
        self.core.window_mut(self.window).map(|w| w.state_mut(id))
    }

    /// The text of the text field with this id.
    pub fn text(&self, id: &str) -> String {
        self.state(id).map(|s| s.text.clone()).unwrap_or_default()
    }

    pub fn set_text(&mut self, id: &str, text: &str) {
        if let Some(s) = self.state_mut(id) {
            s.text = text.to_string();
            s.cursor = s.text.chars().count();
        }
    }

    /// The text, emptying the field.
    pub fn take_text(&mut self, id: &str) -> String {
        let t = self.text(id);
        self.set_text(id, "");
        t
    }

    pub fn checked(&self, id: &str) -> bool {
        self.state(id).is_some_and(|s| s.checked)
    }

    pub fn set_checked(&mut self, id: &str, on: bool) {
        if let Some(s) = self.state_mut(id) {
            s.checked = on;
        }
    }

    pub fn value(&self, id: &str) -> i32 {
        self.state(id).map_or(0, |s| s.value)
    }

    pub fn set_value(&mut self, id: &str, v: i32) {
        if let Some(s) = self.state_mut(id) {
            s.value = v;
        }
    }

    pub fn selected(&self, id: &str) -> Option<usize> {
        self.state(id).and_then(|s| s.selected)
    }

    pub fn set_selected(&mut self, id: &str, i: Option<usize>) {
        if let Some(s) = self.state_mut(id) {
            s.selected = i;
        }
    }

    /// Move keyboard focus to the widget with this id.
    pub fn focus(&mut self, id: &str) {
        if let Some(w) = self.core.window_mut(self.window) {
            w.focus(Some(id));
        }
    }

    pub fn send(&mut self, m: Message) {
        self.core.send(m);
    }

    /// Ask for a `Frame` event at the next refresh (only while visible).
    pub fn request_frame(&mut self) {
        let surface = self.window;
        self.core.send(Message::RequestFrame(proto::RequestFrame { surface }));
    }

    pub fn close_window(&mut self) {
        let w = self.window;
        self.core.close_window(w);
    }

    pub fn quit(&mut self) {
        self.core.quit();
    }

    pub fn core(&mut self) -> &mut Core {
        self.core
    }
}

type Callback = Rc<dyn Fn(&mut Ctx, &UiEvent)>;
type Hook = Rc<dyn Fn(&mut Ctx)>;

/// A Rust GUI app (§13).
///
/// ```ignore
/// let mut app = App::new("dev.evan.notes");
/// app.window("Notes", |cx| column().gap(8).padding(12).child(label("Hello")));
/// app.run();
/// ```
pub struct App {
    pub core: Core,
    named: HashMap<String, Callback>,
    hotkeys: HashMap<u8, Hook>,
    on_suspend: Option<Box<dyn FnMut() -> Vec<u8>>>,
    on_frame: Option<Rc<dyn Fn(&mut Ctx, u64)>>,
    on_close: Option<Hook>,
    on_message: Option<Rc<dyn Fn(&mut Ctx, &Message)>>,
}

impl App {
    /// Connect to the compositor on this computer. Panics in console mode;
    /// use [`App::connect`] to handle that.
    pub fn new(app_id: &str) -> App {
        match App::connect(app_id) {
            Ok(a) => a,
            Err(e) => panic!("{app_id}: {e}"),
        }
    }

    pub fn connect(app_id: &str) -> Result<App, ConnError> {
        App::with_connection(app_id, default_connection(proto::host::ROLE_APP)?)
    }

    pub fn with_connection(app_id: &str, conn: Box<dyn Connection>) -> Result<App, ConnError> {
        Ok(App::from_core(Core::connect(app_id, conn, proto::Role::App)?))
    }

    pub fn from_core(core: Core) -> App {
        App { core, named: HashMap::new(), hotkeys: HashMap::new(), on_suspend: None, on_frame: None, on_close: None, on_message: None }
    }

    /// A window whose elements `view` builds on every render.
    pub fn window(&mut self, title: &str, view: impl FnMut(&ViewCtx) -> Element + 'static) -> u32 {
        self.core.create_window(title, Content::View(Box::new(view)))
    }

    /// A window showing a retained element tree (replace it with [`App::set_root`]).
    pub fn window_element(&mut self, title: &str, root: Element) -> u32 {
        self.core.create_window(title, Content::Static(root))
    }

    /// A window from a `.ui.json` layout (handlers by name, see [`App::handler`]).
    pub fn window_from_layout(&mut self, json: &str) -> Result<u32, String> {
        let layout = crate::layout_file::load(json)?;
        Ok(self.window_element(&layout.title, layout.root))
    }

    pub fn set_root(&mut self, window: u32, root: Element) {
        self.core.set_root(window, root);
    }

    /// Register a named handler (layout files refer to handlers by name).
    pub fn handler(&mut self, name: &str, f: impl Fn(&mut Ctx, &UiEvent) + 'static) {
        self.named.insert(name.to_string(), Rc::new(f));
    }

    /// Claim ePhone hotkey slot 1-6; `f` runs when it is pressed.
    pub fn hotkey(&mut self, slot: u8, label: &str, f: impl Fn(&mut Ctx) + 'static) {
        self.core.claim_hotkey(slot, label);
        self.hotkeys.insert(slot, Rc::new(f));
    }

    /// The state to save before a suspend (§11).
    pub fn on_suspend(&mut self, f: impl FnMut() -> Vec<u8> + 'static) {
        self.on_suspend = Some(Box::new(f));
    }

    /// [`App::on_suspend`] with JSON (what the Python module saves).
    pub fn on_suspend_json(&mut self, mut f: impl FnMut() -> Json + 'static) {
        self.on_suspend = Some(Box::new(move || f().to_string().into_bytes()));
    }

    /// Bytes saved by the previous run, if the app was restored.
    pub fn restore(&self) -> Option<&[u8]> {
        self.core.restore()
    }

    /// The saved state parsed as JSON (empty object if none or not JSON).
    pub fn restore_json(&self) -> Json {
        self.core
            .restore()
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(|s| Json::parse(s).ok())
            .unwrap_or(Json::Obj(Vec::new()))
    }

    pub fn on_frame(&mut self, f: impl Fn(&mut Ctx, u64) + 'static) {
        self.on_frame = Some(Rc::new(f));
    }

    /// Runs when the player asks to close a window (default: close it, and
    /// quit when no windows are left).
    pub fn on_close(&mut self, f: impl Fn(&mut Ctx) + 'static) {
        self.on_close = Some(Rc::new(f));
    }

    /// Messages the toolkit doesn't handle itself (clipboard, files, …).
    pub fn on_message(&mut self, f: impl Fn(&mut Ctx, &Message) + 'static) {
        self.on_message = Some(Rc::new(f));
    }

    fn first_window(&self) -> u32 {
        self.core.windows.first().map_or(0, |w| w.surface)
    }

    /// Call the handler for one event.
    pub fn dispatch(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Widget(e) => {
                let f = match &e.handler {
                    Handler::Callback(f) => Some(f.clone()),
                    Handler::Named(n) => self.named.get(n).cloned(),
                    Handler::Ref(_) => None,
                };
                if let Some(f) = f {
                    let mut cx = Ctx { core: &mut self.core, window: e.window };
                    f(&mut cx, &e);
                }
            }
            AppEvent::Hotkey { slot, down: true } => {
                if let Some(f) = self.hotkeys.get(&slot).cloned() {
                    let w = self.first_window();
                    f(&mut Ctx { core: &mut self.core, window: w });
                }
            }
            AppEvent::Hotkey { .. } | AppEvent::ThemeChanged => {}
            AppEvent::Suspend { .. } => {
                let data = self.on_suspend.as_mut().map(|f| f()).unwrap_or_default();
                self.core.save_state(data);
            }
            AppEvent::Frame { window, time_ms } => {
                if let Some(f) = self.on_frame.clone() {
                    f(&mut Ctx { core: &mut self.core, window }, time_ms);
                }
            }
            AppEvent::CloseRequested { window } => match self.on_close.clone() {
                Some(f) => f(&mut Ctx { core: &mut self.core, window }),
                None => {
                    self.core.close_window(window);
                    if self.core.windows.is_empty() {
                        self.core.quit();
                    }
                }
            },
            AppEvent::Message(m) => {
                if let Some(f) = self.on_message.clone() {
                    let w = self.first_window();
                    f(&mut Ctx { core: &mut self.core, window: w }, &m);
                }
            }
        }
    }

    /// One loop iteration: render, wait up to `timeout_ms`, dispatch.
    /// False once the connection is closed or the app quit.
    pub fn step(&mut self, timeout_ms: i32) -> bool {
        self.core.mark_all_dirty();
        if self.core.render().is_err() {
            return false;
        }
        let Ok(events) = self.core.step(timeout_ms) else { return false };
        for ev in events {
            self.dispatch(ev);
        }
        !self.core.is_closed()
    }

    /// Run until the connection closes or the app quits.
    pub fn run(&mut self) {
        while self.step(-1) {}
        let _ = self.core.flush();
    }
}
