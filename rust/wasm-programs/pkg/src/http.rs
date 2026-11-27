//! Just enough HTTP/1.0 for repositories: URLs, GET requests and responses, and
//! the request line a tiny server needs. Pure; sockets live in the binary.

/// How a repository is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// Plain HTTP over the computer's network (a repository in the game: `pkg serve`).
    Http,
    /// HTTPS over the computer's network.
    Https,
    /// The server's own link to the official repository, when the server
    /// offers it to this computer: no network needed. `host://repo/r/stable/`.
    Host,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Url {
    pub scheme: Scheme,
    pub host: String,
    pub port: u16,
    /// Base path, always ending in '/'.
    pub path: String,
}

/// The only host a `host://` address may name.
pub const HOST_REPO: &str = "repo";

impl Url {
    pub fn parse(s: &str) -> Result<Url, String> {
        let (scheme, rest, default_port) = if let Some(r) = s.strip_prefix("http://") {
            (Scheme::Http, r, 80)
        } else if let Some(r) = s.strip_prefix("https://") {
            (Scheme::Https, r, 443)
        } else if let Some(r) = s.strip_prefix("host://") {
            (Scheme::Host, r, 0)
        } else {
            return Err(format!("{}: a repository address starts with http://, https:// or host://", s));
        };
        let (hp, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (host, port) = match hp.rfind(':') {
            Some(i) => (&hp[..i], hp[i + 1..].parse::<u16>().map_err(|_| format!("{}: bad port", s))?),
            None => (hp, default_port),
        };
        if host.is_empty() || !host.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-') {
            return Err(format!("{}: bad host", s));
        }
        if scheme == Scheme::Host && (host != HOST_REPO || port != 0) {
            return Err(format!("{}: the only host:// address is host://{}/...", s, HOST_REPO));
        }
        if path.bytes().any(|c| c <= b' ' || c == 0x7F || c == b'?' || c == b'#') {
            return Err(format!("{}: bad path", s));
        }
        let mut path = path.to_string();
        if !path.ends_with('/') {
            path.push('/');
        }
        Ok(Url { scheme, host: host.to_ascii_lowercase(), port, path })
    }

    /// A file in this directory.
    pub fn join(&self, file: &str) -> String {
        format!("{}{}", self.path, file)
    }

    /// `scheme://host[:port]`, without a path: where the repository's API lives.
    pub fn origin(&self) -> String {
        match self.scheme {
            Scheme::Http if self.port == 80 => format!("http://{}", self.host),
            Scheme::Http => format!("http://{}:{}", self.host, self.port),
            Scheme::Https if self.port == 443 => format!("https://{}", self.host),
            Scheme::Https => format!("https://{}:{}", self.host, self.port),
            Scheme::Host => format!("host://{}", self.host),
        }
    }

    pub fn display(&self) -> String {
        format!("{}{}", self.origin(), self.path)
    }

    /// Bytes travel encrypted, or never leave the server (signing in needs one of the two).
    pub fn private(&self) -> bool {
        self.scheme != Scheme::Http
    }
}

pub fn get_request(host: &str, path: &str) -> Vec<u8> {
    format!("GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: ecm-pkg/1\r\nConnection: close\r\n\r\n", path, host).into_bytes()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

fn find_header_end(b: &[u8]) -> Option<usize> {
    b.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Parse a whole response (read until the server closed). With a Content-Length the
/// body must be complete.
pub fn parse_response(raw: &[u8]) -> Result<Response, String> {
    let end = find_header_end(raw).ok_or("no HTTP header in the reply")?;
    let head = std::str::from_utf8(&raw[..end]).map_err(|_| "bad HTTP header")?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let mut parts = status_line.split_whitespace();
    let proto = parts.next().unwrap_or("");
    if !proto.starts_with("HTTP/") {
        return Err(format!("not an HTTP reply: {}", status_line));
    }
    let status: u16 = parts.next().and_then(|s| s.parse().ok()).ok_or("bad HTTP status")?;
    let mut len: Option<usize> = None;
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().ok();
            }
        }
    }
    let body = &raw[end + 4..];
    let body = match len {
        Some(n) if body.len() < n => return Err(format!("reply cut short: {} of {} bytes", body.len(), n)),
        Some(n) => &body[..n],
        None => body,
    };
    Ok(Response { status, body: body.to_vec() })
}

