//! discordd: a Discord backend.
//!
//! `discordd [PORT] [--name NAME] [--data DIR]` listens on TCP (default 7777) for the
//! Discord app (and the `chat` command, which lands in the home server's `#general`)
//! and on UDP PORT+1 for voice. It hosts servers with text and voice channels
//! (`ecm_discord::server::Backend`), keeps them in `DIR/state` (default
//! `/var/discord`), and announces itself to `disco serve` so Discord apps on the
//! network find it by themselves.

use ecm_discord::proto::DEFAULT_PORT;
use ecm_discord::server::{Backend, Out};
use ecm_host_abi::socket::{
    self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLERR, POLLHUP, POLLIN, SOCK_DGRAM, SOCK_STREAM, SOL_SOCKET, SO_REUSEADDR,
};
use std::io::Write;

const USAGE: &str = "Usage: discordd [PORT] [--name NAME] [--data DIR]\n\
  A Discord backend: servers with text and voice channels for the Discord app.\n\
  TCP PORT (default 7777) for clients, UDP PORT+1 for voice. NAME names its home\n\
  server (default \"Discord\"); DIR keeps its servers (default /var/discord).\n\
  Start it in the background with 'discordd &' or from services.cfg; run\n\
  'disco serve &' too so Discord apps on the network find it.";

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn log(text: &str) {
    println!("[{}] {}", ecm_chat_time(now_ms()), text);
    let _ = std::io::stdout().flush();
}

