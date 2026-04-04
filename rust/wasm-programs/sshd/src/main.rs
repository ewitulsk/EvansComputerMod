//! SSH server (sshd) — standalone WASI program.
//!
//! Uses the ecm-net networking stack directly with raw Ethernet frame host
//! functions. Listens for SSH connections, authenticates users, and relays
//! shell I/O via IPC host functions.

use ecm_host_abi::{ipc, fs};
use ecm_ssh_protocol::{packet, transport::SshTransport, kex, auth, channel};
use ecm_ssh_crypto as crypto;
use ecm_net::NetStack;
use ecm_net::types::{Ipv4Addr, NetError};

mod hostkey;

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(22);

    // Initialize the networking stack (discovers interfaces from host)
    NetStack::init();
    let stack = match NetStack::get() {
        Some(s) => s,
        None => {
            eprintln!("sshd: failed to initialize network stack");
            return;
        }
    };

    // Configure network from environment variables (set by kernel when spawning)
    let ip_str = std::env::var("NET_IP").unwrap_or_else(|_| "10.0.0.1/24".to_string());
    let gw_str = std::env::var("NET_GATEWAY").unwrap_or_else(|_| "10.0.0.254".to_string());
    let dns_str = std::env::var("NET_DNS").unwrap_or_else(|_| "10.0.0.254".to_string());

    // Parse and apply IP config
    if let Some((ip, prefix)) = parse_cidr(&ip_str) {
        stack.configure_iface(0, ip, prefix);
    }
    if let Some(gw) = parse_ip(&gw_str) {
        let _ = stack.routing.add_route(Ipv4Addr::ZERO, 0, gw, 0);
    }
    if let Some(dns) = parse_ip(&dns_str) {
        stack.dns_server = dns;
    }

    eprintln!("sshd: starting on port {}", port);

    // Listen
    let listener = match stack.tcp_connections.listen(Ipv4Addr::ZERO, port) {
        Ok(idx) => idx,
        Err(e) => {
            eprintln!("sshd: listen failed: {:?}", e);
            return;
        }
    };

    // Accept loop
    loop {
        stack.poll_rx();
        stack.poll_timers();

        match stack.tcp_accept(listener, 500) {
            Ok(conn) => {
                eprintln!("sshd: accepted connection (idx={})", conn);
                handle_connection(stack, conn);
            }
            Err(NetError::TimedOut) => continue,
            Err(e) => {
                eprintln!("sshd: accept error: {:?}", e);
                break;
            }
        }
    }

    stack.tcp_close(listener);
}

