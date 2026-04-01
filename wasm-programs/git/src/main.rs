//! Minimal git implementation using standard git object format.
//!
//! Produces GitHub-compatible repositories. Objects are stored as
//! zlib-compressed blobs addressed by SHA-1 hash, matching real git.
//!
//! The index uses a simplified text format (not binary) since it
//! never transfers to remotes -- only objects and refs do.

use sha1::{Sha1, Digest};
use flate2::write::ZlibEncoder;
use flate2::read::ZlibDecoder;
use flate2::Compression;
use std::io::{Read, Write};

fn shell_read_line(prompt: &str) -> String {
    print!("{}", prompt);
    let _ = std::io::stdout().flush();
    let mut buf = String::new();
    let _ = std::io::stdin().read_line(&mut buf);
    buf.trim_end().to_string()
}

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

        let content = match std::fs::read_to_string(&path).ok() {
            Some(c) => c,
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
fn glob_match(pattern: &str, path: &str) -> bool {
    if !pattern.contains('/') {
        let basename = path.rsplit('/').next().unwrap_or(path);
        if glob_match_segment(pattern, basename) {
            return true;
        }
        for component in path.split('/') {
            if glob_match_segment(pattern, component) {
                return true;
            }
        }
        return false;
    }

    if pattern.contains("**") {
        let parts: Vec<&str> = pattern.split("**").collect();
        if parts.len() == 2 {
            let prefix = parts[0].trim_end_matches('/');
            let suffix = parts[1].trim_start_matches('/');
            if prefix.is_empty() && suffix.is_empty() {
                return true;
            }
            if prefix.is_empty() {
                return path.ends_with(suffix) || glob_match_segment(suffix, path);
            }
            if suffix.is_empty() {
                return path.starts_with(prefix);
            }
            return path.starts_with(prefix) && path.ends_with(suffix);
        }
    }

    glob_match_segment(pattern, path)
}

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

#[derive(Clone, PartialEq, Eq, Hash)]
struct ObjectId([u8; 20]);

const ROOT_SENTINEL: ObjectId = ObjectId([0u8; 20]);

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

    fn dir(&self) -> String {
        format!("{:02x}", self.0[0])
    }

    fn file(&self) -> String {
        self.0[1..].iter().map(|b| format!("{:02x}", b)).collect()
    }
}

#[derive(Clone)]
struct IndexEntry {
    mode: String,
    hash: ObjectId,
    name: String,
}

// ============================================================
// Git directory helpers
// ============================================================

fn get_cwd() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn find_git_dir() -> Option<String> {
    let cwd = get_cwd();
    let mut search = cwd.clone();

    loop {
        let git_path = if search.is_empty() {
            ".git".to_string()
        } else {
            format!("{}/.git", search)
        };

        if std::path::Path::new(&git_path).exists() && std::path::Path::new(&git_path).is_dir() {
            return Some(git_path);
        }

        if search.is_empty() {
            break;
        }
        match search.rfind('/') {
            Some(pos) => search = search[..pos].to_string(),
            None => search = String::new(),
        }
    }

    if std::path::Path::new(".git").exists() && std::path::Path::new(".git").is_dir() {
        return Some(".git".to_string());
    }

    None
}

fn require_git_dir() -> Result<String, String> {
    find_git_dir().ok_or_else(|| "fatal: not a git repository (run 'git init')".to_string())
}

fn work_tree_root(git_dir: &str) -> String {
    work_tree_root_static(git_dir)
}

// ============================================================
// Object operations
// ============================================================

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

fn write_object(git_dir: &str, data: &[u8], obj_type: &str) -> Result<ObjectId, String> {
    let id = hash_object(data, obj_type);

    let obj_path = format!("{}/objects/{}/{}", git_dir, id.dir(), id.file());
    if std::path::Path::new(&obj_path).exists() {
        return Ok(id);
    }

    let dir_path = format!("{}/objects/{}", git_dir, id.dir());
    let _ = std::fs::create_dir_all(&dir_path);

    let header = format!("{} {}\0", obj_type, data.len());
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(header.as_bytes()).map_err(|e| format!("compress error: {}", e))?;
    encoder.write_all(data).map_err(|e| format!("compress error: {}", e))?;
    let compressed = encoder.finish().map_err(|e| format!("compress error: {}", e))?;

    if std::fs::write(&obj_path, &compressed).is_err() {
        return Err(format!("failed to write object {}", id.to_hex()));
    }

    Ok(id)
}

fn read_object(git_dir: &str, id: &ObjectId) -> Result<(String, Vec<u8>), String> {
    let obj_path = format!("{}/objects/{}/{}", git_dir, id.dir(), id.file());
    let compressed = std::fs::read(&obj_path)
        .map_err(|_| format!("object {} not found", id.to_hex()))?;

    let mut decoder = ZlibDecoder::new(&compressed[..]);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed).map_err(|e| format!("decompress error: {}", e))?;

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
    let content = match std::fs::read_to_string(&index_path).ok() {
        Some(c) => c,
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
    let _ = std::fs::write(&index_path, &content);
}

// ============================================================
// Tree operations
// ============================================================

