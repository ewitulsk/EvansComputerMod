//! `dev.ecm.phone` — the ePhone shell (ROADMAP P5, docs/gui/EPHONE.md "Phone
//! shell"). It owns only layer surfaces, never windows:
//!
//! * **home** (`background` layer): wallpaper and the grid of installed apps;
//! * **status** (`top` layer, 16 lu at the top): time, unread dot,
//!   background-mode pills, cellular and Wi-Fi bars, battery;
//! * **nav** (`top` layer, 20 lu at the bottom): back, home, recent apps;
//! * **overlay** (`overlay` layer, created while shown): the notification
//!   sheet, the app switcher, the permission dialog and the Controls
//!   (hotkeys) screen.
//!
//! One app is on screen at a time, placed between the bars; the others are
//! hidden (and the OS suspends them). Keys (chords, since the shell has no
//! window): **Alt+H** home, **Alt+Backspace** back, **Alt+Tab** next app,
//! **Alt+N** notifications. The ePhone's switch-app key arrives as
//! `Chord{0xEC01}` and does what Alt+Tab does, without opening the phone.
//!
//! The policy is plain code driven by messages and an [`Env`] (status, apps,
//! clock), so a recorded session replays it exactly for the golden images.

use ecm_ui::diff::{SceneNode, TreeState};
use ecm_ui::shell::{Shell, ShellEvent};
use ecm_ui::{ConnError, Connection, Theme};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Font, Message, NodeKind, Prop};
use std::collections::HashMap;

/// Status bar height (lu).
pub const STATUS_H: i32 = 16;
/// Navigation bar height (lu).
pub const NAV_H: i32 = 20;
/// The phone's OS switch-app key (Compositor.CHORD_SWITCH_APP).
pub const CHORD_SWITCH_APP: u32 = 0xEC01;
pub const CHORD_HOME: u32 = 1;
pub const CHORD_BACK: u32 = 2;
pub const CHORD_NEXT: u32 = 3;
pub const CHORD_SHEET: u32 = 4;

/// HID usages of the chord keys.
pub mod hid {
    pub const H: u16 = 0x0B;
    pub const N: u16 = 0x11;
    pub const BACKSPACE: u16 = 0x2A;
    pub const TAB: u16 = 0x2B;
}

const SURF_HOME: u32 = 1;
const SURF_STATUS: u32 = 2;
const SURF_NAV: u32 = 3;
const SURF_OVERLAY: u32 = 4;
/// Icon resources start here (one per app).
const RES_ICON: u32 = 100;

/// The shell's colours (also sent to apps as the `ecm-phone` theme).
pub mod colors {
    pub const BAR: u32 = 0x0b0f14e6;
    pub const BAR_TEXT: u32 = 0xe8edf2ff;
    pub const ACCENT: u32 = 0x2fb98aff;
    pub const SHEET: u32 = 0x141a22f2;
    pub const CARD: u32 = 0x232b36ff;
    pub const CARD_TEXT: u32 = 0xe8edf2ff;
    pub const MUTED: u32 = 0x93a0b0ff;
    pub const SCRIM: u32 = 0x00000099;
    pub const DANGER: u32 = 0xe5534bff;
    pub const WALL_TOP: u32 = 0x0f3b4a;
    pub const WALL_BOTTOM: u32 = 0x1a1f3a;
}

/// The phone's theme: the default toolkit theme with the shell's accent.
pub fn theme() -> Theme {
    let mut t = Theme { name: "ecm-phone".into(), ..Theme::default() };
    t.accent = colors::ACCENT;
    t.focus = 0x6fe0b8ff;
    t
}

/// What the status bar shows (`/dev/uictl phone`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub battery: f32,
    pub state: String,
    /// 0-4, -1 = no Wi-Fi
    pub wifi: i32,
    /// 0-4, -1 = no modem
    pub cell: i32,
    /// The carrier celld is on ("" = none).
    pub carrier: String,
    pub foreground: String,
    /// (app id, mode)
    pub modes: Vec<(String, String)>,
    pub hotkeys: Vec<HotkeyClaim>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HotkeyClaim {
    pub app: String,
    pub slot: u8,
    pub focused: bool,
    pub label: String,
}

impl Status {
    /// Parse the `/dev/uictl phone` answer.
    pub fn parse(text: &str) -> Status {
        let mut s = Status { battery: 100.0, wifi: -1, cell: -1, ..Status::default() };
        for line in text.lines() {
            let mut it = line.splitn(2, ' ');
            let key = it.next().unwrap_or("");
            let rest = it.next().unwrap_or("").trim();
            match key {
                "state" => s.state = rest.to_string(),
                "battery" => s.battery = rest.parse().unwrap_or(100.0),
                "wifi" => s.wifi = rest.parse().unwrap_or(-1),
                "cell" => s.cell = rest.parse().unwrap_or(-1),
                "carrier" => s.carrier = if rest == "-" { String::new() } else { rest.to_string() },
                "foreground" => s.foreground = if rest == "-" { String::new() } else { rest.to_string() },
                "mode" => {
                    let mut p = rest.splitn(2, ' ');
                    if let (Some(a), Some(m)) = (p.next(), p.next()) {
                        s.modes.push((a.to_string(), m.to_string()));
                    }
                }
                "hotkey" => {
                    let p: Vec<&str> = rest.splitn(4, ' ').collect();
                    if p.len() == 4 {
                        if let Ok(slot) = p[1].parse() {
                            s.hotkeys.push(HotkeyClaim { app: p[0].into(), slot, focused: p[2] == "*", label: p[3].into() });
                        }
                    }
                }
                _ => {}
            }
        }
        s
    }
}

/// One app on the home screen.
#[derive(Clone, Debug, PartialEq)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    /// The app's icon.png, if it has one.
    pub icon: Option<Vec<u8>>,
}

/// Where the shell gets what it shows. The program reads the computer; the
/// golden-image recorder supplies fixed values.
pub trait Env {
    /// The clock for the status bar, "HH:MM".
    fn clock(&mut self) -> String;
    fn status(&mut self) -> Status;
    fn apps(&mut self) -> Vec<AppEntry>;
    /// Apps that were open when the phone last shut down (reopened at a cold boot).
    fn session(&mut self) -> Vec<String>;
}

/// What a tap on a hit id does.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Launch(String),
    Controls,
    PullSheet,
    Back,
    Home,
    Recents,
    Show(u32),
    Close(u32),
    Notification(u32),
    ClearNotifications,
    Answer { allow: bool, remember: bool },
    Dismiss,
    Nothing,
}

