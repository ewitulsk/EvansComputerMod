//! The system file picker (UI Protocol §12 "File picker"): an app sends
//! `PickFile{req, mode, types, suggested}`; the compositor starts
//!
//! ```text
//! picker --req <token> --app <app id> --mode open|save [--types .txt,.md] [--suggested <name>]
//! ```
//!
//! as a window attached to the app's window. The picker is drawn by the
//! system, not the app, and can show what the app itself can't read. The
//! player's answer goes back as `picked <token> <path>` / `picked <token> cancel`
//! on `/dev/uictl` ([`crate::uictl`]); the compositor turns it into
//! `FilePicked{req, path}` and a one-file grant for that app.
//!
//! What the picker never offers: the app database (`/var/lib/apps`), other
//! apps' data (`/var/apps/<other>`), devices (`/dev`) and the package signing
//! key; it never saves into `/apps`, `/etc` or `/var/lib`.

use crate::uictl::{self, Answer};
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::element::{button, column, label, list, row, scroll_view, spacer, text_field, Element};
use ecm_ui::{App, Align, ViewCtx};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Open,
    Save,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickRequest {
    pub token: String,
    /// The app that asked (shown in the title, and its own data folder is a place).
    pub app: String,
    pub mode: Mode,
    /// Lower-case extensions (`.txt`); empty = any file.
    pub types: Vec<String>,
    pub suggested: String,
}

pub fn parse_args(args: &[String]) -> Result<PickRequest, String> {
    let token = uictl::arg(args, "--req").ok_or("--req <token> is missing")?;
    let mode = match uictl::arg(args, "--mode").as_deref() {
        Some("open") | None => Mode::Open,
        Some("save") => Mode::Save,
        Some(m) => return Err(format!("unknown mode {m}")),
    };
    let types = uictl::arg(args, "--types")
        .map(|t| {
            t.split([',', ' '])
                .filter(|x| !x.is_empty())
                .map(|x| if x.starts_with('.') { x.to_lowercase() } else { format!(".{}", x.to_lowercase()) })
                .collect()
        })
        .unwrap_or_default();
    Ok(PickRequest { token, app: uictl::arg(args, "--app").unwrap_or_default(), mode, types, suggested: uictl::arg(args, "--suggested").unwrap_or_default() })
}

/// A directory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// The disk as the picker sees it.
pub trait Fs {
    fn list(&self, dir: &str) -> Result<Vec<FsEntry>, String>;
    /// `Some(is_dir)` if the path exists.
    fn kind(&self, path: &str) -> Option<bool>;
}

/// `std::fs` under a root prefix (`""` on a computer).
pub struct HostFs {
    pub root: String,
}

impl Fs for HostFs {
    fn list(&self, dir: &str) -> Result<Vec<FsEntry>, String> {
        let rd = std::fs::read_dir(format!("{}{}", self.root, dir)).map_err(|e| format!("can't open {dir}: {e}"))?;
        Ok(rd
            .flatten()
            .map(|e| {
                let md = e.metadata().ok();
                FsEntry {
                    name: e.file_name().to_string_lossy().into_owned(),
                    is_dir: md.as_ref().is_some_and(|m| m.is_dir()),
                    size: md.map_or(0, |m| m.len()),
                }
            })
            .collect())
    }

    fn kind(&self, path: &str) -> Option<bool> {
        std::fs::metadata(format!("{}{}", self.root, path)).ok().map(|m| m.is_dir())
    }
}

/// An absolute path with `.` and `..` resolved; `None` for relative paths,
/// backslashes or NULs. `..` never climbs above `/`.
pub fn normalize(path: &str) -> Option<String> {
    if !path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            x => parts.push(x),
        }
    }
    Some(format!("/{}", parts.join("/")))
}

fn under(path: &str, dir: &str) -> bool {
    path == dir || path.starts_with(&format!("{dir}/")) || dir == "/"
}

/// May the picker show (and hand out) `path` to `app`?
pub fn allowed(path: &str, app: &str) -> bool {
    let Some(p) = normalize(path) else { return false };
    if under(&p, "/var/lib/apps") || under(&p, "/dev") || p == ecm_pkg::paths::SIGNING_KEY {
        return false;
    }
    if under(&p, "/var/apps") && p != "/var/apps" {
        let own = format!("/var/apps/{app}");
        return !app.is_empty() && under(&p, &own);
    }
    true
}

/// May a save go to `path`? (Never into installed apps' code or system settings.)
pub fn may_save(path: &str, app: &str) -> bool {
    allowed(path, app) && !["/apps", "/etc", "/var/lib", "/bin"].iter().any(|d| normalize(path).is_some_and(|p| under(&p, d)))
}

