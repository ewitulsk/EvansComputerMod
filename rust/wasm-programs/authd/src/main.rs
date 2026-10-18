//! authd: the general-purpose Auth server (plan: `plans/background-apps/MUSIC_AND_AUTH.md`).
//!
//! `authd [PORT] [--name REALM] [--data DIR] [--key-file FILE]` listens on TCP (default
//! 7600) and runs one realm ([`ecm_auth::Realm`]): accounts that Discord, Music and other
//! backends trust by the realm key's fingerprint (`--auth <host:port> --auth-key <fp>`).
//! Each connection is an [`ecm_auth::ServerConn`] fed from the poll loop. The realm key
//! is `DIR/realm.key` (hex seed, made on first start) unless `--key-file` names one;
//! the users are `DIR/users` (default DIR `/var/auth`), saved after every change.
//!
//! The admin commands (`authd key|users|disable|enable|reset|revoke`) work on the data
//! folder directly; a running authd notices the changed users file and reloads it.

use ecm_auth::{fingerprint, display_fingerprint, hex, unhex_n, Realm, ServerConn, DEFAULT_PORT};
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLERR, POLLHUP, POLLIN, SOCK_STREAM, SOL_SOCKET, SO_REUSEADDR};
use std::collections::HashMap;
use std::io::Write;

const USAGE: &str = "Usage: authd [PORT] [--name REALM] [--data DIR] [--key-file FILE]\n\
       authd key|users [--data DIR] [--key-file FILE]\n\
       authd disable|enable|reset|revoke USER [--data DIR]\n\
  The Auth server: accounts shared by Discord, Music and other backends.\n\
  TCP PORT (default 7600). REALM names the realm (default \"Auth\"). DIR keeps\n\
  the users and the realm key (default /var/auth; the key is DIR/realm.key,\n\
  made on first start, unless --key-file names one).\n\
  key      print the realm's fingerprint and public key (give it to the\n\
           backends: --auth HOST:7600 --auth-key FINGERPRINT)\n\
  users    list the accounts\n\
  disable  stop USER signing in (enable undoes it); signs USER out everywhere\n\
  reset    clear USER's password: the next sign-in with that name sets a new one\n\
  revoke   sign USER out on every device\n\
  Start it with 'authd &' and run 'disco serve &' so apps find it.";

/// Connections idle this long are closed.
const IDLE_MS: u64 = 60_000;
const MAX_CLIENTS: usize = 64;

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
    eprintln!("authd: {msg}");
    std::process::exit(1);
}

#[derive(Debug, PartialEq)]
struct Opts {
    port: u16,
    name: String,
    data: String,
    key_file: Option<String>,
    /// The command and its argument (`users`, `reset alice`), or a port.
    words: Vec<String>,
}

/// The options; None for --help.
fn parse_args(args: &[String]) -> Result<Option<Opts>, String> {
    let mut o = Opts { port: DEFAULT_PORT, name: "Auth".into(), data: "/var/auth".into(), key_file: None, words: Vec::new() };
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| args.get(i + 1).cloned().ok_or_else(|| format!("{} needs a value", args[i]));
        match args[i].as_str() {
            "-h" | "--help" => return Ok(None),
            "--name" => {
                o.name = ecm_auth::display_name(&value(i)?).map_err(|e| format!("--name: {e}"))?;
                i += 1;
            }
            "--data" => {
                o.data = value(i)?.trim_end_matches('/').to_string();
                i += 1;
            }
            "--key-file" => {
                o.key_file = Some(value(i)?);
                i += 1;
            }
            w => o.words.push(w.to_string()),
        }
        i += 1;
    }
    if let Some(p) = o.words.first() {
        if let Ok(n) = p.parse::<u16>() {
            if n == 0 || n == u16::MAX {
                return Err(format!("bad port '{p}'"));
            }
            o.port = n;
            o.words.remove(0);
        }
    }
    Ok(Some(o))
}

/// `authd users`: one line per account.
fn users_table(realm: &Realm, now: u64) -> Vec<String> {
    if realm.users().is_empty() {
        return vec!["no accounts yet".into()];
    }
    realm
        .users()
        .iter()
        .map(|u| {
            let state = if u.disabled {
                "disabled"
            } else if u.key.is_none() {
                "reset"
            } else {
                "ok"
            };
            format!("{:>4} {:<16} {:<8} {} signed in  {}", u.id, u.username, state, realm.sessions(&u.username, now), u.display_name)
        })
        .collect()
}

