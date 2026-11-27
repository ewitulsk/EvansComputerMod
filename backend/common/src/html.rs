//! Server-rendered pages: escaping, the shared layout, the stylesheet and the
//! security headers every HTML response carries.

use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

/// Escape text for HTML content and attribute values.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Percent-encode a query-string value.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The whole page. `nav` and `body` are HTML (already escaped by the caller); `title` is text.
pub fn page(title: &str, stylesheet: &str, nav: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title} - ECM packages</title>\n<link rel=\"stylesheet\" href=\"{css}\">\n</head>\n<body>\n\
         <header><a class=\"brand\" href=\"/\">ECM packages</a><nav>{nav}</nav></header>\n\
         <main>\n{body}\n</main>\n</body>\n</html>\n",
        title = esc(title),
        css = esc(stylesheet),
    )
}

const CSP: &str = "default-src 'none'; style-src 'self'; img-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'";

/// An HTML response with the security headers. Pages are never cached: they hold
/// CSRF tokens and account details.
pub fn respond(status: StatusCode, html: String) -> Response {
    let mut r = (status, html).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(HeaderName::from_static("permissions-policy"), HeaderValue::from_static("geolocation=(), camera=(), microphone=()"));
    r
}

/// 303 to a path on this site.
pub fn see_other(location: &str) -> Response {
    let mut r = StatusCode::SEE_OTHER.into_response();
    let value = HeaderValue::from_str(location).unwrap_or(HeaderValue::from_static("/"));
    r.headers_mut().insert(header::LOCATION, value);
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// The stylesheet response.
pub fn stylesheet() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        STYLE,
    )
        .into_response()
}

/// One small stylesheet for both services.
pub const STYLE: &str = r#"
:root { --ink: #1c2024; --soft: #5b6470; --line: #d9dee4; --paper: #ffffff; --wash: #f4f6f8; --accent: #1f6feb; --bad: #b42318; --good: #17663a; }
@media (prefers-color-scheme: dark) {
  :root { --ink: #e6e9ed; --soft: #9aa4b1; --line: #343b44; --paper: #15181c; --wash: #1d2126; --accent: #6ea8ff; --bad: #ff8a80; --good: #6fd08c; }
}
* { box-sizing: border-box; }
html { font: 16px/1.5 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif; color: var(--ink); background: var(--wash); }
body { margin: 0; }
header { display: flex; flex-wrap: wrap; gap: .5rem 1rem; align-items: center; justify-content: space-between; padding: .75rem 1rem; background: var(--paper); border-bottom: 1px solid var(--line); }
header .brand { font-weight: 700; color: var(--ink); text-decoration: none; }
nav { display: flex; flex-wrap: wrap; gap: .75rem; align-items: center; }
nav form { display: inline; margin: 0; }
main { max-width: 44rem; margin: 0 auto; padding: 1rem; }
h1 { font-size: 1.5rem; margin: .5rem 0 1rem; }
h2 { font-size: 1.15rem; margin: 0 0 .75rem; }
a { color: var(--accent); }
section, .card { background: var(--paper); border: 1px solid var(--line); border-radius: .5rem; padding: 1rem; margin: 0 0 1rem; }
label { display: block; font-weight: 600; margin: .75rem 0 .25rem; }
label.check { display: inline-block; font-weight: 400; margin: .25rem 1rem .25rem 0; }
input[type=text], input[type=email], input[type=password], input[type=search], select { width: 100%; padding: .55rem .6rem; font: inherit; color: inherit; background: var(--paper); border: 1px solid var(--line); border-radius: .375rem; }
button { font: inherit; padding: .5rem .9rem; margin-top: .75rem; color: #fff; background: var(--accent); border: 0; border-radius: .375rem; cursor: pointer; }
@media (prefers-color-scheme: dark) { button { color: #0b1220; } }
button.plain { color: var(--accent); background: none; padding: 0; margin: 0; text-decoration: underline; }
button.danger { background: var(--bad); }
.note, .error, .ok { border-radius: .375rem; padding: .6rem .8rem; margin: 0 0 1rem; border: 1px solid var(--line); background: var(--paper); }
.error { border-color: var(--bad); color: var(--bad); }
.ok { border-color: var(--good); color: var(--good); }
.hint, .meta { color: var(--soft); font-size: .9rem; }
.token, code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: .9rem; overflow-wrap: anywhere; }
.token { display: block; padding: .6rem; background: var(--wash); border: 1px dashed var(--line); border-radius: .375rem; }
.scroll { overflow-x: auto; }
table { width: 100%; border-collapse: collapse; }
th, td { text-align: left; padding: .4rem .5rem .4rem 0; border-bottom: 1px solid var(--line); vertical-align: top; }
td form { margin: 0; }
td button { margin: 0; padding: .25rem .6rem; }
.search { display: grid; grid-template-columns: 1fr; gap: .5rem; }
.search button { margin: 0; }
@media (min-width: 36rem) { .search { grid-template-columns: 1fr 9rem 9rem auto; } }
.pkg h2 { margin: 0; }
.pkg p { margin: .25rem 0; }
.badge { display: inline-block; padding: 0 .45rem; font-size: .8rem; border: 1px solid var(--line); border-radius: 1rem; color: var(--soft); }
.pager { display: flex; justify-content: space-between; }
.actions { display: flex; flex-wrap: wrap; gap: .75rem; }
"#;

#[cfg(test)]
mod tests {
    #[test]
    fn escaping() {
        assert_eq!(super::esc("<a href=\"x\">&'"), "&lt;a href=&quot;x&quot;&gt;&amp;&#39;");
        assert_eq!(super::url_encode("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
        let p = super::page("<t>", "/s.css", "", "<p>x</p>");
        assert!(p.contains("<title>&lt;t&gt; - ECM packages</title>") && p.contains("<p>x</p>"));
    }
}
