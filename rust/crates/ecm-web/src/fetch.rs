//! Fetching: `http://` as HTTP/1.0 with `Host:` (Content-Length, chunked
//! bodies, redirects 301/302/303/307/308 up to five, form posts, per-host
//! session cookies) and `file://`, over a [`Transport`]: the computer's
//! sockets and disk in the program, a scripted one in tests.

use crate::url::{Scheme, Url};
use std::collections::HashMap;

/// At most this many redirects are followed.
pub const MAX_REDIRECTS: usize = 5;
/// Responses are cut off at this size.
pub const MAX_BODY: usize = 4 << 20;
pub const USER_AGENT: &str = "ECM-Browser/1.0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    /// The host name doesn't resolve: the name and why ("name not found", "no answer from 100.67.0.53").
    Resolve(String, String),
    /// Nobody answered at the address (refused, unreachable or timed out).
    Connect(String),
    /// This computer has no network to send on.
    NoNetwork,
    /// The app isn't allowed to use the network (or files).
    NotAllowed,
    /// The answer wasn't HTTP.
    Protocol(String),
    TooManyRedirects,
    /// A file that isn't there.
    NoFile(String),
    /// Other trouble reading or sending.
    Io(String),
    Unsupported(String),
}

impl FetchError {
    /// A title and an explanation for the error page.
    pub fn explain(&self) -> (String, String) {
        match self {
            FetchError::Resolve(h, why) => (
                "Can't find the server".into(),
                format!("No address for {h}: {why}. Check the name, or this computer's DNS settings (/etc/resolv.conf)."),
            ),
            FetchError::Connect(a) => (
                "Can't reach this page".into(),
                format!("{a} didn't answer. The server may be down, the address wrong, or there is no route to it (try ping)."),
            ),
            FetchError::NoNetwork => ("No network".into(), "This computer isn't connected to a network. Plug in a cable, join Wi-Fi or get a cellular signal.".into()),
            FetchError::NotAllowed => ("Not allowed".into(), "The Browser isn't allowed to use the network. Allow it in Settings > Apps > Browser.".into()),
            FetchError::Protocol(m) => ("Bad answer".into(), format!("The server's answer isn't HTTP: {m}")),
            FetchError::TooManyRedirects => ("Too many redirects".into(), format!("The server redirected more than {MAX_REDIRECTS} times.")),
            FetchError::NoFile(p) => ("File not found".into(), format!("There is no file {p} on this computer.")),
            FetchError::Io(m) => ("Can't load the page".into(), m.clone()),
            FetchError::Unsupported(s) => ("Can't open this address".into(), format!("The Browser opens http:// and file:// addresses, not {s}.")),
        }
    }
}

