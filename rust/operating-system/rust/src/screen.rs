//! In-world Screen cluster graphics framebuffer.
//!
//! Mirror of [`gfx`](crate::gfx) that targets a SECOND memory-mapped graphics
//! region at [`SCREEN_GFX_BASE`] (0x50000). This region is populated by the
//! Java host when an in-world Screen block cluster is attached to the computer;
//! its width and height reflect the cluster's pixel dimensions (tile_cols * 128
//! and tile_rows * 72 by default).
//!
//! Text rendering is handled entirely Rust-side via a small built-in 5x7
//! bitmap font rasterized into the graphics buffer — the Java host treats the
//! screen display as pure graphics mode (mode=1), which simplifies the client
//! renderer to a single textured quad per cluster.
//!
//! Layout (identical to [`crate::gfx`]):
//!
//! ```text
//! Offset  Size    Field
//! 0x00    u16     magic (0xFB02)
//! 0x02    u8      mode (0=detached, 1=attached)
//! 0x03    u8      reserved
//! 0x04    u16     width (pixels)
//! 0x06    u16     height (pixels)
//! 0x08    u32     palette_dirty counter
//! 0x0C    u32     pixel_dirty counter
//! 0x40    768B    palette
//! 0x400+          indexed pixel data
//! ```

/// Base address of the in-world Screen graphics framebuffer in WASM linear memory.
pub const SCREEN_GFX_BASE: usize = 0x50000;

/// Offset of palette data from SCREEN_GFX_BASE.
pub const GFX_PALETTE_OFF: usize = 0x40;

/// Offset of pixel data from SCREEN_GFX_BASE.
pub const GFX_PIXEL_OFF: usize = 0x400;

const GFX_PALETTE_ADDR: usize = SCREEN_GFX_BASE + GFX_PALETTE_OFF;
const GFX_PIXEL_ADDR: usize = SCREEN_GFX_BASE + GFX_PIXEL_OFF;

/// Magic identifying a valid screen graphics header.
pub const GFX_MAGIC: u16 = 0xFB02;

const OFF_MAGIC: usize = 0x00;
const OFF_MODE: usize = 0x02;
const OFF_WIDTH: usize = 0x04;
const OFF_HEIGHT: usize = 0x06;
const OFF_PALETTE_DIRTY: usize = 0x08;
const OFF_PIXEL_DIRTY: usize = 0x0C;

// --- Host imports ---

extern "C" {
    /// 1 if an in-world Screen cluster is attached, else 0.
    fn screen_is_attached() -> i32;
    /// Graphics pixel width of the attached cluster (0 if none).
    fn screen_get_gfx_width() -> i32;
    /// Graphics pixel height of the attached cluster (0 if none).
    fn screen_get_gfx_height() -> i32;
    /// Hint to the host: read the screen framebuffer now and push a delta
    /// to clients. Rate-limited at the host (20 Hz).
    fn screen_fb_sync();
}

/// Returns true if the computer currently has a valid screen cluster attached.
pub fn is_attached() -> bool {
    unsafe { screen_is_attached() != 0 }
}

/// Returns the screen cluster's graphics width in pixels.
/// Prefers the live value from the host; falls back to the memory-mapped
/// header if the host call returns 0 but the header is populated.
pub fn host_width() -> u16 {
    let w = unsafe { screen_get_gfx_width() };
    if w > 0 {
        w as u16
    } else {
        unsafe { read_u16(OFF_WIDTH) }
    }
}

/// Returns the screen cluster's graphics height in pixels.
pub fn host_height() -> u16 {
    let h = unsafe { screen_get_gfx_height() };
    if h > 0 {
        h as u16
    } else {
        unsafe { read_u16(OFF_HEIGHT) }
    }
}

/// Push the current screen framebuffer to clients (rate-limited at the host).
pub fn sync() {
    unsafe { screen_fb_sync(); }
}

// --- Unsafe volatile accessors ---

#[inline(always)]
unsafe fn write_u16(offset: usize, val: u16) {
    core::ptr::write_volatile((SCREEN_GFX_BASE + offset) as *mut u16, val);
}

#[inline(always)]
unsafe fn read_u16(offset: usize) -> u16 {
    core::ptr::read_volatile((SCREEN_GFX_BASE + offset) as *const u16)
}

