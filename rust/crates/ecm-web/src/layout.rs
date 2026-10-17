//! Layout: the box tree from the DOM and its computed styles, then block flow,
//! inline text broken into lines with the ECM font metrics, and flexbox.
//!
//! Everything is in page coordinates (lu): a box's `x, y, w, h` is its border
//! box. Margins collapse between adjacent block siblings (not through
//! parents). Inline content becomes line boxes of text and atomic (inline
//! block, image, form control) fragments aligned on a common baseline.
//! `position: relative` shifts a box and its contents after layout;
//! `absolute` and `fixed` boxes leave the flow and are placed after it,
//! against their nearest positioned ancestor (fixed: the viewport). Tables
//! size their columns from the cells' minimum and preferred widths.

use crate::dom::{Dom, NodeData, NodeId, DOCUMENT};
use crate::style::{self, Align, Display, Justify, ListStyle, Overflow, Position, Style, TextAlign, UaColors, VAlign};
use ecm_ui_proto::fonts::{self, Face};
use ecm_ui_proto::Font;

/// What layout needs besides the boxes.
pub struct Env<'a> {
    pub dom: &'a Dom,
    pub styles: &'a [Option<Style>],
    pub viewport: (f32, f32),
    pub ua: &'a UaColors,
    /// The natural size of a picture by its address (None until it is loaded).
    pub images: &'a dyn Fn(&str) -> Option<(f32, f32)>,
    /// The page's address (pictures are resolved against it).
    pub base: Option<&'a crate::url::Url>,
    /// The focused text control and its caret (characters).
    pub caret: Option<(NodeId, usize)>,
}

impl Env<'_> {
    /// A picture's address resolved against the page.
    pub fn image_url(&self, src: &str) -> Option<String> {
        let src = src.trim();
        if src.is_empty() {
            return None;
        }
        match self.base {
            Some(b) => b.join(src).map(|u| u.to_string()),
            None => Some(src.to_string()),
        }
    }

    /// The natural size of `<img>` `node`, once loaded.
    pub fn img_size(&self, node: NodeId) -> Option<(f32, f32)> {
        let url = self.image_url(self.dom.attr(node, "src")?)?;
        (self.images)(&url)
    }

    pub fn style(&self, id: NodeId) -> &Style {
        self.styles.get(id).and_then(|s| s.as_ref()).or_else(|| self.styles[DOCUMENT].as_ref()).expect("the document has a style")
    }
}

#[derive(Clone, Debug)]
pub struct LayoutBox {
    /// None for anonymous boxes (inline runs between blocks, flex items made of text).
    pub node: Option<NodeId>,
    pub style: Style,
    pub content: Content,
    pub replaced: Option<Replaced>,
    pub margin: [f32; 4],
    pub border: [f32; 4],
    pub padding: [f32; 4],
    /// The border box.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// A height a flex container gives this item (border box).
    pub forced_h: Option<f32>,
    pub marker: Option<Marker>,
    /// Absolutely positioned and fixed boxes whose element sits in this one.
    pub oof: Vec<LayoutBox>,
}

#[derive(Clone, Debug)]
pub enum Content {
    Empty,
    Blocks(Vec<LayoutBox>),
    Inline(InlineContent),
    Flex(Vec<LayoutBox>),
    Table(TableContent),
}

#[derive(Clone, Debug, Default)]
pub struct TableContent {
    pub captions: Vec<LayoutBox>,
    pub rows: Vec<TableRow>,
    /// Column widths after layout.
    pub cols: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct TableRow {
    pub node: Option<NodeId>,
    pub style: Style,
    pub cells: Vec<LayoutBox>,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Debug, Default)]
pub struct InlineContent {
    pub items: Vec<InlineItem>,
    pub atomics: Vec<LayoutBox>,
    pub lines: Vec<Line>,
    /// Inline elements' boxes, one piece per line they are on (backgrounds, borders, hits).
    pub decos: Vec<Deco>,
}

#[derive(Clone, Debug)]
pub enum InlineItem {
    /// `el` is the element whose style the text has.
    Text { node: NodeId, el: NodeId, text: String },
    Open(NodeId),
    Close(NodeId),
    Atomic(usize),
    Break,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub y: f32,
    pub h: f32,
    pub baseline: f32,
    pub frags: Vec<Frag>,
}

#[derive(Clone, Debug)]
pub struct Frag {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub el: NodeId,
    pub kind: FragKind,
}

#[derive(Clone, Debug)]
pub enum FragKind {
    /// One line of text drawn at `size` in `font`; `node` is its text node.
    Text { text: String, font: Font, size: u8, node: NodeId },
    Atomic(usize),
}

#[derive(Clone, Debug)]
pub struct Deco {
    pub el: NodeId,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// This piece holds the element's start / end (left / right borders and padding).
    pub first: bool,
    pub last: bool,
}

#[derive(Clone, Debug)]
pub enum Replaced {
    Image { alt: String },
    Control { kind: ControlKind, text: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    Text,
    Button,
    Checkbox { checked: bool },
    Radio { checked: bool },
    Select,
    TextArea,
}

/// A list item's marker.
#[derive(Clone, Debug)]
pub enum Marker {
    /// A bullet: (x, y, size), filled / hollow / square.
    Bullet { x: f32, y: f32, d: f32, kind: ListStyle, color: u32 },
    Text { x: f32, y: f32, text: String, size: u8, color: u32 },
}

// ------------------------------------------------------------------ fonts

/// The face text of `style` is drawn with. The mono font has 16 and 32 only:
/// preformatted text uses it at 16 or 32, small inline code the ui font.
pub fn face(style: &Style) -> (Font, Face) {
    let size = style.draw_size();
    if style.mono && (style.white_space.keeps_spaces() || size >= 16) {
        let s = if size >= 24 { 32 } else { 16 };
        if let Some(f) = fonts::face(Font::Mono, s) {
            return (Font::Mono, f);
        }
    }
    (Font::Ui, fonts::face(Font::Ui, size).or_else(|| fonts::face(Font::Ui, 12)).expect("ui 12"))
}

fn text_width(face: &Face, s: &str) -> f32 {
    face.text_width(s) as f32
}

/// Characters the ECM fonts (Latin-1) don't have, as text they do.
pub fn latin1(s: &str) -> String {
    if s.chars().all(|c| (c as u32) < 0x100) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{2032}' => out.push('\''),
            '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{2033}' => out.push('"'),
            '\u{2010}'..='\u{2015}' | '\u{2212}' => out.push('-'),
            '\u{2026}' => out.push_str("..."),
            '\u{2022}' | '\u{25cf}' | '\u{2219}' => out.push('\u{b7}'),
            '\u{2190}' => out.push_str("<-"),
            '\u{2192}' => out.push_str("->"),
            '\u{2191}' => out.push('^'),
            '\u{2193}' => out.push('v'),
            '\u{2122}' => out.push_str("TM"),
            '\u{20ac}' => out.push_str("EUR"),
            '\u{2009}' | '\u{200a}' | '\u{2002}' | '\u{2003}' | '\u{202f}' => out.push(' '),
            '\u{200b}'..='\u{200d}' | '\u{feff}' => {}
            c if (c as u32) < 0x100 => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

// ------------------------------------------------------------------ the box tree

impl LayoutBox {
    fn new(node: Option<NodeId>, style: Style, content: Content) -> LayoutBox {
        LayoutBox {
            node,
            style,
            content,
            replaced: None,
            margin: [0.0; 4],
            border: [0.0; 4],
            padding: [0.0; 4],
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            forced_h: None,
            marker: None,
            oof: Vec::new(),
        }
    }

    /// The margin box's outer width and height.
    pub fn outer_w(&self) -> f32 {
        self.w + self.margin[1] + self.margin[3]
    }

    pub fn outer_h(&self) -> f32 {
        self.h + self.margin[0] + self.margin[2]
    }

    /// The content box.
    pub fn content_rect(&self) -> (f32, f32, f32, f32) {
        let x = self.x + self.border[3] + self.padding[3];
        let y = self.y + self.border[0] + self.padding[0];
        let w = self.w - self.border[1] - self.border[3] - self.padding[1] - self.padding[3];
        let h = self.h - self.border[0] - self.border[2] - self.padding[0] - self.padding[2];
        (x, y, w.max(0.0), h.max(0.0))
    }

    /// The padding box (where `overflow` clips).
    pub fn padding_rect(&self) -> (f32, f32, f32, f32) {
        (self.x + self.border[3], self.y + self.border[0], (self.w - self.border[1] - self.border[3]).max(0.0), (self.h - self.border[0] - self.border[2]).max(0.0))
    }