/// `authd disable|enable|reset|revoke USER` on a loaded realm: what to tell the operator.
fn admin(realm: &mut Realm, cmd: &str, user: &str) -> Result<String, ecm_auth::AuthError> {
    match cmd {
        "disable" => realm.disable(user).map(|_| format!("{user} is disabled and signed out everywhere")),
        "enable" => realm.enable(user).map(|_| format!("{user} can sign in again")),
        "reset" => realm.reset(user).map(|_| format!("{user}'s password is cleared: the next sign-in as {user} sets a new one")),
        _ => realm.revoke(user).map(|n| format!("{user} is signed out ({n} device{})", if n == 1 { "" } else { "s" })),
    }
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
    let seed = realm_seed(&o);
    let users_file = format!("{}/users", o.data);
    let load_realm = |text: Option<String>| -> Realm {
        match text {
            Some(t) => Realm::load(&seed, &o.name, &t).unwrap_or_else(|e| fail(&format!("{users_file}: {e}"))),
            None => Realm::new(&seed, &o.name).unwrap_or_else(|e| fail(&e.to_string())),
        }
    };
    let words: Vec<&str> = o.words.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["key"] => {
            let realm = load_realm(None);
            let fp = realm.fingerprint();
            println!("fingerprint {fp}");
            println!("display     {}", display_fingerprint(&fp));
            println!("public key  {}", hex(&realm.public_key()));
            println!("realm line  {}", ecm_auth::RealmRef { name: o.name.clone(), fp, endpoint: None }.to_line());
        }
        ["users"] => {
            for line in users_table(&load_realm(read(&users_file)), now_ms()) {
                println!("{line}");
            }
        }
        [cmd @ ("disable" | "enable" | "reset" | "revoke"), user] => {
            let mut realm = load_realm(read(&users_file));
            match admin(&mut realm, cmd, user) {
                Ok(msg) => {
                    save(&users_file, &realm.save_text());
                    println!("{msg}");
                }
                Err(e) => fail(&e.to_string()),
            }
        }
        [] => serve(&o, &seed, load_realm(read(&users_file)), &users_file),
        _ => fail(&format!("bad arguments: {}\n{USAGE}", o.words.join(" "))),
    }
}
/// The realm key's seed: `--key-file`, or `DIR/realm.key` (made now if missing).
fn realm_seed(o: &Opts) -> [u8; 32] {
    let parse = |text: &str, file: &str| -> [u8; 32] {
        unhex_n(text.trim()).unwrap_or_else(|| fail(&format!("{file}: not a key (64 hex digits)")))
    };
    if let Some(f) = &o.key_file {
        return match read(f) {
            Some(t) => parse(&t, f),
            None => fail(&format!("{f}: no such key file")),
        };
    }
    let file = format!("{}/realm.key", o.data);
    match read(&file) {
        Some(t) => parse(&t, &file),
        None => {
            let seed: [u8; 32] = ecm_auth::random();
            save(&file, &format!("{}\n", hex(&seed)));
            if read(&file).is_none() {
                fail(&format!("can't write {file}"));
            }
            log(&format!("made a new realm key in {file}: {}", display_fingerprint(&fingerprint(&ecm_auth::public_key(&ecm_auth::signing_key(&seed))))));
            seed
        }
    }
}

