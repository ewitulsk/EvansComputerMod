//! chatd: the Tech Village instant-messaging server.
//!
//! `chatd [PORT] [--room NAME]` listens on TCP (default 7777) and runs one chat room
//! (`ecm_chat::server::Room`): nicknames, broadcast with timestamps, join/leave notices,
//! history replay for new joiners. One `poll` loop serves every client; a client that
//! disconnects or fails a send is dropped without affecting the others.

use ecm_chat::proto::DEFAULT_PORT;
use ecm_chat::server::{Out, Room};
use ecm_host_abi::socket::{
    self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLERR, POLLHUP, POLLIN, SOCK_STREAM, SOL_SOCKET, SO_REUSEADDR,
};
use std::io::Write;

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn log(text: &str) {
    println!("[{}] {}", ecm_chat::proto::hms(now()), text);
    let _ = std::io::stdout().flush();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut port = DEFAULT_PORT;
    let mut room_name = String::from("Tech Village chat");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("Usage: chatd [PORT] [--room NAME]   (default port {})", DEFAULT_PORT);
                println!("  Start it in the background with 'chatd &' or from services.cfg.");
                return;
            }
            "--room" => {
                i += 1;
                if let Some(n) = args.get(i) {
                    room_name = n.clone();
                }
            }
            p => match p.parse() {
                Ok(n) => port = n,
                Err(_) => {
                    eprintln!("chatd: bad port '{}'", p);
                    std::process::exit(1);
                }
            },
        }
        i += 1;
    }

    let listener = socket::socket(AF_INET, SOCK_STREAM, 0);
    if listener < 0 {
        eprintln!("chatd: cannot create a socket");
        std::process::exit(1);
    }
    socket::setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &1i32.to_le_bytes());
    if socket::bind(listener, &SockAddrIn::any(port)) != 0 || socket::listen(listener, 8) != 0 {
        eprintln!("chatd: cannot listen on port {} (already in use?)", port);
        socket::close(listener);
        std::process::exit(1);
    }
    log(&format!("chatd listening on port {} (room \"{}\"). Ctrl+T to stop.", port, room_name));

    let mut room = Room::new(&room_name);
    let mut clients: Vec<i32> = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        let mut fds: Vec<PollFd> = Vec::with_capacity(clients.len() + 1);
        fds.push(PollFd { fd: listener, events: POLLIN, revents: 0 });
        for &c in &clients {
            fds.push(PollFd { fd: c, events: POLLIN, revents: 0 });
        }
        if socket::poll(&mut fds, 1000) < 0 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }
        let mut outs: Vec<Out> = Vec::new();
        let mut gone: Vec<i32> = Vec::new();
        if fds[0].revents & POLLIN != 0 {
            let mut peer = SockAddrIn::default();
            let c = socket::accept(listener, &mut peer);
            if c >= 0 {
                clients.push(c);
                outs.extend(room.connect(c as u32, &peer.to_string()));
            }
        }
        for f in &fds[1..] {
            if f.revents & (POLLIN | POLLERR | POLLHUP) == 0 {
                continue;
            }
            match socket::recv(f.fd, &mut buf, MSG_DONTWAIT) {
                n if n > 0 => outs.extend(room.receive(f.fd as u32, &buf[..n as usize], now())),
                -2 => {}
                _ => gone.push(f.fd),
            }
        }
        // Perform the actions; a failed send counts as a disconnect, whose notices are
        // performed in turn.
        loop {
            for fd in gone.drain(..) {
                if clients.contains(&fd) {
                    clients.retain(|&c| c != fd);
                    socket::close(fd);
                    outs.extend(room.disconnect(fd as u32, now()));
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
                    Out::Log(text) => log(&text),
                }
            }
        }
    }
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
