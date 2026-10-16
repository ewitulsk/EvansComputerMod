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
use ecm_pkg::client::{self, Client, SocketNet};
use ecm_pkg::http;
use ecm_pkg::manifest;
use ecm_pkg::package::{self, Index, Trust};
use ecm_pkg::paths;
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
  open <path> [--with <id>]             open a file with its default app (or the one named)
  build <dir> [out-dir] [--sign <key>]  make <id>-<version>.ecmpkg from a folder
  keygen [file]                         make a signing key (default /etc/pkg/signing.key)
  serve <dir> [port] [--name NAME]      host <dir> as a repository (default port 8080); with
                                        disco serve on this computer it is found on the network
  discover                              repositories found on the network (disco)";

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
        ["open", path] => open(path, None),
        ["open", path, "--with", id] => open(path, Some(id)),
        ["build", dir, rest @ ..] => build(dir, rest),
        ["keygen"] => keygen(paths::SIGNING_KEY),
        ["keygen", file] => keygen(file),
        ["serve", dir, rest @ ..] => serve_args(dir, rest),
        ["discover"] => discover(),
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
        println!("No apps found.");
    }
    for a in found {
        let e = &a.entry;
        let signed = if e.signature.is_some() { "signed" } else { "unsigned" };
        println!("{} {}  {}  [{}, {}]", e.id, e.version, e.name, a.repo.name, signed);
        if !e.summary.is_empty() {
            println!("    {}", e.summary);
        }
        if !e.permissions.is_empty() {
            println!("    asks for: {}", e.permissions.join(", "));
        }
    }
    Ok(())
}

fn install(target: &str, force: bool) -> Result<(), String> {
    let mut c = client();
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
        println!("Downloading {} {} from {}...", a.entry.id, a.entry.version, a.repo.name);
        c.install_available(&a, &mut |_, _| {})?
    };
    reindex();
    let m = &report.manifest;
    println!("Installed {} {} ({}) - {}", m.name, m.version, m.id, report.trust.label());
    if !m.permissions.is_empty() {
        println!("It asks for: {} (you are asked on first use)", m.permissions.join(", "));
    }
    Ok(())
}

fn remove(id: &str, purge: bool) -> Result<(), String> {
    client().remove(id, purge)?;
    reindex();
    println!("Removed {}{}", id, if purge { " and its data" } else { "" });
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
        let r = c.install_available(a, &mut |_, _| {})?;
        println!("Installed {} {} ({}) - {}", r.manifest.name, r.manifest.version, r.manifest.id, r.trust.label());
    }
    if !ups.is_empty() {
        reindex();
    }
    println!("{}", if ups.is_empty() { "Everything is up to date.".to_string() } else { format!("{} upgraded.", ups.len()) });
    Ok(())
}

fn list() -> Result<(), String> {
    let all = client().installed();
    if all.is_empty() {
        println!("No apps installed.");
    }
    for i in all {
        println!("{} {}  {}", i.manifest.id, i.manifest.version, i.manifest.name);
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
    println!("  entry: {}", m.entry);
    if !m.open_with.is_empty() {
        println!("  opens: {}", m.open_with.join(" "));
    }
    if let Some(o) = c.origin(id) {
        println!("  origin: {} ({})", o.origin, o.trust);
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

/// `pkg serve <dir> [port] [--name NAME]`.
fn serve_args(dir: &str, rest: &[&str]) -> Result<(), String> {
    let mut port = 8080;
    let mut name = String::new();
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "--name" => {
                i += 1;
                name = rest.get(i).ok_or("--name wants a name")?.to_string();
                if !ecm_pkg::valid_repo_name(&name) {
                    return Err(format!("bad repository name '{}' (letters, digits, - and _)", name));
                }
            }
            p => port = p.parse().map_err(|_| format!("bad port {}", p))?,
        }
        i += 1;
    }
    serve(dir, port, &name)
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

fn serve(dir: &str, port: u16, name: &str) -> Result<(), String> {
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
    // Found by `pkg discover` and the App Store when this computer runs `disco serve`.
    ecm_disco::sys::announce(ecm_disco::types::PKG, port, name);
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
            } else if let Some(b) = serve_asset(dir, &ix, name) {
                println!("{} GET /{}", peer, name);
                http::response(200, "OK", "image/png", &b)
            } else {
                http::response(404, "Not Found", "text/plain", b"not found\n")
            }
        }
        None => http::response(400, "Bad Request", "text/plain", b"bad request\n"),
    };
    SocketNet::send_all(c, &reply);
    std::thread::sleep(std::time::Duration::from_millis(50));
    socket::close(c);
}
