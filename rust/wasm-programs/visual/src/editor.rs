//! The editor: everything the Visual Programmer's window does, as plain
//! state changes (the [`crate::view`] module draws it). Tested without a
//! compositor.

use crate::blocks::Registry;
use crate::codegen::{self, AppTarget};
use crate::designer::{self, events_of, parse_path, props_of, LayoutDoc, PropType};
use crate::files::Files;
use crate::geometry::{block_geom, zoom_index, Metrics, DEFAULT_ZOOM, ZOOMS};
use crate::graph::{Graph, PortRef};
use crate::package;
use crate::runner::Runner;
use ecm_ui::{Json, PointerInfo};
use ecm_ui_proto::{Button, PointerKind};
use std::cell::Cell;
use std::collections::HashMap;

/// Where the editor keeps programs.
pub const DIR: &str = "visual";
/// Output lines the run panel keeps.
pub const RUN_LINES: usize = 200;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Page {
    /// The node graph.
    Program,
    /// The GUI designer.
    Design,
}

/// A drag on the program canvas.
#[derive(Clone, Debug, PartialEq)]
pub enum Drag {
    None,
    Pan { start: (i32, i32), offset: (f64, f64) },
    /// `grab` is the pointer's offset inside the block (canvas units).
    Block { id: u32, grab: (f64, f64) },
    Wire { from: PortRef, at: (i32, i32) },
}

/// A drag on the designer board.
#[derive(Clone, Debug, PartialEq)]
pub enum DesignDrag {
    None,
    /// A new widget from the palette.
    New { kind: String, at: (i32, i32) },
    /// An existing widget.
    Move { path: Vec<usize>, start: (i32, i32), at: (i32, i32), moved: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Dialog {
    None,
    /// Pick a file to open.
    Open(Vec<String>),
    /// A message with a title.
    Message(String, String),
    /// A block's "more information" popup (the block type).
    Info(String),
}

/// The output of the program started by Run.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RunPanel {
    pub title: String,
    pub text: String,
    pub running: bool,
}

impl RunPanel {
    pub fn append(&mut self, s: &str) {
        self.text.push_str(s);
        let lines = self.text.matches('\n').count();
        if lines > RUN_LINES {
            let mut cut = 0;
            for _ in 0..lines - RUN_LINES {
                cut += self.text[cut..].find('\n').map_or(0, |i| i + 1);
            }
            self.text.drain(..cut);
        }
    }

    /// The last `n` lines.
    pub fn tail(&self, n: usize) -> Vec<&str> {
        let lines: Vec<&str> = self.text.split('\n').collect();
        let start = lines.len().saturating_sub(n);
        lines[start..].to_vec()
    }
}

/// The old editor's file-name rule: letters, digits, `_` and `-`, at most 64.
pub fn sanitize(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').take(64).collect()
}

pub struct Editor {
    pub reg: Registry,
    pub graph: Graph,
    /// The `.vpl` the graph came from / was saved to.
    pub path: Option<String>,
    pub zoom: usize,
    pub selected: Option<u32>,
    pub drag: Drag,
    /// A palette block waiting to be placed with a click on the canvas.
    pub placing: Option<String>,
    pub page: Page,
    pub doc: Option<LayoutDoc>,
    pub design_sel: Option<Vec<usize>>,
    pub design_drag: DesignDrag,
    pub dialog: Dialog,
    pub run: Option<RunPanel>,
    pub status: String,
    pub files: Box<dyn Files>,
    pub runner: Box<dyn Runner>,
    /// The canvas size last drawn (for placing blocks in view).
    pub canvas_size: Cell<(i32, i32)>,
    /// Bumped when the inspector's fields must reload their text.
    pub insp_rev: u64,
    pub insp_seen: Cell<u64>,
    /// Palette sections the player opened or folded (category name -> folded),
    /// kept while the editor runs; others follow `Category::collapsed`.
    pub folded: HashMap<String, bool>,
}

impl Editor {
    pub fn new(reg: Registry, files: Box<dyn Files>, runner: Box<dyn Runner>) -> Editor {
        Editor {
            reg,
            graph: Graph::new("untitled"),
            path: None,
            zoom: DEFAULT_ZOOM,
            selected: None,
            drag: Drag::None,
            placing: None,
            page: Page::Program,
            doc: None,
            design_sel: None,
            design_drag: DesignDrag::None,
            dialog: Dialog::None,
            run: None,
            status: "Pick a block on the left, then click the canvas to place it.".into(),
            files,
            runner,
            canvas_size: Cell::new((480, 320)),
            insp_rev: 1,
            insp_seen: Cell::new(0),
            folded: HashMap::new(),
        }
    }

