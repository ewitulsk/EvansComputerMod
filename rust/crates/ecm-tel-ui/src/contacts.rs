//! **Contacts** (`dev.ecm.contacts`, program `contacts`): the phone's address book
//! (`/home/.contacts/contacts.vcf` through [`Book`] over [`Files`]; the `contacts`
//! permission).
//!
//! Screens:
//! * **List**: My card (this phone's own number, from telephonyd's `status` on
//!   [`ecm_tel::ctl::SMS_PORT`]), a search field ([`Book::search`]), the A-Z index
//!   ([`Book::letters`]; a letter shows only the names under it, again for all),
//!   the contacts (avatar or photo, name, first number, a star for favourites),
//!   Add and Import (a `.vcf` chosen with the file picker).
//! * **Contact**: the big avatar or photo, the name, each number with its label
//!   and Call / Text (they start Phone with `--call <n>` and Messages with
//!   `--to <n>`), emails, the note; Edit, Favourite, Share (writes
//!   `/home/Documents/<name>.vcf`; then Send in Messages starts Messages with
//!   `--vcard <path>`) and Delete (asks first).
//! * **Edit / Add**: name, numbers with labels (mobile, home, work; rows added
//!   and removed), email, note, photo (a `.png` chosen with the file picker);
//!   Save needs a name or a number.
//!
//! A `.vcf` opened with Contacts (`OpenFile`) or given as the program's
//! argument is imported. `--add <number>` opens Add with the number,
//! `--show <number>` that contact (Add when the number is unknown).
//!
//! * **compact** (< 400 lu) and **medium**: one screen at a time.
//! * **wide** (>= 800 lu): the list beside the contact or the form.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::book::Book;
use crate::widgets::{avatar, message, pill, AMBER};
use crate::{pretty, Files, Tel, CONTACTS_ID, DIALER_ID, MESSAGES_ID, POLL_MS};
use ecm_tel::vcard::{Contact, Tel as Number};
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::element::{button, column, dialog, image, label, list, panel, row, scroll_view, spacer, stack, text_area, text_field, Element};
use ecm_ui::{Align, App, ViewCtx};
use ecm_ui_proto as proto;
use ecm_ui_proto::{Message, SizeClass};

/// Where Share writes a contact's card.
pub const DOCUMENTS: &str = "/home/Documents";
/// `FilePicked.req` of the photo picker and of Import.
pub const PICK_PHOTO: u32 = 1;
pub const PICK_IMPORT: u32 = 2;
/// How often My card asks telephonyd for the number (a SIM may be put in), ms.
pub const STATUS_MS: u64 = 10_000;

/// The labels a number can have: (vCard `TYPE`, what the app shows).
pub const LABELS: [(&str, &str); 3] = [("CELL", "mobile"), ("HOME", "home"), ("WORK", "work")];

/// How a vCard label reads (`CELL` is "mobile").
pub fn label_name(l: &str) -> String {
    match l.to_ascii_uppercase().as_str() {
        "" | "CELL" | "VOICE" => "mobile".into(),
        "HOME" => "home".into(),
        "WORK" => "work".into(),
        "MAIN" => "main".into(),
        o => o.to_lowercase(),
    }
}

/// The label after `l` in [`LABELS`] (the label button cycles).
pub fn next_label(l: &str) -> String {
    let i = LABELS.iter().position(|(t, _)| t.eq_ignore_ascii_case(l)).map_or(0, |i| (i + 1) % LABELS.len());
    LABELS[i].0.into()
}

/// The name a contact is listed under (its number when it has no name).
pub fn shown_name(c: &Contact) -> String {
    if !c.name.trim().is_empty() {
        c.name.clone()
    } else {
        c.primary().map(pretty).unwrap_or_else(|| "No name".into())
    }
}

/// The letter of the A-Z index a contact is under.
pub fn letter_of(c: &Contact) -> char {
    c.name.chars().next().map(|x| x.to_ascii_uppercase()).filter(|x| x.is_ascii_alphabetic()).unwrap_or('#')
}

/// A file name made from a contact's name (`Alex Smith` -> `Alex Smith.vcf`).
pub fn vcf_name(name: &str) -> String {
    let n: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
    let n = n.trim().trim_matches('.').to_string();
    format!("{}.vcf", if n.is_empty() { "contact" } else { &n })
}

