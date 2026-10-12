//! `files`: browse the computer's disk. Double-click (or Enter) opens a
//! folder, or a file with the app for its type (until P4's "open with":
//! `.py`, `.txt` and other text go to the Text Editor, `.vpl` to the Visual
//! Programmer, `.gba` to the emulator). New folder, rename (F2), delete
//! (Delete, with a confirmation), copy and paste (Ctrl+C / Ctrl+V; a copy
//! also puts the path on the clipboard as text). Backspace goes up.

use crate::sys::{self, Entry, Sys};
use ecm_ui::element::{button, column, dialog, image, label, list, row, scroll_view, spacer, stack, text_field, Align, Element};
use ecm_ui::window::{keys, EventValue};
use ecm_ui::{App, Ctx};
use ecm_ui_proto as proto;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Which app opens a file (program name), by extension.
pub fn handler_for(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    Some(match ext {
        "vpl" => "visual",
        "gba" => "gba",
        "wasm" => return None,
        _ => "editor",
    })
}

/// A dialog over the file list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    NewFolder,
    Rename(String),
    Delete(String),
}

/// What Files shows.
pub struct State {
    pub cwd: String,
    pub entries: Vec<Entry>,
    pub status: String,
    pub dialog: Option<Dialog>,
    /// Paths copied with Copy (pasted with Paste).
    pub copied: Vec<String>,
    pub icons: HashMap<&'static str, u32>,
}

type Shared = Rc<RefCell<State>>;
type SysRef = Rc<RefCell<Box<dyn Sys>>>;

/// Read `dir` into the state (the status line says what went wrong).
pub fn load(st: &Shared, sys: &SysRef, dir: &str) -> bool {
    let r = sys::list_dir(&**sys.borrow(), dir);
    let mut s = st.borrow_mut();
    match r {
        Ok(entries) => {
            s.cwd = dir.to_string();
            s.status = format!("{} item{}", entries.len(), if entries.len() == 1 { "" } else { "s" });
            s.entries = entries;
            true
        }
        Err(e) => {
            s.status = format!("Can't open {dir}: {e}");
            false
        }
    }
}

fn selected(cx: &Ctx, st: &Shared) -> Option<Entry> {
    let i = cx.selected("list")?;
    st.borrow().entries.get(i).cloned()
}

fn go(cx: &mut Ctx, st: &Shared, sys: &SysRef, dir: &str) {
    if load(st, sys, dir) {
        let cwd = st.borrow().cwd.clone();
        cx.set_text("path", &cwd);
        cx.set_selected("list", None);
    }
}

fn open_entry(cx: &mut Ctx, st: &Shared, sys: &SysRef, e: &Entry) {
    let path = sys::join(&st.borrow().cwd, &e.name);
    if e.dir {
        go(cx, st, sys, &path);
        return;
    }
    match handler_for(&e.name) {
        Some(app) => {
            cx.launch(app, &[path.clone()]);
            st.borrow_mut().status = format!("Opening {} in {}", e.name, app);
        }
        None => {
            let prog = e.name.trim_end_matches(".wasm").to_string();
            cx.launch(&prog, &[]);
            st.borrow_mut().status = format!("Starting {prog}");
        }
    }
}

fn copy_selected(cx: &mut Ctx, st: &Shared) {
    if let Some(e) = selected(cx, st) {
        let path = sys::join(&st.borrow().cwd, &e.name);
        cx.copy(&path);
        let mut s = st.borrow_mut();
        s.status = format!("Copied {}", e.name);
        s.copied = vec![path];
    }
}

fn paste(cx: &mut Ctx, st: &Shared, sys: &SysRef) {
    let (cwd, copied) = {
        let s = st.borrow();
        (s.cwd.clone(), s.copied.clone())
    };
    if copied.is_empty() {
        st.borrow_mut().status = "Nothing to paste: copy a file first".into();
        return;
    }
    let mut msg = String::new();
    for src in &copied {
        let name = sys::free_name(&**sys.borrow(), &cwd, sys::file_name(src));
        let dst = sys::join(&cwd, &name);
        msg = match sys::copy(&**sys.borrow(), src, &dst) {
            Ok(()) => format!("Pasted {name}"),
            Err(e) => format!("Can't paste {}: {e}", sys::file_name(src)),
        };
    }
    load(st, sys, &cwd);
    st.borrow_mut().status = msg;
    let _ = cx;
}

fn ask(cx: &mut Ctx, st: &Shared, d: Dialog) {
    if let Dialog::Rename(name) = &d {
        cx.set_text("name", name);
    } else {
        cx.set_text("name", "");
    }
    let field = !matches!(d, Dialog::Delete(_));
    st.borrow_mut().dialog = Some(d);
    if field {
        cx.focus("name");
    }
}

