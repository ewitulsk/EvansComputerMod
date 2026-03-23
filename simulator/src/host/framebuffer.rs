//! Framebuffer host functions for the simulator.
//!
//! Implements all fb_* host functions. When --display WxH is specified,
//! the framebuffer is active. Otherwise, all functions return -1.

use wasmtime::*;
use crate::wasm_host::HostState;
use super::memory;

/// Host function names registered by this module.
pub const FUNCTIONS: &[&str] = &[
    "fb_get_width",
    "fb_get_height",
    "fb_set_pixel",
    "fb_fill_rect",
    "fb_write_region",
    "fb_clear",
    "fb_flush",
    "fb_blit_text",
];

/// Simulated framebuffer state.
pub struct SimFramebuffer {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,     // RGBA, length = width * height * 4
    prev_pixels: Vec<u8>,    // Snapshot for diffing
    pub dirty: bool,
}

impl SimFramebuffer {
    pub fn new(width: usize, height: usize) -> Self {
        let size = width * height * 4;
        let mut pixels = vec![0u8; size];
        // Initialize to black with full alpha
        for i in (0..size).step_by(4) {
            pixels[i + 3] = 255; // alpha
        }
        Self {
            width,
            height,
            pixels: pixels.clone(),
            prev_pixels: pixels,
            dirty: false,
        }
    }

