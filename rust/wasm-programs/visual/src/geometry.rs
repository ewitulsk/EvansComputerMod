//! Where blocks, ports and fields sit on the canvas, at a zoom level.
//!
//! Zoom levels are the `ui` font sizes (8, 12, 16, 24 lu = 67 %, 100 %,
//! 133 %, 200 %), so block text stays sharp: every length scales with the
//! font and text is measured with the real ECM font metrics. All positions
//! here are relative to the block's top-left corner, in lu at that zoom.

use crate::blocks::{BlockDef, PortDef};
use crate::graph::{Block, Graph};
use ecm_ui_proto::fonts::{self, Face};
use ecm_ui_proto::Font;

/// Zoom levels: the font size blocks are drawn with.
pub const ZOOMS: [u8; 4] = [8, 12, 16, 24];
/// The zoom level for 100 %.
pub const DEFAULT_ZOOM: usize = 1;
/// Text fields are this much taller than their font (2 × the theme's control padding).
pub const FIELD_PAD_Y: i32 = 8;
/// A block title wider than this (lu at 100 %) wraps onto a second line,
/// and is cut with "..." after that: the info button shows all of it.
pub const TITLE_MAX: i32 = 200;
/// Value fields grow with their text up to this width (lu at 100 %); a text
/// field scrolls beyond it, an option field cuts its label with "...".
pub const FIELD_MAX: i32 = 160;

/// The zoom level closest to a saved zoom factor.
pub fn zoom_index(zoom: f64) -> usize {
    let mut best = DEFAULT_ZOOM;
    let mut err = f64::MAX;
    for (i, z) in ZOOMS.iter().enumerate() {
        let e = (*z as f64 / 12.0 - zoom).abs();
        if e < err - 1e-9 {
            best = i;
            err = e;
        }
    }
    best
}

/// Lengths at one zoom level.
#[derive(Copy, Clone, Debug)]
pub struct Metrics {
    /// Font size (lu).
    pub fs: u8,
    pub face: Face,
}

impl Metrics {
    pub fn new(zoom: usize) -> Metrics {
        let fs = ZOOMS[zoom.min(ZOOMS.len() - 1)];
        Metrics { fs, face: fonts::face(Font::Ui, fs).expect("ui font size") }
    }
    /// The zoom factor.
    pub fn factor(&self) -> f64 {
        self.fs as f64 / 12.0
    }
    /// A length designed at 100 %, scaled.
    pub fn s(&self, v: i32) -> i32 {
        (v as f64 * self.factor()).round() as i32
    }
    pub fn text(&self, s: &str) -> i32 {
        self.face.text_width(s)
    }
    pub fn header(&self) -> i32 {
        self.s(24)
    }
    pub fn row(&self) -> i32 {
        (self.fs as i32 + FIELD_PAD_Y + self.s(4)).max(self.s(24))
    }
    pub fn port_r(&self) -> i32 {
        self.s(5).max(3)
    }
    pub fn field_h(&self) -> i32 {
        self.fs as i32 + FIELD_PAD_Y
    }
    /// The line height of the block font.
    pub fn lh(&self) -> i32 {
        self.face.line_height()
    }
    /// The side of the square info button in a block's header.
    pub fn info(&self) -> i32 {
        self.s(14).max(self.lh() + 2)
    }
}

/// What sits in front of an unconnected data input.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    /// A text field.
    Text,
    /// Click to cycle `options`.
    Options,
}

/// One data input row.
#[derive(Clone, Debug)]
pub struct InputRow {
    pub port: String,
    pub y: i32,
    pub label_x: i32,
    /// The `any` type tag, if shown: (x, width).
    pub tag: Option<(i32, i32)>,
    /// The value field, if the input is unconnected: (kind, x, width).
    pub field: Option<(Field, i32, i32)>,
    /// Option fields: the label they show, cut to fit the field.
    pub option_text: Option<String>,
}

/// A block's layout.
#[derive(Clone, Debug)]
pub struct BlockGeom {
    pub w: i32,
    pub h: i32,
    /// Height of the colored header (one or two title lines).
    pub header: i32,
    /// The title, wrapped (at most two lines; the second cut with "...").
    pub title: Vec<String>,
    /// Where the first title line starts (relative to the block).
    pub title_x: i32,
    pub title_y: i32,
    /// The info button: (x, y, side), in the header's right corner.
    pub info: (i32, i32, i32),
    /// Labels of the flow outputs when there are several: (text, x, y) of
    /// each label's top-left corner.
    pub flow_labels: Vec<(String, i32, i32)>,
    pub flow_in: Option<(String, i32, i32)>,
    pub flow_outs: Vec<(String, i32, i32)>,
    pub data_ins: Vec<(String, i32, i32)>,
    pub data_outs: Vec<(String, i32, i32)>,
    pub rows: Vec<InputRow>,
}

