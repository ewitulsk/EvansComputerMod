//! `_ui` native module: the Python `ui` module (ui.py) over the same
//! `ecm-ui` engine Rust apps use. Python keeps the widget objects and calls
//! the handlers; layout, text wrapping, hit routing, focus and the TreeOps
//! diff all run here, so a Python app and the same app in Rust send the same
//! bytes (tests.rs checks that).
//!
//! Apps are kept in a per-thread table and addressed by integer handles.
//! Widget trees cross over as nested dicts in the `.ui.json` form, with
//! integer handler references that ui.py maps back to callables.

use ecm_ui::conn::{default_connection, Connection, MockConnection};
use ecm_ui::{AppEvent, Core, EventValue, Handler, Json};
use ecm_ui_proto as proto;
use rustpython_vm::{
    builtins::{PyBytes, PyDict, PyFloat, PyInt, PyList, PyStr, PyTuple},
    pymodule, AsObject, PyObjectRef, PyResult, VirtualMachine,
};
use std::cell::RefCell;
use std::collections::HashMap;

/// Source of the Python-level `ui` module.
pub const UI_PY: &str = include_str!("ui.py");

#[derive(Default)]
struct Table {
    next: i64,
    apps: HashMap<i64, Core>,
    mocks: HashMap<i64, MockConnection>,
    /// Connect through this mock instead of the UI fd (tests).
    use_mock: Option<i64>,
}

thread_local! {
    static TABLE: RefCell<Table> = RefCell::new(Table::default());
}

fn err(vm: &VirtualMachine, msg: impl Into<String>) -> rustpython_vm::builtins::PyBaseExceptionRef {
    vm.new_runtime_error(msg.into())
}

fn with_app<T>(h: i64, vm: &VirtualMachine, f: impl FnOnce(&mut Core) -> T) -> PyResult<T> {
    TABLE.with(|t| match t.borrow_mut().apps.get_mut(&h) {
        Some(c) => Ok(f(c)),
        None => Err(err(vm, format!("no ui app {h}"))),
    })
}

const MAX_DEPTH: usize = 64;

/// A Python value (None, bool, int, float, str, list, tuple, dict) as JSON.
pub fn to_json(obj: &PyObjectRef, vm: &VirtualMachine, depth: usize) -> PyResult<Json> {
    if depth > MAX_DEPTH {
        return Err(vm.new_value_error("value nested too deeply".to_owned()));
    }
    if vm.is_none(obj) {
        return Ok(Json::Null);
    }
    if obj.class().is(vm.ctx.types.bool_type) {
        return Ok(Json::Bool(obj.is(&vm.ctx.true_value)));
    }
    if let Some(i) = obj.payload::<PyInt>() {
        return Ok(Json::Int(i.try_to_primitive::<i64>(vm)?));
    }
    if let Some(f) = obj.payload::<PyFloat>() {
        return Ok(Json::Float(f.to_f64()));
    }
    if let Some(s) = obj.payload::<PyStr>() {
        return Ok(Json::Str(s.as_str().to_owned()));
    }
    if let Some(l) = obj.payload::<PyList>() {
        let items: Vec<PyObjectRef> = l.borrow_vec().to_vec();
        return items.iter().map(|o| to_json(o, vm, depth + 1)).collect::<PyResult<_>>().map(Json::Arr);
    }
    if let Some(t) = obj.payload::<PyTuple>() {
        return t.as_slice().iter().map(|o| to_json(o, vm, depth + 1)).collect::<PyResult<_>>().map(Json::Arr);
    }
    if let Some(d) = obj.payload::<PyDict>() {
        let mut kv = Vec::new();
        for (k, v) in d {
            let Some(k) = k.payload::<PyStr>() else {
                return Err(vm.new_type_error("dict keys must be strings".to_owned()));
            };
            kv.push((k.as_str().to_owned(), to_json(&v, vm, depth + 1)?));
        }
        return Ok(Json::Obj(kv));
    }
    Err(vm.new_type_error(format!(
        "can't convert a '{}' (use None, bool, int, float, str, list, tuple or dict)",
        obj.class().name()
    )))
}

pub fn to_py(j: &Json, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    Ok(match j {
        Json::Null => vm.ctx.none(),
        Json::Bool(b) => vm.ctx.new_bool(*b).into(),
        Json::Int(i) => vm.ctx.new_int(*i).into(),
        Json::Float(f) => vm.ctx.new_float(*f).into(),
        Json::Str(s) => vm.ctx.new_str(s.as_str()).into(),
        Json::Arr(a) => {
            let items = a.iter().map(|x| to_py(x, vm)).collect::<PyResult<Vec<_>>>()?;
            vm.ctx.new_list(items).into()
        }
        Json::Obj(kv) => {
            let d = vm.ctx.new_dict();
            for (k, v) in kv {
                d.set_item(k.as_str(), to_py(v, vm)?, vm)?;
            }
            d.into()
        }
    })
}