/// Width and height of a PNG, from its header.
pub fn png_size(b: &[u8]) -> Option<(i32, i32)> {
    if b.len() < 24 || &b[..8] != b"\x89PNG\r\n\x1a\n" || &b[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]) as i32;
    let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]) as i32;
    (w > 0 && h > 0 && w <= 4096 && h <= 4096).then_some((w, h))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    List,
    /// A contact's page (by uid).
    Contact(String),
    /// The Edit / Add form ([`Model::draft`]).
    Edit,
}

/// The Edit / Add form's fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    /// The contact being edited (uid empty for Add): what the form doesn't show is kept.
    pub base: Contact,
    pub name: String,
    pub tels: Vec<Number>,
    pub email: String,
    pub note: String,
    pub photo: String,
}

impl Draft {
    pub fn of(c: &Contact) -> Draft {
        let mut tels = c.tels.clone();
        if tels.is_empty() {
            tels.push(Number { label: "CELL".into(), number: String::new() });
        }
        Draft { base: c.clone(), name: c.name.clone(), tels, email: c.emails.first().cloned().unwrap_or_default(), note: c.note.clone(), photo: c.photo.clone() }
    }

    /// The contact to save, or why not.
    pub fn contact(&self) -> Result<Contact, String> {
        let mut c = self.base.clone();
        c.tels = self.tels.iter().filter(|t| !t.number.trim().is_empty()).map(|t| Number { label: t.label.clone(), number: t.number.trim().to_string() }).collect();
        c.name = self.name.trim().to_string();
        if c.name.is_empty() && c.tels.is_empty() {
            return Err("Give the contact a name or a number".into());
        }
        if c.name.is_empty() {
            c.name = pretty(&c.tels[0].number);
        }
        let email = self.email.trim();
        match (email.is_empty(), c.emails.is_empty()) {
            (false, true) => c.emails.push(email.into()),
            (false, false) => c.emails[0] = email.into(),
            (true, false) => {
                c.emails.remove(0);
            }
            (true, true) => {}
        }
        c.note = self.note.trim_end().to_string();
        c.photo = self.photo.trim().to_string();
        Ok(c)
    }
}

/// Work for the program loop (it has the files).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Save,
    Delete(String),
    Favourite(String),
    Share(String),
    Import(String),
}

#[derive(Debug)]
pub struct Model {
    pub book: Book,
    pub screen: Screen,
    pub query: String,
    /// The A-Z index's letter, when one is chosen.
    pub letter: Option<char>,
    pub draft: Option<Draft>,
    /// Delete is waiting for "are you sure".
    pub confirm_delete: bool,
    /// This phone's number (None: no SIM / telephonyd not answering).
    pub my_number: Option<String>,
    pub message: Option<(bool, String)>,
    /// The last card Share wrote (Send in Messages sends it).
    pub shared: Option<String>,
    /// Photos by path: the image resource and its size (None: not a readable PNG).
    pub photos: HashMap<String, Option<u32>>,
    pub ops: Vec<Op>,
    pub dirty: bool,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            book: Book::default(),
            screen: Screen::List,
            query: String::new(),
            letter: None,
            draft: None,
            confirm_delete: false,
            my_number: None,
            message: None,
            shared: None,
            photos: HashMap::new(),
            ops: Vec::new(),
            dirty: true,
        }
    }
}

impl Model {
    /// The contacts the list shows (the search, then the letter).
    pub fn visible(&self) -> Vec<&Contact> {
        self.book.search(&self.query).into_iter().filter(|c| self.letter.is_none_or(|l| letter_of(c) == l)).collect()
    }

    pub fn current(&self) -> Option<&Contact> {
        match &self.screen {
            Screen::Contact(uid) => self.book.by_uid(uid),
            _ => None,
        }
    }

    pub fn show(&mut self, uid: &str) {
        self.screen = Screen::Contact(uid.into());
        self.confirm_delete = false;
        self.shared = None;
        self.message = None;
        self.dirty = true;
    }

    pub fn back(&mut self) {
        self.screen = Screen::List;
        self.draft = None;
        self.confirm_delete = false;
        self.message = None;
        self.dirty = true;
    }

    /// Add: an empty form (with `number` when given).
    pub fn add(&mut self, number: &str) {
        let mut c = Contact::default();
        c.tels.push(Number { label: "CELL".into(), number: number.trim().into() });
        self.draft = Some(Draft::of(&c));
        self.screen = Screen::Edit;
        self.message = None;
        self.dirty = true;
    }