    pub fn metrics(&self) -> Metrics {
        Metrics::new(self.zoom)
    }

    /// Canvas coordinates of a canvas-board point.
    pub fn to_canvas(&self, x: i32, y: i32) -> (f64, f64) {
        let f = self.metrics().factor();
        ((x as f64 - self.graph.offset.0) / f, (y as f64 - self.graph.offset.1) / f)
    }

    /// Board coordinates of a canvas point.
    pub fn to_screen(&self, x: f64, y: f64) -> (i32, i32) {
        let f = self.metrics().factor();
        ((x * f + self.graph.offset.0).round() as i32, (y * f + self.graph.offset.1).round() as i32)
    }

    /// The program's file name stem.
    pub fn base(&self) -> String {
        let s = sanitize(&self.graph.name);
        if s.is_empty() {
            "untitled".into()
        } else {
            s
        }
    }

    fn named(&self) -> bool {
        self.graph.name != "untitled" && !sanitize(&self.graph.name).is_empty()
    }

    // ------------------------------------------------------------ program canvas

    /// A block id from a canvas element id (`node:<id>` or `node:<id>:<part>`).
    fn node_target(target: &str) -> Option<u32> {
        target.strip_prefix("node:")?.split(':').next()?.parse().ok()
    }

    /// `tag:<id>:<port>` / `opt:<id>:<port>`.
    fn port_target<'a>(target: &'a str, prefix: &str) -> Option<(u32, &'a str)> {
        let rest = target.strip_prefix(prefix)?;
        let (id, port) = rest.split_once(':')?;
        Some((id.parse().ok()?, port))
    }

    /// Whether a palette section shows only its header.
    pub fn is_collapsed(&self, category: &str) -> bool {
        self.folded
            .get(category)
            .copied()
            .unwrap_or_else(|| self.reg.categories.iter().find(|c| c.name == category).is_some_and(|c| c.collapsed))
    }

    /// Open a folded palette section, or fold an open one.
    pub fn toggle_category(&mut self, category: &str) {
        let now = !self.is_collapsed(category);
        self.folded.insert(category.to_string(), now);
    }

    /// Show a block type's "more information" popup.
    pub fn show_info(&mut self, ty: &str) {
        if self.reg.find(ty).is_some() {
            self.dialog = Dialog::Info(ty.to_string());
        }
    }

    /// Pick a palette block: the next click on the canvas places it.
    pub fn pick(&mut self, ty: &str) {
        if let Some(def) = self.reg.find(ty) {
            self.status = format!("Click on the canvas to place \"{}\" (Esc cancels).", def.label);
            self.placing = Some(ty.to_string());
        }
    }

    /// Place a block with its header under board point (x, y) (the old
    /// editor centred it horizontally, 14 units above the pointer).
    pub fn place_at(&mut self, ty: &str, x: i32, y: i32) -> Option<u32> {
        let def = self.reg.find(ty)?.clone();
        let (cx, cy) = self.to_canvas(x, y);
        let m = Metrics::new(DEFAULT_ZOOM);
        let w = (m.s(120)).max(m.text(&def.label) + m.s(20)) as f64;
        let id = self.graph.add_block(&def, cx - w / 2.0, cy - 14.0);
        self.selected = Some(id);
        self.status = format!("Added {}.", def.label);
        Some(id)
    }