/// What is on the overlay layer.
#[derive(Clone, Debug, PartialEq)]
pub enum Overlay {
    Sheet,
    Recents,
    Controls,
    Permission { req: u32, app: String, permission: String },
}

#[derive(Clone, Debug)]
pub struct Notification {
    pub nid: u32,
    pub app_id: String,
    pub title: String,
    pub body: String,
}

/// One of the shell's layer surfaces.
struct Layer {
    id: u32,
    title: &'static str,
    layer: proto::Layer,
    tree: TreeState,
    serial: Option<u32>,
    size: (i32, i32),
    configured: bool,
}

impl Layer {
    fn new(id: u32, title: &'static str, layer: proto::Layer) -> Layer {
        Layer { id, title, layer, tree: TreeState::new(), serial: None, size: (0, 0), configured: false }
    }

    fn create(&self, rect: (i32, i32, i32, i32)) -> Message {
        Message::SurfaceCreate(proto::SurfaceCreate {
            surface: self.id,
            kind: proto::SurfaceKind::Tree,
            role: proto::SurfaceRole::Layer,
            layer: self.layer,
            parent: 0,
            x: rect.0,
            y: rect.1,
            w: rect.2,
            h: rect.3,
            format: proto::PixelFormat::None,
            scale: 1,
            title: self.title.into(),
        })
    }

    fn render(&mut self, scene: &[SceneNode]) -> Vec<Message> {
        if !self.configured {
            return Vec::new();
        }
        let ops = self.tree.update(scene);
        let ack = self.serial.take();
        let mut out = Vec::new();
        if !ops.is_empty() {
            out.push(Message::TreeOps(proto::TreeOps { surface: self.id, ops }));
        }
        if let Some(serial) = ack {
            out.push(Message::AckConfigure(proto::AckConfigure { surface: self.id, serial }));
        }
        if !out.is_empty() {
            out.push(Message::Commit(proto::Commit { surface: self.id }));
        }
        out
    }
}

/// Builds one surface's scene and the hit ids in it.
struct Painter<'a> {
    nodes: Vec<SceneNode>,
    hits: &'a mut Hits,
}

/// Hit ids, stable per action (so a re-render changes nothing it needn't).
#[derive(Default)]
struct Hits {
    ids: HashMap<Action, u32>,
    actions: HashMap<u32, Action>,
}

impl Hits {
    fn id(&mut self, a: Action) -> u32 {
        if let Some(&id) = self.ids.get(&a) {
            return id;
        }
        let id = self.ids.len() as u32 + 1;
        self.ids.insert(a.clone(), id);
        self.actions.insert(id, a);
        id
    }
}

fn face(size: u8) -> proto::fonts::Face {
    proto::fonts::face(Font::Ui, size).expect("ui font size")
}

/// Width of `s` in the ui font at `size` (lu).
pub fn text_w(s: &str, size: u8) -> i32 {
    face(size).text_width(s)
}

/// `s` cut with "..." to fit `max` lu.
pub fn ellipsize(s: &str, size: u8, max: i32) -> String {
    if text_w(s, size) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        let next = format!("{out}{c}...");
        if text_w(&next, size) > max {
            break;
        }
        out.push(c);
    }
    format!("{out}...")
}