    pub fn edit(&mut self, uid: &str) {
        if let Some(c) = self.book.by_uid(uid) {
            self.draft = Some(Draft::of(c));
            self.screen = Screen::Edit;
            self.message = None;
            self.dirty = true;
        }
    }

    /// Cancel on the form: back where it came from.
    pub fn cancel(&mut self) {
        let uid = self.draft.take().map(|d| d.base.uid).unwrap_or_default();
        if self.book.by_uid(&uid).is_some() {
            self.show(&uid);
        } else {
            self.back();
        }
    }

    /// `--show <number>`: that contact, or Add with the number.
    pub fn show_number(&mut self, number: &str) {
        match self.book.find(number).map(|c| c.uid.clone()) {
            Some(uid) => self.show(&uid),
            None => self.add(number),
        }
    }

    /// Tap on a letter of the index (the same letter again: all).
    pub fn tap_letter(&mut self, l: char) {
        self.letter = if self.letter == Some(l) { None } else { Some(l) };
        self.dirty = true;
    }

    fn draft_mut(&mut self) -> Option<&mut Draft> {
        self.dirty = true;
        self.draft.as_mut()
    }
}

pub type M = Rc<RefCell<Model>>;

/// How the program was started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    List,
    Add(String),
    Show(String),
    Import(String),
}

impl Start {
    /// `--add <number>`, `--show <number>`, `<file>.vcf`, else the list.
    pub fn parse(args: &[String]) -> Start {
        match args {
            [a, n, ..] if a == "--add" => Start::Add(n.clone()),
            [a] if a == "--add" => Start::Add(String::new()),
            [a, n, ..] if a == "--show" => Start::Show(n.clone()),
            [p, ..] if p.to_ascii_lowercase().ends_with(".vcf") => Start::Import(p.clone()),
            _ => Start::List,
        }
    }
}

pub struct ContactsApp<T: Tel, F: Files> {
    pub app: App,
    pub model: M,
    pub tel: T,
    pub files: F,
    pub window: u32,
    /// The book file as last read or written (a change by another app reloads it).
    last_file: Option<Vec<u8>>,
    last_poll: Option<u64>,
    last_status: Option<u64>,
}

impl<T: Tel, F: Files> ContactsApp<T, F> {
    pub fn new(conn: Box<dyn Connection>, tel: T, mut files: F) -> Result<Self, ConnError> {
        let mut app = App::with_connection(CONTACTS_ID, conn)?;
        let model: M = Rc::new(RefCell::new(Model { book: Book::load(&mut files), ..Default::default() }));
        let last_file = files.read(crate::book::PATH);
        let v = model.clone();
        let window = app.window("Contacts", move |cx| view(&v, cx));
        let mm = model.clone();
        app.on_message(move |_cx, msg| {
            let mut m = mm.borrow_mut();
            match msg {
                Message::OpenFile(o) if o.path.to_ascii_lowercase().ends_with(".vcf") => m.ops.push(Op::Import(o.path.clone())),
                Message::FilePicked(p) if p.path.is_empty() => {}
                Message::FilePicked(p) if p.req == PICK_IMPORT => m.ops.push(Op::Import(p.path.clone())),
                Message::FilePicked(p) if p.req == PICK_PHOTO => {
                    if let Some(d) = m.draft_mut() {
                        d.photo = p.path.clone();
                    }
                }
                _ => {}
            }
            m.dirty = true;
        });
        Ok(ContactsApp { app, model, tel, files, window, last_file, last_poll: None, last_status: None })
    }

    /// Apply the program's arguments ([`Start::parse`]).
    pub fn start(&mut self, s: Start) {
        let mut m = self.model.borrow_mut();
        match s {
            Start::List => {}
            Start::Add(n) => m.add(&n),
            Start::Show(n) => m.show_number(&n),
            Start::Import(p) => m.ops.push(Op::Import(p)),
        }
        m.dirty = true;
    }

    pub fn step(&mut self, timeout_ms: i32, now_ms: u64) -> bool {
        let wait = if self.model.borrow().ops.is_empty() { if timeout_ms < 0 { 250 } else { timeout_ms.min(250) } } else { 0 };
        if !self.app.step(wait) {
            return false;
        }
        self.process(now_ms);
        !self.app.core.is_closed()
    }

    pub fn run(&mut self) {
        let start = std::time::Instant::now();
        while self.step(-1, start.elapsed().as_millis() as u64) {}
    }