fn build_tree(git_dir: &str, entries: &[IndexEntry], prefix: &str) -> Result<ObjectId, String> {
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

    let mut tree_data: Vec<u8> = Vec::new();

    for (dir_name, _sub_entries) in &subdirs {
        let sub_prefix = if prefix.is_empty() {
            format!("{}/", dir_name)
        } else {
            format!("{}{}/", prefix, dir_name)
        };
        let sub_tree_id = build_tree(git_dir, entries, &sub_prefix)?;
        tree_data.extend_from_slice(format!("40000 {}\0", dir_name).as_bytes());
        tree_data.extend_from_slice(&sub_tree_id.0);
    }

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

fn read_tree(git_dir: &str, tree_id: &ObjectId) -> Result<Vec<(String, String, ObjectId)>, String> {
    let (obj_type, data) = read_object(git_dir, tree_id)?;
    if obj_type != "tree" {
        return Err(format!("expected tree, got {}", obj_type));
    }

    let mut entries = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let space = data[pos..].iter().position(|&b| b == b' ')
            .ok_or("invalid tree entry")? + pos;
        let mode = std::str::from_utf8(&data[pos..space])
            .map_err(|_| "invalid mode")?.to_string();

        let null = data[space+1..].iter().position(|&b| b == 0)
            .ok_or("invalid tree entry")? + space + 1;
        let name = std::str::from_utf8(&data[space+1..null])
            .map_err(|_| "invalid name")?.to_string();

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

fn read_head(git_dir: &str) -> Option<String> {
    std::fs::read_to_string(&format!("{}/HEAD", git_dir)).ok().map(|s| s.trim().to_string())
}

fn current_branch(git_dir: &str) -> Option<String> {
    let head = read_head(git_dir)?;
    if head.starts_with("ref: refs/heads/") {
        Some(head["ref: refs/heads/".len()..].to_string())
    } else {
        None
    }
}

fn resolve_head(git_dir: &str) -> Option<ObjectId> {
    let head = read_head(git_dir)?;
    if head.starts_with("ref: ") {
        let ref_path = format!("{}/{}", git_dir, &head[5..]);
        let hash_str = std::fs::read_to_string(&ref_path).ok()?;
        ObjectId::from_hex(hash_str.trim())
    } else {
        ObjectId::from_hex(&head)
    }
}

fn update_head_ref(git_dir: &str, commit_id: &ObjectId) {
    let head = read_head(git_dir).unwrap_or_default();
    if head.starts_with("ref: ") {
        let ref_path = format!("{}/{}", git_dir, &head[5..]);
        let _ = std::fs::write(&ref_path, format!("{}\n", commit_id.to_hex()));
    } else {
        let _ = std::fs::write(&format!("{}/HEAD", git_dir), format!("{}\n", commit_id.to_hex()));
    }
}

fn read_ref(git_dir: &str, branch: &str) -> Option<ObjectId> {
    let ref_path = format!("{}/refs/heads/{}", git_dir, branch);
    let hash_str = std::fs::read_to_string(&ref_path).ok()?;
    ObjectId::from_hex(hash_str.trim())
}

// ============================================================
// Timestamp helper
// ============================================================

fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ============================================================
// Commit parsing helper
// ============================================================

struct CommitInfo {
    tree: ObjectId,
    parent: Option<ObjectId>,
    author: String,
    #[allow(dead_code)]
    committer: String,
    message: String,
}

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

fn find_merge_base(git_dir: &str, id1: &ObjectId, id2: &ObjectId) -> Option<ObjectId> {
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

fn collect_all_commits(git_dir: &str, from: &ObjectId) -> Result<Vec<ObjectId>, String> {
    let mut commits = Vec::new();
    let mut current = from.clone();

    loop {
        commits.push(current.clone());
        let (_, data) = read_object(git_dir, &current)?;
        let info = parse_commit(&data)?;
        match info.parent {
            Some(p) => current = p,
            None => break,
        }
    }

    commits.reverse();
    Ok(commits)
}

fn create_empty_tree(git_dir: &str) -> Result<ObjectId, String> {
    write_object(git_dir, &[], "tree")
}

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
            None => break,
        }
    }

    commits.reverse();
    Ok(commits)
}

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

    for (name, old_hash) in &old_files {
        if !new_files.iter().any(|(n, _)| n == name) {
            deleted.push((name.clone(), old_hash.clone()));
        }
    }

    Ok((added, modified, deleted))
}

fn cherry_pick(git_dir: &str, commit_id: &ObjectId, onto: Option<&ObjectId>) -> Result<ObjectId, String> {
    let (_, commit_data) = read_object(git_dir, commit_id)?;
    let commit_info = parse_commit(&commit_data)?;

    let _parent_tree_files = match &commit_info.parent {
        Some(parent_id) => {
            let (_, parent_data) = read_object(git_dir, parent_id)?;
            let parent_info = parse_commit(&parent_data)?;
            flatten_tree(git_dir, &parent_info.tree, "")?
        }
        None => Vec::new(),
    };

    let mut current_files = match onto {
        Some(onto_id) => {
            let (_, onto_data) = read_object(git_dir, onto_id)?;
            let onto_info = parse_commit(&onto_data)?;
            flatten_tree(git_dir, &onto_info.tree, "")?
        }
        None => Vec::new(),
    };

    let parent_tree_id = match &commit_info.parent {
        Some(parent_id) => {
            let (_, pd) = read_object(git_dir, parent_id)?;
            parse_commit(&pd)?.tree
        }
        None => create_empty_tree(git_dir)?,
    };
    let (added, modified, deleted) = diff_trees(git_dir, &parent_tree_id, &commit_info.tree)?;

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

    current_files.sort_by(|a, b| a.0.cmp(&b.0));
    let index_entries: Vec<IndexEntry> = current_files.into_iter().map(|(name, hash)| {
        IndexEntry { mode: "100644".to_string(), hash, name }
    }).collect();

    let new_tree = build_tree(git_dir, &index_entries, "")?;

    let timestamp = unix_timestamp();
    let new_author = format!("Terminal User <user@terminal.os> {} +0000", timestamp);
    let mut content = String::new();
    content.push_str(&format!("tree {}\n", new_tree.to_hex()));
    if let Some(onto_id) = onto {
        content.push_str(&format!("parent {}\n", onto_id.to_hex()));
    }
    content.push_str(&format!("author {}\n", commit_info.author));
    content.push_str(&format!("committer {}\n", new_author));
    content.push_str(&format!("\n{}\n", commit_info.message));

    write_object(git_dir, content.as_bytes(), "commit")
}

// ============================================================
// Commands
// ============================================================