    pub fn set_pixel(&mut self, x: i32, y: i32, r: u8, g: u8, b: u8, a: u8) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        let idx = (y as usize * self.width + x as usize) * 4;
        self.pixels[idx] = r;
        self.pixels[idx + 1] = g;
        self.pixels[idx + 2] = b;
        self.pixels[idx + 3] = a;
        self.dirty = true;
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: u8, g: u8, b: u8, a: u8) {
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = ((x + w) as usize).min(self.width);
        let y1 = ((y + h) as usize).min(self.height);

        for py in y0..y1 {
            let row_base = py * self.width * 4;
            for px in x0..x1 {
                let idx = row_base + px * 4;
                self.pixels[idx] = r;
                self.pixels[idx + 1] = g;
                self.pixels[idx + 2] = b;
                self.pixels[idx + 3] = a;
            }
        }
        self.dirty = true;
    }

    pub fn write_region(&mut self, x: i32, y: i32, w: i32, h: i32, data: &[u8]) {
        if data.len() < (w * h * 4) as usize {
            return;
        }
        for row in 0..h {
            let src_y = y + row;
            if src_y < 0 || src_y as usize >= self.height {
                continue;
            }
            for col in 0..w {
                let src_x = x + col;
                if src_x < 0 || src_x as usize >= self.width {
                    continue;
                }
                let src_idx = (row * w + col) as usize * 4;
                let dst_idx = (src_y as usize * self.width + src_x as usize) * 4;
                self.pixels[dst_idx] = data[src_idx];
                self.pixels[dst_idx + 1] = data[src_idx + 1];
                self.pixels[dst_idx + 2] = data[src_idx + 2];
                self.pixels[dst_idx + 3] = data[src_idx + 3];
            }
        }
        self.dirty = true;
    }

    pub fn clear(&mut self, r: u8, g: u8, b: u8, a: u8) {
        for i in (0..self.pixels.len()).step_by(4) {
            self.pixels[i] = r;
            self.pixels[i + 1] = g;
            self.pixels[i + 2] = b;
            self.pixels[i + 3] = a;
        }
        self.dirty = true;
    }

    /// Flush: count changed tiles (16x16) and update snapshot.
    /// Returns number of changed tiles.
    pub fn flush(&mut self) -> i32 {
        if !self.dirty {
            return 0;
        }
        let tile_size = 16usize;
        let tiles_x = (self.width + tile_size - 1) / tile_size;
        let tiles_y = (self.height + tile_size - 1) / tile_size;
        let mut changed = 0i32;

        for ty in 0..tiles_y {
            for tx in 0..tiles_x {
                let px0 = tx * tile_size;
                let py0 = ty * tile_size;
                let px1 = (px0 + tile_size).min(self.width);
                let py1 = (py0 + tile_size).min(self.height);

                let mut tile_changed = false;
                for py in py0..py1 {
                    let base = py * self.width * 4;
                    for px in px0..px1 {
                        let idx = base + px * 4;
                        if self.pixels[idx] != self.prev_pixels[idx]
                            || self.pixels[idx + 1] != self.prev_pixels[idx + 1]
                            || self.pixels[idx + 2] != self.prev_pixels[idx + 2]
                            || self.pixels[idx + 3] != self.prev_pixels[idx + 3]
                        {
                            tile_changed = true;
                            break;
                        }
                    }
                    if tile_changed {
                        break;
                    }
                }

                if tile_changed {
                    changed += 1;
                    // Update snapshot for this tile
                    for py in py0..py1 {
                        let base = py * self.width * 4;
                        for px in px0..px1 {
                            let idx = base + px * 4;
                            self.prev_pixels[idx] = self.pixels[idx];
                            self.prev_pixels[idx + 1] = self.pixels[idx + 1];
                            self.prev_pixels[idx + 2] = self.pixels[idx + 2];
                            self.prev_pixels[idx + 3] = self.pixels[idx + 3];
                        }
                    }
                }
            }
        }

        self.dirty = false;
        changed
    }
}

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap("env", "fb_get_width", |caller: Caller<'_, HostState>| -> i32 {
        match caller.data().get_custom::<SimFramebuffer>() {
            Some(fb) => fb.width as i32,
            None => -1,
        }
    })?;

    linker.func_wrap("env", "fb_get_height", |caller: Caller<'_, HostState>| -> i32 {
        match caller.data().get_custom::<SimFramebuffer>() {
            Some(fb) => fb.height as i32,
            None => -1,
        }
    })?;

    linker.func_wrap("env", "fb_set_pixel", |mut caller: Caller<'_, HostState>,
        x: i32, y: i32, r: i32, g: i32, b: i32, a: i32| {
        if let Some(fb) = caller.data_mut().get_custom_mut::<SimFramebuffer>() {
            fb.set_pixel(x, y, r as u8, g as u8, b as u8, a as u8);
        }
    })?;

    linker.func_wrap("env", "fb_fill_rect", |mut caller: Caller<'_, HostState>,
        x: i32, y: i32, w: i32, h: i32, r: i32, g: i32, b: i32, a: i32| {
        if let Some(fb) = caller.data_mut().get_custom_mut::<SimFramebuffer>() {
            fb.fill_rect(x, y, w, h, r as u8, g as u8, b as u8, a as u8);
        }
    })?;

    linker.func_wrap("env", "fb_write_region", |mut caller: Caller<'_, HostState>,
        x: i32, y: i32, w: i32, h: i32, ptr: i32, len: i32| -> i32 {
        let data = match memory::read_bytes(&mut caller, ptr, len) {
            Some(d) => d,
            None => return -1,
        };
        let expected = (w * h * 4) as usize;
        if data.len() < expected {
            return -1;
        }
        if let Some(fb) = caller.data_mut().get_custom_mut::<SimFramebuffer>() {
            fb.write_region(x, y, w, h, &data);
            0
        } else {
            -1
        }
    })?;

    linker.func_wrap("env", "fb_clear", |mut caller: Caller<'_, HostState>,
        r: i32, g: i32, b: i32, a: i32| {
        if let Some(fb) = caller.data_mut().get_custom_mut::<SimFramebuffer>() {
            fb.clear(r as u8, g as u8, b as u8, a as u8);
        }
    })?;

    linker.func_wrap("env", "fb_flush", |mut caller: Caller<'_, HostState>| -> i32 {
        if let Some(fb) = caller.data_mut().get_custom_mut::<SimFramebuffer>() {
            let tiles = fb.flush();
            tiles
        } else {
            -1
        }
    })?;

    linker.func_wrap("env", "fb_blit_text", |mut caller: Caller<'_, HostState>,
        x: i32, y: i32, ptr: i32, len: i32,
        fg_r: i32, fg_g: i32, fg_b: i32,
        bg_r: i32, bg_g: i32, bg_b: i32| -> i32 {
        let text = match memory::read_string(&mut caller, ptr, len) {
            Some(s) => s,
            None => return -1,
        };
        if let Some(fb) = caller.data_mut().get_custom_mut::<SimFramebuffer>() {
            let char_w = 8i32;
            let char_h = 16i32;
            for (i, ch) in text.chars().enumerate() {
                let cx = x + i as i32 * char_w;
                fb.fill_rect(cx, y, char_w, char_h, bg_r as u8, bg_g as u8, bg_b as u8, 255);
                if ch != ' ' {
                    fb.fill_rect(cx + 1, y + 2, char_w - 2, char_h - 4, fg_r as u8, fg_g as u8, fg_b as u8, 255);
                }
            }
            text.len() as i32
        } else {
            -1
        }
    })?;

    Ok(())
}
