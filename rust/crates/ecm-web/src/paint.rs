//! Paint: a laid-out box tree into a display list of rects and single-line
//! texts in page coordinates, plus hit boxes that map a point back to the
//! DOM node under it (links, form controls, `:hover`).

use crate::dom::{NodeId, DOCUMENT};
use crate::layout::{self, Content, ControlKind, Env, FragKind, LayoutBox, Marker, Replaced};
use crate::style::{ListStyle, Overflow, Position, Style};
use ecm_ui_proto::Font;

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Rect { x: i32, y: i32, w: i32, h: i32, fill: u32, radius: u16, border_w: u16, border_color: u32 },
    Text { x: i32, y: i32, text: String, font: Font, size: u8, color: u32 },
    /// A picture (`<img>`, `background-image`) by its address, drawn into this rect.
    Image { x: i32, y: i32, w: i32, h: i32, src: String },
    /// Clip what follows (up to the matching [`Item::Pop`]) to this rect.
    Clip { x: i32, y: i32, w: i32, h: i32 },
    /// Draw what follows (up to the matching [`Item::Pop`]) at this opacity.
    Opacity(u8),
    Pop,
}

/// A box on the page belonging to a DOM node (an element).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub node: NodeId,
}

impl Hit {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

#[derive(Clone, Debug, Default)]
pub struct DisplayList {
    pub items: Vec<Item>,
    /// In paint order: the last one containing a point is on top.
    pub hits: Vec<Hit>,
    /// The page's size (at least the viewport's width).
    pub width: f32,
    pub height: f32,
    /// What shows behind the page (the root's or body's background).
    pub background: u32,
    /// `position: fixed` boxes: drawn in viewport coordinates, over the page, not scrolled.
    pub fixed: Vec<Item>,
    pub fixed_hits: Vec<Hit>,
}

impl DisplayList {
    /// The topmost element at a point of the page.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        self.hits.iter().rev().find(|h| h.contains(x, y)).map(|h| h.node)
    }

    /// The topmost element at a point of the viewport scrolled down by `scroll`
    /// (fixed boxes first).
    pub fn hit_test_view(&self, x: f32, y: f32, scroll: f32) -> Option<NodeId> {
        self.fixed_hits.iter().rev().find(|h| h.contains(x, y)).map(|h| h.node).or_else(|| self.hit_test(x, y + scroll))
    }

    /// Where an element is first drawn (anchors, `#fragment`).
    pub fn node_rect(&self, node: NodeId) -> Option<Hit> {
        self.hits.iter().find(|h| h.node == node).copied()
    }
}

struct Painter<'a> {
    env: &'a Env<'a>,
    list: DisplayList,
    /// Painting a fixed box: items and hits go to the fixed layer.
    in_fixed: bool,
}

fn r(v: f32) -> i32 {
    v.round() as i32
}

