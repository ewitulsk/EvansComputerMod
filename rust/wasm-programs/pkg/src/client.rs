//! The repository client shared by `pkg` and the App Store (docs/gui/APPS.md):
//! the repository list, cached and fetched indexes, the catalog across
//! repositories, install / update / remove planning, downloads with progress,
//! unpacking into `/apps/<id>/` and the `.pkg-origin` record.
//!
//! The file system is plain `std::fs` under a root prefix (`""` on a computer,
//! a temporary folder in tests); the network is the [`Net`] trait
//! ([`SocketNet`] on a computer, a fake in tests).

use crate::http::Url;
use crate::manifest::{self, Manifest};
use crate::package::{self, Index, IndexEntry, Trust};
use crate::{paths, Repo};
use std::cmp::Ordering;
use std::fs;
use std::path::Path;

/// Download progress: (bytes so far, total if known).
pub type Progress<'a> = &'a mut dyn FnMut(usize, Option<usize>);

/// How the client reaches repositories.
pub trait Net {
    /// GET `file` (relative to the repository URL `base`), calling `progress`
    /// after every chunk.
    fn get(&mut self, base: &Url, file: &str, progress: Progress) -> Result<Vec<u8>, String>;
}

/// An app that is installed: its manifest and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub manifest: Manifest,
    pub origin: Option<Origin>,
}

/// `/apps/<id>/.pkg-origin`: where an installed app came from, and its trust.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Origin {
    /// `repo <name> <url>` or `file <path>`.
    pub origin: String,
    pub version: String,
    pub sha256: String,
    /// [`Trust::label`] at install time.
    pub trust: String,
}

impl Origin {
    pub fn to_json(&self) -> String {
        serde_json::json!({ "origin": self.origin, "version": self.version, "sha256": self.sha256, "trust": self.trust })
            .to_string()
            + "\n"
    }

    pub fn parse(text: &str) -> Option<Origin> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        Some(Origin { origin: s("origin"), version: s("version"), sha256: s("sha256"), trust: s("trust") })
    }

    /// The repository name, for `repo <name> <url>` origins.
    pub fn repo(&self) -> Option<&str> {
        self.origin.strip_prefix("repo ")?.split_whitespace().next()
    }
}

/// What a signature check will say before the download, from the index alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustHint {
    /// Signed, and the repository pins the publisher's key: "verified publisher".
    Verified,
    /// Signed with the index's key, which isn't pinned: "unverified publisher (signed)".
    SignedUnpinned,
    /// Not signed: "unverified publisher".
    Unsigned,
    /// The repository pins a key but the package isn't signed: installing is refused.
    Refused,
}

impl TrustHint {
    pub fn of(entry: &IndexEntry, pinned: Option<&str>, publisher_key: Option<&str>) -> TrustHint {
        match (entry.signature.is_some(), pinned, publisher_key) {
            (true, Some(_), _) => TrustHint::Verified,
            (true, None, Some(_)) => TrustHint::SignedUnpinned,
            (false, Some(_), _) => TrustHint::Refused,
            _ => TrustHint::Unsigned,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TrustHint::Verified => "verified publisher",
            TrustHint::SignedUnpinned => "unverified publisher (signed)",
            TrustHint::Unsigned => "unverified publisher",
            TrustHint::Refused => "unsigned (refused: the repository requires signatures)",
        }
    }

    pub fn verified(self) -> bool {
        self == TrustHint::Verified
    }
}

/// An app a repository offers (the newest version there).
#[derive(Debug, Clone, PartialEq)]
pub struct Available {
    pub repo: Repo,
    pub publisher_key: Option<String>,
    pub entry: IndexEntry,
}

impl Available {
    pub fn trust(&self) -> TrustHint {
        TrustHint::of(&self.entry, self.repo.key.as_deref(), self.publisher_key.as_deref())
    }

    pub fn url(&self) -> Result<Url, String> {
        Url::parse(&self.repo.url)
    }
}

