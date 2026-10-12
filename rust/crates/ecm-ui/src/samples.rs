//! Sample apps and scripted sessions shared by tests (the Rust half of the
//! Rust/Python parity test, ROADMAP P3 "Done when").

use crate::conn::Connection;
use crate::element::{button, column, label, list, row, text_field};
use crate::json::Json;
use crate::App;
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use std::cell::RefCell;
use std::rc::Rc;

/// The §13 "Notes" example in Rust. Mirrors `NOTES_PY` line by line.
pub fn notes(conn: Box<dyn Connection>) -> App {
    let mut app = App::with_connection("dev.evan.notes", conn).expect("connect");
    let saved: Vec<String> = app
        .restore_json()
        .get("notes")
        .and_then(Json::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let notes = Rc::new(RefCell::new(saved));

    let add = {
        let notes = notes.clone();
        Rc::new(move |cx: &mut crate::Ctx| {
            let text = cx.text("entry");
            if !text.is_empty() {
                notes.borrow_mut().push(text);
                cx.set_text("entry", "");
            }
        })
    };

    let view_notes = notes.clone();
    let click = add.clone();
    app.window("Notes", move |_| {
        column()
            .gap(8)
            .padding(12)
            .child(
                row()
                    .gap(8)
                    .child(text_field().id("entry").placeholder("New note"))
                    .child(button("Add").on_click({
                        let click = click.clone();
                        move |cx, _| click(cx)
                    })),
            )
            .child(list().children(view_notes.borrow().iter().map(label)))
    });
    let hot = add.clone();
    app.hotkey(1, "Add note", move |cx| hot(cx));
    let saved = notes.clone();
    app.on_suspend_json(move || {
        Json::Obj(vec![("notes".into(), Json::Arr(saved.borrow().iter().map(|s| Json::Str(s.clone())).collect()))])
    });
    app
}

/// The same app in Python (`import ui`), as in UI_PROTOCOL §13.
pub const NOTES_PY: &str = r#"
import ui

app = ui.App("dev.evan.notes")
notes = app.restore().get("notes", [])
entry = ui.TextField(placeholder="New note")
rows = ui.List(items=[ui.Label(n) for n in notes])

def add(_=None):
    if entry.text:
        notes.append(entry.text)
        entry.text = ""
        rows.items = [ui.Label(n) for n in notes]

app.window("Notes", ui.Column(gap=8, padding=12, children=[
    ui.Row(gap=8, children=[entry,
        ui.Button("Add", on_click=add)]),
    rows,
]))
app.hotkey(1, "Add note", add)
app.on_suspend(lambda: {"notes": notes})
app.run()
"#;

/// A Welcome for tests (Terminal tier 2, default theme).
pub fn welcome(restore_follows: bool) -> Message {
    Message::Welcome(proto::Welcome {
        version: proto::VERSION,
        app_id: "dev.evan.notes".into(),
        display: proto::DisplayInfo {
            w: 640,
            h: 400,
            scale: 1,
            size_class: proto::SizeClass::Medium,
            tier: 2,
            device: proto::Device::Terminal,
        },
        theme: crate::Theme::default().to_wire(),
        font_metrics: proto::fonts::METRICS_VERSION,
        capabilities: Vec::new(),
        restore_follows,
    })
}

fn pointer(kind: proto::PointerKind, hit_id: u32, x: i32, y: i32) -> Message {
    Message::Pointer(proto::Pointer {
        surface: 1,
        kind,
        x,
        y,
        buttons: if kind == proto::PointerKind::Down { proto::buttons::LEFT } else { 0 },
        button: if matches!(kind, proto::PointerKind::Down | proto::PointerKind::Up) { proto::Button::Left } else { proto::Button::None },
        scroll_x: 0,
        scroll_y: 0,
        hit_id,
    })
}

/// A click: down and up on the same hit id.
pub fn click(hit_id: u32, x: i32, y: i32) -> Vec<Message> {
    vec![pointer(proto::PointerKind::Down, hit_id, x, y), pointer(proto::PointerKind::Up, hit_id, x, y)]
}

/// The scripted session both notes apps run: restore one note, configure,
/// type and click Add, type and press hotkey 1, suspend, change the theme.
/// Each inner vector is what one `recv` returns.
pub fn notes_session(field_hit: u32, add_hit: u32) -> Vec<Vec<u8>> {
    let mut night = crate::Theme::default();
    night.accent = 0xff8800ff;
    night.name = "night".into();
    let chunks: Vec<Vec<Message>> = vec![
        vec![welcome(true), Message::Restore(proto::Restore { data: br#"{"notes":["first"]}"#.to_vec() })],
        vec![Message::Configure(proto::Configure {
            surface: 1,
            serial: 7,
            w: 400,
            h: 300,
            states: proto::window_states::ACTIVATED,
        })],
        click(field_hit, 30, 20),
        vec![Message::Text(proto::Text { surface: 1, text: "milk".into() })],
        click(add_hit, 380, 20),
        vec![Message::Text(proto::Text { surface: 1, text: "eggs \u{e9}".into() })],
        vec![Message::Hotkey(proto::Hotkey { slot: 1, down: true }), Message::Hotkey(proto::Hotkey { slot: 1, down: false })],
        vec![Message::WillSuspend(proto::WillSuspend { deadline_ms: 500 })],
        vec![Message::ThemeChanged(proto::ThemeChanged { theme: night.to_wire() })],
        vec![Message::Configure(proto::Configure { surface: 1, serial: 8, w: 300, h: 200, states: 0 })],
    ];
    chunks.iter().map(|c| proto::encode_all(c)).collect()
}
