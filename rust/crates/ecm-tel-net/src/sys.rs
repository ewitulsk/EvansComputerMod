//! The computer side (WASI): sockets, the HSS client, DNS (ENUM, SRV, MX),
//! an HTTP server that can answer later, HTTP and SMTP clients, logs and CDRs.

use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::time::{Duration, Instant};

use ecm_cell::admin::Replies;
use ecm_dns::wire::{Message, RData, A, MX, NAPTR, SRV};
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLERR, POLLHUP, POLLIN, SOCK_DGRAM, SOCK_STREAM};
use ecm_tel::txn::Addr;

pub fn sa(a: Addr) -> SockAddrIn {
    SockAddrIn::from_ip_port(a.ip, a.port)
}

pub fn addr(s: &SockAddrIn) -> Addr {
    Addr::new(s.ip(), s.port())
}

/// A UDP socket bound to `port` (all addresses).
pub fn udp_bind(port: u16) -> Result<i32, String> {
    let fd = socket::socket(AF_INET, SOCK_DGRAM, 0);
    if fd < 0 {
        return Err(if fd == -13 { "not allowed to use the network".into() } else { format!("no socket ({fd})") });
    }
    socket::setsockopt(fd, socket::SOL_SOCKET, socket::SO_REUSEADDR, &1i32.to_le_bytes());
    if socket::bind(fd, &SockAddrIn::any(port)) != 0 {
        socket::close(fd);
        return Err(format!("UDP port {port} is in use (is it already running?)"));
    }
    Ok(fd)
}

/// Every datagram waiting on `fd` (at most `max`).
pub fn udp_drain(fd: i32, buf: &mut [u8], max: usize) -> Vec<(Addr, Vec<u8>)> {
    let mut v = Vec::new();
    for _ in 0..max {
        let mut from = SockAddrIn::default();
        let n = socket::recvfrom(fd, buf, MSG_DONTWAIT, &mut from);
        if n <= 0 {
            break;
        }
        v.push((addr(&from), buf[..n as usize].to_vec()));
    }
    v
}

pub fn udp_send(fd: i32, to: Addr, data: &[u8]) {
    socket::sendto(fd, data, 0, &sa(to));
}

/// Which `fds` are readable within `timeout_ms`.
pub fn readable(fds: &[i32], timeout_ms: i32) -> Vec<bool> {
    if fds.is_empty() {
        std::thread::sleep(Duration::from_millis(timeout_ms.max(0) as u64));
        return Vec::new();
    }
    let mut p: Vec<PollFd> = fds.iter().map(|&fd| PollFd { fd, events: POLLIN, revents: 0 }).collect();
    socket::poll(&mut p, timeout_ms);
    p.iter().map(|x| x.revents & (POLLIN | POLLHUP | POLLERR) != 0).collect()
}

pub fn tcp_listen(port: u16) -> Result<i32, String> {
    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    if fd < 0 {
        return Err(format!("no socket ({fd})"));
    }
    socket::setsockopt(fd, socket::SOL_SOCKET, socket::SO_REUSEADDR, &1i32.to_le_bytes());
    if socket::bind(fd, &SockAddrIn::any(port)) != 0 || socket::listen(fd, 16) != 0 {
        socket::close(fd);
        return Err(format!("TCP port {port} is in use"));
    }
    Ok(fd)
}

pub fn tcp_connect(to: Addr) -> Option<i32> {
    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    if fd < 0 {
        return None;
    }
    if socket::connect(fd, &sa(to)) != 0 {
        socket::close(fd);
        return None;
    }
    Some(fd)
}

pub fn send_all(fd: i32, mut b: &[u8]) -> bool {
    while !b.is_empty() {
        let n = socket::send(fd, b, 0);
        if n == -2 {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }
        if n <= 0 {
            return false;
        }
        b = &b[(n as usize).min(b.len())..];
    }
    true
}

/// Bytes waiting (`Some(empty)`: none now), or None when the peer closed.
pub fn read_now(fd: i32, buf: &mut [u8]) -> Option<Vec<u8>> {
    match socket::recv(fd, buf, MSG_DONTWAIT) {
        n if n > 0 => Some(buf[..n as usize].to_vec()),
        -2 => Some(Vec::new()),
        _ => None,
    }
}

// ------------------------------------------------------------ HSS client

