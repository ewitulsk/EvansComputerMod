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

/// The card widgets through `import ui`: a PixelGrid's .pixels follows the
/// player's painting (and the eyedropper sets .color), pushes from Python
/// reach the screen, a locked texel stays, and a ColorPicker's .value follows
/// drags and typed hex while pushes from Python restyle it.
#[test]
fn python_pixel_grid_and_color_picker_follow_input_and_push_changes() {
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
            r##"
import ui
log = []
mask = [0] * 32
mask[5] = 1
grid = ui.PixelGrid(width=8, height=4, zoom=10, color=3, mask=mask, id="grid",
                    palette=[0x000000, 0xffffff, 0xff0000, 0x3d8bfd],
                    on_change=lambda e: log.append(("change", e.value)),
                    on_select=lambda e: log.append(("select", e.value)))
pick = ui.ColorPicker(value=0xff0000, id="pick", width=120, on_change=lambda e: log.append(("color", e.value)))
app = ui.App("dev.test.cards")
win = app.window("Cards", ui.Column(gap=0, children=[grid, pick]))
assert grid.width == 8 and grid.height == 4 and len(grid.pixels) == 32
APP = app._h
"##,
        );
        let app_h: i64 = scope.globals.get_item("APP", vm).unwrap().try_into_value(vm).unwrap();
        let step = |msgs: Vec<Message>| {
            mock.push(&msgs);
            run(vm, &scope, "app.step(0)\napp.step(0)\n");
        };
        step(vec![Message::Configure(ecm_ui_proto::Configure { surface: 1, serial: 1, w: 200, h: 300, states: ecm_ui_proto::window_states::ACTIVATED })]);
        // Texel (2, 1) is at (25, 15); the locked texel (5, 0) at (55, 5).
        step(click(hit(app_h, "grid"), 25, 15));
        step(click(hit(app_h, "grid"), 55, 5));
        run(
            vm,
            &scope,
            r##"
assert grid.pixel(2, 1) == 3, grid.pixels
assert grid.pixel(5, 0) == 0, grid.pixels
assert log == [("change", [(2, 1, 3)])], log
grid.tool = "eyedropper"
app.step(0)
"##,
        );
        step(click(hit(app_h, "grid"), 25, 15));
        run(
            vm,
            &scope,
            r##"
assert log[-1] == ("select", 3), log
assert grid.color == 3
grid.tool = "fill"
grid.color = 1
grid.set_pixel(0, 0, 2)
app.step(0)
"##,
        );
        let pushed = with_core(app_h, |c| c.window(1).unwrap().pixels("grid")).unwrap();
        assert_eq!(pushed[0], 2, "set_pixel reached the toolkit");
        assert_eq!(pushed[10], 3);
        step(click(hit(app_h, "grid"), 15, 5));
        run(
            vm,
            &scope,
            r##"
assert grid.pixel(1, 0) == 1 and grid.pixel(7, 3) == 1 and grid.pixel(0, 0) == 2 and grid.pixel(5, 0) == 0, grid.pixels
assert log[-1][0] == "change" and len(log[-1][1]) == 32 - 3, len(log[-1][1])
"##,
        );
        // The colour picker: a press on its hue strip, then a pushed value.
        let (hue, _, field) = with_core(app_h, |c| c.window(1).unwrap().picker_hit_ids("pick")).flatten().unwrap();
        let initial = with_core(app_h, |c| c.window(1).unwrap().state("pick").unwrap().value).unwrap();
        assert_eq!(initial, 0xff0000, "the constructor's value reached the toolkit");
        step(click(hue, 120 - 8, 40 + 58));
        run(
            vm,
            &scope,
            r##"
assert log[-1][0] == "color" and log[-1][1] == pick.value, (log[-1], pick.value)
assert pick.value != 0xff0000 and (pick.value >> 16) == 0, hex(pick.value)
pick.value = 0x123456
app.step(0)
assert pick.hex == "#123456"
"##,
        );
        let v = with_core(app_h, |c| c.window(1).unwrap().state("pick").unwrap().value).unwrap();
        assert_eq!(v, 0x123456);
        step(click(field, 60, 40 + 130));
        for _ in 0..7 {
            step(vec![Message::Key(ecm_ui_proto::Key { surface: 1, key: 0x2A, mods: 0, down: true, repeat: false })]);
        }
        step(vec![Message::Text(ecm_ui_proto::Text { surface: 1, text: "#00ff00".into() })]);
        run(vm, &scope, "assert pick.value == 0x00ff00, hex(pick.value)\nassert log[-1] == ('color', 0x00ff00), log[-1]\n_ui.use_mock(None)\n");
    });
    let sent = mock.sent_messages();
    assert!(sent.iter().any(|m| matches!(m, Message::TreeOps(_))));
}

