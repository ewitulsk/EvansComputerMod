//! Host tests of the Python `ui` module: a real RustPython interpreter with
//! `_ui` registered and ui.py loaded exactly as the bootstrap does, against
//! mock connections.
//!
//! The headline test is ROADMAP P3's "the Python and Rust versions of the
//! same sample app produce identical tree operations": the §13 Notes app in
//! Python and in Rust play the same scripted session, and every byte they
//! send must match.

use super::*;
use ecm_ui::samples::{self, click};
use ecm_ui_proto::Message;
use rustpython_vm::{compiler::Mode, Interpreter, Settings};

const SETUP: &str = "import sys, _ui\nM = type(sys)('ui')\nexec(compile(_ui._source, '<ui>', 'exec'), M.__dict__)\nsys.modules['ui'] = M\n";

fn interpreter() -> Interpreter {
    Interpreter::with_init(Settings::default(), |vm| {
        vm.add_native_module("_ui".to_owned(), Box::new(ui_native::make_module));
    })
}

/// Run `code` in `scope`; panics with the Python traceback on an exception.
fn run(vm: &VirtualMachine, scope: &rustpython_vm::scope::Scope, code: &str) {
    let code_obj = vm.compile(code, Mode::Exec, "<test>".to_owned()).expect("test script compiles");
    if let Err(e) = vm.run_code_obj(code_obj, scope.clone()) {
        let mut s = String::new();
        vm.write_exception(&mut s, &e).unwrap();
        panic!("python raised:\n{s}\nin:\n{code}");
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Describe the first difference between two message streams.
fn first_difference(a: &[u8], b: &[u8]) -> String {
    let ma = Message::decode_all(a).expect("rust stream decodes");
    let mb = Message::decode_all(b).expect("python stream decodes");
    for (i, (x, y)) in ma.iter().zip(mb.iter()).enumerate() {
        if x != y {
            return format!("message {i} differs:\n  rust:   {}\n  python: {}", x.to_json(), y.to_json());
        }
    }
    format!("rust sent {} messages, python {}", ma.len(), mb.len())
}

#[test]
fn python_and_rust_notes_apps_send_identical_bytes() {
    // Hit ids are deterministic; find the field's and the button's with a dry run.
    let dry = MockConnection::new();
    for chunk in samples::notes_session(0, 0).into_iter().take(2) {
        dry.push_bytes(chunk);
    }
    let mut app = samples::notes(Box::new(dry));
    app.step(0);
    app.step(0);
    let w = app.core.window(1).unwrap();
    let (field, add) = (w.hit_id_of("entry").unwrap(), w.hit_id_of("/0column/0row/1button").unwrap());
    let session = samples::notes_session(field, add);

    // Rust.
    let conn = MockConnection::new();
    for chunk in session.clone() {
        conn.push_bytes(chunk);
    }
    samples::notes(Box::new(conn.clone())).run();
    let rust = conn.sent();

    // Python, through `import ui`.
    let pm = MockConnection::new();
    for chunk in session {
        pm.push_bytes(chunk);
    }
    let handle = register_mock(pm.clone());
    let interp = interpreter();
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        run(vm, &scope, &format!("{SETUP}_ui.use_mock({handle})\n"));
        run(vm, &scope, samples::NOTES_PY);
        run(vm, &scope, "_ui.use_mock(None)\n");
    });
    let python = pm.sent();

    assert!(rust == python, "{}", first_difference(&rust, &python));

    // And the session really exercised the app.
    let msgs = Message::decode_all(&rust).unwrap();
    let tree_ops = msgs.iter().filter(|m| matches!(m, Message::TreeOps(_))).count();
    assert!(tree_ops >= 6, "only {tree_ops} TreeOps messages");
    let saved = msgs.iter().find_map(|m| if let Message::SaveState(s) = m { Some(s.data.clone()) } else { None }).unwrap();
    assert_eq!(String::from_utf8(saved).unwrap(), "{\"notes\":[\"first\",\"milk\",\"eggs \u{e9}\"]}");
    assert!(msgs.iter().any(|m| matches!(m, Message::HotkeyClaim(h) if h.slot == 1 && h.label == "Add note")));
    let acks: Vec<u32> = msgs.iter().filter_map(|m| if let Message::AckConfigure(a) = m { Some(a.serial) } else { None }).collect();
    assert_eq!(acks, vec![7, 8]);
    let _ = hex(&rust[..8]);
}

