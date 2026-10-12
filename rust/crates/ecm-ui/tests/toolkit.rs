//! ecm-ui against a mock compositor: layout at the three size classes, text
//! wrapping, diff minimality, hit routing, focus, `.ui.json` loading and the
//! app lifecycle. A small reference tree (`Model`) applies every TreeOps the
//! toolkit sends and checks it is consistent, the way the compositor will.

use ecm_ui::prelude::*;
use ecm_ui::samples::{click, welcome};
use ecm_ui::{EventValue, MockConnection, Theme};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Message, NodeKind, Prop, TreeOp};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// ------------------------------------------------------------------ reference tree

#[derive(Clone, Debug)]
struct Node {
    kind: NodeKind,
    parent: u32,
    props: Vec<Prop>,
    children: Vec<u32>,
}

#[derive(Default)]
struct Model {
    nodes: HashMap<u32, Node>,
    root: Vec<u32>,
}

impl Model {
    fn children_mut(&mut self, parent: u32) -> &mut Vec<u32> {
        if parent == 0 {
            &mut self.root
        } else {
            &mut self.nodes.get_mut(&parent).expect("parent exists").children
        }
    }

    fn remove_subtree(&mut self, id: u32) {
        let n = self.nodes.remove(&id).expect("removed node exists");
        for c in n.children {
            self.remove_subtree(c);
        }
    }

    fn apply(&mut self, ops: &[TreeOp]) {
        for op in ops {
            match op {
                TreeOp::Create { id, parent, index, kind, props } => {
                    assert!(!self.nodes.contains_key(id), "id {id} reused");
                    if *parent != 0 {
                        assert_eq!(self.nodes[parent].kind, NodeKind::Group, "only groups have children");
                    }
                    let v = self.children_mut(*parent);
                    let i = (*index as usize).min(v.len());
                    assert!(*index as usize <= v.len() || *index == u32::MAX, "index past the end");
                    v.insert(i, *id);
                    self.nodes.insert(*id, Node { kind: *kind, parent: *parent, props: props.clone(), children: vec![] });
                }
                TreeOp::Set { id, props } => {
                    let n = self.nodes.get_mut(id).expect("Set on a missing node");
                    for p in props {
                        n.props.retain(|q| q.tag() != p.tag());
                        n.props.push(p.clone());
                    }
                }
                TreeOp::Move { id, parent, index } => {
                    let old = self.nodes[id].parent;
                    self.children_mut(old).retain(|x| x != id);
                    let v = self.children_mut(*parent);
                    assert!(*index as usize <= v.len(), "move index past the end");
                    v.insert(*index as usize, *id);
                    self.nodes.get_mut(id).unwrap().parent = *parent;
                }
                TreeOp::Remove { id } => {
                    let p = self.nodes[id].parent;
                    self.children_mut(p).retain(|x| x != id);
                    self.remove_subtree(*id);
                }
            }
        }
    }

    fn prop<T>(&self, id: u32, f: impl Fn(&Prop) -> Option<T>) -> Option<T> {
        self.nodes[&id].props.iter().find_map(f)
    }

    fn xy(&self, id: u32) -> (i32, i32) {
        let x = self.prop(id, |p| if let Prop::X(v) = p { Some(*v) } else { None }).unwrap_or(0);
        let y = self.prop(id, |p| if let Prop::Y(v) = p { Some(*v) } else { None }).unwrap_or(0);
        (x, y)
    }

    /// Absolute position (sum of the group offsets above).
    fn abs(&self, id: u32) -> (i32, i32) {
        let (mut x, mut y) = self.xy(id);
        let mut p = self.nodes[&id].parent;
        while p != 0 {
            let (px, py) = self.xy(p);
            x += px;
            y += py;
            p = self.nodes[&p].parent;
        }
        (x, y)
    }

    fn texts(&self) -> Vec<(u32, String)> {
        let mut v: Vec<(u32, String)> = self
            .nodes
            .iter()
            .filter_map(|(id, n)| n.props.iter().find_map(|p| if let Prop::Text(s) = p { Some((*id, s.clone())) } else { None }))
            .collect();
        v.sort();
        v
    }

    fn text_id(&self, s: &str) -> Option<u32> {
        self.texts().into_iter().find(|(_, t)| t == s).map(|(id, _)| id)
    }
}

