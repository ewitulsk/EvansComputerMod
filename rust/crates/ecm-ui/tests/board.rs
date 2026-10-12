//! Boards, panels and wires (the Visual Programmer's node canvas and
//! designer): absolute placement, line nodes, pointer grabs with board
//! coordinates and target ids, capture mode, keys, and layout-file props.

use ecm_ui::prelude::*;
use ecm_ui::samples::{click, welcome};
use ecm_ui::{MockConnection, UiEvent};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Message, NodeKind, Point, PointerKind, Prop, TreeOp};
use std::cell::RefCell;
use std::rc::Rc;

struct Rig {
    conn: MockConnection,
    app: App,
}

impl Rig {
    fn new(view: impl FnMut(&ViewCtx) -> Element + 'static) -> Rig {
        let conn = MockConnection::new();
        conn.state.borrow_mut().idle_timeouts = true;
        conn.push(&[welcome(false)]);
        let mut app = App::with_connection("dev.test.board", Box::new(conn.clone())).unwrap();
        app.window("Board", view);
        let mut rig = Rig { conn, app };
        rig.send(vec![Message::Configure(proto::Configure {
            surface: 1,
            serial: 1,
            w: 400,
            h: 300,
            states: proto::window_states::ACTIVATED,
        })]);
        rig
    }

    fn send(&mut self, msgs: Vec<Message>) -> Vec<TreeOp> {
        self.conn.clear_sent();
        self.conn.push(&msgs);
        self.app.step(0);
        self.app.step(0);
        self.conn
            .sent_messages()
            .into_iter()
            .filter_map(|m| if let Message::TreeOps(t) = m { Some(t.ops) } else { None })
            .flatten()
            .collect()
    }

    fn hit(&self, id: &str) -> u32 {
        self.app.core.window(1).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no hit id for {id}"))
    }
}

fn pointer(kind: PointerKind, hit_id: u32, x: i32, y: i32, buttons: u8) -> Message {
    Message::Pointer(proto::Pointer {
        surface: 1,
        kind,
        x,
        y,
        buttons,
        button: if matches!(kind, PointerKind::Down | PointerKind::Up) { proto::Button::Left } else { proto::Button::None },
        scroll_x: 0,
        scroll_y: if kind == PointerKind::Scroll { -2 } else { 0 },
        hit_id,
    })
}

type Log = Rc<RefCell<Vec<UiEvent>>>;

fn recorder(log: &Log) -> impl Fn(&mut Ctx, &UiEvent) + 'static {
    let log = log.clone();
    move |_, e| log.borrow_mut().push(e.clone())
}

fn pointers(log: &Log) -> Vec<(PointerKind, i32, i32, Option<String>)> {
    log.borrow()
        .iter()
        .filter_map(|e| match &e.value {
            EventValue::Pointer(p) => Some((p.kind, p.x, p.y, p.target.clone())),
            _ => None,
        })
        .collect()
}

fn created(ops: &[TreeOp], kind: NodeKind) -> Vec<Vec<Prop>> {
    ops.iter()
        .filter_map(|o| match o {
            TreeOp::Create { kind: k, props, .. } if *k == kind => Some(props.clone()),
            _ => None,
        })
        .collect()
}

fn graph(log: &Log) -> Element {
    // A toolbar above a board with two panels, a wire and a text field.
    column()
        .child(row().padding(4).child(button("Run").id("run")))
        .child(
            board()
                .id("canvas")
                .fill(0x101010ff)
                .on_pointer(recorder(log))
                .on_key(recorder(log))
                .child(wire([(60, 40), (100, 40), (140, 90)]).id("w1").stroke(2).color(0xffffffff))
                .child(panel().id("node:1").at(20, 20).sized(80, 40).fill(0x9c27b0ff).child(label("Start")))
                .child(panel().id("port:1:flow").at(56, 56).sized(8, 8).fill(0xffffffff))
                .child(text_field().id("val").at(150, 100).width(60)),
        )
}

