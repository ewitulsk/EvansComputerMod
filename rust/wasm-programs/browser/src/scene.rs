//! The engine's display list as scene nodes for the page's custom area: the
//! page background, one group moved by the scroll offset holding the items
//! that are on screen (clip and opacity groups kept), and a scrollbar.

use ecm_ui::diff::SceneNode;
use ecm_ui_proto::{NodeKind, Prop};
use ecm_web::paint::{DisplayList, Item};

/// Items this far outside the visible band are still sent (smooth small scrolls).
const MARGIN: i32 = 64;

fn rect(key: String, x: i32, y: i32, w: i32, h: i32, fill: u32) -> SceneNode {
    SceneNode::new(key, NodeKind::Rect, vec![Prop::X(x), Prop::Y(y), Prop::W(w), Prop::H(h), Prop::Fill(fill)])
}

/// Nodes for a `w x h` area showing `list` scrolled down by `scroll`.
pub fn page_nodes(list: &DisplayList, w: i32, h: i32, scroll: i32, thumb: u32) -> Vec<SceneNode> {
    let mut out = vec![rect("bg".into(), 0, 0, w, h, list.background)];
    let top = scroll - MARGIN;
    let bottom = scroll + h + MARGIN;
    let visible = |y: i32, hh: i32| y + hh >= top && y <= bottom;
    // Group stack: (node, origin x, origin y).
    let mut doc = SceneNode::new("doc", NodeKind::Group, vec![Prop::X(0), Prop::Y(-scroll), Prop::W(w), Prop::H(list.height.ceil() as i32)]);
    let mut stack: Vec<(SceneNode, i32, i32)> = Vec::new();
    let mut origin = (0, 0);
    let mut children: Vec<Vec<SceneNode>> = vec![Vec::new()];
    for (i, item) in list.items.iter().enumerate() {
        let (ox, oy) = origin;
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
                children.last_mut().unwrap().push(SceneNode::new(format!("i{i}"), NodeKind::Rect, props));
            }
            Item::Text { x, y, text, font, size, color } => {
                if !visible(*y, *size as i32) {
                    continue;
                }
                children.last_mut().unwrap().push(SceneNode::new(
                    format!("i{i}"),
                    NodeKind::Text,
                    vec![Prop::X(x - ox), Prop::Y(y - oy), Prop::Text(text.clone()), Prop::Font(*font), Prop::Size(*size), Prop::Color(*color)],
                ));
            }
            Item::Image { .. } => {}
            Item::Clip { x, y, w, h } => {
                let g = SceneNode::new(format!("i{i}"), NodeKind::Group, vec![Prop::X(x - ox), Prop::Y(y - oy), Prop::W(*w), Prop::H(*h), Prop::Clip(true)]);
                stack.push((g, ox, oy));
                origin = (*x, *y);
                children.push(Vec::new());
            }
            Item::Opacity(o) => {
                let g = SceneNode::new(format!("i{i}"), NodeKind::Group, vec![Prop::X(0), Prop::Y(0), Prop::Opacity(*o)]);
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
    doc.children = children.pop().unwrap_or_default();
    out.push(doc);
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
