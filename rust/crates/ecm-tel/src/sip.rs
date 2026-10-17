//! SIP messages (RFC 3261, trimmed), over UDP and TCP.
//!
//! The wire format is the real one, so tcpdump/Wireshark and anyone reading a
//! capture recognise it:
//!
//! ```text
//! INVITE sip:+12145550142@ims.ecm;user=phone SIP/2.0\r\n     <- start line
//! Via: SIP/2.0/UDP 10.45.0.7:5060;branch=z9hG4bK...;rport\r\n
//! From: "Alice" <sip:+12145550100@ims.ecm>;tag=4fa3\r\n
//! To: <tel:+12145550142>\r\n
//! Call-ID: 0123456789abcdef@10.45.0.7\r\n
//! CSeq: 1 INVITE\r\n
//! Content-Type: application/sdp\r\n
//! Content-Length: 142\r\n
//! \r\n
//! <body bytes>
//! ```
//!
//! **Parsing** ([`parse`]) takes one UDP datagram. It tolerates what real
//! stacks send: CR/LF before the start line, bare LF line ends, odd spacing
//! around `:`, obsolete header folding (a line starting with SP/HT continues
//! the previous header, joined with one space), and the compact header forms
//! (`v` Via, `f` From, `t` To, `i` Call-ID, `m` Contact, `l` Content-Length,
//! `c` Content-Type, `k` Supported, `s` Subject, `e` Content-Encoding, plus
//! `o` Event, `r` Refer-To, `u` Allow-Events, `x` Session-Expires, `b`
//! Referred-By), which are expanded to the full name. Everything else is
//! strict: a bad start line, header name, URI or Content-Length is an error,
//! never a panic.
//!
//! **Content-Length** is framing, not content: the parser removes it from the
//! header list and uses it to cut the body, and serialization always writes a
//! fresh one (the body's real length) as the last header. On UDP a datagram
//! longer than Content-Length is cut (RFC 3261 18.3); a shorter one is
//! [`SipError::Truncated`]. Without Content-Length a datagram's body is the
//! rest of the datagram. On TCP, [`StreamFramer`] splits the byte stream by
//! Content-Length (missing = 0) and skips CRLF keep-alives between messages.
//!
//! Bodies are raw bytes (`Vec<u8>`): SDP is text, but a MESSAGE carrying an SMS
//! (`application/vnd.3gpp.sms`) is binary.
//!
//! **Headers** keep their order; names compare case-insensitively. Typed
//! helpers on [`Headers`] parse the common ones ([`Via`], [`NameAddr`] for
//! From/To/Contact/Route/P-Asserted-Identity..., [`CSeq`], [`Digest`] for
//! Authorization/WWW-Authenticate, [`ParamHeader`] for Event/Reason/...).
//!
//! **Randomness** never comes from here: [`new_branch`], [`new_tag`] and
//! [`new_call_id`] take a caller-supplied `u64`.
//!
//! **Authentication** is the ECM flavour of IMS AKA ([`AKA_ALGORITHM`]): the
//! network's 401 carries `nonce = base64(RAND)` (16 bytes, no AUTN); the SIM
//! (elsewhere) computes `RES`; the phone answers with
//! `response = hex(HMAC-SHA256(RES, "username:realm:nonce:method:uri"))`
//! ([`digest_response`]) and the S-CSCF checks it with [`verify_digest_response`].

