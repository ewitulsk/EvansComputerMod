//! Lua values as JSON (the `.ui.json` widget trees, saved state, protocol
//! messages) and back.
//!
//! Lua has one table type for lists and objects: a table whose keys are
//! exactly 1..n is a list, any other table is an object. An empty table is an
//! object unless it is marked as a list (`ui.array(t)`, or any list that came
//! from JSON), so `{children = ui.array()}` stays `"children":[]`.

use ecm_ui::Json;
use mlua::{Lua, Table, Value};

const MAX_DEPTH: usize = 64;

/// Metatable field that marks a table as a JSON list.
pub const ARRAY_MARK: &str = "__jsonarray";

/// The shared metatable that marks lists (kept in the registry).
pub fn array_meta(lua: &Lua) -> mlua::Result<Table> {
    if let Ok(t) = lua.named_registry_value::<Table>("ecm.json_array") {
        return Ok(t);
    }
    let t = lua.create_table()?;
    t.set(ARRAY_MARK, true)?;
    lua.set_named_registry_value("ecm.json_array", t.clone())?;
    Ok(t)
}

fn marked_array(t: &Table) -> bool {
    t.metatable().and_then(|m| m.raw_get::<bool>(ARRAY_MARK).ok()).unwrap_or(false)
}

/// A whole number that fits an i64 exactly, as an integer.
fn number(n: f64) -> Json {
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        Json::Int(n as i64)
    } else {
        Json::Float(n)
    }
}

pub fn to_json(v: &Value, depth: usize) -> mlua::Result<Json> {
    if depth > MAX_DEPTH {
        return Err(mlua::Error::runtime("value nested too deeply"));
    }
    Ok(match v {
        Value::Nil => Json::Null,
        Value::Boolean(b) => Json::Bool(*b),
        Value::Integer(i) => Json::Int(*i as i64),
        Value::Number(n) => number(*n),
        Value::String(s) => Json::Str(String::from_utf8_lossy(&s.as_bytes()).into_owned()),
        Value::Table(t) => {
            let len = t.raw_len();
            let mut pairs = Vec::new();
            for pair in t.pairs::<Value, Value>() {
                pairs.push(pair?);
            }
            if (len > 0 && pairs.len() == len) || (pairs.is_empty() && marked_array(t)) {
                let items = (1..=len)
                    .map(|i| t.raw_get::<Value>(i).and_then(|v| to_json(&v, depth + 1)))
                    .collect::<mlua::Result<_>>()?;
                Json::Arr(items)
            } else {
                let mut kv = Vec::with_capacity(pairs.len());
                for (k, v) in &pairs {
                    let key = match k {
                        Value::String(s) => String::from_utf8_lossy(&s.as_bytes()).into_owned(),
                        other => {
                            return Err(mlua::Error::runtime(format!(
                                "object keys must be strings, got a {} (a list is a table with keys 1..n)",
                                other.type_name()
                            )))
                        }
                    };
                    kv.push((key, to_json(v, depth + 1)?));
                }
                // Lua tables have no order: sort, so the same table always gives the same text.
                kv.sort_by(|a, b| a.0.cmp(&b.0));
                Json::Obj(kv)
            }
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "can't convert a {} (use nil, boolean, number, string or table)",
                other.type_name()
            )))
        }
    })
}

pub fn to_lua(lua: &Lua, j: &Json) -> mlua::Result<Value> {
    Ok(match j {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Boolean(*b),
        Json::Int(i) => Value::Number(*i as f64),
        Json::Float(f) => Value::Number(*f),
        Json::Str(s) => Value::String(lua.create_string(s)?),
        Json::Arr(a) => {
            let t = lua.create_table_with_capacity(a.len(), 0)?;
            for (i, x) in a.iter().enumerate() {
                t.raw_set(i + 1, to_lua(lua, x)?)?;
            }
            t.set_metatable(Some(array_meta(lua)?))?;
            Value::Table(t)
        }
        Json::Obj(kv) => {
            let t = lua.create_table_with_capacity(0, kv.len())?;
            for (k, v) in kv {
                t.raw_set(k.as_str(), to_lua(lua, v)?)?;
            }
            Value::Table(t)
        }
    })
}

/// The bytes of a Lua string argument (Lua strings hold any bytes).
pub fn bytes(v: &Value, what: &str) -> mlua::Result<Vec<u8>> {
    match v {
        Value::String(s) => Ok(s.as_bytes().to_vec()),
        other => Err(mlua::Error::runtime(format!("{what}: expected a string, got a {}", other.type_name()))),
    }
}
