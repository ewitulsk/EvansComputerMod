//! File-system helpers over the host file imports.
//!
//! Paths passed to the host are always "absolute" relative to the computer's
//! storage root, without a leading slash (`""` is the root). Relative paths
//! are resolved against an explicit working directory owned by the shell.

use crate::hal;

/// Resolve `input` against `cwd`, normalising `.` and `..` so the host never
/// sees them. Going above the root clamps to the root.
pub fn resolve(cwd: &str, input: &str) -> String {
    let input = input.trim();
    let combined = if let Some(abs) = input.strip_prefix('/') {
        abs.to_string()
    } else if cwd.is_empty() {
        input.to_string()
    } else if input.is_empty() {
        cwd.to_string()
    } else {
        format!("{}/{}", cwd, input)
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in combined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

pub fn read_to_string(path: &str) -> Option<String> {
    hal::file::read(path).and_then(|b| String::from_utf8(b).ok())
}

pub fn write(path: &str, data: &[u8]) -> bool {
    hal::file::write(path, data)
}

pub fn exists(path: &str) -> bool {
    hal::file::exists(path)
}

pub fn is_dir(path: &str) -> bool {
    hal::file::is_dir(path)
}

pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Directory listing, directories first, each group sorted by name.
pub fn list_dir(path: &str) -> Vec<DirEntry> {
    let raw = hal::file::list_dir(path);
    let mut out: Vec<DirEntry> = raw
        .split('\n')
        .filter_map(|line| {
            let (kind, name) = line.split_at_checked(2)?;
            if name.is_empty() {
                return None;
            }
            Some(DirEntry { name: name.to_string(), is_dir: kind == "d:" })
        })
        .collect();
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    out
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn resolve_normalises_and_clamps() {
        assert_eq!(resolve("", "a/b"), "a/b");
        assert_eq!(resolve("a", "b/../c"), "a/c");
        assert_eq!(resolve("a/b", "../../../x"), "x");
        assert_eq!(resolve("a/b", "/etc"), "etc");
        assert_eq!(resolve("a/b", ""), "a/b");
        assert_eq!(resolve("a", "./."), "a");
    }
}
