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
        let sys = ((x >> 8) % 17) as i32;
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

#[test]
fn every_syscall_on_every_socket_kind_survives_tiny_result_buffers() {
    let mut p = Pair::new();
    let mut ia = SocketIpc::new();
    let kinds = [
        i32s(&[AF_INET, SOCK_STREAM, 0]),
        i32s(&[AF_INET, SOCK_DGRAM, 0]),
        i32s(&[AF_INET, SOCK_RAW, IPPROTO_ICMP]),
        i32s(&[AF_NETLINK, SOCK_DGRAM, 0]),
        i32s(&[AF_PACKET, SOCK_RAW, 0x0300]),
        i32s(&[AF_PACKET, SOCK_RAW, 0x0008]),
    ];
    let mut ids = Vec::new();
    for k in &kinds {
        let (_, id, _) = call(&mut ia, &mut p.a, 1, SOCK_SOCKET, k, 0);
        assert!(id >= 0);
        ids.push(id);
    }
    // A listener too, so accept/recv paths on TCP kinds are reachable.
    let (_, l, _) = call(&mut ia, &mut p.a, 1, SOCK_SOCKET, &kinds[0], 0);
    call(&mut ia, &mut p.a, 1, SOCK_BIND, &with_sock(l, &sockaddr([0, 0, 0, 0], 1234)), 0);
    call(&mut ia, &mut p.a, 1, SOCK_LISTEN, &i32s(&[l, 2]), 0);
    ids.push(l);
    // One packet socket bound, one left unbound.
    let mut bind = with_sock(ids[4], &sockaddr_ll_bytes("eth0", 0));
    bind.truncate(4 + 16);
    call(&mut ia, &mut p.a, 1, SOCK_BIND, &bind, 0);
    for &id in &ids {
        for sys in 0..17 {
            if sys == SOCK_CLOSE {
                continue;
            }
            for cap in 0..32usize {
                let mut args = with_sock(id, &i32s(&[64, 0]));
                args.extend_from_slice(&blob(&sockaddr([10, 0, 0, 2], 9)));
                args.extend_from_slice(&blob(b"data"));
                let mut res = vec![0u8; cap];
                let mut fx = IpcEffects::default();
                ia.dispatch(&mut p.a, 1, sys, &args, &mut res, 0, &mut fx);
            }
        }
    }
}

// ------------------------------------------------------------ AF_PACKET

const ETH_LOCAL1: u16 = 0x88B5; // IEEE local experimental EtherTypes
const ETH_LOCAL2: u16 = 0x88B6;
const MAC_A: [u8; 6] = [2, 0, 0, 0, 0, 1];
const MAC_B: [u8; 6] = [2, 0, 0, 0, 0, 2];

/// socket(AF_PACKET, SOCK_RAW, htons(ethertype)) as a little-endian child
/// computes it.
fn packet_socket(ipc: &mut SocketIpc, stack: &mut Stack, pid: i32, ethertype: u16) -> i32 {
    let proto = ethertype.swap_bytes() as i32;
    call(ipc, stack, pid, SOCK_SOCKET, &i32s(&[AF_PACKET, SOCK_RAW, proto]), 0).1
}

fn sll(ifname: &str, proto: u16) -> Vec<u8> {
    sockaddr_ll_bytes(ifname, proto).to_vec()
}

fn bind_packet(ipc: &mut SocketIpc, stack: &mut Stack, pid: i32, sock: i32, ifname: &str) -> i32 {
    call(ipc, stack, pid, SOCK_BIND, &with_sock(sock, &sll(ifname, 0)), 0).1
}

fn frame(dst: [u8; 6], src: [u8; 6], ethertype: u16, body: &[u8]) -> Vec<u8> {
    let mut f = dst.to_vec();
    f.extend_from_slice(&src);
    f.extend_from_slice(&ethertype.to_be_bytes());
    f.extend_from_slice(body);
    f
}

