//! The engine's display list as scene nodes for the page's custom area: the
//! page background, one group moved by the scroll offset holding the items
//! that are on screen (clip and opacity groups kept), the fixed layer over
//! it (not scrolled), and a scrollbar. Pictures are image nodes on the
//! resources the driver uploaded.

use crate::model::Img;
use ecm_ui::diff::SceneNode;
use ecm_ui_proto::{NodeKind, Prop};
use ecm_web::paint::{DisplayList, Item};
use std::collections::HashMap;

/// Items this far outside the visible band are still sent (smooth small scrolls).
const MARGIN: i32 = 64;

fn rect(key: String, x: i32, y: i32, w: i32, h: i32, fill: u32) -> SceneNode {
    SceneNode::new(key, NodeKind::Rect, vec![Prop::X(x), Prop::Y(y), Prop::W(w), Prop::H(h), Prop::Fill(fill)])
}

/// Items as nodes (keys `<prefix><index>`), only those whose y range meets `[top, bottom]`.
fn convert(items: &[Item], prefix: &str, top: i32, bottom: i32, images: &HashMap<String, Option<Img>>) -> Vec<SceneNode> {
    let visible = |y: i32, hh: i32| y + hh >= top && y <= bottom;
    let mut stack: Vec<(SceneNode, i32, i32)> = Vec::new();
    let mut origin = (0, 0);
    let mut children: Vec<Vec<SceneNode>> = vec![Vec::new()];
    for (i, item) in items.iter().enumerate() {
        let (ox, oy) = origin;
        let key = format!("{prefix}{i}");
        match item {
            Item::Rect { x, y, w, h, fill, radius, border_w, border_color } => {
                if !visible(*y, *h) {
                    continue;
                }
                let mut props = vec![Prop::X(x - ox), Prop::Y(y - oy), Prop::W(*w), Prop::H(*h), Prop::Fill(*fill)];
                if *radius > 0 {
                    props.push(Prop::Radius(*radius));
                }
                if *border_w > 0 {
                    props.push(Prop::BorderW(*border_w));
                    props.push(Prop::BorderColor(*border_color));
                }
                children.last_mut().unwrap().push(SceneNode::new(key, NodeKind::Rect, props));
            }
            Item::Text { x, y, text, font, size, color } => {
                if !visible(*y, *size as i32) {
                    continue;
                }
                children.last_mut().unwrap().push(SceneNode::new(
                    key,
                    NodeKind::Text,
                    vec![Prop::X(x - ox), Prop::Y(y - oy), Prop::Text(text.clone()), Prop::Font(*font), Prop::Size(*size), Prop::Color(*color)],
                ));
            }
            Item::Image { x, y, w, h, src } => {
                if !visible(*y, *h) || *w <= 0 || *h <= 0 {
                    continue;
                }
                if let Some(Some(img)) = images.get(src) {
                    children.last_mut().unwrap().push(SceneNode::new(
                        key,
                        NodeKind::Image,
                        vec![Prop::X(x - ox), Prop::Y(y - oy), Prop::W(*w), Prop::H(*h), Prop::Resource(img.resource)],
                    ));
                }
            }
            Item::Clip { x, y, w, h } => {
                let g = SceneNode::new(key, NodeKind::Group, vec![Prop::X(x - ox), Prop::Y(y - oy), Prop::W(*w), Prop::H(*h), Prop::Clip(true)]);
                stack.push((g, ox, oy));
                origin = (*x, *y);
                children.push(Vec::new());
            }
            Item::Opacity(o) => {
                let g = SceneNode::new(key, NodeKind::Group, vec![Prop::X(0), Prop::Y(0), Prop::Opacity(*o)]);
                stack.push((g, ox, oy));
                children.push(Vec::new());
            }
            Item::Pop => {
                if let Some((mut g, px, py)) = stack.pop() {
                    g.children = children.pop().unwrap_or_default();
                    origin = (px, py);
                    if !g.children.is_empty() {
                        children.last_mut().unwrap().push(g);
                    }
                }
            }
        }
    }
    // Unbalanced pushes (never from the painter): close them.
    while let Some((mut g, _, _)) = stack.pop() {
        g.children = children.pop().unwrap_or_default();
        children.last_mut().unwrap().push(g);
    }
    children.pop().unwrap_or_default()
}

/// Nodes for a `w x h` area showing `list` scrolled down by `scroll`.
pub fn page_nodes(list: &DisplayList, w: i32, h: i32, scroll: i32, thumb: u32, images: &HashMap<String, Option<Img>>) -> Vec<SceneNode> {
    let mut out = vec![rect("bg".into(), 0, 0, w, h, list.background)];
    let mut doc = SceneNode::new("doc", NodeKind::Group, vec![Prop::X(0), Prop::Y(-scroll), Prop::W(w), Prop::H(list.height.ceil() as i32)]);
    doc.children = convert(&list.items, "i", scroll - MARGIN, scroll + h + MARGIN, images);
    out.push(doc);
    if !list.fixed.is_empty() {
        let mut fixed = SceneNode::new("fixed", NodeKind::Group, vec![Prop::X(0), Prop::Y(0), Prop::W(w), Prop::H(h)]);
        fixed.children = convert(&list.fixed, "f", -MARGIN, h + MARGIN, images);
        out.push(fixed);
    }
    // A scrollbar when the page is taller than the area.
    let ph = list.height.ceil() as i32;
    if ph > h && h > 0 {
        let th = ((h as i64 * h as i64) / ph as i64).max(16) as i32;
        let ty = ((h - th) as i64 * scroll as i64 / (ph - h).max(1) as i64) as i32;
        let mut t = rect("thumb".into(), w - 5, ty, 4, th, thumb);
        t.props.push(Prop::Radius(2));
        t.props.sort_by_key(|p| p.tag());
        out.push(t);
    }
    out
}