    /// A pointer event on the program canvas.
    pub fn canvas_pointer(&mut self, p: &PointerInfo) {
        let target = p.target.as_deref();
        match p.kind {
            PointerKind::Down => self.canvas_down(p, target),
            PointerKind::Move => match self.drag.clone() {
                Drag::Pan { start, offset } => {
                    self.graph.offset = (offset.0 + (p.x - start.0) as f64, offset.1 + (p.y - start.1) as f64);
                }
                Drag::Block { id, grab } => {
                    let (cx, cy) = self.to_canvas(p.x, p.y);
                    if let Some(b) = self.graph.block_mut(id) {
                        b.x = cx - grab.0;
                        b.y = cy - grab.1;
                    }
                }
                Drag::Wire { from, .. } => self.drag = Drag::Wire { from, at: (p.x, p.y) },
                Drag::None => {}
            },
            PointerKind::Up => {
                if let Drag::Wire { from, .. } = std::mem::replace(&mut self.drag, Drag::None) {
                    if let Some(to) = target.and_then(PortRef::parse) {
                        match self.graph.connect(&self.reg, &from, &to) {
                            Ok(()) => self.status = "Connected.".into(),
                            Err(e) => self.status = format!("Can't connect: {e}."),
                        }
                    }
                }
                self.drag = Drag::None;
            }
            PointerKind::Scroll => {
                let delta = if p.scroll_y < 0 { 1 } else if p.scroll_y > 0 { -1 } else { 0 };
                self.zoom_by(delta, (p.x, p.y));
            }
            _ => {}
        }
    }

    fn canvas_down(&mut self, p: &PointerInfo, target: Option<&str>) {
        if p.button == Button::Right {
            if self.placing.take().is_some() {
                self.status = "Cancelled.".into();
                return;
            }
            if let Some(port) = target.and_then(PortRef::parse) {
                self.graph.disconnect(&port);
                self.status = "Disconnected.".into();
            } else if let Some(id) = target.and_then(Self::node_target) {
                self.delete_block(id);
            }
            return;
        }
        if p.button == Button::Middle {
            self.drag = Drag::Pan { start: (p.x, p.y), offset: self.graph.offset };
            return;
        }
        // A block's info button (before placing: a click on it is for the popup).
        if let Some(id) = target.and_then(|t| t.strip_prefix("info:")).and_then(|i| i.parse::<u32>().ok()) {
            if let Some(ty) = self.graph.block(id).map(|b| b.ty.clone()) {
                self.show_info(&ty);
            }
            return;
        }
        if let Some(ty) = self.placing.take() {
            self.place_at(&ty, p.x, p.y);
            return;
        }
        if let Some((id, port)) = target.and_then(|t| Self::port_target(t, "tag:")) {
            self.cycle_tag(id, port);
            return;
        }
        if let Some((id, port)) = target.and_then(|t| Self::port_target(t, "opt:")) {
            self.cycle_option(id, port);
            return;
        }
        if let Some(port) = target.and_then(PortRef::parse) {
            self.drag = Drag::Wire { from: port, at: (p.x, p.y) };
            return;
        }
        if let Some(id) = target.and_then(Self::node_target) {
            if let Some(b) = self.graph.block(id) {
                let (cx, cy) = self.to_canvas(p.x, p.y);
                self.drag = Drag::Block { id, grab: (cx - b.x, cy - b.y) };
                self.selected = Some(id);
                self.graph.bring_to_front(id);
            }
            return;
        }
        self.selected = None;
        self.drag = Drag::Pan { start: (p.x, p.y), offset: self.graph.offset };
    }

    /// A key on the focused canvas.
    pub fn canvas_key(&mut self, key: u16) {
        use ecm_ui::window::keys;
        match key {
            keys::DELETE | keys::BACKSPACE => {
                if let Some(id) = self.selected {
                    self.delete_block(id);
                }
            }
            keys::ESCAPE => {
                self.placing = None;
                self.drag = Drag::None;
                self.status = "Cancelled.".into();
            }
            _ => {}
        }
    }

