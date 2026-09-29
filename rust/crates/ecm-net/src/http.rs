//! HTTP/1.0 request parser and response builder (pure helpers; no I/O).

/// Split raw bytes into (header text, body start offset). Only the header must be UTF-8.
fn split_head(data: &[u8]) -> Option<(&str, usize)> {
    let end = data.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = core::str::from_utf8(data.get(..end)?).ok()?;
    Some((head, end + 4))
}

fn parse_headers<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<(String, String)> {
    lines
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

/// Parsed HTTP request.
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub version: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// Parse an HTTP request from raw bytes (headers must be complete).
    pub fn parse(data: &[u8]) -> Option<Self> {
        let (head, body_start) = split_head(data)?;
        let mut lines = head.split("\r\n");
        let mut parts = lines.next()?.splitn(3, ' ');
        let method = parts.next()?.to_string();
        let path = parts.next()?.to_string();
        let version = parts.next().unwrap_or("HTTP/1.0").to_string();
        Some(HttpRequest {
            method,
            path,
            version,
            headers: parse_headers(lines),
            body: data.get(body_start..).unwrap_or(&[]).to_vec(),
        })
    }

    /// Get a header value by name (case-insensitive).
    pub fn get_header(&self, name: &str) -> Option<&str> {
        find_header(&self.headers, name)
    }

    /// Serialize an HTTP request to wire format.
    pub fn serialize(method: &str, host: &str, port: u16, path: &str, body: Option<&[u8]>, extra_headers: &[(&str, &str)]) -> Vec<u8> {
        let mut req = format!("{} {} HTTP/1.0\r\n", method, path);
        if port == 80 {
            req.push_str(&format!("Host: {}\r\n", host));
        } else {
            req.push_str(&format!("Host: {}:{}\r\n", host, port));
        }
        req.push_str("Connection: close\r\n");
        if let Some(b) = body {
            req.push_str(&format!("Content-Length: {}\r\n", b.len()));
        }
        for (k, v) in extra_headers {
            req.push_str(&format!("{}: {}\r\n", k, v));
        }
        req.push_str("\r\n");
        let mut result = req.into_bytes();
        if let Some(b) = body {
            result.extend_from_slice(b);
        }
        result
    }
}

/// HTTP response.
pub struct HttpResponse {
    pub status_code: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(status: u16, text: &str) -> Self {
        HttpResponse { status_code: status, status_text: text.to_string(), headers: Vec::new(), body: Vec::new() }
    }

    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers.push((key.to_string(), value.to_string()));
        self
    }

    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    pub fn ok(body: &str, content_type: &str) -> Self {
        let body_bytes = body.as_bytes().to_vec();
        HttpResponse::new(200, "OK")
            .with_header("Content-Type", content_type)
            .with_header("Content-Length", &format!("{}", body_bytes.len()))
            .with_body(body_bytes)
    }

    pub fn not_found() -> Self {
        let body = b"404 Not Found".to_vec();
        HttpResponse::new(404, "Not Found")
            .with_header("Content-Type", "text/plain")
            .with_header("Content-Length", &format!("{}", body.len()))
            .with_body(body)
    }

    pub fn bad_request(msg: &str) -> Self {
        let body = msg.as_bytes().to_vec();
        HttpResponse::new(400, "Bad Request")
            .with_header("Content-Type", "text/plain")
            .with_header("Content-Length", &format!("{}", body.len()))
            .with_body(body)
    }

    /// Serialize to wire format.
    pub fn serialize(&self) -> Vec<u8> {
        let mut resp = format!("HTTP/1.0 {} {}\r\n", self.status_code, self.status_text);
        resp.push_str("Server: TerminalOS/1.0\r\n");
        for (k, v) in &self.headers {
            resp.push_str(&format!("{}: {}\r\n", k, v));
        }
        if find_header(&self.headers, "content-length").is_none() {
            resp.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
        }
        resp.push_str("\r\n");
        let mut result = resp.into_bytes();
        result.extend_from_slice(&self.body);
        result
    }

    /// Parse an HTTP response from raw bytes (headers must be complete; body may be binary).
    pub fn parse(data: &[u8]) -> Option<Self> {
        let (head, body_start) = split_head(data)?;
        let mut lines = head.split("\r\n");
        let mut parts = lines.next()?.splitn(3, ' ');
        let _version = parts.next()?;
        let status_code: u16 = parts.next()?.parse().ok()?;
        let status_text = parts.next().unwrap_or("").to_string();
        Some(HttpResponse {
            status_code,
            status_text,
            headers: parse_headers(lines),
            body: data.get(body_start..).unwrap_or(&[]).to_vec(),
        })
    }

    /// Get a header value by name (case-insensitive).
    pub fn get_header(&self, name: &str) -> Option<&str> {
        find_header(&self.headers, name)
    }
}

