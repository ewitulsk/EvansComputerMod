//! `music` — Music (`dev.ecm.music`), the GUI app (docs/MUSIC.md).
//!
//!   music                    the Music window (needs the GUI); installed as the app
//!                            dev.ecm.music it is what the phone's Music runs
//!   music --source <dir>     write the app's package folder (app.toml, icon.png,
//!                            main.wasm = this program) for: pkg build <dir> <repo dir>

use ecm_auth::{AuthError, Stream};
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLERR, POLLHUP, POLLIN, SOCK_STREAM};
use ecm_ui::conn::{default_connection, ConnError};
use music_app::app::MusicApp;
use music_app::net::{Net, Speaker};
use std::collections::BTreeSet;
use std::time::Instant;

/// TCP through the computer's network stack.
struct SocketNet {
    started: Instant,
    open: BTreeSet<i32>,
}

fn resolve(host: &str) -> Result<[u8; 4], String> {
    if let Some(ip) = ecm_disco::parse_ip(host) {
        return Ok(ip);
    }
    let mut addr = SockAddrIn::default();
    if socket::getaddrinfo(host, &mut addr) != 0 {
        return Err(format!("can't find {host}"));
    }
    Ok(addr.ip())
}

fn json_field<'a>(j: &'a str, k: &str) -> Option<&'a str> {
    ecm_disco::json_field(j, k)
}

impl Net for SocketNet {
    fn connect(&mut self, host: &str, port: u16) -> Result<i32, String> {
        let ip = resolve(host)?;
        let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
        if fd == -13 {
            return Err("Music isn't allowed to use the network (Settings > Apps > Music)".into());
        }
        if fd < 0 {
            return Err("No network: cannot open a connection".into());
        }
        if socket::connect(fd, &SockAddrIn::from_ip_port(ip, port)) != 0 {
            socket::close(fd);
            return Err(format!("{host}:{port} doesn't answer (is musicd running there?)"));
        }
        self.open.insert(fd);
        Ok(fd)
    }

    fn send(&mut self, fd: i32, mut data: &[u8]) -> Result<(), String> {
        while !data.is_empty() {
            let n = socket::send(fd, &data[..data.len().min(8192)], 0);
            if n <= 0 {
                return Err(String::new());
            }
            data = &data[n as usize..];
        }
        Ok(())
    }

    fn recv(&mut self, fd: i32) -> Result<Vec<u8>, String> {
        let mut fds = [PollFd { fd, events: POLLIN, revents: 0 }];
        socket::poll(&mut fds, 0);
        if fds[0].revents & (POLLIN | POLLERR | POLLHUP) == 0 {
            return Ok(Vec::new());
        }
        let mut buf = vec![0u8; 16 * 1024];
        match socket::recv(fd, &mut buf, MSG_DONTWAIT) {
            n if n > 0 => Ok(buf[..n as usize].to_vec()),
            -2 => Ok(Vec::new()),
            _ => Err(String::new()),
        }
    }

    fn close(&mut self, fd: i32) {
        if self.open.remove(&fd) {
            socket::shutdown(fd, socket::SHUT_WR);
            socket::close(fd);
        }
    }

    fn wait(&mut self, fd: i32, ms: u64) {
        let mut fds = [PollFd { fd, events: POLLIN, revents: 0 }];
        socket::poll(&mut fds, ms as i32);
    }

    fn now_ms(&mut self) -> u64 {
        self.started.elapsed().as_millis() as u64 + 1
    }

    fn discover(&mut self) -> Vec<ecm_disco::Found> {
        ecm_disco::sys::search(ecm_disco::types::MUSIC, ecm_disco::CLIENT_WAIT_MS).unwrap_or_default()
    }

    fn realm(&mut self, host: &str) -> Result<Box<dyn Stream>, AuthError> {
        ecm_auth::host::connect(host)
    }

