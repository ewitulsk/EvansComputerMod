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