fn cmd_init() {
    let cwd = get_cwd();
    let git_dir = if cwd.is_empty() {
        ".git".to_string()
    } else {
        format!("{}/.git", cwd)
    };

    if std::path::Path::new(&git_dir).exists() {
        println!("{}", "Reinitialized existing Git repository");
        return;
    }

    let _ = std::fs::create_dir_all(&git_dir);
    let _ = std::fs::create_dir_all(&format!("{}/objects", git_dir));
    let _ = std::fs::create_dir_all(&format!("{}/refs", git_dir));
    let _ = std::fs::create_dir_all(&format!("{}/refs/heads", git_dir));
    let _ = std::fs::write(&format!("{}/HEAD", git_dir), "ref: refs/heads/main\n");

    println!("{}", "Initialized empty Git repository");
}

fn cmd_add(args: &str) {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { println!("{}", e); return; }
    };
    let root = work_tree_root(&git_dir);

    if args.trim().is_empty() {
        println!("{}", "Nothing specified, nothing added.");
        return;
    }

    let mut index = read_index(&git_dir);
    let ignore = GitIgnore::load(&git_dir);

    for arg in args.split_whitespace() {
        if arg == "." {
            let cwd = get_cwd();
            add_directory(&git_dir, &root, &cwd, &mut index, &ignore);
        } else {
            // Resolve path: if relative, join with cwd
            let resolved = if std::path::Path::new(arg).is_absolute() {
                arg.to_string()
            } else {
                let cwd = get_cwd();
                if cwd.is_empty() {
                    arg.to_string()
                } else {
                    format!("{}/{}", cwd, arg)
                }
            };

            let rel_path = if root.is_empty() {
                resolved.clone()
            } else if resolved.starts_with(&root) && resolved.len() > root.len() {
                resolved[root.len()+1..].to_string()
            } else {
                resolved.clone()
            };

            let p = std::path::Path::new(&resolved);
            if p.is_dir() {
                add_directory(&git_dir, &root, &resolved, &mut index, &ignore);
            } else if p.exists() {
                add_file(&git_dir, &rel_path, &mut index);
            } else {
                print!("{}", "fatal: pathspec '");
                print!("{}", arg);
                println!("{}", "' did not match any files");
            }
        }
    }

    write_index(&git_dir, &index);
}

fn add_file(git_dir: &str, rel_path: &str, index: &mut Vec<IndexEntry>) {
    let root = work_tree_root(git_dir);
    let abs_path = if root.is_empty() {
        rel_path.to_string()
    } else {
        format!("{}/{}", root, rel_path)
    };

    let content = match std::fs::read(&abs_path).ok() {
        Some(c) => c,
        None => {
            print!("{}", "error: cannot read '");
            print!("{}", rel_path);
            println!("{}", "'");
            return;
        }
    };

    let blob_id = match write_object(git_dir, &content, "blob") {
        Ok(id) => id,
        Err(e) => {
            println!("{}", e);
            return;
        }
    };

    if let Some(entry) = index.iter_mut().find(|e| e.name == rel_path) {
        entry.hash = blob_id;
    } else {
        index.push(IndexEntry {
            mode: "100644".to_string(),
            hash: blob_id,
            name: rel_path.to_string(),
        });
    }
    index.sort_by(|a, b| a.name.cmp(&b.name));
}

/// Helper: list directory entries with name and is_dir flag.
fn list_dir(path: &str) -> Vec<(String, bool)> {
    let mut entries = Vec::new();
    if let Ok(rd) = std::fs::read_dir(path) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            entries.push((name, is_dir));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

fn add_directory(git_dir: &str, root: &str, dir_path: &str, index: &mut Vec<IndexEntry>, ignore: &GitIgnore) {
    let entries = list_dir(dir_path);
    for (name, is_dir) in &entries {
        let child = if dir_path.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", dir_path, name)
        };

        if name == ".git" { continue; }

        let rel = if root.is_empty() {
            child.clone()
        } else if child.starts_with(root) && child.len() > root.len() {
            child[root.len()+1..].to_string()
        } else {
            child.clone()
        };

        if ignore.is_ignored(&rel, *is_dir) { continue; }

        if *is_dir {
            add_directory(git_dir, root, &child, index, ignore);
        } else {
            add_file(git_dir, &rel, index);
        }
    }
}

fn cmd_status() {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { println!("{}", e); return; }
    };
    let root = work_tree_root(&git_dir);

    match current_branch(&git_dir) {
        Some(branch) => {
            print!("{}", "On branch ");
            println!("{}", branch);
        }
        None => println!("{}", "HEAD detached"),
    }

    let index = read_index(&git_dir);

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
            println!("{}", "\nNo commits yet\n");
            Vec::new()
        }
    };

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
            _ => {}
        }
    }
    for (name, _) in &committed {
        if !index.iter().any(|e| e.name == *name) {
            staged_deleted.push(name.clone());
        }
    }

    if !staged_new.is_empty() || !staged_modified.is_empty() || !staged_deleted.is_empty() {
        println!("{}", "Changes to be committed:");
        for f in &staged_new { print!("{}", "  new file:   "); println!("{}", f); }
        for f in &staged_modified { print!("{}", "  modified:   "); println!("{}", f); }
        for f in &staged_deleted { print!("{}", "  deleted:    "); println!("{}", f); }
        println!();
    }

    let ignore = GitIgnore::load(&git_dir);
    let mut untracked: Vec<String> = Vec::new();
    collect_working_files(&root, &root, &mut untracked, &ignore);
    let untracked: Vec<String> = untracked.into_iter()
        .filter(|f| !index.iter().any(|e| e.name == *f))
        .collect();

    if !untracked.is_empty() {
        println!("{}", "Untracked files:");
        for f in &untracked {
            print!("{}", "  ");
            println!("{}", f);
        }
        println!();
    }

    if staged_new.is_empty() && staged_modified.is_empty() && staged_deleted.is_empty() && untracked.is_empty() {
        println!("{}", "nothing to commit, working tree clean");
    }
}

