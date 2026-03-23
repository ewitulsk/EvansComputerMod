//! Minimal git implementation using standard git object format.
//!
//! Produces GitHub-compatible repositories. Objects are stored as
//! zlib-compressed blobs addressed by SHA-1 hash, matching real git.
//!
//! The index uses a simplified text format (not binary) since it
//! never transfers to remotes — only objects and refs do.

use sha1::{Sha1, Digest};
use flate2::write::ZlibEncoder;
use flate2::read::ZlibDecoder;
use flate2::Compression;
use std::io::{Read, Write};

use crate::fs;
use crate::terminal;

// ============================================================
// .gitignore support
// ============================================================

struct IgnorePattern {
    /// The glob pattern (after stripping ! and trailing /)
    pattern: String,
    /// If true, this pattern un-ignores (starts with !)
    negated: bool,
    /// If true, only matches directories (original pattern ended with /)
    dir_only: bool,
}

struct GitIgnore {
    patterns: Vec<IgnorePattern>,
}

impl GitIgnore {
    /// Parse a .gitignore file. Returns an empty GitIgnore if file doesn't exist.
    fn load(git_dir: &str) -> Self {
        let root = work_tree_root_static(git_dir);
        let path = if root.is_empty() {
            ".gitignore".to_string()
        } else {
            format!("{}/.gitignore", root)
        };

        let content = match fs::read_file_absolute(&path) {
            Some(c) => c.to_string(),
            None => return Self { patterns: Vec::new() },
        };

        let mut patterns = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let mut negated = false;
            let mut pat = line;

            if pat.starts_with('!') {
                negated = true;
                pat = &pat[1..];
            }

            let dir_only = pat.ends_with('/');
            let pat = if dir_only {
                &pat[..pat.len() - 1]
            } else {
                pat
            };

            patterns.push(IgnorePattern {
                pattern: pat.to_string(),
                negated,
                dir_only,
            });
        }

        Self { patterns }
    }

    /// Check if a path should be ignored.
    /// `rel_path` is relative to the repo root (e.g., "src/main.py").
    /// `is_dir` indicates whether the path is a directory.
    /// Returns true if the path should be ignored.
    fn is_ignored(&self, rel_path: &str, is_dir: bool) -> bool {
        let mut ignored = false;

        for pat in &self.patterns {
            if pat.dir_only && !is_dir {
                continue;
            }

            if glob_match(&pat.pattern, rel_path) {
                ignored = !pat.negated;
            }
        }

        ignored
    }
}

/// Simple glob matching supporting `*`, `**`, and `?`.
/// Matches against both the full path and the filename component.
fn glob_match(pattern: &str, path: &str) -> bool {
    // If pattern contains no slash, match against the filename only
    // (as well as the full path for ** patterns)
    if !pattern.contains('/') {
        // Match against basename
        let basename = path.rsplit('/').next().unwrap_or(path);
        if glob_match_segment(pattern, basename) {
            return true;
        }
        // Also match against each path component for directory patterns
        for component in path.split('/') {
            if glob_match_segment(pattern, component) {
                return true;
            }
        }
        return false;
    }

    // Pattern contains slash — match against full path
    if pattern.contains("**") {
        // ** matches across directories
        let parts: Vec<&str> = pattern.split("**").collect();
        if parts.len() == 2 {
            let prefix = parts[0].trim_end_matches('/');
            let suffix = parts[1].trim_start_matches('/');
            if prefix.is_empty() && suffix.is_empty() {
                return true; // "**" alone matches everything
            }
            if prefix.is_empty() {
                // "**/suffix" — match suffix at any depth
                return path.ends_with(suffix) || glob_match_segment(suffix, path);
            }
            if suffix.is_empty() {
                // "prefix/**" — match prefix at start
                return path.starts_with(prefix);
            }
            // "prefix/**/suffix"
            return path.starts_with(prefix) && path.ends_with(suffix);
        }
    }

    glob_match_segment(pattern, path)
}

/// Match a single glob pattern (with * and ?) against a string.
fn glob_match_segment(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_match_recursive(&pat, 0, &txt, 0)
}

fn glob_match_recursive(pat: &[char], pi: usize, txt: &[char], ti: usize) -> bool {
    if pi == pat.len() {
        return ti == txt.len();
    }

    if pat[pi] == '*' {
        // * matches zero or more characters (not including /)
        // Try matching zero chars, then one, then two, etc.
        let mut t = ti;
        loop {
            if glob_match_recursive(pat, pi + 1, txt, t) {
                return true;
            }
            if t >= txt.len() {
                break;
            }
            t += 1;
        }
        return false;
    }

    if ti >= txt.len() {
        return false;
    }

    if pat[pi] == '?' || pat[pi] == txt[ti] {
        return glob_match_recursive(pat, pi + 1, txt, ti + 1);
    }

    false
}

/// Helper to get work tree root without needing &self (used by GitIgnore::load)
fn work_tree_root_static(git_dir: &str) -> String {
    if git_dir == ".git" {
        String::new()
    } else {
        git_dir[..git_dir.len() - 5].to_string()
    }
}

// ============================================================
// Core types
// ============================================================

/// A 20-byte SHA-1 object ID.
#[derive(Clone, PartialEq, Eq, Hash)]
struct ObjectId([u8; 20]);

impl ObjectId {
    fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() != 40 { return None; }
        let mut bytes = [0u8; 20];
        for i in 0..20 {
            bytes[i] = u8::from_str_radix(&hex[i*2..i*2+2], 16).ok()?;
        }
        Some(Self(bytes))
    }

    fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// First 2 hex chars (used as object directory name)
    fn dir(&self) -> String {
        format!("{:02x}", self.0[0])
    }

    /// Remaining 38 hex chars (used as object filename)
    fn file(&self) -> String {
        self.0[1..].iter().map(|b| format!("{:02x}", b)).collect()
    }
}

/// An entry in the staging index.
#[derive(Clone)]
struct IndexEntry {
    mode: String, // "100644" for regular files
    hash: ObjectId,
    name: String,
}

// ============================================================
// Git directory helpers
// ============================================================

/// Find the .git directory by looking from CWD upward.
/// Returns the absolute path to .git/ (without trailing slash).
fn find_git_dir() -> Option<String> {
    // Check CWD and parent directories
    let cwd = fs::get_cwd().to_string();
    let mut search = cwd.clone();

    loop {
        let git_path = if search.is_empty() {
            ".git".to_string()
        } else {
            format!("{}/.git", search)
        };

        if fs::exists_absolute(&git_path) && fs::is_dir_absolute(&git_path) {
            return Some(git_path);
        }

        // Move up one directory
        if search.is_empty() {
            break;
        }
        match search.rfind('/') {
            Some(pos) => search = search[..pos].to_string(),
            None => search = String::new(),
        }
    }

    // Final check at root
    if fs::exists_absolute(".git") && fs::is_dir_absolute(".git") {
        return Some(".git".to_string());
    }

    None
}

fn require_git_dir() -> Result<String, String> {
    find_git_dir().ok_or_else(|| "fatal: not a git repository (run 'git init')".to_string())
}

/// Get the working tree root (parent of .git/)
fn work_tree_root(git_dir: &str) -> String {
    work_tree_root_static(git_dir)
}

// ============================================================
// Object operations
// ============================================================

/// Compute SHA-1 of a git object (type + size + content).
fn hash_object(data: &[u8], obj_type: &str) -> ObjectId {
    let header = format!("{} {}\0", obj_type, data.len());
    let mut hasher = Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(data);
    let result = hasher.finalize();
    let mut id = [0u8; 20];
    id.copy_from_slice(&result);
    ObjectId(id)
}

/// Write a git object to the object store. Returns the object ID.
fn write_object(git_dir: &str, data: &[u8], obj_type: &str) -> Result<ObjectId, String> {
    let id = hash_object(data, obj_type);

    // Check if object already exists
    let obj_path = format!("{}/objects/{}/{}", git_dir, id.dir(), id.file());
    if fs::exists_absolute(&obj_path) {
        return Ok(id);
    }

    // Create directory
    let dir_path = format!("{}/objects/{}", git_dir, id.dir());
    fs::mkdir_absolute(&dir_path);

    // Compress: header + data
    let header = format!("{} {}\0", obj_type, data.len());
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(header.as_bytes()).map_err(|e| format!("compress error: {}", e))?;
    encoder.write_all(data).map_err(|e| format!("compress error: {}", e))?;
    let compressed = encoder.finish().map_err(|e| format!("compress error: {}", e))?;

    // Write object
    if !fs::write_file_bytes_absolute(&obj_path, &compressed) {
        return Err(format!("failed to write object {}", id.to_hex()));
    }

    Ok(id)
}