fn confirm(cx: &mut Ctx, st: &Shared, sys: &SysRef) {
    let Some(d) = st.borrow_mut().dialog.take() else { return };
    let cwd = st.borrow().cwd.clone();
    let name = cx.text("name").trim().to_string();
    let bad = |n: &str| n.is_empty() || n.contains('/') || n == "." || n == "..";
    let msg = match d {
        Dialog::NewFolder if bad(&name) => "Folder names can't be empty or contain /".to_string(),
        Dialog::NewFolder => match sys::make_dir(&**sys.borrow(), &sys::join(&cwd, &name)) {
            Ok(()) => format!("Created {name}"),
            Err(e) => format!("Can't create {name}: {e}"),
        },
        Dialog::Rename(_) if bad(&name) => "Names can't be empty or contain /".to_string(),
        Dialog::Rename(old) => match sys::rename(&**sys.borrow(), &sys::join(&cwd, &old), &sys::join(&cwd, &name)) {
            Ok(()) => format!("Renamed {old} to {name}"),
            Err(e) => format!("Can't rename {old}: {e}"),
        },
        Dialog::Delete(n) => match sys::remove(&**sys.borrow(), &sys::join(&cwd, &n)) {
            Ok(()) => format!("Deleted {n}"),
            Err(e) => format!("Can't delete {n}: {e}"),
        },
    };
    load(st, sys, &cwd);
    st.borrow_mut().status = msg;
    cx.focus("list");
}

fn cancel(cx: &mut Ctx, st: &Shared) {
    st.borrow_mut().dialog = None;
    cx.focus("list");
}

/// The Files app, opened at `start` (default `/`).
pub fn app(conn: Box<dyn ecm_ui::Connection>, sys: Box<dyn Sys>, start: Option<&str>) -> Result<App, ecm_ui::ConnError> {
    let mut app = App::with_connection("dev.ecm.files", conn)?;
    let sys: SysRef = Rc::new(RefCell::new(sys));
    let mut icons = HashMap::new();
    for name in ["folder", "file", "code", "game"] {
        if let Some(i) = ecm_ui::icons::builtin(name) {
            if let Ok(id) = app.core.image_rgba(i.w, i.h, &i.rgba) {
                icons.insert(name, id);
            }
        }
    }
    let st: Shared = Rc::new(RefCell::new(State { cwd: "/".into(), entries: Vec::new(), status: String::new(), dialog: None, copied: Vec::new(), icons }));
    let dir = sys::normalize("/", start.unwrap_or("/"));
    if !load(&st, &sys, &dir) {
        load(&st, &sys, "/");
    }
    let (s1, y1) = (st.clone(), sys.clone());
    let win = app.window("Files", move |vcx| view(vcx, &s1, &y1));
    if let Some(w) = app.core.window_mut(win) {
        w.state_mut("path").text = st.borrow().cwd.clone();
        w.focus(Some("list"));
    }
    let (s2, y2) = (st.clone(), sys.clone());
    app.on_key(move |cx, key, mods| {
        if s2.borrow().dialog.is_some() {
            if key == keys::ESCAPE {
                cancel(cx, &s2);
                return true;
            }
            return false;
        }
        let ctrl = mods & proto::mods::CTRL != 0;
        match key {
            keys::DELETE => {
                if let Some(e) = selected(cx, &s2) {
                    ask(cx, &s2, Dialog::Delete(e.name));
                }
            }
            keys::F2 => {
                if let Some(e) = selected(cx, &s2) {
                    ask(cx, &s2, Dialog::Rename(e.name));
                }
            }
            keys::BACKSPACE => {
                let up = sys::parent(&s2.borrow().cwd);
                go(cx, &s2, &y2, &up);
            }
            keys::F5 => {
                let cwd = s2.borrow().cwd.clone();
                load(&s2, &y2, &cwd);
            }
            keys::C if ctrl => copy_selected(cx, &s2),
            keys::V if ctrl => paste(cx, &s2, &y2),
            _ => return false,
        }
        true
    });
    Ok(app)
}

fn icon_of(e: &Entry) -> &'static str {
    if e.dir {
        return "folder";
    }
    match e.name.rsplit_once('.').map(|(_, x)| x.to_lowercase()).as_deref() {
        Some("py" | "rs" | "sh" | "json" | "toml" | "vpl") => "code",
        Some("gba") => "game",
        _ => "file",
    }
}

