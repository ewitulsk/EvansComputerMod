//! Graphics test program — exercises all graphics framebuffer features.
//!
//! Usage:
//!   `gfxtest`                — run on the terminal display at 320x200
//!   `gfxtest 640`            — run on the terminal at 640x400
//!   `gfxtest screen`         — run on the in-world Screen cluster at its
//!                              native (cluster-scaled) resolution
//!   `gfxtest screen <WxH>`   — run on the screen with an explicit override
//!
//! All tests scale proportionally so they look reasonable on any resolution
//! from a single-tile screen (128x72) up to a multi-tile cluster or the
//! 640x400 terminal.

use crate::gfx;
use crate::screen;
use crate::terminal;

#[derive(Copy, Clone, PartialEq)]
enum Target {
    Terminal,
    Screen,
}

pub fn run(args: &[&str]) {
    // Parse target and resolution from args.
    let mut target = Target::Terminal;
    let mut explicit_size: Option<(u16, u16)> = None;

    let mut idx = 0;
    if idx < args.len() {
        match args[idx] {
            "screen" | "monitor" => {
                target = Target::Screen;
                idx += 1;
            }
            _ => {}
        }
    }
    if idx < args.len() {
        let arg = args[idx];
        if let Some((w, h)) = parse_size(arg) {
            explicit_size = Some((w, h));
        } else if arg == "640" {
            explicit_size = Some((640, 400));
        } else if arg == "320" {
            explicit_size = Some((320, 200));
        }
    }

    // Resolve the target resolution.
    let (width, height) = match target {
        Target::Terminal => explicit_size.unwrap_or((320, 200)),
        Target::Screen => {
            if !screen::is_attached() {
                terminal::println("No screen attached. Use 'gfxtest' to run on the terminal.");
                return;
            }
            match explicit_size {
                Some(s) => s,
                None => (screen::host_width(), screen::host_height()),
            }
        }
    };

    if width == 0 || height == 0 {
        terminal::println("Invalid resolution (0x0).");
        return;
    }

    terminal::println(&format!(
        "Graphics test ({}): {}x{}",
        match target {
            Target::Terminal => "terminal",
            Target::Screen => "screen",
        },
        width,
        height
    ));
    terminal::println("Starting in 1 second...");
    terminal::sync();
    terminal::sleep(1000);

    // Initialize the selected target.
    match target {
        Target::Terminal => {
            gfx::init(width, height);
            gfx::set_mode(1);
            terminal::sync();
        }
        Target::Screen => {
            // The host already wrote width/height/mode into the screen header
            // when the cluster was formed. init() populates the palette and
            // clears the framebuffer.
            if !screen::init() {
                terminal::println("Screen init failed.");
                return;
            }
            screen::sync();
        }
    }

    test_palette_bars(target, width, height);
    test_rectangles(target, width, height);
    test_palette_animation(target, width, height);
    test_bouncing_ball(target, width, height);
    test_overlay(target, width, height);

    // Clean up the target and report completion on the host terminal.
    match target {
        Target::Terminal => {
            gfx::set_mode(0);
            terminal::clear();
            terminal::println("Graphics test complete.");
            terminal::sync();
        }
        Target::Screen => {
            // Blank the screen so the final overlay frame doesn't linger,
            // then turn the monitor off so its face reverts to the
            // inactive "no signal" texture.
            screen::clear(0);
            screen::mark_pixel_dirty();
            screen::sync();
            screen::set_power(false);
            // Report completion on the host terminal where the command was run.
            terminal::println("Graphics test complete.");
            terminal::sync();
        }
    }
}

fn parse_size(s: &str) -> Option<(u16, u16)> {
    let s = s.to_ascii_lowercase();
    let idx = s.find('x')?;
    let w: u16 = s[..idx].parse().ok()?;
    let h: u16 = s[idx + 1..].parse().ok()?;
    if w == 0 || h == 0 { return None; }
    Some((w, h))
}

// ---------------------------------------------------------------------------
// Target-aware drawing helpers — dispatch to gfx or screen module.
// ---------------------------------------------------------------------------

fn t_clear(target: Target, color: u8) {
    match target {
        Target::Terminal => gfx::clear(color),
        Target::Screen => screen::clear(color),
    }
}

fn t_fill_rect(target: Target, x: u16, y: u16, w: u16, h: u16, color: u8) {
    match target {
        Target::Terminal => gfx::fill_rect(x, y, w, h, color),
        Target::Screen => screen::fill_rect(x, y, w, h, color),
    }
}

fn t_rect(target: Target, x: u16, y: u16, w: u16, h: u16, color: u8) {
    match target {
        Target::Terminal => gfx::rect(x, y, w, h, color),
        Target::Screen => screen::rect(x, y, w, h, color),
    }
}

fn t_set_palette_entry(target: Target, idx: u8, r: u8, g: u8, b: u8) {
    match target {
        Target::Terminal => gfx::set_palette_entry(idx, r, g, b),
        Target::Screen => screen::set_palette_entry(idx, r, g, b),
    }
}

