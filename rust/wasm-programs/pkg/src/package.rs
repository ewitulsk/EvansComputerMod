//! Packages, repository indexes and signatures (docs/gui/APPS.md).

use crate::manifest::{self, Manifest};
use crate::tar::{self, Entry};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// `index.json` format tag.
pub const INDEX_FORMAT: &str = "ecm-repo-1";

/// A repository index (`index.json`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Index {
    pub format: String,
    #[serde(default)]
    pub name: String,
    /// Hex ed25519 public key that signed the packages, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_key: Option<String>,
    pub packages: Vec<IndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IndexEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    /// File name next to index.json.
    pub file: String,
    pub size: u64,
    pub sha256: String,
    /// Hex ed25519 signature over [`signed_message`], if signed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// The manifest's `publisher` (optional; the App Store shows it).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub publisher: String,
    /// The icon's path inside the package (optional). A repository serves
    /// package members at `<file>/<member>` (see [`asset_path`]).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    /// Screenshots: PNG members under `screenshots/` (optional, at most
    /// [`MAX_SCREENSHOTS`]), served like the icon.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screenshots: Vec<String>,
    /// `app` (the default, left out), `program`, `library` or `source`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// The manifest's `depends`, as written there (`id`, `id >= 1.2.0`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends: Vec<String>,
    /// Programs: the shell commands the package installs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
    /// Libraries: the modules the package installs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provides: Vec<String>,
    /// The format level a client needs ([`manifest::FORMAT_LEVEL`]); 0 or 1 = any client.
    #[serde(default, skip_serializing_if = "is_legacy_level")]
    pub level: u32,
    /// Hex ed25519 key of the author, when the author signed the package
    /// (a repository with many publishers signs with its own key as well).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_key: Option<String>,
    /// The author's signature over [`signed_message`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_signature: Option<String>,
}

fn is_legacy_level(l: &u32) -> bool {
    *l <= 1
}

impl IndexEntry {
    pub fn kind(&self) -> manifest::Kind {
        manifest::Kind::parse(&self.kind).unwrap_or(manifest::Kind::App)
    }

    pub fn deps(&self) -> Vec<manifest::Dep> {
        self.depends.iter().filter_map(|d| manifest::Dep::parse(d)).collect()
    }

    /// Can this client install it?
    pub fn installable(&self) -> bool {
        self.level <= manifest::FORMAT_LEVEL
    }

    /// The author's key when the author's signature over this entry checks out.
    pub fn author(&self) -> Option<&str> {
        let (k, sig) = (self.author_key.as_deref()?, self.author_signature.as_deref()?);
        crate::sign::verify(k, &signed_message(&self.id, &self.version, &self.sha256), sig).then_some(k)
    }
}

/// Most screenshots an index entry lists.
pub const MAX_SCREENSHOTS: usize = 4;

/// Where a repository serves a member of a package: `<file>/<member>`.
pub fn asset_path(entry: &IndexEntry, member: &str) -> String {
    format!("{}/{}", entry.file, member)
}

/// The members a repository may serve from a package: its icon and screenshots.
pub fn servable(entry: &IndexEntry, member: &str) -> bool {
    (!entry.icon.is_empty() && entry.icon == member) || entry.screenshots.iter().any(|s| s == member)
}

/// The screenshots of a package: PNG files under `screenshots/`, sorted, at most [`MAX_SCREENSHOTS`].
pub fn screenshots(p: &Package) -> Vec<String> {
    let mut out: Vec<String> = p
        .entries
        .iter()
        .filter(|e| !e.is_dir && e.path.starts_with("screenshots/") && e.path.ends_with(".png"))
        .map(|e| e.path.clone())
        .collect();
    out.sort();
    out.truncate(MAX_SCREENSHOTS);
    out
}

/// A member's bytes (icon, screenshot).
pub fn member<'a>(p: &'a Package, path: &str) -> Option<&'a [u8]> {
    p.entries.iter().find(|e| !e.is_dir && e.path == path).map(|e| e.data.as_slice())
}

/// A package read from its bytes: manifest checked, files listed.
#[derive(Debug, Clone)]
pub struct Package {
    pub manifest: Manifest,
    pub entries: Vec<Entry>,
    pub sha256: String,
}

pub fn sha256_hex(b: &[u8]) -> String {
    hex(&Sha256::digest(b))
}

