//! `httpd` — an HTTP/1.0 server: static files, virtual hosts and CGI scripts
//! (see lib.rs for the command line and the CGI protocol).
//!
//!   httpd <port> [dir] [--cgi <prefix>]

use ecm_host_abi::ipc;
use ecm_host_abi::socket::{self, SockAddrIn, AF_INET, SOCK_STREAM, SOL_SOCKET, SO_REUSEADDR};
use httpd::{cgi_command, cgi_input, cgi_response, content_type, local, parse_args, response, site_root, Config, Request, CGI_TIMEOUT_MS};
use std::fs;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match parse_args(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("httpd: {e}");
            eprintln!("Usage: httpd <port> [dir] [--cgi <prefix>]   (dir: the folder to serve, default the computer's root)");
            std::process::exit(1);
        }
    };
    if !cfg.root.is_empty() && !std::path::Path::new(&cfg.root).is_dir() {
        eprintln!("httpd: {} is not a folder", cfg.root);
        std::process::exit(1);
    }
    let port = cfg.port;

    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    if fd < 0 {
        eprintln!("Failed to create socket");
        std::process::exit(1);
    }
    let optval: i32 = 1;
    socket::setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &optval.to_le_bytes());
    let addr = SockAddrIn::any(port);
    if socket::bind(fd, &addr) != 0 {
        eprintln!("Failed to bind to port {}", port);
        socket::close(fd);
        std::process::exit(1);
    }
    if socket::listen(fd, 5) != 0 {
        eprintln!("Failed to listen on port {}", port);
        socket::close(fd);
        std::process::exit(1);
    }

    let mut banner = format!("HTTP server listening on port {port}");
    if !cfg.root.is_empty() {
        banner.push_str(&format!(", serving {}", cfg.root));
    }
    if let Some(c) = &cfg.cgi {
        banner.push_str(&format!(", CGI under {c}"));
    }
    println!("{banner}. Ctrl+T to stop.");

    loop {
        let mut peer = SockAddrIn::default();
        let conn = socket::accept(fd, &mut peer);
        if conn == -2 {
            continue;
        }
        if conn < 0 {
            eprintln!("Accept error");
            break;
        }
        handle_connection(&cfg, conn, &peer.to_string());
    }

    socket::close(fd);
    println!("HTTP server stopped.");
}

fn read_request(conn: i32) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = socket::recv(conn, &mut buf, 0);
        if n <= 0 {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if data.len() - end - 4 >= length {
                break;
            }
        }
        if data.len() > 65536 + 8192 {
            break;
        }
    }
    data
}

fn send(conn: i32, bytes: &[u8]) {
    let mut rest = bytes;
    while !rest.is_empty() {
        let n = socket::send(conn, &rest[..rest.len().min(4096)], 0);
        if n <= 0 {
            break;
        }
        rest = &rest[n as usize..];
    }
    std::thread::sleep(Duration::from_millis(50));
    socket::close(conn);
}

fn text(conn: i32, status: u16, reason: &str, body: &str) {
    send(conn, &response(status, reason, &[("Content-Type".into(), "text/plain".into())], body.as_bytes()));
}

fn handle_connection(cfg: &Config, conn: i32, peer: &str) {
    let raw = read_request(conn);
    if raw.is_empty() {
        socket::close(conn);
        return;
    }
    let Some(req) = Request::parse(&raw) else {
        text(conn, 400, "Bad Request", "Malformed request");
        return;
    };
    println!("{} {}", req.method, req.target);
    let root = site_root(&cfg.root, req.header("host"), |d| std::path::Path::new(d).is_dir());
    let path = req.path().to_string();
    if let Some(prefix) = &cfg.cgi {
        if path == *prefix || path.starts_with(&format!("{prefix}/")) {
            run_cgi(conn, &root, &req, peer);
            return;
        }
    }
    match req.method.as_str() {
        "GET" | "HEAD" => get(conn, &root, &path),
        "POST" => post(conn, &root, &path, &req.body),
        _ => text(conn, 405, "Method Not Allowed", "Method not supported"),
    }
}