fn hit(app: i64, id: &str) -> u32 {
    with_core(app, |c| c.window(1).unwrap().hit_id_of(id)).flatten().unwrap_or_else(|| panic!("no hit id for {id}"))
}

#[test]
fn python_widgets_follow_input_and_push_changes() {
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    mock.push(&[samples::welcome(false)]);
    let handle = register_mock(mock.clone());
    let interp = interpreter();
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        run(vm, &scope, &format!("{SETUP}_ui.use_mock({handle})\n"));
        run(
            vm,
            &scope,
            r#"
import ui
log = []
app = ui.App("dev.test.widgets")
name = ui.TextField(id="name", placeholder="Name", on_submit=lambda e: log.append(("submit", e.value)))
agree = ui.Checkbox("Agree", id="agree", on_change=lambda e: log.append(("agree", e.value)))
vol = ui.Slider(0, 10, id="vol", width=110)
tabs = ui.Tabs(["One", "Two"], [ui.Label("page one"), ui.Label("page two")], id="tabs")
pick = ui.List(items=["a", "b", "c"], id="pick")
win = app.window("Widgets", ui.Column(gap=4, children=[name, agree, vol, tabs, pick]))
APP = app._h
"#,
        );
        let app_h: i64 = scope.globals.get_item("APP", vm).unwrap().try_into_value(vm).unwrap();
        let step = |msgs: Vec<Message>| {
            mock.push(&msgs);
            run(vm, &scope, "app.step(0)\napp.step(0)\n");
        };
        step(vec![Message::Configure(ecm_ui_proto::Configure {
            surface: 1,
            serial: 1,
            w: 400,
            h: 300,
            states: ecm_ui_proto::window_states::ACTIVATED,
        })]);
        step(click(hit(app_h, "agree"), 0, 0));
        step(click(hit(app_h, "name"), 0, 0));
        step(vec![Message::Text(ecm_ui_proto::Text { surface: 1, text: "Ann".into() })]);
        step(vec![Message::Key(ecm_ui_proto::Key { surface: 1, key: 0x28, mods: 0, down: true, repeat: false })]);
        step(vec![Message::Key(ecm_ui_proto::Key { surface: 1, key: 0x2B, mods: 0, down: true, repeat: false })]);
        step(click(hit(app_h, "tabs") + 1, 0, 0));
        step(click(hit(app_h, "pick") + 2, 0, 0));
        run(
            vm,
            &scope,
            r#"
assert agree.checked is True, agree.checked
assert name.text == "Ann", name.text
assert tabs.selected == 1, tabs.selected
assert pick.selected == 2, pick.selected
assert log == [("agree", True), ("submit", "Ann")], log
# Changing values from Python reaches the screen.
name.text = ""
agree.checked = False
vol.value = 7
app.step(0)
"#,
        );
        let state = with_core(app_h, |c| {
            let w = c.window(1).unwrap();
            (w.state("name").unwrap().text.clone(), w.state("agree").unwrap().checked, w.state("vol").unwrap().value)
        })
        .unwrap();
        assert_eq!(state, (String::new(), false, 7));
        // Every message the module sent is valid.
        let sent = mock.sent_messages();
        assert!(sent.iter().any(|m| matches!(m, Message::TreeOps(_))));

        // json helpers are the canonical encoder.
        run(
            vm,
            &scope,
            r#"
assert _ui.json_dumps({"a": [1, 2.5, None, True], "b": "x\"y"}) == '{"a":[1,2.5,null,true],"b":"x\\"y"}'
assert _ui.json_loads(b'{"n": [1, "two"]}') == {"n": [1, "two"]}
assert app.restore() == {}
_ui.use_mock(None)
"#,
        );
    });
}

