//! SSH server (sshd) implementation.
//!
//! Runs as a kernel builtin, mirroring the httpd pattern: a blocking
//! accept-loop that uses the TCP stack's high-level API.

use std::string::String;
use std::vec::Vec;

use crate::net;
use crate::net::types::Ipv4Addr;
use crate::crypto;
use crate::{print, println};
use crate::shell::ShellInstance;
use super::transport::{SshTransport, TransportState};
use super::kex;
use super::auth;
use super::channel;
use super::packet;

/// Handle a single SSH connection from accept to close.
fn handle_connection(stack: &mut net::NetStack, conn: usize) {
    // --- Transport state ---
    let mut transport = SshTransport::new(true);
    let mut channels = channel::ChannelManager::new();
    let mut authenticated = false;
    let mut username = String::new();
    let mut our_kexinit: Vec<u8> = Vec::new();
    let mut peer_kexinit: Vec<u8> = Vec::new();
    let mut service_requested = false;

    // SSH shell instance for this connection
    let mut ssh_shell = ShellInstance::new_ssh();
    // Line buffer for accumulating input
    let mut line_buf: Vec<u8> = Vec::new();

    // Load host key
    let (host_pub, host_priv) = crypto::load_or_generate_host_key();

    // 1. Send our version string
    let version_line = transport.version_line();
    let _ = stack.tcp_send(conn, &version_line);

    // 2. Read peer version string
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
                        // Keep any remaining bytes after the version line
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
        stack.tcp_close(conn);
        return;
    }

    // 3. Send our KEXINIT
    let kexinit_payload = kex::build_kexinit();
    our_kexinit = kexinit_payload.clone();
    let pkt = transport.encode_packet(&kexinit_payload);
    let _ = stack.tcp_send(conn, &pkt);
    transport.state = TransportState::KexInitSent;

    // 4. Main message loop
    let mut active = true;
    while active {
        // Poll network
        stack.poll_rx();
        stack.poll_timers();

        // Try to read data (short timeout to stay responsive)
        let n = match stack.tcp_recv(conn, &mut buf, 500) {
            Ok(n) => n,
            Err(net::types::NetError::TimedOut) => 0,
            Err(_) => { active = false; continue; }
        };

        // Feed data into the transport layer
        let payloads = if n > 0 {
            transport.feed(&buf[..n])
        } else {
            // Try to decode packets from existing buffer
            transport.feed(&[])
        };

        for payload in payloads {
            if payload.is_empty() { continue; }
            let msg_type = payload[0];

            match msg_type {
                packet::msg::KEXINIT => {
                    peer_kexinit = payload.to_vec();
                    if matches!(transport.state, TransportState::KexInitSent) {
                        transport.state = TransportState::KexInProgress {
                            our_kexinit: our_kexinit.clone(),
                            peer_kexinit: peer_kexinit.clone(),
                        };
                    }
                }
                packet::msg::KEX_ECDH_INIT => {
                    // Client's ephemeral public key
                    if let Some((client_pub_bytes, _)) = packet::decode_string(&payload, 1) {
                        if client_pub_bytes.len() == 32 {
                            let mut client_pub = [0u8; 32];
                            client_pub.copy_from_slice(client_pub_bytes);

                            // Generate our ephemeral key
                            let (eph_public_key, eph_secret) = crypto::x25519_generate_keypair();
                            let eph_pub_bytes = eph_public_key.to_bytes();

                            // Compute shared secret
                            let client_x25519 = x25519_dalek::PublicKey::from(client_pub);
                            let shared_secret = crypto::x25519_diffie_hellman(&eph_secret, &client_x25519);

                            // Compute host key blob
                            let host_key_blob = kex::encode_ed25519_public_key(&host_pub.to_bytes());

                            // Compute exchange hash
                            let hash = kex::compute_exchange_hash(
                                &transport.peer_version,
                                &transport.our_version,
                                &peer_kexinit,
                                &our_kexinit,
                                &host_key_blob,
                                &client_pub,
                                &eph_pub_bytes,
                                &shared_secret,
                            );

                            // Sign the hash
                            let signature = crypto::ed25519_sign(&host_priv, &hash);
                            let sig_blob = kex::encode_ed25519_signature(&signature);

                            // Build KEX_ECDH_REPLY
                            let mut reply = Vec::new();
                            reply.push(packet::msg::KEX_ECDH_REPLY);
                            reply.extend_from_slice(&packet::encode_string(&host_key_blob));
                            reply.extend_from_slice(&packet::encode_string(&eph_pub_bytes));
                            reply.extend_from_slice(&packet::encode_string(&sig_blob));

                            let pkt = transport.encode_packet(&reply);
                            let _ = stack.tcp_send(conn, &pkt);

                            // Send NEWKEYS
                            let newkeys = transport.encode_packet(&[packet::msg::NEWKEYS]);
                            let _ = stack.tcp_send(conn, &newkeys);

                            // Set session keys
                            transport.set_keys(&shared_secret, &hash);
                        }
                    }
                }
                packet::msg::NEWKEYS => {
                    // Client acknowledged key exchange — transport is established
                }
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
                packet::msg::USERAUTH_REQUEST => {
                    if let Some(request) = auth::parse_userauth_request(&payload) {
                        let db = auth::UserDb::load();
                        let success = match &request {
                            auth::AuthRequest::Password { username: u, password } => {
                                username = u.clone();
                                db.check_password(u, password)
                            }
                            auth::AuthRequest::PublicKey { username: u, key, .. } => {
                                username = u.clone();
                                db.check_public_key(u, key)
                            }
                            auth::AuthRequest::None { username: u } => {
                                username = u.clone();
                                db.check_password(u, "")
                            }
                        };

                        let response = if success {
                            authenticated = true;
                            let msg = format!("sshd: {} authenticated", username);
                            println(&msg);
                            auth::build_userauth_success()
                        } else {
                            auth::build_userauth_failure(&["password", "publickey"], false)
                        };
                        let pkt = transport.encode_packet(&response);
                        let _ = stack.tcp_send(conn, &pkt);
                    }
                }
                packet::msg::CHANNEL_OPEN => {
                    if let Some(req) = channel::parse_channel_open(&payload) {
                        if req.channel_type == "session" && authenticated {
                            if let Some(local_id) = channels.open(req.sender_channel, req.initial_window, req.max_packet) {
                                let confirm = channel::build_channel_open_confirmation(
                                    req.sender_channel, local_id, 32768, 32768,
                                );
                                let pkt = transport.encode_packet(&confirm);
                                let _ = stack.tcp_send(conn, &pkt);
                            } else {
                                let fail = channel::build_channel_open_failure(
                                    req.sender_channel, 4, "max channels reached",
                                );
                                let pkt = transport.encode_packet(&fail);
                                let _ = stack.tcp_send(conn, &pkt);
                            }
                        } else {
                            let fail = channel::build_channel_open_failure(
                                req.sender_channel, 2, "not authenticated",
                            );
                            let pkt = transport.encode_packet(&fail);
                            let _ = stack.tcp_send(conn, &pkt);
                        }
                    }
                }
                packet::msg::CHANNEL_REQUEST => {
                    if let Some(req) = channel::parse_channel_request(&payload) {
                        match req {
                            channel::ChannelRequest::PtyReq { recipient_channel, want_reply, .. } => {
                                if let Some(ch) = channels.get_mut(recipient_channel) {
                                    ch.tty_id = Some(0); // placeholder
                                }
                                if want_reply {
                                    let remote_id = channels.get(recipient_channel)
                                        .map(|c| c.remote_id).unwrap_or(0);
                                    let success = channel::build_channel_success(remote_id);
                                    let pkt = transport.encode_packet(&success);
                                    let _ = stack.tcp_send(conn, &pkt);
                                }
                            }
                            channel::ChannelRequest::Shell { recipient_channel, want_reply } => {
                                if want_reply {
                                    let remote_id = channels.get(recipient_channel)
                                        .map(|c| c.remote_id).unwrap_or(0);
                                    let success = channel::build_channel_success(remote_id);
                                    let pkt = transport.encode_packet(&success);
                                    let _ = stack.tcp_send(conn, &pkt);
                                }

                                // Send a welcome banner and prompt
                                let remote_id = channels.get(recipient_channel)
                                    .map(|c| c.remote_id).unwrap_or(0);
                                let cwd_display = if ssh_shell.cwd().is_empty() { "/" } else { ssh_shell.cwd() };
                                let banner = format!(
                                    "Welcome to TerminalOS SSH ({}@computer)\r\n/ > ",
                                    username
                                );
                                let data_pkt = channel::build_channel_data(remote_id, banner.as_bytes());
                                let pkt = transport.encode_packet(&data_pkt);
                                let _ = stack.tcp_send(conn, &pkt);
                            }
                            channel::ChannelRequest::Exec { recipient_channel, want_reply, command } => {
                                let remote_id = channels.get(recipient_channel)
                                    .map(|c| c.remote_id).unwrap_or(0);

                                // Execute the command via the SSH shell
                                crate::process_command(&mut ssh_shell, &command);
                                let mut output = ssh_shell.drain_output();

                                // Convert \n to \r\n for terminal
                                let converted = convert_lf_to_crlf(&output);

                                if !converted.is_empty() {
                                    let data_pkt = channel::build_channel_data(remote_id, &converted);
                                    let pkt = transport.encode_packet(&data_pkt);
                                    let _ = stack.tcp_send(conn, &pkt);
                                }

                                if want_reply {
                                    let success = channel::build_channel_success(remote_id);
                                    let pkt = transport.encode_packet(&success);
                                    let _ = stack.tcp_send(conn, &pkt);
                                }
                            }
                            channel::ChannelRequest::WindowChange { .. } => {
                                // Acknowledge but no action needed
                            }
                        }
                    }
                }
                packet::msg::CHANNEL_DATA => {
                    if let Some((channel_id, data)) = channel::parse_channel_data(&payload) {
                        if let Some(ch) = channels.get(channel_id) {
                            let remote_id = ch.remote_id;

                            // Process each byte: accumulate into line_buf, execute on Enter
                            for &byte in data {
                                match byte {
                                    3 => {
                                        // Ctrl+C: clear line buffer, send new prompt
                                        line_buf.clear();
                                        let cwd = ssh_shell.cwd().to_string();
                                        let cwd_display = if cwd.is_empty() { "/".to_string() } else { format!("/{}", cwd) };
                                        let prompt = format!("\r\n{} > ", cwd_display);
                                        let data_pkt = channel::build_channel_data(remote_id, prompt.as_bytes());
                                        let pkt = transport.encode_packet(&data_pkt);
                                        let _ = stack.tcp_send(conn, &pkt);
                                    }
                                    4 => {
                                        // Ctrl+D: close connection
                                        if line_buf.is_empty() {
                                            active = false;
                                        }
                                    }
                                    8 | 127 => {
                                        // Backspace
                                        if !line_buf.is_empty() {
                                            line_buf.pop();
                                            // Echo backspace: move back, space, move back
                                            let bs = b"\x08 \x08";
                                            let data_pkt = channel::build_channel_data(remote_id, bs);
                                            let pkt = transport.encode_packet(&data_pkt);
                                            let _ = stack.tcp_send(conn, &pkt);
                                        }
                                    }
                                    b'\r' | b'\n' => {
                                        // Enter: execute command
                                        // Echo newline
                                        let nl = b"\r\n";
                                        let data_pkt = channel::build_channel_data(remote_id, nl);
                                        let pkt = transport.encode_packet(&data_pkt);
                                        let _ = stack.tcp_send(conn, &pkt);

                                        if !line_buf.is_empty() {
                                            let cmd = String::from_utf8_lossy(&line_buf).to_string();
                                            line_buf.clear();

                                            // Execute command via shell
                                            ssh_shell.with_cwd(|shell| {
                                                crate::process_command(shell, &cmd);
                                            });

                                            let output = ssh_shell.drain_output();
                                            if !output.is_empty() {
                                                let converted = convert_lf_to_crlf(&output);
                                                let data_pkt = channel::build_channel_data(remote_id, &converted);
                                                let pkt = transport.encode_packet(&data_pkt);
                                                let _ = stack.tcp_send(conn, &pkt);
                                            }
                                        }

                                        // Send prompt
                                        let cwd = ssh_shell.cwd().to_string();
                                        let cwd_display = if cwd.is_empty() { "/".to_string() } else { format!("/{}", cwd) };
                                        let prompt = format!("{} > ", cwd_display);
                                        let data_pkt = channel::build_channel_data(remote_id, prompt.as_bytes());
                                        let pkt = transport.encode_packet(&data_pkt);
                                        let _ = stack.tcp_send(conn, &pkt);
                                    }
                                    _ if byte >= 32 && byte < 127 => {
                                        // Printable character: add to buffer, echo back
                                        line_buf.push(byte);
                                        let echo = [byte];
                                        let data_pkt = channel::build_channel_data(remote_id, &echo);
                                        let pkt = transport.encode_packet(&data_pkt);
                                        let _ = stack.tcp_send(conn, &pkt);
                                    }
                                    _ => {
                                        // Ignore other control characters
                                    }
                                }
                            }

                            // Window adjust
                            let adjust = channel::build_window_adjust(remote_id, data.len() as u32);
                            let pkt = transport.encode_packet(&adjust);
                            let _ = stack.tcp_send(conn, &pkt);
                        }
                    }
                }
                packet::msg::CHANNEL_EOF | packet::msg::CHANNEL_CLOSE => {
                    let channel_id = if payload.len() >= 5 {
                        u32::from_be_bytes([payload[1], payload[2], payload[3], payload[4]])
                    } else {
                        0
                    };

                    if let Some(ch) = channels.get(channel_id) {
                        let remote_id = ch.remote_id;
                        if msg_type == packet::msg::CHANNEL_CLOSE {
                            let close = channel::build_channel_close(remote_id);
                            let pkt = transport.encode_packet(&close);
                            let _ = stack.tcp_send(conn, &pkt);
                        }
                    }
                    channels.close(channel_id);
                }
                packet::msg::CHANNEL_WINDOW_ADJUST => {
                    // Update remote window — ignore for now
                }
                packet::msg::DISCONNECT => {
                    active = false;
                }
                packet::msg::IGNORE => {
                    // Ignore
                }
                _ => {
                    // Unknown message — send UNIMPLEMENTED
                    let mut resp = Vec::new();
                    resp.push(packet::msg::UNIMPLEMENTED);
                    resp.extend_from_slice(&transport.recv_seq.to_be_bytes());
                    let pkt = transport.encode_packet(&resp);
                    let _ = stack.tcp_send(conn, &pkt);
                }
            }
        }
    }

    stack.tcp_close(conn);
}

