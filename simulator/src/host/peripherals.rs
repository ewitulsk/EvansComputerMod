//! CC:Tweaked peripheral host function stubs.
//!
//! Returns empty results since there are no Minecraft peripherals in the simulator.

use wasmtime::*;
use crate::wasm_host::HostState;
use super::memory;

/// Host function names registered by this module.
pub const FUNCTIONS: &[&str] = &[
    "peripheral_list",
    "peripheral_get_methods",
    "peripheral_call",
];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap("env", "peripheral_list", |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32| -> i32 {
        let json = b"[]";
        let write_len = json.len().min(buf_len as usize);
        memory::write_bytes(&mut caller, buf_ptr, &json[..write_len]);
        write_len as i32
    })?;

    linker.func_wrap("env", "peripheral_get_methods", |mut caller: Caller<'_, HostState>, _name_ptr: i32, _name_len: i32, buf_ptr: i32, buf_len: i32| -> i32 {
        let json = b"[]";
        let write_len = json.len().min(buf_len as usize);
        memory::write_bytes(&mut caller, buf_ptr, &json[..write_len]);
        write_len as i32
    })?;

    linker.func_wrap("env", "peripheral_call", |_caller: Caller<'_, HostState>, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 {
        -1
    })?;

    Ok(())
}
