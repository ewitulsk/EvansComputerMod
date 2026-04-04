//! SSH client — standalone WASI program.
//!
//! Uses the ecm-net networking stack directly with raw Ethernet frame host
//! functions. Connects to a remote SSH server, authenticates, opens a session
//! channel, and relays between local stdin/stdout and the remote shell.

use std::io::{self, Read, Write};

use ecm_ssh_protocol::{packet, transport::SshTransport, kex, channel};
use ecm_ssh_crypto as crypto;
use ecm_net::NetStack;
use ecm_net::types::{Ipv4Addr, NetError};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: ssh [user[:password]@]host[:port]");
        std::process::exit(1);
    }

    let (username, password, host, port) = parse_target(&args[1]);

    // Initialize the networking stack
    NetStack::init();
    let stack = match NetStack::get() {
        Some(s) => s,
        None => {
            eprintln!("ssh: failed to initialize network stack");
            std::process::exit(1);
        }
    };

    // Configure network from environment
    let ip_str = std::env::var("NET_IP").unwrap_or_else(|_| "10.0.0.1/24".to_string());
    let gw_str = std::env::var("NET_GATEWAY").unwrap_or_else(|_| "10.0.0.254".to_string());
    let dns_str = std::env::var("NET_DNS").unwrap_or_else(|_| "10.0.0.254".to_string());

    if let Some((ip, prefix)) = parse_cidr(&ip_str) {
        stack.configure_iface(0, ip, prefix);
    }
    if let Some(gw) = parse_ipv4(&gw_str) {
        let _ = stack.routing.add_route(Ipv4Addr::ZERO, 0, gw, 0);
    }
    if let Some(dns) = parse_ipv4(&dns_str) {
        stack.dns_server = dns;
    }

    eprintln!("ssh: connecting to {}@{}:{}", username, host, port);

    // Resolve hostname
    let remote_ip = if host.chars().all(|c| c.is_ascii_digit() || c == '.') {
        match parse_ipv4(&host) {
            Some(ip) => ip,
            None => {
                eprintln!("ssh: invalid IP address");
                std::process::exit(1);
            }
        }
    } else {
        match stack.dns_resolve(&host, 5000) {
            Ok(ip) => ip,
            Err(_) => {
                eprintln!("ssh: failed to resolve {}", host);
                std::process::exit(1);
            }
        }
    };

    // Connect
    let remote_addr = ecm_net::types::SocketAddr { ip: remote_ip, port };
    let conn = match stack.tcp_connect(remote_addr, 10_000) {
        Ok(idx) => idx,
        Err(e) => {
            eprintln!("ssh: connection failed: {:?}", e);
            std::process::exit(1);
        }
    };

    // SSH handshake
    let mut transport = SshTransport::new(false);

    // 1. Send version
    let version_line = transport.version_line();
    let _ = stack.tcp_send(conn, &version_line);

    // 2. Read server version
    let mut ver_buf: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match stack.tcp_recv(conn, &mut buf, 10_000) {
            Ok(0) | Err(_) => {
                eprintln!("ssh: timeout waiting for server version");
                std::process::exit(1);
            }
            Ok(n) => {
                ver_buf.extend_from_slice(&buf[..n]);
                if let Some(pos) = ver_buf.iter().position(|&b| b == b'\n') {
                    let line = std::str::from_utf8(&ver_buf[..pos]).unwrap_or("");
                    if !transport.set_peer_version(line) {
                        eprintln!("ssh: invalid server version");
                        std::process::exit(1);
                    }
                    let remaining = ver_buf[pos + 1..].to_vec();
                    if !remaining.is_empty() {
                        transport.recv_buffer.extend_from_slice(&remaining);
                    }
                    break;
                }
            }
        }
    }

    // 3. Send KEXINIT
    let our_kexinit_payload = kex::build_kexinit();
    let kexinit_pkt = transport.build_kexinit_packet();
    let _ = stack.tcp_send(conn, &kexinit_pkt);

    // 4. Read server KEXINIT
    let server_kexinit = read_next_payload(stack, conn, &mut transport);
    if server_kexinit.is_empty() || server_kexinit[0] != packet::msg::KEXINIT {
        eprintln!("ssh: expected KEXINIT");
        std::process::exit(1);
    }

    // 5. Generate ephemeral keypair and send KEX_ECDH_INIT
    let (client_epub, client_secret) = crypto::x25519_generate_keypair();
    let client_epub_bytes = client_epub.to_bytes();
    let mut kex_init_payload = Vec::new();
    kex_init_payload.push(packet::msg::KEX_ECDH_INIT);
    kex_init_payload.extend_from_slice(&packet::encode_string(&client_epub_bytes));
    let pkt = transport.encode_packet(&kex_init_payload);
    let _ = stack.tcp_send(conn, &pkt);

    // 6. Read KEX_ECDH_REPLY
    let kex_reply = read_next_payload(stack, conn, &mut transport);
    if kex_reply.is_empty() || kex_reply[0] != packet::msg::KEX_ECDH_REPLY {
        eprintln!("ssh: expected KEX_ECDH_REPLY");
        std::process::exit(1);
    }

    let mut offset = 1;
    let (host_key_blob, new_off) = packet::decode_string(&kex_reply, offset).unwrap();
    offset = new_off;
    let (server_epub_bytes, new_off) = packet::decode_string(&kex_reply, offset).unwrap();
    offset = new_off;
    let (_sig_blob, _) = packet::decode_string(&kex_reply, offset).unwrap();

    let mut server_epub = [0u8; 32];
    server_epub.copy_from_slice(server_epub_bytes);

    let their_pub = crypto::X25519Public::from(server_epub);
    let shared_secret = crypto::x25519_diffie_hellman(&client_secret, &their_pub);

    let exchange_hash = kex::compute_exchange_hash(
        &transport.our_version,
        &transport.peer_version,
        &our_kexinit_payload,
        &server_kexinit,
        host_key_blob,
        &client_epub_bytes,
        &server_epub,
        &shared_secret,
    );

    // 7. Read NEWKEYS from server
    let newkeys = read_next_payload(stack, conn, &mut transport);
    if newkeys.is_empty() || newkeys[0] != packet::msg::NEWKEYS {
        eprintln!("ssh: expected NEWKEYS");
        std::process::exit(1);
    }

    let newkeys_pkt = transport.encode_packet(&[packet::msg::NEWKEYS]);
    let _ = stack.tcp_send(conn, &newkeys_pkt);
    transport.set_keys(&shared_secret, &exchange_hash);

    // 8. Request ssh-userauth service
    let mut service_req = Vec::new();
    service_req.push(packet::msg::SERVICE_REQUEST);
    service_req.extend_from_slice(&packet::encode_string(b"ssh-userauth"));
    let pkt = transport.encode_packet(&service_req);
    let _ = stack.tcp_send(conn, &pkt);

    let service_accept = read_next_payload(stack, conn, &mut transport);
    if service_accept.is_empty() || service_accept[0] != packet::msg::SERVICE_ACCEPT {
        eprintln!("ssh: service accept failed");
        std::process::exit(1);
    }

    // 9. Authenticate with password
    let mut auth_req = Vec::new();
    auth_req.push(packet::msg::USERAUTH_REQUEST);
    auth_req.extend_from_slice(&packet::encode_string(username.as_bytes()));
    auth_req.extend_from_slice(&packet::encode_string(b"ssh-connection"));
    auth_req.extend_from_slice(&packet::encode_string(b"password"));
    auth_req.push(0);
    auth_req.extend_from_slice(&packet::encode_string(password.as_bytes()));
    let pkt = transport.encode_packet(&auth_req);
    let _ = stack.tcp_send(conn, &pkt);

    let auth_response = read_next_payload(stack, conn, &mut transport);
    if auth_response.is_empty() || auth_response[0] != packet::msg::USERAUTH_SUCCESS {
        eprintln!("ssh: authentication failed");
        std::process::exit(1);
    }

    // 10. Open session channel
    let local_channel_id: u32 = 0;
    let chan_open = channel::build_channel_open_session(local_channel_id, 32768, 32768);
    let pkt = transport.encode_packet(&chan_open);
    let _ = stack.tcp_send(conn, &pkt);

    let chan_confirm = read_next_payload(stack, conn, &mut transport);
    let (_, remote_channel_id, _, _) = channel::parse_channel_open_confirmation(&chan_confirm)
        .expect("ssh: channel open failed");

    // 11. Request PTY
    let pty_req = channel::build_pty_request(remote_channel_id, "xterm", 80, 24, 0, 0);
    let pkt = transport.encode_packet(&pty_req);
    let _ = stack.tcp_send(conn, &pkt);
    let _ = read_next_payload(stack, conn, &mut transport);

    // 12. Request shell
    let shell_req = channel::build_shell_request(remote_channel_id);
    let pkt = transport.encode_packet(&shell_req);
    let _ = stack.tcp_send(conn, &pkt);
    let _ = read_next_payload(stack, conn, &mut transport);

    eprintln!("ssh: connected (Ctrl+T to disconnect)");

    // 13. Relay loop
    let mut stdout = io::stdout();

    loop {
        stack.poll_rx();
        stack.poll_timers();

        // Read from remote
        match stack.tcp_recv(conn, &mut buf, 50) {
            Ok(0) => break,
            Ok(n) => {
                let payloads = transport.feed(&buf[..n]);
                for payload in payloads {
                    if payload.is_empty() { continue; }
                    match payload[0] {
                        packet::msg::CHANNEL_DATA => {
                            if let Some((_ch, data)) = channel::parse_channel_data(&payload) {
                                let _ = stdout.write_all(data);
                                let _ = stdout.flush();
                            }
                        }
                        packet::msg::CHANNEL_EOF | packet::msg::CHANNEL_CLOSE => {
                            eprintln!("\r\nssh: connection closed by remote");
                            stack.tcp_close_immediate(conn);
                            return;
                        }
                        packet::msg::CHANNEL_WINDOW_ADJUST => {}
                        packet::msg::DISCONNECT => {
                            eprintln!("\r\nssh: disconnected");
                            stack.tcp_close_immediate(conn);
                            return;
                        }
                        _ => {}
                    }
                }
            }
            Err(NetError::TimedOut) => {}
            Err(_) => break,
        }

        // Read from stdin (non-blocking)
        let mut input_buf = [0u8; 256];
        match io::stdin().read(&mut input_buf) {
            Ok(0) => {}
            Ok(n) => {
                if input_buf[..n].contains(&0x14) {
                    eprintln!("\r\nssh: disconnected (Ctrl+T)");
                    let close = channel::build_channel_close(remote_channel_id);
                    let pkt = transport.encode_packet(&close);
                    let _ = stack.tcp_send(conn, &pkt);
                    stack.tcp_close_immediate(conn);
                    return;
                }

                let chan_data = channel::build_channel_data(remote_channel_id, &input_buf[..n]);
                let pkt = transport.encode_packet(&chan_data);
                let _ = stack.tcp_send(conn, &pkt);
            }
            Err(_) => {}
        }
    }

    stack.tcp_close_immediate(conn);
}

