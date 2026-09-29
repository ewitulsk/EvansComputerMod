//! Socket syscalls for child processes, proxied by the host.
//!
//! The host calls `handle_sock_ipc(session, syscall, args, result)` on the
//! kernel thread. Handlers NEVER block: an operation that can't complete yet
//! returns [`IPC_PENDING`] and the host retries it after the next network
//! event or tick. A child thread only has one call in flight at a time, so
//! each session tracks at most one pending operation (its start time drives
//! the per-socket timeout).
//!
//! Result encoding: `[status: i32 LE][payload]`, return value = total length.
//! Child-visible status values are unchanged from the previous kernel:
//! accept -2 = timeout; recv 0 = EOF, -2 = timeout; recvfrom 0 = timeout.

use std::collections::BTreeMap;

use ecm_net::types::{Ipv4Addr, NetError, SocketAddr};
use ecm_net::{DnsHandle, DnsStatus, SocketHandle, Stack, TcpState};

pub const IPC_PENDING: i32 = -11;

// Syscall ids (match Java SocketFd).
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
pub const SOCK_DESTROY_SESSION: i32 = 99;

const AF_INET: i32 = 2;
const AF_NETLINK: i32 = 16;
const SOCK_STREAM: i32 = 1;
const SOCK_DGRAM: i32 = 2;
const SOCK_RAW: i32 = 3;
const IPPROTO_ICMP: i32 = 1;
const SOL_SOCKET: i32 = 1;
const SO_RCVTIMEO: i32 = 20;
const SHUT_RD: i32 = 0;

const MAX_SESSIONS: usize = 64;
const MAX_SOCKETS: usize = 32;
const CONNECT_TIMEOUT_MS: i64 = 10_000;
/// recvfrom on UDP/raw sockets historically timed out after 5 s returning 0.
const DGRAM_DEFAULT_TIMEOUT_MS: i64 = 5_000;
/// sockaddr_in is 16 bytes on the wire.
const SOCKADDR_LEN: usize = 16;

enum Kind {
    TcpNew { bind: Option<SocketAddr> },
    Tcp(SocketHandle),
    Listener(SocketHandle),
    Udp(SocketHandle),
    Icmp(SocketHandle),
    Netlink { resp: Vec<u8>, off: usize },
}

struct Sock {
    kind: Kind,
    /// SO_RCVTIMEO; `None` = wait forever (or the kind's legacy default).
    rcvtimeo_ms: Option<i64>,
}

/// The one in-flight blocking operation for a session.
struct Pending {
    syscall: i32,
    sock: usize,
    started_ms: i64,
    dns: Option<DnsHandle>,
    timeout_ms: Option<i64>,
}

struct Session {
    sockets: Vec<Option<Sock>>,
    pending: Option<Pending>,
}

impl Session {
    fn new() -> Self {
        Self { sockets: (0..MAX_SOCKETS).map(|_| None).collect(), pending: None }
    }
    fn alloc(&mut self, sock: Sock) -> Option<usize> {
        let slot = self.sockets.iter().position(|s| s.is_none())?;
        self.sockets[slot] = Some(sock);
        Some(slot)
    }
}

/// Side effects of a syscall the kernel must apply outside the stack.
#[derive(Default)]
pub struct IpcEffects {
    /// Netlink changed the configuration: persist network.cfg.
    pub config_changed: bool,
    /// Netlink set interfaces administratively up/down: tell the host.
    pub admin: Vec<(usize, bool)>,
}

pub struct SocketIpc {
    sessions: BTreeMap<i32, Session>,
}

// ---------------------------------------------------------------- helpers

struct Args<'a>(&'a [u8]);