    pub fn delete_block(&mut self, id: u32) {
        if self.graph.remove_block(id) {
            if self.selected == Some(id) {
                self.selected = None;
            }
            self.status = "Deleted the block.".into();
        }
    }

    /// Zoom one level in (+1) or out (-1), keeping board point `around` fixed.
    pub fn zoom_by(&mut self, delta: i32, around: (i32, i32)) {
        let new = (self.zoom as i32 + delta).clamp(0, ZOOMS.len() as i32 - 1) as usize;
        if new == self.zoom {
            return;
        }
        let (cx, cy) = self.to_canvas(around.0, around.1);
        self.zoom = new;
        let f = self.metrics().factor();
        self.graph.offset = (around.0 as f64 - cx * f, around.1 as f64 - cy * f);
        self.graph.zoom = f;
    }

    pub fn set_input(&mut self, block: u32, port: &str, value: &str) {
        if let Some(b) = self.graph.block_mut(block) {
            b.set(port, value);
        }
    }

    pub fn cycle_option(&mut self, block: u32, port: &str) {
        let Some(b) = self.graph.block(block) else { return };
        let Some(def) = self.reg.find(&b.ty).and_then(|d| d.input(port)) else { return };
        let cur = b.get(port).map(String::from).or_else(|| def.default.clone()).unwrap_or_default();
        let next = def.next_option_value(&cur);
        self.set_input(block, port, &next);
    }

    /// `any` inputs: text → int → decimal → text.
    pub fn cycle_tag(&mut self, block: u32, port: &str) {
        let key = format!("{port}_type");
        let cur = self.graph.block(block).and_then(|b| b.get(&key)).unwrap_or("string").to_string();
        let next = match cur.as_str() {
            "string" => "int",
            "int" => "float",
            _ => "string",
        };
        self.set_input(block, &key, next);
    }

    /// Scroll the canvas so a block is in the middle.
    pub fn center_on(&mut self, id: u32) {
        let Some(b) = self.graph.block(id) else { return };
        let (bx, by) = (b.x, b.y);
        let f = self.metrics().factor();
        let (w, h) = self.canvas_size.get();
        self.graph.offset = ((w / 2) as f64 - (bx + 60.0) * f, (h / 2) as f64 - (by + 20.0) * f);
    }

    // ------------------------------------------------------------ files

    pub fn new_program(&mut self) {
        if self.runner.running() {
            self.runner.stop();
        }
        self.graph = Graph::new("untitled");
        self.path = None;
        self.doc = None;
        self.selected = None;
        self.design_sel = None;
        self.zoom = DEFAULT_ZOOM;
        self.page = Page::Program;
        self.insp_rev += 1;
        self.status = "New program.".into();
    }

    /// The files the Open dialog lists.
    pub fn openable(&self) -> Vec<String> {
        self.files
            .list(DIR)
            .into_iter()
            .filter(|f| f.ends_with(".vpl") || f.ends_with(".ui.json"))
            .map(|f| format!("{DIR}/{f}"))
            .collect()
    }

    pub fn show_open(&mut self) {
        let list = self.openable();
        if list.is_empty() {
            self.status = format!("No programs in /{DIR} yet.");
        }
        self.dialog = Dialog::Open(list);
    }

