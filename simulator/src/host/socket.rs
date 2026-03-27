//! Socket host functions for the kernel.
//!
//! These allow the kernel to create socket FDs for child processes.
//! Currently stubs — the kernel's sshd uses the raw TCP stack directly
//! (it IS the kernel), so socket FDs are only needed when user WASI
//! processes want network access.

use wasmtime::*;
use crate::wasm_host::HostState;

pub const FUNCTIONS: &[&str] = &[
    "sock_tcp_connect",
    "sock_tcp_listen",
    "sock_tcp_accept",
    "sock_send",
    "sock_recv",
    "sock_shutdown",
];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    // sock_tcp_connect(ip_ptr, ip_len, port) -> fd or -1
    linker.func_wrap("env", "sock_tcp_connect",
        |_caller: Caller<'_, HostState>, _ip_ptr: i32, _ip_len: i32, _port: i32| -> i32 {
            -1 // TODO: implement when user processes need TCP
        },
    )?;

    // sock_tcp_listen(port, backlog) -> fd or -1
    linker.func_wrap("env", "sock_tcp_listen",
        |_caller: Caller<'_, HostState>, _port: i32, _backlog: i32| -> i32 {
            -1
        },
    )?;

    // sock_tcp_accept(listener_fd, addr_ptr, addr_len_ptr) -> fd or -1
    linker.func_wrap("env", "sock_tcp_accept",
        |_caller: Caller<'_, HostState>, _fd: i32, _addr_ptr: i32, _addr_len_ptr: i32| -> i32 {
            -1
        },
    )?;

    // sock_send(fd, buf_ptr, buf_len) -> bytes_sent or -1
    linker.func_wrap("env", "sock_send",
        |_caller: Caller<'_, HostState>, _fd: i32, _buf_ptr: i32, _buf_len: i32| -> i32 {
            -1
        },
    )?;

    // sock_recv(fd, buf_ptr, buf_len) -> bytes_received or -1
    linker.func_wrap("env", "sock_recv",
        |_caller: Caller<'_, HostState>, _fd: i32, _buf_ptr: i32, _buf_len: i32| -> i32 {
            -1
        },
    )?;

    // sock_shutdown(fd, how) -> 0 or -1
    linker.func_wrap("env", "sock_shutdown",
        |_caller: Caller<'_, HostState>, _fd: i32, _how: i32| -> i32 {
            -1
        },
    )?;

    Ok(())
}
