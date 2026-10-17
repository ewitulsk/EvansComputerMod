//! The document tree: an arena of nodes (the document, elements, text).

pub type NodeId = usize;

/// The document node is always node 0.
pub const DOCUMENT: NodeId = 0;

#[derive(Clone, Debug, PartialEq)]
pub enum NodeData {
    Document,
    Element(Element),
    Text(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    /// Lower-case tag name.
    pub tag: String,
    /// Attributes in source order, names lower-cased.
    pub attrs: Vec<(String, String)>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn set_attr(&mut self, name: &str, value: &str) {
        match self.attrs.iter_mut().find(|(k, _)| k == name) {
            Some(a) => a.1 = value.to_string(),
            None => self.attrs.push((name.to_string(), value.to_string())),
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.attr("id")
    }

    pub fn has_class(&self, c: &str) -> bool {
        self.attr("class").is_some_and(|v| v.split_ascii_whitespace().any(|x| x == c))
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub data: NodeData,
}

#[derive(Clone, Debug)]
pub struct Dom {
    pub nodes: Vec<Node>,
}

impl Default for Dom {
    fn default() -> Self {
        Dom::new()
    }
}

impl Dom {
    pub fn new() -> Dom {
        Dom { nodes: vec![Node { parent: None, children: Vec::new(), data: NodeData::Document }] }
    }

    pub fn push(&mut self, parent: NodeId, data: NodeData) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node { parent: Some(parent), children: Vec::new(), data });
        self.nodes[parent].children.push(id);
        id
    }

    pub fn element(&self, id: NodeId) -> Option<&Element> {
        match &self.nodes.get(id)?.data {
            NodeData::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn element_mut(&mut self, id: NodeId) -> Option<&mut Element> {
        match &mut self.nodes.get_mut(id)?.data {
            NodeData::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn tag(&self, id: NodeId) -> Option<&str> {
        self.element(id).map(|e| e.tag.as_str())
    }

    pub fn attr(&self, id: NodeId, name: &str) -> Option<&str> {
        self.element(id).and_then(|e| e.attr(name))
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.get(id).and_then(|n| n.parent)
    }

    /// The parent if it is an element.
    pub fn parent_element(&self, id: NodeId) -> Option<NodeId> {
        self.parent(id).filter(|p| self.element(*p).is_some())
    }

    /// Element children only.
    pub fn element_children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes[id].children.iter().copied().filter(move |c| self.element(*c).is_some())
    }

    /// `id` and every node under it, in document order.
    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            out.push(n);
            for c in self.nodes[n].children.iter().rev() {
                stack.push(*c);
            }
        }
        out
    }

    /// The first element with this tag, in document order.
    pub fn find(&self, tag: &str) -> Option<NodeId> {
        self.descendants(DOCUMENT).into_iter().find(|n| self.tag(*n) == Some(tag))
    }

    /// Every element with this tag, in document order.
    pub fn find_all(&self, tag: &str) -> Vec<NodeId> {
        self.descendants(DOCUMENT).into_iter().filter(|n| self.tag(*n) == Some(tag)).collect()
    }

    /// The element with this id attribute.
    pub fn by_id(&self, id: &str) -> Option<NodeId> {
        self.descendants(DOCUMENT).into_iter().find(|n| self.element(*n).and_then(|e| e.id()) == Some(id))
    }

    /// All the text under `id`, joined.
    pub fn text_content(&self, id: NodeId) -> String {
        let mut s = String::new();
        for n in self.descendants(id) {
            if let NodeData::Text(t) = &self.nodes[n].data {
                s.push_str(t);
            }
        }
        s
    }

    /// The nearest element, `id` itself first, that `f` accepts.
    pub fn closest(&self, id: NodeId, f: impl Fn(&Element) -> bool) -> Option<NodeId> {
        let mut cur = Some(id);
        while let Some(n) = cur {
            if let Some(e) = self.element(n) {
                if f(e) {
                    return Some(n);
                }
            }
            cur = self.parent(n);
        }
        None
    }

    /// The page title (`<title>`), whitespace collapsed.
    pub fn title(&self) -> String {
        self.find("title").map(|t| collapse_ws(&self.text_content(t))).unwrap_or_default()
    }

    /// Position among the parent's element children (0-based) and how many there are.
    pub fn element_index(&self, id: NodeId) -> (usize, usize) {
        let Some(p) = self.parent(id) else { return (0, 1) };
        let sibs: Vec<NodeId> = self.element_children(p).collect();
        (sibs.iter().position(|s| *s == id).unwrap_or(0), sibs.len())
    }

    /// The element sibling before `id`.
    pub fn prev_element(&self, id: NodeId) -> Option<NodeId> {
        let p = self.parent(id)?;
        let mut prev = None;
        for c in self.element_children(p) {
            if c == id {
                return prev;
            }
            prev = Some(c);
        }
        None
    }
}

/// Whitespace runs to one space, trimmed.
pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ------------------------------------------------------------------ changes (scripts)

/// Elements that are written without an end tag.
const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

fn esc_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

impl Dom {
    /// A new node outside the tree (`createElement`, `createTextNode`).
    pub fn create(&mut self, data: NodeData) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node { parent: None, children: Vec::new(), data });
        id
    }

