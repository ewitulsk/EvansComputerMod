use ecm_host_abi::socket::{self, SockAddrIn, AF_INET, SOCK_STREAM, SOL_SOCKET, SO_REUSEADDR};
use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: httpd <port>");
        std::process::exit(1);
    }

    let port: u16 = match args[1].parse() {
        Ok(p) => p,
        Err(_) => { eprintln!("Invalid port number"); std::process::exit(1); }
    };

    // Create TCP socket
    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    if fd < 0 {
        eprintln!("Failed to create socket");
        std::process::exit(1);
    }

    // Set SO_REUSEADDR
    let optval: i32 = 1;
    socket::setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &optval.to_le_bytes());

    // Bind to port
    let addr = SockAddrIn::any(port);
    if socket::bind(fd, &addr) != 0 {
        eprintln!("Failed to bind to port {}", port);
        socket::close(fd);
        std::process::exit(1);
    }

    // Listen
    if socket::listen(fd, 5) != 0 {
        eprintln!("Failed to listen on port {}", port);
        socket::close(fd);
        std::process::exit(1);
    }

    println!("HTTP server listening on port {}. Ctrl+T to stop.", port);

    loop {
        let mut peer_addr = SockAddrIn::default();
        let conn = socket::accept(fd, &mut peer_addr);
        if conn == -2 {
            // Timeout, loop
            continue;
        }
        if conn < 0 {
            eprintln!("Accept error");
            break;
        }
        handle_connection(conn);
    }

    socket::close(fd);
    println!("HTTP server stopped.");
}

fn handle_connection(conn: i32) {
    // Read request
    let mut request_data = Vec::new();
    let mut buf = [0u8; 4096];

    loop {
        let n = socket::recv(conn, &mut buf, 0);
        if n <= 0 { break; }
        request_data.extend_from_slice(&buf[..n as usize]);
        if request_data.windows(4).any(|w| w == b"\r\n\r\n") {
            if let Ok(text) = std::str::from_utf8(&request_data) {
                if let Some(header_end) = text.find("\r\n\r\n") {
                    let headers = &text[..header_end];
                    let body_start = header_end + 4;
                    let mut content_length = 0usize;
                    for line in headers.split("\r\n") {
                        if line.to_lowercase().starts_with("content-length:") {
                            if let Some(val) = line.split(':').nth(1) {
                                content_length = val.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                    let body_received = request_data.len() - body_start;
                    if body_received >= content_length {
                        break;
                    }
                }
            }
        }
        if request_data.len() > 16384 { break; }
    }

    if request_data.is_empty() {
        socket::close(conn);
        return;
    }

    let request_str = String::from_utf8_lossy(&request_data);
    let first_line = request_str.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        send_response(conn, 400, "Bad Request", "text/plain", b"Malformed request");
        return;
    }

    let method = parts[0];
    let path = parts[1];
    println!("{} {}", method, path);

    match method {
        "GET" => handle_get(conn, path),
        "POST" => {
            let body_start = request_str.find("\r\n\r\n").map(|i| i + 4).unwrap_or(request_str.len());
            let body = &request_data[body_start..];
            handle_post(conn, path, body);
        }
        _ => send_response(conn, 400, "Bad Request", "text/plain", b"Method not supported"),
    }
}

fn handle_get(conn: i32, path: &str) {
    if path == "/" {
        let mut body = String::from("<html><head><title>Terminal OS File Server</title></head><body>\n");
        body.push_str("<h1>Files</h1>\n<ul>\n");
        if let Ok(entries) = fs::read_dir(".") {
            let mut items: Vec<(String, bool)> = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                items.push((name, is_dir));
            }
            items.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, is_dir) in &items {
                if *is_dir {
                    body.push_str(&format!("<li><a href=\"/{}\">{}/</a></li>\n", name, name));
                } else {
                    body.push_str(&format!("<li><a href=\"/{}\">{}</a></li>\n", name, name));
                }
            }
        }
        body.push_str("</ul>\n</body></html>");
        send_response(conn, 200, "OK", "text/html", body.as_bytes());
    } else {
        let filename = path.trim_start_matches('/');
        match fs::read(filename) {
            Ok(content) => {
                let content_type = if filename.ends_with(".html") || filename.ends_with(".htm") {
                    "text/html"
                } else if filename.ends_with(".json") {
                    "application/json"
                } else if filename.ends_with(".py") {
                    "text/x-python"
                } else {
                    "text/plain"
                };
                send_response(conn, 200, "OK", content_type, &content);
            }
            Err(_) => send_response(conn, 404, "Not Found", "text/plain", b"File not found"),
        }
    }
}

fn handle_post(conn: i32, path: &str, body: &[u8]) {
    let filename = path.trim_start_matches('/');
    if filename.is_empty() {
        send_response(conn, 400, "Bad Request", "text/plain", b"No filename specified");
        return;
    }
    match fs::write(filename, body) {
        Ok(_) => send_response(conn, 200, "OK", "text/plain", b"OK"),
        Err(_) => send_response(conn, 500, "Internal Server Error", "text/plain", b"Failed to write file"),
    }
}

fn send_response(conn: i32, status: u16, status_text: &str, content_type: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.0 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status, status_text, content_type, body.len()
    );
    socket::send(conn, header.as_bytes(), 0);
    if !body.is_empty() {
        socket::send(conn, body, 0);
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    socket::close(conn);
}
