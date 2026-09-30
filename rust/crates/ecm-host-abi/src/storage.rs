//! Typed client for the item storage peripherals (MC 1.21.1).
//!
//! Every storage peripheral (a Drive, a Storage Module, a Wired Bus Module)
//! answers for the computer's whole storage net, so any one of them will do:
//!
//! ```ignore
//! let net = ecm_host_abi::storage::Storage::find().expect("no storage");
//! for item in net.items(Some("iron"), Sort::Count, 0, 50)? {
//!     println!("{} x{}", item.name, item.count);
//! }
//! net.extract(&item.key, 16, None)?;   // out of the first reachable decoder
//! ```
//!
//! All calls run on the server thread, one at a time; the ledger they act on
//! is the only authority on what is stored.

use crate::peripheral::{self, Error, Value};
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

/// Peripheral types that carry the storage API.
pub const TYPES: [&str; 3] = ["storage_drive", "storage_module", "wired_sensors"];

/// Sort orders for [`Storage::items`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Count,
    Name,
    Id,
    Mod,
}

impl Sort {
    pub fn as_str(self) -> &'static str {
        match self {
            Sort::Count => "count",
            Sort::Name => "name",
            Sort::Id => "id",
            Sort::Mod => "mod",
        }
    }
}

/// One stored item type.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub key: String,
    pub id: String,
    pub name: String,
    pub mod_id: String,
    pub count: i64,
}

/// One reachable cell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    pub id: String,
    pub short: String,
    pub tier: String,
    pub device: String,
    pub slot: i64,
    pub bytes_used: i64,
    pub bytes_total: i64,
    pub types_used: i64,
    pub types_total: i64,
    pub items: i64,
}

/// An encoder or decoder in reach.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Port {
    pub name: String,
    pub kind: String,
    pub id: String,
}

/// An unspent token issued from a cell in reach (or a lost item, token empty).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TokenInfo {
    pub token: String,
    pub key: String,
    pub name: String,
    pub count: i64,
    pub expires_in: i64,
}

/// A storage net, reached through one attached storage peripheral.
#[derive(Clone, Debug)]
pub struct Storage {
    name: String,
}

