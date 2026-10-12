//! The P3 toolkit additions against a mock compositor: text selection and
//! the clipboard keys in text fields and text areas (with the clipboard
//! round trip through `ClipboardSet` / `ClipboardRequest` / `ClipboardData`),
//! text area editing and scrolling, popup menus as popup surfaces, hover
//! visuals, caret blink on a clock, scrollbar and content drags, Tab focus
//! into and out of text areas, custom areas' raw events, size-class
//! visibility, resource upload helpers and app-level shortcuts.

use ecm_ui::prelude::*;
use ecm_ui::samples::{click, welcome};
use ecm_ui::window::keys;
use ecm_ui::{MockConnection, Theme, TEXT_MIME};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Message, NodeKind, Prop, TreeOp};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// ------------------------------------------------------------------ reference tree (per surface)

#[derive(Clone, Debug)]
struct Node {
    kind: NodeKind,
    parent: u32,
    props: Vec<Prop>,
}

#[derive(Default)]
struct Model {
    nodes: HashMap<u32, Node>,
}

impl Model {
    fn apply(&mut self, ops: &[TreeOp]) {
        for op in ops {
            match op {
                TreeOp::Create { id, parent, kind, props, .. } => {
                    assert!(!self.nodes.contains_key(id), "id {id} reused");
                    self.nodes.insert(*id, Node { kind: *kind, parent: *parent, props: props.clone() });
                }
                TreeOp::Set { id, props } => {
                    let n = self.nodes.get_mut(id).expect("Set on a missing node");
                    for p in props {
                        n.props.retain(|q| q.tag() != p.tag());
                        n.props.push(p.clone());
                    }
                }
                TreeOp::Move { id, parent, .. } => self.nodes.get_mut(id).unwrap().parent = *parent,
                TreeOp::Remove { id } => {
                    let mut gone = vec![*id];
                    while let Some(g) = gone.pop() {
                        self.nodes.remove(&g);
                        gone.extend(self.nodes.iter().filter(|(_, n)| n.parent == g).map(|(k, _)| *k));
                    }
                }
            }
        }
    }

    fn texts(&self) -> Vec<String> {
        let mut v: Vec<(u32, String)> = self
            .nodes
            .iter()
            .filter_map(|(id, n)| n.props.iter().find_map(|p| if let Prop::Text(s) = p { Some((*id, s.clone())) } else { None }))
            .collect();
        v.sort();
        v.into_iter().map(|(_, s)| s).collect()
    }

    fn fills(&self) -> Vec<u32> {
        self.nodes.values().filter_map(|n| n.props.iter().find_map(|p| if let Prop::Fill(c) = p { Some(*c) } else { None })).collect()
    }
}

struct Rig {
    conn: MockConnection,
    app: App,
    models: HashMap<u32, Model>,
}

impl Rig {
    fn new(view: impl FnMut(&ViewCtx) -> Element + 'static) -> Rig {
        let conn = MockConnection::new();
        conn.state.borrow_mut().idle_timeouts = true;
        conn.push(&[welcome(false)]);
        let mut app = App::with_connection("dev.test.app", Box::new(conn.clone())).unwrap();
        app.window("Test", view);
        Rig { conn, app, models: HashMap::new() }
    }

    fn configure(&mut self, w: i32, h: i32) {
        self.send(vec![Message::Configure(proto::Configure { surface: 1, serial: 1, w, h, states: proto::window_states::ACTIVATED })]);
    }

    fn send(&mut self, msgs: Vec<Message>) -> Vec<Message> {
        self.conn.clear_sent();
        self.conn.push(&msgs);
        self.app.step(0);
        self.app.step(0);
        let sent = self.conn.sent_messages();
        for m in &sent {
            if let Message::TreeOps(t) = m {
                self.models.entry(t.surface).or_default().apply(&t.ops);
            }
        }
        sent
    }

    fn model(&self) -> &Model {
        &self.models[&1]
    }

    fn hit(&self, id: &str) -> u32 {
        self.app.core.window(1).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no hit id for {id}"))
    }

    fn state(&self, id: &str) -> ecm_ui::window::WidgetState {
        self.app.core.window(1).unwrap().state(id).cloned().unwrap_or_default()
    }

    fn focused(&self) -> Option<String> {
        self.app.core.window(1).unwrap().focused().map(String::from)
    }
}

