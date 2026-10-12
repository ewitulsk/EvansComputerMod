//! The GUI designer's document: a `.ui.json` layout as an editable tree.
//!
//! The document is the file's JSON (`ecm_ui::layout_file` loads the same
//! text): every node is `{kind: {props..., "children": [...]}}`. Props the
//! designer doesn't edit (responsive branches, a scroll view's `child`)
//! are kept as they are. Widgets are addressed by their path of child
//! indices from the root.
//!
//! A control's events (`on_click`, `on_change`, …) name handlers; the
//! designer never asks for handler names: [`sync_handlers`] sets each
//! control's `on_<event>` to the handler of the "When Widget" block that
//! listens to it, and removes handlers no block listens to.

use crate::codegen::handler_name;
use crate::graph::Graph;
use ecm_ui::layout_file;
use ecm_ui::Json;

/// A widget node.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: String,
    /// Props other than `children`, in file order.
    pub props: Vec<(String, Json)>,
    pub children: Vec<Node>,
}

/// A layout file.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutDoc {
    pub title: String,
    pub root: Node,
}

/// Widgets in the designer's palette, in order.
pub const PALETTE: &[&str] =
    &["label", "button", "textfield", "checkbox", "toggle", "slider", "list", "row", "column", "stack", "grid", "panel", "scroll", "spacer"];

/// Kinds that hold children the designer can drop into.
pub fn is_container(kind: &str) -> bool {
    matches!(kind, "row" | "column" | "stack" | "grid" | "panel" | "scroll" | "dialog")
}

/// The events a kind of widget sends.
pub fn events_of(kind: &str) -> &'static [&'static str] {
    match kind {
        "button" | "label" | "image" => &["click"],
        "textfield" => &["change", "submit"],
        "checkbox" | "toggle" | "slider" => &["change"],
        "list" | "menu" | "tabs" => &["select"],
        _ => &[],
    }
}

/// How a prop is edited in the inspector.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PropType {
    Str,
    Int,
    Bool,
    Color,
}

/// Bool props that are on when absent.
pub fn default_true(key: &str) -> bool {
    matches!(key, "enabled" | "wrap")
}

/// The props the inspector shows for a kind.
pub fn props_of(kind: &str) -> Vec<(&'static str, PropType)> {
    use PropType::*;
    let mut v = vec![("id", Str)];
    match kind {
        "label" => v.extend([("text", Str), ("style", Str), ("size", Int), ("color", Color), ("wrap", Bool)]),
        "button" => v.extend([("text", Str), ("style", Str), ("enabled", Bool)]),
        "textfield" => v.extend([("placeholder", Str), ("enabled", Bool)]),
        "checkbox" | "toggle" => v.extend([("text", Str), ("checked", Bool)]),
        "slider" => v.extend([("min", Int), ("max", Int), ("value", Int), ("step", Int)]),
        "row" | "column" | "stack" => v.extend([("gap", Int), ("padding", Int), ("align", Str)]),
        "grid" => v.extend([("columns", Int), ("gap", Int), ("padding", Int)]),
        "panel" => v.extend([("padding", Int), ("fill", Color), ("radius", Int)]),
        "list" | "menu" | "tabs" => v.extend([("selected", Int)]),
        _ => {}
    }
    v.extend([("width", Int), ("height", Int), ("grow", Int)]);
    v
}

