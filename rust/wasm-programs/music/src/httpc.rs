//! The client side of HTTP/1.1 for talking to musicd: request text, and a
//! response reader fed bytes as they arrive (the body is handed on in pieces,
//! so a song chunk can go to the decoder before all of it is in).

/// A GET/POST/... request with `headers` (complete lines ending in CRLF).
pub fn request(method: &str, path: &str, host: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut s = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\n{headers}");
    if !body.is_empty() || method == "POST" || method == "PUT" {
        s.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", body.len()));
    }
    s.push_str("\r\n");
    let mut v = s.into_bytes();
    v.extend_from_slice(body);
    v
}

/// A response's status line and headers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Head {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub content_length: usize,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// `Content-Range: bytes a-b/total`: (a, b, total).
    pub fn content_range(&self) -> Option<(u64, u64, u64)> {
        let v = self.header("content-range")?.strip_prefix("bytes ")?;
        let (range, total) = v.split_once('/')?;
        let total = total.trim().parse().ok()?;
        let (a, b) = range.split_once('-')?;
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?, total))
    }

    /// The total size of the resource (`Content-Range` or, for a 200, `Content-Length`).
    pub fn total(&self) -> Option<u64> {
        if let Some((_, _, t)) = self.content_range() {
            return Some(t);
        }
        if self.header("content-range").is_some_and(|v| v.starts_with("bytes */")) {
            return self.header("content-range")?.strip_prefix("bytes */")?.trim().parse().ok();
        }
        (self.status == 200).then_some(self.content_length as u64)
    }
}

/// Reads one response.
#[derive(Default, Debug)]
pub struct Reader {
    buf: Vec<u8>,
    pub head: Option<Head>,
    /// Body bytes received so far.
    pub got: usize,
}

impl Reader {
    pub fn new() -> Reader {
        Reader::default()
    }

    /// Feed bytes; returns the body bytes among them (Err for a broken head).
    pub fn push(&mut self, data: &[u8]) -> Result<Vec<u8>, String> {
        if self.head.is_some() {
            let want = self.remaining();
            let take = data.len().min(want);
            self.got += take;
            return Ok(data[..take].to_vec());
        }
        self.buf.extend_from_slice(data);
        let Some(end) = self.buf.windows(4).position(|w| w == b"\r\n\r\n") else {
            if self.buf.len() > 16 * 1024 {
                return Err("the response head is too large".into());
            }
            return Ok(Vec::new());
        };
        let text = String::from_utf8_lossy(&self.buf[..end]).into_owned();
        let mut lines = text.split("\r\n");
        let status_line = lines.next().unwrap_or("");
        let mut parts = status_line.splitn(3, ' ');
        let version = parts.next().unwrap_or("");
        let status: u16 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| format!("not an HTTP answer: {status_line}"))?;
        if !version.starts_with("HTTP/") {
            return Err(format!("not an HTTP answer: {status_line}"));
        }
        let headers: Vec<(String, String)> =
            lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
        let content_length = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
        self.head = Some(Head { status, headers, content_length });
        let rest = self.buf.split_off(end + 4);
        self.buf.clear();
        self.push(&rest)
    }

    pub fn remaining(&self) -> usize {
        self.head.as_ref().map_or(usize::MAX, |h| h.content_length - self.got)
    }

    /// The whole response is in.
    pub fn done(&self) -> bool {
        self.head.is_some() && self.remaining() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_response_in_pieces() {
        let raw = b"HTTP/1.1 206 Partial Content\r\nContent-Length: 5\r\nContent-Range: bytes 10-14/100\r\n\r\nhello";
        let mut r = Reader::new();
        let mut body = Vec::new();
        for c in raw.chunks(7) {
            body.extend(r.push(c).unwrap());
        }
        assert!(r.done());
        assert_eq!(body, b"hello");
        let h = r.head.unwrap();
        assert_eq!((h.status, h.content_range(), h.total()), (206, Some((10, 14, 100)), Some(100)));
        assert!(Reader::new().push(b"garbage\r\n\r\n").is_err());
        let req = String::from_utf8(request("PUT", "/p/1", "h:7800", "X: y\r\n", b"{}")).unwrap();
        assert!(req.starts_with("PUT /p/1 HTTP/1.1\r\nHost: h:7800\r\nX: y\r\n") && req.ends_with("Content-Length: 2\r\n\r\n{}"));
    }
}
