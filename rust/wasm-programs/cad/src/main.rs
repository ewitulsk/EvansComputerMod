//! `cad` - a certificate authority for the game's networks (docs/TLS.md).
//!
//!   cad init [name...]   make this computer an authority
//!   cad [serve] [port]   answer certificate orders (port 8555)
//!   cad show             print the line other computers trust it by

use ecm_ca::{challenge_content, refuse_name, Authority, OrderRequest, PORT};
use ecm_host_abi::socket::{self, SockAddrIn, AF_INET, SOCK_STREAM, SOL_SOCKET, SO_REUSEADDR};
use ecm_http::sys::Socket;
use ecm_http::{Request as Fetch, Transfer};
use ecm_tls::x509::SubjectName;
use httpd::{response, Request};
use std::io::{Read, Write};
use std::time::Duration;

const KEY_FILE: &str = "/etc/cad/ca.key";
const NAME_FILE: &str = "/etc/cad/ca.name";
const ROOT_FILE: &str = "/etc/cad/ca.crt";
const LOG_FILE: &str = "/var/lib/cad/issued.log";

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    ecm_host_abi::random::fill(&mut b);
    b
}

fn load() -> Result<Authority, String> {
    let key = std::fs::read_to_string(KEY_FILE).map_err(|_| format!("this computer is not an authority yet ({KEY_FILE} is missing); run: cad init <name>"))?;
    let seed: [u8; 32] = ecm_tls::unhex(key.trim()).and_then(|v| v.try_into().ok()).ok_or_else(|| format!("{KEY_FILE} is not a key (64 hex digits)"))?;
    let name = std::fs::read_to_string(NAME_FILE).map(|s| s.trim().to_string()).unwrap_or_default();
    Ok(Authority::new(seed, if name.is_empty() { "Certificate Authority" } else { &name }))
}

fn init(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 || name.chars().any(|c| c.is_control()) {
        return Err("give the authority a name of at most 64 characters: cad init Evan's CA".into());
    }
    let w = |p: &str, d: &str| {
        if let Some(dir) = std::path::Path::new(p).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(p, d).map_err(|e| format!("cannot write {p}: {e}"))
    };
    if std::fs::metadata(KEY_FILE).is_err() {
        w(KEY_FILE, &format!("{}\n", ecm_tls::hex(&random::<32>())))?;
    } else {
        println!("Keeping the key in {KEY_FILE}.");
    }
    w(NAME_FILE, &format!("{name}\n"))?;
    let ca = load()?;
    w(ROOT_FILE, &ca.root_pem())?;
    println!("This computer is the authority \"{}\".", ca.common_name);
    println!("Other computers trust it with:  cert trust <this computer's address>");
    println!("or by saving this line as {}/{}:", ecm_tls::CERTS_DIR, ecm_ca::anchor_file(&ca.common_name));
    print!("  {}", ca.anchor_line());
    println!("Start it with:  cad &   (or add the line to /services.cfg)");
    Ok(())
}

fn text(status: u16, reason: &str, body: &str) -> Vec<u8> {
    response(status, reason, &[("Content-Type".into(), "text/plain".into())], body.as_bytes())
}

/// Fetch the challenge from one name; `Err` says why the name is not proven.
fn prove(name: &SubjectName, token: &str, want: &str) -> Result<(), String> {
    let resolved = match name {
        SubjectName::Dns(d) => ecm_http::sys::resolve(d).ok(),
        SubjectName::Ip(_) => None,
    };
    if let Some(why) = refuse_name(name, resolved) {
        return Err(why);
    }
    let url = format!("http://{name}{}{token}", ecm_ca::CHALLENGE_PREFIX);
    // The check is plain http on port 80: this program has no TLS in it.
    let mut client = ecm_http::sys::plain_client("cad/1", Duration::from_secs(8));
    let req = Fetch::get(&url).map_err(|e| e.to_string())?;
    let r = client.exchange(&req, &mut Transfer { max_body: 4096, ..Transfer::default() }).map_err(|e| format!("{name}: {e} (the authority asks port 80 of that name for the challenge)"))?;
    if r.status != 200 {
        return Err(format!("{name}: the challenge page answered HTTP {} (is a web server serving {}?)", r.status, ecm_ca::CHALLENGE_PREFIX));
    }
    if r.text().trim() != want {
        return Err(format!("{name}: another computer answers at that name (the challenge did not match)"));
    }
    Ok(())
}