fn get<'a>(m: &'a Value, key: &str) -> Option<&'a Value> {
    if let Value::Map(entries) = m {
        for (k, v) in entries {
            if let Value::Str(s) = k {
                if s == key {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn s(m: &Value, key: &str) -> String {
    match get(m, key) {
        Some(Value::Str(v)) => v.clone(),
        _ => String::new(),
    }
}

fn i(m: &Value, key: &str) -> i64 {
    match get(m, key) {
        Some(Value::Int(v)) => *v,
        Some(Value::Float(f)) => *f as i64,
        _ => 0,
    }
}

fn list(v: Value) -> Result<Vec<Value>, Error> {
    match v {
        Value::List(items) => Ok(items),
        _ => Err(Error::Malformed("expected a list")),
    }
}

fn opt_str(v: Option<&str>) -> Value {
    v.map(|s| Value::Str(s.to_string())).unwrap_or(Value::Nil)
}

impl Storage {
    /// The first attached storage peripheral.
    pub fn find() -> Option<Storage> {
        let all = peripheral::list().ok()?;
        for want in TYPES {
            if let Some((name, _)) = all.iter().find(|(_, ty)| ty == want) {
                return Some(Storage { name: name.clone() });
            }
        }
        None
    }

    /// Use the peripheral attached as `name`.
    pub fn at(name: &str) -> Storage {
        Storage { name: name.to_string() }
    }

    pub fn attachment(&self) -> &str {
        &self.name
    }

    /// Call any storage method.
    pub fn call(&self, method: &str, args: &[Value]) -> Result<Value, Error> {
        peripheral::call(&self.name, method, args)
    }

    /// Items in reach matching `query` (substring of id or name; `@mod` filters by namespace).
    pub fn items(&self, query: Option<&str>, sort: Sort, offset: i64, limit: i64) -> Result<Vec<Item>, Error> {
        let v = self.call("items", &[opt_str(query), Value::Str(sort.as_str().to_string()), Value::Int(offset), Value::Int(limit)])?;
        Ok(list(v)?
            .iter()
            .map(|m| Item { key: s(m, "key"), id: s(m, "id"), name: s(m, "name"), mod_id: s(m, "mod"), count: i(m, "count") })
            .collect())
    }

    pub fn cells(&self) -> Result<Vec<Cell>, Error> {
        let v = self.call("cells", &[])?;
        Ok(list(v)?
            .iter()
            .map(|m| Cell {
                id: s(m, "id"),
                short: s(m, "short"),
                tier: s(m, "tier"),
                device: s(m, "device"),
                slot: i(m, "slot"),
                bytes_used: i(m, "bytes_used"),
                bytes_total: i(m, "bytes_total"),
                types_used: i(m, "types_used"),
                types_total: i(m, "types_total"),
                items: i(m, "items"),
            })
            .collect())
    }

    /// Encoders and decoders in reach.
    pub fn ports(&self) -> Result<Vec<Port>, Error> {
        let v = self.call("devices", &[])?;
        let ports = get(&v, "ports").cloned().unwrap_or(Value::List(vec![]));
        Ok(list(ports)?.iter().map(|m| Port { name: s(m, "name"), kind: s(m, "kind"), id: s(m, "id") }).collect())
    }

    /// Send up to `count` out of a decoder (by name, or the first one). Returns how many left.
    pub fn extract(&self, key: &str, count: i64, decoder: Option<&str>) -> Result<i64, Error> {
        match self.call("extract", &[Value::Str(key.to_string()), Value::Int(count), opt_str(decoder)])? {
            Value::Int(n) => Ok(n),
            _ => Err(Error::Malformed("extract result is not an int")),
        }
    }

    pub fn tokens_issued(&self) -> Result<Vec<TokenInfo>, Error> {
        let v = self.call("tokens_issued", &[])?;
        Ok(list(v)?
            .iter()
            .map(|m| TokenInfo { token: s(m, "token"), key: s(m, "key"), name: s(m, "name"), count: i(m, "count"), expires_in: i(m, "expires_in") })
            .collect())
    }

    /// Items from expired tokens waiting in lost &amp; found.
    pub fn lost(&self) -> Result<Vec<TokenInfo>, Error> {
        let v = self.call("lost", &[])?;
        Ok(list(v)?
            .iter()
            .map(|m| TokenInfo { token: String::new(), key: s(m, "key"), name: s(m, "name"), count: i(m, "count"), expires_in: 0 })
            .collect())
    }

    pub fn claim_lost(&self) -> Result<i64, Error> {
        match self.call("claim_lost", &[])? {
            Value::Int(n) => Ok(n),
            _ => Err(Error::Malformed("claim_lost result is not an int")),
        }
    }
}

/// A count in at most 4 characters, for slot labels: `999`, `2.9k`, `12k`,
/// `340k`, `3.4M`, `45M`, `1.2G`.
pub fn short_count(n: i64) -> String {
    use core::fmt::Write;
    let mut out = String::new();
    let n = n.max(0);
    let units = [(1_000_000_000i64, "G"), (1_000_000, "M"), (1_000, "k")];
    if n < 1000 {
        let _ = write!(out, "{}", n);
        return out;
    }
    for (scale, unit) in units {
        if n >= scale {
            let whole = n / scale;
            if whole < 10 {
                let _ = write!(out, "{}.{}{}", whole, (n / (scale / 10)) % 10, unit);
            } else if whole < 1000 {
                let _ = write!(out, "{}{}", whole, unit);
            } else {
                let _ = write!(out, "999{}", unit);
            }
            return out;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_counts() {
        assert_eq!(short_count(64), "64");
        assert_eq!(short_count(999), "999");
        assert_eq!(short_count(2_969), "2.9k");
        assert_eq!(short_count(12_500), "12k");
        assert_eq!(short_count(340_000), "340k");
        assert_eq!(short_count(3_400_000), "3.4M");
        assert_eq!(short_count(45_000_000), "45M");
        assert_eq!(short_count(1_250_000_000), "1.2G");
        for n in [0, 5, 999, 1_000, 9_999, 99_999, 999_999, 12_345_678, i64::MAX] {
            assert!(short_count(n).len() <= 4, "{} -> {}", n, short_count(n));
        }
    }

    #[test]
    fn reads_maps() {
        let m = Value::Map(vec![(Value::Str("key".into()), Value::Str("k1".into())), (Value::Str("count".into()), Value::Int(7))]);
        assert_eq!(s(&m, "key"), "k1");
        assert_eq!(i(&m, "count"), 7);
        assert_eq!(i(&m, "missing"), 0);
    }
}