    pub fn process(&mut self, now_ms: u64) {
        let ops: Vec<Op> = std::mem::take(&mut self.model.borrow_mut().ops);
        for op in ops {
            self.run_op(op);
        }
        if self.last_status.is_none_or(|t| now_ms.saturating_sub(t) >= STATUS_MS) {
            self.last_status = Some(now_ms);
            let n = match self.tel.call(ecm_tel::ctl::SMS_PORT, "status") {
                Ok(Ok(body)) => Some(ecm_tel::ctl::value(&body, "msisdn").to_string()).filter(|n| !n.is_empty() && n != "-"),
                _ => None,
            };
            let mut m = self.model.borrow_mut();
            if m.my_number != n {
                m.my_number = n;
                m.dirty = true;
            }
        }
        if self.last_poll.is_none_or(|t| now_ms.saturating_sub(t) >= POLL_MS) {
            self.last_poll = Some(now_ms);
            self.reload_if_changed();
        }
        self.load_photos();
        let dirty = std::mem::take(&mut self.model.borrow_mut().dirty);
        if dirty {
            self.app.core.mark_all_dirty();
            let _ = self.app.core.render();
        }
    }

    /// Another app (Messages' "add to contacts", a script) changed the file.
    fn reload_if_changed(&mut self) {
        let now = self.files.read(crate::book::PATH);
        if now == self.last_file {
            return;
        }
        self.last_file = now;
        let book = Book::load(&mut self.files);
        let mut m = self.model.borrow_mut();
        if m.book != book {
            m.book = book;
            if matches!(&m.screen, Screen::Contact(u) if m.book.by_uid(u).is_none()) {
                m.screen = Screen::List;
            }
            m.dirty = true;
        }
    }

    fn save(&mut self) -> Result<(), String> {
        let r = self.model.borrow().book.save(&mut self.files);
        self.last_file = self.files.read(crate::book::PATH);
        r
    }

    fn say(&self, ok: bool, text: String) {
        let mut m = self.model.borrow_mut();
        m.message = Some((ok, text));
        m.dirty = true;
    }

    fn run_op(&mut self, op: Op) {
        match op {
            Op::Save => {
                let c = {
                    let m = self.model.borrow();
                    let Some(d) = &m.draft else { return };
                    d.contact()
                };
                match c {
                    Err(e) => self.say(false, e),
                    Ok(c) => {
                        let name = c.name.clone();
                        let uid = self.model.borrow_mut().book.put(c);
                        match self.save() {
                            Ok(()) => {
                                let mut m = self.model.borrow_mut();
                                m.draft = None;
                                m.show(&uid);
                                m.message = Some((true, format!("Saved {name}")));
                            }
                            Err(e) => self.say(false, format!("Could not save the contacts: {e}")),
                        }
                    }
                }
            }
            Op::Delete(uid) => {
                let name = self.model.borrow().book.by_uid(&uid).map(shown_name).unwrap_or_default();
                if !self.model.borrow_mut().book.remove(&uid) {
                    return;
                }
                match self.save() {
                    Ok(()) => {
                        let mut m = self.model.borrow_mut();
                        m.back();
                        m.message = Some((true, format!("Deleted {name}")));
                    }
                    Err(e) => self.say(false, format!("Could not save the contacts: {e}")),
                }
            }
            Op::Favourite(uid) => {
                let on = {
                    let mut m = self.model.borrow_mut();
                    let Some(mut c) = m.book.by_uid(&uid).cloned() else { return };
                    c.favourite = !c.favourite;
                    let on = c.favourite;
                    m.book.put(c);
                    on
                };
                match self.save() {
                    Ok(()) => self.say(true, if on { "Added to Favourites".into() } else { "Removed from Favourites".into() }),
                    Err(e) => self.say(false, format!("Could not save the contacts: {e}")),
                }
            }
            Op::Share(uid) => {
                let Some(c) = self.model.borrow().book.by_uid(&uid).cloned() else { return };
                let path = format!("{DOCUMENTS}/{}", vcf_name(&shown_name(&c)));
                match self.files.write(&path, c.to_vcard().as_bytes()) {
                    Ok(()) => {
                        self.model.borrow_mut().shared = Some(path.clone());
                        self.say(true, format!("Saved the card to {path}"));
                    }
                    Err(e) => self.say(false, format!("Could not write {path}: {e}")),
                }
            }
            Op::Import(path) => {
                let Some(bytes) = self.files.read(&path) else {
                    self.say(false, format!("Could not read {path}"));
                    return;
                };
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let before = self.model.borrow().book.contacts.len();
                let r = self.model.borrow_mut().book.import(&text);
                match r {
                    Err(e) => self.say(false, format!("{path} is not a vCard file ({e})")),
                    Ok(0) => self.say(false, format!("No contacts in {path}")),
                    Ok(n) => {
                        let added = self.model.borrow().book.contacts.len().saturating_sub(before);
                        let merged = n - added.min(n);
                        match self.save() {
                            Ok(()) => {
                                let mut m = self.model.borrow_mut();
                                m.back();
                                m.message = Some((true, format!("Imported {n} contact{}: {added} added, {merged} merged", if n == 1 { "" } else { "s" })));
                            }
                            Err(e) => self.say(false, format!("Could not save the contacts: {e}")),
                        }
                    }
                }
            }
        }
    }

