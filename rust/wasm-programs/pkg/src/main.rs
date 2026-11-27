//! `pkg`: the ECM package manager (docs/gui/APPS.md).
//!
//! Repositories are plain HTTP servers with an `index.json` and `.ecmpkg` files;
//! `pkg serve` is one. Installed apps live in `/apps/<id>/`, their data in
//! `/var/apps/<id>/`. After installing or removing, `pkg` asks the app manager to
//! reindex through `app_ctl`, which also launches apps (`pkg run`) and manages
//! permissions (`pkg perm`). The repository logic is `ecm_pkg::client`, shared
//! with the App Store.

use ecm_host_abi::apps;
use ecm_host_abi::socket::{self, SockAddrIn, AF_INET, SOCK_STREAM, SOL_SOCKET, SO_REUSEADDR};
use ecm_pkg::account::{self, HostSecrets, Secrets};
use ecm_pkg::client::{self, Client, SocketNet};
use ecm_pkg::http::{self, Scheme, Url};
use ecm_pkg::manifest::{self, Kind};
use ecm_pkg::mirror::{self, Selection};
use ecm_pkg::package::{self, Index, Trust};
use ecm_pkg::paths;
use std::fs;
use std::path::Path;

const USAGE: &str = "\
usage: pkg <command> [args]
  repo add <name> <url> [--key <hex>]   add a repository (a pinned key makes signatures required)
  repo list | repo remove <name>
  update                                refresh indexes, upgrade what is installed
  search [term]                         find packages in the repositories
  install <id> | <file.ecmpkg>          install from a repository or a file, with what it needs
      [--force] [--unverified]          (--unverified: a program or library nobody vouches for)
  remove <id> [--purge] [--force]       uninstall (--purge also deletes its data)
  autoremove                            remove dependencies nothing needs any more
  list                                  what is installed
  info <id>                             manifest, origin and permissions
  run <id> [args]                       launch an app and show its output
  perm list [id] | perm grant|deny|reset <id> <permission>
  open <path> [--with <id>]             open a file with its default app (or the one named)
  build <dir> [out-dir] [--sign <key>]  make <id>-<version>.ecmpkg from a folder
  keygen [file]                         make a signing key (default /etc/pkg/signing.key)
  serve <dir> [port] [--name NAME] [--tls]
                                        host <dir> as a repository (default port 8080); with
                                        disco serve on this computer it is found on the network
  mirror <repo> <dir> [--only <id,id>] [--list <file>] [--kind <kind>] [--prune]
                                        copy a repository (or part of it) into <dir> to serve
  discover                              repositories found on the network (disco)
  online [repo]                         can this computer reach its repositories?
 with an account at a repository (anyone can download without one):
  login [repo] [--key <api key>]        sign in (shows a code to enter in your browser)
  logout [repo] | whoami [repo]
  publish <dir | file.ecmpkg> [repo]    upload a package under your name
  yank <id> <version> [repo]            take a version out of the listings
  like | unlike | favorite | unfavorite <id> [repo]
  favorites [repo] | mine [repo]        your favorites; what you published";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let r = match a.as_slice() {
        ["repo", "add", name, url, rest @ ..] => repo_add(name, url, rest),
        ["repo", "list"] | ["repo"] => repo_list(),
        ["repo", "remove", name] => repo_remove(name),
        ["update"] => update(),
        ["search"] => search(""),
        ["search", term] => search(term),
        ["install", target, rest @ ..] => install(target, rest),
        ["remove", id, rest @ ..] => remove(id, rest.contains(&"--purge"), rest.contains(&"--force")),
        ["autoremove"] => autoremove(),
        ["list"] => list(),
        ["info", id] => info(id),
        ["run", id, rest @ ..] => run(id, rest),
        ["perm"] | ["perm", "list"] => perm(&["perm", "list"]),
        ["perm", "list", id] => perm(&["perm", "list", id]),
        ["perm", op @ ("grant" | "deny" | "reset"), id, p] => perm(&["perm", op, id, p]),
        ["open", path] => open(path, None),
        ["open", path, "--with", id] => open(path, Some(id)),
        ["build", dir, rest @ ..] => build(dir, rest),
        ["keygen"] => keygen(paths::SIGNING_KEY),
        ["keygen", file] => keygen(file),
        ["serve", dir, rest @ ..] => serve_args(dir, rest),
        ["mirror", repo, dir, rest @ ..] => mirror(repo, dir, rest),
        ["discover"] => discover(),
        ["online", rest @ ..] => online(rest.first().copied()),
        ["login", rest @ ..] => login(rest),
        ["logout", rest @ ..] => logout(rest.first().copied()),
        ["whoami", rest @ ..] => whoami(rest.first().copied()),
        ["publish", what, rest @ ..] => publish(what, rest.first().copied()),
        ["yank", id, version, rest @ ..] => yank(id, version, rest.first().copied()),
        [op @ ("like" | "unlike" | "favorite" | "unfavorite"), id, rest @ ..] => mark(op, id, rest.first().copied()),
        ["favorites", rest @ ..] => mine("favorites", rest.first().copied()),
        ["mine", rest @ ..] => mine("packages", rest.first().copied()),
        _ => {
            println!("{}", USAGE);
            Ok(())
        }
    };
    if let Err(e) = r {
        println!("pkg: {}", e);
        std::process::exit(1);
    }
}

