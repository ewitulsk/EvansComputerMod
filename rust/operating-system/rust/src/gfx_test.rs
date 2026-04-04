//! Graphics test program — exercises all graphics framebuffer features.
//!
//! Run as shell command: `gfxtest` (320×200) or `gfxtest 640` (640×400).
//!
//! Tests:
//! 1. Color palette bars — all 256 colors
//! 2. Overlapping colored rectangles
//! 3. Palette animation (color cycling)
//! 4. Bouncing ball animation
//! 5. Overlay mode — text on top of graphics

use crate::gfx;
use crate::terminal;

pub fn run(args: &[&str]) {
    let (width, height) = match args.first().copied() {
        Some("640") => (640u16, 400u16),
        _ => (320u16, 200u16),
    };

    terminal::println(&format!("Graphics test: {}x{}", width, height));
    terminal::println("Starting in 1 second...");
    terminal::sync();
    terminal::sleep(1000);

    // Initialize graphics mode
    gfx::init(width, height);
    gfx::set_mode(1); // graphics-only
    terminal::sync();

    // === Test 1: Color palette bars ===
    test_palette_bars(width, height);

    // === Test 2: Colored rectangles ===
    test_rectangles(width, height);

    // === Test 3: Palette animation ===
    test_palette_animation(width, height);

    // === Test 4: Bouncing ball ===
    test_bouncing_ball(width, height);

    // === Test 5: Overlay mode ===
    test_overlay(width, height);

    // Return to text mode
    gfx::set_mode(0);
    terminal::clear();
    terminal::println("Graphics test complete.");
    terminal::sync();
}

/// Test 1: Draw vertical bars showing all 256 palette colors.
fn test_palette_bars(width: u16, height: u16) {
    gfx::clear(0);

    // Draw 256 colored bars across the top quarter of the screen
    let quarter_h = height / 4;
    let bar_w = width / 256;
    let bar_w = if bar_w == 0 { 1 } else { bar_w };

    for color in 0u16..256 {
        let x = (color * bar_w).min(width - 1);
        gfx::fill_rect(x, 0, bar_w, quarter_h, color as u8);
    }

    // Draw a label area in the bottom — white rect with text
    gfx::fill_rect(10, quarter_h + 10, 200, 20, 15); // white
    // Write "256 COLORS" using pixel art (simple block letters)
    draw_text_simple(14, quarter_h + 14, "256 COLORS", 0);

    gfx::mark_pixel_dirty();
    terminal::sync();
    terminal::sleep(2000);
}

/// Test 2: Draw overlapping colored rectangles.
fn test_rectangles(width: u16, height: u16) {
    gfx::clear(0);

    let colors: [u8; 10] = [1, 2, 4, 5, 6, 9, 10, 12, 13, 14];

    for (i, &color) in colors.iter().enumerate() {
        let x = (i as u16) * (width / 15) + 10;
        let y = 20 + (i as u16) * 8;
        let w = width / 4;
        let h = height / 3;
        gfx::fill_rect(x, y, w, h, color);

        // Draw outline
        gfx::rect(x, y, w, h, 15); // white outline
    }

    gfx::mark_pixel_dirty();
    terminal::sync();
    terminal::sleep(2000);
}

/// Test 3: Palette animation — cycle colors without changing pixel data.
fn test_palette_animation(width: u16, height: u16) {
    gfx::clear(0);

    // Draw a grid of colored squares using indices 16-231 (color cube)
    let cols = 36u16;
    let rows = 6u16;
    let cell_w = width / cols;
    let cell_h = height / rows;

    for idx in 0u16..216 {
        let col = idx % cols;
        let row = idx / cols;
        let color = (idx + 16) as u8;
        gfx::fill_rect(col * cell_w, row * cell_h, cell_w, cell_h, color);
    }
    gfx::mark_pixel_dirty();
    terminal::sync();
    terminal::sleep(500);

    // Now animate by cycling palette entries (pixel data stays the same)
    for frame in 0u16..180 {
        for i in 0u8..216 {
            let shifted = ((i as u16 + frame) % 216) as u8;
            let (r, g, b) = gfx::default_vga_color(16 + shifted);
            gfx::set_palette_entry(16 + i, r, g, b);
        }
        gfx::mark_palette_dirty();
        terminal::sync();
        terminal::sleep(33); // ~30fps

        terminal::yield_interrupts();
    }

    // Restore default palette
    for i in 0u8..216 {
        let (r, g, b) = gfx::default_vga_color(16 + i);
        gfx::set_palette_entry(16 + i, r, g, b);
    }
    gfx::mark_palette_dirty();
    terminal::sync();
}

