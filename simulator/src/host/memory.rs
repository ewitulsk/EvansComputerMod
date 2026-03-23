//! WASM memory helpers for reading/writing data between host and guest.
//!
//! These are the building blocks for any host function that needs to exchange
//! strings or byte buffers with the WASM module. Use these instead of working
//! with wasmtime's memory API directly.
//!
//! # Examples
//!
//! ```rust,ignore
//! use crate::host::memory;
//!
//! // Reading a string argument from WASM
//! let name = match memory::read_string(&mut caller, name_ptr, name_len) {
//!     Some(s) => s,
//!     None => return -1,
//! };
//!
//! // Writing a response back to WASM
//! let response = b"hello";
//! memory::write_bytes(&mut caller, buf_ptr, response);
//! ```

use wasmtime::*;
use crate::wasm_host::HostState;

/// Read a UTF-8 string from WASM linear memory.
///
/// Returns `None` if the pointer/length is out of bounds, the length exceeds
/// 64 KiB, or the bytes are not valid UTF-8.
pub fn read_string(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> Option<String> {
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

/// Read raw bytes from WASM linear memory.
///
/// Returns `None` if the pointer/length is out of bounds or the length
/// exceeds 1 MiB.
pub fn read_bytes(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> Option<Vec<u8>> {
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

/// Write bytes into WASM linear memory at the given pointer.
///
/// Silently does nothing if the memory export is missing or the write
/// would go out of bounds.
pub fn write_bytes(caller: &mut Caller<'_, HostState>, ptr: i32, bytes: &[u8]) {
    if let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory()) {
        let data = memory.data_mut(caller);
        let start = ptr as usize;
        let end = start + bytes.len();
        if end <= data.len() {
            data[start..end].copy_from_slice(bytes);
        }
    }
}
