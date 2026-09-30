//! Rectangles and colours.

fn clamp_i32(v: i64) -> i32 {
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// Axis-aligned rectangle. A rect with `w <= 0` or `h <= 0` is empty; every
/// helper tolerates empty/negative sizes and never overflows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const EMPTY: Rect = Rect { x: 0, y: 0, w: 0, h: 0 };

    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// Exclusive right edge.
    pub fn right(&self) -> i32 {
        clamp_i32(self.x as i64 + self.w.max(0) as i64)
    }

    /// Exclusive bottom edge.
    pub fn bottom(&self) -> i32 {
        clamp_i32(self.y as i64 + self.h.max(0) as i64)
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        !self.is_empty()
            && x >= self.x
            && y >= self.y
            && (x as i64) < self.x as i64 + self.w as i64
            && (y as i64) < self.y as i64 + self.h as i64
    }

    /// Overlap of two rects (empty, with zero size, if they do not overlap).
    pub fn intersect(&self, o: Rect) -> Rect {
        if self.is_empty() || o.is_empty() {
            return Rect::new(self.x, self.y, 0, 0);
        }
        let x0 = self.x.max(o.x) as i64;
        let y0 = self.y.max(o.y) as i64;
        let x1 = (self.x as i64 + self.w as i64).min(o.x as i64 + o.w as i64);
        let y1 = (self.y as i64 + self.h as i64).min(o.y as i64 + o.h as i64);
        if x1 <= x0 || y1 <= y0 {
            return Rect::new(x0 as i32, y0 as i32, 0, 0);
        }
        Rect::new(x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32)
    }

    pub fn intersects(&self, o: Rect) -> bool {
        !self.intersect(o).is_empty()
    }

    /// Smallest rect covering both (empty inputs are ignored).
    pub fn union(&self, o: Rect) -> Rect {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = self.right().max(o.right());
        let y1 = self.bottom().max(o.bottom());
        Rect::new(x0, y0, clamp_i32(x1 as i64 - x0 as i64), clamp_i32(y1 as i64 - y0 as i64))
    }

    /// Shrink by `n` on every side (negative `n` grows). Size clamps at 0.
    pub fn inset(&self, n: i32) -> Rect {
        let n = n as i64;
        let w = (self.w as i64 - 2 * n).max(0);
        let h = (self.h as i64 - 2 * n).max(0);
        Rect::new(clamp_i32(self.x as i64 + n), clamp_i32(self.y as i64 + n), clamp_i32(w), clamp_i32(h))
    }

    /// Shrink by different amounts horizontally / vertically.
    pub fn inset_xy(&self, dx: i32, dy: i32) -> Rect {
        let w = (self.w as i64 - 2 * dx as i64).max(0);
        let h = (self.h as i64 - 2 * dy as i64).max(0);
        Rect::new(clamp_i32(self.x as i64 + dx as i64), clamp_i32(self.y as i64 + dy as i64), clamp_i32(w), clamp_i32(h))
    }

    pub fn translate(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x.saturating_add(dx), self.y.saturating_add(dy), self.w, self.h)
    }

    /// `(top strip of height h, remainder)`; `h` is clamped to `[0, self.h]`.
    pub fn split_top(&self, h: i32) -> (Rect, Rect) {
        let sh = self.h.max(0);
        let h = h.clamp(0, sh);
        (Rect::new(self.x, self.y, self.w, h), Rect::new(self.x, self.y.saturating_add(h), self.w, sh - h))
    }

    /// `(bottom strip of height h, remainder)`.
    pub fn split_bottom(&self, h: i32) -> (Rect, Rect) {
        let sh = self.h.max(0);
        let h = h.clamp(0, sh);
        (Rect::new(self.x, self.y.saturating_add(sh - h), self.w, h), Rect::new(self.x, self.y, self.w, sh - h))
    }

    /// `(left strip of width w, remainder)`.
    pub fn split_left(&self, w: i32) -> (Rect, Rect) {
        let sw = self.w.max(0);
        let w = w.clamp(0, sw);
        (Rect::new(self.x, self.y, w, self.h), Rect::new(self.x.saturating_add(w), self.y, sw - w, self.h))
    }

    /// `(right strip of width w, remainder)`.
    pub fn split_right(&self, w: i32) -> (Rect, Rect) {
        let sw = self.w.max(0);
        let w = w.clamp(0, sw);
        (Rect::new(self.x.saturating_add(sw - w), self.y, w, self.h), Rect::new(self.x, self.y, sw - w, self.h))
    }

    /// `n` horizontal bands of (nearly) equal height covering the rect exactly.
    pub fn rows(&self, n: usize) -> Vec<Rect> {
        let n = n.min(1 << 16);
        let h = self.h.max(0) as i64;
        (0..n)
            .map(|i| {
                let y0 = h * i as i64 / n as i64;
                let y1 = h * (i as i64 + 1) / n as i64;
                Rect::new(self.x, clamp_i32(self.y as i64 + y0), self.w, (y1 - y0) as i32)
            })
            .collect()
    }

    /// `n` vertical bands of (nearly) equal width covering the rect exactly.
    pub fn cols(&self, n: usize) -> Vec<Rect> {
        let n = n.min(1 << 16);
        let w = self.w.max(0) as i64;
        (0..n)
            .map(|i| {
                let x0 = w * i as i64 / n as i64;
                let x1 = w * (i as i64 + 1) / n as i64;
                Rect::new(clamp_i32(self.x as i64 + x0), self.y, (x1 - x0) as i32, self.h)
            })
            .collect()
    }

    /// A `w`x`h` rect centred in this one (may extend past it if larger).
    pub fn center(&self, w: i32, h: i32) -> Rect {
        let w = w.max(0);
        let h = h.max(0);
        Rect::new(
            clamp_i32(self.x as i64 + (self.w.max(0) as i64 - w as i64) / 2),
            clamp_i32(self.y as i64 + (self.h.max(0) as i64 - h as i64) / 2),
            w,
            h,
        )
    }

    /// Like [`Rect::center`] but the size is first clamped to fit inside.
    pub fn center_fit(&self, w: i32, h: i32) -> Rect {
        self.center(w.min(self.w.max(0)), h.min(self.h.max(0)))
    }
}

/// 8-bit RGBA colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    /// From `0xRRGGBB` (opaque).
    pub const fn hex(v: u32) -> Color {
        Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// Packed `0xRRGGBBAA`.
    pub const fn to_u32(self) -> u32 {
        (self.r as u32) << 24 | (self.g as u32) << 16 | (self.b as u32) << 8 | self.a as u32
    }

    /// From packed `0xRRGGBBAA`.
    pub const fn from_u32(v: u32) -> Color {
        Color::rgba((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    pub const fn with_alpha(self, a: u8) -> Color {
        Color { a, ..self }
    }

    /// Linear mix: `t = 0` gives `self`, `t = 255` gives `o`.
    pub fn mix(self, o: Color, t: u8) -> Color {
        let m = |a: u8, b: u8| ((a as u32 * (255 - t as u32) + b as u32 * t as u32 + 127) / 255) as u8;
        Color::rgba(m(self.r, o.r), m(self.g, o.g), m(self.b, o.b), m(self.a, o.a))
    }
}
