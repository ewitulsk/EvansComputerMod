//! `_console` native module: the program's console (ecm-console's
//! [`Session`]: its term, its event queue, its clocks). The `term`,
//! `events`, `computer` and `peripheral` modules and the ComputerCraft
//! globals (lua/cc.lua) are built on it; Python's `_console` has the same
//! functions.

use crate::periph_values::{to_lua, to_value};
use ecm_console::events::Event;
use ecm_console::session::Session;
use ecm_host_abi::peripheral::Value;
use mlua::{Lua, LuaString, MultiValue, Table, Value as LuaValue, Variadic};
use std::cell::RefCell;
use std::rc::Rc;

fn err(e: String) -> mlua::Error {
    mlua::Error::runtime(e)
}

fn event_values(lua: &Lua, ev: Option<Event>) -> mlua::Result<MultiValue> {
    let Some(ev) = ev else { return Ok(MultiValue::new()) };
    let mut out = vec![LuaValue::String(lua.create_string(&ev.name)?)];
    for a in ev.args {
        out.push(to_lua(lua, a)?);
    }
    Ok(MultiValue::from_iter(out))
}

/// A Lua value for textutils: tables in full (an empty table is a map, as
/// in CC), with CC's errors for a table inside itself and for values that
/// have no text form. `empty_array` is textutils.empty_json_array.
fn text_value(v: &LuaValue, seen: &mut Vec<*const std::ffi::c_void>, empty_array: &LuaValue) -> mlua::Result<Value> {
    Ok(match v {
        LuaValue::Table(t) => {
            if let LuaValue::Table(e) = empty_array {
                if e == t {
                    return Ok(Value::List(Vec::new()));
                }
            }
            let ptr = t.to_pointer();
            if seen.contains(&ptr) {
                return Err(mlua::Error::runtime("Cannot serialize table with recursive entries"));
            }
            seen.push(ptr);
            let mut entries = Vec::new();
            for pair in t.pairs::<LuaValue, LuaValue>() {
                let (k, v) = pair?;
                entries.push((text_value(&k, seen, empty_array)?, text_value(&v, seen, empty_array)?));
            }
            seen.pop();
            Value::Map(entries)
        }
        LuaValue::Function(_) | LuaValue::Thread(_) | LuaValue::UserData(_) | LuaValue::LightUserData(_) => {
            return Err(mlua::Error::runtime(format!("Cannot serialize type {}", v.type_name())));
        }
        other => to_value(other, 0)?,
    })
}

fn attributes_table(lua: &Lua, a: ecm_console::fs::Attributes) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("size", a.size as f64)?;
    t.set("isDir", a.is_dir)?;
    t.set("isReadOnly", a.is_read_only)?;
    t.set("created", a.created as f64)?;
    t.set("modified", a.modified as f64)?;
    t.set("modification", a.modified as f64)?;
    Ok(t)
}

