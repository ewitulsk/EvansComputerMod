//! The parts of `httpd` that don't touch the network, tested on the host:
//! the command line, where a request's file is (virtual hosts), content
//! types, and the CGI protocol (what a script is handed on stdin and how its
//! output becomes a response).
//!
//!   httpd <port> [dir] [--cgi <prefix>]
//!
//! * `dir`: the folder served (default: the computer's root, as the village
//!   web servers do). `/` is its `index.html`, or a listing.
//! * Virtual hosting: a request whose `Host:` names a folder inside `dir`
//!   (`dir/web.v4.ring/`) is served from that folder.
//! * `--cgi /cgi`: a request under the prefix runs the script it names
//!   (`.py` with `python`, `.lua` with `lua`) in a command session of its
//!   own: the request goes to its stdin as `NAME=value` lines (REQUEST_METHOD,
//!   PATH_INFO, QUERY_STRING, CONTENT_TYPE, CONTENT_LENGTH, REMOTE_ADDR,
//!   HTTP_HOST, HTTP_COOKIE, HTTP_USER_AGENT), a blank line and the body; what
//!   it prints is the response: optional `Status:`, `Content-Type:`,
//!   `Location:`, `Set-Cookie:` lines, a blank line, the body. The Python and
//!   Lua `web.cgi(handler)` speak this.

/// How long a CGI script may take, ms.
pub const CGI_TIMEOUT_MS: u64 = 10_000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub port: u16,
    /// Empty: the computer's root.
    pub root: String,
    pub cgi: Option<String>,
}

pub fn parse_args(args: &[String]) -> Result<Config, String> {
    let mut c = Config::default();
    let mut port = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cgi" => {
                i += 1;
                let p = args.get(i).ok_or("--cgi needs a path prefix, e.g. --cgi /cgi")?;
                let p = format!("/{}", p.trim_matches('/'));
                c.cgi = Some(p);
            }
            a if port.is_none() => port = Some(a.parse::<u16>().map_err(|_| format!("bad port {a}"))?),
            a if c.root.is_empty() => c.root = a.trim_end_matches('/').to_string(),
            a => return Err(format!("unexpected argument {a}")),
        }
        i += 1;
    }
    c.port = port.ok_or("no port")?;
    Ok(c)
}