fn get(conn: i32, root: &str, path: &str) {
    let Some(file) = local(root, path) else {
        text(conn, 404, "Not Found", "File not found");
        return;
    };
    let is_dir = std::path::Path::new(&file).is_dir();
    if is_dir {
        let index = format!("{}/index.html", file.trim_end_matches('/'));
        if let Ok(page) = fs::read(&index) {
            send(conn, &response(200, "OK", &[("Content-Type".into(), "text/html".into())], &page));
            return;
        }
        // A listing of the folder.
        let shown = if path.ends_with('/') { path.to_string() } else { format!("{path}/") };
        let mut body = format!("<html><head><title>Terminal OS File Server</title></head><body>\n<h1>Files</h1>\n<p>{shown}</p>\n<ul>\n");
        if let Ok(entries) = fs::read_dir(&file) {
            let mut items: Vec<(String, bool)> = entries
                .flatten()
                .map(|e| (e.file_name().to_string_lossy().to_string(), e.file_type().map(|t| t.is_dir()).unwrap_or(false)))
                .collect();
            items.sort();
            for (name, dir) in items {
                let slash = if dir { "/" } else { "" };
                body.push_str(&format!("<li><a href=\"{shown}{name}{slash}\">{name}{slash}</a></li>\n"));
            }
        }
        body.push_str("</ul>\n</body></html>");
        send(conn, &response(200, "OK", &[("Content-Type".into(), "text/html".into())], body.as_bytes()));
        return;
    }
    match fs::read(&file) {
        Ok(content) => send(conn, &response(200, "OK", &[("Content-Type".into(), content_type(&file).into())], &content)),
        Err(_) => text(conn, 404, "Not Found", "File not found"),
    }
}

fn post(conn: i32, root: &str, path: &str, body: &[u8]) {
    let Some(file) = local(root, path).filter(|f| !f.ends_with('/') && f != root) else {
        text(conn, 400, "Bad Request", "No filename specified");
        return;
    };
    match fs::write(&file, body) {
        Ok(_) => text(conn, 200, "OK", "OK"),
        Err(_) => text(conn, 500, "Internal Server Error", "Failed to write file"),
    }
}

/// Run the CGI script the request names and send what it printed.
fn run_cgi(conn: i32, root: &str, req: &Request, peer: &str) {
    let Some(script) = local(root, req.path()) else {
        text(conn, 404, "Not Found", "File not found");
        return;
    };
    if !std::path::Path::new(&script).is_file() {
        text(conn, 404, "Not Found", "File not found");
        return;
    }
    let Some(cmd) = cgi_command(&script) else {
        text(conn, 403, "Forbidden", "Not a script (.py or .lua)");
        return;
    };
    let id = ipc::spawn_command("httpd", &cmd);
    if id < 0 {
        text(conn, 503, "Service Unavailable", "Can't start the script");
        return;
    }
    // A wide terminal, so long lines aren't wrapped.
    ipc::session_resize(id, 1000, 200);
    ipc::session_write(id, &cgi_input(req, peer.split(':').next().unwrap_or("")));
    let started = Instant::now();
    let mut out = Vec::new();
    let mut buf = vec![0u8; 8192];
    loop {
        let n = ipc::session_read_blocking(id, &mut buf, 200);
        if n > 0 {
            out.extend_from_slice(&buf[..n as usize]);
            continue;
        }
        if ipc::session_status(id) != 0 {
            // Exited: take what is left.
            loop {
                let n = ipc::session_read(id, &mut buf);
                if n <= 0 {
                    break;
                }
                out.extend_from_slice(&buf[..n as usize]);
            }
            break;
        }
        if started.elapsed() > Duration::from_millis(CGI_TIMEOUT_MS) {
            ipc::session_close(id);
            text(conn, 504, "Gateway Timeout", "The script took too long");
            return;
        }
    }
    ipc::session_close(id);
    let (status, reason, headers, body) = cgi_response(&String::from_utf8_lossy(&out));
    println!("  {} -> {status}", req.path());
    send(conn, &response(status, &reason, &headers, &body));
}
