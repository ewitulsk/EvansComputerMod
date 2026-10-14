//! The Visual Programmer app against a mock compositor: the tree it draws
//! (golden), placing, wiring, dragging and editing blocks with the pointer,
//! Save / Open / Run through the file system and a process runner, the
//! GUI designer (drag from the palette, select, inspector, events become
//! graph entry points), and Package with inferred permissions.
//!
//! Goldens live in `tests/golden/`; run with `UPDATE_GOLDENS=1` to rewrite
//! them after an intended change (then review the diff).

use ecm_ui::samples::{click, welcome};
use ecm_ui::{App, Json, MockConnection};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Message, PointerKind, TreeOp};
use ecm_visual::blocks::{catalog_from_json, Registry};
use ecm_visual::editor::{Dialog, Editor, Page};
use ecm_visual::files::{Files, MemFiles};
use ecm_visual::graph::PortRef;
use ecm_visual::runner::MockRunner;
use ecm_visual::samples::build_counter;
use ecm_visual::view::{self, Shared};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn registry() -> Registry {
    let mut r = Registry::builtin();
    let cat = Json::parse(&std::fs::read_to_string(manifest().join("tests/vpl/peripheral_catalog.json")).unwrap()).unwrap();
    r.add_peripherals(&catalog_from_json(&cat).unwrap());
    r
}

/// Compare with (or, with UPDATE_GOLDENS=1, write) `tests/golden/<name>`.
fn golden(name: &str, got: &str) {
    let path = manifest().join("tests/golden").join(name);
    if std::env::var("UPDATE_GOLDENS").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing golden {name}; run with UPDATE_GOLDENS=1"));
    if want != got {
        let line = got.lines().zip(want.lines()).position(|(a, b)| a != b).unwrap_or(0);
        panic!("{name} differs from its golden at line {}\n--- got ---\n{got}\n--- golden ---\n{want}", line + 1);
    }
}

struct Rig {
    conn: MockConnection,
    app: App,
    ed: Shared,
    files: Rc<RefCell<MemFiles>>,
    runner: Rc<RefCell<MockRunner>>,
    /// The nodes on screen: id -> (parent, text).
    model: RefCell<HashMap<u32, (u32, Option<String>)>>,
}

impl Rig {
    fn new() -> Rig {
        let files = Rc::new(RefCell::new(MemFiles::default()));
        let runner = Rc::new(RefCell::new(MockRunner::default()));
        let ed = Rc::new(RefCell::new(Editor::new(registry(), Box::new(files.clone()), Box::new(runner.clone()))));
        let conn = MockConnection::new();
        conn.state.borrow_mut().idle_timeouts = true;
        conn.push(&[welcome(false)]);
        let mut app = App::with_connection(ecm_visual::APP_ID, Box::new(conn.clone())).unwrap();
        view::install(&mut app, ed.clone());
        let mut rig = Rig { conn, app, ed, files, runner, model: RefCell::new(HashMap::new()) };
        rig.send(vec![Message::Configure(proto::Configure { surface: 1, serial: 1, w: 800, h: 500, states: proto::window_states::ACTIVATED })]);
        rig
    }

    /// Deliver messages, dispatch and render; the TreeOps sent.
    fn send(&mut self, msgs: Vec<Message>) -> Vec<TreeOp> {
        self.conn.clear_sent();
        self.conn.push(&msgs);
        self.app.step(0);
        self.ed.borrow_mut().poll_run();
        self.app.step(0);
        let ops: Vec<TreeOp> = self.conn.sent_messages().into_iter().filter_map(|m| if let Message::TreeOps(t) = m { Some(t.ops) } else { None }).flatten().collect();
        let mut m = self.model.borrow_mut();
        for op in &ops {
            let text = |props: &[proto::Prop]| props.iter().find_map(|p| if let proto::Prop::Text(t) = p { Some(t.clone()) } else { None });
            match op {
                TreeOp::Create { id, parent, props, .. } => {
                    m.insert(*id, (*parent, text(props)));
                }
                TreeOp::Set { id, props } => {
                    if let (Some(n), Some(t)) = (m.get_mut(id), text(props)) {
                        n.1 = Some(t);
                    }
                }
                TreeOp::Move { id, parent, .. } => {
                    if let Some(n) = m.get_mut(id) {
                        n.0 = *parent;
                    }
                }
                TreeOp::Remove { id } => {
                    let mut gone = vec![*id];
                    while let Some(g) = gone.pop() {
                        m.remove(&g);
                        gone.extend(m.iter().filter(|(_, n)| n.0 == g).map(|(k, _)| *k));
                    }
                }
            }
        }
        drop(m);
        ops
    }

