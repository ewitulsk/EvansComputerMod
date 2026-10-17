//! DNS over the computer's sockets (wasm32 only): one query over UDP (TCP when the
//! answer is truncated), AXFR over TCP, a blocking driver for the [`Resolver`], the
//! `named` control channel, and the stub resolver's `/etc/resolv.conf`.

use crate::name::{ip_str, parse_ip, Ip};
use crate::resolver::{QueryId, Resolution, Resolver};
use crate::wire::Message;
use crate::xfr::{frame, Deframer};
use ecm_host_abi::socket::{
    self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLIN, SOCK_DGRAM, SOCK_STREAM, SOL_SOCKET, SO_RCVTIMEO, SO_SNDTIMEO,
};
use std::time::Instant;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn seed() -> u32 {
    let mut b = [0u8; 4];
    ecm_host_abi::random::fill(&mut b);
    u32::from_le_bytes(b)
}

pub fn random_id() -> u16 {
    seed() as u16
}

/// The stub resolver's settings, as `/etc/resolv.conf` has them.
#[derive(Clone, Debug, Default)]
pub struct ResolvConf {
    pub nameservers: Vec<Ip>,
    pub search: Vec<String>,
    pub ndots: usize,
}

pub fn resolv_conf() -> ResolvConf {
    let text = std::fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
    let mut c = ResolvConf { ndots: 1, ..Default::default() };
    for line in text.lines() {
        let line = line.split(['#', ';']).next().unwrap_or("").trim();
        let mut w = line.split_whitespace();
        match w.next() {
            Some("nameserver") => c.nameservers.extend(w.next().and_then(parse_ip)),
            Some("search") => c.search = w.map(|d| d.trim_end_matches('.').to_ascii_lowercase()).collect(),
            Some("domain") => c.search = w.next().map(|d| d.trim_end_matches('.').to_ascii_lowercase()).into_iter().collect(),
            Some("options") => {
                for o in w {
                    if let Some(n) = o.strip_prefix("ndots:").and_then(|n| n.parse().ok()) {
                        c.ndots = n;
                    }
                }
            }
            _ => {}
        }
    }
    c
}

impl ResolvConf {
    /// Names to try for `name` (the same rules as the kernel's stub resolver).
    pub fn candidates(&self, name: &str) -> Vec<String> {
        if let Some(abs) = name.strip_suffix('.') {
            return vec![abs.to_string()];
        }
        let dots = name.matches('.').count();
        let searched = self.search.iter().map(|d| format!("{name}.{d}"));
        if dots >= self.ndots {
            std::iter::once(name.to_string()).chain(searched).collect()
        } else {
            searched.chain(std::iter::once(name.to_string())).collect()
        }
    }
}

/// The default server: the first `nameserver` of `/etc/resolv.conf`.
pub fn default_server() -> Option<Ip> {
    resolv_conf().nameservers.first().copied()
}

/// Server given as an address or a name (resolved through the kernel).
pub fn server_addr(s: &str) -> Result<Ip, String> {
    if let Some(ip) = parse_ip(s) {
        return Ok(ip);
    }
    socket::resolve(s).map_err(|e| format!("cannot resolve server {s}: {}", e.describe()))
}

pub struct Answer {
    pub msg: Message,
    pub bytes: usize,
    pub ms: i64,
    pub tcp: bool,
}

/// One query and its answer: UDP, then TCP when the answer came back truncated.
pub fn query(server: Ip, port: u16, msg: &Message, timeout_ms: i64) -> Result<Answer, String> {
    let fd = socket::socket(AF_INET, SOCK_DGRAM, 0);
    if fd == -13 {
        return Err("not allowed to use the network".into());
    }
    if fd < 0 {
        return Err("cannot open a socket".into());
    }
    socket::bind(fd, &SockAddrIn::any(0));
    let start = Instant::now();
    let data = msg.encode();
    if socket::sendto(fd, &data, 0, &SockAddrIn::from_ip_port(server, port)) < 0 {
        socket::close(fd);
        return Err(format!("cannot send to {} (no route?)", ip_str(server)));
    }
    let mut buf = vec![0u8; 4096];
    loop {
        let left = timeout_ms - start.elapsed().as_millis() as i64;
        if left <= 0 {
            socket::close(fd);
            return Err(format!("no answer from {} in {} ms", ip_str(server), timeout_ms));
        }
        let mut fds = [PollFd { fd, events: POLLIN, revents: 0 }];
        if socket::poll(&mut fds, left.min(250) as i32) <= 0 || fds[0].revents & POLLIN == 0 {
            continue;
        }
        let mut from = SockAddrIn::default();
        let n = socket::recvfrom(fd, &mut buf, MSG_DONTWAIT, &mut from);
        if n <= 0 || from.ip() != server {
            continue;
        }
        let Ok(m) = Message::decode(&buf[..n as usize]) else { continue };
        if m.header.id != msg.header.id || !m.header.qr {
            continue;
        }
        socket::close(fd);
        if m.header.tc {
            if let Ok(mut t) = tcp_query(server, port, msg, timeout_ms) {
                if let Some(m) = t.pop() {
                    return Ok(Answer { bytes: m.encode().len(), msg: m, ms: start.elapsed().as_millis() as i64, tcp: true });
                }
            }
        }
        return Ok(Answer { msg: m, bytes: n as usize, ms: start.elapsed().as_millis() as i64, tcp: false });
    }
}

