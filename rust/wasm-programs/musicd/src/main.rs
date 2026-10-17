//! musicd: the music backend. See the crate docs (`lib.rs`) and `man musicd`.
//!
//! The poll loop: HTTP clients on PORT ([`musicd::conn::Conn`], answered by
//! [`musicd::api::App`]), the realm handshake on PORT+1 with `--auth local`
//! ([`ecm_auth::ServerConn`]), and background work (scan, Data saver
//! transcodes) a slice per turn. Sends happen only on POLLOUT and at most a
//! chunk per client per turn, so one slow listener can't stall the others.

use ecm_auth::{display_fingerprint, fingerprint, hex, public_key, signing_key, unhex_n, Realm, ServerConn};
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLERR, POLLHUP, POLLIN, POLLOUT, SOCK_STREAM, SOL_SOCKET, SO_REUSEADDR};
use musicd::api::{App, Config};
use musicd::auth::Gatekeeper;
use musicd::config::{parse_args, AuthMode, Command, Opts, USAGE};
use musicd::conn::{Conn, CHUNK};
use musicd::disk::{Disk, StdDisk};
use std::collections::HashMap;
use std::io::Write;

/// The poll call takes at most 64 sockets: two listeners and these.
const MAX_CLIENTS: usize = 50;
const MAX_AUTH_CLIENTS: usize = 10;
/// Connections idle this long (nothing to send, nothing received) are closed.
const IDLE_MS: u64 = 120_000;
const AUTH_IDLE_MS: u64 = 60_000;
/// How often the library folder is checked for changes.
const RESCAN_MS: u64 = 30_000;

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn clock(ms: u64) -> String {
    let s = (ms / 1000) % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

fn log(text: &str) {
    println!("[{}] {}", clock(now_ms()), text);
    let _ = std::io::stdout().flush();
}

fn fail(msg: &str) -> ! {
    eprintln!("musicd: {msg}");
    std::process::exit(1);
}

/// The backend key's seed: `DATA/backend.key` (made now if missing).
fn backend_seed(disk: &mut StdDisk, data: &str) -> [u8; 32] {
    let file = format!("{data}/backend.key");
    if let Some(t) = disk.read_text(&file) {
        return unhex_n(t.trim()).unwrap_or_else(|| fail(&format!("{file}: not a key (64 hex digits)")));
    }
    let seed: [u8; 32] = ecm_auth::random();
    if !disk.write(&file, format!("{}\n", hex(&seed)).as_bytes()) {
        fail(&format!("can't write {file}"));
    }
    log(&format!("made a new backend key in {file}: {}", display_fingerprint(&fingerprint(&public_key(&signing_key(&seed))))));
    seed
}

/// The realm's public key for `--auth HOST:PORT`: given, cached, or fetched from the realm.
fn remote_realm(disk: &mut StdDisk, data: &str, endpoint: &str, fp: &str, given: Option<[u8; 32]>) -> Option<([u8; 32], Option<String>)> {
    if let Some(p) = given {
        return Some((p, None));
    }
    let cache = format!("{data}/realm.pub");
    if let Some(p) = disk.read_text(&cache).and_then(|t| unhex_n::<32>(t.trim())).filter(|p| fingerprint(p) == fp) {
        return Some((p, None));
    }
    let stream = ecm_auth::host::connect(endpoint).ok()?;
    let (_, info) = ecm_auth::handshake(stream).ok()?;
    if info.realm_fp != fp {
        log(&format!("the realm at {endpoint} has key {}, not the --auth-key {}: refusing it", display_fingerprint(&info.realm_fp), display_fingerprint(fp)));
        return None;
    }
    disk.write(&cache, format!("{}\n", hex(&info.realm_pub)).as_bytes());
    Some((info.realm_pub, Some(info.realm_name)))
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
    let mut disk = StdDisk;
    let seed = backend_seed(&mut disk, &o.data);
    let own_pub = public_key(&signing_key(&seed));
    let own_fp = fingerprint(&own_pub);
    if o.command == Command::Key {
        println!("fingerprint {own_fp}");
        println!("display     {}", display_fingerprint(&own_fp));
        println!("public key  {}", hex(&own_pub));
        if o.auth == AuthMode::Local {
            println!("realm line  {}", ecm_auth::RealmRef { name: o.name.clone(), fp: own_fp.clone(), endpoint: None }.to_line());
        }
        return;
    }
    let (cfg, gate, realm) = match &o.auth {
        AuthMode::Local => {
            let users = format!("{}/users", o.data);
            let realm = match disk.read_text(&users) {
                Some(t) => Realm::load(&seed, &o.name, &t).unwrap_or_else(|e| fail(&format!("{users}: {e}"))),
                None => Realm::new(&seed, &o.name).unwrap_or_else(|e| fail(&e.to_string())),
            };
            (config(&o, &o.name, &own_fp, format!("-:{}", o.port + 1)), Gatekeeper::new(own_pub, &own_fp, Some(own_pub)), Some(realm))
        }
        AuthMode::Remote { endpoint, fp, public } => {
            let known = if o.command == Command::Serve { remote_realm(&mut disk, &o.data, endpoint, fp, *public) } else { None };
            let name = known.as_ref().and_then(|k| k.1.clone()).unwrap_or_else(|| "Auth".into());
            (config(&o, &name, fp, endpoint.clone()), Gatekeeper::new(own_pub, fp, known.map(|k| k.0)), None)
        }
    };
    let mut app = App::new(cfg, disk, gate);
    for l in app.log.drain(..) {
        log(&l);
    }
    match o.command {
        Command::Scan => {
            let r = app.scan_now();
            for l in app.log.drain(..) {
                println!("{l}");
            }
            if r == Default::default() {
                println!("no changes: {} songs", app.lib.len());
            }
            for w in app.lib.warnings() {
                println!("warning: {} is {} kbps (above {})", w.path, w.kbps, app.cfg.max_kbps);
            }
            // A running musicd reloads the index when it sees this.
            let trigger = app.cfg.trigger_file();
            app.disk.write(&trigger, b"rescan\n");
        }
        Command::Status => {
            for l in app.status_lines() {
                println!("{l}");
            }
            match app.disk.read_text(&app.cfg.status_file()) {
                Some(s) => print!("{s}"),
                None => println!("no running musicd has reported yet"),
            }
        }
        _ => serve(&o, app, realm),
    }
}

fn config(o: &Opts, realm_name: &str, realm_fp: &str, auth_endpoint: String) -> Config {
    Config {
        name: o.name.clone(),
        port: o.port,
        library_dir: o.library.clone(),
        data_dir: o.data.clone(),
        transcode: o.transcode,
        public_browse: o.public_browse,
        max_kbps: o.max_kbps,
        realm_name: realm_name.to_string(),
        realm_fp: realm_fp.to_string(),
        auth_endpoint,
    }
}

fn listen(port: u16) -> i32 {
    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    if fd < 0 {
        fail("cannot create a socket");
    }
    socket::setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &1i32.to_le_bytes());
    if socket::bind(fd, &SockAddrIn::any(port)) != 0 || socket::listen(fd, 16) != 0 {
        socket::close(fd);
        fail(&format!("cannot listen on port {port} (already in use?)"));
    }
    fd
}

