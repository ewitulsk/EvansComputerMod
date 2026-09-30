//! RGBA8888 software canvas: primitives, clipping, text, dirty tracking.

use crate::font::{BOX, FONT, GLYPH_ROWS, GLYPH_W};
use crate::geom::{Color, Rect};

/// Largest accepted canvas edge (keeps allocations bounded).
pub const MAX_CANVAS_EDGE: i32 = 4096;
/// Cap on rects returned by [`Canvas::take_dirty_rects`] before it falls back
/// to a single bounding rect.
pub const MAX_DIRTY_RECTS: usize = 64;

/// Horizontal advance of one character cell at scale 1 (5px glyph + 1px gap).
pub const CHAR_ADVANCE: i32 = 6;
/// Full glyph box height at scale 1 (7 cap rows + 2 descender rows).
pub const GLYPH_HEIGHT: i32 = GLYPH_ROWS as i32;
/// Cap height at scale 1 (rows above the baseline).
pub const CAP_HEIGHT: i32 = 7;
/// Recommended line pitch at scale 1.
pub const LINE_HEIGHT: i32 = 10;

fn sc(scale: i32) -> i32 {
    scale.clamp(1, 64)
}

/// Width in pixels of `s` drawn at `scale` (ink extent: no trailing gap).
pub fn text_width(s: &str, scale: i32) -> i32 {
    let n = s.chars().count() as i64;
    if n == 0 {
        return 0;
    }
    let s = sc(scale) as i64;
    (n * CHAR_ADVANCE as i64 * s - s).min(i32::MAX as i64) as i32
}

/// Glyph box height (including descenders) at `scale`.
pub fn text_height(scale: i32) -> i32 {
    GLYPH_HEIGHT * sc(scale)
}

/// Truncate `s` so it fits in `max_w` pixels, appending ".." when cut.
/// Returns an empty string if not even ".." fits.
pub fn fit_text(s: &str, max_w: i32, scale: i32) -> String {
    if text_width(s, scale) <= max_w {
        return s.to_string();
    }
    let adv = CHAR_ADVANCE * sc(scale);
    let dots = text_width("..", scale);
    if dots > max_w {
        return String::new();
    }
    // Width of k chars followed by "..": (k + 2) * adv - scale.
    let k = ((max_w as i64 + sc(scale) as i64) / adv as i64 - 2).max(0) as usize;
    let mut out: String = s.chars().take(k).collect();
    out.push_str("..");
    out
}

fn glyph(ch: char) -> &'static [u8; 9] {
    let c = ch as u32;
    if (0x20..0x7F).contains(&c) {
        &FONT[(c - 0x20) as usize]
    } else {
        &BOX
    }
}

/// An RGBA8888 framebuffer (row-major, 4 bytes/pixel in R,G,B,A order).
#[derive(Clone, Debug)]
pub struct Canvas {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
    clip: Rect,
    clip_stack: Vec<Rect>,
    shadow: Vec<u8>,
    shadow_valid: bool,
}

impl Canvas {
    pub fn new(width: i32, height: i32) -> Canvas {
        let w = width.clamp(0, MAX_CANVAS_EDGE);
        let h = height.clamp(0, MAX_CANVAS_EDGE);
        Canvas {
            width: w,
            height: h,
            pixels: vec![0; (w * h * 4) as usize],
            clip: Rect::new(0, 0, w, h),
            clip_stack: Vec::new(),
            shadow: Vec::new(),
            shadow_valid: false,
        }
    }

