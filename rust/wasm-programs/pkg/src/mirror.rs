//! `pkg mirror`: copy a repository, or a chosen part of it, into a folder
//! that `pkg serve` then offers on the game's network (docs/REPO.md, "Data
//! center repositories").
//!
//! The copies keep their signatures: next to each package go `<file>.sig`
//! (the repository's signature) and `<file>.author` (the author's key and
//! signature), and `publisher.pub` holds the repository's key. Computers that
//! pin that key verify a mirrored package exactly as they would the original.

use crate::client::{Available, Catalog, Client, Net, Progress};
use crate::package::{self, IndexEntry};
use crate::{manifest, paths, Repo};

/// What to copy.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Selection {
    /// Only these ids (and what they depend on). Empty: everything.
    pub only: Vec<String>,
    /// Only these kinds (`app`, `program`, `library`, `source`). Empty: every kind.
    pub kinds: Vec<String>,
}

impl Selection {
    /// Parse a list file: one id per line, `#` comments.
    pub fn ids_from(text: &str) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for l in text.lines().map(|l| l.split('#').next().unwrap_or("").trim()).filter(|l| !l.is_empty()) {
            if !manifest::valid_id(l) {
                return Err(format!("'{}' is not a package id", l));
            }
            if !out.iter().any(|o| o == l) {
                out.push(l.to_string());
            }
        }
        Ok(out)
    }

    /// The entries of `catalog` (one repository's) this selection takes: the
    /// chosen ids, plus whatever they need from the same repository.
    pub fn choose(&self, catalog: &Catalog) -> Result<Vec<Available>, String> {
        let wanted_kind = |e: &IndexEntry| self.kinds.is_empty() || self.kinds.iter().any(|k| k == e.kind().name());
        let mut out: Vec<Available> = Vec::new();
        let add = |a: &Available, out: &mut Vec<Available>| {
            if !out.iter().any(|o| o.entry.id == a.entry.id) {
                out.push(a.clone());
            }
        };
        let roots: Vec<&Available> = if self.only.is_empty() {
            catalog.apps.iter().filter(|a| wanted_kind(&a.entry)).collect()
        } else {
            let mut r = Vec::new();
            for id in &self.only {
                r.push(catalog.get(id).ok_or_else(|| format!("{} is not in that repository", id))?);
            }
            r
        };
        for a in roots {
            if !a.entry.installable() {
                continue;
            }
            // Dependencies are copied whatever their kind: without them the package cannot be installed.
            match catalog.plan(&a.entry, &[]) {
                Ok(deps) => {
                    for d in &deps {
                        add(d, &mut out);
                    }
                    add(a, &mut out);
                }
                Err(e) if self.only.is_empty() => {
                    let _ = e; // one broken package does not stop a whole mirror
                }
                Err(e) => return Err(e),
            }
        }
        out.sort_by(|a, b| a.entry.id.cmp(&b.entry.id));
        Ok(out)
    }
}

/// What a mirror run did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Report {
    pub copied: Vec<String>,
    /// Already there with the same hash.
    pub kept: Vec<String>,
    /// Files removed because the source no longer lists them (with `prune`).
    pub removed: Vec<String>,
}

/// `<key hex> <signature hex>` in `<file>.author`.
pub fn parse_author(text: &str) -> Option<(String, String)> {
    let mut it = text.split_whitespace();
    let (k, s) = (it.next()?, it.next()?);
    (package::unhex(k)?.len() == 32 && package::unhex(s)?.len() == 64).then(|| (k.to_string(), s.to_string()))
}

