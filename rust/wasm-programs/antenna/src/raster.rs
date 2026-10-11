//! Raster plots for a display (indexed8 with the host's default RGB332
//! palette): the SWR curve and the polar pattern, with a tiny 3x5 font
//! for the axis labels.

use crate::ascii::{gain_at, peak, polar_rho, polar_xy, y_frac};
use crate::cli::Plane;

/// RGB332 palette indices.
pub const BLACK: u8 = 0x00;
pub const WHITE: u8 = 0xFF;
pub const GREY: u8 = 0x92;
pub const DARK: u8 = 0x49;
pub const GREEN: u8 = 0x1C;
pub const YELLOW: u8 = 0xFC;
pub const RED: u8 = 0xE0;
pub const CYAN: u8 = 0x1F;

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Self {
        Canvas { w, h, px: vec![BLACK; w * h] }
    }

    pub fn get(&self, x: usize, y: usize) -> u8 {
        self.px[y * self.w + x]
    }

    pub fn set(&mut self, x: i32, y: i32, c: u8) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.px[y as usize * self.w + x as usize] = c;
        }
    }

    pub fn line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, c: u8) {
        let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
        let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
        let mut err = dx + dy;
        loop {
            self.set(x0, y0, c);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u8) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, c);
            }
        }
    }

    pub fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, c: u8) {
        self.line(x, y, x + w - 1, y, c);
        self.line(x, y + h - 1, x + w - 1, y + h - 1, c);
        self.line(x, y, x, y + h - 1, c);
        self.line(x + w - 1, y, x + w - 1, y + h - 1, c);
    }

    /// Draws `text` with the 3x5 font at `scale`, top-left at (x, y).
    /// Returns the width drawn.
    pub fn text(&mut self, x: i32, y: i32, text: &str, c: u8, scale: i32) -> i32 {
        let mut cx = x;
        for ch in text.chars() {
            let g = glyph(ch);
            for row in 0..5 {
                for col in 0..3 {
                    if g & (1 << (14 - (row * 3 + col))) != 0 {
                        self.rect(cx + col * scale, y + row * scale, scale, scale, c);
                    }
                }
            }
            cx += 4 * scale;
        }
        cx - x
    }
}

/// Width of `text` in the 3x5 font.
pub fn text_width(text: &str, scale: i32) -> i32 {
    text.chars().count() as i32 * 4 * scale
}

/// 3x5 glyph, 15 bits row-major from the top-left. Unknown characters are blank.
pub fn glyph(ch: char) -> u16 {
    match ch.to_ascii_uppercase() {
        '0' => 0b111_101_101_101_111,
        '1' => 0b010_110_010_010_111,
        '2' => 0b111_001_111_100_111,
        '3' => 0b111_001_111_001_111,
        '4' => 0b101_101_111_001_001,
        '5' => 0b111_100_111_001_111,
        '6' => 0b111_100_111_101_111,
        '7' => 0b111_001_010_010_010,
        '8' => 0b111_101_111_101_111,
        '9' => 0b111_101_111_001_111,
        '.' => 0b000_000_000_000_010,
        ':' => 0b000_010_000_010_000,
        '-' => 0b000_000_111_000_000,
        '+' => 0b000_010_111_010_000,
        'B' => 0b110_101_110_101_110,
        'D' => 0b110_101_101_101_110,
        'E' => 0b111_100_110_100_111,
        'H' => 0b101_101_111_101_101,
        'I' => 0b111_010_010_010_111,
        'K' => 0b101_101_110_101_101,
        'M' => 0b101_111_111_101_101,
        'N' => 0b110_101_101_101_101,
        'R' => 0b110_101_110_101_101,
        'S' => 0b011_100_010_001_110,
        'U' => 0b101_101_101_101_111,
        'W' => 0b101_101_111_111_101,
        'Z' => 0b111_001_010_100_111,
        _ => 0,
    }
}