fn client() -> Client<SocketNet> {
    Client::new("", SocketNet)
}

/// Print and clear the client's per-repository warnings.
fn warn(c: &mut Client<SocketNet>) {
    for w in c.warnings.drain(..) {
        println!("pkg: {}", w);
    }
}

// ------------------------------------------------------------------ files

fn read_text(p: &str) -> Option<String> {
    fs::read_to_string(p).ok()
}

fn write_file(p: &str, data: &[u8]) -> Result<(), String> {
    if let Some(parent) = Path::new(p).parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(p, data).map_err(|e| format!("cannot write {}: {}", p, e))
}

/// Every file under `dir` as (relative path, bytes), skipping hidden files.
fn walk(dir: &str, rel: &str, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
    let path = if rel.is_empty() { dir.to_string() } else { format!("{}/{}", dir, rel) };
    let rd = fs::read_dir(&path).map_err(|e| format!("cannot read {}: {}", path, e))?;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let r = if rel.is_empty() { name.clone() } else { format!("{}/{}", rel, name) };
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            walk(dir, &r, out)?;
        } else {
            let data = fs::read(format!("{}/{}", dir, r)).map_err(|e| format!("cannot read {}: {}", r, e))?;
            out.push((r, data));
        }
        if out.len() > ecm_pkg::tar::MAX_ENTRIES {
            return Err("too many files".into());
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ app manager

fn ctl(words: &[&str]) -> Result<(i32, String), String> {
    match apps::ctl(words) {
        Some((s, r)) => Ok((s, String::from_utf8_lossy(&r).into_owned())),
        None => Err("no app manager on this computer".into()),
    }
}

fn reindex() {
    match client::reindex() {
        Ok(_) => {}
        Err(e) if e.starts_with("no app manager") => println!("pkg: {} (installed files only)", e),
        Err(e) => println!("pkg: {}", e),
    }
}

// ------------------------------------------------------------------ repositories

fn repo_add(name: &str, url: &str, rest: &[&str]) -> Result<(), String> {
    let key = match rest {
        [] => None,
        ["--key", k] => Some(*k),
        _ => return Err("usage: pkg repo add <name> <url> [--key <hex>]".into()),
    };
    let mut c = client();
    let r = c.add_repo(name, url, key)?;
    match c.fetch_index(&r) {
        Ok(ix) => println!("Added {} ({}): {} packages", name, r.url, ix.packages.len()),
        Err(e) => println!("Added {} ({}); index not fetched yet: {}", name, r.url, e),
    }
    Ok(())
}

fn repo_list() -> Result<(), String> {
    let all = client().repos();
    if all.is_empty() {
        println!("No repositories. Add one with: pkg repo add <name> http://<host>:8080/");
    }
    for r in all {
        println!("{}  {}{}", r.name, r.url, if r.key.is_some() { "  (pinned key)" } else { "" });
    }
    Ok(())
}

fn repo_remove(name: &str) -> Result<(), String> {
    client().remove_repo(name)?;
    println!("Removed {}", name);
    Ok(())
}

fn search(term: &str) -> Result<(), String> {
    let mut c = client();
    let cat = c.catalog(false);
    for (name, e) in &cat.errors {
        println!("pkg: {}: {}", name, e);
    }
    let found = cat.search(term);
    if found.is_empty() {
        println!("No packages found.");
    }
    for a in found {
        let e = &a.entry;
        let signed = if e.signature.is_some() { "signed" } else { "unsigned" };
        let kind = if e.kind().name() == "app" { String::new() } else { format!("{}, ", e.kind().name()) };
        println!("{} {}  {}  [{}{}, {}]", e.id, e.version, e.name, kind, a.repo.name, signed);
        if !e.depends.is_empty() {
            println!("    needs: {}", e.depends.join(", "));
        }
        if !e.summary.is_empty() {
            println!("    {}", e.summary);
        }
        if !e.permissions.is_empty() {
            println!("    asks for: {}", e.permissions.join(", "));
        }
    }
    Ok(())
}

/// What a package put on the computer, in words.
fn report_install(r: &client::InstallReport) {
    let m = &r.manifest;
    for (id, v) in &r.also {
        println!("Installed {} {} (needed by {})", id, v, m.id);
    }
    println!("Installed {} {} ({}) - {}", m.name, m.version, m.id, r.trust.label());
    match m.kind {
        Kind::App => {
            if !m.permissions.is_empty() {
                println!("It asks for: {} (you are asked on first use)", m.permissions.join(", "));
            }
        }
        Kind::Program => println!("New commands: {}", m.commands().join(", ")),
        Kind::Library => println!("New modules: {} (import them in Python, require them in Lua)", m.provides.join(", ")),
        Kind::Source => println!("Its files are in {}/{}/", paths::SOURCE_DIR, m.id),
    }
}

fn install(target: &str, flags: &[&str]) -> Result<(), String> {
    if let Some(bad) = flags.iter().find(|f| !["--force", "--unverified"].contains(f)) {
        return Err(format!("unknown option {}", bad));
    }
    let force = flags.contains(&"--force");
    let mut c = client();
    c.allow_unverified = flags.contains(&"--unverified");
    c.allow_author_change = force;
    let report = if target.ends_with(".ecmpkg") {
        let b = fs::read(target).map_err(|e| format!("cannot read {}: {}", target, e))?;
        c.unpack(&b, &format!("file {}", target), &Trust::Unsigned, None)?
    } else {
        let a = c.find(target, false);
        warn(&mut c);
        let a = a?;
        if let Some(m) = c.installed_manifest(target) {
            if !force && manifest::compare_versions(&m.version, &a.entry.version).is_ge() {
                println!("{} {} is already installed.", m.id, m.version);
                return Ok(());
            }
        }
        if a.entry.kind().unconfined() {
            println!("Note: {} is a {}. Unlike an app it is not sandboxed: it can read and change everything on this computer.", a.entry.id, a.entry.kind().name());
        }
        println!("Downloading {} {} from {}...", a.entry.id, a.entry.version, a.repo.name);
        c.install_available(&a, &mut |_, _| {})?
    };
    reindex();
    report_install(&report);
    Ok(())
}

fn remove(id: &str, purge: bool, force: bool) -> Result<(), String> {
    client().remove_with(id, purge, force)?;
    reindex();
    println!("Removed {}{}", id, if purge { " and its data" } else { "" });
    Ok(())
}

fn autoremove() -> Result<(), String> {
    let c = client();
    let mut n = 0;
    // Removing one may orphan the next.
    loop {
        let orphans = c.orphans();
        if orphans.is_empty() {
            break;
        }
        for id in orphans {
            c.remove(&id, false)?;
            println!("Removed {} (nothing needs it any more)", id);
            n += 1;
        }
    }
    if n == 0 {
        println!("Nothing to remove.");
    } else {
        reindex();
    }
    Ok(())
}

fn update() -> Result<(), String> {
    let mut c = client();
    for r in c.repos() {
        match c.fetch_index(&r) {
            Ok(ix) => println!("{}: {} packages", r.name, ix.packages.len()),
            Err(e) => println!("pkg: {}: {}", r.name, e),
        }
    }
    let cat = c.catalog(false);
    let installed = c.installed();
    let ups: Vec<_> = cat.updates(&installed).into_iter().map(|(i, a)| (i.manifest.version.clone(), a.clone())).collect();
    for (from, a) in &ups {
        println!("Upgrading {} {} -> {}", a.entry.id, from, a.entry.version);
        match c.install_available(a, &mut |_, _| {}) {
            Ok(r) => println!("Installed {} {} ({}) - {}", r.manifest.name, r.manifest.version, r.manifest.id, r.trust.label()),
            // One package that cannot be upgraded does not stop the rest.
            Err(e) => println!("pkg: {}", e),
        }
    }
    if !ups.is_empty() {
        reindex();
    }
    println!("{}", if ups.is_empty() { "Everything is up to date.".to_string() } else { format!("{} upgraded.", ups.len()) });
    let orphans = c.orphans();
    if !orphans.is_empty() {
        println!("No longer needed: {} (pkg autoremove)", orphans.join(", "));
    }
    Ok(())
}

fn list() -> Result<(), String> {
    let all = client().installed();
    if all.is_empty() {
        println!("Nothing installed.");
    }
    for i in all {
        let kind = if i.manifest.kind == Kind::App { String::new() } else { format!("  [{}]", i.manifest.kind.name()) };
        println!("{} {}  {}{}", i.manifest.id, i.manifest.version, i.manifest.name, kind);
    }
    Ok(())
}

fn info(id: &str) -> Result<(), String> {
    let c = client();
    let m = c.installed_manifest(id).ok_or_else(|| format!("{} is not installed", id))?;
    println!("{} {} ({})", m.name, m.version, m.id);
    if !m.summary.is_empty() {
        println!("  {}", m.summary);
    }
    match m.kind {
        Kind::App => println!("  entry: {}", m.entry),
        Kind::Program => println!("  program; commands: {}", m.commands().join(", ")),
        Kind::Library => println!("  library; modules: {}", m.provides.join(", ")),
        Kind::Source => println!("  source; files in {}/{}/", paths::SOURCE_DIR, m.id),
    }
    if !m.depends.is_empty() {
        println!("  needs: {}", m.depends.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", "));
    }
    let needed_by = c.dependents(id);
    if !needed_by.is_empty() {
        println!("  needed by: {}", needed_by.join(", "));
    }
    if !m.open_with.is_empty() {
        println!("  opens: {}", m.open_with.join(" "));
    }
    if let Some(o) = c.origin(id) {
        println!("  origin: {} ({})", o.origin, o.trust);
        if !o.author.is_empty() {
            println!("  author key: {}", o.author);
        }
        if o.auto {
            println!("  installed as a dependency");
        }
    }
    if m.kind == Kind::App {
        println!("  asks for: {}", if m.permissions.is_empty() { "nothing".to_string() } else { m.permissions.join(", ") });
        if let Ok((0, r)) = ctl(&["perm", "list", id]) {
            for l in r.lines() {
                println!("  {}", l);
            }
        }
    }
    Ok(())
}