/// A connection to `cored`'s HSS port (ecm_cell::hss), asked asynchronously.
pub struct HssClient {
    pub addr: Addr,
    origin: String,
    secret: String,
    fd: Option<i32>,
    replies: Replies,
    /// Requests sent, oldest first (replies come in order).
    inflight: VecDeque<u64>,
    /// Requests waiting for a connection.
    queue: VecDeque<(u64, String)>,
    done: Vec<(u64, Result<String, String>)>,
    retry_at: Option<Instant>,
    pub last_error: String,
}

impl HssClient {
    pub fn new(addr: Addr, origin: &str, secret: &str) -> HssClient {
        HssClient { addr, origin: origin.into(), secret: secret.into(), fd: None, replies: Replies::default(), inflight: VecDeque::new(), queue: VecDeque::new(), done: Vec::new(), retry_at: None, last_error: String::new() }
    }

    pub fn fd(&self) -> Option<i32> {
        self.fd
    }

    pub fn connected(&self) -> bool {
        self.fd.is_some()
    }

    fn connect(&mut self) {
        if self.fd.is_some() || self.retry_at.is_some_and(|t| Instant::now() < t) {
            return;
        }
        match tcp_connect(self.addr) {
            Some(fd) => {
                let proof = ecm_cell::hss::peer_proof(self.secret.as_bytes(), &self.origin);
                if send_all(fd, format!("CER {} {}\n", self.origin, proof).as_bytes()) {
                    self.fd = Some(fd);
                    self.inflight.push_back(0);
                    self.last_error.clear();
                } else {
                    socket::close(fd);
                }
            }
            None => {
                self.last_error = format!("the HSS at {} doesn't answer (is cored running?)", self.addr);
                self.retry_at = Some(Instant::now() + Duration::from_secs(3));
            }
        }
    }

    fn fail_all(&mut self, why: &str) {
        for id in self.inflight.drain(..) {
            if id != 0 {
                self.done.push((id, Err(why.into())));
            }
        }
        if let Some(fd) = self.fd.take() {
            socket::close(fd);
        }
        self.replies = Replies::default();
        self.retry_at = Some(Instant::now() + Duration::from_secs(2));
    }

    pub fn request(&mut self, id: u64, line: &str) {
        self.queue.push_back((id, line.to_string()));
        self.pump();
    }

    /// Send what waits, read what came; returns finished requests.
    pub fn pump(&mut self) -> Vec<(u64, Result<String, String>)> {
        if !self.queue.is_empty() {
            self.connect();
        }
        if let Some(fd) = self.fd {
            while let Some((id, line)) = self.queue.pop_front() {
                if !send_all(fd, format!("{}\n", line).as_bytes()) {
                    self.queue.push_front((id, line));
                    self.fail_all("the HSS connection failed");
                    break;
                }
                self.inflight.push_back(id);
            }
        } else if self.retry_at.is_some() && !self.queue.is_empty() && self.retry_at.is_some_and(|t| Instant::now() >= t + Duration::from_secs(5)) {
            // still unreachable after a while: give up on what waits
            let e = self.last_error.clone();
            for (id, _) in self.queue.drain(..) {
                self.done.push((id, Err(e.clone())));
            }
        }
        if let Some(fd) = self.fd {
            let mut buf = [0u8; 8192];
            loop {
                match read_now(fd, &mut buf) {
                    Some(b) if b.is_empty() => break,
                    Some(b) => match self.replies.push(&b) {
                        Ok(rs) => {
                            for r in rs {
                                let id = self.inflight.pop_front().unwrap_or(0);
                                if id == 0 {
                                    if let Err(e) = r {
                                        self.last_error = format!("the HSS refused us: {e}");
                                        self.fail_all(&format!("the HSS refused us: {e}"));
                                        return std::mem::take(&mut self.done);
                                    }
                                    continue;
                                }
                                self.done.push((id, r.map(|lines| lines.join("\n") + "\n")));
                            }
                        }
                        Err(_) => {
                            self.fail_all("bad reply from the HSS");
                            break;
                        }
                    },
                    None => {
                        self.fail_all("the HSS closed the connection");
                        break;
                    }
                }
            }
        }
        std::mem::take(&mut self.done)
    }
}

// ------------------------------------------------------------ DNS: ENUM, SRV, MX, A

