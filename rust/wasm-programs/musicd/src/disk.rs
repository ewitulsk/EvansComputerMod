//! The files musicd reads and writes, behind a trait so the request handlers
//! run against an in-memory disk in host tests.

use ecm_music::library::FileStat;
use std::collections::BTreeMap;

pub trait Disk {
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    /// Up to `len` bytes from `start`.
    fn read_range(&self, path: &str, start: u64, len: usize) -> Option<Vec<u8>>;
    fn size(&self, path: &str) -> Option<u64>;
    /// Write a file, making its folders.
    fn write(&mut self, path: &str, data: &[u8]) -> bool;
    fn remove(&mut self, path: &str) -> bool;
    /// Every file under `dir` (recursively), with paths relative to it.
    fn list(&self, dir: &str) -> Vec<FileStat>;

    fn read_text(&self, path: &str) -> Option<String> {
        self.read(path).map(|b| String::from_utf8_lossy(&b).into_owned())
    }
}

/// Files in memory (tests).
#[derive(Default, Clone)]
pub struct MemDisk {
    pub files: BTreeMap<String, (Vec<u8>, u64)>,
}

impl MemDisk {
    pub fn put(&mut self, path: &str, data: &[u8], mtime: u64) {
        self.files.insert(path.to_string(), (data.to_vec(), mtime));
    }
}

impl Disk for MemDisk {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(path).map(|f| f.0.clone())
    }

    fn read_range(&self, path: &str, start: u64, len: usize) -> Option<Vec<u8>> {
        let f = &self.files.get(path)?.0;
        let s = (start as usize).min(f.len());
        Some(f[s..(s + len).min(f.len())].to_vec())
    }

    fn size(&self, path: &str) -> Option<u64> {
        self.files.get(path).map(|f| f.0.len() as u64)
    }

    fn write(&mut self, path: &str, data: &[u8]) -> bool {
        self.files.insert(path.to_string(), (data.to_vec(), 0));
        true
    }

    fn remove(&mut self, path: &str) -> bool {
        self.files.remove(path).is_some()
    }

    fn list(&self, dir: &str) -> Vec<FileStat> {
        let prefix = format!("{}/", dir.trim_end_matches('/'));
        self.files
            .iter()
            .filter_map(|(p, (d, m))| p.strip_prefix(&prefix).map(|rel| FileStat { path: rel.to_string(), size: d.len() as u64, mtime: *m }))
            .collect()
    }
}

/// The computer's files (std::fs, with the computer's own calls as a fallback).
#[derive(Default)]
pub struct StdDisk;

impl Disk for StdDisk {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(path).ok().or_else(|| ecm_host_abi::fs::read_file_bytes(path.trim_start_matches('/')))
    }

    fn read_range(&self, path: &str, start: u64, len: usize) -> Option<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(path).ok()?;
        f.seek(SeekFrom::Start(start)).ok()?;
        let mut buf = vec![0u8; len];
        let mut got = 0;
        while got < len {
            match f.read(&mut buf[got..]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(_) => return None,
            }
        }
        buf.truncate(got);
        Some(buf)
    }

    fn size(&self, path: &str) -> Option<u64> {
        std::fs::metadata(path).ok().filter(|m| m.is_file()).map(|m| m.len())
    }

    fn write(&mut self, path: &str, data: &[u8]) -> bool {
        if std::fs::write(path, data).is_ok() {
            return true;
        }
        if let Some((parent, _)) = path.rsplit_once('/') {
            if !parent.is_empty() && std::fs::create_dir_all(parent).is_err() {
                // The computer's own calls, one level at a time.
                let mut dir = String::new();
                for part in parent.trim_start_matches('/').split('/') {
                    if !dir.is_empty() {
                        dir.push('/');
                    }
                    dir.push_str(part);
                    ecm_host_abi::fs::mkdir(&dir);
                }
            }
        }
        std::fs::write(path, data).is_ok() || ecm_host_abi::fs::write_file_bytes(path.trim_start_matches('/'), data) >= 0
    }

    fn remove(&mut self, path: &str) -> bool {
        std::fs::remove_file(path).is_ok()
    }

    fn list(&self, dir: &str) -> Vec<FileStat> {
        fn walk(root: &str, rel: &str, out: &mut Vec<FileStat>, depth: usize) {
            let path = if rel.is_empty() { root.to_string() } else { format!("{root}/{rel}") };
            let Ok(rd) = std::fs::read_dir(&path) else { return };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let child = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                let Ok(md) = e.metadata() else { continue };
                if md.is_dir() {
                    if depth < 8 {
                        walk(root, &child, out, depth + 1);
                    }
                } else {
                    let mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                    out.push(FileStat { path: child, size: md.len(), mtime });
                }
            }
        }
        let mut out = Vec::new();
        walk(dir.trim_end_matches('/'), "", &mut out, 0);
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }
}
