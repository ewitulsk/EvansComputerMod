//! All host function implementations registered on the wasmtime Linker.

use wasmtime::*;
use crate::wasm_host::HostState;

/// The default WASM module name for imports.
/// The Rust OS declares `extern "C"` functions which compile to the "env" module.
const MODULE: &str = "env";

/// Names of all host functions we register (used to skip them in stub generation).
pub const KNOWN_FUNCTION_NAMES: &[&str] = &[
    "terminal_write",
    "terminal_clear",
    "terminal_set_cursor",
    "terminal_get_width",
    "terminal_get_height",
    "terminal_read_line",
    "open_visual_editor",
    "file_write",
    "file_read",
    "file_size",
    "file_exists",
    "file_delete",
    "file_list",
    "redstone_set_output",
    "redstone_get_input",
    "redstone_get_all_input",
    "sleep_ms",
    "interrupt_poll",
    "interrupt_poll_len",
    "peripheral_list",
    "peripheral_get_methods",
    "peripheral_call",
    "__getrandom_v03_custom",
];

/// Register all known host functions on the linker.
pub fn register_all(linker: &mut Linker<HostState>) -> Result<()> {
    // === Terminal I/O ===
    linker.func_wrap(MODULE, "terminal_write", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> i32 {
        let text = match read_string(&mut caller, ptr, len) {
            Some(s) => s,
            None => return -1,
        };
        caller.data_mut().terminal.write_text(&text);
        // Render immediately
        let _ = caller.data_mut().terminal.render();
        len
    })?;

    linker.func_wrap(MODULE, "terminal_clear", |mut caller: Caller<'_, HostState>| {
        caller.data_mut().terminal.clear();
        let _ = caller.data_mut().terminal.render();
    })?;

    linker.func_wrap(MODULE, "terminal_set_cursor", |mut caller: Caller<'_, HostState>, x: i32, y: i32| {
        caller.data_mut().terminal.set_cursor(x, y);
        let _ = caller.data_mut().terminal.render();
    })?;

    linker.func_wrap(MODULE, "terminal_get_width", |caller: Caller<'_, HostState>| -> i32 {
        caller.data().terminal.width as i32
    })?;

    linker.func_wrap(MODULE, "terminal_get_height", |caller: Caller<'_, HostState>| -> i32 {
        caller.data().terminal.height as i32
    })?;

    linker.func_wrap(MODULE, "terminal_read_line", |mut caller: Caller<'_, HostState>, _prompt_ptr: i32, _prompt_len: i32, buf_ptr: i32, buf_len: i32| -> i32 {
        // Block until we get a complete line from the input channel
        let input_rx = caller.data().input_rx.clone();
        let shutdown = caller.data().shutdown.clone();
        let interrupt_queue = caller.data().interrupt_queue.clone();

        let mut line_buf = String::new();
        loop {
            // Check for shutdown
            if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                return -1;
            }

            // Check for interrupts (terminate)
            if let Some(evt) = interrupt_queue.pop() {
                if evt.irq == crate::interrupts::IRQ_TERMINATE {
                    return -2;
                }
                // Re-queue non-terminate interrupts for later
                interrupt_queue.push(evt.irq, evt.payload);
            }

            // Try to receive input with timeout
            match input_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(ch_str) => {
                    for ch in ch_str.chars() {
                        match ch {
                            '\n' | '\r' => {
                                // Line complete
                                let bytes = line_buf.as_bytes();
                                let write_len = bytes.len().min(buf_len as usize);
                                if write_len > 0 {
                                    write_bytes(&mut caller, buf_ptr, &bytes[..write_len]);
                                }
                                return write_len as i32;
                            }
                            '\x08' | '\x7f' => {
                                // Backspace
                                line_buf.pop();
                            }
                            '\x14' => {
                                // Ctrl+T - interrupted
                                return -2;
                            }
                            c if c >= ' ' => {
                                line_buf.push(c);
                            }
                            _ => {}
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return -1,
            }
        }
    })?;

    linker.func_wrap(MODULE, "open_visual_editor", |mut caller: Caller<'_, HostState>| {
        caller.data_mut().terminal.write_text("[Visual editor not available in simulator]\n");
        let _ = caller.data_mut().terminal.render();
    })?;

    // === File System ===
    linker.func_wrap(MODULE, "file_write", |mut caller: Caller<'_, HostState>, path_ptr: i32, path_len: i32, data_ptr: i32, data_len: i32| -> i32 {
        let filename = match read_string(&mut caller, path_ptr, path_len) {
            Some(s) => s,
            None => return -1,
        };
        let data = match read_bytes(&mut caller, data_ptr, data_len) {
            Some(d) => d,
            None => return -1,
        };
        caller.data().filesystem.write_file(&filename, &data)
    })?;

    linker.func_wrap(MODULE, "file_read", |mut caller: Caller<'_, HostState>, path_ptr: i32, path_len: i32, buf_ptr: i32, buf_len: i32| -> i32 {
        let filename = match read_string(&mut caller, path_ptr, path_len) {
            Some(s) => s,
            None => return -1,
        };
        match caller.data().filesystem.read_file(&filename) {
            Some(data) => {
                let write_len = data.len().min(buf_len as usize);
                write_bytes(&mut caller, buf_ptr, &data[..write_len]);
                write_len as i32
            }
            None => -1,
        }
    })?;

    linker.func_wrap(MODULE, "file_size", |mut caller: Caller<'_, HostState>, path_ptr: i32, path_len: i32| -> i32 {
        match read_string(&mut caller, path_ptr, path_len) {
            Some(filename) => caller.data().filesystem.file_size(&filename),
            None => -1,
        }
    })?;

    linker.func_wrap(MODULE, "file_exists", |mut caller: Caller<'_, HostState>, path_ptr: i32, path_len: i32| -> i32 {
        match read_string(&mut caller, path_ptr, path_len) {
            Some(filename) => if caller.data().filesystem.file_exists(&filename) { 1 } else { 0 },
            None => 0,
        }
    })?;

    linker.func_wrap(MODULE, "file_delete", |mut caller: Caller<'_, HostState>, path_ptr: i32, path_len: i32| -> i32 {
        match read_string(&mut caller, path_ptr, path_len) {
            Some(filename) => if caller.data().filesystem.delete_file(&filename) { 1 } else { 0 },
            None => 0,
        }
    })?;

    linker.func_wrap(MODULE, "file_list", |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32| -> i32 {
        let listing = caller.data().filesystem.list_files();
        let bytes = listing.as_bytes();
        let write_len = bytes.len().min(buf_len as usize);
        if write_len > 0 {
            write_bytes(&mut caller, buf_ptr, &bytes[..write_len]);
        }
        write_len as i32
    })?;

    // === Redstone ===
    linker.func_wrap(MODULE, "redstone_set_output", |caller: Caller<'_, HostState>, side: i32, power: i32| -> i32 {
        caller.data().redstone.set_output(side, power)
    })?;

    linker.func_wrap(MODULE, "redstone_get_input", |caller: Caller<'_, HostState>, side: i32| -> i32 {
        caller.data().redstone.get_input(side)
    })?;

    linker.func_wrap(MODULE, "redstone_get_all_input", |mut caller: Caller<'_, HostState>, buf_ptr: i32| -> i32 {
        let inputs = caller.data().redstone.get_all_inputs();
        let mut bytes = [0u8; 24]; // 6 * 4 bytes
        for (i, &val) in inputs.iter().enumerate() {
            let le_bytes = val.to_le_bytes();
            bytes[i * 4..i * 4 + 4].copy_from_slice(&le_bytes);
        }
        write_bytes(&mut caller, buf_ptr, &bytes);
        0
    })?;

    // === Sleep ===
    linker.func_wrap(MODULE, "sleep_ms", |caller: Caller<'_, HostState>, ms: i32| {
        let ms = ms.clamp(0, 60_000); // Max 60 seconds like Java
        let shutdown = caller.data().shutdown.clone();
        // Sleep in small chunks to allow interrupt checking
        let mut remaining = ms;
        while remaining > 0 && !shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            let chunk = remaining.min(50);
            std::thread::sleep(std::time::Duration::from_millis(chunk as u64));
            remaining -= chunk;
        }
    })?;

    // === Interrupts ===
    linker.func_wrap(MODULE, "interrupt_poll", |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32| -> i32 {
        let evt = caller.data().interrupt_queue.pop();
        match evt {
            Some(event) => {
                let payload_bytes = event.payload.as_bytes();
                let write_len = payload_bytes.len().min(buf_len as usize);
                if write_len > 0 {
                    write_bytes(&mut caller, buf_ptr, &payload_bytes[..write_len]);
                }
                caller.data_mut().last_interrupt_payload_len = write_len as i32;
                event.irq
            }
            None => {
                caller.data_mut().last_interrupt_payload_len = 0;
                -1
            }
        }
    })?;

    linker.func_wrap(MODULE, "interrupt_poll_len", |caller: Caller<'_, HostState>| -> i32 {
        caller.data().last_interrupt_payload_len
    })?;

    // === Peripherals (stubs) ===
    linker.func_wrap(MODULE, "peripheral_list", |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32| -> i32 {
        let json = b"[]";
        let write_len = json.len().min(buf_len as usize);
        write_bytes(&mut caller, buf_ptr, &json[..write_len]);
        write_len as i32
    })?;

    linker.func_wrap(MODULE, "peripheral_get_methods", |mut caller: Caller<'_, HostState>, _name_ptr: i32, _name_len: i32, buf_ptr: i32, buf_len: i32| -> i32 {
        let json = b"[]";
        let write_len = json.len().min(buf_len as usize);
        write_bytes(&mut caller, buf_ptr, &json[..write_len]);
        write_len as i32
    })?;

    linker.func_wrap(MODULE, "peripheral_call", |_caller: Caller<'_, HostState>, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 {
        -1
    })?;

    // === Custom getrandom ===
    linker.func_wrap(MODULE, "__getrandom_v03_custom", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> i32 {
        if len <= 0 || len > 4096 {
            return -1;
        }
        use rand::RngCore;
        let mut bytes = vec![0u8; len as usize];
        rand::thread_rng().fill_bytes(&mut bytes);
        write_bytes(&mut caller, ptr, &bytes);
        0
    })?;

    Ok(())
}

/// Read a UTF-8 string from WASM memory.
fn read_string(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> Option<String> {
    if len <= 0 || len > 65536 {
        return None;
    }
    let memory = caller.get_export("memory")?.into_memory()?;
    let data = memory.data(&caller);
    let start = ptr as usize;
    let end = start + len as usize;
    if end > data.len() {
        return None;
    }
    String::from_utf8(data[start..end].to_vec()).ok()
}

/// Read raw bytes from WASM memory.
fn read_bytes(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> Option<Vec<u8>> {
    if len < 0 || len > 1024 * 1024 {
        return None;
    }
    let memory = caller.get_export("memory")?.into_memory()?;
    let data = memory.data(&caller);
    let start = ptr as usize;
    let end = start + len as usize;
    if end > data.len() {
        return None;
    }
    Some(data[start..end].to_vec())
}

/// Write bytes to WASM memory.
fn write_bytes(caller: &mut Caller<'_, HostState>, ptr: i32, bytes: &[u8]) {
    if let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory()) {
        let data = memory.data_mut(caller);
        let start = ptr as usize;
        let end = start + bytes.len();
        if end <= data.len() {
            data[start..end].copy_from_slice(bytes);
        }
    }
}
