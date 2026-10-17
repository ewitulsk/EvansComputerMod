//! Elements: the declarative description of a window's widgets.
//!
//! An app describes its window as a tree of [`Element`]s, either rebuilt by a
//! view function on every render (Rust) or kept as a retained tree and
//! replaced when it changes (Python, `.ui.json`). The window keeps the
//! widgets' own state (text being edited, focus, scroll offsets, slider
//! drags) between renders, keyed by the element's `id` or, without one, by
//! its position and kind in the tree.

use crate::app::Ctx;
use crate::diff::SceneNode;
use crate::window::UiEvent;
use ecm_ui_proto::Font;
use std::fmt;
use std::rc::Rc;

/// What a widget is.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Label,
    Button,
    TextField,
    List,
    ScrollView,
    Checkbox,
    Toggle,
    Slider,
    Tabs,
    Menu,
    Dialog,
    Image,
    Canvas,
    Row,
    Column,
    Stack,
    Grid,
    Spacer,
    /// Picks one of up to three children by the window's size class
    /// (compact, medium, wide; missing ones fall back to the previous).
    Responsive,
    /// A multi-line text editor (selection, clipboard keys, scrolling,
    /// optional line numbers).
    TextArea,
    /// An area the app draws itself with display-list nodes
    /// ([`Element::nodes`]); it gets raw pointer, key, text and paste events.
    Custom,
    /// A free-form surface (node editors, designers): children with `x` / `y`
    /// sit at those board coordinates at their own size, others fill the
    /// board. Clips its children. Pointer presses on the board or on a
    /// non-interactive child are grabbed by the board and reported to its
    /// `on_pointer` handler in board coordinates (moves and the release
    /// follow while the button is held); with `capture` every press inside
    /// is grabbed, interactive children included. Takes keyboard focus on a
    /// press and reports keys to `on_key`.
    Board,
    /// A rectangle with `fill`, `radius` and a border, holding its children
    /// like a stack (inside `padding`). Hit-testable, so a board reports
    /// which panel a press landed on.
    Panel,
    /// A polyline (`points`, at most 256, relative to the parent), `stroke`
    /// lu wide in `color`: wires and connectors.
    Wire,
    /// An editable indexed bitmap (card faces, icons): `cols` x `rows`
    /// texels drawn `zoom` lu each (auto-fit without `zoom`) in colours of a
    /// `palette` of up to 16 RGB values. The pointer paints with `tool`
    /// (`pencil`, `fill`, `eyedropper`, `line`) in palette index `color`,
    /// mirrored left-right with `mirror_h`; texels whose `mask` entry is
    /// non-zero are locked and drawn hatched. `change` carries the changed
    /// texels ([`crate::EventValue::Texels`]); the eyedropper emits `select`
    /// with the picked index. The bitmap lives in the widget's state
    /// (`WidgetState::pixels`), read back with `Ctx::pixels`.
    PixelGrid,
    /// An RGB colour picker: a hue strip, a saturation/value square and an
    /// editable `#rrggbb` field. `value` is the colour as `0xRRGGBB`;
    /// `change` fires with every drag and every valid hex edit.
    ColorPicker,
}