fn run(id: &str, rest: &[&str]) -> Result<(), String> {
    let mut words = vec!["launch", id];
    words.extend_from_slice(rest);
    let (s, r) = ctl(&words)?;
    if s != 0 {
        return Err(format!("cannot launch {}: {}", id, r.trim()));
    }
    let pid = r.trim().to_string();
    let mut code = 0;
    loop {
        let out = apps::ctl(&["output", &pid]).map(|(_, b)| b).unwrap_or_default();
        if !out.is_empty() {
            print!("{}", String::from_utf8_lossy(&out));
        }
        let (s, st) = ctl(&["status", &pid])?;
        if s != 0 {
            break;
        }
        let mut lines = st.lines();
        let first = lines.next().unwrap_or("");
        if let Some(c) = first.strip_prefix("exited ") {
            // Drain what arrived after the last read.
            let tail = apps::ctl(&["output", &pid]).map(|(_, b)| b).unwrap_or_default();
            if !tail.is_empty() {
                print!("{}", String::from_utf8_lossy(&tail));
            }
            code = c.trim().parse().unwrap_or(1);
            let declared = client().installed_manifest(id).map(|m| m.permissions).unwrap_or_default();
            for l in lines {
                if let Some(p) = l.strip_prefix("denied ") {
                    if declared.iter().any(|d| d == p) {
                        println!("pkg: {} was denied {} (grant it with: pkg perm grant {} {})", id, p, id, p);
                    } else {
                        println!("pkg: {} was denied {} (its app.toml doesn't ask for it)", id, p);
                    }
                }
            }
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if code != 0 {
        println!("pkg: {} exited with code {}", id, code);
    }
    Ok(())
}

fn perm(words: &[&str]) -> Result<(), String> {
    let (s, r) = ctl(words)?;
    if s != 0 {
        return Err(r.trim().to_string());
    }
    let r = r.trim();
    println!("{}", if r.is_empty() { "No permission decisions yet." } else { r });
    Ok(())
}

fn open(path: &str, with: Option<&str>) -> Result<(), String> {
    let (s, r) = match with {
        Some(id) => ctl(&["openwith", path, id])?,
        None => ctl(&["open", path])?,
    };
    if s == -35 {
        return Err(format!("{} (pick one with: pkg open {} --with <id>)", r.trim(), path));
    }
    if s != 0 {
        return Err(r.trim().to_string());
    }
    println!("Opened {} with {}", path, r.trim());
    Ok(())
}

// ------------------------------------------------------------------ publishing

fn build(dir: &str, rest: &[&str]) -> Result<(), String> {
    let mut out_dir = ".".to_string();
    let mut key_file: Option<String> = None;
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "--sign" if i + 1 < rest.len() => {
                key_file = Some(rest[i + 1].to_string());
                i += 1;
            }
            d if !d.starts_with("--") => out_dir = d.to_string(),
            other => return Err(format!("unknown option {}", other)),
        }
        i += 1;
    }
    let dir = dir.trim_end_matches('/');
    let mut files = Vec::new();
    walk(dir, "", &mut files)?;
    let (p, bytes) = package::build(files)?;
    let name = package::file_name(&p.manifest);
    let out = format!("{}/{}", out_dir.trim_end_matches('/'), name);
    write_file(&out, &bytes)?;
    println!("Built {} ({} files, {} bytes)", out, p.entries.len(), bytes.len());
    if let Some(kf) = key_file {
        let secret = read_text(&kf).ok_or_else(|| format!("cannot read key {}", kf))?;
        let secret = secret.trim();
        let public = ecm_pkg::sign::public_of(secret).ok_or_else(|| format!("{} is not a pkg signing key", kf))?;
        let sig = ecm_pkg::sign::sign(secret, &package::signed_message(&p.manifest.id, &p.manifest.version, &p.sha256));
        write_file(&format!("{}.sig", out), sig.as_bytes())?;
        write_file(&format!("{}/{}", out_dir.trim_end_matches('/'), paths::PUBLISHER_KEY_FILE), public.as_bytes())?;
        println!("Signed with key {}", public);
    }
    Ok(())
}

