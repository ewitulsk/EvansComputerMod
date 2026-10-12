//! The computer's file system, behind a trait so the editor runs against
//! the real storage ([`HostFiles`], wasm32) or a directory / memory in tests.
//! Paths are relative to the computer's storage root (`visual/hello.vpl`),
//! like every ECM program; a leading `/` is ignored.

use std::collections::BTreeMap;

pub trait Files {
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), String>;
    /// File names (not directories) directly in `dir`, sorted.
    fn list(&self, dir: &str) -> Vec<String>;
    fn exists(&self, path: &str) -> bool;

    fn read_string(&self, path: &str) -> Option<String> {
        self.read(path).and_then(|b| String::from_utf8(b).ok())
    }
}

/// `/a//b/` → `a/b`.
pub fn norm(path: &str) -> String {
    path.split('/').filter(|s| !s.is_empty() && *s != ".").collect::<Vec<_>>().join("/")
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn parent(path: &str) -> Option<&str> {
    path.rfind('/').map(|i| &path[..i])
}

/// Files in memory (tests).
#[derive(Default, Clone, Debug)]
pub struct MemFiles {
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Files for MemFiles {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(&norm(path)).cloned()
    }
    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        self.files.insert(norm(path), data.to_vec());
        Ok(())
    }
    fn list(&self, dir: &str) -> Vec<String> {
        let d = norm(dir);
        let prefix = if d.is_empty() { String::new() } else { format!("{d}/") };
        self.files
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .filter(|rest| !rest.contains('/'))
            .map(String::from)
            .collect()
    }
    fn exists(&self, path: &str) -> bool {
        let p = norm(path);
        self.files.contains_key(&p) || self.files.keys().any(|k| k.starts_with(&format!("{p}/")))
    }
}

/// Shared memory files, so a test keeps a handle while the editor owns another.
impl Files for std::rc::Rc<std::cell::RefCell<MemFiles>> {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.borrow().read(path)
    }
    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        self.borrow_mut().write(path, data)
    }
    fn list(&self, dir: &str) -> Vec<String> {
        self.borrow().list(dir)
    }
    fn exists(&self, path: &str) -> bool {
        self.borrow().exists(path)
    }
}

/// A host directory standing in for the storage root (native runs).
pub struct DirFiles {
    pub root: std::path::PathBuf,
}

impl Files for DirFiles {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.root.join(norm(path))).ok()
    }
    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        let p = self.root.join(norm(path));
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(p, data).map_err(|e| e.to_string())
    }
    fn list(&self, dir: &str) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.root.join(norm(dir)))
            .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        v.sort();
        v
    }
    fn exists(&self, path: &str) -> bool {
        self.root.join(norm(path)).exists()
    }
}

/// The computer's storage (`ecm_host_abi::fs`).
#[cfg(target_arch = "wasm32")]
pub struct HostFiles;

#[cfg(target_arch = "wasm32")]
impl Files for HostFiles {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        ecm_host_abi::fs::read_file_bytes(&norm(path))
    }
    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        let p = norm(path);
        // Create the parent directories first.
        let mut dirs = Vec::new();
        let mut cur = parent(&p);
        while let Some(d) = cur {
            if d.is_empty() || ecm_host_abi::fs::exists(d) {
                break;
            }
            dirs.push(d.to_string());
            cur = parent(d);
        }
        for d in dirs.iter().rev() {
            ecm_host_abi::fs::mkdir(d);
        }
        if ecm_host_abi::fs::write_file_bytes(&p, data) < 0 {
            return Err(format!("can't write {p}"));
        }
        Ok(())
    }
    fn list(&self, dir: &str) -> Vec<String> {
        let mut v: Vec<String> = ecm_host_abi::fs::list_dir(&norm(dir))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.strip_prefix("f:").map(String::from).or_else(|| (!l.contains(':') && !l.ends_with('/')).then(|| l.to_string())))
            .filter(|s| !s.is_empty())
            .collect();
        v.sort();
        v
    }
    fn exists(&self, path: &str) -> bool {
        ecm_host_abi::fs::exists(&norm(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_files_list_one_level() {
        let mut f = MemFiles::default();
        f.write("/visual/a.vpl", b"1").unwrap();
        f.write("visual/apps/x/app.toml", b"2").unwrap();
        f.write("visual//b.ui.json", b"3").unwrap();
        assert_eq!(f.list("visual"), ["a.vpl", "b.ui.json"]);
        assert!(f.exists("visual/apps"));
        assert_eq!(f.read_string("visual/a.vpl").as_deref(), Some("1"));
        assert_eq!(norm("/a//b/./c/"), "a/b/c");
    }
}
