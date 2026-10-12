//! The app manager's control channel (`app_ctl`, docs/gui/APPS.md) for the
//! store, the chooser and the Settings pane: a trait so the UI code runs
//! against the host ([`HostCtl`]) or an in-memory app manager in tests
//! ([`FakeCtl`]), and parsers for the replies.
//!
//! | Request | Reply lines |
//! | --- | --- |
//! | `list` | `<id>\t<version>\t<name>` |
//! | `info <id>` | `<key>\t<value>`: name, version, publisher, summary, permissions, open_with, trust, icon |
//! | `perm list <id>` | `<id> <permission> allow\|deny\|ask`, `<id> file <path> rw\|r` |
//! | `perm grant\|deny\|reset <id> <permission>` | `<id> <permission> <state>` |
//! | `perm revoke <id> <path>` | (a one-file grant removed) |
//! | `handlers <path>` | `<id>\t<name>\tdefault\|-` |
//! | `defaults` | `<.ext>\t<id>` |
//! | `default <.ext> <id\|->` | |
//! | `launch <id> [args]` | `<pid>` |
//! | `openwith <path> <id>` | `<id> <pid>` |

use std::collections::BTreeMap;

/// Status codes (`ecm_host_abi::apps`).
pub const OK: i32 = 0;
pub const NOT_FOUND: i32 = -2;
pub const DENIED: i32 = -13;
pub const INVALID: i32 = -22;
pub const CHOOSE: i32 = -35;

/// One `app_ctl` request: `(status, reply)`, `None` without an app manager.
pub trait AppCtl {
    fn ctl(&mut self, words: &[&str]) -> Option<(i32, String)>;

    /// `Ok(reply)` for status 0, otherwise the reply (or why there was none) as the error.
    fn call(&mut self, words: &[&str]) -> Result<String, String> {
        match self.ctl(words) {
            Some((OK, r)) => Ok(r),
            Some((s, r)) => Err(if r.trim().is_empty() { format!("{} failed ({})", words.first().unwrap_or(&""), s) } else { r.trim().to_string() }),
            None => Err("no app manager on this computer".into()),
        }
    }
}

/// The computer's app manager.
pub struct HostCtl;

