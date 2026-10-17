//! The card widgets (docs/cards/CARDS.md "Programs and apps"): the
//! `pixel_grid` bitmap editor (pencil, fill, eyedropper, line, mirror,
//! masks, change events, reading the bitmap back) and the `color_picker`
//! (hue and SV drags, the hex field), against a mock compositor.

use ecm_ui::prelude::*;
use ecm_ui::samples::welcome;
use ecm_ui::window::{self, keys, Rect, WidgetState};
use ecm_ui::{Kind, MockConnection};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Message, NodeKind, TreeOp};
use std::cell::RefCell;
use std::rc::Rc;

/// What the handlers saw: `(kind, value)` of every grid and picker event.
type Log = Rc<RefCell<Vec<(EventKind, EventValue)>>>;

struct Rig {
    conn: MockConnection,
    app: App,
    log: Log,
    tool: Rc<RefCell<String>>,
    mirror: Rc<RefCell<bool>>,
    nodes: usize,
}

const COLS: u16 = 8;
const ROWS: u16 = 4;
const ZOOM: i32 = 10;

impl Rig {
    /// A 8x4 grid at zoom 10 (texel (5, 0) locked), colour 3, over a 120x140 picker.
    fn new() -> Rig {
        let conn = MockConnection::new();
        conn.state.borrow_mut().idle_timeouts = true;
        conn.push(&[welcome(false)]);
        let log: Log = Rc::default();
        let tool = Rc::new(RefCell::new("pencil".to_string()));
        let mirror = Rc::new(RefCell::new(false));
        // The app's copy of the picker's colour (a view passes `value` every
        // render, so it keeps it in step through `on_change`, as apps do).
        let color = Rc::new(RefCell::new(0xff0000));
        let mut app = App::with_connection("dev.test.cards", Box::new(conn.clone())).unwrap();
        let (l1, l2, l3, t, m, c) = (log.clone(), log.clone(), log.clone(), tool.clone(), mirror.clone(), color.clone());
        app.window("Cards", move |_| {
            let mut mask = vec![0u8; (COLS * ROWS) as usize];
            mask[5] = 1;
            column()
                .gap(0)
                .child(
                    pixel_grid(COLS, ROWS)
                        .id("grid")
                        .zoom(ZOOM as u16)
                        .color(3)
                        .tool(t.borrow().clone())
                        .mirror_h(*m.borrow())
                        .mask(mask)
                        .on_change({
                            let l = l1.clone();
                            move |_, e| l.borrow_mut().push((e.kind, e.value.clone()))
                        })
                        .on_select({
                            let l = l2.clone();
                            move |_, e| l.borrow_mut().push((e.kind, e.value.clone()))
                        }),
                )
                .child(color_picker().id("pick").width(120).value(*c.borrow()).on_change({
                    let (l, c) = (l3.clone(), c.clone());
                    move |_, e| {
                        if let EventValue::Int(v) = e.value {
                            *c.borrow_mut() = v;
                        }
                        l.borrow_mut().push((e.kind, e.value.clone()))
                    }
                }))
        });
        let mut rig = Rig { conn, app, log, tool, mirror, nodes: 0 };
        rig.send(vec![Message::Configure(proto::Configure { surface: 1, serial: 1, w: 200, h: 300, states: proto::window_states::ACTIVATED })]);
        rig
    }

    fn send(&mut self, msgs: Vec<Message>) -> Vec<Message> {
        self.conn.clear_sent();
        self.conn.push(&msgs);
        self.app.step(0);
        self.app.step(0);
        let sent = self.conn.sent_messages();
        for m in &sent {
            if let Message::TreeOps(t) = m {
                for op in &t.ops {
                    match op {
                        TreeOp::Create { .. } => self.nodes += 1,
                        TreeOp::Remove { .. } => self.nodes -= 1,
                        _ => {}
                    }
                }
            }
        }
        sent
    }

    fn window(&self) -> &window::Window {
        self.app.core.window(1).unwrap()
    }

    fn state(&self, id: &str) -> WidgetState {
        self.window().state(id).cloned().unwrap_or_default()
    }

    fn pixels(&self) -> Vec<u8> {
        self.window().pixels("grid")
    }

    fn grid_rect(&self) -> Rect {
        self.window().rect_of("grid").unwrap()
    }