fn t_mark_pixel_dirty(target: Target) {
    match target {
        Target::Terminal => gfx::mark_pixel_dirty(),
        Target::Screen => screen::mark_pixel_dirty(),
    }
}

fn t_mark_palette_dirty(target: Target) {
    match target {
        Target::Terminal => gfx::mark_palette_dirty(),
        Target::Screen => screen::mark_palette_dirty(),
    }
}

fn t_sync(target: Target) {
    match target {
        Target::Terminal => terminal::sync(),
        Target::Screen => screen::sync(),
    }
}

fn t_set_mode(target: Target, mode: u8) {
    match target {
        Target::Terminal => gfx::set_mode(mode),
        // Screen is always graphics mode from the host's perspective;
        // overlay text is blitted as pixels directly.
        Target::Screen => { let _ = mode; }
    }
}

// ---------------------------------------------------------------------------
// Tests. All sizes derived from the target resolution so the tests scale
// cleanly from a 128x72 single-tile screen up to a 640x400 terminal.
// ---------------------------------------------------------------------------

fn test_palette_bars(target: Target, width: u16, height: u16) {
    t_clear(target, 0);

    // 256 colored vertical bars across the top quarter
    let quarter_h = (height / 4).max(1);
    let bar_w = (width / 256).max(1);
    for color in 0u16..256 {
        let x = (color * bar_w).min(width.saturating_sub(1));
        t_fill_rect(target, x, 0, bar_w, quarter_h, color as u8);
    }

    // Label box: proportional size and position
    let label_w = ((width as u32 * 2) / 3) as u16;
    let label_h = (height / 10).max(12);
    let label_x = width / 20;
    let label_y = quarter_h + (height / 24).max(2);
    t_fill_rect(target, label_x, label_y, label_w, label_h, 15);

    // Scale the 5x7 glyphs so they stay readable on larger screens.
    let text_scale = (width / 160).max(1);
    let inset = (label_h.saturating_sub(7 * text_scale)) / 2;
    draw_label(target, label_x + 4 * text_scale, label_y + inset, "256 COLORS", 0, text_scale);

    t_mark_pixel_dirty(target);
    t_sync(target);
    terminal::sleep(2000);
}

fn test_rectangles(target: Target, width: u16, height: u16) {
    t_clear(target, 0);

    let colors: [u8; 10] = [1, 2, 4, 5, 6, 9, 10, 12, 13, 14];
    let pad_x = (width / 64).max(1);
    let pad_y = (height / 32).max(1);

    for (i, &color) in colors.iter().enumerate() {
        let step_x = (width / 15) as u16;
        let x = (i as u16) * step_x + pad_x;
        let y = pad_y * 4 + (i as u16) * pad_y * 2;
        let w = width / 4;
        let h = height / 3;
        t_fill_rect(target, x, y, w, h, color);
        t_rect(target, x, y, w, h, 15);
    }

    t_mark_pixel_dirty(target);
    t_sync(target);
    terminal::sleep(2000);
}

fn test_palette_animation(target: Target, width: u16, height: u16) {
    t_clear(target, 0);

    let cols = 36u16;
    let rows = 6u16;
    let cell_w = (width / cols).max(1);
    let cell_h = (height / rows).max(1);

    for idx in 0u16..216 {
        let col = idx % cols;
        let row = idx / cols;
        let color = (idx + 16) as u8;
        t_fill_rect(target, col * cell_w, row * cell_h, cell_w, cell_h, color);
    }
    t_mark_pixel_dirty(target);
    t_sync(target);
    terminal::sleep(500);

    // Animate by cycling palette entries (pixel data stays the same)
    for frame in 0u16..180 {
        for i in 0u8..216 {
            let shifted = ((i as u16 + frame) % 216) as u8;
            // Use the gfx module's default_vga_color helper for the
            // terminal target; for the screen target, reproduce inline.
            let (r, g, b) = default_vga_color(16 + shifted);
            t_set_palette_entry(target, 16 + i, r, g, b);
        }
        t_mark_palette_dirty(target);
        t_sync(target);
        terminal::sleep(33);
        terminal::yield_interrupts();
    }

    // Restore default palette
    for i in 0u8..216 {
        let (r, g, b) = default_vga_color(16 + i);
        t_set_palette_entry(target, 16 + i, r, g, b);
    }
    t_mark_palette_dirty(target);
    t_sync(target);
}

