//! `dev.ecm.desktop`: the desktop shell (ROADMAP P3).
//!
//! * **Wallpaper** on the `background` layer and a **taskbar** on the `top`
//!   layer: the Apps button (a launcher popup listing the core apps and
//!   anything installed under `/apps`), one button per window (click to
//!   focus, click the focused one to minimize), Wi-Fi and cellular bars
//!   (click opens Settings) and a clock.
//! * **Floating windows with title bars drawn by the shell.** The compositor
//!   never shows the shell an app's pixels, so each app window gets a
//!   *frame*: a window surface of the shell's own, stacked right below the
//!   app's and placed around it. The frame draws the title, the minimize /
//!   maximize / close buttons and the border; dragging the title moves the
//!   window and dragging the border resizes it. While dragging, an outline
//!   is drawn on the `overlay` layer and the window is placed when the
//!   button is released, so the frame never moves under the pointer.
//! * **Alt+Tab** shows a switcher (most recently used first); each press
//!   moves on, and the selection is focused once Tab stays up for a moment.
//!   **Alt+F4** closes the focused window, **Ctrl+Esc** opens the launcher.
//! * **Notification toasts** (`NotificationPosted`) in the top right corner;
//!   clicking one (or an action) sends `NotificationInvoke`.
//! * **Permission prompts** (`PermissionPrompt`) as a dialog on the overlay
//!   layer, answered with `AnswerPermission`.
//! * **The theme** is `/etc/ui.toml`'s `theme`: sent with `SetTheme` at start
//!   and again whenever the file changes (Settings writes it), so every app
//!   restyles live.
//! * **Restart-safe**: after a crash the compositor starts the shell again
//!   with `--restart` and reports every open window; the desktop frames them
//!   again and doesn't relaunch the session's `--launch` apps.
//! * **The last session**: at its first start (not with `--restart`) the
//!   desktop relaunches the apps that were running when the computer last
//!   shut down (`app_ctl session`, the app manager's `lastSession()`).
//! * **The tray** (in the taskbar, left of the Wi-Fi bars): one icon per app
//!   with an active background mode (`AppModes`) or a window hidden to the
//!   tray. Closing the window of an app with an active mode hides it to the
//!   tray (`SetVisible` false) instead of closing it; a click on its icon
//!   shows it again, right-click has Open and Quit (Quit = `CloseWindow` on
//!   its windows). A speaker icon opens the Volume Mixer popup: Now Playing
//!   (`MediaSessionChanged`, buttons send `MediaControl`), a master strip per
//!   speaker, a strip per app (volume 0-150 %, mute, meter, "ducked", and a
//!   route picker with several speakers) and the microphones (`/dev/mixer`).

use crate::sys::{self, CellState, Sys, WifiState};
use crate::uitoml;
use ecm_audio::mixer::{self, Strips};
use ecm_ui::{MediaSession, MediaState};
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::element::{button, checkbox, column, custom, image, label, list, row, slider, spacer, stack, Align, Element};
use ecm_ui::window::{Content, EventValue};
use ecm_ui::{App, Core, Ctx, Theme};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Message, NodeKind, Prop};
use ecm_ui::diff::SceneNode;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

/// Taskbar height (lu).
pub const TASKBAR_H: i32 = 24;
/// Title bar height (lu).
pub const TITLE_H: i32 = 18;
/// Border around a window (lu): the resize grip.
pub const BORDER: i32 = 3;
/// Width of the frame's minimize / maximize / close buttons at the title bar's right end.
pub const TITLE_BUTTONS_W: i32 = 3 * 20 + 2 * 1 + 2 * 1;
/// How long a toast stays (ms).
pub const TOAST_MS: u64 = 6000;
/// Alt+Tab commits the selection after this long without another press (ms).
pub const SWITCH_COMMIT_MS: u64 = 900;
/// Status (Wi-Fi, cellular, clock) refresh period (ms).
pub const STATUS_MS: u64 = 2000;
/// How often `/etc/ui.toml` is checked for a new theme (ms).
pub const THEME_POLL_MS: u64 = 1000;
/// How often the Volume Mixer popup's strips and meters are read again while it is open (ms).
pub const MIXER_MS: u64 = 500;
/// Width of the Volume Mixer popup (lu).
pub const MIXER_W: i32 = 300;

pub const CHORD_SWITCH: u32 = 1;
pub const CHORD_CLOSE: u32 = 2;
pub const CHORD_LAUNCHER: u32 = 3;

const HID_TAB: u16 = 0x2B;
const HID_F4: u16 = 0x3D;
const HID_ESCAPE: u16 = 0x29;

/// One launcher entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchItem {
    pub label: String,
    /// The program (or app id) `Launch` starts.
    pub program: String,
    pub args: Vec<String>,
    /// A built-in icon name (`ecm_ui::icons`).
    pub icon: String,
}

fn item(label: &str, program: &str, icon: &str) -> LaunchItem {
    LaunchItem { label: label.into(), program: program.into(), args: Vec::new(), icon: icon.into() }
}

/// The core apps, in launcher order.
pub fn core_apps() -> Vec<LaunchItem> {
    vec![
        item("Terminal", "terminal", "terminal"),
        item("Files", "files", "files"),
        item("Text Editor", "editor", "editor"),
        item("Settings", "settings", "settings"),
        item("Notes", "notes", "notes"),
        item("Visual Programmer", "visual", "visual"),
        // Not an app: the compositor goes back to the console (the GUI keeps running).
        item("Console", "console", "terminal"),
    ]
}

/// System apps that are copies of a core app above (the Text Editor that
/// "open with" uses, confined to the file it was given; the ePhone's built-in
/// Notes app, the same `notes` program, should its `/apps` folder be present):
/// not listed twice.
pub const SHADOWED_APPS: &[&str] = &["dev.ecm.editor", "dev.ecm.notes"];

/// Apps installed under `/apps/<id>/` (an `app.toml` with `name = "..."`),
/// the App Store among them. `Launch` of the id starts them through the app
/// manager (their identity and sandbox).
pub fn installed_apps(sys: &dyn Sys) -> Vec<LaunchItem> {
    let Ok(entries) = sys::list_dir(sys, "/apps") else { return Vec::new() };
    let mut out = Vec::new();
    for e in entries.into_iter().filter(|e| e.dir && !SHADOWED_APPS.contains(&e.name.as_str())) {
        let name = app_name(sys, &e.name).unwrap_or_else(|| e.name.clone());
        out.push(LaunchItem { label: name, program: e.name.clone(), args: Vec::new(), icon: icon_for(&e.name).into() });
    }
    out
}

/// The launcher's entries: the core apps, then what is installed now.
pub fn launch_items(sys: &dyn Sys) -> Vec<LaunchItem> {
    let mut items = core_apps();
    items.extend(installed_apps(sys));
    items
}

/// An installed app's `name` from `/apps/<id>/app.toml` (the permission dialog names the app with it).
pub fn app_name(sys: &dyn Sys, app_id: &str) -> Option<String> {
    if app_id.is_empty() || app_id.contains('/') || app_id.contains("..") {
        return None;
    }
    let toml = sys::read_file(sys, &format!("/apps/{app_id}/app.toml")).ok().map(|b| String::from_utf8_lossy(&b).into_owned())?;
    uitoml::get(&toml, "name").filter(|n| !n.is_empty())
}

/// The icon for an app id (program name or reverse-DNS id).
pub fn icon_for(app_id: &str) -> &'static str {
    let last = app_id.rsplit('.').next().unwrap_or(app_id);
    match last {
        "terminal" => "terminal",
        "files" => "files",
        "editor" => "editor",
        "settings" => "settings",
        "notes" => "notes",
        "gba" => "game",
        "visual" => "visual",
        "store" => "store",
        _ => "app",
    }
}

// ------------------------------------------------------------------ the computer's app manager and mixer

/// What the desktop asks the computer's app manager for besides [`Sys`] (which has the
/// Volume Mixer, `Sys::mixer`): the last session, and stopping an app with no window.
pub trait DesktopHost {
    /// Apps that were running at the last shutdown (`app_ctl session`).
    fn last_session(&mut self) -> Vec<String>;
    /// Stop an app that has no window to close (`app_ctl kill <pid>`).
    fn kill(&mut self, pid: u32);
}

/// The computer's app manager (`app_ctl`; nothing off-target).
pub struct LiveHost;

impl DesktopHost for LiveHost {
    fn last_session(&mut self) -> Vec<String> {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some((0, s)) = ecm_host_abi::apps::ctl(&["session"]) {
                return String::from_utf8_lossy(&s).lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
            }
        }
        Vec::new()
    }

    fn kill(&mut self, pid: u32) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ecm_host_abi::apps::ctl(&["kill", &pid.to_string()]);
        }
        let _ = pid;
    }
}

/// What a [`FakeHost`] holds and was asked.
#[derive(Clone, Debug, Default)]
pub struct FakeHostState {
    pub session: Vec<String>,
    pub killed: Vec<u32>,
}

/// An in-memory computer for tests and golden sessions (shared, so the test can still look).
#[derive(Clone, Default)]
pub struct FakeHost {
    pub state: Rc<RefCell<FakeHostState>>,
}

impl DesktopHost for FakeHost {
    fn last_session(&mut self) -> Vec<String> {
        self.state.borrow().session.clone()
    }
    fn kill(&mut self, pid: u32) {
        self.state.borrow_mut().killed.push(pid);
    }
}

type HostRef = Rc<RefCell<Box<dyn DesktopHost>>>;

/// What a background mode keeps an app running for, in the first-time prompt.
pub fn background_words(mode: &str) -> String {
    match mode {
        "call" => "calls".into(),
        "audio" => "audio".into(),
        "radio" => "radio".into(),
        "transfer" => "downloads".into(),
        "navigation" => "navigation".into(),
        "refresh" => "updates".into(),
        other => other.to_string(),
    }
}

/// The permission dialog's question: "Allow <app> to use <permission>?", or for a background
/// mode's first-time prompt (`background:<mode>`) "Keep <app> running in the background for <calls|...>?".
pub fn prompt_question(app_name: &str, permission: &str) -> String {
    match permission.strip_prefix("background:") {
        Some(mode) => format!("Keep {app_name} running in the background for {}?", background_words(mode)),
        None => format!("Allow {app_name} to use {permission}?"),
    }
}

// ------------------------------------------------------------------ geometry

/// (x, y, w, h)
pub type R = (i32, i32, i32, i32);

/// The area windows may use: the display above the taskbar.
pub fn work_area(d: &proto::DisplayInfo) -> R {
    (0, 0, d.w, (d.h - TASKBAR_H).max(TITLE_H + 20))
}

/// The frame around a window's content rect.
pub fn frame_rect(content: R, maximized: bool) -> R {
    let b = if maximized { 0 } else { BORDER };
    (content.0 - b, content.1 - TITLE_H, content.2 + 2 * b, content.3 + TITLE_H + b)
}

/// The content rect of a maximized window.
pub fn maximized_rect(d: &proto::DisplayInfo) -> R {
    let wa = work_area(d);
    (wa.0, wa.1 + TITLE_H, wa.2, wa.3 - TITLE_H)
}

/// Where the `n`-th new window goes (cascading), at least `min` big.
pub fn initial_rect(d: &proto::DisplayInfo, n: usize, min: (i32, i32)) -> R {
    let wa = work_area(d);
    let w = (wa.2 * 3 / 4).min(560).max(min.0).min(wa.2 - 2 * BORDER);
    let h = (wa.3 * 3 / 4).min(340).max(min.1).min(wa.3 - TITLE_H - BORDER);
    let k = (n % 6) as i32;
    let x = (24 + 24 * k).min((wa.2 - w - BORDER).max(BORDER));
    let y = (TITLE_H + 12 + 22 * k).min((wa.3 - h - BORDER).max(TITLE_H));
    (x, y, w, h)
}