/// What installing `available` would do to what is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    Install,
    Update { from: String },
    UpToDate,
    /// The installed version is newer than the repository's.
    Newer { installed: String },
}

impl Plan {
    pub fn of(installed: Option<&str>, available: &str) -> Plan {
        match installed {
            None => Plan::Install,
            Some(v) => match manifest::compare_versions(available, v) {
                Ordering::Greater => Plan::Update { from: v.to_string() },
                Ordering::Equal => Plan::UpToDate,
                Ordering::Less => Plan::Newer { installed: v.to_string() },
            },
        }
    }
}

/// Every app across the configured repositories: the newest version of each
/// id (earlier repositories win ties), plus the repositories that failed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalog {
    pub apps: Vec<Available>,
    pub errors: Vec<(String, String)>,
}

impl Catalog {
    /// Merge indexes (in repository order).
    pub fn merge(indexes: Vec<(Repo, Index)>) -> Catalog {
        let mut apps: Vec<Available> = Vec::new();
        for (repo, ix) in indexes {
            for e in package::latest(&ix) {
                let cand = Available { repo: repo.clone(), publisher_key: ix.publisher_key.clone(), entry: e.clone() };
                match apps.iter().position(|a| a.entry.id == e.id) {
                    Some(i) => {
                        if manifest::compare_versions(&e.version, &apps[i].entry.version).is_gt() {
                            apps[i] = cand;
                        }
                    }
                    None => apps.push(cand),
                }
            }
        }
        apps.sort_by(|a, b| a.entry.name.to_lowercase().cmp(&b.entry.name.to_lowercase()).then(a.entry.id.cmp(&b.entry.id)));
        Catalog { apps, errors: Vec::new() }
    }

    pub fn get(&self, id: &str) -> Option<&Available> {
        self.apps.iter().find(|a| a.entry.id == id)
    }

    /// Apps whose id, name, summary or publisher contain every word of `term` (case-insensitive).
    pub fn search(&self, term: &str) -> Vec<&Available> {
        let words: Vec<String> = term.split_whitespace().map(str::to_lowercase).collect();
        self.apps
            .iter()
            .filter(|a| {
                let hay = format!("{} {} {} {}", a.entry.id, a.entry.name, a.entry.summary, a.entry.publisher).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .collect()
    }

    /// Installed apps a repository has a newer version of.
    pub fn updates<'a>(&'a self, installed: &'a [Installed]) -> Vec<(&'a Installed, &'a Available)> {
        installed
            .iter()
            .filter_map(|i| {
                let a = self.get(&i.manifest.id)?;
                matches!(Plan::of(Some(&i.manifest.version), &a.entry.version), Plan::Update { .. }).then_some((i, a))
            })
            .collect()
    }
}

/// The result of an install.
#[derive(Debug, Clone, PartialEq)]
pub struct InstallReport {
    pub manifest: Manifest,
    pub trust: Trust,
    /// The version that was replaced, if any.
    pub replaced: Option<String>,
}

/// The repository client.
pub struct Client<N: Net> {
    /// Prefix for every path (`""` on a computer).
    pub root: String,
    pub net: N,
    /// Problems with single repositories that didn't stop the operation (shown by the caller).
    pub warnings: Vec<String>,
}

impl<N: Net> Client<N> {
    pub fn new(root: &str, net: N) -> Client<N> {
        Client { root: root.trim_end_matches('/').to_string(), net, warnings: Vec::new() }
    }

    /// A guest path on the host (the root prefix added).
    pub fn path(&self, p: &str) -> String {
        format!("{}{}", self.root, p)
    }

    fn read_text(&self, p: &str) -> Option<String> {
        fs::read_to_string(self.path(p)).ok()
    }

    fn write(&self, p: &str, data: &[u8]) -> Result<(), String> {
        let host = self.path(p);
        if let Some(parent) = Path::new(&host).parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(&host, data).map_err(|e| format!("cannot write {}: {}", p, e))
    }