    /// Surface coordinates of the centre of texel `(x, y)`.
    fn texel(&self, x: i32, y: i32) -> (i32, i32) {
        let r = self.grid_rect();
        (r.x + x * ZOOM + ZOOM / 2, r.y + y * ZOOM + ZOOM / 2)
    }

    fn pointer(&mut self, kind: proto::PointerKind, hit: u32, x: i32, y: i32) {
        let down = kind == proto::PointerKind::Down;
        self.send(vec![Message::Pointer(proto::Pointer {
            surface: 1,
            kind,
            x,
            y,
            // Moves in these tests are drags (the button is held).
            buttons: if down || kind == proto::PointerKind::Move { proto::buttons::LEFT } else { 0 },
            button: if down || kind == proto::PointerKind::Up { proto::Button::Left } else { proto::Button::None },
            scroll_x: 0,
            scroll_y: 0,
            hit_id: hit,
        })]);
    }

    fn grid_hit(&self) -> u32 {
        self.window().hit_id_of("grid").unwrap()
    }

    fn press_texel(&mut self, x: i32, y: i32) {
        let (px, py) = self.texel(x, y);
        let hit = self.grid_hit();
        self.pointer(proto::PointerKind::Down, hit, px, py);
    }

    fn move_texel(&mut self, x: i32, y: i32) {
        let (px, py) = self.texel(x, y);
        let hit = self.grid_hit();
        self.pointer(proto::PointerKind::Move, hit, px, py);
    }

    fn release_texel(&mut self, x: i32, y: i32) {
        let (px, py) = self.texel(x, y);
        let hit = self.grid_hit();
        self.pointer(proto::PointerKind::Up, hit, px, py);
    }

    fn click_texel(&mut self, x: i32, y: i32) {
        self.press_texel(x, y);
        self.release_texel(x, y);
    }

    fn set_tool(&mut self, tool: &str) {
        *self.tool.borrow_mut() = tool.to_string();
        self.send(vec![]);
    }

    fn events(&self) -> Vec<(EventKind, EventValue)> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    fn texels(&self, x: i32, y: i32) -> u8 {
        self.pixels()[(y * COLS as i32 + x) as usize]
    }
}

fn texels_of(v: &EventValue) -> Vec<[i32; 3]> {
    match v {
        EventValue::Texels(t) => t.clone(),
        other => panic!("not texels: {other:?}"),
    }
}

#[test]
fn the_pencil_paints_a_texel_and_reports_it() {
    let mut rig = Rig::new();
    assert_eq!(rig.pixels().len(), (COLS * ROWS) as usize);
    assert!(rig.pixels().iter().all(|p| *p == 0));
    rig.click_texel(2, 1);
    assert_eq!(rig.texels(2, 1), 3);
    let ev = rig.events();
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].0, EventKind::Change);
    assert_eq!(texels_of(&ev[0].1), vec![[2, 1, 3]]);
    // The same colour again changes nothing and says nothing.
    rig.click_texel(2, 1);
    assert!(rig.events().is_empty());
    // The grid took keyboard focus.
    assert_eq!(rig.window().focused(), Some("#grid"));
    // The toolkit's own reader and the app's see the same bitmap.
    assert_eq!(rig.app.core.window(1).unwrap().state("grid").unwrap().pixels, rig.pixels());
}

#[test]
fn a_pencil_drag_paints_the_line_between_moves() {
    let mut rig = Rig::new();
    rig.press_texel(0, 0);
    rig.move_texel(3, 0);
    rig.move_texel(3, 3);
    rig.release_texel(3, 3);
    for x in 0..=3 {
        assert_eq!(rig.texels(x, 0), 3, "texel {x},0");
    }
    for y in 0..=3 {
        assert_eq!(rig.texels(3, y), 3, "texel 3,{y}");
    }
    assert_eq!(rig.texels(4, 0), 0);
    assert_eq!(rig.events().len(), 3, "one change per press / move that painted");
}

