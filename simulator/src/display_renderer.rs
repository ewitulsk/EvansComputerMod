//! Display renderer for the simulator.
//!
//! Renders the framebuffer to the terminal using half-block characters (▀▄█)
//! with 24-bit ANSI color. Each terminal cell represents 2 vertical pixels.

use std::io::{self, Write};
use crossterm::{cursor, execute};

use crate::host::framebuffer::SimFramebuffer;

/// Render the framebuffer to the terminal using half-block characters.
/// Each terminal cell displays 2 vertical pixels using the upper-half block (▀)
/// with separate foreground (top pixel) and background (bottom pixel) colors.
///
/// If `offset_x` and `offset_y` are non-zero, the display is positioned
/// at that terminal position (for split mode).
pub fn render_framebuffer(
    fb: &SimFramebuffer,
    offset_col: u16,
    offset_row: u16,
) -> io::Result<()> {
    let mut stdout = io::stdout();
    let w = fb.width;
    let h = fb.height;

    // Each terminal row represents 2 pixel rows
    let term_rows = (h + 1) / 2;

    for trow in 0..term_rows {
        let py_top = trow * 2;
        let py_bot = py_top + 1;

        execute!(stdout, cursor::MoveTo(offset_col, offset_row + trow as u16))?;

        let mut row_str = String::with_capacity(w * 30); // rough estimate for ANSI sequences

        for px in 0..w {
            let top_idx = (py_top * w + px) * 4;
            let tr = fb.pixels[top_idx];
            let tg = fb.pixels[top_idx + 1];
            let tb = fb.pixels[top_idx + 2];

            let (br, bg, bb) = if py_bot < h {
                let bot_idx = (py_bot * w + px) * 4;
                (fb.pixels[bot_idx], fb.pixels[bot_idx + 1], fb.pixels[bot_idx + 2])
            } else {
                (0, 0, 0)
            };

            // Use ▀ (upper half block): foreground = top pixel, background = bottom pixel
            row_str.push_str(&format!(
                "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m▀",
                tr, tg, tb, br, bg, bb
            ));
        }
        // Reset colors
        row_str.push_str("\x1b[0m");
        write!(stdout, "{}", row_str)?;
    }

    stdout.flush()?;
    Ok(())
}

/// Render the framebuffer as PPM to a file (for debugging).
pub fn render_framebuffer_ppm(fb: &SimFramebuffer, path: &str) -> io::Result<()> {
    use std::fs::File;
    let mut f = File::create(path)?;
    write!(f, "P6\n{} {}\n255\n", fb.width, fb.height)?;
    for i in (0..fb.pixels.len()).step_by(4) {
        f.write_all(&[fb.pixels[i], fb.pixels[i + 1], fb.pixels[i + 2]])?;
    }
    Ok(())
}

/// Render a border around the display area.
pub fn render_display_border(
    width: usize,
    height: usize,
    offset_col: u16,
    offset_row: u16,
) -> io::Result<()> {
    let mut stdout = io::stdout();
    let term_rows = ((height + 1) / 2) as u16;
    let display_cols = width as u16;

    // Top border
    execute!(stdout, cursor::MoveTo(offset_col.saturating_sub(1), offset_row.saturating_sub(1)))?;
    write!(stdout, "┌{}┐", "─".repeat(display_cols as usize))?;

    // Side borders
    for row in 0..term_rows {
        execute!(stdout, cursor::MoveTo(offset_col.saturating_sub(1), offset_row + row))?;
        write!(stdout, "│")?;
        execute!(stdout, cursor::MoveTo(offset_col + display_cols, offset_row + row))?;
        write!(stdout, "│")?;
    }

    // Bottom border
    execute!(stdout, cursor::MoveTo(offset_col.saturating_sub(1), offset_row + term_rows))?;
    write!(stdout, "└{}┘", "─".repeat(display_cols as usize))?;

    stdout.flush()?;
    Ok(())
}