    /// Move the box and everything in it.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        self.x += dx;
        self.y += dy;
        match &mut self.marker {
            Some(Marker::Bullet { x, y, .. }) | Some(Marker::Text { x, y, .. }) => {
                *x += dx;
                *y += dy;
            }
            None => {}
        }
        match &mut self.content {
            Content::Empty => {}
            Content::Blocks(c) | Content::Flex(c) => c.iter_mut().for_each(|b| b.translate(dx, dy)),
            Content::Inline(ic) => {
                for l in &mut ic.lines {
                    l.y += dy;
                    l.baseline += dy;
                    for f in &mut l.frags {
                        f.x += dx;
                        f.y += dy;
                    }
                }
                for d in &mut ic.decos {
                    d.x += dx;
                    d.y += dy;
                }
                ic.atomics.iter_mut().for_each(|b| b.translate(dx, dy));
            }
            Content::Table(t) => {
                t.captions.iter_mut().for_each(|b| b.translate(dx, dy));
                for r in &mut t.rows {
                    r.x += dx;
                    r.y += dy;
                    r.cells.iter_mut().for_each(|b| b.translate(dx, dy));
                }
            }
        }
        self.oof.iter_mut().for_each(|b| b.translate(dx, dy));
    }

    /// The boxes inside this one in flow (for walks over the tree).
    pub fn children_mut(&mut self) -> Vec<&mut LayoutBox> {
        match &mut self.content {
            Content::Empty => Vec::new(),
            Content::Blocks(c) | Content::Flex(c) => c.iter_mut().collect(),
            Content::Inline(ic) => ic.atomics.iter_mut().collect(),
            Content::Table(t) => t.captions.iter_mut().chain(t.rows.iter_mut().flat_map(|r| r.cells.iter_mut())).collect(),
        }
    }

    /// The baseline of the first line inside, if any.
    pub fn first_baseline(&self) -> Option<f32> {
        match &self.content {
            Content::Inline(ic) => ic.lines.first().map(|l| l.baseline),
            Content::Blocks(c) | Content::Flex(c) => c.iter().find_map(|b| b.first_baseline()),
            Content::Table(t) => t.rows.first().and_then(|r| r.cells.iter().find_map(|c| c.first_baseline())),
            Content::Empty => None,
        }
    }
}

enum Child {
    Block(LayoutBox),
    Inline(InlineItem),
    Atomic(LayoutBox),
    /// `position: absolute | fixed`: out of the flow.
    OutOfFlow(LayoutBox),
}

const REPLACED: &[&str] = &["img", "input", "select", "textarea"];

/// The document's box tree: an anonymous root as wide as the viewport.
pub fn build(env: &Env) -> LayoutBox {
    let mut st = env.style(DOCUMENT).clone();
    st.display = Display::Block;
    let mut kids = Vec::new();
    collect(env, DOCUMENT, &mut kids, false);
    let (content, oof) = assemble(env, kids, &st, false);
    let mut b = LayoutBox::new(Some(DOCUMENT), st, content);
    b.oof = oof;
    b
}

fn blockified(d: Display) -> Display {
    match d {
        Display::Inline | Display::InlineBlock => Display::Block,
        Display::InlineFlex => Display::Flex,
        d => d,
    }
}

fn collect(env: &Env, node: NodeId, out: &mut Vec<Child>, blockify: bool) {
    let dom = env.dom;
    for &c in &dom.nodes[node].children {
        match &dom.nodes[c].data {
            NodeData::Text(t) => out.push(Child::Inline(InlineItem::Text { node: c, el: node, text: t.clone() })),
            NodeData::Element(e) => {
                let s = env.style(c);
                let mut d = s.display;
                if d == Display::None {
                    continue;
                }
                if blockify {
                    d = blockified(d);
                }
                // Table parts outside a table are blocks.
                if matches!(d, Display::TableRowGroup | Display::TableRow | Display::TableCell | Display::TableCaption) {
                    d = Display::Block;
                }
                if matches!(s.position, Position::Absolute | Position::Fixed) {
                    let inner = match blockified(d) {
                        Display::Flex => Display::Flex,
                        Display::Table => Display::Table,
                        _ => Display::Block,
                    };
                    let b = if REPLACED.contains(&e.tag.as_str()) { build_replaced(env, c) } else { build_box(env, c, inner) };
                    out.push(Child::OutOfFlow(b));
                    continue;
                }
                if e.tag == "br" {
                    out.push(Child::Inline(InlineItem::Break));
                    continue;
                }
                if REPLACED.contains(&e.tag.as_str()) {
                    let b = build_replaced(env, c);
                    out.push(if d.is_inline_level() { Child::Atomic(b) } else { Child::Block(b) });
                    continue;
                }
                match d {
                    Display::Inline => {
                        out.push(Child::Inline(InlineItem::Open(c)));
                        collect(env, c, out, false);
                        out.push(Child::Inline(InlineItem::Close(c)));
                    }
                    Display::InlineBlock | Display::InlineFlex => out.push(Child::Atomic(build_box(env, c, d))),
                    _ => out.push(Child::Block(build_box(env, c, d))),
                }
            }
            NodeData::Document => {}
        }
    }
}

fn build_box(env: &Env, el: NodeId, display: Display) -> LayoutBox {
    let mut st = env.style(el).clone();
    st.display = display;
    if display == Display::Table {
        let (t, oof) = build_table(env, el);
        let mut b = LayoutBox::new(Some(el), st, Content::Table(t));
        b.oof = oof;
        return b;
    }
    let flex = matches!(display, Display::Flex | Display::InlineFlex);
    let mut kids = Vec::new();
    collect(env, el, &mut kids, flex);
    let (content, oof) = assemble(env, kids, &st, flex);
    let mut b = LayoutBox::new(Some(el), st, content);
    b.oof = oof;
    b
}

/// A table's captions and rows (rows from row groups too, in document order);
/// cells stray in a table get a row of their own.
fn build_table(env: &Env, table: NodeId) -> (TableContent, Vec<LayoutBox>) {
    let dom = env.dom;
    let mut t = TableContent::default();
    let mut oof = Vec::new();
    let mut stray: Option<TableRow> = None;
    let row_of = |env: &Env, tr: NodeId| -> TableRow {
        let cells = env
            .dom
            .element_children(tr)
            .filter(|c| env.style(*c).display != Display::None)
            .map(|c| if REPLACED.contains(&env.dom.tag(c).unwrap_or("")) { build_replaced(env, c) } else { build_box(env, c, Display::Block) })
            .collect();
        TableRow { node: Some(tr), style: env.style(tr).clone(), cells, x: 0.0, y: 0.0, w: 0.0, h: 0.0 }
    };
    let mut stack: Vec<NodeId> = dom.element_children(table).collect();
    stack.reverse();
    while let Some(c) = stack.pop() {
        let s = env.style(c);
        match s.display {
            Display::None => {}
            Display::TableCaption => t.captions.push(build_box(env, c, Display::Block)),
            Display::TableRowGroup => {
                let mut kids: Vec<NodeId> = dom.element_children(c).collect();
                kids.reverse();
                stack.extend(kids);
            }
            Display::TableRow => {
                if let Some(r) = stray.take() {
                    t.rows.push(r);
                }
                t.rows.push(row_of(env, c));
            }
            _ if matches!(s.position, Position::Absolute | Position::Fixed) => oof.push(build_box(env, c, Display::Block)),
            _ => {
                let cell = build_box(env, c, Display::Block);
                stray
                    .get_or_insert_with(|| TableRow { node: None, style: env.style(table).inherit_from(env.ua), cells: Vec::new(), x: 0.0, y: 0.0, w: 0.0, h: 0.0 })
                    .cells
                    .push(cell);
            }
        }
    }
    if let Some(r) = stray.take() {
        t.rows.push(r);
    }
    (t, oof)
}

