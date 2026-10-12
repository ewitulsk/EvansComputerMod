//! Records the UI protocol sessions that the Java golden-image test replays
//! through the compositor and rasterizer (ROADMAP P2 "Golden images").
//!
//!   cargo run -p ecm-ui --example record_ui_sessions -- <out dir>
//!   (the checked-in copies live in src/test/resources/ui-sessions/)
//!
//! Each session is what real clients send: the Notes sample app and the test
//! shell's own policy (`ecm_ui::shell::tiling`) driven through mock
//! connections, plus hand-written clients for the node kinds and layers the
//! sample doesn't use. The compositor's answers are predicted here (window ids
//! and configure serials are deterministic), so the recorded acks match.
//!
//! File format (little-endian): "ECMUISS1", u32 device w, u32 device h, u32 scale,
//! then records `u8 type` + fields:
//!   1 open    u32 conn, u32 pid, u8 role, u16 len + program name
//!   2 send    u32 conn, u32 len + bytes (whole messages)
//!   3 pixels  u32 conn, u32 target, u32 n, n x (i32 x, y, w, h), u32 len + bytes
//!   4 upload  u32 conn, u32 resource, u32 kind, u32 len + bytes
//!   5 advance u32 ms (then the compositor ticks)
//!   6 snap    u16 len + name (the frame must equal <name>.png)
//!   7 pointer u8 kind, i32 x, i32 y (device px), u8 buttons, u8 button, i32 scroll,
//!             u32 conn, u32 expected hit id (checked on the Pointer that conn gets)
//!   9 close   u32 conn

use ecm_ui::shell::{keys, tiling, Bar, Shell, ShellEvent, Wallpaper};
use ecm_ui::{samples, MockConnection};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Message, NodeKind, Prop, TreeOp};
use std::collections::HashMap;

struct Session {
    out: Vec<u8>,
}

impl Session {
    fn new(w: u32, h: u32, scale: u32) -> Session {
        let mut out = b"ECMUISS1".to_vec();
        for v in [w, h, scale] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        Session { out }
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.out.extend_from_slice(b);
    }
    fn open(&mut self, conn: u32, pid: u32, role: u8, name: &str) {
        self.out.push(1);
        self.u32(conn);
        self.u32(pid);
        self.out.push(role);
        self.out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.out.extend_from_slice(name.as_bytes());
    }
    fn send(&mut self, conn: u32, b: &[u8]) {
        if b.is_empty() {
            return;
        }
        Message::decode_all(b).expect("only valid messages are recorded");
        self.out.push(2);
        self.u32(conn);
        self.bytes(b);
    }
    fn send_msgs(&mut self, conn: u32, msgs: &[Message]) {
        self.send(conn, &proto::encode_all(msgs));
    }
    fn pixels(&mut self, conn: u32, target: u32, rects: &[[i32; 4]], b: &[u8]) {
        self.out.push(3);
        self.u32(conn);
        self.u32(target);
        self.u32(rects.len() as u32);
        for r in rects {
            for v in r {
                self.i32(*v);
            }
        }
        self.bytes(b);
    }
    fn upload(&mut self, conn: u32, res: u32, kind: u32, b: &[u8]) {
        self.out.push(4);
        self.u32(conn);
        self.u32(res);
        self.u32(kind);
        self.bytes(b);
    }
    fn advance(&mut self, ms: u32) {
        self.out.push(5);
        self.u32(ms);
    }
    fn snap(&mut self, name: &str) {
        self.out.push(6);
        self.out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.out.extend_from_slice(name.as_bytes());
    }
    #[allow(clippy::too_many_arguments)]
    fn pointer(&mut self, kind: proto::PointerKind, x: i32, y: i32, buttons: u8, button: proto::Button, conn: u32, hit: u32) {
        self.out.push(7);
        self.out.push(kind as u8);
        self.i32(x);
        self.i32(y);
        self.out.push(buttons);
        self.out.push(button as u8);
        self.i32(0);
        self.u32(conn);
        self.u32(hit);
    }
    fn close(&mut self, conn: u32) {
        self.out.push(9);
        self.u32(conn);
    }
}

fn take(m: &MockConnection) -> Vec<u8> {
    let b = m.sent();
    m.clear_sent();
    b
}