fn key(code: u16, mods: u16) -> Message {
    Message::Key(proto::Key { surface: 1, key: code, mods, down: true, repeat: false })
}

fn text(s: &str) -> Message {
    Message::Text(proto::Text { surface: 1, text: s.into() })
}

fn pointer(kind: proto::PointerKind, x: i32, y: i32, hit: u32) -> Message {
    let down = kind == proto::PointerKind::Down;
    Message::Pointer(proto::Pointer {
        surface: 1,
        kind,
        x,
        y,
        buttons: if down || kind == proto::PointerKind::Move { proto::buttons::LEFT } else { 0 },
        button: if matches!(kind, proto::PointerKind::Down | proto::PointerKind::Up) { proto::Button::Left } else { proto::Button::None },
        scroll_x: 0,
        scroll_y: 0,
        hit_id: hit,
    })
}

const CTRL: u16 = proto::mods::CTRL;
const SHIFT: u16 = proto::mods::SHIFT;

// ------------------------------------------------------------------ tests

#[test]
fn text_field_selection_and_clipboard_keys_round_trip() {
    let mut rig = Rig::new(|_| column().padding(8).child(text_field().id("f")).child(text_field().id("g")));
    rig.configure(400, 200);
    rig.send(click(rig.hit("f"), 20, 20));
    rig.send(vec![text("hello world")]);
    // Shift+Left x5 selects "world"; the selection is drawn.
    for _ in 0..5 {
        rig.send(vec![key(keys::LEFT, SHIFT)]);
    }
    assert_eq!(rig.state("f").selected_text(), "world");
    assert!(rig.model().fills().contains(&Theme::default().selection), "selection highlight");
    // Ctrl+C sends ClipboardSet with the selection.
    let sent = rig.send(vec![key(keys::C, CTRL)]);
    let set = sent.iter().find_map(|m| if let Message::ClipboardSet(c) = m { Some(c.clone()) } else { None }).expect("ClipboardSet");
    assert_eq!((set.mime.as_str(), set.data.as_slice()), (TEXT_MIME, b"world".as_slice()));
    // Typing replaces the selection.
    rig.send(vec![text("there")]);
    assert_eq!(rig.state("f").text, "hello there");
    // Ctrl+A then Ctrl+X: cut everything.
    rig.send(vec![key(keys::A, CTRL)]);
    let sent = rig.send(vec![key(keys::X, CTRL)]);
    assert!(sent.iter().any(|m| matches!(m, Message::ClipboardSet(c) if c.data == b"hello there")));
    assert_eq!(rig.state("f").text, "");
    // Ctrl+V in the other field asks for the clipboard; the answer is pasted there.
    rig.send(vec![key(keys::TAB, 0)]);
    assert_eq!(rig.focused().as_deref(), Some("#g"));
    let sent = rig.send(vec![key(keys::V, CTRL)]);
    assert!(sent.iter().any(|m| matches!(m, Message::ClipboardRequest(r) if r.mime == TEXT_MIME)));
    rig.send(vec![Message::ClipboardData(proto::ClipboardData { mime: TEXT_MIME.into(), data: b"pasted\ntext".to_vec() })]);
    assert_eq!(rig.state("g").text, "pasted text", "a one-line field turns newlines into spaces");
    // A refused request (empty mime) changes nothing.
    rig.send(vec![key(keys::V, CTRL)]);
    rig.send(vec![Message::ClipboardData(proto::ClipboardData { mime: String::new(), data: vec![] })]);
    assert_eq!(rig.state("g").text, "pasted text");
    // Ctrl+Left jumps a word; Home/End.
    rig.send(vec![key(keys::LEFT, CTRL)]);
    assert_eq!(rig.state("g").cursor, 7);
    rig.send(vec![key(keys::HOME, 0)]);
    assert_eq!(rig.state("g").cursor, 0);
}

