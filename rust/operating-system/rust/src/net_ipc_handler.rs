//! Socket IPC handler — dispatches socket syscalls from child WASI processes
//! to the kernel's NetStack.
//!
//! The Java host calls `handle_sock_ipc` with a session ID (child PID),
//! syscall ID, args buffer, and result buffer. This module parses the args,
//! dispatches to the appropriate NetStack method, and writes results.

extern crate alloc;
use alloc::vec::Vec;
use ecm_net::{NetStack, types::*};

// Syscall IDs (must match Java SocketFd constants)
const SOCK_SOCKET: i32 = 0;
const SOCK_BIND: i32 = 1;
const SOCK_CONNECT: i32 = 2;
const SOCK_LISTEN: i32 = 3;
const SOCK_ACCEPT: i32 = 4;
const SOCK_SEND: i32 = 5;
const SOCK_RECV: i32 = 6;
const SOCK_CLOSE: i32 = 7;
const SOCK_SETSOCKOPT: i32 = 8;
const SOCK_SENDTO: i32 = 9;
const SOCK_RECVFROM: i32 = 10;
const SOCK_GETADDRINFO: i32 = 11;
const SOCK_GETSOCKNAME: i32 = 12;
const SOCK_GETPEERNAME: i32 = 13;
const SOCK_SHUTDOWN: i32 = 14;
const SOCK_DESTROY_SESSION: i32 = 99;

// Socket domain/type/protocol constants (match Linux)
const AF_INET: i32 = 2;
const AF_NETLINK: i32 = 16;
const SOCK_STREAM: i32 = 1;
const SOCK_DGRAM: i32 = 2;
const SOCK_RAW: i32 = 3;
const IPPROTO_ICMP: i32 = 1;

// Maximum number of concurrent IPC sessions (child processes)
const MAX_SESSIONS: usize = 16;
// Maximum sockets per session
const MAX_SOCKETS: usize = 32;

/// Socket types managed by the IPC handler.
#[derive(Clone)]
enum IpcSocket {
    /// Unconnected TCP socket, optionally bound to a port.
    TcpUnbound { bound_port: u16 },
    TcpStream { conn_idx: usize },
    TcpListener { conn_idx: usize, port: u16 },
    UdpSocket { sock_idx: usize, bound_port: u16 },
    RawIcmp { reply_buf: Vec<u8> },
    Netlink { response_buf: Vec<u8>, read_offset: usize },
}

/// Per-session socket table.
struct IpcSession {
    session_id: i32,
    sockets: [Option<IpcSocket>; MAX_SOCKETS],
}

impl IpcSession {
    fn new(session_id: i32) -> Self {
        Self {
            session_id,
            sockets: Default::default(),
        }
    }

    fn alloc(&mut self) -> Option<usize> {
        for i in 0..MAX_SOCKETS {
            if self.sockets[i].is_none() {
                return Some(i);
            }
        }
        None
    }
}

/// Global session table.
static mut SESSIONS: [Option<IpcSession>; MAX_SESSIONS] = {
    // Can't use Default in const context, initialize manually
    const NONE: Option<IpcSession> = None;
    [NONE; MAX_SESSIONS]
};

fn get_or_create_session(session_id: i32) -> Option<&'static mut IpcSession> {
    unsafe {
        // Find existing
        for s in SESSIONS.iter_mut() {
            if let Some(ref mut sess) = s {
                if sess.session_id == session_id {
                    return Some(sess);
                }
            }
        }
        // Create new
        for s in SESSIONS.iter_mut() {
            if s.is_none() {
                *s = Some(IpcSession::new(session_id));
                return s.as_mut();
            }
        }
        None
    }
}

