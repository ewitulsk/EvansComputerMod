//! Multi-stack integration tests over a virtual link.

mod common;

use common::*;
use ecm_net::arp::{ArpPacket, ARP_REPLY, ARP_REQUEST};
use ecm_net::eth::{build_frame, EthHeader, VlanTag, ETHERTYPE_ARP, ETHERTYPE_IPV4};
use ecm_net::icmp::{IcmpPacket, ICMP_ECHO_REPLY, ICMP_ECHO_REQUEST};
use ecm_net::ipv4::{build_packet, Ipv4Header, PROTO_TCP, PROTO_UDP};
use ecm_net::tcp::{build_segment, TcpHeader, ACK, RST, SYN};
use ecm_net::*;

const A: usize = 0;
const B: usize = 1;

#[allow(clippy::too_many_arguments)]
fn tcp_frame(src_mac: MacAddr, dst_mac: MacAddr, src: SocketAddr, dst: SocketAddr, seq: u32, ack: u32, flags: u8, payload: &[u8]) -> Vec<u8> {
    let seg = build_segment(src.ip, dst.ip, src.port, dst.port, seq, ack, flags, 8192, None, payload);
    let pkt = build_packet(src.ip, dst.ip, PROTO_TCP, 1, &seg).unwrap();
    build_frame(dst_mac, src_mac, None, ETHERTYPE_IPV4, &pkt).unwrap()
}

fn arp_frame(op: u16, src_mac: MacAddr, sender_ip: Ipv4Addr, target_ip: Ipv4Addr, eth_dst: MacAddr) -> Vec<u8> {
    let p = ArpPacket { operation: op, sender_mac: src_mac, sender_ip, target_mac: MacAddr::ZERO, target_ip };
    build_frame(eth_dst, src_mac, None, ETHERTYPE_ARP, &p.to_bytes()).unwrap()
}

fn neighbor(net: &Net, node: usize, ip_: Ipv4Addr) -> Option<(MacAddr, NeighborState)> {
    net.stacks[node].neighbors(0).find(|n| n.0 == ip_).map(|n| (n.1, n.2))
}

/// Establish a TCP connection A → B:port. Returns (client, server).
fn connect(net: &mut Net, a: usize, b: usize, dst: SocketAddr) -> (SocketHandle, SocketHandle) {
    let l = net.s(b).tcp_listen(SocketAddr::new(Ipv4Addr::ZERO, dst.port), 8).unwrap();
    let now = net.now;
    let c = net.s(a).tcp_connect(dst, now).unwrap();
    let mut srv = None;
    assert!(net.run(10_000, |n| {
        if srv.is_none() {
            srv = n.s(b).tcp_accept(l).unwrap();
        }
        srv.is_some() && n.s(a).tcp_state(c) == Ok(TcpState::Established)
    }));
    let now = net.now;
    net.s(b).tcp_close(l, now);
    (c, srv.unwrap())
}

// ------------------------------------------------------------------------ ARP

#[test]
fn arp_resolve_and_learning() {
    let mut net = two_hosts(1);
    let u = net.s(A).udp_bind(sa("0.0.0.0", 0)).unwrap();
    let now = net.now;
    net.s(A).udp_send_to(u, sa("10.0.0.2", 9), b"x", now).unwrap();
    net.advance(10);
    assert_eq!(neighbor(&net, A, ip("10.0.0.2")), Some((mac(2), NeighborState::Reachable)));
    // B learned A from the request that targeted B's IP
    assert_eq!(neighbor(&net, B, ip("10.0.0.1")), Some((mac(1), NeighborState::Reachable)));
    // exactly one request and one reply were exchanged
    let arps: Vec<_> = net.capture.iter().filter(|c| c.eth().ethertype == ETHERTYPE_ARP).collect();
    assert_eq!(arps.len(), 1 + 1 + 2, "2 gratuitous + request + reply"); // capture includes boot GARPs? (cleared below)
    // entry goes stale after 60 s, and is re-probed (not dropped) on next use
    net.advance(61_000);
    assert_eq!(neighbor(&net, A, ip("10.0.0.2")).unwrap().1, NeighborState::Stale);
    net.clear_capture();
    let now = net.now;
    net.s(A).udp_send_to(u, sa("10.0.0.2", 9), b"y", now).unwrap();
    net.advance(10);
    assert!(net.capture.iter().any(|c| c.eth().ethertype == ETHERTYPE_IPV4 && c.eth().dst == mac(2)));
    assert_eq!(neighbor(&net, A, ip("10.0.0.2")).unwrap().1, NeighborState::Reachable);
}

#[test]
fn arp_failure_reports_host_unreachable() {
    let mut net = two_hosts(2);
    net.clear_capture();
    let u = net.s(A).udp_bind(sa("0.0.0.0", 5000)).unwrap();
    let now = net.now;
    for i in 0..6 {
        net.s(A).udp_send_to(u, sa("10.0.0.99", 9), &[i], now).unwrap();
    }
    assert_eq!(neighbor(&net, A, ip("10.0.0.99")).unwrap().1, NeighborState::Incomplete);
    let t0 = net.now;
    let mut buf = [0u8; 16];
    let mut err = None;
    assert!(net.run(10_000, |n| {
        match n.s(A).udp_recv_from(u, &mut buf) {
            Err(e) => {
                err = Some((e, n.now));
                true
            }
            _ => false,
        }
    }));
    let (e, t) = err.unwrap();
    assert_eq!(e, NetError::HostUnreachable);
    assert!(t - t0 >= 3000 && t - t0 < 3100, "failed after {} ms", t - t0);
    let reqs = net.capture.iter().filter(|c| c.node == A && c.eth().ethertype == ETHERTYPE_ARP).count();
    assert_eq!(reqs, 3);
    // 4 queued + 2 overflow, all dropped
    assert_eq!(net.stacks[A].iface(0).unwrap().stats.tx_dropped, 6);
    assert!(neighbor(&net, A, ip("10.0.0.99")).is_none());
    assert_eq!(net.stacks[A].stats().arp_failures, 1);
}

