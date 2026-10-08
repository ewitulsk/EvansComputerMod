//! POSIX-style socket API for WASI programs.
//!
//! Provides safe Rust wrappers around the `sock_*` host functions.
//! Socket file descriptors are integrated into the WASI FdTable,
//! so `fd_read`/`fd_write`/`fd_close` also work on sockets.

extern crate alloc;

// Socket domains
pub const AF_INET: i32 = 2;
pub const AF_NETLINK: i32 = 16;
/// Whole Ethernet frames, bound to one interface (see [`SockAddrLl`]).
pub const AF_PACKET: i32 = 17;

// Socket types
pub const SOCK_STREAM: i32 = 1;
pub const SOCK_DGRAM: i32 = 2;
pub const SOCK_RAW: i32 = 3;

// Protocols
pub const IPPROTO_ICMP: i32 = 1;
pub const IPPROTO_TCP: i32 = 6;
pub const IPPROTO_UDP: i32 = 17;
pub const NETLINK_ROUTE: i32 = 0;

// EtherTypes for AF_PACKET (host order; pass `htons(..)` to `socket`).
pub const ETH_P_ALL: u16 = 0x0003;
pub const ETH_P_IP: u16 = 0x0800;
pub const ETH_P_ARP: u16 = 0x0806;
pub const ETH_P_PAE: u16 = 0x888E;

// recv/recvfrom flags
/// Don't wait: recv returns -2 / recvfrom returns 0 when nothing is queued.
pub const MSG_DONTWAIT: i32 = 0x40;

// poll events
pub const POLLIN: i16 = 0x1;
pub const POLLOUT: i16 = 0x4;
pub const POLLERR: i16 = 0x8;
pub const POLLHUP: i16 = 0x10;
pub const POLLNVAL: i16 = 0x20;

// Socket options
pub const SOL_SOCKET: i32 = 1;
pub const SO_REUSEADDR: i32 = 2;
pub const SO_RCVTIMEO: i32 = 20;
pub const SO_SNDTIMEO: i32 = 21;
pub const IPPROTO_IP: i32 = 0;
pub const IP_TTL: i32 = 2;

// Shutdown flags
pub const SHUT_RD: i32 = 0;
pub const SHUT_WR: i32 = 1;
pub const SHUT_RDWR: i32 = 2;

/// IPv4 socket address, matching Linux `struct sockaddr_in` layout.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SockAddrIn {
    pub sin_family: u16,   // AF_INET = 2
    pub sin_port: u16,     // network byte order (big-endian)
    pub sin_addr: [u8; 4], // network byte order
    pub sin_zero: [u8; 8], // padding
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
        alloc::format!(
            "{}.{}.{}.{}:{}",
            self.sin_addr[0],
            self.sin_addr[1],
            self.sin_addr[2],
            self.sin_addr[3],
            self.port()
        )
    }
}

/// Interface name bytes in a [`SockAddrLl`].
pub const IFNAME_LEN: usize = 12;

/// Link-layer address for `AF_PACKET` sockets (16 bytes, so it fits every
/// host's sockaddr path). Differs from Linux `sockaddr_ll`: the interface is
/// named, not numbered.
///
/// `[family u16 LE = 17][protocol u16 network order][ifname, NUL-padded]`
/// - `bind`: names the interface; a non-zero protocol replaces the
///   socket's EtherType filter.
/// - `sendto`: a non-empty name sends out of that interface.
/// - `recvfrom` / `getsockname`: the arrival interface and the frame's
///   EtherType.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SockAddrLl {
    pub sll_family: u16,
    pub sll_protocol: u16,
    pub sll_ifname: [u8; IFNAME_LEN],
}

impl SockAddrLl {
    /// Address of interface `ifname` (truncated to 12 bytes); `ethertype`
    /// in host order, 0 = keep the socket's.
    pub fn new(ifname: &str, ethertype: u16) -> Self {
        let mut name = [0u8; IFNAME_LEN];
        let n = ifname.len().min(IFNAME_LEN);
        name[..n].copy_from_slice(&ifname.as_bytes()[..n]);
        Self {
            sll_family: AF_PACKET as u16,
            sll_protocol: ethertype.to_be(),
            sll_ifname: name,
        }
    }

    pub fn ifname(&self) -> &str {
        let end = self.sll_ifname.iter().position(|&b| b == 0).unwrap_or(IFNAME_LEN);
        core::str::from_utf8(&self.sll_ifname[..end]).unwrap_or("")
    }

    /// EtherType in host order.
    pub fn ethertype(&self) -> u16 {
        u16::from_be(self.sll_protocol)
    }
}

/// One entry of [`poll`]: a socket fd, the events wanted, the events seen.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PollFd {
    pub fd: i32,
    pub events: i16,
    pub revents: i16,
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
    fn sock_sendto(
        fd: i32,
        buf_ptr: i32,
        buf_len: i32,
        flags: i32,
        addr_ptr: i32,
        addr_len: i32,
    ) -> i32;
    fn sock_recvfrom(
        fd: i32,
        buf_ptr: i32,
        buf_len: i32,
        flags: i32,
        addr_ptr: i32,
        addr_len_ptr: i32,
    ) -> i32;
    fn sock_setsockopt(fd: i32, level: i32, optname: i32, optval_ptr: i32, optlen: i32) -> i32;
    fn sock_getsockname(fd: i32, addr_ptr: i32, addr_len_ptr: i32) -> i32;
    fn sock_getpeername(fd: i32, addr_ptr: i32, addr_len_ptr: i32) -> i32;
    fn sock_shutdown(fd: i32, how: i32) -> i32;
    fn sock_getaddrinfo(host_ptr: i32, host_len: i32, result_ptr: i32, result_len: i32) -> i32;
    fn sock_poll(fds_ptr: i32, nfds: i32, timeout_ms: i32) -> i32;
}