impl<'a> Painter<'a> {
    fn new(hits: &'a mut Hits) -> Painter<'a> {
        Painter { nodes: Vec::new(), hits }
    }

    fn rect(&mut self, key: &str, r: (i32, i32, i32, i32), fill: u32, radius: u16, hit: Option<Action>) {
        let mut p = vec![Prop::X(r.0), Prop::Y(r.1), Prop::W(r.2), Prop::H(r.3), Prop::Fill(fill)];
        if radius > 0 {
            p.push(Prop::Radius(radius));
        }
        if let Some(a) = hit {
            p.push(Prop::HitId(self.hits.id(a)));
        }
        self.nodes.push(SceneNode::new(key, NodeKind::Rect, p));
    }

    fn frame(&mut self, key: &str, r: (i32, i32, i32, i32), color: u32, width: u16, radius: u16) {
        self.nodes.push(SceneNode::new(
            key,
            NodeKind::Rect,
            vec![Prop::X(r.0), Prop::Y(r.1), Prop::W(r.2), Prop::H(r.3), Prop::BorderW(width), Prop::BorderColor(color), Prop::Radius(radius)],
        ));
    }

    fn text(&mut self, key: &str, x: i32, y: i32, s: &str, size: u8, color: u32, hit: Option<Action>) {
        let mut p = vec![Prop::X(x), Prop::Y(y), Prop::Text(s.to_string()), Prop::Font(Font::Ui), Prop::Size(size), Prop::Color(color)];
        if let Some(a) = hit {
            p.push(Prop::HitId(self.hits.id(a)));
        }
        self.nodes.push(SceneNode::new(key, NodeKind::Text, p));
    }

    /// Text centred on `cx`.
    fn text_c(&mut self, key: &str, cx: i32, y: i32, s: &str, size: u8, color: u32, hit: Option<Action>) {
        let w = text_w(s, size);
        self.text(key, cx - w / 2, y, s, size, color, hit);
    }

    fn image(&mut self, key: &str, r: (i32, i32, i32, i32), resource: u32, hit: Option<Action>) {
        let mut p = vec![Prop::X(r.0), Prop::Y(r.1), Prop::W(r.2), Prop::H(r.3), Prop::Resource(resource)];
        if let Some(a) = hit {
            p.push(Prop::HitId(self.hits.id(a)));
        }
        self.nodes.push(SceneNode::new(key, NodeKind::Image, p));
    }

    fn line(&mut self, key: &str, pts: &[(i32, i32)], width: u16, color: u32) {
        let points = pts.iter().map(|&(x, y)| proto::Point { x, y }).collect();
        self.nodes.push(SceneNode::new(key, NodeKind::Line, vec![Prop::Points(points), Prop::Width(width), Prop::Color(color)]));
    }
}

/// A short phrase for a permission in the dialog ("Allow Notes to use ...?").
pub fn permission_words(p: &str) -> String {
    match p {
        "network" => "the network".into(),
        "radio" => "radios".into(),
        "cellular" => "the cellular modem".into(),
        "controller" => "game controllers".into(),
        "location" => "your location".into(),
        "notifications" => "notifications".into(),
        "files:documents" => "your documents".into(),
        "files:all" => "all your files".into(),
        "packages" => "installing apps".into(),
        _ => match p.strip_prefix("peripherals:") {
            Some(t) => format!("the {t}"),
            None => p.to_string(),
        },
    }
}

/// The phone shell.
pub struct PhoneShell<E: Env> {
    pub sh: Shell,
    pub env: E,
    home: Layer,
    status_l: Layer,
    nav: Layer,
    overlay_l: Option<Layer>,
    hits: Hits,
    pub apps: Vec<AppEntry>,
    icons: HashMap<String, (u32, i32, i32)>,
    next_res: u32,
    /// Uploads to do after the next flush: (resource, png).
    uploads: Vec<(u32, Vec<u8>)>,
    pub status: Status,
    pub clock: String,
    /// The app window on screen, if any.
    pub current: Option<u32>,
    /// Open app windows, most recent first.
    pub recents: Vec<u32>,
    /// Windows the player hasn't been shown yet (reopened at boot): they stay hidden.
    background: Vec<String>,
    /// An app the player asked for: its window is shown when it opens.
    pending_show: Option<String>,
    pub overlay: Option<Overlay>,
    permissions: Vec<Overlay>,
    pub notifications: Vec<Notification>,
    pub unread: usize,
    down: HashMap<u32, u32>,
    pull_from: Option<i32>,
}

impl<E: Env> PhoneShell<E> {
    /// Connect as the shell, create the layers, bind the chords, set the theme.
    /// `restarted` (the compositor's `--restart`) skips reopening the last session.
    pub fn new(conn: Box<dyn Connection>, mut env: E, restarted: bool) -> Result<PhoneShell<E>, ConnError> {
        let sh = Shell::connect(conn)?;
        let clock = env.clock();
        let status = env.status();
        let mut s = PhoneShell {
            sh,
            env,
            home: Layer::new(SURF_HOME, "home", proto::Layer::Background),
            status_l: Layer::new(SURF_STATUS, "status", proto::Layer::Top),
            nav: Layer::new(SURF_NAV, "nav", proto::Layer::Top),
            overlay_l: None,
            hits: Hits::default(),
            apps: Vec::new(),
            icons: HashMap::new(),
            next_res: RES_ICON,
            uploads: Vec::new(),
            status,
            clock,
            current: None,
            recents: Vec::new(),
            background: Vec::new(),
            pending_show: None,
            overlay: None,
            permissions: Vec::new(),
            notifications: Vec::new(),
            unread: 0,
            down: HashMap::new(),
            pull_from: None,
        };
        let (w, h) = (s.sh.display.w, s.sh.display.h);
        let m = s.home.create((0, 0, 0, 0));
        s.sh.send(m);
        let m = s.status_l.create((0, 0, 0, STATUS_H));
        s.sh.send(m);
        let m = s.nav.create((0, h - NAV_H, w, NAV_H));
        s.sh.send(m);
        s.sh.send(Message::SetTheme(proto::SetTheme { theme: theme().to_wire() }));
        s.sh.bind_chord(CHORD_HOME, proto::mods::ALT, &[hid::H]);
        s.sh.bind_chord(CHORD_BACK, proto::mods::ALT, &[hid::BACKSPACE]);
        s.sh.bind_chord(CHORD_NEXT, proto::mods::ALT, &[hid::TAB]);
        s.sh.bind_chord(CHORD_SHEET, proto::mods::ALT, &[hid::N]);
        s.refresh_apps();
        if !restarted {
            for id in s.env.session() {
                if s.apps.iter().any(|a| a.id == id) {
                    s.background.push(id.clone());
                    s.sh.launch(&id, &[]);
                }
            }
        }
        Ok(s)
    }