fn welcome(w: i32, h: i32) -> Message {
    let mut m = samples::welcome(false);
    if let Message::Welcome(ref mut wl) = m {
        wl.display.w = w;
        wl.display.h = h;
        wl.display.size_class = proto::SizeClass::for_width(w);
    }
    m
}

fn sorted(mut p: Vec<Prop>) -> Vec<Prop> {
    p.sort_by_key(|x| x.tag());
    p
}

fn create(id: u32, parent: u32, kind: NodeKind, props: Vec<Prop>) -> TreeOp {
    TreeOp::Create { id, parent, index: proto::APPEND, kind, props: sorted(props) }
}

fn surface(id: u32, kind: proto::SurfaceKind, role: proto::SurfaceRole, layer: proto::Layer, rect: (i32, i32, i32, i32), format: proto::PixelFormat, title: &str) -> Message {
    surface_child(id, kind, role, layer, 0, rect, format, title)
}

#[allow(clippy::too_many_arguments)]
fn surface_child(id: u32, kind: proto::SurfaceKind, role: proto::SurfaceRole, layer: proto::Layer, parent: u32, rect: (i32, i32, i32, i32), format: proto::PixelFormat, title: &str) -> Message {
    Message::SurfaceCreate(proto::SurfaceCreate {
        surface: id,
        kind,
        role,
        layer,
        parent,
        x: rect.0,
        y: rect.1,
        w: rect.2,
        h: rect.3,
        format,
        scale: 1,
        title: title.into(),
    })
}

fn hello(role: proto::Role) -> Message {
    Message::Hello(proto::Hello { version: proto::VERSION, role, capabilities: vec![] })
}

fn ops(surface: u32, ops: Vec<TreeOp>) -> Message {
    Message::TreeOps(proto::TreeOps { surface, ops })
}

fn ack(surface: u32, serial: u32) -> Message {
    Message::AckConfigure(proto::AckConfigure { surface, serial })
}

fn commit(surface: u32) -> Message {
    Message::Commit(proto::Commit { surface })
}

fn place(window: u32, r: (i32, i32, i32, i32), states: u32, ms: u16) -> Message {
    Message::Place(proto::Place { window, x: r.0, y: r.1, w: r.2, h: r.3, states, transition: proto::Transition { ms, easing: proto::Easing::EaseOut } })
}

fn opened(window: u32, pid: u32, app: &str, title: &str) -> Message {
    Message::WindowOpened(proto::WindowOpened {
        window,
        pid,
        app_id: app.into(),
        title: title.into(),
        icon: String::new(),
        min_w: 0,
        min_h: 0,
        max_w: 0,
        max_h: 0,
        parent: 0,
    })
}

fn configure(surface: u32, serial: u32, w: i32, h: i32, states: u32) -> Message {
    Message::Configure(proto::Configure { surface, serial, w, h, states })
}

fn focus(surface: u32, gained: bool) -> Message {
    Message::Focus(proto::Focus { surface, gained })
}

/// RGB565 diagonal gradient, `w x h`.
fn gradient565(w: usize, h: usize) -> Vec<u8> {
    let mut b = Vec::with_capacity(w * h * 2);
    for y in 0..h {
        for x in 0..w {
            let r = (x * 31 / w.max(1)) as u16;
            let g = (y * 63 / h.max(1)) as u16;
            let bl = (((x + y) * 31) / (w + h)) as u16;
            let v = (r << 11) | (g << 5) | bl;
            b.extend_from_slice(&v.to_le_bytes());
        }
    }
    b
}

