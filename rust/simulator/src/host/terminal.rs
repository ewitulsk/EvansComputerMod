//! Terminal host functions.
//!
//! Most terminal output functions (terminal_write, terminal_clear, terminal_set_cursor,
//! terminal_get_width, terminal_get_height) have been removed. The WASM OS now writes
//! directly to a memory-mapped framebuffer, and the host reads it.
//!
//! Remaining functions:
//! - terminal_read_line: blocking line input (host cooperation needed)
//! - fb_sync: hint to host to read the framebuffer immediately
//! - open_visual_editor: stub

use wasmtime::*;
use crate::wasm_host::HostState;
use super::memory;

/// Host function names registered by this module.
pub const FUNCTIONS: &[&str] = &[
    "terminal_read_line",
    "fb_sync",
    "open_visual_editor",
];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    // terminal_read_line: blocking line input
    linker.func_wrap("env", "terminal_read_line", |mut caller: Caller<'_, HostState>, _prompt_ptr: i32, _prompt_len: i32, buf_ptr: i32, buf_len: i32| -> i32 {
        let input_rx = caller.data().input_rx.clone();
        let shutdown = caller.data().shutdown.clone();
        let interrupt_queue = caller.data().interrupt_queue.clone();

        let mut line_buf = String::new();
        loop {
            if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                return -1;
            }

            if let Some(evt) = interrupt_queue.pop() {
                if evt.irq == crate::interrupts::IRQ_TERMINATE {
                    return -2;
                }
                interrupt_queue.push(evt.irq, evt.payload);
            }

            match input_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(ch_str) => {
                    for ch in ch_str.chars() {
                        match ch {
                            '\n' | '\r' => {
                                let bytes = line_buf.as_bytes();
                                let write_len = bytes.len().min(buf_len as usize);
                                if write_len > 0 {
                                    memory::write_bytes(&mut caller, buf_ptr, &bytes[..write_len]);
                                }
                                return write_len as i32;
                            }
                            '\x08' | '\x7f' => { line_buf.pop(); }
                            '\x14' => { return -2; }
                            c if c >= ' ' => { line_buf.push(c); }
                            _ => {}
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return -1,
            }
        }
    })?;

    // fb_sync: hint to render the framebuffer now
    linker.func_wrap("env", "fb_sync", |mut caller: Caller<'_, HostState>| {
        // The actual rendering happens in the worker loop after this call returns.
        caller.data_mut().force_render = true;
    })?;

    // open_visual_editor: stub
    linker.func_wrap("env", "open_visual_editor", |_caller: Caller<'_, HostState>| {
        eprintln!("[Simulator] Visual editor not available");
    })?;

    Ok(())
}