fn view(vcx: &ecm_ui::ViewCtx, st: &Shared, sys: &SysRef) -> Element {
    let s = st.borrow();
    let compact = vcx.class == proto::SizeClass::Compact;
    let (a, b, c, d, e, f, g, h) = (st.clone(), st.clone(), st.clone(), st.clone(), st.clone(), st.clone(), st.clone(), st.clone());
    let (ya, yb, yc, yd) = (sys.clone(), sys.clone(), sys.clone(), sys.clone());
    let mut tools = row()
        .gap(4)
        .padding(4)
        .align(Align::Center)
        .child(button("Up").id("up").on_click(move |cx, _| {
            let up = sys::parent(&a.borrow().cwd);
            go(cx, &a, &ya, &up);
        }))
        .child(text_field().id("path").placeholder("/").on_submit(move |cx, ev| {
            if let EventValue::Text(t) = &ev.value {
                let cwd = b.borrow().cwd.clone();
                let p = sys::normalize(&cwd, t.trim());
                go(cx, &b, &yb, &p);
            }
        }));
    let open = move |cx: &mut Ctx, _: &ecm_ui::UiEvent| {
        if let Some(e) = selected(cx, &c) {
            open_entry(cx, &c, &yc, &e);
        }
    };
    if compact {
        let (m, ym) = (d.clone(), yd.clone());
        tools = tools.child(button("Open").id("open").on_click(open)).child(button("...").id("more").on_click(move |cx, _| {
            let (m2, ym2) = (m.clone(), ym.clone());
            cx.popup_menu(150, 24, ["New folder", "Rename", "Delete", "Copy", "Paste"], move |cx, i| match i {
                0 => ask(cx, &m2, Dialog::NewFolder),
                1 | 2 => {
                    if let Some(e) = selected(cx, &m2) {
                        ask(cx, &m2, if i == 1 { Dialog::Rename(e.name) } else { Dialog::Delete(e.name) });
                    }
                }
                3 => copy_selected(cx, &m2),
                _ => paste(cx, &m2, &ym2),
            });
        }));
    } else {
        tools = tools
            .child(button("New folder").id("mkdir").on_click(move |cx, _| ask(cx, &d, Dialog::NewFolder)))
            .child(button("Rename").id("rename").on_click(move |cx, _| {
                if let Some(en) = selected(cx, &e) {
                    ask(cx, &e, Dialog::Rename(en.name));
                }
            }))
            .child(button("Delete").id("delete").style("danger").on_click(move |cx, _| {
                if let Some(en) = selected(cx, &f) {
                    ask(cx, &f, Dialog::Delete(en.name));
                }
            }))
            .child(button("Copy").id("copy").on_click(move |cx, _| copy_selected(cx, &g)))
            .child(button("Paste").id("paste").on_click(move |cx, _| paste(cx, &h, &yd)))
            .child(button("Open").id("open").style("primary").on_click(open));
    }
    let rows: Vec<Element> = s
        .entries
        .iter()
        .map(|en| {
            let mut r = row().gap(6).align(Align::Center);
            if let Some(res) = s.icons.get(icon_of(en)) {
                r = r.child(image(*res, 16, 16));
            }
            r = r.child(label(en.name.clone()).wrap(false));
            if !compact {
                r = r.child(spacer()).child(label(if en.dir { String::from("folder") } else { sys::human_size(en.size) }).style("caption").wrap(false));
            }
            r
        })
        .collect();
    let (i1, y1) = (st.clone(), sys.clone());
    let lst = list().id("list").children(rows).on_activate(move |cx, ev| {
        if let EventValue::Index(i) = ev.value {
            let en = i1.borrow().entries.get(i).cloned();
            if let Some(en) = en {
                open_entry(cx, &i1, &y1, &en);
            }
        }
    });
    let body = if s.entries.is_empty() { column().padding(12).child(label("This folder is empty.").style("caption")) } else { column().child(lst) };
    let main = column()
        .gap(0)
        .child(tools)
        .child(scroll_view(body).id("scroll"))
        .child(row().padding(4).child(label(s.status.clone()).id("status").style("caption").wrap(false)));
    let mut root = stack().child(main);
    if let Some(dl) = &s.dialog {
        let (k1, y2, k2) = (st.clone(), sys.clone(), st.clone());
        let (title, ok, field) = match dl {
            Dialog::NewFolder => ("New folder".to_string(), "Create", true),
            Dialog::Rename(n) => (format!("Rename {n}"), "Rename", true),
            Dialog::Delete(n) => (format!("Delete {n}?"), "Delete", false),
        };
        let mut dlg = dialog(title).id("dialog");
        if field {
            let (k3, y3) = (st.clone(), sys.clone());
            dlg = dlg.child(text_field().id("name").placeholder("Name").on_submit(move |cx, _| confirm(cx, &k3, &y3)));
        } else {
            dlg = dlg.child(label("It is deleted for good (there is no recycle bin).").style("caption"));
        }
        dlg = dlg.child(
            row()
                .gap(6)
                .child(spacer())
                .child(button("Cancel").id("cancel").on_click(move |cx, _| cancel(cx, &k2)))
                .child(button(ok).id("ok").style(if field { "primary" } else { "danger" }).on_click(move |cx, _| confirm(cx, &k1, &y2))),
        );
        root = root.child(dlg);
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handlers_by_extension() {
        assert_eq!(handler_for("main.py"), Some("editor"));
        assert_eq!(handler_for("notes.TXT"), Some("editor"));
        assert_eq!(handler_for("prog.vpl"), Some("visual"));
        assert_eq!(handler_for("game.gba"), Some("gba"));
        assert_eq!(handler_for("README"), Some("editor"));
        assert_eq!(handler_for("tool.wasm"), None);
    }
}
