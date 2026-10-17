//! The browser's state: the page shown, history, scrolling, the pointer, and
//! the navigation the driver does next. Handlers only change this; the I/O
//! happens between loop turns ([`crate::app`]).

use ecm_web::css::Dynamic;
use ecm_web::dom::NodeId;
use ecm_web::page::Rendered;
use ecm_web::style::UaColors;
use ecm_web::{Page, Url};

/// Wheel steps and arrow keys scroll this far, lu.
pub const SCROLL_STEP: f32 = 36.0;
/// A press that moves further than this is a drag (scrolling), not a click.
pub const DRAG_SLOP: i32 = 6;
/// History is kept to this many pages.
pub const MAX_HISTORY: usize = 50;
pub const HOME: &str = "about:home";

#[derive(Clone, Debug, PartialEq)]
pub enum Nav {
    /// Load an address; `push` adds it to the history.
    Go { url: Url, push: bool },
    Back,
    Forward,
    Reload,
}

#[derive(Clone, Debug)]
pub struct Press {
    pub x: i32,
    pub y: i32,
    pub scroll0: f32,
    pub dragging: bool,
}

/// The render of the current page for one viewport (re-made when the size,
/// the theme, the hovered element or the page change).
pub struct Cache {
    pub key: (i32, i32, u64, Option<NodeId>),
    pub rendered: Rendered,
}

pub struct Model {
    pub ua: UaColors,
    pub page: Option<Page>,
    /// Bumped for every new page (the render cache key).
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
        self.page = Some(page);
        self.status = status;
        self.generation += 1;
        self.cache = None;
        self.scroll = 0.0;
        self.hover = None;
        self.hover_link = None;
        self.press = None;
        self.loads += 1;
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

    /// The `:hover` state for the render.
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
        Dynamic { hover, ..Default::default() }
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
        let dy = if hover_key.is_some() { self.dynamic() } else { Dynamic::default() };
        let rendered = page.render(w, h, &self.ua, &dy, &|_| None);
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

    /// The element at a point of the page area.
    pub fn node_at(&self, x: i32, y: i32) -> Option<NodeId> {
        self.cache.as_ref()?.rendered.list.hit_test(x as f32, y as f32 + self.scroll)
    }

    /// The link at a point of the page area.
    pub fn link_at(&self, x: i32, y: i32) -> Option<Url> {
        let n = self.node_at(x, y)?;
        self.page.as_ref()?.link_at(n)
    }

    /// The pointer moved to `(x, y)` (no button): hover. True if the page must be redrawn.
    pub fn hover_at(&mut self, x: i32, y: i32) -> bool {
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

    /// The status line: a link under the pointer, or what is loading.
    pub fn status_text(&self) -> String {
        if let Some(l) = &self.loading {
            return format!("Loading {l} ...");
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
}

/// A key for the theme colours (the render cache).
pub fn ua_hash(u: &UaColors) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for v in [u.text, u.background, u.link, u.muted, u.border, u.surface, u.button] {
        h = (h ^ v as u64).wrapping_mul(1099511628211);
    }
    h
}
