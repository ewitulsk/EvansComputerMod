//! Network IPC API for WASI programs.
//!
//! Provides TCP connections, DNS resolution, and other network services
//! via the POSIX socket API exposed by the host.

extern crate alloc;
use alloc::string::String;

use crate::socket::{self, SockAddrIn, AF_INET, SOCK_STREAM, SOCK_DGRAM, IPPROTO_TCP};

/// Resolve a hostname via DNS. Writes the IP to `ip_out`.
/// Returns 0 on success, -1 on failure.
pub fn dns_resolve(name: &str, ip_out: &mut [u8]) -> i32 {
    if ip_out.len() < 4 { return -1; }

    // Try parsing as IP address first
    if let Some(ip) = parse_ip(name) {
        ip_out[..4].copy_from_slice(&ip);
        return 0;
    }

    // DNS query via UDP socket to port 53
    let fd = socket::socket(AF_INET, SOCK_DGRAM, 0);
    if fd < 0 { return -1; }

    // Set timeout
    let timeout = [5000i32, 0i32];
    let timeout_bytes = unsafe {
        core::slice::from_raw_parts(timeout.as_ptr() as *const u8, 8)
    };
    socket::setsockopt(fd, crate::socket::SOL_SOCKET, crate::socket::SO_RCVTIMEO, timeout_bytes);

    // Build DNS query
    let mut query = [0u8; 512];
    let qlen = build_dns_query(name, &mut query);
    if qlen == 0 {
        socket::close(fd);
        return -1;
    }

    // Send to DNS server (8.8.8.8:53 as fallback)
    let dns_addr = SockAddrIn::new(8, 8, 8, 8, 53);
    socket::sendto(fd, &query[..qlen], 0, &dns_addr);

    // Receive response
    let mut buf = [0u8; 512];
    let mut from = SockAddrIn::default();
    let n = socket::recvfrom(fd, &mut buf, 0, &mut from);
    socket::close(fd);

    if n <= 0 { return -1; }

    // Parse DNS response for A record
    if let Some(ip) = parse_dns_response(&buf[..n as usize]) {
        ip_out[..4].copy_from_slice(&ip);
        0
    } else {
        -1
    }
}

/// Connect to a TCP server. Returns socket fd on success, -1 on failure.
pub fn tcp_connect(host: &str, port: u16, _timeout_ms: i32) -> i32 {
    let ip = if let Some(b) = parse_ip(host) {
        b
    } else {
        let mut ip_out = [0u8; 4];
        if dns_resolve(host, &mut ip_out) < 0 { return -1; }
        ip_out
    };

    let fd = socket::socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if fd < 0 { return -1; }

    let addr = SockAddrIn::new(ip[0], ip[1], ip[2], ip[3], port);

    let rc = socket::connect(fd, &addr);
    if rc < 0 {
        socket::close(fd);
        return -1;
    }
    fd
}

/// Listen on a TCP port. Returns listener fd on success, -1 on failure.
pub fn tcp_listen(port: u16) -> i32 {
    let fd = socket::socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if fd < 0 { return -1; }

    let addr = SockAddrIn::new(0, 0, 0, 0, port);

    if socket::bind(fd, &addr) < 0 {
        socket::close(fd);
        return -1;
    }
    if socket::listen(fd, 5) < 0 {
        socket::close(fd);
        return -1;
    }
    fd
}

/// Accept a connection on a listener. Returns new socket fd.
pub fn tcp_accept(listener: i32, _timeout_ms: i32) -> i32 {
    let mut addr = SockAddrIn::default();
    socket::accept(listener, &mut addr)
}

/// Send data on a TCP socket. Returns bytes sent.
pub fn tcp_send(fd: i32, data: &[u8]) -> i32 {
    socket::send(fd, data, 0)
}

/// Receive data from a TCP socket. Returns bytes read.
pub fn tcp_recv(fd: i32, buf: &mut [u8], _timeout_ms: i32) -> i32 {
    socket::recv(fd, buf, 0)
}

/// Close a TCP socket.
pub fn tcp_close(fd: i32) {
    socket::close(fd);
}

// --- Internal helpers ---

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let parts: alloc::vec::Vec<&str> = s.split('.').collect();
    if parts.len() != 4 { return None; }
    let a: u8 = parts[0].parse().ok()?;
    let b: u8 = parts[1].parse().ok()?;
    let c: u8 = parts[2].parse().ok()?;
    let d: u8 = parts[3].parse().ok()?;
    Some([a, b, c, d])
}

fn build_dns_query(name: &str, buf: &mut [u8]) -> usize {
    if buf.len() < 512 { return 0; }
    // Transaction ID
    buf[0] = 0x00; buf[1] = 0x01;
    // Flags: standard query
    buf[2] = 0x01; buf[3] = 0x00;
    // Questions: 1
    buf[4] = 0x00; buf[5] = 0x01;
    // Answers, Authority, Additional: 0
    for i in 6..12 { buf[i] = 0; }

    let mut off = 12;
    for label in name.split('.') {
        let len = label.len();
        if len == 0 || len > 63 || off + 1 + len >= 500 { return 0; }
        buf[off] = len as u8;
        off += 1;
        buf[off..off+len].copy_from_slice(label.as_bytes());
        off += len;
    }
    buf[off] = 0; off += 1; // root label

    // Type A (1)
    buf[off] = 0x00; buf[off+1] = 0x01; off += 2;
    // Class IN (1)
    buf[off] = 0x00; buf[off+1] = 0x01; off += 2;

    off
}

fn parse_dns_response(data: &[u8]) -> Option<[u8; 4]> {
    if data.len() < 12 { return None; }
    let ancount = u16::from_be_bytes([data[6], data[7]]);
    if ancount == 0 { return None; }

    // Skip header + question section
    let mut off = 12;
    // Skip question
    while off < data.len() && data[off] != 0 {
        let len = data[off] as usize;
        if len >= 0xC0 { off += 2; break; } // compression pointer
        off += 1 + len;
    }
    if off < data.len() && data[off] == 0 { off += 1; }
    off += 4; // skip qtype + qclass

    // Parse answers
    for _ in 0..ancount {
        if off + 12 > data.len() { return None; }
        // Skip name (handle compression)
        if data[off] & 0xC0 == 0xC0 {
            off += 2;
        } else {
            while off < data.len() && data[off] != 0 {
                let len = data[off] as usize;
                off += 1 + len;
            }
            off += 1;
        }
        if off + 10 > data.len() { return None; }
        let rtype = u16::from_be_bytes([data[off], data[off+1]]);
        let rdlen = u16::from_be_bytes([data[off+8], data[off+9]]) as usize;
        off += 10;
        if rtype == 1 && rdlen == 4 && off + 4 <= data.len() {
            return Some([data[off], data[off+1], data[off+2], data[off+3]]);
        }
        off += rdlen;
    }
    None
}