pub fn hex(b: &[u8]) -> String {
    let d = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push(d[(x >> 4) as usize] as char);
        s.push(d[(x & 15) as usize] as char);
    }
    s
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// Check an archive and its manifest: app.toml first, valid, entry file present,
/// icon present if named.
pub fn open(bytes: &[u8]) -> Result<Package, String> {
    let entries = tar::read(bytes)?;
    let first = entries.first().ok_or("empty package")?;
    if first.path != "app.toml" || first.is_dir {
        return Err("the first file in a package must be app.toml".into());
    }
    let text = std::str::from_utf8(&first.data).map_err(|_| "app.toml is not UTF-8")?;
    let m = manifest::parse(text).map_err(|e| e.to_string())?;
    let has = |p: &str| entries.iter().any(|e| !e.is_dir && e.path == p);
    match m.kind {
        manifest::Kind::App => {
            if !has(&m.entry) {
                return Err(format!("entry {} is not in the package", m.entry));
            }
        }
        manifest::Kind::Program => {
            if let Some(b) = m.bin.iter().find(|b| !has(b)) {
                return Err(format!("bin {} is not in the package", b));
            }
        }
        manifest::Kind::Library => {
            for module in &m.provides {
                if library_files(&entries, module).next().is_none() {
                    return Err(format!("module {} is not in the package (python/{0}.py, python/{0}/, lua/{0}.lua or lua/{0}/)", module));
                }
            }
            for e in entries.iter().filter(|e| !e.is_dir) {
                if let Some((_, module)) = library_member(&e.path) {
                    if !m.provides.iter().any(|p| p == module) {
                        return Err(format!("{} belongs to module {}, which provides does not list", e.path, module));
                    }
                }
            }
        }
        manifest::Kind::Source => {}
    }
    if !m.icon.is_empty() && !entries.iter().any(|e| !e.is_dir && e.path == m.icon) {
        return Err(format!("icon {} is not in the package", m.icon));
    }
    Ok(Package { manifest: m, entries, sha256: sha256_hex(bytes) })
}

/// For a library file `python/<module>...` or `lua/<module>...`: (language, top-level module).
pub fn library_member(path: &str) -> Option<(&str, &str)> {
    let (lang, rest) = path.split_once('/')?;
    if lang != "python" && lang != "lua" {
        return None;
    }
    let top = rest.split('/').next()?;
    let module = if rest.contains('/') { top } else { top.strip_suffix(if lang == "python" { ".py" } else { ".lua" })? };
    Some((lang, module))
}

/// The files of one module of a library package.
pub fn library_files<'a>(entries: &'a [Entry], module: &'a str) -> impl Iterator<Item = &'a Entry> {
    entries.iter().filter(move |e| !e.is_dir && library_member(&e.path).map(|(_, m)| m) == Some(module))
}

/// Build a package from files (relative path, bytes). `app.toml` must be among
/// them; it is written first and the rest in sorted order. Checks the result with
/// [`open`].
pub fn build(mut files: Vec<(String, Vec<u8>)>) -> Result<(Package, Vec<u8>), String> {
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let pos = files.iter().position(|(p, _)| p == "app.toml").ok_or("no app.toml in the folder")?;
    let app = files.remove(pos);
    let mut entries = vec![Entry { path: app.0, data: app.1, is_dir: false }];
    for (p, d) in files {
        entries.push(Entry { path: p, data: d, is_dir: false });
    }
    let bytes = tar::write(&entries)?;
    let pkg = open(&bytes)?;
    Ok((pkg, bytes))
}

/// The package file name a repository uses: `<id>-<version>.ecmpkg`.
pub fn file_name(m: &Manifest) -> String {
    format!("{}-{}.ecmpkg", m.id, m.version)
}

/// What a signature covers: binds id, version and the archive hash.
pub fn signed_message(id: &str, version: &str, sha256: &str) -> Vec<u8> {
    format!("ecmpkg-sig-v1\n{}\n{}\n{}\n", id, version, sha256).into_bytes()
}

pub fn index_entry(p: &Package, file: &str, size: u64, signature: Option<String>) -> IndexEntry {
    IndexEntry {
        id: p.manifest.id.clone(),
        name: p.manifest.name.clone(),
        version: p.manifest.version.clone(),
        summary: p.manifest.summary.clone(),
        permissions: p.manifest.permissions.clone(),
        file: file.to_string(),
        size,
        sha256: p.sha256.clone(),
        signature,
        publisher: p.manifest.publisher.clone(),
        icon: p.manifest.icon.clone(),
        screenshots: screenshots(p),
        kind: if p.manifest.kind == manifest::Kind::App { String::new() } else { p.manifest.kind.name().to_string() },
        depends: p.manifest.depends.iter().map(|d| d.to_string()).collect(),
        commands: p.manifest.commands().into_iter().map(str::to_string).collect(),
        provides: p.manifest.provides.clone(),
        level: if p.manifest.level() <= 1 { 0 } else { p.manifest.level() },
        author_key: None,
        author_signature: None,
    }
}

