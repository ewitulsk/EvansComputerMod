//! `browser` — the Browser (`dev.ecm.browser`, docs/gui/BROWSER.md), a system
//! app on every computer: the Terminal desktop and the ePhone.
//!
//!   browser [address]     open the window (needs the GUI) at an address:
//!                         http://100.65.0.10/, /home/site/index.html, ...
//!
//! Pages come over the computer's network (TCP sockets, HTTP/1.0) and from
//! its disk (`file://`); the app needs the `network` and `files:documents`
//! permissions for them.

use browser_app::app::BrowserApp;
use ecm_host_abi::socket::{self, SockAddrIn, AF_INET, SOCK_STREAM, SOL_SOCKET, SO_RCVTIMEO};
use ecm_ui::conn::{default_connection, ConnError};
use ecm_web::fetch::{FetchError, Transport};
use std::time::{Duration, Instant};

/// A response that hasn't finished after this long is shown as it is.
const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// The computer's sockets and disk.
struct HostTransport;

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let parts: Vec<u8> = s.split('.').map(|p| p.parse::<u8>()).collect::<Result<_, _>>().ok()?;
    (parts.len() == 4).then(|| [parts[0], parts[1], parts[2], parts[3]])
}

impl Transport for HostTransport {
    fn exchange(&mut self, host: &str, port: u16, request: &[u8]) -> Result<Vec<u8>, FetchError> {
        let ip = match parse_ipv4(host) {
            Some(ip) => ip,
            None => {
                let mut addr = SockAddrIn::default();
                if socket::getaddrinfo(host, &mut addr) != 0 {
                    return Err(FetchError::Resolve(host.into()));
                }
                addr.ip()
            }
        };
        let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
        if fd == -13 {
            return Err(FetchError::NotAllowed);
        }
        if fd < 0 {
            return Err(FetchError::NoNetwork);
        }
        let endpoint = format!("{}.{}.{}.{}:{port}", ip[0], ip[1], ip[2], ip[3]);
        if socket::connect(fd, &SockAddrIn::from_ip_port(ip, port)) != 0 {
            socket::close(fd);
            return Err(FetchError::Connect(endpoint));
        }
        let mut rest = request;
        while !rest.is_empty() {
            let n = socket::send(fd, &rest[..rest.len().min(1400)], 0);
            if n <= 0 {
                socket::close(fd);
                return Err(FetchError::Io(format!("the connection to {endpoint} broke while sending")));
            }
            rest = &rest[n as usize..];
        }
        socket::setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &1000i32.to_le_bytes());
        let started = Instant::now();
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = socket::recv(fd, &mut buf, 0);
            if n > 0 {
                out.extend_from_slice(&buf[..n as usize]);
                if out.len() >= ecm_web::fetch::MAX_BODY {
                    break;
                }
                continue;
            }
            // -2: no data yet (the receive timeout); 0 / -1: closed.
            if n == -2 && started.elapsed() < READ_TIMEOUT {
                continue;
            }
            break;
        }
        socket::shutdown(fd, socket::SHUT_RDWR);
        socket::close(fd);
        if out.is_empty() && started.elapsed() >= READ_TIMEOUT {
            return Err(FetchError::Connect(endpoint));
        }
        Ok(out)
    }

    fn read_file(&mut self, path: &str) -> Result<Vec<u8>, FetchError> {
        let p = std::path::Path::new(path);
        let p = if p.is_dir() { p.join("index.html") } else { p.to_path_buf() };
        std::fs::read(&p).map_err(|e| match e.kind() {
            std::io::ErrorKind::PermissionDenied => FetchError::NotAllowed,
            std::io::ErrorKind::NotFound => FetchError::NoFile(path.into()),
            _ => FetchError::Io(format!("{path}: {e}")),
        })
    }

    fn list_dir(&mut self, path: &str) -> Option<Vec<(String, bool)>> {
        let mut out: Vec<(String, bool)> = std::fs::read_dir(path)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.file_type().is_ok_and(|t| t.is_dir())))
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Some(out)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("--help") | Some("-h")) {
        println!("usage: browser [address]    the Browser window (needs the GUI)");
        println!("       address: http://100.65.0.10/, 100.65.0.10:8080/page.html, /home/site/index.html, file:///home/");
        return;
    }
    let conn = match default_connection(0) {
        Ok(c) => c,
        Err(ConnError::Console) => {
            println!("browser: the Browser needs the GUI (start it with `gui`); in the console use curl");
            std::process::exit(1);
        }
        Err(e) => {
            println!("browser: {e}");
            std::process::exit(1);
        }
    };
    let mut app = match BrowserApp::new(conn, HostTransport, args.first().map(String::as_str)) {
        Ok(a) => a,
        Err(e) => {
            println!("browser: {e}");
            std::process::exit(1);
        }
    };
    println!("browser: open");
    app.run();
}
