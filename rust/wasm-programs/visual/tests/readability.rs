//! Block text never runs out of its block or over other text: every block
//! (blocks.json, the peripheral catalog fixture and a peripheral with very
//! long names), at every zoom level, with default values and with long ones,
//! is drawn by the real view, and every text it draws is measured with the
//! ECM font metrics against the block, its header and its neighbours.

use ecm_ui::Json;
use ecm_ui::{Element, Kind};
use ecm_ui_proto::fonts;
use ecm_ui_proto::Font;
use ecm_visual::blocks::{catalog_from_json, PeripheralMethod, PeripheralType, Registry};
use ecm_visual::editor::Editor;
use ecm_visual::files::MemFiles;
use ecm_visual::geometry::ZOOMS;
use ecm_visual::runner::MockRunner;
use ecm_visual::view;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Copy, Clone, Debug, PartialEq)]
struct R {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl R {
    fn inside(&self, o: &R) -> bool {
        self.x >= o.x && self.y >= o.y && self.x + self.w <= o.x + o.w && self.y + self.h <= o.y + o.h
    }
    fn overlaps(&self, o: &R) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
}

fn registry() -> Registry {
    let mut r = Registry::builtin();
    let cat = Json::parse(include_str!("vpl/peripheral_catalog.json")).unwrap();
    r.add_peripherals(&catalog_from_json(&cat).unwrap());
    // A peripheral with long names everywhere (real catalogs have these).
    r.add_peripherals(&[PeripheralType {
        name: "create_redstone_link".into(),
        description: "Create Redstone Link frequency pair".into(),
        methods: vec![
            PeripheralMethod {
                name: "set_signal_strength_for_frequency".into(),
                description: "Set the signal strength sent on this link's frequency pair, from 0 to 15 (0 turns it off)".into(),
                params: vec![("first_frequency_item".into(), "string".into(), "".into()), ("signal_strength".into(), "number".into(), "15".into())],
                returns: "number".into(),
            },
            PeripheralMethod { name: "get_received_signal_strength_of_the_link".into(), description: String::new(), params: vec![], returns: "number".into() },
        ],
    }]);
    r
}

/// The box an element of the canvas board covers (board coordinates).
fn boxed(el: &Element) -> Option<R> {
    let p = &el.props;
    let (x, y) = (p.x?, p.y?);
    let size = p.size.unwrap_or(12);
    match el.kind {
        Kind::Label => {
            let f = fonts::face(p.font.unwrap_or(Font::Ui), size).unwrap();
            Some(R { x, y, w: f.text_width(p.text.as_deref().unwrap_or("")), h: f.line_height() })
        }
        Kind::TextField => {
            let f = fonts::face(Font::Ui, size).unwrap();
            Some(R { x, y, w: p.width?, h: f.line_height() + 2 * 4 })
        }
        Kind::Panel => Some(R { x, y, w: p.width?, h: p.height? }),
        _ => None,
    }
}

/// Which block an element id belongs to, and what it is.
fn owner(id: &str) -> Option<(u32, String)> {
    let mut parts = id.splitn(3, ':');
    let kind = parts.next()?;
    let block: u32 = parts.next()?.parse().ok()?;
    Some((block, format!("{kind}:{}", parts.next().unwrap_or(""))))
}

fn check(ed: &Editor, sh: &view::Shared, case: &str) -> usize {
    let board = view::canvas(ed, sh);
    let mut per: HashMap<u32, Vec<(String, R)>> = HashMap::new();
    for c in &board.children {
        let Some(id) = c.id.as_deref() else { continue };
        let Some(r) = boxed(c) else { continue };
        if let Some((b, what)) = owner(id) {
            per.entry(b).or_default().push((what, r));
        }
    }
    let mut checked = 0;
    for block in &ed.graph.blocks {
        let items = per.get(&block.id).unwrap_or_else(|| panic!("{case}: block {} drew nothing", block.ty));
        let get = |k: &str| items.iter().find(|(w, _)| w == k).map(|(_, r)| *r);
        let body = get("node:").unwrap_or_else(|| panic!("{case}: {} has no body", block.ty));
        let head = get("node:head").unwrap();
        let info = get("info:").unwrap_or_else(|| panic!("{case}: {} has no info button", block.ty));
        assert!(info.inside(&head), "{case}: {}'s info button {info:?} is outside its header {head:?}", block.ty);
        // Text and fields: inside the block; titles inside the header.
        let leaves: Vec<&(String, R)> = items
            .iter()
            .filter(|(w, _)| w.starts_with("t:") && w != "t:info" && !w.starts_with("t:tag:") && !w.starts_with("t:opt:") || w.starts_with("in:") || w.starts_with("opt:") || w.starts_with("tag:") || w == "info:")
            .collect();
        for (what, r) in &leaves {
            assert!(r.inside(&body), "{case}: {} {what} {r:?} runs out of the block {body:?}", block.ty);
            if what.starts_with("t:title:") {
                assert!(r.inside(&head), "{case}: {} title {r:?} runs out of the header {head:?}", block.ty);
            }
            checked += 1;
        }
        for (i, (wa, ra)) in leaves.iter().enumerate() {
            for (wb, rb) in leaves.iter().skip(i + 1) {
                assert!(!ra.overlaps(rb), "{case}: {}: {wa} {ra:?} overlaps {wb} {rb:?}", block.ty);
            }
        }
        // Text drawn on a panel (type tags, option fields, the info "?") stays on it.
        for (what, r) in items.iter().filter(|(w, _)| w == "t:info" || w.starts_with("t:tag:") || w.starts_with("t:opt:")) {
            let panel = match what.as_str() {
                "t:info" => info,
                w if w.starts_with("t:tag:") => get(&format!("tag:{}", &w[6..])).unwrap(),
                w => get(&format!("opt:{}", &w[6..])).unwrap(),
            };
            assert!(r.inside(&panel), "{case}: {} {what} {r:?} runs out of its box {panel:?}", block.ty);
            checked += 1;
        }
    }
    checked
}

#[test]
fn no_block_text_leaves_its_block_at_any_zoom() {
    let reg = registry();
    let names: Vec<String> = reg.categories.iter().flat_map(|c| c.blocks.iter().map(|b| b.name.clone())).collect();
    let mut total = 0;
    for long_values in [false, true] {
        for zoom in 0..ZOOMS.len() {
            let ed = Rc::new(RefCell::new(Editor::new(reg.clone(), Box::new(MemFiles::default()), Box::new(MockRunner::default()))));
            {
                let mut e = ed.borrow_mut();
                e.zoom = zoom;
                for (i, n) in names.iter().enumerate() {
                    let def = e.reg.find(n).unwrap().clone();
                    let id = e.graph.add_block(&def, (i % 4) as f64 * 700.0, (i / 4) as f64 * 400.0);
                    if long_values {
                        for p in def.data_inputs().filter(|p| !p.has_options()) {
                            e.graph.block_mut(id).unwrap().set(&p.name, "a very long value typed into this field, longer than any block");
                        }
                    }
                }
            }
            let case = format!("zoom {} %, long values {long_values}", ZOOMS[zoom] as i32 * 100 / 12);
            total += check(&ed.borrow(), &ed, &case);
        }
    }
    assert!(total > 2000, "only {total} texts checked");
    // The long peripheral title wraps onto two lines and is cut with "...".
    let ed = Rc::new(RefCell::new(Editor::new(reg.clone(), Box::new(MemFiles::default()), Box::new(MockRunner::default()))));
    {
        let mut e = ed.borrow_mut();
        let def = e.reg.find("create_redstone_link_set_signal_strength_for_frequency").unwrap().clone();
        e.graph.add_block(&def, 0.0, 0.0);
    }
    let board = view::canvas(&ed.borrow(), &ed);
    let titles: Vec<String> = board.children.iter().filter(|c| c.id.as_deref().is_some_and(|i| i.starts_with("t:1:title:"))).map(|c| c.props.text.clone().unwrap()).collect();
    assert_eq!(titles.len(), 2, "{titles:?}");
    assert!(titles[1].ends_with("..."), "{titles:?}");
}