#[test]
fn boards_place_children_and_draw_wires_as_lines() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let mut rig = Rig::new(move |_| graph(&l));
    let ops = rig.send(vec![]);
    assert!(ops.is_empty(), "nothing changed");
    // Re-render from scratch to inspect the created nodes.
    let conn = MockConnection::new();
    conn.state.borrow_mut().idle_timeouts = true;
    conn.push(&[welcome(false)]);
    let mut app = App::with_connection("dev.test.board", Box::new(conn.clone())).unwrap();
    let l = log.clone();
    app.window("Board", move |_| graph(&l));
    conn.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w: 400, h: 300, states: 0 })]);
    app.step(0);
    app.step(0);
    let all: Vec<TreeOp> = conn
        .sent_messages()
        .into_iter()
        .filter_map(|m| if let Message::TreeOps(t) = m { Some(t.ops) } else { None })
        .flatten()
        .collect();
    // The wire is one line node, its points relative to the board group.
    let lines = created(&all, NodeKind::Line);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains(&Prop::Points(vec![Point { x: 60, y: 40 }, Point { x: 100, y: 40 }, Point { x: 140, y: 90 }])));
    assert!(lines[0].contains(&Prop::Width(2)));
    // The board is a clipping group; the panel a group at its board position.
    let groups = created(&all, NodeKind::Group);
    assert!(groups.iter().any(|g| g.contains(&Prop::Clip(true)) && g.contains(&Prop::W(400))), "{groups:?}");
    assert!(groups.iter().any(|g| g.contains(&Prop::X(20)) && g.contains(&Prop::Y(20)) && g.contains(&Prop::W(80))));
    // Panels have a filled, hit-testable background.
    let rects = created(&all, NodeKind::Rect);
    assert!(rects.iter().any(|r| r.contains(&Prop::Fill(0x9c27b0ff)) && r.iter().any(|p| matches!(p, Prop::HitId(h) if *h > 0))));
}

#[test]
fn a_press_on_a_board_grabs_the_pointer_and_reports_board_coordinates() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let mut rig = Rig::new(move |_| graph(&l));
    // The board sits below the toolbar: find its origin from a press on its background.
    let bg = rig.hit("canvas");
    let port = rig.hit("port:1:flow");
    let node = rig.hit("node:1");
    rig.send(vec![pointer(PointerKind::Down, port, 60, 100, proto::buttons::LEFT)]);
    let p = pointers(&log);
    assert_eq!(p.len(), 1);
    let (kind, x, y, target) = p[0].clone();
    assert_eq!((kind, target.as_deref()), (PointerKind::Down, Some("port:1:flow")));
    let top = 100 - y; // the board's y in the window
    assert_eq!(x, 60);
    assert!(top > 0, "the board is below the toolbar");
    // Moves follow the grab even off the board's nodes (hit 0) ...
    rig.send(vec![pointer(PointerKind::Move, 0, 200, 150, proto::buttons::LEFT)]);
    // ... and the release names what it landed on.
    rig.send(vec![pointer(PointerKind::Up, node, 30, top + 30, 0)]);
    let p = pointers(&log);
    assert_eq!(p[1], (PointerKind::Move, 200, 150 - top, None));
    assert_eq!(p[2], (PointerKind::Up, 30, 30, Some("node:1".to_string())));
    // A label inside a panel reports the panel (the innermost element with an id).
    log.borrow_mut().clear();
    rig.send(vec![pointer(PointerKind::Down, bg, 300, top + 200, proto::buttons::LEFT)]);
    rig.send(vec![pointer(PointerKind::Up, bg, 300, top + 200, 0)]);
    assert_eq!(pointers(&log)[0], (PointerKind::Down, 300, 200, None));
    // Wheel steps over the board go to it.
    log.borrow_mut().clear();
    rig.send(vec![pointer(PointerKind::Scroll, node, 40, top + 40, 0)]);
    let ev = log.borrow()[0].clone();
    match ev.value {
        EventValue::Pointer(p) => assert_eq!((p.kind, p.scroll_y, p.target.as_deref()), (PointerKind::Scroll, -2, Some("node:1"))),
        other => panic!("{other:?}"),
    }
}