fn dns(name: &str, qtype: u16) -> Vec<ecm_dns::wire::Record> {
    let Some(server) = ecm_dns::sys::default_server() else { return Vec::new() };
    let q = Message::query(ecm_dns::sys::random_id(), name, qtype, true);
    match ecm_dns::sys::query(server, 53, &q, 2500) {
        Ok(a) => a.msg.answers.into_iter().chain(a.msg.additional).collect(),
        Err(_) => Vec::new(),
    }
}

/// The IPv4 address of `host` (a literal or an A lookup).
pub fn resolve_a(host: &str) -> Option<[u8; 4]> {
    if let Some(ip) = Addr::parse_ip(host) {
        return Some(ip);
    }
    let h = host.trim_end_matches('.');
    dns(h, A).into_iter().find_map(|r| match r.data {
        RData::A(ip) => Some(ip),
        _ => None,
    })
}

/// ENUM: the SIP domain serving `e164` (NAPTR under `e164.arpa`), and its
/// border from `_sip._udp.<domain>` SRV.
pub fn enum_lookup(e164: &str) -> Option<ecm_tel::ims::EnumAnswer> {
    let name = ecm_dns::enum_dns::enum_domain(e164, ecm_dns::enum_dns::E164_ARPA);
    let recs = dns(&name, NAPTR);
    // a number record, else its block's
    let recs = if recs.is_empty() {
        let digits: String = e164.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() > 4 {
            let block = format!("+{}", &digits[..digits.len() - 4]);
            dns(&ecm_dns::enum_dns::enum_block_domain(&block, ecm_dns::enum_dns::E164_ARPA), NAPTR)
        } else {
            recs
        }
    } else {
        recs
    };
    let uri = ecm_dns::enum_dns::enum_sip_uri(ecm_dns::enum_dns::naptrs(&recs), e164)?;
    let domain = uri.split('@').nth(1)?.split([';', '>', ':']).next()?.to_string();
    let srv = dns(&format!("_sip._udp.{}", domain), SRV);
    let border = srv.into_iter().find_map(|r| match r.data {
        RData::Srv(s) => resolve_a(&s.target).map(|ip| Addr::new(ip, s.port)),
        _ => None,
    });
    Some(ecm_tel::ims::EnumAnswer { domain, border })
}

/// The mail exchanger of `domain` (MX, else the domain's own A).
pub fn mx_lookup(domain: &str) -> Option<[u8; 4]> {
    let mut mx: Vec<(u16, String)> = dns(domain, MX).into_iter().filter_map(|r| if let RData::Mx(p, h) = r.data { Some((p, h)) } else { None }).collect();
    mx.sort();
    mx.into_iter().find_map(|(_, h)| resolve_a(&h)).or_else(|| resolve_a(domain))
}

/// Send one message by SMTP to `host:25`; `Ok` when the server accepted it.
pub fn smtp_send(host: [u8; 4], helo: &str, from: &str, rcpt: &[String], message: &str) -> Result<(), String> {
    let fd = tcp_connect(Addr::new(host, 25)).ok_or("the MMSC's mail port doesn't answer")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut buf = [0u8; 2048];
    let mut pending = String::new();
    let mut read_reply = |pending: &mut String| -> Result<String, String> {
        loop {
            if let Some(i) = pending.find('\n') {
                let l: String = pending.drain(..=i).collect();
                let l = l.trim_end().to_string();
                if l.len() >= 4 && l.as_bytes()[3] == b'-' {
                    continue;
                }
                return Ok(l);
            }
            if Instant::now() > deadline {
                return Err("SMTP timed out".into());
            }
            readable(&[fd], 200);
            match read_now(fd, &mut buf) {
                Some(b) => pending.push_str(&String::from_utf8_lossy(&b)),
                None => return Err("SMTP connection closed".into()),
            }
        }
    };
    let r = (|| {
        let g = read_reply(&mut pending)?;
        if !g.starts_with("220") {
            return Err(g);
        }
        for line in crate::smtp::client_script(helo, from, rcpt, message) {
            if !send_all(fd, format!("{}\r\n", line).as_bytes()) {
                return Err("SMTP send failed".into());
            }
            let r = read_reply(&mut pending)?;
            if !(r.starts_with('2') || r.starts_with('3')) {
                return Err(r);
            }
        }
        Ok(())
    })();
    socket::close(fd);
    r
}

// ------------------------------------------------------------ HTTP

