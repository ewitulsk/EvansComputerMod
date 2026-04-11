//! File system bindings for the terminal host functions.
//! Provides safe Rust wrappers around the low-level host functions.
//! Supports nested directories and a current working directory (CWD).

// Host functions for file system operations
extern "C" {
    /// Writes data to a file. Creates parent directories as needed.
    fn file_write(path_ptr: *const u8, path_len: usize, data_ptr: *const u8, data_len: usize) -> i32;
    /// Reads data from a file into a buffer.
    fn file_read(path_ptr: *const u8, path_len: usize, buf_ptr: *mut u8, buf_len: usize) -> i32;
    /// Gets the size of a file. Returns -1 if not found.
    fn file_size(path_ptr: *const u8, path_len: usize) -> i32;
    /// Checks if a path exists. Returns 1 if exists, 0 if not.
    fn file_exists(path_ptr: *const u8, path_len: usize) -> i32;
    /// Deletes a file. Returns 1 on success, 0 on failure.
    fn file_delete(path_ptr: *const u8, path_len: usize) -> i32;
    /// Lists all files in root. Writes newline-separated filenames.
    fn file_list(buf_ptr: *mut u8, buf_len: usize) -> i32;
    /// Creates a directory (and parents). Returns 0 on success, -1 on error.
    fn file_mkdir(path_ptr: *const u8, path_len: usize) -> i32;
    /// Checks if a path is a directory. Returns 1 if dir, 0 if not.
    fn file_is_dir(path_ptr: *const u8, path_len: usize) -> i32;
    /// Lists entries in a directory. Writes "type:name\n" format.
    fn file_list_dir(path_ptr: *const u8, path_len: usize, buf_ptr: *mut u8, buf_len: usize) -> i32;
}

/// Maximum file size we support reading into memory
const MAX_FILE_SIZE: usize = 64 * 1024; // 64KB

/// Buffer for file operations
static mut FILE_BUFFER: [u8; MAX_FILE_SIZE] = [0u8; MAX_FILE_SIZE];

/// Buffer for directory listing
static mut LIST_BUFFER: [u8; 8192] = [0u8; 8192];

/// Current working directory (empty = root)
static mut CWD: [u8; 256] = [0u8; 256];
static mut CWD_LEN: usize = 0;

/// Get the current working directory.
pub fn get_cwd() -> &'static str {
    unsafe {
        if CWD_LEN == 0 {
            ""
        } else {
            std::str::from_utf8_unchecked(&CWD[..CWD_LEN])
        }
    }
}

/// Set the current working directory.
pub fn set_cwd(path: &str) {
    unsafe {
        let bytes = path.as_bytes();
        let len = bytes.len().min(CWD.len());
        CWD[..len].copy_from_slice(&bytes[..len]);
        CWD_LEN = len;
    }
}