/// The canvas API (protocol v2 timed frames): app.canvas, put, commit_at,
/// queue_flush, the content hint and ui.now_us; the same script runs in Lua
/// (lua/src/tests.rs) and both send these messages.
#[test]
fn python_canvases_put_commit_at_flush_and_hint() {
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    mock.push(&[samples::welcome(false), Message::QueueLimits(ecm_ui_proto::QueueLimits { max_queue_frames: 12, max_queue_ms: 500 })]);
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
app = ui.App("dev.test.video")
cv = app.canvas(4, 2, "rgb565")
assert cv.resource == 1 and cv.width == 4 and cv.height == 2 and cv.scale == 1
assert cv.content == "auto"
win = app.window("Video", ui.Column(children=[cv]))
cv.content = "motion"
assert cv.content == "motion"
try:
    cv.content = "blurry"
    raise AssertionError("bad hint accepted")
except ValueError:
    pass
try:
    ui.Canvas(resource=5).put(b"")
    raise AssertionError("an unbound canvas has no pixels")
except RuntimeError:
    pass
t0 = ui.now_us()
assert isinstance(t0, int) and ui.now_us() >= t0
"#,
        );
        let step = |msgs: Vec<Message>| {
            mock.push(&msgs);
            run(vm, &scope, "app.step(0)\napp.step(0)\n");
        };
        step(vec![Message::Configure(ecm_ui_proto::Configure { surface: 1, serial: 1, w: 200, h: 100, states: 0 })]);
        let sent = mock.sent_messages();
        assert!(sent.iter().any(|m| matches!(m, Message::ResourceCreate(r) if r.id == 1 && r.kind == ecm_ui_proto::ResourceKind::Canvas && r.format == ecm_ui_proto::PixelFormat::Rgb565 && r.w == 4)));
        assert!(sent.iter().any(|m| matches!(m, Message::SurfaceSetContent(c) if c.surface == 1 && c.resource == 1 && c.hint == 2)));
        // An untimed put: shown with the next loop turn's Commit.
        mock.clear_sent();
        run(vm, &scope, "cv.put(bytes(16), [(0, 0, 2, 1)])\napp.step(0)\n");
        let sent = mock.sent_messages();
        assert!(sent.iter().any(|m| matches!(m, Message::Commit(_))), "{sent:?}");
        // A timed one: CommitAt instead, then QueueFlush.
        mock.clear_sent();
        run(vm, &scope, "cv.put(b'\x01' * 16)\ncv.commit_at(t0 + 250000)\napp.step(0)\ncv.queue_flush()\n_ui.use_mock(None)\n");
        let sent = mock.sent_messages();
        let t0: i64 = scope.globals.get_item("t0", vm).unwrap().try_into_value(vm).unwrap();
        let names: Vec<&str> = sent.iter().map(|m| m.name()).collect();
        assert_eq!(names, vec!["CommitAt", "QueueFlush"], "{sent:?}");
        assert!(matches!(sent[0], Message::CommitAt(ecm_ui_proto::CommitAt { surface: 1, pts_us }) if pts_us == t0 as u64 + 250_000));
    });
    let px = &mock.state.borrow().pixels;
    assert_eq!(px.len(), 2);
    assert_eq!((px[0].0, px[0].1.len(), px[0].2.clone()), (1, 16, vec![[0, 0, 2, 1]]));
    assert_eq!(px[1].1, vec![1u8; 16]);
}

/// Background apps from Python: background_mode() sends BackgroundMode, the
/// answer reaches on_background_result; media_session() sends the Now Playing
/// card and a MediaAction reaches on_media; bad modes, states and actions are
/// refused before anything is sent.
#[test]
fn python_background_modes_and_media_sessions() {
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
app = ui.App("dev.test.music")
app.window("Music", ui.Label("Now playing"))
app.on_media(lambda action, pos: log.append(("media", action, pos)))
app.on_background_result(lambda mode, granted: log.append(("bg", mode, granted)))
app.background_mode("audio", True)
app.background_mode("call", on=False)
app.media_session(title="Song", artist="Band", album="LP", state="paused", position_ms=1500, duration_ms=180000, actions=["play", "next"])
for bad in (lambda: app.background_mode("party", True), lambda: app.media_session(state="loud"), lambda: app.media_session(actions=["louder"])):
    try:
        bad()
        raise AssertionError("not refused")
    except ValueError:
        pass
app.step(0)
"#,
        );
        mock.push(&[
            Message::MediaAction(ecm_ui_proto::MediaAction { action: "seek".into(), position_ms: 42000 }),
            Message::BackgroundModeResult(ecm_ui_proto::BackgroundModeResult { mode: "audio".into(), granted: true }),
        ]);
        run(vm, &scope, "app.step(0)\napp.step(0)\nassert log == [('media', 'seek', 42000), ('bg', 'audio', True)], log\napp.media_session(state='none')\napp.step(0)\n_ui.use_mock(None)\n");
    });
    let sent = mock.sent_messages();
    let modes: Vec<(String, bool)> = sent.iter().filter_map(|m| if let Message::BackgroundMode(b) = m { Some((b.mode.clone(), b.on)) } else { None }).collect();
    assert_eq!(modes, vec![("audio".to_string(), true), ("call".to_string(), false)]);
    let sessions: Vec<&ecm_ui_proto::MediaInfo> = sent.iter().filter_map(|m| if let Message::MediaSession(s) = m { Some(&s.info) } else { None }).collect();
    assert_eq!(sessions.len(), 2, "the card, then its end");
    let want = ecm_ui::MediaSession {
        title: "Song".into(),
        artist: "Band".into(),
        album: "LP".into(),
        art: None,
        state: ecm_ui::MediaState::Paused,
        position_ms: 1500,
        duration_ms: 180000,
        actions: vec!["play".into(), "next".into()],
    };
    assert_eq!(*sessions[0], want.to_wire());
    assert_eq!(*sessions[1], ecm_ui::MediaSession::ended().to_wire());
}

/// ui.wakelock(ms) writes `wakelock MS` to /dev/uictl and raises the refusal; without a
/// GUI device (here, on the host) it raises OSError; a bad ms is a TypeError/ValueError.
#[test]
fn python_wakelock_needs_the_gui_device() {
    let interp = interpreter();
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        run(vm, &scope, &format!("{SETUP}"));
        run(
            vm,
            &scope,
            "import ui
assert callable(ui.wakelock)
try:
    ui.wakelock(5000)
    raised = False
except OSError:
    raised = True
assert raised
",
        );
    });
}
