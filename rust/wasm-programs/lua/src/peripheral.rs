//! `_peripheral` native module: thin bindings over `ecm_host_abi::peripheral`
//! (the same functions as Python's `_peripheral`). The user-facing
//! `peripheral` module (lua/peripheral.lua) is built on top of it.
//!
//!   _peripheral.list()                      → { {name, type}, ... }
//!   _peripheral.methods(name)               → true, type, {methods} | false, message
//!   _peripheral.call(name, method, ...)     → true, result | false, message
//!   _peripheral.wait_event(filter, ms)      → event, attachment, args... (nil on timeout)

use ecm_host_abi::peripheral::{self as abi, Error, Value};
use mlua::{Lua, MultiValue, Table, Value as LuaValue, Variadic};

const MAX_DEPTH: usize = 32;

fn host_error(e: Error) -> mlua::Error {
    match e {
        Error::Unavailable => mlua::Error::runtime("peripherals are not available here"),
        Error::Malformed(what) => mlua::Error::runtime(format!("bad reply from the computer: {what}")),
        Error::Peripheral(msg) => mlua::Error::runtime(msg),
    }
}

/// A Lua value as a peripheral value. A table whose keys are exactly 1..n is a
/// list; any other table is a map (an empty table is an empty list).
pub fn to_value(v: &LuaValue, depth: usize) -> mlua::Result<Value> {
    if depth > MAX_DEPTH {
        return Err(mlua::Error::runtime("value nested too deeply to send to a peripheral"));
    }
    Ok(match v {
        LuaValue::Nil => Value::Nil,
        LuaValue::Boolean(b) => Value::Bool(*b),
        LuaValue::Integer(i) => Value::Int(*i as i64),
        // Lua 5.2 numbers are all floats: send whole numbers as integers.
        LuaValue::Number(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Value::Int(*n as i64),
        LuaValue::Number(n) => Value::Float(*n),
        LuaValue::String(s) => match s.to_str() {
            Ok(text) => Value::Str(text.to_owned()),
            Err(_) => Value::Bytes(s.as_bytes().to_vec()),
        },
        LuaValue::Table(t) => {
            let len = t.raw_len();
            let mut pairs = Vec::new();
            for pair in t.pairs::<LuaValue, LuaValue>() {
                pairs.push(pair?);
            }
            if pairs.len() == len {
                let items = (1..=len)
                    .map(|i| t.raw_get::<LuaValue>(i).and_then(|v| to_value(&v, depth + 1)))
                    .collect::<mlua::Result<_>>()?;
                Value::List(items)
            } else {
                let entries = pairs
                    .iter()
                    .map(|(k, v)| Ok((to_value(k, depth + 1)?, to_value(v, depth + 1)?)))
                    .collect::<mlua::Result<_>>()?;
                Value::Map(entries)
            }
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "can't send a {} to a peripheral (use nil, boolean, number, string or table)",
                other.type_name()
            )))
        }
    })
}

pub fn to_lua(lua: &Lua, v: Value) -> mlua::Result<LuaValue> {
    Ok(match v {
        Value::Nil => LuaValue::Nil,
        Value::Bool(b) => LuaValue::Boolean(b),
        Value::Int(i) => LuaValue::Number(i as f64),
        Value::Float(f) => LuaValue::Number(f),
        Value::Str(s) => LuaValue::String(lua.create_string(s)?),
        Value::Bytes(b) => LuaValue::String(lua.create_string(b)?),
        Value::List(items) => {
            let t = lua.create_table_with_capacity(items.len(), 0)?;
            for (i, item) in items.into_iter().enumerate() {
                t.raw_set(i + 1, to_lua(lua, item)?)?;
            }
            LuaValue::Table(t)
        }
        Value::Map(entries) => {
            let t = lua.create_table_with_capacity(0, entries.len())?;
            for (k, v) in entries {
                t.raw_set(to_lua(lua, k)?, to_lua(lua, v)?)?;
            }
            LuaValue::Table(t)
        }
    })
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