fn build_replaced(env: &Env, el: NodeId) -> LayoutBox {
    let dom = env.dom;
    let e = dom.element(el).expect("element");
    let attr = |n: &str| e.attr(n).unwrap_or("").to_string();
    let r = match e.tag.as_str() {
        "img" => Replaced::Image { alt: attr("alt") },
        "select" => {
            let opts: Vec<NodeId> = dom.descendants(el).into_iter().filter(|n| dom.tag(*n) == Some("option")).collect();
            let chosen = opts.iter().copied().find(|o| dom.attr(*o, "selected").is_some()).or(opts.first().copied());
            Replaced::Control { kind: ControlKind::Select, text: chosen.map(|o| crate::dom::collapse_ws(&dom.text_content(o))).unwrap_or_default() }
        }
        "textarea" => Replaced::Control { kind: ControlKind::TextArea, text: dom.text_content(el) },
        _ => {
            let ty = attr("type").to_ascii_lowercase();
            match ty.as_str() {
                "checkbox" => Replaced::Control { kind: ControlKind::Checkbox { checked: e.attr("checked").is_some() }, text: String::new() },
                "radio" => Replaced::Control { kind: ControlKind::Radio { checked: e.attr("checked").is_some() }, text: String::new() },
                "submit" | "button" | "reset" => {
                    let def = match ty.as_str() {
                        "submit" => "Submit",
                        "reset" => "Reset",
                        _ => "",
                    };
                    Replaced::Control { kind: ControlKind::Button, text: e.attr("value").unwrap_or(def).to_string() }
                }
                _ => {
                    let v = attr("value");
                    let text = if v.is_empty() {
                        attr("placeholder")
                    } else if ty == "password" {
                        "*".repeat(v.chars().count())
                    } else {
                        v
                    };
                    Replaced::Control { kind: ControlKind::Text, text }
                }
            }
        }
    };
    let mut b = LayoutBox::new(Some(el), env.style(el).clone(), Content::Empty);
    b.replaced = Some(r);
    b
}

fn is_blank_run(env: &Env, run: &[InlineItem]) -> bool {
    run.iter().all(|i| match i {
        InlineItem::Text { text, el, .. } => !env.style(*el).white_space.keeps_spaces() && text.chars().all(|c| c.is_ascii_whitespace()),
        InlineItem::Open(_) | InlineItem::Close(_) => true,
        _ => false,
    })
}

/// Children into a box's content: block children, or inline content, or flex
/// items; and the out-of-flow boxes among them.
fn assemble(env: &Env, kids: Vec<Child>, parent: &Style, flex: bool) -> (Content, Vec<LayoutBox>) {
    let mut oof = Vec::new();
    let kids: Vec<Child> = kids
        .into_iter()
        .filter_map(|k| match k {
            Child::OutOfFlow(b) => {
                oof.push(b);
                None
            }
            k => Some(k),
        })
        .collect();
    (assemble_flow(env, kids, parent, flex), oof)
}

fn assemble_flow(env: &Env, kids: Vec<Child>, parent: &Style, flex: bool) -> Content {
    let has_block = flex || kids.iter().any(|k| matches!(k, Child::Block(_)));
    if !has_block {
        let mut ic = InlineContent::default();
        for k in kids {
            match k {
                Child::Inline(i) => ic.items.push(i),
                Child::Atomic(b) => {
                    ic.items.push(InlineItem::Atomic(ic.atomics.len()));
                    ic.atomics.push(b);
                }
                Child::Block(_) | Child::OutOfFlow(_) => unreachable!(),
            }
        }
        if ic.atomics.is_empty() && is_blank_run(env, &ic.items) {
            return Content::Empty;
        }
        return Content::Inline(ic);
    }
    let mut out = Vec::new();
    let mut run = InlineContent::default();
    let mut open: Vec<NodeId> = Vec::new();
    let anon_style = || {
        let mut s = parent.inherit_from(env.ua);
        s.display = Display::Block;
        s
    };
    let finish = |run: &mut InlineContent, open: &[NodeId], out: &mut Vec<LayoutBox>| {
        let mut r = std::mem::take(run);
        for o in open.iter().rev() {
            r.items.push(InlineItem::Close(*o));
        }
        if !(r.atomics.is_empty() && is_blank_run(env, &r.items)) {
            out.push(LayoutBox::new(None, anon_style(), Content::Inline(r)));
        }
        for o in open {
            run.items.push(InlineItem::Open(*o));
        }
    };
    for k in kids {
        match k {
            Child::Inline(i) => {
                match &i {
                    InlineItem::Open(e) => open.push(*e),
                    InlineItem::Close(e) => {
                        if let Some(p) = open.iter().rposition(|o| o == e) {
                            open.remove(p);
                        }
                    }
                    _ => {}
                }
                run.items.push(i);
            }
            Child::Atomic(b) => {
                if flex {
                    finish(&mut run, &open, &mut out);
                    out.push(b);
                } else {
                    run.items.push(InlineItem::Atomic(run.atomics.len()));
                    run.atomics.push(b);
                }
            }
            Child::Block(b) => {
                finish(&mut run, &open, &mut out);
                out.push(b);
            }
            Child::OutOfFlow(_) => {}
        }
    }
    for o in open.iter().rev() {
        run.items.push(InlineItem::Close(*o));
    }
    if !(run.atomics.is_empty() && is_blank_run(env, &run.items)) {
        out.push(LayoutBox::new(None, anon_style(), Content::Inline(run)));
    }
    if flex {
        Content::Flex(out)
    } else {
        Content::Blocks(out)
    }
}

// ------------------------------------------------------------------ block layout

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WidthMode {
    /// Fill the containing block (normal flow).
    Fill,
    /// Shrink to fit (inline blocks, flex measurement).
    Fit,
    /// This border-box width (flex items).
    Exact(f32),
}

fn edges(s: &Style, cb_w: f32) -> ([f32; 4], [f32; 4], [f32; 4]) {
    let m = [s.margin[0].resolve(cb_w), s.margin[1].resolve(cb_w), s.margin[2].resolve(cb_w), s.margin[3].resolve(cb_w)];
    let p = [s.padding[0].resolve(cb_w), s.padding[1].resolve(cb_w), s.padding[2].resolve(cb_w), s.padding[3].resolve(cb_w)];
    (m, s.borders(), p)
}

/// A replaced element's natural content size.
fn replaced_size(env: &Env, b: &LayoutBox) -> (f32, f32) {
    let s = &b.style;
    let (_, f) = face(s);
    let lh = s.line_height_px();
    match &b.replaced {
        Some(Replaced::Image { alt }) => {
            let natural = b.node.and_then(|n| env.img_size(n));
            match natural {
                Some((w, h)) => (w, h),
                None if alt.is_empty() => (16.0, 16.0),
                None => (text_width(&f, &latin1(alt)) + 4.0, lh),
            }
        }
        Some(Replaced::Control { kind, text }) => {
            let digit = f.advance('0') as f32;
            let attr = |n: &str| b.node.and_then(|id| env.dom.attr(id, n)).and_then(|v| v.parse::<f32>().ok());
            match kind {
                ControlKind::Checkbox { .. } | ControlKind::Radio { .. } => (12.0, 12.0),
                ControlKind::Button => (text_width(&f, text), lh),
                ControlKind::Select => (text_width(&f, text) + 14.0, lh),
                ControlKind::Text => (attr("size").unwrap_or(20.0) * digit, lh),
                ControlKind::TextArea => (attr("cols").unwrap_or(30.0) * digit, attr("rows").unwrap_or(3.0) * lh),
            }
        }
        None => (0.0, 0.0),
    }
}

/// A replaced box's content size from its style and natural size.
fn replaced_used(env: &Env, b: &LayoutBox, cb_w: f32, cb_h: Option<f32>, bp: (f32, f32)) -> (f32, f32) {
    let (nw, nh) = replaced_size(env, b);
    let s = &b.style;
    let adj = |v: f32, bpv: f32| if s.border_box { (v - bpv).max(0.0) } else { v };
    let w = s.width.definite(Some(cb_w)).map(|v| adj(v, bp.0));
    let h = s.height.definite(cb_h).map(|v| adj(v, bp.1));
    let ratio = if nh > 0.0 { nw / nh } else { 1.0 };
    let is_img = matches!(b.replaced, Some(Replaced::Image { .. }));
    match (w, h) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) if is_img => (w, w / ratio),
        (None, Some(h)) if is_img => (h * ratio, h),
        (Some(w), None) => (w, nh),
        (None, Some(h)) => (nw, h),
        (None, None) => {
            // Images shrink to the containing block, keeping their shape.
            if is_img && nw > cb_w && cb_w > 0.0 {
                (cb_w, cb_w / ratio)
            } else {
                (nw, nh)
            }
        }
    }
}

fn clamp_w(s: &Style, w: f32, cb_w: f32, bp: f32) -> f32 {
    let adj = |v: f32| if s.border_box { (v - bp).max(0.0) } else { v };
    let mut w = w;
    if let Some(mx) = s.max_width.definite(Some(cb_w)) {
        w = w.min(adj(mx));
    }
    if let Some(mn) = s.min_width.definite(Some(cb_w)) {
        w = w.max(adj(mn));
    }
    w.max(0.0)
}

fn clamp_h(s: &Style, h: f32, cb_h: Option<f32>, bp: f32) -> f32 {
    let adj = |v: f32| if s.border_box { (v - bp).max(0.0) } else { v };
    let mut h = h;
    if let Some(mx) = s.max_height.definite(cb_h) {
        h = h.min(adj(mx));
    }
    if let Some(mn) = s.min_height.definite(cb_h) {
        h = h.max(adj(mn));
    }
    h.max(0.0)
}