    fn hit(&self, id: &str) -> u32 {
        self.app.core.window(1).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no hit id for {id}"))
    }

    fn click(&mut self, id: &str) -> Vec<TreeOp> {
        let h = self.hit(id);
        self.send(click(h, 0, 0))
    }

    fn press(&mut self, kind: PointerKind, id: Option<&str>, x: i32, y: i32, button: proto::Button) {
        let hit = id.map_or(0, |i| self.hit(i));
        let buttons = if kind == PointerKind::Up || kind == PointerKind::Scroll {
            0
        } else {
            match button {
                proto::Button::Right => proto::buttons::RIGHT,
                proto::Button::Middle => proto::buttons::MIDDLE,
                _ => proto::buttons::LEFT,
            }
        };
        let b = if matches!(kind, PointerKind::Down | PointerKind::Up) { button } else { proto::Button::None };
        self.send(vec![Message::Pointer(proto::Pointer {
            surface: 1,
            kind,
            x,
            y,
            buttons,
            button: b,
            scroll_x: 0,
            scroll_y: if kind == PointerKind::Scroll { -1 } else { 0 },
            hit_id: hit,
        })]);
    }

    /// Press on `from`, move, release on `to` (window coordinates).
    fn drag(&mut self, from: &str, a: (i32, i32), to: Option<&str>, b: (i32, i32)) {
        self.press(PointerKind::Down, Some(from), a.0, a.1, proto::Button::Left);
        self.press(PointerKind::Move, None, (a.0 + b.0) / 2, (a.1 + b.1) / 2, proto::Button::None);
        self.press(PointerKind::Move, to, b.0, b.1, proto::Button::None);
        self.press(PointerKind::Up, to, b.0, b.1, proto::Button::Left);
    }

    fn keys(&mut self, key: u16, n: usize) {
        for _ in 0..n {
            self.send(vec![Message::Key(proto::Key { surface: 1, key, mods: 0, down: true, repeat: false })]);
        }
    }

    fn type_text(&mut self, s: &str) {
        self.send(vec![Message::Text(proto::Text { surface: 1, text: s.into() })]);
    }
}

/// Every Create the app sent, one line each: kind and props, in order.
fn dump(ops: &[TreeOp]) -> String {
    let mut out = String::new();
    for op in ops {
        if let TreeOp::Create { id, parent, kind, props, .. } = op {
            out.push_str(&format!("{id} <{parent}> {kind:?} {props:?}\n"));
        }
    }
    out
}

#[test]
fn an_old_vpl_opens_and_draws_its_canvas() {
    let mut rig = Rig::new();
    let hello = std::fs::read(manifest().join("tests/vpl/hello.vpl")).unwrap();
    rig.files.borrow_mut().write("visual/hello.vpl", &hello).unwrap();
    rig.click("open");
    assert_eq!(rig.ed.borrow().dialog, Dialog::Open(vec!["visual/hello.vpl".into()]));
    // A fresh connection so every node is created again, then open the file.
    rig.ed.borrow_mut().open("visual/hello.vpl").unwrap();
    let conn = MockConnection::new();
    conn.state.borrow_mut().idle_timeouts = true;
    conn.push(&[welcome(false)]);
    let mut app = App::with_connection(ecm_visual::APP_ID, Box::new(conn.clone())).unwrap();
    view::install(&mut app, rig.ed.clone());
    conn.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w: 800, h: 500, states: 0 })]);
    app.step(0);
    app.step(0);
    let ops: Vec<TreeOp> = conn.sent_messages().into_iter().filter_map(|m| if let Message::TreeOps(t) = m { Some(t.ops) } else { None }).flatten().collect();
    let creates = ops.iter().filter(|o| matches!(o, TreeOp::Create { .. })).count();
    assert!(creates < 8192, "{creates} nodes");
    // The two blocks, their wire (one line node) and the labels are there.
    let lines = ops.iter().filter(|o| matches!(o, TreeOp::Create { kind: proto::NodeKind::Line, .. })).count();
    assert_eq!(lines, 1, "one wire");
    let dumped = dump(&ops);
    assert!(dumped.contains("Text(\"Start\")") && dumped.contains("Text(\"Print\")") && dumped.contains("Text(\"Hello, world!\")"));
    golden("hello_canvas.txt", &dumped);
    // Same Python as before.
    assert_eq!(rig.ed.borrow().python(), std::fs::read_to_string(manifest().join("tests/vpl/hello.py")).unwrap());
}

