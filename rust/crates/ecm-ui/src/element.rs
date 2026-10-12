//! Elements: the declarative description of a window's widgets.
//!
//! An app describes its window as a tree of [`Element`]s, either rebuilt by a
//! view function on every render (Rust) or kept as a retained tree and
//! replaced when it changes (Python, `.ui.json`). The window keeps the
//! widgets' own state (text being edited, focus, scroll offsets, slider
//! drags) between renders, keyed by the element's `id` or, without one, by
//! its position and kind in the tree.

use crate::app::Ctx;
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
    ];

    pub fn from_name(name: &str) -> Option<Kind> {
        let name = match name {
            "scroll_view" | "scrollview" => "scroll",
            "text_field" => "textfield",
            n => n,
        };
        Kind::ALL.iter().copied().find(|k| k.name() == name)
    }

    /// Takes keyboard focus (Tab cycles through these).
    pub fn focusable(self) -> bool {
        matches!(
            self,
            Kind::Button | Kind::TextField | Kind::List | Kind::Checkbox | Kind::Toggle | Kind::Slider | Kind::Tabs | Kind::Menu
        )
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
}

impl EventKind {
    pub fn name(self) -> &'static str {
        match self {
            EventKind::Click => "click",
            EventKind::Change => "change",
            EventKind::Submit => "submit",
            EventKind::Select => "select",
        }
    }
    pub fn from_prop(key: &str) -> Option<EventKind> {
        Some(match key {
            "on_click" => EventKind::Click,
            "on_change" => EventKind::Change,
            "on_submit" => EventKind::Submit,
            "on_select" => EventKind::Select,
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
}

impl Element {
    pub fn new(kind: Kind) -> Element {
        Element { kind, id: None, props: Props::default(), children: Vec::new(), handlers: Vec::new() }
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
