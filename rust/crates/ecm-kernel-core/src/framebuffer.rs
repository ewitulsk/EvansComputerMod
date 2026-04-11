//! Memory-mapped framebuffer for the WASM terminal display.
//!
//! Layout in WASM linear memory at `FB_BASE` (0x20000):
//!
//! ```text
//! Offset  Size  Field
//! 0x00    u16   magic (0xFB01)
//! 0x02    u16   width (columns)
//! 0x04    u16   height (rows)
//! 0x06    u16   cursor_x
//! 0x08    u16   cursor_y
//! 0x0A    u8    cursor_visible
//! 0x0B    u8    cursor_shape (0=block, 1=underline, 2=bar)
//! 0x0C    u32   dirty_counter
//! 0x10..0x3F    reserved
//! 0x40..        cell buffer (4 bytes per cell: char, attr, flags, reserved)
//! ```
//!
//! Attribute byte: low nibble = foreground color (0-15), high nibble = background color (0-15).
//! Flags byte: bit 0 = bold, bit 1 = underline, bit 2 = blink, bit 3 = inverse.

/// Base address of the framebuffer region in WASM linear memory.
pub const FB_BASE: usize = 0x20000;

/// Size of the control header in bytes.
pub const FB_HEADER_SIZE: usize = 64;

/// Start of the cell buffer (immediately after header).
pub const FB_CELL_BASE: usize = FB_BASE + FB_HEADER_SIZE;

/// Default framebuffer width in columns.
pub const DEFAULT_WIDTH: u16 = 160;

/// Default framebuffer height in rows.
pub const DEFAULT_HEIGHT: u16 = 50;

/// Bytes per cell.
pub const CELL_SIZE: usize = 4;

/// Magic number identifying a valid framebuffer header.
pub const FB_MAGIC: u16 = 0xFB01;

/// Default attribute: green (color 2) on black (color 0) = 0x02.
/// Using bright green (color 10 = 0x0A) for the classic terminal look.
pub const DEFAULT_ATTR: u8 = 0x0A;

// --- Header field offsets from FB_BASE ---
const OFF_MAGIC: usize = 0x00;
const OFF_WIDTH: usize = 0x02;
const OFF_HEIGHT: usize = 0x04;
const OFF_CURSOR_X: usize = 0x06;
const OFF_CURSOR_Y: usize = 0x08;
const OFF_CURSOR_VISIBLE: usize = 0x0A;
const OFF_CURSOR_SHAPE: usize = 0x0B;
const OFF_DIRTY_COUNTER: usize = 0x0C;

// --- Unsafe raw pointer helpers ---

#[inline(always)]
unsafe fn write_u16(offset: usize, val: u16) {
    let ptr = (FB_BASE + offset) as *mut u16;
    core::ptr::write_volatile(ptr, val);
}

#[inline(always)]
unsafe fn read_u16(offset: usize) -> u16 {
    let ptr = (FB_BASE + offset) as *const u16;
    core::ptr::read_volatile(ptr)
}

#[inline(always)]
unsafe fn write_u8(offset: usize, val: u8) {
    let ptr = (FB_BASE + offset) as *mut u8;
    core::ptr::write_volatile(ptr, val);
}

#[inline(always)]
unsafe fn write_u32(offset: usize, val: u32) {
    let ptr = (FB_BASE + offset) as *mut u32;
    core::ptr::write_volatile(ptr, val);
}

#[inline(always)]
unsafe fn read_u32(offset: usize) -> u32 {
    let ptr = (FB_BASE + offset) as *const u32;
    core::ptr::read_volatile(ptr)
}

/// Initialize the framebuffer: write header and clear all cells.
pub fn init() {
    unsafe {
        write_u16(OFF_MAGIC, FB_MAGIC);
        write_u16(OFF_WIDTH, DEFAULT_WIDTH);
        write_u16(OFF_HEIGHT, DEFAULT_HEIGHT);
        write_u16(OFF_CURSOR_X, 0);
        write_u16(OFF_CURSOR_Y, 0);
        write_u8(OFF_CURSOR_VISIBLE, 1);
        write_u8(OFF_CURSOR_SHAPE, 0); // block cursor
        write_u32(OFF_DIRTY_COUNTER, 0);
        // Zero out reserved bytes
        for i in 0x10..FB_HEADER_SIZE {
            *((FB_BASE + i) as *mut u8) = 0;
        }
    }
    clear(DEFAULT_ATTR);
}

/// Read the framebuffer width from the header.
pub fn width() -> u16 {
    unsafe { read_u16(OFF_WIDTH) }
}

/// Read the framebuffer height from the header.
pub fn height() -> u16 {
    unsafe { read_u16(OFF_HEIGHT) }
}

/// Compute the byte offset of a cell in the cell buffer.
#[inline(always)]
fn cell_offset(x: u16, y: u16) -> usize {
    FB_CELL_BASE + (y as usize * width() as usize + x as usize) * CELL_SIZE
}

/// Write a single cell at (x, y).
pub fn write_cell(x: u16, y: u16, ch: u8, attr: u8, flags: u8) {
    let w = width();
    let h = height();
    if x >= w || y >= h {
        return;
    }
    let off = cell_offset(x, y);
    unsafe {
        *(off as *mut u8) = ch;
        *((off + 1) as *mut u8) = attr;
        *((off + 2) as *mut u8) = flags;
        *((off + 3) as *mut u8) = 0; // reserved
    }
}

