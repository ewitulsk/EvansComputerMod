//! The node graph and its `.vpl` file format.
//!
//! `.vpl` is the old Java editor's JSON (`VisualProgramSerializer`), read
//! and written the same way:
//!
//! ```json
//! {"version":1,"name":"hello","canvas":{"offsetX":0.0,"offsetY":0.0,"zoom":1.0},
//!  "blocks":[{"id":1,"type":"start","x":40.0,"y":40.0,"inputs":{}}, ...],
//!  "connections":[{"fromBlock":1,"fromPort":"flow","toBlock":2,"toPort":"flow"}, ...]}
//! ```
//!
//! Loading renumbers block ids from a starting id (1 for a fresh editor)
//! and skips blocks whose type is unknown, with their connections. Inputs
//! start at their defaults; only values that differ from the default are
//! saved. Two optional keys are new: `"layout"` (the `.ui.json` the graph's
//! widget blocks drive) and `"app"` (`{"id", "name", "version"}` for
//! packaging). The old editor ignored keys it didn't know.

use crate::blocks::{BlockDef, Registry};
use ecm_ui::Json;
use std::fmt;

/// A placed block.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub id: u32,
    /// The block definition's name.
    pub ty: String,
    pub x: f64,
    pub y: f64,
    /// Input values (and `<port>_type` tags of `any` ports), in insertion order.
    pub inputs: Vec<(String, String)>,
}

impl Block {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.inputs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, key: &str, value: &str) {
        match self.inputs.iter_mut().find(|(k, _)| k == key) {
            Some(e) => e.1 = value.to_string(),
            None => self.inputs.push((key.to_string(), value.to_string())),
        }
    }

    /// A fresh block of `def` with every input at its default.
    pub fn new(id: u32, def: &BlockDef, x: f64, y: f64) -> Block {
        let inputs = def.data_inputs().filter_map(|p| p.default.as_ref().map(|d| (p.name.clone(), d.clone()))).collect();
        Block { id, ty: def.name.clone(), x, y, inputs }
    }
}

/// A wire from an output port to an input port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conn {
    pub from: u32,
    pub from_port: String,
    pub to: u32,
    pub to_port: String,
}

/// One end of a wire being drawn: a port of a block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortRef {
    pub block: u32,
    pub port: String,
    pub output: bool,
}

impl fmt::Display for PortRef {
    /// The element id the editor gives the port's dot: `port:<block>:<o|i>:<name>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "port:{}:{}:{}", self.block, if self.output { "o" } else { "i" }, self.port)
    }
}

impl PortRef {
    pub fn parse(id: &str) -> Option<PortRef> {
        let rest = id.strip_prefix("port:")?;
        let mut it = rest.splitn(3, ':');
        let block = it.next()?.parse().ok()?;
        let output = match it.next()? {
            "o" => true,
            "i" => false,
            _ => return None,
        };
        Some(PortRef { block, port: it.next()?.to_string(), output })
    }
}

/// App metadata kept with a graph that builds a GUI app.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub version: String,
}

/// Why a wire can't be made.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ConnectError {
    SameBlock,
    BothOutputs,
    BothInputs,
    Incompatible,
    Unknown,
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ConnectError::SameBlock => "a block can't connect to itself",
            ConnectError::BothOutputs => "connect an output to an input",
            ConnectError::BothInputs => "connect an output to an input",
            ConnectError::Incompatible => "those port types don't match",
            ConnectError::Unknown => "unknown port",
        })
    }
}

/// A program: blocks, wires and the view it was saved with.
#[derive(Clone, Debug, PartialEq)]
pub struct Graph {
    pub name: String,
    pub blocks: Vec<Block>,
    pub conns: Vec<Conn>,
    /// Canvas pan (screen = canvas * zoom + offset) and zoom, as saved.
    pub offset: (f64, f64),
    pub zoom: f64,
    pub next_id: u32,
    /// The `.ui.json` this graph's widget blocks drive, if any.
    pub layout: Option<String>,
    pub app: Option<AppInfo>,
}

impl Default for Graph {
    fn default() -> Self {
        Graph::new("untitled")
    }
}

fn num(j: Option<&Json>) -> Option<f64> {
    match j? {
        Json::Int(i) => Some(*i as f64),
        Json::Float(f) => Some(*f),
        Json::Str(s) => s.parse().ok(),
        _ => None,
    }
}

