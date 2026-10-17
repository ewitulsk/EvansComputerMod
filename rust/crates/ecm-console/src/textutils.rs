//! `textutils`: values as text and back, as ComputerCraft writes them.
//!
//! - [`serialize`]: a value as a Lua literal, in CC's layout (a table across
//!   lines, two spaces of indent, a comma after every entry; or compact).
//! - [`parse_lua`]: such a literal back into a value (what Python's
//!   `textutils.unserialize` uses; Lua runs the literal itself).
//! - [`to_json`] / [`parse_json`], [`format_time`], [`url_encode`].
//!
//! Values are peripheral values: a list is a table with keys 1..n, a map any
//! other table. Map keys are written in a fixed order (numbers, then strings,
//! sorted) so the output doesn't change between runs.

use ecm_host_abi::peripheral::Value;

const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// A number as Lua writes it: whole numbers without a fraction, the rest in
/// the shortest form that reads back the same; nan and infinities as
/// expressions that make them.
pub fn number(n: f64) -> String {
    if n.is_nan() {
        "0/0".into()
    } else if n.is_infinite() {
        if n > 0.0 { "1/0".into() } else { "-1/0".into() }
    } else if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// A string quoted as Lua 5.2's `%q` quotes it (a newline as "\" and a newline).
pub fn quote(s: &[u8]) -> String {
    let mut out = String::from("\"");
    for (i, &b) in s.iter().enumerate() {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\\n"),
            b'\r' => out.push_str("\\r"),
            0 => out.push_str(if s.get(i + 1).is_some_and(u8::is_ascii_digit) { "\\000" } else { "\\0" }),
            c if c < 32 || c == 127 => {
                if s.get(i + 1).is_some_and(u8::is_ascii_digit) {
                    out.push_str(&format!("\\{c:03}"));
                } else {
                    out.push_str(&format!("\\{c}"));
                }
            }
            c => out.push(c as char),
        }
    }
    out.push('"');
    out
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !KEYWORDS.contains(&s)
}

fn key_order(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let rank = |v: &Value| match v {
        Value::Int(_) | Value::Float(_) => 0,
        Value::Bool(_) => 1,
        _ => 2,
    };
    let num = |v: &Value| match v {
        Value::Int(i) => *i as f64,
        Value::Float(f) => *f,
        _ => 0.0,
    };
    match rank(a).cmp(&rank(b)) {
        Ordering::Equal => match (a, b) {
            (Value::Str(x), Value::Str(y)) => x.cmp(y),
            (Value::Bytes(x), Value::Bytes(y)) => x.cmp(y),
            _ => num(a).total_cmp(&num(b)),
        },
        o => o,
    }
}

/// A map's entries as CC writes a table: the values at 1, 2, 3... first
/// (without keys), then the other keys in order.
fn split_entries(entries: &[(Value, Value)]) -> (Vec<&Value>, Vec<(&Value, &Value)>) {
    let mut positional = Vec::new();
    let mut n = 1i64;
    loop {
        let found = entries.iter().find(|(k, _)| matches!(k, Value::Int(i) if *i == n) || matches!(k, Value::Float(f) if *f == n as f64));
        match found {
            Some((_, v)) => positional.push(v),
            None => break,
        }
        n += 1;
    }
    let count = positional.len() as i64;
    let mut rest: Vec<(&Value, &Value)> = entries
        .iter()
        .filter(|(k, _)| !match k {
            Value::Int(i) => *i >= 1 && *i <= count,
            Value::Float(f) => f.fract() == 0.0 && *f >= 1.0 && *f <= count as f64,
            _ => false,
        })
        .map(|(k, v)| (k, v))
        .collect();
    rest.sort_by(|a, b| key_order(a.0, b.0));
    (positional, rest)
}

/// `textutils.serialize`: `v` as a Lua literal.
pub fn serialize(v: &Value, compact: bool) -> String {
    let mut out = String::new();
    write_value(v, compact, 0, &mut out);
    out
}

