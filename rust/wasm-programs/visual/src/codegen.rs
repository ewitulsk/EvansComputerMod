//! Graph → Python.
//!
//! [`generate`] is a faithful port of the old Java `VisualCodeGenerator`:
//! for the same graph it produces the same bytes (checked against goldens
//! exported from the Java code, `tests/vpl/*.py`). It follows flow wires
//! from the Start block and from event blocks, nests `if` / `elif` / `else`,
//! loops and event handlers by indentation, emits data-only blocks just
//! before the block that consumes them, and substitutes `{port}`
//! placeholders in block templates.
//!
//! Graphs with widget blocks (the App category) are GUI apps: [`generate`]
//! then wraps the same chains in a `ui` program that loads the `.ui.json`
//! layout, turns each "When Widget" block into a handler function named in
//! the layout, runs the Start chain and the event loop.

use crate::blocks::{BlockDef, PortDef, Registry};
use crate::graph::{Block, Graph};
use std::collections::{HashMap, HashSet};

/// Where a GUI app finds its layout and which id it runs as.
#[derive(Clone, Debug, PartialEq)]
pub struct AppTarget {
    /// The layout path: absolute (Run from the editor) or relative to the
    /// script (a packaged app, resolved next to `main.py`).
    pub layout: String,
    pub app_id: String,
}

/// The block names whose graphs are GUI apps.
pub fn is_ui_block(name: &str) -> bool {
    name == "on_ui_event" || name.starts_with("ui_")
}

/// Does this graph build a GUI app?
pub fn is_app(g: &Graph) -> bool {
    g.blocks.iter().any(|b| is_ui_block(&b.ty))
}

/// The handler name a "When Widget" block registers in the layout
/// (`on_<event>: "<widget>_<event>"`).
pub fn handler_name(widget: &str, event: &str) -> String {
    let clean = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect::<String>();
    format!("{}_{}", clean(widget), clean(event))
}

/// A Python string literal (the old generator's quoting).
pub fn py_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The Python for a graph: the old generator's output, or a GUI app when the
/// graph has widget blocks (`app` says where its layout is; without it the
/// layout is `<name>.ui.json` next to the script).
pub fn generate(g: &Graph, reg: &Registry, app: Option<&AppTarget>) -> String {
    let gen = Gen::new(g, reg);
    if is_app(g) {
        let fallback;
        let target = match app {
            Some(t) => t,
            None => {
                fallback = AppTarget { layout: format!("{}.ui.json", g.name), app_id: format!("dev.visual.{}", g.name) };
                &fallback
            }
        };
        gen.app(target)
    } else {
        gen.classic()
    }
}

struct Gen<'a> {
    blocks: &'a [Block],
    by_id: HashMap<u32, (&'a Block, &'a BlockDef)>,
    /// (block, flow output) → next block.
    flow_next: HashMap<(u32, String), u32>,
    /// (block, data input) → (source block, source output).
    data: HashMap<(u32, String), (u32, String)>,
}

