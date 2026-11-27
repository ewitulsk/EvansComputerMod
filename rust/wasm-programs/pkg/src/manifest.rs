//! `app.toml`: the app manifest (docs/gui/APPS.md, "app.toml").
//!
//! One schema, two implementations: this one (used by `pkg`) and
//! computer/apps/AppManifest.java (the app manager). Both run the vectors in
//! abi/app-manifest-vectors, where `bad_<field>_*.toml` must fail with error field
//! `<field>`.

use crate::toml_lite::{self, Table, Value};
use std::cmp::Ordering;

pub const MAX_ID: usize = 128;
pub const MAX_NAME: usize = 64;
pub const MAX_SUMMARY: usize = 256;
pub const MAX_PUBLISHER: usize = 64;
pub const SIZES: [&str; 3] = ["compact", "medium", "wide"];
pub const BACKGROUND_MODES: [&str; 5] = ["audio", "call", "radio", "transfer", "navigation"];
/// Plain permission names (besides `peripherals:<type>` and `background:<mode>`).
pub const PERMISSIONS: [&str; 14] = [
    "network",
    "radio",
    "cellular",
    "controller",
    "microphone",
    "location",
    "notifications",
    "files:documents",
    "files:all",
    "packages",
    "mixer",
    "contacts",
    "phone",
    "sms",
];
const KNOWN_KEYS: [&str; 16] = [
    "id", "name", "version", "entry", "summary", "publisher", "sizes", "permissions", "background", "open_with",
    "hotkeys", "icon", "kind", "bin", "provides", "depends",
];

/// What a package is (`kind`, default `app`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A launchable app in `/apps/<id>/` (needs `entry`).
    App,
    /// Commands for the shell: each `bin` file is installed as `/bin/<name>.wasm`.
    Program,
    /// Python and Lua modules: `python/` and `lua/` are installed into `/lib/python` and `/lib/lua`.
    Library,
    /// Plain code and files, unpacked into `/home/src/<id>/`.
    Source,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::App, Kind::Program, Kind::Library, Kind::Source];

    pub fn name(self) -> &'static str {
        match self {
            Kind::App => "app",
            Kind::Program => "program",
            Kind::Library => "library",
            Kind::Source => "source",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == s)
    }

    /// Runs with the whole computer in reach (no app sandbox): the store warns before installing.
    pub fn unconfined(self) -> bool {
        matches!(self, Kind::Program | Kind::Library)
    }
}

/// How a dependency constrains the version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepOp {
    Any,
    AtLeast,
    Exactly,
}

/// One `depends` entry: `id`, `id >= 1.2.0` or `id = 1.2.0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dep {
    pub id: String,
    pub op: DepOp,
    /// Empty for [`DepOp::Any`].
    pub version: String,
}

impl Dep {
    pub fn parse(s: &str) -> Option<Dep> {
        let mut parts = s.split_whitespace();
        let id = parts.next()?;
        if !valid_id(id) {
            return None;
        }
        let dep = match (parts.next(), parts.next(), parts.next()) {
            (None, _, _) => Dep { id: id.into(), op: DepOp::Any, version: String::new() },
            (Some(op), Some(v), None) if valid_version(v) => {
                let op = match op {
                    ">=" => DepOp::AtLeast,
                    "=" => DepOp::Exactly,
                    _ => return None,
                };
                Dep { id: id.into(), op, version: v.into() }
            }
            _ => return None,
        };
        Some(dep)
    }

    /// Does `version` satisfy this dependency?
    pub fn accepts(&self, version: &str) -> bool {
        match self.op {
            DepOp::Any => true,
            DepOp::AtLeast => compare_versions(version, &self.version).is_ge(),
            DepOp::Exactly => compare_versions(version, &self.version).is_eq(),
        }
    }
}

impl std::fmt::Display for Dep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.op {
            DepOp::Any => write!(f, "{}", self.id),
            DepOp::AtLeast => write!(f, "{} >= {}", self.id, self.version),
            DepOp::Exactly => write!(f, "{} = {}", self.id, self.version),
        }
    }
}

/// A shell command name: 1..32 of `[a-z0-9_-]`, starting with a letter or digit.
pub fn valid_command(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
        && name.as_bytes()[0].is_ascii_alphanumeric()
}

/// The command a `bin` file installs: its file name without `.wasm`.
pub fn command_of(bin: &str) -> &str {
    let file = bin.rsplit('/').next().unwrap_or(bin);
    file.strip_suffix(".wasm").unwrap_or(file)
}