fn destroy_session(session_id: i32) {
    unsafe {
        for s in SESSIONS.iter_mut() {
            if let Some(ref sess) = s {
                if sess.session_id == session_id {
                    // Close all sockets
                    if let Some(stack) = NetStack::get() {
                        for sock in sess.sockets.iter() {
                            if let Some(ref sock) = sock {
                                match sock {
                                    IpcSocket::TcpStream { conn_idx } => stack.tcp_close_immediate(*conn_idx),
                                    IpcSocket::TcpListener { conn_idx, .. } => stack.tcp_close_immediate(*conn_idx),
                                    IpcSocket::UdpSocket { sock_idx, .. } => stack.udp_sockets.close(*sock_idx),
                                    _ => {}
                                }
                            }
                        }
                    }
                    *s = None;
                    return;
                }
            }
        }
    }
}

// --- Arg/Result helpers ---

fn read_i32(args: &[u8], off: usize) -> i32 {
    if args.len() < off + 4 { return 0; }
    i32::from_le_bytes([args[off], args[off+1], args[off+2], args[off+3]])
}

fn read_u16(args: &[u8], off: usize) -> u16 {
    if args.len() < off + 2 { return 0; }
    u16::from_le_bytes([args[off], args[off+1]])
}

fn write_i32(result: &mut [u8], off: usize, val: i32) {
    let bytes = val.to_le_bytes();
    if result.len() >= off + 4 {
        result[off..off+4].copy_from_slice(&bytes);
    }
}

/// Parse a sockaddr_in from bytes: [family: u16, port: u16 BE, addr: u32 BE, zero: 8]
fn parse_sockaddr_in(data: &[u8]) -> Option<(Ipv4Addr, u16)> {
    if data.len() < 8 { return None; }
    let port = u16::from_be_bytes([data[2], data[3]]);
    let ip = Ipv4Addr::new(data[4], data[5], data[6], data[7]);
    Some((ip, port))
}

/// Write a sockaddr_in to bytes
fn write_sockaddr_in(result: &mut [u8], off: usize, ip: &Ipv4Addr, port: u16) {
    if result.len() < off + 16 { return; }
    result[off] = (AF_INET as u16).to_le_bytes()[0]; // sin_family low byte
    result[off+1] = (AF_INET as u16).to_le_bytes()[1]; // sin_family high byte
    let port_be = port.to_be_bytes();
    result[off+2] = port_be[0];
    result[off+3] = port_be[1];
    result[off+4] = ip.0[0];
    result[off+5] = ip.0[1];
    result[off+6] = ip.0[2];
    result[off+7] = ip.0[3];
    // zero-fill remaining 8 bytes
    for i in 8..16 {
        result[off+i] = 0;
    }
}

/// Read a string from args: [len: u16 LE, utf8 bytes...]
fn read_string(args: &[u8], off: usize) -> (&str, usize) {
    let len = read_u16(args, off) as usize;
    let start = off + 2;
    let end = start + len;
    if end > args.len() {
        return ("", off + 2);
    }
    let s = core::str::from_utf8(&args[start..end]).unwrap_or("");
    (s, end)
}

// --- Main dispatcher ---

/// Entry point called by Java via the kernel's WASM export.
pub fn dispatch(
    session_id: i32,
    syscall_id: i32,
    args: &[u8],
    result: &mut [u8],
) -> i32 {
    if syscall_id == SOCK_DESTROY_SESSION {
        destroy_session(session_id);
        return 0;
    }

    let session = match get_or_create_session(session_id) {
        Some(s) => s,
        None => return -1, // too many sessions
    };

    match syscall_id {
        SOCK_SOCKET => handle_socket(session, args, result),
        SOCK_BIND => handle_bind(session, args, result),
        SOCK_CONNECT => handle_connect(session, args, result),
        SOCK_LISTEN => handle_listen(session, args, result),
        SOCK_ACCEPT => handle_accept(session, args, result),
        SOCK_SEND => handle_send(session, args, result),
        SOCK_RECV => handle_recv(session, args, result),
        SOCK_CLOSE => handle_close(session, args, result),
        SOCK_SETSOCKOPT => handle_setsockopt(session, args, result),
        SOCK_SENDTO => handle_sendto(session, args, result),
        SOCK_RECVFROM => handle_recvfrom(session, args, result),
        SOCK_GETADDRINFO => handle_getaddrinfo(args, result),
        SOCK_GETSOCKNAME => handle_getsockname(session, args, result),
        SOCK_GETPEERNAME => handle_getpeername(session, args, result),
        SOCK_SHUTDOWN => handle_shutdown(session, args, result),
        _ => -1,
    }
}

