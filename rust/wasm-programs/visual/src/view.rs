//! Draws the editor as ecm-ui elements: toolbar, block palette, the node
//! canvas (a board with panels, labels, fields and wire lines), the GUI
//! designer (a capturing board with the widget palette and a live preview
//! of the layout) with its inspector, the run panel and dialogs.

use crate::blocks::PortDef;
use crate::designer::{events_of, PropType, PALETTE};
use crate::editor::{DesignDrag, Dialog, Drag, Editor, Page};
use crate::geometry::{block_geom, shown_value, tag_text, wire_points, Field};
use crate::graph::PortRef;
use ecm_ui::prelude::*;
use ecm_ui::Theme;
use std::cell::RefCell;
use std::rc::Rc;

pub type Shared = Rc<RefCell<Editor>>;

/// Canvas colors (the old editor's palette).
pub mod colors {
    pub const CANVAS: u32 = 0x1e1e2eff;
    pub const BLOCK: u32 = 0x2a2a3eff;
    pub const BORDER: u32 = 0x000000ff;
    pub const SELECTED: u32 = 0xffd54fff;
    pub const TEXT: u32 = 0xffffffff;
    pub const PORT_LABEL: u32 = 0xaaaaaaff;
    pub const FIELD_BG: u32 = 0x1a1a2eff;
    pub const FIELD_BORDER: u32 = 0x555577ff;
    pub const FLOW: u32 = 0xffffffff;
    pub const STRING: u32 = 0x4ec9b0ff;
    pub const NUMBER: u32 = 0x569cd6ff;
    pub const ANY: u32 = 0xdcdcaaff;
    pub const WIRE: u32 = 0xccccccff;
}

pub fn port_color(ty: &str) -> u32 {
    match ty {
        "flow" => colors::FLOW,
        "string" => colors::STRING,
        "number" => colors::NUMBER,
        _ => colors::ANY,
    }
}