/// Parse a URL into (host, port, path).
/// Supports: "http://host:port/path", "http://host/path", "host:port/path", "host/path"
pub fn parse_url(url: &str) -> Option<(String, u16, String)> {
    let url = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = match url.find('/') {
        Some(i) => (url.get(..i)?, url.get(i..)?),
        None => (url, "/"),
    };
    let (host, port) = match host_port.split_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok()?),
        None => (host_port, 80u16),
    };
    Some((host.to_string(), port, path.to_string()))
}

/// If `data` holds a complete response (headers + `Content-Length` bytes of body), parse it.
/// Returns `None` if more data is needed or there is no Content-Length (read until EOF).
pub fn parse_complete_response(data: &[u8]) -> Option<HttpResponse> {
    let (head, body_start) = split_head(data)?;
    let cl: usize = head
        .split("\r\n")
        .skip(1)
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse().ok())?;
    if data.len().saturating_sub(body_start) >= cl {
        HttpResponse::parse(data.get(..body_start.checked_add(cl)?)?)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let raw = HttpRequest::serialize("POST", "example.com", 8080, "/x", Some(b"\xff\x00bin"), &[("X-A", "1")]);
        let r = HttpRequest::parse(&raw).unwrap();
        assert_eq!((r.method.as_str(), r.path.as_str()), ("POST", "/x"));
        assert_eq!(r.get_header("host"), Some("example.com:8080"));
        assert_eq!(r.get_header("x-a"), Some("1"));
        assert_eq!(r.body, b"\xff\x00bin");
    }

    #[test]
    fn response_roundtrip_and_complete() {
        let raw = HttpResponse::ok("hello", "text/plain").serialize();
        let r = HttpResponse::parse(&raw).unwrap();
        assert_eq!(r.status_code, 200);
        assert_eq!(r.body, b"hello");
        assert!(parse_complete_response(&raw[..raw.len() - 1]).is_none());
        assert_eq!(parse_complete_response(&raw).unwrap().body, b"hello");
    }

    #[test]
    fn urls() {
        assert_eq!(parse_url("http://a.b:81/c"), Some(("a.b".into(), 81, "/c".into())));
        assert_eq!(parse_url("a.b"), Some(("a.b".into(), 80, "/".into())));
        assert_eq!(parse_url("a.b:x/"), None);
        assert_eq!(parse_url("a.b:99999/"), None);
    }

    #[test]
    fn truncated_and_garbage() {
        let raw = HttpResponse::ok("hello", "text/plain").serialize();
        for n in 0..raw.len() {
            let _ = HttpResponse::parse(&raw[..n]);
            let _ = HttpRequest::parse(&raw[..n]);
            let _ = parse_complete_response(&raw[..n]);
        }
        assert!(HttpResponse::parse(b"\xff\xfe\r\n\r\n").is_none());
        assert!(HttpResponse::parse(b"HTTP/1.0 abc\r\n\r\n").is_none());
        assert!(parse_complete_response(b"HTTP/1.0 200 OK\r\nContent-Length: 18446744073709551615\r\n\r\n").is_none());
        let _ = parse_url("\u{e9}:\u{e9}/");
    }
}