fn tree(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<ecm_ui::Element> {
    let j = to_json(obj, vm, 0)?;
    ecm_ui::layout_file::element(&j, "root").map_err(|e| vm.new_value_error(e))
}

fn bytes_of(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    if let Some(b) = obj.payload::<PyBytes>() {
        return Ok(b.as_bytes().to_vec());
    }
    if let Some(s) = obj.payload::<PyStr>() {
        return Ok(s.as_str().as_bytes().to_vec());
    }
    Err(vm.new_type_error(format!("expected bytes, got {}", obj.class().name())))
}

fn event_tuple(ev: AppEvent, vm: &VirtualMachine) -> PyObjectRef {
    let s = |x: &str| -> PyObjectRef { vm.ctx.new_str(x).into() };
    let i = |x: i64| -> PyObjectRef { vm.ctx.new_int(x).into() };
    let items: Vec<PyObjectRef> = match ev {
        AppEvent::Widget(e) => {
            let handler = match &e.handler {
                Handler::Ref(r) => i(*r),
                Handler::Named(n) => s(n),
                Handler::Callback(_) => vm.ctx.none(),
            };
            let value = match &e.value {
                EventValue::None => vm.ctx.none(),
                EventValue::Bool(b) => vm.ctx.new_bool(*b).into(),
                EventValue::Int(v) => i(*v as i64),
                EventValue::Text(t) => s(t),
                EventValue::Index(x) => i(*x as i64),
                EventValue::Pointer(p) => {
                    let d = vm.ctx.new_dict();
                    let kind = match p.kind {
                        ecm_ui_proto::PointerKind::Down => "down",
                        ecm_ui_proto::PointerKind::Up => "up",
                        ecm_ui_proto::PointerKind::Move => "move",
                        ecm_ui_proto::PointerKind::Scroll => "scroll",
                        ecm_ui_proto::PointerKind::Enter => "enter",
                        ecm_ui_proto::PointerKind::Leave => "leave",
                    };
                    let _ = d.set_item("kind", s(kind), vm);
                    let _ = d.set_item("x", i(p.x as i64), vm);
                    let _ = d.set_item("y", i(p.y as i64), vm);
                    let _ = d.set_item("button", i(p.button.code() as i64), vm);
                    let _ = d.set_item("buttons", i(p.buttons as i64), vm);
                    let _ = d.set_item("scroll", i(p.scroll_y as i64), vm);
                    let target = p.target.as_deref().map_or_else(|| vm.ctx.none(), s);
                    let _ = d.set_item("target", target, vm);
                    d.into()
                }
                EventValue::Key { key, mods } => {
                    let d = vm.ctx.new_dict();
                    let _ = d.set_item("key", i(*key as i64), vm);
                    let _ = d.set_item("mods", i(*mods as i64), vm);
                    d.into()
                }
            };
            vec![s("widget"), i(e.window as i64), handler, s(e.kind.name()), value]
        }
        AppEvent::Hotkey { slot, down } => vec![s("hotkey"), i(slot as i64), vm.ctx.new_bool(down).into()],
        AppEvent::Suspend { deadline_ms } => vec![s("suspend"), i(deadline_ms as i64)],
        AppEvent::Frame { window, time_ms } => vec![s("frame"), i(window as i64), i(time_ms as i64)],
        AppEvent::CloseRequested { window } => vec![s("close"), i(window as i64)],
        AppEvent::ThemeChanged => vec![s("theme")],
        AppEvent::Message(m) => vec![s("message"), s(m.name()), s(&m.to_json())],
    };
    vm.ctx.new_tuple(items).into()
}

/// The `_ui` native module for the interpreter's module table.
pub fn make_native_module(vm: &VirtualMachine) -> rustpython_vm::PyRef<rustpython_vm::builtins::PyModule> {
    ui_native::make_module(vm)
}

#[pymodule]
pub mod ui_native {
    use super::*;

    /// Source of the user-facing module (see ui.py).
    #[pyattr]
    fn _source(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(UI_PY).into()
    }

    /// Connect to the compositor (role 0 app, 1 shell); returns a handle.
    #[pyfunction]
    fn connect(app_id: PyObjectRef, role: i32, vm: &VirtualMachine) -> PyResult<i64> {
        let app_id = app_id.str(vm)?.as_str().to_owned();
        let mock = TABLE.with(|t| {
            let t = t.borrow();
            t.use_mock.and_then(|m| t.mocks.get(&m).cloned())
        });
        let conn: Box<dyn Connection> = match mock {
            Some(m) => Box::new(m),
            None => default_connection(role).map_err(|e| err(vm, format!("ui: {e}")))?,
        };
        let role = if role == 1 { proto::Role::Shell } else { proto::Role::App };
        let core = Core::connect(&app_id, conn, role).map_err(|e| err(vm, format!("ui: {e}")))?;
        Ok(TABLE.with(|t| {
            let mut t = t.borrow_mut();
            t.next += 1;
            let h = t.next;
            t.apps.insert(h, core);
            h
        }))
    }

    /// Saved state from the previous run (bytes), or None.
    #[pyfunction]
    fn restore(h: i64, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let r = with_app(h, vm, |c| c.restore().map(|b| b.to_vec()))?;
        Ok(match r {
            Some(b) => vm.ctx.new_bytes(b).into(),
            None => vm.ctx.none(),
        })
    }

    /// Create a window showing `tree`; returns its id.
    #[pyfunction]
    fn window(h: i64, title: PyObjectRef, tree_obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<u32> {
        let title = title.str(vm)?.as_str().to_owned();
        let root = tree(&tree_obj, vm)?;
        with_app(h, vm, |c| c.create_window(&title, ecm_ui::window::Content::Static(root)))
    }

    /// Replace a window's widget tree.
    #[pyfunction]
    fn set_root(h: i64, win: u32, tree_obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let root = tree(&tree_obj, vm)?;
        with_app(h, vm, |c| c.set_root(win, root))
    }

    #[pyfunction]
    fn close_window(h: i64, win: u32, vm: &VirtualMachine) -> PyResult<()> {
        with_app(h, vm, |c| c.close_window(win))
    }

    /// (width, height, size class name) of a window (0, 0 before it is configured).
    #[pyfunction]
    fn window_size(h: i64, win: u32, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let (w, hh, class) = with_app(h, vm, |c| c.window(win).map_or((0, 0, "compact"), |w| (w.width, w.height, w.class().name())))?;
        Ok(vm.ctx.new_tuple(vec![vm.ctx.new_int(w).into(), vm.ctx.new_int(hh).into(), vm.ctx.new_str(class).into()]).into())
    }

    /// The state of the widget with this id in a window:
    /// {"text", "checked", "value", "selected"} (None if it has none yet).
    #[pyfunction]
    fn widget_state(h: i64, win: u32, id: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let id = id.str(vm)?.as_str().to_string();
        let st = with_app(h, vm, |c| c.window(win).and_then(|w| w.state(&id).cloned()))?;
        let Some(st) = st else { return Ok(vm.ctx.none()) };
        let d = vm.ctx.new_dict();
        d.set_item("text", vm.ctx.new_str(st.text.as_str()).into(), vm)?;
        d.set_item("checked", vm.ctx.new_bool(st.checked).into(), vm)?;
        d.set_item("value", vm.ctx.new_int(st.value).into(), vm)?;
        let sel = st.selected.map_or_else(|| vm.ctx.none(), |x| vm.ctx.new_int(x).into());
        d.set_item("selected", sel, vm)?;
        Ok(d.into())
    }

    #[pyfunction]
    fn hotkey(h: i64, slot: u8, label: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if !(1..=6).contains(&slot) {
            return Err(vm.new_value_error("hotkey slots are 1-6".to_owned()));
        }
        let label = label.str(vm)?.as_str().to_owned();
        with_app(h, vm, |c| c.claim_hotkey(slot, &label))
    }

    /// Render changed windows and send everything queued. False once closed.
    #[pyfunction]
    fn render(h: i64, vm: &VirtualMachine) -> PyResult<bool> {
        with_app(h, vm, |c| c.render().is_ok())
    }

    #[pyfunction]
    fn flush(h: i64, vm: &VirtualMachine) -> PyResult<bool> {
        with_app(h, vm, |c| c.flush().is_ok())
    }

    /// Wait up to `timeout_ms` (< 0 forever) and return the events as tuples,
    /// or None once the connection is closed.
    #[pyfunction]
    fn step(h: i64, timeout_ms: i32, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let r = with_app(h, vm, |c| c.step(timeout_ms))?;
        Ok(match r {
            Ok(evs) => vm.ctx.new_list(evs.into_iter().map(|e| event_tuple(e, vm)).collect()).into(),
            Err(_) => vm.ctx.none(),
        })
    }

    #[pyfunction]
    fn is_closed(h: i64, vm: &VirtualMachine) -> PyResult<bool> {
        with_app(h, vm, |c| c.is_closed())
    }

    #[pyfunction]
    fn quit(h: i64, vm: &VirtualMachine) -> PyResult<()> {
        with_app(h, vm, |c| c.quit())
    }

    /// Answer WillSuspend with the bytes to keep.
    #[pyfunction]
    fn save_state(h: i64, data: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let data = bytes_of(&data, vm)?;
        with_app(h, vm, |c| c.save_state(data))
    }

    #[pyfunction]
    fn request_frame(h: i64, win: u32, vm: &VirtualMachine) -> PyResult<()> {
        with_app(h, vm, |c| c.send(proto::Message::RequestFrame(proto::RequestFrame { surface: win })))
    }

    /// Send a protocol message given as (name, JSON fields) — the services
    /// (Notify, ClipboardSet, PickFile, Open, Launch, BackgroundDone, …).
    #[pyfunction]
    fn send(h: i64, name: PyObjectRef, fields: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let name = name.str(vm)?.as_str().to_owned();
        let f = to_json(&fields, vm, 0)?;
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
            other => return Err(vm.new_value_error(format!("ui.send: unsupported message {other:?}"))),
        };
        // Reject what the compositor would (limits) before it closes us.
        proto::Message::decode_exact(&m.to_bytes()).map_err(|e| vm.new_value_error(format!("ui.send: {e}")))?;
        with_app(h, vm, |c| c.send(m))
    }

    /// Canonical JSON text of a Python value (what Rust apps write too).
    #[pyfunction]
    fn json_dumps(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        Ok(to_json(&obj, vm, 0)?.to_string())
    }

    #[pyfunction]
    fn json_loads(text: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let text = match text.payload::<PyBytes>() {
            Some(b) => String::from_utf8(b.as_bytes().to_vec()).map_err(|_| vm.new_value_error("not UTF-8".to_owned()))?,
            None => text.str(vm)?.as_str().to_owned(),
        };
        let j = Json::parse(&text).map_err(|e| vm.new_value_error(e))?;
        to_py(&j, vm)
    }

    // ---- test support: a scripted connection instead of the UI fd

    /// A mock connection that returns these byte chunks from successive recvs.
    #[pyfunction]
    fn mock_new(chunks: PyObjectRef, vm: &VirtualMachine) -> PyResult<i64> {
        let m = MockConnection::new();
        let items: Vec<PyObjectRef> = match chunks.payload::<PyList>() {
            Some(l) => l.borrow_vec().to_vec(),
            None => return Err(vm.new_type_error("chunks must be a list of bytes".to_owned())),
        };
        for c in items {
            m.push_bytes(bytes_of(&c, vm)?);
        }
        Ok(TABLE.with(|t| {
            let mut t = t.borrow_mut();
            t.next += 1;
            let h = t.next;
            t.mocks.insert(h, m);
            h
        }))
    }

    /// Make `connect` use this mock (None = the real UI fd again).
    #[pyfunction]
    fn use_mock(h: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let h = if vm.is_none(&h) { None } else { Some(h.try_into_value::<i64>(vm)?) };
        TABLE.with(|t| t.borrow_mut().use_mock = h);
        Ok(())
    }

    /// Everything sent through a mock.
    #[pyfunction]
    fn mock_sent(h: i64, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let b = TABLE.with(|t| t.borrow().mocks.get(&h).map(|m| m.sent()));
        match b {
            Some(b) => Ok(vm.ctx.new_bytes(b).into()),
            None => Err(err(vm, format!("no mock {h}"))),
        }
    }
}

/// Bytes a mock connection made with `mock_new` has sent (Rust-side tests).
pub fn mock_sent(h: i64) -> Option<Vec<u8>> {
    TABLE.with(|t| t.borrow().mocks.get(&h).map(|m| m.sent()))
}

/// Register a mock connection a Rust test keeps a handle to; returns the
/// handle for `_ui.use_mock`.
pub fn register_mock(m: MockConnection) -> i64 {
    TABLE.with(|t| {
        let mut t = t.borrow_mut();
        t.next += 1;
        let h = t.next;
        t.mocks.insert(h, m);
        h
    })
}

/// Run `f` on a Python app's engine (Rust-side tests: hit ids, state).
pub fn with_core<T>(h: i64, f: impl FnOnce(&mut Core) -> T) -> Option<T> {
    TABLE.with(|t| t.borrow_mut().apps.get_mut(&h).map(f))
}

#[cfg(test)]
mod tests;
