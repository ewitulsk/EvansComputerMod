//! POSIX-style socket API for WASI programs.
//!
//! Provides safe Rust wrappers around the `sock_*` host functions.
//! Socket file descriptors are integrated into the WASI FdTable,
//! so `fd_read`/`fd_write`/`fd_close` also work on sockets.

extern crate alloc;

// Socket domains
pub const AF_INET: i32 = 2;
pub const AF_NETLINK: i32 = 16;

// Socket types
pub const SOCK_STREAM: i32 = 1;
pub const SOCK_DGRAM: i32 = 2;
pub const SOCK_RAW: i32 = 3;

// Protocols
pub const IPPROTO_ICMP: i32 = 1;
pub const IPPROTO_TCP: i32 = 6;
pub const IPPROTO_UDP: i32 = 17;
pub const NETLINK_ROUTE: i32 = 0;

// Socket options
pub const SOL_SOCKET: i32 = 1;
pub const SO_REUSEADDR: i32 = 2;
pub const SO_RCVTIMEO: i32 = 20;
pub const SO_SNDTIMEO: i32 = 21;

// Shutdown flags
pub const SHUT_RD: i32 = 0;
pub const SHUT_WR: i32 = 1;
pub const SHUT_RDWR: i32 = 2;

/// IPv4 socket address, matching Linux `struct sockaddr_in` layout.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SockAddrIn {
    pub sin_family: u16,     // AF_INET = 2
    pub sin_port: u16,       // network byte order (big-endian)
    pub sin_addr: [u8; 4],   // network byte order
    pub sin_zero: [u8; 8],   // padding
}

impl SockAddrIn {
    /// Create a new sockaddr_in for the given IP and port.
    pub fn new(a: u8, b: u8, c: u8, d: u8, port: u16) -> Self {
        Self {
            sin_family: AF_INET as u16,
            sin_port: port.to_be(),
            sin_addr: [a, b, c, d],
            sin_zero: [0; 8],
        }
    }

    /// Create from IP octets and port.
    pub fn from_ip_port(ip: [u8; 4], port: u16) -> Self {
        Self {
            sin_family: AF_INET as u16,
            sin_port: port.to_be(),
            sin_addr: ip,
            sin_zero: [0; 8],
        }
    }

    /// Create a wildcard address (0.0.0.0) with the given port.
    pub fn any(port: u16) -> Self {
        Self::new(0, 0, 0, 0, port)
    }

    /// Get the port in host byte order.
    pub fn port(&self) -> u16 {
        u16::from_be(self.sin_port)
    }

    /// Get the IP address octets.
    pub fn ip(&self) -> [u8; 4] {
        self.sin_addr
    }

    /// Format as "a.b.c.d:port" string.
    pub fn to_string(&self) -> alloc::string::String {
        alloc::format!("{}.{}.{}.{}:{}",
            self.sin_addr[0], self.sin_addr[1],
            self.sin_addr[2], self.sin_addr[3],
            self.port())
    }
}

// Host function declarations
extern "C" {
    fn sock_socket(domain: i32, sock_type: i32, protocol: i32) -> i32;
    fn sock_bind(fd: i32, addr_ptr: i32, addr_len: i32) -> i32;
    fn sock_connect(fd: i32, addr_ptr: i32, addr_len: i32) -> i32;
    fn sock_listen(fd: i32, backlog: i32) -> i32;
    fn sock_accept(fd: i32, addr_ptr: i32, addr_len_ptr: i32) -> i32;
    fn sock_send(fd: i32, buf_ptr: i32, buf_len: i32, flags: i32) -> i32;
    fn sock_recv(fd: i32, buf_ptr: i32, buf_len: i32, flags: i32) -> i32;
    fn sock_sendto(fd: i32, buf_ptr: i32, buf_len: i32, flags: i32, addr_ptr: i32, addr_len: i32) -> i32;
    fn sock_recvfrom(fd: i32, buf_ptr: i32, buf_len: i32, flags: i32, addr_ptr: i32, addr_len_ptr: i32) -> i32;
    fn sock_setsockopt(fd: i32, level: i32, optname: i32, optval_ptr: i32, optlen: i32) -> i32;
    fn sock_getsockname(fd: i32, addr_ptr: i32, addr_len_ptr: i32) -> i32;
    fn sock_getpeername(fd: i32, addr_ptr: i32, addr_len_ptr: i32) -> i32;
    fn sock_shutdown(fd: i32, how: i32) -> i32;
    fn sock_getaddrinfo(host_ptr: i32, host_len: i32, result_ptr: i32, result_len: i32) -> i32;
}

