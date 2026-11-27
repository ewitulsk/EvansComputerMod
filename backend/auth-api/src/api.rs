//! The JSON API: the device flow, `whoami`, token revocation and the internal introspection call.

use crate::{db, App, GRANTABLE_SCOPES};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use backend_common::http::{bearer, client_ip, json, no_content, JsonBody};
use backend_common::{token, ApiError};
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;

#[derive(Deserialize)]
pub struct StartRequest {
    #[serde(default)]
    client_name: String,
    #[serde(default)]
    scopes: Option<Vec<String>>,
}

/// A name a person will read on the approval page: printable, at most 64 characters.
fn clean_name(s: &str, fallback: &str) -> String {
    let n: String = s.chars().filter(|c| !c.is_control()).take(64).collect();
    let n = n.trim();
    if n.is_empty() {
        fallback.into()
    } else {
        n.into()
    }
}

/// Requested scopes: only grantable ones, no duplicates, at least one.
pub fn check_scopes(asked: &[String]) -> Result<Vec<String>, ApiError> {
    let mut out: Vec<String> = Vec::new();
    for s in asked {
        if !GRANTABLE_SCOPES.contains(&s.as_str()) {
            return Err(ApiError::bad_request(format!("unknown scope '{}' (social, publish)", s.chars().take(32).collect::<String>())));
        }
        if !out.contains(s) {
            out.push(s.clone());
        }
    }
    if out.is_empty() {
        return Err(ApiError::bad_request("ask for at least one scope (social, publish)"));
    }
    Ok(out)
}

pub async fn device_start(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    JsonBody(req): JsonBody<StartRequest>,
) -> Result<Response, ApiError> {
    let ip = client_ip(&headers, peer, app.cfg.trust_proxy);
    if !app.limiter.hit(&format!("device:{ip}"), app.cfg.limits.device_start) {
        return Err(ApiError::rate_limited("too many device sign-ins from this address; try again in a minute"));
    }
    let scopes = match &req.scopes {
        Some(s) => check_scopes(s)?,
        None => GRANTABLE_SCOPES.iter().map(|s| s.to_string()).collect(),
    };
    let name = clean_name(&req.client_name, "a computer");
    let (device_code, user_code) = db::create_device_code(&app.db.lock().unwrap(), &name, &scopes)?;
    let uri = format!("{}/device", app.cfg.public_origin);
    Ok(json(
        StatusCode::OK,
        &json!({
            "device_code": device_code,
            "user_code": user_code,
            "verification_uri": uri,
            "verification_uri_complete": format!("{uri}?code={user_code}"),
            "interval": 3,
            "expires_in": db::DEVICE_SECONDS,
        }),
    ))
}

#[derive(Deserialize)]
pub struct TokenRequest {
    device_code: String,
}

pub async fn device_token(State(app): State<App>, JsonBody(req): JsonBody<TokenRequest>) -> Result<Response, ApiError> {
    let conn = app.db.lock().unwrap();
    let Some(d) = db::device_by_code(&conn, &req.device_code)? else {
        // Expired codes are deleted after an hour, so an unknown code is treated as an expired one.
        return Err(ApiError::gone("the device code is unknown or expired; start again"));
    };
    match d.status.as_str() {
        "denied" => Err(ApiError::forbidden("the sign-in was denied")),
        "consumed" => Err(ApiError::gone("this device code was already used")),
        _ if d.expires_at <= backend_common::now() => Err(ApiError::gone("the device code expired; start again")),
        "approved" => {
            let user = d.user_id.map(|id| db::user_by_id(&conn, id)).transpose()?.flatten();
            let Some(user) = user else { return Err(ApiError::internal("approved device code without a user")) };
            if !db::consume_device(&conn, &d.hash)? {
                return Err(ApiError::gone("this device code was already used"));
            }
            let token = db::create_token(&conn, user.id, "device", &d.client_name, &d.scopes)?;
            Ok(json(
                StatusCode::OK,
                &json!({ "token": token, "handle": user.handle, "scopes": app.effective_scopes(&user, &d.scopes) }),
            ))
        }
        _ => Err(ApiError::pending("waiting for the sign-in to be approved")),
    }
}

/// The API key or device token of a request, with its user.
fn authenticate(app: &App, headers: &HeaderMap) -> Result<(db::TokenRow, db::User), ApiError> {
    let presented = bearer(headers).ok_or_else(|| ApiError::unauthorized("send Authorization: Bearer <token>"))?;
    db::find_token(&app.db.lock().unwrap(), presented, &["key", "device"])?
        .ok_or_else(|| ApiError::unauthorized("the token is unknown or was revoked"))
}

pub async fn whoami(State(app): State<App>, headers: HeaderMap) -> Result<Response, ApiError> {
    let (t, user) = authenticate(&app, &headers)?;
    Ok(json(
        StatusCode::OK,
        &json!({
            "handle": user.handle,
            "email_verified": user.email_verified,
            "scopes": app.effective_scopes(&user, &t.scopes),
            "token_name": t.name,
        }),
    ))
}

pub async fn revoke(State(app): State<App>, headers: HeaderMap) -> Result<Response, ApiError> {
    let (t, user) = authenticate(&app, &headers)?;
    db::revoke_token(&app.db.lock().unwrap(), user.id, t.id)?;
    Ok(no_content())
}

#[derive(Deserialize)]
pub struct IntrospectRequest {
    token: String,
}

/// `POST /internal/introspect`: what `repo-api` asks about a bearer token. Not routed by the proxy.
pub async fn introspect(
    State(app): State<App>,
    headers: HeaderMap,
    JsonBody(req): JsonBody<IntrospectRequest>,
) -> Result<Response, ApiError> {
    let secret = headers.get("x-internal-secret").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !token::same(secret, &app.cfg.internal_secret) {
        return Err(ApiError::forbidden("wrong internal secret"));
    }
    let found = db::find_token(&app.db.lock().unwrap(), &req.token, &["key", "device"])?;
    let body = match found {
        Some((t, user)) => {
            let scopes = app.effective_scopes(&user, &t.scopes);
            json!({
                "active": true,
                "user_id": user.id,
                "handle": user.handle,
                "email_verified": user.email_verified,
                "admin": scopes.iter().any(|s| s == "admin"),
                "scopes": scopes,
            })
        }
        None => json!({ "active": false }),
    };
    Ok(json(StatusCode::OK, &body))
}
