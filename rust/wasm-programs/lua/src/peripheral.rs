//! `_peripheral` native module: thin bindings over `ecm_host_abi::peripheral`
//! (the same functions as Python's `_peripheral`). The user-facing
//! `peripheral` module (lua/peripheral.lua) is built on top of it.
//!
//!   _peripheral.list()                      → { {name, type}, ... }
//!   _peripheral.methods(name)               → true, type, {methods} | false, message
//!   _peripheral.call(name, method, ...)     → true, result | false, message
//!   _peripheral.wait_event(filter, ms)      → event, attachment, args... (nil on timeout)

use ecm_host_abi::peripheral::{self as abi, Error};
use ecm_lua::periph_values::{to_lua, to_value};
use mlua::{Lua, MultiValue, Table, Value as LuaValue, Variadic};

fn host_error(e: Error) -> mlua::Error {
    match e {
        Error::Unavailable => mlua::Error::runtime("peripherals are not available here"),
        Error::Malformed(what) => mlua::Error::runtime(format!("bad reply from the computer: {what}")),
        Error::Peripheral(msg) => mlua::Error::runtime(msg),
    }
}

pub fn module(lua: &Lua) -> mlua::Result<Table> {
    let m = lua.create_table()?;

    m.set("list", lua.create_function(|lua, ()| {
        let items = abi::list().map_err(host_error)?;
        let out = lua.create_table()?;
        for (i, (name, ty)) in items.into_iter().enumerate() {
            out.raw_set(i + 1, lua.create_sequence_from([name, ty])?)?;
        }
        Ok(out)
    })?)?;

    m.set("methods", lua.create_function(|lua, name: String| {
        match abi::methods(&name) {
            Ok((ty, names)) => Ok(MultiValue::from_iter([
                LuaValue::Boolean(true),
                LuaValue::String(lua.create_string(ty)?),
                LuaValue::Table(lua.create_sequence_from(names)?),
            ])),
            Err(Error::Peripheral(msg)) => {
                Ok(MultiValue::from_iter([LuaValue::Boolean(false), LuaValue::String(lua.create_string(msg)?)]))
            }
            Err(e) => Err(host_error(e)),
        }
    })?)?;

    m.set("call", lua.create_function(|lua, (name, method, args): (String, String, Variadic<LuaValue>)| {
        let args = args.iter().map(|a| to_value(a, 0)).collect::<mlua::Result<Vec<_>>>()?;
        match abi::call(&name, &method, &args) {
            Ok(v) => Ok(MultiValue::from_iter([LuaValue::Boolean(true), to_lua(lua, v)?])),
            Err(Error::Peripheral(msg)) => {
                Ok(MultiValue::from_iter([LuaValue::Boolean(false), LuaValue::String(lua.create_string(msg)?)]))
            }
            Err(e) => Err(host_error(e)),
        }
    })?)?;

    // filter nil = any event; timeout_ms < 0 = wait forever, 0 = don't wait.
    m.set("wait_event", lua.create_function(|lua, (filter, timeout_ms): (Option<String>, Option<i32>)| {
        match abi::wait_event(filter.as_deref(), timeout_ms.unwrap_or(-1)).map_err(host_error)? {
            None => Ok(MultiValue::new()),
            Some(items) => items.into_iter().map(|v| to_lua(lua, v)).collect(),
        }
    })?)?;

    Ok(m)
}
