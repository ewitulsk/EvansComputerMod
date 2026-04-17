//! `mousetest` — draws a filled circle that follows the mouse cursor
//! over the terminal's graphics framebuffer. Exercises the mouse
//! capture host API ([`ecm_host_abi::mouse`]) and the new
//! [`ecm_host_abi::gfx_child::blit_rect`] pixel-push path.
//!
//! Controls:
//! * Move        — circle follows the cursor
//! * Left click  — flash white for a few frames
//! * Right click — cycle through a small color palette
//! * Middle      — reset to black
//! * Scroll      — grow/shrink radius (clamped 3..=60)
//! * Ctrl+T      — terminate (standard shell reset path)
//!
//! The program draws into a `Vec<u8>` of the full framebuffer each
//! frame, then blits the whole buffer in one `blit_rect` call. A
//! dirty-rect pass would be faster but this is simpler and the
//! 320×200 buffer is small enough that the full push is cheap.

use ecm_host_abi::gfx_child::{self, FORMAT_INDEXED8};
use ecm_host_abi::mouse::{self, MouseKind};
use ecm_host_abi::video::Target;
use std::thread;
use std::time::Duration;

const GFX_W: i32 = 320;
const GFX_H: i32 = 200;
const BUF_LEN: usize = (GFX_W * GFX_H) as usize;

/// Palette indices in the installed RGB332 palette (RRRGGGBB). These
/// are exact palette entries, so they survive a set_mode cycle.
const COLOR_BLUE: u8 = 0x03;
const COLOR_GREEN: u8 = 0x1C;
const COLOR_YELLOW: u8 = 0xFC;
const COLOR_RED: u8 = 0xE0;
const COLOR_BLACK: u8 = 0x00;
const COLOR_WHITE: u8 = 0xFF;

const COLORS: [u8; 4] = [COLOR_BLUE, COLOR_GREEN, COLOR_YELLOW, COLOR_RED];

fn main() {
    if gfx_child::init(Target::Terminal, GFX_W, GFX_H).is_err() {
        eprintln!("mousetest: gfx_init failed");
        std::process::exit(1);
    }
    if gfx_child::set_mode(Target::Terminal, 1).is_err() {
        eprintln!("mousetest: set_mode(gfx) failed");
        std::process::exit(1);
    }

    if !mouse::enable() {
        eprintln!("mousetest: mouse capture not available.");
        eprintln!("  open the terminal UI and ensure the display is in gfx mode.");
        let _ = gfx_child::set_mode(Target::Terminal, 0);
        std::process::exit(1);
    }

    println!("mousetest: move the mouse over the screen to steer the circle.");
    println!("  L click flash   R click cycle   M click black   scroll size   Ctrl+T exit");

    let mut buf: Vec<u8> = vec![0u8; BUF_LEN];
    let mut cx: i32 = GFX_W / 2;
    let mut cy: i32 = GFX_H / 2;
    let mut radius: i32 = 18;
    let mut color_idx: usize = 0;
    let mut flash_frames: i32 = 0;
    // Leftover scroll ticks; we only change radius on whole-ticks.
    // The host normalizes scroll to ±1 so this is just a direct add.

    loop {
        // Drain all queued mouse events before drawing. The ring is
        // drop-oldest on overflow, so this is bounded.
        while let Some(ev) = mouse::poll() {
            match ev.kind {
                MouseKind::Move => {
                    cx = ev.x as i32;
                    cy = ev.y as i32;
                }
                MouseKind::Down => match ev.button_code {
                    0 => flash_frames = 4,
                    1 => color_idx = (color_idx + 1) % COLORS.len(),
                    2 => color_idx = usize::MAX, // sentinel: draw black
                    _ => {}
                },
                MouseKind::Scroll => {
                    radius = (radius + (ev.scroll_dir as i32) * 2).clamp(3, 60);
                }
                _ => {}
            }
        }

        // Clear to black.
        for px in buf.iter_mut() {
            *px = COLOR_BLACK;
        }

        // Pick the fill color for this frame.
        let color = if flash_frames > 0 {
            COLOR_WHITE
        } else if color_idx == usize::MAX {
            COLOR_BLACK
        } else {
            COLORS[color_idx]
        };

        draw_circle(&mut buf, cx, cy, radius, color);
        if flash_frames > 0 {
            flash_frames -= 1;
        }

        if gfx_child::blit_rect(Target::Terminal, 0, 0, GFX_W, GFX_H, &buf, FORMAT_INDEXED8)
            .is_err()
        {
            // Host rejected the blit — most likely Ctrl+T was pressed
            // and the child is being aborted. Exit cleanly.
            break;
        }

        thread::sleep(Duration::from_millis(16));

        // If the host disabled capture on us (e.g. display mode change),
        // stop instead of spinning on an empty ring.
        if !mouse::is_active() {
            break;
        }
    }

    mouse::disable();
    let _ = gfx_child::set_mode(Target::Terminal, 0);
}

/// Filled-circle rasterizer. Uses squared radius to stay in integer
/// math; the branch per pixel is fine for radii we actually draw.
fn draw_circle(buf: &mut [u8], cx: i32, cy: i32, radius: i32, color: u8) {
    if radius <= 0 {
        return;
    }
    let r2 = radius * radius;
    let y0 = (cy - radius).max(0);
    let y1 = (cy + radius).min(GFX_H - 1);
    let x0 = (cx - radius).max(0);
    let x1 = (cx + radius).min(GFX_W - 1);
    for py in y0..=y1 {
        let dy = py - cy;
        let row = (py * GFX_W) as usize;
        for px in x0..=x1 {
            let dx = px - cx;
            if dx * dx + dy * dy <= r2 {
                buf[row + px as usize] = color;
            }
        }
    }
}
