//! `GET /`: a small server-rendered catalogue (search, kinds, likes) linking to `/account`.

use crate::{ops, App};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use backend_common::html::{esc, page, respond, url_encode};
use std::collections::HashMap;

const STYLESHEET: &str = "/static/repo/style.css";
const SORTS: [(&str, &str); 4] = [("likes", "Most liked"), ("new", "Newest"), ("downloads", "Most downloaded"), ("name", "Name")];

fn options(choices: &[(&str, &str)], selected: &str) -> String {
    choices
        .iter()
        .map(|(value, label)| format!("<option value=\"{}\"{}>{}</option>", esc(value), if *value == selected { " selected" } else { "" }, esc(label)))
        .collect()
}

pub async fn catalogue(State(app): State<App>, Query(q): Query<HashMap<String, String>>) -> Response {
    let get = |k: &str| q.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
    let query = get("q").unwrap_or("");
    let kind = get("kind").filter(|k| ecm_pkg::manifest::Kind::parse(k).is_some());
    let sort_name = get("sort").filter(|s| SORTS.iter().any(|(v, _)| v == s)).unwrap_or("likes");
    let sort = ops::Sort::parse(sort_name).unwrap_or(ops::Sort::Likes);
    let page_no: usize = get("page").and_then(|p| p.parse().ok()).filter(|n| *n >= 1).unwrap_or(1);

    let nav = "<a href=\"/account\">Account</a>";
    let loaded = ops::load(&app.db.lock().unwrap(), None);
    let Ok(pkgs) = loaded else {
        return respond(StatusCode::INTERNAL_SERVER_ERROR, page("Error", STYLESHEET, nav, "<h1>Something went wrong</h1>"));
    };
    let found = ops::search(&pkgs, &ops::Search { channel: None, q: query, kind, sort });

    let mut body = format!(
        "<h1>Packages</h1><section><form class=\"search\" method=\"get\" action=\"/\">\
         <input type=\"search\" name=\"q\" value=\"{q}\" placeholder=\"Search packages\" aria-label=\"Search packages\">\
         <select name=\"kind\" aria-label=\"Kind\">{kinds}</select><select name=\"sort\" aria-label=\"Sort by\">{sorts}</select>\
         <button>Search</button></form></section>",
        q = esc(query),
        kinds = options(&[("", "All kinds"), ("app", "Apps"), ("program", "Programs"), ("library", "Libraries"), ("source", "Source")], kind.unwrap_or("")),
        sorts = options(&SORTS, sort_name),
    );
    if found.is_empty() {
        body.push_str("<p class=\"note\">No packages found.</p>");
    }
    let start = (page_no - 1).saturating_mul(ops::PER_PAGE);
    for (p, v) in found.iter().skip(start).take(ops::PER_PAGE) {
        body.push_str(&format!(
            "<article class=\"card pkg\"><h2>{name} <span class=\"badge\">{kind}</span></h2>\
             <p class=\"meta\"><code>{id}</code> {version} by {owner}</p><p>{summary}</p>\
             <p class=\"meta\">{likes} likes, {downloads} downloads, in {channels}</p></article>",
            name = esc(&v.entry.name),
            kind = esc(ops::kind_of(&v.entry)),
            id = esc(&p.id),
            version = esc(&v.version),
            owner = esc(&p.owner),
            summary = esc(&v.entry.summary),
            likes = p.likes,
            downloads = p.downloads,
            channels = esc(&p.channels().join(", ")),
        ));
    }
    let link = |n: usize| format!("/?q={}&amp;kind={}&amp;sort={}&amp;page={}", url_encode(query), url_encode(kind.unwrap_or("")), url_encode(sort_name), n);
    let (older, newer) = (page_no > 1 && start > 0, start + ops::PER_PAGE < found.len());
    if older || newer {
        body.push_str("<p class=\"pager\">");
        body.push_str(&if older { format!("<a href=\"{}\">Previous</a>", link(page_no - 1)) } else { "<span></span>".into() });
        body.push_str(&if newer { format!("<a href=\"{}\">Next</a>", link(page_no + 1)) } else { "<span></span>".into() });
        body.push_str("</p>");
    }
    body.push_str(&format!(
        "<section><h2>Use this repository in the game</h2><p>On a computer, add a channel and install from it:</p>\
         <code class=\"token\">pkg repo add official {origin}/r/stable/ --key {key}</code>\
         <p class=\"hint\">Channels: <code>stable</code> and <code>beta</code> are curated, <code>community</code> has every upload. \
         To publish, <a href=\"/signup\">make an account</a> and run <code>pkg login</code>.</p></section>",
        origin = esc(&app.cfg.public_origin),
        key = esc(&app.public_key),
    ));
    respond(StatusCode::OK, page("Packages", STYLESHEET, nav, &body))
}
