//! A small JSON value with a strict parser and a canonical writer (the
//! `.card` file), the shape of `ecm_ui::json` without its std dependency:
//! objects keep their key order, the writer puts no spaces in, integers
//! have no fraction.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

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
            Json::Float(f) if *f == (*f as i64) as f64 => Some(*f as i64),
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
                    let _ = write!(o, "{f:?}");
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
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn err(&self, what: &str) -> String {
        let mut m = String::new();
        let _ = write!(m, "json: {what} at byte {}", self.i);
        m
    }
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.err("bad literal"))
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        if self.depth > 64 {
            return Err(self.err("nested too deeply"));
        }
        match self.peek() {
            None => Err(self.err("unexpected end")),
            Some(b'n') => self.lit("null", Json::Null),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                self.depth += 1;
                let mut v = Vec::new();
                self.ws();
                if !self.eat(b']') {
                    loop {
                        self.ws();
                        v.push(self.value()?);
                        self.ws();
                        if self.eat(b']') {
                            break;
                        }
                        if !self.eat(b',') {
                            return Err(self.err("expected , or ]"));
                        }
                    }
                }
                self.depth -= 1;
                Ok(Json::Arr(v))
            }
            Some(b'{') => {
                self.i += 1;
                self.depth += 1;
                let mut kv = Vec::new();
                self.ws();
                if !self.eat(b'}') {
                    loop {
                        self.ws();
                        if self.peek() != Some(b'"') {
                            return Err(self.err("expected a key"));
                        }
                        let k = self.string()?;
                        self.ws();
                        if !self.eat(b':') {
                            return Err(self.err("expected :"));
                        }
                        self.ws();
                        let v = self.value()?;
                        kv.push((k, v));
                        self.ws();
                        if self.eat(b'}') {
                            break;
                        }
                        if !self.eat(b',') {
                            return Err(self.err("expected , or }"));
                        }
                    }
                }
                self.depth -= 1;
                Ok(Json::Obj(kv))
            }
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }
    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        let mut float = false;
        if self.eat(b'-') {}
        while let Some(c) = self.peek() {
            match c {
                b'0'..=b'9' => {}
                b'.' | b'e' | b'E' | b'+' | b'-' => float = true,
                _ => break,
            }
            self.i += 1;
        }
        let text = core::str::from_utf8(&self.s[start..self.i]).map_err(|_| self.err("bad number"))?;
        if !float {
            if let Ok(i) = text.parse::<i64>() {
                return Ok(Json::Int(i));
            }
        }
        text.parse::<f64>().map(Json::Float).map_err(|_| self.err("bad number"))
    }
    fn hex4(&mut self) -> Result<u32, String> {
        let chunk = self.s.get(self.i..self.i + 4).ok_or_else(|| self.err("bad \\u escape"))?;
        let text = core::str::from_utf8(chunk).map_err(|_| self.err("bad \\u escape"))?;
        let v = u32::from_str_radix(text, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(v)
    }
    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // opening quote
        let mut out = Vec::new();
        loop {
            let c = self.peek().ok_or_else(|| self.err("unterminated string"))?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = self.peek().ok_or_else(|| self.err("unterminated escape"))?;
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) && self.s[self.i..].starts_with(b"\\u") {
                                self.i += 2;
                                let lo = self.hex4()?;
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                c if c < 0x20 => return Err(self.err("control character in string")),
                c => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| self.err("string is not UTF-8"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn round_trip_and_errors() {
        let text = r#"{"kind":"bank","palette":[[255,0,0],[0,0,0]],"n":-12,"f":1.5,"ok":true,"none":null,"s":"a\"b\\c\n\u00e9"}"#;
        let v = Json::parse(text).unwrap();
        assert_eq!(v.get("kind").unwrap().as_str(), Some("bank"));
        assert_eq!(v.get("palette").unwrap().as_array().unwrap()[0].as_array().unwrap()[0].as_i64(), Some(255));
        assert_eq!(v.get("n").unwrap().as_i64(), Some(-12));
        assert_eq!(v.get("f"), Some(&Json::Float(1.5)));
        assert_eq!(v.get("s").unwrap().as_str(), Some("a\"b\\c\né"));
        let out = v.to_string();
        assert_eq!(Json::parse(&out).unwrap(), v);
        assert_eq!(Json::parse(" [ 1 , 2 ] ").unwrap(), Json::Arr(alloc::vec![Json::Int(1), Json::Int(2)]));
        for bad in ["", "{", "[1,]", "{\"a\" 1}", "tru", "\"x", "1 2", "{a:1}"] {
            assert!(Json::parse(bad).is_err(), "{bad} parsed");
        }
        assert_eq!(Json::Str("\u{1}".to_string()).to_string(), "\"\\u0001\"");
    }
}
