//! Socket IPC tests: two real `Stack`s wired back to back, driven through
//! `SocketIpc::dispatch` exactly as the host would.

use super::*;
use ecm_net::types::MacAddr;
use ecm_net::StackConfig;

struct Pair {
    a: Stack,
    b: Stack,
    now: i64,
}

impl Pair {
    fn new() -> Self {
        let mut a = Stack::new(StackConfig { seed: 1 });
        let mut b = Stack::new(StackConfig { seed: 2 });
        a.add_interface("eth0", MacAddr([2, 0, 0, 0, 0, 1]));
        b.add_interface("eth0", MacAddr([2, 0, 0, 0, 0, 2]));
        a.configure_addr(0, Ipv4Addr::new(10, 0, 0, 1), 24, 0);
        b.configure_addr(0, Ipv4Addr::new(10, 0, 0, 2), 24, 0);
        Self { a, b, now: 0 }
    }

    /// Move frames both ways and advance virtual time by `ms`.
    fn run(&mut self, ms: i64) {
        let end = self.now + ms;
        while self.now < end {
            for _ in 0..64 {
                let mut moved = false;
                while let Some((_, f)) = self.a.pop_tx() {
                    self.b.handle_frame(0, &f, self.now);
                    moved = true;
                }
                while let Some((_, f)) = self.b.pop_tx() {
                    self.a.handle_frame(0, &f, self.now);
                    moved = true;
                }
                if !moved {
                    break;
                }
            }
            self.a.poll(self.now);
            self.b.poll(self.now);
            self.now += 5;
        }
    }
}

/// Returns (raw return, status, payload).
fn call(ipc: &mut SocketIpc, stack: &mut Stack, pid: i32, sys: i32, args: &[u8], now: i64) -> (i32, i32, Vec<u8>) {
    let mut res = vec![0u8; 2048];
    let mut fx = IpcEffects::default();
    let r = ipc.dispatch(stack, pid, sys, args, &mut res, now, &mut fx);
    if r == IPC_PENDING {
        return (r, 0, Vec::new());
    }
    assert!(r >= 4, "result must carry a status word, got {}", r);
    let status = i32::from_le_bytes([res[0], res[1], res[2], res[3]]);
    (r, status, res[4..r as usize].to_vec())
}

fn sockaddr(ip: [u8; 4], port: u16) -> Vec<u8> {
    sockaddr_bytes(&SocketAddr { ip: Ipv4Addr(ip), port }).to_vec()
}

