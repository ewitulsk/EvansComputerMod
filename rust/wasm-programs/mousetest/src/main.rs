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
//! `mousetest screen` instead takes over the attached Screen wall, which
//! must be a touch wall (Tier 3 HD Screens): clicking the wall in the world
//! draws a dot there and prints `touch <kind> <x> <y> buttons=0x..` for
//! every pointer event, in the Screen's pixels.
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
    if std::env::args().skip(1).any(|a| a == "screen" || a == "--screen") {
        touch_screen();
        return;
    }
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

/// `mousetest screen`: report touches on a Tier 3 Screen wall.
fn touch_screen() {
    use std::io::Write;
    let Some(m) = gfx_child::modes(Target::Screen) else {
        eprintln!("mousetest: no Screen wall attached");
        std::process::exit(1);
    };
    if !m.touch || m.max_width == 0 || m.max_height == 0 {
        eprintln!("mousetest: this Screen wall is Tier {} and has no touch (Tier 3 HD Screens do)", m.tier);
        std::process::exit(1);
    }
    let (w, h) = (m.max_width as i32, m.max_height as i32);
    if gfx_child::init2(Target::Screen, w, h, FORMAT_INDEXED8, 0).is_err() {
        eprintln!("mousetest: the Screen is busy");
        std::process::exit(1);
    }
    gfx_child::set_screen_power(true);
    let mut buf = vec![COLOR_BLUE; (w * h) as usize];
    let _ = gfx_child::blit_rect(Target::Screen, 0, 0, w, h, &buf, FORMAT_INDEXED8);
    if !mouse::enable() {
        eprintln!("mousetest: mouse capture not available on the Screen");
        std::process::exit(1);
    }
    println!("mousetest: touch the Screen ({}x{}), Ctrl+T to stop", w, h);
    let _ = std::io::stdout().flush();
    loop {
        let mut drew = false;
        while let Some(ev) = mouse::poll() {
            let kind = match ev.kind {
                MouseKind::Move => "move",
                MouseKind::Down => "down",
                MouseKind::Up => "up",
                MouseKind::Scroll => "scroll",
                MouseKind::Other => "other",
            };
            println!("touch {} {} {} buttons={:#04x}", kind, ev.x, ev.y, ev.buttons);
            if ev.kind == MouseKind::Down && ev.from_screen() {
                draw_dot(&mut buf, w, h, ev.x as i32, ev.y as i32, COLOR_YELLOW);
                drew = true;
            }
        }
        let _ = std::io::stdout().flush();
        if drew && gfx_child::blit_rect(Target::Screen, 0, 0, w, h, &buf, FORMAT_INDEXED8).is_err() {
            break;
        }
        if !mouse::is_active() {
            break;
        }
        thread::sleep(Duration::from_millis(16));
    }
    mouse::disable();
}

fn draw_dot(buf: &mut [u8], w: i32, h: i32, cx: i32, cy: i32, color: u8) {
    for y in (cy - 2).max(0)..=(cy + 2).min(h - 1) {
        for x in (cx - 2).max(0)..=(cx + 2).min(w - 1) {
            buf[(y * w + x) as usize] = color;
        }
    }
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
