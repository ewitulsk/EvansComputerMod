//! [`BrowserApp`]: the window, the event loop and the loading between turns.
//!
//! Handlers only change the [`Model`]; after each loop turn the driver does
//! the rest:
//!
//! * page events (clicks, typing, keys, form submits) go to the page's
//!   scripts first; their defaults (follow the link, send the form) happen
//!   when no listener called `preventDefault()`;
//! * what was asked for is loaded: "Loading ..." first (the fetch blocks),
//!   then the page (or a form's answer) and its style sheets and scripts
//!   through the [`Fetcher`], shown as the page, an error page, a text file,
//!   or saved as a download;
//! * the page's timers and animation frames run (`ecm-js`, under its time
//!   budget), and a document they changed is rendered again, once;
//! * pictures are fetched one per turn, decoded and uploaded;
//! * the settings (home page, bookmarks, visits) and each host's
//!   `localStorage` are saved in the app's data folder.
//!
//! A script's `alert` / `confirm` shows a dialog in the window and runs the
//! UI until the player answers (the page's script waits, as in a browser).

use crate::model::{After, Dialog, Img, Model, Nav, PageEvent, HOME};
use crate::{pages, view};
use ecm_js::{Fetched, Script};
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::json::Json;
use ecm_ui::{App, Ctx};
use ecm_web::fetch::{FetchError, Fetcher, Response, Transport};
use ecm_web::url::Scheme;
use ecm_web::{Page, Url};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Where downloads go.
pub const DOWNLOADS: &str = "/home/Downloads";
/// Animation frames run at most this often, ms.
pub const FRAME_MS: u64 = 16;

pub struct BrowserApp<T: Transport + 'static> {
    pub app: Rc<RefCell<App>>,
    pub model: Rc<RefCell<Model>>,
    pub fetcher: Rc<RefCell<Fetcher<T>>>,
    pub window: u32,
    /// The page's scripts (None: the page has none).
    pub script: Option<Script>,
    /// When the scripts' next timer is due.
    next_tick: Option<Instant>,
    /// The app's data folder (`ECM_APP_DATA`); None = settings aren't saved.
    data_dir: Option<String>,
}

/// What a suspend saves: the page, where it was scrolled, and the history.
pub fn save_state(m: &Model) -> Json {
    Json::Obj(vec![
        ("history".into(), Json::Arr(m.history.iter().map(|u| Json::Str(u.to_string())).collect())),
        ("pos".into(), Json::Int(m.hist_pos as i64)),
        ("scroll".into(), Json::Int(m.scroll as i64)),
    ])
}

/// Bring back a saved state: the page is loaded again at the next turn.
pub fn restore(m: &mut Model, saved: &Json) -> bool {
    let Some(h) = saved.get("history").and_then(|h| h.as_array()) else { return false };
    let hist: Vec<Url> = h.iter().filter_map(|u| u.as_str().and_then(Url::parse)).collect();
    if hist.is_empty() {
        return false;
    }
    let pos = saved.get("pos").and_then(|p| p.as_i64()).unwrap_or(0).clamp(0, hist.len() as i64 - 1) as usize;
    m.history = hist;
    m.hist_pos = pos;
    m.pending = Some(Nav::Reload);
    m.pending_scroll = saved.get("scroll").and_then(|s| s.as_i64()).map(|s| s as f32);
    true
}

/// A download's file name in [`DOWNLOADS`], made safe.
pub fn download_path(url: &Url) -> String {
    let name: String = url.file_name().chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect();
    let name = name.trim_start_matches('.').to_string();
    format!("{DOWNLOADS}/{}", if name.is_empty() { "download".into() } else { name })
}

/// The `localStorage` file of a page's host (`file:` pages share one).
fn storage_name(url: &Url) -> String {
    let h = match url.scheme {
        Scheme::Http => format!("{}_{}", url.host, url.port),
        _ => "local".into(),
    };
    h.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect()
}