/// recv with MSG_DONTWAIT: (status, data).
fn try_recv(ipc: &mut SocketIpc, stack: &mut Stack, pid: i32, sock: i32) -> (i32, Vec<u8>) {
    let (r, st, data) = call(ipc, stack, pid, SOCK_RECV, &i32s(&[sock, 2048, MSG_DONTWAIT]), 0);
    assert_ne!(r, IPC_PENDING, "MSG_DONTWAIT must never pend");
    (st, data)
}

#[test]
fn packet_socket_sends_and_receives_whole_frames_on_its_interface() {
    let mut p = Pair::new();
    let (mut ia, mut ib) = (SocketIpc::new(), SocketIpc::new());
    let sa = packet_socket(&mut ia, &mut p.a, 1, ETH_LOCAL1);
    let sb = packet_socket(&mut ib, &mut p.b, 1, ETH_LOCAL1);
    assert!(sa >= 0 && sb >= 0);
    assert_eq!(bind_packet(&mut ia, &mut p.a, 1, sa, "eth0"), 0);
    assert_eq!(bind_packet(&mut ib, &mut p.b, 1, sb, "eth0"), 0);
    // getsockname reports the binding.
    let (_, st, name) = call(&mut ib, &mut p.b, 1, SOCK_GETSOCKNAME, &i32s(&[sb]), 0);
    assert_eq!(st, 0);
    assert_eq!(name, sll("eth0", ETH_LOCAL1));

    // Blocking recv pends until a frame arrives.
    let recv = i32s(&[sb, 2048, 0]);
    assert_eq!(call(&mut ib, &mut p.b, 1, SOCK_RECV, &recv, p.now).0, IPC_PENDING);
    let f = frame(MAC_B, MAC_A, ETH_LOCAL1, b"hello packet socket, padded to 46 bytes......");
    let (_, sent, _) = call(&mut ia, &mut p.a, 1, SOCK_SEND, &with_sock(sa, &blob(&f)), p.now);
    assert_eq!(sent, f.len() as i32);
    p.run(10);
    let (_, n, data) = call(&mut ib, &mut p.b, 1, SOCK_RECV, &recv, p.now);
    assert_eq!(n, f.len() as i32);
    assert_eq!(data, f, "the whole frame arrives unchanged");
    assert!(!ib.has_pending());

    // sendto + recvfrom: the source address names interface and EtherType.
    let mut args = with_sock(sa, &blob(&sll("eth0", 0)));
    args.extend_from_slice(&blob(&f));
    assert_eq!(call(&mut ia, &mut p.a, 1, SOCK_SENDTO, &args, p.now).1, f.len() as i32);
    p.run(10);
    let (_, n, payload) = call(&mut ib, &mut p.b, 1, SOCK_RECVFROM, &i32s(&[sb, 2048, 0]), p.now);
    assert_eq!(n, f.len() as i32);
    assert_eq!(&payload[..16], &sll("eth0", ETH_LOCAL1)[..]);
    assert_eq!(&payload[16..], &f[..]);

    // A truncating read returns what fits.
    call(&mut ia, &mut p.a, 1, SOCK_SEND, &with_sock(sa, &blob(&f)), p.now);
    p.run(10);
    let (_, n, data) = call(&mut ib, &mut p.b, 1, SOCK_RECV, &i32s(&[sb, 20, 0]), p.now);
    assert_eq!((n, data.as_slice()), (20, &f[..20]));

    // Close frees the socket; further calls on the id fail.
    assert_eq!(call(&mut ib, &mut p.b, 1, SOCK_CLOSE, &i32s(&[sb]), p.now).1, 0);
    assert_eq!(try_recv(&mut ib, &mut p.b, 1, sb).0, -1);
}

