//! The accounts database.

use backend_common::{now, token, ApiError};
use rusqlite::{params, Connection, OptionalExtension};

pub const MIGRATIONS: &[&str] = &["
CREATE TABLE users (
    id             INTEGER PRIMARY KEY,
    email          TEXT NOT NULL UNIQUE,
    handle         TEXT NOT NULL UNIQUE,
    password_hash  TEXT NOT NULL,
    email_verified INTEGER NOT NULL DEFAULT 0,
    is_admin       INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL
);
-- API keys (kind 'key'), device tokens ('device') and web sessions ('session').
CREATE TABLE tokens (
    id           INTEGER PRIMARY KEY,
    user_id      INTEGER NOT NULL REFERENCES users(id),
    kind         TEXT NOT NULL,
    hash         TEXT NOT NULL UNIQUE,
    name         TEXT NOT NULL,
    scopes       TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER,
    expires_at   INTEGER,
    revoked_at   INTEGER
);
CREATE INDEX tokens_user ON tokens(user_id);
-- Links sent by email: purpose 'verify' or 'reset'.
CREATE TABLE email_tokens (
    hash       TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id),
    purpose    TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at    INTEGER
);
-- The device flow: status 'pending', 'approved', 'denied' or 'consumed'.
CREATE TABLE device_codes (
    hash        TEXT PRIMARY KEY,
    user_code   TEXT NOT NULL UNIQUE,
    client_name TEXT NOT NULL,
    scopes      TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'pending',
    user_id     INTEGER REFERENCES users(id),
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL
);
"];

pub const SESSION_SECONDS: i64 = 30 * 24 * 3600;
pub const VERIFY_SECONDS: i64 = 24 * 3600;
pub const RESET_SECONDS: i64 = 3600;
pub const DEVICE_SECONDS: i64 = 600;

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub handle: String,
    pub password_hash: String,
    pub email_verified: bool,
    pub is_admin: bool,
}