// --- Syscall implementations ---

fn handle_socket(session: &mut IpcSession, args: &[u8], result: &mut [u8]) -> i32 {
    let domain = read_i32(args, 0);
    let sock_type = read_i32(args, 4);
    let protocol = read_i32(args, 8);

    let slot = match session.alloc() {
        Some(s) => s,
        None => { write_i32(result, 0, -1); return 4; }
    };

    let sock = match (domain, sock_type, protocol) {
        (AF_INET, SOCK_STREAM, _) => {
            // TCP socket — unbound initially, becomes TcpStream on connect or TcpListener on listen
            IpcSocket::TcpUnbound { bound_port: 0 }
        }
        (AF_INET, SOCK_DGRAM, _) => {
            // UDP socket — bind ephemeral port
            let stack = match NetStack::get() {
                Some(s) => s,
                None => { write_i32(result, 0, -1); return 4; }
            };
            match stack.udp_sockets.bind_ephemeral() {
                Ok((idx, port)) => IpcSocket::UdpSocket { sock_idx: idx, bound_port: port },
                Err(_) => { write_i32(result, 0, -1); return 4; }
            }
        }
        (AF_INET, SOCK_RAW, IPPROTO_ICMP) => {
            IpcSocket::RawIcmp { reply_buf: Vec::new() }
        }
        (AF_NETLINK, SOCK_DGRAM, _) => {
            IpcSocket::Netlink { response_buf: Vec::new(), read_offset: 0 }
        }
        _ => {
            write_i32(result, 0, -1);
            return 4;
        }
    };

    session.sockets[slot] = Some(sock);
    write_i32(result, 0, slot as i32);
    4
}

fn handle_bind(session: &mut IpcSession, args: &[u8], result: &mut [u8]) -> i32 {
    let sock_id = read_i32(args, 0) as usize;
    if sock_id >= MAX_SOCKETS { write_i32(result, 0, -1); return 4; }

    let addr_data = &args[4..];
    let (ip, port) = match parse_sockaddr_in(addr_data) {
        Some(v) => v,
        None => { write_i32(result, 0, -1); return 4; }
    };

    let sock = match session.sockets[sock_id].as_mut() {
        Some(s) => s,
        None => { write_i32(result, 0, -1); return 4; }
    };

    let stack = match NetStack::get() {
        Some(s) => s,
        None => { write_i32(result, 0, -1); return 4; }
    };

    match sock {
        IpcSocket::UdpSocket { ref mut sock_idx, ref mut bound_port, .. } => {
            // Close ephemeral binding and rebind to requested port
            stack.udp_sockets.close(*sock_idx);
            match stack.udp_sockets.bind(port) {
                Ok(idx) => {
                    *sock_idx = idx;
                    *bound_port = port;
                    write_i32(result, 0, 0);
                }
                Err(_) => { write_i32(result, 0, -1); }
            }
        }
        IpcSocket::TcpUnbound { ref mut bound_port, .. } => {
            // TCP bind remembers the port for later listen()
            *bound_port = port;
            write_i32(result, 0, 0);
        }
        _ => { write_i32(result, 0, -1); }
    }
    4
}

fn handle_connect(session: &mut IpcSession, args: &[u8], result: &mut [u8]) -> i32 {
    let sock_id = read_i32(args, 0) as usize;
    if sock_id >= MAX_SOCKETS { write_i32(result, 0, -1); return 4; }

    let addr_data = &args[4..];
    let (ip, port) = match parse_sockaddr_in(addr_data) {
        Some(v) => v,
        None => { write_i32(result, 0, -1); return 4; }
    };

    let sock = match session.sockets[sock_id].as_mut() {
        Some(s) => s,
        None => { write_i32(result, 0, -1); return 4; }
    };

    match sock {
        IpcSocket::TcpUnbound { .. } => {
            let stack = match NetStack::get() {
                Some(s) => s,
                None => { write_i32(result, 0, -1); return 4; }
            };
            let remote = SocketAddr { ip, port };
            match stack.tcp_connect(remote, 10000) {
                Ok(idx) => {
                    // Transition from TcpUnbound to TcpStream
                    session.sockets[sock_id] = Some(IpcSocket::TcpStream { conn_idx: idx });
                    write_i32(result, 0, 0);
                }
                Err(_) => { write_i32(result, 0, -1); }
            }
        }
        _ => { write_i32(result, 0, -1); }
    }
    4
}

