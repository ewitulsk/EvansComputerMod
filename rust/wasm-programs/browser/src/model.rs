//! The browser's state: the page shown, history, scrolling, the pointer, form
//! editing, pictures, the saved settings (home page, bookmarks, visits), and
//! the navigation the driver does next. Handlers only change this; the I/O
//! happens between loop turns ([`crate::app`]).

use ecm_ui::json::Json;
use ecm_web::css::Dynamic;
use ecm_web::dom::NodeId;
use ecm_web::forms::{self, Control, Submission};
use ecm_web::page::Rendered;
use ecm_web::style::UaColors;
use ecm_web::{Page, Url};
use std::collections::HashMap;

/// Wheel steps and arrow keys scroll this far, lu.
pub const SCROLL_STEP: f32 = 36.0;
/// A press that moves further than this is a drag (scrolling), not a click.
pub const DRAG_SLOP: i32 = 6;
/// History is kept to this many pages.
pub const MAX_HISTORY: usize = 50;
/// Visits remembered for the History page.
pub const MAX_VISITS: usize = 100;
pub const HOME: &str = "about:home";

/// The computer's own default home page: `home <address>` in `/etc/browser.conf` (a Tech
/// Village house PC's points at its village's website). Lines starting with `#` are comments.
pub const CONF: &str = "/etc/browser.conf";

/// The `home` of a `/etc/browser.conf`.
pub fn home_from_conf(text: &str) -> Option<String> {
    text.lines().map(str::trim).filter(|l| !l.starts_with('#')).find_map(|l| {
        let (k, v) = l.split_once(char::is_whitespace)?;
        let v = v.trim();
        (k == "home" && Url::from_input(v).is_some()).then(|| v.to_string())
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum Nav {
    /// Load an address; `push` adds it to the history.
    Go { url: Url, push: bool },
    Back,
    Forward,
    Reload,
    /// Send a form.
    Submit(Submission),
}

#[derive(Clone, Debug)]
pub struct Press {
    pub x: i32,
    pub y: i32,
    pub scroll0: f32,
    pub dragging: bool,
}

/// The render of the current page for one viewport (re-made when the size,
/// the theme, the hovered element, the focus or the page change).
pub struct Cache {
    pub key: (i32, i32, u64, Option<NodeId>),
    pub rendered: Rendered,
}

/// A picture uploaded as an image resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Img {
    pub w: u32,
    pub h: u32,
    pub resource: u32,
}

/// What happens after a page event if no script prevents it.
#[derive(Clone, Debug, PartialEq)]
pub enum After {
    Nothing,
    /// Follow a link.
    Go(Url),
    /// Send a form: fires `submit` at it first.
    Submit { form: NodeId, submitter: Option<NodeId> },
}

/// Something that happened on the page, for its scripts (then the default).
#[derive(Clone, Debug, PartialEq)]
pub struct PageEvent {
    pub target: NodeId,
    /// `click`, `input`, `change`, `keydown`, `submit`.
    pub kind: String,
    /// Key events: the key's name (`a`, `Enter`, `ArrowUp`).
    pub key: String,
    pub after: After,
}

impl PageEvent {
    pub fn new(target: NodeId, kind: &str) -> PageEvent {
        PageEvent { target, kind: kind.into(), key: String::new(), after: After::Nothing }
    }
}

/// A script's `alert` / `confirm` on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub text: String,
    /// `confirm`: OK and Cancel; `alert`: OK.
    pub ask: bool,
}

/// What a click on the page asks the window to do.
#[derive(Clone, Debug, PartialEq)]
pub enum ClickResult {
    Nothing,
    /// Show a popup menu of a select's options (labels, the chosen one).
    Choose { select: NodeId, labels: Vec<String> },
}

/// Saved in the app's data folder (`browser.json`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    /// None: the default home page.
    pub home: Option<String>,
    /// (title, address)
    pub bookmarks: Vec<(String, String)>,
    /// Recent visits, newest last: (title, address).
    pub visits: Vec<(String, String)>,
}

impl Settings {
    pub fn to_json(&self) -> Json {
        let pairs = |v: &[(String, String)]| Json::Arr(v.iter().map(|(t, u)| Json::Arr(vec![Json::Str(t.clone()), Json::Str(u.clone())])).collect());
        Json::Obj(vec![
            ("home".into(), self.home.clone().map(Json::Str).unwrap_or(Json::Null)),
            ("bookmarks".into(), pairs(&self.bookmarks)),
            ("visits".into(), pairs(&self.visits)),
        ])
    }