fn keygen(file: &str) -> Result<(), String> {
    if fs::metadata(file).is_ok() {
        return Err(format!("{} exists; delete it first to make a new key", file));
    }
    let (public, secret) = ecm_pkg::sign::generate();
    write_file(file, secret.as_bytes())?;
    println!("Signing key written to {}", file);
    println!("Public key (give it to people who pin your repository):");
    println!("{}", public);
    Ok(())
}

/// index.json for every valid package in `dir` (signatures from the files next to them).
fn make_index(dir: &str) -> (Index, Vec<String>) {
    let (ix, files, skipped) = mirror::index_of_folder(dir, dir);
    for s in skipped {
        println!("pkg serve: skipping {}", s);
    }
    (ix, files)
}

/// `pkg serve <dir> [port] [--name NAME] [--tls]`.
fn serve_args(dir: &str, rest: &[&str]) -> Result<(), String> {
    let mut port = None;
    let mut name = String::new();
    let mut tls = false;
    let mut upstream = None;
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "--mirror" => {
                i += 1;
                upstream = Some(rest.get(i).ok_or("--mirror wants a repository's name")?.to_string());
            }
            "--name" => {
                i += 1;
                name = rest.get(i).ok_or("--name wants a name")?.to_string();
                if !ecm_pkg::valid_repo_name(&name) {
                    return Err(format!("bad repository name '{}' (letters, digits, - and _)", name));
                }
            }
            "--tls" => tls = true,
            p => port = Some(p.parse().map_err(|_| format!("bad port {}", p))?),
        }
        i += 1;
    }
    let identity = if tls {
        Some(ecm_tls::Identity::load(ecm_tls::SERVER_CERT, ecm_tls::SERVER_KEY).map_err(|e| format!("{} (get a certificate first: cert request <this computer's name>)", e))?)
    } else {
        None
    };
    if let Some(repo) = upstream {
        // A repository this computer does not have is the end of it (nothing to offer);
        // one that cannot be reached right now is not: the last copy is still served.
        let mut c = client();
        let source = find_repo(&c, Some(&repo))?;
        println!("Mirroring {} ({}) into {} ...", source.name, source.url, dir);
        match c.mirror(&source, dir, &Selection::default(), false, &mut |_, _| {}) {
            Ok(r) => println!("{} copied, {} already there.", r.copied.len(), r.kept.len()),
            Err(e) => println!("pkg: {} cannot be copied now ({}); serving the copy that is here", source.name, e),
        }
    }
    serve(dir, port.unwrap_or(if tls { 8443 } else { 8080 }), &name, identity)
}

