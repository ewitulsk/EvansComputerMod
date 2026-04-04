//! Memory-mapped graphics framebuffer for pixel-based rendering.
//!
//! Provides indexed-color pixel graphics at two resolutions:
//! - 320×200 (64 KB pixel data)
//! - 640×400 (256 KB pixel data)
//!
//! Layout in WASM linear memory at `GFX_BASE` (0x30000):
//!
//! ```text
//! Offset  Size    Field
//! 0x00    u16     magic (0xFB02)
//! 0x02    u8      mode (0=text-only, 1=gfx-only, 2=overlay)
//! 0x03    u8      reserved
//! 0x04    u16     width (pixels)
//! 0x06    u16     height (pixels)
//! 0x08    u32     palette_dirty counter
//! 0x0C    u32     pixel_dirty counter
//! 0x10..0x3F      reserved (zeros)
//! 0x40    768B    palette (256 × 3 bytes RGB)
//! 0x340..0x3FF    padding
//! 0x400..         pixel data (width × height bytes, 8-bit indexed color)
//! ```
//!
//! Each pixel is a single byte indexing into the 256-entry RGB palette.

/// Base address of the graphics framebuffer in WASM linear memory.
pub const GFX_BASE: usize = 0x30000;

/// Size of the graphics header in bytes.
pub const GFX_HEADER_SIZE: usize = 64;

/// Offset of palette data from GFX_BASE.
pub const GFX_PALETTE_OFF: usize = 0x40;

/// Offset of pixel data from GFX_BASE.
pub const GFX_PIXEL_OFF: usize = 0x400;

/// Absolute address of the palette.
pub const GFX_PALETTE_ADDR: usize = GFX_BASE + GFX_PALETTE_OFF;

/// Absolute address of pixel data.
pub const GFX_PIXEL_ADDR: usize = GFX_BASE + GFX_PIXEL_OFF;

/// Magic number identifying a valid graphics framebuffer header.
pub const GFX_MAGIC: u16 = 0xFB02;

// Header field offsets from GFX_BASE
const OFF_MAGIC: usize = 0x00;
const OFF_MODE: usize = 0x02;
const OFF_WIDTH: usize = 0x04;
const OFF_HEIGHT: usize = 0x06;
const OFF_PALETTE_DIRTY: usize = 0x08;
const OFF_PIXEL_DIRTY: usize = 0x0C;

// --- Unsafe raw pointer helpers (same pattern as framebuffer.rs) ---

#[inline(always)]
unsafe fn write_u16(offset: usize, val: u16) {
    let ptr = (GFX_BASE + offset) as *mut u16;
    core::ptr::write_volatile(ptr, val);
}

#[inline(always)]
unsafe fn read_u16(offset: usize) -> u16 {
    let ptr = (GFX_BASE + offset) as *const u16;
    core::ptr::read_volatile(ptr)
}

#[inline(always)]
unsafe fn write_u8(offset: usize, val: u8) {
    let ptr = (GFX_BASE + offset) as *mut u8;
    core::ptr::write_volatile(ptr, val);
}

#[inline(always)]
unsafe fn read_u8(offset: usize) -> u8 {
    let ptr = (GFX_BASE + offset) as *const u8;
    core::ptr::read_volatile(ptr)
}

#[inline(always)]
unsafe fn write_u32(offset: usize, val: u32) {
    let ptr = (GFX_BASE + offset) as *mut u32;
    core::ptr::write_volatile(ptr, val);
}

#[inline(always)]
unsafe fn read_u32(offset: usize) -> u32 {
    let ptr = (GFX_BASE + offset) as *const u32;
    core::ptr::read_volatile(ptr)
}

/// Initialize the graphics framebuffer with the given resolution.
///
/// Sets the mode to graphics-only (1), writes the default VGA 256-color palette,
/// and clears all pixels to color index 0 (black).
pub fn init(width: u16, height: u16) {
    unsafe {
        // Write header
        write_u16(OFF_MAGIC, GFX_MAGIC);
        write_u8(OFF_MODE, 1); // graphics-only by default
        write_u8(OFF_MODE + 1, 0); // reserved
        write_u16(OFF_WIDTH, width);
        write_u16(OFF_HEIGHT, height);
        write_u32(OFF_PALETTE_DIRTY, 0);
        write_u32(OFF_PIXEL_DIRTY, 0);

        // Zero reserved bytes
        for i in 0x10..GFX_HEADER_SIZE {
            *((GFX_BASE + i) as *mut u8) = 0;
        }
    }

    // Initialize default VGA 256-color palette
    init_default_palette();

    // Clear pixel data to index 0
    clear(0);
}