#[test]
fn packet_sockets_filter_by_ethertype_and_eth_p_all_sees_everything() {
    let mut p = Pair::new();
    p.run(10); // deliver the gratuitous ARPs of the initial addresses
    let (mut ia, mut ib) = (SocketIpc::new(), SocketIpc::new());
    let tx = packet_socket(&mut ia, &mut p.a, 1, ETH_LOCAL1);
    bind_packet(&mut ia, &mut p.a, 1, tx, "eth0");
    let one = packet_socket(&mut ib, &mut p.b, 1, ETH_LOCAL1);
    let two = packet_socket(&mut ib, &mut p.b, 1, ETH_LOCAL2);
    let all = packet_socket(&mut ib, &mut p.b, 1, ecm_net::ETH_P_ALL);
    for s in [one, two, all] {
        assert_eq!(bind_packet(&mut ib, &mut p.b, 1, s, "eth0"), 0);
    }
    let f2 = frame(MAC_B, MAC_A, ETH_LOCAL2, &[7u8; 46]);
    call(&mut ia, &mut p.a, 1, SOCK_SEND, &with_sock(tx, &blob(&f2)), p.now);
    p.run(10);
    assert_eq!(try_recv(&mut ib, &mut p.b, 1, one).0, -2, "0x88B5 socket must not see 0x88B6");
    assert_eq!(try_recv(&mut ib, &mut p.b, 1, two).1, f2);
    assert_eq!(try_recv(&mut ib, &mut p.b, 1, all).1, f2);
    // Frames to some other unicast MAC are not delivered (not for this NIC).
    let other = frame([2, 9, 9, 9, 9, 9], MAC_A, ETH_LOCAL2, &[1u8; 46]);
    call(&mut ia, &mut p.a, 1, SOCK_SEND, &with_sock(tx, &blob(&other)), p.now);
    // Multicast (e.g. the EAPOL PAE group address) is.
    let group = frame([0x01, 0x80, 0xC2, 0, 0, 3], MAC_A, ETH_LOCAL2, &[2u8; 46]);
    call(&mut ia, &mut p.a, 1, SOCK_SEND, &with_sock(tx, &blob(&group)), p.now);
    p.run(10);
    assert_eq!(try_recv(&mut ib, &mut p.b, 1, two).1, group);
    assert_eq!(try_recv(&mut ib, &mut p.b, 1, two).0, -2);
    // Drain what ETH_P_ALL got so far, then make the stack talk.
    while try_recv(&mut ib, &mut p.b, 1, all).0 > 0 {}
    // ETH_P_ALL also sees the stack traffic (ARP + IP of a UDP send).
    let (_, u, _) = call(&mut ia, &mut p.a, 2, SOCK_SOCKET, &i32s(&[AF_INET, SOCK_DGRAM, 0]), 0);
    let mut send = with_sock(u, &blob(&sockaddr([10, 0, 0, 2], 9)));
    send.extend_from_slice(&blob(b"udp"));
    call(&mut ia, &mut p.a, 2, SOCK_SENDTO, &send, p.now);
    p.run(50);
    let mut types = Vec::new();
    loop {
        let (st, data) = try_recv(&mut ib, &mut p.b, 1, all);
        if st <= 0 {
            break;
        }
        types.push(u16::from_be_bytes([data[12], data[13]]));
    }
    assert!(types.contains(&0x0806), "ARP seen: {:x?}", types);
    assert!(types.contains(&0x0800), "IPv4 seen: {:x?}", types);
    assert_eq!(try_recv(&mut ib, &mut p.b, 1, two).0, -2);
}

