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
//!
//! `Core` also runs the toolkit's services: the text widgets' clipboard keys
//! become `ClipboardSet` / `ClipboardRequest` (and the answer is pasted into
//! the widget that asked), popup menus are popup surfaces that close
//! themselves, carets blink (when the connection has a clock), and images
//! and canvases are uploaded through [`Core::image_rgba`] /
//! [`Core::canvas`].
//!
//! Timed canvases (protocol v2): [`Core::put_canvas_at`] (or
//! [`Core::put_canvas`] then [`Core::commit_at`]) has the compositor show a
//! canvas frame at a time on the program's monotonic clock
//! ([`crate::conn::monotonic_us`]) with `CommitAt`, so video stays smooth
//! when the server or the network hitch; [`Core::queue_flush`] drops what is
//! queued and [`Core::set_content`] marks a canvas as motion (lossy allowed).
//! The scroll views take the client's `Scrolled` (it scrolled them at once).

use crate::conn::{default_connection, ConnError, Connection};
use crate::element::{menu, Element, EventKind, Handler};
use crate::json::Json;
use crate::theme::Theme;
use crate::window::{self, Content, Effect, UiEvent, ViewCtx, WidgetState, Window, WindowRole, TEXT_MIME};
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

/// How long `Core::connect` waits for `Welcome` / `Restore`.
pub const HANDSHAKE_TIMEOUT_MS: i32 = 5000;
/// Caret blink half-period.
pub const BLINK_MS: u64 = 530;

/// What happened, for the front end to dispatch.
#[derive(Clone, Debug)]
pub enum AppEvent {
    /// A widget event with the handler to call.
    Widget(UiEvent),
    /// A key no widget used (shortcuts like Ctrl+S): pressed, repeated or released.
    Key { window: u32, key: u16, mods: u16, down: bool, repeat: bool },
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
    /// actions, pointer lock changes, shell messages, errors, …).
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
    next_resource: u32,
    closed: bool,
    quit: bool,
    /// A text widget asked for the clipboard: (window, widget key).
    pending_paste: Option<(u32, String)>,
    blink_t0: u64,
    /// How far `CommitAt` may queue (`QueueLimits`, protocol v2; None from a v1 compositor).
    queue_limits: Option<proto::QueueLimits>,
    /// Content hints set before any window existed (sent with the first one).
    pending_content: Vec<(u32, u8)>,
}