/// The I/O a fetch needs.
pub trait Transport {
    /// Send `request` to `host:port` over TCP and return everything the
    /// server sends back until it closes the connection.
    fn exchange(&mut self, host: &str, port: u16, request: &[u8]) -> Result<Vec<u8>, FetchError>;
    /// The bytes of a local file (a directory: its `index.html`).
    fn read_file(&mut self, path: &str) -> Result<Vec<u8>, FetchError>;
    /// A folder's entries: (name, is a folder), sorted; None if it isn't one.
    fn list_dir(&mut self, _path: &str) -> Option<Vec<(String, bool)>> {
        None
    }
    /// Write a file (downloads), making its folder. `Err` says why.
    fn save_file(&mut self, path: &str, _data: &[u8]) -> Result<(), FetchError> {
        Err(FetchError::Io(format!("can't save {path} here")))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    /// The final address (after redirects).
    pub url: Url,
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// How many redirects led here.
    pub redirects: usize,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The MIME type, lower case, without parameters (guessed when missing).
    pub fn content_type(&self) -> String {
        match self.header("content-type") {
            Some(ct) => ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase(),
            None => guess_type(&self.url.path, &self.body).to_string(),
        }
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn is_html(&self) -> bool {
        let ct = self.content_type();
        ct == "text/html" || ct == "application/xhtml+xml"
    }
}

/// A MIME type from a file name, or a look at the bytes.
pub fn guess_type(path: &str, body: &[u8]) -> &'static str {
    let lower = path.to_ascii_lowercase();
    let ext = lower.rsplit('/').next().and_then(|n| n.rsplit_once('.')).map(|(_, e)| e).unwrap_or("");
    match ext {
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "text/javascript",
        "json" => "application/json",
        "png" => "image/png",
        "txt" | "md" | "py" | "lua" | "toml" | "cfg" | "conf" | "log" | "csv" | "sh" | "rs" => "text/plain",
        "ecmpkg" => "application/x-ecmpkg",
        _ => {
            if body.starts_with(b"\x89PNG") {
                "image/png"
            } else {
                let head = String::from_utf8_lossy(&body[..body.len().min(256)]).trim_start().to_ascii_lowercase();
                if head.starts_with('<') {
                    "text/html"
                } else if body.iter().take(512).any(|b| *b == 0) {
                    "application/octet-stream"
                } else {
                    "text/plain"
                }
            }
        }
    }
}

/// Session cookies per host (no expiry beyond the session; `Max-Age=0` deletes).
#[derive(Clone, Debug, Default)]
pub struct Cookies {
    jar: HashMap<String, Vec<(String, String)>>,
}

impl Cookies {
    pub fn header(&self, host: &str) -> Option<String> {
        let c = self.jar.get(host)?;
        if c.is_empty() {
            return None;
        }
        Some(c.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; "))
    }

    /// Take a `Set-Cookie:` header value from `host`.
    pub fn set(&mut self, host: &str, value: &str) {
        let mut parts = value.split(';');
        let Some((name, val)) = parts.next().and_then(|p| p.split_once('=')) else { return };
        let (name, val) = (name.trim().to_string(), val.trim().to_string());
        if name.is_empty() {
            return;
        }
        let delete = parts.any(|a| {
            let a = a.trim().to_ascii_lowercase();
            a.strip_prefix("max-age=").is_some_and(|v| v.trim().parse::<i64>().is_ok_and(|n| n <= 0))
        });
        let list = self.jar.entry(host.to_string()).or_default();
        list.retain(|(k, _)| *k != name);
        if !delete {
            list.push((name, val));
        }
    }

    pub fn get(&self, host: &str, name: &str) -> Option<&str> {
        self.jar.get(host)?.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn clear(&mut self) {
        self.jar.clear();
    }
}

/// The request bytes.
pub fn build_request(method: &str, url: &Url, body: Option<(&str, &[u8])>, cookie: Option<&str>) -> Vec<u8> {
    let mut req = format!("{method} {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: {USER_AGENT}\r\nAccept: text/html, */*\r\nConnection: close\r\n", url.request_target(), url.authority());
    if let Some(c) = cookie {
        req.push_str(&format!("Cookie: {c}\r\n"));
    }
    if let Some((ct, b)) = body {
        req.push_str(&format!("Content-Type: {ct}\r\nContent-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    let mut out = req.into_bytes();
    if let Some((_, b)) = body {
        out.extend_from_slice(b);
    }
    out
}

/// Status, reason, headers and body of a raw response.
pub fn parse_response(raw: &[u8]) -> Result<(u16, String, Vec<(String, String)>, Vec<u8>), FetchError> {
    if raw.is_empty() {
        return Err(FetchError::Protocol("the server closed the connection without answering".into()));
    }
    if !raw.starts_with(b"HTTP/") {
        // HTTP/0.9 style: the whole answer is the body.
        return Ok((200, "OK".into(), Vec::new(), raw.to_vec()));
    }
    let (head_end, body_start) = match raw.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(p) => (p, p + 4),
        None => match raw.windows(2).position(|w| w == b"\n\n") {
            Some(p) => (p, p + 2),
            None => (raw.len(), raw.len()),
        },
    };
    let head = String::from_utf8_lossy(&raw[..head_end]);
    let mut lines = head.lines();
    let status_line = lines.next().unwrap_or("");
    let mut sp = status_line.splitn(3, ' ');
    let _version = sp.next();
    let status: u16 = sp.next().and_then(|s| s.trim().parse().ok()).ok_or_else(|| FetchError::Protocol(format!("bad status line {status_line:?}")))?;
    let reason = sp.next().unwrap_or("").trim().to_string();
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    let mut body = raw[body_start..].to_vec();
    let h = |n: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.clone());
    if h("transfer-encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked")) {
        body = dechunk(&body);
    } else if let Some(n) = h("content-length").and_then(|v| v.parse::<usize>().ok()) {
        body.truncate(n);
    }
    Ok((status, reason, headers, body))
}

fn dechunk(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let Some(nl) = b[i..].windows(2).position(|w| w == b"\r\n") else { break };
        let size_line = String::from_utf8_lossy(&b[i..i + nl]);
        let Ok(n) = usize::from_str_radix(size_line.split(';').next().unwrap_or("").trim(), 16) else { break };
        i += nl + 2;
        if n == 0 {
            break;
        }
        let end = (i + n).min(b.len());
        out.extend_from_slice(&b[i..end]);
        i = end + 2;
    }
    out
}

/// A fetcher: a transport and the cookie jar.
pub struct Fetcher<T: Transport> {
    pub transport: T,
    pub cookies: Cookies,
}

impl<T: Transport> Fetcher<T> {
    pub fn new(transport: T) -> Self {
        Fetcher { transport, cookies: Cookies::default() }
    }

