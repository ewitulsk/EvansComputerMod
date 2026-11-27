//! The API's error shape: `{"error":"<code>","message":"<text>"}` with a fixed status per code.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

macro_rules! ctor {
    ($name:ident, $status:expr, $code:literal) => {
        pub fn $name(message: impl Into<String>) -> ApiError {
            ApiError { status: $status, code: $code, message: message.into() }
        }
    };
}

impl ApiError {
    ctor!(bad_request, StatusCode::BAD_REQUEST, "bad_request");
    ctor!(unauthorized, StatusCode::UNAUTHORIZED, "unauthorized");
    ctor!(forbidden, StatusCode::FORBIDDEN, "forbidden");
    ctor!(not_found, StatusCode::NOT_FOUND, "not_found");
    ctor!(conflict, StatusCode::CONFLICT, "conflict");
    ctor!(gone, StatusCode::GONE, "gone");
    ctor!(too_large, StatusCode::PAYLOAD_TOO_LARGE, "too_large");
    ctor!(pending, StatusCode::PRECONDITION_REQUIRED, "pending");
    ctor!(rate_limited, StatusCode::TOO_MANY_REQUESTS, "rate_limited");

    /// A failure that is ours. The detail goes to the log, not to the caller.
    pub fn internal(detail: impl std::fmt::Display) -> ApiError {
        eprintln!("internal error: {detail}");
        ApiError { status: StatusCode::INTERNAL_SERVER_ERROR, code: "internal", message: "internal error".into() }
    }

    pub fn body(&self) -> String {
        serde_json::json!({ "error": self.code, "message": self.message }).to_string()
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, [(header::CONTENT_TYPE, "application/json")], self.body()).into_response()
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        ApiError::internal(format!("sqlite: {e}"))
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
