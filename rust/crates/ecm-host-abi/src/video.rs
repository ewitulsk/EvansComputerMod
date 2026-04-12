//! Host-function wrappers for MP4 video decoding.
//!
//! The host side (Java + bytedeco FFmpeg) owns an AVFormatContext and
//! AVCodecContext per handle. Frames are decoded, resampled to the target
//! resolution, converted to packed 3-3-2 indexed color (`AV_PIX_FMT_RGB8`),
//! and blitted straight into the selected display's framebuffer without
//! round-tripping through the WASM child's linear memory.
//!
//! Two blit destinations are available: the terminal's built-in graphics
//! framebuffer, and an attached in-world Screen cluster. The caller picks
//! the destination per-frame via [`Target`].
//!
//! The WASM child only drives lifecycle (`open`, `close`), pacing, seek,
//! and error handling — it never sees pixel bytes.

extern crate alloc;

use core::mem::MaybeUninit;

/// Selects which display a decoded frame is blitted to.
#[repr(i32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The terminal block's built-in graphics framebuffer.
    Terminal = 0,
    /// The in-world Screen cluster attached to this computer, at the
    /// cluster's native pixel dimensions (query via
    /// [`super::gfx_child::screen_dims`]).
    Screen = 1,
}

extern "C" {
    fn video_open(path_ptr: i32, path_len: i32, target_w: i32, target_h: i32) -> i32;
    fn video_get_info(handle: i32, out_ptr: i32) -> i32;
    fn video_decode_to_gfx(handle: i32, target: i32) -> i64;
    fn video_seek(handle: i32, pts_ms: i64) -> i32;
    fn video_close(handle: i32) -> i32;
}

/// Returned by [`get_info`]. Laid out to match the 32-byte struct the host
/// writes at `out_ptr`: `u32 w, u32 h, u32 fps_num, u32 fps_den, u64 frame_count, u64 duration_ms`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub frame_count: u64,
    pub duration_ms: u64,
}

/// Opaque decoder handle. The zero value is never valid.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Handle(pub i32);

/// Open an MP4 file from the computer's VFS. Returns a valid handle on success.
/// The target size is the resolution frames will be resampled to; the player
/// should match this to the size passed to [`super::gfx::init`].
pub fn open(path: &str, target_w: i32, target_h: i32) -> Result<Handle, ()> {
    let rc = unsafe {
        video_open(
            path.as_ptr() as i32,
            path.len() as i32,
            target_w,
            target_h,
        )
    };
    if rc < 0 {
        Err(())
    } else {
        Ok(Handle(rc))
    }
}

/// Fetch the static metadata for an open decoder.
pub fn get_info(h: Handle) -> Result<VideoInfo, ()> {
    let mut info = MaybeUninit::<VideoInfo>::uninit();
    let rc = unsafe { video_get_info(h.0, info.as_mut_ptr() as i32) };
    if rc < 0 {
        Err(())
    } else {
        Ok(unsafe { info.assume_init() })
    }
}

/// Decoded-frame outcome returned by [`decode_to_gfx`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DecodeStep {
    /// The host pushed a fresh frame into the gfx framebuffer. The value is
    /// the presentation timestamp in milliseconds, relative to stream start.
    Frame(i64),
    /// The stream has been fully drained.
    Eof,
    /// A decoder error occurred. Details are logged on the host side.
    Error,
}

/// Advance the decoder by one frame, writing pixel data directly into
/// the host display selected by `target`. The caller never sees pixel
/// bytes. Pass [`Target::Terminal`] to blit into the terminal's built-in
/// gfx framebuffer or [`Target::Screen`] to blit into the attached
/// in-world Screen cluster.
pub fn decode_to_gfx(h: Handle, target: Target) -> DecodeStep {
    let rc = unsafe { video_decode_to_gfx(h.0, target as i32) };
    if rc == -1 {
        DecodeStep::Eof
    } else if rc == -2 {
        DecodeStep::Error
    } else {
        DecodeStep::Frame(rc)
    }
}

/// Seek the stream to the given presentation timestamp (milliseconds).
pub fn seek(h: Handle, pts_ms: i64) -> Result<(), ()> {
    let rc = unsafe { video_seek(h.0, pts_ms) };
    if rc < 0 { Err(()) } else { Ok(()) }
}

/// Close the decoder and free its native resources.
pub fn close(h: Handle) -> Result<(), ()> {
    let rc = unsafe { video_close(h.0) };
    if rc < 0 { Err(()) } else { Ok(()) }
}
