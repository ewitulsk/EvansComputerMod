//! Peripheral host function wrappers and the value encoding they use.
//!
//! A computer's peripherals are the blocks on its six sides and the modules
//! in its bays, each known by an attachment name (`"left"`, `"left_bay_1"`).
//! Programs list them, call their methods and wait for their events.
//!
//! Values cross the boundary in a tagged binary encoding shared with the Java
//! host (`computer/peripheral/PeripheralValues.java`), little-endian:
//!
//! ```text
//! value  = tag:u8 payload
//!   0 Nil   | 1 Str  len:u32 utf8 | 2 I32 i32 | 3 I64 i64 | 4 F64 f64
//!   5 Bool u8 | 6 List count:u32 value* | 7 Map count:u32 (key value)*
//!   8 Bytes len:u32 u8*
//! frame  = status:u8 value      0 = ok, 1 = error (value is a Str message)
//! ```
//!
//! Every host call returns the frame length. If it exceeds the buffer the
//! host keeps the frame and [`take_pending`] fetches it; the wrappers here do
//! that transparently.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[cfg(target_arch = "wasm32")]
extern "C" {
    fn periph_list(buf: i32, cap: i32) -> i32;
    fn periph_methods(name: i32, name_len: i32, buf: i32, cap: i32) -> i32;
    fn periph_call(
        name: i32,
        name_len: i32,
        method: i32,
        method_len: i32,
        args: i32,
        args_len: i32,
        buf: i32,
        cap: i32,
    ) -> i32;
    fn periph_wait_event(filter: i32, filter_len: i32, timeout_ms: i32, buf: i32, cap: i32) -> i32;
    fn periph_take_pending(buf: i32, cap: i32) -> i32;
}

// Native builds (unit tests of programs that use this module) have no host:
// every call reports "unavailable" (-1).
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
mod native {
    pub unsafe fn periph_list(_: i32, _: i32) -> i32 { -1 }
    pub unsafe fn periph_methods(_: i32, _: i32, _: i32, _: i32) -> i32 { -1 }
    pub unsafe fn periph_call(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 { -1 }
    pub unsafe fn periph_wait_event(_: i32, _: i32, _: i32, _: i32, _: i32) -> i32 { -1 }
    pub unsafe fn periph_take_pending(_: i32, _: i32) -> i32 { -1 }
}
#[cfg(not(target_arch = "wasm32"))]
use native::*;

const TAG_NIL: u8 = 0;
const TAG_STR: u8 = 1;
const TAG_I32: u8 = 2;
const TAG_I64: u8 = 3;
const TAG_F64: u8 = 4;
const TAG_BOOL: u8 = 5;
const TAG_LIST: u8 = 6;
const TAG_MAP: u8 = 7;
const TAG_BYTES: u8 = 8;

const MAX_DEPTH: usize = 32;
const INITIAL_BUF: usize = 16 * 1024;

/// A value passed to or returned from a peripheral.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Map(Vec<(Value, Value)>),
}

/// Error from the peripheral layer: the host's message, or a local failure.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// The peripheral (or the host) reported an error.
    Peripheral(String),
    /// The host bridge is unavailable (not running inside a computer).
    Unavailable,
    /// The host sent bytes that don't decode.
    Malformed(&'static str),
}

// ----------------------------------------------------------------- encoding

pub fn encode(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Nil => out.push(TAG_NIL),
        Value::Bool(b) => {
            out.push(TAG_BOOL);
            out.push(*b as u8);
        }
        Value::Int(i) => {
            if let Ok(small) = i32::try_from(*i) {
                out.push(TAG_I32);
                out.extend_from_slice(&small.to_le_bytes());
            } else {
                out.push(TAG_I64);
                out.extend_from_slice(&i.to_le_bytes());
            }
        }
        Value::Float(f) => {
            out.push(TAG_F64);
            out.extend_from_slice(&f.to_le_bytes());
        }
        Value::Str(s) => blob(out, TAG_STR, s.as_bytes()),
        Value::Bytes(b) => blob(out, TAG_BYTES, b),
        Value::List(items) => {
            out.push(TAG_LIST);
            out.extend_from_slice(&(items.len() as u32).to_le_bytes());
            for v in items {
                encode(v, out);
            }
        }
        Value::Map(entries) => {
            out.push(TAG_MAP);
            out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
            for (k, v) in entries {
                encode(k, out);
                encode(v, out);
            }
        }
    }
}