fn handle_listen(session: &mut IpcSession, args: &[u8], result: &mut [u8]) -> i32 {
    let sock_id = read_i32(args, 0) as usize;
    let _backlog = read_i32(args, 4);
    if sock_id >= MAX_SOCKETS { write_i32(result, 0, -1); return 4; }

    let bound_port = match &session.sockets[sock_id] {
        Some(IpcSocket::TcpUnbound { bound_port }) => *bound_port,
        _ => { write_i32(result, 0, -1); return 4; }
    };

    if bound_port == 0 {
        write_i32(result, 0, -1); // must bind first
        return 4;
    }

    let stack = match NetStack::get() {
        Some(s) => s,
        None => { write_i32(result, 0, -1); return 4; }
    };

    // Find local IP for the listener (use first configured interface)
    let local_ip = stack.source_ip_for(&Ipv4Addr::ZERO)
        .unwrap_or(Ipv4Addr::ZERO);

    match stack.tcp_connections.listen(local_ip, bound_port) {
        Ok(conn_idx) => {
            session.sockets[sock_id] = Some(IpcSocket::TcpListener { conn_idx, port: bound_port });
            write_i32(result, 0, 0);
        }
        Err(_) => { write_i32(result, 0, -1); }
    }
    4
}

fn handle_accept(session: &mut IpcSession, args: &[u8], result: &mut [u8]) -> i32 {
    let sock_id = read_i32(args, 0) as usize;
    if sock_id >= MAX_SOCKETS { write_i32(result, 0, -1); return 4; }

    let (listener_idx, port) = match &session.sockets[sock_id] {
        Some(IpcSocket::TcpListener { conn_idx, port }) => (*conn_idx, *port),
        _ => { write_i32(result, 0, -1); return 4; }
    };

    let stack = match NetStack::get() {
        Some(s) => s,
        None => { write_i32(result, 0, -1); return 4; }
    };

    match stack.tcp_accept(listener_idx, 500) {
        Ok(conn_idx) => {
            // Allocate new socket for accepted connection
            let slot = match session.alloc() {
                Some(s) => s,
                None => { write_i32(result, 0, -1); return 4; }
            };
            // Get peer address
            let c = &stack.tcp_connections.connections[conn_idx];
            let peer_ip = c.remote.ip;
            let peer_port = c.remote.port;
            session.sockets[slot] = Some(IpcSocket::TcpStream { conn_idx });
            write_i32(result, 0, slot as i32);
            write_sockaddr_in(result, 4, &peer_ip, peer_port);
            20 // 4 bytes sock_id + 16 bytes sockaddr_in
        }
        Err(NetError::TimedOut) => {
            // Timeout — return -2 (EAGAIN equivalent)
            write_i32(result, 0, -2);
            4
        }
        Err(_) => {
            write_i32(result, 0, -1);
            4
        }
    }
}

fn handle_send(session: &mut IpcSession, args: &[u8], result: &mut [u8]) -> i32 {
    let sock_id = read_i32(args, 0) as usize;
    if sock_id >= MAX_SOCKETS { write_i32(result, 0, -1); return 4; }

    let data_len = read_u16(args, 4) as usize;
    let data = &args[6..6 + data_len.min(args.len().saturating_sub(6))];

    let conn_idx = match &session.sockets[sock_id] {
        Some(IpcSocket::TcpStream { conn_idx }) => *conn_idx,
        _ => { write_i32(result, 0, -1); return 4; }
    };

    let stack = match NetStack::get() {
        Some(s) => s,
        None => { write_i32(result, 0, -1); return 4; }
    };

    match stack.tcp_send(conn_idx, data) {
        Ok(n) => { write_i32(result, 0, n as i32); }
        Err(_) => { write_i32(result, 0, -1); }
    }
    4
}

