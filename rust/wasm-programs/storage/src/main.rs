//! `storage` — browse the items this computer can reach and request them.
//!
//! ```text
//! storage            on the terminal (mouse + keyboard; Esc quits)
//! storage --screen   on the attached Screen cluster (right-click to tap, sneak for the right button;
//!                    full colour when it fits, --indexed forces the 256-colour palette)
//! ```
//!
//! Items are drawn by the Minecraft client over the program's pixels (an
//! item overlay), so every mod's items show with their real icons. Clicking
//! an item sends it out of a decoder in reach: left-click one, shift-click a
//! stack, right-click (or tap, on a Screen) for the request dialog.
//!
//! It only uses the public storage API (`ecm_host_abi::storage`), like any
//! player program could.

mod app;

#[cfg(target_arch = "wasm32")]
mod host {
    use crate::app::{App, Backend};
    use ecm_host_abi::gfx_child::{self, OverlayItem, FORMAT_INDEXED8, FORMAT_RGBA8888};
    use ecm_host_abi::mouse::{self, source, MouseKind};
    use ecm_host_abi::peripheral;
    use ecm_host_abi::storage::{Cell, Item, Port, Sort, Storage, TokenInfo};
    use ecm_host_abi::video::Target;
    use ecm_ui::{InputEvent, Key, Modifiers, MouseButton};
    use std::io::Read;
    use std::time::{Duration, Instant};

    const TERMINAL_W: i32 = 320;
    const TERMINAL_H: i32 = 200;
    /// Bytes of a Screen cluster's framebuffer region (kernel `SCREEN_CAP`).
    const SCREEN_CAP: i64 = 0x10_0000;
    const REFRESH_EVERY: Duration = Duration::from_secs(5);

    struct Net(Storage);

    fn err(e: peripheral::Error) -> String {
        match e {
            peripheral::Error::Peripheral(m) => m,
            peripheral::Error::Unavailable => "storage unavailable".to_string(),
            peripheral::Error::Malformed(m) => m.to_string(),
        }
    }

    impl Backend for Net {
        fn items(&mut self, query: &str, sort: Sort) -> Result<Vec<Item>, String> {
            let q = if query.is_empty() { None } else { Some(query) };
            self.0.items(q, sort, 0, 5000).map_err(err)
        }
        fn cells(&mut self) -> Result<Vec<Cell>, String> {
            self.0.cells().map_err(err)
        }
        fn decoders(&mut self) -> Result<Vec<Port>, String> {
            Ok(self.0.ports().map_err(err)?.into_iter().filter(|p| p.kind == "decoder").collect())
        }
        fn extract(&mut self, key: &str, count: i64, decoder: Option<&str>) -> Result<i64, String> {
            self.0.extract(key, count, decoder).map_err(err)
        }
        fn tokens(&mut self) -> Result<Vec<TokenInfo>, String> {
            self.0.tokens_issued().map_err(err)
        }
        fn lost(&mut self) -> Result<Vec<TokenInfo>, String> {
            self.0.lost().map_err(err)
        }
        fn claim_lost(&mut self) -> Result<i64, String> {
            self.0.claim_lost().map_err(err)
        }
    }

    #[link(wasm_import_module = "wasi_snapshot_preview1")]
    extern "C" {
        fn fd_fdstat_set_flags(fd: u32, flags: u32) -> u32;
    }

    /// Keyboard bytes (from the terminal) to UI events; keeps a partial escape sequence.
    #[derive(Default)]
    struct Keys {
        pending: Vec<u8>,
    }

