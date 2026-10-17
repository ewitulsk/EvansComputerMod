//! IP multicast: joining groups, sending to them, the group MAC filter, port sharing
//! with SO_REUSEADDR, loopback to this computer's own members, and IGMPv2 reports,
//! leaves and query answers.

mod common;

use common::*;
use ecm_net::eth::ETHERTYPE_IPV4;
use ecm_net::igmp::{self, IgmpMessage};
use ecm_net::ipv4::Ipv4Header;
use ecm_net::*;

const GROUP: &str = "224.0.0.254";

/// Three hosts on one segment: 10.0.0.1, .2, .3.
fn three_hosts() -> Net {
    let mut net = Net::new(11);
    let a = net.host(1, "10.0.0.1/24");
    let b = net.host(2, "10.0.0.2/24");
    let c = net.host(3, "10.0.0.3/24");
    net.segment(&[(a, 0), (b, 0), (c, 0)]);
    net.pump();
    net
}

fn recv_all(net: &mut Net, node: usize, h: SocketHandle) -> Vec<(SocketAddr, Vec<u8>)> {
    let mut out = Vec::new();
    let mut buf = [0u8; 2048];
    while let Ok(Some((from, n))) = net.s(node).udp_recv_from(h, &mut buf) {
        out.push((from, buf[..n].to_vec()));
    }
    out
}

/// IGMP messages captured from `node`: (destination IP, message, TTL).
fn igmp_sent(net: &Net, node: usize) -> Vec<(Ipv4Addr, IgmpMessage, u8)> {
    net.capture
        .iter()
        .filter(|c| c.node == node && c.eth().ethertype == ETHERTYPE_IPV4)
        .filter_map(|c| {
            let (_, ip_part) = ecm_net::eth::EthHeader::parse(&c.frame)?;
            let (h, payload) = Ipv4Header::parse_checked(ip_part).ok()?;
            (h.protocol == igmp::PROTO_IGMP).then(|| (h.dst, IgmpMessage::parse(payload).unwrap(), h.ttl))
        })
        .collect()
}

#[test]
fn members_receive_a_group_and_others_do_not() {
    let mut net = three_hosts();
    let g = ip(GROUP);
    let b = net.s(1).udp_bind(sa("0.0.0.0", 5354)).unwrap();
    net.s(1).udp_join_group(b, g, Ipv4Addr::ZERO).unwrap();
    // C listens on the port but never joined: its NIC filter drops the group's frames.
    let c = net.s(2).udp_bind(sa("0.0.0.0", 5354)).unwrap();
    let a = net.s(0).udp_bind(sa("0.0.0.0", 0)).unwrap();
    let now = net.now;
    net.s(0).udp_send_to(a, SocketAddr::new(g, 5354), b"who is there", now).unwrap();
    net.advance(10);
    let got = recv_all(&mut net, 1, b);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0.ip, ip("10.0.0.1"));
    assert_eq!(got[0].1, b"who is there");
    assert!(recv_all(&mut net, 2, c).is_empty(), "control: not a member");
    // The frame went to the group's MAC with TTL 1 (link only), no ARP for it.
    let f = net.capture.iter().find(|c| c.node == 0 && c.eth().dst == igmp::group_mac(g)).expect("the group frame");
    let (_, ip_part) = ecm_net::eth::EthHeader::parse(&f.frame).unwrap();
    assert_eq!(Ipv4Header::parse_checked(ip_part).unwrap().0.ttl, 1);
    assert_eq!(net.stacks[1].multicast_groups(0), vec![igmp::ALL_HOSTS, g]);
    // Leaving: B stops receiving.
    net.s(1).udp_leave_group(b, g, Ipv4Addr::ZERO).unwrap();
    let now = net.now;
    net.s(0).udp_send_to(a, SocketAddr::new(g, 5354), b"again", now).unwrap();
    net.advance(10);
    assert!(recv_all(&mut net, 1, b).is_empty(), "left the group");
    assert_eq!(net.s(1).udp_join_group(b, ip("10.1.2.3"), Ipv4Addr::ZERO), Err(NetError::InvalidInput), "not a group");
}

