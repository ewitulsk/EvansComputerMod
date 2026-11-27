//! The repository database. Indexes and the catalogue are built from it.

pub const MIGRATIONS: &[&str] = &["
CREATE TABLE channels (
    name       TEXT PRIMARY KEY,
    title      TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
INSERT INTO channels (name, title, created_at) VALUES
    ('stable', 'Stable', strftime('%s', 'now')),
    ('beta', 'Beta', strftime('%s', 'now')),
    ('community', 'Community', strftime('%s', 'now'));

-- A package id belongs to the account that first published it, for good:
-- a taken-down package keeps its row so the id cannot be claimed by someone else.
CREATE TABLE packages (
    id         TEXT PRIMARY KEY,
    owner_id   INTEGER NOT NULL,
    owner      TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    downloads  INTEGER NOT NULL DEFAULT 0,
    taken_down INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX packages_owner ON packages(owner_id);

-- Versions are immutable. `entry` is the signed index entry as JSON.
-- `removed`: taken down by an admin. `blob_deleted`: the file is gone from the store.
CREATE TABLE versions (
    pkg_id       TEXT NOT NULL REFERENCES packages(id),
    version      TEXT NOT NULL,
    file         TEXT NOT NULL UNIQUE,
    size         INTEGER NOT NULL,
    sha256       TEXT NOT NULL,
    published_at INTEGER NOT NULL,
    yanked_at    INTEGER,
    removed      INTEGER NOT NULL DEFAULT 0,
    blob_deleted INTEGER NOT NULL DEFAULT 0,
    uploader_id  INTEGER NOT NULL,
    entry        TEXT NOT NULL,
    PRIMARY KEY (pkg_id, version)
);
CREATE INDEX versions_uploader ON versions(uploader_id);

CREATE TABLE channel_versions (
    channel  TEXT NOT NULL REFERENCES channels(name),
    pkg_id   TEXT NOT NULL,
    version  TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (channel, pkg_id, version),
    FOREIGN KEY (pkg_id, version) REFERENCES versions(pkg_id, version)
);

CREATE TABLE likes (
    user_id    INTEGER NOT NULL,
    pkg_id     TEXT NOT NULL REFERENCES packages(id),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, pkg_id)
);
CREATE INDEX likes_pkg ON likes(pkg_id);

CREATE TABLE favorites (
    user_id    INTEGER NOT NULL,
    pkg_id     TEXT NOT NULL REFERENCES packages(id),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, pkg_id)
);

CREATE TABLE reports (
    id         INTEGER PRIMARY KEY,
    pkg_id     TEXT NOT NULL,
    user_id    INTEGER NOT NULL,
    handle     TEXT NOT NULL,
    reason     TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

-- `generation` goes up with every change that an index or the catalogue shows, so a
-- server notices changes made by `repo-api admin` in another process.
CREATE TABLE meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
INSERT INTO meta (key, value) VALUES ('generation', 0);
"];

/// Note that the indexes changed. Call inside the transaction that changes them.
pub fn bump(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute("UPDATE meta SET value = value + 1 WHERE key = 'generation'", [])?;
    Ok(())
}

pub fn generation(conn: &rusqlite::Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT value FROM meta WHERE key = 'generation'", [], |r| r.get(0))
}
