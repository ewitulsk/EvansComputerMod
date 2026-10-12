//! Primitive encoding shared by the generated message code: little-endian
//! integers, `u32`-length strings / byte strings / arrays, and the canonical
//! JSON rendering used by the shared test vectors.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Why a buffer is not a valid message. Field names are `Message.field` paths.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Not a whole message yet: at least this many bytes are needed in total.
    Incomplete(usize),
    /// The header's opcode is not part of this protocol version.
    UnknownOpcode(u16),
    /// The header's flags are not 0.
    HeaderFlags,
    /// The header's length exceeds `MAX_MESSAGE`.
    TooLarge(u32),
    /// A field runs past the end of the body.
    Truncated(&'static str),
    /// Bytes left over after the last field.
    Trailing,
    /// A bool that is not 0 or 1.
    BadBool(&'static str),
    /// A string that is not valid UTF-8.
    BadUtf8(&'static str),
    /// An enum value, union tag or flag bit that isn't defined.
    BadEnum(&'static str),
    BadFlags(&'static str),
    /// A string, byte string or array over its limit.
    TooLong(&'static str),
    /// An integer outside its range or set.
    OutOfRange(&'static str),
    /// A sorted list that isn't strictly increasing.
    Unsorted(&'static str),
    /// A node prop not accepted by the node's kind.
    PropNotAllowed(&'static str),
}

impl DecodeError {
    /// The protocol `ErrorCode` the compositor answers this with.
    pub fn error_code(&self) -> crate::ErrorCode {
        match self {
            DecodeError::UnknownOpcode(_) => crate::ErrorCode::UnknownOpcode,
            DecodeError::TooLarge(_) => crate::ErrorCode::Limit,
            _ => crate::ErrorCode::Malformed,
        }
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Incomplete(n) => write!(f, "incomplete message (need {n} bytes)"),
            DecodeError::UnknownOpcode(op) => write!(f, "unknown opcode {op:#06x}"),
            DecodeError::HeaderFlags => write!(f, "header flags must be 0"),
            DecodeError::TooLarge(n) => write!(f, "message body of {n} bytes is too large"),
            DecodeError::Truncated(p) => write!(f, "truncated at {p}"),
            DecodeError::Trailing => write!(f, "trailing bytes after the last field"),
            DecodeError::BadBool(p) => write!(f, "bad bool at {p}"),
            DecodeError::BadUtf8(p) => write!(f, "invalid UTF-8 at {p}"),
            DecodeError::BadEnum(p) => write!(f, "bad enum value at {p}"),
            DecodeError::BadFlags(p) => write!(f, "undefined flag bits at {p}"),
            DecodeError::TooLong(p) => write!(f, "too long: {p}"),
            DecodeError::OutOfRange(p) => write!(f, "out of range: {p}"),
            DecodeError::Unsorted(p) => write!(f, "not sorted or duplicated: {p}"),
            DecodeError::PropNotAllowed(p) => write!(f, "prop not allowed for this node kind: {p}"),
        }
    }
}

/// Appends little-endian fields to a byte vector.
pub struct Writer<'a> {
    pub buf: &'a mut Vec<u8>,
}

impl<'a> Writer<'a> {
    pub fn new(buf: &'a mut Vec<u8>) -> Self {
        Writer { buf }
    }
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }
    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.buf.extend_from_slice(b);
    }
}

/// Reads little-endian fields from one message body.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.remaining() < n {
            return Err(DecodeError::Truncated("body"));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn arr<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }

    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.arr()?))
    }
    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.arr()?))
    }
    pub fn i16(&mut self) -> Result<i16, DecodeError> {
        Ok(i16::from_le_bytes(self.arr()?))
    }
    pub fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.arr()?))
    }
    pub fn bool(&mut self, path: &'static str) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::BadBool(path)),
        }
    }

    fn len_prefixed(&mut self, max: u32, path: &'static str) -> Result<&'a [u8], DecodeError> {
        let n = self.u32()?;
        if n > max {
            return Err(DecodeError::TooLong(path));
        }
        if n as usize > self.remaining() {
            return Err(DecodeError::Truncated(path));
        }
        self.take(n as usize)
    }

    pub fn str(&mut self, max: u32, path: &'static str) -> Result<String, DecodeError> {
        let raw = self.len_prefixed(max, path)?;
        core::str::from_utf8(raw).map(String::from).map_err(|_| DecodeError::BadUtf8(path))
    }

    pub fn bytes(&mut self, max: u32, path: &'static str) -> Result<Vec<u8>, DecodeError> {
        Ok(self.len_prefixed(max, path)?.to_vec())
    }

    /// An array count: at most `max`, and small enough that `count` items of
    /// at least `min_item` bytes each fit in what is left (so a hostile count
    /// can't make us allocate).
    pub fn count(&mut self, max: u32, min_item: usize, path: &'static str) -> Result<usize, DecodeError> {
        let n = self.u32()?;
        if n > max {
            return Err(DecodeError::TooLong(path));
        }
        if (n as usize).saturating_mul(min_item) > self.remaining() {
            return Err(DecodeError::Truncated(path));
        }
        Ok(n as usize)
    }

    pub fn finish(&self) -> Result<(), DecodeError> {
        if self.remaining() != 0 {
            return Err(DecodeError::Trailing);
        }
        Ok(())
    }
}

// ------------------------------------------------------------- canonical JSON

/// A JSON string: escapes only `"`, `\` and U+0000..U+001F (as `\u00xx`).
pub fn json_str(o: &mut String, s: &str) {
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = fmt::Write::write_fmt(o, format_args!("\\u{:04x}", c as u32));
            }
            c => o.push(c),
        }
    }
    o.push('"');
}

/// Bytes as a lowercase hex JSON string.
pub fn json_hex(o: &mut String, b: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    o.push('"');
    for &x in b {
        o.push(HEX[(x >> 4) as usize] as char);
        o.push(HEX[(x & 15) as usize] as char);
    }
    o.push('"');
}

pub fn json_u(o: &mut String, v: u64) {
    let _ = fmt::Write::write_fmt(o, format_args!("{v}"));
}

pub fn json_i(o: &mut String, v: i64) {
    let _ = fmt::Write::write_fmt(o, format_args!("{v}"));
}
