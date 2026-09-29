//! Per-computer storage, with the same virtual mount table as the Java
//! `ComputerInstance`: user storage (read-write) at the root, and the
//! system programs directory mounted read-only at `server-bin/`.
//! Paths are relative to the storage root without a leading slash.

use std::path::{Component, Path, PathBuf};

pub const SERVER_BIN: &str = "server-bin";
pub const MAX_WRITE: usize = 1024 * 1024;

pub struct Storage {
    pub root: PathBuf,
    pub bin: Option<PathBuf>,
}

/// Join a guest path under `base`, refusing anything that would escape it.
pub fn safe_join(base: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.trim_start_matches('/');
    let mut out = base.to_path_buf();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

impl Storage {
    pub fn new(root: PathBuf, bin: Option<PathBuf>) -> Self {
        let _ = std::fs::create_dir_all(&root);
        Storage { root, bin }
    }

    fn bin_rel(name: &str) -> Option<&str> {
        if name == SERVER_BIN {
            Some("")
        } else {
            name.strip_prefix("server-bin/")
        }
    }

    /// Read resolution: first mount where the file exists, else the first
    /// matching mount (for "not found").
    pub fn resolve_read(&self, name: &str) -> Option<(PathBuf, bool)> {
        let name = name.trim_start_matches('/');
        if name.is_empty() {
            return None;
        }
        let user = safe_join(&self.root, name);
        let bin = match (&self.bin, Self::bin_rel(name)) {
            (Some(b), Some(r)) => safe_join(b, r),
            _ => None,
        };
        if let Some(u) = &user {
            if u.exists() {
                return Some((u.clone(), false));
            }
        }
        if let Some(b) = &bin {
            if b.exists() {
                return Some((b.clone(), true));
            }
        }
        user.map(|u| (u, false)).or(bin.map(|b| (b, true)))
    }

    pub fn resolve_write(&self, name: &str) -> Option<PathBuf> {
        let name = name.trim_start_matches('/');
        if name.is_empty() {
            return None;
        }
        safe_join(&self.root, name)
    }

    pub fn write(&self, name: &str, data: &[u8]) -> i32 {
        if data.len() > MAX_WRITE {
            return -1;
        }
        let Some(p) = self.resolve_write(name) else { return -1 };
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&p, data) {
            Ok(()) => data.len() as i32,
            Err(_) => -1,
        }
    }

    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        let (p, _) = self.resolve_read(name)?;
        if p.is_dir() {
            return None;
        }
        std::fs::read(p).ok()
    }

    pub fn size(&self, name: &str) -> i32 {
        match self.resolve_read(name) {
            Some((p, _)) => std::fs::metadata(p).map(|m| m.len().min(i32::MAX as u64) as i32).unwrap_or(-1),
            None => -1,
        }
    }

    pub fn exists(&self, name: &str) -> bool {
        self.resolve_read(name).is_some_and(|(p, _)| p.exists())
    }

    pub fn is_dir(&self, name: &str) -> bool {
        let name = name.trim_start_matches('/');
        if name.is_empty() {
            return true;
        }
        self.resolve_read(name).is_some_and(|(p, _)| p.is_dir())
    }

    /// Merged "d:name"/"f:name" listing, sorted, newline-joined. None if no
    /// mount has such a directory.
    pub fn list_dir(&self, name: &str) -> Option<String> {
        let name = name.trim_start_matches('/');
        let mut seen = std::collections::BTreeSet::new();
        let mut found = false;
        let mut dirs: Vec<PathBuf> = Vec::new();
        if name.is_empty() {
            dirs.push(self.root.clone());
        } else if let Some(u) = safe_join(&self.root, name) {
            dirs.push(u);
        }
        if let (Some(b), Some(r)) = (&self.bin, Self::bin_rel(name)) {
            if let Some(p) = safe_join(b, r) {
                dirs.push(p);
            }
        }
        for d in dirs {
            if let Ok(rd) = std::fs::read_dir(&d) {
                found = true;
                for e in rd.flatten() {
                    let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    seen.insert(format!("{}{}", if is_dir { "d:" } else { "f:" }, e.file_name().to_string_lossy()));
                }
            }
        }
        if name.is_empty() && self.bin.is_some() {
            seen.insert(format!("d:{}", SERVER_BIN));
        }
        if !found && seen.is_empty() {
            return None;
        }
        Some(seen.into_iter().collect::<Vec<_>>().join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mounts_and_escape() {
        let base = std::env::temp_dir().join(format!("ecm-fs-test-{}", std::process::id()));
        let root = base.join("root");
        let bin = base.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("echo.wasm"), b"x").unwrap();
        let s = Storage::new(root.clone(), Some(bin.clone()));
        assert!(s.exists("server-bin/echo.wasm"));
        assert!(!s.exists("echo.wasm"));
        assert_eq!(s.write("a/b.txt", b"hi"), 2);
        assert_eq!(s.read("/a/b.txt").unwrap(), b"hi");
        assert!(s.resolve_write("../x").is_none());
        assert!(s.resolve_write("server-bin/../../x").is_none());
        let l = s.list_dir("").unwrap();
        assert!(l.contains("d:a") && l.contains("d:server-bin"), "{}", l);
        assert_eq!(s.list_dir("server-bin").unwrap(), "f:echo.wasm");
        assert!(s.is_dir("") && s.is_dir("a") && !s.is_dir("a/b.txt"));
        let _ = std::fs::remove_dir_all(base);
    }
}
