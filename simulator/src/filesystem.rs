use std::fs;
use std::path::{Path, PathBuf};

pub struct FileSystem {
    storage_dir: PathBuf,
}

impl FileSystem {
    pub fn new(storage_dir: PathBuf) -> Self {
        fs::create_dir_all(&storage_dir).ok();
        Self { storage_dir }
    }

    fn sanitize_path(&self, filename: &str) -> Option<PathBuf> {
        if filename.is_empty()
            || filename.contains("..")
            || filename.starts_with('/')
            || filename.starts_with('\\')
            || filename.contains(':')
            || filename.contains('\0')
        {
            return None;
        }
        let resolved = self.storage_dir.join(filename);
        let resolved = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());
        // For new files that don't exist yet, check parent
        let check_path = if resolved.exists() {
            resolved.clone()
        } else {
            resolved.parent().unwrap_or(Path::new("")).to_path_buf()
        };
        let storage_canonical = self.storage_dir.canonicalize().unwrap_or_else(|_| self.storage_dir.clone());
        let check_canonical = check_path.canonicalize().unwrap_or_else(|_| check_path);
        if !check_canonical.starts_with(&storage_canonical) && check_canonical != storage_canonical {
            return None;
        }
        Some(self.storage_dir.join(filename))
    }

    pub fn write_file(&self, filename: &str, data: &[u8]) -> i32 {
        if data.len() > 1024 * 1024 {
            return -1;
        }
        match self.sanitize_path(filename) {
            Some(path) => match fs::write(&path, data) {
                Ok(_) => data.len() as i32,
                Err(_) => -1,
            },
            None => -1,
        }
    }

    pub fn read_file(&self, filename: &str) -> Option<Vec<u8>> {
        let path = self.sanitize_path(filename)?;
        fs::read(&path).ok()
    }

    pub fn file_size(&self, filename: &str) -> i32 {
        match self.sanitize_path(filename) {
            Some(path) => match fs::metadata(&path) {
                Ok(m) => m.len() as i32,
                Err(_) => -1,
            },
            None => -1,
        }
    }

    pub fn file_exists(&self, filename: &str) -> bool {
        match self.sanitize_path(filename) {
            Some(path) => path.exists(),
            None => false,
        }
    }

    pub fn delete_file(&self, filename: &str) -> bool {
        match self.sanitize_path(filename) {
            Some(path) => fs::remove_file(&path).is_ok(),
            None => false,
        }
    }

    pub fn list_files(&self) -> String {
        let entries = match fs::read_dir(&self.storage_dir) {
            Ok(e) => e,
            Err(_) => return String::new(),
        };
        let mut names: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() {
                if ft.is_file() {
                    if let Some(name) = entry.file_name().to_str() {
                        names.push(name.to_string());
                    }
                }
            }
        }
        names.sort();
        names.join("\n")
    }
}