fn write_value(v: &Value, compact: bool, depth: usize, out: &mut String) {
    match v {
        Value::Nil => out.push_str("nil"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => out.push_str(&number(*f)),
        Value::Str(s) => out.push_str(&quote(s.as_bytes())),
        Value::Bytes(b) => out.push_str(&quote(b)),
        Value::List(items) => {
            let positional: Vec<&Value> = items.iter().collect();
            write_table(&positional, &[], compact, depth, out);
        }
        Value::Map(entries) => {
            let (positional, rest) = split_entries(entries);
            write_table(&positional, &rest, compact, depth, out);
        }
    }
}

fn write_table(positional: &[&Value], rest: &[(&Value, &Value)], compact: bool, depth: usize, out: &mut String) {
    if positional.is_empty() && rest.is_empty() {
        out.push_str("{}");
        return;
    }
    let (nl, indent, outer, eq) = if compact {
        ("", String::new(), String::new(), "=")
    } else {
        ("\n", "  ".repeat(depth + 1), "  ".repeat(depth), " = ")
    };
    out.push('{');
    out.push_str(nl);
    for v in positional {
        out.push_str(&indent);
        write_value(v, compact, depth + 1, out);
        out.push(',');
        out.push_str(nl);
    }
    for (k, v) in rest {
        out.push_str(&indent);
        match k {
            Value::Str(s) if is_identifier(s) => out.push_str(s),
            other => {
                out.push_str(if compact { "[" } else { "[ " });
                write_value(other, compact, depth + 1, out);
                out.push_str(if compact { "]" } else { " ]" });
            }
        }
        out.push_str(eq);
        write_value(v, compact, depth + 1, out);
        out.push(',');
        out.push_str(nl);
    }
    out.push_str(&outer);
    out.push('}');
}

// ------------------------------------------------------------- Lua literals

struct Reader<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn ws(&mut self) {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"--") {
                self.i += 2;
                if let Some(level) = self.long_open() {
                    let _ = self.long_body(level);
                } else {
                    while self.i < self.s.len() && self.s[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                continue;
            }
            break;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at character {}", self.i + 1))
    }

    /// "[" "="* "[": the level of a long bracket, consumed.
    fn long_open(&mut self) -> Option<usize> {
        if self.peek() != Some(b'[') {
            return None;
        }
        let mut j = self.i + 1;
        while self.s.get(j) == Some(&b'=') {
            j += 1;
        }
        if self.s.get(j) == Some(&b'[') {
            let level = j - self.i - 1;
            self.i = j + 1;
            Some(level)
        } else {
            None
        }
    }

    fn long_body(&mut self, level: usize) -> Result<Vec<u8>, String> {
        let close: Vec<u8> = [b"]".to_vec(), vec![b'='; level], b"]".to_vec()].concat();
        if self.peek() == Some(b'\r') {
            self.i += 1;
        }
        if self.peek() == Some(b'\n') {
            self.i += 1;
        }
        let start = self.i;
        while self.i + close.len() <= self.s.len() {
            if self.s[self.i..].starts_with(&close) {
                let body = self.s[start..self.i].to_vec();
                self.i += close.len();
                return Ok(body);
            }
            self.i += 1;
        }
        self.err("unfinished long string")
    }

    fn string(&mut self, q: u8) -> Result<Vec<u8>, String> {
        self.i += 1;
        let mut out = Vec::new();
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                c if c == q => return Ok(out),
                b'\\' => {
                    let Some(e) = self.peek() else { break };
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'a' => out.push(7),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'v' => out.push(11),
                        b'\n' => out.push(b'\n'),
                        b'x' => {
                            let h = std::str::from_utf8(self.s.get(self.i..self.i + 2).unwrap_or(b"")).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                            let Some(h) = h else { return self.err("bad \\x escape") };
                            out.push(h);
                            self.i += 2;
                        }
                        b'z' => {
                            while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                                self.i += 1;
                            }
                        }
                        d if d.is_ascii_digit() => {
                            let mut n = (d - b'0') as u32;
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d) if d.is_ascii_digit() => {
                                        n = n * 10 + (d - b'0') as u32;
                                        self.i += 1;
                                    }
                                    _ => break,
                                }
                            }
                            if n > 255 {
                                return self.err("escape too large");
                            }
                            out.push(n as u8);
                        }
                        other => out.push(other),
                    }
                }
                b'\n' => return self.err("unfinished string"),
                c => out.push(c),
            }
        }
        self.err("unfinished string")
    }

    fn number(&mut self) -> Result<f64, String> {
        let start = self.i;
        if self.s[self.i..].starts_with(b"0x") || self.s[self.i..].starts_with(b"0X") {
            self.i += 2;
            while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                self.i += 1;
            }
            let h = std::str::from_utf8(&self.s[start + 2..self.i]).unwrap();
            return u64::from_str_radix(h, 16).map(|v| v as f64).or_else(|_| self.err("bad number"));
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'.') {
            self.i += 1;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.i += 1;
            }
        }
        std::str::from_utf8(&self.s[start..self.i]).unwrap().parse::<f64>().or_else(|_| self.err("bad number"))
    }

    fn value(&mut self) -> Result<Value, String> {
        self.ws();
        let Some(c) = self.peek() else { return self.err("value expected") };
        let v = match c {
            b'{' => {
                self.i += 1;
                return self.table();
            }
            b'"' | b'\'' => bytes_value(self.string(c)?),
            b'[' => match self.long_open() {
                Some(level) => bytes_value(self.long_body(level)?),
                None => return self.err("unexpected ["),
            },
            b'-' => {
                self.i += 1;
                return match self.value()? {
                    Value::Int(i) => Ok(Value::Int(-i)),
                    Value::Float(f) => Ok(Value::Float(-f)),
                    _ => self.err("number expected after -"),
                };
            }
            b'(' => {
                self.i += 1;
                let v = self.value()?;
                if !self.eat(b')') {
                    return self.err("')' expected");
                }
                v
            }
            c if c.is_ascii_digit() || c == b'.' => num_value(self.number()?),
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = self.i;
                while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_') {
                    self.i += 1;
                }
                match &self.s[start..self.i] {
                    b"nil" => Value::Nil,
                    b"true" => Value::Bool(true),
                    b"false" => Value::Bool(false),
                    _ => return self.err("only literal values can be read"),
                }
            }
            _ => return self.err("unexpected symbol"),
        };
        // A division, for 0/0, 1/0 and -1/0.
        self.ws();
        if self.peek() == Some(b'/') {
            self.i += 1;
            let d = self.value()?;
            let (a, b) = (as_f64(&v), as_f64(&d));
            return match (a, b) {
                (Some(a), Some(b)) => Ok(num_value(a / b)),
                _ => self.err("number expected"),
            };
        }
        Ok(v)
    }

    fn table(&mut self) -> Result<Value, String> {
        let mut entries: Vec<(Value, Value)> = Vec::new();
        let mut next = 1i64;
        loop {
            if self.eat(b'}') {
                break;
            }
            self.ws();
            let (k, v) = if self.peek() == Some(b'[') && self.long_open_peek().is_none() {
                self.i += 1;
                let k = self.value()?;
                if !self.eat(b']') || !self.eat(b'=') {
                    return self.err("']' '=' expected");
                }
                (k, self.value()?)
            } else if let Some(name) = self.name_then_equals() {
                (Value::Str(name), self.value()?)
            } else {
                let v = self.value()?;
                let k = Value::Int(next);
                next += 1;
                (k, v)
            };
            if !matches!(v, Value::Nil) {
                entries.retain(|(e, _)| !same_key(e, &k));
                entries.push((k, v));
            }
            if !self.eat(b',') && !self.eat(b';') {
                if self.eat(b'}') {
                    break;
                }
                return self.err("'}' expected");
            }
        }
        Ok(table_value(entries))
    }

    fn long_open_peek(&self) -> Option<usize> {
        let mut j = self.i + 1;
        while self.s.get(j) == Some(&b'=') {
            j += 1;
        }
        (self.s.get(j) == Some(&b'[')).then_some(j - self.i - 1)
    }

    /// `name =` (not `==`), consumed; None (nothing consumed) otherwise.
    fn name_then_equals(&mut self) -> Option<String> {
        let start = self.i;
        if !self.peek().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_') {
            return None;
        }
        let mut j = self.i;
        while self.s.get(j).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
            j += 1;
        }
        let name = std::str::from_utf8(&self.s[start..j]).unwrap().to_string();
        let mut k = j;
        while self.s.get(k).is_some_and(|c| c.is_ascii_whitespace()) {
            k += 1;
        }
        if self.s.get(k) == Some(&b'=') && self.s.get(k + 1) != Some(&b'=') && !KEYWORDS.contains(&name.as_str()) {
            self.i = k + 1;
            Some(name)
        } else {
            None
        }
    }
}

