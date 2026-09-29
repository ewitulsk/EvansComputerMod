//! SSH server (sshd) — standalone WASI program.
//!
//! Listens for SSH connections on a kernel-proxied TCP socket
//! (`ecm_host_abi::socket`), authenticates users, and relays shell I/O via
//! IPC host functions. Interface, route and DNS configuration is owned by the
//! kernel. Connections are served one at a time.

use ecm_host_abi::{ipc, fs};
use ecm_host_abi::socket::{self, SockAddrIn, AF_INET, SOCK_STREAM, SOL_SOCKET, SO_REUSEADDR, SO_RCVTIMEO};
use ecm_ssh_protocol::{packet, transport::SshTransport, kex, auth, channel};
use ecm_ssh_crypto as crypto;

mod hostkey;

/// Overall timeout for the client's version line.
const VERSION_TIMEOUT_MS: u64 = 10_000;
/// Receive timeout on a connection, so the loop wakes up to pump shell output.
const CONN_POLL_MS: i32 = 50;

/// Result of one `recv` on the socket.
enum Recv {
    Data(usize),
    /// SO_RCVTIMEO elapsed with no data (-2).
    Timeout,
    /// Orderly EOF (0) or error/reset (-1).
    Closed,
}

fn recv(fd: i32, buf: &mut [u8]) -> Recv {
    match socket::recv(fd, buf, 0) {
        n if n > 0 => Recv::Data(n as usize),
        -2 => Recv::Timeout,
        _ => Recv::Closed,
    }
}

/// Send the whole buffer, looping over partial sends. Chunked so each socket
/// IPC request stays small.
fn send_all(fd: i32, mut data: &[u8]) -> bool {
    const CHUNK: usize = 1400;
    while !data.is_empty() {
        let n = socket::send(fd, &data[..data.len().min(CHUNK)], 0);
        if n <= 0 {
            return false;
        }
        data = &data[n as usize..];
    }
    true
}

fn main() {
    let arg = std::env::args().nth(1);
    if matches!(arg.as_deref(), Some("-h") | Some("--help")) {
        eprintln!("Usage: sshd [port]   (default 22)");
        eprintln!("  Uses this computer's network configuration (ifconfig/ip/route).");
        return;
    }
    let port: u16 = arg.and_then(|s| s.parse().ok()).unwrap_or(22);

    eprintln!("sshd: starting on port {}", port);

    // Listen
    let listener = socket::socket(AF_INET, SOCK_STREAM, 0);
    if listener < 0 {
        eprintln!("sshd: failed to create socket");
        return;
    }
    socket::setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &1i32.to_le_bytes());
    if socket::bind(listener, &SockAddrIn::any(port)) != 0 {
        eprintln!("sshd: bind to port {} failed", port);
        socket::close(listener);
        return;
    }
    if socket::listen(listener, 4) != 0 {
        eprintln!("sshd: listen failed");
        socket::close(listener);
        return;
    }

    // Accept loop (accept blocks; -2 means a timeout, just retry)
    loop {
        let mut peer = SockAddrIn::default();
        let conn = socket::accept(listener, &mut peer);
        if conn >= 0 {
            eprintln!("sshd: accepted connection from {}", peer.to_string());
            socket::setsockopt(conn, SOL_SOCKET, SO_RCVTIMEO, &CONN_POLL_MS.to_le_bytes());
            handle_connection(conn);
            socket::close(conn);
        } else if conn == -2 {
            continue;
        } else {
            eprintln!("sshd: accept error ({})", conn);
            break;
        }
    }

    socket::close(listener);
}

fn handle_connection(conn: i32) {
    let mut transport = SshTransport::new(true);
    let mut channels = channel::ChannelManager::new();
    let mut authenticated = false;
    let mut username = String::new();
    let our_kexinit: Vec<u8>;
    let mut peer_kexinit: Vec<u8> = Vec::new();
    let mut service_requested = false;
    let mut ipc_session: Option<i32> = None;
    let mut active_remote_channel_id: Option<u32> = None;

    // Load host key
    let (host_pub, host_priv) = hostkey::load_or_generate_host_key();

    // 1. Send version string
    let version_line = transport.version_line();
    let _ = send_all(conn, &version_line);

    // 2. Read peer version
    let mut ver_buf: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(VERSION_TIMEOUT_MS);
    let peer_version_ok = loop {
        match recv(conn, &mut buf) {
            Recv::Data(n) => {
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
            Recv::Timeout if std::time::Instant::now() < deadline => {}
            Recv::Timeout | Recv::Closed => break false,
        }
    };

    if !peer_version_ok {
        return;
    }

    // 3. Send KEXINIT
    let kexinit_pkt = transport.build_kexinit_packet();
    our_kexinit = kex::build_kexinit();
    let _ = send_all(conn, &kexinit_pkt);

    // 4. Main packet loop
    'conn: loop {
        // Read from TCP
        match recv(conn, &mut buf) {
            Recv::Closed => {
                if let Some(sid) = ipc_session.take() {
                    ipc::session_close(sid);
                }
                break;
            }
            Recv::Data(n) => {
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
                                    let _ = send_all(conn, &reply_pkt);

                                    let newkeys = transport.encode_packet(&[packet::msg::NEWKEYS]);
                                    let _ = send_all(conn, &newkeys);

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
                                    let _ = send_all(conn, &pkt);
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
                                    let _ = send_all(conn, &pkt);
                                } else {
                                    let failure = auth::build_userauth_failure(&["password", "publickey"], false);
                                    let pkt = transport.encode_packet(&failure);
                                    let _ = send_all(conn, &pkt);
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
                                        let _ = send_all(conn, &pkt);
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
                                                let _ = send_all(conn, &pkt);
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
                                                    let _ = send_all(conn, &pkt);
                                                }
                                            }
                                        } else if want_reply {
                                            if let Some(ch) = channels.get(recipient_channel) {
                                                let pkt = transport.encode_packet(
                                                    &channel::build_channel_failure(ch.remote_id),
                                                );
                                                let _ = send_all(conn, &pkt);
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
                            break 'conn;
                        }
                        packet::msg::DISCONNECT => {
                            if let Some(sid) = ipc_session.take() {
                                ipc::session_close(sid);
                            }
                            break 'conn;
                        }
                        _ => {}
                    }
                }
            }
            Recv::Timeout => {}
        }

        // Read shell output via IPC and send to SSH client
        if let (Some(sid), Some(remote_ch)) = (ipc_session, active_remote_channel_id) {
            let mut ipc_buf = [0u8; 4096];
            let m = ipc::session_read(sid, &mut ipc_buf);
            if m > 0 {
                let chan_data = channel::build_channel_data(remote_ch, &ipc_buf[..m as usize]);
                let pkt = transport.encode_packet(&chan_data);
                let _ = send_all(conn, &pkt);
            }

            if ipc::session_status(sid) != 0 {
                let close_pkt = transport.encode_packet(&channel::build_channel_close(remote_ch));
                let _ = send_all(conn, &close_pkt);
                ipc::session_close(sid);
                break 'conn;
            }
        }
    }
}

use std::string::String;
use std::vec::Vec;
use std::format;