/// `pkg mirror <repo> <dir> [--only a,b] [--list file] [--kind k] [--prune]`.
fn mirror(repo: &str, dir: &str, rest: &[&str]) -> Result<(), String> {
    let mut sel = Selection::default();
    let mut prune = false;
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "--only" => {
                i += 1;
                let ids = rest.get(i).ok_or("--only wants ids, comma separated")?;
                sel.only.extend(Selection::ids_from(&ids.replace(',', "\n"))?);
            }
            "--list" => {
                i += 1;
                let f = rest.get(i).ok_or("--list wants a file of ids")?;
                sel.only.extend(Selection::ids_from(&read_text(f).ok_or_else(|| format!("cannot read {}", f))?)?);
            }
            "--kind" => {
                i += 1;
                let k = rest.get(i).ok_or("--kind wants app, program, library or source")?;
                Kind::parse(k).ok_or_else(|| format!("unknown kind {} (app, program, library, source)", k))?;
                sel.kinds.push(k.to_string());
            }
            "--prune" => prune = true,
            other => return Err(format!("unknown option {}", other)),
        }
        i += 1;
    }
    let mut c = client();
    let source = find_repo(&c, Some(repo))?;
    println!("Mirroring {} ({}) into {} ...", source.name, source.url, dir);
    let r = c.mirror(&source, dir, &sel, prune, &mut |_, _| {})?;
    for f in &r.copied {
        println!("  copied {}", f);
    }
    for f in &r.removed {
        println!("  removed {}", f);
    }
    println!("{} copied, {} already there{}.", r.copied.len(), r.kept.len(), if r.removed.is_empty() { String::new() } else { format!(", {} removed", r.removed.len()) });
    println!("Offer it on the network with: pkg serve {} &", dir);
    Ok(())
}