/// Read a single cell at (x, y). Returns (char, attr, flags).
pub fn read_cell(x: u16, y: u16) -> (u8, u8, u8) {
    let w = width();
    let h = height();
    if x >= w || y >= h {
        return (b' ', DEFAULT_ATTR, 0);
    }
    let off = cell_offset(x, y);
    unsafe {
        (
            *(off as *const u8),
            *((off + 1) as *const u8),
            *((off + 2) as *const u8),
        )
    }
}

/// Update cursor position and visibility in the header.
pub fn set_cursor(x: u16, y: u16, visible: bool) {
    unsafe {
        write_u16(OFF_CURSOR_X, x);
        write_u16(OFF_CURSOR_Y, y);
        write_u8(OFF_CURSOR_VISIBLE, if visible { 1 } else { 0 });
    }
}

/// Read cursor position from the header.
pub fn cursor() -> (u16, u16) {
    unsafe { (read_u16(OFF_CURSOR_X), read_u16(OFF_CURSOR_Y)) }
}

/// Read cursor visibility from the header.
pub fn cursor_visible() -> bool {
    unsafe { *((FB_BASE + OFF_CURSOR_VISIBLE) as *const u8) != 0 }
}

/// Increment the dirty counter to signal the host that the framebuffer changed.
pub fn mark_dirty() {
    unsafe {
        let val = read_u32(OFF_DIRTY_COUNTER);
        write_u32(OFF_DIRTY_COUNTER, val.wrapping_add(1));
    }
}

/// Clear the entire cell buffer: fill every cell with space + the given attribute.
pub fn clear(attr: u8) {
    let w = width() as usize;
    let h = height() as usize;
    let total = w * h;
    for i in 0..total {
        let off = FB_CELL_BASE + i * CELL_SIZE;
        unsafe {
            *(off as *mut u8) = b' ';
            *((off + 1) as *mut u8) = attr;
            *((off + 2) as *mut u8) = 0;
            *((off + 3) as *mut u8) = 0;
        }
    }
    mark_dirty();
}

/// Copy one row to another (for scrolling).
pub fn copy_row(src_y: u16, dst_y: u16) {
    let w = width() as usize;
    let src_off = FB_CELL_BASE + src_y as usize * w * CELL_SIZE;
    let dst_off = FB_CELL_BASE + dst_y as usize * w * CELL_SIZE;
    let row_bytes = w * CELL_SIZE;
    unsafe {
        core::ptr::copy(src_off as *const u8, dst_off as *mut u8, row_bytes);
    }
}

/// Clear a single row with the given attribute.
pub fn clear_row(y: u16, attr: u8) {
    let w = width();
    for x in 0..w {
        write_cell(x, y, b' ', attr, 0);
    }
}

/// Clear cells in a row from start_x to end_x (exclusive) with the given attribute.
pub fn clear_row_range(y: u16, start_x: u16, end_x: u16, attr: u8) {
    let end = end_x.min(width());
    for x in start_x..end {
        write_cell(x, y, b' ', attr, 0);
    }
}

/// Scroll the screen up by `n` rows. The bottom `n` rows are cleared with the given attribute.
/// Returns a Vec of the rows that scrolled off the top (each row is width*CELL_SIZE bytes).
pub fn scroll_up(n: u16, attr: u8) -> Vec<Vec<u8>> {
    let h = height();
    let w = width() as usize;
    let row_bytes = w * CELL_SIZE;

    // Save rows that will be lost
    let save_count = n.min(h) as usize;
    let mut saved = Vec::with_capacity(save_count);
    for i in 0..save_count {
        let off = FB_CELL_BASE + i * w * CELL_SIZE;
        let mut row = vec![0u8; row_bytes];
        unsafe {
            core::ptr::copy_nonoverlapping(off as *const u8, row.as_mut_ptr(), row_bytes);
        }
        saved.push(row);
    }

    // Shift rows up
    let n_clamped = n.min(h);
    for y in n_clamped..h {
        copy_row(y, y - n_clamped);
    }

    // Clear the bottom n rows
    for y in (h - n_clamped)..h {
        clear_row(y, attr);
    }

    mark_dirty();
    saved
}

/// Scroll the screen down by `n` rows. The top `n` rows are cleared with the given attribute.
pub fn scroll_down(n: u16, attr: u8) {
    let h = height();
    let n_clamped = n.min(h);

    // Shift rows down (iterate from bottom to avoid overwriting)
    for y in (0..h - n_clamped).rev() {
        copy_row(y, y + n_clamped);
    }

    // Clear the top n rows
    for y in 0..n_clamped {
        clear_row(y, attr);
    }

    mark_dirty();
}

/// Read an entire row as raw bytes (width * CELL_SIZE bytes).
pub fn read_row(y: u16) -> Vec<u8> {
    let w = width() as usize;
    let row_bytes = w * CELL_SIZE;
    let off = FB_CELL_BASE + y as usize * w * CELL_SIZE;
    let mut row = vec![0u8; row_bytes];
    unsafe {
        core::ptr::copy_nonoverlapping(off as *const u8, row.as_mut_ptr(), row_bytes);
    }
    row
}

/// Write an entire row from raw bytes.
pub fn write_row(y: u16, data: &[u8]) {
    let w = width() as usize;
    let row_bytes = w * CELL_SIZE;
    let off = FB_CELL_BASE + y as usize * w * CELL_SIZE;
    let len = data.len().min(row_bytes);
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), off as *mut u8, len);
    }
}