fn test_bouncing_ball(target: Target, width: u16, height: u16) {
    let ball_size: u16 = (width / 32).max(4);
    let mut ball_x: i16 = width as i16 / 2;
    let mut ball_y: i16 = height as i16 / 2;
    let mut dx: i16 = ((width / 160).max(1)) as i16;
    let mut dy: i16 = ((height / 100).max(1)) as i16;

    let border_inset = (width / 160).max(1);
    let bar_thick = (width / 80).max(2);
    let bar_len = height / 2;

    for _frame in 0..300 {
        t_clear(target, 4);

        t_rect(target, 0, 0, width, height, 7);
        t_rect(target, 1, 1, width.saturating_sub(2), height.saturating_sub(2), 8);

        t_fill_rect(target, width / 4, height / 4, bar_thick, bar_len, 1);
        t_fill_rect(target, 3 * width / 4, height / 4, bar_thick, bar_len, 2);
        t_fill_rect(target, width / 4, height / 2, width / 2, bar_thick, 5);

        // Shadow
        t_fill_rect(
            target,
            (ball_x + 2).max(0) as u16,
            (ball_y + 2).max(0) as u16,
            ball_size,
            ball_size,
            0,
        );
        // Ball
        t_fill_rect(target, ball_x.max(0) as u16, ball_y.max(0) as u16, ball_size, ball_size, 12);
        // Highlight
        let hl = (ball_size / 3).max(2);
        t_fill_rect(
            target,
            (ball_x + 2).max(0) as u16,
            (ball_y + 2).max(0) as u16,
            hl,
            hl,
            15,
        );

        t_mark_pixel_dirty(target);
        t_sync(target);

        ball_x += dx;
        ball_y += dy;

        if ball_x <= border_inset as i16 || ball_x + ball_size as i16 >= width as i16 - border_inset as i16 {
            dx = -dx;
            ball_x += dx;
        }
        if ball_y <= border_inset as i16 || ball_y + ball_size as i16 >= height as i16 - border_inset as i16 {
            dy = -dy;
            ball_y += dy;
        }

        terminal::sleep(16);
        terminal::yield_interrupts();
    }
}

fn test_overlay(target: Target, width: u16, height: u16) {
    t_clear(target, 0);
    // Vertical gradient across the color cube
    for y in 0..height {
        let color = 16 + ((y as usize * 215) / height.max(1) as usize) as u8;
        t_fill_rect(target, 0, y, width, 1, color);
    }
    t_mark_pixel_dirty(target);

    // For the terminal, enable overlay mode so text is drawn on top by the
    // VTE layer. For the screen (always graphics-mode), rasterize the text
    // directly into the pixel buffer using the built-in bitmap font so the
    // overlay message is visible regardless of client rendering mode.
    match target {
        Target::Terminal => {
            t_set_mode(target, 2);
            terminal::sync();
            terminal::sleep(500);

            terminal::clear();
            terminal::println("");
            terminal::println("  === OVERLAY MODE ===");
            terminal::println("");
            terminal::println("  Text is rendered on top of graphics.");
            terminal::println("  Background cells with color 0 are transparent,");
            terminal::println("  showing the gradient through.");
            terminal::println("");
            terminal::println(&format!("  Resolution: {}x{}", width, height));
            terminal::println("  Press any key or wait 5 seconds...");
            terminal::sync();
        }
        Target::Screen => {
            let text_scale = (width / 128).max(1);
            let x0 = width / 16;
            let mut y = height / 6;
            let line_h = 10 * text_scale;
            screen::draw_text(x0, y, "OVERLAY MODE", 15, text_scale);
            y += line_h * 2;
            screen::draw_text(x0, y, &format!("RESOLUTION: {}X{}", width, height), 11, text_scale);
            y += line_h;
            screen::draw_text(x0, y, "SCREEN CLUSTER TEST", 14, text_scale);
            screen::mark_pixel_dirty();
            screen::sync();
        }
    }

    terminal::sleep(5000);
}

// Small pixel-art label helper routed through the target (avoids the need
// for a text rasterizer in the terminal's `gfx` module).
fn draw_label(target: Target, x: u16, y: u16, text: &str, color: u8, scale: u16) {
    match target {
        Target::Terminal => draw_text_simple_gfx(x, y, text, color, scale),
        Target::Screen => screen::draw_text(x, y, text, color, scale),
    }
}

fn draw_text_simple_gfx(x: u16, y: u16, text: &str, color: u8, scale: u16) {
    let s = scale.max(1);
    let mut cx = x;
    for ch in text.chars() {
        if let Some(glyph) = get_simple_glyph(ch) {
            for (row, &bits) in glyph.iter().enumerate() {
                for col in 0..5u16 {
                    if bits & (1 << (4 - col)) != 0 {
                        gfx::fill_rect(cx + col * s, y + row as u16 * s, s, s, color);
                    }
                }
            }
        }
        cx += 6 * s;
    }
}

/// Default VGA palette color (same as gfx::default_vga_color, reproduced so
/// the screen path doesn't need to call into gfx's accessors).
fn default_vga_color(idx: u8) -> (u8, u8, u8) {
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

fn get_simple_glyph(ch: char) -> Option<[u8; 7]> {
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
        ' ' => [0b00000; 7],
        _ => return None,
    })
}