#[test]
fn arp_learning_rules_and_mac_filter() {
    let mut net = two_hosts(3);
    let x = mac(0x33);
    let now = net.now;
    // Gratuitous ARP from a stranger: must not create an entry.
    let garp = arp_frame(ARP_REQUEST, x, ip("10.0.0.50"), ip("10.0.0.50"), MacAddr::BROADCAST);
    net.s(A).handle_frame(0, &garp, now);
    // Unsolicited reply not addressed to us: no entry.
    let rep = arp_frame(ARP_REPLY, x, ip("10.0.0.51"), ip("10.0.0.2"), mac(1));
    net.s(A).handle_frame(0, &rep, now);
    // Request for someone else: no entry, no reply.
    let other = arp_frame(ARP_REQUEST, x, ip("10.0.0.52"), ip("10.0.0.2"), MacAddr::BROADCAST);
    net.s(A).handle_frame(0, &other, now);
    assert_eq!(net.stacks[A].neighbors(0).count(), 0);
    assert!(net.s(A).pop_tx().is_none());

    // A frame addressed to another MAC (promiscuous NIC): ignored entirely.
    let before = net.stacks[A].iface(0).unwrap().stats;
    let unicast_other = arp_frame(ARP_REQUEST, x, ip("10.0.0.53"), ip("10.0.0.1"), mac(0x44));
    net.s(A).handle_frame(0, &unicast_other, now);
    let mcast = arp_frame(ARP_REQUEST, x, ip("10.0.0.53"), ip("10.0.0.1"), MacAddr([0x01, 0x80, 0xc2, 0, 0, 0]));
    net.s(A).handle_frame(0, &mcast, now);
    assert_eq!(net.stacks[A].iface(0).unwrap().stats, before);
    assert_eq!(net.stacks[A].neighbors(0).count(), 0);
    assert!(net.s(A).pop_tx().is_none());

    // Broadcast request for our IP: answered and learned.
    let req = arp_frame(ARP_REQUEST, x, ip("10.0.0.53"), ip("10.0.0.1"), MacAddr::BROADCAST);
    net.s(A).handle_frame(0, &req, now);
    assert_eq!(neighbor(&net, A, ip("10.0.0.53")), Some((x, NeighborState::Reachable)));
    let (_, f) = net.s(A).pop_tx().unwrap();
    let (eh, p) = EthHeader::parse(&f).unwrap();
    let r = ArpPacket::parse(p).unwrap();
    assert_eq!((eh.dst, r.operation, r.sender_ip, r.target_mac), (x, ARP_REPLY, ip("10.0.0.1"), x));

    // Gratuitous ARP from a known neighbour updates it.
    let x2 = mac(0x34);
    let garp2 = arp_frame(ARP_REQUEST, x2, ip("10.0.0.53"), ip("10.0.0.53"), MacAddr::BROADCAST);
    net.s(A).handle_frame(0, &garp2, now);
    assert_eq!(neighbor(&net, A, ip("10.0.0.53")), Some((x2, NeighborState::Reachable)));
    // Claims for our own IP are ignored.
    let spoof = arp_frame(ARP_REPLY, x, ip("10.0.0.1"), ip("10.0.0.1"), MacAddr::BROADCAST);
    net.s(A).handle_frame(0, &spoof, now);
    assert!(neighbor(&net, A, ip("10.0.0.1")).is_none());
}

#[test]
fn neighbor_table_is_bounded() {
    let mut net = two_hosts(4);
    let now = net.now;
    for i in 0..100u8 {
        let req = arp_frame(ARP_REQUEST, mac(100 + i), Ipv4Addr::new(10, 0, 0, 100 + i), ip("10.0.0.1"), MacAddr::BROADCAST);
        net.s(A).handle_frame(0, &req, now + i as i64);
    }
    assert_eq!(net.stacks[A].neighbors(0).count(), MAX_NEIGHBORS);
    // most recent entries survive
    assert!(neighbor(&net, A, Ipv4Addr::new(10, 0, 0, 199)).is_some());
}

// ----------------------------------------------------------------------- ICMP

#[test]
fn ping_and_per_socket_icmp_queues() {
    let mut net = two_hosts(5);
    let s1 = net.s(A).icmp_open().unwrap();
    let s2 = net.s(A).icmp_open().unwrap();
    let echo = IcmpPacket::build_echo(ICMP_ECHO_REQUEST, 0x4242, 7, b"ping-payload");
    let now = net.now;
    assert_eq!(net.s(A).icmp_send(s1, ip("10.0.0.2"), &echo, now), Ok(echo.len()));
    let mut buf = [0u8; 128];
    let mut got = None;
    assert!(net.run(2000, |n| {
        got = n.s(A).icmp_recv(s1, &mut buf).unwrap();
        got.is_some()
    }));
    let (from, len) = got.unwrap();
    assert_eq!(from, ip("10.0.0.2"));
    let (p, body) = IcmpPacket::parse(&buf[..len]).unwrap();
    assert_eq!((p.icmp_type, p.id, p.seq), (ICMP_ECHO_REPLY, 0x4242, 7));
    assert_eq!(body, b"ping-payload");
    // the other socket got its own copy
    let (from2, len2) = net.s(A).icmp_recv(s2, &mut buf).unwrap().unwrap();
    assert_eq!((from2, len2), (from, len));
    assert_eq!(net.s(A).icmp_recv(s2, &mut buf), Ok(None));
    // close → BadHandle
    net.s(A).icmp_close(s1);
    assert_eq!(net.s(A).icmp_recv(s1, &mut buf), Err(NetError::BadHandle));
    assert_eq!(net.s(A).icmp_send(s1, ip("10.0.0.2"), &echo, now), Err(NetError::BadHandle));
    assert_eq!(net.s(A).icmp_send(s2, ip("10.0.0.2"), &[0; 4], now), Err(NetError::InvalidInput));
    assert_eq!(net.s(A).icmp_send(s2, ip("10.0.0.2"), &[0; 1481], now), Err(NetError::MessageTooLong));
}

#[test]
fn icmp_unreachable_reported_on_arp_failure() {
    let mut net = two_hosts(6);
    let s = net.s(A).icmp_open().unwrap();
    let echo = IcmpPacket::build_echo(ICMP_ECHO_REQUEST, 1, 1, b"");
    let now = net.now;
    net.s(A).icmp_send(s, ip("10.0.0.77"), &echo, now).unwrap();
    let mut buf = [0u8; 64];
    assert!(net.run(5000, |n| n.s(A).icmp_recv(s, &mut buf) == Err(NetError::HostUnreachable)));
}

// ------------------------------------------------------------------------ UDP

#[test]
fn udp_echo() {
    let mut net = two_hosts(7);
    let srv = net.s(B).udp_bind(sa("0.0.0.0", 7)).unwrap();
    let cli = net.s(A).udp_bind(sa("10.0.0.1", 0)).unwrap();
    let cport = net.stacks[A].udp_local_addr(cli).unwrap().port;
    assert!(cport >= 49152);
    assert_eq!(net.s(B).udp_bind(sa("10.0.0.2", 7)), Err(NetError::AddrInUse));
    assert_eq!(net.s(B).udp_bind(sa("10.9.9.9", 8)), Err(NetError::InvalidInput));
    let now = net.now;
    // at most ARP_PENDING_PER_NEIGHBOR datagrams wait for resolution
    for i in 0..4u8 {
        net.s(A).udp_send_to(cli, sa("10.0.0.2", 7), &[i; 100], now).unwrap();
    }
    net.s(A).udp_send_to(cli, sa("10.0.0.2", 7), &[9; 100], now).unwrap();
    assert_eq!(net.stacks[A].iface(0).unwrap().stats.tx_dropped, 1);
    net.advance(5);
    net.s(A).udp_send_to(cli, sa("10.0.0.2", 7), &[4; 100], now).unwrap();
    let mut replies = Vec::new();
    let mut buf = [0u8; 2048];
    assert!(net.run(2000, |n| {
        while let Ok(Some((from, len))) = n.s(B).udp_recv_from(srv, &mut buf) {
            assert_eq!(from, SocketAddr::new(ip("10.0.0.1"), cport));
            let now = n.now;
            n.s(B).udp_send_to(srv, from, &buf[..len], now).unwrap();
        }
        while let Ok(Some((from, len))) = n.s(A).udp_recv_from(cli, &mut buf) {
            assert_eq!(from, sa("10.0.0.2", 7));
            replies.push(buf[..len].to_vec());
        }
        replies.len() == 5
    }));
    for (i, r) in replies.iter().enumerate() {
        assert_eq!(r, &vec![i as u8; 100]);
    }
    assert_eq!(net.s(A).udp_send_to(cli, sa("10.0.0.2", 7), &[0; 1473], now), Err(NetError::MessageTooLong));
    assert_eq!(net.s(A).udp_send_to(cli, sa("10.0.0.2", 7), &[0; 1472], now), Ok(1472));
    assert_eq!(net.s(A).udp_send_to(cli, sa("10.0.0.2", 0), b"x", now), Err(NetError::InvalidInput));
    assert_eq!(net.s(A).udp_send_to(cli, sa("192.168.1.1", 1), b"x", now), Err(NetError::NoRoute));
}

