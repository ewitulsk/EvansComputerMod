//! Custom getrandom host function for WASM.

use wasmtime::*;
use crate::wasm_host::HostState;
use super::memory;

/// Host function names registered by this module.
pub const FUNCTIONS: &[&str] = &["__getrandom_v03_custom"];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap("env", "__getrandom_v03_custom", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> i32 {
        if len <= 0 || len > 4096 {
            return -1;
        }
        use rand::RngCore;
        let mut bytes = vec![0u8; len as usize];
        rand::thread_rng().fill_bytes(&mut bytes);
        memory::write_bytes(&mut caller, ptr, &bytes);
        0
    })?;

    Ok(())
}