/// An HTTP request client side (blocking up to `timeout`).
pub fn http_request(method: &str, url: &str, content_type: Option<&str>, body: &[u8], timeout: Duration) -> Result<crate::http::Response, String> {
    let (host, port, path) = crate::http::split_url(url).ok_or("bad URL")?;
    let ip = resolve_a(&host).ok_or_else(|| format!("can't find {host}"))?;
    let fd = tcp_connect(Addr::new(ip, port)).ok_or_else(|| format!("{host}:{port} doesn't answer"))?;
    let req = crate::http::encode_request(method, &host, &path, content_type, body);
    if !send_all(fd, &req) {
        socket::close(fd);
        return Err("sending failed".into());
    }
    let deadline = Instant::now() + timeout;
    let mut got = Vec::new();
    let mut buf = vec![0u8; 16384];
    let r = loop {
        if Instant::now() > deadline {
            break Err("no answer in time".to_string());
        }
        readable(&[fd], 200);
        match read_now(fd, &mut buf) {
            Some(b) => {
                got.extend_from_slice(&b);
                if let Ok(Some(r)) = crate::http::parse_response(&got, false) {
                    break Ok(r);
                }
            }
            None => break crate::http::parse_response(&got, true).and_then(|r| r.ok_or_else(|| "no answer".to_string())),
        }
    };
    socket::close(fd);
    r
}

/// A non-blocking HTTP server whose handler may answer later.
pub struct HttpServer {
    pub listener: i32,
    conns: BTreeMap<u64, (i32, Vec<u8>, [u8; 4], bool)>,
    next: u64,
}

impl HttpServer {
    pub fn bind(port: u16) -> Result<HttpServer, String> {
        Ok(HttpServer { listener: tcp_listen(port)?, conns: BTreeMap::new(), next: 1 })
    }

    pub fn fds(&self) -> Vec<i32> {
        let mut v = vec![self.listener];
        v.extend(self.conns.values().map(|c| c.0));
        v
    }

    /// Accept and read; returns complete requests as (connection, from, request).
    pub fn poll(&mut self, ready: &[bool]) -> Vec<(u64, [u8; 4], crate::http::Request)> {
        let mut out = Vec::new();
        if ready.first().copied().unwrap_or(false) {
            let mut a = SockAddrIn::default();
            let fd = socket::accept(self.listener, &mut a);
            if fd >= 0 {
                let id = self.next;
                self.next += 1;
                self.conns.insert(id, (fd, Vec::new(), a.ip(), false));
            }
        }
        let ids: Vec<u64> = self.conns.keys().copied().collect();
        let mut buf = vec![0u8; 16384];
        for (i, id) in ids.iter().enumerate() {
            if !ready.get(i + 1).copied().unwrap_or(true) {
                continue;
            }
            let (fd, _, _, done) = self.conns[id];
            if done {
                continue;
            }
            match read_now(fd, &mut buf) {
                Some(b) if b.is_empty() => {}
                Some(b) => {
                    let c = self.conns.get_mut(id).expect("listed");
                    c.1.extend_from_slice(&b);
                    match crate::http::parse_request(&c.1) {
                        Ok(Some((req, _))) => {
                            c.3 = true;
                            out.push((*id, c.2, req));
                        }
                        Ok(None) => {}
                        Err(_) => self.reply(*id, 400, "text/plain", b"bad request\n"),
                    }
                }
                None => {
                    socket::close(fd);
                    self.conns.remove(id);
                }
            }
        }
        out
    }

    pub fn reply(&mut self, conn: u64, status: u16, content_type: &str, body: &[u8]) {
        if let Some((fd, ..)) = self.conns.remove(&conn) {
            send_all(fd, &crate::http::encode_response(status, content_type, body));
            socket::shutdown(fd, socket::SHUT_WR);
            socket::close(fd);
        }
    }
}

// ------------------------------------------------------------ logs and CDRs

/// A program's log: `/var/log/<name>.log` (rotated at 64 KB), the last lines in memory.
pub struct Log {
    pub path: String,
    pub lines: VecDeque<String>,
    start: Instant,
    echo: bool,
}

impl Log {
    pub fn new(name: &str, echo: bool) -> Log {
        let _ = std::fs::create_dir_all("/var/log");
        Log { path: format!("/var/log/{}.log", name), lines: VecDeque::new(), start: Instant::now(), echo }
    }

