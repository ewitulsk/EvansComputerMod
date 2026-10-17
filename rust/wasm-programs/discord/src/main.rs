//! `discord` — Discord (`dev.ecm.discord`), the GUI app (docs/DISCORD.md).
//!
//!   discord                      the Discord window (needs the GUI); installed as the
//!                                app dev.ecm.discord it is what the phone's Discord runs
//!   discord --source <dir>       write the app's package folder (app.toml, icon.png,
//!                                main.wasm = this program) for: pkg build <dir> <repo dir>
//!
//! It talks to every Discord backend (`discordd`) it finds on the network, plus the
//! one in `/etc/chat.conf` and the ones the player added.

use discord_app::app::{Audio, DiscordApp, Net};
use ecm_chat::client::{self, Problem};
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLERR, POLLHUP, POLLIN, SOCK_DGRAM, SOCK_STREAM};
use ecm_ui::conn::{default_connection, ConnError};
use std::collections::BTreeMap;
use std::time::Instant;

/// TCP to backends and UDP for voice, through the computer's network stack.
struct SocketNet {
    started: Instant,
    open: BTreeMap<i32, ()>,
    voice: i32,
    voice_to: Option<SockAddrIn>,
}

fn resolve(host: &str) -> Result<[u8; 4], String> {
    if let Some(ip) = client::parse_ipv4(host) {
        return Ok(ip);
    }
    let mut addr = SockAddrIn::default();
    if socket::getaddrinfo(host, &mut addr) != 0 {
        return Err(first_line(&client::hint(&Problem::Unresolved(host.into()))));
    }
    Ok(addr.ip())
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim_start_matches("chat: ").to_string()
}

impl Net for SocketNet {
    fn connect(&mut self, host: &str, port: u16) -> Result<i32, String> {
        let ip = resolve(host)?;
        let endpoint = format!("{}.{}.{}.{}:{}", ip[0], ip[1], ip[2], ip[3], port);
        let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
        if fd == -13 {
            return Err("Discord isn't allowed to use the network (Settings > Apps > Discord)".into());
        }
        if fd < 0 {
            return Err("No network: cannot open a connection".into());
        }
        let started = Instant::now();
        if socket::connect(fd, &SockAddrIn::from_ip_port(ip, port)) != 0 {
            socket::close(fd);
            let p = if started.elapsed().as_millis() < 8000 { Problem::Refused(endpoint) } else { Problem::Timeout(endpoint) };
            return Err(first_line(&client::hint(&p)).replace("chatd", "discordd"));
        }
        // A wake socket: a message for a suspended Discord wakes it at once to post its notification.
        socket::set_wake(fd, true);
        self.open.insert(fd, ());
        Ok(fd)
    }

    fn send(&mut self, fd: i32, line: &str) -> Result<(), String> {
        let data = format!("{}\n", line);
        let mut rest = data.as_bytes();
        while !rest.is_empty() {
            let n = socket::send(fd, rest, 0);
            if n <= 0 {
                return Err(String::new());
            }
            rest = &rest[n as usize..];
        }
        Ok(())
    }

    fn recv(&mut self, fd: i32) -> Result<Vec<u8>, String> {
        let mut fds = [PollFd { fd, events: POLLIN, revents: 0 }];
        socket::poll(&mut fds, 0);
        if fds[0].revents & (POLLIN | POLLERR | POLLHUP) == 0 {
            return Ok(Vec::new());
        }
        let mut buf = [0u8; 4096];
        match socket::recv(fd, &mut buf, MSG_DONTWAIT) {
            n if n > 0 => Ok(buf[..n as usize].to_vec()),
            -2 => Ok(Vec::new()),
            _ => Err(String::new()),
        }
    }

    fn close(&mut self, fd: i32) {
        if self.open.remove(&fd).is_some() {
            socket::shutdown(fd, socket::SHUT_WR);
            socket::close(fd);
        }
    }

    fn discover(&mut self) -> Vec<ecm_disco::Found> {
        ecm_disco::sys::search(ecm_disco::types::DISCORD, ecm_disco::CLIENT_WAIT_MS).unwrap_or_default()
    }

    fn default_nick(&mut self) -> String {
        for i in 0..ecm_host_abi::net_config::iface_count() {
            if let Some(info) = ecm_host_abi::net_config::iface_info(i) {
                if let Some(mac) = client::json_field(&info, "mac") {
                    if !mac.is_empty() && mac != "00:00:00:00:00:00" {
                        return client::nick_from_mac(mac);
                    }
                }
            }
        }
        "guest".into()
    }

    fn now_ms(&mut self) -> u64 {
        self.started.elapsed().as_millis() as u64 + 1
    }

    fn voice_open(&mut self, host: &str, port: u16) -> Result<(), String> {
        self.voice_close();
        let ip = resolve(host)?;
        let fd = socket::socket(AF_INET, SOCK_DGRAM, 0);
        if fd < 0 {
            return Err("no UDP socket for voice".into());
        }
        self.voice = fd;
        self.voice_to = Some(SockAddrIn::from_ip_port(ip, port));
        Ok(())
    }

    fn voice_send(&mut self, data: &[u8]) {
        if let Some(to) = &self.voice_to {
            socket::sendto(self.voice, data, 0, to);
        }
    }

