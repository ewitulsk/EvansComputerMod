//! `.ui.json` layout files (§13 "Layout files, for the Visual Programmer").
//!
//! ```json
//! { "window": "Notes",
//!   "root": { "column": { "gap": 8, "padding": 12, "children": [
//!     { "row": { "gap": 8, "children": [
//!       { "textfield": { "id": "entry", "placeholder": "New note" } },
//!       { "button": { "text": "Add", "on_click": "add_note" } } ] } },
//!     { "list": { "id": "rows" } } ] } } }
//! ```
//!
//! A node is an object with one key, the widget kind (`label`, `button`,
//! `textfield`, `list`, `scroll`, `checkbox`, `toggle`, `slider`, `tabs`,
//! `menu`, `dialog`, `image`, `canvas`, `row`, `column`, `stack`, `grid`,
//! `spacer`, `responsive`, `textarea`, `custom`), whose value holds its props:
//!
//! | Prop | Type |
//! | --- | --- |
//! | `id`, `text`, `placeholder`, `style` | string |
//! | `gap`, `padding`, `width`, `height`, `value`, `min`, `max`, `step`, `grow`, `size`, `resource`, `scale` | integer |
//! | `checked`, `wrap`, `enabled`, `line_numbers`, `password` | bool |
//! | `visible_in` | `[compact, medium, wide]` bools |
//! | `selected` | integer or null |
//! | `align` | `"start"`, `"center"`, `"end"`, `"stretch"` |
//! | `items` | strings (tab titles, menu items) |
//! | `columns` | integer, or `[compact, medium, wide]` |
//! | `font` | `"ui"` or `"mono"` |
//! | `color` | integer `0xRRGGBBAA` or `"#rrggbb"` / `"#rrggbbaa"` |
//! | `on_click`, `on_change`, `on_submit`, `on_select`, `on_activate`, `on_pointer`, `on_key`, `on_text`, `on_paste` | handler name (string) |
//! | `children` | nodes; `child` = one node (scroll views) |
//! | `compact`, `medium`, `wide` | nodes (`responsive` only) |
//!
//! Unknown kinds and props are errors, so typos don't fail silently. The
//! Python module hands its widget trees over in the same form, with integer
//! handler references instead of names.

use crate::element::{Align, Element, EventKind, Handler, Kind};
use crate::json::Json;
use ecm_ui_proto::Font;

/// A loaded layout file.
#[derive(Clone, Debug)]
pub struct LayoutFile {
    pub title: String,
    pub root: Element,
}

/// Load a `.ui.json` file's text.
pub fn load(text: &str) -> Result<LayoutFile, String> {
    let j = Json::parse(text)?;
    let obj = j.as_object().ok_or("a layout file is an object with \"window\" and \"root\"")?;
    let mut title = None;
    let mut root = None;
    for (k, v) in obj {
        match k.as_str() {
            "window" => title = Some(v.as_str().ok_or("\"window\" must be a string")?.to_string()),
            "root" => root = Some(element(v, "root")?),
            other => return Err(format!("unknown key {other:?} in the layout file")),
        }
    }
    Ok(LayoutFile { title: title.unwrap_or_default(), root: root.ok_or("the layout file has no \"root\"")? })
}

fn int(v: &Json, path: &str, key: &str) -> Result<i64, String> {
    v.as_i64().ok_or_else(|| format!("{path}.{key} must be an integer"))
}

fn i32_of(v: &Json, path: &str, key: &str) -> Result<i32, String> {
    i32::try_from(int(v, path, key)?).map_err(|_| format!("{path}.{key} is out of range"))
}

fn string(v: &Json, path: &str, key: &str) -> Result<String, String> {
    v.as_str().map(String::from).ok_or_else(|| format!("{path}.{key} must be a string"))
}

fn boolean(v: &Json, path: &str, key: &str) -> Result<bool, String> {
    v.as_bool().ok_or_else(|| format!("{path}.{key} must be true or false"))
}

fn color(v: &Json, path: &str) -> Result<u32, String> {
    if let Some(i) = v.as_i64() {
        return u32::try_from(i).map_err(|_| format!("{path}.color is out of range"));
    }
    let s = v.as_str().ok_or_else(|| format!("{path}.color must be an integer or \"#rrggbb[aa]\""))?;
    let hex = s.strip_prefix('#').ok_or_else(|| format!("{path}.color must start with #"))?;
    let n = u32::from_str_radix(hex, 16).map_err(|_| format!("{path}.color is not hex"))?;
    match hex.len() {
        6 => Ok(n << 8 | 0xff),
        8 => Ok(n),
        _ => Err(format!("{path}.color must have 6 or 8 hex digits")),
    }
}

fn handler(v: &Json, path: &str, key: &str) -> Result<Handler, String> {
    match v {
        Json::Str(s) => Ok(Handler::Named(s.clone())),
        Json::Int(i) => Ok(Handler::Ref(*i)),
        _ => Err(format!("{path}.{key} must be a handler name")),
    }
}

