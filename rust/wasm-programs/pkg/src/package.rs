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
    if !entries.iter().any(|e| !e.is_dir && e.path == m.entry) {
        return Err(format!("entry {} is not in the package", m.entry));
    }
    if !m.icon.is_empty() && !entries.iter().any(|e| !e.is_dir && e.path == m.icon) {
        return Err(format!("icon {} is not in the package", m.icon));
    }
    Ok(Package { manifest: m, entries, sha256: sha256_hex(bytes) })
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
    fn index_rejects_path_tricks() {
        let t = r#"{"format":"ecm-repo-1","packages":[{"id":"a.b","name":"x","version":"1.0.0","file":"../x.ecmpkg","size":1,"sha256":"00"}]}"#;
        assert!(parse_index(t).is_err());
        assert!(parse_index(r#"{"format":"other","packages":[]}"#).is_err());
    }
}