/// A handler that changes the editor.
fn act(sh: &Shared, f: impl Fn(&mut Editor) + 'static) -> impl Fn(&mut Ctx, &UiEvent) + 'static {
    let sh = sh.clone();
    move |_, _| f(&mut sh.borrow_mut())
}

/// A handler that changes the editor with the event.
fn act_ev(sh: &Shared, f: impl Fn(&mut Editor, &UiEvent) + 'static) -> impl Fn(&mut Ctx, &UiEvent) + 'static {
    let sh = sh.clone();
    move |_, e| f(&mut sh.borrow_mut(), e)
}

fn text_of(e: &UiEvent) -> String {
    match &e.value {
        EventValue::Text(t) => t.clone(),
        _ => String::new(),
    }
}

/// The whole window.
pub fn build(ed: &Editor, sh: &Shared, cx: &ViewCtx) -> Element {
    let theme = cx.theme;
    let main = match ed.page {
        Page::Program => program_page(ed, sh, cx),
        Page::Design => design_page(ed, sh, theme),
    };
    let mut col = column().gap(0).child(toolbar(ed, sh)).child(main.grow(1));
    if let Some(run) = &ed.run {
        col = col.child(run_panel(run, sh));
    }
    col = col.child(row().padding(4).child(label(ed.status.clone()).style("caption").wrap(false).id("status")));
    let mut root = stack().child(col);
    match &ed.dialog {
        Dialog::None => {}
        Dialog::Open(files) => {
            let mut d = dialog("Open a program");
            if files.is_empty() {
                d = d.child(label("No .vpl or .ui.json files in /visual yet."));
            }
            for f in files {
                let path = f.clone();
                d = d.child(button(f.clone()).on_click(act(sh, move |ed| {
                    if let Err(e) = ed.open(&path) {
                        ed.status = e;
                    }
                })));
            }
            d = d.child(button("Cancel").on_click(act(sh, |ed| ed.dialog = Dialog::None)));
            root = root.child(d);
        }
        Dialog::Message(title, body) => {
            root = root.child(
                dialog(title.clone())
                    .child(label(body.clone()))
                    .child(button("OK").style("primary").on_click(act(sh, |ed| ed.dialog = Dialog::None))),
            );
        }
    }
    root
}

fn toolbar(ed: &Editor, sh: &Shared) -> Element {
    let tab = |name: &str, page: Page| {
        let b = button(name);
        if ed.page == page {
            b.style("primary")
        } else {
            b
        }
    };
    let running = ed.runner.running();
    let run = if running {
        button("Stop").style("danger").id("stop").on_click(act(sh, |ed| ed.stop()))
    } else {
        button("Run").style("primary").id("run").on_click(act(sh, |ed| ed.run()))
    };
    row()
        .gap(4)
        .padding(4)
        .child(tab("Program", Page::Program).id("tab-program").on_click(act(sh, |ed| ed.page = Page::Program)))
        .child(tab("Design", Page::Design).id("tab-design").on_click(act(sh, |ed| ed.show_design())))
        .child(text_field().id("name").width(110).placeholder("name").text(ed.graph.name.clone()).on_change(act_ev(sh, |ed, e| ed.graph.name = text_of(e))))
        .child(button("New").id("new").on_click(act(sh, |ed| ed.new_program())))
        .child(button("Open").id("open").on_click(act(sh, |ed| ed.show_open())))
        .child(button("Save").id("save").on_click(act(sh, |ed| {
            let _ = ed.save();
        })))
        .child(run)
        .child(button("Package").id("package").on_click(act(sh, |ed| {
            if let Err(e) = ed.package() {
                ed.status = e;
            }
        })))
        .child(spacer())
        .child(button("-").id("zoom-out").on_click(act(sh, |ed| {
            let (w, h) = ed.canvas_size.get();
            ed.zoom_by(-1, (w / 2, h / 2));
        })))
        .child(label(format!("{}%", ed.metrics().factor() * 100.0).replace(".0%", "%")).wrap(false).id("zoom"))
        .child(button("+").id("zoom-in").on_click(act(sh, |ed| {
            let (w, h) = ed.canvas_size.get();
            ed.zoom_by(1, (w / 2, h / 2));
        })))
}

fn palette(ed: &Editor, sh: &Shared) -> Element {
    let mut col = column().gap(2).padding(4);
    for cat in &ed.reg.categories {
        col = col.child(label(cat.name.clone()).style("caption").wrap(false));
        for b in &cat.blocks {
            let ty = b.name.clone();
            let mut btn = button(b.label.clone()).id(format!("pal:{}", b.name)).grow(1).on_click(act(sh, move |ed| ed.pick(&ty)));
            if ed.placing.as_deref() == Some(b.name.as_str()) {
                btn = btn.style("primary");
            }
            col = col.child(row().gap(2).child(panel().sized(4, 16).fill(cat.color).radius(1)).child(btn));
        }
    }
    board().width(150).child(scroll_view(col).id("palette"))
}

fn program_page(ed: &Editor, sh: &Shared, cx: &ViewCtx) -> Element {
    // Remember roughly how big the canvas is (for placing blocks in view).
    ed.canvas_size.set(((cx.width - 150).max(100), (cx.height - 80).max(100)));
    row().align(Align::Stretch).gap(0).child(palette(ed, sh)).child(canvas(ed, sh))
}

/// The node canvas.
pub fn canvas(ed: &Editor, sh: &Shared) -> Element {
    let m = ed.metrics();
    let fs = m.fs;
    let mut b = board()
        .id("canvas")
        .fill(colors::CANVAS)
        .on_pointer(act_ev(sh, |ed, e| {
            if let EventValue::Pointer(p) = &e.value {
                ed.canvas_pointer(p);
            }
        }))
        .on_key(act_ev(sh, |ed, e| {
            if let EventValue::Key { key, .. } = e.value {
                ed.canvas_key(key);
            }
        }));
    // Wires under the blocks.
    for c in &ed.graph.conns {
        let from = PortRef { block: c.from, port: c.from_port.clone(), output: true };
        let to = PortRef { block: c.to, port: c.to_port.clone(), output: false };
        let (Some(a), Some(z)) = (ed.port_screen(&from), ed.port_screen(&to)) else { continue };
        let ty = ed.graph.block(c.from).and_then(|x| ed.reg.find(&x.ty)).and_then(|d| d.output(&c.from_port)).map_or("any", |p| p.ty.as_str());
        let flow = ty == "flow";
        let color = if flow { colors::FLOW } else { port_color(ty) };
        b = b.child(wire(wire_points(a, z, flow, &m)).stroke(if flow { 2 } else { 1 }.max((fs / 12) as u16)).color(color));
    }
    for block in &ed.graph.blocks {
        let Some(def) = ed.reg.find(&block.ty) else { continue };
        let g = block_geom(def, block, &ed.graph, &m);
        let (sx, sy) = ed.to_screen(block.x, block.y);
        let id = block.id;
        let selected = ed.selected == Some(id);
        let color = ed.reg.color_for(&block.ty);
        let border = if selected { (2, colors::SELECTED) } else { (1, colors::BORDER) };
        b = b.child(panel().id(format!("node:{id}")).at(sx, sy).sized(g.w, g.h).fill(colors::BLOCK).radius(3).border(border.0, border.1));
        b = b.child(panel().id(format!("node:{id}:head")).at(sx, sy).sized(g.w, m.header()).fill(color).radius(3).border(border.0, border.1));
        let title_y = sy + (m.header() - fs as i32) / 2;
        b = b.child(label(def.label.clone()).at(sx + m.s(8), title_y).size(fs).wrap(false).color(colors::TEXT));
        // Data inputs: dot, name, type tag, value field.
        for (row_i, row) in g.rows.iter().enumerate() {
            let port: &PortDef = def.data_inputs().nth(row_i).unwrap();
            let cy = sy + row.y + m.row() / 2;
            let text_y = cy - fs as i32 / 2;
            b = b.child(port_dot(&PortRef { block: id, port: row.port.clone(), output: false }, sx, cy, &port.ty, &m));
            b = b.child(label(row.port.clone()).at(sx + row.label_x, text_y).size(fs).wrap(false).color(colors::PORT_LABEL));
            if let Some((tx, tw)) = row.tag {
                let fh = m.field_h();
                b = b.child(panel().id(format!("tag:{id}:{}", row.port)).at(sx + tx, cy - fh / 2).sized(tw, fh).fill(colors::FIELD_BG).radius(2));
                b = b.child(label(tag_text(block, &row.port)).at(sx + tx + m.s(2), text_y).size(fs).wrap(false).color(colors::ANY));
            }
            if let Some((kind, fx, fw)) = &row.field {
                let fh = m.field_h();
                match kind {
                    Field::Text => {
                        let (bid, pname) = (id, row.port.clone());
                        b = b.child(
                            text_field()
                                .id(format!("in:{id}:{}", row.port))
                                .at(sx + fx, cy - fh / 2)
                                .width(*fw)
                                .size(fs)
                                .text(shown_value(block, port))
                                .on_change(act_ev(sh, move |ed, e| ed.set_input(bid, &pname, &text_of(e)))),
                        );
                    }
                    Field::Options => {
                        b = b.child(
                            panel()
                                .id(format!("opt:{id}:{}", row.port))
                                .at(sx + fx, cy - fh / 2)
                                .sized(*fw, fh)
                                .fill(colors::FIELD_BG)
                                .radius(2)
                                .border(1, colors::FIELD_BORDER),
                        );
                        b = b.child(label(shown_value(block, port)).at(sx + fx + 8, text_y).size(fs).wrap(false).color(colors::TEXT));
                    }
                }
            }
        }
        // Data outputs: name, dot.
        for (name, px, py) in &g.data_outs {
            let ty = def.output(name).map_or("any", |p| p.ty.as_str());
            let tw = m.text(name);
            b = b.child(label(name.clone()).at(sx + px - m.port_r() - m.s(3) - tw, sy + py - fs as i32 / 2).size(fs).wrap(false).color(colors::PORT_LABEL));
            b = b.child(port_dot(&PortRef { block: id, port: name.clone(), output: true }, sx + px, sy + py, ty, &m));
        }
        // Flow ports: in at the top, outs along the bottom (labelled when several).
        if let Some((name, px, py)) = &g.flow_in {
            b = b.child(port_dot(&PortRef { block: id, port: name.clone(), output: false }, sx + px, sy + py, "flow", &m));
        }
        let several = g.flow_outs.len() > 1;
        for (name, px, py) in &g.flow_outs {
            if several {
                let tw = m.text(name);
                b = b.child(label(name.clone()).at(sx + px - tw / 2, sy + py - fs as i32 - m.s(4)).size(fs).wrap(false).color(colors::PORT_LABEL));
            }
            b = b.child(port_dot(&PortRef { block: id, port: name.clone(), output: true }, sx + px, sy + py, "flow", &m));
        }
    }
    // The wire being drawn.
    if let Drag::Wire { from, at } = &ed.drag {
        if let Some(a) = ed.port_screen(from) {
            let flow = ed.graph.block(from.block).and_then(|x| ed.reg.find(&x.ty)).and_then(|d| if from.output { d.output(&from.port) } else { d.input(&from.port) }).is_some_and(PortDef::is_flow);
            let (p, q) = if from.output { (a, *at) } else { (*at, a) };
            b = b.child(wire(wire_points(p, q, flow, &m)).stroke(2).color(colors::SELECTED).id("drag-wire"));
        }
    }
    if ed.graph.blocks.is_empty() {
        b = b.child(label("Pick a block on the left, then click here to place it. Drag from a port to another to wire them.").at(16, 16).color(colors::PORT_LABEL));
    }
    b
}

fn port_dot(p: &PortRef, cx: i32, cy: i32, ty: &str, m: &crate::geometry::Metrics) -> Element {
    let r = m.port_r();
    panel().id(p.to_string()).at(cx - r, cy - r).sized(2 * r, 2 * r).fill(port_color(ty)).radius(r as u16).border(1, colors::BORDER)
}

fn design_page(ed: &Editor, sh: &Shared, theme: &Theme) -> Element {
    let mut pal = column().gap(4).padding(6).width(110).child(label("Widgets").style("caption"));
    for kind in PALETTE {
        pal = pal.child(panel().id(format!("dpal:{kind}")).padding(4).fill(theme.button).child(label(kind_label(kind)).wrap(false)));
    }
    let preview = match &ed.doc {
        Some(doc) => match doc.preview(ed.design_sel.as_deref(), theme.focus) {
            Ok(el) => el,
            Err(e) => label(format!("This layout has an error: {e}")),
        },
        None => label("No screen yet."),
    };
    let title = ed.doc.as_ref().map_or(String::new(), |d| d.title.clone());
    let frame = panel()
        .id("dframe")
        .padding(1)
        .fill(theme.bg)
        .border(1, theme.border)
        .child(column().gap(0).child(row().padding(4).child(label(if title.is_empty() { "(untitled window)".to_string() } else { title }).style("caption"))).child(preview));
    let mut b = board()
        .id("design")
        .capture(true)
        .fill(theme.surface_alt)
        .on_pointer(act_ev(sh, |ed, e| {
            if let EventValue::Pointer(p) = &e.value {
                ed.design_pointer(p);
            }
        }))
        .on_key(act_ev(sh, |ed, e| {
            if let EventValue::Key { key, .. } = e.value {
                ed.design_key(key);
            }
        }))
        .child(row().align(Align::Stretch).gap(8).padding(8).child(pal).child(column().grow(1).child(frame)));
    match &ed.design_drag {
        DesignDrag::New { kind, at } => {
            b = b.child(panel().at(at.0 + 6, at.1 + 6).padding(4).fill(theme.accent).child(label(kind_label(kind)).color(theme.accent_text).wrap(false)));
        }
        DesignDrag::Move { at, moved: true, .. } => {
            b = b.child(panel().at(at.0 + 6, at.1 + 6).padding(4).fill(theme.accent).child(label("Move here").color(theme.accent_text).wrap(false)));
        }
        _ => {}
    }
    row().align(Align::Stretch).child(b).child(board().width(200).child(scroll_view(inspector(ed, sh)).id("inspector")))
}

fn kind_label(kind: &str) -> String {
    match kind {
        "textfield" => "Text field".into(),
        "scroll" => "Scroll view".into(),
        k => {
            let mut c = k.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    }
}

fn inspector(ed: &Editor, sh: &Shared) -> Element {
    // Fields get their text only when the selection changes, so typing isn't overwritten.
    let fresh = ed.insp_seen.get() != ed.insp_rev;
    ed.insp_seen.set(ed.insp_rev);
    let rev = ed.insp_rev;
    let field = |id: String, value: &str| {
        let f = text_field().id(format!("insp{rev}:{id}"));
        if fresh {
            f.text(value)
        } else {
            f
        }
    };
    let mut col = column().gap(6).padding(8);
    let sel = ed.design_sel.as_ref().and_then(|p| Some((p.clone(), ed.doc.as_ref()?.node(p)?.clone())));
    let Some((path, node)) = sel else {
        let app = ed.graph.app.clone().unwrap_or_default();
        let title = ed.doc.as_ref().map_or(String::new(), |d| d.title.clone());
        col = col
            .child(label("Screen").style("title"))
            .child(label("Window title").style("caption"))
            .child(field("title".into(), &title).on_change(act_ev(sh, |ed, e| ed.doc().title = text_of(e))))
            .child(label("App id").style("caption"))
            .child(field("appid".into(), &app.id).placeholder(ed.app_id()).on_change(act_ev(sh, |ed, e| ed.set_app_field("id", &text_of(e)))))
            .child(label("App name").style("caption"))
            .child(field("appname".into(), &app.name).on_change(act_ev(sh, |ed, e| ed.set_app_field("name", &text_of(e)))))
            .child(label("Version").style("caption"))
            .child(field("appver".into(), &app.version).placeholder("1.0.0").on_change(act_ev(sh, |ed, e| ed.set_app_field("version", &text_of(e)))))
            .child(label("Drag widgets from the left onto the screen. Click one to edit it.").style("caption"));
        return col;
    };
    col = col.child(label(kind_label(&node.kind)).style("title"));
    for (key, ty, value) in ed.inspector() {
        let k = key.to_string();
        let input = match ty {
            PropType::Bool => checkbox(key)
                .id(format!("insp{rev}:{key}"))
                .checked(value == "true" || value.is_empty() && crate::designer::default_true(key))
                .on_change(act_ev(sh, move |ed, e| {
                    let on = matches!(e.value, EventValue::Bool(true));
                    if let Err(err) = ed.set_prop(&k, PropType::Bool, if on { "true" } else { "false" }) {
                        ed.status = err;
                    }
                })),
            _ => field(key.to_string(), &value).on_change(act_ev(sh, move |ed, e| match ed.set_prop(&k, ty, &text_of(e)) {
                Ok(()) => {}
                Err(err) => ed.status = err,
            })),
        };
        if ty == PropType::Bool {
            col = col.child(input);
        } else {
            col = col.child(label(key).style("caption")).child(input);
        }
    }
    let events = events_of(&node.kind);
    if !events.is_empty() {
        col = col.child(label("Events").style("caption"));
        for ev in events {
            let id = node.id().unwrap_or("");
            let has = ed.graph.blocks.iter().any(|b| b.ty == "on_ui_event" && b.get("widget") == Some(id) && b.get("event") == Some(ev));
            let text = format!("{} when {}", if has { "Edit blocks" } else { "Add blocks" }, crate::editor::past(ev));
            let e = ev.to_string();
            col = col.child(button(text).id(format!("ev:{ev}")).on_click(act(sh, move |ed| ed.edit_handler(&e))));
        }
    }
    let root = path.is_empty();
    col = col.child(
        row()
            .gap(4)
            .child(button("Up").id("w-up").enabled(!root).on_click(act(sh, |ed| ed.shift_widget(-1))))
            .child(button("Down").id("w-down").enabled(!root).on_click(act(sh, |ed| ed.shift_widget(1))))
            .child(button("Parent").id("w-parent").enabled(!root).on_click(act(sh, |ed| ed.select_parent()))),
    );
    col.child(button("Delete").style("danger").id("w-delete").enabled(!root).on_click(act(sh, |ed| ed.delete_widget())))
}

fn run_panel(run: &crate::editor::RunPanel, sh: &Shared) -> Element {
    let mut out = column().gap(0).padding(4);
    for line in run.tail(7) {
        out = out.child(label(line.to_string()).font(ecm_ui_proto::Font::Mono).size(16).wrap(false));
    }
    let close = if run.running {
        button("Stop").style("danger").id("run-stop").on_click(act(sh, |ed| ed.stop()))
    } else {
        button("Close").id("run-close").on_click(act(sh, |ed| ed.run = None))
    };
    let shc = sh.clone();
    panel()
        .id("run-panel")
        .padding(4)
        .child(
            column()
                .gap(4)
                .child(row().gap(8).child(label(run.title.clone()).style("caption").wrap(false)).child(spacer()).child(close))
                .child(out.height(7 * 16 + 8))
                .child(text_field().id("stdin").placeholder("Type input for the program, then Enter").on_submit(move |cx, e| {
                    shc.borrow_mut().send_input(&text_of(e));
                    cx.set_text("stdin", "");
                })),
        )
}

/// Install the editor's window into an app.
pub fn install(app: &mut App, sh: Shared) -> u32 {
    let view = sh.clone();
    app.window("Visual Programmer", move |cx| build(&view.borrow(), &view, cx))
}
