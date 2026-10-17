//! `dev.ecm.phone` — the ePhone shell (ROADMAP P5, docs/gui/EPHONE.md "Phone
//! shell"). It owns only layer surfaces, never windows:
//!
//! * **home** (`background` layer): wallpaper and the grid of installed apps;
//! * **status** (`top` layer, 16 lu at the top): time, unread dot,
//!   background-mode pills, cellular and Wi-Fi bars, battery;
//! * **nav** (`top` layer, 20 lu at the bottom): back, home, recent apps;
//! * **overlay** (`overlay` layer, created while shown): the notification
//!   sheet (Now Playing cards, the volume slider and its per-app strips,
//!   then the notifications), the app switcher, the permission dialog
//!   (worded "Keep <App> running in the background for ...?" for a
//!   `background:<mode>` prompt) and the Controls (hotkeys) screen.
//!
//! One app is on screen at a time, placed between the bars; the others are
//! hidden (and the OS suspends them). Keys (chords, since the shell has no
//! window): **Alt+H** home, **Alt+Backspace** back, **Alt+Tab** next app,
//! **Alt+N** notifications. The ePhone's switch-app key arrives as
//! `Chord{0xEC01}` and does what Alt+Tab does, without opening the phone.
//!
//! The policy is plain code driven by messages and an [`Env`] (status, apps,
//! clock), so a recorded session replays it exactly for the golden images.

use ecm_audio::mixer::{self, Strips};
use ecm_ui::diff::{SceneNode, TreeState};
use ecm_ui::shell::{Shell, ShellEvent};
use ecm_ui::{ConnError, Connection, MediaSession, MediaState, Theme};
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
    /// The MIC pill: a hidden app hears you (iOS's orange dot).
    pub const MIC: u32 = 0xff9f0aff;
    /// The CALL pill.
    pub const CALL: u32 = 0x34c759ff;
    /// Slider tracks and empty meters.
    pub const TRACK: u32 = 0x3a4452ff;
    pub const METER: u32 = 0x5ad17aff;
    pub const DUCKED: u32 = 0xe8b339ff;
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
    /// Hidden apps that read the microphone now (`mic <app>`: the MIC pill).
    pub mics: Vec<String>,
    /// (app id, state) of the media sessions (`media <app> <state>`).
    pub media: Vec<(String, String)>,
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
                "mic" if !rest.is_empty() => s.mics.push(rest.to_string()),
                "media" => {
                    let mut p = rest.splitn(2, ' ');
                    if let (Some(a), Some(st)) = (p.next(), p.next()) {
                        s.media.push((a.to_string(), st.to_string()));
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
    /// `/dev/mixer`'s text (the sheet's volume slider and strips; "" = no mixer).
    fn mixer(&mut self) -> String {
        String::new()
    }
    /// One `/dev/mixer` command (`set master 80`, `mute dev.ecm.music toggle`, `gain top 60`).
    fn mixer_command(&mut self, _cmd: &str) {}
}

/// What a volume slider or mute toggle in the sheet controls.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Knob {
    /// The phone's speaker (`set master <n>`).
    Master,
    /// An app's strip (`set <app> <n>`, 0-150 %).
    App(String),
    /// A microphone's gain (`gain <side> <n>`).
    Mic(String),
}

impl Knob {
    /// The slider's top value (percent).
    pub fn max(&self) -> u32 {
        match self {
            Knob::App(_) => mixer::MAX_APP_PERCENT,
            _ => 100,
        }
    }

    /// The mixer command that sets it to `v`.
    pub fn set_command(&self, v: u32) -> String {
        match self {
            Knob::Master => format!("set master {v}"),
            Knob::App(a) => format!("set {a} {v}"),
            Knob::Mic(side) => format!("gain {side} {v}"),
        }
    }

    /// The mixer command that flips its mute (None: microphones have no mute strip).
    pub fn mute_command(&self) -> Option<String> {
        match self {
            Knob::Master => Some("mute master toggle".into()),
            Knob::App(a) => Some(format!("mute {a} toggle")),
            Knob::Mic(_) => None,
        }
    }
}

/// One strip of the sheet's expanded mixer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MixerRow {
    pub knob: Knob,
    pub label: String,
    /// Percent (volume or gain).
    pub value: u32,
    /// None: no mute toggle (microphones).
    pub mute: Option<bool>,
    /// Meter fill, 0-100.
    pub meter: u32,
    /// A voice or phone stream lowers it right now.
    pub ducked: bool,
}

/// The strips the expanded sheet shows: each app (named with `name_of`), then the microphones.
pub fn mixer_rows(s: &Strips, name_of: &dyn Fn(&str) -> String) -> Vec<MixerRow> {
    let mut rows: Vec<MixerRow> = s
        .apps
        .iter()
        .map(|a| MixerRow {
            knob: Knob::App(a.app.clone()),
            label: name_of(&a.app),
            value: a.volume.min(mixer::MAX_APP_PERCENT),
            mute: Some(a.mute),
            meter: if a.mute { 0 } else { mixer::meter_percent(a.peak) },
            ducked: a.ducked,
        })
        .collect();
    let many = s.inputs.len() > 1;
    for i in &s.inputs {
        rows.push(MixerRow {
            knob: Knob::Mic(i.side.clone()),
            label: if many { format!("Microphone ({})", i.side) } else { "Microphone".into() },
            value: i.gain.min(100),
            mute: None,
            meter: (((i.level.clamp(-60, 0) + 60) * 100) / 60) as u32,
            ducked: false,
        });
    }
    rows
}

