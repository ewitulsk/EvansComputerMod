//! Text framebuffer shared with the host (region `hal::FB`).
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
//! 0x40..        cells (4 bytes each: char, attr, flags, reserved)
//! ```
//! Attribute byte: low nibble = fg colour, high nibble = bg colour.
//! Flags: bit 0 bold, bit 1 underline, bit 2 blink, bit 3 inverse.
//!
//! All access is bounds-checked against the region; out-of-range writes are
//! dropped rather than trusted.

use crate::hal;

pub const FB_HEADER_SIZE: usize = 64;
pub const DEFAULT_WIDTH: u16 = 160;
pub const DEFAULT_HEIGHT: u16 = 50;
pub const CELL_SIZE: usize = 4;
pub const FB_MAGIC: u16 = 0xFB01;
/// Bright green on black.
pub const DEFAULT_ATTR: u8 = 0x0A;

const OFF_MAGIC: usize = 0x00;
const OFF_WIDTH: usize = 0x02;
const OFF_HEIGHT: usize = 0x04;
const OFF_CURSOR_X: usize = 0x06;
const OFF_CURSOR_Y: usize = 0x08;
const OFF_CURSOR_VISIBLE: usize = 0x0A;
const OFF_CURSOR_SHAPE: usize = 0x0B;
const OFF_DIRTY: usize = 0x0C;

fn fb() -> &'static mut [u8] {
    hal::FB.bytes()
}

fn put_u16(off: usize, v: u16) {
    if let Some(s) = fb().get_mut(off..off + 2) {
        s.copy_from_slice(&v.to_le_bytes());
    }
}

fn get_u16(off: usize) -> u16 {
    fb().get(off..off + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).unwrap_or(0)
}

fn put_u32(off: usize, v: u32) {
    if let Some(s) = fb().get_mut(off..off + 4) {
        s.copy_from_slice(&v.to_le_bytes());
    }
}

fn get_u32(off: usize) -> u32 {
    fb().get(off..off + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0)
}

/// Write the header and clear every cell. Dimensions are clamped so the cell
/// grid always fits the region.
pub fn init(width: u16, height: u16) {
    let max_cells = (hal::FB_CAP - FB_HEADER_SIZE) / CELL_SIZE;
    let w = width.clamp(1, max_cells.min(u16::MAX as usize) as u16);
    let h = height.max(1).min((max_cells / w as usize).max(1) as u16);
    for b in fb()[..FB_HEADER_SIZE].iter_mut() {
        *b = 0;
    }
    put_u16(OFF_MAGIC, FB_MAGIC);
    put_u16(OFF_WIDTH, w);
    put_u16(OFF_HEIGHT, h);
    put_u16(OFF_CURSOR_X, 0);
    put_u16(OFF_CURSOR_Y, 0);
    fb()[OFF_CURSOR_VISIBLE] = 1;
    fb()[OFF_CURSOR_SHAPE] = 0;
    put_u32(OFF_DIRTY, 0);
    clear(DEFAULT_ATTR);
}

pub fn width() -> u16 {
    get_u16(OFF_WIDTH)
}

pub fn height() -> u16 {
    get_u16(OFF_HEIGHT)
}

fn cell_off(x: u16, y: u16) -> Option<usize> {
    let (w, h) = (width(), height());
    if x >= w || y >= h {
        return None;
    }
    let off = FB_HEADER_SIZE + (y as usize * w as usize + x as usize) * CELL_SIZE;
    if off + CELL_SIZE <= hal::FB_CAP { Some(off) } else { None }
}

pub fn write_cell(x: u16, y: u16, ch: u8, attr: u8, flags: u8) {
    if let Some(off) = cell_off(x, y) {
        fb()[off..off + CELL_SIZE].copy_from_slice(&[ch, attr, flags, 0]);
    }
}

pub fn read_cell(x: u16, y: u16) -> (u8, u8, u8) {
    match cell_off(x, y) {
        Some(off) => {
            let c = &fb()[off..off + 3];
            (c[0], c[1], c[2])
        }
        None => (b' ', DEFAULT_ATTR, 0),
    }
}

pub fn set_cursor(x: u16, y: u16, visible: bool) {
    put_u16(OFF_CURSOR_X, x);
    put_u16(OFF_CURSOR_Y, y);
    fb()[OFF_CURSOR_VISIBLE] = visible as u8;
}

pub fn mark_dirty() {
    put_u32(OFF_DIRTY, get_u32(OFF_DIRTY).wrapping_add(1));
}

fn row_range(y: u16) -> Option<core::ops::Range<usize>> {
    let w = width() as usize;
    let start = FB_HEADER_SIZE + y as usize * w * CELL_SIZE;
    let end = start + w * CELL_SIZE;
    if y < height() && end <= hal::FB_CAP { Some(start..end) } else { None }
}

pub fn clear(attr: u8) {
    for y in 0..height() {
        clear_row(y, attr);
    }
    mark_dirty();
}

pub fn clear_row(y: u16, attr: u8) {
    if let Some(r) = row_range(y) {
        for cell in fb()[r].chunks_exact_mut(CELL_SIZE) {
            cell.copy_from_slice(&[b' ', attr, 0, 0]);
        }
    }
}

pub fn copy_row(src_y: u16, dst_y: u16) {
    if let (Some(src), Some(dst)) = (row_range(src_y), row_range(dst_y)) {
        fb().copy_within(src, dst.start);
    }
}

pub fn read_row(y: u16) -> Vec<u8> {
    match row_range(y) {
        Some(r) => fb()[r].to_vec(),
        None => vec![0u8; width() as usize * CELL_SIZE],
    }
}

pub fn write_row(y: u16, data: &[u8]) {
    if let Some(r) = row_range(y) {
        let n = data.len().min(r.len());
        fb()[r.start..r.start + n].copy_from_slice(&data[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_and_cells_are_bounds_checked() {
        let _g = hal::test_lock();
        init(160, 50);
        assert_eq!(width(), 160);
        assert_eq!(height(), 50);
        write_cell(3, 4, b'X', 0x1F, 0);
        assert_eq!(read_cell(3, 4), (b'X', 0x1F, 0));
        // Out of range writes are ignored, reads return blanks.
        write_cell(500, 500, b'Y', 0, 0);
        assert_eq!(read_cell(500, 500).0, b' ');
        copy_row(4, 5);
        assert_eq!(read_cell(3, 5).0, b'X');
    }

    #[test]
    fn oversized_dimensions_are_clamped_to_region() {
        let _g = hal::test_lock();
        init(u16::MAX, u16::MAX);
        let cells = width() as usize * height() as usize;
        assert!(FB_HEADER_SIZE + cells * CELL_SIZE <= hal::FB_CAP);
        init(DEFAULT_WIDTH, DEFAULT_HEIGHT);
    }
}