/// Open a packet socket for `ethertype` (host order, e.g. [`ETH_P_IP`] or
/// [`ETH_P_ALL`]) and bind it to interface `ifname`. Returns the fd or -1.
pub fn packet_socket(ifname: &str, ethertype: u16) -> i32 {
    let fd = socket(AF_PACKET, SOCK_RAW, ethertype.to_be() as i32);
    if fd < 0 {
        return -1;
    }
    if bind_ll(fd, &SockAddrLl::new(ifname, 0)) < 0 {
        close(fd);
        return -1;
    }
    fd
}

/// Bind a packet socket to an interface.
pub fn bind_ll(fd: i32, addr: &SockAddrLl) -> i32 {
    unsafe {
        sock_bind(
            fd,
            addr as *const SockAddrLl as i32,
            core::mem::size_of::<SockAddrLl>() as i32,
        )
    }
}

/// Send a whole frame out of `addr`'s interface.
pub fn sendto_ll(fd: i32, frame: &[u8], addr: &SockAddrLl) -> i32 {
    unsafe {
        sock_sendto(
            fd,
            frame.as_ptr() as i32,
            frame.len() as i32,
            0,
            addr as *const SockAddrLl as i32,
            core::mem::size_of::<SockAddrLl>() as i32,
        )
    }
}

/// Receive one frame and where it came from. Returns its length (truncated
/// to `buf`), 0 on timeout (or nothing queued with [`MSG_DONTWAIT`]), -1 on
/// error.
pub fn recvfrom_ll(fd: i32, buf: &mut [u8], flags: i32, addr: &mut SockAddrLl) -> i32 {
    let mut addr_len: i32 = core::mem::size_of::<SockAddrLl>() as i32;
    unsafe {
        sock_recvfrom(
            fd,
            buf.as_mut_ptr() as i32,
            buf.len() as i32,
            flags,
            addr as *mut SockAddrLl as i32,
            &mut addr_len as *mut i32 as i32,
        )
    }
}

/// Wait until one of `fds` (socket fds) has an event, or `timeout_ms`
/// passes (0 = check only, negative = forever). Returns how many entries
/// have `revents` set, 0 on timeout, -1 on error. Non-socket fds report
/// `POLLNVAL`.
pub fn poll(fds: &mut [PollFd], timeout_ms: i32) -> i32 {
    unsafe { sock_poll(fds.as_mut_ptr() as i32, fds.len() as i32, timeout_ms) }
}

/// Create a socket. Returns a file descriptor or -1 on error.
pub fn socket(domain: i32, sock_type: i32, protocol: i32) -> i32 {
    unsafe { sock_socket(domain, sock_type, protocol) }
}

/// Bind a socket to a local address.
pub fn bind(fd: i32, addr: &SockAddrIn) -> i32 {
    unsafe {
        sock_bind(
            fd,
            addr as *const SockAddrIn as i32,
            core::mem::size_of::<SockAddrIn>() as i32,
        )
    }
}

/// Connect to a remote address.
pub fn connect(fd: i32, addr: &SockAddrIn) -> i32 {
    unsafe {
        sock_connect(
            fd,
            addr as *const SockAddrIn as i32,
            core::mem::size_of::<SockAddrIn>() as i32,
        )
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
        sock_accept(
            fd,
            addr as *mut SockAddrIn as i32,
            &mut addr_len as *mut i32 as i32,
        )
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
        sock_sendto(
            fd,
            buf.as_ptr() as i32,
            buf.len() as i32,
            flags,
            addr as *const SockAddrIn as i32,
            core::mem::size_of::<SockAddrIn>() as i32,
        )
    }
}

/// Receive data with source address (UDP/raw).
pub fn recvfrom(fd: i32, buf: &mut [u8], flags: i32, addr: &mut SockAddrIn) -> i32 {
    let mut addr_len: i32 = core::mem::size_of::<SockAddrIn>() as i32;
    unsafe {
        sock_recvfrom(
            fd,
            buf.as_mut_ptr() as i32,
            buf.len() as i32,
            flags,
            addr as *mut SockAddrIn as i32,
            &mut addr_len as *mut i32 as i32,
        )
    }
}

/// Set a socket option.
pub fn setsockopt(fd: i32, level: i32, optname: i32, optval: &[u8]) -> i32 {
    unsafe {
        sock_setsockopt(
            fd,
            level,
            optname,
            optval.as_ptr() as i32,
            optval.len() as i32,
        )
    }
}

/// Get the local address of a socket.
pub fn getsockname(fd: i32, addr: &mut SockAddrIn) -> i32 {
    let mut addr_len: i32 = core::mem::size_of::<SockAddrIn>() as i32;
    unsafe {
        sock_getsockname(
            fd,
            addr as *mut SockAddrIn as i32,
            &mut addr_len as *mut i32 as i32,
        )
    }
}

/// Get the remote address of a connected socket.
pub fn getpeername(fd: i32, addr: &mut SockAddrIn) -> i32 {
    let mut addr_len: i32 = core::mem::size_of::<SockAddrIn>() as i32;
    unsafe {
        sock_getpeername(
            fd,
            addr as *mut SockAddrIn as i32,
            &mut addr_len as *mut i32 as i32,
        )
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
            core::mem::size_of::<SockAddrIn>() as i32,
        )
    };
    if n >= 16 {
        0
    } else {
        -1
    }
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