#[test]
fn interactive_children_keep_their_presses_and_keys_go_to_a_focused_board() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let mut rig = Rig::new(move |_| graph(&l));
    // A text field inside the board edits as usual.
    let field = rig.hit("val");
    rig.send(click(field, 160, 120));
    rig.send(vec![Message::Text(proto::Text { surface: 1, text: "42".into() })]);
    assert!(pointers(&log).is_empty());
    assert_eq!(rig.app.core.window(1).unwrap().state("val").unwrap().text, "42");
    // Pressing the board focuses it; keys then reach on_key.
    let bg = rig.hit("canvas");
    rig.send(vec![pointer(PointerKind::Down, bg, 10, 200, 1), pointer(PointerKind::Up, bg, 10, 200, 0)]);
    rig.send(vec![Message::Key(proto::Key { surface: 1, key: 0x4C, mods: proto::mods::SHIFT, down: true, repeat: false })]);
    let keys: Vec<EventValue> = log.borrow().iter().filter(|e| e.kind == EventKind::Key).map(|e| e.value.clone()).collect();
    assert_eq!(keys, vec![EventValue::Key { key: 0x4C, mods: proto::mods::SHIFT }]);
}

#[test]
fn a_capturing_board_turns_every_press_into_a_selection() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let clicks = Rc::new(RefCell::new(0));
    let (l, c) = (log.clone(), clicks.clone());
    let mut rig = Rig::new(move |_| {
        let c = c.clone();
        board().id("design").capture(true).on_pointer(recorder(&l)).child(
            column()
                .id("root")
                .padding(8)
                .child(label("Count: 0").id("count"))
                .child(button("Add").id("add").on_click(move |_, _| *c.borrow_mut() += 1)),
        )
    });
    // Labels and containers become pointable inside a capturing board.
    let label_hit = rig.hit("count");
    let button_hit = rig.hit("add");
    rig.send(click(button_hit, 20, 30));
    rig.send(click(label_hit, 20, 10));
    assert_eq!(*clicks.borrow(), 0, "the button did not fire");
    let targets: Vec<Option<String>> = pointers(&log).into_iter().map(|p| p.3).collect();
    assert_eq!(targets, vec![Some("add".into()), Some("add".into()), Some("count".into()), Some("count".into())]);
}

#[test]
fn layout_files_take_panels_boards_and_wires() {
    let f = ecm_ui::layout_file::load(
        r##"{ "window": "Shapes", "root": { "board": { "id": "b", "capture": true, "fill": "#101010", "on_pointer": "pt", "on_key": "k",
            "children": [
              { "panel": { "id": "p", "x": 10, "y": 20, "width": 50, "height": 30, "fill": 4278190335, "radius": 3,
                           "border_w": 1, "border_color": "#ffffff", "padding": 4, "children": [ { "label": { "text": "hi" } } ] } },
              { "wire": { "points": [[0, 0], [10, 5]], "stroke": 2, "color": "#ff0000" } } ] } } }"##,
    )
    .unwrap();
    let b = &f.root;
    assert_eq!(b.kind, ecm_ui::Kind::Board);
    assert_eq!((b.props.capture, b.props.fill), (Some(true), Some(0x101010ff)));
    assert!(matches!(b.handler(EventKind::Pointer), Some(Handler::Named(n)) if n == "pt"));
    assert!(matches!(b.handler(EventKind::Key), Some(Handler::Named(n)) if n == "k"));
    let p = &b.children[0];
    assert_eq!((p.props.x, p.props.y, p.props.radius, p.props.border_w, p.props.border_color), (Some(10), Some(20), Some(3), Some(1), Some(0xffffffff)));
    let w = &b.children[1];
    assert_eq!((w.props.points.clone(), w.props.stroke), (vec![[0, 0], [10, 5]], Some(2)));
    assert!(ecm_ui::layout_file::load(r#"{ "root": { "wire": { "points": [[1]] } } }"#).is_err());
}
