//! Sample programs built the way a player builds them, shared by the tests
//! (here and in the Python `ui` module's tests) and the `visual_app`
//! scenario: the designer-built counter app (a button that increments a
//! label).

use crate::designer::PropType;
use crate::editor::Editor;
use crate::graph::PortRef;

/// The counter app's program name.
pub const COUNTER: &str = "counter";

fn out(block: u32, port: &str) -> PortRef {
    PortRef { block, port: port.into(), output: true }
}

fn inp(block: u32, port: &str) -> PortRef {
    PortRef { block, port: port.into(), output: false }
}

/// Build the counter app in an editor, through the designer and the graph
/// as a player would: a label `count` ("Count: 0") and a button `add`; the
/// Start chain sets `count = 0`; "When add is clicked" adds one and shows
/// `"Count: " + str(count)` in the label.
pub fn build_counter(ed: &mut Editor) {
    ed.new_program();
    ed.graph.name = COUNTER.into();
    ed.show_design();
    ed.doc().title = "Counter".into();
    // Drop a label and a button onto the screen and name them.
    for (kind, id, text) in [("label", "count", "Count: 0"), ("button", "add", "Add one")] {
        let node = ed.doc().new_widget(kind);
        let path = ed.doc().drop_on(node, &[]).expect("dropped on the root");
        ed.design_sel = Some(path);
        ed.set_prop("id", PropType::Str, id).unwrap();
        ed.set_prop("text", PropType::Str, text).unwrap();
    }
    // "Add blocks when clicked" on the button makes the event block.
    let add_path = ed.doc().find_id("add").unwrap();
    ed.design_sel = Some(add_path);
    ed.edit_handler("click");
    let event = ed.selected.expect("the event block is selected");
    let reg = ed.reg.clone();
    let mut place = |ty: &str, x: f64, y: f64| ed.graph.add_block(reg.find(ty).unwrap(), x, y);
    let start = place("start", 20.0, 20.0);
    let init = place("set_variable", 20.0, 90.0);
    let bump = place("set_variable", 300.0, 150.0);
    let var = place("variable", 620.0, 20.0);
    let plus = place("add", 620.0, 120.0);
    let text = place("to_str", 620.0, 240.0);
    let join = place("concat", 620.0, 330.0);
    let show = place("ui_set_text", 300.0, 280.0);
    if let Some(b) = ed.graph.block_mut(event) {
        (b.x, b.y) = (300.0, 20.0);
    }
    let g = &mut ed.graph;
    for (b, k, v) in [
        (init, "name", "count"),
        (init, "name_type", "int"),
        (init, "value", "0"),
        (init, "value_type", "int"),
        (bump, "name", "count"),
        (bump, "name_type", "int"),
        (var, "name", "count"),
        (var, "name_type", "int"),
        (plus, "b", "1"),
        (join, "a", "Count: "),
        (show, "widget", "count"),
    ] {
        g.block_mut(b).unwrap().set(k, v);
    }
    for (a, b) in [
        (out(start, "flow"), inp(init, "flow")),
        (out(event, "body"), inp(bump, "flow")),
        (out(var, "name"), inp(plus, "a")),
        (out(plus, "result"), inp(bump, "value")),
        (out(bump, "flow"), inp(show, "flow")),
        (out(var, "name"), inp(text, "value")),
        (out(text, "result"), inp(join, "b")),
        (out(join, "result"), inp(show, "text")),
    ] {
        g.connect(&reg, &a, &b).expect("sample wiring is valid");
    }
}

/// Sample programs `visual --install-samples` writes into `/visual`: some of
/// the old editor's test graphs and the counter app with its layout.
pub fn sample_files(reg: &crate::blocks::Registry) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = [
        ("hello", include_str!("../tests/vpl/hello.vpl")),
        ("counter_loop", include_str!("../tests/vpl/counter_loop.vpl")),
        ("for_loop", include_str!("../tests/vpl/for_loop.vpl")),
        ("branches", include_str!("../tests/vpl/branches.vpl")),
        ("math_constants", include_str!("../tests/vpl/math_constants.vpl")),
        ("events", include_str!("../tests/vpl/events.vpl")),
    ]
    .iter()
    .map(|(n, t)| (format!("{}/{n}.vpl", crate::editor::DIR), t.to_string()))
    .collect();
    let mut ed = Editor::new(reg.clone(), Box::new(crate::files::MemFiles::default()), Box::new(crate::runner::MockRunner::default()));
    build_counter(&mut ed);
    if ed.save().is_ok() {
        for f in [format!("{}/{COUNTER}.vpl", crate::editor::DIR), format!("{}/{COUNTER}.ui.json", crate::editor::DIR)] {
            if let Some(t) = ed.files.read_string(&f) {
                out.push((f, t));
            }
        }
    }
    out
}