#[test]
fn two_packet_sockets_both_get_a_copy_and_ip_still_works() {
    let mut p = Pair::new();
    let (mut ia, mut ib) = (SocketIpc::new(), SocketIpc::new());
    // Two processes on B each watch IPv4.
    let s1 = packet_socket(&mut ib, &mut p.b, 1, 0x0800);
    let s2 = packet_socket(&mut ib, &mut p.b, 2, 0x0800);
    bind_packet(&mut ib, &mut p.b, 1, s1, "eth0");
    bind_packet(&mut ib, &mut p.b, 2, s2, "eth0");
    // A normal UDP exchange A -> B still reaches B's UDP socket.
    let (_, ub, _) = call(&mut ib, &mut p.b, 3, SOCK_SOCKET, &i32s(&[AF_INET, SOCK_DGRAM, 0]), 0);
    call(&mut ib, &mut p.b, 3, SOCK_BIND, &with_sock(ub, &sockaddr([0, 0, 0, 0], 7000)), 0);
    let (_, ua, _) = call(&mut ia, &mut p.a, 1, SOCK_SOCKET, &i32s(&[AF_INET, SOCK_DGRAM, 0]), 0);
    let mut send = with_sock(ua, &blob(&sockaddr([10, 0, 0, 2], 7000)));
    send.extend_from_slice(&blob(b"still-ip"));
    assert_eq!(call(&mut ia, &mut p.a, 1, SOCK_SENDTO, &send, 0).1, 8);
    p.run(50);
    let (_, n, payload) = call(&mut ib, &mut p.b, 3, SOCK_RECVFROM, &with_sock(ub, &i32s(&[64, 0])), p.now);
    assert_eq!(n, 8);
    assert_eq!(&payload[16..], b"still-ip");
    // Both packet sockets saw the same IPv4 frame carrying it.
    let (st1, f1) = try_recv(&mut ib, &mut p.b, 1, s1);
    let (st2, f2) = try_recv(&mut ib, &mut p.b, 2, s2);
    assert!(st1 > 0 && st2 > 0);
    assert_eq!(f1, f2);
    assert!(f1.windows(8).any(|w| w == b"still-ip"));
    // And TCP through the same interface is unaffected.
    let (_, srv, _) = call(&mut ib, &mut p.b, 4, SOCK_SOCKET, &i32s(&[AF_INET, SOCK_STREAM, 0]), 0);
    call(&mut ib, &mut p.b, 4, SOCK_BIND, &with_sock(srv, &sockaddr([0, 0, 0, 0], 81)), 0);
    call(&mut ib, &mut p.b, 4, SOCK_LISTEN, &i32s(&[srv, 2]), 0);
    let (_, cli, _) = call(&mut ia, &mut p.a, 2, SOCK_SOCKET, &i32s(&[AF_INET, SOCK_STREAM, 0]), 0);
    let conn = with_sock(cli, &sockaddr([10, 0, 0, 2], 81));
    let mut connected = false;
    for _ in 0..100 {
        let (r, st, _) = call(&mut ia, &mut p.a, 2, SOCK_CONNECT, &conn, p.now);
        if r != IPC_PENDING {
            connected = st == 0;
            break;
        }
        p.run(10);
    }
    assert!(connected, "TCP connect works with packet sockets open");
}

#[test]
fn unbound_or_invalid_packet_sockets_are_rejected() {
    let mut p = Pair::new();
    let mut ia = SocketIpc::new();
    let s = packet_socket(&mut ia, &mut p.a, 1, ETH_LOCAL1);
    let f = frame(MAC_B, MAC_A, ETH_LOCAL1, &[0u8; 46]);
    while p.a.pop_tx().is_some() {}
    // Unbound: send and recv fail at once (no frame leaves).
    assert_eq!(call(&mut ia, &mut p.a, 1, SOCK_SEND, &with_sock(s, &blob(&f)), 0).1, -1);
    assert!(p.a.pop_tx().is_none());
    assert_eq!(call(&mut ia, &mut p.a, 1, SOCK_RECV, &i32s(&[s, 2048, 0]), 0).1, -1);
    // Unknown interface or an AF_INET address: rejected.
    assert_eq!(bind_packet(&mut ia, &mut p.a, 1, s, "eth7"), -1);
    assert_eq!(call(&mut ia, &mut p.a, 1, SOCK_BIND, &with_sock(s, &sockaddr([0, 0, 0, 0], 0)), 0).1, -1);
    // Bound now; a runt frame is still rejected.
    assert_eq!(bind_packet(&mut ia, &mut p.a, 1, s, "eth0"), 0);
    assert_eq!(call(&mut ia, &mut p.a, 1, SOCK_SEND, &with_sock(s, &blob(&f[..10])), 0).1, -1);
    // An administratively down interface sends nothing.
    p.a.set_admin_up(0, false, 0);
    assert_eq!(call(&mut ia, &mut p.a, 1, SOCK_SEND, &with_sock(s, &blob(&f)), 0).1, -1);
    p.a.set_admin_up(0, true, 0);
    while p.a.pop_tx().is_some() {}
    assert_eq!(call(&mut ia, &mut p.a, 1, SOCK_SEND, &with_sock(s, &blob(&f)), 0).1, f.len() as i32);
    assert_eq!(p.a.pop_tx(), Some((0, f.clone())));
    // AF_PACKET only supports SOCK_RAW.
    let (_, st, _) = call(&mut ia, &mut p.a, 1, SOCK_SOCKET, &i32s(&[AF_PACKET, SOCK_DGRAM, 0x0008]), 0);
    assert_eq!(st, -1);
    // Destroying the session closes the packet socket (its handle goes stale).
    ia.destroy_session(&mut p.a, 1);
    assert!(p.a.packet_binding(ecm_net::SocketHandle::from_raw(1 << 16)).is_err());
}