/// Where the node with hit id `hit` is in a recorded tree: the centre of its box (surface lu).
fn hit_point(tree_ops: &[TreeOp], hit: u32) -> (i32, i32) {
    #[derive(Default, Clone)]
    struct N {
        parent: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        hit: u32,
    }
    let mut nodes: HashMap<u32, N> = HashMap::new();
    let apply = |n: &mut N, props: &[Prop]| {
        for p in props {
            match p {
                Prop::X(v) => n.x = *v,
                Prop::Y(v) => n.y = *v,
                Prop::W(v) => n.w = *v,
                Prop::H(v) => n.h = *v,
                Prop::HitId(v) => n.hit = *v,
                _ => {}
            }
        }
    };
    for op in tree_ops {
        match op {
            TreeOp::Create { id, parent, props, .. } => {
                let mut n = N { parent: *parent, ..Default::default() };
                apply(&mut n, props);
                nodes.insert(*id, n);
            }
            TreeOp::Set { id, props } => {
                if let Some(n) = nodes.get_mut(id) {
                    apply(n, props);
                }
            }
            TreeOp::Move { id, parent, .. } => {
                if let Some(n) = nodes.get_mut(id) {
                    n.parent = *parent;
                }
            }
            TreeOp::Remove { id } => {
                nodes.remove(id);
            }
        }
    }
    let (id, n) = nodes.iter().filter(|(_, n)| n.hit == hit).min_by_key(|(id, _)| **id).expect("hit id in the tree");
    let _ = id;
    let (mut x, mut y) = (n.x + n.w / 2, n.y + n.h.max(2) / 2);
    let mut p = n.parent;
    while p != 0 {
        let g = &nodes[&p];
        x += g.x;
        y += g.y;
        p = g.parent;
    }
    (x, y)
}

fn decoded_ops(bytes: &[u8]) -> Vec<TreeOp> {
    Message::decode_all(bytes)
        .unwrap()
        .into_iter()
        .filter_map(|m| if let Message::TreeOps(t) = m { Some(t.ops) } else { None })
        .flatten()
        .collect()
}

// ------------------------------------------------------------------ sessions

