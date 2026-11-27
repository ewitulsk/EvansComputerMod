//! Who is calling: bearer tokens are checked with `auth-api`'s internal
//! introspection call; a positive answer is cached for 30 seconds.

use backend_common::{token, ApiError};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const CACHE_FOR: Duration = Duration::from_secs(30);

/// The account behind a token.
#[derive(Debug, Clone, Deserialize)]
pub struct Caller {
    pub user_id: i64,
    pub handle: String,
    #[serde(default)]
    pub email_verified: bool,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub admin: bool,
}

impl Caller {
    pub fn has(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}

#[derive(Deserialize)]
struct Answer {
    active: bool,
    #[serde(flatten)]
    caller: Option<Caller>,
}

pub struct AuthClient {
    url: String,
    secret: String,
    http: reqwest::Client,
    cache: Mutex<HashMap<String, (Instant, Caller)>>,
}

impl AuthClient {
    pub fn new(auth_internal_url: &str, secret: &str) -> Result<AuthClient, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(AuthClient {
            url: format!("{auth_internal_url}/internal/introspect"),
            secret: secret.to_string(),
            http,
            cache: Mutex::default(),
        })
    }

    /// The account a token belongs to; `None` when it is unknown, revoked or expired.
    pub async fn introspect(&self, presented: &str) -> Result<Option<Caller>, ApiError> {
        let looks_right = presented.len() <= 128 && (presented.starts_with(token::API_KEY) || presented.starts_with(token::DEVICE));
        if !looks_right {
            return Ok(None);
        }
        let key = token::hash(presented);
        if let Some((at, caller)) = self.cache.lock().unwrap().get(&key) {
            if at.elapsed() < CACHE_FOR {
                return Ok(Some(caller.clone()));
            }
        }
        let resp = self
            .http
            .post(&self.url)
            .header("x-internal-secret", &self.secret)
            .json(&serde_json::json!({ "token": presented }))
            .send()
            .await
            .map_err(|e| ApiError::internal(format!("auth-api is unreachable: {}", e.without_url())))?;
        if !resp.status().is_success() {
            return Err(ApiError::internal(format!("auth-api introspection answered {}", resp.status())));
        }
        let answer: Answer = resp.json().await.map_err(|e| ApiError::internal(format!("auth-api introspection: {e}")))?;
        let caller = if answer.active { answer.caller } else { None };
        let mut cache = self.cache.lock().unwrap();
        if cache.len() > 10_000 {
            cache.retain(|_, (at, _)| at.elapsed() < CACHE_FOR);
        }
        match &caller {
            Some(c) => cache.insert(key, (Instant::now(), c.clone())),
            None => cache.remove(&key),
        };
        Ok(caller)
    }
}