impl Args<'_> {
    fn i32(&self, off: usize) -> Option<i32> {
        let b = self.0.get(off..off + 4)?;
        Some(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u16(&self, off: usize) -> Option<u16> {
        let b = self.0.get(off..off + 2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }
    fn bytes(&self, off: usize, len: usize) -> Option<&[u8]> {
        self.0.get(off..off.checked_add(len)?)
    }
    /// [len: u16][bytes]
    fn blob(&self, off: usize) -> Option<(&[u8], usize)> {
        let len = self.u16(off)? as usize;
        Some((self.bytes(off + 2, len)?, off + 2 + len))
    }
}

fn parse_sockaddr(b: &[u8]) -> Option<SocketAddr> {
    if b.len() < 8 {
        return None;
    }
    Some(SocketAddr { ip: Ipv4Addr::new(b[4], b[5], b[6], b[7]), port: u16::from_be_bytes([b[2], b[3]]) })
}

fn sockaddr_bytes(a: &SocketAddr) -> [u8; SOCKADDR_LEN] {
    let mut out = [0u8; SOCKADDR_LEN];
    out[0..2].copy_from_slice(&(AF_INET as u16).to_le_bytes());
    out[2..4].copy_from_slice(&a.port.to_be_bytes());
    out[4..8].copy_from_slice(&a.ip.0);
    out
}

/// Writes `[status][payload]` into the result buffer.
struct Out<'a> {
    buf: &'a mut [u8],
}

impl Out<'_> {
    fn status(self, v: i32) -> i32 {
        self.with(v, &[])
    }
    fn with(self, v: i32, payload: &[u8]) -> i32 {
        if self.buf.len() < 4 {
            return -1;
        }
        self.buf[..4].copy_from_slice(&v.to_le_bytes());
        let n = payload.len().min(self.buf.len() - 4);
        self.buf[4..4 + n].copy_from_slice(&payload[..n]);
        (4 + n) as i32
    }
    /// Room left for payload after the status word.
    fn cap(&self) -> usize {
        self.buf.len().saturating_sub(4)
    }
    fn payload_mut(&mut self) -> &mut [u8] {
        let len = self.buf.len();
        &mut self.buf[4.min(len)..]
    }
    fn raw(self, v: i32, payload_len: usize) -> i32 {
        if self.buf.len() < 4 {
            return -1;
        }
        self.buf[..4].copy_from_slice(&v.to_le_bytes());
        (4 + payload_len.min(self.buf.len() - 4)) as i32
    }
}

enum Block {
    /// Not ready; retry later.
    Pending,
    /// Timed out; complete with this status.
    TimedOut,
}

impl SocketIpc {
    pub fn new() -> Self {
        Self { sessions: BTreeMap::new() }
    }

    /// Earliest time a pending operation will time out.
    pub fn next_deadline(&self) -> Option<i64> {
        self.sessions
            .values()
            .filter_map(|s| s.pending.as_ref())
            .filter_map(|p| p.timeout_ms.map(|t| p.started_ms + t))
            .min()
    }

    pub fn has_pending(&self) -> bool {
        self.sessions.values().any(|s| s.pending.is_some())
    }

    /// Release every socket of a session (process exited or was killed).
    pub fn destroy_session(&mut self, stack: &mut Stack, pid: i32) {
        if let Some(sess) = self.sessions.remove(&pid) {
            if let Some(q) = sess.pending.and_then(|p| p.dns) {
                stack.dns_cancel(q);
            }
            for sock in sess.sockets.into_iter().flatten() {
                close_kind(stack, sock.kind, true, 0);
            }
        }
    }