    pub fn get(&mut self, url: &Url) -> Result<Response, FetchError> {
        self.request("GET", url, None)
    }

    /// Post a form (`application/x-www-form-urlencoded` body).
    pub fn post_form(&mut self, url: &Url, body: &str) -> Result<Response, FetchError> {
        self.request("POST", url, Some(("application/x-www-form-urlencoded", body.as_bytes())))
    }

    pub fn request(&mut self, method: &str, url: &Url, body: Option<(&str, &[u8])>) -> Result<Response, FetchError> {
        let mut url = url.without_fragment();
        let mut method = method.to_string();
        let mut body: Option<(String, Vec<u8>)> = body.map(|(c, b)| (c.to_string(), b.to_vec()));
        for redirects in 0..=MAX_REDIRECTS {
            match url.scheme {
                Scheme::File => {
                    let path = url.file_path();
                    let data = self.transport.read_file(&path)?;
                    let ct = guess_type(&path, &data).to_string();
                    return Ok(Response { url, status: 200, reason: "OK".into(), headers: vec![("Content-Type".into(), ct)], body: data, redirects });
                }
                Scheme::About => return Err(FetchError::Unsupported(url.to_string())),
                Scheme::Http => {}
            }
            let cookie = self.cookies.header(&url.host);
            let req = build_request(&method, &url, body.as_ref().map(|(c, b)| (c.as_str(), b.as_slice())), cookie.as_deref());
            let mut raw = self.transport.exchange(&url.host, url.port, &req)?;
            raw.truncate(MAX_BODY);
            let (status, reason, headers, data) = parse_response(&raw)?;
            for (k, v) in &headers {
                if k.eq_ignore_ascii_case("set-cookie") {
                    self.cookies.set(&url.host, v);
                }
            }
            let location = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("location")).map(|(_, v)| v.clone());
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                if let Some(next) = location.and_then(|l| url.join(&l)) {
                    if redirects == MAX_REDIRECTS {
                        return Err(FetchError::TooManyRedirects);
                    }
                    if !matches!(status, 307 | 308) {
                        method = "GET".into();
                        body = None;
                    }
                    url = next.without_fragment();
                    continue;
                }
            }
            return Ok(Response { url, status, reason, headers, body: data, redirects });
        }
        Err(FetchError::TooManyRedirects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers by `host:port path`, records requests.
    #[derive(Default)]
    struct Script {
        answers: Vec<(String, Vec<u8>)>,
        sent: Vec<String>,
        files: Vec<(String, Vec<u8>)>,
    }

    impl Transport for Script {
        fn exchange(&mut self, host: &str, port: u16, request: &[u8]) -> Result<Vec<u8>, FetchError> {
            let req = String::from_utf8_lossy(request).into_owned();
            let path = req.split(' ').nth(1).unwrap_or("").to_string();
            self.sent.push(req);
            let key = format!("{host}:{port} {path}");
            self.answers.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()).ok_or(FetchError::Connect(format!("{host}:{port}")))
        }
        fn read_file(&mut self, path: &str) -> Result<Vec<u8>, FetchError> {
            self.files.iter().find(|(p, _)| p == path).map(|(_, b)| b.clone()).ok_or(FetchError::NoFile(path.into()))
        }
    }

