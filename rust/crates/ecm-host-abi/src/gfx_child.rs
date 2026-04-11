//! Graphics host-function wrappers for WASI child programs.
//!
//! This is deliberately minimal: WASI children drive gfx state only through
//! [`init`] and [`set_mode`]. All actual pixel pushes go through
//! [`super::video::decode_to_gfx`], where the host does decode + resample +
//! palette conversion + blit in one step.
//!
//! Named `gfx_child` to distinguish it from the kernel's own `gfx.rs` module
//! (which writes directly to wasm linear memory — a path the WASI children
//! cannot use because they have their own linear memory distinct from the
//! kernel's).

extern "C" {
    fn gfx_init(width: i32, height: i32) -> i32;
    fn gfx_set_mode(mode: i32) -> i32;
}

/// Initialize the graphics framebuffer with the given dimensions. The host
/// installs the canonical 3-3-2 palette and clears pixels to index 0.
/// Display mode is set to 1 (graphics-only). Call this once before the first
/// [`super::video::decode_to_gfx`] so the client has a blank canvas to draw
/// into.
pub fn init(width: i32, height: i32) -> Result<(), ()> {
    let rc = unsafe { gfx_init(width, height) };
    if rc < 0 { Err(()) } else { Ok(()) }
}

/// Switch the display mode. Typical usage:
/// * 0 — return control to the text shell on program exit
/// * 1 — graphics-only (default after [`init`])
/// * 2 — overlay (graphics underneath, text layered on top)
pub fn set_mode(mode: i32) -> Result<(), ()> {
    let rc = unsafe { gfx_set_mode(mode) };
    if rc < 0 { Err(()) } else { Ok(()) }
}
