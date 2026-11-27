//! `ecm_pkg`: the formats behind `pkg` (docs/gui/APPS.md), host-testable.
//!
//! - [`toml_lite`]: the TOML subset of `app.toml`
//! - [`manifest`]: the `app.toml` schema and validator
//! - [`tar`]: `.ecmpkg` archives (ustar)
//! - [`package`]: packages, `index.json`, signatures and trust
//! - [`sign`]: ed25519 keys and signatures as hex
//! - [`http`]: URLs and HTTP/1.0 messages
//! - [`Repo`]: the `/etc/pkg/repos` list
//! - [`client`]: the repository client (catalog, install, update, remove) shared by `pkg` and the App Store
//! - [`account`]: signing in to a repository with accounts; publishing, likes and favorites
//! - [`mirror`]: copying a repository (or a chosen part of it) into a folder to serve

pub mod account;
pub mod client;
pub mod mirror;
pub mod http;
pub mod manifest;
pub mod package;
pub mod sign;
pub mod tar;
pub mod toml_lite;

/// Where `pkg` keeps its state on a computer.
pub mod paths {
    pub const REPOS: &str = "/etc/pkg/repos";
    pub const SIGNING_KEY: &str = "/etc/pkg/signing.key";
    pub const CACHE_DIR: &str = "/var/lib/pkg";
    pub const APPS_DIR: &str = "/apps";
    pub const APP_DATA_DIR: &str = "/var/apps";
    /// Written into `/apps/<id>/` on install: where it came from and its trust.
    pub const ORIGIN_FILE: &str = ".pkg-origin";
    /// Published next to index.json by `pkg build --sign`.
    pub const PUBLISHER_KEY_FILE: &str = "publisher.pub";
    /// What is installed besides apps (programs, libraries, source): `<id>/` with
    /// the unpacked package, `.pkg-origin` and `.pkg-files` (what it put elsewhere).
    pub const DB_DIR: &str = "/var/lib/pkg/db";
    pub const FILES_FILE: &str = ".pkg-files";
    /// Where a program's commands go (the shell looks here before the built-in ones).
    pub const BIN_DIR: &str = "/bin";
    /// Built-in commands; a package may not take one of their names.
    pub const SYSTEM_BIN_DIR: &str = "/server-bin";
    /// Where `import` (Python) and `require` (Lua) find installed libraries.
    pub const PYTHON_LIB_DIR: &str = "/lib/python";
    pub const LUA_LIB_DIR: &str = "/lib/lua";
    /// Where source packages are unpacked: `/home/src/<id>/`.
    pub const SOURCE_DIR: &str = "/home/src";
    /// The repository sign-in when the computer has no secret store (docs/REPO.md).
    pub const ACCOUNT_FILE: &str = "/etc/pkg/account";
}

/// One configured repository: `<name> <url> [<pinned key hex>]` per line.
#[derive(Debug, Clone, PartialEq)]
pub struct Repo {
    pub name: String,
    pub url: String,
    pub key: Option<String>,
}

pub fn valid_repo_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 32 && n.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

/// A repository name from a URL's host: `http://10.0.30.1:8080/` -> `10-0-30-1`.
pub fn repo_name_for(url: &str) -> String {
    let host = url.split("://").nth(1).unwrap_or(url).split(['/', ':']).next().unwrap_or("");
    let name: String = host.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).take(32).collect();
    if name.trim_matches('-').is_empty() {
        "repo".into()
    } else {
        name
    }
}

pub fn parse_repos(text: &str) -> Vec<Repo> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            let name = p.next()?.to_string();
            let url = p.next()?.to_string();
            let key = p.next().map(str::to_string);
            valid_repo_name(&name).then_some(Repo { name, url, key })
        })
        .collect()
}

pub fn format_repos(repos: &[Repo]) -> String {
    let mut s = String::from("# pkg repositories: <name> <url> [<pinned ed25519 key>]\n");
    for r in repos {
        s.push_str(&r.name);
        s.push(' ');
        s.push_str(&r.url);
        if let Some(k) = &r.key {
            s.push(' ');
            s.push_str(k);
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repos_round_trip() {
        let r = vec![
            Repo { name: "a".into(), url: "http://10.0.7.1:8080/".into(), key: None },
            Repo { name: "b-2".into(), url: "http://x/".into(), key: Some("ab".repeat(32)) },
        ];
        assert_eq!(parse_repos(&format_repos(&r)), r);
        assert!(parse_repos("bad! http://x/").is_empty());
    }
}
