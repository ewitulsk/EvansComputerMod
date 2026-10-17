//! `_ui` native module: the Lua `ui` module (lua/ui.lua) over the same
//! `ecm-ui` engine Rust apps and the Python `ui` module use. Lua keeps the
//! widget objects and calls the handlers; layout, text wrapping, hit routing,
//! focus and the TreeOps diff all run here, so a Lua app and the same app in
//! Rust send the same bytes (tests.rs checks that).
//!
//! Apps are kept in a per-thread table and addressed by integer handles.
//! Widget trees cross over as nested tables in the `.ui.json` form, with
//! integer handler references that ui.lua maps back to functions.

use crate::values::{bytes, to_json, to_lua};
use ecm_ui::conn::{default_connection, Connection, MockConnection};
use ecm_ui::{AppEvent, Core, EventValue, Handler, Json};
use ecm_ui_proto as proto;
use mlua::{Lua, MultiValue, Table, Value};
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Default)]
struct Apps {
    next: i64,
    apps: HashMap<i64, Core>,
    mocks: HashMap<i64, MockConnection>,
    /// Connect through this mock instead of the UI fd (tests).
    use_mock: Option<i64>,
}

thread_local! {
    static TABLE: RefCell<Apps> = RefCell::new(Apps::default());
}

fn err(msg: impl Into<String>) -> mlua::Error {
    mlua::Error::runtime(msg.into())
}

fn with_app<T>(h: i64, f: impl FnOnce(&mut Core) -> T) -> mlua::Result<T> {
    TABLE.with(|t| match t.borrow_mut().apps.get_mut(&h) {
        Some(c) => Ok(f(c)),
        None => Err(err(format!("no ui app {h}"))),
    })
}

fn tree(v: &Value) -> mlua::Result<ecm_ui::Element> {
    let j = to_json(v, 0)?;
    ecm_ui::layout_file::element(&j, "root").map_err(err)
}

/// An event as a list: {"widget", window, handler, kind, value}, {"key", window,
/// key, mods, down}, {"hotkey", slot, down}, {"suspend", deadline_ms},
/// {"frame", window, time_ms}, {"close", window}, {"theme"}, {"message", name, json}.
/// A handler that isn't a reference or a name is `false`.
fn event_table(lua: &Lua, ev: AppEvent) -> mlua::Result<Table> {
    let s = |x: &str| -> mlua::Result<Value> { Ok(Value::String(lua.create_string(x)?)) };
    let i = |x: i64| Value::Number(x as f64);
    let items: Vec<Value> = match ev {
        AppEvent::Widget(e) => {
            let handler = match &e.handler {
                Handler::Ref(r) => i(*r),
                Handler::Named(n) => s(n)?,
                Handler::Callback(_) => Value::Boolean(false),
            };
            let value = match &e.value {
                EventValue::None => Value::Nil,
                EventValue::Bool(b) => Value::Boolean(*b),
                EventValue::Int(v) => i(*v as i64),
                EventValue::Text(t) => s(t)?,
                EventValue::Index(x) => i(*x as i64),
                EventValue::Pointer(p) => {
                    let d = lua.create_table()?;
                    let kind = match p.kind {
                        proto::PointerKind::Down => "down",
                        proto::PointerKind::Up => "up",
                        proto::PointerKind::Move => "move",
                        proto::PointerKind::Scroll => "scroll",
                        proto::PointerKind::Enter => "enter",
                        proto::PointerKind::Leave => "leave",
                    };
                    d.set("kind", kind)?;
                    d.set("x", p.x as f64)?;
                    d.set("y", p.y as f64)?;
                    d.set("button", p.button.code() as f64)?;
                    d.set("buttons", p.buttons as f64)?;
                    d.set("scroll", p.scroll_y as f64)?;
                    d.set("target", p.target.clone())?;
                    Value::Table(d)
                }
                EventValue::Key { key, mods, down, repeat } => {
                    let d = lua.create_table()?;
                    d.set("key", *key as f64)?;
                    d.set("mods", *mods as f64)?;
                    d.set("down", *down)?;
                    d.set("repeat", *repeat)?;
                    Value::Table(d)
                }
                // Pixel grids: the changed texels as {x, y, index} lists.
                EventValue::Texels(ts) => {
                    let out = lua.create_table_with_capacity(ts.len(), 0)?;
                    for (n, t) in ts.iter().enumerate() {
                        let e = lua.create_table_with_capacity(3, 0)?;
                        e.raw_set(1, t[0] as f64)?;
                        e.raw_set(2, t[1] as f64)?;
                        e.raw_set(3, t[2] as f64)?;
                        out.raw_set(n + 1, e)?;
                    }
                    Value::Table(out)
                }
            };
            vec![s("widget")?, i(e.window as i64), handler, s(e.kind.name())?, value]
        }
        AppEvent::Key { window, key, mods, down, .. } => {
            vec![s("key")?, i(window as i64), i(key as i64), i(mods as i64), Value::Boolean(down)]
        }
        AppEvent::Hotkey { slot, down } => vec![s("hotkey")?, i(slot as i64), Value::Boolean(down)],
        AppEvent::Suspend { deadline_ms } => vec![s("suspend")?, i(deadline_ms as i64)],
        AppEvent::Frame { window, time_ms } => vec![s("frame")?, i(window as i64), i(time_ms as i64)],
        AppEvent::CloseRequested { window } => vec![s("close")?, i(window as i64)],
        AppEvent::ThemeChanged => vec![s("theme")?],
        AppEvent::Message(m) => vec![s("message")?, s(m.name())?, s(&m.to_json())?],
    };
    let t = lua.create_table_with_capacity(items.len(), 0)?;
    for (n, v) in items.into_iter().enumerate() {
        t.raw_set(n + 1, v)?;
    }
    Ok(t)
}

