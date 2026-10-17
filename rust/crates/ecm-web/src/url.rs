//! Addresses: `http://host[:port]/path?query#fragment`, `file:///path` and the
//! browser's own `about:` pages. Parsing, resolving a link against the page's
//! address, and what the address bar makes of typed text.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scheme {
    Http,
    File,
    /// `about:blank`, `about:home`, …: pages the browser makes itself.
    About,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub scheme: Scheme,
    /// Lower-cased host (empty for `file:` and `about:`).
    pub host: String,
    pub port: u16,
    /// Starts with `/` for http and file; the page name for about.
    pub path: String,
    /// Without the `?` (None = no query).
    pub query: Option<String>,
    /// Without the `#`.
    pub fragment: Option<String>,
}

impl Url {
    /// Parse an absolute address. None for other schemes or a bad port.
    pub fn parse(s: &str) -> Option<Url> {
        let s = s.trim();
        let (scheme, rest) = s.split_once(':')?;
        match scheme.to_ascii_lowercase().as_str() {
            "http" => {
                let rest = rest.strip_prefix("//")?;
                let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
                let (authority, tail) = rest.split_at(end);
                // user@ is not supported; drop it.
                let authority = authority.rsplit('@').next().unwrap_or("");
                let (host, port) = match authority.rsplit_once(':') {
                    Some((h, p)) if !p.is_empty() => (h, p.parse::<u16>().ok().filter(|p| *p > 0)?),
                    Some((h, _)) => (h, 80),
                    None => (authority, 80),
                };
                if host.is_empty() {
                    return None;
                }
                let mut u = Url { scheme: Scheme::Http, host: host.to_ascii_lowercase(), port, path: String::new(), query: None, fragment: None };
                u.set_tail(tail);
                Some(u)
            }
            "file" => {
                // file:///a/b, file:/a/b and file://localhost/a/b all mean /a/b.
                let tail = match rest.strip_prefix("//") {
                    Some(r) => {
                        let start = r.find('/').unwrap_or(r.len());
                        &r[start..]
                    }
                    None => rest,
                };
                let mut u = Url { scheme: Scheme::File, host: String::new(), port: 0, path: String::new(), query: None, fragment: None };
                u.set_tail(tail);
                Some(u)
            }
            "about" => {
                let (page, frag) = match rest.split_once('#') {
                    Some((p, f)) => (p, Some(f.to_string())),
                    None => (rest, None),
                };
                Some(Url { scheme: Scheme::About, host: String::new(), port: 0, path: page.to_ascii_lowercase(), query: None, fragment: frag })
            }
            _ => None,
        }
    }

    /// What the address bar makes of typed text: an address as is; a path
    /// (`/home/site/index.html`) as a file; anything else as `http://<text>`.
    pub fn from_input(s: &str) -> Option<Url> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let lower = s.to_ascii_lowercase();
        if lower.starts_with("http:") || lower.starts_with("file:") || lower.starts_with("about:") {
            return Url::parse(s);
        }
        if lower.starts_with("https:") {
            // No TLS in the game: the same address over http.
            return Url::parse(&format!("http:{}", &s[6..]));
        }
        if s.starts_with('/') {
            return Url::parse(&format!("file://{s}"));
        }
        if s.contains(' ') {
            return None;
        }
        Url::parse(&format!("http://{s}"))
    }

    /// Path, query and fragment from the part after the authority.
    fn set_tail(&mut self, tail: &str) {
        let (rest, frag) = match tail.split_once('#') {
            Some((r, f)) => (r, Some(f.to_string())),
            None => (tail, None),
        };
        let (path, query) = match rest.split_once('?') {
            Some((p, q)) => (p, Some(q.to_string())),
            None => (rest, None),
        };
        self.path = normalize(if path.is_empty() { "/" } else { path });
        self.query = query;
        self.fragment = frag;
    }

    /// Resolve `href` (a link, a form action, a style sheet) against this address.
    pub fn join(&self, href: &str) -> Option<Url> {
        let href = href.trim();
        if href.is_empty() {
            let mut u = self.clone();
            u.fragment = None;
            return Some(u);
        }
        // An absolute address has a scheme: letters, then ':'.
        if let Some((scheme, _)) = href.split_once(':') {
            if !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
                return Url::from_input(href).filter(|_| matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https" | "file" | "about"));
            }
        }
        if self.scheme == Scheme::About {
            return None;
        }
        if let Some(rest) = href.strip_prefix("//") {
            return match self.scheme {
                Scheme::Http => Url::parse(&format!("http://{rest}")),
                _ => Url::parse(&format!("file://{rest}")),
            };
        }
        let mut u = self.clone();
        u.fragment = None;
        if let Some(f) = href.strip_prefix('#') {
            u.fragment = Some(f.to_string());
            return Some(u);
        }
        if href.starts_with('?') {
            u.set_tail(&format!("{}{}", self.path, href));
            return Some(u);
        }
        if href.starts_with('/') {
            u.set_tail(href);
            return Some(u);
        }
        let dir = match self.path.rfind('/') {
            Some(i) => &self.path[..=i],
            None => "/",
        };
        u.set_tail(&format!("{dir}{href}"));
        Some(u)
    }

    /// `host` or `host:port` (the `Host:` header).
    pub fn authority(&self) -> String {
        if self.port == 80 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// The path and query, as an HTTP request line wants them.
    pub fn request_target(&self) -> String {
        match &self.query {
            Some(q) => format!("{}?{}", self.path, q),
            None => self.path.clone(),
        }
    }

    /// The same address without its fragment (history, cookies, caches).
    pub fn without_fragment(&self) -> Url {
        let mut u = self.clone();
        u.fragment = None;
        u
    }

    /// The file path of a `file:` address, percent-decoded.
    pub fn file_path(&self) -> String {
        percent_decode(&self.path)
    }

    /// The last path segment (a download's file name).
    pub fn file_name(&self) -> String {
        let name = percent_decode(self.path.rsplit('/').next().unwrap_or(""));
        if name.is_empty() {
            "index.html".into()
        } else {
            name
        }
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.scheme {
            Scheme::Http => write!(f, "http://{}{}", self.authority(), self.request_target())?,
            Scheme::File => write!(f, "file://{}", self.request_target())?,
            Scheme::About => write!(f, "about:{}", self.path)?,
        }
        if let Some(fr) = &self.fragment {
            write!(f, "#{fr}")?;
        }
        Ok(())
    }
}