#[test]
fn a_locked_texel_is_never_painted() {
    let mut rig = Rig::new();
    rig.click_texel(5, 0);
    assert_eq!(rig.texels(5, 0), 0);
    assert!(rig.events().is_empty());
    // Dragging across it skips it.
    rig.press_texel(4, 0);
    rig.move_texel(6, 0);
    rig.release_texel(6, 0);
    assert_eq!((rig.texels(4, 0), rig.texels(5, 0), rig.texels(6, 0)), (3, 0, 3));
    // Locked texels are drawn hatched: a wash rect and a line node over the run.
    assert!(rig.window().node_id("#grid:m0x5").is_some());
    assert!(rig.window().node_id("#grid:mh0x5").is_some());
}

#[test]
fn fill_floods_the_connected_area_and_stops_at_locks() {
    let mut rig = Rig::new();
    // A wall of colour 3 down column 2, then fill the left part.
    for y in 0..ROWS as i32 {
        rig.click_texel(2, y);
    }
    rig.events();
    rig.set_tool("fill");
    // Fill with a different colour: change the paint colour through the eyedropper's state.
    rig.click_texel(0, 0);
    let ev = rig.events();
    assert_eq!(ev.len(), 1);
    let changed = texels_of(&ev[0].1);
    assert_eq!(changed.len(), 2 * ROWS as usize, "columns 0 and 1 only");
    assert!(changed.iter().all(|t| t[0] < 2 && t[2] == 3));
    assert_eq!(rig.texels(3, 0), 0, "the wall keeps the right side");
    // The right side: the lock at (5, 0) stays, everything else fills.
    rig.click_texel(7, 3);
    assert_eq!(rig.texels(5, 0), 0);
    assert_eq!(rig.texels(6, 0), 3);
    assert_eq!(rig.texels(7, 3), 3);
    assert!(rig.pixels().iter().enumerate().all(|(i, p)| *p == 3 || i == 5));
    assert_eq!(rig.events().len(), 1);
    // Filling with the colour already there is a no-op.
    rig.click_texel(0, 0);
    assert!(rig.events().is_empty());
}

#[test]
fn mirror_h_paints_both_halves() {
    let mut rig = Rig::new();
    *rig.mirror.borrow_mut() = true;
    rig.send(vec![]);
    rig.click_texel(1, 2);
    assert_eq!((rig.texels(1, 2), rig.texels(6, 2)), (3, 3));
    let ev = rig.events();
    let mut t = texels_of(&ev[0].1);
    t.sort();
    assert_eq!(t, vec![[1, 2, 3], [6, 2, 3]]);
    // The mirror of a locked texel is painted, the locked one not.
    rig.click_texel(2, 0);
    assert_eq!((rig.texels(2, 0), rig.texels(5, 0)), (3, 0));
    // Mirrored fill floods both areas.
    rig.set_tool("fill");
    rig.click_texel(0, 3);
    assert_eq!((rig.texels(0, 3), rig.texels(7, 3)), (3, 3));
}

#[test]
fn the_eyedropper_selects_the_texels_colour() {
    let mut rig = Rig::new();
    rig.click_texel(4, 1);
    rig.events();
    rig.set_tool("eyedropper");
    rig.click_texel(4, 1);
    let ev = rig.events();
    assert_eq!(ev, vec![(EventKind::Select, EventValue::Index(3))]);
    assert_eq!(rig.state("grid").selected, Some(3));
    rig.click_texel(0, 0);
    assert_eq!(rig.events(), vec![(EventKind::Select, EventValue::Index(0))]);
    assert_eq!(rig.texels(0, 0), 0, "picking doesn't paint");
}

#[test]
fn the_line_tool_previews_and_paints_on_release() {
    let mut rig = Rig::new();
    rig.set_tool("line");
    rig.press_texel(0, 3);
    rig.move_texel(7, 0);
    // Nothing painted yet, but the preview is drawn.
    assert!(rig.pixels().iter().all(|p| *p == 0));
    assert!(rig.events().is_empty());
    let preview = rig.window().node_id("#grid:pv0").is_some();
    assert!(preview, "the line preview is on screen while dragging");
    rig.release_texel(7, 3);
    for x in 0..COLS as i32 {
        assert_eq!(rig.texels(x, 3), 3, "texel {x},3");
    }
    assert_eq!(rig.texels(7, 2), 0);
    let ev = rig.events();
    assert_eq!(ev.len(), 1);
    assert_eq!(texels_of(&ev[0].1).len(), COLS as usize);
    assert!(rig.window().node_id("#grid:pv0").is_none(), "the preview is gone");
}