    // ------------------------------------------------------------ repositories

    pub fn repos(&self) -> Vec<Repo> {
        crate::parse_repos(&self.read_text(paths::REPOS).unwrap_or_default())
    }

    pub fn save_repos(&self, r: &[Repo]) -> Result<(), String> {
        self.write(paths::REPOS, crate::format_repos(r).as_bytes())
    }

    /// Add (or replace) a repository; the index isn't fetched.
    pub fn add_repo(&self, name: &str, url: &str, key: Option<&str>) -> Result<Repo, String> {
        if !crate::valid_repo_name(name) {
            return Err(format!("bad repository name '{}' (letters, digits, - and _)", name));
        }
        let u = Url::parse(url)?;
        let key = match key {
            None => None,
            Some(k) => {
                if package::unhex(k).map(|b| b.len()) != Some(32) {
                    return Err("--key wants a 64-digit hex ed25519 public key".into());
                }
                Some(k.to_lowercase())
            }
        };
        let mut all = self.repos();
        all.retain(|r| r.name != name);
        let r = Repo { name: name.into(), url: u.display(), key };
        all.push(r.clone());
        self.save_repos(&all)?;
        Ok(r)
    }

    pub fn remove_repo(&self, name: &str) -> Result<(), String> {
        let mut all = self.repos();
        let n = all.len();
        all.retain(|r| r.name != name);
        if all.len() == n {
            return Err(format!("no repository named {}", name));
        }
        self.save_repos(&all)?;
        let _ = fs::remove_file(self.path(&cache_path(name)));
        Ok(())
    }

    // ------------------------------------------------------------ indexes

    pub fn cached_index(&self, r: &Repo) -> Option<Index> {
        package::parse_index(&self.read_text(&cache_path(&r.name))?).ok()
    }

    /// Fetch a repository's index and cache it.
    pub fn fetch_index(&mut self, r: &Repo) -> Result<Index, String> {
        let url = Url::parse(&r.url)?;
        let body = self.net.get(&url, "index.json", &mut |_, _| {})?;
        let text = String::from_utf8(body).map_err(|_| "index.json is not UTF-8".to_string())?;
        let ix = package::parse_index(&text)?;
        self.write(&cache_path(&r.name), text.as_bytes())?;
        Ok(ix)
    }

    /// The cached index, fetched if there is none yet.
    pub fn index_of(&mut self, r: &Repo) -> Result<Index, String> {
        match self.cached_index(r) {
            Some(ix) => Ok(ix),
            None => self.fetch_index(r),
        }
    }

    /// Every repository's apps (`refresh` fetches every index again).
    pub fn catalog(&mut self, refresh: bool) -> Catalog {
        let mut indexes = Vec::new();
        let mut errors = Vec::new();
        for r in self.repos() {
            match if refresh { self.fetch_index(&r) } else { self.index_of(&r) } {
                Ok(ix) => indexes.push((r, ix)),
                Err(e) => errors.push((r.name.clone(), e)),
            }
        }
        let mut c = Catalog::merge(indexes);
        c.errors = errors;
        c
    }

    /// The newest version of `id` across repositories.
    pub fn find(&mut self, id: &str, refresh: bool) -> Result<Available, String> {
        if self.repos().is_empty() {
            return Err("no repositories; add one with pkg repo add".into());
        }
        let c = self.catalog(refresh);
        for (name, e) in &c.errors {
            self.warnings.push(format!("{}: {}", name, e));
        }
        c.get(id).cloned().ok_or_else(|| format!("{} is not in any repository", id))
    }

    // ------------------------------------------------------------ installed apps

    pub fn installed_manifest(&self, id: &str) -> Option<Manifest> {
        manifest::parse(&self.read_text(&format!("{}/{}/app.toml", paths::APPS_DIR, id))?).ok()
    }