impl Kind {
    /// The name used in `.ui.json` files and widget keys.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Label => "label",
            Kind::Button => "button",
            Kind::TextField => "textfield",
            Kind::List => "list",
            Kind::ScrollView => "scroll",
            Kind::Checkbox => "checkbox",
            Kind::Toggle => "toggle",
            Kind::Slider => "slider",
            Kind::Tabs => "tabs",
            Kind::Menu => "menu",
            Kind::Dialog => "dialog",
            Kind::Image => "image",
            Kind::Canvas => "canvas",
            Kind::Row => "row",
            Kind::Column => "column",
            Kind::Stack => "stack",
            Kind::Grid => "grid",
            Kind::Spacer => "spacer",
            Kind::Responsive => "responsive",
            Kind::TextArea => "textarea",
            Kind::Custom => "custom",
            Kind::Board => "board",
            Kind::Panel => "panel",
            Kind::Wire => "wire",
            Kind::PixelGrid => "pixel_grid",
            Kind::ColorPicker => "color_picker",
        }
    }

    pub const ALL: &'static [Kind] = &[
        Kind::Label,
        Kind::Button,
        Kind::TextField,
        Kind::List,
        Kind::ScrollView,
        Kind::Checkbox,
        Kind::Toggle,
        Kind::Slider,
        Kind::Tabs,
        Kind::Menu,
        Kind::Dialog,
        Kind::Image,
        Kind::Canvas,
        Kind::Row,
        Kind::Column,
        Kind::Stack,
        Kind::Grid,
        Kind::Spacer,
        Kind::Responsive,
        Kind::TextArea,
        Kind::Custom,
        Kind::Board,
        Kind::Panel,
        Kind::Wire,
        Kind::PixelGrid,
        Kind::ColorPicker,
    ];

    pub fn from_name(name: &str) -> Option<Kind> {
        let name = match name {
            "scroll_view" | "scrollview" => "scroll",
            "text_field" => "textfield",
            "text_area" => "textarea",
            "pixelgrid" => "pixel_grid",
            "colorpicker" => "color_picker",
            n => n,
        };
        Kind::ALL.iter().copied().find(|k| k.name() == name)
    }

    /// Takes keyboard focus (Tab cycles through these).
    pub fn focusable(self) -> bool {
        matches!(
            self,
            Kind::Button
                | Kind::TextField
                | Kind::List
                | Kind::Checkbox
                | Kind::Toggle
                | Kind::Slider
                | Kind::Tabs
                | Kind::Menu
                | Kind::TextArea
                | Kind::Custom
                | Kind::Board
                | Kind::PixelGrid
                | Kind::ColorPicker
        )
    }

    /// Reacts to presses itself (a board leaves presses on these to them
    /// unless it captures).
    pub fn interactive(self) -> bool {
        self != Kind::Board && self.focusable()
    }
}

/// Cross-axis alignment of children (rows and columns) or of a stack's children.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Auto,
    Start,
    Center,
    End,
    Stretch,
}

impl Align {
    pub fn from_name(s: &str) -> Option<Align> {
        Some(match s {
            "start" => Align::Start,
            "center" => Align::Center,
            "end" => Align::End,
            "stretch" => Align::Stretch,
            _ => return None,
        })
    }
}

/// Which handler an event goes to.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// Buttons.
    Click,
    /// Text fields (every edit), checkboxes, toggles, sliders.
    Change,
    /// Enter in a text field.
    Submit,
    /// Lists, tabs and menus: a row / tab / item was chosen.
    Select,
    /// Lists: a row was double-clicked or Enter was pressed on it.
    Activate,
    /// Custom areas and boards: a press, drag, release or wheel step
    /// ([`crate::EventValue::Pointer`]).
    Pointer,
    /// Custom areas (presses, repeats and releases) and focused boards
    /// (presses and repeats): a key ([`crate::EventValue::Key`]).
    Key,
    /// Custom areas: typed text (`EventValue::Text`).
    Text,
    /// Custom areas: clipboard text asked for with `Ctx::paste_into`.
    Paste,
}

impl EventKind {
    pub fn name(self) -> &'static str {
        match self {
            EventKind::Click => "click",
            EventKind::Change => "change",
            EventKind::Submit => "submit",
            EventKind::Select => "select",
            EventKind::Activate => "activate",
            EventKind::Pointer => "pointer",
            EventKind::Key => "key",
            EventKind::Text => "text",
            EventKind::Paste => "paste",
        }
    }
    pub fn from_prop(key: &str) -> Option<EventKind> {
        Some(match key {
            "on_click" => EventKind::Click,
            "on_change" => EventKind::Change,
            "on_submit" => EventKind::Submit,
            "on_select" => EventKind::Select,
            "on_activate" => EventKind::Activate,
            "on_pointer" => EventKind::Pointer,
            "on_key" => EventKind::Key,
            "on_text" => EventKind::Text,
            "on_paste" => EventKind::Paste,
            _ => return None,
        })
    }
}