fn collect_working_files(root: &str, dir: &str, result: &mut Vec<String>, ignore: &GitIgnore) {
    let entries = list_dir(dir);
    for (name, is_dir) in &entries {
        if name == ".git" { continue; }
        let child = if dir.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", dir, name)
        };

        let rel = if root.is_empty() {
            child.clone()
        } else if child.starts_with(root) && child.len() > root.len() {
            child[root.len()+1..].to_string()
        } else {
            child.clone()
        };

        if ignore.is_ignored(&rel, *is_dir) { continue; }

        if *is_dir {
            collect_working_files(root, &child, result, ignore);
        } else {
            result.push(rel);
        }
    }
}

fn cmd_commit(args: &str) {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { println!("{}", e); return; }
    };

    let message = if args.trim_start().starts_with("-m") {
        let rest = args.trim_start()[2..].trim();
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
        println!("{}", "error: must provide commit message with -m");
        return;
    } else {
        args.trim()
    };

    let index = read_index(&git_dir);
    if index.is_empty() {
        println!("{}", "nothing to commit");
        return;
    }

    let tree_id = match build_tree(&git_dir, &index, "") {
        Ok(id) => id,
        Err(e) => { println!("{}", e); return; }
    };

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
        Err(e) => { println!("{}", e); return; }
    };

    update_head_ref(&git_dir, &commit_id);

    let branch = current_branch(&git_dir).unwrap_or_else(|| "HEAD".to_string());
    print!("{}", "[");
    print!("{}", branch);
    print!("{}", " ");
    print!("{}", &commit_id.to_hex()[..7]);
    print!("{}", "] ");
    println!("{}", message);

    print!("{}", " ");
    let count = index.len();
    if count == 1 {
        println!("{}", "1 file changed");
    } else {
        print!("{}", format!("{}", count));
        println!("{}", " files changed");
    }
}

fn cmd_log() {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { println!("{}", e); return; }
    };

    let mut current = match resolve_head(&git_dir) {
        Some(id) => id,
        None => {
            println!("{}", "fatal: no commits yet");
            return;
        }
    };

    let head_branch = current_branch(&git_dir);
    let mut is_first = true;

    loop {
        let (obj_type, data) = match read_object(&git_dir, &current) {
            Ok(r) => r,
            Err(e) => { println!("{}", e); break; }
        };
        if obj_type != "commit" {
            println!("{}", "error: expected commit object");
            break;
        }

        let commit_str = String::from_utf8_lossy(&data).to_string();

        print!("{}", "commit ");
        print!("{}", current.to_hex());
        if is_first {
            if let Some(ref branch) = head_branch {
                print!("{}", " (HEAD -> ");
                print!("{}", branch);
                print!("{}", ")");
            }
        }
        println!();

        let mut parent_hash: Option<String> = None;
        let mut in_message = false;
        for line in commit_str.lines() {
            if in_message {
                if !line.is_empty() {
                    print!("{}", "    ");
                    println!("{}", line);
                }
            } else if line.is_empty() {
                in_message = true;
            } else if line.starts_with("author ") {
                print!("{}", "Author: ");
                let author_part = &line[7..];
                if let Some(angle) = author_part.find('>') {
                    println!("{}", &author_part[..angle+1]);
                } else {
                    println!("{}", author_part);
                }
            } else if line.starts_with("parent ") {
                parent_hash = Some(line[7..].to_string());
            }
        }
        println!();

        match parent_hash {
            Some(hash) => {
                match ObjectId::from_hex(&hash) {
                    Some(id) => current = id,
                    None => break,
                }
            }
            None => break,
        }
        is_first = false;
    }
}

fn cmd_branch(args: &str) {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { println!("{}", e); return; }
    };

    let name = args.trim();

    if name.is_empty() {
        let current = current_branch(&git_dir);
        let refs_dir = format!("{}/refs/heads", git_dir);
        let entries = list_dir(&refs_dir);

        if entries.is_empty() {
            if let Some(branch) = &current {
                print!("{}", "* ");
                println!("{}", branch);
            }
            return;
        }

        for (entry_name, is_dir) in &entries {
            if !is_dir {
                if current.as_deref() == Some(entry_name.as_str()) {
                    print!("{}", "* ");
                } else {
                    print!("{}", "  ");
                }
                println!("{}", entry_name);
            }
        }
    } else {
        let commit_id = match resolve_head(&git_dir) {
            Some(id) => id,
            None => {
                println!("{}", "fatal: not a valid object name: no commits yet");
                return;
            }
        };

        let ref_path = format!("{}/refs/heads/{}", git_dir, name);
        if std::path::Path::new(&ref_path).exists() {
            print!("{}", "fatal: branch '");
            print!("{}", name);
            println!("{}", "' already exists");
            return;
        }

        let _ = std::fs::write(&ref_path, format!("{}\n", commit_id.to_hex()));
    }
}

fn cmd_checkout(args: &str) {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { println!("{}", e); return; }
    };

    let target = args.trim();
    if target.is_empty() {
        println!("{}", "error: specify a branch to checkout");
        return;
    }

    let ref_path = format!("{}/refs/heads/{}", git_dir, target);
    if !std::path::Path::new(&ref_path).exists() {
        print!("{}", "error: branch '");
        print!("{}", target);
        println!("{}", "' not found");
        return;
    }

    let target_commit = match read_ref(&git_dir, target) {
        Some(id) => id,
        None => {
            println!("{}", "error: invalid branch ref");
            return;
        }
    };

    let _ = std::fs::write(
        &format!("{}/HEAD", git_dir),
        format!("ref: refs/heads/{}\n", target),
    );

    let (_, commit_data) = match read_object(&git_dir, &target_commit) {
        Ok(r) => r,
        Err(e) => { println!("{}", e); return; }
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

    print!("{}", "Switched to branch '");
    print!("{}", target);
    println!("{}", "'");
}