fn add_mock(m: MockConnection) -> i64 {
    TABLE.with(|t| {
        let mut t = t.borrow_mut();
        t.next += 1;
        let h = t.next;
        t.mocks.insert(h, m);
        h
    })
}

pub fn module(lua: &Lua) -> mlua::Result<Table> {
    let m = lua.create_table()?;

    // Connect to the compositor (role 0 app, 1 shell); returns a handle.
    m.set("connect", lua.create_function(|_, (app_id, role): (String, i32)| {
        let mock = TABLE.with(|t| {
            let t = t.borrow();
            t.use_mock.and_then(|m| t.mocks.get(&m).cloned())
        });
        let conn: Box<dyn Connection> = match mock {
            Some(m) => Box::new(m),
            None => default_connection(role).map_err(|e| err(format!("ui: {e}")))?,
        };
        let role = if role == 1 { proto::Role::Shell } else { proto::Role::App };
        let core = Core::connect(&app_id, conn, role).map_err(|e| err(format!("ui: {e}")))?;
        Ok(TABLE.with(|t| {
            let mut t = t.borrow_mut();
            t.next += 1;
            let h = t.next;
            t.apps.insert(h, core);
            h
        }))
    })?)?;

    // Saved state from the previous run (a string), or nil.
    m.set("restore", lua.create_function(|lua, h: i64| {
        match with_app(h, |c| c.restore().map(|b| b.to_vec()))? {
            Some(b) => Ok(Value::String(lua.create_string(b)?)),
            None => Ok(Value::Nil),
        }
    })?)?;

    // Create a window showing `tree`; returns its id.
    m.set("window", lua.create_function(|_, (h, title, t): (i64, String, Value)| {
        let root = tree(&t)?;
        with_app(h, |c| c.create_window(&title, ecm_ui::window::Content::Static(root)))
    })?)?;
    // Replace a window's widget tree.
    m.set("set_root", lua.create_function(|_, (h, win, t): (i64, u32, Value)| {
        let root = tree(&t)?;
        with_app(h, |c| c.set_root(win, root))
    })?)?;
    m.set("close_window", lua.create_function(|_, (h, win): (i64, u32)| with_app(h, |c| c.close_window(win)))?)?;

    // width, height, size class name of a window (0, 0 before it is configured).
    m.set("window_size", lua.create_function(|_, (h, win): (i64, u32)| {
        with_app(h, |c| c.window(win).map_or((0, 0, "compact"), |w| (w.width, w.height, w.class().name())))
    })?)?;

    // The state of the widget with this id in a window:
    // {text, checked, value, selected, pixels} (nil if it has none yet).
    m.set("widget_state", lua.create_function(|lua, (h, win, id): (i64, u32, String)| {
        let Some(st) = with_app(h, |c| c.window(win).and_then(|w| w.state(&id).cloned()))? else {
            return Ok(Value::Nil);
        };
        let d = lua.create_table()?;
        d.set("text", st.text.as_str())?;
        d.set("checked", st.checked)?;
        d.set("value", st.value as f64)?;
        d.set("selected", st.selected.map(|x| x as f64))?;
        let px = lua.create_table_with_capacity(st.pixels.len(), 0)?;
        for (n, p) in st.pixels.iter().enumerate() {
            px.raw_set(n + 1, *p as f64)?;
        }
        px.set_metatable(Some(crate::values::array_meta(lua)?))?;
        d.set("pixels", px)?;
        Ok(Value::Table(d))
    })?)?;

    m.set("hotkey", lua.create_function(|_, (h, slot, label): (i64, u8, String)| {
        if !(1..=6).contains(&slot) {
            return Err(err("hotkey slots are 1-6"));
        }
        with_app(h, |c| c.claim_hotkey(slot, &label))
    })?)?;

    // Render changed windows and send everything queued. false once closed.
    m.set("render", lua.create_function(|_, h: i64| with_app(h, |c| c.render().is_ok()))?)?;
    m.set("flush", lua.create_function(|_, h: i64| with_app(h, |c| c.flush().is_ok()))?)?;

    // Wait up to `timeout_ms` (< 0 forever) and return the events, or nil once closed.
    m.set("step", lua.create_function(|lua, (h, timeout_ms): (i64, i32)| {
        match with_app(h, |c| c.step(timeout_ms))? {
            Ok(evs) => {
                let out = lua.create_table()?;
                for (i, e) in evs.into_iter().enumerate() {
                    out.raw_set(i + 1, event_table(lua, e)?)?;
                }
                Ok(Value::Table(out))
            }
            Err(_) => Ok(Value::Nil),
        }
    })?)?;

    m.set("is_closed", lua.create_function(|_, h: i64| with_app(h, |c| c.is_closed()))?)?;
    m.set("quit", lua.create_function(|_, h: i64| with_app(h, |c| c.quit()))?)?;

    // Answer WillSuspend with the bytes to keep.
    m.set("save_state", lua.create_function(|_, (h, data): (i64, Value)| {
        let data = bytes(&data, "state")?;
        with_app(h, |c| c.save_state(data))
    })?)?;

    m.set("request_frame", lua.create_function(|_, (h, win): (i64, u32)| {
        with_app(h, |c| c.send(proto::Message::RequestFrame(proto::RequestFrame { surface: win })))
    })?)?;

    // Send a protocol message given as (name, fields): the services
    // (Notify, ClipboardSet, PickFile, Open, Launch, BackgroundDone, ...).
    m.set("send", lua.create_function(|_, (h, name, fields): (i64, String, Value)| {
        let f = to_json(&fields, 0)?;
        let s = |k: &str| f.get(k).and_then(Json::as_str).unwrap_or("").to_string();
        let n = |k: &str| f.get(k).and_then(Json::as_i64).unwrap_or(0);
        let list = |k: &str| -> Vec<String> {
            f.get(k).and_then(Json::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
        };
        let bytes = |k: &str| -> Vec<u8> { f.get(k).and_then(Json::as_str).map(|x| x.as_bytes().to_vec()).unwrap_or_default() };
        use proto::Message as M;
        let m = match name.as_str() {
            "Notify" => M::Notify(proto::Notify { tag: s("tag"), title: s("title"), body: s("body"), icon: s("icon"), actions: list("actions") }),
            "ClipboardSet" => M::ClipboardSet(proto::ClipboardSet { mime: s("mime"), data: bytes("data") }),
            "ClipboardRequest" => M::ClipboardRequest(proto::ClipboardRequest { mime: s("mime") }),
            "PickFile" => M::PickFile(proto::PickFile {
                req: n("req") as u32,
                mode: if s("mode") == "save" { proto::PickMode::Save } else { proto::PickMode::Open },
                types: list("types"),
                suggested: s("suggested"),
            }),
            "Open" => M::Open(proto::Open { path: s("path") }),
            "Launch" => M::Launch(proto::Launch { app_id: s("app_id"), args: list("args") }),
            "BackgroundDone" => M::BackgroundDone(proto::BackgroundDone),
            "PointerLock" => M::PointerLock(proto::PointerLock { surface: n("surface") as u32, lock: f.get("lock").and_then(Json::as_bool).unwrap_or(false) }),
            other => return Err(err(format!("ui.send: unsupported message {other:?}"))),
        };
        // Reject what the compositor would (limits) before it closes us.
        proto::Message::decode_exact(&m.to_bytes()).map_err(|e| err(format!("ui.send: {e}")))?;
        with_app(h, |c| c.send(m))
    })?)?;

    // Canonical JSON text of a Lua value (what Rust apps write too), and back.
    m.set("json_dumps", lua.create_function(|_, v: Value| Ok(to_json(&v, 0)?.to_string()))?)?;
    m.set("json_loads", lua.create_function(|lua, text: mlua::LuaString| {
        let text = text.to_str().map_err(|_| err("not UTF-8"))?.to_string();
        let j = Json::parse(&text).map_err(err)?;
        to_lua(lua, &j)
    })?)?;
    // A table marked as a JSON list (so an empty one stays `[]`).
    m.set("array", lua.create_function(|lua, t: Option<Table>| {
        let t = match t {
            Some(t) => t,
            None => lua.create_table()?,
        };
        t.set_metatable(Some(crate::values::array_meta(lua)?))?;
        Ok(t)
    })?)?;

    // ---- test support: a scripted connection instead of the UI fd

    // A mock connection that returns these byte chunks from successive recvs.
    m.set("mock_new", lua.create_function(|_, chunks: Table| {
        let m = MockConnection::new();
        for c in chunks.sequence_values::<Value>() {
            m.push_bytes(bytes(&c?, "chunk")?);
        }
        Ok(add_mock(m))
    })?)?;
    // Make `connect` use this mock (nil = the real UI fd again).
    m.set("use_mock", lua.create_function(|_, h: Option<i64>| {
        TABLE.with(|t| t.borrow_mut().use_mock = h);
        Ok(())
    })?)?;
    // Everything sent through a mock.
    m.set("mock_sent", lua.create_function(|lua, h: i64| {
        match TABLE.with(|t| t.borrow().mocks.get(&h).map(|m| m.sent())) {
            Some(b) => Ok(MultiValue::from_iter([Value::String(lua.create_string(b)?)])),
            None => Err(err(format!("no mock {h}"))),
        }
    })?)?;

    Ok(m)
}

/// Register a mock connection a Rust test keeps a handle to; returns the
/// handle for `_ui.use_mock`.
pub fn register_mock(m: MockConnection) -> i64 {
    add_mock(m)
}

/// Run `f` on a Lua app's engine (Rust-side tests: hit ids, state).
pub fn with_core<T>(h: i64, f: impl FnOnce(&mut Core) -> T) -> Option<T> {
    TABLE.with(|t| t.borrow_mut().apps.get_mut(&h).map(f))
}