    /// Upload the pictures of contacts that have a PNG photo (once per path).
    fn load_photos(&mut self) {
        let paths: Vec<String> = {
            let m = self.model.borrow();
            m.book.contacts.iter().map(|c| c.photo.clone()).chain(m.draft.iter().map(|d| d.photo.clone())).filter(|p| !p.is_empty() && !m.photos.contains_key(p)).collect()
        };
        for p in paths {
            let res = self.files.read(&p).and_then(|b| png_size(&b).map(|(w, h)| (w, h, b))).and_then(|(w, h, b)| self.app.core.image_png(w, h, &b).ok());
            let mut m = self.model.borrow_mut();
            m.photos.insert(p, res);
            m.dirty = true;
        }
    }
}

// ------------------------------------------------------------ the window

const TITLE: i32 = 20;

/// A contact's picture: the photo when it is a readable PNG, else the initials.
fn picture(st: &Model, c: &Contact, size: i32) -> Element {
    picture_of(st, &c.photo, &c.initials(), size)
}

fn picture_of(st: &Model, photo: &str, initials: &str, size: i32) -> Element {
    match st.photos.get(photo) {
        Some(Some(res)) => panel().radius((size / 2) as u16).width(size).height(size).child(image(*res, size, size)),
        _ => avatar(initials, size),
    }
}

/// The favourite tag on list rows and the contact page.
fn star() -> Element {
    pill("Fav", AMBER)
}

pub fn view(m: &M, cx: &ViewCtx) -> Element {
    let st = m.borrow();
    let pad = if cx.class == SizeClass::Compact { 6 } else { 10 };
    let body_h = cx.height - 2 * pad - TITLE - 8;
    let body = if cx.class == SizeClass::Wide {
        let right = match &st.screen {
            Screen::Edit => edit_form(m, &st, cx),
            Screen::Contact(_) if st.current().is_some() => contact_page(m, &st, cx),
            _ => column().gap(6).child(label("Choose a contact, or Add one.").style("caption")).child(message(cx, &st.message)),
        };
        row()
            .gap(12)
            .align(Align::Start)
            .child(panel().fill(cx.theme.surface).radius(6).padding(8).child(column().width(320).child(list_screen(m, &st, cx, body_h - 16))))
            .child(scroll_view(right).id("pane").height(body_h).grow(1))
    } else {
        match &st.screen {
            Screen::Edit => scroll_view(edit_form(m, &st, cx)).id("edit_scroll").height(body_h),
            Screen::Contact(_) if st.current().is_some() => scroll_view(contact_page(m, &st, cx)).id("page_scroll").height(body_h),
            _ => list_screen(m, &st, cx, body_h),
        }
    };
    let root = column().gap(8).padding(pad).child(header(m, &st, cx)).child(body);
    let mut s = stack().child(root);
    if st.confirm_delete {
        if let Some(c) = st.current() {
            let (no, yes, uid) = (m.clone(), m.clone(), c.uid.clone());
            s = s.child(
                dialog(format!("Delete {}?", shown_name(c))).id("confirm").child(label("The contact is removed from this phone.").style("caption")).child(
                    row()
                        .gap(6)
                        .child(spacer())
                        .child(button("Cancel").id("delete_cancel").on_click(move |_, _| {
                            let mut m = no.borrow_mut();
                            m.confirm_delete = false;
                            m.dirty = true;
                        }))
                        .child(button("Delete").id("delete_yes").style("danger").on_click(move |_, _| {
                            let mut m = yes.borrow_mut();
                            m.confirm_delete = false;
                            m.ops.push(Op::Delete(uid.clone()));
                        })),
                ),
            );
        }
    }
    s
}