#[test]
fn blocks_are_placed_wired_moved_edited_and_deleted_with_the_pointer() {
    let mut rig = Rig::new();
    // Pick Start in the palette, click the canvas.
    rig.click("pal:start");
    assert_eq!(rig.ed.borrow().placing.as_deref(), Some("start"));
    rig.press(PointerKind::Down, Some("canvas"), 400, 150, proto::Button::Left);
    rig.press(PointerKind::Up, Some("canvas"), 400, 150, proto::Button::Left);
    let (bx, by) = {
        let ed = rig.ed.borrow();
        assert_eq!(ed.graph.blocks.len(), 1);
        let b = &ed.graph.blocks[0];
        (b.x, b.y)
    };
    // The block's header was centred 14 units above the pointer: that gives the canvas origin.
    let w = ecm_visual::geometry::Metrics::new(1).s(120).max(ecm_visual::geometry::Metrics::new(1).text("Start") + 20) as f64;
    let origin = ((400.0 - (bx + w / 2.0)).round() as i32, (150.0 - (by + 14.0)).round() as i32);
    rig.click("pal:print");
    rig.press(PointerKind::Down, Some("canvas"), 420, 300, proto::Button::Left);
    rig.press(PointerKind::Up, Some("canvas"), 420, 300, proto::Button::Left);
    assert_eq!(rig.ed.borrow().graph.blocks.len(), 2);
    let at = |rig: &Rig, p: &PortRef| {
        let (x, y) = rig.ed.borrow().port_screen(p).unwrap();
        (x + origin.0, y + origin.1)
    };
    // Wire Start's flow output to Print's flow input.
    let from = PortRef { block: 1, port: "flow".into(), output: true };
    let to = PortRef { block: 2, port: "flow".into(), output: false };
    let (a, b) = (at(&rig, &from), at(&rig, &to));
    rig.drag(&from.to_string(), a, Some(&to.to_string()), b);
    assert_eq!(rig.ed.borrow().graph.conns.len(), 1, "{}", rig.ed.borrow().status);
    // Drag Print by its header.
    let before = rig.ed.borrow().graph.block(2).unwrap().x;
    rig.drag("node:2:head", (b.0, b.1 + 5), None, (b.0 + 40, b.1 + 15));
    let after = rig.ed.borrow().graph.block(2).unwrap().x;
    assert!((after - before - 40.0).abs() < 0.01, "{before} -> {after}");
    // Type into Print's text field and switch its type tag.
    rig.click("in:2:text");
    rig.type_text("Hi there ");
    assert_eq!(rig.ed.borrow().graph.block(2).unwrap().get("text"), Some("Hi there Hello!"));
    rig.click("tag:2:text");
    rig.click("tag:2:text");
    assert_eq!(rig.ed.borrow().graph.block(2).unwrap().get("text_type"), Some("float"));
    // Wheel over the canvas zooms in.
    rig.press(PointerKind::Scroll, Some("canvas"), 300, 300, proto::Button::None);
    assert_eq!(rig.ed.borrow().zoom, 2);
    // Right-click a port to cut its wire; Delete removes the selected block.
    rig.press(PointerKind::Down, Some("port:2:i:flow"), 0, 0, proto::Button::Right);
    rig.press(PointerKind::Up, Some("port:2:i:flow"), 0, 0, proto::Button::Right);
    assert!(rig.ed.borrow().graph.conns.is_empty());
    rig.press(PointerKind::Down, Some("node:1"), 0, 0, proto::Button::Left);
    rig.press(PointerKind::Up, Some("node:1"), 0, 0, proto::Button::Left);
    rig.keys(ecm_ui::window::keys::DELETE, 1);
    assert_eq!(rig.ed.borrow().graph.blocks.iter().map(|b| b.id).collect::<Vec<_>>(), vec![2]);
}