/// Read a git object from the object store.
/// Returns (type, data) on success.
fn read_object(git_dir: &str, id: &ObjectId) -> Result<(String, Vec<u8>), String> {
    let obj_path = format!("{}/objects/{}/{}", git_dir, id.dir(), id.file());
    let compressed = fs::read_file_bytes_absolute(&obj_path)
        .ok_or_else(|| format!("object {} not found", id.to_hex()))?;

    // Decompress
    let mut decoder = ZlibDecoder::new(&compressed[..]);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed).map_err(|e| format!("decompress error: {}", e))?;

    // Parse header: "type size\0data"
    let null_pos = decompressed.iter().position(|&b| b == 0)
        .ok_or("invalid object: no null byte")?;
    let header = std::str::from_utf8(&decompressed[..null_pos])
        .map_err(|_| "invalid object header")?;
    let space_pos = header.find(' ').ok_or("invalid object header: no space")?;
    let obj_type = header[..space_pos].to_string();
    let data = decompressed[null_pos + 1..].to_vec();

    Ok((obj_type, data))
}

// ============================================================
// Index operations (simplified text format)
// ============================================================

fn read_index(git_dir: &str) -> Vec<IndexEntry> {
    let index_path = format!("{}/index", git_dir);
    let content = match fs::read_file_absolute(&index_path) {
        Some(c) => c.to_string(),
        None => return Vec::new(),
    };

    let mut entries = Vec::new();
    for line in content.lines() {
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        if parts.len() == 3 {
            if let Some(hash) = ObjectId::from_hex(parts[1]) {
                entries.push(IndexEntry {
                    mode: parts[0].to_string(),
                    hash,
                    name: parts[2].to_string(),
                });
            }
        }
    }
    entries
}

fn write_index(git_dir: &str, entries: &[IndexEntry]) {
    let mut content = String::new();
    for entry in entries {
        content.push_str(&format!("{} {} {}\n", entry.mode, entry.hash.to_hex(), entry.name));
    }
    let index_path = format!("{}/index", git_dir);
    fs::write_file_absolute(&index_path, &content);
}

// ============================================================
// Tree operations
// ============================================================

/// Build a tree object from index entries.
/// Handles nested directories by recursively creating sub-trees.
fn build_tree(git_dir: &str, entries: &[IndexEntry], prefix: &str) -> Result<ObjectId, String> {
    // Group entries by top-level name at this prefix level
    let mut direct_files: Vec<&IndexEntry> = Vec::new();
    let mut subdirs: std::collections::BTreeMap<String, Vec<&IndexEntry>> = std::collections::BTreeMap::new();

    for entry in entries {
        let name = if prefix.is_empty() {
            &entry.name
        } else if entry.name.starts_with(prefix) {
            &entry.name[prefix.len()..]
        } else {
            continue;
        };

        if let Some(slash) = name.find('/') {
            let dir_name = &name[..slash];
            subdirs.entry(dir_name.to_string()).or_default().push(entry);
        } else {
            direct_files.push(entry);
        }
    }

    // Build tree data in git's binary format
    let mut tree_data: Vec<u8> = Vec::new();

    // Add subdirectories (mode 40000)
    for (dir_name, sub_entries) in &subdirs {
        let sub_prefix = if prefix.is_empty() {
            format!("{}/", dir_name)
        } else {
            format!("{}{}/", prefix, dir_name)
        };
        let sub_tree_id = build_tree(git_dir, entries, &sub_prefix)?;
        // Format: "40000 dirname\0<20-byte-hash>"
        tree_data.extend_from_slice(format!("40000 {}\0", dir_name).as_bytes());
        tree_data.extend_from_slice(&sub_tree_id.0);
    }

    // Add files
    for entry in &direct_files {
        let file_name = if prefix.is_empty() {
            &entry.name
        } else {
            &entry.name[prefix.len()..]
        };
        tree_data.extend_from_slice(format!("{} {}\0", entry.mode, file_name).as_bytes());
        tree_data.extend_from_slice(&entry.hash.0);
    }

    write_object(git_dir, &tree_data, "tree")
}

/// Read a tree object and return its entries as (mode, name, hash).
fn read_tree(git_dir: &str, tree_id: &ObjectId) -> Result<Vec<(String, String, ObjectId)>, String> {
    let (obj_type, data) = read_object(git_dir, tree_id)?;
    if obj_type != "tree" {
        return Err(format!("expected tree, got {}", obj_type));
    }

    let mut entries = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        // Find space (separates mode from name)
        let space = data[pos..].iter().position(|&b| b == b' ')
            .ok_or("invalid tree entry")? + pos;
        let mode = std::str::from_utf8(&data[pos..space])
            .map_err(|_| "invalid mode")?.to_string();

        // Find null (separates name from hash)
        let null = data[space+1..].iter().position(|&b| b == 0)
            .ok_or("invalid tree entry")? + space + 1;
        let name = std::str::from_utf8(&data[space+1..null])
            .map_err(|_| "invalid name")?.to_string();

        // Next 20 bytes are the hash
        if null + 21 > data.len() {
            return Err("truncated tree entry".to_string());
        }
        let mut hash = [0u8; 20];
        hash.copy_from_slice(&data[null+1..null+21]);

        entries.push((mode, name, ObjectId(hash)));
        pos = null + 21;
    }

    Ok(entries)
}

/// Flatten a tree into a list of (path, hash) for all blobs.
fn flatten_tree(git_dir: &str, tree_id: &ObjectId, prefix: &str) -> Result<Vec<(String, ObjectId)>, String> {
    let entries = read_tree(git_dir, tree_id)?;
    let mut result = Vec::new();

    for (mode, name, hash) in entries {
        let full_path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };

        if mode == "40000" {
            // Recurse into subdirectory
            result.extend(flatten_tree(git_dir, &hash, &full_path)?);
        } else {
            result.push((full_path, hash));
        }
    }

    Ok(result)
}

// ============================================================
// Ref operations
// ============================================================

/// Read HEAD — returns either a branch name or a commit hash.
fn read_head(git_dir: &str) -> Option<String> {
    fs::read_file_absolute(&format!("{}/HEAD", git_dir)).map(|s| s.trim().to_string())
}

/// Get the current branch name from HEAD (if it's a symbolic ref).
fn current_branch(git_dir: &str) -> Option<String> {
    let head = read_head(git_dir)?;
    if head.starts_with("ref: refs/heads/") {
        Some(head["ref: refs/heads/".len()..].to_string())
    } else {
        None // detached HEAD
    }
}

/// Resolve HEAD to a commit hash.
fn resolve_head(git_dir: &str) -> Option<ObjectId> {
    let head = read_head(git_dir)?;
    if head.starts_with("ref: ") {
        let ref_path = format!("{}/{}", git_dir, &head[5..]);
        let hash_str = fs::read_file_absolute(&ref_path)?;
        ObjectId::from_hex(hash_str.trim())
    } else {
        ObjectId::from_hex(&head)
    }
}

/// Update the ref that HEAD points to with a new commit hash.
fn update_head_ref(git_dir: &str, commit_id: &ObjectId) {
    let head = read_head(git_dir).unwrap_or_default();
    if head.starts_with("ref: ") {
        let ref_path = format!("{}/{}", git_dir, &head[5..]);
        fs::write_file_absolute(&ref_path, &format!("{}\n", commit_id.to_hex()));
    } else {
        fs::write_file_absolute(&format!("{}/HEAD", git_dir), &format!("{}\n", commit_id.to_hex()));
    }
}

/// Read a branch ref — returns the commit hash it points to.
fn read_ref(git_dir: &str, branch: &str) -> Option<ObjectId> {
    let ref_path = format!("{}/refs/heads/{}", git_dir, branch);
    let hash_str = fs::read_file_absolute(&ref_path)?;
    ObjectId::from_hex(hash_str.trim())
}

// ============================================================
// Timestamp helper
// ============================================================

/// Get current Unix timestamp (seconds since epoch).
/// Uses chrono which gets time from the wasm-bindgen Date.now() stub.
fn unix_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

// ============================================================
// Commit parsing helper
// ============================================================

/// Parsed commit data.
struct CommitInfo {
    tree: ObjectId,
    parent: Option<ObjectId>,
    author: String,
    committer: String,
    message: String,
}

/// Parse a commit object's data into structured fields.
fn parse_commit(data: &[u8]) -> Result<CommitInfo, String> {
    let text = String::from_utf8_lossy(data).to_string();
    let mut tree: Option<ObjectId> = None;
    let mut parent: Option<ObjectId> = None;
    let mut author = String::new();
    let mut committer = String::new();
    let mut in_message = false;
    let mut message_lines: Vec<&str> = Vec::new();

    for line in text.lines() {
        if in_message {
            message_lines.push(line);
        } else if line.is_empty() {
            in_message = true;
        } else if let Some(hash) = line.strip_prefix("tree ") {
            tree = ObjectId::from_hex(hash.trim());
        } else if let Some(hash) = line.strip_prefix("parent ") {
            parent = ObjectId::from_hex(hash.trim());
        } else if let Some(a) = line.strip_prefix("author ") {
            author = a.to_string();
        } else if let Some(c) = line.strip_prefix("committer ") {
            committer = c.to_string();
        }
    }

    let tree = tree.ok_or("commit missing tree")?;
    let message = message_lines.join("\n").trim().to_string();

    Ok(CommitInfo { tree, parent, author, committer, message })
}

