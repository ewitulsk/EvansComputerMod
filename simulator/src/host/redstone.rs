//! Redstone host functions.

use wasmtime::*;
use crate::wasm_host::HostState;
use super::memory;

/// Host function names registered by this module.
pub const FUNCTIONS: &[&str] = &[
    "redstone_set_output",
    "redstone_get_input",
    "redstone_get_all_input",
];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap("env", "redstone_set_output", |caller: Caller<'_, HostState>, side: i32, power: i32| -> i32 {
        caller.data().redstone.set_output(side, power)
    })?;

    linker.func_wrap("env", "redstone_get_input", |caller: Caller<'_, HostState>, side: i32| -> i32 {
        caller.data().redstone.get_input(side)
    })?;

    linker.func_wrap("env", "redstone_get_all_input", |mut caller: Caller<'_, HostState>, buf_ptr: i32| -> i32 {
        let inputs = caller.data().redstone.get_all_inputs();
        let mut bytes = [0u8; 24];
        for (i, &val) in inputs.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&val.to_le_bytes());
        }
        memory::write_bytes(&mut caller, buf_ptr, &bytes);
        0
    })?;

    Ok(())
}