    pub fn dispatch(
        &mut self,
        stack: &mut Stack,
        pid: i32,
        syscall: i32,
        args: &[u8],
        result: &mut [u8],
        now: i64,
        fx: &mut IpcEffects,
    ) -> i32 {
        let out = Out { buf: result };
        if syscall == SOCK_DESTROY_SESSION {
            self.destroy_session(stack, pid);
            return out.status(0);
        }
        if !self.sessions.contains_key(&pid) {
            if self.sessions.len() >= MAX_SESSIONS {
                return out.status(-1);
            }
            self.sessions.insert(pid, Session::new());
        }
        let Some(sess) = self.sessions.get_mut(&pid) else { return out.status(-1) };
        // A child has one call in flight; a different syscall means the
        // previous one was abandoned (e.g. the child's thread was
        // interrupted). Release anything it held.
        if let Some(p) = &sess.pending {
            if p.syscall != syscall {
                if let Some(q) = p.dns {
                    stack.dns_cancel(q);
                }
                sess.pending = None;
            }
        }
        let a = Args(args);
        let r = match syscall {
            SOCK_SOCKET => op_socket(sess, stack, &a, out),
            SOCK_BIND => op_bind(sess, stack, &a, out),
            SOCK_CONNECT => op_connect(sess, stack, &a, out, now),
            SOCK_LISTEN => op_listen(sess, stack, &a, out),
            SOCK_ACCEPT => op_accept(sess, stack, &a, out, now),
            SOCK_SEND => op_send(sess, stack, &a, out, now),
            SOCK_RECV => op_recv(sess, stack, &a, out, now),
            SOCK_CLOSE => op_close(sess, stack, &a, out, now),
            SOCK_SETSOCKOPT => op_setsockopt(sess, &a, out),
            SOCK_SENDTO => op_sendto(sess, stack, &a, out, now, fx),
            SOCK_RECVFROM => op_recvfrom(sess, stack, &a, out, now),
            SOCK_GETADDRINFO => op_getaddrinfo(sess, stack, &a, out, now),
            SOCK_GETSOCKNAME => op_name(sess, stack, &a, out, false),
            SOCK_GETPEERNAME => op_name(sess, stack, &a, out, true),
            SOCK_SHUTDOWN => op_shutdown(sess, stack, &a, out, now),
            _ => out.status(-1),
        };
        if r != IPC_PENDING {
            if let Some(p) = &sess.pending {
                if p.syscall == syscall {
                    sess.pending = None;
                }
            }
        }
        r
    }
}

fn close_kind(stack: &mut Stack, kind: Kind, abort: bool, now: i64) {
    match kind {
        Kind::Tcp(h) if abort => stack.tcp_abort(h),
        Kind::Tcp(h) => stack.tcp_close(h, now),
        Kind::Listener(h) => stack.tcp_abort(h),
        Kind::Udp(h) => stack.udp_close(h),
        Kind::Icmp(h) => stack.icmp_close(h),
        Kind::TcpNew { .. } | Kind::Netlink { .. } => {}
    }
}

/// Record (or continue) the session's pending op and decide whether it has
/// timed out.
fn block(sess: &mut Session, syscall: i32, sock: usize, now: i64, timeout_ms: Option<i64>) -> Block {
    let same = matches!(&sess.pending, Some(p) if p.syscall == syscall && p.sock == sock);
    if !same {
        sess.pending = Some(Pending { syscall, sock, started_ms: now, dns: None, timeout_ms });
    }
    let p = sess.pending.as_ref().expect("pending just set");
    match p.timeout_ms {
        Some(t) if now - p.started_ms >= t => Block::TimedOut,
        _ => Block::Pending,
    }
}

fn sock_id(a: &Args) -> Option<usize> {
    let id = a.i32(0)?;
    (0..MAX_SOCKETS as i32).contains(&id).then_some(id as usize)
}

fn op_socket(sess: &mut Session, stack: &mut Stack, a: &Args, out: Out) -> i32 {
    let (Some(domain), Some(ty)) = (a.i32(0), a.i32(4)) else { return out.status(-1) };
    let proto = a.i32(8).unwrap_or(0);
    let kind = match (domain, ty, proto) {
        (AF_INET, SOCK_STREAM, _) => Kind::TcpNew { bind: None },
        (AF_INET, SOCK_DGRAM, _) => match stack.udp_bind(SocketAddr { ip: Ipv4Addr::ZERO, port: 0 }) {
            Ok(h) => Kind::Udp(h),
            Err(_) => return out.status(-1),
        },
        (AF_INET, SOCK_RAW, IPPROTO_ICMP) => match stack.icmp_open() {
            Ok(h) => Kind::Icmp(h),
            Err(_) => return out.status(-1),
        },
        (AF_NETLINK, _, _) => Kind::Netlink { resp: Vec::new(), off: 0 },
        _ => return out.status(-1),
    };
    match sess.alloc(Sock { kind, rcvtimeo_ms: None }) {
        Some(slot) => out.status(slot as i32),
        None => out.status(-1),
    }
}