use std::fmt;

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// Largest header block accepted (start line + headers).
pub const MAX_HEAD: usize = 16 * 1024;
/// Largest body accepted.
pub const MAX_BODY: usize = 256 * 1024;
/// The RFC 3261 magic cookie every branch starts with.
pub const BRANCH_COOKIE: &str = "z9hG4bK";
/// The digest algorithm name of ECM's IMS AKA.
pub const AKA_ALGORITHM: &str = "ECM-AKAv1-HMAC-SHA256";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SipError {
    /// Nothing but line ends.
    Empty,
    /// The header block is not UTF-8.
    Utf8,
    /// The start line is malformed.
    StartLine,
    /// Not `SIP/2.0`.
    Version,
    /// A method that isn't a token.
    Method,
    /// A status code outside 100..=699 or not three digits.
    Status,
    /// A malformed URI (which part).
    Uri(&'static str),
    /// A malformed header line (why).
    Header(&'static str),
    /// A required header is missing.
    Missing(&'static str),
    /// A header's value is malformed (which header).
    Value(&'static str),
    /// Content-Length isn't a number, or two disagree.
    ContentLength,
    /// The body is shorter than Content-Length, or the header block never ends.
    Truncated,
    /// Over [`MAX_HEAD`] or [`MAX_BODY`].
    TooLarge,
    /// A response was given where a request is needed.
    NotRequest,
}

impl fmt::Display for SipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SipError::Empty => write!(f, "empty message"),
            SipError::Utf8 => write!(f, "header block is not UTF-8"),
            SipError::StartLine => write!(f, "malformed start line"),
            SipError::Version => write!(f, "not SIP/2.0"),
            SipError::Method => write!(f, "bad method"),
            SipError::Status => write!(f, "bad status code"),
            SipError::Uri(w) => write!(f, "bad URI ({w})"),
            SipError::Header(w) => write!(f, "bad header line ({w})"),
            SipError::Missing(h) => write!(f, "missing {h}"),
            SipError::Value(h) => write!(f, "bad {h} value"),
            SipError::ContentLength => write!(f, "bad Content-Length"),
            SipError::Truncated => write!(f, "truncated message"),
            SipError::TooLarge => write!(f, "message too large"),
            SipError::NotRequest => write!(f, "not a request"),
        }
    }
}

impl std::error::Error for SipError {}

type Res<T> = Result<T, SipError>;

// ---------------------------------------------------------------- lexing

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-.!%*_+`'~".contains(c)
}

fn is_token(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_token_char)
}

/// Parses a quoted string at the start of `s` (which starts with `"`):
/// returns the unescaped content and what follows the closing quote.
fn parse_quoted(s: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut it = s.char_indices();
    if it.next()?.1 != '"' {
        return None;
    }
    while let Some((i, c)) = it.next() {
        match c {
            '\\' => out.push(it.next()?.1),
            '"' => return Some((out, &s[i + 1..])),
            _ => out.push(c),
        }
    }
    None
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Removes surrounding quotes (and escapes) if present.
pub fn unquote(s: &str) -> String {
    if s.starts_with('"') {
        if let Some((v, rest)) = parse_quoted(s) {
            if rest.is_empty() {
                return v;
            }
        }
    }
    s.to_string()
}

/// Splits on `sep` outside quoted strings.
fn split_outside_quotes(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut in_q, mut esc) = (0, false, false);
    for (i, c) in s.char_indices() {
        if esc {
            esc = false;
        } else if in_q && c == '\\' {
            esc = true;
        } else if c == '"' {
            in_q = !in_q;
        } else if c == sep && !in_q {
            out.push(&s[start..i]);
            start = i + c.len_utf8();
        }
    }
    out.push(&s[start..]);
    out
}

/// Splits a header list value on commas outside quotes and `<...>`; empty
/// elements are dropped.
pub fn split_list(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut in_q, mut esc, mut angle) = (0, false, false, false);
    for (i, c) in s.char_indices() {
        if esc {
            esc = false;
        } else if in_q {
            match c {
                '\\' => esc = true,
                '"' => in_q = false,
                _ => {}
            }
        } else {
            match c {
                '"' => in_q = true,
                '<' => angle = true,
                '>' => angle = false,
                ',' if !angle => {
                    out.push(s[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            }
        }
    }
    out.push(s[start..].trim());
    out.retain(|p| !p.is_empty());
    out
}

fn split_first(s: &str, c: char) -> (&str, &str) {
    match s.find(c) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

fn hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 15) as usize] as char);
    }
    s
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let d = |c: u8| (c as char).to_digit(16).map(|v| v as u8);
    s.as_bytes().chunks(2).map(|p| Some(d(p[0])? << 4 | d(p[1])?)).collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(data: &[u8]) -> String {
    let mut s = String::new();
    for ch in data.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= ch.len() {
                s.push(B64[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

fn unbase64(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::new();
    for (ci, ch) in b.chunks(4).enumerate() {
        let last = ci == b.len() / 4 - 1;
        let mut n = 0u32;
        let mut pad = 0;
        for (k, &c) in ch.iter().enumerate() {
            let v = if c == b'=' {
                if !last || k < 2 {
                    return None;
                }
                pad += 1;
                0
            } else {
                if pad > 0 {
                    return None;
                }
                B64.iter().position(|&x| x == c)? as u32
            };
            n = n << 6 | v;
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

// ---------------------------------------------------------------- methods

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    Register,
    Invite,
    Ack,
    Bye,
    Cancel,
    Options,
    Message,
    Subscribe,
    Notify,
    Refer,
    Prack,
    Update,
    Info,
    /// Any other token (methods are case-sensitive).
    Other(String),
}

impl Method {
    pub fn from_token(s: &str) -> Res<Method> {
        if !is_token(s) {
            return Err(SipError::Method);
        }
        Ok(match s {
            "REGISTER" => Method::Register,
            "INVITE" => Method::Invite,
            "ACK" => Method::Ack,
            "BYE" => Method::Bye,
            "CANCEL" => Method::Cancel,
            "OPTIONS" => Method::Options,
            "MESSAGE" => Method::Message,
            "SUBSCRIBE" => Method::Subscribe,
            "NOTIFY" => Method::Notify,
            "REFER" => Method::Refer,
            "PRACK" => Method::Prack,
            "UPDATE" => Method::Update,
            "INFO" => Method::Info,
            other => Method::Other(other.to_string()),
        })
    }

    pub fn as_str(&self) -> &str {
        match self {
            Method::Register => "REGISTER",
            Method::Invite => "INVITE",
            Method::Ack => "ACK",
            Method::Bye => "BYE",
            Method::Cancel => "CANCEL",
            Method::Options => "OPTIONS",
            Method::Message => "MESSAGE",
            Method::Subscribe => "SUBSCRIBE",
            Method::Notify => "NOTIFY",
            Method::Refer => "REFER",
            Method::Prack => "PRACK",
            Method::Update => "UPDATE",
            Method::Info => "INFO",
            Method::Other(s) => s,
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------- params

/// `;name=value;flag` parameters, in order; names compare case-insensitively.
/// Header parameter values keep their quotes as written (see [`Params::get_unquoted`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Params(pub Vec<(String, Option<String>)>);

impl Params {
    pub fn new() -> Params {
        Params(Vec::new())
    }

    /// The value (`""` for a flag such as `;lr`), or `None` when absent.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_deref().unwrap_or(""))
    }

    pub fn get_unquoted(&self, name: &str) -> Option<String> {
        self.get(name).map(unquote)
    }

    pub fn has(&self, name: &str) -> bool {
        self.0.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
    }

    /// Replaces the first `name` (or appends it); `None` makes it a flag.
    pub fn set(&mut self, name: &str, value: Option<&str>) {
        let v = value.map(str::to_string);
        match self.0.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            Some(slot) => slot.1 = v,
            None => self.0.push((name.to_string(), v)),
        }
    }

    pub fn remove(&mut self, name: &str) {
        self.0.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, Option<&str>)> {
        self.0.iter().map(|(n, v)| (n.as_str(), v.as_deref()))
    }

    /// Parses `""` or `;a=b;c...`. `header` allows quoted values (header
    /// params) and forbids nothing else extra; URI params never have quotes.
    fn parse(s: &str, header: bool) -> Option<Params> {
        let s = s.trim();
        if s.is_empty() {
            return Some(Params::new());
        }
        let s = s.strip_prefix(';')?;
        let segs = if header { split_outside_quotes(s, ';') } else { s.split(';').collect() };
        let mut out = Vec::new();
        for seg in segs {
            let seg = seg.trim();
            let (name, value) = match seg.find('=') {
                Some(i) => (seg[..i].trim(), Some(seg[i + 1..].trim())),
                None => (seg, None),
            };
            if !is_token(name) {
                return None;
            }
            if let Some(v) = value {
                if header && v.starts_with('"') {
                    let (_, rest) = parse_quoted(v)?;
                    if !rest.is_empty() {
                        return None;
                    }
                } else if !v.chars().all(|c| c.is_ascii_graphic() && !";,\"<>?".contains(c)) {
                    return None;
                }
            }
            out.push((name.to_string(), value.map(str::to_string)));
        }
        Some(Params(out))
    }
}

impl fmt::Display for Params {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (n, v) in &self.0 {
            match v {
                Some(v) => write!(f, ";{n}={v}")?,
                None => write!(f, ";{n}")?,
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- URIs

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Sip,
    Sips,
    Tel,
}

/// `sip:user@host:port;params?headers`, `sips:...`, or `tel:+12145550142;params`.
/// For `tel:` the number is the [`Uri::user`] and the host is empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uri {
    pub scheme: Scheme,
    /// The userinfo (`+12145550142`, `alice`, `alice:secret`), or the tel number.
    pub user: Option<String>,
    /// A host name, IPv4 address or bracketed IPv6 address; empty for tel.
    pub host: String,
    pub port: Option<u16>,
    pub params: Params,
    /// `?name=value&...` headers.
    pub headers: Vec<(String, String)>,
}

fn parse_hostport(s: &str) -> Option<(String, Option<u16>)> {
    let (host, rest) = if s.starts_with('[') {
        let end = s.find(']')?;
        let inner = &s[1..end];
        if inner.is_empty() || !inner.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.') {
            return None;
        }
        (&s[..=end], &s[end + 1..])
    } else {
        let (h, r) = split_first(s, ':');
        if h.is_empty() || !h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_') {
            return None;
        }
        (h, r)
    };
    let port = if rest.is_empty() {
        None
    } else {
        let p = rest.strip_prefix(':')?;
        if p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(p.parse::<u16>().ok()?)
    };
    Some((host.to_string(), port))
}

impl Uri {
    /// A `sip:` URI (not validated).
    pub fn sip(user: Option<&str>, host: &str, port: Option<u16>) -> Uri {
        Uri { scheme: Scheme::Sip, user: user.map(str::to_string), host: host.to_string(), port, params: Params::new(), headers: Vec::new() }
    }

    /// A `tel:` URI (not validated).
    pub fn tel(number: &str) -> Uri {
        Uri { scheme: Scheme::Tel, user: Some(number.to_string()), host: String::new(), port: None, params: Params::new(), headers: Vec::new() }
    }

    pub fn parse(s: &str) -> Res<Uri> {
        let s = s.trim();
        if !s.is_ascii() {
            return Err(SipError::Uri("non-ASCII"));
        }
        let colon = s.find(':').ok_or(SipError::Uri("scheme"))?;
        let scheme = match s[..colon].to_ascii_lowercase().as_str() {
            "sip" => Scheme::Sip,
            "sips" => Scheme::Sips,
            "tel" => Scheme::Tel,
            _ => return Err(SipError::Uri("scheme")),
        };
        let rest = &s[colon + 1..];
        if scheme == Scheme::Tel {
            let (num, params) = split_first(rest, ';');
            if num.is_empty() || !num.chars().all(|c| c.is_ascii_hexdigit() || "*#+-.()".contains(c)) {
                return Err(SipError::Uri("number"));
            }
            if rest.contains('?') {
                return Err(SipError::Uri("tel headers"));
            }
            let params = Params::parse(params, false).ok_or(SipError::Uri("params"))?;
            return Ok(Uri { scheme, user: Some(num.to_string()), host: String::new(), port: None, params, headers: Vec::new() });
        }
        let (main, hdrs) = match rest.find('?') {
            Some(i) => (&rest[..i], Some(&rest[i + 1..])),
            None => (rest, None),
        };
        let (user, hostpart) = match main.find('@') {
            Some(i) => (Some(&main[..i]), &main[i + 1..]),
            None => (None, main),
        };
        if let Some(u) = user {
            if u.is_empty() || !u.chars().all(|c| c.is_ascii_graphic() && !"<>\"".contains(c)) {
                return Err(SipError::Uri("user"));
            }
        }
        let (hp, params) = split_first(hostpart, ';');
        let (host, port) = parse_hostport(hp).ok_or(SipError::Uri("host"))?;
        let params = Params::parse(params, false).ok_or(SipError::Uri("params"))?;
        let mut headers = Vec::new();
        if let Some(h) = hdrs {
            for kv in h.split('&') {
                let (k, v) = kv.split_once('=').ok_or(SipError::Uri("headers"))?;
                let ok = |x: &str| x.chars().all(|c| c.is_ascii_graphic() && !"&<>\"".contains(c));
                if k.is_empty() || !ok(k) || !ok(v) {
                    return Err(SipError::Uri("headers"));
                }
                headers.push((k.to_string(), v.to_string()));
            }
        }
        Ok(Uri { scheme, user: user.map(str::to_string), host, port, params, headers })
    }

    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// A URI parameter's value (`""` for a flag like `lr`).
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.get(name)
    }

    pub fn has_param(&self, name: &str) -> bool {
        self.params.has(name)
    }

    pub fn set_param(&mut self, name: &str, value: Option<&str>) {
        self.params.set(name, value);
    }

    pub fn remove_param(&mut self, name: &str) {
        self.params.remove(name);
    }

    /// A `tel:` URI, or a SIP URI with `user=phone`.
    pub fn is_phone(&self) -> bool {
        self.scheme == Scheme::Tel || self.params.get("user").is_some_and(|v| v.eq_ignore_ascii_case("phone"))
    }
}

impl fmt::Display for Uri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.scheme {
            Scheme::Tel => return write!(f, "tel:{}{}", self.user.as_deref().unwrap_or(""), self.params),
            Scheme::Sip => f.write_str("sip:")?,
            Scheme::Sips => f.write_str("sips:")?,
        }
        if let Some(u) = &self.user {
            write!(f, "{u}@")?;
        }
        f.write_str(&self.host)?;
        if let Some(p) = self.port {
            write!(f, ":{p}")?;
        }
        write!(f, "{}", self.params)?;
        for (i, (k, v)) in self.headers.iter().enumerate() {
            write!(f, "{}{k}={v}", if i == 0 { '?' } else { '&' })?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- typed headers

/// `"Display" <uri>;params` (From, To, Contact, Route, P-Asserted-Identity...).
/// The bare addr-spec form (`sip:a@b;tag=x`) parses too; its `;` params are
/// header params, per RFC 3261 20. It always serializes with angle brackets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameAddr {
    pub display: Option<String>,
    pub uri: Uri,
    pub params: Params,
}

impl NameAddr {
    pub fn new(uri: Uri) -> NameAddr {
        NameAddr { display: None, uri, params: Params::new() }
    }

    pub fn parse(s: &str) -> Res<NameAddr> {
        let e = SipError::Value("name-addr");
        let s = s.trim();
        let (display, uri_s, rest) = if s.starts_with('"') {
            let (d, after) = parse_quoted(s).ok_or(e.clone())?;
            let after = after.trim_start();
            if !after.starts_with('<') {
                return Err(e);
            }
            let close = after.find('>').ok_or(e.clone())?;
            (Some(d), &after[1..close], &after[close + 1..])
        } else if let Some(lt) = s.find('<') {
            let d = s[..lt].trim();
            if d.contains(['"', '>']) {
                return Err(e);
            }
            let close = lt + s[lt..].find('>').ok_or(e.clone())?;
            (Some(d.to_string()), &s[lt + 1..close], &s[close + 1..])
        } else {
            let (u, r) = split_first(s, ';');
            (None, u, r)
        };
        let uri = Uri::parse(uri_s)?;
        let params = Params::parse(rest, true).ok_or(e)?;
        Ok(NameAddr { display: display.filter(|d| !d.is_empty()), uri, params })
    }

    pub fn tag(&self) -> Option<&str> {
        self.params.get("tag")
    }

    pub fn set_tag(&mut self, tag: &str) {
        self.params.set("tag", Some(tag));
    }

    /// The `expires` param (Contact).
    pub fn expires(&self) -> Option<u32> {
        self.params.get("expires").and_then(|v| v.parse().ok())
    }
}

impl fmt::Display for NameAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(d) = &self.display {
            write!(f, "{} ", quote(d))?;
        }
        write!(f, "<{}>{}", self.uri, self.params)
    }
}

/// One Via value: `SIP/2.0/UDP host:port;branch=...;received=...;rport`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Via {
    pub transport: String,
    pub host: String,
    pub port: Option<u16>,
    pub params: Params,
}

impl Via {
    pub fn new(transport: &str, host: &str, port: Option<u16>, branch: &str) -> Via {
        let mut params = Params::new();
        params.set("branch", Some(branch));
        Via { transport: transport.to_string(), host: host.to_string(), port, params }
    }

    pub fn parse(s: &str) -> Res<Via> {
        let e = SipError::Value("Via");
        let mut parts = s.splitn(3, '/');
        let name = parts.next().ok_or(e.clone())?.trim();
        let ver = parts.next().ok_or(e.clone())?.trim();
        let rest = parts.next().ok_or(e.clone())?.trim_start();
        if !name.eq_ignore_ascii_case("SIP") || ver != "2.0" {
            return Err(e);
        }
        let tend = rest.find(char::is_whitespace).ok_or(e.clone())?;
        let transport = &rest[..tend];
        if !is_token(transport) {
            return Err(e);
        }
        let (sb, params) = split_first(rest[tend..].trim_start(), ';');
        let (host, port) = parse_hostport(sb.trim()).ok_or(e.clone())?;
        let params = Params::parse(params, true).ok_or(e)?;
        Ok(Via { transport: transport.to_string(), host, port, params })
    }

    pub fn branch(&self) -> Option<&str> {
        self.params.get("branch")
    }

    pub fn received(&self) -> Option<&str> {
        self.params.get("received")
    }

    /// `None`: no rport; `Some(None)`: requested (`;rport`); `Some(Some(p))`: filled in.
    pub fn rport(&self) -> Option<Option<u16>> {
        self.params.get("rport").map(|v| v.parse().ok())
    }

    pub fn set_received(&mut self, addr: &str) {
        self.params.set("received", Some(addr));
    }

    pub fn set_rport(&mut self, port: u16) {
        self.params.set("rport", Some(&port.to_string()));
    }
}

impl fmt::Display for Via {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SIP/2.0/{} {}", self.transport, self.host)?;
        if let Some(p) = self.port {
            write!(f, ":{p}")?;
        }
        write!(f, "{}", self.params)
    }
}

/// `CSeq: 1 INVITE`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CSeq {
    pub seq: u32,
    pub method: Method,
}

