//! `repo-api`: channels served as `ecm-repo-1` repositories, the catalogue, publishing,
//! social and admin operations (docs/REPO_API.md).

pub mod admin;
pub mod auth;
pub mod db;
pub mod ops;
pub mod routes;
pub mod storage;
pub mod web;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post, put};
use axum::Router;
use backend_common::env;
use backend_common::ratelimit::{Limit, RateLimiter};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub enum StorageConfig {
    Fs { dir: String },
    S3(storage::S3Config),
}

#[derive(Debug, Clone)]
pub struct Config {
    pub public_origin: String,
    pub listen: String,
    pub database_path: String,
    pub internal_secret: String,
    pub auth_internal_url: String,
    pub storage: StorageConfig,
    /// 64 hex digits: the ed25519 seed the repository signs with.
    pub repo_signing_key: String,
    pub max_package_bytes: u64,
    pub account_quota_bytes: u64,
    pub trust_proxy: bool,
    pub upload_limit: Limit,
    pub report_limit: Limit,
}

/// `REPO_SIGNING_KEY` checked: (seed hex, public key hex).
pub fn signing_key(seed_hex: &str) -> Result<(String, String), String> {
    let seed = seed_hex.trim().to_ascii_lowercase();
    if seed.len() != 64 || !seed.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("REPO_SIGNING_KEY must be 64 hex digits (make one with `repo-api admin keygen`)".into());
    }
    let public = ecm_pkg::sign::public_of(&seed).ok_or("REPO_SIGNING_KEY is not a valid key")?;
    Ok((seed, public))
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        let storage = match env::var_or("STORAGE", "fs").as_str() {
            "fs" => StorageConfig::Fs { dir: env::var_or("STORAGE_DIR", "/data/blobs") },
            "s3" => StorageConfig::S3(storage::S3Config {
                endpoint: env::required("S3_ENDPOINT")?,
                region: env::var_or("S3_REGION", "us-east-1"),
                bucket: env::required("S3_BUCKET")?,
                access_key: env::required("S3_ACCESS_KEY")?,
                secret_key: env::required("S3_SECRET_KEY")?,
                path_style: match env::var_or("S3_PATH_STYLE", "auto").to_ascii_lowercase().as_str() {
                    "auto" => None,
                    "true" | "1" | "yes" => Some(true),
                    "false" | "0" | "no" => Some(false),
                    other => return Err(format!("S3_PATH_STYLE must be auto, true or false, not '{other}'")),
                },
            }),
            other => return Err(format!("STORAGE must be fs or s3, not '{other}'")),
        };
        let internal_secret = env::required("INTERNAL_SECRET")?;
        if internal_secret.len() < 16 {
            return Err("INTERNAL_SECRET must be at least 16 characters".into());
        }
        Ok(Config {
            public_origin: env::origin("PUBLIC_ORIGIN", "http://localhost:8080")?,
            listen: env::var_or("LISTEN", "0.0.0.0:8082"),
            database_path: env::var_or("DATABASE_PATH", "/data/repo.db"),
            internal_secret,
            auth_internal_url: env::var_or("AUTH_INTERNAL_URL", "http://auth-api:8081").trim_end_matches('/').to_string(),
            storage,
            repo_signing_key: signing_key(&env::required("REPO_SIGNING_KEY")?)?.0,
            max_package_bytes: env::number("MAX_PACKAGE_BYTES", 8 * 1024 * 1024)?,
            account_quota_bytes: env::number("ACCOUNT_QUOTA_BYTES", 200 * 1024 * 1024)?,
            trust_proxy: env::flag("TRUST_PROXY", false),
            upload_limit: Limit::per_hour(30),
            report_limit: Limit::per_hour(20),
        })
    }
}

pub struct Inner {
    pub cfg: Config,
    pub db: Mutex<rusqlite::Connection>,
    pub storage: Arc<dyn storage::Storage>,
    /// The repository key's public half: every index's `publisher_key`.
    pub public_key: String,
    pub limiter: RateLimiter,
    pub auth: auth::AuthClient,
    /// One publish, takedown or purge at a time (they touch the database and the blob store).
    pub write_lock: tokio::sync::Mutex<()>,
    /// Rendered indexes, valid for one generation of the database.
    pub index_cache: Mutex<(i64, HashMap<String, Arc<String>>)>,
}

pub type App = Arc<Inner>;

/// Open the database (running migrations) and the blob store.
pub fn build(cfg: Config) -> Result<App, String> {
    let (_, public_key) = signing_key(&cfg.repo_signing_key)?;
    let db = backend_common::db::open(&cfg.database_path, db::MIGRATIONS)?;
    let storage: Arc<dyn storage::Storage> = match &cfg.storage {
        StorageConfig::Fs { dir } => Arc::new(storage::FsStorage::new(dir)?),
        StorageConfig::S3(s3) => Arc::new(storage::S3Storage::new(s3.clone())?),
    };
    let auth = auth::AuthClient::new(&cfg.auth_internal_url, &cfg.internal_secret)?;
    Ok(Arc::new(Inner {
        cfg,
        db: Mutex::new(db),
        storage,
        public_key,
        limiter: RateLimiter::new(),
        auth,
        write_lock: tokio::sync::Mutex::new(()),
        index_cache: Mutex::new((-1, HashMap::new())),
    }))
}

pub fn router(app: App) -> Router {
    Router::new()
        .route("/healthz", get(routes::health))
        .route("/v1/health", get(routes::health))
        .route("/r/{channel}/{*rest}", get(routes::repository))
        .route("/v1/packages", get(routes::list_packages))
        .route("/v1/packages/{id}", get(routes::package_detail))
        .route("/v1/me/packages", get(routes::my_packages))
        .route("/v1/me/favorites", get(routes::my_favorites))
        .route("/v1/me/likes", get(routes::my_likes))
        .route("/v1/packages/{id}/like", put(routes::like).delete(routes::unlike))
        .route("/v1/packages/{id}/favorite", put(routes::favorite).delete(routes::unfavorite))
        .route("/v1/packages/{id}/report", post(routes::report))
        .route("/v1/packages/{id}/yank", post(routes::yank))
        // The publish handler enforces MAX_PACKAGE_BYTES itself, after the token check.
        .route("/v1/publish", put(routes::publish).layer(DefaultBodyLimit::disable()))
        .route("/v1/admin/promote", post(routes::admin_promote))
        .route("/v1/admin/demote", post(routes::admin_demote))
        .route("/v1/admin/takedown", post(routes::admin_takedown))
        .route("/v1/admin/channels", post(routes::admin_channels))
        .route("/v1/admin/reports", get(routes::admin_reports))
        .route("/", get(web::catalogue))
        .route("/static/repo/style.css", get(|| async { backend_common::html::stylesheet() }))
        .fallback(routes::not_found)
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(app)
}

/// Serve, and purge long-yanked files every hour, until the task is dropped.
pub async fn serve(app: App, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    let janitor = app.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = ops::purge_yanked(&janitor).await {
                eprintln!("purge of yanked files failed: {e}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });
    axum::serve(listener, router(app).into_make_service_with_connect_info::<SocketAddr>()).await
}
