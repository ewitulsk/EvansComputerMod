//! The HTTP client programs on a computer share (`pkg`, `curl`, the Browser,
//! Python's and Lua's `net`): `http://` and `https://`, HTTP/1.1 with
//! `Connection: close`, Content-Length and chunked bodies, redirects, bodies
//! streamed into a file with progress, and a staged connectivity check.
//!
//! The protocol here is plain Rust over a [`Connector`]; [`sys::Sockets`] is
//! the computer's network, [`native::Native`] the host's (tests and tools).

pub mod probe;
pub mod url;

#[cfg(target_os = "wasi")]
pub mod sys;

#[cfg(not(target_arch = "wasm32"))]
pub mod native;

pub use ecm_tls::{Check, TlsError, Trust};
pub use url::Url;

use std::io::{Read, Write};

pub const MAX_REDIRECTS: usize = 5;
/// The largest response head (status line and headers) read.
pub const MAX_HEAD: usize = 64 * 1024;
/// The default cap on a body kept in memory.
pub const DEFAULT_MAX_BODY: u64 = 16 * 1024 * 1024;

/// Why a request failed, by how far it got. The order is the order a
/// connection is made in, so the first failing step names the problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    BadUrl(String),
    /// The program may not use the network (an app without the permission).
    NotAllowed,
    /// This computer has no network to send on.
    NoNetwork,
    /// The name has no address: (host, why).
    Resolve(String, String),
    /// Nobody answered at the address (refused, unreachable, timed out).
    Connect(String),
    /// The server's certificate was refused, or TLS broke.
    Tls(TlsError),
    /// The answer was not HTTP, or was cut short.
    Protocol(String),
    TooManyRedirects,
    /// The body is larger than the caller allows.
    TooLarge(u64),
    /// The connection broke, or the file being written could not be.
    Io(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::BadUrl(m) => write!(f, "{m}"),
            HttpError::NotAllowed => write!(f, "not allowed to use the network"),
            HttpError::NoNetwork => write!(f, "this computer has no network"),
            HttpError::Resolve(h, why) => write!(f, "cannot resolve {h}: {why}"),
            HttpError::Connect(a) => write!(f, "cannot connect to {a}"),
            HttpError::Tls(e) => write!(f, "{e}"),
            HttpError::Protocol(m) => write!(f, "bad answer from the server: {m}"),
            HttpError::TooManyRedirects => write!(f, "more than {MAX_REDIRECTS} redirects"),
            HttpError::TooLarge(max) => write!(f, "the answer is larger than {max} bytes"),
            HttpError::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for HttpError {}

/// A TCP connection: plain bytes both ways.
pub trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

/// How the client reaches a server.
pub trait Connector {
    /// Connect to `host:port`; also says which address that was (it decides
    /// whom a TLS client trusts, see `ecm_tls`).
    fn connect(&mut self, host: &str, port: u16) -> Result<(Box<dyn Stream>, [u8; 4]), HttpError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn new(method: &str, url: &str) -> Result<Request, HttpError> {
        Ok(Request { method: method.to_ascii_uppercase(), url: Url::parse(url).map_err(HttpError::BadUrl)?, headers: Vec::new(), body: Vec::new() })
    }

    pub fn get(url: &str) -> Result<Request, HttpError> {
        Request::new("GET", url)
    }

    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn body(mut self, content_type: &str, body: Vec<u8>) -> Request {
        self.body = body;
        self.header("Content-Type", content_type)
    }

    pub fn bearer(self, token: &str) -> Request {
        self.header("Authorization", &format!("Bearer {token}"))
    }

    fn has(&self, name: &str) -> bool {
        self.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
    }

    /// The bytes sent to the server.
    pub fn encode(&self, user_agent: &str) -> Vec<u8> {
        let mut out = format!("{} {} HTTP/1.1\r\nHost: {}\r\n", self.method, self.url.target, self.url.authority());
        if !self.has("User-Agent") {
            out.push_str(&format!("User-Agent: {user_agent}\r\n"));
        }
        if !self.has("Accept") {
            out.push_str("Accept: */*\r\n");
        }
        for (k, v) in &self.headers {
            out.push_str(&format!("{k}: {v}\r\n"));
        }
        if !self.body.is_empty() || matches!(self.method.as_str(), "POST" | "PUT" | "PATCH") {
            out.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
        }
        out.push_str("Connection: close\r\n\r\n");
        let mut bytes = out.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The final address (after redirects).
    pub url: Url,
    /// `HTTP/1.1` or `HTTP/1.0`, as the server wrote it.
    pub version: String,
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    /// The body, unless it went to a sink.
    pub body: Vec<u8>,
    /// Bytes of body received (also when they went to a sink).
    pub length: u64,
    /// `TLSv1.3` or `TLSv1.2` for https.
    pub tls: Option<&'static str>,
    pub peer: [u8; 4],
    pub redirects: usize,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Where a body goes and how much of it is welcome.
pub struct Transfer<'a> {
    /// Write the body here instead of keeping it (only for a 2xx answer).
    pub sink: Option<&'a mut dyn Write>,
    /// Called after each chunk: (bytes so far, total if the server said).
    pub progress: Option<&'a mut dyn FnMut(u64, Option<u64>)>,
    pub max_body: u64,
}

impl Default for Transfer<'_> {
    fn default() -> Self {
        Transfer { sink: None, progress: None, max_body: DEFAULT_MAX_BODY }
    }
}

/// The status line and headers of an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// `HTTP/1.1` or `HTTP/1.0`, as the server wrote it.
    pub version: String,
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn parse(text: &str) -> Result<Head, HttpError> {
        let mut lines = text.split("\r\n");
        let status_line = lines.next().unwrap_or("");
        let mut parts = status_line.splitn(3, ' ');
        let proto = parts.next().unwrap_or("");
        if !proto.starts_with("HTTP/") {
            let shown: String = status_line.chars().take(40).collect();
            return Err(HttpError::Protocol(format!("not HTTP: {shown:?}")));
        }
        let status: u16 = parts.next().and_then(|s| s.parse().ok()).filter(|s| (100..600).contains(s)).ok_or_else(|| HttpError::Protocol("bad status line".into()))?;
        let reason = parts.next().unwrap_or("").trim().to_string();
        let mut headers = Vec::new();
        for l in lines.filter(|l| !l.is_empty()) {
            let (k, v) = l.split_once(':').ok_or_else(|| HttpError::Protocol("bad header line".into()))?;
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
        Ok(Head { version: proto.to_string(), status, reason, headers })
    }
}

fn io_err(what: &str, e: std::io::Error) -> HttpError {
    HttpError::Io(format!("{what}: {e}"))
}

/// Reads an answer from a stream, keeping what it read ahead.
struct Reader<'s> {
    stream: &'s mut dyn Read,
    buf: Vec<u8>,
    eof: bool,
}