#[test]
fn udp_queue_is_bounded_drop_newest() {
    let mut net = two_hosts(8);
    let srv = net.s(B).udp_bind(sa("0.0.0.0", 9)).unwrap();
    let cli = net.s(A).udp_bind(sa("0.0.0.0", 0)).unwrap();
    let now = net.now;
    net.s(A).udp_send_to(cli, sa("10.0.0.2", 9), &[0], now).unwrap();
    net.advance(10); // resolve ARP
    for i in 1..40u8 {
        let now = net.now;
        net.s(A).udp_send_to(cli, sa("10.0.0.2", 9), &[i], now).unwrap();
        net.advance(1);
    }
    let mut buf = [0u8; 8];
    let mut got = Vec::new();
    while let Ok(Some((_, n))) = net.s(B).udp_recv_from(srv, &mut buf) {
        assert_eq!(n, 1);
        got.push(buf[0]);
    }
    assert_eq!(got, (0..UDP_QUEUE_LEN as u8).collect::<Vec<_>>());
    assert_eq!(net.stacks[B].stats().udp_rx_queue_full, 40 - UDP_QUEUE_LEN as u64);
}

#[test]
fn udp_bad_checksum_and_length_rejected() {
    let mut net = two_hosts(9);
    let srv = net.s(B).udp_bind(sa("0.0.0.0", 9)).unwrap();
    let src = ip("10.0.0.1");
    let dst = ip("10.0.0.2");
    let mut dg = ecm_net::udp::build(src, dst, 1234, 9, b"hello");
    dg[8] ^= 0xff; // corrupt payload → checksum mismatch
    let pkt = build_packet(src, dst, PROTO_UDP, 1, &dg).unwrap();
    let now = net.now;
    net.s(B).handle_frame(0, &build_frame(mac(2), mac(1), None, ETHERTYPE_IPV4, &pkt).unwrap(), now);
    // length field smaller than 8
    let mut dg2 = ecm_net::udp::build(src, dst, 1234, 9, b"hello");
    dg2[4..6].copy_from_slice(&4u16.to_be_bytes());
    let pkt2 = build_packet(src, dst, PROTO_UDP, 2, &dg2).unwrap();
    net.s(B).handle_frame(0, &build_frame(mac(2), mac(1), None, ETHERTYPE_IPV4, &pkt2).unwrap(), now);
    let mut buf = [0u8; 64];
    assert_eq!(net.s(B).udp_recv_from(srv, &mut buf), Ok(None));
    assert_eq!(net.stacks[B].stats().udp_rx_bad, 2);
    assert_eq!(net.stacks[B].iface(0).unwrap().stats.rx_errors, 2);
    // zero checksum is accepted
    let mut dg3 = ecm_net::udp::build(src, dst, 1234, 9, b"hello");
    dg3[6] = 0;
    dg3[7] = 0;
    let pkt3 = build_packet(src, dst, PROTO_UDP, 3, &dg3).unwrap();
    net.s(B).handle_frame(0, &build_frame(mac(2), mac(1), None, ETHERTYPE_IPV4, &pkt3).unwrap(), now);
    assert_eq!(net.s(B).udp_recv_from(srv, &mut buf), Ok(Some((sa("10.0.0.1", 1234), 5))));
}

#[test]
fn ip_fragments_and_bad_headers_dropped() {
    let mut net = two_hosts(10);
    let srv = net.s(B).udp_bind(sa("0.0.0.0", 9)).unwrap();
    let (src, dst) = (ip("10.0.0.1"), ip("10.0.0.2"));
    let dg = ecm_net::udp::build(src, dst, 1, 9, b"frag");
    let mut pkt = build_packet(src, dst, PROTO_UDP, 1, &dg).unwrap();
    pkt[6] = 0x20; // MF
    pkt[10] = 0;
    pkt[11] = 0;
    let c = ecm_net::checksum::internet_checksum(&pkt[..20]);
    pkt[10..12].copy_from_slice(&c.to_be_bytes());
    let now = net.now;
    net.s(B).handle_frame(0, &build_frame(mac(2), mac(1), None, ETHERTYPE_IPV4, &pkt).unwrap(), now);
    assert_eq!(net.stacks[B].stats().ip_rx_fragments_dropped, 1);
    // total_length < header length (the old remote panic)
    let mut bad = build_packet(src, dst, PROTO_UDP, 1, &dg).unwrap();
    bad[2] = 0;
    bad[3] = 4;
    net.s(B).handle_frame(0, &build_frame(mac(2), mac(1), None, ETHERTYPE_IPV4, &bad).unwrap(), now);
    assert_eq!(net.stacks[B].stats().ip_rx_bad, 1);
    let mut buf = [0u8; 16];
    assert_eq!(net.s(B).udp_recv_from(srv, &mut buf), Ok(None));
}

// ------------------------------------------------------------------------ DNS

/// Fake DNS server on `node`: answers `names` with the given address, NXDOMAIN otherwise.
fn serve_dns(net: &mut Net, node: usize, sock: SocketHandle, names: &[(&str, Ipv4Addr)]) -> usize {
    let mut buf = [0u8; 1500];
    let mut served = 0;
    while let Ok(Some((from, len))) = net.s(node).udp_recv_from(sock, &mut buf) {
        let q = &buf[..len];
        let (qname, _) = ecm_net::dns::read_name(q, 12).unwrap();
        let resp = match names.iter().find(|(n, _)| *n == qname) {
            Some((_, a)) => ecm_net::dns::build_response(q, &[*a], 0).unwrap(),
            None => ecm_net::dns::build_response(q, &[], 3).unwrap(),
        };
        let now = net.now;
        net.s(node).udp_send_to(sock, from, &resp, now).unwrap();
        served += 1;
    }
    served
}

#[test]
fn dns_against_fake_server() {
    let mut net = two_hosts(11);
    let srv = net.s(B).udp_bind(sa("0.0.0.0", 53)).unwrap();
    net.s(A).set_dns_server(ip("10.0.0.2"));
    let now = net.now;
    let q1 = net.s(A).dns_query("Host.Example.COM", now).unwrap();
    let q2 = net.s(A).dns_query("missing.example", now).unwrap();
    let mut r1 = DnsStatus::Pending;
    let mut r2 = DnsStatus::Pending;
    assert!(net.run(5000, |n| {
        serve_dns(n, B, srv, &[("host.example.com", ip("93.184.216.34"))]);
        if r1 == DnsStatus::Pending {
            r1 = n.s(A).dns_poll(q1);
        }
        if r2 == DnsStatus::Pending {
            r2 = n.s(A).dns_poll(q2);
        }
        r1 != DnsStatus::Pending && r2 != DnsStatus::Pending
    }));
    assert_eq!(r1, DnsStatus::Resolved(ip("93.184.216.34")));
    assert_eq!(r2, DnsStatus::Failed(NetError::NotFound));
    // handles are freed after a final result
    assert_eq!(net.s(A).dns_poll(q1), DnsStatus::Failed(NetError::BadHandle));
    // literals and localhost resolve without traffic
    let now = net.now;
    let q = net.s(A).dns_query("1.2.3.4", now).unwrap();
    assert_eq!(net.s(A).dns_poll(q), DnsStatus::Resolved(ip("1.2.3.4")));
    let q = net.s(A).dns_query("localhost", now).unwrap();
    assert_eq!(net.s(A).dns_poll(q), DnsStatus::Resolved(Ipv4Addr::LOCALHOST));
    assert_eq!(net.s(A).dns_query("bad..name", now), Err(NetError::InvalidInput));
}

