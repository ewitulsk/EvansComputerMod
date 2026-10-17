//! `net` module: TCP/IP, DNS, ICMP and HTTP over the host ABI, with the same
//! functions as Python's `net` (errors are raised as Lua errors).
//!
//!   net.interfaces()                    → { {name=, ip=, prefix_len=, link_up=, ...}, ... }
//!   net.iface_set("eth0", "10.0.0.1/24"), net.iface_up(name), net.iface_down(name)
//!   net.routes()                        → { {destination=, prefix_len=, gateway=, dev=}, ... }
//!   net.route_add("default", "10.0.0.1", "eth0"), net.route_del(dest)
//!   net.dns_set(ip), net.dns_get(), net.resolve(name), net.ping(host) → ms
//!   net.tcp_connect(host, port) → conn, net.tcp_listen(port) → listener,
//!   net.tcp_accept(listener) → conn, net.tcp_send(conn, data) → n,
//!   net.tcp_recv(conn, max) → data, net.tcp_close(conn)
//!   net.http_get(url), net.http_post(url, body) → {status=, body=, headers={...}}

use ecm_host_abi::{net_config, net_ipc};
use mlua::{Lua, Table};

fn err(msg: impl Into<String>) -> mlua::Error {
    mlua::Error::runtime(msg.into())
}

/// "10.0.0.1/24" -> ("10.0.0.1", 24).
fn parse_cidr(cidr: &str) -> Option<(String, u8)> {
    let (ip, prefix) = cidr.split_once('/')?;
    let prefix: u8 = prefix.parse().ok()?;
    let octets: Vec<&str> = ip.split('.').collect();
    if octets.len() != 4 || octets.iter().any(|o| o.parse::<u8>().is_err()) {
        return None;
    }
    Some((ip.to_string(), prefix))
}

fn find_iface_index(name: &str) -> Option<i32> {
    (0..net_config::iface_count()).find(|&i| {
        net_config::iface_info(i)
            .map(|info| parse_json_object(info.trim()).iter().any(|(k, v)| k == "name" && v == name))
            .unwrap_or(false)
    })
}

fn iface_index(name: &str) -> mlua::Result<i32> {
    find_iface_index(name).ok_or_else(|| err(format!("Unknown interface: {name}")))
}

/// Flat JSON object {"key": value, ...} -> (key, value text) pairs; strings
/// unquoted, numbers/booleans/null as written, nested arrays/objects as text.
fn parse_json_object(json: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let t = json.trim();
    if !t.starts_with('{') || !t.ends_with('}') {
        return pairs;
    }
    let inner = &t[1..t.len() - 1];
    let b = inner.as_bytes();
    let n = b.len();
    let mut pos = 0;
    while pos < n {
        while pos < n && matches!(b[pos], b' ' | b',' | b'\n' | b'\r' | b'\t') {
            pos += 1;
        }
        if pos >= n || b[pos] != b'"' {
            break;
        }
        pos += 1;
        let ks = pos;
        while pos < n && b[pos] != b'"' {
            pos += 1;
        }
        if pos >= n {
            break;
        }
        let key = inner[ks..pos].to_string();
        pos += 1;
        while pos < n && matches!(b[pos], b':' | b' ') {
            pos += 1;
        }
        if pos >= n {
            break;
        }
        match b[pos] {
            b'"' => {
                pos += 1;
                let vs = pos;
                while pos < n && b[pos] != b'"' {
                    if b[pos] == b'\\' {
                        pos += 1;
                    }
                    pos += 1;
                }
                pairs.push((key, inner[vs..pos.min(n)].to_string()));
                pos += 1;
            }
            b'[' | b'{' => {
                let (open, close) = if b[pos] == b'[' { (b'[', b']') } else { (b'{', b'}') };
                let vs = pos;
                let mut depth = 1;
                pos += 1;
                while pos < n && depth > 0 {
                    if b[pos] == open {
                        depth += 1;
                    } else if b[pos] == close {
                        depth -= 1;
                    }
                    pos += 1;
                }
                pairs.push((key, inner[vs..pos].to_string()));
            }
            _ => {
                let vs = pos;
                while pos < n && !matches!(b[pos], b',' | b' ' | b'\n' | b'\r' | b'\t') {
                    pos += 1;
                }
                pairs.push((key, inner[vs..pos].to_string()));
            }
        }
    }
    pairs
}

