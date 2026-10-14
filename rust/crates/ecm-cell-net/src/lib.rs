//! Socket plumbing shared by the Cellular v2 programs on a computer (`celld-site`,
//! `cored`, the Cell Site and Carrier Console apps): the loopback admin socket
//! (server and client, framed as in `ecm_cell::admin`), TCP/UDP helpers over the
//! kernel's sockets, and WASI randomness.
//!
//! Only programs use this crate (it calls the host ABI); the logic it serves is
//! tested natively in `ecm-cell`, `celld-site`, `cored` and `ecm-cell-ui`.

use ecm_cell::admin::{reply, Lines, Replies};
use ecm_cell::core::Entropy;
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLERR, POLLHUP, POLLIN, SOCK_DGRAM, SOCK_STREAM};
use std::time::{Duration, Instant};

pub const LOOPBACK: [u8; 4] = [127, 0, 0, 1];

/// WASI `random_get` (the host's SecureRandom).
pub struct WasiEntropy;

impl Entropy for WasiEntropy {
    fn fill(&mut self, b: &mut [u8]) {
        if getrandom::getrandom(b).is_err() {
            // No random source: fall back to the clock (never expected on a computer).
            let t = Instant::now().elapsed().as_nanos() as u64 ^ 0x9e37_79b9_7f4a_7c15;
            for (i, x) in b.iter_mut().enumerate() {
                *x = (t >> ((i % 8) * 8)) as u8 ^ i as u8;
            }
        }
    }
}

pub fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    WasiEntropy.fill(&mut b);
    b
}

/// A TCP listener on `ip:port`, or why not.
pub fn tcp_listen(ip: [u8; 4], port: u16) -> Result<i32, String> {
    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    if fd < 0 {
        return Err(if fd == -13 { "not allowed to use the network".into() } else { format!("no socket ({fd})") });
    }
    if socket::bind(fd, &SockAddrIn::from_ip_port(ip, port)) != 0 {
        socket::close(fd);
        return Err(format!("port {port} is in use"));
    }
    if socket::listen(fd, 16) != 0 {
        socket::close(fd);
        return Err(format!("can't listen on port {port}"));
    }
    Ok(fd)
}

/// A UDP socket bound to `port` on every address.
pub fn udp_bind(port: u16) -> Result<i32, String> {
    let fd = socket::socket(AF_INET, SOCK_DGRAM, 0);
    if fd < 0 {
        return Err(format!("no socket ({fd})"));
    }
    if socket::bind(fd, &SockAddrIn::any(port)) != 0 {
        socket::close(fd);
        return Err(format!("UDP port {port} is in use"));
    }
    Ok(fd)
}

/// Connect TCP (blocks until connected, refused, or the kernel's 10 s timeout).
pub fn tcp_connect(ip: [u8; 4], port: u16) -> Option<i32> {
    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    if fd < 0 {
        return None;
    }
    if socket::connect(fd, &SockAddrIn::from_ip_port(ip, port)) != 0 {
        socket::close(fd);
        return None;
    }
    Some(fd)
}

/// Send everything; false if the connection failed.
pub fn send_all(fd: i32, mut b: &[u8]) -> bool {
    while !b.is_empty() {
        let n = socket::send(fd, b, 0);
        if n <= 0 {
            return false;
        }
        b = &b[(n as usize).min(b.len())..];
    }
    true
}

/// What a non-blocking read found.
pub enum Read {
    Data(Vec<u8>),
    Nothing,
    Closed,
}

pub fn read_now(fd: i32, buf: &mut [u8]) -> Read {
    match socket::recv(fd, buf, MSG_DONTWAIT) {
        n if n > 0 => Read::Data(buf[..n as usize].to_vec()),
        -2 => Read::Nothing,
        _ => Read::Closed,
    }
}

/// Which of `fds` are readable (or hung up), waiting up to `timeout_ms`.
pub fn readable(fds: &[i32], timeout_ms: i32) -> Vec<bool> {
    let mut p: Vec<PollFd> = fds.iter().map(|&fd| PollFd { fd, events: POLLIN, revents: 0 }).collect();
    if p.is_empty() {
        if timeout_ms > 0 {
            std::thread::sleep(Duration::from_millis(timeout_ms as u64));
        }
        return Vec::new();
    }
    socket::poll(&mut p, timeout_ms);
    p.iter().map(|x| x.revents & (POLLIN | POLLHUP | POLLERR) != 0).collect()
}

