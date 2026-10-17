//! Style sheets: rules, selectors (with specificity and matching against the
//! DOM), declarations with shorthands expanded, and `@media` width queries.
//!
//! Selectors: `tag`, `.class`, `#id`, `*`, `[x]`, `[x=y]` (and `~= |= ^= $= *=`),
//! the combinators descendant, `>`, `+` and `~`, and the pseudo-classes
//! `:hover`, `:focus`, `:first-child`, `:last-child`, `:only-child`,
//! `:nth-child()`, `:nth-last-child()`, `:root`, `:link`, `:checked`,
//! `:disabled`, `:enabled`, `:empty` and `:not()`. A selector with anything
//! else (pseudo-elements like `::before` included) never matches.

use crate::dom::{Dom, NodeData, NodeId};

#[derive(Clone, Debug, Default)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub decls: Vec<Decl>,
    /// Every one of these must match (nested `@media` blocks).
    pub media: Vec<Media>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Decl {
    /// Lower-case longhand property name.
    pub name: String,
    pub value: String,
    pub important: bool,
}

/// A media query list: matches when any of its queries does.
#[derive(Clone, Debug, PartialEq)]
pub struct Media {
    queries: Vec<MediaQuery>,
}

#[derive(Clone, Debug, PartialEq)]
struct MediaQuery {
    not: bool,
    /// The media type is `screen`/`all` (or none was given).
    type_ok: bool,
    conds: Vec<(Feature, f32)>,
    /// An unknown feature: the query never matches.
    unknown: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Feature {
    MinWidth,
    MaxWidth,
    MinHeight,
    MaxHeight,
}

impl Media {
    pub fn parse(s: &str) -> Media {
        let queries = split_top(s, ',')
            .into_iter()
            .map(|q| {
                let q = q.trim().to_ascii_lowercase();
                let mut mq = MediaQuery { not: false, type_ok: true, conds: Vec::new(), unknown: false };
                let mut rest = q.as_str();
                if let Some(r) = rest.strip_prefix("not ") {
                    mq.not = true;
                    rest = r.trim_start();
                } else if let Some(r) = rest.strip_prefix("only ") {
                    rest = r.trim_start();
                }
                for part in rest.split(" and ") {
                    let part = part.trim();
                    if part.is_empty() {
                        continue;
                    }
                    if let Some(inner) = part.strip_prefix('(').and_then(|p| p.strip_suffix(')')) {
                        let Some((name, value)) = inner.split_once(':') else {
                            // (color), (hover) and the like: a screen has them.
                            if !matches!(inner.trim(), "color" | "hover" | "pointer") {
                                mq.unknown = true;
                            }
                            continue;
                        };
                        let f = match name.trim() {
                            "min-width" => Feature::MinWidth,
                            "max-width" => Feature::MaxWidth,
                            "min-height" => Feature::MinHeight,
                            "max-height" => Feature::MaxHeight,
                            "orientation" | "prefers-color-scheme" | "hover" | "pointer" => continue,
                            _ => {
                                mq.unknown = true;
                                continue;
                            }
                        };
                        match parse_length_px(value.trim(), 12.0) {
                            Some(v) => mq.conds.push((f, v)),
                            None => mq.unknown = true,
                        }
                    } else {
                        mq.type_ok = matches!(part, "screen" | "all");
                    }
                }
                mq
            })
            .collect();
        Media { queries }
    }