fn bytes_value(b: Vec<u8>) -> Value {
    match String::from_utf8(b) {
        Ok(s) => Value::Str(s),
        Err(e) => Value::Bytes(e.into_bytes()),
    }
}

fn num_value(f: f64) -> Value {
    if f.fract() == 0.0 && f.abs() < 9.0e15 {
        Value::Int(f as i64)
    } else {
        Value::Float(f)
    }
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

fn same_key(a: &Value, b: &Value) -> bool {
    match (as_f64(a), as_f64(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// Entries with keys exactly 1..n as a list; any other table as a map.
fn table_value(entries: Vec<(Value, Value)>) -> Value {
    let n = entries.len() as i64;
    let is_list = (1..=n).all(|i| entries.iter().any(|(k, _)| matches!(k, Value::Int(x) if *x == i)));
    if is_list {
        let mut items = vec![Value::Nil; entries.len()];
        for (k, v) in entries {
            if let Value::Int(i) = k {
                items[(i - 1) as usize] = v;
            }
        }
        Value::List(items)
    } else {
        Value::Map(entries)
    }
}

/// `textutils.unserialize`: a Lua literal back into a value.
pub fn parse_lua(text: &str) -> Result<Value, String> {
    let mut r = Reader { s: text.as_bytes(), i: 0 };
    let v = r.value()?;
    r.ws();
    if r.i != r.s.len() {
        return r.err("unexpected text after the value");
    }
    Ok(v)
}

// ------------------------------------------------------------- JSON

/// `textutils.serializeJSON`: lists as arrays, maps as objects (string and
/// number keys), nil as null. `unicode`: characters past ASCII as \u escapes.
pub fn to_json(v: &Value, unicode: bool) -> Result<String, String> {
    let mut out = String::new();
    json_value(v, unicode, &mut out)?;
    Ok(out)
}

fn json_string(s: &str, unicode: bool, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 32 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if unicode && (c as u32) > 126 => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", u));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn json_value(v: &Value, unicode: bool, out: &mut String) -> Result<(), String> {
    match v {
        Value::Nil => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) if f.is_finite() => out.push_str(&number(*f)),
        Value::Float(_) => return Err("Cannot serialize NaN or infinity as JSON".into()),
        Value::Str(s) => json_string(s, unicode, out),
        Value::Bytes(b) => json_string(&String::from_utf8_lossy(b), unicode, out),
        Value::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                json_value(item, unicode, out)?;
            }
            out.push(']');
        }
        Value::Map(entries) if !entries.is_empty() && split_entries(entries).1.is_empty() => {
            // Keys 1..n: an array.
            let (items, _) = split_entries(entries);
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                json_value(item, unicode, out)?;
            }
            out.push(']');
        }
        Value::Map(entries) => {
            let mut sorted: Vec<&(Value, Value)> = entries.iter().collect();
            sorted.sort_by(|a, b| key_order(&a.0, &b.0));
            out.push('{');
            for (i, (k, val)) in sorted.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let key = match k {
                    Value::Str(s) => s.clone(),
                    Value::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
                    Value::Int(n) => n.to_string(),
                    Value::Float(f) => number(*f),
                    _ => return Err("Cannot serialize a table with a key that is not a string or a number".into()),
                };
                json_string(&key, unicode, out);
                out.push(':');
                json_value(val, unicode, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// `textutils.unserializeJSON`: JSON into a value. Arrays become lists,
/// objects maps; null is nil (dropped from objects, kept in arrays as nil).
pub fn parse_json(text: &str) -> Result<Value, String> {
    let mut r = Reader { s: text.as_bytes(), i: 0 };
    let v = json_parse(&mut r)?;
    while r.peek().is_some_and(|c| c.is_ascii_whitespace()) {
        r.i += 1;
    }
    if r.i != r.s.len() {
        return r.err("unexpected text after the value");
    }
    Ok(v)
}

fn json_ws(r: &mut Reader) {
    while r.peek().is_some_and(|c| c.is_ascii_whitespace()) {
        r.i += 1;
    }
}

fn json_parse(r: &mut Reader) -> Result<Value, String> {
    json_ws(r);
    match r.peek() {
        Some(b'{') => {
            r.i += 1;
            let mut entries = Vec::new();
            json_ws(r);
            if r.peek() == Some(b'}') {
                r.i += 1;
                return Ok(Value::Map(entries));
            }
            loop {
                json_ws(r);
                if r.peek() != Some(b'"') {
                    return r.err("string expected");
                }
                let k = json_str(r)?;
                json_ws(r);
                if r.peek() != Some(b':') {
                    return r.err("':' expected");
                }
                r.i += 1;
                let v = json_parse(r)?;
                if !matches!(v, Value::Nil) {
                    entries.push((Value::Str(k), v));
                }
                json_ws(r);
                match r.peek() {
                    Some(b',') => r.i += 1,
                    Some(b'}') => {
                        r.i += 1;
                        return Ok(Value::Map(entries));
                    }
                    _ => return r.err("',' or '}' expected"),
                }
            }
        }
        Some(b'[') => {
            r.i += 1;
            let mut items = Vec::new();
            json_ws(r);
            if r.peek() == Some(b']') {
                r.i += 1;
                return Ok(Value::List(items));
            }
            loop {
                items.push(json_parse(r)?);
                json_ws(r);
                match r.peek() {
                    Some(b',') => r.i += 1,
                    Some(b']') => {
                        r.i += 1;
                        return Ok(Value::List(items));
                    }
                    _ => return r.err("',' or ']' expected"),
                }
            }
        }
        Some(b'"') => Ok(Value::Str(json_str(r)?)),
        Some(c) if c == b'-' || c.is_ascii_digit() => {
            let start = r.i;
            r.i += 1;
            while r.peek().is_some_and(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-')) {
                r.i += 1;
            }
            let t = std::str::from_utf8(&r.s[start..r.i]).unwrap();
            t.parse::<f64>().map(num_value).or_else(|_| r.err("bad number"))
        }
        _ => {
            for (word, v) in [("true", Value::Bool(true)), ("false", Value::Bool(false)), ("null", Value::Nil)] {
                if r.s[r.i..].starts_with(word.as_bytes()) {
                    r.i += word.len();
                    return Ok(v);
                }
            }
            r.err("unexpected character")
        }
    }
}

fn json_str(r: &mut Reader) -> Result<String, String> {
    r.i += 1;
    let mut out: Vec<u16> = Vec::new();
    let mut bytes: Vec<u8> = Vec::new();
    let flush = |bytes: &mut Vec<u8>, out: &mut Vec<u16>| {
        out.extend(String::from_utf8_lossy(bytes).encode_utf16());
        bytes.clear();
    };
    while let Some(c) = r.peek() {
        r.i += 1;
        match c {
            b'"' => {
                flush(&mut bytes, &mut out);
                return Ok(String::from_utf16_lossy(&out));
            }
            b'\\' => {
                flush(&mut bytes, &mut out);
                let Some(e) = r.peek() else { break };
                r.i += 1;
                match e {
                    b'n' => out.push(10),
                    b't' => out.push(9),
                    b'r' => out.push(13),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'u' => {
                        let h = std::str::from_utf8(r.s.get(r.i..r.i + 4).unwrap_or(b"")).ok().and_then(|h| u16::from_str_radix(h, 16).ok());
                        let Some(h) = h else { return r.err("bad \\u escape") };
                        out.push(h);
                        r.i += 4;
                    }
                    other => out.push(other as u16),
                }
            }
            c => bytes.push(c),
        }
    }
    r.err("unfinished string")
}

// ------------------------------------------------------------- the rest

/// `textutils.formatTime`: an hour of the day (os.time()) as "1:30 PM" or "13:30".
pub fn format_time(t: f64, twenty_four: bool) -> String {
    let t = t.rem_euclid(24.0);
    let hour = t.floor() as i64;
    let minute = ((t - hour as f64) * 60.0 + 1e-9).floor() as i64;
    if twenty_four {
        format!("{hour}:{minute:02}")
    } else {
        let h12 = match hour % 12 {
            0 => 12,
            h => h,
        };
        format!("{h12}:{minute:02} {}", if hour >= 12 { "PM" } else { "AM" })
    }
}

/// `textutils.urlEncode`: letters, digits and "-_." as they are, spaces as
/// "+", newlines as CRLF, everything else (each UTF-8 byte) as %XX.
pub fn url_encode(s: &[u8]) -> String {
    let mut out = String::new();
    for &b in s {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(b as char),
            b' ' => out.push('+'),
            b'\n' => out.push_str("%0D%0A"),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(e: Vec<(&str, Value)>) -> Value {
        Value::Map(e.into_iter().map(|(k, v)| (Value::Str(k.into()), v)).collect())
    }

    #[test]
    fn serialize_lays_tables_out_like_cc() {
        let v = Value::Map(vec![
            (Value::Int(1), Value::Str("a".into())),
            (Value::Int(2), Value::Float(2.5)),
            (Value::Str("name".into()), Value::Str("Ann \"A\"\n".into())),
            (Value::Str("x y".into()), Value::Bool(true)),
            (Value::Str("nested".into()), Value::List(vec![Value::Int(1)])),
            (Value::Str("empty".into()), Value::List(vec![])),
        ]);
        assert_eq!(
            serialize(&v, false),
            "{\n  \"a\",\n  2.5,\n  empty = {},\n  name = \"Ann \\\"A\\\"\\\n\",\n  nested = {\n    1,\n  },\n  [ \"x y\" ] = true,\n}"
        );
        assert_eq!(serialize(&v, true), "{\"a\",2.5,empty={},name=\"Ann \\\"A\\\"\\\n\",nested={1,},[\"x y\"]=true,}");
        assert_eq!(serialize(&Value::Float(f64::NAN), false), "0/0");
        assert_eq!(serialize(&map(vec![("end", Value::Int(1))]), true), "{[\"end\"]=1,}", "a keyword is not a name");
        assert_eq!(quote(b"a\x01b\x002"), "\"a\\1b\\0002\"");
    }

    #[test]
    fn lua_literals_read_back() {
        let v = map(vec![("a", Value::Int(1)), ("b", Value::List(vec![Value::Str("x".into()), Value::Float(-1.5)]))]);
        assert_eq!(parse_lua(&serialize(&v, false)), Ok(v.clone()));
        assert_eq!(parse_lua(&serialize(&v, true)), Ok(v));
        assert_eq!(parse_lua("{ 1, 2; [5] = 'q', s = [[long\nstr]], h = 0x10 }").unwrap(), Value::Map(vec![
            (Value::Int(1), Value::Int(1)), (Value::Int(2), Value::Int(2)), (Value::Int(5), Value::Str("q".into())),
            (Value::Str("s".into()), Value::Str("long\nstr".into())), (Value::Str("h".into()), Value::Int(16)),
        ]));
        assert_eq!(parse_lua("\"tab\\tnl\\\nz\\65\"").unwrap(), Value::Str("tab\tnl\nzA".into()));
        assert!(matches!(parse_lua("1/0"), Ok(Value::Float(f)) if f.is_infinite()));
        assert!(parse_lua("os.exit()").is_err(), "only literals");
        assert!(parse_lua("{1,").is_err());
    }

    #[test]
    fn json_both_ways() {
        let v = map(vec![("b", Value::List(vec![Value::Int(1), Value::Nil, Value::Bool(false)])), ("a", Value::Str("é\n".into()))]);
        assert_eq!(to_json(&v, false).unwrap(), "{\"a\":\"é\\n\",\"b\":[1,null,false]}");
        assert_eq!(to_json(&v, true).unwrap(), "{\"a\":\"\\u00e9\\n\",\"b\":[1,null,false]}");
        assert_eq!(parse_json(" {\"a\": [1, 2.5, \"\\u00e9\"], \"n\": null} ").unwrap(),
            map(vec![("a", Value::List(vec![Value::Int(1), Value::Float(2.5), Value::Str("é".into())]))]));
        assert!(parse_json("{\"a\" 1}").is_err() && to_json(&Value::Float(f64::INFINITY), false).is_err());
    }

    #[test]
    fn times_and_urls() {
        assert_eq!(format_time(13.5, false), "1:30 PM");
        assert_eq!(format_time(0.25, false), "12:15 AM");
        assert_eq!(format_time(6.05, true), "6:03");
        assert_eq!(url_encode(b"a b&c=d/\xc3\xa9"), "a+b%26c%3Dd%2F%C3%A9");
    }
}
