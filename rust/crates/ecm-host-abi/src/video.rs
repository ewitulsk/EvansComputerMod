//! Host-function wrappers for video decoding (abi/child-abi.toml `[imports.video]`).
//!
//! The host side (Java + bytedeco FFmpeg) owns an AVFormatContext and
//! AVCodecContext per handle. Frames are decoded and scaled to the size given
//! to [`open`], in a display pixel format ([`PixelFormat`]).
//!
//! A frame goes one of two ways:
//!
//! * [`decode_to_gfx`]: straight into a display's framebuffer (the
//!   terminal's own, or an attached in-world Screen cluster, picked per
//!   frame with [`Target`]); the program never sees the pixels.
//! * [`decode_frame`]: into the program's own buffer, for apps that show it
//!   themselves (a UI canvas).
//!
//! Sound is off until [`audio_open`]; [`read_audio`] then gives signed
//! 16-bit PCM, aligned so the first byte after [`open`] or a [`seek`] is at
//! the stream position.

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

/// Output pixel format for [`open`]. Indexed8 is the legacy 1-byte RGB332
/// palette format; Rgba8888 is full color, 4 bytes per pixel; Rgb565 is
/// 2 bytes per pixel (little-endian `rrrrrggggggbbbbb`).
#[repr(i32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Indexed8 = 0,
    Rgba8888 = 1,
    Rgb565 = 2,
}

impl PixelFormat {
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            PixelFormat::Indexed8 => 1,
            PixelFormat::Rgb565 => 2,
            PixelFormat::Rgba8888 => 4,
        }
    }
}

extern "C" {
    fn video_open(path_ptr: i32, path_len: i32, target_w: i32, target_h: i32, format: i32) -> i32;
    fn video_get_info(handle: i32, out_ptr: i32) -> i32;
    fn video_decode_to_gfx(handle: i32, target: i32) -> i64;
    fn video_decode_frame(handle: i32, buf: i32, cap: i32) -> i64;
    fn video_seek(handle: i32, pts_ms: i64) -> i32;
    fn video_audio_open(handle: i32, rate: i32, channels: i32) -> i32;
    fn video_read_audio(handle: i32, buf: i32, cap: i32) -> i32;
    fn video_audio_source(handle: i32) -> i32;
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

/// Open an MP4 file from the computer's VFS in the given output pixel
/// format. Returns a valid handle on success. The target size is the
/// resolution frames will be resampled to; the player should match this
/// to the size passed to [`super::gfx_child::init`].
pub fn open(path: &str, target_w: i32, target_h: i32, format: PixelFormat) -> Result<Handle, ()> {
    let rc = unsafe {
        video_open(
            path.as_ptr() as i32,
            path.len() as i32,
            target_w,
            target_h,
            format as i32,
        )
    };
    if rc < 0 {
        Err(())
    } else {
        Ok(Handle(rc))
    }
}

impl VideoInfo {
    /// Frames per second, 0 when the file doesn't say.
    pub fn fps(&self) -> f64 {
        if self.fps_den > 0 {
            self.fps_num as f64 / self.fps_den as f64
        } else {
            0.0
        }
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

/// Decode the next frame into `buf`, which must hold a whole frame
/// (`w * h * bytes_per_pixel` of the size and format given to [`open`]).
pub fn decode_frame(h: Handle, buf: &mut [u8]) -> DecodeStep {
    let rc = unsafe { video_decode_frame(h.0, buf.as_mut_ptr() as i32, buf.len() as i32) };
    if rc == -1 {
        DecodeStep::Eof
    } else if rc < 0 {
        DecodeStep::Error
    } else {
        DecodeStep::Frame(rc)
    }
}

/// Start decoding the file's sound as signed 16-bit little-endian PCM at
/// `rate` Hz with `channels` (1 or 2). `Err` when the file has no audio.
pub fn audio_open(h: Handle, rate: u32, channels: u8) -> Result<(), ()> {
    let rc = unsafe { video_audio_open(h.0, rate as i32, channels as i32) };
    if rc < 0 { Err(()) } else { Ok(()) }
}

/// Take queued PCM into `buf` (whole sample frames). `Ok(0)` at the end of
/// the sound.
pub fn read_audio(h: Handle, buf: &mut [u8]) -> Result<usize, ()> {
    let rc = unsafe { video_read_audio(h.0, buf.as_mut_ptr() as i32, buf.len() as i32) };
    if rc < 0 { Err(()) } else { Ok(rc as usize) }
}

/// The file's own audio format `(rate, channels)`, or None without sound.
pub fn audio_source(h: Handle) -> Option<(u32, u8)> {
    let rc = unsafe { video_audio_source(h.0) };
    if rc <= 0 {
        None
    } else {
        Some(((rc as u32) >> 8, (rc & 0xFF) as u8))
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