impl<T: Transport + 'static> BrowserApp<T> {
    /// Connect, open the window, and load `start` (an address as typed), the
    /// restored page, or the home page.
    pub fn new(conn: Box<dyn Connection>, transport: T, start: Option<&str>) -> Result<Self, ConnError> {
        Self::with_settings(conn, transport, start, None, HOME)
    }

    /// [`BrowserApp::new`] with the data folder (settings, `localStorage`) and the
    /// home page until one is set.
    pub fn with_settings(conn: Box<dyn Connection>, transport: T, start: Option<&str>, data_dir: Option<String>, default_home: &str) -> Result<Self, ConnError> {
        let mut app = App::with_connection(crate::APP_ID, conn)?;
        let mut m = Model { default_home: default_home.to_string(), ..Model::default() };
        if let Some(dir) = &data_dir {
            if let Some(j) = std::fs::read_to_string(format!("{dir}/browser.json")).ok().and_then(|t| Json::parse(&t).ok()) {
                m.settings = crate::model::Settings::from_json(&j);
            }
        }
        let restored = restore(&mut m, &app.restore_json());
        match start {
            Some(s) if m.go_input(s) => {}
            _ if restored => {}
            _ => {
                let home = m.home();
                m.go(home);
            }
        }
        let model = Rc::new(RefCell::new(m));
        let v = model.clone();
        let window = app.window("Browser", move |cx| view::view(&v, cx));
        let saved = model.clone();
        app.on_suspend_json(move || save_state(&saved.borrow()));
        let k = model.clone();
        app.on_key(move |cx, key, mods| view::page_key(&k, cx, key, mods));
        let h1 = model.clone();
        app.hotkey(1, "Back", move |_| h1.borrow_mut().pending = Some(Nav::Back));
        let h2 = model.clone();
        app.hotkey(2, "Up", move |cx| {
            let page = h2.borrow().view.1 - 24.0;
            h2.borrow_mut().scroll_by(-page);
            cx.invalidate();
        });
        let h3 = model.clone();
        app.hotkey(3, "Down", move |cx| {
            let page = h3.borrow().view.1 - 24.0;
            h3.borrow_mut().scroll_by(page);
            cx.invalidate();
        });
        Ok(BrowserApp {
            app: Rc::new(RefCell::new(app)),
            model,
            fetcher: Rc::new(RefCell::new(Fetcher::new(transport))),
            window,
            script: None,
            next_tick: None,
            data_dir,
        })
    }

    /// One loop turn: draw, wait for input (not while a load waits; until
    /// the next timer), then load and run what is due.
    pub fn step(&mut self, timeout_ms: i32) -> bool {
        let busy = {
            let m = self.model.borrow();
            m.pending.is_some() || !m.events.is_empty() || !m.missing_images().is_empty()
        };
        let mut wait = if busy { 0 } else { timeout_ms };
        if let Some(t) = self.next_tick {
            let ms = t.saturating_duration_since(Instant::now()).as_millis() as i32;
            wait = if wait < 0 { ms } else { wait.min(ms) };
        }
        let alive = self.app.borrow_mut().step(wait);
        if !alive {
            return false;
        }
        self.process();
        !self.app.borrow().core.is_closed()
    }

    pub fn run(&mut self) {
        while self.step(-1) {}
    }

    fn redraw(&mut self) {
        self.sync_window();
        let mut app = self.app.borrow_mut();
        app.core.mark_all_dirty();
        let _ = app.core.render();
    }

    fn mark_dirty(&self) {
        self.app.borrow_mut().core.mark_all_dirty();
    }

    /// The title and the address bar follow the model.
    fn sync_window(&mut self) {
        let title = self.model.borrow().title();
        let mut app = self.app.borrow_mut();
        app.core.set_title(self.window, &title);
        let addr = self.model.borrow_mut().set_address.take();
        if let Some(a) = addr {
            let mut cx = Ctx::new(&mut app.core, self.window);
            cx.set_text("url", &a);
        }
    }

    /// The I/O between loop turns.
    pub fn process(&mut self) {
        self.page_events();
        let nav = self.model.borrow_mut().pending.take();
        match nav {
            Some(Nav::Submit(s)) => self.submit(s),
            Some(nav) => {
                let target = self.model.borrow_mut().resolve(nav);
                if let Some((url, push)) = target {
                    self.load(url, push);
                }
            }
            None => {}
        }
        self.run_timers();
        self.load_one_image();
        self.save_settings();
        self.sync_window();
    }

    /// Run `f` with the page moved into the scripts' host (see `ecm_js::Script`).
    fn with_script<R>(&mut self, f: impl FnOnce(&mut Script, &mut Option<Page>) -> R) -> Option<R> {
        let script = self.script.as_mut()?;
        let mut page = self.model.borrow_mut().page.take();
        let r = f(script, &mut page);
        self.model.borrow_mut().page = page;
        Some(r)
    }

    /// After the scripts ran: their changes, console, storage and navigation.
    fn after_scripts(&mut self) {
        let Some(script) = self.script.as_ref() else { return };
        let dirty = script.take_dirty();
        let (console, nav, storage) = {
            let mut h = script.host.borrow_mut();
            let console = h.console.clone();
            let nav = h.navigate.take();
            let storage = std::mem::take(&mut h.storage_dirty).then(|| h.storage.clone());
            (console, nav, storage)
        };
        let mut m = self.model.borrow_mut();
        if dirty {
            m.changed();
        }
        if m.console.len() != console.len() || m.console.last() != console.last() {
            m.console = console;
        }
        match nav {
            Some(ecm_js::Nav::Go(u)) => {
                if let Some(u) = Url::parse(&u) {
                    m.go(u);
                }
            }
            Some(ecm_js::Nav::Back) => m.pending = Some(Nav::Back),
            Some(ecm_js::Nav::Forward) => m.pending = Some(Nav::Forward),
            Some(ecm_js::Nav::Reload) => m.pending = Some(Nav::Reload),
            None => {}
        }
        let host = m.page.as_ref().map(|p| storage_name(&p.url));
        drop(m);
        if let (Some(store), Some(dir), Some(host)) = (storage, &self.data_dir, host) {
            let j = Json::Obj(store.into_iter().map(|(k, v)| (k, Json::Str(v))).collect());
            let _ = std::fs::create_dir_all(format!("{dir}/storage"));
            let _ = std::fs::write(format!("{dir}/storage/{host}.json"), j.to_string());
        }
        self.mark_dirty();
    }

    /// Page events to the scripts, then their defaults.
    fn page_events(&mut self) {
        let mut queue: std::collections::VecDeque<PageEvent> = std::mem::take(&mut self.model.borrow_mut().events).into();
        if queue.is_empty() {
            return;
        }
        while let Some(ev) = queue.pop_front() {
            let prevented = self.with_script(|s, p| s.dispatch(p, ev.target, &ev.kind, &ev.key)).unwrap_or(false);
            if prevented {
                continue;
            }
            match ev.after {
                After::Nothing => {}
                After::Go(u) => self.model.borrow_mut().go(u),
                After::Submit { form, submitter } => {
                    if ev.kind == "submit" {
                        self.model.borrow_mut().send_form(form, submitter);
                    } else {
                        // The `submit` event at the form, then the send.
                        queue.push_front(PageEvent { target: form, kind: "submit".into(), key: String::new(), after: After::Submit { form, submitter } });
                    }
                }
            }
        }
        if self.script.is_some() {
            self.after_scripts();
            self.schedule();
        }
    }

    /// When the scripts' next timer or frame is due.
    fn schedule(&mut self) {
        let pending = self.script.as_ref().is_some_and(|s| s.pending());
        if !pending {
            self.next_tick = None;
        } else if self.next_tick.is_none() {
            self.next_tick = Some(Instant::now());
        }
    }

    fn run_timers(&mut self) {
        let Some(due) = self.next_tick else { return };
        if Instant::now() < due {
            return;
        }
        let next = self.with_script(|s, p| s.tick(p)).flatten();
        self.next_tick = next.map(|ms| Instant::now() + Duration::from_millis((ms as u64).max(if ms <= 0.0 { FRAME_MS } else { 1 })));
        self.after_scripts();
    }

    fn save_settings(&mut self) {
        let dirty = std::mem::take(&mut self.model.borrow_mut().settings_dirty);
        if !dirty {
            return;
        }
        if let Some(dir) = &self.data_dir {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(format!("{dir}/browser.json"), self.model.borrow().settings.to_json().to_string());
        }
    }

    /// Fetch, decode and upload one picture the page wants.
    fn load_one_image(&mut self) {
        let Some(url) = self.model.borrow().missing_images().into_iter().next() else { return };
        let resp = Url::parse(&url).and_then(|u| self.fetcher.borrow_mut().get(&u).ok());
        let img = resp
            .filter(|r| r.status == 200)
            .and_then(|r| ecm_web::png::decode(&r.body).ok())
            .and_then(|rgba| self.app.borrow_mut().core.image_rgba(rgba.w as i32, rgba.h as i32, &rgba.px).ok().map(|resource| Img { w: rgba.w, h: rgba.h, resource }));
        let mut m = self.model.borrow_mut();
        m.images.insert(url, img);
        m.changed();
        drop(m);
        self.mark_dirty();
    }

    /// Show "Loading ..." for `url` before a blocking fetch.
    fn loading(&mut self, url: &Url) {
        {
            let mut m = self.model.borrow_mut();
            m.loading = Some(url.to_string());
            m.notice = None;
            m.set_address = Some(url.to_string());
        }
        self.redraw();
    }

    fn shown(&mut self, page: Page, status: u16, push: bool) {
        let shown = page.url.clone();
        let scripts = self.collect_scripts(&page);
        {
            let mut m = self.model.borrow_mut();
            m.loading = None;
            m.show(page, status);
            m.visit(&shown, push);
            m.set_address = Some(shown.to_string());
        }
        self.start_scripts(&shown, scripts);
        self.mark_dirty();
        println!("browser: {status} {shown}");
    }

    /// A page's scripts in document order: (name, source). External ones are fetched.
    fn collect_scripts(&mut self, page: &Page) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for n in page.dom.find_all("script") {
            let ty = page.dom.attr(n, "type").unwrap_or("").to_ascii_lowercase();
            if !(ty.is_empty() || ty.contains("javascript") || ty == "module") {
                continue;
            }
            match page.dom.attr(n, "src") {
                Some(src) => {
                    let Some(u) = page.url.join(src) else { continue };
                    let code = self.fetcher.borrow_mut().get(&u).ok().filter(|r| r.status == 200).map(|r| r.text());
                    match code {
                        Some(c) => out.push((u.to_string(), c)),
                        None => out.push((u.to_string(), format!("console.error({:?})", format!("can't load the script {u}")))),
                    }
                }
                None => out.push((format!("{}#script{}", page.url, out.len() + 1), page.dom.text_content(n))),
            }
        }
        out
    }

    /// A new script runtime for the page shown, if it has scripts.
    fn start_scripts(&mut self, url: &Url, scripts: Vec<(String, String)>) {
        self.script = None;
        self.next_tick = None;
        if scripts.is_empty() {
            return;
        }
        let script = match Script::new() {
            Ok(s) => s,
            Err(e) => {
                self.model.borrow_mut().console.push(format!("[error] scripts can't run: {e}"));
                return;
            }
        };
        {
            let mut h = script.host.borrow_mut();
            h.viewport = self.model.borrow().view;
            if let Some(dir) = &self.data_dir {
                if let Some(Json::Obj(kv)) = std::fs::read_to_string(format!("{dir}/storage/{}.json", storage_name(url))).ok().and_then(|t| Json::parse(&t).ok()) {
                    h.storage = kv.into_iter().filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string()))).collect::<BTreeMap<_, _>>();
                }
            }
            let fetcher = self.fetcher.clone();
            h.fetch = Some(Box::new(move |method, url, body| {
                let mut f = fetcher.borrow_mut();
                let r = match body {
                    Some(b) => f.request(method, url, Some(("text/plain;charset=UTF-8", b.as_bytes()))),
                    None => f.request(method, url, None),
                };
                r.map(|r| Fetched { status: r.status, content_type: r.content_type(), url: r.url.to_string(), body: r.text() }).map_err(|e| e.explain().0)
            }));
            let app = self.app.clone();
            let model = self.model.clone();
            h.modal = Some(Box::new(move |text, ask| modal(&app, &model, text, ask)));
        }
        self.script = Some(script);
        self.with_script(|s, p| s.run_scripts(p, &scripts));
        self.after_scripts();
        self.schedule();
    }

    /// Load and show `url`.
    pub fn load(&mut self, url: Url, push: bool) {
        // A link to another part of the page shown: just scroll there.
        let same_doc = {
            let m = self.model.borrow();
            push && url.fragment.is_some() && m.page.as_ref().is_some_and(|p| p.url.without_fragment() == url.without_fragment())
        };
        if same_doc {
            let mut m = self.model.borrow_mut();
            m.visit(&url, true);
            m.pending_anchor = url.fragment.clone();
            m.cache = None;
            m.set_address = Some(url.to_string());
            drop(m);
            self.mark_dirty();
            return;
        }
        self.loading(&url);
        let (page, status) = self.fetch_page(&url);
        self.shown(page, status, push);
    }

    /// Send a form and show the answer.
    pub fn submit(&mut self, s: ecm_web::forms::Submission) {
        self.loading(&s.url);
        let result = {
            let mut f = self.fetcher.borrow_mut();
            if s.post {
                f.post_form(&s.url, s.body.as_deref().unwrap_or(""))
            } else {
                f.get(&s.url)
            }
        };
        let (page, status) = self.answer(&s.url, result);
        self.shown(page, status, true);
    }

    /// Fetch `url` and make the page to show (an error page when it fails).
    pub fn fetch_page(&mut self, url: &Url) -> (Page, u16) {
        match url.scheme {
            Scheme::About => {
                let (name, html) = {
                    let m = self.model.borrow();
                    match url.path.as_str() {
                        "blank" => ("blank", String::new()),
                        "bookmarks" => ("bookmarks", pages::bookmarks(&m.settings.bookmarks)),
                        "history" => ("history", pages::history(&m.settings.visits)),
                        _ => ("home", pages::home()),
                    }
                };
                let mut p = Page::parse(Url::parse(&format!("about:{name}")).expect("about"), &html);
                p.url.fragment = url.fragment.clone();
                (p, 0)
            }
            Scheme::File if url.path.ends_with('/') => {
                let index = url.join("index.html").expect("index");
                let found = self.fetcher.borrow_mut().get(&index);
                match found {
                    Ok(r) => self.answer(url, Ok(r)),
                    Err(_) => {
                        let entries = self.fetcher.borrow_mut().transport.list_dir(&url.file_path());
                        match entries {
                            Some(entries) => (Page::parse(url.clone(), &pages::folder(&url.to_string(), &entries)), 200),
                            None => {
                                let e = FetchError::NoFile(url.file_path());
                                (Page::parse(url.clone(), &pages::error(&url.to_string(), &e)), 0)
                            }
                        }
                    }
                }
            }
            _ => {
                let r = self.fetcher.borrow_mut().get(url);
                self.answer(url, r)
            }
        }
    }

    /// The page for a fetch's result: HTML (with its style sheets), text, a
    /// download, or an error page.
    fn answer(&mut self, asked: &Url, result: Result<Response, FetchError>) -> (Page, u16) {
        match result {
            Ok(resp) => {
                let mut final_url = resp.url.clone();
                final_url.fragment = asked.fragment.clone();
                let ct = resp.content_type();
                let attachment = resp.header("content-disposition").is_some_and(|d| d.to_ascii_lowercase().starts_with("attachment"));
                let mut page = if resp.is_html() && !attachment {
                    let mut p = Page::parse(final_url, &resp.text());
                    let fetcher = self.fetcher.clone();
                    p.load_sheets(&mut |u| fetcher.borrow_mut().get(u).ok().filter(|r| r.status == 200).map(|r| r.text()));
                    p
                } else if (ct.starts_with("text/") || ct == "application/json") && !attachment {
                    Page::plain_text(final_url, &resp.text())
                } else if ct == "image/png" && !attachment {
                    let src = final_url.to_string();
                    Page::parse(final_url.clone(), &format!("<html><head><title>{}</title></head><body><img src=\"{src}\"></body></html>", final_url.file_name()))
                } else if resp.status == 200 && final_url.scheme == Scheme::Http {
                    // Anything else from a server is a download.
                    let path = download_path(&final_url);
                    let saved = self.fetcher.borrow_mut().transport.save_file(&path, &resp.body);
                    match saved {
                        Ok(()) => Page::parse(final_url.clone(), &pages::downloaded(&final_url.to_string(), &path, resp.body.len(), &ct)),
                        Err(e) => Page::parse(final_url.clone(), &pages::error(&final_url.to_string(), &e)),
                    }
                } else {
                    Page::parse(final_url, &pages::unshowable(&resp.url.to_string(), &ct, resp.body.len()))
                };
                if page.title.is_empty() && resp.status >= 400 {
                    page.title = format!("{} {}", resp.status, resp.reason);
                }
                (page, resp.status)
            }
            Err(e) => (Page::parse(asked.clone(), &pages::error(&asked.to_string(), &e)), 0),
        }
    }
}

/// Show a script's dialog and run the UI until the player answers it.
fn modal(app: &Rc<RefCell<App>>, model: &Rc<RefCell<Model>>, text: &str, ask: bool) -> bool {
    {
        let mut m = model.borrow_mut();
        // Tests answer dialogs without a player.
        if let Some(a) = m.dialog_auto {
            m.console.push(format!("[{}] {text} -> {a}", if ask { "confirm" } else { "alert" }));
            return a || !ask;
        }
        m.dialog = Some(Dialog { text: text.chars().take(2000).collect(), ask });
        m.dialog_answer = None;
    }
    // A dialog from inside a nested loop (it can't happen: handlers don't run scripts) would deadlock: answer it.
    let Ok(mut a) = app.try_borrow_mut() else {
        model.borrow_mut().dialog = None;
        return !ask;
    };
    a.core.mark_all_dirty();
    // Until the player answers (a connection that never blocks, as in tests, gives up after a while).
    for _ in 0..100_000 {
        if !a.step(-1) {
            break;
        }
        if model.borrow().dialog_answer.is_some() {
            break;
        }
    }
    let mut m = model.borrow_mut();
    m.dialog = None;
    a.core.mark_all_dirty();
    m.dialog_answer.take().unwrap_or(false)
}