/// A module name both Python and Lua can import: an identifier of at most 64 characters.
pub fn valid_module(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        && !name.as_bytes()[0].is_ascii_digit()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hotkey {
    pub slot: u8,
    pub action: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub entry: String,
    pub summary: String,
    pub publisher: String,
    pub sizes: Vec<String>,
    pub permissions: Vec<String>,
    pub background_refresh: bool,
    pub background_modes: Vec<String>,
    pub open_with: Vec<String>,
    pub hotkeys: Vec<Hotkey>,
    pub icon: String,
    pub kind: Kind,
    /// Programs: the `.wasm` files that become shell commands.
    pub bin: Vec<String>,
    /// Libraries: the top-level module names under `python/` and `lua/`.
    pub provides: Vec<String>,
    /// Packages that must be installed first.
    pub depends: Vec<Dep>,
}

impl Manifest {
    /// The shell commands a program package installs.
    pub fn commands(&self) -> Vec<&str> {
        self.bin.iter().map(|b| command_of(b)).collect()
    }

    /// The package format level a client must understand (1: apps only, 2: kinds and dependencies).
    pub fn level(&self) -> u32 {
        if self.kind != Kind::App || !self.depends.is_empty() || self.entry.ends_with(".lua") {
            2
        } else {
            1
        }
    }
}

/// The highest [`Manifest::level`] this client installs.
pub const FORMAT_LEVEL: u32 = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct ManifestError {
    /// Which field is wrong: a key name, `syntax` or `unknown`.
    pub field: String,
    pub message: String,
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "app.toml: {}: {}", self.field, self.message)
    }
}

fn err<T>(field: &str, msg: impl Into<String>) -> Result<T, ManifestError> {
    Err(ManifestError { field: field.into(), message: msg.into() })
}

/// True for a valid permission name (docs/gui/UI_PROTOCOL.md §10).
pub fn valid_permission(p: &str) -> bool {
    if PERMISSIONS.contains(&p) {
        return true;
    }
    if let Some(t) = p.strip_prefix("peripherals:") {
        return !t.is_empty() && t.len() <= 32 && t.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_');
    }
    if let Some(m) = p.strip_prefix("background:") {
        return m == "refresh" || BACKGROUND_MODES.contains(&m);
    }
    false
}

/// Reverse-DNS app id: two or more dot-separated labels of `[a-z0-9_-]`, the first
/// starting with a letter; at most 128 characters.
pub fn valid_id(id: &str) -> bool {
    if id.is_empty() || id.len() > MAX_ID {
        return false;
    }
    let labels: Vec<&str> = id.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    for (i, l) in labels.iter().enumerate() {
        if l.is_empty() || !l.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-') {
            return false;
        }
        if i == 0 && !l.as_bytes()[0].is_ascii_lowercase() {
            return false;
        }
    }
    true
}

/// `MAJOR.MINOR.PATCH`, optionally followed by `-pre` or `+build` (`[0-9A-Za-z.-]`).
pub fn valid_version(v: &str) -> bool {
    parse_version(v).is_some()
}

fn parse_version(v: &str) -> Option<([u64; 3], &str)> {
    if v.is_empty() || v.len() > 32 {
        return None;
    }
    let (core, rest) = match v.find(|c| c == '-' || c == '+') {
        Some(i) => (&v[..i], &v[i..]),
        None => (v, ""),
    };
    if !rest.is_empty()
        && (rest.len() < 2 || !rest[1..].bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-'))
    {
        return None;
    }
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut n = [0u64; 3];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() || p.len() > 9 || !p.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        n[i] = p.parse().ok()?;
    }
    Some((n, rest))
}

/// Compare versions: numerically by MAJOR.MINOR.PATCH, then a pre-release (`-x`)
/// sorts before the release, then the suffixes as strings. Invalid versions sort first.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    match (parse_version(a), parse_version(b)) {
        (None, None) => a.cmp(b),
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some((na, ra)), Some((nb, rb))) => na.cmp(&nb).then_with(|| {
            let pa = ra.starts_with('-');
            let pb = rb.starts_with('-');
            match (pa, pb) {
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => ra.cmp(rb),
            }
        }),
    }
}

/// A safe relative path inside the package: no leading `/`, no `..`, no `\`, no empty parts.
pub fn safe_rel_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 255
        && !p.starts_with('/')
        && !p.contains('\\')
        && p.split('/').all(|s| !s.is_empty() && s != ".." && s != ".")
        && !p.bytes().any(|c| c < 0x20)
}