/// The test shell with its bar, the Notes app and a pixel window, tiled side
/// by side; a click into Notes (hit-tested by the compositor), typing, then the
/// shell going away (safe mode).
fn tiled_shell() -> Vec<u8> {
    let (dw, dh) = (640, 400);
    let mut s = Session::new(dw as u32, dh as u32, 1);

    // The shell, exactly as test-shell starts.
    let shell_mock = MockConnection::new();
    shell_mock.state.borrow_mut().idle_timeouts = true;
    shell_mock.push(&[welcome(dw, dh)]);
    s.open(1, 10, 1, "test-shell");
    let mut sh = Shell::connect(Box::new(shell_mock.clone())).unwrap();
    let mut bar = Bar::new(1, tiling::BAR_H);
    let mut wallpaper = Wallpaper::new(2, tiling::WALLPAPER);
    sh.send(bar.create(proto::Layer::Top));
    sh.send(wallpaper.create());
    sh.bind_chord(1, proto::mods::ALT, &[keys::TAB]);
    sh.bind_chord(2, proto::mods::ALT, &[keys::N]);
    sh.bind_chord(3, proto::mods::ALT, &[keys::F4]);
    sh.flush().unwrap();
    s.send(1, &take(&shell_mock));
    // Compositor: Configure(bar, serial 1), Configure(wallpaper, serial 1).
    bar.configure(&proto::Configure { surface: 1, serial: 1, w: dw, h: tiling::BAR_H, states: 0 });
    wallpaper.configure(&proto::Configure { surface: 2, serial: 1, w: dw, h: dh, states: 0 });
    for m in wallpaper.render() {
        sh.send(m);
    }
    let redraw_bar = |sh: &mut Shell, bar: &mut Bar| {
        bar.set_text(&tiling::bar_text(sh));
        for m in bar.render() {
            sh.send(m);
        }
    };
    redraw_bar(&mut sh, &mut bar);
    sh.flush().unwrap();
    s.send(1, &take(&shell_mock));
    let mut placed = Vec::new();
    let shell_react = |sh: &mut Shell, bar: &mut Bar, placed: &mut Vec<u32>, msgs: Vec<Message>| {
        shell_mock.push(&msgs);
        for ev in sh.step(0).unwrap() {
            match ev {
                ShellEvent::Opened(id) => sh.focus(id),
                ShellEvent::Request { window, action: proto::WindowAction::Activate, .. } => sh.focus(window),
                _ => {}
            }
        }
        tiling::retile(sh, placed);
        redraw_bar(sh, bar);
        sh.flush().unwrap();
        take(&shell_mock)
    };

    // Notes opens window 1.
    let notes_mock = MockConnection::new();
    notes_mock.state.borrow_mut().idle_timeouts = true;
    notes_mock.push(&[welcome(dw, dh)]);
    s.open(2, 11, 0, "notes");
    let mut notes = samples::notes(Box::new(notes_mock.clone()));
    notes.step(0);
    s.send(2, &take(&notes_mock));
    let b = shell_react(&mut sh, &mut bar, &mut placed, vec![opened(1, 11, "notes", "Notes")]);
    s.send(1, &b);
    let r1 = tiling::layout(&sh.display, 1)[0];
    notes_mock.push(&[focus(1, true), configure(1, 1, r1.2, r1.3, proto::window_states::ACTIVATED)]);
    notes.step(0);
    notes.step(0);
    let notes_first = take(&notes_mock);
    s.send(2, &notes_first);

    // A program with a pixel surface opens window 2.
    s.open(3, 12, 0, "gradient");
    s.send_msgs(3, &[hello(proto::Role::App), surface(1, proto::SurfaceKind::Pixels, proto::SurfaceRole::Window, proto::Layer::None, (0, 0, 200, 120), proto::PixelFormat::Rgb565, "Gradient")]);
    s.pixels(3, 1, &[], &gradient565(200, 120));
    let b = shell_react(&mut sh, &mut bar, &mut placed, vec![opened(2, 12, "gradient", "Gradient")]);
    s.send(1, &b);
    let r2 = tiling::layout(&sh.display, 2);
    s.send_msgs(3, &[ack(1, 1), commit(1)]);
    notes_mock.push(&[focus(1, false), configure(1, 2, r2[0].2, r2[0].3, 0)]);
    notes.step(0);
    notes.step(0);
    let notes_second = take(&notes_mock);
    s.send(2, &notes_second);

    // Halfway through the 120 ms move, then settled.
    s.advance(60);
    s.snap("tiled_moving");
    s.advance(200);
    s.snap("tiled_shell");

    // Click the Notes text field: the compositor hit-tests it, asks the shell to activate the window.
    let all_ops: Vec<TreeOp> = decoded_ops(&notes_first).into_iter().chain(decoded_ops(&notes_second)).collect();
    let field = notes.core.window(1).unwrap().hit_id_of("entry").unwrap();
    let (fx, fy) = hit_point(&all_ops, field);
    let (px, py) = (r2[0].0 + fx, r2[0].1 + fy);
    s.pointer(proto::PointerKind::Move, px, py, 0, proto::Button::None, 2, field);
    s.pointer(proto::PointerKind::Down, px, py, proto::buttons::LEFT, proto::Button::Left, 2, field);
    s.pointer(proto::PointerKind::Up, px, py, 0, proto::Button::Left, 2, field);
    let b = shell_react(&mut sh, &mut bar, &mut placed, vec![Message::WindowRequest(proto::WindowRequest { window: 1, action: proto::WindowAction::Activate, edges: 0 })]);
    s.send(1, &b);
    s.send_msgs(3, &[ack(1, 2), commit(1)]);
    let mut clickmsgs = samples::click(field, fx, fy);
    clickmsgs.insert(0, focus(1, true));
    clickmsgs.push(Message::Text(proto::Text { surface: 1, text: "buy milk".into() }));
    clickmsgs.push(configure(1, 3, r2[0].2, r2[0].3, proto::window_states::ACTIVATED));
    notes_mock.push(&clickmsgs);
    notes.step(0);
    notes.step(0);
    s.send(2, &take(&notes_mock));
    s.advance(200);
    s.snap("notes_typed");

    // The shell goes away: safe mode maximizes the focused window under its bar.
    s.close(1);
    let lw = dw;
    let lh = dh - 14;
    notes_mock.push(&[configure(1, 4, lw, lh, proto::window_states::MAXIMIZED | proto::window_states::ACTIVATED)]);
    notes.step(0);
    notes.step(0);
    s.send(2, &take(&notes_mock));
    s.advance(50);
    s.snap("safe_mode");
    s.out
}

