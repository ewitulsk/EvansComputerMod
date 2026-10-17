//! A tree-surface window: its element tree, widget state, layout, painting
//! into a display-list scene, and pointer / keyboard routing.
//!
//! Every render builds the element tree, lays it out at the configured size
//! (choosing layouts by size class), paints it into a [`SceneNode`] tree and
//! diffs that against the committed tree ([`TreeState`]). Interactive parts
//! carry a `hit_id`, so a `Pointer` event names the widget it hit and no
//! hit-testing happens here (§7).
//!
//! Text fields and text areas keep a caret and a selection (an anchor and
//! the caret, in characters). Their clipboard keys (Ctrl+C / X / V, Ctrl+A,
//! Shift+Insert, …) don't touch the clipboard themselves: they leave an
//! [`Effect`] that the app engine (`Core`) turns into `ClipboardSet` /
//! `ClipboardRequest` messages, so the compositor's clipboard rules (§12)
//! apply.

use crate::diff::{SceneNode, TreeState};
use crate::element::{Align, Element, EventKind, Handler, Kind};
use crate::text;
use crate::theme::Theme;
use ecm_ui_proto::fonts::{self, Face};
use ecm_ui_proto::{Button, Font, NodeKind, Point, Pointer, PointerKind, Prop, SizeClass, TreeOp};
use std::collections::HashMap;

/// USB HID usages (keyboard page) the toolkit reacts to.
pub mod keys {
    pub const A: u16 = 0x04;
    pub const C: u16 = 0x06;
    pub const F: u16 = 0x09;
    pub const N: u16 = 0x11;
    pub const O: u16 = 0x12;
    pub const S: u16 = 0x16;
    pub const V: u16 = 0x19;
    pub const X: u16 = 0x1B;
    pub const Z: u16 = 0x1D;
    pub const ENTER: u16 = 0x28;
    pub const ESCAPE: u16 = 0x29;
    pub const BACKSPACE: u16 = 0x2A;
    pub const TAB: u16 = 0x2B;
    pub const SPACE: u16 = 0x2C;
    pub const F2: u16 = 0x3B;
    pub const F3: u16 = 0x3C;
    pub const F5: u16 = 0x3E;
    pub const INSERT: u16 = 0x49;
    pub const HOME: u16 = 0x4A;
    pub const PAGE_UP: u16 = 0x4B;
    pub const DELETE: u16 = 0x4C;
    pub const END: u16 = 0x4D;
    pub const PAGE_DOWN: u16 = 0x4E;
    pub const RIGHT: u16 = 0x4F;
    pub const LEFT: u16 = 0x50;
    pub const DOWN: u16 = 0x51;
    pub const UP: u16 = 0x52;
    pub const KP_ENTER: u16 = 0x58;
}

/// Largest text a text field holds (a text node carries at most 4 KB).
pub const MAX_FIELD_TEXT: usize = 4000;
/// Largest text a text area holds.
pub const MAX_AREA_TEXT: usize = 1 << 20;
/// Two clicks on the same part within this many ms are a double click.
pub const DOUBLE_CLICK_MS: u64 = 450;
/// The clipboard MIME type text widgets use.
pub const TEXT_MIME: &str = "text/plain;charset=utf-8";

/// A widget event, handed to the element's handler.
#[derive(Clone, Debug)]
pub struct UiEvent {
    pub kind: EventKind,
    /// The window (surface id).
    pub window: u32,
    /// The widget's key (`#id`, or its path in the tree).
    pub widget: String,
    /// The element's id, if it has one.
    pub id: Option<String>,
    pub value: EventValue,
    pub handler: Handler,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EventValue {
    None,
    Bool(bool),
    Int(i32),
    Text(String),
    Index(usize),
    /// Custom areas and boards (`on_pointer`).
    Pointer(PointerInfo),
    /// A key on a custom area (presses, repeats and releases) or a focused
    /// board (presses and repeats): HID usage, `mods` bits, pressed /
    /// released, auto-repeat.
    Key { key: u16, mods: u16, down: bool, repeat: bool },
    /// Pixel grids (`on_change`): the texels that changed, `[x, y, index]` each.
    Texels(Vec<[i32; 3]>),
}

/// Something a widget wants done outside the window (by `Core`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Put this text on the clipboard.
    Copy(String),
    /// Ask for the clipboard's text for the widget with this key.
    Paste(String),
}

/// The P3 name of [`PointerInfo`] (custom areas).
pub type PointerEv = PointerInfo;

/// A pointer event on a custom area or a board, in its own coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointerInfo {
    /// Down, Move, Up or Scroll.
    pub kind: PointerKind,
    pub x: i32,
    pub y: i32,
    /// The button that changed (Down / Up).
    pub button: Button,
    /// Buttons held (`ecm_ui_proto::buttons`).
    pub buttons: u8,
    /// Wheel steps (Scroll; positive = towards the end).
    pub scroll_y: i32,
    /// Boards: the id of the element under the pointer: the innermost one
    /// with an id between the hit node and the board (None on the board
    /// itself; always None on custom areas).
    pub target: Option<String>,
}

/// Widget state kept between renders.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WidgetState {
    pub text: String,
    /// Caret position in characters.
    pub cursor: usize,
    /// The other end of the selection (characters); `None` = no selection.
    pub anchor: Option<usize>,
    pub checked: bool,
    pub value: i32,
    pub selected: Option<usize>,
    /// Vertical scroll offset (scroll views, text areas) / horizontal text offset (fields).
    pub scroll: i32,
    /// Horizontal scroll offset (text areas).
    pub scroll_x: i32,
    /// Scroll so the caret shows at the next render (after typing or moving it).
    pub follow: bool,
    /// Pixel grids: the bitmap (palette indices, `cols * rows`).
    pub pixels: Vec<u8>,
    /// Colour pickers: hue 0-359, saturation and value 0-255 (kept next to
    /// `value`, so a grey keeps its hue while dragging).
    pub hsv: [i32; 3],
}

impl WidgetState {
    /// The selected range `(start, end)` in characters, if any is non-empty.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        if a == self.cursor {
            return None;
        }
        Some((a.min(self.cursor), a.max(self.cursor)))
    }

    /// The selected text ("" without a selection).
    pub fn selected_text(&self) -> String {
        match self.selection() {
            Some((a, b)) => self.text.chars().skip(a).take(b - a).collect(),
            None => String::new(),
        }
    }

    /// Select `start..end` (characters), the caret at `end`.
    pub fn select(&mut self, start: usize, end: usize) {
        let n = self.text.chars().count();
        self.anchor = Some(start.min(n));
        self.cursor = end.min(n);
        self.follow = true;
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w: w.max(0), h: h.max(0) }
    }
}

/// What a hit id points at.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Main,
    Row(usize),
    Scrim,
    /// A scrollbar thumb (scroll views, text areas).
    Thumb,
    /// The scrollbar's track around the thumb: a press pages towards it.
    Track,
    /// A colour picker's hue strip, saturation/value square and hex field.
    Hue,
    Sv,
    Field,
}

/// What input routing needs to know about a painted widget.
#[derive(Clone, Debug)]
struct Info {
    kind: Kind,
    id: Option<String>,
    rect: Rect,
    parent: Option<String>,
    handlers: Vec<(EventKind, Handler)>,
    enabled: bool,
    range: (i32, i32, i32),
    rows: usize,
    /// Text fields and areas: absolute position of the first character
    /// (line 0, column 0, scrolled), and the face.
    text_x: i32,
    text_y: i32,
    face: (Font, u8),
    /// Scrollbars: (view height, content height, thumb height, absolute
    /// top of the bar).
    geom: (i32, i32, i32, i32),
    /// Password fields.
    secret: bool,
    /// Boards: grab every press inside.
    capture: bool,
    /// Pixel grids: what painting needs to know.
    grid: GridInfo,
    /// Colour pickers: the hue strip, the SV square and the hex field (absolute).
    picker: [Rect; 3],
}

/// A pixel grid's painting geometry and rules at the last render.
#[derive(Clone, Debug, Default)]
struct GridInfo {
    cols: i32,
    rows: i32,
    zoom: i32,
    /// Locked texels (non-zero), `cols * rows` (shorter = unlocked).
    mask: Vec<u8>,
    tool: Tool,
    mirror: bool,
    /// The palette index to paint with: the `color` prop, else the last eyedropped index.
    color: Option<u8>,
    colors: u8,
}

/// Pixel grid tools.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
enum Tool {
    #[default]
    Pencil,
    Fill,
    Eyedropper,
    Line,
}

impl Tool {
    fn from_name(s: Option<&str>) -> Tool {
        match s {
            Some("fill") => Tool::Fill,
            Some("eyedropper") | Some("pick") => Tool::Eyedropper,
            Some("line") => Tool::Line,
            _ => Tool::Pencil,
        }
    }
}

/// The palette a pixel grid uses when the app gives none (`0xRRGGBB`).
pub const DEFAULT_PALETTE: [u32; 16] = [
    0x000000, 0xffffff, 0xe5534b, 0xff9f1c, 0xffd400, 0x4caf50, 0x00bcd4, 0x3d8bfd, 0x7e57c2, 0xf06292, 0x8d6e63, 0x9e9e9e, 0x424242,
    0x1e3a5f, 0xc8e6c9, 0xffe0b2,
];

/// A colour picker's layout: the hue strip is this wide, with `PICKER_GAP`
/// to the SV square and above the hex field.
pub const HUE_STRIP_W: i32 = 16;
pub const PICKER_GAP: i32 = 4;
/// Hue bands drawn in the strip and cells per side of the SV square.
const HUE_BANDS: i32 = 24;
const SV_CELLS: i32 = 16;

/// A pointer drag in progress.
#[derive(Clone, Debug, PartialEq)]
enum Drag {
    Slider(String),
    /// Selecting text in a field or area.
    Select(String),
    Thumb { key: String, y0: i32, scroll0: i32 },
    Content { key: String, y0: i32, scroll0: i32 },
    Custom(String),
    /// Painting a pixel grid with the pencil (`cursor` holds the last texel).
    Paint(String),
    /// Dragging a pixel grid's line tool (`anchor` .. `cursor` is the preview).
    Line(String),
    /// Dragging a colour picker's hue strip / SV square.
    Hue(String),
    Sv(String),
}

/// One built widget: an element with its key, laid out.
struct W<'e> {
    key: String,
    el: &'e Element,
    children: Vec<W<'e>>,
    rect: Rect,
    /// Labels: the wrapped lines.
    lines: Vec<String>,
    /// List rows, menu items and tab headers (absolute).
    parts: Vec<Rect>,
    /// Dialog card / scroll content (absolute).
    inner: Rect,
}

/// Read access to a window for view functions.
pub struct ViewCtx<'a> {
    pub window: u32,
    pub width: i32,
    pub height: i32,
    pub class: SizeClass,
    pub theme: &'a Theme,
    states: &'a HashMap<String, WidgetState>,
}

impl ViewCtx<'_> {
    /// Text of the text field with this id.
    pub fn text(&self, id: &str) -> String {
        self.states.get(&format!("#{id}")).map(|s| s.text.clone()).unwrap_or_default()
    }
    pub fn checked(&self, id: &str) -> bool {
        self.states.get(&format!("#{id}")).is_some_and(|s| s.checked)
    }
    pub fn value(&self, id: &str) -> i32 {
        self.states.get(&format!("#{id}")).map_or(0, |s| s.value)
    }
    pub fn selected(&self, id: &str) -> Option<usize> {
        self.states.get(&format!("#{id}")).and_then(|s| s.selected)
    }
    /// The whole state of the widget with this id.
    pub fn state(&self, id: &str) -> Option<&WidgetState> {
        self.states.get(&key_for(id))
    }
    /// The bitmap of the pixel grid with this id (palette indices, `cols * rows`).
    pub fn pixels(&self, id: &str) -> Vec<u8> {
        self.states.get(&format!("#{id}")).map(|s| s.pixels.clone()).unwrap_or_default()
    }
}

/// Where a window's elements come from.
pub enum Content {
    /// Rebuilt by this function on every render.
    View(Box<dyn FnMut(&ViewCtx) -> Element>),
    /// A retained tree, replaced with `set_root`.
    Static(Element),
}

/// What kind of surface a window is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WindowRole {
    /// An ordinary window the shell places.
    Window,
    /// A popup (menus) anchored in its parent surface.
    Popup { parent: u32 },
    /// A shell layer surface (wallpaper, taskbar, overlays).
    Layer,
}

pub struct Window {
    pub surface: u32,
    pub title: String,
    pub width: i32,
    pub height: i32,
    pub configured: bool,
    pub activated: bool,
    pub role: WindowRole,
    /// Popup menus: close the popup after an item is chosen.
    pub close_on_select: bool,
    /// Commit even if nothing changed (canvas pixels were put).
    pub force_commit: bool,
    /// Whether a blinking caret is in its "on" phase.
    pub caret_on: bool,
    pub(crate) pending_ack: Option<u32>,
    pub(crate) dirty: bool,
    content: Content,
    pub(crate) states: HashMap<String, WidgetState>,
    focus: Option<String>,
    pressed: Option<(String, Part)>,
    hover: Option<(String, Part)>,
    drag: Option<Drag>,
    last_click: Option<(String, Part, u64)>,
    pub(crate) effects: Vec<Effect>,
    /// The board holding the pointer (pressed on it, until the release).
    grab: Option<String>,
    tree: TreeState,
    hit_ids: HashMap<String, u32>,
    hits: HashMap<u32, (String, Part)>,
    next_hit: u32,
    info: HashMap<String, Info>,
    focus_order: Vec<String>,
}

fn face_of(font: Font, size: u8) -> Face {
    fonts::face(font, size).or_else(|| fonts::face(font, 16)).or_else(|| fonts::face(Font::Ui, 12)).unwrap()
}

