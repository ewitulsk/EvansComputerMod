//! Graphics host-function wrappers for WASI child programs.
//!
//! This is deliberately minimal: WASI children drive gfx state only
//! through [`init`], [`set_mode`], and [`screen_dims`]. All actual pixel
//! pushes go through [`super::video::decode_to_gfx`], where the host
//! does decode + resample + palette conversion + blit in one step.
//!
//! Every call is parameterized by a [`super::video::Target`] so the
//! child can drive either the terminal's built-in gfx framebuffer or an
//! attached in-world Screen cluster.
//!
//! Named `gfx_child` to distinguish it from the kernel's own `gfx.rs`
//! module (which writes directly to wasm linear memory — a path the
//! WASI children cannot use because they have their own linear memory
//! distinct from the kernel's).

use super::video::Target;
use alloc::vec::Vec;
use core::mem::MaybeUninit;

extern "C" {
    fn gfx_init(target: i32, width: i32, height: i32) -> i32;
    fn gfx_set_mode(target: i32, mode: i32) -> i32;
    fn gfx_blit_rect(
        target: i32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        buf_ptr: i32,
        buf_len: i32,
        format: i32,
    ) -> i32;
    fn screen_query_dims(out_ptr: i32) -> i32;
    // Signature must match the `screen_set_power` extern already
    // declared in terminal-os's `screen.rs` (which also depends on
    // this crate); a mismatch here fails the terminal-os link step.
    fn screen_set_power(on: i32);
    // Same signature constraint with `screen.rs::screen_set_pixel_format`.
    fn screen_set_pixel_format(format: i32);
    // Item overlay (MC 1.21.1 hosts). Only linked into programs that call items_set.
    fn gfx_items_set(target: i32, list_ptr: i32, list_len: i32) -> i32;
}

/// Pixel format codes for [`blit_rect`]. Values must match the host's
/// `ComputerInstance.PIXEL_FORMAT_*` constants.
pub const FORMAT_INDEXED8: i32 = 0;
pub const FORMAT_RGBA8888: i32 = 1;

/// Initialize the selected display's graphics framebuffer with the given
/// dimensions. The host installs the canonical 3-3-2 palette and clears
/// pixels to index 0. Display mode is set to 1 (graphics-only). Call this
/// once before the first [`super::video::decode_to_gfx`] so the client
/// has a blank canvas to draw into.
pub fn init(target: Target, width: i32, height: i32) -> Result<(), ()> {
    let rc = unsafe { gfx_init(target as i32, width, height) };
    if rc < 0 { Err(()) } else { Ok(()) }
}

/// Switch the display mode on the selected target. Typical usage:
/// * 0 — return control to the text shell on program exit
/// * 1 — graphics-only (default after [`init`])
/// * 2 — overlay (graphics underneath, text layered on top)
///
/// Mode transitions only really matter for [`Target::Terminal`]; the
/// in-world Screen cluster is always rendered as pure graphics, so
/// calling this with [`Target::Screen`] is mostly harmless bookkeeping.
pub fn set_mode(target: Target, mode: i32) -> Result<(), ()> {
    let rc = unsafe { gfx_set_mode(target as i32, mode) };
    if rc < 0 { Err(()) } else { Ok(()) }
}

/// Query the attached Screen cluster's pixel dimensions. Returns
/// `Some((width, height))` if a cluster is attached, or `None`
/// otherwise. Used before [`super::video::open`] to size the decoder to
/// the cluster's native resolution.
pub fn screen_dims() -> Option<(u32, u32)> {
    let mut buf = MaybeUninit::<[u32; 2]>::uninit();
    let rc = unsafe { screen_query_dims(buf.as_mut_ptr() as i32) };
    if rc < 0 {
        None
    } else {
        let [w, h] = unsafe { buf.assume_init() };
        if w == 0 || h == 0 { None } else { Some((w, h)) }
    }
}

/// Power the attached Screen cluster on or off. No-op if no cluster
/// is attached; callers that need a "no cluster" signal should use
/// [`screen_dims`] (which returns `None` in that case). Powering on
/// restores the cluster's member blocks to their "active" face
/// texture and resumes client rendering of the framebuffer quad.
pub fn set_screen_power(on: bool) {
    unsafe { screen_set_power(if on { 1 } else { 0 }); }
}

/// Copy a `w x h` sub-rectangle of pixels at `(x, y)` on the selected
/// target's framebuffer. `pixels` is tightly packed row-major at the
/// given `format` (see [`FORMAT_INDEXED8`] and [`FORMAT_RGBA8888`]);
/// length must be at least `w * h * bpp`.
///
/// Unlike `video::decode_to_gfx`, this is a raw pixel push — no
/// palette conversion, no resampling. Programs that want a full-frame
/// push can pass `x=0, y=0, w=fb_w, h=fb_h`.
pub fn blit_rect(
    target: Target,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    pixels: &[u8],
    format: i32,
) -> Result<(), ()> {
    let rc = unsafe {
        gfx_blit_rect(
            target as i32,
            x,
            y,
            w,
            h,
            pixels.as_ptr() as i32,
            pixels.len() as i32,
            format,
        )
    };
    if rc < 0 { Err(()) } else { Ok(()) }
}

