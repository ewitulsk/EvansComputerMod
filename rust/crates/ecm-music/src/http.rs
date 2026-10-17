//! The HTTP pieces `musicd` needs, without any I/O: the daemon reads bytes
//! from its sockets, hands the head to [`parse_request`], and writes back the
//! response heads built here followed by the body (a file region for a
//! `Range` stream, or JSON).
//!
//! Also a small JSON value type with a parser for request bodies and an
//! encoder with correct string escaping, and query-string decoding.

use std::fmt::Write as _;

/// Largest request head accepted.
pub const MAX_HEAD: usize = 16 * 1024;

/// Why a request couldn't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    /// Malformed request line or header.
    Bad(&'static str),
    /// The head is larger than [`MAX_HEAD`].
    TooLarge,
}

/// A parsed request head.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub method: String,
    /// The target as sent (`/search?q=daft+punk`).
    pub target: String,
    /// The path part, percent-decoded (`/search`).
    pub path: String,
    /// Decoded query parameters in order.
    pub query: Vec<(String, String)>,
    pub version: String,
    pub headers: Vec<(String, String)>,
}

impl Request {
    /// A header value (name compared without case).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The first value of a query parameter.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// A numeric query parameter, or `default` when missing or unreadable.
    pub fn param_num(&self, name: &str, default: usize) -> usize {
        self.param(name).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    pub fn content_length(&self) -> Option<usize> {
        self.header("content-length").and_then(|v| v.trim().parse().ok())
    }

    /// The token of `Authorization: Bearer <token>`.
    pub fn bearer(&self) -> Option<&str> {
        let v = self.header("authorization")?.trim();
        let (scheme, tok) = v.split_once(' ')?;
        scheme.eq_ignore_ascii_case("bearer").then(|| tok.trim()).filter(|t| !t.is_empty())
    }

    /// Decoded path segments (`/songs/abc` gives `["songs", "abc"]`; an
    /// escaped `%2F` stays inside its segment).
    pub fn segments(&self) -> Vec<String> {
        let raw = self.target.split('?').next().unwrap_or("");
        raw.split('/').filter(|s| !s.is_empty()).map(|s| percent_decode(s, false)).collect()
    }

    /// Whether the client wants the connection kept open.
    pub fn keep_alive(&self) -> bool {
        match self.header("connection") {
            Some(v) if v.eq_ignore_ascii_case("close") => false,
            Some(v) if v.eq_ignore_ascii_case("keep-alive") => true,
            _ => self.version == "HTTP/1.1",
        }
    }
}

/// Split a request line into method, target and version.
pub fn parse_request_line(line: &str) -> Option<(String, String, String)> {
    let mut it = line.split(' ').filter(|s| !s.is_empty());
    let method = it.next()?;
    let target = it.next()?;
    let version = it.next().unwrap_or("HTTP/1.0");
    if it.next().is_some() || !method.chars().all(|c| c.is_ascii_uppercase()) || !version.starts_with("HTTP/") {
        return None;
    }
    Some((method.to_string(), target.to_string(), version.to_string()))
}

/// Parse a request head from the start of `buf`. `Ok(None)` while the blank
/// line ending the head hasn't arrived; otherwise the request and the number
/// of bytes the head took (the body, if any, follows).
pub fn parse_request(buf: &[u8]) -> Result<Option<(Request, usize)>, HttpError> {
    let end = match buf.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(p) => p,
        None => {
            // Also accept bare LF line ends.
            match buf.windows(2).position(|w| w == b"\n\n") {
                Some(p) => p,
                None if buf.len() > MAX_HEAD => return Err(HttpError::TooLarge),
                None => return Ok(None),
            }
        }
    };
    let crlf = buf.get(end..end + 4) == Some(b"\r\n\r\n");
    let used = end + if crlf { 4 } else { 2 };
    if end > MAX_HEAD {
        return Err(HttpError::TooLarge);
    }
    let head = std::str::from_utf8(&buf[..end]).map_err(|_| HttpError::Bad("head is not UTF-8"))?;
    let mut lines = head.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
    let (method, target, version) = parse_request_line(lines.next().unwrap_or("")).ok_or(HttpError::Bad("bad request line"))?;
    let mut headers = Vec::new();
    for l in lines {
        if l.is_empty() {
            continue;
        }
        let (k, v) = l.split_once(':').ok_or(HttpError::Bad("bad header line"))?;
        if k.is_empty() || k.contains(' ') {
            return Err(HttpError::Bad("bad header name"));
        }
        headers.push((k.to_string(), v.trim().to_string()));
    }
    let (path, qs) = target.split_once('?').unwrap_or((&target, ""));
    let req = Request { path: percent_decode(path, false), query: parse_query(qs), method, target: target.clone(), version, headers };
    Ok(Some((req, used)))
}