#[derive(Debug, Clone)]
pub struct TokenRow {
    pub id: i64,
    pub user_id: i64,
    pub kind: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct DeviceCode {
    pub hash: String,
    pub user_code: String,
    pub client_name: String,
    pub scopes: Vec<String>,
    pub status: String,
    pub user_id: Option<i64>,
    pub expires_at: i64,
}

fn split(scopes: &str) -> Vec<String> {
    scopes.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect()
}

const USER_COLS: &str = "id, email, handle, password_hash, email_verified, is_admin";

fn user_row(r: &rusqlite::Row) -> rusqlite::Result<User> {
    Ok(User {
        id: r.get(0)?,
        email: r.get(1)?,
        handle: r.get(2)?,
        password_hash: r.get(3)?,
        email_verified: r.get::<_, i64>(4)? != 0,
        is_admin: r.get::<_, i64>(5)? != 0,
    })
}

pub fn user_by_email(c: &Connection, email: &str) -> rusqlite::Result<Option<User>> {
    c.query_row(&format!("SELECT {USER_COLS} FROM users WHERE email = ?1"), [email], user_row).optional()
}

pub fn user_by_handle(c: &Connection, handle: &str) -> rusqlite::Result<Option<User>> {
    c.query_row(&format!("SELECT {USER_COLS} FROM users WHERE handle = ?1"), [handle], user_row).optional()
}

pub fn list_users(c: &Connection) -> rusqlite::Result<Vec<User>> {
    let mut st = c.prepare(&format!("SELECT {USER_COLS} FROM users ORDER BY handle"))?;
    let rows = st.query_map([], user_row)?.collect();
    rows
}

pub fn user_by_id(c: &Connection, id: i64) -> rusqlite::Result<Option<User>> {
    c.query_row(&format!("SELECT {USER_COLS} FROM users WHERE id = ?1"), [id], user_row).optional()
}

/// Create an account. `Ok(None)` when the email or the handle is taken.
pub fn create_user(c: &Connection, email: &str, handle: &str, password_hash: &str) -> rusqlite::Result<Option<i64>> {
    let r = c.execute(
        "INSERT INTO users (email, handle, password_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![email, handle, password_hash, now()],
    );
    match r {
        Ok(_) => Ok(Some(c.last_insert_rowid())),
        Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn set_password(c: &Connection, user_id: i64, password_hash: &str) -> rusqlite::Result<()> {
    c.execute("UPDATE users SET password_hash = ?1 WHERE id = ?2", params![password_hash, user_id])?;
    Ok(())
}

/// Store a new token (only its hash) and return the token itself, to be shown once.
pub fn create_token(c: &Connection, user_id: i64, kind: &str, name: &str, scopes: &[String]) -> rusqlite::Result<String> {
    let (prefix, expires) = match kind {
        "key" => (token::API_KEY, None),
        "device" => (token::DEVICE, None),
        _ => (token::SESSION, Some(now() + SESSION_SECONDS)),
    };
    let t = token::generate(prefix);
    c.execute(
        "INSERT INTO tokens (user_id, kind, hash, name, scopes, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![user_id, kind, token::hash(&t), name, scopes.join(","), now(), expires],
    )?;
    Ok(t)
}

/// The live token row and its user for a presented token. `kinds` limits what is accepted.
pub fn find_token(c: &Connection, presented: &str, kinds: &[&str]) -> rusqlite::Result<Option<(TokenRow, User)>> {
    let row = c
        .query_row(
            "SELECT id, user_id, kind, name, scopes, created_at, last_used_at FROM tokens
             WHERE hash = ?1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > ?2)",
            params![token::hash(presented), now()],
            token_row,
        )
        .optional()?;
    let Some(row) = row else { return Ok(None) };
    if !kinds.contains(&row.kind.as_str()) {
        return Ok(None);
    }
    // Remember the use, at most once a minute.
    if row.last_used_at.is_none_or(|t| now() - t >= 60) {
        c.execute("UPDATE tokens SET last_used_at = ?1 WHERE id = ?2", params![now(), row.id])?;
    }
    Ok(user_by_id(c, row.user_id)?.map(|u| (row, u)))
}

fn token_row(r: &rusqlite::Row) -> rusqlite::Result<TokenRow> {
    Ok(TokenRow {
        id: r.get(0)?,
        user_id: r.get(1)?,
        kind: r.get(2)?,
        name: r.get(3)?,
        scopes: split(&r.get::<_, String>(4)?),
        created_at: r.get(5)?,
        last_used_at: r.get(6)?,
    })
}

/// A user's live API keys and device tokens, newest first.
pub fn list_tokens(c: &Connection, user_id: i64) -> rusqlite::Result<Vec<TokenRow>> {
    let mut st = c.prepare(
        "SELECT id, user_id, kind, name, scopes, created_at, last_used_at FROM tokens
         WHERE user_id = ?1 AND kind IN ('key', 'device') AND revoked_at IS NULL ORDER BY created_at DESC, id DESC",
    )?;
    let rows = st.query_map([user_id], token_row)?.collect();
    rows
}

pub fn revoke_token(c: &Connection, user_id: i64, token_id: i64) -> rusqlite::Result<bool> {
    let n = c.execute(
        "UPDATE tokens SET revoked_at = ?1 WHERE id = ?2 AND user_id = ?3 AND revoked_at IS NULL",
        params![now(), token_id, user_id],
    )?;
    Ok(n > 0)
}

/// End a user's web sessions, except the one with id `keep`.
pub fn revoke_sessions(c: &Connection, user_id: i64, keep: Option<i64>) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE tokens SET revoked_at = ?1 WHERE user_id = ?2 AND kind = 'session' AND revoked_at IS NULL AND id IS NOT ?3",
        params![now(), user_id, keep],
    )?;
    Ok(())
}

/// A new emailed link token for `purpose`; earlier unused ones of that purpose stop working.
pub fn create_email_token(c: &Connection, user_id: i64, purpose: &str, lifetime: i64) -> rusqlite::Result<String> {
    c.execute("DELETE FROM email_tokens WHERE user_id = ?1 AND purpose = ?2 AND used_at IS NULL", params![user_id, purpose])?;
    c.execute("DELETE FROM email_tokens WHERE expires_at < ?1", [now() - 7 * 24 * 3600])?;
    let t = token::random_hex(32);
    c.execute(
        "INSERT INTO email_tokens (hash, user_id, purpose, expires_at) VALUES (?1, ?2, ?3, ?4)",
        params![token::hash(&t), user_id, purpose, now() + lifetime],
    )?;
    Ok(t)
}