/// The value a field shows for a port (option ports show their label).
pub fn shown_value(block: &Block, port: &PortDef) -> String {
    let v = block.get(&port.name).map(String::from).or_else(|| port.default.clone()).unwrap_or_default();
    if port.has_options() {
        format!("{} v", port.label_for_value(&v))
    } else {
        v
    }
}

/// The type tag text of an `any` port.
pub fn tag_text(block: &Block, port: &str) -> &'static str {
    match block.get(&format!("{port}_type")).unwrap_or("string") {
        "int" => "[Int]",
        "float" => "[Dec]",
        _ => "[Txt]",
    }
}

/// The title's lines: wrapped at [`TITLE_MAX`], at most two, the second cut
/// with "..." when the rest doesn't fit.
pub fn title_lines(label: &str, m: &Metrics) -> Vec<String> {
    let max = m.s(TITLE_MAX);
    let lines = ecm_ui::text::wrap(&m.face, label, Some(max));
    if lines.len() <= 2 {
        return lines;
    }
    let rest = lines[1..].join(" ");
    vec![lines[0].clone(), ecm_ui::text::ellipsize(&m.face, &rest, max)]
}

/// Lay out `block` (`VisualProgrammingScreen.getBlockWidth/Height` and the
/// port positions, scaled to the ECM font). Every text is measured with the
/// font: nothing is drawn outside the block or over another text.
pub fn block_geom(def: &BlockDef, block: &Block, graph: &Graph, m: &Metrics) -> BlockGeom {
    let lh = m.lh();
    let title = title_lines(&def.label, m);
    let title_w = title.iter().map(|l| m.text(l)).max().unwrap_or(0);
    let header = m.header().max(title.len() as i32 * lh + m.s(8));
    let isz = m.info();
    let row = m.row().max(lh + m.s(4));
    let gap = m.s(4);
    let mut left = 0;
    let mut rows = Vec::new();
    for (i, p) in def.data_inputs().enumerate() {
        let connected = graph.is_input_connected(block.id, &p.name);
        let label_x = m.port_r() + m.s(3);
        let mut x = label_x + m.text(&p.name) + gap;
        let tag = if p.ty == "any" && !connected {
            let w = m.text(tag_text(block, &p.name)) + m.s(4);
            let t = Some((x, w));
            x += w + m.s(2);
            t
        } else {
            None
        };
        let mut option_text = None;
        let field = if connected {
            None
        } else {
            let shown = shown_value(block, p);
            let w = (m.text(&shown) + 2 * 8 + 4).max(m.s(80)).min(m.s(FIELD_MAX).max(m.s(80)));
            let kind = if p.has_options() { Field::Options } else { Field::Text };
            if kind == Field::Options {
                option_text = Some(ecm_ui::text::ellipsize(&m.face, &shown, w - 2 * 8));
            }
            let f = Some((kind, x, w));
            x += w;
            f
        };
        left = left.max(x + m.s(10));
        rows.push(InputRow { port: p.name.clone(), y: header + i as i32 * row, label_x, tag, field, option_text });
    }
    let right = def.data_outputs().map(|p| m.text(&p.name) + m.s(20)).max().unwrap_or(0);
    let flow_outs: Vec<&PortDef> = def.flow_outputs().collect();
    let n = flow_outs.len() as i32;
    // Several flow outputs are labelled, spread evenly along the bottom:
    // each label gets a slot of w / (n + 1) around its port.
    let flow_label_w = if n > 1 { flow_outs.iter().map(|p| m.text(&p.name)).max().unwrap_or(0) + m.s(8) } else { 0 };
    let w = m
        .s(120)
        .max(m.s(8) + title_w + m.s(6) + isz + m.s(4))
        .max(left + right + m.s(10))
        .max(flow_label_w * (n + 1));
    let nrows = def.data_inputs().count().max(def.data_outputs().count()) as i32;
    let mut h = header + nrows * row + m.s(8);
    if n > 1 {
        h += lh + m.s(4);
    }
    let flow_in = def.flow_inputs().next().map(|p| (p.name.clone(), w / 2, 0));
    let flow_labels = if n > 1 {
        flow_outs.iter().enumerate().map(|(i, p)| (p.name.clone(), w * (i as i32 + 1) / (n + 1) - m.text(&p.name) / 2, h - lh - m.s(4))).collect()
    } else {
        Vec::new()
    };
    let flow_outs = flow_outs.iter().enumerate().map(|(i, p)| (p.name.clone(), w * (i as i32 + 1) / (n + 1), h)).collect();
    let data_ins = def.data_inputs().enumerate().map(|(i, p)| (p.name.clone(), 0, header + i as i32 * row + row / 2)).collect();
    let data_outs = def.data_outputs().enumerate().map(|(i, p)| (p.name.clone(), w, header + i as i32 * row + row / 2)).collect();
    let title_y = (header - title.len() as i32 * lh) / 2;
    let info = (w - m.s(4) - isz, (header - isz) / 2, isz);
    BlockGeom { w, h, header, title, title_x: m.s(8), title_y, info, flow_labels, flow_in, flow_outs, data_ins, data_outs, rows }
}