    fn ok(body: &str) -> Vec<u8> {
        format!("HTTP/1.0 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[test]
    fn get_with_host_header() {
        let mut s = Script::default();
        s.answers.push(("web.v4.ring:8080 /a?b=1".into(), ok("<h1>hi</h1>trailing junk is cut")));
        let mut f = Fetcher::new(s);
        let r = f.get(&Url::parse("http://web.v4.ring:8080/a?b=1#frag").unwrap()).unwrap();
        assert_eq!(r.status, 200);
        assert!(r.is_html());
        assert_eq!(r.text(), "<h1>hi</h1>trailing junk is cut");
        let req = &f.transport.sent[0];
        assert!(req.starts_with("GET /a?b=1 HTTP/1.0\r\nHost: web.v4.ring:8080\r\n"), "{req}");
        assert!(req.ends_with("\r\n\r\n"));
    }

    #[test]
    fn redirects_and_cookies() {
        let mut s = Script::default();
        s.answers.push(("h:80 /form".into(), b"HTTP/1.0 303 See Other\r\nLocation: /done\r\nSet-Cookie: sid=42; Path=/\r\n\r\n".to_vec()));
        s.answers.push(("h:80 /done".into(), ok("thanks")));
        let mut f = Fetcher::new(s);
        let r = f.post_form(&Url::parse("http://h/form").unwrap(), "name=a+b").unwrap();
        assert_eq!(r.url.to_string(), "http://h/done");
        assert_eq!(r.redirects, 1);
        let sent = &f.transport.sent;
        assert!(sent[0].starts_with("POST /form") && sent[0].ends_with("name=a+b") && sent[0].contains("Content-Length: 8"));
        assert!(sent[1].starts_with("GET /done") && sent[1].contains("Cookie: sid=42\r\n"), "{}", sent[1]);
        assert_eq!(f.cookies.get("h", "sid"), Some("42"));
        assert!(f.cookies.header("other").is_none(), "cookies stay with their host");
        f.cookies.set("h", "sid=; Max-Age=0");
        assert!(f.cookies.get("h", "sid").is_none());
    }

    #[test]
    fn redirect_loop_stops() {
        let mut s = Script::default();
        s.answers.push(("h:80 /".into(), b"HTTP/1.0 302 Found\r\nLocation: http://h/\r\n\r\n".to_vec()));
        let mut f = Fetcher::new(s);
        assert_eq!(f.get(&Url::parse("http://h/").unwrap()), Err(FetchError::TooManyRedirects));
        assert_eq!(f.transport.sent.len(), MAX_REDIRECTS + 1);
    }

    #[test]
    fn errors_and_odd_answers() {
        let mut s = Script::default();
        s.answers.push(("h:80 /404".into(), b"HTTP/1.0 404 Not Found\r\n\r\n404 Not Found".to_vec()));
        s.answers.push(("h:80 /old".into(), b"<p>no headers at all".to_vec()));
        s.answers.push(("h:80 /chunked".into(), b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n".to_vec()));
        s.answers.push(("h:80 /empty".into(), Vec::new()));
        s.answers.push(("h:80 /bad".into(), b"HTTP/1.0 abc\r\n\r\n".to_vec()));
        let mut f = Fetcher::new(s);
        let g = |f: &mut Fetcher<Script>, p: &str| f.get(&Url::parse(&format!("http://h{p}")).unwrap());
        let r = g(&mut f, "/404").unwrap();
        assert_eq!((r.status, r.text().as_str()), (404, "404 Not Found"));
        assert_eq!(g(&mut f, "/old").unwrap().text(), "<p>no headers at all");
        assert!(g(&mut f, "/old").unwrap().is_html());
        assert_eq!(g(&mut f, "/chunked").unwrap().text(), "hello world");
        assert!(matches!(g(&mut f, "/empty"), Err(FetchError::Protocol(_))));
        assert!(matches!(g(&mut f, "/bad"), Err(FetchError::Protocol(_))));
        assert!(matches!(g(&mut f, "/nothing"), Err(FetchError::Connect(_))));
    }

    #[test]
    fn files() {
        let mut s = Script::default();
        s.files.push(("/home/my site/index.html".into(), b"<h1>local</h1>".to_vec()));
        s.files.push(("/home/my site/style.css".into(), b"h1{}".to_vec()));
        let mut f = Fetcher::new(s);
        let r = f.get(&Url::parse("file:///home/my%20site/index.html").unwrap()).unwrap();
        assert!(r.is_html());
        assert_eq!(f.get(&Url::parse("file:///home/my%20site/style.css").unwrap()).unwrap().content_type(), "text/css");
        assert_eq!(f.get(&Url::parse("file:///nope.html").unwrap()), Err(FetchError::NoFile("/nope.html".into())));
        assert!(f.transport.sent.is_empty());
    }
}
