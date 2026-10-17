//! Pages the browser makes itself: `about:home` and the error pages.

use ecm_web::fetch::FetchError;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The start page (`about:home`): the Tech Village sites and the player's files.
pub fn home() -> String {
    let mut villages = String::new();
    for v in 1..=10 {
        villages.push_str(&format!("<li><a href=\"http://100.{}.0.10/\">Tech Village {v}</a></li>", 64 + v));
    }
    format!(
        "<html><head><title>Start</title><style>\
         body {{ margin: 12px }} h1 {{ margin-top: 0 }} .hint {{ opacity: 0.75 }}\
         .grid {{ display: flex; flex-wrap: wrap; gap: 4px 16px; padding: 0; list-style-type: none }} .grid li {{ width: 120px }}\
         </style></head><body>\
         <h1>Browser</h1>\
         <p class=hint>Type an address above: <code>100.65.0.10</code>, <code>http://host:8080/page.html</code> or a file, \
         <code>/home/site/index.html</code>.</p>\
         <h3>Tech Village sites</h3><ul class=grid>{villages}</ul>\
         <h3>Your files</h3><p><a href=\"file:///home/\">file:///home/</a>: pages you write with <code>edit</code>. \
         Serve a folder to other computers with <code>httpd 80</code>.</p>\
         </body></html>"
    )
}

/// An error page for `url`.
pub fn error(url: &str, e: &FetchError) -> String {
    let (title, detail) = e.explain();
    format!(
        "<html><head><title>{t}</title></head><body style='margin: 16px'>\
         <h2>{t}</h2><p>{d}</p><p><code>{u}</code></p><p><a href=\"{u}\">Try again</a></p></body></html>",
        t = esc(&title),
        d = esc(&detail),
        u = esc(url)
    )
}

/// A page for a file the browser can't show.
pub fn unshowable(url: &str, mime: &str, size: usize) -> String {
    format!(
        "<html><head><title>{n}</title></head><body style='margin: 16px'><h2>Can't show this file</h2>\
         <p><code>{u}</code> is <code>{m}</code>, {size} bytes.</p></body></html>",
        n = esc(url.rsplit('/').next().unwrap_or(url)),
        u = esc(url),
        m = esc(mime)
    )
}

/// A listing of a `file://` folder.
pub fn folder(url: &str, entries: &[(String, bool)]) -> String {
    let mut items = String::new();
    if url.trim_end_matches('/') != "file://" {
        items.push_str("<li><a href=\"../\">../</a></li>");
    }
    for (name, dir) in entries {
        let shown = if *dir { format!("{name}/") } else { name.clone() };
        items.push_str(&format!("<li><a href=\"{}\">{}</a></li>", esc(&shown.replace(' ', "%20")), esc(&shown)));
    }
    format!("<html><head><title>Index of {u}</title></head><body><h2>Index of {u}</h2><ul>{items}</ul></body></html>", u = esc(url))
}

/// `about:bookmarks`.
pub fn bookmarks(list: &[(String, String)]) -> String {
    let mut items = String::new();
    for (t, u) in list.iter().rev() {
        items.push_str(&format!("<li><a href=\"{}\">{}</a> <small>{}</small></li>", esc(u), esc(t), esc(u)));
    }
    if items.is_empty() {
        items = "<p>No bookmarks yet. Open a page, then Menu &gt; Bookmark this page.</p>".into();
    } else {
        items = format!("<ul>{items}</ul>");
    }
    format!("<html><head><title>Bookmarks</title></head><body style='margin: 12px'><h2>Bookmarks</h2>{items}</body></html>")
}

/// `about:history`: recent visits, newest first.
pub fn history(list: &[(String, String)]) -> String {
    let mut items = String::new();
    for (t, u) in list.iter().rev() {
        items.push_str(&format!("<li><a href=\"{}\">{}</a> <small>{}</small></li>", esc(u), esc(t), esc(u)));
    }
    if items.is_empty() {
        items = "<p>Nothing visited yet.</p>".into();
    } else {
        items = format!("<ul>{items}</ul>");
    }
    format!("<html><head><title>History</title></head><body style='margin: 12px'><h2>History</h2>{items}</body></html>")
}

/// A file saved to the Downloads folder.
pub fn downloaded(url: &str, path: &str, size: usize, mime: &str) -> String {
    let install = if path.ends_with(".ecmpkg") {
        format!("<p>An app package. Install it from a Terminal: <code>pkg install {}</code></p>", esc(path))
    } else {
        String::new()
    };
    format!(
        "<html><head><title>Downloaded</title></head><body style='margin: 16px'><h2>Downloaded</h2>\
         <p><code>{u}</code> ({m}, {size} bytes) is saved as <code>{p}</code>.</p>{install}\
         <p><a href=\"file://{p}\">Open it in the Browser</a> &middot; <a href=\"file:///home/Downloads/\">Downloads</a></p></body></html>",
        u = esc(url),
        m = esc(mime),
        p = esc(path)
    )
}