fn serve(o: &Opts, mut app: App<StdDisk>, mut realm: Option<Realm>) {
    let listener = listen(o.port);
    let auth_listener = realm.as_ref().map(|_| listen(o.port + 1));
    let users_file = format!("{}/users", o.data);
    if let Some(r) = realm.as_mut() {
        app.disk.write(&users_file, r.save_text().as_bytes());
        r.take_dirty();
    }
    app.start_scan();
    let started = now_ms();
    log(&format!(
        "musicd listening on port {}{}, \"{}\": {} songs, accounts {} {}. Ctrl+T to stop.",
        o.port,
        if realm.is_some() { format!(" (sign-in on {})", o.port + 1) } else { String::new() },
        o.name,
        app.lib.len(),
        app.cfg.realm_name,
        ecm_auth::short_fingerprint(&app.cfg.realm_fp)
    ));
    if app.gate.realm_pub().is_none() {
        log(&format!("can't reach the realm at {} yet: sign-ins wait until it answers", app.cfg.auth_endpoint));
    }
    #[cfg(target_arch = "wasm32")]
    ecm_disco::sys::announce(ecm_disco::types::MUSIC, o.port, &o.name);

    let mut clients: HashMap<i32, Conn> = HashMap::new();
    let mut auth_clients: HashMap<i32, (ServerConn, u64)> = HashMap::new();
    let mut buf = vec![0u8; 8192];
    let mut last_rescan = now_ms();
    let mut last_trigger = 0u64;
    let mut last_status = 0u64;
    let mut last_realm_try = now_ms();
    loop {
        let mut fds: Vec<PollFd> = Vec::with_capacity(clients.len() + auth_clients.len() + 2);
        fds.push(PollFd { fd: listener, events: POLLIN, revents: 0 });
        if let Some(a) = auth_listener {
            fds.push(PollFd { fd: a, events: POLLIN, revents: 0 });
        }
        let first_client = fds.len();
        for (&fd, c) in &clients {
            fds.push(PollFd { fd, events: POLLIN | if c.wants_write() { POLLOUT } else { 0 }, revents: 0 });
        }
        let first_auth = fds.len();
        for &fd in auth_clients.keys() {
            fds.push(PollFd { fd, events: POLLIN, revents: 0 });
        }
        let wait = if app.busy() { 0 } else { 1000 };
        if socket::poll(&mut fds, wait) < 0 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }
        let now = now_ms();
        let mut gone: Vec<i32> = Vec::new();
        if fds[0].revents & POLLIN != 0 {
            let mut peer = SockAddrIn::default();
            let c = socket::accept(listener, &mut peer);
            if c >= 0 {
                if clients.len() >= MAX_CLIENTS {
                    socket::close(c);
                } else {
                    clients.insert(c, Conn::new(&peer.to_string(), now));
                }
            }
        }
        if let Some(a) = auth_listener {
            if fds[1].revents & POLLIN != 0 {
                let mut peer = SockAddrIn::default();
                let c = socket::accept(a, &mut peer);
                if c >= 0 {
                    if auth_clients.len() >= MAX_AUTH_CLIENTS {
                        socket::close(c);
                    } else {
                        auth_clients.insert(c, (ServerConn::new(&peer.to_string()), now));
                    }
                }
            }
        }
        for f in &fds[first_client..first_auth] {
            let Some(conn) = clients.get_mut(&f.fd) else { continue };
            if f.revents & (POLLIN | POLLERR | POLLHUP) != 0 {
                match socket::recv(f.fd, &mut buf, MSG_DONTWAIT) {
                    n if n > 0 => conn.receive(&mut app, &buf[..n as usize], now),
                    -2 => {}
                    _ => {
                        gone.push(f.fd);
                        continue;
                    }
                }
            }
            if f.revents & POLLOUT != 0 {
                let data = conn.pending(&app.disk);
                if !data.is_empty() {
                    let n = socket::send(f.fd, &data[..data.len().min(CHUNK)], 0);
                    if n <= 0 {
                        gone.push(f.fd);
                        continue;
                    }
                    conn.sent(n as usize);
                    conn.last_ms = now;
                }
                // The response is out: answer a pipelined request waiting behind it.
                conn.process(&mut app, now);
            }
            if conn.finished() {
                socket::shutdown(f.fd, socket::SHUT_WR);
                gone.push(f.fd);
            }
        }
        if let Some(r) = realm.as_mut() {
            for f in &fds[first_auth..] {
                if f.revents & (POLLIN | POLLERR | POLLHUP) == 0 {
                    continue;
                }
                let Some((conn, seen)) = auth_clients.get_mut(&f.fd) else { continue };
                match socket::recv(f.fd, &mut buf, MSG_DONTWAIT) {
                    n if n > 0 => {
                        *seen = now;
                        let out = conn.receive(r, &buf[..n as usize], now);
                        for line in &out.log {
                            log(line);
                        }
                        if !out.send.is_empty() && !send_all(f.fd, &out.send) {
                            auth_clients.remove(&f.fd);
                            socket::close(f.fd);
                            continue;
                        }
                        if out.close {
                            socket::shutdown(f.fd, socket::SHUT_WR);
                            auth_clients.remove(&f.fd);
                            socket::close(f.fd);
                        }
                    }
                    -2 => {}
                    _ => {
                        auth_clients.remove(&f.fd);
                        socket::close(f.fd);
                    }
                }
            }
            if r.take_dirty() {
                app.disk.write(&users_file, r.save_text().as_bytes());
            }
        }
        for (&fd, c) in &clients {
            if c.idle() && now.saturating_sub(c.last_ms) > IDLE_MS {
                gone.push(fd);
            }
        }
        for fd in gone {
            if clients.remove(&fd).is_some() {
                socket::close(fd);
            }
        }
        auth_clients.retain(|&fd, (_, seen)| {
            let keep = now.saturating_sub(*seen) <= AUTH_IDLE_MS;
            if !keep {
                socket::close(fd);
            }
            keep
        });

        // Background work, a slice per turn.
        app.step();
        if now.saturating_sub(last_trigger) >= 2000 {
            last_trigger = now;
            let trigger = app.cfg.trigger_file();
            if app.disk.size(&trigger).is_some() {
                app.disk.remove(&trigger);
                log("musicd scan ran: reloading the index");
                app.reload_index();
                app.start_scan();
            }
        }
        if now.saturating_sub(last_rescan) >= RESCAN_MS {
            last_rescan = now;
            if app.start_scan() {
                log("the library folder changed: scanning");
            }
        }
        if app.gate.realm_pub().is_none() && now.saturating_sub(last_realm_try) >= RESCAN_MS {
            last_realm_try = now;
            if let AuthMode::Remote { endpoint, fp, public } = &o.auth {
                if let Some((p, name)) = remote_realm(&mut app.disk, &o.data, endpoint, fp, *public) {
                    app.gate.set_realm_pub(p);
                    if let Some(n) = name {
                        app.cfg.realm_name = n;
                    }
                    log(&format!("reached the realm {} at {endpoint}", app.cfg.realm_name));
                }
            }
        }
        if now.saturating_sub(last_status) >= RESCAN_MS {
            last_status = now;
            let s = format!(
                "running since {} on port {}: {} listener connection(s), {} song(s) waiting for Data saver{}\n",
                clock(started),
                o.port,
                clients.len(),
                app.transcode_backlog(),
                if app.scanning() { ", scanning" } else { "" }
            );
            let f = app.cfg.status_file();
            app.disk.write(&f, s.as_bytes());
        }
        for l in app.log.drain(..) {
            log(&l);
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