#[test]
fn reuseaddr_sockets_share_a_port_and_each_get_a_copy() {
    let mut net = three_hosts();
    let g = ip(GROUP);
    // Two programs on B: both set SO_REUSEADDR before binding the port.
    let mk = |net: &mut Net| {
        let h = net.s(1).udp_bind(sa("0.0.0.0", 0)).unwrap();
        net.s(1).udp_set_reuse(h, true).unwrap();
        let h = net.s(1).udp_rebind(h, sa("0.0.0.0", 5354)).unwrap();
        net.s(1).udp_join_group(h, g, Ipv4Addr::ZERO).unwrap();
        h
    };
    let b1 = mk(&mut net);
    let b2 = mk(&mut net);
    // Control: a third without SO_REUSEADDR can't take the port.
    assert_eq!(net.s(1).udp_bind(sa("0.0.0.0", 5354)), Err(NetError::AddrInUse));
    let a = net.s(0).udp_bind(sa("0.0.0.0", 0)).unwrap();
    let now = net.now;
    net.s(0).udp_send_to(a, SocketAddr::new(g, 5354), b"x", now).unwrap();
    net.s(0).udp_send_to(a, sa("10.0.0.255", 5354), b"bcast", now).unwrap();
    net.advance(10);
    for h in [b1, b2] {
        let got: Vec<Vec<u8>> = recv_all(&mut net, 1, h).into_iter().map(|x| x.1).collect();
        assert_eq!(got, vec![b"x".to_vec(), b"bcast".to_vec()]);
    }
    // One closes: the other is still a member, so no leave goes out.
    net.clear_capture();
    net.s(1).udp_close(b1);
    net.pump();
    assert!(igmp_sent(&net, 1).is_empty());
    net.s(1).udp_close(b2);
    net.pump();
    let left = igmp_sent(&net, 1);
    assert_eq!(left.len(), 1);
    assert_eq!((left[0].0, left[0].1.kind, left[0].1.group), (igmp::ALL_ROUTERS, igmp::LEAVE_GROUP, g));
}

#[test]
fn a_hosts_own_members_get_its_multicast_unless_loop_is_off() {
    let mut net = three_hosts();
    let g = ip(GROUP);
    let listen = net.s(0).udp_bind(sa("0.0.0.0", 5354)).unwrap();
    net.s(0).udp_join_group(listen, g, Ipv4Addr::ZERO).unwrap();
    let send = net.s(0).udp_bind(sa("0.0.0.0", 0)).unwrap();
    let now = net.now;
    net.s(0).udp_send_to(send, SocketAddr::new(g, 5354), b"me too", now).unwrap();
    net.pump();
    let got = recv_all(&mut net, 0, listen);
    assert_eq!(got.len(), 1, "looped back");
    assert_eq!(got[0].0.ip, ip("10.0.0.1"));
    net.s(0).udp_set_multicast_loop(send, false).unwrap();
    let now = net.now;
    net.s(0).udp_send_to(send, SocketAddr::new(g, 5354), b"not me", now).unwrap();
    net.pump();
    assert!(recv_all(&mut net, 0, listen).is_empty(), "IP_MULTICAST_LOOP off");
}