fn tcp_connect(server: Ip, port: u16, timeout_ms: i64) -> Result<i32, String> {
    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    if fd < 0 {
        return Err("cannot open a socket".into());
    }
    let t = (timeout_ms.clamp(100, 60_000) as i32).to_le_bytes();
    socket::setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &t);
    socket::setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &t);
    if socket::connect(fd, &SockAddrIn::from_ip_port(server, port)) != 0 {
        socket::close(fd);
        return Err(format!("cannot connect to {}:{}", ip_str(server), port));
    }
    Ok(fd)
}

pub fn send_all(fd: i32, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        let n = socket::send(fd, &data[..data.len().min(1400)], 0);
        if n <= 0 {
            return false;
        }
        data = &data[n as usize..];
    }
    true
}

/// A query over TCP. For AXFR the messages up to the closing SOA, otherwise the one
/// answer.
pub fn tcp_query(server: Ip, port: u16, msg: &Message, timeout_ms: i64) -> Result<Vec<Message>, String> {
    let fd = tcp_connect(server, port, timeout_ms)?;
    let axfr = msg.question().is_some_and(|q| q.qtype == crate::wire::AXFR);
    if !send_all(fd, &frame(&msg.encode())) {
        socket::close(fd);
        return Err("connection lost while sending".into());
    }
    let start = Instant::now();
    let mut d = Deframer::default();
    let mut out: Vec<Message> = Vec::new();
    let mut soas = 0;
    let mut buf = vec![0u8; 8192];
    let result = loop {
        if start.elapsed().as_millis() as i64 > timeout_ms {
            break Err(format!("{} stopped answering", ip_str(server)));
        }
        let n = socket::recv(fd, &mut buf, 0);
        if n == -2 {
            continue;
        }
        if n <= 0 {
            break if out.is_empty() { Err(format!("{} closed the connection", ip_str(server))) } else { Ok(()) };
        }
        let mut done = false;
        for raw in d.push(&buf[..n as usize]) {
            let m = Message::decode(&raw).map_err(|e| format!("bad message: {e:?}"))?;
            if m.header.id != msg.header.id {
                continue;
            }
            soas += m.answers.iter().filter(|r| r.rtype == crate::wire::SOA).count();
            let failed = m.header.rcode != 0;
            out.push(m);
            if !axfr || failed || soas >= 2 {
                done = true;
            }
        }
        if done {
            break Ok(());
        }
    };
    socket::close(fd);
    result.map(|_| out)
}

/// Fetch a whole zone by AXFR.
pub fn axfr(server: Ip, zone: &str, timeout_ms: i64) -> Result<crate::zone::Zone, String> {
    let mut c = crate::xfr::AxfrClient::new(zone, random_id());
    let msgs = tcp_query(server, 53, &c.query(), timeout_ms)?;
    for m in msgs {
        if let Some(z) = c.feed(&m.encode())? {
            return Ok(z);
        }
    }
    Err("the transfer ended before the closing SOA".into())
}

/// Run the resolver until `q` is answered (or `timeout_ms` passes), on a socket of
/// its own. `on_send` sees every datagram as it goes (for progress output).
pub fn drive(res: &mut Resolver, q: QueryId, timeout_ms: i64) -> Result<Resolution, String> {
    let fd = socket::socket(AF_INET, SOCK_DGRAM, 0);
    if fd < 0 {
        return Err("cannot open a socket".into());
    }
    socket::bind(fd, &SockAddrIn::any(0));
    let start = now_ms();
    let mut buf = vec![0u8; 4096];
    let result = loop {
        for (to, data) in res.take_sends() {
            socket::sendto(fd, &data, 0, &SockAddrIn::from_ip_port(to, 53));
        }
        if let Some(r) = res.take_done().into_iter().find(|(id, _)| *id == q) {
            break Ok(r.1);
        }
        let now = now_ms();
        if now - start > timeout_ms {
            break Err("gave up: no answer in time".into());
        }
        let wait = res.next_deadline().map_or(200, |d| (d - now).clamp(1, 200));
        let mut fds = [PollFd { fd, events: POLLIN, revents: 0 }];
        if socket::poll(&mut fds, wait as i32) > 0 && fds[0].revents & POLLIN != 0 {
            let mut from = SockAddrIn::default();
            let n = socket::recvfrom(fd, &mut buf, MSG_DONTWAIT, &mut from);
            if n > 0 && from.port() == 53 {
                res.on_packet(from.ip(), &buf[..n as usize], now_ms());
            }
        }
        res.tick(now_ms());
    };
    socket::close(fd);
    result
}

