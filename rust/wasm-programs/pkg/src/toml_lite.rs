//! The TOML subset `app.toml` is written in (docs/gui/APPS.md, "TOML subset").
//!
//! Supported: `key = value` lines, `#` comments, one level of `[table]` headers,
//! basic strings (`"..."` with `\" \\ \n \t \r \uXXXX`), literal strings (`'...'`),
//! decimal integers, booleans, arrays (may span lines, trailing comma allowed) and
//! inline tables. Not supported (a syntax error): dotted keys, nested headers,
//! arrays of tables (`[[x]]`), multi-line strings, floats and dates.
//! The Java app manager has the same parser (computer/apps/MiniToml.java); both run
//! the vectors in abi/app-manifest-vectors.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    Array(Vec<Value>),
    Table(BTreeMap<String, Value>),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Str(_) => "string",
            Value::Int(_) => "integer",
            Value::Bool(_) => "boolean",
            Value::Array(_) => "array",
            Value::Table(_) => "table",
        }
    }
}

pub type Table = BTreeMap<String, Value>;

struct P<'a> {
    s: &'a [u8],
    i: usize,
    line: usize,
}

pub fn parse(text: &str) -> Result<Table, String> {
    let mut p = P { s: text.as_bytes(), i: 0, line: 1 };
    let mut root = Table::new();
    let mut current: Option<String> = None;
    loop {
        p.skip_ws_comments_newlines();
        if p.i >= p.s.len() {
            break;
        }
        if p.peek() == Some(b'[') {
            p.i += 1;
            if p.peek() == Some(b'[') {
                return Err(p.err("arrays of tables ([[...]]) are not supported"));
            }
            p.skip_inline_ws();
            let name = p.key()?;
            p.skip_inline_ws();
            if p.peek() != Some(b']') {
                return Err(p.err("expected ']' after the table name"));
            }
            p.i += 1;
            p.end_of_line()?;
            if root.contains_key(&name) {
                return Err(p.err(&format!("duplicate key '{}'", name)));
            }
            root.insert(name.clone(), Value::Table(Table::new()));
            current = Some(name);
            continue;
        }
        let key = p.key()?;
        p.skip_inline_ws();
        if p.peek() != Some(b'=') {
            return Err(p.err(&format!("expected '=' after '{}'", key)));
        }
        p.i += 1;
        p.skip_inline_ws();
        let v = p.value()?;
        p.end_of_line()?;
        let target = match &current {
            None => &mut root,
            Some(t) => match root.get_mut(t) {
                Some(Value::Table(m)) => m,
                _ => unreachable!(),
            },
        };
        if target.contains_key(&key) {
            return Err(p.err(&format!("duplicate key '{}'", key)));
        }
        target.insert(key, v);
    }
    Ok(root)
}

