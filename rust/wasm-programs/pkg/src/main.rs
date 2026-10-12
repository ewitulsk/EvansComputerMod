//! `pkg`: the ECM package manager (docs/gui/APPS.md).
//!
//! Repositories are plain HTTP servers with an `index.json` and `.ecmpkg` files;
//! `pkg serve` is one. Installed apps live in `/apps/<id>/`, their data in
//! `/var/apps/<id>/`. After installing or removing, `pkg` asks the app manager to
//! reindex through `app_ctl`, which also launches apps (`pkg run`) and manages
//! permissions (`pkg perm`).

use ecm_host_abi::apps;
use ecm_host_abi::socket::{self, SockAddrIn, AF_INET, SOCK_STREAM, SOL_SOCKET, SO_REUSEADDR};
use ecm_pkg::http::{self, Url};
use ecm_pkg::manifest::{self, Manifest};
use ecm_pkg::package::{self, Index, IndexEntry, Trust};
use ecm_pkg::{paths, Repo};
use std::fs;
use std::path::Path;

const USAGE: &str = "\
usage: pkg <command> [args]
  repo add <name> <url> [--key <hex>]   add a repository (a pinned key makes signatures required)
  repo list | repo remove <name>
  update                                refresh indexes, upgrade installed apps
  search [term]                         find apps in the repositories
  install <id> | <file.ecmpkg>          install from a repository or a file
  remove <id> [--purge]                 uninstall (--purge also deletes /var/apps/<id>)
  list                                  installed apps
  info <id>                             manifest, origin and permissions
  run <id> [args]                       launch an app and show its output
  perm list [id] | perm grant|deny|reset <id> <permission>
  open <path>                           open a file with its default app
  build <dir> [out-dir] [--sign <key>]  make <id>-<version>.ecmpkg from a folder
  keygen [file]                         make a signing key (default /etc/pkg/signing.key)
  serve <dir> [port]                    host <dir> as a repository (default port 8080)";

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
        ["install", target, rest @ ..] => install(target, rest.contains(&"--force")),
        ["remove", id, rest @ ..] => remove(id, rest.contains(&"--purge")),
        ["list"] => list(),
        ["info", id] => info(id),
        ["run", id, rest @ ..] => run(id, rest),
        ["perm"] | ["perm", "list"] => perm(&["perm", "list"]),
        ["perm", "list", id] => perm(&["perm", "list", id]),
        ["perm", op @ ("grant" | "deny" | "reset"), id, p] => perm(&["perm", op, id, p]),
        ["open", path] => open(path),
        ["build", dir, rest @ ..] => build(dir, rest),
        ["keygen"] => keygen(paths::SIGNING_KEY),
        ["keygen", file] => keygen(file),
        ["serve", dir] => serve(dir, 8080),
        ["serve", dir, port] => match port.parse() {
            Ok(p) => serve(dir, p),
            Err(_) => Err(format!("bad port {}", port)),
        },
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

/// Delete a directory tree (files, then the emptied directories).
fn remove_tree(p: &str) -> Result<(), String> {
    let Ok(rd) = fs::read_dir(p) else { return Ok(()) };
    for e in rd.flatten() {
        let child = format!("{}/{}", p, e.file_name().to_string_lossy());
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            remove_tree(&child)?;
        } else {
            fs::remove_file(&child).map_err(|e| format!("cannot delete {}: {}", child, e))?;
        }
    }
    fs::remove_dir(p).map_err(|e| format!("cannot delete {}: {}", p, e))
}

fn installed_manifest(id: &str) -> Option<Manifest> {
    let text = read_text(&format!("{}/{}/app.toml", paths::APPS_DIR, id))?;
    manifest::parse(&text).ok()
}