#[test]
fn text_area_edits_lines_scrolls_and_shows_line_numbers() {
    let changes = Rc::new(RefCell::new(0));
    let c = changes.clone();
    let mut rig = Rig::new(move |_| {
        let c = c.clone();
        column().child(text_area().id("t").font(proto::Font::Mono).line_numbers(true).height(100).on_change(move |_, _| *c.borrow_mut() += 1))
    });
    rig.configure(300, 200);
    rig.send(click(rig.hit("t"), 50, 20));
    assert_eq!(rig.focused().as_deref(), Some("#t"));
    rig.send(vec![text("line one"), key(keys::ENTER, 0), text("line two"), key(keys::ENTER, 0), text("three")]);
    assert_eq!(rig.state("t").text, "line one\nline two\nthree");
    assert!(*changes.borrow() >= 3);
    assert!(rig.model().texts().contains(&"line two".to_string()));
    for n in ["1", "2", "3"] {
        assert!(rig.model().texts().contains(&n.to_string()), "line number {n}");
    }
    // Up moves to the same column on the line above; Shift+Home selects to its start.
    rig.send(vec![key(keys::UP, 0)]);
    assert_eq!(ecm_ui::window::line_col(&rig.state("t").text, rig.state("t").cursor), (1, 5));
    rig.send(vec![key(keys::HOME, SHIFT)]);
    assert_eq!(rig.state("t").selected_text(), "line ");
    // Tab inserts spaces in a text area instead of moving focus.
    rig.send(vec![key(keys::TAB, 0)]);
    assert_eq!(rig.focused().as_deref(), Some("#t"));
    assert!(rig.state("t").text.contains("\n    two"));
    // Ctrl+Tab leaves it.
    rig.send(vec![key(keys::TAB, CTRL)]);
    assert_eq!(rig.focused().as_deref(), Some("#t"), "the only focusable widget");
    // Many lines: the view follows the caret and draws only visible lines.
    rig.send(click(rig.hit("t"), 50, 20));
    rig.send(vec![key(keys::END, CTRL)]);
    let many: String = (0..60).map(|i| format!("\nrow {i}")).collect();
    rig.send(vec![key(keys::A, CTRL)]);
    assert_eq!(rig.state("t").selected_text(), rig.state("t").text);
    let sent = rig.send(vec![key(keys::V, CTRL)]);
    assert!(sent.iter().any(|m| matches!(m, Message::ClipboardRequest(_))), "{sent:?}");
    rig.send(vec![Message::ClipboardData(proto::ClipboardData { mime: TEXT_MIME.into(), data: many.into_bytes() })]);
    let st = rig.state("t");
    assert_eq!(st.text.lines().count(), 61);
    assert!(st.scroll > 0, "scrolled to the caret at the end");
    let texts = rig.model().texts();
    assert!(texts.contains(&"row 59".to_string()));
    assert!(!texts.contains(&"row 0".to_string()), "lines above the view aren't drawn");
    // The wheel scrolls back up (3 lines a step) without moving the caret.
    let h = rig.hit("t");
    let cursor = st.cursor;
    rig.send(vec![Message::Pointer(proto::Pointer {
        surface: 1,
        kind: proto::PointerKind::Scroll,
        x: 10,
        y: 10,
        buttons: 0,
        button: proto::Button::None,
        scroll_x: 0,
        scroll_y: -100,
        hit_id: h,
    })]);
    assert_eq!(rig.state("t").scroll, 0);
    assert_eq!(rig.state("t").cursor, cursor);
    assert!(rig.model().texts().contains(&"row 0".to_string()));
}