impl<N: Net> Client<N> {
    /// Copy `selection` of repository `source` into `dir` (a path on this
    /// computer). Every package is downloaded and verified first; with a
    /// pinned key an unsigned or tampered package stops the run. `prune`
    /// deletes packages in `dir` the selection no longer includes.
    pub fn mirror(&mut self, source: &Repo, dir: &str, selection: &Selection, prune: bool, progress: Progress) -> Result<Report, String> {
        let ix = self.fetch_index(source)?;
        let key = ix.publisher_key.clone();
        let catalog = Catalog::merge(vec![(source.clone(), ix)]);
        let chosen = selection.choose(&catalog)?;
        let dir = dir.trim_end_matches('/');
        let mut report = Report::default();
        let read = |c: &Client<N>, p: &str| std::fs::read(c.path(p)).ok();
        for a in &chosen {
            let file = &a.entry.file;
            let dest = format!("{}/{}", dir, file);
            if read(self, &dest).map(|b| package::sha256_hex(&b)) == Some(a.entry.sha256.clone()) {
                report.kept.push(file.clone());
            } else {
                let (bytes, _) = self.download(a, progress)?;
                self.write_mirror(&dest, &bytes)?;
                report.copied.push(file.clone());
            }
            match &a.entry.signature {
                Some(sig) => self.write_mirror(&format!("{}.sig", dest), sig.as_bytes())?,
                None => {
                    let _ = std::fs::remove_file(self.path(&format!("{}.sig", dest)));
                }
            }
            match (a.entry.author(), &a.entry.author_signature) {
                (Some(k), Some(s)) => self.write_mirror(&format!("{}.author", dest), format!("{} {}\n", k, s).as_bytes())?,
                _ => {
                    let _ = std::fs::remove_file(self.path(&format!("{}.author", dest)));
                }
            }
        }
        match &key {
            Some(k) => self.write_mirror(&format!("{}/{}", dir, paths::PUBLISHER_KEY_FILE), k.as_bytes())?,
            None => {
                let _ = std::fs::remove_file(self.path(&format!("{}/{}", dir, paths::PUBLISHER_KEY_FILE)));
            }
        }
        if prune {
            if let Ok(rd) = std::fs::read_dir(self.path(dir)) {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.ends_with(".ecmpkg") && !chosen.iter().any(|a| a.entry.file == name) {
                        for f in [name.clone(), format!("{}.sig", name), format!("{}.author", name)] {
                            let _ = std::fs::remove_file(self.path(&format!("{}/{}", dir, f)));
                        }
                        report.removed.push(name);
                    }
                }
            }
            report.removed.sort();
        }
        Ok(report)
    }

    fn write_mirror(&self, path: &str, data: &[u8]) -> Result<(), String> {
        let host = self.path(path);
        if let Some(parent) = std::path::Path::new(&host).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&host, data).map_err(|e| format!("cannot write {}: {}", path, e))
    }
}