/// Lay out `b` with its margin box's top-left at `(x, y)`, inside a
/// containing block `cb_w` wide (and `cb_h` high, when that is known).
pub fn layout_block(b: &mut LayoutBox, env: &Env, cb_w: f32, cb_h: Option<f32>, x: f32, y: f32, mode: WidthMode) {
    let (mut m, bd, p) = edges(&b.style, cb_w);
    b.border = bd;
    b.padding = p;
    let bp_h = bd[1] + bd[3] + p[1] + p[3];
    let bp_v = bd[0] + bd[2] + p[0] + p[2];
    let s = b.style.clone();
    let auto_l = s.margin[3].auto;
    let auto_r = s.margin[1].auto;
    let specified = s.width.definite(Some(cb_w)).map(|w| if s.border_box { (w - bp_h).max(0.0) } else { w });
    let mut replaced_h = None;
    let cw = match mode {
        WidthMode::Exact(bw) => (bw - bp_h).max(0.0),
        _ if b.replaced.is_some() => {
            let (w, h) = replaced_used(env, b, cb_w, cb_h, (bp_h, bp_v));
            replaced_h = Some(h);
            clamp_w(&s, w, cb_w, bp_h)
        }
        _ => match specified {
            Some(w) => clamp_w(&s, w, cb_w, bp_h),
            None => {
                let avail = (cb_w - m[1] - m[3] - bp_h).max(0.0);
                // An auto-width table is as wide as its columns want, at most the containing block.
                let mode = if s.display == Display::Table { WidthMode::Fit } else { mode };
                let w = match mode {
                    WidthMode::Fit => {
                        let (mn, mx) = intrinsic(b, env);
                        mx.min(avail).max(mn)
                    }
                    _ => avail,
                };
                clamp_w(&s, w, cb_w, bp_h)
            }
        },
    };
    // Auto margins centre a block narrower than its containing block.
    if mode == WidthMode::Fill {
        let free = cb_w - cw - bp_h - if auto_l { 0.0 } else { m[3] } - if auto_r { 0.0 } else { m[1] };
        if auto_l && auto_r {
            m[3] = (free / 2.0).max(0.0);
            m[1] = (free / 2.0).max(0.0);
        } else if auto_l {
            m[3] = free.max(0.0);
        }
    }
    b.margin = m;
    b.x = x + m[3];
    b.y = y + m[0];
    let cx = b.x + bd[3] + p[3];
    let cy = b.y + bd[0] + p[0];
    let forced = b.forced_h.map(|fh| (fh - bp_v).max(0.0));
    let spec_h = forced.or(replaced_h).or_else(|| s.height.definite(cb_h).map(|h| if s.border_box { (h - bp_v).max(0.0) } else { h }));
    // The document's percentages of height refer to the viewport.
    let inner_cb_h = if b.node == Some(DOCUMENT) { cb_h } else { spec_h.map(|h| clamp_h(&s, h, cb_h, bp_v)) };
    let content_h = match &mut b.content {
        Content::Empty => 0.0,
        Content::Blocks(kids) => layout_blocks(kids, env, cw, inner_cb_h, cx, cy),
        Content::Inline(ic) => layout_inline(ic, env, &s, cw, cx, cy),
        Content::Flex(items) => layout_flex(items, env, &s, cw, inner_cb_h, cx, cy),
        Content::Table(t) => layout_table(t, env, &s, cw, cx, cy),
    };
    let ch = match spec_h {
        Some(h) if b.forced_h.is_some() => h,
        Some(h) => clamp_h(&s, h, cb_h, bp_v),
        None => clamp_h(&s, content_h, cb_h, bp_v),
    };
    b.w = cw + bp_h;
    b.h = ch + bp_v;
    if s.display == Display::ListItem && s.list_style != ListStyle::None {
        b.marker = marker(b, env);
    }
}

fn layout_blocks(kids: &mut [LayoutBox], env: &Env, cw: f32, cb_h: Option<f32>, cx: f32, cy: f32) -> f32 {
    let mut cursor = cy;
    let mut prev_mb = 0.0f32;
    let mut started = false;
    for k in kids.iter_mut() {
        let mt = k.style.margin[0].resolve(cw);
        // Adjacent margins collapse to the larger one (negative ones add).
        let gap = if started {
            if prev_mb >= 0.0 && mt >= 0.0 {
                prev_mb.max(mt)
            } else if prev_mb < 0.0 && mt < 0.0 {
                prev_mb.min(mt)
            } else {
                prev_mb + mt
            }
        } else {
            mt
        };
        layout_block(k, env, cw, cb_h, cx, cursor + gap - mt, WidthMode::Fill);
        relative(k, cw, cb_h);
        // A relative shift doesn't move what follows.
        let (rdx, rdy) = rel_offset(&k.style, cw, cb_h);
        cursor = k.y - rdy + k.h;
        let _ = rdx;
        prev_mb = k.margin[2];
        started = true;
    }
    if started {
        cursor + prev_mb - cy
    } else {
        0.0
    }
}

fn rel_offset(s: &Style, cb_w: f32, cb_h: Option<f32>) -> (f32, f32) {
    if s.position != Position::Relative {
        return (0.0, 0.0);
    }
    let [t, r, bt, l] = s.inset;
    let dx = if !l.auto { l.resolve(cb_w) } else if !r.auto { -r.resolve(cb_w) } else { 0.0 };
    let dy = if !t.auto { t.definite(cb_h).unwrap_or(t.px) } else if !bt.auto { -bt.definite(cb_h).unwrap_or(bt.px) } else { 0.0 };
    (dx, dy)
}

fn relative(b: &mut LayoutBox, cb_w: f32, cb_h: Option<f32>) {
    let (dx, dy) = rel_offset(&b.style, cb_w, cb_h);
    b.translate(dx, dy);
}

