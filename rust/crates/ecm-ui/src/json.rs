//! A small JSON value with a strict parser and a canonical writer.
//!
//! Used for `.ui.json` layout files, saved app state (`SaveState` /
//! `Restore` bytes) and the Python bridge. Objects keep their key order, and
//! [`Json::to_string`] is canonical (no spaces, `"` `\` and control
//! characters escaped, integers without a fraction), so Rust and Python apps
//! that save the same data send the same bytes.

use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Int(i) => Some(*i),
            Json::Float(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => Some(*f as i64),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Obj(v) => Some(v),
            _ => None,
        }
    }

    /// Canonical compact text.
    pub fn to_string(&self) -> String {
        let mut o = String::new();
        self.write(&mut o);
        o
    }

    pub fn write(&self, o: &mut String) {
        match self {
            Json::Null => o.push_str("null"),
            Json::Bool(b) => o.push_str(if *b { "true" } else { "false" }),
            Json::Int(i) => {
                let _ = write!(o, "{i}");
            }
            Json::Float(f) => {
                if f.is_finite() {
                    if f.fract() == 0.0 && f.abs() < 1e15 {
                        let _ = write!(o, "{f:.1}");
                    } else {
                        let _ = write!(o, "{f}");
                    }
                } else {
                    o.push_str("null");
                }
            }
            Json::Str(s) => write_str(o, s),
            Json::Arr(v) => {
                o.push('[');
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    x.write(o);
                }
                o.push(']');
            }
            Json::Obj(kv) => {
                o.push('{');
                for (i, (k, v)) in kv.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    write_str(o, k);
                    o.push(':');
                    v.write(o);
                }
                o.push('}');
            }
        }
    }

    pub fn parse(text: &str) -> Result<Json, String> {
        let mut p = Parser { s: text.as_bytes(), i: 0, depth: 0 };
        p.ws();
        let v = p.value()?;
        p.ws();
        if p.i != p.s.len() {
            return Err(p.err("trailing characters"));
        }
        Ok(v)
    }
}

fn write_str(o: &mut String, s: &str) {
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
}

const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

impl Parser<'_> {
    fn err(&self, what: &str) -> String {
        let line = 1 + self.s[..self.i.min(self.s.len())].iter().filter(|&&b| b == b'\n').count();
        format!("JSON error at line {line}: {what}")
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        if self.i >= self.s.len() {
            return Err(self.err("unexpected end"));
        }
        let c = self.s[self.i];
        match c {
            b'{' => self.nested(|p| p.object()),
            b'[' => self.nested(|p| p.array()),
            b'"' => self.string().map(Json::Str),
            b't' if self.eat("true") => Ok(Json::Bool(true)),
            b'f' if self.eat("false") => Ok(Json::Bool(false)),
            b'n' if self.eat("null") => Ok(Json::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(self.err("unexpected character")),
        }
    }

    fn nested(&mut self, f: impl FnOnce(&mut Self) -> Result<Json, String>) -> Result<Json, String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err("nested too deeply"));
        }
        let r = f(self);
        self.depth -= 1;
        r
    }

    fn object(&mut self) -> Result<Json, String> {
        self.i += 1;
        let mut kv: Vec<(String, Json)> = Vec::new();
        self.ws();
        if self.eat("}") {
            return Ok(Json::Obj(kv));
        }
        loop {
            self.ws();
            if self.i >= self.s.len() || self.s[self.i] != b'"' {
                return Err(self.err("expected a key"));
            }
            let k = self.string()?;
            if kv.iter().any(|(e, _)| *e == k) {
                return Err(self.err(&format!("duplicate key {k:?}")));
            }
            self.ws();
            if !self.eat(":") {
                return Err(self.err("expected ':'"));
            }
            self.ws();
            let v = self.value()?;
            kv.push((k, v));
            self.ws();
            if self.eat(",") {
                continue;
            }
            if self.eat("}") {
                return Ok(Json::Obj(kv));
            }
            return Err(self.err("expected ',' or '}'"));
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.i += 1;
        let mut v = Vec::new();
        self.ws();
        if self.eat("]") {
            return Ok(Json::Arr(v));
        }
        loop {
            self.ws();
            v.push(self.value()?);
            self.ws();
            if self.eat(",") {
                continue;
            }
            if self.eat("]") {
                return Ok(Json::Arr(v));
            }
            return Err(self.err("expected ',' or ']'"));
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        if self.i + 4 > self.s.len() {
            return Err(self.err("bad \\u escape"));
        }
        let h = std::str::from_utf8(&self.s[self.i..self.i + 4]).map_err(|_| self.err("bad \\u escape"))?;
        let v = u32::from_str_radix(h, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            if self.i >= self.s.len() {
                return Err(self.err("unterminated string"));
            }
            let b = self.s[self.i];
            match b {
                b'"' => {
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.i += 1;
                    let e = *self.s.get(self.i).ok_or_else(|| self.err("bad escape"))?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut c = self.hex4()?;
                            if (0xD800..0xDC00).contains(&c) && self.eat("\\u") {
                                let lo = self.hex4()?;
                                c = 0x10000 + ((c - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            out.push(char::from_u32(c).ok_or_else(|| self.err("bad code point"))?);
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                b if b < 0x20 => return Err(self.err("control character in string")),
                _ => {
                    // Copy one UTF-8 sequence (the input is a &str, so it is valid).
                    let len = match b {
                        0x00..=0x7F => 1,
                        0xC0..=0xDF => 2,
                        0xE0..=0xEF => 3,
                        _ => 4,
                    };
                    out.push_str(std::str::from_utf8(&self.s[self.i..self.i + len]).map_err(|_| self.err("bad UTF-8"))?);
                    self.i += len;
                }
            }
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        if self.s[self.i] == b'-' {
            self.i += 1;
        }
        let mut float = false;
        while self.i < self.s.len() {
            match self.s[self.i] {
                b'0'..=b'9' => {}
                b'.' | b'e' | b'E' | b'+' | b'-' => float = true,
                _ => break,
            }
            self.i += 1;
        }
        let t = std::str::from_utf8(&self.s[start..self.i]).unwrap();
        if !float {
            if let Ok(i) = t.parse::<i64>() {
                return Ok(Json::Int(i));
            }
        }
        t.parse::<f64>().map(Json::Float).map_err(|_| self.err("bad number"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_write_round_trip() {
        let text = r#"{ "a": [1, -2, 3.5, true, null], "b": "x\"y\\z\né😀", "c": {} }"#;
        let v = Json::parse(text).unwrap();
        assert_eq!(v.get("a").unwrap().as_array().unwrap()[1], Json::Int(-2));
        assert_eq!(v.get("b").unwrap().as_str().unwrap(), "x\"y\\z\n\u{e9}\u{1f600}");
        let out = v.to_string();
        assert_eq!(out, "{\"a\":[1,-2,3.5,true,null],\"b\":\"x\\\"y\\\\z\\n\u{e9}\u{1f600}\",\"c\":{}}");
        assert_eq!(Json::parse(&out).unwrap(), v);
    }

    #[test]
    fn rejects_bad_input() {
        for bad in ["{", "[1,]", "{\"a\":1,\"a\":2}", "01x", "\"\u{1}\"", "[] []", "tru"] {
            assert!(Json::parse(bad).is_err(), "{bad} parsed");
        }
    }
}
