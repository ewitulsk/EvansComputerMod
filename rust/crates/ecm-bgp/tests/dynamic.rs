//! Dynamic neighbors (listen ranges), peer groups and maximum-prefix.
use ecm_bgp::wire::Message;
use ecm_bgp::*;
use ecm_net::Ipv4Addr;

fn ip(s: &str) -> Ipv4Addr {
    Ipv4Addr::parse(s).unwrap()
}
fn pfx(s: &str) -> Prefix {
    let (a, l) = Ipv4Addr::parse_cidr(s).unwrap();
    Prefix::new(a, l)
}

/// One BGP speaker with its own address on the shared segment.
struct Node {
    addr: Ipv4Addr,
    e: Engine,
    sent: Vec<Message>,
}
struct Net {
    nodes: Vec<Node>,
}
impl Net {
    fn idx(&self, a: Ipv4Addr) -> Option<usize> {
        self.nodes.iter().position(|n| n.addr == a)
    }
    /// The TCP connection from `from` (active) to `to` (listener) completes.
    fn connect(&mut self, from: usize, to: usize, now: i64) -> Result<(), Reject> {
        let (fa, ta) = (self.nodes[from].addr, self.nodes[to].addr);
        self.nodes[to].e.accept(fa, now)?;
        self.nodes[to].e.connected(fa, ta, now);
        self.nodes[from].e.connected(ta, fa, now);
        Ok(())
    }
    /// Deliver messages until quiet. A Close tears the TCP connection down on both sides.
    fn pump(&mut self, now: i64) {
        for _ in 0..1000 {
            let mut work = Vec::new();
            for (i, n) in self.nodes.iter_mut().enumerate() {
                while let Some(a) = n.e.pop_action() {
                    work.push((i, a));
                }
            }
            if work.is_empty() {
                return;
            }
            for (i, a) in work {
                let src = self.nodes[i].addr;
                match a {
                    Action::Send(dst, b) => {
                        if let Some(j) = self.idx(dst) {
                            if let Ok(Some((m, _))) = Message::decode(&b, true) {
                                self.nodes[i].sent.push(m);
                            }
                            self.nodes[j].e.receive(src, &b, now);
                        }
                    }
                    Action::Close(dst) => {
                        if let Some(j) = self.idx(dst) {
                            if self.nodes[j].e.peers.get(&src).is_some_and(|p| {
                                !matches!(p.state, State::Idle | State::Active)
                            }) {
                                self.nodes[j].e.disconnected(src, now);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        panic!("did not quiesce");
    }
    fn state(&self, at: usize, peer: usize) -> Option<State> {
        self.nodes[at]
            .e
            .peers
            .get(&self.nodes[peer].addr)
            .map(|p| p.state)
    }
}

/// Village ISP 172.31.3.1 (AS 65003) with an open-peering group on 172.31.3.0/28.
fn village(rule: RemoteAs, tweak: impl FnOnce(&mut Config, &mut Policy)) -> Node {
    let mut c = Config::default();
    c.asn = 65003;
    c.router_id = ip("100.67.0.1");
    c.keepalive = 1;
    c.hold = 3;
    let mut g = Neighbor::default();
    g.remote_as = Some(rule);
    g.active = true;
    c.groups.insert("TAPS".into(), g);
    c.listen.push(ListenRange {
        prefix: pfx("172.31.3.0/28"),
        group: "TAPS".into(),
        as_range: None,
        limit: None,
    });
    let mut p = Policy::default();
    tweak(&mut c, &mut p);
    let mut e = Engine::new(c, p, 0);
    e.set_origins(&[pfx("100.67.0.0/24"), pfx("100.67.1.0/24")]);
    Node {
        addr: ip("172.31.3.1"),
        e,
        sent: Vec::new(),
    }
}
/// A player router at `addr` with AS `asn`, statically peering with the village.
fn tap(addr: &str, asn: u32, origins: &[&str]) -> Node {
    let mut c = Config::default();
    c.asn = asn;
    c.router_id = ip(addr);
    c.keepalive = 1;
    c.hold = 3;
    let mut n = Neighbor::new(65003);
    n.active = true;
    c.neighbors.insert(ip("172.31.3.1"), n);
    let mut e = Engine::new(c, Policy::default(), 0);
    let o: Vec<_> = origins.iter().map(|s| pfx(s)).collect();
    e.set_origins(&o);
    Node {
        addr: ip(addr),
        e,
        sent: Vec::new(),
    }
}
fn net(v: Node, taps: Vec<Node>) -> Net {
    let mut nodes = vec![v];
    nodes.extend(taps);
    let mut n = Net { nodes };
    n.pump(0);
    n
}
fn notified(n: &Node, code: u8, subcode: u8) -> bool {
    n.sent
        .iter()
        .any(|m| *m == Message::Notification { code, subcode })
}

#[test]
fn in_range_peer_is_accepted_learns_as_and_exchanges_routes() {
    let mut n = net(
        village(RemoteAs::External, |_, _| {}),
        vec![tap("172.31.3.5", 65200, &["10.200.0.0/24"])],
    );
    n.connect(1, 0, 0).unwrap();
    n.pump(0);
    assert_eq!(n.state(0, 1), Some(State::Established));
    assert_eq!(n.state(1, 0), Some(State::Established));
    let p = &n.nodes[0].e.peers[&ip("172.31.3.5")];
    assert_eq!(p.remote_as, 65200, "AS learned from the OPEN");
    assert_eq!(p.dynamic.as_deref(), Some("TAPS"));
    assert!(n.nodes[0].e.rib.contains_key(&pfx("10.200.0.0/24")));
    assert_eq!(
        n.nodes[0].e.rib[&pfx("10.200.0.0/24")].attributes.path,
        vec![65200]
    );
    assert!(n.nodes[0].e.rib[&pfx("10.200.0.0/24")].external);
    // The tap learns the village's prefixes with the village AS prepended.
    assert_eq!(
        n.nodes[1].e.rib[&pfx("100.67.0.0/24")].attributes.path,
        vec![65003]
    );
    let summary = n.nodes[0].e.show_summary(5000);
    assert!(summary.contains("*172.31.3.5"), "{}", summary);
    assert!(summary.contains("65200"), "{}", summary);
    assert!(summary.contains("Established"), "{}", summary);
    let detail = n.nodes[0].e.show_neighbors(Some(ip("172.31.3.5")), 5000);
    assert!(detail.contains("dynamic, peer-group TAPS, listen range 172.31.3.0/28"), "{}", detail);
}

#[test]
fn out_of_range_and_inactive_group_are_refused() {
    let mut v = village(RemoteAs::External, |_, _| {});
    assert_eq!(v.e.accept(ip("172.31.4.5"), 0), Err(Reject::Unknown));
    assert_eq!(v.e.accept(ip("172.31.3.16"), 0), Err(Reject::Unknown));
    assert!(v.e.peers.is_empty());
    let mut v = village(RemoteAs::External, |c, _| {
        c.groups.get_mut("TAPS").unwrap().active = false
    });
    assert_eq!(v.e.accept(ip("172.31.3.5"), 0), Err(Reject::Inactive));
}

#[test]
fn passive_dynamic_peer_never_connects_out() {
    let mut v = village(RemoteAs::External, |_, _| {});
    v.e.accept(ip("172.31.3.5"), 0).unwrap();
    for t in [0, 10_000, 60_000] {
        v.e.poll(t);
        while let Some(a) = v.e.pop_action() {
            assert!(!matches!(a, Action::Connect(_)), "dynamic peers are passive");
        }
    }
}

#[test]
fn external_rejects_own_as_and_internal_requires_it() {
    let mut n = net(
        village(RemoteAs::External, |_, _| {}),
        vec![tap("172.31.3.5", 65003, &[])],
    );
    n.connect(1, 0, 0).unwrap();
    n.pump(0);
    assert!(notified(&n.nodes[0], 2, 2), "Bad Peer AS");
    assert!(!n.nodes[0].e.peers.contains_key(&ip("172.31.3.5")), "removed");

    let mut n = net(
        village(RemoteAs::Internal, |_, _| {}),
        vec![tap("172.31.3.5", 65200, &[]), tap("172.31.3.6", 65003, &[])],
    );
    n.connect(1, 0, 0).unwrap();
    n.connect(2, 0, 0).unwrap();
    n.pump(0);
    assert!(notified(&n.nodes[0], 2, 2));
    assert_eq!(n.state(0, 1), None);
    assert_eq!(n.state(0, 2), Some(State::Established));
    assert_eq!(n.nodes[0].e.peers[&ip("172.31.3.6")].remote_as, 65003);
}

#[test]
fn explicit_asn_and_as_range() {
    let mut n = net(
        village(RemoteAs::Asn(65200), |_, _| {}),
        vec![tap("172.31.3.5", 65201, &[]), tap("172.31.3.6", 65200, &[])],
    );
    n.connect(1, 0, 0).unwrap();
    n.connect(2, 0, 0).unwrap();
    n.pump(0);
    assert_eq!(n.state(0, 1), None);
    assert_eq!(n.state(0, 2), Some(State::Established));

    let mut n = net(
        village(RemoteAs::External, |c, _| {
            c.listen[0].as_range = parse_as_range("65100-65199,65300");
        }),
        vec![
            tap("172.31.3.5", 65200, &[]),
            tap("172.31.3.6", 65150, &[]),
            tap("172.31.3.7", 65300, &[]),
        ],
    );
    for i in 1..4 {
        n.connect(i, 0, 0).unwrap();
    }
    n.pump(0);
    assert_eq!(n.state(0, 1), None, "65200 outside the as-range");
    assert_eq!(n.state(0, 2), Some(State::Established));
    assert_eq!(n.state(0, 3), Some(State::Established));
}

#[test]
fn range_limit_and_listen_limit() {
    let mut v = village(RemoteAs::External, |c, _| c.listen[0].limit = Some(2));
    v.e.accept(ip("172.31.3.5"), 0).unwrap();
    v.e.accept(ip("172.31.3.6"), 0).unwrap();
    assert_eq!(v.e.accept(ip("172.31.3.7"), 0), Err(Reject::Limit));
    // An address that already has a session is not a new peer.
    assert_eq!(v.e.accept(ip("172.31.3.6"), 0), Ok(()));
    let mut v = village(RemoteAs::External, |c, _| c.listen_limit = Some(1));
    v.e.accept(ip("172.31.3.5"), 0).unwrap();
    assert_eq!(v.e.accept(ip("172.31.3.6"), 0), Err(Reject::Limit));
}

#[test]
fn dynamic_peer_is_removed_on_disconnect_and_can_return() {
    let mut n = net(
        village(RemoteAs::External, |c, _| c.listen[0].limit = Some(1)),
        vec![tap("172.31.3.5", 65200, &["10.200.0.0/24"])],
    );
    n.connect(1, 0, 0).unwrap();
    n.pump(0);
    assert!(n.nodes[0].e.rib.contains_key(&pfx("10.200.0.0/24")));
    // Carrier loss / TCP reset.
    n.nodes[0].e.disconnected(ip("172.31.3.5"), 100);
    n.nodes[1].e.disconnected(ip("172.31.3.1"), 100);
    n.pump(100);
    assert!(n.nodes[0].e.peers.is_empty(), "dynamic peer removed");
    assert!(!n.nodes[0].e.rib.contains_key(&pfx("10.200.0.0/24")));
    // The slot is free again (limit 1), and the same router re-peers.
    n.connect(1, 0, 6000).unwrap();
    n.pump(6000);
    assert_eq!(n.state(0, 1), Some(State::Established));
    assert!(n.nodes[0].e.rib.contains_key(&pfx("10.200.0.0/24")));
}

#[test]
fn maximum_prefix_ceases_session_until_clear_or_restart() {
    let many = ["10.200.0.0/24", "10.200.1.0/24", "10.200.2.0/24"];
    let mut n = net(
        village(RemoteAs::External, |c, _| {
            c.groups.get_mut("TAPS").unwrap().maximum_prefix = Some(MaxPrefix::new(2))
        }),
        vec![tap("172.31.3.5", 65200, &many)],
    );
    n.connect(1, 0, 0).unwrap();
    n.pump(0);
    assert!(notified(&n.nodes[0], 6, 1), "Cease/Maximum Number of Prefixes Reached");
    assert!(!n.nodes[0].e.peers.contains_key(&ip("172.31.3.5")));
    assert!(n.nodes[0].e.rib.keys().all(|p| p.address.0[0] != 10));
    assert_eq!(n.nodes[0].e.accept(ip("172.31.3.5"), 600_000), Err(Reject::PrefixLimit));
    let detail = n.nodes[0].e.show_neighbors(None, 1000);
    assert!(detail.contains("until 'clear bgp'"), "{}", detail);
    n.nodes[0].e.clear(Some(ip("172.31.3.5")), 700_000);
    assert_eq!(n.nodes[0].e.accept(ip("172.31.3.5"), 700_000), Ok(()));

    // With `restart 30` the address may return after 30 s.
    let mut n = net(
        village(RemoteAs::External, |c, _| {
            let mut m = MaxPrefix::new(2);
            m.restart = Some(30);
            c.groups.get_mut("TAPS").unwrap().maximum_prefix = Some(m)
        }),
        vec![tap("172.31.3.5", 65200, &many)],
    );
    n.connect(1, 0, 0).unwrap();
    n.pump(0);
    assert_eq!(n.nodes[0].e.accept(ip("172.31.3.5"), 29_000), Err(Reject::PrefixLimit));
    assert_eq!(n.nodes[0].e.accept(ip("172.31.3.5"), 30_000), Ok(()));

    // Control: warning-only keeps the session and the prefixes.
    let mut n = net(
        village(RemoteAs::External, |c, _| {
            let mut m = MaxPrefix::new(2);
            m.warning_only = true;
            c.groups.get_mut("TAPS").unwrap().maximum_prefix = Some(m)
        }),
        vec![tap("172.31.3.5", 65200, &many)],
    );
    n.connect(1, 0, 0).unwrap();
    n.pump(0);
    assert_eq!(n.state(0, 1), Some(State::Established));
    assert_eq!(n.nodes[0].e.peers[&ip("172.31.3.5")].adj_in.len(), 3);
}

/// The village import filter: player prefixes up to /24, no village ranges,
/// no ring links, no default.
fn safe_filter(c: &mut Config, p: &mut Policy) {
    let rule = |seq, permit, s: &str, min, max| PrefixRule {
        seq,
        permit,
        prefix: pfx(s),
        min,
        max,
    };
    p.lists.insert(
        "TAP-IN".into(),
        vec![
            rule(10, false, "100.64.0.0/10", 10, 32),
            rule(20, false, "172.31.0.0/16", 16, 32),
            rule(30, false, "0.0.0.0/0", 0, 0),
            rule(40, true, "0.0.0.0/0", 0, 24),
        ],
    );
    p.maps.insert(
        "TAP-IN".into(),
        vec![MapRule {
            seq: 10,
            permit: true,
            prefix_list: Some("TAP-IN".into()),
            local_pref: None,
            med: None,
            community: None,
        }],
    );
    let g = c.groups.get_mut("TAPS").unwrap();
    g.inbound = Some("TAP-IN".into());
    g.maximum_prefix = Some(MaxPrefix::new(20));
}

#[test]
fn group_inbound_route_map_filters_hijacks() {
    let mut n = net(
        village(RemoteAs::External, safe_filter),
        vec![tap(
            "172.31.3.5",
            65200,
            &[
                "10.200.0.0/24",
                "10.201.0.0/16",
                "100.67.0.0/24",
                "100.65.0.0/24",
                "172.31.5.0/28",
                "0.0.0.0/0",
                "10.200.9.0/25",
            ],
        )],
    );
    n.connect(1, 0, 0).unwrap();
    n.pump(0);
    assert_eq!(n.state(0, 1), Some(State::Established));
    let rib = &n.nodes[0].e.rib;
    assert!(rib.contains_key(&pfx("10.200.0.0/24")));
    assert!(rib.contains_key(&pfx("10.201.0.0/16")));
    // Hijack of the village's own /24: the village keeps its local route.
    assert!(rib[&pfx("100.67.0.0/24")].peer.is_none());
    for bad in ["100.65.0.0/24", "172.31.5.0/28", "0.0.0.0/0", "10.200.9.0/25"] {
        assert!(!rib.contains_key(&pfx(bad)), "{} must be filtered", bad);
    }
    assert_eq!(n.nodes[0].e.peers[&ip("172.31.3.5")].adj_in.len(), 2);
}

#[test]
fn configured_neighbor_in_a_range_stays_static() {
    let mut v = village(RemoteAs::External, |c, _| {
        let mut nb = Neighbor::new(65004);
        nb.active = true;
        c.neighbors.insert(ip("172.31.3.2"), nb);
    });
    v.e.accept(ip("172.31.3.2"), 0).unwrap();
    let p = &v.e.peers[&ip("172.31.3.2")];
    assert!(!p.is_dynamic());
    assert_eq!(p.remote_as, 65004);
    // Static peers still connect out.
    v.e.poll(0);
    let mut connects = 0;
    while let Some(a) = v.e.pop_action() {
        if matches!(a, Action::Connect(x) if x == ip("172.31.3.2")) {
            connects += 1;
        }
    }
    assert_eq!(connects, 1);
}

#[test]
fn static_member_inherits_group_settings_and_static_external() {
    let mut c = Config::default();
    c.asn = 65003;
    let mut g = Neighbor::default();
    g.remote_as = Some(RemoteAs::Asn(65200));
    g.active = true;
    g.inbound = Some("IN".into());
    c.groups.insert("G".into(), g);
    let mut m = Neighbor::default();
    m.peer_group = Some("G".into());
    c.neighbors.insert(ip("10.0.0.2"), m);
    let mut x = Neighbor::default();
    x.remote_as = Some(RemoteAs::External);
    x.active = true;
    c.neighbors.insert(ip("10.0.0.3"), x);
    let e = Engine::new(c, Policy::default(), 0);
    let p = &e.peers[&ip("10.0.0.2")];
    assert_eq!(p.remote_as, 65200);
    assert!(p.config.active);
    assert_eq!(p.config.inbound.as_deref(), Some("IN"));
    assert_eq!(e.peers[&ip("10.0.0.3")].remote_as, 0, "learned at OPEN");
}

#[test]
fn longest_listen_range_wins() {
    let v = village(RemoteAs::External, |c, _| {
        c.groups.insert("WIDE".into(), Neighbor::default());
        c.listen.push(ListenRange {
            prefix: pfx("172.31.0.0/16"),
            group: "WIDE".into(),
            as_range: None,
            limit: None,
        });
    });
    assert_eq!(v.e.config.listen_range(ip("172.31.3.5")).unwrap().group, "TAPS");
    assert_eq!(v.e.config.listen_range(ip("172.31.9.5")).unwrap().group, "WIDE");
}

#[test]
fn as_range_parse_and_render() {
    assert_eq!(parse_as_range("65001-65010,65100"), Some(vec![(65001, 65010), (65100, 65100)]));
    assert_eq!(render_as_range(&[(65001, 65010), (65100, 65100)]), "65001-65010,65100");
    for bad in ["", "0", "5-3", "a", "1,,2", "1-"] {
        assert_eq!(parse_as_range(bad), None, "{}", bad);
    }
}
