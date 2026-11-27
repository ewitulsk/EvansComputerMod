//! The host's own network: for tests and for tools that run off a computer.

use crate::{Connector, HttpError, Stream};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Default)]
pub struct Native {
    /// Say the connection went to this address instead of the real one
    /// (tests use it to stand a loopback server in for a public one).
    pub pretend_peer: Option<[u8; 4]>,
}

impl Connector for Native {
    fn connect(&mut self, host: &str, port: u16) -> Result<(Box<dyn Stream>, [u8; 4]), HttpError> {
        let addr = (host, port)
            .to_socket_addrs()
            .map_err(|e| HttpError::Resolve(host.into(), e.to_string()))?
            .find(SocketAddr::is_ipv4)
            .ok_or_else(|| HttpError::Resolve(host.into(), "no IPv4 address".into()))?;
        let s = TcpStream::connect_timeout(&addr, Duration::from_secs(10)).map_err(|_| HttpError::Connect(addr.to_string()))?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(30)));
        let ip = match addr {
            SocketAddr::V4(a) => a.ip().octets(),
            SocketAddr::V6(_) => [0; 4],
        };
        Ok((Box::new(s), self.pretend_peer.unwrap_or(ip)))
    }
}