#[test]
fn dns_ignores_spoofed_replies_and_times_out() {
    let mut net = two_hosts(12);
    // B listens on 5353, not 53: replies come from the wrong port and must be ignored.
    let srv = net.s(B).udp_bind(sa("0.0.0.0", 53)).unwrap();
    net.s(A).set_dns_server(ip("10.0.0.2"));
    let spoof = net.s(B).udp_bind(sa("0.0.0.0", 5353)).unwrap();
    let now = net.now;
    let q = net.s(A).dns_query("host.example", now).unwrap();
    let t0 = net.now;
    let mut queries = 0;
    let mut st = DnsStatus::Pending;
    let mut buf = [0u8; 1500];
    assert!(net.run(20_000, |n| {
        while let Ok(Some((from, len))) = n.s(B).udp_recv_from(srv, &mut buf) {
            queries += 1;
            let mut resp = ecm_net::dns::build_response(&buf[..len], &[ip("6.6.6.6")], 0).unwrap();
            let now = n.now;
            // wrong source port
            n.s(B).udp_send_to(spoof, from, &resp, now).unwrap();
            // right port, wrong transaction id
            resp[0] ^= 0xff;
            n.s(B).udp_send_to(srv, from, &resp, now).unwrap();
        }
        st = n.s(A).dns_poll(q);
        st != DnsStatus::Pending
    }));
    assert_eq!(st, DnsStatus::Failed(NetError::TimedOut));
    assert_eq!(queries, 2, "3 s x 2 tries");
    let dt = net.now - t0;
    assert!((6000..6100).contains(&dt), "timed out after {dt} ms");
    let now = net.now;
    let mut s2 = ecm_net::Stack::new(StackConfig { seed: 1 });
    assert_eq!(s2.dns_query("x.y", now), Err(NetError::NotConfigured));
}

// ------------------------------------------------------------------------ TCP

fn pattern(len: usize, seed: u64) -> Vec<u8> {
    Rng(seed).bytes(len)
}

/// Stream `data` from (a, ca) to (b, cb), closing after the last byte; returns what b read.
fn transfer(net: &mut Net, a: usize, ca: SocketHandle, b: usize, cb: SocketHandle, data: &[u8], max_ms: i64) -> Vec<u8> {
    let mut sent = 0;
    let mut closed = false;
    let mut got = Vec::new();
    let mut buf = vec![0u8; 8192];
    let ok = net.run(max_ms, |n| {
        let now = n.now;
        while sent < data.len() {
            match n.s(a).tcp_send(ca, &data[sent..], now) {
                Ok(k) => sent += k,
                Err(NetError::WouldBlock) => break,
                Err(e) => panic!("send: {e:?}"),
            }
        }
        if sent == data.len() && !closed {
            n.s(a).tcp_close(ca, now);
            closed = true;
        }
        loop {
            match n.s(b).tcp_recv(cb, &mut buf) {
                Ok(0) => return true,
                Ok(k) => got.extend_from_slice(&buf[..k]),
                Err(NetError::WouldBlock) => return false,
                Err(e) => panic!("recv: {e:?}"),
            }
        }
    });
    assert!(ok, "transfer did not finish: {} of {} bytes", got.len(), data.len());
    got
}

#[test]
fn tcp_1mib_with_loss_and_reordering() {
    let mut net = two_hosts(13);
    net.capture_on = false;
    let (c, s) = connect(&mut net, A, B, sa("10.0.0.2", 5001));
    net.segments[0].loss = 0.05;
    net.segments[0].reorder = 0.10;
    let data = pattern(1 << 20, 99);
    let t0 = net.now;
    let got = transfer(&mut net, A, c, B, s, &data, 3_600_000);
    assert_eq!(got.len(), data.len());
    assert!(got == data, "stream corrupted");
    let rex = net.stacks[A].stats().tcp_retransmits;
    eprintln!("1 MiB: {} ms virtual, {} frames delivered, {} lost, {} retransmits", net.now - t0, net.delivered, net.lost, rex);
    assert!(net.lost > 20 && rex > 0, "loss was actually exercised");
    net.s(B).tcp_close(s, 0);
}

#[test]
fn tcp_bidirectional_lossy_echo() {
    let mut net = two_hosts(14);
    net.capture_on = false;
    net.segments[0].loss = 0.05;
    net.segments[0].reorder = 0.2;
    let (c, s) = connect(&mut net, A, B, sa("10.0.0.2", 7));
    let data = pattern(200_000, 5);
    let (mut sent, mut echoed, mut back) = (0, 0usize, Vec::new());
    let mut pend: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 4096];
    assert!(net.run(3_600_000, |n| {
        let now = n.now;
        if sent < data.len() {
            if let Ok(k) = n.s(A).tcp_send(c, &data[sent..], now) {
                sent += k;
            }
        }
        while let Ok(k) = n.s(B).tcp_recv(s, &mut buf) {
            if k == 0 {
                break;
            }
            pend.extend_from_slice(&buf[..k]);
        }
        if !pend.is_empty() {
            if let Ok(k) = n.s(B).tcp_send(s, &pend, now) {
                pend.drain(..k);
                echoed += k;
            }
        }
        while let Ok(k) = n.s(A).tcp_recv(c, &mut buf) {
            if k == 0 {
                break;
            }
            back.extend_from_slice(&buf[..k]);
        }
        back.len() == data.len()
    }));
    assert_eq!(echoed, data.len());
    assert!(back == data);
}

#[test]
fn tcp_close_with_pending_tx_delivers_everything_before_fin() {
    let mut net = two_hosts(15);
    let (c, s) = connect(&mut net, A, B, sa("10.0.0.2", 80));
    let data = pattern(60_000, 7);
    let now = net.now;
    // queue everything, then close immediately (no pumping in between)
    assert_eq!(net.s(A).tcp_send(c, &data, now), Ok(60_000));
    net.s(A).tcp_close(c, now);
    assert_eq!(net.stacks[A].tcp_state(c), Err(NetError::BadHandle));
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    let mut eof = false;
    assert!(net.run(60_000, |n| {
        loop {
            match n.s(B).tcp_recv(s, &mut buf) {
                Ok(0) => {
                    eof = true;
                    return true;
                }
                Ok(k) => got.extend_from_slice(&buf[..k]),
                Err(_) => return false,
            }
        }
    }));
    assert!(eof);
    assert!(got == data, "{} bytes before FIN", got.len());
    assert_eq!(net.stacks[B].tcp_state(s), Ok(TcpState::CloseWait));
    let now = net.now;
    net.s(B).tcp_close(s, now);
    net.advance(100);
    // A is the active closer → TIME_WAIT
    let states: Vec<_> = net.stacks[A].tcp_list().map(|t| t.3).collect();
    assert_eq!(states, vec![TcpState::TimeWait]);
    assert_eq!(net.stacks[B].tcp_list().count(), 0, "B (LAST_ACK) freed after the final ACK");
}

