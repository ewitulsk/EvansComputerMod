//! `editor`: the Text Editor. A monospace text area with line numbers,
//! selection and the clipboard (Ctrl+C / X / V, Ctrl+A); open and save files
//! (Ctrl+O, Ctrl+S; Save As asks for a path until P4's file picker), find
//! (Ctrl+F, Enter / F3 for the next match), a new file (Ctrl+N), and a
//! question before closing or opening over unsaved changes.
//!
//!   editor [FILE]

use crate::sys::{self, Sys};
use ecm_ui::element::{button, column, dialog, label, row, spacer, stack, text_area, text_field, Align, Element};
use ecm_ui::window::{keys, line_col};
use ecm_ui::{App, Ctx};
use ecm_ui_proto as proto;
use std::cell::RefCell;
use std::rc::Rc;

/// A question over the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Path to open.
    Open,
    /// Path to save to.
    SaveAs,
    /// Unsaved changes before: closing, a new file, opening a file.
    Unsaved(Then),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Then {
    Close,
    New,
    Open,
}

/// What the editor knows besides the text (which lives in the text area).
#[derive(Default)]
pub struct State {
    pub path: Option<String>,
    /// The text as last loaded or saved.
    pub saved: String,
    pub find: bool,
    pub ask: Option<Ask>,
    pub status: String,
}

type Shared = Rc<RefCell<State>>;
type SysRef = Rc<RefCell<Box<dyn Sys>>>;

fn modified(cx: &Ctx, st: &Shared) -> bool {
    cx.text("doc") != st.borrow().saved
}

/// The window title: file name, a star when modified.
pub fn title_for(path: Option<&str>, modified: bool) -> String {
    let name = path.map(sys::file_name).unwrap_or("Untitled");
    format!("{}{} - Text Editor", if modified { "*" } else { "" }, name)
}

fn update_title(cx: &mut Ctx, st: &Shared) {
    let m = modified(cx, st);
    let t = title_for(st.borrow().path.as_deref(), m);
    cx.set_title(&t);
}

/// Load `path` into the editor.
pub fn open_path(cx: &mut Ctx, st: &Shared, sys: &SysRef, path: &str) {
    let path = sys::normalize("/", path);
    match sys::read_file(&**sys.borrow(), &path) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
            cx.set_text("doc", &text);
            cx.select_text("doc", 0, 0);
            let mut s = st.borrow_mut();
            s.saved = cx.text("doc");
            s.status = format!("Opened {path}");
            s.path = Some(path);
        }
        Err(e) if !sys::exists(&**sys.borrow(), &path) => {
            // A new file at that path.
            cx.set_text("doc", "");
            let mut s = st.borrow_mut();
            s.saved.clear();
            s.status = format!("New file {path} ({e})");
            s.path = Some(path);
        }
        Err(e) => st.borrow_mut().status = format!("Can't open {path}: {e}"),
    }
    cx.focus("doc");
    update_title(cx, st);
}

/// Save to the current path (or ask for one).
pub fn save(cx: &mut Ctx, st: &Shared, sys: &SysRef) -> bool {
    let Some(path) = st.borrow().path.clone() else {
        ask(cx, st, Ask::SaveAs);
        return false;
    };
    let text = cx.text("doc");
    let r = sys::write_file(&**sys.borrow(), &path, text.as_bytes());
    let ok = r.is_ok();
    {
        let mut s = st.borrow_mut();
        match r {
            Ok(()) => {
                s.saved = text;
                s.status = format!("Saved {path}");
            }
            Err(e) => s.status = format!("Can't save {path}: {e}"),
        }
    }
    update_title(cx, st);
    ok
}

fn ask(cx: &mut Ctx, st: &Shared, a: Ask) {
    let seed = match &a {
        Ask::SaveAs => st.borrow().path.clone().unwrap_or_else(|| "/untitled.txt".into()),
        Ask::Open => st.borrow().path.as_deref().map(sys::parent).map(|p| if p == "/" { p } else { p + "/" }).unwrap_or_else(|| "/".into()),
        Ask::Unsaved(_) => String::new(),
    };
    let field = matches!(a, Ask::Open | Ask::SaveAs);
    st.borrow_mut().ask = Some(a);
    if field {
        cx.set_text("askpath", &seed);
        cx.focus("askpath");
    }
}

fn new_file(cx: &mut Ctx, st: &Shared) {
    cx.set_text("doc", "");
    {
        let mut s = st.borrow_mut();
        s.path = None;
        s.saved.clear();
        s.status = "New file".into();
    }
    cx.focus("doc");
    update_title(cx, st);
}