impl Node {
    pub fn new(kind: &str) -> Node {
        Node { kind: kind.to_string(), props: Vec::new(), children: Vec::new() }
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        self.props.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Json::as_str)
    }

    pub fn id(&self) -> Option<&str> {
        self.get_str("id")
    }

    pub fn set(&mut self, key: &str, value: Json) {
        match self.props.iter_mut().find(|(k, _)| k == key) {
            Some(e) => e.1 = value,
            None => self.props.push((key.to_string(), value)),
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.props.retain(|(k, _)| k != key);
    }

    fn from_json(j: &Json, path: &str) -> Result<Node, String> {
        let obj = j.as_object().ok_or_else(|| format!("{path} must be an object"))?;
        let [(kind, body)] = obj else { return Err(format!("{path} must have exactly one key, the widget kind")) };
        let body = body.as_object().ok_or_else(|| format!("{path}.{kind} must be an object"))?;
        let mut n = Node::new(kind);
        for (k, v) in body {
            if k == "children" {
                for (i, c) in v.as_array().ok_or_else(|| format!("{path}.{kind}.children must be a list"))?.iter().enumerate() {
                    n.children.push(Node::from_json(c, &format!("{path}.{kind}.children[{i}]"))?);
                }
            } else {
                n.props.push((k.clone(), v.clone()));
            }
        }
        Ok(n)
    }

    pub fn to_json(&self) -> Json {
        let mut body = self.props.clone();
        if !self.children.is_empty() || is_container(&self.kind) && self.get("child").is_none() {
            body.push(("children".into(), Json::Arr(self.children.iter().map(Node::to_json).collect())));
        }
        Json::Obj(vec![(self.kind.clone(), Json::Obj(body))])
    }

    /// Every node with its path, in tree order.
    pub fn walk(&self) -> Vec<(Vec<usize>, &Node)> {
        let mut out = Vec::new();
        fn go<'a>(n: &'a Node, path: &mut Vec<usize>, out: &mut Vec<(Vec<usize>, &'a Node)>) {
            out.push((path.clone(), n));
            for (i, c) in n.children.iter().enumerate() {
                path.push(i);
                go(c, path, out);
                path.pop();
            }
        }
        go(self, &mut Vec::new(), &mut out);
        out
    }
}

/// `0.2.1` ↔ `[0, 2, 1]` (the root is the empty path, written `r`).
pub fn path_key(path: &[usize]) -> String {
    if path.is_empty() {
        "r".into()
    } else {
        path.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(".")
    }
}

pub fn parse_path(key: &str) -> Option<Vec<usize>> {
    if key == "r" {
        return Some(Vec::new());
    }
    key.split('.').map(|s| s.parse().ok()).collect()
}

/// Where a dropped widget goes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Drop {
    /// Last child of the target.
    Inside,
    /// Right after the target, in its parent.
    After,
}

impl LayoutDoc {
    /// A new layout: a padded column.
    pub fn new(title: &str) -> LayoutDoc {
        let mut root = Node::new("column");
        root.set("gap", Json::Int(8));
        root.set("padding", Json::Int(12));
        LayoutDoc { title: title.to_string(), root }
    }

    pub fn parse(text: &str) -> Result<LayoutDoc, String> {
        // The toolkit must accept it too: unknown widgets and props are errors.
        layout_file::load(text)?;
        let j = Json::parse(text)?;
        let title = j.get("window").and_then(Json::as_str).unwrap_or("").to_string();
        let root = Node::from_json(j.get("root").ok_or("the layout has no root")?, "root")?;
        Ok(LayoutDoc { title, root })
    }

    pub fn to_json(&self) -> Json {
        Json::Obj(vec![("window".into(), Json::Str(self.title.clone())), ("root".into(), self.root.to_json())])
    }