/// A JSON value as Gson's `getAsString` sees it.
fn as_string(j: &Json) -> String {
    match j {
        Json::Str(s) => s.clone(),
        Json::Int(i) => i.to_string(),
        Json::Float(f) => f.to_string(),
        Json::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

impl Graph {
    pub fn new(name: &str) -> Graph {
        Graph {
            name: name.to_string(),
            blocks: Vec::new(),
            conns: Vec::new(),
            offset: (0.0, 0.0),
            zoom: 1.0,
            next_id: 1,
            layout: None,
            app: None,
        }
    }

    /// Load a `.vpl`, numbering blocks from `start_id`. Returns the graph and
    /// the types of blocks that were skipped as unknown.
    pub fn load(text: &str, reg: &Registry, start_id: u32) -> Result<(Graph, Vec<String>), String> {
        let root = Json::parse(text)?;
        if root.as_object().is_none() {
            return Err("a .vpl file is a JSON object".into());
        }
        let mut g = Graph::new(root.get("name").and_then(Json::as_str).unwrap_or("untitled"));
        if let Some(c) = root.get("canvas") {
            g.offset = (num(c.get("offsetX")).unwrap_or(0.0), num(c.get("offsetY")).unwrap_or(0.0));
            g.zoom = num(c.get("zoom")).unwrap_or(1.0);
        }
        let mut skipped = Vec::new();
        let mut remap = std::collections::HashMap::new();
        let mut next = start_id;
        for b in root.get("blocks").and_then(Json::as_array).unwrap_or(&[]) {
            let old = b.get("id").and_then(Json::as_i64).ok_or("a block has no id")?;
            let ty = b.get("type").and_then(Json::as_str).ok_or("a block has no type")?;
            let Some(def) = reg.find(ty) else {
                skipped.push(ty.to_string());
                continue;
            };
            let id = next;
            next += 1;
            remap.insert(old, id);
            let mut block = Block::new(id, def, num(b.get("x")).unwrap_or(0.0), num(b.get("y")).unwrap_or(0.0));
            if let Some(inputs) = b.get("inputs").and_then(Json::as_object) {
                for (k, v) in inputs {
                    block.set(k, &as_string(v));
                }
            }
            g.blocks.push(block);
        }
        for c in root.get("connections").and_then(Json::as_array).unwrap_or(&[]) {
            let (Some(f), Some(t)) = (c.get("fromBlock").and_then(Json::as_i64), c.get("toBlock").and_then(Json::as_i64)) else {
                continue;
            };
            let (Some(&from), Some(&to)) = (remap.get(&f), remap.get(&t)) else { continue };
            let port = |k: &str| c.get(k).and_then(Json::as_str).unwrap_or("").to_string();
            g.conns.push(Conn { from, from_port: port("fromPort"), to, to_port: port("toPort") });
        }
        g.next_id = next;
        g.layout = root.get("layout").and_then(Json::as_str).map(String::from);
        if let Some(a) = root.get("app") {
            let s = |k: &str| a.get(k).and_then(Json::as_str).unwrap_or("").to_string();
            g.app = Some(AppInfo { id: s("id"), name: s("name"), version: s("version") });
        }
        Ok((g, skipped))
    }

    /// The `.vpl` text.
    pub fn save(&self, reg: &Registry) -> String {
        let f = Json::Float;
        let mut blocks = Vec::new();
        for b in &self.blocks {
            let def = reg.find(&b.ty);
            let mut inputs = Vec::new();
            for (k, v) in &b.inputs {
                let default = def.and_then(|d| d.data_inputs().find(|p| p.name == *k)).map(|p| p.default.as_deref());
                // Only values that differ from the port's default are kept.
                if default != Some(Some(v.as_str())) {
                    inputs.push((k.clone(), Json::Str(v.clone())));
                }
            }
            blocks.push(Json::Obj(vec![
                ("id".into(), Json::Int(b.id as i64)),
                ("type".into(), Json::Str(b.ty.clone())),
                ("x".into(), f(b.x)),
                ("y".into(), f(b.y)),
                ("inputs".into(), Json::Obj(inputs)),
            ]));
        }
        let conns = self
            .conns
            .iter()
            .map(|c| {
                Json::Obj(vec![
                    ("fromBlock".into(), Json::Int(c.from as i64)),
                    ("fromPort".into(), Json::Str(c.from_port.clone())),
                    ("toBlock".into(), Json::Int(c.to as i64)),
                    ("toPort".into(), Json::Str(c.to_port.clone())),
                ])
            })
            .collect();
        let mut root = vec![
            ("version".into(), Json::Int(1)),
            ("name".into(), Json::Str(self.name.clone())),
            ("canvas".into(), Json::Obj(vec![("offsetX".into(), f(self.offset.0)), ("offsetY".into(), f(self.offset.1)), ("zoom".into(), f(self.zoom))])),
            ("blocks".into(), Json::Arr(blocks)),
            ("connections".into(), Json::Arr(conns)),
        ];
        if let Some(l) = &self.layout {
            root.push(("layout".into(), Json::Str(l.clone())));
        }
        if let Some(a) = &self.app {
            root.push((
                "app".into(),
                Json::Obj(vec![
                    ("id".into(), Json::Str(a.id.clone())),
                    ("name".into(), Json::Str(a.name.clone())),
                    ("version".into(), Json::Str(a.version.clone())),
                ]),
            ));
        }
        Json::Obj(root).to_string()
    }

    pub fn block(&self, id: u32) -> Option<&Block> {
        self.blocks.iter().find(|b| b.id == id)
    }

    pub fn block_mut(&mut self, id: u32) -> Option<&mut Block> {
        self.blocks.iter_mut().find(|b| b.id == id)
    }

    /// Place a new block; returns its id.
    pub fn add_block(&mut self, def: &BlockDef, x: f64, y: f64) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.blocks.push(Block::new(id, def, x, y));
        id
    }

    /// Delete a block and its wires.
    pub fn remove_block(&mut self, id: u32) -> bool {
        let n = self.blocks.len();
        self.blocks.retain(|b| b.id != id);
        self.conns.retain(|c| c.from != id && c.to != id);
        self.blocks.len() != n
    }

    /// Draw it last (and keep that order, like the old editor).
    pub fn bring_to_front(&mut self, id: u32) {
        if let Some(i) = self.blocks.iter().position(|b| b.id == id) {
            let b = self.blocks.remove(i);
            self.blocks.push(b);
        }
    }

    pub fn is_input_connected(&self, block: u32, port: &str) -> bool {
        self.conns.iter().any(|c| c.to == block && c.to_port == port)
    }

    /// Remove the wires at a port.
    pub fn disconnect(&mut self, p: &PortRef) {
        if p.output {
            self.conns.retain(|c| !(c.from == p.block && c.from_port == p.port));
        } else {
            self.conns.retain(|c| !(c.to == p.block && c.to_port == p.port));
        }
    }

    /// Wire two ports (dragged from `a`, released on `b`) with the old
    /// editor's rules: output to input only, matching types (`any` matches
    /// everything), an input takes one wire, a flow output starts one.
    pub fn connect(&mut self, reg: &Registry, a: &PortRef, b: &PortRef) -> Result<(), ConnectError> {
        if a.block == b.block {
            return Err(ConnectError::SameBlock);
        }
        let (from, to) = match (a.output, b.output) {
            (true, false) => (a, b),
            (false, true) => (b, a),
            (true, true) => return Err(ConnectError::BothOutputs),
            (false, false) => return Err(ConnectError::BothInputs),
        };
        let fdef = self.block(from.block).and_then(|x| reg.find(&x.ty)).and_then(|d| d.output(&from.port)).ok_or(ConnectError::Unknown)?;
        let tdef = self.block(to.block).and_then(|x| reg.find(&x.ty)).and_then(|d| d.input(&to.port)).ok_or(ConnectError::Unknown)?;
        if !(fdef.ty == tdef.ty || fdef.ty == "any" || tdef.ty == "any") {
            return Err(ConnectError::Incompatible);
        }
        let flow = fdef.is_flow();
        self.conns.retain(|c| !(c.to == to.block && c.to_port == to.port));
        if flow {
            self.conns.retain(|c| !(c.from == from.block && c.from_port == from.port));
        }
        self.conns.push(Conn { from: from.block, from_port: from.port.clone(), to: to.block, to_port: to.port.clone() });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_ids_round_trip() {
        let p = PortRef { block: 12, port: "a:b".into(), output: true };
        assert_eq!(p.to_string(), "port:12:o:a:b");
        assert_eq!(PortRef::parse(&p.to_string()), Some(p));
        assert_eq!(PortRef::parse("node:1"), None);
    }

    #[test]
    fn loading_renumbers_skips_unknown_blocks_and_fills_defaults() {
        let reg = Registry::builtin();
        let (g, skipped) = Graph::load(include_str!("../tests/vpl/unknown_and_ids.vpl"), &reg, 1).unwrap();
        assert_eq!(skipped, ["teleport"]);
        let ids: Vec<(u32, &str)> = g.blocks.iter().map(|b| (b.id, b.ty.as_str())).collect();
        assert_eq!(ids[..3], [(1, "print"), (2, "start"), (3, "input")]);
        assert_eq!(g.next_id, 8);
        assert!(g.conns.iter().all(|c| c.from != 0 && c.to != 0));
        assert_eq!(g.conns.len(), 7, "the three wires of the unknown block are dropped");
        assert_eq!((g.offset, g.zoom), ((-40.25, 8.0), 2.5));
        // set_variable's name keeps its default; value was saved.
        let sv = g.blocks.iter().find(|b| b.ty == "set_variable").unwrap();
        assert_eq!(sv.get("name"), Some("total"));
        assert_eq!(sv.get("value_type"), Some("int"));
    }

    #[test]
    fn save_keeps_only_changed_inputs_and_reloads_the_same() {
        let reg = Registry::builtin();
        let mut g = Graph::new("demo");
        let s = g.add_block(reg.find("start").unwrap(), 10.0, 20.5);
        let p = g.add_block(reg.find("print").unwrap(), 10.0, 90.0);
        g.block_mut(p).unwrap().set("text_type", "int");
        g.connect(&reg, &PortRef { block: s, port: "flow".into(), output: true }, &PortRef { block: p, port: "flow".into(), output: false })
            .unwrap();
        g.layout = Some("/visual/demo.ui.json".into());
        g.app = Some(AppInfo { id: "dev.me.demo".into(), name: "Demo".into(), version: "1.0.0".into() });
        let text = g.save(&reg);
        assert!(text.contains(r#"{"id":2,"type":"print","x":10.0,"y":90.0,"inputs":{"text_type":"int"}}"#), "{text}");
        assert!(text.starts_with(r#"{"version":1,"name":"demo","canvas":{"offsetX":0.0,"offsetY":0.0,"zoom":1.0}"#));
        let (back, skipped) = Graph::load(&text, &reg, 1).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(back, g);
    }

    #[test]
    fn wiring_follows_the_old_rules() {
        let reg = Registry::builtin();
        let mut g = Graph::new("w");
        let s = g.add_block(reg.find("start").unwrap(), 0.0, 0.0);
        let a = g.add_block(reg.find("print").unwrap(), 0.0, 0.0);
        let b = g.add_block(reg.find("print").unwrap(), 0.0, 0.0);
        let n = g.add_block(reg.find("number_constant").unwrap(), 0.0, 0.0);
        let st = g.add_block(reg.find("string_constant").unwrap(), 0.0, 0.0);
        let add = g.add_block(reg.find("add").unwrap(), 0.0, 0.0);
        let o = |block, port: &str| PortRef { block, port: port.into(), output: true };
        let i = |block, port: &str| PortRef { block, port: port.into(), output: false };
        // A flow output starts one wire: the second replaces the first.
        g.connect(&reg, &o(s, "flow"), &i(a, "flow")).unwrap();
        g.connect(&reg, &i(b, "flow"), &o(s, "flow")).unwrap();
        assert_eq!(g.conns, vec![Conn { from: s, from_port: "flow".into(), to: b, to_port: "flow".into() }]);
        assert_eq!(g.connect(&reg, &o(s, "flow"), &o(a, "flow")), Err(ConnectError::BothOutputs));
        assert_eq!(g.connect(&reg, &i(a, "flow"), &i(b, "flow")), Err(ConnectError::BothInputs));
        assert_eq!(g.connect(&reg, &o(a, "flow"), &i(a, "flow")), Err(ConnectError::SameBlock));
        assert_eq!(g.connect(&reg, &o(st, "value"), &i(add, "a")), Err(ConnectError::Incompatible));
        // `any` takes everything; an input keeps one wire; data outputs fan out.
        g.connect(&reg, &o(n, "value"), &i(a, "text")).unwrap();
        g.connect(&reg, &o(st, "value"), &i(a, "text")).unwrap();
        g.connect(&reg, &o(n, "value"), &i(add, "a")).unwrap();
        g.connect(&reg, &o(n, "value"), &i(b, "text")).unwrap();
        assert_eq!(g.conns.iter().filter(|c| c.to == a && c.to_port == "text").count(), 1);
        assert_eq!(g.conns.iter().filter(|c| c.from == n).count(), 2);
        g.disconnect(&o(n, "value"));
        assert_eq!(g.conns.iter().filter(|c| c.from == n).count(), 0);
        assert!(g.remove_block(st));
        assert!(g.conns.iter().all(|c| c.from != st && c.to != st));
    }
}