// ------------------------------------------------------------------ harness

struct Rig {
    conn: MockConnection,
    app: App,
    model: Model,
}

impl Rig {
    fn new(view: impl FnMut(&ViewCtx) -> Element + 'static) -> Rig {
        let conn = MockConnection::new();
        conn.state.borrow_mut().idle_timeouts = true;
        conn.push(&[welcome(false)]);
        let mut app = App::with_connection("dev.test.app", Box::new(conn.clone())).unwrap();
        app.window("Test", view);
        Rig { conn, app, model: Model::default() }
    }

    fn configure(&mut self, w: i32, h: i32) {
        self.send(vec![Message::Configure(proto::Configure { surface: 1, serial: 1, w, h, states: proto::window_states::ACTIVATED })]);
    }

    /// Deliver messages, run the loop until they are handled and rendered,
    /// and return what the app sent meanwhile.
    fn send(&mut self, msgs: Vec<Message>) -> Vec<Message> {
        self.conn.clear_sent();
        self.conn.push(&msgs);
        self.app.step(0); // render pending, then receive and dispatch
        self.app.step(0); // render the result
        let sent = self.conn.sent_messages();
        for m in &sent {
            if let Message::TreeOps(t) = m {
                self.model.apply(&t.ops);
            }
        }
        sent
    }

    /// Render after outside state changed.
    fn render(&mut self) -> Vec<Message> {
        self.send(vec![])
    }

    fn hit(&self, id: &str) -> u32 {
        self.app.core.window(1).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no hit id for {id}"))
    }

    fn click(&mut self, id: &str) -> Vec<Message> {
        let h = self.hit(id);
        self.send(click(h, 0, 0))
    }
}

fn ops(sent: &[Message]) -> Vec<TreeOp> {
    sent.iter().filter_map(|m| if let Message::TreeOps(t) = m { Some(t.ops.clone()) } else { None }).flatten().collect()
}

fn log() -> Rc<RefCell<Vec<String>>> {
    Rc::new(RefCell::new(Vec::new()))
}

// ------------------------------------------------------------------ tests

#[test]
fn layout_switches_at_the_three_size_classes() {
    let mut rig = Rig::new(|cx| {
        let tiles = (0..6).map(|i| button(format!("Tile {i}")));
        column()
            .padding(8)
            .child(responsive(label("compact layout"), label("medium layout"), label("wide layout")))
            .child(grid(1).columns(1, 2, 3).children(tiles))
            .child(label(format!("{:?}", cx.class)))
    });
    let columns_at = |rig: &Rig| {
        let mut xs: Vec<i32> = (0..6).map(|i| rig.model.abs(rig.model.text_id(&format!("Tile {i}")).unwrap()).0).collect();
        xs.sort();
        xs.dedup();
        xs.len()
    };
    rig.configure(320, 200);
    assert!(rig.model.text_id("compact layout").is_some());
    assert!(rig.model.text_id("Compact").is_some());
    assert_eq!(columns_at(&rig), 1);
    rig.configure(640, 400);
    assert!(rig.model.text_id("medium layout").is_some() && rig.model.text_id("compact layout").is_none());
    assert_eq!(columns_at(&rig), 2);
    rig.configure(1000, 600);
    assert!(rig.model.text_id("wide layout").is_some());
    assert_eq!(columns_at(&rig), 3);
    // Everything stays inside the window.
    for (id, s) in rig.model.texts() {
        let (x, _) = rig.model.abs(id);
        let f = proto::fonts::face(proto::Font::Ui, 12).unwrap();
        assert!(x >= 0 && x + f.text_width(&s) <= 1000, "{s:?} at {x}");
    }
}

#[test]
fn labels_wrap_to_the_window_width() {
    let text = "The quick brown fox jumps over the lazy dog and keeps on running";
    let mut rig = Rig::new(move |_| column().padding(10).child(label(text)));
    rig.configure(120, 200);
    let face = proto::fonts::face(proto::Font::Ui, 12).unwrap();
    let lines: Vec<(u32, String)> = rig.model.texts();
    assert!(lines.len() >= 4, "{lines:?}");
    let mut ys = Vec::new();
    for (id, s) in &lines {
        assert!(face.text_width(s) <= 100, "{s:?} is wider than the 100 lu column");
        ys.push(rig.model.abs(*id).1);
    }
    ys.sort();
    for w in ys.windows(2) {
        assert_eq!(w[1] - w[0], 12, "one text node per 12 lu line");
    }
    assert_eq!(lines.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>().join(" "), text);
    // Wider: fewer lines; the surplus line nodes are removed.
    let sent = rig.send(vec![Message::Configure(proto::Configure { surface: 1, serial: 2, w: 600, h: 200, states: 0 })]);
    assert!(ops(&sent).iter().any(|o| matches!(o, TreeOp::Remove { .. })));
    assert_eq!(rig.model.texts().len(), 1);
}