/// Decode `%XX` escapes (and `+` as a space when `plus` is set). Invalid
/// escapes are kept as they are; invalid UTF-8 becomes U+FFFD.
pub fn percent_decode(s: &str, plus: bool) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match (hex(b[i + 1]), hex(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h << 4 | l);
                    i += 3;
                    continue;
                }
                _ => out.push(b'%'),
            },
            b'+' if plus => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Encode everything but unreserved characters (for building URLs).
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Decode a query string (`q=daft+punk&limit=20`).
pub fn parse_query(qs: &str) -> Vec<(String, String)> {
    qs.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k, true), percent_decode(v, true))
        })
        .collect()
}

/// A `Range: bytes=...` request (single range only).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Range {
    /// `bytes=a-b` (inclusive).
    FromTo(u64, u64),
    /// `bytes=a-`
    From(u64),
    /// `bytes=-n`: the last n bytes.
    Suffix(u64),
}

/// Parse a `Range` header value. None for anything else (other units,
/// several ranges, junk): the whole file is then sent, as RFC 9110 allows.
pub fn parse_range(v: &str) -> Option<Range> {
    let spec = v.trim().strip_prefix("bytes=")?.trim();
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    let num = |s: &str| if s.chars().all(|c| c.is_ascii_digit()) { s.parse::<u64>().ok() } else { None };
    match (a.is_empty(), b.is_empty()) {
        (true, false) => Some(Range::Suffix(num(b)?)),
        (false, true) => Some(Range::From(num(a)?)),
        (false, false) => {
            let (a, b) = (num(a)?, num(b)?);
            (b >= a).then_some(Range::FromTo(a, b))
        }
        _ => None,
    }
}

/// What to send for a resource of `len` bytes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Span {
    /// 200 with everything.
    Full,
    /// 206 with bytes `start..=end`.
    Partial { start: u64, end: u64 },
    /// 416.
    Unsatisfiable,
}

impl Span {
    /// The byte range to send (start, length).
    pub fn region(&self, len: u64) -> (u64, u64) {
        match *self {
            Span::Full => (0, len),
            Span::Partial { start, end } => (start, end - start + 1),
            Span::Unsatisfiable => (0, 0),
        }
    }
}

/// Resolve a `Range` header against a resource of `len` bytes.
pub fn resolve_range(header: Option<&str>, len: u64) -> Span {
    let Some(r) = header.and_then(parse_range) else { return Span::Full };
    let (start, end) = match r {
        Range::FromTo(a, b) => (a, b.min(len.saturating_sub(1))),
        Range::From(a) => (a, len.saturating_sub(1)),
        Range::Suffix(0) => return Span::Unsatisfiable,
        Range::Suffix(n) => (len.saturating_sub(n), len.saturating_sub(1)),
    };
    if len == 0 || start >= len {
        return Span::Unsatisfiable;
    }
    Span::Partial { start, end }
}

/// The reason phrase for a status code.
pub fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        206 => "Partial Content",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "",
    }
}