// ============================================================
// Rebase helpers
// ============================================================

/// Find the merge-base (common ancestor) of two commits.
/// Walks both chains and returns the first intersection.
fn find_merge_base(git_dir: &str, id1: &ObjectId, id2: &ObjectId) -> Option<ObjectId> {
    // Collect all ancestors of id1
    let mut ancestors1 = std::collections::HashSet::new();
    let mut current = id1.clone();
    loop {
        ancestors1.insert(current.to_hex());
        let (_, data) = read_object(git_dir, &current).ok()?;
        let info = parse_commit(&data).ok()?;
        match info.parent {
            Some(p) => current = p,
            None => break,
        }
    }

    // Walk id2's chain and find the first commit that's in id1's ancestors
    let mut current = id2.clone();
    loop {
        if ancestors1.contains(&current.to_hex()) {
            return Some(current);
        }
        let (_, data) = read_object(git_dir, &current).ok()?;
        let info = parse_commit(&data).ok()?;
        match info.parent {
            Some(p) => current = p,
            None => break,
        }
    }

    None
}

/// Collect ALL commits from `from` back to the root.
/// Returns in oldest-first order (ready to replay).
fn collect_all_commits(git_dir: &str, from: &ObjectId) -> Result<Vec<ObjectId>, String> {
    let mut commits = Vec::new();
    let mut current = from.clone();

    loop {
        commits.push(current.clone());
        let (_, data) = read_object(git_dir, &current)?;
        let info = parse_commit(&data)?;
        match info.parent {
            Some(p) => current = p,
            None => break, // Reached root
        }
    }

    commits.reverse(); // oldest first
    Ok(commits)
}

/// Create an empty tree object (no files). Returns its ObjectId.
/// This is used as the "onto" base when rebasing --root.
fn create_empty_tree(git_dir: &str) -> Result<ObjectId, String> {
    write_object(git_dir, &[], "tree")
}

/// Collect commits from `from` back to `to_exclusive` (not including it).
/// Returns in oldest-first order (ready to replay).
fn collect_commits(git_dir: &str, from: &ObjectId, to_exclusive: &ObjectId) -> Result<Vec<ObjectId>, String> {
    let mut commits = Vec::new();
    let mut current = from.clone();

    loop {
        if current == *to_exclusive {
            break;
        }
        commits.push(current.clone());
        let (_, data) = read_object(git_dir, &current)?;
        let info = parse_commit(&data)?;
        match info.parent {
            Some(p) => current = p,
            None => break, // Reached root without finding merge-base
        }
    }

    commits.reverse(); // oldest first
    Ok(commits)
}

/// Compute the diff between two trees as (added, modified, deleted) file lists.
/// Returns (added: Vec<(name, hash)>, modified: Vec<(name, old_hash, new_hash)>, deleted: Vec<(name, hash)>)
fn diff_trees(
    git_dir: &str,
    old_tree: &ObjectId,
    new_tree: &ObjectId,
) -> Result<(Vec<(String, ObjectId)>, Vec<(String, ObjectId, ObjectId)>, Vec<(String, ObjectId)>), String> {
    let old_files = flatten_tree(git_dir, old_tree, "")?;
    let new_files = flatten_tree(git_dir, new_tree, "")?;

    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();

    // Find added and modified
    for (name, new_hash) in &new_files {
        match old_files.iter().find(|(n, _)| n == name) {
            Some((_, old_hash)) => {
                if old_hash != new_hash {
                    modified.push((name.clone(), old_hash.clone(), new_hash.clone()));
                }
            }
            None => added.push((name.clone(), new_hash.clone())),
        }
    }

    // Find deleted
    for (name, old_hash) in &old_files {
        if !new_files.iter().any(|(n, _)| n == name) {
            deleted.push((name.clone(), old_hash.clone()));
        }
    }

    Ok((added, modified, deleted))
}

/// Cherry-pick a commit onto a new parent.
/// `onto` can be `None` for creating a new root commit (--root rebase).
/// Returns the new commit's ObjectId.
fn cherry_pick(git_dir: &str, commit_id: &ObjectId, onto: Option<&ObjectId>) -> Result<ObjectId, String> {
    // Parse the commit being cherry-picked
    let (_, commit_data) = read_object(git_dir, commit_id)?;
    let commit_info = parse_commit(&commit_data)?;

    // Get the parent tree (empty if root commit)
    let parent_tree_files = match &commit_info.parent {
        Some(parent_id) => {
            let (_, parent_data) = read_object(git_dir, parent_id)?;
            let parent_info = parse_commit(&parent_data)?;
            flatten_tree(git_dir, &parent_info.tree, "")?
        }
        None => Vec::new(), // Root commit: parent tree is empty
    };

    // Get the onto tree files (empty if creating new root)
    let mut current_files = match onto {
        Some(onto_id) => {
            let (_, onto_data) = read_object(git_dir, onto_id)?;
            let onto_info = parse_commit(&onto_data)?;
            flatten_tree(git_dir, &onto_info.tree, "")?
        }
        None => Vec::new(), // New root: start from empty tree
    };

    // Diff: what changed between parent and this commit
    let parent_tree_id = match &commit_info.parent {
        Some(parent_id) => {
            let (_, pd) = read_object(git_dir, parent_id)?;
            parse_commit(&pd)?.tree
        }
        None => create_empty_tree(git_dir)?,
    };
    let (added, modified, deleted) = diff_trees(git_dir, &parent_tree_id, &commit_info.tree)?;

    // Apply changes
    for (name, hash) in &added {
        if let Some((_, existing_hash)) = current_files.iter().find(|(n, _)| n == name) {
            if existing_hash != hash {
                return Err(format!("CONFLICT (add/add): {}", name));
            }
        } else {
            current_files.push((name.clone(), hash.clone()));
        }
    }

    for (name, old_hash, new_hash) in &modified {
        if let Some(entry) = current_files.iter_mut().find(|(n, _)| n == name) {
            if entry.1 == *old_hash {
                entry.1 = new_hash.clone();
            } else if entry.1 == *new_hash {
                // Already has the new content, skip
            } else {
                return Err(format!("CONFLICT (content): {}", name));
            }
        } else {
            return Err(format!("CONFLICT (modify/delete): {}", name));
        }
    }

    for (name, _old_hash) in &deleted {
        current_files.retain(|(n, _)| n != name);
    }

    // Sort and build new index entries, then tree
    current_files.sort_by(|a, b| a.0.cmp(&b.0));
    let index_entries: Vec<IndexEntry> = current_files.into_iter().map(|(name, hash)| {
        IndexEntry { mode: "100644".to_string(), hash, name }
    }).collect();

    let new_tree = build_tree(git_dir, &index_entries, "")?;

    // Create new commit with same message but new/no parent
    let timestamp = unix_timestamp();
    let new_author = format!("Terminal User <user@terminal.os> {} +0000", timestamp);
    let mut content = String::new();
    content.push_str(&format!("tree {}\n", new_tree.to_hex()));
    if let Some(onto_id) = onto {
        content.push_str(&format!("parent {}\n", onto_id.to_hex()));
    }
    // else: no parent line = root commit
    content.push_str(&format!("author {}\n", commit_info.author));
    content.push_str(&format!("committer {}\n", new_author));
    content.push_str(&format!("\n{}\n", commit_info.message));

    write_object(git_dir, content.as_bytes(), "commit")
}

// ============================================================
// Commands
// ============================================================

pub fn cmd_init() {
    let cwd = fs::get_cwd().to_string();
    let git_dir = if cwd.is_empty() {
        ".git".to_string()
    } else {
        format!("{}/.git", cwd)
    };

    if fs::exists_absolute(&git_dir) {
        terminal::println("Reinitialized existing Git repository");
        return;
    }

    fs::mkdir_absolute(&git_dir);
    fs::mkdir_absolute(&format!("{}/objects", git_dir));
    fs::mkdir_absolute(&format!("{}/refs", git_dir));
    fs::mkdir_absolute(&format!("{}/refs/heads", git_dir));
    fs::write_file_absolute(&format!("{}/HEAD", git_dir), "ref: refs/heads/main\n");

    terminal::println("Initialized empty Git repository");
}

pub fn cmd_add(args: &str) {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { terminal::println(&e); return; }
    };
    let root = work_tree_root(&git_dir);

    if args.trim().is_empty() {
        terminal::println("Nothing specified, nothing added.");
        return;
    }

    let mut index = read_index(&git_dir);
    let ignore = GitIgnore::load(&git_dir);

    for arg in args.split_whitespace() {
        if arg == "." {
            // Add all files in current working tree
            add_directory(&git_dir, &root, &fs::get_cwd().to_string(), &mut index, &ignore);
        } else {
            let resolved = fs::resolve_path(arg);
            // Make path relative to repo root
            let rel_path = if root.is_empty() {
                resolved.clone()
            } else if resolved.starts_with(&root) {
                resolved[root.len()+1..].to_string()
            } else {
                resolved.clone()
            };

            if fs::is_dir(&arg) {
                add_directory(&git_dir, &root, &resolved, &mut index, &ignore);
            } else if fs::exists(&arg) {
                add_file(&git_dir, &rel_path, &mut index);
            } else {
                terminal::print("fatal: pathspec '");
                terminal::print(arg);
                terminal::println("' did not match any files");
            }
        }
    }

    write_index(&git_dir, &index);
}