    /// Does the media match a viewport of `w x h` lu?
    pub fn matches(&self, w: f32, h: f32) -> bool {
        self.queries.iter().any(|q| {
            let ok = !q.unknown
                && q.type_ok
                && q.conds.iter().all(|(f, v)| match f {
                    Feature::MinWidth => w >= *v,
                    Feature::MaxWidth => w <= *v,
                    Feature::MinHeight => h >= *v,
                    Feature::MaxHeight => h <= *v,
                });
            ok != q.not
        })
    }
}

// ------------------------------------------------------------------ selectors

#[derive(Clone, Debug, PartialEq)]
pub struct Selector {
    /// Left to right.
    pub compounds: Vec<Compound>,
    /// `combinators[i]` joins `compounds[i]` and `compounds[i + 1]`.
    pub combinators: Vec<Combinator>,
    /// ids * 10000 + classes/attributes/pseudo-classes * 100 + tags.
    pub specificity: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Combinator {
    Descendant,
    Child,
    Next,
    Subsequent,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compound {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrSel>,
    pub pseudos: Vec<Pseudo>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttrSel {
    pub name: String,
    pub op: AttrOp,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AttrOp {
    Exists,
    Eq,
    Includes,
    Dash,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pseudo {
    Hover,
    Focus,
    Active,
    FirstChild,
    LastChild,
    OnlyChild,
    NthChild(i32, i32),
    NthLastChild(i32, i32),
    Root,
    Link,
    Visited,
    Checked,
    Disabled,
    Enabled,
    Empty,
    Not(Vec<Compound>),
    /// Something unsupported: never matches.
    Never,
}

/// What `:hover` and `:focus` mean right now.
#[derive(Clone, Debug, Default)]
pub struct Dynamic {
    /// The hovered element and its ancestors.
    pub hover: Vec<NodeId>,
    pub focus: Option<NodeId>,
    pub active: Option<NodeId>,
}

impl Selector {
    /// Does the selector match element `id`?
    pub fn matches(&self, dom: &Dom, id: NodeId, dy: &Dynamic) -> bool {
        let n = self.compounds.len();
        if n == 0 {
            return false;
        }
        self.match_from(dom, id, n - 1, dy)
    }

    fn match_from(&self, dom: &Dom, id: NodeId, i: usize, dy: &Dynamic) -> bool {
        if !self.compounds[i].matches(dom, id, dy) {
            return false;
        }
        if i == 0 {
            return true;
        }
        match self.combinators[i - 1] {
            Combinator::Child => dom.parent_element(id).is_some_and(|p| self.match_from(dom, p, i - 1, dy)),
            Combinator::Descendant => {
                let mut cur = dom.parent_element(id);
                while let Some(p) = cur {
                    if self.match_from(dom, p, i - 1, dy) {
                        return true;
                    }
                    cur = dom.parent_element(p);
                }
                false
            }
            Combinator::Next => dom.prev_element(id).is_some_and(|p| self.match_from(dom, p, i - 1, dy)),
            Combinator::Subsequent => {
                let mut cur = dom.prev_element(id);
                while let Some(p) = cur {
                    if self.match_from(dom, p, i - 1, dy) {
                        return true;
                    }
                    cur = dom.prev_element(p);
                }
                false
            }
        }
    }

    /// Does the selector use `:hover` (hovering then needs a restyle)?
    pub fn uses_hover(&self) -> bool {
        self.compounds.iter().any(|c| c.pseudos.iter().any(|p| matches!(p, Pseudo::Hover | Pseudo::Active | Pseudo::Focus)))
    }
}

fn nth(a: i32, b: i32, pos: i32) -> bool {
    // pos (1-based) = a*n + b for some n >= 0.
    if a == 0 {
        return pos == b;
    }
    let d = pos - b;
    d % a == 0 && d / a >= 0
}

impl Compound {
    pub fn matches(&self, dom: &Dom, id: NodeId, dy: &Dynamic) -> bool {
        let Some(e) = dom.element(id) else { return false };
        if let Some(t) = &self.tag {
            if *t != e.tag {
                return false;
            }
        }
        if let Some(i) = &self.id {
            if e.id() != Some(i.as_str()) {
                return false;
            }
        }
        if !self.classes.iter().all(|c| e.has_class(c)) {
            return false;
        }
        for a in &self.attrs {
            let Some(v) = e.attr(&a.name) else { return false };
            let ok = match a.op {
                AttrOp::Exists => true,
                AttrOp::Eq => v == a.value,
                AttrOp::Includes => v.split_ascii_whitespace().any(|w| w == a.value),
                AttrOp::Dash => v == a.value || v.starts_with(&format!("{}-", a.value)),
                AttrOp::Prefix => !a.value.is_empty() && v.starts_with(&a.value),
                AttrOp::Suffix => !a.value.is_empty() && v.ends_with(&a.value),
                AttrOp::Substring => !a.value.is_empty() && v.contains(&a.value),
            };
            if !ok {
                return false;
            }
        }
        for p in &self.pseudos {
            let ok = match p {
                Pseudo::Hover => dy.hover.contains(&id),
                Pseudo::Focus => dy.focus == Some(id),
                Pseudo::Active => dy.active == Some(id),
                Pseudo::FirstChild => dom.element_index(id).0 == 0,
                Pseudo::LastChild => {
                    let (i, n) = dom.element_index(id);
                    i + 1 == n
                }
                Pseudo::OnlyChild => dom.element_index(id).1 == 1,
                Pseudo::NthChild(a, b) => nth(*a, *b, dom.element_index(id).0 as i32 + 1),
                Pseudo::NthLastChild(a, b) => {
                    let (i, n) = dom.element_index(id);
                    nth(*a, *b, (n - i) as i32)
                }
                Pseudo::Root => dom.parent_element(id).is_none(),
                Pseudo::Link => e.tag == "a" && e.attr("href").is_some(),
                Pseudo::Visited => false,
                Pseudo::Checked => e.attr("checked").is_some() || e.attr("selected").is_some(),
                Pseudo::Disabled => e.attr("disabled").is_some(),
                Pseudo::Enabled => matches!(e.tag.as_str(), "input" | "button" | "select" | "textarea") && e.attr("disabled").is_none(),
                Pseudo::Empty => dom.nodes[id].children.iter().all(|c| match &dom.nodes[*c].data {
                    NodeData::Text(t) => t.is_empty(),
                    _ => false,
                }),
                Pseudo::Not(list) => !list.iter().any(|c| c.matches(dom, id, dy)),
                Pseudo::Never => false,
            };
            if !ok {
                return false;
            }
        }
        true
    }

    fn specificity(&self) -> u32 {
        let mut s = 0;
        if self.id.is_some() {
            s += 10000;
        }
        s += 100 * (self.classes.len() + self.attrs.len()) as u32;
        for p in &self.pseudos {
            s += match p {
                Pseudo::Not(list) => list.iter().map(|c| c.specificity()).max().unwrap_or(0),
                _ => 100,
            };
        }
        if self.tag.is_some() {
            s += 1;
        }
        s
    }
}

/// Parse a selector list (`h1, .a > b`). Selectors that don't parse are dropped.
pub fn parse_selectors(s: &str) -> Vec<Selector> {
    split_top(s, ',').into_iter().filter_map(|p| parse_selector(p.trim())).collect()
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_' || !c.is_ascii()
}

fn parse_selector(s: &str) -> Option<Selector> {
    if s.is_empty() {
        return None;
    }
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut compounds = Vec::new();
    let mut combinators = Vec::new();
    let mut pending: Option<Combinator> = None;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            if !compounds.is_empty() && pending.is_none() {
                pending = Some(Combinator::Descendant);
            }
            i += 1;
            continue;
        }
        if c == '>' || c == '+' || c == '~' {
            if compounds.is_empty() {
                return None;
            }
            pending = Some(match c {
                '>' => Combinator::Child,
                '+' => Combinator::Next,
                _ => Combinator::Subsequent,
            });
            i += 1;
            continue;
        }
        let (comp, next) = parse_compound(&cs, i)?;
        if next == i {
            return None;
        }
        i = next;
        if !compounds.is_empty() {
            combinators.push(pending.take()?);
        }
        pending = None;
        compounds.push(comp);
    }
    if compounds.is_empty() || (pending.is_some() && pending != Some(Combinator::Descendant)) {
        return None;
    }
    let specificity = compounds.iter().map(|c| c.specificity()).sum();
    Some(Selector { compounds, combinators, specificity })
}

fn read_ident(cs: &[char], mut i: usize) -> (String, usize) {
    let start = i;
    while i < cs.len() && (is_ident(cs[i]) || (cs[i] == '\\' && i + 1 < cs.len())) {
        if cs[i] == '\\' {
            i += 1;
        }
        i += 1;
    }
    (cs[start..i].iter().filter(|c| **c != '\\').collect(), i)
}

/// A compound selector starting at `i`; None if it is malformed.
fn parse_compound(cs: &[char], mut i: usize) -> Option<(Compound, usize)> {
    let mut c = Compound::default();
    if cs.get(i) == Some(&'*') {
        i += 1;
    } else if cs.get(i).is_some_and(|ch| is_ident(*ch)) {
        let (t, n) = read_ident(cs, i);
        c.tag = Some(t.to_ascii_lowercase());
        i = n;
    }
    while i < cs.len() {
        match cs[i] {
            '#' => {
                let (t, n) = read_ident(cs, i + 1);
                if t.is_empty() {
                    return None;
                }
                c.id = Some(t);
                i = n;
            }
            '.' => {
                let (t, n) = read_ident(cs, i + 1);
                if t.is_empty() {
                    return None;
                }
                c.classes.push(t);
                i = n;
            }
            '[' => {
                let end = i + cs[i..].iter().position(|ch| *ch == ']')?;
                let inner: String = cs[i + 1..end].iter().collect();
                c.attrs.push(parse_attr_sel(&inner)?);
                i = end + 1;
            }
            ':' => {
                let element = cs.get(i + 1) == Some(&':');
                let (name, mut n) = read_ident(cs, if element { i + 2 } else { i + 1 });
                let name = name.to_ascii_lowercase();
                let mut arg = None;
                if cs.get(n) == Some(&'(') {
                    let mut depth = 0;
                    let mut j = n;
                    while j < cs.len() {
                        match cs[j] {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    arg = Some(cs[n + 1..j.min(cs.len())].iter().collect::<String>());
                    n = (j + 1).min(cs.len());
                }
                let p = if element {
                    Pseudo::Never
                } else {
                    match (name.as_str(), arg) {
                        ("hover", None) => Pseudo::Hover,
                        ("focus", None) | ("focus-visible", None) | ("focus-within", None) => Pseudo::Focus,
                        ("active", None) => Pseudo::Active,
                        ("first-child", None) => Pseudo::FirstChild,
                        ("last-child", None) => Pseudo::LastChild,
                        ("only-child", None) => Pseudo::OnlyChild,
                        ("root", None) => Pseudo::Root,
                        ("link", None) | ("any-link", None) => Pseudo::Link,
                        ("visited", None) => Pseudo::Visited,
                        ("checked", None) => Pseudo::Checked,
                        ("disabled", None) => Pseudo::Disabled,
                        ("enabled", None) => Pseudo::Enabled,
                        ("empty", None) => Pseudo::Empty,
                        ("nth-child", Some(a)) => {
                            let (x, y) = parse_nth(&a)?;
                            Pseudo::NthChild(x, y)
                        }
                        ("nth-last-child", Some(a)) => {
                            let (x, y) = parse_nth(&a)?;
                            Pseudo::NthLastChild(x, y)
                        }
                        ("not", Some(a)) => {
                            let mut list = Vec::new();
                            for part in split_top(&a, ',') {
                                let pc: Vec<char> = part.trim().chars().collect();
                                let (comp, end) = parse_compound(&pc, 0)?;
                                if end != pc.len() {
                                    return None;
                                }
                                list.push(comp);
                            }
                            Pseudo::Not(list)
                        }
                        _ => Pseudo::Never,
                    }
                };
                c.pseudos.push(p);
                i = n;
            }
            _ => break,
        }
    }
    Some((c, i))
}

fn parse_attr_sel(s: &str) -> Option<AttrSel> {
    let s = s.trim();
    let ops = [("~=", AttrOp::Includes), ("|=", AttrOp::Dash), ("^=", AttrOp::Prefix), ("$=", AttrOp::Suffix), ("*=", AttrOp::Substring), ("=", AttrOp::Eq)];
    for (tok, op) in ops {
        if let Some((n, v)) = s.split_once(tok) {
            let v = v.trim();
            // [x="y" i]: the case flag is ignored.
            let v = v.strip_suffix(" i").unwrap_or(v).trim();
            let v = v.trim_matches(|c| c == '"' || c == '\'');
            return Some(AttrSel { name: n.trim().to_ascii_lowercase(), op, value: v.to_string() });
        }
    }
    if s.is_empty() || !s.chars().all(is_ident) {
        return None;
    }
    Some(AttrSel { name: s.to_ascii_lowercase(), op: AttrOp::Exists, value: String::new() })
}

/// `an+b`, `odd`, `even`, `3`.
fn parse_nth(s: &str) -> Option<(i32, i32)> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    match s.as_str() {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        _ => {}
    }
    if let Some((a, b)) = s.split_once('n') {
        let a = match a {
            "" | "+" => 1,
            "-" => -1,
            x => x.parse().ok()?,
        };
        let b = if b.is_empty() { 0 } else { b.parse().ok()? };
        Some((a, b))
    } else {
        Some((0, s.parse().ok()?))
    }
}

// ------------------------------------------------------------------ sheets

/// Remove `/* comments */`.
fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find("/*") {
        out.push_str(&rest[..p]);
        match rest[p + 2..].find("*/") {
            Some(e) => rest = &rest[p + 2 + e + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Split at `sep` outside parentheses, brackets and quotes.
pub fn split_top(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ if c == sep && depth <= 0 => {
                out.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// The index just after the `}` that closes the block opened at `open` (`{`).
fn block_end(s: &str, open: usize) -> usize {
    let mut depth = 0;
    let mut quote: Option<char> = None;
    for (i, c) in s[open..].char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return open + i + 1;
                }
            }
            _ => {}
        }
    }
    s.len()
}

impl Stylesheet {
    pub fn parse(src: &str) -> Stylesheet {
        let s = strip_comments(src);
        let mut rules = Vec::new();
        parse_rules(&s, &[], &mut rules);
        Stylesheet { rules }
    }
}

fn parse_rules(s: &str, media: &[Media], out: &mut Vec<Rule>) {
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let skip = rest.len() - rest.trim_start().len();
        i += skip;
        if i >= s.len() {
            break;
        }
        let rest = &s[i..];
        if rest.starts_with('}') || rest.starts_with(';') {
            i += 1;
            continue;
        }
        if rest.starts_with('@') {
            let brace = rest.find('{');
            let semi = rest.find(';');
            match (brace, semi) {
                (Some(b), sc) if sc.is_none_or(|sc| b < sc) => {
                    let end = block_end(s, i + b);
                    let name = rest[1..b].split_whitespace().next().unwrap_or("").to_ascii_lowercase();
                    let inner = &s[i + b + 1..end.saturating_sub(1).max(i + b + 1)];
                    if name == "media" {
                        let mut m = media.to_vec();
                        m.push(Media::parse(&rest[6..b]));
                        parse_rules(inner, &m, out);
                    } else if name == "supports" {
                        parse_rules(inner, media, out);
                    }
                    i = end;
                }
                (_, Some(sc)) => i += sc + 1,
                _ => break,
            }
            continue;
        }
        let Some(b) = rest.find('{') else { break };
        let end = block_end(s, i + b);
        let prelude = &rest[..b];
        let body = &s[i + b + 1..end.saturating_sub(1).max(i + b + 1)];
        let selectors = parse_selectors(prelude);
        if !selectors.is_empty() {
            out.push(Rule { selectors, decls: parse_declarations(body), media: media.to_vec() });
        }
        i = end;
    }
}

/// Declarations (`color: red; margin: 0 auto !important`), shorthands
/// expanded into longhands.
pub fn parse_declarations(s: &str) -> Vec<Decl> {
    let mut out = Vec::new();
    for part in split_top(&strip_comments(s), ';') {
        let Some((name, value)) = part.split_once(':') else { continue };
        let name = name.trim().to_ascii_lowercase();
        let mut value = value.trim().to_string();
        let mut important = false;
        if let Some(p) = value.rfind('!') {
            if value[p + 1..].trim().eq_ignore_ascii_case("important") {
                important = true;
                value = value[..p].trim().to_string();
            }
        }
        if name.is_empty() || value.is_empty() {
            continue;
        }
        expand(&name, &value, important, &mut out);
    }
    out
}

/// Space-separated components outside parentheses.
pub fn components(s: &str) -> Vec<&str> {
    split_top(s, ' ').into_iter().flat_map(|p| p.split(['\t', '\n'])).filter(|p| !p.is_empty()).collect()
}

const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];

/// 1-4 values to top, right, bottom, left.
fn four<'a>(v: &[&'a str]) -> Option<[&'a str; 4]> {
    Some(match v.len() {
        1 => [v[0], v[0], v[0], v[0]],
        2 => [v[0], v[1], v[0], v[1]],
        3 => [v[0], v[1], v[2], v[1]],
        4 => [v[0], v[1], v[2], v[3]],
        _ => return None,
    })
}

fn expand(name: &str, value: &str, important: bool, out: &mut Vec<Decl>) {
    let mut push = |n: &str, v: &str| out.push(Decl { name: n.to_string(), value: v.to_string(), important });
    let wide = matches!(value.to_ascii_lowercase().as_str(), "inherit" | "initial" | "unset");
    let parts = components(value);
    match name {
        "margin" | "padding" => {
            if wide {
                for s in SIDES {
                    push(&format!("{name}-{s}"), value);
                }
            } else if let Some(v) = four(&parts) {
                for (s, x) in SIDES.iter().zip(v) {
                    push(&format!("{name}-{s}"), x);
                }
            }
        }
        "inset" => {
            if let Some(v) = four(&parts) {
                for (s, x) in SIDES.iter().zip(v) {
                    push(s, x);
                }
            }
        }
        "border-width" | "border-style" | "border-color" => {
            let kind = &name[7..];
            if wide {
                for s in SIDES {
                    push(&format!("border-{s}-{kind}"), value);
                }
            } else if let Some(v) = four(&parts) {
                for (s, x) in SIDES.iter().zip(v) {
                    push(&format!("border-{s}-{kind}"), x);
                }
            }
        }
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" | "outline" => {
            if name == "outline" {
                return;
            }
            let sides: Vec<&str> = if name == "border" { SIDES.to_vec() } else { vec![&name[7..]] };
            let (mut w, mut st, mut c) = ("medium".to_string(), "none".to_string(), "currentcolor".to_string());
            if wide {
                (w, st, c) = (value.into(), value.into(), value.into());
            } else {
                for p in &parts {
                    let l = p.to_ascii_lowercase();
                    if matches!(l.as_str(), "none" | "hidden" | "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset") {
                        st = l;
                    } else if matches!(l.as_str(), "thin" | "medium" | "thick") || l.starts_with(|ch: char| ch.is_ascii_digit() || ch == '.') {
                        w = l;
                    } else {
                        c = p.to_string();
                    }
                }
            }
            for s in sides {
                push(&format!("border-{s}-width"), &w);
                push(&format!("border-{s}-style"), &st);
                push(&format!("border-{s}-color"), &c);
            }
        }
        "border-radius" => push("border-radius", parts.first().map(|p| p.split('/').next().unwrap_or(p)).unwrap_or(value)),
        "border-top-left-radius" | "border-top-right-radius" | "border-bottom-left-radius" | "border-bottom-right-radius" => {
            push("border-radius", parts.first().copied().unwrap_or(value))
        }
        "background" => {
            if wide {
                push("background-color", value);
                push("background-image", value);
                return;
            }
            let mut color = "transparent".to_string();
            let mut image = "none".to_string();
            for p in &parts {
                let l = p.to_ascii_lowercase();
                if l.starts_with("url(") {
                    image = p.to_string();
                } else if crate::style::parse_color(p, 0).is_some() {
                    color = p.to_string();
                }
            }
            push("background-color", &color);
            push("background-image", &image);
        }
        "flex" => {
            let l = value.to_ascii_lowercase();
            let (g, s, b) = match l.as_str() {
                "none" => ("0", "0", "auto"),
                "auto" => ("1", "1", "auto"),
                "initial" => ("0", "1", "auto"),
                _ => {
                    let nums: Vec<&str> = parts.iter().copied().filter(|p| p.parse::<f32>().is_ok()).collect();
                    let basis = parts.iter().copied().find(|p| p.parse::<f32>().is_err()).unwrap_or("0%");
                    (nums.first().copied().unwrap_or("1"), nums.get(1).copied().unwrap_or("1"), basis)
                }
            };
            push("flex-grow", g);
            push("flex-shrink", s);
            push("flex-basis", b);
        }
        "flex-flow" => {
            for p in &parts {
                if p.contains("wrap") {
                    push("flex-wrap", p);
                } else {
                    push("flex-direction", p);
                }
            }
        }
        "gap" | "grid-gap" => {
            push("row-gap", parts.first().copied().unwrap_or(value));
            push("column-gap", parts.get(1).or(parts.first()).copied().unwrap_or(value));
        }
        "font" => {
            // [style] [weight] size[/line-height] family...
            for (i, p) in parts.iter().enumerate() {
                let l = p.to_ascii_lowercase();
                if l == "italic" || l == "oblique" {
                    push("font-style", "italic");
                } else if l == "bold" || l == "bolder" || l.parse::<u32>().is_ok_and(|w| w >= 600) {
                    push("font-weight", "bold");
                } else if l.starts_with(|c: char| c.is_ascii_digit() || c == '.')
                    || matches!(l.as_str(), "small" | "medium" | "large" | "x-large" | "xx-large" | "x-small" | "xx-small" | "smaller" | "larger")
                {
                    let (size, lh) = match p.split_once('/') {
                        Some((s, lh)) => (s, Some(lh)),
                        None => (*p, None),
                    };
                    push("font-size", size);
                    if let Some(lh) = lh {
                        push("line-height", lh);
                    }
                    push("font-family", &parts[i + 1..].join(" "));
                    break;
                }
            }
        }
        "list-style" => {
            for p in &parts {
                let l = p.to_ascii_lowercase();
                if matches!(l.as_str(), "none" | "disc" | "circle" | "square" | "decimal" | "lower-alpha" | "upper-alpha" | "lower-roman" | "upper-roman") {
                    push("list-style-type", &l);
                }
            }
        }
        "text-decoration" | "text-decoration-line" => {
            let lines: Vec<&str> = parts.iter().copied().filter(|p| matches!(p.to_ascii_lowercase().as_str(), "none" | "underline" | "line-through" | "overline")).collect();
            push("text-decoration", &if lines.is_empty() { value.to_string() } else { lines.join(" ") });
        }
        "overflow-x" | "overflow-y" => push("overflow", value),
        "overflow" => push("overflow", parts.last().copied().unwrap_or(value)),
        "word-wrap" => push("overflow-wrap", value),
        _ => push(name, value),
    }
}

/// A length in px against a font size of `em` (no percentages): `10px`, `2em`, `0`.
pub fn parse_length_px(s: &str, em: f32) -> Option<f32> {
    let s = s.trim().to_ascii_lowercase();
    if s == "0" {
        return Some(0.0);
    }
    let unit_at = s.find(|c: char| c.is_ascii_alphabetic() || c == '%')?;
    let n: f32 = s[..unit_at].parse().ok()?;
    Some(match &s[unit_at..] {
        "px" => n,
        "em" | "rem" => n * em,
        "pt" => n * 4.0 / 3.0,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    fn sel(s: &str) -> Selector {
        parse_selector(s).unwrap_or_else(|| panic!("{s} doesn't parse"))
    }

    #[test]
    fn parses_rules_media_and_comments() {
        let css = "/* c */ h1, .a > b { color: red; margin: 0 auto !important }\n@media (max-width: 400px) { nav { display: none } }\n@import url(x.css);\n@font-face { font-family: x }\np{}";
        let s = Stylesheet::parse(css);
        assert_eq!(s.rules.len(), 3);
        assert_eq!(s.rules[0].selectors.len(), 2);
        assert_eq!(s.rules[0].decls[0], Decl { name: "color".into(), value: "red".into(), important: false });
        let m: Vec<_> = s.rules[0].decls.iter().filter(|d| d.name.starts_with("margin")).map(|d| (d.name.as_str(), d.value.as_str(), d.important)).collect();
        assert_eq!(m, [("margin-top", "0", true), ("margin-right", "auto", true), ("margin-bottom", "0", true), ("margin-left", "auto", true)]);
        assert_eq!(s.rules[1].media.len(), 1);
        assert!(s.rules[1].media[0].matches(300.0, 200.0));
        assert!(!s.rules[1].media[0].matches(500.0, 200.0));
    }

    #[test]
    fn media_queries() {
        let m = |q: &str, w: f32| Media::parse(q).matches(w, 300.0);
        assert!(m("screen and (min-width: 600px)", 800.0));
        assert!(!m("screen and (min-width: 600px)", 500.0));
        assert!(!m("print", 800.0));
        assert!(m("print, (max-width: 40em)", 400.0));
        assert!(m("not print", 400.0));
        assert!(!m("(min-resolution: 2dppx)", 400.0));
    }

    #[test]
    fn specificity() {
        assert_eq!(sel("p").specificity, 1);
        assert_eq!(sel("div p.x").specificity, 102);
        assert_eq!(sel("#a .b[c]:hover").specificity, 10300);
        assert_eq!(sel("*").specificity, 0);
        assert_eq!(sel("a:not(.x)").specificity, 101);
        assert!(parse_selector("> p").is_none());
        assert!(parse_selector("p >").is_none());
        assert!(parse_selector(".").is_none());
    }

    #[test]
    fn matching() {
        let d = html::parse("<div id=main class='box wide'><p>a</p><p class=x lang=en-us>b</p><ul><li>1<li>2<li>3</ul></div><a href=x>l</a>");
        let dy = Dynamic::default();
        let ps = d.find_all("p");
        let lis = d.find_all("li");
        let m = |s: &str, n: NodeId| sel(s).matches(&d, n, &dy);
        assert!(m("div p", ps[0]));
        assert!(m("#main > p", ps[0]));
        assert!(m(".box.wide p.x", ps[1]));
        assert!(!m(".box.narrow p", ps[1]));
        assert!(m("p:first-child", ps[0]));
        assert!(!m("p:first-child", ps[1]));
        assert!(m("p + p", ps[1]));
        assert!(m("p ~ ul", d.find("ul").unwrap()));
        assert!(m("li:nth-child(2n+1)", lis[0]) && !m("li:nth-child(odd)", lis[1]) && m("li:nth-child(odd)", lis[2]));
        assert!(m("li:last-child", lis[2]));
        assert!(m("li:nth-last-child(3)", lis[0]));
        assert!(m("[lang|=en]", ps[1]) && m("[class~=x]", ps[1]) && m("[lang^=en]", ps[1]) && m("[lang$=us]", ps[1]));
        assert!(m("p:not(.x)", ps[0]) && !m("p:not(.x)", ps[1]));
        assert!(m("a:link", d.find("a").unwrap()));
        assert!(!m("p::before", ps[0]));
        assert!(!m("p:hover", ps[0]));
        let hov = Dynamic { hover: vec![ps[0], d.find("div").unwrap()], ..Default::default() };
        assert!(sel("div:hover p:hover").matches(&d, ps[0], &hov));
    }

    #[test]
    fn shorthands() {
        let names = |s: &str| parse_declarations(s).into_iter().map(|d| format!("{}={}", d.name, d.value)).collect::<Vec<_>>();
        assert_eq!(names("border: 1px solid #ccc").len(), 12);
        assert!(names("border-bottom: 2px dashed red").contains(&"border-bottom-color=red".to_string()));
        assert_eq!(names("background: #333 url(a.png)"), ["background-color=#333", "background-image=url(a.png)"]);
        assert_eq!(names("flex: 1"), ["flex-grow=1", "flex-shrink=1", "flex-basis=0%"]);
        assert_eq!(names("flex: 0 0 200px"), ["flex-grow=0", "flex-shrink=0", "flex-basis=200px"]);
        assert_eq!(names("flex: none"), ["flex-grow=0", "flex-shrink=0", "flex-basis=auto"]);
        assert_eq!(names("gap: 4px 8px"), ["row-gap=4px", "column-gap=8px"]);
        assert_eq!(names("font: bold 16px/20px monospace"), ["font-weight=bold", "font-size=16px", "line-height=20px", "font-family=monospace"]);
        assert_eq!(names("padding: 1px 2px 3px"), ["padding-top=1px", "padding-right=2px", "padding-bottom=3px", "padding-left=2px"]);
        assert_eq!(names("color: rgb(1, 2, 3); ; junk; :x"), ["color=rgb(1, 2, 3)"]);
    }
}
