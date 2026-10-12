//! The diff engine: turns a new display-list scene into the minimal TreeOps
//! that change the committed one into it.
//!
//! A scene is a tree of [`SceneNode`]s, each with a stable string key (made
//! from the widget key and the node's part). The engine remembers the
//! committed tree and the node ids it handed out, and emits
//!
//! 1. `Remove` for nodes that disappeared (only the topmost of a removed
//!    subtree), first when nothing inside survives, last otherwise;
//! 2. in pre-order of the new tree: `Create` for new nodes (with every prop),
//!    `Move` for nodes whose parent or position changed, and `Set` with only
//!    the props whose values changed.
//!
//! Props a node no longer sets are reset to the protocol default. Node ids
//! are handed out in order of first appearance and never reused, so two
//! front ends that build the same scenes produce identical bytes.

use ecm_ui_proto::{NodeKind, Prop, Rect, TreeOp, ROOT_NODE};
use std::collections::{HashMap, HashSet};

/// One node of a scene: what the toolkit wants on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneNode {
    pub key: String,
    pub kind: NodeKind,
    pub props: Vec<Prop>,
    pub children: Vec<SceneNode>,
}

impl SceneNode {
    pub fn new(key: impl Into<String>, kind: NodeKind, mut props: Vec<Prop>) -> SceneNode {
        props.sort_by_key(|p| p.tag());
        props.dedup_by_key(|p| p.tag());
        SceneNode { key: key.into(), kind, props, children: Vec::new() }
    }
}

/// The protocol default of a prop (what a node shows when it doesn't set it).
pub fn default_prop(tag: u8) -> Prop {
    match tag {
        1 => Prop::X(0),
        2 => Prop::Y(0),
        3 => Prop::W(0),
        4 => Prop::H(0),
        5 => Prop::Clip(false),
        6 => Prop::Opacity(255),
        7 => Prop::Scale(1),
        8 => Prop::HitId(0),
        9 => Prop::Fill(0),
        10 => Prop::Radius(0),
        11 => Prop::BorderW(0),
        12 => Prop::BorderColor(0),
        13 => Prop::Text(String::new()),
        14 => Prop::Font(ecm_ui_proto::Font::Ui),
        15 => Prop::Size(12),
        16 => Prop::Color(0x0000_00ff),
        17 => Prop::Resource(0),
        18 => Prop::Src(Rect::default()),
        19 => Prop::Sampling(ecm_ui_proto::Sampling::Nearest),
        20 => Prop::Tint(0xffff_ffff),
        21 => Prop::Points(Vec::new()),
        22 => Prop::Width(1),
        _ => Prop::X(0),
    }
}

struct Committed {
    id: u32,
    kind: NodeKind,
    parent: u32,
    props: Vec<Prop>,
}

/// The committed tree of one surface and its node ids.
#[derive(Default)]
pub struct TreeState {
    nodes: HashMap<String, Committed>,
    children: HashMap<u32, Vec<u32>>,
    keys: HashMap<u32, String>,
    next_id: u32,
}

impl TreeState {
    pub fn new() -> TreeState {
        TreeState { next_id: 1, ..Default::default() }
    }

    /// The id of the node with `key` in the committed tree.
    pub fn id_of(&self, key: &str) -> Option<u32> {
        self.nodes.get(key).map(|n| n.id)
    }

    /// Number of nodes in the committed tree.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Ops that turn the committed tree into `scene` (the root's children),
    /// and remember `scene` as committed.
    pub fn update(&mut self, scene: &[SceneNode]) -> Vec<TreeOp> {
        let mut ops = Vec::new();

        // Which keys survive (same key and kind)?
        let mut new_keys: HashMap<&str, NodeKind> = HashMap::new();
        fn collect<'a>(n: &'a SceneNode, out: &mut HashMap<&'a str, NodeKind>) {
            let dup = out.insert(n.key.as_str(), n.kind);
            debug_assert!(dup.is_none(), "duplicate scene key {}", n.key);
            for c in &n.children {
                collect(c, out);
            }
        }
        for n in scene {
            collect(n, &mut new_keys);
        }
        let removed: HashSet<u32> = self
            .nodes
            .iter()
            .filter(|(k, n)| new_keys.get(k.as_str()) != Some(&n.kind))
            .map(|(_, n)| n.id)
            .collect();
        // Topmost removed nodes, in a deterministic order (by id).
        let mut top: Vec<u32> = removed
            .iter()
            .copied()
            .filter(|id| {
                let p = self.nodes[&self.keys[id]].parent;
                p == ROOT_NODE || !removed.contains(&p)
            })
            .collect();
        top.sort_unstable();
        let mut deferred = Vec::new();
        for id in top {
            if self.has_survivor(id, &removed) {
                deferred.push(id);
            } else {
                ops.push(TreeOp::Remove { id });
                self.forget(id);
            }
        }
        // Kind changes leave the old node under the same key until removed:
        // move those entries aside so the new node gets a fresh id.
        for &id in &deferred {
            let key = self.keys[&id].clone();
            if let Some(n) = self.nodes.remove(&key) {
                let aside = format!("\u{0}removed:{id}");
                self.nodes.insert(aside.clone(), n);
                self.keys.insert(id, aside);
            }
        }

