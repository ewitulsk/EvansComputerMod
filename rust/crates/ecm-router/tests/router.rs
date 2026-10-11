use ecm_net::{
    dhcp::{Client, Message, State, ACK, DISCOVER, REQUEST},
    ipv4::{build_packet, Ipv4Header},
    udp, *,
};
use ecm_router::{dhcp::*, nat::*, *};
fn ip(s: &str) -> Ipv4Addr {
    Ipv4Addr::parse(s).unwrap()
}
fn udp_packet(src: &str, dst: &str, s: u16, d: u16) -> Vec<u8> {
    let (src, dst) = (ip(src), ip(dst));
    build_packet(src, dst, 17, 1, &udp::build(src, dst, s, d, b"payload")).unwrap()
}
#[test]
fn nat_endpoint_independent_mapping_checksums_and_timeout() {
    let mut n = Nat::new();
    let mut p = udp_packet("192.168.1.2", "8.8.8.8", 1234, 53);
    n.outbound(&mut p, ip("100.65.1.2"), &[], 0).unwrap();
    let (h, seg) = Ipv4Header::parse(&p).unwrap();
    let (u, _) = udp::UdpHeader::parse(seg).unwrap();
    assert_eq!(h.src, ip("100.65.1.2"));
    assert!(udp::verify_checksum(&h.src, &h.dst, seg));
    let mapped = u.src_port;
    let mut p = udp_packet("192.168.1.2", "1.1.1.1", 1234, 53);
    n.outbound(&mut p, ip("100.65.1.2"), &[], 1).unwrap();
    assert_eq!(n.mappings.len(), 1);
    assert_eq!(n.mappings[0].external, mapped);
    let mut reply = udp_packet("1.1.1.1", "100.65.1.2", 53, mapped);
    n.inbound(&mut reply, ip("100.65.1.2"), &[], 2).unwrap();
    let (h, seg) = Ipv4Header::parse(&reply).unwrap();
    assert_eq!(h.dst, ip("192.168.1.2"));
    assert_eq!(udp::UdpHeader::parse(seg).unwrap().0.dst_port, 1234);
    assert!(udp::verify_checksum(&h.src, &h.dst, seg));
    n.expire(UDP_TIMEOUT + 1);
    assert_eq!(n.mappings.len(), 1);
    n.expire(UDP_TIMEOUT + 2);
    assert!(n.mappings.is_empty());
    assert!(n
        .inbound(
            &mut udp_packet("8.8.8.8", "100.65.1.2", 53, mapped),
            ip("100.65.1.2"),
            &[],
            UDP_TIMEOUT + 3
        )
        .is_err());
}
#[test]
fn tcp_lifetime_and_static_forward() {
    let mut n = Nat::new();
    let src = ip("192.168.1.2");
    let dst = ip("8.8.8.8");
    for (time, flags, timeout) in [
        (0, 2, TCP_TRANSITORY_TIMEOUT),
        (1, 16, TCP_ESTABLISHED_TIMEOUT),
        (2, 17, TCP_TRANSITORY_TIMEOUT),
    ] {
        let seg = ecm_net::tcp::build_segment(src, dst, 1234, 443, 1, 1, flags, 8192, None, &[]);
        let mut p = build_packet(src, dst, 6, 1, &seg).unwrap();
        n.outbound(&mut p, ip("100.65.1.2"), &[], time).unwrap();
        assert_eq!(n.mappings[0].expires, time + timeout);
    }
    let f = Forward {
        protocol: 17,
        inside: src,
        inside_port: 53,
        outside_port: 1053,
    };
    let mut p = udp_packet("8.8.8.8", "100.65.1.2", 1234, 1053);
    n.inbound(&mut p, ip("100.65.1.2"), &[f], 1).unwrap();
    assert_eq!(Ipv4Header::parse(&p).unwrap().0.dst, src);
}
#[test]
fn quoted_udp_icmp_checksum_and_tcp_eight_byte_quote() {
    for proto in [6, 17] {
        let mut nat = Nat::new();
        let src = ip("192.168.1.2");
        let dst = ip("8.8.8.8");
        let outside = ip("100.65.1.2");
        let segment = if proto == 17 {
            udp::build(src, dst, 1234, 53, b"test")
        } else {
            ecm_net::tcp::build_segment(src, dst, 1234, 443, 1, 0, 2, 8192, None, &[])
        };
        let mut packet = build_packet(src, dst, proto, 1, &segment).unwrap();
        nat.outbound(&mut packet, outside, &[], 0).unwrap();
        let mut error = vec![3, 3, 0, 0, 0, 0, 0, 0];
        error.extend_from_slice(&packet[..28]);
        let sum = ecm_net::checksum::internet_checksum(&error);
        error[2..4].copy_from_slice(&sum.to_be_bytes());
        let mut reply = build_packet(dst, outside, 1, 2, &error).unwrap();
        nat.inbound(&mut reply, outside, &[], 1).unwrap();
        let (h, body) = Ipv4Header::parse(&reply).unwrap();
        assert_eq!(h.dst, src);
        assert_eq!(ecm_net::checksum::internet_checksum(body), 0);
        assert_eq!(&body[20..24], &src.0);
        assert_eq!(&body[28..30], &1234u16.to_be_bytes());
        if proto == 17 {
            assert_eq!(&body[34..36], &segment[6..8]);
        }
    }
}
#[test]
fn inside_icmp_errors_and_static_forward_quotes_translate_and_corruption_is_dropped() {
    let outside = ip("100.65.1.2");
    let inside = ip("192.168.1.2");
    let remote = ip("8.8.8.8");
    for static_forward in [false, true] {
        let mut nat = Nat::new();
        let forwards = if static_forward {
            vec![Forward {
                protocol: 17,
                inside,
                inside_port: 1234,
                outside_port: 1053,
            }]
        } else {
            vec![]
        };
        let mut original = udp_packet("192.168.1.2", "8.8.8.8", 1234, 53);
        nat.outbound(&mut original, outside, &forwards, 0).unwrap();
        let external = udp::UdpHeader::parse(Ipv4Header::parse(&original).unwrap().1)
            .unwrap()
            .0
            .src_port;
        let mut incoming = udp_packet("8.8.8.8", "100.65.1.2", 53, external);
        nat.inbound(&mut incoming, outside, &forwards, 1).unwrap();
        let quoted_checksum = &incoming[26..28];
        let mut error = vec![3, 3, 0, 0, 0, 0, 0, 0];
        error.extend_from_slice(&incoming[..28]);
        let sum = ecm_net::checksum::internet_checksum(&error);
        error[2..4].copy_from_slice(&sum.to_be_bytes());
        let mut packet = build_packet(inside, remote, 1, 2, &error).unwrap();
        nat.outbound(&mut packet, outside, &forwards, 2).unwrap();
        let (h, body) = Ipv4Header::parse(&packet).unwrap();
        assert_eq!(h.src, outside);
        assert_eq!(ecm_net::checksum::internet_checksum(body), 0);
        assert_eq!(&body[24..28], &outside.0);
        assert_eq!(&body[30..32], &external.to_be_bytes());
        assert_ne!(&body[34..36], quoted_checksum);
    }
    let mut nat = Nat::new();
    let mut corrupt = udp_packet("192.168.1.2", "8.8.8.8", 1234, 53);
    corrupt[28] ^= 1;
    let unchanged = corrupt.clone();
    assert!(nat.outbound(&mut corrupt, outside, &[], 0).is_err());
    assert_eq!(corrupt, unchanged);
    assert!(nat.mappings.is_empty());
}
#[test]
fn dhcp_dora_renew_release_and_persistence() {
    let mut cfg = Config::default();
    cfg.pools.insert(
        "lan".into(),
        Pool {
            start: ip("192.168.1.10"),
            end: ip("192.168.1.11"),
            router: ip("192.168.1.1"),
            dns: ip("1.1.1.1"),
            lease_secs: 8,
            enabled: true,
        },
    );
    let mut s = Server::new();
    let mac = MacAddr([2, 0, 0, 0, 0, 1]);
    let mut c = Client::new(mac, 1, 0);
    let (_, d) = c.poll(0).unwrap();
    assert_eq!(d.kind, DISCOVER);
    let offer = s
        .receive(
            &cfg.pools,
            &Message::parse(&d.encode()).unwrap(),
            ip("192.168.1.1"),
            0,
        )
        .unwrap();
    assert!(!c.receive(&offer, 0));
    assert_eq!(c.state, State::Requesting);
    let (_, r) = c.poll(0).unwrap();
    assert_eq!(r.kind, REQUEST);
    let ack = s.receive(&cfg.pools, &r, ip("192.168.1.1"), 0).unwrap();
    assert_eq!(ack.kind, ACK);
    assert!(c.receive(&ack, 0));
    assert_eq!(c.lease.as_ref().unwrap().address, ip("192.168.1.10"));
    let text = s.render();
    let mut restored = Server::new();
    restored.restore(&text, 1);
    assert_eq!(restored.leases.len(), 1);
    let (dst, renew) = c.poll(4000).unwrap();
    assert_eq!(dst, ip("192.168.1.1"));
    assert_eq!(renew.ciaddr, ip("192.168.1.10"));
    let ack = restored.receive(&cfg.pools, &renew, dst, 4000).unwrap();
    assert!(c.receive(&ack, 4000));
    assert_eq!(c.lease.as_ref().unwrap().expires, 12000);
    let mut release = Message::new(ecm_net::dhcp::RELEASE, 1, mac);
    release.ciaddr = renew.ciaddr;
    assert!(restored.receive(&cfg.pools, &release, dst, 5000).is_none());
    assert!(restored.leases.is_empty());
    c.poll(12000);
    assert!(c.lease.is_none());
    assert_eq!(c.state, State::Selecting);
}
#[test]
fn dhcp_exhaustion_nak_and_other_server_control() {
    let mut cfg = Config::default();
    cfg.pools.insert(
        "lan".into(),
        Pool {
            start: ip("192.168.1.10"),
            end: ip("192.168.1.10"),
            router: ip("192.168.1.1"),
            dns: ip("1.1.1.1"),
            lease_secs: 8,
            enabled: true,
        },
    );
    let mut s = Server::new();
    let mut m = Message::new(DISCOVER, 1, MacAddr([2, 0, 0, 0, 0, 1]));
    let server = ip("192.168.1.1");
    assert!(s.receive(&cfg.pools, &m, server, 0).is_some());
    m.mac.0[5] = 2;
    assert!(s.receive(&cfg.pools, &m, server, 0).is_none());
    m.kind = REQUEST;
    m.requested = Some(ip("192.168.1.10"));
    assert_eq!(
        s.receive(&cfg.pools, &m, server, 0).unwrap().kind,
        ecm_net::dhcp::NAK
    );
    m.server = Some(ip("192.168.1.9"));
    assert!(s.receive(&cfg.pools, &m, server, 0).is_none());
}
#[test]
fn config_roundtrip_and_invalid_command_control() {
    let text="configure terminal\nip routing\ninterface eth0\nrouting\nip address 192.168.1.1/24\nip nat inside\nexit\ninterface eth1\nip dhcp\nip nat outside\nexit\nip route 0.0.0.0/0 100.65.1.1 eth1\ndhcp-server vrf default\npool lan\nrange 192.168.1.10 192.168.1.200\ndefault-router 192.168.1.1\ndns-server 1.1.1.1\nlease 86400\nenable\nend\n";
    let c = cli::load(text).unwrap();
    let rendered = c.render();
    assert_eq!(cli::load(&rendered).unwrap().render(), rendered);
    assert!(cli::load("configure terminal\ninterface eth0\nip address garbage").is_err());
}