#[test]
fn poll_waits_for_a_frame_and_times_out() {
    let mut p = Pair::new();
    let (mut ia, mut ib) = (SocketIpc::new(), SocketIpc::new());
    let tx = packet_socket(&mut ia, &mut p.a, 1, ETH_LOCAL1);
    bind_packet(&mut ia, &mut p.a, 1, tx, "eth0");
    let rx = packet_socket(&mut ib, &mut p.b, 1, ETH_LOCAL1);
    bind_packet(&mut ib, &mut p.b, 1, rx, "eth0");
    let (_, u, _) = call(&mut ib, &mut p.b, 1, SOCK_SOCKET, &i32s(&[AF_INET, SOCK_DGRAM, 0]), 0);
    let poll_args = |timeout: i32, items: &[(i32, i16)]| {
        let mut a = i32s(&[timeout, items.len() as i32]);
        for (id, ev) in items {
            a.extend_from_slice(&id.to_le_bytes());
            a.extend_from_slice(&ev.to_le_bytes());
            a.extend_from_slice(&[0, 0]);
        }
        a
    };
    let revents = |payload: &[u8], i: usize| i16::from_le_bytes([payload[2 * i], payload[2 * i + 1]]);
    // Nothing to read: a zero timeout returns at once with no events.
    let (_, st, _) = call(&mut ib, &mut p.b, 1, SOCK_POLL, &poll_args(0, &[(rx, POLLIN), (u, POLLIN)]), p.now);
    assert_eq!(st, 0);
    // POLLOUT on a bound packet socket is ready at once; bad ids are POLLNVAL.
    let (_, st, pl) = call(&mut ib, &mut p.b, 1, SOCK_POLL, &poll_args(100, &[(rx, POLLOUT), (31, POLLIN)]), p.now);
    assert_eq!(st, 2);
    assert_eq!((revents(&pl, 0), revents(&pl, 1)), (POLLOUT, POLLNVAL));
    // Wait for input: pending until the frame arrives.
    let wait = poll_args(5_000, &[(u, POLLIN), (rx, POLLIN)]);
    assert_eq!(call(&mut ib, &mut p.b, 1, SOCK_POLL, &wait, p.now).0, IPC_PENDING);
    assert_eq!(ib.next_deadline(), Some(p.now + 5_000));
    let f = frame(MAC_B, MAC_A, ETH_LOCAL1, &[3u8; 46]);
    call(&mut ia, &mut p.a, 1, SOCK_SEND, &with_sock(tx, &blob(&f)), p.now);
    p.run(10);
    let (_, st, pl) = call(&mut ib, &mut p.b, 1, SOCK_POLL, &wait, p.now);
    assert_eq!(st, 1);
    assert_eq!((revents(&pl, 0), revents(&pl, 1)), (0, POLLIN));
    assert!(!ib.has_pending());
    assert_eq!(try_recv(&mut ib, &mut p.b, 1, rx).1, f);
    // Timeout path: pending, then zero events when the time is up.
    let t0 = p.now;
    let wait = poll_args(300, &[(rx, POLLIN)]);
    assert_eq!(call(&mut ib, &mut p.b, 1, SOCK_POLL, &wait, t0).0, IPC_PENDING);
    assert_eq!(call(&mut ib, &mut p.b, 1, SOCK_POLL, &wait, t0 + 299).0, IPC_PENDING);
    let (_, st, pl) = call(&mut ib, &mut p.b, 1, SOCK_POLL, &wait, t0 + 300);
    assert_eq!((st, revents(&pl, 0)), (0, 0));
    // An unbound packet socket reports POLLERR (its calls fail at once).
    let lone = packet_socket(&mut ib, &mut p.b, 1, ETH_LOCAL1);
    let (_, st, pl) = call(&mut ib, &mut p.b, 1, SOCK_POLL, &poll_args(-1, &[(lone, POLLIN)]), p.now);
    assert_eq!((st, revents(&pl, 0)), (1, POLLERR));
    // Too many entries is an error, not a panic.
    assert_eq!(call(&mut ib, &mut p.b, 1, SOCK_POLL, &i32s(&[0, 65]), p.now).1, -1);
}

