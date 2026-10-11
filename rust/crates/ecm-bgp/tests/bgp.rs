use ecm_bgp::wire::Message;
use ecm_bgp::*;
use ecm_net::Ipv4Addr;
#[test]
fn as_sets_preserve_membership_and_count_once_for_best_path() {
    let mut attributes = Attributes::default();
    attributes.path = vec![65002, 70000, 80000, 65003];
    attributes.path_sets = vec![(1, 2)];
    attributes.next_hop = ip(1);
    let update = Message::Update {
        withdrawn: vec![],
        attributes: attributes.clone(),
        nlri: vec![prefix(1)],
    };
    for as4 in [false, true] {
        let encoded = update.encode(as4);
        let (decoded, _) = Message::decode(&encoded, as4).unwrap().unwrap();
        assert_eq!(decoded, update);
    }
    assert_eq!(attributes.path_length(), 3);
    let short = Route {
        prefix: prefix(1),
        attributes,
        peer: Some(ip(1)),
        external: true,
        router_id: ip(1),
    };
    let mut long = short.clone();
    long.attributes.path_sets.clear();
    assert!(better(&short, &long));
    assert!(!better(&long, &short));
}
#[test]
fn malformed_origin_is_withdrawn_without_reset_and_unknown_transitive_survives() {
    let mut nodes = ring(2);
    nodes[0].connected(ip(1), ip(0), 0);
    nodes[1].connected(ip(0), ip(1), 0);
    drain(&mut nodes, None, 0);
    let mut a = Attributes::default();
    a.path = vec![65002];
    a.next_hop = ip(1);
    a.unknown_transitive.push((99, vec![1, 2, 3]));
    let update = Message::Update {
        withdrawn: vec![],
        attributes: a.clone(),
        nlri: vec![prefix(1)],
    };
    let wire = update.encode_for_peer(true, false);
    let (decoded, _) = Message::decode(&wire, true).unwrap().unwrap();
    if let Message::Update { attributes, .. } = decoded {
        assert_eq!(attributes.unknown_transitive, a.unknown_transitive);
    } else {
        panic!("not update");
    }
    nodes[0].receive(ip(1), &wire, 1);
    assert!(nodes[0].rib.contains_key(&prefix(1)));
    let mut corrupt = wire;
    corrupt[26] = 3;
    nodes[0].receive(ip(1), &corrupt, 2);
    assert_eq!(nodes[0].peers[&ip(1)].state, State::Established);
    assert!(!nodes[0].rib.contains_key(&prefix(1)));
}
fn ip(i: usize) -> Ipv4Addr {
    Ipv4Addr::new(10, i as u8, 0, 1)
}
fn prefix(i: usize) -> Prefix {
    Prefix::new(Ipv4Addr::new(100, 65 + i as u8, 0, 0), 16)
}
fn ring(count: usize) -> Vec<Engine> {
    (0..count)
        .map(|i| {
            let mut c = Config::default();
            c.asn = 65001 + i as u32;
            c.router_id = ip(i);
            c.keepalive = 1;
            c.hold = 3;
            for j in [(i + 1) % count, (i + count - 1) % count] {
                let mut n = Neighbor::new(65001 + j as u32);
                n.active = true;
                c.neighbors.insert(ip(j), n);
            }
            let mut e = Engine::new(c, Policy::default(), 0);
            e.set_origins(&[prefix(i)]);
            e
        })
        .collect()
}
fn drain(nodes: &mut [Engine], cut: Option<(usize, usize)>, now: i64) {
    for _ in 0..1000 {
        let mut sent = Vec::new();
        for (i, e) in nodes.iter_mut().enumerate() {
            while let Some(a) = e.pop_action() {
                if let Action::Send(dst, b) = a {
                    sent.push((i, dst.0[1] as usize, b));
                }
            }
        }
        if sent.is_empty() {
            return;
        }
        for (a, b, data) in sent {
            if !cut.is_some_and(|(x, y)| (a == x && b == y) || (a == y && b == x)) {
                nodes[b].receive(ip(a), &data, now);
            }
        }
    }
    panic!("BGP did not quiesce");
}
#[test]
fn pair_state_machine_capabilities_and_hold_timer() {
    let mut e = ring(2);
    e[0].poll(0);
    assert_eq!(e[0].peers[&ip(1)].state, State::Connect);
    e[0].connected(ip(1), ip(0), 0);
    e[1].connected(ip(0), ip(1), 0);
    drain(&mut e, None, 0);
    assert_eq!(e[0].peers[&ip(1)].state, State::Established);
    assert!(e[0].peers[&ip(1)].as4);
    assert!(e[0].peers[&ip(1)].refresh);
    assert_eq!(e[0].rib[&prefix(1)].attributes.path, vec![65002]);
    e[0].poll(3001);
    assert_eq!(e[0].peers[&ip(1)].state, State::Active);
    assert!(!e[0].rib.contains_key(&prefix(1)));
}
#[test]
fn ring10_cut_withdraws_and_reroutes_long_way() {
    let mut e = ring(10);
    for i in 0..10 {
        for j in [(i + 1) % 10, (i + 9) % 10] {
            e[i].connected(ip(j), ip(i), 0);
        }
    }
    drain(&mut e, None, 0);
    for n in &e {
        assert_eq!(n.rib.len(), 10);
        assert!(n.peers.values().all(|p| p.state == State::Established));
    }
    assert_eq!(e[0].rib[&prefix(2)].attributes.path.len(), 2);
    e[0].disconnected(ip(1), 100);
    e[1].disconnected(ip(0), 100);
    drain(&mut e, Some((0, 1)), 100);
    assert_eq!(e[0].rib[&prefix(2)].attributes.path.len(), 8);
    assert_eq!(e[0].rib[&prefix(2)].peer, Some(ip(9)));
    // Cut both sides: local routes survive, remote reachability disappears.
    e[0].disconnected(ip(9), 200);
    e[9].disconnected(ip(0), 200);
    drain(&mut e, Some((0, 9)), 200);
    assert_eq!(e[0].rib.len(), 1);
}
#[test]
fn decision_process_local_pref_path_origin_med_external_id() {
    let mut a = Route {
        prefix: prefix(0),
        attributes: Attributes::default(),
        peer: Some(ip(1)),
        external: true,
        router_id: ip(1),
    };
    let mut b = a.clone();
    b.router_id = ip(2);
    assert!(better(&a, &b));
    b.external = false;
    assert!(better(&a, &b));
    b.external = true;
    b.attributes.med = 10;
    assert!(better(&a, &b));
    b.attributes.med = 0;
    b.attributes.origin = 2;
    assert!(better(&a, &b));
    b.attributes.origin = 0;
    b.attributes.path = vec![1];
    assert!(better(&a, &b));
    a.attributes.local_pref = 90;
    assert!(better(&b, &a));
}
#[test]
fn wire_roundtrips_4byte_as_legacy_and_tcp_stream_boundaries() {
    let open = Message::Open {
        asn: 4200000001,
        hold: 180,
        id: ip(0),
        as4: true,
        refresh: true,
    };
    let wire = open.encode(true);
    assert_eq!(Message::decode(&wire, true).unwrap().unwrap().0, open);
    for n in 0..wire.len() {
        assert_eq!(Message::decode(&wire[..n], true), Ok(None));
    }
    let mut a = Attributes::default();
    a.path = vec![65001, 4200000001];
    a.next_hop = ip(1);
    a.communities = vec![(65001 << 16) | 1];
    let m = Message::Update {
        withdrawn: vec![prefix(0)],
        attributes: a,
        nlri: vec![prefix(1)],
    };
    for four in [false, true] {
        assert_eq!(
            Message::decode(&m.encode(four), four).unwrap().unwrap().0,
            m
        );
    }
    for len in 0..200 {
        let bytes = vec![len as u8; len];
        let _ = Message::decode(&bytes, true);
    }
}
#[test]
fn loop_and_prefix_policy_controls() {
    let mut e = ring(2);
    e[0].connected(ip(1), ip(0), 0);
    e[1].connected(ip(0), ip(1), 0);
    drain(&mut e, None, 0);
    let mut a = Attributes::default();
    a.path = vec![65002, 65001];
    a.next_hop = ip(1);
    e[0].receive(
        ip(1),
        &Message::Update {
            withdrawn: Vec::new(),
            attributes: a,
            nlri: vec![prefix(9)],
        }
        .encode(true),
        1,
    );
    assert!(!e[0].rib.contains_key(&prefix(9)));
    let mut p = Policy::default();
    p.lists.insert(
        "village".into(),
        vec![PrefixRule {
            seq: 10,
            permit: true,
            prefix: prefix(1),
            min: 16,
            max: 24,
        }],
    );
    p.maps.insert(
        "customer".into(),
        vec![MapRule {
            seq: 10,
            permit: true,
            prefix_list: Some("village".into()),
            local_pref: Some(200),
            med: None,
            community: None,
        }],
    );
    let mut a = Attributes::default();
    assert!(p.apply(Some("customer"), prefix(1), &mut a));
    assert_eq!(a.local_pref, 200);
    assert!(!p.apply(Some("customer"), prefix(2), &mut a));
}
