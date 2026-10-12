//! Helpers for programs with the shell role (§8): connecting as the shell,
//! tracking the windows the compositor reports, a tiling layout, and a
//! one-line text bar on the `top` layer. The P2 test shell
//! (`rust/wasm-programs/test-shell`) is built from these; the real desktop
//! shell (P3) will use them too.

use crate::conn::{ConnError, Connection};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Message, NodeKind, Prop, TreeOp};
use std::collections::VecDeque;

/// One window as the shell knows it (from `WindowOpened` / `WindowChanged`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellWindow {
    pub id: u32,
    pub pid: u32,
    pub app_id: String,
    pub title: String,
    pub min: (i32, i32),
    pub max: (i32, i32),
    pub parent: u32,
}

/// What [`Shell::step`] reports.
#[derive(Clone, Debug)]
pub enum ShellEvent {
    Opened(u32),
    Changed(u32),
    Closed(u32),
    /// The app or the player asked for something (`activate` on a click into an unfocused window).
    Request { window: u32, action: proto::WindowAction, edges: u8 },
    /// A chord the shell bound was pressed.
    Chord(u32),
    Display(proto::DisplayInfo),
    /// A configure for one of the shell's own surfaces (already acked by [`Shell::ack`]).
    Configure(proto::Configure),
    /// Anything else (pointer events on the shell's surfaces, notifications, …).
    Message(Message),
}

/// A connection with the shell role.
pub struct Shell {
    conn: Box<dyn Connection>,
    pub welcome: proto::Welcome,
    pub display: proto::DisplayInfo,
    pub windows: Vec<ShellWindow>,
    pub focused: Option<u32>,
    out: Vec<u8>,
    pending: VecDeque<Message>,
}

impl Shell {
    /// Say Hello as the shell and wait for Welcome.
    pub fn connect(mut conn: Box<dyn Connection>) -> Result<Shell, ConnError> {
        let hello = Message::Hello(proto::Hello { version: proto::VERSION, role: proto::Role::Shell, capabilities: Vec::new() });
        conn.send(&hello.to_bytes())?;
        let mut pending = VecDeque::new();
        let welcome = loop {
            match pending.pop_front() {
                Some(Message::Welcome(w)) => break w,
                Some(_) => return Err(ConnError::Protocol),
                None => {
                    let bytes = conn.recv(crate::app::HANDSHAKE_TIMEOUT_MS)?;
                    if bytes.is_empty() {
                        return Err(ConnError::Protocol);
                    }
                    pending.extend(Message::decode_all(&bytes).map_err(|_| ConnError::Protocol)?);
                }
            }
        };
        let display = welcome.display.clone();
        Ok(Shell { conn, welcome, display, windows: Vec::new(), focused: None, out: Vec::new(), pending })
    }

    /// Queue a message (sent at the next [`Shell::flush`] or [`Shell::step`]).
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

    pub fn window(&self, id: u32) -> Option<&ShellWindow> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// Flush, wait up to `timeout_ms` for messages, and turn them into events.
    pub fn step(&mut self, timeout_ms: i32) -> Result<Vec<ShellEvent>, ConnError> {
        self.flush()?;
        if self.pending.is_empty() {
            let bytes = self.conn.recv(timeout_ms)?;
            self.pending.extend(Message::decode_all(&bytes).map_err(|_| ConnError::Protocol)?);
        }
        let mut ev = Vec::new();
        while let Some(m) = self.pending.pop_front() {
            match m {
                Message::WindowOpened(o) => {
                    self.windows.retain(|w| w.id != o.window);
                    self.windows.push(ShellWindow {
                        id: o.window,
                        pid: o.pid,
                        app_id: o.app_id,
                        title: o.title,
                        min: (o.min_w, o.min_h),
                        max: (o.max_w, o.max_h),
                        parent: o.parent,
                    });
                    ev.push(ShellEvent::Opened(o.window));
                }
                Message::WindowChanged(c) => {
                    if let Some(w) = self.windows.iter_mut().find(|w| w.id == c.window) {
                        w.title = c.title;
                        w.min = (c.min_w, c.min_h);
                        w.max = (c.max_w, c.max_h);
                    }
                    ev.push(ShellEvent::Changed(c.window));
                }
                Message::WindowClosed(c) => {
                    self.windows.retain(|w| w.id != c.window);
                    if self.focused == Some(c.window) {
                        self.focused = None;
                    }
                    ev.push(ShellEvent::Closed(c.window));
                }
                Message::WindowRequest(r) => ev.push(ShellEvent::Request { window: r.window, action: r.action, edges: r.edges }),
                Message::Chord(c) => ev.push(ShellEvent::Chord(c.id)),
                Message::DisplayChanged(d) => {
                    self.display = d.display.clone();
                    ev.push(ShellEvent::Display(d.display));
                }
                Message::Configure(c) => ev.push(ShellEvent::Configure(c)),
                Message::Error(e) => return Err(if e.code == proto::ErrorCode::Version { ConnError::Refused } else { ConnError::Protocol }),
                other => ev.push(ShellEvent::Message(other)),
            }
        }
        Ok(ev)
    }