/// End of a control reply.
pub const CONTROL_END: &str = "\n.\n";

/// `named <command>`: send it to the running daemon on 127.0.0.1 and return its reply.
pub fn control(command: &str, port: u16) -> Result<String, String> {
    let fd = socket::socket(AF_INET, SOCK_DGRAM, 0);
    if fd < 0 {
        return Err("cannot open a socket".into());
    }
    socket::bind(fd, &SockAddrIn::any(0));
    socket::sendto(fd, command.as_bytes(), 0, &SockAddrIn::new(127, 0, 0, 1, port));
    let start = Instant::now();
    let mut out = String::new();
    let mut buf = vec![0u8; 2048];
    let r = loop {
        if start.elapsed().as_millis() > 3000 {
            break Err("named is not running (no answer on the control port)".to_string());
        }
        let mut fds = [PollFd { fd, events: POLLIN, revents: 0 }];
        if socket::poll(&mut fds, 250) <= 0 {
            continue;
        }
        let mut from = SockAddrIn::default();
        let n = socket::recvfrom(fd, &mut buf, MSG_DONTWAIT, &mut from);
        if n > 0 {
            out.push_str(&String::from_utf8_lossy(&buf[..n as usize]));
            if let Some(body) = out.strip_suffix(CONTROL_END) {
                break Ok(body.to_string());
            }
        }
    };
    socket::close(fd);
    r
}

/// The control reply as datagrams (each under one Ethernet frame).
pub fn control_chunks(text: &str) -> Vec<Vec<u8>> {
    let full = format!("{}{}", text.trim_end_matches('\n'), CONTROL_END);
    full.as_bytes().chunks(1200).map(|c| c.to_vec()).collect()
}

/// This computer's IPv4 addresses.
pub fn own_addresses() -> Vec<Ip> {
    use ecm_host_abi::net_config;
    (0..net_config::iface_count())
        .filter_map(|i| {
            let info = net_config::iface_info(i)?;
            let at = info.find("\"ip\":")? + 5;
            let rest = info[at..].trim_start().trim_start_matches('"');
            let end = rest.find(|c: char| c == '"' || c == ',' || c == '}').unwrap_or(rest.len());
            parse_ip(rest[..end].trim()).filter(|ip| *ip != [0, 0, 0, 0])
        })
        .collect()
}

/// Write a file, making its directory first.
pub fn write_file(path: &str, text: &str) -> bool {
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(path, text).is_ok()
}

pub fn read_file(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

// ------------------------------------------------------------------ small HTTP + line servers

/// A listening TCP socket on `port` (all addresses).
pub fn listen(port: u16) -> Result<i32, String> {
    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    if fd < 0 {
        return Err("cannot open a socket".into());
    }
    socket::setsockopt(fd, SOL_SOCKET, ecm_host_abi::socket::SO_REUSEADDR, &1i32.to_le_bytes());
    if socket::bind(fd, &SockAddrIn::any(port)) != 0 || socket::listen(fd, 8) != 0 {
        socket::close(fd);
        return Err(format!("cannot listen on TCP port {port} (already in use?)"));
    }
    Ok(fd)
}

/// One HTTP request, read whole (headers and a Content-Length body).
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub body: String,
    /// The `Cookie:` header ("" when none).
    pub cookie: String,
}

pub fn read_request(fd: i32, timeout_ms: i64) -> Option<Request> {
    let start = Instant::now();
    let mut data = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        if start.elapsed().as_millis() as i64 > timeout_ms || data.len() > 65536 {
            return None;
        }
        let n = socket::recv(fd, &mut buf, 0);
        if n == -2 {
            continue;
        }
        if n <= 0 {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&data[..end]).to_string();
            let len = head
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0);
            if data.len() >= end + 4 + len {
                let first = head.lines().next().unwrap_or("");
                let mut w = first.split_whitespace();
                let method = w.next().unwrap_or("").to_string();
                let target = w.next().unwrap_or("/").to_string();
                let (path, query) = target.split_once('?').map_or((target.clone(), String::new()), |(p, q)| (p.to_string(), q.to_string()));
                let body = String::from_utf8_lossy(&data[end + 4..end + 4 + len]).to_string();
                let cookie = head
                    .lines()
                    .find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("cookie")).map(|(_, v)| v.trim().to_string()))
                    .unwrap_or_default();
                return Some(Request { method, path, query, body, cookie });
            }
        }
    }
    None
}