/// The phone speaker's master strip: (volume, muted), from the first output.
pub fn master_of(s: &Strips) -> Option<(u32, bool)> {
    s.outputs.first().map(|o| (o.volume.min(100), o.mute))
}

/// A Now Playing card: one per media session.
#[derive(Clone, Debug)]
pub struct NowPlaying {
    pub pid: u32,
    pub app_id: String,
    pub session: MediaSession,
    /// The cover art's image resource.
    art: Option<u32>,
    art_key: (u16, u16, u64),
}

impl NowPlaying {
    /// What the play/pause button sends now.
    pub fn toggle_action(&self) -> &'static str {
        match self.session.state {
            MediaState::Playing | MediaState::Buffering => "pause",
            _ => "play",
        }
    }

    /// Whether the app offers `action` (an app that lists none offers all).
    pub fn offers(&self, action: &str) -> bool {
        self.session.actions.is_empty() || self.session.actions.iter().any(|a| a == action)
    }
}

/// A name for an app that isn't installed as one (a program's strip): `dev.ecm.discord` ->
/// "Discord"; program names (`aplay`) as they are.
pub fn short_name(id: &str) -> String {
    let last = id.rsplit('.').next().unwrap_or(id);
    if !id.contains('.') || last.is_empty() {
        return id.to_string();
    }
    let mut c = last.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// "m:ss" for the Now Playing card.
pub fn clock_of(ms: u32) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

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

/// The permission dialog's question and its buttons `(key, label, allow, remember)`. A
/// background mode's first-time prompt (`background:<mode>`) asks "Keep <App> running in the
/// background for <calls|audio|...>?" with Allow / Don't allow, both remembered.
pub fn prompt_text(app_name: &str, permission: &str) -> (String, Vec<(&'static str, &'static str, bool, bool)>) {
    match permission.strip_prefix("background:") {
        Some(mode) => (
            format!("Keep {app_name} running in the background for {}?", background_words(mode)),
            vec![("allow", "Allow", true, true), ("deny", "Don't allow", false, true)],
        ),
        None => (
            format!("Allow {app_name} to use {}?", permission_words(permission)),
            vec![("allow", "Allow", true, true), ("once", "Only this time", true, false), ("deny", "Don't allow", false, true)],
        ),
    }
}

/// The status bar's pills, left to right: `(key, label, fill)`. MIC (orange) while any hidden app
/// reads the microphone, then one per background mode in use (CALL, AUDIO, RADIO, DL, NAV).
pub fn status_pills(st: &Status) -> Vec<(String, String, u32)> {
    let mut out = Vec::new();
    if !st.mics.is_empty() {
        out.push(("mic".to_string(), "MIC".to_string(), colors::MIC));
    }
    let mut seen: Vec<&str> = Vec::new();
    // A call first: it is the one the player most needs to see.
    let mut modes: Vec<&str> = st.modes.iter().map(|(_, m)| m.as_str()).collect();
    modes.sort_by_key(|m| *m != "call");
    for mode in modes {
        if seen.contains(&mode) {
            continue;
        }
        seen.push(mode);
        let (label, fill) = match mode {
            "call" => ("CALL", colors::CALL),
            "audio" => ("AUDIO", colors::ACCENT),
            "radio" => ("RADIO", colors::ACCENT),
            "transfer" => ("DL", colors::ACCENT),
            "navigation" => ("NAV", colors::ACCENT),
            other => (other, colors::ACCENT),
        };
        out.push((mode.to_string(), label.to_string(), fill));
    }
    out
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
    /// A Now Playing button: MediaControl to the app (play, pause, next, previous).
    Media { pid: u32, action: String },
    /// A volume slider's track (pressed and dragged).
    Slider(Knob),
    Mute(Knob),
    /// Expand / collapse the per-app strips under the volume slider.
    ToggleMixer,
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

    /// A slider: a track `w` lu long at `(x, y)`, filled to `value` of `max`, a knob, an optional
    /// notch at 100 %, and a taller hit area on top (pressed and dragged). Returns the track `(x, w)`.
    #[allow(clippy::too_many_arguments)]
    fn slider(&mut self, key: &str, x: i32, y: i32, w: i32, value: u32, max: u32, notch: bool, knob: Knob) -> (i32, i32) {
        let fill = (w as i64 * value.min(max) as i64 / max.max(1) as i64) as i32;
        self.rect(&format!("{key}:track"), (x, y, w, 4), colors::TRACK, 2, None);
        if fill > 0 {
            self.rect(&format!("{key}:fill"), (x, y, fill, 4), colors::ACCENT, 2, None);
        }
        if notch {
            let nx = x + (w as i64 * 100 / max.max(1) as i64) as i32;
            self.rect(&format!("{key}:notch"), (nx, y - 2, 1, 8), colors::MUTED, 0, None);
        }
        self.rect(&format!("{key}:knob"), (x + fill - 4, y - 2, 8, 8), colors::BAR_TEXT, 4, None);
        self.rect(&format!("{key}:hit"), (x - 6, y - 6, w + 12, 16), 0, 0, Some(Action::Slider(knob)));
        (x, w)
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
    /// Uploads to do after the next flush: (resource, kind, data).
    uploads: Vec<(u32, i32, Vec<u8>)>,
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
    /// Media sessions, newest first.
    pub media: Vec<NowPlaying>,
    /// `/dev/mixer` as last read (while the sheet is open).
    pub mixer: Strips,
    /// The per-app strips are expanded under the volume slider.
    pub mixer_open: bool,
    /// The slider being dragged.
    slider_drag: Option<Knob>,
    /// Where each slider's track was drawn: (x, w).
    sliders: HashMap<Knob, (i32, i32)>,
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
            media: Vec::new(),
            mixer: Strips::default(),
            mixer_open: false,
            slider_drag: None,
            sliders: HashMap::new(),
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
                        self.uploads.push((res, proto::host::UPLOAD_PNG, png.clone()));
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

    /// The notification sheet is open (the program then re-reads the mixer about twice a second).
    pub fn sheet_open(&self) -> bool {
        self.overlay == Some(Overlay::Sheet)
    }

    /// Re-read `/dev/mixer` for the sheet's slider, strips and meters (not while a slider is dragged).
    pub fn refresh_mixer(&mut self) {
        if self.slider_drag.is_none() {
            self.mixer = mixer::strips(&self.env.mixer());
        }
    }

    /// The value a slider shows now.
    fn knob_value(&self, k: &Knob) -> u32 {
        match k {
            Knob::Master => master_of(&self.mixer).map_or(0, |m| m.0),
            Knob::App(a) => self.mixer.app(a).map_or(100, |s| s.volume),
            Knob::Mic(side) => self.mixer.inputs.iter().find(|i| &i.side == side).map_or(0, |i| i.gain),
        }
    }

    /// Set a slider from a pointer at `x` on its track: the strip changes at once and the mixer is told.
    fn slide(&mut self, k: &Knob, x: i32) {
        let Some(&(tx, tw)) = self.sliders.get(k) else { return };
        let max = k.max() as i32;
        let mut v = (((x - tx) * max + tw / 2) / tw.max(1)).clamp(0, max) as u32;
        if matches!(k, Knob::App(_)) && v.abs_diff(100) <= 3 {
            v = 100; // the 100 % notch
        }
        if v == self.knob_value(k) {
            return;
        }
        match k {
            Knob::Master => self.mixer.outputs.iter_mut().for_each(|o| o.volume = v),
            Knob::App(a) => {
                if let Some(s) = self.mixer.apps.iter_mut().find(|s| &s.app == a) {
                    s.volume = v;
                }
            }
            Knob::Mic(side) => {
                if let Some(i) = self.mixer.inputs.iter_mut().find(|i| &i.side == side) {
                    i.gain = v;
                }
            }
        }
        self.env.mixer_command(&k.set_command(v));
    }

    fn toggle_mute(&mut self, k: &Knob) {
        let Some(cmd) = k.mute_command() else { return };
        match k {
            Knob::Master => self.mixer.outputs.iter_mut().for_each(|o| o.mute = !o.mute),
            Knob::App(a) => {
                if let Some(s) = self.mixer.apps.iter_mut().find(|s| &s.app == a) {
                    s.mute = !s.mute;
                }
            }
            Knob::Mic(_) => {}
        }
        self.env.mixer_command(&cmd);
    }

    /// A `MediaSessionChanged`: the card is added (newest first), updated in place, or removed.
    fn media_changed(&mut self, m: proto::MediaSessionChanged) {
        let session = MediaSession::from_wire(&m.info);
        let pos = self.media.iter().position(|n| n.pid == m.pid);
        if session.state == MediaState::None {
            if let Some(i) = pos {
                if let Some(res) = self.media.remove(i).art {
                    self.sh.send(Message::ResourceDestroy(proto::ResourceDestroy { id: res }));
                }
            }
            return;
        }
        let key = match &session.art {
            Some((w, h, px)) => (*w, *h, px.iter().enumerate().step_by(61).fold(px.len() as u64, |a, (i, b)| a.wrapping_mul(31).wrapping_add(*b as u64 + i as u64))),
            None => (0, 0, 0),
        };
        let (mut art, old_key) = match pos {
            Some(i) => (self.media[i].art, self.media[i].art_key),
            None => (None, (0, 0, 0)),
        };
        if key != old_key {
            if let Some(res) = art.take() {
                self.sh.send(Message::ResourceDestroy(proto::ResourceDestroy { id: res }));
            }
            if let Some((w, h, px)) = &session.art {
                let res = self.next_res;
                self.next_res += 1;
                self.sh.send(Message::ResourceCreate(proto::ResourceCreate {
                    id: res,
                    kind: proto::ResourceKind::Image,
                    w: *w as i32,
                    h: *h as i32,
                    format: proto::PixelFormat::Rgba8888,
                }));
                self.uploads.push((res, proto::host::UPLOAD_RGBA, px.clone()));
                art = Some(res);
            }
        }
        let card = NowPlaying { pid: m.pid, app_id: m.app_id, session, art, art_key: key };
        match pos {
            Some(i) => self.media[i] = card,
            None => self.media.insert(0, card),
        }
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
        let sheet = o == Overlay::Sheet;
        if sheet {
            self.unread = 0;
        }
        self.overlay = Some(o);
        if sheet {
            self.refresh_mixer();
        }
    }

    fn close_overlay(&mut self) {
        if matches!(self.overlay, Some(Overlay::Permission { .. })) {
            return; // only an answer closes it
        }
        self.overlay = None;
        self.mixer_open = false;
        self.slider_drag = None;
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
            Message::MediaSessionChanged(m) => self.media_changed(m),
            _ => {}
        }
    }

    fn pointer(&mut self, p: proto::Pointer) {
        match p.kind {
            proto::PointerKind::Down => {
                if let Some(Action::Slider(k)) = self.hits.actions.get(&p.hit_id).cloned() {
                    self.slide(&k, p.x);
                    self.slider_drag = Some(k);
                    return;
                }
                self.down.insert(p.surface, p.hit_id);
                if p.surface == SURF_STATUS {
                    self.pull_from = Some(p.y);
                }
            }
            proto::PointerKind::Move if self.slider_drag.is_some() => {
                if let Some(k) = self.slider_drag.clone() {
                    self.slide(&k, p.x);
                }
            }
            proto::PointerKind::Up if self.slider_drag.is_some() => {
                if let Some(k) = self.slider_drag.take() {
                    self.slide(&k, p.x);
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
            Action::Media { pid, action } => {
                self.sh.send(Message::MediaControl(proto::MediaControl { pid, action, position_ms: 0 }));
            }
            Action::Slider(_) => {} // pressed and dragged (see pointer)
            Action::Mute(k) => self.toggle_mute(&k),
            Action::ToggleMixer => {
                self.mixer_open = !self.mixer_open;
                self.refresh_mixer();
            }
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
        for (res, kind, data) in std::mem::take(&mut self.uploads) {
            self.sh.connection().upload(res, kind, &data)?;
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
        // Background modes in use (audio playing in the background, a call, ...); first MIC,
        // in orange, while a hidden app reads the microphone.
        for (key, label, fill) in status_pills(&st) {
            let tw = text_w(&label, 8);
            let dot = if key == "mic" { 6 } else { 0 };
            if x + tw + 6 + dot > w / 2 + 20 {
                break;
            }
            p.rect(&format!("mode:{key}"), (x, 3, tw + 6 + dot, h - 6), fill, 3, None);
            if dot > 0 {
                p.rect("mode:mic:dot", (x + 3, h / 2 - 2, 4, 4), 0x0b0f14ff, 2, None);
            }
            p.text(&format!("mode:{key}:t"), x + 3 + dot, (h - 8) / 2, &label, 8, 0x0b0f14ff, None);
            x += tw + 10 + dot;
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
        let media = self.media.clone();
        let mixer = self.mixer.clone();
        let mixer_open = self.mixer_open;
        let mut sliders: Vec<(Knob, (i32, i32))> = Vec::new();
        let mut p = Painter::new(&mut self.hits);
        match overlay {
            None => {}
            Some(Overlay::Sheet) => {
                let sh = if mixer_open { h - NAV_H } else { (h * 3 / 4).max(120) };
                p.rect("scrim", (0, 0, w, h), colors::SCRIM, 0, Some(Action::Dismiss));
                p.rect("sheet", (0, 0, w, sh), colors::SHEET, 0, Some(Action::Nothing));
                let mut y = 6;
                // Now Playing: one card per media session, newest first.
                for np in media.iter().take(2) {
                    let app = names.get(&np.app_id).cloned().unwrap_or_else(|| short_name(&np.app_id));
                    paint_now_playing(&mut p, np, &app, y, w);
                    y += 78;
                }
                // The volume slider (the phone speaker's master), and the strips under it.
                if let Some((vol, muted)) = master_of(&mixer) {
                    let tx = 30;
                    let tw = w - 30 - 64;
                    speaker_glyph(&mut p, "vol:icon", 10, y + 2, muted);
                    p.rect("vol:icon:hit", (4, y - 2, 24, 18), 0, 0, Some(Action::Mute(Knob::Master)));
                    sliders.push((Knob::Master, p.slider("vol", tx, y + 6, tw, vol, 100, false, Knob::Master)));
                    let pct = if muted { "Muted".to_string() } else { format!("{vol}%") };
                    p.text("vol:pct", tx + tw + 8, y + 2, &pct, 8, if muted { colors::DANGER } else { colors::CARD_TEXT }, None);
                    // The expand control: a chevron (down = more, up = less).
                    let cx = w - 16;
                    let cy = y + 7;
                    let pts = if mixer_open { [(cx - 4, cy + 2), (cx, cy - 2), (cx + 4, cy + 2)] } else { [(cx - 4, cy - 2), (cx, cy + 2), (cx + 4, cy - 2)] };
                    p.rect("vol:more:bg", (w - 26, y - 1, 20, 16), colors::CARD, 4, None);
                    p.line("vol:more", &pts, 2, colors::CARD_TEXT);
                    p.rect("vol:more:hit", (w - 28, y - 3, 24, 20), 0, 0, Some(Action::ToggleMixer));
                    y += 20;
                    if mixer_open {
                        let rows = mixer_rows(&mixer, &|id: &str| names.get(id).cloned().unwrap_or_else(|| short_name(id)));
                        if rows.is_empty() {
                            p.text("mix:none", 12, y, "Nothing is playing", 8, colors::MUTED, None);
                            y += 14;
                        }
                        for r in rows {
                            if y + 30 > sh - 12 {
                                break;
                            }
                            let k = match &r.knob {
                                Knob::App(a) => format!("mix:{a}"),
                                Knob::Mic(s) => format!("mic:{s}"),
                                Knob::Master => "mix:master".into(),
                            };
                            let label = ellipsize(&r.label, 8, w - 100);
                            p.text(&format!("{k}:name"), 12, y, &label, 8, colors::CARD_TEXT, None);
                            if r.ducked {
                                let bx = 12 + text_w(&label, 8) + 4;
                                let bw = text_w("ducked", 8) + 6;
                                p.rect(&format!("{k}:ducked"), (bx, y - 1, bw, 10), colors::DUCKED, 3, None);
                                p.text(&format!("{k}:ducked:t"), bx + 3, y, "ducked", 8, 0x0b0f14ff, None);
                            }
                            let muted = r.mute == Some(true);
                            let pct = if muted { "muted".to_string() } else { format!("{}%", r.value) };
                            p.text(&format!("{k}:pct"), w - 12 - text_w(&pct, 8), y, &pct, 8, if muted { colors::DANGER } else { colors::MUTED }, None);
                            let sx = 48;
                            let sw = w - sx - 14;
                            if r.mute.is_some() {
                                let fill = if muted { colors::DANGER } else { colors::CARD };
                                p.rect(&format!("{k}:mute"), (12, y + 10, 30, 12), fill, 3, None);
                                p.text_c(&format!("{k}:mute:t"), 27, y + 12, "Mute", 8, colors::CARD_TEXT, None);
                                p.rect(&format!("{k}:mute:hit"), (10, y + 8, 34, 16), 0, 0, Some(Action::Mute(r.knob.clone())));
                            } else {
                                speaker_mic_glyph(&mut p, &format!("{k}:icon"), 22, y + 10);
                            }
                            let max = r.knob.max();
                            sliders.push((r.knob.clone(), p.slider(&k, sx, y + 13, sw, r.value, max, max > 100, r.knob.clone())));
                            p.rect(&format!("{k}:meter:bg"), (sx, y + 21, sw, 2), colors::TRACK, 0, None);
                            if r.meter > 0 {
                                p.rect(&format!("{k}:meter"), (sx, y + 21, (sw * r.meter as i32 / 100).max(1), 2), colors::METER, 0, None);
                            }
                            y += 30;
                        }
                    }
                    y += 4;
                }
                p.text("title", 8, y, "Notifications", 12, colors::CARD_TEXT, None);
                if notifications.is_empty() {
                    p.text("empty", 8, y + 24, "No notifications", 8, colors::MUTED, None);
                } else {
                    p.text("clear", w - 8 - text_w("Clear all", 8), y + 2, "Clear all", 8, colors::ACCENT, Some(Action::ClearNotifications));
                }
                let mut y = y + 18;
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
                let (question, buttons) = prompt_text(&name, &permission);
                let lines = ecm_ui::text::wrap(&face(12), &question, Some(cw - 16));
                let mut ty = cy + 8;
                for (i, l) in lines.iter().take(3).enumerate() {
                    p.text(&format!("q{i}"), cx + 8, ty, l, 12, colors::CARD_TEXT, None);
                    ty += 13;
                }
                let detail = if permission.starts_with("background:") { "It keeps working while you use other apps.".to_string() } else { permission.clone() };
                p.text("perm", cx + 8, ty + 2, &ellipsize(&detail, 8, cw - 16), 8, colors::MUTED, None);
                let bh = 18;
                let mut by = cy + ch - buttons.len() as i32 * (bh + 4) - 2;
                for (k, label, allow, remember) in buttons {
                    let a = Action::Answer { allow, remember };
                    let fill = if k == "allow" { colors::ACCENT } else { colors::CARD };
                    p.rect(&format!("{k}:b"), (cx + 8, by, cw - 16, bh), fill, 4, Some(a.clone()));
                    let tc = if fill == colors::ACCENT { 0x0b0f14ff } else { colors::CARD_TEXT };
                    p.text_c(&format!("{k}:t"), cx + cw / 2, by + (bh - 12) / 2, label, 12, tc, Some(a));
                    by += bh + 4;
                }
            }
        }
        let nodes = p.nodes;
        self.sliders = sliders.into_iter().collect();
        nodes
    }
}

/// A speaker: a box and a cone; crossed out when muted.
fn speaker_glyph(p: &mut Painter, key: &str, x: i32, y: i32, muted: bool) {
    let c = colors::CARD_TEXT;
    p.rect(&format!("{key}:box"), (x, y + 4, 4, 6), c, 0, None);
    p.line(&format!("{key}:cone"), &[(x + 4, y + 4), (x + 9, y), (x + 9, y + 14), (x + 4, y + 10)], 2, c);
    if muted {
        p.line(&format!("{key}:x1"), &[(x + 12, y + 4), (x + 17, y + 10)], 2, colors::DANGER);
        p.line(&format!("{key}:x2"), &[(x + 17, y + 4), (x + 12, y + 10)], 2, colors::DANGER);
    } else {
        p.line(&format!("{key}:wave"), &[(x + 12, y + 3), (x + 14, y + 7), (x + 12, y + 11)], 1, c);
    }
}

/// A microphone (the mic strip has no mute toggle; it shows this instead).
fn speaker_mic_glyph(p: &mut Painter, key: &str, cx: i32, y: i32) {
    p.rect(&format!("{key}:head"), (cx - 3, y, 6, 8), colors::MUTED, 3, None);
    p.line(&format!("{key}:cup"), &[(cx - 5, y + 5), (cx - 5, y + 8), (cx, y + 11), (cx + 5, y + 8), (cx + 5, y + 5)], 1, colors::MUTED);
}

/// One Now Playing card (74 lu tall) at `y`: art, app, title, artist, progress, controls.
fn paint_now_playing(p: &mut Painter, np: &NowPlaying, app_name: &str, y: i32, w: i32) {
    let k = format!("np{}", np.pid);
    let s = &np.session;
    p.rect(&format!("{k}:card"), (6, y, w - 12, 74), colors::CARD, 4, Some(Action::Nothing));
    match np.art {
        Some(res) => p.image(&format!("{k}:art"), (12, y + 6, 36, 36), res, None),
        None => {
            p.rect(&format!("{k}:art"), (12, y + 6, 36, 36), colors::TRACK, 4, None);
            p.rect(&format!("{k}:note:head"), (22, y + 28, 8, 6), colors::CARD_TEXT, 3, None);
            p.rect(&format!("{k}:note:stem"), (28, y + 14, 2, 18), colors::CARD_TEXT, 0, None);
            p.line(&format!("{k}:note:flag"), &[(29, y + 15), (36, y + 20)], 2, colors::CARD_TEXT);
        }
    }
    let tx = 54;
    let tmax = w - tx - 14;
    let state = match s.state {
        MediaState::Paused => " - Paused",
        MediaState::Buffering => " - Loading",
        _ => "",
    };
    p.text(&format!("{k}:app"), tx, y + 5, &ellipsize(&format!("{app_name}{state}"), 8, tmax), 8, colors::MUTED, None);
    let title = if s.title.is_empty() { "Unknown title" } else { &s.title };
    p.text(&format!("{k}:title"), tx, y + 14, &ellipsize(title, 12, tmax), 12, colors::CARD_TEXT, None);
    p.text(&format!("{k}:artist"), tx, y + 28, &ellipsize(&s.artist, 8, tmax), 8, colors::MUTED, None);
    // Progress.
    let bw = tmax;
    p.rect(&format!("{k}:bar"), (tx, y + 40, bw, 3), colors::TRACK, 1, None);
    if s.duration_ms > 0 {
        let fill = (bw as i64 * s.position_ms.min(s.duration_ms) as i64 / s.duration_ms as i64) as i32;
        if fill > 0 {
            p.rect(&format!("{k}:fill"), (tx, y + 40, fill, 3), colors::ACCENT, 1, None);
        }
        let total = clock_of(s.duration_ms);
        p.text(&format!("{k}:pos"), tx, y + 45, &clock_of(s.position_ms), 8, colors::MUTED, None);
        p.text(&format!("{k}:dur"), tx + bw - text_w(&total, 8), y + 45, &total, 8, colors::MUTED, None);
    }
    // Controls: previous, play/pause, next.
    let cx = w / 2;
    let by = y + 57;
    let toggle = np.toggle_action();
    for (i, action) in ["previous", toggle, "next"].into_iter().enumerate() {
        let bx = cx + (i as i32 - 1) * 40;
        let on = np.offers(action);
        let c = if on { colors::CARD_TEXT } else { colors::TRACK };
        let g = format!("{k}:{i}");
        match action {
            "previous" => {
                p.rect(&format!("{g}:bar"), (bx - 6, by, 2, 12), c, 0, None);
                p.line(&format!("{g}:tri"), &[(bx + 5, by), (bx - 3, by + 6), (bx + 5, by + 12), (bx + 5, by)], 2, c);
            }
            "next" => {
                p.rect(&format!("{g}:bar"), (bx + 4, by, 2, 12), c, 0, None);
                p.line(&format!("{g}:tri"), &[(bx - 5, by), (bx + 3, by + 6), (bx - 5, by + 12), (bx - 5, by)], 2, c);
            }
            "pause" => {
                p.rect(&format!("{g}:l"), (bx - 4, by, 3, 12), c, 0, None);
                p.rect(&format!("{g}:r"), (bx + 1, by, 3, 12), c, 0, None);
            }
            _ => p.line(&format!("{g}:tri"), &[(bx - 4, by), (bx + 6, by + 6), (bx - 4, by + 12), (bx - 4, by)], 2, c),
        }
        if on {
            p.rect(&format!("{g}:hit"), (bx - 14, by - 3, 28, 18), 0, 0, Some(Action::Media { pid: np.pid, action: action.to_string() }));
        }
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

    const MIXER: &str = "output left volume=70 mute=0 peak=1200 label=ePhone_speaker\n\
                         app dev.ecm.music volume=80 mute=0 route=- on=left peak=5000 ducked=1 role=media streams=1\n\
                         app dev.ecm.discord volume=120 mute=1 route=- on=left peak=9000 ducked=0 role=voice streams=1\n\
                         input top gain=50 level=-30\n";

    /// An Env with a mixer: counts reads and logs commands.
    #[derive(Clone, Default)]
    struct Mixed {
        reads: std::rc::Rc<std::cell::Cell<u32>>,
        cmds: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    }
    impl Env for Mixed {
        fn clock(&mut self) -> String {
            "12:00".into()
        }
        fn status(&mut self) -> Status {
            Status::parse("state awake\nmode dev.ecm.music audio\nmode dev.ecm.discord call\nmic dev.ecm.discord\nmedia dev.ecm.music playing\n")
        }
        fn apps(&mut self) -> Vec<AppEntry> {
            vec![
                AppEntry { id: "dev.ecm.music".into(), name: "Music".into(), icon: None },
                AppEntry { id: "dev.ecm.discord".into(), name: "Discord".into(), icon: None },
            ]
        }
        fn session(&mut self) -> Vec<String> {
            vec![]
        }
        fn mixer(&mut self) -> String {
            self.reads.set(self.reads.get() + 1);
            MIXER.into()
        }
        fn mixer_command(&mut self, cmd: &str) {
            self.cmds.borrow_mut().push(cmd.into());
        }
    }

    fn texts(nodes: &[SceneNode]) -> Vec<String> {
        nodes
            .iter()
            .flat_map(|n| n.props.iter().filter_map(|p| if let Prop::Text(t) = p { Some(t.clone()) } else { None }))
            .collect()
    }

    fn mixed_shell(env: Mixed) -> (PhoneShell<Mixed>, MockConnection) {
        let mock = MockConnection::new();
        mock.push(&[welcome()]);
        let mut ps = PhoneShell::new(Box::new(mock.clone()), env, true).unwrap();
        for (s, w, h) in [(1, 240, 400), (2, 240, 16), (3, 240, 20)] {
            ps.handle(vec![ShellEvent::Configure(proto::Configure { surface: s, serial: 1, w, h, states: 0 })]);
        }
        (ps, mock)
    }

    #[test]
    fn status_has_mic_call_and_media_and_the_pills_put_mic_and_call_first() {
        let s = Mixed::default().status();
        assert_eq!(s.mics, vec!["dev.ecm.discord".to_string()]);
        assert_eq!(s.media, vec![("dev.ecm.music".to_string(), "playing".to_string())]);
        let pills: Vec<(String, u32)> = status_pills(&s).into_iter().map(|(_, l, c)| (l, c)).collect();
        assert_eq!(pills, vec![("MIC".into(), colors::MIC), ("CALL".into(), colors::CALL), ("AUDIO".into(), colors::ACCENT)]);
        assert!(status_pills(&Status::parse("mode a audio\nmode b audio\n")).len() == 1, "one pill per mode");
        assert!(status_pills(&Status::parse("mode a audio\n")).iter().all(|p| p.0 != "mic"), "no MIC without a mic line");
        let (mut ps, _) = mixed_shell(Mixed::default());
        let t = texts(&ps.paint_status());
        assert!(t.contains(&"MIC".to_string()) && t.contains(&"CALL".to_string()), "{t:?}");
    }

    #[test]
    fn background_prompts_ask_to_keep_the_app_running() {
        let (q, b) = prompt_text("Discord", "background:call");
        assert_eq!(q, "Keep Discord running in the background for calls?");
        assert_eq!(b, vec![("allow", "Allow", true, true), ("deny", "Don't allow", false, true)], "both answers are remembered");
        for (mode, words) in [("audio", "audio"), ("radio", "radio"), ("transfer", "downloads"), ("navigation", "navigation")] {
            assert_eq!(prompt_text("Music", &format!("background:{mode}")).0, format!("Keep Music running in the background for {words}?"));
        }
        let (q, b) = prompt_text("Notes", "network");
        assert_eq!(q, "Allow Notes to use the network?");
        assert_eq!(b.len(), 3, "ordinary permissions keep Only this time");
        // The dialog shows it.
        let (mut ps, mock) = mixed_shell(Mixed::default());
        ps.handle(vec![ShellEvent::Message(Message::PermissionPrompt(proto::PermissionPrompt { req: 3, app_id: "dev.ecm.discord".into(), permission: "background:call".into() }))]);
        ps.handle(vec![ShellEvent::Configure(proto::Configure { surface: SURF_OVERLAY, serial: 1, w: 240, h: 400, states: 0 })]);
        let t = texts(&ps.paint_overlay()).join(" ");
        assert!(t.contains("Keep Discord running") && t.contains("for calls?") && t.contains("Don't allow") && !t.contains("Only this time"), "{t}");
        ps.render().unwrap();
        mock.clear_sent();
        ps.act(Action::Answer { allow: false, remember: true });
        ps.render().unwrap();
        assert!(mock.sent_messages().iter().any(|m| matches!(m, Message::AnswerPermission(a) if a.req == 3 && !a.allow && a.remember)));
    }

    #[test]
    fn mixer_rows_come_from_the_mixer_text() {
        let s = mixer::strips(MIXER);
        assert_eq!(master_of(&s), Some((70, false)));
        let names = |id: &str| if id == "dev.ecm.music" { "Music".to_string() } else { id.to_string() };
        let rows = mixer_rows(&s, &names);
        assert_eq!(rows.len(), 3, "two apps and the microphone");
        assert_eq!(rows[0], MixerRow { knob: Knob::App("dev.ecm.music".into()), label: "Music".into(), value: 80, mute: Some(false), meter: mixer::meter_percent(5000), ducked: true });
        assert_eq!((rows[1].value, rows[1].mute, rows[1].meter), (120, Some(true), 0), "muted strips show no level");
        assert_eq!(rows[2], MixerRow { knob: Knob::Mic("top".into()), label: "Microphone".into(), value: 50, mute: None, meter: 50, ducked: false });
        assert_eq!(Knob::App("dev.ecm.music".into()).set_command(150), "set dev.ecm.music 150");
        assert_eq!(Knob::Master.set_command(40), "set master 40");
        assert_eq!(Knob::Mic("top".into()).set_command(60), "gain top 60");
        assert_eq!(Knob::Master.mute_command().as_deref(), Some("mute master toggle"));
        assert!(mixer_rows(&mixer::strips(""), &names).is_empty());
        assert_eq!((short_name("dev.ecm.discord").as_str(), short_name("aplay").as_str()), ("Discord", "aplay"));
    }

    #[test]
    fn the_sheet_has_a_volume_slider_that_expands_into_strips_and_reads_the_mixer_only_while_open() {
        let env = Mixed::default();
        let (reads, cmds) = (env.reads.clone(), env.cmds.clone());
        let (mut ps, _mock) = mixed_shell(env);
        ps.refresh_status();
        assert_eq!(reads.get(), 0, "closed: the mixer isn't read");
        ps.act(Action::PullSheet);
        assert_eq!(reads.get(), 1, "opening the sheet reads it");
        ps.handle(vec![ShellEvent::Configure(proto::Configure { surface: SURF_OVERLAY, serial: 1, w: 240, h: 400, states: 0 })]);
        let t = texts(&ps.paint_overlay());
        assert!(t.contains(&"70%".to_string()), "the master volume: {t:?}");
        assert!(!t.contains(&"ducked".to_string()), "collapsed: no strips");
        // A press on the master slider sets the speaker's volume where it was pressed.
        let (tx, tw) = ps.sliders[&Knob::Master];
        let hit = ps.hit_of(&Action::Slider(Knob::Master)).unwrap();
        let ptr = |kind, x| Message::Pointer(proto::Pointer { surface: SURF_OVERLAY, kind, x, y: 10, buttons: 1, button: proto::Button::Left, scroll_x: 0, scroll_y: 0, hit_id: hit });
        ps.handle(vec![ShellEvent::Message(ptr(proto::PointerKind::Down, tx + tw / 2))]);
        assert_eq!(cmds.borrow().last().map(String::as_str), Some("set master 50"));
        ps.handle(vec![ShellEvent::Message(ptr(proto::PointerKind::Move, tx + tw))]);
        ps.handle(vec![ShellEvent::Message(ptr(proto::PointerKind::Up, tx + tw))]);
        assert_eq!(cmds.borrow().last().map(String::as_str), Some("set master 100"), "dragged to the end");
        // Expand: one strip per app (ducked badge, mute) and the microphone.
        ps.act(Action::ToggleMixer);
        assert!(ps.mixer_open);
        let t = texts(&ps.paint_overlay());
        for want in ["Music", "Discord", "ducked", "80%", "muted", "Microphone", "50%"] {
            assert!(t.contains(&want.to_string()), "{want} in {t:?}");
        }
        ps.act(Action::Mute(Knob::App("dev.ecm.music".into())));
        assert_eq!(cmds.borrow().last().map(String::as_str), Some("mute dev.ecm.music toggle"));
        assert!(ps.mixer.app("dev.ecm.music").unwrap().mute, "shown muted at once");
        let (ax, aw) = ps.sliders[&Knob::App("dev.ecm.music".into())];
        let hit = ps.hit_of(&Action::Slider(Knob::App("dev.ecm.music".into()))).unwrap();
        ps.handle(vec![ShellEvent::Message(Message::Pointer(proto::Pointer { surface: SURF_OVERLAY, kind: proto::PointerKind::Down, x: ax + aw, y: 0, buttons: 1, button: proto::Button::Left, scroll_x: 0, scroll_y: 0, hit_id: hit }))]);
        assert_eq!(cmds.borrow().last().map(String::as_str), Some("set dev.ecm.music 150"), "apps go to 150 %");
        // Closing the sheet collapses it.
        ps.act(Action::Dismiss);
        assert!(!ps.sheet_open() && !ps.mixer_open);
    }

    #[test]
    fn now_playing_cards_send_media_controls_and_go_away_when_the_session_ends() {
        let (mut ps, mock) = mixed_shell(Mixed::default());
        let info = |title: &str, state: proto::MediaState, art: bool| proto::MediaInfo {
            title: title.into(),
            artist: "C418".into(),
            album: String::new(),
            art_w: if art { 2 } else { 0 },
            art_h: if art { 2 } else { 0 },
            art_rgba: if art { vec![255; 16] } else { Vec::new() },
            state,
            position_ms: 61_000,
            duration_ms: 185_000,
            actions: vec!["play".into(), "pause".into(), "next".into()],
        };
        let changed = |pid: u32, app: &str, i: proto::MediaInfo| ShellEvent::Message(Message::MediaSessionChanged(proto::MediaSessionChanged { pid, app_id: app.into(), info: i }));
        ps.handle(vec![changed(30, "dev.ecm.music", info("Sweden", proto::MediaState::Playing, true))]);
        ps.handle(vec![changed(31, "dev.ecm.radio", info("News", proto::MediaState::Paused, false))]);
        assert_eq!(ps.media.iter().map(|m| m.pid).collect::<Vec<_>>(), vec![31, 30], "newest first");
        ps.handle(vec![changed(30, "dev.ecm.music", info("Wet Hands", proto::MediaState::Paused, true))]);
        assert_eq!(ps.media.len(), 2, "one card per session");
        assert_eq!(ps.media[1].session.title, "Wet Hands", "updated in place");
        ps.act(Action::PullSheet);
        ps.handle(vec![ShellEvent::Configure(proto::Configure { surface: SURF_OVERLAY, serial: 1, w: 240, h: 400, states: 0 })]);
        let t = texts(&ps.paint_overlay());
        for want in ["Wet Hands", "News", "C418", "1:01", "3:05"] {
            assert!(t.contains(&want.to_string()), "{want} in {t:?}");
        }
        assert_eq!(ps.media[1].toggle_action(), "play");
        assert!(ps.hit_of(&Action::Media { pid: 30, action: "play".into() }).is_some());
        assert!(ps.hit_of(&Action::Media { pid: 30, action: "previous".into() }).is_none(), "not offered: no button");
        ps.render().unwrap();
        mock.clear_sent();
        ps.act(Action::Media { pid: 30, action: "play".into() });
        ps.render().unwrap();
        assert!(mock.sent_messages().iter().any(|m| matches!(m, Message::MediaControl(c) if c.pid == 30 && c.action == "play")));
        // The art was uploaded as RGBA; the end of the session removes the card and its art.
        let art = ps.media[1].art.expect("art resource");
        mock.clear_sent();
        ps.handle(vec![changed(30, "dev.ecm.music", info("", proto::MediaState::None, false))]);
        ps.render().unwrap();
        assert_eq!(ps.media.len(), 1);
        assert!(mock.sent_messages().iter().any(|m| matches!(m, Message::ResourceDestroy(d) if d.id == art)));
    }
}