    fn voice_recv(&mut self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        if self.voice < 0 {
            return out;
        }
        let mut buf = [0u8; 2048];
        for _ in 0..64 {
            let mut from = SockAddrIn::default();
            let n = socket::recvfrom(self.voice, &mut buf, MSG_DONTWAIT, &mut from);
            if n <= 0 {
                break;
            }
            out.push(buf[..n as usize].to_vec());
        }
        out
    }

    fn voice_close(&mut self) {
        if self.voice >= 0 {
            socket::close(self.voice);
        }
        self.voice = -1;
        self.voice_to = None;
    }
}

/// The computer's default microphone and speaker (Settings > Sound), at 16 kHz mono.
#[derive(Default)]
struct HostAudio {
    speaker: Option<ecm_audio::Device>,
    mic: Option<ecm_audio::Mic>,
    capturing: bool,
}

impl Audio for HostAudio {
    fn open(&mut self) -> Result<(), String> {
        let rate = ecm_discord::voice::RATE;
        let mut problems = Vec::new();
        match ecm_audio::Device::open(None) {
            Ok(mut d) => {
                let _ = d.set_format(rate, 16, 1);
                let _ = d.set_latency_ms(400);
                // A call is a voice stream: while someone talks, music on the same speaker is ducked.
                let _ = d.set_role(ecm_audio::Role::Voice);
                self.speaker = Some(d);
            }
            Err(e) => problems.push(format!("no speaker ({e})")),
        }
        match ecm_audio::Mic::open(None) {
            Ok(mut m) => {
                let _ = m.set_format(rate, 16, 1);
                self.mic = Some(m);
            }
            Err(e) => problems.push(format!("no microphone ({e}): listening only")),
        }
        self.capturing = false;
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems.join(", "))
        }
    }

    fn capture(&mut self) -> Vec<i16> {
        let Some(mic) = self.mic.as_mut() else { return Vec::new() };
        let frame = ecm_discord::voice::FRAME;
        if !self.capturing {
            // The stream's clock starts with its first read.
            self.capturing = true;
            return mic.read_samples(frame).unwrap_or_default();
        }
        let ms = mic.stat("buffered_ms").ok().flatten().unwrap_or(0).max(0) as usize;
        let n = ms * ecm_discord::voice::RATE as usize / 1000;
        if n == 0 {
            return Vec::new();
        }
        mic.read_samples(n.min(ecm_discord::voice::RATE as usize / 2)).unwrap_or_default()
    }

    fn play(&mut self, pcm: &[i16]) {
        if let Some(d) = self.speaker.as_mut() {
            let _ = d.write_samples(pcm);
        }
    }

    fn queued_ms(&mut self) -> i64 {
        self.speaker.as_mut().and_then(|d| d.stat("buffered_ms").ok().flatten()).unwrap_or(0)
    }

    fn headphones(&mut self) -> bool {
        self.speaker.as_mut().and_then(|d| d.stat("headphones").ok().flatten()).unwrap_or(0) == 1
    }

    fn close(&mut self) {
        if let Some(d) = self.speaker.as_mut() {
            let _ = d.flush_queue();
        }
        self.speaker = None;
        self.mic = None;
        self.capturing = false;
    }
}

/// This program's own module, to put into a package.
fn own_wasm() -> Option<Vec<u8>> {
    for p in ["server-bin/discord.wasm", "/server-bin/discord.wasm", "bin/discord.wasm"] {
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
            println!("usage: discord                  the Discord window (needs the GUI)");
            println!("       discord --source <dir>   write Discord's package folder, then: pkg build <dir> <repo dir>");
            return;
        }
        Some("--source") => {
            let Some(dir) = args.get(1) else {
                println!("usage: discord --source <dir>");
                std::process::exit(2);
            };
            let Some(program) = own_wasm() else {
                println!("discord: can't read server-bin/discord.wasm to package it");
                std::process::exit(1);
            };
            match discord_app::source::write(dir, &program) {
                Ok(lines) => {
                    for l in &lines {
                        println!("  {l}");
                    }
                    println!("discord: wrote Discord ({}) 1.0.0 to {dir}; pack it with: pkg build {dir} <repo dir>", discord_app::APP_ID);
                }
                Err(e) => {
                    println!("discord: {e}");
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
            println!("discord: Discord needs the GUI (start it with `gui`); in the console use chat");
            std::process::exit(1);
        }
        Err(e) => {
            println!("discord: {e}");
            std::process::exit(1);
        }
    };
    // /etc/chat.conf names a Tech Village's backend (every app may read it: public system
    // configuration, docs/gui/APPS.md "Files").
    let chat_conf = ["/etc/chat.conf", "etc/chat.conf"].iter().find_map(|p| std::fs::read_to_string(p).ok());
    let data = std::env::var("ECM_APP_DATA").ok().filter(|d| !d.is_empty());
    let net = SocketNet { started: Instant::now(), open: BTreeMap::new(), voice: -1, voice_to: None };
    let mut app = match DiscordApp::new(conn, net, HostAudio::default(), data, chat_conf.as_deref()) {
        Ok(a) => a,
        Err(e) => {
            println!("discord: {e}");
            std::process::exit(1);
        }
    };
    println!("discord: open");
    app.run();
}