fn header(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    let wide = cx.class == SizeClass::Wide;
    let mut r = row().gap(6).align(Align::Center);
    if !wide && st.screen != Screen::List {
        let b = m.clone();
        r = r.child(button("Back").id("back").on_click(move |_, _| {
            let mut m = b.borrow_mut();
            if m.screen == Screen::Edit {
                m.cancel();
            } else {
                m.back();
            }
        }));
    }
    let title = match (&st.screen, wide) {
        (Screen::Edit, false) if st.draft.as_ref().is_some_and(|d| d.base.uid.is_empty()) => "New contact",
        (Screen::Edit, false) => "Edit contact",
        _ => "Contacts",
    };
    r = r.child(label(title).style("title").wrap(false)).child(spacer());
    if wide || st.screen == Screen::List {
        let add = m.clone();
        r = r
            .child(button("Import").id("import").on_click(move |cx, _| {
                cx.send(Message::PickFile(proto::PickFile { req: PICK_IMPORT, mode: proto::PickMode::Open, types: vec!["vcf".into()], suggested: String::new() }));
            }))
            .child(button("Add").id("add").style("primary").on_click(move |_, _| add.borrow_mut().add("")));
    }
    r
}

fn my_card(st: &Model) -> Element {
    let number = st.my_number.as_deref().map(pretty).unwrap_or_else(|| "No number (insert a SIM)".into());
    row()
        .gap(8)
        .align(Align::Center)
        .padding(2)
        .child(avatar("ME", 32))
        .child(column().gap(1).grow(1).child(label("My card").wrap(false)).child(label(number).id("my_number").style("caption").wrap(false)))
}

fn contact_row(st: &Model, c: &Contact) -> Element {
    let number = c.primary().map(pretty).unwrap_or_default();
    let mut r = row()
        .gap(8)
        .align(Align::Center)
        .child(picture(st, c, 28))
        .child(column().gap(1).grow(1).child(label(shown_name(c)).wrap(false)).child(label(number).style("caption").wrap(false)));
    if c.favourite {
        r = r.child(star());
    }
    r
}

fn list_screen(m: &M, st: &Model, cx: &ViewCtx, h: i32) -> Element {
    let q = m.clone();
    let search = text_field().id("search").placeholder("Search names and numbers").text(st.query.clone()).grow(1).on_change(move |cx, _| {
        let t = cx.text("search");
        let mut m = q.borrow_mut();
        m.query = t;
        m.dirty = true;
    });
    let shown = st.visible();
    let uids: Vec<String> = shown.iter().map(|c| c.uid.clone()).collect();
    let sel = match &st.screen {
        Screen::Contact(u) => uids.iter().position(|x| x == u),
        _ => None,
    };
    let rows: Element = if shown.is_empty() {
        let why = if st.book.contacts.is_empty() {
            "No contacts yet. Add one, or Import a .vcf file."
        } else {
            "No contacts match."
        };
        label(why).style("caption")
    } else {
        let mm = m.clone();
        let mut l = list().id("contacts").children(shown.iter().map(|c| contact_row(st, c))).on_select(move |cx, _| {
            if let Some(uid) = cx.selected("contacts").and_then(|i| uids.get(i).cloned()) {
                mm.borrow_mut().show(&uid);
            }
        });
        if let Some(i) = sel {
            l = l.selected(i);
        }
        l
    };
    // the A-Z index
    let mut az = column().gap(2).align(Align::Center).width(16);
    for l in st.book.letters() {
        let t = m.clone();
        let on = st.letter == Some(l);
        let mut e = label(l.to_string()).id(format!("az_{}", if l == '#' { "num".to_string() } else { l.to_string() })).size(8).wrap(false).on_click(move |_, _| t.borrow_mut().tap_letter(l));
        if on {
            e = e.color(cx.theme.accent);
        }
        az = az.child(e);
    }
    let mut top = column().gap(6).child(my_card(st)).child(row().gap(6).child(search));
    if let Some(l) = st.letter {
        top = top.child(label(format!("Names under {l} (tap {l} again for all)")).style("caption").wrap(false));
    }
    let used = 32 + 6 + 24 + 6 + if st.letter.is_some() { 16 } else { 0 } + 20;
    let body = row().gap(4).align(Align::Start).child(scroll_view(rows).id("list_scroll").height((h - used).max(60)).grow(1)).child(az);
    let c = column().gap(6).child(top).child(body);
    // wide: the message is in the right-hand pane
    if cx.class == SizeClass::Wide {
        c
    } else {
        c.child(message(cx, &st.message))
    }
}