fn handle_connection(stack: &mut NetStack, conn: usize) {
    let mut transport = SshTransport::new(true);
    let mut channels = channel::ChannelManager::new();
    let mut authenticated = false;
    let mut username = String::new();
    let mut our_kexinit: Vec<u8> = Vec::new();
    let mut peer_kexinit: Vec<u8> = Vec::new();
    let mut service_requested = false;
    let mut ipc_session: Option<i32> = None;
    let mut active_remote_channel_id: Option<u32> = None;

    // Load host key
    let (host_pub, host_priv) = hostkey::load_or_generate_host_key();

    // 1. Send version string
    let version_line = transport.version_line();
    let _ = stack.tcp_send(conn, &version_line);

    // 2. Read peer version
    let mut ver_buf: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    let peer_version_ok = loop {
        match stack.tcp_recv(conn, &mut buf, 10_000) {
            Ok(0) => break false,
            Ok(n) => {
                ver_buf.extend_from_slice(&buf[..n]);
                if let Some(pos) = ver_buf.iter().position(|&b| b == b'\n') {
                    let line = core::str::from_utf8(&ver_buf[..pos]).unwrap_or("");
                    if transport.set_peer_version(line) {
                        let remaining = ver_buf[pos + 1..].to_vec();
                        if !remaining.is_empty() {
                            transport.recv_buffer.extend_from_slice(&remaining);
                        }
                        break true;
                    } else {
                        break false;
                    }
                }
            }
            Err(_) => break false,
        }
    };

    if !peer_version_ok {
        stack.tcp_close_immediate(conn);
        return;
    }

    // 3. Send KEXINIT
    let kexinit_pkt = transport.build_kexinit_packet();
    our_kexinit = kex::build_kexinit();
    let _ = stack.tcp_send(conn, &kexinit_pkt);

    // 4. Main packet loop
    loop {
        stack.poll_rx();
        stack.poll_timers();

        // Read from TCP
        match stack.tcp_recv(conn, &mut buf, 100) {
            Ok(0) => break,
            Ok(n) => {
                let payloads = transport.feed(&buf[..n]);
                for payload in payloads {
                    if payload.is_empty() { continue; }
                    let msg_type = payload[0];

                    match msg_type {
                        packet::msg::KEXINIT => {
                            peer_kexinit = payload.clone();
                        }
                        packet::msg::KEX_ECDH_INIT => {
                            if let Some((client_epub_bytes, _)) = packet::decode_string(&payload, 1) {
                                if client_epub_bytes.len() == 32 {
                                    let mut client_epub = [0u8; 32];
                                    client_epub.copy_from_slice(client_epub_bytes);

                                    let (server_epub, server_secret) = crypto::x25519_generate_keypair();
                                    let their_pub = crypto::X25519Public::from(client_epub);
                                    let shared_secret = crypto::x25519_diffie_hellman(&server_secret, &their_pub);

                                    let host_pub_bytes = crypto::ed25519_public_key_bytes(&host_pub);
                                    let host_key_blob = kex::encode_ed25519_public_key(&host_pub_bytes);

                                    let server_epub_bytes = server_epub.to_bytes();
                                    let exchange_hash = kex::compute_exchange_hash(
                                        &transport.peer_version,
                                        &transport.our_version,
                                        &peer_kexinit,
                                        &our_kexinit,
                                        &host_key_blob,
                                        &client_epub,
                                        &server_epub_bytes,
                                        &shared_secret,
                                    );

                                    let signature = crypto::ed25519_sign(&host_priv, &exchange_hash);
                                    let sig_blob = kex::encode_ed25519_signature(&signature);

                                    let mut reply_payload = Vec::new();
                                    reply_payload.push(packet::msg::KEX_ECDH_REPLY);
                                    reply_payload.extend_from_slice(&packet::encode_string(&host_key_blob));
                                    reply_payload.extend_from_slice(&packet::encode_string(&server_epub_bytes));
                                    reply_payload.extend_from_slice(&packet::encode_string(&sig_blob));

                                    let reply_pkt = transport.encode_packet(&reply_payload);
                                    let _ = stack.tcp_send(conn, &reply_pkt);

                                    let newkeys = transport.encode_packet(&[packet::msg::NEWKEYS]);
                                    let _ = stack.tcp_send(conn, &newkeys);

                                    transport.set_keys(&shared_secret, &exchange_hash);
                                }
                            }
                        }
                        packet::msg::NEWKEYS => {}
                        packet::msg::SERVICE_REQUEST => {
                            if let Some((service, _)) = packet::decode_string(&payload, 1) {
                                if service == b"ssh-userauth" {
                                    let accept = auth::build_service_accept("ssh-userauth");
                                    let pkt = transport.encode_packet(&accept);
                                    let _ = stack.tcp_send(conn, &pkt);
                                    service_requested = true;
                                }
                            }
                        }
                        packet::msg::USERAUTH_REQUEST if service_requested => {
                            let passwd_content = fs::read_file("etc/passwd");
                            let user_db = auth::UserDb::load(
                                passwd_content.as_deref(),
                                |user| fs::read_file(&format!("home/{}/.ssh/authorized_keys", user)),
                            );

                            if let Some(req) = auth::parse_userauth_request(&payload) {
                                let (success, user) = match req {
                                    auth::AuthRequest::Password { username: u, password: p } => {
                                        (user_db.check_password(&u, &p), u)
                                    }
                                    auth::AuthRequest::PublicKey { username: u, key, signature } => {
                                        if signature.is_some() {
                                            (user_db.check_public_key(&u, &key), u)
                                        } else {
                                            (false, u)
                                        }
                                    }
                                    auth::AuthRequest::None { username: u } => {
                                        (user_db.check_password(&u, ""), u)
                                    }
                                };

                                if success {
                                    authenticated = true;
                                    username = user;
                                    let pkt = transport.encode_packet(&auth::build_userauth_success());
                                    let _ = stack.tcp_send(conn, &pkt);
                                } else {
                                    let failure = auth::build_userauth_failure(&["password", "publickey"], false);
                                    let pkt = transport.encode_packet(&failure);
                                    let _ = stack.tcp_send(conn, &pkt);
                                }
                            }
                        }
                        packet::msg::CHANNEL_OPEN if authenticated => {
                            if let Some(req) = channel::parse_channel_open(&payload) {
                                if req.channel_type == "session" {
                                    if let Some(local_id) = channels.open(req.sender_channel, req.initial_window, req.max_packet) {
                                        let confirm = channel::build_channel_open_confirmation(
                                            req.sender_channel, local_id, 32768, 32768,
                                        );
                                        let pkt = transport.encode_packet(&confirm);
                                        let _ = stack.tcp_send(conn, &pkt);
                                    }
                                }
                            }
                        }
                        packet::msg::CHANNEL_REQUEST if authenticated => {
                            if let Some(req) = channel::parse_channel_request(&payload) {
                                match req {
                                    channel::ChannelRequest::PtyReq { recipient_channel, want_reply, .. } => {
                                        if want_reply {
                                            if let Some(ch) = channels.get(recipient_channel) {
                                                let pkt = transport.encode_packet(
                                                    &channel::build_channel_success(ch.remote_id),
                                                );
                                                let _ = stack.tcp_send(conn, &pkt);
                                            }
                                        }
                                    }
                                    channel::ChannelRequest::Shell { recipient_channel, want_reply } => {
                                        let session_id = ipc::spawn_shell(&username);
                                        if session_id >= 0 {
                                            ipc_session = Some(session_id);
                                            if let Some(ch) = channels.get(recipient_channel) {
                                                active_remote_channel_id = Some(ch.remote_id);
                                            }
                                            if want_reply {
                                                if let Some(ch) = channels.get(recipient_channel) {
                                                    let pkt = transport.encode_packet(
                                                        &channel::build_channel_success(ch.remote_id),
                                                    );
                                                    let _ = stack.tcp_send(conn, &pkt);
                                                }
                                            }
                                        } else if want_reply {
                                            if let Some(ch) = channels.get(recipient_channel) {
                                                let pkt = transport.encode_packet(
                                                    &channel::build_channel_failure(ch.remote_id),
                                                );
                                                let _ = stack.tcp_send(conn, &pkt);
                                            }
                                        }
                                    }
                                    channel::ChannelRequest::WindowChange { width_chars, height_rows, .. } => {
                                        if let Some(sid) = ipc_session {
                                            ipc::session_resize(sid, width_chars as u16, height_rows as u16);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        packet::msg::CHANNEL_DATA if authenticated => {
                            if let Some((_ch_id, data)) = channel::parse_channel_data(&payload) {
                                if let Some(sid) = ipc_session {
                                    ipc::session_write(sid, data);
                                }
                            }
                        }
                        packet::msg::CHANNEL_WINDOW_ADJUST => {}
                        packet::msg::CHANNEL_EOF | packet::msg::CHANNEL_CLOSE => {
                            if let Some(sid) = ipc_session.take() {
                                ipc::session_close(sid);
                            }
                            break;
                        }
                        packet::msg::DISCONNECT => {
                            if let Some(sid) = ipc_session.take() {
                                ipc::session_close(sid);
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
            Err(NetError::TimedOut) => {}
            Err(_) => break,
        }

        // Read shell output via IPC and send to SSH client
        if let (Some(sid), Some(remote_ch)) = (ipc_session, active_remote_channel_id) {
            let mut ipc_buf = [0u8; 4096];
            let m = ipc::session_read(sid, &mut ipc_buf);
            if m > 0 {
                let chan_data = channel::build_channel_data(remote_ch, &ipc_buf[..m as usize]);
                let pkt = transport.encode_packet(&chan_data);
                let _ = stack.tcp_send(conn, &pkt);
            }

            if ipc::session_status(sid) != 0 {
                let close_pkt = transport.encode_packet(&channel::build_channel_close(remote_ch));
                let _ = stack.tcp_send(conn, &close_pkt);
                ipc::session_close(sid);
                break;
            }
        }
    }

    stack.tcp_close_immediate(conn);
}

fn parse_cidr(s: &str) -> Option<(Ipv4Addr, u8)> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 { return None; }
    let ip = parse_ip(parts[0])?;
    let prefix: u8 = parts[1].parse().ok()?;
    Some((ip, prefix))
}

fn parse_ip(s: &str) -> Option<Ipv4Addr> {
    let octets: Vec<u8> = s.split('.').filter_map(|o| o.parse().ok()).collect();
    if octets.len() != 4 { return None; }
    Some(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]))
}

use std::string::String;
use std::vec::Vec;
use std::format;