fn roman(mut n: usize, upper: bool) -> String {
    let table = [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
    let mut s = String::new();
    for (v, r) in table {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    if upper {
        s.to_uppercase()
    } else {
        s
    }
}

fn marker(b: &LayoutBox, env: &Env) -> Option<Marker> {
    let s = &b.style;
    let (_, f) = face(s);
    let size = f.size;
    let baseline = b.first_baseline().unwrap_or(b.y + b.border[0] + b.padding[0] + f.ascent() as f32);
    let (cx, _, _, _) = b.content_rect();
    let color = s.color;
    let n = || -> usize {
        // The item's number: its position among the list's items (+ start).
        let Some(id) = b.node else { return 1 };
        let Some(parent) = env.dom.parent(id) else { return 1 };
        let start = env.dom.attr(parent, "start").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1);
        let pos = env.dom.element_children(parent).filter(|c| env.dom.tag(*c) == Some("li")).position(|c| c == id).unwrap_or(0) as i64;
        let v = env.dom.attr(id, "value").and_then(|v| v.parse::<i64>().ok());
        v.unwrap_or(start + pos).max(1) as usize
    };
    match s.list_style {
        ListStyle::None => None,
        ListStyle::Disc | ListStyle::Circle | ListStyle::Square => {
            let d = (size as f32 / 3.0).round().max(4.0);
            Some(Marker::Bullet { x: cx - d - 6.0, y: baseline - f.ascent() as f32 / 2.0 - d / 2.0, d, kind: s.list_style, color })
        }
        kind => {
            let k = n();
            let text = match kind {
                ListStyle::LowerAlpha => format!("{}.", (b'a' + ((k - 1) % 26) as u8) as char),
                ListStyle::UpperAlpha => format!("{}.", (b'A' + ((k - 1) % 26) as u8) as char),
                ListStyle::LowerRoman => format!("{}.", roman(k, false)),
                ListStyle::UpperRoman => format!("{}.", roman(k, true)),
                _ => format!("{k}."),
            };
            let w = text_width(&f, &text);
            Some(Marker::Text { x: cx - w - 5.0, y: baseline - f.ascent() as f32, text, size, color })
        }
    }
}

// ------------------------------------------------------------------ intrinsic sizes

/// (min-content, max-content) width of the content box.
pub fn intrinsic(b: &LayoutBox, env: &Env) -> (f32, f32) {
    if b.replaced.is_some() {
        let (w, _) = replaced_size(env, b);
        if let Some(sw) = b.style.width.definite(None) {
            return (sw, sw);
        }
        return (w, w);
    }
    match &b.content {
        Content::Empty => (0.0, 0.0),
        Content::Blocks(kids) => kids.iter().map(|k| outer_intrinsic(k, env)).fold((0.0, 0.0), |a, x| (a.0.max(x.0), a.1.max(x.1))),
        Content::Inline(ic) => inline_intrinsic(ic, env, &b.style),
        Content::Table(t) => {
            let (mins, maxs) = table_columns(t, env);
            let sp = b.style.border_spacing * (mins.len() as f32 + 1.0);
            let caps = t.captions.iter().map(|c| outer_intrinsic(c, env)).fold((0.0f32, 0.0f32), |a, x| (a.0.max(x.0), a.1.max(x.1)));
            ((mins.iter().sum::<f32>() + sp).max(caps.0), (maxs.iter().sum::<f32>() + sp).max(caps.1))
        }
        Content::Flex(items) => {
            let s = &b.style;
            let sizes: Vec<(f32, f32)> = items.iter().map(|k| outer_intrinsic(k, env)).collect();
            if s.flex_direction.is_row() {
                let gaps = s.column_gap * items.len().saturating_sub(1) as f32;
                let max = sizes.iter().map(|x| x.1).sum::<f32>() + gaps;
                let min = if s.flex_wrap { sizes.iter().map(|x| x.0).fold(0.0, f32::max) } else { sizes.iter().map(|x| x.0).sum::<f32>() + gaps };
                (min, max)
            } else {
                sizes.iter().fold((0.0, 0.0), |a, x| (a.0.max(x.0), a.1.max(x.1)))
            }
        }
    }
}

/// [`intrinsic`] plus fixed margins, borders and padding (or the specified width).
pub fn outer_intrinsic(b: &LayoutBox, env: &Env) -> (f32, f32) {
    let s = &b.style;
    let (m, bd, p) = edges(s, 0.0);
    let bp = bd[1] + bd[3] + p[1] + p[3];
    let extra = bp + m[1] + m[3];
    if b.replaced.is_none() {
        if let Some(w) = s.width.definite(None) {
            let w = if s.border_box { w } else { w + bp };
            let w = clamp_w(s, w, 0.0, 0.0);
            return (w + m[1] + m[3], w + m[1] + m[3]);
        }
    }
    let (mn, mx) = intrinsic(b, env);
    let mut mn = mn;
    let mut mx = mx;
    if let Some(maxw) = s.max_width.definite(None) {
        mx = mx.min(maxw);
        mn = mn.min(maxw);
    }
    if let Some(minw) = s.min_width.definite(None) {
        mx = mx.max(minw);
        mn = mn.max(minw);
    }
    (mn + extra, mx + extra)
}

fn inline_intrinsic(ic: &InlineContent, env: &Env, container: &Style) -> (f32, f32) {
    let pieces = pieces(ic, env, container);
    let mut min = 0.0f32;
    let mut line = 0.0f32;
    let mut max = 0.0f32;
    let mut word = 0.0f32;
    for p in &pieces {
        match p {
            Piece::Word { w, .. } => {
                word += w;
                line += w;
            }
            Piece::Space { w, .. } => {
                min = min.max(word);
                word = 0.0;
                line += w;
            }
            Piece::Edge { w, .. } => {
                word += w;
                line += w;
            }
            Piece::Atomic { idx } => {
                let (amn, amx) = outer_intrinsic(&ic.atomics[*idx], env);
                min = min.max(word).max(amn);
                word = 0.0;
                line += amx;
            }
            Piece::Newline => {
                min = min.max(word);
                word = 0.0;
                max = max.max(line);
                line = 0.0;
            }
        }
    }
    min = min.max(word);
    max = max.max(line);
    (min.ceil(), max.ceil() + container.text_indent.max(0.0))
}

// ------------------------------------------------------------------ inline layout

#[derive(Clone, Debug)]
enum Piece {
    Word { text: String, el: NodeId, node: NodeId, w: f32 },
    /// A breakable space (`text` is what it draws as).
    Space { text: String, el: NodeId, node: NodeId, w: f32, collapsible: bool },
    /// An inline element's start (`open`) or end: its border, padding and margin on that side.
    Edge { el: NodeId, open: bool, w: f32 },
    Atomic { idx: usize },
    Newline,
}

fn edge_width(s: &Style, side: usize) -> f32 {
    let b = s.borders();
    s.margin[side].resolve(0.0) + b[side] + s.padding[side].resolve(0.0)
}

/// The inline items as words, spaces and breaks, white space processed.
fn pieces(ic: &InlineContent, env: &Env, _container: &Style) -> Vec<Piece> {
    let mut out = Vec::new();
    // At the start of the content (or after a collapsible space): drop leading spaces.
    let mut after_space = true;
    for item in &ic.items {
        match item {
            InlineItem::Text { node, el, text } => {
                let s = env.style(*el);
                let (_, f) = face(s);
                let ws = s.white_space;
                let text = latin1(&style::transform_text(text, s.text_transform));
                let mut word = String::new();
                let flush = |word: &mut String, out: &mut Vec<Piece>| {
                    if !word.is_empty() {
                        let t = word.replace('\u{a0}', " ");
                        let w = text_width(&f, &t);
                        out.push(Piece::Word { text: t, el: *el, node: *node, w });
                        word.clear();
                    }
                };
                for c in text.chars() {
                    match c {
                        '\n' if ws.keeps_newlines() => {
                            flush(&mut word, &mut out);
                            out.push(Piece::Newline);
                            after_space = true;
                        }
                        '\r' if ws.keeps_newlines() => {}
                        ' ' | '\t' | '\n' | '\r' | '\x0c' => {
                            flush(&mut word, &mut out);
                            if ws.keeps_spaces() {
                                let t = if c == '\t' { "    " } else { " " };
                                out.push(Piece::Space { text: t.into(), el: *el, node: *node, w: text_width(&f, t), collapsible: false });
                            } else if !after_space {
                                out.push(Piece::Space { text: " ".into(), el: *el, node: *node, w: text_width(&f, " "), collapsible: true });
                                after_space = true;
                            }
                        }
                        c => {
                            word.push(c);
                            after_space = false;
                        }
                    }
                }
                flush(&mut word, &mut out);
            }
            InlineItem::Open(el) => out.push(Piece::Edge { el: *el, open: true, w: edge_width(env.style(*el), 3) }),
            InlineItem::Close(el) => out.push(Piece::Edge { el: *el, open: false, w: edge_width(env.style(*el), 1) }),
            InlineItem::Atomic(idx) => {
                out.push(Piece::Atomic { idx: *idx });
                after_space = false;
            }
            InlineItem::Break => {
                out.push(Piece::Newline);
                after_space = true;
            }
        }
    }
    out
}

/// One placed piece on a line being built.
#[derive(Clone, Debug)]
struct Placed {
    piece: Piece,
    w: f32,
}

fn layout_inline(ic: &mut InlineContent, env: &Env, container: &Style, cw: f32, cx: f32, cy: f32) -> f32 {
    let ps = pieces(ic, env, container);
    // Atomic boxes first: their size decides how they fit.
    for a in ic.atomics.iter_mut() {
        layout_block(a, env, cw, None, 0.0, 0.0, WidthMode::Fit);
    }
    let wraps = |el: NodeId| env.style(el).white_space.wraps();
    let mut lines: Vec<Vec<Placed>> = Vec::new();
    let mut cur: Vec<Placed> = Vec::new();
    let mut cur_w = container.text_indent;
    let mut can_break = true;
    let has_content = |cur: &[Placed]| cur.iter().any(|p| matches!(p.piece, Piece::Word { .. } | Piece::Atomic { .. }));
    let finish = |cur: &mut Vec<Placed>, lines: &mut Vec<Vec<Placed>>| {
        // Collapsible spaces at the end of a line go.
        while let Some(i) = cur.iter().rposition(|p| !matches!(p.piece, Piece::Edge { .. })) {
            if matches!(cur[i].piece, Piece::Space { collapsible: true, .. }) {
                cur.remove(i);
            } else {
                break;
            }
        }
        lines.push(std::mem::take(cur));
    };
    for p in ps {
        match &p {
            Piece::Word { text, el, node, w } => {
                let wrap = wraps(*el);
                if wrap && can_break && cur_w + w > cw && has_content(&cur) {
                    finish(&mut cur, &mut lines);
                    cur_w = 0.0;
                }
                if wrap && cur_w + w > cw && !has_content(&cur) && *w > cw {
                    // A word wider than the line: split it between characters.
                    let s = env.style(*el);
                    let (_, f) = face(s);
                    let mut chunk = String::new();
                    let mut chunk_w = 0.0;
                    for c in text.chars() {
                        let a = f.advance(c) as f32;
                        if cur_w + chunk_w + a > cw && !chunk.is_empty() {
                            cur.push(Placed { piece: Piece::Word { text: std::mem::take(&mut chunk), el: *el, node: *node, w: chunk_w }, w: chunk_w });
                            finish(&mut cur, &mut lines);
                            cur_w = 0.0;
                            chunk_w = 0.0;
                        }
                        chunk.push(c);
                        chunk_w += a;
                    }
                    cur_w += chunk_w;
                    cur.push(Placed { piece: Piece::Word { text: chunk, el: *el, node: *node, w: chunk_w }, w: chunk_w });
                } else {
                    cur_w += w;
                    cur.push(Placed { piece: p.clone(), w: *w });
                }
                can_break = false;
            }
            Piece::Space { w, collapsible, .. } => {
                if *collapsible && !has_content(&cur) {
                    continue;
                }
                cur_w += w;
                cur.push(Placed { piece: p.clone(), w: *w });
                can_break = true;
            }
            Piece::Edge { w, .. } => {
                cur_w += w;
                cur.push(Placed { piece: p.clone(), w: *w });
            }
            Piece::Atomic { idx } => {
                let aw = ic.atomics[*idx].outer_w();
                if cur_w + aw > cw && has_content(&cur) {
                    finish(&mut cur, &mut lines);
                    cur_w = 0.0;
                }
                cur_w += aw;
                cur.push(Placed { piece: p.clone(), w: aw });
                can_break = true;
            }
            Piece::Newline => {
                finish(&mut cur, &mut lines);
                cur_w = 0.0;
                can_break = true;
            }
        }
    }
    if !cur.is_empty() && (has_content(&cur) || cur.iter().any(|p| matches!(p.piece, Piece::Edge { .. }))) {
        finish(&mut cur, &mut lines);
    }
    // Lines that hold nothing but edges of empty inline elements stay (they have a height).
    place_lines(ic, env, container, lines, cw, cx, cy)
}

fn text_metrics(s: &Style) -> (f32, f32, Font, Face) {
    let (font, f) = face(s);
    let lh = s.line_height_px();
    let size = f.size as f32;
    let above = ((lh - size) / 2.0).floor() + f.ascent() as f32;
    (above, lh - above, font, f)
}

fn place_lines(ic: &mut InlineContent, env: &Env, container: &Style, lines: Vec<Vec<Placed>>, cw: f32, cx: f32, cy: f32) -> f32 {
    let (strut_above, strut_below, _, _) = text_metrics(container);
    let mut y = cy;
    let mut out_lines = Vec::new();
    let mut decos: Vec<Deco> = Vec::new();
    // Inline elements open at the start of the next line (they continue onto it).
    let mut open: Vec<NodeId> = Vec::new();
    let total = lines.len();
    for (li, line) in lines.into_iter().enumerate() {
        let line_w: f32 = line.iter().map(|p| p.w).sum::<f32>() + if li == 0 { container.text_indent } else { 0.0 };
        let shift = match container.text_align {
            TextAlign::Left => 0.0,
            TextAlign::Center => ((cw - line_w) / 2.0).max(0.0),
            TextAlign::Right => (cw - line_w).max(0.0),
        };
        let _ = total;
        let mut x = cx + shift + if li == 0 { container.text_indent } else { 0.0 };
        let mut above = strut_above;
        let mut below = strut_below;
        // (piece, x) pairs; text runs merged afterwards.
        let mut placed: Vec<(Placed, f32)> = Vec::new();
        for p in line {
            match &p.piece {
                Piece::Word { el, .. } | Piece::Space { el, .. } => {
                    let (a, b, _, _) = text_metrics(env.style(*el));
                    above = above.max(a);
                    below = below.max(b);
                }
                Piece::Atomic { idx } => {
                    let a = &ic.atomics[*idx];
                    let asc = atomic_ascent(a);
                    above = above.max(asc);
                    below = below.max(a.outer_h() - asc);
                }
                Piece::Edge { el, .. } => {
                    let (a, b, _, _) = text_metrics(env.style(*el));
                    above = above.max(a);
                    below = below.max(b);
                }
                Piece::Newline => {}
            }
            let w = p.w;
            placed.push((p, x));
            x += w;
        }
        let line_h = above + below;
        let baseline = y + above;
        let line_end = x;
        let mut frags: Vec<Frag> = Vec::new();
        // Deco pieces: start x of each element open on this line.
        let mut starts: Vec<(NodeId, f32, bool)> = open.iter().map(|e| (*e, cx + shift, false)).collect();
        for (p, px) in &placed {
            match &p.piece {
                Piece::Word { text, el, node, w } | Piece::Space { text, el, node, w, .. } => {
                    let s = env.style(*el);
                    let (_, _, font, f) = text_metrics(s);
                    let ty = baseline - f.ascent() as f32;
                    // Merge with the previous text fragment of the same element.
                    if let Some(last) = frags.last_mut() {
                        if let FragKind::Text { text: lt, node: ln, .. } = &mut last.kind {
                            if last.el == *el && *ln == *node && (last.x + last.w - px).abs() < 0.01 {
                                lt.push_str(text);
                                last.w += w;
                                continue;
                            }
                        }
                    }
                    frags.push(Frag { x: *px, y: ty, w: *w, h: f.size as f32, el: *el, kind: FragKind::Text { text: text.clone(), font, size: f.size, node: *node } });
                }
                Piece::Atomic { idx } => {
                    let a = &mut ic.atomics[*idx];
                    let (dx, dy) = (px - a.x + a.margin[3], baseline - atomic_ascent(a) + a.margin[0] - a.y);
                    a.translate(dx, dy);
                    let el = a.node.unwrap_or(DOCUMENT);
                    frags.push(Frag { x: a.x, y: a.y, w: a.w, h: a.h, el, kind: FragKind::Atomic(*idx) });
                }
                Piece::Edge { el, open: true, .. } => {
                    let s = env.style(*el);
                    starts.push((*el, px + s.margin[3].resolve(0.0), true));
                    open.push(*el);
                }
                Piece::Edge { el, open: false, w } => {
                    let s = env.style(*el);
                    if let Some(i) = starts.iter().rposition(|(e, _, _)| e == el) {
                        let (_, sx, first) = starts.remove(i);
                        let ex = px + w - s.margin[1].resolve(0.0);
                        decos.push(deco(env, *el, sx, ex, baseline, first, true));
                    }
                    if let Some(i) = open.iter().rposition(|e| e == el) {
                        open.remove(i);
                    }
                }
                Piece::Newline => {}
            }
        }
        // Elements still open continue on the next line.
        for (el, sx, first) in starts {
            decos.push(deco(env, el, sx, line_end, baseline, first, false));
        }
        out_lines.push(Line { y, h: line_h, baseline, frags });
        y += line_h;
    }
    ic.lines = out_lines;
    ic.decos = decos;
    y - cy
}

/// An inline block's baseline, from the top of its margin box: its first line's
/// text (a form control: its text), else its bottom margin edge (pictures,
/// checkboxes, empty boxes).
fn atomic_ascent(a: &LayoutBox) -> f32 {
    let top = a.y - a.margin[0];
    match &a.replaced {
        Some(Replaced::Control { kind, .. }) if !matches!(kind, ControlKind::Checkbox { .. } | ControlKind::Radio { .. }) => {
            let (_, f) = face(&a.style);
            let (_, cy, _, ch) = a.content_rect();
            let ty = if *kind == ControlKind::TextArea { cy } else { cy + ((ch - f.size as f32) / 2.0).max(0.0).floor() };
            ty + f.ascent() as f32 - top
        }
        Some(_) => a.outer_h(),
        None => a.first_baseline().map(|b| b - top).unwrap_or(a.outer_h()),
    }
}

fn deco(env: &Env, el: NodeId, x0: f32, x1: f32, baseline: f32, first: bool, last: bool) -> Deco {
    let s = env.style(el);
    let (_, f) = face(s);
    let b = s.borders();
    let top = baseline - f.ascent() as f32 - s.padding[0].resolve(0.0) - b[0];
    let h = f.size as f32 + s.padding[0].resolve(0.0) + s.padding[2].resolve(0.0) + b[0] + b[2];
    Deco { el, x: x0, y: top, w: (x1 - x0).max(0.0), h, first, last }
}

// ------------------------------------------------------------------ flexbox

struct FlexItem {
    idx: usize,
    margin: [f32; 4],
    auto_margin: [bool; 4],
    /// Hypothetical main size of the border box.
    base: f32,
    min: f32,
    max: f32,
    main: f32,
    grow: f32,
    shrink: f32,
    frozen: bool,
}

#[allow(clippy::too_many_arguments)]
fn layout_flex(items: &mut [LayoutBox], env: &Env, s: &Style, cw: f32, ch: Option<f32>, cx: f32, cy: f32) -> f32 {
    if items.is_empty() {
        return 0.0;
    }
    let row = s.flex_direction.is_row();
    let gap_main = if row { s.column_gap } else { s.row_gap };
    let gap_cross = if row { s.row_gap } else { s.column_gap };
    let avail_main = if row { Some(cw) } else { ch };
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|i| items[*i].style.order);
    let mut fi: Vec<FlexItem> = Vec::new();
    for &i in &order {
        let it = &mut items[i];
        let st = it.style.clone();
        let (m, bd, p) = edges(&st, cw);
        let auto_margin = [st.margin[0].auto, st.margin[1].auto, st.margin[2].auto, st.margin[3].auto];
        let bp_main = if row { bd[1] + bd[3] + p[1] + p[3] } else { bd[0] + bd[2] + p[0] + p[2] };
        let main_ref = avail_main;
        let to_border = |v: f32| if st.border_box { v.max(bp_main) } else { v + bp_main };
        let basis = st.flex_basis.definite(main_ref).map(to_border);
        let size_prop = if row { st.width } else { st.height };
        let base = match basis.or_else(|| size_prop.definite(main_ref).map(to_border)) {
            Some(b) => b,
            None => {
                if row {
                    let (_, mx) = outer_intrinsic(it, env);
                    mx - m[1] - m[3]
                } else {
                    // The natural height at the width the item gets.
                    let w = if st.width.auto && item_align(s, &st) == Align::Stretch { WidthMode::Exact((cw - m[1] - m[3]).max(0.0)) } else { WidthMode::Fit };
                    it.forced_h = None;
                    layout_block(it, env, cw, None, 0.0, 0.0, w);
                    it.h
                }
            }
        };
        let (min_prop, max_prop) = if row { (st.min_width, st.max_width) } else { (st.min_height, st.max_height) };
        let min = match min_prop.definite(main_ref) {
            Some(v) => to_border(v),
            // min-width: auto keeps text from being crushed below its longest word.
            None if row && st.overflow == Overflow::Visible => {
                let (mn, _) = outer_intrinsic(it, env);
                (mn - m[1] - m[3]).min(base.max(0.0))
            }
            None => 0.0,
        };
        let max = max_prop.definite(main_ref).map(to_border).unwrap_or(f32::INFINITY);
        let base = base.max(0.0);
        fi.push(FlexItem { idx: i, margin: m, auto_margin, base, min, max: max.max(min), main: base.clamp(min, max.max(min)), grow: st.flex_grow, shrink: st.flex_shrink, frozen: false });
    }
    let outer_main = |f: &FlexItem, size: f32| size + if row { f.margin[1] + f.margin[3] } else { f.margin[0] + f.margin[2] };
    // Lines.
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut used = 0.0;
    for (k, f) in fi.iter().enumerate() {
        let o = outer_main(f, f.main);
        if s.flex_wrap && !cur.is_empty() {
            if let Some(a) = avail_main {
                if used + gap_main + o > a {
                    lines.push(std::mem::take(&mut cur));
                    used = 0.0;
                }
            }
        }
        used += o + if cur.is_empty() { 0.0 } else { gap_main };
        cur.push(k);
    }
    lines.push(cur);
    let mut cross_cursor = 0.0;
    let mut line_results: Vec<(Vec<usize>, f32)> = Vec::new();
    for line in &lines {
        // Resolve flexible lengths.
        if let Some(a) = avail_main {
            for _ in 0..4 {
                let gaps = gap_main * line.len().saturating_sub(1) as f32;
                let used: f32 = line.iter().map(|k| outer_main(&fi[*k], if fi[*k].frozen { fi[*k].main } else { fi[*k].base })).sum::<f32>() + gaps;
                let free = a - used;
                let grow: f32 = line.iter().filter(|k| !fi[**k].frozen).map(|k| fi[*k].grow).sum();
                let shrink: f32 = line.iter().filter(|k| !fi[**k].frozen).map(|k| fi[*k].shrink * fi[*k].base).sum();
                let mut clamped = false;
                for &k in line {
                    let f = &mut fi[k];
                    if f.frozen {
                        continue;
                    }
                    let target = if free > 0.0 && grow > 0.0 {
                        f.base + free * f.grow / grow.max(1.0).max(grow)
                    } else if free < 0.0 && shrink > 0.0 {
                        f.base + free * (f.shrink * f.base) / shrink
                    } else {
                        f.base
                    };
                    // flex-grow below 1 in total takes only that share of the space.
                    let target = if free > 0.0 && grow > 0.0 && grow < 1.0 { f.base + free * f.grow } else { target };
                    let c = target.clamp(f.min, f.max);
                    if (c - target).abs() > 0.01 {
                        f.frozen = true;
                        clamped = true;
                    }
                    f.main = c;
                }
                if !clamped {
                    break;
                }
            }
        }
        // Lay the items out at their main size; the line is as tall as its tallest.
        let mut line_cross: f32 = 0.0;
        for &k in line {
            let f = &fi[k];
            let it = &mut items[f.idx];
            it.forced_h = None;
            if row {
                layout_block(it, env, cw, ch, 0.0, 0.0, WidthMode::Exact(f.main));
                line_cross = line_cross.max(it.outer_h());
            } else {
                let w = if it.style.width.auto && item_align(s, &it.style) == Align::Stretch { WidthMode::Exact((cw - f.margin[1] - f.margin[3]).max(0.0)) } else { WidthMode::Fit };
                it.forced_h = Some(f.main);
                layout_block(it, env, cw, ch, 0.0, 0.0, w);
                line_cross = line_cross.max(it.outer_w());
            }
        }
        if lines.len() == 1 {
            if row {
                if let Some(h) = ch {
                    line_cross = h;
                }
            } else {
                line_cross = cw;
            }
        }
        // Stretch (row: heights).
        if row {
            for &k in line {
                let f = &fi[k];
                let it = &mut items[f.idx];
                if item_align(s, &it.style) == Align::Stretch && it.style.height.auto && !f.auto_margin[0] && !f.auto_margin[2] {
                    let h = (line_cross - f.margin[0] - f.margin[2]).max(0.0);
                    if (h - it.h).abs() > 0.01 {
                        let bp_v = it.border[0] + it.border[2] + it.padding[0] + it.padding[2];
                        let h = clamp_h(&it.style, h - bp_v, ch, bp_v) + bp_v;
                        it.forced_h = Some(h);
                        layout_block(it, env, cw, ch, 0.0, 0.0, WidthMode::Exact(f.main));
                    }
                }
            }
        }
        line_results.push((line.clone(), line_cross));
        let _ = cross_cursor;
        cross_cursor += line_cross;
    }
    // Main-axis placement and cross-axis alignment.
    let mut cross_pos = 0.0;
    for (li, (line, line_cross)) in line_results.iter().enumerate() {
        if li > 0 {
            cross_pos += gap_cross;
        }
        let sizes: Vec<f32> = line.iter().map(|k| {
            let it = &items[fi[*k].idx];
            if row { it.outer_w() } else { it.outer_h() }
        }).collect();
        let used: f32 = sizes.iter().sum::<f32>() + gap_main * line.len().saturating_sub(1) as f32;
        let free = avail_main.map(|a| a - used).unwrap_or(0.0);
        let autos: usize = line
            .iter()
            .map(|k| {
                let a = fi[*k].auto_margin;
                if row { a[1] as usize + a[3] as usize } else { a[0] as usize + a[2] as usize }
            })
            .sum();
        let n = line.len() as f32;
        let (mut pos, between) = if autos > 0 || free <= 0.0 {
            (0.0, 0.0)
        } else {
            match s.justify {
                Justify::Start => (0.0, 0.0),
                Justify::End => (free, 0.0),
                Justify::Center => (free / 2.0, 0.0),
                Justify::SpaceBetween => (0.0, if n > 1.0 { free / (n - 1.0) } else { 0.0 }),
                Justify::SpaceAround => (free / n / 2.0, free / n),
                Justify::SpaceEvenly => (free / (n + 1.0), free / (n + 1.0)),
            }
        };
        let auto_share = if autos > 0 && free > 0.0 { free / autos as f32 } else { 0.0 };
        let mut seq: Vec<(usize, f32)> = line.iter().copied().zip(sizes.iter().copied()).collect();
        if s.flex_direction.is_reverse() {
            seq.reverse();
            // Reversed lines start from the end.
            if autos == 0 && free > 0.0 && s.justify == Justify::Start {
                pos = free;
            }
        }
        for (k, size) in seq {
            let f = &fi[k];
            let it = &mut items[f.idx];
            let (lead, trail) = if row { (f.auto_margin[3], f.auto_margin[1]) } else { (f.auto_margin[0], f.auto_margin[2]) };
            if lead {
                pos += auto_share;
            }
            let cross_size = if row { it.outer_h() } else { it.outer_w() };
            let (c_lead, c_trail) = if row { (f.auto_margin[0], f.auto_margin[2]) } else { (f.auto_margin[3], f.auto_margin[1]) };
            let room = line_cross - cross_size;
            let off = if c_lead && c_trail {
                room / 2.0
            } else if c_lead {
                room
            } else if c_trail {
                0.0
            } else {
                match item_align(s, &it.style) {
                    Align::Start | Align::Stretch | Align::Baseline => 0.0,
                    Align::End => room,
                    Align::Center => room / 2.0,
                }
            };
            let (tx, ty) = if row { (cx + pos, cy + cross_pos + off) } else { (cx + cross_pos + off, cy + pos) };
            // Items were laid out with their margin box at (0, 0).
            it.translate(tx, ty);
            relative(it, cw, ch);
            pos += size + gap_main + between;
            if trail {
                pos += auto_share;
            }
        }
        cross_pos += line_cross;
    }
    if row {
        cross_pos
    } else {
        // The column's natural height: the items and the gaps.
        let mut total = 0.0;
        for (line, _) in &line_results {
            let used: f32 = line.iter().map(|k| items[fi[*k].idx].outer_h()).sum::<f32>() + gap_main * line.len().saturating_sub(1) as f32;
            total = f32::max(total, used);
        }
        total
    }
}