fn contact_page(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    let Some(c) = st.current() else { return spacer() };
    let compact = cx.class == SizeClass::Compact;
    let big = if compact { 64 } else { 88 };
    let mut name_row = row().gap(6).align(Align::Center).child(label(shown_name(c)).style("title").wrap(false));
    if c.favourite {
        name_row = name_row.child(star());
    }
    // numbers, each with Call and Text
    let mut nums = column().gap(6);
    if c.tels.is_empty() {
        nums = nums.child(label("No numbers").style("caption"));
    }
    for (i, t) in c.tels.iter().enumerate() {
        let (n1, n2) = (t.number.clone(), t.number.clone());
        nums = nums.child(
            row()
                .gap(6)
                .align(Align::Center)
                .child(column().gap(1).grow(1).child(label(label_name(&t.label)).style("caption").wrap(false)).child(label(pretty(&t.number)).wrap(false)))
                .child(button("Call").id(format!("call_{i}")).style("primary").on_click(move |cx, _| cx.launch(DIALER_ID, &["--call".into(), n1.clone()])))
                .child(button("Text").id(format!("text_{i}")).on_click(move |cx, _| cx.launch(MESSAGES_ID, &["--to".into(), n2.clone()]))),
        );
    }
    let mut info = column().gap(8).grow(1).child(name_row).child(nums);
    for e in &c.emails {
        info = info.child(column().gap(1).child(label("email").style("caption")).child(label(e.clone()).wrap(false)));
    }
    if !c.note.is_empty() {
        info = info.child(column().gap(1).child(label("notes").style("caption")).child(label(c.note.clone()).id("note").wrap(true)));
    }
    let uid = c.uid.clone();
    let (e, f, s, d) = (m.clone(), m.clone(), m.clone(), m.clone());
    let (u1, u2, u3) = (uid.clone(), uid.clone(), uid.clone());
    let mut actions = row()
        .gap(6)
        .child(button("Edit").id("edit").on_click(move |_, _| e.borrow_mut().edit(&u1)))
        .child(button(if c.favourite { "Unfavourite" } else { "Favourite" }).id("favourite").on_click(move |_, _| f.borrow_mut().ops.push(Op::Favourite(u2.clone()))))
        .child(button("Share").id("share").on_click(move |_, _| s.borrow_mut().ops.push(Op::Share(u3.clone()))))
        .child(button("Delete").id("delete").style("danger").on_click(move |_, _| {
            let mut m = d.borrow_mut();
            m.confirm_delete = true;
            m.dirty = true;
        }));
    if compact {
        actions = actions.gap(4);
    }
    let mut col = column().gap(10);
    if compact {
        col = col.child(row().child(spacer()).child(picture(st, c, big)).child(spacer())).child(info);
    } else {
        col = col.child(row().gap(14).align(Align::Start).child(picture(st, c, big)).child(info));
    }
    col = col.child(actions);
    if let Some(p) = st.shared.clone() {
        col = col.child(row().gap(6).child(button("Send in Messages").id("send_vcard").on_click(move |cx, _| cx.launch(MESSAGES_ID, &["--vcard".into(), p.clone()]))).child(spacer()));
    }
    col.child(message(cx, &st.message))
}