#[test]
fn pixels_are_pushed_from_the_app_and_read_back() {
    let conn = MockConnection::new();
    conn.state.borrow_mut().idle_timeouts = true;
    conn.push(&[welcome(false)]);
    let bitmap = Rc::new(RefCell::new(vec![1u8, 2, 3, 4, 5, 6]));
    let mut app = App::with_connection("dev.test.cards", Box::new(conn.clone())).unwrap();
    let b = bitmap.clone();
    app.window("Cards", move |cx| {
        let seen = cx.pixels("grid");
        let _ = seen;
        pixel_grid(3, 2).id("grid").zoom(4).pixels(b.borrow().clone())
    });
    conn.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w: 100, h: 100, states: 0 })]);
    app.step(0);
    app.step(0);
    assert_eq!(app.core.window(1).unwrap().pixels("grid"), vec![1, 2, 3, 4, 5, 6]);
    // A handler can replace the bitmap; the next render shows it and the view reads it.
    let mut cx = Ctx::new(&mut app.core, 1);
    cx.set_pixels("grid", &[6, 5, 4, 3, 2, 1]);
    assert_eq!(cx.pixels("grid"), vec![6, 5, 4, 3, 2, 1]);
    // A shorter push is padded to cols * rows.
    *bitmap.borrow_mut() = vec![9, 9];
    app.step(0);
    app.step(0);
    assert_eq!(app.core.window(1).unwrap().pixels("grid"), vec![9, 9, 0, 0, 0, 0]);
}

#[test]
fn a_card_sized_grid_stays_well_under_the_node_limit() {
    let conn = MockConnection::new();
    conn.state.borrow_mut().idle_timeouts = true;
    conn.push(&[welcome(false)]);
    let mut app = App::with_connection("dev.test.cards", Box::new(conn.clone())).unwrap();
    app.window("Cards", |_| {
        // A noisy bitmap: every texel differs from its neighbour (the worst case for runs).
        let px: Vec<u8> = (0..32 * 20).map(|i| (i % 2 + (i / 32) % 2) as u8).collect();
        pixel_grid(32, 20).id("grid").zoom(6).pixels(px).grid(true)
    });
    conn.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w: 640, h: 400, states: 0 })]);
    app.step(0);
    app.step(0);
    let created = conn
        .sent_messages()
        .iter()
        .filter_map(|m| if let Message::TreeOps(t) = m { Some(t.ops.iter().filter(|o| matches!(o, TreeOp::Create { .. })).count()) } else { None })
        .sum::<usize>();
    assert!(created < 800, "{created} nodes");
    assert!(created > 640, "{created} nodes: every texel plus gridlines");
    assert!(conn.sent_messages().iter().any(|m| matches!(m, Message::TreeOps(t) if t.ops.iter().any(|o| matches!(o, TreeOp::Create { kind: NodeKind::Rect, .. })))));
}

#[test]
fn zoom_fits_the_layout_when_not_given() {
    let conn = MockConnection::new();
    conn.state.borrow_mut().idle_timeouts = true;
    conn.push(&[welcome(false)]);
    let mut app = App::with_connection("dev.test.cards", Box::new(conn.clone())).unwrap();
    app.window("Cards", |_| column().child(pixel_grid(32, 20).id("grid")));
    conn.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w: 200, h: 300, states: 0 })]);
    app.step(0);
    app.step(0);
    let w = app.core.window(1).unwrap();
    let r = w.rect_of("grid").unwrap();
    assert_eq!((r.w, r.h), (200, 300), "fills the window without a zoom");
    // 200 / 32 = 6, 300 / 20 = 15: texels are 6 lu, so the pointer maps accordingly.
    let hit = w.hit_id_of("grid").unwrap();
    conn.push(&ecm_ui::samples::click(hit, 6 * 7 + 3, 6 * 2 + 3));
    app.step(0);
    app.step(0);
    assert_eq!(app.core.window(1).unwrap().pixels("grid")[2 * 32 + 7], 0, "paints colour 0 over 0: unchanged");
}

// ------------------------------------------------------------ colour picker