/// Create a socket. Returns a file descriptor or -1 on error.
pub fn socket(domain: i32, sock_type: i32, protocol: i32) -> i32 {
    unsafe { sock_socket(domain, sock_type, protocol) }
}

/// Bind a socket to a local address.
pub fn bind(fd: i32, addr: &SockAddrIn) -> i32 {
    unsafe {
        sock_bind(fd,
            addr as *const SockAddrIn as i32,
            core::mem::size_of::<SockAddrIn>() as i32)
    }
}

/// Connect to a remote address.
pub fn connect(fd: i32, addr: &SockAddrIn) -> i32 {
    unsafe {
        sock_connect(fd,
            addr as *const SockAddrIn as i32,
            core::mem::size_of::<SockAddrIn>() as i32)
    }
}

/// Listen for incoming connections.
pub fn listen(fd: i32, backlog: i32) -> i32 {
    unsafe { sock_listen(fd, backlog) }
}

/// Accept an incoming connection. Returns new fd or -1 on error, -2 on timeout.
pub fn accept(fd: i32, addr: &mut SockAddrIn) -> i32 {
    let mut addr_len: i32 = core::mem::size_of::<SockAddrIn>() as i32;
    unsafe {
        sock_accept(fd,
            addr as *mut SockAddrIn as i32,
            &mut addr_len as *mut i32 as i32)
    }
}

/// Send data on a connected socket.
pub fn send(fd: i32, buf: &[u8], flags: i32) -> i32 {
    unsafe { sock_send(fd, buf.as_ptr() as i32, buf.len() as i32, flags) }
}

/// Receive data from a connected socket.
pub fn recv(fd: i32, buf: &mut [u8], flags: i32) -> i32 {
    unsafe { sock_recv(fd, buf.as_mut_ptr() as i32, buf.len() as i32, flags) }
}

/// Send data to a specific destination (UDP/raw).
pub fn sendto(fd: i32, buf: &[u8], flags: i32, addr: &SockAddrIn) -> i32 {
    unsafe {
        sock_sendto(fd,
            buf.as_ptr() as i32, buf.len() as i32,
            flags,
            addr as *const SockAddrIn as i32,
            core::mem::size_of::<SockAddrIn>() as i32)
    }
}

/// Receive data with source address (UDP/raw).
pub fn recvfrom(fd: i32, buf: &mut [u8], flags: i32, addr: &mut SockAddrIn) -> i32 {
    let mut addr_len: i32 = core::mem::size_of::<SockAddrIn>() as i32;
    unsafe {
        sock_recvfrom(fd,
            buf.as_mut_ptr() as i32, buf.len() as i32,
            flags,
            addr as *mut SockAddrIn as i32,
            &mut addr_len as *mut i32 as i32)
    }
}

/// Set a socket option.
pub fn setsockopt(fd: i32, level: i32, optname: i32, optval: &[u8]) -> i32 {
    unsafe {
        sock_setsockopt(fd, level, optname,
            optval.as_ptr() as i32, optval.len() as i32)
    }
}

/// Get the local address of a socket.
pub fn getsockname(fd: i32, addr: &mut SockAddrIn) -> i32 {
    let mut addr_len: i32 = core::mem::size_of::<SockAddrIn>() as i32;
    unsafe {
        sock_getsockname(fd,
            addr as *mut SockAddrIn as i32,
            &mut addr_len as *mut i32 as i32)
    }
}

/// Get the remote address of a connected socket.
pub fn getpeername(fd: i32, addr: &mut SockAddrIn) -> i32 {
    let mut addr_len: i32 = core::mem::size_of::<SockAddrIn>() as i32;
    unsafe {
        sock_getpeername(fd,
            addr as *mut SockAddrIn as i32,
            &mut addr_len as *mut i32 as i32)
    }
}

/// Shut down part of a socket connection.
pub fn shutdown(fd: i32, how: i32) -> i32 {
    unsafe { sock_shutdown(fd, how) }
}

/// Resolve a hostname to a sockaddr_in. Returns 0 on success.
pub fn getaddrinfo(hostname: &str, result: &mut SockAddrIn) -> i32 {
    let n = unsafe {
        sock_getaddrinfo(
            hostname.as_ptr() as i32,
            hostname.len() as i32,
            result as *mut SockAddrIn as i32,
            core::mem::size_of::<SockAddrIn>() as i32)
    };
    if n >= 16 { 0 } else { -1 }
}

/// Convenience: close a socket fd. Uses WASI fd_close under the hood.
pub fn close(fd: i32) -> i32 {
    // Socket FDs are in the WASI FdTable, so fd_close works
    #[link(wasm_import_module = "wasi_snapshot_preview1")]
    extern "C" {
        fn fd_close(fd: i32) -> i32;
    }
    unsafe { fd_close(fd) }
}