fn read_next_payload(stack: &mut NetStack, conn: usize, transport: &mut SshTransport) -> Vec<u8> {
    // Keep a thread-local pending queue for payloads decoded in bulk from a single TCP segment.
    // transport.feed() may return multiple payloads at once; we return one and save the rest.
    static mut PENDING: Vec<Vec<u8>> = Vec::new();

    // Check pending queue first
    unsafe {
        if !PENDING.is_empty() {
            return PENDING.remove(0);
        }
    }

    let mut buf = [0u8; 4096];
    loop {
        // Try decoding from existing transport buffer
        let mut payloads = transport.feed(&[]);
        if !payloads.is_empty() {
            let first = payloads.remove(0);
            unsafe { PENDING.extend(payloads); }
            return first;
        }

        stack.poll_rx();
        stack.poll_timers();

        match stack.tcp_recv(conn, &mut buf, 10_000) {
            Ok(0) | Err(_) => return Vec::new(),
            Ok(n) => {
                let mut payloads = transport.feed(&buf[..n]);
                if !payloads.is_empty() {
                    let first = payloads.remove(0);
                    unsafe { PENDING.extend(payloads); }
                    return first;
                }
            }
        }
    }
}

fn parse_target(target: &str) -> (String, String, String, u16) {
    let (userinfo, hostport) = if let Some(at_pos) = target.rfind('@') {
        (Some(&target[..at_pos]), &target[at_pos + 1..])
    } else {
        (None, target)
    };

    let (username, password) = match userinfo {
        Some(ui) => {
            if let Some(colon) = ui.find(':') {
                (ui[..colon].to_string(), ui[colon + 1..].to_string())
            } else {
                (ui.to_string(), String::new())
            }
        }
        None => ("root".to_string(), String::new()),
    };

    let (host, port) = if let Some(colon) = hostport.rfind(':') {
        let port_str = &hostport[colon + 1..];
        if let Ok(p) = port_str.parse::<u16>() {
            (hostport[..colon].to_string(), p)
        } else {
            (hostport.to_string(), 22)
        }
    } else {
        (hostport.to_string(), 22)
    };

    (username, password, host, port)
}

fn parse_cidr(s: &str) -> Option<(Ipv4Addr, u8)> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 { return None; }
    let ip = parse_ipv4(parts[0])?;
    let prefix: u8 = parts[1].parse().ok()?;
    Some((ip, prefix))
}

fn parse_ipv4(s: &str) -> Option<Ipv4Addr> {
    let octets: Vec<u8> = s.split('.').filter_map(|o| o.parse().ok()).collect();
    if octets.len() != 4 { return None; }
    Some(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]))
}