#[test]
fn tcp_40_concurrent_connections() {
    let mut net = two_hosts(16);
    net.capture_on = false;
    net.segments[0].loss = 0.02;
    let l = net.s(B).tcp_listen(sa("0.0.0.0", 8080), 64).unwrap();
    let now = net.now;
    let clients: Vec<SocketHandle> = (0..40).map(|_| net.s(A).tcp_connect(sa("10.0.0.2", 8080), now).unwrap()).collect();
    let ports: std::collections::HashSet<u16> = clients.iter().map(|c| net.stacks[A].tcp_local_addr(*c).unwrap().port).collect();
    assert_eq!(ports.len(), 40);
    let mut servers: Vec<SocketHandle> = Vec::new();
    let mut sent = [false; 40];
    let mut replies: Vec<Vec<u8>> = vec![Vec::new(); 40];
    let mut buf = [0u8; 256];
    assert!(net.run(120_000, |n| {
        let now = n.now;
        while let Ok(Some(h)) = n.s(B).tcp_accept(l) {
            servers.push(h);
        }
        for (i, c) in clients.iter().enumerate() {
            if !sent[i] && n.s(A).tcp_state(*c) == Ok(TcpState::Established) {
                let msg = format!("hello from client {i:02}");
                assert_eq!(n.s(A).tcp_send(*c, msg.as_bytes(), now), Ok(msg.len()));
                sent[i] = true;
            }
        }
        for s in &servers {
            while let Ok(k) = n.s(B).tcp_recv(*s, &mut buf) {
                if k == 0 {
                    break;
                }
                let echo: Vec<u8> = buf[..k].to_ascii_uppercase();
                n.s(B).tcp_send(*s, &echo, now).unwrap();
            }
        }
        for (i, c) in clients.iter().enumerate() {
            while let Ok(k) = n.s(A).tcp_recv(*c, &mut buf) {
                if k == 0 {
                    break;
                }
                replies[i].extend_from_slice(&buf[..k]);
            }
        }
        replies.iter().enumerate().all(|(i, r)| r == format!("HELLO FROM CLIENT {i:02}").as_bytes())
    }));
    assert_eq!(servers.len(), 40);
}

#[test]
fn tcp_connect_refused_by_rst() {
    let mut net = two_hosts(17);
    let now = net.now;
    let c = net.s(A).tcp_connect(sa("10.0.0.2", 81), now).unwrap();
    assert_eq!(net.stacks[A].tcp_state(c), Ok(TcpState::SynSent));
    assert!(!net.stacks[A].tcp_can_read(c));
    assert!(net.run(2000, |n| n.stacks[A].tcp_state(c) == Ok(TcpState::Closed)));
    let mut buf = [0u8; 4];
    assert_eq!(net.s(A).tcp_recv(c, &mut buf), Err(NetError::ConnectionRefused));
    let now = net.now;
    assert_eq!(net.s(A).tcp_send(c, b"x", now), Err(NetError::ConnectionRefused));
    assert!(net.stacks[A].tcp_can_read(c));
    assert!(net.stacks[B].stats().tcp_rst_sent >= 1);
    net.s(A).tcp_close(c, now);
    assert_eq!(net.stacks[A].tcp_list().count(), 0);
}

#[test]
fn tcp_connect_to_unresolvable_host_fails() {
    let mut net = two_hosts(18);
    let now = net.now;
    let c = net.s(A).tcp_connect(sa("10.0.0.99", 80), now).unwrap();
    assert!(net.run(10_000, |n| n.stacks[A].tcp_state(c) == Ok(TcpState::Closed)));
    let mut buf = [0u8; 4];
    assert_eq!(net.s(A).tcp_recv(c, &mut buf), Err(NetError::HostUnreachable));
    assert_eq!(net.s(A).tcp_connect(sa("172.16.0.1", 80), now), Err(NetError::NoRoute));
    assert_eq!(net.s(A).tcp_connect(sa("10.0.0.2", 0), now), Err(NetError::InvalidInput));
    assert_eq!(net.s(A).tcp_connect(sa("10.0.0.255", 80), now), Err(NetError::InvalidInput));
}

#[test]
fn tcp_rst_handling() {
    let mut net = two_hosts(19);
    let (c, s) = connect(&mut net, A, B, sa("10.0.0.2", 9000));
    let local = net.stacks[A].tcp_local_addr(c).unwrap();
    let remote = net.stacks[A].tcp_peer_addr(c).unwrap();
    let now = net.now;
    // Out-of-window RST (random seq) must be ignored.
    let bogus = tcp_frame(mac(2), mac(1), remote, local, 0x1234_5678, 0, RST, b"");
    net.s(A).handle_frame(0, &bogus, now);
    assert_eq!(net.stacks[A].tcp_state(c), Ok(TcpState::Established));
    // Bad checksum RST ignored too.
    let mut badck = tcp_frame(mac(2), mac(1), remote, local, 0, 0, RST, b"");
    badck[14 + 20 + 16] ^= 0x55; // TCP checksum byte
    net.s(A).handle_frame(0, &badck, now);
    assert_eq!(net.stacks[A].tcp_state(c), Ok(TcpState::Established));
    assert!(net.stacks[A].stats().tcp_rx_bad >= 1);
    net.advance(10);
    // Abort from B → A sees ConnectionReset.
    net.s(B).tcp_abort(s);
    assert_eq!(net.stacks[B].tcp_state(s), Err(NetError::BadHandle));
    assert!(net.run(1000, |n| n.stacks[A].tcp_state(c) == Ok(TcpState::Closed)));
    let mut buf = [0u8; 8];
    assert_eq!(net.s(A).tcp_recv(c, &mut buf), Err(NetError::ConnectionReset));
    assert_eq!(net.stacks[B].tcp_list().count(), 0);
}

#[test]
fn tcp_syn_ack_with_bad_ack_is_rejected() {
    // Craft a SYN-ACK acknowledging the wrong ISN: A must answer RST and stay in SYN_SENT.
    let mut net = two_hosts(20);
    net.capture_on = true;
    net.segments[0].up = false; // A's SYN goes nowhere
    let now = net.now;
    let c = net.s(A).tcp_connect(sa("10.0.0.2", 80), now).unwrap();
    // make A know B's MAC so the RST can go out
    let req = arp_frame(ARP_REQUEST, mac(2), ip("10.0.0.2"), ip("10.0.0.1"), MacAddr::BROADCAST);
    net.s(A).handle_frame(0, &req, now);
    while net.s(A).pop_tx().is_some() {}
    let local = net.stacks[A].tcp_local_addr(c).unwrap();
    let f = tcp_frame(mac(2), mac(1), sa("10.0.0.2", 80), local, 5000, 0xdead_beef, SYN | ACK, b"");
    net.s(A).handle_frame(0, &f, now);
    assert_eq!(net.stacks[A].tcp_state(c), Ok(TcpState::SynSent));
    let (_, out) = net.s(A).pop_tx().expect("RST");
    let (_, ip_) = EthHeader::parse(&out).unwrap();
    let (_, seg) = Ipv4Header::parse(ip_).unwrap();
    let (th, _) = TcpHeader::parse(seg).unwrap();
    assert!(th.has(RST));
    assert_eq!(th.seq_num, 0xdead_beef);
    // An ACK to a listener without a connection also gets RST (and no socket).
    let l = net.s(A).tcp_listen(sa("0.0.0.0", 22), 4).unwrap();
    let f = tcp_frame(mac(2), mac(1), sa("10.0.0.2", 4444), sa("10.0.0.1", 22), 1, 77, ACK, b"");
    net.s(A).handle_frame(0, &f, now);
    let (_, out) = net.s(A).pop_tx().expect("RST");
    let (_, ip_) = EthHeader::parse(&out).unwrap();
    let (_, seg) = Ipv4Header::parse(ip_).unwrap();
    assert!(TcpHeader::parse(seg).unwrap().0.has(RST));
    assert_eq!(net.s(A).tcp_accept(l), Ok(None));
}