#[test]
fn changing_one_label_emits_one_set() {
    let title = Rc::new(RefCell::new(String::from("Inbox")));
    let t = title.clone();
    let mut rig = Rig::new(move |_| {
        column().gap(8).padding(12).child(label(t.borrow().clone())).child(label("Second")).child(button("OK")).child(text_field())
    });
    rig.configure(400, 300);
    let before = rig.model.nodes.len();
    *title.borrow_mut() = String::from("Outbox");
    let sent = rig.render();
    let id = rig.model.text_id("Outbox").unwrap();
    assert_eq!(ops(&sent), vec![TreeOp::Set { id, props: vec![Prop::Text("Outbox".into())] }]);
    assert!(matches!(sent.last(), Some(Message::Commit(_))));
    assert_eq!(rig.model.nodes.len(), before);
    // Nothing changed: nothing sent at all.
    assert!(rig.render().is_empty());
}

#[test]
fn hits_route_to_widgets() {
    let events = log();
    let ev = events.clone();
    let rec = move |name: &'static str| {
        let ev = ev.clone();
        move |_: &mut Ctx, e: &UiEvent| ev.borrow_mut().push(format!("{name}:{:?}", e.value))
    };
    let mut rig = Rig::new(move |_| {
        column()
            .gap(4)
            .child(button("Go").id("go").on_click(rec("click")))
            .child(button("Other").id("other"))
            .child(checkbox("Agree").id("agree").on_change(rec("check")))
            .child(toggle("Wi-Fi").id("wifi").on_change(rec("toggle")))
            .child(text_field().id("name").placeholder("Name").on_change(rec("edit")).on_submit(rec("submit")))
            .child(slider(0, 10).id("vol").width(110).on_change(rec("slide")))
            .child(list().id("items").children([label("one"), label("two"), label("three")]).on_select(rec("pick")))
            .child(menu(["Cut", "Copy"]).id("menu").on_select(rec("menu")))
    });
    rig.configure(400, 400);

    rig.click("go");
    // Down on one widget, up on another: no click.
    let (a, b) = (rig.hit("go"), rig.hit("other"));
    rig.send(vec![click(a, 0, 0)[0].clone(), click(b, 0, 0)[1].clone()]);
    rig.click("agree");
    assert!(rig.model.nodes.values().any(|n| n.kind == NodeKind::Line), "checked box draws its tick");
    rig.click("wifi");
    rig.click("name");
    rig.send(vec![Message::Text(proto::Text { surface: 1, text: "Bob".into() })]);
    assert!(rig.model.text_id("Bob").is_some());
    rig.send(vec![key(0x2A)]); // backspace
    rig.send(vec![key(0x28)]); // enter
    // Slider: press at the far right.
    let vol = rig.hit("vol");
    let (sx, _) = rig.model.abs(rig.model.nodes.iter().find(|(_, n)| n.props.contains(&Prop::HitId(vol))).map(|(id, _)| *id).unwrap());
    rig.send(vec![click(vol, sx + 200, 0)[0].clone()]);
    rig.send(vec![click(vol, sx + 5, 0)[1].clone()]);
    // List row 2 and menu item 2: each row has its own hit id, in order.
    let items_first = rig.hit("items");
    rig.send(click(items_first + 1, 0, 0));
    let menu_first = rig.hit("menu");
    rig.send(click(menu_first + 1, 0, 0));

    assert_eq!(
        *events.borrow(),
        vec![
            "click:None",
            "check:Bool(true)",
            "toggle:Bool(true)",
            "edit:Text(\"Bob\")",
            "edit:Text(\"Bo\")",
            "submit:Text(\"Bo\")",
            "slide:Int(10)",
            "pick:Index(1)",
            "menu:Index(1)",
        ]
    );
}