/// Convert \n to \r\n in a byte buffer (for SSH terminal output).
fn convert_lf_to_crlf(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(data.len() + data.len() / 10);
    for &byte in data {
        if byte == b'\n' {
            result.push(b'\r');
        }
        result.push(byte);
    }
    result
}

/// Run the SSH server (blocking accept loop, like httpd).
pub fn run_sshd(port: u16) {
    let stack = match net::NetStack::get() {
        Some(s) => s,
        None => {
            println("sshd: network stack not initialized");
            return;
        }
    };

    if !stack.configured() {
        println("sshd: network not configured. Use: ifconfig <iface> <ip>/<prefix>");
        return;
    }

    let listener = match stack.tcp_connections.listen(Ipv4Addr::ZERO, port) {
        Ok(idx) => idx,
        Err(e) => {
            let msg = format!("sshd: failed to listen: {:?}", e);
            println(&msg);
            return;
        }
    };

    let msg = format!("sshd: listening on port {}. Ctrl+T to stop.", port);
    println(&msg);

    // Accept loop (mirrors httpd pattern)
    loop {
        stack.poll_rx();
        stack.poll_timers();

        match stack.tcp_accept(listener, 500) {
            Ok(conn) => {
                println("sshd: new connection");
                handle_connection(stack, conn);
                println("sshd: connection closed");
            }
            Err(net::types::NetError::TimedOut) => {
                // No connection yet, loop around
                continue;
            }
            Err(e) => {
                let msg = format!("sshd: accept error: {:?}", e);
                println(&msg);
                break;
            }
        }
    }

    stack.tcp_close(listener);
    println("sshd: stopped.");
}