fn installed() -> Vec<Manifest> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(paths::APPS_DIR) {
        for e in rd.flatten() {
            if let Some(m) = installed_manifest(&e.file_name().to_string_lossy()) {
                out.push(m);
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

// ------------------------------------------------------------------ app manager

fn ctl(words: &[&str]) -> Result<(i32, String), String> {
    match apps::ctl(words) {
        Some((s, r)) => Ok((s, String::from_utf8_lossy(&r).into_owned())),
        None => Err("no app manager on this computer".into()),
    }
}

fn reindex() {
    match ctl(&["reindex"]) {
        Ok((0, _)) => {}
        Ok((s, r)) => println!("pkg: app manager reindex failed ({}): {}", s, r.trim()),
        Err(e) => println!("pkg: {} (installed files only)", e),
    }
}

// ------------------------------------------------------------------ HTTP client

fn connect(url: &Url) -> Result<i32, String> {
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

fn send_all(fd: i32, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        let n = socket::send(fd, data, 0);
        if n <= 0 {
            return false;
        }
        data = &data[n as usize..];
    }
    true
}

fn http_get(url: &Url, file: &str) -> Result<Vec<u8>, String> {
    let fd = connect(url)?;
    let path = url.join(file);
    if !send_all(fd, &http::get_request(&url.host, &path)) {
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
        if http::response_complete(&raw) {
            break;
        }
        if raw.len() > 16 * 1024 * 1024 {
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

// ------------------------------------------------------------------ repositories

fn repos() -> Vec<Repo> {
    ecm_pkg::parse_repos(&read_text(paths::REPOS).unwrap_or_default())
}

fn save_repos(r: &[Repo]) -> Result<(), String> {
    write_file(paths::REPOS, ecm_pkg::format_repos(r).as_bytes())
}

fn cache_path(repo: &str) -> String {
    format!("{}/{}.index.json", paths::CACHE_DIR, repo)
}

fn fetch_index(r: &Repo) -> Result<Index, String> {
    let url = Url::parse(&r.url)?;
    let body = http_get(&url, "index.json")?;
    let text = String::from_utf8(body).map_err(|_| "index.json is not UTF-8".to_string())?;
    let ix = package::parse_index(&text)?;
    write_file(&cache_path(&r.name), text.as_bytes())?;
    Ok(ix)
}

/// Cached index of a repository, fetched if there is none yet.
fn index_of(r: &Repo) -> Result<Index, String> {
    if let Some(text) = read_text(&cache_path(&r.name)) {
        if let Ok(ix) = package::parse_index(&text) {
            return Ok(ix);
        }
    }
    fetch_index(r)
}

fn repo_add(name: &str, url: &str, rest: &[&str]) -> Result<(), String> {
    if !ecm_pkg::valid_repo_name(name) {
        return Err(format!("bad repository name '{}' (letters, digits, - and _)", name));
    }
    let u = Url::parse(url)?;
    let key = match rest {
        [] => None,
        ["--key", k] => {
            if package::unhex(k).map(|b| b.len()) != Some(32) {
                return Err("--key wants a 64-digit hex ed25519 public key".into());
            }
            Some(k.to_lowercase())
        }
        _ => return Err("usage: pkg repo add <name> <url> [--key <hex>]".into()),
    };
    let mut all = repos();
    all.retain(|r| r.name != name);
    let r = Repo { name: name.into(), url: u.display(), key };
    all.push(r.clone());
    save_repos(&all)?;
    match fetch_index(&r) {
        Ok(ix) => println!("Added {} ({}): {} packages", name, r.url, ix.packages.len()),
        Err(e) => println!("Added {} ({}); index not fetched yet: {}", name, r.url, e),
    }
    Ok(())
}

fn repo_list() -> Result<(), String> {
    let all = repos();
    if all.is_empty() {
        println!("No repositories. Add one with: pkg repo add <name> http://<host>:8080/");
    }
    for r in all {
        println!("{}  {}{}", r.name, r.url, if r.key.is_some() { "  (pinned key)" } else { "" });
    }
    Ok(())
}

fn repo_remove(name: &str) -> Result<(), String> {
    let mut all = repos();
    let n = all.len();
    all.retain(|r| r.name != name);
    if all.len() == n {
        return Err(format!("no repository named {}", name));
    }
    save_repos(&all)?;
    let _ = fs::remove_file(cache_path(name));
    println!("Removed {}", name);
    Ok(())
}

/// The newest entry for `id` across repositories (earlier repositories win ties).
fn find(id: &str, refresh: bool) -> Result<(Repo, Index, IndexEntry), String> {
    let mut best: Option<(Repo, Index, IndexEntry)> = None;
    let all = repos();
    if all.is_empty() {
        return Err("no repositories; add one with pkg repo add".into());
    }
    for r in all {
        let ix = match if refresh { fetch_index(&r) } else { index_of(&r) } {
            Ok(ix) => ix,
            Err(e) => {
                println!("pkg: {}: {}", r.name, e);
                continue;
            }
        };
        let cand = package::latest(&ix).into_iter().find(|e| e.id == id).cloned();
        if let Some(e) = cand {
            let better = match &best {
                None => true,
                Some((_, _, b)) => manifest::compare_versions(&e.version, &b.version).is_gt(),
            };
            if better {
                best = Some((r.clone(), ix.clone(), e));
            }
        }
    }
    best.ok_or_else(|| format!("{} is not in any repository", id))
}

fn search(term: &str) -> Result<(), String> {
    let t = term.to_lowercase();
    let mut found = 0;
    for r in repos() {
        let ix = match index_of(&r) {
            Ok(ix) => ix,
            Err(e) => {
                println!("pkg: {}: {}", r.name, e);
                continue;
            }
        };
        for e in package::latest(&ix) {
            let hay = format!("{} {} {}", e.id, e.name, e.summary).to_lowercase();
            if !hay.contains(&t) {
                continue;
            }
            found += 1;
            let signed = if e.signature.is_some() { "signed" } else { "unsigned" };
            println!("{} {}  {}  [{}, {}]", e.id, e.version, e.name, r.name, signed);
            if !e.summary.is_empty() {
                println!("    {}", e.summary);
            }
            if !e.permissions.is_empty() {
                println!("    asks for: {}", e.permissions.join(", "));
            }
        }
    }
    if found == 0 {
        println!("No apps found.");
    }
    Ok(())
}

fn install(target: &str, force: bool) -> Result<(), String> {
    let (bytes, origin, trust) = if target.ends_with(".ecmpkg") {
        let b = fs::read(target).map_err(|e| format!("cannot read {}: {}", target, e))?;
        (b, format!("file {}", target), Trust::Unsigned)
    } else {
        let (repo, ix, entry) = find(target, false)?;
        if let Some(m) = installed_manifest(target) {
            if !force && manifest::compare_versions(&m.version, &entry.version).is_ge() {
                println!("{} {} is already installed.", m.id, m.version);
                return Ok(());
            }
        }
        let url = Url::parse(&repo.url)?;
        println!("Downloading {} {} from {}...", entry.id, entry.version, repo.name);
        let b = http_get(&url, &entry.file)?;
        let trust = package::verify(&entry, ix.publisher_key.as_deref(), repo.key.as_deref(), &b)?;
        (b, format!("repo {} {}", repo.name, repo.url), trust)
    };
    let p = package::open(&bytes)?;
    if !target.ends_with(".ecmpkg") && p.manifest.id != target {
        return Err(format!("the package is {}, not {}", p.manifest.id, target));
    }
    let m = &p.manifest;
    let dir = format!("{}/{}", paths::APPS_DIR, m.id);
    remove_tree(&dir)?;
    for e in &p.entries {
        let path = format!("{}/{}", dir, e.path);
        if e.is_dir {
            fs::create_dir_all(&path).map_err(|e| format!("cannot create {}: {}", path, e))?;
        } else {
            write_file(&path, &e.data)?;
        }
    }
    let origin_json = format!(
        "{{\"origin\":{:?},\"version\":{:?},\"sha256\":{:?},\"trust\":{:?}}}\n",
        origin,
        m.version,
        p.sha256,
        trust.label()
    );
    write_file(&format!("{}/{}", dir, paths::ORIGIN_FILE), origin_json.as_bytes())?;
    let _ = fs::create_dir_all(format!("{}/{}", paths::APP_DATA_DIR, m.id));
    reindex();
    println!("Installed {} {} ({}) - {}", m.name, m.version, m.id, trust.label());
    if !m.permissions.is_empty() {
        println!("It asks for: {} (you are asked on first use)", m.permissions.join(", "));
    }
    Ok(())
}

fn remove(id: &str, purge: bool) -> Result<(), String> {
    let dir = format!("{}/{}", paths::APPS_DIR, id);
    if installed_manifest(id).is_none() && fs::metadata(&dir).is_err() {
        return Err(format!("{} is not installed", id));
    }
    remove_tree(&dir)?;
    if purge {
        remove_tree(&format!("{}/{}", paths::APP_DATA_DIR, id))?;
    }
    reindex();
    println!("Removed {}{}", id, if purge { " and its data" } else { "" });
    Ok(())
}

fn update() -> Result<(), String> {
    for r in repos() {
        match fetch_index(&r) {
            Ok(ix) => println!("{}: {} packages", r.name, ix.packages.len()),
            Err(e) => println!("pkg: {}: {}", r.name, e),
        }
    }
    let mut n = 0;
    for m in installed() {
        if let Ok((_, _, e)) = find(&m.id, false) {
            if manifest::compare_versions(&e.version, &m.version).is_gt() {
                println!("Upgrading {} {} -> {}", m.id, m.version, e.version);
                install(&m.id, false)?;
                n += 1;
            }
        }
    }
    println!("{}", if n == 0 { "Everything is up to date.".to_string() } else { format!("{} upgraded.", n) });
    Ok(())
}

fn list() -> Result<(), String> {
    let all = installed();
    if all.is_empty() {
        println!("No apps installed.");
    }
    for m in all {
        println!("{} {}  {}", m.id, m.version, m.name);
    }
    Ok(())
}

fn info(id: &str) -> Result<(), String> {
    let m = installed_manifest(id).ok_or_else(|| format!("{} is not installed", id))?;
    println!("{} {} ({})", m.name, m.version, m.id);
    if !m.summary.is_empty() {
        println!("  {}", m.summary);
    }
    println!("  entry: {}", m.entry);
    if !m.open_with.is_empty() {
        println!("  opens: {}", m.open_with.join(" "));
    }
    if let Some(o) = read_text(&format!("{}/{}/{}", paths::APPS_DIR, id, paths::ORIGIN_FILE)) {
        println!("  origin: {}", o.trim());
    }
    println!("  asks for: {}", if m.permissions.is_empty() { "nothing".to_string() } else { m.permissions.join(", ") });
    if let Ok((0, r)) = ctl(&["perm", "list", id]) {
        for l in r.lines() {
            println!("  {}", l);
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
            for l in lines {
                if let Some(p) = l.strip_prefix("denied ") {
                    println!("pkg: {} was denied {} (grant it with: pkg perm grant {} {})", id, p, id, p);
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

fn open(path: &str) -> Result<(), String> {
    let (s, r) = ctl(&["open", path])?;
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

/// index.json for every valid package in `dir` (signatures from `<file>.sig`).
fn make_index(dir: &str) -> (Index, Vec<String>) {
    let mut packages = Vec::new();
    let mut files = Vec::new();
    let publisher_key = read_text(&format!("{}/{}", dir, paths::PUBLISHER_KEY_FILE)).map(|s| s.trim().to_string());
    if let Ok(rd) = fs::read_dir(dir) {
        let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        for n in names {
            if !n.ends_with(".ecmpkg") {
                continue;
            }
            let Ok(bytes) = fs::read(format!("{}/{}", dir, n)) else { continue };
            match package::open(&bytes) {
                Ok(p) => {
                    let sig = read_text(&format!("{}/{}.sig", dir, n)).map(|s| s.trim().to_string());
                    packages.push(package::index_entry(&p, &n, bytes.len() as u64, sig));
                    files.push(n);
                }
                Err(e) => println!("pkg serve: skipping {}: {}", n, e),
            }
        }
    }
    let ix = Index { format: package::INDEX_FORMAT.into(), name: dir.to_string(), publisher_key, packages };
    (ix, files)
}

fn serve(dir: &str, port: u16) -> Result<(), String> {
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
    println!("Serving {} packages from {} on port {}. Ctrl+T to stop.", ix.packages.len(), dir, port);
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
        serve_one(c, &dir, &peer.to_string());
    }
    socket::close(fd);
    Ok(())
}

fn serve_one(c: i32, dir: &str, peer: &str) {
    let mut raw = Vec::new();
    let mut buf = [0u8; 2048];
    let req = loop {
        let n = socket::recv(c, &mut buf, 0);
        if n <= 0 {
            break None;
        }
        raw.extend_from_slice(&buf[..n as usize]);
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
            } else {
                http::response(404, "Not Found", "text/plain", b"not found\n")
            }
        }
        None => http::response(400, "Bad Request", "text/plain", b"bad request\n"),
    };
    send_all(c, &reply);
    std::thread::sleep(std::time::Duration::from_millis(50));
    socket::close(c);
}
