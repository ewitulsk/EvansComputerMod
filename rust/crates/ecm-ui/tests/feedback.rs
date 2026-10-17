//! Protocol v2 in the toolkit: timed canvas frames (`CommitAt`, `QueueFlush`,
//! `SurfaceSetContent`, `QueueLimits`) and the client feedback props (hover /
//! pressed looks, scroll containers and `Scrolled`, the caret), against a
//! mock compositor.

use ecm_ui::prelude::*;
use ecm_ui::samples::{click, welcome};
use ecm_ui::{MockConnection, Theme};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Message, NodeKind, Prop, TreeOp};
use std::collections::HashMap;

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

    /// Nodes carrying `hit`, as (id, node).
    fn with_hit(&self, hit: u32) -> Vec<(u32, &Node)> {
        let mut v: Vec<(u32, &Node)> = self.nodes.iter().filter(|(_, n)| n.props.contains(&Prop::HitId(hit))).map(|(i, n)| (*i, n)).collect();
        v.sort_by_key(|(i, _)| *i);
        v
    }

    fn find(&self, f: impl Fn(&Node) -> bool) -> Option<(u32, &Node)> {
        let mut v: Vec<(u32, &Node)> = self.nodes.iter().filter(|(_, n)| f(n)).map(|(i, n)| (*i, n)).collect();
        v.sort_by_key(|(i, _)| *i);
        v.into_iter().next()
    }
}

fn prop<T>(n: &Node, f: impl Fn(&Prop) -> Option<T>) -> Option<T> {
    n.props.iter().find_map(f)
}

struct Rig {
    conn: MockConnection,
    app: App,
    model: Model,
}

impl Rig {
    fn with_welcome(view: impl FnMut(&ViewCtx) -> Element + 'static, first: Vec<Message>) -> Rig {
        let conn = MockConnection::new();
        conn.state.borrow_mut().idle_timeouts = true;
        conn.push(&first);
        let mut app = App::with_connection("dev.test.app", Box::new(conn.clone())).unwrap();
        app.window("Test", view);
        Rig { conn, app, model: Model::default() }
    }

    fn new(view: impl FnMut(&ViewCtx) -> Element + 'static) -> Rig {
        Rig::with_welcome(view, vec![welcome(false)])
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
                if t.surface == 1 {
                    self.model.apply(&t.ops);
                }
            }
        }
        sent
    }

    fn hit(&self, id: &str) -> u32 {
        self.app.core.window(1).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no hit id for {id}"))
    }
}

fn queue_limits() -> Message {
    Message::QueueLimits(proto::QueueLimits { max_queue_frames: 12, max_queue_ms: 500 })
}

#[test]
fn hello_says_version_2_and_queue_limits_are_kept() {
    let conn = MockConnection::new();
    conn.push(&[welcome(true), queue_limits(), Message::Restore(proto::Restore { data: b"{}".to_vec() })]);
    let app = App::with_connection("dev.test.app", Box::new(conn.clone())).unwrap();
    assert!(matches!(&conn.sent_messages()[0], Message::Hello(h) if h.version == 2));
    assert_eq!(app.core.queue_limits(), Some((12, 500)), "QueueLimits between Welcome and Restore");
    assert_eq!(app.core.restore(), Some(&b"{}"[..]));
    // A version 1 compositor sends none.
    let conn = MockConnection::new();
    conn.push(&[welcome(false)]);
    let app = App::with_connection("dev.test.app", Box::new(conn)).unwrap();
    assert_eq!(app.core.queue_limits(), None);
}

#[test]
fn timed_canvas_frames_commit_at_their_time_after_the_tree() {
    let mut rig = Rig::with_welcome(|_| column().child(label("Video")).child(canvas(1, 8, 4)), vec![welcome(false), queue_limits()]);
    let cv = rig.app.core.canvas(8, 4, proto::PixelFormat::Rgba8888);
    assert_eq!(cv, 1);
    rig.app.core.set_content(cv, 2);
    rig.configure(200, 100);
    let sent = rig.send(vec![]);
    let _ = sent;
    // A tree change is pending (the label) when a timed frame is put: it goes first with Commit.
    rig.conn.clear_sent();
    rig.app.set_root(1, column().child(label("Video 2")).child(canvas(1, 8, 4)));
    rig.app.core.put_canvas_at(cv, &[7u8; 8 * 4 * 4], &[], 5_000_000).unwrap();
    let sent = rig.conn.sent_messages();
    let names: Vec<&str> = sent.iter().map(|m| m.name()).collect();
    assert_eq!(names, vec!["TreeOps", "Commit", "CommitAt"], "{names:?}");
    assert!(matches!(sent[2], Message::CommitAt(proto::CommitAt { surface: 1, pts_us: 5_000_000 })));
    assert_eq!(rig.conn.state.borrow().pixels.last().unwrap().0, cv);
    // The loop turn after it sends no extra Commit for those pixels.
    rig.conn.clear_sent();
    rig.app.step(0);
    assert!(!rig.conn.sent_messages().iter().any(|m| matches!(m, Message::Commit(_))));

    // put + commit_at (what the Python and Lua Canvas.commit_at do): CommitAt instead of the next Commit.
    rig.conn.clear_sent();
    rig.app.core.put_canvas(cv, &[1u8; 8 * 4 * 4], &[[0, 0, 1, 1]]).unwrap();
    rig.app.core.commit_at(6_000_000).unwrap();
    rig.app.step(0);
    let sent = rig.conn.sent_messages();
    assert_eq!(sent.iter().filter(|m| matches!(m, Message::CommitAt(c) if c.pts_us == 6_000_000)).count(), 1);
    assert!(!sent.iter().any(|m| matches!(m, Message::Commit(_))), "the pixels were committed timed: {sent:?}");

    rig.conn.clear_sent();
    rig.app.core.queue_flush().unwrap();
    assert_eq!(rig.conn.sent_messages(), vec![Message::QueueFlush(proto::QueueFlush { surface: 1 })]);
}