#[test]
fn popup_menus_are_popup_surfaces_that_close_themselves() {
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let c = chosen.clone();
    let mut rig = Rig::new(move |_| {
        let c = c.clone();
        column().child(button("Menu").id("m").on_click(move |cx, _| {
            let c = c.clone();
            cx.popup_menu(10, 30, ["Copy", "Paste", "Select all"], move |_, i| c.borrow_mut().push(i));
        }))
    });
    rig.configure(300, 200);
    let sent = rig.send(click(rig.hit("m"), 5, 5));
    let create = sent
        .iter()
        .find_map(|m| if let Message::SurfaceCreate(s) = m { Some(s.clone()) } else { None })
        .expect("a SurfaceCreate for the popup");
    assert_eq!((create.role, create.parent, create.x, create.y), (proto::SurfaceRole::Popup, 1, 10, 30));
    assert!(create.w >= 120 && create.h > 0);
    let popup = create.surface;
    // The compositor configures it; the menu is drawn into the popup surface.
    rig.send(vec![Message::Configure(proto::Configure { surface: popup, serial: 1, w: create.w, h: create.h, states: 0 })]);
    assert!(rig.models[&popup].texts().contains(&"Paste".to_string()));
    // Hovering an item highlights it.
    let win = rig.app.core.window(popup).unwrap();
    let paste_hit = win.hit_id_of_row("popup_menu", 1).unwrap();
    let before = rig.models[&popup].fills();
    rig.send(vec![Message::Pointer(proto::Pointer {
        surface: popup,
        kind: proto::PointerKind::Move,
        x: 20,
        y: 25,
        buttons: 0,
        button: proto::Button::None,
        scroll_x: 0,
        scroll_y: 0,
        hit_id: paste_hit,
    })]);
    assert_ne!(rig.models[&popup].fills(), before, "hover highlight");
    // Clicking "Paste" calls the handler and destroys the popup.
    let mut msgs = click(paste_hit, 20, 25);
    for m in &mut msgs {
        if let Message::Pointer(p) = m {
            p.surface = popup;
        }
    }
    let sent = rig.send(msgs);
    assert_eq!(*chosen.borrow(), vec![1]);
    assert!(sent.iter().any(|m| matches!(m, Message::SurfaceDestroy(d) if d.surface == popup)));
    assert!(rig.app.core.window(popup).is_none());
    // A second menu dismissed by a click elsewhere (PopupDone) is destroyed too.
    let sent = rig.send(click(rig.hit("m"), 5, 5));
    let p2 = sent.iter().find_map(|m| if let Message::SurfaceCreate(s) = m { Some(s.surface) } else { None }).unwrap();
    let sent = rig.send(vec![Message::PopupDone(proto::PopupDone { surface: p2 })]);
    assert!(sent.iter().any(|m| matches!(m, Message::SurfaceDestroy(d) if d.surface == p2)));
    assert_eq!(chosen.borrow().len(), 1);
}

#[test]
fn hover_changes_buttons_and_rows() {
    let mut rig = Rig::new(|_| column().child(button("Plain").id("b")).child(button("Go").id("p").style("primary")).child(list().id("l").child(label("a")).child(label("b"))));
    rig.configure(300, 200);
    let t = Theme::default();
    assert!(!rig.model().fills().contains(&t.hover));
    rig.send(vec![pointer(proto::PointerKind::Enter, 5, 5, rig.hit("b"))]);
    assert!(rig.model().fills().contains(&t.hover));
    rig.send(vec![pointer(proto::PointerKind::Leave, 5, 5, 0)]);
    assert!(!rig.model().fills().contains(&t.hover), "back to normal after the pointer leaves");
    let p = rig.hit("p");
    rig.send(vec![Message::Pointer(proto::Pointer { surface: 1, kind: proto::PointerKind::Move, x: 5, y: 40, buttons: 0, button: proto::Button::None, scroll_x: 0, scroll_y: 0, hit_id: p })]);
    assert!(rig.model().fills().contains(&t.accent_hover));
    let row = rig.app.core.window(1).unwrap().hit_id_of_row("l", 1).unwrap();
    rig.send(vec![Message::Pointer(proto::Pointer { surface: 1, kind: proto::PointerKind::Move, x: 5, y: 80, buttons: 0, button: proto::Button::None, scroll_x: 0, scroll_y: 0, hit_id: row })]);
    assert!(rig.model().fills().contains(&t.row_hover));
}

#[test]
fn caret_blinks_on_the_clock_and_typing_shows_it_again() {
    let mut rig = Rig::new(|_| column().child(text_field().id("f")));
    rig.conn.state.borrow_mut().now_ms = Some(1000);
    rig.configure(300, 100);
    rig.send(click(rig.hit("f"), 10, 10));
    let caret = |rig: &Rig| rig.model().nodes.values().any(|n| n.kind == NodeKind::Rect && n.props.contains(&Prop::W(1)));
    assert!(caret(&rig));
    rig.conn.state.borrow_mut().now_ms = Some(1000 + 600);
    rig.send(vec![]);
    assert!(!caret(&rig), "off after one blink period");
    rig.conn.state.borrow_mut().now_ms = Some(1000 + 1100);
    rig.send(vec![]);
    assert!(caret(&rig), "on again");
    rig.conn.state.borrow_mut().now_ms = Some(1000 + 1700);
    rig.send(vec![]);
    assert!(!caret(&rig));
    rig.send(vec![text("x")]);
    assert!(caret(&rig), "typing shows the caret at once");
}

