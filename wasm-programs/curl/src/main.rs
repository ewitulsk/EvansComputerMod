extern crate ecm_host_abi;
use ecm_host_abi::net_ipc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: curl [-v] [-X METHOD] [-d DATA] [-H HEADER] <url>");
        std::process::exit(1);
    }

    let mut verbose = false;
    let mut method = "GET".to_string();
    let mut body: Option<String> = None;
    let mut extra_headers: Vec<(String, String)> = Vec::new();
    let mut url: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-v" => verbose = true,
            "-X" => { i += 1; if i < args.len() { method = args[i].clone(); } }
            "-d" => {
                i += 1;
                if i < args.len() {
                    body = Some(args[i].clone());
                    if method == "GET" { method = "POST".to_string(); }
                }
            }
            "-H" => {
                i += 1;
                if i < args.len() {
                    if let Some(colon) = args[i].find(':') {
                        let key = args[i][..colon].trim().to_string();
                        let value = args[i][colon + 1..].trim().to_string();
                        extra_headers.push((key, value));
                    }
                }
            }
            _ => { url = Some(args[i].clone()); }
        }
        i += 1;
    }

    let url = match url {
        Some(u) => u,
        None => { eprintln!("No URL specified"); std::process::exit(1); }
    };

    let (host, port, path) = match parse_url(&url) {
        Some(v) => v,
        None => { eprintln!("Invalid URL"); std::process::exit(1); }
    };

    // Resolve hostname
    let mut ip_buf = [0u8; 4];
    let ip_str;

    // Check if host is already an IP
    let octets: Vec<&str> = host.split('.').collect();
    if octets.len() == 4 && octets.iter().all(|o| o.parse::<u8>().is_ok()) {
        ip_str = host.clone();
    } else {
        let res = net_ipc::dns_resolve(&host, &mut ip_buf);
        if res != 0 {
            eprintln!("curl: Could not resolve host: {}", host);
            std::process::exit(1);
        }
        ip_str = format!("{}.{}.{}.{}", ip_buf[0], ip_buf[1], ip_buf[2], ip_buf[3]);
    }

    // Connect
    let conn = net_ipc::tcp_connect(&ip_str, port, 5000);
    if conn < 0 {
        eprintln!("curl: Failed to connect to {}:{}", host, port);
        std::process::exit(1);
    }

    // Build HTTP request
    let mut request = format!("{} {} HTTP/1.0\r\nHost: {}\r\n", method, path, host);
    for (k, v) in &extra_headers {
        request.push_str(&format!("{}: {}\r\n", k, v));
    }
    if let Some(ref b) = body {
        request.push_str(&format!("Content-Length: {}\r\n", b.len()));
        if !extra_headers.iter().any(|(k, _)| k.to_lowercase() == "content-type") {
            request.push_str("Content-Type: application/x-www-form-urlencoded\r\n");
        }
    }
    request.push_str("Connection: close\r\n\r\n");
    if let Some(ref b) = body {
        request.push_str(b);
    }

    if verbose {
        println!("> {} {} HTTP/1.0", method, path);
        println!("> Host: {}:{}", host, port);
        println!(">");
    }

    // Send
    net_ipc::tcp_send(conn, request.as_bytes());

    // Receive
    let mut response_data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = net_ipc::tcp_recv(conn, &mut buf, 5000);
        if n <= 0 { break; }
        response_data.extend_from_slice(&buf[..n as usize]);
    }
    net_ipc::tcp_close(conn);

    // Parse response
    let response_str = String::from_utf8_lossy(&response_data);
    if let Some(header_end) = response_str.find("\r\n\r\n") {
        let headers = &response_str[..header_end];
        let body_str = &response_str[header_end + 4..];

        if verbose {
            for line in headers.lines() {
                println!("< {}", line);
            }
            println!("<");
        }

        print!("{}", body_str);
        if !body_str.ends_with('\n') {
            println!();
        }
    } else {
        // No headers found, print raw
        print!("{}", response_str);
    }
}

fn parse_url(url: &str) -> Option<(String, u16, String)> {
    let url = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = if let Some(slash) = url.find('/') {
        (&url[..slash], format!("/{}", &url[slash + 1..]))
    } else {
        (url, "/".to_string())
    };

    let (host, port) = if let Some(colon) = host_port.find(':') {
        let h = &host_port[..colon];
        let p: u16 = host_port[colon + 1..].parse().ok()?;
        (h.to_string(), p)
    } else {
        (host_port.to_string(), 80)
    };

    Some((host, port, path))
}