/// Resolve a path relative to the CWD, normalizing `.` and `..` segments.
///
/// Security: `..` segments are resolved here so the host never sees them.
/// Going above root clamps to root (empty string). The host `sanitizePath()`
/// still rejects `..` as a second layer of defense.
pub fn resolve_path(input: &str) -> String {
    let input = input.trim();
    let combined = if input.is_empty() {
        get_cwd().to_string()
    } else if input.starts_with('/') {
        // Absolute path — don't prepend CWD
        input[1..].to_string()
    } else {
        let cwd = get_cwd();
        if cwd.is_empty() {
            input.to_string()
        } else {
            format!("{}/{}", cwd, input)
        }
    };

    // Normalize: process . and .. segments
    let mut parts: Vec<&str> = Vec::new();
    for segment in combined.split('/') {
        match segment {
            "" | "." => {} // skip empty segments and current-dir
            ".." => { parts.pop(); } // go up (clamped to root)
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Writes content to a file (resolved against CWD).
pub fn write_file(filename: &str, content: &str) -> bool {
    let path = resolve_path(filename);
    unsafe {
        file_write(path.as_ptr(), path.len(), content.as_ptr(), content.len()) >= 0
    }
}

/// Writes raw bytes to a file at an absolute path (no CWD resolution).
/// Used by git for writing binary objects.
pub fn write_file_bytes_absolute(path: &str, data: &[u8]) -> bool {
    unsafe {
        file_write(path.as_ptr(), path.len(), data.as_ptr(), data.len()) >= 0
    }
}

/// Reads raw bytes from a file at an absolute path (no CWD resolution).
/// Returns the bytes read, or None on error.
pub fn read_file_bytes_absolute(path: &str) -> Option<Vec<u8>> {
    unsafe {
        let size = file_size(path.as_ptr(), path.len());
        if size < 0 { return None; }
        let size = size as usize;
        if size > MAX_FILE_SIZE { return None; }

        let mut buf = vec![0u8; size];
        let bytes_read = file_read(path.as_ptr(), path.len(), buf.as_mut_ptr(), size);
        if bytes_read < 0 { return None; }
        buf.truncate(bytes_read as usize);
        Some(buf)
    }
}

/// Write a string to an absolute path (no CWD resolution).
pub fn write_file_absolute(path: &str, content: &str) -> bool {
    unsafe {
        file_write(path.as_ptr(), path.len(), content.as_ptr(), content.len()) >= 0
    }
}

/// Read a string from an absolute path (no CWD resolution).
pub fn read_file_absolute(path: &str) -> Option<&'static str> {
    unsafe {
        let size = file_size(path.as_ptr(), path.len());
        if size < 0 { return None; }
        let size = size as usize;
        if size > MAX_FILE_SIZE { return None; }

        let bytes_read = file_read(path.as_ptr(), path.len(), FILE_BUFFER.as_mut_ptr(), size);
        if bytes_read < 0 { return None; }
        std::str::from_utf8(&FILE_BUFFER[..bytes_read as usize]).ok()
    }
}

/// Check if an absolute path exists (no CWD resolution).
pub fn exists_absolute(path: &str) -> bool {
    unsafe { file_exists(path.as_ptr(), path.len()) == 1 }
}

/// Create a directory at an absolute path (no CWD resolution).
pub fn mkdir_absolute(path: &str) -> bool {
    unsafe { file_mkdir(path.as_ptr(), path.len()) == 0 }
}

/// Reads a file's content (resolved against CWD).
pub fn read_file(filename: &str) -> Option<&'static str> {
    let path = resolve_path(filename);
    unsafe {
        let size = file_size(path.as_ptr(), path.len());
        if size < 0 { return None; }
        let size = size as usize;
        if size > MAX_FILE_SIZE { return None; }

        let bytes_read = file_read(path.as_ptr(), path.len(), FILE_BUFFER.as_mut_ptr(), size);
        if bytes_read < 0 { return None; }

        std::str::from_utf8(&FILE_BUFFER[..bytes_read as usize]).ok()
    }
}

/// Checks if a path exists (resolved against CWD).
pub fn exists(filename: &str) -> bool {
    let path = resolve_path(filename);
    unsafe { file_exists(path.as_ptr(), path.len()) == 1 }
}

/// Deletes a file (resolved against CWD).
pub fn delete_file(filename: &str) -> bool {
    let path = resolve_path(filename);
    unsafe { file_delete(path.as_ptr(), path.len()) == 1 }
}

/// Delete using an already-resolved path (no CWD prepending).
pub fn delete_absolute(path: &str) -> bool {
    unsafe { file_delete(path.as_ptr(), path.len()) == 1 }
}

/// List directory using an already-resolved path (no CWD prepending).
pub fn list_dir_absolute(path: &str) -> Vec<DirEntry> {
    let raw = unsafe {
        let bytes_written = file_list_dir(
            path.as_ptr(), path.len(),
            LIST_BUFFER.as_mut_ptr(), LIST_BUFFER.len(),
        );
        if bytes_written <= 0 { return Vec::new(); }
        std::str::from_utf8(&LIST_BUFFER[..bytes_written as usize]).unwrap_or("")
    };

    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for line in raw.split('\n') {
        if line.len() < 3 { continue; }
        let entry_type = &line[..2];
        let name = &line[2..];
        let is_dir = entry_type == "d:";
        let entry = DirEntry { name: name.to_string(), is_dir };
        if is_dir { dirs.push(entry); } else { files.push(entry); }
    }
    dirs.sort_by(|a, b| a.name.cmp(&b.name));
    files.sort_by(|a, b| a.name.cmp(&b.name));
    dirs.extend(files);
    dirs
}

/// Gets the size of a file (resolved against CWD).
pub fn get_size(filename: &str) -> Option<usize> {
    let path = resolve_path(filename);
    unsafe {
        let size = file_size(path.as_ptr(), path.len());
        if size >= 0 { Some(size as usize) } else { None }
    }
}

/// Lists all files in root (legacy, for backward compat).
pub fn list_files() -> &'static str {
    unsafe {
        let bytes_written = file_list(LIST_BUFFER.as_mut_ptr(), LIST_BUFFER.len());
        if bytes_written <= 0 { return ""; }
        std::str::from_utf8(&LIST_BUFFER[..bytes_written as usize]).unwrap_or("")
    }
}

/// Directory entry from list_dir
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// List entries in a directory (resolved against CWD).
/// Returns a vector of DirEntry. Directories are sorted first, then files.
pub fn list_dir(path: &str) -> Vec<DirEntry> {
    let resolved = resolve_path(path);
    let raw = unsafe {
        let bytes_written = file_list_dir(
            resolved.as_ptr(), resolved.len(),
            LIST_BUFFER.as_mut_ptr(), LIST_BUFFER.len(),
        );
        if bytes_written <= 0 { return Vec::new(); }
        std::str::from_utf8(&LIST_BUFFER[..bytes_written as usize]).unwrap_or("")
    };

    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for line in raw.split('\n') {
        if line.len() < 3 { continue; }
        let entry_type = &line[..2];
        let name = &line[2..];
        let is_dir = entry_type == "d:";
        let entry = DirEntry { name: name.to_string(), is_dir };
        if is_dir { dirs.push(entry); } else { files.push(entry); }
    }
    dirs.sort_by(|a, b| a.name.cmp(&b.name));
    files.sort_by(|a, b| a.name.cmp(&b.name));
    dirs.extend(files);
    dirs
}

/// Create a directory (resolved against CWD).
pub fn mkdir(dirname: &str) -> bool {
    let path = resolve_path(dirname);
    unsafe { file_mkdir(path.as_ptr(), path.len()) == 0 }
}

/// Check if a path is a directory (resolved against CWD).
pub fn is_dir(path: &str) -> bool {
    let resolved = resolve_path(path);
    unsafe { file_is_dir(resolved.as_ptr(), resolved.len()) == 1 }
}

/// Check if an already-resolved path is a directory (no CWD prepending).
pub fn is_dir_absolute(path: &str) -> bool {
    if path.is_empty() {
        return true;
    }
    unsafe { file_is_dir(path.as_ptr(), path.len()) == 1 }
}