#[test]
fn content_hints_go_out_as_surface_set_content() {
    let conn = MockConnection::new();
    conn.state.borrow_mut().idle_timeouts = true;
    conn.push(&[welcome(false)]);
    let mut app = App::with_connection("dev.test.app", Box::new(conn.clone())).unwrap();
    let cv = app.core.canvas(4, 4, proto::PixelFormat::Rgb565);
    // Before any window: sent with the first one.
    app.core.set_content(cv, 2);
    let _ = app.core.flush();
    assert!(!conn.sent_messages().iter().any(|m| matches!(m, Message::SurfaceSetContent(_))));
    app.window("V", |_| column());
    app.core.set_content(cv, 9);
    let _ = app.core.flush();
    let hints: Vec<(u32, u32, u8)> = conn
        .sent_messages()
        .iter()
        .filter_map(|m| if let Message::SurfaceSetContent(c) = m { Some((c.surface, c.resource, c.hint)) } else { None })
        .collect();
    assert_eq!(hints, vec![(1, cv, 2), (1, cv, 2)], "a hint over 2 is clamped to motion");
}

#[test]
fn buttons_and_rows_carry_their_hover_and_pressed_looks() {
    let mut rig = Rig::new(|_| {
        column()
            .child(button("Plain").id("b"))
            .child(button("Go").id("p").style("primary"))
            .child(button("Off").id("d").enabled(false))
            .child(list().id("l").child(label("a")).child(label("b")))
            .child(tabs(["One", "Two"]).id("t").child(label("x")))
    });
    rig.configure(300, 300);
    let t = Theme::default();
    let b = rig.hit("b");
    let nodes = rig.model.with_hit(b);
    assert_eq!(nodes.len(), 2, "background and label share the button's hit id");
    let (_, bg) = nodes.iter().find(|(_, n)| n.kind == NodeKind::Rect).unwrap();
    assert_eq!(prop(bg, |p| if let Prop::Fill(v) = p { Some(*v) } else { None }), Some(t.button));
    assert!(bg.props.contains(&Prop::HoverFill(t.hover)) && bg.props.contains(&Prop::PressedFill(t.surface_alt)));
    let (_, label) = nodes.iter().find(|(_, n)| n.kind == NodeKind::Text).unwrap();
    assert!(label.props.contains(&Prop::HoverColor(t.text)) && label.props.contains(&Prop::PressedColor(t.text)));
    let p = rig.hit("p");
    let (_, pbg) = rig.model.with_hit(p).into_iter().find(|(_, n)| n.kind == NodeKind::Rect).unwrap();
    assert!(pbg.props.contains(&Prop::HoverFill(t.accent_hover)));
    // Disabled: the looks are the normal look (nothing to predict).
    let d = rig.hit("d");
    let (_, dbg) = rig.model.with_hit(d).into_iter().find(|(_, n)| n.kind == NodeKind::Rect).unwrap();
    assert!(dbg.props.contains(&Prop::HoverFill(t.surface)) && dbg.props.contains(&Prop::PressedFill(t.surface)));
    // List rows: hover = row_hover, pressed = unhovered.
    let row = rig.app.core.window(1).unwrap().hit_id_of_row("l", 0).unwrap();
    let (_, rbg) = rig.model.with_hit(row)[0];
    assert!(rbg.props.contains(&Prop::HoverFill(t.row_hover)) && rbg.props.contains(&Prop::PressedFill(0)));
    // Tabs: the unselected tab's title brightens on hover and carries the tab's hit id.
    let tab = rig.app.core.window(1).unwrap().hit_id_of_row("t", 1).unwrap();
    let tt = rig.model.with_hit(tab).into_iter().find(|(_, n)| n.kind == NodeKind::Text).map(|(_, n)| n.clone()).unwrap();
    assert!(tt.props.contains(&Prop::HoverColor(t.text)) && tt.props.contains(&Prop::PressedColor(t.text_muted)));

    // The props stay put while the server's own look changes (hovering it).
    let before = bg.props.iter().filter(|p| matches!(p, Prop::HoverFill(_) | Prop::PressedFill(_))).cloned().collect::<Vec<_>>();
    rig.send(vec![Message::Pointer(proto::Pointer { surface: 1, kind: proto::PointerKind::Move, x: 5, y: 5, buttons: 0, button: proto::Button::None, scroll_x: 0, scroll_y: 0, hit_id: b })]);
    let (_, bg) = rig.model.with_hit(b).into_iter().find(|(_, n)| n.kind == NodeKind::Rect).unwrap();
    assert!(bg.props.contains(&Prop::Fill(t.hover)), "the server draws the hover itself");
    for p in before {
        assert!(bg.props.contains(&p));
    }
}

