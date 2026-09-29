//! Pixel graphics planes shared with the host.
//!
//! Two planes use the same header layout:
//! - the terminal gfx plane (region `hal::GFX`, indexed colour only), drawn
//!   under/over the text framebuffer depending on `mode`;
//! - the in-world Screen cluster plane (region `hal::SCREEN`), indexed or
//!   RGBA8888, dimensions set by the host when a cluster attaches.
//!
//! ```text
//! Offset  Size  Field
//! 0x00    u16   magic (0xFB02)
//! 0x02    u8    mode (terminal: 0 text, 1 gfx, 2 overlay; screen: 0 detached, 1 attached)
//! 0x04    u16   width  (pixels)
//! 0x06    u16   height (pixels)
//! 0x08    u32   palette dirty counter
//! 0x0C    u32   pixel dirty counter
//! 0x10    u8    pixel format (0 INDEXED8, 1 RGBA8888)
//! 0x40    768B  palette (256 × RGB)
//! 0x400         pixels
//! ```
//! Every access is bounds-checked against the plane's region.

use crate::hal;

pub const GFX_MAGIC: u16 = 0xFB02;
pub const PALETTE_OFF: usize = 0x40;
pub const PIXEL_OFF: usize = 0x400;
pub const PIXEL_FORMAT_INDEXED8: u8 = 0;
pub const PIXEL_FORMAT_RGBA8888: u8 = 1;

const OFF_MAGIC: usize = 0x00;
const OFF_MODE: usize = 0x02;
const OFF_WIDTH: usize = 0x04;
const OFF_HEIGHT: usize = 0x06;
const OFF_PAL_DIRTY: usize = 0x08;
const OFF_PIX_DIRTY: usize = 0x0C;
const OFF_FORMAT: usize = 0x10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Plane {
    Terminal,
    Screen,
}