/// A request's `Host:` without its port, lower case, if it is a safe folder name.
pub fn host_name(host: &str) -> Option<String> {
    let h = host.trim().rsplit_once(':').map_or(host.trim(), |(h, p)| if p.chars().all(|c| c.is_ascii_digit()) { h } else { host.trim() });
    let h = h.to_ascii_lowercase();
    (!h.is_empty() && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') && !h.starts_with('.')).then_some(h)
}

/// The folder a request is served from: `root/<host>` when that folder exists, else `root`.
pub fn site_root(root: &str, host: Option<&str>, is_dir: impl Fn(&str) -> bool) -> String {
    if !root.is_empty() {
        if let Some(h) = host.and_then(host_name) {
            let d = format!("{root}/{h}");
            if is_dir(&d) {
                return d;
            }
        }
    }
    root.to_string()
}

/// Where a request path's file is on disk under `root` (None: the path climbs out with `..`).
pub fn local(root: &str, path: &str) -> Option<String> {
    let path = path.split('?').next().unwrap_or("").trim_start_matches('/');
    let path = percent_decode(path);
    if path.split('/').any(|seg| seg == "..") {
        return None;
    }
    Some(if root.is_empty() {
        if path.is_empty() {
            ".".to_string()
        } else {
            path
        }
    } else if path.is_empty() {
        root.to_string()
    } else {
        format!("{}/{}", root.trim_end_matches('/'), path)
    })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn content_type(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    match lower.rsplit('.').next().unwrap_or("") {
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "text/javascript",
        "png" => "image/png",
        "json" => "application/json",
        "py" => "text/x-python",
        "lua" => "text/x-lua",
        "ecmpkg" => "application/octet-stream",
        _ => "text/plain",
    }
}

/// The command line that runs a CGI script, by its extension (None: not a script).
pub fn cgi_command(script: &str) -> Option<String> {
    let lower = script.to_ascii_lowercase();
    if lower.ends_with(".py") {
        Some(format!("python {script}"))
    } else if lower.ends_with(".lua") {
        Some(format!("lua {script}"))
    } else {
        None
    }
}

/// A parsed request.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub method: String,
    /// Path and query, as sent.
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn parse(raw: &[u8]) -> Option<Request> {
        let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
        let head = String::from_utf8_lossy(&raw[..end]);
        let mut lines = head.split("\r\n");
        let mut first = lines.next()?.split_whitespace();
        let method = first.next()?.to_string();
        let target = first.next()?.to_string();
        let headers = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
        Some(Request { method, target, headers, body: raw[end + 4..].to_vec() })
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("/")
    }

    pub fn query(&self) -> &str {
        self.target.split_once('?').map_or("", |(_, q)| q)
    }
}

/// What a CGI script reads on stdin.
pub fn cgi_input(req: &Request, remote: &str) -> Vec<u8> {
    let clean = |s: &str| s.replace(['\r', '\n'], " ");
    let mut s = String::new();
    s.push_str(&format!("REQUEST_METHOD={}\n", clean(&req.method)));
    s.push_str(&format!("PATH_INFO={}\n", clean(req.path())));
    s.push_str(&format!("QUERY_STRING={}\n", clean(req.query())));
    s.push_str(&format!("CONTENT_TYPE={}\n", clean(req.header("content-type").unwrap_or(""))));
    s.push_str(&format!("CONTENT_LENGTH={}\n", req.body.len()));
    s.push_str(&format!("REMOTE_ADDR={}\n", clean(remote)));
    for (h, name) in [("host", "HTTP_HOST"), ("cookie", "HTTP_COOKIE"), ("user-agent", "HTTP_USER_AGENT")] {
        if let Some(v) = req.header(h) {
            s.push_str(&format!("{name}={}\n", clean(v)));
        }
    }
    s.push('\n');
    let mut out = s.into_bytes();
    // A command session's terminal ends a program on Ctrl+C / Ctrl+T: never pass those bytes.
    out.extend(req.body.iter().copied().filter(|b| *b != 0x03 && *b != 0x14));
    out
}

/// A response from what a CGI script printed (its terminal turns newlines into CRLF).
pub fn cgi_response(output: &str) -> (u16, String, Vec<(String, String)>, Vec<u8>) {
    let text = output.replace("\r\n", "\n").replace('\r', "");
    let (head, body) = match text.split_once("\n\n") {
        Some((h, b)) if h.lines().all(|l| l.contains(':')) && !h.is_empty() => (h.to_string(), b.to_string()),
        _ => (String::new(), text.clone()),
    };
    let mut status = 200;
    let mut reason = "OK".to_string();
    let mut headers = Vec::new();
    let mut typed = false;
    let mut located = false;
    for line in head.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let (k, v) = (k.trim(), v.trim());
        if k.eq_ignore_ascii_case("status") {
            let mut parts = v.splitn(2, ' ');
            status = parts.next().and_then(|s| s.parse().ok()).unwrap_or(200);
            reason = parts.next().unwrap_or("").to_string();
            continue;
        }
        if k.eq_ignore_ascii_case("content-type") {
            typed = true;
        }
        if k.eq_ignore_ascii_case("location") {
            located = true;
        }
        if k.eq_ignore_ascii_case("content-length") || k.eq_ignore_ascii_case("connection") {
            continue;
        }
        headers.push((k.to_string(), v.to_string()));
    }
    if located && status == 200 {
        status = 302;
        reason = "Found".into();
    }
    if !typed {
        headers.insert(0, ("Content-Type".into(), "text/html".into()));
    }
    if reason.is_empty() {
        reason = "OK".into();
    }
    (status, reason, headers, body.into_bytes())
}

