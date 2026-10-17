//! The computer's files as ComputerCraft programs see them (`fs`): paths
//! from the computer's root, with or without a leading "/", "a/b" style.
//! Operations work on a real directory (`root`: "/" inside a computer, a
//! temporary folder in tests) through std::fs.
//!
//! Errors are worded as CC's are, so programs that check them still work:
//! "/x: No such file", "/x: Not a directory", "/x: File exists",
//! "/..: Invalid Path".

use std::path::{Path, PathBuf};

/// A path made canonical: no leading or trailing "/", no "." parts, ".."
/// applied. Err if it climbs above the root.
pub fn sanitize(path: &str) -> Result<String, String> {
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split(['/', '\\']) {
        match p {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!("/{}: Invalid Path", path.trim_start_matches('/')));
                }
            }
            other => parts.push(other),
        }
    }
    Ok(parts.join("/"))
}

/// `fs.combine`: the parts joined, then made canonical (".." above the root
/// stays as "..", as CC does).
pub fn combine(parts: &[&str]) -> String {
    let joined = parts.join("/");
    let mut out: Vec<&str> = Vec::new();
    for p in joined.split(['/', '\\']) {
        match p {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|l| *l != "..") {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out.join("/")
}

/// `fs.getName`: the last part ("root" for the root itself).
pub fn get_name(path: &str) -> String {
    let p = combine(&[path]);
    if p.is_empty() {
        return "root".into();
    }
    p.rsplit('/').next().unwrap_or("").to_string()
}

/// `fs.getDir`: everything but the last part ("" for a top-level name,
/// ".." for the root itself).
pub fn get_dir(path: &str) -> String {
    let p = combine(&[path]);
    if p.is_empty() {
        return "..".into();
    }
    match p.rsplit_once('/') {
        Some((dir, _)) => dir.to_string(),
        None => String::new(),
    }
}

/// Whether `name` matches a `fs.find` part: `*` any run of characters, `?` one.
fn wildcard(pattern: &[u8], name: &[u8]) -> bool {
    match (pattern.first(), name.first()) {
        (None, None) => true,
        (Some(b'*'), _) => wildcard(&pattern[1..], name) || (!name.is_empty() && wildcard(pattern, &name[1..])),
        (Some(b'?'), Some(_)) => wildcard(&pattern[1..], &name[1..]),
        (Some(p), Some(n)) if p == n => wildcard(&pattern[1..], &name[1..]),
        _ => false,
    }
}

/// Attributes of a file or directory (`fs.attributes`); times in ms since 1970.
#[derive(Clone, Debug, PartialEq)]
pub struct Attributes {
    pub size: u64,
    pub is_dir: bool,
    pub is_read_only: bool,
    pub created: i64,
    pub modified: i64,
}

/// The files of one computer, under `root`.
pub struct Files {
    pub root: PathBuf,
}

fn ms(t: std::io::Result<std::time::SystemTime>) -> i64 {
    t.ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl Files {
    pub fn new(root: impl Into<PathBuf>) -> Files {
        Files { root: root.into() }
    }

    /// The real path of a computer path (Err above the root).
    fn real(&self, path: &str) -> Result<(PathBuf, String), String> {
        let p = sanitize(path)?;
        let real = if p.is_empty() { self.root.clone() } else { self.root.join(&p) };
        Ok((real, p))
    }

    fn missing(p: &str) -> String {
        format!("/{p}: No such file")
    }

    pub fn exists(&self, path: &str) -> bool {
        self.real(path).map(|(r, _)| r.exists()).unwrap_or(false)
    }

    pub fn is_dir(&self, path: &str) -> bool {
        self.real(path).map(|(r, _)| r.is_dir()).unwrap_or(false)
    }

    pub fn is_read_only(&self, path: &str) -> bool {
        match self.real(path) {
            Ok((r, _)) => std::fs::metadata(&r).map(|m| m.permissions().readonly()).unwrap_or(false),
            Err(_) => true,
        }
    }

    /// Names in a directory, sorted.
    pub fn list(&self, path: &str) -> Result<Vec<String>, String> {
        let (r, p) = self.real(path)?;
        if !r.exists() {
            return Err(Self::missing(&p));
        }
        if !r.is_dir() {
            return Err(format!("/{p}: Not a directory"));
        }
        let mut names: Vec<String> = std::fs::read_dir(&r)
            .map_err(|e| format!("/{p}: {e}"))?
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect();
        names.sort();
        Ok(names)
    }

    pub fn size(&self, path: &str) -> Result<u64, String> {
        let (r, p) = self.real(path)?;
        let m = std::fs::metadata(&r).map_err(|_| Self::missing(&p))?;
        Ok(if m.is_dir() { 0 } else { m.len() })
    }

    pub fn attributes(&self, path: &str) -> Result<Attributes, String> {
        let (r, p) = self.real(path)?;
        let m = std::fs::metadata(&r).map_err(|_| Self::missing(&p))?;
        Ok(Attributes {
            size: if m.is_dir() { 0 } else { m.len() },
            is_dir: m.is_dir(),
            is_read_only: m.permissions().readonly(),
            created: ms(m.created()),
            modified: ms(m.modified()),
        })
    }

    /// Make a directory and its parents (fine if it is there already).
    pub fn make_dir(&self, path: &str) -> Result<(), String> {
        let (r, p) = self.real(path)?;
        if r.is_file() {
            return Err(format!("/{p}: File exists"));
        }
        std::fs::create_dir_all(&r).map_err(|e| format!("/{p}: {e}"))
    }

    /// Delete a file or a directory with everything in it (fine if it isn't there).
    pub fn delete(&self, path: &str) -> Result<(), String> {
        let (r, p) = self.real(path)?;
        if p.is_empty() {
            return Err("/: Access denied".into());
        }
        if r.is_dir() {
            std::fs::remove_dir_all(&r).map_err(|e| format!("/{p}: {e}"))
        } else if r.exists() {
            std::fs::remove_file(&r).map_err(|e| format!("/{p}: {e}"))
        } else {
            Ok(())
        }
    }

    fn check_target(&self, from: &str, to: &str) -> Result<(PathBuf, PathBuf), String> {
        let (rf, pf) = self.real(from)?;
        let (rt, pt) = self.real(to)?;
        if !rf.exists() {
            return Err(Self::missing(&pf));
        }
        if rt.exists() {
            return Err(format!("/{pt}: File exists"));
        }
        if pt == pf || pt.starts_with(&format!("{pf}/")) {
            return Err(format!("/{pt}: Can't copy a directory inside itself"));
        }
        if let Some(parent) = rt.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("/{pt}: {e}"))?;
        }
        Ok((rf, rt))
    }

    pub fn copy(&self, from: &str, to: &str) -> Result<(), String> {
        let (rf, rt) = self.check_target(from, to)?;
        copy_tree(&rf, &rt).map_err(|e| format!("/{}: {e}", sanitize(to).unwrap_or_default()))
    }

    pub fn move_(&self, from: &str, to: &str) -> Result<(), String> {
        let (rf, rt) = self.check_target(from, to)?;
        if std::fs::rename(&rf, &rt).is_err() {
            copy_tree(&rf, &rt).map_err(|e| e.to_string())?;
            if rf.is_dir() { std::fs::remove_dir_all(&rf) } else { std::fs::remove_file(&rf) }.map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// `fs.find`: the paths matching `pattern` (`*` and `?` within a part), sorted.
    pub fn find(&self, pattern: &str) -> Vec<String> {
        let Ok(p) = sanitize(pattern) else { return Vec::new() };
        let parts: Vec<&str> = if p.is_empty() { Vec::new() } else { p.split('/').collect() };
        let mut found = vec![String::new()];
        for part in parts {
            let mut next = Vec::new();
            for dir in &found {
                if part.contains('*') || part.contains('?') {
                    for name in self.list(dir).unwrap_or_default() {
                        if wildcard(part.as_bytes(), name.as_bytes()) {
                            next.push(if dir.is_empty() { name } else { format!("{dir}/{name}") });
                        }
                    }
                } else {
                    let candidate = if dir.is_empty() { part.to_string() } else { format!("{dir}/{part}") };
                    if self.exists(&candidate) {
                        next.push(candidate);
                    }
                }
            }
            found = next;
        }
        found.retain(|f| !f.is_empty());
        found.sort();
        found
    }

    /// Free bytes, if the computer limits its storage (None: unlimited).
    pub fn free_space(&self, _path: &str) -> Option<u64> {
        None
    }
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_tree(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_from_the_root() {
        assert_eq!(combine(&["a", "b/../c", "/d/"]), "a/c/d");
        assert_eq!(combine(&["..", "x"]), "../x");
        assert_eq!(get_name("/a/b.lua"), "b.lua");
        assert_eq!(get_name(""), "root");
        assert_eq!((get_dir("a/b"), get_dir("a"), get_dir("")), ("a".into(), "".into(), "..".into()));
        assert_eq!(sanitize("/x/./y/"), Ok("x/y".into()));
        assert!(sanitize("../etc").is_err());
        assert!(wildcard(b"*.lua", b"startup.lua") && wildcard(b"a?c", b"abc") && !wildcard(b"*.lua", b"x.py"));
    }

    #[test]
    fn files_list_copy_move_delete_find() {
        let d = std::env::temp_dir().join(format!("ecm-console-fs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = Files::new(&d);
        f.make_dir("/progs/lib").unwrap();
        std::fs::write(d.join("progs/a.lua"), "x").unwrap();
        std::fs::write(d.join("progs/lib/b.lua"), "yy").unwrap();
        assert_eq!(f.list("progs").unwrap(), ["a.lua", "lib"]);
        assert_eq!(f.list("nope").unwrap_err(), "/nope: No such file");
        assert_eq!(f.list("progs/a.lua").unwrap_err(), "/progs/a.lua: Not a directory");
        assert!(f.is_dir("progs") && !f.is_dir("progs/a.lua") && f.exists("/progs/lib/b.lua"));
        assert_eq!(f.size("progs/lib/b.lua"), Ok(2));
        let a = f.attributes("progs/lib").unwrap();
        assert!(a.is_dir && a.size == 0 && a.modified > 0);
        f.copy("progs", "backup/progs").unwrap();
        assert_eq!(f.find("backup/*/*.lua"), ["backup/progs/a.lua"]);
        assert_eq!(f.find("*/lib/?.lua"), ["progs/lib/b.lua"]);
        assert_eq!(f.copy("progs", "backup/progs").unwrap_err(), "/backup/progs: File exists");
        assert!(f.copy("progs", "progs/inside").is_err());
        f.move_("progs/a.lua", "moved.lua").unwrap();
        assert!(!f.exists("progs/a.lua") && f.exists("moved.lua"));
        f.delete("backup").unwrap();
        assert!(!f.exists("backup"));
        assert!(f.delete("").is_err() && f.list("../..").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
