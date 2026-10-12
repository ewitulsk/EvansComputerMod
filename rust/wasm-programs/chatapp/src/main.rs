//! `chatapp` — Chat (`dev.ecm.chat`), the GUI messaging app (docs/gui/APPS.md "Chat").
//!
//!   chatapp                       the Chat window (needs the GUI); installed as the
//!                                 app dev.ecm.chat it is what the phone's Chat runs
//!   chatapp --source <dir>        write the app's package folder (app.toml, icon.png,
//!                                 main.wasm = this program) for: pkg build <dir> <repo>
//!
//! It talks to `chatd` with the same protocol as the `chat` command.

use chat_app::app::{ChatApp, Net};
use chat_app::model::Model;
use ecm_chat::client::{self, Problem};
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLERR, POLLHUP, POLLIN, SOCK_STREAM};
use ecm_ui::conn::{default_connection, ConnError};
use std::time::Instant;

/// TCP to the chat server through the computer's network stack.
struct SocketNet {
    fd: i32,
}

impl Net for SocketNet {
    fn connect(&mut self, host: &str, port: u16) -> Result<(), String> {
        self.close();
        let ip = match client::parse_ipv4(host) {
            Some(ip) => ip,
            None => {
                let mut addr = SockAddrIn::default();
                if socket::getaddrinfo(host, &mut addr) != 0 {
                    return Err(first_line(&client::hint(&Problem::Unresolved(host.into()))));
                }
                addr.ip()
            }
        };
        let endpoint = format!("{}.{}.{}.{}:{}", ip[0], ip[1], ip[2], ip[3], port);
        let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
        if fd == -13 {
            return Err("Chat isn't allowed to use the network (Settings > Apps > Chat)".into());
        }
        if fd < 0 {
            return Err("No network: cannot open a connection".into());
        }
        let started = Instant::now();
        if socket::connect(fd, &SockAddrIn::from_ip_port(ip, port)) != 0 {
            socket::close(fd);
            let p = if started.elapsed().as_millis() < 8000 { Problem::Refused(endpoint) } else { Problem::Timeout(endpoint) };
            return Err(first_line(&client::hint(&p)));
        }
        self.fd = fd;
        Ok(())
    }

    fn send(&mut self, line: &str) -> Result<(), String> {
        let data = format!("{}\n", line);
        let mut rest = data.as_bytes();
        while !rest.is_empty() {
            let n = socket::send(self.fd, rest, 0);
            if n <= 0 {
                return Err(String::new());
            }
            rest = &rest[n as usize..];
        }
        Ok(())
    }

    fn recv(&mut self) -> Result<Vec<u8>, String> {
        if self.fd < 0 {
            return Err(String::new());
        }
        let mut fds = [PollFd { fd: self.fd, events: POLLIN, revents: 0 }];
        socket::poll(&mut fds, 0);
        if fds[0].revents & (POLLIN | POLLERR | POLLHUP) == 0 {
            return Ok(Vec::new());
        }
        let mut buf = [0u8; 4096];
        match socket::recv(self.fd, &mut buf, MSG_DONTWAIT) {
            n if n > 0 => Ok(buf[..n as usize].to_vec()),
            -2 => Ok(Vec::new()),
            _ => Err(String::new()),
        }
    }

    fn close(&mut self) {
        if self.fd >= 0 {
            socket::shutdown(self.fd, socket::SHUT_WR);
            socket::close(self.fd);
            self.fd = -1;
        }
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
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim_start_matches("chat: ").to_string()
}

/// This program's own module, to put into a package.
fn own_wasm() -> Option<Vec<u8>> {
    for p in ["server-bin/chatapp.wasm", "/server-bin/chatapp.wasm", "bin/chatapp.wasm"] {
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
            println!("usage: chatapp                  the Chat window (needs the GUI)");
            println!("       chatapp --source <dir>   write Chat's package folder, then: pkg build <dir> <repo dir>");
            return;
        }
        Some("--source") => {
            let Some(dir) = args.get(1) else {
                println!("usage: chatapp --source <dir>");
                std::process::exit(2);
            };
            let Some(program) = own_wasm() else {
                println!("chatapp: can't read server-bin/chatapp.wasm to package it");
                std::process::exit(1);
            };
            match chat_app::source::write(dir, &program) {
                Ok(lines) => {
                    for l in &lines {
                        println!("  {l}");
                    }
                    println!("chatapp: wrote Chat ({}) 1.0.0 to {dir}; pack it with: pkg build {dir} <repo dir>", chat_app::APP_ID);
                }
                Err(e) => {
                    println!("chatapp: {e}");
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
            println!("chatapp: Chat needs the GUI (start it with `gui`); in the console use chat");
            std::process::exit(1);
        }
        Err(e) => {
            println!("chatapp: {e}");
            std::process::exit(1);
        }
    };
    // Defaults: /etc/chat.conf when this program may read it (typed in a terminal, not as an app).
    let mut defaults = Model::default();
    for path in ["etc/chat.conf", "/etc/chat.conf"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            defaults.load_settings(&text);
            break;
        }
    }
    let data = std::env::var("ECM_APP_DATA").ok().filter(|d| !d.is_empty());
    let mut app = match ChatApp::new(conn, SocketNet { fd: -1 }, data, defaults) {
        Ok(a) => a,
        Err(e) => {
            println!("chatapp: {e}");
            std::process::exit(1);
        }
    };
    println!("chatapp: open");
    app.run();
}