/// Add a single file to the index.
fn add_file(git_dir: &str, rel_path: &str, index: &mut Vec<IndexEntry>) {
    let root = work_tree_root(git_dir);
    let abs_path = if root.is_empty() {
        rel_path.to_string()
    } else {
        format!("{}/{}", root, rel_path)
    };

    let content = match fs::read_file_bytes_absolute(&abs_path) {
        Some(c) => c,
        None => {
            terminal::print("error: cannot read '");
            terminal::print(rel_path);
            terminal::println("'");
            return;
        }
    };

    let blob_id = match write_object(git_dir, &content, "blob") {
        Ok(id) => id,
        Err(e) => {
            terminal::println(&e);
            return;
        }
    };

    // Update or add to index
    if let Some(entry) = index.iter_mut().find(|e| e.name == rel_path) {
        entry.hash = blob_id;
    } else {
        index.push(IndexEntry {
            mode: "100644".to_string(),
            hash: blob_id,
            name: rel_path.to_string(),
        });
    }
    // Keep index sorted
    index.sort_by(|a, b| a.name.cmp(&b.name));
}

/// Recursively add all files in a directory, respecting .gitignore.
fn add_directory(git_dir: &str, root: &str, dir_path: &str, index: &mut Vec<IndexEntry>, ignore: &GitIgnore) {
    let entries = fs::list_dir_absolute(dir_path);
    for entry in &entries {
        let child = if dir_path.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", dir_path, entry.name)
        };

        // Skip .git directory
        if entry.name == ".git" { continue; }

        // Compute path relative to repo root for .gitignore matching
        let rel = if root.is_empty() {
            child.clone()
        } else if child.starts_with(root) && child.len() > root.len() {
            child[root.len()+1..].to_string()
        } else {
            child.clone()
        };

        // Check .gitignore
        if ignore.is_ignored(&rel, entry.is_dir) { continue; }

        if entry.is_dir {
            add_directory(git_dir, root, &child, index, ignore);
        } else {
            add_file(git_dir, &rel, index);
        }
    }
}

pub fn cmd_status() {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { terminal::println(&e); return; }
    };
    let root = work_tree_root(&git_dir);

    // Show current branch
    match current_branch(&git_dir) {
        Some(branch) => {
            terminal::print("On branch ");
            terminal::println(&branch);
        }
        None => terminal::println("HEAD detached"),
    }

    let index = read_index(&git_dir);

    // Get committed files (from HEAD tree)
    let committed = match resolve_head(&git_dir) {
        Some(commit_id) => {
            match read_object(&git_dir, &commit_id) {
                Ok((_, commit_data)) => {
                    let commit_str = String::from_utf8_lossy(&commit_data);
                    if let Some(tree_line) = commit_str.lines().next() {
                        if let Some(tree_hash) = tree_line.strip_prefix("tree ") {
                            if let Some(tree_id) = ObjectId::from_hex(tree_hash) {
                                flatten_tree(&git_dir, &tree_id, "").unwrap_or_default()
                            } else { Vec::new() }
                        } else { Vec::new() }
                    } else { Vec::new() }
                }
                Err(_) => Vec::new(),
            }
        }
        None => {
            terminal::println("\nNo commits yet\n");
            Vec::new()
        }
    };

    // Changes staged for commit (index vs HEAD)
    let mut staged_new: Vec<String> = Vec::new();
    let mut staged_modified: Vec<String> = Vec::new();
    let mut staged_deleted: Vec<String> = Vec::new();

    for entry in &index {
        match committed.iter().find(|(name, _)| name == &entry.name) {
            Some((_, old_hash)) if *old_hash != entry.hash => {
                staged_modified.push(entry.name.clone());
            }
            None => {
                staged_new.push(entry.name.clone());
            }
            _ => {} // unchanged
        }
    }
    for (name, _) in &committed {
        if !index.iter().any(|e| e.name == *name) {
            staged_deleted.push(name.clone());
        }
    }

    if !staged_new.is_empty() || !staged_modified.is_empty() || !staged_deleted.is_empty() {
        terminal::println("Changes to be committed:");
        for f in &staged_new { terminal::print("  new file:   "); terminal::println(f); }
        for f in &staged_modified { terminal::print("  modified:   "); terminal::println(f); }
        for f in &staged_deleted { terminal::print("  deleted:    "); terminal::println(f); }
        terminal::println("");
    }

    // Untracked files (working tree files not in index)
    let ignore = GitIgnore::load(&git_dir);
    let mut untracked: Vec<String> = Vec::new();
    collect_working_files(&root, &root, &mut untracked, &ignore);
    let untracked: Vec<String> = untracked.into_iter()
        .filter(|f| !index.iter().any(|e| e.name == *f))
        .collect();

    if !untracked.is_empty() {
        terminal::println("Untracked files:");
        for f in &untracked {
            terminal::print("  ");
            terminal::println(f);
        }
        terminal::println("");
    }

    if staged_new.is_empty() && staged_modified.is_empty() && staged_deleted.is_empty() && untracked.is_empty() {
        terminal::println("nothing to commit, working tree clean");
    }
}

/// Collect all files in the working tree (relative to repo root), respecting .gitignore.
fn collect_working_files(root: &str, dir: &str, result: &mut Vec<String>, ignore: &GitIgnore) {
    let entries = fs::list_dir_absolute(dir);
    for entry in &entries {
        if entry.name == ".git" { continue; }
        let child = if dir.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", dir, entry.name)
        };

        let rel = if root.is_empty() {
            child.clone()
        } else if child.starts_with(root) && child.len() > root.len() {
            child[root.len()+1..].to_string()
        } else {
            child.clone()
        };

        // Check .gitignore
        if ignore.is_ignored(&rel, entry.is_dir) { continue; }

        if entry.is_dir {
            collect_working_files(root, &child, result, ignore);
        } else {
            result.push(rel);
        }
    }
}

pub fn cmd_commit(args: &str) {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { terminal::println(&e); return; }
    };

    // Parse -m "message"
    let message = if args.trim_start().starts_with("-m") {
        let rest = args.trim_start()[2..].trim();
        // Handle both -m "msg" and -m msg
        if rest.starts_with('"') && rest.len() > 1 {
            if let Some(end) = rest[1..].find('"') {
                &rest[1..1+end]
            } else {
                &rest[1..]
            }
        } else {
            rest
        }
    } else if args.trim().is_empty() {
        terminal::println("error: must provide commit message with -m");
        return;
    } else {
        args.trim()
    };

    let index = read_index(&git_dir);
    if index.is_empty() {
        terminal::println("nothing to commit");
        return;
    }

    // Build tree from index
    let tree_id = match build_tree(&git_dir, &index, "") {
        Ok(id) => id,
        Err(e) => { terminal::println(&e); return; }
    };

    // Build commit object
    let parent = resolve_head(&git_dir);
    let timestamp = unix_timestamp();
    let author = format!("Terminal User <user@terminal.os> {} +0000", timestamp);

    let mut commit_content = String::new();
    commit_content.push_str(&format!("tree {}\n", tree_id.to_hex()));
    if let Some(ref parent_id) = parent {
        commit_content.push_str(&format!("parent {}\n", parent_id.to_hex()));
    }
    commit_content.push_str(&format!("author {}\n", author));
    commit_content.push_str(&format!("committer {}\n", author));
    commit_content.push_str(&format!("\n{}\n", message));

    let commit_id = match write_object(&git_dir, commit_content.as_bytes(), "commit") {
        Ok(id) => id,
        Err(e) => { terminal::println(&e); return; }
    };

    // Update HEAD ref
    update_head_ref(&git_dir, &commit_id);

    let branch = current_branch(&git_dir).unwrap_or_else(|| "HEAD".to_string());
    terminal::print("[");
    terminal::print(&branch);
    terminal::print(" ");
    terminal::print(&commit_id.to_hex()[..7]);
    terminal::print("] ");
    terminal::println(message);

    // Count files
    terminal::print(" ");
    let count = index.len();
    if count == 1 {
        terminal::println("1 file changed");
    } else {
        terminal::print(&format!("{}", count));
        terminal::println(" files changed");
    }
}