/// Does `name` have one of the extensions (any name when there are none)?
pub fn matches(name: &str, types: &[String]) -> bool {
    types.is_empty() || crate::appctl::extension(name).is_some_and(|e| types.contains(&e))
}

/// Check a file name typed into the save field; adds the first type's
/// extension when the name has none of them.
pub fn validate_name(name: &str, types: &[String]) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("Type a file name.".into());
    }
    if n.chars().count() > 64 {
        return Err("The name is too long (64 characters at most).".into());
    }
    if n.contains('/') || n.contains('\\') || n.chars().any(|c| c.is_control()) {
        return Err("A file name can't contain / or \\.".into());
    }
    if n == "." || n == ".." || n.starts_with('.') {
        return Err("A file name can't start with a dot.".into());
    }
    if matches(n, types) {
        Ok(n.to_string())
    } else {
        Ok(format!("{}{}", n, types[0]))
    }
}

pub fn join(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// The picker's state.
pub struct Picker {
    pub req: PickRequest,
    pub cwd: String,
    pub entries: Vec<FsEntry>,
    pub selected: Option<usize>,
    pub name: String,
    pub error: String,
    /// A save over an existing file, waiting for "Replace".
    pub confirm: Option<String>,
    /// The answer, once given: `Some(path)` or `None` (cancelled).
    pub done: Option<Option<String>>,
}

impl Picker {
    pub fn new(req: PickRequest, fs: &dyn Fs) -> Picker {
        let name = req.suggested.rsplit('/').next().unwrap_or("").to_string();
        let mut p = Picker { req, cwd: "/".into(), entries: Vec::new(), selected: None, name, error: String::new(), confirm: None, done: None };
        let start = p.places().into_iter().map(|(_, d)| d).find(|d| fs.kind(d) == Some(true)).unwrap_or_else(|| "/".into());
        p.open_dir(fs, &start);
        p
    }

    /// Shortcuts: Documents, the app's own data, the whole computer.
    pub fn places(&self) -> Vec<(String, String)> {
        let mut v = vec![("Documents".to_string(), "/home".to_string())];
        if !self.req.app.is_empty() && ecm_pkg::manifest::valid_id(&self.req.app) {
            v.push(("App data".into(), format!("/var/apps/{}", self.req.app)));
        }
        v.push(("Computer".into(), "/".into()));
        v
    }

    pub fn title(&self) -> String {
        let who = if self.req.app.is_empty() { "an app".to_string() } else { self.req.app.clone() };
        match self.req.mode {
            Mode::Open => format!("Open a file for {who}"),
            Mode::Save => format!("Save a file for {who}"),
        }
    }

    pub fn open_dir(&mut self, fs: &dyn Fs, dir: &str) {
        let Some(d) = normalize(dir) else { return };
        if !allowed(&d, &self.req.app) {
            self.error = format!("{d} is not available here.");
            return;
        }
        match fs.list(&d) {
            Ok(mut es) => {
                es.retain(|e| {
                    !e.name.starts_with('.') && allowed(&join(&d, &e.name), &self.req.app) && (e.is_dir || matches(&e.name, &self.req.types))
                });
                es.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
                self.entries = es;
                self.cwd = d;
                self.selected = None;
                self.error.clear();
                self.confirm = None;
            }
            Err(e) => self.error = e,
        }
    }

    pub fn up(&mut self, fs: &dyn Fs) {
        let parent = normalize(&format!("{}/..", self.cwd)).unwrap_or_else(|| "/".into());
        self.open_dir(fs, &parent);
    }

    /// A row was chosen: folders open, files are selected (and named, when saving).
    pub fn select(&mut self, fs: &dyn Fs, i: usize) {
        let Some(e) = self.entries.get(i).cloned() else { return };
        if e.is_dir {
            let d = join(&self.cwd, &e.name);
            self.open_dir(fs, &d);
        } else {
            self.selected = Some(i);
            self.confirm = None;
            if self.req.mode == Mode::Save {
                self.name = e.name;
            }
        }
    }

    /// Open / Save pressed. Sets [`Picker::done`] when there is an answer.
    pub fn confirm(&mut self, fs: &dyn Fs) {
        self.error.clear();
        match self.req.mode {
            Mode::Open => match self.selected.and_then(|i| self.entries.get(i)) {
                Some(e) => self.done = Some(Some(join(&self.cwd, &e.name))),
                None => self.error = "Choose a file first.".into(),
            },
            Mode::Save => {
                let name = match validate_name(&self.name, &self.req.types) {
                    Ok(n) => n,
                    Err(e) => {
                        self.error = e;
                        return;
                    }
                };
                let path = join(&self.cwd, &name);
                if !may_save(&path, &self.req.app) {
                    self.error = format!("Apps can't save into {}.", self.cwd);
                    return;
                }
                match fs.kind(&path) {
                    Some(true) => self.error = format!("{name} is a folder."),
                    Some(false) if self.confirm.as_deref() != Some(path.as_str()) => {
                        self.error = format!("{name} already exists. Press Replace to overwrite it.");
                        self.confirm = Some(path);
                    }
                    _ => self.done = Some(Some(path)),
                }
            }
        }
    }

    pub fn cancel(&mut self) {
        self.done = Some(None);
    }

    /// The `/dev/uictl` line for the answer, once there is one.
    pub fn answer_line(&self) -> Option<String> {
        self.done.as_ref().map(|d| uictl::picked_line(&self.req.token, d.as_deref()))
    }
}

/// What the picker's handlers reach: the state and the disk.
pub struct Shared {
    pub picker: Picker,
    pub fs: Box<dyn Fs>,
}

type P = Rc<RefCell<Shared>>;

/// The picker's window.
pub fn view(p: &P, cx: &ViewCtx) -> Element {
    let sh = p.borrow();
    let pk = &sh.picker;
    let compact = cx.class == ecm_ui_proto::SizeClass::Compact;
    let mut places = row().gap(6);
    for (i, (name, dir)) in pk.places().into_iter().enumerate() {
        let h = p.clone();
        places = places.child(button(name).id(format!("place:{i}")).on_click(move |_, _| {
            let mut s = h.borrow_mut();
            let Shared { picker, fs } = &mut *s;
            picker.open_dir(fs.as_ref(), &dir);
        }));
    }
    let up = p.clone();
    let path_row = row()
        .gap(6)
        .child(button("Up").id("up").enabled(pk.cwd != "/").on_click(move |_, _| {
            let mut s = up.borrow_mut();
            let Shared { picker, fs } = &mut *s;
            picker.up(fs.as_ref());
        }))
        .child(label(pk.cwd.clone()).wrap(false).id("cwd"));
    let rows: Vec<Element> = pk
        .entries
        .iter()
        .map(|e| {
            if e.is_dir {
                label(format!("{}/", e.name)).wrap(false)
            } else {
                row().gap(6).child(label(e.name.clone()).wrap(false)).child(spacer()).child(label(crate::widgets::size_text(e.size)).style("caption").wrap(false))
            }
        })
        .collect();
    let sel = p.clone();
    let mut files = list().id("files").children(rows).on_select(move |cx, _| {
        let i = cx.selected("files").unwrap_or(0);
        let mut s = sel.borrow_mut();
        let Shared { picker, fs } = &mut *s;
        picker.select(fs.as_ref(), i);
    });
    if let Some(i) = pk.selected {
        files = files.selected(i);
    }
    let empty = pk.entries.is_empty();
    let fixed = 10 * 2 + 20 + 8 + 23 + 8 + 23 + 8 + 12 + 8 + if pk.req.mode == Mode::Save { 31 } else { 0 } + 23 + 8 + if pk.error.is_empty() { 0 } else { 20 };
    let list_h = (cx.height - fixed).max(60);
    let body = if empty { column().child(label("Nothing to show here.").style("caption")) } else { column().child(files) };
    let mut c = column()
        .gap(8)
        .padding(10)
        .child(label(pk.title()).style("title"))
        .child(places)
        .child(path_row)
        .child(scroll_view(body).id("scroll").height(list_h))
        .child(label(if pk.req.types.is_empty() { "Showing all files".to_string() } else { format!("Showing {}", pk.req.types.join(" ")) }).style("caption"));
    if pk.req.mode == Mode::Save {
        let n = p.clone();
        c = c.child(row().gap(6).child(label("Name").wrap(false)).child(text_field().id("name").text(pk.name.clone()).on_change(move |cx, _| {
            let t = cx.text("name");
            let mut s = n.borrow_mut();
            s.picker.name = t;
            s.picker.confirm = None;
        })));
    }
    if !pk.error.is_empty() {
        c = c.child(label(pk.error.clone()).color(crate::widgets::RED).id("error"));
    }
    let cancel = p.clone();
    let ok = p.clone();
    let ok_text = match (pk.req.mode, pk.confirm.is_some()) {
        (Mode::Open, _) => "Open",
        (Mode::Save, false) => "Save",
        (Mode::Save, true) => "Replace",
    };
    let mut buttons = row().gap(8).align(Align::Center);
    if !compact {
        buttons = buttons.child(spacer());
    }
    buttons = buttons
        .child(button("Cancel").id("cancel").on_click(move |_, _| cancel.borrow_mut().picker.cancel()))
        .child(button(ok_text).style("primary").id("ok").on_click(move |_, _| {
            let mut s = ok.borrow_mut();
            let Shared { picker, fs } = &mut *s;
            picker.confirm(fs.as_ref());
        }));
    c.child(buttons)
}

/// The picker program: a window until the player answers, then the answer line.
pub struct PickerApp<A: Answer> {
    pub app: App,
    pub state: P,
    pub answer: A,
}

impl<A: Answer> PickerApp<A> {
    pub fn new(conn: Box<dyn Connection>, req: PickRequest, fs: Box<dyn Fs>, answer: A) -> Result<Self, ConnError> {
        let mut app = App::with_connection("picker", conn)?;
        let picker = Picker::new(req, fs.as_ref());
        let title = picker.title();
        let state = Rc::new(RefCell::new(Shared { picker, fs }));
        let s = state.clone();
        app.window(&title, move |cx| view(&s, cx));
        let c = state.clone();
        // Closing the window is a cancel.
        app.on_close(move |_| c.borrow_mut().picker.cancel());
        Ok(PickerApp { app, state, answer })
    }

    /// One loop turn; false once answered (the answer is sent) or the connection closed.
    pub fn step(&mut self, timeout_ms: i32) -> bool {
        let alive = self.app.step(timeout_ms);
        let line = self.state.borrow().picker.answer_line();
        if let Some(l) = line {
            let _ = self.answer.send(&l);
            return false;
        }
        if !alive {
            // Window gone without an answer: cancel, so the app isn't left waiting.
            let l = uictl::picked_line(&self.state.borrow().picker.req.token, None);
            let _ = self.answer.send(&l);
        }
        alive
    }

    pub fn run(&mut self) {
        while self.step(-1) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A disk in memory: path -> is_dir.
    struct MemFs(BTreeMap<String, bool>);

    impl MemFs {
        fn new(paths: &[&str]) -> MemFs {
            let mut m = BTreeMap::new();
            m.insert("/".to_string(), true);
            for p in paths {
                let dir = p.ends_with('/');
                let p = p.trim_end_matches('/');
                let mut acc = String::new();
                let parts: Vec<&str> = p.split('/').filter(|x| !x.is_empty()).collect();
                for (i, part) in parts.iter().enumerate() {
                    acc = format!("{acc}/{part}");
                    m.insert(acc.clone(), i + 1 < parts.len() || dir);
                }
            }
            MemFs(m)
        }
    }

    impl Fs for MemFs {
        fn list(&self, dir: &str) -> Result<Vec<FsEntry>, String> {
            if self.0.get(dir) != Some(&true) {
                return Err(format!("can't open {dir}"));
            }
            Ok(self
                .0
                .iter()
                .filter(|(p, _)| p.as_str() != dir && p.rsplit_once('/').map(|(a, _)| if a.is_empty() { "/" } else { a }) == Some(dir))
                .map(|(p, d)| FsEntry { name: p.rsplit('/').next().unwrap().to_string(), is_dir: *d, size: 10 })
                .collect())
        }
        fn kind(&self, path: &str) -> Option<bool> {
            self.0.get(path).copied()
        }
    }

    fn req(mode: Mode, types: &[&str], suggested: &str) -> PickRequest {
        PickRequest { token: "t1".into(), app: "dev.ecm.notes".into(), mode, types: types.iter().map(|s| s.to_string()).collect(), suggested: suggested.into() }
    }

    fn disk() -> MemFs {
        MemFs::new(&[
            "/home/a.txt",
            "/home/b.md",
            "/home/pic.png",
            "/home/.hidden.txt",
            "/home/sub/c.txt",
            "/var/apps/dev.ecm.notes/state",
            "/var/apps/dev.other/secret.txt",
            "/var/lib/apps/db.json",
            "/etc/pkg/signing.key",
            "/apps/dev.ecm.notes/main.py",
        ])
    }

    #[test]
    fn args() {
        let a: Vec<String> = ["--req", "x9", "--app", "dev.a", "--mode", "save", "--types", "txt,.MD", "--suggested", "my notes.txt"].iter().map(|s| s.to_string()).collect();
        let r = parse_args(&a).unwrap();
        assert_eq!(r.mode, Mode::Save);
        assert_eq!(r.types, vec![".txt", ".md"]);
        assert_eq!(r.suggested, "my notes.txt");
        assert!(parse_args(&["--mode".into(), "open".into()]).is_err());
        assert!(parse_args(&["--req".into(), "a".into(), "--mode".into(), "delete".into()]).is_err());
    }

    #[test]
    fn path_safety() {
        assert_eq!(normalize("/home/../etc/./x").as_deref(), Some("/etc/x"));
        assert_eq!(normalize("/../../x").as_deref(), Some("/x"));
        assert_eq!(normalize("home/x"), None);
        assert_eq!(normalize("/home\\x"), None);
        let app = "dev.ecm.notes";
        assert!(allowed("/home/a.txt", app));
        assert!(allowed("/var/apps/dev.ecm.notes/state", app));
        assert!(allowed("/var/apps", app));
        assert!(!allowed("/var/apps/dev.other/secret.txt", app));
        assert!(!allowed("/var/apps/dev.ecm.notes/../dev.other/x", app));
        assert!(!allowed("/var/lib/apps/db.json", app));
        assert!(!allowed("/var/lib/apps", app));
        assert!(!allowed("/dev/uictl", app));
        assert!(!allowed("/etc/pkg/signing.key", app));
        assert!(!allowed("/var/apps/x", ""));
        assert!(may_save("/home/n.txt", app));
        assert!(!may_save("/apps/dev.ecm.notes/main.py", app));
        assert!(!may_save("/etc/ui.toml", app));
    }

    #[test]
    fn names_and_filters() {
        let t = vec![".txt".to_string(), ".md".to_string()];
        assert_eq!(validate_name("notes", &t).unwrap(), "notes.txt");
        assert_eq!(validate_name(" todo.md ", &t).unwrap(), "todo.md");
        assert_eq!(validate_name("x.TXT", &t).unwrap(), "x.TXT");
        assert!(validate_name("", &t).is_err());
        assert!(validate_name("a/b.txt", &t).is_err());
        assert!(validate_name("..", &t).is_err());
        assert!(validate_name(".bashrc", &t).is_err());
        assert!(validate_name(&"x".repeat(65), &t).is_err());
        assert_eq!(validate_name("data.bin", &[]).unwrap(), "data.bin");
        assert!(matches("a.txt", &t) && !matches("a.png", &t) && matches("a.png", &[]));
    }

    #[test]
    fn open_mode_browses_and_picks_one_file() {
        let fs = disk();
        let mut p = Picker::new(req(Mode::Open, &[".txt"], ""), &fs);
        assert_eq!(p.cwd, "/home");
        let names: Vec<_> = p.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["sub", "a.txt"], "folders first, filtered by type, hidden files left out");
        p.confirm(&fs);
        assert!(p.done.is_none() && !p.error.is_empty(), "nothing chosen yet");
        p.select(&fs, 0);
        assert_eq!(p.cwd, "/home/sub");
        p.select(&fs, 0);
        p.confirm(&fs);
        assert_eq!(p.answer_line().unwrap(), "picked t1 /home/sub/c.txt");
        // Other apps' data and the app database are not listed.
        let mut p = Picker::new(req(Mode::Open, &[], ""), &fs);
        p.open_dir(&fs, "/var/apps");
        assert_eq!(p.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["dev.ecm.notes"]);
        p.open_dir(&fs, "/var/lib");
        assert!(p.entries.is_empty());
        p.open_dir(&fs, "/var/apps/dev.other");
        assert_eq!(p.cwd, "/var/lib", "refused, stays where it was");
        assert!(p.error.contains("not available"));
        p.cancel();
        assert_eq!(p.answer_line().unwrap(), "picked t1 cancel");
    }

    #[test]
    fn save_mode_names_confirms_overwrites_and_refuses_system_folders() {
        let fs = disk();
        let mut p = Picker::new(req(Mode::Save, &[".txt"], "/somewhere/new note"), &fs);
        assert_eq!(p.name, "new note");
        p.confirm(&fs);
        assert_eq!(p.answer_line().unwrap(), "picked t1 /home/new note.txt");
        let mut p = Picker::new(req(Mode::Save, &[".txt"], "a.txt"), &fs);
        p.confirm(&fs);
        assert!(p.done.is_none() && p.confirm.is_some(), "asks before replacing");
        p.confirm(&fs);
        assert_eq!(p.answer_line().unwrap(), "picked t1 /home/a.txt");
        let mut p = Picker::new(req(Mode::Save, &[], "sub"), &fs);
        p.confirm(&fs);
        assert!(p.error.contains("folder"));
        p.open_dir(&fs, "/apps/dev.ecm.notes");
        p.name = "x.py".into();
        p.confirm(&fs);
        assert!(p.done.is_none() && p.error.contains("can't save"));
    }
}
