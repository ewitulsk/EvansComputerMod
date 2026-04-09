//! Socket host functions for child WASI processes.
//!
//! Registers sock_socket, sock_connect, sock_send, sock_recv, etc.
//! Each function dispatches through the SockIpcBridge to the kernel thread.

use std::sync::Arc;
use wasmtime::*;

use crate::sock_ipc::SockIpcBridge;
use crate::wasm_host::HostState;
use super::memory;

/// Host function names registered by this module.
pub const FUNCTIONS: &[&str] = &[
    "sock_socket", "sock_bind", "sock_connect", "sock_listen", "sock_accept",
    "sock_send", "sock_recv", "sock_sendto", "sock_recvfrom", "sock_setsockopt",
    "sock_getsockname", "sock_getpeername", "sock_shutdown", "sock_getaddrinfo",
];

// Syscall IDs matching kernel's net_ipc_handler.rs
const SOCK_SOCKET: i32 = 0;
const SOCK_BIND: i32 = 1;
const SOCK_CONNECT: i32 = 2;
const SOCK_LISTEN: i32 = 3;
const SOCK_ACCEPT: i32 = 4;
const SOCK_SEND: i32 = 5;
const SOCK_RECV: i32 = 6;
const _SOCK_CLOSE: i32 = 7;
const SOCK_SETSOCKOPT: i32 = 8;
const SOCK_SENDTO: i32 = 9;
const SOCK_RECVFROM: i32 = 10;
const SOCK_GETADDRINFO: i32 = 11;
const SOCK_GETSOCKNAME: i32 = 12;
const SOCK_GETPEERNAME: i32 = 13;
const SOCK_SHUTDOWN: i32 = 14;

fn get_bridge(caller: &Caller<'_, HostState>) -> Option<Arc<SockIpcBridge>> {
    caller.data().get_custom::<Arc<SockIpcBridge>>().cloned()
}

fn get_session_id(caller: &Caller<'_, HostState>) -> i32 {
    // Use the PID stored in custom state, or default to 1
    caller.data().get_custom::<ChildSessionId>().map(|s| s.0).unwrap_or(1)
}

/// Session ID wrapper for custom state.
pub struct ChildSessionId(pub i32);