    /// Re-read the installed apps; new icons are uploaded at the next render.
    pub fn refresh_apps(&mut self) {
        let mut apps = self.env.apps();
        apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        for a in &apps {
            if self.icons.contains_key(&a.id) {
                continue;
            }
            if let Some(png) = &a.icon {
                if png.len() >= 24 && &png[1..4] == b"PNG" {
                    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]) as i32;
                    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]) as i32;
                    if w > 0 && h > 0 && w <= 256 && h <= 256 {
                        let res = self.next_res;
                        self.next_res += 1;
                        self.sh.send(Message::ResourceCreate(proto::ResourceCreate {
                            id: res,
                            kind: proto::ResourceKind::Image,
                            w,
                            h,
                            format: proto::PixelFormat::Rgba8888,
                        }));
                        self.uploads.push((res, png.clone()));
                        self.icons.insert(a.id.clone(), (res, w, h));
                    }
                }
            }
        }
        self.apps = apps;
    }

    /// Re-read the clock and the status (once a second).
    pub fn refresh_status(&mut self) {
        self.clock = self.env.clock();
        self.status = self.env.status();
    }

    fn app_area(&self) -> (i32, i32, i32, i32) {
        let d = &self.sh.display;
        (0, STATUS_H, d.w, d.h - STATUS_H - NAV_H)
    }

    // ------------------------------------------------------------ app windows

    /// Put `win` on screen (every other app window hidden), with the keyboard.
    pub fn show(&mut self, win: u32) {
        if self.sh.window(win).is_none() {
            return;
        }
        let ids: Vec<u32> = self.sh.windows.iter().map(|w| w.id).collect();
        for id in ids {
            if id != win {
                self.sh.send(Message::SetVisible(proto::SetVisible { window: id, visible: false }));
            }
        }
        let area = self.app_area();
        self.sh.place(win, area, proto::window_states::MAXIMIZED | proto::window_states::ACTIVATED, 0);
        self.sh.send(Message::SetVisible(proto::SetVisible { window: win, visible: true }));
        self.sh.focus(win);
        self.current = Some(win);
        self.recents.retain(|&w| w != win);
        self.recents.insert(0, win);
        if let Some(app) = self.sh.window(win).map(|w| w.app_id.clone()) {
            self.background.retain(|b| *b != app);
        }
        self.close_overlay();
    }

    /// Back to the home screen: every app hidden (and suspended by the OS).
    pub fn home(&mut self) {
        let ids: Vec<u32> = self.sh.windows.iter().map(|w| w.id).collect();
        for id in ids {
            self.sh.send(Message::SetVisible(proto::SetVisible { window: id, visible: false }));
        }
        self.current = None;
        self.sh.focused = None;
        self.close_overlay();
    }

    /// Open an app: its window if it is open, else launch it.
    pub fn open_app(&mut self, id: &str) {
        let existing = self.sh.windows.iter().find(|w| w.app_id == id && w.parent == 0).map(|w| w.id);
        match existing {
            Some(w) => self.show(w),
            None => {
                self.pending_show = Some(id.to_string());
                self.background.retain(|b| b != id);
                self.sh.launch(id, &[]);
                self.close_overlay();
            }
        }
    }

    /// The next open app after the one on screen (the switch-app key, Alt+Tab).
    pub fn next_app(&mut self) {
        if self.recents.is_empty() {
            return;
        }
        let next = match self.current {
            Some(c) => {
                let mut order: Vec<u32> = self.recents.clone();
                order.sort();
                let i = order.iter().position(|&w| w == c).map_or(0, |i| i + 1);
                order[i % order.len()]
            }
            None => self.recents[0],
        };
        self.show(next);
    }

    pub fn back(&mut self) {
        if self.overlay.is_some() {
            if let Some(Overlay::Permission { req, .. }) = self.overlay.clone() {
                self.answer(req, false, false);
            } else {
                self.close_overlay();
            }
        } else if self.current.is_some() {
            self.home();
        }
    }

    fn window_opened(&mut self, id: u32) {
        let Some(w) = self.sh.window(id).cloned() else { return };
        if w.parent != 0 {
            // A dialog of an app: on screen with its app.
            let area = self.app_area();
            self.sh.place(id, area, proto::window_states::MAXIMIZED | proto::window_states::ACTIVATED, 0);
            self.sh.focus(id);
            return;
        }
        let wanted = self.pending_show.as_deref() == Some(w.app_id.as_str());
        if self.background.contains(&w.app_id) && !wanted {
            // Reopened from the last session: in the switcher, not on screen.
            self.sh.send(Message::SetVisible(proto::SetVisible { window: id, visible: false }));
            self.recents.push(id);
            return;
        }
        self.pending_show = None;
        self.show(id);
    }

    fn window_closed(&mut self, id: u32) {
        self.recents.retain(|&w| w != id);
        if self.current == Some(id) {
            self.current = None;
            if let Some(&next) = self.recents.first() {
                self.show(next);
            }
        }
    }

    // ------------------------------------------------------------ overlay

    fn open_overlay(&mut self, o: Overlay) {
        if self.overlay_l.is_none() {
            let l = Layer::new(SURF_OVERLAY, "overlay", proto::Layer::Overlay);
            let m = l.create((0, 0, 0, 0));
            self.sh.send(m);
            self.overlay_l = Some(l);
        }
        if o == Overlay::Sheet {
            self.unread = 0;
        }
        self.overlay = Some(o);
    }

    fn close_overlay(&mut self) {
        if matches!(self.overlay, Some(Overlay::Permission { .. })) {
            return; // only an answer closes it
        }
        self.overlay = None;
        if let Some(next) = self.permissions.first().cloned() {
            self.permissions.remove(0);
            self.open_overlay(next);
            return;
        }
        if self.overlay_l.take().is_some() {
            self.sh.send(Message::SurfaceDestroy(proto::SurfaceDestroy { surface: SURF_OVERLAY }));
        }
    }

    fn answer(&mut self, req: u32, allow: bool, remember: bool) {
        self.sh.send(Message::AnswerPermission(proto::AnswerPermission { req, allow, remember }));
        self.overlay = None;
        self.close_overlay();
    }

    // ------------------------------------------------------------ events

    /// Handle what [`Shell::step`] returned.
    pub fn handle(&mut self, events: Vec<ShellEvent>) {
        for ev in events {
            match ev {
                ShellEvent::Opened(id) => self.window_opened(id),
                ShellEvent::Closed(id) => self.window_closed(id),
                ShellEvent::Changed(_) => {}
                ShellEvent::Request { window, action: proto::WindowAction::Activate, .. } => self.show(window),
                ShellEvent::Request { .. } => {}
                ShellEvent::Chord(CHORD_HOME) => self.home(),
                ShellEvent::Chord(CHORD_BACK) => self.back(),
                ShellEvent::Chord(CHORD_NEXT) | ShellEvent::Chord(CHORD_SWITCH_APP) => self.next_app(),
                ShellEvent::Chord(CHORD_SHEET) => {
                    if self.overlay == Some(Overlay::Sheet) {
                        self.close_overlay();
                    } else if !matches!(self.overlay, Some(Overlay::Permission { .. })) {
                        self.open_overlay(Overlay::Sheet);
                    }
                }
                ShellEvent::Chord(_) => {}
                ShellEvent::Display(_) => self.display_changed(),
                ShellEvent::Configure(c) => self.configure(c),
                ShellEvent::Message(m) => self.message(m),
            }
        }
    }

    /// Another panel (hot-plug): the layers are made again at the new size and the open app re-placed.
    fn display_changed(&mut self) {
        let (w, h) = (self.sh.display.w, self.sh.display.h);
        for id in [SURF_HOME, SURF_STATUS, SURF_NAV] {
            self.sh.send(Message::SurfaceDestroy(proto::SurfaceDestroy { surface: id }));
        }
        self.home = Layer::new(SURF_HOME, "home", proto::Layer::Background);
        self.status_l = Layer::new(SURF_STATUS, "status", proto::Layer::Top);
        self.nav = Layer::new(SURF_NAV, "nav", proto::Layer::Top);
        let m = self.home.create((0, 0, 0, 0));
        self.sh.send(m);
        let m = self.status_l.create((0, 0, 0, STATUS_H));
        self.sh.send(m);
        let m = self.nav.create((0, h - NAV_H, w, NAV_H));
        self.sh.send(m);
        if self.overlay_l.is_some() {
            self.sh.send(Message::SurfaceDestroy(proto::SurfaceDestroy { surface: SURF_OVERLAY }));
            let l = Layer::new(SURF_OVERLAY, "overlay", proto::Layer::Overlay);
            let m = l.create((0, 0, 0, 0));
            self.sh.send(m);
            self.overlay_l = Some(l);
        }
        if let Some(c) = self.current {
            let area = self.app_area();
            self.sh.place(c, area, proto::window_states::MAXIMIZED | proto::window_states::ACTIVATED, 0);
        }
    }

    fn configure(&mut self, c: proto::Configure) {
        let l = match c.surface {
            SURF_HOME => &mut self.home,
            SURF_STATUS => &mut self.status_l,
            SURF_NAV => &mut self.nav,
            SURF_OVERLAY => match self.overlay_l.as_mut() {
                Some(l) => l,
                None => return,
            },
            _ => return,
        };
        l.serial = Some(c.serial);
        l.size = (c.w, c.h);
        l.configured = true;
    }

    fn message(&mut self, m: Message) {
        match m {
            Message::Pointer(p) => self.pointer(p),
            Message::NotificationPosted(n) => {
                self.notifications.retain(|x| !(x.app_id == n.app_id && !n.tag.is_empty() && x.title == n.title && x.body == n.body));
                self.notifications.insert(0, Notification { nid: n.nid, app_id: n.app_id, title: n.title, body: n.body });
                self.notifications.truncate(20);
                if self.overlay != Some(Overlay::Sheet) {
                    self.unread += 1;
                }
            }
            Message::PermissionPrompt(p) => {
                let o = Overlay::Permission { req: p.req, app: p.app_id, permission: p.permission };
                if matches!(self.overlay, Some(Overlay::Permission { .. })) {
                    self.permissions.push(o);
                } else {
                    self.open_overlay(o);
                }
            }
            _ => {}
        }
    }

    fn pointer(&mut self, p: proto::Pointer) {
        match p.kind {
            proto::PointerKind::Down => {
                self.down.insert(p.surface, p.hit_id);
                if p.surface == SURF_STATUS {
                    self.pull_from = Some(p.y);
                }
            }
            proto::PointerKind::Move => {
                if let Some(y0) = self.pull_from {
                    // Pull the status bar down: the notification sheet.
                    if p.surface == SURF_STATUS && p.y - y0 >= STATUS_H && self.overlay.is_none() {
                        self.pull_from = None;
                        self.open_overlay(Overlay::Sheet);
                    }
                }
            }
            proto::PointerKind::Up => {
                self.pull_from = None;
                let down = self.down.remove(&p.surface).unwrap_or(0);
                if down != 0 && down == p.hit_id {
                    if let Some(a) = self.hits.actions.get(&down).cloned() {
                        self.act(a);
                    }
                }
            }
            _ => {}
        }
    }

    /// Do what a tap asked for.
    pub fn act(&mut self, a: Action) {
        match a {
            Action::Launch(id) => self.open_app(&id),
            Action::Controls => self.open_overlay(Overlay::Controls),
            Action::PullSheet => {
                if self.overlay.is_none() {
                    self.open_overlay(Overlay::Sheet);
                } else if self.overlay == Some(Overlay::Sheet) {
                    self.close_overlay();
                }
            }
            Action::Back => self.back(),
            Action::Home => self.home(),
            Action::Recents => {
                if self.overlay == Some(Overlay::Recents) {
                    self.close_overlay();
                } else if self.overlay.is_none() || self.overlay == Some(Overlay::Sheet) {
                    self.overlay = None;
                    self.open_overlay(Overlay::Recents);
                }
            }
            Action::Show(w) => self.show(w),
            Action::Close(w) => {
                self.sh.close_window(w);
            }
            Action::Notification(nid) => {
                self.sh.send(Message::NotificationInvoke(proto::NotificationInvoke { nid, action: String::new() }));
                if let Some(n) = self.notifications.iter().position(|n| n.nid == nid) {
                    let app = self.notifications.remove(n).app_id;
                    if !app.is_empty() {
                        self.open_app(&app);
                    }
                }
                self.close_overlay();
            }
            Action::ClearNotifications => {
                self.notifications.clear();
                self.close_overlay();
            }
            Action::Answer { allow, remember } => {
                if let Some(Overlay::Permission { req, .. }) = self.overlay.clone() {
                    self.answer(req, allow, remember);
                }
            }
            Action::Dismiss => self.close_overlay(),
            Action::Nothing => {}
        }
    }

    /// The hit id a tap on `a` carries, once drawn (golden-image recordings check taps with it).
    pub fn hit_of(&self, a: &Action) -> Option<u32> {
        self.hits.ids.get(a).copied()
    }

    // ------------------------------------------------------------ drawing

    /// Bring every surface up to date and flush (then upload new icons).
    pub fn render(&mut self) -> Result<(), ConnError> {
        let home = self.paint_home();
        for m in self.home.render(&home) {
            self.sh.send(m);
        }
        let status = self.paint_status();
        for m in self.status_l.render(&status) {
            self.sh.send(m);
        }
        let nav = self.paint_nav();
        for m in self.nav.render(&nav) {
            self.sh.send(m);
        }
        if self.overlay_l.is_some() {
            let o = self.paint_overlay();
            if let Some(l) = self.overlay_l.as_mut() {
                for m in l.render(&o) {
                    self.sh.send(m);
                }
            }
        }
        self.sh.flush()?;
        for (res, png) in std::mem::take(&mut self.uploads) {
            self.sh.connection().upload(res, proto::host::UPLOAD_PNG, &png)?;
        }
        Ok(())
    }

    fn paint_home(&mut self) -> Vec<SceneNode> {
        let (w, h) = self.home.size;
        let apps = self.apps.clone();
        let icons = self.icons.clone();
        let mut p = Painter::new(&mut self.hits);
        // Wallpaper: eight bands from teal to night blue.
        let bands = 8;
        for i in 0..bands {
            let t = i as u32;
            let mix = |a: u32, b: u32, sh: u32| -> u32 {
                let ca = (a >> sh) & 0xff;
                let cb = (b >> sh) & 0xff;
                (ca * (bands - 1 - t) + cb * t) / (bands - 1)
            };
            let c = (mix(colors::WALL_TOP, colors::WALL_BOTTOM, 16) << 24)
                | (mix(colors::WALL_TOP, colors::WALL_BOTTOM, 8) << 16)
                | (mix(colors::WALL_TOP, colors::WALL_BOTTOM, 0) << 8)
                | 0xff;
            let y0 = h * i as i32 / bands as i32;
            let y1 = h * (i as i32 + 1) / bands as i32;
            p.rect(&format!("wall{i}"), (0, y0, w, y1 - y0), c, 0, None);
        }
        // The app grid.
        let cols = if w >= 200 { 4 } else { 3 };
        let icon = if w >= 200 { 32 } else { 16 };
        let cell_w = w / cols;
        let cell_h = icon + 22;
        let mut entries: Vec<(String, String, Action)> =
            apps.iter().map(|a| (a.id.clone(), a.name.clone(), Action::Launch(a.id.clone()))).collect();
        entries.push(("controls".into(), "Controls".into(), Action::Controls));
        for (i, (id, name, action)) in entries.into_iter().enumerate() {
            let col = i as i32 % cols;
            let row = i as i32 / cols;
            let x = col * cell_w + (cell_w - icon) / 2;
            let y = STATUS_H + 10 + row * cell_h;
            if y + cell_h > h - NAV_H {
                break;
            }
            let key = format!("app:{id}");
            match icons.get(&id) {
                Some(&(res, _, _)) => p.image(&format!("{key}:icon"), (x, y, icon, icon), res, Some(action.clone())),
                None => {
                    let shade = 0x3a6ea5ff ^ ((id.len() as u32).wrapping_mul(0x1d3a_5700) & 0x7f7f_7f00);
                    p.rect(&format!("{key}:icon"), (x, y, icon, icon), shade, (icon / 4) as u16, Some(action.clone()));
                    let initial: String = name.chars().take(1).collect();
                    let sz = if icon >= 32 { 16 } else { 8 };
                    p.text_c(&format!("{key}:initial"), x + icon / 2, y + (icon - sz as i32) / 2, &initial, sz, 0xffffffff, None);
                }
            }
            let label = ellipsize(&name, 8, cell_w - 4);
            p.text_c(&format!("{key}:label"), col * cell_w + cell_w / 2, y + icon + 3, &label, 8, 0xf0f4f8ff, Some(action));
        }
        p.nodes
    }

    fn paint_status(&mut self) -> Vec<SceneNode> {
        let (w, h) = self.status_l.size;
        let st = self.status.clone();
        let clock = self.clock.clone();
        let unread = self.unread;
        let mut p = Painter::new(&mut self.hits);
        p.rect("bg", (0, 0, w, h), colors::BAR, 0, Some(Action::PullSheet));
        p.text("clock", 4, (h - 12) / 2, &clock, 12, colors::BAR_TEXT, Some(Action::PullSheet));
        let mut x = 4 + text_w(&clock, 12) + 4;
        if unread > 0 {
            p.rect("unread", (x, h / 2 - 2, 5, 5), colors::ACCENT, 2, None);
            x += 9;
        }
        // Background modes in use (audio playing in the background, ...).
        let mut seen: Vec<String> = Vec::new();
        for (_, mode) in &st.modes {
            if seen.contains(mode) {
                continue;
            }
            seen.push(mode.clone());
            let label = match mode.as_str() {
                "audio" => "AUDIO",
                "radio" => "RADIO",
                "transfer" => "DL",
                "navigation" => "NAV",
                other => other,
            }
            .to_string();
            let tw = text_w(&label, 8);
            if x + tw + 6 > w / 2 + 20 {
                break;
            }
            p.rect(&format!("mode:{mode}"), (x, 3, tw + 6, h - 6), colors::ACCENT, 3, None);
            p.text(&format!("mode:{mode}:t"), x + 3, (h - 8) / 2, &label, 8, 0x0b0f14ff, None);
            x += tw + 10;
        }
        // Right side, right to left: battery, Wi-Fi, cellular.
        let mut rx = w - 4;
        let bw = 16;
        let by = (h - 8) / 2;
        rx -= 2;
        p.rect("bat:nub", (rx, by + 2, 2, 4), colors::BAR_TEXT, 0, None);
        rx -= bw;
        p.frame("bat:frame", (rx, by, bw, 8), colors::BAR_TEXT, 1, 1);
        let pct = st.battery.clamp(0.0, 100.0);
        let fill_w = ((bw - 4) as f32 * pct / 100.0).round() as i32;
        let bc = if pct <= 15.0 { colors::DANGER } else if pct <= 35.0 { 0xe8b339ff } else { 0x5ad17aff };
        p.rect("bat:fill", (rx + 2, by + 2, fill_w.max(1), 4), bc, 0, None);
        let pct_s = format!("{:.0}%", pct);
        let tw = text_w(&pct_s, 8);
        rx -= tw + 3;
        p.text("bat:pct", rx, (h - 8) / 2, &pct_s, 8, colors::BAR_TEXT, None);
        if st.wifi >= 0 {
            // A fan of three arcs, as stacked bars that narrow downwards.
            rx -= 14;
            let lit = match st.wifi {
                0 => 0,
                1 => 1,
                2 | 3 => 2,
                _ => 3,
            };
            for i in 0..3 {
                let bw = 12 - i * 4;
                let c = if 3 - i <= lit { colors::BAR_TEXT } else { 0x4a5563ff };
                p.rect(&format!("wifi{i}"), (rx + i * 2, 3 + i * 3, bw, 2), c, 0, None);
            }
            p.rect("wifi:dot", (rx + 5, 12, 2, 2), if lit > 0 { colors::BAR_TEXT } else { 0x4a5563ff }, 0, None);
        }
        if st.cell >= 0 {
            rx -= 15;
            for i in 0..4 {
                let bh = 3 + i * 2;
                let c = if i < st.cell { colors::BAR_TEXT } else { 0x4a5563ff };
                p.rect(&format!("cell{i}"), (rx + i * 3, 12 - bh + 1, 2, bh), c, 0, None);
            }
            // The carrier's name before the bars (celld's report), when there is room.
            if !st.carrier.is_empty() {
                let name = ellipsize(&st.carrier, 8, 64);
                let tw = text_w(&name, 8);
                if rx - tw - 3 > x + 4 {
                    rx -= tw + 3;
                    p.text("carrier", rx, (h - 8) / 2, &name, 8, colors::BAR_TEXT, None);
                }
            }
        }
        p.nodes
    }

    fn paint_nav(&mut self) -> Vec<SceneNode> {
        let (w, h) = self.nav.size;
        let mut p = Painter::new(&mut self.hits);
        p.rect("bg", (0, 0, w, h), colors::BAR, 0, None);
        let third = w / 3;
        let cy = h / 2;
        let fg = colors::BAR_TEXT;
        // Back: a chevron.
        p.rect("back:hit", (0, 0, third, h), 0, 0, Some(Action::Back));
        let bx = third / 2;
        p.line("back", &[(bx + 3, cy - 5), (bx - 3, cy), (bx + 3, cy + 5)], 2, fg);
        // Home: a ring.
        p.rect("home:hit", (third, 0, third, h), 0, 0, Some(Action::Home));
        p.frame("home", (w / 2 - 6, cy - 6, 12, 12), fg, 2, 6);
        // Recent apps: a square.
        p.rect("recents:hit", (2 * third, 0, w - 2 * third, h), 0, 0, Some(Action::Recents));
        p.frame("recents", (2 * third + third / 2 - 5, cy - 5, 10, 10), fg, 2, 1);
        p.nodes
    }

    fn paint_overlay(&mut self) -> Vec<SceneNode> {
        let (w, h) = self.overlay_l.as_ref().map(|l| l.size).unwrap_or((0, 0));
        let overlay = self.overlay.clone();
        let notifications = self.notifications.clone();
        let recents: Vec<(u32, String, String)> = self
            .recents
            .iter()
            .filter_map(|&id| self.sh.window(id).map(|w| (id, w.title.clone(), w.app_id.clone())))
            .collect();
        let names: HashMap<String, String> = self.apps.iter().map(|a| (a.id.clone(), a.name.clone())).collect();
        let hotkeys = self.status.hotkeys.clone();
        let name_of = |id: &str| names.get(id).cloned().unwrap_or_else(|| id.to_string());
        let mut p = Painter::new(&mut self.hits);
        match overlay {
            None => {}
            Some(Overlay::Sheet) => {
                let sh = (h * 3 / 4).max(120);
                p.rect("scrim", (0, 0, w, h), colors::SCRIM, 0, Some(Action::Dismiss));
                p.rect("sheet", (0, 0, w, sh), colors::SHEET, 0, Some(Action::Nothing));
                p.text("title", 8, 6, "Notifications", 12, colors::CARD_TEXT, None);
                if notifications.is_empty() {
                    p.text("empty", 8, 30, "No notifications", 8, colors::MUTED, None);
                } else {
                    p.text("clear", w - 8 - text_w("Clear all", 8), 8, "Clear all", 8, colors::ACCENT, Some(Action::ClearNotifications));
                }
                let mut y = 24;
                for n in notifications.iter() {
                    if y + 32 > sh - 12 {
                        break;
                    }
                    let k = format!("n{}", n.nid);
                    let a = Action::Notification(n.nid);
                    p.rect(&format!("{k}:card"), (6, y, w - 12, 30), colors::CARD, 4, Some(a.clone()));
                    let app = ellipsize(&name_of(&n.app_id), 8, w - 24);
                    p.text(&format!("{k}:app"), 12, y + 3, &app, 8, colors::MUTED, Some(a.clone()));
                    p.text(&format!("{k}:title"), 12, y + 11, &ellipsize(&n.title, 8, w - 24), 8, colors::CARD_TEXT, Some(a.clone()));
                    p.text(&format!("{k}:body"), 12, y + 20, &ellipsize(&n.body, 8, w - 24), 8, colors::MUTED, Some(a));
                    y += 34;
                }
                p.rect("handle", (w / 2 - 12, sh - 6, 24, 3), colors::MUTED, 1, Some(Action::Dismiss));
            }
            Some(Overlay::Recents) => {
                p.rect("scrim", (0, 0, w, h), 0x05080ce6, 0, Some(Action::Dismiss));
                p.text("title", 8, STATUS_H + 6, "Recent apps", 12, colors::CARD_TEXT, None);
                if recents.is_empty() {
                    p.text("empty", 8, STATUS_H + 26, "No open apps", 8, colors::MUTED, None);
                }
                let mut y = STATUS_H + 24;
                for (id, title, app) in recents {
                    if y + 34 > h - NAV_H {
                        break;
                    }
                    let k = format!("w{id}");
                    p.rect(&format!("{k}:card"), (6, y, w - 12, 30), colors::CARD, 4, Some(Action::Show(id)));
                    let label = if title.is_empty() { name_of(&app) } else { title };
                    p.text(&format!("{k}:title"), 12, y + 4, &ellipsize(&label, 12, w - 50), 12, colors::CARD_TEXT, Some(Action::Show(id)));
                    p.text(&format!("{k}:app"), 12, y + 18, &ellipsize(&name_of(&app), 8, w - 50), 8, colors::MUTED, Some(Action::Show(id)));
                    p.rect(&format!("{k}:x"), (w - 30, y + 7, 16, 16), 0x3a4452ff, 8, Some(Action::Close(id)));
                    p.line(&format!("{k}:x1"), &[(w - 26, y + 11), (w - 18, y + 19)], 2, colors::CARD_TEXT);
                    p.line(&format!("{k}:x2"), &[(w - 18, y + 11), (w - 26, y + 19)], 2, colors::CARD_TEXT);
                    y += 34;
                }
            }
            Some(Overlay::Controls) => {
                p.rect("scrim", (0, 0, w, h), 0x05080cf2, 0, Some(Action::Nothing));
                p.text("title", 8, STATUS_H + 6, "Controls", 12, colors::CARD_TEXT, None);
                p.text("hint", 8, STATUS_H + 22, "Hotkey slots of open apps (in hand)", 8, colors::MUTED, None);
                let mut y = STATUS_H + 36;
                if hotkeys.is_empty() {
                    p.text("none", 8, y, "No app claims a slot", 8, colors::MUTED, None);
                    y += 12;
                }
                let mut sorted = hotkeys.clone();
                sorted.sort_by(|a, b| b.focused.cmp(&a.focused).then(a.app.cmp(&b.app)).then(a.slot.cmp(&b.slot)));
                let mut last_app = String::new();
                for hk in sorted {
                    if y + 12 > h - NAV_H - 60 {
                        break;
                    }
                    if hk.app != last_app {
                        last_app = hk.app.clone();
                        let head = format!("{}{}", name_of(&hk.app), if hk.focused { " (open)" } else { "" });
                        p.text(&format!("h:{}", hk.app), 8, y, &ellipsize(&head, 8, w - 16), 8, colors::ACCENT, None);
                        y += 11;
                    }
                    let k = format!("hk:{}:{}", hk.app, hk.slot);
                    p.rect(&format!("{k}:slot"), (12, y - 1, 14, 10), colors::CARD, 2, None);
                    p.text_c(&format!("{k}:n"), 19, y, &hk.slot.to_string(), 8, colors::CARD_TEXT, None);
                    p.text(&format!("{k}:label"), 32, y, &ellipsize(&hk.label, 8, w - 40), 8, colors::CARD_TEXT, None);
                    y += 12;
                }
                let keys = [
                    "Bind slots: Options > Controls > ePhone",
                    "Alt+H home   Alt+Backspace back",
                    "Alt+Tab next app   Alt+N notifications",
                    "Switch-app key: next app, in hand",
                ];
                let mut ky = h - NAV_H - 52;
                for (i, k) in keys.iter().enumerate() {
                    p.text(&format!("keys{i}"), 8, ky, &ellipsize(k, 8, w - 16), 8, colors::MUTED, None);
                    ky += 11;
                }
            }
            Some(Overlay::Permission { app, permission, .. }) => {
                p.rect("scrim", (0, 0, w, h), colors::SCRIM, 0, Some(Action::Nothing));
                let cw = w - 24;
                let ch = 128;
                let (cx, cy) = (12, (h - ch) / 2);
                p.rect("card", (cx, cy, cw, ch), colors::SHEET, 6, Some(Action::Nothing));
                let name = name_of(&app);
                let lines = ecm_ui::text::wrap(&face(12), &format!("Allow {} to use {}?", name, permission_words(&permission)), Some(cw - 16));
                let mut ty = cy + 8;
                for (i, l) in lines.iter().take(3).enumerate() {
                    p.text(&format!("q{i}"), cx + 8, ty, l, 12, colors::CARD_TEXT, None);
                    ty += 13;
                }
                p.text("perm", cx + 8, ty + 2, &ellipsize(&permission, 8, cw - 16), 8, colors::MUTED, None);
                let bh = 18;
                let mut by = cy + ch - 3 * (bh + 4) - 2;
                for (k, label, a, fill) in [
                    ("allow", "Allow", Action::Answer { allow: true, remember: true }, colors::ACCENT),
                    ("once", "Only this time", Action::Answer { allow: true, remember: false }, colors::CARD),
                    ("deny", "Don't allow", Action::Answer { allow: false, remember: true }, colors::CARD),
                ] {
                    p.rect(&format!("{k}:b"), (cx + 8, by, cw - 16, bh), fill, 4, Some(a.clone()));
                    let tc = if fill == colors::ACCENT { 0x0b0f14ff } else { colors::CARD_TEXT };
                    p.text_c(&format!("{k}:t"), cx + cw / 2, by + (bh - 12) / 2, label, 12, tc, Some(a));
                    by += bh + 4;
                }
            }
        }
        p.nodes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecm_ui::MockConnection;

    struct Fixed;
    impl Env for Fixed {
        fn clock(&mut self) -> String {
            "12:00".into()
        }
        fn status(&mut self) -> Status {
            Status::parse("state awake\nbattery 80\nwifi 3\ncell 2\ncarrier cell-red\nforeground -\nmode dev.x audio\nhotkey dev.ecm.notes 1 * Add note\n")
        }
        fn apps(&mut self) -> Vec<AppEntry> {
            vec![AppEntry { id: "dev.ecm.notes".into(), name: "Notes".into(), icon: None }]
        }
        fn session(&mut self) -> Vec<String> {
            vec![]
        }
    }

    fn welcome() -> Message {
        let mut m = ecm_ui::samples::welcome(false);
        if let Message::Welcome(ref mut w) = m {
            w.display.w = 240;
            w.display.h = 400;
            w.display.size_class = proto::SizeClass::Compact;
        }
        m
    }

    #[test]
    fn status_parses() {
        let s = Fixed.status();
        assert_eq!(s.battery, 80.0);
        assert_eq!((s.wifi, s.cell, s.carrier.as_str()), (3, 2, "cell-red"));
        assert_eq!(Status::parse("cell 1\ncarrier -\n").carrier, "", "- = on no cell");
        assert_eq!(s.modes, vec![("dev.x".to_string(), "audio".to_string())]);
        assert_eq!(s.hotkeys, vec![HotkeyClaim { app: "dev.ecm.notes".into(), slot: 1, focused: true, label: "Add note".into() }]);
    }

    #[test]
    fn opens_apps_one_at_a_time_and_goes_home() {
        let mock = MockConnection::new();
        mock.push(&[welcome()]);
        let mut ps = PhoneShell::new(Box::new(mock.clone()), Fixed, false).unwrap();
        for (s, w, h) in [(1, 240, 400), (2, 240, 16), (3, 240, 20)] {
            ps.handle(vec![ShellEvent::Configure(proto::Configure { surface: s, serial: 1, w, h, states: 0 })]);
        }
        ps.render().unwrap();
        mock.clear_sent();
        ps.open_app("dev.ecm.notes");
        assert!(mock.sent_messages().is_empty(), "nothing until render flushes");
        ps.render().unwrap();
        assert!(mock.sent_messages().iter().any(|m| matches!(m, Message::Launch(l) if l.app_id == "dev.ecm.notes")));
        // The window opens: shown between the bars, focused.
        ps.sh.windows.push(ecm_ui::shell::ShellWindow { id: 7, pid: 5, app_id: "dev.ecm.notes".into(), title: "Notes".into(), min: (0, 0), max: (0, 0), parent: 0 });
        mock.clear_sent();
        ps.handle(vec![ShellEvent::Opened(7)]);
        ps.render().unwrap();
        let sent = mock.sent_messages();
        assert!(sent.iter().any(|m| matches!(m, Message::Place(p) if p.window == 7 && p.y == STATUS_H && p.h == 400 - STATUS_H - NAV_H)));
        assert!(sent.iter().any(|m| matches!(m, Message::FocusWindow(f) if f.window == 7)));
        assert_eq!(ps.current, Some(7));
        mock.clear_sent();
        ps.handle(vec![ShellEvent::Chord(CHORD_HOME)]);
        ps.render().unwrap();
        assert!(mock.sent_messages().iter().any(|m| matches!(m, Message::SetVisible(v) if v.window == 7 && !v.visible)));
        assert_eq!(ps.current, None);
        // The switch-app key brings it back.
        ps.handle(vec![ShellEvent::Chord(CHORD_SWITCH_APP)]);
        assert_eq!(ps.current, Some(7));
    }

    #[test]
    fn permission_dialog_answers() {
        let mock = MockConnection::new();
        mock.push(&[welcome()]);
        let mut ps = PhoneShell::new(Box::new(mock.clone()), Fixed, true).unwrap();
        ps.handle(vec![ShellEvent::Message(Message::PermissionPrompt(proto::PermissionPrompt {
            req: 9,
            app_id: "dev.ecm.notes".into(),
            permission: "network".into(),
        }))]);
        assert!(matches!(ps.overlay, Some(Overlay::Permission { req: 9, .. })));
        ps.handle(vec![ShellEvent::Configure(proto::Configure { surface: SURF_OVERLAY, serial: 1, w: 240, h: 400, states: 0 })]);
        ps.render().unwrap();
        mock.clear_sent();
        ps.act(Action::Answer { allow: true, remember: false });
        ps.render().unwrap();
        let sent = mock.sent_messages();
        assert!(sent.iter().any(|m| matches!(m, Message::AnswerPermission(a) if a.req == 9 && a.allow && !a.remember)));
        assert!(sent.iter().any(|m| matches!(m, Message::SurfaceDestroy(d) if d.surface == SURF_OVERLAY)));
        assert_eq!(ps.overlay, None);
    }
}
