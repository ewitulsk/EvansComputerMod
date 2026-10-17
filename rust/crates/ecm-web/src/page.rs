//! A page: its address, DOM and style sheets, rendered for a viewport.

use crate::css::{Dynamic, Stylesheet};
use crate::dom::{Dom, NodeId};
use crate::html;
use crate::layout::{self, Env};
use crate::paint::{self, DisplayList};
use crate::style::{self, Style, UaColors};
use crate::url::Url;

/// Where a page's style sheets come from, in document order.
#[derive(Clone, Debug, PartialEq)]
pub enum SheetSource {
    /// A `<style>` element's text.
    Inline(String),
    /// A `<link rel=stylesheet>` (loaded or not).
    Link(Url),
}

#[derive(Clone, Debug)]
pub struct Page {
    pub url: Url,
    pub dom: Dom,
    /// Parsed sheets in document order (a link that didn't load is left out).
    pub sheets: Vec<Stylesheet>,
    pub title: String,
}

/// A page laid out and painted for one viewport.
#[derive(Clone, Debug)]
pub struct Rendered {
    pub list: DisplayList,
    pub styles: Vec<Option<Style>>,
    /// The style sheets use `:hover`/`:focus`: pointer moves need a restyle.
    pub dynamic_selectors: bool,
}

impl Page {
    /// Parse a document; only `<style>` sheets so far (see [`Page::load_sheets`]).
    pub fn parse(url: Url, src: &str) -> Page {
        let dom = html::parse(src);
        let title = dom.title();
        let mut p = Page { url, dom, sheets: Vec::new(), title };
        p.load_sheets(&mut |_| None);
        p
    }

    /// A plain-text document shown as it is.
    pub fn plain_text(url: Url, text: &str) -> Page {
        let esc = text.replace('&', "&amp;").replace('<', "&lt;");
        Page::parse(url, &format!("<pre style='white-space: pre-wrap'>{esc}</pre>"))
    }

    /// The page's style sheet sources in document order.
    pub fn sheet_sources(&self) -> Vec<SheetSource> {
        let mut out = Vec::new();
        for n in self.dom.descendants(crate::dom::DOCUMENT) {
            match self.dom.tag(n) {
                Some("style") => {
                    let media = self.dom.attr(n, "media").unwrap_or("");
                    let text = self.dom.text_content(n);
                    out.push(SheetSource::Inline(if media.is_empty() { text } else { format!("@media {media} {{ {text} }}") }));
                }
                Some("link") => {
                    let rel = self.dom.attr(n, "rel").unwrap_or("").to_ascii_lowercase();
                    if rel.split_ascii_whitespace().any(|r| r == "stylesheet") && !rel.contains("alternate") {
                        if let Some(u) = self.dom.attr(n, "href").and_then(|h| self.url.join(h)) {
                            out.push(SheetSource::Link(u));
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// (Re)build the sheets; `fetch` returns a linked sheet's text (None: it didn't load).
    pub fn load_sheets(&mut self, fetch: &mut dyn FnMut(&Url) -> Option<String>) {
        let mut sheets = Vec::new();
        for src in self.sheet_sources() {
            match src {
                SheetSource::Inline(t) => sheets.push(Stylesheet::parse(&t)),
                SheetSource::Link(u) => {
                    if let Some(t) = fetch(&u) {
                        sheets.push(Stylesheet::parse(&t));
                    }
                }
            }
        }
        self.sheets = sheets;
    }

    /// Style, lay out and paint for a viewport of `w x h` lu.
    pub fn render(&self, w: f32, h: f32, ua: &UaColors, dynamic: &Dynamic, image_size: &dyn Fn(NodeId) -> Option<(f32, f32)>) -> Rendered {
        let ua_sheet = style::ua_sheet(ua);
        let env = style::StyleEnv { ua_colors: ua, ua_sheet: &ua_sheet, sheets: &self.sheets, viewport: (w, h), dynamic };
        let styles = style::compute(&self.dom, &env);
        let lenv = Env { dom: &self.dom, styles: &styles, viewport: (w, h), ua, image_size };
        let root = layout::layout_document(&lenv);
        let list = paint::paint(&root, &lenv);
        let dynamic_selectors = self.sheets.iter().any(|s| s.rules.iter().any(|r| r.selectors.iter().any(|sel| sel.uses_hover())));
        Rendered { list, styles, dynamic_selectors }
    }

    /// The link (`<a href>`) at or around `node`, resolved.
    pub fn link_at(&self, node: NodeId) -> Option<Url> {
        let a = self.dom.closest(node, |e| (e.tag == "a" || e.tag == "area") && e.attr("href").is_some())?;
        self.url.join(self.dom.attr(a, "href")?)
    }

    /// The element a `#fragment` names (`id`, or `<a name>`).
    pub fn anchor(&self, frag: &str) -> Option<NodeId> {
        if frag.is_empty() {
            return None;
        }
        self.dom.by_id(frag).or_else(|| self.dom.find_all("a").into_iter().find(|a| self.dom.attr(*a, "name") == Some(frag)))
    }

    /// Every link on the page in document order: (text, address).
    pub fn links(&self) -> Vec<(String, Url)> {
        self.dom
            .find_all("a")
            .into_iter()
            .filter_map(|a| {
                let u = self.url.join(self.dom.attr(a, "href")?)?;
                Some((crate::dom::collapse_ws(&self.dom.text_content(a)), u))
            })
            .collect()
    }
}
