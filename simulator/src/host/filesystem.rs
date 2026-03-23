//! File system host functions.

use wasmtime::*;
use crate::wasm_host::HostState;
use super::memory;

/// Host function names registered by this module.
pub const FUNCTIONS: &[&str] = &[
    "file_write",
    "file_read",
    "file_size",
    "file_exists",
    "file_delete",
    "file_list",
];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap("env", "file_write", |mut caller: Caller<'_, HostState>, path_ptr: i32, path_len: i32, data_ptr: i32, data_len: i32| -> i32 {
        let filename = match memory::read_string(&mut caller, path_ptr, path_len) {
            Some(s) => s,
            None => return -1,
        };
        let data = match memory::read_bytes(&mut caller, data_ptr, data_len) {
            Some(d) => d,
            None => return -1,
        };
        caller.data().filesystem.write_file(&filename, &data)
    })?;

    linker.func_wrap("env", "file_read", |mut caller: Caller<'_, HostState>, path_ptr: i32, path_len: i32, buf_ptr: i32, buf_len: i32| -> i32 {
        let filename = match memory::read_string(&mut caller, path_ptr, path_len) {
            Some(s) => s,
            None => return -1,
        };
        match caller.data().filesystem.read_file(&filename) {
            Some(data) => {
                let write_len = data.len().min(buf_len as usize);
                memory::write_bytes(&mut caller, buf_ptr, &data[..write_len]);
                write_len as i32
            }
            None => -1,
        }
    })?;

    linker.func_wrap("env", "file_size", |mut caller: Caller<'_, HostState>, path_ptr: i32, path_len: i32| -> i32 {
        match memory::read_string(&mut caller, path_ptr, path_len) {
            Some(filename) => caller.data().filesystem.file_size(&filename),
            None => -1,
        }
    })?;

    linker.func_wrap("env", "file_exists", |mut caller: Caller<'_, HostState>, path_ptr: i32, path_len: i32| -> i32 {
        match memory::read_string(&mut caller, path_ptr, path_len) {
            Some(filename) => if caller.data().filesystem.file_exists(&filename) { 1 } else { 0 },
            None => 0,
        }
    })?;

    linker.func_wrap("env", "file_delete", |mut caller: Caller<'_, HostState>, path_ptr: i32, path_len: i32| -> i32 {
        match memory::read_string(&mut caller, path_ptr, path_len) {
            Some(filename) => if caller.data().filesystem.delete_file(&filename) { 1 } else { 0 },
            None => 0,
        }
    })?;

    linker.func_wrap("env", "file_list", |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32| -> i32 {
        let listing = caller.data().filesystem.list_files();
        let bytes = listing.as_bytes();
        let write_len = bytes.len().min(buf_len as usize);
        if write_len > 0 {
            memory::write_bytes(&mut caller, buf_ptr, &bytes[..write_len]);
        }
        write_len as i32
    })?;

    Ok(())
}