impl Plane {
    fn bytes(self) -> &'static mut [u8] {
        match self {
            Plane::Terminal => hal::GFX.bytes(),
            Plane::Screen => hal::SCREEN.bytes(),
        }
    }

    fn put_u16(self, off: usize, v: u16) {
        if let Some(s) = self.bytes().get_mut(off..off + 2) {
            s.copy_from_slice(&v.to_le_bytes());
        }
    }
    fn get_u16(self, off: usize) -> u16 {
        self.bytes().get(off..off + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).unwrap_or(0)
    }
    fn bump_u32(self, off: usize) {
        if let Some(s) = self.bytes().get_mut(off..off + 4) {
            let v = u32::from_le_bytes([s[0], s[1], s[2], s[3]]).wrapping_add(1);
            s.copy_from_slice(&v.to_le_bytes());
        }
    }

    fn bpp(self) -> usize {
        if self.pixel_format() == PIXEL_FORMAT_RGBA8888 { 4 } else { 1 }
    }

    /// Largest (w, h) of the given aspect that fits the region at `bpp`.
    fn fits(self, w: u16, h: u16, bpp: usize) -> bool {
        PIXEL_OFF + w as usize * h as usize * bpp <= self.bytes().len()
    }

    /// Write the header, default palette and clear pixels (INDEXED8).
    /// Returns false if the requested size doesn't fit the region.
    pub fn init(self, width: u16, height: u16, mode: u8) -> bool {
        if width == 0 || height == 0 || !self.fits(width, height, 1) {
            return false;
        }
        for b in self.bytes()[..PIXEL_OFF].iter_mut() {
            *b = 0;
        }
        self.put_u16(OFF_MAGIC, GFX_MAGIC);
        self.bytes()[OFF_MODE] = mode;
        self.put_u16(OFF_WIDTH, width);
        self.put_u16(OFF_HEIGHT, height);
        self.bytes()[OFF_FORMAT] = PIXEL_FORMAT_INDEXED8;
        for i in 0..=255u8 {
            let (r, g, b) = default_vga_color(i);
            self.set_palette_entry(i, r, g, b);
        }
        self.clear(0);
        self.mark_palette_dirty();
        self.mark_pixel_dirty();
        true
    }

    pub fn set_mode(self, mode: u8) {
        self.bytes()[OFF_MODE] = mode;
    }
    pub fn mode(self) -> u8 {
        self.bytes()[OFF_MODE]
    }
    pub fn width(self) -> u16 {
        self.get_u16(OFF_WIDTH)
    }
    pub fn height(self) -> u16 {
        self.get_u16(OFF_HEIGHT)
    }
    pub fn pixel_format(self) -> u8 {
        self.bytes()[OFF_FORMAT]
    }

    fn pixels(self) -> Option<&'static mut [u8]> {
        let (w, h) = (self.width() as usize, self.height() as usize);
        let end = PIXEL_OFF + w * h * self.bpp();
        self.bytes().get_mut(PIXEL_OFF..end)
    }

    pub fn clear(self, color: u8) {
        if self.bpp() == 1 {
            if let Some(px) = self.pixels() {
                px.fill(color);
            }
        } else {
            let (r, g, b) = default_vga_color(color);
            if let Some(px) = self.pixels() {
                for p in px.chunks_exact_mut(4) {
                    p.copy_from_slice(&[r, g, b, 255]);
                }
            }
        }
    }

    pub fn fill_rect(self, x: u16, y: u16, w: u16, h: u16, color: u8) {
        let (fw, fh) = (self.width() as u32, self.height() as u32);
        let (x0, y0) = ((x as u32).min(fw), (y as u32).min(fh));
        let (x1, y1) = ((x as u32 + w as u32).min(fw), (y as u32 + h as u32).min(fh));
        let bpp = self.bpp();
        let rgba = default_vga_color(color);
        let Some(px) = self.pixels() else { return };
        for py in y0..y1 {
            for pxl in x0..x1 {
                let off = (py * fw + pxl) as usize * bpp;
                if bpp == 1 {
                    if let Some(p) = px.get_mut(off) {
                        *p = color;
                    }
                } else if let Some(p) = px.get_mut(off..off + 4) {
                    p.copy_from_slice(&[rgba.0, rgba.1, rgba.2, 255]);
                }
            }
        }
    }

    pub fn rect(self, x: u16, y: u16, w: u16, h: u16, color: u8) {
        if w == 0 || h == 0 {
            return;
        }
        self.fill_rect(x, y, w, 1, color);
        self.fill_rect(x, y.saturating_add(h - 1), w, 1, color);
        self.fill_rect(x, y, 1, h, color);
        self.fill_rect(x.saturating_add(w - 1), y, 1, h, color);
    }

    pub fn set_palette_entry(self, idx: u8, r: u8, g: u8, b: u8) {
        let off = PALETTE_OFF + idx as usize * 3;
        if let Some(s) = self.bytes().get_mut(off..off + 3) {
            s.copy_from_slice(&[r, g, b]);
        }
    }

    pub fn mark_pixel_dirty(self) {
        self.bump_u32(OFF_PIX_DIRTY);
    }
    pub fn mark_palette_dirty(self) {
        self.bump_u32(OFF_PAL_DIRTY);
    }

    /// 5×7 bitmap text, each lit bit drawn as a `scale`² block.
    pub fn draw_text(self, x: u16, y: u16, text: &str, color: u8, scale: u16) {
        let s = scale.max(1);
        let mut cx = x;
        for ch in text.chars() {
            if let Some(glyph) = glyph(ch) {
                for (row, bits) in glyph.iter().enumerate() {
                    for col in 0..5u16 {
                        if bits & (1 << (4 - col)) != 0 {
                            self.fill_rect(cx.saturating_add(col * s), y.saturating_add(row as u16 * s), s, s, color);
                        }
                    }
                }
            }
            cx = cx.saturating_add(6 * s);
        }
    }

    /// Hand the display back to the text terminal / power the screen down.
    pub fn reset(self) {
        match self {
            Plane::Terminal => {
                self.clear(0);
                self.set_mode(0);
                self.mark_pixel_dirty();
            }
            Plane::Screen => {
                if hal::screen::attached() {
                    hal::screen::set_pixel_format(PIXEL_FORMAT_INDEXED8);
                    self.set_mode(0);
                    self.mark_pixel_dirty();
                    hal::screen::set_power(false);
                }
            }
        }
    }

    pub fn sync(self) {
        match self {
            Plane::Terminal => hal::fb_sync(),
            Plane::Screen => hal::screen::sync(),
        }
    }
}

/// Standard VGA 256 palette: 16 ANSI, 6×6×6 cube, 24 greys.
pub fn default_vga_color(idx: u8) -> (u8, u8, u8) {
    const ANSI: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00), (0xAA, 0x00, 0x00), (0x00, 0xAA, 0x00), (0xAA, 0x55, 0x00),
        (0x00, 0x00, 0xAA), (0xAA, 0x00, 0xAA), (0x00, 0xAA, 0xAA), (0xAA, 0xAA, 0xAA),
        (0x55, 0x55, 0x55), (0xFF, 0x55, 0x55), (0x55, 0xFF, 0x55), (0xFF, 0xFF, 0x55),
        (0x55, 0x55, 0xFF), (0xFF, 0x55, 0xFF), (0x55, 0xFF, 0xFF), (0xFF, 0xFF, 0xFF),
    ];
    if idx < 16 {
        ANSI[idx as usize]
    } else if idx < 232 {
        let i = idx - 16;
        ((i / 36) * 51, ((i / 6) % 6) * 51, (i % 6) * 51)
    } else {
        let v = (idx - 232) * 10 + 8;
        (v, v, v)
    }
}

fn glyph(ch: char) -> Option<[u8; 7]> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_rejects_oversize_and_draws_in_bounds() {
        let _g = hal::test_lock();
        assert!(!Plane::Terminal.init(4096, 4096, 1));
        assert!(Plane::Terminal.init(320, 200, 1));
        Plane::Terminal.fill_rect(310, 190, 100, 100, 7); // clipped, no panic
        Plane::Terminal.draw_text(u16::MAX - 3, u16::MAX - 3, "HI", 1, 9);
        assert_eq!(Plane::Terminal.width(), 320);
        assert!(Plane::Screen.init(640, 360, 1));
    }
}