    pub fn origin(&self, id: &str) -> Option<Origin> {
        Origin::parse(&self.read_text(&format!("{}/{}/{}", paths::APPS_DIR, id, paths::ORIGIN_FILE))?)
    }

    /// Installed apps, sorted by id.
    pub fn installed(&self) -> Vec<Installed> {
        let mut out = Vec::new();
        if let Ok(rd) = fs::read_dir(self.path(paths::APPS_DIR)) {
            for e in rd.flatten() {
                let id = e.file_name().to_string_lossy().into_owned();
                if let Some(m) = self.installed_manifest(&id) {
                    out.push(Installed { origin: self.origin(&id), manifest: m });
                }
            }
        }
        out.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        out
    }

    /// Bytes of a file of an installed app (its icon, for the store).
    pub fn installed_file(&self, id: &str, rel: &str) -> Option<Vec<u8>> {
        if !manifest::safe_rel_path(rel) {
            return None;
        }
        fs::read(self.path(&format!("{}/{}/{}", paths::APPS_DIR, id, rel))).ok()
    }

    // ------------------------------------------------------------ install / remove

    /// Download a package and check it against its index entry (size, hash, signature).
    pub fn download(&mut self, a: &Available, progress: Progress) -> Result<(Vec<u8>, Trust), String> {
        if a.trust() == TrustHint::Refused {
            return Err(format!("{}: not signed, but the repository has a pinned key", a.entry.file));
        }
        let url = a.url()?;
        let bytes = self.net.get(&url, &a.entry.file, progress)?;
        let trust = package::verify(&a.entry, a.publisher_key.as_deref(), a.repo.key.as_deref(), &bytes)?;
        Ok((bytes, trust))
    }

    /// A package member a repository serves (icon, screenshot).
    pub fn fetch_asset(&mut self, a: &Available, member: &str) -> Result<Vec<u8>, String> {
        if !package::servable(&a.entry, member) {
            return Err(format!("{} is not an icon or screenshot of {}", member, a.entry.id));
        }
        let url = a.url()?;
        self.net.get(&url, &package::asset_path(&a.entry, member), &mut |_, _| {})
    }

    /// Unpack a checked package into `/apps/<id>/` (replacing an older copy),
    /// write `.pkg-origin` and create `/var/apps/<id>/`. With `expect_id` the
    /// package must be that app.
    pub fn unpack(&self, bytes: &[u8], origin: &str, trust: &Trust, expect_id: Option<&str>) -> Result<InstallReport, String> {
        let p = package::open(bytes)?;
        if let Some(id) = expect_id {
            if p.manifest.id != id {
                return Err(format!("the package is {}, not {}", p.manifest.id, id));
            }
        }
        let m = p.manifest.clone();
        let replaced = self.installed_manifest(&m.id).map(|o| o.version);
        let dir = format!("{}/{}", paths::APPS_DIR, m.id);
        self.remove_tree(&dir)?;
        for e in &p.entries {
            let path = format!("{}/{}", dir, e.path);
            if e.is_dir {
                fs::create_dir_all(self.path(&path)).map_err(|e| format!("cannot create {}: {}", path, e))?;
            } else {
                self.write(&path, &e.data)?;
            }
        }
        let o = Origin { origin: origin.to_string(), version: m.version.clone(), sha256: p.sha256.clone(), trust: trust.label() };
        self.write(&format!("{}/{}", dir, paths::ORIGIN_FILE), o.to_json().as_bytes())?;
        let _ = fs::create_dir_all(self.path(&format!("{}/{}", paths::APP_DATA_DIR, m.id)));
        Ok(InstallReport { manifest: m, trust: trust.clone(), replaced })
    }

    /// Install (or update to) the newest version of `id` from the repositories.
    /// `Ok(None)` when it is already installed at that version or newer (unless `force`).
    pub fn install(&mut self, id: &str, force: bool, progress: Progress) -> Result<Option<InstallReport>, String> {
        let a = self.find(id, false)?;
        if !force {
            if let Some(m) = self.installed_manifest(id) {
                if !matches!(Plan::of(Some(&m.version), &a.entry.version), Plan::Install | Plan::Update { .. }) {
                    return Ok(None);
                }
            }
        }
        self.install_available(&a, progress).map(Some)
    }

