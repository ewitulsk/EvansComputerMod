//! The "open with" chooser (UI Protocol §12 "Open with"). When
//! `AppManager.open(path)` finds several apps for a file type and no default
//! (`AppException.CHOOSE`), the compositor starts
//!
//! ```text
//! chooser --req <token> --path <path>
//! ```
//!
//! which lists the handlers (`app_ctl handlers <path>`), offers "Always use
//! this app for .ext files", and answers `chosen <token> <app id> [always]` or
//! `chosen <token> cancel` on `/dev/uictl`. The compositor then opens the
//! file with that app (a one-file grant) and, with `always`, makes it the default.

use crate::appctl::{self, AppCtl, Handler};
use crate::uictl::{self, Answer};
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::element::{button, checkbox, column, label, list, row, spacer, Element};
use ecm_ui::{App, ViewCtx};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChooseRequest {
    pub token: String,
    pub path: String,
}

pub fn parse_args(args: &[String]) -> Result<ChooseRequest, String> {
    let token = uictl::arg(args, "--req").ok_or("--req <token> is missing")?;
    let path = uictl::arg(args, "--path").ok_or("--path <file> is missing")?;
    if !path.starts_with('/') {
        return Err("--path must be absolute".into());
    }
    Ok(ChooseRequest { token, path })
}

pub struct Chooser {
    pub req: ChooseRequest,
    pub handlers: Vec<Handler>,
    pub selected: Option<usize>,
    pub always: bool,
    pub error: String,
    pub done: Option<Option<String>>,
}

impl Chooser {
    pub fn new(req: ChooseRequest, ctl: &mut dyn AppCtl) -> Chooser {
        let (handlers, error) = match ctl.call(&["handlers", &req.path]) {
            Ok(r) => (appctl::parse_handlers(&r), String::new()),
            Err(e) => (Vec::new(), e),
        };
        let selected = handlers.iter().position(|h| h.is_default).or(if handlers.is_empty() { None } else { Some(0) });
        let error = if handlers.is_empty() && error.is_empty() { "No installed app opens this kind of file.".into() } else { error };
        Chooser { req, handlers, selected, always: false, error, done: None }
    }

    pub fn file_name(&self) -> &str {
        self.req.path.rsplit('/').next().unwrap_or(&self.req.path)
    }

    pub fn extension(&self) -> Option<String> {
        appctl::extension(&self.req.path)
    }

    pub fn select(&mut self, i: usize) {
        if i < self.handlers.len() {
            self.selected = Some(i);
        }
    }

    pub fn open(&mut self) {
        match self.selected.and_then(|i| self.handlers.get(i)) {
            Some(h) => self.done = Some(Some(h.id.clone())),
            None => self.error = "Choose an app first.".into(),
        }
    }

    pub fn cancel(&mut self) {
        self.done = Some(None);
    }

    pub fn answer_line(&self) -> Option<String> {
        // "Always" only makes sense for a file type.
        let always = self.always && self.extension().is_some();
        self.done.as_ref().map(|d| uictl::chosen_line(&self.req.token, d.as_deref(), always))
    }
}

type C = Rc<RefCell<Chooser>>;

pub fn view(c: &C, _cx: &ViewCtx) -> Element {
    let ch = c.borrow();
    let s = c.clone();
    let mut l = list().id("apps").children(ch.handlers.iter().map(|h| {
        row().gap(6).child(label(h.name.clone()).wrap(false)).child(spacer()).child(label(if h.is_default { "default" } else { "" }).style("caption").wrap(false))
    }));
    if let Some(i) = ch.selected {
        l = l.selected(i);
    }
    l = l.on_select(move |cx, _| {
        let i = cx.selected("apps").unwrap_or(0);
        s.borrow_mut().select(i);
    });
    let mut col = column().gap(8).padding(10).child(label(format!("Open \"{}\" with", ch.file_name())).style("title")).child(l);
    if let Some(ext) = ch.extension() {
        let a = c.clone();
        col = col.child(checkbox(format!("Always use this app for {ext} files")).id("always").checked(ch.always).on_change(move |cx, _| {
            let on = cx.checked("always");
            a.borrow_mut().always = on;
        }));
    }
    if !ch.error.is_empty() {
        col = col.child(label(ch.error.clone()).color(crate::widgets::RED).id("error"));
    }
    let cancel = c.clone();
    let ok = c.clone();
    col.child(
        row()
            .gap(8)
            .child(spacer())
            .child(button("Cancel").id("cancel").on_click(move |_, _| cancel.borrow_mut().cancel()))
            .child(button("Open").style("primary").id("open").enabled(ch.selected.is_some()).on_click(move |_, _| ok.borrow_mut().open())),
    )
}