pub fn cmd_log() {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { terminal::println(&e); return; }
    };

    let mut current = match resolve_head(&git_dir) {
        Some(id) => id,
        None => {
            terminal::println("fatal: no commits yet");
            return;
        }
    };

    let head_branch = current_branch(&git_dir);
    let mut is_first = true;

    loop {
        let (obj_type, data) = match read_object(&git_dir, &current) {
            Ok(r) => r,
            Err(e) => { terminal::println(&e); break; }
        };
        if obj_type != "commit" {
            terminal::println("error: expected commit object");
            break;
        }

        let commit_str = String::from_utf8_lossy(&data).to_string();

        // Print commit hash
        terminal::print("commit ");
        terminal::print(&current.to_hex());
        if is_first {
            if let Some(ref branch) = head_branch {
                terminal::print(" (HEAD -> ");
                terminal::print(branch);
                terminal::print(")");
            }
        }
        terminal::println("");

        // Parse and print author + message
        let mut parent_hash: Option<String> = None;
        let mut in_message = false;
        for line in commit_str.lines() {
            if in_message {
                if !line.is_empty() {
                    terminal::print("    ");
                    terminal::println(line);
                }
            } else if line.is_empty() {
                in_message = true;
            } else if line.starts_with("author ") {
                terminal::print("Author: ");
                // Parse "author Name <email> timestamp tz"
                let author_part = &line[7..];
                if let Some(angle) = author_part.find('>') {
                    terminal::println(&author_part[..angle+1]);
                } else {
                    terminal::println(author_part);
                }
            } else if line.starts_with("parent ") {
                parent_hash = Some(line[7..].to_string());
            }
        }
        terminal::println("");

        // Follow parent chain
        match parent_hash {
            Some(hash) => {
                match ObjectId::from_hex(&hash) {
                    Some(id) => current = id,
                    None => break,
                }
            }
            None => break, // No parent = initial commit
        }
        is_first = false;
    }
}

pub fn cmd_branch(args: &str) {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { terminal::println(&e); return; }
    };

    let name = args.trim();

    if name.is_empty() {
        // List branches
        let current = current_branch(&git_dir);
        let refs_dir = format!("{}/refs/heads", git_dir);
        let entries = fs::list_dir_absolute(&refs_dir);

        if entries.is_empty() {
            // No branches yet, show what HEAD points to
            if let Some(branch) = &current {
                terminal::print("* ");
                terminal::println(branch);
            }
            return;
        }

        for entry in &entries {
            if !entry.is_dir {
                if current.as_deref() == Some(&entry.name) {
                    terminal::print("* ");
                } else {
                    terminal::print("  ");
                }
                terminal::println(&entry.name);
            }
        }
    } else {
        // Create branch
        let commit_id = match resolve_head(&git_dir) {
            Some(id) => id,
            None => {
                terminal::println("fatal: not a valid object name: no commits yet");
                return;
            }
        };

        let ref_path = format!("{}/refs/heads/{}", git_dir, name);
        if fs::exists_absolute(&ref_path) {
            terminal::print("fatal: branch '");
            terminal::print(name);
            terminal::println("' already exists");
            return;
        }

        fs::write_file_absolute(&ref_path, &format!("{}\n", commit_id.to_hex()));
    }
}

pub fn cmd_checkout(args: &str) {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { terminal::println(&e); return; }
    };

    let target = args.trim();
    if target.is_empty() {
        terminal::println("error: specify a branch to checkout");
        return;
    }

    // Check if target is a branch
    let ref_path = format!("{}/refs/heads/{}", git_dir, target);
    if !fs::exists_absolute(&ref_path) {
        terminal::print("error: branch '");
        terminal::print(target);
        terminal::println("' not found");
        return;
    }

    // Read target commit
    let target_commit = match read_ref(&git_dir, target) {
        Some(id) => id,
        None => {
            terminal::println("error: invalid branch ref");
            return;
        }
    };

    // Update HEAD to point to the branch
    fs::write_file_absolute(
        &format!("{}/HEAD", git_dir),
        &format!("ref: refs/heads/{}\n", target),
    );

    // Update index to match the target commit's tree
    let (_, commit_data) = match read_object(&git_dir, &target_commit) {
        Ok(r) => r,
        Err(e) => { terminal::println(&e); return; }
    };
    let commit_str = String::from_utf8_lossy(&commit_data);
    if let Some(tree_line) = commit_str.lines().next() {
        if let Some(tree_hash) = tree_line.strip_prefix("tree ") {
            if let Some(tree_id) = ObjectId::from_hex(tree_hash) {
                let files = flatten_tree(&git_dir, &tree_id, "").unwrap_or_default();
                let entries: Vec<IndexEntry> = files.into_iter().map(|(name, hash)| {
                    IndexEntry { mode: "100644".to_string(), hash, name }
                }).collect();
                write_index(&git_dir, &entries);
            }
        }
    }

    terminal::print("Switched to branch '");
    terminal::print(target);
    terminal::println("'");
}

pub fn cmd_diff() {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { terminal::println(&e); return; }
    };
    let root = work_tree_root(&git_dir);

    let index = read_index(&git_dir);
    let mut has_diff = false;

    for entry in &index {
        // Read current file content
        let abs_path = if root.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", root, entry.name)
        };

        let current_content = match fs::read_file_bytes_absolute(&abs_path) {
            Some(c) => c,
            None => {
                // File deleted
                terminal::print("diff --git a/");
                terminal::print(&entry.name);
                terminal::print(" b/");
                terminal::println(&entry.name);
                terminal::println("deleted file");
                terminal::println("");
                has_diff = true;
                continue;
            }
        };

        // Hash current content and compare to index
        let current_hash = hash_object(&current_content, "blob");
        if current_hash == entry.hash {
            continue; // No changes
        }

        has_diff = true;

        // Show diff header
        terminal::print("diff --git a/");
        terminal::print(&entry.name);
        terminal::print(" b/");
        terminal::println(&entry.name);
        terminal::print("index ");
        terminal::print(&entry.hash.to_hex()[..7]);
        terminal::print("..");
        terminal::println(&current_hash.to_hex()[..7]);

        // Read indexed version
        let old_content = match read_object(&git_dir, &entry.hash) {
            Ok((_, data)) => String::from_utf8_lossy(&data).to_string(),
            Err(_) => String::new(),
        };
        let new_content = String::from_utf8_lossy(&current_content).to_string();

        // Simple line-by-line diff
        let old_lines: Vec<&str> = old_content.lines().collect();
        let new_lines: Vec<&str> = new_content.lines().collect();

        terminal::print("--- a/");
        terminal::println(&entry.name);
        terminal::print("+++ b/");
        terminal::println(&entry.name);

        // Very simple diff: show removed and added lines
        for line in &old_lines {
            if !new_lines.contains(line) {
                terminal::print("-");
                terminal::println(line);
            }
        }
        for line in &new_lines {
            if !old_lines.contains(line) {
                terminal::print("+");
                terminal::println(line);
            }
        }
        terminal::println("");
    }

    if !has_diff {
        // No output (matches real git behavior)
    }
}

// ============================================================
// Ref spec parsing (HEAD~N)
// ============================================================

/// Resolve a ref spec like "HEAD~3", "HEAD", or a branch name to a commit ID.
/// Prints diagnostic messages on failure to help debug issues.
fn resolve_ref_spec(git_dir: &str, spec: &str) -> Option<ObjectId> {
    if spec == "HEAD" {
        return resolve_head(git_dir);
    }

    if spec.starts_with("HEAD~") {
        let n: usize = match spec[5..].parse() {
            Ok(n) => n,
            Err(_) => {
                terminal::println("error: invalid number after HEAD~");
                return None;
            }
        };
        let mut current = match resolve_head(git_dir) {
            Some(id) => id,
            None => {
                terminal::println("error: cannot resolve HEAD (is .git/HEAD readable?)");
                return None;
            }
        };
        for i in 0..n {
            let (_, data) = match read_object(git_dir, &current) {
                Ok(r) => r,
                Err(e) => {
                    terminal::print(&format!("error: cannot read commit {} at step {}: ", &current.to_hex()[..7], i + 1));
                    terminal::println(&e);
                    return None;
                }
            };
            let info = match parse_commit(&data) {
                Ok(i) => i,
                Err(e) => {
                    terminal::print(&format!("error: cannot parse commit {} at step {}: ", &current.to_hex()[..7], i + 1));
                    terminal::println(&e);
                    return None;
                }
            };
            match info.parent {
                Some(p) => current = p,
                None => {
                    terminal::println(&format!(
                        "fatal: HEAD~{} goes beyond the root commit (only {} commits in history, max is HEAD~{})",
                        n, i + 1, i
                    ));
                    return None;
                }
            }
        }
        return Some(current);
    }

    // Try as branch name
    read_ref(git_dir, spec)
}

// ============================================================
// Interactive rebase state
// ============================================================

const REBASE_DIR: &str = "rebase-merge";

fn rebase_state_dir(git_dir: &str) -> String {
    format!("{}/{}", git_dir, REBASE_DIR)
}

fn is_rebase_in_progress(git_dir: &str) -> bool {
    fs::exists_absolute(&format!("{}/git-rebase-todo", rebase_state_dir(git_dir)))
}