#[inline(always)]
unsafe fn write_u8(offset: usize, val: u8) {
    core::ptr::write_volatile((SCREEN_GFX_BASE + offset) as *mut u8, val);
}

#[inline(always)]
unsafe fn read_u8(offset: usize) -> u8 {
    core::ptr::read_volatile((SCREEN_GFX_BASE + offset) as *const u8)
}

#[inline(always)]
unsafe fn write_u32(offset: usize, val: u32) {
    core::ptr::write_volatile((SCREEN_GFX_BASE + offset) as *mut u32, val);
}

#[inline(always)]
unsafe fn read_u32(offset: usize) -> u32 {
    core::ptr::read_volatile((SCREEN_GFX_BASE + offset) as *const u32)
}

/// Initialize the screen framebuffer header (mode=1, width/height from host).
/// The palette is populated with the standard VGA 256-color palette. Pixel
/// memory is cleared to color 0.
///
/// No-op if no screen cluster is currently attached.
pub fn init() -> bool {
    if !is_attached() {
        return false;
    }
    let w = host_width();
    let h = host_height();
    if w == 0 || h == 0 {
        return false;
    }
    unsafe {
        write_u16(OFF_MAGIC, GFX_MAGIC);
        write_u8(OFF_MODE, 1);
        write_u8(OFF_MODE + 1, 0);
        write_u16(OFF_WIDTH, w);
        write_u16(OFF_HEIGHT, h);
        write_u32(OFF_PALETTE_DIRTY, 0);
        write_u32(OFF_PIXEL_DIRTY, 0);
        for i in 0x10..GFX_PALETTE_OFF {
            *((SCREEN_GFX_BASE + i) as *mut u8) = 0;
        }
    }
    init_default_palette();
    clear(0);
    mark_palette_dirty();
    mark_pixel_dirty();
    true
}

/// Mode byte: 0=detached, 1=attached (graphics only).
pub fn set_mode(mode: u8) {
    unsafe { write_u8(OFF_MODE, mode); }
}

/// Get the current mode byte.
pub fn get_mode() -> u8 {
    unsafe { read_u8(OFF_MODE) }
}

/// Width of the currently allocated screen graphics buffer.
pub fn width() -> u16 {
    unsafe { read_u16(OFF_WIDTH) }
}

/// Height of the currently allocated screen graphics buffer.
pub fn height() -> u16 {
    unsafe { read_u16(OFF_HEIGHT) }
}

#[inline(always)]
fn pixel_offset(x: u16, y: u16) -> usize {
    GFX_PIXEL_ADDR + (y as usize * width() as usize + x as usize)
}

/// Set a single pixel.
pub fn set_pixel(x: u16, y: u16, color_idx: u8) {
    let w = width();
    let h = height();
    if x >= w || y >= h { return; }
    unsafe { *(pixel_offset(x, y) as *mut u8) = color_idx; }
}

/// Get a single pixel.
pub fn get_pixel(x: u16, y: u16) -> u8 {
    let w = width();
    let h = height();
    if x >= w || y >= h { return 0; }
    unsafe { *(pixel_offset(x, y) as *const u8) }
}

/// Fill a rectangle with the given palette color.
pub fn fill_rect(x: u16, y: u16, w: u16, h: u16, color_idx: u8) {
    let fb_w = width();
    let fb_h = height();
    let x_start = x.min(fb_w);
    let y_start = y.min(fb_h);
    let x_end = (x as u32 + w as u32).min(fb_w as u32) as u16;
    let y_end = (y as u32 + h as u32).min(fb_h as u32) as u16;
    for py in y_start..y_end {
        let row_base = GFX_PIXEL_ADDR + py as usize * fb_w as usize;
        for px in x_start..x_end {
            unsafe { *((row_base + px as usize) as *mut u8) = color_idx; }
        }
    }
}

/// Draw a rectangle outline (not filled).
pub fn rect(x: u16, y: u16, w: u16, h: u16, color_idx: u8) {
    if w == 0 || h == 0 { return; }
    fill_rect(x, y, w, 1, color_idx);
    fill_rect(x, y + h.saturating_sub(1), w, 1, color_idx);
    fill_rect(x, y, 1, h, color_idx);
    fill_rect(x + w.saturating_sub(1), y, 1, h, color_idx);
}