fn edit_form(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    let Some(d) = &st.draft else { return spacer() };
    let compact = cx.class == SizeClass::Compact;
    let field = |id: &str, ph: &str, text: &str, set: fn(&mut Draft, String)| {
        let mm = m.clone();
        let idc = id.to_string();
        text_field().id(id).placeholder(ph).text(text.to_string()).grow(1).on_change(move |cx, _| {
            let t = cx.text(&idc);
            if let Some(d) = mm.borrow_mut().draft_mut() {
                set(d, t);
            }
        })
    };
    let initials = Contact { name: d.name.clone(), ..Default::default() }.initials();
    let clear = m.clone();
    let mut photo_row = row()
        .gap(8)
        .align(Align::Center)
        .child(picture_of(st, &d.photo, &initials, 48))
        .child(button(if d.photo.is_empty() { "Add photo" } else { "Change photo" }).id("photo").on_click(move |cx, _| {
            cx.send(Message::PickFile(proto::PickFile { req: PICK_PHOTO, mode: proto::PickMode::Open, types: vec!["png".into()], suggested: String::new() }));
        }));
    if !d.photo.is_empty() {
        photo_row = photo_row.child(button("Remove").id("photo_clear").on_click(move |_, _| {
            if let Some(d) = clear.borrow_mut().draft_mut() {
                d.photo.clear();
            }
        }));
    }
    let mut c = column().gap(8).child(photo_row).child(column().gap(2).child(label("Name").style("caption")).child(field("e_name", "Name", &d.name, |d, t| d.name = t)));
    // numbers
    let mut nums = column().gap(4).child(label("Numbers").style("caption"));
    for (i, t) in d.tels.iter().enumerate() {
        let (lb, nm, rm) = (m.clone(), m.clone(), m.clone());
        let id = format!("e_num_{i}");
        let idc = id.clone();
        nums = nums.child(
            row()
                .gap(4)
                .align(Align::Center)
                .child(button(label_name(&t.label)).id(format!("e_label_{i}")).width(if compact { 60 } else { 70 }).on_click(move |_, _| {
                    if let Some(t) = lb.borrow_mut().draft_mut().and_then(|d| d.tels.get_mut(i)) {
                        t.label = next_label(&t.label);
                    }
                }))
                .child(text_field().id(id).placeholder("Number").text(t.number.clone()).grow(1).on_change(move |cx, _| {
                    let v = cx.text(&idc);
                    if let Some(t) = nm.borrow_mut().draft_mut().and_then(|d| d.tels.get_mut(i)) {
                        t.number = v;
                    }
                }))
                .child(button("x").id(format!("e_remove_{i}")).on_click(move |_, _| {
                    if let Some(d) = rm.borrow_mut().draft_mut() {
                        if i < d.tels.len() {
                            d.tels.remove(i);
                        }
                    }
                })),
        );
    }
    let add = m.clone();
    nums = nums.child(row().child(button("Add number").id("e_add_num").on_click(move |_, _| {
        if let Some(d) = add.borrow_mut().draft_mut() {
            let label = LABELS.get(d.tels.len()).map_or("CELL", |l| l.0);
            d.tels.push(Number { label: label.into(), number: String::new() });
        }
    })).child(spacer()));
    c = c.child(nums);
    c = c.child(column().gap(2).child(label("Email").style("caption")).child(field("e_email", "name@example.ecm", &d.email, |d, t| d.email = t)));
    let nt = m.clone();
    c = c.child(column().gap(2).child(label("Notes").style("caption")).child(text_area().id("e_note").text(d.note.clone()).height(48).on_change(move |cx, _| {
        let t = cx.text("e_note");
        if let Some(d) = nt.borrow_mut().draft_mut() {
            d.note = t;
        }
    })));
    let (sv, cn) = (m.clone(), m.clone());
    c = c.child(
        row()
            .gap(6)
            .child(spacer())
            .child(button("Cancel").id("cancel").on_click(move |_, _| cn.borrow_mut().cancel()))
            .child(button("Save").id("save").style("primary").on_click(move |_, _| sv.borrow_mut().ops.push(Op::Save))),
    );
    c.child(message(cx, &st.message))
}

/// A book of five contacts (Alex a favourite, with a work number and a note) and
/// this phone's SIM, for the tests and the golden recorder.
pub fn sample_files() -> crate::fake::MemFiles {
    let mut f = crate::fake::MemFiles::default();
    let mut b = Book::default();
    let mut alex = Contact::new("Alex Smith", "+12145550142");
    alex.tels.push(Number { label: "WORK".into(), number: "+12145550100".into() });
    alex.emails.push("alex@village.ecm".into());
    alex.note = "Met at the Phone Store. Has the spare SIM cards.".into();
    alex.favourite = true;
    b.put(alex);
    b.put(Contact::new("Bo Jones", "+12145550143"));
    let mut cy = Contact::new("Cy Miller", "+12145550144");
    cy.tels[0].label = "HOME".into();
    b.put(cy);
    b.put(Contact::new("Dana Lee", "+12145550145"));
    b.put(Contact::new("Mia Park", "+12145550146"));
    b.save(&mut f).unwrap();
    f
}

#[cfg(test)]
mod tests;