    pub fn line(&mut self, s: &str) {
        let t = self.start.elapsed().as_millis();
        let l = format!("[{:>6}.{:01}] {}", t / 1000, (t % 1000) / 100, s);
        if self.echo {
            println!("{}", s);
        }
        if std::fs::metadata(&self.path).map(|m| m.len() > 64 * 1024).unwrap_or(false) {
            let _ = std::fs::rename(&self.path, format!("{}.1", self.path));
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(f, "{}", l);
        }
        if self.lines.len() >= 300 {
            self.lines.pop_front();
        }
        self.lines.push_back(l);
    }

    pub fn tail(&self, n: usize) -> String {
        let skip = self.lines.len().saturating_sub(n);
        self.lines.iter().skip(skip).cloned().collect::<Vec<_>>().join("\n")
    }
}

/// Append a CDR line to `/var/log/cdr/<program>.cdr`.
pub fn cdr(program: &str, line: &str) {
    let _ = std::fs::create_dir_all("/var/log/cdr");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(format!("/var/log/cdr/{}.cdr", program)) {
        let _ = writeln!(f, "{}", line);
    }
}

/// Write a file, making its directory.
pub fn write_file(path: &str, data: &[u8]) -> Result<(), String> {
    if let Some(d) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::write(path, data).map_err(|e| format!("can't write {path}: {e}"))
}

/// `key = value` config file, strict about keys (`keys`); missing file = defaults.
pub fn load_conf(path: &str, keys: &[(&str, &str)]) -> Result<BTreeMap<String, String>, String> {
    let mut m: BTreeMap<String, String> = keys.iter().map(|(k, d)| (k.to_string(), d.to_string())).collect();
    let Ok(text) = std::fs::read_to_string(path) else { return Ok(m) };
    for l in ecm_cell::admin::parse_conf(&text).map_err(|e| format!("{path}: {e}"))? {
        if !keys.iter().any(|(k, _)| *k == l.key) {
            return Err(format!("{path} line {}: unknown key '{}'", l.line, l.key));
        }
        m.insert(l.key, l.value);
    }
    Ok(m)
}

/// This computer's IPv4 addresses (not loopback).
pub fn own_addresses() -> Vec<[u8; 4]> {
    ecm_dns::sys::own_addresses()
}

/// Ask the HSS `NUM` (the carrier, domain, P-CSCF, IMS pool, MMSC) once it
/// answers. `wait`: keep trying (a program started at boot before cored);
/// otherwise give up after a few seconds.
pub fn numbering(hss: &mut HssClient, wait: bool) -> Result<String, String> {
    let started = Instant::now();
    loop {
        hss.request(1, "NUM");
        for _ in 0..40 {
            match hss.fd() {
                Some(fd) => {
                    readable(&[fd], 50);
                }
                None => std::thread::sleep(Duration::from_millis(50)),
            }
            for (id, r) in hss.pump() {
                if id == 1 {
                    match r {
                        Ok(b) => return Ok(b),
                        Err(e) if !wait => return Err(format!("the HSS: {e}")),
                        Err(_) => {}
                    }
                }
            }
        }
        if !wait && started.elapsed() > Duration::from_secs(5) {
            return Err(if hss.last_error.is_empty() { "the HSS doesn't answer".into() } else { hss.last_error.clone() });
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// `key value` from a reply body.
pub fn kv(body: &str, k: &str) -> String {
    ecm_tel::ims::kv(body, k)
}

/// Add or remove a `services.cfg` line starting with `prefix` (`enable` / `disable`).
pub fn service_line(prefix: &str, line: &str, on: bool) -> Result<(), String> {
    let t = std::fs::read_to_string("/services.cfg").unwrap_or_default();
    let mut lines: Vec<String> = t.lines().filter(|l| !l.trim().starts_with(prefix)).map(String::from).collect();
    if on {
        lines.push(line.into());
    }
    write_file("/services.cfg", format!("{}\n", lines.join("\n")).as_bytes())
}

/// Save a config map as `key = "value"` lines (only what differs from the defaults).
pub fn save_conf(path: &str, title: &str, keys: &[(&str, &str)], c: &BTreeMap<String, String>) -> Result<(), String> {
    let mut t = format!("# {title}\n");
    for (k, d) in keys {
        let v = c.get(*k).cloned().unwrap_or_default();
        if v != *d {
            t.push_str(&format!("{} = \"{}\"\n", k, v));
        }
    }
    write_file(path, t.as_bytes())
}
