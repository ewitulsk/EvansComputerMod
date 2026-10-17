//! [`BrowserApp`]: the window, the event loop and the loading between turns.
//!
//! Handlers only change the [`Model`]; after each loop turn the driver loads
//! what was asked for: it shows "Loading ..." first (the fetch blocks), then
//! fetches the page and its style sheets through the [`Fetcher`], and shows
//! the page, an error page, or a text file.

use crate::model::{Model, Nav, HOME};
use crate::{pages, view};
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::json::Json;
use ecm_ui::{App, Ctx};
use ecm_web::fetch::{Fetcher, Transport};
use ecm_web::url::Scheme;
use ecm_web::{Page, Url};
use std::cell::RefCell;
use std::rc::Rc;

pub struct BrowserApp<T: Transport> {
    pub app: App,
    pub model: Rc<RefCell<Model>>,
    pub fetcher: Fetcher<T>,
    pub window: u32,
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

impl<T: Transport> BrowserApp<T> {
    /// Connect, open the window, and load `start` (an address as typed), the
    /// restored page, or the start page.
    pub fn new(conn: Box<dyn Connection>, transport: T, start: Option<&str>) -> Result<Self, ConnError> {
        let mut app = App::with_connection(crate::APP_ID, conn)?;
        let mut m = Model::default();
        let restored = restore(&mut m, &app.restore_json());
        match start {
            Some(s) if m.go_input(s) => {}
            _ if restored => {}
            _ => m.go(Url::parse(HOME).expect("home")),
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
        Ok(BrowserApp { app, model, fetcher: Fetcher::new(transport), window })
    }

    /// One loop turn: draw, wait for input (not while a load is waiting), then load.
    pub fn step(&mut self, timeout_ms: i32) -> bool {
        let wait = if self.model.borrow().pending.is_some() { 0 } else { timeout_ms };
        if !self.app.step(wait) {
            return false;
        }
        self.process();
        !self.app.core.is_closed()
    }

    pub fn run(&mut self) {
        while self.step(-1) {}
    }

    fn redraw(&mut self) {
        self.sync_window();
        self.app.core.mark_all_dirty();
        let _ = self.app.core.render();
    }

    /// The title and the address bar follow the model.
    fn sync_window(&mut self) {
        let title = self.model.borrow().title();
        self.app.core.set_title(self.window, &title);
        let addr = self.model.borrow_mut().set_address.take();
        if let Some(a) = addr {
            let mut cx = Ctx::new(&mut self.app.core, self.window);
            cx.set_text("url", &a);
        }
    }

    /// The I/O between loop turns.
    pub fn process(&mut self) {
        let nav = self.model.borrow_mut().pending.take();
        if let Some(nav) = nav {
            let target = self.model.borrow_mut().resolve(nav);
            if let Some((url, push)) = target {
                self.load(url, push);
            }
        }
        self.sync_window();
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
            self.app.core.mark_all_dirty();
            return;
        }
        {
            let mut m = self.model.borrow_mut();
            m.loading = Some(url.to_string());
            m.set_address = Some(url.to_string());
        }
        // Show "Loading ..." before the (blocking) fetch.
        self.redraw();
        let (page, status) = self.fetch_page(&url);
        let shown = page.url.clone();
        {
            let mut m = self.model.borrow_mut();
            m.loading = None;
            m.show(page, status);
            m.visit(&shown, push);
            m.set_address = Some(shown.to_string());
        }
        self.app.core.mark_all_dirty();
        println!("browser: {status} {shown}");
    }

    /// Fetch `url` and make the page to show (an error page when it fails).
    pub fn fetch_page(&mut self, url: &Url) -> (Page, u16) {
        match url.scheme {
            Scheme::About => {
                let html = if url.path == "blank" { String::new() } else { pages::home() };
                let mut p = Page::parse(Url::parse(&format!("about:{}", if url.path == "blank" { "blank" } else { "home" })).expect("about"), &html);
                p.url.fragment = url.fragment.clone();
                (p, 0)
            }
            Scheme::File if url.path.ends_with('/') => {
                let index = url.join("index.html").expect("index");
                match self.fetcher.get(&index) {
                    Ok(_) => self.fetch_http_or_file(&index, url),
                    Err(_) => match self.fetcher.transport.list_dir(&url.file_path()) {
                        Some(entries) => (Page::parse(url.clone(), &pages::folder(&url.to_string(), &entries)), 200),
                        None => {
                            let e = ecm_web::fetch::FetchError::NoFile(url.file_path());
                            (Page::parse(url.clone(), &pages::error(&url.to_string(), &e)), 0)
                        }
                    },
                }
            }
            _ => self.fetch_http_or_file(url, url),
        }
    }

    fn fetch_http_or_file(&mut self, url: &Url, asked: &Url) -> (Page, u16) {
        match self.fetcher.get(url) {
            Ok(resp) => {
                let mut final_url = resp.url.clone();
                final_url.fragment = asked.fragment.clone();
                let ct = resp.content_type();
                let mut page = if resp.is_html() {
                    let mut p = Page::parse(final_url, &resp.text());
                    let fetcher = &mut self.fetcher;
                    p.load_sheets(&mut |u| fetcher.get(u).ok().filter(|r| r.status == 200).map(|r| r.text()));
                    p
                } else if ct.starts_with("text/") || ct == "application/json" {
                    Page::plain_text(final_url, &resp.text())
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