    /// Place a window; `ms` > 0 animates from where it is (ease-out).
    pub fn place(&mut self, window: u32, rect: (i32, i32, i32, i32), states: u32, ms: u16) {
        self.send(Message::Place(proto::Place {
            window,
            x: rect.0,
            y: rect.1,
            w: rect.2.max(0),
            h: rect.3.max(0),
            states,
            transition: proto::Transition { ms, easing: proto::Easing::EaseOut },
        }));
    }

    /// Give a window the keyboard (and raise it).
    pub fn focus(&mut self, window: u32) {
        self.focused = Some(window);
        self.send(Message::FocusWindow(proto::FocusWindow { window }));
        let mut order: Vec<u32> = self.windows.iter().map(|w| w.id).filter(|&id| id != window).collect();
        order.push(window);
        self.send(Message::Stack(proto::Stack { windows: order }));
    }

    pub fn bind_chord(&mut self, id: u32, mods: u16, keys: &[u16]) {
        self.send(Message::BindChord(proto::BindChord { id, mods, keys: keys.to_vec() }));
    }

    pub fn launch(&mut self, app_id: &str, args: &[String]) {
        self.send(Message::Launch(proto::Launch { app_id: app_id.into(), args: args.to_vec() }));
    }

    pub fn close_window(&mut self, window: u32) {
        self.send(Message::CloseWindow(proto::CloseWindow { window }));
    }

    /// The window after the focused one (wrapping), for a switch-app chord.
    pub fn next_window(&self) -> Option<u32> {
        if self.windows.is_empty() {
            return None;
        }
        let i = self.focused.and_then(|f| self.windows.iter().position(|w| w.id == f)).map_or(0, |i| i + 1);
        Some(self.windows[i % self.windows.len()].id)
    }
}

/// Tile `n` windows into `area` (x, y, w, h) with `gap` lu around and between
/// them: side by side up to three, then a grid with `ceil(sqrt(n))` columns.
pub fn tile(area: (i32, i32, i32, i32), n: usize, gap: i32) -> Vec<(i32, i32, i32, i32)> {
    if n == 0 {
        return Vec::new();
    }
    let cols = if n <= 3 { n } else { (n as f64).sqrt().ceil() as usize };
    let rows = n.div_ceil(cols);
    let (ax, ay, aw, ah) = area;
    let cw = ((aw - gap * (cols as i32 + 1)) / cols as i32).max(1);
    let rh = ((ah - gap * (rows as i32 + 1)) / rows as i32).max(1);
    (0..n)
        .map(|i| {
            let c = (i % cols) as i32;
            let r = (i / cols) as i32;
            (ax + gap + c * (cw + gap), ay + gap + r * (rh + gap), cw, rh)
        })
        .collect()
}

/// A one-line text bar on a layer (the test shell's status bar): a filled
/// rectangle and one text node, redrawn with a single `Set` when the text changes.
pub struct Bar {
    pub surface: u32,
    pub height: i32,
    pub bg: u32,
    pub fg: u32,
    text: String,
    drawn: Option<String>,
    serial: Option<u32>,
    width: i32,
    created: bool,
}

impl Bar {
    pub fn new(surface: u32, height: i32) -> Bar {
        Bar { surface, height, bg: 0x202733ff, fg: 0xe6e9eeff, text: String::new(), drawn: None, serial: None, width: 0, created: false }
    }

    /// The `SurfaceCreate` for this bar on `layer`, full display width.
    pub fn create(&self, layer: proto::Layer) -> Message {
        Message::SurfaceCreate(proto::SurfaceCreate {
            surface: self.surface,
            kind: proto::SurfaceKind::Tree,
            role: proto::SurfaceRole::Layer,
            layer,
            parent: 0,
            x: 0,
            y: 0,
            w: 0,
            h: self.height,
            format: proto::PixelFormat::None,
            scale: 1,
            title: "bar".into(),
        })
    }

    pub fn set_text(&mut self, t: &str) {
        self.text = t.chars().take(200).collect();
    }

    /// Feed a configure; returns true if it was for this bar.
    pub fn configure(&mut self, c: &proto::Configure) -> bool {
        if c.surface != self.surface {
            return false;
        }
        self.serial = Some(c.serial);
        self.width = c.w;
        true
    }

