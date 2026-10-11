mod common;
use common::*;
use ecm_net::*;
use ecm_net::{eth::EthHeader, icmp::IcmpPacket, ipv4::Ipv4Header};

fn topology(forward: bool) -> Net {
    let mut n = Net::new(100);
    let a = n.host(1, "192.168.1.2/24");
    let r = n.host(2, "192.168.1.1/24");
    let b = n.host(3, "10.0.0.2/24");
    let now = n.now;
    n.s(r).add_interface("eth1", mac(4));
    n.s(r).configure_addr(1, ip("10.0.0.1"), 24, now);
    n.s(r).set_forwarding(forward);
    n.s(a)
        .add_route(Ipv4Addr::ZERO, 0, ip("192.168.1.1"), 0)
        .unwrap();
    n.s(b)
        .add_route(Ipv4Addr::ZERO, 0, ip("10.0.0.1"), 0)
        .unwrap();
    n.segment(&[(a, 0), (r, 0)]);
    n.segment(&[(r, 1), (b, 0)]);
    n
}

#[test]
fn forwards_udp_decrements_ttl_and_disabled_control() {
    for enabled in [false, true] {
        let mut n = topology(enabled);
        let now = n.now;
        let a = n.s(0).udp_bind(sa("192.168.1.2", 9000)).unwrap();
        let b = n.s(2).udp_bind(sa("10.0.0.2", 9000)).unwrap();
        n.s(0)
            .udp_send_to(a, sa("10.0.0.2", 9000), b"routed", now)
            .unwrap();
        n.run(100, |_| false);
        let mut buf = [0; 100];
        let recv = n.s(2).udp_recv_from(b, &mut buf).unwrap();
        assert_eq!(recv.is_some(), enabled);
        if enabled {
            assert_eq!(&buf[..6], b"routed");
            let ttl = n
                .capture
                .iter()
                .filter(|f| f.node == 1 && f.iface == 1)
                .find_map(|f| {
                    let (_, p) = EthHeader::parse(&f.frame)?;
                    let (h, _) = Ipv4Header::parse(p)?;
                    (h.protocol == 17).then_some(h.ttl)
                })
                .unwrap();
            assert_eq!(ttl, 63);
        }
    }
}
#[test]
fn ttl_expired_quotes_original_and_rate_limits() {
    let mut n = topology(true);
    let now = n.now;
    let h = n.s(0).icmp_open().unwrap();
    n.s(0).icmp_set_ttl(h, 1).unwrap();
    let request = IcmpPacket::build_echo(8, 44, 1, b"test");
    for _ in 0..4 {
        n.s(0).icmp_send(h, ip("10.0.0.2"), &request, now).unwrap();
    }
    n.run(100, |_| false);
    let mut buf = [0; 256];
    let (from, len) = n.s(0).icmp_recv(h, &mut buf).unwrap().unwrap();
    assert_eq!(from, ip("192.168.1.1"));
    let (e, quote) = IcmpPacket::parse(&buf[..len]).unwrap();
    assert_eq!((e.icmp_type, e.code), (11, 0));
    assert_eq!(quote[8], 1);
    assert_eq!(&quote[24..28], &request[4..8]);
    assert!(n.s(0).icmp_recv(h, &mut buf).unwrap().is_none());
}
#[test]
fn route_and_port_unreachable() {
    let mut n = topology(true);
    let now = n.now;
    let h = n.s(0).icmp_open().unwrap();
    let u = n.s(0).udp_bind(sa("192.168.1.2", 9000)).unwrap();
    n.s(0)
        .udp_send_to(u, sa("8.8.8.8", 1234), b"x", now)
        .unwrap();
    n.run(100, |_| false);
    let mut buf = [0; 256];
    let (_, len) = n.s(0).icmp_recv(h, &mut buf).unwrap().unwrap();
    assert_eq!((buf[0], buf[1]), (3, 0));
    assert!(IcmpPacket::parse(&buf[..len]).is_some());
    let now = n.now;
    n.s(0)
        .udp_send_to(u, sa("10.0.0.2", 1234), b"x", now)
        .unwrap();
    n.run(100, |_| false);
    let (_, len) = n.s(0).icmp_recv(h, &mut buf).unwrap().unwrap();
    assert_eq!((buf[0], buf[1]), (3, 3));
    assert!(IcmpPacket::parse(&buf[..len]).is_some());
}
#[test]
fn route_priorities_survive_readdress_and_bind() {
    let mut s = Stack::new(StackConfig::default());
    for i in 0..2 {
        s.add_interface(&format!("eth{i}"), mac(i));
    }
    s.configure_addr(0, ip("10.0.0.1"), 24, 0);
    s.configure_addr(1, ip("10.1.0.1"), 24, 0);
    s.add_protocol_route(
        ip("10.0.0.0"),
        24,
        ip("10.1.0.2"),
        1,
        RouteSource::Bgp,
        20,
        10,
    )
    .unwrap();
    assert_eq!(s.lookup_route(ip("10.0.0.10")).unwrap().0, 0);
    s.configure_addr(0, ip("10.2.0.1"), 24, 1);
    assert_eq!(s.lookup_route(ip("10.0.0.10")).unwrap().0, 1);
    assert_eq!(
        s.routes()
            .iter()
            .filter(|r| r.source == RouteSource::Connected && r.iface == 0)
            .count(),
        1
    );
    let h = s
        .tcp_connect_bound(sa("10.2.0.1", 32000), sa("10.1.0.2", 179), 1)
        .unwrap();
    assert_eq!(s.tcp_local_addr(h).unwrap(), sa("10.2.0.1", 32000));
}
