//! Strict big-endian codec helpers shared by the air, backhaul and tunnel
//! formats. Every decoder reads through [`R`], which fails (never panics) on
//! short input, and callers end with [`R::end`] so trailing bytes are an error.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The input ended inside a field.
    Short,
    /// Bytes left over after the message.
    Trailing,
    /// Unknown message / item type.
    Type(u8),
    /// A field holds a value the format does not allow.
    Value(&'static str),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Short => write!(f, "truncated"),
            DecodeError::Trailing => write!(f, "trailing bytes"),
            DecodeError::Type(t) => write!(f, "unknown type 0x{:02x}", t),
            DecodeError::Value(w) => write!(f, "bad {}", w),
        }
    }
}

pub type DResult<T> = Result<T, DecodeError>;

/// Writer.
#[derive(Default)]
pub struct W(pub Vec<u8>);

impl W {
    pub fn new() -> W {
        W(Vec::new())
    }
    pub fn u8(&mut self, v: u8) -> &mut W {
        self.0.push(v);
        self
    }
    pub fn i8(&mut self, v: i8) -> &mut W {
        self.0.push(v as u8);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut W {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn i16(&mut self, v: i16) -> &mut W {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut W {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn u64(&mut self, v: u64) -> &mut W {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn raw(&mut self, v: &[u8]) -> &mut W {
        self.0.extend_from_slice(v);
        self
    }
    /// `u8 len` + UTF-8 bytes (at most 255).
    pub fn str8(&mut self, s: &str) -> &mut W {
        let b = s.as_bytes();
        let n = b.len().min(255);
        self.0.push(n as u8);
        self.0.extend_from_slice(&b[..n]);
        self
    }
    /// `u16 len` + bytes.
    pub fn blob16(&mut self, b: &[u8]) -> &mut W {
        let n = b.len().min(u16::MAX as usize);
        self.u16(n as u16);
        self.0.extend_from_slice(&b[..n]);
        self
    }
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

/// Reader over a byte slice.
pub struct R<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> R<'a> {
    pub fn new(b: &'a [u8]) -> R<'a> {
        R { b, p: 0 }
    }
    pub fn left(&self) -> usize {
        self.b.len() - self.p
    }
    pub fn take(&mut self, n: usize) -> DResult<&'a [u8]> {
        if self.left() < n {
            return Err(DecodeError::Short);
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.b[self.p..];
        self.p = self.b.len();
        s
    }
    pub fn u8(&mut self) -> DResult<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn i8(&mut self) -> DResult<i8> {
        Ok(self.u8()? as i8)
    }
    pub fn u16(&mut self) -> DResult<u16> {
        let s = self.take(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }
    pub fn i16(&mut self) -> DResult<i16> {
        Ok(self.u16()? as i16)
    }
    pub fn u32(&mut self) -> DResult<u32> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    pub fn u64(&mut self) -> DResult<u64> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(u64::from_be_bytes(a))
    }
    pub fn arr<const N: usize>(&mut self) -> DResult<[u8; N]> {
        let s = self.take(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Ok(a)
    }
    pub fn str8(&mut self) -> DResult<String> {
        let n = self.u8()? as usize;
        let s = self.take(n)?;
        core::str::from_utf8(s).map(String::from).map_err(|_| DecodeError::Value("utf-8"))
    }
    pub fn blob16(&mut self) -> DResult<&'a [u8]> {
        let n = self.u16()? as usize;
        self.take(n)
    }
    /// A count prefix that cannot ask for more elements than bytes remain
    /// (each element is at least `min_size` bytes): stops garbage from
    /// allocating huge vectors.
    pub fn count8(&mut self, min_size: usize) -> DResult<usize> {
        let n = self.u8()? as usize;
        if n * min_size.max(1) > self.left() {
            return Err(DecodeError::Short);
        }
        Ok(n)
    }
    pub fn count16(&mut self, min_size: usize) -> DResult<usize> {
        let n = self.u16()? as usize;
        if n * min_size.max(1) > self.left() {
            return Err(DecodeError::Short);
        }
        Ok(n)
    }
    pub fn end(&self) -> DResult<()> {
        if self.p != self.b.len() {
            return Err(DecodeError::Trailing);
        }
        Ok(())
    }
}

/// Lower-case hex.
pub fn hex(b: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for &x in b {
        s.push(D[(x >> 4) as usize] as char);
        s.push(D[(x & 15) as usize] as char);
    }
    s
}

/// Parse hex (either case) of exactly `N` bytes.
pub fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let s = s.as_bytes();
    if s.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for i in 0..N {
        let h = (s[2 * i] as char).to_digit(16)?;
        let l = (s[2 * i + 1] as char).to_digit(16)?;
        out[i] = (h * 16 + l) as u8;
    }
    Some(out)
}

/// Dotted-quad IPv4.
pub fn fmt_ip(ip: [u8; 4]) -> String {
    alloc::format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

pub fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut n = 0;
    for part in s.split('.') {
        if n == 4 || part.is_empty() || part.len() > 3 || !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let v: u16 = part.parse().ok()?;
        if v > 255 {
            return None;
        }
        out[n] = v as u8;
        n += 1;
    }
    (n == 4).then_some(out)
}

/// `a.b.c.d/len`.
pub fn parse_cidr(s: &str) -> Option<([u8; 4], u8)> {
    let (ip, len) = s.split_once('/')?;
    let len: u8 = len.parse().ok()?;
    if len > 32 {
        return None;
    }
    Some((parse_ip(ip)?, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_is_strict() {
        let mut w = W::new();
        w.u8(1).u16(2).u32(3).u64(4).str8("hi").blob16(&[9, 9]);
        let b = w.into_vec();
        let mut r = R::new(&b);
        assert_eq!(r.u8(), Ok(1));
        assert_eq!(r.u16(), Ok(2));
        assert_eq!(r.u32(), Ok(3));
        assert_eq!(r.u64(), Ok(4));
        assert_eq!(r.str8().as_deref(), Ok("hi"));
        assert_eq!(r.blob16(), Ok(&[9u8, 9][..]));
        assert_eq!(r.end(), Ok(()));
        assert_eq!(r.u8(), Err(DecodeError::Short));
        let mut r = R::new(&b[..3]);
        r.u8().unwrap();
        assert_eq!(r.end(), Err(DecodeError::Trailing));
        // a count asking for more elements than bytes left
        let mut r = R::new(&[200, 1, 2]);
        assert_eq!(r.count8(1), Err(DecodeError::Short));
    }

    #[test]
    fn hex_ip_cidr() {
        assert_eq!(hex(&[0, 0xab, 0xff]), "00abff");
        assert_eq!(unhex::<3>("00ABff"), Some([0, 0xab, 0xff]));
        assert_eq!(unhex::<3>("00ab"), None);
        assert_eq!(unhex::<1>("zz"), None);
        assert_eq!(parse_ip("10.0.0.256"), None);
        assert_eq!(parse_ip("10.0.0"), None);
        assert_eq!(parse_ip("10.0.0.1.2"), None);
        assert_eq!(parse_ip("100.65.2.1"), Some([100, 65, 2, 1]));
        assert_eq!(parse_cidr("100.65.2.0/24"), Some(([100, 65, 2, 0], 24)));
        assert_eq!(parse_cidr("100.65.2.0/33"), None);
        assert_eq!(fmt_ip([1, 2, 3, 4]), "1.2.3.4");
    }
}
