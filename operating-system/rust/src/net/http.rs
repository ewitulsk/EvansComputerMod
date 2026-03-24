//! HTTP/1.0 request parser, response builder, and client/server functions.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

extern crate alloc;

use super::types::{Ipv4Addr, SocketAddr, NetError};
use super::NetStack;

/// Parsed HTTP request.
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub version: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// Parse an HTTP request from raw bytes.
    pub fn parse(data: &[u8]) -> Option<Self> {
        let text = core::str::from_utf8(data).ok()?;

        // Find end of headers
        let header_end = text.find("\r\n\r\n")?;
        let header_section = &text[..header_end];
        let body_start = header_end + 4;

        let mut lines = header_section.split("\r\n");

        // Request line: "GET /path HTTP/1.0"
        let request_line = lines.next()?;
        let mut parts = request_line.splitn(3, ' ');
        let method = parts.next()?.to_string();
        let path = parts.next()?.to_string();
        let version = parts.next().unwrap_or("HTTP/1.0").to_string();

        // Headers
        let mut headers = Vec::new();
        for line in lines {
            if let Some(colon_pos) = line.find(':') {
                let key = line[..colon_pos].trim().to_string();
                let value = line[colon_pos + 1..].trim().to_string();
                headers.push((key, value));
            }
        }

        // Body
        let body = if body_start < data.len() {
            data[body_start..].to_vec()
        } else {
            Vec::new()
        };

        Some(HttpRequest {
            method,
            path,
            version,
            headers,
            body,
        })
    }

    /// Get a header value by name (case-insensitive).
    pub fn get_header(&self, name: &str) -> Option<&str> {
        let lower = name.to_lowercase();
        for (k, v) in &self.headers {
            if k.to_lowercase() == lower {
                return Some(v.as_str());
            }
        }
        None
    }

    /// Serialize an HTTP request to wire format.
    pub fn serialize(
        method: &str,
        host: &str,
        port: u16,
        path: &str,
        body: Option<&[u8]>,
        extra_headers: &[(&str, &str)],
    ) -> Vec<u8> {
        let mut req = format!("{} {} HTTP/1.0\r\n", method, path);
        if port == 80 {
            req.push_str(&format!("Host: {}\r\n", host));
        } else {
            req.push_str(&format!("Host: {}:{}\r\n", host, port));
        }
        req.push_str("Connection: close\r\n");

        if let Some(b) = body {
            req.push_str(&format!("Content-Length: {}\r\n", b.len()));
        }

        for (k, v) in extra_headers {
            req.push_str(&format!("{}: {}\r\n", k, v));
        }

        req.push_str("\r\n");

        let mut result = req.into_bytes();
        if let Some(b) = body {
            result.extend_from_slice(b);
        }
        result
    }
}