#[test]
fn scroll_views_scroll_by_dragging_the_thumb_and_the_content() {
    let mut rig = Rig::new(|_| column().child(scroll_view(column().gap(0).children((0..40).map(|i| label(format!("Row {i}"))))).id("s").height(100)));
    rig.configure(300, 200);
    let main = rig.hit("s");
    let thumb = rig.app.core.window(1).unwrap().hit_ids_of("s").into_iter().find(|h| *h != main).expect("a thumb hit id");
    // Drag the thumb down 20 lu: content scrolls by 20 * (content - view) / (view - thumb).
    rig.send(vec![pointer(proto::PointerKind::Down, 296, 5, thumb)]);
    rig.send(vec![pointer(proto::PointerKind::Move, 296, 25, thumb)]);
    let s1 = rig.state("s").scroll;
    assert!(s1 > 20, "thumb drag scrolls faster than the pointer moves: {s1}");
    rig.send(vec![pointer(proto::PointerKind::Up, 296, 25, thumb)]);
    // Drag the content up by 30: scrolls 30 further.
    rig.send(vec![pointer(proto::PointerKind::Down, 100, 80, main)]);
    rig.send(vec![pointer(proto::PointerKind::Move, 100, 50, main)]);
    rig.send(vec![pointer(proto::PointerKind::Up, 100, 50, main)]);
    assert_eq!(rig.state("s").scroll, s1 + 30);
}

#[test]
fn custom_areas_get_raw_pointer_key_text_and_paste_events() {
    let log = Rc::new(RefCell::new(Vec::<String>::new()));
    let l = log.clone();
    let mut rig = Rig::new(move |_| {
        let (a, b, c, d) = (l.clone(), l.clone(), l.clone(), l.clone());
        column().padding(10).child(
            custom()
                .id("c")
                .width(200)
                .height(100)
                .nodes(vec![SceneNode::new("dot", NodeKind::Rect, vec![Prop::X(5), Prop::Y(5), Prop::W(3), Prop::H(3), Prop::Fill(0xff0000ff)])])
                .on_pointer(move |cx, e| {
                    if let EventValue::Pointer(p) = &e.value {
                        a.borrow_mut().push(format!("{:?} {} {}", p.kind, p.x, p.y));
                        if p.button == proto::Button::Right {
                            cx.paste_into("c");
                        }
                    }
                })
                .on_key(move |_, e| {
                    if let EventValue::Key { key, down, .. } = &e.value {
                        b.borrow_mut().push(format!("key {key:#x} {down}"));
                    }
                })
                .on_text(move |_, e| c.borrow_mut().push(format!("text {:?}", e.value)))
                .on_paste(move |_, e| d.borrow_mut().push(format!("paste {:?}", e.value))),
        )
    });
    rig.configure(300, 200);
    assert!(rig.model().fills().contains(&0xff0000ff), "its nodes are drawn");
    let h = rig.hit("c");
    rig.send(vec![pointer(proto::PointerKind::Down, 30, 40, h)]);
    // Moves keep going to it while the button is held, even off its hit area.
    rig.send(vec![pointer(proto::PointerKind::Move, 400, 40, 0)]);
    rig.send(vec![pointer(proto::PointerKind::Up, 400, 40, 0)]);
    assert_eq!(rig.focused().as_deref(), Some("#c"));
    // Tab goes to it as a key; text too.
    rig.send(vec![key(keys::TAB, 0), Message::Key(proto::Key { surface: 1, key: keys::TAB, mods: 0, down: false, repeat: false }), text("ls")]);
    let mut right = click(h, 50, 50);
    for m in &mut right {
        if let Message::Pointer(p) = m {
            p.button = proto::Button::Right;
        }
    }
    let sent = rig.send(right);
    assert!(sent.iter().any(|m| matches!(m, Message::ClipboardRequest(_))));
    rig.send(vec![Message::ClipboardData(proto::ClipboardData { mime: TEXT_MIME.into(), data: b"echo hi".to_vec() })]);
    assert_eq!(
        *log.borrow(),
        vec![
            "Down 20 30",
            "Move 390 30",
            "Up 390 30",
            "key 0x2b true",
            "key 0x2b false",
            "text Text(\"ls\")",
            "Down 40 40",
            "Up 40 40",
            "paste Text(\"echo hi\")"
        ]
    );
}

