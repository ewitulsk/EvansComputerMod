//! `auth-api`: accounts, web sessions, API keys and the device flow (docs/REPO_API.md).

pub mod admin;
pub mod api;
pub mod db;
pub mod email;
pub mod web;

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use backend_common::ratelimit::{Limit, RateLimiter};
use backend_common::{env, ApiError};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};

/// The scopes a person can give a token. `admin` is never asked for: it comes from the account.
pub const GRANTABLE_SCOPES: [&str; 2] = ["social", "publish"];

#[derive(Debug, Clone)]
pub struct Limits {
    pub signin: Limit,
    pub signup: Limit,
    pub forgot: Limit,
    pub device_start: Limit,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            signin: Limit::per_minute(10),
            signup: Limit::per_hour(5),
            forgot: Limit::per_hour(5),
            device_start: Limit::per_minute(20),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub public_origin: String,
    pub listen: String,
    pub database_path: String,
    pub internal_secret: String,
    pub email_provider: String,
    pub email_api_key: Option<String>,
    pub email_from: String,
    pub admin_emails: Vec<String>,
    pub trust_proxy: bool,
    pub limits: Limits,
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        let internal_secret = env::required("INTERNAL_SECRET")?;
        if internal_secret.len() < 16 {
            return Err("INTERNAL_SECRET must be at least 16 characters".into());
        }
        Ok(Config {
            public_origin: env::origin("PUBLIC_ORIGIN", "http://localhost:8080")?,
            listen: env::var_or("LISTEN", "0.0.0.0:8081"),
            database_path: env::var_or("DATABASE_PATH", "/data/auth.db"),
            internal_secret,
            email_provider: env::var_or("EMAIL_PROVIDER", "log"),
            email_api_key: env::var("EMAIL_API_KEY"),
            email_from: env::var_or("EMAIL_FROM", "ECM packages <noreply@localhost>"),
            admin_emails: env::var_or("ADMIN_EMAILS", "")
                .split(',')
                .map(|e| e.trim().to_lowercase())
                .filter(|e| !e.is_empty())
                .collect(),
            trust_proxy: env::flag("TRUST_PROXY", false),
            limits: Limits::default(),
        })
    }
}

pub struct Inner {
    pub cfg: Config,
    pub db: Mutex<rusqlite::Connection>,
    pub mailer: Arc<dyn email::Mailer>,
    /// What the `log` mail provider sent.
    pub outbox: email::Outbox,
    pub limiter: RateLimiter,
}

pub type App = Arc<Inner>;

/// Open the database (running migrations) and set up the mailer.
pub fn build(cfg: Config) -> Result<App, String> {
    let db = backend_common::db::open(&cfg.database_path, db::MIGRATIONS)?;
    let (mailer, outbox) = email::mailer(&cfg.email_provider, cfg.email_api_key.as_deref())?;
    Ok(Arc::new(Inner { cfg, db: Mutex::new(db), mailer, outbox, limiter: RateLimiter::new() }))
}

impl Inner {
    /// Cookies are `Secure` when the site is served over https.
    pub fn secure(&self) -> bool {
        self.cfg.public_origin.starts_with("https://")
    }

    /// An account is an admin when the database says so or its verified email is in `ADMIN_EMAILS`.
    pub fn is_admin(&self, user: &db::User) -> bool {
        user.is_admin || (user.email_verified && self.cfg.admin_emails.iter().any(|e| *e == user.email))
    }

    /// A token's scopes as other services see them: an admin's tokens also carry `admin`.
    pub fn effective_scopes(&self, user: &db::User, token_scopes: &[String]) -> Vec<String> {
        let mut scopes = token_scopes.to_vec();
        if self.is_admin(user) && !scopes.iter().any(|s| s == "admin") {
            scopes.push("admin".into());
        }
        scopes
    }

    /// Send a mail in the background, so a response does not wait for (or reveal) the delivery.
    pub fn send_mail(self: &Arc<Self>, to: &str, subject: &str, text: String) {
        let app = self.clone();
        let mail = email::Mail { to: to.to_string(), subject: subject.to_string(), text };
        tokio::spawn(async move {
            if let Err(e) = app.mailer.send(&app.cfg.email_from, &mail).await {
                eprintln!("mail was not sent: {e}");
            }
        });
    }
}

/// Argon2id (the crate's default parameters: 19 MiB, 2 passes), off the async threads.
pub async fn hash_password(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default().hash_password(password.as_bytes(), &salt).map(|h| h.to_string())
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)
}

/// Check a password. Without a stored hash a dummy one is checked, so an unknown
/// account takes as long as a wrong password.
pub async fn verify_password(password: String, stored: Option<String>) -> bool {
    static DUMMY: OnceLock<String> = OnceLock::new();
    tokio::task::spawn_blocking(move || {
        let known = stored.is_some();
        let hash = stored.unwrap_or_else(|| {
            DUMMY
                .get_or_init(|| {
                    Argon2::default().hash_password(b"no such account", &SaltString::generate(&mut OsRng)).unwrap().to_string()
                })
                .clone()
        });
        let ok = PasswordHash::new(&hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok());
        ok && known
    })
    .await
    .unwrap_or(false)
}

pub fn router(app: App) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok\n" }))
        .route("/auth/v1/health", get(|| async { "ok\n" }))
        .route("/auth/v1/device/start", post(api::device_start))
        .route("/auth/v1/device/token", post(api::device_token))
        .route("/auth/v1/whoami", get(api::whoami))
        .route("/auth/v1/token/revoke", post(api::revoke))
        .route("/internal/introspect", post(api::introspect))
        .route("/signup", get(web::signup_page).post(web::signup))
        .route("/signin", get(web::signin_page).post(web::signin))
        .route("/signout", post(web::signout))
        .route("/verify", get(web::verify))
        .route("/forgot", get(web::forgot_page).post(web::forgot))
        .route("/reset", get(web::reset_page).post(web::reset))
        .route("/account", get(web::account_page).post(web::account))
        .route("/device", get(web::device_page).post(web::device))
        .route("/static/auth/style.css", get(|| async { backend_common::html::stylesheet() }))
        .fallback(web::not_found)
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(app)
}

/// Serve until the task is dropped or the process is told to stop.
pub async fn serve(app: App, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(listener, router(app).into_make_service_with_connect_info::<SocketAddr>()).await
}
