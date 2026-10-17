//! The real [`Tel`] and [`Files`] on a computer: telephonyd's control sockets on
//! loopback (reconnected when it restarts) and the WASI file system.

use std::time::Duration;

use ecm_cell_net::{AdminClient, Read};

use crate::{Event, Files, Tel};

/// telephonyd over its control sockets: one request connection per port and a
/// `watch` connection (on `watch_port`) for the events.
pub struct Net {
    clients: Vec<(u16, AdminClient)>,
    watch_port: u16,
    watch: Option<i32>,
    partial: Vec<u8>,
}

impl Net {
    /// `watch_port`: [`ecm_tel::ctl::PORT`] for Phone, [`ecm_tel::ctl::SMS_PORT`] for Messages.
    pub fn new(watch_port: u16) -> Net {
        Net { clients: Vec::new(), watch_port, watch: None, partial: Vec::new() }
    }

    fn open_watch(&mut self) {
        let Some(fd) = ecm_cell_net::tcp_connect(ecm_cell_net::LOOPBACK, self.watch_port) else { return };
        // Events wake a sleeping phone (an incoming text while Messages is open).
        ecm_host_abi::socket::set_wake(fd, true);
        if ecm_cell_net::send_all(fd, b"watch\n") {
            self.watch = Some(fd);
            self.partial.clear();
        } else {
            ecm_host_abi::socket::close(fd);
        }
    }
}

impl Tel for Net {
    fn call(&mut self, port: u16, req: &str) -> Result<Result<Vec<String>, String>, String> {
        for _ in 0..2 {
            if !self.clients.iter().any(|(p, _)| *p == port) {
                match AdminClient::connect(port) {
                    Some(c) => self.clients.push((port, c)),
                    None => return Err("telephonyd is not running".into()),
                }
            }
            let i = self.clients.iter().position(|(p, _)| *p == port).unwrap();
            match self.clients[i].1.call(req, Duration::from_secs(5)) {
                Ok(r) => return Ok(r),
                Err(_) => {
                    let (_, c) = self.clients.remove(i);
                    c.close();
                }
            }
        }
        Err("the connection to telephonyd failed".into())
    }

    fn events(&mut self) -> Vec<Event> {
        if self.watch.is_none() {
            self.open_watch();
        }
        let Some(fd) = self.watch else { return Vec::new() };
        let mut buf = [0u8; 4096];
        loop {
            match ecm_cell_net::read_now(fd, &mut buf) {
                Read::Data(d) => self.partial.extend_from_slice(&d),
                Read::Nothing => break,
                Read::Closed => {
                    ecm_host_abi::socket::close(fd);
                    self.watch = None;
                    break;
                }
            }
        }
        let mut out = Vec::new();
        while let Some(n) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=n).collect();
            let l = String::from_utf8_lossy(&line).trim().to_string();
            if let Some(e) = Event::parse(&l) {
                out.push(e);
            }
        }
        out
    }
}

/// The WASI file system.
pub struct Fs;

impl Files for Fs {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }
    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        if let Some((dir, _)) = path.rsplit_once('/') {
            if !dir.is_empty() {
                let _ = std::fs::create_dir_all(dir);
            }
        }
        std::fs::write(path, data).map_err(|e| e.to_string())
    }
    fn list(&mut self, dir: &str) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).map(|r| r.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        v.sort();
        v
    }
}