        fn parents<'a>(n: &'a SceneNode, parent: &'a str, out: &mut HashMap<String, String>) {
            out.insert(n.key.clone(), parent.to_string());
            for c in &n.children {
                parents(c, &n.key, out);
            }
        }
        let mut plan = Plan { new_parent: HashMap::new(), deferred: deferred.iter().copied().collect() };
        for n in scene {
            parents(n, "", &mut plan.new_parent);
        }

        for (i, n) in scene.iter().enumerate() {
            self.visit(n, ROOT_NODE, "", i, &plan, &mut ops);
        }

        for id in deferred {
            ops.push(TreeOp::Remove { id });
            self.forget(id);
        }
        ops
    }

    fn has_survivor(&self, id: u32, removed: &HashSet<u32>) -> bool {
        self.children.get(&id).is_some_and(|cs| cs.iter().any(|c| !removed.contains(c) || self.has_survivor(*c, removed)))
    }

    /// Drop `id` and its whole committed subtree.
    fn forget(&mut self, id: u32) {
        if let Some(cs) = self.children.remove(&id) {
            for c in cs {
                if self.keys.contains_key(&c) && self.nodes.get(&self.keys[&c]).is_some_and(|n| n.parent == id) {
                    self.forget(c);
                }
            }
        }
        if let Some(key) = self.keys.remove(&id) {
            if let Some(n) = self.nodes.remove(&key) {
                if let Some(siblings) = self.children.get_mut(&n.parent) {
                    siblings.retain(|&s| s != id);
                }
            }
        }
    }

    /// A committed child of `parent_key` that is going away from it (removed
    /// at the end, or moving to another parent later in this update).
    fn stale(&self, id: u32, parent_key: &str, plan: &Plan) -> bool {
        plan.deferred.contains(&id) || plan.new_parent.get(&self.keys[&id]).map(String::as_str) != Some(parent_key)
    }

    /// The position in `parent`'s committed child list that puts a node at
    /// `index` among the children that stay there.
    fn real_index(&self, parent: u32, parent_key: &str, index: usize, exclude: u32, plan: &Plan) -> usize {
        let list: Vec<u32> =
            self.children.get(&parent).map(|v| v.iter().copied().filter(|&c| c != exclude).collect()).unwrap_or_default();
        let mut count = 0;
        for (pos, &c) in list.iter().enumerate() {
            if !self.stale(c, parent_key, plan) {
                if count == index {
                    return pos;
                }
                count += 1;
            }
        }
        list.len()
    }

    fn visit(&mut self, n: &SceneNode, parent: u32, parent_key: &str, index: usize, plan: &Plan, ops: &mut Vec<TreeOp>) {
        let id = match self.nodes.get(&n.key) {
            Some(c) if c.kind == n.kind => {
                let id = c.id;
                let cur_parent = c.parent;
                let in_place = cur_parent == parent && {
                    let mut staying = self.children[&parent].iter().copied().filter(|&x| !self.stale(x, parent_key, plan));
                    staying.nth(index) == Some(id)
                };
                if !in_place {
                    let real = self.real_index(parent, parent_key, index, id, plan);
                    ops.push(TreeOp::Move { id, parent, index: real as u32 });
                    if let Some(v) = self.children.get_mut(&cur_parent) {
                        v.retain(|&x| x != id);
                    }
                    let v = self.children.entry(parent).or_default();
                    v.insert(real.min(v.len()), id);
                    self.nodes.get_mut(&n.key).unwrap().parent = parent;
                }
                let changed = changed_props(&self.nodes[&n.key].props, &n.props);
                if !changed.is_empty() {
                    ops.push(TreeOp::Set { id, props: changed });
                    self.nodes.get_mut(&n.key).unwrap().props = n.props.clone();
                }
                id
            }
            _ => {
                let id = self.next_id;
                self.next_id += 1;
                let real = self.real_index(parent, parent_key, index, 0, plan);
                ops.push(TreeOp::Create { id, parent, index: real as u32, kind: n.kind, props: n.props.clone() });
                let v = self.children.entry(parent).or_default();
                v.insert(real.min(v.len()), id);
                self.nodes.insert(n.key.clone(), Committed { id, kind: n.kind, parent, props: n.props.clone() });
                self.keys.insert(id, n.key.clone());
                id
            }
        };
        for (i, c) in n.children.iter().enumerate() {
            self.visit(c, id, &n.key, i, plan, ops);
        }
    }
}

