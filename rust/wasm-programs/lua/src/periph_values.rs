//! Peripheral values (ecm_host_abi::peripheral::Value) as Lua values and
//! back: peripheral calls, peripheral events and the console's events.

use ecm_host_abi::peripheral::Value;
use mlua::{Lua, Value as LuaValue};

const MAX_DEPTH: usize = 32;

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