impl Reader<'_> {
    fn fill(&mut self) -> Result<usize, HttpError> {
        if self.eof {
            return Ok(0);
        }
        let mut chunk = [0u8; 8192];
        let n = self.stream.read(&mut chunk).map_err(|e| io_err("the connection broke", e))?;
        if n == 0 {
            self.eof = true;
        }
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(n)
    }

    fn head(&mut self) -> Result<Head, HttpError> {
        loop {
            if let Some(i) = self.buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let text = String::from_utf8_lossy(&self.buf[..i]).into_owned();
                self.buf.drain(..i + 4);
                let h = Head::parse(&text)?;
                // 100 Continue and friends: the real answer follows.
                if (100..200).contains(&h.status) {
                    continue;
                }
                return Ok(h);
            }
            if self.buf.len() > MAX_HEAD {
                return Err(HttpError::Protocol("the headers are too long".into()));
            }
            if self.fill()? == 0 {
                return Err(HttpError::Protocol(if self.buf.is_empty() { "the server closed the connection without answering".into() } else { "the headers are cut short".into() }));
            }
        }
    }

    /// One CRLF-terminated line (without the CRLF).
    fn line(&mut self) -> Result<String, HttpError> {
        loop {
            if let Some(i) = self.buf.windows(2).position(|w| w == b"\r\n") {
                let s = String::from_utf8_lossy(&self.buf[..i]).into_owned();
                self.buf.drain(..i + 2);
                return Ok(s);
            }
            if self.buf.len() > 1024 || self.fill()? == 0 {
                return Err(HttpError::Protocol("a chunked body is cut short".into()));
            }
        }
    }

    /// Up to `want` body bytes (fewer at the end of the stream).
    fn take(&mut self, want: usize) -> Result<Vec<u8>, HttpError> {
        if self.buf.is_empty() {
            self.fill()?;
        }
        let n = want.min(self.buf.len());
        Ok(self.buf.drain(..n).collect())
    }
}

struct Body<'a, 't> {
    out: Vec<u8>,
    got: u64,
    total: Option<u64>,
    to_sink: bool,
    transfer: &'a mut Transfer<'t>,
}

impl Body<'_, '_> {
    fn push(&mut self, data: &[u8]) -> Result<(), HttpError> {
        self.got += data.len() as u64;
        if self.got > self.transfer.max_body {
            return Err(HttpError::TooLarge(self.transfer.max_body));
        }
        match (&mut self.transfer.sink, self.to_sink) {
            (Some(s), true) => s.write_all(data).map_err(|e| io_err("cannot write the download", e))?,
            _ => self.out.extend_from_slice(data),
        }
        if let Some(p) = &mut self.transfer.progress {
            p(self.got, self.total);
        }
        Ok(())
    }
}

