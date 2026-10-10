//! chat: the Tech Village instant-messaging client.
//!
//! `chat [server[:port]] [--nick NAME]`. With no server it reads `/etc/chat.conf`
//! (provisioned on Tech Village computers). Incoming messages print above the line you
//! are typing; `/nick`, `/who`, `/help`, `/quit`. When the server cannot be reached it
//! says why and how to fix it (no address, no route, refused, no answer).

use ecm_chat::client::{self, Editor, Key, Problem, Typed, PROMPT};
use ecm_chat::proto::{take_lines, Event};
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLERR, POLLHUP, POLLIN, SOCK_STREAM};
use std::io::{Read, Write};
use std::time::Instant;

fn out(s: &str) {
    let mut o = std::io::stdout();
    let _ = o.write_all(s.as_bytes());
    let _ = o.flush();
}

fn fail(p: Problem) -> ! {
    out(&format!("{}\n", client::hint(&p)));
    std::process::exit(1);
}

fn config() -> client::Config {
    for path in ["etc/chat.conf", "/etc/chat.conf"] {
        if let Some(text) = ecm_host_abi::fs::read_file(path) {
            return client::parse_config(&text);
        }
    }
    client::Config::default()
}

/// First interface MAC, for a default nickname.
fn mac_nick() -> String {
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

fn has_address() -> bool {
    (0..ecm_host_abi::net_config::iface_count()).any(|i| {
        ecm_host_abi::net_config::iface_info(i)
            .and_then(|info| client::json_field(&info, "ip").and_then(client::parse_ipv4))
            .is_some_and(|ip| ip != [0, 0, 0, 0] && ip[0] != 127)
    })
}

#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn fd_fdstat_set_flags(fd: u32, flags: u32) -> u32;
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match client::parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            out(&format!("chat: {}\n{}\n", e, client::USAGE));
            std::process::exit(2);
        }
    };
    if args.help {
        out(&format!("{}\n", client::USAGE));
        return;
    }
    let target = match client::resolve(&args, &config(), &mac_nick()) {
        Ok(t) => t,
        Err(p) => fail(p),
    };
    let ip = match client::parse_ipv4(&target.host) {
        Some(ip) => ip,
        None => {
            let mut addr = SockAddrIn::default();
            if socket::getaddrinfo(&target.host, &mut addr) != 0 {
                fail(Problem::Unresolved(target.host.clone()));
            }
            addr.ip()
        }
    };
    let ip_text = format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
    let endpoint = format!("{}:{}", ip_text, target.port);
    if !has_address() {
        fail(Problem::NoAddress);
    }
    if let Some(json) = ecm_host_abi::net_config::route_list() {
        if !client::routed(&client::parse_routes(&json), ip) {
            fail(Problem::NoRoute(ip_text));
        }
    }

    out(&format!("chat: connecting to {} as {} ...\n", endpoint, target.nick));
    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    if fd < 0 {
        out("chat: cannot create a socket\n");
        std::process::exit(1);
    }
    let started = Instant::now();
    if socket::connect(fd, &SockAddrIn::from_ip_port(ip, target.port)) != 0 {
        socket::close(fd);
        // A reset comes back at once; silence runs into the 10 s connect timeout.
        fail(if started.elapsed().as_millis() < 8000 { Problem::Refused(endpoint) } else { Problem::Timeout(endpoint) });
    }
    if !send_line(fd, &format!("NICK {}", target.nick)) {
        fail(Problem::Lost(endpoint));
    }

    unsafe {
        fd_fdstat_set_flags(0, 4); // stdin O_NONBLOCK
    }
    let mut me = target.nick.clone();
    let mut editor = Editor::default();
    let mut inbuf = Vec::new();
    let mut buf = [0u8; 2048];
    out(PROMPT);
    loop {
        let mut fds = [PollFd { fd, events: POLLIN, revents: 0 }];
        socket::poll(&mut fds, 50);
        if fds[0].revents & (POLLIN | POLLERR | POLLHUP) != 0 {
            match socket::recv(fd, &mut buf, MSG_DONTWAIT) {
                n if n > 0 => {
                    for line in take_lines(&mut inbuf, &buf[..n as usize]) {
                        let Some(event) = Event::parse(&line) else { continue };
                        match &event {
                            Event::Welcome { nick, .. } => me = nick.clone(),
                            Event::Nick { old, new, .. } if *old == me => me = new.clone(),
                            _ => {}
                        }
                        if let Some(text) = client::show(&event, &me) {
                            out(&editor.above(&text));
                        }
                    }
                }
                -2 => {}
                _ => {
                    out(&format!("\r\x1b[K{}\n", client::hint(&Problem::Lost(endpoint))));
                    socket::close(fd);
                    return;
                }
            }
        }
        let mut keys = [0u8; 256];
        let n = match std::io::stdin().read(&mut keys) {
            Ok(n) => n,
            Err(_) => 0,
        };
        for &b in &keys[..n] {
            match editor.key(b) {
                Key::Echo(e) => out(&e),
                Key::Nothing => {}
                Key::Quit => quit(fd),
                Key::Line(line) => {
                    // The server echoes messages back with a timestamp: clear the typed line.
                    out(&format!("\r\x1b[K{}", PROMPT));
                    match client::typed(&line) {
                        Typed::Send(req) => {
                            if !send_line(fd, &req) {
                                out(&format!("\r\x1b[K{}\n", client::hint(&Problem::Lost(endpoint.clone()))));
                                socket::close(fd);
                                return;
                            }
                        }
                        Typed::Show(text) => out(&editor.above(&text)),
                        Typed::Quit => quit(fd),
                        Typed::Nothing => {}
                    }
                }
            }
        }
    }
}

fn quit(fd: i32) -> ! {
    send_line(fd, "QUIT");
    socket::shutdown(fd, socket::SHUT_WR);
    socket::close(fd);
    out("\r\x1b[K*** bye\n");
    std::process::exit(0);
}

fn send_line(fd: i32, line: &str) -> bool {
    let data = format!("{}\n", line);
    let mut rest = data.as_bytes();
    while !rest.is_empty() {
        let n = socket::send(fd, rest, 0);
        if n <= 0 {
            return false;
        }
        rest = &rest[n as usize..];
    }
    true
}