fn handle_recv(session: &mut IpcSession, args: &[u8], result: &mut [u8]) -> i32 {
    let sock_id = read_i32(args, 0) as usize;
    let max_len = read_i32(args, 4) as usize;
    let _flags = read_i32(args, 8);
    if sock_id >= MAX_SOCKETS { write_i32(result, 0, -1); return 4; }

    let conn_idx = match &session.sockets[sock_id] {
        Some(IpcSocket::TcpStream { conn_idx }) => *conn_idx,
        _ => { write_i32(result, 0, -1); return 4; }
    };

    let stack = match NetStack::get() {
        Some(s) => s,
        None => { write_i32(result, 0, -1); return 4; }
    };

    let buf_len = max_len.min(result.len());
    match stack.tcp_recv(conn_idx, &mut result[..buf_len], 5000) {
        Ok(n) if n > 0 => n as i32,
        Ok(_) => {
            // EOF — connection closing
            write_i32(result, 0, -1);
            4
        }
        Err(NetError::TimedOut) => 0, // timeout, no data
        Err(_) => {
            write_i32(result, 0, -1);
            4
        }
    }
}

fn handle_close(session: &mut IpcSession, args: &[u8], result: &mut [u8]) -> i32 {
    let sock_id = read_i32(args, 0) as usize;
    if sock_id >= MAX_SOCKETS { write_i32(result, 0, -1); return 4; }

    if let Some(sock) = session.sockets[sock_id].take() {
        if let Some(stack) = NetStack::get() {
            match sock {
                IpcSocket::TcpStream { conn_idx } => {
                    stack.tcp_close(conn_idx);
                }
                IpcSocket::TcpListener { conn_idx, .. } => {
                    stack.tcp_close_immediate(conn_idx);
                }
                IpcSocket::UdpSocket { sock_idx, .. } => {
                    stack.udp_sockets.close(sock_idx);
                }
                _ => {}
            }
        }
    }
    write_i32(result, 0, 0);
    4
}

fn handle_setsockopt(_session: &mut IpcSession, _args: &[u8], result: &mut [u8]) -> i32 {
    // Most socket options are no-ops for our simplified stack
    write_i32(result, 0, 0);
    4
}

fn handle_sendto(session: &mut IpcSession, args: &[u8], result: &mut [u8]) -> i32 {
    let sock_id = read_i32(args, 0) as usize;
    if sock_id >= MAX_SOCKETS { write_i32(result, 0, -1); return 4; }

    let addr_len = read_u16(args, 4) as usize;
    let addr_data = &args[6..6 + addr_len.min(args.len().saturating_sub(6))];
    let (ip, port) = match parse_sockaddr_in(addr_data) {
        Some(v) => v,
        None => { write_i32(result, 0, -1); return 4; }
    };

    let data_off = 6 + addr_len;
    let data_len = read_u16(args, data_off) as usize;
    let data = &args[data_off + 2..data_off + 2 + data_len.min(args.len().saturating_sub(data_off + 2))];

    let sock = match &mut session.sockets[sock_id] {
        Some(s) => s,
        None => { write_i32(result, 0, -1); return 4; }
    };

    let stack = match NetStack::get() {
        Some(s) => s,
        None => { write_i32(result, 0, -1); return 4; }
    };

    match sock {
        IpcSocket::UdpSocket { sock_idx, .. } => {
            let dst = SocketAddr { ip, port };
            match stack.udp_send(*sock_idx, dst, data) {
                Ok(()) => { write_i32(result, 0, data.len() as i32); }
                Err(_) => { write_i32(result, 0, -1); }
            }
        }
        IpcSocket::RawIcmp { .. } => {
            // Send raw ICMP packet with fast-first ARP retry schedule.
            // Typical ARP replies arrive quickly, so avoid fixed 100ms stalls.
            const ARP_RETRY_DELAYS_MS: [u32; 4] = [5, 10, 20, 40];
            let mut retry_idx = 0usize;
            loop {
                match stack.send_ipv4(ip, ecm_net::ipv4::PROTO_ICMP, data) {
                    Ok(()) => {
                        write_i32(result, 0, data.len() as i32);
                        break;
                    }
                    Err(NetError::WouldBlock) => {
                        if retry_idx >= ARP_RETRY_DELAYS_MS.len() {
                            write_i32(result, 0, -1);
                            break;
                        }
                        let delay_ms = ARP_RETRY_DELAYS_MS[retry_idx];
                        retry_idx += 1;
                        ecm_net::host_sleep_ms(delay_ms);
                        stack.now_ms = ecm_net::current_time_ms();
                        stack.poll_rx();
                    }
                    Err(_) => {
                        write_i32(result, 0, -1);
                        break;
                    }
                }
            }
        }
        IpcSocket::Netlink { ref mut response_buf, ref mut read_offset } => {
            // Netlink sendto — parse and handle netlink message
            let response = crate::netlink::handle_netlink_message(data);
            *response_buf = response;
            *read_offset = 0;
            write_i32(result, 0, data.len() as i32);
        }
        _ => { write_i32(result, 0, -1); }
    }
    4
}