    /// The `.ui.json` text, one widget per line for readable diffs.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        fn node(n: &Node, depth: usize, out: &mut String) {
            let pad = "  ".repeat(depth);
            let mut body: Vec<String> = n.props.iter().map(|(k, v)| format!("{}:{}", Json::Str(k.clone()).to_string(), v.to_string())).collect();
            let kids = !n.children.is_empty() || is_container(&n.kind) && n.get("child").is_none();
            if kids && !n.children.is_empty() {
                let mut s = String::from("\"children\": [\n");
                for (i, c) in n.children.iter().enumerate() {
                    node(c, depth + 2, &mut s);
                    if i + 1 < n.children.len() {
                        s.push(',');
                    }
                    s.push('\n');
                }
                s.push_str(&format!("{pad}  ]"));
                body.push(s);
            } else if kids {
                body.push("\"children\": []".into());
            }
            out.push_str(&format!("{pad}{{ {}: {{ {} }} }}", Json::Str(n.kind.clone()).to_string(), body.join(", ")));
        }
        out.push_str(&format!("{{ \"window\": {},\n  \"root\":\n", Json::Str(self.title.clone()).to_string()));
        node(&self.root, 2, &mut out);
        out.push_str("\n}\n");
        out
    }

    pub fn node(&self, path: &[usize]) -> Option<&Node> {
        let mut n = &self.root;
        for &i in path {
            n = n.children.get(i)?;
        }
        Some(n)
    }

    pub fn node_mut(&mut self, path: &[usize]) -> Option<&mut Node> {
        let mut n = &mut self.root;
        for &i in path {
            n = n.children.get_mut(i)?;
        }
        Some(n)
    }

    /// The path of the widget with this id.
    pub fn find_id(&self, id: &str) -> Option<Vec<usize>> {
        self.root.walk().into_iter().find(|(_, n)| n.id() == Some(id)).map(|(p, _)| p)
    }

    /// Every widget id in the layout, in tree order.
    pub fn ids(&self) -> Vec<String> {
        self.root.walk().into_iter().filter_map(|(_, n)| n.id().map(String::from)).collect()
    }

    /// `kind1`, `kind2`, … : the first id not in use.
    pub fn fresh_id(&self, kind: &str) -> String {
        let used = self.ids();
        (1..).map(|i| format!("{kind}{i}")).find(|c| !used.contains(c)).unwrap()
    }

    /// A new widget of `kind` with sensible defaults and a fresh id.
    pub fn new_widget(&self, kind: &str) -> Node {
        let mut n = Node::new(kind);
        let id = self.fresh_id(kind);
        match kind {
            "label" => {
                n.set("id", Json::Str(id));
                n.set("text", Json::Str("Label".into()));
            }
            "button" => {
                n.set("id", Json::Str(id));
                n.set("text", Json::Str("Button".into()));
            }
            "textfield" => {
                n.set("id", Json::Str(id));
                n.set("placeholder", Json::Str("Type here".into()));
            }
            "checkbox" | "toggle" => {
                n.set("id", Json::Str(id));
                n.set("text", Json::Str("Option".into()));
            }
            "slider" => {
                n.set("id", Json::Str(id));
                n.set("min", Json::Int(0));
                n.set("max", Json::Int(100));
            }
            "list" => n.set("id", Json::Str(id)),
            "row" | "column" | "stack" => n.set("gap", Json::Int(8)),
            "grid" => {
                n.set("columns", Json::Int(2));
                n.set("gap", Json::Int(8));
            }
            "panel" => n.set("padding", Json::Int(8)),
            "scroll" => n.set("height", Json::Int(120)),
            _ => {}
        }
        n
    }

    /// Insert `node` relative to the widget at `target`; returns its path.
    pub fn insert(&mut self, node: Node, target: &[usize], how: Drop) -> Option<Vec<usize>> {
        let tkind = self.node(target)?.kind.clone();
        if how == Drop::Inside || target.is_empty() {
            if !is_container(&tkind) {
                return if target.is_empty() { None } else { self.insert(node, target, Drop::After) };
            }
            let t = self.node_mut(target)?;
            t.children.push(node);
            let mut p = target.to_vec();
            p.push(t.children.len() - 1);
            Some(p)
        } else {
            let (last, parent) = target.split_last()?;
            let p = self.node_mut(parent)?;
            p.children.insert(last + 1, node);
            let mut out = parent.to_vec();
            out.push(last + 1);
            Some(out)
        }
    }

    /// Drop a widget where the player released it: inside a container,
    /// after anything else.
    pub fn drop_on(&mut self, node: Node, target: &[usize]) -> Option<Vec<usize>> {
        let how = if self.node(target).is_some_and(|n| is_container(&n.kind)) { Drop::Inside } else { Drop::After };
        self.insert(node, target, how)
    }

    /// Remove the widget at `path` (not the root).
    pub fn remove(&mut self, path: &[usize]) -> Option<Node> {
        let (last, parent) = path.split_last()?;
        let p = self.node_mut(parent)?;
        (*last < p.children.len()).then(|| p.children.remove(*last))
    }

    /// Move the widget at `from` onto `target` (see [`LayoutDoc::drop_on`]).
    /// Refuses to move a widget into itself.
    pub fn move_to(&mut self, from: &[usize], target: &[usize]) -> Option<Vec<usize>> {
        if from.is_empty() || target.starts_with(from) {
            return None;
        }
        let node = self.node(from)?.clone();
        // Work out the target's path after the removal.
        let mut t = target.to_vec();
        let (flast, fparent) = from.split_last()?;
        if t.len() > fparent.len() && t.starts_with(fparent) && t[fparent.len()] > *flast {
            t[fparent.len()] -= 1;
        }
        self.remove(from)?;
        self.drop_on(node, &t)
    }

    /// Move a widget one place earlier (-1) or later (+1) among its siblings.
    pub fn shift(&mut self, path: &[usize], delta: i32) -> Option<Vec<usize>> {
        let (last, parent) = path.split_last()?;
        let p = self.node_mut(parent)?;
        let j = *last as i32 + delta;
        if j < 0 || j as usize >= p.children.len() {
            return None;
        }
        p.children.swap(*last, j as usize);
        let mut out = parent.to_vec();
        out.push(j as usize);
        Some(out)
    }

    /// The tree the designer previews: every node's id replaced by its path
    /// (`d:<path>`), so the board reports which widget was pointed at, and
    /// handlers dropped. The selected widget gets an outline.
    pub fn preview(&self, selected: Option<&[usize]>, outline: u32) -> Result<ecm_ui::Element, String> {
        fn conv(n: &Node, path: &mut Vec<usize>, sel: Option<&[usize]>, outline: u32) -> Json {
            let mut body: Vec<(String, Json)> =
                n.props.iter().filter(|(k, _)| k != "id" && !k.starts_with("on_")).cloned().collect();
            body.insert(0, ("id".into(), Json::Str(format!("d:{}", path_key(path)))));
            if !n.children.is_empty() {
                let mut kids = Vec::new();
                for (i, c) in n.children.iter().enumerate() {
                    path.push(i);
                    kids.push(conv(c, path, sel, outline));
                    path.pop();
                }
                body.push(("children".into(), Json::Arr(kids)));
            }
            let j = Json::Obj(vec![(n.kind.clone(), Json::Obj(body))]);
            if sel == Some(path.as_slice()) {
                // Wrap in an outline that lays out like the widget.
                let mut wrap: Vec<(String, Json)> = vec![
                    ("fill".into(), Json::Int(0)),
                    ("border_w".into(), Json::Int(2)),
                    ("border_color".into(), Json::Int(outline as i64)),
                    ("radius".into(), Json::Int(2)),
                ];
                for k in ["grow", "width", "height", "align"] {
                    if let Some(v) = n.get(k) {
                        wrap.push((k.into(), v.clone()));
                    }
                }
                wrap.push(("children".into(), Json::Arr(vec![j])));
                return Json::Obj(vec![("panel".into(), Json::Obj(wrap))]);
            }
            j
        }
        let j = conv(&self.root, &mut Vec::new(), selected, outline);
        layout_file::element(&j, "root")
    }
}