#[test]
fn the_multicast_interface_and_ttl_are_the_sockets_choice() {
    // A has two links: eth0 10.0.0.1 to B, eth1 10.1.0.1 to C.
    let mut net = Net::new(5);
    let mut s = Stack::new(StackConfig { seed: 9 });
    let e0 = s.add_interface("eth0", mac(1)).unwrap();
    let e1 = s.add_interface("eth1", mac(4)).unwrap();
    s.configure_addr(e0, ip("10.0.0.1"), 24, net.now);
    s.configure_addr(e1, ip("10.1.0.1"), 24, net.now);
    let a = net.add_stack(s);
    let b = net.host(2, "10.0.0.2/24");
    let c = net.host(3, "10.1.0.3/24");
    net.segment(&[(a, 0), (b, 0)]);
    net.segment(&[(a, 1), (c, 0)]);
    net.pump();
    let g = ip(GROUP);
    let hb = net.s(b).udp_bind(sa("0.0.0.0", 5354)).unwrap();
    net.s(b).udp_join_group(hb, g, Ipv4Addr::ZERO).unwrap();
    let hc = net.s(c).udp_bind(sa("0.0.0.0", 5354)).unwrap();
    net.s(c).udp_join_group(hc, g, Ipv4Addr::ZERO).unwrap();
    let ha = net.s(a).udp_bind(sa("0.0.0.0", 0)).unwrap();
    net.s(a).udp_set_multicast_if(ha, ip("10.1.0.1")).unwrap();
    net.s(a).udp_set_multicast_ttl(ha, 4).unwrap();
    let now = net.now;
    net.s(a).udp_send_to(ha, SocketAddr::new(g, 5354), b"eth1", now).unwrap();
    net.advance(10);
    let got = recv_all(&mut net, c, hc);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0.ip, ip("10.1.0.1"), "sent from eth1's address");
    assert!(recv_all(&mut net, b, hb).is_empty(), "control: not out of eth0");
    let ttl = net
        .capture
        .iter()
        .filter(|f| f.node == a && f.iface == 1 && f.eth().dst == igmp::group_mac(g))
        .map(|f| Ipv4Header::parse_checked(ecm_net::eth::EthHeader::parse(&f.frame).unwrap().1).unwrap().0.ttl)
        .next();
    assert_eq!(ttl, Some(4));
    assert_eq!(net.s(a).udp_set_multicast_if(ha, ip("10.9.9.9")), Err(NetError::InvalidInput));
}

#[test]
fn joins_are_reported_and_queries_answered() {
    let mut net = three_hosts();
    let g = ip(GROUP);
    net.clear_capture();
    let h = net.s(1).udp_bind(sa("0.0.0.0", 5354)).unwrap();
    net.s(1).udp_join_group(h, g, Ipv4Addr::ZERO).unwrap();
    // A second member on the same computer: no second report.
    let h2 = net.s(1).udp_bind(sa("0.0.0.0", 0)).unwrap();
    net.s(1).udp_join_group(h2, g, Ipv4Addr::ZERO).unwrap();
    assert_eq!(net.s(1).udp_join_group(h2, g, Ipv4Addr::ZERO), Err(NetError::AddrInUse));
    net.pump();
    let sent = igmp_sent(&net, 1);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!((sent[0].0, sent[0].1.kind, sent[0].1.group, sent[0].2), (g, igmp::V2_MEMBERSHIP_REPORT, g, 1));
    // A router's general query to 224.0.0.1: B reports its group (never 224.0.0.1).
    net.clear_capture();
    let q = IgmpMessage { kind: igmp::MEMBERSHIP_QUERY, max_resp: 100, group: Ipv4Addr::ZERO }.to_bytes();
    let pkt = ecm_net::ipv4::build_packet(ip("10.0.0.3"), igmp::ALL_HOSTS, igmp::PROTO_IGMP, 1, &q).unwrap();
    let frame = ecm_net::eth::build_frame(igmp::group_mac(igmp::ALL_HOSTS), mac(3), None, ETHERTYPE_IPV4, &pkt).unwrap();
    let now = net.now;
    net.s(1).handle_frame(0, &frame, now);
    net.pump();
    let sent = igmp_sent(&net, 1);
    assert_eq!(sent.iter().map(|s| (s.1.kind, s.1.group)).collect::<Vec<_>>(), vec![(igmp::V2_MEMBERSHIP_REPORT, g)]);
    // Control: A, a member of nothing, answers the same query with nothing.
    net.clear_capture();
    net.s(0).handle_frame(0, &frame, now);
    net.pump();
    assert!(igmp_sent(&net, 0).is_empty());
}