/// True once `raw` holds a complete response (header and, with a Content-Length,
/// the whole body), so a client can stop reading without waiting for the close.
pub fn response_complete(raw: &[u8]) -> bool {
    let Some(end) = find_header_end(raw) else { return false };
    let head = String::from_utf8_lossy(&raw[..end]);
    for l in head.split("\r\n") {
        if let Some((k, v)) = l.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                if let Ok(n) = v.trim().parse::<usize>() {
                    return raw.len() >= end + 4 + n;
                }
            }
        }
    }
    false
}

/// The Content-Length of a reply once its header is complete (download progress).
pub fn content_length(raw: &[u8]) -> Option<usize> {
    let end = find_header_end(raw)?;
    let head = String::from_utf8_lossy(&raw[..end]);
    head.split("\r\n").find_map(|l| {
        let (k, v) = l.split_once(':')?;
        if k.trim().eq_ignore_ascii_case("content-length") {
            v.trim().parse().ok()
        } else {
            None
        }
    })
}

/// Bytes of the body received so far (0 until the header is complete).
pub fn body_received(raw: &[u8]) -> usize {
    find_header_end(raw).map_or(0, |e| raw.len().saturating_sub(e + 4))
}

/// (method, path) of a request once its header is complete; None while incomplete.
pub fn parse_request_line(raw: &[u8]) -> Option<(String, String)> {
    find_header_end(raw)?;
    let text = String::from_utf8_lossy(raw);
    let first = text.lines().next()?;
    let mut p = first.split_whitespace();
    Some((p.next()?.to_string(), p.next()?.to_string()))
}

pub fn response(status: u16, reason: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.0 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status,
        reason,
        content_type,
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        let u = Url::parse("http://10.0.7.1:8080/repo").unwrap();
        assert_eq!((u.scheme, u.host.as_str(), u.port, u.path.as_str()), (Scheme::Http, "10.0.7.1", 8080, "/repo/"));
        assert_eq!(u.join("index.json"), "/repo/index.json");
        assert_eq!(Url::parse("http://apps.ecm").unwrap().display(), "http://apps.ecm/");
        let s = Url::parse("https://Repo.EvanWitulski.com/r/stable").unwrap();
        assert_eq!((s.scheme, s.port, s.display().as_str()), (Scheme::Https, 443, "https://repo.evanwitulski.com/r/stable/"));
        assert_eq!(s.origin(), "https://repo.evanwitulski.com");
        assert!(s.private() && !u.private());
        assert_eq!(Url::parse("https://h:8443/").unwrap().origin(), "https://h:8443");
        let h = Url::parse("host://repo/r/community/").unwrap();
        assert_eq!((h.scheme, h.display().as_str(), h.origin().as_str()), (Scheme::Host, "host://repo/r/community/", "host://repo"));
        for bad in ["ftp://x/", "http://a b/", "host://elsewhere/r/", "host://repo:80/", "http://h/a?b", "x"] {
            assert!(Url::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn responses() {
        let raw = response(200, "OK", "application/json", b"{\"a\":1}");
        assert!(response_complete(&raw));
        assert!(!response_complete(&raw[..raw.len() - 1]));
        let r = parse_response(&raw).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"{\"a\":1}");
        assert!(parse_response(&raw[..raw.len() - 2]).is_err());
        assert_eq!(parse_response(b"HTTP/1.0 404 Not Found\r\n\r\nnope").unwrap().status, 404);
        assert_eq!(parse_request_line(b"GET /index.json HTTP/1.0\r\nHost: x\r\n\r\n"), Some(("GET".into(), "/index.json".into())));
        assert_eq!(parse_request_line(b"GET /index.json HTTP/1.0\r\n"), None);
        assert_eq!(content_length(&raw), Some(7));
        assert_eq!(body_received(&raw[..raw.len() - 3]), 4);
        assert_eq!(content_length(b"HTTP/1.0 200 OK\r\n"), None);
    }
}