/// Read one answer from `stream`. `method` matters for HEAD (no body).
pub fn read_response(stream: &mut dyn Read, method: &str, transfer: &mut Transfer) -> Result<(Head, Vec<u8>, u64), HttpError> {
    let mut r = Reader { stream, buf: Vec::new(), eof: false };
    let head = r.head()?;
    let no_body = method == "HEAD" || head.status == 204 || head.status == 304;
    let chunked = head.header("Transfer-Encoding").map(|v| v.to_ascii_lowercase().contains("chunked")).unwrap_or(false);
    let length: Option<u64> = match head.header("Content-Length") {
        Some(v) => Some(v.trim().parse().map_err(|_| HttpError::Protocol("bad Content-Length".into()))?),
        None => None,
    };
    let total = if chunked { None } else { length };
    if let Some(t) = total {
        if t > transfer.max_body && !no_body {
            return Err(HttpError::TooLarge(transfer.max_body));
        }
    }
    let to_sink = (200..300).contains(&head.status);
    let mut body = Body { out: Vec::new(), got: 0, total, to_sink, transfer };
    if no_body {
        return Ok((head, Vec::new(), 0));
    }
    if let Some(p) = &mut body.transfer.progress {
        p(0, total);
    }
    if chunked {
        loop {
            let line = r.line()?;
            let size = usize::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16).map_err(|_| HttpError::Protocol("bad chunk size".into()))?;
            if size == 0 {
                break;
            }
            let mut left = size;
            while left > 0 {
                let part = r.take(left)?;
                if part.is_empty() {
                    return Err(HttpError::Protocol("a chunked body is cut short".into()));
                }
                left -= part.len();
                body.push(&part)?;
            }
            r.line()?;
        }
    } else if let Some(n) = length {
        let mut left = n;
        while left > 0 {
            let part = r.take(left.min(1 << 20) as usize)?;
            if part.is_empty() {
                return Err(HttpError::Protocol(format!("the body is cut short ({} of {n} bytes)", n - left)));
            }
            left -= part.len() as u64;
            body.push(&part)?;
        }
    } else {
        loop {
            let part = r.take(1 << 20)?;
            if part.is_empty() {
                break;
            }
            body.push(&part)?;
        }
    }
    Ok((head, body.out, body.got))
}

pub struct Client<C: Connector> {
    pub connector: C,
    /// The in-game authorities trusted; read from `/etc/ssl/certs` on first use when unset.
    pub trust: Option<Trust>,
    pub check: Check,
    pub user_agent: String,
    pub max_redirects: usize,
}

impl<C: Connector> Client<C> {
    pub fn new(connector: C) -> Client<C> {
        Client { connector, trust: None, check: Check::Full, user_agent: "ecm-http/1".into(), max_redirects: MAX_REDIRECTS }
    }

    pub fn agent(mut self, user_agent: &str) -> Client<C> {
        self.user_agent = user_agent.to_string();
        self
    }

    /// GET into memory.
    pub fn get(&mut self, url: &str) -> Result<Response, HttpError> {
        self.send(&Request::get(url)?, &mut Transfer::default())
    }

    /// One exchange, no redirects followed.
    pub fn exchange(&mut self, req: &Request, transfer: &mut Transfer) -> Result<Response, HttpError> {
        let (mut stream, peer) = self.connector.connect(&req.url.host, req.url.port)?;
        let bytes = req.encode(&self.user_agent);
        if !req.url.https {
            stream.write_all(&bytes).and_then(|_| stream.flush()).map_err(|e| io_err("the connection broke while sending", e))?;
            let (head, body, length) = read_response(&mut *stream, &req.method, transfer)?;
            return Ok(Response { url: req.url.clone(), version: head.version, status: head.status, reason: head.reason, headers: head.headers, body, length, tls: None, peer, redirects: 0 });
        }
        let check = self.check;
        let trust = self.trust.get_or_insert_with(|| Trust::load(ecm_tls::CERTS_DIR));
        let mut tls = trust.connect(stream, &req.url.host, peer, check).map_err(HttpError::Tls)?;
        tls.write_all(&bytes).and_then(|_| tls.flush()).map_err(|e| io_err("the connection broke while sending", e))?;
        let version = tls.version();
        let (head, body, length) = read_response(&mut tls, &req.method, transfer)?;
        tls.close();
        Ok(Response { url: req.url.clone(), version: head.version, status: head.status, reason: head.reason, headers: head.headers, body, length, tls: Some(version), peer, redirects: 0 })
    }

    /// Send a request, following redirects. A redirect to another host drops
    /// `Authorization`; 303 (and 301/302 for POST) turn into GET.
    pub fn send(&mut self, req: &Request, transfer: &mut Transfer) -> Result<Response, HttpError> {
        let mut req = req.clone();
        for hop in 0..=self.max_redirects {
            let mut r = self.exchange(&req, transfer)?;
            r.redirects = hop;
            let location = match r.status {
                301 | 302 | 303 | 307 | 308 => r.header("Location").map(str::to_string),
                _ => None,
            };
            let Some(location) = location else { return Ok(r) };
            let next = req.url.join(&location).map_err(HttpError::BadUrl)?;
            if next.host != req.url.host || (req.url.https && !next.https) {
                req.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("Authorization") && !k.eq_ignore_ascii_case("Cookie"));
            }
            if r.status == 303 || (matches!(r.status, 301 | 302) && req.method == "POST") {
                req.method = "GET".into();
                req.body.clear();
                req.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("Content-Type"));
            }
            req.url = next;
        }
        Err(HttpError::TooManyRedirects)
    }
}

#[cfg(test)]
mod tests;