#[test]
fn tcp_time_wait_expiry_frees_the_slot() {
    let mut net = two_hosts(21);
    // Fill every slot but one on A with UDP sockets.
    let mut udp = Vec::new();
    for _ in 0..MAX_SOCKETS - 1 {
        udp.push(net.s(A).udp_bind(sa("0.0.0.0", 0)).unwrap());
    }
    let (c, s) = connect(&mut net, A, B, sa("10.0.0.2", 7000));
    assert_eq!(net.s(A).udp_bind(sa("0.0.0.0", 0)), Err(NetError::NoSockets));
    let now = net.now;
    net.s(A).tcp_close(c, now);
    assert!(net.run(1000, |n| n.stacks[B].tcp_state(s) == Ok(TcpState::CloseWait)));
    let now = net.now;
    net.s(B).tcp_close(s, now);
    net.advance(100);
    assert_eq!(net.stacks[A].tcp_list().next().unwrap().3, TcpState::TimeWait);
    let t_tw = net.now;
    assert_eq!(net.s(A).udp_bind(sa("0.0.0.0", 0)), Err(NetError::NoSockets), "TIME_WAIT still holds the slot");
    assert_eq!(net.s(A).tcp_connect(sa("10.0.0.2", 7000), t_tw), Err(NetError::NoSockets));
    net.advance(TIME_WAIT_MS - 1000);
    assert_eq!(net.stacks[A].tcp_list().count(), 1);
    net.advance(2000);
    assert_eq!(net.stacks[A].tcp_list().count(), 0);
    assert!(net.s(A).udp_bind(sa("0.0.0.0", 0)).is_ok());
}

#[test]
fn tcp_fin_wait2_timeout_for_orphans() {
    let mut net = two_hosts(22);
    let (c, s) = connect(&mut net, A, B, sa("10.0.0.2", 7001));
    let now = net.now;
    net.s(A).tcp_close(c, now);
    net.advance(100);
    assert_eq!(net.stacks[A].tcp_list().next().unwrap().3, TcpState::FinWait2);
    assert_eq!(net.stacks[B].tcp_state(s), Ok(TcpState::CloseWait));
    net.advance(FIN_WAIT2_TIMEOUT_MS + 1000);
    assert_eq!(net.stacks[A].tcp_list().count(), 0);
}

#[test]
fn tcp_half_close_keeps_receiving() {
    let mut net = two_hosts(23);
    let (c, s) = connect(&mut net, A, B, sa("10.0.0.2", 7002));
    let now = net.now;
    net.s(A).tcp_shutdown_write(c, now).unwrap();
    let mut buf = [0u8; 64];
    assert!(net.run(1000, |n| n.s(B).tcp_recv(s, &mut buf) == Ok(0)));
    let now = net.now;
    net.s(B).tcp_send(s, b"late data", now).unwrap();
    let mut got = Vec::new();
    assert!(net.run(1000, |n| {
        while let Ok(k) = n.s(A).tcp_recv(c, &mut buf) {
            if k == 0 {
                return false;
            }
            got.extend_from_slice(&buf[..k]);
        }
        got == b"late data"
    }));
    assert_eq!(net.stacks[A].tcp_state(c), Ok(TcpState::FinWait2));
    assert_eq!(net.s(A).tcp_send(c, b"x", now), Err(NetError::NotConnected));
}

#[test]
fn tcp_zero_window_and_persist() {
    let mut net = two_hosts(24);
    net.capture_on = false;
    let (c, s) = connect(&mut net, A, B, sa("10.0.0.2", 7003));
    let data = pattern(200_000, 3);
    let mut sent = 0;
    // B does not read: A fills B's receive window and its own send buffer.
    assert!(!net.run(10_000, |n| {
        let now = n.now;
        if let Ok(k) = n.s(A).tcp_send(c, &data[sent..], now) {
            sent += k;
        }
        false
    }));
    assert_eq!(sent, TCP_RX_BUF + TCP_TX_BUF, "sender blocked with both buffers full");
    assert!(!net.stacks[A].tcp_can_write(c));
    // Keep B from reading for a couple of minutes: the persist timer must not kill the connection.
    net.advance(180_000);
    assert_eq!(net.stacks[A].tcp_state(c), Ok(TcpState::Established));
    // Now read everything.
    let got = transfer_from(&mut net, c, s, &data, sent);
    assert!(got == data);
}

fn transfer_from(net: &mut Net, c: SocketHandle, s: SocketHandle, data: &[u8], mut sent: usize) -> Vec<u8> {
    let mut got = Vec::new();
    let mut buf = vec![0u8; 3000];
    let mut closed = false;
    assert!(net.run(600_000, |n| {
        let now = n.now;
        while sent < data.len() {
            match n.s(A).tcp_send(c, &data[sent..], now) {
                Ok(k) => sent += k,
                Err(_) => break,
            }
        }
        if sent == data.len() && !closed {
            n.s(A).tcp_close(c, now);
            closed = true;
        }
        loop {
            match n.s(B).tcp_recv(s, &mut buf) {
                Ok(0) => return true,
                Ok(k) => got.extend_from_slice(&buf[..k]),
                Err(_) => return false,
            }
        }
    }));
    got
}

#[test]
fn tcp_retransmit_limit_aborts() {
    let mut net = two_hosts(25);
    let (c, _s) = connect(&mut net, A, B, sa("10.0.0.2", 7004));
    net.segments[0].up = false; // cable cut, carrier still up
    let now = net.now;
    net.s(A).tcp_send(c, b"into the void", now).unwrap();
    let t0 = net.now;
    let mut buf = [0u8; 4];
    assert!(net.run(1_000_000, |n| n.s(A).tcp_recv(c, &mut buf) == Err(NetError::TimedOut)));
    let dt = net.now - t0;
    assert!(dt > 60_000 && dt < 600_000, "aborted after {dt} ms");
    assert_eq!(net.stacks[A].tcp_state(c), Ok(TcpState::Closed));
}

#[test]
fn tcp_listen_accept_semantics() {
    let mut net = two_hosts(26);
    let l = net.s(B).tcp_listen(sa("0.0.0.0", 443), 2).unwrap();
    assert_eq!(net.s(B).tcp_listen(sa("10.0.0.2", 443), 2), Err(NetError::AddrInUse));
    assert_eq!(net.s(B).tcp_listen(sa("10.1.1.1", 444), 2), Err(NetError::InvalidInput));
    let eph = net.s(B).tcp_listen(sa("0.0.0.0", 0), 1).unwrap();
    assert!(net.stacks[B].tcp_local_addr(eph).unwrap().port >= 49152);
    let mut buf = [0u8; 4];
    assert_eq!(net.s(B).tcp_recv(l, &mut buf), Err(NetError::NotConnected));
    // Backlog 2: the third SYN is dropped until something is accepted.
    let now = net.now;
    let cs: Vec<_> = (0..3).map(|_| net.s(A).tcp_connect(sa("10.0.0.2", 443), now).unwrap()).collect();
    net.advance(50);
    let est = cs.iter().filter(|c| net.stacks[A].tcp_state(**c) == Ok(TcpState::Established)).count();
    assert_eq!(est, 2);
    // A connection that is closed by the peer before accept is still accepted (CLOSE_WAIT) and readable.
    let first = cs.iter().copied().find(|c| net.stacks[A].tcp_state(*c) == Ok(TcpState::Established)).unwrap();
    let now = net.now;
    net.s(A).tcp_send(first, b"hi", now).unwrap();
    net.s(A).tcp_shutdown_write(first, now).unwrap();
    net.advance(50);
    let mut accepted = Vec::new();
    assert!(net.run(10_000, |n| {
        while let Ok(Some(h)) = n.s(B).tcp_accept(l) {
            accepted.push(h);
        }
        accepted.len() == 3
    }));
    let states: Vec<_> = accepted.iter().map(|h| net.stacks[B].tcp_state(*h).unwrap()).collect();
    assert!(states.contains(&TcpState::CloseWait), "{states:?}");
    let cw = accepted[states.iter().position(|s| *s == TcpState::CloseWait).unwrap()];
    let mut got = [0u8; 8];
    assert_eq!(net.s(B).tcp_recv(cw, &mut got), Ok(2));
    assert_eq!(net.s(B).tcp_recv(cw, &mut got), Ok(0));
    // Closing the listener resets nothing already accepted.
    let now = net.now;
    net.s(B).tcp_close(l, now);
    assert_eq!(net.s(B).tcp_accept(l), Err(NetError::BadHandle));
    assert!(net.stacks[B].tcp_state(accepted[0]).is_ok());
}