/// Set the display mode.
///
/// - 0 = text-only (graphics framebuffer ignored)
/// - 1 = graphics-only (text framebuffer ignored)
/// - 2 = overlay (graphics rendered first, then text on top with transparent bg)
pub fn set_mode(mode: u8) {
    unsafe { write_u8(OFF_MODE, mode); }
}

/// Get the current display mode.
pub fn get_mode() -> u8 {
    unsafe { read_u8(OFF_MODE) }
}

/// Get the graphics framebuffer width in pixels.
pub fn width() -> u16 {
    unsafe { read_u16(OFF_WIDTH) }
}

/// Get the graphics framebuffer height in pixels.
pub fn height() -> u16 {
    unsafe { read_u16(OFF_HEIGHT) }
}

/// Compute the byte offset of a pixel in the pixel data buffer.
#[inline(always)]
fn pixel_offset(x: u16, y: u16) -> usize {
    GFX_PIXEL_ADDR + (y as usize * width() as usize + x as usize)
}

/// Set a single pixel to the given palette color index.
pub fn set_pixel(x: u16, y: u16, color_idx: u8) {
    let w = width();
    let h = height();
    if x >= w || y >= h {
        return;
    }
    let off = pixel_offset(x, y);
    unsafe {
        *(off as *mut u8) = color_idx;
    }
}

/// Get the palette color index at a pixel.
pub fn get_pixel(x: u16, y: u16) -> u8 {
    let w = width();
    let h = height();
    if x >= w || y >= h {
        return 0;
    }
    let off = pixel_offset(x, y);
    unsafe { *(off as *const u8) }
}

/// Fill a rectangle with the given palette color index.
pub fn fill_rect(x: u16, y: u16, w: u16, h: u16, color_idx: u8) {
    let fb_w = width();
    let fb_h = height();

    // Clamp to framebuffer bounds
    let x_end = (x + w).min(fb_w);
    let y_end = (y + h).min(fb_h);
    let x_start = x.min(fb_w);
    let y_start = y.min(fb_h);

    for py in y_start..y_end {
        let row_base = GFX_PIXEL_ADDR + py as usize * fb_w as usize;
        for px in x_start..x_end {
            unsafe {
                *((row_base + px as usize) as *mut u8) = color_idx;
            }
        }
    }
}

/// Bulk copy pixel data from a source slice into the framebuffer at position (x, y).
///
/// The source data is interpreted as `w` pixels per row, `h` rows total,
/// laid out row-major.
pub fn blit(x: u16, y: u16, w: u16, h: u16, src: &[u8]) {
    let fb_w = width();
    let fb_h = height();

    for row in 0..h {
        let dst_y = y + row;
        if dst_y >= fb_h {
            break;
        }
        let src_off = row as usize * w as usize;
        let dst_base = GFX_PIXEL_ADDR + dst_y as usize * fb_w as usize;

        for col in 0..w {
            let dst_x = x + col;
            if dst_x >= fb_w {
                break;
            }
            let src_idx = src_off + col as usize;
            if src_idx >= src.len() {
                return;
            }
            unsafe {
                *((dst_base + dst_x as usize) as *mut u8) = src[src_idx];
            }
        }
    }
}

/// Set a single palette entry (index 0-255) to the given RGB color.
pub fn set_palette_entry(idx: u8, r: u8, g: u8, b: u8) {
    let off = GFX_PALETTE_ADDR + idx as usize * 3;
    unsafe {
        *(off as *mut u8) = r;
        *((off + 1) as *mut u8) = g;
        *((off + 2) as *mut u8) = b;
    }
}

/// Get a palette entry as (R, G, B).
pub fn get_palette_entry(idx: u8) -> (u8, u8, u8) {
    let off = GFX_PALETTE_ADDR + idx as usize * 3;
    unsafe {
        (
            *(off as *const u8),
            *((off + 1) as *const u8),
            *((off + 2) as *const u8),
        )
    }
}

/// Set the full 256-entry palette from a 768-byte RGB slice.
pub fn set_palette(data: &[u8]) {
    let len = data.len().min(768);
    unsafe {
        core::ptr::copy_nonoverlapping(
            data.as_ptr(),
            GFX_PALETTE_ADDR as *mut u8,
            len,
        );
    }
}

/// Clear all pixels to the given color index.
pub fn clear(color_idx: u8) {
    let w = width() as usize;
    let h = height() as usize;
    let total = w * h;
    unsafe {
        core::ptr::write_bytes(GFX_PIXEL_ADDR as *mut u8, color_idx, total);
    }
}