/// The content of a tree window using every node kind and prop combination (§6).
fn primitives_client(s: &mut Session, conn: u32) {
    let white = 0xe6e9eeff;
    let mut t = vec![
        create(1, 0, NodeKind::Rect, vec![Prop::X(0), Prop::Y(0), Prop::W(640), Prop::H(400), Prop::Fill(0x1e2228ff)]),
        create(2, 0, NodeKind::Rect, vec![Prop::X(10), Prop::Y(10), Prop::W(120), Prop::H(60), Prop::Fill(0x3d8bfdff), Prop::Radius(8), Prop::HitId(101)]),
        create(3, 0, NodeKind::Rect, vec![Prop::X(140), Prop::Y(10), Prop::W(120), Prop::H(60), Prop::Fill(0xffffff40), Prop::Radius(12), Prop::BorderW(3), Prop::BorderColor(0xe5534bff)]),
        create(4, 0, NodeKind::Rect, vec![Prop::X(270), Prop::Y(10), Prop::W(120), Prop::H(60), Prop::Fill(0x2a2f37ff), Prop::BorderW(2), Prop::BorderColor(white)]),
        create(5, 0, NodeKind::Group, vec![Prop::X(400), Prop::Y(10), Prop::W(100), Prop::H(60), Prop::Clip(true), Prop::HitId(102)]),
        create(6, 5, NodeKind::Rect, vec![Prop::X(-20), Prop::Y(-20), Prop::W(80), Prop::H(80), Prop::Fill(0x55ff55ff)]),
        create(7, 5, NodeKind::Text, vec![Prop::X(10), Prop::Y(40), Prop::Text("clipped text that runs long".into()), Prop::Size(12), Prop::Color(white), Prop::HitId(103)]),
        create(8, 0, NodeKind::Group, vec![Prop::X(510), Prop::Y(10), Prop::Opacity(128)]),
        create(9, 8, NodeKind::Rect, vec![Prop::X(0), Prop::Y(0), Prop::W(60), Prop::H(60), Prop::Fill(0xff3030ff)]),
        create(10, 8, NodeKind::Rect, vec![Prop::X(30), Prop::Y(30), Prop::W(60), Prop::H(30), Prop::Fill(0x3030ffff)]),
    ];
    let texts: [(i32, proto::Font, u8, &str); 8] = [
        (90, proto::Font::Ui, 8, "ui 8: The quick brown fox jumps over the lazy dog 0123456789"),
        (100, proto::Font::Ui, 12, "ui 12: The quick brown fox jumps over the lazy dog"),
        (114, proto::Font::Ui, 16, "ui 16: The quick brown fox"),
        (132, proto::Font::Ui, 24, "ui 24: Quick fox"),
        (158, proto::Font::Ui, 32, "ui 32: Fox"),
        (192, proto::Font::Mono, 16, "mono 16: fn main() { println!(\"hi\"); }"),
        (210, proto::Font::Mono, 32, "mono 32"),
        (244, proto::Font::Ui, 12, "Latin-1: Café naïve façade Ünïcödé  missing: \u{2605}"),
    ];
    let mut id = 20;
    for (y, font, size, text) in texts {
        t.push(create(id, 0, NodeKind::Text, vec![Prop::X(10), Prop::Y(y), Prop::Text(text.into()), Prop::Font(font), Prop::Size(size), Prop::Color(white)]));
        id += 1;
    }
    t.push(create(40, 0, NodeKind::Group, vec![Prop::X(330), Prop::Y(200), Prop::Scale(2)]));
    t.push(create(41, 40, NodeKind::Rect, vec![Prop::X(0), Prop::Y(0), Prop::W(40), Prop::H(10), Prop::Fill(0xffaa00ff), Prop::Radius(3)]));
    t.push(create(42, 40, NodeKind::Text, vec![Prop::X(0), Prop::Y(12), Prop::Text("x2 group".into()), Prop::Size(8), Prop::Color(white)]));
    // Images: nearest, linear, tinted, a sub-rect.
    t.push(create(50, 0, NodeKind::Image, vec![Prop::X(400), Prop::Y(100), Prop::W(64), Prop::H(64), Prop::Resource(1)]));
    t.push(create(51, 0, NodeKind::Image, vec![Prop::X(480), Prop::Y(100), Prop::W(64), Prop::H(64), Prop::Resource(1), Prop::Sampling(proto::Sampling::Linear)]));
    t.push(create(52, 0, NodeKind::Image, vec![Prop::X(560), Prop::Y(100), Prop::W(64), Prop::H(64), Prop::Resource(1), Prop::Tint(0xff8080ff)]));
    t.push(create(53, 0, NodeKind::Image, vec![Prop::X(400), Prop::Y(180), Prop::W(64), Prop::H(64), Prop::Resource(1), Prop::Src(proto::Rect { x: 4, y: 4, w: 8, h: 8 })]));
    // A canvas at scale 2.
    t.push(create(54, 0, NodeKind::Canvas, vec![Prop::X(480), Prop::Y(180), Prop::W(64), Prop::H(48), Prop::Resource(2), Prop::Scale(2)]));
    // Lines: thin, thick translucent with joints, a single point.
    t.push(create(60, 0, NodeKind::Line, vec![Prop::Points(vec![proto::Point { x: 20, y: 300 }, proto::Point { x: 120, y: 262 }, proto::Point { x: 220, y: 330 }, proto::Point { x: 320, y: 280 }]), Prop::Color(0xffee55ff)]));
    t.push(create(61, 0, NodeKind::Line, vec![Prop::Points(vec![proto::Point { x: 20, y: 350 }, proto::Point { x: 120, y: 310 }, proto::Point { x: 220, y: 380 }, proto::Point { x: 320, y: 330 }]), Prop::Width(5), Prop::Color(0x00ffff80)]));
    t.push(create(62, 0, NodeKind::Line, vec![Prop::Points(vec![proto::Point { x: 340, y: 300 }]), Prop::Width(6), Prop::Color(0xff55ffff)]));

    s.send_msgs(conn, &[
        Message::ResourceCreate(proto::ResourceCreate { id: 1, kind: proto::ResourceKind::Image, w: 16, h: 16, format: proto::PixelFormat::Rgba8888 }),
        Message::ResourceCreate(proto::ResourceCreate { id: 2, kind: proto::ResourceKind::Canvas, w: 32, h: 24, format: proto::PixelFormat::Indexed8 }),
    ]);
    let mut img = Vec::new();
    for y in 0..16u32 {
        for x in 0..16u32 {
            let a = if (x + y) % 5 == 0 { 0x40 } else { 0xff };
            img.extend_from_slice(&[(x * 16) as u8, (y * 16) as u8, (255 - x * 8) as u8, a]);
        }
    }
    s.upload(conn, 1, 1, &img);
    let canvas: Vec<u8> = (0..32 * 24).map(|i| ((i % 32) * 8 ^ (i / 32) * 10) as u8).collect();
    s.pixels(conn, 2, &[], &canvas);
    s.send_msgs(conn, &[ops(1, t), ack(1, 1), commit(1)]);
}

