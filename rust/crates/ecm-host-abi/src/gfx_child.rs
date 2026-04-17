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
