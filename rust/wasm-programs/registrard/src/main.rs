//! registrard: the registrar (docs/DNS.md "The registrar"): accounts, the web pages at
//! http://nic.ring/, the plain-text API the `domain` program uses (`POST /api`), and whois on
//! TCP 43. Domains are created and changed at the registry over EPP-lite
//! (`ecm_dns::registrar`, `ecm_dns::registry`).
//!
//! `/etc/registrard.conf`: `registry <address> <port>`, `id <registrar id>`, `secret <secret>`,
//! `port 80`, `tld ecm`, `db /etc/registrar.db`.

use ecm_dns::registrar::{form, Accounts, Registrar};
use ecm_dns::registrar_web::{self as web, Sessions};
use ecm_dns::sys::{self, now_ms, LineClient};
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, POLLIN};
use std::collections::BTreeMap;
use std::io::Write;

struct Conf {
    registry: [u8; 4],
    registry_port: u16,
    id: String,
    secret: String,
    port: u16,
    tld: String,
    db: String,
}

fn log(text: &str) {
    let t = now_ms() / 1000;
    println!("[{:02}:{:02}:{:02}] registrard: {}", (t / 3600) % 24, (t / 60) % 60, t % 60, text);
    let _ = std::io::stdout().flush();
}

fn conf(path: &str) -> Result<Conf, String> {
    let text = std::fs::read_to_string(path).map_err(|_| format!("cannot read {path}"))?;
    let mut c = Conf { registry: [0; 4], registry_port: 700, id: String::new(), secret: String::new(), port: 80, tld: "ecm".into(), db: "/etc/registrar.db".into() };
    for line in text.lines() {
        let w: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
        match w.as_slice() {
            [] => {}
            ["registry", ip, port] => {
                c.registry = ecm_dns::name::parse_ip(ip).ok_or_else(|| format!("bad address '{ip}'"))?;
                c.registry_port = port.parse().map_err(|_| format!("bad port '{port}'"))?;
            }
            ["id", id] => c.id = id.to_string(),
            ["secret", s] => c.secret = s.to_string(),
            ["port", p] => c.port = p.parse().map_err(|_| format!("bad port '{p}'"))?,
            ["tld", t] => c.tld = t.to_string(),
            ["db", f] => c.db = f.to_string(),
            _ => return Err(format!("{path}: cannot read '{line}'")),
        }
    }
    if c.id.is_empty() || c.registry == [0; 4] {
        return Err(format!("{path}: needs 'registry ADDRESS PORT', 'id' and 'secret'"));
    }
    Ok(c)
}

