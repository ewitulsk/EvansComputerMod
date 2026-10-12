//! Package: turn a designer-built program into an app folder
//! (docs/gui/DISPLAY_PIPELINE.md §8 "Package format"):
//!
//! ```text
//! visual/apps/<app id>/
//! ├── app.toml        id, name, version, entry, icon, sizes, permissions
//! ├── main.py         the generated program (layout found next to it)
//! ├── main.ui.json    the layout
//! └── icon.png        32×32, the app's initial on its category color
//! ```
//!
//! Permissions are inferred from the blocks used: a peripheral block asks
//! for `peripherals:<type>`, the redstone blocks for `peripherals:redstone`,
//! Notify for `notifications`, and blocks whose code opens sockets for
//! `network`.

use crate::blocks::Registry;
use crate::codegen::{generate, AppTarget};
use crate::designer::LayoutDoc;
use crate::graph::Graph;
use ecm_ui_proto::fonts;
use ecm_ui_proto::Font;

/// The files of an app folder.
#[derive(Clone, Debug, PartialEq)]
pub struct AppFolder {
    pub dir: String,
    pub files: Vec<(String, Vec<u8>)>,
    pub permissions: Vec<String>,
}

/// `My App!` → `my_app` (for ids and file names).
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_').to_string();
    if out.is_empty() {
        "app".into()
    } else {
        out
    }
}

/// A reverse-DNS app id for a program name.
pub fn default_app_id(name: &str) -> String {
    format!("dev.visual.{}", slug(name))
}

/// The permission a peripheral type's calls need: the app platform checks
/// radio hardware as `radio`, modems as `cellular`, controllers as
/// `controller`, everything else as `peripherals:<type>` (docs/gui/APPS.md,
/// "Where permissions are enforced").
pub fn peripheral_permission(ty: &str) -> String {
    match ty {
        "sdr" | "amplifier" | "antenna" | "dish" | "microwave_radio" | "wifi" => "radio".into(),
        "lte_modem" | "mmwave_modem" => "cellular".into(),
        "xbox_controller" | "controller_receiver" => "controller".into(),
        t => format!("peripherals:{t}"),
    }
}

/// The permissions a graph's blocks need, sorted.
pub fn infer_permissions(g: &Graph, reg: &Registry) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |p: String| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    for b in &g.blocks {
        if let Some(cat) = reg.category_of(&b.ty) {
            if let Some(t) = &cat.peripheral {
                add(peripheral_permission(t));
            }
        }
        match b.ty.as_str() {
            "set_redstone" | "on_redstone" => add("peripherals:redstone".into()),
            "ui_notify" => add("notifications".into()),
            _ => {}
        }
        if b.ty.starts_with("net_") || b.ty.starts_with("http_") {
            add("network".into());
        }
        if let Some(def) = reg.find(&b.ty) {
            if ["socket.", "urllib", "http.client", "requests."].iter().any(|k| def.code.contains(k)) {
                add("network".into());
            }
        }
    }
    out.sort();
    out
}

fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The `app.toml` text.
pub fn app_toml(id: &str, name: &str, version: &str, permissions: &[String], source: &str) -> String {
    let perms: Vec<String> = permissions.iter().map(|p| toml_str(p)).collect();
    format!(
        "# Built by the Visual Programmer from {source}\nid          = {}\nname        = {}\nversion     = {}\nentry       = \"main.py\"\nicon        = \"icon.png\"\nsizes       = [\"compact\", \"medium\", \"wide\"]\npermissions = [{}]\n",
        toml_str(id),
        toml_str(name),
        toml_str(version),
        perms.join(", ")
    )
}