impl CSeq {
    pub fn parse(s: &str) -> Res<CSeq> {
        let e = SipError::Value("CSeq");
        let mut it = s.split_whitespace();
        let (n, m) = (it.next().ok_or(e.clone())?, it.next().ok_or(e.clone())?);
        if it.next().is_some() || n.len() > 10 || !n.bytes().all(|b| b.is_ascii_digit()) {
            return Err(e);
        }
        Ok(CSeq { seq: n.parse().map_err(|_| e)?, method: Method::from_token(m)? })
    }
}

impl fmt::Display for CSeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.seq, self.method)
    }
}

/// The Contact header: `*` (REGISTER unbind-all) or a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Contact {
    Star,
    Addrs(Vec<NameAddr>),
}

/// A value with params: Content-Type (`application/sdp`), Event (`reg`),
/// Subscription-State (`active;expires=600`), Reason (`SIP;cause=200;text="..."`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamHeader {
    pub value: String,
    pub params: Params,
}

impl ParamHeader {
    pub fn new(value: &str) -> ParamHeader {
        ParamHeader { value: value.to_string(), params: Params::new() }
    }

    pub fn parse(s: &str) -> Res<ParamHeader> {
        let e = SipError::Value("param header");
        let (v, p) = split_first(s, ';');
        let v = v.trim();
        if v.is_empty() || v.contains('"') {
            return Err(e);
        }
        Ok(ParamHeader { value: v.to_string(), params: Params::parse(p, true).ok_or(e)? })
    }
}

impl fmt::Display for ParamHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.value, self.params)
    }
}

/// Digest-style credentials or challenges:
/// `Digest realm="ims.ecm", nonce="...", algorithm=ECM-AKAv1-HMAC-SHA256`.
/// Values are stored unquoted; `algorithm`, `nc`, `stale` and `qop` are written
/// bare when they are tokens, everything else quoted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Digest {
    pub scheme: String,
    pub params: Vec<(String, String)>,
}

impl Digest {
    pub fn new(scheme: &str) -> Digest {
        Digest { scheme: scheme.to_string(), params: Vec::new() }
    }