/// The user an unused, unexpired link token belongs to.
pub fn peek_email_token(c: &Connection, presented: &str, purpose: &str) -> rusqlite::Result<Option<i64>> {
    c.query_row(
        "SELECT user_id FROM email_tokens WHERE hash = ?1 AND purpose = ?2 AND used_at IS NULL AND expires_at > ?3",
        params![token::hash(presented), purpose, now()],
        |r| r.get(0),
    )
    .optional()
}

/// Use up a link token. The user it belonged to, when it was still good.
pub fn take_email_token(c: &Connection, presented: &str, purpose: &str) -> rusqlite::Result<Option<i64>> {
    let Some(user) = peek_email_token(c, presented, purpose)? else { return Ok(None) };
    c.execute("UPDATE email_tokens SET used_at = ?1 WHERE hash = ?2", params![now(), token::hash(presented)])?;
    Ok(Some(user))
}

/// Unambiguous letters and digits for the code a person types.
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ23456789";

/// Start a device authorization: (device code, user code `ABCD-EFGH`).
pub fn create_device_code(c: &Connection, client_name: &str, scopes: &[String]) -> Result<(String, String), ApiError> {
    c.execute("DELETE FROM device_codes WHERE expires_at < ?1", [now() - 3600])?;
    for _ in 0..5 {
        let device = format!("ecm_dc_{}", token::random_hex(32));
        let raw = token::random_from(USER_CODE_ALPHABET, 8);
        let user_code = format!("{}-{}", &raw[..4], &raw[4..]);
        let r = c.execute(
            "INSERT INTO device_codes (hash, user_code, client_name, scopes, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![token::hash(&device), user_code, client_name, scopes.join(","), now(), now() + DEVICE_SECONDS],
        );
        match r {
            Ok(_) => return Ok((device, user_code)),
            Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(ApiError::internal("no free user code"))
}

const DEVICE_COLS: &str = "hash, user_code, client_name, scopes, status, user_id, expires_at";

fn device_row(r: &rusqlite::Row) -> rusqlite::Result<DeviceCode> {
    Ok(DeviceCode {
        hash: r.get(0)?,
        user_code: r.get(1)?,
        client_name: r.get(2)?,
        scopes: split(&r.get::<_, String>(3)?),
        status: r.get(4)?,
        user_id: r.get(5)?,
        expires_at: r.get(6)?,
    })
}

pub fn device_by_code(c: &Connection, device_code: &str) -> rusqlite::Result<Option<DeviceCode>> {
    c.query_row(&format!("SELECT {DEVICE_COLS} FROM device_codes WHERE hash = ?1"), [token::hash(device_code)], device_row)
        .optional()
}

/// The pending, unexpired authorization a person typed the code of.
pub fn pending_device(c: &Connection, user_code: &str) -> rusqlite::Result<Option<DeviceCode>> {
    c.query_row(
        &format!("SELECT {DEVICE_COLS} FROM device_codes WHERE user_code = ?1 AND status = 'pending' AND expires_at > ?2"),
        params![user_code, now()],
        device_row,
    )
    .optional()
}

/// Approve or deny a pending authorization. False when it is no longer pending.
pub fn decide_device(c: &Connection, user_code: &str, user_id: i64, approve: bool) -> rusqlite::Result<bool> {
    let n = c.execute(
        "UPDATE device_codes SET status = ?1, user_id = ?2 WHERE user_code = ?3 AND status = 'pending' AND expires_at > ?4",
        params![if approve { "approved" } else { "denied" }, user_id, user_code, now()],
    )?;
    Ok(n > 0)
}

/// Mark an approved authorization as used. False when another poll got there first.
pub fn consume_device(c: &Connection, hash: &str) -> rusqlite::Result<bool> {
    Ok(c.execute("UPDATE device_codes SET status = 'consumed' WHERE hash = ?1 AND status = 'approved'", [hash])? > 0)
}
