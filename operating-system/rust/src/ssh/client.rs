//! SSH client implementation.
//!
//! Connects to a remote SSH server (e.g. another in-game computer running sshd),
//! performs version exchange, key exchange, password authentication, opens a
//! session channel with a PTY, and bridges the local terminal to the remote shell.

use std::format;
use std::string::String;
use std::vec::Vec;

use crate::crypto;
use crate::net;
use crate::net::types::{Ipv4Addr, SocketAddr, NetError};
use crate::shell::ShellInstance;

use super::transport::SshTransport;
use super::kex;
use super::packet;
use super::channel;

/// Run an SSH client session connecting to `host:port` as `username`.
/// If `password` is `Some`, use it directly; otherwise prompt interactively.
pub fn ssh_connect(shell: &mut ShellInstance, host: &str, port: u16, username: &str, password: Option<&str>) {
    let stack = match net::NetStack::get() {
        Some(s) => s,
        None => {
            shell.println("ssh: network stack not initialized");
            return;
        }
    };

    if !stack.configured() {
        shell.println("ssh: network not configured. Use: ifconfig <iface> <ip>/<prefix>");
        return;
    }

    // Resolve host to IP
    let ip = if let Some(ip) = Ipv4Addr::parse(host) {
        ip
    } else {
        match stack.dns_resolve(host, 5000) {
            Ok(ip) => ip,
            Err(_) => {
                shell.print("ssh: could not resolve ");
                shell.println(host);
                return;
            }
        }
    };

    let remote = SocketAddr { ip, port };
    shell.print("Connecting to ");
    shell.print(&format!("{}:{}...", host, port));
    shell.println("");

    // TCP connect (blocking, 10s timeout)
    let conn = match stack.tcp_connect(remote, 10000) {
        Ok(idx) => idx,
        Err(e) => {
            shell.println(&format!("ssh: connection failed: {:?}", e));
            return;
        }
    };

    // Create transport in client mode
    let mut transport = SshTransport::new(false);

    // --- Version exchange ---
    let version_line = transport.version_line();
    if stack.tcp_send(conn, &version_line).is_err() {
        shell.println("ssh: failed to send version");
        stack.tcp_close(conn);
        return;
    }

    // Read server version line
    let mut version_buf = Vec::new();
    let server_version_ok = 'version: {
        for _ in 0..30 {
            let mut buf = [0u8; 256];
            match stack.tcp_recv(conn, &mut buf, 500) {
                Ok(n) if n > 0 => {
                    version_buf.extend_from_slice(&buf[..n]);
                    if let Some(newline_pos) = version_buf.iter().position(|&b| b == b'\n') {
                        // Parse only up to the newline (version line may be followed by binary data)
                        let line = core::str::from_utf8(&version_buf[..newline_pos]).unwrap_or("");
                        if transport.set_peer_version(line) {
                            // Keep any remaining data after the version line for packet parsing
                            let remaining = version_buf[newline_pos + 1..].to_vec();
                            if !remaining.is_empty() {
                                transport.recv_buffer.extend_from_slice(&remaining);
                            }
                            break 'version true;
                        } else {
                            shell.println("ssh: invalid server version");
                            break 'version false;
                        }
                    }
                }
                Err(NetError::TimedOut) => continue,
                _ => {
                    break 'version false;
                }
            }
        }
        false
    };

    if !server_version_ok {
        if transport.peer_version.is_empty() {
            shell.println("ssh: no version from server");
        }
        stack.tcp_close(conn);
        return;
    }

    shell.print("Connected to ");
    shell.println(&transport.peer_version);

    // --- Key Exchange ---
    // Build and send our KEXINIT
    let our_kexinit_payload = kex::build_kexinit();
    let our_kexinit_raw = our_kexinit_payload.clone(); // save for hash computation
    let kexinit_pkt = transport.encode_packet(&our_kexinit_payload);
    if stack.tcp_send(conn, &kexinit_pkt).is_err() {
        shell.println("ssh: failed to send KEXINIT");
        stack.tcp_close(conn);
        return;
    }

    // Generate ephemeral X25519 keypair
    let (eph_public, eph_secret) = crypto::x25519_generate_keypair();

    // Wait for server KEXINIT
    // First, check if KEXINIT was already buffered with the version line
    let mut peer_kexinit = Vec::new();
    let buffered = transport.feed(&[]);
    for payload in buffered {
        if !payload.is_empty() && payload[0] == packet::msg::KEXINIT {
            peer_kexinit = payload;
        }
    }
    // If not buffered, read from TCP
    if peer_kexinit.is_empty() {
        for _ in 0..50 {
            let mut buf = [0u8; 4096];
            match stack.tcp_recv(conn, &mut buf, 500) {
                Ok(n) if n > 0 => {
                    let payloads = transport.feed(&buf[..n]);
                    for payload in payloads {
                        if !payload.is_empty() && payload[0] == packet::msg::KEXINIT {
                            peer_kexinit = payload;
                        }
                    }
                }
                _ => {}
            }
            if !peer_kexinit.is_empty() {
                break;
            }
        }
    }

    if peer_kexinit.is_empty() {
        shell.println("ssh: no KEXINIT from server");
        stack.tcp_close(conn);
        return;
    }

    // Send KEX_ECDH_INIT with our ephemeral public key
    let mut kex_init_msg = Vec::new();
    kex_init_msg.push(packet::msg::KEX_ECDH_INIT);
    kex_init_msg.extend_from_slice(&packet::encode_string(&eph_public.to_bytes()));
    let pkt = transport.encode_packet(&kex_init_msg);
    if stack.tcp_send(conn, &pkt).is_err() {
        shell.println("ssh: failed to send KEX_ECDH_INIT");
        stack.tcp_close(conn);
        return;
    }

    // Wait for KEX_ECDH_REPLY and NEWKEYS
    let mut got_reply = false;
    let mut got_newkeys = false;
    for _ in 0..50 {
        let mut buf = [0u8; 4096];
        match stack.tcp_recv(conn, &mut buf, 500) {
            Ok(n) if n > 0 => {
                let payloads = transport.feed(&buf[..n]);
                for payload in payloads {
                    if payload.is_empty() {
                        continue;
                    }
                    match payload[0] {
                        packet::msg::KEX_ECDH_REPLY => {
                            let mut offset = 1;
                            let (host_key_blob, off) = match packet::decode_string(&payload, offset) {
                                Some(x) => x,
                                None => continue,
                            };
                            offset = off;
                            let (server_eph_pub_bytes, off) = match packet::decode_string(&payload, offset) {
                                Some(x) => x,
                                None => continue,
                            };
                            offset = off;
                            // Signature blob (we skip verification for in-game use)
                            let _signature = packet::decode_string(&payload, offset);

                            if server_eph_pub_bytes.len() == 32 {
                                let mut server_pub = [0u8; 32];
                                server_pub.copy_from_slice(server_eph_pub_bytes);
                                let server_x25519 = x25519_dalek::PublicKey::from(server_pub);
                                let shared_secret = crypto::x25519_diffie_hellman(&eph_secret, &server_x25519);

                                let hash = kex::compute_exchange_hash(
                                    &transport.our_version,
                                    &transport.peer_version,
                                    &our_kexinit_raw,
                                    &peer_kexinit,
                                    host_key_blob,
                                    &eph_public.to_bytes(),
                                    &server_pub,
                                    &shared_secret,
                                );

                                transport.set_keys(&shared_secret, &hash);
                                got_reply = true;
                            }
                        }
                        packet::msg::NEWKEYS => {
                            // Server sent NEWKEYS, send ours back
                            let newkeys_pkt = transport.encode_packet(&[packet::msg::NEWKEYS]);
                            let _ = stack.tcp_send(conn, &newkeys_pkt);
                            got_newkeys = true;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        if got_reply && got_newkeys {
            break;
        }
    }

    if !got_reply || !got_newkeys {
        shell.println("ssh: key exchange failed");
        stack.tcp_close(conn);
        return;
    }

    // --- Service request ---
    let mut service_req = Vec::new();
    service_req.push(packet::msg::SERVICE_REQUEST);
    service_req.extend_from_slice(&packet::encode_string(b"ssh-userauth"));
    let pkt = transport.encode_packet(&service_req);
    if stack.tcp_send(conn, &pkt).is_err() {
        shell.println("ssh: failed to request service");
        stack.tcp_close(conn);
        return;
    }

    // Wait for SERVICE_ACCEPT
    let mut service_accepted = false;
    for _ in 0..30 {
        let mut buf = [0u8; 4096];
        match stack.tcp_recv(conn, &mut buf, 500) {
            Ok(n) if n > 0 => {
                let payloads = transport.feed(&buf[..n]);
                for payload in payloads {
                    if !payload.is_empty() && payload[0] == packet::msg::SERVICE_ACCEPT {
                        service_accepted = true;
                    }
                }
            }
            _ => {}
        }
        if service_accepted {
            break;
        }
    }

    if !service_accepted {
        shell.println("ssh: service request denied");
        stack.tcp_close(conn);
        return;
    }

    // --- Password authentication ---
    let password = match password {
        Some(p) => p.to_string(),
        None => shell.read_line(&format!("{}@{}'s password: ", username, host)),
    };

    let mut auth_req = Vec::new();
    auth_req.push(packet::msg::USERAUTH_REQUEST);
    auth_req.extend_from_slice(&packet::encode_string(username.as_bytes()));
    auth_req.extend_from_slice(&packet::encode_string(b"ssh-connection"));
    auth_req.extend_from_slice(&packet::encode_string(b"password"));
    auth_req.push(0); // not a password change request
    auth_req.extend_from_slice(&packet::encode_string(password.as_bytes()));
    let pkt = transport.encode_packet(&auth_req);
    if stack.tcp_send(conn, &pkt).is_err() {
        shell.println("ssh: failed to send auth request");
        stack.tcp_close(conn);
        return;
    }

    // Wait for auth response
    let mut authenticated = false;
    for _ in 0..30 {
        let mut buf = [0u8; 4096];
        match stack.tcp_recv(conn, &mut buf, 500) {
            Ok(n) if n > 0 => {
                let payloads = transport.feed(&buf[..n]);
                for payload in payloads {
                    if payload.is_empty() {
                        continue;
                    }
                    match payload[0] {
                        packet::msg::USERAUTH_SUCCESS => {
                            authenticated = true;
                        }
                        packet::msg::USERAUTH_FAILURE => {
                            shell.println("ssh: authentication failed");
                            stack.tcp_close(conn);
                            return;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        if authenticated {
            break;
        }
    }

    if !authenticated {
        shell.println("ssh: authentication timed out");
        stack.tcp_close(conn);
        return;
    }

    shell.println("Authenticated.");

    // --- Open session channel ---
    let local_channel_id: u32 = 0;
    let mut chan_open = Vec::new();
    chan_open.push(packet::msg::CHANNEL_OPEN);
    chan_open.extend_from_slice(&packet::encode_string(b"session"));
    chan_open.extend_from_slice(&local_channel_id.to_be_bytes()); // sender channel
    chan_open.extend_from_slice(&32768u32.to_be_bytes()); // initial window
    chan_open.extend_from_slice(&32768u32.to_be_bytes()); // max packet size
    let pkt = transport.encode_packet(&chan_open);
    if stack.tcp_send(conn, &pkt).is_err() {
        shell.println("ssh: failed to open channel");
        stack.tcp_close(conn);
        return;
    }

    // Wait for CHANNEL_OPEN_CONFIRMATION
    let mut remote_channel_id: u32 = 0;
    let mut channel_open = false;
    for _ in 0..30 {
        let mut buf = [0u8; 4096];
        match stack.tcp_recv(conn, &mut buf, 500) {
            Ok(n) if n > 0 => {
                let payloads = transport.feed(&buf[..n]);
                for payload in payloads {
                    if !payload.is_empty() && payload[0] == packet::msg::CHANNEL_OPEN_CONFIRMATION {
                        // recipient_channel (ours), sender_channel (remote), window, max_packet
                        if let Some((_our_id, offset)) = packet::decode_u32(&payload, 1) {
                            if let Some((rid, _offset)) = packet::decode_u32(&payload, offset) {
                                remote_channel_id = rid;
                                channel_open = true;
                            }
                        }
                    }
                    if !payload.is_empty() && payload[0] == packet::msg::CHANNEL_OPEN_FAILURE {
                        shell.println("ssh: server refused channel open");
                        stack.tcp_close(conn);
                        return;
                    }
                }
            }
            _ => {}
        }
        if channel_open {
            break;
        }
    }

    if !channel_open {
        shell.println("ssh: channel open timed out");
        stack.tcp_close(conn);
        return;
    }

    // Request PTY
    let mut pty_req = Vec::new();
    pty_req.push(packet::msg::CHANNEL_REQUEST);
    pty_req.extend_from_slice(&remote_channel_id.to_be_bytes());
    pty_req.extend_from_slice(&packet::encode_string(b"pty-req"));
    pty_req.push(1); // want reply
    pty_req.extend_from_slice(&packet::encode_string(b"xterm"));
    pty_req.extend_from_slice(&80u32.to_be_bytes());  // width chars
    pty_req.extend_from_slice(&24u32.to_be_bytes());  // height rows
    pty_req.extend_from_slice(&0u32.to_be_bytes());   // width pixels
    pty_req.extend_from_slice(&0u32.to_be_bytes());   // height pixels
    pty_req.extend_from_slice(&packet::encode_string(b"")); // terminal modes (empty)
    let pkt = transport.encode_packet(&pty_req);
    let _ = stack.tcp_send(conn, &pkt);

    // Request shell
    let mut shell_req = Vec::new();
    shell_req.push(packet::msg::CHANNEL_REQUEST);
    shell_req.extend_from_slice(&remote_channel_id.to_be_bytes());
    shell_req.extend_from_slice(&packet::encode_string(b"shell"));
    shell_req.push(1); // want reply
    let pkt = transport.encode_packet(&shell_req);
    let _ = stack.tcp_send(conn, &pkt);

    // Wait for channel success replies and any initial output
    for _ in 0..20 {
        let mut buf = [0u8; 4096];
        match stack.tcp_recv(conn, &mut buf, 300) {
            Ok(n) if n > 0 => {
                let payloads = transport.feed(&buf[..n]);
                for payload in payloads {
                    if payload.is_empty() {
                        continue;
                    }
                    match payload[0] {
                        packet::msg::CHANNEL_DATA => {
                            if let Some((_ch, data)) = channel::parse_channel_data(&payload) {
                                if let Ok(text) = core::str::from_utf8(data) {
                                    shell.print(text);
                                }
                            }
                        }
                        packet::msg::CHANNEL_SUCCESS | packet::msg::CHANNEL_FAILURE => {
                            // ok, continue
                        }
                        _ => {}
                    }
                }
            }
            _ => break,
        }
    }

    // --- Interactive loop ---
    // Since read_line blocks, we use a line-at-a-time approach:
    // 1. Read a line from the user
    // 2. Send it as CHANNEL_DATA (with newline appended)
    // 3. Read and display any response data
    shell.println("SSH session ready. Type 'exit' to disconnect.\n");

    loop {
        // Read a line from the local terminal
        let line = shell.read_line("");

        // Check for disconnect
        if line.is_empty() {
            // Could be interrupt (Ctrl+T) — just disconnect
            shell.println("\r\nConnection closed.");
            break;
        }

        // Send the line + newline as channel data
        let mut data_to_send = line.as_bytes().to_vec();
        data_to_send.push(b'\n');

        let chan_data = channel::build_channel_data(remote_channel_id, &data_to_send);
        let pkt = transport.encode_packet(&chan_data);
        if stack.tcp_send(conn, &pkt).is_err() {
            shell.println("\r\nConnection lost.");
            break;
        }

        // Read response data from server (with retries to get all output)
        let mut got_close = false;
        for _ in 0..20 {
            let mut buf = [0u8; 4096];
            match stack.tcp_recv(conn, &mut buf, 500) {
                Ok(n) if n > 0 => {
                    let payloads = transport.feed(&buf[..n]);
                    for payload in payloads {
                        if payload.is_empty() {
                            continue;
                        }
                        match payload[0] {
                            packet::msg::CHANNEL_DATA => {
                                if let Some((_ch, data)) = channel::parse_channel_data(&payload) {
                                    if let Ok(text) = core::str::from_utf8(data) {
                                        shell.print(text);
                                    }
                                }
                            }
                            packet::msg::CHANNEL_WINDOW_ADJUST => {
                                // Window adjusted, continue reading
                            }
                            packet::msg::CHANNEL_EOF | packet::msg::CHANNEL_CLOSE => {
                                got_close = true;
                            }
                            packet::msg::DISCONNECT => {
                                got_close = true;
                            }
                            _ => {}
                        }
                    }
                }
                Ok(_) => break, // EOF / 0 bytes
                Err(NetError::TimedOut) => break, // no more data right now
                Err(_) => {
                    got_close = true;
                    break;
                }
            }
        }

        if got_close {
            shell.println("\r\nConnection closed by remote host.");
            break;
        }
    }

    // Send channel close + disconnect
    let close_pkt = channel::build_channel_close(remote_channel_id);
    let pkt = transport.encode_packet(&close_pkt);
    let _ = stack.tcp_send(conn, &pkt);

    stack.tcp_close(conn);
}