fn serve(o: &Opts, seed: &[u8; 32], mut realm: Realm, users_file: &str) {
    let listener = socket::socket(AF_INET, SOCK_STREAM, 0);
    if listener < 0 {
        fail("cannot create a socket");
    }
    socket::setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &1i32.to_le_bytes());
    if socket::bind(listener, &SockAddrIn::any(o.port)) != 0 || socket::listen(listener, 8) != 0 {
        socket::close(listener);
        fail(&format!("cannot listen on port {} (already in use?)", o.port));
    }
    let mut saved = realm.save_text();
    save(users_file, &saved);
    realm.take_dirty();
    let n = realm.users().len();
    log(&format!(
        "authd listening on port {}, realm \"{}\" {}: {} account{}. Ctrl+T to stop.",
        o.port,
        realm.name(),
        display_fingerprint(&realm.fingerprint()),
        n,
        if n == 1 { "" } else { "s" }
    ));
    // Found by apps when this computer runs `disco serve`; trust still comes from the pinned key.
    #[cfg(target_arch = "wasm32")]
    ecm_disco::sys::announce(ecm_disco::types::AUTH, o.port, realm.name());

    let mut clients: HashMap<i32, (ServerConn, u64)> = HashMap::new();
    let mut buf = [0u8; 4096];
    let mut last_check = now_ms();
    loop {
        let mut fds: Vec<PollFd> = Vec::with_capacity(clients.len() + 1);
        fds.push(PollFd { fd: listener, events: POLLIN, revents: 0 });
        for &c in clients.keys() {
            fds.push(PollFd { fd: c, events: POLLIN, revents: 0 });
        }
        if socket::poll(&mut fds, 1000) < 0 {
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
                    clients.insert(c, (ServerConn::new(&peer.to_string()), now));
                }
            }
        }
        for f in &fds[1..] {
            if f.revents & (POLLIN | POLLERR | POLLHUP) == 0 {
                continue;
            }
            let Some((conn, seen)) = clients.get_mut(&f.fd) else { continue };
            match socket::recv(f.fd, &mut buf, MSG_DONTWAIT) {
                n if n > 0 => {
                    *seen = now;
                    let out = conn.receive(&mut realm, &buf[..n as usize], now);
                    for line in &out.log {
                        log(line);
                    }
                    if !out.send.is_empty() && !send_all(f.fd, &out.send) {
                        gone.push(f.fd);
                    }
                    if out.close {
                        socket::shutdown(f.fd, socket::SHUT_WR);
                        gone.push(f.fd);
                    }
                }
                -2 => {}
                _ => gone.push(f.fd),
            }
        }
        for (&fd, (_, seen)) in &clients {
            if now.saturating_sub(*seen) > IDLE_MS {
                gone.push(fd);
            }
        }
        for fd in gone {
            if clients.remove(&fd).is_some() {
                socket::close(fd);
            }
        }
        if realm.take_dirty() {
            saved = realm.save_text();
            save(users_file, &saved);
        } else if now.saturating_sub(last_check) >= 2000 {
            // An admin command (authd disable, ...) changed the file: take it.
            last_check = now;
            if let Some(text) = read(users_file) {
                if text != saved {
                    match Realm::load(seed, &o.name, &text) {
                        Ok(r) => {
                            realm = r;
                            saved = text;
                            log("the users file changed: reloaded");
                        }
                        Err(e) => log(&format!("the users file changed but can't be read ({e}); keeping the running state")),
                    }
                }
            }
        }
    }
}

/// Write a file (best effort past the folder: a full disk only costs persistence).
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

fn read(file: &str) -> Option<String> {
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
        let o = parse_args(&args("7601 --name Tech Village --data /var/v/")).unwrap().unwrap();
        assert_eq!((o.port, o.name.as_str(), o.data.as_str()), (7601, "Tech", "/var/v"));
        assert_eq!(o.words, ["Village"], "a name with spaces is one argument (quoted by the shell)");
        let o = parse_args(&["--name".into(), "Tech Village".into(), "--key-file".into(), "/etc/k".into()]).unwrap().unwrap();
        assert_eq!((o.port, o.name.as_str(), o.key_file.as_deref(), o.words.len()), (DEFAULT_PORT, "Tech Village", Some("/etc/k"), 0));
        assert_eq!(parse_args(&args("reset alice")).unwrap().unwrap().words, ["reset", "alice"]);
        assert_eq!(parse_args(&args("--help")).unwrap(), None);
        assert!(parse_args(&args("0")).is_err() && parse_args(&args("--data")).is_err());
    }

    #[test]
    fn admin_commands_and_the_users_list() {
        let mut realm = Realm::new(&[1; 32], "Auth").unwrap();
        assert_eq!(users_table(&realm, 0), ["no accounts yet"]);
        let text = "ecm-auth-users 1\nkdf 50000\nnext 2\nU 1 alice ok 0000000000000000000000000000000000000000000000000000000000000000 5 Alice A.\nR 1 1111111111111111111111111111111111111111111111111111111111111111 100\n";
        realm = Realm::load(&[1; 32], "Auth", text).unwrap();
        assert_eq!(users_table(&realm, 50), ["   1 alice            ok       1 signed in  Alice A."]);
        assert_eq!(admin(&mut realm, "revoke", "alice").unwrap(), "alice is signed out (1 device)");
        assert_eq!(admin(&mut realm, "reset", "ALICE").unwrap(), "ALICE's password is cleared: the next sign-in as ALICE sets a new one");
        assert!(users_table(&realm, 50)[0].contains(" reset "));
        admin(&mut realm, "disable", "alice").unwrap();
        assert!(realm.save_text().contains("U 1 alice disabled - 5 Alice A."));
        admin(&mut realm, "enable", "alice").unwrap();
        assert_eq!(admin(&mut realm, "disable", "bob").unwrap_err().to_string(), "no user bob");
    }
}