/// An event handler.
#[derive(Clone)]
pub enum Handler {
    /// A named handler, registered with `App::handler` (layout files).
    Named(String),
    /// An opaque number the embedding language maps back (Python).
    Ref(i64),
    /// A Rust closure.
    Callback(Rc<dyn Fn(&mut Ctx, &UiEvent)>),
}

impl fmt::Debug for Handler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Handler::Named(n) => write!(f, "Named({n:?})"),
            Handler::Ref(r) => write!(f, "Ref({r})"),
            Handler::Callback(_) => write!(f, "Callback"),
        }
    }
}

/// Widget properties. `None` means "the theme's / widget's default".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Props {
    /// Label and button text, checkbox / toggle label, dialog title. For a
    /// text field, setting it replaces the text being edited.
    pub text: Option<String>,
    pub placeholder: Option<String>,
    pub gap: Option<i32>,
    pub padding: Option<i32>,
    /// Share of the leftover main-axis space in a row or column.
    pub grow: Option<u16>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub align: Align,
    /// Checkbox / toggle state (replaces the widget's state when set).
    pub checked: Option<bool>,
    /// Slider value / range (integers).
    pub value: Option<i32>,
    pub min: Option<i32>,
    pub max: Option<i32>,
    pub step: Option<i32>,
    /// Selected row / tab / item.
    pub selected: Option<usize>,
    /// Tab titles, menu items.
    pub items: Vec<String>,
    /// Grid columns per size class (compact, medium, wide).
    pub columns: Option<[u16; 3]>,
    pub font: Option<Font>,
    pub size: Option<u8>,
    pub color: Option<u32>,
    /// Labels wrap by default.
    pub wrap: Option<bool>,
    /// "primary" or "danger" for buttons; "title", "caption" for labels.
    pub style: Option<String>,
    pub enabled: Option<bool>,
    pub resource: Option<u32>,
    pub scale: Option<u8>,
    /// Text areas: draw a line-number gutter.
    pub line_numbers: Option<bool>,
    /// Text fields: show `*` for every character and refuse to copy.
    pub password: Option<bool>,
    /// Built only in these size classes (compact, medium, wide); None = all.
    pub visible_in: Option<[bool; 3]>,
    /// Position inside a board (board coordinates).
    pub x: Option<i32>,
    pub y: Option<i32>,
    /// Panel / board background.
    pub fill: Option<u32>,
    pub radius: Option<u16>,
    pub border_w: Option<u16>,
    pub border_color: Option<u32>,
    /// Wire points, relative to the parent (at most 256 are drawn).
    pub points: Vec<[i32; 2]>,
    /// Wire width in lu.
    pub stroke: Option<u16>,
    /// Boards: grab every press inside, interactive children included
    /// (designers use this to select widgets in a live preview).
    pub capture: Option<bool>,
    /// Pixel grids: the bitmap's size in texels.
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    /// Pixel grids: lu per texel (None = fit the element's size).
    pub zoom: Option<u16>,
    /// Pixel grids: up to 16 RGB colours (`0xRRGGBB`); empty = the default palette.
    pub palette: Vec<u32>,
    /// Pixel grids: palette indices, `cols * rows` (replaces the state when set).
    pub pixels: Vec<u8>,
    /// Pixel grids: `cols * rows` entries, non-zero = locked (drawn hatched, never painted).
    pub mask: Vec<u8>,
    /// Pixel grids: `pencil` (default), `fill`, `eyedropper` or `line`.
    pub tool: Option<String>,
    /// Pixel grids: paint the mirrored texel too.
    pub mirror_h: Option<bool>,
    /// Pixel grids: draw gridlines (when a texel is at least 4 lu).
    pub grid: Option<bool>,
}

/// One node of an element tree.
#[derive(Clone, Debug)]
pub struct Element {
    pub kind: Kind,
    /// A stable key: keeps the widget's state when its position changes, and
    /// lets handlers find it (`cx.text("entry")`).
    pub id: Option<String>,
    pub props: Props,
    pub children: Vec<Element>,
    pub handlers: Vec<(EventKind, Handler)>,
    /// Custom areas: the nodes to draw, in the area's own coordinates.
    pub nodes: Vec<SceneNode>,
}

