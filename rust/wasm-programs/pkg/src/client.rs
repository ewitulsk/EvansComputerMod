//! The repository client shared by `pkg` and the App Store (docs/gui/APPS.md):
//! the repository list, cached and fetched indexes, the catalog across
//! repositories, install / update / remove planning, downloads with progress,
//! unpacking into `/apps/<id>/` and the `.pkg-origin` record.
//!
//! The file system is plain `std::fs` under a root prefix (`""` on a computer,
//! a temporary folder in tests); the network is the [`Net`] trait
//! ([`SocketNet`] on a computer, a fake in tests).

use crate::http::{Scheme, Url};
use crate::manifest::{self, Kind, Manifest};
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

    /// One call to the repository's own API (`/v1/...`, `/auth/v1/...`) at the
    /// origin of `base`: the answer's status and body, whatever the status.
    fn call(&mut self, base: &Url, call: &Call) -> Result<Reply, String> {
        let _ = call;
        Err(format!("{} has no accounts (it is a plain repository)", base.origin()))
    }

    /// Repositories (`pkg serve`) found on the network (docs/DISCOVERY.md). None by default.
    fn discover(&mut self) -> Vec<ecm_disco::Found> {
        Vec::new()
    }
}

/// A request to a repository's API.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Call {
    pub method: String,
    /// Path and query under the origin, starting with `/`.
    pub path: String,
    /// The sign-in token, sent as `Authorization: Bearer`.
    pub token: Option<String>,
    pub headers: Vec<(String, String)>,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl Call {
    pub fn get(path: &str) -> Call {
        Call { method: "GET".into(), path: path.into(), ..Call::default() }
    }

    pub fn json(method: &str, path: &str, body: &serde_json::Value) -> Call {
        Call { method: method.into(), path: path.into(), content_type: "application/json".into(), body: body.to_string().into_bytes(), ..Call::default() }
    }

    pub fn token(mut self, token: Option<&str>) -> Call {
        self.token = token.map(str::to_string);
        self
    }

    /// Every header to send (the token and content type included).
    pub fn all_headers(&self) -> Vec<(String, String)> {
        let mut h = self.headers.clone();
        if let Some(t) = &self.token {
            h.push(("Authorization".into(), format!("Bearer {t}")));
        }
        if !self.content_type.is_empty() {
            h.push(("Content-Type".into(), self.content_type.clone()));
        }
        h
    }
}

/// The answer to a [`Call`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }

    /// The repository's error text (`{"error","message"}`), or the status.
    pub fn error(&self) -> String {
        let v = self.json();
        match v.get("message").and_then(|m| m.as_str()) {
            Some(m) if !m.is_empty() => m.to_string(),
            _ => format!("HTTP {}", self.status),
        }
    }
}

/// The repositories in `found` that aren't configured yet (same URL), as unpinned [`Repo`]s
/// named by the server (or from the address), never reusing a configured name.
pub fn found_repos(configured: &[Repo], found: &[ecm_disco::Found]) -> Vec<Repo> {
    let norm = |u: &str| Url::parse(u).map(|u| u.display()).unwrap_or_else(|_| u.to_string());
    let mut taken: Vec<String> = configured.iter().map(|r| r.name.clone()).collect();
    let have: Vec<String> = configured.iter().map(|r| norm(&r.url)).collect();
    let mut out: Vec<Repo> = Vec::new();
    for f in found.iter().filter(|f| f.kind == ecm_disco::types::PKG) {
        let url = norm(&format!("http://{}/", f.endpoint()));
        if have.contains(&url) || out.iter().any(|r| r.url == url) {
            continue;
        }
        let base = if crate::valid_repo_name(&f.name) { f.name.clone() } else { crate::repo_name_for(&url) };
        let mut name = base.clone();
        let mut n = 2;
        while taken.contains(&name) {
            let suffix = format!("-{n}");
            name = format!("{}{}", &base[..base.len().min(32 - suffix.len())], suffix);
            n += 1;
        }
        taken.push(name.clone());
        out.push(Repo { name, url, key: None });
    }
    out
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
    /// The author's key when the package carried the author's signature. An
    /// update signed by another author is refused.
    pub author: String,
    /// Installed because another package needs it, not because it was asked for.
    pub auto: bool,
}

impl Origin {
    pub fn to_json(&self) -> String {
        let mut v = serde_json::json!({ "origin": self.origin, "version": self.version, "sha256": self.sha256, "trust": self.trust });
        if !self.author.is_empty() {
            v["author"] = self.author.clone().into();
        }
        if self.auto {
            v["auto"] = true.into();
        }
        v.to_string() + "\n"
    }

    pub fn parse(text: &str) -> Option<Origin> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        Some(Origin {
            origin: s("origin"),
            version: s("version"),
            sha256: s("sha256"),
            trust: s("trust"),
            author: s("author"),
            auto: v.get("auto").and_then(|x| x.as_bool()).unwrap_or(false),
        })
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