    /// The messages that bring the bar up to date (empty if nothing changed).
    pub fn render(&mut self) -> Vec<Message> {
        let mut out = Vec::new();
        let ack = self.serial.take();
        let mut ops = Vec::new();
        if !self.created && (ack.is_some() || self.width > 0) {
            self.created = true;
            ops.push(TreeOp::Create {
                id: 1,
                parent: proto::ROOT_NODE,
                index: proto::APPEND,
                kind: NodeKind::Rect,
                props: vec![Prop::X(0), Prop::Y(0), Prop::W(self.width), Prop::H(self.height), Prop::Fill(self.bg)],
            });
            ops.push(TreeOp::Create {
                id: 2,
                parent: proto::ROOT_NODE,
                index: proto::APPEND,
                kind: NodeKind::Text,
                props: vec![
                    Prop::X(4),
                    Prop::Y((self.height - 12) / 2),
                    Prop::Text(self.text.clone()),
                    Prop::Font(proto::Font::Ui),
                    Prop::Size(12),
                    Prop::Color(self.fg),
                ],
            });
            self.drawn = Some(self.text.clone());
        } else if self.created {
            if ack.is_some() {
                ops.push(TreeOp::Set { id: 1, props: vec![Prop::W(self.width)] });
            }
            if self.drawn.as_deref() != Some(self.text.as_str()) {
                ops.push(TreeOp::Set { id: 2, props: vec![Prop::Text(self.text.clone())] });
                self.drawn = Some(self.text.clone());
            }
        }
        if !self.created {
            return out;
        }
        let commit = !ops.is_empty() || ack.is_some();
        if !ops.is_empty() {
            out.push(Message::TreeOps(proto::TreeOps { surface: self.surface, ops }));
        }
        if let Some(serial) = ack {
            out.push(Message::AckConfigure(proto::AckConfigure { surface: self.surface, serial }));
        }
        if commit {
            out.push(Message::Commit(proto::Commit { surface: self.surface }));
        }
        out
    }
}

/// HID usages the shells bind chords to.
pub mod keys {
    pub const TAB: u16 = 0x2B;
    pub const ENTER: u16 = 0x28;
    pub const N: u16 = 0x11;
    pub const F4: u16 = 0x3D;
}

/// The test shell's whole policy, as plain functions so a recorded golden
/// session uses exactly what the program does.
pub mod tiling {
    use super::*;

    /// Status bar height in lu.
    pub const BAR_H: i32 = 16;
    /// Gap between tiles in lu.
    pub const GAP: i32 = 4;
    /// Transition for windows that move when another opens or closes.
    pub const MOVE_MS: u16 = 120;

    /// Where every window goes, in `shell.windows` order.
    pub fn layout(display: &proto::DisplayInfo, n: usize) -> Vec<(i32, i32, i32, i32)> {
        tile((0, BAR_H, display.w, display.h - BAR_H), n, GAP)
    }

    /// The bar's text.
    pub fn bar_text(shell: &Shell) -> String {
        let focused = shell.focused.and_then(|f| shell.window(f)).map(|w| w.title.clone()).unwrap_or_else(|| "-".into());
        format!(
            "ECM test shell   {} window{}   focus: {}   Alt+Tab next  Alt+N notes  Alt+F4 close",
            shell.windows.len(),
            if shell.windows.len() == 1 { "" } else { "s" },
            focused
        )
    }

    /// Place every window by [`layout`]; windows already placed move with a transition.
    pub fn retile(shell: &mut Shell, placed: &mut Vec<u32>) {
        let rects = layout(&shell.display, shell.windows.len());
        let ids: Vec<u32> = shell.windows.iter().map(|w| w.id).collect();
        for (id, r) in ids.iter().zip(rects) {
            let states = if shell.focused == Some(*id) { proto::window_states::ACTIVATED } else { 0 };
            let ms = if placed.contains(id) { MOVE_MS } else { 0 };
            shell.place(*id, r, states, ms);
            if !placed.contains(id) {
                placed.push(*id);
            }
        }
        placed.retain(|p| ids.contains(p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_side_by_side_then_grid() {
        assert_eq!(tile((0, 16, 640, 384), 1, 4), vec![(4, 20, 632, 376)]);
        let two = tile((0, 16, 640, 384), 2, 4);
        assert_eq!(two, vec![(4, 20, 314, 376), (322, 20, 314, 376)]);
        let four = tile((0, 0, 640, 400), 4, 4);
        assert_eq!(four.len(), 4);
        assert_eq!(four[2].0, 4);
        assert!(four[2].1 > four[0].1, "second row below the first");
        for (x, y, w, h) in tile((0, 0, 640, 400), 7, 4) {
            assert!(x >= 0 && y >= 0 && x + w <= 640 && y + h <= 400);
        }
    }

    #[test]
    fn bar_creates_once_then_sets_text() {
        let mut b = Bar::new(1, 16);
        b.set_text("hello");
        assert!(b.render().is_empty(), "nothing before the first configure");
        b.configure(&proto::Configure { surface: 1, serial: 1, w: 640, h: 16, states: 0 });
        let first = b.render();
        assert_eq!(first.len(), 3, "TreeOps, AckConfigure, Commit");
        assert!(b.render().is_empty(), "nothing changed");
        b.set_text("world");
        let next = b.render();
        match &next[0] {
            Message::TreeOps(t) => assert_eq!(t.ops, vec![TreeOp::Set { id: 2, props: vec![Prop::Text("world".into())] }]),
            m => panic!("unexpected {m:?}"),
        }
        // Everything it sends is valid on the wire.
        for m in first.iter().chain(next.iter()) {
            Message::decode_exact(&m.to_bytes()).unwrap();
        }
    }
}