impl<'a> Gen<'a> {
    fn new(g: &'a Graph, reg: &'a Registry) -> Gen<'a> {
        let mut by_id = HashMap::new();
        for b in &g.blocks {
            if let Some(d) = reg.find(&b.ty) {
                by_id.insert(b.id, (b, d));
            }
        }
        let mut flow_next = HashMap::new();
        let mut data = HashMap::new();
        for c in &g.conns {
            let Some((_, fdef)) = by_id.get(&c.from) else { continue };
            let is_flow = fdef.outputs.iter().any(|p| p.name == c.from_port && p.is_flow());
            if is_flow {
                flow_next.insert((c.from, c.from_port.clone()), c.to);
            } else {
                data.insert((c.to, c.to_port.clone()), (c.from, c.from_port.clone()));
            }
        }
        Gen { blocks: &g.blocks, by_id, flow_next, data }
    }

    fn known(&self) -> impl Iterator<Item = (&'a Block, &'a BlockDef)> + '_ {
        self.blocks.iter().filter_map(|b| self.by_id.get(&b.id).copied())
    }

    fn start(&self) -> Option<&'a Block> {
        self.known().find(|(_, d)| d.name == "start").map(|(b, _)| b)
    }

    fn classic_events(&self) -> Vec<&'a Block> {
        self.known().filter(|(_, d)| d.name == "on_key_press" || d.name == "on_redstone").map(|(b, _)| b).collect()
    }

    fn imports(&self) -> (bool, bool) {
        let mut terminal = false;
        let mut peripheral = false;
        for (_, d) in self.known() {
            if d.code.contains("terminal.") {
                terminal = true;
            }
            if d.code.contains("peripheral.") {
                peripheral = true;
            }
            if d.name == "on_key_press" || d.name == "on_redstone" {
                terminal = true;
            }
        }
        (terminal, peripheral)
    }

    fn flow_target(&self, id: u32, port: &str) -> Option<u32> {
        self.flow_next.get(&(id, port.to_string())).copied()
    }

    /// `VisualCodeGenerator.generate`.
    fn classic(&self) -> String {
        let start = self.start();
        let events = self.classic_events();
        if start.is_none() && events.is_empty() {
            return "ERROR: No Start block or event blocks found. Add a Start block or event handler to begin.".to_string();
        }
        let (terminal, peripheral) = self.imports();
        let mut code = String::new();
        if terminal {
            code.push_str("import terminal\n");
        }
        if peripheral {
            code.push_str("import peripheral\n");
        }
        if terminal || peripheral {
            code.push('\n');
        }
        let mut emitted = HashSet::new();
        let mut visited = HashSet::new();
        for e in events {
            self.chain(Some(e.id), &mut code, 0, &mut visited, &mut emitted);
        }
        if let Some(s) = start {
            if let Some(first) = self.flow_target(s.id, "flow") {
                self.chain(Some(first), &mut code, 0, &mut visited, &mut emitted);
            }
        }
        code
    }

    /// A GUI app around the same chains.
    fn app(&self, target: &AppTarget) -> String {
        let (terminal, peripheral) = self.imports();
        let mut code = String::from("import ui\n");
        if terminal {
            code.push_str("import terminal\n");
        }
        if peripheral {
            code.push_str("import peripheral\n");
        }
        code.push('\n');
        let relative = !target.layout.starts_with('/');
        if relative {
            code.push_str(
                "def _here(name):\n    try:\n        import os\n        return os.path.join(os.path.dirname(os.path.abspath(__file__)), name)\n    except Exception:\n        return name\n\n",
            );
        }
        code.push_str("_app = None\n_layout = None\n\n");
        let globals = self.assigned_globals();
        let mut emitted = HashSet::new();
        let mut visited = HashSet::new();
        let mut handlers: Vec<(String, String)> = Vec::new();
        for (b, d) in self.known() {
            if d.name != "on_ui_event" {
                continue;
            }
            let widget = self.raw(b, "widget");
            let event = self.raw(b, "event");
            let func = format!("_on_{}_{}", handler_name(&widget, &event), b.id);
            code.push_str(&format!("def {func}(e):\n"));
            if !globals.is_empty() {
                code.push_str(&format!("    global {}\n", globals.join(", ")));
            }
            code.push_str(&format!("    _block{}_value = e.value\n", b.id));
            visited.insert(b.id);
            let body = self.flow_target(b.id, "body");
            self.chain(body, &mut code, 1, &mut visited.clone(), &mut emitted);
            code.push('\n');
            let name = handler_name(&widget, &event);
            match handlers.iter_mut().find(|(n, _)| *n == name) {
                Some(h) => h.1 = func,
                None => handlers.push((name, func)),
            }
        }
        for e in self.classic_events() {
            self.chain(Some(e.id), &mut code, 0, &mut visited, &mut emitted);
        }
        let layout = if relative { format!("_here({})", py_str(&target.layout)) } else { py_str(&target.layout) };
        let table: Vec<String> = handlers.iter().map(|(n, f)| format!("{}: {f}", py_str(n))).collect();
        code.push_str(&format!("_app, _win = ui.load({layout}, {{{}}}, app_id={})\n", table.join(", "), py_str(&target.app_id)));
        code.push_str("_layout = _app.layout(_win)\n");
        if let Some(s) = self.start() {
            if let Some(first) = self.flow_target(s.id, "flow") {
                self.chain(Some(first), &mut code, 0, &mut visited, &mut emitted);
            }
        }
        code.push_str("_app.run()\n");
        code
    }

    /// Variables "Set Variable" blocks assign by name (handlers declare them global).
    fn assigned_globals(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (b, d) in self.known() {
            if d.name != "set_variable" || self.data.contains_key(&(b.id, "name".to_string())) {
                continue;
            }
            if b.get("name_type").unwrap_or("string") == "string" {
                continue;
            }
            let name = self.raw(b, "name");
            let ident = !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit());
            if ident && !out.contains(&name) {
                out.push(name);
            }
        }
        out
    }

    /// `generateChain`.
    fn chain(&self, id: Option<u32>, code: &mut String, indent: usize, visited: &mut HashSet<u32>, emitted: &mut HashSet<u32>) {
        let Some(id) = id else { return };
        if visited.contains(&id) {
            return;
        }
        visited.insert(id);
        let Some(&(block, def)) = self.by_id.get(&id) else { return };
        let pad = "    ".repeat(indent);
        self.emit_data_deps(block, def, code, indent, emitted);
        match def.generator_or_name() {
            "if" | "else_if" => {
                let kw = if def.generator_or_name() == "if" { "if" } else { "elif" };
                let cond = self.raw(block, "condition");
                code.push_str(&format!("{pad}{kw} {cond}:\n"));
                self.chain(self.flow_target(id, "true"), code, indent + 1, &mut visited.clone(), emitted);
                self.false_branch(self.flow_target(id, "false"), code, indent, visited, emitted);
            }
            "else" => {
                code.push_str(&format!("{pad}else:\n"));
                self.chain(self.flow_target(id, "body"), code, indent + 1, &mut visited.clone(), emitted);
            }
            "loop" => {
                let times = self.raw(block, "times");
                code.push_str(&format!("{pad}for _loop{} in range(int({times})):\n", block.id));
                self.chain(self.flow_target(id, "body"), code, indent + 1, &mut visited.clone(), emitted);
                self.chain(self.flow_target(id, "done"), code, indent, visited, emitted);
            }
            "loop_forever" => {
                code.push_str(&format!("{pad}while True:\n"));
                self.chain(self.flow_target(id, "body"), code, indent + 1, &mut visited.clone(), emitted);
            }
            g @ ("on_key_press" | "on_redstone") => {
                let (func, var, field, irq) = if g == "on_key_press" {
                    (format!("_on_key_{}", block.id), format!("_block{}_key", block.id), "key", "IRQ_KEYBOARD")
                } else {
                    (format!("_on_redstone_{}", block.id), format!("_block{}_sides", block.id), "sides", "IRQ_REDSTONE")
                };
                code.push_str(&format!("{pad}def {func}(data):\n"));
                code.push_str(&format!("{pad}    {var} = data['{field}']\n"));
                self.chain(self.flow_target(id, "body"), code, indent + 1, &mut visited.clone(), emitted);
                code.push_str(&format!("{pad}terminal.on_interrupt(terminal.{irq}, {func})\n"));
                self.chain(self.flow_target(id, "flow"), code, indent, visited, emitted);
            }
            "for_loop" => {
                let var = self.raw(block, "variable");
                let range = self.raw(block, "range");
                code.push_str(&format!("{pad}for {var} in range({range}):\n"));
                self.chain(self.flow_target(id, "body"), code, indent + 1, &mut visited.clone(), emitted);
                self.chain(self.flow_target(id, "done"), code, indent, visited, emitted);
            }
            "on_ui_event" => {
                // Handlers are emitted as functions by `app`; reached by a
                // wire, the block contributes nothing.
            }
            _ => {
                if !def.code.is_empty() {
                    let line = self.substitute(block, def, &def.code);
                    code.push_str(&format!("{pad}{line}\n"));
                }
                self.chain(self.flow_target(id, "flow"), code, indent, visited, emitted);
            }
        }
    }

    /// `emitDataDependencies`.
    fn emit_data_deps(&self, block: &Block, def: &BlockDef, code: &mut String, indent: usize, emitted: &mut HashSet<u32>) {
        for input in def.data_inputs() {
            let Some((from, _)) = self.data.get(&(block.id, input.name.clone())) else { continue };
            if emitted.contains(from) {
                continue;
            }
            let Some(&(src, sdef)) = self.by_id.get(from) else { continue };
            if sdef.flow_inputs().next().is_some() || sdef.flow_outputs().next().is_some() {
                continue;
            }
            if sdef.code.is_empty() {
                self.emit_data_deps(src, sdef, code, indent, emitted);
                continue;
            }
            self.emit_data_deps(src, sdef, code, indent, emitted);
            let line = self.substitute(src, sdef, &sdef.code);
            code.push_str(&format!("{}{line}\n", "    ".repeat(indent)));
            emitted.insert(*from);
        }
    }

    /// `substituteTemplate`.
    fn substitute(&self, block: &Block, def: &BlockDef, template: &str) -> String {
        let mut t = template.to_string();
        for input in def.data_inputs() {
            let value = self.resolve(block, def, input);
            t = t.replace(&format!("{{{}}}", input.name), &value);
        }
        for output in def.data_outputs() {
            t = t.replace(&format!("{{{}}}", output.name), &format!("_block{}_{}", block.id, output.name));
        }
        t
    }

    /// `resolveInput`: the connected variable, or the literal (quoted for strings).
    fn resolve(&self, block: &Block, _def: &BlockDef, input: &PortDef) -> String {
        if let Some((from, port)) = self.data.get(&(block.id, input.name.clone())) {
            if let Some(&(src, sdef)) = self.by_id.get(from) {
                if sdef.code.is_empty() {
                    if let Some(si) = sdef.data_inputs().find(|p| p.name == *port) {
                        return self.resolve(src, sdef, si);
                    }
                }
            }
            return format!("_block{from}_{port}");
        }
        let literal = block.get(&input.name).map(String::from).or_else(|| input.default.clone()).unwrap_or_default();
        match input.ty.as_str() {
            "string" => py_str(&literal),
            "any" => {
                let mode = block.get(&format!("{}_type", input.name)).unwrap_or("string");
                if mode == "string" {
                    py_str(&literal)
                } else {
                    literal
                }
            }
            _ => literal,
        }
    }

    /// `resolveInputRaw`: a raw Python expression (control-flow inputs).
    fn raw(&self, block: &Block, name: &str) -> String {
        if let Some((from, port)) = self.data.get(&(block.id, name.to_string())) {
            if let Some(&(src, sdef)) = self.by_id.get(from) {
                if sdef.code.is_empty() {
                    if let Some(si) = sdef.data_inputs().find(|p| p.name == *port) {
                        return src.get(&si.name).map(String::from).or_else(|| si.default.clone()).unwrap_or_default();
                    }
                }
            }
            return format!("_block{from}_{port}");
        }
        let def = self.by_id.get(&block.id).map(|x| x.1);
        if let Some(input) = def.and_then(|d| d.data_inputs().find(|p| p.name == name)) {
            return block.get(name).map(String::from).or_else(|| input.default.clone()).unwrap_or_default();
        }
        block.get(name).unwrap_or("").to_string()
    }

    /// `handleFalseBranch`.
    fn false_branch(&self, target: Option<u32>, code: &mut String, indent: usize, visited: &mut HashSet<u32>, emitted: &mut HashSet<u32>) {
        let Some(t) = target else { return };
        let Some(&(_, d)) = self.by_id.get(&t) else { return };
        let g = d.generator_or_name();
        if g == "else_if" || g == "else" {
            self.chain(Some(t), code, indent, visited, emitted);
        } else {
            code.push_str(&format!("{}else:\n", "    ".repeat(indent)));
            self.chain(Some(t), code, indent + 1, &mut visited.clone(), emitted);
        }
    }
}