/// Keep a window's title bar reachable and its size within its limits.
pub fn clamp_rect(d: &proto::DisplayInfo, r: R, min: (i32, i32), max: (i32, i32)) -> R {
    let wa = work_area(d);
    let mut w = r.2.max(min.0.max(80));
    let mut h = r.3.max(min.1.max(40));
    if max.0 > 0 {
        w = w.min(max.0);
    }
    if max.1 > 0 {
        h = h.min(max.1);
    }
    w = w.min(wa.2);
    h = h.min(wa.3 - TITLE_H);
    let x = r.0.clamp(-w + 40, wa.2 - 40);
    let y = r.1.clamp(TITLE_H, wa.3 - 8);
    (x, y, w, h)
}

/// A window rect after dragging by `(dx, dy)`: moved, or resized at `edges`.
pub fn dragged(r0: R, edges: u8, dx: i32, dy: i32) -> R {
    use proto::edges as e;
    if edges == 0 {
        return (r0.0 + dx, r0.1 + dy, r0.2, r0.3);
    }
    let (mut x, mut y, mut w, mut h) = r0;
    if edges & e::LEFT != 0 {
        x += dx;
        w -= dx;
    }
    if edges & e::RIGHT != 0 {
        w += dx;
    }
    if edges & e::TOP != 0 {
        y += dy;
        h -= dy;
    }
    if edges & e::BOTTOM != 0 {
        h += dy;
    }
    (x, y, w, h)
}

// ------------------------------------------------------------------ state

/// One app window the desktop manages.
#[derive(Clone, Debug)]
pub struct Win {
    pub id: u32,
    pub pid: u32,
    pub app_id: String,
    pub title: String,
    pub min: (i32, i32),
    pub max: (i32, i32),
    /// The shell's frame surface (its own surface id) and the compositor's id for it.
    pub frame_surface: u32,
    pub frame_wid: Option<u32>,
    /// Content rect (lu).
    pub rect: R,
    /// Rect before maximizing.
    pub saved: Option<R>,
    pub minimized: bool,
    /// Closed while its app had a background mode: hidden to the tray (not in the taskbar).
    pub hidden: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    sent: Option<(R, u32)>,
    sent_frame: Option<R>,
    shown: Option<bool>,
}

/// A drag on a frame.
#[derive(Clone, Debug)]
pub struct Drag {
    pub win: u32,
    /// 0 = move, otherwise `proto::edges` bits.
    pub edges: u8,
    pub start: (i32, i32),
    pub rect0: R,
    pub rect: R,
    pub outline: u32,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub nid: u32,
    pub app_id: String,
    pub tag: String,
    pub height: i32,
    pub title: String,
    pub body: String,
    pub actions: Vec<String>,
    pub surface: u32,
    pub until: u64,
}

#[derive(Clone, Debug)]
pub struct Prompt {
    pub req: u32,
    pub app_id: String,
    /// The app's `name` from its `app.toml` (the id when it has none).
    pub app_name: String,
    pub permission: String,
}

/// An app's active background modes (`AppModes`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayApp {
    pub pid: u32,
    pub app_id: String,
    pub modes: Vec<String>,
}

/// An app's media session (`MediaSessionChanged`), for Now Playing in the mixer popup.
#[derive(Clone, Debug)]
pub struct TrayMedia {
    pub pid: u32,
    pub app_id: String,
    pub session: MediaSession,
}

/// One icon in the tray.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayEntry {
    pub app_id: String,
    pub pid: u32,
    /// Active background modes (may be empty while a window is still hidden to the tray).
    pub modes: Vec<String>,
    /// Windows of it hidden to the tray.
    pub hidden: Vec<u32>,
}

/// What closing a window's title bar does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    /// Its app has an active background mode: hide it to the tray.
    HideToTray,
    /// `CloseWindow` (the app quits or closes the window).
    Close,
}

#[derive(Clone, Debug)]
pub struct Switcher {
    pub order: Vec<u32>,
    pub index: usize,
    pub surface: u32,
    pub last: u64,
}

/// Everything the desktop knows.
pub struct State {
    pub display: proto::DisplayInfo,
    pub own_app_id: String,
    /// App windows, bottom to top.
    pub wins: Vec<Win>,
    pub focused: Option<u32>,
    /// Most recently focused first (Alt+Tab order).
    pub mru: Vec<u32>,
    pub opened: usize,
    pub drag: Option<Drag>,
    pub toasts: Vec<Toast>,
    pub prompts: VecDeque<Prompt>,
    pub prompt_surface: Option<u32>,
    pub switcher: Option<Switcher>,
    pub launcher: Option<u32>,
    pub launch_items: Vec<LaunchItem>,
    pub wifi: WifiState,
    pub cells: Vec<CellState>,
    pub clock: Option<(u32, u32)>,
    pub theme_name: String,
    pub taskbar: u32,
    pub wallpaper: u32,
    pub icons: HashMap<String, u32>,
    pub last_status: Option<u64>,
    pub last_theme_poll: Option<u64>,
    pub last_title_click: Option<(u32, u64)>,
    stacked: Vec<u32>,
    /// Apps with active background modes, in the order they started.
    pub tray: Vec<TrayApp>,
    /// Media sessions, newest first.
    pub media: Vec<TrayMedia>,
    /// `/dev/mixer` as last read.
    pub mixer: Strips,
    pub last_mixer: Option<u64>,
    /// The Volume Mixer popup and what it was sized for.
    pub mixer_popup: Option<u32>,
    mixer_shape: Vec<String>,
    host: HostRef,
    /// The computer (the mixer's commands are sent from widget handlers).
    sys: SysRef,
}

impl State {
    pub fn win(&self, id: u32) -> Option<&Win> {
        self.wins.iter().find(|w| w.id == id)
    }
    fn win_mut(&mut self, id: u32) -> Option<&mut Win> {
        self.wins.iter_mut().find(|w| w.id == id)
    }
    /// The app window a compositor window id belongs to (itself or its frame).
    pub fn owner(&self, wid: u32) -> Option<u32> {
        self.wins.iter().find(|w| w.id == wid || w.frame_wid == Some(wid)).map(|w| w.id)
    }
    fn topmost_shown(&self) -> Option<u32> {
        self.wins.iter().rev().find(|w| !w.minimized && !w.hidden).map(|w| w.id)
    }

    /// The active background modes of the app a window belongs to (same pid or app id).
    pub fn modes_of(&self, wid: u32) -> Vec<String> {
        let Some(w) = self.win(wid) else { return Vec::new() };
        self.tray.iter().filter(|t| t.pid == w.pid || t.app_id == w.app_id).flat_map(|t| t.modes.iter().cloned()).collect()
    }

    /// What the title bar's close does for `wid`: an app with an active background mode is
    /// hidden to the tray; any other window is closed.
    pub fn close_action(&self, wid: u32) -> CloseAction {
        if self.modes_of(wid).is_empty() {
            CloseAction::Close
        } else {
            CloseAction::HideToTray
        }
    }

    /// The tray's icons: every app with an active mode, then apps that only have windows
    /// hidden to the tray (their mode ended since).
    pub fn tray_entries(&self) -> Vec<TrayEntry> {
        let mut out: Vec<TrayEntry> = Vec::new();
        for t in &self.tray {
            if t.app_id == self.own_app_id {
                continue;
            }
            match out.iter_mut().find(|e| e.app_id == t.app_id) {
                Some(e) => e.modes.extend(t.modes.iter().cloned()),
                None => out.push(TrayEntry { app_id: t.app_id.clone(), pid: t.pid, modes: t.modes.clone(), hidden: Vec::new() }),
            }
        }
        for w in self.wins.iter().filter(|w| w.hidden) {
            match out.iter_mut().find(|e| e.app_id == w.app_id) {
                Some(e) => e.hidden.push(w.id),
                None => out.push(TrayEntry { app_id: w.app_id.clone(), pid: w.pid, modes: Vec::new(), hidden: vec![w.id] }),
            }
        }
        out
    }

    /// Windows of an app (hidden or not), bottom to top.
    pub fn windows_of(&self, app_id: &str) -> Vec<u32> {
        self.wins.iter().filter(|w| w.app_id == app_id).map(|w| w.id).collect()
    }
}

type Shared = Rc<RefCell<State>>;
type SysRef = Rc<RefCell<Box<dyn Sys>>>;

/// The desktop shell over a connection with the shell role.
pub struct Desktop {
    pub app: App,
    pub state: Shared,
    pub sys: SysRef,
}

fn host(st: &Shared) -> HostRef {
    st.borrow().host.clone()
}

fn ms(sys: &SysRef) -> u64 {
    sys.borrow_mut().now_ms()
}

impl Desktop {
    /// Connect as the shell and set up the desktop. `args`: the program's
    /// arguments (`--restart`, `--launch PROG[:ARG...]`).
    pub fn new(conn: Box<dyn Connection>, sys: Box<dyn Sys>, args: &[String]) -> Result<Desktop, ConnError> {
        Desktop::with_host(conn, sys, Box::new(LiveHost), args)
    }

    /// [`Desktop::new`] with the app manager and mixer it uses (tests and golden sessions pass a [`FakeHost`]).
    pub fn with_host(conn: Box<dyn Connection>, sys: Box<dyn Sys>, host: Box<dyn DesktopHost>, args: &[String]) -> Result<Desktop, ConnError> {
        let core = Core::connect("dev.ecm.desktop", conn, proto::Role::Shell)?;
        let app = App::from_core(core);
        let sys: SysRef = Rc::new(RefCell::new(sys));
        let display = app.core.welcome.display.clone();
        let own = app.core.welcome.app_id.clone();
        let items = launch_items(&**sys.borrow());
        let state = Rc::new(RefCell::new(State {
            display,
            own_app_id: own,
            wins: Vec::new(),
            focused: None,
            mru: Vec::new(),
            opened: 0,
            drag: None,
            toasts: Vec::new(),
            prompts: VecDeque::new(),
            prompt_surface: None,
            switcher: None,
            launcher: None,
            launch_items: items,
            wifi: WifiState::default(),
            cells: Vec::new(),
            clock: None,
            theme_name: String::new(),
            taskbar: 0,
            wallpaper: 0,
            icons: HashMap::new(),
            last_status: None,
            last_theme_poll: None,
            last_title_click: None,
            stacked: Vec::new(),
            tray: Vec::new(),
            media: Vec::new(),
            mixer: Strips::default(),
            last_mixer: None,
            mixer_popup: None,
            mixer_shape: Vec::new(),
            host: Rc::new(RefCell::new(host)),
            sys: sys.clone(),
        }));
        let mut d = Desktop { app, state, sys };
        d.setup(args);
        Ok(d)
    }

    fn setup(&mut self, args: &[String]) {
        // Icons for the launcher and the taskbar.
        for name in ecm_ui::icons::NAMES {
            if let Some(i) = ecm_ui::icons::builtin(name) {
                if let Ok(id) = self.app.core.image_rgba(i.w, i.h, &i.rgba) {
                    self.state.borrow_mut().icons.insert(name.to_string(), id);
                }
            }
        }
        // The theme from /etc/ui.toml.
        let text = read_ui_toml(&self.sys);
        let name = text.as_deref().and_then(|t| uitoml::get(t, "theme")).unwrap_or_else(|| "ecm-default".into());
        apply_theme(&mut self.app.core, &self.state, &name);
        self.make_layers();
        let core = &mut self.app.core;
        core.send(Message::BindChord(proto::BindChord { id: CHORD_SWITCH, mods: proto::mods::ALT, keys: vec![HID_TAB] }));
        core.send(Message::BindChord(proto::BindChord { id: CHORD_CLOSE, mods: proto::mods::ALT, keys: vec![HID_F4] }));
        core.send(Message::BindChord(proto::BindChord { id: CHORD_LAUNCHER, mods: proto::mods::CTRL, keys: vec![HID_ESCAPE] }));
        refresh_status(&self.sys, &self.state, true);
        refresh_mixer(&self.sys, &self.state, true);

        // Session apps, unless this is a restart after a crash.
        let restarted = args.first().map(String::as_str) == Some("--restart");
        let mut launched: Vec<String> = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if a == "--launch" {
                if let Some(spec) = it.next() {
                    if restarted {
                        continue;
                    }
                    let mut parts = spec.split(':').map(String::from);
                    if let Some(prog) = parts.next() {
                        let rest: Vec<String> = parts.collect();
                        launched.push(prog.clone());
                        self.app.core.send(Message::Launch(proto::Launch { app_id: prog, args: rest }));
                    }
                }
            }
        }
        // The apps that were running at the last shutdown (the app manager's lastSession()).
        if !restarted {
            let own = self.state.borrow().own_app_id.clone();
            let session = host(&self.state).borrow_mut().last_session();
            for id in session {
                if id == own || id == "dev.ecm.desktop" || launched.contains(&id) {
                    continue;
                }
                launched.push(id.clone());
                self.app.core.send(Message::Launch(proto::Launch { app_id: id, args: Vec::new() }));
            }
        }