/// A response head with `Content-Type`, `Content-Length` and any `extra`
/// headers, ending in the blank line.
pub fn head(status: u16, content_type: &str, content_length: u64, extra: &[(&str, &str)]) -> Vec<u8> {
    let mut s = format!("HTTP/1.1 {status} {}\r\n", reason(status));
    if !content_type.is_empty() {
        let _ = write!(s, "Content-Type: {content_type}\r\n");
    }
    let _ = write!(s, "Content-Length: {content_length}\r\n");
    for (k, v) in extra {
        let _ = write!(s, "{k}: {v}\r\n");
    }
    s.push_str("\r\n");
    s.into_bytes()
}

/// The answer to a (possibly ranged) GET of a resource of `total` bytes:
/// the head to send, then `len` bytes of the resource from `start`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeAnswer {
    pub status: u16,
    pub start: u64,
    pub len: u64,
    pub head: Vec<u8>,
}

/// Build the 200 / 206 / 416 head for a GET with an optional `Range` header.
pub fn range_answer(range: Option<&str>, total: u64, content_type: &str, extra: &[(&str, &str)]) -> RangeAnswer {
    let span = resolve_range(range, total);
    let (start, len) = span.region(total);
    let mut ex: Vec<(&str, &str)> = vec![("Accept-Ranges", "bytes")];
    let cr;
    let status = match span {
        Span::Full => 200,
        Span::Partial { start, end } => {
            cr = format!("bytes {start}-{end}/{total}");
            ex.push(("Content-Range", &cr));
            206
        }
        Span::Unsatisfiable => {
            cr = format!("bytes */{total}");
            ex.push(("Content-Range", &cr));
            416
        }
    };
    ex.extend_from_slice(extra);
    let ct = if status == 416 { "" } else { content_type };
    RangeAnswer { status, start, len, head: head(status, ct, len, &ex) }
}

/// A whole response: head and body.
pub fn response(status: u16, content_type: &str, body: &[u8], extra: &[(&str, &str)]) -> Vec<u8> {
    let mut out = head(status, content_type, body.len() as u64, extra);
    out.extend_from_slice(body);
    out
}

/// A JSON response.
pub fn json_response(status: u16, json: &Json) -> Vec<u8> {
    response(status, "application/json", json.to_string().as_bytes(), &[])
}

/// A JSON error: `{"error": message}`.
pub fn error_response(status: u16, message: &str) -> Vec<u8> {
    json_response(status, &Json::obj([("error", Json::from(message))]))
}

// ---------------------------------------------------------------------------
// JSON

/// A JSON value. Objects keep their key order.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// Append `s` as a quoted, escaped JSON string.
pub fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            // U+2028/2029 break JavaScript string literals.
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `s` as a quoted, escaped JSON string.
pub fn json_string(s: &str) -> String {
    let mut out = String::new();
    write_json_string(&mut out, s);
    out
}