fn blob(out: &mut Vec<u8>, tag: u8, bytes: &[u8]) {
    out.push(tag);
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

/// Decode one value that fills `data` exactly.
pub fn decode(data: &[u8]) -> Result<Value, Error> {
    let mut pos = 0;
    let v = read(data, &mut pos, 0)?;
    if pos != data.len() {
        return Err(Error::Malformed("trailing bytes after value"));
    }
    Ok(v)
}

/// Decode a result frame: `Ok(value)` or the peripheral's error message.
pub fn decode_frame(frame: &[u8]) -> Result<Value, Error> {
    let (&status, body) = frame.split_first().ok_or(Error::Malformed("empty frame"))?;
    let value = decode(body)?;
    match status {
        0 => Ok(value),
        1 => match value {
            Value::Str(msg) => Err(Error::Peripheral(msg)),
            _ => Err(Error::Peripheral(String::from("error"))),
        },
        _ => Err(Error::Malformed("bad frame status")),
    }
}

fn take<'a>(data: &'a [u8], pos: &mut usize, n: usize) -> Result<&'a [u8], Error> {
    let end = pos.checked_add(n).ok_or(Error::Malformed("length overflow"))?;
    let s = data.get(*pos..end).ok_or(Error::Malformed("truncated value"))?;
    *pos = end;
    Ok(s)
}

fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32, Error> {
    let b = take(data, pos, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read(data: &[u8], pos: &mut usize, depth: usize) -> Result<Value, Error> {
    if depth > MAX_DEPTH {
        return Err(Error::Malformed("value nested too deeply"));
    }
    let tag = take(data, pos, 1)?[0];
    Ok(match tag {
        TAG_NIL => Value::Nil,
        TAG_BOOL => Value::Bool(take(data, pos, 1)?[0] != 0),
        TAG_I32 => {
            let b = take(data, pos, 4)?;
            Value::Int(i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i64)
        }
        TAG_I64 => {
            let b = take(data, pos, 8)?;
            let mut a = [0u8; 8];
            a.copy_from_slice(b);
            Value::Int(i64::from_le_bytes(a))
        }
        TAG_F64 => {
            let b = take(data, pos, 8)?;
            let mut a = [0u8; 8];
            a.copy_from_slice(b);
            Value::Float(f64::from_le_bytes(a))
        }
        TAG_STR => {
            let n = read_u32(data, pos)? as usize;
            let b = take(data, pos, n)?;
            Value::Str(String::from_utf8(b.to_vec()).map_err(|_| Error::Malformed("string is not UTF-8"))?)
        }
        TAG_BYTES => {
            let n = read_u32(data, pos)? as usize;
            Value::Bytes(take(data, pos, n)?.to_vec())
        }
        TAG_LIST => {
            let n = read_u32(data, pos)? as usize;
            if n > data.len() - *pos {
                return Err(Error::Malformed("bad list length"));
            }
            let mut items = Vec::with_capacity(n);
            for _ in 0..n {
                items.push(read(data, pos, depth + 1)?);
            }
            Value::List(items)
        }
        TAG_MAP => {
            let n = read_u32(data, pos)? as usize;
            if n > data.len() - *pos {
                return Err(Error::Malformed("bad map length"));
            }
            let mut entries = Vec::with_capacity(n);
            for _ in 0..n {
                let k = read(data, pos, depth + 1)?;
                let v = read(data, pos, depth + 1)?;
                entries.push((k, v));
            }
            Value::Map(entries)
        }
        _ => return Err(Error::Malformed("unknown value tag")),
    })
}

// ----------------------------------------------------------------- host calls

/// Run a host call that writes a frame into `(buf, cap)` and returns its
/// length, fetching an oversized frame through `periph_take_pending`.
/// Returns `None` when the call returned 0 (only `periph_wait_event`: timeout).
fn frame_call(call: impl FnOnce(i32, i32) -> i32) -> Result<Option<Vec<u8>>, Error> {
    let mut buf = vec![0u8; INITIAL_BUF];
    let n = call(buf.as_mut_ptr() as i32, buf.len() as i32);
    if n < 0 {
        return Err(Error::Unavailable);
    }
    if n == 0 {
        return Ok(None);
    }
    let n = n as usize;
    if n > buf.len() {
        buf = vec![0u8; n];
        let got = unsafe { periph_take_pending(buf.as_mut_ptr() as i32, n as i32) };
        if got < 0 || got as usize != n {
            return Err(Error::Malformed("pending result lost"));
        }
    }
    buf.truncate(n);
    Ok(Some(buf))
}

fn frame_value(call: impl FnOnce(i32, i32) -> i32) -> Result<Value, Error> {
    match frame_call(call)? {
        Some(frame) => decode_frame(&frame),
        None => Err(Error::Malformed("empty result")),
    }
}

/// Attached peripherals as `(name, type)` pairs.
pub fn list() -> Result<Vec<(String, String)>, Error> {
    let v = frame_value(|b, c| unsafe { periph_list(b, c) })?;
    let Value::List(items) = v else {
        return Err(Error::Malformed("list result is not a list"));
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if let Value::List(pair) = item {
            if let [Value::Str(name), Value::Str(ty)] = pair.as_slice() {
                out.push((name.clone(), ty.clone()));
                continue;
            }
        }
        return Err(Error::Malformed("list entry is not [name, type]"));
    }
    Ok(out)
}

/// The type and method names of the peripheral attached as `name`.
pub fn methods(name: &str) -> Result<(String, Vec<String>), Error> {
    let v = frame_value(|b, c| unsafe {
        periph_methods(name.as_ptr() as i32, name.len() as i32, b, c)
    })?;
    if let Value::List(parts) = v {
        if let [Value::Str(ty), Value::List(names)] = parts.as_slice() {
            let names = names
                .iter()
                .filter_map(|n| if let Value::Str(s) = n { Some(s.clone()) } else { None })
                .collect();
            return Ok((ty.clone(), names));
        }
    }
    Err(Error::Malformed("methods result is not [type, [names]]"))
}

/// Call `method` on the peripheral attached as `name`.
pub fn call(name: &str, method: &str, args: &[Value]) -> Result<Value, Error> {
    let mut encoded = Vec::new();
    encode(&Value::List(args.to_vec()), &mut encoded);
    frame_value(|b, c| unsafe {
        periph_call(
            name.as_ptr() as i32,
            name.len() as i32,
            method.as_ptr() as i32,
            method.len() as i32,
            encoded.as_ptr() as i32,
            encoded.len() as i32,
            b,
            c,
        )
    })
}

/// Wait for the next event named `filter` (any if `None`), up to `timeout_ms`
/// (0 = don't wait, negative = forever). Returns `(event, attachment, *args)`
/// as a list, or `None` on timeout.
pub fn wait_event(filter: Option<&str>, timeout_ms: i32) -> Result<Option<Vec<Value>>, Error> {
    let (fp, fl) = match filter {
        Some(f) => (f.as_ptr() as i32, f.len() as i32),
        None => (0, 0),
    };
    let Some(frame) = frame_call(|b, c| unsafe { periph_wait_event(fp, fl, timeout_ms, b, c) })? else {
        return Ok(None);
    };
    match decode_frame(&frame)? {
        Value::List(items) => Ok(Some(items)),
        _ => Err(Error::Malformed("event is not a list")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(v: Value) {
        let mut out = Vec::new();
        encode(&v, &mut out);
        assert_eq!(decode(&out).unwrap(), v);
    }

    #[test]
    fn values_roundtrip() {
        roundtrip(Value::Nil);
        roundtrip(Value::Bool(true));
        roundtrip(Value::Int(-5));
        roundtrip(Value::Int(1 << 40));
        roundtrip(Value::Float(1.5));
        roundtrip(Value::Str(String::from("minecraft:red_dye")));
        roundtrip(Value::Bytes(vec![0, 255, 7]));
        roundtrip(Value::List(vec![Value::Int(1), Value::Str(String::from("a")), Value::List(vec![])]));
        roundtrip(Value::Map(vec![(Value::Str(String::from("mode")), Value::Str(String::from("tx")))]));
    }

    #[test]
    fn small_ints_use_i32_tag() {
        let mut out = Vec::new();
        encode(&Value::Int(15), &mut out);
        assert_eq!(out, vec![TAG_I32, 15, 0, 0, 0]);
    }

    #[test]
    fn error_frame_carries_message() {
        let mut frame = vec![1u8];
        encode(&Value::Str(String::from("no peripheral named 'x'")), &mut frame);
        assert_eq!(decode_frame(&frame), Err(Error::Peripheral(String::from("no peripheral named 'x'"))));
    }

    #[test]
    fn rejects_truncated_and_hostile_input() {
        assert!(decode(&[TAG_STR, 10, 0, 0, 0, b'a']).is_err());
        assert!(decode(&[TAG_LIST, 0xFF, 0xFF, 0xFF, 0x7F]).is_err());
        assert!(decode(&[42]).is_err());
        assert!(decode(&[TAG_NIL, TAG_NIL]).is_err());
        let deep: Vec<u8> = core::iter::repeat([TAG_LIST, 1, 0, 0, 0]).take(40).flatten().collect();
        assert!(decode(&deep).is_err());
    }
}
