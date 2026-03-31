//! Kernel-mediated network IPC host function wrappers.
//!
//! These host functions route TCP operations through the kernel's NetStack.
//! The host (simulator/Java) calls into the kernel WASM's exported TCP functions
//! on behalf of the calling WASI program.

extern "C" {
    fn ipc_net_tcp_listen(port: i32) -> i32;
    fn ipc_net_tcp_accept(listener_idx: i32, timeout_ms: i32) -> i32;
    fn ipc_net_tcp_connect(ip_ptr: i32, ip_len: i32, port: i32, timeout_ms: i32) -> i32;
    fn ipc_net_tcp_send(conn_idx: i32, buf_ptr: i32, buf_len: i32) -> i32;
    fn ipc_net_tcp_recv(conn_idx: i32, buf_ptr: i32, buf_len: i32, timeout_ms: i32) -> i32;
    fn ipc_net_tcp_close(conn_idx: i32) -> i32;
    fn ipc_net_dns_resolve(name_ptr: i32, name_len: i32, ip_out_ptr: i32) -> i32;
}

/// Listen on a TCP port via the kernel's network stack.
/// Returns a connection index or -1 on error.
pub fn tcp_listen(port: u16) -> i32 {
    unsafe { ipc_net_tcp_listen(port as i32) }
}

/// Accept a connection on a listener. Returns conn index, -1 on error, -2 on timeout.
pub fn tcp_accept(listener_idx: i32, timeout_ms: i32) -> i32 {
    unsafe { ipc_net_tcp_accept(listener_idx, timeout_ms) }
}

/// Connect to a remote TCP endpoint. Returns conn index or -1 on error.
pub fn tcp_connect(ip: &str, port: u16, timeout_ms: i32) -> i32 {
    unsafe {
        ipc_net_tcp_connect(
            ip.as_ptr() as i32,
            ip.len() as i32,
            port as i32,
            timeout_ms,
        )
    }
}

/// Send data on a TCP connection. Returns bytes sent or -1 on error.
pub fn tcp_send(conn_idx: i32, data: &[u8]) -> i32 {
    unsafe { ipc_net_tcp_send(conn_idx, data.as_ptr() as i32, data.len() as i32) }
}

/// Receive data from a TCP connection with timeout.
/// Returns bytes received, 0 on timeout, -1 on error/closed.
pub fn tcp_recv(conn_idx: i32, buf: &mut [u8], timeout_ms: i32) -> i32 {
    unsafe {
        ipc_net_tcp_recv(
            conn_idx,
            buf.as_mut_ptr() as i32,
            buf.len() as i32,
            timeout_ms,
        )
    }
}

/// Close a TCP connection. Returns 0 on success.
pub fn tcp_close(conn_idx: i32) -> i32 {
    unsafe { ipc_net_tcp_close(conn_idx) }
}

/// Resolve a hostname to an IPv4 address (4 bytes written to `ip_out`).
/// Returns 0 on success, -1 on failure.
pub fn dns_resolve(hostname: &str, ip_out: &mut [u8; 4]) -> i32 {
    unsafe {
        ipc_net_dns_resolve(
            hostname.as_ptr() as i32,
            hostname.len() as i32,
            ip_out.as_mut_ptr() as i32,
        )
    }
}
