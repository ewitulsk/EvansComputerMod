//! chat: the Tech Village instant-messaging client.
//!
//! `chat [--user NAME] [server[:port]]`, `chat --forget FINGERPRINT`. With no server it
//! reads `/etc/chat.conf` (provisioned on Tech Village computers) or asks the network.
//! Incoming messages print above the line you are typing; `/who`, `/help`, `/quit`.
//! When the server cannot be reached it says why and how to fix it (no address, no
//! route, refused, no answer).
//!
//! Accounts are required (the backend, `discordd`, trusts one realm). The backend's
//! greeting names the realm; `chat --user NAME` asks for the password (not shown),
//! signs in there through `ecm_auth` and keeps the login (a refresh token, never the
//! password) in `~/.chat/accounts/`, so a plain `chat` signs in by itself next time.
//! The first sign-in at a realm shows its name and fingerprint and asks before
//! trusting it; a realm whose key changed is refused until `chat --forget <fp>`.

mod account;

use account::{account_args, USAGE};
use ecm_chat::client::{self, Editor, Key, Problem, Typed, PROMPT};
use ecm_auth::host::DirFiles;
use ecm_auth::{AccountStore, AuthError, Login, PinCheck, RealmRef};
use ecm_chat::proto::{take_lines, Event};
use ecm_discord::proto::{realm_host, Greeting};
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
    let all: Vec<String> = std::env::args().skip(1).collect();
    let (acct, argv) = match account_args(&all) {
        Ok(x) => x,
        Err(e) => {
            out(&format!("chat: {}\n{}\n", e, USAGE));
            std::process::exit(2);
        }
    };
    if let Some(fp) = &acct.forget {
        forget(fp);
        return;
    }
    let args = match client::parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            out(&format!("chat: {}\n{}\n", e, USAGE));
            std::process::exit(2);
        }
    };
    if args.help || acct.help {
        out(&format!("{}\n", USAGE));
        return;
    }
    let mut config = config();
    if args.server.is_none() && config.server.is_none() {
        // No server given or configured: ask the network (docs/DISCOVERY.md).
        out("chat: no server given; looking on the network ...\n");
        #[cfg(target_arch = "wasm32")]
        let found = ecm_disco::sys::search(ecm_disco::types::CHAT, ecm_disco::CLIENT_WAIT_MS).unwrap_or_default();
        // (Host builds, for the unit tests: no network.)
        #[cfg(not(target_arch = "wasm32"))]
        let found: Vec<ecm_disco::Found> = Vec::new();
        match found.as_slice() {
            [] => fail(Problem::NoServer),
            [one] => {
                out(&format!("chat: found {} at {}\n", one.label(), one.endpoint()));
                config.server = Some(one.address());
                config.port = Some(one.port);
            }
            many => {
                out("chat: several chat servers on the network; pick one with chat ADDRESS[:PORT]:\n");
                for f in many {
                    out(&format!("  {:<21} {}\n", f.endpoint(), f.label()));
                }
                std::process::exit(1);
            }
        }
    }
    // The nickname is no longer used: chat shows the account's username.
    let target = match client::resolve(&args, &config, "-") {
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

    out(&format!("chat: connecting to {} ...\n", endpoint));
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
    let mut inbuf = Vec::new();
    let line = sign_in(fd, &mut inbuf, &ip_text, &acct, &endpoint);
    if !send_line(fd, &line) {
        fail(Problem::Lost(endpoint));
    }

    unsafe {
        fd_fdstat_set_flags(0, 4); // stdin O_NONBLOCK
    }
    let mut me = String::new();
    let mut editor = Editor::default();
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
                            Event::Err(why) if me.is_empty() => {
                                out(&format!("\r\x1b[Kchat: the backend refused the sign-in: {why}\n"));
                                std::process::exit(1);
                            }
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
                    // The server echoes messages back with a timestamp: clear a typed message.
                    // A command (/who, /nick...) stays on screen above its answer.
                    if line.trim_start().starts_with('/') {
                        out(&format!("\n{}", PROMPT));
                    } else {
                        out(&format!("\r\x1b[K{}", PROMPT));
                    }
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

// ---------------------------------------------------------------- signing in

/// The accounts folder in the user's home.
fn store() -> AccountStore<DirFiles> {
    AccountStore::new(DirFiles::new(&account::accounts_dir(std::env::var("HOME").ok().as_deref())))
}

/// `chat --forget FP`: remove a realm's pin (and its saved login) by hand.
fn forget(fp: &str) {
    match store().remove_pin(fp) {
        Ok(true) => out(&format!("chat: forgot the realm {} and its login\n", ecm_auth::display_fingerprint(fp))),
        Ok(false) => out(&format!("chat: no realm {} is pinned\n", ecm_auth::display_fingerprint(fp))),
        Err(e) => refuse(&e),
    }
}

fn refuse(e: &AuthError) -> ! {
    out(&format!("{}\n", account::explain(e)));
    std::process::exit(1);
}

/// A connection to the realm at `host`.
fn realm(host: &str) -> Box<dyn ecm_auth::Stream> {
    ecm_auth::host::connect(host).unwrap_or_else(|e| refuse(&e))
}

/// Read a line from the keyboard (before the chat loop makes stdin non-blocking);
/// `echo` false for a password: nothing is shown.
fn ask(prompt: &str, echo: bool) -> String {
    out(prompt);
    let mut ed = Editor::default();
    let mut b = [0u8; 1];
    loop {
        match std::io::stdin().read(&mut b) {
            Ok(1) => {}
            _ => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
        match ed.key(b[0]) {
            Key::Echo(e) if echo => out(&e),
            Key::Line(l) => {
                out("\n");
                return l;
            }
            Key::Quit => {
                out("\n");
                std::process::exit(130);
            }
            _ => {}
        }
        b[0] = 0;
    }
}

/// The backend's two greeting lines.
fn greeting(fd: i32, inbuf: &mut Vec<u8>, endpoint: &str) -> Greeting {
    let mut g = Greeting::default();
    let started = Instant::now();
    let mut buf = [0u8; 2048];
    while !g.complete() {
        if started.elapsed().as_secs() > 10 {
            out(&format!("chat: {endpoint} didn't greet: is it a Discord backend (discordd)?\n"));
            std::process::exit(1);
        }
        let mut fds = [PollFd { fd, events: POLLIN, revents: 0 }];
        socket::poll(&mut fds, 200);
        if fds[0].revents & (POLLIN | POLLERR | POLLHUP) == 0 {
            continue;
        }
        match socket::recv(fd, &mut buf, MSG_DONTWAIT) {
            n if n > 0 => {
                for line in ecm_discord::server::take_lines(inbuf, &buf[..n as usize]) {
                    if let Some(chat_err) = line.strip_prefix("ERR ") {
                        out(&format!("chat: {chat_err}\n"));
                        std::process::exit(1);
                    }
                    g.feed(&line);
                }
            }
            -2 => {}
            _ => fail(Problem::Lost(endpoint.to_string())),
        }
    }
    g
}

/// Sign in with the password (asking first whether to trust a realm seen for the first time).
fn password_sign_in(store: &mut AccountStore<DirFiles>, trusted: &RealmRef, host: &str, user: &str, new: bool) -> Login {
    let (info, check) = store.fingerprint(realm(host), host).unwrap_or_else(|e| refuse(&e));
    if info.realm_fp != trusted.fp {
        out(&format!(
            "chat: the Auth server at {host} is \"{}\" ({}), not the realm this backend trusts ({})\n",
            info.realm_name,
            ecm_auth::display_fingerprint(&info.realm_fp),
            ecm_auth::display_fingerprint(&trusted.fp)
        ));
        std::process::exit(1);
    }
    if check == PinCheck::FirstContact && !account::yes(&ask(&account::first_contact(&info.realm_name, host, &info.realm_fp), true)) {
        out("chat: not signed in\n");
        std::process::exit(1);
    }
    for attempt in 0..3 {
        let pw = ask(&format!("Password for {user} at {}: ", info.realm_name), false);
        let r = if new {
            if ask("Password again: ", false) != pw {
                out("chat: the passwords are different\n");
                continue;
            }
            store.sign_up(realm(host), host, user, &pw, None)
        } else {
            store.sign_in(realm(host), host, user, &pw)
        };
        match r {
            Ok(l) => {
                out(&format!("chat: signed in to {} as {}\n", info.realm_name, l.display_name));
                return l;
            }
            Err(AuthError::WrongPassword) if attempt < 2 => out("chat: wrong username or password\n"),
            Err(e) => refuse(&e),
        }
    }
    std::process::exit(1);
}

/// Read the greeting, sign in at the realm the backend trusts (saved login, or the
/// password with `--user`), and make the `LOGIN` line for the backend.
fn sign_in(fd: i32, inbuf: &mut Vec<u8>, backend_ip: &str, acct: &account::AccountArgs, endpoint: &str) -> String {
    let g = greeting(fd, inbuf, endpoint);
    let trusted = g.realm.clone().unwrap();
    let Some(host) = realm_host(&trusted, backend_ip) else {
        out("chat: the backend doesn't say where its accounts are\n");
        std::process::exit(1);
    };
    let mut store = store();
    let user = acct.user.as_deref();
    let saved = if acct.new { None } else { store.login(&trusted.fp).filter(|l| user.is_none_or(|u| l.username == u)) };
    let mut login = match saved {
        Some(l) => l,
        None => match user {
            Some(u) => password_sign_in(&mut store, &trusted, &host, u, acct.new),
            None => {
                out(&format!("chat: sign in first: chat --user NAME (accounts of \"{}\")\n", trusted.name));
                std::process::exit(1);
            }
        },
    };
    let token = match store.token(realm(&login.host), &trusted.fp, &g.backend_fp) {
        Ok((t, _)) => t,
        Err(AuthError::SignInExpired) => {
            out("chat: your sign-in has expired\n");
            let u = login.username.clone();
            login = password_sign_in(&mut store, &trusted, &host, &u, false);
            store.token(realm(&login.host), &trusted.fp, &g.backend_fp).unwrap_or_else(|e| refuse(&e)).0
        }
        Err(e) => refuse(&e),
    };
    let login = store.login(&trusted.fp).unwrap_or(login);
    g.sign_in_line(&token, &login.session_key(), true)
}