    /// Open a `.vpl` program or a `.ui.json` layout.
    pub fn open(&mut self, path: &str) -> Result<(), String> {
        self.dialog = Dialog::None;
        let path = crate::files::norm(path);
        let text = self.files.read_string(&path).ok_or_else(|| format!("can't read {path}"))?;
        if path.ends_with(".ui.json") {
            let doc = LayoutDoc::parse(&text).map_err(|e| format!("{path}: {e}"))?;
            let stem = path.rsplit('/').next().unwrap_or("layout").trim_end_matches(".ui.json").to_string();
            self.new_program();
            self.graph.name = stem;
            self.graph.layout = Some(path.clone());
            self.doc = Some(doc);
            self.page = Page::Design;
            self.status = format!("Opened {path}.");
            return Ok(());
        }
        let (g, skipped) = Graph::load(&text, &self.reg, 1).map_err(|e| format!("{path}: {e}"))?;
        self.new_program();
        self.zoom = zoom_index(g.zoom);
        self.graph = g;
        self.path = Some(path.clone());
        if let Some(l) = self.graph.layout.clone() {
            match self.files.read_string(&l).map(|t| LayoutDoc::parse(&t)) {
                Some(Ok(d)) => self.doc = Some(d),
                Some(Err(e)) => self.status = format!("{l}: {e}"),
                None => {}
            }
        }
        self.status = if skipped.is_empty() {
            format!("Opened {path}.")
        } else {
            format!("Opened {path}; skipped unknown blocks: {}.", skipped.join(", "))
        };
        Ok(())
    }

    /// Save the program (and its layout) under `/visual/<name>`.
    pub fn save(&mut self) -> Result<String, String> {
        if !self.named() {
            self.status = "Type a name for the program in the box at the top, then Save.".into();
            return Err(self.status.clone());
        }
        let base = self.base();
        if let Some(doc) = &mut self.doc {
            designer::sync_handlers(doc, &self.graph);
            let lpath = format!("{DIR}/{base}.ui.json");
            self.files.write(&lpath, doc.to_text().as_bytes())?;
            self.graph.layout = Some(lpath);
        }
        self.graph.zoom = self.metrics().factor();
        let path = format!("{DIR}/{base}.vpl");
        let text = self.graph.save(&self.reg);
        self.files.write(&path, text.as_bytes())?;
        self.path = Some(path.clone());
        self.status = format!("Saved {path}.");
        Ok(path)
    }

    // ------------------------------------------------------------ run

    /// The Python for the program as Run would execute it.
    pub fn python(&self) -> String {
        let target = AppTarget { layout: format!("/{DIR}/{}.ui.json", self.base()), app_id: self.app_id() };
        codegen::generate(&self.graph, &self.reg, Some(&target))
    }

    pub fn app_id(&self) -> String {
        self.graph.app.as_ref().map(|a| a.id.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| package::default_app_id(&self.graph.name))
    }

    /// Run: write `/visual/<name>.py` and start `python` on it.
    pub fn run(&mut self) {
        if self.runner.running() {
            self.status = "A program is already running (Stop it first).".into();
            return;
        }
        let base = self.base();
        if codegen::is_app(&self.graph) {
            let doc = self.doc.get_or_insert_with(|| LayoutDoc::new(&self.graph.name));
            designer::sync_handlers(doc, &self.graph);
            let text = doc.to_text();
            if let Err(e) = self.files.write(&format!("{DIR}/{base}.ui.json"), text.as_bytes()) {
                self.status = e;
                return;
            }
        }
        let code = self.python();
        if code.starts_with("ERROR:") {
            self.status = code;
            return;
        }
        let script = format!("{DIR}/{base}.py");
        if let Err(e) = self.files.write(&script, code.as_bytes()) {
            self.status = e;
            return;
        }
        self.start_command(&format!("python /{script}"));
    }

    /// Start a command in the run panel.
    pub fn start_command(&mut self, cmd: &str) {
        match self.runner.start(cmd) {
            Ok(()) => {
                self.run = Some(RunPanel { title: cmd.to_string(), text: String::new(), running: true });
                self.status = format!("Running {cmd}");
            }
            Err(e) => self.status = e,
        }
    }

    pub fn stop(&mut self) {
        self.runner.stop();
        if let Some(r) = &mut self.run {
            r.running = false;
            r.append("\n[stopped]\n");
        }
        self.status = "Stopped.".into();
    }

