//! The cascade: every element's computed [`Style`] from the browser's own
//! sheet (in the shell theme's colours), the page's sheets in order, and
//! `style=""` attributes, by importance, specificity and source order, with
//! inheritance and `inherit` / `initial` / `unset`.
//!
//! Font sizes are computed in px (lu) and drawn at the nearest size the ECM
//! fonts have (8, 12, 16, 24, 32). There is no bold or italic font: bold is
//! drawn twice, one lu apart, and italic text is drawn as it is.

use crate::css::{self, Decl, Dynamic, Stylesheet};
use crate::dom::{Dom, NodeData, NodeId};

/// A length: `px + pct% of the reference`, or `auto`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Len {
    pub px: f32,
    pub pct: f32,
    pub auto: bool,
}

impl Len {
    pub const AUTO: Len = Len { px: 0.0, pct: 0.0, auto: true };
    pub const ZERO: Len = Len { px: 0.0, pct: 0.0, auto: false };

    pub fn px(v: f32) -> Len {
        Len { px: v, pct: 0.0, auto: false }
    }

    pub fn pct(v: f32) -> Len {
        Len { px: 0.0, pct: v, auto: false }
    }

    /// The value against `base` (auto = 0).
    pub fn resolve(&self, base: f32) -> f32 {
        if self.auto {
            0.0
        } else {
            self.px + self.pct * base / 100.0
        }
    }