fn then(cx: &mut Ctx, st: &Shared, t: Then) {
    match t {
        Then::Close => {
            cx.close_window();
            cx.quit();
        }
        Then::New => new_file(cx, st),
        Then::Open => ask(cx, st, Ask::Open),
    }
}

/// Answer the open question: `ok` = the primary button.
fn answer(cx: &mut Ctx, st: &Shared, sys: &SysRef, choice: u8) {
    let Some(a) = st.borrow_mut().ask.take() else { return };
    match (a, choice) {
        (Ask::Open, 1) => {
            let p = cx.text("askpath");
            open_path(cx, st, sys, p.trim());
        }
        (Ask::SaveAs, 1) => {
            let p = sys::normalize("/", cx.text("askpath").trim());
            st.borrow_mut().path = Some(p);
            save(cx, st, sys);
        }
        (Ask::Unsaved(t), 1) => {
            // Save, then go on (unless saving needs a path first).
            if save(cx, st, sys) {
                then(cx, st, t);
            }
        }
        (Ask::Unsaved(t), 2) => then(cx, st, t),
        _ => cx.focus("doc"),
    }
}

/// Find the next match of the find field after the caret (wrapping).
pub fn find_next(cx: &mut Ctx, st: &Shared) {
    let needle = cx.text("find");
    if needle.is_empty() {
        return;
    }
    let doc = cx.widget("doc").unwrap_or_default();
    let chars: Vec<char> = doc.text.chars().collect();
    let pat: Vec<char> = needle.chars().collect();
    let lower = |c: &char| c.to_lowercase().next().unwrap_or(*c);
    let hit_at = |i: usize| i + pat.len() <= chars.len() && chars[i..i + pat.len()].iter().map(lower).eq(pat.iter().map(lower));
    let start = doc.cursor;
    let found = (start..chars.len()).find(|&i| hit_at(i)).or_else(|| (0..start.min(chars.len())).find(|&i| hit_at(i)));
    match found {
        Some(i) => {
            cx.select_text("doc", i, i + pat.len());
            let (l, c) = line_col(&doc.text, i);
            st.borrow_mut().status = format!("Found at line {}, column {}", l + 1, c + 1);
        }
        None => st.borrow_mut().status = format!("'{needle}' isn't in the file"),
    }
}

/// The editor, opening `file` if given.
pub fn app(conn: Box<dyn ecm_ui::Connection>, sys: Box<dyn Sys>, file: Option<&str>) -> Result<App, ecm_ui::ConnError> {
    let mut app = App::with_connection("dev.ecm.editor", conn)?;
    let sys: SysRef = Rc::new(RefCell::new(sys));
    let st: Shared = Rc::new(RefCell::new(State::default()));
    let (s1, y1) = (st.clone(), sys.clone());
    let win = app.window(&title_for(None, false), move |vcx| view(vcx, &s1, &y1));
    {
        let mut cx = Ctx::new(&mut app.core, win);
        match file {
            Some(f) => open_path(&mut cx, &st, &sys, f),
            None => {
                cx.focus("doc");
                st.borrow_mut().status = "New file".into();
            }
        }
    }
    let (s2, y2) = (st.clone(), sys.clone());
    app.on_key(move |cx, key, mods| {
        let ctrl = mods & proto::mods::CTRL != 0;
        if key == keys::ESCAPE {
            if s2.borrow().ask.is_some() {
                s2.borrow_mut().ask = None;
                cx.focus("doc");
            } else if s2.borrow().find {
                s2.borrow_mut().find = false;
                cx.focus("doc");
            } else {
                return false;
            }
            return true;
        }
        match key {
            keys::S if ctrl && mods & proto::mods::SHIFT != 0 => ask(cx, &s2, Ask::SaveAs),
            keys::S if ctrl => {
                save(cx, &s2, &y2);
            }
            keys::O if ctrl => {
                if modified(cx, &s2) {
                    ask(cx, &s2, Ask::Unsaved(Then::Open));
                } else {
                    ask(cx, &s2, Ask::Open);
                }
            }
            keys::N if ctrl => {
                if modified(cx, &s2) {
                    ask(cx, &s2, Ask::Unsaved(Then::New));
                } else {
                    new_file(cx, &s2);
                }
            }
            keys::F if ctrl => {
                s2.borrow_mut().find = true;
                let sel = cx.selected_text("doc");
                if !sel.is_empty() && !sel.contains('\n') {
                    cx.set_text("find", &sel);
                }
                cx.focus("find");
            }
            keys::F3 => find_next(cx, &s2),
            _ => return false,
        }
        true
    });
    let s3 = st.clone();
    app.on_close(move |cx| {
        if modified(cx, &s3) {
            ask(cx, &s3, Ask::Unsaved(Then::Close));
        } else {
            cx.close_window();
            cx.quit();
        }
    });
    Ok(app)
}

