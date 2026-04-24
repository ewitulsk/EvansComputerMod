//! Computer module bridge host function wrappers.
//!
//! Provides access to Java-registered `@ComputerModule` methods from WASI
//! programs.  Uses a binary protocol for arguments / results and JSON for
//! the one-time metadata query.

extern crate alloc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

extern "C" {
    fn module_call(
        module_ptr: i32, module_len: i32,
        method_ptr: i32, method_len: i32,
        args_ptr: i32, args_len: i32,
        result_ptr: i32, result_len: i32,
    ) -> i32;

    fn module_list(buf_ptr: i32, buf_len: i32) -> i32;
}

// Type tags — must match Java side (ModuleMethodInvoker)
pub const TAG_NULL: u8   = 0x00;
pub const TAG_STRING: u8 = 0x01;
pub const TAG_I32: u8    = 0x02;
pub const TAG_I64: u8    = 0x03;
pub const TAG_F64: u8    = 0x04;
pub const TAG_BOOL: u8   = 0x05;

pub const STATUS_OK: u8    = 0x00;
pub const STATUS_ERROR: u8 = 0x01;

/// A typed value from the binary protocol.
#[derive(Debug)]
pub enum BinaryValue {
    Null,
    Str(String),
    I32(i32),
    I64(i64),
    F64(f64),
    Bool(bool),
}

/// Get JSON metadata describing all registered computer modules.
pub fn list_modules_raw() -> String {
    let mut buf = vec![0u8; 16384];
    let n = unsafe { module_list(buf.as_mut_ptr() as i32, buf.len() as i32) };
    if n <= 0 {
        return String::from("{\"modules\":{}}");
    }
    buf.truncate(n as usize);
    String::from_utf8(buf).unwrap_or_else(|_| String::from("{\"modules\":{}}"))
}

/// Call a method on a registered computer module using binary protocol.
pub fn call_method(module_name: &str, method_name: &str, args_bin: &[u8]) -> Result<BinaryValue, String> {
    let mut buf = vec![0u8; 16384];
    let n = unsafe {
        module_call(
            module_name.as_ptr() as i32, module_name.len() as i32,
            method_name.as_ptr() as i32, method_name.len() as i32,
            args_bin.as_ptr() as i32, args_bin.len() as i32,
            buf.as_mut_ptr() as i32, buf.len() as i32,
        )
    };
    if n < 0 {
        return Err(String::from("host call failed"));
    }
    parse_result(&buf[..n as usize])
}

fn parse_result(data: &[u8]) -> Result<BinaryValue, String> {
    if data.is_empty() {
        return Err(String::from("empty result"));
    }
    let status = data[0];
    let payload = &data[1..];

    if status == STATUS_ERROR {
        if payload.len() >= 5 && payload[0] == TAG_STRING {
            let len = u32_le(&payload[1..]) as usize;
            if payload.len() >= 5 + len {
                let msg = core::str::from_utf8(&payload[5..5 + len]).unwrap_or("error");
                return Err(String::from(msg));
            }
        }
        return Err(String::from("unknown error"));
    }

    if payload.is_empty() {
        return Ok(BinaryValue::Null);
    }
    read_tagged(payload).map(|(v, _)| v)
}

fn read_tagged(data: &[u8]) -> Result<(BinaryValue, usize), String> {
    if data.is_empty() {
        return Err(String::from("unexpected end"));
    }
    let tag = data[0];
    let p = &data[1..];
    match tag {
        TAG_NULL => Ok((BinaryValue::Null, 1)),
        TAG_STRING => {
            if p.len() < 4 { return Err(String::from("truncated string")); }
            let len = u32_le(p) as usize;
            if p.len() < 4 + len { return Err(String::from("truncated string data")); }
            let s = core::str::from_utf8(&p[4..4 + len]).unwrap_or("");
            Ok((BinaryValue::Str(String::from(s)), 1 + 4 + len))
        }
        TAG_I32 => {
            if p.len() < 4 { return Err(String::from("truncated i32")); }
            Ok((BinaryValue::I32(u32_le(p) as i32), 5))
        }
        TAG_I64 => {
            if p.len() < 8 { return Err(String::from("truncated i64")); }
            Ok((BinaryValue::I64(i64_le(p)), 9))
        }
        TAG_F64 => {
            if p.len() < 8 { return Err(String::from("truncated f64")); }
            Ok((BinaryValue::F64(f64::from_bits(i64_le(p) as u64)), 9))
        }
        TAG_BOOL => {
            if p.is_empty() { return Err(String::from("truncated bool")); }
            Ok((BinaryValue::Bool(p[0] != 0), 2))
        }
        _ => Err(String::from("unknown tag")),
    }
}

/// Encode typed arguments into the binary wire format.
pub fn encode_args(args: &[BinaryValue]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(args.len() as u8);
    for arg in args {
        match arg {
            BinaryValue::Null => buf.push(TAG_NULL),
            BinaryValue::Str(s) => {
                buf.push(TAG_STRING);
                let b = s.as_bytes();
                push_u32_le(&mut buf, b.len() as u32);
                buf.extend_from_slice(b);
            }
            BinaryValue::I32(v) => { buf.push(TAG_I32); push_u32_le(&mut buf, *v as u32); }
            BinaryValue::I64(v) => { buf.push(TAG_I64); push_i64_le(&mut buf, *v); }
            BinaryValue::F64(v) => { buf.push(TAG_F64); push_i64_le(&mut buf, v.to_bits() as i64); }
            BinaryValue::Bool(b) => { buf.push(TAG_BOOL); buf.push(if *b { 1 } else { 0 }); }
        }
    }
    buf
}

fn u32_le(d: &[u8]) -> u32 {
    (d[0] as u32) | ((d[1] as u32) << 8) | ((d[2] as u32) << 16) | ((d[3] as u32) << 24)
}

fn i64_le(d: &[u8]) -> i64 {
    let lo = u32_le(d) as u64;
    let hi = u32_le(&d[4..]) as u64;
    (lo | (hi << 32)) as i64
}

fn push_u32_le(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_i64_le(buf: &mut Vec<u8>, v: i64) {
    buf.extend_from_slice(&v.to_le_bytes());
}