/// One node of a layout tree.
pub fn element(node: &Json, path: &str) -> Result<Element, String> {
    let obj = node.as_object().ok_or_else(|| format!("{path} must be an object like {{\"label\": {{...}}}}"))?;
    if obj.len() != 1 {
        return Err(format!("{path} must have exactly one key, the widget kind"));
    }
    let (kind_name, body) = &obj[0];
    let kind = Kind::from_name(kind_name).ok_or_else(|| format!("{path}: unknown widget {kind_name:?}"))?;
    let path = format!("{path}.{kind_name}");
    let props = body.as_object().ok_or_else(|| format!("{path} must be an object of props"))?;
    let mut e = Element::new(kind);
    let mut responsive: [Option<Element>; 3] = [None, None, None];
    for (k, v) in props {
        let k = k.as_str();
        let p = &mut e.props;
        match k {
            "id" => e.id = Some(string(v, &path, k)?),
            "text" => p.text = Some(string(v, &path, k)?),
            "placeholder" => p.placeholder = Some(string(v, &path, k)?),
            "style" => p.style = Some(string(v, &path, k)?),
            "gap" => p.gap = Some(i32_of(v, &path, k)?),
            "padding" => p.padding = Some(i32_of(v, &path, k)?),
            "width" => p.width = Some(i32_of(v, &path, k)?),
            "height" => p.height = Some(i32_of(v, &path, k)?),
            "value" => p.value = Some(i32_of(v, &path, k)?),
            "min" => p.min = Some(i32_of(v, &path, k)?),
            "max" => p.max = Some(i32_of(v, &path, k)?),
            "step" => p.step = Some(i32_of(v, &path, k)?),
            "grow" => p.grow = Some(u16::try_from(int(v, &path, k)?).map_err(|_| format!("{path}.grow is out of range"))?),
            "size" => p.size = Some(u8::try_from(int(v, &path, k)?).map_err(|_| format!("{path}.size is out of range"))?),
            "resource" => p.resource = Some(u32::try_from(int(v, &path, k)?).map_err(|_| format!("{path}.resource is out of range"))?),
            "scale" => p.scale = Some(u8::try_from(int(v, &path, k)?).map_err(|_| format!("{path}.scale is out of range"))?),
            "checked" => p.checked = Some(boolean(v, &path, k)?),
            "wrap" => p.wrap = Some(boolean(v, &path, k)?),
            "enabled" => p.enabled = Some(boolean(v, &path, k)?),
            "line_numbers" => p.line_numbers = Some(boolean(v, &path, k)?),
            "password" => p.password = Some(boolean(v, &path, k)?),
            "visible_in" => {
                let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| format!("{path}.visible_in must be [compact, medium, wide]"))?;
                p.visible_in = Some([boolean(&a[0], &path, k)?, boolean(&a[1], &path, k)?, boolean(&a[2], &path, k)?]);
            }
            "selected" => {
                p.selected = match v {
                    Json::Null => None,
                    _ => Some(usize::try_from(int(v, &path, k)?).map_err(|_| format!("{path}.selected is out of range"))?),
                }
            }
            "align" => p.align = Align::from_name(&string(v, &path, k)?).ok_or_else(|| format!("{path}.align: start, center, end or stretch"))?,
            "items" => {
                let a = v.as_array().ok_or_else(|| format!("{path}.items must be a list of strings"))?;
                p.items = a.iter().map(|x| string(x, &path, k)).collect::<Result<_, _>>()?;
            }
            "columns" => {
                p.columns = Some(match v {
                    Json::Arr(a) if a.len() == 3 => {
                        let c = |i: usize| u16::try_from(int(&a[i], &path, k)?).map_err(|_| format!("{path}.columns is out of range"));
                        [c(0)?, c(1)?, c(2)?]
                    }
                    _ => {
                        let n = u16::try_from(int(v, &path, k)?).map_err(|_| format!("{path}.columns is out of range"))?;
                        [n, n, n]
                    }
                })
            }
            "font" => {
                p.font = Some(match v.as_str() {
                    Some("ui") => Font::Ui,
                    Some("mono") => Font::Mono,
                    _ => return Err(format!("{path}.font must be \"ui\" or \"mono\"")),
                })
            }
            "color" => p.color = Some(color(v, &path)?),
            "on_click" | "on_change" | "on_submit" | "on_select" | "on_activate" | "on_pointer" | "on_key" | "on_text" | "on_paste" => {
                let h = handler(v, &path, k)?;
                e = e.on(EventKind::from_prop(k).unwrap(), h);
            }
            "children" => {
                let a = v.as_array().ok_or_else(|| format!("{path}.children must be a list"))?;
                for (i, c) in a.iter().enumerate() {
                    e.children.push(element(c, &format!("{path}.children[{i}]"))?);
                }
            }
            "child" => e.children.push(element(v, &format!("{path}.child"))?),
            "compact" | "medium" | "wide" if kind == Kind::Responsive => {
                let i = ["compact", "medium", "wide"].iter().position(|s| *s == k).unwrap();
                responsive[i] = Some(element(v, &format!("{path}.{k}"))?);
            }
            other => return Err(format!("{path}: unknown prop {other:?}")),
        }
    }
    if kind == Kind::Responsive && responsive.iter().any(Option::is_some) {
        // Missing classes fall back to the next smaller one (or the next larger).
        let first = responsive.iter().flatten().next().cloned().unwrap();
        let mut last = first;
        let mut out = Vec::new();
        for r in responsive.into_iter() {
            if let Some(r) = r {
                last = r;
            }
            out.push(last.clone());
        }
        e.children = out;
    }
    Ok(e)
}
