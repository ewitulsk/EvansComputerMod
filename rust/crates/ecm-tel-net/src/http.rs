//! Just enough HTTP/1.0 for the MMSC and visual voicemail: requests and
//! responses with binary bodies (Content-Length), parsed incrementally.

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub method: String,
    /// Path with the query (`/v/abc?r=...`).
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Largest request or response accepted (an MMS is at most 600 KB by default).
pub const MAX_MESSAGE: usize = 2 * 1024 * 1024;

fn head_end(b: &[u8]) -> Option<usize> {
    b.windows(4).position(|w| w == b"\r\n\r\n")
}

fn headers(lines: &mut std::str::Lines) -> Vec<(String, String)> {
    lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect()
}

fn content_length(h: &[(String, String)]) -> usize {
    h.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0)
}

/// A whole request from the start of `b`: `Ok(Some((request, bytes used)))`, `Ok(None)` for more bytes, `Err` for garbage.
pub fn parse_request(b: &[u8]) -> Result<Option<(Request, usize)>, String> {
    if b.len() > MAX_MESSAGE {
        return Err("too large".into());
    }
    let Some(end) = head_end(b) else {
        return if b.len() > 16 * 1024 { Err("header too long".into()) } else { Ok(None) };
    };
    let head = std::str::from_utf8(&b[..end]).map_err(|_| "header not UTF-8")?;
    let mut lines = head.lines();
    let first = lines.next().unwrap_or("");
    let mut w = first.split_whitespace();
    let (Some(method), Some(target), Some(ver)) = (w.next(), w.next(), w.next()) else {
        return Err("bad request line".into());
    };
    if !ver.starts_with("HTTP/1.") {
        return Err("not HTTP/1.x".into());
    }
    let hs = headers(&mut lines);
    let len = content_length(&hs);
    if len > MAX_MESSAGE {
        return Err("body too large".into());
    }
    if b.len() < end + 4 + len {
        return Ok(None);
    }
    let body = b[end + 4..end + 4 + len].to_vec();
    Ok(Some((Request { method: method.to_string(), target: target.to_string(), headers: hs, body }, end + 4 + len)))
}

/// A whole response (a client reads until the connection closes or the body is complete).
pub fn parse_response(b: &[u8], closed: bool) -> Result<Option<Response>, String> {
    let Some(end) = head_end(b) else {
        return if closed { Err("connection closed before the answer".into()) } else { Ok(None) };
    };
    let head = std::str::from_utf8(&b[..end]).map_err(|_| "header not UTF-8")?;
    let mut lines = head.lines();
    let first = lines.next().unwrap_or("");
    let status: u16 = first.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or("bad status line")?;
    let hs = headers(&mut lines);
    let has_len = hs.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-length"));
    let len = content_length(&hs);
    let body_have = b.len() - end - 4;
    if has_len && body_have < len {
        return if closed { Err("connection closed in the body".into()) } else { Ok(None) };
    }
    if !has_len && !closed {
        return Ok(None);
    }
    let body = if has_len { b[end + 4..end + 4 + len].to_vec() } else { b[end + 4..].to_vec() };
    Ok(Some(Response { status, headers: hs, body }))
}

pub fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

pub fn encode_response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut v = format!("HTTP/1.0 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", status, reason(status), content_type, body.len()).into_bytes();
    v.extend_from_slice(body);
    v
}

pub fn encode_request(method: &str, host: &str, target: &str, content_type: Option<&str>, body: &[u8]) -> Vec<u8> {
    let mut h = format!("{} {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: ECM-telephonyd/1\r\nConnection: close\r\n", method, target, host);
    if let Some(ct) = content_type {
        h.push_str(&format!("Content-Type: {}\r\n", ct));
    }
    h.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    let mut v = h.into_bytes();
    v.extend_from_slice(body);
    v
}

/// `http://host[:port]/path` -> (host, port, path).
pub fn split_url(url: &str) -> Option<(String, u16, String)> {
    let rest = url.strip_prefix("http://")?;
    let (hp, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (h, p) = match hp.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok()?),
        None => (hp, 80),
    };
    Some((h.to_string(), p, path.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_responses() {
        let r = encode_request("POST", "100.65.2.1", "/mms", Some("application/vnd.wap.mms-message"), &[1, 2, 3, 0]);
        assert_eq!(parse_request(&r[..r.len() - 1]).unwrap(), None, "body incomplete");
        let (q, n) = parse_request(&r).unwrap().unwrap();
        assert_eq!(n, r.len());
        assert_eq!((q.method.as_str(), q.target.as_str(), q.body.as_slice()), ("POST", "/mms", &[1u8, 2, 3, 0][..]));
        assert_eq!(q.header("content-type"), Some("application/vnd.wap.mms-message"));
        let s = encode_response(200, "audio/wav", &[9; 10]);
        assert_eq!(parse_response(&s[..20], false).unwrap(), None);
        let p = parse_response(&s, false).unwrap().unwrap();
        assert_eq!((p.status, p.body.len()), (200, 10));
        assert!(parse_request(b"GARBAGE\r\n\r\n").is_err());
        assert_eq!(split_url("http://100.65.2.1:8002/mms/x"), Some(("100.65.2.1".into(), 8002, "/mms/x".into())));
        assert_eq!(split_url("http://h"), Some(("h".into(), 80, "/".into())));
    }
}