pub fn parse_index(text: &str) -> Result<Index, String> {
    let ix: Index = serde_json::from_str(text).map_err(|e| format!("index.json: {}", e))?;
    if ix.format != INDEX_FORMAT {
        return Err(format!("index.json: unsupported format '{}'", ix.format));
    }
    for e in &ix.packages {
        if !manifest::valid_id(&e.id) || !manifest::valid_version(&e.version) {
            return Err(format!("index.json: bad entry {} {}", e.id, e.version));
        }
        if !manifest::safe_rel_path(&e.file) || e.file.contains('/') {
            return Err(format!("index.json: bad file name {}", e.file));
        }
        if !e.kind.is_empty() && manifest::Kind::parse(&e.kind).is_none() && e.installable() {
            return Err(format!("index.json: bad kind '{}' in {}", e.kind, e.id));
        }
        if e.installable()
            && (e.depends.iter().any(|d| manifest::Dep::parse(d).is_none())
                || e.commands.iter().any(|c| !manifest::valid_command(c))
                || e.provides.iter().any(|m| !manifest::valid_module(m)))
        {
            return Err(format!("index.json: bad dependency, command or module in {}", e.id));
        }
        let png = |m: &str| manifest::safe_rel_path(m) && m.ends_with(".png");
        if (!e.icon.is_empty() && !png(&e.icon))
            || e.screenshots.len() > MAX_SCREENSHOTS
            || e.screenshots.iter().any(|s| !png(s) || !s.starts_with("screenshots/"))
        {
            return Err(format!("index.json: bad icon or screenshot in {}", e.id));
        }
    }
    Ok(ix)
}

