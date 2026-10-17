//! `ecm-js`: page scripts for the Browser (docs/gui/BROWSER.md, phase 3).
//!
//! One QuickJS runtime and context per page ([`Script`]), inside the Browser's
//! own process. The Rust side is small: native functions (`__n.*`) that read
//! and change `ecm-web`'s document (find, create, move, attributes, text,
//! `innerHTML`, inline styles, form values), plus `fetch`, `localStorage`,
//! navigation, the console and dialogs through the [`Host`]. The JS prelude
//! (prelude.js) builds the web API on them: `document`, Element objects,
//! `classList`, `style`, events with bubbling and `preventDefault`,
//! `setTimeout`/`setInterval`/`requestAnimationFrame`, `fetch()` (Promises),
//! `localStorage`, `location`, `history`, `console`, `alert`/`confirm`.
//!
//! Every entry into JS (a script, an event, a timer tick) runs under a time
//! budget: QuickJS's interrupt handler stops a script that runs too long
//! (`while (true) {}`), so a page can't hang the Browser or the computer.
//!
//! The page lives in the Browser's model; it is moved into the [`Host`] only
//! while JS runs ([`Script::with_page`]).

use ecm_web::css::{self, Dynamic};
use ecm_web::dom::{Dom, Element, NodeData, NodeId, DOCUMENT};
use ecm_web::{forms, Page, Url};
use rquickjs::{Context, Ctx, Function, Object, Runtime};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How long one entry into JS (an event, a timer tick) may run.
pub const BUDGET: Duration = Duration::from_millis(250);
/// How long a page's scripts may run when it loads.
pub const LOAD_BUDGET: Duration = Duration::from_millis(1000);
/// The memory a page's scripts may use.
pub const MEMORY_LIMIT: usize = 16 << 20;
/// Console lines kept.
pub const MAX_CONSOLE: usize = 200;

const PRELUDE: &str = include_str!("prelude.js");

/// What `fetch()` got.
#[derive(Clone, Debug, PartialEq)]
pub struct Fetched {
    pub status: u16,
    pub body: String,
    pub content_type: String,
    pub url: String,
}

/// A navigation a script asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum Nav {
    Go(String),
    Back,
    Forward,
    Reload,
}

/// What the scripts reach outside the page: set by the Browser.
#[derive(Default)]
pub struct Host {
    /// The page, while JS runs.
    pub page: Option<Page>,
    /// The scripts changed the document: render again.
    pub dirty: bool,
    pub console: Vec<String>,
    pub navigate: Option<Nav>,
    /// This page's host's `localStorage`.
    pub storage: BTreeMap<String, String>,
    pub storage_dirty: bool,
    /// `fetch()`: method, absolute address, body → the answer or why not.
    pub fetch: Option<Box<dyn FnMut(&str, &Url, Option<&str>) -> Result<Fetched, String>>>,
    /// `alert` (false) / `confirm` (true): shows the dialog, waits for the answer.
    pub modal: Option<Box<dyn FnMut(&str, bool) -> bool>>,
    /// The viewport (`innerWidth`, `innerHeight`).
    pub viewport: (f32, f32),
}

impl Host {
    fn log(&mut self, line: String) {
        self.console.push(line);
        if self.console.len() > MAX_CONSOLE {
            let n = self.console.len() - MAX_CONSOLE;
            self.console.drain(..n);
        }
    }
}

type H = Rc<RefCell<Host>>;

/// A page's script runtime.
pub struct Script {
    rt: Runtime,
    ctx: Context,
    pub host: H,
    deadline: Rc<Cell<Option<Instant>>>,
    started: Instant,
    /// Entries stopped by the budget.
    pub stopped: u32,
}

fn with_page<R: Default>(h: &H, f: impl FnOnce(&mut Page) -> R) -> R {
    let mut host = h.borrow_mut();
    match host.page.as_mut() {
        Some(p) => f(p),
        None => R::default(),
    }
}

fn with_dom<R: Default>(h: &H, f: impl FnOnce(&mut Dom) -> R) -> R {
    with_page(h, |p| f(&mut p.dom))
}

fn node(dom: &Dom, id: i32) -> Option<NodeId> {
    (id >= 0 && (id as usize) < dom.nodes.len()).then_some(id as usize)
}