fn key(code: u16) -> Message {
    Message::Key(proto::Key { surface: 1, key: code, mods: 0, down: true, repeat: false })
}

#[test]
fn focus_moves_with_tab_and_keys_drive_widgets() {
    let events = log();
    let ev = events.clone();
    let mut rig = Rig::new(move |_| {
        let ev1 = ev.clone();
        let ev2 = ev.clone();
        column()
            .child(text_field().id("a"))
            .child(button("B").id("b").on_click(move |_, _| ev1.borrow_mut().push("b".into())))
            .child(tabs(["One", "Two"]).id("t").child(label("page one")).child(label("page two")).on_select(move |_, e| {
                ev2.borrow_mut().push(format!("tab {:?}", e.value))
            }))
    });
    rig.configure(400, 300);
    rig.send(vec![key(0x2B)]); // Tab -> a
    assert_eq!(rig.app.core.window(1).unwrap().focused(), Some("#a"));
    // The focused field shows a focus border and a caret.
    let caret = rig.model.nodes.values().filter(|n| n.kind == NodeKind::Rect && n.props.contains(&Prop::W(1))).count();
    assert_eq!(caret, 1);
    rig.send(vec![key(0x2B)]); // -> b
    rig.send(vec![key(0x2C)]); // space clicks
    rig.send(vec![key(0x2B)]); // -> tabs
    rig.send(vec![key(0x4F)]); // right: tab two
    assert!(rig.model.text_id("page two").is_some() && rig.model.text_id("page one").is_none());
    rig.send(vec![Message::Key(proto::Key { surface: 1, key: 0x2B, mods: proto::mods::SHIFT, down: true, repeat: false })]);
    assert_eq!(rig.app.core.window(1).unwrap().focused(), Some("#b"));
    assert_eq!(*events.borrow(), vec!["b", "tab Index(1)"]);
    // Clicking empty space clears focus.
    rig.send(click(0, 5, 5));
    assert_eq!(rig.app.core.window(1).unwrap().focused(), None);
}

#[test]
fn scroll_views_scroll_and_dialogs_block_clicks() {
    let clicked = log();
    let c = clicked.clone();
    let show_dialog = Rc::new(RefCell::new(false));
    let d = show_dialog.clone();
    let mut rig = Rig::new(move |_| {
        let c = c.clone();
        let content = column().children((0..30).map(|i| label(format!("Row {i}"))));
        let mut s = stack()
            .child(column().child(button("Behind").id("behind").on_click(move |_, _| c.borrow_mut().push("behind".into()))).child(scroll_view(content).id("scroll").height(100)));
        if *d.borrow() {
            s = s.child(dialog("Really?").id("dlg").child(label("This cannot be undone.")).child(button("OK").id("ok")));
        }
        s
    });
    rig.configure(300, 200);
    let row0 = rig.model.text_id("Row 0").unwrap();
    let y0 = rig.model.abs(row0).1;
    let h = rig.hit("scroll");
    let scroll = |dy: i16| {
        Message::Pointer(proto::Pointer {
            surface: 1,
            kind: proto::PointerKind::Scroll,
            x: 10,
            y: 50,
            buttons: 0,
            button: proto::Button::None,
            scroll_x: 0,
            scroll_y: dy,
            hit_id: h,
        })
    };
    rig.send(vec![scroll(2)]);
    assert_eq!(rig.model.abs(row0).1, y0 - 32, "two wheel steps of 16 lu");
    rig.send(vec![scroll(100)]);
    let last = rig.model.text_id("Row 29").unwrap();
    let bottom = rig.model.abs(last).1 + 12;
    let scroll_node = rig.model.nodes.iter().find(|(_, n)| n.props.contains(&Prop::HitId(h))).map(|(id, _)| *id).unwrap();
    let view_bottom = rig.model.abs(scroll_node).1 + 100;
    assert_eq!(bottom, view_bottom, "scrolling stops at the end of the content");
    rig.send(vec![scroll(-1000)]);
    assert_eq!(rig.model.abs(row0).1, y0);

    *show_dialog.borrow_mut() = true;
    rig.render();
    assert!(rig.model.text_id("Really?").is_some());
    let scrim = rig.app.core.window(1).unwrap().hit_id_of("dlg");
    assert!(scrim.is_none(), "the scrim is not the dialog's main part");
    // The scrim lies over "Behind"; clicking it does nothing.
    let scrim_hit = rig.model.nodes.values().filter_map(|n| n.props.iter().find_map(|p| if let Prop::HitId(h) = p { Some(*h) } else { None })).max().unwrap();
    rig.send(click(scrim_hit, 10, 10));
    assert!(clicked.borrow().is_empty());
}