/// Newest entry per id.
pub fn latest(ix: &Index) -> Vec<&IndexEntry> {
    let mut out: Vec<&IndexEntry> = Vec::new();
    for e in &ix.packages {
        match out.iter().position(|o| o.id == e.id) {
            Some(i) => {
                if manifest::compare_versions(&e.version, &out[i].version).is_gt() {
                    out[i] = e;
                }
            }
            None => out.push(e),
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Publisher status of a downloaded package, as `pkg` and the App Store show it.
#[derive(Debug, Clone, PartialEq)]
pub enum Trust {
    /// Signed with the key pinned for the repository.
    Verified,
    /// Signed, but the repository has no pinned key.
    SignedUnpinned(String),
    /// No signature: "unverified publisher".
    Unsigned,
}

impl Trust {
    pub fn label(&self) -> String {
        match self {
            Trust::Verified => "verified publisher".into(),
            Trust::SignedUnpinned(k) => format!("unverified publisher (signed by key {}..., not pinned)", &k[..k.len().min(16)]),
            Trust::Unsigned => "unverified publisher".into(),
        }
    }
}

/// Check a downloaded package against its index entry: hash, then signature.
/// `pinned` = the repository's pinned key (hex); with a pinned key a valid signature
/// is required.
pub fn verify(entry: &IndexEntry, publisher_key: Option<&str>, pinned: Option<&str>, bytes: &[u8]) -> Result<Trust, String> {
    let got = sha256_hex(bytes);
    if got != entry.sha256 {
        return Err(format!("{}: checksum mismatch (index {}, got {})", entry.file, entry.sha256, got));
    }
    if bytes.len() as u64 != entry.size {
        return Err(format!("{}: size mismatch", entry.file));
    }
    let key = pinned.or(publisher_key);
    match (&entry.signature, key) {
        (Some(sig), Some(k)) => {
            if !crate::sign::verify(k, &signed_message(&entry.id, &entry.version, &entry.sha256), sig) {
                return Err(format!("{}: bad signature", entry.file));
            }
            if pinned.is_some() {
                Ok(Trust::Verified)
            } else {
                Ok(Trust::SignedUnpinned(k.to_string()))
            }
        }
        _ if pinned.is_some() => Err(format!("{}: not signed, but the repository has a pinned key", entry.file)),
        _ => Ok(Trust::Unsigned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<(String, Vec<u8>)> {
        vec![
            ("main.py".into(), b"print('hi')\n".to_vec()),
            ("app.toml".into(), b"id = \"dev.ecm.hello\"\nname = \"Hello\"\nversion = \"1.0.0\"\nentry = \"main.py\"\n".to_vec()),
        ]
    }

    #[test]
    fn build_then_open() {
        let (p, bytes) = build(files()).unwrap();
        assert_eq!(p.manifest.id, "dev.ecm.hello");
        assert_eq!(p.entries[0].path, "app.toml");
        assert_eq!(open(&bytes).unwrap().sha256, p.sha256);
        assert_eq!(file_name(&p.manifest), "dev.ecm.hello-1.0.0.ecmpkg");
    }

    #[test]
    fn missing_entry_is_refused() {
        let mut f = files();
        f.remove(0);
        assert!(build(f).unwrap_err().contains("entry main.py"));
    }

    #[test]
    fn index_round_trip_latest_and_verify() {
        let (p, bytes) = build(files()).unwrap();
        let (pk, sk) = crate::sign::keypair_from_seed(&[9u8; 32]);
        let sig = crate::sign::sign(&sk, &signed_message(&p.manifest.id, &p.manifest.version, &p.sha256));
        let e = index_entry(&p, "dev.ecm.hello-1.0.0.ecmpkg", bytes.len() as u64, Some(sig));
        let mut old = e.clone();
        old.version = "0.9.0".into();
        let ix = Index { format: INDEX_FORMAT.into(), name: "t".into(), publisher_key: Some(pk.clone()), packages: vec![old, e.clone()] };
        let text = serde_json::to_string(&ix).unwrap();
        let back = parse_index(&text).unwrap();
        assert_eq!(back, ix);
        assert_eq!(latest(&back)[0].version, "1.0.0");

        assert_eq!(verify(&e, Some(&pk), Some(&pk), &bytes).unwrap(), Trust::Verified);
        assert!(matches!(verify(&e, Some(&pk), None, &bytes).unwrap(), Trust::SignedUnpinned(_)));
        let (other, _) = crate::sign::keypair_from_seed(&[1u8; 32]);
        assert!(verify(&e, Some(&pk), Some(&other), &bytes).unwrap_err().contains("bad signature"));
        let mut tampered = bytes.clone();
        let n = tampered.len();
        tampered[n - 1100] ^= 1;
        assert!(verify(&e, None, None, &tampered).unwrap_err().contains("checksum"));
        let mut unsigned = e.clone();
        unsigned.signature = None;
        assert_eq!(verify(&unsigned, None, None, &bytes).unwrap(), Trust::Unsigned);
        assert!(verify(&unsigned, None, Some(&pk), &bytes).is_err());
    }

    #[test]
    fn screenshots_and_icon_in_the_index() {
        let mut f = files();
        f[1].1 = b"id = \"dev.ecm.hello\"\nname = \"Hello\"\nversion = \"1.0.0\"\nentry = \"main.py\"\npublisher = \"Evan\"\nicon = \"icon.png\"\n".to_vec();
        f.push(("icon.png".into(), vec![1, 2, 3]));
        f.push(("screenshots/b.png".into(), vec![4]));
        f.push(("screenshots/a.png".into(), vec![5]));
        f.push(("screenshots/notes.txt".into(), vec![6]));
        let (p, bytes) = build(f).unwrap();
        let e = index_entry(&p, "x.ecmpkg", bytes.len() as u64, None);
        assert_eq!(e.publisher, "Evan");
        assert_eq!(e.icon, "icon.png");
        assert_eq!(e.screenshots, vec!["screenshots/a.png", "screenshots/b.png"]);
        assert!(servable(&e, "screenshots/a.png") && servable(&e, "icon.png") && !servable(&e, "main.py"));
        assert_eq!(member(&p, "screenshots/b.png"), Some(&[4u8][..]));
        assert_eq!(asset_path(&e, "icon.png"), "x.ecmpkg/icon.png");
        let ix = Index { format: INDEX_FORMAT.into(), name: "t".into(), publisher_key: None, packages: vec![e.clone()] };
        assert_eq!(parse_index(&serde_json::to_string(&ix).unwrap()).unwrap(), ix);
        // Old indexes without the fields still parse; bad members are refused.
        let old = r#"{"format":"ecm-repo-1","packages":[{"id":"a.b","name":"x","version":"1.0.0","file":"a.ecmpkg","size":1,"sha256":"00"}]}"#;
        assert!(parse_index(old).unwrap().packages[0].screenshots.is_empty());
        let bad = r#"{"format":"ecm-repo-1","packages":[{"id":"a.b","name":"x","version":"1.0.0","file":"a.ecmpkg","size":1,"sha256":"00","screenshots":["../x.png"]}]}"#;
        assert!(parse_index(bad).is_err());
        let bad_icon = r#"{"format":"ecm-repo-1","packages":[{"id":"a.b","name":"x","version":"1.0.0","file":"a.ecmpkg","size":1,"sha256":"00","icon":"main.py"}]}"#;
        assert!(parse_index(bad_icon).is_err());
    }

    #[test]
    fn kinds_in_packages_and_the_index() {
        let toml = |rest: &str| format!("id = \"dev.me.x\"\nname = \"X\"\nversion = \"1.0.0\"\n{rest}").into_bytes();
        // A program needs its bin files.
        let prog = toml("kind = \"program\"\nbin = [\"hexdump.wasm\"]\ndepends = [\"dev.me.lib >= 1.0.0\"]\n");
        assert!(build(vec![("app.toml".into(), prog.clone())]).unwrap_err().contains("bin hexdump.wasm"));
        let (p, bytes) = build(vec![("app.toml".into(), prog), ("hexdump.wasm".into(), vec![0])]).unwrap();
        let e = index_entry(&p, "x.ecmpkg", bytes.len() as u64, None);
        assert_eq!((e.kind.as_str(), e.commands.clone(), e.level), ("program", vec!["hexdump".to_string()], 2));
        assert_eq!(e.deps()[0].id, "dev.me.lib");
        assert!(e.installable());
        // A library needs every module it provides, and nothing else under python/ or lua/.
        let lib = toml("kind = \"library\"\nprovides = [\"geo\", \"shapes\"]\n");
        let files = vec![("app.toml".to_string(), lib.clone()), ("python/geo.py".to_string(), vec![1]), ("lua/geo/init.lua".to_string(), vec![2])];
        assert!(build(files.clone()).unwrap_err().contains("module shapes"));
        let mut ok = files.clone();
        ok.push(("python/shapes/__init__.py".into(), vec![3]));
        ok.push(("README.md".into(), vec![4]));
        let (p, _) = build(ok.clone()).unwrap();
        assert_eq!(library_files(&p.entries, "geo").count(), 2);
        ok.push(("python/os.py".into(), vec![5]));
        assert!(build(ok).unwrap_err().contains("module os"));
        assert_eq!(library_member("python/a/b/c.py"), Some(("python", "a")));
        assert_eq!(library_member("lua/a.lua"), Some(("lua", "a")));
        assert_eq!(library_member("python/notes.txt"), None);
        assert_eq!(library_member("docs/a.py"), None);
        // Source needs nothing; an index entry from a newer format is listed but not installable.
        build(vec![("app.toml".into(), toml("kind = \"source\"\n")), ("a.py".into(), vec![1])]).unwrap();
        let newer = r#"{"format":"ecm-repo-1","packages":[{"id":"a.b","name":"x","version":"1.0.0","file":"a.ecmpkg","size":1,"sha256":"00","kind":"hologram","level":9}]}"#;
        assert!(!parse_index(newer).unwrap().packages[0].installable());
        let bad = r#"{"format":"ecm-repo-1","packages":[{"id":"a.b","name":"x","version":"1.0.0","file":"a.ecmpkg","size":1,"sha256":"00","kind":"hologram"}]}"#;
        assert!(parse_index(bad).is_err());
        // The author's signature travels in the entry.
        let (pk, sk) = crate::sign::keypair_from_seed(&[5u8; 32]);
        let mut e = e;
        assert_eq!(e.author(), None);
        e.author_key = Some(pk.clone());
        e.author_signature = Some(crate::sign::sign(&sk, &signed_message(&e.id, &e.version, &e.sha256)));
        assert_eq!(e.author(), Some(pk.as_str()));
        e.version = "1.0.1".into();
        assert_eq!(e.author(), None);
    }

    #[test]
    fn index_rejects_path_tricks() {
        let t = r#"{"format":"ecm-repo-1","packages":[{"id":"a.b","name":"x","version":"1.0.0","file":"../x.ecmpkg","size":1,"sha256":"00"}]}"#;
        assert!(parse_index(t).is_err());
        assert!(parse_index(r#"{"format":"other","packages":[]}"#).is_err());
    }
}