fn item_align(container: &Style, item: &Style) -> Align {
    item.align_self.unwrap_or(container.align_items)
}

// ------------------------------------------------------------------ tables

/// Each column's minimum and preferred width (border boxes of its cells).
fn table_columns(t: &TableContent, env: &Env) -> (Vec<f32>, Vec<f32>) {
    let n = t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    let mut mins = vec![0.0f32; n];
    let mut maxs = vec![0.0f32; n];
    for r in &t.rows {
        for (i, c) in r.cells.iter().enumerate() {
            let (mn, mx) = outer_intrinsic(c, env);
            mins[i] = mins[i].max(mn.ceil());
            maxs[i] = maxs[i].max(mx.ceil());
        }
    }
    (mins, maxs)
}

fn layout_table(t: &mut TableContent, env: &Env, s: &Style, cw: f32, cx: f32, cy: f32) -> f32 {
    let mut y = cy;
    for c in t.captions.iter_mut() {
        layout_block(c, env, cw, None, cx, y, WidthMode::Fill);
        y += c.outer_h();
    }
    let (mins, maxs) = table_columns(t, env);
    let n = mins.len();
    let sp = s.border_spacing;
    let avail = (cw - sp * (n as f32 + 1.0)).max(0.0);
    let (smin, smax) = (mins.iter().sum::<f32>(), maxs.iter().sum::<f32>());
    let cols: Vec<f32> = (0..n)
        .map(|i| {
            if smax <= avail {
                // Room for every column's preferred width: share what is left by it.
                let extra = avail - smax;
                maxs[i] + if smax > 0.0 { extra * maxs[i] / smax } else { extra / n as f32 }
            } else if smin >= avail {
                mins[i]
            } else {
                mins[i] + (maxs[i] - mins[i]) * (avail - smin) / (smax - smin).max(0.001)
            }
        })
        .collect();
    y += sp;
    let width = cols.iter().sum::<f32>() + sp * (n as f32 + 1.0);
    for row in t.rows.iter_mut() {
        let mut x = cx + sp;
        let mut h: f32 = 0.0;
        for (i, cell) in row.cells.iter_mut().enumerate() {
            cell.forced_h = None;
            layout_block(cell, env, cols[i], None, x, y, WidthMode::Exact(cols[i] - cell.style.margin[1].resolve(cw) - cell.style.margin[3].resolve(cw)));
            h = h.max(cell.outer_h());
            x += cols[i] + sp;
        }
        // Every cell as tall as the row; its content placed by vertical-align.
        let mut x = cx + sp;
        for (i, cell) in row.cells.iter_mut().enumerate() {
            let natural = cell.outer_h();
            if (h - natural).abs() > 0.01 {
                let inner_h = natural - cell.margin[0] - cell.margin[2];
                cell.forced_h = Some(h - cell.margin[0] - cell.margin[2]);
                layout_block(cell, env, cols[i], None, x, y, WidthMode::Exact(cols[i] - cell.margin[1] - cell.margin[3]));
                let shift = match cell.style.vertical_align {
                    VAlign::Top => 0.0,
                    VAlign::Middle => ((h - natural) / 2.0).floor(),
                    VAlign::Bottom => h - natural,
                };
                if shift > 0.0 {
                    let (bx, by, bw, bh) = (cell.x, cell.y, cell.w, cell.h);
                    for k in cell.children_mut() {
                        k.translate(0.0, shift);
                    }
                    if let Content::Inline(ic) = &mut cell.content {
                        for l in &mut ic.lines {
                            l.y += shift;
                            l.baseline += shift;
                            for f in &mut l.frags {
                                if matches!(f.kind, FragKind::Text { .. }) {
                                    f.y += shift;
                                }
                            }
                        }
                        for d in &mut ic.decos {
                            d.y += shift;
                        }
                    }
                    let _ = (bx, by, bw, bh, inner_h);
                }
            }
            x += cols[i] + sp;
        }
        row.x = cx;
        row.y = y;
        row.w = width;
        row.h = h;
        y += h + sp;
    }
    t.cols = cols;
    if t.rows.is_empty() {
        y -= sp;
    }
    y - cy
}