#[test]
fn hue_and_sv_drags_change_the_colour() {
    let mut rig = Rig::new();
    assert_eq!(rig.state("pick").value, 0xff0000);
    assert_eq!(rig.state("pick").hsv, [0, 255, 255]);
    let (hue, sv, _) = rig.window().picker_hit_ids("pick").unwrap();
    let r = rig.window().rect_of("pick").unwrap();
    assert_eq!((r.w, r.h), (120, 140), "default size");
    // Drag the hue strip to its middle: cyan-ish (hue 180).
    let strip_h = r.h - 20 - window::PICKER_GAP;
    let hx = r.x + r.w - window::HUE_STRIP_W / 2;
    rig.pointer(proto::PointerKind::Down, hue, hx, r.y + strip_h / 2);
    let v = rig.state("pick");
    assert!((179..=181).contains(&v.hsv[0]), "hue {}", v.hsv[0]);
    assert_eq!(v.value, window::hsv_to_rgb(v.hsv[0], 255, 255));
    assert!(v.value >> 16 == 0 && v.value & 0xff == 0xff && (v.value >> 8) & 0xff >= 0xf0, "cyan-ish: {:06x}", v.value);
    assert_eq!(rig.events(), vec![(EventKind::Change, EventValue::Int(v.value))]);
    rig.pointer(proto::PointerKind::Move, hue, hx, r.y + strip_h - 1);
    assert_eq!(rig.state("pick").hsv[0], 359);
    rig.pointer(proto::PointerKind::Up, hue, hx, r.y + strip_h - 1);
    assert_eq!(rig.events().len(), 1);
    // The SV square: the top-left corner is white, the bottom edge black.
    let sq_w = r.w - window::HUE_STRIP_W - window::PICKER_GAP;
    rig.pointer(proto::PointerKind::Down, sv, r.x, r.y);
    assert_eq!(rig.state("pick").value, 0xffffff);
    rig.pointer(proto::PointerKind::Move, sv, r.x + sq_w / 2, r.y + strip_h - 1);
    assert_eq!(rig.state("pick").value, 0x000000);
    rig.pointer(proto::PointerKind::Up, sv, r.x + sq_w / 2, r.y + strip_h - 1);
    // A grey keeps its hue for the next drag.
    assert_eq!(rig.state("pick").hsv[0], 359);
    assert_eq!(rig.events().len(), 2);
    // The text shows the colour.
    assert_eq!(rig.state("pick").text, "#000000");
}

#[test]
fn the_hex_field_parses_what_is_typed() {
    let mut rig = Rig::new();
    let (_, _, field) = rig.window().picker_hit_ids("pick").unwrap();
    let r = rig.window().rect_of("pick").unwrap();
    rig.pointer(proto::PointerKind::Down, field, r.x + 40, r.y + r.h - 10);
    rig.pointer(proto::PointerKind::Up, field, r.x + 40, r.y + r.h - 10);
    assert_eq!(rig.window().focused(), Some("#pick"));
    assert!(rig.events().is_empty());
    // Clear it and type a colour: applied once it is complete.
    for _ in 0..7 {
        rig.send(vec![Message::Key(proto::Key { surface: 1, key: keys::BACKSPACE, mods: 0, down: true, repeat: false })]);
    }
    assert_eq!(rig.state("pick").text, "");
    rig.send(vec![Message::Text(proto::Text { surface: 1, text: "12AB".into() })]);
    assert_eq!(rig.state("pick").text, "#12ab", "a # is implied and digits are lowercased");
    assert!(rig.events().is_empty());
    rig.send(vec![Message::Text(proto::Text { surface: 1, text: "zz".into() })]);
    assert_eq!(rig.state("pick").text, "#12ab", "non-hex characters are dropped");
    rig.send(vec![Message::Text(proto::Text { surface: 1, text: "cd".into() })]);
    assert_eq!(rig.state("pick").value, 0x12abcd);
    assert_eq!(rig.events(), vec![(EventKind::Change, EventValue::Int(0x12abcd))]);
    // Enter on a half-typed colour puts the field back.
    rig.send(vec![Message::Key(proto::Key { surface: 1, key: keys::BACKSPACE, mods: 0, down: true, repeat: false })]);
    rig.send(vec![Message::Key(proto::Key { surface: 1, key: keys::ENTER, mods: 0, down: true, repeat: false })]);
    assert_eq!(rig.state("pick").text, "#12abcd");
    assert_eq!(rig.state("pick").value, 0x12abcd);
    assert!(rig.events().is_empty());
    // Pasting a colour works too (through the clipboard request).
    rig.send(vec![Message::Key(proto::Key { surface: 1, key: keys::V, mods: proto::mods::CTRL, down: true, repeat: false })]);
    let asked = rig.conn.sent_messages().iter().any(|m| matches!(m, Message::ClipboardRequest(_)));
    assert!(asked);
}