impl Element {
    pub fn new(kind: Kind) -> Element {
        Element { kind, id: None, props: Props::default(), children: Vec::new(), handlers: Vec::new(), nodes: Vec::new() }
    }

    pub fn handler(&self, kind: EventKind) -> Option<&Handler> {
        self.handlers.iter().find(|(k, _)| *k == kind).map(|(_, h)| h)
    }

    // ---- builder methods

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
    pub fn text(mut self, t: impl Into<String>) -> Self {
        self.props.text = Some(t.into());
        self
    }
    pub fn placeholder(mut self, t: impl Into<String>) -> Self {
        self.props.placeholder = Some(t.into());
        self
    }
    pub fn gap(mut self, g: i32) -> Self {
        self.props.gap = Some(g);
        self
    }
    pub fn padding(mut self, p: i32) -> Self {
        self.props.padding = Some(p);
        self
    }
    pub fn grow(mut self, g: u16) -> Self {
        self.props.grow = Some(g);
        self
    }
    pub fn width(mut self, w: i32) -> Self {
        self.props.width = Some(w);
        self
    }
    pub fn height(mut self, h: i32) -> Self {
        self.props.height = Some(h);
        self
    }
    pub fn align(mut self, a: Align) -> Self {
        self.props.align = a;
        self
    }
    pub fn checked(mut self, c: bool) -> Self {
        self.props.checked = Some(c);
        self
    }
    pub fn value(mut self, v: i32) -> Self {
        self.props.value = Some(v);
        self
    }
    pub fn range(mut self, min: i32, max: i32) -> Self {
        self.props.min = Some(min);
        self.props.max = Some(max);
        self
    }
    pub fn step(mut self, s: i32) -> Self {
        self.props.step = Some(s);
        self
    }
    pub fn selected(mut self, i: usize) -> Self {
        self.props.selected = Some(i);
        self
    }
    pub fn items<S: Into<String>>(mut self, items: impl IntoIterator<Item = S>) -> Self {
        self.props.items = items.into_iter().map(Into::into).collect();
        self
    }
    /// Grid columns for compact, medium and wide windows.
    pub fn columns(mut self, compact: u16, medium: u16, wide: u16) -> Self {
        self.props.columns = Some([compact, medium, wide]);
        self
    }
    pub fn font(mut self, f: Font) -> Self {
        self.props.font = Some(f);
        self
    }
    pub fn size(mut self, s: u8) -> Self {
        self.props.size = Some(s);
        self
    }
    pub fn color(mut self, c: u32) -> Self {
        self.props.color = Some(c);
        self
    }
    pub fn wrap(mut self, w: bool) -> Self {
        self.props.wrap = Some(w);
        self
    }
    pub fn style(mut self, s: impl Into<String>) -> Self {
        self.props.style = Some(s.into());
        self
    }
    pub fn enabled(mut self, e: bool) -> Self {
        self.props.enabled = Some(e);
        self
    }
    /// Text fields: hide the text (`*`) and don't copy it.
    pub fn password(mut self, on: bool) -> Self {
        self.props.password = Some(on);
        self
    }
    /// Text areas: show line numbers.
    pub fn line_numbers(mut self, on: bool) -> Self {
        self.props.line_numbers = Some(on);
        self
    }
    /// Build this element only in the given size classes.
    pub fn visible_in(mut self, compact: bool, medium: bool, wide: bool) -> Self {
        self.props.visible_in = Some([compact, medium, wide]);
        self
    }
    /// Custom areas: the display-list nodes to draw (keys unique within the area).
    pub fn nodes(mut self, nodes: Vec<SceneNode>) -> Self {
        self.nodes = nodes;
        self
    }
    /// Place the element at board coordinates (inside a `board`).
    pub fn at(mut self, x: i32, y: i32) -> Self {
        self.props.x = Some(x);
        self.props.y = Some(y);
        self
    }
    /// Width and height together.
    pub fn sized(self, w: i32, h: i32) -> Self {
        self.width(w).height(h)
    }
    pub fn fill(mut self, c: u32) -> Self {
        self.props.fill = Some(c);
        self
    }
    pub fn radius(mut self, r: u16) -> Self {
        self.props.radius = Some(r);
        self
    }
    pub fn border(mut self, w: u16, c: u32) -> Self {
        self.props.border_w = Some(w);
        self.props.border_color = Some(c);
        self
    }
    pub fn points(mut self, pts: impl IntoIterator<Item = (i32, i32)>) -> Self {
        self.props.points = pts.into_iter().map(|(x, y)| [x, y]).collect();
        self
    }
    pub fn stroke(mut self, w: u16) -> Self {
        self.props.stroke = Some(w);
        self
    }
    pub fn capture(mut self, on: bool) -> Self {
        self.props.capture = Some(on);
        self
    }
    /// Pixel grids: the bitmap's size in texels.
    pub fn texels(mut self, cols: u16, rows: u16) -> Self {
        self.props.cols = Some(cols);
        self.props.rows = Some(rows);
        self
    }
    /// Pixel grids: lu per texel.
    pub fn zoom(mut self, z: u16) -> Self {
        self.props.zoom = Some(z);
        self
    }
    /// Pixel grids: the palette (`0xRRGGBB`, at most 16 are used).
    pub fn palette(mut self, colors: impl IntoIterator<Item = u32>) -> Self {
        self.props.palette = colors.into_iter().take(16).collect();
        self
    }
    /// Pixel grids: the bitmap (palette indices, `cols * rows`); replaces the edited one.
    pub fn pixels(mut self, px: impl IntoIterator<Item = u8>) -> Self {
        self.props.pixels = px.into_iter().collect();
        self
    }
    /// Pixel grids: locked texels (non-zero), `cols * rows`.
    pub fn mask(mut self, m: impl IntoIterator<Item = u8>) -> Self {
        self.props.mask = m.into_iter().collect();
        self
    }
    /// Pixel grids: `pencil`, `fill`, `eyedropper` or `line`.
    pub fn tool(mut self, t: impl Into<String>) -> Self {
        self.props.tool = Some(t.into());
        self
    }
    /// Pixel grids: paint the left-right mirrored texel too.
    pub fn mirror_h(mut self, on: bool) -> Self {
        self.props.mirror_h = Some(on);
        self
    }
    /// Pixel grids: draw gridlines.
    pub fn grid(mut self, on: bool) -> Self {
        self.props.grid = Some(on);
        self
    }
    pub fn child(mut self, c: Element) -> Self {
        self.children.push(c);
        self
    }
    pub fn children(mut self, cs: impl IntoIterator<Item = Element>) -> Self {
        self.children.extend(cs);
        self
    }
    pub fn on(mut self, kind: EventKind, h: Handler) -> Self {
        self.handlers.retain(|(k, _)| *k != kind);
        self.handlers.push((kind, h));
        self
    }
    pub fn on_click(self, f: impl Fn(&mut Ctx, &UiEvent) + 'static) -> Self {
        self.on(EventKind::Click, Handler::Callback(Rc::new(f)))
    }
    pub fn on_change(self, f: impl Fn(&mut Ctx, &UiEvent) + 'static) -> Self {
        self.on(EventKind::Change, Handler::Callback(Rc::new(f)))
    }
    pub fn on_submit(self, f: impl Fn(&mut Ctx, &UiEvent) + 'static) -> Self {
        self.on(EventKind::Submit, Handler::Callback(Rc::new(f)))
    }
    pub fn on_select(self, f: impl Fn(&mut Ctx, &UiEvent) + 'static) -> Self {
        self.on(EventKind::Select, Handler::Callback(Rc::new(f)))
    }
    pub fn on_activate(self, f: impl Fn(&mut Ctx, &UiEvent) + 'static) -> Self {
        self.on(EventKind::Activate, Handler::Callback(Rc::new(f)))
    }
    pub fn on_pointer(self, f: impl Fn(&mut Ctx, &UiEvent) + 'static) -> Self {
        self.on(EventKind::Pointer, Handler::Callback(Rc::new(f)))
    }
    pub fn on_key(self, f: impl Fn(&mut Ctx, &UiEvent) + 'static) -> Self {
        self.on(EventKind::Key, Handler::Callback(Rc::new(f)))
    }
    pub fn on_text(self, f: impl Fn(&mut Ctx, &UiEvent) + 'static) -> Self {
        self.on(EventKind::Text, Handler::Callback(Rc::new(f)))
    }
    pub fn on_paste(self, f: impl Fn(&mut Ctx, &UiEvent) + 'static) -> Self {
        self.on(EventKind::Paste, Handler::Callback(Rc::new(f)))
    }
}