/// Build the app folder for a program and its layout.
pub fn build(g: &Graph, layout: &LayoutDoc, reg: &Registry, source: &str) -> AppFolder {
    let info = g.app.clone().unwrap_or_default();
    let id = if info.id.is_empty() { default_app_id(&g.name) } else { info.id.clone() };
    let name = if info.name.is_empty() { layout.title.clone() } else { info.name.clone() };
    let name = if name.is_empty() { g.name.clone() } else { name };
    let version = if info.version.is_empty() { "1.0.0".to_string() } else { info.version.clone() };
    let permissions = infer_permissions(g, reg);
    let mut doc = layout.clone();
    crate::designer::sync_handlers(&mut doc, g);
    let code = generate(g, reg, Some(&AppTarget { layout: "main.ui.json".into(), app_id: id.clone() }));
    let dir = format!("visual/apps/{id}");
    let files = vec![
        ("app.toml".to_string(), app_toml(&id, &name, &version, &permissions, source).into_bytes()),
        ("main.py".to_string(), code.into_bytes()),
        ("main.ui.json".to_string(), doc.to_text().into_bytes()),
        ("icon.png".to_string(), icon_png(&name, 0x00897bff)),
    ];
    AppFolder { dir, files, permissions }
}

// ------------------------------------------------------------------ icon

/// A 32×32 RGBA icon: a rounded square in `color` with the name's first
/// letter in white (the `ui` font's regular design, doubled).
pub fn icon_png(name: &str, color: u32) -> Vec<u8> {
    const S: usize = 32;
    let mut px = vec![0u8; S * S * 4];
    let rgba = [(color >> 24) as u8, (color >> 16) as u8, (color >> 8) as u8, 0xff];
    for y in 0..S {
        for x in 0..S {
            // Rounded corners: radius 6.
            let (cx, cy) = (x.min(S - 1 - x), y.min(S - 1 - y));
            if cx < 6 && cy < 6 {
                let (dx, dy) = (6 - cx, 6 - cy);
                if dx * dx + dy * dy > 36 {
                    continue;
                }
            }
            px[(y * S + x) * 4..(y * S + x) * 4 + 4].copy_from_slice(&rgba);
        }
    }
    if let (Some(c), Some(face)) = (name.chars().find(|c| c.is_alphanumeric()), fonts::face(Font::Ui, 24)) {
        let c = c.to_uppercase().next().unwrap_or(c);
        let g = face.glyph(c);
        let scale = face.scale as usize;
        let (gw, gh) = (g.width as usize * scale, g.rows.len() * scale);
        let (ox, oy) = ((S.saturating_sub(gw)) / 2, (S.saturating_sub(gh)) / 2);
        for (ry, bits) in g.rows.iter().enumerate() {
            for col in 0..g.width as usize {
                if bits & (1 << col) == 0 {
                    continue;
                }
                for sy in 0..scale {
                    for sx in 0..scale {
                        let (x, y) = (ox + col * scale + sx, oy + ry * scale + sy);
                        if x < S && y < S {
                            px[(y * S + x) * 4..(y * S + x) * 4 + 4].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
                        }
                    }
                }
            }
        }
    }
    png_rgba(S as u32, S as u32, &px)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// An uncompressed (stored-deflate) PNG.
pub fn png_rgba(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for row in rgba.chunks(w as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    while let Some(c) = chunks.next() {
        z.push(if chunks.peek().is_none() { 1 } else { 0 });
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_ids() {
        assert_eq!(slug("My Counter App!"), "my_counter_app");
        assert_eq!(slug("!!"), "app");
        assert_eq!(default_app_id("Counter"), "dev.visual.counter");
    }

    #[test]
    fn peripheral_permissions_follow_the_app_platform() {
        assert_eq!(peripheral_permission("sdr"), "radio");
        assert_eq!(peripheral_permission("lte_modem"), "cellular");
        assert_eq!(peripheral_permission("controller_receiver"), "controller");
        assert_eq!(peripheral_permission("redstone_link"), "peripherals:redstone_link");
    }

    #[test]
    fn png_has_valid_structure() {
        let png = icon_png("counter", 0x00897bff);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 32);
        // CRC of IHDR matches.
        let crc = u32::from_be_bytes(png[29..33].try_into().unwrap());
        assert_eq!(crc, crc32(&png[12..29]));
        assert!(png.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82]));
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }
}