    /// Resize (contents cleared to transparent black); the next
    /// [`take_dirty_rects`](Self::take_dirty_rects) reports the whole canvas.
    pub fn resize(&mut self, width: i32, height: i32) {
        let w = width.clamp(0, MAX_CANVAS_EDGE);
        let h = height.clamp(0, MAX_CANVAS_EDGE);
        if w == self.width && h == self.height {
            return;
        }
        *self = Canvas::new(w, h);
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    // ---- clipping -------------------------------------------------------

    /// Current clip rect (always within the canvas bounds).
    pub fn clip(&self) -> Rect {
        self.clip
    }

    /// Intersect the clip with `r` until the matching [`pop_clip`](Self::pop_clip).
    pub fn push_clip(&mut self, r: Rect) {
        self.clip_stack.push(self.clip);
        self.clip = self.clip.intersect(r);
    }

    pub fn pop_clip(&mut self) {
        if let Some(c) = self.clip_stack.pop() {
            self.clip = c;
        }
    }

    /// Drop all clips (back to full canvas).
    pub fn reset_clip(&mut self) {
        self.clip_stack.clear();
        self.clip = self.bounds();
    }

    // ---- pixels ---------------------------------------------------------

    pub fn get_pixel(&self, x: i32, y: i32) -> Color {
        if !self.bounds().contains(x, y) {
            return Color::TRANSPARENT;
        }
        let i = ((y * self.width + x) * 4) as usize;
        Color::rgba(self.pixels[i], self.pixels[i + 1], self.pixels[i + 2], self.pixels[i + 3])
    }

    /// Write a pixel (respecting clip), blending when `c.a < 255`.
    pub fn set_pixel(&mut self, x: i32, y: i32, c: Color) {
        self.fill_rect(Rect::new(x, y, 1, 1), c);
    }

    /// Alpha-blend `c` onto one pixel (respecting clip).
    pub fn blend(&mut self, x: i32, y: i32, c: Color) {
        self.fill_rect(Rect::new(x, y, 1, 1), c);
    }

    pub fn clear(&mut self, c: Color) {
        for px in self.pixels.chunks_exact_mut(4) {
            px.copy_from_slice(&[c.r, c.g, c.b, c.a]);
        }
    }

    /// Fill `r` (clipped). Opaque colours overwrite; translucent ones blend
    /// ("src over"); fully transparent colours are a no-op.
    pub fn fill_rect(&mut self, r: Rect, c: Color) {
        if c.a == 0 {
            return;
        }
        let a = self.clip.intersect(r);
        if a.is_empty() {
            return;
        }
        let stride = self.width as usize * 4;
        for y in a.y..a.bottom() {
            let row = y as usize * stride;
            let s = row + a.x as usize * 4;
            let e = row + a.right() as usize * 4;
            let span = &mut self.pixels[s..e];
            if c.a == 255 {
                for px in span.chunks_exact_mut(4) {
                    px.copy_from_slice(&[c.r, c.g, c.b, 255]);
                }
            } else {
                let sa = c.a as u32;
                let ia = 255 - sa;
                for px in span.chunks_exact_mut(4) {
                    px[0] = ((c.r as u32 * sa + px[0] as u32 * ia + 127) / 255) as u8;
                    px[1] = ((c.g as u32 * sa + px[1] as u32 * ia + 127) / 255) as u8;
                    px[2] = ((c.b as u32 * sa + px[2] as u32 * ia + 127) / 255) as u8;
                    px[3] = (sa + (px[3] as u32 * ia + 127) / 255).min(255) as u8;
                }
            }
        }
    }

    pub fn hline(&mut self, x: i32, y: i32, w: i32, c: Color) {
        self.fill_rect(Rect::new(x, y, w, 1), c);
    }

    pub fn vline(&mut self, x: i32, y: i32, h: i32, c: Color) {
        self.fill_rect(Rect::new(x, y, 1, h), c);
    }

    /// 1px outline just inside `r`.
    pub fn stroke_rect(&mut self, r: Rect, c: Color) {
        self.stroke_rect_w(r, c, 1);
    }

    /// Outline of thickness `t` just inside `r`.
    pub fn stroke_rect_w(&mut self, r: Rect, c: Color, t: i32) {
        if r.is_empty() || t <= 0 {
            return;
        }
        let t = t.min(r.w).min(r.h);
        if t * 2 >= r.w || t * 2 >= r.h {
            self.fill_rect(r, c);
            return;
        }
        self.fill_rect(Rect::new(r.x, r.y, r.w, t), c);
        self.fill_rect(Rect::new(r.x, r.bottom() - t, r.w, t), c);
        self.fill_rect(Rect::new(r.x, r.y + t, t, r.h - 2 * t), c);
        self.fill_rect(Rect::new(r.right() - t, r.y + t, t, r.h - 2 * t), c);
    }

    // ---- text -----------------------------------------------------------

    pub fn text_width(&self, s: &str, scale: i32) -> i32 {
        text_width(s, scale)
    }

    pub fn text_height(&self, scale: i32) -> i32 {
        text_height(scale)
    }

    pub fn fit_text(&self, s: &str, max_w: i32, scale: i32) -> String {
        fit_text(s, max_w, scale)
    }

    /// Draw `s` with its top-left glyph-box corner at `(x, y)`. Returns the x
    /// just past the last advance. Characters outside 0x20..=0x7E draw as a box.
    pub fn draw_text(&mut self, x: i32, y: i32, s: &str, c: Color, scale: i32) -> i32 {
        let s_ = sc(scale);
        let adv = CHAR_ADVANCE * s_;
        let clip = self.clip;
        let mut cx = x as i64;
        let visible_rows = !(clip.is_empty() || (y as i64 + (GLYPH_HEIGHT * s_) as i64) <= clip.y as i64 || y >= clip.bottom());
        for ch in s.chars() {
            if visible_rows && cx < clip.right() as i64 && cx + (GLYPH_W * s_) as i64 > clip.x as i64 && ch != ' ' {
                let g = glyph(ch);
                let gx = cx as i32;
                for (row, bits) in g.iter().enumerate() {
                    if *bits == 0 {
                        continue;
                    }
                    let gy = y.saturating_add(row as i32 * s_);
                    let mut col = 0;
                    while col < GLYPH_W {
                        if bits & (1 << (GLYPH_W - 1 - col)) != 0 {
                            let start = col;
                            while col < GLYPH_W && bits & (1 << (GLYPH_W - 1 - col)) != 0 {
                                col += 1;
                            }
                            self.fill_rect(Rect::new(gx.saturating_add(start * s_), gy, (col - start) * s_, s_), c);
                        } else {
                            col += 1;
                        }
                    }
                }
            }
            cx += adv as i64;
        }
        cx.clamp(i32::MIN as i64, i32::MAX as i64) as i32
    }

    /// Draw `s` inside `r`, truncated with ".." to fit, vertically centred
    /// (on cap height) and aligned horizontally. Clipped to `r`.
    pub fn draw_text_in(&mut self, r: Rect, s: &str, c: Color, scale: i32, align: Align) {
        if r.is_empty() {
            return;
        }
        let t = fit_text(s, r.w, scale);
        let w = text_width(&t, scale);
        let x = match align {
            Align::Left => r.x,
            Align::Center => r.x + (r.w - w) / 2,
            Align::Right => r.right() - w,
        };
        let y = r.y + (r.h - CAP_HEIGHT * sc(scale)) / 2;
        self.push_clip(r);
        self.draw_text(x, y, &t, c, scale);
        self.pop_clip();
    }

    /// Alias of [`draw_text_in`](Self::draw_text_in) with left alignment.
    pub fn draw_text_clipped(&mut self, r: Rect, s: &str, c: Color, scale: i32) {
        self.draw_text_in(r, s, c, scale, Align::Left);
    }

    // ---- dirty tracking -------------------------------------------------

    /// Force the next [`take_dirty_rects`](Self::take_dirty_rects) to report
    /// the whole canvas (e.g. after the host lost the screen contents).
    pub fn invalidate(&mut self) {
        self.shadow_valid = false;
    }

    /// Compare against the frame captured by the previous call and return the
    /// changed areas, as `tile`-aligned rectangles (horizontal runs of dirty
    /// tiles, merged vertically when they line up). More than
    /// [`MAX_DIRTY_RECTS`] rects collapse into one bounding rect. The first call
    /// after `new`/`resize`/`invalidate` returns the whole canvas.
    pub fn take_dirty_rects(&mut self, tile: i32) -> Vec<Rect> {
        let (w, h) = (self.width, self.height);
        if w == 0 || h == 0 {
            self.shadow_valid = true;
            self.shadow.clear();
            return Vec::new();
        }
        if !self.shadow_valid || self.shadow.len() != self.pixels.len() {
            self.shadow = self.pixels.clone();
            self.shadow_valid = true;
            return vec![self.bounds()];
        }
        let tile = tile.clamp(1, 1024);
        let tw = ((w + tile - 1) / tile) as usize;
        let stride = w as usize * 4;
        let mut out: Vec<Rect> = Vec::new();
        let mut prev_row: Vec<usize> = Vec::new(); // indices into `out` touching previous tile row
        let mut dirty = vec![false; tw];
        let mut y0 = 0;
        while y0 < h {
            let y1 = (y0 + tile).min(h);
            dirty.iter_mut().for_each(|d| *d = false);
            for y in y0..y1 {
                let row = y as usize * stride;
                for (tx, d) in dirty.iter_mut().enumerate() {
                    if *d {
                        continue;
                    }
                    let x0 = tx * tile as usize;
                    let x1 = (x0 + tile as usize).min(w as usize);
                    let (s, e) = (row + x0 * 4, row + x1 * 4);
                    if self.pixels[s..e] != self.shadow[s..e] {
                        *d = true;
                    }
                }
            }
            let mut this_row = Vec::new();
            let mut tx = 0;
            while tx < tw {
                if !dirty[tx] {
                    tx += 1;
                    continue;
                }
                let start = tx;
                while tx < tw && dirty[tx] {
                    tx += 1;
                }
                let x0 = start as i32 * tile;
                let x1 = (tx as i32 * tile).min(w);
                let run = Rect::new(x0, y0, x1 - x0, y1 - y0);
                // Extend a rect from the previous tile row with the same span.
                if let Some(&i) = prev_row.iter().find(|&&i| out[i].x == run.x && out[i].w == run.w && out[i].bottom() == y0) {
                    out[i].h += run.h;
                    this_row.push(i);
                } else {
                    out.push(run);
                    this_row.push(out.len() - 1);
                }
            }
            prev_row = this_row;
            y0 = y1;
        }
        if !out.is_empty() {
            self.shadow.copy_from_slice(&self.pixels);
        }
        if out.len() > MAX_DIRTY_RECTS {
            let b = out.iter().fold(Rect::EMPTY, |a, r| a.union(*r));
            return vec![b];
        }
        out
    }

    /// RGBA bytes of `r` (clipped to the canvas), row-major, tightly packed.
    pub fn pixels_of(&self, r: Rect) -> Vec<u8> {
        let a = self.bounds().intersect(r);
        if a.is_empty() {
            return Vec::new();
        }
        let stride = self.width as usize * 4;
        let mut out = Vec::with_capacity(a.w as usize * a.h as usize * 4);
        for y in a.y..a.bottom() {
            let s = y as usize * stride + a.x as usize * 4;
            out.extend_from_slice(&self.pixels[s..s + a.w as usize * 4]);
        }
        out
    }
}

/// Horizontal text alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}