#[test]
fn stale_handles_are_rejected() {
    let mut net = two_hosts(27);
    let u = net.s(A).udp_bind(sa("0.0.0.0", 1000)).unwrap();
    net.s(A).udp_close(u);
    let u2 = net.s(A).udp_bind(sa("0.0.0.0", 1000)).unwrap(); // reuses the slot
    assert_ne!(u, u2);
    let mut buf = [0u8; 4];
    assert_eq!(net.s(A).udp_recv_from(u, &mut buf), Err(NetError::BadHandle));
    assert_eq!(net.s(A).udp_recv_from(u2, &mut buf), Ok(None));
    // wrong-kind handle
    assert_eq!(net.s(A).tcp_state(u2), Err(NetError::BadHandle));
    assert_eq!(net.s(A).icmp_recv(u2, &mut buf), Err(NetError::BadHandle));
    // closing a stale handle must not close the new socket
    net.s(A).udp_close(u);
    assert_eq!(net.s(A).udp_recv_from(u2, &mut buf), Ok(None));
    assert_eq!(SocketHandle::from_raw(u2.to_raw()), u2);
    let junk = SocketHandle::from_raw(0xffff_ffff);
    assert_eq!(net.s(A).tcp_recv(junk, &mut buf), Err(NetError::BadHandle));
    let now = net.now;
    net.s(A).tcp_close(junk, now);
    net.s(A).tcp_abort(junk);
}

// -------------------------------------------------------- routing / L2 / loopback

#[test]
fn default_route_via_gateway() {
    let mut net = Net::new(28);
    let a = net.host(1, "10.0.0.1/24");
    let g = net.host(0xfe, "10.0.0.254/24");
    let c = net.host(3, "10.0.0.3/24");
    net.segment(&[(a, 0), (g, 0), (c, 0)]);
    assert_eq!(net.s(a).add_route(ip("0.0.0.0"), 0, ip("10.0.0.254"), 5), Err(NetError::InvalidInput));
    assert_eq!(net.s(a).add_route(ip("0.0.0.0"), 0, ip("10.0.0.1"), 0), Err(NetError::InvalidInput));
    net.s(a).add_route(ip("0.0.0.0"), 0, ip("10.0.0.254"), 0).unwrap();
    net.s(a).add_route(ip("192.168.7.99"), 24, ip("10.0.0.3"), 0).unwrap();
    assert!(net.stacks[a].routes().iter().any(|r| r.dst == ip("192.168.7.0") && r.prefix == 24));
    net.pump();
    net.clear_capture();
    let u = net.s(a).udp_bind(sa("0.0.0.0", 0)).unwrap();
    let now = net.now;
    net.s(a).udp_send_to(u, sa("8.8.8.8", 53), b"far", now).unwrap();
    net.s(a).udp_send_to(u, sa("10.0.0.3", 9), b"near", now).unwrap();
    net.s(a).udp_send_to(u, sa("192.168.7.5", 9), b"specific", now).unwrap();
    net.advance(20);
    let ipv4: Vec<_> = net
        .capture
        .iter()
        .filter(|f| f.node == a && f.eth().ethertype == ETHERTYPE_IPV4)
        .map(|f| {
            let (eh, p) = EthHeader::parse(&f.frame).unwrap();
            (eh.dst, Ipv4Header::parse(p).unwrap().0.dst)
        })
        .collect();
    assert!(ipv4.contains(&(mac(0xfe), ip("8.8.8.8"))), "{ipv4:?}");
    assert!(ipv4.contains(&(mac(3), ip("10.0.0.3"))));
    assert!(ipv4.contains(&(mac(3), ip("192.168.7.5"))), "longest prefix wins");
    // A only ARPed for the gateway and C, never for 8.8.8.8
    assert!(neighbor(&net, a, ip("8.8.8.8")).is_none());
    assert_eq!(neighbor(&net, a, ip("10.0.0.254")).unwrap().0, mac(0xfe));
    // The gateway (a host, not a router) drops the transit packet.
    assert_eq!(net.stacks[g].stats().ip_rx_not_for_us, 1);
    net.s(a).del_route(ip("0.0.0.0"), 0).unwrap();
    assert_eq!(net.s(a).del_route(ip("0.0.0.0"), 0), Err(NetError::NotFound));
    let now = net.now;
    assert_eq!(net.s(a).udp_send_to(u, sa("8.8.8.8", 53), b"far", now), Err(NetError::NoRoute));
}

#[test]
fn vlan_tagged_interfaces() {
    let mut net = Net::new(29);
    let a = net.host(1, "10.10.0.1/24");
    let b = net.host(2, "10.10.0.2/24");
    let c = net.host(3, "10.10.0.3/24"); // untagged on the same wire
    net.pump(); // flush the boot-time (untagged) gratuitous ARPs
    net.clear_capture();
    net.s(a).set_vlan(0, Some(10));
    net.s(b).set_vlan(0, Some(10));
    net.s(a).set_vlan(0, Some(4095)); // invalid: ignored
    assert_eq!(net.stacks[a].iface(0).unwrap().vlan, Some(10));
    net.segment(&[(a, 0), (b, 0), (c, 0)]);
    net.clear_capture();
    let (cl, sv) = connect(&mut net, a, b, sa("10.10.0.2", 23));
    let now = net.now;
    net.s(a).tcp_send(cl, b"tagged", now).unwrap();
    let mut buf = [0u8; 16];
    assert!(net.run(1000, |n| n.s(b).tcp_recv(sv, &mut buf) == Ok(6)));
    // every frame from a and b carries VID 10
    assert!(net.capture.iter().filter(|f| f.node != c).all(|f| f.eth().vlan_tag.map(|t| t.vid) == Some(10)));
    assert!(net.capture.iter().any(|f| f.node == a && f.eth().ethertype == ETHERTYPE_IPV4));
    // c never learned a or b (ignored tagged frames), and a ignores c's untagged ping
    assert!(neighbor(&net, c, ip("10.10.0.1")).is_none());
    let drops_before = net.stacks[a].iface(0).unwrap().stats.rx_dropped;
    let s = net.s(c).icmp_open().unwrap();
    let echo = IcmpPacket::build_echo(ICMP_ECHO_REQUEST, 9, 9, b"");
    let now = net.now;
    net.s(c).icmp_send(s, ip("10.10.0.1"), &echo, now).unwrap();
    net.advance(5000);
    assert!(neighbor(&net, c, ip("10.10.0.1")).is_none());
    assert!(net.stacks[a].iface(0).unwrap().stats.rx_dropped > drops_before);
    // a wrong tag is dropped too; untagged iface accepts priority-tagged (VID 0)
    let p = ArpPacket { operation: ARP_REQUEST, sender_mac: mac(9), sender_ip: ip("10.10.0.9"), target_mac: MacAddr::ZERO, target_ip: ip("10.10.0.3") };
    let wrong = build_frame(MacAddr::BROADCAST, mac(9), Some(VlanTag::new(11)), ETHERTYPE_ARP, &p.to_bytes()).unwrap();
    let prio = build_frame(MacAddr::BROADCAST, mac(9), Some(VlanTag::new(0)), ETHERTYPE_ARP, &p.to_bytes()).unwrap();
    let now = net.now;
    net.s(c).handle_frame(0, &wrong, now);
    assert!(neighbor(&net, c, ip("10.10.0.9")).is_none());
    net.s(c).handle_frame(0, &prio, now);
    assert!(neighbor(&net, c, ip("10.10.0.9")).is_some());
}