/// Sends `primitives_client` after the shell places window 1 full screen.
fn primitives_session(scale: u32) -> Vec<u8> {
    let mut s = Session::new(640 * scale, 400 * scale, scale);
    s.open(1, 10, 1, "shell");
    s.send_msgs(1, &[hello(proto::Role::Shell)]);
    s.open(2, 11, 0, "primitives");
    s.send_msgs(2, &[hello(proto::Role::App), surface(1, proto::SurfaceKind::Tree, proto::SurfaceRole::Window, proto::Layer::None, (0, 0, 0, 0), proto::PixelFormat::None, "Primitives")]);
    s.send_msgs(1, &[place(1, (0, 0, 640, 400), proto::window_states::FULLSCREEN, 0)]);
    primitives_client(&mut s, 2);
    s.advance(10);
    s.snap(if scale == 1 { "primitives" } else { "primitives_x2" });
    // Hit tests through groups, clips and text boxes (device pixels).
    s.pointer(proto::PointerKind::Move, 50 * scale as i32, 40 * scale as i32, 0, proto::Button::None, 2, 101);
    s.pointer(proto::PointerKind::Move, 415 * scale as i32, 55 * scale as i32, 0, proto::Button::None, 2, 103);
    s.pointer(proto::PointerKind::Move, 480 * scale as i32, 20 * scale as i32, 0, proto::Button::None, 2, 102);
    s.pointer(proto::PointerKind::Move, 520 * scale as i32, 300 * scale as i32, 0, proto::Button::None, 2, 0);
    s.out
}