// ---- constructors (the `prelude` exports these)

pub fn label(text: impl Into<String>) -> Element {
    Element::new(Kind::Label).text(text)
}
pub fn button(text: impl Into<String>) -> Element {
    Element::new(Kind::Button).text(text)
}
pub fn text_field() -> Element {
    Element::new(Kind::TextField)
}
/// A multi-line text editor.
pub fn text_area() -> Element {
    Element::new(Kind::TextArea)
}
/// An area drawn by the app with display-list nodes (`.nodes(...)`).
pub fn custom() -> Element {
    Element::new(Kind::Custom)
}
pub fn list() -> Element {
    Element::new(Kind::List)
}
pub fn scroll_view(child: Element) -> Element {
    Element::new(Kind::ScrollView).child(child)
}
pub fn checkbox(text: impl Into<String>) -> Element {
    Element::new(Kind::Checkbox).text(text)
}
pub fn toggle(text: impl Into<String>) -> Element {
    Element::new(Kind::Toggle).text(text)
}
pub fn slider(min: i32, max: i32) -> Element {
    Element::new(Kind::Slider).range(min, max)
}
/// Tabs with these titles; add one child page per tab.
pub fn tabs<S: Into<String>>(titles: impl IntoIterator<Item = S>) -> Element {
    Element::new(Kind::Tabs).items(titles)
}
pub fn menu<S: Into<String>>(items: impl IntoIterator<Item = S>) -> Element {
    Element::new(Kind::Menu).items(items)
}
/// A modal dialog: fills its area with the scrim and centres a card with the
/// title and its children. Put it last in a `stack` over the window content.
pub fn dialog(title: impl Into<String>) -> Element {
    Element::new(Kind::Dialog).text(title)
}
pub fn image(resource: u32, w: i32, h: i32) -> Element {
    let mut e = Element::new(Kind::Image).width(w).height(h);
    e.props.resource = Some(resource);
    e
}
pub fn canvas(resource: u32, w: i32, h: i32) -> Element {
    let mut e = Element::new(Kind::Canvas).width(w).height(h);
    e.props.resource = Some(resource);
    e
}
pub fn row() -> Element {
    Element::new(Kind::Row)
}
pub fn column() -> Element {
    Element::new(Kind::Column)
}
pub fn stack() -> Element {
    Element::new(Kind::Stack)
}
pub fn grid(columns: u16) -> Element {
    Element::new(Kind::Grid).columns(columns, columns, columns)
}
pub fn spacer() -> Element {
    Element::new(Kind::Spacer)
}
/// One layout per size class (compact < 400 lu, medium < 800, wide).
pub fn responsive(compact: Element, medium: Element, wide: Element) -> Element {
    Element::new(Kind::Responsive).child(compact).child(medium).child(wide)
}
/// A free-form board (node editors, designers); see [`Kind::Board`].
pub fn board() -> Element {
    Element::new(Kind::Board)
}
/// A filled, bordered rectangle holding its children; see [`Kind::Panel`].
pub fn panel() -> Element {
    Element::new(Kind::Panel)
}
/// A polyline through `points` (relative to the parent).
pub fn wire(points: impl IntoIterator<Item = (i32, i32)>) -> Element {
    Element::new(Kind::Wire).points(points)
}
/// An editable indexed bitmap of `cols` x `rows` texels; see [`Kind::PixelGrid`].
pub fn pixel_grid(cols: u16, rows: u16) -> Element {
    Element::new(Kind::PixelGrid).texels(cols, rows)
}
/// An RGB colour picker; see [`Kind::ColorPicker`].
pub fn color_picker() -> Element {
    Element::new(Kind::ColorPicker)
}
