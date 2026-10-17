//! `_console` native module: the program's console (ecm-console's
//! [`Session`]: its term, its event queue, its clocks). The `term`,
//! `events`, `computer`, `colors` and `keys` modules (term.py, ...) are
//! built on it, and `peripheral.pull_event` reads its queue; Lua's
//! `_console` has the same functions.
//!
//! The binary installs the computer's console ([`install_session`]); without
//! one (host tests) a headless console on virtual time is used.

use crate::values::{to_py, to_value};
use ecm_console::events::Event;
use ecm_console::session::Session;
use ecm_host_abi::peripheral::Value;
use rustpython_vm::{
    builtins::{PyBytes, PyStr},
    function::ArgIntoFloat,
    pymodule, PyObjectRef, PyResult, VirtualMachine,
};
use std::cell::RefCell;

pub const TERM_PY: &str = include_str!("term.py");
pub const EVENTS_PY: &str = include_str!("events.py");
pub const COMPUTER_PY: &str = include_str!("computer.py");
pub const COLORS_PY: &str = include_str!("colors.py");
pub const KEYS_PY: &str = include_str!("keys.py");

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// Use `session` as this program's console.
pub fn install_session(session: Session) {
    SESSION.with(|s| *s.borrow_mut() = Some(session));
}

/// Restore the console's colours and cursor after the program, if it drew.
pub fn finish() {
    SESSION.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            s.finish();
        }
    });
}

/// Run `f` on the session (a headless one if none was installed).
pub fn with<R>(f: impl FnOnce(&mut Session) -> R) -> R {
    SESSION.with(|s| {
        let mut s = s.borrow_mut();
        let s = s.get_or_insert_with(|| {
            let term = ecm_console::term::Term::new(51, 19, 1, Some(1));
            Session::new(term, Box::new(ecm_console::session::fake::Fake::default()))
        });
        f(s)
    })
}

/// Text for the term: bytes as they are, a str one byte per character (code
/// points past 255 become "?"), anything else as str() of it.
fn text(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    if let Some(b) = obj.payload::<PyBytes>() {
        return Ok(b.as_bytes().to_vec());
    }
    let s = match obj.payload::<PyStr>() {
        Some(s) => s.as_str().to_owned(),
        None => obj.str(vm)?.as_str().to_owned(),
    };
    Ok(s.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect())
}

