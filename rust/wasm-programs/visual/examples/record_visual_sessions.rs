//! Records the Visual Programmer sessions that the Java golden-image test
//! (UiGoldenTest) replays through the compositor and rasterizer:
//!
//!   cargo run -p visual --example record_visual_sessions -- <out dir>
//!   (the checked-in copies live in src/test/resources/ui-sessions/)
//!
//! Same file format as ecm-ui's `record_ui_sessions` (see there). A shell
//! places the window full screen; the editor is the real app driven through
//! a mock connection.
//!
//! * `visual_program` (640x400): a program with a long peripheral block
//!   (two-line title, cut with "..."), Set Redstone's option field, the
//!   palette with its folded sections (Redstone opened by a click on its
//!   header) and its `?` buttons.
//! * then `visual_info`: the "more information" popup of Set Redstone, opened
//!   with its `?` in the palette.

use ecm_ui::{samples as ui_samples, App, Json, MockConnection};
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use ecm_visual::blocks::{catalog_from_json, PeripheralMethod, PeripheralType, Registry};
use ecm_visual::editor::Editor;
use ecm_visual::files::MemFiles;
use ecm_visual::graph::{Conn, PortRef};
use ecm_visual::runner::MockRunner;
use ecm_visual::view;
use std::cell::RefCell;
use std::rc::Rc;

struct Session {
    out: Vec<u8>,
}

impl Session {
    fn new(w: u32, h: u32) -> Session {
        let mut out = b"ECMUISS1".to_vec();
        for v in [w, h, 1] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        Session { out }
    }
    fn u32(&mut self, v: u32) {
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
    fn advance(&mut self, ms: u32) {
        self.out.push(5);
        self.u32(ms);
    }
    fn snap(&mut self, name: &str) {
        self.out.push(6);
        self.out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.out.extend_from_slice(name.as_bytes());
    }
    fn flush_app(&mut self, conn: u32, mock: &MockConnection) {
        let sent = mock.sent();
        mock.clear_sent();
        self.send(conn, &sent);
    }
}

fn shell(s: &mut Session) {
    s.open(1, 10, 1, "shell");
    s.send(1, &proto::encode_all(&[Message::Hello(proto::Hello { version: proto::VERSION, role: proto::Role::Shell, capabilities: vec![] })]));
}

fn place_full(s: &mut Session, w: i32, h: i32) {
    s.send(
        1,
        &proto::encode_all(&[Message::Place(proto::Place {
            window: 1,
            x: 0,
            y: 0,
            w,
            h,
            states: proto::window_states::FULLSCREEN,
            transition: proto::Transition { ms: 0, easing: proto::Easing::Linear },
        })]),
    );
}

fn registry() -> Registry {
    let mut r = Registry::builtin();
    r.add_peripherals(&catalog_from_json(&Json::parse(include_str!("../tests/vpl/peripheral_catalog.json")).unwrap()).unwrap());
    r.add_peripherals(&[PeripheralType {
        name: "redstone_link".into(),
        description: "Create Redstone Link".into(),
        methods: vec![PeripheralMethod {
            name: "set_signal".into(),
            description: "Set the signal strength sent on this link's frequency pair, from 0 to 15 (0 turns it off)".into(),
            params: vec![("strength".into(), "number".into(), "15".into())],
            returns: String::new(),
        }],
    }]);
    r
}

fn click(app: &mut App, m: &MockConnection, id: &str) {
    let hit = app.core.window(1).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no widget {id}"));
    m.push(&ui_samples::click(hit, 10, 10));
}

fn session() -> Vec<u8> {
    let (w, h) = (640, 400);
    let mut s = Session::new(w as u32, h as u32);
    shell(&mut s);
    let m = MockConnection::new();
    m.state.borrow_mut().idle_timeouts = true;
    let mut wl = ui_samples::welcome(false);
    if let Message::Welcome(ref mut x) = wl {
        x.display.w = w;
        x.display.h = h;
        x.display.size_class = proto::SizeClass::for_width(w);
    }
    m.push(&[wl]);
    s.open(2, 11, 0, "visual");
    let ed = Rc::new(RefCell::new(Editor::new(registry(), Box::new(MemFiles::default()), Box::new(MockRunner::default()))));
    {
        let mut e = ed.borrow_mut();
        e.graph.name = "lamp".into();
        let reg = e.reg.clone();
        let start = e.graph.add_block(reg.find("start").unwrap(), 10.0, 10.0);
        let print = e.graph.add_block(reg.find("print").unwrap(), 10.0, 80.0);
        let link = e.graph.add_block(reg.find("redstone_link_set_signal").unwrap(), 180.0, 10.0);
        let red = e.graph.add_block(reg.find("set_redstone").unwrap(), 10.0, 190.0);
        e.graph.block_mut(print).unwrap().set("text", "Lights on");
        e.graph.conns.push(Conn { from: start, from_port: "flow".into(), to: print, to_port: "flow".into() });
        e.graph.conns.push(Conn { from: print, from_port: "flow".into(), to: red, to_port: "flow".into() });
        let _ = (link, PortRef::parse("port:1:o:flow"));
        e.graph.offset = (8.0, 8.0);
        e.selected = Some(link);
    }
    let mut app = App::with_connection(ecm_visual::APP_ID, Box::new(m.clone())).unwrap();
    view::install(&mut app, ed.clone());
    app.core.flush().unwrap();
    s.flush_app(2, &m);
    place_full(&mut s, w, h);
    m.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w, h, states: proto::window_states::FULLSCREEN })]);
    app.step(0);
    app.step(0);
    // Open the folded Redstone section with a click on its header.
    click(&mut app, &m, "cat:Redstone");
    app.step(0);
    app.step(0);
    s.flush_app(2, &m);
    s.advance(20);
    s.snap("visual_program");
    // The ? beside Set Redstone in the palette.
    click(&mut app, &m, "pinfo:set_redstone");
    app.step(0);
    app.step(0);
    s.flush_app(2, &m);
    s.advance(20);
    s.snap("visual_info");
    s.out
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "../src/test/resources/ui-sessions".into());
    std::fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in [("visual_program", session())] {
        let path = format!("{dir}/{name}.session");
        std::fs::write(&path, &bytes).unwrap();
        println!("wrote {path} ({} bytes)", bytes.len());
    }
}
