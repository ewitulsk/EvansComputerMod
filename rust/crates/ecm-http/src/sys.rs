//! The computer's network: the kernel's sockets and resolver.

use crate::probe::NetState;
use crate::{Connector, HttpError, Stream};
use ecm_host_abi::net_config;
use ecm_host_abi::socket::{self, SockAddrIn, AF_INET, SOCK_STREAM, SOL_SOCKET, SO_RCVTIMEO};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

/// A connected TCP socket. A read waits up to `idle` for more bytes.
pub struct Socket {
    fd: i32,
    idle: Duration,
}

impl Socket {
    /// Wrap a connected socket (a server's accepted connection, for example).
    pub fn from_fd(fd: i32, idle: Duration) -> Socket {
        socket::setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &1000i32.to_le_bytes());
        Socket { fd, idle }
    }

    pub fn fd(&self) -> i32 {
        self.fd
    }
}

impl Read for Socket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let started = Instant::now();
        loop {
            let n = socket::recv(self.fd, buf, 0);
            if n > 0 {
                return Ok(n as usize);
            }
            // -2: nothing yet (the one-second receive timeout); 0 or -1: closed.
            if n == -2 {
                if started.elapsed() < self.idle {
                    continue;
                }
                return Err(io::Error::new(io::ErrorKind::TimedOut, "the server stopped answering"));
            }
            return Ok(0);
        }
    }
}

impl Write for Socket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // One segment at a time keeps the kernel's send buffer moving.
        let n = socket::send(self.fd, &buf[..buf.len().min(1400)], 0);
        if n <= 0 {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "the connection broke"));
        }
        Ok(n as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        socket::close(self.fd);
    }
}

/// Connects through the computer's own network stack.
pub struct Sockets {
    /// How long a read waits for more bytes before giving up.
    pub idle: Duration,
}

impl Default for Sockets {
    fn default() -> Self {
        Sockets { idle: Duration::from_secs(30) }
    }
}

/// `a.b.c.d` as an address.
pub fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    ecm_tls::x509::parse_ipv4(s)
}

/// An address for `host`: a dotted address as is, a name through the kernel's resolver.
pub fn resolve(host: &str) -> Result<[u8; 4], HttpError> {
    match parse_ipv4(host) {
        Some(ip) => Ok(ip),
        None => socket::resolve(host).map_err(|e| HttpError::Resolve(host.into(), e.describe())),
    }
}

impl Connector for Sockets {
    fn connect(&mut self, host: &str, port: u16) -> Result<(Box<dyn Stream>, [u8; 4]), HttpError> {
        let ip = resolve(host)?;
        let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
        if fd == -13 {
            return Err(HttpError::NotAllowed);
        }
        if fd < 0 {
            return Err(HttpError::NoNetwork);
        }
        if socket::connect(fd, &SockAddrIn::from_ip_port(ip, port)) != 0 {
            socket::close(fd);
            return Err(HttpError::Connect(format!("{}.{}.{}.{}:{port}", ip[0], ip[1], ip[2], ip[3])));
        }
        Ok((Box::new(Socket::from_fd(fd, self.idle)), ip))
    }
}

/// A client on the computer's network.
pub fn client(user_agent: &str) -> crate::Client<Sockets> {
    crate::Client::new(Sockets::default()).agent(user_agent)
}

fn json_str<'a>(obj: &'a str, key: &str) -> Option<&'a str> {
    let i = obj.find(&format!("\"{key}\":\""))? + key.len() + 4;
    obj[i..].split('"').next()
}

/// The interfaces and default route as the kernel reports them.
pub fn net_state() -> NetState {
    let mut s = NetState::default();
    for i in 0..net_config::iface_count() {
        if let Some(info) = net_config::iface_info(i) {
            let name = json_str(&info, "name").unwrap_or("").to_string();
            let ip = json_str(&info, "ip").unwrap_or("").to_string();
            s.interfaces.push((name, info.contains("\"link_up\":true"), ip));
        }
    }
    if let Some(routes) = net_config::route_list() {
        s.default_route = routes.contains("\"dest\":\"0.0.0.0\",\"prefix_len\":0");
    }
    s
}

/// Is `url` reachable from this computer, and if not, which step fails?
pub fn online(url: &str) -> crate::probe::Online {
    let mut c = crate::Client::new(Sockets { idle: Duration::from_secs(8) }).agent("ecm-online/1");
    crate::probe::check(&mut c, &net_state(), url)
}
