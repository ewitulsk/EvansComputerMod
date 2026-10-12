//! Small shared pieces: trust badges, progress bars, app icons.

use crate::png::Img;
use ecm_ui::element::{image, label, panel, row, Element};
use ecm_ui::{Align, Theme};

pub const GREEN: u32 = 0x2ea0_43ff;
pub const AMBER: u32 = 0x9e6a_03ff;
pub const RED: u32 = 0xb623_24ff;

/// A coloured pill with short text.
pub fn pill(text: &str, fill: u32) -> Element {
    panel()
        .fill(fill)
        .radius(3)
        .padding(2)
        .align(Align::Start)
        .child(label(text).size(8).color(0xffff_ffff).wrap(false))
}

/// "Verified publisher" in green, anything else in amber.
pub fn trust_badge(verified: bool, text: &str) -> Element {
    pill(text, if verified { GREEN } else { AMBER })
}

/// A bar `pct` percent full, `w` lu wide.
pub fn progress_bar(theme: &Theme, pct: u32, w: i32) -> Element {
    let pct = pct.min(100) as i32;
    panel()
        .fill(theme.track)
        .radius(2)
        .sized(w, 6)
        .align(Align::Start)
        .child(panel().fill(theme.accent).radius(2).sized((w * pct / 100).max(if pct > 0 { 2 } else { 0 }), 6).align(Align::Start))
}

/// An app icon at `size` lu, or a tile with the name's first letter.
pub fn icon(img: Option<Img>, name: &str, size: i32, theme: &Theme) -> Element {
    match img {
        Some(i) => image(i.resource, size, size),
        None => panel()
            .fill(theme.surface_alt)
            .radius(4)
            .sized(size, size)
            .align(Align::Start)
            .child(
                label(name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into()))
                    .size(if size >= 48 { 24 } else { 16 })
                    .align(Align::Center),
            ),
    }
}

/// A row of label text with a muted caption after it.
pub fn caption_row(main: &str, caption: &str) -> Element {
    row().gap(6).child(label(main).wrap(false)).child(label(caption).style("caption").wrap(false))
}

/// "12.3 KB"
pub fn size_text(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes() {
        assert_eq!(super::size_text(900), "900 B");
        assert_eq!(super::size_text(3072), "3.0 KB");
        assert_eq!(super::size_text(5 * 1024 * 1024), "5.0 MB");
    }
}