impl BlockGeom {
    /// A port's position relative to the block.
    pub fn port(&self, name: &str, output: bool) -> Option<(i32, i32)> {
        let find = |v: &[(String, i32, i32)]| v.iter().find(|p| p.0 == name).map(|p| (p.1, p.2));
        if output {
            find(&self.flow_outs).or_else(|| find(&self.data_outs))
        } else {
            match &self.flow_in {
                Some(f) if f.0 == name => Some((f.1, f.2)),
                _ => find(&self.data_ins),
            }
        }
    }
}

/// A wire from `a` to `b`: a cubic curve leaving outputs downward (flow)
/// or rightward (data) and entering inputs the same way.
pub fn wire_points(a: (i32, i32), b: (i32, i32), flow: bool, m: &Metrics) -> Vec<(i32, i32)> {
    let (ax, ay, bx, by) = (a.0 as f64, a.1 as f64, b.0 as f64, b.1 as f64);
    let reach = if flow { ((by - ay).abs() / 2.0).max(m.s(30) as f64) } else { ((bx - ax).abs() / 2.0).max(m.s(30) as f64) };
    let (c1, c2) = if flow { ((ax, ay + reach), (bx, by - reach)) } else { ((ax + reach, ay), (bx - reach, by)) };
    const STEPS: usize = 20;
    let mut out: Vec<(i32, i32)> = Vec::with_capacity(STEPS + 1);
    for i in 0..=STEPS {
        let t = i as f64 / STEPS as f64;
        let u = 1.0 - t;
        let x = u * u * u * ax + 3.0 * u * u * t * c1.0 + 3.0 * u * t * t * c2.0 + t * t * t * bx;
        let y = u * u * u * ay + 3.0 * u * u * t * c1.1 + 3.0 * u * t * t * c2.1 + t * t * t * by;
        let p = (x.round() as i32, y.round() as i32);
        if out.last() != Some(&p) {
            out.push(p);
        }
    }
    if out.len() == 1 {
        out.push(out[0]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Registry;

    #[test]
    fn zoom_levels_follow_font_sizes() {
        assert_eq!(zoom_index(1.0), 1);
        assert_eq!(zoom_index(0.25), 0);
        assert_eq!(zoom_index(1.15), 1);
        assert_eq!(zoom_index(1.3), 2);
        assert_eq!(zoom_index(3.0), 3);
        assert_eq!(Metrics::new(3).factor(), 2.0);
    }

    #[test]
    fn blocks_grow_with_their_ports_and_zoom() {
        let reg = Registry::builtin();
        let mut g = Graph::new("t");
        let p = g.add_block(reg.find("print").unwrap(), 0.0, 0.0);
        let s = g.add_block(reg.find("if").unwrap(), 0.0, 0.0);
        let m = Metrics::new(DEFAULT_ZOOM);
        let pg = block_geom(reg.find("print").unwrap(), g.block(p).unwrap(), &g, &m);
        assert!(pg.w >= 120);
        assert_eq!(pg.flow_in, Some(("flow".into(), pg.w / 2, 0)));
        assert_eq!(pg.port("flow", true), Some((pg.w / 2, pg.h)));
        let row = &pg.rows[0];
        assert!(row.tag.is_some(), "print's text is an `any` port");
        assert!(matches!(row.field, Some((Field::Text, _, w)) if w >= 80));
        // Two flow outputs spread along the bottom, with room for labels.
        let ig = block_geom(reg.find("if").unwrap(), g.block(s).unwrap(), &g, &m);
        assert_eq!(ig.flow_outs.len(), 2);
        assert!(ig.flow_outs[0].1 < ig.flow_outs[1].1);
        // Zooming in makes everything bigger.
        let big = block_geom(reg.find("print").unwrap(), g.block(p).unwrap(), &g, &Metrics::new(3));
        assert!(big.w > pg.w && big.h > pg.h);
        // A connected input loses its field.
        let n = g.add_block(reg.find("number_constant").unwrap(), 0.0, 0.0);
        g.conns.push(crate::graph::Conn { from: n, from_port: "value".into(), to: p, to_port: "text".into() });
        let pc = block_geom(reg.find("print").unwrap(), g.block(p).unwrap(), &g, &m);
        assert!(pc.rows[0].field.is_none() && pc.rows[0].tag.is_none());
    }

    #[test]
    fn wires_are_short_polylines_between_ports() {
        let m = Metrics::new(DEFAULT_ZOOM);
        let pts = wire_points((10, 10), (200, 100), false, &m);
        assert_eq!(pts.first(), Some(&(10, 10)));
        assert_eq!(pts.last(), Some(&(200, 100)));
        assert!(pts.len() <= 21);
        let flow = wire_points((50, 0), (50, 0), true, &m);
        assert!(flow.len() >= 2);
    }
}
