//! Framebuffer API for pixel display operations.
//!
//! These functions interact with the host's framebuffer via WASM host functions.
//! They return -1 when no display is attached to the computer.

extern "C" {
    fn fb_get_width() -> i32;
    fn fb_get_height() -> i32;
    fn fb_set_pixel(x: i32, y: i32, r: i32, g: i32, b: i32, a: i32);
    fn fb_fill_rect(x: i32, y: i32, w: i32, h: i32, r: i32, g: i32, b: i32, a: i32);
    fn fb_write_region(x: i32, y: i32, w: i32, h: i32, ptr: *const u8, len: i32) -> i32;
    fn fb_clear(r: i32, g: i32, b: i32, a: i32);
    fn fb_flush() -> i32;
    fn fb_blit_text(
        x: i32, y: i32, ptr: *const u8, len: i32,
        fg_r: i32, fg_g: i32, fg_b: i32,
        bg_r: i32, bg_g: i32, bg_b: i32,
    ) -> i32;
}

/// Returns the display width in pixels, or -1 if no display attached.
pub fn get_width() -> i32 {
    unsafe { fb_get_width() }
}

/// Returns the display height in pixels, or -1 if no display attached.
pub fn get_height() -> i32 {
    unsafe { fb_get_height() }
}

/// Returns true if a display is attached.
pub fn is_attached() -> bool {
    get_width() > 0
}

/// Set a single pixel.
pub fn set_pixel(x: i32, y: i32, r: u8, g: u8, b: u8, a: u8) {
    unsafe { fb_set_pixel(x, y, r as i32, g as i32, b as i32, a as i32) }
}

/// Fill a rectangle with a solid color.
pub fn fill_rect(x: i32, y: i32, w: i32, h: i32, r: u8, g: u8, b: u8, a: u8) {
    unsafe { fb_fill_rect(x, y, w, h, r as i32, g as i32, b as i32, a as i32) }
}

/// Write a region of RGBA pixels from a byte slice.
pub fn write_region(x: i32, y: i32, w: i32, h: i32, data: &[u8]) -> i32 {
    unsafe { fb_write_region(x, y, w, h, data.as_ptr(), data.len() as i32) }
}

/// Clear the entire display with a solid color.
pub fn clear(r: u8, g: u8, b: u8, a: u8) {
    unsafe { fb_clear(r as i32, g as i32, b as i32, a as i32) }
}

/// Flush dirty tiles to connected clients. Returns number of tiles flushed.
pub fn flush() -> i32 {
    unsafe { fb_flush() }
}

/// Render text at a pixel position with foreground and background colors.
pub fn blit_text(x: i32, y: i32, text: &str, fg: (u8, u8, u8), bg: (u8, u8, u8)) -> i32 {
    unsafe {
        fb_blit_text(
            x, y, text.as_ptr(), text.len() as i32,
            fg.0 as i32, fg.1 as i32, fg.2 as i32,
            bg.0 as i32, bg.1 as i32, bg.2 as i32,
        )
    }
}

/// Draw a horizontal line.
pub fn hline(x: i32, y: i32, w: i32, r: u8, g: u8, b: u8, a: u8) {
    fill_rect(x, y, w, 1, r, g, b, a);
}

/// Draw a vertical line.
pub fn vline(x: i32, y: i32, h: i32, r: u8, g: u8, b: u8, a: u8) {
    fill_rect(x, y, 1, h, r, g, b, a);
}

/// Draw a rectangle outline (not filled).
pub fn rect(x: i32, y: i32, w: i32, h: i32, r: u8, g: u8, b: u8, a: u8) {
    hline(x, y, w, r, g, b, a);         // top
    hline(x, y + h - 1, w, r, g, b, a); // bottom
    vline(x, y, h, r, g, b, a);         // left
    vline(x + w - 1, y, h, r, g, b, a); // right
}