/// Remove `.` and `..` segments (`..` never goes above the root).
fn normalize(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let segs: Vec<&str> = path.split('/').collect();
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        match *seg {
            "" if i == 0 => {}
            "." => {
                if last {
                    out.push("");
                }
            }
            ".." => {
                out.pop();
                if last {
                    out.push("");
                }
            }
            s => out.push(s),
        }
    }
    format!("/{}", out.join("/"))
}

/// `%41` -> `A` (and `+` stays `+`; forms use [`form_decode`]).
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A form field as `application/x-www-form-urlencoded` (spaces as `+`).
pub fn form_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The inverse of [`form_encode`].
pub fn form_decode(s: &str) -> String {
    percent_decode(&s.replace('+', " "))
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_and_file() {
        let u = Url::parse("http://100.65.0.10/").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("100.65.0.10", 80, "/"));
        let u = Url::parse("HTTP://Web.V4.Ring:8080/a/b.html?x=1#top").unwrap();
        assert_eq!(u.host, "web.v4.ring");
        assert_eq!(u.port, 8080);
        assert_eq!(u.request_target(), "/a/b.html?x=1");
        assert_eq!(u.fragment.as_deref(), Some("top"));
        assert_eq!(u.to_string(), "http://web.v4.ring:8080/a/b.html?x=1#top");
        assert_eq!(Url::parse("http://h").unwrap().to_string(), "http://h/");
        let f = Url::parse("file:///home/site/index.html").unwrap();
        assert_eq!(f.scheme, Scheme::File);
        assert_eq!(f.path, "/home/site/index.html");
        assert_eq!(f.to_string(), "file:///home/site/index.html");
        assert!(Url::parse("ftp://x/").is_none());
        assert!(Url::parse("http://h:99999/").is_none());
        assert!(Url::parse("http:///x").is_none());
    }

    #[test]
    fn address_bar_input() {
        assert_eq!(Url::from_input("100.66.0.10").unwrap().to_string(), "http://100.66.0.10/");
        assert_eq!(Url::from_input("/home/a.html").unwrap().to_string(), "file:///home/a.html");
        assert_eq!(Url::from_input("https://x/y").unwrap().to_string(), "http://x/y");
        assert!(Url::from_input("two words").is_none());
        assert_eq!(Url::from_input("about:blank").unwrap().scheme, Scheme::About);
    }

    #[test]
    fn joins_links() {
        let base = Url::parse("http://h/dir/page.html?q=1#f").unwrap();
        let j = |s: &str| base.join(s).unwrap().to_string();
        assert_eq!(j("other.html"), "http://h/dir/other.html");
        assert_eq!(j("../up.html"), "http://h/up.html");
        assert_eq!(j("../../../x"), "http://h/x");
        assert_eq!(j("/abs"), "http://h/abs");
        assert_eq!(j("?p=2"), "http://h/dir/page.html?p=2");
        assert_eq!(j("#sec"), "http://h/dir/page.html?q=1#sec");
        assert_eq!(j("//other:81/z"), "http://other:81/z");
        assert_eq!(j("http://100.65.0.10/"), "http://100.65.0.10/");
        assert_eq!(j("./a/./b/"), "http://h/dir/a/b/");
        assert!(base.join("javascript:void(0)").is_none());
        assert!(base.join("mailto:x@y").is_none());
        let f = Url::parse("file:///home/site/index.html").unwrap();
        assert_eq!(f.join("style.css").unwrap().to_string(), "file:///home/site/style.css");
    }

    #[test]
    fn form_coding_round_trips() {
        assert_eq!(form_encode("a b&c=d/é"), "a+b%26c%3Dd%2F%C3%A9");
        assert_eq!(form_decode("a+b%26c%3Dd%2F%C3%A9"), "a b&c=d/é");
        assert_eq!(percent_decode("/my%20site/x%2"), "/my site/x%2");
    }
}