/// A DHCP server on a packet socket (the same frames `dhcpd` uses) gives a
/// lease to the kernel DHCP client on an unconfigured interface.
#[test]
fn dhcp_over_packet_socket_leases_to_the_kernel_client() {
    use ecm_net::dhcp_server::{parse_client_frame, reply_frame, LeaseDb, PoolSpec};
    let server = Ipv4Addr::new(10, 0, 0, 1);
    let mut p = Pair::new();
    p.b.clear_addr(0);
    let mut ia = SocketIpc::new();
    let s = packet_socket(&mut ia, &mut p.a, 1, 0x0800);
    bind_packet(&mut ia, &mut p.a, 1, s, "eth0");
    let pool = PoolSpec {
        name: "eth0".into(),
        start: Ipv4Addr::new(10, 0, 0, 50),
        end: Ipv4Addr::new(10, 0, 0, 60),
        prefix: 24,
        router: Some(server),
        dns: Some(Ipv4Addr::new(10, 0, 0, 53)),
        lease_secs: 3600,
    };
    let mut db = LeaseDb::new();
    let serve = |ia: &mut SocketIpc, p: &mut Pair, db: &mut LeaseDb| loop {
        let (st, f) = try_recv(ia, &mut p.a, 1, s);
        if st <= 0 {
            break;
        }
        let Some(req) = parse_client_frame(&f) else { continue };
        if let Some(reply) = db.handle(&pool, &[], &req.msg, server, p.now) {
            let out = reply_frame(MacAddr(MAC_A), server, &req, &reply, 1).unwrap();
            call(ia, &mut p.a, 1, SOCK_SEND, &with_sock(s, &blob(&out)), p.now);
        }
    };
    p.b.start_dhcp(0, p.now).unwrap();
    assert_eq!(p.b.dhcp_status(0).unwrap().state, ecm_net::dhcp::State::Init);
    for _ in 0..200 {
        p.run(20);
        serve(&mut ia, &mut p, &mut db);
        if p.b.iface(0).unwrap().ip != Ipv4Addr::ZERO {
            break;
        }
    }
    assert_eq!(p.b.iface(0).unwrap().ip, Ipv4Addr::new(10, 0, 0, 50));
    let st = p.b.dhcp_status(0).unwrap();
    assert_eq!(st.state, ecm_net::dhcp::State::Bound);
    assert_eq!(st.server, Some(server));
    assert_eq!(p.b.dns_server(), Ipv4Addr::new(10, 0, 0, 53));
    assert_eq!(db.bound("eth0").count(), 1);
    // Release: the server frees the lease; the client drops the address.
    assert!(p.b.release_dhcp(0, p.now));
    p.run(20);
    serve(&mut ia, &mut p, &mut db);
    assert_eq!(db.bound("eth0").count(), 0);
    assert_eq!(p.b.iface(0).unwrap().ip, Ipv4Addr::ZERO);
    assert_eq!(p.b.dns_server(), Ipv4Addr::ZERO);
    assert!(p.b.dhcp_status(0).is_none());
}
