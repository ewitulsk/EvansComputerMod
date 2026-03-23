//! Interrupt polling host functions.

use wasmtime::*;
use crate::wasm_host::HostState;
use super::memory;

/// Host function names registered by this module.
pub const FUNCTIONS: &[&str] = &[
    "interrupt_poll",
    "interrupt_poll_len",
];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap("env", "interrupt_poll", |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32| -> i32 {
        let evt = caller.data().interrupt_queue.pop();
        match evt {
            Some(event) => {
                let payload_bytes = event.payload.as_bytes();
                let write_len = payload_bytes.len().min(buf_len as usize);
                if write_len > 0 {
                    memory::write_bytes(&mut caller, buf_ptr, &payload_bytes[..write_len]);
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

    linker.func_wrap("env", "interrupt_poll_len", |caller: Caller<'_, HostState>| -> i32 {
        caller.data().last_interrupt_payload_len
    })?;

    Ok(())
}