/// Test 4: Bouncing ball animation.
fn test_bouncing_ball(width: u16, height: u16) {
    let ball_size: u16 = if width >= 640 { 20 } else { 10 };
    let mut ball_x: i16 = width as i16 / 2;
    let mut ball_y: i16 = height as i16 / 2;
    let mut dx: i16 = 3;
    let mut dy: i16 = 2;

    for _frame in 0..300 {
        // Clear with dark blue background
        gfx::clear(4);

        // Draw border
        gfx::rect(0, 0, width, height, 7);
        gfx::rect(1, 1, width - 2, height - 2, 8);

        // Draw a few static obstacles
        gfx::fill_rect(width / 4, height / 4, 4, height / 2, 1);
        gfx::fill_rect(3 * width / 4, height / 4, 4, height / 2, 2);
        gfx::fill_rect(width / 4, height / 2, width / 2, 4, 5);

        // Draw shadow
        gfx::fill_rect(
            (ball_x + 2) as u16,
            (ball_y + 2) as u16,
            ball_size,
            ball_size,
            0,
        );

        // Draw ball
        gfx::fill_rect(ball_x as u16, ball_y as u16, ball_size, ball_size, 12);
        // Ball highlight (top-left shine)
        let hl_size = ball_size / 3;
        gfx::fill_rect(
            (ball_x + 2) as u16,
            (ball_y + 2) as u16,
            hl_size.max(2),
            hl_size.max(2),
            15,
        );

        gfx::mark_pixel_dirty();
        terminal::sync();

        // Move ball
        ball_x += dx;
        ball_y += dy;

        // Bounce off walls
        if ball_x <= 2 || ball_x + ball_size as i16 >= width as i16 - 2 {
            dx = -dx;
            ball_x += dx; // prevent sticking
        }
        if ball_y <= 2 || ball_y + ball_size as i16 >= height as i16 - 2 {
            dy = -dy;
            ball_y += dy;
        }

        terminal::sleep(16); // ~60fps
        terminal::yield_interrupts();
    }
}

/// Test 5: Overlay mode — graphics background with text on top.
fn test_overlay(width: u16, height: u16) {
    // Draw a gradient background
    gfx::clear(0);
    for y in 0..height {
        // Map y position to a color index in the 6x6x6 cube
        let color = 16 + ((y as usize * 215) / height as usize) as u8;
        gfx::fill_rect(0, y, width, 1, color);
    }
    gfx::mark_pixel_dirty();

    // Switch to overlay mode
    gfx::set_mode(2);
    terminal::sync();
    terminal::sleep(500);

    // Print text that appears on top of the gradient
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

    terminal::sleep(5000);
}

// ---------------------------------------------------------------------------
// Simple pixel-art text rendering (5x7 font for labels)
// ---------------------------------------------------------------------------

/// Draw a string using a minimal 5×7 pixel font.
/// Only supports uppercase A-Z, 0-9, and space.
fn draw_text_simple(x: u16, y: u16, text: &str, color: u8) {
    let mut cx = x;
    for ch in text.chars() {
        if let Some(glyph) = get_simple_glyph(ch) {
            for (row, &bits) in glyph.iter().enumerate() {
                for col in 0..5u16 {
                    if bits & (1 << (4 - col)) != 0 {
                        gfx::set_pixel(cx + col, y + row as u16, color);
                    }
                }
            }
        }
        cx += 6; // 5px char + 1px spacing
    }
}

/// Get a 5×7 glyph bitmap for a character. Each byte is a row, MSB-first.
fn get_simple_glyph(ch: char) -> Option<[u8; 7]> {
    Some(match ch {
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
        ' ' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000],
        _ => return None,
    })
}