fn cmd_diff() {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { println!("{}", e); return; }
    };
    let root = work_tree_root(&git_dir);

    let index = read_index(&git_dir);
    let mut has_diff = false;

    for entry in &index {
        let abs_path = if root.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", root, entry.name)
        };

        let current_content = match std::fs::read(&abs_path).ok() {
            Some(c) => c,
            None => {
                print!("{}", "diff --git a/");
                print!("{}", entry.name);
                print!("{}", " b/");
                println!("{}", entry.name);
                println!("{}", "deleted file");
                println!();
                has_diff = true;
                continue;
            }
        };

        let current_hash = hash_object(&current_content, "blob");
        if current_hash == entry.hash {
            continue;
        }

        has_diff = true;

        print!("{}", "diff --git a/");
        print!("{}", entry.name);
        print!("{}", " b/");
        println!("{}", entry.name);
        print!("{}", "index ");
        print!("{}", &entry.hash.to_hex()[..7]);
        print!("{}", "..");
        println!("{}", &current_hash.to_hex()[..7]);

        let old_content = match read_object(&git_dir, &entry.hash) {
            Ok((_, data)) => String::from_utf8_lossy(&data).to_string(),
            Err(_) => String::new(),
        };
        let new_content = String::from_utf8_lossy(&current_content).to_string();

        let old_lines: Vec<&str> = old_content.lines().collect();
        let new_lines: Vec<&str> = new_content.lines().collect();

        print!("{}", "--- a/");
        println!("{}", entry.name);
        print!("{}", "+++ b/");
        println!("{}", entry.name);

        for line in &old_lines {
            if !new_lines.contains(line) {
                print!("{}", "-");
                println!("{}", line);
            }
        }
        for line in &new_lines {
            if !old_lines.contains(line) {
                print!("{}", "+");
                println!("{}", line);
            }
        }
        println!();
    }

    if !has_diff {
        // No output (matches real git behavior)
    }
}

// ============================================================
// Ref spec parsing (HEAD~N)
// ============================================================

