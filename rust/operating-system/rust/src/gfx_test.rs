//! `gfxtest`: a graphics demo for the terminal gfx plane or a Screen cluster.
//!
//! Runs as a tick-driven job: `step` draws one frame (or advances one stage)
//! and returns the time it wants to run next. It never sleeps, so the kernel
//! stays responsive and Ctrl+T can stop it at any point.

use crate::console::{Console, Term};
use crate::gfx::{default_vga_color, Plane};
use crate::hal;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Intro,
    Bars,
    Rects,
    AnimSetup,
    Anim,
    Ball,
    Overlay,
    Finish,
}

pub struct GfxTest {
    plane: Plane,
    w: u16,
    h: u16,
    phase: Phase,
    frame: u32,
    ball: (i32, i32, i32, i32),
}

fn parse_size(s: &str) -> Option<(u16, u16)> {
    let (w, h) = s.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

impl GfxTest {
    /// Parse args, print the banner and return the job plus its first wake-up.
    pub fn start(args: &[String], con: &mut Console, now: i64) -> Option<(GfxTest, i64)> {
        let mut it = args.iter().map(|s| s.as_str()).peekable();
        let plane = match it.peek() {
            Some(&"screen") | Some(&"monitor") => {
                it.next();
                Plane::Screen
            }
            _ => Plane::Terminal,
        };
        let explicit = it.next().and_then(|a| match a {
            "640" => Some((640, 400)),
            "320" => Some((320, 200)),
            other => parse_size(other),
        });
        let (w, h) = match plane {
            Plane::Terminal => explicit.unwrap_or((320, 200)),
            Plane::Screen => {
                if !hal::screen::attached() {
                    con.println("No screen attached. Use 'gfxtest' to run on the terminal.");
                    return None;
                }
                explicit.unwrap_or_else(hal::screen::host_dims)
            }
        };
        if w == 0 || h == 0 {
            con.println("Invalid resolution (0x0).");
            return None;
        }
        let name = if plane == Plane::Screen { "screen" } else { "terminal" };
        con.println(&format!("Graphics test ({}): {}x{}", name, w, h));
        con.println("Starting in 1 second... (Ctrl+T to stop)");
        let job = GfxTest {
            plane,
            w,
            h,
            phase: Phase::Intro,
            frame: 0,
            ball: (w as i32 / 2, h as i32 / 2, (w as i32 / 160).max(1), (h as i32 / 100).max(1)),
        };
        Some((job, now + 1000))
    }

    /// Advance the demo. Returns the next deadline, or `None` once finished.
    pub fn step(&mut self, con: &mut Console, now: i64) -> Option<i64> {
        let p = self.plane;
        let (w, h) = (self.w, self.h);
        match self.phase {
            Phase::Intro => {
                if !p.init(w, h, 1) {
                    con.println("Resolution too large for this display.");
                    return None;
                }
                if p == Plane::Screen {
                    hal::screen::set_power(true);
                }
                self.phase = Phase::Bars;
                Some(now)
            }
            Phase::Bars => {
                p.clear(0);
                let quarter = (h / 4).max(1);
                let bar_w = (w / 256).max(1);
                for c in 0u16..256 {
                    p.fill_rect((c * bar_w).min(w.saturating_sub(1)), 0, bar_w, quarter, c as u8);
                }
                let (lw, lh) = (((w as u32 * 2) / 3) as u16, (h / 10).max(12));
                let (lx, ly) = (w / 20, quarter + (h / 24).max(2));
                p.fill_rect(lx, ly, lw, lh, 15);
                let s = (w / 160).max(1);
                p.draw_text(lx + 4 * s, ly + lh.saturating_sub(7 * s) / 2, "256 COLORS", 0, s);
                self.flush();
                self.phase = Phase::Rects;
                Some(now + 2000)
            }
            Phase::Rects => {
                p.clear(0);
                let (pad_x, pad_y) = ((w / 64).max(1), (h / 32).max(1));
                for (i, &c) in [1u8, 2, 4, 5, 6, 9, 10, 12, 13, 14].iter().enumerate() {
                    let x = i as u16 * (w / 15) + pad_x;
                    let y = pad_y * 4 + i as u16 * pad_y * 2;
                    p.fill_rect(x, y, w / 4, h / 3, c);
                    p.rect(x, y, w / 4, h / 3, 15);
                }
                self.flush();
                self.phase = Phase::AnimSetup;
                Some(now + 2000)
            }
            Phase::AnimSetup => {
                p.clear(0);
                let (cw, ch) = ((w / 36).max(1), (h / 6).max(1));
                for idx in 0u16..216 {
                    p.fill_rect((idx % 36) * cw, (idx / 36) * ch, cw, ch, (idx + 16) as u8);
                }
                self.flush();
                self.phase = Phase::Anim;
                self.frame = 0;
                Some(now + 500)
            }
            Phase::Anim => {
                if self.frame >= 180 {
                    for i in 0u8..216 {
                        let (r, g, b) = default_vga_color(16 + i);
                        p.set_palette_entry(16 + i, r, g, b);
                    }
                    p.mark_palette_dirty();
                    p.sync();
                    self.phase = Phase::Ball;
                    self.frame = 0;
                    return Some(now);
                }
                for i in 0u8..216 {
                    let shifted = ((i as u32 + self.frame) % 216) as u8;
                    let (r, g, b) = default_vga_color(16 + shifted);
                    p.set_palette_entry(16 + i, r, g, b);
                }
                p.mark_palette_dirty();
                p.sync();
                self.frame += 1;
                Some(now + 33)
            }
            Phase::Ball => {
                if self.frame >= 300 {
                    self.phase = Phase::Overlay;
                    self.frame = 0;
                    return Some(now);
                }
                self.draw_ball();
                self.frame += 1;
                Some(now + 16)
            }
            Phase::Overlay => {
                p.clear(0);
                for y in 0..h {
                    p.fill_rect(0, y, w, 1, 16 + ((y as usize * 215) / h.max(1) as usize) as u8);
                }
                match p {
                    Plane::Terminal => {
                        p.set_mode(2);
                        con.clear();
                        con.println("");
                        con.println("  === OVERLAY MODE ===");
                        con.println("");
                        con.println("  Text is rendered on top of graphics.");
                        con.println("  Background cells with color 0 are transparent,");
                        con.println("  showing the gradient through.");
                        con.println("");
                        con.println(&format!("  Resolution: {}x{}", w, h));
                    }
                    Plane::Screen => {
                        let s = (w / 128).max(1);
                        let x0 = w / 16;
                        let y = h / 6;
                        p.draw_text(x0, y, "OVERLAY MODE", 15, s);
                        p.draw_text(x0, y + 20 * s, &format!("RESOLUTION: {}X{}", w, h), 11, s);
                        p.draw_text(x0, y + 30 * s, "SCREEN CLUSTER TEST", 14, s);
                    }
                }
                self.flush();
                self.phase = Phase::Finish;
                Some(now + 5000)
            }
            Phase::Finish => {
                self.abort();
                if p == Plane::Terminal {
                    con.clear();
                }
                con.println("Graphics test complete.");
                None
            }
        }
    }

    fn flush(&self) {
        self.plane.mark_pixel_dirty();
        self.plane.sync();
    }

    fn draw_ball(&mut self) {
        let p = self.plane;
        let (w, h) = (self.w as i32, self.h as i32);
        let size = (w / 32).max(4);
        let inset = (w / 160).max(1);
        let bar = (w / 80).max(2) as u16;
        p.clear(4);
        p.rect(0, 0, w as u16, h as u16, 7);
        p.rect(1, 1, (w - 2).max(0) as u16, (h - 2).max(0) as u16, 8);
        p.fill_rect((w / 4) as u16, (h / 4) as u16, bar, (h / 2) as u16, 1);
        p.fill_rect((3 * w / 4) as u16, (h / 4) as u16, bar, (h / 2) as u16, 2);
        p.fill_rect((w / 4) as u16, (h / 2) as u16, (w / 2) as u16, bar, 5);
        let (bx, by, dx, dy) = self.ball;
        let at = |v: i32| v.max(0) as u16;
        p.fill_rect(at(bx + 2), at(by + 2), size as u16, size as u16, 0);
        p.fill_rect(at(bx), at(by), size as u16, size as u16, 12);
        let hl = (size / 3).max(2) as u16;
        p.fill_rect(at(bx + 2), at(by + 2), hl, hl, 15);
        self.flush();
        let (mut nx, mut ny, mut ndx, mut ndy) = (bx + dx, by + dy, dx, dy);
        if nx <= inset || nx + size >= w - inset {
            ndx = -ndx;
            nx += ndx;
        }
        if ny <= inset || ny + size >= h - inset {
            ndy = -ndy;
            ny += ndy;
        }
        self.ball = (nx, ny, ndx, ndy);
    }

    /// Stop immediately and give the display back to the text console.
    pub fn abort(&mut self) {
        self.plane.reset();
        Plane::Terminal.set_mode(0);
        Plane::Terminal.mark_pixel_dirty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_to_completion_on_virtual_time() {
        let _g = crate::hal::test_lock();
        let mut con = Console::new();
        let (mut job, mut next) = GfxTest::start(&["640".to_string()], &mut con, 0).unwrap();
        let mut steps = 0;
        while let Some(t) = job.step(&mut con, next) {
            assert!(t >= next);
            next = t;
            steps += 1;
            assert!(steps < 2000, "gfxtest never finished");
        }
        assert_eq!(Plane::Terminal.mode(), 0);
    }
}