    /// Download, check and unpack one catalog entry.
    pub fn install_available(&mut self, a: &Available, progress: Progress) -> Result<InstallReport, String> {
        let (bytes, trust) = self.download(a, progress)?;
        self.unpack(&bytes, &format!("repo {} {}", a.repo.name, a.repo.url), &trust, Some(&a.entry.id))
    }

    /// Uninstall; `purge` also deletes `/var/apps/<id>`.
    pub fn remove(&self, id: &str, purge: bool) -> Result<(), String> {
        let dir = format!("{}/{}", paths::APPS_DIR, id);
        if !manifest::valid_id(id) || (self.installed_manifest(id).is_none() && fs::metadata(self.path(&dir)).is_err()) {
            return Err(format!("{} is not installed", id));
        }
        self.remove_tree(&dir)?;
        if purge {
            self.remove_tree(&format!("{}/{}", paths::APP_DATA_DIR, id))?;
        }
        Ok(())
    }

    /// Delete a directory tree (files, then the emptied directories).
    pub fn remove_tree(&self, p: &str) -> Result<(), String> {
        let host = self.path(p);
        let Ok(rd) = fs::read_dir(&host) else { return Ok(()) };
        for e in rd.flatten() {
            let child = format!("{}/{}", p, e.file_name().to_string_lossy());
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                self.remove_tree(&child)?;
            } else {
                fs::remove_file(self.path(&child)).map_err(|e| format!("cannot delete {}: {}", child, e))?;
            }
        }
        fs::remove_dir(&host).map_err(|e| format!("cannot delete {}: {}", p, e))
    }
}

/// Where an index is cached: `/var/lib/pkg/<repo>.index.json`.
pub fn cache_path(repo: &str) -> String {
    format!("{}/{}.index.json", paths::CACHE_DIR, repo)
}

/// Ask the app manager to rescan `/apps` (`app_ctl reindex`). `Err` when there
/// is none or it failed.
pub fn reindex() -> Result<String, String> {
    match ecm_host_abi::apps::ctl(&["reindex"]) {
        Some((0, r)) => Ok(String::from_utf8_lossy(&r).into_owned()),
        Some((s, r)) => Err(format!("app manager reindex failed ({}): {}", s, String::from_utf8_lossy(&r).trim())),
        None => Err("no app manager on this computer".into()),
    }
}

/// HTTP over the computer's sockets (wasm32 only).
#[cfg(target_arch = "wasm32")]
pub struct SocketNet;

#[cfg(target_arch = "wasm32")]
impl SocketNet {
    fn connect(url: &Url) -> Result<i32, String> {
        use ecm_host_abi::socket::{self, SockAddrIn, AF_INET, SOCK_STREAM};
        let mut addr = SockAddrIn::default();
        if socket::getaddrinfo(&url.host, &mut addr) != 0 {
            return Err(format!("cannot resolve {}", url.host));
        }
        addr.sin_port = url.port.to_be();
        let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
        if fd < 0 {
            return Err(format!("cannot open a socket ({})", fd));
        }
        if socket::connect(fd, &addr) != 0 {
            socket::close(fd);
            return Err(format!("cannot connect to {}:{}", url.host, url.port));
        }
        Ok(fd)
    }

    /// Send all of `data`.
    pub fn send_all(fd: i32, mut data: &[u8]) -> bool {
        use ecm_host_abi::socket;
        while !data.is_empty() {
            let n = socket::send(fd, data, 0);
            if n <= 0 {
                return false;
            }
            data = &data[n as usize..];
        }
        true
    }
}