    pub fn parse(s: &str) -> Res<Digest> {
        let e = SipError::Value("Digest");
        let s = s.trim();
        let (scheme, mut rest) = match s.find(char::is_whitespace) {
            Some(i) => (&s[..i], s[i..].trim_start()),
            None => (s, ""),
        };
        if !is_token(scheme) {
            return Err(e);
        }
        let mut params = Vec::new();
        while !rest.is_empty() {
            let eq = rest.find('=').ok_or(e.clone())?;
            let name = rest[..eq].trim();
            if !is_token(name) {
                return Err(e);
            }
            let after = rest[eq + 1..].trim_start();
            let (value, tail) = if after.starts_with('"') {
                parse_quoted(after).ok_or(e.clone())?
            } else {
                let (v, t) = split_first(after, ',');
                let v = v.trim();
                if !is_token(v) && !v.chars().all(|c| c.is_ascii_graphic() && !"\",".contains(c)) || v.is_empty() {
                    return Err(e);
                }
                (v.to_string(), t)
            };
            params.push((name.to_string(), value));
            let tail = tail.trim_start();
            rest = match tail.strip_prefix(',') {
                Some(t) => {
                    let t = t.trim_start();
                    if t.is_empty() {
                        return Err(e);
                    }
                    t
                }
                None if tail.is_empty() => "",
                None => return Err(e),
            };
        }
        Ok(Digest { scheme: scheme.to_string(), params })
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, name: &str, value: &str) {
        match self.params.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            Some(slot) => slot.1 = value.to_string(),
            None => self.params.push((name.to_string(), value.to_string())),
        }
    }

    pub fn with(mut self, name: &str, value: &str) -> Digest {
        self.set(name, value);
        self
    }

    pub fn realm(&self) -> Option<&str> {
        self.get("realm")
    }
    pub fn nonce(&self) -> Option<&str> {
        self.get("nonce")
    }
    pub fn response(&self) -> Option<&str> {
        self.get("response")
    }
    pub fn username(&self) -> Option<&str> {
        self.get("username")
    }
    pub fn uri(&self) -> Option<&str> {
        self.get("uri")
    }
    pub fn algorithm(&self) -> Option<&str> {
        self.get("algorithm")
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.scheme)?;
        for (i, (n, v)) in self.params.iter().enumerate() {
            let bare = ["algorithm", "nc", "stale", "qop"].iter().any(|b| n.eq_ignore_ascii_case(b)) && is_token(v);
            let v = if bare { v.clone() } else { quote(v) };
            write!(f, "{}{n}={v}", if i == 0 { " " } else { ", " })?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- header list

fn expand_name(name: &str) -> &str {
    if name.len() != 1 {
        return name;
    }
    match name.as_bytes()[0].to_ascii_lowercase() {
        b'v' => "Via",
        b'f' => "From",
        b't' => "To",
        b'i' => "Call-ID",
        b'm' => "Contact",
        b'l' => "Content-Length",
        b'c' => "Content-Type",
        b'k' => "Supported",
        b's' => "Subject",
        b'e' => "Content-Encoding",
        b'o' => "Event",
        b'r' => "Refer-To",
        b'u' => "Allow-Events",
        b'x' => "Session-Expires",
        b'b' => "Referred-By",
        _ => name,
    }
}

fn name_is(a: &str, b: &str) -> bool {
    expand_name(a).eq_ignore_ascii_case(expand_name(b))
}

/// The header list: order kept, names case-insensitive (compact forms match
/// their full names).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Headers(pub Vec<(String, String)>);

impl Headers {
    pub fn new() -> Headers {
        Headers(Vec::new())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(n, v)| (n.as_str(), v.as_str()))
    }

    /// The first value of `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(n, _)| name_is(n, name)).map(|(_, v)| v.as_str())
    }

    /// Every header line of `name`, in order.
    pub fn get_all(&self, name: &str) -> Vec<&str> {
        self.0.iter().filter(|(n, _)| name_is(n, name)).map(|(_, v)| v.as_str()).collect()
    }

    /// Every comma-separated element of every `name` line (for list headers).
    pub fn values(&self, name: &str) -> Vec<&str> {
        self.get_all(name).into_iter().flat_map(split_list).collect()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Appends (compact names are expanded).
    pub fn push(&mut self, name: &str, value: impl Into<String>) {
        self.0.push((expand_name(name).to_string(), value.into()));
    }

    /// Inserts before every other header (a proxy's Via).
    pub fn prepend(&mut self, name: &str, value: impl Into<String>) {
        self.0.insert(0, (expand_name(name).to_string(), value.into()));
    }

    /// Replaces the first `name` in place and drops the others; appends if absent.
    pub fn set(&mut self, name: &str, value: impl Into<String>) {
        let value = value.into();
        match self.0.iter().position(|(n, _)| name_is(n, name)) {
            Some(i) => {
                self.0[i].1 = value;
                let mut k = 0;
                self.0.retain(|(n, _)| {
                    k += 1;
                    k - 1 == i || !name_is(n, name)
                });
            }
            None => self.push(name, value),
        }
    }

    /// Removes every `name`; returns how many.
    pub fn remove(&mut self, name: &str) -> usize {
        let before = self.0.len();
        self.0.retain(|(n, _)| !name_is(n, name));
        before - self.0.len()
    }

    fn required(&self, name: &'static str) -> Res<&str> {
        self.get(name).ok_or(SipError::Missing(name))
    }

    fn name_addrs(&self, name: &str) -> Res<Vec<NameAddr>> {
        self.values(name).into_iter().map(NameAddr::parse).collect()
    }

    fn number(&self, name: &'static str) -> Res<Option<u32>> {
        let Some(v) = self.get(name) else { return Ok(None) };
        let v = v.trim();
        let digits = &v[..v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len())];
        if digits.is_empty() || digits.len() > 10 {
            return Err(SipError::Value(name));
        }
        digits.parse().map(Some).map_err(|_| SipError::Value(name))
    }

    fn optional<T>(&self, name: &str, f: impl Fn(&str) -> Res<T>) -> Res<Option<T>> {
        self.get(name).map(f).transpose()
    }

    /// Every Via, topmost first.
    pub fn vias(&self) -> Res<Vec<Via>> {
        self.values("Via").into_iter().map(Via::parse).collect()
    }

    pub fn top_via(&self) -> Res<Via> {
        Via::parse(self.values("Via").first().ok_or(SipError::Missing("Via"))?)
    }

    pub fn from(&self) -> Res<NameAddr> {
        NameAddr::parse(self.required("From")?)
    }

    pub fn to(&self) -> Res<NameAddr> {
        NameAddr::parse(self.required("To")?)
    }

    pub fn call_id(&self) -> Res<&str> {
        let v = self.required("Call-ID")?.trim();
        if v.is_empty() || v.contains(char::is_whitespace) {
            return Err(SipError::Value("Call-ID"));
        }
        Ok(v)
    }

    pub fn cseq(&self) -> Res<CSeq> {
        CSeq::parse(self.required("CSeq")?)
    }

    /// Contact (absent = an empty list).
    pub fn contact(&self) -> Res<Contact> {
        let vals = self.values("Contact");
        if vals.contains(&"*") {
            return if vals.len() == 1 { Ok(Contact::Star) } else { Err(SipError::Value("Contact")) };
        }
        vals.into_iter().map(NameAddr::parse).collect::<Res<_>>().map(Contact::Addrs)
    }

    pub fn max_forwards(&self) -> Res<Option<u32>> {
        self.number("Max-Forwards")
    }

    pub fn expires(&self) -> Res<Option<u32>> {
        self.number("Expires")
    }

    /// Retry-After seconds (a trailing comment or params are ignored).
    pub fn retry_after(&self) -> Res<Option<u32>> {
        self.number("Retry-After")
    }

    pub fn content_type(&self) -> Res<Option<ParamHeader>> {
        self.optional("Content-Type", ParamHeader::parse)
    }

    pub fn routes(&self) -> Res<Vec<NameAddr>> {
        self.name_addrs("Route")
    }

    pub fn record_routes(&self) -> Res<Vec<NameAddr>> {
        self.name_addrs("Record-Route")
    }

    pub fn p_asserted_identity(&self) -> Res<Vec<NameAddr>> {
        self.name_addrs("P-Asserted-Identity")
    }

    pub fn p_associated_uri(&self) -> Res<Vec<NameAddr>> {
        self.name_addrs("P-Associated-URI")
    }

    pub fn p_preferred_identity(&self) -> Res<Vec<NameAddr>> {
        self.name_addrs("P-Preferred-Identity")
    }

    pub fn p_called_party_id(&self) -> Res<Option<NameAddr>> {
        self.optional("P-Called-Party-ID", NameAddr::parse)
    }

    /// Privacy values (`id`, `header`, `none`...), split on `;` and `,`.
    pub fn privacy(&self) -> Vec<&str> {
        self.get_all("Privacy").into_iter().flat_map(|v| v.split([';', ','])).map(str::trim).filter(|v| !v.is_empty()).collect()
    }

    pub fn reasons(&self) -> Res<Vec<ParamHeader>> {
        self.values("Reason").into_iter().map(ParamHeader::parse).collect()
    }

    pub fn authorization(&self) -> Res<Option<Digest>> {
        self.optional("Authorization", Digest::parse)
    }

    pub fn proxy_authorization(&self) -> Res<Option<Digest>> {
        self.optional("Proxy-Authorization", Digest::parse)
    }

    pub fn www_authenticate(&self) -> Res<Option<Digest>> {
        self.optional("WWW-Authenticate", Digest::parse)
    }

    pub fn proxy_authenticate(&self) -> Res<Option<Digest>> {
        self.optional("Proxy-Authenticate", Digest::parse)
    }

    pub fn event(&self) -> Res<Option<ParamHeader>> {
        self.optional("Event", ParamHeader::parse)
    }

    pub fn subscription_state(&self) -> Res<Option<ParamHeader>> {
        self.optional("Subscription-State", ParamHeader::parse)
    }

    pub fn allow(&self) -> Res<Vec<Method>> {
        self.values("Allow").into_iter().map(Method::from_token).collect()
    }

    pub fn supported(&self) -> Vec<&str> {
        self.values("Supported")
    }

    pub fn accept(&self) -> Vec<&str> {
        self.values("Accept")
    }

    /// Diversion entries, raw.
    pub fn diversion(&self) -> Vec<&str> {
        self.values("Diversion")
    }

    /// History-Info entries, raw.
    pub fn history_info(&self) -> Vec<&str> {
        self.values("History-Info")
    }
}

// ---------------------------------------------------------------- messages

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Request { method: Method, uri: Uri, headers: Headers, body: Vec<u8> },
    Response { code: u16, reason: String, headers: Headers, body: Vec<u8> },
}

/// The standard reason phrase for `code` (`""` when unknown).
pub fn reason_phrase(code: u16) -> &'static str {
    match code {
        100 => "Trying",
        180 => "Ringing",
        181 => "Call Is Being Forwarded",
        182 => "Queued",
        183 => "Session Progress",
        200 => "OK",
        202 => "Accepted",
        301 => "Moved Permanently",
        302 => "Moved Temporarily",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        415 => "Unsupported Media Type",
        480 => "Temporarily Unavailable",
        481 => "Call/Transaction Does Not Exist",
        486 => "Busy Here",
        487 => "Request Terminated",
        488 => "Not Acceptable Here",
        500 => "Server Internal Error",
        503 => "Service Unavailable",
        600 => "Busy Everywhere",
        603 => "Decline",
        604 => "Does Not Exist Anywhere",
        _ => "",
    }
}

/// A fresh Via branch: the RFC 3261 cookie plus 16 hex digits of `r`.
pub fn new_branch(r: u64) -> String {
    format!("{BRANCH_COOKIE}{r:016x}")
}

/// A fresh From/To tag.
pub fn new_tag(r: u64) -> String {
    format!("{r:016x}")
}

/// A fresh Call-ID `hex@host`.
pub fn new_call_id(r: u64, host: &str) -> String {
    format!("{r:016x}@{host}")
}

impl Message {
    pub fn request(method: Method, uri: Uri) -> Message {
        Message::Request { method, uri, headers: Headers::new(), body: Vec::new() }
    }

    /// A bare response; `reason` defaults to [`reason_phrase`].
    pub fn response(code: u16, reason: Option<&str>) -> Message {
        let reason = reason.unwrap_or(reason_phrase(code)).to_string();
        Message::Response { code, reason, headers: Headers::new(), body: Vec::new() }
    }

    pub fn is_request(&self) -> bool {
        matches!(self, Message::Request { .. })
    }

    pub fn method(&self) -> Option<&Method> {
        match self {
            Message::Request { method, .. } => Some(method),
            Message::Response { .. } => None,
        }
    }

    pub fn uri(&self) -> Option<&Uri> {
        match self {
            Message::Request { uri, .. } => Some(uri),
            Message::Response { .. } => None,
        }
    }

    pub fn code(&self) -> Option<u16> {
        match self {
            Message::Response { code, .. } => Some(*code),
            Message::Request { .. } => None,
        }
    }

    pub fn headers(&self) -> &Headers {
        match self {
            Message::Request { headers, .. } | Message::Response { headers, .. } => headers,
        }
    }