    /// Collect the running program's output.
    pub fn poll_run(&mut self) {
        if !self.runner.running() {
            if let Some(r) = &mut self.run {
                if r.running {
                    r.running = false;
                }
            }
            return;
        }
        let p = self.runner.poll();
        if let Some(r) = &mut self.run {
            if !p.output.is_empty() {
                r.append(&p.output);
            }
            if p.finished {
                r.running = false;
                if !r.text.ends_with('\n') && !r.text.is_empty() {
                    r.append("\n");
                }
                r.append("[finished]\n");
                self.status = "The program finished.".into();
            }
        }
    }

    /// A line typed into the run panel goes to the program's input.
    pub fn send_input(&mut self, line: &str) {
        self.runner.write(&format!("{line}\n"));
        if let Some(r) = &mut self.run {
            r.append(&format!("{line}\n"));
        }
    }

    // ------------------------------------------------------------ package

    /// Whether this computer has the `pkg` command.
    pub fn has_pkg(&self) -> bool {
        ["server-bin/pkg.wasm", "bin/pkg.wasm", "server-bin/pkg", "bin/pkg"].iter().any(|p| self.files.exists(p))
    }

    /// Package: save, write the app folder, and build it with `pkg` if there is one.
    pub fn package(&mut self) -> Result<String, String> {
        let folder = self.package_folder()?;
        let perms = if folder.permissions.is_empty() { "none".to_string() } else { folder.permissions.join(", ") };
        let mut body = format!("Wrote /{}/ with app.toml, main.py, main.ui.json and icon.png.\nPermissions: {perms}.", folder.dir);
        if self.has_pkg() {
            body.push_str("\nBuilding it with `pkg build`; publish it with `pkg serve` on this computer.");
            self.start_command(&format!("pkg build /{}", folder.dir));
        } else {
            body.push_str("\nThis computer has no `pkg` command yet: once it does, run `pkg build` on that folder and `pkg serve` to publish it.");
        }
        self.dialog = Dialog::Message("Packaged".into(), body);
        self.status = format!("Packaged /{}.", folder.dir);
        Ok(folder.dir)
    }

    /// Save and write the app folder (no `pkg` step); the folder written.
    pub fn package_folder(&mut self) -> Result<package::AppFolder, String> {
        if self.doc.is_none() && !codegen::is_app(&self.graph) {
            self.status = "Package builds GUI apps: design a screen first (Design).".into();
            return Err(self.status.clone());
        }
        self.save()?;
        let doc = self.doc.get_or_insert_with(|| LayoutDoc::new(&self.graph.name)).clone();
        let source = self.path.clone().unwrap_or_default();
        let folder = package::build(&self.graph, &doc, &self.reg, &source);
        for (name, data) in &folder.files {
            self.files.write(&format!("{}/{name}", folder.dir), data)?;
        }
        Ok(folder)
    }

    // ------------------------------------------------------------ designer

    /// The layout, created on first use.
    pub fn doc(&mut self) -> &mut LayoutDoc {
        let title = self.graph.name.clone();
        self.doc.get_or_insert_with(|| LayoutDoc::new(&title))
    }

    pub fn show_design(&mut self) {
        self.doc();
        self.page = Page::Design;
        self.insp_rev += 1;
    }

    fn select_widget(&mut self, path: Option<Vec<usize>>) {
        if self.design_sel != path {
            self.design_sel = path;
            self.insp_rev += 1;
        }
    }

    /// The widget path a designer element id names (`d:<path>`; the preview frame is the root).
    fn widget_target(target: Option<&str>) -> Option<Vec<usize>> {
        match target? {
            "dframe" => Some(Vec::new()),
            t => parse_path(t.strip_prefix("d:")?),
        }
    }