#[test]
fn python_loads_ui_json_layouts_and_reports_console_mode() {
    let dir = std::env::temp_dir().join(format!("ecm-pyui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("notes.ui.json");
    std::fs::write(
        &path,
        r#"{ "window": "Notes",
  "root": { "column": { "gap": 8, "padding": 12, "children": [
    { "row": { "gap": 8, "children": [
      { "textfield": { "id": "entry", "placeholder": "New note" } },
      { "button": { "id": "add", "text": "Add", "on_click": "add_note" } } ] } },
    { "list": { "id": "rows" } } ] } } }"#,
    )
    .unwrap();
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    mock.push(&[samples::welcome(false)]);
    let handle = register_mock(mock.clone());
    let interp = interpreter();
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        run(vm, &scope, &format!("{SETUP}_ui.use_mock({handle})\nPATH = {:?}\n", path.to_string_lossy().replace('\\', "/")));
        run(
            vm,
            &scope,
            r#"
import ui
hits = []
app, win = ui.load(PATH, handlers={"add_note": lambda e: hits.append(e.kind)})
APP = app._h
"#,
        );
        let app_h: i64 = scope.globals.get_item("APP", vm).unwrap().try_into_value(vm).unwrap();
        mock.push(&[Message::Configure(ecm_ui_proto::Configure { surface: 1, serial: 1, w: 400, h: 300, states: 0 })]);
        run(vm, &scope, "app.step(0)\napp.step(0)\n");
        mock.push(&click(hit(app_h, "add"), 0, 0));
        run(vm, &scope, "app.step(0)\nassert hits == ['click'], hits\n_ui.use_mock(None)\n");
        // No compositor: a clear error.
        run(
            vm,
            &scope,
            r#"
try:
    ui.App("dev.test.console")
    raise AssertionError("connected without a GUI")
except RuntimeError as e:
    assert "console mode" in str(e), str(e)
"#,
        );
    });
}

/// ROADMAP P6: the designer-built counter app (a button that increments a
/// label), generated by the Visual Programmer, runs through `import ui`:
/// two clicks make the label say "Count: 2", and the layout helpers read
/// and change widgets by id.
#[test]
fn a_visual_programmer_counter_app_counts_clicks() {
    use ecm_visual::blocks::Registry;
    use ecm_visual::codegen::{generate, AppTarget};
    use ecm_visual::designer::sync_handlers;
    use ecm_visual::editor::Editor;
    use ecm_visual::files::MemFiles;
    use ecm_visual::runner::MockRunner;
    use ecm_visual::samples::build_counter;

    let mut ed = Editor::new(Registry::builtin(), Box::new(MemFiles::default()), Box::new(MockRunner::default()));
    build_counter(&mut ed);
    let mut doc = ed.doc.clone().unwrap();
    sync_handlers(&mut doc, &ed.graph);
    let dir = std::env::temp_dir().join(format!("ecm-visual-counter-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let layout = dir.join("counter.ui.json");
    std::fs::write(&layout, doc.to_text()).unwrap();
    let path = layout.to_string_lossy().replace('\\', "/");
    let code = generate(&ed.graph, &ed.reg, Some(&AppTarget { layout: path, app_id: "dev.visual.counter".into() }));
    // Everything but the event loop: the test steps the app itself.
    let body = code.strip_suffix("_app.run()\n").expect("the app ends with its event loop");

    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    mock.push(&[samples::welcome(false)]);
    let handle = register_mock(mock.clone());
    let interp = interpreter();
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        run(vm, &scope, &format!("{SETUP}_ui.use_mock({handle})\n"));
        run(vm, &scope, body);
        run(vm, &scope, "APP = _app._h\nassert count == 0\nassert _layout.get('count', 'text') == 'Count: 0'\n");
        let app_h: i64 = scope.globals.get_item("APP", vm).unwrap().try_into_value(vm).unwrap();
        mock.push(&[Message::Configure(ecm_ui_proto::Configure { surface: 1, serial: 1, w: 320, h: 200, states: 0 })]);
        run(vm, &scope, "_app.step(0)\n_app.step(0)\n");
        for _ in 0..2 {
            mock.push(&click(hit(app_h, "add"), 0, 0));
            run(vm, &scope, "_app.step(0)\n");
        }
        run(vm, &scope, "_app.step(0)\nassert count == 2, count\nassert _layout.get('count', 'text') == 'Count: 2'\nassert _layout.value('count') == 'Count: 2'\n_ui.use_mock(None)\n");
    });
    // The label really changed on screen.
    let sent = mock.sent_messages();
    let shown = sent.iter().any(|m| match m {
        Message::TreeOps(t) => t.ops.iter().any(|op| match op {
            ecm_ui_proto::TreeOp::Set { props, .. } | ecm_ui_proto::TreeOp::Create { props, .. } => {
                props.iter().any(|p| matches!(p, ecm_ui_proto::Prop::Text(s) if s == "Count: 2"))
            }
            _ => false,
        }),
        _ => false,
    });
    assert!(shown, "no TreeOps set the label to Count: 2");
}