    pub fn headers_mut(&mut self) -> &mut Headers {
        match self {
            Message::Request { headers, .. } | Message::Response { headers, .. } => headers,
        }
    }

    pub fn body(&self) -> &[u8] {
        match self {
            Message::Request { body, .. } | Message::Response { body, .. } => body,
        }
    }

    /// Sets the body and its Content-Type (an empty body removes Content-Type).
    pub fn set_body(&mut self, content_type: &str, data: Vec<u8>) {
        if data.is_empty() {
            self.headers_mut().remove("Content-Type");
        } else {
            self.headers_mut().set("Content-Type", content_type);
        }
        match self {
            Message::Request { body, .. } | Message::Response { body, .. } => *body = data,
        }
    }

    /// The wire bytes, CRLF line ends, with a computed Content-Length last.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut s = match self {
            Message::Request { method, uri, .. } => format!("{method} {uri} SIP/2.0\r\n"),
            Message::Response { code, reason, .. } => format!("SIP/2.0 {code} {reason}\r\n"),
        };
        for (n, v) in self.headers().iter() {
            if !name_is(n, "Content-Length") {
                s.push_str(n);
                s.push_str(": ");
                s.push_str(v);
                s.push_str("\r\n");
            }
        }
        s.push_str(&format!("Content-Length: {}\r\n\r\n", self.body().len()));
        let mut out = s.into_bytes();
        out.extend_from_slice(self.body());
        out
    }

    /// A response to this request (RFC 3261 8.2.6): copies every Via, From,
    /// To, Call-ID and CSeq; adds `to_tag` to To unless it has one (or the code
    /// is 100); copies Record-Route on 101..=299 responses.
    pub fn response_to(&self, code: u16, to_tag: Option<&str>) -> Res<Message> {
        let Message::Request { headers, .. } = self else { return Err(SipError::NotRequest) };
        let mut r = Message::response(code, None);
        let h = r.headers_mut();
        for v in headers.get_all("Via") {
            h.push("Via", v);
        }
        h.push("From", headers.required("From")?);
        let to_raw = headers.required("To")?;
        match to_tag {
            Some(tag) if code != 100 => {
                let mut to = NameAddr::parse(to_raw)?;
                if to.tag().is_none() {
                    to.set_tag(tag);
                    h.push("To", to.to_string());
                } else {
                    h.push("To", to_raw);
                }
            }
            _ => h.push("To", to_raw),
        }
        h.push("Call-ID", headers.required("Call-ID")?);
        h.push("CSeq", headers.required("CSeq")?);
        if (101..300).contains(&code) {
            for v in headers.get_all("Record-Route") {
                h.push("Record-Route", v);
            }
        }
        Ok(r)
    }
}

/// Finds the blank line ending the header block: `(head_end, body_start)`.
fn find_blank(buf: &[u8]) -> Option<(usize, usize)> {
    for i in 0..buf.len() {
        if buf[i] == b'\n' {
            match (buf.get(i + 1), buf.get(i + 2)) {
                (Some(b'\n'), _) => return Some((i, i + 2)),
                (Some(b'\r'), Some(b'\n')) => return Some((i, i + 3)),
                _ => {}
            }
        }
    }
    None
}

fn skip_line_ends(buf: &[u8]) -> usize {
    buf.iter().position(|&b| b != b'\r' && b != b'\n').unwrap_or(buf.len())
}

enum Start {
    Request(Method, Uri),
    Response(u16, String),
}

/// Parses a header block (start line + headers, no blank line): the start
/// line, the headers without Content-Length, and Content-Length.
fn parse_head(head: &[u8]) -> Res<(Start, Headers, Option<usize>)> {
    if head.len() > MAX_HEAD {
        return Err(SipError::TooLarge);
    }
    let text = std::str::from_utf8(head).map_err(|_| SipError::Utf8)?;
    let mut lines = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
    let first = lines.next().ok_or(SipError::Empty)?.trim();
    let first_tok = first.split(char::is_whitespace).next().unwrap_or("");
    let start = if first_tok.eq_ignore_ascii_case("SIP/2.0") {
        let rest = first[first_tok.len()..].trim_start();
        let (code, reason) = match rest.find(char::is_whitespace) {
            Some(i) => (&rest[..i], rest[i..].trim()),
            None => (rest, ""),
        };
        if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
            return Err(SipError::Status);
        }
        let code: u16 = code.parse().map_err(|_| SipError::Status)?;
        if !(100..=699).contains(&code) {
            return Err(SipError::Status);
        }
        Start::Response(code, reason.to_string())
    } else if first_tok.len() >= 4 && first_tok[..4].eq_ignore_ascii_case("SIP/") {
        return Err(SipError::Version);
    } else {
        let parts: Vec<&str> = first.split_whitespace().collect();
        if parts.len() != 3 {
            return Err(SipError::StartLine);
        }
        if !parts[2].eq_ignore_ascii_case("SIP/2.0") {
            return Err(SipError::Version);
        }
        Start::Request(Method::from_token(parts[0])?, Uri::parse(parts[1])?)
    };
    let mut headers = Headers::new();
    for line in lines {
        if line.starts_with([' ', '\t']) {
            let (_, v) = headers.0.last_mut().ok_or(SipError::Header("fold before any header"))?;
            let cont = line.trim();
            if !cont.is_empty() {
                if !v.is_empty() {
                    v.push(' ');
                }
                v.push_str(cont);
            }
            continue;
        }
        let colon = line.find(':').ok_or(SipError::Header("no colon"))?;
        let name = line[..colon].trim();
        if !is_token(name) {
            return Err(SipError::Header("name"));
        }
        headers.push(name, line[colon + 1..].trim());
    }
    let mut cl = None;
    for v in headers.get_all("Content-Length") {
        let v = v.trim();
        if v.is_empty() || v.len() > 9 || !v.bytes().all(|b| b.is_ascii_digit()) {
            return Err(SipError::ContentLength);
        }
        let n: usize = v.parse().map_err(|_| SipError::ContentLength)?;
        if cl.is_some_and(|c| c != n) {
            return Err(SipError::ContentLength);
        }
        cl = Some(n);
    }
    if cl.is_some_and(|c| c > MAX_BODY) {
        return Err(SipError::TooLarge);
    }
    headers.remove("Content-Length");
    Ok((start, headers, cl))
}

fn build(start: Start, headers: Headers, body: Vec<u8>) -> Message {
    match start {
        Start::Request(method, uri) => Message::Request { method, uri, headers, body },
        Start::Response(code, reason) => Message::Response { code, reason, headers, body },
    }
}

/// Parses one datagram (UDP). See the module docs for Content-Length rules.
pub fn parse(data: &[u8]) -> Res<Message> {
    let data = &data[skip_line_ends(data)..];
    if data.is_empty() {
        return Err(SipError::Empty);
    }
    let (head_end, body_start) = find_blank(data).ok_or(if data.len() > MAX_HEAD { SipError::TooLarge } else { SipError::Truncated })?;
    let (start, headers, cl) = parse_head(&data[..head_end])?;
    let rest = &data[body_start..];
    let body = match cl {
        Some(n) if rest.len() < n => return Err(SipError::Truncated),
        Some(n) => &rest[..n],
        None if rest.len() > MAX_BODY => return Err(SipError::TooLarge),
        None => rest,
    };
    Ok(build(start, headers, body.to_vec()))
}

/// Splits a TCP byte stream into messages by Content-Length (missing = 0).
/// CR/LF keep-alives between messages are skipped. After an error the stream
/// can't be resynchronised: close the connection.
#[derive(Clone, Debug, Default)]
pub struct StreamFramer {
    buf: Vec<u8>,
}