/// Top-level elements of a JSON array's inner text.
fn split_json_array(s: &str) -> Vec<&str> {
    let (mut out, mut depth, mut start, mut in_str, mut esc) = (Vec::new(), 0, 0, false, false);
    for (i, c) in s.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '\\' => esc = true,
            '"' => in_str = !in_str,
            '{' | '[' if !in_str => depth += 1,
            '}' | ']' if !in_str => depth -= 1,
            ',' if !in_str && depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

fn iface_name(idx: i32) -> Option<String> {
    let info = net_config::iface_info(idx)?;
    parse_json_object(info.trim()).into_iter().find(|(k, _)| k == "name").map(|(_, v)| v)
}

fn resolve_host(host: &str) -> mlua::Result<String> {
    if host.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Ok(host.to_string());
    }
    let mut ip = [0u8; 4];
    if net_ipc::dns_resolve(host, &mut ip) < 0 {
        return Err(err(format!("DNS resolution failed for {host}")));
    }
    Ok(format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]))
}

/// A plain HTTP/1.1 request over the computer's TCP stack.
fn http(lua: &Lua, method: &str, url: &str, body: Option<&[u8]>) -> mlua::Result<Table> {
    let rest = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match host_port.split_once(':') {
        Some((h, p)) => (h, p.parse().unwrap_or(80)),
        None => (host_port, 80u16),
    };
    let ip = resolve_host(host)?;
    let conn = net_ipc::tcp_connect(&ip, port, 10000);
    if conn < 0 {
        return Err(err(format!("TCP connect failed to {ip}:{port}")));
    }
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    if net_ipc::tcp_send(conn, req.as_bytes()) < 0 {
        net_ipc::tcp_close(conn);
        return Err(err("Failed to send HTTP request"));
    }
    if let Some(b) = body {
        net_ipc::tcp_send(conn, b);
    }
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = net_ipc::tcp_recv(conn, &mut buf, 5000);
        if n <= 0 {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    net_ipc::tcp_close(conn);

    let split = data.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| err("Invalid HTTP response"))?;
    let head = String::from_utf8_lossy(&data[..split]).into_owned();
    let status: f64 = head.lines().next().and_then(|l| l.split_whitespace().nth(1)).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let headers = lua.create_table()?;
    for line in head.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            headers.set(k.trim().to_lowercase(), v.trim())?;
        }
    }
    let out = lua.create_table()?;
    out.set("status", status)?;
    out.set("body", lua.create_string(&data[split + 4..])?)?;
    out.set("headers", headers)?;
    Ok(out)
}

