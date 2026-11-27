//! Test support shared by the auth-api and repo-api integration tests: a real
//! auth-api on an ephemeral port with a temporary database, a browser-like client
//! for the web forms, and raw-socket requests.
#![allow(dead_code)]

use auth_api::{App, Config, Limits};
use backend_common::ratelimit::Limit;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const SECRET: &str = "test-internal-secret-0123456789";
pub const PASSWORD: &str = "correct horse battery";
pub const ADMIN_EMAIL: &str = "admin@example.com";

pub struct Auth {
    pub app: App,
    pub addr: SocketAddr,
    pub url: String,
    pub dir: tempfile::TempDir,
}

/// Limits high enough that only the tests about limits meet them.
pub fn roomy() -> Limits {
    let l = Limit::per_minute(10_000);
    Limits { signin: l, signup: l, forgot: l, device_start: l }
}

pub async fn start_auth(tweak: impl FnOnce(&mut Config)) -> Auth {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config {
        public_origin: "http://repo.test".into(),
        listen: "127.0.0.1:0".into(),
        database_path: dir.path().join("auth.db").to_str().unwrap().into(),
        internal_secret: SECRET.into(),
        email_provider: "log".into(),
        email_api_key: None,
        email_from: "ECM packages <noreply@repo.test>".into(),
        admin_emails: vec![ADMIN_EMAIL.into()],
        trust_proxy: false,
        limits: roomy(),
    };
    tweak(&mut cfg);
    let app = auth_api::build(cfg).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(auth_api::serve(app.clone(), listener));
    Auth { app, addr, url: format!("http://{addr}"), dir }
}

/// A response, read whole.
pub struct Page {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: String,
}

impl Page {
    pub fn location(&self) -> &str {
        self.headers.get("location").map(|v| v.to_str().unwrap()).unwrap_or("")
    }

    pub fn cookies(&self) -> Vec<String> {
        self.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect()
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
}

pub async fn page(r: reqwest::RequestBuilder) -> Page {
    let resp = r.send().await.unwrap();
    Page { status: resp.status().as_u16(), headers: resp.headers().clone(), body: resp.text().await.unwrap() }
}

/// The value of the first `name="<name>" value="..."` in a page.
pub fn hidden(body: &str, name: &str) -> Option<String> {
    let at = body.find(&format!("name=\"{name}\" value=\""))? + name.len() + 15;
    Some(body[at..].split('"').next()?.to_string())
}

/// A person at a browser: keeps cookies, does not follow redirects, fills in the CSRF field of forms.
pub struct Browser {
    pub http: reqwest::Client,
    pub base: String,
}

impl Browser {
    pub fn new(base: &str) -> Browser {
        let http = reqwest::Client::builder().cookie_store(true).redirect(reqwest::redirect::Policy::none()).build().unwrap();
        Browser { http, base: base.to_string() }
    }

    pub async fn get(&self, path: &str) -> Page {
        page(self.http.get(format!("{}{path}", self.base))).await
    }

    /// Open `form_page`, take its CSRF token and post `fields` to `path`.
    pub async fn submit(&self, form_page: &str, path: &str, fields: &[(&str, &str)]) -> Page {
        let shown = self.get(form_page).await;
        let csrf = hidden(&shown.body, "csrf").unwrap_or_else(|| panic!("{form_page} has no CSRF field ({}): {}", shown.status, shown.body));
        let mut all: Vec<(&str, &str)> = vec![("csrf", &csrf)];
        all.extend_from_slice(fields);
        page(self.http.post(format!("{}{path}", self.base)).form(&all)).await
    }

    pub async fn sign_up(&self, email: &str, handle: &str) -> Page {
        self.submit("/signup", "/signup", &[("email", email), ("handle", handle), ("password", PASSWORD)]).await
    }

    pub async fn sign_in(&self, email: &str, password: &str) -> Page {
        self.submit("/signin", "/signin", &[("email", email), ("password", password)]).await
    }

    /// Make an API key on the account page; the key as the page shows it.
    pub async fn api_key(&self, name: &str, scopes: &[&str]) -> String {
        let mut fields = vec![("action", "create_key"), ("name", name)];
        for s in scopes {
            fields.push(match *s {
                "social" => ("scope_social", "on"),
                "publish" => ("scope_publish", "on"),
                other => panic!("no such scope {other}"),
            });
        }
        let p = self.submit("/account", "/account", &fields).await;
        assert_eq!(p.status, 200, "{}", p.body);
        let at = p.body.find("ecm_k_").expect("the new key is shown");
        p.body[at..at + 6 + 64].to_string()
    }
}

/// The newest mailed link to `to` whose path starts with `path` (`/verify`, `/reset`), as a path and query.
pub async fn mailed_link(auth: &Auth, to: &str, path: &str) -> Option<String> {
    let marker = format!("{}{path}?token=", auth.app.cfg.public_origin);
    // Mails are sent in the background.
    for _ in 0..100 {
        let found = auth.app.outbox.lock().unwrap().iter().rev().find(|m| m.to == to && m.text.contains(&marker)).map(|m| m.text.clone());
        if let Some(text) = found {
            let at = text.find(&marker).unwrap() + auth.app.cfg.public_origin.len();
            return Some(text[at..].split_whitespace().next().unwrap().to_string());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    None
}

/// Sign up `<handle>@example.com` and, with `verify`, confirm the address. The signed-in browser.
pub async fn new_user(auth: &Auth, handle: &str, verify: bool) -> Browser {
    let b = Browser::new(&auth.url);
    let email = format!("{handle}@example.com");
    let p = b.sign_up(&email, handle).await;
    assert_eq!((p.status, p.location()), (303, "/account"), "{}", p.body);
    if verify {
        let link = mailed_link(auth, &email, "/verify").await.expect("a verification mail");
        assert_eq!(b.get(&link).await.status, 200);
    }
    b
}

/// One HTTP exchange over a plain socket, the way the in-game client does it:
/// send, read until the server closes. Returns (head, body).
pub async fn raw(addr: SocketAddr, request: &[u8]) -> (String, Vec<u8>) {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(request).await.unwrap();
    let mut all = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut all)).await.expect("the server closes the connection").unwrap();
    let split = all.windows(4).position(|w| w == b"\r\n\r\n").expect("a header block");
    (String::from_utf8_lossy(&all[..split]).to_string(), all[split + 4..].to_vec())
}

pub fn raw_status(head: &str) -> u16 {
    head.split_whitespace().nth(1).unwrap().parse().unwrap()
}

pub fn raw_header(head: &str, name: &str) -> Option<String> {
    head.lines().skip(1).find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
    })
}

/// What the in-game client needs of a response: a Content-Length that matches the
/// body, no chunked encoding, and the connection closed afterwards.
pub fn assert_plain_framing(head: &str, body: &[u8]) {
    assert!(raw_header(head, "transfer-encoding").is_none(), "chunked response:\n{head}");
    let length = raw_header(head, "content-length").unwrap_or_else(|| panic!("no Content-Length:\n{head}"));
    assert_eq!(length.parse::<usize>().unwrap(), body.len(), "{head}");
}