/// Clip a string to what one text node may carry.
fn clip_text(s: &str) -> String {
    if s.len() <= 4096 {
        return s.to_string();
    }
    let mut end = 4096;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

impl Window {
    pub fn new(surface: u32, title: impl Into<String>, content: Content) -> Window {
        Window {
            surface,
            title: title.into(),
            width: 0,
            height: 0,
            configured: false,
            activated: false,
            role: WindowRole::Window,
            close_on_select: false,
            force_commit: false,
            caret_on: true,
            pending_ack: None,
            dirty: true,
            content,
            states: HashMap::new(),
            focus: None,
            pressed: None,
            hover: None,
            drag: None,
            last_click: None,
            effects: Vec::new(),
            grab: None,
            tree: TreeState::new(),
            hit_ids: HashMap::new(),
            hits: HashMap::new(),
            next_hit: 1,
            info: HashMap::new(),
            focus_order: Vec::new(),
        }
    }

    pub fn set_content(&mut self, content: Content) {
        self.content = content;
        self.dirty = true;
    }

    pub fn class(&self) -> SizeClass {
        SizeClass::for_width(self.width)
    }

    pub(crate) fn configure(&mut self, serial: u32, w: i32, h: i32, states: u32) {
        self.width = w;
        self.height = h;
        if self.role == WindowRole::Window {
            self.activated = states & ecm_ui_proto::window_states::ACTIVATED != 0;
        }
        self.configured = true;
        self.pending_ack = Some(serial);
        self.dirty = true;
    }

    /// The state of the widget with this id (`#id`) or key.
    pub fn state(&self, id_or_key: &str) -> Option<&WidgetState> {
        self.states.get(&key_for(id_or_key))
    }

    pub fn state_mut(&mut self, id_or_key: &str) -> &mut WidgetState {
        self.dirty = true;
        self.states.entry(key_for(id_or_key)).or_default()
    }

    /// The key of the focused widget.
    pub fn focused(&self) -> Option<&str> {
        self.focus.as_deref()
    }

    pub fn focus(&mut self, id_or_key: Option<&str>) {
        self.focus = id_or_key.map(key_for);
        self.dirty = true;
    }

    /// Does the focused widget show a caret (text fields and areas)?
    pub fn caret_shown(&self) -> bool {
        self.activated
            && self.focus.as_ref().and_then(|f| self.info.get(f)).is_some_and(|i| matches!(i.kind, Kind::TextField | Kind::TextArea | Kind::ColorPicker))
    }

    /// The hit id of a widget's main interactive part (tests, tools).
    pub fn hit_id_of(&self, id_or_key: &str) -> Option<u32> {
        let key = key_for(id_or_key);
        self.hits.iter().filter(|(_, (k, p))| *k == key && *p == Part::Main).map(|(id, _)| *id).min().or_else(|| {
            self.hits.iter().filter(|(_, (k, p))| *k == key && *p != Part::Scrim).map(|(id, _)| *id).min()
        })
    }

    /// Every hit id a widget's parts carry, sorted (tests, tools).
    pub fn hit_ids_of(&self, id_or_key: &str) -> Vec<u32> {
        let key = key_for(id_or_key);
        let mut v: Vec<u32> = self.hits.iter().filter(|(_, (k, _))| *k == key).map(|(id, _)| *id).collect();
        v.sort_unstable();
        v
    }

    /// The hit ids of a scroll view's or text area's scrollbar: `(track,
    /// thumb)` (tests, tools; None while everything fits).
    pub fn scrollbar_hit_ids(&self, id_or_key: &str) -> Option<(u32, u32)> {
        let key = key_for(id_or_key);
        let find = |part: Part| self.hits.iter().find(|(_, (k, p))| *k == key && *p == part).map(|(id, _)| *id);
        Some((find(Part::Track)?, find(Part::Thumb)?))
    }

    /// The hit ids of a colour picker's parts: `(hue strip, SV square, hex
    /// field)` (tests, tools).
    pub fn picker_hit_ids(&self, id_or_key: &str) -> Option<(u32, u32, u32)> {
        let key = key_for(id_or_key);
        let find = |part: Part| self.hits.iter().find(|(_, (k, p))| *k == key && *p == part).map(|(id, _)| *id);
        Some((find(Part::Hue)?, find(Part::Sv)?, find(Part::Field)?))
    }

    /// The hit id of row / tab / item `i` of a list, tabs or menu (tests, tools).
    pub fn hit_id_of_row(&self, id_or_key: &str, i: usize) -> Option<u32> {
        let key = key_for(id_or_key);
        self.hits.iter().find(|(_, (k, p))| *k == key && *p == Part::Row(i)).map(|(id, _)| *id)
    }

    /// The absolute rectangle a widget was laid out in at the last render.
    pub fn rect_of(&self, id_or_key: &str) -> Option<Rect> {
        self.info.get(&key_for(id_or_key)).map(|i| i.rect)
    }

    /// The node id the diff engine gave a scene key (tests, tools).
    pub fn node_id(&self, scene_key: &str) -> Option<u32> {
        self.tree.id_of(scene_key)
    }

    /// Widget effects (clipboard) waiting for the engine.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    // ------------------------------------------------------------ render

    /// Lay out and paint the current elements; the TreeOps that bring the
    /// committed tree up to date (empty if nothing changed).
    pub fn render(&mut self, theme: &Theme) -> Vec<TreeOp> {
        self.dirty = false;
        let class = self.class();
        let root = match &mut self.content {
            Content::View(f) => {
                let cx = ViewCtx {
                    window: self.surface,
                    width: self.width,
                    height: self.height,
                    class,
                    theme,
                    states: &self.states,
                };
                f(&cx)
            }
            Content::Static(e) => e.clone(),
        };
        let mut tree = build(&root, "", 0, class, &mut self.states);
        {
            let lay = Layout { theme, states: &self.states, class };
            lay.arrange(&mut tree, Rect::new(0, 0, self.width, self.height));
        }
        let mut p = Painter {
            theme,
            states: &mut self.states,
            hit_ids: &mut self.hit_ids,
            next_hit: &mut self.next_hit,
            hits: HashMap::new(),
            info: HashMap::new(),
            focus_order: Vec::new(),
            focus: self.focus.as_deref(),
            pressed: self.pressed.as_ref().map(|(k, _)| k.as_str()),
            hover: self.hover.as_ref().map(|(k, p)| (k.as_str(), p.clone())),
            activated: self.activated,
            caret_on: self.caret_on,
            capture: 0,
        };
        let bg = SceneNode::new("root:bg", NodeKind::Rect, vec![Prop::W(self.width), Prop::H(self.height), Prop::Fill(theme.bg)]);
        let mut scene = vec![bg];
        p.paint(&tree, None, (0, 0), &mut scene);
        let (hits, info, order) = (p.hits, p.info, p.focus_order);
        self.hits = hits;
        self.info = info;
        self.focus_order = order;
        if let Some(f) = &self.focus {
            if !self.info.contains_key(f) {
                self.focus = None;
            }
        }
        self.tree.update(&scene)
    }

    // ------------------------------------------------------------ input

    fn event(&self, key: &str, kind: EventKind, value: EventValue) -> Option<UiEvent> {
        let info = self.info.get(key)?;
        let handler = info.handlers.iter().find(|(k, _)| *k == kind)?.1.clone();
        Some(UiEvent { kind, window: self.surface, widget: key.to_string(), id: info.id.clone(), value, handler })
    }

    fn enabled(&self, key: &str) -> bool {
        self.info.get(key).is_some_and(|i| i.enabled)
    }

    /// The board that gets a press on widget `key`: its nearest board
    /// ancestor (or itself), unless the widget handles presses itself and
    /// that board doesn't capture.
    fn board_for(&self, key: &str, part: &Part) -> Option<String> {
        let first = self.info.get(key)?;
        if first.kind == Kind::Board {
            return Some(key.to_string());
        }
        // A scrollbar (thumb or track) handles its own presses, like a
        // button: a scroll view inside a plain board can still be dragged.
        let interactive = first.kind.interactive() || matches!(part, Part::Thumb | Part::Track);
        let mut cur = first.parent.clone();
        while let Some(k) = cur {
            let i = self.info.get(&k)?;
            if i.kind == Kind::Board {
                return if interactive && !i.capture { None } else { Some(k) };
            }
            cur = i.parent.clone();
        }
        None
    }

    /// Is there a scroll view between widget `key` (inclusive) and `board`?
    fn scroll_view_within(&self, key: &str, board: &str) -> bool {
        let mut cur = Some(key.to_string());
        while let Some(k) = cur {
            if k == board {
                return false;
            }
            let Some(i) = self.info.get(&k) else { return false };
            if i.kind == Kind::ScrollView {
                return true;
            }
            cur = i.parent.clone();
        }
        false
    }

    /// The id a board reports for the widget `hit`: the innermost element
    /// with an id between it and the board (None when `hit` is the board or
    /// not inside it).
    fn board_target(&self, board: &str, hit: Option<&str>) -> Option<String> {
        let mut target = None;
        let mut cur = hit.map(String::from);
        while let Some(k) = cur {
            if k == board {
                return target;
            }
            let i = self.info.get(&k)?;
            if target.is_none() {
                target = i.id.clone();
            }
            cur = i.parent.clone();
        }
        None
    }

    /// A pointer event for board `board`, with `hit` the widget under the pointer.
    fn board_event(&self, board: &str, p: &Pointer, hit: Option<&str>) -> Option<UiEvent> {
        let info = self.info.get(board)?;
        let value = EventValue::Pointer(PointerInfo {
            kind: p.kind,
            x: p.x - info.rect.x,
            y: p.y - info.rect.y,
            button: p.button,
            buttons: p.buttons,
            scroll_y: p.scroll_y as i32,
            target: self.board_target(board, hit),
        });
        self.event(board, EventKind::Pointer, value)
    }

    /// A protocol pointer event (no double clicks); returns the widget events it caused.
    pub fn pointer_event(&mut self, p: &Pointer, theme: &Theme) -> Vec<UiEvent> {
        self.pointer_full(p.kind, p.x, p.y, p.buttons, p.button, p.scroll_y as i32, p.hit_id, theme, None)
    }

    fn kind_of(&self, key: &str) -> Option<Kind> {
        self.info.get(key).map(|i| i.kind)
    }

    /// A pointer event; returns the widget events it caused.
    pub fn pointer(&mut self, kind: PointerKind, x: i32, y: i32, scroll_y: i32, hit_id: u32, theme: &Theme) -> Vec<UiEvent> {
        let (buttons, button) = match kind {
            PointerKind::Down => (ecm_ui_proto::buttons::LEFT, Button::Left),
            PointerKind::Up => (0, Button::Left),
            _ => (0, Button::None),
        };
        self.pointer_full(kind, x, y, buttons, button, scroll_y, hit_id, theme, None)
    }

    /// A pointer event with every field (`now_ms` enables double clicks).
    #[allow(clippy::too_many_arguments)]
    pub fn pointer_full(
        &mut self,
        kind: PointerKind,
        x: i32,
        y: i32,
        buttons: u8,
        button: Button,
        scroll_y: i32,
        hit_id: u32,
        theme: &Theme,
        now_ms: Option<u64>,
    ) -> Vec<UiEvent> {
        let mut out = Vec::new();
        let target = self.hits.get(&hit_id).cloned();
        // Hover follows the pointer (not while a button is held).
        match kind {
            PointerKind::Enter | PointerKind::Move if self.pressed.is_none() => {
                if self.hover != target {
                    self.hover = target.clone();
                    self.dirty = true;
                }
            }
            PointerKind::Leave => {
                if self.hover.take().is_some() {
                    self.dirty = true;
                }
            }
            _ => {}
        }
        let p = Pointer {
            surface: self.surface,
            kind,
            x,
            y,
            buttons,
            button,
            scroll_x: 0,
            scroll_y: scroll_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            hit_id,
        };
        // A board holding the pointer gets everything until the release.
        if let Some(b) = self.grab.clone() {
            if matches!(kind, PointerKind::Down | PointerKind::Move | PointerKind::Up | PointerKind::Scroll) {
                if kind == PointerKind::Up && buttons == 0 {
                    self.grab = None;
                }
                return self.board_event(&b, &p, target.as_ref().map(|t| t.0.as_str())).into_iter().collect();
            }
            return out;
        }
        // Presses on a board (or a non-interactive child of one) go to the
        // board (not while another widget is dragging).
        if let Some((key, part)) = &target {
            if *part != Part::Scrim && self.drag.is_none() {
                if let Some(b) = self.board_for(key, part) {
                    match kind {
                        PointerKind::Down => {
                            self.grab = Some(b.clone());
                            if self.focus.as_deref() != Some(b.as_str()) {
                                self.focus = Some(b.clone());
                                self.dirty = true;
                            }
                            return self.board_event(&b, &p, Some(key)).into_iter().collect();
                        }
                        PointerKind::Move => return self.board_event(&b, &p, Some(key)).into_iter().collect(),
                        // Wheel steps scroll a scroll view inside the board.
                        PointerKind::Scroll if !self.scroll_view_within(key, &b) => {
                            return self.board_event(&b, &p, Some(key)).into_iter().collect()
                        }
                        _ => {}
                    }
                }
            }
        }
        // Custom areas get every event while they hold the pointer.
        let custom_target = match &self.drag {
            Some(Drag::Custom(k)) => Some(k.clone()),
            // (not while another widget, e.g. a scrollbar, is dragging)
            _ if self.drag.is_some() => None,
            _ => target.as_ref().filter(|(k, _)| self.kind_of(k) == Some(Kind::Custom)).map(|(k, _)| k.clone()),
        };
        if let Some(key) = custom_target {
            if self.enabled(&key) && !matches!(kind, PointerKind::Enter | PointerKind::Leave) {
                let r = self.info[&key].rect;
                let ev = PointerInfo { kind, x: x - r.x, y: y - r.y, buttons, button, scroll_y, target: None };
                match kind {
                    PointerKind::Down => {
                        if self.focus.as_deref() != Some(key.as_str()) {
                            self.focus = Some(key.clone());
                            self.dirty = true;
                        }
                        self.drag = Some(Drag::Custom(key.clone()));
                    }
                    PointerKind::Up if buttons == 0 => self.drag = None,
                    _ => {}
                }
                out.extend(self.event(&key, EventKind::Pointer, EventValue::Pointer(ev)));
                return out;
            }
        }
        match kind {
            PointerKind::Down => {
                let Some((key, part)) = target else {
                    if self.focus.take().is_some() {
                        self.dirty = true;
                    }
                    return out;
                };
                if part == Part::Scrim || !self.enabled(&key) {
                    return out;
                }
                let kind = self.info[&key].kind;
                if matches!(
                    kind,
                    Kind::TextField | Kind::TextArea | Kind::Slider | Kind::List | Kind::Tabs | Kind::Menu | Kind::PixelGrid | Kind::ColorPicker
                ) {
                    if self.focus.as_deref() != Some(key.as_str()) && kind == Kind::ColorPicker {
                        // Focus lands in the hex field: it starts from the current colour.
                        let v = self.states.get(&key).map_or(0, |s| s.value);
                        self.states.entry(key.clone()).or_default().text = hex_of(v);
                    }
                    self.focus = Some(key.clone());
                }
                let double = match (&self.last_click, now_ms) {
                    (Some((k, p, t)), Some(now)) => *k == key && *p == part && now.saturating_sub(*t) <= DOUBLE_CLICK_MS,
                    _ => false,
                };
                self.last_click = now_ms.map(|t| (key.clone(), part.clone(), t));
                match (kind, &part) {
                    (Kind::ScrollView | Kind::TextArea, Part::Thumb) => {
                        let scroll0 = self.states.get(&key).map_or(0, |s| s.scroll);
                        self.drag = Some(Drag::Thumb { key: key.clone(), y0: y, scroll0 });
                    }
                    (Kind::ScrollView | Kind::TextArea, Part::Track) => {
                        // Page towards the pointer: a view less one wheel step, so a
                        // little of what was showing stays in sight.
                        let (view, content, thumb, top) = self.info[&key].geom;
                        let cur = self.states.get(&key).map_or(0, |s| s.scroll);
                        let max = (content - view).max(0);
                        let ty = top + (cur.min(max) as i64 * (view - thumb) as i64 / max.max(1) as i64) as i32;
                        let page = (view - theme.scroll_step as i32).max(view / 2).max(1);
                        if y < ty {
                            self.set_scroll(&key, (cur - page).max(0));
                        } else if y >= ty + thumb {
                            self.set_scroll(&key, (cur + page).min(max));
                        }
                    }
                    (Kind::ScrollView, Part::Main) => {
                        let scroll0 = self.states.get(&key).map_or(0, |s| s.scroll);
                        self.drag = Some(Drag::Content { key: key.clone(), y0: y, scroll0 });
                    }
                    (Kind::TextField | Kind::TextArea, _) => {
                        let at = self.text_index_at(&key, x, y);
                        let st = self.states.entry(key.clone()).or_default();
                        if double {
                            let (a, b) = word_at(&st.text, at);
                            st.anchor = Some(a);
                            st.cursor = b;
                        } else {
                            st.cursor = at;
                            st.anchor = Some(at);
                            self.drag = Some(Drag::Select(key.clone()));
                        }
                        st.follow = true;
                    }
                    (Kind::Slider, _) => {
                        self.drag = Some(Drag::Slider(key.clone()));
                        out.extend(self.slide_to(&key, x));
                    }
                    (Kind::PixelGrid, Part::Main) => out.extend(self.grid_press(&key, x, y)),
                    (Kind::ColorPicker, Part::Hue) => {
                        self.drag = Some(Drag::Hue(key.clone()));
                        out.extend(self.hue_to(&key, y));
                    }
                    (Kind::ColorPicker, Part::Sv) => {
                        self.drag = Some(Drag::Sv(key.clone()));
                        out.extend(self.sv_to(&key, x, y));
                    }
                    _ => {}
                }
                if double {
                    if let (Kind::List, Part::Row(i)) = (kind, &part) {
                        out.extend(self.event(&key, EventKind::Activate, EventValue::Index(*i)));
                    }
                }
                self.pressed = Some((key, part));
                self.dirty = true;
            }
            PointerKind::Move => match self.drag.clone() {
                Some(Drag::Slider(key)) => out.extend(self.slide_to(&key, x)),
                Some(Drag::Select(key)) => {
                    let at = self.text_index_at(&key, x, y);
                    let st = self.states.entry(key).or_default();
                    if st.cursor != at {
                        st.cursor = at;
                        st.follow = true;
                        self.dirty = true;
                    }
                }
                Some(Drag::Thumb { key, y0, scroll0 }) => {
                    let (view, content, thumb, _) = self.info.get(&key).map_or((1, 1, 1, 0), |i| i.geom);
                    let span = (view - thumb).max(1);
                    let s = scroll0 + ((y - y0) as i64 * (content - view).max(0) as i64 / span as i64) as i32;
                    self.set_scroll(&key, s);
                }
                Some(Drag::Content { key, y0, scroll0 }) => self.set_scroll(&key, scroll0 - (y - y0)),
                Some(Drag::Paint(key)) => out.extend(self.grid_drag(&key, x, y)),
                Some(Drag::Line(key)) => {
                    let i = self.texel_at(&key, x, y);
                    let st = self.states.entry(key).or_default();
                    if st.cursor != i {
                        st.cursor = i;
                        self.dirty = true;
                    }
                }
                Some(Drag::Hue(key)) => out.extend(self.hue_to(&key, y)),
                Some(Drag::Sv(key)) => out.extend(self.sv_to(&key, x, y)),
                _ => {}
            },
            PointerKind::Up => {
                if let Some(Drag::Line(key)) = self.drag.clone() {
                    out.extend(self.grid_line_done(&key, x, y));
                }
                self.drag = None;
                let pressed = self.pressed.take();
                if pressed.is_some() {
                    self.dirty = true;
                }
                if let (Some((pk, pp)), Some((tk, tp))) = (pressed, target.clone()) {
                    if pk == tk && pp == tp {
                        out.extend(self.activate(&tk, &tp));
                    }
                }
                self.hover = target;
            }
            PointerKind::Scroll => {
                let Some((mut key, _)) = target else { return out };
                loop {
                    if self.info.get(&key).is_some_and(|i| matches!(i.kind, Kind::ScrollView | Kind::TextArea)) {
                        let cur = self.states.get(&key).map_or(0, |s| s.scroll);
                        let step = if self.kind_of(&key) == Some(Kind::TextArea) {
                            let i = &self.info[&key];
                            3 * face_of(i.face.0, i.face.1).line_height()
                        } else {
                            theme.scroll_step as i32
                        };
                        self.set_scroll(&key, cur + scroll_y * step);
                        break;
                    }
                    match self.info.get(&key).and_then(|i| i.parent.clone()) {
                        Some(p) => key = p,
                        None => break,
                    }
                }
            }
            PointerKind::Enter | PointerKind::Leave => {}
        }
        out
    }

    /// Scroll a scroll view to its end (its last child shows) at the next render, which
    /// clamps the offset to the content; e.g. a chat history after a new message.
    pub fn scroll_to_end(&mut self, id_or_key: &str) {
        let st = self.states.entry(key_for(id_or_key)).or_default();
        st.scroll = i32::MAX / 2;
        st.follow = false;
        self.dirty = true;
    }

    fn set_scroll(&mut self, key: &str, s: i32) {
        let st = self.states.entry(key.to_string()).or_default();
        let s = s.max(0);
        st.follow = false;
        if st.scroll != s {
            st.scroll = s;
            self.dirty = true;
        }
    }

    /// The character index under absolute point (x, y) of a text field or area.
    fn text_index_at(&self, key: &str, x: i32, y: i32) -> usize {
        let Some(i) = self.info.get(key) else { return 0 };
        let f = face_of(i.face.0, i.face.1);
        let st = self.states.get(key).cloned().unwrap_or_default();
        if i.kind == Kind::TextField {
            let shown = if i.secret { mask(&st.text) } else { st.text.clone() };
            return text::index_at(&f, &shown, x - i.text_x);
        }
        let lh = f.line_height().max(1);
        let line = ((y - i.text_y).max(0) / lh) as usize;
        let mut start = 0;
        for (n, l) in st.text.split('\n').enumerate() {
            let len = l.chars().count();
            if n == line {
                return start + text::index_at(&f, l, x - i.text_x);
            }
            start += len + 1;
        }
        st.text.chars().count()
    }

    fn slide_to(&mut self, key: &str, x: i32) -> Option<UiEvent> {
        let i = self.info.get(key)?;
        let (min, max, step) = i.range;
        let r = i.rect;
        let span = (r.w - 10).max(1);
        let raw = min as i64 + ((x - r.x - 5).clamp(0, span) as i64 * (max - min) as i64 + span as i64 / 2) / span as i64;
        let v = snap(raw as i32, min, max, step);
        self.set_value(key, v)
    }

    fn set_value(&mut self, key: &str, v: i32) -> Option<UiEvent> {
        let st = self.states.entry(key.to_string()).or_default();
        if st.value == v {
            return None;
        }
        st.value = v;
        self.dirty = true;
        self.event(key, EventKind::Change, EventValue::Int(v))
    }

    /// A click (or Enter / Space) on a widget part.
    fn activate(&mut self, key: &str, part: &Part) -> Option<UiEvent> {
        let kind = self.info.get(key)?.kind;
        if !self.enabled(key) {
            return None;
        }
        match (kind, part) {
            (Kind::Button, _) => self.event(key, EventKind::Click, EventValue::None),
            (Kind::Image | Kind::Canvas | Kind::Label, _) => self.event(key, EventKind::Click, EventValue::None),
            (Kind::Checkbox | Kind::Toggle, _) => {
                let st = self.states.entry(key.to_string()).or_default();
                st.checked = !st.checked;
                let v = st.checked;
                self.dirty = true;
                self.event(key, EventKind::Change, EventValue::Bool(v))
            }
            (Kind::List | Kind::Tabs | Kind::Menu, Part::Row(i)) => self.select(key, *i, true),
            _ => None,
        }
    }

    fn select(&mut self, key: &str, i: usize, always: bool) -> Option<UiEvent> {
        let st = self.states.entry(key.to_string()).or_default();
        if st.selected == Some(i) && !always {
            return None;
        }
        st.selected = Some(i);
        self.dirty = true;
        self.event(key, EventKind::Select, EventValue::Index(i))
    }

    /// Typed text for the focused widget.
    pub fn text_input(&mut self, s: &str) -> Vec<UiEvent> {
        let Some(key) = self.focus.clone() else { return vec![] };
        if !self.enabled(&key) {
            return vec![];
        }
        match self.kind_of(&key) {
            Some(Kind::TextField | Kind::TextArea) => self.insert_text(&key, s),
            Some(Kind::Custom) => self.event(&key, EventKind::Text, EventValue::Text(s.to_string())).into_iter().collect(),
            Some(Kind::ColorPicker) => self.picker_text(&key, s),
            _ => vec![],
        }
    }

    /// Clipboard text arrived for the widget with this key (after `Effect::Paste`).
    pub fn paste(&mut self, key: &str, s: &str) -> Vec<UiEvent> {
        match self.kind_of(key) {
            Some(Kind::TextField | Kind::TextArea) if self.enabled(key) => self.insert_text(key, s),
            Some(Kind::Custom) => self.event(key, EventKind::Paste, EventValue::Text(s.to_string())).into_iter().collect(),
            Some(Kind::ColorPicker) if self.enabled(key) => self.picker_text(key, s),
            _ => vec![],
        }
    }

    /// Insert `s` at the caret of a text field or area, replacing the selection.
    fn insert_text(&mut self, key: &str, s: &str) -> Vec<UiEvent> {
        let area = self.kind_of(key) == Some(Kind::TextArea);
        let max = if area { MAX_AREA_TEXT } else { MAX_FIELD_TEXT };
        let st = self.states.entry(key.to_string()).or_default();
        let mut changed = delete_selection(st);
        let clean: String = if area {
            s.replace("\r\n", "\n").replace('\r', "\n").replace('\t', "    ").chars().filter(|c| *c == '\n' || !c.is_control()).collect()
        } else {
            s.chars().map(|c| if c == '\n' || c == '\t' { ' ' } else { c }).filter(|c| !c.is_control()).collect()
        };
        let mut at = byte_at(&st.text, st.cursor);
        for c in clean.chars() {
            if st.text.len() + c.len_utf8() > max {
                break;
            }
            st.text.insert(at, c);
            at += c.len_utf8();
            st.cursor += 1;
            changed = true;
        }
        if !changed {
            return vec![];
        }
        st.anchor = None;
        st.follow = true;
        let t = st.text.clone();
        self.dirty = true;
        self.event(key, EventKind::Change, EventValue::Text(t)).into_iter().collect()
    }

    /// A key press (down or repeat) for the focused widget.
    pub fn key(&mut self, key_code: u16, mods: u16, down: bool) -> Vec<UiEvent> {
        self.key_event(key_code, mods, down, false).0
    }

    /// A key for the focused widget: the events it caused, and whether a
    /// widget used it (unused keys go to the app's key handler).
    pub fn key_event(&mut self, key_code: u16, mods: u16, down: bool, repeat: bool) -> (Vec<UiEvent>, bool) {
        let shift = mods & ecm_ui_proto::mods::SHIFT != 0;
        let ctrl = mods & ecm_ui_proto::mods::CTRL != 0;
        // Custom areas take every key, releases too, Tab included (except Ctrl+Tab).
        if let Some(key) = self.focus.clone() {
            if self.kind_of(&key) == Some(Kind::Custom) && self.enabled(&key) && !(ctrl && key_code == keys::TAB) {
                let ev = self.event(&key, EventKind::Key, EventValue::Key { key: key_code, mods, down, repeat });
                let used = ev.is_some();
                return (ev.into_iter().collect(), used);
            }
        }
        if !down {
            return (vec![], false);
        }
        let focused_kind = self.focus.as_ref().and_then(|f| self.kind_of(f));
        if key_code == keys::TAB && (focused_kind != Some(Kind::TextArea) || ctrl) {
            self.cycle_focus(shift);
            return (vec![], true);
        }
        let Some(key) = self.focus.clone() else { return (vec![], false) };
        let Some(info) = self.info.get(&key).cloned() else { return (vec![], false) };
        if !info.enabled {
            return (vec![], false);
        }
        let enter = key_code == keys::ENTER || key_code == keys::KP_ENTER;
        match info.kind {
            Kind::TextField | Kind::TextArea => self.text_key(&key, &info, key_code, mods),
            Kind::ColorPicker => self.picker_key(&key, key_code, mods),
            Kind::Board => {
                let ev = self.event(&key, EventKind::Key, EventValue::Key { key: key_code, mods, down, repeat });
                let used = ev.is_some();
                (ev.into_iter().collect(), used)
            }
            Kind::Button | Kind::Checkbox | Kind::Toggle if enter || key_code == keys::SPACE => {
                (self.activate(&key, &Part::Main).into_iter().collect(), true)
            }
            Kind::Slider => {
                let (min, max, step) = info.range;
                let cur = self.states.get(&key).map_or(min, |s| s.value);
                let st = step.max(1);
                let v = match key_code {
                    keys::LEFT | keys::DOWN => cur - st,
                    keys::RIGHT | keys::UP => cur + st,
                    keys::HOME => min,
                    keys::END => max,
                    _ => return (vec![], false),
                };
                (self.set_value(&key, snap(v, min, max, step)).into_iter().collect(), true)
            }
            Kind::List | Kind::Tabs | Kind::Menu => {
                let n = info.rows;
                if n == 0 {
                    return (vec![], false);
                }
                let mut cur = self.states.get(&key).and_then(|s| s.selected);
                if info.kind == Kind::Tabs {
                    cur = Some(cur.unwrap_or(0));
                }
                let (prev, next) = if info.kind == Kind::Tabs { (keys::LEFT, keys::RIGHT) } else { (keys::UP, keys::DOWN) };
                let i = match key_code {
                    k if k == prev => cur.map_or(0, |c| c.saturating_sub(1)),
                    k if k == next => cur.map_or(0, |c| (c + 1).min(n - 1)),
                    keys::HOME => 0,
                    keys::END => n - 1,
                    _ if enter || key_code == keys::SPACE => {
                        return match cur {
                            Some(c) => {
                                let mut evs: Vec<UiEvent> = self.select(&key, c, true).into_iter().collect();
                                if enter && info.kind == Kind::List {
                                    evs.extend(self.event(&key, EventKind::Activate, EventValue::Index(c)));
                                }
                                (evs, true)
                            }
                            None => (vec![], false),
                        }
                    }
                    _ => return (vec![], false),
                };
                (self.select(&key, i, false).into_iter().collect(), true)
            }
            _ => (vec![], false),
        }
    }

    /// Editing keys of text fields and areas.
    fn text_key(&mut self, key: &str, info: &Info, code: u16, mods: u16) -> (Vec<UiEvent>, bool) {
        let shift = mods & ecm_ui_proto::mods::SHIFT != 0;
        let ctrl = mods & ecm_ui_proto::mods::CTRL != 0;
        let area = info.kind == Kind::TextArea;
        let rows = info.rows.max(1);
        let enter = code == keys::ENTER || code == keys::KP_ENTER;
        // Clipboard and select-all.
        let copy = (ctrl && code == keys::C) || (ctrl && code == keys::INSERT);
        let cut = (ctrl && code == keys::X) || (shift && code == keys::DELETE);
        let paste = (ctrl && code == keys::V) || (shift && code == keys::INSERT);
        if ctrl && code == keys::A {
            let st = self.states.entry(key.to_string()).or_default();
            let n = st.text.chars().count();
            st.anchor = Some(0);
            st.cursor = n;
            self.dirty = true;
            return (vec![], true);
        }
        if (copy || cut) && info.secret {
            return (vec![], true);
        }
        if copy || cut {
            let st = self.states.entry(key.to_string()).or_default();
            let t = st.selected_text();
            if t.is_empty() {
                return (vec![], true);
            }
            self.effects.push(Effect::Copy(t));
            if cut {
                let st = self.states.entry(key.to_string()).or_default();
                delete_selection(st);
                st.follow = true;
                let t = st.text.clone();
                self.dirty = true;
                return (self.event(key, EventKind::Change, EventValue::Text(t)).into_iter().collect(), true);
            }
            return (vec![], true);
        }
        if paste {
            self.effects.push(Effect::Paste(key.to_string()));
            return (vec![], true);
        }
        if ctrl && !matches!(code, keys::LEFT | keys::RIGHT | keys::HOME | keys::END | keys::BACKSPACE | keys::DELETE) {
            return (vec![], false);
        }
        if enter && !area {
            let t = self.states.get(key).map(|s| s.text.clone()).unwrap_or_default();
            return (self.event(key, EventKind::Submit, EventValue::Text(t)).into_iter().collect(), true);
        }
        if enter {
            return (self.insert_text(key, "\n"), true);
        }
        if code == keys::TAB {
            return (self.insert_text(key, "    "), true);
        }
        let st = self.states.entry(key.to_string()).or_default();
        let n = st.text.chars().count();
        let mut changed = false;
        let target: Option<usize> = match code {
            keys::LEFT => Some(match (st.selection(), shift, ctrl) {
                (Some((a, _)), false, false) => a,
                (_, _, true) => word_left(&st.text, st.cursor),
                _ => st.cursor.saturating_sub(1),
            }),
            keys::RIGHT => Some(match (st.selection(), shift, ctrl) {
                (Some((_, b)), false, false) => b,
                (_, _, true) => word_right(&st.text, st.cursor),
                _ => (st.cursor + 1).min(n),
            }),
            keys::HOME if area && !ctrl => Some(line_start(&st.text, st.cursor)),
            keys::END if area && !ctrl => Some(line_end(&st.text, st.cursor)),
            keys::HOME => Some(0),
            keys::END => Some(n),
            keys::UP if area => Some(move_lines(&st.text, st.cursor, -1)),
            keys::DOWN if area => Some(move_lines(&st.text, st.cursor, 1)),
            keys::PAGE_UP if area => Some(move_lines(&st.text, st.cursor, -(rows as i32 - 1).max(1))),
            keys::PAGE_DOWN if area => Some(move_lines(&st.text, st.cursor, (rows as i32 - 1).max(1))),
            keys::BACKSPACE => {
                if delete_selection(st) {
                    changed = true;
                } else if st.cursor > 0 {
                    let from = if ctrl { word_left(&st.text, st.cursor) } else { st.cursor - 1 };
                    let (a, b) = (byte_at(&st.text, from), byte_at(&st.text, st.cursor));
                    st.text.replace_range(a..b, "");
                    st.cursor = from;
                    changed = true;
                }
                None
            }
            keys::DELETE => {
                if delete_selection(st) {
                    changed = true;
                } else if st.cursor < n {
                    let to = if ctrl { word_right(&st.text, st.cursor) } else { st.cursor + 1 };
                    let (a, b) = (byte_at(&st.text, st.cursor), byte_at(&st.text, to));
                    st.text.replace_range(a..b, "");
                    changed = true;
                }
                None
            }
            _ => return (vec![], false),
        };
        if let Some(t) = target {
            if shift {
                if st.anchor.is_none() {
                    st.anchor = Some(st.cursor);
                }
            } else {
                st.anchor = None;
            }
            st.cursor = t.min(n);
        }
        st.follow = true;
        self.dirty = true;
        if changed {
            let st = self.states.get_mut(key).unwrap();
            st.anchor = None;
            let t = st.text.clone();
            return (self.event(key, EventKind::Change, EventValue::Text(t)).into_iter().collect(), true);
        }
        (vec![], true)
    }

    // ---------------------------------------------------------- pixel grids

    /// The bitmap of the pixel grid with this id or key (empty if none).
    pub fn pixels(&self, id_or_key: &str) -> Vec<u8> {
        self.states.get(&key_for(id_or_key)).map(|s| s.pixels.clone()).unwrap_or_default()
    }

    /// Replace the bitmap of the pixel grid with this id or key (`cols *
    /// rows` palette indices; shorter is padded with 0).
    pub fn set_pixels(&mut self, id_or_key: &str, pixels: &[u8]) {
        let st = self.state_mut(id_or_key);
        st.pixels = pixels.to_vec();
    }

    /// The texel index under absolute `(x, y)` of a pixel grid, clamped into the grid.
    fn texel_at(&self, key: &str, x: i32, y: i32) -> usize {
        let Some(i) = self.info.get(key) else { return 0 };
        let g = &i.grid;
        let z = g.zoom.max(1);
        let tx = ((x - i.rect.x).div_euclid(z)).clamp(0, g.cols - 1);
        let ty = ((y - i.rect.y).div_euclid(z)).clamp(0, g.rows - 1);
        (ty * g.cols + tx) as usize
    }

    /// Is absolute `(x, y)` inside the texels of the grid (not the leftover space)?
    fn texel_inside(&self, key: &str, x: i32, y: i32) -> bool {
        let Some(i) = self.info.get(key) else { return false };
        let g = &i.grid;
        let (lx, ly) = (x - i.rect.x, y - i.rect.y);
        lx >= 0 && ly >= 0 && lx < g.cols * g.zoom && ly < g.rows * g.zoom
    }

    /// Paint texel `i` (and its mirror) of grid `key` with `color`; the texels that changed.
    fn paint_texel(&mut self, key: &str, i: usize, color: u8, changed: &mut Vec<[i32; 3]>) {
        let Some(info) = self.info.get(key) else { return };
        let g = info.grid.clone();
        let st = self.states.entry(key.to_string()).or_default();
        let n = (g.cols * g.rows) as usize;
        if st.pixels.len() != n {
            st.pixels.resize(n, 0);
        }
        let (x, y) = (i as i32 % g.cols, i as i32 / g.cols);
        let mut targets = vec![(x, y)];
        if g.mirror {
            targets.push((g.cols - 1 - x, y));
        }
        for (tx, ty) in targets {
            let j = (ty * g.cols + tx) as usize;
            if j >= n || g.mask.get(j).is_some_and(|m| *m != 0) || st.pixels[j] == color {
                continue;
            }
            st.pixels[j] = color;
            changed.push([tx, ty, color as i32]);
        }
    }

    /// Paint every texel of the line from texel `a` to texel `b` (Bresenham).
    fn paint_line(&mut self, key: &str, a: usize, b: usize, color: u8, changed: &mut Vec<[i32; 3]>) {
        let cols = self.info.get(key).map_or(1, |i| i.grid.cols.max(1));
        for (x, y) in bresenham((a as i32 % cols, a as i32 / cols), (b as i32 % cols, b as i32 / cols)) {
            self.paint_texel(key, (y * cols + x) as usize, color, changed);
        }
    }

    fn grid_changed(&mut self, key: &str, changed: Vec<[i32; 3]>) -> Option<UiEvent> {
        if changed.is_empty() {
            return None;
        }
        self.dirty = true;
        self.event(key, EventKind::Change, EventValue::Texels(changed))
    }

    /// The palette index a grid paints with.
    fn paint_color(&self, key: &str) -> u8 {
        let Some(i) = self.info.get(key) else { return 0 };
        let st = self.states.get(key);
        let c = i.grid.color.or_else(|| st.and_then(|s| s.selected).map(|s| s as u8)).unwrap_or(0);
        c.min(i.grid.colors.saturating_sub(1))
    }

    /// A press on a pixel grid: paint, fill, pick or start a line.
    fn grid_press(&mut self, key: &str, x: i32, y: i32) -> Vec<UiEvent> {
        if !self.texel_inside(key, x, y) {
            return vec![];
        }
        let i = self.texel_at(key, x, y);
        let Some(info) = self.info.get(key) else { return vec![] };
        let tool = info.grid.tool;
        let color = self.paint_color(key);
        match tool {
            Tool::Pencil => {
                let mut changed = Vec::new();
                self.paint_texel(key, i, color, &mut changed);
                self.states.entry(key.to_string()).or_default().cursor = i;
                self.drag = Some(Drag::Paint(key.to_string()));
                self.grid_changed(key, changed).into_iter().collect()
            }
            Tool::Fill => {
                let changed = self.flood(key, i, color);
                self.grid_changed(key, changed).into_iter().collect()
            }
            Tool::Eyedropper => {
                let st = self.states.entry(key.to_string()).or_default();
                let picked = st.pixels.get(i).copied().unwrap_or(0);
                st.selected = Some(picked as usize);
                self.dirty = true;
                self.event(key, EventKind::Select, EventValue::Index(picked as usize)).into_iter().collect()
            }
            Tool::Line => {
                let st = self.states.entry(key.to_string()).or_default();
                st.anchor = Some(i);
                st.cursor = i;
                self.drag = Some(Drag::Line(key.to_string()));
                self.dirty = true;
                vec![]
            }
        }
    }

    /// The pencil moved: paint from the last texel to this one.
    fn grid_drag(&mut self, key: &str, x: i32, y: i32) -> Vec<UiEvent> {
        let i = self.texel_at(key, x, y);
        let last = self.states.get(key).map_or(i, |s| s.cursor);
        if last == i {
            return vec![];
        }
        let color = self.paint_color(key);
        let mut changed = Vec::new();
        self.paint_line(key, last, i, color, &mut changed);
        self.states.entry(key.to_string()).or_default().cursor = i;
        self.grid_changed(key, changed).into_iter().collect()
    }

    /// The line tool was released: paint the line.
    fn grid_line_done(&mut self, key: &str, x: i32, y: i32) -> Vec<UiEvent> {
        let end = self.texel_at(key, x, y);
        let Some(start) = self.states.get(key).and_then(|s| s.anchor) else { return vec![] };
        let color = self.paint_color(key);
        let mut changed = Vec::new();
        self.paint_line(key, start, end, color, &mut changed);
        let st = self.states.entry(key.to_string()).or_default();
        st.anchor = None;
        st.cursor = end;
        self.dirty = true;
        self.grid_changed(key, changed).into_iter().collect()
    }

    /// Flood-fill the texels connected to `i` with its colour (and the
    /// mirrored area with `mirror_h`); locked texels are walls.
    fn flood(&mut self, key: &str, i: usize, color: u8) -> Vec<[i32; 3]> {
        let Some(info) = self.info.get(key) else { return vec![] };
        let g = info.grid.clone();
        let (cols, rows) = (g.cols, g.rows);
        let n = (cols * rows) as usize;
        let st = self.states.entry(key.to_string()).or_default();
        if st.pixels.len() != n {
            st.pixels.resize(n, 0);
        }
        let mut changed = Vec::new();
        let mut seeds = vec![i];
        if g.mirror {
            let (x, y) = (i as i32 % cols, i as i32 / cols);
            seeds.push((y * cols + cols - 1 - x) as usize);
        }
        for seed in seeds {
            if seed >= n || g.mask.get(seed).is_some_and(|m| *m != 0) {
                continue;
            }
            let from = st.pixels[seed];
            if from == color {
                continue;
            }
            let mut stack = vec![seed];
            while let Some(j) = stack.pop() {
                if st.pixels[j] != from || g.mask.get(j).is_some_and(|m| *m != 0) {
                    continue;
                }
                st.pixels[j] = color;
                let (x, y) = (j as i32 % cols, j as i32 / cols);
                changed.push([x, y, color as i32]);
                if x > 0 {
                    stack.push(j - 1);
                }
                if x + 1 < cols {
                    stack.push(j + 1);
                }
                if y > 0 {
                    stack.push(j - cols as usize);
                }
                if y + 1 < rows {
                    stack.push(j + cols as usize);
                }
            }
        }
        changed
    }

    // ------------------------------------------------------- colour pickers

    /// Set a picker's colour from its HSV state; a `Change` event if it changed.
    fn picker_apply(&mut self, key: &str, hsv: [i32; 3]) -> Option<UiEvent> {
        self.picker_set(key, hsv, hsv_to_rgb(hsv[0], hsv[1], hsv[2]))
    }

    /// Set a picker's colour (its HSV and the exact RGB); a `Change` event if it changed.
    fn picker_set(&mut self, key: &str, hsv: [i32; 3], rgb: i32) -> Option<UiEvent> {
        let st = self.states.entry(key.to_string()).or_default();
        let changed = st.value != rgb;
        st.hsv = hsv;
        st.value = rgb;
        st.text = hex_of(rgb);
        self.dirty = true;
        if changed {
            self.event(key, EventKind::Change, EventValue::Int(rgb))
        } else {
            None
        }
    }

    fn hue_to(&mut self, key: &str, y: i32) -> Option<UiEvent> {
        let strip = self.info.get(key)?.picker[0];
        // The first row is hue 0, the last 359 (the marker uses the inverse mapping).
        let h = ((y - strip.y).clamp(0, strip.h.max(1) - 1) as i64 * 359 / (strip.h.max(2) - 1) as i64) as i32;
        let mut hsv = self.states.get(key).map_or([0, 0, 0], |s| s.hsv);
        hsv[0] = h.clamp(0, 359);
        self.picker_apply(key, hsv)
    }

    fn sv_to(&mut self, key: &str, x: i32, y: i32) -> Option<UiEvent> {
        let sq = self.info.get(key)?.picker[1];
        let s = ((x - sq.x).clamp(0, sq.w.max(1) - 1) as i64 * 255 / (sq.w.max(2) - 1) as i64) as i32;
        let v = 255 - ((y - sq.y).clamp(0, sq.h.max(1) - 1) as i64 * 255 / (sq.h.max(2) - 1) as i64) as i32;
        let mut hsv = self.states.get(key).map_or([0, 0, 0], |s| s.hsv);
        hsv[1] = s.clamp(0, 255);
        hsv[2] = v.clamp(0, 255);
        self.picker_apply(key, hsv)
    }

    /// Typed text for a colour picker's hex field: hex digits (and a leading `#`).
    fn picker_text(&mut self, key: &str, s: &str) -> Vec<UiEvent> {
        let st = self.states.entry(key.to_string()).or_default();
        for c in s.chars() {
            let c = c.to_ascii_lowercase();
            if c == '#' && st.text.is_empty() {
                st.text.push('#');
            } else if c.is_ascii_hexdigit() {
                if !st.text.starts_with('#') {
                    st.text.insert(0, '#');
                }
                if st.text.len() < 7 {
                    st.text.push(c);
                }
            }
        }
        st.cursor = st.text.chars().count();
        self.dirty = true;
        self.picker_parse(key, false).into_iter().collect()
    }

    /// Apply the hex field when it holds a whole colour (`force`: Enter,
    /// which also puts the field back to the current colour).
    fn picker_parse(&mut self, key: &str, force: bool) -> Option<UiEvent> {
        let text = self.states.get(key).map(|s| s.text.clone()).unwrap_or_default();
        match parse_hex(&text) {
            Some(rgb) => {
                let cur = self.states.get(key).map_or(0, |s| s.value);
                if rgb == cur {
                    return None;
                }
                let hsv = rgb_to_hsv(rgb, self.states.get(key).map_or(0, |s| s.hsv[0]));
                self.picker_set(key, hsv, rgb)
            }
            None if force => {
                let v = self.states.get(key).map_or(0, |s| s.value);
                self.states.entry(key.to_string()).or_default().text = hex_of(v);
                self.dirty = true;
                None
            }
            None => None,
        }
    }

    fn picker_key(&mut self, key: &str, code: u16, mods: u16) -> (Vec<UiEvent>, bool) {
        let ctrl = mods & ecm_ui_proto::mods::CTRL != 0;
        let shift = mods & ecm_ui_proto::mods::SHIFT != 0;
        if (ctrl && code == keys::V) || (shift && code == keys::INSERT) {
            self.effects.push(Effect::Paste(key.to_string()));
            return (vec![], true);
        }
        if (ctrl && code == keys::C) || (ctrl && code == keys::INSERT) {
            let v = self.states.get(key).map_or(0, |s| s.value);
            self.effects.push(Effect::Copy(hex_of(v)));
            return (vec![], true);
        }
        match code {
            keys::BACKSPACE => {
                let st = self.states.entry(key.to_string()).or_default();
                st.text.pop();
                st.cursor = st.text.chars().count();
                self.dirty = true;
                (vec![], true)
            }
            keys::ENTER | keys::KP_ENTER => (self.picker_parse(key, true).into_iter().collect(), true),
            keys::ESCAPE => {
                let v = self.states.get(key).map_or(0, |s| s.value);
                self.states.entry(key.to_string()).or_default().text = hex_of(v);
                self.dirty = true;
                (vec![], true)
            }
            _ => (vec![], false),
        }
    }

    fn cycle_focus(&mut self, back: bool) {
        let order: Vec<&String> = self.focus_order.iter().filter(|k| self.enabled(k)).collect();
        if order.is_empty() {
            return;
        }
        let cur = self.focus.as_ref().and_then(|f| order.iter().position(|k| *k == f));
        let n = order.len();
        let i = match (cur, back) {
            (None, false) => 0,
            (None, true) => n - 1,
            (Some(c), false) => (c + 1) % n,
            (Some(c), true) => (c + n - 1) % n,
        };
        self.focus = Some(order[i].clone());
        self.dirty = true;
    }
}

/// Delete the selected text; true if there was a selection.
fn delete_selection(st: &mut WidgetState) -> bool {
    let Some((a, b)) = st.selection() else {
        st.anchor = None;
        return false;
    };
    let (ba, bb) = (byte_at(&st.text, a), byte_at(&st.text, b));
    st.text.replace_range(ba..bb, "");
    st.cursor = a;
    st.anchor = None;
    true
}

/// What a password field shows.
fn mask(s: &str) -> String {
    "*".repeat(s.chars().count())
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The word (or run of other characters) around character `at`.
fn word_at(text: &str, at: usize) -> (usize, usize) {
    let cs: Vec<char> = text.chars().collect();
    if cs.is_empty() {
        return (0, 0);
    }
    let i = at.min(cs.len() - 1);
    let w = is_word(cs[i]);
    let mut a = i;
    while a > 0 && is_word(cs[a - 1]) == w && cs[a - 1] != '\n' {
        a -= 1;
    }
    let mut b = i;
    while b < cs.len() && is_word(cs[b]) == w && cs[b] != '\n' {
        b += 1;
    }
    (a, b)
}

fn word_left(text: &str, at: usize) -> usize {
    let cs: Vec<char> = text.chars().collect();
    let mut i = at.min(cs.len());
    while i > 0 && !is_word(cs[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word(cs[i - 1]) {
        i -= 1;
    }
    i
}

fn word_right(text: &str, at: usize) -> usize {
    let cs: Vec<char> = text.chars().collect();
    let mut i = at.min(cs.len());
    while i < cs.len() && !is_word(cs[i]) {
        i += 1;
    }
    while i < cs.len() && is_word(cs[i]) {
        i += 1;
    }
    i
}

/// (line, column) of character index `at`.
pub fn line_col(text: &str, at: usize) -> (usize, usize) {
    let (mut line, mut col) = (0, 0);
    for c in text.chars().take(at) {
        if c == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Character index of (line, column), the column clamped to the line.
pub fn index_of(text: &str, line: usize, col: usize) -> usize {
    let mut start = 0;
    for (n, l) in text.split('\n').enumerate() {
        let len = l.chars().count();
        if n == line {
            return start + col.min(len);
        }
        start += len + 1;
    }
    text.chars().count()
}

fn line_start(text: &str, at: usize) -> usize {
    let (l, _) = line_col(text, at);
    index_of(text, l, 0)
}

fn line_end(text: &str, at: usize) -> usize {
    let (l, _) = line_col(text, at);
    index_of(text, l, usize::MAX)
}

fn move_lines(text: &str, at: usize, by: i32) -> usize {
    let (l, c) = line_col(text, at);
    let lines = text.split('\n').count() as i64;
    let nl = (l as i64 + by as i64).clamp(0, lines - 1) as usize;
    index_of(text, nl, c)
}

fn snap(v: i32, min: i32, max: i32, step: i32) -> i32 {
    let (lo, hi) = (min.min(max), min.max(max));
    let mut v = v.clamp(lo, hi);
    if step > 1 {
        v = lo + ((v - lo + step / 2) / step) * step;
        v = v.min(hi);
    }
    v
}

fn byte_at(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(i, _)| i)
}

/// `#rrggbb` of an `0xRRGGBB` colour.
pub fn hex_of(rgb: i32) -> String {
    format!("#{:06x}", rgb as u32 & 0xff_ffff)
}

/// `0xRRGGBB` of `#rrggbb` / `rrggbb` (`#rgb` too); None otherwise.
pub fn parse_hex(s: &str) -> Option<i32> {
    let h = s.trim().strip_prefix('#').unwrap_or(s.trim());
    if h.is_empty() || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match h.len() {
        6 => i32::from_str_radix(h, 16).ok(),
        3 => {
            let v = i32::from_str_radix(h, 16).ok()?;
            let (r, g, b) = ((v >> 8) & 15, (v >> 4) & 15, v & 15);
            Some((r * 17) << 16 | (g * 17) << 8 | b * 17)
        }
        _ => None,
    }
}

/// HSV (hue 0-359, saturation and value 0-255) to `0xRRGGBB`, in integers.
pub fn hsv_to_rgb(h: i32, s: i32, v: i32) -> i32 {
    let (h, s, v) = (h.rem_euclid(360), s.clamp(0, 255), v.clamp(0, 255));
    let region = h / 60;
    let rem = h % 60;
    let p = v * (255 - s) / 255;
    let q = v * (255 * 60 - s * rem) / (255 * 60);
    let t = v * (255 * 60 - s * (60 - rem)) / (255 * 60);
    let (r, g, b) = match region {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    r << 16 | g << 8 | b
}

/// `0xRRGGBB` to HSV; a grey keeps `keep_hue`.
pub fn rgb_to_hsv(rgb: i32, keep_hue: i32) -> [i32; 3] {
    let (r, g, b) = ((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let v = max;
    let s = if max == 0 { 0 } else { d * 255 / max };
    let h = if d == 0 {
        keep_hue.rem_euclid(360)
    } else if max == r {
        (60 * (g - b) / d).rem_euclid(360)
    } else if max == g {
        60 * (b - r) / d + 120
    } else {
        60 * (r - g) / d + 240
    };
    [h, s, v]
}

/// The texels of a line from `a` to `b` (Bresenham), `a` first.
fn bresenham(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let (mut x0, mut y0) = a;
    let (x1, y1) = b;
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut out = Vec::new();
    loop {
        out.push((x0, y0));
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
    out
}

fn key_for(id_or_key: &str) -> String {
    if id_or_key.starts_with('/') || id_or_key.starts_with('#') {
        id_or_key.to_string()
    } else {
        format!("#{id_or_key}")
    }
}

fn class_index(class: SizeClass) -> usize {
    match class {
        SizeClass::Compact => 0,
        SizeClass::Medium => 1,
        SizeClass::Wide => 2,
    }
}

// ================================================================ build

/// Build the widget tree: resolve keys, apply state-replacing props, choose
/// responsive children and tab pages.
fn build<'e>(el: &'e Element, parent: &str, index: usize, class: SizeClass, states: &mut HashMap<String, WidgetState>) -> W<'e> {
    let key = match &el.id {
        Some(id) => format!("#{id}"),
        None => format!("{parent}/{index}{}", el.kind.name()),
    };
    // Props that replace widget state.
    let p = &el.props;
    match el.kind {
        Kind::TextField | Kind::TextArea => {
            if let Some(t) = &p.text {
                let max = if el.kind == Kind::TextArea { MAX_AREA_TEXT } else { MAX_FIELD_TEXT };
                let st = states.entry(key.clone()).or_default();
                if st.text != *t {
                    st.text = t.chars().take(max).collect();
                    st.cursor = st.text.chars().count();
                    st.anchor = None;
                }
            }
        }
        Kind::Checkbox | Kind::Toggle => {
            if let Some(c) = p.checked {
                states.entry(key.clone()).or_default().checked = c;
            }
        }
        Kind::Slider => {
            let st = states.entry(key.clone()).or_default();
            let (min, max) = (p.min.unwrap_or(0), p.max.unwrap_or(100));
            if let Some(v) = p.value {
                st.value = v;
            }
            st.value = snap(st.value, min, max, p.step.unwrap_or(1));
        }
        Kind::List | Kind::Tabs | Kind::Menu => {
            if let Some(s) = p.selected {
                states.entry(key.clone()).or_default().selected = Some(s);
            }
        }
        Kind::PixelGrid => {
            let st = states.entry(key.clone()).or_default();
            let n = p.cols.unwrap_or(16) as usize * p.rows.unwrap_or(16) as usize;
            if !p.pixels.is_empty() && st.pixels != p.pixels {
                st.pixels = p.pixels.clone();
            }
            if st.pixels.len() != n {
                st.pixels.resize(n, 0);
            }
        }
        Kind::ColorPicker => {
            let fresh = !states.contains_key(&key);
            let st = states.entry(key.clone()).or_default();
            if let Some(v) = p.value {
                let v = v & 0xff_ffff;
                if st.value != v {
                    st.value = v;
                    st.hsv = rgb_to_hsv(v, st.hsv[0]);
                    st.text = hex_of(v);
                }
            }
            if fresh {
                // The hex field starts out showing the colour (the player may empty it while editing).
                st.text = hex_of(st.value);
            }
        }
        _ => {}
    }
    let mut children = Vec::new();
    match el.kind {
        Kind::Responsive => {
            if !el.children.is_empty() {
                let i = class_index(class).min(el.children.len() - 1);
                children.push(build(&el.children[i], &key, i, class, states));
            }
        }
        Kind::Tabs => {
            if !el.children.is_empty() {
                let sel = states.get(&key).and_then(|s| s.selected).unwrap_or(0).min(el.children.len() - 1);
                children.push(build(&el.children[sel], &key, sel, class, states));
            }
        }
        _ => {
            for (i, c) in el.children.iter().enumerate() {
                if c.props.visible_in.is_some_and(|v| !v[class_index(class)]) {
                    continue;
                }
                children.push(build(c, &key, i, class, states));
            }
        }
    }
    W { key, el, children, rect: Rect::default(), lines: Vec::new(), parts: Vec::new(), inner: Rect::default() }
}

// ================================================================ layout

struct Layout<'a> {
    theme: &'a Theme,
    states: &'a HashMap<String, WidgetState>,
    class: SizeClass,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Axis {
    H,
    V,
}

/// The preferred size of an element at `avail` lu of width (popup sizing).
pub fn measure(el: &Element, theme: &Theme, class: SizeClass, avail: i32) -> (i32, i32) {
    let mut states = HashMap::new();
    let w = build(el, "", 0, class, &mut states);
    let lay = Layout { theme, states: &states, class };
    lay.measure(&w, avail)
}

impl Layout<'_> {
    fn face(&self, el: &Element) -> Face {
        let t = self.theme;
        let default = match el.props.style.as_deref() {
            Some("title") => t.title(),
            Some("caption") => t.small(),
            _ => t.body(),
        };
        let font = el.props.font.unwrap_or(Font::Ui);
        let default = if font == Font::Mono { 16 } else { default };
        face_of(font, el.props.size.unwrap_or(default))
    }

    fn pad(&self, el: &Element) -> i32 {
        match el.kind {
            Kind::Dialog => el.props.padding.unwrap_or(self.theme.padding as i32),
            _ => el.props.padding.unwrap_or(0),
        }
    }

    fn gap(&self, el: &Element) -> i32 {
        el.props.gap.unwrap_or(match el.kind {
            Kind::Row | Kind::Column | Kind::Grid | Kind::Dialog => self.theme.spacing as i32,
            _ => 0,
        })
    }

    fn grow(&self, el: &Element, axis: Axis) -> u16 {
        if let Some(g) = el.props.grow {
            return g;
        }
        match (el.kind, axis) {
            (Kind::Spacer, _) => 1,
            (Kind::TextField | Kind::Slider | Kind::TextArea, Axis::H) => 1,
            (Kind::ScrollView | Kind::TextArea, Axis::V) if el.props.height.is_none() => 1,
            (Kind::Board, Axis::H) if el.props.width.is_none() => 1,
            (Kind::Board, Axis::V) if el.props.height.is_none() => 1,
            (Kind::PixelGrid, Axis::H) if el.props.width.is_none() && el.props.zoom.is_none() => 1,
            (Kind::PixelGrid, Axis::V) if el.props.height.is_none() && el.props.zoom.is_none() => 1,
            _ => 0,
        }
    }

    fn columns(&self, el: &Element) -> i32 {
        let c = el.props.columns.unwrap_or([1, 2, 3]);
        c[class_index(self.class)].max(1) as i32
    }

    fn state(&self, key: &str) -> Option<&WidgetState> {
        self.states.get(key)
    }

    /// Preferred size within `avail` lu of width.
    fn measure(&self, w: &W, avail: i32) -> (i32, i32) {
        let el = w.el;
        let p = &el.props;
        let t = self.theme;
        let (px, py) = (t.px(), t.py());
        let (mw, mh) = match el.kind {
            Kind::Label => {
                let f = self.face(el);
                let width = p.width.unwrap_or(avail);
                let lines = text::wrap(&f, p.text.as_deref().unwrap_or(""), if p.wrap.unwrap_or(true) { Some(width) } else { None });
                let lw = lines.iter().map(|l| f.text_width(l)).max().unwrap_or(0);
                (p.width.unwrap_or(lw), lines.len() as i32 * f.line_height())
            }
            Kind::Button => {
                let f = self.face(el);
                (f.text_width(p.text.as_deref().unwrap_or("")) + 2 * px, f.line_height() + 2 * py)
            }
            Kind::TextField | Kind::Slider => {
                let f = self.face(el);
                (p.width.unwrap_or(120.min(avail)), f.line_height() + 2 * py)
            }
            Kind::TextArea => {
                let f = self.face(el);
                (p.width.unwrap_or(avail), 6 * f.line_height() + 2 * py)
            }
            Kind::Custom => (p.width.unwrap_or(0), p.height.unwrap_or(0)),
            Kind::Checkbox | Kind::Toggle => {
                let f = self.face(el);
                let lh = f.line_height();
                let bw = if el.kind == Kind::Toggle { 2 * lh } else { lh };
                let tw = f.text_width(p.text.as_deref().unwrap_or(""));
                (bw + if tw > 0 { 6 + tw } else { 0 }, lh)
            }
            Kind::List => {
                let inner = avail - 2 * px;
                let mut wmax = 0;
                let mut h = 0;
                for c in &w.children {
                    let (cw, ch) = self.measure(c, inner);
                    wmax = wmax.max(cw);
                    h += ch + 2 * py;
                }
                (wmax + 2 * px, h)
            }
            Kind::Menu => {
                let f = self.face(el);
                let wmax = p.items.iter().map(|s| f.text_width(s)).max().unwrap_or(0);
                (wmax + 2 * px, p.items.len() as i32 * (f.line_height() + 2 * py))
            }
            Kind::Tabs => {
                let f = self.face(el);
                let header: i32 = p.items.iter().map(|s| f.text_width(s) + 2 * px).sum();
                let (cw, ch) = w.children.first().map_or((0, 0), |c| self.measure(c, avail));
                (header.max(cw), f.line_height() + 2 * py + 1 + ch)
            }
            Kind::ScrollView => {
                let bar = t.scrollbar as i32;
                let (cw, ch) = w.children.first().map_or((0, 0), |c| self.measure(c, avail - bar));
                (cw + bar, p.height.unwrap_or(ch))
            }
            Kind::Dialog => {
                let (cw, ch) = self.card_size(w, avail);
                (cw, ch)
            }
            Kind::Image | Kind::Canvas => (p.width.unwrap_or(0), p.height.unwrap_or(0)),
            Kind::Spacer => (0, 0),
            Kind::Row => {
                let pad = self.pad(el);
                let inner = avail - 2 * pad;
                let widths = self.row_widths(w, inner);
                let any_grow = w.children.iter().any(|c| self.grow(c.el, Axis::H) > 0);
                let used: i32 = widths.iter().sum::<i32>() + self.gap(el) * (w.children.len() as i32 - 1).max(0);
                let h = w.children.iter().zip(&widths).map(|(c, cw)| self.measure(c, *cw).1).max().unwrap_or(0);
                (if any_grow { avail } else { used + 2 * pad }, h + 2 * pad)
            }
            Kind::Column => {
                let pad = self.pad(el);
                let inner = avail - 2 * pad;
                let mut wmax = 0;
                let mut h = 0;
                for c in &w.children {
                    let (cw, ch) = self.measure(c, inner);
                    wmax = wmax.max(cw);
                    h += ch;
                }
                h += self.gap(el) * (w.children.len() as i32 - 1).max(0);
                (wmax + 2 * pad, h + 2 * pad)
            }
            Kind::Stack => {
                let pad = self.pad(el);
                let mut s = (0, 0);
                for c in &w.children {
                    let (cw, ch) = self.measure(c, avail - 2 * pad);
                    s = (s.0.max(cw), s.1.max(ch));
                }
                (s.0 + 2 * pad, s.1 + 2 * pad)
            }
            Kind::Grid => {
                let pad = self.pad(el);
                let gap = self.gap(el);
                let cols = self.columns(el);
                let col_w = ((avail - 2 * pad - gap * (cols - 1)) / cols).max(0);
                let mut h = 0;
                for (r, chunk) in w.children.chunks(cols as usize).enumerate() {
                    if r > 0 {
                        h += gap;
                    }
                    h += chunk.iter().map(|c| self.measure(c, col_w).1).max().unwrap_or(0);
                }
                (avail, h + 2 * pad)
            }
            Kind::Responsive => w.children.first().map_or((0, 0), |c| self.measure(c, avail)),
            Kind::Board | Kind::Wire => (0, 0),
            Kind::PixelGrid => {
                let (cols, rows) = (p.cols.unwrap_or(16) as i32, p.rows.unwrap_or(16) as i32);
                match p.zoom {
                    Some(z) => (cols * z as i32, rows * z as i32),
                    None => {
                        let w = p.width.unwrap_or(avail).max(1);
                        (w, (w as i64 * rows as i64 / cols.max(1) as i64) as i32)
                    }
                }
            }
            Kind::ColorPicker => (120, 140),
            Kind::Panel => {
                let pad = self.pad(el);
                let mut s = (0, 0);
                for c in &w.children {
                    let (cw, ch) = self.measure(c, avail - 2 * pad);
                    s = (s.0.max(cw), s.1.max(ch));
                }
                (s.0 + 2 * pad, s.1 + 2 * pad)
            }
        };
        (p.width.unwrap_or(mw), p.height.unwrap_or(mh))
    }

    fn card_size(&self, w: &W, avail: i32) -> (i32, i32) {
        let el = w.el;
        let pad = self.pad(el);
        let gap = self.gap(el);
        let tf = face_of(Font::Ui, self.theme.title());
        let max_inner = (avail - 2 * 16 - 2 * pad).max(0);
        let title_w = tf.text_width(el.props.text.as_deref().unwrap_or(""));
        let mut inner_w = title_w.min(max_inner);
        for c in &w.children {
            inner_w = inner_w.max(self.measure(c, max_inner).0.min(max_inner));
        }
        let mut h = tf.line_height();
        for c in &w.children {
            h += gap + self.measure(c, inner_w).1;
        }
        (inner_w + 2 * pad, h + 2 * pad)
    }

    /// Widths of a row's children within `inner` lu.
    fn row_widths(&self, w: &W, inner: i32) -> Vec<i32> {
        let gap = self.gap(w.el);
        let n = w.children.len() as i32;
        let mut widths: Vec<i32> = w
            .children
            .iter()
            .map(|c| if self.grow(c.el, Axis::H) > 0 && c.el.props.width.is_none() { 0 } else { self.measure(c, inner).0.min(inner.max(0)) })
            .collect();
        let total_grow: i32 = w.children.iter().map(|c| if c.el.props.width.is_none() { self.grow(c.el, Axis::H) as i32 } else { 0 }).sum();
        if total_grow > 0 {
            let left = (inner - widths.iter().sum::<i32>() - gap * (n - 1).max(0)).max(0);
            let mut given = 0;
            let mut seen = 0;
            for (i, c) in w.children.iter().enumerate() {
                let g = if c.el.props.width.is_none() { self.grow(c.el, Axis::H) as i32 } else { 0 };
                if g == 0 {
                    continue;
                }
                seen += g;
                let share = if seen == total_grow { left - given } else { left * g / total_grow };
                widths[i] += share;
                given += share;
            }
        }
        widths
    }

    /// Place `w` in `r` (absolute) and lay out its children.
    fn arrange(&self, w: &mut W, r: Rect) {
        w.rect = r;
        let el = w.el;
        let p = &el.props;
        let t = self.theme;
        let (px, py) = (t.px(), t.py());
        match el.kind {
            Kind::Label => {
                let f = self.face(el);
                w.lines = text::wrap(&f, p.text.as_deref().unwrap_or(""), if p.wrap.unwrap_or(true) { Some(r.w) } else { None });
            }
            Kind::List => {
                let mut y = r.y;
                let inner = r.w - 2 * px;
                for c in w.children.iter_mut() {
                    let (_, ch) = self.measure(c, inner);
                    let row = Rect::new(r.x, y, r.w, ch + 2 * py);
                    w.parts.push(row);
                    self.arrange(c, Rect::new(r.x + px, y + py, inner, ch));
                    y += ch + 2 * py;
                }
            }
            Kind::Menu => {
                let f = self.face(el);
                let rh = f.line_height() + 2 * py;
                for i in 0..p.items.len() {
                    w.parts.push(Rect::new(r.x, r.y + i as i32 * rh, r.w, rh));
                }
            }
            Kind::Tabs => {
                let f = self.face(el);
                let hh = f.line_height() + 2 * py;
                let mut x = r.x;
                for s in &p.items {
                    let tw = f.text_width(s) + 2 * px;
                    w.parts.push(Rect::new(x, r.y, tw, hh));
                    x += tw;
                }
                if let Some(c) = w.children.first_mut() {
                    self.arrange(c, Rect::new(r.x, r.y + hh + 1, r.w, r.h - hh - 1));
                }
            }
            Kind::ScrollView => {
                let bar = t.scrollbar as i32;
                let scroll = self.state(&w.key).map_or(0, |s| s.scroll);
                if let Some(c) = w.children.first_mut() {
                    let (_, ch) = self.measure(c, r.w - bar);
                    let max_scroll = (ch - r.h).max(0);
                    let s = scroll.min(max_scroll);
                    w.inner = Rect::new(r.x, r.y - s, r.w - bar, ch);
                    let inner = w.inner;
                    self.arrange(c, inner);
                }
            }
            Kind::Dialog => {
                let (cw, ch) = self.card_size(w, r.w);
                let card = Rect::new(r.x + (r.w - cw) / 2, r.y + ((r.h - ch) / 2).max(0), cw, ch);
                w.inner = card;
                let pad = self.pad(el);
                let gap = self.gap(el);
                let tf = face_of(Font::Ui, t.title());
                let mut y = card.y + pad + tf.line_height();
                let inner_w = card.w - 2 * pad;
                for c in w.children.iter_mut() {
                    y += gap;
                    let (_, chh) = self.measure(c, inner_w);
                    self.arrange(c, Rect::new(card.x + pad, y, inner_w, chh));
                    y += chh;
                }
            }
            Kind::Row => {
                let pad = self.pad(el);
                let gap = self.gap(el);
                let inner = Rect::new(r.x + pad, r.y + pad, r.w - 2 * pad, r.h - 2 * pad);
                let widths = self.row_widths(w, inner.w);
                let mut x = inner.x;
                for (c, cw) in w.children.iter_mut().zip(widths) {
                    let (_, mh) = self.measure(c, cw);
                    let mut align = if c.el.props.align != Align::Auto { c.el.props.align } else { p.align };
                    if align == Align::Stretch && c.el.props.height.is_some() {
                        align = Align::Center;
                    }
                    let (y, h) = match align {
                        Align::Stretch => (inner.y, inner.h),
                        Align::Start => (inner.y, mh),
                        Align::End => (inner.y + inner.h - mh, mh),
                        Align::Auto | Align::Center => (inner.y + (inner.h - mh) / 2, mh),
                    };
                    self.arrange(c, Rect::new(x, y, cw, h));
                    x += cw + gap;
                }
            }
            Kind::Column => {
                let pad = self.pad(el);
                let gap = self.gap(el);
                let inner = Rect::new(r.x + pad, r.y + pad, r.w - 2 * pad, r.h - 2 * pad);
                let sizes: Vec<(i32, i32)> = w.children.iter().map(|c| self.measure(c, inner.w)).collect();
                let n = w.children.len() as i32;
                let used: i32 = sizes.iter().map(|s| s.1).sum::<i32>() + gap * (n - 1).max(0);
                let total_grow: i32 = w.children.iter().map(|c| self.grow(c.el, Axis::V) as i32).sum();
                let left = (inner.h - used).max(0);
                let mut y = inner.y;
                let mut given = 0;
                let mut seen = 0;
                for (c, (mw, mh)) in w.children.iter_mut().zip(sizes) {
                    let g = self.grow(c.el, Axis::V) as i32;
                    let mut h = mh;
                    if g > 0 && total_grow > 0 {
                        seen += g;
                        let share = if seen == total_grow { left - given } else { left * g / total_grow };
                        h += share;
                        given += share;
                    }
                    let mut align = if c.el.props.align != Align::Auto { c.el.props.align } else { p.align };
                    if align == Align::Auto && c.el.props.width.is_some() {
                        align = Align::Start;
                    }
                    let cw = mw.min(inner.w);
                    let (x, wid) = match align {
                        Align::Start => (inner.x, cw),
                        Align::Center => (inner.x + (inner.w - cw) / 2, cw),
                        Align::End => (inner.x + inner.w - cw, cw),
                        Align::Auto | Align::Stretch => (inner.x, inner.w),
                    };
                    self.arrange(c, Rect::new(x, y, wid, h));
                    y += h + gap;
                }
            }
            Kind::Board => {
                for c in w.children.iter_mut() {
                    let cp = &c.el.props;
                    let rect = if cp.x.is_some() || cp.y.is_some() {
                        let avail = cp.width.unwrap_or(r.w);
                        let (cw, ch) = self.measure(c, avail);
                        Rect::new(r.x + cp.x.unwrap_or(0), r.y + cp.y.unwrap_or(0), cw, ch)
                    } else {
                        r
                    };
                    self.arrange(c, rect);
                }
            }
            Kind::Stack | Kind::Panel => {
                let pad = self.pad(el);
                let inner = Rect::new(r.x + pad, r.y + pad, r.w - 2 * pad, r.h - 2 * pad);
                for c in w.children.iter_mut() {
                    let align = if c.el.props.align != Align::Auto { c.el.props.align } else { p.align };
                    let rect = match align {
                        Align::Auto | Align::Stretch => inner,
                        a => {
                            let (cw, ch) = self.measure(c, inner.w);
                            let (cw, ch) = (cw.min(inner.w), ch.min(inner.h));
                            let off = |space: i32, size: i32| match a {
                                Align::Start => 0,
                                Align::End => space - size,
                                _ => (space - size) / 2,
                            };
                            Rect::new(inner.x + off(inner.w, cw), inner.y + off(inner.h, ch), cw, ch)
                        }
                    };
                    self.arrange(c, rect);
                }
            }
            Kind::Grid => {
                let pad = self.pad(el);
                let gap = self.gap(el);
                let cols = self.columns(el);
                let inner = Rect::new(r.x + pad, r.y + pad, r.w - 2 * pad, r.h - 2 * pad);
                let col_w = ((inner.w - gap * (cols - 1)) / cols).max(0);
                let mut y = inner.y;
                for chunk in w.children.chunks_mut(cols as usize) {
                    let rh = chunk.iter().map(|c| self.measure(c, col_w).1).max().unwrap_or(0);
                    for (i, c) in chunk.iter_mut().enumerate() {
                        self.arrange(c, Rect::new(inner.x + i as i32 * (col_w + gap), y, col_w, rh));
                    }
                    y += rh + gap;
                }
            }
            Kind::Responsive => {
                if let Some(c) = w.children.first_mut() {
                    self.arrange(c, r);
                }
            }
            Kind::Button
            | Kind::TextField
            | Kind::TextArea
            | Kind::Custom
            | Kind::Checkbox
            | Kind::Toggle
            | Kind::Slider
            | Kind::Image
            | Kind::Canvas
            | Kind::Spacer
            | Kind::Wire
            | Kind::PixelGrid
            | Kind::ColorPicker => {}
        }
    }
}

// ================================================================ paint

struct Painter<'a> {
    theme: &'a Theme,
    states: &'a mut HashMap<String, WidgetState>,
    hit_ids: &'a mut HashMap<String, u32>,
    next_hit: &'a mut u32,
    hits: HashMap<u32, (String, Part)>,
    info: HashMap<String, Info>,
    focus_order: Vec<String>,
    focus: Option<&'a str>,
    pressed: Option<&'a str>,
    hover: Option<(&'a str, Part)>,
    activated: bool,
    caret_on: bool,
    /// Inside this many capturing boards: every element gets a hit rect.
    capture: u32,
}

fn node(key: String, kind: NodeKind, props: Vec<Prop>) -> SceneNode {
    SceneNode::new(key, kind, props)
}

/// Give a custom area's nodes keys unique in the window.
fn prefixed(prefix: &str, n: &SceneNode) -> SceneNode {
    let mut out = n.clone();
    out.key = format!("{prefix}{}", n.key);
    out.children = n.children.iter().map(|c| prefixed(prefix, c)).collect();
    out
}

impl Painter<'_> {
    fn hit(&mut self, node_key: &str, widget: &str, part: Part) -> u32 {
        let id = match self.hit_ids.get(node_key) {
            Some(id) => *id,
            None => {
                let id = *self.next_hit;
                *self.next_hit += 1;
                self.hit_ids.insert(node_key.to_string(), id);
                id
            }
        };
        self.hits.insert(id, (widget.to_string(), part));
        id
    }

    #[allow(clippy::too_many_arguments)]
    fn rect(&self, key: String, r: Rect, o: (i32, i32), fill: u32, radius: u32, border: Option<(u32, u32)>, hit: u32) -> SceneNode {
        let mut props = vec![
            Prop::X(r.x - o.0),
            Prop::Y(r.y - o.1),
            Prop::W(r.w),
            Prop::H(r.h),
            Prop::Fill(fill),
            Prop::Radius(radius.min(u16::MAX as u32) as u16),
        ];
        if let Some((bw, bc)) = border {
            props.push(Prop::BorderW(bw.min(u16::MAX as u32) as u16));
            props.push(Prop::BorderColor(bc));
        }
        if hit != 0 {
            props.push(Prop::HitId(hit));
        }
        node(key, NodeKind::Rect, props)
    }

    fn text(&self, key: String, x: i32, y: i32, s: &str, f: &Face, color: u32) -> SceneNode {
        node(
            key,
            NodeKind::Text,
            vec![
                Prop::X(x),
                Prop::Y(y),
                Prop::Text(clip_text(s)),
                Prop::Font(f.design_font()),
                Prop::Size(f.size),
                Prop::Color(color),
            ],
        )
    }

    fn group(&self, key: String, r: Rect, o: (i32, i32), clip: bool) -> SceneNode {
        let mut props = vec![Prop::X(r.x - o.0), Prop::Y(r.y - o.1), Prop::W(r.w), Prop::H(r.h)];
        if clip {
            props.push(Prop::Clip(true));
        }
        node(key, NodeKind::Group, props)
    }

    fn face(&self, el: &Element) -> Face {
        Layout { theme: self.theme, states: &HashMap::new(), class: SizeClass::Compact }.face(el)
    }

    fn focused(&self, key: &str) -> bool {
        self.focus == Some(key)
    }

    fn hovered(&self, key: &str, part: &Part) -> bool {
        self.pressed.is_none() && self.hover.as_ref().is_some_and(|(k, p)| *k == key && p == part)
    }

    fn focus_border(&self, key: &str, normal: Option<(u32, u32)>) -> Option<(u32, u32)> {
        if self.focused(key) {
            Some((self.theme.border_w.max(1), self.theme.focus))
        } else {
            normal
        }
    }

    fn record(&mut self, w: &W, parent: Option<&str>) {
        let el = w.el;
        let p = &el.props;
        let enabled = p.enabled.unwrap_or(true);
        let f = self.face(el);
        let rows = match el.kind {
            Kind::List => w.children.len(),
            Kind::Tabs | Kind::Menu => p.items.len(),
            _ => 0,
        };
        self.info.insert(
            w.key.clone(),
            Info {
                kind: el.kind,
                id: el.id.clone(),
                rect: w.rect,
                parent: parent.map(String::from),
                handlers: el.handlers.clone(),
                enabled,
                range: (p.min.unwrap_or(0), p.max.unwrap_or(100), p.step.unwrap_or(1)),
                rows,
                text_x: 0,
                text_y: 0,
                face: (f.design_font(), f.size),
                geom: (1, 1, 1, 0),
                secret: p.password.unwrap_or(false),
                capture: el.kind == Kind::Board && p.capture.unwrap_or(false),
                grid: GridInfo::default(),
                picker: [Rect::default(); 3],
            },
        );
        if el.kind.focusable() && enabled {
            self.focus_order.push(w.key.clone());
        }
    }

    /// A vertical scrollbar thumb for a view of `view` lu showing `content` lu scrolled by `scroll`.
    #[allow(clippy::too_many_arguments)]
    fn scrollbar(&mut self, k: &str, r: Rect, local: (i32, i32), view: i32, content: i32, scroll: i32, out: &mut Vec<SceneNode>) {
        let t = self.theme;
        if content <= view || view <= 0 {
            return;
        }
        let bar = t.scrollbar as i32;
        let bh = (view as i64 * view as i64 / content as i64).max(8) as i32;
        let by = r.y + (scroll as i64 * (view - bh) as i64 / (content - view).max(1) as i64) as i32;
        // The whole bar column is a target: the track pages, the thumb drags.
        let track = self.hit(&format!("{k}:track"), k, Part::Track);
        out.push(self.rect(format!("{k}:track"), Rect::new(r.x + r.w - bar, r.y, bar, view), local, 0, 0, None, track));
        let hit = self.hit(&format!("{k}:bar"), k, Part::Thumb);
        let hovered = self.hovered(k, &Part::Thumb);
        out.push(self.rect(format!("{k}:bar"), Rect::new(r.x + r.w - bar, by, bar, bh), local, if hovered { t.text_muted } else { t.border }, (bar / 2) as u32, None, hit));
        if let Some(i) = self.info.get_mut(k) {
            i.geom = (view, content, bh, r.y);
        }
    }

    /// Paint `w` into `out`; `o` is the absolute origin of the enclosing group.
    fn paint(&mut self, w: &W, parent: Option<&str>, o: (i32, i32), out: &mut Vec<SceneNode>) {
        let el = w.el;
        let p = &el.props;
        let t = self.theme;
        let r = w.rect;
        let k = &w.key;
        let enabled = p.enabled.unwrap_or(true);
        let (px, py) = (t.px(), t.py());
        self.record(w, parent);
        if self.capture > 0 && el.kind != Kind::Wire {
            // Inside a capturing board every element can be pointed at.
            let hit = self.hit(&format!("{k}:cap"), k, Part::Main);
            out.push(self.rect(format!("{k}:cap"), r, o, 0, 0, None, hit));
        }
        match el.kind {
            Kind::Label => {
                let f = self.face(el);
                let color = p.color.unwrap_or(if p.style.as_deref() == Some("caption") { t.text_muted } else { t.text });
                let lh = f.line_height();
                for (i, line) in w.lines.iter().enumerate() {
                    out.push(self.text(format!("{k}:l{i}"), r.x - o.0, r.y - o.1 + i as i32 * lh, line, &f, color));
                }
                if el.handler(EventKind::Click).is_some() {
                    let hit = self.hit(&format!("{k}:hit"), k, Part::Main);
                    out.push(self.rect(format!("{k}:hit"), r, o, 0, 0, None, hit));
                }
            }
            Kind::Button => {
                let f = self.face(el);
                let hovered = enabled && self.hovered(k, &Part::Main);
                let (fill, fg) = match (enabled, p.style.as_deref()) {
                    (false, _) => (t.surface, t.text_muted),
                    (true, Some("primary")) => (if hovered { t.accent_hover } else { t.accent }, t.accent_text),
                    (true, Some("danger")) => (t.danger, t.accent_text),
                    (true, Some("flat")) => (if hovered { t.hover } else { 0 }, t.text),
                    _ => (if hovered { t.hover } else { t.button }, t.text),
                };
                let fill = if self.pressed == Some(k.as_str()) { t.surface_alt } else { fill };
                let hit = self.hit(&format!("{k}:bg"), k, Part::Main);
                let border = self.focus_border(k, None);
                out.push(self.rect(format!("{k}:bg"), r, o, fill, t.radius, border, hit));
                // Text wider than the whole button (a grown or fixed-width
                // button in a narrow place) is cut with "..." inside its
                // padding instead of running out; text that only eats into the
                // padding (small icon buttons) is drawn as it is.
                let full = p.text.as_deref().unwrap_or("");
                let cut = if f.text_width(full) > r.w { Some(text::ellipsize(&f, full, if r.w > 2 * px { r.w - 2 * px } else { r.w })) } else { None };
                let s = cut.as_deref().unwrap_or(full);
                let tx = r.x + (r.w - f.text_width(s)) / 2;
                let ty = r.y + (r.h - f.line_height()) / 2;
                out.push(self.text(format!("{k}:text"), tx - o.0, ty - o.1, s, &f, fg));
            }
            Kind::TextField => {
                let f = self.face(el);
                let mut g = self.group(format!("{k}:g"), r, o, true);
                let hit = self.hit(&format!("{k}:bg"), k, Part::Main);
                let border = self.focus_border(k, Some((t.border_w, t.border)));
                let local = (r.x, r.y);
                g.children.push(self.rect(format!("{k}:bg"), r, local, t.surface, t.radius, border, hit));
                let focused = self.focused(k);
                let secret = p.password.unwrap_or(false);
                let st = self.states.entry(k.clone()).or_default();
                st.cursor = st.cursor.min(st.text.chars().count());
                let shown = if secret { mask(&st.text) } else { st.text.clone() };
                let caret_x = text::prefix_width(&f, &shown, st.cursor);
                let inner_w = (r.w - 2 * px - 1).max(0);
                if caret_x - st.scroll > inner_w {
                    st.scroll = caret_x - inner_w;
                }
                if caret_x < st.scroll {
                    st.scroll = caret_x;
                }
                let full = f.text_width(&shown);
                st.scroll = st.scroll.min((full - inner_w).max(0)).max(0);
                st.follow = false;
                let (sx, value) = (st.scroll, shown);
                let sel = st.selection();
                let ty = (r.h - f.line_height()) / 2;
                if let Some(i) = self.info.get_mut(k) {
                    i.text_x = r.x + px - sx;
                    i.text_y = r.y + ty;
                }
                if let (Some((a, b)), true) = (sel, focused) {
                    let (x0, x1) = (text::prefix_width(&f, &value, a), text::prefix_width(&f, &value, b));
                    let sr = Rect::new(r.x + px - sx + x0, r.y + ty, x1 - x0, f.line_height());
                    g.children.push(self.rect(format!("{k}:sel"), sr, local, t.selection, 0, None, 0));
                }
                if value.is_empty() {
                    let ph = p.placeholder.as_deref().unwrap_or("");
                    g.children.push(self.text(format!("{k}:text"), px, ty, ph, &f, t.text_muted));
                } else {
                    g.children.push(self.text(format!("{k}:text"), px - sx, ty, &value, &f, if enabled { t.text } else { t.text_muted }));
                }
                if focused && self.activated && self.caret_on {
                    let cr = Rect::new(r.x + px - sx + caret_x, r.y + ty, 1, f.line_height());
                    g.children.push(self.rect(format!("{k}:caret"), cr, local, t.text, 0, None, 0));
                }
                out.push(g);
            }
            Kind::TextArea => self.paint_text_area(w, o, out),
            Kind::Custom => {
                let mut g = self.group(format!("{k}:g"), r, o, true);
                let hit = self.hit(&format!("{k}:hit"), k, Part::Main);
                g.children.push(self.rect(format!("{k}:hit"), r, (r.x, r.y), 0, 0, None, hit));
                let prefix = format!("{k}:n:");
                for n in &el.nodes {
                    g.children.push(prefixed(&prefix, n));
                }
                out.push(g);
            }
            Kind::Checkbox | Kind::Toggle => {
                let f = self.face(el);
                let lh = f.line_height();
                let hit = self.hit(&format!("{k}:hit"), k, Part::Main);
                out.push(self.rect(format!("{k}:hit"), r, o, 0, 0, None, hit));
                let on = self.states.get(k).is_some_and(|s| s.checked);
                let by = r.y + (r.h - lh) / 2;
                let label_x;
                if el.kind == Kind::Checkbox {
                    let b = Rect::new(r.x, by, lh, lh);
                    let hovered = enabled && self.hovered(k, &Part::Main);
                    let border = self.focus_border(k, Some((t.border_w, if on || hovered { t.accent } else { t.border })));
                    out.push(self.rect(format!("{k}:box"), b, o, if on { t.accent } else { t.surface }, 2, border, 0));
                    if on {
                        let s = lh as f32 / 12.0;
                        let pt = |x: f32, y: f32| Point { x: b.x - o.0 + (x * s) as i32, y: b.y - o.1 + (y * s) as i32 };
                        out.push(node(
                            format!("{k}:check"),
                            NodeKind::Line,
                            vec![
                                Prop::Points(vec![pt(3.0, 6.0), pt(5.0, 9.0), pt(9.0, 3.0)]),
                                Prop::Width((2.0 * s).max(1.0) as u16),
                                Prop::Color(t.accent_text),
                            ],
                        ));
                    }
                    label_x = b.x + lh + 6;
                } else {
                    let tr = Rect::new(r.x, by, 2 * lh, lh);
                    let border = self.focus_border(k, None);
                    out.push(self.rect(format!("{k}:track"), tr, o, if on { t.accent } else { t.track }, (lh / 2) as u32, border, 0));
                    let ks = lh - 4;
                    let kx = if on { tr.x + tr.w - ks - 2 } else { tr.x + 2 };
                    out.push(self.rect(format!("{k}:knob"), Rect::new(kx, tr.y + 2, ks, ks), o, t.accent_text, (ks / 2) as u32, None, 0));
                    label_x = tr.x + tr.w + 6;
                }
                let s = p.text.as_deref().unwrap_or("");
                if !s.is_empty() {
                    let color = if enabled { t.text } else { t.text_muted };
                    out.push(self.text(format!("{k}:text"), label_x - o.0, r.y + (r.h - lh) / 2 - o.1, s, &f, color));
                }
            }
            Kind::Slider => {
                let (min, max) = (p.min.unwrap_or(0), p.max.unwrap_or(100));
                let v = self.states.get(k).map_or(min, |s| s.value);
                let hit = self.hit(&format!("{k}:hit"), k, Part::Main);
                out.push(self.rect(format!("{k}:hit"), r, o, 0, 0, None, hit));
                let cy = r.y + r.h / 2;
                let span = (r.w - 10).max(0);
                let frac = if max == min { 0 } else { ((v - min) as i64 * span as i64 / (max - min) as i64) as i32 };
                let kx = r.x + 5 + frac;
                out.push(self.rect(format!("{k}:track"), Rect::new(r.x, cy - 2, r.w, 4), o, t.track, 2, None, 0));
                out.push(self.rect(format!("{k}:fill"), Rect::new(r.x, cy - 2, kx - r.x, 4), o, if enabled { t.accent } else { t.text_muted }, 2, None, 0));
                let border = self.focus_border(k, None);
                out.push(self.rect(format!("{k}:knob"), Rect::new(kx - 5, cy - 5, 10, 10), o, t.accent_text, 5, border, 0));
            }
            Kind::List => {
                let mut g = self.group(format!("{k}:g"), r, o, false);
                let border = self.focus_border(k, None);
                if border.is_some() {
                    g.children.push(self.rect(format!("{k}:focus"), r, (r.x, r.y), 0, t.radius, border, 0));
                }
                let sel = self.states.get(k).and_then(|s| s.selected);
                for (i, c) in w.children.iter().enumerate() {
                    let row = w.parts[i];
                    let mut rg = self.group(format!("{}:row", c.key), row, (r.x, r.y), false);
                    let hit = self.hit(&format!("{}:rowbg", c.key), k, Part::Row(i));
                    let fill = if sel == Some(i) {
                        t.surface_alt
                    } else if enabled && self.hovered(k, &Part::Row(i)) {
                        t.row_hover
                    } else {
                        0
                    };
                    rg.children.push(self.rect(format!("{}:rowbg", c.key), row, (row.x, row.y), fill, t.radius, None, hit));
                    self.paint(c, Some(k), (row.x, row.y), &mut rg.children);
                    g.children.push(rg);
                }
                out.push(g);
            }
            Kind::Menu => {
                let f = self.face(el);
                let mut g = self.group(format!("{k}:g"), r, o, false);
                let local = (r.x, r.y);
                let border = self.focus_border(k, Some((t.border_w, t.border)));
                g.children.push(self.rect(format!("{k}:bg"), r, local, t.surface, t.radius, border, 0));
                let sel = self.states.get(k).and_then(|s| s.selected);
                for (i, item) in p.items.iter().enumerate() {
                    let row = w.parts[i];
                    let hit = self.hit(&format!("{k}:i{i}"), k, Part::Row(i));
                    let hovered = enabled && self.hovered(k, &Part::Row(i));
                    let fill = if sel == Some(i) || hovered { t.surface_alt } else { 0 };
                    g.children.push(self.rect(format!("{k}:i{i}"), row, local, fill, 0, None, hit));
                    g.children.push(self.text(format!("{k}:t{i}"), row.x + px - r.x, row.y + py - r.y, item, &f, t.text));
                }
                out.push(g);
            }
            Kind::Tabs => {
                let f = self.face(el);
                let mut g = self.group(format!("{k}:g"), r, o, false);
                let local = (r.x, r.y);
                let sel = self.states.get(k).and_then(|s| s.selected).unwrap_or(0);
                for (i, title) in p.items.iter().enumerate() {
                    let tr = w.parts[i];
                    let hit = self.hit(&format!("{k}:tab{i}"), k, Part::Row(i));
                    let border = if i == sel { self.focus_border(k, None) } else { None };
                    let hovered = i != sel && self.hovered(k, &Part::Row(i));
                    let fill = if i == sel {
                        t.surface
                    } else if hovered {
                        t.row_hover
                    } else {
                        0
                    };
                    g.children.push(self.rect(format!("{k}:tab{i}"), tr, local, fill, 0, border, hit));
                    let color = if i == sel || hovered { t.text } else { t.text_muted };
                    g.children.push(self.text(format!("{k}:tt{i}"), tr.x + px - r.x, tr.y + py - r.y, title, &f, color));
                }
                let hh = f.line_height() + 2 * py;
                g.children.push(self.rect(format!("{k}:div"), Rect::new(r.x, r.y + hh, r.w, 1), local, t.border, 0, None, 0));
                if let Some(tr) = w.parts.get(sel) {
                    g.children.push(self.rect(format!("{k}:ul"), Rect::new(tr.x, tr.y + hh - 2, tr.w, 2), local, t.accent, 0, None, 0));
                }
                for c in &w.children {
                    self.paint(c, Some(k), local, &mut g.children);
                }
                out.push(g);
            }
            Kind::ScrollView => {
                let mut g = self.group(format!("{k}:g"), r, o, true);
                let local = (r.x, r.y);
                let hit = self.hit(&format!("{k}:hit"), k, Part::Main);
                g.children.push(self.rect(format!("{k}:hit"), r, local, 0, 0, None, hit));
                let inner = w.inner;
                let mut content = self.group(format!("{k}:content"), inner, local, false);
                for c in &w.children {
                    self.paint(c, Some(k), (inner.x, inner.y), &mut content.children);
                }
                g.children.push(content);
                if inner.h > r.h && r.h > 0 {
                    let s = r.y - inner.y;
                    self.scrollbar(k, r, local, r.h, inner.h, s, &mut g.children);
                }
                // Keep the stored offset within range (the content may have shrunk).
                let s = (r.y - inner.y).max(0);
                self.states.entry(k.clone()).or_default().scroll = s;
                out.push(g);
            }
            Kind::Dialog => {
                let mut g = self.group(format!("{k}:g"), r, o, false);
                let local = (r.x, r.y);
                let hit = self.hit(&format!("{k}:scrim"), k, Part::Scrim);
                g.children.push(self.rect(format!("{k}:scrim"), r, local, t.scrim, 0, None, hit));
                let card = w.inner;
                let mut cg = self.group(format!("{k}:card"), card, local, false);
                let cl = (card.x, card.y);
                cg.children.push(self.rect(format!("{k}:cardbg"), card, cl, t.surface, t.radius * 2, Some((t.border_w, t.border)), 0));
                let tf = face_of(Font::Ui, t.title());
                let pad = el.props.padding.unwrap_or(t.padding as i32);
                cg.children.push(self.text(format!("{k}:title"), pad, pad, p.text.as_deref().unwrap_or(""), &tf, t.text));
                for c in &w.children {
                    self.paint(c, Some(k), cl, &mut cg.children);
                }
                g.children.push(cg);
                out.push(g);
            }
            Kind::Image => {
                let mut props = vec![
                    Prop::X(r.x - o.0),
                    Prop::Y(r.y - o.1),
                    Prop::W(r.w),
                    Prop::H(r.h),
                    Prop::Resource(p.resource.unwrap_or(0)),
                ];
                if el.handler(EventKind::Click).is_some() {
                    props.push(Prop::HitId(self.hit(&format!("{k}:img"), k, Part::Main)));
                }
                out.push(node(format!("{k}:img"), NodeKind::Image, props));
            }
            Kind::Canvas => {
                let mut props = vec![
                    Prop::X(r.x - o.0),
                    Prop::Y(r.y - o.1),
                    Prop::W(r.w),
                    Prop::H(r.h),
                    Prop::Resource(p.resource.unwrap_or(0)),
                    Prop::Scale(p.scale.unwrap_or(1).clamp(1, 8)),
                ];
                if el.handler(EventKind::Click).is_some() {
                    props.push(Prop::HitId(self.hit(&format!("{k}:canvas"), k, Part::Main)));
                }
                out.push(node(format!("{k}:canvas"), NodeKind::Canvas, props));
            }
            Kind::Row | Kind::Column | Kind::Stack | Kind::Grid => {
                let mut g = self.group(format!("{k}:g"), r, o, false);
                for c in &w.children {
                    self.paint(c, Some(k), (r.x, r.y), &mut g.children);
                }
                out.push(g);
            }
            Kind::Responsive => {
                for c in &w.children {
                    self.paint(c, Some(k), o, out);
                }
            }
            Kind::Board => {
                let mut g = self.group(format!("{k}:g"), r, o, true);
                let local = (r.x, r.y);
                let hit = self.hit(&format!("{k}:bg"), k, Part::Main);
                let border = match (p.border_w, p.border_color) {
                    (Some(bw), Some(bc)) if bw > 0 => Some((bw as u32, bc)),
                    _ => None,
                };
                g.children.push(self.rect(format!("{k}:bg"), r, local, p.fill.unwrap_or(0), p.radius.unwrap_or(0) as u32, border, hit));
                let capture = p.capture.unwrap_or(false);
                if capture {
                    self.capture += 1;
                }
                for c in &w.children {
                    self.paint(c, Some(k), local, &mut g.children);
                }
                if capture {
                    self.capture -= 1;
                }
                out.push(g);
            }
            Kind::Panel => {
                let mut g = self.group(format!("{k}:g"), r, o, false);
                let local = (r.x, r.y);
                let hit = self.hit(&format!("{k}:bg"), k, Part::Main);
                let border = match (p.border_w, p.border_color) {
                    (Some(bw), Some(bc)) if bw > 0 => Some((bw as u32, bc)),
                    _ => None,
                };
                let fill = p.fill.unwrap_or(t.surface);
                let radius = p.radius.map_or(t.radius, u32::from);
                g.children.push(self.rect(format!("{k}:bg"), r, local, fill, radius, border, hit));
                for c in &w.children {
                    self.paint(c, Some(k), local, &mut g.children);
                }
                out.push(g);
            }
            Kind::Wire => {
                if p.points.len() >= 2 {
                    let pts: Vec<Point> = p.points.iter().take(256).map(|[x, y]| Point { x: r.x - o.0 + x, y: r.y - o.1 + y }).collect();
                    out.push(node(
                        format!("{k}:line"),
                        NodeKind::Line,
                        vec![Prop::Points(pts), Prop::Width(p.stroke.unwrap_or(1).max(1)), Prop::Color(p.color.unwrap_or(t.text))],
                    ));
                }
            }
            Kind::PixelGrid => self.paint_pixel_grid(w, o, out),
            Kind::ColorPicker => self.paint_color_picker(w, o, out),
            Kind::Spacer => {}
        }
    }

    /// A pixel grid: runs of equal texels as rects, the line tool's preview,
    /// hatched locked texels and gridlines.
    fn paint_pixel_grid(&mut self, w: &W, o: (i32, i32), out: &mut Vec<SceneNode>) {
        let el = w.el;
        let p = &el.props;
        let t = self.theme;
        let r = w.rect;
        let k = &w.key;
        let (cols, rows) = (p.cols.unwrap_or(16).max(1) as i32, p.rows.unwrap_or(16).max(1) as i32);
        let zoom = match p.zoom {
            Some(z) => z.max(1) as i32,
            None => (r.w / cols).min(r.h / rows).max(1),
        };
        let palette: Vec<u32> = if p.palette.is_empty() { DEFAULT_PALETTE.to_vec() } else { p.palette.iter().take(16).copied().collect() };
        let color_of = |i: u8| palette.get(i as usize).copied().unwrap_or(0) << 8 | 0xff;
        let local = (r.x, r.y);
        let mut g = self.group(format!("{k}:g"), r, o, true);
        // The texel area (the layout may give the grid more room than it fills).
        let area = Rect::new(r.x, r.y, cols * zoom, rows * zoom);
        let hit = self.hit(&format!("{k}:bg"), k, Part::Main);
        let border = self.focus_border(k, Some((t.border_w, t.border)));
        g.children.push(self.rect(format!("{k}:bg"), area, local, t.surface, 0, border, hit));
        let st = self.states.entry(k.clone()).or_default();
        let n = (cols * rows) as usize;
        if st.pixels.len() != n {
            st.pixels.resize(n, 0);
        }
        let pixels = st.pixels.clone();
        let preview: Option<(usize, usize)> = st.anchor.map(|a| (a, st.cursor));
        let selected = st.selected;
        // Runs of equal colour per row.
        for y in 0..rows {
            let mut x = 0;
            while x < cols {
                let i = (y * cols + x) as usize;
                let c = pixels[i];
                let mut end = x + 1;
                while end < cols && pixels[(y * cols + end) as usize] == c {
                    end += 1;
                }
                let rr = Rect::new(r.x + x * zoom, r.y + y * zoom, (end - x) * zoom, zoom);
                g.children.push(self.rect(format!("{k}:y{y}x{x}"), rr, local, color_of(c), 0, None, 0));
                x = end;
            }
        }
        // The line tool's preview in the paint colour.
        if let Some((a, b)) = preview {
            let color = p.color.map(|c| c.min(255) as u8).or(selected.map(|s| s as u8)).unwrap_or(0).min(palette.len().saturating_sub(1) as u8);
            for (i, (x, y)) in bresenham((a as i32 % cols, a as i32 / cols), (b as i32 % cols, b as i32 / cols)).into_iter().enumerate() {
                let rr = Rect::new(r.x + x * zoom, r.y + y * zoom, zoom, zoom);
                g.children.push(self.rect(format!("{k}:pv{i}"), rr, local, color_of(color), 0, None, 0));
            }
        }
        // Locked texels: a translucent wash and a diagonal hatch per run.
        for y in 0..rows {
            let mut x = 0;
            while x < cols {
                if p.mask.get((y * cols + x) as usize).is_some_and(|m| *m != 0) {
                    let mut end = x + 1;
                    while end < cols && p.mask.get((y * cols + end) as usize).is_some_and(|m| *m != 0) {
                        end += 1;
                    }
                    let rr = Rect::new(r.x + x * zoom, r.y + y * zoom, (end - x) * zoom, zoom);
                    g.children.push(self.rect(format!("{k}:m{y}x{x}"), rr, local, t.masked, 0, None, 0));
                    if zoom >= 3 {
                        let mut pts = Vec::new();
                        for i in 0..(end - x).min(127) {
                            pts.push(Point { x: rr.x - r.x + i * zoom, y: rr.y - r.y + zoom - 1 });
                            pts.push(Point { x: rr.x - r.x + (i + 1) * zoom - 1, y: rr.y - r.y });
                        }
                        g.children.push(node(format!("{k}:mh{y}x{x}"), NodeKind::Line, vec![Prop::Points(pts), Prop::Width(1), Prop::Color(t.bg)]));
                    }
                    x = end;
                } else {
                    x += 1;
                }
            }
        }
        if p.grid.unwrap_or(true) && zoom >= 4 {
            for x in 1..cols {
                g.children.push(self.rect(format!("{k}:gv{x}"), Rect::new(r.x + x * zoom, r.y, 1, rows * zoom), local, t.grid_line, 0, None, 0));
            }
            for y in 1..rows {
                g.children.push(self.rect(format!("{k}:gh{y}"), Rect::new(r.x, r.y + y * zoom, cols * zoom, 1), local, t.grid_line, 0, None, 0));
            }
        }
        if let Some(i) = self.info.get_mut(k) {
            i.grid = GridInfo {
                cols,
                rows,
                zoom,
                mask: p.mask.clone(),
                tool: Tool::from_name(p.tool.as_deref()),
                mirror: p.mirror_h.unwrap_or(false),
                color: p.color.map(|c| c.min(255) as u8),
                colors: palette.len() as u8,
            };
        }
        out.push(g);
    }

    /// A colour picker: the SV square (cells), the hue strip (bands), their
    /// markers, and the hex field with a swatch.
    fn paint_color_picker(&mut self, w: &W, o: (i32, i32), out: &mut Vec<SceneNode>) {
        let el = w.el;
        let p = &el.props;
        let t = self.theme;
        let r = w.rect;
        let k = &w.key;
        let enabled = p.enabled.unwrap_or(true);
        let f = self.face(el);
        let lh = f.line_height();
        let (px, py) = (t.px(), t.py());
        let field_h = lh + 2 * py;
        let sv = Rect::new(r.x, r.y, r.w - HUE_STRIP_W - PICKER_GAP, r.h - field_h - PICKER_GAP);
        let hue = Rect::new(r.x + r.w - HUE_STRIP_W, r.y, HUE_STRIP_W, sv.h);
        let field = Rect::new(r.x, r.y + r.h - field_h, r.w, field_h);
        let local = (r.x, r.y);
        let mut g = self.group(format!("{k}:g"), r, o, true);
        let st = self.states.entry(k.clone()).or_default();
        let (hsv, value, text) = (st.hsv, st.value, st.text.clone());
        // The SV square at the current hue.
        let sv_hit = self.hit(&format!("{k}:sv"), k, Part::Sv);
        g.children.push(self.rect(format!("{k}:sv"), sv, local, 0, 0, None, sv_hit));
        for j in 0..SV_CELLS {
            for i in 0..SV_CELLS {
                let cell = Rect::new(
                    sv.x + i * sv.w / SV_CELLS,
                    sv.y + j * sv.h / SV_CELLS,
                    (i + 1) * sv.w / SV_CELLS - i * sv.w / SV_CELLS,
                    (j + 1) * sv.h / SV_CELLS - j * sv.h / SV_CELLS,
                );
                let c = hsv_to_rgb(hsv[0], i * 255 / (SV_CELLS - 1), 255 - j * 255 / (SV_CELLS - 1)) as u32;
                g.children.push(self.rect(format!("{k}:c{i}_{j}"), cell, local, c << 8 | 0xff, 0, None, 0));
            }
        }
        let hue_hit = self.hit(&format!("{k}:hue"), k, Part::Hue);
        g.children.push(self.rect(format!("{k}:hue"), hue, local, 0, 0, None, hue_hit));
        for i in 0..HUE_BANDS {
            let band = Rect::new(hue.x, hue.y + i * hue.h / HUE_BANDS, hue.w, (i + 1) * hue.h / HUE_BANDS - i * hue.h / HUE_BANDS);
            let c = hsv_to_rgb(i * 360 / HUE_BANDS, 255, 255) as u32;
            g.children.push(self.rect(format!("{k}:h{i}"), band, local, c << 8 | 0xff, 0, None, 0));
        }
        // Markers.
        let my = hue.y + (hsv[0] as i64 * (hue.h.max(2) - 1) as i64 / 359) as i32;
        g.children.push(self.rect(format!("{k}:hm"), Rect::new(hue.x - 1, my - 1, hue.w + 2, 3), local, 0, 0, Some((1, t.text)), 0));
        let mx = sv.x + (hsv[1] as i64 * (sv.w.max(2) - 1) as i64 / 255) as i32;
        let mv = sv.y + ((255 - hsv[2]) as i64 * (sv.h.max(2) - 1) as i64 / 255) as i32;
        g.children.push(self.rect(format!("{k}:svm"), Rect::new(mx - 3, mv - 3, 7, 7), local, 0, 3, Some((1, t.text)), 0));
        g.children.push(self.rect(format!("{k}:svm2"), Rect::new(mx - 2, mv - 2, 5, 5), local, 0, 2, Some((1, t.bg)), 0));
        // The hex field with a swatch.
        let focused = self.focused(k);
        let field_hit = self.hit(&format!("{k}:field"), k, Part::Field);
        let border = self.focus_border(k, Some((t.border_w, t.border)));
        g.children.push(self.rect(format!("{k}:fbg"), field, local, t.surface, t.radius, border, field_hit));
        let sw = lh;
        g.children.push(self.rect(
            format!("{k}:swatch"),
            Rect::new(field.x + px / 2, field.y + py, sw, sw),
            local,
            (value as u32) << 8 | 0xff,
            2,
            Some((1, t.border)),
            0,
        ));
        let shown = if focused { text.clone() } else { hex_of(value) };
        let tx = field.x + px / 2 + sw + px / 2;
        let ty = field.y + (field.h - lh) / 2;
        g.children.push(self.text(format!("{k}:ftext"), tx - r.x, ty - r.y, &shown, &f, if enabled { t.text } else { t.text_muted }));
        if focused && self.activated && self.caret_on {
            let cx = tx + f.text_width(&shown);
            g.children.push(self.rect(format!("{k}:caret"), Rect::new(cx, ty, 1, lh), local, t.text, 0, None, 0));
        }
        if let Some(i) = self.info.get_mut(k) {
            i.picker = [hue, sv, field];
            i.text_x = tx;
            i.text_y = ty;
        }
        out.push(g);
    }

    /// A text area: visible lines only, a line-number gutter, the selection,
    /// the caret and a scrollbar.
    fn paint_text_area(&mut self, w: &W, o: (i32, i32), out: &mut Vec<SceneNode>) {
        let el = w.el;
        let p = &el.props;
        let t = self.theme;
        let r = w.rect;
        let k = &w.key;
        let enabled = p.enabled.unwrap_or(true);
        let (px, py) = (t.px(), t.py());
        let f = self.face(el);
        let lh = f.line_height().max(1);
        let focused = self.focused(k);
        let local = (r.x, r.y);
        let mut g = self.group(format!("{k}:g"), r, o, true);
        let hit = self.hit(&format!("{k}:bg"), k, Part::Main);
        let border = self.focus_border(k, Some((t.border_w, t.border)));
        g.children.push(self.rect(format!("{k}:bg"), r, local, t.surface, t.radius, border, hit));

        let st = self.states.entry(k.clone()).or_default();
        let n_chars = st.text.chars().count();
        st.cursor = st.cursor.min(n_chars);
        let n_lines = st.text.split('\n').count().max(1);
        let digits = n_lines.to_string().len().max(2) as i32;
        let gutter = if p.line_numbers.unwrap_or(false) { digits * f.advance('0') + 2 * px } else { 0 };
        let bar = t.scrollbar as i32;
        let view_h = (r.h - 2 * py).max(lh);
        let view_w = (r.w - gutter - 2 * px - bar).max(1);
        let content_h = n_lines as i32 * lh;
        let (cl, cc) = line_col(&st.text, st.cursor);
        let caret_line: &str = st.text.split('\n').nth(cl).unwrap_or("");
        let caret_x = text::prefix_width(&f, caret_line, cc);
        // Keep the caret in view after an edit or a caret move.
        if st.follow {
            let cy = cl as i32 * lh;
            if cy < st.scroll {
                st.scroll = cy;
            }
            if cy + lh > st.scroll + view_h {
                st.scroll = cy + lh - view_h;
            }
            if caret_x < st.scroll_x {
                st.scroll_x = (caret_x - view_w / 4).max(0);
            }
            if caret_x > st.scroll_x + view_w - 2 {
                st.scroll_x = caret_x - view_w + view_w / 4;
            }
            st.follow = false;
        }
        st.scroll = st.scroll.clamp(0, (content_h - view_h).max(0));
        st.scroll_x = st.scroll_x.max(0);
        let (sy, sx) = (st.scroll, st.scroll_x);
        let first = (sy / lh) as usize;
        let rows = (view_h / lh + 2) as usize;
        let sel = st.selection();
        let text_x = r.x + gutter + px - sx;
        let text_y = r.y + py - sy;

        // Visible lines (with their character offsets) and the selection rects.
        let mut visible: Vec<(usize, usize, String)> = Vec::new();
        let mut start = 0usize;
        for (n, l) in st.text.split('\n').enumerate() {
            let len = l.chars().count();
            if n >= first && n < first + rows {
                visible.push((n, start, l.to_string()));
            }
            if n >= first + rows {
                break;
            }
            start += len + 1;
        }
        let empty = st.text.is_empty();
        let mut view = self.group(format!("{k}:view"), Rect::new(r.x + gutter + px, r.y + py, view_w, view_h), local, true);
        let vl = (r.x + gutter + px, r.y + py);
        if let (Some((a, b)), true) = (sel, focused) {
            for (slot, (n, s0, l)) in visible.iter().enumerate() {
                let len = l.chars().count();
                let (la, lb) = (a.max(*s0), b.min(s0 + len + 1));
                if la >= lb {
                    continue;
                }
                let x0 = text::prefix_width(&f, l, la - s0);
                let x1 = text::prefix_width(&f, l, (lb - s0).min(len)) + if lb > s0 + len { f.advance(' ') } else { 0 };
                let sr = Rect::new(text_x + x0, text_y + *n as i32 * lh, x1 - x0, lh);
                view.children.push(self.rect(format!("{k}:s{slot}"), sr, vl, t.selection, 0, None, 0));
            }
        }
        let color = if enabled { t.text } else { t.text_muted };
        if empty {
            let ph = p.placeholder.as_deref().unwrap_or("");
            view.children.push(self.text(format!("{k}:l0"), text_x - vl.0, text_y - vl.1, ph, &f, t.text_muted));
        } else {
            for (slot, (n, _, l)) in visible.iter().enumerate() {
                view.children.push(self.text(format!("{k}:l{slot}"), text_x - vl.0, text_y + *n as i32 * lh - vl.1, l, &f, color));
            }
        }
        if focused && self.activated && self.caret_on {
            let cr = Rect::new(text_x + caret_x, text_y + cl as i32 * lh, if f.design_font() == Font::Mono { 2 } else { 1 }, lh);
            view.children.push(self.rect(format!("{k}:caret"), cr, vl, t.text, 0, None, 0));
        }
        g.children.push(view);
        if gutter > 0 {
            g.children.push(self.rect(format!("{k}:gut"), Rect::new(r.x, r.y, gutter, r.h), local, t.bg, 0, None, 0));
            for (slot, (n, _, _)) in visible.iter().enumerate() {
                let s = (n + 1).to_string();
                let nx = r.x + gutter - px / 2 - f.text_width(&s);
                let y = text_y + *n as i32 * lh;
                if y + lh <= r.y || y >= r.y + r.h {
                    continue;
                }
                let c = if *n == cl { t.text } else { t.text_muted };
                g.children.push(self.text(format!("{k}:n{slot}"), nx - r.x, y - r.y, &s, &f, c));
            }
        }
        if let Some(i) = self.info.get_mut(k) {
            i.text_x = text_x;
            i.text_y = text_y;
            i.rows = (view_h / lh).max(1) as usize;
        }
        let sbr = Rect::new(r.x, r.y + py, r.w, view_h);
        self.scrollbar(k, sbr, local, view_h, content_h, sy, &mut g.children);
        out.push(g);
    }
}

trait FaceExt {
    fn design_font(&self) -> Font;
}

impl FaceExt for Face {
    fn design_font(&self) -> Font {
        if self.design.font == Font::Mono.code() {
            Font::Mono
        } else {
            Font::Ui
        }
    }
}