    impl Keys {
        fn feed(&mut self, bytes: &[u8], out: &mut Vec<InputEvent>) {
            self.pending.extend_from_slice(bytes);
            let mut i = 0;
            let b = std::mem::take(&mut self.pending);
            while i < b.len() {
                let c = b[i];
                if c == 0x1b {
                    if i + 1 >= b.len() {
                        out.push(InputEvent::Key(Key::Escape));
                        i += 1;
                        continue;
                    }
                    if b[i + 1] == b'[' {
                        let mut j = i + 2;
                        while j < b.len() && !(0x40..=0x7e).contains(&b[j]) {
                            j += 1;
                        }
                        if j >= b.len() {
                            self.pending.extend_from_slice(&b[i..]);
                            return;
                        }
                        let key = match (&b[i + 2..j], b[j]) {
                            (_, b'A') => Some(Key::Up),
                            (_, b'B') => Some(Key::Down),
                            (_, b'C') => Some(Key::Right),
                            (_, b'D') => Some(Key::Left),
                            (_, b'H') => Some(Key::Home),
                            (_, b'F') => Some(Key::End),
                            (b"3", b'~') => Some(Key::Delete),
                            (b"5", b'~') => Some(Key::PageUp),
                            (b"6", b'~') => Some(Key::PageDown),
                            (b"1", b'~') | (b"7", b'~') => Some(Key::Home),
                            (b"4", b'~') | (b"8", b'~') => Some(Key::End),
                            _ => None,
                        };
                        if let Some(k) = key {
                            out.push(InputEvent::Key(k));
                        }
                        i = j + 1;
                        continue;
                    }
                    out.push(InputEvent::Key(Key::Escape));
                    i += 1;
                    continue;
                }
                match c {
                    b'\r' | b'\n' => out.push(InputEvent::Key(Key::Enter)),
                    0x7f | 0x08 => out.push(InputEvent::Key(Key::Backspace)),
                    b'\t' => out.push(InputEvent::Key(Key::Tab)),
                    0x20..=0x7e => out.push(InputEvent::Char(c as char)),
                    _ => {}
                }
                i += 1;
            }
        }
    }

    fn button(code: u8) -> MouseButton {
        match code {
            1 => MouseButton::Right,
            2 => MouseButton::Middle,
            _ => MouseButton::Left,
        }
    }

    /// RGBA to the host's indexed palette: packed 2-3-3 (`RRGGGBBB`, FFmpeg's RGB8).
    fn to_indexed(px: &[u8]) -> Vec<u8> {
        px.chunks_exact(4).map(|p| ((p[0] >> 6) << 6) | ((p[1] >> 5) << 3) | (p[2] >> 5)).collect()
    }

