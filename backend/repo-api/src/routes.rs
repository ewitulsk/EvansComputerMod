//! The HTTP routes of `repo-api`. Every response is a complete body (so it carries
//! `Content-Length`), and every refusal is an `ApiError`.

use crate::auth::Caller;
use crate::{ops, App};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use backend_common::http::{bearer, json, no_content, JsonBody};
use backend_common::ApiError;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

pub async fn health() -> Response {
    ([(header::CONTENT_TYPE, "text/plain")], "ok\n").into_response()
}

pub async fn not_found() -> ApiError {
    ApiError::not_found("no such endpoint")
}

// ------------------------------------------------------------------ callers

/// The caller, when a good token was sent. A bad token on a public call is just nobody.
async fn maybe_caller(app: &App, headers: &HeaderMap) -> Option<Caller> {
    app.auth.introspect(bearer(headers)?).await.ok().flatten()
}

/// The caller, who must hold `scope` (`None`: any signed-in caller). 401 without a good token, 403 without the scope.
async fn require(app: &App, headers: &HeaderMap, scope: Option<&str>) -> Result<Caller, ApiError> {
    let presented = bearer(headers).ok_or_else(|| ApiError::unauthorized("send Authorization: Bearer <token>"))?;
    let caller = app.auth.introspect(presented).await?.ok_or_else(|| ApiError::unauthorized("the token is unknown or was revoked"))?;
    match scope {
        Some("admin") if !caller.admin => Err(ApiError::forbidden("this needs an admin")),
        Some(s) if s != "admin" && !caller.has(s) => Err(ApiError::forbidden(format!("this token lacks the '{s}' scope"))),
        _ => Ok(caller),
    }
}

// ------------------------------------------------------------------ repositories

pub async fn repository(State(app): State<App>, Path((channel, rest)): Path<(String, String)>) -> Result<Response, ApiError> {
    if !ops::valid_channel(&channel) {
        return Err(ApiError::not_found("no such channel"));
    }
    if rest == "index.json" {
        let text = ops::index_json(&app, &channel)?;
        return Ok(([(header::CONTENT_TYPE, "application/json"), (header::CACHE_CONTROL, "no-cache")], text.as_bytes().to_vec()).into_response());
    }
    if rest == ecm_pkg::paths::PUBLISHER_KEY_FILE {
        ops::index_json(&app, &channel)?; // 404 for an unknown channel
        return Ok(([(header::CONTENT_TYPE, "text/plain")], format!("{}\n", app.public_key)).into_response());
    }
    const SUFFIX: &str = ".ecmpkg";
    if let Some(file) = rest.strip_suffix(SUFFIX).filter(|f| !f.contains('/')) {
        let bytes = ops::download(&app, &channel, &format!("{file}{SUFFIX}")).await?;
        return Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response());
    }
    if let Some((file, member)) = rest.split_once(".ecmpkg/") {
        if !file.contains('/') && ecm_pkg::manifest::safe_rel_path(member) {
            let bytes = ops::member(&app, &channel, &format!("{file}{SUFFIX}"), member).await?;
            let headers = [
                (header::CONTENT_TYPE, "image/png"),
                (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ];
            return Ok((headers, bytes).into_response());
        }
    }
    Err(ApiError::not_found("no such file"))
}

// ------------------------------------------------------------------ catalogue

