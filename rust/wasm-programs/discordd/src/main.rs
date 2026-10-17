//! discordd: a Discord backend.
//!
//! `discordd [PORT] [--name NAME] [--data DIR] [--auth local|<host:port> --auth-key <fp>]`
//! listens on TCP (default 7777) for the Discord app and the `chat` command, and on UDP
//! PORT+1 for voice. It hosts servers with text and voice channels
//! (`ecm_discord::server::Backend`), keeps them in `DIR/state` (default
//! `/var/discord`), and announces itself to `disco serve` so Discord apps on the
//! network find it by themselves.
//!
//! Accounts are required. The backend has its own ed25519 key (`DIR/backend.key`,
//! made on first start; `discordd key` prints its fingerprint): tokens must name it.
//! `--auth local` (the default) runs the backend's own realm, whose key is the backend
//! key, with its users in `DIR/users`, answering the sign-in handshake on TCP PORT+2.
//! `--auth <host:port> --auth-key <fp>` trusts only tokens from the Auth server
//! (`authd`) with that key fingerprint; the backend learns the realm's public key from
//! the server once (checking the fingerprint) and keeps it in `DIR/realm.pub`.

use ecm_auth::{display_fingerprint, fingerprint, hex, unhex_n, RealmRef, ServerConn};
use ecm_discord::proto::DEFAULT_PORT;
use ecm_discord::server::{AuthSetup, Backend, Out};
use ecm_host_abi::socket::{
    self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLERR, POLLHUP, POLLIN, SOCK_DGRAM, SOCK_STREAM, SOL_SOCKET, SO_REUSEADDR,
};
use std::collections::HashMap;
use std::io::Write;

const USAGE: &str = "Usage: discordd [PORT] [--name NAME] [--data DIR] [--auth local|HOST:PORT --auth-key FP]\n\
       discordd key [--data DIR]\n\
  A Discord backend: servers with text and voice channels for the Discord app.\n\
  TCP PORT (default 7777) for clients, UDP PORT+1 for voice. NAME names its home\n\
  server (default \"Discord\"); DIR keeps its servers and keys (default /var/discord).\n\
  Accounts are required. --auth local (the default): the backend keeps its own\n\
  accounts and answers sign-ins on TCP PORT+2. --auth HOST:PORT --auth-key FP: use\n\
  the accounts of the Auth server (authd) at HOST:PORT whose key fingerprint is FP\n\
  ('authd key' prints it). 'discordd key' prints this backend's fingerprint.\n\
  Start it in the background with 'discordd &' or from services.cfg; run\n\
  'disco serve &' too so Discord apps on the network find it.";

/// Sign-in connections to the local realm at once (they are short).
const MAX_REALM_CLIENTS: usize = 4;
const REALM_IDLE_MS: u64 = 30_000;

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

fn fail(msg: &str) -> ! {
    eprintln!("discordd: {msg}");
    std::process::exit(1);
}

/// Where the accounts come from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AuthMode {
    Local,
    Shared { host: String, fp: String },
}

#[derive(Debug, PartialEq)]
struct Opts {
    port: u16,
    name: String,
    data: String,
    auth: AuthMode,
    /// `key`, or nothing.
    words: Vec<String>,
}