fn handle_recvfrom(session: &mut IpcSession, args: &[u8], result: &mut [u8]) -> i32 {
    let sock_id = read_i32(args, 0) as usize;
    let max_len = read_i32(args, 4) as usize;
    if sock_id >= MAX_SOCKETS { write_i32(result, 0, -1); return 4; }

    let sock = match &mut session.sockets[sock_id] {
        Some(s) => s,
        None => { write_i32(result, 0, -1); return 4; }
    };

    let stack = match NetStack::get() {
        Some(s) => s,
        None => { write_i32(result, 0, -1); return 4; }
    };

    match sock {
        IpcSocket::UdpSocket { sock_idx, .. } => {
            // Poll for data with timeout
            let deadline = ecm_net::current_time_ms() + 5000;
            loop {
                let buf_start = 20; // 4 (data_len) + 16 (sockaddr_in)
                let buf_end = buf_start + max_len.min(result.len().saturating_sub(buf_start));
                if let Some((from, n)) = stack.udp_sockets.recv(*sock_idx, &mut result[buf_start..buf_end]) {
                    write_i32(result, 0, n as i32);
                    write_sockaddr_in(result, 4, &from.ip, from.port);
                    return (buf_start + n) as i32;
                }
                if ecm_net::current_time_ms() >= deadline {
                    write_i32(result, 0, 0); // timeout
                    return 4;
                }
                stack.poll_rx();
                ecm_net::host_sleep_ms(10);
            }
        }
        IpcSocket::RawIcmp { .. } => {
            // Poll for ICMP replies with timeout
            let deadline = ecm_net::current_time_ms() + 5000;
            let mut sleep_ms = 1u32;
            loop {
                stack.poll_rx();
                if stack.raw_icmp_reply_count > 0 {
                    // Dequeue first reply
                    let (src_ip, ref pkt_data, pkt_len) = stack.raw_icmp_replies[0];
                    let buf_start = 20; // 4 (data_len) + 16 (sockaddr_in)
                    let copy_len = pkt_len.min(max_len).min(result.len().saturating_sub(buf_start));
                    result[buf_start..buf_start + copy_len].copy_from_slice(&pkt_data[..copy_len]);
                    write_i32(result, 0, copy_len as i32);
                    write_sockaddr_in(result, 4, &src_ip, 0);
                    // Shift remaining replies down
                    for i in 1..stack.raw_icmp_reply_count {
                        stack.raw_icmp_replies[i - 1] = stack.raw_icmp_replies[i];
                    }
                    stack.raw_icmp_reply_count -= 1;
                    return (buf_start + copy_len) as i32;
                }
                if ecm_net::current_time_ms() >= deadline {
                    write_i32(result, 0, 0); // timeout
                    return 4;
                }
                ecm_net::host_sleep_ms(sleep_ms);
                sleep_ms = (sleep_ms.saturating_mul(2)).min(8);
            }
        }
        IpcSocket::Netlink { ref mut response_buf, ref mut read_offset } => {
            // Return buffered netlink response data
            let avail = response_buf.len() - *read_offset;
            if avail == 0 {
                write_i32(result, 0, 0);
                return 4;
            }
            let buf_start = 20;
            let copy_len = avail.min(max_len).min(result.len().saturating_sub(buf_start));
            result[buf_start..buf_start + copy_len]
                .copy_from_slice(&response_buf[*read_offset..*read_offset + copy_len]);
            *read_offset += copy_len;
            // Source address for netlink is kernel (pid=0)
            write_i32(result, 0, copy_len as i32);
            // Write netlink sockaddr (simplified)
            for i in 4..20 { result[i] = 0; }
            (buf_start + copy_len) as i32
        }
        _ => {
            write_i32(result, 0, -1);
            4
        }
    }
}