struct Plan {
    /// New parent key of every key in the new scene ("" = the root).
    new_parent: HashMap<String, String>,
    /// Removed nodes that still hold survivors; removed last.
    deferred: HashSet<u32>,
}

/// Props of `new` that differ from `old`, plus defaults for props `new`
/// dropped; sorted by tag.
fn changed_props(old: &[Prop], new: &[Prop]) -> Vec<Prop> {
    let mut out: Vec<Prop> = new.iter().filter(|p| !old.contains(p)).cloned().collect();
    for p in old {
        if !new.iter().any(|q| q.tag() == p.tag()) {
            let d = default_prop(p.tag());
            if d != *p {
                out.push(d);
            }
        }
    }
    out.sort_by_key(|p| p.tag());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(key: &str, s: &str, y: i32) -> SceneNode {
        SceneNode::new(key, NodeKind::Text, vec![Prop::Text(s.into()), Prop::Y(y)])
    }

    fn group(key: &str, children: Vec<SceneNode>) -> SceneNode {
        let mut g = SceneNode::new(key, NodeKind::Group, vec![Prop::X(1)]);
        g.children = children;
        g
    }

    #[test]
    fn first_update_creates_in_preorder() {
        let mut t = TreeState::new();
        let ops = t.update(&[group("g", vec![text("a", "A", 0), text("b", "B", 12)])]);
        assert_eq!(ops.len(), 3);
        assert!(matches!(ops[0], TreeOp::Create { id: 1, parent: 0, index: 0, kind: NodeKind::Group, .. }));
        assert!(matches!(ops[1], TreeOp::Create { id: 2, parent: 1, index: 0, .. }));
        assert!(matches!(ops[2], TreeOp::Create { id: 3, parent: 1, index: 1, .. }));
        // Same scene again: nothing to do.
        assert!(t.update(&[group("g", vec![text("a", "A", 0), text("b", "B", 12)])]).is_empty());
    }

    #[test]
    fn one_changed_prop_is_one_set() {
        let mut t = TreeState::new();
        t.update(&[group("g", vec![text("a", "A", 0), text("b", "B", 12)])]);
        let ops = t.update(&[group("g", vec![text("a", "A", 0), text("b", "Bee", 12)])]);
        assert_eq!(ops, vec![TreeOp::Set { id: 3, props: vec![Prop::Text("Bee".into())] }]);
    }

    #[test]
    fn insert_and_remove_are_single_ops() {
        let mut t = TreeState::new();
        t.update(&[group("g", vec![text("a", "A", 0), text("c", "C", 0)])]);
        let ops = t.update(&[group("g", vec![text("a", "A", 0), text("b", "B", 0), text("c", "C", 0)])]);
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], TreeOp::Create { id: 4, parent: 1, index: 1, .. }));
        let ops = t.update(&[group("g", vec![text("b", "B", 0), text("c", "C", 0)])]);
        assert_eq!(ops, vec![TreeOp::Remove { id: 2 }]);
        // Removing a group removes its subtree with one op.
        let ops = t.update(&[]);
        assert_eq!(ops, vec![TreeOp::Remove { id: 1 }]);
        assert!(t.is_empty());
    }

    #[test]
    fn moves_reparenting_and_kind_changes() {
        let mut t = TreeState::new();
        t.update(&[group("g1", vec![text("a", "A", 0)]), group("g2", vec![])]);
        // a moves into g2 and g1 goes away: g1 is removed after the move, and
        // g2 needs no move of its own (it is first once g1 is gone).
        let ops = t.update(&[group("g2", vec![text("a", "A", 0)])]);
        assert_eq!(ops, vec![TreeOp::Move { id: 2, parent: 3, index: 0 }, TreeOp::Remove { id: 1 }]);
        // A key whose kind changes gets a new node.
        let ops = t.update(&[group("g2", vec![SceneNode::new("a", NodeKind::Rect, vec![Prop::W(5)])])]);
        assert_eq!(ops[0], TreeOp::Remove { id: 2 });
        assert!(matches!(ops[1], TreeOp::Create { id: 4, kind: NodeKind::Rect, .. }));
    }

    #[test]
    fn dropped_props_reset_to_defaults() {
        let mut t = TreeState::new();
        t.update(&[SceneNode::new("r", NodeKind::Rect, vec![Prop::W(5), Prop::BorderW(2), Prop::HitId(9)])]);
        let ops = t.update(&[SceneNode::new("r", NodeKind::Rect, vec![Prop::W(5)])]);
        assert_eq!(ops, vec![TreeOp::Set { id: 1, props: vec![Prop::HitId(0), Prop::BorderW(0)] }]);
    }
}