/// Write the interactive rebase state files.
/// `onto_str` is either a hex hash or "ROOT" for --root rebases.
fn write_rebase_state(
    git_dir: &str,
    branch: &str,
    orig_head: &ObjectId,
    onto_str: &str,
    todo_content: &str,
) {
    let state_dir = rebase_state_dir(git_dir);
    fs::mkdir_absolute(&state_dir);
    fs::write_file_absolute(&format!("{}/head-name", state_dir), branch);
    fs::write_file_absolute(&format!("{}/orig-head", state_dir), &orig_head.to_hex());
    fs::write_file_absolute(&format!("{}/onto", state_dir), onto_str);
    fs::write_file_absolute(&format!("{}/git-rebase-todo", state_dir), todo_content);
}

/// Clean up rebase state files.
fn cleanup_rebase_state(git_dir: &str) {
    let state_dir = rebase_state_dir(git_dir);
    // Delete all state files
    for name in &["head-name", "orig-head", "onto", "git-rebase-todo", "current-tip", "stopped-at"] {
        fs::delete_absolute(&format!("{}/{}", state_dir, name));
    }
    fs::delete_absolute(&state_dir);
    // Also clean up legacy REBASE_HEAD
    fs::delete_absolute(&format!("{}/REBASE_HEAD", git_dir));
}

/// Generate the todo file content for interactive rebase.
fn generate_todo(git_dir: &str, commits: &[ObjectId]) -> String {
    let mut content = String::new();
    for commit_id in commits {
        if let Ok((_, data)) = read_object(git_dir, commit_id) {
            if let Ok(info) = parse_commit(&data) {
                // Truncate message to first line
                let first_line = info.message.lines().next().unwrap_or(&info.message);
                content.push_str(&format!(
                    "pick {} {}\n",
                    &commit_id.to_hex()[..7],
                    first_line
                ));
            }
        }
    }
    content.push_str("\n# Rebase interactive — edit this file then save and exit.\n");
    content.push_str("# Run 'git rebase --continue' to execute, or 'git rebase --abort' to cancel.\n");
    content.push_str("#\n");
    content.push_str("# Commands:\n");
    content.push_str("# p, pick   = use commit\n");
    content.push_str("# s, squash = meld into previous commit (combine messages)\n");
    content.push_str("# f, fixup  = like squash but discard this commit's message\n");
    content.push_str("# d, drop   = remove commit\n");
    content.push_str("# r, reword = use commit but edit the message\n");
    content.push_str("# e, edit   = stop after this commit for amending\n");
    content.push_str("#\n");
    content.push_str("# Reorder lines to reorder commits.\n");
    content.push_str("# Lines starting with # are ignored.\n");
    content
}

/// A parsed todo entry.
struct TodoEntry {
    action: String,
    hash_prefix: String,
    message: String,
}

/// Parse the edited todo file into entries.
fn parse_todo(git_dir: &str) -> Vec<TodoEntry> {
    let state_dir = rebase_state_dir(git_dir);
    let content = match fs::read_file_absolute(&format!("{}/git-rebase-todo", state_dir)) {
        Some(c) => c.to_string(),
        None => return Vec::new(),
    };

    let mut entries = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        if parts.len() >= 2 {
            entries.push(TodoEntry {
                action: parts[0].to_string(),
                hash_prefix: parts[1].to_string(),
                message: if parts.len() >= 3 { parts[2].to_string() } else { String::new() },
            });
        }
    }
    entries
}

/// Find a commit by hash prefix (first 7+ chars).
fn find_commit_by_prefix(git_dir: &str, prefix: &str, candidates: &[ObjectId]) -> Option<ObjectId> {
    for id in candidates {
        if id.to_hex().starts_with(prefix) {
            return Some(id.clone());
        }
    }
    // If not in candidates, try walking from orig-head
    let state_dir = rebase_state_dir(git_dir);
    if let Some(orig_hex) = fs::read_file_absolute(&format!("{}/orig-head", state_dir)) {
        if let Some(orig_id) = ObjectId::from_hex(orig_hex.trim()) {
            let mut current = orig_id;
            for _ in 0..100 {
                if current.to_hex().starts_with(prefix) {
                    return Some(current);
                }
                if let Ok((_, data)) = read_object(git_dir, &current) {
                    if let Ok(info) = parse_commit(&data) {
                        match info.parent {
                            Some(p) => current = p,
                            None => break,
                        }
                    } else { break; }
                } else { break; }
            }
        }
    }
    None
}