fn ecm_chat_time(ms: u64) -> String {
    let s = (ms / 1000) % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

fn seed() -> u64 {
    let mut b = [0u8; 8];
    ecm_host_abi::random::fill(&mut b);
    u64::from_le_bytes(b) ^ now_ms()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut port = DEFAULT_PORT;
    let mut name = String::from("Discord");
    let mut data = String::from("/var/discord");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            "--name" => {
                i += 1;
                match args.get(i).map(|n| ecm_discord::proto::guild_name(n)) {
                    Some(Ok(n)) => name = n,
                    Some(Err(e)) => {
                        eprintln!("discordd: --name: {e}");
                        std::process::exit(1);
                    }
                    None => {
                        eprintln!("discordd: --name needs a name");
                        std::process::exit(1);
                    }
                }
            }
            "--data" => {
                i += 1;
                if let Some(d) = args.get(i) {
                    data = d.trim_end_matches('/').to_string();
                }
            }
            p => match p.parse::<u16>() {
                Ok(n) if n > 0 && n < u16::MAX => port = n,
                _ => {
                    eprintln!("discordd: bad port '{}'\n{USAGE}", p);
                    std::process::exit(1);
                }
            },
        }
        i += 1;
    }
    let voice_port = port + 1;
    let state_file = format!("{data}/state");

    let listener = socket::socket(AF_INET, SOCK_STREAM, 0);
    if listener < 0 {
        eprintln!("discordd: cannot create a socket");
        std::process::exit(1);
    }
    socket::setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &1i32.to_le_bytes());
    if socket::bind(listener, &SockAddrIn::any(port)) != 0 || socket::listen(listener, 8) != 0 {
        eprintln!("discordd: cannot listen on port {} (already in use?)", port);
        socket::close(listener);
        std::process::exit(1);
    }
    let udp = socket::socket(AF_INET, SOCK_DGRAM, 0);
    if udp < 0 || socket::bind(udp, &SockAddrIn::any(voice_port)) != 0 {
        eprintln!("discordd: cannot use UDP port {} for voice (already in use?)", voice_port);
        socket::close(listener);
        std::process::exit(1);
    }

    let mut backend = match load(&state_file) {
        Some(text) => Backend::load(&name, voice_port, seed(), &text),
        None => Backend::new(&name, voice_port, seed()),
    };
    save(&state_file, &backend);
    let servers = backend.guild_names().len();
    log(&format!(
        "discordd listening on port {} (voice on UDP {}), \"{}\": {} server{}. Ctrl+T to stop.",
        port,
        voice_port,
        name,
        servers,
        if servers == 1 { "" } else { "s" }
    ));
    // Found by Discord apps on the network (and the chat command) when this computer runs `disco serve`.
    #[cfg(target_arch = "wasm32")]
    {
        ecm_disco::sys::announce(ecm_disco::types::DISCORD, port, &name);
        ecm_disco::sys::announce(ecm_disco::types::CHAT, port, &name);
    }

    let mut clients: Vec<i32> = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let mut fds: Vec<PollFd> = Vec::with_capacity(clients.len() + 2);
        fds.push(PollFd { fd: listener, events: POLLIN, revents: 0 });
        fds.push(PollFd { fd: udp, events: POLLIN, revents: 0 });
        for &c in &clients {
            fds.push(PollFd { fd: c, events: POLLIN, revents: 0 });
        }
        let wait = if backend.anyone_speaking() { 100 } else { 1000 };
        if socket::poll(&mut fds, wait) < 0 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }
        let now = now_ms();
        let mut outs: Vec<Out> = backend.tick(now);
        let mut gone: Vec<i32> = Vec::new();
        if fds[0].revents & POLLIN != 0 {
            let mut peer = SockAddrIn::default();
            let c = socket::accept(listener, &mut peer);
            if c >= 0 {
                clients.push(c);
                outs.extend(backend.connect(c as u32, &peer.to_string()));
            }
        }
        if fds[1].revents & POLLIN != 0 {
            // Everything waiting: voice comes in bursts.
            for _ in 0..64 {
                let mut from = SockAddrIn::default();
                let n = socket::recvfrom(udp, &mut buf, MSG_DONTWAIT, &mut from);
                if n <= 0 {
                    break;
                }
                outs.extend(backend.udp(from.ip(), from.port(), &buf[..n as usize], now));
            }
        }
        for f in &fds[2..] {
            if f.revents & (POLLIN | POLLERR | POLLHUP) == 0 {
                continue;
            }
            match socket::recv(f.fd, &mut buf, MSG_DONTWAIT) {
                n if n > 0 => outs.extend(backend.receive(f.fd as u32, &buf[..n as usize], now)),
                -2 => {}
                _ => gone.push(f.fd),
            }
        }
        // Perform the actions; a failed send counts as a disconnect, whose notices are
        // performed in turn.
        let mut dirty = false;
        loop {
            for fd in gone.drain(..) {
                if clients.contains(&fd) {
                    clients.retain(|&c| c != fd);
                    socket::close(fd);
                    outs.extend(backend.disconnect(fd as u32, now));
                }
            }
            if outs.is_empty() {
                break;
            }
            for out in std::mem::take(&mut outs) {
                match out {
                    Out::Send(id, line) => {
                        let fd = id as i32;
                        if clients.contains(&fd) && !send_all(fd, format!("{}\n", line).as_bytes()) {
                            gone.push(fd);
                        }
                    }
                    Out::Close(id) => {
                        let fd = id as i32;
                        if clients.contains(&fd) {
                            clients.retain(|&c| c != fd);
                            socket::shutdown(fd, socket::SHUT_WR);
                            socket::close(fd);
                        }
                    }
                    Out::Udp(ip, port, data) => {
                        socket::sendto(udp, &data, 0, &SockAddrIn::from_ip_port(ip, port));
                    }
                    Out::Log(text) => log(&text),
                    Out::Save => dirty = true,
                }
            }
        }
        if dirty {
            save(&state_file, &backend);
        }
    }
}

/// Write the state (best effort: a full disk only costs persistence).
fn save(file: &str, backend: &Backend) {
    let text = backend.save_text();
    if std::fs::write(file, &text).is_ok() {
        return;
    }
    // The folder doesn't exist yet: make every level of it (the computer's own calls), then write.
    let rel = file.trim_start_matches('/');
    let mut dir = String::new();
    if let Some((parent, _)) = rel.rsplit_once('/') {
        for part in parent.split('/') {
            if !dir.is_empty() {
                dir.push('/');
            }
            dir.push_str(part);
            ecm_host_abi::fs::mkdir(&dir);
        }
    }
    if std::fs::write(file, &text).is_err() {
        ecm_host_abi::fs::write_file(rel, &text);
    }
}

/// The saved state, if any.
fn load(file: &str) -> Option<String> {
    std::fs::read_to_string(file)
        .ok()
        .or_else(|| ecm_host_abi::fs::read_file_bytes(file.trim_start_matches('/')).map(|b| String::from_utf8_lossy(&b).into_owned()))
}

fn send_all(fd: i32, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        let n = socket::send(fd, &data[..data.len().min(1400)], 0);
        if n <= 0 {
            return false;
        }
        data = &data[n as usize..];
    }
    true
}