impl<'a> P<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn err(&self, msg: &str) -> String {
        format!("line {}: {}", self.line, msg)
    }

    fn skip_inline_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == b' ' || c == b'\t' {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn skip_comment(&mut self) {
        if self.peek() == Some(b'#') {
            while let Some(c) = self.peek() {
                if c == b'\n' {
                    break;
                }
                self.i += 1;
            }
        }
    }

    /// Whitespace, comments and newlines (inside arrays and between statements).
    fn skip_ws_comments_newlines(&mut self) {
        loop {
            self.skip_inline_ws();
            self.skip_comment();
            match self.peek() {
                Some(b'\n') => {
                    self.line += 1;
                    self.i += 1;
                }
                Some(b'\r') => self.i += 1,
                _ => break,
            }
        }
    }

    fn end_of_line(&mut self) -> Result<(), String> {
        self.skip_inline_ws();
        self.skip_comment();
        match self.peek() {
            None => Ok(()),
            Some(b'\n') => Ok(()),
            Some(b'\r') if self.s.get(self.i + 1) == Some(&b'\n') => Ok(()),
            Some(c) => Err(self.err(&format!("unexpected '{}' after the value", c as char))),
        }
    }

    fn key(&mut self) -> Result<String, String> {
        if self.peek() == Some(b'"') {
            return self.basic_string();
        }
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                self.i += 1;
            } else {
                break;
            }
        }
        if self.i == start {
            return Err(self.err("expected a key"));
        }
        if self.peek() == Some(b'.') {
            return Err(self.err("dotted keys are not supported"));
        }
        Ok(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.peek() {
            Some(b'"') => {
                if self.s[self.i..].starts_with(b"\"\"\"") {
                    return Err(self.err("multi-line strings are not supported"));
                }
                Ok(Value::Str(self.basic_string()?))
            }
            Some(b'\'') => {
                if self.s[self.i..].starts_with(b"'''") {
                    return Err(self.err("multi-line strings are not supported"));
                }
                self.i += 1;
                let start = self.i;
                while let Some(c) = self.peek() {
                    if c == b'\'' {
                        let s = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                        self.i += 1;
                        return Ok(Value::Str(s));
                    }
                    if c == b'\n' {
                        break;
                    }
                    self.i += 1;
                }
                Err(self.err("unterminated string"))
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws_comments_newlines();
                    if self.peek() == Some(b']') {
                        self.i += 1;
                        return Ok(Value::Array(items));
                    }
                    items.push(self.value()?);
                    self.skip_ws_comments_newlines();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {}
                        _ => return Err(self.err("expected ',' or ']' in an array")),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut t = Table::new();
                self.skip_inline_ws();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                    return Ok(Value::Table(t));
                }
                loop {
                    self.skip_inline_ws();
                    let k = self.key()?;
                    self.skip_inline_ws();
                    if self.peek() != Some(b'=') {
                        return Err(self.err(&format!("expected '=' after '{}'", k)));
                    }
                    self.i += 1;
                    self.skip_inline_ws();
                    let v = self.value()?;
                    if t.contains_key(&k) {
                        return Err(self.err(&format!("duplicate key '{}'", k)));
                    }
                    t.insert(k, v);
                    self.skip_inline_ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Value::Table(t));
                        }
                        _ => return Err(self.err("expected ',' or '}' in an inline table")),
                    }
                }
            }
            Some(b't') if self.s[self.i..].starts_with(b"true") => {
                self.i += 4;
                Ok(Value::Bool(true))
            }
            Some(b'f') if self.s[self.i..].starts_with(b"false") => {
                self.i += 5;
                Ok(Value::Bool(false))
            }
            Some(c) if c.is_ascii_digit() || c == b'-' || c == b'+' => {
                let start = self.i;
                self.i += 1;
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() || c == b'_' {
                        self.i += 1;
                    } else {
                        break;
                    }
                }
                if matches!(self.peek(), Some(b'.') | Some(b'e') | Some(b'E') | Some(b':')) {
                    return Err(self.err("floats and dates are not supported"));
                }
                let txt: String = String::from_utf8_lossy(&self.s[start..self.i]).replace('_', "");
                txt.parse::<i64>()
                    .map(Value::Int)
                    .map_err(|_| self.err(&format!("bad integer '{}'", txt)))
            }
            _ => Err(self.err("expected a value")),
        }
    }

    fn basic_string(&mut self) -> Result<String, String> {
        self.i += 1; // opening quote
        let mut out: Vec<u8> = Vec::new();
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|_| self.err("invalid UTF-8 in a string")),
                b'\n' => break,
                b'\\' => {
                    let e = self.peek().ok_or_else(|| self.err("unterminated string"))?;
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'u' => {
                            let hex = self.s.get(self.i..self.i + 4).ok_or_else(|| self.err("bad \\u escape"))?;
                            let h = std::str::from_utf8(hex).map_err(|_| self.err("bad \\u escape"))?;
                            let cp = u32::from_str_radix(h, 16).map_err(|_| self.err("bad \\u escape"))?;
                            let ch = char::from_u32(cp).ok_or_else(|| self.err("bad \\u escape"))?;
                            let mut b = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                            self.i += 4;
                        }
                        _ => return Err(self.err(&format!("unknown escape '\\{}'", e as char))),
                    }
                }
                _ => out.push(c),
            }
        }
        Err(self.err("unterminated string"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_example() {
        let t = parse(
            r#"
# app.toml
id          = "dev.evan.notes"         # reverse-DNS
name        = "Notes"
permissions = ["files:documents",
               "notifications", ]
background  = { refresh = true, modes = ['audio'] }
hotkeys     = [{ slot = 1, action = "new_note", label = "New \"note\"" }]
[extra]
n = -12_000
"#,
        )
        .unwrap();
        assert_eq!(t["id"], Value::Str("dev.evan.notes".into()));
        assert_eq!(t["permissions"], Value::Array(vec![Value::Str("files:documents".into()), Value::Str("notifications".into())]));
        let Value::Table(bg) = &t["background"] else { panic!() };
        assert_eq!(bg["refresh"], Value::Bool(true));
        let Value::Array(hk) = &t["hotkeys"] else { panic!() };
        let Value::Table(h) = &hk[0] else { panic!() };
        assert_eq!(h["label"], Value::Str("New \"note\"".into()));
        let Value::Table(x) = &t["extra"] else { panic!() };
        assert_eq!(x["n"], Value::Int(-12000));
    }

    #[test]
    fn rejects_what_the_subset_leaves_out() {
        for bad in ["a.b = 1", "[[x]]", "x = 1.5", "x = \"\"\"a\"\"\"", "x = 1 y", "x = \"open", "x = 1\nx = 2"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