/// HTTP response.
pub struct HttpResponse {
    pub status_code: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(status: u16, text: &str) -> Self {
        HttpResponse {
            status_code: status,
            status_text: text.to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers.push((key.to_string(), value.to_string()));
        self
    }

    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    pub fn ok(body: &str, content_type: &str) -> Self {
        let body_bytes = body.as_bytes().to_vec();
        HttpResponse::new(200, "OK")
            .with_header("Content-Type", content_type)
            .with_header("Content-Length", &format!("{}", body_bytes.len()))
            .with_body(body_bytes)
    }

    pub fn not_found() -> Self {
        let body = b"404 Not Found".to_vec();
        HttpResponse::new(404, "Not Found")
            .with_header("Content-Type", "text/plain")
            .with_header("Content-Length", &format!("{}", body.len()))
            .with_body(body)
    }

    pub fn bad_request(msg: &str) -> Self {
        let body = msg.as_bytes().to_vec();
        HttpResponse::new(400, "Bad Request")
            .with_header("Content-Type", "text/plain")
            .with_header("Content-Length", &format!("{}", body.len()))
            .with_body(body)
    }

    /// Serialize to wire format.
    pub fn serialize(&self) -> Vec<u8> {
        let mut resp = format!("HTTP/1.0 {} {}\r\n", self.status_code, self.status_text);
        resp.push_str("Server: TerminalOS/1.0\r\n");

        for (k, v) in &self.headers {
            resp.push_str(&format!("{}: {}\r\n", k, v));
        }

        // Ensure Content-Length is set
        let has_cl = self.headers.iter().any(|(k, _)| k.to_lowercase() == "content-length");
        if !has_cl {
            resp.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
        }

        resp.push_str("\r\n");

        let mut result = resp.into_bytes();
        result.extend_from_slice(&self.body);
        result
    }

    /// Parse an HTTP response from raw bytes.
    pub fn parse(data: &[u8]) -> Option<Self> {
        let text = core::str::from_utf8(data).ok()?;
        let header_end = text.find("\r\n\r\n")?;
        let header_section = &text[..header_end];
        let body_start = header_end + 4;

        let mut lines = header_section.split("\r\n");

        // Status line: "HTTP/1.0 200 OK"
        let status_line = lines.next()?;
        let mut parts = status_line.splitn(3, ' ');
        let _version = parts.next()?;
        let status_code: u16 = parts.next()?.parse().ok()?;
        let status_text = parts.next().unwrap_or("").to_string();

        let mut headers = Vec::new();
        for line in lines {
            if let Some(colon_pos) = line.find(':') {
                let key = line[..colon_pos].trim().to_string();
                let value = line[colon_pos + 1..].trim().to_string();
                headers.push((key, value));
            }
        }

        let body = if body_start < data.len() {
            data[body_start..].to_vec()
        } else {
            Vec::new()
        };

        Some(HttpResponse {
            status_code,
            status_text,
            headers,
            body,
        })
    }

    /// Get a header value by name (case-insensitive).
    pub fn get_header(&self, name: &str) -> Option<&str> {
        let lower = name.to_lowercase();
        for (k, v) in &self.headers {
            if k.to_lowercase() == lower {
                return Some(v.as_str());
            }
        }
        None
    }
}

/// Parse a URL into (host, port, path).
/// Supports: "http://host:port/path", "http://host/path", "host:port/path", "host/path"
pub fn parse_url(url: &str) -> Option<(String, u16, String)> {
    let url = if url.starts_with("http://") {
        &url[7..]
    } else {
        url
    };

    // Split host:port from path
    let (host_port, path) = if let Some(slash_pos) = url.find('/') {
        (&url[..slash_pos], &url[slash_pos..])
    } else {
        (url, "/")
    };

    // Split host from port
    let (host, port) = if let Some(colon_pos) = host_port.find(':') {
        let port: u16 = host_port[colon_pos + 1..].parse().ok()?;
        (&host_port[..colon_pos], port)
    } else {
        (host_port, 80u16)
    };

    Some((host.to_string(), port, path.to_string()))
}

/// Perform an HTTP request. Returns the response.
pub fn http_request(
    stack: &mut NetStack,
    method: &str,
    host: &str,
    port: u16,
    path: &str,
    body: Option<&[u8]>,
    extra_headers: &[(&str, &str)],
) -> Result<HttpResponse, NetError> {
    // Resolve host IP
    let ip = if let Some(ip) = Ipv4Addr::parse(host) {
        ip
    } else {
        stack.dns_resolve(host, 5000)?
    };

    let remote = SocketAddr { ip, port };

    // TCP connect
    let conn = stack.tcp_connect(remote, 10000)?;

    // Build and send request
    let req = HttpRequest::serialize(method, host, port, path, body, extra_headers);
    stack.tcp_send(conn, &req)?;

    // Read response (accumulate until connection closes or Content-Length reached)
    let mut response_data = Vec::new();
    let mut buf = [0u8; 1460];

    loop {
        match stack.tcp_recv(conn, &mut buf, 10000) {
            Ok(0) => break, // EOF
            Ok(n) => {
                response_data.extend_from_slice(&buf[..n]);

                // Check if we have complete headers + body
                if let Some(resp) = try_parse_complete_response(&response_data) {
                    stack.tcp_close(conn);
                    return Ok(resp);
                }

                // Safety limit
                if response_data.len() > 32768 {
                    break;
                }
            }
            Err(NetError::TimedOut) => break,
            Err(NetError::NotConnected) => break,
            Err(e) => {
                stack.tcp_close(conn);
                return Err(e);
            }
        }
    }

    stack.tcp_close(conn);

    // Try to parse whatever we got
    HttpResponse::parse(&response_data).ok_or(NetError::InvalidPacket)
}

/// Try to parse a complete HTTP response (headers + full body per Content-Length).
fn try_parse_complete_response(data: &[u8]) -> Option<HttpResponse> {
    let text = core::str::from_utf8(data).ok()?;
    let header_end = text.find("\r\n\r\n")?;
    let body_start = header_end + 4;

    // Parse Content-Length from headers
    let header_section = &text[..header_end];
    let mut content_length: Option<usize> = None;
    for line in header_section.split("\r\n").skip(1) {
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim();
            let value = line[colon + 1..].trim();
            if key.eq_ignore_ascii_case("content-length") {
                content_length = value.parse().ok();
            }
        }
    }

    match content_length {
        Some(cl) => {
            if data.len() >= body_start + cl {
                // We have the full response
                HttpResponse::parse(&data[..body_start + cl])
            } else {
                None // need more data
            }
        }
        None => {
            // No Content-Length — can't determine completeness from headers alone.
            // Return None so we keep reading until EOF.
            None
        }
    }
}