#[test]
fn scroll_views_are_scroll_containers_and_take_scrolled() {
    let mut rig = Rig::new(|_| column().child(scroll_view(column().gap(0).children((0..40).map(|i| label(format!("Row {i}"))))).id("s").height(100)));
    rig.configure(300, 200);
    let (vid, vp) = rig.model.find(|n| n.kind == NodeKind::Group && n.props.iter().any(|p| matches!(p, Prop::Scroll(_)))).expect("a scroll group");
    assert_eq!(rig.app.core.window(1).unwrap().node_id("#s:vp"), Some(vid), "the viewport group <key>:vp");
    assert!(vp.props.contains(&Prop::Scroll(2)));
    assert!(vp.props.contains(&Prop::H(100)), "the viewport is the view");
    let ch = prop(vp, |p| if let Prop::ContentH(v) = p { Some(*v) } else { None }).unwrap();
    assert!(ch > 100, "content height {ch}");
    assert!(vp.props.contains(&Prop::ScrollY(0)) && vp.props.contains(&Prop::ScrollX(0)));
    // The client scrolled it to 50: the app lays the content out there and confirms.
    rig.send(vec![Message::Scrolled(proto::Scrolled { surface: 1, node: vid, x: 0, y: 50 })]);
    assert_eq!(rig.app.core.window(1).unwrap().state("s").unwrap().scroll, 50);
    let vp = &rig.model.nodes[&vid];
    assert!(vp.props.contains(&Prop::ScrollY(50)));
    // Its only child is the content group, laid out at -50.
    let kids: Vec<&Node> = rig.model.nodes.values().filter(|n| n.parent == vid).collect();
    assert_eq!(kids.len(), 1);
    assert!(kids[0].props.contains(&Prop::Y(-50)));
    // Too far: clamped to the end.
    rig.send(vec![Message::Scrolled(proto::Scrolled { surface: 1, node: vid, x: 0, y: 100_000 })]);
    assert_eq!(rig.app.core.window(1).unwrap().state("s").unwrap().scroll, ch - 100);
    // The wheel still works (old clients): 16 lu per step.
    rig.send(vec![Message::Pointer(proto::Pointer {
        surface: 1,
        kind: proto::PointerKind::Scroll,
        x: 10,
        y: 10,
        buttons: 0,
        button: proto::Button::None,
        scroll_x: 0,
        scroll_y: -1,
        hit_id: rig.hit("s"),
    })]);
    assert_eq!(rig.app.core.window(1).unwrap().state("s").unwrap().scroll, ch - 100 - Theme::default().scroll_step as i32);
}

#[test]
fn the_focused_text_field_carries_its_caret() {
    let mut rig = Rig::new(|_| column().child(text_field().id("f")).child(text_field().id("g")));
    rig.configure(300, 100);
    rig.send(click(rig.hit("f"), 10, 10));
    // Empty: the placeholder shows, nothing to predict into.
    assert!(rig.model.find(|n| n.props.iter().any(|p| matches!(p, Prop::Caret(_)))).is_none());
    rig.send(vec![Message::Text(proto::Text { surface: 1, text: "héllo".into() })]);
    let (tid, tn) = rig.model.find(|n| n.props.iter().any(|p| matches!(p, Prop::Caret(_)))).expect("a caret text node");
    assert!(tn.props.contains(&Prop::Text("héllo".into())));
    assert!(tn.props.contains(&Prop::Caret(5)), "characters, not bytes");
    let (_, caret) = rig.model.find(|n| n.kind == NodeKind::Rect && n.props.contains(&Prop::W(1))).expect("caret rect");
    assert!(caret.props.contains(&Prop::CaretOf(tid)));
    // Focus moves to the other field: the first field's text node loses Caret (a new node without it).
    rig.send(click(rig.hit("g"), 10, 40));
    let carets: Vec<u32> = rig.model.nodes.values().filter_map(|n| prop(n, |p| if let Prop::Caret(v) = p { Some(*v) } else { None })).collect();
    assert!(carets.is_empty(), "no text node keeps a Caret: {carets:?}");
    assert!(rig.model.find(|n| n.props.contains(&Prop::Text("héllo".into()))).is_some());
}
