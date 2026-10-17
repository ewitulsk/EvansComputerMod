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
    Ok(m)
}