/// Every layer, overlapping translucent windows, a popup, then an exclusive fullscreen window.
fn layers() -> Vec<u8> {
    let mut s = Session::new(640, 400, 1);
    let white = 0xe6e9eeff;
    s.open(1, 10, 1, "shell");
    s.send_msgs(1, &[
        hello(proto::Role::Shell),
        surface(1, proto::SurfaceKind::Tree, proto::SurfaceRole::Layer, proto::Layer::Background, (0, 0, 0, 0), proto::PixelFormat::None, "wallpaper"),
        surface(2, proto::SurfaceKind::Tree, proto::SurfaceRole::Layer, proto::Layer::Bottom, (0, 360, 640, 40), proto::PixelFormat::None, "dock"),
        surface(3, proto::SurfaceKind::Tree, proto::SurfaceRole::Layer, proto::Layer::Top, (0, 0, 640, 16), proto::PixelFormat::None, "bar"),
        surface(4, proto::SurfaceKind::Tree, proto::SurfaceRole::Layer, proto::Layer::Overlay, (420, 24, 200, 50), proto::PixelFormat::None, "toast"),
    ]);
    let mut wall = Vec::new();
    for i in 0..8 {
        wall.push(create(1 + i, 0, NodeKind::Rect, vec![Prop::X(0), Prop::Y(i as i32 * 50), Prop::W(640), Prop::H(50), Prop::Fill(0x203040ff + (i << 16) * 9 + (i << 8) * 5)]));
    }
    s.send_msgs(1, &[
        ops(1, wall), ack(1, 1), commit(1),
        ops(2, vec![create(1, 0, NodeKind::Rect, vec![Prop::X(0), Prop::Y(0), Prop::W(640), Prop::H(40), Prop::Fill(0x101418e0)]),
                    create(2, 0, NodeKind::Text, vec![Prop::X(8), Prop::Y(14), Prop::Text("dock (bottom layer)".into()), Prop::Color(white)])]),
        ack(2, 1), commit(2),
        ops(3, vec![create(1, 0, NodeKind::Rect, vec![Prop::X(0), Prop::Y(0), Prop::W(640), Prop::H(16), Prop::Fill(0x202733ff)]),
                    create(2, 0, NodeKind::Text, vec![Prop::X(4), Prop::Y(2), Prop::Text("bar (top layer)".into()), Prop::Color(white)])]),
        ack(3, 1), commit(3),
        ops(4, vec![create(1, 0, NodeKind::Rect, vec![Prop::X(0), Prop::Y(0), Prop::W(200), Prop::H(50), Prop::Fill(0x2a2f37f0), Prop::Radius(6), Prop::BorderW(1), Prop::BorderColor(0x6faeffff)]),
                    create(2, 0, NodeKind::Text, vec![Prop::X(10), Prop::Y(8), Prop::Text("Notification".into()), Prop::Size(16), Prop::Color(white)]),
                    create(3, 0, NodeKind::Text, vec![Prop::X(10), Prop::Y(30), Prop::Text("overlay layer, above everything".into()), Prop::Size(8), Prop::Color(0x8c95a3ff)])]),
        ack(4, 1), commit(4),
    ]);
    let app = |s: &mut Session, conn: u32, pid: u32, title: &str, fill: u32| {
        s.open(conn, pid, 0, title);
        s.send_msgs(conn, &[hello(proto::Role::App), surface(1, proto::SurfaceKind::Tree, proto::SurfaceRole::Window, proto::Layer::None, (0, 0, 0, 0), proto::PixelFormat::None, title)]);
        let _ = fill;
    };
    app(&mut s, 2, 21, "Alpha", 0);
    app(&mut s, 3, 22, "Beta", 0);
    s.send_msgs(1, &[place(1, (40, 40, 300, 200), proto::window_states::ACTIVATED, 0), place(2, (200, 120, 300, 200), 0, 0),
                     Message::SetOpacity(proto::SetOpacity { window: 2, alpha: 160, transition: proto::Transition { ms: 0, easing: proto::Easing::Linear } })]);
    let body = |title: &str, fill: u32| {
        vec![create(1, 0, NodeKind::Rect, vec![Prop::X(0), Prop::Y(0), Prop::W(300), Prop::H(200), Prop::Fill(fill), Prop::Radius(6)]),
             create(2, 0, NodeKind::Rect, vec![Prop::X(0), Prop::Y(0), Prop::W(300), Prop::H(18), Prop::Fill(0x00000060)]),
             create(3, 0, NodeKind::Text, vec![Prop::X(6), Prop::Y(3), Prop::Text(title.into()), Prop::Color(white)]),
             create(4, 0, NodeKind::Text, vec![Prop::X(6), Prop::Y(30), Prop::Text("window content".into()), Prop::Size(16), Prop::Color(white)])]
    };
    s.send_msgs(2, &[ops(1, body("Alpha (focused)", 0x3a6ea5ff)), ack(1, 1), commit(1)]);
    s.send_msgs(3, &[ops(1, body("Beta (opacity 160)", 0xa5503aff)), ack(1, 1), commit(1)]);
    // A popup menu of Alpha.
    s.send_msgs(2, &[surface_child(2, proto::SurfaceKind::Tree, proto::SurfaceRole::Popup, proto::Layer::None, 1, (20, 60, 120, 52), proto::PixelFormat::None, "menu")]);
    s.send_msgs(2, &[ops(2, vec![create(1, 0, NodeKind::Rect, vec![Prop::X(0), Prop::Y(0), Prop::W(120), Prop::H(52), Prop::Fill(0x2a2f37ff), Prop::BorderW(1), Prop::BorderColor(0x454c58ff)]),
                                 create(2, 0, NodeKind::Text, vec![Prop::X(8), Prop::Y(6), Prop::Text("Open".into()), Prop::Color(white)]),
                                 create(3, 0, NodeKind::Rect, vec![Prop::X(1), Prop::Y(22), Prop::W(118), Prop::H(16), Prop::Fill(0x3d8bfdff)]),
                                 create(4, 0, NodeKind::Text, vec![Prop::X(8), Prop::Y(24), Prop::Text("Save".into()), Prop::Color(white)])]),
                     ack(2, 1), commit(2)]);
    s.advance(10);
    s.snap("layers");
    // The popup goes, and a pixel window goes exclusive fullscreen: only it and the overlay show.
    s.send_msgs(2, &[Message::SurfaceDestroy(proto::SurfaceDestroy { surface: 2 })]);
    s.open(4, 23, 0, "game");
    s.send_msgs(4, &[hello(proto::Role::App), surface(1, proto::SurfaceKind::Pixels, proto::SurfaceRole::Window, proto::Layer::None, (0, 0, 320, 200), proto::PixelFormat::Rgb565, "Game")]);
    s.pixels(4, 1, &[], &gradient565(320, 200));
    s.send_msgs(1, &[place(3, (0, 0, 640, 400), proto::window_states::FULLSCREEN | proto::window_states::ACTIVATED, 0),
                     Message::Stack(proto::Stack { windows: vec![1, 2, 3] })]);
    s.send_msgs(4, &[ack(1, 1), commit(1)]);
    s.advance(10);
    s.snap("fullscreen");
    // Damage only: one pixel row band changes.
    let mut px = gradient565(320, 200);
    for x in 0..320 * 2 {
        px[100 * 320 * 2 + x] = 0xff;
    }
    s.pixels(4, 1, &[[0, 100, 320, 1]], &px);
    s.send_msgs(4, &[commit(1)]);
    s.advance(10);
    s.snap("fullscreen_damage");
    s.out
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "../src/test/resources/ui-sessions".into());
    std::fs::create_dir_all(&dir).unwrap();
    let sessions: Vec<(&str, Vec<u8>)> = vec![
        ("tiled_shell", tiled_shell()),
        ("primitives", primitives_session(1)),
        ("primitives_x2", primitives_session(2)),
        ("layers", layers()),
    ];
    for (name, bytes) in sessions {
        let path = format!("{dir}/{name}.session");
        std::fs::write(&path, &bytes).unwrap();
        println!("wrote {path} ({} bytes)", bytes.len());
    }
}
