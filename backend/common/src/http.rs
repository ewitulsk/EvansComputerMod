//! Request helpers: JSON bodies with API-shaped errors, bearer tokens, cookies, the client's address.

use crate::ApiError;
use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use std::net::{IpAddr, SocketAddr};

/// A JSON request body. Unlike `axum::Json`, every refusal is an [`ApiError`]
/// (the client only understands that shape), and no `Content-Type` is required.
pub struct JsonBody<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for JsonBody<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        let bytes = Bytes::from_request(req, state).await.map_err(|e| {
            if e.into_response().status() == StatusCode::PAYLOAD_TOO_LARGE {
                ApiError::too_large("the request body is too large")
            } else {
                ApiError::bad_request("the request body could not be read")
            }
        })?;
        serde_json::from_slice(&bytes).map(JsonBody).map_err(|e| ApiError::bad_request(format!("bad JSON body: {e}")))
    }
}

/// A JSON response with a status.
pub fn json(status: StatusCode, value: &impl serde::Serialize) -> Response {
    match serde_json::to_vec(value) {
        Ok(body) => (status, [(header::CONTENT_TYPE, "application/json")], body).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// 204. It has no body and, as HTTP requires (RFC 9110 section 8.6) and hyper enforces,
/// no `Content-Length`; the in-game client asks for `Connection: close` and reads to the end.
pub fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

/// The token of `Authorization: Bearer <token>`.
pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// A cookie's value.
pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == name).then_some(v)
    })
}

/// The address a request came from. `X-Forwarded-For` is only believed when the
/// service runs behind the proxy (`TRUST_PROXY=true`); then its last entry counts,
/// which is the one the proxy itself wrote.
pub fn client_ip(headers: &HeaderMap, peer: SocketAddr, trust_proxy: bool) -> IpAddr {
    if trust_proxy {
        let forwarded = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse::<IpAddr>().ok());
        if let Some(ip) = forwarded {
            return ip;
        }
    }
    peer.ip()
}

/// `<binary> healthcheck`: ask the service on this machine for `/healthz`. For container
/// health checks, so the image needs no curl. `listen` is the service's `LISTEN` value.
pub fn healthcheck(listen: &str) -> bool {
    use std::io::{Read, Write};
    let port = listen.rsplit(':').next().unwrap_or("");
    let timeout = Some(std::time::Duration::from_secs(3));
    let Ok(mut s) = std::net::TcpStream::connect(format!("127.0.0.1:{port}")) else { return false };
    let _ = (s.set_read_timeout(timeout), s.set_write_timeout(timeout));
    if s.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n").is_err() {
        return false;
    }
    let mut head = [0u8; 12];
    s.read_exact(&mut head).is_ok() && head.ends_with(b" 200")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn headers() {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer abc"));
        h.insert(header::COOKIE, HeaderValue::from_static("a=1; ecm_session=tok; b=2"));
        h.insert("x-forwarded-for", HeaderValue::from_static("6.6.6.6, 10.1.2.3"));
        assert_eq!(bearer(&h), Some("abc"));
        assert_eq!(cookie(&h, "ecm_session"), Some("tok"));
        assert_eq!(cookie(&h, "session"), None);
        let peer: SocketAddr = "127.0.0.1:9".parse().unwrap();
        assert_eq!(client_ip(&h, peer, false).to_string(), "127.0.0.1");
        assert_eq!(client_ip(&h, peer, true).to_string(), "10.1.2.3");
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        assert_eq!(bearer(&h), None);
    }
}