impl Painter<'_> {
    fn grow(&mut self, x: f32, y: f32) {
        self.list.width = self.list.width.max(x);
        self.list.height = self.list.height.max(y);
    }

    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: u32, radius: f32, border_w: f32, border_color: u32) {
        if w <= 0.0 || h <= 0.0 || (fill & 0xff == 0 && (border_w <= 0.0 || border_color & 0xff == 0)) {
            return;
        }
        let rad = radius.min(w / 2.0).min(h / 2.0).max(0.0);
        self.list.items.push(Item::Rect { x: r(x), y: r(y), w: r(w).max(1), h: r(h).max(1), fill, radius: rad as u16, border_w: border_w as u16, border_color });
        self.grow(x + w, y + h);
    }

    fn text(&mut self, x: f32, y: f32, text: &str, font: Font, size: u8, color: u32, s: &Style) {
        if text.is_empty() || color & 0xff == 0 {
            return;
        }
        let t: String = text.chars().take(512).collect();
        self.list.items.push(Item::Text { x: r(x), y: r(y), text: t.clone(), font, size, color });
        if s.bold {
            self.list.items.push(Item::Text { x: r(x) + 1, y: r(y), text: t, font, size, color });
        }
    }

    /// Background and borders of a box.
    fn background(&mut self, b: &LayoutBox) {
        let s = &b.style;
        if s.hidden {
            return;
        }
        let bw = b.border;
        let colors = [s.border_colour(0), s.border_colour(1), s.border_colour(2), s.border_colour(3)];
        let uniform = bw.iter().all(|w| *w == bw[0]) && colors.iter().all(|c| *c == colors[0]);
        if uniform {
            self.rect(b.x, b.y, b.w, b.h, s.background, s.radius, bw[0], colors[0]);
        } else {
            self.rect(b.x, b.y, b.w, b.h, s.background, s.radius, 0.0, 0);
            let (x, y, w, h) = (b.x, b.y, b.w, b.h);
            self.rect(x, y, w, bw[0], colors[0], 0.0, 0.0, 0);
            self.rect(x + w - bw[1], y, bw[1], h, colors[1], 0.0, 0.0, 0);
            self.rect(x, y + h - bw[2], w, bw[2], colors[2], 0.0, 0.0, 0);
            self.rect(x, y, bw[3], h, colors[3], 0.0, 0.0, 0);
        }
        if let Some(url) = s.background_image.as_deref().and_then(|u| self.env.image_url(u)) {
            if let Some((iw, ih)) = (self.env.images)(&url) {
                // Drawn once at the padding box's corner (no repeat in v1), clipped to it.
                let (px, py, pw, ph) = b.padding_rect();
                self.list.items.push(Item::Clip { x: r(px), y: r(py), w: r(pw), h: r(ph) });
                self.list.items.push(Item::Image { x: r(px), y: r(py), w: r(iw), h: r(ih), src: url });
                self.list.items.push(Item::Pop);
            }
        }
        self.grow(b.x + b.w, b.y + b.h);
    }

    fn paint_box(&mut self, b: &LayoutBox) {
        let s = &b.style;
        let opacity = (s.opacity * 255.0).round() as u8;
        if opacity == 0 {
            return;
        }
        if opacity < 255 {
            self.list.items.push(Item::Opacity(opacity));
        }
        self.background(b);
        if let Some(n) = b.node.filter(|n| *n != DOCUMENT) {
            self.hit(Hit { x: b.x, y: b.y, w: b.w, h: b.h, node: n });
        }
        // Negative z-index: behind the box's content.
        self.positioned(b, |z| z < 0);
        if !s.hidden {
            if let Some(m) = &b.marker {
                self.marker(m);
            }
        }
        if let Some(rep) = &b.replaced {
            self.replaced(b, rep);
        }
        let clip = s.overflow != Overflow::Visible;
        if clip {
            let (x, y, w, h) = b.padding_rect();
            self.list.items.push(Item::Clip { x: r(x), y: r(y), w: r(w), h: r(h) });
        }
        match &b.content {
            Content::Empty => {}
            Content::Blocks(kids) | Content::Flex(kids) => {
                for k in kids {
                    self.paint_box(k);
                }
            }
            Content::Table(t) => {
                for c in &t.captions {
                    self.paint_box(c);
                }
                for row in &t.rows {
                    if !row.style.hidden {
                        self.rect(row.x, row.y, row.w, row.h, row.style.background, 0.0, 0.0, 0);
                    }
                    if let Some(n) = row.node {
                        self.hit(Hit { x: row.x, y: row.y, w: row.w, h: row.h, node: n });
                    }
                    for c in &row.cells {
                        self.paint_box(c);
                    }
                }
            }
            Content::Inline(ic) => {
                for d in &ic.decos {
                    let ds = self.env.style(d.el).clone();
                    if !ds.hidden {
                        let bw = ds.borders();
                        let fill = ds.background;
                        let bc = ds.border_colour(0);
                        let uniform = bw.iter().all(|w| *w == bw[0]);
                        if uniform && d.first && d.last {
                            self.rect(d.x, d.y, d.w, d.h, fill, ds.radius, bw[0], bc);
                        } else {
                            self.rect(d.x, d.y, d.w, d.h, fill, 0.0, 0.0, 0);
                            self.rect(d.x, d.y, d.w, bw[0], ds.border_colour(0), 0.0, 0.0, 0);
                            self.rect(d.x, d.y + d.h - bw[2], d.w, bw[2], ds.border_colour(2), 0.0, 0.0, 0);
                            if d.first {
                                self.rect(d.x, d.y, bw[3], d.h, ds.border_colour(3), 0.0, 0.0, 0);
                            }
                            if d.last {
                                self.rect(d.x + d.w - bw[1], d.y, bw[1], d.h, ds.border_colour(1), 0.0, 0.0, 0);
                            }
                        }
                    }
                    self.hit(Hit { x: d.x, y: d.y, w: d.w, h: d.h, node: d.el });
                }
                for line in &ic.lines {
                    for f in &line.frags {
                        match &f.kind {
                            FragKind::Text { text, font, size, .. } => {
                                let ts = self.env.style(f.el).clone();
                                self.grow(f.x + f.w, line.y + line.h);
                                if f.el != DOCUMENT {
                                    self.hit(Hit { x: f.x, y: line.y, w: f.w, h: line.h, node: f.el });
                                }
                                if ts.hidden || text.trim().is_empty() && !ts.underline {
                                    continue;
                                }
                                self.text(f.x, f.y, text, *font, *size, ts.color, &ts);
                                let thick = (*size as f32 / 12.0).floor().max(1.0);
                                let (_, face) = layout::face(&ts);
                                if ts.underline {
                                    self.rect(f.x, f.y + face.ascent() as f32 + thick, f.w, thick, ts.color, 0.0, 0.0, 0);
                                }
                                if ts.line_through {
                                    self.rect(f.x, f.y + (*size as f32 * 0.55).round(), f.w, thick, ts.color, 0.0, 0.0, 0);
                                }
                            }
                            FragKind::Atomic(i) => self.paint_box(&ic.atomics[*i]),
                        }
                    }
                    self.grow(0.0, line.y + line.h);
                }
            }
        }
        if clip {
            self.list.items.push(Item::Pop);
        }
        // Positioned boxes over the content, by z-index (auto = 0), in tree order.
        self.positioned(b, |z| z >= 0);
        if opacity < 255 {
            self.list.items.push(Item::Pop);
        }
    }

    fn hit(&mut self, h: Hit) {
        if self.in_fixed {
            self.list.fixed_hits.push(h);
        } else {
            self.list.hits.push(h);
        }
    }

    /// Paint `b`'s absolutely positioned boxes whose z-index passes `which`
    /// (fixed ones go to the fixed layer).
    fn positioned(&mut self, b: &LayoutBox, which: impl Fn(i32) -> bool) {
        let mut order: Vec<&LayoutBox> = b.oof.iter().filter(|o| which(o.style.z_index.unwrap_or(0))).collect();
        order.sort_by_key(|o| o.style.z_index.unwrap_or(0));
        for o in order {
            if o.style.position == Position::Fixed && !self.in_fixed {
                let page = std::mem::take(&mut self.list.items);
                let (w, h) = (self.list.width, self.list.height);
                self.in_fixed = true;
                self.paint_box(o);
                self.in_fixed = false;
                let fixed = std::mem::replace(&mut self.list.items, page);
                self.list.fixed.extend(fixed);
                self.list.width = w;
                self.list.height = h;
            } else {
                self.paint_box(o);
            }
        }
    }

    fn marker(&mut self, m: &Marker) {
        match m {
            Marker::Bullet { x, y, d, kind, color } => match kind {
                ListStyle::Circle => self.rect(*x, *y, *d, *d, 0, *d / 2.0, 1.0, *color),
                ListStyle::Square => self.rect(*x, *y, *d, *d, *color, 0.0, 0.0, 0),
                _ => self.rect(*x, *y, *d, *d, *color, *d / 2.0, 0.0, 0),
            },
            Marker::Text { x, y, text, size, color } => {
                let s = Style::initial(self.env.ua);
                self.text(*x, *y, text, Font::Ui, *size, *color, &s);
            }
        }
    }

    fn replaced(&mut self, b: &LayoutBox, rep: &Replaced) {
        let s = &b.style;
        if s.hidden {
            return;
        }
        let (cx, cy, cw, ch) = b.content_rect();
        let (font, face) = layout::face(s);
        let ty = cy + ((ch - face.size as f32) / 2.0).max(0.0).floor();
        match rep {
            Replaced::Image { alt } => {
                if let Some(n) = b.node {
                    if self.env.img_size(n).is_some() {
                        if let Some(src) = self.env.dom.attr(n, "src").and_then(|s| self.env.image_url(s)) {
                            self.list.items.push(Item::Image { x: r(cx), y: r(cy), w: r(cw), h: r(ch), src });
                            self.grow(cx + cw, cy + ch);
                            return;
                        }
                    }
                }
                // Not loaded: a frame with the alt text.
                self.rect(cx, cy, cw, ch, 0, 0.0, 1.0, self.env.ua.border);
                self.list.items.push(Item::Clip { x: r(cx), y: r(cy), w: r(cw), h: r(ch) });
                self.text(cx + 2.0, ty, &layout::latin1(alt), font, face.size, self.env.ua.muted, s);
                self.list.items.push(Item::Pop);
            }
            Replaced::Control { kind, text } => match kind {
                ControlKind::Checkbox { checked } | ControlKind::Radio { checked } => {
                    if *checked {
                        let radius = if matches!(kind, ControlKind::Radio { .. }) { cw / 2.0 } else { 1.0 };
                        self.rect(cx + 2.0, cy + 2.0, cw - 4.0, ch - 4.0, self.env.ua.link, radius, 0.0, 0);
                    }
                }
                _ => {
                    self.list.items.push(Item::Clip { x: r(cx), y: r(cy), w: r(cw), h: r(ch) });
                    let lines: Vec<&str> = if *kind == ControlKind::TextArea { text.lines().collect() } else { vec![text.as_str()] };
                    let color = if matches!(kind, ControlKind::Text) && b.node.and_then(|n| self.env.dom.attr(n, "value")).is_none_or(|v| v.is_empty()) {
                        self.env.ua.muted
                    } else {
                        s.color
                    };
                    let mut tx = if *kind == ControlKind::Button { cx + ((cw - face.text_width(text) as f32) / 2.0).max(0.0).floor() } else { cx };
                    // The focused field: its caret, and the text moved left to keep the caret in view.
                    let caret = self.env.caret.filter(|(n, _)| Some(*n) == b.node).map(|(_, c)| c);
                    let mut caret_at = None;
                    if let (Some(c), Some(n)) = (caret, b.node) {
                        let value = crate::forms::value(self.env.dom, n);
                        let shown: String = if matches!(crate::forms::control(self.env.dom, n), Some(crate::forms::Control::Text { password: true })) {
                            "*".repeat(value.chars().count())
                        } else {
                            value
                        };
                        let before: String = shown.chars().take(c).collect();
                        if *kind == ControlKind::TextArea {
                            let line = before.matches('\n').count();
                            let col = before.rsplit('\n').next().unwrap_or("");
                            caret_at = Some((cx + face.text_width(&layout::latin1(col)) as f32, cy + line as f32 * s.line_height_px()));
                        } else {
                            let w = face.text_width(&layout::latin1(&before)) as f32;
                            if w > cw - 2.0 {
                                tx -= w - (cw - 2.0);
                            }
                            caret_at = Some((tx + w, ty));
                        }
                    }
                    for (i, l) in lines.iter().enumerate() {
                        let ly = if *kind == ControlKind::TextArea { cy + i as f32 * s.line_height_px() } else { ty };
                        self.text(tx, ly, &layout::latin1(l), font, face.size, color, s);
                    }
                    if let Some((x, y)) = caret_at {
                        self.rect(x, y, 1.0, face.size as f32, s.color, 0.0, 0.0, 0);
                    }
                    self.list.items.push(Item::Pop);
                    if *kind == ControlKind::Select {
                        self.text(cx + cw - 10.0, ty, "v", font, face.size, s.color, s);
                    }
                }
            },
        }
    }
}

/// Paint a laid-out document.
pub fn paint(root: &LayoutBox, env: &Env) -> DisplayList {
    let mut p = Painter { env, list: DisplayList::default(), in_fixed: false };
    p.list.width = env.viewport.0;
    p.paint_box(root);
    p.list.height = p.list.height.ceil();
    // The canvas takes the root's background, or the body's.
    let dom = env.dom;
    let bg = |tag: &str| dom.find(tag).and_then(|n| env.styles[n].as_ref()).map(|s| s.background).filter(|c| c & 0xff != 0);
    p.list.background = bg("html").or_else(|| bg("body")).unwrap_or(env.ua.background);
    p.list
}