impl StreamFramer {
    pub fn new() -> StreamFramer {
        StreamFramer::default()
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Bytes waiting for the rest of a message.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// The next complete message, `Ok(None)` when more bytes are needed.
    pub fn next_message(&mut self) -> Res<Option<Message>> {
        let skip = skip_line_ends(&self.buf);
        self.buf.drain(..skip);
        if self.buf.is_empty() {
            return Ok(None);
        }
        let Some((head_end, body_start)) = find_blank(&self.buf) else {
            return if self.buf.len() > MAX_HEAD + 3 { Err(SipError::TooLarge) } else { Ok(None) };
        };
        let (start, headers, cl) = parse_head(&self.buf[..head_end])?;
        let end = body_start + cl.unwrap_or(0);
        if self.buf.len() < end {
            return Ok(None);
        }
        let body = self.buf[body_start..end].to_vec();
        self.buf.drain(..end);
        Ok(Some(build(start, headers, body)))
    }
}

// ---------------------------------------------------------------- AKA auth

type HmacSha256 = Hmac<Sha256>;

/// `hex(HMAC-SHA256(RES, "username:realm:nonce:method:uri"))`.
pub fn digest_response(username: &str, realm: &str, nonce: &str, uri: &str, method: &str, res: &[u8]) -> String {
    let mut m = <HmacSha256 as Mac>::new_from_slice(res).expect("hmac takes any key length");
    m.update(format!("{username}:{realm}:{nonce}:{method}:{uri}").as_bytes());
    hex(&m.finalize().into_bytes())
}

/// Checks a `response` (hex, any case) in constant time.
pub fn verify_digest_response(username: &str, realm: &str, nonce: &str, uri: &str, method: &str, res: &[u8], response: &str) -> bool {
    let want = digest_response(username, realm, nonce, uri, method, res);
    let got = response.trim().to_ascii_lowercase();
    if got.len() != want.len() {
        return false;
    }
    got.bytes().zip(want.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// The nonce of an ECM AKA challenge: `base64(RAND)`.
pub fn aka_nonce(rand: &[u8; 16]) -> String {
    base64(rand)
}

/// RAND from a nonce: base64 (24 chars) or hex (32 chars) of 16 bytes.
pub fn aka_rand(nonce: &str) -> Res<[u8; 16]> {
    let n = nonce.trim();
    let bytes = if n.len() == 32 && n.bytes().all(|b| b.is_ascii_hexdigit()) { unhex(n) } else { unbase64(n) };
    bytes.and_then(|b| <[u8; 16]>::try_from(b.as_slice()).ok()).ok_or(SipError::Value("nonce"))
}

/// The 401's `WWW-Authenticate` value.
pub fn aka_challenge(realm: &str, rand: &[u8; 16]) -> Digest {
    Digest::new("Digest").with("realm", realm).with("nonce", &aka_nonce(rand)).with("algorithm", AKA_ALGORITHM)
}

/// The phone's `Authorization` value answering a challenge.
pub fn aka_authorization(username: &str, realm: &str, nonce: &str, uri: &str, method: &str, res: &[u8]) -> Digest {
    Digest::new("Digest")
        .with("username", username)
        .with("realm", realm)
        .with("nonce", nonce)
        .with("uri", uri)
        .with("response", &digest_response(username, realm, nonce, uri, method, res))
        .with("algorithm", AKA_ALGORITHM)
}

/// Verifies an `Authorization` against the expected RES for `method`.
pub fn verify_authorization(auth: &Digest, method: &str, res: &[u8]) -> bool {
    let (Some(u), Some(r), Some(n), Some(uri), Some(resp)) = (auth.username(), auth.realm(), auth.nonce(), auth.uri(), auth.response()) else {
        return false;
    };
    auth.scheme.eq_ignore_ascii_case("Digest")
        && auth.algorithm().is_some_and(|a| a.eq_ignore_ascii_case(AKA_ALGORITHM))
        && verify_digest_response(u, r, n, uri, method, res, resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGISTER: &str = "REGISTER sip:ims.ecm SIP/2.0\r\n\
Via: SIP/2.0/UDP 10.45.0.7:5060;branch=z9hG4bK1a2b3c4d;rport\r\n\
Max-Forwards: 70\r\n\
From: <sip:+12145550142@ims.ecm>;tag=4fa3\r\n\
To: <sip:+12145550142@ims.ecm>\r\n\
Call-ID: a84b4c76e66710@10.45.0.7\r\n\
CSeq: 1 REGISTER\r\n\
Contact: <sip:+12145550142@10.45.0.7:5060>;expires=600;+sip.instance=\"<urn:gsma:imei:35209900-176148-1>\"\r\n\
Authorization: Digest username=\"310150123456789@ims.ecm\", realm=\"ims.ecm\", nonce=\"\", uri=\"sip:ims.ecm\", response=\"\"\r\n\
Expires: 600\r\n\
Supported: path, gruu\r\n\
Allow: INVITE, ACK, CANCEL, BYE, MESSAGE, OPTIONS\r\n\
P-Preferred-Identity: <sip:+12145550142@ims.ecm>\r\n\
Content-Length: 0\r\n\
\r\n";

    const SDP: &str = "v=0\r\no=- 1 1 IN IP4 10.45.0.7\r\ns=-\r\nc=IN IP4 10.45.0.7\r\nt=0 0\r\nm=audio 40000 RTP/AVP 96 101\r\na=rtpmap:96 ADPCM-WB/16000\r\na=rtpmap:101 telephone-event/8000\r\n";

    fn invite() -> Vec<u8> {
        let mut m = format!(
            "INVITE sip:+12145550199@ims.ecm;user=phone SIP/2.0\r\n\
Via: SIP/2.0/TCP 10.45.0.7:5060;branch=z9hG4bKdeadbeef\r\n\
Via: SIP/2.0/UDP 10.0.0.1;branch=z9hG4bKcafe;received=1.2.3.4;rport=5061\r\n\
Record-Route: <sip:pcscf.ims.ecm;lr>, <sip:scscf.ims.ecm;lr>\r\n\
Max-Forwards: 69\r\n\
From: \"Alice \\\"A\\\" Smith\" <sip:+12145550142@ims.ecm>;tag=abc\r\n\
To: <tel:+12145550199>\r\n\
Call-ID: 0011@10.45.0.7\r\n\
CSeq: 10 INVITE\r\n\
Contact: <sip:+12145550142@10.45.0.7:5060>\r\n\
P-Asserted-Identity: <sip:+12145550142@ims.ecm>, <tel:+12145550142>\r\n\
Privacy: id;header\r\n\
Content-Type: application/sdp\r\n\
Content-Length: {}\r\n\r\n",
            SDP.len()
        )
        .into_bytes();
        m.extend_from_slice(SDP.as_bytes());
        m
    }

    #[test]
    fn parses_register() {
        let m = parse(REGISTER.as_bytes()).unwrap();
        assert_eq!(m.method(), Some(&Method::Register));
        assert_eq!(m.uri().unwrap().host(), "ims.ecm");
        let h = m.headers();
        let via = h.top_via().unwrap();
        assert_eq!((via.transport.as_str(), via.host.as_str(), via.port), ("UDP", "10.45.0.7", Some(5060)));
        assert_eq!(via.branch(), Some("z9hG4bK1a2b3c4d"));
        assert_eq!(via.rport(), Some(None));
        assert_eq!(h.max_forwards().unwrap(), Some(70));
        assert_eq!(h.from().unwrap().tag(), Some("4fa3"));
        assert_eq!(h.to().unwrap().tag(), None);
        assert_eq!(h.call_id().unwrap(), "a84b4c76e66710@10.45.0.7");
        assert_eq!(h.cseq().unwrap(), CSeq { seq: 1, method: Method::Register });
        let Contact::Addrs(c) = h.contact().unwrap() else { panic!() };
        assert_eq!(c[0].expires(), Some(600));
        assert_eq!(c[0].params.get_unquoted("+sip.instance").unwrap(), "<urn:gsma:imei:35209900-176148-1>");
        let a = h.authorization().unwrap().unwrap();
        assert_eq!(a.username(), Some("310150123456789@ims.ecm"));
        assert_eq!(a.response(), Some(""));
        assert_eq!(h.expires().unwrap(), Some(600));
        assert_eq!(h.supported(), vec!["path", "gruu"]);
        assert_eq!(h.allow().unwrap().len(), 6);
        assert_eq!(h.p_preferred_identity().unwrap()[0].uri.user(), Some("+12145550142"));
        assert!(!h.contains("Content-Length"));
        assert!(m.body().is_empty());
    }

    #[test]
    fn parses_invite_with_sdp() {
        let m = parse(&invite()).unwrap();
        let h = m.headers();
        assert!(m.uri().unwrap().is_phone());
        let vias = h.vias().unwrap();
        assert_eq!(vias.len(), 2);
        assert_eq!(vias[1].received(), Some("1.2.3.4"));
        assert_eq!(vias[1].rport(), Some(Some(5061)));
        assert_eq!(vias[1].port, None);
        let rr = h.record_routes().unwrap();
        assert_eq!(rr.len(), 2);
        assert!(rr[0].uri.has_param("lr"));
        assert_eq!(h.from().unwrap().display.as_deref(), Some("Alice \"A\" Smith"));
        let to = h.to().unwrap();
        assert_eq!(to.uri.scheme, Scheme::Tel);
        assert_eq!(to.uri.user(), Some("+12145550199"));
        assert_eq!(h.p_asserted_identity().unwrap().len(), 2);
        assert_eq!(h.privacy(), vec!["id", "header"]);
        assert_eq!(h.content_type().unwrap().unwrap().value, "application/sdp");
        assert_eq!(m.body(), SDP.as_bytes());
    }

    #[test]
    fn responses_from_request() {
        let inv = parse(&invite()).unwrap();
        let r180 = inv.response_to(180, Some("t1")).unwrap();
        assert_eq!(r180.code(), Some(180));
        let Message::Response { reason, .. } = &r180 else { panic!() };
        assert_eq!(reason, "Ringing");
        let h = r180.headers();
        assert_eq!(h.vias().unwrap().len(), 2);
        assert_eq!(h.to().unwrap().tag(), Some("t1"));
        assert_eq!(h.from().unwrap().tag(), Some("abc"));
        assert_eq!(h.call_id().unwrap(), "0011@10.45.0.7");
        assert_eq!(h.cseq().unwrap().method, Method::Invite);
        assert_eq!(h.record_routes().unwrap().len(), 2);
        let r100 = inv.response_to(100, Some("t1")).unwrap();
        assert_eq!(r100.headers().to().unwrap().tag(), None);
        assert!(!r100.headers().contains("Record-Route"));
        let mut r200 = inv.response_to(200, Some("t1")).unwrap();
        r200.set_body("application/sdp", SDP.as_bytes().to_vec());
        let wire = r200.to_bytes();
        assert!(wire.starts_with(b"SIP/2.0 200 OK\r\n"));
        let back = parse(&wire).unwrap();
        assert_eq!(back, r200);
        assert_eq!(r200.response_to(200, None), Err(SipError::NotRequest));
        // An existing To tag is kept.
        let again = back.headers().to().unwrap();
        let mut req = Message::request(Method::Bye, Uri::sip(Some("b"), "h", None));
        for (n, v) in back.headers().iter() {
            req.headers_mut().push(n, v);
        }
        assert_eq!(req.response_to(200, Some("other")).unwrap().headers().to().unwrap(), again);
    }

    #[test]
    fn parses_180_response() {
        let raw = "SIP/2.0 180 Ringing\r\nv: SIP/2.0/UDP a.b;branch=z9hG4bKx\r\nf: <sip:a@b>;tag=1\r\nt: <sip:c@d>;tag=2\r\ni: id1\r\nCSeq: 1 INVITE\r\nl: 0\r\n\r\n";
        let m = parse(raw.as_bytes()).unwrap();
        assert_eq!(m.code(), Some(180));
        assert_eq!(m.headers().to().unwrap().tag(), Some("2"));
        assert_eq!(m.headers().0[0].0, "Via");
        assert_eq!(m.headers().call_id().unwrap(), "id1");
    }

    #[test]
    fn message_with_binary_body() {
        let body: Vec<u8> = vec![0x00, 0x01, 0x0d, 0x0a, 0x0d, 0x0a, 0xff, 0x80, 0x41];
        let mut m = Message::request(Method::Message, Uri::parse("sip:+12145550199@ims.ecm;user=phone").unwrap());
        let h = m.headers_mut();
        h.push("Via", Via::new("UDP", "10.45.0.7", Some(5060), &new_branch(7)).to_string());
        h.push("From", format!("<sip:+12145550142@ims.ecm>;tag={}", new_tag(9)));
        h.push("To", "<sip:+12145550199@ims.ecm>");
        h.push("Call-ID", new_call_id(0xabc, "10.45.0.7"));
        h.push("CSeq", "1 MESSAGE");
        h.push("Max-Forwards", "70");
        m.set_body("application/vnd.3gpp.sms", body.clone());
        let wire = m.to_bytes();
        let back = parse(&wire).unwrap();
        assert_eq!(back.body(), &body[..]);
        assert_eq!(back.headers().content_type().unwrap().unwrap().value, "application/vnd.3gpp.sms");
        assert_eq!(back, m);
        assert_eq!(back.headers().top_via().unwrap().branch(), Some("z9hG4bK0000000000000007"));
    }

    #[test]
    fn compact_and_odd_whitespace() {
        let raw = "\r\n\r\nOPTIONS   sip:h.example   SIP/2.0\n\
v:SIP / 2.0 / UDP  h.example:5060 ;branch=z9hG4bK1 ; rport\n\
f :  Bob <sip:bob@h.example>  ;tag=9\n\
t: sip:x@h.example;tag=3\n\
i:  abc\n\
CSeq:\t5   OPTIONS\n\
Subject: a long\n  folded\n\tsubject\n\
m: *\n\
k: timer,  100rel\n\
c: text/plain\n\
l: 2\n\nhiEXTRA";
        let m = parse(raw.as_bytes()).unwrap();
        let h = m.headers();
        let via = h.top_via().unwrap();
        assert_eq!(via.port, Some(5060));
        assert_eq!(via.rport(), Some(None));
        let f = h.from().unwrap();
        assert_eq!(f.display.as_deref(), Some("Bob"));
        assert_eq!(f.tag(), Some("9"));
        let t = h.to().unwrap();
        assert_eq!(t.tag(), Some("3"));
        assert!(t.uri.params.is_empty());
        assert_eq!(h.cseq().unwrap().seq, 5);
        assert_eq!(h.get("subject"), Some("a long folded subject"));
        assert_eq!(h.contact().unwrap(), Contact::Star);
        assert_eq!(h.supported(), vec!["timer", "100rel"]);
        assert_eq!(h.get("Content-Type"), Some("text/plain"));
        assert_eq!(h.get("c"), Some("text/plain"));
        assert_eq!(m.body(), b"hi");
        let again = parse(&m.to_bytes()).unwrap();
        assert_eq!(again, m);
    }

    #[test]
    fn round_trip() {
        for raw in [REGISTER.as_bytes().to_vec(), invite()] {
            let m = parse(&raw).unwrap();
            let wire = m.to_bytes();
            assert_eq!(parse(&wire).unwrap(), m);
            assert_eq!(parse(&wire).unwrap().to_bytes(), wire);
        }
        assert_eq!(parse(REGISTER.as_bytes()).unwrap().to_bytes(), REGISTER.as_bytes());
    }

    #[test]
    fn uris() {
        let u = Uri::parse("sip:+12145550142@ims.ecm:5060;user=phone;lr?Subject=hi&X=y").unwrap();
        assert_eq!(u.user(), Some("+12145550142"));
        assert_eq!(u.host(), "ims.ecm");
        assert_eq!(u.port(), Some(5060));
        assert_eq!(u.param("lr"), Some(""));
        assert!(u.is_phone());
        assert_eq!(u.headers.len(), 2);
        assert_eq!(u.to_string(), "sip:+12145550142@ims.ecm:5060;user=phone;lr?Subject=hi&X=y");
        let mut u2 = Uri::parse("SIP:[2001:db8::1]:5070").unwrap();
        assert_eq!(u2.host(), "[2001:db8::1]");
        u2.set_param("transport", Some("tcp"));
        assert_eq!(u2.to_string(), "sip:[2001:db8::1]:5070;transport=tcp");
        u2.remove_param("transport");
        assert!(!u2.has_param("transport"));
        let t = Uri::parse("tel:+12145550142;phone-context=ims.ecm").unwrap();
        assert_eq!(t.scheme, Scheme::Tel);
        assert_eq!(t.user(), Some("+12145550142"));
        assert!(t.is_phone());
        assert_eq!(t.to_string(), "tel:+12145550142;phone-context=ims.ecm");
        for bad in ["", "sip:", "http://x", "sip:@h", "sip:a@", "sip:a@h:99999", "sip:a@h;", "sip:h?x", "tel:", "tel:12a#?", "sip:a b@h", "sip:h:5x", "sip:[zz]"] {
            assert!(Uri::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn content_length_mismatch() {
        let base = "MESSAGE sip:a@b SIP/2.0\r\nCall-ID: 1\r\nContent-Length: 5\r\n\r\n";
        assert_eq!(parse(format!("{base}abc").as_bytes()), Err(SipError::Truncated));
        assert_eq!(parse(format!("{base}abcdefgh").as_bytes()).unwrap().body(), b"abcde");
        let two = "MESSAGE sip:a@b SIP/2.0\r\nl: 1\r\nContent-Length: 2\r\n\r\nab";
        assert_eq!(parse(two.as_bytes()), Err(SipError::ContentLength));
        let neg = "MESSAGE sip:a@b SIP/2.0\r\nContent-Length: -1\r\n\r\n";
        assert_eq!(parse(neg.as_bytes()), Err(SipError::ContentLength));
        let none = "MESSAGE sip:a@b SIP/2.0\r\nCall-ID: 1\r\n\r\nxyz";
        assert_eq!(parse(none.as_bytes()).unwrap().body(), b"xyz");
        let huge = "MESSAGE sip:a@b SIP/2.0\r\nContent-Length: 999999999\r\n\r\n";
        assert_eq!(parse(huge.as_bytes()), Err(SipError::TooLarge));
        assert_eq!(parse(b"MESSAGE sip:a@b SIP/2.0\r\nCall-ID: 1\r\n"), Err(SipError::Truncated));
    }

    #[test]
    fn errors() {
        assert_eq!(parse(b""), Err(SipError::Empty));
        assert_eq!(parse(b"\r\n\r\n"), Err(SipError::Empty));
        assert_eq!(parse(b"SIP/2.0 99 Low\r\n\r\n"), Err(SipError::Status));
        assert_eq!(parse(b"SIP/3.0 200 OK\r\n\r\n"), Err(SipError::Version));
        assert_eq!(parse(b"INVITE sip:a@b SIP/1.0\r\n\r\n"), Err(SipError::Version));
        assert_eq!(parse(b"INV(TE sip:a@b SIP/2.0\r\n\r\n"), Err(SipError::Method));
        assert_eq!(parse(b"INVITE sip:a@b\r\n\r\n"), Err(SipError::StartLine));
        assert_eq!(parse(b"INVITE sip:a@b SIP/2.0\r\nNoColon\r\n\r\n"), Err(SipError::Header("no colon")));
        assert_eq!(parse(b"INVITE sip:a@b SIP/2.0\r\n folded\r\n\r\n"), Err(SipError::Header("fold before any header")));
        assert_eq!(parse(b"INVITE sip:a@b SIP/2.0\r\nBad Name: x\r\n\r\n"), Err(SipError::Header("name")));
        assert_eq!(parse(b"INVITE sip:a@b SIP/2.0\r\nX: \xff\r\n\r\n"), Err(SipError::Utf8));
        let m = parse(b"SIP/2.0 200\r\n\r\n").unwrap();
        assert_eq!(m.headers().from(), Err(SipError::Missing("From")));
        assert!(m.headers().contact().unwrap() == Contact::Addrs(vec![]));
    }

    #[test]
    fn typed_helpers() {
        let mut h = Headers::new();
        h.push("Reason", "SIP;cause=200;text=\"Call completed; elsewhere\", Q.850;cause=16");
        h.push("Event", "reg;id=7");
        h.push("Subscription-State", "active;expires=600");
        h.push("Retry-After", "120 (busy);duration=60");
        h.push("WWW-Authenticate", aka_challenge("ims.ecm", &[7; 16]).to_string());
        h.push("P-Called-Party-ID", "<sip:+12145550199@ims.ecm;user=phone>");
        h.push("P-Associated-URI", "<sip:+12145550142@ims.ecm>, <tel:+12145550142>");
        h.push("Diversion", "<tel:+12145550100>;reason=unconditional");
        h.push("History-Info", "<sip:a@b>;index=1, <sip:c@d>;index=1.1");
        h.push("Accept", "application/sdp, application/3gpp-ims+xml");
        let r = h.reasons().unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].params.get_unquoted("text").unwrap(), "Call completed; elsewhere");
        assert_eq!(r[1].value, "Q.850");
        assert_eq!(h.event().unwrap().unwrap().params.get("id"), Some("7"));
        assert_eq!(h.subscription_state().unwrap().unwrap().value, "active");
        assert_eq!(h.retry_after().unwrap(), Some(120));
        let w = h.www_authenticate().unwrap().unwrap();
        assert_eq!(w.algorithm(), Some(AKA_ALGORITHM));
        assert_eq!(aka_rand(w.nonce().unwrap()).unwrap(), [7; 16]);
        assert!(h.p_called_party_id().unwrap().unwrap().uri.is_phone());
        assert_eq!(h.p_associated_uri().unwrap().len(), 2);
        assert_eq!(h.diversion().len(), 1);
        assert_eq!(h.history_info().len(), 2);
        assert_eq!(h.accept().len(), 2);
        h.set("event", "presence");
        assert_eq!(h.get_all("Event"), vec!["presence"]);
        h.push("Event", "x");
        h.set("Event", "y");
        assert_eq!(h.get_all("Event"), vec!["y"]);
        assert_eq!(h.remove("EVENT"), 1);
        h.prepend("v", "SIP/2.0/UDP a");
        assert_eq!(h.0[0].0, "Via");
        let mut via = h.top_via().unwrap();
        via.set_received("9.9.9.9");
        via.set_rport(4000);
        assert_eq!(via.to_string(), "SIP/2.0/UDP a;received=9.9.9.9;rport=4000");
        assert_eq!(reason_phrase(488), "Not Acceptable Here");
        assert_eq!(reason_phrase(299), "");
        for code in [100, 180, 181, 182, 183, 200, 202, 301, 302, 400, 401, 403, 404, 407, 408, 415, 480, 481, 486, 487, 488, 500, 503, 600, 603, 604] {
            assert!(!reason_phrase(code).is_empty());
        }
    }

    #[test]
    fn aka_digest() {
        let res = [0x5au8; 16];
        let rand = [0x11u8; 16];
        let nonce = aka_nonce(&rand);
        assert_eq!(nonce.len(), 24);
        assert_eq!(aka_rand(&nonce).unwrap(), rand);
        assert_eq!(aka_rand(&hex(&rand)).unwrap(), rand);
        assert!(aka_rand("short").is_err());
        assert!(aka_rand("").is_err());
        let r = digest_response("u@ims.ecm", "ims.ecm", &nonce, "sip:ims.ecm", "REGISTER", &res);
        assert_eq!(r.len(), 64);
        assert!(verify_digest_response("u@ims.ecm", "ims.ecm", &nonce, "sip:ims.ecm", "REGISTER", &res, &r.to_uppercase()));
        assert!(!verify_digest_response("u@ims.ecm", "ims.ecm", &nonce, "sip:ims.ecm", "INVITE", &res, &r));
        assert!(!verify_digest_response("u@ims.ecm", "ims.ecm", &nonce, "sip:ims.ecm", "REGISTER", &[0; 16], &r));
        let auth = aka_authorization("u@ims.ecm", "ims.ecm", &nonce, "sip:ims.ecm", "REGISTER", &res);
        let wire = auth.to_string();
        assert!(wire.starts_with("Digest username=\"u@ims.ecm\", realm=\"ims.ecm\""));
        assert!(wire.ends_with("algorithm=ECM-AKAv1-HMAC-SHA256"));
        let back = Digest::parse(&wire).unwrap();
        assert_eq!(back, auth);
        assert!(verify_authorization(&back, "REGISTER", &res));
        assert!(!verify_authorization(&back, "REGISTER", &[1; 16]));
        assert!(!verify_authorization(&back.clone().with("algorithm", "MD5"), "REGISTER", &res));
        for bad in ["", "Digest realm", "Digest realm=\"x", "Digest a=b,", "Digest a=b c=d", "Di\"gest a=b"] {
            assert!(Digest::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(Digest::parse("Digest").unwrap().params.len(), 0);
        // RFC 4648 vectors.
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(unbase64("Zm9vYmFy").unwrap(), b"foobar");
        assert!(unbase64("Zg=a").is_none());
    }

    #[test]
    fn tcp_framer_any_split() {
        let mut stream = b"\r\n\r\n".to_vec();
        let msgs = [parse(REGISTER.as_bytes()).unwrap(), parse(&invite()).unwrap(), parse(b"SIP/2.0 200 OK\r\nCall-ID: x\r\n\r\n").unwrap()];
        for m in &msgs {
            stream.extend(m.to_bytes());
            stream.extend(b"\r\n\r\n");
        }
        let mut seed = 0x1234_5678u64;
        for split in 0..200 {
            let mut f = StreamFramer::new();
            let mut got = Vec::new();
            let mut i = 0;
            while i < stream.len() {
                let n = if split < stream.len() && split > 0 && i == 0 { split } else { 1 + (xorshift(&mut seed) % 97) as usize };
                let end = (i + n).min(stream.len());
                f.push(&stream[i..end]);
                i = end;
                while let Some(m) = f.next_message().unwrap() {
                    got.push(m);
                }
            }
            assert_eq!(got, msgs);
            assert_eq!(f.buffered(), 0);
        }
        let mut f = StreamFramer::new();
        f.push(b"BYE sip:a@b SIP/2.0\r\nCall-ID: 1\r\n\r\n");
        assert_eq!(f.next_message().unwrap().unwrap().body(), b"");
        f.push(&vec![b'a'; MAX_HEAD + 10]);
        assert_eq!(f.next_message(), Err(SipError::TooLarge));
    }

    fn xorshift(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    fn exercise(m: &Message) {
        let h = m.headers();
        let _ = (h.vias(), h.top_via(), h.from(), h.to(), h.call_id(), h.cseq(), h.contact(), h.max_forwards(), h.expires());
        let _ = (h.retry_after(), h.content_type(), h.routes(), h.record_routes(), h.p_asserted_identity(), h.p_associated_uri());
        let _ = (h.p_preferred_identity(), h.p_called_party_id(), h.privacy(), h.reasons(), h.authorization(), h.www_authenticate());
        let _ = (h.proxy_authorization(), h.proxy_authenticate(), h.event(), h.subscription_state(), h.allow(), h.supported());
        if let Ok(Some(d)) = h.authorization() {
            assert_eq!(Digest::parse(&d.to_string()).unwrap(), d);
            let _ = verify_authorization(&d, "REGISTER", &[1; 16]);
        }
        if let Ok(f) = h.from() {
            assert_eq!(NameAddr::parse(&f.to_string()).unwrap(), f);
        }
        if let Ok(v) = h.vias() {
            for v in v {
                assert_eq!(Via::parse(&v.to_string()).unwrap(), v);
            }
        }
        let _ = m.response_to(200, Some("t"));
        let wire = m.to_bytes();
        assert_eq!(&parse(&wire).unwrap(), m);
    }

    #[test]
    fn malformed_never_panics() {
        let samples: Vec<Vec<u8>> = vec![
            REGISTER.as_bytes().to_vec(),
            invite(),
            b"SIP/2.0 180 Ringing\r\nv: SIP/2.0/UDP a.b;branch=z9hG4bKx\r\nf: \"A\" <sip:a@b>;tag=1\r\nt: <tel:+1>;tag=2\r\ni: id1\r\nCSeq: 1 INVITE\r\nl: 0\r\n\r\n".to_vec(),
            b"MESSAGE sip:a@[::1]:5060 SIP/2.0\r\nContent-Type: application/vnd.3gpp.sms\r\nContent-Length: 4\r\n\r\n\x00\x01\x02\x03".to_vec(),
        ];
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut ok = 0;
        for i in 0..20_000 {
            let r = xorshift(&mut seed);
            let mut data = if i % 4 == 0 {
                (0..(r % 300)).map(|_| xorshift(&mut seed) as u8).collect::<Vec<u8>>()
            } else {
                samples[(r % samples.len() as u64) as usize].clone()
            };
            if i % 4 != 0 {
                for _ in 0..1 + r % 4 {
                    let k = xorshift(&mut seed);
                    let pos = (k as usize >> 8) % data.len();
                    match k % 4 {
                        0 => data[pos] ^= 1 << (k >> 3 & 7),
                        1 => data[pos] = b" :;,<>\"\r\n@?=&/\\"[(k >> 5) as usize % 15],
                        2 => data.truncate(pos),
                        _ => data.insert(pos, (k >> 9) as u8),
                    }
                    if data.is_empty() {
                        break;
                    }
                }
            }
            if let Ok(m) = parse(&data) {
                ok += 1;
                exercise(&m);
            }
            let mut f = StreamFramer::new();
            for chunk in data.chunks(1 + (r % 13) as usize) {
                f.push(chunk);
                while let Ok(Some(m)) = f.next_message() {
                    exercise(&m);
                }
            }
            if let Ok(s) = std::str::from_utf8(&data) {
                if let Ok(u) = Uri::parse(s) {
                    assert_eq!(Uri::parse(&u.to_string()).unwrap(), u);
                }
                let _ = (NameAddr::parse(s), Via::parse(s), Digest::parse(s), ParamHeader::parse(s), CSeq::parse(s), aka_rand(s));
            }
        }
        assert!(ok > 1000, "only {ok} mutated inputs parsed");
    }
}
