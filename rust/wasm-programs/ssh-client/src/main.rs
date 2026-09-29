//! SSH client — standalone WASI program.
//!
//! Connects to a remote SSH server over a kernel-proxied TCP socket
//! (`ecm_host_abi::socket`), authenticates, opens a session channel, and
//! relays between local stdin/stdout and the remote shell. Interface, route
//! and DNS configuration is owned by the kernel.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use ecm_host_abi::socket::{self, SockAddrIn, AF_INET, SOCK_STREAM, SOL_SOCKET, SO_RCVTIMEO};
use ecm_ssh_protocol::{packet, transport::SshTransport, kex, channel};
use ecm_ssh_crypto as crypto;

/// Overall timeout for each handshake step.
const HANDSHAKE_TIMEOUT_MS: i32 = 10_000;
/// Receive timeout while relaying, so the loop wakes up to service stdin.
const RELAY_POLL_MS: i32 = 50;

/// Result of one `recv` on the socket.
enum Recv {
    Data(usize),
    /// SO_RCVTIMEO elapsed with no data (-2).
    Timeout,
    /// Orderly EOF (0) or error/reset (-1).
    Closed,
}

/// A connected TCP socket plus the queue of SSH payloads decoded but not yet
/// consumed (one TCP read may yield several packets).
struct Conn {
    fd: i32,
    pending: VecDeque<Vec<u8>>,
}

impl Conn {
    /// Set SO_RCVTIMEO (4-byte little-endian milliseconds).
    fn set_recv_timeout(&self, ms: i32) {
        socket::setsockopt(self.fd, SOL_SOCKET, SO_RCVTIMEO, &ms.to_le_bytes());
    }

    /// Send the whole buffer, looping over partial sends. Chunked so each
    /// socket IPC request stays small.
    fn send(&self, mut data: &[u8]) -> bool {
        const CHUNK: usize = 1400;
        while !data.is_empty() {
            let n = socket::send(self.fd, &data[..data.len().min(CHUNK)], 0);
            if n <= 0 {
                return false;
            }
            data = &data[n as usize..];
        }
        true
    }

    fn recv(&self, buf: &mut [u8]) -> Recv {
        match socket::recv(self.fd, buf, 0) {
            n if n > 0 => Recv::Data(n as usize),
            -2 => Recv::Timeout,
            _ => Recv::Closed,
        }
    }

    fn close(&self) {
        socket::close(self.fd);
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("ssh: {}", msg);
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1] == "-h" || args[1] == "--help" {
        eprintln!("Usage: ssh [user[:password]@]host[:port]");
        eprintln!("  Uses this computer's network configuration (ifconfig/ip/route).");
        std::process::exit(1);
    }

    let (username, password, host, port) = parse_target(&args[1]);

    eprintln!("ssh: connecting to {}@{}:{}", username, host, port);

    // Resolve hostname (dotted-quad literals are used directly).
    let remote_addr = match parse_ipv4(&host) {
        Some(ip) => SockAddrIn::from_ip_port(ip, port),
        None => {
            let mut addr = SockAddrIn::default();
            if socket::getaddrinfo(&host, &mut addr) != 0 {
                fail(&format!("failed to resolve {}", host));
            }
            SockAddrIn::from_ip_port(addr.ip(), port)
        }
    };

    // Connect
    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    if fd < 0 {
        fail("failed to create socket");
    }
    if socket::connect(fd, &remote_addr) != 0 {
        socket::close(fd);
        fail(&format!("connection to {} failed", remote_addr.to_string()));
    }
    let mut conn = Conn { fd, pending: VecDeque::new() };
    // Short poll interval; handshake steps enforce their own overall deadline.
    conn.set_recv_timeout(RELAY_POLL_MS);

    // SSH handshake
    let mut transport = SshTransport::new(false);

    // 1. Send version
    let version_line = transport.version_line();
    let _ = conn.send(&version_line);

    // 2. Read server version
    let mut ver_buf: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    let deadline = Instant::now() + Duration::from_millis(HANDSHAKE_TIMEOUT_MS as u64);
    loop {
        match conn.recv(&mut buf) {
            Recv::Data(n) => {
                ver_buf.extend_from_slice(&buf[..n]);
                if let Some(pos) = ver_buf.iter().position(|&b| b == b'\n') {
                    let line = std::str::from_utf8(&ver_buf[..pos]).unwrap_or("");
                    if !transport.set_peer_version(line) {
                        conn.close();
                        fail("invalid server version");
                    }
                    let remaining = ver_buf[pos + 1..].to_vec();
                    if !remaining.is_empty() {
                        transport.recv_buffer.extend_from_slice(&remaining);
                    }
                    break;
                }
            }
            Recv::Timeout if Instant::now() < deadline => {}
            Recv::Timeout => {
                conn.close();
                fail("timeout waiting for server version");
            }
            Recv::Closed => {
                conn.close();
                fail("connection closed before server version");
            }
        }
    }

    // 3. Send KEXINIT
    let our_kexinit_payload = kex::build_kexinit();
    let kexinit_pkt = transport.build_kexinit_packet();
    let _ = conn.send(&kexinit_pkt);

    // 4. Read server KEXINIT
    let server_kexinit = read_next_payload(&mut conn, &mut transport);
    if server_kexinit.is_empty() || server_kexinit[0] != packet::msg::KEXINIT {
        conn.close();
        fail("expected KEXINIT");
    }

    // 5. Generate ephemeral keypair and send KEX_ECDH_INIT
    let (client_epub, client_secret) = crypto::x25519_generate_keypair();
    let client_epub_bytes = client_epub.to_bytes();
    let mut kex_init_payload = Vec::new();
    kex_init_payload.push(packet::msg::KEX_ECDH_INIT);
    kex_init_payload.extend_from_slice(&packet::encode_string(&client_epub_bytes));
    let pkt = transport.encode_packet(&kex_init_payload);
    let _ = conn.send(&pkt);

