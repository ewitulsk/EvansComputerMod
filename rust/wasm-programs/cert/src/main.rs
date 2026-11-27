//! `cert` - this computer's certificates (docs/TLS.md).
//!
//!   cert trust <authority>            trust an authority (asks it for its key)
//!   cert trust --key <hex> <name...>  trust an authority by its key
//!   cert untrust <name>
//!   cert authorities                  who this computer trusts
//!   cert request <name>... [--ca <authority>] [--webroot <dir>]
//!   cert renew [--force]
//!   cert show [file]

use ecm_ca::{anchor_file, authority_address, challenge_content, days_left, finalize_request, Order, OrderRequest, Renewal, RENEW_DAYS_LEFT};
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, AF_INET, POLLIN, SOCK_STREAM, SOL_SOCKET, SO_REUSEADDR};
use ecm_http::{Request, Transfer};
use ecm_tls::x509::{self, SubjectName};
use ecm_tls::{Authority, Trust, CERTS_DIR, SERVER_CERT, SERVER_KEY};
use std::time::{Duration, Instant};

const CA_CONF: &str = "/etc/ssl/ca.conf";
const REQ_FILE: &str = "/etc/ssl/server.req";
const USAGE: &str = "Usage:
  cert trust <authority>             trust the authority at that name or address
  cert trust --key <hex> <name...>   trust an authority by its key
  cert untrust <name>                stop trusting one
  cert authorities                   list who this computer trusts
  cert request <name>... [--ca <authority>] [--webroot <dir>]
  cert renew [--force]               ask again when fewer than 30 days are left
  cert show [file]                   this computer's certificate (or a file's)";

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn write(path: &str, data: &str) -> Result<(), String> {
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(path, data).map_err(|e| format!("cannot write {path}: {e}"))
}

fn client() -> ecm_http::Client<ecm_http::sys::Sockets> {
    ecm_http::Client::new(ecm_http::sys::Sockets { idle: Duration::from_secs(30) }).agent("cert/1")
}

fn fetch(req: &Request) -> Result<ecm_http::Response, String> {
    client().exchange(req, &mut Transfer { max_body: 64 * 1024, ..Transfer::default() }).map_err(|e| e.to_string())
}

/// The authority to ask: `--ca`, else the one last trusted, else the name `ca`.
fn default_authority(arg: Option<&str>) -> Result<(String, u16), String> {
    let s = match arg {
        Some(a) => a.to_string(),
        None => std::fs::read_to_string(CA_CONF).map(|s| s.trim().to_string()).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "ca".into()),
    };
    authority_address(&s).ok_or_else(|| format!("'{s}' is not an authority's name or address"))
}

fn trust(args: &[String]) -> Result<(), String> {
    let (line, from) = if args.first().map(String::as_str) == Some("--key") {
        let key = args.get(1).ok_or("cert trust --key <hex> <name...>")?;
        let name = args[2..].join(" ");
        (format!("ed25519 {} {}\n", key.to_ascii_lowercase(), name), None)
    } else {
        let (host, port) = authority_address(args.first().ok_or(USAGE)?).ok_or("that is not an authority's name or address")?;
        let r = fetch(&Request::get(&format!("http://{host}:{port}/ca")).map_err(|e| e.to_string())?)?;
        if r.status != 200 {
            return Err(format!("{host}:{port} answered HTTP {} (is cad running there?)", r.status));
        }
        (r.text(), Some(format!("{host}:{port}")))
    };
    let a = Authority::parse_anchor(&line, "").ok_or("that is not an authority (expected: ed25519 <64 hex digits> <name>)")?;
    let file = format!("{CERTS_DIR}/{}", anchor_file(&a.common_name));
    write(&file, &format!("{}\n", line.trim()))?;
    if let Some(addr) = from {
        write(CA_CONF, &format!("{addr}\n"))?;
    }
    let key = line.split_whitespace().nth(1).unwrap_or("");
    println!("Now trusting \"{}\"", a.common_name);
    println!("  key {}", key);
    println!("  saved as {file}");
    println!("Check the key with the authority's owner (cad show) if it matters who it is.");
    Ok(())
}

fn untrust(name: &str) -> Result<(), String> {
    let t = Trust::load(CERTS_DIR);
    let lower = name.to_ascii_lowercase();
    let hit = t.ingame.iter().find(|a| a.common_name.to_ascii_lowercase() == lower || a.source == name).ok_or_else(|| format!("no trusted authority named {name} (see: cert authorities)"))?;
    let file = format!("{CERTS_DIR}/{}", hit.source);
    std::fs::remove_file(&file).map_err(|e| format!("cannot delete {file}: {e}"))?;
    println!("No longer trusting \"{}\".", hit.common_name);
    Ok(())
}

fn authorities() {
    let t = Trust::load(CERTS_DIR);
    if t.ingame.is_empty() {
        println!("This computer trusts no in-game authority (cert trust <authority> adds one).");
    }
    for a in &t.ingame {
        println!("{:<32} {}/{}", a.common_name, CERTS_DIR, a.source);
    }
    for s in &t.skipped {
        println!("(not an authority: {CERTS_DIR}/{s})");
    }
    println!("Servers on the real internet are checked against the built-in public authorities.");
}