fn text(field: &str, v: &Value, max: usize, required_nonempty: bool) -> Result<String, ManifestError> {
    let Value::Str(s) = v else { return err(field, format!("must be a string, not {}", v.type_name())) };
    if required_nonempty && s.trim().is_empty() {
        return err(field, "must not be empty");
    }
    if s.chars().count() > max {
        return err(field, format!("longer than {} characters", max));
    }
    if s.chars().any(|c| c.is_control()) {
        return err(field, "contains control characters");
    }
    Ok(s.clone())
}

fn str_list(field: &str, v: &Value) -> Result<Vec<String>, ManifestError> {
    let Value::Array(items) = v else { return err(field, "must be an array of strings") };
    let mut out = Vec::new();
    for it in items {
        let Value::Str(s) = it else { return err(field, "must be an array of strings") };
        if out.contains(s) {
            return err(field, format!("'{}' is listed twice", s));
        }
        out.push(s.clone());
    }
    Ok(out)
}

pub fn parse(text_src: &str) -> Result<Manifest, ManifestError> {
    let t = toml_lite::parse(text_src).map_err(|m| ManifestError { field: "syntax".into(), message: m })?;
    from_table(&t)
}

pub fn from_table(t: &Table) -> Result<Manifest, ManifestError> {
    for k in t.keys() {
        if !KNOWN_KEYS.contains(&k.as_str()) {
            return err("unknown", format!("unknown key '{}'", k));
        }
    }
    let get = |k: &str| t.get(k);
    let id = match get("id") {
        Some(v) => text("id", v, MAX_ID, true)?,
        None => return err("id", "missing"),
    };
    if !valid_id(&id) {
        return err("id", format!("'{}' is not a reverse-DNS id like dev.you.app", id));
    }
    let name = match get("name") {
        Some(v) => text("name", v, MAX_NAME, true)?,
        None => return err("name", "missing"),
    };
    let version = match get("version") {
        Some(v) => text("version", v, 32, true)?,
        None => return err("version", "missing"),
    };
    if !valid_version(&version) {
        return err("version", format!("'{}' is not MAJOR.MINOR.PATCH", version));
    }
    let kind = match get("kind") {
        Some(v) => {
            let s = text("kind", v, 16, true)?;
            match Kind::parse(&s) {
                Some(k) => k,
                None => return err("kind", format!("unknown kind '{}' (app, program, library, source)", s)),
            }
        }
        None => Kind::App,
    };
    let entry = match get("entry") {
        Some(v) => text("entry", v, 255, true)?,
        None if kind == Kind::App => return err("entry", "missing"),
        None => String::new(),
    };
    if !entry.is_empty() {
        if kind != Kind::App {
            return err("entry", format!("a {} has no entry", kind.name()));
        }
        if !safe_rel_path(&entry) || !(entry.ends_with(".wasm") || entry.ends_with(".py") || entry.ends_with(".lua")) {
            return err("entry", format!("'{}' must be a relative path to a .wasm, .py or .lua file", entry));
        }
    }
    let bin = match get("bin") {
        Some(v) => {
            let b = str_list("bin", v)?;
            if kind != Kind::Program {
                return err("bin", format!("only a program has bin, not a {}", kind.name()));
            }
            for f in &b {
                if !safe_rel_path(f) || !f.ends_with(".wasm") || !valid_command(command_of(f)) {
                    return err("bin", format!("'{}' must be a relative path to a .wasm file named like a command", f));
                }
            }
            for (i, f) in b.iter().enumerate() {
                if b[..i].iter().any(|o| command_of(o) == command_of(f)) {
                    return err("bin", format!("the command '{}' is listed twice", command_of(f)));
                }
            }
            b
        }
        None => Vec::new(),
    };
    if kind == Kind::Program && bin.is_empty() {
        return err("bin", "a program must list at least one .wasm file");
    }
    let provides = match get("provides") {
        Some(v) => {
            let p = str_list("provides", v)?;
            if kind != Kind::Library {
                return err("provides", format!("only a library has provides, not a {}", kind.name()));
            }
            if let Some(bad) = p.iter().find(|m| !valid_module(m)) {
                return err("provides", format!("'{}' is not a module name", bad));
            }
            p
        }
        None => Vec::new(),
    };
    if kind == Kind::Library && provides.is_empty() {
        return err("provides", "a library must list at least one module");
    }
    let depends = match get("depends") {
        Some(v) => {
            let mut out: Vec<Dep> = Vec::new();
            for d in str_list("depends", v)? {
                let Some(dep) = Dep::parse(&d) else {
                    return err("depends", format!("'{}' must look like dev.you.lib or dev.you.lib >= 1.2.0", d));
                };
                if dep.id == id {
                    return err("depends", "a package cannot depend on itself");
                }
                if out.iter().any(|o| o.id == dep.id) {
                    return err("depends", format!("'{}' is listed twice", dep.id));
                }
                out.push(dep);
            }
            if out.len() > 32 {
                return err("depends", "more than 32 dependencies");
            }
            out
        }
        None => Vec::new(),
    };
    let summary = match get("summary") {
        Some(v) => text("summary", v, MAX_SUMMARY, false)?,
        None => String::new(),
    };
    let publisher = match get("publisher") {
        Some(v) => text("publisher", v, MAX_PUBLISHER, false)?,
        None => String::new(),
    };
    let sizes = match get("sizes") {
        Some(v) => {
            let s = str_list("sizes", v)?;
            if s.is_empty() {
                return err("sizes", "must list at least one size");
            }
            if let Some(bad) = s.iter().find(|x| !SIZES.contains(&x.as_str())) {
                return err("sizes", format!("unknown size '{}' (compact, medium, wide)", bad));
            }
            s
        }
        None => SIZES.iter().map(|s| s.to_string()).collect(),
    };
    let mut permissions = match get("permissions") {
        Some(v) => str_list("permissions", v)?,
        None => Vec::new(),
    };
    if let Some(bad) = permissions.iter().find(|p| !valid_permission(p)) {
        return err("permissions", format!("unknown permission '{}'", bad));
    }
    let (mut background_refresh, mut background_modes) = (false, Vec::new());
    if let Some(v) = get("background") {
        let Value::Table(bg) = v else { return err("background", "must be a table like { refresh = true }") };
        for k in bg.keys() {
            if k != "refresh" && k != "modes" {
                return err("background", format!("unknown key '{}'", k));
            }
        }
        if let Some(r) = bg.get("refresh") {
            let Value::Bool(b) = r else { return err("background", "refresh must be true or false") };
            background_refresh = *b;
        }
        if let Some(m) = bg.get("modes") {
            background_modes = str_list("background", m)?;
            if let Some(bad) = background_modes.iter().find(|m| !BACKGROUND_MODES.contains(&m.as_str())) {
                return err("background", format!("unknown mode '{}' (audio, call, radio, transfer, navigation)", bad));
            }
        }
    }
    // Background wishes declare their permissions.
    if background_refresh && !permissions.iter().any(|p| p == "background:refresh") {
        permissions.push("background:refresh".into());
    }
    for m in &background_modes {
        let p = format!("background:{}", m);
        if !permissions.contains(&p) {
            permissions.push(p);
        }
    }
    let open_with = match get("open_with") {
        Some(v) => {
            let s = str_list("open_with", v)?;
            for e in &s {
                let ok = e.len() >= 2
                    && e.len() <= 17
                    && e.starts_with('.')
                    && e[1..].bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
                if !ok {
                    return err("open_with", format!("'{}' must look like \".txt\" (lower case)", e));
                }
            }
            s
        }
        None => Vec::new(),
    };
    let mut hotkeys = Vec::new();
    if let Some(v) = get("hotkeys") {
        let Value::Array(items) = v else { return err("hotkeys", "must be an array of { slot, action, label }") };
        for it in items {
            let Value::Table(h) = it else { return err("hotkeys", "must be an array of { slot, action, label }") };
            for k in h.keys() {
                if k != "slot" && k != "action" && k != "label" {
                    return err("hotkeys", format!("unknown key '{}'", k));
                }
            }
            let slot = match h.get("slot") {
                Some(Value::Int(n)) if (1..=6).contains(n) => *n as u8,
                _ => return err("hotkeys", "slot must be 1..6"),
            };
            if hotkeys.iter().any(|k: &Hotkey| k.slot == slot) {
                return err("hotkeys", format!("slot {} is used twice", slot));
            }
            let action = match h.get("action") {
                Some(Value::Str(s))
                    if !s.is_empty()
                        && s.len() <= 64
                        && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_') =>
                {
                    s.clone()
                }
                _ => return err("hotkeys", "action must be a name like new_note"),
            };
            let label = match h.get("label") {
                Some(v) => text("hotkeys", v, 32, true)?,
                None => return err("hotkeys", "label is missing"),
            };
            hotkeys.push(Hotkey { slot, action, label });
        }
    }
    let icon = match get("icon") {
        Some(v) => {
            let s = text("icon", v, 255, true)?;
            if !safe_rel_path(&s) || !s.ends_with(".png") {
                return err("icon", format!("'{}' must be a relative path to a .png file", s));
            }
            s
        }
        None => String::new(),
    };
    Ok(Manifest {
        id,
        name,
        version,
        entry,
        summary,
        publisher,
        sizes,
        permissions,
        background_refresh,
        background_modes,
        open_with,
        hotkeys,
        icon,
        kind,
        bin,
        provides,
        depends,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn vectors() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../abi/app-manifest-vectors")
    }

    /// The shared vectors: `ok_*` parse, `bad_<field>_*` fail on that field.
    #[test]
    fn shared_vectors() {
        let mut n = 0;
        for e in std::fs::read_dir(vectors()).expect("abi/app-manifest-vectors") {
            let p = e.unwrap().path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if !name.ends_with(".toml") {
                continue;
            }
            let src = std::fs::read_to_string(&p).unwrap();
            let r = parse(&src);
            if name.starts_with("ok_") {
                assert!(r.is_ok(), "{name}: {:?}", r.err());
            } else if let Some(rest) = name.strip_prefix("bad_") {
                let field = rest.rsplit_once('_').map(|(f, _)| f).unwrap_or(rest);
                let field = field.trim_end_matches(".toml");
                match r {
                    Ok(_) => panic!("{name} parsed"),
                    Err(e) => assert_eq!(e.field, field, "{name}: {}", e),
                }
            }
            n += 1;
        }
        assert!(n >= 10, "only {n} vectors");
    }

    #[test]
    fn background_declares_its_permissions() {
        let m = parse("id='a.b'\nname='A'\nversion='1.0.0'\nentry='main.py'\nbackground={refresh=true, modes=['audio']}").unwrap();
        assert!(m.permissions.contains(&"background:refresh".to_string()));
        assert!(m.permissions.contains(&"background:audio".to_string()));
        assert_eq!(m.sizes.len(), 3);
    }

    #[test]
    fn kinds_and_dependencies() {
        let base = "id='dev.me.x'\nname='X'\nversion='1.0.0'\n";
        let p = parse(&format!("{base}kind='program'\nbin=['tools/hexdump.wasm','b64.wasm']\ndepends=['dev.me.lib >= 1.2.0','dev.me.other']")).unwrap();
        assert_eq!((p.kind, p.commands()), (Kind::Program, vec!["hexdump", "b64"]));
        assert_eq!(p.depends[0].to_string(), "dev.me.lib >= 1.2.0");
        assert!(p.depends[0].accepts("1.2.0") && p.depends[0].accepts("2.0.0") && !p.depends[0].accepts("1.1.9"));
        assert!(p.depends[1].accepts("0.0.1"));
        assert_eq!(p.level(), 2);
        let l = parse(&format!("{base}kind='library'\nprovides=['geometry']")).unwrap();
        assert_eq!((l.kind, l.entry.as_str()), (Kind::Library, ""));
        assert!(l.kind.unconfined());
        let s = parse(&format!("{base}kind='source'")).unwrap();
        assert!(!s.kind.unconfined());
        let a = parse(&format!("{base}entry='main.lua'")).unwrap();
        assert_eq!((a.kind, a.level()), (Kind::App, 2));
        assert_eq!(parse(&format!("{base}entry='main.py'")).unwrap().level(), 1);
        for (bad, field) in [
            ("kind='gadget'\nentry='main.py'", "kind"),
            ("kind='program'", "bin"),
            ("kind='program'\nbin=['Tool.wasm']", "bin"),
            ("kind='program'\nbin=['a/x.wasm','b/x.wasm']", "bin"),
            ("entry='main.py'\nbin=['x.wasm']", "bin"),
            ("kind='library'", "provides"),
            ("kind='library'\nprovides=['9lives']", "provides"),
            ("kind='library'\nprovides=['m']\nentry='main.py'", "entry"),
            ("entry='main.py'\ndepends=['dev.me.x']", "depends"),
            ("entry='main.py'\ndepends=['dev.me.y > 1.0.0']", "depends"),
            ("entry='main.py'\ndepends=['dev.me.y','dev.me.y = 1.0.0']", "depends"),
        ] {
            assert_eq!(parse(&format!("{base}{bad}")).unwrap_err().field, field, "{bad}");
        }
        assert_eq!(Dep::parse("dev.a.b = 1.0.0").unwrap().op, DepOp::Exactly);
        assert!(Dep::parse("nodots").is_none());
    }

    #[test]
    fn version_order() {
        assert_eq!(compare_versions("1.2.0", "1.10.0"), Ordering::Less);
        assert_eq!(compare_versions("1.0.0-beta", "1.0.0"), Ordering::Less);
        assert_eq!(compare_versions("2.0.0", "1.9.9"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0", "1.0.0"), Ordering::Equal);
    }
}