/// The options; None for --help.
fn parse_args(args: &[String]) -> Result<Option<Opts>, String> {
    let mut o = Opts { port: DEFAULT_PORT, name: "Discord".into(), data: "/var/discord".into(), auth: AuthMode::Local, words: Vec::new() };
    let mut auth: Option<String> = None;
    let mut key: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| args.get(i + 1).cloned().ok_or_else(|| format!("{} needs a value", args[i]));
        match args[i].as_str() {
            "-h" | "--help" => return Ok(None),
            "--name" => {
                o.name = ecm_discord::proto::guild_name(&value(i)?).map_err(|e| format!("--name: {e}"))?;
                i += 1;
            }
            "--data" => {
                o.data = value(i)?.trim_end_matches('/').to_string();
                i += 1;
            }
            "--auth" => {
                auth = Some(value(i)?);
                i += 1;
            }
            "--auth-key" => {
                key = Some(value(i)?);
                i += 1;
            }
            "key" => o.words.push("key".into()),
            p => match p.parse::<u16>() {
                Ok(n) if n > 0 && n < u16::MAX - 2 => o.port = n,
                _ => return Err(format!("bad port '{p}'")),
            },
        }
        i += 1;
    }
    o.auth = match (auth.as_deref(), key) {
        (None | Some("local"), None) => AuthMode::Local,
        (None | Some("local"), Some(_)) => return Err("--auth-key goes with --auth HOST:PORT".into()),
        (Some("none"), _) => return Err("accounts are required: --auth local or --auth HOST:PORT --auth-key FP".into()),
        (Some(host), Some(k)) => AuthMode::Shared {
            host: ecm_auth::endpoint(host).map_err(|e| format!("--auth: {e}"))?,
            fp: ecm_auth::parse_fingerprint(&k).ok_or_else(|| format!("--auth-key: '{k}' is not a fingerprint (32 hex digits)"))?,
        },
        (Some(_), None) => return Err("--auth HOST:PORT needs --auth-key FP (the Auth server's fingerprint: authd key)".into()),
    };
    Ok(Some(o))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = match parse_args(&args) {
        Ok(Some(o)) => o,
        Ok(None) => {
            println!("{USAGE}");
            return;
        }
        Err(e) => fail(&format!("{e}\n{USAGE}")),
    };
    let backend_seed = key_seed(&format!("{}/backend.key", o.data));
    let backend_pub = ecm_auth::public_key(&ecm_auth::signing_key(&backend_seed));
    if o.words.iter().any(|w| w == "key") {
        let fp = fingerprint(&backend_pub);
        println!("fingerprint {fp}");
        println!("display     {}", display_fingerprint(&fp));
        println!("public key  {}", hex(&backend_pub));
        return;
    }
    serve(&o, backend_seed);
}

/// A 32-byte seed kept as hex in `file` (made now if missing).
fn key_seed(file: &str) -> [u8; 32] {
    if let Some(t) = load(file) {
        return unhex_n(t.trim()).unwrap_or_else(|| fail(&format!("{file}: not a key (64 hex digits)")));
    }
    let seed: [u8; 32] = ecm_auth::random();
    save(file, &format!("{}\n", hex(&seed)));
    if load(file).is_none() {
        fail(&format!("can't write {file}"));
    }
    seed
}

/// The shared realm's public key and name: from `DIR/realm.pub` if it has this
/// fingerprint, else from the Auth server (retried until it answers with that key).
fn shared_realm(data: &str, host: &str, fp: &str) -> ([u8; 32], String) {
    let cache = format!("{data}/realm.pub");
    if let Some(t) = load(&cache) {
        let w: Vec<&str> = t.trim().splitn(3, ' ').collect();
        if w.len() == 3 && w[0] == fp {
            if let Some(k) = unhex_n::<32>(w[1]).filter(|k| fingerprint(k) == fp) {
                return (k, w[2].to_string());
            }
        }
    }
    loop {
        match ecm_auth::host::connect(host).and_then(ecm_auth::handshake) {
            Ok((_, info)) if info.realm_fp == fp => {
                save(&cache, &format!("{} {} {}\n", fp, hex(&info.realm_pub), info.realm_name));
                log(&format!("trusting the realm \"{}\" at {} ({})", info.realm_name, host, display_fingerprint(fp)));
                return (info.realm_pub, info.realm_name);
            }
            Ok((_, info)) => fail(&format!(
                "the Auth server at {host} has the key {}, not {} (--auth-key): refusing it",
                display_fingerprint(&info.realm_fp),
                display_fingerprint(fp)
            )),
            Err(e) => {
                log(&format!("waiting for the Auth server at {host}: {e}"));
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }
    }
}

fn listen_tcp(port: u16, what: &str) -> i32 {
    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    if fd < 0 {
        fail("cannot create a socket");
    }
    socket::setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &1i32.to_le_bytes());
    if socket::bind(fd, &SockAddrIn::any(port)) != 0 || socket::listen(fd, 8) != 0 {
        socket::close(fd);
        fail(&format!("cannot listen on port {port} for {what} (already in use?)"));
    }
    fd
}