fn view(vcx: &ecm_ui::ViewCtx, st: &Shared, sys: &SysRef) -> Element {
    let s = st.borrow();
    let compact = vcx.class == proto::SizeClass::Compact;
    let (a, b, ya, c, yc, d, f) = (st.clone(), st.clone(), sys.clone(), st.clone(), sys.clone(), st.clone(), st.clone());
    let mut tools = row()
        .gap(4)
        .padding(4)
        .align(Align::Center)
        .child(button("New").id("new").on_click(move |cx, _| {
            if modified(cx, &a) {
                ask(cx, &a, Ask::Unsaved(Then::New));
            } else {
                new_file(cx, &a);
            }
        }))
        .child(button("Open").id("open").on_click(move |cx, _| {
            if modified(cx, &b) {
                ask(cx, &b, Ask::Unsaved(Then::Open));
            } else {
                ask(cx, &b, Ask::Open);
            }
        }))
        .child(button("Save").id("save").style("primary").on_click(move |cx, _| {
            save(cx, &c, &yc);
        }));
    if !compact {
        tools = tools.child(button("Save as").id("saveas").on_click(move |cx, _| ask(cx, &d, Ask::SaveAs)));
    }
    tools = tools
        .child(button("Find").id("findbtn").on_click(move |cx, _| {
            f.borrow_mut().find = true;
            cx.focus("find");
        }))
        .child(spacer());
    let _ = ya;
    let (g, h) = (st.clone(), st.clone());
    let doc = text_area().id("doc").font(proto::Font::Mono).line_numbers(!compact).on_change(move |cx, _| update_title(cx, &g));
    let mut col = column().gap(0).child(tools).child(row().padding(4).grow(1).align(Align::Stretch).child(doc));
    if s.find {
        let (k, l) = (st.clone(), st.clone());
        col = col.child(
            row()
                .gap(4)
                .padding(4)
                .align(Align::Center)
                .child(label("Find").wrap(false))
                .child(text_field().id("find").placeholder("text to find").on_submit(move |cx, _| find_next(cx, &k)))
                .child(button("Next").id("next").on_click(move |cx, _| find_next(cx, &l)))
                .child(button("Close").id("closefind").on_click(move |cx, _| {
                    h.borrow_mut().find = false;
                    cx.focus("doc");
                })),
        );
    }
    let pos = vcx
        .state("doc")
        .map(|w| {
            let (l, c) = line_col(&w.text, w.cursor);
            format!("Ln {}, Col {}", l + 1, c + 1)
        })
        .unwrap_or_default();
    col = col.child(
        row()
            .gap(8)
            .padding(4)
            .child(label(s.status.clone()).id("status").style("caption").wrap(false))
            .child(spacer())
            .child(label(pos).id("pos").style("caption").wrap(false)),
    );
    let mut root = stack().child(col);
    if let Some(a) = &s.ask {
        let (title, primary, second): (String, &str, Option<&str>) = match a {
            Ask::Open => ("Open a file".into(), "Open", None),
            Ask::SaveAs => ("Save as".into(), "Save", None),
            Ask::Unsaved(_) => (format!("Save changes to {}?", s.path.as_deref().map(sys::file_name).unwrap_or("Untitled")), "Save", Some("Don't save")),
        };
        let (p1, y1, p2, y2, p3, y3) = (st.clone(), sys.clone(), st.clone(), sys.clone(), st.clone(), sys.clone());
        let mut dlg = dialog(title).id("ask");
        if matches!(a, Ask::Open | Ask::SaveAs) {
            let (p4, y4) = (st.clone(), sys.clone());
            dlg = dlg.child(text_field().id("askpath").width(260).on_submit(move |cx, _| answer(cx, &p4, &y4, 1)));
        }
        let mut btns = row().gap(6).child(spacer()).child(button("Cancel").id("askcancel").on_click(move |cx, _| answer(cx, &p3, &y3, 0)));
        if let Some(sec) = second {
            btns = btns.child(button(sec).id("asknosave").style("danger").on_click(move |cx, _| answer(cx, &p2, &y2, 2)));
        }
        btns = btns.child(button(primary).id("askok").style("primary").on_click(move |cx, _| answer(cx, &p1, &y1, 1)));
        root = root.child(dlg.child(btns));
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(title_for(None, false), "Untitled - Text Editor");
        assert_eq!(title_for(Some("/home/a.py"), true), "*a.py - Text Editor");
    }
}