/// A response's bytes.
pub fn response(status: u16, reason: &str, headers: &[(String, String)], body: &[u8]) -> Vec<u8> {
    let mut h = format!("HTTP/1.0 {status} {reason}\r\n");
    for (k, v) in headers {
        h.push_str(&format!("{k}: {v}\r\n"));
    }
    h.push_str(&format!("Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()));
    let mut out = h.into_bytes();
    out.extend_from_slice(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn command_line() {
        assert_eq!(parse_args(&args("80")).unwrap(), Config { port: 80, root: String::new(), cgi: None });
        assert_eq!(parse_args(&args("80 /srv/www/ --cgi cgi")).unwrap(), Config { port: 80, root: "/srv/www".into(), cgi: Some("/cgi".into()) });
        assert_eq!(parse_args(&args("--cgi /bin 8080 /x")).unwrap().cgi.as_deref(), Some("/bin"));
        assert!(parse_args(&args("eighty")).is_err());
        assert!(parse_args(&args("80 /a /b")).is_err());
        assert!(parse_args(&args("80 --cgi")).is_err());
    }

    #[test]
    fn virtual_hosts_and_paths() {
        let dirs = |d: &str| d == "/srv/www/web.v4.ring";
        assert_eq!(site_root("/srv/www", Some("Web.V4.Ring:80"), dirs), "/srv/www/web.v4.ring");
        assert_eq!(site_root("/srv/www", Some("other.ring"), dirs), "/srv/www");
        assert_eq!(site_root("/srv/www", Some("../etc"), dirs), "/srv/www");
        assert_eq!(site_root("", Some("web.v4.ring"), dirs), "", "no folder given: no virtual hosts");
        assert_eq!(local("/srv/www", "/a/b.html?x=1").as_deref(), Some("/srv/www/a/b.html"));
        assert_eq!(local("/srv/www", "/my%20page.html").as_deref(), Some("/srv/www/my page.html"));
        assert_eq!(local("", "/index.html").as_deref(), Some("index.html"));
        assert_eq!(local("/srv/www", "/../etc/passwd"), None);
        assert_eq!(local("/srv/www", "/a/%2e%2e/x"), None);
        assert_eq!(content_type("style.CSS"), "text/css");
        assert_eq!(cgi_command("/srv/www/cgi/g.py").as_deref(), Some("python /srv/www/cgi/g.py"));
        assert_eq!(cgi_command("/srv/www/cgi/g.lua").as_deref(), Some("lua /srv/www/cgi/g.lua"));
        assert_eq!(cgi_command("/srv/www/cgi/g.sh"), None);
    }

    #[test]
    fn cgi_protocol() {
        let req = Request::parse(b"POST /cgi/g.py?x=1 HTTP/1.0\r\nHost: h\r\nCookie: who=Pat\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 7\r\n\r\nname=Bo").unwrap();
        let input = String::from_utf8(cgi_input(&req, "10.0.0.2")).unwrap();
        assert_eq!(
            input,
            "REQUEST_METHOD=POST\nPATH_INFO=/cgi/g.py\nQUERY_STRING=x=1\nCONTENT_TYPE=application/x-www-form-urlencoded\nCONTENT_LENGTH=7\n\
             REMOTE_ADDR=10.0.0.2\nHTTP_HOST=h\nHTTP_COOKIE=who=Pat\n\nname=Bo"
        );
        let (s, r, h, b) = cgi_response("Status: 400 Bad Request\r\nContent-Type: text/html\r\nSet-Cookie: a=b\r\n\r\n<p>no</p>\r\n");
        assert_eq!((s, r.as_str()), (400, "Bad Request"));
        assert_eq!(h, vec![("Content-Type".to_string(), "text/html".to_string()), ("Set-Cookie".to_string(), "a=b".to_string())]);
        assert_eq!(b, b"<p>no</p>\n");
        let (s, _, h, _) = cgi_response("Location: /\r\n\r\n");
        assert_eq!(s, 302);
        assert!(h.iter().any(|(k, v)| k == "Location" && v == "/"));
        // No header block: the whole output is the page.
        let (s, _, h, b) = cgi_response("<h1>hello</h1>\r\n");
        assert_eq!((s, b.as_slice()), (200, b"<h1>hello</h1>\n".as_slice()));
        assert_eq!(h[0].1, "text/html");
        let resp = String::from_utf8(response(200, "OK", &h, b"x")).unwrap();
        assert!(resp.starts_with("HTTP/1.0 200 OK\r\nContent-Type: text/html\r\nContent-Length: 1\r\n"));
    }
}
