//! `http://` and `https://` addresses.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub https: bool,
    pub host: String,
    pub port: u16,
    /// Path and query, starting with `/`.
    pub target: String,
}

impl Url {
    pub fn parse(s: &str) -> Result<Url, String> {
        let s = s.trim();
        let (https, rest) = if let Some(r) = s.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = s.strip_prefix("http://") {
            (false, r)
        } else {
            return Err(format!("{s}: only http:// and https:// addresses are supported"));
        };
        let rest = rest.split('#').next().unwrap_or("");
        let cut = rest.find(|c| c == '/' || c == '?').unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(cut);
        let (host, port) = match authority.rfind(':') {
            Some(i) => (&authority[..i], authority[i + 1..].parse::<u16>().map_err(|_| format!("{s}: bad port"))?),
            None => (authority, if https { 443 } else { 80 }),
        };
        if host.is_empty() || !host.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-') {
            return Err(format!("{s}: bad host name"));
        }
        if tail.bytes().any(|c| c <= b' ' || c == 0x7F) {
            return Err(format!("{s}: spaces and control characters are not allowed in an address"));
        }
        let target = match tail.chars().next() {
            Some('/') => tail.to_string(),
            Some(_) => format!("/{tail}"),
            None => "/".to_string(),
        };
        Ok(Url { https, host: host.to_ascii_lowercase(), port, target })
    }

    pub fn scheme(&self) -> &'static str {
        if self.https {
            "https"
        } else {
            "http"
        }
    }

    fn default_port(&self) -> bool {
        self.port == if self.https { 443 } else { 80 }
    }

    /// `host` or `host:port`, as the Host header carries it.
    pub fn authority(&self) -> String {
        if self.default_port() {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// The path without the query.
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("/")
    }

    /// Resolve a `Location` (absolute, `//host/..`, `/path` or relative) against this address.
    pub fn join(&self, location: &str) -> Result<Url, String> {
        let l = location.trim();
        if l.starts_with("http://") || l.starts_with("https://") {
            return Url::parse(l);
        }
        if let Some(r) = l.strip_prefix("//") {
            return Url::parse(&format!("{}://{}", self.scheme(), r));
        }
        let target = if l.starts_with('/') {
            l.to_string()
        } else {
            let dir = &self.path()[..=self.path().rfind('/').unwrap_or(0)];
            format!("{dir}{l}")
        };
        Url::parse(&format!("{}://{}{}", self.scheme(), self.authority(), target))
    }
}

impl std::fmt::Display for Url {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}://{}{}", self.scheme(), self.authority(), self.target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display() {
        let u = Url::parse("https://Repo.EvanWitulski.com/r/stable/index.json?x=1#frag").unwrap();
        assert_eq!((u.https, u.host.as_str(), u.port, u.target.as_str()), (true, "repo.evanwitulski.com", 443, "/r/stable/index.json?x=1"));
        assert_eq!(u.to_string(), "https://repo.evanwitulski.com/r/stable/index.json?x=1");
        assert_eq!(u.path(), "/r/stable/index.json");
        let v = Url::parse("http://10.0.7.1:8080").unwrap();
        assert_eq!((v.port, v.target.as_str(), v.authority()), (8080, "/", "10.0.7.1:8080".to_string()));
        assert_eq!(Url::parse("http://h?q=1").unwrap().target, "/?q=1");
        assert_eq!(Url::parse("https://h:443/").unwrap().authority(), "h");
        for bad in ["ftp://h/", "http:///x", "http://h:99999/", "http://a b/", "http://h/a b", "h/x"] {
            assert!(Url::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn join_locations() {
        let u = Url::parse("http://h:8080/a/b/c?x=1").unwrap();
        assert_eq!(u.join("https://other/z").unwrap().to_string(), "https://other/z");
        assert_eq!(u.join("//cdn/z").unwrap().to_string(), "http://cdn/z");
        assert_eq!(u.join("/z").unwrap().to_string(), "http://h:8080/z");
        assert_eq!(u.join("d?y=2").unwrap().to_string(), "http://h:8080/a/b/d?y=2");
    }
}