pub async fn list_packages(State(app): State<App>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<Response, ApiError> {
    let caller = maybe_caller(&app, &headers).await;
    let get = |k: &str| q.get(k).map(String::as_str).filter(|v| !v.is_empty());
    let sort = ops::Sort::parse(get("sort").unwrap_or("")).ok_or_else(|| ApiError::bad_request("sort is likes, new, downloads or name"))?;
    if let Some(k) = get("kind") {
        if ecm_pkg::manifest::Kind::parse(k).is_none() {
            return Err(ApiError::bad_request("kind is app, program, library or source"));
        }
    }
    let page: usize = match get("page") {
        Some(p) => p.parse().ok().filter(|n| *n >= 1).ok_or_else(|| ApiError::bad_request("page is a number from 1"))?,
        None => 1,
    };
    let conn = app.db.lock().unwrap();
    let pkgs = ops::load(&conn, None)?;
    let marks = ops::marks(&conn, caller.as_ref().map(|c| c.user_id))?;
    let found = ops::search(&pkgs, &ops::Search { channel: get("channel"), q: get("q").unwrap_or(""), kind: get("kind"), sort });
    let shown: Vec<Value> =
        found.iter().skip((page - 1).saturating_mul(ops::PER_PAGE)).take(ops::PER_PAGE).map(|(p, v)| ops::summary(p, v, &marks)).collect();
    Ok(json(StatusCode::OK, &json!({ "total": found.len(), "page": page, "per_page": ops::PER_PAGE, "packages": shown })))
}

pub async fn package_detail(State(app): State<App>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, ApiError> {
    let caller = maybe_caller(&app, &headers).await;
    let conn = app.db.lock().unwrap();
    let pkgs = ops::load(&conn, Some(&id))?;
    let Some(p) = pkgs.first() else { return Err(ApiError::not_found(format!("no package {id}"))) };
    let marks = ops::marks(&conn, caller.as_ref().map(|c| c.user_id))?;
    Ok(json(StatusCode::OK, &ops::detail(p, &marks)))
}

#[derive(Clone, Copy)]
enum Mine {
    Owned,
    Favorites,
    Likes,
}

async fn mine(app: App, headers: HeaderMap, which: Mine) -> Result<Response, ApiError> {
    let caller = require(&app, &headers, None).await?;
    let conn = app.db.lock().unwrap();
    let pkgs = ops::load(&conn, None)?;
    let marks = ops::marks(&conn, Some(caller.user_id))?;
    let mut list: Vec<&ops::Pkg> = pkgs
        .iter()
        .filter(|p| match which {
            Mine::Owned => p.owner_id == caller.user_id,
            Mine::Favorites => marks.favorited.contains(&p.id),
            Mine::Likes => marks.liked.contains(&p.id),
        })
        .collect();
    list.sort_by(|a, b| a.id.cmp(&b.id));
    // An owner also sees packages whose versions are all yanked.
    let out: Vec<Value> = list
        .iter()
        .filter_map(|p| match which {
            Mine::Owned => Some((p, p.newest(None).unwrap_or(&p.versions[0]))),
            _ => p.newest(None).map(|v| (p, v)),
        })
        .map(|(p, v)| ops::summary(p, v, &marks))
        .collect();
    Ok(json(StatusCode::OK, &out))
}

pub async fn my_packages(State(app): State<App>, headers: HeaderMap) -> Result<Response, ApiError> {
    mine(app, headers, Mine::Owned).await
}

pub async fn my_favorites(State(app): State<App>, headers: HeaderMap) -> Result<Response, ApiError> {
    mine(app, headers, Mine::Favorites).await
}

pub async fn my_likes(State(app): State<App>, headers: HeaderMap) -> Result<Response, ApiError> {
    mine(app, headers, Mine::Likes).await
}

// ------------------------------------------------------------------ social

async fn set_mark(app: App, headers: HeaderMap, id: String, table: &'static str, on: bool) -> Result<Response, ApiError> {
    let caller = require(&app, &headers, Some("social")).await?;
    ops::mark(&app, caller.user_id, &id, table, on)?;
    Ok(no_content())
}

pub async fn like(State(app): State<App>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, ApiError> {
    set_mark(app, headers, id, "likes", true).await
}

pub async fn unlike(State(app): State<App>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, ApiError> {
    set_mark(app, headers, id, "likes", false).await
}

pub async fn favorite(State(app): State<App>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, ApiError> {
    set_mark(app, headers, id, "favorites", true).await
}

pub async fn unfavorite(State(app): State<App>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, ApiError> {
    set_mark(app, headers, id, "favorites", false).await
}

#[derive(Deserialize)]
pub struct ReportBody {
    reason: String,
}

pub async fn report(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<ReportBody>,
) -> Result<Response, ApiError> {
    let caller = require(&app, &headers, Some("social")).await?;
    if !app.limiter.hit(&format!("report:{}", caller.user_id), app.cfg.report_limit) {
        return Err(ApiError::rate_limited("too many reports; try again later"));
    }
    ops::report(&app, &caller, &id, &body.reason)?;
    Ok(no_content())
}

// ------------------------------------------------------------------ publishing

/// May this caller publish and yank? Scope `publish` and a verified email.
async fn publisher(app: &App, headers: &HeaderMap) -> Result<Caller, ApiError> {
    let caller = require(app, headers, Some("publish")).await?;
    if !caller.email_verified {
        return Err(ApiError::forbidden("confirm your email address before publishing (see your account page)"));
    }
    Ok(caller)
}

pub async fn publish(State(app): State<App>, req: Request) -> Result<Response, ApiError> {
    let (parts, body) = req.into_parts();
    let caller = publisher(&app, &parts.headers).await?;
    let max = app.cfg.max_package_bytes;
    let too_large = || ApiError::too_large(format!("a package is at most {max} bytes"));
    let declared = parts.headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > max) {
        return Err(too_large());
    }
    let bytes = axum::body::to_bytes(body, max as usize).await.map_err(|_| too_large())?;
    let text = |name: &str| parts.headers.get(name).map(|v| v.to_str().unwrap_or("").trim().to_string());
    let author = (text("x-ecm-author-key"), text("x-ecm-author-signature"));
    let published = ops::publish(&app, &caller, bytes.to_vec(), author).await?;
    Ok(json(StatusCode::CREATED, &published))
}

#[derive(Deserialize)]
pub struct YankBody {
    version: String,
}

pub async fn yank(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<YankBody>,
) -> Result<Response, ApiError> {
    let caller = publisher(&app, &headers).await?;
    ops::yank(&app, &caller, &id, &body.version)?;
    Ok(no_content())
}

// ------------------------------------------------------------------ admin

#[derive(Deserialize)]
pub struct ChannelMove {
    id: String,
    version: String,
    channel: String,
}

fn done() -> Response {
    json(StatusCode::OK, &json!({ "ok": true }))
}

pub async fn admin_promote(State(app): State<App>, headers: HeaderMap, JsonBody(b): JsonBody<ChannelMove>) -> Result<Response, ApiError> {
    require(&app, &headers, Some("admin")).await?;
    ops::promote(&app, &b.id, &b.version, &b.channel)?;
    Ok(done())
}

pub async fn admin_demote(State(app): State<App>, headers: HeaderMap, JsonBody(b): JsonBody<ChannelMove>) -> Result<Response, ApiError> {
    require(&app, &headers, Some("admin")).await?;
    ops::demote(&app, &b.id, &b.version, &b.channel)?;
    Ok(done())
}

#[derive(Deserialize)]
pub struct Takedown {
    id: String,
    #[serde(default)]
    version: Option<String>,
}

pub async fn admin_takedown(State(app): State<App>, headers: HeaderMap, JsonBody(b): JsonBody<Takedown>) -> Result<Response, ApiError> {
    require(&app, &headers, Some("admin")).await?;
    ops::takedown(&app, &b.id, b.version.as_deref()).await?;
    Ok(done())
}

#[derive(Deserialize)]
pub struct NewChannel {
    name: String,
    #[serde(default)]
    title: String,
}

pub async fn admin_channels(State(app): State<App>, headers: HeaderMap, JsonBody(b): JsonBody<NewChannel>) -> Result<Response, ApiError> {
    require(&app, &headers, Some("admin")).await?;
    let title = if b.title.trim().is_empty() { b.name.clone() } else { b.title };
    ops::create_channel(&app, &b.name, &title)?;
    Ok(json(StatusCode::CREATED, &json!({ "name": b.name, "title": title.trim() })))
}

pub async fn admin_reports(State(app): State<App>, headers: HeaderMap) -> Result<Response, ApiError> {
    require(&app, &headers, Some("admin")).await?;
    Ok(json(StatusCode::OK, &ops::reports(&app)?))
}
