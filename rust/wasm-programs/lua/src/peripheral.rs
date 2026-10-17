//! `peripheral` module: bindings over `ecm_host_abi::peripheral`.
//!
//!   peripheral.list()                    → { {name=..., type=...}, ... }
//!   peripheral.methods(name)             → type, { method names }
//!   peripheral.call(name, method, ...)   → the method's result (errors raise)
//!   peripheral.wrap(name)                → table with one function per method
//!   peripheral.wait_event([filter], [timeout_s]) → event, attachment, args... (nil on timeout)

use ecm_host_abi::peripheral::{self as abi, Error, Value};
use mlua::{Lua, MultiValue, Table, Value as LuaValue, Variadic};

const MAX_DEPTH: usize = 32;

/// `peripheral.wrap`, written in Lua over `methods` and `call`.
const WRAP: &str = r#"
local peripheral = ...
return function(name)
    local ty, methods = peripheral.methods(name)
    local p = { name = name, type = ty }
    for _, m in ipairs(methods) do
        p[m] = function(...) return peripheral.call(name, m, ...) end
    end
    return p
end
"#;

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
            let entry = lua.create_table()?;
            entry.set("name", name)?;
            entry.set("type", ty)?;
            out.raw_set(i + 1, entry)?;
        }
        Ok(out)
    })?)?;

    m.set("methods", lua.create_function(|lua, name: String| {
        let (ty, names) = abi::methods(&name).map_err(host_error)?;
        Ok((ty, lua.create_sequence_from(names)?))
    })?)?;

    m.set("call", lua.create_function(|lua, (name, method, args): (String, String, Variadic<LuaValue>)| {
        let args = args.iter().map(|a| to_value(a, 0)).collect::<mlua::Result<Vec<_>>>()?;
        to_lua(lua, abi::call(&name, &method, &args).map_err(host_error)?)
    })?)?;

    // filter nil = any event; timeout nil = wait forever, 0 = don't wait.
    m.set("wait_event", lua.create_function(|lua, (filter, timeout): (Option<String>, Option<f64>)| {
        let timeout_ms = match timeout {
            None => -1,
            Some(s) => (s.max(0.0) * 1000.0).min(i32::MAX as f64) as i32,
        };
        match abi::wait_event(filter.as_deref(), timeout_ms).map_err(host_error)? {
            None => Ok(MultiValue::new()),
            Some(items) => items.into_iter().map(|v| to_lua(lua, v)).collect(),
        }
    })?)?;

    let wrap: mlua::Function = lua.load(WRAP).set_name("=peripheral").call(m.clone())?;
    m.set("wrap", wrap)?;
    Ok(m)
}