/// Clear the entire framebuffer to a single color.
pub fn clear(color_idx: u8) {
    let w = width() as usize;
    let h = height() as usize;
    let total = w * h;
    if total == 0 { return; }
    unsafe {
        core::ptr::write_bytes(GFX_PIXEL_ADDR as *mut u8, color_idx, total);
    }
}

/// Set a single palette entry.
pub fn set_palette_entry(idx: u8, r: u8, g: u8, b: u8) {
    let off = GFX_PALETTE_ADDR + idx as usize * 3;
    unsafe {
        *(off as *mut u8) = r;
        *((off + 1) as *mut u8) = g;
        *((off + 2) as *mut u8) = b;
    }
}

/// Increment the pixel dirty counter.
pub fn mark_pixel_dirty() {
    unsafe {
        let v = read_u32(OFF_PIXEL_DIRTY);
        write_u32(OFF_PIXEL_DIRTY, v.wrapping_add(1));
    }
}

/// Increment the palette dirty counter.
pub fn mark_palette_dirty() {
    unsafe {
        let v = read_u32(OFF_PALETTE_DIRTY);
        write_u32(OFF_PALETTE_DIRTY, v.wrapping_add(1));
    }
}

// --- Default VGA 256-color palette (same as gfx.rs) ---

fn init_default_palette() {
    static ANSI: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00), (0xAA, 0x00, 0x00), (0x00, 0xAA, 0x00), (0xAA, 0x55, 0x00),
        (0x00, 0x00, 0xAA), (0xAA, 0x00, 0xAA), (0x00, 0xAA, 0xAA), (0xAA, 0xAA, 0xAA),
        (0x55, 0x55, 0x55), (0xFF, 0x55, 0x55), (0x55, 0xFF, 0x55), (0xFF, 0xFF, 0x55),
        (0x55, 0x55, 0xFF), (0xFF, 0x55, 0xFF), (0x55, 0xFF, 0xFF), (0xFF, 0xFF, 0xFF),
    ];
    for (i, &(r, g, b)) in ANSI.iter().enumerate() {
        set_palette_entry(i as u8, r, g, b);
    }
    for i in 0u8..216 {
        let r = (i / 36) * 51;
        let g = ((i / 6) % 6) * 51;
        let b = (i % 6) * 51;
        set_palette_entry(16 + i, r, g, b);
    }
    for i in 0u8..24 {
        let v = i * 10 + 8;
        set_palette_entry(232 + i, v, v, v);
    }
}

// --- Bitmap font text rendering (same 5x7 font used by gfx_test's label helper) ---

/// Draw a string using a 5x7 bitmap font, at pixel coordinates (x, y) in
/// `color_idx`. Supports uppercase A-Z, digits, and a few punctuation glyphs.
/// Each "on" bit is drawn as a `scale x scale` filled square.
pub fn draw_text(x: u16, y: u16, text: &str, color_idx: u8, scale: u16) {
    let s = scale.max(1);
    let mut cx = x;
    for ch in text.chars() {
        if let Some(glyph) = get_glyph(ch) {
            for (row, &bits) in glyph.iter().enumerate() {
                for col in 0..5u16 {
                    if bits & (1 << (4 - col)) != 0 {
                        fill_rect(cx + col * s, y + row as u16 * s, s, s, color_idx);
                    }
                }
            }
        }
        cx += 6 * s;
    }
}

fn get_glyph(ch: char) -> Option<[u8; 7]> {
    Some(match ch.to_ascii_uppercase() {
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111],
        '3' => [0b01110, 0b10001, 0b00001, 0b00110, 0b00001, 0b10001, 0b01110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b01110, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110],
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01110, 0b10001, 0b10000, 0b01110, 0b00001, 0b10001, 0b01110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        ' ' => [0; 7],
        '.' => [0, 0, 0, 0, 0, 0b00100, 0b00100],
        ':' => [0, 0b00100, 0, 0, 0, 0b00100, 0],
        '-' => [0, 0, 0, 0b11111, 0, 0, 0],
        '/' => [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000],
        '!' => [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0, 0b00100],
        _ => return None,
    })
}