const NOTES_LAYOUT: &str = r#"
{ "window": "Notes",
  "root": { "column": { "gap": 8, "padding": 12, "children": [
    { "row": { "gap": 8, "children": [
      { "textfield": { "id": "entry", "placeholder": "New note" } },
      { "button": { "text": "Add", "on_click": "add_note" } } ] } },
    { "list": { "id": "rows" } } ] } } }
"#;

#[test]
fn ui_json_layouts_load_with_named_handlers() {
    let layout = ecm_ui::layout_file::load(NOTES_LAYOUT).unwrap();
    assert_eq!(layout.title, "Notes");
    assert_eq!(layout.root.kind, ecm_ui::Kind::Column);
    assert_eq!(layout.root.children[0].children[1].handler(EventKind::Click).map(|h| format!("{h:?}")), Some("Named(\"add_note\")".into()));

    let conn = MockConnection::new();
    conn.state.borrow_mut().idle_timeouts = true;
    conn.push(&[welcome(false)]);
    let mut app = App::with_connection("dev.test.notes", Box::new(conn.clone())).unwrap();
    let win = app.window_from_layout(NOTES_LAYOUT).unwrap();
    let added = log();
    let a = added.clone();
    app.handler("add_note", move |cx, _| {
        let t = cx.take_text("entry");
        a.borrow_mut().push(t);
    });
    let mut rig = Rig { conn, app, model: Model::default() };
    rig.configure(400, 300);
    assert!(rig.model.text_id("New note").is_some());
    rig.click("entry");
    rig.send(vec![Message::Text(proto::Text { surface: 1, text: "milk".into() })]);
    let add = rig.model.nodes.iter().find(|(_, n)| n.props.contains(&Prop::Text("Add".into()))).map(|(id, _)| *id).unwrap();
    // The button's background is the sibling just before its label.
    let parent = rig.model.nodes[&add].parent;
    let siblings = rig.model.nodes[&parent].children.clone();
    let bg = siblings[siblings.iter().position(|x| *x == add).unwrap() - 1];
    let hit = rig.model.prop(bg, |p| if let Prop::HitId(h) = p { Some(*h) } else { None }).unwrap();
    rig.send(click(hit, 0, 0));
    assert_eq!(*added.borrow(), vec!["milk"]);
    assert!(rig.model.text_id("New note").is_some(), "the field was emptied");
    let _ = win;

    // Errors name the problem.
    let err = ecm_ui::layout_file::load(r#"{"window":"x","root":{"button":{"txt":"a"}}}"#).unwrap_err();
    assert!(err.contains("unknown prop \"txt\""), "{err}");
    let err = ecm_ui::layout_file::load(r#"{"root":{"buton":{}}}"#).unwrap_err();
    assert!(err.contains("unknown widget"), "{err}");
}

#[test]
fn lifecycle_handshake_configure_suspend_restore_theme() {
    let conn = MockConnection::new();
    conn.state.borrow_mut().idle_timeouts = true;
    conn.push(&[welcome(true), Message::Restore(proto::Restore { data: b"{\"n\":3}".to_vec() })]);
    let mut app = App::with_connection("dev.test.life", Box::new(conn.clone())).unwrap();
    assert_eq!(app.restore_json().get("n"), Some(&Json::Int(3)));
    let frames = log();
    let f = frames.clone();
    app.window("Life", |_| column().child(label("hi")).child(button("Go").style("primary")));
    app.hotkey(2, "Go", |cx| cx.request_frame());
    app.on_suspend(|| b"saved".to_vec());
    app.on_frame(move |_, t| f.borrow_mut().push(format!("frame {t}")));
    app.step(0);
    let sent = conn.sent_messages();
    assert!(matches!(&sent[0], Message::Hello(h) if h.version == proto::VERSION && h.role == proto::Role::App));
    assert!(matches!(&sent[1], Message::SurfaceCreate(s) if s.surface == 1 && s.title == "Life" && s.kind == proto::SurfaceKind::Tree));
    assert!(matches!(&sent[2], Message::HotkeyClaim(h) if h.slot == 2 && h.label == "Go"));
    assert_eq!(sent.len(), 3, "nothing is drawn before the first Configure");

    let mut rig = Rig { conn, app, model: Model::default() };
    let sent = rig.send(vec![Message::Configure(proto::Configure { surface: 1, serial: 42, w: 200, h: 100, states: 0 })]);
    let names: Vec<&str> = sent.iter().map(|m| m.name()).collect();
    assert_eq!(names, vec!["TreeOps", "AckConfigure", "Commit"]);
    assert!(matches!(sent[1], Message::AckConfigure(proto::AckConfigure { surface: 1, serial: 42 })));
    // A configure that changes nothing visible still gets acked and committed.
    let sent = rig.send(vec![Message::Configure(proto::Configure { surface: 1, serial: 43, w: 200, h: 100, states: 0 })]);
    assert_eq!(sent.iter().map(|m| m.name()).collect::<Vec<_>>(), vec!["AckConfigure", "Commit"]);

    // Hotkey -> handler -> RequestFrame; Frame -> on_frame.
    let sent = rig.send(vec![Message::Hotkey(proto::Hotkey { slot: 2, down: true })]);
    assert!(sent.iter().any(|m| matches!(m, Message::RequestFrame(proto::RequestFrame { surface: 1 }))));
    rig.send(vec![Message::Frame(proto::Frame { surface: 1, time_ms: 1234 })]);
    assert_eq!(*frames.borrow(), vec!["frame 1234"]);

    // WillSuspend is answered with SaveState before anything else queued after it.
    let sent = rig.send(vec![Message::WillSuspend(proto::WillSuspend { deadline_ms: 500 }), Message::Hotkey(proto::Hotkey { slot: 2, down: true })]);
    assert!(matches!(&sent[0], Message::SaveState(s) if s.data == b"saved"));

    // A new theme restyles live: the background and the primary button change color.
    let mut night = Theme::default();
    night.bg = 0x000000ff;
    night.accent = 0xff8800ff;
    let sent = rig.send(vec![Message::ThemeChanged(proto::ThemeChanged { theme: night.to_wire() })]);
    let fills: Vec<u32> = ops(&sent)
        .iter()
        .filter_map(|o| if let TreeOp::Set { props, .. } = o { props.iter().find_map(|p| if let Prop::Fill(f) = p { Some(*f) } else { None }) } else { None })
        .collect();
    assert!(fills.contains(&0x000000ff) && fills.contains(&0xff8800ff), "{fills:x?}");

    // Closing the last window quits.
    let sent = rig.send(vec![Message::CloseRequested(proto::CloseRequested { surface: 1 })]);
    assert!(sent.iter().any(|m| matches!(m, Message::SurfaceDestroy(_))));
    assert!(rig.app.core.is_closed());
}

#[test]
fn notes_sample_runs_its_session() {
    // Find the hit ids with a dry run, then play the whole session.
    let conn = MockConnection::new();
    for chunk in ecm_ui::samples::notes_session(0, 0).into_iter().take(2) {
        conn.push_bytes(chunk);
    }
    let mut app = ecm_ui::samples::notes(Box::new(conn.clone()));
    app.step(0);
    app.step(0);
    let w = app.core.window(1).unwrap();
    let (field, add) = (w.hit_id_of("entry").unwrap(), w.hit_id_of("/0column/0row/1button").unwrap());

    let conn = MockConnection::new();
    for chunk in ecm_ui::samples::notes_session(field, add) {
        conn.push_bytes(chunk);
    }
    let mut app = ecm_ui::samples::notes(Box::new(conn.clone()));
    app.run();
    let sent = conn.sent_messages();
    let mut model = Model::default();
    for m in &sent {
        if let Message::TreeOps(t) = m {
            model.apply(&t.ops);
        }
    }
    for note in ["first", "milk", "eggs \u{e9}"] {
        assert!(model.text_id(note).is_some(), "{note} is listed");
    }
    let saved = sent.iter().find_map(|m| if let Message::SaveState(s) = m { Some(s.data.clone()) } else { None }).unwrap();
    assert_eq!(String::from_utf8(saved).unwrap(), r#"{"notes":["first","milk","eggs é"]}"#);
    let _ = EventValue::None;
}