fn mhz_label(f: f64) -> String {
    let v = if f >= 1e6 { f / 1e6 } else { f / 1e3 };
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

/// Plot area of an SWR plot on a `w x h` canvas: (left, top, width, height).
pub fn swr_area(w: usize, h: usize) -> (i32, i32, i32, i32) {
    let s = scale_for(w);
    let left = 20 * s;
    let top = 8 * s;
    (left, top, w as i32 - left - 4 * s, h as i32 - top - 12 * s)
}

fn scale_for(w: usize) -> i32 {
    ((w / 256) as i32).max(1)
}

/// SWR against frequency: log SWR axis (1..10), grid at 1.5/2/3/5 with the
/// 2:1 line yellow, the curve green, the best point red, frequency labels
/// along the bottom (MHz) and the best reading at the top.
pub fn draw_swr(c: &mut Canvas, hz: &[f64], swr: &[f64]) {
    let s = scale_for(c.w);
    let (left, top, w, h) = swr_area(c.w, c.h);
    if hz.len() < 2 || w < 8 || h < 8 {
        return;
    }
    let row = |v: f64| top + ((1.0 - y_frac(v)) * (h - 1) as f64).round() as i32;
    let (f0, f1) = (hz[0], hz[hz.len() - 1]);
    let col = |f: f64| left + (((f - f0) / (f1 - f0)) * (w - 1) as f64).round() as i32;
    for g in [1.5, 2.0, 3.0, 5.0] {
        let r = row(g);
        c.line(left, r, left + w - 1, r, if g == 2.0 { YELLOW } else { DARK });
        let label = if g == 1.5 { "1.5".to_string() } else { format!("{}", g as i32) };
        c.text(left - 2 * s - text_width(&label, s), r - 2 * s, &label, if g == 2.0 { YELLOW } else { GREY }, s);
    }
    c.text(left - 2 * s - text_width("1", s), top + h - 5 * s, "1", GREY, s);
    c.text(left - 2 * s - text_width("10", s), top, "10", GREY, s);
    for k in 0..=4 {
        let f = f0 + (f1 - f0) * k as f64 / 4.0;
        let x = col(f);
        c.line(x, top, x, top + h - 1, DARK);
        let l = mhz_label(f);
        let lw = text_width(&l, s);
        c.text((x - lw / 2).clamp(0, c.w as i32 - lw), top + h + 3 * s, &l, GREY, s);
    }
    c.frame(left - 1, top - 1, w + 2, h + 2, GREY);
    let mut prev: Option<(i32, i32)> = None;
    for (f, v) in hz.iter().zip(swr) {
        if v.is_nan() || *v == f64::INFINITY {
            prev = None;
            continue;
        }
        let p = (col(*f), row(*v));
        if let Some(q) = prev {
            c.line(q.0, q.1, p.0, p.1, GREEN);
        } else {
            c.set(p.0, p.1, GREEN);
        }
        prev = Some(p);
    }
    if let Some(i) = crate::min_index(swr) {
        let (x, y) = (col(hz[i]), row(swr[i]));
        c.rect(x - s, y - s, 2 * s + 1, 2 * s + 1, RED);
        let label = format!("SWR {:.2}:1 {} MHZ", swr[i], mhz_label(hz[i]));
        c.text(left, 1 * s, &label, RED, s);
    }
}

/// A polar gain plot centred on the canvas: rings at the peak and -10/-20 dB,
/// the pattern (radius linear in dB over 30 dB) and the peak gain.
pub fn draw_polar(c: &mut Canvas, gains: &[f64], plane: Plane) {
    let s = scale_for(c.w);
    let cx = c.w as i32 / 2;
    let cy = c.h as i32 / 2 + 3 * s;
    let r = ((c.h as i32 - 14 * s) / 2).min(c.w as i32 / 2 - 4 * s).max(4);
    for (k, col) in [(1.0, GREY), (2.0 / 3.0, DARK), (1.0 / 3.0, DARK)] {
        let rr = r as f64 * k;
        let mut prev = None;
        for a in 0..=360 {
            let t = (a as f64).to_radians();
            let p = (cx + (rr * t.cos()).round() as i32, cy - (rr * t.sin()).round() as i32);
            if let Some(q) = prev {
                let (q0, q1): (i32, i32) = q;
                c.line(q0, q1, p.0, p.1, col);
            }
            prev = Some(p);
        }
    }
    c.line(cx - r, cy, cx + r, cy, DARK);
    c.line(cx, cy - r, cx, cy + r, DARK);
    let pk = peak(gains);
    if pk.is_finite() {
        let mut prev: Option<(i32, i32)> = None;
        for k in 0..=720 {
            let a = k as f64 * 0.5;
            let rho = polar_rho(gain_at(gains, a), pk);
            let (x, y) = polar_xy(plane, a);
            let p = (cx + (x * rho * r as f64).round() as i32, cy - (y * rho * r as f64).round() as i32);
            if let Some(q) = prev {
                c.line(q.0, q.1, p.0, p.1, GREEN);
            }
            prev = Some(p);
        }
        c.text(2 * s, 1 * s, &format!("{:.1} DBI", pk), CYAN, s);
    }
    let (t, b, l, rt) = match plane {
        Plane::Azimuth => ("N", "S", "W", "E"),
        Plane::Elevation => ("U", "D", "-", "+"),
    };
    c.text(cx - s, cy - r - 6 * s, t, WHITE, s);
    c.text(cx - s, cy + r + 2 * s, b, WHITE, s);
    c.text(cx - r - 5 * s, cy - 2 * s, l, WHITE, s);
    c.text(cx + r + 2 * s, cy - 2 * s, rt, WHITE, s);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(c: &Canvas, col: u8) -> usize {
        c.px.iter().filter(|p| **p == col).count()
    }

    #[test]
    fn glyphs_and_text() {
        assert_eq!(glyph('8').count_ones(), 13);
        assert_eq!(glyph('?'), 0);
        let mut c = Canvas::new(32, 8);
        let w = c.text(0, 0, "10", WHITE, 1);
        assert_eq!(w, 8);
        assert_eq!(count(&c, WHITE), (glyph('1').count_ones() + glyph('0').count_ones()) as usize);
    }

    #[test]
    fn swr_curve_dips_at_the_best_point() {
        let hz: Vec<f64> = (0..21).map(|i| 6e6 + i as f64 * 0.1e6).collect();
        let swr: Vec<f64> = hz.iter().map(|f| 1.1 + ((f - 7.1e6).abs() / 0.1e6) * 0.5).collect();
        let mut c = Canvas::new(256, 160);
        draw_swr(&mut c, &hz, &swr);
        let (left, top, w, h) = swr_area(256, 160);
        // the red marker sits at 7.1 MHz, low on the plot
        let x = left + ((1.1e6 / 2e6) * (w - 1) as f64).round() as i32;
        let y = top + ((1.0 - y_frac(1.1)) * (h - 1) as f64).round() as i32;
        assert_eq!(c.get(x as usize, y as usize), RED);
        // the 2:1 line is yellow across the plot
        let y2 = top + ((1.0 - y_frac(2.0)) * (h - 1) as f64).round() as i32;
        assert_eq!(c.get((left + 3) as usize, y2 as usize), YELLOW);
        assert!(count(&c, GREEN) > 50);
    }

    #[test]
    fn polar_draws_a_pattern() {
        let gains: Vec<f64> = (0..72).map(|i| if i < 36 { 0.0 } else { -20.0 }).collect();
        let mut c = Canvas::new(256, 160);
        draw_polar(&mut c, &gains, Plane::Azimuth);
        assert!(count(&c, GREEN) > 100);
        // the strong half (east side, bearings 0..180) reaches further right than the weak half reaches left
        let rightmost = (0..256).rev().find(|x| (0..160).any(|y| c.get(*x, y) == GREEN)).unwrap();
        let leftmost = (0..256).find(|x| (0..160).any(|y| c.get(*x, y) == GREEN)).unwrap();
        assert!(rightmost - 128 > 128 - leftmost, "right {rightmost} left {leftmost}");
    }
}
