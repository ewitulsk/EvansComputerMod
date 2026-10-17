//! registrard: the registrar (docs/DNS.md "The registrar"): accounts, the web pages at
//! http://nic.ring/, the plain-text API the `domain` program uses (`POST /api`), and whois on
//! TCP 43. Domains are created and changed at the registry over EPP-lite
//! (`ecm_dns::registrar`, `ecm_dns::registry`).
//!
//! `/etc/registrard.conf`: `registry <address> <port>`, `id <registrar id>`, `secret <secret>`,
//! `port 80`, `tld ecm`, `db /etc/registrar.db`.

use ecm_dns::registrar::{form, Accounts, Registrar};
use ecm_dns::sys::{self, esc, now_ms, LineClient};
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

fn page(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html><head><title>{t}</title></head>\n<body>\n<h1>nic.ring</h1>\n<p><a href=\"/\">home</a></p>\n<h2>{t}</h2>\n{body}\n</body></html>\n",
        t = esc(title)
    )
}

fn home(c: &Conf) -> String {
    page(
        "Register a .ecm name",
        &format!(
            "<p>The registrar of the Tech Village ring: register a <code>.{tld}</code> name and point it at your own computer, or at your own nameserver.\
             The .{tld} registry runs in another village; this registrar sends it your changes.</p>\n\
             <h3>Is it free?</h3>\n<form method=\"get\" action=\"/search\"><input name=\"domain\" value=\"mybase.{tld}\"><input type=\"submit\" value=\"Search\"></form>\n\
             <h3>Account</h3>\n<form method=\"post\" action=\"/web\"><input type=\"hidden\" name=\"op\" value=\"account\">\
             handle <input name=\"handle\"> password <input name=\"password\" type=\"password\"><input type=\"submit\" value=\"Create account\"></form>\n\
             <h3>Register</h3>\n<form method=\"post\" action=\"/web\"><input type=\"hidden\" name=\"op\" value=\"register\">\
             domain <input name=\"domain\"> handle <input name=\"handle\"> password <input name=\"password\" type=\"password\"><input type=\"submit\" value=\"Register\"></form>\n\
             <h3>Add a record</h3>\n<form method=\"post\" action=\"/web\"><input type=\"hidden\" name=\"op\" value=\"set\">\
             domain <input name=\"domain\"> name <input name=\"name\" value=\"www\"> type <input name=\"type\" value=\"A\"> data <input name=\"data\">\
             ttl <input name=\"ttl\" value=\"300\"> handle <input name=\"handle\"> password <input name=\"password\" type=\"password\"><input type=\"submit\" value=\"Save\"></form>\n\
             <h3>From a terminal</h3>\n<pre>domain account create HANDLE PASSWORD\ndomain register mybase.{tld}\ndomain set mybase.{tld} www A 100.70.0.120\n\
             domain ns mybase.{tld} ns1.mybase.{tld}=10.200.0.53\ndomain show mybase.{tld}\nwhois mybase.{tld}</pre>\n\
             <p>Rules: names are 3-63 letters, digits and hyphens; some are reserved (nic, www, root, v1-v10, ...); five per account; TTLs of at least 60 seconds.</p>",
            tld = c.tld
        ),
    )
}

fn handle(c: &Conf, fd: i32) {
    let Some(req) = sys::read_request(fd, 3000) else {
        socket::close(fd);
        return;
    };
    let mut accounts = Accounts::parse(&std::fs::read_to_string(&c.db).unwrap_or_default());
    let before = accounts.clone();
    let now = (now_ms() / 1000) as u64;
    match (req.method.as_str(), req.path.as_str()) {
        ("POST", "/api") | ("POST", "/web") => {
            let f: BTreeMap<String, String> = form(&req.body);
            let s = salt();
            let out = with_registrar(c, &mut accounts, |r| r.api(&f, &s, now));
            log(&format!(
                "{} {} {}: {}",
                f.get("op").map(String::as_str).unwrap_or("?"),
                f.get("handle").map(String::as_str).unwrap_or("-"),
                f.get("domain").map(String::as_str).unwrap_or(""),
                out.lines().next().unwrap_or("")
            ));
            if req.path == "/api" {
                sys::reply(fd, "200 OK", "text/plain", &format!("{out}\n"));
            } else {
                sys::reply(fd, "200 OK", "text/html", &page("Result", &format!("<pre>{}</pre>", esc(&out))));
            }
        }
        ("GET", "/search") => {
            let f = form(&req.query);
            let out = with_registrar(c, &mut accounts, |r| r.api(&BTreeMap::from([("op".into(), "check".into()), ("domain".into(), f.get("domain").cloned().unwrap_or_default())]), "", now));
            sys::reply(fd, "200 OK", "text/html", &page("Search", &format!("<pre>{}</pre>", esc(&out))));
        }
        ("GET", "/whois") => {
            let f = form(&req.query);
            let out = with_registrar(c, &mut accounts, |r| r.whois(f.get("domain").map(String::as_str).unwrap_or("")));
            sys::reply(fd, "200 OK", "text/plain", &out);
        }
        ("GET", "/") | ("GET", "/index.html") => sys::reply(fd, "200 OK", "text/html", &home(c)),
        _ => sys::reply(fd, "404 Not Found", "text/plain", "not found\n"),
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
                handle(&c, conn);
            } else {
                whois(&c, conn);
            }
        }
    }
}