/// The daemon side of the admin socket: a loopback listener and its connections.
pub struct AdminServer {
    pub listener: i32,
    conns: Vec<(i32, Lines)>,
}

impl AdminServer {
    /// Listen on 127.0.0.1:`port`. Fails when another daemon already holds the port.
    pub fn bind(port: u16) -> Result<AdminServer, String> {
        Ok(AdminServer { listener: tcp_listen(LOOPBACK, port)?, conns: Vec::new() })
    }

    /// The fds to poll (listener first).
    pub fn fds(&self) -> Vec<i32> {
        let mut v = vec![self.listener];
        v.extend(self.conns.iter().map(|c| c.0));
        v
    }

    /// Accept and read what is waiting; `handle` answers each request line.
    pub fn serve(&mut self, ready: &[bool], handle: &mut dyn FnMut(&str) -> Result<String, String>) {
        if ready.first().copied().unwrap_or(false) {
            let mut a = SockAddrIn::default();
            let fd = socket::accept(self.listener, &mut a);
            if fd >= 0 {
                if self.conns.len() >= 16 {
                    socket::close(fd);
                } else {
                    self.conns.push((fd, Lines::default()));
                }
            }
        }
        let mut buf = [0u8; 2048];
        let mut i = 0;
        while i < self.conns.len() {
            if !ready.get(i + 1).copied().unwrap_or(false) {
                i += 1;
                continue;
            }
            let fd = self.conns[i].0;
            let mut closed = false;
            match read_now(fd, &mut buf) {
                Read::Data(d) => match self.conns[i].1.push(&d) {
                    Ok(lines) => {
                        for l in lines {
                            let r = handle(&l);
                            if !send_all(fd, reply(&r).as_bytes()) {
                                closed = true;
                                break;
                            }
                        }
                    }
                    Err(_) => closed = true,
                },
                Read::Nothing => {}
                Read::Closed => closed = true,
            }
            if closed {
                socket::close(fd);
                self.conns.remove(i);
            } else {
                i += 1;
            }
        }
    }

    pub fn close(&mut self) {
        for (fd, _) in self.conns.drain(..) {
            socket::close(fd);
        }
        socket::close(self.listener);
    }
}

/// A client connection to a daemon's admin socket.
pub struct AdminClient {
    fd: i32,
    replies: Replies,
}

impl AdminClient {
    /// Connect to 127.0.0.1:`port`; None when nothing listens there (the daemon isn't running).
    pub fn connect(port: u16) -> Option<AdminClient> {
        tcp_connect(LOOPBACK, port).map(|fd| AdminClient { fd, replies: Replies::default() })
    }

    /// Send one command and wait (up to `timeout`) for its reply.
    /// Outer error: the connection failed; inner: the daemon's `err` reply.
    pub fn call(&mut self, cmd: &str, timeout: Duration) -> Result<Result<Vec<String>, String>, String> {
        if !send_all(self.fd, format!("{}\n", cmd.trim()).as_bytes()) {
            return Err("the connection to the daemon failed".into());
        }
        let start = Instant::now();
        let mut buf = [0u8; 4096];
        loop {
            let left = timeout.saturating_sub(start.elapsed());
            if left.is_zero() {
                return Err("no answer from the daemon".into());
            }
            if !readable(&[self.fd], left.as_millis().min(500) as i32)[0] {
                continue;
            }
            match read_now(self.fd, &mut buf) {
                Read::Data(d) => {
                    let mut got = self.replies.push(&d).map_err(|e| format!("bad reply: {e}"))?;
                    if !got.is_empty() {
                        return Ok(got.remove(0));
                    }
                }
                Read::Nothing => {}
                Read::Closed => return Err("the daemon closed the connection".into()),
            }
        }
    }

    pub fn close(self) {
        socket::close(self.fd);
    }
}

/// One command to the daemon on `port`, or why not (`not running` when nothing listens).
pub fn admin_call(port: u16, cmd: &str) -> Result<Vec<String>, String> {
    let mut c = AdminClient::connect(port).ok_or_else(|| "not running".to_string())?;
    let r = c.call(cmd, Duration::from_secs(10));
    c.close();
    r?
}