/// [`reply`] with more header lines (`Set-Cookie`, `Location`).
pub fn reply_with(fd: i32, status: &str, content_type: &str, headers: &[(&str, &str)], body: &str) {
    let mut head = format!("HTTP/1.0 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    send_all(fd, head.as_bytes());
    send_all(fd, body.as_bytes());
    socket::shutdown(fd, socket::SHUT_WR);
}

pub fn reply(fd: i32, status: &str, content_type: &str, body: &str) {
    let head = format!(
        "HTTP/1.0 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    send_all(fd, head.as_bytes());
    send_all(fd, body.as_bytes());
    socket::shutdown(fd, socket::SHUT_WR);
}

/// `http://host[:port]/path` -> (host, port, path).
pub fn split_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url.strip_prefix("http://").ok_or_else(|| format!("'{url}' is not an http:// address"))?;
    let (hostport, path) = rest.split_once('/').map_or((rest, String::from("/")), |(h, p)| (h, format!("/{p}")));
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().map_err(|_| format!("bad port in '{url}'"))?),
        None => (hostport.to_string(), 80),
    };
    Ok((host, port, path))
}

/// POST a form to a URL, return the body of the answer.
pub fn http_post(url: &str, body: &str, timeout_ms: i64) -> Result<String, String> {
    let (host, port, path) = split_url(url)?;
    let ip = server_addr(&host)?;
    let fd = tcp_connect(ip, port, timeout_ms).map_err(|_| format!("cannot reach {host} ({}) on port {port}", ip_str(ip)))?;
    let req = format!(
        "POST {path} HTTP/1.0\r\nHost: {host}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    if !send_all(fd, req.as_bytes()) {
        socket::close(fd);
        return Err(format!("connection to {host} lost"));
    }
    let text = read_all(fd, timeout_ms);
    socket::close(fd);
    let text = text.ok_or_else(|| format!("{host} did not answer"))?;
    match text.split_once("\r\n\r\n") {
        Some((_, b)) => Ok(b.to_string()),
        None => Err(format!("{host} sent no HTTP answer")),
    }
}

/// Everything until the peer closes (or the timeout).
pub fn read_all(fd: i32, timeout_ms: i64) -> Option<String> {
    let start = Instant::now();
    let mut data = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        if start.elapsed().as_millis() as i64 > timeout_ms {
            break;
        }
        let n = socket::recv(fd, &mut buf, 0);
        if n == -2 {
            continue;
        }
        if n <= 0 {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    (!data.is_empty()).then(|| String::from_utf8_lossy(&data).to_string())
}

/// A line-oriented TCP client (EPP-lite): connect, then [`LineClient::call`] per command.
pub struct LineClient {
    fd: i32,
    pending: Vec<u8>,
}

impl LineClient {
    pub fn connect(ip: Ip, port: u16, timeout_ms: i64) -> Result<LineClient, String> {
        Ok(LineClient { fd: tcp_connect(ip, port, timeout_ms)?, pending: Vec::new() })
    }

    /// Send one line; read one reply line, or for a reply starting `OK` and ending without
    /// "." on a multi-line answer, up to the "." line.
    pub fn call(&mut self, line: &str, multi: bool) -> Result<String, String> {
        if !send_all(self.fd, format!("{line}\r\n").as_bytes()) {
            return Err("connection lost".into());
        }
        let first = self.read_line()?;
        if !multi || !first.starts_with("OK") {
            return Ok(first);
        }
        let mut out = first;
        loop {
            let l = self.read_line()?;
            out.push('\n');
            out.push_str(&l);
            if l == "." {
                return Ok(out);
            }
        }
    }

    /// One reply line (the greeting a server sends first, for example).
    pub fn read_line(&mut self) -> Result<String, String> {
        let start = Instant::now();
        let mut buf = [0u8; 1024];
        loop {
            if let Some(i) = self.pending.iter().position(|&b| b == b'\n') {
                let l: Vec<u8> = self.pending.drain(..=i).collect();
                return Ok(String::from_utf8_lossy(&l).trim_end().to_string());
            }
            if start.elapsed().as_millis() > 5000 {
                return Err("no answer".into());
            }
            let n = socket::recv(self.fd, &mut buf, 0);
            if n == -2 {
                continue;
            }
            if n <= 0 {
                return Err("connection closed".into());
            }
            self.pending.extend_from_slice(&buf[..n as usize]);
        }
    }
}

impl Drop for LineClient {
    fn drop(&mut self) {
        socket::close(self.fd);
    }
}

/// HTML-escape.
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