#[test]
fn hex_and_hsv_helpers_round_trip() {
    assert_eq!(window::parse_hex("#abc"), Some(0xaabbcc));
    assert_eq!(window::parse_hex("ABCDEF"), Some(0xabcdef));
    assert_eq!(window::parse_hex("#12345"), None);
    assert_eq!(window::parse_hex(""), None);
    assert_eq!(window::hex_of(0x00ff7f), "#00ff7f");
    assert_eq!(window::hsv_to_rgb(0, 255, 255), 0xff0000);
    assert_eq!(window::hsv_to_rgb(120, 255, 255), 0x00ff00);
    assert_eq!(window::hsv_to_rgb(240, 255, 255), 0x0000ff);
    assert_eq!(window::hsv_to_rgb(60, 0, 128), 0x808080);
    for rgb in [0x000000, 0xffffff, 0x123456, 0xfedcba, 0x3d8bfd, 0x808080] {
        let [h, s, v] = window::rgb_to_hsv(rgb, 0);
        let back = window::hsv_to_rgb(h, s, v);
        let d = |a: i32, b: i32, sh: i32| ((a >> sh) & 255 - (b >> sh) & 255).abs();
        assert!(d(rgb, back, 16) <= 2 && d(rgb, back, 8) <= 2 && d(rgb, back, 0) <= 2, "{rgb:06x} -> {h},{s},{v} -> {back:06x}");
    }
    assert_eq!(window::rgb_to_hsv(0x808080, 77)[0], 77, "a grey keeps the hue it is given");
}

#[test]
fn layout_files_describe_both_widgets() {
    let json = r##"{ "window": "Card", "root": { "row": { "children": [
        { "pixel_grid": { "id": "face", "cols": 4, "rows": 2, "zoom": 8, "palette": ["#000000", "#ffffff", 16711680],
                          "pixels": "01230123", "mask": [0, 0, 0, 1, 0, 0, 0, 1], "tool": "fill", "mirror_h": true, "grid": false, "color": 2, "on_change": "painted" } },
        { "color_picker": { "id": "pick", "value": 65280, "width": 100, "height": 120, "on_change": "picked" } } ] } } }"##;
    let layout = ecm_ui::layout_file::load(json).unwrap();
    let grid = &layout.root.children[0];
    assert_eq!(grid.kind, Kind::PixelGrid);
    let p = &grid.props;
    assert_eq!((p.cols, p.rows, p.zoom), (Some(4), Some(2), Some(8)));
    assert_eq!(p.palette, vec![0x000000, 0xffffff, 0xff0000]);
    assert_eq!(p.pixels, vec![0, 1, 2, 3, 0, 1, 2, 3]);
    assert_eq!(p.mask, vec![0, 0, 0, 1, 0, 0, 0, 1]);
    assert_eq!(p.tool.as_deref(), Some("fill"));
    assert_eq!((p.mirror_h, p.grid, p.color), (Some(true), Some(false), Some(2)));
    assert!(grid.handler(EventKind::Change).is_some());
    let pick = &layout.root.children[1];
    assert_eq!(pick.kind, Kind::ColorPicker);
    assert_eq!(pick.props.value, Some(65280));
    assert!(ecm_ui::layout_file::load(r#"{"root": {"pixel_grid": {"pixels": "0g"}}}"#).is_err());
    assert!(ecm_ui::layout_file::load(r#"{"root": {"pixel_grid": {"palette": [1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17]}}}"#).is_err());
    assert_eq!(Kind::from_name("pixelgrid"), Some(Kind::PixelGrid));
    assert_eq!(Kind::from_name("color_picker"), Some(Kind::ColorPicker));
}