fn read_i32_le(data: &[u8], off: usize) -> i32 {
    if data.len() < off + 4 { return 0; }
    i32::from_le_bytes([data[off], data[off+1], data[off+2], data[off+3]])
}

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    // sock_socket(domain, type, protocol) -> fd
    linker.func_wrap("env", "sock_socket",
        |caller: Caller<'_, HostState>, domain: i32, sock_type: i32, protocol: i32| -> i32 {
            let bridge = match get_bridge(&caller) { Some(b) => b, None => return -1 };
            let session = get_session_id(&caller);
            let mut args = vec![0u8; 12];
            args[0..4].copy_from_slice(&domain.to_le_bytes());
            args[4..8].copy_from_slice(&sock_type.to_le_bytes());
            args[8..12].copy_from_slice(&protocol.to_le_bytes());
            let resp = bridge.call_blocking(session, SOCK_SOCKET, args, 5000);
            read_i32_le(&resp, 0)
        },
    )?;

    // sock_bind(fd, addr_ptr, addr_len) -> i32
    linker.func_wrap("env", "sock_bind",
        |mut caller: Caller<'_, HostState>, fd: i32, addr_ptr: i32, addr_len: i32| -> i32 {
            let bridge = match get_bridge(&caller) { Some(b) => b, None => return -1 };
            let session = get_session_id(&caller);
            let addr = memory::read_bytes(&mut caller, addr_ptr, addr_len.min(16)).unwrap_or_default();
            let mut args = vec![0u8; 4 + addr.len()];
            args[0..4].copy_from_slice(&fd.to_le_bytes());
            args[4..].copy_from_slice(&addr);
            let resp = bridge.call_blocking(session, SOCK_BIND, args, 5000);
            read_i32_le(&resp, 0)
        },
    )?;

    // sock_connect(fd, addr_ptr, addr_len) -> i32
    linker.func_wrap("env", "sock_connect",
        |mut caller: Caller<'_, HostState>, fd: i32, addr_ptr: i32, addr_len: i32| -> i32 {
            let bridge = match get_bridge(&caller) { Some(b) => b, None => return -1 };
            let session = get_session_id(&caller);
            let addr = memory::read_bytes(&mut caller, addr_ptr, addr_len.min(16)).unwrap_or_default();
            let mut args = vec![0u8; 4 + addr.len()];
            args[0..4].copy_from_slice(&fd.to_le_bytes());
            args[4..].copy_from_slice(&addr);
            let resp = bridge.call_blocking(session, SOCK_CONNECT, args, 15000);
            read_i32_le(&resp, 0)
        },
    )?;

    // sock_listen(fd, backlog) -> i32
    linker.func_wrap("env", "sock_listen",
        |caller: Caller<'_, HostState>, fd: i32, backlog: i32| -> i32 {
            let bridge = match get_bridge(&caller) { Some(b) => b, None => return -1 };
            let session = get_session_id(&caller);
            let mut args = vec![0u8; 8];
            args[0..4].copy_from_slice(&fd.to_le_bytes());
            args[4..8].copy_from_slice(&backlog.to_le_bytes());
            let resp = bridge.call_blocking(session, SOCK_LISTEN, args, 5000);
            read_i32_le(&resp, 0)
        },
    )?;

    // sock_accept(fd, addr_ptr, addr_len_ptr) -> new_fd
    linker.func_wrap("env", "sock_accept",
        |mut caller: Caller<'_, HostState>, fd: i32, addr_ptr: i32, addr_len_ptr: i32| -> i32 {
            let bridge = match get_bridge(&caller) { Some(b) => b, None => return -1 };
            let session = get_session_id(&caller);
            let args = fd.to_le_bytes().to_vec();
            let resp = bridge.call_blocking(session, SOCK_ACCEPT, args, 30000);
            if resp.len() < 4 { return -1; }
            let result = read_i32_le(&resp, 0);
            // Write peer address if available
            if result >= 0 && resp.len() >= 20 && addr_ptr != 0 {
                memory::write_bytes(&mut caller, addr_ptr, &resp[4..20]);
                if addr_len_ptr != 0 {
                    memory::write_bytes(&mut caller, addr_len_ptr, &16i32.to_le_bytes());
                }
            }
            result
        },
    )?;

    // sock_send(fd, buf_ptr, buf_len, flags) -> bytes_sent
    linker.func_wrap("env", "sock_send",
        |mut caller: Caller<'_, HostState>, fd: i32, buf_ptr: i32, buf_len: i32, _flags: i32| -> i32 {
            let bridge = match get_bridge(&caller) { Some(b) => b, None => return -1 };
            let session = get_session_id(&caller);
            let data = memory::read_bytes(&mut caller, buf_ptr, buf_len).unwrap_or_default();
            let mut args = vec![0u8; 4 + 2 + data.len()];
            args[0..4].copy_from_slice(&fd.to_le_bytes());
            args[4..6].copy_from_slice(&(data.len() as u16).to_le_bytes());
            args[6..].copy_from_slice(&data);
            let resp = bridge.call_blocking(session, SOCK_SEND, args, 30000);
            read_i32_le(&resp, 0)
        },
    )?;

    // sock_recv(fd, buf_ptr, buf_len, flags) -> bytes_received
    linker.func_wrap("env", "sock_recv",
        |mut caller: Caller<'_, HostState>, fd: i32, buf_ptr: i32, buf_len: i32, flags: i32| -> i32 {
            let bridge = match get_bridge(&caller) { Some(b) => b, None => return -1 };
            let session = get_session_id(&caller);
            let mut args = vec![0u8; 12];
            args[0..4].copy_from_slice(&fd.to_le_bytes());
            args[4..8].copy_from_slice(&buf_len.to_le_bytes());
            args[8..12].copy_from_slice(&flags.to_le_bytes());
            let resp = bridge.call_blocking(session, SOCK_RECV, args, 30000);
            if resp.is_empty() { return 0; }
            // Check if response is error (4 bytes with negative value)
            if resp.len() == 4 {
                let val = read_i32_le(&resp, 0);
                if val <= 0 { return val; }
            }
            // Response is raw data bytes
            let copy_len = resp.len().min(buf_len as usize);
            memory::write_bytes(&mut caller, buf_ptr, &resp[..copy_len]);
            copy_len as i32
        },
    )?;

    // sock_sendto(fd, buf_ptr, buf_len, flags, addr_ptr, addr_len) -> bytes_sent
    linker.func_wrap("env", "sock_sendto",
        |mut caller: Caller<'_, HostState>, fd: i32, buf_ptr: i32, buf_len: i32,
         _flags: i32, addr_ptr: i32, addr_len: i32| -> i32 {
            let bridge = match get_bridge(&caller) { Some(b) => b, None => return -1 };
            let session = get_session_id(&caller);
            let data = memory::read_bytes(&mut caller, buf_ptr, buf_len).unwrap_or_default();
            let addr = memory::read_bytes(&mut caller, addr_ptr, addr_len.min(16)).unwrap_or_default();
            // Args: [sock_id: i32, addr_len: u16, addr..., data_len: u16, data...]
            let mut args = vec![0u8; 4 + 2 + addr.len() + 2 + data.len()];
            args[0..4].copy_from_slice(&fd.to_le_bytes());
            args[4..6].copy_from_slice(&(addr.len() as u16).to_le_bytes());
            args[6..6+addr.len()].copy_from_slice(&addr);
            let d_off = 6 + addr.len();
            args[d_off..d_off+2].copy_from_slice(&(data.len() as u16).to_le_bytes());
            args[d_off+2..].copy_from_slice(&data);
            let resp = bridge.call_blocking(session, SOCK_SENDTO, args, 30000);
            read_i32_le(&resp, 0)
        },
    )?;

    // sock_recvfrom(fd, buf_ptr, buf_len, flags, addr_ptr, addr_len_ptr) -> bytes
    linker.func_wrap("env", "sock_recvfrom",
        |mut caller: Caller<'_, HostState>, fd: i32, buf_ptr: i32, buf_len: i32,
         _flags: i32, addr_ptr: i32, addr_len_ptr: i32| -> i32 {
            let bridge = match get_bridge(&caller) { Some(b) => b, None => return -1 };
            let session = get_session_id(&caller);
            let mut args = vec![0u8; 12];
            args[0..4].copy_from_slice(&fd.to_le_bytes());
            args[4..8].copy_from_slice(&buf_len.to_le_bytes());
            args[8..12].copy_from_slice(&0i32.to_le_bytes()); // flags
            let resp = bridge.call_blocking(session, SOCK_RECVFROM, args, 30000);
            if resp.len() < 4 { return -1; }
            // Response: [data_len: i32, addr (16 bytes), data...]
            let data_len = read_i32_le(&resp, 0);
            if data_len <= 0 { return data_len; }
            // Write source address
            if addr_ptr != 0 && resp.len() >= 20 {
                memory::write_bytes(&mut caller, addr_ptr, &resp[4..20]);
                if addr_len_ptr != 0 {
                    memory::write_bytes(&mut caller, addr_len_ptr, &16i32.to_le_bytes());
                }
            }
            // Write data
            let data_start = 20;
            let copy_len = (data_len as usize).min(resp.len().saturating_sub(data_start)).min(buf_len as usize);
            if copy_len > 0 && resp.len() > data_start {
                memory::write_bytes(&mut caller, buf_ptr, &resp[data_start..data_start + copy_len]);
            }
            copy_len as i32
        },
    )?;

    // sock_setsockopt(fd, level, optname, optval_ptr, optlen) -> i32
    linker.func_wrap("env", "sock_setsockopt",
        |caller: Caller<'_, HostState>, fd: i32, level: i32, optname: i32,
         _optval_ptr: i32, _optlen: i32| -> i32 {
            let bridge = match get_bridge(&caller) { Some(b) => b, None => return -1 };
            let session = get_session_id(&caller);
            let mut args = vec![0u8; 12];
            args[0..4].copy_from_slice(&fd.to_le_bytes());
            args[4..8].copy_from_slice(&level.to_le_bytes());
            args[8..12].copy_from_slice(&optname.to_le_bytes());
            let resp = bridge.call_blocking(session, SOCK_SETSOCKOPT, args, 5000);
            read_i32_le(&resp, 0)
        },
    )?;

    // sock_getsockname(fd, addr_ptr, addr_len_ptr) -> i32
    linker.func_wrap("env", "sock_getsockname",
        |mut caller: Caller<'_, HostState>, fd: i32, addr_ptr: i32, addr_len_ptr: i32| -> i32 {
            let bridge = match get_bridge(&caller) { Some(b) => b, None => return -1 };
            let session = get_session_id(&caller);
            let args = fd.to_le_bytes().to_vec();
            let resp = bridge.call_blocking(session, SOCK_GETSOCKNAME, args, 5000);
            if resp.len() < 16 { return -1; }
            memory::write_bytes(&mut caller, addr_ptr, &resp[..16]);
            if addr_len_ptr != 0 {
                memory::write_bytes(&mut caller, addr_len_ptr, &16i32.to_le_bytes());
            }
            0
        },
    )?;

    // sock_getpeername(fd, addr_ptr, addr_len_ptr) -> i32
    linker.func_wrap("env", "sock_getpeername",
        |mut caller: Caller<'_, HostState>, fd: i32, addr_ptr: i32, addr_len_ptr: i32| -> i32 {
            let bridge = match get_bridge(&caller) { Some(b) => b, None => return -1 };
            let session = get_session_id(&caller);
            let args = fd.to_le_bytes().to_vec();
            let resp = bridge.call_blocking(session, SOCK_GETPEERNAME, args, 5000);
            if resp.len() < 16 { return -1; }
            memory::write_bytes(&mut caller, addr_ptr, &resp[..16]);
            if addr_len_ptr != 0 {
                memory::write_bytes(&mut caller, addr_len_ptr, &16i32.to_le_bytes());
            }
            0
        },
    )?;

    // sock_shutdown(fd, how) -> i32
    linker.func_wrap("env", "sock_shutdown",
        |caller: Caller<'_, HostState>, fd: i32, how: i32| -> i32 {
            let bridge = match get_bridge(&caller) { Some(b) => b, None => return -1 };
            let session = get_session_id(&caller);
            let mut args = vec![0u8; 8];
            args[0..4].copy_from_slice(&fd.to_le_bytes());
            args[4..8].copy_from_slice(&how.to_le_bytes());
            let resp = bridge.call_blocking(session, SOCK_SHUTDOWN, args, 5000);
            read_i32_le(&resp, 0)
        },
    )?;

    // sock_getaddrinfo(host_ptr, host_len, result_ptr, result_len) -> i32
    linker.func_wrap("env", "sock_getaddrinfo",
        |mut caller: Caller<'_, HostState>, host_ptr: i32, host_len: i32,
         result_ptr: i32, result_len: i32| -> i32 {
            let bridge = match get_bridge(&caller) { Some(b) => b, None => return -1 };
            let session = get_session_id(&caller);
            let host = memory::read_string(&mut caller, host_ptr, host_len).unwrap_or_default();
            let host_bytes = host.as_bytes();
            let mut args = vec![0u8; 2 + host_bytes.len()];
            args[0..2].copy_from_slice(&(host_bytes.len() as u16).to_le_bytes());
            args[2..].copy_from_slice(host_bytes);
            let resp = bridge.call_blocking(session, SOCK_GETADDRINFO, args, 10000);
            if resp.len() < 4 { return -1; }
            let copy_len = resp.len().min(result_len as usize);
            memory::write_bytes(&mut caller, result_ptr, &resp[..copy_len]);
            copy_len as i32
        },
    )?;

    Ok(())
}