pub fn module(lua: &Lua, session: Session) -> mlua::Result<Table> {
    let s = Rc::new(RefCell::new(session));
    let m = lua.create_table()?;
    macro_rules! f {
        ($name:literal, |$st:ident, $args:tt : $ty:ty| $body:expr) => {{
            let s = s.clone();
            m.set($name, lua.create_function(move |_, $args: $ty| {
                #[allow(unused_mut)]
                let mut $st = s.borrow_mut();
                $body
            })?)?;
        }};
    }

    // --- term ---
    f!("write", |st, text: LuaString| { st.draw(|t| t.write(&text.as_bytes())); Ok(()) });
    f!("write_wrapped", |st, text: LuaString| Ok(st.draw(|t| t.write_wrapped(&text.as_bytes()))));
    f!("blit", |st, (text, fg, bg): (LuaString, LuaString, LuaString)| {
        st.draw(|t| t.blit(&text.as_bytes(), &fg.as_bytes(), &bg.as_bytes())).map_err(err)
    });
    f!("clear", |st, (): ()| { st.draw(|t| t.clear()); Ok(()) });
    f!("clear_line", |st, (): ()| { st.draw(|t| t.clear_line()); Ok(()) });
    f!("scroll", |st, n: f64| { st.draw(|t| t.scroll(n as i32)); Ok(()) });
    f!("get_cursor_pos", |st, (): ()| Ok(st.term.cursor()));
    f!("set_cursor_pos", |st, (x, y): (f64, f64)| { st.draw(|t| t.set_cursor(x.floor() as i32, y.floor() as i32)); Ok(()) });
    f!("get_cursor_blink", |st, (): ()| Ok(st.term.blink()));
    f!("set_cursor_blink", |st, on: bool| { st.draw(|t| t.set_blink(on)); Ok(()) });
    f!("get_size", |st, (): ()| Ok(st.term.size()));
    f!("get_text_color", |st, (): ()| Ok(st.term.text_colour()));
    f!("set_text_color", |st, c: f64| st.term.set_text_colour(c).map_err(err));
    f!("get_background_color", |st, (): ()| Ok(st.term.background_colour()));
    f!("set_background_color", |st, c: f64| st.term.set_background_colour(c).map_err(err));
    f!("get_palette_color", |st, c: f64| st.term.palette(c).map_err(err));
    f!("set_palette_color", |st, (c, rgb): (f64, f64)| st.term.set_palette(c, rgb as u32).map_err(err));
    f!("finish", |st, (): ()| { st.finish(); Ok(()) });

    // --- events ---
    {
        let s = s.clone();
        m.set("pull", lua.create_function(move |lua, (filter, timeout): (Option<String>, Option<f64>)| {
            let ev = s.borrow_mut().pull(filter.as_deref(), timeout);
            event_values(lua, ev)
        })?)?;
    }
    {
        let s = s.clone();
        m.set("pull_quiet", lua.create_function(move |lua, (filter, timeout): (Option<String>, Option<f64>)| {
            let ev = s.borrow_mut().pull_quiet(filter.as_deref(), timeout);
            event_values(lua, ev)
        })?)?;
    }
    f!("queue", |st, (name, args): (String, Variadic<LuaValue>)| {
        // Values that can't travel (functions, userdata) arrive as nil.
        let args = args.iter().map(|a| to_value(a, 0).unwrap_or(Value::Nil)).collect();
        st.queue(Event { name, args });
        Ok(())
    });
    f!("start_timer", |st, secs: f64| st.start_timer(secs).map_err(err));
    f!("cancel_timer", |st, id: f64| { st.cancel_timer(id as u32); Ok(()) });
    f!("set_alarm", |st, hour: f64| st.set_alarm(hour).map_err(err));
    f!("cancel_alarm", |st, id: f64| { st.cancel_alarm(id as u32); Ok(()) });
    f!("sleep", |st, secs: Option<f64>| { st.sleep(secs.unwrap_or(0.0)); Ok(()) });

    // --- keys ---
    m.set("key_names", lua.create_function(|lua, ()| {
        let t = lua.create_table()?;
        for (name, code) in ecm_console::keys::names() {
            t.set(name, code)?;
        }
        Ok(t)
    })?)?;

    // --- computer ---
    f!("id", |st, (): ()| Ok(st.host.computer_id()));
    f!("label", |st, (): ()| Ok(st.host.label()));
    f!("set_label", |st, label: Option<String>| { st.host.set_label(label.as_deref()); Ok(()) });
    f!("uptime", |st, (): ()| Ok(st.uptime()));
    f!("time", |st, locale: Option<String>| st.time(locale.as_deref()).map_err(err));
    f!("day", |st, locale: Option<String>| st.day(locale.as_deref()).map_err(err));
    f!("epoch", |st, locale: Option<String>| st.epoch(locale.as_deref()).map_err(err));

    // --- fs (paths from the computer's root) ---
    f!("fs_list", |st, path: String| st.files().list(&path).map_err(err));
    f!("fs_exists", |st, path: String| Ok(st.files().exists(&path)));
    f!("fs_is_dir", |st, path: String| Ok(st.files().is_dir(&path)));
    f!("fs_is_read_only", |st, path: String| Ok(st.files().is_read_only(&path)));
    f!("fs_get_size", |st, path: String| st.files().size(&path).map(|n| n as f64).map_err(err));
    f!("fs_make_dir", |st, path: String| st.files().make_dir(&path).map_err(err));
    f!("fs_delete", |st, path: String| st.files().delete(&path).map_err(err));
    f!("fs_copy", |st, (from, to): (String, String)| st.files().copy(&from, &to).map_err(err));
    f!("fs_move", |st, (from, to): (String, String)| st.files().move_(&from, &to).map_err(err));
    f!("fs_find", |st, pattern: String| Ok(st.files().find(&pattern)));
    f!("fs_get_free_space", |st, path: String| Ok(st.files().free_space(&path).map(|n| n as f64)));
    // The path to hand to io.open for a computer path (Err above the root).
    f!("fs_real_path", |st, path: String| {
        let p = ecm_console::fs::sanitize(&path).map_err(err)?;
        Ok(st.host.root().join(p).to_string_lossy().replace('\\', "/"))
    });
    {
        let s = s.clone();
        m.set("fs_attributes", lua.create_function(move |lua, path: String| {
            let a = s.borrow().files().attributes(&path).map_err(err)?;
            attributes_table(lua, a)
        })?)?;
    }
    m.set("fs_combine", lua.create_function(|_, parts: Variadic<String>| {
        let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
        Ok(ecm_console::fs::combine(&refs))
    })?)?;
    m.set("fs_get_name", lua.create_function(|_, p: String| Ok(ecm_console::fs::get_name(&p)))?)?;
    m.set("fs_get_dir", lua.create_function(|_, p: String| Ok(ecm_console::fs::get_dir(&p)))?)?;

    // --- textutils ---
    m.set("serialize", lua.create_function(|_, (v, compact): (LuaValue, Option<bool>)| {
        let v = text_value(&v, &mut Vec::new(), &LuaValue::Nil)?;
        Ok(ecm_console::textutils::serialize(&v, compact.unwrap_or(false)))
    })?)?;
    m.set("unserialize", lua.create_function(|lua, text: String| {
        match ecm_console::textutils::parse_lua(&text) {
            Ok(v) => to_lua(lua, v),
            Err(_) => Ok(LuaValue::Nil),
        }
    })?)?;
    m.set("serialize_json", lua.create_function(|_, (v, unicode, empty_array): (LuaValue, Option<bool>, LuaValue)| {
        let v = text_value(&v, &mut Vec::new(), &empty_array)?;
        ecm_console::textutils::to_json(&v, unicode.unwrap_or(false)).map_err(err)
    })?)?;
    m.set("unserialize_json", lua.create_function(|lua, text: String| {
        match ecm_console::textutils::parse_json(&text) {
            Ok(v) => Ok((to_lua(lua, v)?, None)),
            Err(e) => Ok((LuaValue::Nil, Some(e))),
        }
    })?)?;
    m.set("format_time", lua.create_function(|_, (t, h24): (f64, Option<bool>)| {
        Ok(ecm_console::textutils::format_time(t, h24.unwrap_or(false)))
    })?)?;
    m.set("url_encode", lua.create_function(|_, s: LuaString| Ok(ecm_console::textutils::url_encode(&s.as_bytes())))?)?;

    // --- redstone (CC side names) ---
    f!("rs_get_input", |st, side: String| {
        let n = ecm_console::redstone::side(&side).map_err(err)?;
        Ok(st.host.redstone_input(n))
    });
    f!("rs_get_output", |st, side: String| {
        let n = ecm_console::redstone::side(&side).map_err(err)?;
        Ok(st.host.redstone_output(n))
    });
    f!("rs_set_output", |st, (side, power): (String, f64)| {
        let n = ecm_console::redstone::side(&side).map_err(err)?;
        st.host.set_redstone_output(n, (power as i32).clamp(0, 15));
        Ok(())
    });

    Ok(m)
}