#[test]
fn save_open_and_run_go_through_files_and_a_process() {
    let mut rig = Rig::new();
    let text = std::fs::read_to_string(manifest().join("tests/vpl/for_loop.vpl")).unwrap();
    rig.files.borrow_mut().write("visual/for_loop.vpl", text.as_bytes()).unwrap();
    rig.ed.borrow_mut().open("visual/for_loop.vpl").unwrap();
    // Saving needs a name; this one has one.
    rig.click("save");
    let saved = rig.files.borrow().read_string("visual/for_loop.vpl").unwrap();
    let reg = registry();
    let (g, _) = ecm_visual::graph::Graph::load(&saved, &reg, 1).unwrap();
    assert_eq!(g.blocks.len(), 9);
    // Run writes the script and starts python on it as a process.
    rig.runner.borrow_mut().script = vec!["2\r\n4\r\n".into(), "6\r\n\u{1b}[2J".into()];
    rig.click("run");
    assert_eq!(rig.runner.borrow().commands, vec!["python /visual/for_loop.py".to_string()]);
    let script = rig.files.borrow().read_string("visual/for_loop.py").unwrap();
    assert_eq!(script, std::fs::read_to_string(manifest().join("tests/vpl/for_loop.py")).unwrap());
    assert!(rig.hit("stop") > 0, "Run became Stop");
    rig.send(vec![]);
    rig.send(vec![]);
    let run = rig.ed.borrow().run.clone().unwrap();
    assert!(!run.running);
    assert_eq!(run.text, "2\n4\n6\n[finished]\n");
    // Input typed into the panel goes to the process.
    rig.runner.borrow_mut().script = vec!["x".into()];
    rig.ed.borrow_mut().start_command("python /visual/for_loop.py");
    rig.click("stdin");
    rig.type_text("hello");
    rig.keys(ecm_ui::window::keys::ENTER, 1);
    assert_eq!(rig.runner.borrow().input, vec!["hello\n".to_string()]);
    // An unnamed program can't be saved.
    rig.click("new");
    rig.click("save");
    assert!(rig.ed.borrow().status.contains("name"));
    // A graph without Start reports the old error instead of running.
    rig.click("pal:print");
    rig.press(PointerKind::Down, Some("canvas"), 400, 150, proto::Button::Left);
    rig.press(PointerKind::Up, Some("canvas"), 400, 150, proto::Button::Left);
    rig.click("run");
    assert!(rig.ed.borrow().status.starts_with("ERROR: No Start block"));
}

