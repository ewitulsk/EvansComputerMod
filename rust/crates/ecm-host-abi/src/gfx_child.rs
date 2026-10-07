//! Graphics host-function wrappers for WASI child programs.
//!
//! A program draws on a *display*: the Terminal's own screen
//! ([`Target::Terminal`]) or the attached in-world Screen cluster
//! ([`Target::Screen`]). The model is a small display controller:
//!
//! * [`init2`] takes the display over (it is busy for other programs until
//!   this one exits or calls `set_mode(target, 0)`) and sets a mode: size,
//!   pixel format ([`FORMAT_INDEXED8`], [`FORMAT_RGB565`], [`FORMAT_RGBA8888`])
//!   and whether it is double-buffered ([`FLAG_DOUBLE_BUFFER`]).
//! * [`blit_rect`] writes pixels into the framebuffer. Single-buffered, the
//!   change is visible at once; double-buffered, it lands in the back buffer
//!   and [`present`] flips it.
//! * The display refreshes at [`set_refresh`] Hz. [`wait_vblank`] and
//!   `present(.., PRESENT_WAIT_VBLANK)` sleep until the next vertical blank.
//! * [`info`] reads the current mode and the vblank / presented counters.
//!
//! The older calls ([`init`], [`set_screen_pixel_format`]) still work: they
//! are a single-buffered display.
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
    fn gfx_init2(target: i32, width: i32, height: i32, format: i32, flags: i32) -> i32;
    fn gfx_set_format(target: i32, format: i32) -> i32;
    fn gfx_set_palette(target: i32, first: i32, count: i32, rgb_ptr: i32) -> i32;
    fn gfx_present(target: i32, flags: i32) -> i64;
    fn gfx_wait_vblank(target: i32) -> i64;
    fn gfx_set_refresh(target: i32, hz: i32) -> i32;
    fn gfx_info(target: i32, out_ptr: i32) -> i32;
}

/// Pixel format codes for [`blit_rect`]. Values must match the host's
/// `ComputerInstance.PIXEL_FORMAT_*` constants.
pub const FORMAT_INDEXED8: i32 = 0;
pub const FORMAT_RGBA8888: i32 = 1;
/// 2 bytes per pixel: little-endian `rrrrrggg gggbbbbb`.
pub const FORMAT_RGB565: i32 = 2;

/// [`init2`] flag: draw into a back buffer that [`present`] flips.
pub const FLAG_DOUBLE_BUFFER: i32 = 1;
/// [`present`] flag: wait for the next vertical blank before flipping.
pub const PRESENT_WAIT_VBLANK: i32 = 1;

/// Errors from the display calls.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum GfxError {
    /// Bad argument (size, format, rectangle outside the display, ...).
    Invalid,
    /// Another program owns the display.
    Busy,
    /// This program doesn't own the display (call [`init2`] first).
    NotOwner,
    /// No such display (e.g. no Screen cluster attached).
    NoDevice,
    /// Interrupted while waiting (the program is being killed).
    Interrupted,
}

fn err(rc: i64) -> GfxError {
    match rc {
        -2 => GfxError::Busy,
        -3 => GfxError::NotOwner,
        -4 => GfxError::NoDevice,
        -5 => GfxError::Interrupted,
        _ => GfxError::Invalid,
    }
}

/// Bytes per pixel of a pixel format.
pub fn bytes_per_pixel(format: i32) -> usize {
    match format {
        FORMAT_RGBA8888 => 4,
        FORMAT_RGB565 => 2,
        _ => 1,
    }
}

/// A display's current state, from [`info`].
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub format: i32,
    /// 0 text (the kernel owns it), 1 graphics, 2 graphics with the text console on top.
    pub mode: u32,
    pub refresh_hz: u32,
    pub flags: u32,
    /// pid of the owning program, 0 for the kernel.
    pub owner_pid: u32,
    /// Vertical blanks since the display was created.
    pub vblank: u64,
    /// Frames scanned out so far.
    pub presented: u64,
}

/// Take the display over and set a mode: `width x height` pixels in
/// `format`, cleared to 0, with the default RGB332 palette (indexed8).
pub fn init2(target: Target, width: i32, height: i32, format: i32, flags: i32) -> Result<(), GfxError> {
    let rc = unsafe { gfx_init2(target as i32, width, height, format, flags) };
    if rc < 0 { Err(err(rc as i64)) } else { Ok(()) }
}

/// Change the pixel format, keeping the size. Clears the display.
pub fn set_format(target: Target, format: i32) -> Result<(), GfxError> {
    let rc = unsafe { gfx_set_format(target as i32, format) };
    if rc < 0 { Err(err(rc as i64)) } else { Ok(()) }
}

/// Set palette entries `first..first + rgb.len() / 3` from packed `R, G, B` bytes
/// (indexed8 only).
pub fn set_palette(target: Target, first: u8, rgb: &[u8]) -> Result<(), GfxError> {
    let count = (rgb.len() / 3) as i32;
    let rc = unsafe { gfx_set_palette(target as i32, first as i32, count, rgb.as_ptr() as i32) };
    if rc < 0 { Err(err(rc as i64)) } else { Ok(()) }
}

/// Show the back buffer (double-buffered displays). With
/// [`PRESENT_WAIT_VBLANK`], first sleep until the next vertical blank.
/// Returns the number of frames presented so far.
pub fn present(target: Target, flags: i32) -> Result<u64, GfxError> {
    let rc = unsafe { gfx_present(target as i32, flags) };
    if rc < 0 { Err(err(rc)) } else { Ok(rc as u64) }
}

/// Sleep until the display's next vertical blank. Returns its number.
pub fn wait_vblank(target: Target) -> Result<u64, GfxError> {
    let rc = unsafe { gfx_wait_vblank(target as i32) };
    if rc < 0 { Err(err(rc)) } else { Ok(rc as u64) }
}

/// Ask for a refresh rate. Returns the rate granted (the server caps it).
pub fn set_refresh(target: Target, hz: u32) -> Result<u32, GfxError> {
    let rc = unsafe { gfx_set_refresh(target as i32, hz as i32) };
    if rc < 0 { Err(err(rc as i64)) } else { Ok(rc as u32) }
}

/// Read a display's current state.
pub fn info(target: Target) -> Option<DisplayInfo> {
    let mut buf = [0u8; 48];
    let rc = unsafe { gfx_info(target as i32, buf.as_mut_ptr() as i32) };
    if rc < 0 {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]);
    let u64_at = |o: usize| (u32_at(o) as u64) | ((u32_at(o + 4) as u64) << 32);
    Some(DisplayInfo {
        width: u32_at(0),
        height: u32_at(4),
        format: u32_at(8) as i32,
        mode: u32_at(12),
        refresh_hz: u32_at(16),
        flags: u32_at(20),
        owner_pid: u32_at(24),
        vblank: u64_at(32),
        presented: u64_at(40),
    })
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

/// Power the attached Screen cluster on or off. No-op if no cluster
/// is attached; callers that need a "no cluster" signal should use
/// [`screen_dims`] (which returns `None` in that case). Powering on
/// restores the cluster's member blocks to their "active" face
/// texture and resumes client rendering of the framebuffer quad.
pub fn set_screen_power(on: bool) {
    unsafe { screen_set_power(if on { 1 } else { 0 }); }
}

/// Copy a `w x h` sub-rectangle of pixels at `(x, y)` on the selected
/// target's framebuffer. `pixels` is tightly packed row-major in
/// `format`, which must be the display's format; length must be at
/// least `w * h * bytes_per_pixel(format)`.
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