// ------------------------------------------------------------------ accounts

/// A configured repository by name, or an address typed in; with none, the
/// first repository that can have accounts (https:// or host://).
fn find_repo(c: &Client<SocketNet>, arg: Option<&str>) -> Result<ecm_pkg::Repo, String> {
    let repos = c.repos();
    match arg {
        Some(a) if a.contains("://") => {
            let u = Url::parse(a)?;
            Ok(repos.iter().find(|r| Url::parse(&r.url).map(|x| x.origin() == u.origin()).unwrap_or(false)).cloned().unwrap_or(ecm_pkg::Repo {
                name: ecm_pkg::repo_name_for(a),
                url: u.display(),
                key: None,
            }))
        }
        Some(a) => repos.into_iter().find(|r| r.name == a).ok_or_else(|| format!("no repository named {} (pkg repo list)", a)),
        None => repos
            .into_iter()
            .find(|r| Url::parse(&r.url).map(|u| u.private()).unwrap_or(false))
            .ok_or_else(|| "no repository with accounts is configured (pkg repo add official https://...)".to_string()),
    }
}

fn account_base(arg: Option<&str>) -> Result<Url, String> {
    Url::parse(&find_repo(&client(), arg)?.url)
}

fn computer_name() -> String {
    read_text("/etc/hostname").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "a computer".into())
}

fn login(rest: &[&str]) -> Result<(), String> {
    let (mut repo, mut key) = (None, None);
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match *a {
            "--key" => key = Some(it.next().ok_or("--key wants the API key from your account page")?.to_string()),
            s if s.starts_with("--") => return Err(format!("unknown option {}", s)),
            s => repo = Some(s),
        }
    }
    let base = account_base(repo)?;
    let mut secrets = HostSecrets::new("");
    let mut net = SocketNet;
    let a = match key {
        Some(k) => account::whoami(&mut net, &base, &k)?,
        None => {
            let d = account::device_start(&mut net, &base, &computer_name(), &["social", "publish"])?;
            println!("To sign in, open this page in your browser and enter the code:");
            println!("  {}", d.verification_uri);
            println!("  code: {}", d.user_code);
            match ecm_host_abi::player::show_link(&d.verification_uri_complete, "Sign in to the package repository") {
                Ok(()) => println!("(A link is in your chat: click it to open the page.)"),
                Err(_) => {}
            }
            println!("Waiting for you to approve it there (Ctrl+C gives up) ...");
            let started = std::time::Instant::now();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(d.interval));
                match account::device_poll(&mut net, &base, &d.device_code)? {
                    account::DevicePoll::Granted(a) => break a,
                    account::DevicePoll::Pending if started.elapsed().as_secs() < d.expires_in => {}
                    account::DevicePoll::Denied => return Err("the sign-in was denied on the web page".into()),
                    _ => return Err("the code ran out; run pkg login again".into()),
                }
            }
        }
    };
    account::save(&mut secrets, &a)?;
    println!("Signed in to {} as {}.", base.host, a.handle);
    println!("The sign-in is kept in {}.", secrets.describe());
    Ok(())
}

fn logout(repo: Option<&str>) -> Result<(), String> {
    let base = account_base(repo)?;
    match account::logout(&mut SocketNet, &mut HostSecrets::new(""), &base) {
        Some(a) => println!("Signed out of {} ({}).", base.host, a.handle),
        None => println!("Not signed in to {}.", base.host),
    }
    Ok(())
}

fn signed_in(base: &Url) -> Option<account::Account> {
    account::load(&HostSecrets::new(""), base)
}

fn whoami(repo: Option<&str>) -> Result<(), String> {
    let base = account_base(repo)?;
    match signed_in(&base) {
        Some(a) => {
            // Ask the repository: the key may have been revoked from the account page.
            let live = account::whoami(&mut SocketNet, &base, &a.token)?;
            println!("{} at {} (may: {})", live.handle, base.host, if live.scopes.is_empty() { "nothing".into() } else { live.scopes.join(", ") });
        }
        None => println!("Not signed in to {} (pkg login). Anyone can download without an account.", base.host),
    }
    Ok(())
}