pub fn module(lua: &Lua) -> mlua::Result<Table> {
    let m = lua.create_table()?;

    m.set("interfaces", lua.create_function(|lua, ()| {
        let out = lua.create_table()?;
        for i in 0..net_config::iface_count() {
            let Some(json) = net_config::iface_info(i) else { continue };
            let t = lua.create_table()?;
            for (k, v) in parse_json_object(json.trim()) {
                match k.as_str() {
                    "prefix_len" | "prefix" => {
                        if let Ok(n) = v.parse::<f64>() {
                            t.set(k, n)?;
                        }
                    }
                    "link_up" => t.set("link_up", v == "true")?,
                    _ => t.set(k, v)?,
                }
            }
            out.raw_set(out.raw_len() + 1, t)?;
        }
        Ok(out)
    })?)?;

    m.set("iface_set", lua.create_function(|_, (name, cidr): (String, String)| {
        let idx = iface_index(&name)?;
        let (ip, prefix) = parse_cidr(&cidr).ok_or_else(|| err("Invalid CIDR (e.g. 10.0.0.1/24)"))?;
        if net_config::iface_configure(idx, &ip, prefix) < 0 {
            return Err(err("Failed to configure interface"));
        }
        Ok(())
    })?)?;
    m.set("iface_up", lua.create_function(|_, name: String| {
        net_config::iface_set_link(iface_index(&name)?, true);
        Ok(())
    })?)?;
    m.set("iface_down", lua.create_function(|_, name: String| {
        net_config::iface_set_link(iface_index(&name)?, false);
        Ok(())
    })?)?;

    m.set("routes", lua.create_function(|lua, ()| {
        let out = lua.create_table()?;
        let json = net_config::route_list().unwrap_or_else(|| "[]".to_string());
        let t = json.trim();
        if !t.starts_with('[') || !t.ends_with(']') {
            return Ok(out);
        }
        let inner = t[1..t.len() - 1].trim();
        if inner.is_empty() {
            return Ok(out);
        }
        for obj in split_json_array(inner) {
            let r = lua.create_table()?;
            for (k, v) in parse_json_object(obj.trim()) {
                match k.as_str() {
                    "prefix" | "prefix_len" => {
                        if let Ok(n) = v.parse::<f64>() {
                            r.set("prefix_len", n)?;
                        }
                    }
                    "dest" | "destination" => r.set("destination", v)?,
                    "iface_idx" => {
                        if let Some(name) = v.parse::<i32>().ok().filter(|&i| i >= 0 && i < net_config::iface_count()).and_then(iface_name) {
                            r.set("dev", name)?;
                        }
                    }
                    _ => r.set(k, v)?,
                }
            }
            out.raw_set(out.raw_len() + 1, r)?;
        }
        Ok(out)
    })?)?;

    m.set("route_add", lua.create_function(|_, (dest, gw, dev): (String, String, String)| {
        let idx = iface_index(&dev)?;
        let (d, p) = if dest == "default" {
            ("0.0.0.0".to_string(), 0)
        } else {
            parse_cidr(&dest).ok_or_else(|| err("Invalid CIDR destination"))?
        };
        if net_config::route_add(&d, p, &gw, idx) < 0 {
            return Err(err("Failed to add route"));
        }
        Ok(())
    })?)?;
    m.set("route_del", lua.create_function(|_, dest: String| {
        if dest == "default" {
            if net_config::route_del("0.0.0.0", 0) < 0 {
                return Err(err("No default route"));
            }
        } else {
            let (d, p) = parse_cidr(&dest).ok_or_else(|| err("Invalid CIDR destination"))?;
            if net_config::route_del(&d, p) < 0 {
                return Err(err("Route not found"));
            }
        }
        Ok(())
    })?)?;

    m.set("dns_set", lua.create_function(|_, ip: String| {
        if net_config::dns_set(&ip) < 0 {
            return Err(err("Failed to set DNS"));
        }
        Ok(())
    })?)?;
    m.set("dns_get", lua.create_function(|_, ()| Ok(net_config::dns_get().unwrap_or_else(|| "0.0.0.0".to_string())))?)?;

    // ICMP echo; round-trip time in ms.
    m.set("ping", lua.create_function(|_, target: String| {
        let rtt = net_config::ping(&target, 2000);
        if rtt < 0 {
            return Err(err(format!("Ping failed for {target}")));
        }
        Ok(rtt)
    })?)?;
    m.set("resolve", lua.create_function(|_, name: String| {
        let mut ip = [0u8; 4];
        if net_ipc::dns_resolve(&name, &mut ip) < 0 {
            return Err(err(format!("DNS resolution failed for {name}")));
        }
        Ok(format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]))
    })?)?;

    // Not available to WASI programs (as in Python).
    m.set("udp_open", lua.create_function(|_, _port: i32| -> mlua::Result<i32> {
        Err(err("UDP not available in WASI programs yet"))
    })?)?;

    m.set("tcp_connect", lua.create_function(|_, (host, port): (String, u16)| {
        let c = net_ipc::tcp_connect(&host, port, 10000);
        if c < 0 {
            return Err(err(format!("TCP connect failed to {host}:{port}")));
        }
        Ok(c)
    })?)?;
    // A wake socket: data arriving on it while the app is suspended wakes the app at
    // once (needs background:refresh). net.set_wake(conn, true); false turns it off.
    m.set("set_wake", lua.create_function(|_, (conn, on): (i32, Option<bool>)| {
        let rc = ecm_host_abi::socket::set_wake(conn, on.unwrap_or(true));
        if rc != 0 {
            return Err(err(format!("set_wake failed on connection {conn} (error {rc})")));
        }
        Ok(())
    })?)?;
    m.set("tcp_listen", lua.create_function(|_, port: u16| {
        let l = net_ipc::tcp_listen(port);
        if l < 0 {
            return Err(err(format!("TCP listen failed on port {port}")));
        }
        Ok(l)
    })?)?;
    m.set("tcp_accept", lua.create_function(|_, listener: i32| {
        let c = net_ipc::tcp_accept(listener, 30000);
        if c < 0 {
            return Err(err("TCP accept failed"));
        }
        Ok(c)
    })?)?;
    m.set("tcp_send", lua.create_function(|_, (conn, data): (i32, mlua::LuaString)| {
        let n = net_ipc::tcp_send(conn, &data.as_bytes());
        if n < 0 {
            return Err(err("TCP send failed"));
        }
        Ok(n)
    })?)?;
    m.set("tcp_recv", lua.create_function(|lua, (conn, max_len): (i32, usize)| {
        let mut buf = vec![0u8; max_len];
        let n = net_ipc::tcp_recv(conn, &mut buf, 10000);
        if n < 0 {
            return Err(err("TCP recv failed"));
        }
        buf.truncate(n as usize);
        lua.create_string(buf)
    })?)?;
    m.set("tcp_close", lua.create_function(|_, conn: i32| {
        net_ipc::tcp_close(conn);
        Ok(())
    })?)?;

    m.set("http_get", lua.create_function(|lua, url: String| http(lua, "GET", &url, None))?)?;
    m.set("http_post", lua.create_function(|lua, (url, body): (String, mlua::LuaString)| {
        http(lua, "POST", &url, Some(&body.as_bytes()))
    })?)?;

    Ok(m)
}