    pub fn from_json(j: &Json) -> Settings {
        let pairs = |k: &str| -> Vec<(String, String)> {
            j.get(k)
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|p| {
                            let p = p.as_array()?;
                            Some((p.first()?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        Settings { home: j.get("home").and_then(|h| h.as_str()).map(str::to_string), bookmarks: pairs("bookmarks"), visits: pairs("visits") }
    }
}

pub struct Model {
    pub ua: UaColors,
    pub page: Option<Page>,
    /// Bumped for every new page and every change to it (the render cache key).
    pub generation: u64,
    pub history: Vec<Url>,
    pub hist_pos: usize,
    pub scroll: f32,
    /// What the driver loads next.
    pub pending: Option<Nav>,
    /// The address being loaded (shown in the status line).
    pub loading: Option<String>,
    /// The address bar's text should become this at the next turn.
    pub set_address: Option<String>,
    pub press: Option<Press>,
    /// The element under the pointer (for `:hover`) and the link there.
    pub hover: Option<NodeId>,
    pub hover_link: Option<String>,
    pub cache: Option<Cache>,
    /// The page area's size at the last render, lu.
    pub view: (f32, f32),
    /// Scroll to this element / position once the page is shown.
    pub pending_anchor: Option<String>,
    pub pending_scroll: Option<f32>,
    /// The HTTP status of the page shown (0 for the browser's own pages).
    pub status: u16,
    /// Pages loaded (tests and the log).
    pub loads: u32,
    /// The focused text control and its caret (characters).
    pub focus: Option<(NodeId, usize)>,
    /// Pictures by address (None: it didn't load).
    pub images: HashMap<String, Option<Img>>,
    pub settings: Settings,
    /// The home page when none is set.
    pub default_home: String,
    /// The settings changed: the driver saves them.
    pub settings_dirty: bool,
    /// A short message for the status line ("Bookmarked").
    pub notice: Option<String>,
    /// Page events waiting for the scripts (then their defaults).
    pub events: Vec<PageEvent>,
    /// A script's dialog, and the answer once the player gave it.
    pub dialog: Option<Dialog>,
    pub dialog_answer: Option<bool>,
    /// The page scripts' console, and whether its pane shows.
    pub console: Vec<String>,
    pub show_console: bool,
    /// Answer every dialog with this at once (tests; None: ask the player).
    pub dialog_auto: Option<bool>,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            ua: UaColors::default(),
            page: None,
            generation: 0,
            history: Vec::new(),
            hist_pos: 0,
            scroll: 0.0,
            pending: None,
            loading: None,
            set_address: None,
            press: None,
            hover: None,
            hover_link: None,
            cache: None,
            view: (0.0, 0.0),
            pending_anchor: None,
            pending_scroll: None,
            status: 0,
            loads: 0,
            focus: None,
            images: HashMap::new(),
            settings: Settings::default(),
            default_home: HOME.into(),
            settings_dirty: false,
            notice: None,
            events: Vec::new(),
            dialog: None,
            dialog_answer: None,
            console: Vec::new(),
            show_console: false,
            dialog_auto: None,
        }
    }
}

impl Model {
    pub fn current(&self) -> Option<&Url> {
        self.history.get(self.hist_pos)
    }

    pub fn can_back(&self) -> bool {
        self.hist_pos > 0
    }

    pub fn can_forward(&self) -> bool {
        self.hist_pos + 1 < self.history.len()
    }

    /// The home page: the one set, else the default.
    pub fn home(&self) -> Url {
        self.settings
            .home
            .as_deref()
            .and_then(Url::parse)
            .or_else(|| Url::parse(&self.default_home))
            .unwrap_or_else(|| Url::parse(HOME).expect("home"))
    }

    /// Navigate to what the player typed. False (and nothing happens) if it isn't an address.
    pub fn go_input(&mut self, text: &str) -> bool {
        match Url::from_input(text) {
            Some(url) => {
                self.pending = Some(Nav::Go { url, push: true });
                true
            }
            None => false,
        }
    }

    pub fn go(&mut self, url: Url) {
        self.pending = Some(Nav::Go { url, push: true });
    }

    /// Resolve a pending history move into the address to load.
    pub fn resolve(&mut self, nav: Nav) -> Option<(Url, bool)> {
        match nav {
            Nav::Go { url, push } => Some((url, push)),
            Nav::Back if self.can_back() => {
                self.hist_pos -= 1;
                self.history.get(self.hist_pos).cloned().map(|u| (u, false))
            }
            Nav::Forward if self.can_forward() => {
                self.hist_pos += 1;
                self.history.get(self.hist_pos).cloned().map(|u| (u, false))
            }
            Nav::Reload => self.current().cloned().map(|u| (u, false)),
            _ => None,
        }
    }

    /// Record a visit (after it loaded): a new entry drops the forward history.
    pub fn visit(&mut self, url: &Url, push: bool) {
        if push {
            if self.history.is_empty() {
                self.history.push(url.clone());
                self.hist_pos = 0;
            } else if self.history.get(self.hist_pos) != Some(url) {
                self.history.truncate(self.hist_pos + 1);
                self.history.push(url.clone());
                self.hist_pos = self.history.len() - 1;
            }
            while self.history.len() > MAX_HISTORY {
                self.history.remove(0);
                self.hist_pos = self.hist_pos.saturating_sub(1);
            }
        } else if let Some(h) = self.history.get_mut(self.hist_pos) {
            *h = url.clone();
        } else {
            self.history.push(url.clone());
            self.hist_pos = self.history.len() - 1;
        }
    }

    /// Show a loaded page.
    pub fn show(&mut self, page: Page, status: u16) {
        self.pending_anchor = page.url.fragment.clone();
        if page.url.scheme != ecm_web::url::Scheme::About {
            let addr = page.url.without_fragment().to_string();
            let title = if page.title.is_empty() { addr.clone() } else { page.title.clone() };
            self.settings.visits.retain(|(_, u)| *u != addr);
            self.settings.visits.push((title, addr));
            while self.settings.visits.len() > MAX_VISITS {
                self.settings.visits.remove(0);
            }
            self.settings_dirty = true;
        }
        self.page = Some(page);
        self.status = status;
        self.changed();
        self.scroll = 0.0;
        self.hover = None;
        self.hover_link = None;
        self.press = None;
        self.focus = None;
        self.events.clear();
        self.console.clear();
        self.loads += 1;
    }

    /// The page changed (a form edit, a picture arrived): render again.
    pub fn changed(&mut self) {
        self.generation += 1;
        self.cache = None;
    }

    /// The page's height at the last render.
    pub fn page_height(&self) -> f32 {
        self.cache.as_ref().map_or(0.0, |c| c.rendered.list.height)
    }

    pub fn max_scroll(&self) -> f32 {
        (self.page_height() - self.view.1).max(0.0)
    }

    pub fn scroll_by(&mut self, dy: f32) -> bool {
        let before = self.scroll;
        self.scroll = (self.scroll + dy).clamp(0.0, self.max_scroll());
        self.scroll != before
    }

    pub fn scroll_to(&mut self, y: f32) {
        self.scroll = y.clamp(0.0, self.max_scroll());
    }

    /// The `:hover` / `:focus` state for the render.
    pub fn dynamic(&self) -> Dynamic {
        let mut hover = Vec::new();
        if let (Some(p), Some(mut n)) = (&self.page, self.hover) {
            loop {
                hover.push(n);
                match p.dom.parent(n) {
                    Some(par) if par != ecm_web::dom::DOCUMENT => n = par,
                    _ => break,
                }
            }
        }
        Dynamic { hover, focus: self.focus.map(|f| f.0), active: None }
    }

    /// Render the page for a `w x h` area (cached).
    pub fn render(&mut self, w: f32, h: f32) {
        self.view = (w, h);
        let Some(page) = &self.page else { return };
        let hover_key = if self.cache.as_ref().is_some_and(|c| c.rendered.dynamic_selectors) { self.hover } else { None };
        let key = (w as i32, h as i32, self.generation ^ ua_hash(&self.ua), hover_key);
        if self.cache.as_ref().is_some_and(|c| c.key == key) {
            return;
        }
        let dy = self.dynamic();
        let images = &self.images;
        let sizes = |u: &str| images.get(u).copied().flatten().map(|i| (i.w as f32, i.h as f32));
        let rendered = page.render_with_caret(w, h, &self.ua, &dy, &sizes, self.focus);
        // The hover key only matters for pages with :hover rules.
        let key = (key.0, key.1, key.2, if rendered.dynamic_selectors { self.hover } else { None });
        self.cache = Some(Cache { key, rendered });
        if let Some(a) = self.pending_anchor.take() {
            if let Some(n) = page.anchor(&a) {
                if let Some(r) = self.cache.as_ref().and_then(|c| c.rendered.list.node_rect(n)) {
                    self.scroll = r.y;
                }
            }
        }
        if let Some(s) = self.pending_scroll.take() {
            self.scroll = s;
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    /// Render again at the last size if a change dropped the render (events
    /// in one batch: a click right after typing must hit-test the new page).
    pub fn ensure_rendered(&mut self) {
        if self.cache.is_none() && self.view.0 > 0.0 {
            let (w, h) = self.view;
            self.render(w, h);
        }
    }

    /// Pictures the page shows that haven't been fetched yet.
    pub fn missing_images(&self) -> Vec<String> {
        self.cache.as_ref().map(|c| c.rendered.images.iter().filter(|u| !self.images.contains_key(*u)).cloned().collect()).unwrap_or_default()
    }

    /// The element at a point of the page area.
    pub fn node_at(&self, x: i32, y: i32) -> Option<NodeId> {
        self.cache.as_ref()?.rendered.list.hit_test_view(x as f32, y as f32, self.scroll)
    }

    /// The link at a point of the page area.
    pub fn link_at(&self, x: i32, y: i32) -> Option<Url> {
        let n = self.node_at(x, y)?;
        self.page.as_ref()?.link_at(n)
    }

    /// The pointer moved to `(x, y)` (no button): hover. True if the page must be redrawn.
    pub fn hover_at(&mut self, x: i32, y: i32) -> bool {
        self.ensure_rendered();
        let n = self.node_at(x, y);
        let link = self.link_at(x, y).map(|u| u.to_string());
        let mut changed = link != self.hover_link;
        self.hover_link = link;
        if n != self.hover {
            self.hover = n;
            if self.cache.as_ref().is_some_and(|c| c.rendered.dynamic_selectors) {
                changed = true;
            }
        }
        changed
    }

    /// A click (press and release without a drag) at a point of the page area:
    /// form controls, then links.
    pub fn click(&mut self, x: i32, y: i32) -> ClickResult {
        self.ensure_rendered();
        let Some(node) = self.node_at(x, y) else {
            self.blur();
            return ClickResult::Nothing;
        };
        let Some(page) = self.page.as_mut() else { return ClickResult::Nothing };
        if let Some(c) = forms::target(&page.dom, node) {
            if forms::disabled(&page.dom, c) {
                return ClickResult::Nothing;
            }
            match forms::control(&page.dom, c) {
                Some(Control::Text { .. }) | Some(Control::TextArea) => {
                    let len = forms::value(&page.dom, c).chars().count();
                    self.focus = Some((c, len));
                    self.changed();
                    self.events.push(PageEvent::new(node, "click"));
                    return ClickResult::Nothing;
                }
                Some(Control::Checkbox) | Some(Control::Radio) => {
                    forms::toggle(&mut page.dom, c);
                    self.focus = None;
                    self.changed();
                    self.events.push(PageEvent::new(node, "click"));
                    self.events.push(PageEvent::new(c, "input"));
                    self.events.push(PageEvent::new(c, "change"));
                    return ClickResult::Nothing;
                }
                Some(Control::Select) => {
                    let labels = forms::options(&page.dom, c).into_iter().map(|o| forms::option_label(&page.dom, o)).collect();
                    self.focus = None;
                    self.changed();
                    self.events.push(PageEvent::new(node, "click"));
                    return ClickResult::Choose { select: c, labels };
                }
                Some(Control::Submit) => {
                    let form = forms::form_of(&page.dom, c);
                    let mut ev = PageEvent::new(node, "click");
                    if let Some(form) = form {
                        ev.after = After::Submit { form, submitter: Some(c) };
                    }
                    self.events.push(ev);
                    return ClickResult::Nothing;
                }
                _ => {
                    self.events.push(PageEvent::new(node, "click"));
                    return ClickResult::Nothing;
                }
            }
        }
        self.blur();
        let mut ev = PageEvent::new(node, "click");
        if let Some(u) = self.link_at(x, y) {
            ev.after = After::Go(u);
        }
        self.events.push(ev);
        ClickResult::Nothing
    }

    pub fn blur(&mut self) {
        if self.focus.take().is_some() {
            self.changed();
        }
    }

    /// Choose option `index` of a select (its popup's answer).
    pub fn choose(&mut self, select: NodeId, index: usize) {
        if let Some(p) = self.page.as_mut() {
            forms::choose(&mut p.dom, select, index);
            self.changed();
            self.events.push(PageEvent::new(select, "input"));
            self.events.push(PageEvent::new(select, "change"));
        }
    }

    /// Submit the form of control `c` (a button pressed, or Enter in a field):
    /// a `keydown` Enter event whose default sends it.
    pub fn submit_from(&mut self, c: NodeId) {
        let Some(page) = self.page.as_ref() else { return };
        let Some(form) = forms::form_of(&page.dom, c) else { return };
        let submitter = match forms::control(&page.dom, c) {
            Some(Control::Submit) => Some(c),
            _ => forms::default_button(&page.dom, form),
        };
        self.events.push(PageEvent { target: c, kind: "keydown".into(), key: "Enter".into(), after: After::Submit { form, submitter } });
    }

    /// What sending `form` (with `submitter`) does.
    pub fn send_form(&mut self, form: NodeId, submitter: Option<NodeId>) {
        let Some(page) = self.page.as_ref() else { return };
        if let Some(s) = forms::submission(&page.dom, &page.url, form, submitter) {
            self.pending = Some(Nav::Submit(s));
        }
    }

    /// Typed text goes to the focused field. True if it was used.
    pub fn type_text(&mut self, text: &str) -> bool {
        let Some((node, caret)) = self.focus else { return false };
        let Some(page) = self.page.as_mut() else { return false };
        let multiline = forms::control(&page.dom, node) == Some(Control::TextArea);
        let text: String = text.chars().filter(|c| !c.is_control() || (multiline && *c == '\n')).collect();
        if text.is_empty() {
            return true;
        }
        let v: Vec<char> = forms::value(&page.dom, node).chars().collect();
        let at = caret.min(v.len());
        let mut nv: String = v[..at].iter().collect();
        nv.push_str(&text);
        nv.extend(&v[at..]);
        let max = page.dom.attr(node, "maxlength").and_then(|m| m.parse::<usize>().ok()).unwrap_or(usize::MAX);
        if nv.chars().count() > max {
            return true;
        }
        forms::set_value(&mut page.dom, node, &nv);
        self.focus = Some((node, at + text.chars().count()));
        self.changed();
        self.events.push(PageEvent::new(node, "input"));
        true
    }

    /// An editing key for the focused field. True if it was used.
    pub fn edit_key(&mut self, key: u16, shift: bool) -> bool {
        use ecm_ui::window::keys;
        let Some((node, caret)) = self.focus else { return false };
        let Some(page) = self.page.as_mut() else { return false };
        let multiline = forms::control(&page.dom, node) == Some(Control::TextArea);
        let v: Vec<char> = forms::value(&page.dom, node).chars().collect();
        let caret = caret.min(v.len());
        let set = |page: &mut Page, nv: String| forms::set_value(&mut page.dom, node, &nv);
        match key {
            keys::BACKSPACE => {
                if caret > 0 {
                    let nv: String = v[..caret - 1].iter().chain(v[caret..].iter()).collect();
                    set(page, nv);
                    self.focus = Some((node, caret - 1));
                    self.events.push(PageEvent::new(node, "input"));
                }
            }
            keys::DELETE => {
                if caret < v.len() {
                    let nv: String = v[..caret].iter().chain(v[caret + 1..].iter()).collect();
                    set(page, nv);
                    self.events.push(PageEvent::new(node, "input"));
                }
            }
            keys::LEFT => self.focus = Some((node, caret.saturating_sub(1))),
            keys::RIGHT => self.focus = Some((node, (caret + 1).min(v.len()))),
            keys::HOME => self.focus = Some((node, 0)),
            keys::END => self.focus = Some((node, v.len())),
            keys::ENTER | keys::KP_ENTER => {
                if multiline {
                    return self.type_text("\n");
                }
                self.submit_from(node);
                return true;
            }
            keys::ESCAPE => {
                self.focus = None;
            }
            keys::TAB => {
                let next = forms::next_control(&page.dom, Some(node), shift);
                self.focus = next.filter(|n| forms::control(&page.dom, *n).is_some_and(|c| c.is_text())).map(|n| (n, forms::value(&page.dom, n).chars().count()));
            }
            _ => return false,
        }
        self.changed();
        true
    }

    /// A key the page's scripts see (`keydown` at the focused field, else the body).
    pub fn key_event(&mut self, key: u16, shift: bool) {
        let Some(name) = key_name(key, shift) else { return };
        let target = self.focus.map(|f| f.0).or_else(|| self.page.as_ref().and_then(|p| p.dom.find("body"))).unwrap_or(ecm_web::dom::DOCUMENT);
        self.events.push(PageEvent { target, kind: "keydown".into(), key: name, after: After::Nothing });
    }

    /// The status line: a notice, what is loading, or a link under the pointer.
    pub fn status_text(&self) -> String {
        if let Some(l) = &self.loading {
            return format!("Loading {l} ...");
        }
        if let Some(n) = &self.notice {
            return n.clone();
        }
        self.hover_link.clone().unwrap_or_default()
    }

    /// The window title.
    pub fn title(&self) -> String {
        let t = self.page.as_ref().map(|p| p.title.clone()).filter(|t| !t.is_empty());
        match t {
            Some(t) => format!("{t} - Browser"),
            None => self.current().map(|u| format!("{u} - Browser")).unwrap_or_else(|| "Browser".into()),
        }
    }

    /// Bookmark the page shown. False if there is none (or it is one of the browser's own).
    pub fn bookmark(&mut self) -> bool {
        let Some(p) = &self.page else { return false };
        if p.url.scheme == ecm_web::url::Scheme::About {
            return false;
        }
        let addr = p.url.to_string();
        let title = if p.title.is_empty() { addr.clone() } else { p.title.clone() };
        self.settings.bookmarks.retain(|(_, u)| *u != addr);
        self.settings.bookmarks.push((title.clone(), addr));
        self.settings_dirty = true;
        self.notice = Some(format!("Bookmarked: {title}"));
        true
    }

    /// The page shown becomes the home page.
    pub fn set_home(&mut self) -> bool {
        let Some(p) = &self.page else { return false };
        self.settings.home = Some(p.url.to_string());
        self.settings_dirty = true;
        self.notice = Some(format!("Home page: {}", p.url));
        true
    }
}

/// A key's name as scripts see it (`KeyboardEvent.key`).
pub fn key_name(key: u16, shift: bool) -> Option<String> {
    use ecm_ui::window::keys;
    Some(match key {
        0x04..=0x1D => {
            let c = (b'a' + (key - 0x04) as u8) as char;
            if shift { c.to_ascii_uppercase().to_string() } else { c.to_string() }
        }
        0x1E..=0x26 => ((b'1' + (key - 0x1E) as u8) as char).to_string(),
        0x27 => "0".into(),
        keys::ENTER | keys::KP_ENTER => "Enter".into(),
        keys::ESCAPE => "Escape".into(),
        keys::BACKSPACE => "Backspace".into(),
        keys::TAB => "Tab".into(),
        keys::SPACE => " ".into(),
        keys::LEFT => "ArrowLeft".into(),
        keys::RIGHT => "ArrowRight".into(),
        keys::UP => "ArrowUp".into(),
        keys::DOWN => "ArrowDown".into(),
        keys::DELETE => "Delete".into(),
        keys::HOME => "Home".into(),
        keys::END => "End".into(),
        keys::PAGE_UP => "PageUp".into(),
        keys::PAGE_DOWN => "PageDown".into(),
        _ => return None,
    })
}

/// A key for the theme colours (the render cache).
pub fn ua_hash(u: &UaColors) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for v in [u.text, u.background, u.link, u.muted, u.border, u.surface, u.button] {
        h = (h ^ v as u64).wrapping_mul(1099511628211);
    }
    h
}