    fn cellular(&mut self) -> bool {
        let Some(routes) = ecm_host_abi::net_config::route_list() else { return false };
        let ifaces: Vec<(String, [u8; 4], u8)> = (0..ecm_host_abi::net_config::iface_count())
            .filter_map(|i| {
                let info = ecm_host_abi::net_config::iface_info(i)?;
                let name = json_field(&info, "name")?.to_string();
                let ip = ecm_disco::parse_ip(json_field(&info, "ip")?)?;
                let prefix = json_field(&info, "prefix_len")?.parse().ok()?;
                Some((name, ip, prefix))
            })
            .collect();
        music_app::net::default_route_is_cellular(&routes, &ifaces)
    }
}

/// The default speaker (Settings > Sound), mono, 400 ms latency.
#[derive(Default)]
struct HostSpeaker {
    dev: Option<ecm_audio::Device>,
}

impl Speaker for HostSpeaker {
    fn open(&mut self, rate: u32) -> Result<(), String> {
        let mut d = ecm_audio::Device::open(None).map_err(|e| format!("no speaker ({e})"))?;
        d.set_format(rate, 16, 1).map_err(|e| e.to_string())?;
        let _ = d.set_latency_ms(400);
        let _ = d.set_role(ecm_audio::Role::Media);
        self.dev = Some(d);
        Ok(())
    }
    fn write(&mut self, pcm: &[i16]) {
        if let Some(d) = self.dev.as_mut() {
            let _ = d.write_samples(pcm);
        }
    }
    fn buffered_ms(&mut self) -> i64 {
        self.dev.as_mut().and_then(|d| d.stat("buffered_ms").ok().flatten()).unwrap_or(0)
    }
    fn next_pts_us(&mut self) -> Option<u64> {
        self.dev.as_mut().and_then(|d| d.next_pts_us()).filter(|p| *p > 0).map(|p| p as u64)
    }
    fn flush(&mut self) {
        if let Some(d) = self.dev.as_mut() {
            let _ = d.flush_queue();
        }
    }
    fn close(&mut self) {
        self.flush();
        self.dev = None;
    }
}

/// This program's own module, to put into a package.
fn own_wasm() -> Option<Vec<u8>> {
    for p in ["server-bin/music.wasm", "/server-bin/music.wasm", "bin/music.wasm"] {
        if let Ok(b) = std::fs::read(p) {
            return Some(b);
        }
        if let Some(b) = ecm_host_abi::fs::read_file_bytes(p.trim_start_matches('/')) {
            return Some(b);
        }
    }
    None
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--help") | Some("-h") => {
            println!("usage: music                  the Music window (needs the GUI)");
            println!("       music --source <dir>   write Music's package folder, then: pkg build <dir> <repo dir>");
            return;
        }
        Some("--source") => {
            let Some(dir) = args.get(1) else {
                println!("usage: music --source <dir>");
                std::process::exit(2);
            };
            let Some(program) = own_wasm() else {
                println!("music: can't read server-bin/music.wasm to package it");
                std::process::exit(1);
            };
            match music_app::source::write(dir, &program) {
                Ok(lines) => {
                    for l in &lines {
                        println!("  {l}");
                    }
                    println!("music: wrote Music ({}) 1.0.0 to {dir}; pack it with: pkg build {dir} <repo dir>", music_app::APP_ID);
                }
                Err(e) => {
                    println!("music: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        _ => {}
    }
    let conn = match default_connection(0) {
        Ok(c) => c,
        Err(ConnError::Console) => {
            println!("music: Music needs the GUI (start it with `gui`)");
            std::process::exit(1);
        }
        Err(e) => {
            println!("music: {e}");
            std::process::exit(1);
        }
    };
    let data = std::env::var("ECM_APP_DATA").ok().filter(|d| !d.is_empty());
    let accounts = format!("{}/accounts", data.clone().unwrap_or_else(|| "/var/apps/dev.ecm.music".into()));
    let files = Box::new(ecm_auth::host::DirFiles::new(&accounts));
    let net = SocketNet { started: Instant::now(), open: BTreeSet::new() };
    let mut app = match MusicApp::new(conn, net, HostSpeaker::default(), data, files) {
        Ok(a) => a,
        Err(e) => {
            println!("music: {e}");
            std::process::exit(1);
        }
    };
    println!("music: open");
    app.run();
}