fn op_bind(sess: &mut Session, stack: &mut Stack, a: &Args, out: Out) -> i32 {
    let Some(id) = sock_id(a) else { return out.status(-1) };
    let Some(addr) = a.0.get(4..).and_then(parse_sockaddr) else { return out.status(-1) };
    let Some(sock) = sess.sockets[id].as_mut() else { return out.status(-1) };
    match &mut sock.kind {
        Kind::TcpNew { bind } => {
            *bind = Some(addr);
            out.status(0)
        }
        Kind::Udp(h) => match stack.udp_bind(addr) {
            // Bind the new port first so a failed rebind keeps the old one.
            Ok(new) => {
                stack.udp_close(*h);
                *h = new;
                out.status(0)
            }
            Err(_) => out.status(-1),
        },
        _ => out.status(-1),
    }
}

fn op_connect(sess: &mut Session, stack: &mut Stack, a: &Args, out: Out, now: i64) -> i32 {
    let Some(id) = sock_id(a) else { return out.status(-1) };
    let Some(remote) = a.0.get(4..).and_then(parse_sockaddr) else { return out.status(-1) };
    let Some(sock) = sess.sockets[id].as_mut() else { return out.status(-1) };
    let h = match sock.kind {
        Kind::TcpNew { .. } => match stack.tcp_connect(remote, now) {
            Ok(h) => {
                sock.kind = Kind::Tcp(h);
                h
            }
            Err(_) => return out.status(-1),
        },
        Kind::Tcp(h) => h,
        _ => return out.status(-1),
    };
    match stack.tcp_state(h) {
        Ok(TcpState::Established) | Ok(TcpState::CloseWait) => out.status(0),
        Ok(TcpState::SynSent) | Ok(TcpState::SynReceived) => match block(sess, SOCK_CONNECT, id, now, Some(CONNECT_TIMEOUT_MS)) {
            Block::Pending => IPC_PENDING,
            Block::TimedOut => {
                stack.tcp_abort(h);
                sess.sockets[id] = Some(Sock { kind: Kind::TcpNew { bind: None }, rcvtimeo_ms: None });
                out.status(-1)
            }
        },
        _ => {
            // Refused / reset: give the program a fresh unconnected socket.
            stack.tcp_abort(h);
            if let Some(s) = sess.sockets[id].as_mut() {
                s.kind = Kind::TcpNew { bind: None };
            }
            out.status(-1)
        }
    }
}

fn op_listen(sess: &mut Session, stack: &mut Stack, a: &Args, out: Out) -> i32 {
    let Some(id) = sock_id(a) else { return out.status(-1) };
    let backlog = a.i32(4).unwrap_or(4).clamp(1, 32) as usize;
    let Some(sock) = sess.sockets[id].as_mut() else { return out.status(-1) };
    let Kind::TcpNew { bind: Some(addr) } = sock.kind else { return out.status(-1) };
    if addr.port == 0 {
        return out.status(-1);
    }
    match stack.tcp_listen(addr, backlog) {
        Ok(h) => {
            sock.kind = Kind::Listener(h);
            out.status(0)
        }
        Err(_) => out.status(-1),
    }
}

