//! The sample repository (`fixtures/repo/`): three small apps that tests,
//! golden sessions and the `app_store` scenario install.
//!
//! * `dev.ecm.notes`: the §13 Notes app in Python, with a screenshot.
//! * `dev.visual.counter`: the Visual Programmer's counter app (main.py and
//!   main.ui.json as `visual --package` writes them).
//! * `dev.ecm.netcheck`: asks for `network` and connects to the repository
//!   host (10.0.7.1:8080 in the scenario), so its first run shows a prompt.
//!
//! `store --sample-repo <dir> [--sign <key>]` writes their packages into a
//! folder for `pkg serve <dir>`.

use ecm_pkg::client::MemoryNet;
use ecm_pkg::package::{self, Index, Package, INDEX_FORMAT};

pub const NOTES_ICON: &[u8] = include_bytes!("../fixtures/repo/dev.ecm.notes/icon.png");
pub const NOTES_SHOT: &[u8] = include_bytes!("../fixtures/repo/dev.ecm.notes/screenshots/1.png");
pub const COUNTER_ICON: &[u8] = include_bytes!("../fixtures/repo/dev.visual.counter/icon.png");
pub const NETCHECK_ICON: &[u8] = include_bytes!("../fixtures/repo/dev.ecm.netcheck/icon.png");

/// One sample app: its folder's files.
pub struct SampleApp {
    pub id: &'static str,
    pub files: &'static [(&'static str, &'static [u8])],
}

pub const APPS: &[SampleApp] = &[
    SampleApp {
        id: "dev.ecm.notes",
        files: &[
            ("app.toml", include_bytes!("../fixtures/repo/dev.ecm.notes/app.toml")),
            ("main.py", include_bytes!("../fixtures/repo/dev.ecm.notes/main.py")),
            ("icon.png", NOTES_ICON),
            ("screenshots/1.png", NOTES_SHOT),
        ],
    },
    SampleApp {
        id: "dev.visual.counter",
        files: &[
            ("app.toml", include_bytes!("../fixtures/repo/dev.visual.counter/app.toml")),
            ("main.py", include_bytes!("../fixtures/repo/dev.visual.counter/main.py")),
            ("main.ui.json", include_bytes!("../fixtures/repo/dev.visual.counter/main.ui.json")),
            ("icon.png", COUNTER_ICON),
        ],
    },
    SampleApp {
        id: "dev.ecm.netcheck",
        files: &[
            ("app.toml", include_bytes!("../fixtures/repo/dev.ecm.netcheck/app.toml")),
            ("main.py", include_bytes!("../fixtures/repo/dev.ecm.netcheck/main.py")),
            ("icon.png", NETCHECK_ICON),
        ],
    },
];

/// Build every sample package: (package, archive bytes, file name).
pub fn packages() -> Result<Vec<(Package, Vec<u8>, String)>, String> {
    APPS.iter()
        .map(|a| {
            let files = a.files.iter().map(|(p, b)| (p.to_string(), b.to_vec())).collect();
            let (p, bytes) = package::build(files).map_err(|e| format!("{}: {}", a.id, e))?;
            if p.manifest.id != a.id {
                return Err(format!("{} has id {}", a.id, p.manifest.id));
            }
            let name = package::file_name(&p.manifest);
            Ok((p, bytes, name))
        })
        .collect()
}

/// The sample repository's index (signed with `secret` if given) and its files.
pub fn repository(secret: Option<&str>) -> Result<(Index, Vec<(String, Vec<u8>)>), String> {
    let mut entries = Vec::new();
    let mut files = Vec::new();
    let public = match secret {
        Some(s) => Some(ecm_pkg::sign::public_of(s).ok_or("not a signing key")?),
        None => None,
    };
    for (p, bytes, name) in packages()? {
        let sig = secret.map(|s| ecm_pkg::sign::sign(s, &package::signed_message(&p.manifest.id, &p.manifest.version, &p.sha256)));
        entries.push(package::index_entry(&p, &name, bytes.len() as u64, sig));
        files.push((name, bytes));
    }
    Ok((Index { format: INDEX_FORMAT.into(), name: "samples".into(), publisher_key: public, packages: entries }, files))
}

/// The sample repository served in memory at `url` (tests, golden sessions).
pub fn memory_repo(url: &str, secret: Option<&str>) -> MemoryNet {
    let mut net = MemoryNet::default();
    let (ix, files) = repository(secret).expect("the sample repository builds");
    net.serve(url, &ix, &files);
    net
}

/// Write the sample packages into `dir` for `pkg serve` (with `.sig` files and
/// `publisher.pub` when signed). Returns one line per file written.
pub fn write_repo(dir: &str, secret: Option<&str>) -> Result<Vec<String>, String> {
    let dir = dir.trim_end_matches('/');
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {dir}: {e}"))?;
    let (ix, files) = repository(secret)?;
    let mut out = Vec::new();
    for (name, bytes) in &files {
        let path = format!("{dir}/{name}");
        std::fs::write(&path, bytes).map_err(|e| format!("cannot write {path}: {e}"))?;
        out.push(format!("{path} ({} bytes)", bytes.len()));
        if let Some(e) = ix.packages.iter().find(|e| &e.file == name) {
            if let Some(sig) = &e.signature {
                std::fs::write(format!("{path}.sig"), sig).map_err(|e| e.to_string())?;
            }
        }
    }
    if let Some(k) = &ix.publisher_key {
        std::fs::write(format!("{dir}/{}", ecm_pkg::paths::PUBLISHER_KEY_FILE), k).map_err(|e| e.to_string())?;
        out.push(format!("{dir}/{} (signed)", ecm_pkg::paths::PUBLISHER_KEY_FILE));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_samples_build_and_list_their_pictures() {
        let p = packages().unwrap();
        assert_eq!(p.len(), 3);
        let (ix, _) = repository(None).unwrap();
        let notes = ix.packages.iter().find(|e| e.id == "dev.ecm.notes").unwrap();
        assert_eq!(notes.screenshots, vec!["screenshots/1.png"]);
        assert_eq!(notes.icon, "icon.png");
        assert_eq!(notes.publisher, "ECM Samples");
        let net = ix.packages.iter().find(|e| e.id == "dev.ecm.netcheck").unwrap();
        assert_eq!(net.permissions, vec!["network"]);
        assert!(ix.packages.iter().all(|e| e.signature.is_none()));
        let (_, k) = ecm_pkg::sign::keypair_from_seed(&[3u8; 32]);
        let (signed, _) = repository(Some(&k)).unwrap();
        assert!(signed.publisher_key.is_some() && signed.packages.iter().all(|e| e.signature.is_some()));
    }

    #[test]
    fn write_repo_makes_a_folder_pkg_serve_can_host() {
        let d = std::env::temp_dir().join(format!("ecm-sample-repo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let dir = d.to_string_lossy().replace('\\', "/");
        let (_, k) = ecm_pkg::sign::keypair_from_seed(&[4u8; 32]);
        let lines = write_repo(&dir, Some(&k)).unwrap();
        assert_eq!(lines.len(), 4);
        for a in APPS {
            let f = format!("{dir}/{}-1.0.0.ecmpkg", a.id);
            let b = std::fs::read(&f).unwrap();
            assert_eq!(package::open(&b).unwrap().manifest.id, a.id);
            assert!(std::fs::metadata(format!("{f}.sig")).is_ok());
        }
    }
}