#[test]
fn the_designer_drops_selects_edits_and_makes_event_blocks() {
    let mut rig = Rig::new();
    rig.ed.borrow_mut().graph.name = "counter".into();
    rig.click("tab-design");
    assert_eq!(rig.ed.borrow().page, Page::Design);
    // Drag a label from the palette onto the empty screen, then a button onto the label.
    rig.drag("dpal:label", (40, 80), Some("dframe"), (400, 200));
    rig.drag("dpal:button", (40, 120), Some("d:0"), (400, 210));
    assert_eq!(rig.ed.borrow().doc.as_ref().unwrap().ids(), ["label1", "button1"]);
    assert_eq!(rig.ed.borrow().design_sel, Some(vec![1]));
    // Rename the button in the inspector.
    let rev = rig.ed.borrow().insp_rev;
    rig.click(&format!("insp{rev}:id"));
    rig.keys(ecm_ui::window::keys::END, 1);
    rig.keys(ecm_ui::window::keys::BACKSPACE, 7);
    rig.type_text("add");
    assert_eq!(rig.ed.borrow().doc.as_ref().unwrap().ids(), ["label1", "add"]);
    rig.click(&format!("insp{rev}:text"));
    rig.keys(ecm_ui::window::keys::END, 1);
    rig.keys(ecm_ui::window::keys::BACKSPACE, 6);
    rig.type_text("Add one");
    // Moving: drag the label after the button.
    rig.drag("d:0", (300, 200), Some("d:1"), (300, 240));
    assert_eq!(rig.ed.borrow().doc.as_ref().unwrap().ids(), ["add", "label1"]);
    // The button's click event becomes an entry point in the graph.
    let sel = rig.ed.borrow().doc.as_ref().unwrap().find_id("add").unwrap();
    rig.ed.borrow_mut().design_sel = Some(sel);
    rig.ed.borrow_mut().insp_rev += 1;
    rig.send(vec![]);
    rig.click("ev:click");
    let ed = rig.ed.borrow();
    assert_eq!(ed.page, Page::Program);
    let ev = ed.graph.blocks.iter().find(|b| b.ty == "on_ui_event").unwrap();
    assert_eq!((ev.get("widget"), ev.get("event")), (Some("add"), Some("click")));
    let doc = ed.doc.as_ref().unwrap();
    let node = doc.node(&doc.find_id("add").unwrap()).unwrap();
    assert_eq!(node.get_str("text"), Some("Add one"));
    assert_eq!(node.get_str("on_click"), Some("add_click"));
}