/// The index of a served folder: every valid `.ecmpkg` in it, with the
/// signatures kept next to them (`pkg build --sign` and `pkg mirror` write those).
pub fn index_of_folder(host_dir: &str, name: &str) -> (package::Index, Vec<String>, Vec<String>) {
    let mut packages = Vec::new();
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let text = |p: String| std::fs::read_to_string(p).ok().map(|s| s.trim().to_string());
    let publisher_key = text(format!("{}/{}", host_dir, paths::PUBLISHER_KEY_FILE));
    if let Ok(rd) = std::fs::read_dir(host_dir) {
        let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        for n in names.into_iter().filter(|n| n.ends_with(".ecmpkg")) {
            let Ok(bytes) = std::fs::read(format!("{}/{}", host_dir, n)) else { continue };
            match package::open(&bytes) {
                Ok(p) => {
                    let mut e = package::index_entry(&p, &n, bytes.len() as u64, text(format!("{}/{}.sig", host_dir, n)));
                    if let Some((k, s)) = text(format!("{}/{}.author", host_dir, n)).and_then(|t| parse_author(&t)) {
                        e.author_key = Some(k);
                        e.author_signature = Some(s);
                    }
                    packages.push(e);
                    files.push(n);
                }
                Err(e) => skipped.push(format!("{}: {}", n, e)),
            }
        }
    }
    (package::Index { format: package::INDEX_FORMAT.into(), name: name.to_string(), publisher_key, packages }, files, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MemoryNet;
    use crate::package::{build, index_entry, Index, INDEX_FORMAT};

    fn pack(id: &str, rest: &str, files: &[(&str, &[u8])]) -> (crate::package::Package, Vec<u8>) {
        let toml = format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\n{rest}\n");
        let mut all = vec![("app.toml".to_string(), toml.into_bytes())];
        all.extend(files.iter().map(|(p, d)| (p.to_string(), d.to_vec())));
        build(all).unwrap()
    }

    fn tmp(name: &str) -> String {
        let d = std::env::temp_dir().join(format!("ecm-pkg-mirror-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().replace('\\', "/")
    }

    #[test]
    fn a_mirror_keeps_signatures_and_serves_like_the_original() {
        let (public, secret) = crate::sign::keypair_from_seed(&[7u8; 32]);
        let (_, author) = crate::sign::keypair_from_seed(&[1u8; 32]);
        let mut net = MemoryNet::default();
        let mut entries = Vec::new();
        let mut files = Vec::new();
        for (p, bytes) in [
            pack("dev.t.app", "entry = \"main.py\"\ndepends = [\"dev.t.lib\"]", &[("main.py", b"x")]),
            pack("dev.t.lib", "kind = \"library\"\nprovides = [\"tlib\"]", &[("lua/tlib.lua", b"l")]),
            pack("dev.t.tool", "kind = \"program\"\nbin = [\"tool.wasm\"]", &[("tool.wasm", b"t")]),
            pack("dev.t.notes", "kind = \"source\"", &[("a.py", b"a")]),
        ] {
            let file = package::file_name(&p.manifest);
            let msg = package::signed_message(&p.manifest.id, &p.manifest.version, &p.sha256);
            let mut e = index_entry(&p, &file, bytes.len() as u64, Some(crate::sign::sign(&secret, &msg)));
            if p.manifest.id == "dev.t.app" {
                e.author_key = crate::sign::public_of(&author);
                e.author_signature = Some(crate::sign::sign(&author, &msg));
            }
            entries.push(e);
            files.push((file, bytes));
        }
        let ix = Index { format: INDEX_FORMAT.into(), name: "official".into(), publisher_key: Some(public.clone()), packages: entries };
        net.serve("https://repo.test/r/stable/", &ix, &files);

        let root = tmp("root");
        let mut c = Client::new(&root, net);
        let source = Repo { name: "official".into(), url: "https://repo.test/r/stable/".into(), key: Some(public.clone()) };

        // A curated part: one app; its library comes along, the rest does not.
        let only = Selection { only: Selection::ids_from("# what this data center offers\ndev.t.app\n").unwrap(), kinds: vec![] };
        let r = c.mirror(&source, "/srv/repo", &only, false, &mut |_, _| {}).unwrap();
        assert_eq!(r.copied, vec!["dev.t.app-1.0.0.ecmpkg", "dev.t.lib-1.0.0.ecmpkg"]);
        let (served, names, skipped) = index_of_folder(&c.path("/srv/repo"), "dc");
        assert!(skipped.is_empty());
        assert_eq!(names.len(), 2);
        assert_eq!(served.publisher_key.as_deref(), Some(public.as_str()));
        let app = served.packages.iter().find(|e| e.id == "dev.t.app").unwrap();
        assert!(app.author().is_some(), "the author's signature travels with the copy");
        // A computer that pins the official key verifies the copy.
        let bytes = std::fs::read(c.path("/srv/repo/dev.t.app-1.0.0.ecmpkg")).unwrap();
        assert_eq!(package::verify(app, served.publisher_key.as_deref(), Some(&public), &bytes).unwrap(), package::Trust::Verified);

        // Again: nothing to download. Everything of one kind; prune drops what is no longer chosen.
        assert_eq!(c.mirror(&source, "/srv/repo", &only, false, &mut |_, _| {}).unwrap().kept.len(), 2);
        let programs = Selection { only: vec![], kinds: vec!["program".into()] };
        let r = c.mirror(&source, "/srv/repo", &programs, true, &mut |_, _| {}).unwrap();
        assert_eq!((r.copied, r.removed), (vec!["dev.t.tool-1.0.0.ecmpkg".to_string()], vec!["dev.t.app-1.0.0.ecmpkg".to_string(), "dev.t.lib-1.0.0.ecmpkg".to_string()]));
        assert!(std::fs::metadata(c.path("/srv/repo/dev.t.app-1.0.0.ecmpkg.author")).is_err());
        // Everything.
        let all = c.mirror(&source, "/srv/all", &Selection::default(), false, &mut |_, _| {}).unwrap();
        assert_eq!(all.copied.len(), 4);

        assert!(c.mirror(&source, "/srv/x", &Selection { only: vec!["dev.t.nope".into()], kinds: vec![] }, false, &mut |_, _| {}).unwrap_err().contains("not in that repository"));
        assert!(Selection::ids_from("Bad Id\n").is_err());
        // A tampered source stops the run: nothing bad is re-served.
        c.net.files.get_mut("https://repo.test/r/stable/dev.t.notes-1.0.0.ecmpkg").unwrap()[600] ^= 1;
        assert!(c.mirror(&source, "/srv/bad", &Selection::default(), false, &mut |_, _| {}).unwrap_err().contains("checksum"));
        assert_eq!(parse_author("zz yy"), None);
    }
}