pub struct ChooserApp<A: Answer> {
    pub app: App,
    pub state: C,
    pub answer: A,
}

impl<A: Answer> ChooserApp<A> {
    pub fn new(conn: Box<dyn Connection>, req: ChooseRequest, ctl: &mut dyn AppCtl, answer: A) -> Result<Self, ConnError> {
        let mut app = App::with_connection("chooser", conn)?;
        let state = Rc::new(RefCell::new(Chooser::new(req, ctl)));
        let s = state.clone();
        app.window("Open with", move |cx| view(&s, cx));
        let c = state.clone();
        app.on_close(move |_| c.borrow_mut().cancel());
        Ok(ChooserApp { app, state, answer })
    }

    pub fn step(&mut self, timeout_ms: i32) -> bool {
        let alive = self.app.step(timeout_ms);
        let line = self.state.borrow().answer_line();
        if let Some(l) = line {
            let _ = self.answer.send(&l);
            return false;
        }
        if !alive {
            let l = uictl::chosen_line(&self.state.borrow().req.token, None, false);
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
    use crate::appctl::FakeCtl;

    fn ctl() -> FakeCtl {
        let mut f = FakeCtl::default();
        f.add("dev.ecm.editor", "Text Editor", "1.0.0", &[], &[".txt", ".md"]);
        f.add("dev.ecm.notes", "Notes", "1.0.0", &[], &[".txt"]);
        f.add("dev.ecm.paint", "Paint", "1.0.0", &[], &[".png"]);
        f
    }

    fn req(path: &str) -> ChooseRequest {
        ChooseRequest { token: "c7".into(), path: path.into() }
    }

    #[test]
    fn lists_the_handlers_and_answers() {
        let mut f = ctl();
        let mut c = Chooser::new(req("/home/todo.txt"), &mut f);
        assert_eq!(c.handlers.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), vec!["Text Editor", "Notes"]);
        assert_eq!(c.selected, Some(0));
        c.select(1);
        c.always = true;
        c.open();
        assert_eq!(c.answer_line().unwrap(), "chosen c7 dev.ecm.notes always");
        let mut c = Chooser::new(req("/home/todo.txt"), &mut f);
        c.cancel();
        assert_eq!(c.answer_line().unwrap(), "chosen c7 cancel");
    }

    #[test]
    fn the_default_is_preselected_and_unknown_types_say_so() {
        let mut f = ctl();
        f.defaults.insert(".txt".into(), "dev.ecm.notes".into());
        let c = Chooser::new(req("/home/a.TXT"), &mut f);
        assert_eq!(c.handlers[c.selected.unwrap()].id, "dev.ecm.notes");
        let mut c = Chooser::new(req("/home/a.zip"), &mut f);
        assert!(c.handlers.is_empty() && c.error.contains("No installed app"));
        c.open();
        assert!(c.done.is_none());
        // "Always" is dropped for files without an extension.
        let mut c = Chooser::new(req("/home/README"), &mut f);
        c.handlers.push(Handler { id: "dev.x".into(), name: "X".into(), is_default: false });
        c.select(0);
        c.always = true;
        c.open();
        assert_eq!(c.answer_line().unwrap(), "chosen c7 dev.x");
    }

    #[test]
    fn args() {
        let a: Vec<String> = ["--req", "k", "--path", "/home/a b.txt"].iter().map(|s| s.to_string()).collect();
        assert_eq!(parse_args(&a).unwrap().path, "/home/a b.txt");
        assert!(parse_args(&["--req".into(), "k".into(), "--path".into(), "rel".into()]).is_err());
    }
}