/// Execute the interactive rebase todo.
fn execute_rebase_todo(git_dir: &str) {
    let state_dir = rebase_state_dir(git_dir);

    let onto_hex = match fs::read_file_absolute(&format!("{}/onto", state_dir)) {
        Some(s) => s.trim().to_string(),
        None => { terminal::println("error: no rebase in progress"); return; }
    };
    let root_mode = onto_hex == "ROOT";
    let onto: Option<ObjectId> = if root_mode {
        None
    } else {
        match ObjectId::from_hex(&onto_hex) {
            Some(id) => Some(id),
            None => { terminal::println("error: invalid onto ref"); return; }
        }
    };

    let orig_hex = match fs::read_file_absolute(&format!("{}/orig-head", state_dir)) {
        Some(s) => s.trim().to_string(),
        None => { terminal::println("error: missing orig-head"); return; }
    };
    let orig_head = match ObjectId::from_hex(&orig_hex) {
        Some(id) => id,
        None => { terminal::println("error: invalid orig-head"); return; }
    };

    // Collect original commits for hash lookup
    let all_commits = if root_mode {
        collect_all_commits(git_dir, &orig_head).unwrap_or_default()
    } else {
        collect_commits(git_dir, &orig_head, onto.as_ref().unwrap()).unwrap_or_default()
    };

    let entries = parse_todo(git_dir);
    if entries.is_empty() {
        terminal::println("Nothing to do (empty todo).");
        cleanup_rebase_state(git_dir);
        return;
    }

    // Check if we have a saved current-tip (from a previous partial run)
    let mut current_tip: Option<ObjectId> = match fs::read_file_absolute(&format!("{}/current-tip", state_dir)) {
        Some(s) => ObjectId::from_hex(s.trim()).or(onto.clone()),
        None => onto.clone(),
    };

    // Track the last commit's message for squash/fixup
    let mut last_message = String::new();
    let mut pending_squash_messages: Vec<String> = Vec::new();

    terminal::print("Executing rebase (");
    terminal::print(&format!("{}", entries.len()));
    terminal::println(" steps)...");

    for (i, entry) in entries.iter().enumerate() {
        let action = match entry.action.as_str() {
            "p" | "pick" => "pick",
            "s" | "squash" => "squash",
            "f" | "fixup" => "fixup",
            "d" | "drop" => "drop",
            "r" | "reword" => "reword",
            "e" | "edit" => "edit",
            other => {
                terminal::print("warning: unknown action '");
                terminal::print(other);
                terminal::println("', treating as pick");
                "pick"
            }
        };

        if action == "drop" {
            terminal::print("  ");
            terminal::print(&format!("{}/{}", i + 1, entries.len()));
            terminal::print(" drop ");
            terminal::println(&entry.hash_prefix);
            continue;
        }

        let commit_id = match find_commit_by_prefix(git_dir, &entry.hash_prefix, &all_commits) {
            Some(id) => id,
            None => {
                terminal::print("error: could not find commit ");
                terminal::println(&entry.hash_prefix);
                terminal::println("Aborting rebase.");
                abort_rebase(git_dir);
                return;
            }
        };

        match cherry_pick(git_dir, &commit_id, current_tip.as_ref()) {
            Ok(new_id) => {
                // Read the new commit's message
                let msg = if let Ok((_, data)) = read_object(git_dir, &commit_id) {
                    parse_commit(&data).map(|i| i.message).unwrap_or_default()
                } else {
                    String::new()
                };

                match action {
                    "pick" => {
                        // Flush any pending squash
                        if !pending_squash_messages.is_empty() {
                            if let Some(ref tip) = current_tip {
                                current_tip = Some(amend_commit_message(git_dir, tip, &pending_squash_messages.join("\n\n")));
                            }
                            pending_squash_messages.clear();
                        }
                        current_tip = Some(new_id);
                        last_message = msg;
                        terminal::print("  ");
                        terminal::print(&format!("{}/{}", i + 1, entries.len()));
                        terminal::print(" pick ");
                        terminal::println(&current_tip.as_ref().unwrap().to_hex()[..7]);
                    }
                    "squash" | "fixup" => {
                        // Squash/fixup: merge this commit's changes into the
                        // previous commit.
                        let prev_info = current_tip.as_ref()
                            .and_then(|tip| read_object(git_dir, tip).ok())
                            .and_then(|(_, d)| parse_commit(&d).ok());
                        let parent_of_prev = prev_info.as_ref()
                            .and_then(|i| i.parent.clone());

                        // Cherry-pick onto parent of previous (combining changes)
                        let combined_id = match cherry_pick(git_dir, &commit_id, parent_of_prev.as_ref()) {
                            Ok(id) => id,
                            Err(_) => new_id,
                        };

                        let combined_msg = if action == "squash" {
                            if pending_squash_messages.is_empty() {
                                pending_squash_messages.push(last_message.clone());
                            }
                            pending_squash_messages.push(msg.clone());
                            pending_squash_messages.join("\n\n")
                        } else {
                            if !pending_squash_messages.is_empty() {
                                pending_squash_messages.join("\n\n")
                            } else {
                                last_message.clone()
                            }
                        };

                        // Build final commit with combined tree and message
                        if let Ok((_, cd)) = read_object(git_dir, &combined_id) {
                            if let Ok(ci) = parse_commit(&cd) {
                                let timestamp = unix_timestamp();
                                let committer = format!("Terminal User <user@terminal.os> {} +0000", timestamp);
                                let prev_author = prev_info.as_ref()
                                    .map(|i| i.author.clone())
                                    .unwrap_or(committer.clone());
                                let mut content = String::new();
                                content.push_str(&format!("tree {}\n", ci.tree.to_hex()));
                                if let Some(ref pop) = parent_of_prev {
                                    content.push_str(&format!("parent {}\n", pop.to_hex()));
                                }
                                content.push_str(&format!("author {}\n", prev_author));
                                content.push_str(&format!("committer {}\n", committer));
                                content.push_str(&format!("\n{}\n", combined_msg));
                                if let Ok(final_id) = write_object(git_dir, content.as_bytes(), "commit") {
                                    current_tip = Some(final_id);
                                }
                            }
                        }

                        if action == "fixup" {
                            last_message = combined_msg;
                        }

                        terminal::print("  ");
                        terminal::print(&format!("{}/{}", i + 1, entries.len()));
                        terminal::print(if action == "squash" { " squash " } else { " fixup " });
                        terminal::println(&current_tip.as_ref().unwrap().to_hex()[..7]);
                    }
                    "reword" => {
                        if !pending_squash_messages.is_empty() {
                            if let Some(ref tip) = current_tip {
                                current_tip = Some(amend_commit_message(git_dir, tip, &pending_squash_messages.join("\n\n")));
                            }
                            pending_squash_messages.clear();
                        }
                        current_tip = Some(new_id);
                        terminal::print("  ");
                        terminal::print(&format!("{}/{}", i + 1, entries.len()));
                        terminal::println(" reword — enter new message:");
                        let new_msg = terminal::read_line("  message: ");
                        if !new_msg.is_empty() {
                            if let Some(ref tip) = current_tip {
                                current_tip = Some(amend_commit_message(git_dir, tip, &new_msg));
                            }
                        }
                        last_message = new_msg;
                    }
                    "edit" => {
                        if !pending_squash_messages.is_empty() {
                            if let Some(ref tip) = current_tip {
                                current_tip = Some(amend_commit_message(git_dir, tip, &pending_squash_messages.join("\n\n")));
                            }
                            pending_squash_messages.clear();
                        }
                        current_tip = Some(new_id);
                        if let Some(ref tip) = current_tip {
                            fs::write_file_absolute(
                                &format!("{}/current-tip", state_dir),
                                &tip.to_hex(),
                            );
                            update_head_ref(git_dir, tip);
                        }
                        let remaining = &entries[i+1..];
                        let mut new_todo = String::new();
                        for r in remaining {
                            new_todo.push_str(&format!("{} {} {}\n", r.action, r.hash_prefix, r.message));
                        }
                        fs::write_file_absolute(
                            &format!("{}/git-rebase-todo", state_dir),
                            &new_todo,
                        );
                        terminal::print("  ");
                        terminal::print(&format!("{}/{}", i + 1, entries.len()));
                        terminal::println(" edit — stopped for editing");
                        terminal::println("Amend the commit, then run 'git rebase --continue'");
                        return;
                    }
                    _ => {
                        current_tip = Some(new_id);
                        last_message = msg;
                    }
                }
            }
            Err(e) => {
                terminal::print("error: ");
                terminal::println(&e);
                terminal::println("Aborting rebase.");
                abort_rebase(git_dir);
                return;
            }
        }
    }

    // Flush any remaining pending squash
    if !pending_squash_messages.is_empty() {
        if let Some(ref tip) = current_tip {
            current_tip = Some(amend_commit_message(git_dir, tip, &pending_squash_messages.join("\n\n")));
        }
    }

    // Success
    if let Some(ref tip) = current_tip {
        update_head_ref(git_dir, tip);
        update_index_to_commit(git_dir, tip);
    }
    cleanup_rebase_state(git_dir);
    terminal::println("Successfully rebased.");
}

/// Amend a commit's message, returning the new commit ID.
fn amend_commit_message(git_dir: &str, commit_id: &ObjectId, new_message: &str) -> ObjectId {
    let (_, data) = match read_object(git_dir, commit_id) {
        Ok(r) => r,
        Err(_) => return commit_id.clone(),
    };
    let info = match parse_commit(&data) {
        Ok(i) => i,
        Err(_) => return commit_id.clone(),
    };

    let mut content = String::new();
    content.push_str(&format!("tree {}\n", info.tree.to_hex()));
    if let Some(ref parent) = info.parent {
        content.push_str(&format!("parent {}\n", parent.to_hex()));
    }
    content.push_str(&format!("author {}\n", info.author));
    content.push_str(&format!("committer {}\n", info.committer));
    content.push_str(&format!("\n{}\n", new_message));

    match write_object(git_dir, content.as_bytes(), "commit") {
        Ok(id) => id,
        Err(_) => commit_id.clone(),
    }
}

/// Update the index to match a commit's tree.
fn update_index_to_commit(git_dir: &str, commit_id: &ObjectId) {
    if let Ok((_, data)) = read_object(git_dir, commit_id) {
        if let Ok(info) = parse_commit(&data) {
            let files = flatten_tree(git_dir, &info.tree, "").unwrap_or_default();
            let entries: Vec<IndexEntry> = files.into_iter().map(|(name, hash)| {
                IndexEntry { mode: "100644".to_string(), hash, name }
            }).collect();
            write_index(git_dir, &entries);
        }
    }
}

/// Abort an interactive rebase: restore original branch position.
fn abort_rebase(git_dir: &str) {
    let state_dir = rebase_state_dir(git_dir);

    if let Some(orig_hex) = fs::read_file_absolute(&format!("{}/orig-head", state_dir)) {
        if let Some(orig_id) = ObjectId::from_hex(orig_hex.trim()) {
            update_head_ref(git_dir, &orig_id);
            update_index_to_commit(git_dir, &orig_id);
        }
    }

    cleanup_rebase_state(git_dir);
    terminal::println("Rebase aborted and branch restored.");
}

/// Public entry point: checks if the todo file exists for the editor hint.
pub fn has_rebase_in_progress() -> bool {
    match find_git_dir() {
        Some(git_dir) => is_rebase_in_progress(&git_dir),
        None => false,
    }
}

/// Get the todo file path for opening in the editor.
pub fn rebase_todo_path() -> Option<String> {
    let git_dir = find_git_dir()?;
    let path = format!("{}/{}/git-rebase-todo", git_dir, REBASE_DIR);
    if fs::exists_absolute(&path) {
        Some(path)
    } else {
        None
    }
}