impl AppCtl for HostCtl {
    fn ctl(&mut self, words: &[&str]) -> Option<(i32, String)> {
        ecm_host_abi::apps::ctl(words).map(|(s, r)| (s, String::from_utf8_lossy(&r).into_owned()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledApp {
    pub id: String,
    pub version: String,
    pub name: String,
}

pub fn parse_list(s: &str) -> Vec<InstalledApp> {
    s.lines()
        .filter_map(|l| {
            let mut p = l.splitn(3, '\t');
            Some(InstalledApp { id: p.next()?.to_string(), version: p.next()?.to_string(), name: p.next().unwrap_or("").to_string() })
        })
        .filter(|a| !a.id.is_empty())
        .collect()
}

/// A remembered (or not yet given) permission answer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PermState {
    Allow,
    Deny,
    /// Declared, not decided: the app is asked on first use.
    Ask,
}

impl PermState {
    pub fn word(self) -> &'static str {
        match self {
            PermState::Allow => "allow",
            PermState::Deny => "deny",
            PermState::Ask => "ask",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermLine {
    pub app: String,
    pub permission: String,
    pub state: PermState,
}

/// A one-file grant (file picker, open with).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileGrant {
    pub app: String,
    pub path: String,
    pub write: bool,
}

/// `perm list [id]` lines.
pub fn parse_perm_list(s: &str) -> (Vec<PermLine>, Vec<FileGrant>) {
    let mut perms = Vec::new();
    let mut files = Vec::new();
    for l in s.lines() {
        let Some((app, rest)) = l.split_once(' ') else { continue };
        if let Some(f) = rest.strip_prefix("file ") {
            if let Some((path, mode)) = f.rsplit_once(' ') {
                files.push(FileGrant { app: app.into(), path: path.into(), write: mode == "rw" });
            }
            continue;
        }
        let Some((perm, state)) = rest.rsplit_once(' ') else { continue };
        let state = match state {
            "allow" => PermState::Allow,
            "deny" => PermState::Deny,
            "ask" => PermState::Ask,
            _ => continue,
        };
        perms.push(PermLine { app: app.into(), permission: perm.into(), state });
    }
    (perms, files)
}

/// An app that opens a file type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handler {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

pub fn parse_handlers(s: &str) -> Vec<Handler> {
    s.lines()
        .filter_map(|l| {
            let mut p = l.split('\t');
            let id = p.next()?.to_string();
            let name = p.next().filter(|n| !n.is_empty()).unwrap_or(&id).to_string();
            let is_default = p.next() == Some("default");
            (!id.is_empty()).then_some(Handler { id, name, is_default })
        })
        .collect()
}

/// `defaults`: extension -> app id.
pub fn parse_defaults(s: &str) -> Vec<(String, String)> {
    s.lines().filter_map(|l| l.split_once('\t')).map(|(e, i)| (e.to_string(), i.to_string())).collect()
}

/// `info <id>`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub summary: String,
    pub permissions: Vec<String>,
    pub open_with: Vec<String>,
    pub trust: String,
    pub icon: String,
}

pub fn parse_info(id: &str, s: &str) -> AppInfo {
    let mut i = AppInfo { id: id.into(), ..Default::default() };
    for l in s.lines() {
        let Some((k, v)) = l.split_once('\t') else { continue };
        let words = || v.split_whitespace().map(String::from).collect::<Vec<_>>();
        match k {
            "name" => i.name = v.into(),
            "version" => i.version = v.into(),
            "publisher" => i.publisher = v.into(),
            "summary" => i.summary = v.into(),
            "permissions" => i.permissions = words(),
            "open_with" => i.open_with = words(),
            "trust" => i.trust = v.into(),
            "icon" => i.icon = v.into(),
            _ => {}
        }
    }
    i
}

/// The extension of a path as `open_with` lists it (`.txt`), lower case.
pub fn extension(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let dot = name.rfind('.')?;
    (dot > 0).then(|| name[dot..].to_lowercase())
}

/// An app manager in memory with the same replies as the Java one (tests,
/// golden sessions). Commands it got are logged.
#[derive(Clone, Debug, Default)]
pub struct FakeCtl {
    pub apps: BTreeMap<String, AppInfo>,
    /// (app, permission) -> answer.
    pub decisions: BTreeMap<(String, String), bool>,
    pub grants: Vec<FileGrant>,
    pub defaults: BTreeMap<String, String>,
    pub log: Vec<String>,
    pub next_pid: i32,
    /// Answer every request with `None` (no app manager).
    pub absent: bool,
}

impl FakeCtl {
    pub fn add(&mut self, id: &str, name: &str, version: &str, permissions: &[&str], open_with: &[&str]) {
        self.apps.insert(
            id.into(),
            AppInfo {
                id: id.into(),
                name: name.into(),
                version: version.into(),
                permissions: permissions.iter().map(|s| s.to_string()).collect(),
                open_with: open_with.iter().map(|s| s.to_string()).collect(),
                trust: "unverified publisher".into(),
                ..Default::default()
            },
        );
    }

    fn handlers(&self, path: &str) -> Vec<&AppInfo> {
        let Some(ext) = extension(path) else { return Vec::new() };
        self.apps.values().filter(|a| a.open_with.contains(&ext)).collect()
    }
}

impl AppCtl for FakeCtl {
    fn ctl(&mut self, words: &[&str]) -> Option<(i32, String)> {
        self.log.push(words.join(" "));
        if self.absent {
            return None;
        }
        let ok = |s: String| Some((OK, s));
        match words {
            ["reindex"] => ok(format!("{} apps\n", self.apps.len())),
            ["list"] => ok(self.apps.values().map(|a| format!("{}\t{}\t{}\n", a.id, a.version, a.name)).collect()),
            ["info", id] => match self.apps.get(*id) {
                Some(a) => ok(format!(
                    "name\t{}\nversion\t{}\npublisher\t{}\nsummary\t{}\npermissions\t{}\nopen_with\t{}\ntrust\t{}\nicon\t{}\n",
                    a.name,
                    a.version,
                    a.publisher,
                    a.summary,
                    a.permissions.join(" "),
                    a.open_with.join(" "),
                    a.trust,
                    a.icon
                )),
                None => Some((NOT_FOUND, format!("{id} is not installed"))),
            },
            ["perm", "list", id] => {
                let Some(a) = self.apps.get(*id) else { return Some((NOT_FOUND, format!("{id} is not installed"))) };
                let mut s = String::new();
                for p in &a.permissions {
                    let st = match self.decisions.get(&(id.to_string(), p.clone())) {
                        Some(true) => "allow",
                        Some(false) => "deny",
                        None => "ask",
                    };
                    s.push_str(&format!("{id} {p} {st}\n"));
                }
                for g in self.grants.iter().filter(|g| g.app == *id) {
                    s.push_str(&format!("{id} file {} {}\n", g.path, if g.write { "rw" } else { "r" }));
                }
                ok(s)
            }
            ["perm", op @ ("grant" | "deny" | "reset"), id, p] => {
                let Some(a) = self.apps.get(*id) else { return Some((NOT_FOUND, format!("{id} is not installed"))) };
                if *op != "reset" && !a.permissions.iter().any(|x| x == p) {
                    return Some((INVALID, format!("{id} does not ask for {p}")));
                }
                let key = (id.to_string(), p.to_string());
                let st = match *op {
                    "grant" => {
                        self.decisions.insert(key, true);
                        "allow"
                    }
                    "deny" => {
                        self.decisions.insert(key, false);
                        "deny"
                    }
                    _ => {
                        self.decisions.remove(&key);
                        "ask"
                    }
                };
                ok(format!("{id} {p} {st}"))
            }
            ["perm", "revoke", id, path] => {
                let n = self.grants.len();
                self.grants.retain(|g| !(g.app == *id && g.path == *path));
                if self.grants.len() == n {
                    Some((NOT_FOUND, "no such grant".into()))
                } else {
                    ok(String::new())
                }
            }
            ["handlers", path] => {
                let d = extension(path).and_then(|e| self.defaults.get(&e).cloned());
                ok(self
                    .handlers(path)
                    .iter()
                    .map(|a| format!("{}\t{}\t{}\n", a.id, a.name, if d.as_deref() == Some(a.id.as_str()) { "default" } else { "-" }))
                    .collect())
            }
            ["defaults"] => ok(self.defaults.iter().map(|(e, i)| format!("{e}\t{i}\n")).collect()),
            ["default", ext, id] => {
                if *id == "-" {
                    self.defaults.remove(*ext);
                } else {
                    self.defaults.insert(ext.to_string(), id.to_string());
                }
                ok(String::new())
            }
            ["launch", id, ..] => {
                if !self.apps.contains_key(*id) {
                    return Some((NOT_FOUND, format!("{id} is not installed")));
                }
                self.next_pid += 1;
                ok(format!("{}", 100 + self.next_pid))
            }
            ["openwith", path, id] => {
                if !self.apps.contains_key(*id) {
                    return Some((NOT_FOUND, format!("{id} is not installed")));
                }
                self.grants.push(FileGrant { app: id.to_string(), path: path.to_string(), write: true });
                self.next_pid += 1;
                ok(format!("{id} {}", 100 + self.next_pid))
            }
            _ => Some((INVALID, format!("unknown request '{}'", words.first().unwrap_or(&"")))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsers_read_the_java_replies() {
        assert_eq!(
            parse_list("dev.a\t1.0.0\tA\ndev.b\t2.0.0\tB b\n"),
            vec![
                InstalledApp { id: "dev.a".into(), version: "1.0.0".into(), name: "A".into() },
                InstalledApp { id: "dev.b".into(), version: "2.0.0".into(), name: "B b".into() }
            ]
        );
        let (p, f) = parse_perm_list("dev.a network allow\ndev.a radio ask\ndev.a file /home/my notes.txt rw\ngarbage\n");
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].state, PermState::Allow);
        assert_eq!(p[1], PermLine { app: "dev.a".into(), permission: "radio".into(), state: PermState::Ask });
        assert_eq!(f, vec![FileGrant { app: "dev.a".into(), path: "/home/my notes.txt".into(), write: true }]);
        let h = parse_handlers("dev.a\tEditor\tdefault\ndev.b\t\t-\n");
        assert!(h[0].is_default && !h[1].is_default);
        assert_eq!(h[1].name, "dev.b");
        assert_eq!(parse_defaults(".txt\tdev.a\n"), vec![(".txt".into(), "dev.a".into())]);
        let i = parse_info("dev.a", "name\tA\npermissions\tnetwork files:documents\nopen_with\t.txt .md\n");
        assert_eq!(i.permissions, vec!["network", "files:documents"]);
        assert_eq!(i.open_with, vec![".txt", ".md"]);
        assert_eq!(extension("/home/A.TXT").as_deref(), Some(".txt"));
        assert_eq!(extension("/home/.profile"), None);
        assert_eq!(extension("/home/noext"), None);
    }

    #[test]
    fn fake_answers_like_the_app_manager() {
        let mut f = FakeCtl::default();
        f.add("dev.a", "A", "1.0.0", &["network"], &[".txt"]);
        f.add("dev.b", "B", "1.0.0", &[], &[".txt", ".md"]);
        assert_eq!(parse_handlers(&f.call(&["handlers", "/home/x.txt"]).unwrap()).len(), 2);
        f.call(&["default", ".txt", "dev.b"]).unwrap();
        let h = parse_handlers(&f.call(&["handlers", "/home/x.txt"]).unwrap());
        assert!(h.iter().any(|h| h.id == "dev.b" && h.is_default));
        assert!(f.call(&["perm", "grant", "dev.a", "radio"]).is_err());
        f.call(&["perm", "grant", "dev.a", "network"]).unwrap();
        assert_eq!(parse_perm_list(&f.call(&["perm", "list", "dev.a"]).unwrap()).0[0].state, PermState::Allow);
        f.call(&["openwith", "/home/x.txt", "dev.a"]).unwrap();
        assert_eq!(parse_perm_list(&f.call(&["perm", "list", "dev.a"]).unwrap()).1.len(), 1);
        f.call(&["perm", "revoke", "dev.a", "/home/x.txt"]).unwrap();
        assert!(f.call(&["perm", "revoke", "dev.a", "/home/x.txt"]).is_err());
        f.absent = true;
        assert_eq!(f.call(&["list"]).unwrap_err(), "no app manager on this computer");
    }
}