/// Set each widget's `on_<event>` to the handler of the "When Widget"
/// block listening to it, and drop handlers no block listens to.
pub fn sync_handlers(doc: &mut LayoutDoc, graph: &Graph) {
    let listening: Vec<(String, String)> = graph
        .blocks
        .iter()
        .filter(|b| b.ty == "on_ui_event")
        .map(|b| (b.get("widget").unwrap_or("").to_string(), b.get("event").unwrap_or("click").to_string()))
        .collect();
    fn go(n: &mut Node, listening: &[(String, String)]) {
        let id = n.id().map(String::from);
        n.props.retain(|(k, _)| !k.starts_with("on_"));
        if let Some(id) = id {
            for ev in events_of(&n.kind) {
                if listening.iter().any(|(w, e)| *w == id && e == ev) {
                    n.props.push((format!("on_{ev}"), Json::Str(handler_name(&id, ev))));
                }
            }
        }
        for c in &mut n.children {
            go(c, listening);
        }
    }
    go(&mut doc.root, &listening);
}

/// A widget id was renamed: make the graph's widget blocks follow.
pub fn rename_widget(graph: &mut Graph, old: &str, new: &str) {
    for b in &mut graph.blocks {
        if (b.ty == "on_ui_event" || b.ty.starts_with("ui_")) && b.get("widget") == Some(old) {
            b.set("widget", new);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Registry;

    const NOTES: &str = r#"{ "window": "Notes",
  "root": { "column": { "gap": 8, "padding": 12, "children": [
    { "row": { "gap": 8, "children": [
      { "textfield": { "id": "entry", "placeholder": "New note" } },
      { "button": { "text": "Add", "on_click": "add_note" } } ] } },
    { "list": { "id": "rows" } } ] } } }"#;

    #[test]
    fn layouts_round_trip_through_the_document() {
        let doc = LayoutDoc::parse(NOTES).unwrap();
        assert_eq!(doc.title, "Notes");
        assert_eq!(doc.root.children[0].children[1].get_str("on_click"), Some("add_note"));
        // Compact JSON and the readable text both load back to the same document.
        for text in [doc.to_json().to_string(), doc.to_text()] {
            assert_eq!(LayoutDoc::parse(&text).unwrap(), doc, "{text}");
            ecm_ui::layout_file::load(&text).unwrap();
        }
        assert!(LayoutDoc::parse(r#"{"root": {"gizmo": {}}}"#).is_err());
    }

    #[test]
    fn widgets_drop_inside_containers_and_after_leaves() {
        let mut doc = LayoutDoc::new("T");
        let b = doc.new_widget("button");
        assert_eq!(b.id(), Some("button1"));
        let pb = doc.drop_on(b, &[]).unwrap();
        assert_eq!(pb, vec![0]);
        let l = doc.new_widget("label");
        assert_eq!(doc.drop_on(l, &[0]).unwrap(), vec![1], "dropped on a button: goes after it");
        let row = doc.new_widget("row");
        let pr = doc.drop_on(row, &[0]).unwrap();
        assert_eq!(pr, vec![1]);
        let b2 = doc.new_widget("button");
        assert_eq!(b2.id(), Some("button2"));
        assert_eq!(doc.drop_on(b2, &pr).unwrap(), vec![1, 0], "dropped on a row: goes inside");
        // Move the label (now at [2]) into the row, before nothing else.
        assert_eq!(doc.move_to(&[2], &[1]).unwrap(), vec![1, 1]);
        assert!(doc.move_to(&[1], &[1, 0]).is_none(), "not into itself");
        // Moving an earlier sibling past a later one fixes the target path.
        assert_eq!(doc.move_to(&[0], &[1]).unwrap(), vec![0, 2]);
        assert_eq!(doc.ids(), ["button2", "label1", "button1"]);
        assert_eq!(doc.shift(&[0, 2], -1).unwrap(), vec![0, 1]);
        assert_eq!(doc.remove(&[0, 0]).unwrap().id(), Some("button2"));
        assert!(doc.remove(&[]).is_none());
        ecm_ui::layout_file::load(&doc.to_text()).unwrap();
    }

    #[test]
    fn handlers_follow_the_graph_and_renames() {
        let reg = Registry::builtin();
        let mut doc = LayoutDoc::parse(NOTES).unwrap();
        let mut g = Graph::new("notes");
        let e = g.add_block(reg.find("on_ui_event").unwrap(), 0.0, 0.0);
        g.block_mut(e).unwrap().set("widget", "entry");
        g.block_mut(e).unwrap().set("event", "submit");
        let s = g.add_block(reg.find("ui_set_text").unwrap(), 0.0, 0.0);
        g.block_mut(s).unwrap().set("widget", "entry");
        sync_handlers(&mut doc, &g);
        assert_eq!(doc.root.children[0].children[0].get_str("on_submit"), Some("entry_submit"));
        assert_eq!(doc.root.children[0].children[1].get_str("on_click"), None, "no block listens to the button");
        rename_widget(&mut g, "entry", "note");
        assert_eq!(g.block(e).unwrap().get("widget"), Some("note"));
        assert_eq!(g.block(s).unwrap().get("widget"), Some("note"));
    }

    #[test]
    fn the_preview_names_widgets_by_path_and_outlines_the_selection() {
        let doc = LayoutDoc::parse(NOTES).unwrap();
        let el = doc.preview(Some(&[0, 1]), 0xff0000ff).unwrap();
        assert_eq!(el.id.as_deref(), Some("d:r"));
        let row = &el.children[0];
        assert_eq!(row.id.as_deref(), Some("d:0"));
        let wrap = &row.children[1];
        assert_eq!(wrap.kind, ecm_ui::Kind::Panel);
        assert_eq!(wrap.props.border_color, Some(0xff0000ff));
        assert_eq!(wrap.children[0].id.as_deref(), Some("d:0.1"));
        assert!(wrap.children[0].handlers.is_empty(), "handlers are not live in the designer");
        assert_eq!(parse_path("0.1"), Some(vec![0, 1]));
        assert_eq!(parse_path("r"), Some(vec![]));
        assert_eq!(path_key(&[2, 0]), "2.0");
    }
}