fn op_accept(sess: &mut Session, stack: &mut Stack, a: &Args, out: Out, now: i64) -> i32 {
    let Some(id) = sock_id(a) else { return out.status(-1) };
    let Some(Sock { kind: Kind::Listener(l), rcvtimeo_ms }) = sess.sockets[id].as_ref() else { return out.status(-1) };
    let (l, timeout) = (*l, *rcvtimeo_ms);
    if !sess.sockets.iter().any(|s| s.is_none()) {
        // No fd slot for the new connection: leave it queued in the stack.
        return out.status(-1);
    }
    match stack.tcp_accept(l) {
        Ok(Some(h)) => {
            let peer = stack.tcp_peer_addr(h).unwrap_or(SocketAddr { ip: Ipv4Addr::ZERO, port: 0 });
            match sess.alloc(Sock { kind: Kind::Tcp(h), rcvtimeo_ms: None }) {
                Some(slot) => out.with(slot as i32, &sockaddr_bytes(&peer)),
                None => {
                    stack.tcp_abort(h);
                    out.status(-1)
                }
            }
        }
        Ok(None) => match block(sess, SOCK_ACCEPT, id, now, timeout) {
            Block::Pending => IPC_PENDING,
            Block::TimedOut => out.status(-2),
        },
        Err(_) => out.status(-1),
    }
}

fn op_send(sess: &mut Session, stack: &mut Stack, a: &Args, out: Out, now: i64) -> i32 {
    let Some(id) = sock_id(a) else { return out.status(-1) };
    let Some((data, _)) = a.blob(4) else { return out.status(-1) };
    let Some(Sock { kind: Kind::Tcp(h), .. }) = sess.sockets[id].as_ref() else { return out.status(-1) };
    let h = *h;
    if data.is_empty() {
        return out.status(0);
    }
    match stack.tcp_send(h, data, now) {
        Ok(n) if n > 0 => out.status(n as i32),
        Ok(_) | Err(NetError::WouldBlock) => match block(sess, SOCK_SEND, id, now, None) {
            Block::Pending => IPC_PENDING,
            Block::TimedOut => out.status(-1),
        },
        Err(_) => out.status(-1),
    }
}

fn op_recv(sess: &mut Session, stack: &mut Stack, a: &Args, mut out: Out, now: i64) -> i32 {
    let Some(id) = sock_id(a) else { return out.status(-1) };
    let max = a.i32(4).unwrap_or(0).max(0) as usize;
    let Some(Sock { kind: Kind::Tcp(h), rcvtimeo_ms }) = sess.sockets[id].as_ref() else { return out.status(-1) };
    let (h, timeout) = (*h, *rcvtimeo_ms);
    let cap = max.min(out.cap());
    match stack.tcp_recv(h, &mut out.payload_mut()[..cap]) {
        Ok(n) => out.raw(n as i32, n), // n == 0 is EOF
        Err(NetError::WouldBlock) => match block(sess, SOCK_RECV, id, now, timeout) {
            Block::Pending => IPC_PENDING,
            Block::TimedOut => out.status(-2),
        },
        Err(_) => out.status(-1),
    }
}

fn op_close(sess: &mut Session, stack: &mut Stack, a: &Args, out: Out, now: i64) -> i32 {
    let Some(id) = sock_id(a) else { return out.status(-1) };
    if let Some(sock) = sess.sockets[id].take() {
        close_kind(stack, sock.kind, false, now);
    }
    if matches!(&sess.pending, Some(p) if p.sock == id) {
        sess.pending = None;
    }
    out.status(0)
}