impl Json {
    /// An object from key/value pairs.
    pub fn obj<K: Into<String>, I: IntoIterator<Item = (K, Json)>>(pairs: I) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    /// Add a field to an object (no-op on other values).
    pub fn with(mut self, key: &str, v: impl Into<Json>) -> Json {
        if let Json::Obj(o) = &mut self {
            o.push((key.to_string(), v.into()));
        }
        self
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(o) => o.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    /// A non-negative integer.
    pub fn as_u64(&self) -> Option<u64> {
        self.as_f64().filter(|n| *n >= 0.0 && n.fract() == 0.0 && *n <= 9.007_199_254_740_992e15).map(|n| n as u64)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    /// Append the compact encoding to `out`.
    pub fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => {
                if !n.is_finite() {
                    out.push_str("null");
                } else if n.fract() == 0.0 && n.abs() < 1e15 {
                    let _ = write!(out, "{}", *n as i64);
                } else {
                    let _ = write!(out, "{n}");
                }
            }
            Json::Str(s) => write_json_string(out, s),
            Json::Arr(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Json::Obj(o) => {
                out.push('{');
                for (i, (k, v)) in o.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_json_string(out, k);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }

    /// Parse a JSON document (nesting is limited to 64 levels).
    pub fn parse(s: &str) -> Result<Json, String> {
        let mut p = Parser { b: s.as_bytes(), i: 0, depth: 0 };
        let v = p.value()?;
        p.ws();
        if p.i != p.b.len() {
            return Err(format!("unexpected text at {}", p.i));
        }
        Ok(v)
    }
}

impl std::fmt::Display for Json {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::new();
        self.write(&mut s);
        f.write_str(&s)
    }
}

impl From<&str> for Json {
    fn from(s: &str) -> Json {
        Json::Str(s.to_string())
    }
}
impl From<String> for Json {
    fn from(s: String) -> Json {
        Json::Str(s)
    }
}
impl From<&String> for Json {
    fn from(s: &String) -> Json {
        Json::Str(s.clone())
    }
}
impl From<bool> for Json {
    fn from(b: bool) -> Json {
        Json::Bool(b)
    }
}
impl From<f64> for Json {
    fn from(n: f64) -> Json {
        Json::Num(n)
    }
}
impl From<u64> for Json {
    fn from(n: u64) -> Json {
        Json::Num(n as f64)
    }
}
impl From<u32> for Json {
    fn from(n: u32) -> Json {
        Json::Num(n as f64)
    }
}
impl From<i64> for Json {
    fn from(n: i64) -> Json {
        Json::Num(n as f64)
    }
}
impl From<usize> for Json {
    fn from(n: usize) -> Json {
        Json::Num(n as f64)
    }
}
impl From<Vec<Json>> for Json {
    fn from(v: Vec<Json>) -> Json {
        Json::Arr(v)
    }
}
impl<T: Into<Json>> From<Option<T>> for Json {
    fn from(v: Option<T>) -> Json {
        v.map(Into::into).unwrap_or(Json::Null)
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    depth: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at {}", self.i))
    }

    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            self.err("bad literal")
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        let Some(&c) = self.b.get(self.i) else { return self.err("unexpected end") };
        match c {
            b'n' => self.lit("null", Json::Null),
            b't' => self.lit("true", Json::Bool(true)),
            b'f' => self.lit("false", Json::Bool(false)),
            b'"' => self.string().map(Json::Str),
            b'[' | b'{' => {
                self.depth += 1;
                if self.depth > 64 {
                    return self.err("nested too deeply");
                }
                let v = if c == b'[' { self.array() } else { self.object() };
                self.depth -= 1;
                v
            }
            b'-' | b'0'..=b'9' => self.number(),
            _ => self.err("unexpected character"),
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.i += 1;
        let mut v = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b']') {
            self.i += 1;
            return Ok(Json::Arr(v));
        }
        loop {
            v.push(self.value()?);
            self.ws();
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Json::Arr(v));
                }
                _ => return self.err("expected , or ]"),
            }
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.i += 1;
        let mut o = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Ok(Json::Obj(o));
        }
        loop {
            self.ws();
            if self.b.get(self.i) != Some(&b'"') {
                return self.err("expected a key");
            }
            let k = self.string()?;
            self.ws();
            if self.b.get(self.i) != Some(&b':') {
                return self.err("expected :");
            }
            self.i += 1;
            let v = self.value()?;
            o.push((k, v));
            self.ws();
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Json::Obj(o));
                }
                _ => return self.err("expected , or }"),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self.b.get(self.i..self.i + 4).ok_or("short \\u escape")?;
        let s = std::str::from_utf8(h).map_err(|_| "bad \\u escape")?;
        let n = u32::from_str_radix(s, 16).map_err(|_| format!("bad \\u escape at {}", self.i))?;
        self.i += 4;
        Ok(n)
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < self.b.len() && self.b[self.i] != b'"' && self.b[self.i] != b'\\' && self.b[self.i] >= 0x20 {
                self.i += 1;
            }
            // The input is a &str, so this run is valid UTF-8.
            out.push_str(std::str::from_utf8(&self.b[start..self.i]).map_err(|_| "bad UTF-8")?);
            match self.b.get(self.i) {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let Some(&e) = self.b.get(self.i) else { return self.err("unterminated string") };
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
                            let mut n = self.hex4()?;
                            if (0xd800..0xdc00).contains(&n) && self.b.get(self.i..self.i + 2) == Some(b"\\u") {
                                let save = self.i;
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xdc00..0xe000).contains(&lo) {
                                    n = 0x10000 + ((n - 0xd800) << 10) + (lo - 0xdc00);
                                } else {
                                    self.i = save;
                                }
                            }
                            out.push(char::from_u32(n).unwrap_or('\u{fffd}'));
                        }
                        _ => return self.err("bad escape"),
                    }
                }
                Some(_) => return self.err("control character in string"),
                None => return self.err("unterminated string"),
            }
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        let digits = |p: &mut Self| {
            let s = p.i;
            while p.i < p.b.len() && p.b[p.i].is_ascii_digit() {
                p.i += 1;
            }
            p.i - s
        };
        let int_start = self.i;
        let n = digits(self);
        if n == 0 || (n > 1 && self.b[int_start] == b'0') {
            return self.err("bad number");
        }
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            if digits(self) == 0 {
                return self.err("bad number");
            }
        }
        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if digits(self) == 0 {
                return self.err("bad number");
            }
        }
        let s = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| "bad number")?;
        s.parse::<f64>().map(Json::Num).map_err(|_| format!("bad number at {start}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_head_parsing() {
        let raw = b"GET /search?q=daft+pu%C3%A9&limit=5&empty HTTP/1.1\r\nHost: x\r\nRange: bytes=0-99\r\nAuthorization: Bearer ecmtok1.abc\r\n\r\nBODY";
        let (r, used) = parse_request(raw).unwrap().unwrap();
        assert_eq!(&raw[used..], b"BODY");
        assert_eq!((r.method.as_str(), r.path.as_str(), r.version.as_str()), ("GET", "/search", "HTTP/1.1"));
        assert_eq!(r.param("q"), Some("daft pué"));
        assert_eq!(r.param_num("limit", 20), 5);
        assert_eq!(r.param_num("offset", 0), 0);
        assert_eq!(r.param("empty"), Some(""));
        assert_eq!(r.header("range"), Some("bytes=0-99"));
        assert_eq!(r.bearer(), Some("ecmtok1.abc"));
        assert!(r.keep_alive());
        assert_eq!(parse_request(b"GET / HTTP/1.1\r\nHost: x\r\n").unwrap(), None, "incomplete");
        assert!(parse_request(b"get / HTTP/1.1\r\n\r\n").is_err());
        assert!(parse_request(b"GET / HTTP/1.1\r\nNoColon\r\n\r\n").is_err());
        assert_eq!(parse_request(&vec![b'a'; MAX_HEAD + 1]), Err(HttpError::TooLarge));
        let (r, _) = parse_request(b"DELETE /playlists/p%2F1 HTTP/1.0\n\n").unwrap().unwrap();
        assert_eq!(r.segments(), vec!["playlists", "p/1"]);
        assert!(!r.keep_alive());
    }

    #[test]
    fn query_strings() {
        assert_eq!(percent_decode("a%20b+c", true), "a b c");
        assert_eq!(percent_decode("a+b", false), "a+b");
        assert_eq!(percent_decode("100%", true), "100%");
        assert_eq!(percent_decode("%zz%4", true), "%zz%4");
        assert_eq!(percent_decode("%FF", true), "\u{fffd}");
        assert_eq!(parse_query("a=1&&b=x%3Dy&c"), vec![("a".into(), "1".into()), ("b".into(), "x=y".into()), ("c".into(), "".into())]);
        assert_eq!(percent_encode("a b/é"), "a%20b%2F%C3%A9");
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-499"), Some(Range::FromTo(0, 499)));
        assert_eq!(parse_range("bytes=500-"), Some(Range::From(500)));
        assert_eq!(parse_range("bytes=-500"), Some(Range::Suffix(500)));
        assert_eq!(parse_range("bytes=5-1"), None);
        assert_eq!(parse_range("bytes=0-1,4-5"), None);
        assert_eq!(parse_range("items=0-1"), None);
        assert_eq!(parse_range("bytes=+1-2"), None);
        assert_eq!(resolve_range(None, 1000), Span::Full);
        assert_eq!(resolve_range(Some("bytes=0-1999"), 1000), Span::Partial { start: 0, end: 999 });
        assert_eq!(resolve_range(Some("bytes=-100"), 1000), Span::Partial { start: 900, end: 999 });
        assert_eq!(resolve_range(Some("bytes=-5000"), 1000), Span::Partial { start: 0, end: 999 });
        assert_eq!(resolve_range(Some("bytes=1000-"), 1000), Span::Unsatisfiable);
        assert_eq!(resolve_range(Some("bytes=-0"), 1000), Span::Unsatisfiable);
        assert_eq!(resolve_range(Some("garbage"), 1000), Span::Full);
        let a = range_answer(Some("bytes=100-199"), 1000, "audio/mpeg", &[("Cache-Control", "no-store")]);
        let h = String::from_utf8(a.head.clone()).unwrap();
        assert_eq!((a.status, a.start, a.len), (206, 100, 100));
        assert!(h.starts_with("HTTP/1.1 206 Partial Content\r\n"));
        assert!(h.contains("Content-Range: bytes 100-199/1000\r\n") && h.contains("Content-Length: 100\r\n"));
        assert!(h.contains("Accept-Ranges: bytes\r\n") && h.contains("Content-Type: audio/mpeg\r\n") && h.contains("Cache-Control: no-store\r\n"));
        assert!(h.ends_with("\r\n\r\n"));
        let full = range_answer(None, 1000, "audio/mpeg", &[]);
        assert_eq!((full.status, full.start, full.len), (200, 0, 1000));
        let bad = range_answer(Some("bytes=2000-"), 1000, "audio/mpeg", &[]);
        let h = String::from_utf8(bad.head).unwrap();
        assert_eq!(bad.status, 416);
        assert!(h.contains("Content-Range: bytes */1000\r\n") && h.contains("Content-Length: 0\r\n"));
    }

    #[test]
    fn json_round_trip_and_escaping() {
        let v = Json::obj([("name", Json::from("a \"quote\" \\ \n\u{1}\u{2028} é 😀")), ("n", Json::from(3u64)), ("f", Json::from(1.5)), ("none", Json::from(None::<u64>))])
            .with("list", vec![Json::from(true), Json::Null]);
        let s = v.to_string();
        assert_eq!(s, r#"{"name":"a \"quote\" \\ \n\u0001\u2028 é 😀","n":3,"f":1.5,"none":null,"list":[true,null]}"#);
        assert_eq!(Json::parse(&s).unwrap(), v);
        let p = Json::parse(r#" { "a" : [1, -2.5e3, "x\u00e9\ud83d\ude00\/"], "b": {"c": false}, "d": 0 } "#).unwrap();
        assert_eq!(p.get("a").unwrap().as_array().unwrap()[1].as_f64(), Some(-2500.0));
        assert_eq!(p.get("a").unwrap().as_array().unwrap()[2].as_str(), Some("xé😀/"));
        assert_eq!(p.get("b").unwrap().get("c").unwrap().as_bool(), Some(false));
        assert_eq!(p.get("d").unwrap().as_u64(), Some(0));
        for bad in ["", "{", "[1,]", "{\"a\" 1}", "01", "1.", "\"\u{1}\"", "nul", "[1] x", "\"\\x\"", "-", "{1:2}"] {
            assert!(Json::parse(bad).is_err(), "{bad:?}");
        }
        assert!(Json::parse(&"[".repeat(100)).is_err(), "depth limit");
        assert_eq!(json_string("\t"), "\"\\t\"");
        let body = String::from_utf8(error_response(404, "no such song")).unwrap();
        assert!(body.starts_with("HTTP/1.1 404 Not Found\r\n") && body.ends_with("{\"error\":\"no such song\"}"));
    }
}