        let (st, sys) = (self.state.clone(), self.sys.clone());
        self.app.on_message(move |cx, m| on_message(cx, &st, &sys, m));
        let (st, sys) = (self.state.clone(), self.sys.clone());
        self.app.every(250, move |cx| tick(cx, &st, &sys));
        // Frames, bars and overlays are this program's own windows: they restyle like any app's.
        self.app.on_theme(|cx| cx.core().mark_all_dirty());
    }

    /// (Re)create the wallpaper and the taskbar for the current display.
    fn make_layers(&mut self) {
        let (old_tb, old_wp) = {
            let s = self.state.borrow();
            (s.taskbar, s.wallpaper)
        };
        if old_tb != 0 {
            self.app.core.close_window(old_tb);
        }
        if old_wp != 0 {
            self.app.core.close_window(old_wp);
        }
        let d = self.state.borrow().display.clone();
        let st = self.state.clone();
        let wp = self.app.core.create_layer(proto::Layer::Background, 0, 0, 0, 0, "wallpaper", Content::View(Box::new(move |cx| wallpaper_view(cx, &st))));
        let (st, sys) = (self.state.clone(), self.sys.clone());
        let tb = self.app.core.create_layer(
            proto::Layer::Top,
            0,
            d.h - TASKBAR_H,
            d.w,
            TASKBAR_H,
            "taskbar",
            Content::View(Box::new(move |cx| taskbar_view(cx, &st, &sys))),
        );
        let mut s = self.state.borrow_mut();
        s.wallpaper = wp;
        s.taskbar = tb;
    }

    /// One loop turn (see `App::step`).
    pub fn step(&mut self, timeout_ms: i32) -> bool {
        let ok = self.app.step(timeout_ms);
        // A DisplayChanged handled in on_message asks for new layers.
        let relayer = {
            let s = self.state.borrow();
            s.taskbar == 0
        };
        if relayer {
            self.make_layers();
            let st = self.state.clone();
            let mut cx = Ctx::new(&mut self.app.core, 0);
            place_all(&mut cx, &st);
        }
        ok
    }

    pub fn run(&mut self) {
        while self.step(-1) {}
    }
}

/// Run the desktop until the connection closes.
pub fn run(conn: Box<dyn Connection>, sys: Box<dyn Sys>, args: &[String]) -> Result<(), ConnError> {
    let mut d = Desktop::new(conn, sys, args)?;
    d.run();
    Ok(())
}