fn op_setsockopt(sess: &mut Session, a: &Args, out: Out) -> i32 {
    let Some(id) = sock_id(a) else { return out.status(-1) };
    let (Some(level), Some(name)) = (a.i32(4), a.i32(8)) else { return out.status(-1) };
    let val = a.0.get(12..).unwrap_or(&[]);
    let Some(sock) = sess.sockets[id].as_mut() else { return out.status(-1) };
    if level == SOL_SOCKET && name == SO_RCVTIMEO {
        let ms: i64 = match val.len() {
            4 => i32::from_le_bytes([val[0], val[1], val[2], val[3]]) as i64,
            8 => {
                let s = i32::from_le_bytes([val[0], val[1], val[2], val[3]]) as i64;
                let us = i32::from_le_bytes([val[4], val[5], val[6], val[7]]) as i64;
                s * 1000 + us / 1000
            }
            n if n >= 16 => {
                let mut sb = [0u8; 8];
                let mut ub = [0u8; 8];
                sb.copy_from_slice(&val[0..8]);
                ub.copy_from_slice(&val[8..16]);
                i64::from_le_bytes(sb).saturating_mul(1000) + i64::from_le_bytes(ub) / 1000
            }
            _ => return out.status(-1),
        };
        // POSIX: a zero timeout means "block forever".
        sock.rcvtimeo_ms = if ms > 0 { Some(ms) } else { None };
    }
    out.status(0)
}

fn op_sendto(sess: &mut Session, stack: &mut Stack, a: &Args, out: Out, now: i64, fx: &mut IpcEffects) -> i32 {
    let Some(id) = sock_id(a) else { return out.status(-1) };
    let Some((addr, next)) = a.blob(4) else { return out.status(-1) };
    let Some((data, _)) = a.blob(next) else { return out.status(-1) };
    let Some(sock) = sess.sockets[id].as_mut() else { return out.status(-1) };
    match &mut sock.kind {
        Kind::Udp(h) => {
            let Some(dst) = parse_sockaddr(addr) else { return out.status(-1) };
            match stack.udp_send_to(*h, dst, data, now) {
                Ok(n) => out.status(n as i32),
                Err(_) => out.status(-1),
            }
        }
        Kind::Icmp(h) => {
            let Some(dst) = parse_sockaddr(addr) else { return out.status(-1) };
            match stack.icmp_send(*h, dst.ip, data, now) {
                Ok(n) => out.status(n as i32),
                Err(_) => out.status(-1),
            }
        }
        Kind::Netlink { resp, off } => {
            let r = super::netlink::handle(stack, data, now);
            fx.config_changed |= r.changed;
            fx.admin.extend(r.admin);
            *resp = r.bytes;
            *off = 0;
            out.status(data.len() as i32)
        }
        _ => out.status(-1),
    }
}

fn op_recvfrom(sess: &mut Session, stack: &mut Stack, a: &Args, mut out: Out, now: i64) -> i32 {
    let Some(id) = sock_id(a) else { return out.status(-1) };
    let max = a.i32(4).unwrap_or(0).max(0) as usize;
    let Some(sock) = sess.sockets[id].as_mut() else { return out.status(-1) };
    let timeout = Some(sock.rcvtimeo_ms.unwrap_or(DGRAM_DEFAULT_TIMEOUT_MS));
    let cap = max.min(out.cap().saturating_sub(SOCKADDR_LEN));
    let got = match &mut sock.kind {
        Kind::Udp(h) => {
            let h = *h;
            let payload = out.payload_mut();
            match stack.udp_recv_from(h, &mut payload[SOCKADDR_LEN..SOCKADDR_LEN + cap]) {
                Ok(Some((from, n))) => Some((from, n)),
                Ok(None) => None,
                Err(_) => return out.status(-1),
            }
        }
        Kind::Icmp(h) => {
            let h = *h;
            let payload = out.payload_mut();
            match stack.icmp_recv(h, &mut payload[SOCKADDR_LEN..SOCKADDR_LEN + cap]) {
                Ok(Some((from, n))) => Some((SocketAddr { ip: from, port: 0 }, n)),
                Ok(None) => None,
                Err(_) => return out.status(-1),
            }
        }
        Kind::Netlink { resp, off } => {
            let avail = resp.len().saturating_sub(*off);
            let n = avail.min(cap);
            let payload = out.payload_mut();
            payload[..SOCKADDR_LEN].fill(0);
            payload[SOCKADDR_LEN..SOCKADDR_LEN + n].copy_from_slice(&resp[*off..*off + n]);
            *off += n;
            return out.raw(n as i32, SOCKADDR_LEN + n);
        }
        _ => return out.status(-1),
    };
    match got {
        Some((from, n)) => {
            out.payload_mut()[..SOCKADDR_LEN].copy_from_slice(&sockaddr_bytes(&from));
            out.raw(n as i32, SOCKADDR_LEN + n)
        }
        None => match block(sess, SOCK_RECVFROM, id, now, timeout) {
            Block::Pending => IPC_PENDING,
            Block::TimedOut => out.status(0),
        },
    }
}