    /// Take `node` out of its parent (it and its subtree stay, detached).
    pub fn detach(&mut self, node: NodeId) {
        if let Some(p) = self.nodes.get(node).and_then(|n| n.parent) {
            self.nodes[p].children.retain(|c| *c != node);
        }
        if let Some(n) = self.nodes.get_mut(node) {
            n.parent = None;
        }
    }

    /// Is `a` `b` or one of its ancestors?
    pub fn contains(&self, a: NodeId, b: NodeId) -> bool {
        let mut cur = Some(b);
        while let Some(n) = cur {
            if n == a {
                return true;
            }
            cur = self.parent(n);
        }
        false
    }

    /// Move `child` to the end of `parent`'s children (false: it would contain its own parent).
    pub fn append(&mut self, parent: NodeId, child: NodeId) -> bool {
        self.insert_before(parent, child, None)
    }

    /// Move `child` into `parent` before `before` (None: at the end).
    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, before: Option<NodeId>) -> bool {
        if parent >= self.nodes.len() || child >= self.nodes.len() || child == crate::dom::DOCUMENT || self.contains(child, parent) {
            return false;
        }
        self.detach(child);
        let at = before.and_then(|b| self.nodes[parent].children.iter().position(|c| *c == b)).unwrap_or(self.nodes[parent].children.len());
        self.nodes[parent].children.insert(at, child);
        self.nodes[child].parent = Some(parent);
        true
    }

    /// Copy `from`'s node `src` (and its subtree) in as the last child of `parent`.
    pub fn graft(&mut self, parent: NodeId, from: &Dom, src: NodeId) {
        let id = self.create(from.nodes[src].data.clone());
        self.append(parent, id);
        for &c in &from.nodes[src].children {
            self.graft(id, from, c);
        }
    }

    /// Replace `node`'s children with what `html` parses to (`innerHTML = ...`).
    pub fn set_inner_html(&mut self, node: NodeId, html: &str) {
        for c in std::mem::take(&mut self.nodes[node].children) {
            self.nodes[c].parent = None;
        }
        let frag = crate::html::parse(html);
        for &c in &frag.nodes[crate::dom::DOCUMENT].children {
            // A fragment parsed as a document may have wrapped itself in html/body: unwrap them.
            if matches!(frag.tag(c), Some("html") | Some("body") | Some("head")) {
                for &g in &frag.nodes[c].children {
                    if frag.tag(g) == Some("head") || frag.tag(g) == Some("body") {
                        for &h in &frag.nodes[g].children {
                            self.graft(node, &frag, h);
                        }
                    } else {
                        self.graft(node, &frag, g);
                    }
                }
            } else {
                self.graft(node, &frag, c);
            }
        }
    }

    /// The HTML of `node`'s children (`innerHTML`).
    pub fn inner_html(&self, node: NodeId) -> String {
        let mut out = String::new();
        for &c in &self.nodes[node].children {
            self.write_html(c, &mut out);
        }
        out
    }

    /// The HTML of `node` itself (`outerHTML`).
    pub fn outer_html(&self, node: NodeId) -> String {
        let mut out = String::new();
        self.write_html(node, &mut out);
        out
    }

    fn write_html(&self, node: NodeId, out: &mut String) {
        match &self.nodes[node].data {
            NodeData::Text(t) => {
                let raw = self.parent(node).and_then(|p| self.tag(p)).is_some_and(|t| t == "script" || t == "style");
                out.push_str(&if raw { t.clone() } else { esc_text(t) });
            }
            NodeData::Element(e) => {
                out.push('<');
                out.push_str(&e.tag);
                for (k, v) in &e.attrs {
                    out.push(' ');
                    out.push_str(k);
                    if !v.is_empty() {
                        out.push_str("=\"");
                        out.push_str(&v.replace('&', "&amp;").replace('"', "&quot;"));
                        out.push('"');
                    }
                }
                out.push('>');
                if VOID.contains(&e.tag.as_str()) {
                    return;
                }
                for &c in &self.nodes[node].children {
                    self.write_html(c, out);
                }
                out.push_str("</");
                out.push_str(&e.tag);
                out.push('>');
            }
            NodeData::Document => {
                for &c in &self.nodes[node].children {
                    self.write_html(c, out);
                }
            }
        }
    }
}

#[cfg(test)]
mod change_tests {
    use super::*;
    use crate::html;

    #[test]
    fn create_move_and_serialize() {
        let mut d = html::parse("<ul id=l><li>a</li></ul><p id=p>x</p>");
        let ul = d.by_id("l").unwrap();
        let li = d.create(NodeData::Element(Element { tag: "li".into(), attrs: vec![("class".into(), "new".into())] }));
        let t = d.create(NodeData::Text("b & c".into()));
        assert!(d.append(li, t));
        assert!(d.append(ul, li));
        assert_eq!(d.inner_html(ul), "<li>a</li><li class=\"new\">b &amp; c</li>");
        let first = d.element_children(ul).next().unwrap();
        assert!(d.insert_before(ul, li, Some(first)));
        assert_eq!(d.text_content(ul), "b & ca");
        assert!(!d.append(li, ul), "a node can't go inside itself");
        let p = d.by_id("p").unwrap();
        d.set_inner_html(p, "<b>bold</b> and <br>more");
        assert_eq!(d.inner_html(p), "<b>bold</b> and <br>more");
        assert_eq!(d.find("b").map(|b| d.text_content(b)), Some("bold".into()));
        d.detach(p);
        assert!(d.by_id("p").is_none() || d.parent(p).is_none());
    }
}