fn handle(ca: &mut Authority, req: &Request, peer: &str) -> Vec<u8> {
    let body = String::from_utf8_lossy(&req.body).into_owned();
    match (req.method.as_str(), req.path()) {
        ("GET", "/ca") => text(200, "OK", &ca.anchor_line()),
        ("GET", "/ca.crt") => text(200, "OK", &ca.root_pem()),
        ("GET", "/") => text(200, "OK", &format!("Certificate authority \"{}\".\nGet a certificate with: cert request <name>\n", ca.common_name)),
        ("POST", "/order") => match OrderRequest::parse(&body) {
            Ok(r) => {
                let names: Vec<String> = r.names.iter().map(|n| n.to_string()).collect();
                println!("{peer}: order for {}", names.join(", "));
                text(200, "OK", &ca.open(r, now(), &random::<32>()).encode())
            }
            Err(e) => text(400, "Bad Request", &format!("{e}\n")),
        },
        ("POST", "/finalize") => {
            let pending = match ca.claim(&body, now()) {
                Ok(p) => p,
                Err(e) => return text(403, "Forbidden", &format!("{e}\n")),
            };
            let want = challenge_content(&pending.order.token, &pending.request.public);
            for n in &pending.request.names {
                if let Err(why) = prove(n, &pending.order.token, &want) {
                    println!("{peer}: refused: {why}");
                    return text(403, "Forbidden", &format!("{why}\n"));
                }
            }
            let issued = ca.issue(&pending, now(), &random::<16>());
            let names: Vec<String> = issued.names.iter().map(|n| n.to_string()).collect();
            println!("{peer}: signed {} until {}", names.join(", "), ecm_tls::x509::format_time(issued.not_after));
            let _ = std::fs::create_dir_all("/var/lib/cad");
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(LOG_FILE) {
                let _ = writeln!(f, "{} {} {} {}", now(), issued.serial, peer, names.join(","));
            }
            text(200, "OK", &issued.pem)
        }
        _ => text(404, "Not Found", "cad answers GET /ca, POST /order and POST /finalize\n"),
    }
}

fn read_request(conn: &mut Socket) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        let n = conn.read(&mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
            let length = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
            if data.len() - end - 4 >= length {
                break;
            }
        }
        if data.len() > 16 * 1024 {
            break;
        }
    }
    data
}

fn serve(port: u16) -> Result<(), String> {
    let mut ca = load()?;
    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    if fd < 0 {
        return Err("cannot open a socket".into());
    }
    socket::setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &1i32.to_le_bytes());
    if socket::bind(fd, &SockAddrIn::any(port)) != 0 || socket::listen(fd, 5) != 0 {
        socket::close(fd);
        return Err(format!("cannot listen on port {port} (is cad already running?)"));
    }
    println!("Certificate authority \"{}\" listening on port {port}. Ctrl+T to stop.", ca.common_name);
    loop {
        let mut peer = SockAddrIn::default();
        let conn = socket::accept(fd, &mut peer);
        if conn == -2 {
            continue;
        }
        if conn < 0 {
            break;
        }
        let ip = peer.ip();
        let who = format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
        let mut sock = Socket::from_fd(conn, Duration::from_secs(10));
        let raw = read_request(&mut sock);
        let answer = match Request::parse(&raw) {
            Some(req) => handle(&mut ca, &req, &who),
            None if raw.is_empty() => continue,
            None => text(400, "Bad Request", "not an HTTP request\n"),
        };
        let _ = sock.write_all(&answer);
        std::thread::sleep(Duration::from_millis(50));
    }
    socket::close(fd);
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("init") => init(args[1..].join(" ").trim()),
        Some("show") => load().map(|ca| {
            println!("Authority \"{}\"", ca.common_name);
            print!("{}", ca.anchor_line());
        }),
        Some("serve") | None => match args.get(1).map(|p| p.parse::<u16>()) {
            Some(Err(_)) => Err(format!("bad port {}", args[1])),
            Some(Ok(p)) => serve(p),
            None => serve(PORT),
        },
        Some(p) if p.parse::<u16>().is_ok() => serve(p.parse().unwrap_or(PORT)),
        Some(_) => Err("Usage: cad init <name> | cad [serve] [port] | cad show".into()),
    };
    if let Err(e) = r {
        eprintln!("cad: {e}");
        std::process::exit(1);
    }
}