fn op_getaddrinfo(sess: &mut Session, stack: &mut Stack, a: &Args, out: Out, now: i64) -> i32 {
    let Some((name, _)) = a.blob(0) else { return out.status(-1) };
    let Ok(name) = core::str::from_utf8(name) else { return out.status(-1) };
    let name = name.trim();
    if name.is_empty() {
        return out.status(-1);
    }
    let answer = |ip: Ipv4Addr, out: Out| out.with(SOCKADDR_LEN as i32, &sockaddr_bytes(&SocketAddr { ip, port: 0 }));
    if let Some(ip) = Ipv4Addr::parse(name) {
        return answer(ip, out);
    }
    if name.eq_ignore_ascii_case("localhost") {
        return answer(Ipv4Addr::new(127, 0, 0, 1), out);
    }
    // Continue an in-flight query, or start one.
    let existing = match &sess.pending {
        Some(p) if p.syscall == SOCK_GETADDRINFO => p.dns,
        _ => None,
    };
    let q = match existing {
        Some(q) => q,
        None => match stack.dns_query(name, now) {
            Ok(q) => {
                sess.pending = Some(Pending { syscall: SOCK_GETADDRINFO, sock: usize::MAX, started_ms: now, dns: Some(q), timeout_ms: None });
                q
            }
            Err(_) => return out.status(-1),
        },
    };
    match stack.dns_poll(q) {
        DnsStatus::Pending => IPC_PENDING,
        DnsStatus::Resolved(ip) => answer(ip, out),
        DnsStatus::Failed(_) => out.status(-1),
    }
}

fn op_name(sess: &mut Session, stack: &mut Stack, a: &Args, out: Out, peer: bool) -> i32 {
    let Some(id) = sock_id(a) else { return out.status(-1) };
    let Some(sock) = sess.sockets[id].as_ref() else { return out.status(-1) };
    let addr = match (&sock.kind, peer) {
        (Kind::Tcp(h), false) | (Kind::Listener(h), false) => stack.tcp_local_addr(*h).ok(),
        (Kind::Tcp(h), true) => stack.tcp_peer_addr(*h).ok(),
        (Kind::TcpNew { bind: Some(b) }, false) => Some(*b),
        (Kind::Udp(h), false) => stack.udp_local_addr(*h).ok(),
        _ => None,
    };
    match addr {
        Some(a) => out.with(0, &sockaddr_bytes(&a)),
        None => out.status(-1),
    }
}

fn op_shutdown(sess: &mut Session, stack: &mut Stack, a: &Args, out: Out, now: i64) -> i32 {
    let Some(id) = sock_id(a) else { return out.status(-1) };
    let how = a.i32(4).unwrap_or(2);
    match sess.sockets[id].as_ref().map(|s| &s.kind) {
        Some(Kind::Tcp(h)) => {
            // SHUT_RD alone is a no-op; SHUT_WR/SHUT_RDWR send FIN once TX
            // drains. The fd stays valid until close().
            if how != SHUT_RD {
                let _ = stack.tcp_shutdown_write(*h, now);
            }
            out.status(0)
        }
        Some(_) => out.status(0),
        None => out.status(-1),
    }
}

#[cfg(test)]
#[path = "ipc_tests.rs"]
mod tests;
