//! Host-function wrappers for the embedded Chromium browser.
//!
//! The host side (Java + jcefmaven) owns a `CefBrowser` per handle that
//! renders off-screen into a BGRA framebuffer sized to match the
//! child-provided width/height. Each [`Browser::render`] call pulls the
//! most recent paint (if any) and stages it through the same RGBA8888
//! gfx-op path the `player` program uses, so no new kernel-side pixel
//! format is needed.
//!
//! Mouse input is forwarded from the terminal UI's mouse-capture ring to
//! the browser one event at a time via [`Browser::send_mouse`]; keyboard
//! is intentionally not plumbed in v1.

use super::mouse::MouseEvent;
use super::video::Target;

extern "C" {
    fn browser_open(url_ptr: i32, url_len: i32, w: i32, h: i32) -> i64;
    fn browser_close(handle: i64) -> i32;
    fn browser_render_to_gfx(handle: i64, target: i32) -> i32;
    fn browser_send_mouse(handle: i64, ev_ptr: i32) -> i32;
    fn browser_navigate(handle: i64, url_ptr: i32, url_len: i32) -> i32;
}

/// Owning handle to an off-screen browser on the host. Closes the browser
/// on drop so a panicking child still releases the CEF helper processes.
pub struct Browser(i64);

impl Browser {
    /// Start a new off-screen Chromium instance pointed at `url`, with an
    /// internal framebuffer of `width × height`. Returns `None` if CEF is
    /// unavailable or the underlying CEF call failed.
    pub fn open(url: &str, width: i32, height: i32) -> Option<Self> {
        let b = url.as_bytes();
        let h = unsafe { browser_open(b.as_ptr() as i32, b.len() as i32, width, height) };
        if h <= 0 { None } else { Some(Browser(h)) }
    }

    /// Push the latest paint to the selected display's framebuffer.
    ///
    /// Returns:
    /// * `Ok(true)` — a new frame was staged, the display will update
    /// * `Ok(false)` — no new paint has arrived since the last call; the
    ///   caller should keep sleeping rather than busy-looping
    /// * `Err(())` — the handle is gone or the child is being aborted
    pub fn render(&self, target: Target) -> Result<bool, ()> {
        let rc = unsafe { browser_render_to_gfx(self.0, target as i32) };
        match rc {
            0 => Ok(true),
            1 => Ok(false),
            _ => Err(()),
        }
    }

    /// Forward one mouse event (as produced by [`super::mouse::poll`]) to
    /// the browser. The 10-byte on-wire layout is re-encoded here so the
    /// host sees exactly the same bytes it would have delivered to the
    /// child via `mouse_poll` — saves the child from round-tripping bytes
    /// through its own memory just to call into the browser bridge.
    pub fn send_mouse(&self, ev: &MouseEvent) -> Result<(), ()> {
        let mut buf = [0u8; 10];
        buf[0] = ev.kind as u8;
        let x = ev.x.to_le_bytes();
        let y = ev.y.to_le_bytes();
        buf[1] = x[0]; buf[2] = x[1];
        buf[3] = y[0]; buf[4] = y[1];
        buf[5] = ev.buttons;
        buf[6] = ev.button_code;
        buf[7] = ev.scroll_dir as u8;
        // buf[8], buf[9] left as zero per the ABI's reserved tail.
        let rc = unsafe { browser_send_mouse(self.0, buf.as_ptr() as i32) };
        if rc < 0 { Err(()) } else { Ok(()) }
    }

    /// Navigate the browser to a new URL. The browser retains its size
    /// and render handler; only the displayed page changes.
    pub fn navigate(&self, url: &str) -> Result<(), ()> {
        let b = url.as_bytes();
        let rc = unsafe { browser_navigate(self.0, b.as_ptr() as i32, b.len() as i32) };
        if rc < 0 { Err(()) } else { Ok(()) }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        unsafe { browser_close(self.0) };
    }
}