#[test]
fn loopback_to_own_ip_and_localhost() {
    let mut net = two_hosts(30);
    net.clear_capture();
    for target in ["10.0.0.1", "127.0.0.1"] {
        let l = net.s(A).tcp_listen(sa("0.0.0.0", 8000), 4).unwrap();
        let now = net.now;
        let c = net.s(A).tcp_connect(sa(target, 8000), now).unwrap();
        // loopback completes synchronously
        assert_eq!(net.stacks[A].tcp_state(c), Ok(TcpState::Established));
        let s = net.s(A).tcp_accept(l).unwrap().unwrap();
        assert_eq!(net.stacks[A].tcp_peer_addr(s).unwrap(), net.stacks[A].tcp_local_addr(c).unwrap());
        let data = pattern(300_000, 11);
        let got = {
            let mut sent = 0;
            let mut got = Vec::new();
            let mut buf = vec![0u8; 5000];
            let mut closed = false;
            assert!(net.run(60_000, |n| {
                let now = n.now;
                while sent < data.len() {
                    match n.s(A).tcp_send(c, &data[sent..], now) {
                        Ok(k) => sent += k,
                        Err(_) => break,
                    }
                }
                if sent == data.len() && !closed {
                    n.s(A).tcp_close(c, now);
                    closed = true;
                }
                loop {
                    match n.s(A).tcp_recv(s, &mut buf) {
                        Ok(0) => return true,
                        Ok(k) => got.extend_from_slice(&buf[..k]),
                        Err(_) => return false,
                    }
                }
            }));
            got
        };
        assert!(got == data);
        let now = net.now;
        net.s(A).tcp_close(s, now);
        net.s(A).tcp_close(l, now);
    }
    // ping ourselves
    let ic = net.s(A).icmp_open().unwrap();
    let echo = IcmpPacket::build_echo(ICMP_ECHO_REQUEST, 3, 4, b"me");
    let now = net.now;
    net.s(A).icmp_send(ic, ip("10.0.0.1"), &echo, now).unwrap();
    let mut buf = [0u8; 64];
    let (from, len) = net.s(A).icmp_recv(ic, &mut buf).unwrap().unwrap();
    assert_eq!(from, ip("10.0.0.1"));
    assert_eq!(IcmpPacket::parse(&buf[..len]).unwrap().0.icmp_type, ICMP_ECHO_REPLY);
    // UDP to 127.0.0.1
    let u1 = net.s(A).udp_bind(sa("127.0.0.1", 5555)).unwrap();
    let u2 = net.s(A).udp_bind(sa("0.0.0.0", 0)).unwrap();
    net.s(A).udp_send_to(u2, sa("127.0.0.1", 5555), b"lo", now).unwrap();
    assert_eq!(net.s(A).udp_recv_from(u1, &mut buf).unwrap().unwrap().1, 2);
    // nothing touched the wire
    assert!(net.capture.iter().all(|f| f.node != A), "loopback leaked {} frames", net.capture.len());
}

#[test]
fn interface_management() {
    let mut s = Stack::new(StackConfig { seed: 5 });
    assert_eq!(s.add_interface("", mac(1)), None);
    assert_eq!(s.add_interface("a-very-long-interface-name", mac(1)), None);
    for i in 0..MAX_INTERFACES {
        assert_eq!(s.add_interface(&format!("eth{i}"), mac(i as u8)), Some(i));
    }
    assert_eq!(s.add_interface("extra", mac(99)), None);
    assert_eq!(s.add_interface("eth0", mac(99)), None);
    assert_eq!(s.iface_count(), MAX_INTERFACES);
    s.configure_addr(3, ip("192.168.3.1"), 24, 0);
    s.configure_addr(3, ip("192.168.4.1"), 24, 0); // replaces the connected route
    assert_eq!(s.routes().len(), 1);
    assert_eq!(s.routes()[0], Route { dst: ip("192.168.4.0"), prefix: 24, gateway: Ipv4Addr::ZERO, iface: 3 });
    s.configure_addr(3, ip("192.168.4.1"), 40, 0); // ignored
    assert_eq!(s.iface(3).unwrap().prefix, 24);
    // gratuitous ARP was queued
    assert!(s.pop_tx().is_some());
    while s.pop_tx().is_some() {}
    let u = s.udp_bind(sa("192.168.4.1", 99)).unwrap();
    let l = s.tcp_listen(sa("192.168.4.1", 99), 1).unwrap();
    s.add_route(ip("0.0.0.0"), 0, ip("192.168.4.254"), 3).unwrap();
    s.remove_interface(3);
    assert!(s.iface(3).is_none());
    assert_eq!(s.find_iface("eth3"), None);
    assert!(s.routes().is_empty());
    let mut buf = [0u8; 4];
    assert_eq!(s.udp_recv_from(u, &mut buf), Err(NetError::ConnectionAborted));
    assert_eq!(s.tcp_state(l), Ok(TcpState::Closed));
    assert_eq!(s.iface_count(), MAX_INTERFACES);
    s.remove_interface(15);
    assert_eq!(s.iface_count(), 15);
    assert_eq!(s.add_interface("svi10", mac(50)), Some(3));
    assert_eq!(s.neighbors(99).count(), 0);
    s.set_link(99, false, 0);
    s.handle_frame(99, &[1, 2, 3], 0);
    s.clear_addr(3);
    s.clear_addr(77);
}

#[test]
fn admin_down_drops_traffic_and_link_up_sends_garp() {
    let mut net = two_hosts(31);
    let srv = net.s(B).udp_bind(sa("0.0.0.0", 9)).unwrap();
    let cli = net.s(A).udp_bind(sa("0.0.0.0", 0)).unwrap();
    let now = net.now;
    net.s(B).set_admin_up(0, false, now);
    net.s(A).udp_send_to(cli, sa("10.0.0.2", 9), b"x", now).unwrap();
    net.advance(5000);
    let mut buf = [0u8; 4];
    assert_eq!(net.s(B).udp_recv_from(srv, &mut buf), Ok(None));
    assert!(net.stacks[B].iface(0).unwrap().stats.rx_dropped > 0);
    // no route while our only iface is down
    let now = net.now;
    net.s(B).set_link(0, false, now);
    net.s(B).set_admin_up(0, true, now);
    assert_eq!(net.s(B).udp_send_to(srv, sa("10.0.0.1", 9), b"x", now), Err(NetError::NoRoute));
    net.clear_capture();
    net.s(B).set_link(0, true, now);
    net.pump();
    let garp = net.capture.iter().find(|f| f.node == B).expect("gratuitous ARP");
    let (_, p) = EthHeader::parse(&garp.frame).unwrap();
    let a = ArpPacket::parse(p).unwrap();
    assert_eq!((a.sender_ip, a.target_ip), (ip("10.0.0.2"), ip("10.0.0.2")));
    // stats move
    let st = net.stacks[A].iface(0).unwrap().stats;
    assert!(st.tx_packets > 0 && st.tx_bytes > 0 && st.rx_packets > 0 && st.rx_bytes > 0);
}