    /// What must be installed before `target`: its dependencies (and theirs)
    /// that are missing or too old, in the order to install them.
    pub fn plan(&self, target: &IndexEntry, installed: &[Installed]) -> Result<Vec<Available>, String> {
        let mut out = Vec::new();
        self.visit(target, installed, &mut out, &mut vec![target.id.clone()])?;
        Ok(out)
    }

    fn visit(&self, entry: &IndexEntry, installed: &[Installed], out: &mut Vec<Available>, stack: &mut Vec<String>) -> Result<(), String> {
        for d in entry.deps() {
            if stack.contains(&d.id) {
                return Err(format!("{} and {} need each other; neither can be installed first", entry.id, d.id));
            }
            if installed.iter().any(|i| i.manifest.id == d.id && d.accepts(&i.manifest.version)) {
                continue;
            }
            if let Some(planned) = out.iter().find(|a| a.entry.id == d.id) {
                if !d.accepts(&planned.entry.version) {
                    return Err(format!("{} needs {}, but another package needs {} {}", entry.id, d, d.id, planned.entry.version));
                }
                continue;
            }
            let Some(a) = self.get(&d.id) else {
                return Err(format!("{} needs {}, which is in no repository", entry.id, d));
            };
            if !d.accepts(&a.entry.version) {
                return Err(format!("{} needs {}, but the repositories have {} {}", entry.id, d, d.id, a.entry.version));
            }
            if !a.entry.installable() {
                return Err(format!("{} needs {}, which needs a newer version of the mod", entry.id, d.id));
            }
            stack.push(d.id.clone());
            self.visit(&a.entry, installed, out, stack)?;
            stack.pop();
            out.push(a.clone());
        }
        Ok(())
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
    /// Dependencies installed along with it: (id, version).
    pub also: Vec<(String, String)>,
    /// What the package put on the computer besides its own folder
    /// (commands in /bin, modules in /lib, files in /home/src).
    pub placed: Vec<String>,
}

/// The repository client.
pub struct Client<N: Net> {
    /// Prefix for every path (`""` on a computer).
    pub root: String,
    pub net: N,
    /// Problems with single repositories that didn't stop the operation (shown by the caller).
    pub warnings: Vec<String>,
    /// Install programs and libraries nobody vouches for (no pinned key, or a
    /// file). They run with the whole computer in reach, so this is off by default.
    pub allow_unverified: bool,
    /// Accept an update signed by another author than the installed copy's.
    pub allow_author_change: bool,
}

impl<N: Net> Client<N> {
    pub fn new(root: &str, net: N) -> Client<N> {
        Client { root: root.trim_end_matches('/').to_string(), net, warnings: Vec::new(), allow_unverified: false, allow_author_change: false }
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

    /// Repositories on the network that aren't configured ([`found_repos`]).
    pub fn found_repos(&mut self) -> Vec<Repo> {
        let found = self.net.discover();
        found_repos(&self.repos(), &found)
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

    // ------------------------------------------------------------ installed packages

    /// The folder holding an installed package's files: `/apps/<id>` for an
    /// app, `/var/lib/pkg/db/<id>` for the other kinds.
    pub fn home_of(&self, id: &str) -> Option<String> {
        if !manifest::valid_id(id) {
            return None;
        }
        [paths::APPS_DIR, paths::DB_DIR]
            .iter()
            .map(|d| format!("{}/{}", d, id))
            .find(|d| fs::metadata(self.path(&format!("{}/app.toml", d))).is_ok())
    }

    pub fn installed_manifest(&self, id: &str) -> Option<Manifest> {
        manifest::parse(&self.read_text(&format!("{}/app.toml", self.home_of(id)?))?).ok()
    }

    pub fn origin(&self, id: &str) -> Option<Origin> {
        Origin::parse(&self.read_text(&format!("{}/{}", self.home_of(id)?, paths::ORIGIN_FILE))?)
    }

    /// What an installed program, library or source package put outside its folder.
    pub fn placed(&self, id: &str) -> Vec<String> {
        self.read_text(&format!("{}/{}/{}", paths::DB_DIR, id, paths::FILES_FILE))
            .map(|t| t.lines().map(str::to_string).filter(|l| !l.is_empty()).collect())
            .unwrap_or_default()
    }

    /// Everything installed (apps and the other kinds), sorted by id.
    pub fn installed(&self) -> Vec<Installed> {
        let mut out: Vec<Installed> = Vec::new();
        for dir in [paths::APPS_DIR, paths::DB_DIR] {
            if let Ok(rd) = fs::read_dir(self.path(dir)) {
                for e in rd.flatten() {
                    let id = e.file_name().to_string_lossy().into_owned();
                    if out.iter().any(|i| i.manifest.id == id) {
                        continue;
                    }
                    if let Some(m) = self.installed_manifest(&id) {
                        out.push(Installed { origin: self.origin(&id), manifest: m });
                    }
                }
            }
        }
        out.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        out
    }

    /// The installed packages that need `id`.
    pub fn dependents(&self, id: &str) -> Vec<String> {
        self.installed().into_iter().filter(|i| i.manifest.depends.iter().any(|d| d.id == id)).map(|i| i.manifest.id).collect()
    }

    /// Packages that were installed only as dependencies and that nothing needs any more.
    pub fn orphans(&self) -> Vec<String> {
        let all = self.installed();
        all.iter()
            .filter(|i| i.origin.as_ref().map(|o| o.auto).unwrap_or(false))
            .filter(|i| !all.iter().any(|o| o.manifest.depends.iter().any(|d| d.id == i.manifest.id)))
            .map(|i| i.manifest.id.clone())
            .collect()
    }

    /// Bytes of a file of an installed app (its icon, for the store).
    pub fn installed_file(&self, id: &str, rel: &str) -> Option<Vec<u8>> {
        if !manifest::safe_rel_path(rel) {
            return None;
        }
        fs::read(self.path(&format!("{}/{}", self.home_of(id)?, rel))).ok()
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

    /// Install a checked package: an app into `/apps/<id>/` (with
    /// `/var/apps/<id>/` for its data); a program, library or source package
    /// into `/var/lib/pkg/db/<id>/`, with its commands, modules or files put
    /// where the computer looks for them. An older copy is replaced. With
    /// `expect_id` the package must be that one.
    pub fn unpack(&self, bytes: &[u8], origin: &str, trust: &Trust, expect_id: Option<&str>) -> Result<InstallReport, String> {
        self.unpack_as(bytes, origin, trust, expect_id, "", None)
    }

    /// [`unpack`](Self::unpack) with the author's key and whether it comes as a dependency
    /// (`None`: keep what the installed copy says).
    pub fn unpack_as(&self, bytes: &[u8], origin: &str, trust: &Trust, expect_id: Option<&str>, author: &str, auto: Option<bool>) -> Result<InstallReport, String> {
        let p = package::open(bytes)?;
        if let Some(id) = expect_id {
            if p.manifest.id != id {
                return Err(format!("the package is {}, not {}", p.manifest.id, id));
            }
        }
        let m = p.manifest.clone();
        if m.level() > manifest::FORMAT_LEVEL {
            return Err(format!("{} needs a newer version of the mod", m.id));
        }
        if m.kind.unconfined() && *trust != Trust::Verified && !self.allow_unverified {
            return Err(format!(
                "{} is a {}: it runs with the whole computer in reach, and nobody vouches for this copy ({}). Install it anyway with --unverified",
                m.id,
                m.kind.name(),
                trust.label()
            ));
        }
        let old = self.installed_manifest(&m.id);
        let old_origin = self.origin(&m.id);
        let replaced = old.as_ref().map(|o| o.version.clone());
        let auto = auto.unwrap_or_else(|| old_origin.as_ref().map(|o| o.auto).unwrap_or(false));
        let o = Origin { origin: origin.to_string(), version: m.version.clone(), sha256: p.sha256.clone(), trust: trust.label(), author: author.to_string(), auto };

        // Where the package's files go besides its own folder.
        let mut place: Vec<(String, &[u8])> = Vec::new();
        match m.kind {
            Kind::App => {}
            Kind::Program => {
                for b in &m.bin {
                    let cmd = manifest::command_of(b);
                    if self.is_system_command(cmd) {
                        return Err(format!("{} wants to install the command '{}', which is one of the system's own", m.id, cmd));
                    }
                    let data = package::member(&p, b).ok_or_else(|| format!("bin {} is not in the package", b))?;
                    place.push((format!("{}/{}.wasm", paths::BIN_DIR, cmd), data));
                }
            }
            Kind::Library => {
                for e in p.entries.iter().filter(|e| !e.is_dir) {
                    if let Some((lang, _)) = package::library_member(&e.path) {
                        let root = if lang == "python" { paths::PYTHON_LIB_DIR } else { paths::LUA_LIB_DIR };
                        place.push((format!("{}/{}", root, &e.path[lang.len() + 1..]), &e.data));
                    }
                }
            }
            Kind::Source => {
                for e in p.entries.iter().filter(|e| !e.is_dir && e.path != "app.toml") {
                    place.push((format!("{}/{}/{}", paths::SOURCE_DIR, m.id, e.path), &e.data));
                }
            }
        }
        // A command or module already on the computer belongs to another package
        // (or to the player): never overwrite it silently.
        let mine = self.placed(&m.id);
        if m.kind != Kind::Source {
            if let Some((taken, _)) = place.iter().find(|(d, _)| !mine.contains(d) && fs::metadata(self.path(d)).is_ok()) {
                return Err(format!("{} would overwrite {}, which it did not install; remove that first", m.id, taken));
            }
        }

        // The old copy goes (a package may change kind between versions).
        if let Some(home) = self.home_of(&m.id) {
            for f in mine.iter().filter(|f| !place.iter().any(|(d, _)| d == *f)) {
                let source = old.as_ref().map(|o| o.kind == Kind::Source).unwrap_or(false);
                if !source {
                    self.remove_placed(f);
                }
            }
            self.remove_tree(&home)?;
        }
        let dir = format!("{}/{}", if m.kind == Kind::App { paths::APPS_DIR } else { paths::DB_DIR }, m.id);
        for e in &p.entries {
            let path = format!("{}/{}", dir, e.path);
            if e.is_dir {
                fs::create_dir_all(self.path(&path)).map_err(|e| format!("cannot create {}: {}", path, e))?;
            } else {
                self.write(&path, &e.data)?;
            }
        }
        self.write(&format!("{}/{}", dir, paths::ORIGIN_FILE), o.to_json().as_bytes())?;
        let placed: Vec<String> = place.iter().map(|(d, _)| d.clone()).collect();
        if m.kind == Kind::App {
            let _ = fs::create_dir_all(self.path(&format!("{}/{}", paths::APP_DATA_DIR, m.id)));
        } else {
            for (dest, data) in &place {
                self.write(dest, data)?;
            }
            self.write(&format!("{}/{}", dir, paths::FILES_FILE), (placed.join("\n") + "\n").as_bytes())?;
        }
        Ok(InstallReport { manifest: m, trust: trust.clone(), replaced, also: Vec::new(), placed })
    }

    /// Is `cmd` one of the built-in programs (in `/server-bin`)? The shell looks
    /// in `/bin` first, so a package with that name would replace it.
    fn is_system_command(&self, cmd: &str) -> bool {
        let files = [format!("{}/{}.wasm", paths::SYSTEM_BIN_DIR, cmd), format!("{}/{}", paths::SYSTEM_BIN_DIR, cmd)];
        if files.iter().any(|f| fs::metadata(self.path(f)).is_ok()) {
            return true;
        }
        // On a computer the built-in programs are a mount of the host's, which
        // only the host's own file calls see.
        #[cfg(target_arch = "wasm32")]
        if self.root.is_empty() && files.iter().any(|f| ecm_host_abi::fs::exists(f.trim_start_matches('/'))) {
            return true;
        }
        false
    }

    /// Delete one placed file and the folders it leaves empty (up to, not including, the shared roots).
    fn remove_placed(&self, file: &str) {
        let _ = fs::remove_file(self.path(file));
        let roots = [paths::BIN_DIR, paths::PYTHON_LIB_DIR, paths::LUA_LIB_DIR, paths::SOURCE_DIR];
        let mut dir = file.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
        while !dir.is_empty() && !roots.contains(&dir.as_str()) && roots.iter().any(|r| dir.starts_with(&format!("{}/", r))) {
            if fs::remove_dir(self.path(&dir)).is_err() {
                break;
            }
            dir = dir.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
        }
    }

    /// Install (or update to) the newest version of `id` from the repositories,
    /// with whatever it depends on. `Ok(None)` when it is already installed at
    /// that version or newer (unless `force`).
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

    /// Download, check and unpack one catalog entry, after the dependencies it
    /// is missing (from the configured repositories).
    pub fn install_available(&mut self, a: &Available, progress: Progress) -> Result<InstallReport, String> {
        if !a.entry.installable() {
            return Err(format!("{} needs a newer version of the mod", a.entry.id));
        }
        let deps = if a.entry.depends.is_empty() {
            Vec::new()
        } else {
            let catalog = self.catalog(false);
            catalog.plan(&a.entry, &self.installed())?
        };
        let mut also = Vec::new();
        for d in &deps {
            let r = self.install_one(d, Some(self.installed_manifest(&d.entry.id).is_none()).filter(|new| *new), progress)?;
            also.push((r.manifest.id.clone(), r.manifest.version.clone()));
        }
        // Asked for by name: no longer just a dependency.
        let mut r = self.install_one(a, Some(false), progress)?;
        r.also = also;
        Ok(r)
    }

    fn install_one(&mut self, a: &Available, auto: Option<bool>, progress: Progress) -> Result<InstallReport, String> {
        // An update must come from the same author as the installed copy.
        let author = a.entry.author().unwrap_or("").to_string();
        if let Some(old) = self.origin(&a.entry.id) {
            if !old.author.is_empty() && old.author != author && !self.allow_author_change {
                return Err(format!(
                    "{}: the installed copy was signed by its author (key {}...), this one {}; refusing the update (remove it first if you trust the new one)",
                    a.entry.id,
                    &old.author[..old.author.len().min(16)],
                    if author.is_empty() { "is not signed by an author".to_string() } else { format!("by another key ({}...)", &author[..16.min(author.len())]) }
                ));
            }
        }
        let (bytes, trust) = self.download(a, progress)?;
        self.unpack_as(&bytes, &format!("repo {} {}", a.repo.name, a.repo.url), &trust, Some(&a.entry.id), &author, auto)
    }

    /// Uninstall; `purge` also deletes an app's `/var/apps/<id>` (or a source
    /// package's files in `/home/src/<id>`). Refused while another installed
    /// package needs it.
    pub fn remove(&self, id: &str, purge: bool) -> Result<(), String> {
        self.remove_with(id, purge, false)
    }

    /// [`remove`](Self::remove); `force` removes it even when other packages need it.
    pub fn remove_with(&self, id: &str, purge: bool, force: bool) -> Result<(), String> {
        if !manifest::valid_id(id) {
            return Err(format!("{} is not installed", id));
        }
        let apps_dir = format!("{}/{}", paths::APPS_DIR, id);
        let Some(home) = self.home_of(id).or_else(|| fs::metadata(self.path(&apps_dir)).is_ok().then_some(apps_dir)) else {
            return Err(format!("{} is not installed", id));
        };
        let needed_by = self.dependents(id);
        if !needed_by.is_empty() && !force {
            return Err(format!("{} is needed by {}; remove that first", id, needed_by.join(", ")));
        }
        let kind = self.installed_manifest(id).map(|m| m.kind).unwrap_or(Kind::App);
        if kind != Kind::Source || purge {
            for f in self.placed(id) {
                self.remove_placed(&f);
            }
            if kind == Kind::Source {
                let _ = self.remove_tree(&format!("{}/{}", paths::SOURCE_DIR, id));
            }
        }
        self.remove_tree(&home)?;
        if purge && kind == Kind::App {
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

/// The real network: `http://` and `https://` through `ecm-http` (the
/// computer's sockets; the host's own when built for the host), `host://`
/// through the server's link to the repository.
pub struct SocketNet;

impl SocketNet {
    #[cfg(target_os = "wasi")]
    fn http() -> ecm_http::Client<ecm_http::sys::Sockets> {
        ecm_http::sys::client("ecm-pkg/2")
    }

    #[cfg(not(target_os = "wasi"))]
    fn http() -> ecm_http::Client<ecm_http::native::Native> {
        ecm_http::Client::new(ecm_http::native::Native::default()).agent("ecm-pkg/2")
    }

    /// Send all of `data` on a socket (`pkg serve`).
    #[cfg(target_arch = "wasm32")]
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

    #[cfg(not(target_arch = "wasm32"))]
    pub fn send_all(_fd: i32, _data: &[u8]) -> bool {
        false
    }

    /// One request over the server's link; the body with progress.
    fn host(method: &str, path: &str, headers: &[(String, String)], body: &[u8], progress: Progress) -> Result<Reply, String> {
        use ecm_host_abi::repo::{encode_request, Support, Ticket};
        match ecm_host_abi::repo::supported() {
            Support::Ready => {}
            other => return Err(other.message().to_string()),
        }
        let h: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let t = Ticket::begin(&encode_request(method, path, &h, body))?;
        let mut out = Vec::new();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let p = t.poll()?;
            loop {
                let n = t.read(&mut buf);
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&buf[..n]);
                if out.len() > MAX_DOWNLOAD {
                    return Err(format!("the answer is larger than {} bytes", MAX_DOWNLOAD));
                }
                progress(out.len(), p.total.map(|t| t as usize));
            }
            if p.done {
                return Ok(Reply { status: p.status, body: out });
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

/// The largest file a repository may send.
pub const MAX_DOWNLOAD: usize = 16 * 1024 * 1024;

impl Net for SocketNet {
    fn get(&mut self, url: &Url, file: &str, progress: Progress) -> Result<Vec<u8>, String> {
        let what = format!("{}{}", url.display(), file);
        let reply = if url.scheme == Scheme::Host {
            SocketNet::host("GET", &url.join(file), &[], &[], progress)?
        } else {
            let req = ecm_http::Request::get(&what).map_err(|e| e.to_string())?;
            let mut p = |done: u64, total: Option<u64>| progress(done as usize, total.map(|t| t as usize));
            let mut t = ecm_http::Transfer { sink: None, progress: Some(&mut p), max_body: MAX_DOWNLOAD as u64 };
            let r = SocketNet::http().send(&req, &mut t).map_err(|e| format!("{}: {}", what, e))?;
            Reply { status: r.status, body: r.body }
        };
        if reply.status != 200 {
            return Err(format!("{}: HTTP {}", what, reply.status));
        }
        Ok(reply.body)
    }

    fn call(&mut self, base: &Url, call: &Call) -> Result<Reply, String> {
        let headers = call.all_headers();
        if base.scheme == Scheme::Host {
            return SocketNet::host(&call.method, &call.path, &headers, &call.body, &mut |_, _| {});
        }
        if call.token.is_some() && !base.private() {
            return Err(format!("{} is not encrypted; a sign-in is only sent over https://", base.origin()));
        }
        let mut req = ecm_http::Request::new(&call.method, &format!("{}{}", base.origin(), call.path)).map_err(|e| e.to_string())?;
        req.body = call.body.clone();
        for (k, v) in &headers {
            req = req.header(k, v);
        }
        // No redirects: a sign-in is for the origin it was made for.
        let r = SocketNet::http().exchange(&req, &mut ecm_http::Transfer::default()).map_err(|e| e.to_string())?;
        Ok(Reply { status: r.status, body: r.body })
    }

    fn discover(&mut self) -> Vec<ecm_disco::Found> {
        #[cfg(target_arch = "wasm32")]
        {
            ecm_disco::sys::search(ecm_disco::types::PKG, ecm_disco::CLIENT_WAIT_MS).unwrap_or_default()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Vec::new()
        }
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
    /// What [`Net::discover`] finds.
    pub found: Vec<ecm_disco::Found>,
    /// Scripted API answers: `<METHOD> <origin><path>` -> reply.
    pub api: std::collections::HashMap<String, Reply>,
    /// API calls seen.
    pub calls: Vec<Call>,
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

    fn call(&mut self, base: &Url, call: &Call) -> Result<Reply, String> {
        self.calls.push(call.clone());
        if self.down.iter().any(|h| *h == base.host) {
            return Err(format!("cannot connect to {}:{}", base.host, base.port));
        }
        let key = format!("{} {}{}", call.method, base.origin(), call.path);
        Ok(self.api.get(&key).cloned().unwrap_or(Reply { status: 404, body: br#"{"error":"not_found","message":"no such page"}"#.to_vec() }))
    }

    fn discover(&mut self) -> Vec<ecm_disco::Found> {
        self.found.clone()
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

    /// A package of any kind: (manifest lines after id/name/version, extra files).
    fn pack(id: &str, version: &str, rest: &str, files: &[(&str, &[u8])]) -> (crate::package::Package, Vec<u8>) {
        let toml = format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\n{rest}\n");
        let mut all = vec![("app.toml".to_string(), toml.into_bytes())];
        all.extend(files.iter().map(|(p, d)| (p.to_string(), d.to_vec())));
        build(all).unwrap()
    }

    /// A signed repository at `url` holding `packages`; returns its public key.
    fn signed_repo(net: &mut MemoryNet, url: &str, packages: Vec<(crate::package::Package, Vec<u8>)>, author: Option<&str>) -> String {
        let (public, secret) = crate::sign::keypair_from_seed(&[7u8; 32]);
        let mut entries = Vec::new();
        let mut files = Vec::new();
        for (p, bytes) in packages {
            let file = package::file_name(&p.manifest);
            let msg = package::signed_message(&p.manifest.id, &p.manifest.version, &p.sha256);
            let mut e = index_entry(&p, &file, bytes.len() as u64, Some(crate::sign::sign(&secret, &msg)));
            if let Some(a) = author {
                e.author_key = crate::sign::public_of(a);
                e.author_signature = Some(crate::sign::sign(a, &msg));
            }
            entries.push(e);
            files.push((file, bytes));
        }
        let ix = Index { format: INDEX_FORMAT.into(), name: url.into(), publisher_key: Some(public.clone()), packages: entries };
        net.serve(url, &ix, &files);
        public
    }

    #[test]
    fn programs_libraries_and_source_go_where_the_computer_looks() {
        let root = tmp("kinds");
        let mut net = MemoryNet::default();
        let key = signed_repo(
            &mut net,
            "https://repo.test/r/stable/",
            vec![
                pack("dev.t.tools", "1.0.0", "kind = \"program\"\nbin = [\"out/hexdump.wasm\", \"b64.wasm\"]", &[("out/hexdump.wasm", b"HEX"), ("b64.wasm", b"B64"), ("README.md", b"docs")]),
                pack("dev.t.geo", "1.0.0", "kind = \"library\"\nprovides = [\"geo\"]", &[("python/geo/__init__.py", b"py"), ("python/geo/shapes.py", b"py2"), ("lua/geo.lua", b"lua")]),
                pack("dev.t.examples", "1.0.0", "kind = \"source\"", &[("blink.py", b"print(1)"), ("lib/util.lua", b"return {}")]),
                pack("dev.t.clash", "1.0.0", "kind = \"program\"\nbin = [\"hexdump.wasm\"]", &[("hexdump.wasm", b"OTHER")]),
                pack("dev.t.ls", "1.0.0", "kind = \"program\"\nbin = [\"ls.wasm\"]", &[("ls.wasm", b"EVIL")]),
            ],
            None,
        );
        let mut c = Client::new(&root, net);
        c.add_repo("official", "https://repo.test/r/stable/", Some(&key)).unwrap();
        fs::create_dir_all(c.path("/server-bin")).unwrap();
        fs::write(c.path("/server-bin/ls.wasm"), b"real ls").unwrap();

        let r = c.install("dev.t.tools", false, &mut |_, _| {}).unwrap().unwrap();
        assert_eq!(r.placed, vec!["/bin/hexdump.wasm", "/bin/b64.wasm"]);
        assert_eq!(fs::read(c.path("/bin/hexdump.wasm")).unwrap(), b"HEX");
        assert!(fs::metadata(c.path("/var/lib/pkg/db/dev.t.tools/README.md")).is_ok());
        assert!(fs::metadata(c.path("/apps/dev.t.tools")).is_err(), "not an app");
        // A command another package installed, and a system command, are never replaced.
        assert!(c.install("dev.t.clash", false, &mut |_, _| {}).unwrap_err().contains("would overwrite /bin/hexdump.wasm"));
        assert!(c.install("dev.t.ls", false, &mut |_, _| {}).unwrap_err().contains("one of the system's own"));

        c.install("dev.t.geo", false, &mut |_, _| {}).unwrap().unwrap();
        assert_eq!(fs::read(c.path("/lib/python/geo/shapes.py")).unwrap(), b"py2");
        assert_eq!(fs::read(c.path("/lib/lua/geo.lua")).unwrap(), b"lua");
        c.install("dev.t.examples", false, &mut |_, _| {}).unwrap().unwrap();
        assert_eq!(fs::read(c.path("/home/src/dev.t.examples/lib/util.lua")).unwrap(), b"return {}");

        let ids: Vec<String> = c.installed().into_iter().map(|i| format!("{}:{}", i.manifest.id, i.manifest.kind.name())).collect();
        assert_eq!(ids, vec!["dev.t.examples:source", "dev.t.geo:library", "dev.t.tools:program"]);
        assert_eq!(c.origin("dev.t.geo").unwrap().trust, "verified publisher");

        // Removing takes the commands and modules away, and the folders they leave empty.
        c.remove("dev.t.tools", false).unwrap();
        assert!(fs::metadata(c.path("/bin/hexdump.wasm")).is_err() && fs::metadata(c.path("/bin")).is_ok());
        c.remove("dev.t.geo", false).unwrap();
        assert!(fs::metadata(c.path("/lib/python/geo")).is_err() && fs::metadata(c.path("/lib/python")).is_ok());
        // Source files are the player's now: kept unless purged.
        fs::write(c.path("/home/src/dev.t.examples/mine.py"), b"x").unwrap();
        c.remove("dev.t.examples", false).unwrap();
        assert!(fs::metadata(c.path("/home/src/dev.t.examples/blink.py")).is_ok());
        c.install("dev.t.examples", false, &mut |_, _| {}).unwrap().unwrap();
        c.remove("dev.t.examples", true).unwrap();
        assert!(fs::metadata(c.path("/home/src/dev.t.examples")).is_err());
        assert!(c.installed().is_empty());
    }

    #[test]
    fn unverified_programs_are_refused_unless_asked_for() {
        let root = tmp("unverified");
        let mut net = MemoryNet::default();
        let (p, bytes) = pack("dev.t.tool", "1.0.0", "kind = \"program\"\nbin = [\"tool.wasm\"]", &[("tool.wasm", b"T")]);
        let file = package::file_name(&p.manifest);
        let ix = Index { format: INDEX_FORMAT.into(), name: "plain".into(), publisher_key: None, packages: vec![index_entry(&p, &file, bytes.len() as u64, None)] };
        net.serve("http://10.0.0.9:8080/", &ix, &[(file, bytes.clone())]);
        let mut c = Client::new(&root, net);
        c.add_repo("plain", "http://10.0.0.9:8080/", None).unwrap();
        let e = c.install("dev.t.tool", false, &mut |_, _| {}).unwrap_err();
        assert!(e.contains("whole computer in reach") && e.contains("--unverified"), "{e}");
        assert!(c.unpack(&bytes, "file x", &Trust::Unsigned, None).is_err(), "a file is no better");
        assert!(fs::metadata(c.path("/bin/tool.wasm")).is_err());
        c.allow_unverified = true;
        c.install("dev.t.tool", false, &mut |_, _| {}).unwrap().unwrap();
        assert!(fs::metadata(c.path("/bin/tool.wasm")).is_ok());
        // Apps are sandboxed: an unsigned one installs as before.
        c.allow_unverified = false;
        let (_, app_bytes) = app("dev.t.app", "1.0.0", "");
        c.unpack(&app_bytes, "file x", &Trust::Unsigned, None).unwrap();
    }

    #[test]
    fn dependencies_come_first_and_hold_what_needs_them() {
        let root = tmp("deps");
        let mut net = MemoryNet::default();
        let key = signed_repo(
            &mut net,
            "https://repo.test/r/stable/",
            vec![
                pack("dev.t.app", "1.0.0", "entry = \"main.py\"\ndepends = [\"dev.t.ui >= 1.2.0\"]", &[("main.py", b"import ui")]),
                pack("dev.t.ui", "1.2.0", "kind = \"library\"\nprovides = [\"tui\"]\ndepends = [\"dev.t.base\"]", &[("python/tui.py", b"u")]),
                pack("dev.t.base", "0.3.0", "kind = \"library\"\nprovides = [\"tbase\"]", &[("python/tbase.py", b"b")]),
                pack("dev.t.old", "1.0.0", "entry = \"main.py\"\ndepends = [\"dev.t.ui >= 2.0.0\"]", &[("main.py", b"x")]),
                pack("dev.t.lost", "1.0.0", "entry = \"main.py\"\ndepends = [\"dev.t.nowhere\"]", &[("main.py", b"x")]),
                pack("dev.t.a", "1.0.0", "kind = \"source\"\ndepends = [\"dev.t.b\"]", &[("a", b"a")]),
                pack("dev.t.b", "1.0.0", "kind = \"source\"\ndepends = [\"dev.t.a\"]", &[("b", b"b")]),
            ],
            None,
        );
        let mut c = Client::new(&root, net);
        c.add_repo("official", "https://repo.test/r/stable/", Some(&key)).unwrap();
        let cat = c.catalog(false);
        let plan: Vec<String> = cat.plan(&cat.get("dev.t.app").unwrap().entry, &[]).unwrap().into_iter().map(|a| a.entry.id).collect();
        assert_eq!(plan, vec!["dev.t.base", "dev.t.ui"], "deepest first");

        let r = c.install("dev.t.app", false, &mut |_, _| {}).unwrap().unwrap();
        assert_eq!(r.also, vec![("dev.t.base".to_string(), "0.3.0".to_string()), ("dev.t.ui".to_string(), "1.2.0".to_string())]);
        assert!(fs::metadata(c.path("/lib/python/tbase.py")).is_ok());
        assert!(c.origin("dev.t.ui").unwrap().auto && !c.origin("dev.t.app").unwrap().auto);
        // Already there: nothing more to fetch.
        assert!(c.catalog(false).plan(&cat.get("dev.t.app").unwrap().entry, &c.installed()).unwrap().is_empty());

        assert!(c.install("dev.t.old", false, &mut |_, _| {}).unwrap_err().contains("needs dev.t.ui >= 2.0.0, but the repositories have dev.t.ui 1.2.0"));
        assert!(c.install("dev.t.lost", false, &mut |_, _| {}).unwrap_err().contains("which is in no repository"));
        assert!(c.install("dev.t.a", false, &mut |_, _| {}).unwrap_err().contains("need each other"));

        // What something needs cannot be pulled out from under it.
        assert_eq!(c.dependents("dev.t.ui"), vec!["dev.t.app"]);
        assert!(c.remove("dev.t.ui", false).unwrap_err().contains("is needed by dev.t.app"));
        assert!(c.orphans().is_empty());
        c.remove("dev.t.app", false).unwrap();
        assert_eq!(c.orphans(), vec!["dev.t.ui"], "base is still needed by ui");
        c.remove("dev.t.ui", false).unwrap();
        assert_eq!(c.orphans(), vec!["dev.t.base"]);
        // Asked for by name, a dependency is no longer an orphan.
        c.install("dev.t.base", true, &mut |_, _| {}).unwrap().unwrap();
        assert!(c.orphans().is_empty());
    }

    #[test]
    fn an_update_from_another_author_is_refused() {
        let root = tmp("author");
        let (_, alice) = crate::sign::keypair_from_seed(&[1u8; 32]);
        let (_, mallory) = crate::sign::keypair_from_seed(&[2u8; 32]);
        let mut net = MemoryNet::default();
        let key = signed_repo(&mut net, "https://repo.test/r/community/", vec![app("dev.t.app", "1.0.0", "")], Some(&alice));
        let mut c = Client::new(&root, net);
        c.add_repo("community", "https://repo.test/r/community/", Some(&key)).unwrap();
        c.install("dev.t.app", false, &mut |_, _| {}).unwrap().unwrap();
        assert_eq!(c.origin("dev.t.app").unwrap().author, crate::sign::public_of(&alice).unwrap());

        // The same author's update is fine.
        signed_repo(&mut c.net, "https://repo.test/r/community/", vec![app("dev.t.app", "1.1.0", "")], Some(&alice));
        c.catalog(true);
        assert_eq!(c.install("dev.t.app", false, &mut |_, _| {}).unwrap().unwrap().manifest.version, "1.1.0");
        // Another author's (the repository's own signature is still good), and an unsigned one, are not.
        for (version, author) in [("1.2.0", Some(mallory.as_str())), ("1.3.0", None)] {
            signed_repo(&mut c.net, "https://repo.test/r/community/", vec![app("dev.t.app", version, "")], author);
            c.catalog(true);
            let e = c.install("dev.t.app", false, &mut |_, _| {}).unwrap_err();
            assert!(e.contains("refusing the update"), "{e}");
        }
        assert_eq!(c.installed_manifest("dev.t.app").unwrap().version, "1.1.0");
        c.allow_author_change = true;
        assert!(c.install("dev.t.app", false, &mut |_, _| {}).unwrap().is_some());
    }

    #[test]
    fn a_newer_format_is_listed_but_not_installed() {
        let root = tmp("level");
        let mut net = MemoryNet::default();
        let (p, bytes) = app("dev.t.future", "1.0.0", "");
        let file = package::file_name(&p.manifest);
        let mut e = index_entry(&p, &file, bytes.len() as u64, None);
        e.level = 9;
        e.kind = "hologram".into();
        let ix = Index { format: INDEX_FORMAT.into(), name: "f".into(), publisher_key: None, packages: vec![e] };
        net.serve("http://f/", &ix, &[(file, bytes)]);
        let mut c = Client::new(&root, net);
        c.add_repo("f", "http://f/", None).unwrap();
        assert_eq!(c.catalog(false).apps.len(), 1);
        assert!(c.install("dev.t.future", false, &mut |_, _| {}).unwrap_err().contains("newer version of the mod"));
    }
}