fn mutate<R: Default>(h: &H, f: impl FnOnce(&mut Dom) -> R) -> R {
    let r = with_dom(h, f);
    h.borrow_mut().dirty = true;
    r
}

/// `fontSize` -> `font-size`.
fn kebab(prop: &str) -> String {
    let mut out = String::new();
    for c in prop.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// An inline style attribute as (property, value) pairs in order.
fn style_pairs(s: &str) -> Vec<(String, String)> {
    css::split_top(s, ';')
        .into_iter()
        .filter_map(|d| d.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

fn install(ctx: &Ctx, h: &H) -> rquickjs::Result<()> {
    let n = Object::new(ctx.clone())?;
    macro_rules! f {
        ($name:expr, $func:expr) => {
            n.set($name, Function::new(ctx.clone(), $func)?)?;
        };
    }
    let hh = h.clone();
    f!("byId", move |id: String| -> i32 { with_dom(&hh, |d| d.by_id(&id).map_or(-1, |n| n as i32)) });
    let hh = h.clone();
    f!("query", move |root: i32, sel: String, all: bool| -> Vec<i32> {
        with_dom(&hh, |d| {
            let Some(root) = node(d, root) else { return Vec::new() };
            let sels = css::parse_selectors(&sel);
            let dy = Dynamic::default();
            let mut out = Vec::new();
            for n in d.descendants(root).into_iter().skip(1) {
                if d.element(n).is_some() && sels.iter().any(|s| s.matches(d, n, &dy)) {
                    out.push(n as i32);
                    if !all {
                        break;
                    }
                }
            }
            out
        })
    });
    let hh = h.clone();
    f!("kind", move |id: i32| -> i32 {
        // 1 element, 3 text, 9 document, 0 none
        with_dom(&hh, |d| match node(d, id).map(|n| &d.nodes[n].data) {
            Some(NodeData::Element(_)) => 1,
            Some(NodeData::Text(_)) => 3,
            Some(NodeData::Document) => 9,
            None => 0,
        })
    });
    let hh = h.clone();
    f!("tag", move |id: i32| -> String { with_dom(&hh, |d| node(d, id).and_then(|n| d.tag(n)).unwrap_or("").to_string()) });
    let hh = h.clone();
    f!("attr", move |id: i32, name: String| -> Option<String> {
        with_dom(&hh, |d| node(d, id).and_then(|n| d.attr(n, &name.to_ascii_lowercase())).map(str::to_string))
    });
    let hh = h.clone();
    f!("attrNames", move |id: i32| -> Vec<String> {
        with_dom(&hh, |d| node(d, id).and_then(|n| d.element(n)).map(|e| e.attrs.iter().map(|(k, _)| k.clone()).collect()).unwrap_or_default())
    });
    let hh = h.clone();
    f!("setAttr", move |id: i32, name: String, value: String| {
        mutate(&hh, |d| {
            if let Some(e) = node(d, id).and_then(|n| d.element_mut(n)) {
                e.set_attr(&name.to_ascii_lowercase(), &value);
            }
        })
    });
    let hh = h.clone();
    f!("removeAttr", move |id: i32, name: String| {
        mutate(&hh, |d| {
            if let Some(e) = node(d, id).and_then(|n| d.element_mut(n)) {
                let name = name.to_ascii_lowercase();
                e.attrs.retain(|(k, _)| *k != name);
            }
        })
    });
    let hh = h.clone();
    f!("text", move |id: i32| -> String {
        with_dom(&hh, |d| match node(d, id) {
            Some(n) => match &d.nodes[n].data {
                NodeData::Text(t) => t.clone(),
                _ => d.text_content(n),
            },
            None => String::new(),
        })
    });
    let hh = h.clone();
    f!("setText", move |id: i32, text: String| {
        mutate(&hh, |d| {
            if let Some(n) = node(d, id) {
                match &mut d.nodes[n].data {
                    NodeData::Text(t) => *t = text,
                    _ => d.set_text(n, &text),
                }
            }
        })
    });
    let hh = h.clone();
    f!("html", move |id: i32, outer: bool| -> String {
        with_dom(&hh, |d| node(d, id).map(|n| if outer { d.outer_html(n) } else { d.inner_html(n) }).unwrap_or_default())
    });
    let hh = h.clone();
    f!("setHtml", move |id: i32, html: String| {
        mutate(&hh, |d| {
            if let Some(n) = node(d, id) {
                d.set_inner_html(n, &html);
            }
        })
    });
    let hh = h.clone();
    f!("children", move |id: i32, elements_only: bool| -> Vec<i32> {
        with_dom(&hh, |d| match node(d, id) {
            Some(n) => d.nodes[n].children.iter().filter(|c| !elements_only || d.element(**c).is_some()).map(|c| *c as i32).collect(),
            None => Vec::new(),
        })
    });
    let hh = h.clone();
    f!("parent", move |id: i32| -> i32 { with_dom(&hh, |d| node(d, id).and_then(|n| d.parent(n)).map_or(-1, |p| p as i32)) });
    let hh = h.clone();
    f!("create", move |tag: String| -> i32 {
        with_dom(&hh, |d| d.create(NodeData::Element(Element { tag: tag.to_ascii_lowercase(), attrs: Vec::new() })) as i32)
    });
    let hh = h.clone();
    f!("createText", move |text: String| -> i32 { with_dom(&hh, |d| d.create(NodeData::Text(text)) as i32) });
    let hh = h.clone();
    f!("insert", move |parent: i32, child: i32, before: i32| -> bool {
        mutate(&hh, |d| match (node(d, parent), node(d, child)) {
            (Some(p), Some(c)) => d.insert_before(p, c, node(d, before)),
            _ => false,
        })
    });
    let hh = h.clone();
    f!("detach", move |id: i32| {
        mutate(&hh, |d| {
            if let Some(n) = node(d, id) {
                d.detach(n);
            }
        })
    });
    let hh = h.clone();
    f!("value", move |id: i32| -> String { with_dom(&hh, |d| node(d, id).map(|n| forms::value(d, n)).unwrap_or_default()) });
    let hh = h.clone();
    f!("setValue", move |id: i32, v: String| {
        mutate(&hh, |d| {
            if let Some(n) = node(d, id) {
                if d.tag(n) == Some("select") {
                    if let Some(i) = forms::options(d, n).iter().position(|o| forms::option_value(d, *o) == v) {
                        forms::choose(d, n, i);
                    }
                } else {
                    forms::set_value(d, n, &v);
                }
            }
        })
    });
    let hh = h.clone();
    f!("style", move |id: i32, prop: String| -> String {
        with_dom(&hh, |d| {
            let prop = kebab(&prop);
            node(d, id).and_then(|n| d.attr(n, "style")).map(|s| style_pairs(s).into_iter().rev().find(|(k, _)| *k == prop).map(|(_, v)| v).unwrap_or_default()).unwrap_or_default()
        })
    });
    let hh = h.clone();
    f!("setStyle", move |id: i32, prop: String, value: String| {
        mutate(&hh, |d| {
            let prop = kebab(&prop);
            if let Some(e) = node(d, id).and_then(|n| d.element_mut(n)) {
                let mut pairs = style_pairs(e.attr("style").unwrap_or(""));
                pairs.retain(|(k, _)| *k != prop);
                if !value.trim().is_empty() {
                    pairs.push((prop, value.trim().to_string()));
                }
                let s = pairs.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("; ");
                e.set_attr("style", &s);
            }
        })
    });
    let hh = h.clone();
    f!("matches", move |id: i32, sel: String| -> bool {
        with_dom(&hh, |d| node(d, id).is_some_and(|n| css::parse_selectors(&sel).iter().any(|s| s.matches(d, n, &Dynamic::default()))))
    });
    let hh = h.clone();
    f!("title", move || -> String { with_page(&hh, |p| p.dom.title()) });
    let hh = h.clone();
    f!("setTitle", move |t: String| {
        with_page(&hh, |p| p.title = t.clone());
        hh.borrow_mut().dirty = true;
    });
    let hh = h.clone();
    f!("url", move || -> String { with_page(&hh, |p| p.url.to_string()) });
    let hh = h.clone();
    f!("resolve", move |href: String| -> Option<String> { with_page(&hh, |p| p.url.join(&href).map(|u| u.to_string())) });
    let hh = h.clone();
    f!("navigate", move |kind: String, href: String| {
        let nav = match kind.as_str() {
            "back" => Some(Nav::Back),
            "forward" => Some(Nav::Forward),
            "reload" => Some(Nav::Reload),
            _ => with_page(&hh, |p| p.url.join(&href).map(|u| Some(Nav::Go(u.to_string()))).unwrap_or(None)),
        };
        if nav.is_some() {
            hh.borrow_mut().navigate = nav;
        }
    });
    let hh = h.clone();
    f!("log", move |level: String, text: String| {
        let line = if level == "log" || level == "info" { text } else { format!("[{level}] {text}") };
        hh.borrow_mut().log(line);
    });
    let hh = h.clone();
    f!("modal", move |text: String, ask: bool| -> bool {
        // Take the callback out while it runs: it may run the UI, which may run more scripts... no, it only waits.
        let cb = hh.borrow_mut().modal.take();
        match cb {
            Some(mut cb) => {
                let r = cb(&text, ask);
                hh.borrow_mut().modal = Some(cb);
                r
            }
            None => {
                hh.borrow_mut().log(format!("[{}] {text}", if ask { "confirm" } else { "alert" }));
                ask
            }
        }
    });
    let hh = h.clone();
    f!("fetch", move |method: String, href: String, body: Option<String>| -> Vec<String> {
        let url = with_page(&hh, |p| p.url.join(&href));
        let Some(url) = url else { return vec!["error".into(), format!("bad address {href}")] };
        let cb = hh.borrow_mut().fetch.take();
        let r = match cb {
            Some(mut cb) => {
                let r = cb(&method.to_ascii_uppercase(), &url, body.as_deref());
                hh.borrow_mut().fetch = Some(cb);
                r
            }
            None => Err("no network".into()),
        };
        match r {
            Ok(f) => vec!["ok".into(), f.status.to_string(), f.body, f.content_type, f.url],
            Err(e) => vec!["error".into(), e],
        }
    });
    let hh = h.clone();
    f!("storage", move |op: String, key: String, value: String| -> Option<String> {
        let mut h = hh.borrow_mut();
        match op.as_str() {
            "get" => h.storage.get(&key).cloned(),
            "set" => {
                h.storage.insert(key, value);
                h.storage_dirty = true;
                None
            }
            "remove" => {
                h.storage.remove(&key);
                h.storage_dirty = true;
                None
            }
            "clear" => {
                h.storage.clear();
                h.storage_dirty = true;
                None
            }
            "key" => key.parse::<usize>().ok().and_then(|i| h.storage.keys().nth(i).cloned()),
            "length" => Some(h.storage.len().to_string()),
            _ => None,
        }
    });
    let hh = h.clone();
    f!("viewport", move || -> Vec<f64> {
        let v = hh.borrow().viewport;
        vec![v.0 as f64, v.1 as f64]
    });
    ctx.globals().set("__n", n)?;
    Ok(())
}

impl Script {
    pub fn new() -> Result<Script, String> {
        let rt = Runtime::new().map_err(|e| e.to_string())?;
        rt.set_memory_limit(MEMORY_LIMIT);
        rt.set_max_stack_size(256 * 1024);
        let deadline: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
        let d = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(move || d.get().is_some_and(|t| Instant::now() > t))));
        let ctx = Context::full(&rt).map_err(|e| e.to_string())?;
        let host: H = Rc::new(RefCell::new(Host::default()));
        let started = Instant::now();
        ctx.with(|c| -> rquickjs::Result<()> {
            install(&c, &host)?;
            c.eval::<(), _>(PRELUDE)
        })
        .map_err(|e| format!("prelude: {e}"))?;
        Ok(Script { rt, ctx, host, deadline, started, stopped: 0 })
    }

    /// Milliseconds since the page's scripts started (`performance.now()`, timers).
    pub fn now_ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    /// Run `f` in the context with `page` moved into the host and a budget;
    /// afterwards the promise jobs it queued run (in the same budget).
    fn enter<R>(&mut self, page: &mut Option<Page>, budget: Duration, f: impl FnOnce(&Ctx) -> rquickjs::Result<R>) -> Option<R> {
        self.host.borrow_mut().page = page.take();
        self.deadline.set(Some(Instant::now() + budget));
        let host = self.host.clone();
        let stopped = Cell::new(false);
        let r = self.ctx.with(|c| {
            let r = match f(&c) {
                Ok(v) => Some(v),
                Err(e) => {
                    report(&c, &host, e, &stopped);
                    None
                }
            };
            r
        });
        // Promise reactions (fetch().then(...)).
        loop {
            match self.rt.execute_pending_job() {
                Ok(true) => continue,
                Ok(false) => break,
                Err(e) => {
                    let msg = e.0.with(|c| {
                        let ex = c.catch();
                        describe(&ex)
                    });
                    if msg.contains("interrupted") {
                        stopped.set(true);
                    }
                    host.borrow_mut().log(format!("[error] {msg}"));
                    if stopped.get() {
                        break;
                    }
                }
            }
        }
        self.deadline.set(None);
        if stopped.get() {
            self.stopped += 1;
            host.borrow_mut().log("[stopped] a script ran too long and was stopped".into());
        }
        *page = self.host.borrow_mut().page.take();
        r
    }

    /// Run the page's scripts (name for errors, source), then `DOMContentLoaded` and `load`.
    pub fn run_scripts(&mut self, page: &mut Option<Page>, scripts: &[(String, String)]) {
        for (name, src) in scripts {
            let src = src.clone();
            let name = name.clone();
            self.enter(page, LOAD_BUDGET, move |c| {
                let mut opts = rquickjs::context::EvalOptions::default();
                opts.global = true;
                opts.strict = false;
                let _ = name;
                c.eval_with_options::<(), _>(src, opts)
            });
        }
        self.dispatch(page, DOCUMENT, "DOMContentLoaded", "");
        self.dispatch(page, usize::MAX, "load", "");
    }

    /// Fire event `kind` at `target` (`usize::MAX`: the window), with `key` for key
    /// events. True if a listener called `preventDefault()`.
    pub fn dispatch(&mut self, page: &mut Option<Page>, target: NodeId, kind: &str, key: &str) -> bool {
        let t = if target == usize::MAX { -1 } else { target as i32 };
        let (kind, key) = (kind.to_string(), key.to_string());
        self.enter(page, BUDGET, move |c| {
            let f: Function = c.globals().get("__dispatch")?;
            f.call::<_, bool>((t, kind, key))
        })
        .unwrap_or(false)
    }

    /// Run the timers that are due and the animation frame callbacks. Returns
    /// the milliseconds until the next timer (None: nothing scheduled).
    pub fn tick(&mut self, page: &mut Option<Page>) -> Option<f64> {
        let now = self.now_ms();
        let next = self
            .enter(page, BUDGET, move |c| {
                let f: Function = c.globals().get("__tick")?;
                f.call::<_, f64>((now,))
            })
            .unwrap_or(-1.0);
        (next >= 0.0).then(|| (next - self.now_ms()).max(0.0))
    }

    /// Is anything scheduled (a timer or an animation frame)?
    pub fn pending(&self) -> bool {
        self.ctx.with(|c| c.globals().get::<_, Function>("__pending").and_then(|f| f.call::<_, bool>(())).unwrap_or(false))
    }

    /// Evaluate an expression (tests and the console): its value as text.
    pub fn eval(&mut self, page: &mut Option<Page>, src: &str) -> Option<String> {
        let src = src.to_string();
        self.enter(page, BUDGET, move |c| {
            let v: rquickjs::Value = c.eval(src)?;
            Ok(describe(&v))
        })
    }

    /// The document changed since the last call.
    pub fn take_dirty(&self) -> bool {
        std::mem::take(&mut self.host.borrow_mut().dirty)
    }
}

/// A value as text: strings as they are, errors with their message.
fn describe(v: &rquickjs::Value) -> String {
    if let Some(s) = v.as_string() {
        return s.to_string().unwrap_or_default();
    }
    if let Some(o) = v.as_object() {
        let msg: Option<String> = o.get("message").ok();
        if let Some(m) = msg {
            let name: Option<String> = o.get("name").ok();
            return format!("{}: {m}", name.unwrap_or_else(|| "Error".into()));
        }
    }
    if v.is_undefined() {
        return "undefined".into();
    }
    if v.is_null() {
        return "null".into();
    }
    if let Some(b) = v.as_bool() {
        return b.to_string();
    }
    if let Some(n) = v.as_number() {
        return if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { n.to_string() };
    }
    format!("{v:?}")
}

fn report(c: &Ctx, host: &H, e: rquickjs::Error, stopped: &Cell<bool>) {
    let msg = match e {
        rquickjs::Error::Exception => describe(&c.catch()),
        other => other.to_string(),
    };
    if msg.contains("interrupted") {
        stopped.set(true);
        return;
    }
    host.borrow_mut().log(format!("[error] {msg}"));
}

#[cfg(test)]
mod tests;