/// Switch the attached screen cluster's pixel format. Pass 0 for
/// indexed8 (the default, with a 256-entry RGB332 palette) or 1 for
/// rgba8888 (full color, 4 bytes per pixel). The host clears the
/// pixel region for the new format's byte count and bumps both dirty
/// counters so clients pick up the layout change.
pub fn set_screen_pixel_format(format: i32) {
    unsafe { screen_set_pixel_format(format); }
}

/// One item drawn over the framebuffer by the Minecraft client (item overlay).
///
/// `item` is a storage key (`"k3f"`) or an item id (`"minecraft:diamond"`).
/// Coordinates are framebuffer pixels; `clip` limits drawing (w or h 0 = no
/// clip). `label` is drawn in the slot's corner like a stack count.
#[derive(Clone, Copy, Debug)]
pub struct OverlayItem<'a> {
    pub x: i32,
    pub y: i32,
    pub size: i32,
    pub clip: (i32, i32, i32, i32),
    pub flags: u8,
    pub item: &'a str,
    pub label: &'a str,
}

/// Overlay flag bits (match `ecm_ui::ITEM_FLAG_*` and the host's `ItemOverlays.FLAG_*`).
pub mod overlay_flag {
    pub const SELECTED: u8 = 0x01;
    pub const HOVERED: u8 = 0x02;
    /// Covered (e.g. by a modal): the client darkens it and shows no tooltip.
    pub const DIMMED: u8 = 0x80;
}

/// Most items one overlay holds; extra entries are dropped.
pub const MAX_OVERLAY_ITEMS: usize = 512;

fn clamp16(v: i32) -> i16 {
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

fn clampu16(v: i32) -> u16 {
    v.clamp(0, u16::MAX as i32) as u16
}

fn put_str(out: &mut Vec<u8>, s: &str, max: usize) {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    out.push(end as u8);
    out.extend_from_slice(&s.as_bytes()[..end]);
}

/// Encode an overlay list in the host's little-endian format.
pub fn encode_items(items: &[OverlayItem]) -> Vec<u8> {
    let n = items.len().min(MAX_OVERLAY_ITEMS);
    let mut out = Vec::with_capacity(2 + n * 24);
    out.extend_from_slice(&(n as u16).to_le_bytes());
    for it in &items[..n] {
        out.extend_from_slice(&clamp16(it.x).to_le_bytes());
        out.extend_from_slice(&clamp16(it.y).to_le_bytes());
        out.extend_from_slice(&clampu16(it.size).to_le_bytes());
        out.extend_from_slice(&clamp16(it.clip.0).to_le_bytes());
        out.extend_from_slice(&clamp16(it.clip.1).to_le_bytes());
        out.extend_from_slice(&clampu16(it.clip.2).to_le_bytes());
        out.extend_from_slice(&clampu16(it.clip.3).to_le_bytes());
        out.push(it.flags);
        put_str(&mut out, it.item, 96);
        put_str(&mut out, it.label, 16);
    }
    out
}

/// Replace the selected display's item overlay (an empty slice clears it).
/// The host also clears a program's overlays when it exits. Returns how
/// many items the host kept (unknown items are skipped).
pub fn items_set(target: Target, items: &[OverlayItem]) -> Result<usize, ()> {
    let data = encode_items(items);
    let rc = unsafe { gfx_items_set(target as i32, data.as_ptr() as i32, data.len() as i32) };
    if rc < 0 { Err(()) } else { Ok(rc as usize) }
}

#[cfg(test)]
mod overlay_tests {
    use super::*;

    #[test]
    fn encodes_items_little_endian() {
        let items = [OverlayItem { x: 3, y: -1, size: 16, clip: (0, 0, 100, 50), flags: 0x80, item: "k1", label: "64" }];
        let b = encode_items(&items);
        assert_eq!(&b[0..2], &[1, 0]);
        assert_eq!(&b[2..4], &[3, 0]);
        assert_eq!(&b[4..6], &[0xFF, 0xFF]);
        assert_eq!(&b[6..8], &[16, 0]);
        assert_eq!(b[16], 0x80);
        assert_eq!(b[17], 2);
        assert_eq!(&b[18..20], b"k1");
        assert_eq!(b[20], 2);
        assert_eq!(&b[21..23], b"64");
        assert_eq!(b.len(), 23);
    }

    #[test]
    fn truncates_long_strings_on_char_boundaries() {
        let label = "\u{e9}".repeat(20);
        let items = [OverlayItem { x: 0, y: 0, size: 8, clip: (0, 0, 0, 0), flags: 0, item: "minecraft:stone", label: &label }];
        let b = encode_items(&items);
        let item_len = b[17] as usize;
        let label_len = b[18 + item_len] as usize;
        assert!(label_len <= 16 && label_len % 2 == 0);
    }
}
