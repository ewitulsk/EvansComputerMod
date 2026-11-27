//! What the repository does, apart from HTTP: the routes and the `admin` command both call these.

use crate::auth::Caller;
use crate::{db, App};
use backend_common::{now, rfc3339, ApiError};
use ecm_pkg::manifest::compare_versions;
use ecm_pkg::package::{self, Index, IndexEntry, INDEX_FORMAT};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A yanked version stays downloadable by exact name this long.
pub const YANK_GRACE_SECONDS: i64 = 30 * 24 * 3600;
/// Ids under these prefixes are published by admins only.
pub const RESERVED_PREFIXES: [&str; 2] = ["dev.ecm", "com.evanwitulski"];
/// Every upload lands here.
pub const UPLOAD_CHANNEL: &str = "community";
pub const PER_PAGE: usize = 50;

pub fn reserved(id: &str) -> bool {
    RESERVED_PREFIXES.iter().any(|p| id == *p || id.strip_prefix(p).is_some_and(|rest| rest.starts_with('.')))
}

pub fn valid_channel(name: &str) -> bool {
    (1..=32).contains(&name.len()) && name.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn blob_key(id: &str, version: &str, file: &str) -> String {
    format!("packages/{id}/{version}/{file}")
}

/// Icons and screenshots are stored next to the package, so serving one does not read the archive.
fn member_key(id: &str, version: &str, member: &str) -> String {
    format!("packages/{id}/{version}/members/{member}")
}

fn members_of(entry: &IndexEntry) -> Vec<String> {
    let mut m: Vec<String> = entry.screenshots.clone();
    if !entry.icon.is_empty() && !m.contains(&entry.icon) {
        m.push(entry.icon.clone());
    }
    m
}

fn storage_error(e: String) -> ApiError {
    ApiError::internal(format!("storage: {e}"))
}

// ------------------------------------------------------------------ publishing

#[derive(Debug, Serialize)]
pub struct Published {
    pub id: String,
    pub version: String,
    pub kind: String,
    pub channel: String,
    pub file: String,
    pub sha256: String,
}

/// Publish an archive for `caller` (whose token, scope and verified email the caller of
/// this function has checked, as well as the size). The remaining checks, in the
/// contract's order: the archive opens, reserved ids, ownership, the version is new,
/// the author's signature, the account quota, the upload rate.
pub async fn publish(app: &App, caller: &Caller, bytes: Vec<u8>, author: (Option<String>, Option<String>)) -> Result<Published, ApiError> {
    let pkg = package::open(&bytes).map_err(|e| ApiError::bad_request(format!("not a valid package: {e}")))?;
    let (id, version) = (pkg.manifest.id.clone(), pkg.manifest.version.clone());
    if reserved(&id) && !caller.admin {
        return Err(ApiError::forbidden(format!("ids under {} are reserved", RESERVED_PREFIXES.join(".* and "))));
    }
    let file = package::file_name(&pkg.manifest);
    let size = bytes.len() as u64;

    let _writing = app.write_lock.lock().await;
    {
        let conn = app.db.lock().unwrap();
        let existing: Option<(i64, i64)> =
            conn.query_row("SELECT owner_id, taken_down FROM packages WHERE id = ?1", [&id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        match existing {
            Some((_, taken_down)) if taken_down != 0 => return Err(ApiError::forbidden(format!("{id} was taken down"))),
            Some((owner, _)) if owner != caller.user_id => return Err(ApiError::forbidden(format!("{id} belongs to another publisher"))),
            _ => {}
        }
        // The file name is checked too: `a.b` `1.0.0-1.0.0` and `a.b-1.0.0` `1.0.0` would share one.
        let taken: i64 = conn.query_row(
            "SELECT COUNT(*) FROM versions WHERE (pkg_id = ?1 AND version = ?2) OR file = ?3",
            params![id, version, file],
            |r| r.get(0),
        )?;
        if taken > 0 {
            return Err(ApiError::conflict(format!("{id} {version} was already published; versions are immutable")));
        }
    }
    let sha256 = pkg.sha256.clone();
    let message = package::signed_message(&id, &version, &sha256);
    let author = match author {
        (Some(key), Some(signature)) => {
            if !ecm_pkg::sign::verify(&key, &message, &signature) {
                return Err(ApiError::bad_request("the author signature does not verify for this package"));
            }
            Some((key, signature))
        }
        (None, None) => None,
        _ => return Err(ApiError::bad_request("send both X-Ecm-Author-Key and X-Ecm-Author-Signature, or neither")),
    };
    {
        let conn = app.db.lock().unwrap();
        let used: i64 = conn.query_row(
            "SELECT COALESCE(SUM(size), 0) FROM versions WHERE uploader_id = ?1 AND blob_deleted = 0",
            [caller.user_id],
            |r| r.get(0),
        )?;
        if used as u64 + size > app.cfg.account_quota_bytes {
            return Err(ApiError::too_large(format!(
                "this upload would put your account over its quota of {} bytes ({} in use)",
                app.cfg.account_quota_bytes, used
            )));
        }
    }
    if !app.limiter.hit(&format!("upload:{}", caller.user_id), app.cfg.upload_limit) {
        return Err(ApiError::rate_limited("too many uploads; try again later"));
    }

    let mut entry = package::index_entry(&pkg, &file, size, Some(ecm_pkg::sign::sign(&app.cfg.repo_signing_key, &message)));
    // The account's handle is the publisher people see, whatever the manifest says.
    entry.publisher = caller.handle.clone();
    if let Some((key, signature)) = author {
        entry.author_key = Some(key.to_ascii_lowercase());
        entry.author_signature = Some(signature.to_ascii_lowercase());
    }
    for member in members_of(&entry) {
        if let Some(data) = package::member(&pkg, &member) {
            app.storage.put(&member_key(&id, &version, &member), data.to_vec(), "image/png").await.map_err(storage_error)?;
        }
    }
    app.storage.put(&blob_key(&id, &version, &file), bytes, "application/octet-stream").await.map_err(storage_error)?;

    let entry_json = serde_json::to_string(&entry).map_err(ApiError::internal)?;
    let mut conn = app.db.lock().unwrap();
    let t = now();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO packages (id, owner_id, owner, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(id) DO UPDATE SET updated_at = ?4",
        params![id, caller.user_id, caller.handle, t],
    )?;
    tx.execute(
        "INSERT INTO versions (pkg_id, version, file, size, sha256, published_at, uploader_id, entry) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![id, version, file, size as i64, sha256, t, caller.user_id, entry_json],
    )?;
    tx.execute(
        "INSERT INTO channel_versions (channel, pkg_id, version, added_at) VALUES (?1, ?2, ?3, ?4)",
        params![UPLOAD_CHANNEL, id, version, t],
    )?;
    db::bump(&tx)?;
    tx.commit()?;
    Ok(Published { id, version, kind: pkg.manifest.kind.name().to_string(), channel: UPLOAD_CHANNEL.into(), file, sha256 })
}

/// Yank a version: it leaves every index. Its owner or an admin may.
pub fn yank(app: &App, caller: &Caller, id: &str, version: &str) -> Result<(), ApiError> {
    let conn = app.db.lock().unwrap();
    let owner: Option<i64> = conn.query_row("SELECT owner_id FROM packages WHERE id = ?1 AND taken_down = 0", [id], |r| r.get(0)).optional()?;
    let Some(owner) = owner else { return Err(ApiError::not_found(format!("no package {id}"))) };
    if owner != caller.user_id && !caller.admin {
        return Err(ApiError::forbidden(format!("{id} belongs to another publisher")));
    }
    let known: i64 =
        conn.query_row("SELECT COUNT(*) FROM versions WHERE pkg_id = ?1 AND version = ?2 AND removed = 0", params![id, version], |r| r.get(0))?;
    if known == 0 {
        return Err(ApiError::not_found(format!("{id} has no version {version}")));
    }
    conn.execute("UPDATE versions SET yanked_at = ?1 WHERE pkg_id = ?2 AND version = ?3 AND yanked_at IS NULL", params![now(), id, version])?;
    db::bump(&conn)?;
    Ok(())
}

/// Delete the files of versions yanked more than 30 days ago. Returns how many.
pub async fn purge_yanked(app: &App) -> Result<usize, ApiError> {
    let _writing = app.write_lock.lock().await;
    let old: Vec<(String, String, String, String)> = {
        let conn = app.db.lock().unwrap();
        let mut st = conn.prepare("SELECT pkg_id, version, file, entry FROM versions WHERE yanked_at IS NOT NULL AND yanked_at < ?1 AND blob_deleted = 0")?;
        let rows = st.query_map([now() - YANK_GRACE_SECONDS], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
        rows
    };
    for (id, version, file, entry) in &old {
        delete_blobs(app, id, version, file, entry).await?;
        app.db.lock().unwrap().execute("UPDATE versions SET blob_deleted = 1 WHERE pkg_id = ?1 AND version = ?2", params![id, version])?;
    }
    Ok(old.len())
}

async fn delete_blobs(app: &App, id: &str, version: &str, file: &str, entry_json: &str) -> Result<(), ApiError> {
    if let Ok(entry) = serde_json::from_str::<IndexEntry>(entry_json) {
        for member in members_of(&entry) {
            app.storage.delete(&member_key(id, version, &member)).await.map_err(storage_error)?;
        }
    }
    app.storage.delete(&blob_key(id, version, file)).await.map_err(storage_error)
}

// ------------------------------------------------------------------ admin

fn require_version(conn: &Connection, id: &str, version: &str) -> Result<bool, ApiError> {
    let row: Option<Option<i64>> = conn
        .query_row(
            "SELECT v.yanked_at FROM versions v JOIN packages p ON p.id = v.pkg_id
             WHERE v.pkg_id = ?1 AND v.version = ?2 AND v.removed = 0 AND p.taken_down = 0",
            params![id, version],
            |r| r.get(0),
        )
        .optional()?;
    match row {
        Some(yanked) => Ok(yanked.is_some()),
        None => Err(ApiError::not_found(format!("no package {id} {version}"))),
    }
}

fn require_channel(conn: &Connection, channel: &str) -> Result<String, ApiError> {
    conn.query_row("SELECT title FROM channels WHERE name = ?1", [channel], |r| r.get(0))
        .optional()?
        .ok_or_else(|| ApiError::not_found(format!("no channel {channel}")))
}

/// Put a published version into a channel.
pub fn promote(app: &App, id: &str, version: &str, channel: &str) -> Result<(), ApiError> {
    let conn = app.db.lock().unwrap();
    require_channel(&conn, channel)?;
    if require_version(&conn, id, version)? {
        return Err(ApiError::conflict(format!("{id} {version} was yanked")));
    }
    conn.execute(
        "INSERT OR IGNORE INTO channel_versions (channel, pkg_id, version, added_at) VALUES (?1, ?2, ?3, ?4)",
        params![channel, id, version, now()],
    )?;
    db::bump(&conn)?;
    Ok(())
}

/// Take a version out of a channel.
pub fn demote(app: &App, id: &str, version: &str, channel: &str) -> Result<(), ApiError> {
    let conn = app.db.lock().unwrap();
    require_channel(&conn, channel)?;
    require_version(&conn, id, version)?;
    conn.execute("DELETE FROM channel_versions WHERE channel = ?1 AND pkg_id = ?2 AND version = ?3", params![channel, id, version])?;
    db::bump(&conn)?;
    Ok(())
}

/// Remove a version, or with `version` = `None` the whole package: index entries and files.
/// The id stays with its owner and removed versions cannot be published again.
pub async fn takedown(app: &App, id: &str, version: Option<&str>) -> Result<(), ApiError> {
    let _writing = app.write_lock.lock().await;
    let doomed: Vec<(String, String, String)> = {
        let mut conn = app.db.lock().unwrap();
        let tx = conn.transaction()?;
        let known: i64 = tx.query_row("SELECT COUNT(*) FROM packages WHERE id = ?1", [id], |r| r.get(0))?;
        if known == 0 {
            return Err(ApiError::not_found(format!("no package {id}")));
        }
        let doomed: Vec<(String, String, String)> = {
            let mut st = tx.prepare(
                "SELECT version, file, entry FROM versions WHERE pkg_id = ?1 AND (?2 IS NULL OR version = ?2) AND (removed = 0 OR blob_deleted = 0)",
            )?;
            let rows = st.query_map(params![id, version], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
            rows
        };
        if let Some(v) = version {
            let n: i64 = tx.query_row("SELECT COUNT(*) FROM versions WHERE pkg_id = ?1 AND version = ?2", params![id, v], |r| r.get(0))?;
            if n == 0 {
                return Err(ApiError::not_found(format!("{id} has no version {v}")));
            }
        } else {
            tx.execute("UPDATE packages SET taken_down = 1 WHERE id = ?1", [id])?;
        }
        tx.execute("DELETE FROM channel_versions WHERE pkg_id = ?1 AND (?2 IS NULL OR version = ?2)", params![id, version])?;
        tx.execute("UPDATE versions SET removed = 1 WHERE pkg_id = ?1 AND (?2 IS NULL OR version = ?2)", params![id, version])?;
        db::bump(&tx)?;
        tx.commit()?;
        doomed
    };
    for (v, file, entry) in &doomed {
        delete_blobs(app, id, v, file, entry).await?;
        app.db.lock().unwrap().execute("UPDATE versions SET blob_deleted = 1 WHERE pkg_id = ?1 AND version = ?2", params![id, v])?;
    }
    Ok(())
}

pub fn create_channel(app: &App, name: &str, title: &str) -> Result<(), ApiError> {
    if !valid_channel(name) {
        return Err(ApiError::bad_request("a channel name is 1 to 32 of a-z, 0-9 and -"));
    }
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 64 || title.chars().any(|c| c.is_control()) {
        return Err(ApiError::bad_request("a channel title is 1 to 64 characters"));
    }
    let conn = app.db.lock().unwrap();
    let n = conn.execute("INSERT OR IGNORE INTO channels (name, title, created_at) VALUES (?1, ?2, ?3)", params![name, title, now()])?;
    if n == 0 {
        return Err(ApiError::conflict(format!("the channel {name} exists already")));
    }
    db::bump(&conn)?;
    Ok(())
}

/// Reports, newest first (at most 500).
pub fn reports(app: &App) -> Result<Vec<Value>, ApiError> {
    let conn = app.db.lock().unwrap();
    let mut st = conn.prepare("SELECT id, pkg_id, handle, reason, created_at FROM reports ORDER BY id DESC LIMIT 500")?;
    let rows = st
        .query_map([], |r| {
            Ok(json!({
                "report": r.get::<_, i64>(0)?,
                "id": r.get::<_, String>(1)?,
                "reporter": r.get::<_, String>(2)?,
                "reason": r.get::<_, String>(3)?,
                "created": rfc3339(r.get(4)?),
            }))
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

// ------------------------------------------------------------------ repositories

/// The channel's `index.json`: every live version, packages by id, newest version of each first.
pub fn index_json(app: &App, channel: &str) -> Result<Arc<String>, ApiError> {
    let conn = app.db.lock().unwrap();
    let generation = db::generation(&conn)?;
    {
        let mut cache = app.index_cache.lock().unwrap();
        if cache.0 != generation {
            *cache = (generation, HashMap::new());
        }
        if let Some(hit) = cache.1.get(channel) {
            return Ok(hit.clone());
        }
    }
    let title = require_channel(&conn, channel)?;
    let mut st = conn.prepare(
        "SELECT v.entry FROM channel_versions cv
         JOIN versions v ON v.pkg_id = cv.pkg_id AND v.version = cv.version
         JOIN packages p ON p.id = v.pkg_id
         WHERE cv.channel = ?1 AND v.yanked_at IS NULL AND v.removed = 0 AND p.taken_down = 0",
    )?;
    let mut packages: Vec<IndexEntry> = Vec::new();
    for row in st.query_map([channel], |r| r.get::<_, String>(0))? {
        packages.push(serde_json::from_str(&row?).map_err(ApiError::internal)?);
    }
    packages.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| compare_versions(&b.version, &a.version)));
    let index = Index { format: INDEX_FORMAT.into(), name: title, publisher_key: Some(app.public_key.clone()), packages };
    let text = Arc::new(serde_json::to_string(&index).map_err(ApiError::internal)?);
    app.index_cache.lock().unwrap().1.insert(channel.to_string(), text.clone());
    Ok(text)
}

struct Served {
    id: String,
    version: String,
    entry: IndexEntry,
}

/// The version behind `/r/<channel>/<file>`, when it may still be downloaded.
fn served(conn: &Connection, channel: &str, file: &str) -> Result<Served, ApiError> {
    let row: Option<(String, String, Option<i64>, i64, String)> = conn
        .query_row(
            "SELECT v.pkg_id, v.version, v.yanked_at, v.blob_deleted, v.entry FROM channel_versions cv
             JOIN versions v ON v.pkg_id = cv.pkg_id AND v.version = cv.version
             JOIN packages p ON p.id = v.pkg_id
             WHERE cv.channel = ?1 AND v.file = ?2 AND v.removed = 0 AND p.taken_down = 0",
            params![channel, file],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((id, version, yanked_at, blob_deleted, entry)) = row else {
        return Err(ApiError::not_found(format!("no {file} in {channel}")));
    };
    if blob_deleted != 0 || yanked_at.is_some_and(|t| t + YANK_GRACE_SECONDS <= now()) {
        return Err(ApiError::gone(format!("{file} was yanked")));
    }
    Ok(Served { id, version, entry: serde_json::from_str(&entry).map_err(ApiError::internal)? })
}

/// A package file; counts a download.
pub async fn download(app: &App, channel: &str, file: &str) -> Result<Vec<u8>, ApiError> {
    let s = served(&app.db.lock().unwrap(), channel, file)?;
    let bytes = app.storage.get(&blob_key(&s.id, &s.version, file)).await.map_err(storage_error)?;
    let Some(bytes) = bytes else { return Err(ApiError::internal(format!("{file} is in the database but not in the store"))) };
    app.db.lock().unwrap().execute("UPDATE packages SET downloads = downloads + 1 WHERE id = ?1", [&s.id])?;
    Ok(bytes)
}

/// An icon or screenshot of a package: only what its index entry lists.
pub async fn member(app: &App, channel: &str, file: &str, member: &str) -> Result<Vec<u8>, ApiError> {
    let s = served(&app.db.lock().unwrap(), channel, file)?;
    if !package::servable(&s.entry, member) {
        return Err(ApiError::not_found(format!("{file} does not offer {member}")));
    }
    app.storage
        .get(&member_key(&s.id, &s.version, member))
        .await
        .map_err(storage_error)?
        .ok_or_else(|| ApiError::not_found(format!("{file} does not offer {member}")))
}

// ------------------------------------------------------------------ catalogue

pub struct Ver {
    pub version: String,
    pub size: u64,
    pub sha256: String,
    pub published: i64,
    pub yanked: bool,
    pub entry: IndexEntry,
    pub channels: Vec<String>,
}

pub struct Pkg {
    pub id: String,
    pub owner: String,
    pub owner_id: i64,
    pub downloads: i64,
    pub likes: i64,
    /// Not taken down, newest first.
    pub versions: Vec<Ver>,
}

impl Pkg {
    /// The newest version that is not yanked (in `channel`, when one is given).
    pub fn newest(&self, channel: Option<&str>) -> Option<&Ver> {
        self.versions.iter().find(|v| !v.yanked && channel.is_none_or(|c| v.channels.iter().any(|x| x == c)))
    }

    /// The channels that list a live version.
    pub fn channels(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for v in self.versions.iter().filter(|v| !v.yanked) {
            for c in &v.channels {
                if !out.contains(c) {
                    out.push(c.clone());
                }
            }
        }
        out.sort();
        out
    }
}

pub fn kind_of(entry: &IndexEntry) -> &str {
    if entry.kind.is_empty() {
        "app"
    } else {
        &entry.kind
    }
}

/// Every package that is not taken down (or just `only`), with its versions.
pub fn load(conn: &Connection, only: Option<&str>) -> Result<Vec<Pkg>, ApiError> {
    let mut pkgs: Vec<Pkg> = {
        let mut st = conn.prepare(
            "SELECT p.id, p.owner, p.owner_id, p.downloads, (SELECT COUNT(*) FROM likes l WHERE l.pkg_id = p.id)
             FROM packages p WHERE p.taken_down = 0 AND (?1 IS NULL OR p.id = ?1)",
        )?;
        let rows = st
            .query_map([only], |r| {
                Ok(Pkg { id: r.get(0)?, owner: r.get(1)?, owner_id: r.get(2)?, downloads: r.get(3)?, likes: r.get(4)?, versions: Vec::new() })
            })?
            .collect::<Result<_, _>>()?;
        rows
    };
    let mut channels: HashMap<(String, String), Vec<String>> = HashMap::new();
    {
        let mut st = conn.prepare("SELECT pkg_id, version, channel FROM channel_versions WHERE (?1 IS NULL OR pkg_id = ?1) ORDER BY channel")?;
        for row in st.query_map([only], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
            let (id, version, channel) = row?;
            channels.entry((id, version)).or_default().push(channel);
        }
    }
    let by_id: HashMap<String, usize> = pkgs.iter().enumerate().map(|(i, p)| (p.id.clone(), i)).collect();
    {
        let mut st = conn.prepare(
            "SELECT pkg_id, version, size, sha256, published_at, yanked_at, entry FROM versions WHERE removed = 0 AND (?1 IS NULL OR pkg_id = ?1)",
        )?;
        let mut rows = st.query([only])?;
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            let Some(&i) = by_id.get(&id) else { continue };
            let version: String = r.get(1)?;
            let entry: IndexEntry = serde_json::from_str(&r.get::<_, String>(6)?).map_err(ApiError::internal)?;
            pkgs[i].versions.push(Ver {
                channels: channels.remove(&(id, version.clone())).unwrap_or_default(),
                version,
                size: r.get::<_, i64>(2)? as u64,
                sha256: r.get(3)?,
                published: r.get(4)?,
                yanked: r.get::<_, Option<i64>>(5)?.is_some(),
                entry,
            });
        }
    }
    for p in &mut pkgs {
        p.versions.sort_by(|a, b| compare_versions(&b.version, &a.version));
    }
    pkgs.retain(|p| !p.versions.is_empty());
    Ok(pkgs)
}

/// What a caller liked and favorited.
#[derive(Default)]
pub struct Marks {
    pub liked: HashSet<String>,
    pub favorited: HashSet<String>,
}

pub fn marks(conn: &Connection, user_id: Option<i64>) -> Result<Marks, ApiError> {
    let Some(user) = user_id else { return Ok(Marks::default()) };
    let ids = |table: &str| -> Result<HashSet<String>, ApiError> {
        let mut st = conn.prepare(&format!("SELECT pkg_id FROM {table} WHERE user_id = ?1"))?;
        let rows = st.query_map([user], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(rows)
    };
    Ok(Marks { liked: ids("likes")?, favorited: ids("favorites")? })
}

/// The contract's `Summary`, describing version `v` of `p`.
pub fn summary(p: &Pkg, v: &Ver, marks: &Marks) -> Value {
    json!({
        "id": p.id,
        "name": v.entry.name,
        "summary": v.entry.summary,
        "kind": kind_of(&v.entry),
        "version": v.version,
        "owner": p.owner,
        "channels": p.channels(),
        "likes": p.likes,
        "downloads": p.downloads,
        "updated": rfc3339(v.published),
        "liked": marks.liked.contains(&p.id),
        "favorited": marks.favorited.contains(&p.id),
    })
}

/// The contract's `Detail`: a `Summary` plus versions, permissions, dependencies and
/// where the icon and screenshots can be fetched (paths on this origin).
pub fn detail(p: &Pkg, marks: &Marks) -> Value {
    let v = p.newest(None).unwrap_or(&p.versions[0]);
    let asset = |member: &str| v.channels.first().map(|c| format!("/r/{c}/{}", package::asset_path(&v.entry, member)));
    let mut d = summary(p, v, marks);
    d["versions"] = p
        .versions
        .iter()
        .map(|x| json!({ "version": x.version, "size": x.size, "sha256": x.sha256, "published": rfc3339(x.published), "yanked": x.yanked }))
        .collect();
    d["permissions"] = json!(v.entry.permissions);
    d["depends"] = json!(v.entry.depends);
    d["icon"] = if v.entry.icon.is_empty() { Value::Null } else { json!(asset(&v.entry.icon)) };
    d["screenshots"] = json!(v.entry.screenshots.iter().filter_map(|s| asset(s)).collect::<Vec<_>>());
    d
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sort {
    Likes,
    New,
    Downloads,
    Name,
}

impl Sort {
    pub fn parse(s: &str) -> Option<Sort> {
        Some(match s {
            "" | "likes" => Sort::Likes,
            "new" => Sort::New,
            "downloads" => Sort::Downloads,
            "name" => Sort::Name,
            _ => return None,
        })
    }
}

pub struct Search<'a> {
    pub channel: Option<&'a str>,
    /// Every word must appear in the id, name, summary or owner (case-insensitive).
    pub q: &'a str,
    pub kind: Option<&'a str>,
    pub sort: Sort,
}

/// The packages a search finds, sorted: each with the version that represents it.
pub fn search<'a>(pkgs: &'a [Pkg], s: &Search) -> Vec<(&'a Pkg, &'a Ver)> {
    let words: Vec<String> = s.q.split_whitespace().map(str::to_lowercase).collect();
    let mut found: Vec<(&Pkg, &Ver)> = pkgs
        .iter()
        .filter_map(|p| p.newest(s.channel).map(|v| (p, v)))
        .filter(|(_, v)| s.kind.is_none_or(|k| kind_of(&v.entry) == k))
        .filter(|(p, v)| {
            let hay = format!("{} {} {} {}", p.id, v.entry.name, v.entry.summary, p.owner).to_lowercase();
            words.iter().all(|w| hay.contains(w))
        })
        .collect();
    let by_name = |a: &(&Pkg, &Ver), b: &(&Pkg, &Ver)| a.1.entry.name.to_lowercase().cmp(&b.1.entry.name.to_lowercase()).then_with(|| a.0.id.cmp(&b.0.id));
    match s.sort {
        Sort::Likes => found.sort_by(|a, b| b.0.likes.cmp(&a.0.likes).then_with(|| by_name(a, b))),
        Sort::Downloads => found.sort_by(|a, b| b.0.downloads.cmp(&a.0.downloads).then_with(|| by_name(a, b))),
        Sort::New => found.sort_by(|a, b| b.1.published.cmp(&a.1.published).then_with(|| by_name(a, b))),
        Sort::Name => found.sort_by(by_name),
    }
    found
}

// ------------------------------------------------------------------ social

fn require_package(conn: &Connection, id: &str) -> Result<(), ApiError> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM packages WHERE id = ?1 AND taken_down = 0", [id], |r| r.get(0))?;
    if n == 0 {
        return Err(ApiError::not_found(format!("no package {id}")));
    }
    Ok(())
}

/// Set or clear a like (`table` = `likes`) or a favorite (`favorites`). Idempotent.
pub fn mark(app: &App, user_id: i64, id: &str, table: &'static str, on: bool) -> Result<(), ApiError> {
    let conn = app.db.lock().unwrap();
    require_package(&conn, id)?;
    if on {
        conn.execute(&format!("INSERT OR IGNORE INTO {table} (user_id, pkg_id, created_at) VALUES (?1, ?2, ?3)"), params![user_id, id, now()])?;
    } else {
        conn.execute(&format!("DELETE FROM {table} WHERE user_id = ?1 AND pkg_id = ?2"), params![user_id, id])?;
    }
    Ok(())
}

pub fn report(app: &App, caller: &Caller, id: &str, reason: &str) -> Result<(), ApiError> {
    let reason = reason.trim();
    if reason.is_empty() || reason.chars().count() > 1000 {
        return Err(ApiError::bad_request("give a reason of at most 1000 characters"));
    }
    let conn = app.db.lock().unwrap();
    require_package(&conn, id)?;
    conn.execute(
        "INSERT INTO reports (pkg_id, user_id, handle, reason, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, caller.user_id, caller.handle, reason, now()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn reserved_ids_and_channel_names() {
        for id in ["dev.ecm", "dev.ecm.chat", "com.evanwitulski.x"] {
            assert!(super::reserved(id), "{id}");
        }
        for id in ["dev.ecmx.chat", "dev.ecm-tools.x", "com.evanwitulskis.x", "org.dev.ecm.x"] {
            assert!(!super::reserved(id), "{id}");
        }
        assert!(super::valid_channel("nightly-2") && !super::valid_channel("") && !super::valid_channel("Beta") && !super::valid_channel(&"a".repeat(33)));
    }
}