fn publish(what: &str, repo: Option<&str>) -> Result<(), String> {
    let base = account_base(repo)?;
    let bytes = if what.ends_with(".ecmpkg") {
        fs::read(what).map_err(|e| format!("cannot read {}: {}", what, e))?
    } else {
        let mut files = Vec::new();
        walk(what.trim_end_matches('/'), "", &mut files)?;
        package::build(files)?.1
    };
    let mut secrets = HostSecrets::new("");
    let a = signed_in(&base);
    let p = package::open(&bytes)?;
    println!("Publishing {} {} ({}, {} bytes) to {} ...", p.manifest.id, p.manifest.version, p.manifest.kind.name(), bytes.len(), base.host);
    let done = account::publish(&mut SocketNet, &mut secrets, &base, a.as_ref(), &bytes)?;
    println!("Published {} {} in the {} channel.", done.id, done.version, if done.channel.is_empty() { "community" } else { &done.channel });
    println!("Others install it with: pkg install {}", done.id);
    Ok(())
}

fn yank(id: &str, version: &str, repo: Option<&str>) -> Result<(), String> {
    let base = account_base(repo)?;
    account::yank(&mut SocketNet, &base, signed_in(&base).as_ref(), id, version)?;
    println!("Yanked {} {}: it is no longer listed.", id, version);
    Ok(())
}

fn mark(op: &str, id: &str, repo: Option<&str>) -> Result<(), String> {
    let base = account_base(repo)?;
    let (what, on) = match op {
        "like" => ("like", true),
        "unlike" => ("like", false),
        "favorite" => ("favorite", true),
        _ => ("favorite", false),
    };
    account::mark(&mut SocketNet, &base, signed_in(&base).as_ref(), id, what, on)?;
    println!("{}", match (what, on) {
        ("like", true) => format!("Liked {}.", id),
        ("like", false) => format!("No longer liking {}.", id),
        (_, true) => format!("{} is in your favorites.", id),
        _ => format!("{} is out of your favorites.", id),
    });
    Ok(())
}

fn mine(what: &str, repo: Option<&str>) -> Result<(), String> {
    let base = account_base(repo)?;
    let list = account::mine(&mut SocketNet, &base, signed_in(&base).as_ref(), what)?;
    if list.is_empty() {
        println!("{}", if what == "favorites" { "No favorites yet (pkg favorite <id>)." } else { "You have published nothing yet (pkg publish <dir>)." });
    }
    let c = client();
    for s in list {
        let here = match c.installed_manifest(&s.id) {
            Some(m) => format!("installed {}", m.version),
            None => "not installed".into(),
        };
        println!("{} {}  {}  [{}, {} likes, {} downloads, {}]", s.id, s.version, s.name, if s.kind.is_empty() { "app" } else { &s.kind }, s.likes, s.downloads, here);
    }
    Ok(())
}

/// `pkg online [repo]`: which repositories answer from here, and if not, why.
fn online(repo: Option<&str>) -> Result<(), String> {
    let c = client();
    let repos = match repo {
        Some(r) => vec![find_repo(&c, Some(r))?],
        None => c.repos(),
    };
    if repos.is_empty() {
        println!("No repositories. Add one with: pkg repo add <name> <url>");
    }
    let mut all = true;
    for r in repos {
        let u = Url::parse(&r.url)?;
        let state = if u.scheme == Scheme::Host {
            match ecm_host_abi::repo::supported() {
                ecm_host_abi::repo::Support::Ready => "online (through the server; no network needed)".to_string(),
                other => {
                    all = false;
                    other.message().to_string()
                }
            }
        } else {
            let o = ecm_http::sys::online(&format!("{}index.json", u.display()));
            if !o.is_online() {
                all = false;
            }
            o.explain()
        };
        println!("{}  {}  {}", r.name, r.url, state);
    }
    if !all {
        std::process::exit(1);
    }
    Ok(())
}

/// `pkg discover`: repositories found on the network, and how to keep one.
fn discover() -> Result<(), String> {
    let found = ecm_disco::sys::search(ecm_disco::types::PKG, ecm_disco::CLIENT_WAIT_MS)?;
    let repos = client().repos();
    let new = ecm_pkg::client::found_repos(&repos, &found);
    if found.is_empty() {
        println!("No repositories found on the network.");
        println!("  A repository is found when its computer runs 'disco serve &' next to 'pkg serve'.");
        return Ok(());
    }
    for f in &found {
        let url = format!("http://{}/", f.endpoint());
        match new.iter().find(|r| r.url == url) {
            Some(r) => println!("{}  {}  (pkg repo add {} {})", r.name, url, r.name, url),
            None => println!("{}  {}  (configured)", f.label(), url),
        }
    }
    if !new.is_empty() {
        println!("The App Store lists their apps already; pkg repo add keeps one (add --key <hex> to pin it).");
    }
    Ok(())
}