#[test]
fn the_counter_app_generates_python_and_a_layout_and_packages() {
    let mut rig = Rig::new();
    build_counter(&mut rig.ed.borrow_mut());
    rig.send(vec![]);
    golden("counter_main.py", &rig.ed.borrow().python());
    rig.click("save");
    let layout = rig.files.borrow().read_string("visual/counter.ui.json").expect("saved layout");
    ecm_ui::layout_file::load(&layout).unwrap();
    golden("counter.ui.json", &layout);
    let vpl = rig.files.borrow().read_string("visual/counter.vpl").unwrap();
    assert!(vpl.contains(r#""layout":"visual/counter.ui.json""#));
    // Reopening gives the same program and layout.
    rig.ed.borrow_mut().open("visual/counter.vpl").unwrap();
    golden("counter_main.py", &rig.ed.borrow().python());
    // Run writes the layout and starts python.
    rig.click("run");
    assert_eq!(rig.runner.borrow().commands.last().unwrap(), "python /visual/counter.py");
    rig.ed.borrow_mut().stop();
    // Package writes the app folder; no pkg here.
    rig.click("package");
    let dir = "visual/apps/dev.visual.counter";
    for f in ["app.toml", "main.py", "main.ui.json", "icon.png"] {
        assert!(rig.files.borrow().exists(&format!("{dir}/{f}")), "{f}");
    }
    golden("counter_app.toml", &rig.files.borrow().read_string(&format!("{dir}/app.toml")).unwrap());
    golden("counter_package_main.py", &rig.files.borrow().read_string(&format!("{dir}/main.py")).unwrap());
    assert!(matches!(&rig.ed.borrow().dialog, Dialog::Message(_, body) if body.contains("no `pkg` command")));
    // With pkg installed, Package also builds it; blocks add permissions.
    rig.files.borrow_mut().write("server-bin/pkg.wasm", b"\0asm").unwrap();
    {
        let mut ed = rig.ed.borrow_mut();
        let reg = ed.reg.clone();
        ed.graph.add_block(reg.find("set_redstone").unwrap(), 0.0, 400.0);
        ed.graph.add_block(reg.find("test_lamp_set_color").unwrap(), 0.0, 480.0);
        ed.graph.add_block(reg.find("ui_notify").unwrap(), 0.0, 560.0);
        ed.dialog = Dialog::None;
    }
    rig.click("package");
    assert_eq!(rig.runner.borrow().commands.last().unwrap(), "pkg build /visual/apps/dev.visual.counter");
    let toml = rig.files.borrow().read_string(&format!("{dir}/app.toml")).unwrap();
    assert!(toml.contains(r#"permissions = ["notifications", "peripherals:redstone", "peripherals:test_lamp"]"#), "{toml}");
}

#[test]
fn palette_sections_fold_and_remember_it_and_the_info_button_explains_blocks() {
    let mut rig = Rig::new();
    let has = |rig: &Rig, id: &str| rig.app.core.window(1).unwrap().hit_id_of(id).is_some();
    // Core sections start open, the extra ones folded to their headers.
    assert!(has(&rig, "pal:start") && has(&rig, "pal:print") && has(&rig, "pal:add"));
    assert!(!has(&rig, "pal:set_redstone") && !has(&rig, "pal:on_ui_event") && !has(&rig, "pal:test_lamp_set_color"));
    for folded in ["Redstone", "Events", "Convert", "App", "Test Lamp"] {
        assert!(has(&rig, &format!("cat:{folded}")), "{folded} header");
    }
    // A header opens its section; clicking it again folds it.
    rig.click("cat:Redstone");
    assert!(has(&rig, "pal:set_redstone"));
    rig.click("cat:Control");
    assert!(!has(&rig, "pal:start"), "Control folds");
    // The choice lasts for the session (a new program keeps it).
    rig.click("new");
    assert!(has(&rig, "pal:set_redstone") && !has(&rig, "pal:start"));
    rig.click("cat:Control");
    assert!(has(&rig, "pal:start"));
    // The ? beside a palette entry opens the block's information.
    rig.click("pinfo:set_redstone");
    assert_eq!(rig.ed.borrow().dialog, Dialog::Info("set_redstone".into()));
    let shown = texts(&rig);
    assert!(shown.iter().any(|t| t.contains("redstone signal strength")), "{shown:?}");
    assert!(shown.iter().any(|t| t.contains("one of Down, Up, Front, Back, Left, Right")), "{shown:?}");
    assert!(shown.iter().any(|t| t == "Example") && shown.iter().any(|t| t == "terminal.set_redstone({side}, {power})"), "{shown:?}");
    rig.click("info-close");
    assert_eq!(rig.ed.borrow().dialog, Dialog::None);
    // A placed block's info button opens it too (and doesn't place or drag).
    rig.click("pal:print");
    rig.press(PointerKind::Down, Some("canvas"), 400, 150, proto::Button::Left);
    rig.press(PointerKind::Up, Some("canvas"), 400, 150, proto::Button::Left);
    let id = rig.ed.borrow().graph.blocks[0].id;
    let before = (rig.ed.borrow().graph.blocks[0].x, rig.ed.borrow().graph.blocks[0].y);
    let info = format!("info:{id}");
    rig.press(PointerKind::Down, Some(&info), 0, 0, proto::Button::Left);
    rig.press(PointerKind::Up, Some(&info), 0, 0, proto::Button::Left);
    assert_eq!(rig.ed.borrow().dialog, Dialog::Info("print".into()));
    assert_eq!((rig.ed.borrow().graph.blocks[0].x, rig.ed.borrow().graph.blocks[0].y), before);
    assert!(texts(&rig).iter().any(|t| t.starts_with("Writes a line of text")));
}

/// The palette's scrollbar is dragged like any scroll view's (it sits in a
/// board, which used to take the press), and its track pages.
#[test]
fn the_palette_scrolls_by_dragging_its_thumb_and_clicking_its_track() {
    let mut rig = Rig::new();
    let win = |rig: &Rig| rig.app.core.window(1).unwrap().scrollbar_hit_ids("palette").expect("the palette scrolls");
    let (track, thumb) = win(&rig);
    let r = rig.app.core.window(1).unwrap().rect_of("palette").unwrap();
    let x = r.x + r.w - 2;
    let scroll = |rig: &Rig| rig.app.core.window(1).unwrap().state("palette").map_or(0, |s| s.scroll);
    let msg = |kind: PointerKind, y: i32, hit: u32, buttons: u8| {
        Message::Pointer(proto::Pointer {
            surface: 1,
            kind,
            x,
            y,
            buttons,
            button: if matches!(kind, PointerKind::Down | PointerKind::Up) { proto::Button::Left } else { proto::Button::None },
            scroll_x: 0,
            scroll_y: 0,
            hit_id: hit,
        })
    };
    rig.send(vec![msg(PointerKind::Down, r.y + 4, thumb, proto::buttons::LEFT)]);
    rig.send(vec![msg(PointerKind::Move, r.y + 60, thumb, proto::buttons::LEFT)]);
    rig.send(vec![msg(PointerKind::Up, r.y + 60, thumb, 0)]);
    let dragged = scroll(&rig);
    assert!(dragged > 60, "pulling the thumb down 56 lu scrolls the palette: {dragged}");
    // Back to the top, then a click low on the track pages down.
    rig.send(vec![msg(PointerKind::Down, r.y + 60, thumb, proto::buttons::LEFT)]);
    rig.send(vec![msg(PointerKind::Move, r.y - 400, 0, proto::buttons::LEFT)]);
    rig.send(vec![msg(PointerKind::Up, r.y - 400, 0, 0)]);
    assert_eq!(scroll(&rig), 0);
    let (track2, _) = win(&rig);
    assert_eq!(track2, track);
    rig.send(vec![msg(PointerKind::Down, r.y + r.h - 3, track, proto::buttons::LEFT)]);
    rig.send(vec![msg(PointerKind::Up, r.y + r.h - 3, track, 0)]);
    assert!(scroll(&rig) > r.h / 2, "a page: {}", scroll(&rig));
}

/// Every text the app has on screen now.
fn texts(rig: &Rig) -> Vec<String> {
    let m = rig.model.borrow();
    let mut v: Vec<(u32, String)> = m.iter().filter_map(|(id, (_, t))| t.clone().map(|t| (*id, t))).collect();
    v.sort();
    v.into_iter().map(|(_, t)| t).collect()
}

/// Writes the scenes of a few editor states as TreeOps JSON (one message
/// per line) to $SCENE_DIR, for looking at them with a renderer.
#[test]
#[ignore]
fn dump_scenes() {
    let dir = std::env::var("SCENE_DIR").expect("SCENE_DIR");
    let shots: Vec<(&str, Box<dyn Fn(&mut Editor)>)> = vec![
        ("hello", Box::new(|ed: &mut Editor| {
            let t = std::fs::read(manifest().join("tests/vpl/math_constants.vpl")).unwrap();
            ed.files.write("visual/m.vpl", &t).unwrap();
            ed.open("visual/m.vpl").unwrap();
            ed.graph.offset = (20.0, 10.0);
        })),
        ("counter", Box::new(|ed: &mut Editor| {
            build_counter(ed);
            ed.graph.offset = (10.0, 10.0);
            ed.selected = Some(1);
        })),
        ("design", Box::new(|ed: &mut Editor| {
            build_counter(ed);
            ed.show_design();
            ed.design_sel = Some(vec![1]);
            ed.insp_rev += 1;
        })),
        ("run", Box::new(|ed: &mut Editor| {
            build_counter(ed);
            ed.run = Some(ecm_visual::editor::RunPanel { title: "python /visual/counter.py".into(), text: "1\n2\nhello\n".into(), running: true });
        })),
    ];
    for (name, f) in shots {
        let rig = Rig::new();
        f(&mut rig.ed.borrow_mut());
        let conn = MockConnection::new();
        conn.state.borrow_mut().idle_timeouts = true;
        conn.push(&[welcome(false)]);
        let mut app = App::with_connection(ecm_visual::APP_ID, Box::new(conn.clone())).unwrap();
        view::install(&mut app, rig.ed.clone());
        conn.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w: 800, h: 500, states: proto::window_states::ACTIVATED })]);
        app.step(0);
        app.step(0);
        let lines: Vec<String> = conn.sent_messages().iter().filter(|m| matches!(m, Message::TreeOps(_))).map(|m| m.to_json()).collect();
        std::fs::write(format!("{dir}/{name}.jsonl"), lines.join("\n")).unwrap();
    }
}
