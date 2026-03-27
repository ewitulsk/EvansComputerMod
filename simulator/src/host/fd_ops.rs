//! File descriptor host functions for the kernel.

use wasmtime::*;
use crate::wasm_host::HostState;
use crate::fd::{FdTable, create_pipe, VfsFileFd, NullFd};
use super::memory;

pub const FUNCTIONS: &[&str] = &[
    "fd_open",
    "fd_read",
    "fd_write",
    "fd_close",
    "fd_dup",
    "fd_dup2",
    "pipe_create",
];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    // fd_open(path_ptr, path_len, flags) -> fd or -1
    linker.func_wrap("env", "fd_open",
        |mut caller: Caller<'_, HostState>, path_ptr: i32, path_len: i32, flags: i32| -> i32 {
            let path = match memory::read_string(&mut caller, path_ptr, path_len) {
                Some(s) => s,
                None => return -1,
            };

            let storage_path = caller.data().filesystem.storage_path().to_path_buf();

            let fd_table = match caller.data_mut().get_custom_mut::<FdTable>() {
                Some(t) => t,
                None => return -1,
            };

            match VfsFileFd::open(&storage_path, &path, flags) {
                Ok(vfs_fd) => fd_table.allocate(Box::new(vfs_fd)),
                Err(_) => -1,
            }
        },
    )?;

    // fd_read(fd, buf_ptr, buf_len) -> bytes_read, 0=EOF, -1=error
    linker.func_wrap("env", "fd_read",
        |mut caller: Caller<'_, HostState>, fd: i32, buf_ptr: i32, buf_len: i32| -> i32 {
            // First, read from the FD into a temp buffer, then drop the borrow,
            // then write to WASM memory.
            let read_result = {
                let fd_table = match caller.data_mut().get_custom_mut::<FdTable>() {
                    Some(t) => t,
                    None => return -1,
                };

                let descriptor = match fd_table.get_mut(fd) {
                    Some(d) => d,
                    None => return -1,
                };

                let mut temp_buf = vec![0u8; buf_len as usize];
                match descriptor.read(&mut temp_buf) {
                    Ok(n) => Ok((temp_buf, n)),
                    Err(_) => Err(()),
                }
            };

            match read_result {
                Ok((buf, n)) => {
                    if n > 0 {
                        memory::write_bytes(&mut caller, buf_ptr, &buf[..n]);
                    }
                    n as i32
                }
                Err(_) => -1,
            }
        },
    )?;

    // fd_write(fd, buf_ptr, buf_len) -> bytes_written or -1
    linker.func_wrap("env", "fd_write",
        |mut caller: Caller<'_, HostState>, fd: i32, buf_ptr: i32, buf_len: i32| -> i32 {
            // Read data from WASM memory first, then write to FD.
            let data = match memory::read_bytes(&mut caller, buf_ptr, buf_len) {
                Some(d) => d,
                None => return -1,
            };

            let fd_table = match caller.data_mut().get_custom_mut::<FdTable>() {
                Some(t) => t,
                None => return -1,
            };

            let descriptor = match fd_table.get_mut(fd) {
                Some(d) => d,
                None => return -1,
            };

            match descriptor.write(&data) {
                Ok(n) => n as i32,
                Err(_) => -1,
            }
        },
    )?;

    // fd_close(fd) -> 0 ok, -1 error
    linker.func_wrap("env", "fd_close",
        |mut caller: Caller<'_, HostState>, fd: i32| -> i32 {
            let fd_table = match caller.data_mut().get_custom_mut::<FdTable>() {
                Some(t) => t,
                None => return -1,
            };
            if fd_table.close(fd) { 0 } else { -1 }
        },
    )?;

    // fd_dup(old_fd) -> new_fd or -1
    // NOTE: Simplified dup - allocates a NullFd placeholder.
    // Full dup with cloneable FDs will be added later.
    linker.func_wrap("env", "fd_dup",
        |mut caller: Caller<'_, HostState>, _old_fd: i32| -> i32 {
            let fd_table = match caller.data_mut().get_custom_mut::<FdTable>() {
                Some(t) => t,
                None => return -1,
            };
            fd_table.allocate(Box::new(NullFd))
        },
    )?;

    // fd_dup2(old_fd, new_fd) -> 0 ok, -1 error
    linker.func_wrap("env", "fd_dup2",
        |mut caller: Caller<'_, HostState>, _old_fd: i32, new_fd: i32| -> i32 {
            let fd_table = match caller.data_mut().get_custom_mut::<FdTable>() {
                Some(t) => t,
                None => return -1,
            };
            // Close new_fd if open, then insert a NullFd placeholder
            fd_table.close(new_fd);
            fd_table.insert_at(new_fd, Box::new(NullFd));
            0
        },
    )?;

    // pipe_create(read_fd_ptr, write_fd_ptr) -> 0 ok, -1 error
    linker.func_wrap("env", "pipe_create",
        |mut caller: Caller<'_, HostState>, read_fd_ptr: i32, write_fd_ptr: i32| -> i32 {
            let (read_end, write_end) = create_pipe();

            // Allocate FDs first, then drop the borrow, then write to memory.
            let (read_fd, write_fd) = {
                let fd_table = match caller.data_mut().get_custom_mut::<FdTable>() {
                    Some(t) => t,
                    None => return -1,
                };

                let read_fd = fd_table.allocate(Box::new(read_end));
                let write_fd = fd_table.allocate(Box::new(write_end));
                (read_fd, write_fd)
            };

            memory::write_bytes(&mut caller, read_fd_ptr, &read_fd.to_le_bytes());
            memory::write_bytes(&mut caller, write_fd_ptr, &write_fd.to_le_bytes());
            0
        },
    )?;

    Ok(())
}