fn i32s(v: &[i32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn with_sock(id: i32, tail: &[u8]) -> Vec<u8> {
    let mut v = id.to_le_bytes().to_vec();
    v.extend_from_slice(tail);
    v
}

fn blob(data: &[u8]) -> Vec<u8> {
    let mut v = (data.len() as u16).to_le_bytes().to_vec();
    v.extend_from_slice(data);
    v
}

#[test]
fn tcp_connect_accept_send_recv_close_through_ipc() {
    let mut p = Pair::new();
    let (mut ia, mut ib) = (SocketIpc::new(), SocketIpc::new());

    // Server on B: socket / bind / listen, then accept is pending.
    let (_, srv, _) = call(&mut ib, &mut p.b, 7, SOCK_SOCKET, &i32s(&[AF_INET, SOCK_STREAM, 0]), p.now);
    assert!(srv >= 0);
    assert_eq!(call(&mut ib, &mut p.b, 7, SOCK_BIND, &with_sock(srv, &sockaddr([0, 0, 0, 0], 8080)), p.now).1, 0);
    assert_eq!(call(&mut ib, &mut p.b, 7, SOCK_LISTEN, &i32s(&[srv, 4]), p.now).1, 0);
    assert_eq!(call(&mut ib, &mut p.b, 7, SOCK_ACCEPT, &i32s(&[srv]), p.now).0, IPC_PENDING);

    // Client on A: connect stays pending until the handshake completes.
    let (_, cli, _) = call(&mut ia, &mut p.a, 3, SOCK_SOCKET, &i32s(&[AF_INET, SOCK_STREAM, 0]), p.now);
    let conn = with_sock(cli, &sockaddr([10, 0, 0, 2], 8080));
    assert_eq!(call(&mut ia, &mut p.a, 3, SOCK_CONNECT, &conn, p.now).0, IPC_PENDING);
    let mut connected = false;
    for _ in 0..200 {
        p.run(10);
        let (r, st, _) = call(&mut ia, &mut p.a, 3, SOCK_CONNECT, &conn, p.now);
        if r != IPC_PENDING {
            assert_eq!(st, 0, "connect failed");
            connected = true;
            break;
        }
    }
    assert!(connected, "connect never completed");
    let (_, acc, peer) = call(&mut ib, &mut p.b, 7, SOCK_ACCEPT, &i32s(&[srv]), p.now);
    assert!(acc >= 0 && peer.len() == 16, "accept should return a new fd and the peer sockaddr");
    assert_eq!(&peer[4..8], &[10, 0, 0, 1]);

    // recv with no data is pending; after a send it returns the bytes.
    let recv = with_sock(acc, &i32s(&[1024, 0]));
    assert_eq!(call(&mut ib, &mut p.b, 7, SOCK_RECV, &recv, p.now).0, IPC_PENDING);
    let (_, sent, _) = call(&mut ia, &mut p.a, 3, SOCK_SEND, &with_sock(cli, &blob(b"hello ipc")), p.now);
    assert_eq!(sent, 9);
    p.run(50);
    let (_, n, data) = call(&mut ib, &mut p.b, 7, SOCK_RECV, &recv, p.now);
    assert_eq!(n, 9);
    assert_eq!(&data[..9], b"hello ipc");

    // Client closes: the server sees EOF (status 0).
    assert_eq!(call(&mut ia, &mut p.a, 3, SOCK_CLOSE, &i32s(&[cli]), p.now).1, 0);
    p.run(100);
    assert_eq!(call(&mut ib, &mut p.b, 7, SOCK_RECV, &recv, p.now).1, 0, "EOF expected");
}

#[test]
fn rcvtimeo_turns_pending_accept_into_timeout() {
    let mut p = Pair::new();
    let mut ib = SocketIpc::new();
    let (_, srv, _) = call(&mut ib, &mut p.b, 1, SOCK_SOCKET, &i32s(&[AF_INET, SOCK_STREAM, 0]), 0);
    call(&mut ib, &mut p.b, 1, SOCK_BIND, &with_sock(srv, &sockaddr([0, 0, 0, 0], 99)), 0);
    call(&mut ib, &mut p.b, 1, SOCK_LISTEN, &i32s(&[srv, 1]), 0);
    // SO_RCVTIMEO = 200 ms, 4-byte millisecond encoding.
    let opt = with_sock(srv, &i32s(&[SOL_SOCKET, SO_RCVTIMEO, 200]));
    assert_eq!(call(&mut ib, &mut p.b, 1, SOCK_SETSOCKOPT, &opt, 0).1, 0);
    assert_eq!(call(&mut ib, &mut p.b, 1, SOCK_ACCEPT, &i32s(&[srv]), 0).0, IPC_PENDING);
    assert_eq!(ib.next_deadline(), Some(200));
    assert_eq!(call(&mut ib, &mut p.b, 1, SOCK_ACCEPT, &i32s(&[srv]), 150).0, IPC_PENDING);
    assert_eq!(call(&mut ib, &mut p.b, 1, SOCK_ACCEPT, &i32s(&[srv]), 200).1, -2);
    assert!(!ib.has_pending());
}

#[test]
fn timeval_rcvtimeo_encoding_is_accepted() {
    let mut p = Pair::new();
    let mut ia = SocketIpc::new();
    let (_, u, _) = call(&mut ia, &mut p.a, 1, SOCK_SOCKET, &i32s(&[AF_INET, SOCK_DGRAM, 0]), 0);
    // struct timeval { i64 sec = 1, i64 usec = 500_000 } => 1500 ms
    let mut tv = Vec::new();
    tv.extend_from_slice(&1i64.to_le_bytes());
    tv.extend_from_slice(&500_000i64.to_le_bytes());
    let mut args = with_sock(u, &i32s(&[SOL_SOCKET, SO_RCVTIMEO]));
    args.extend_from_slice(&tv);
    assert_eq!(call(&mut ia, &mut p.a, 1, SOCK_SETSOCKOPT, &args, 0).1, 0);
    let rf = with_sock(u, &i32s(&[256, 0]));
    assert_eq!(call(&mut ia, &mut p.a, 1, SOCK_RECVFROM, &rf, 0).0, IPC_PENDING);
    assert_eq!(ia.next_deadline(), Some(1500));
}

#[test]
fn udp_recvfrom_times_out_with_legacy_zero_and_destroy_frees_everything() {
    let mut p = Pair::new();
    let mut ia = SocketIpc::new();
    let (_, u, _) = call(&mut ia, &mut p.a, 5, SOCK_SOCKET, &i32s(&[AF_INET, SOCK_DGRAM, 0]), 0);
    let rf = with_sock(u, &i32s(&[512, 0]));
    assert_eq!(call(&mut ia, &mut p.a, 5, SOCK_RECVFROM, &rf, 0).0, IPC_PENDING);
    assert_eq!(call(&mut ia, &mut p.a, 5, SOCK_RECVFROM, &rf, 5_000).1, 0);
    ia.destroy_session(&mut p.a, 5);
    assert!(!ia.has_pending());
    // A fresh session starts from slot 0 again.
    let (_, u2, _) = call(&mut ia, &mut p.a, 5, SOCK_SOCKET, &i32s(&[AF_INET, SOCK_DGRAM, 0]), 0);
    assert_eq!(u2, 0);
}

#[test]
fn udp_datagram_roundtrip_with_source_address() {
    let mut p = Pair::new();
    let (mut ia, mut ib) = (SocketIpc::new(), SocketIpc::new());
    let (_, ub, _) = call(&mut ib, &mut p.b, 2, SOCK_SOCKET, &i32s(&[AF_INET, SOCK_DGRAM, 0]), 0);
    call(&mut ib, &mut p.b, 2, SOCK_BIND, &with_sock(ub, &sockaddr([0, 0, 0, 0], 5353)), 0);
    let (_, ua, _) = call(&mut ia, &mut p.a, 1, SOCK_SOCKET, &i32s(&[AF_INET, SOCK_DGRAM, 0]), 0);
    let mut send = with_sock(ua, &blob(&sockaddr([10, 0, 0, 2], 5353)));
    send.extend_from_slice(&blob(b"ping!"));
    assert_eq!(call(&mut ia, &mut p.a, 1, SOCK_SENDTO, &send, 0).1, 5);
    p.run(100);
    let (_, n, payload) = call(&mut ib, &mut p.b, 2, SOCK_RECVFROM, &with_sock(ub, &i32s(&[64, 0])), p.now);
    assert_eq!(n, 5);
    assert_eq!(&payload[4..8], &[10, 0, 0, 1], "source address");
    assert_eq!(&payload[16..21], b"ping!");
}

#[test]
fn hostile_arguments_never_panic() {
    let mut p = Pair::new();
    let mut ia = SocketIpc::new();
    let mut x: u64 = 0xdead_beef_cafe_f00d;
    for i in 0..20_000i64 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let len = (x % 64) as usize;
        let args: Vec<u8> = (0..len).map(|k| (x >> (k % 56)) as u8).collect();
        let sys = ((x >> 8) % 16) as i32;
        let mut res = vec![0u8; ((x >> 20) % 64) as usize];
        let mut fx = IpcEffects::default();
        ia.dispatch(&mut p.a, (i % 3) as i32, sys, &args, &mut res, i, &mut fx);
    }
}

#[test]
fn getaddrinfo_literal_and_localhost_are_immediate() {
    let mut p = Pair::new();
    let mut ia = SocketIpc::new();
    let (_, st, sa) = call(&mut ia, &mut p.a, 1, SOCK_GETADDRINFO, &blob(b"10.0.0.2"), 0);
    assert_eq!(st, 16);
    assert_eq!(&sa[4..8], &[10, 0, 0, 2]);
    let (_, _, sa) = call(&mut ia, &mut p.a, 1, SOCK_GETADDRINFO, &blob(b"localhost"), 0);
    assert_eq!(&sa[4..8], &[127, 0, 0, 1]);
    // A real name with no reachable DNS server eventually fails, never hangs.
    let mut t = 0;
    loop {
        let (r, st, _) = call(&mut ia, &mut p.a, 1, SOCK_GETADDRINFO, &blob(b"example.com"), t);
        if r != IPC_PENDING {
            assert!(st < 0);
            break;
        }
        t += 500;
        p.a.poll(t);
        assert!(t < 30_000, "DNS query never gave up");
    }
}