#[test]
fn visible_in_hides_elements_by_size_class() {
    let mut rig = Rig::new(|_| row().child(label("always")).child(label("sidebar").visible_in(false, true, true)).child(label("wide only").visible_in(false, false, true)));
    rig.configure(300, 200);
    assert_eq!(rig.model().texts(), vec!["always"]);
    rig.send(vec![Message::Configure(proto::Configure { surface: 1, serial: 2, w: 600, h: 200, states: 0 })]);
    assert!(rig.model().texts().contains(&"sidebar".to_string()) && !rig.model().texts().contains(&"wide only".to_string()));
    rig.send(vec![Message::Configure(proto::Configure { surface: 1, serial: 3, w: 900, h: 200, states: 0 })]);
    assert_eq!(rig.model().texts().len(), 3);
}

#[test]
fn image_and_canvas_helpers_create_and_upload_resources() {
    let conn = MockConnection::new();
    conn.state.borrow_mut().idle_timeouts = true;
    conn.push(&[welcome(false)]);
    let mut app = App::with_connection("dev.test.app", Box::new(conn.clone())).unwrap();
    let icon = ecm_ui::icons::builtin("terminal").unwrap();
    let id = app.core.image_rgba(icon.w, icon.h, &icon.rgba).unwrap();
    let canvas = app.core.canvas(8, 4, proto::PixelFormat::Rgba8888);
    app.core.put_canvas(canvas, &[0u8; 8 * 4 * 4], &[[0, 0, 2, 2]]).unwrap();
    let _ = app.core.flush();
    let sent = conn.sent_messages();
    assert!(sent.iter().any(|m| matches!(m, Message::ResourceCreate(r) if r.id == id && r.kind == proto::ResourceKind::Image && r.w == 16)));
    assert!(sent.iter().any(|m| matches!(m, Message::ResourceCreate(r) if r.id == canvas && r.kind == proto::ResourceKind::Canvas)));
    let st = conn.state.borrow();
    assert_eq!(st.uploads[0].0, id);
    assert_eq!(st.uploads[0].1, 1, "raw RGBA");
    assert_eq!(st.pixels[0].0, canvas);
    assert_eq!(st.pixels[0].2, vec![[0, 0, 2, 2]]);
}

#[test]
fn unused_keys_reach_the_app_key_handler() {
    let saved = Rc::new(RefCell::new(0));
    let s = saved.clone();
    let mut rig = Rig::new(|_| column().child(text_area().id("t")));
    rig.app.on_key(move |_, k, m| {
        if k == keys::S && m == CTRL {
            *s.borrow_mut() += 1;
            return true;
        }
        false
    });
    rig.configure(300, 200);
    rig.send(click(rig.hit("t"), 10, 10));
    rig.send(vec![text("abc"), key(keys::S, CTRL)]);
    assert_eq!(*saved.borrow(), 1, "Ctrl+S isn't a text area key");
    assert_eq!(rig.state("t").text, "abc");
}

#[test]
fn double_click_selects_a_word_and_activates_list_rows() {
    let activated = Rc::new(RefCell::new(Vec::new()));
    let a = activated.clone();
    let mut rig = Rig::new(move |_| {
        let a = a.clone();
        column().child(text_field().id("f").text("alpha beta")).child(list().id("l").child(label("one")).child(label("two")).on_activate(move |_, e| a.borrow_mut().push(e.value.clone())))
    });
    rig.conn.state.borrow_mut().now_ms = Some(5000);
    rig.configure(300, 200);
    let f = rig.hit("f");
    rig.send(click(f, 60, 10));
    rig.conn.state.borrow_mut().now_ms = Some(5200);
    rig.send(click(f, 60, 10));
    assert_eq!(rig.state("f").selected_text(), "beta");
    let row = rig.app.core.window(1).unwrap().hit_id_of_row("l", 1).unwrap();
    rig.send(click(row, 10, 50));
    rig.conn.state.borrow_mut().now_ms = Some(5300);
    rig.send(click(row, 10, 50));
    assert_eq!(*activated.borrow(), vec![EventValue::Index(1)]);
    // Enter on the focused list activates too.
    rig.send(vec![key(keys::ENTER, 0)]);
    assert_eq!(activated.borrow().len(), 2);
}