    // 6. Read KEX_ECDH_REPLY
    let kex_reply = read_next_payload(&mut conn, &mut transport);
    if kex_reply.is_empty() || kex_reply[0] != packet::msg::KEX_ECDH_REPLY {
        conn.close();
        fail("expected KEX_ECDH_REPLY");
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
    let newkeys = read_next_payload(&mut conn, &mut transport);
    if newkeys.is_empty() || newkeys[0] != packet::msg::NEWKEYS {
        conn.close();
        fail("expected NEWKEYS");
    }

    let newkeys_pkt = transport.encode_packet(&[packet::msg::NEWKEYS]);
    let _ = conn.send(&newkeys_pkt);
    transport.set_keys(&shared_secret, &exchange_hash);

    // 8. Request ssh-userauth service
    let mut service_req = Vec::new();
    service_req.push(packet::msg::SERVICE_REQUEST);
    service_req.extend_from_slice(&packet::encode_string(b"ssh-userauth"));
    let pkt = transport.encode_packet(&service_req);
    let _ = conn.send(&pkt);

    let service_accept = read_next_payload(&mut conn, &mut transport);
    if service_accept.is_empty() || service_accept[0] != packet::msg::SERVICE_ACCEPT {
        conn.close();
        fail("service accept failed");
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
    let _ = conn.send(&pkt);

    let auth_response = read_next_payload(&mut conn, &mut transport);
    if auth_response.is_empty() || auth_response[0] != packet::msg::USERAUTH_SUCCESS {
        conn.close();
        fail("authentication failed");
    }

    // 10. Open session channel
    let local_channel_id: u32 = 0;
    let chan_open = channel::build_channel_open_session(local_channel_id, 32768, 32768);
    let pkt = transport.encode_packet(&chan_open);
    let _ = conn.send(&pkt);

    let chan_confirm = read_next_payload(&mut conn, &mut transport);
    let remote_channel_id = match channel::parse_channel_open_confirmation(&chan_confirm) {
        Some((_, remote_id, _, _)) => remote_id,
        None => {
            conn.close();
            fail("channel open failed");
        }
    };

    // 11. Request PTY
    let pty_req = channel::build_pty_request(remote_channel_id, "xterm", 80, 24, 0, 0);
    let pkt = transport.encode_packet(&pty_req);
    let _ = conn.send(&pkt);
    let _ = read_next_payload(&mut conn, &mut transport);

    // 12. Request shell
    let shell_req = channel::build_shell_request(remote_channel_id);
    let pkt = transport.encode_packet(&shell_req);
    let _ = conn.send(&pkt);
    let _ = read_next_payload(&mut conn, &mut transport);

    eprintln!("ssh: connected (Ctrl+T to disconnect)");

    // 13. Relay loop
    let mut stdout = io::stdout();

    loop {
        // Payloads already decoded during the handshake come first.
        let mut payloads: Vec<Vec<u8>> = conn.pending.drain(..).collect();
        match conn.recv(&mut buf) {
            Recv::Data(n) => payloads.extend(transport.feed(&buf[..n])),
            Recv::Timeout => {}
            Recv::Closed => {
                eprintln!("\r\nssh: connection lost");
                break;
            }
        }

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
                    conn.close();
                    return;
                }
                packet::msg::CHANNEL_WINDOW_ADJUST => {}
                packet::msg::DISCONNECT => {
                    eprintln!("\r\nssh: disconnected");
                    conn.close();
                    return;
                }
                _ => {}
            }
        }

        // Read from stdin
        let mut input_buf = [0u8; 256];
        match io::stdin().read(&mut input_buf) {
            Ok(0) => {}
            Ok(n) => {
                if input_buf[..n].contains(&0x14) {
                    eprintln!("\r\nssh: disconnected (Ctrl+T)");
                    let close = channel::build_channel_close(remote_channel_id);
                    let pkt = transport.encode_packet(&close);
                    let _ = conn.send(&pkt);
                    conn.close();
                    return;
                }

                let chan_data = channel::build_channel_data(remote_channel_id, &input_buf[..n]);
                let pkt = transport.encode_packet(&chan_data);
                if !conn.send(&pkt) {
                    eprintln!("\r\nssh: send failed");
                    break;
                }
            }
            Err(_) => {}
        }
    }

    conn.close();
}

/// Return the next SSH payload, reading from the socket as needed. Returns an
/// empty payload on timeout or connection loss.
fn read_next_payload(conn: &mut Conn, transport: &mut SshTransport) -> Vec<u8> {
    if let Some(p) = conn.pending.pop_front() {
        return p;
    }

    let mut buf = [0u8; 4096];
    let deadline = Instant::now() + Duration::from_millis(HANDSHAKE_TIMEOUT_MS as u64);
    loop {
        // Try decoding from existing transport buffer
        conn.pending.extend(transport.feed(&[]));
        if let Some(p) = conn.pending.pop_front() {
            return p;
        }

        match conn.recv(&mut buf) {
            Recv::Data(n) => {
                conn.pending.extend(transport.feed(&buf[..n]));
                if let Some(p) = conn.pending.pop_front() {
                    return p;
                }
            }
            Recv::Timeout if Instant::now() < deadline => {}
            Recv::Timeout | Recv::Closed => return Vec::new(),
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

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 { return None; }
    let mut ip = [0u8; 4];
    for (o, p) in ip.iter_mut().zip(parts) {
        *o = p.parse().ok()?;
    }
    Some(ip)
}