fn salt() -> String {
    let mut b = [0u8; 8];
    ecm_host_abi::random::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Run `f` with a registrar whose EPP channel is a fresh, logged-in connection to the registry.
fn with_registrar<T>(c: &Conf, accounts: &mut Accounts, f: impl FnOnce(&mut Registrar) -> T) -> T {
    let mut conn: Option<Result<LineClient, String>> = None;
    let mut epp = |cmd: &str| -> String {
        if conn.is_none() {
            conn = Some(LineClient::connect(c.registry, c.registry_port, 3000).and_then(|mut l| {
                let _greeting = l.read_line();
                let r = l.call(&format!("LOGIN {} {}", c.id, c.secret), false)?;
                if r.starts_with("OK") {
                    Ok(l)
                } else {
                    Err(format!("the registry refused our login: {r}"))
                }
            }));
        }
        match conn.as_mut().unwrap() {
            Ok(l) => {
                let multi = cmd.starts_with("INFO") || cmd.starts_with("LIST");
                l.call(cmd, multi).unwrap_or_else(|e| format!("ERR 2400 {e}"))
            }
            Err(e) => format!("ERR 2400 the registry is unreachable: {e}"),
        }
    };
    let mut r = Registrar { accounts, epp: &mut epp, tld: c.tld.clone() };
    f(&mut r)
}

fn handle(c: &Conf, sessions: &mut Sessions, fd: i32) {
    let Some(req) = sys::read_request(fd, 3000) else {
        socket::close(fd);
        return;
    };
    let mut accounts = Accounts::parse(&std::fs::read_to_string(&c.db).unwrap_or_default());
    let before = accounts.clone();
    let now = (now_ms() / 1000) as u64;
    let login = sessions.get(&req.cookie).cloned();
    let who = login.as_ref().map(|(h, _)| h.as_str());
    match (req.method.as_str(), req.path.as_str()) {
        ("POST", "/api") | ("POST", "/web") => {
            let mut f: BTreeMap<String, String> = form(&req.body);
            let op = f.get("op").cloned().unwrap_or_default();
            if req.path == "/web" && op == "logout" {
                sessions.end(&req.cookie);
                sys::reply_with(fd, "303 See Other", "text/plain", &[("Location", "/"), ("Set-Cookie", &web::set_cookie(""))], "");
                socket::close(fd);
                return;
            }
            // The web forms after a login only ask for the domain: the session fills the rest.
            if let (true, Some((h, pw))) = (req.path == "/web" && op != "account" && op != "login", &login) {
                f.entry("handle".into()).or_insert_with(|| h.clone());
                f.entry("password".into()).or_insert_with(|| pw.clone());
            }
            let s = salt();
            let out = with_registrar(c, &mut accounts, |r| r.api(&f, &s, now));
            log(&format!(
                "{} {} {}: {}",
                op,
                f.get("handle").map(String::as_str).unwrap_or("-"),
                f.get("domain").map(String::as_str).unwrap_or(""),
                out.lines().next().unwrap_or("")
            ));
            if req.path == "/api" {
                sys::reply(fd, "200 OK", "text/plain", &format!("{out}
"));
            } else if (op == "account" || op == "login") && out.starts_with("OK") {
                // Created or logged in: a session cookie, then the home page with the account's domains.
                let token = salt() + &salt();
                let h = f.get("handle").cloned().unwrap_or_default();
                sessions.start(&token, &h, f.get("password").map(String::as_str).unwrap_or(""));
                sys::reply_with(fd, "303 See Other", "text/plain", &[("Location", "/"), ("Set-Cookie", &web::set_cookie(&token))], "");
            } else {
                sys::reply(fd, "200 OK", "text/html", &web::result(&op, &out, who));
            }
        }
        ("GET", "/search") => {
            let f = form(&req.query);
            let out = with_registrar(c, &mut accounts, |r| r.api(&BTreeMap::from([("op".into(), "check".into()), ("domain".into(), f.get("domain").cloned().unwrap_or_default())]), "", now));
            sys::reply(fd, "200 OK", "text/html", &web::result("check", &out, who));
        }
        ("GET", "/whois") => {
            let f = form(&req.query);
            let out = with_registrar(c, &mut accounts, |r| r.whois(f.get("domain").map(String::as_str).unwrap_or("")));
            sys::reply(fd, "200 OK", "text/plain", &out);
        }
        ("GET", "/") | ("GET", "/index.html") => {
            let domains = match &login {
                Some((h, pw)) => {
                    let f = BTreeMap::from([("op".into(), "list".into()), ("handle".into(), h.clone()), ("password".into(), pw.clone())]);
                    web::domains_of_list(&with_registrar(c, &mut accounts, |r| r.api(&f, "", now)))
                }
                None => Vec::new(),
            };
            sys::reply(fd, "200 OK", "text/html", &web::home(&c.tld, who, &domains))
        }
        _ => sys::reply(fd, "404 Not Found", "text/plain", "not found
"),
    }
    socket::close(fd);
    if accounts != before && !sys::write_file(&c.db, &accounts.render()) {
        log(&format!("cannot write {}", c.db));
    }
}

fn whois(c: &Conf, fd: i32) {
    let mut line = Vec::new();
    let mut buf = [0u8; 256];
    let start = std::time::Instant::now();
    while !line.contains(&b'\n') && line.len() < 256 && start.elapsed().as_millis() < 3000 {
        let n = socket::recv(fd, &mut buf, 0);
        if n == -2 {
            continue;
        }
        if n <= 0 {
            break;
        }
        line.extend_from_slice(&buf[..n as usize]);
    }
    let q = String::from_utf8_lossy(&line).trim().to_string();
    let mut accounts = Accounts::parse(&std::fs::read_to_string(&c.db).unwrap_or_default());
    let out = with_registrar(c, &mut accounts, |r| r.whois(&q));
    log(&format!("whois {q}"));
    sys::send_all(fd, out.replace('\n', "\r\n").as_bytes());
    socket::shutdown(fd, socket::SHUT_WR);
    socket::close(fd);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = match args.as_slice() {
        [] => "/etc/registrard.conf".to_string(),
        [f, p] if f == "-c" => p.clone(),
        _ => {
            println!("Usage: registrard [-c FILE]   the registrar: http://nic.ring/ and whois (registrard &)");
            return;
        }
    };
    let c = match conf(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("registrard: {e}");
            std::process::exit(1);
        }
    };
    let web = match sys::listen(c.port) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("registrard: {e}");
            std::process::exit(1);
        }
    };
    let who = sys::listen(ecm_dns::registrar::WHOIS_PORT).unwrap_or(-1);
    log(&format!(
        "the registrar {}: web and API on port {}, whois on port 43, registry {}:{}. Ctrl+T to stop.",
        c.id,
        c.port,
        ecm_dns::name::ip_str(c.registry),
        c.registry_port
    ));
    let mut sessions = Sessions::default();
    loop {
        let mut fds = vec![PollFd { fd: web, events: POLLIN, revents: 0 }];
        if who >= 0 {
            fds.push(PollFd { fd: who, events: POLLIN, revents: 0 });
        }
        if socket::poll(&mut fds, 1000) <= 0 {
            continue;
        }
        for f in fds {
            if f.revents & POLLIN == 0 {
                continue;
            }
            let mut peer = SockAddrIn::default();
            let conn = socket::accept(f.fd, &mut peer);
            if conn < 0 {
                continue;
            }
            if f.fd == web {
                handle(&c, &mut sessions, conn);
            } else {
                whois(&c, conn);
            }
        }
    }
}