#[cfg(target_arch = "wasm32")]
impl Net for SocketNet {
    fn get(&mut self, url: &Url, file: &str, progress: Progress) -> Result<Vec<u8>, String> {
        use crate::http;
        use ecm_host_abi::socket;
        let fd = SocketNet::connect(url)?;
        let path = url.join(file);
        if !SocketNet::send_all(fd, &http::get_request(&url.host, &path)) {
            socket::close(fd);
            return Err(format!("send to {} failed", url.host));
        }
        let mut raw = Vec::new();
        let mut buf = vec![0u8; 8192];
        loop {
            let n = socket::recv(fd, &mut buf, 0);
            if n <= 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n as usize]);
            progress(http::body_received(&raw), http::content_length(&raw));
            if http::response_complete(&raw) || raw.len() > 16 * 1024 * 1024 {
                break;
            }
        }
        socket::close(fd);
        let r = http::parse_response(&raw).map_err(|e| format!("{}{}: {}", url.display(), file, e))?;
        if r.status != 200 {
            return Err(format!("{}{}: HTTP {}", url.display(), file, r.status));
        }
        Ok(r.body)
    }
}

/// Off a computer (native builds) there are no sockets.
#[cfg(not(target_arch = "wasm32"))]
pub struct SocketNet;

#[cfg(not(target_arch = "wasm32"))]
impl SocketNet {
    pub fn send_all(_fd: i32, _data: &[u8]) -> bool {
        false
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Net for SocketNet {
    fn get(&mut self, url: &Url, _file: &str, _progress: Progress) -> Result<Vec<u8>, String> {
        Err(format!("cannot connect to {}:{} (no network off a computer)", url.host, url.port))
    }
}

/// A repository in memory, for tests: file name -> bytes, per base URL path.
#[derive(Default, Clone)]
pub struct MemoryNet {
    pub files: std::collections::HashMap<String, Vec<u8>>,
    /// Requests seen, as `<url><file>`.
    pub log: Vec<String>,
    /// Hosts that refuse connections.
    pub down: Vec<String>,
}

impl MemoryNet {
    /// Serve `bytes` as `<url><file>`.
    pub fn put(&mut self, url: &str, file: &str, bytes: Vec<u8>) {
        self.files.insert(format!("{}{}", url, file), bytes);
    }

    /// Serve a whole repository the way `pkg serve` does: packages (with
    /// signatures in the entries already), their icons and screenshots, and the index.
    pub fn serve(&mut self, url: &str, index: &Index, packages: &[(String, Vec<u8>)]) {
        self.put(url, "index.json", serde_json::to_vec_pretty(index).unwrap_or_default());
        for (file, bytes) in packages {
            self.put(url, file, bytes.clone());
            if let (Some(e), Ok(p)) = (index.packages.iter().find(|e| &e.file == file), package::open(bytes)) {
                for m in e.screenshots.iter().chain(std::iter::once(&e.icon)).filter(|m| !m.is_empty()) {
                    if let Some(b) = package::member(&p, m) {
                        self.put(url, &package::asset_path(e, m), b.to_vec());
                    }
                }
            }
        }
    }
}

impl Net for MemoryNet {
    fn get(&mut self, base: &Url, file: &str, progress: Progress) -> Result<Vec<u8>, String> {
        let key = format!("{}{}", base.display(), file);
        self.log.push(key.clone());
        if self.down.iter().any(|h| *h == base.host) {
            return Err(format!("cannot connect to {}:{}", base.host, base.port));
        }
        let b = self.files.get(&key).cloned().ok_or_else(|| format!("{}: HTTP 404", key))?;
        // Report progress in 1 KiB steps, as a socket would.
        let mut done = 0;
        while done < b.len() {
            done = (done + 1024).min(b.len());
            progress(done, Some(b.len()));
        }
        if b.is_empty() {
            progress(0, Some(0));
        }
        Ok(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::{build, index_entry, INDEX_FORMAT};

    fn tmp(name: &str) -> String {
        let d = std::env::temp_dir().join(format!("ecm-pkg-client-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().replace('\\', "/")
    }

    fn app(id: &str, version: &str, perms: &str) -> (crate::package::Package, Vec<u8>) {
        let toml = format!(
            "id = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\nentry = \"main.py\"\nsummary = \"app {id}\"\npermissions = [{perms}]\n"
        );
        build(vec![("app.toml".into(), toml.into_bytes()), ("main.py".into(), b"print(1)\n".to_vec()), ("screenshots/1.png".into(), vec![0x89, b'P'])]).unwrap()
    }

    /// A repository at `url` with these packages, signed with `key` if given.
    fn repo(net: &mut MemoryNet, url: &str, apps: &[(&str, &str)], key: Option<&str>) -> Option<String> {
        let mut entries = Vec::new();
        let mut files = Vec::new();
        let mut public = None;
        for (id, v) in apps {
            let (p, bytes) = app(id, v, "\"network\"");
            let file = package::file_name(&p.manifest);
            let sig = key.map(|k| {
                public = crate::sign::public_of(k);
                crate::sign::sign(k, &package::signed_message(&p.manifest.id, &p.manifest.version, &p.sha256))
            });
            entries.push(index_entry(&p, &file, bytes.len() as u64, sig));
            files.push((file, bytes));
        }
        let ix = Index { format: INDEX_FORMAT.into(), name: url.into(), publisher_key: public.clone(), packages: entries };
        net.serve(url, &ix, &files);
        public
    }

    #[test]
    fn catalog_install_update_remove() {
        let root = tmp("flow");
        let mut net = MemoryNet::default();
        repo(&mut net, "http://10.0.0.1:8080/", &[("dev.a.one", "1.0.0"), ("dev.a.two", "1.0.0")], None);
        repo(&mut net, "http://10.0.0.2:8080/", &[("dev.a.one", "1.2.0")], None);
        let mut c = Client::new(&root, net);
        assert!(c.find("dev.a.one", false).unwrap_err().contains("no repositories"));
        c.add_repo("a", "http://10.0.0.1:8080/", None).unwrap();
        c.add_repo("b", "http://10.0.0.2:8080", None).unwrap();
        assert_eq!(c.repos().len(), 2);

        let cat = c.catalog(false);
        assert!(cat.errors.is_empty());
        assert_eq!(cat.apps.len(), 2);
        let one = cat.get("dev.a.one").unwrap();
        assert_eq!((one.entry.version.as_str(), one.repo.name.as_str()), ("1.2.0", "b"), "the newest version wins");
        assert_eq!(one.trust(), TrustHint::Unsigned);
        assert_eq!(cat.search("TWO").len(), 1);
        assert_eq!(cat.search("app dev").len(), 2);
        assert!(cat.search("nothing").is_empty());

        let mut steps = Vec::new();
        let r = c.install("dev.a.one", false, &mut |d, t| steps.push((d, t))).unwrap().unwrap();
        assert_eq!((r.manifest.version.as_str(), r.replaced.clone()), ("1.2.0", None));
        assert!(!steps.is_empty() && steps.last().unwrap().1 == Some(steps.last().unwrap().0), "progress reaches the total");
        assert!(fs::metadata(c.path("/apps/dev.a.one/main.py")).is_ok());
        assert!(fs::metadata(c.path("/var/apps/dev.a.one")).unwrap().is_dir());
        let o = c.origin("dev.a.one").unwrap();
        assert_eq!((o.repo(), o.trust.as_str()), (Some("b"), "unverified publisher"));
        assert_eq!(c.install("dev.a.one", false, &mut |_, _| {}).unwrap(), None, "already installed");

        // An older copy on disk is an update; planning says so.
        let (_, old) = app("dev.a.two", "0.9.0", "");
        c.unpack(&old, "file x", &Trust::Unsigned, None).unwrap();
        let inst = c.installed();
        assert_eq!(inst.len(), 2);
        let cat = c.catalog(false);
        let ups = cat.updates(&inst);
        assert_eq!(ups.len(), 1);
        assert_eq!(ups[0].0.manifest.id, "dev.a.two");
        assert_eq!(Plan::of(Some("0.9.0"), "1.0.0"), Plan::Update { from: "0.9.0".into() });
        assert_eq!(Plan::of(Some("2.0.0"), "1.0.0"), Plan::Newer { installed: "2.0.0".into() });
        assert_eq!(Plan::of(None, "1.0.0"), Plan::Install);
        let r = c.install_available(ups[0].1, &mut |_, _| {}).unwrap();
        assert_eq!(r.replaced.as_deref(), Some("0.9.0"));
        assert!(c.catalog(false).updates(&c.installed()).is_empty());

        // Remove keeps data unless purged.
        fs::write(c.path("/var/apps/dev.a.two/state"), b"x").unwrap();
        c.remove("dev.a.two", false).unwrap();
        assert!(c.installed_manifest("dev.a.two").is_none());
        assert!(fs::metadata(c.path("/var/apps/dev.a.two/state")).is_ok());
        c.unpack(&old, "file x", &Trust::Unsigned, None).unwrap();
        c.remove("dev.a.two", true).unwrap();
        assert!(fs::metadata(c.path("/var/apps/dev.a.two")).is_err());
        assert!(c.remove("dev.a.two", false).is_err());
        assert!(c.remove("../etc", false).is_err());
    }

    #[test]
    fn signatures_pinning_and_assets() {
        let root = tmp("trust");
        let mut net = MemoryNet::default();
        let (_, secret) = crate::sign::keypair_from_seed(&[7u8; 32]);
        let public = repo(&mut net, "http://signed/", &[("dev.s.app", "1.0.0")], Some(&secret)).unwrap();
        repo(&mut net, "http://plain/", &[("dev.p.app", "1.0.0")], None);
        let mut c = Client::new(&root, net);
        c.add_repo("signed", "http://signed/", None).unwrap();
        c.add_repo("plain", "http://plain/", Some(&public)).unwrap();
        let cat = c.catalog(true);
        assert_eq!(cat.get("dev.s.app").unwrap().trust(), TrustHint::SignedUnpinned);
        assert_eq!(cat.get("dev.p.app").unwrap().trust(), TrustHint::Refused, "pinned key, unsigned package");
        assert!(c.install("dev.p.app", false, &mut |_, _| {}).unwrap_err().contains("pinned key"));
        let r = c.install("dev.s.app", false, &mut |_, _| {}).unwrap().unwrap();
        assert!(matches!(r.trust, Trust::SignedUnpinned(_)));
        // Pin the key: verified.
        c.add_repo("signed", "http://signed/", Some(&public)).unwrap();
        let cat = c.catalog(false);
        let s = cat.get("dev.s.app").unwrap().clone();
        assert_eq!(s.trust(), TrustHint::Verified);
        let r = c.install_available(&s, &mut |_, _| {}).unwrap();
        assert_eq!(r.trust, Trust::Verified);
        // Screenshots come from the package, served next to it.
        assert_eq!(s.entry.screenshots, vec!["screenshots/1.png"]);
        assert_eq!(c.fetch_asset(&s, "screenshots/1.png").unwrap(), vec![0x89, b'P']);
        assert!(c.fetch_asset(&s, "main.py").is_err());
        // A tampered download is refused.
        c.net.files.get_mut("http://signed/dev.s.app-1.0.0.ecmpkg").unwrap()[700] ^= 1;
        assert!(c.install_available(&s, &mut |_, _| {}).unwrap_err().contains("checksum"));
        // An unreachable repository is reported, the others still listed.
        c.net.down.push("plain".into());
        let cat = c.catalog(true);
        assert_eq!(cat.errors.len(), 1);
        assert!(cat.get("dev.s.app").is_some());
        c.remove_repo("plain").unwrap();
        assert!(c.remove_repo("plain").is_err());
    }
}