fn describe(text: &str, what: &str) -> Result<(), String> {
    let ders = x509::pem_blocks(text, x509::CERTIFICATE);
    let info = ders.first().and_then(|d| x509::describe(d)).ok_or_else(|| format!("{what} holds no certificate"))?;
    let names: Vec<String> = info.names.iter().map(|n| n.to_string()).collect();
    println!("{what}");
    println!("  for        {}", if names.is_empty() { info.subject.clone() } else { names.join(", ") });
    println!("  signed by  {}", info.issuer);
    println!("  from       {}", x509::format_time(info.not_before));
    let left = days_left(info.not_after, now());
    let state = if now() >= info.not_after {
        "OUT OF DATE".to_string()
    } else if left < RENEW_DAYS_LEFT {
        format!("{left} days left: cert renew")
    } else {
        format!("{left} days left")
    };
    println!("  until      {} ({state})", x509::format_time(info.not_after));
    Ok(())
}

/// One request to the authority, answered while this program also serves the
/// challenge on port 80 (so no web server is needed).
fn finalize_standalone(host: &str, port: u16, body: &str, challenge_path: &str, content: &str) -> Result<String, String> {
    let ip = ecm_http::sys::resolve(host).map_err(|e| e.to_string())?;
    let listen = socket::socket(AF_INET, SOCK_STREAM, 0);
    if listen < 0 {
        return Err("cannot open a socket".into());
    }
    socket::setsockopt(listen, SOL_SOCKET, SO_REUSEADDR, &1i32.to_le_bytes());
    if socket::bind(listen, &SockAddrIn::any(80)) != 0 || socket::listen(listen, 4) != 0 {
        socket::close(listen);
        return Err("port 80 is in use here (a web server?); give its folder with --webroot <dir>".into());
    }
    let ca = socket::socket(AF_INET, SOCK_STREAM, 0);
    let fail = |m: String| {
        socket::close(listen);
        socket::close(ca);
        Err(m)
    };
    if ca < 0 || socket::connect(ca, &SockAddrIn::from_ip_port(ip, port)) != 0 {
        return fail(format!("cannot connect to the authority at {host}:{port}"));
    }
    let req = Request::new("POST", &format!("http://{host}:{port}/finalize")).map_err(|e| e.to_string())?.body("text/plain", body.as_bytes().to_vec());
    let bytes = req.encode("cert/1");
    let mut rest = &bytes[..];
    while !rest.is_empty() {
        let n = socket::send(ca, &rest[..rest.len().min(1400)], 0);
        if n <= 0 {
            return fail("the connection to the authority broke".into());
        }
        rest = &rest[n as usize..];
    }
    let started = Instant::now();
    let mut answer = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        if started.elapsed() > Duration::from_secs(60) {
            return fail("the authority did not answer within a minute".into());
        }
        let mut fds = [PollFd { fd: listen, events: POLLIN, revents: 0 }, PollFd { fd: ca, events: POLLIN, revents: 0 }];
        if socket::poll(&mut fds, 500) <= 0 {
            continue;
        }
        if fds[0].revents & POLLIN != 0 {
            let mut peer = SockAddrIn::default();
            let conn = socket::accept(listen, &mut peer);
            if conn >= 0 {
                let mut sock = ecm_http::sys::Socket::from_fd(conn, Duration::from_secs(5));
                let mut data = Vec::new();
                while !data.windows(4).any(|w| w == b"\r\n\r\n") && data.len() < 8192 {
                    let n = std::io::Read::read(&mut sock, &mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n]);
                }
                let asked = String::from_utf8_lossy(&data).split(' ').nth(1).unwrap_or("").to_string();
                let reply = if asked == challenge_path {
                    format!("HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{content}", content.len())
                } else {
                    "HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_string()
                };
                let _ = std::io::Write::write_all(&mut sock, reply.as_bytes());
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        if fds[1].revents != 0 {
            let n = socket::recv(ca, &mut buf, 0);
            if n > 0 {
                answer.extend_from_slice(&buf[..n as usize]);
                continue;
            }
            if n == -2 {
                continue;
            }
            break;
        }
    }
    socket::close(listen);
    socket::close(ca);
    let (head, body, _) = ecm_http::read_response(&mut &answer[..], "POST", &mut Transfer::default()).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&body).into_owned();
    if head.status != 200 {
        return Err(format!("the authority refused: {}", text.trim()));
    }
    Ok(text)
}