pub fn cmd_rebase(args: &str) {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { terminal::println(&e); return; }
    };

    let args = args.trim();

    // Handle --continue
    if args == "--continue" {
        if !is_rebase_in_progress(&git_dir) {
            terminal::println("error: no rebase in progress");
            return;
        }
        execute_rebase_todo(&git_dir);
        return;
    }

    // Handle --abort
    if args == "--abort" {
        if !is_rebase_in_progress(&git_dir) {
            terminal::println("error: no rebase in progress");
            return;
        }
        abort_rebase(&git_dir);
        return;
    }

    // Check if rebase already in progress
    if is_rebase_in_progress(&git_dir) {
        terminal::println("error: rebase already in progress");
        terminal::println("Use 'git rebase --continue' or 'git rebase --abort'");
        return;
    }

    // Parse flags: -i (interactive), --root (rebase all commits)
    let mut interactive = false;
    let mut root_mode = false;
    let mut target_spec = "";

    for part in args.split_whitespace() {
        match part {
            "-i" => interactive = true,
            "--root" => root_mode = true,
            _ => {
                if target_spec.is_empty() {
                    target_spec = part;
                }
            }
        }
    }

    if !root_mode && target_spec.is_empty() {
        terminal::println("usage: git rebase [-i] [--root | <branch|HEAD~N>]");
        return;
    }

    // Must be on a branch
    let current = match current_branch(&git_dir) {
        Some(b) => b,
        None => {
            terminal::println("fatal: cannot rebase with detached HEAD");
            return;
        }
    };

    let head_id = match resolve_head(&git_dir) {
        Some(id) => id,
        None => {
            terminal::println("fatal: no commits on current branch");
            return;
        }
    };

    // Resolve onto target and collect commits
    let onto_id: Option<ObjectId>; // None = root (no parent)
    let commits;

    if root_mode {
        // --root: rebase all commits, onto = empty (new root)
        onto_id = None;
        commits = match collect_all_commits(&git_dir, &head_id) {
            Ok(c) => c,
            Err(e) => { terminal::println(&e); return; }
        };
    } else {
        let target_id = match resolve_ref_spec(&git_dir, target_spec) {
            Some(id) => id,
            None => {
                terminal::print("fatal: invalid ref '");
                terminal::print(target_spec);
                terminal::println("'");
                return;
            }
        };

        // For non-interactive rebase onto a branch, check same-branch
        if !interactive && !target_spec.starts_with("HEAD~") && current == target_spec {
            terminal::println("fatal: cannot rebase a branch onto itself");
            return;
        }

        if target_spec.starts_with("HEAD~") {
            // HEAD~N: rebase the last N commits onto the Nth ancestor
            onto_id = Some(target_id.clone());
            commits = match collect_commits(&git_dir, &head_id, &target_id) {
                Ok(c) => c,
                Err(e) => { terminal::println(&e); return; }
            };
        } else {
            // Branch name: find merge-base
            let merge_base = match find_merge_base(&git_dir, &head_id, &target_id) {
                Some(mb) => mb,
                None => {
                    terminal::println("fatal: no common ancestor found");
                    return;
                }
            };

            if head_id == target_id {
                terminal::println("Current branch is up to date.");
                return;
            }
            if merge_base == head_id {
                update_head_ref(&git_dir, &target_id);
                update_index_to_commit(&git_dir, &target_id);
                terminal::print("Fast-forwarded to ");
                terminal::println(target_spec);
                return;
            }
            if merge_base == target_id {
                terminal::println("Current branch is up to date.");
                return;
            }

            onto_id = Some(target_id.clone());
            commits = match collect_commits(&git_dir, &head_id, &merge_base) {
                Ok(c) => c,
                Err(e) => { terminal::println(&e); return; }
            };
        }
    }

    if commits.is_empty() {
        terminal::println("Nothing to rebase.");
        return;
    }

    // Store onto as hex string ("ROOT" for --root, hash otherwise)
    let onto_hex = match &onto_id {
        Some(id) => id.to_hex(),
        None => "ROOT".to_string(),
    };

    if interactive {
        // Interactive: write todo file, save state, open in editor
        let todo = generate_todo(&git_dir, &commits);
        write_rebase_state(&git_dir, &current, &head_id, &onto_hex, &todo);

        terminal::println("Opening rebase todo in editor...");
        terminal::println("Edit the plan, save (Ctrl+S), and exit (Ctrl+E).");
        terminal::println("Then run 'git rebase --continue' to execute.");

        let todo_path = format!("{}/{}/git-rebase-todo", git_dir, REBASE_DIR);
        unsafe {
            REBASE_EDIT_PATH_LEN = todo_path.len().min(REBASE_EDIT_PATH.len());
            REBASE_EDIT_PATH[..REBASE_EDIT_PATH_LEN].copy_from_slice(&todo_path.as_bytes()[..REBASE_EDIT_PATH_LEN]);
        }
    } else {
        // Non-interactive: execute immediately
        let orig_ref_path = format!("{}/REBASE_HEAD", git_dir);
        fs::write_file_absolute(&orig_ref_path, &head_id.to_hex());

        let label = if root_mode { "--root" } else { target_spec };
        terminal::print("Rebasing ");
        terminal::print(&format!("{}", commits.len()));
        terminal::print(" commit(s) onto ");
        terminal::print(label);
        terminal::println("...");

        let mut current_tip = onto_id.clone();
        for (i, commit_id) in commits.iter().enumerate() {
            match cherry_pick(&git_dir, commit_id, current_tip.as_ref()) {
                Ok(new_id) => {
                    terminal::print("  ");
                    terminal::print(&format!("{}/{}", i + 1, commits.len()));
                    terminal::print(" ");
                    terminal::println(&new_id.to_hex()[..7]);
                    current_tip = Some(new_id);
                }
                Err(e) => {
                    terminal::print("error: ");
                    terminal::println(&e);
                    terminal::println("Aborting rebase.");
                    update_head_ref(&git_dir, &head_id);
                    fs::delete_absolute(&orig_ref_path);
                    return;
                }
            }
        }

        if let Some(tip) = &current_tip {
            update_head_ref(&git_dir, tip);
            update_index_to_commit(&git_dir, tip);
        }
        fs::delete_absolute(&orig_ref_path);
        terminal::print("Successfully rebased onto ");
        terminal::println(label);
    }
}

pub fn cmd_debug() {
    terminal::println("=== Git Debug Info ===");

    // Check .git directory
    match find_git_dir() {
        Some(git_dir) => {
            terminal::print(".git directory: ");
            terminal::println(&git_dir);

            // Check HEAD
            match read_head(&git_dir) {
                Some(head) => {
                    terminal::print("HEAD: ");
                    terminal::println(&head);
                }
                None => terminal::println("HEAD: UNREADABLE"),
            }

            // Check current branch
            match current_branch(&git_dir) {
                Some(branch) => {
                    terminal::print("Branch: ");
                    terminal::println(&branch);
                }
                None => terminal::println("Branch: (detached or unreadable)"),
            }

            // Check HEAD commit
            match resolve_head(&git_dir) {
                Some(id) => {
                    terminal::print("HEAD commit: ");
                    terminal::println(&id.to_hex());

                    // Try to read the commit object
                    match read_object(&git_dir, &id) {
                        Ok((obj_type, data)) => {
                            terminal::print("  Object type: ");
                            terminal::println(&obj_type);
                            terminal::print("  Object size: ");
                            terminal::println(&format!("{} bytes", data.len()));

                            // Parse commit
                            match parse_commit(&data) {
                                Ok(info) => {
                                    terminal::print("  Tree: ");
                                    terminal::println(&info.tree.to_hex());
                                    match &info.parent {
                                        Some(p) => {
                                            terminal::print("  Parent: ");
                                            terminal::println(&p.to_hex());
                                        }
                                        None => terminal::println("  Parent: (none — root commit)"),
                                    }
                                    terminal::print("  Message: ");
                                    terminal::println(&info.message);
                                }
                                Err(e) => {
                                    terminal::print("  Parse error: ");
                                    terminal::println(&e);
                                    // Show raw content for debugging
                                    terminal::println("  Raw content (first 200 bytes):");
                                    let preview = String::from_utf8_lossy(&data[..data.len().min(200)]);
                                    terminal::print("  ");
                                    terminal::println(&preview);
                                }
                            }
                        }
                        Err(e) => {
                            terminal::print("  Read error: ");
                            terminal::println(&e);
                            // Check if the object file exists
                            let obj_path = format!("{}/objects/{}/{}", git_dir, id.dir(), id.file());
                            if fs::exists_absolute(&obj_path) {
                                terminal::print("  Object file exists at: ");
                                terminal::println(&obj_path);
                                match fs::read_file_bytes_absolute(&obj_path) {
                                    Some(bytes) => {
                                        terminal::print("  Raw file size: ");
                                        terminal::println(&format!("{} bytes", bytes.len()));
                                    }
                                    None => terminal::println("  Could not read raw file"),
                                }
                            } else {
                                terminal::print("  Object file MISSING: ");
                                terminal::println(&obj_path);
                            }
                        }
                    }

                    // Count commits
                    let mut count = 0;
                    let mut cur = id;
                    loop {
                        count += 1;
                        match read_object(&git_dir, &cur) {
                            Ok((_, data)) => match parse_commit(&data) {
                                Ok(info) => match info.parent {
                                    Some(p) => cur = p,
                                    None => break,
                                },
                                Err(_) => break,
                            },
                            Err(_) => break,
                        }
                    }
                    terminal::print("Total commits: ");
                    terminal::println(&format!("{}", count));
                }
                None => terminal::println("HEAD commit: UNRESOLVABLE"),
            }

            // List index
            let index = read_index(&git_dir);
            terminal::print("Index entries: ");
            terminal::println(&format!("{}", index.len()));
        }
        None => terminal::println("Not a git repository"),
    }

    terminal::println("=== End Debug ===");
}

/// Static buffer for passing the rebase todo path to lib.rs for editor opening.
static mut REBASE_EDIT_PATH: [u8; 256] = [0u8; 256];
static mut REBASE_EDIT_PATH_LEN: usize = 0;

/// Check if a rebase -i just requested the editor to open, and return the path.
/// Clears the request after reading.
pub fn take_rebase_edit_request() -> Option<String> {
    unsafe {
        if REBASE_EDIT_PATH_LEN > 0 {
            let path = std::str::from_utf8_unchecked(&REBASE_EDIT_PATH[..REBASE_EDIT_PATH_LEN]).to_string();
            REBASE_EDIT_PATH_LEN = 0;
            Some(path)
        } else {
            None
        }
    }
}