fn serve(o: &Opts, backend_seed: [u8; 32]) {
    let port = o.port;
    let voice_port = port + 1;
    let realm_port = port + 2;
    let state_file = format!("{}/state", o.data);
    let users_file = format!("{}/users", o.data);
    let backend_pub = ecm_auth::public_key(&ecm_auth::signing_key(&backend_seed));

    // Who the backend trusts; with --auth local, the realm it runs.
    let mut realm = None;
    let auth = match &o.auth {
        AuthMode::Local => {
            let r = match load(&users_file) {
                Some(t) => ecm_auth::Realm::load(&backend_seed, &o.name, &t).unwrap_or_else(|e| fail(&format!("{users_file}: {e}"))),
                None => ecm_auth::Realm::new(&backend_seed, &o.name).unwrap_or_else(|e| fail(&e.to_string())),
            };
            realm = Some(r);
            AuthSetup {
                realm: RealmRef { name: o.name.clone(), fp: fingerprint(&backend_pub), endpoint: Some(format!("-:{realm_port}")) },
                realm_pub: backend_pub,
                backend_pub,
            }
        }
        AuthMode::Shared { host, fp } => {
            let (realm_pub, realm_name) = shared_realm(&o.data, host, fp);
            AuthSetup { realm: RealmRef { name: realm_name, fp: fp.clone(), endpoint: Some(host.clone()) }, realm_pub, backend_pub }
        }
    };

    let listener = listen_tcp(port, "clients");
    let udp = socket::socket(AF_INET, SOCK_DGRAM, 0);
    if udp < 0 || socket::bind(udp, &SockAddrIn::any(voice_port)) != 0 {
        socket::close(listener);
        fail(&format!("cannot use UDP port {voice_port} for voice (already in use?)"));
    }
    let realm_listener = if realm.is_some() { listen_tcp(realm_port, "sign-ins") } else { -1 };
    if let Some(r) = &mut realm {
        save(&users_file, &r.save_text());
        r.take_dirty();
    }

    let mut backend = match load(&state_file) {
        Some(text) => Backend::load(&o.name, voice_port, seed(), auth.clone(), &text),
        None => Backend::new(&o.name, voice_port, seed(), auth.clone()),
    };
    save(&state_file, &backend.save_text());
    let servers = backend.guild_names().len();
    log(&format!(
        "discordd listening on port {} (voice on UDP {}), \"{}\": {} server{}. Ctrl+T to stop.",
        port,
        voice_port,
        o.name,
        servers,
        if servers == 1 { "" } else { "s" }
    ));
    log(&format!("backend key {}", display_fingerprint(&backend.audience())));
    match &o.auth {
        AuthMode::Local => log(&format!("accounts: its own (--auth local), sign-ins on TCP {realm_port}")),
        AuthMode::Shared { host, .. } => log(&format!("accounts: \"{}\" at {} ({})", auth.realm.name, host, display_fingerprint(&auth.realm.fp))),
    }
    // Found by Discord apps on the network (and the chat command) when this computer runs `disco serve`.
    #[cfg(target_arch = "wasm32")]
    {
        ecm_disco::sys::announce(ecm_disco::types::DISCORD, port, &o.name);
        ecm_disco::sys::announce(ecm_disco::types::CHAT, port, &o.name);
    }

    let mut clients: Vec<i32> = Vec::new();
    let mut realm_clients: HashMap<i32, (ServerConn, u64)> = HashMap::new();
    let mut buf = [0u8; 4096];
    loop {
        let mut fds: Vec<PollFd> = Vec::with_capacity(clients.len() + realm_clients.len() + 3);
        fds.push(PollFd { fd: listener, events: POLLIN, revents: 0 });
        fds.push(PollFd { fd: udp, events: POLLIN, revents: 0 });
        if realm_listener >= 0 {
            fds.push(PollFd { fd: realm_listener, events: POLLIN, revents: 0 });
        }
        let fixed = fds.len();
        for &c in &clients {
            fds.push(PollFd { fd: c, events: POLLIN, revents: 0 });
        }
        let realm_from = fds.len();
        for &c in realm_clients.keys() {
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
        // The local realm's sign-ins (--auth local).
        if let Some(r) = &mut realm {
            if fds[2].revents & POLLIN != 0 {
                let mut peer = SockAddrIn::default();
                let c = socket::accept(realm_listener, &mut peer);
                if c >= 0 {
                    if realm_clients.len() >= MAX_REALM_CLIENTS {
                        socket::close(c);
                    } else {
                        realm_clients.insert(c, (ServerConn::new(&peer.to_string()), now));
                    }
                }
            }
            let mut done: Vec<i32> = Vec::new();
            for f in &fds[realm_from..] {
                if f.revents & (POLLIN | POLLERR | POLLHUP) == 0 {
                    continue;
                }
                let Some((conn, seen)) = realm_clients.get_mut(&f.fd) else { continue };
                match socket::recv(f.fd, &mut buf, MSG_DONTWAIT) {
                    n if n > 0 => {
                        *seen = now;
                        let out = conn.receive(r, &buf[..n as usize], now);
                        for line in &out.log {
                            log(line);
                        }
                        if (!out.send.is_empty() && !send_all(f.fd, &out.send)) || out.close {
                            done.push(f.fd);
                        }
                    }
                    -2 => {}
                    _ => done.push(f.fd),
                }
            }
            for (&fd, (_, seen)) in &realm_clients {
                if now.saturating_sub(*seen) > REALM_IDLE_MS {
                    done.push(fd);
                }
            }
            for fd in done {
                if realm_clients.remove(&fd).is_some() {
                    socket::shutdown(fd, socket::SHUT_WR);
                    socket::close(fd);
                }
            }
            if r.take_dirty() {
                save(&users_file, &r.save_text());
            }
        }
        for f in &fds[fixed..realm_from] {
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
            save(&state_file, &backend.save_text());
        }
    }
}

/// Write a file (best effort: a full disk only costs persistence).
fn save(file: &str, text: &str) {
    if std::fs::write(file, text).is_ok() {
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
    if std::fs::write(file, text).is_err() {
        ecm_host_abi::fs::write_file(rel, text);
    }
}

/// A saved file, if any.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn arguments() {
        let o = parse_args(&args("7790 --name Village --data /var/v/")).unwrap().unwrap();
        assert_eq!((o.port, o.name.as_str(), o.data.as_str(), &o.auth), (7790, "Village", "/var/v", &AuthMode::Local));
        let fp = "3fa19c0d00000000000000000000beef";
        let o = parse_args(&args(&format!("--auth 10.0.8.1 --auth-key {}", ecm_auth::display_fingerprint(fp)))).unwrap().unwrap();
        assert_eq!(o.auth, AuthMode::Shared { host: "10.0.8.1:7600".into(), fp: fp.into() });
        assert_eq!(parse_args(&args("--auth local")).unwrap().unwrap().auth, AuthMode::Local);
        assert_eq!(parse_args(&args("key --data /x")).unwrap().unwrap().words, ["key"]);
        assert_eq!(parse_args(&args("--help")).unwrap(), None);
        // Accounts are required; a shared realm needs its key.
        assert!(parse_args(&args("--auth none")).unwrap_err().contains("required"));
        assert!(parse_args(&args("--auth 10.0.8.1:7600")).unwrap_err().contains("--auth-key"));
        assert!(parse_args(&args("--auth-key 00")).is_err());
        assert!(parse_args(&args("--auth 10.0.8.1 --auth-key nothex")).is_err());
        assert!(parse_args(&args("70000")).is_err());
    }
}
