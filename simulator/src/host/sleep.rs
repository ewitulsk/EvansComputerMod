//! Sleep and time host functions.

use wasmtime::*;
use crate::wasm_host::HostState;
use crate::terminal_io::FB_BASE;

/// Host function names registered by this module.
pub const FUNCTIONS: &[&str] = &["sleep_ms", "get_time_ms"];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap("env", "sleep_ms", |mut caller: Caller<'_, HostState>, ms: i32| {
        let ms = ms.clamp(0, 60_000);
        let shutdown = caller.data().shutdown.clone();
        let mut remaining = ms;
        while remaining > 0 && !shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            let chunk = remaining.min(50);
            std::thread::sleep(std::time::Duration::from_millis(chunk as u64));
            remaining -= chunk;

            // Render the framebuffer during sleep so display updates appear
            // immediately during blocking loops (sshd accept, tcp_connect, etc.)
            render_during_sleep(&mut caller);
        }
    })?;

    linker.func_wrap("env", "get_time_ms", |_caller: Caller<'_, HostState>| -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    })?;

    Ok(())
}

/// Read the framebuffer from WASM memory and render it.
/// Called during sleep_ms to keep the display updated during blocking WASM calls.
fn render_during_sleep(caller: &mut Caller<'_, HostState>) {
    let memory = match caller.get_export("memory") {
        Some(Extern::Memory(m)) => m,
        _ => return,
    };

    let data = memory.data(&*caller);
    if data.len() < FB_BASE + 16 { return; }

    let width = u16::from_le_bytes([data[FB_BASE + 2], data[FB_BASE + 3]]) as usize;
    let height = u16::from_le_bytes([data[FB_BASE + 4], data[FB_BASE + 5]]) as usize;
    if width == 0 || height == 0 { return; }

    let fb_size = 64 + width * height * 4;
    let end = FB_BASE + fb_size;
    if data.len() < end { return; }

    let fb_copy = data[FB_BASE..end].to_vec();

    let renderer = &mut caller.data_mut().renderer;
    let _ = renderer.render_from_fb_slice(&fb_copy);
}
