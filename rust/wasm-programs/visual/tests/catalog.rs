//! The palette's peripheral blocks as the catalog changes (docs/CC_COMPAT.md,
//! "Dynamic Visual Programmer blocks"): a type seen again with more methods
//! gains their blocks, and a saved program carries the signatures it uses, so
//! it opens and generates the same Python where those peripherals aren't attached.

use ecm_ui::Json;
use ecm_visual::blocks::{catalog_from_json, PeripheralMethod, PeripheralType, Registry};
use ecm_visual::codegen::generate;
use ecm_visual::graph::{Block, Graph};

fn method(name: &str, params: &[(&str, &str, &str)], returns: &str) -> PeripheralMethod {
    PeripheralMethod {
        name: name.into(),
        description: String::new(),
        params: params.iter().map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string())).collect(),
        returns: returns.into(),
    }
}

fn chest(methods: Vec<PeripheralMethod>) -> PeripheralType {
    PeripheralType { name: "inventory".into(), description: "Any inventory".into(), methods }
}

#[test]
fn a_type_seen_again_gains_the_methods_it_lacked() {
    let mut reg = Registry::builtin();
    assert!(reg.add_peripherals(&[chest(vec![method("size", &[], "number")])]));
    assert!(reg.find("inventory_size").is_some() && reg.find("inventory_tanks").is_none());
    let categories = reg.categories.len();
    // The same block seen with a tank as well (a block with items and fluids).
    assert!(reg.add_peripherals(&[chest(vec![method("size", &[], "number"), method("tanks", &[], "any")])]));
    assert!(reg.find("inventory_tanks").is_some() && reg.find("inventory_size").is_some());
    assert_eq!(reg.categories.len(), categories, "still one Inventory category");
    assert!(!reg.add_peripherals(&[chest(vec![method("tanks", &[], "any")])]), "nothing new: no change");
}

#[test]
fn a_saved_program_keeps_the_signatures_it_uses() {
    let mut live = Registry::builtin();
    live.add_peripherals(&[
        chest(vec![method("push_items", &[("to_name", "string", ""), ("from_slot", "number", "0")], "number")]),
        PeripheralType { name: "speaker".into(), description: String::new(), methods: vec![method("status", &[], "string")] },
    ]);
    let mut g = Graph::new("mover");
    g.blocks.push(Block::new(1, live.find("start").unwrap(), 0.0, 0.0));
    let mut push = Block::new(2, live.find("inventory_push_items").unwrap(), 200.0, 0.0);
    push.set("to_name", "right");
    push.set("from_slot", "3");
    g.blocks.push(push);
    g.conns.push(ecm_visual::graph::Conn { from: 1, from_port: "flow".into(), to: 2, to_port: "flow".into() });
    let text = g.save(&live);
    let python = generate(&g, &live, None);

    let root = Json::parse(&text).unwrap();
    let saved = catalog_from_json(root.get("peripherals").expect("the signatures are saved")).unwrap();
    assert_eq!(saved.len(), 1, "only the types the program uses: {saved:?}");
    assert_eq!(saved[0].name, "inventory");

    // Where no inventory is attached the block would be dropped...
    let (_, skipped) = Graph::load(&text, &Registry::builtin(), 1).unwrap();
    assert_eq!(skipped, ["inventory_push_items"]);
    // ...unless the editor learns the saved signatures first (Editor::open does).
    let mut bare = Registry::builtin();
    bare.add_peripherals(&saved);
    let (g2, skipped) = Graph::load(&text, &bare, 1).unwrap();
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(generate(&g2, &bare, None), python, "the same program");
}