    pub fn run() {
        let screen = std::env::args().any(|a| a == "--screen" || a == "screen");
        let target = if screen { Target::Screen } else { Target::Terminal };

        let Some(net) = Storage::find() else {
            eprintln!("storage: no storage peripheral attached.");
            eprintln!("  Put a Drive next to this computer, a Storage Module in a bay,");
            eprintln!("  or wire a Drive to a Wired Bus Module.");
            std::process::exit(1);
        };

        let (w, h, format) = if screen {
            gfx_child::set_screen_power(true);
            let Some((sw, sh)) = gfx_child::screen_dims() else {
                eprintln!("storage: no Screen cluster attached to this computer");
                std::process::exit(1);
            };
            let (sw, sh) = (sw as i32, sh as i32);
            let rgba = !std::env::args().any(|a| a == "--indexed") && 0x400 + sw as i64 * sh as i64 * 4 <= SCREEN_CAP;
            (sw, sh, if rgba { FORMAT_RGBA8888 } else { FORMAT_INDEXED8 })
        } else {
            (TERMINAL_W, TERMINAL_H, FORMAT_INDEXED8)
        };
        if gfx_child::init(target, w, h).is_err() {
            eprintln!("storage: can't open the display");
            std::process::exit(1);
        }
        if screen && format == FORMAT_RGBA8888 {
            gfx_child::set_screen_pixel_format(FORMAT_RGBA8888);
        }
        let _ = gfx_child::set_mode(target, 1);
        if !mouse::enable() {
            eprintln!("storage: mouse input isn't available (open the terminal screen, or attach a Screen)");
        }
        unsafe {
            fd_fdstat_set_flags(0, 4); // stdin non-blocking
        }
        if !screen {
            println!("storage: Esc to quit");
        }

        let mut backend = Net(net);
        let mut app = App::new(w, h, screen);
        app.refresh(&mut backend);
        let mut keys = Keys::default();
        let mut last_refresh = Instant::now();
        let started = Instant::now();
        let mut stdin_buf = [0u8; 256];
        let mut first = true;
        let mut blink_at = Instant::now();

        loop {
            let mut events = Vec::new();
            while let Some(ev) = mouse::poll() {
                let (x, y) = (ev.x as i32, ev.y as i32);
                if ev.source == source::TERMINAL {
                    let mods = Modifiers {
                        shift: ev.modifiers & mouse::modifier::SHIFT != 0,
                        ctrl: ev.modifiers & mouse::modifier::CTRL != 0,
                    };
                    if mods != app.ui.modifiers {
                        events.push(InputEvent::Modifiers(mods));
                    }
                }
                match ev.kind {
                    MouseKind::Move => events.push(InputEvent::MouseMove { x, y }),
                    MouseKind::Down => {
                        events.push(InputEvent::MouseMove { x, y });
                        events.push(InputEvent::MouseDown { x, y, button: button(ev.button_code) });
                    }
                    MouseKind::Up => events.push(InputEvent::MouseUp { x, y, button: button(ev.button_code) }),
                    MouseKind::Scroll => events.push(InputEvent::Scroll { x, y, delta: ev.scroll_dir as i32 }),
                    MouseKind::Other => {}
                }
            }
            loop {
                match std::io::stdin().read(&mut stdin_buf) {
                    Ok(0) => break,
                    Ok(n) => keys.feed(&stdin_buf[..n], &mut events),
                    Err(_) => break,
                }
            }
            let mut changed = false;
            for _ in 0..32 {
                match peripheral::wait_event(Some("storage_changed"), 0) {
                    Ok(Some(_)) => changed = true,
                    _ => break,
                }
            }
            if changed || last_refresh.elapsed() >= REFRESH_EVERY {
                app.refresh(&mut backend);
                last_refresh = Instant::now();
            }

            let blink = blink_at.elapsed() >= Duration::from_millis(500);
            if first || !events.is_empty() || app.dirty || blink {
                if blink {
                    blink_at = Instant::now();
                }
                app.ui.time_ms = Some(started.elapsed().as_millis() as u64);
                let mut out = app.frame(&events, &mut backend);
                let mut guard = 0;
                while out.redraw_now && guard < 4 {
                    out = app.frame(&[], &mut backend);
                    guard += 1;
                }
                for r in app.canvas.take_dirty_rects(16) {
                    let px = app.canvas.pixels_of(r);
                    let data = if format == FORMAT_INDEXED8 { to_indexed(&px) } else { px };
                    if gfx_child::blit_rect(target, r.x, r.y, r.w, r.h, &data, format).is_err() {
                        return finish(target, screen);
                    }
                }
                if out.items_changed || first {
                    let list: Vec<OverlayItem> = out
                        .items
                        .iter()
                        .map(|p| OverlayItem {
                            x: p.x,
                            y: p.y,
                            size: p.size,
                            clip: (p.clip.x, p.clip.y, p.clip.w, p.clip.h),
                            flags: p.flags,
                            item: &p.item,
                            label: &p.label,
                        })
                        .collect();
                    let _ = gfx_child::items_set(target, &list);
                }
                first = false;
            }
            if app.quit {
                break;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        finish(target, screen);
    }

    fn finish(target: Target, screen: bool) {
        let _ = gfx_child::items_set(target, &[]);
        mouse::disable();
        if screen {
            gfx_child::set_screen_pixel_format(FORMAT_INDEXED8);
        } else {
            let _ = gfx_child::set_mode(target, 0);
        }
    }
}

fn main() {
    #[cfg(target_arch = "wasm32")]
    host::run();
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("storage runs on an in-game computer");
}
