//! SQLite: one file per service, WAL mode, migrations applied at startup.

use rusqlite::Connection;
use std::path::Path;

/// Open (and create) the database and apply the migrations it has not seen yet.
/// `migrations[i]` takes the schema from version `i` to `i + 1` (`PRAGMA user_version`).
pub fn open(path: &str, migrations: &[&str]) -> Result<Connection, String> {
    if let Some(dir) = Path::new(path).parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
    }
    let mut conn = Connection::open(path).map_err(|e| format!("{path}: {e}"))?;
    let run = |c: &mut Connection| -> rusqlite::Result<()> {
        c.pragma_update(None, "journal_mode", "WAL")?;
        c.pragma_update(None, "synchronous", "NORMAL")?;
        c.pragma_update(None, "foreign_keys", "ON")?;
        c.busy_timeout(std::time::Duration::from_secs(5))?;
        let have: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, sql) in migrations.iter().enumerate().skip(have as usize) {
            let tx = c.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
        }
        Ok(())
    };
    run(&mut conn).map_err(|e| format!("{path}: {e}"))?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    #[test]
    fn migrations_run_once_and_wal_is_on() {
        let dir = std::env::temp_dir().join(format!("ecm-db-test-{}", crate::token::random_hex(6)));
        let path = dir.join("sub/t.db");
        let path = path.to_str().unwrap();
        let m1 = ["CREATE TABLE a(x INTEGER); INSERT INTO a VALUES (1);"];
        drop(super::open(path, &m1).unwrap());
        let m2 = [m1[0], "CREATE TABLE b(y INTEGER);"];
        let c = super::open(path, &m2).unwrap();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM a", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        let mode: String = c.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
        let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 2);
        drop(c);
        let _ = std::fs::remove_dir_all(dir);
    }
}
