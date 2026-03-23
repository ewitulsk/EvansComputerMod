//! Terminal I/O host functions.

use wasmtime::*;
use crate::wasm_host::HostState;
use super::memory;

/// Host function names registered by this module.
pub const FUNCTIONS: &[&str] = &[
    "terminal_write",
    "terminal_clear",
    "terminal_set_cursor",
    "terminal_get_width",
    "terminal_get_height",
    "terminal_read_line",
    "open_visual_editor",
];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap("env", "terminal_write", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> i32 {
        let text = match memory::read_string(&mut caller, ptr, len) {
            Some(s) => s,
            None => return -1,
        };
        caller.data_mut().terminal.write_text(&text);
        let _ = caller.data_mut().terminal.render();
        len
    })?;

    linker.func_wrap("env", "terminal_clear", |mut caller: Caller<'_, HostState>| {
        caller.data_mut().terminal.clear();
        let _ = caller.data_mut().terminal.render();
    })?;

    linker.func_wrap("env", "terminal_set_cursor", |mut caller: Caller<'_, HostState>, x: i32, y: i32| {
        caller.data_mut().terminal.set_cursor(x, y);
        let _ = caller.data_mut().terminal.render();
    })?;

    linker.func_wrap("env", "terminal_get_width", |caller: Caller<'_, HostState>| -> i32 {
        caller.data().terminal.width as i32
    })?;

    linker.func_wrap("env", "terminal_get_height", |caller: Caller<'_, HostState>| -> i32 {
        caller.data().terminal.height as i32
    })?;

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

    linker.func_wrap("env", "open_visual_editor", |mut caller: Caller<'_, HostState>| {
        caller.data_mut().terminal.write_text("[Visual editor not available in simulator]\n");
        let _ = caller.data_mut().terminal.render();
    })?;

    Ok(())
}