impl Core {
    /// Open a connection in `role`, say Hello and wait for Welcome (and the
    /// saved state, if the compositor has some).
    pub fn connect(app_id: &str, mut conn: Box<dyn Connection>, role: proto::Role) -> Result<Core, ConnError> {
        let hello = Message::Hello(proto::Hello { version: proto::VERSION, role, capabilities: Vec::new() });
        conn.send(&hello.to_bytes())?;
        let mut pending = VecDeque::new();
        // The compositor answers at once; silence means nobody is there.
        let wait = |conn: &mut Box<dyn Connection>, pending: &mut VecDeque<Message>| -> Result<(), ConnError> {
            let bytes = conn.recv(HANDSHAKE_TIMEOUT_MS)?;
            if bytes.is_empty() {
                return Err(ConnError::Protocol);
            }
            pending.extend(Message::decode_all(&bytes).map_err(|_| ConnError::Protocol)?);
            Ok(())
        };
        let welcome = loop {
            match pending.pop_front() {
                Some(Message::Welcome(w)) => break w,
                Some(_) => return Err(ConnError::Protocol),
                None => wait(&mut conn, &mut pending)?,
            }
        };
        let mut restore = None;
        let mut queue_limits = None;
        // Version 2: QueueLimits comes right after Welcome (before Restore).
        if let Some(Message::QueueLimits(_)) = pending.front() {
            if let Some(Message::QueueLimits(q)) = pending.pop_front() {
                queue_limits = Some(q);
            }
        }
        if welcome.restore_follows {
            loop {
                match pending.pop_front() {
                    Some(Message::Restore(r)) => {
                        restore = Some(r.data);
                        break;
                    }
                    Some(Message::QueueLimits(q)) => queue_limits = Some(q),
                    Some(_) => return Err(ConnError::Protocol),
                    None => wait(&mut conn, &mut pending)?,
                }
            }
        }
        let theme = Theme::from_wire(&welcome.theme);
        let blink_t0 = conn.now_ms().unwrap_or(0);
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
            next_resource: 1,
            closed: false,
            quit: false,
            pending_paste: None,
            blink_t0,
            queue_limits,
            pending_content: Vec::new(),
        })
    }

    /// `(max_queue_frames, max_queue_ms)`: how far ahead `CommitAt` may queue
    /// (`QueueLimits`; None from a version 1 compositor).
    pub fn queue_limits(&self) -> Option<(u16, u32)> {
        self.queue_limits.as_ref().map(|q| (q.max_queue_frames, q.max_queue_ms))
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

    /// The connection's clock (None for scripted connections).
    pub fn now_ms(&mut self) -> Option<u64> {
        self.conn.now_ms()
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

    fn alloc_surface(&mut self) -> u32 {
        let s = self.next_surface;
        self.next_surface += 1;
        s
    }

    /// Create a tree window (sent as `SurfaceCreate`; it is drawn once the
    /// compositor configures it).
    pub fn create_window(&mut self, title: &str, content: Content) -> u32 {
        let surface = self.alloc_surface();
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
        for (resource, hint) in std::mem::take(&mut self.pending_content) {
            self.send(Message::SurfaceSetContent(proto::SurfaceSetContent { surface, resource, hint }));
        }
        surface
    }

    /// A shell layer surface (`layer` role, shells only) at `(x, y)`, `w x h`
    /// lu (0 = the display's width / height).
    pub fn create_layer(&mut self, layer: proto::Layer, x: i32, y: i32, w: i32, h: i32, title: &str, content: Content) -> u32 {
        let surface = self.alloc_surface();
        self.send(Message::SurfaceCreate(proto::SurfaceCreate {
            surface,
            kind: proto::SurfaceKind::Tree,
            role: proto::SurfaceRole::Layer,
            layer,
            parent: 0,
            x,
            y,
            w,
            h,
            format: proto::PixelFormat::None,
            scale: 1,
            title: title.chars().take(64).collect(),
        }));
        let mut win = Window::new(surface, title, content);
        win.role = WindowRole::Layer;
        win.activated = false;
        self.windows.push(win);
        surface
    }

    /// A popup surface anchored at `(x, y)` lu in `parent`, `w x h` lu. It is
    /// dismissed by a click outside it (`PopupDone`, then it is destroyed).
    pub fn create_popup(&mut self, parent: u32, x: i32, y: i32, w: i32, h: i32, content: Content) -> u32 {
        let surface = self.alloc_surface();
        self.send(Message::SurfaceCreate(proto::SurfaceCreate {
            surface,
            kind: proto::SurfaceKind::Tree,
            role: proto::SurfaceRole::Popup,
            layer: proto::Layer::None,
            parent,
            x,
            y,
            w: w.max(1),
            h: h.max(1),
            format: proto::PixelFormat::None,
            scale: 1,
            title: "popup".into(),
        }));
        let mut win = Window::new(surface, "popup", content);
        win.role = WindowRole::Popup { parent };
        self.windows.push(win);
        surface
    }

    /// A popup menu at `(x, y)` in `parent`: choosing an item calls `handler`
    /// with a `Select` event (`EventValue::Index`, `window` = the parent) and
    /// closes the menu; a click elsewhere just closes it. The menu is kept
    /// inside the parent window (moved left or up where it would leave it).
    pub fn popup_menu<S: Into<String>>(&mut self, parent: u32, x: i32, y: i32, items: impl IntoIterator<Item = S>, handler: Handler) -> u32 {
        self.popup_menu_placed(parent, Place::At(x, y), items, handler)
    }

    /// [`Core::popup_menu`] for a button's menu: below `anchor` (the button's
    /// rect in `parent`), or above it when there is no room below (a button in
    /// a phone's bottom bar).
    pub fn popup_menu_below<S: Into<String>>(&mut self, parent: u32, anchor: window::Rect, items: impl IntoIterator<Item = S>, handler: Handler) -> u32 {
        self.popup_menu_placed(parent, Place::Below(anchor), items, handler)
    }

    fn popup_menu_placed<S: Into<String>>(&mut self, parent: u32, place: Place, items: impl IntoIterator<Item = S>, handler: Handler) -> u32 {
        let el = menu(items).id("popup_menu").on(EventKind::Select, handler);
        let (mw, mh) = window::measure(&el, &self.theme, proto::SizeClass::Compact, 400);
        let w = mw.max(120);
        let el = el.width(w);
        let (pw, ph) = self.window(parent).map_or((0, 0), |p| (p.width, p.height));
        let (x, y) = fit_popup(place, w, mh, pw, ph);
        let s = self.create_popup(parent, x, y, w, mh, Content::Static(el));
        if let Some(win) = self.window_mut(s) {
            win.close_on_select = true;
        }
        s
    }

    /// Close every popup of `parent` (Escape, or the app is done with them).
    pub fn close_popups(&mut self, parent: u32) {
        let ids: Vec<u32> = self.windows.iter().filter(|w| w.role == WindowRole::Popup { parent }).map(|w| w.surface).collect();
        for id in ids {
            self.close_window(id);
        }
    }

    pub fn close_window(&mut self, surface: u32) {
        // Popups go with their parent.
        let kids: Vec<u32> = self.windows.iter().filter(|w| w.role == WindowRole::Popup { parent: surface }).map(|w| w.surface).collect();
        for k in kids {
            self.close_window(k);
        }
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

    /// Change a window's title (the shell shows it).
    pub fn set_title(&mut self, surface: u32, title: &str) {
        let title: String = title.chars().take(64).collect();
        if let Some(w) = self.window_mut(surface) {
            if w.title == title {
                return;
            }
            w.title = title.clone();
        }
        self.send(Message::SurfaceSetTitle(proto::SurfaceSetTitle { surface, title }));
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

    /// Put text on the clipboard.
    pub fn copy_text(&mut self, text: &str) {
        let mut data = text.as_bytes().to_vec();
        data.truncate(1_048_000);
        while std::str::from_utf8(&data).is_err() {
            data.pop();
        }
        self.send(Message::ClipboardSet(proto::ClipboardSet { mime: TEXT_MIME.into(), data }));
    }

    /// Ask for the clipboard's text for widget `key` of `window` (a text
    /// widget pastes it; a custom area gets a `Paste` event). The compositor
    /// only answers right after a paste key or a click in the focused window.
    pub fn paste_into(&mut self, window: u32, key: &str) {
        let key = if key.starts_with('#') || key.starts_with('/') { key.to_string() } else { format!("#{key}") };
        self.pending_paste = Some((window, key));
        self.send(Message::ClipboardRequest(proto::ClipboardRequest { mime: TEXT_MIME.into() }));
    }

    fn alloc_resource(&mut self) -> u32 {
        let r = self.next_resource;
        self.next_resource += 1;
        r
    }

    /// An image resource from raw RGBA8888 rows; returns its id for `image()`.
    pub fn image_rgba(&mut self, w: i32, h: i32, rgba: &[u8]) -> Result<u32, ConnError> {
        let id = self.alloc_resource();
        self.send(Message::ResourceCreate(proto::ResourceCreate { id, kind: proto::ResourceKind::Image, w, h, format: proto::PixelFormat::Rgba8888 }));
        self.flush()?;
        self.conn.upload(id, 1, rgba)?;
        Ok(id)
    }

    /// An image resource from a PNG file of `w x h` pixels.
    pub fn image_png(&mut self, w: i32, h: i32, png: &[u8]) -> Result<u32, ConnError> {
        let id = self.alloc_resource();
        self.send(Message::ResourceCreate(proto::ResourceCreate { id, kind: proto::ResourceKind::Image, w, h, format: proto::PixelFormat::Rgba8888 }));
        self.flush()?;
        self.conn.upload(id, 0, png)?;
        Ok(id)
    }

    /// A canvas resource (`canvas()` nodes show it); fill it with [`Core::put_canvas`].
    pub fn canvas(&mut self, w: i32, h: i32, format: proto::PixelFormat) -> u32 {
        let id = self.alloc_resource();
        self.send(Message::ResourceCreate(proto::ResourceCreate { id, kind: proto::ResourceKind::Canvas, w, h, format }));
        id
    }

    /// New pixels for a canvas (the whole buffer, `damage` = changed rects, empty = all);
    /// shown at the next commit, which this asks for.
    pub fn put_canvas(&mut self, id: u32, pixels: &[u8], damage: &[[i32; 4]]) -> Result<(), ConnError> {
        self.flush()?;
        self.conn.put_pixels(id, pixels, damage)?;
        if let Some(w) = self.windows.iter_mut().find(|w| w.configured) {
            w.force_commit = true;
        }
        Ok(())
    }

    /// The surface timed canvas frames commit on: the first configured window
    /// (the one [`Core::put_canvas`] commits).
    fn timed_surface(&self) -> Option<u32> {
        self.windows.iter().find(|w| w.configured).map(|w| w.surface)
    }

    /// New pixels for a canvas, shown at `pts_us` on the program's monotonic
    /// clock ([`crate::conn::monotonic_us`]) instead of at the next commit:
    /// `CommitAt` (protocol v2). Pending tree changes go first with a normal
    /// `Commit` (a `CommitAt` may only carry pixels). Before the window is
    /// configured the pixels wait for the first commit, as with `put_canvas`.
    pub fn put_canvas_at(&mut self, id: u32, pixels: &[u8], damage: &[[i32; 4]], pts_us: u64) -> Result<(), ConnError> {
        self.render()?;
        self.conn.put_pixels(id, pixels, damage)?;
        match self.timed_surface() {
            Some(surface) => {
                self.send(Message::CommitAt(proto::CommitAt { surface, pts_us }));
                self.flush()
            }
            None => Ok(()),
        }
    }

    /// Commit the canvas pixels put since the last commit ([`Core::put_canvas`])
    /// for `pts_us` (`CommitAt`) instead of at the next loop turn. No-op
    /// before a window is configured.
    pub fn commit_at(&mut self, pts_us: u64) -> Result<(), ConnError> {
        let Some(surface) = self.timed_surface() else { return Ok(()) };
        // Tree changes are only sent by render, which commits them: none are pending here.
        if let Some(w) = self.window_mut(surface) {
            w.force_commit = false;
        }
        self.flush()?;
        self.send(Message::CommitAt(proto::CommitAt { surface, pts_us }));
        self.flush()
    }

    /// Drop the queued `CommitAt` frames (seek, pause, stop): `QueueFlush`.
    pub fn queue_flush(&mut self) -> Result<(), ConnError> {
        let surface = self.timed_surface().or_else(|| self.windows.first().map(|w| w.surface));
        if let Some(surface) = surface {
            self.send(Message::QueueFlush(proto::QueueFlush { surface }));
        }
        self.flush()
    }

    /// How a canvas (`resource`; 0 = a pixel surface itself) may travel to
    /// players, the hint `gfx_set_hint` takes: 0 auto, 1 exact (never lossy),
    /// 2 motion (video, lossy allowed): `SurfaceSetContent`.
    pub fn set_content(&mut self, resource: u32, hint: u8) {
        let hint = hint.min(2);
        match self.windows.iter().find(|w| w.role == WindowRole::Window).or_else(|| self.windows.first()).map(|w| w.surface) {
            Some(surface) => self.send(Message::SurfaceSetContent(proto::SurfaceSetContent { surface, resource, hint })),
            None => self.pending_content.push((resource, hint)),
        }
    }

    pub fn destroy_resource(&mut self, id: u32) {
        self.send(Message::ResourceDestroy(proto::ResourceDestroy { id }));
    }

    /// Render dirty, configured windows and flush.
    pub fn render(&mut self) -> Result<(), ConnError> {
        let mut out = Vec::new();
        for w in &mut self.windows {
            if !w.configured || !(w.dirty || w.pending_ack.is_some() || w.force_commit) {
                continue;
            }
            let ops = if w.dirty || w.pending_ack.is_some() { w.render(&self.theme) } else { Vec::new() };
            let ack = w.pending_ack.take();
            let commit = !ops.is_empty() || ack.is_some() || w.force_commit;
            w.force_commit = false;
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

    /// How long to wait at most so a caret blinks on time.
    fn blink_wait(&mut self, timeout_ms: i32) -> i32 {
        let Some(now) = self.conn.now_ms() else { return timeout_ms };
        if !self.windows.iter().any(|w| w.caret_shown()) {
            return timeout_ms;
        }
        let since = now.saturating_sub(self.blink_t0);
        let wait = (BLINK_MS - since % BLINK_MS) as i32 + 1;
        if timeout_ms < 0 {
            wait
        } else {
            timeout_ms.min(wait)
        }
    }

    fn update_blink(&mut self) {
        let Some(now) = self.conn.now_ms() else { return };
        let on = (now.saturating_sub(self.blink_t0) / BLINK_MS) % 2 == 0;
        for w in &mut self.windows {
            if w.caret_on != on && w.caret_shown() {
                w.caret_on = on;
                w.dirty = true;
            }
        }
    }

    /// Input arrived: the caret shows solid again.
    fn restart_blink(&mut self) {
        if let Some(now) = self.conn.now_ms() {
            self.blink_t0 = now;
        }
        for w in &mut self.windows {
            if !w.caret_on {
                w.caret_on = true;
                w.dirty = true;
            }
        }
    }

    /// Clipboard effects a window's widgets left.
    fn run_effects(&mut self, surface: u32) {
        let effects = self.window_mut(surface).map(|w| w.take_effects()).unwrap_or_default();
        for e in effects {
            match e {
                Effect::Copy(t) => self.copy_text(&t),
                Effect::Paste(key) => self.paste_into(surface, &key),
            }
        }
    }

    /// Wait up to `timeout_ms` for messages and turn them into events.
    /// `Err` once the connection is closed.
    pub fn step(&mut self, timeout_ms: i32) -> Result<Vec<AppEvent>, ConnError> {
        if self.is_closed() {
            return Err(ConnError::Closed);
        }
        self.flush()?;
        if self.pending.is_empty() {
            let wait = self.blink_wait(timeout_ms);
            let bytes = match self.conn.recv(wait) {
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
                    let now = self.conn.now_ms();
                    if p.kind == proto::PointerKind::Down {
                        self.restart_blink();
                    }
                    let mut close = None;
                    if let Some(w) = self.window_mut(p.surface) {
                        let mut evs = w.pointer_full(p.kind, p.x, p.y, p.buttons, p.button, p.scroll_y as i32, p.hit_id, &theme, now);
                        if let WindowRole::Popup { parent } = w.role {
                            if w.close_on_select && evs.iter().any(|e| e.kind == EventKind::Select) {
                                close = Some(w.surface);
                                for e in &mut evs {
                                    e.window = parent;
                                }
                            }
                        }
                        events.extend(evs.into_iter().map(AppEvent::Widget));
                    }
                    if let Some(s) = close {
                        self.close_window(s);
                    }
                }
                Message::Key(k) => {
                    self.restart_blink();
                    if k.down && k.key == window::keys::ESCAPE && self.windows.iter().any(|w| w.role == WindowRole::Popup { parent: k.surface }) {
                        self.close_popups(k.surface);
                        continue;
                    }
                    let mut used = false;
                    if let Some(w) = self.window_mut(k.surface) {
                        let (evs, u) = w.key_event(k.key, k.mods, k.down, k.repeat);
                        used = u;
                        events.extend(evs.into_iter().map(AppEvent::Widget));
                    }
                    self.run_effects(k.surface);
                    if !used {
                        events.push(AppEvent::Key { window: k.surface, key: k.key, mods: k.mods, down: k.down, repeat: k.repeat });
                    }
                }
                Message::Text(t) => {
                    self.restart_blink();
                    if let Some(w) = self.window_mut(t.surface) {
                        events.extend(w.text_input(&t.text).into_iter().map(AppEvent::Widget));
                    }
                }
                Message::Focus(f) => {
                    if let Some(w) = self.window_mut(f.surface) {
                        w.activated = f.gained;
                        w.dirty = true;
                    }
                    if !f.gained {
                        self.close_popups(f.surface);
                    }
                }
                Message::ClipboardData(d) => match self.pending_paste.take() {
                    Some((surface, key)) => {
                        if !d.mime.is_empty() {
                            let text = String::from_utf8_lossy(&d.data).into_owned();
                            if let Some(w) = self.window_mut(surface) {
                                events.extend(w.paste(&key, &text).into_iter().map(AppEvent::Widget));
                            }
                        }
                    }
                    None => events.push(AppEvent::Message(Message::ClipboardData(d))),
                },
                Message::PopupDone(p) => {
                    if self.window(p.surface).is_some() {
                        self.close_window(p.surface);
                    } else {
                        events.push(AppEvent::Message(Message::PopupDone(p)));
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
                Message::QueueLimits(q) => self.queue_limits = Some(q),
                // The client scrolled a scroll view itself (v2): lay it out there.
                Message::Scrolled(sc) => {
                    let used = self.window_mut(sc.surface).is_some_and(|w| w.scrolled(sc.node, sc.x, sc.y));
                    if !used {
                        events.push(AppEvent::Message(Message::Scrolled(sc)));
                    }
                }
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
        self.update_blink();
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

    /// Run the rest of this handler against another window.
    pub fn set_window(&mut self, window: u32) {
        self.window = window;
    }

    /// The configured size of this handler's window.
    pub fn window_size(&self) -> (i32, i32) {
        self.core.window(self.window).map_or((0, 0), |w| (w.width, w.height))
    }

    fn state(&self, id: &str) -> Option<&WidgetState> {
        self.core.window(self.window).and_then(|w| w.state(id))
    }

    fn state_mut(&mut self, id: &str) -> Option<&mut WidgetState> {
        self.core.window_mut(self.window).map(|w| w.state_mut(id))
    }

    /// The whole state of a widget (text, caret, selection, …).
    pub fn widget(&self, id: &str) -> Option<WidgetState> {
        self.state(id).cloned()
    }

    /// The text of the text field with this id.
    pub fn text(&self, id: &str) -> String {
        self.state(id).map(|s| s.text.clone()).unwrap_or_default()
    }

    pub fn set_text(&mut self, id: &str, text: &str) {
        if let Some(s) = self.state_mut(id) {
            s.text = text.to_string();
            s.cursor = s.text.chars().count();
            s.anchor = None;
            s.follow = true;
        }
    }

    /// The text, emptying the field.
    pub fn take_text(&mut self, id: &str) -> String {
        let t = self.text(id);
        self.set_text(id, "");
        t
    }

    /// Select characters `start..end` of a text field or area (the caret at `end`).
    pub fn select_text(&mut self, id: &str, start: usize, end: usize) {
        if let Some(s) = self.state_mut(id) {
            s.select(start, end);
        }
    }

    /// The selected text of a text field or area.
    pub fn selected_text(&self, id: &str) -> String {
        self.state(id).map(|s| s.selected_text()).unwrap_or_default()
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

    /// The bitmap of the pixel grid with this id (palette indices, `cols * rows`).
    pub fn pixels(&self, id: &str) -> Vec<u8> {
        self.state(id).map(|s| s.pixels.clone()).unwrap_or_default()
    }

    /// Replace the bitmap of the pixel grid with this id.
    pub fn set_pixels(&mut self, id: &str, pixels: &[u8]) {
        if let Some(s) = self.state_mut(id) {
            s.pixels = pixels.to_vec();
        }
    }

    /// Move keyboard focus to the widget with this id.
    pub fn focus(&mut self, id: &str) {
        if let Some(w) = self.core.window_mut(self.window) {
            w.focus(Some(id));
        }
    }

    /// Re-render this window at the next loop turn.
    pub fn invalidate(&mut self) {
        if let Some(w) = self.core.window_mut(self.window) {
            w.dirty = true;
        }
    }

    pub fn set_title(&mut self, title: &str) {
        let w = self.window;
        self.core.set_title(w, title);
    }

    pub fn send(&mut self, m: Message) {
        self.core.send(m);
    }

    /// Put text on the clipboard.
    pub fn copy(&mut self, text: &str) {
        self.core.copy_text(text);
    }

    /// Paste the clipboard into the widget with this id (see [`Core::paste_into`]).
    pub fn paste_into(&mut self, id: &str) {
        let w = self.window;
        self.core.paste_into(w, id);
    }

    /// A button's menu below `anchor` (its rect in this window), or above it when it doesn't fit below;
    /// `f` gets the chosen index.
    pub fn popup_menu_below<S: Into<String>>(&mut self, anchor: crate::window::Rect, items: impl IntoIterator<Item = S>, f: impl Fn(&mut Ctx, usize) + 'static) {
        let w = self.window;
        let h = Handler::Callback(Rc::new(move |cx: &mut Ctx, e: &UiEvent| {
            if let crate::window::EventValue::Index(i) = e.value {
                f(cx, i);
            }
        }));
        self.core.popup_menu_below(w, anchor, items, h);
    }

    /// A popup menu at `(x, y)` lu in this window; `f` gets the chosen index.
    pub fn popup_menu<S: Into<String>>(&mut self, x: i32, y: i32, items: impl IntoIterator<Item = S>, f: impl Fn(&mut Ctx, usize) + 'static) {
        let w = self.window;
        let h = Handler::Callback(Rc::new(move |cx: &mut Ctx, e: &UiEvent| {
            if let crate::window::EventValue::Index(i) = e.value {
                f(cx, i);
            }
        }));
        self.core.popup_menu(w, x, y, items, h);
    }

    /// Ask the compositor to start an app (by program name or app id).
    pub fn launch(&mut self, app: &str, args: &[String]) {
        self.core.send(Message::Launch(proto::Launch { app_id: app.into(), args: args.to_vec() }));
    }

    /// Open a file with its default app (`Open{path}`): the app manager picks
    /// the handler (or the compositor shows the "open with" chooser), and the
    /// app gets a one-file grant and `OpenFile{path}`.
    pub fn open(&mut self, path: &str) {
        self.core.send(Message::Open(proto::Open { path: path.into() }));
    }

    /// Post a notification (the shell shows it; a click comes back as `NotificationAction`).
    pub fn notify(&mut self, tag: &str, title: &str, body: &str, actions: &[&str]) {
        self.core.send(Message::Notify(proto::Notify {
            tag: tag.chars().take(64).collect(),
            title: title.chars().take(256).collect(),
            body: body.chars().take(1024).collect(),
            icon: String::new(),
            actions: actions.iter().take(4).map(|a| a.chars().take(64).collect()).collect(),
        }));
    }

    /// Ask for a `Frame` event at the next refresh (only while visible).
    pub fn request_frame(&mut self) {
        let surface = self.window;
        self.core.send(Message::RequestFrame(proto::RequestFrame { surface }));
    }

    /// Canvas pixels shown at `pts_us` on the monotonic clock ([`Ctx::now_us`]); see [`Core::put_canvas_at`].
    pub fn put_canvas_at(&mut self, id: u32, pixels: &[u8], damage: &[[i32; 4]], pts_us: u64) -> Result<(), ConnError> {
        self.core.put_canvas_at(id, pixels, damage, pts_us)
    }

    /// Commit the canvas pixels put since the last commit for `pts_us` ([`Core::commit_at`]).
    pub fn commit_at(&mut self, pts_us: u64) -> Result<(), ConnError> {
        self.core.commit_at(pts_us)
    }

    /// Drop the queued timed frames ([`Core::queue_flush`]).
    pub fn queue_flush(&mut self) -> Result<(), ConnError> {
        self.core.queue_flush()
    }

    /// A canvas's content hint: 0 auto, 1 exact, 2 motion ([`Core::set_content`]).
    pub fn set_content(&mut self, resource: u32, hint: u8) {
        self.core.set_content(resource, hint);
    }

    /// The monotonic clock in µs that `CommitAt` times are on.
    pub fn now_us(&self) -> u64 {
        crate::conn::monotonic_us()
    }

    /// `(max_queue_frames, max_queue_ms)` from the compositor ([`Core::queue_limits`]).
    pub fn queue_limits(&self) -> Option<(u16, u32)> {
        self.core.queue_limits()
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
type KeyHook = Rc<dyn Fn(&mut Ctx, u16, u16) -> bool>;

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
    on_key: Option<KeyHook>,
    on_theme: Option<Hook>,
    tick: Option<(i32, Rc<dyn Fn(&mut Ctx) -> bool>)>,
    last_tick: Option<u64>,
    refresh: bool,
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
        App {
            core,
            named: HashMap::new(),
            hotkeys: HashMap::new(),
            on_suspend: None,
            on_frame: None,
            on_close: None,
            on_message: None,
            on_key: None,
            on_theme: None,
            tick: None,
            last_tick: None,
            refresh: true,
        }
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

    /// Key presses no widget used (`key` = HID usage, `mods` bits): shortcuts.
    /// Return true if the key was handled.
    pub fn on_key(&mut self, f: impl Fn(&mut Ctx, u16, u16) -> bool + 'static) {
        self.on_key = Some(Rc::new(f));
    }

    /// Runs after the theme changed (windows re-render by themselves).
    pub fn on_theme(&mut self, f: impl Fn(&mut Ctx) + 'static) {
        self.on_theme = Some(Rc::new(f));
    }

    /// Call `f` about every `ms` milliseconds (polling sockets, sessions,
    /// clocks); it returns true when it changed something to redraw.
    pub fn every(&mut self, ms: i32, f: impl Fn(&mut Ctx) -> bool + 'static) {
        self.tick = Some((ms.max(1), Rc::new(f)));
    }

    fn first_window(&self) -> u32 {
        self.core.windows.iter().find(|w| w.role == WindowRole::Window).map_or(0, |w| w.surface)
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
            AppEvent::Key { window, key, mods, down: true, .. } => {
                if let Some(f) = self.on_key.clone() {
                    f(&mut Ctx { core: &mut self.core, window }, key, mods);
                }
            }
            AppEvent::Key { .. } => {}
            AppEvent::Hotkey { slot, down: true } => {
                if let Some(f) = self.hotkeys.get(&slot).cloned() {
                    let w = self.first_window();
                    f(&mut Ctx { core: &mut self.core, window: w });
                }
            }
            AppEvent::Hotkey { .. } => {}
            AppEvent::ThemeChanged => {
                if let Some(f) = self.on_theme.clone() {
                    let w = self.first_window();
                    f(&mut Ctx { core: &mut self.core, window: w });
                }
            }
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
                    if !self.core.windows.iter().any(|w| w.role == WindowRole::Window) {
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

    /// Run the `every` callback if it is due.
    fn run_tick(&mut self) {
        let Some((ms, f)) = self.tick.clone() else { return };
        let now = self.core.now_ms();
        let due = match (now, self.last_tick) {
            (Some(n), Some(l)) => n.saturating_sub(l) >= ms as u64,
            _ => true,
        };
        if !due {
            return;
        }
        self.last_tick = now;
        let w = self.first_window();
        if f(&mut Ctx { core: &mut self.core, window: w }) {
            self.refresh = true;
        }
    }

    /// One loop iteration: render, wait up to `timeout_ms`, dispatch.
    /// False once the connection is closed or the app quit.
    pub fn step(&mut self, timeout_ms: i32) -> bool {
        // View functions read the app's own state: re-render after handlers ran.
        if self.refresh {
            self.core.mark_all_dirty();
            self.refresh = false;
        }
        if self.core.render().is_err() {
            return false;
        }
        let timeout = match &self.tick {
            Some((ms, _)) if timeout_ms < 0 || timeout_ms > *ms => *ms,
            _ => timeout_ms,
        };
        let Ok(events) = self.core.step(timeout) else { return false };
        // Re-render everything next turn, unless this turn was only a quiet
        // `every` tick (polling apps would otherwise redraw constantly).
        self.refresh = !events.is_empty() || self.tick.is_none();
        for ev in events {
            self.dispatch(ev);
        }
        self.run_tick();
        !self.core.is_closed()
    }

    /// Run until the connection closes or the app quits.
    pub fn run(&mut self) {
        while self.step(-1) {}
        let _ = self.core.flush();
    }
}

/// Where a popup menu goes.
#[derive(Clone, Copy, Debug)]
enum Place {
    At(i32, i32),
    Below(window::Rect),
}

/// A `w` x `h` popup's position inside a `pw` x `ph` parent (0: unknown, not limited):
/// below its anchor or else above it, then moved left/up to stay inside.
fn fit_popup(place: Place, w: i32, h: i32, pw: i32, ph: i32) -> (i32, i32) {
    let (x, mut y) = match place {
        Place::At(x, y) => (x, y),
        Place::Below(a) => (a.x, a.y + a.h),
    };
    if let Place::Below(a) = place {
        if ph > 0 && y + h > ph && a.y - h >= 0 {
            y = a.y - h;
        }
    }
    let x = if pw > 0 { x.min(pw - w) } else { x };
    let y = if ph > 0 { y.min(ph - h) } else { y };
    (x.max(0), y.max(0))
}

#[cfg(test)]
mod popup_fit_tests {
    use super::*;

    #[test]
    fn menus_stay_inside_their_window() {
        let button = window::Rect { x: 300, y: 560, w: 60, h: 30 };
        // A button at the bottom of a 360 x 600 phone window: the menu opens above it.
        assert_eq!(fit_popup(Place::Below(button), 140, 100, 360, 600), (220, 460));
        // Room below: below it.
        let top = window::Rect { x: 10, y: 10, w: 60, h: 30 };
        assert_eq!(fit_popup(Place::Below(top), 140, 100, 360, 600), (10, 40));
        // A point menu near the corner moves in; an unknown size leaves it.
        assert_eq!(fit_popup(Place::At(350, 590), 140, 100, 360, 600), (220, 500));
        assert_eq!(fit_popup(Place::At(350, 590), 140, 100, 0, 0), (350, 590));
    }
}