fn handle_getaddrinfo(args: &[u8], result: &mut [u8]) -> i32 {
    let (hostname, _) = read_string(args, 0);
    if hostname.is_empty() {
        write_i32(result, 0, -1);
        return 4;
    }

    // Check if it's already an IP address
    if let Some(ip) = Ipv4Addr::parse(hostname) {
        write_sockaddr_in(result, 0, &ip, 0);
        return 16;
    }

    // DNS resolve
    let stack = match NetStack::get() {
        Some(s) => s,
        None => { write_i32(result, 0, -1); return 4; }
    };

    match stack.dns_resolve(hostname, 5000) {
        Ok(ip) => {
            write_sockaddr_in(result, 0, &ip, 0);
            16
        }
        Err(_) => {
            write_i32(result, 0, -1);
            4
        }
    }
}

fn handle_getsockname(session: &mut IpcSession, args: &[u8], result: &mut [u8]) -> i32 {
    let sock_id = read_i32(args, 0) as usize;
    if sock_id >= MAX_SOCKETS { return -1; }

    let sock = match &session.sockets[sock_id] {
        Some(s) => s,
        None => return -1,
    };

    let stack = match NetStack::get() {
        Some(s) => s,
        None => return -1,
    };

    match sock {
        IpcSocket::TcpStream { conn_idx } => {
            let c = &stack.tcp_connections.connections[*conn_idx];
            write_sockaddr_in(result, 0, &c.local.ip, c.local.port);
            16
        }
        IpcSocket::TcpListener { conn_idx, port } => {
            let c = &stack.tcp_connections.connections[*conn_idx];
            write_sockaddr_in(result, 0, &c.local.ip, *port);
            16
        }
        IpcSocket::UdpSocket { bound_port, .. } => {
            write_sockaddr_in(result, 0, &Ipv4Addr::ZERO, *bound_port);
            16
        }
        _ => -1,
    }
}

fn handle_getpeername(session: &mut IpcSession, args: &[u8], result: &mut [u8]) -> i32 {
    let sock_id = read_i32(args, 0) as usize;
    if sock_id >= MAX_SOCKETS { return -1; }

    let sock = match &session.sockets[sock_id] {
        Some(s) => s,
        None => return -1,
    };

    let stack = match NetStack::get() {
        Some(s) => s,
        None => return -1,
    };

    match sock {
        IpcSocket::TcpStream { conn_idx } => {
            let c = &stack.tcp_connections.connections[*conn_idx];
            write_sockaddr_in(result, 0, &c.remote.ip, c.remote.port);
            16
        }
        _ => -1,
    }
}

fn handle_shutdown(session: &mut IpcSession, args: &[u8], result: &mut [u8]) -> i32 {
    let sock_id = read_i32(args, 0) as usize;
    if sock_id >= MAX_SOCKETS { write_i32(result, 0, -1); return 4; }
    // For our stack, shutdown is equivalent to close
    handle_close(session, args, result)
}