    /// None for `auto`, or a percentage of an unknown reference.
    pub fn definite(&self, base: Option<f32>) -> Option<f32> {
        if self.auto {
            return None;
        }
        if self.pct != 0.0 {
            return base.map(|b| self.px + self.pct * b / 100.0);
        }
        Some(self.px)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
    None,
    Block,
    Inline,
    InlineBlock,
    Flex,
    InlineFlex,
    ListItem,
}

impl Display {
    pub fn is_inline_level(self) -> bool {
        matches!(self, Display::Inline | Display::InlineBlock | Display::InlineFlex)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhiteSpace {
    Normal,
    NoWrap,
    Pre,
    PreWrap,
    PreLine,
}

impl WhiteSpace {
    /// Spaces and tabs are kept as they are.
    pub fn keeps_spaces(self) -> bool {
        matches!(self, WhiteSpace::Pre | WhiteSpace::PreWrap)
    }
    /// Newlines break the line.
    pub fn keeps_newlines(self) -> bool {
        matches!(self, WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::PreLine)
    }
    /// Lines wrap at the box's edge.
    pub fn wraps(self) -> bool {
        !matches!(self, WhiteSpace::Pre | WhiteSpace::NoWrap)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

impl FlexDirection {
    pub fn is_row(self) -> bool {
        matches!(self, FlexDirection::Row | FlexDirection::RowReverse)
    }
    pub fn is_reverse(self) -> bool {
        matches!(self, FlexDirection::RowReverse | FlexDirection::ColumnReverse)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Justify {
    Start,
    End,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Stretch,
    Start,
    End,
    Center,
    Baseline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListStyle {
    None,
    Disc,
    Circle,
    Square,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overflow {
    Visible,
    Hidden,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextTransform {
    None,
    Upper,
    Lower,
    Capitalize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    Normal,
    /// A multiple of the font size.
    Factor(f32),
    Px(f32),
}

/// The font sizes the ECM fonts are drawn at.
pub const FONT_SIZES: [u8; 5] = [8, 12, 16, 24, 32];

/// The nearest drawable size to `px`.
pub fn snap_font_size(px: f32) -> u8 {
    let mut best = FONT_SIZES[0];
    for s in FONT_SIZES {
        if (s as f32 - px).abs() < (best as f32 - px).abs() {
            best = s;
        }
    }
    best
}

/// One element's computed style.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    // ---- inherited
    pub color: u32,
    /// Computed px (before snapping).
    pub font_size: f32,
    pub mono: bool,
    pub bold: bool,
    pub italic: bool,
    pub text_align: TextAlign,
    pub line_height: LineHeight,
    pub white_space: WhiteSpace,
    pub list_style: ListStyle,
    pub hidden: bool,
    pub text_transform: TextTransform,
    // ---- text decorations propagate to the text inside
    pub underline: bool,
    pub line_through: bool,
    // ---- not inherited
    pub display: Display,
    pub position: Position,
    /// top, right, bottom, left
    pub inset: [Len; 4],
    pub margin: [Len; 4],
    pub padding: [Len; 4],
    pub border_width: [f32; 4],
    pub border_style: [bool; 4],
    /// None = currentcolor.
    pub border_color: [Option<u32>; 4],
    pub radius: f32,
    pub width: Len,
    pub height: Len,
    pub min_width: Len,
    pub min_height: Len,
    pub max_width: Len,
    pub max_height: Len,
    pub border_box: bool,
    pub background: u32,
    pub background_image: Option<String>,
    pub flex_direction: FlexDirection,
    pub flex_wrap: bool,
    pub justify: Justify,
    pub align_items: Align,
    pub align_self: Option<Align>,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Len,
    pub order: i32,
    pub row_gap: f32,
    pub column_gap: f32,
    pub overflow: Overflow,
    pub opacity: f32,
    pub text_indent: f32,
}

impl Style {
    /// The root's starting point.
    pub fn initial(ua: &UaColors) -> Style {
        Style {
            color: ua.text,
            font_size: 12.0,
            mono: false,
            bold: false,
            italic: false,
            text_align: TextAlign::Left,
            line_height: LineHeight::Normal,
            white_space: WhiteSpace::Normal,
            list_style: ListStyle::Disc,
            hidden: false,
            text_transform: TextTransform::None,
            underline: false,
            line_through: false,
            display: Display::Inline,
            position: Position::Static,
            inset: [Len::AUTO; 4],
            margin: [Len::ZERO; 4],
            padding: [Len::ZERO; 4],
            border_width: [3.0; 4],
            border_style: [false; 4],
            border_color: [None; 4],
            radius: 0.0,
            width: Len::AUTO,
            height: Len::AUTO,
            min_width: Len::AUTO,
            min_height: Len::AUTO,
            max_width: Len::AUTO,
            max_height: Len::AUTO,
            border_box: false,
            background: 0,
            background_image: None,
            flex_direction: FlexDirection::Row,
            flex_wrap: false,
            justify: Justify::Start,
            align_items: Align::Stretch,
            align_self: None,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Len::AUTO,
            order: 0,
            row_gap: 0.0,
            column_gap: 0.0,
            overflow: Overflow::Visible,
            opacity: 1.0,
            text_indent: 0.0,
        }
    }

    /// A child's starting style: the inherited properties of `self`, the rest initial.
    pub fn inherit_from(&self, ua: &UaColors) -> Style {
        let mut s = Style::initial(ua);
        s.color = self.color;
        s.font_size = self.font_size;
        s.mono = self.mono;
        s.bold = self.bold;
        s.italic = self.italic;
        s.text_align = self.text_align;
        s.line_height = self.line_height;
        s.white_space = self.white_space;
        s.list_style = self.list_style;
        s.hidden = self.hidden;
        s.text_transform = self.text_transform;
        s.underline = self.underline;
        s.line_through = self.line_through;
        s
    }

    /// The size the text is drawn at.
    pub fn draw_size(&self) -> u8 {
        snap_font_size(self.font_size)
    }

    /// The line box height of this style's text, lu.
    pub fn line_height_px(&self) -> f32 {
        let size = self.draw_size() as f32;
        match self.line_height {
            LineHeight::Normal => (size * 1.25).round(),
            LineHeight::Factor(f) => (size * f).round().max(1.0),
            LineHeight::Px(p) => p.round().max(1.0),
        }
    }

    /// Used border widths (0 where the style is none).
    pub fn borders(&self) -> [f32; 4] {
        let mut b = [0.0; 4];
        for i in 0..4 {
            if self.border_style[i] {
                b[i] = self.border_width[i];
            }
        }
        b
    }

    pub fn border_colour(&self, side: usize) -> u32 {
        self.border_color[side].unwrap_or(self.color)
    }
}

/// The theme colours unstyled pages use, so they look native in light and dark.
#[derive(Clone, Debug, PartialEq)]
pub struct UaColors {
    pub text: u32,
    pub background: u32,
    pub link: u32,
    pub muted: u32,
    pub border: u32,
    pub surface: u32,
    pub button: u32,
}

impl Default for UaColors {
    fn default() -> Self {
        // ecm-ui's default (dark) theme.
        UaColors { text: 0xe6e9eeff, background: 0x1e2228ff, link: 0x6faeffff, muted: 0x8c95a3ff, border: 0x454c58ff, surface: 0x2a2f37ff, button: 0x3a404bff }
    }
}

fn css_colour(c: u32) -> String {
    format!("#{c:08x}")
}

/// The browser's own style sheet.
pub fn ua_sheet(ua: &UaColors) -> Stylesheet {
    let src = format!(
        "html, body, div, section, header, footer, nav, main, article, aside, p, h1, h2, h3, h4, h5, h6, ul, ol, li, pre, \
         blockquote, form, hr, address, figure, figcaption, fieldset, legend, dl, dt, dd, table, thead, tbody, tfoot, caption, \
         center, details, summary, menu, dir, listing, xmp, plaintext {{ display: block }}
         head, title, style, script, meta, link, base, template, noscript, iframe, video, audio, canvas, object, embed, svg, \
         datalist, param, source, track, map, area {{ display: none }}
         li {{ display: list-item }}
         body {{ margin: 8px }}
         p, dl {{ margin: 8px 0 }}
         h1 {{ font-size: 32px; margin: 12px 0; font-weight: bold }}
         h2 {{ font-size: 24px; margin: 10px 0; font-weight: bold }}
         h3 {{ font-size: 16px; margin: 8px 0; font-weight: bold }}
         h4 {{ font-size: 16px; margin: 8px 0; font-weight: bold }}
         h5, h6 {{ font-size: 12px; margin: 8px 0; font-weight: bold }}
         ul, ol, menu, dir {{ margin: 8px 0; padding-left: 24px }}
         ol {{ list-style-type: decimal }}
         ul ul, ol ul, ul ol, ol ol {{ margin: 0 }}
         ul ul {{ list-style-type: circle }}
         ul ul ul {{ list-style-type: square }}
         dd {{ margin-left: 24px }}
         pre, xmp, listing, plaintext {{ white-space: pre; font-family: monospace; margin: 8px 0 }}
         code, kbd, samp, tt, var {{ font-family: monospace }}
         b, strong, th, dt, legend, summary {{ font-weight: bold }}
         em, i, cite, dfn, address {{ font-style: italic }}
         u, ins {{ text-decoration: underline }}
         s, strike, del {{ text-decoration: line-through }}
         small, sub, sup {{ font-size: smaller }}
         big {{ font-size: larger }}
         blockquote, figure {{ margin: 8px 24px }}
         hr {{ border-top: 1px solid {border}; margin: 8px 0 }}
         center {{ text-align: center }}
         a:link {{ color: {link}; text-decoration: underline }}
         mark {{ background-color: #ffe066; color: #000000 }}
         table {{ margin: 4px 0 }}
         tr {{ display: flex }}
         td, th {{ display: block; flex: 1 1 0; padding: 2px 4px }}
         caption {{ text-align: center }}
         img {{ display: inline-block }}
         input, select, textarea, button {{ display: inline-block; border: 1px solid {border}; padding: 2px 4px; \
            background-color: {surface}; color: {text}; border-radius: 2px }}
         button, input[type=submit], input[type=button], input[type=reset] {{ background-color: {button} }}
         input[type=hidden] {{ display: none }}
         textarea {{ white-space: pre-wrap; font-family: monospace }}
         fieldset {{ border: 1px solid {border}; padding: 4px 8px; margin: 4px 0 }}",
        border = css_colour(ua.border),
        link = css_colour(ua.link),
        surface = css_colour(ua.surface),
        button = css_colour(ua.button),
        text = css_colour(ua.text),
    );
    Stylesheet::parse(&src)
}

/// Everything the cascade needs besides the DOM.
pub struct StyleEnv<'a> {
    pub ua_colors: &'a UaColors,
    pub ua_sheet: &'a Stylesheet,
    /// The page's sheets in document order.
    pub sheets: &'a [Stylesheet],
    /// The viewport (media queries, vw, vh).
    pub viewport: (f32, f32),
    pub dynamic: &'a Dynamic,
}

/// The computed style of every element (None for other nodes), indexed by node id.
pub fn compute(dom: &Dom, env: &StyleEnv) -> Vec<Option<Style>> {
    let mut out: Vec<Option<Style>> = vec![None; dom.nodes.len()];
    let root = Style::initial(env.ua_colors);
    let mut root_font = 12.0;
    out[crate::dom::DOCUMENT] = Some(root.clone());
    // Inline style attributes parsed once.
    for id in dom.descendants(crate::dom::DOCUMENT) {
        if dom.element(id).is_none() {
            continue;
        }
        let parent = dom.parent_element(id).and_then(|p| out[p].clone()).unwrap_or_else(|| root.clone());
        let mut decls: Vec<(u8, u32, usize, &Decl)> = Vec::new();
        let mut order = 0usize;
        let all = std::iter::once((env.ua_sheet, 0u8)).chain(env.sheets.iter().map(|s| (s, 1u8)));
        for (sheet, origin) in all {
            for rule in &sheet.rules {
                order += 1;
                if !rule.media.iter().all(|m| m.matches(env.viewport.0, env.viewport.1)) {
                    continue;
                }
                let Some(spec) = rule.selectors.iter().filter(|s| s.matches(dom, id, env.dynamic)).map(|s| s.specificity).max() else {
                    continue;
                };
                for d in &rule.decls {
                    // UA normal 0, author normal 1, author important 2, UA important 3.
                    let class = match (origin, d.important) {
                        (0, false) => 0,
                        (_, false) => 1,
                        (0, true) => 3,
                        (_, true) => 2,
                    };
                    decls.push((class, spec, order, d));
                }
            }
        }
        let inline: Vec<Decl> = dom.attr(id, "style").map(css::parse_declarations).unwrap_or_default();
        for d in &inline {
            order += 1;
            decls.push((if d.important { 2 } else { 1 }, 1_000_000, order, d));
        }
        decls.sort_by_key(|(c, s, o, _)| (*c, *s, *o));
        let mut s = parent.inherit_from(env.ua_colors);
        let initial = Style::initial(env.ua_colors);
        let ctx = Ctx { parent: &parent, initial: &initial, root_font, viewport: env.viewport };
        // Font size first: em lengths depend on it.
        for (_, _, _, d) in decls.iter().filter(|(_, _, _, d)| d.name == "font-size") {
            apply(&mut s, d, &ctx);
        }
        for (_, _, _, d) in decls.iter().filter(|(_, _, _, d)| d.name != "font-size") {
            apply(&mut s, d, &ctx);
        }
        // Legacy presentational attributes (below every CSS rule but the UA's).
        presentational(dom, id, &mut s);
        if dom.parent_element(id).is_none() {
            root_font = s.font_size;
        }
        out[id] = Some(s);
    }
    out
}

/// `<font color>`, `bgcolor`, `align`, `width`/`height`, `hidden`.
fn presentational(dom: &Dom, id: NodeId, s: &mut Style) {
    let Some(e) = dom.element(id) else { return };
    let has_style = |p: &str| e.attr("style").is_some_and(|st| st.contains(p));
    if let Some(c) = e.attr("bgcolor").and_then(|c| parse_color(c, s.color)) {
        if !has_style("background") {
            s.background = c;
        }
    }
    if e.tag == "font" {
        if let Some(c) = e.attr("color").and_then(|c| parse_color(c, s.color)) {
            s.color = c;
        }
    }
    if let Some(a) = e.attr("align") {
        if !matches!(e.tag.as_str(), "img") && !has_style("text-align") {
            match a.to_ascii_lowercase().as_str() {
                "center" => s.text_align = TextAlign::Center,
                "right" => s.text_align = TextAlign::Right,
                "left" => s.text_align = TextAlign::Left,
                _ => {}
            }
        }
    }
    if matches!(e.tag.as_str(), "img" | "table" | "td" | "th" | "input" | "canvas" | "textarea") {
        for (attr, field) in [("width", 0), ("height", 1)] {
            if let Some(v) = e.attr(attr) {
                if has_style(attr) {
                    continue;
                }
                let v = v.trim();
                let len = if let Some(p) = v.strip_suffix('%') {
                    p.trim().parse::<f32>().ok().map(Len::pct)
                } else {
                    v.trim_end_matches("px").parse::<f32>().ok().map(Len::px)
                };
                if let Some(l) = len {
                    if field == 0 {
                        s.width = l;
                    } else {
                        s.height = l;
                    }
                }
            }
        }
    }
    if e.attr("hidden").is_some() && !has_style("display") {
        s.display = Display::None;
    }
}

struct Ctx<'a> {
    parent: &'a Style,
    initial: &'a Style,
    root_font: f32,
    viewport: (f32, f32),
}

/// Copy property `name` from `src` (inherit / initial).
fn copy_prop(s: &mut Style, name: &str, src: &Style) {
    let side = |n: &str| SIDE_NAMES.iter().position(|x| n.contains(x));
    match name {
        "color" => s.color = src.color,
        "font-size" => s.font_size = src.font_size,
        "font-family" => s.mono = src.mono,
        "font-weight" => s.bold = src.bold,
        "font-style" => s.italic = src.italic,
        "text-align" => s.text_align = src.text_align,
        "line-height" => s.line_height = src.line_height,
        "white-space" => s.white_space = src.white_space,
        "list-style-type" => s.list_style = src.list_style,
        "visibility" => s.hidden = src.hidden,
        "text-transform" => s.text_transform = src.text_transform,
        "text-decoration" => {
            s.underline = src.underline;
            s.line_through = src.line_through;
        }
        "display" => s.display = src.display,
        "position" => s.position = src.position,
        "top" | "right" | "bottom" | "left" => {
            let i = SIDE_NAMES.iter().position(|x| *x == name).unwrap_or(0);
            s.inset[i] = src.inset[i];
        }
        n if n.starts_with("margin-") => {
            if let Some(i) = side(n) {
                s.margin[i] = src.margin[i];
            }
        }
        n if n.starts_with("padding-") => {
            if let Some(i) = side(n) {
                s.padding[i] = src.padding[i];
            }
        }
        n if n.starts_with("border-") && n.ends_with("-width") => {
            if let Some(i) = side(n) {
                s.border_width[i] = src.border_width[i];
            }
        }
        n if n.starts_with("border-") && n.ends_with("-style") => {
            if let Some(i) = side(n) {
                s.border_style[i] = src.border_style[i];
            }
        }
        n if n.starts_with("border-") && n.ends_with("-color") => {
            if let Some(i) = side(n) {
                s.border_color[i] = src.border_color[i];
            }
        }
        "border-radius" => s.radius = src.radius,
        "width" => s.width = src.width,
        "height" => s.height = src.height,
        "min-width" => s.min_width = src.min_width,
        "min-height" => s.min_height = src.min_height,
        "max-width" => s.max_width = src.max_width,
        "max-height" => s.max_height = src.max_height,
        "box-sizing" => s.border_box = src.border_box,
        "background-color" => s.background = src.background,
        "background-image" => s.background_image = src.background_image.clone(),
        "flex-direction" => s.flex_direction = src.flex_direction,
        "flex-wrap" => s.flex_wrap = src.flex_wrap,
        "justify-content" => s.justify = src.justify,
        "align-items" => s.align_items = src.align_items,
        "align-self" => s.align_self = src.align_self,
        "flex-grow" => s.flex_grow = src.flex_grow,
        "flex-shrink" => s.flex_shrink = src.flex_shrink,
        "flex-basis" => s.flex_basis = src.flex_basis,
        "order" => s.order = src.order,
        "row-gap" => s.row_gap = src.row_gap,
        "column-gap" => s.column_gap = src.column_gap,
        "overflow" => s.overflow = src.overflow,
        "opacity" => s.opacity = src.opacity,
        "text-indent" => s.text_indent = src.text_indent,
        _ => {}
    }
}

const SIDE_NAMES: [&str; 4] = ["top", "right", "bottom", "left"];

const INHERITED: &[&str] = &[
    "color", "font-size", "font-family", "font-weight", "font-style", "text-align", "line-height", "white-space", "list-style-type",
    "visibility", "text-transform",
];

fn apply(s: &mut Style, d: &Decl, cx: &Ctx) {
    let v = d.value.trim();
    let lower = v.to_ascii_lowercase();
    match lower.as_str() {
        "inherit" => return copy_prop(s, &d.name, cx.parent),
        "initial" => return copy_prop(s, &d.name, cx.initial),
        "unset" | "revert" => {
            let src = if INHERITED.contains(&d.name.as_str()) { cx.parent } else { cx.initial };
            return copy_prop(s, &d.name, src);
        }
        _ => {}
    }
    let em = s.font_size;
    let len = |v: &str| parse_len(v, em, cx.root_font, cx.viewport);
    let side = SIDE_NAMES.iter().position(|x| d.name.contains(x));
    match d.name.as_str() {
        "color" => {
            if let Some(c) = parse_color(v, cx.parent.color) {
                s.color = c;
            }
        }
        "font-size" => {
            let p = cx.parent.font_size;
            let size = match lower.as_str() {
                "xx-small" | "x-small" => Some(8.0),
                "small" => Some(10.0),
                "medium" => Some(12.0),
                "large" => Some(14.0),
                "x-large" => Some(18.0),
                "xx-large" => Some(24.0),
                "xxx-large" => Some(32.0),
                "smaller" => Some(p * 0.75),
                "larger" => Some(p * 1.33),
                _ => match parse_len(v, p, cx.root_font, cx.viewport) {
                    Some(l) if !l.auto => Some(l.px + l.pct * p / 100.0),
                    _ => None,
                },
            };
            if let Some(sz) = size {
                s.font_size = sz.clamp(1.0, 200.0);
            }
        }
        "font-family" => s.mono = ["monospace", "mono", "courier", "consolas", "menlo", "monaco", "fixed"].iter().any(|m| lower.contains(m)),
        "font-weight" => {
            s.bold = match lower.as_str() {
                "bold" | "bolder" => true,
                "normal" | "lighter" => false,
                n => n.parse::<u32>().map_or(s.bold, |w| w >= 600),
            }
        }
        "font-style" => s.italic = lower == "italic" || lower == "oblique",
        "text-align" => {
            s.text_align = match lower.as_str() {
                "center" | "-webkit-center" => TextAlign::Center,
                "right" | "end" => TextAlign::Right,
                "left" | "start" | "justify" => TextAlign::Left,
                _ => s.text_align,
            }
        }
        "line-height" => {
            if lower == "normal" {
                s.line_height = LineHeight::Normal;
            } else if let Ok(f) = lower.parse::<f32>() {
                s.line_height = LineHeight::Factor(f);
            } else if let Some(l) = len(v) {
                if !l.auto {
                    s.line_height = if l.pct != 0.0 { LineHeight::Factor(l.pct / 100.0) } else { LineHeight::Px(l.px) };
                }
            }
        }
        "white-space" => {
            s.white_space = match lower.as_str() {
                "nowrap" => WhiteSpace::NoWrap,
                "pre" => WhiteSpace::Pre,
                "pre-wrap" | "break-spaces" => WhiteSpace::PreWrap,
                "pre-line" => WhiteSpace::PreLine,
                "normal" => WhiteSpace::Normal,
                _ => s.white_space,
            }
        }
        "list-style-type" => {
            s.list_style = match lower.as_str() {
                "none" => ListStyle::None,
                "disc" => ListStyle::Disc,
                "circle" => ListStyle::Circle,
                "square" => ListStyle::Square,
                "decimal" | "decimal-leading-zero" => ListStyle::Decimal,
                "lower-alpha" | "lower-latin" => ListStyle::LowerAlpha,
                "upper-alpha" | "upper-latin" => ListStyle::UpperAlpha,
                "lower-roman" => ListStyle::LowerRoman,
                "upper-roman" => ListStyle::UpperRoman,
                _ => s.list_style,
            }
        }
        "visibility" => s.hidden = lower == "hidden" || lower == "collapse",
        "text-transform" => {
            s.text_transform = match lower.as_str() {
                "uppercase" => TextTransform::Upper,
                "lowercase" => TextTransform::Lower,
                "capitalize" => TextTransform::Capitalize,
                _ => TextTransform::None,
            }
        }
        "text-decoration" => {
            if lower.contains("none") {
                s.underline = false;
                s.line_through = false;
            }
            if lower.contains("underline") {
                s.underline = true;
            }
            if lower.contains("line-through") {
                s.line_through = true;
            }
        }
        "display" => {
            s.display = match lower.as_str() {
                "none" => Display::None,
                "block" | "table" | "table-cell" | "table-caption" | "table-row-group" | "table-header-group" | "table-footer-group"
                | "grid" | "flow-root" | "run-in" => Display::Block,
                "table-row" | "flex" => Display::Flex,
                "inline" | "contents" => Display::Inline,
                "inline-block" | "inline-table" | "inline-grid" => Display::InlineBlock,
                "inline-flex" => Display::InlineFlex,
                "list-item" => Display::ListItem,
                _ => s.display,
            }
        }
        "position" => {
            s.position = match lower.as_str() {
                "relative" | "sticky" => Position::Relative,
                "absolute" => Position::Absolute,
                "fixed" => Position::Fixed,
                "static" => Position::Static,
                _ => s.position,
            }
        }
        "top" | "right" | "bottom" | "left" => {
            if let (Some(i), Some(l)) = (side, len(v)) {
                s.inset[i] = l;
            }
        }
        n if n.starts_with("margin-") => {
            if let (Some(i), Some(l)) = (side, len(v)) {
                s.margin[i] = l;
            }
        }
        n if n.starts_with("padding-") => {
            if let (Some(i), Some(l)) = (side, len(v)) {
                if !l.auto && l.px >= 0.0 && l.pct >= 0.0 {
                    s.padding[i] = l;
                }
            }
        }
        n if n.starts_with("border-") && n.ends_with("-width") => {
            let w = match lower.as_str() {
                "thin" => Some(1.0),
                "medium" => Some(3.0),
                "thick" => Some(5.0),
                _ => len(v).filter(|l| !l.auto).map(|l| l.px.max(0.0)),
            };
            if let (Some(i), Some(w)) = (side, w) {
                s.border_width[i] = if w > 0.0 { w.max(1.0).round() } else { 0.0 };
            }
        }
        n if n.starts_with("border-") && n.ends_with("-style") => {
            if let Some(i) = side {
                s.border_style[i] = !matches!(lower.as_str(), "none" | "hidden");
            }
        }
        n if n.starts_with("border-") && n.ends_with("-color") => {
            if let Some(i) = side {
                if lower == "currentcolor" {
                    s.border_color[i] = None;
                } else if let Some(c) = parse_color(v, s.color) {
                    s.border_color[i] = Some(c);
                }
            }
        }
        "border-radius" => {
            if let Some(l) = len(v) {
                // A percentage of a 50% circle on a small box: take the box size later? Approximate with 999.
                s.radius = if l.pct >= 50.0 { 999.0 } else { l.px.max(0.0) };
            }
        }
        "width" | "height" | "min-width" | "min-height" | "max-width" | "max-height" | "flex-basis" => {
            let l = match lower.as_str() {
                "none" | "auto" | "fit-content" | "max-content" | "min-content" | "content" => Some(Len::AUTO),
                _ => len(v),
            };
            if let Some(l) = l {
                if !l.auto && (l.px < 0.0 || l.pct < 0.0) {
                    return;
                }
                match d.name.as_str() {
                    "width" => s.width = l,
                    "height" => s.height = l,
                    "min-width" => s.min_width = l,
                    "min-height" => s.min_height = l,
                    "max-width" => s.max_width = l,
                    "max-height" => s.max_height = l,
                    _ => s.flex_basis = l,
                }
            }
        }
        "box-sizing" => s.border_box = lower == "border-box",
        "background-color" => {
            if let Some(c) = parse_color(v, s.color) {
                s.background = c;
            }
        }
        "background-image" => {
            s.background_image = lower.strip_prefix("url(").and_then(|_| {
                let inner = v[4..].trim_end_matches(')').trim().trim_matches(|c| c == '"' || c == '\'');
                (!inner.is_empty()).then(|| inner.to_string())
            });
        }
        "flex-direction" => {
            s.flex_direction = match lower.as_str() {
                "row-reverse" => FlexDirection::RowReverse,
                "column" => FlexDirection::Column,
                "column-reverse" => FlexDirection::ColumnReverse,
                _ => FlexDirection::Row,
            }
        }
        "flex-wrap" => s.flex_wrap = lower == "wrap" || lower == "wrap-reverse",
        "justify-content" => {
            s.justify = match lower.trim_start_matches("safe ").trim_start_matches("unsafe ") {
                "flex-end" | "end" | "right" => Justify::End,
                "center" => Justify::Center,
                "space-between" => Justify::SpaceBetween,
                "space-around" => Justify::SpaceAround,
                "space-evenly" => Justify::SpaceEvenly,
                _ => Justify::Start,
            }
        }
        "align-items" | "align-self" => {
            let a = match lower.trim_start_matches("safe ").trim_start_matches("unsafe ") {
                "flex-start" | "start" | "self-start" => Some(Align::Start),
                "flex-end" | "end" | "self-end" => Some(Align::End),
                "center" => Some(Align::Center),
                "baseline" | "first baseline" | "last baseline" => Some(Align::Baseline),
                "stretch" | "normal" => Some(Align::Stretch),
                _ => None,
            };
            if d.name == "align-items" {
                if let Some(a) = a {
                    s.align_items = a;
                }
            } else {
                s.align_self = a;
            }
        }
        "flex-grow" => {
            if let Ok(f) = lower.parse::<f32>() {
                s.flex_grow = f.max(0.0);
            }
        }
        "flex-shrink" => {
            if let Ok(f) = lower.parse::<f32>() {
                s.flex_shrink = f.max(0.0);
            }
        }
        "order" => {
            if let Ok(o) = lower.parse::<i32>() {
                s.order = o;
            }
        }
        "row-gap" | "column-gap" => {
            if let Some(l) = len(v) {
                let g = if l.auto { 0.0 } else { l.px.max(0.0) };
                if d.name == "row-gap" {
                    s.row_gap = g;
                } else {
                    s.column_gap = g;
                }
            }
        }
        "overflow" => {
            s.overflow = match lower.as_str() {
                "hidden" | "clip" => Overflow::Hidden,
                "auto" | "scroll" | "overlay" => Overflow::Auto,
                _ => Overflow::Visible,
            }
        }
        "opacity" => {
            let o = if let Some(p) = lower.strip_suffix('%') { p.parse::<f32>().ok().map(|p| p / 100.0) } else { lower.parse::<f32>().ok() };
            if let Some(o) = o {
                s.opacity = o.clamp(0.0, 1.0);
            }
        }
        "text-indent" => {
            if let Some(l) = len(v) {
                s.text_indent = l.px;
            }
        }
        _ => {}
    }
}

/// A length or percentage. `em` is the element's font size, `rem` the root's.
pub fn parse_len(v: &str, em: f32, rem: f32, viewport: (f32, f32)) -> Option<Len> {
    let v = v.trim().to_ascii_lowercase();
    if v == "auto" {
        return Some(Len::AUTO);
    }
    if let Some(inner) = v.strip_prefix("calc(").and_then(|r| r.strip_suffix(')')) {
        return parse_calc(inner, em, rem, viewport);
    }
    if v == "0" {
        return Some(Len::ZERO);
    }
    if let Some(p) = v.strip_suffix('%') {
        return p.trim().parse::<f32>().ok().map(Len::pct);
    }
    let unit_at = v.find(|c: char| c.is_ascii_alphabetic())?;
    let n: f32 = v[..unit_at].trim().parse().ok()?;
    let px = match &v[unit_at..] {
        "px" => n,
        "em" | "ch" => n * em,
        "ex" => n * em / 2.0,
        "rem" => n * rem,
        "vw" => n * viewport.0 / 100.0,
        "vh" => n * viewport.1 / 100.0,
        "vmin" => n * viewport.0.min(viewport.1) / 100.0,
        "vmax" => n * viewport.0.max(viewport.1) / 100.0,
        "pt" => n * 4.0 / 3.0,
        "pc" => n * 16.0,
        "in" => n * 96.0,
        "cm" => n * 96.0 / 2.54,
        "mm" => n * 96.0 / 25.4,
        _ => return None,
    };
    Some(Len::px(px))
}

/// `calc()` with `+` and `-` of lengths and percentages (and `* n`, `/ n` on one term).
fn parse_calc(s: &str, em: f32, rem: f32, viewport: (f32, f32)) -> Option<Len> {
    let mut total = Len::ZERO;
    let mut sign = 1.0;
    for tok in css::components(s) {
        match tok {
            "+" => sign = 1.0,
            "-" => sign = -1.0,
            "*" | "/" => return None,
            t => {
                let t = t.trim_start_matches('(').trim_end_matches(')');
                let l = parse_len(t, em, rem, viewport).or_else(|| t.parse::<f32>().ok().filter(|n| *n == 0.0).map(|_| Len::ZERO))?;
                if l.auto {
                    return None;
                }
                total.px += sign * l.px;
                total.pct += sign * l.pct;
            }
        }
    }
    Some(total)
}

/// `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`/`rgba()`, names,
/// `transparent` and `currentcolor` (= `current`). RGBA8888.
pub fn parse_color(v: &str, current: u32) -> Option<u32> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(h) = v.strip_prefix('#') {
        let n = |s: &str| u32::from_str_radix(s, 16).ok();
        let d = |c: &str| n(c).map(|x| x * 17);
        return match h.len() {
            3 => Some(d(&h[0..1])? << 24 | d(&h[1..2])? << 16 | d(&h[2..3])? << 8 | 0xff),
            4 => Some(d(&h[0..1])? << 24 | d(&h[1..2])? << 16 | d(&h[2..3])? << 8 | d(&h[3..4])?),
            6 => Some(n(h)? << 8 | 0xff),
            8 => n(h),
            _ => None,
        };
    }
    if let Some(args) = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb(")).and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<&str> = args.split([',', ' ', '/']).map(str::trim).filter(|p| !p.is_empty()).collect();
        if parts.len() < 3 {
            return None;
        }
        let ch = |p: &str| -> Option<u32> {
            Some(if let Some(pc) = p.strip_suffix('%') { (pc.parse::<f32>().ok()? * 2.55).round() as u32 } else { p.parse::<f32>().ok()?.round() as u32 }.min(255))
        };
        let a = match parts.get(3) {
            Some(p) => {
                if let Some(pc) = p.strip_suffix('%') {
                    (pc.parse::<f32>().ok()? * 2.55).round() as u32
                } else {
                    (p.parse::<f32>().ok()? * 255.0).round() as u32
                }
            }
            None => 255,
        }
        .min(255);
        return Some(ch(parts[0])? << 24 | ch(parts[1])? << 16 | ch(parts[2])? << 8 | a);
    }
    if let Some(args) = v.strip_prefix("hsla(").or_else(|| v.strip_prefix("hsl(")).and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<&str> = args.split([',', ' ', '/']).map(str::trim).filter(|p| !p.is_empty()).collect();
        if parts.len() < 3 {
            return None;
        }
        let h = parts[0].trim_end_matches("deg").parse::<f32>().ok()? / 360.0;
        let s = parts[1].trim_end_matches('%').parse::<f32>().ok()? / 100.0;
        let l = parts[2].trim_end_matches('%').parse::<f32>().ok()? / 100.0;
        let a = parts.get(3).and_then(|p| p.parse::<f32>().ok()).unwrap_or(1.0);
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        let f = |t: f32| {
            let t = t.rem_euclid(1.0);
            let c = if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            };
            (c * 255.0).round().clamp(0.0, 255.0) as u32
        };
        return Some(f(h + 1.0 / 3.0) << 24 | f(h) << 16 | f(h - 1.0 / 3.0) << 8 | (a * 255.0).round().clamp(0.0, 255.0) as u32);
    }
    let rgb = match v.as_str() {
        "transparent" => return Some(0),
        "currentcolor" => return Some(current),
        "black" => 0x000000,
        "white" => 0xffffff,
        "red" => 0xff0000,
        "green" => 0x008000,
        "blue" => 0x0000ff,
        "yellow" => 0xffff00,
        "cyan" | "aqua" => 0x00ffff,
        "magenta" | "fuchsia" => 0xff00ff,
        "gray" | "grey" => 0x808080,
        "silver" => 0xc0c0c0,
        "maroon" => 0x800000,
        "olive" => 0x808000,
        "lime" => 0x00ff00,
        "navy" => 0x000080,
        "teal" => 0x008080,
        "purple" => 0x800080,
        "orange" => 0xffa500,
        "pink" => 0xffc0cb,
        "brown" => 0xa52a2a,
        "gold" => 0xffd700,
        "indigo" => 0x4b0082,
        "violet" => 0xee82ee,
        "coral" => 0xff7f50,
        "salmon" => 0xfa8072,
        "tomato" => 0xff6347,
        "crimson" => 0xdc143c,
        "orchid" => 0xda70d6,
        "khaki" => 0xf0e68c,
        "beige" => 0xf5f5dc,
        "ivory" => 0xfffff0,
        "lavender" => 0xe6e6fa,
        "tan" => 0xd2b48c,
        "chocolate" => 0xd2691e,
        "turquoise" => 0x40e0d0,
        "skyblue" => 0x87ceeb,
        "steelblue" => 0x4682b4,
        "royalblue" => 0x4169e1,
        "dodgerblue" => 0x1e90ff,
        "slategray" | "slategrey" => 0x708090,
        "darkslategray" | "darkslategrey" => 0x2f4f4f,
        "lightgray" | "lightgrey" => 0xd3d3d3,
        "darkgray" | "darkgrey" => 0xa9a9a9,
        "dimgray" | "dimgrey" => 0x696969,
        "gainsboro" => 0xdcdcdc,
        "whitesmoke" => 0xf5f5f5,
        "darkred" => 0x8b0000,
        "darkgreen" => 0x006400,
        "darkblue" => 0x00008b,
        "darkorange" => 0xff8c00,
        "lightblue" => 0xadd8e6,
        "lightgreen" => 0x90ee90,
        "lightyellow" => 0xffffe0,
        "forestgreen" => 0x228b22,
        "seagreen" => 0x2e8b57,
        "limegreen" => 0x32cd32,
        "midnightblue" => 0x191970,
        "firebrick" => 0xb22222,
        "goldenrod" => 0xdaa520,
        "plum" => 0xdda0dd,
        "aliceblue" => 0xf0f8ff,
        "honeydew" => 0xf0fff0,
        "mintcream" => 0xf5fffa,
        "rebeccapurple" => 0x663399,
        _ => return None,
    };
    Some(rgb << 8 | 0xff)
}

/// The colour text gets on the page's background (`fg` blended over `bg` by its alpha).
pub fn blend(fg: u32, bg: u32) -> u32 {
    let a = (fg & 0xff) as f32 / 255.0;
    let ch = |sh: u32| -> u32 {
        let f = ((fg >> sh) & 0xff) as f32;
        let b = ((bg >> sh) & 0xff) as f32;
        (f * a + b * (1.0 - a)).round() as u32
    };
    ch(24) << 24 | ch(16) << 16 | ch(8) << 8 | 0xff
}

/// Text of a node with `text-transform` applied.
pub fn transform_text(t: &str, tt: TextTransform) -> String {
    match tt {
        TextTransform::None => t.to_string(),
        TextTransform::Upper => t.to_uppercase(),
        TextTransform::Lower => t.to_lowercase(),
        TextTransform::Capitalize => {
            let mut out = String::with_capacity(t.len());
            let mut start = true;
            for c in t.chars() {
                if start && c.is_alphabetic() {
                    out.extend(c.to_uppercase());
                } else {
                    out.push(c);
                }
                start = c.is_whitespace();
            }
            out
        }
    }
}

/// The style of a text node: its parent element's.
pub fn of_text(dom: &Dom, styles: &[Option<Style>], id: NodeId) -> Option<Style> {
    match dom.nodes[id].data {
        NodeData::Text(_) => dom.parent(id).and_then(|p| styles.get(p).cloned().flatten()),
        _ => styles.get(id).cloned().flatten(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    fn styles(src: &str, sheets: &[&str], w: f32) -> (Dom, Vec<Option<Style>>) {
        let dom = html::parse(src);
        let ua = UaColors::default();
        let ua_sheet = ua_sheet(&ua);
        let sheets: Vec<Stylesheet> = sheets.iter().map(|s| Stylesheet::parse(s)).collect();
        let dy = Dynamic::default();
        let env = StyleEnv { ua_colors: &ua, ua_sheet: &ua_sheet, sheets: &sheets, viewport: (w, 300.0), dynamic: &dy };
        let st = compute(&dom, &env);
        (dom, st)
    }

    #[test]
    fn colours() {
        assert_eq!(parse_color("#fff", 0), Some(0xffffffff));
        assert_eq!(parse_color("#11223344", 0), Some(0x11223344));
        assert_eq!(parse_color("#1234", 0), Some(0x11223344));
        assert_eq!(parse_color("rgb(255, 0, 10)", 0), Some(0xff000aff));
        assert_eq!(parse_color("rgba(0,0,0,0.5)", 0), Some(0x00000080));
        assert_eq!(parse_color("rgb(0 128 255 / 50%)", 0), Some(0x0080ff80));
        assert_eq!(parse_color("Navy", 0), Some(0x000080ff));
        assert_eq!(parse_color("currentColor", 7), Some(7));
        assert_eq!(parse_color("hsl(0, 100%, 50%)", 0), Some(0xff0000ff));
        assert_eq!(parse_color("bogus", 0), None);
    }

    #[test]
    fn lengths() {
        let l = |s: &str| parse_len(s, 16.0, 12.0, (400.0, 300.0)).unwrap();
        assert_eq!(l("10px"), Len::px(10.0));
        assert_eq!(l("2em"), Len::px(32.0));
        assert_eq!(l("2rem"), Len::px(24.0));
        assert_eq!(l("50%"), Len::pct(50.0));
        assert_eq!(l("10vw"), Len::px(40.0));
        assert_eq!(l("calc(100% - 20px)"), Len { px: -20.0, pct: 100.0, auto: false });
        assert!(parse_len("1furlong", 1.0, 1.0, (1.0, 1.0)).is_none());
        assert_eq!(snap_font_size(14.0), 12);
        assert_eq!(snap_font_size(15.0), 16);
        assert_eq!(snap_font_size(100.0), 32);
    }

    #[test]
    fn cascade_order() {
        let (d, st) = styles(
            "<div id=a class=c style='color: blue'><p class=c>x</p><p id=b>y</p></div>",
            &["p { color: red } .c { color: green } #b { color: yellow } p { color: purple !important } #a { color: navy !important }"],
            400.0,
        );
        let div = d.find("div").unwrap();
        let ps = d.find_all("p");
        // !important in a sheet beats the style attribute.
        assert_eq!(st[div].as_ref().unwrap().color, parse_color("navy", 0).unwrap());
        // important beats specificity
        assert_eq!(st[ps[0]].as_ref().unwrap().color, parse_color("purple", 0).unwrap());
        assert_eq!(st[ps[1]].as_ref().unwrap().color, parse_color("purple", 0).unwrap());
    }

    #[test]
    fn specificity_and_source_order() {
        let (d, st) = styles("<p class=c id=i>x</p><span class=c>s</span>", &["p { color: red } p.c { color: green }", ".c { color: blue } span { color: black }"], 400.0);
        let p = d.find("p").unwrap();
        let span = d.find("span").unwrap();
        assert_eq!(st[p].as_ref().unwrap().color, 0x008000ff);
        assert_eq!(st[span].as_ref().unwrap().color, 0x0000ffff);
    }

    #[test]
    fn inheritance_and_keywords() {
        let (d, st) = styles(
            "<div style='color:#123456; font-size: 24px; border: 1px solid'><span>t</span><b style='color: initial'>b</b><i style='border-top-width: inherit'>i</i></div>",
            &[],
            400.0,
        );
        let ua = UaColors::default();
        let span = d.find("span").unwrap();
        let s = st[span].as_ref().unwrap();
        assert_eq!(s.color, 0x123456ff);
        assert_eq!(s.font_size, 24.0);
        assert_eq!(s.borders(), [0.0; 4], "borders are not inherited");
        assert!(st[d.find("b").unwrap()].as_ref().unwrap().bold);
        assert_eq!(st[d.find("b").unwrap()].as_ref().unwrap().color, ua.text);
        assert_eq!(st[d.find("i").unwrap()].as_ref().unwrap().border_width[0], 1.0);
        let div = st[d.find("div").unwrap()].as_ref().unwrap();
        assert_eq!(div.borders(), [1.0; 4]);
        assert_eq!(div.border_colour(0), 0x123456ff, "currentcolor");
    }

    #[test]
    fn ua_defaults_and_media() {
        let (d, st) = styles(
            "<h1>T</h1><a href=x>l</a><pre>p</pre><nav>n</nav>",
            &["@media (max-width: 500px) { nav { display: none } }"],
            400.0,
        );
        let ua = UaColors::default();
        let h1 = st[d.find("h1").unwrap()].as_ref().unwrap();
        assert_eq!((h1.font_size, h1.bold, h1.display), (32.0, true, Display::Block));
        let a = st[d.find("a").unwrap()].as_ref().unwrap();
        assert_eq!((a.color, a.underline), (ua.link, true));
        assert!(st[d.find("pre").unwrap()].as_ref().unwrap().mono);
        assert_eq!(st[d.find("nav").unwrap()].as_ref().unwrap().display, Display::None);
        let (d, st) = styles("<nav>n</nav>", &["@media (max-width: 500px) { nav { display: none } }"], 800.0);
        assert_eq!(st[d.find("nav").unwrap()].as_ref().unwrap().display, Display::Block);
    }

    #[test]
    fn em_uses_own_font_size() {
        let (d, st) = styles("<div style='font-size: 16px; margin-left: 2em; width: 50%'><p style='font-size: 2em'>x</p></div>", &[], 400.0);
        let div = st[d.find("div").unwrap()].as_ref().unwrap();
        assert_eq!(div.margin[3], Len::px(32.0));
        assert_eq!(div.width, Len::pct(50.0));
        assert_eq!(st[d.find("p").unwrap()].as_ref().unwrap().font_size, 32.0);
    }
}