// ------------------------------------------------------------------ positioned boxes

/// A rectangle: x, y, w, h.
pub type Rect = (f32, f32, f32, f32);

/// Lay out `b`'s absolutely positioned boxes (and those below it). `cb` is
/// the containing block for absolute boxes: the padding box of the nearest
/// positioned ancestor, the viewport at the top.
fn place_positioned(b: &mut LayoutBox, env: &Env, cb: Rect, viewport: Rect) {
    let my_cb = if b.style.position != Position::Static && b.node != Some(DOCUMENT) { b.padding_rect() } else { cb };
    for k in b.children_mut() {
        place_positioned(k, env, my_cb, viewport);
    }
    // Static position: where the element's parent box starts its content.
    let (sx, sy, _, _) = b.content_rect();
    let mut oof = std::mem::take(&mut b.oof);
    for o in oof.iter_mut() {
        let ocb = if o.style.position == Position::Fixed { viewport } else { my_cb };
        layout_abs(o, env, ocb, (sx, sy));
        place_positioned(o, env, o.padding_rect(), viewport);
    }
    b.oof = oof;
}

fn layout_abs(b: &mut LayoutBox, env: &Env, cb: Rect, stat: (f32, f32)) {
    let (cx, cy, cw, ch) = cb;
    let s = b.style.clone();
    let [t, r, bt, l] = s.inset;
    let (m, _, _) = edges(&s, cw);
    let width_auto = s.width.auto && b.replaced.is_none();
    let mode = if width_auto && !l.auto && !r.auto {
        WidthMode::Exact((cw - l.resolve(cw) - r.resolve(cw) - m[1] - m[3]).max(0.0))
    } else {
        WidthMode::Fit
    };
    b.forced_h = None;
    layout_block(b, env, cw, Some(ch), 0.0, 0.0, mode);
    if s.height.auto && !t.auto && !bt.auto && b.replaced.is_none() {
        b.forced_h = Some((ch - t.resolve(ch) - bt.resolve(ch) - m[0] - m[2]).max(0.0));
        layout_block(b, env, cw, Some(ch), 0.0, 0.0, mode);
    }
    let x = if !l.auto {
        cx + l.resolve(cw)
    } else if !r.auto {
        cx + cw - r.resolve(cw) - b.outer_w()
    } else {
        stat.0
    };
    let y = if !t.auto {
        cy + t.resolve(ch)
    } else if !bt.auto {
        cy + ch - bt.resolve(ch) - b.outer_h()
    } else {
        stat.1
    };
    let (ox, oy) = (b.x - b.margin[3], b.y - b.margin[0]);
    b.translate(x - ox, y - oy);
}

/// Lay out the whole document for a viewport `w` lu wide. Returns the root box.
pub fn layout_document(env: &Env) -> LayoutBox {
    let mut root = build(env);
    let (w, h) = env.viewport;
    layout_block(&mut root, env, w, Some(h), 0.0, 0.0, WidthMode::Fill);
    let vp = (0.0, 0.0, w, h);
    place_positioned(&mut root, env, vp, vp);
    root
}