fn request(names: &[String], authority: (String, u16), webroot: &str) -> Result<(), String> {
    let (host, port) = authority;
    let parsed: Vec<SubjectName> = names.iter().map(|n| SubjectName::parse(n).ok_or_else(|| format!("'{n}' is not a host name or an address"))).collect::<Result<_, _>>()?;
    let mut seed = [0u8; 32];
    ecm_host_abi::random::fill(&mut seed);
    let public = x509::public_of(&seed);
    println!("Asking {host}:{port} for a certificate for {} ...", names.join(", "));
    let order_req = OrderRequest { public, names: parsed.clone() };
    let r = fetch(&Request::new("POST", &format!("http://{host}:{port}/order")).map_err(|e| e.to_string())?.body("text/plain", order_req.encode().into_bytes()))
        .map_err(|e| format!("{e} (is the authority running? cert trust <authority> sets which one to ask)"))?;
    if r.status != 200 {
        return Err(format!("the authority refused: {}", r.text().trim()));
    }
    let order = Order::parse(&r.text())?;
    let content = challenge_content(&order.token, &public);
    let finalize = finalize_request(&order, &seed);
    let pem = if webroot.is_empty() {
        println!("Answering the authority's check on port 80 ...");
        finalize_standalone(&host, port, &finalize, &order.path(), &content)?
    } else {
        let file = format!("{}{}", webroot.trim_end_matches('/'), order.path());
        write(&file, &content)?;
        println!("Wrote the authority's check to {file}");
        let r = fetch(&Request::new("POST", &format!("http://{host}:{port}/finalize")).map_err(|e| e.to_string())?.body("text/plain", finalize.into_bytes()));
        let _ = std::fs::remove_file(&file);
        let r = r?;
        if r.status != 200 {
            return Err(format!("the authority refused: {}", r.text().trim()));
        }
        r.text()
    };
    // Never install what was not asked for: our key, our names.
    let der = x509::pem_blocks(&pem, x509::CERTIFICATE).into_iter().next().ok_or("the authority's answer holds no certificate")?;
    let info = x509::describe(&der).ok_or("the authority's answer is not a certificate")?;
    if info.names != parsed || !der.windows(32).any(|w| w == public) {
        return Err("the authority sent a certificate for something else; nothing was installed".into());
    }
    write(SERVER_KEY, &x509::pem(x509::PRIVATE_KEY, &x509::pkcs8(&seed)))?;
    write(SERVER_CERT, &pem)?;
    write(REQ_FILE, &Renewal { names: names.to_vec(), authority: format!("{host}:{port}"), webroot: webroot.to_string() }.encode())?;
    println!("Certificate installed: {SERVER_CERT} (key in {SERVER_KEY})");
    println!("  signed by {}, good until {}", info.issuer, x509::format_time(info.not_after));
    println!("Serve HTTPS with:  httpd 443 <dir> --tls &");
    Ok(())
}

fn renew(force: bool) -> Result<(), String> {
    let r = std::fs::read_to_string(REQ_FILE).ok().and_then(|t| Renewal::parse(&t)).ok_or("nothing to renew: get a certificate first with cert request <name>")?;
    let current = std::fs::read_to_string(SERVER_CERT).ok().and_then(|t| x509::pem_blocks(&t, x509::CERTIFICATE).into_iter().next()).and_then(|d| x509::describe(&d));
    if let (Some(info), false) = (&current, force) {
        let left = days_left(info.not_after, now());
        if left >= RENEW_DAYS_LEFT {
            println!("The certificate has {left} days left; nothing to do (cert renew --force asks anyway).");
            return Ok(());
        }
    }
    let authority = authority_address(&r.authority).ok_or("the saved authority address is broken; run cert request again")?;
    request(&r.names, authority, &r.webroot)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    let result = match args.first().map(String::as_str) {
        Some("trust") if !rest.is_empty() => trust(rest),
        Some("untrust") if !rest.is_empty() => untrust(&rest.join(" ")),
        Some("authorities") => {
            authorities();
            Ok(())
        }
        Some("request") => {
            let (mut names, mut ca, mut webroot) = (Vec::new(), None, String::new());
            let mut it = rest.iter();
            let mut bad = None;
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--ca" => ca = it.next().cloned(),
                    "--webroot" => webroot = it.next().cloned().unwrap_or_default(),
                    s if s.starts_with("--") => bad = Some(format!("unknown option {s}")),
                    s => names.push(s.to_string()),
                }
            }
            match (bad, names.is_empty()) {
                (Some(b), _) => Err(b),
                (None, true) => Err("cert request <name>... : name this computer (its host name or address)".into()),
                (None, false) => default_authority(ca.as_deref()).and_then(|a| request(&names, a, &webroot)),
            }
        }
        Some("renew") => renew(rest.iter().any(|a| a == "--force")),
        Some("show") => {
            let file = rest.first().map(String::as_str).unwrap_or(SERVER_CERT);
            std::fs::read_to_string(file).map_err(|_| format!("no certificate in {file} (cert request <name> gets one)")).and_then(|t| describe(&t, file))
        }
        _ => Err(USAGE.to_string()),
    };
    if let Err(e) = result {
        eprintln!("{}{e}", if e.starts_with("Usage") { "" } else { "cert: " });
        std::process::exit(1);
    }
}