/// Increment the pixel dirty counter to signal the host that pixel data changed.
pub fn mark_pixel_dirty() {
    unsafe {
        let val = read_u32(OFF_PIXEL_DIRTY);
        write_u32(OFF_PIXEL_DIRTY, val.wrapping_add(1));
    }
}

/// Increment the palette dirty counter to signal the host that the palette changed.
pub fn mark_palette_dirty() {
    unsafe {
        let val = read_u32(OFF_PALETTE_DIRTY);
        write_u32(OFF_PALETTE_DIRTY, val.wrapping_add(1));
    }
}

/// Draw a horizontal line.
pub fn hline(x: u16, y: u16, length: u16, color_idx: u8) {
    fill_rect(x, y, length, 1, color_idx);
}

/// Draw a vertical line.
pub fn vline(x: u16, y: u16, length: u16, color_idx: u8) {
    fill_rect(x, y, 1, length, color_idx);
}

/// Draw a rectangle outline (not filled).
pub fn rect(x: u16, y: u16, w: u16, h: u16, color_idx: u8) {
    if w == 0 || h == 0 {
        return;
    }
    hline(x, y, w, color_idx);                 // top
    hline(x, y + h - 1, w, color_idx);         // bottom
    vline(x, y, h, color_idx);                 // left
    vline(x + w - 1, y, h, color_idx);         // right
}

// ---------------------------------------------------------------------------
// Default VGA 256-color palette
// ---------------------------------------------------------------------------

/// Initialize the standard VGA 256-color palette:
/// - Indices 0-15: ANSI colors (matching the text framebuffer palette)
/// - Indices 16-231: 6×6×6 color cube
/// - Indices 232-255: 24-step grayscale ramp
fn init_default_palette() {
    // ANSI 16 colors (must match framebuffer.rs / terminal_io.rs PALETTE)
    static ANSI: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00), // 0  Black
        (0xAA, 0x00, 0x00), // 1  Red
        (0x00, 0xAA, 0x00), // 2  Green
        (0xAA, 0x55, 0x00), // 3  Yellow/Brown
        (0x00, 0x00, 0xAA), // 4  Blue
        (0xAA, 0x00, 0xAA), // 5  Magenta
        (0x00, 0xAA, 0xAA), // 6  Cyan
        (0xAA, 0xAA, 0xAA), // 7  Light Gray
        (0x55, 0x55, 0x55), // 8  Dark Gray
        (0xFF, 0x55, 0x55), // 9  Light Red
        (0x55, 0xFF, 0x55), // 10 Light Green
        (0xFF, 0xFF, 0x55), // 11 Yellow
        (0x55, 0x55, 0xFF), // 12 Light Blue
        (0xFF, 0x55, 0xFF), // 13 Light Magenta
        (0x55, 0xFF, 0xFF), // 14 Light Cyan
        (0xFF, 0xFF, 0xFF), // 15 White
    ];

    for (i, &(r, g, b)) in ANSI.iter().enumerate() {
        set_palette_entry(i as u8, r, g, b);
    }

    // 6×6×6 color cube (indices 16-231)
    for i in 0u8..216 {
        let r = (i / 36) * 51;
        let g = ((i / 6) % 6) * 51;
        let b = (i % 6) * 51;
        set_palette_entry(16 + i, r, g, b);
    }

    // 24-step grayscale ramp (indices 232-255)
    for i in 0u8..24 {
        let v = i * 10 + 8;
        set_palette_entry(232 + i, v, v, v);
    }
}

/// Look up the default VGA palette color for a given index.
/// Useful for palette animation (to get the "original" color for an index).
pub fn default_vga_color(idx: u8) -> (u8, u8, u8) {
    static ANSI: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00), (0xAA, 0x00, 0x00), (0x00, 0xAA, 0x00), (0xAA, 0x55, 0x00),
        (0x00, 0x00, 0xAA), (0xAA, 0x00, 0xAA), (0x00, 0xAA, 0xAA), (0xAA, 0xAA, 0xAA),
        (0x55, 0x55, 0x55), (0xFF, 0x55, 0x55), (0x55, 0xFF, 0x55), (0xFF, 0xFF, 0x55),
        (0x55, 0x55, 0xFF), (0xFF, 0x55, 0xFF), (0x55, 0xFF, 0xFF), (0xFF, 0xFF, 0xFF),
    ];

    if idx < 16 {
        ANSI[idx as usize]
    } else if idx < 232 {
        let i = idx - 16;
        let r = (i / 36) * 51;
        let g = ((i / 6) % 6) * 51;
        let b = (i % 6) * 51;
        (r, g, b)
    } else {
        let v = (idx - 232) * 10 + 8;
        (v, v, v)
    }
}