fn resolve_ref_spec(git_dir: &str, spec: &str) -> Option<ObjectId> {
    if spec == "HEAD" {
        return resolve_head(git_dir);
    }

    if spec.starts_with("HEAD~") {
        let n: usize = match spec[5..].parse() {
            Ok(n) => n,
            Err(_) => {
                println!("{}", "error: invalid number after HEAD~");
                return None;
            }
        };
        let mut current = match resolve_head(git_dir) {
            Some(id) => id,
            None => {
                println!("{}", "error: cannot resolve HEAD (is .git/HEAD readable?)");
                return None;
            }
        };
        for i in 0..n {
            let (_, data) = match read_object(git_dir, &current) {
                Ok(r) => r,
                Err(e) => {
                    print!("{}", format!("error: cannot read commit {} at step {}: ", &current.to_hex()[..7], i + 1));
                    println!("{}", e);
                    return None;
                }
            };
            let info = match parse_commit(&data) {
                Ok(i) => i,
                Err(e) => {
                    print!("{}", format!("error: cannot parse commit {} at step {}: ", &current.to_hex()[..7], i + 1));
                    println!("{}", e);
                    return None;
                }
            };
            match info.parent {
                Some(p) => current = p,
                None => {
                    return Some(ROOT_SENTINEL);
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
    std::path::Path::new(&format!("{}/git-rebase-todo", rebase_state_dir(git_dir))).exists()
}

fn write_rebase_state(
    git_dir: &str,
    branch: &str,
    orig_head: &ObjectId,
    onto_str: &str,
    todo_content: &str,
) {
    let state_dir = rebase_state_dir(git_dir);
    let _ = std::fs::create_dir_all(&state_dir);
    let _ = std::fs::write(&format!("{}/head-name", state_dir), branch);
    let _ = std::fs::write(&format!("{}/orig-head", state_dir), orig_head.to_hex());
    let _ = std::fs::write(&format!("{}/onto", state_dir), onto_str);
    let _ = std::fs::write(&format!("{}/git-rebase-todo", state_dir), todo_content);
}

fn cleanup_rebase_state(git_dir: &str) {
    let state_dir = rebase_state_dir(git_dir);
    for name in &["head-name", "orig-head", "onto", "git-rebase-todo", "current-tip", "stopped-at"] {
        let _ = std::fs::remove_file(&format!("{}/{}", state_dir, name));
    }
    let _ = std::fs::remove_dir(&state_dir);
    let _ = std::fs::remove_file(&format!("{}/REBASE_HEAD", git_dir));
}

fn generate_todo(git_dir: &str, commits: &[ObjectId]) -> String {
    let mut content = String::new();
    for commit_id in commits {
        if let Ok((_, data)) = read_object(git_dir, commit_id) {
            if let Ok(info) = parse_commit(&data) {
                let first_line = info.message.lines().next().unwrap_or(&info.message);
                content.push_str(&format!(
                    "pick {} {}\n",
                    &commit_id.to_hex()[..7],
                    first_line
                ));
            }
        }
    }
    content.push_str("\n# Rebase interactive -- edit this file then save and exit.\n");
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

struct TodoEntry {
    action: String,
    hash_prefix: String,
    message: String,
}

fn parse_todo(git_dir: &str) -> Vec<TodoEntry> {
    let state_dir = rebase_state_dir(git_dir);
    let content = match std::fs::read_to_string(&format!("{}/git-rebase-todo", state_dir)).ok() {
        Some(c) => c,
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

fn find_commit_by_prefix(git_dir: &str, prefix: &str, candidates: &[ObjectId]) -> Option<ObjectId> {
    for id in candidates {
        if id.to_hex().starts_with(prefix) {
            return Some(id.clone());
        }
    }
    let state_dir = rebase_state_dir(git_dir);
    if let Some(orig_hex) = std::fs::read_to_string(&format!("{}/orig-head", state_dir)).ok() {
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

fn execute_rebase_todo(git_dir: &str) {
    let state_dir = rebase_state_dir(git_dir);

    let onto_hex = match std::fs::read_to_string(&format!("{}/onto", state_dir)).ok() {
        Some(s) => s.trim().to_string(),
        None => { println!("{}", "error: no rebase in progress"); return; }
    };
    let root_mode = onto_hex == "ROOT";
    let onto: Option<ObjectId> = if root_mode {
        None
    } else {
        match ObjectId::from_hex(&onto_hex) {
            Some(id) => Some(id),
            None => { println!("{}", "error: invalid onto ref"); return; }
        }
    };

    let orig_hex = match std::fs::read_to_string(&format!("{}/orig-head", state_dir)).ok() {
        Some(s) => s.trim().to_string(),
        None => { println!("{}", "error: missing orig-head"); return; }
    };
    let orig_head = match ObjectId::from_hex(&orig_hex) {
        Some(id) => id,
        None => { println!("{}", "error: invalid orig-head"); return; }
    };

    let all_commits = if root_mode {
        collect_all_commits(git_dir, &orig_head).unwrap_or_default()
    } else {
        collect_commits(git_dir, &orig_head, onto.as_ref().unwrap()).unwrap_or_default()
    };

    let entries = parse_todo(git_dir);
    if entries.is_empty() {
        println!("{}", "Nothing to do (empty todo).");
        cleanup_rebase_state(git_dir);
        return;
    }

    let mut current_tip: Option<ObjectId> = match std::fs::read_to_string(&format!("{}/current-tip", state_dir)).ok() {
        Some(s) => ObjectId::from_hex(s.trim()).or(onto.clone()),
        None => onto.clone(),
    };

    let mut last_message = String::new();
    let mut pending_squash_messages: Vec<String> = Vec::new();

    print!("{}", "Executing rebase (");
    print!("{}", format!("{}", entries.len()));
    println!("{}", " steps)...");

    for (i, entry) in entries.iter().enumerate() {
        let action = match entry.action.as_str() {
            "p" | "pick" => "pick",
            "s" | "squash" => "squash",
            "f" | "fixup" => "fixup",
            "d" | "drop" => "drop",
            "r" | "reword" => "reword",
            "e" | "edit" => "edit",
            other => {
                print!("{}", "warning: unknown action '");
                print!("{}", other);
                println!("{}", "', treating as pick");
                "pick"
            }
        };

        if action == "drop" {
            print!("{}", "  ");
            print!("{}", format!("{}/{}", i + 1, entries.len()));
            print!("{}", " drop ");
            println!("{}", entry.hash_prefix);
            continue;
        }

        let commit_id = match find_commit_by_prefix(git_dir, &entry.hash_prefix, &all_commits) {
            Some(id) => id,
            None => {
                print!("{}", "error: could not find commit ");
                println!("{}", entry.hash_prefix);
                println!("{}", "Aborting rebase.");
                abort_rebase(git_dir);
                return;
            }
        };

        match cherry_pick(git_dir, &commit_id, current_tip.as_ref()) {
            Ok(new_id) => {
                let msg = if let Ok((_, data)) = read_object(git_dir, &commit_id) {
                    parse_commit(&data).map(|i| i.message).unwrap_or_default()
                } else {
                    String::new()
                };

                match action {
                    "pick" => {
                        if !pending_squash_messages.is_empty() {
                            if let Some(ref tip) = current_tip {
                                current_tip = Some(amend_commit_message(git_dir, tip, &pending_squash_messages.join("\n\n")));
                            }
                            pending_squash_messages.clear();
                        }
                        current_tip = Some(new_id);
                        last_message = msg;
                        print!("{}", "  ");
                        print!("{}", format!("{}/{}", i + 1, entries.len()));
                        print!("{}", " pick ");
                        println!("{}", &current_tip.as_ref().unwrap().to_hex()[..7]);
                    }
                    "squash" | "fixup" => {
                        let prev_info = current_tip.as_ref()
                            .and_then(|tip| read_object(git_dir, tip).ok())
                            .and_then(|(_, d)| parse_commit(&d).ok());
                        let parent_of_prev = prev_info.as_ref()
                            .and_then(|i| i.parent.clone());

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

                        if let Ok((_, cd)) = read_object(git_dir, &combined_id) {
                            if let Ok(ci) = parse_commit(&cd) {
                                let timestamp = unix_timestamp();
                                let committer_str = format!("Terminal User <user@terminal.os> {} +0000", timestamp);
                                let prev_author = prev_info.as_ref()
                                    .map(|i| i.author.clone())
                                    .unwrap_or(committer_str.clone());
                                let mut content = String::new();
                                content.push_str(&format!("tree {}\n", ci.tree.to_hex()));
                                if let Some(ref pop) = parent_of_prev {
                                    content.push_str(&format!("parent {}\n", pop.to_hex()));
                                }
                                content.push_str(&format!("author {}\n", prev_author));
                                content.push_str(&format!("committer {}\n", committer_str));
                                content.push_str(&format!("\n{}\n", combined_msg));
                                if let Ok(final_id) = write_object(git_dir, content.as_bytes(), "commit") {
                                    current_tip = Some(final_id);
                                }
                            }
                        }

                        if action == "fixup" {
                            last_message = combined_msg;
                        }

                        print!("{}", "  ");
                        print!("{}", format!("{}/{}", i + 1, entries.len()));
                        print!("{}", if action == "squash" { " squash " } else { " fixup " });
                        println!("{}", &current_tip.as_ref().unwrap().to_hex()[..7]);
                    }
                    "reword" => {
                        if !pending_squash_messages.is_empty() {
                            if let Some(ref tip) = current_tip {
                                current_tip = Some(amend_commit_message(git_dir, tip, &pending_squash_messages.join("\n\n")));
                            }
                            pending_squash_messages.clear();
                        }
                        current_tip = Some(new_id);
                        print!("{}", "  ");
                        print!("{}", format!("{}/{}", i + 1, entries.len()));
                        println!("{}", " reword -- enter new message:");
                        let new_msg = shell_read_line("  message: ");
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
                            let _ = std::fs::write(
                                &format!("{}/current-tip", state_dir),
                                tip.to_hex(),
                            );
                            update_head_ref(git_dir, tip);
                        }
                        let remaining = &entries[i+1..];
                        let mut new_todo = String::new();
                        for r in remaining {
                            new_todo.push_str(&format!("{} {} {}\n", r.action, r.hash_prefix, r.message));
                        }
                        let _ = std::fs::write(
                            &format!("{}/git-rebase-todo", state_dir),
                            &new_todo,
                        );
                        print!("{}", "  ");
                        print!("{}", format!("{}/{}", i + 1, entries.len()));
                        println!("{}", " edit -- stopped for editing");
                        println!("{}", "Amend the commit, then run 'git rebase --continue'");
                        return;
                    }
                    _ => {
                        current_tip = Some(new_id);
                        last_message = msg;
                    }
                }
            }
            Err(e) => {
                print!("{}", "error: ");
                println!("{}", e);
                println!("{}", "Aborting rebase.");
                abort_rebase(git_dir);
                return;
            }
        }
    }

    if !pending_squash_messages.is_empty() {
        if let Some(ref tip) = current_tip {
            current_tip = Some(amend_commit_message(git_dir, tip, &pending_squash_messages.join("\n\n")));
        }
    }

    if let Some(ref tip) = current_tip {
        update_head_ref(git_dir, tip);
        update_index_to_commit(git_dir, tip);
    }
    cleanup_rebase_state(git_dir);
    println!("{}", "Successfully rebased.");
}

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

fn abort_rebase(git_dir: &str) {
    let state_dir = rebase_state_dir(git_dir);

    if let Some(orig_hex) = std::fs::read_to_string(&format!("{}/orig-head", state_dir)).ok() {
        if let Some(orig_id) = ObjectId::from_hex(orig_hex.trim()) {
            update_head_ref(git_dir, &orig_id);
            update_index_to_commit(git_dir, &orig_id);
        }
    }

    cleanup_rebase_state(git_dir);
    println!("{}", "Rebase aborted and branch restored.");
}

fn cmd_rebase(args: &str) {
    let git_dir = match require_git_dir() {
        Ok(d) => d,
        Err(e) => { println!("{}", e); return; }
    };

    let args = args.trim();

    if args == "--continue" {
        if !is_rebase_in_progress(&git_dir) {
            println!("{}", "error: no rebase in progress");
            return;
        }
        execute_rebase_todo(&git_dir);
        return;
    }

    if args == "--abort" {
        if !is_rebase_in_progress(&git_dir) {
            println!("{}", "error: no rebase in progress");
            return;
        }
        abort_rebase(&git_dir);
        return;
    }

    if is_rebase_in_progress(&git_dir) {
        println!("{}", "error: rebase already in progress");
        println!("{}", "Use 'git rebase --continue' or 'git rebase --abort'");
        return;
    }

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
        println!("{}", "usage: git rebase [-i] [--root | <branch|HEAD~N>]");
        return;
    }

    let current = match current_branch(&git_dir) {
        Some(b) => b,
        None => {
            println!("{}", "fatal: cannot rebase with detached HEAD");
            return;
        }
    };

    let head_id = match resolve_head(&git_dir) {
        Some(id) => id,
        None => {
            println!("{}", "fatal: no commits on current branch");
            return;
        }
    };

    let onto_id: Option<ObjectId>;
    let commits;

    if root_mode {
        onto_id = None;
        commits = match collect_all_commits(&git_dir, &head_id) {
            Ok(c) => c,
            Err(e) => { println!("{}", e); return; }
        };
    } else {
        let target_id = match resolve_ref_spec(&git_dir, target_spec) {
            Some(id) => id,
            None => {
                print!("{}", "fatal: invalid ref '");
                print!("{}", target_spec);
                println!("{}", "'");
                return;
            }
        };

        if !interactive && !target_spec.starts_with("HEAD~") && current == target_spec {
            println!("{}", "fatal: cannot rebase a branch onto itself");
            return;
        }

        if target_spec.starts_with("HEAD~") {
            if target_id == ROOT_SENTINEL {
                onto_id = None;
                commits = match collect_all_commits(&git_dir, &head_id) {
                    Ok(c) => c,
                    Err(e) => { println!("{}", e); return; }
                };
            } else {
                onto_id = Some(target_id.clone());
                commits = match collect_commits(&git_dir, &head_id, &target_id) {
                    Ok(c) => c,
                    Err(e) => { println!("{}", e); return; }
                };
            }
        } else {
            let merge_base = match find_merge_base(&git_dir, &head_id, &target_id) {
                Some(mb) => mb,
                None => {
                    println!("{}", "fatal: no common ancestor found");
                    return;
                }
            };

            if head_id == target_id {
                println!("{}", "Current branch is up to date.");
                return;
            }
            if merge_base == head_id {
                update_head_ref(&git_dir, &target_id);
                update_index_to_commit(&git_dir, &target_id);
                print!("{}", "Fast-forwarded to ");
                println!("{}", target_spec);
                return;
            }
            if merge_base == target_id {
                println!("{}", "Current branch is up to date.");
                return;
            }

            onto_id = Some(target_id.clone());
            commits = match collect_commits(&git_dir, &head_id, &merge_base) {
                Ok(c) => c,
                Err(e) => { println!("{}", e); return; }
            };
        }
    }

    if commits.is_empty() {
        println!("{}", "Nothing to rebase.");
        return;
    }

    let onto_hex = match &onto_id {
        Some(id) => id.to_hex(),
        None => "ROOT".to_string(),
    };

    if interactive {
        let todo = generate_todo(&git_dir, &commits);
        write_rebase_state(&git_dir, &current, &head_id, &onto_hex, &todo);

        println!("{}", "Opening rebase todo in editor...");
        println!("{}", "Edit the plan, save (Ctrl+S), and exit (Ctrl+E).");
        println!("{}", "Then run 'git rebase --continue' to execute.");

        // In standalone mode, print the todo path so the user can edit it
        let todo_path = format!("{}/{}/git-rebase-todo", git_dir, REBASE_DIR);
        print!("{}", "Todo file: ");
        println!("{}", todo_path);
    } else {
        let orig_ref_path = format!("{}/REBASE_HEAD", git_dir);
        let _ = std::fs::write(&orig_ref_path, head_id.to_hex());

        let label = if root_mode { "--root" } else { target_spec };
        print!("{}", "Rebasing ");
        print!("{}", format!("{}", commits.len()));
        print!("{}", " commit(s) onto ");
        print!("{}", label);
        println!("{}", "...");

        let mut current_tip = onto_id.clone();
        for (i, commit_id) in commits.iter().enumerate() {
            match cherry_pick(&git_dir, commit_id, current_tip.as_ref()) {
                Ok(new_id) => {
                    print!("{}", "  ");
                    print!("{}", format!("{}/{}", i + 1, commits.len()));
                    print!("{}", " ");
                    println!("{}", &new_id.to_hex()[..7]);
                    current_tip = Some(new_id);
                }
                Err(e) => {
                    print!("{}", "error: ");
                    println!("{}", e);
                    println!("{}", "Aborting rebase.");
                    update_head_ref(&git_dir, &head_id);
                    let _ = std::fs::remove_file(&orig_ref_path);
                    return;
                }
            }
        }

        if let Some(tip) = &current_tip {
            update_head_ref(&git_dir, tip);
            update_index_to_commit(&git_dir, tip);
        }
        let _ = std::fs::remove_file(&orig_ref_path);
        print!("{}", "Successfully rebased onto ");
        println!("{}", label);
    }
}

fn cmd_debug() {
    println!("{}", "=== Git Debug Info ===");

    match find_git_dir() {
        Some(git_dir) => {
            print!("{}", ".git directory: ");
            println!("{}", git_dir);

            match read_head(&git_dir) {
                Some(head) => {
                    print!("{}", "HEAD: ");
                    println!("{}", head);
                }
                None => println!("{}", "HEAD: UNREADABLE"),
            }

            match current_branch(&git_dir) {
                Some(branch) => {
                    print!("{}", "Branch: ");
                    println!("{}", branch);
                }
                None => println!("{}", "Branch: (detached or unreadable)"),
            }

            match resolve_head(&git_dir) {
                Some(id) => {
                    print!("{}", "HEAD commit: ");
                    println!("{}", id.to_hex());

                    match read_object(&git_dir, &id) {
                        Ok((obj_type, data)) => {
                            print!("{}", "  Object type: ");
                            println!("{}", obj_type);
                            print!("{}", "  Object size: ");
                            println!("{}", format!("{} bytes", data.len()));

                            match parse_commit(&data) {
                                Ok(info) => {
                                    print!("{}", "  Tree: ");
                                    println!("{}", info.tree.to_hex());
                                    match &info.parent {
                                        Some(p) => {
                                            print!("{}", "  Parent: ");
                                            println!("{}", p.to_hex());
                                        }
                                        None => println!("{}", "  Parent: (none -- root commit)"),
                                    }
                                    print!("{}", "  Message: ");
                                    println!("{}", info.message);
                                }
                                Err(e) => {
                                    print!("{}", "  Parse error: ");
                                    println!("{}", e);
                                    println!("{}", "  Raw content (first 200 bytes):");
                                    let preview = String::from_utf8_lossy(&data[..data.len().min(200)]);
                                    print!("{}", "  ");
                                    println!("{}", preview);
                                }
                            }
                        }
                        Err(e) => {
                            print!("{}", "  Read error: ");
                            println!("{}", e);
                            let obj_path = format!("{}/objects/{}/{}", git_dir, id.dir(), id.file());
                            if std::path::Path::new(&obj_path).exists() {
                                print!("{}", "  Object file exists at: ");
                                println!("{}", obj_path);
                                match std::fs::read(&obj_path).ok() {
                                    Some(bytes) => {
                                        print!("{}", "  Raw file size: ");
                                        println!("{}", format!("{} bytes", bytes.len()));
                                    }
                                    None => println!("{}", "  Could not read raw file"),
                                }
                            } else {
                                print!("{}", "  Object file MISSING: ");
                                println!("{}", obj_path);
                            }
                        }
                    }

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
                    print!("{}", "Total commits: ");
                    println!("{}", format!("{}", count));
                }
                None => println!("{}", "HEAD commit: UNRESOLVABLE"),
            }

            let index = read_index(&git_dir);
            print!("{}", "Index entries: ");
            println!("{}", format!("{}", index.len()));
        }
        None => println!("{}", "Not a git repository"),
    }

    println!("{}", "=== End Debug ===");
}

// ============================================================
// Main entry point
// ============================================================

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        println!("{}", "usage: git <command> [<args>]");
        println!();
        println!("{}", "Available commands:");
        println!("{}", "  init       Create an empty Git repository");
        println!("{}", "  add        Add file contents to the index");
        println!("{}", "  status     Show the working tree status");
        println!("{}", "  commit     Record changes to the repository");
        println!("{}", "  log        Show commit logs");
        println!("{}", "  branch     List or create branches");
        println!("{}", "  checkout   Switch branches");
        println!("{}", "  diff       Show changes between index and working tree");
        println!("{}", "  rebase     Reapply commits on top of another base tip");
        println!("{}", "  debug      Show internal debug info");
        std::process::exit(1);
    }

    let command = &args[1];
    let rest: String = if args.len() > 2 {
        args[2..].join(" ")
    } else {
        String::new()
    };

    match command.as_str() {
        "init" => cmd_init(),
        "add" => cmd_add(&rest),
        "status" => cmd_status(),
        "commit" => cmd_commit(&rest),
        "log" => cmd_log(),
        "branch" => cmd_branch(&rest),
        "checkout" => cmd_checkout(&rest),
        "diff" => cmd_diff(),
        "rebase" => cmd_rebase(&rest),
        "debug" => cmd_debug(),
        other => {
            print!("{}", "git: '");
            print!("{}", other);
            println!("{}", "' is not a git command.");
            std::process::exit(1);
        }
    }
}