fn read_ui_toml(sys: &SysRef) -> Option<String> {
    let s = sys.borrow();
    sys::read_file(&**s, uitoml::PATH).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Send `SetTheme` for a built-in theme name (unknown names fall back to the default).
fn apply_theme(core: &mut Core, st: &Shared, name: &str) {
    let theme = Theme::named(name).unwrap_or_else(|| Theme::named("ecm-default").unwrap());
    st.borrow_mut().theme_name = name.to_string();
    core.send(Message::SetTheme(proto::SetTheme { theme: theme.to_wire() }));
}

fn refresh_status(sys: &SysRef, st: &Shared, force: bool) -> bool {
    let now = ms(sys);
    {
        let s = st.borrow();
        if !force && s.last_status.is_some_and(|t| now.saturating_sub(t) < STATUS_MS) {
            return false;
        }
    }
    let (wifi, cells, clock) = {
        let mut s = sys.borrow_mut();
        (s.wifi(), s.cells(), s.clock())
    };
    let mut s = st.borrow_mut();
    s.last_status = Some(now);
    let changed = s.wifi != wifi || s.cells != cells || s.clock != clock;
    s.wifi = wifi;
    s.cells = cells;
    s.clock = clock;
    changed
}

/// Re-read `/dev/mixer`: every [`STATUS_MS`] for the tray's speaker icon, every [`MIXER_MS`]
/// while the Volume Mixer popup is open. Returns whether it was read.
fn refresh_mixer(sys: &SysRef, st: &Shared, force: bool) -> bool {
    let now = ms(sys);
    {
        let s = st.borrow();
        let period = if s.mixer_popup.is_some() { MIXER_MS } else { STATUS_MS };
        if !force && s.last_mixer.is_some_and(|t| now.saturating_sub(t) < period) {
            return false;
        }
    }
    let strips = sys.borrow_mut().mixer().unwrap_or_default();
    let mut s = st.borrow_mut();
    s.last_mixer = Some(now);
    s.mixer = strips;
    true
}

/// What the tray's speaker icon shows: (there is a speaker, the first one is muted).
fn speaker_state(m: &Strips) -> (bool, bool) {
    (!m.outputs.is_empty(), m.outputs.first().is_some_and(|o| o.mute))
}

// ------------------------------------------------------------------ the timer

fn tick(cx: &mut Ctx, st: &Shared, sys: &SysRef) -> bool {
    let now = ms(sys);
    let mut changed = false;
    if refresh_status(sys, st, false) {
        let tb = st.borrow().taskbar;
        cx.set_window(tb);
        cx.invalidate();
        changed = true;
    }
    // The Volume Mixer: the tray's speaker icon, and the popup's meters while it is open.
    let before = speaker_state(&st.borrow().mixer);
    if refresh_mixer(sys, st, false) {
        if speaker_state(&st.borrow().mixer) != before {
            invalidate_taskbar(cx, st);
            changed = true;
        }
        if st.borrow().mixer_popup.is_some() {
            refresh_mixer_popup(cx, st, sys);
            changed = true;
        }
    }
    // Theme changes written by Settings.
    let poll = st.borrow().last_theme_poll.map_or(true, |t| now.saturating_sub(t) >= THEME_POLL_MS);
    if poll {
        st.borrow_mut().last_theme_poll = Some(now);
        if let Some(t) = read_ui_toml(sys) {
            let name = uitoml::get(&t, "theme").unwrap_or_else(|| "ecm-default".into());
            if name != st.borrow().theme_name {
                apply_theme(cx.core(), st, &name);
                changed = true;
            }
        }
    }
    // Toasts time out.
    let expired: Vec<u32> = st.borrow().toasts.iter().filter(|t| now >= t.until).map(|t| t.nid).collect();
    for nid in expired {
        remove_toast(cx, st, nid);
        changed = true;
    }
    // Alt+Tab: Tab stayed up long enough, focus the choice.
    let commit = st.borrow().switcher.as_ref().is_some_and(|s| now.saturating_sub(s.last) >= SWITCH_COMMIT_MS);
    if commit {
        finish_switch(cx, st);
        changed = true;
    }
    changed
}

// ------------------------------------------------------------------ compositor messages

fn on_message(cx: &mut Ctx, st: &Shared, sys: &SysRef, m: &Message) {
    match m {
        Message::WindowOpened(o) => {
            let own = st.borrow().own_app_id.clone();
            if o.app_id == own && o.title.starts_with("frame:") {
                // One of our frames: remember its window id.
                if let Ok(app) = o.title["frame:".len()..].parse::<u32>() {
                    if let Some(w) = st.borrow_mut().win_mut(app) {
                        w.frame_wid = Some(o.window);
                    }
                    place_all(cx, st);
                }
                return;
            }
            if o.app_id == own {
                return; // another surface of ours (none today)
            }
            let (rect, maximized) = {
                let s = st.borrow();
                let compact = s.display.size_class == proto::SizeClass::Compact;
                if compact {
                    (maximized_rect(&s.display), true)
                } else {
                    (initial_rect(&s.display, s.opened, (o.min_w, o.min_h)), false)
                }
            };
            let (st2, sys2) = (st.clone(), sys.clone());
            let wid = o.window;
            let frame = cx
                .core()
                .create_window(&format!("frame:{wid}"), Content::View(Box::new(move |vcx| frame_view(vcx, &st2, &sys2, wid))));
            {
                let mut s = st.borrow_mut();
                s.opened += 1;
                s.wins.retain(|w| w.id != wid);
                s.wins.push(Win {
                    id: wid,
                    pid: o.pid,
                    app_id: o.app_id.clone(),
                    title: o.title.clone(),
                    min: (o.min_w, o.min_h),
                    max: (o.max_w, o.max_h),
                    frame_surface: frame,
                    frame_wid: None,
                    rect,
                    saved: None,
                    minimized: false,
                    hidden: false,
                    maximized,
                    fullscreen: false,
                    sent: None,
                    sent_frame: None,
                    shown: None,
                });
            }
            focus(cx, st, wid);
        }
        Message::WindowChanged(c) => {
            let frame = {
                let mut s = st.borrow_mut();
                match s.win_mut(c.window) {
                    Some(w) => {
                        w.title = c.title.clone();
                        w.min = (c.min_w, c.min_h);
                        w.max = (c.max_w, c.max_h);
                        Some(w.frame_surface)
                    }
                    None => None,
                }
            };
            if let Some(f) = frame {
                invalidate(cx, f);
                invalidate_taskbar(cx, st);
            }
        }
        Message::WindowClosed(c) => {
            let gone = {
                let mut s = st.borrow_mut();
                let pos = s.wins.iter().position(|w| w.id == c.window);
                pos.map(|i| s.wins.remove(i))
            };
            if let Some(w) = gone {
                cx.core().close_window(w.frame_surface);
                let next = {
                    let mut s = st.borrow_mut();
                    s.mru.retain(|x| *x != w.id);
                    if s.focused == Some(w.id) {
                        s.focused = None;
                    }
                    if let Some(sw) = &mut s.switcher {
                        sw.order.retain(|x| *x != w.id);
                    }
                    if s.focused.is_none() {
                        s.topmost_shown()
                    } else {
                        None
                    }
                };
                match next {
                    Some(n) => focus(cx, st, n),
                    None => place_all(cx, st),
                }
                invalidate_taskbar(cx, st);
            }
        }
        Message::WindowRequest(r) => {
            let Some(id) = st.borrow().owner(r.window) else { return };
            match r.action {
                proto::WindowAction::Activate => focus(cx, st, id),
                proto::WindowAction::Minimize => minimize(cx, st, id),
                proto::WindowAction::Maximize => toggle_maximize(cx, st, id),
                proto::WindowAction::Restore => restore(cx, st, id),
                proto::WindowAction::Fullscreen => {
                    if let Some(w) = st.borrow_mut().win_mut(id) {
                        w.fullscreen = !w.fullscreen;
                    }
                    place_all(cx, st);
                }
                _ => {}
            }
        }
        Message::Chord(c) => match c.id {
            CHORD_SWITCH => switch_next(cx, st, sys),
            CHORD_CLOSE => {
                let f = st.borrow().focused;
                if let Some(f) = f {
                    request_close(cx, st, f);
                }
            }
            CHORD_LAUNCHER => open_launcher(cx, st, sys),
            _ => {}
        },
        Message::DisplayChanged(d) => {
            {
                let mut s = st.borrow_mut();
                s.display = d.display.clone();
                s.taskbar = 0; // new layers at the next turn
                let disp = s.display.clone();
                for w in &mut s.wins {
                    if w.maximized || disp.size_class == proto::SizeClass::Compact {
                        w.maximized = true;
                        w.rect = maximized_rect(&disp);
                    } else {
                        w.rect = clamp_rect(&disp, w.rect, w.min, w.max);
                    }
                }
            }
        }
        Message::NotificationPosted(n) => post_toast(cx, st, sys, n),
        Message::PermissionPrompt(p) => {
            let name = app_name(&**sys.borrow(), &p.app_id).unwrap_or_else(|| p.app_id.clone());
            st.borrow_mut().prompts.push_back(Prompt { req: p.req, app_id: p.app_id.clone(), app_name: name, permission: p.permission.clone() });
            show_prompt(cx, st);
        }
        Message::AppModes(m) => {
            {
                let mut s = st.borrow_mut();
                let pos = s.tray.iter().position(|t| t.pid == m.pid);
                match (pos, m.modes.is_empty()) {
                    (Some(i), true) => {
                        s.tray.remove(i);
                    }
                    (Some(i), false) => s.tray[i].modes = m.modes.clone(),
                    (None, false) => s.tray.push(TrayApp { pid: m.pid, app_id: m.app_id.clone(), modes: m.modes.clone() }),
                    (None, true) => {}
                }
            }
            invalidate_taskbar(cx, st);
        }
        Message::MediaSessionChanged(m) => {
            let session = MediaSession::from_wire(&m.info);
            {
                let mut s = st.borrow_mut();
                let pos = s.media.iter().position(|x| x.pid == m.pid);
                match (pos, session.state == MediaState::None) {
                    (Some(i), true) => {
                        s.media.remove(i);
                    }
                    (Some(i), false) => s.media[i].session = session,
                    (None, false) => s.media.insert(0, TrayMedia { pid: m.pid, app_id: m.app_id.clone(), session }),
                    (None, true) => {}
                }
            }
            if st.borrow().mixer_popup.is_some() {
                refresh_mixer_popup(cx, st, sys);
            }
        }
        Message::PopupDone(p) => {
            let mut s = st.borrow_mut();
            if s.mixer_popup == Some(p.surface) {
                s.mixer_popup = None;
            }
            if s.launcher == Some(p.surface) {
                s.launcher = None;
            }
        }
        _ => {}
    }
}

fn invalidate(cx: &mut Ctx, surface: u32) {
    let keep = cx.window();
    cx.set_window(surface);
    cx.invalidate();
    cx.set_window(keep);
}

fn invalidate_taskbar(cx: &mut Ctx, st: &Shared) {
    let tb = st.borrow().taskbar;
    if tb != 0 {
        invalidate(cx, tb);
    }
}

// ------------------------------------------------------------------ window management

/// Send what changed: visibility, placements, stacking and focus.
pub fn place_all(cx: &mut Ctx, st: &Shared) {
    let mut msgs = Vec::new();
    let mut frames = Vec::new();
    {
        let mut s = st.borrow_mut();
        let d = s.display.clone();
        let focused = s.focused;
        for w in &mut s.wins {
            let show = !w.minimized && !w.hidden;
            if w.shown != Some(show) {
                w.shown = Some(show);
                msgs.push(Message::SetVisible(proto::SetVisible { window: w.id, visible: show }));
                if let Some(f) = w.frame_wid {
                    msgs.push(Message::SetVisible(proto::SetVisible { window: f, visible: show && !w.fullscreen }));
                }
            }
            if !show {
                continue;
            }
            let mut states = 0;
            if focused == Some(w.id) {
                states |= proto::window_states::ACTIVATED;
            }
            let rect = if w.fullscreen {
                states |= proto::window_states::FULLSCREEN;
                (0, 0, d.w, d.h)
            } else {
                if w.maximized {
                    states |= proto::window_states::MAXIMIZED;
                }
                w.rect
            };
            if w.sent != Some((rect, states)) {
                w.sent = Some((rect, states));
                msgs.push(Message::Place(proto::Place {
                    window: w.id,
                    x: rect.0,
                    y: rect.1,
                    w: rect.2.max(1),
                    h: rect.3.max(1),
                    states,
                    transition: proto::Transition { ms: 0, easing: proto::Easing::EaseOut },
                }));
            }
            if let Some(f) = w.frame_wid {
                let fr = frame_rect(w.rect, w.maximized);
                if w.sent_frame != Some(fr) && !w.fullscreen {
                    w.sent_frame = Some(fr);
                    msgs.push(Message::Place(proto::Place {
                        window: f,
                        x: fr.0,
                        y: fr.1,
                        w: fr.2,
                        h: fr.3,
                        states: 0,
                        transition: proto::Transition { ms: 0, easing: proto::Easing::EaseOut },
                    }));
                    frames.push(w.frame_surface);
                }
            }
        }
        // Stacking: each frame right below its window.
        let mut order = Vec::new();
        for w in &s.wins {
            if let Some(f) = w.frame_wid {
                order.push(f);
            }
            order.push(w.id);
        }
        if order != s.stacked {
            s.stacked = order.clone();
            msgs.push(Message::Stack(proto::Stack { windows: order }));
        }
    }
    for m in msgs {
        cx.send(m);
    }
    for f in frames {
        invalidate(cx, f);
    }
}

/// Raise a window, give it the keyboard and un-minimize it.
pub fn focus(cx: &mut Ctx, st: &Shared, id: u32) {
    let prev = {
        let mut s = st.borrow_mut();
        let Some(i) = s.wins.iter().position(|w| w.id == id) else { return };
        let mut w = s.wins.remove(i);
        w.minimized = false;
        w.hidden = false;
        s.wins.push(w);
        let prev = s.focused.replace(id);
        s.mru.retain(|x| *x != id);
        s.mru.insert(0, id);
        prev
    };
    cx.send(Message::FocusWindow(proto::FocusWindow { window: id }));
    place_all(cx, st);
    let frames: Vec<u32> = st.borrow().wins.iter().filter(|w| w.id == id || Some(w.id) == prev).map(|w| w.frame_surface).collect();
    for f in frames {
        invalidate(cx, f);
    }
    invalidate_taskbar(cx, st);
}

pub fn minimize(cx: &mut Ctx, st: &Shared, id: u32) {
    let next = {
        let mut s = st.borrow_mut();
        let Some(w) = s.win_mut(id) else { return };
        w.minimized = true;
        if s.focused == Some(id) {
            s.focused = None;
            s.wins.iter().rev().find(|w| !w.minimized && !w.hidden).map(|w| w.id)
        } else {
            None
        }
    };
    match next {
        Some(n) => focus(cx, st, n),
        None => {
            place_all(cx, st);
            invalidate_taskbar(cx, st);
        }
    }
}

pub fn toggle_maximize(cx: &mut Ctx, st: &Shared, id: u32) {
    {
        let mut s = st.borrow_mut();
        let d = s.display.clone();
        let Some(w) = s.win_mut(id) else { return };
        if w.maximized {
            w.maximized = false;
            w.rect = w.saved.take().unwrap_or_else(|| initial_rect(&d, 0, w.min));
        } else {
            w.saved = Some(w.rect);
            w.maximized = true;
            w.rect = maximized_rect(&d);
        }
    }
    focus(cx, st, id);
}

pub fn restore(cx: &mut Ctx, st: &Shared, id: u32) {
    {
        let mut s = st.borrow_mut();
        let Some(w) = s.win_mut(id) else { return };
        w.minimized = false;
        w.fullscreen = false;
        if w.maximized {
            w.maximized = false;
            if let Some(r) = w.saved.take() {
                w.rect = r;
            }
        }
    }
    focus(cx, st, id);
}

// ------------------------------------------------------------------ frames

fn frame_view(vcx: &ecm_ui::ViewCtx, st: &Shared, sys: &SysRef, wid: u32) -> Element {
    let s = st.borrow();
    let t = vcx.theme;
    let Some(w) = s.win(wid) else { return stack() };
    let focused = s.focused == Some(wid);
    let (fw, fh) = (vcx.width, vcx.height);
    let bar = if focused { t.titlebar } else { t.titlebar_inactive };
    let fg = if focused { t.titlebar_text } else { t.text };
    let title: String = w.title.chars().take(80).collect();
    // The title bar between the resize edges and the buttons: dragging it moves
    // the frame and the app window (DragWindow, protocol v2), which the client
    // shows at once; the Place this desktop sends after the drag confirms it.
    // Maximized windows don't move. Invisible: it only carries the prop.
    let grip = BORDER + 2;
    let mut drag = vec![Prop::X(grip), Prop::Y(2), Prop::W((fw - TITLE_BUTTONS_W - grip).max(0)), Prop::H(TITLE_H - 2), Prop::Fill(0)];
    if !w.maximized {
        drag.push(Prop::DragWindow(wid));
    }
    let nodes = vec![
        SceneNode::new("edge", NodeKind::Rect, vec![Prop::X(0), Prop::Y(0), Prop::W(fw), Prop::H(fh), Prop::Fill(bar)]),
        SceneNode::new("drag", NodeKind::Rect, drag),
        SceneNode::new("icon", NodeKind::Image, {
            let mut p = vec![Prop::X(4), Prop::Y(1), Prop::W(16), Prop::H(16)];
            if let Some(r) = s.icons.get(icon_for(&w.app_id)) {
                p.push(Prop::Resource(*r));
            }
            p
        }),
        SceneNode::new(
            "title",
            NodeKind::Text,
            vec![Prop::X(24), Prop::Y((TITLE_H - 12) / 2), Prop::Text(title), Prop::Font(proto::Font::Ui), Prop::Size(12), Prop::Color(fg)],
        ),
    ];
    let (st1, sys1) = (st.clone(), sys.clone());
    let area = custom().id("frame").nodes(nodes).on_pointer(move |cx, e| {
        if let EventValue::Pointer(p) = &e.value {
            frame_pointer(cx, &st1, &sys1, wid, p);
        }
    });
    let mk = |text: &str, id: &str| button(text).id(id).style("flat").size(8).width(20);
    let (s1, s2, s3) = (st.clone(), st.clone(), st.clone());
    let buttons = row()
        .gap(1)
        .height(TITLE_H)
        .padding(1)
        .child(spacer())
        .child(mk("_", "min").on_click(move |cx, _| minimize(cx, &s1, wid)))
        .child(mk(if w.maximized { "=" } else { "[]" }, "max").on_click(move |cx, _| toggle_maximize(cx, &s2, wid)))
        .child(mk("x", "close").on_click(move |cx, _| request_close(cx, &s3, wid)));
    stack().child(area).child(column().gap(0).child(buttons).child(spacer()))
}

/// The title bar's close (button, menu, Alt+F4): an app with an active background mode is
/// hidden to the tray and keeps running; any other window gets `CloseWindow`.
pub fn request_close(cx: &mut Ctx, st: &Shared, wid: u32) {
    let action = st.borrow().close_action(wid);
    match action {
        CloseAction::Close => cx.send(Message::CloseWindow(proto::CloseWindow { window: wid })),
        CloseAction::HideToTray => hide_to_tray(cx, st, wid),
    }
}

/// Hide a window to the tray: not shown, not in the taskbar or Alt+Tab; its app keeps running.
pub fn hide_to_tray(cx: &mut Ctx, st: &Shared, wid: u32) {
    let next = {
        let mut s = st.borrow_mut();
        let Some(w) = s.win_mut(wid) else { return };
        w.hidden = true;
        s.mru.retain(|x| *x != wid);
        if s.focused == Some(wid) {
            s.focused = None;
            s.topmost_shown()
        } else {
            None
        }
    };
    match next {
        Some(n) => focus(cx, st, n),
        None => place_all(cx, st),
    }
    invalidate_taskbar(cx, st);
}

/// A click on an app's tray icon (or Open): its windows come back, the top one focused.
pub fn tray_open(cx: &mut Ctx, st: &Shared, app_id: &str) {
    let wins = st.borrow().windows_of(app_id);
    let Some(&top) = wins.last() else { return };
    for &w in &wins {
        if let Some(x) = st.borrow_mut().win_mut(w) {
            x.hidden = false;
            x.minimized = false;
        }
    }
    focus(cx, st, top);
}

/// Quit from the tray: `CloseWindow` on every window of the app (it then quits); an app
/// with no window is stopped through the app manager.
pub fn tray_quit(cx: &mut Ctx, st: &Shared, app_id: &str, pid: u32) {
    let wins = st.borrow().windows_of(app_id);
    if wins.is_empty() {
        host(st).borrow_mut().kill(pid);
        let mut s = st.borrow_mut();
        s.tray.retain(|t| t.app_id != app_id);
    }
    for w in wins {
        cx.send(Message::CloseWindow(proto::CloseWindow { window: w }));
    }
    invalidate_taskbar(cx, st);
}

/// Which edges a point of the frame grabs (0 = the title bar = move; None = nothing).
fn grab_edges(fw: i32, fh: i32, x: i32, y: i32, maximized: bool) -> Option<u8> {
    use proto::edges as e;
    if maximized {
        return if y < TITLE_H { Some(0) } else { None };
    }
    let g = BORDER + 2;
    let mut edges = 0;
    if x < g {
        edges |= e::LEFT;
    }
    if x >= fw - g {
        edges |= e::RIGHT;
    }
    if y >= fh - g {
        edges |= e::BOTTOM;
    }
    if y < 2 {
        edges |= e::TOP;
    }
    if edges != 0 {
        return Some(edges);
    }
    if y < TITLE_H {
        Some(0)
    } else {
        None
    }
}

fn frame_pointer(cx: &mut Ctx, st: &Shared, sys: &SysRef, wid: u32, p: &ecm_ui::PointerEv) {
    let Some(w) = st.borrow().win(wid).cloned() else { return };
    let fr = frame_rect(w.rect, w.maximized);
    let (ax, ay) = (fr.0 + p.x, fr.1 + p.y);
    match p.kind {
        proto::PointerKind::Down if p.button == proto::Button::Left => {
            focus(cx, st, wid);
            let Some(edges) = grab_edges(fr.2, fr.3, p.x, p.y, w.maximized) else { return };
            // Double click on the title bar: maximize / restore.
            let now = ms(sys);
            let double = edges == 0 && st.borrow().last_title_click.is_some_and(|(id, t)| id == wid && now.saturating_sub(t) <= 450);
            if edges == 0 {
                st.borrow_mut().last_title_click = Some((wid, now));
            }
            if double {
                st.borrow_mut().last_title_click = None;
                toggle_maximize(cx, st, wid);
                return;
            }
            if w.maximized {
                return; // maximized windows don't move
            }
            let st2 = st.clone();
            let outline = cx.core().create_layer(proto::Layer::Overlay, 0, 0, 0, 0, "outline", Content::View(Box::new(move |vcx| outline_view(vcx, &st2))));
            st.borrow_mut().drag = Some(Drag { win: wid, edges, start: (ax, ay), rect0: w.rect, rect: w.rect, outline });
        }
        proto::PointerKind::Down if p.button == proto::Button::Right && p.y < TITLE_H => {
            let st2 = st.clone();
            let items = vec!["Minimize", if w.maximized { "Restore" } else { "Maximize" }, "Close"];
            cx.popup_menu(p.x, p.y, items, move |cx, i| match i {
                0 => minimize(cx, &st2, wid),
                1 => toggle_maximize(cx, &st2, wid),
                _ => request_close(cx, &st2, wid),
            });
        }
        proto::PointerKind::Move => {
            let outline = {
                let mut s = st.borrow_mut();
                let d = s.display.clone();
                let Some(dr) = s.drag.as_mut().filter(|d| d.win == wid) else { return };
                let r = dragged(dr.rect0, dr.edges, ax - dr.start.0, ay - dr.start.1);
                dr.rect = clamp_rect(&d, r, w.min, w.max);
                dr.outline
            };
            invalidate(cx, outline);
        }
        proto::PointerKind::Up => {
            let done = st.borrow_mut().drag.take().filter(|d| d.win == wid);
            if let Some(d) = done {
                cx.core().close_window(d.outline);
                if let Some(w) = st.borrow_mut().win_mut(wid) {
                    w.rect = d.rect;
                }
                place_all(cx, st);
            }
        }
        _ => {}
    }
}

fn outline_view(vcx: &ecm_ui::ViewCtx, st: &Shared) -> Element {
    let s = st.borrow();
    let t = vcx.theme;
    let mut nodes = Vec::new();
    if let Some(d) = &s.drag {
        let f = frame_rect(d.rect, false);
        nodes.push(SceneNode::new(
            "outline",
            NodeKind::Rect,
            vec![
                Prop::X(f.0),
                Prop::Y(f.1),
                Prop::W(f.2),
                Prop::H(f.3),
                Prop::Fill(t.accent & 0xffff_ff00 | 0x30),
                Prop::BorderW(2),
                Prop::BorderColor(t.accent),
                Prop::Radius(2),
            ],
        ));
    }
    custom().id("outline").nodes(nodes).grow(1)
}

// ------------------------------------------------------------------ wallpaper and taskbar

fn wallpaper_view(vcx: &ecm_ui::ViewCtx, _st: &Shared) -> Element {
    let t = vcx.theme;
    let (w, h) = (vcx.width, vcx.height);
    let mut nodes = vec![SceneNode::new("wall", NodeKind::Rect, vec![Prop::W(w), Prop::H(h), Prop::Fill(t.wallpaper)])];
    // A gentle vertical shade: darker towards the taskbar.
    let bands = 12;
    for i in 0..bands {
        let y0 = h * i / bands;
        let y1 = h * (i + 1) / bands;
        nodes.push(SceneNode::new(
            format!("shade{i}"),
            NodeKind::Rect,
            vec![Prop::X(0), Prop::Y(y0), Prop::W(w), Prop::H(y1 - y0), Prop::Fill((i as u32) * 5)],
        ));
    }
    let name = "Terminal OS";
    let f = ecm_ui_proto::fonts::face(proto::Font::Ui, 16).unwrap();
    nodes.push(SceneNode::new(
        "name",
        NodeKind::Text,
        vec![
            Prop::X(w - f.text_width(name) - 12),
            Prop::Y(h - TASKBAR_H - 24),
            Prop::Text(name.into()),
            Prop::Font(proto::Font::Ui),
            Prop::Size(16),
            Prop::Color(t.text_muted & 0xffff_ff00 | 0xa0),
        ],
    ));
    custom().id("wall").nodes(nodes).grow(1)
}

/// Signal bars (0-4) as nodes, `n` lit.
fn bars_nodes(prefix: &str, n: u8, on: u32, off: u32) -> Vec<SceneNode> {
    (0..4)
        .map(|i| {
            let h = 3 + i * 3;
            SceneNode::new(
                format!("{prefix}{i}"),
                NodeKind::Rect,
                vec![Prop::X(i * 4), Prop::Y(14 - h), Prop::W(3), Prop::H(h), Prop::Fill(if (i as u8) < n { on } else { off })],
            )
        })
        .collect()
}

fn taskbar_view(vcx: &ecm_ui::ViewCtx, st: &Shared, sys: &SysRef) -> Element {
    let s = st.borrow();
    let t = vcx.theme;
    let bg = SceneNode::new("bg", NodeKind::Rect, vec![Prop::W(vcx.width), Prop::H(vcx.height), Prop::Fill(t.taskbar)]);
    let line = SceneNode::new("line", NodeKind::Rect, vec![Prop::W(vcx.width), Prop::H(1), Prop::Fill(t.border)]);
    let (st1, sys1) = (st.clone(), sys.clone());
    let mut r = row().gap(3).padding(2).align(Align::Center).child(button("Apps").id("apps").style("primary").on_click(move |cx, _| open_launcher(cx, &st1, &sys1)));
    let compact = vcx.class == proto::SizeClass::Compact;
    let bw = if compact { 60 } else { 110 };
    for w in s.wins.iter().filter(|w| !w.hidden) {
        let id = w.id;
        let st2 = st.clone();
        let active = s.focused == Some(id) && !w.minimized;
        let max = if compact { 8 } else { 16 };
        let mut title: String = w.title.chars().take(max).collect();
        if w.title.chars().count() > max {
            title.push_str("..");
        }
        let b = button(title).id(format!("task{id}")).width(bw).style(if active { "primary" } else { "flat" }).on_click(move |cx, _| {
            let (focused, minimized) = {
                let s = st2.borrow();
                (s.focused == Some(id), s.win(id).is_some_and(|w| w.minimized))
            };
            if focused && !minimized {
                minimize(cx, &st2, id);
            } else {
                focus(cx, &st2, id);
            }
        });
        r = r.child(b);
    }
    r = r.child(spacer());
    // The tray: apps running in the background, then the speaker (the Volume Mixer).
    for e in s.tray_entries() {
        let id = format!("tray:{}", e.app_id);
        let mut nodes = Vec::new();
        let mut img = vec![Prop::X(0), Prop::Y(0), Prop::W(16), Prop::H(16)];
        if let Some(res) = s.icons.get(icon_for(&e.app_id)) {
            img.push(Prop::Resource(*res));
        }
        nodes.push(SceneNode::new("icon", NodeKind::Image, img));
        if !e.modes.is_empty() {
            // A dot: it is doing something in the background now.
            nodes.push(SceneNode::new("dot", NodeKind::Rect, vec![Prop::X(11), Prop::Y(11), Prop::W(5), Prop::H(5), Prop::Fill(t.accent), Prop::Radius(2)]));
        }
        let (st2, app, pid, key) = (st.clone(), e.app_id.clone(), e.pid, id.clone());
        r = r.child(custom().id(id).width(16).height(16).nodes(nodes).on_pointer(move |cx, ev| {
            if let EventValue::Pointer(p) = &ev.value {
                if p.kind != proto::PointerKind::Down {
                    return;
                }
                if p.button == proto::Button::Right {
                    tray_menu(cx, &st2, &key, &app, pid);
                } else {
                    tray_open(cx, &st2, &app);
                }
            }
        }));
    }
    let (has_speaker, muted) = speaker_state(&s.mixer);
    if has_speaker {
        let (st2, sys2) = (st.clone(), sys.clone());
        r = r.child(custom().id("tray:mixer").width(16).height(16).nodes(speaker_nodes(t.text, if muted { t.danger } else { t.text }, muted)).on_pointer(move |cx, ev| {
            if let EventValue::Pointer(p) = &ev.value {
                if p.kind == proto::PointerKind::Down {
                    toggle_mixer_popup(cx, &st2, &sys2);
                }
            }
        }));
    }
    // Wi-Fi.
    let wifi_bars = if s.wifi.connected { sys::bars(s.wifi.signal_dbm.unwrap_or(-90)) } else { 0 };
    let wifi_on = if s.wifi.connected { t.text } else { t.text_muted };
    let mut wnodes = bars_nodes("w", wifi_bars, wifi_on, t.border);
    if !s.wifi.connected {
        wnodes.push(SceneNode::new("x", NodeKind::Text, vec![Prop::X(10), Prop::Y(0), Prop::Text("x".into()), Prop::Font(proto::Font::Ui), Prop::Size(8), Prop::Color(t.danger)]));
    }
    let _ = sys;
    r = r.child(custom().id("wifi").width(16).height(16).nodes(wnodes).on_pointer(|cx, e| {
        if let EventValue::Pointer(p) = &e.value {
            if p.kind == proto::PointerKind::Down {
                cx.launch("settings", &["wifi".to_string()]);
            }
        }
    }));
    // Cellular, when there is a modem.
    if let Some(c) = s.cells.first() {
        let mut cnodes = bars_nodes("c", c.bars, t.text, t.border);
        let tech = if c.tech.is_empty() { "LTE".to_string() } else { c.tech.to_uppercase() };
        cnodes.push(SceneNode::new("t", NodeKind::Text, vec![Prop::X(17), Prop::Y(4), Prop::Text(tech.chars().take(6).collect()), Prop::Font(proto::Font::Ui), Prop::Size(8), Prop::Color(t.text)]));
        r = r.child(custom().id("cell").width(46).height(16).nodes(cnodes).on_pointer(|cx, e| {
            if let EventValue::Pointer(p) = &e.value {
                if p.kind == proto::PointerKind::Down {
                    cx.launch("settings", &["cellular".to_string()]);
                }
            }
        }));
    }
    let clock = s.clock.map(|(h, m)| format!("{h:02}:{m:02}")).unwrap_or_else(|| "--:--".into());
    r = r.child(label(clock).id("clock").wrap(false));
    stack().child(custom().id("bg").nodes(vec![bg, line])).child(r)
}

// ------------------------------------------------------------------ tray and Volume Mixer

/// The tray's speaker: a box, a cone, and waves (or an X when muted).
fn speaker_nodes(fg: u32, accent: u32, muted: bool) -> Vec<SceneNode> {
    let line = |key: &str, pts: &[(i32, i32)], w: u16, c: u32| SceneNode::new(key, NodeKind::Line, vec![Prop::Points(pts.iter().map(|&(x, y)| proto::Point { x, y }).collect()), Prop::Width(w), Prop::Color(c)]);
    let mut n = vec![
        SceneNode::new("box", NodeKind::Rect, vec![Prop::X(1), Prop::Y(6), Prop::W(4), Prop::H(5), Prop::Fill(fg)]),
        line("cone", &[(5, 6), (9, 2), (9, 14), (5, 10)], 2, fg),
    ];
    if muted {
        n.push(line("x1", &[(11, 5), (15, 11)], 2, accent));
        n.push(line("x2", &[(15, 5), (11, 11)], 2, accent));
    } else {
        n.push(line("w1", &[(11, 5), (13, 8), (11, 11)], 1, fg));
        n.push(line("w2", &[(13, 3), (16, 8), (13, 13)], 1, fg));
    }
    n
}

/// Right-click on a tray icon: Open, Quit.
fn tray_menu(cx: &mut Ctx, st: &Shared, key: &str, app_id: &str, pid: u32) {
    let tb = st.borrow().taskbar;
    let x = cx.core().window(tb).and_then(|w| w.rect_of(key)).map_or(0, |r| r.x);
    let (st2, app) = (st.clone(), app_id.to_string());
    let el = ecm_ui::element::menu(["Open", "Quit"]).id("tray_menu").on_select(move |cx, e| {
        if let EventValue::Index(i) = e.value {
            if i == 0 {
                tray_open(cx, &st2, &app);
            } else {
                tray_quit(cx, &st2, &app, pid);
            }
        }
    });
    let theme = cx.core().theme.clone();
    let (w, h) = ecm_ui::window::measure(&el, &theme, proto::SizeClass::Compact, 200);
    let w = w.max(90);
    let dw = st.borrow().display.w;
    let s = cx.core().create_popup(tb, x.min(dw - w).max(0), -h - 2, w, h, Content::Static(el));
    if let Some(win) = cx.core().window_mut(s) {
        win.close_on_select = true;
    }
}

/// What the Volume Mixer popup is laid out for (it is made again when this changes).
fn mixer_shape(s: &State) -> Vec<String> {
    let mut v: Vec<String> = s.media.iter().map(|m| format!("np{}", m.pid)).collect();
    v.extend(s.mixer.outputs.iter().map(|o| format!("out:{}", o.side)));
    v.extend(s.mixer.apps.iter().map(|a| format!("app:{}", a.app)));
    v.extend(s.mixer.inputs.iter().map(|i| format!("in:{}", i.side)));
    v
}

/// The speaker icon: open the Volume Mixer popup, or close it.
pub fn toggle_mixer_popup(cx: &mut Ctx, st: &Shared, sys: &SysRef) {
    let open = st.borrow().mixer_popup;
    if let Some(p) = open {
        st.borrow_mut().mixer_popup = None;
        if cx.core().window(p).is_some() {
            cx.core().close_window(p);
            return;
        }
    }
    refresh_mixer(sys, st, true);
    open_mixer_popup(cx, st, sys);
}

fn open_mixer_popup(cx: &mut Ctx, st: &Shared, sys: &SysRef) {
    let theme = cx.core().theme.clone();
    let el = mixer_content(st, sys, &theme);
    let (_, h) = ecm_ui::window::measure(&el, &theme, proto::SizeClass::Compact, MIXER_W);
    let (tb, d) = {
        let s = st.borrow();
        (s.taskbar, s.display.clone())
    };
    let w = MIXER_W.min(d.w - 8);
    let h = h.min(d.h - TASKBAR_H - 8);
    let (st2, sys2) = (st.clone(), sys.clone());
    let p = cx.core().create_popup(tb, d.w - w - 4, -h - 2, w, h, Content::View(Box::new(move |vcx| mixer_view(vcx, &st2, &sys2))));
    let mut s = st.borrow_mut();
    s.mixer_popup = Some(p);
    s.mixer_shape = mixer_shape(&s);
}

/// New strips or sessions: the popup is made again at its new size; otherwise it is redrawn.
fn refresh_mixer_popup(cx: &mut Ctx, st: &Shared, sys: &SysRef) {
    let (popup, same) = {
        let s = st.borrow();
        (s.mixer_popup, s.mixer_shape == mixer_shape(&s))
    };
    let Some(p) = popup else { return };
    if cx.core().window(p).is_none() {
        st.borrow_mut().mixer_popup = None;
        return;
    }
    if same {
        invalidate(cx, p);
    } else {
        cx.core().close_window(p);
        open_mixer_popup(cx, st, sys);
    }
}

/// One mixer command; the strips change at once (`edit`) so the popup doesn't jump back.
fn mixer_set(st: &Shared, cmd: String, edit: impl FnOnce(&mut Strips)) {
    edit(&mut st.borrow_mut().mixer);
    let sys = st.borrow().sys.clone();
    let _ = sys.borrow_mut().mixer_command(&cmd);
}

/// The meters' level colour.
const METER: u32 = 0x5ad17aff;

/// A level meter, `fill` 0-100, `w` lu wide.
fn meter(id: String, w: i32, fill: u32, bg: u32, fg: u32) -> Element {
    let mut n = vec![SceneNode::new("bg", NodeKind::Rect, vec![Prop::W(w), Prop::H(3), Prop::Fill(bg), Prop::Radius(1)])];
    if fill > 0 {
        n.push(SceneNode::new("fill", NodeKind::Rect, vec![Prop::W((w * fill.min(100) as i32 / 100).max(1)), Prop::H(3), Prop::Fill(fg), Prop::Radius(1)]));
    }
    custom().id(id).width(w).height(3).nodes(n)
}

fn badge(id: String, text: &str, fill: u32, fg: u32) -> Element {
    let f = ecm_ui_proto::fonts::face(proto::Font::Ui, 8).unwrap();
    let w = f.text_width(text) + 6;
    custom().id(id).width(w).height(11).nodes(vec![
        SceneNode::new("bg", NodeKind::Rect, vec![Prop::W(w), Prop::H(11), Prop::Fill(fill), Prop::Radius(3)]),
        SceneNode::new("t", NodeKind::Text, vec![Prop::X(3), Prop::Y(1), Prop::Text(text.into()), Prop::Font(proto::Font::Ui), Prop::Size(8), Prop::Color(fg)]),
    ])
}

/// A name for an app that isn't installed as one: `dev.ecm.discord` -> "Discord"; a
/// program's strip (`aplay`) keeps its name.
fn short_name(id: &str) -> String {
    let last = id.rsplit('.').next().unwrap_or(id);
    if !id.contains('.') || last.is_empty() {
        return id.to_string();
    }
    let mut c = last.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn clock_of(ms: u32) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The Volume Mixer popup: Now Playing, a master strip per speaker, a strip per app, the microphones.
fn mixer_content(st: &Shared, sys: &SysRef, t: &Theme) -> Element {
    let s = st.borrow();
    let inner = MIXER_W - 16;
    let mut c = column().padding(8).gap(3);
    let name_of = |id: &str| app_name(&**sys.borrow(), id).unwrap_or_else(|| short_name(id));
    // Now Playing.
    for m in &s.media {
        let ses = &m.session;
        let pid = m.pid;
        let title = if ses.artist.is_empty() { ses.title.clone() } else { format!("{} - {}", ses.title, ses.artist) };
        c = c.child(label(format!("Now Playing: {}", name_of(&m.app_id))).style("caption").wrap(false).id(format!("np{pid}:app")));
        c = c.child(label(title).wrap(false).id(format!("np{pid}:title")));
        let fill = if ses.duration_ms > 0 { (ses.position_ms.min(ses.duration_ms) as u64 * 100 / ses.duration_ms as u64) as u32 } else { 0 };
        c = c.child(meter(format!("np{pid}:bar"), inner, fill, t.border, t.accent));
        let offers = |a: &str| ses.actions.is_empty() || ses.actions.iter().any(|x| x == a);
        let toggle = if matches!(ses.state, MediaState::Playing | MediaState::Buffering) { "pause" } else { "play" };
        let mut r = row().gap(4).align(Align::Center);
        for (text, action, key) in [("|<", "previous", "prev"), (if toggle == "pause" { "||" } else { ">" }, toggle, "play"), (">|", "next", "next")] {
            r = r.child(button(text).id(format!("np{pid}:{key}")).style("flat").size(8).width(28).enabled(offers(action)).on_click(move |cx, _| {
                cx.send(Message::MediaControl(proto::MediaControl { pid, action: action.into(), position_ms: 0 }));
            }));
        }
        r = r.child(spacer());
        if ses.duration_ms > 0 {
            r = r.child(label(format!("{} / {}", clock_of(ses.position_ms), clock_of(ses.duration_ms))).style("caption").wrap(false));
        }
        c = c.child(r);
    }
    c = c.child(label("Volume Mixer").style("title").id("mixer:title"));
    let outputs = s.mixer.outputs.clone();
    let several = outputs.len() > 1;
    // A master strip per speaker.
    for o in &outputs {
        let side = o.side.clone();
        let target = if several { format!("master.{side}") } else { "master".to_string() };
        let label_text = if o.label.is_empty() { format!("Speaker ({side})") } else { o.label.clone() };
        let (st1, st2, tg1, tg2, s1, s2) = (st.clone(), st.clone(), target.clone(), target.clone(), side.clone(), side.clone());
        c = c.child(row().gap(4).align(Align::Center).child(label(label_text).wrap(false).id(format!("out:{side}:name"))).child(spacer()).child(label(if o.mute { "muted".to_string() } else { format!("{}%", o.volume) }).style("caption").wrap(false)));
        c = c.child(
            row()
                .gap(4)
                .align(Align::Center)
                .child(slider(0, 100).value(o.volume.min(100) as i32).id(format!("vol:out:{side}")).grow(1).on_change(move |cx, e| {
                    if let EventValue::Int(v) = e.value {
                        let v = v.clamp(0, 100) as u32;
                        let s1 = s1.clone();
                        mixer_set(&st1, format!("set {tg1} {v}"), move |m| m.outputs.iter_mut().filter(|o| o.side == s1).for_each(|o| o.volume = v));
                        cx.invalidate();
                    }
                }))
                .child(checkbox("Mute").checked(o.mute).id(format!("mute:out:{side}")).on_change(move |cx, _| {
                    let s2 = s2.clone();
                    mixer_set(&st2, format!("mute {tg2} toggle"), move |m| m.outputs.iter_mut().filter(|o| o.side == s2).for_each(|o| o.mute = !o.mute));
                    cx.invalidate();
                })),
        );
        c = c.child(meter(format!("out:{side}:meter"), inner, if o.mute { 0 } else { mixer::meter_percent(o.peak) }, t.track, METER));
    }
    // A strip per app.
    for a in &s.mixer.apps {
        let id = a.app.clone();
        let mut head = row().gap(4).align(Align::Center).child(label(name_of(&id)).wrap(false).id(format!("app:{id}:name")));
        if a.ducked {
            head = head.child(badge(format!("app:{id}:ducked"), "ducked", 0xe8b339ff, 0x0b0f14ff));
        }
        head = head.child(spacer()).child(label(if a.mute { "muted".to_string() } else { format!("{}%", a.volume) }).style("caption").wrap(false).id(format!("app:{id}:pct")));
        c = c.child(head);
        let (st1, st2, i1, i2) = (st.clone(), st.clone(), id.clone(), id.clone());
        let mut controls = row()
            .gap(4)
            .align(Align::Center)
            .child(slider(0, mixer::MAX_APP_PERCENT as i32).value(a.volume.min(mixer::MAX_APP_PERCENT) as i32).id(format!("vol:app:{id}")).grow(1).on_change(move |cx, e| {
                if let EventValue::Int(v) = e.value {
                    let v = v.clamp(0, mixer::MAX_APP_PERCENT as i32) as u32;
                    let i1b = i1.clone();
                    mixer_set(&st1, format!("set {i1} {v}"), move |m| m.apps.iter_mut().filter(|a| a.app == i1b).for_each(|a| a.volume = v));
                    cx.invalidate();
                }
            }))
            .child(checkbox("Mute").checked(a.mute).id(format!("mute:app:{id}")).on_change(move |cx, _| {
                let i2b = i2.clone();
                mixer_set(&st2, format!("mute {i2} toggle"), move |m| m.apps.iter_mut().filter(|a| a.app == i2b).for_each(|a| a.mute = !a.mute));
                cx.invalidate();
            }));
        if several {
            // The route picker: which speaker it plays on.
            let current = a.route.as_ref().and_then(|r| outputs.iter().find(|o| &o.side == r)).map_or("Default".to_string(), |o| o.label.clone());
            let (st3, i3, outs) = (st.clone(), id.clone(), outputs.clone());
            let key = format!("route:{id}");
            let key2 = key.clone();
            controls = controls.child(button(current).id(key).size(8).on_click(move |cx, _| {
                let me = cx.window();
                let anchor = cx.core().window(me).and_then(|w| w.rect_of(&key2)).unwrap_or_default();
                let mut items = vec!["Default".to_string()];
                items.extend(outs.iter().map(|o| if o.label.is_empty() { o.side.clone() } else { o.label.clone() }));
                let (st4, i4, outs2) = (st3.clone(), i3.clone(), outs.clone());
                cx.popup_menu_below(anchor, items, move |cx, i| {
                    let side = if i == 0 { None } else { outs2.get(i - 1).map(|o| o.side.clone()) };
                    let i4b = i4.clone();
                    let s2 = side.clone();
                    mixer_set(&st4, format!("route {i4} {}", side.as_deref().unwrap_or("-")), move |m| m.apps.iter_mut().filter(|a| a.app == i4b).for_each(|a| a.route = s2.clone()));
                    let p = st4.borrow().mixer_popup;
                    if let Some(p) = p {
                        invalidate(cx, p);
                    }
                });
            }));
        }
        c = c.child(controls);
        c = c.child(meter(format!("app:{id}:meter"), inner, if a.mute { 0 } else { mixer::meter_percent(a.peak) }, t.track, METER));
    }
    if s.mixer.apps.is_empty() {
        c = c.child(label("No app is playing sound").style("caption").id("mixer:none"));
    }
    // The microphones.
    let many_in = s.mixer.inputs.len() > 1;
    for i in &s.mixer.inputs {
        let side = i.side.clone();
        let name = if many_in { format!("Microphone ({side})") } else { "Microphone".to_string() };
        c = c.child(row().gap(4).child(label(name).wrap(false).id(format!("in:{side}:name"))).child(spacer()).child(label(format!("{}%", i.gain)).style("caption").wrap(false)));
        let (st1, s1) = (st.clone(), side.clone());
        c = c.child(slider(0, 100).value(i.gain.min(100) as i32).id(format!("gain:{side}")).on_change(move |cx, e| {
            if let EventValue::Int(v) = e.value {
                let v = v.clamp(0, 100) as u32;
                let s1b = s1.clone();
                mixer_set(&st1, format!("gain {s1} {v}"), move |m| m.inputs.iter_mut().filter(|i| i.side == s1b).for_each(|i| i.gain = v));
                cx.invalidate();
            }
        }));
        let fill = (((i.level.clamp(-60, 0) + 60) * 100) / 60) as u32;
        c = c.child(meter(format!("in:{side}:meter"), inner, fill, t.track, METER));
    }
    c
}

/// The popup's view: the strips on a card filling the popup.
fn mixer_view(vcx: &ecm_ui::ViewCtx, st: &Shared, sys: &SysRef) -> Element {
    let t = vcx.theme;
    let card = SceneNode::new(
        "card",
        NodeKind::Rect,
        vec![Prop::W(vcx.width), Prop::H(vcx.height), Prop::Fill(t.surface), Prop::BorderW(1), Prop::BorderColor(t.border), Prop::Radius(4)],
    );
    stack().child(custom().id("mixer:card").nodes(vec![card])).child(mixer_content(st, sys, t))
}

// ------------------------------------------------------------------ launcher

fn open_launcher(cx: &mut Ctx, st: &Shared, sys: &SysRef) {
    // What is installed now (the App Store may have added an app since the desktop started).
    let fresh = launch_items(&**sys.borrow());
    st.borrow_mut().launch_items = fresh;
    let (tb, items, icons, open) = {
        let s = st.borrow();
        (s.taskbar, s.launch_items.clone(), s.icons.clone(), s.launcher)
    };
    if let Some(p) = open {
        if cx.core().window(p).is_some() {
            cx.core().close_window(p);
            st.borrow_mut().launcher = None;
            return;
        }
    }
    let rows: Vec<Element> = items
        .iter()
        .map(|it| {
            let mut r = row().gap(6).align(Align::Center);
            if let Some(res) = icons.get(&it.icon) {
                r = r.child(image(*res, 16, 16));
            }
            r.child(label(it.label.clone()).wrap(false))
        })
        .collect();
    let st2 = st.clone();
    let el = column().padding(4).gap(0).child(list().id("apps").children(rows).on_select(move |cx, e| {
        if let EventValue::Index(i) = e.value {
            let it = st2.borrow().launch_items.get(i).cloned();
            st2.borrow_mut().launcher = None;
            if let Some(it) = it {
                cx.launch(&it.program, &it.args);
            }
        }
    }));
    let theme = cx.core().theme.clone();
    let (w, h) = ecm_ui::window::measure(&el, &theme, proto::SizeClass::Compact, 400);
    let w = w.max(150);
    let s = cx.core().create_popup(tb, 2, -h - 2, w, h, Content::Static(el));
    if let Some(win) = cx.core().window_mut(s) {
        win.close_on_select = true;
    }
    st.borrow_mut().launcher = Some(s);
}

// ------------------------------------------------------------------ Alt+Tab

fn switch_next(cx: &mut Ctx, st: &Shared, sys: &SysRef) {
    let now = ms(sys);
    let existing = st.borrow().switcher.as_ref().map(|s| s.surface);
    match existing {
        Some(surface) => {
            {
                let mut s = st.borrow_mut();
                let sw = s.switcher.as_mut().unwrap();
                if !sw.order.is_empty() {
                    sw.index = (sw.index + 1) % sw.order.len();
                }
                sw.last = now;
            }
            invalidate(cx, surface);
        }
        None => {
            let order: Vec<u32> = {
                let s = st.borrow();
                let mut o: Vec<u32> = s.mru.iter().copied().filter(|id| s.win(*id).is_some_and(|w| !w.hidden)).collect();
                for w in s.wins.iter().rev() {
                    if !o.contains(&w.id) && !w.hidden {
                        o.push(w.id);
                    }
                }
                o
            };
            if order.len() < 2 {
                if let Some(&only) = order.first() {
                    focus(cx, st, only);
                }
                return;
            }
            let (d, n) = (st.borrow().display.clone(), order.len() as i32);
            let w = (n * 104 + 16).min(d.w - 16);
            let h = 64;
            let st2 = st.clone();
            let surface = cx.core().create_layer(
                proto::Layer::Overlay,
                (d.w - w) / 2,
                (d.h - TASKBAR_H - h) / 2,
                w,
                h,
                "switcher",
                Content::View(Box::new(move |vcx| switcher_view(vcx, &st2))),
            );
            st.borrow_mut().switcher = Some(Switcher { order, index: 1, surface, last: now });
        }
    }
}

fn switcher_view(vcx: &ecm_ui::ViewCtx, st: &Shared) -> Element {
    let s = st.borrow();
    let t = vcx.theme;
    let Some(sw) = &s.switcher else { return stack() };
    let card = SceneNode::new(
        "card",
        NodeKind::Rect,
        vec![Prop::W(vcx.width), Prop::H(vcx.height), Prop::Fill(t.surface), Prop::Radius(8), Prop::BorderW(1), Prop::BorderColor(t.border)],
    );
    let mut r = row().gap(4).padding(8).align(Align::Center);
    for (i, id) in sw.order.iter().enumerate() {
        let Some(w) = s.win(*id) else { continue };
        let mut title: String = w.title.chars().take(14).collect();
        if w.title.chars().count() > 14 {
            title.push_str("..");
        }
        let mut c = column().gap(2).width(100).align(Align::Center);
        if let Some(res) = s.icons.get(icon_for(&w.app_id)) {
            c = c.child(image(*res, 16, 16));
        }
        c = c.child(label(title).wrap(false).style(if i == sw.index { "" } else { "caption" }));
        let mut cell = stack().width(100).height(44);
        if i == sw.index {
            cell = cell.child(custom().nodes(vec![SceneNode::new(
                "sel",
                NodeKind::Rect,
                vec![Prop::W(100), Prop::H(44), Prop::Fill(t.surface_alt), Prop::Radius(4), Prop::BorderW(2), Prop::BorderColor(t.accent)],
            )]));
        }
        r = r.child(cell.child(c.padding(4)));
    }
    stack().child(custom().nodes(vec![card])).child(r)
}

fn finish_switch(cx: &mut Ctx, st: &Shared) {
    let sw = st.borrow_mut().switcher.take();
    if let Some(sw) = sw {
        cx.core().close_window(sw.surface);
        if let Some(&id) = sw.order.get(sw.index) {
            focus(cx, st, id);
        }
    }
}

// ------------------------------------------------------------------ notifications

fn post_toast(cx: &mut Ctx, st: &Shared, sys: &SysRef, n: &proto::NotificationPosted) {
    let now = ms(sys);
    // Same app and tag: replace.
    let old: Vec<u32> = st.borrow().toasts.iter().filter(|t| t.app_id == n.app_id && !n.tag.is_empty() && t.tag == n.tag).map(|t| t.nid).collect();
    for nid in old {
        remove_toast(cx, st, nid);
    }
    let toast = Toast {
        nid: n.nid,
        app_id: n.app_id.clone(),
        tag: n.tag.clone(),
        height: 0,
        title: n.title.clone(),
        body: n.body.clone(),
        actions: n.actions.clone(),
        surface: 0,
        until: now + TOAST_MS,
    };
    let theme = cx.core().theme.clone();
    let el = toast_view_static(&toast, st, &theme);
    let tw = 240;
    let (_, h) = ecm_ui::window::measure(&el, &theme, proto::SizeClass::Compact, tw);
    let (d, y) = {
        let s = st.borrow();
        let y = 8 + s.toasts.iter().map(|t| t.height + 6).sum::<i32>();
        (s.display.clone(), y)
    };
    let (st2, nid) = (st.clone(), n.nid);
    let surface = cx.core().create_layer(
        proto::Layer::Overlay,
        d.w - tw - 8,
        y,
        tw,
        h,
        "toast",
        Content::View(Box::new(move |vcx| {
            let s = st2.borrow();
            match s.toasts.iter().find(|t| t.nid == nid) {
                Some(t) => toast_view_static(t, &st2, vcx.theme),
                None => stack(),
            }
        })),
    );
    let mut t = toast;
    t.surface = surface;
    t.height = h;
    st.borrow_mut().toasts.push(t);
}

fn toast_view_static(t: &Toast, st: &Shared, theme: &Theme) -> Element {
    let nid = t.nid;
    let (s1, s2) = (st.clone(), st.clone());
    let mut c = column().gap(4).padding(8).child(
        row()
            .gap(4)
            .child(label(t.title.clone()).id("title").wrap(false).on_click(move |cx, _| invoke(cx, &s1, nid, "")))
            .child(spacer())
            .child(button("x").id("dismiss").style("flat").size(8).on_click(move |cx, _| remove_toast(cx, &s2, nid))),
    );
    if !t.body.is_empty() {
        let s3 = st.clone();
        c = c.child(label(t.body.clone()).id("body").style("caption").on_click(move |cx, _| invoke(cx, &s3, nid, "")));
    }
    if !t.actions.is_empty() {
        let mut r = row().gap(4).child(spacer());
        for (i, a) in t.actions.iter().enumerate() {
            let (s4, act) = (st.clone(), a.clone());
            r = r.child(button(a.clone()).id(format!("act{i}")).on_click(move |cx, _| invoke(cx, &s4, nid, &act)));
        }
        c = c.child(r);
    }
    stack().child(custom().id("card").nodes(vec![SceneNode::new(
        "card",
        NodeKind::Rect,
        vec![Prop::W(240), Prop::H(t.height.max(1)), Prop::Fill(theme.surface), Prop::BorderW(1), Prop::BorderColor(theme.focus), Prop::Radius(4)],
    )])).child(c)
}

fn invoke(cx: &mut Ctx, st: &Shared, nid: u32, action: &str) {
    cx.send(Message::NotificationInvoke(proto::NotificationInvoke { nid, action: action.chars().take(64).collect() }));
    remove_toast(cx, st, nid);
}

fn remove_toast(cx: &mut Ctx, st: &Shared, nid: u32) {
    let gone = {
        let mut s = st.borrow_mut();
        let pos = s.toasts.iter().position(|t| t.nid == nid);
        pos.map(|i| s.toasts.remove(i))
    };
    if let Some(t) = gone {
        cx.core().close_window(t.surface);
    }
}

// ------------------------------------------------------------------ permission prompts

fn show_prompt(cx: &mut Ctx, st: &Shared) {
    if st.borrow().prompt_surface.is_some() {
        return;
    }
    let Some(p) = st.borrow().prompts.front().cloned() else { return };
    let d = st.borrow().display.clone();
    let el = prompt_view(&p, st);
    let theme = cx.core().theme.clone();
    let w = 300.min(d.w - 16);
    let (_, h) = ecm_ui::window::measure(&el, &theme, proto::SizeClass::Compact, w);
    let _ = el;
    let (st2, p2) = (st.clone(), p.clone());
    let surface = cx.core().create_layer(
        proto::Layer::Overlay,
        (d.w - w) / 2,
        ((d.h - TASKBAR_H - h) / 2).max(0),
        w,
        h,
        "permission",
        Content::View(Box::new(move |vcx| {
            // A card: the dialog stands out from the windows under it.
            let t = vcx.theme;
            let card = SceneNode::new(
                "card",
                NodeKind::Rect,
                vec![Prop::W(vcx.width), Prop::H(vcx.height), Prop::Fill(t.surface), Prop::Radius(6), Prop::BorderW(2), Prop::BorderColor(t.accent)],
            );
            stack().child(custom().id("card").nodes(vec![card])).child(prompt_view(&p2, &st2))
        })),
    );
    st.borrow_mut().prompt_surface = Some(surface);
}

fn prompt_view(p: &Prompt, st: &Shared) -> Element {
    let req = p.req;
    let (s1, s2) = (st.clone(), st.clone());
    if p.permission.starts_with("background:") {
        // The first-time background prompt: Allow / Don't allow, remembered either way.
        return column()
            .gap(6)
            .padding(10)
            .child(label(prompt_question(&p.app_name, &p.permission)).style("title").id("question"))
            .child(label("It keeps working while its window is closed; quit it from the tray.").style("caption"))
            .child(
                row()
                    .gap(6)
                    .child(spacer())
                    .child(button("Don't allow").id("deny").on_click(move |cx, _| answer_as(cx, &s1, req, false, true)))
                    .child(button("Allow").id("allow").style("primary").on_click(move |cx, _| answer_as(cx, &s2, req, true, true))),
            );
    }
    let what = match p.permission.split(':').next().unwrap_or("") {
        "network" => "the network (sockets, DNS, HTTP)",
        "radio" => "the radio (raw Wi-Fi frames, SDR, transmitting)",
        "cellular" => "the cellular modem",
        "files" => "your files",
        "controller" => "Wireless Controllers",
        "microphone" => "the microphone",
        "location" => "your location",
        "notifications" => "notifications",
        "background" => "running in the background",
        "peripherals" => "peripherals",
        _ => "",
    };
    column()
        .gap(6)
        .padding(10)
        .child(label(prompt_question(&p.app_name, &p.permission)).style("title").id("question"))
        .child(label(if what.is_empty() { String::from("The app asked for this permission.") } else { format!("It wants to use {what}.") }).style("caption"))
        .child(ecm_ui::element::checkbox("Remember my choice").id("remember"))
        .child(
            row()
                .gap(6)
                .child(spacer())
                .child(button("Deny").id("deny").on_click(move |cx, _| answer(cx, &s1, req, false)))
                .child(button("Allow").id("allow").style("primary").on_click(move |cx, _| answer(cx, &s2, req, true))),
        )
}

fn answer(cx: &mut Ctx, st: &Shared, req: u32, allow: bool) {
    let remember = cx.checked("remember");
    answer_as(cx, st, req, allow, remember);
}

fn answer_as(cx: &mut Ctx, st: &Shared, req: u32, allow: bool, remember: bool) {
    cx.send(Message::AnswerPermission(proto::AnswerPermission { req, allow, remember }));
    let surface = {
        let mut s = st.borrow_mut();
        s.prompts.retain(|p| p.req != req);
        s.prompt_surface.take()
    };
    if let Some(sf) = surface {
        cx.core().close_window(sf);
    }
    show_prompt(cx, st);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disp(w: i32, h: i32) -> proto::DisplayInfo {
        proto::DisplayInfo { w, h, scale: 1, size_class: proto::SizeClass::for_width(w), tier: 2, device: proto::Device::Terminal }
    }

    #[test]
    fn geometry() {
        let d = disp(640, 400);
        assert_eq!(work_area(&d), (0, 0, 640, 376));
        let r = initial_rect(&d, 0, (0, 0));
        let f = frame_rect(r, false);
        assert!(f.0 >= 0 && f.1 >= 0 && f.0 + f.2 <= 640 && f.1 + f.3 <= 376, "{f:?} inside the work area");
        assert_ne!(initial_rect(&d, 1, (0, 0)), r, "cascades");
        assert_eq!(frame_rect(maximized_rect(&d), true), (0, 0, 640, 376));
        assert_eq!(dragged((10, 30, 100, 80), 0, 5, -3), (15, 27, 100, 80));
        assert_eq!(dragged((10, 30, 100, 80), proto::edges::LEFT | proto::edges::BOTTOM, 5, 7), (15, 30, 95, 87));
        let c = clamp_rect(&d, (-500, -40, 10, 10), (120, 60), (0, 0));
        assert!(c.0 + c.2 >= 40 && c.1 >= TITLE_H && c.2 >= 120 && c.3 >= 60);
        assert_eq!(grab_edges(200, 100, 100, 5, false), Some(0));
        assert_eq!(grab_edges(200, 100, 1, 50, false), Some(proto::edges::LEFT));
        assert_eq!(grab_edges(200, 100, 199, 99, false), Some(proto::edges::RIGHT | proto::edges::BOTTOM));
        assert_eq!(grab_edges(200, 100, 100, 50, false), None);
    }

    fn state() -> State {
        let sys: SysRef = Rc::new(RefCell::new(Box::new(sys::MockSys::new(std::env::temp_dir()))));
        State {
            display: disp(640, 400),
            own_app_id: "dev.ecm.desktop".into(),
            wins: Vec::new(),
            focused: None,
            mru: Vec::new(),
            opened: 0,
            drag: None,
            toasts: Vec::new(),
            prompts: VecDeque::new(),
            prompt_surface: None,
            switcher: None,
            launcher: None,
            launch_items: Vec::new(),
            wifi: WifiState::default(),
            cells: Vec::new(),
            clock: None,
            theme_name: String::new(),
            taskbar: 0,
            wallpaper: 0,
            icons: HashMap::new(),
            last_status: None,
            last_theme_poll: None,
            last_title_click: None,
            stacked: Vec::new(),
            tray: Vec::new(),
            media: Vec::new(),
            mixer: Strips::default(),
            last_mixer: None,
            mixer_popup: None,
            mixer_shape: Vec::new(),
            host: Rc::new(RefCell::new(Box::new(FakeHost::default()))),
            sys,
        }
    }

    fn win(id: u32, pid: u32, app: &str) -> Win {
        Win {
            id,
            pid,
            app_id: app.into(),
            title: app.into(),
            min: (0, 0),
            max: (0, 0),
            frame_surface: 100 + id,
            frame_wid: None,
            rect: (10, 30, 200, 100),
            saved: None,
            minimized: false,
            hidden: false,
            maximized: false,
            fullscreen: false,
            sent: None,
            sent_frame: None,
            shown: None,
        }
    }

    #[test]
    fn close_goes_to_the_tray_only_while_the_app_has_a_mode() {
        let mut s = state();
        s.wins.push(win(1, 41, "dev.ecm.discord"));
        s.wins.push(win(2, 42, "terminal"));
        assert_eq!(s.close_action(1), CloseAction::Close, "no mode: close");
        assert!(s.tray_entries().is_empty());
        // A mode, matched by pid ...
        s.tray.push(TrayApp { pid: 41, app_id: "dev.ecm.discord".into(), modes: vec!["call".into()] });
        assert_eq!(s.close_action(1), CloseAction::HideToTray);
        assert_eq!(s.close_action(2), CloseAction::Close, "other apps still close");
        // ... or by app id (a window of another process of the app).
        s.wins.push(win(3, 77, "dev.ecm.discord"));
        assert_eq!(s.close_action(3), CloseAction::HideToTray);
        assert_eq!(s.tray_entries(), vec![TrayEntry { app_id: "dev.ecm.discord".into(), pid: 41, modes: vec!["call".into()], hidden: vec![] }]);
        // Hidden, then the mode ends: still in the tray (to bring it back or quit it), and close closes.
        s.wins[0].hidden = true;
        s.tray.clear();
        assert_eq!(s.tray_entries(), vec![TrayEntry { app_id: "dev.ecm.discord".into(), pid: 41, modes: vec![], hidden: vec![1] }]);
        assert_eq!(s.close_action(3), CloseAction::Close);
        assert_eq!(s.topmost_shown(), Some(3), "hidden windows aren't focused");
        assert_eq!(s.windows_of("dev.ecm.discord"), vec![1, 3]);
        // The desktop's own surfaces never get an icon.
        s.tray.push(TrayApp { pid: 1, app_id: "dev.ecm.desktop".into(), modes: vec!["audio".into()] });
        assert_eq!(s.tray_entries().len(), 1);
    }

    #[test]
    fn background_prompts_are_worded_for_the_mode() {
        assert_eq!(prompt_question("Discord", "background:call"), "Keep Discord running in the background for calls?");
        assert_eq!(prompt_question("Music", "background:audio"), "Keep Music running in the background for audio?");
        assert_eq!(prompt_question("Radio", "background:radio"), "Keep Radio running in the background for radio?");
        assert_eq!(prompt_question("Maps", "background:navigation"), "Keep Maps running in the background for navigation?");
        assert_eq!(prompt_question("Store", "background:transfer"), "Keep Store running in the background for downloads?");
        assert_eq!(prompt_question("Notes", "files:documents"), "Allow Notes to use files:documents?");
    }
}