fn serve(dir: &str, port: u16, name: &str, identity: Option<ecm_tls::Identity>) -> Result<(), String> {
    let dir = dir.trim_end_matches('/').to_string();
    let (ix, _) = make_index(&dir);
    let json = serde_json::to_string_pretty(&ix).map_err(|e| e.to_string())?;
    write_file(&format!("{}/index.json", dir), json.as_bytes())?;
    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    if fd < 0 {
        return Err(format!("cannot open a socket ({})", fd));
    }
    socket::setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &1i32.to_le_bytes());
    if socket::bind(fd, &SockAddrIn::any(port)) != 0 || socket::listen(fd, 8) != 0 {
        socket::close(fd);
        return Err(format!("cannot listen on port {}", port));
    }
    println!("Serving {} packages from {} on port {}{}. Ctrl+T to stop.", ix.packages.len(), dir, port, if identity.is_some() { " (HTTPS)" } else { "" });
    // Found by `pkg discover` and the App Store when this computer runs `disco serve`
    // (plain HTTP only: a found repository is added as http://).
    if identity.is_none() {
        ecm_disco::sys::announce(ecm_disco::types::PKG, port, name);
    }
    for p in &ix.packages {
        println!("  {} {}{}", p.id, p.version, if p.signature.is_some() { " (signed)" } else { "" });
    }
    loop {
        let mut peer = SockAddrIn::default();
        let c = socket::accept(fd, &mut peer);
        if c == -2 {
            continue;
        }
        if c < 0 {
            break;
        }
        let sock = ecm_http::sys::Socket::from_fd(c, std::time::Duration::from_secs(10));
        let who = peer.to_string();
        match &identity {
            Some(id) => match id.accept(sock) {
                Ok(mut t) => serve_one(&mut t, &dir, &who),
                Err(e) => println!("{}: {}", who, e),
            },
            None => {
                let mut s = sock;
                serve_one(&mut s, &dir, &who)
            }
        }
    }
    socket::close(fd);
    Ok(())
}

/// A package's icon or screenshot: `GET /<file>/<member>`.
fn serve_asset(dir: &str, ix: &Index, name: &str) -> Option<Vec<u8>> {
    let (file, member) = name.split_once('/')?;
    let e = ix.packages.iter().find(|e| e.file == file)?;
    if !package::servable(e, member) {
        return None;
    }
    let bytes = fs::read(format!("{}/{}", dir, file)).ok()?;
    let p = package::open(&bytes).ok()?;
    package::member(&p, member).map(|b| b.to_vec())
}

fn serve_one(c: &mut dyn ecm_http::Stream, dir: &str, peer: &str) {
    let mut raw = Vec::new();
    let mut buf = [0u8; 2048];
    let req = loop {
        let n = c.read(&mut buf).unwrap_or(0);
        if n == 0 {
            break None;
        }
        raw.extend_from_slice(&buf[..n]);
        if let Some(r) = http::parse_request_line(&raw) {
            break Some(r);
        }
        if raw.len() > 8192 {
            break None;
        }
    };
    let reply = match req {
        Some((m, _)) if m != "GET" => http::response(405, "Method Not Allowed", "text/plain", b"GET only\n"),
        Some((_, path)) => {
            let name = path.trim_start_matches('/');
            let (ix, files) = make_index(dir);
            if name == "index.json" || name.is_empty() {
                println!("{} GET /index.json", peer);
                let json = serde_json::to_string_pretty(&ix).unwrap_or_default();
                http::response(200, "OK", "application/json", json.as_bytes())
            } else if files.iter().any(|f| f == name) || name == paths::PUBLISHER_KEY_FILE {
                println!("{} GET /{}", peer, name);
                match fs::read(format!("{}/{}", dir, name)) {
                    Ok(b) => http::response(200, "OK", "application/octet-stream", &b),
                    Err(_) => http::response(404, "Not Found", "text/plain", b"not found\n"),
                }
            } else if let Some(b) = serve_asset(dir, &ix, name) {
                println!("{} GET /{}", peer, name);
                http::response(200, "OK", "image/png", &b)
            } else {
                http::response(404, "Not Found", "text/plain", b"not found\n")
            }
        }
        None => http::response(400, "Bad Request", "text/plain", b"bad request\n"),
    };
    let _ = c.write_all(&reply);
    let _ = c.flush();
    std::thread::sleep(std::time::Duration::from_millis(50));
}
