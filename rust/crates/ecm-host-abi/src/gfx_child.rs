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
    fn screen_query_dims(out_ptr: i32) -> i32;
}

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
