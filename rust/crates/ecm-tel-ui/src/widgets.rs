//! Small pieces the telephony apps draw with.

use ecm_ui::element::{column, label, panel, row, Element};
use ecm_ui::{Align, ViewCtx};

pub const GREEN: u32 = 0x2f9e_5bff;
pub const AMBER: u32 = 0xc98a_1eff;
pub const RED: u32 = 0xd0443eff;
pub const GREY: u32 = 0x5a62_70ff;

/// A small coloured tag.
pub fn pill(text: &str, fill: u32) -> Element {
    panel().fill(fill).radius(3).padding(2).align(Align::Start).child(label(text).size(8).color(0xffff_ffff).wrap(false))
}

/// A titled card on the surface colour.
pub fn card(cx: &ViewCtx, title: &str, body: Element) -> Element {
    panel().fill(cx.theme.surface).radius(6).padding(8).child(column().gap(6).child(label(title).style("title")).child(body))
}

/// `key  value` rows with the keys in a column `kw` wide.
pub fn kv_rows(rows: &[(&str, String)], kw: i32) -> Element {
    let mut c = column().gap(3);
    for (k, v) in rows {
        c = c.child(row().gap(6).child(label(*k).style("caption").width(kw).wrap(false)).child(label(v.clone()).grow(1)));
    }
    c
}

/// A message line: green for done, red for a failure.
pub fn message(cx: &ViewCtx, m: &Option<(bool, String)>) -> Element {
    match m {
        Some((true, t)) => label(t.clone()).color(GREEN).id("message"),
        Some((false, t)) => label(t.clone()).color(cx.theme.danger).id("message"),
        None => label("").id("message"),
    }
}

/// A round badge with someone's initials (contacts without a photo).
pub fn avatar(initials: &str, size: i32) -> Element {
    let hue = initials.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let fills = [0x3a7b_d5ff, 0x2f9e_5bff, 0xc98a_1eff, 0x8e55_c2ff, 0xd0443eff, 0x2a9d_a8ff];
    panel().fill(fills[(hue % fills.len() as u32) as usize]).radius((size / 2) as u16).width(size).height(size).align(Align::Center).child(label(initials).size((size / 2).clamp(8, 40) as u8).color(0xffff_ffff).wrap(false))
}