    /// A pointer event on the designer board.
    pub fn design_pointer(&mut self, p: &PointerInfo) {
        let target = p.target.as_deref();
        match p.kind {
            PointerKind::Down => {
                if let Some(kind) = target.and_then(|t| t.strip_prefix("dpal:")) {
                    self.design_drag = DesignDrag::New { kind: kind.to_string(), at: (p.x, p.y) };
                    self.status = format!("Drop the {kind} on the screen: inside a row, column or panel, or after a widget.");
                } else if let Some(path) = Self::widget_target(target) {
                    self.select_widget(Some(path.clone()));
                    self.design_drag = DesignDrag::Move { path, start: (p.x, p.y), at: (p.x, p.y), moved: false };
                } else {
                    self.select_widget(None);
                }
            }
            PointerKind::Move => match &mut self.design_drag {
                DesignDrag::New { at, .. } => *at = (p.x, p.y),
                DesignDrag::Move { start, at, moved, .. } => {
                    *at = (p.x, p.y);
                    if (p.x - start.0).abs() + (p.y - start.1).abs() > 4 {
                        *moved = true;
                    }
                }
                DesignDrag::None => {}
            },
            PointerKind::Up => {
                let drag = std::mem::replace(&mut self.design_drag, DesignDrag::None);
                let dest = Self::widget_target(target);
                match drag {
                    DesignDrag::New { kind, .. } => {
                        let Some(dest) = dest else {
                            self.status = "Dropped outside the screen.".into();
                            return;
                        };
                        let node = self.doc().new_widget(&kind);
                        let path = self.doc().drop_on(node, &dest);
                        if path.is_some() {
                            self.status = format!("Added a {kind}.");
                        }
                        self.select_widget(path);
                    }
                    DesignDrag::Move { path, moved: true, .. } => {
                        if let Some(dest) = dest {
                            if dest != path {
                                match self.doc().move_to(&path, &dest) {
                                    Some(np) => {
                                        self.status = "Moved.".into();
                                        self.select_widget(Some(np));
                                    }
                                    None => self.status = "A widget can't go inside itself.".into(),
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    pub fn design_key(&mut self, key: u16) {
        use ecm_ui::window::keys;
        if matches!(key, keys::DELETE | keys::BACKSPACE) {
            self.delete_widget();
        }
    }

    pub fn delete_widget(&mut self) {
        let Some(path) = self.design_sel.clone() else { return };
        if path.is_empty() {
            self.status = "The screen's root stays; delete what's inside it.".into();
            return;
        }
        if self.doc().remove(&path).is_some() {
            self.select_widget(None);
            self.status = "Deleted the widget.".into();
        }
    }

    pub fn shift_widget(&mut self, delta: i32) {
        let Some(path) = self.design_sel.clone() else { return };
        if let Some(np) = self.doc().shift(&path, delta) {
            self.select_widget(Some(np));
        }
    }

    pub fn select_parent(&mut self) {
        if let Some(mut p) = self.design_sel.clone() {
            if p.pop().is_some() {
                self.select_widget(Some(p));
            }
        }
    }

    /// Change a prop of the selected widget from the inspector's text
    /// (empty removes it). Invalid values leave the widget unchanged.
    pub fn set_prop(&mut self, key: &str, ty: PropType, text: &str) -> Result<(), String> {
        let path = self.design_sel.clone().ok_or("nothing selected")?;
        let text = text.trim();
        let value = if text.is_empty() {
            None
        } else {
            Some(match ty {
                PropType::Str => Json::Str(text.to_string()),
                PropType::Int => Json::Int(text.parse::<i64>().map_err(|_| format!("{key} must be a whole number"))?),
                PropType::Bool => Json::Bool(matches!(text, "true" | "yes" | "1" | "on")),
                PropType::Color => {
                    let hex = text.trim_start_matches('#');
                    if !(hex.len() == 6 || hex.len() == 8) || u32::from_str_radix(hex, 16).is_err() {
                        return Err(format!("{key} is a color like #ff8800"));
                    }
                    Json::Str(format!("#{hex}"))
                }
            })
        };
        if key == "id" {
            let new = text;
            let old = self.doc().node(&path).and_then(|n| n.id().map(String::from));
            if new.is_empty() || !new.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err("an id is letters, digits and _".into());
            }
            if old.as_deref() != Some(new) && self.doc().find_id(new).is_some() {
                return Err(format!("another widget is already called {new}"));
            }
            if let Some(old) = old {
                designer::rename_widget(&mut self.graph, &old, new);
            }
        }
        let doc = self.doc();
        let before = doc.clone();
        let node = doc.node_mut(&path).ok_or("the widget is gone")?;
        match value {
            Some(v) => node.set(key, v),
            None => node.remove(key),
        }
        if let Err(e) = ecm_ui::layout_file::load(&doc.to_text()) {
            *doc = before;
            return Err(e);
        }
        Ok(())
    }

    /// Show (or create) the "When Widget" block for the selected widget's
    /// `event` and switch to the program.
    pub fn edit_handler(&mut self, event: &str) {
        let Some(path) = self.design_sel.clone() else { return };
        let Some(node) = self.doc().node(&path).cloned() else { return };
        if !events_of(&node.kind).contains(&event) {
            return;
        }
        let id = match node.id() {
            Some(id) => id.to_string(),
            None => {
                let id = self.doc().fresh_id(&node.kind);
                if let Some(n) = self.doc().node_mut(&path) {
                    n.set("id", Json::Str(id.clone()));
                }
                id
            }
        };
        let existing = self.graph.blocks.iter().find(|b| b.ty == "on_ui_event" && b.get("widget") == Some(&id) && b.get("event") == Some(event)).map(|b| b.id);
        let block = match existing {
            Some(b) => b,
            None => {
                let (w, h) = self.canvas_size.get();
                let n = self.graph.blocks.iter().filter(|b| b.ty == "on_ui_event").count() as i32;
                let Some(b) = self.place_at("on_ui_event", w / 3 + 30 * n, h / 3 + 30 * n) else { return };
                self.set_input(b, "widget", &id);
                self.set_input(b, "event", event);
                b
            }
        };
        self.selected = Some(block);
        self.page = Page::Program;
        self.center_on(block);
        let doc = self.doc.as_mut().unwrap();
        designer::sync_handlers(doc, &self.graph);
        self.status = format!("Wire what happens when {id} is {} to the block's body.", past(event));
    }

    /// The inspector's props for the selection: (key, type, current text).
    pub fn inspector(&self) -> Vec<(&'static str, PropType, String)> {
        let Some(n) = self.design_sel.as_ref().and_then(|p| self.doc.as_ref()?.node(p)) else { return Vec::new() };
        props_of(&n.kind)
            .into_iter()
            .map(|(k, t)| {
                let v = match n.get(k) {
                    Some(Json::Str(s)) => s.clone(),
                    Some(Json::Null) | None => String::new(),
                    Some(other) => other.to_string(),
                };
                (k, t, v)
            })
            .collect()
    }

    /// Change the app's id, name or version (kept in the .vpl).
    pub fn set_app_field(&mut self, field: &str, value: &str) {
        let app = self.graph.app.get_or_insert_with(Default::default);
        match field {
            "id" => app.id = value.trim().to_string(),
            "name" => app.name = value.to_string(),
            "version" => app.version = value.trim().to_string(),
            _ => {}
        }
    }

    /// Where a block's port is on the canvas board (for wires).
    pub fn port_screen(&self, port: &PortRef) -> Option<(i32, i32)> {
        let b = self.graph.block(port.block)?;
        let def = self.reg.find(&b.ty)?;
        let m = self.metrics();
        let g = block_geom(def, b, &self.graph, &m);
        let (px, py) = g.port(&port.port, port.output)?;
        let (sx, sy) = self.to_screen(b.x, b.y);
        Some((sx + px, sy + py))
    }
}

pub fn past(event: &str) -> &'static str {
    match event {
        "click" => "clicked",
        "change" => "changed",
        "submit" => "submitted",
        "select" => "selected",
        _ => "used",
    }
}