fn opt_str(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<String>> {
    if vm.is_none(obj) {
        Ok(None)
    } else {
        Ok(Some(obj.str(vm)?.as_str().to_owned()))
    }
}

fn opt_f64(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<f64>> {
    if vm.is_none(obj) {
        Ok(None)
    } else {
        Ok(Some(obj.try_float(vm)?.to_f64()))
    }
}

fn err(vm: &VirtualMachine, e: String) -> rustpython_vm::builtins::PyBaseExceptionRef {
    vm.new_value_error(e)
}

/// (name, *args), or None. A typed character is a one-character str.
fn event_tuple(ev: Option<Event>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    let Some(ev) = ev else { return Ok(vm.ctx.none()) };
    let chars = ev.name == "char" || ev.name == "paste";
    let mut items = vec![vm.ctx.new_str(ev.name).into()];
    for a in ev.args {
        items.push(match a {
            Value::Bytes(b) if chars => vm.ctx.new_str(b.iter().map(|&c| c as char).collect::<String>()).into(),
            other => to_py(other, vm)?,
        });
    }
    Ok(vm.ctx.new_tuple(items).into())
}

/// The `_console` native module for the interpreter's module table.
pub fn make_native_module(vm: &VirtualMachine) -> rustpython_vm::PyRef<rustpython_vm::builtins::PyModule> {
    console_native::make_module(vm)
}

#[pymodule]
pub mod console_native {
    use super::*;

    #[pyattr]
    fn _term_source(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(TERM_PY).into()
    }
    #[pyattr]
    fn _events_source(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(EVENTS_PY).into()
    }
    #[pyattr]
    fn _computer_source(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(COMPUTER_PY).into()
    }
    #[pyattr]
    fn _colors_source(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(COLORS_PY).into()
    }
    #[pyattr]
    fn _keys_source(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(KEYS_PY).into()
    }

    // --- term ---

    #[pyfunction]
    fn write(s: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let b = text(&s, vm)?;
        with(|st| st.draw(|t| t.write(&b)));
        Ok(())
    }

    #[pyfunction]
    fn write_wrapped(s: PyObjectRef, vm: &VirtualMachine) -> PyResult<u32> {
        let b = text(&s, vm)?;
        Ok(with(|st| st.draw(|t| t.write_wrapped(&b))))
    }

    #[pyfunction]
    fn blit(s: PyObjectRef, fg: PyObjectRef, bg: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let (s, fg, bg) = (text(&s, vm)?, text(&fg, vm)?, text(&bg, vm)?);
        with(|st| st.draw(|t| t.blit(&s, &fg, &bg))).map_err(|e| err(vm, e))
    }

    #[pyfunction]
    fn clear() {
        with(|st| st.draw(|t| t.clear()));
    }

    #[pyfunction]
    fn clear_line() {
        with(|st| st.draw(|t| t.clear_line()));
    }

    #[pyfunction]
    fn scroll(n: i32) {
        with(|st| st.draw(|t| t.scroll(n)));
    }

    #[pyfunction]
    fn get_cursor_pos() -> (i32, i32) {
        with(|st| st.term.cursor())
    }

    #[pyfunction]
    fn set_cursor_pos(x: ArgIntoFloat, y: ArgIntoFloat) {
        let x: f64 = *x;
        let y: f64 = *y;
        with(|st| st.draw(|t| t.set_cursor(x.floor() as i32, y.floor() as i32)));
    }

    #[pyfunction]
    fn get_cursor_blink() -> bool {
        with(|st| st.term.blink())
    }

    #[pyfunction]
    fn set_cursor_blink(on: bool) {
        with(|st| st.draw(|t| t.set_blink(on)));
    }

    #[pyfunction]
    fn get_size() -> (i32, i32) {
        with(|st| st.term.size())
    }

    #[pyfunction]
    fn get_text_color() -> u32 {
        with(|st| st.term.text_colour())
    }

    #[pyfunction]
    fn set_text_color(c: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<()> {
        let c: f64 = *c;
        with(|st| st.term.set_text_colour(c)).map_err(|e| err(vm, e))
    }

    #[pyfunction]
    fn get_background_color() -> u32 {
        with(|st| st.term.background_colour())
    }

    #[pyfunction]
    fn set_background_color(c: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<()> {
        let c: f64 = *c;
        with(|st| st.term.set_background_colour(c)).map_err(|e| err(vm, e))
    }

    #[pyfunction]
    fn get_palette_color(c: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<u32> {
        let c: f64 = *c;
        with(|st| st.term.palette(c)).map_err(|e| err(vm, e))
    }

    #[pyfunction]
    fn set_palette_color(c: ArgIntoFloat, rgb: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<()> {
        let c: f64 = *c;
        let rgb: f64 = *rgb;
        with(|st| st.term.set_palette(c, rgb as u32)).map_err(|e| err(vm, e))
    }

    #[pyfunction(name = "finish")]
    fn finish_term() {
        super::finish();
    }

    // --- events ---

    #[pyfunction]
    fn pull(filter: PyObjectRef, timeout: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let (f, t) = (opt_str(&filter, vm)?, opt_f64(&timeout, vm)?);
        event_tuple(with(|st| st.pull(f.as_deref(), t)), vm)
    }

    #[pyfunction]
    fn pull_quiet(filter: PyObjectRef, timeout: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let (f, t) = (opt_str(&filter, vm)?, opt_f64(&timeout, vm)?);
        event_tuple(with(|st| st.pull_quiet(f.as_deref(), t)), vm)
    }

    /// queue(name, args): args a list/tuple of plain values.
    #[pyfunction]
    fn queue(name: PyObjectRef, args: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let name = name.str(vm)?.as_str().to_owned();
        let args = match to_value(&args, vm, 0)? {
            Value::List(items) => items,
            Value::Nil => Vec::new(),
            other => vec![other],
        };
        with(|st| st.queue(Event { name, args }));
        Ok(())
    }

    #[pyfunction]
    fn start_timer(secs: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<u32> {
        let secs: f64 = *secs;
        with(|st| st.start_timer(secs)).map_err(|e| err(vm, e))
    }

    #[pyfunction]
    fn cancel_timer(id: ArgIntoFloat) {
        let id: f64 = *id;
        with(|st| st.cancel_timer(id as u32));
    }

    #[pyfunction]
    fn set_alarm(hour: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<u32> {
        let hour: f64 = *hour;
        with(|st| st.set_alarm(hour)).map_err(|e| err(vm, e))
    }

    #[pyfunction]
    fn cancel_alarm(id: ArgIntoFloat) {
        let id: f64 = *id;
        with(|st| st.cancel_alarm(id as u32));
    }

    #[pyfunction]
    fn sleep(secs: ArgIntoFloat) {
        let secs: f64 = *secs;
        with(|st| st.sleep(secs));
    }

    // --- keys ---

    #[pyfunction]
    fn key_names(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let d = vm.ctx.new_dict();
        for (name, code) in ecm_console::keys::names() {
            d.set_item(name.as_str(), vm.ctx.new_int(code).into(), vm)?;
        }
        Ok(d.into())
    }

    // --- computer ---

    #[pyfunction]
    fn id() -> Option<i32> {
        with(|st| st.host.computer_id())
    }

    #[pyfunction]
    fn label() -> Option<String> {
        with(|st| st.host.label())
    }

    #[pyfunction]
    fn set_label(label: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let l = opt_str(&label, vm)?;
        with(|st| st.host.set_label(l.as_deref()));
        Ok(())
    }

    #[pyfunction]
    fn uptime() -> f64 {
        with(|st| st.uptime())
    }

    #[pyfunction]
    fn time(locale: PyObjectRef, vm: &VirtualMachine) -> PyResult<f64> {
        let l = opt_str(&locale, vm)?;
        with(|st| st.time(l.as_deref())).map_err(|e| err(vm, e))
    }

    #[pyfunction]
    fn day(locale: PyObjectRef, vm: &VirtualMachine) -> PyResult<i64> {
        let l = opt_str(&locale, vm)?;
        with(|st| st.day(l.as_deref())).map_err(|e| err(vm, e))
    }

    #[pyfunction]
    fn epoch(locale: PyObjectRef, vm: &VirtualMachine) -> PyResult<i64> {
        let l = opt_str(&locale, vm)?;
        with(|st| st.epoch(l.as_deref())).map_err(|e| err(vm, e))
    }
}

#[cfg(test)]
mod tests;
