//! Tests of the real kernel dispatcher (`Net`): host `Stack`s and switch
//! `Net`s wired together by cables in a virtual-time world. This exercises
//! exactly the code the kernel runs (RX ownership, bridge/stack routing,
//! SVIs, carrier), with in-memory NICs instead of the host.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use ecm_bridge::{AllowedList, Bridge, PortMode, PortRef};
use ecm_net::types::{Ipv4Addr, MacAddr, SocketAddr};
use ecm_net::{NetError, Stack, StackConfig, TcpState};

use super::{Net, Nics};

#[derive(Default)]
struct Fabric {
    inbox: VecDeque<(usize, Vec<u8>)>,
    outbox: Vec<(usize, Vec<u8>)>,
    carrier: Vec<bool>,
    macs: Vec<[u8; 6]>,
}

struct TestNics(Rc<RefCell<Fabric>>);

impl Nics for TestNics {
    fn count(&self) -> usize {
        self.0.borrow().macs.len()
    }
    fn mac(&self, port: usize) -> Option<[u8; 6]> {
        self.0.borrow().macs.get(port).copied()
    }
    fn tx(&mut self, port: usize, frame: &[u8]) {
        self.0.borrow_mut().outbox.push((port, frame.to_vec()));
    }
    fn rx(&mut self, buf: &mut [u8]) -> Option<(usize, usize)> {
        let (port, f) = self.0.borrow_mut().inbox.pop_front()?;
        let n = f.len().min(buf.len());
        buf[..n].copy_from_slice(&f[..n]);
        Some((port, n))
    }
    fn set_promiscuous(&mut self, _port: usize, _on: bool) {}
    fn carrier(&self, port: usize) -> bool {
        self.0.borrow().carrier.get(port).copied().unwrap_or(false)
    }
}

enum Node {
    Host(Stack),
    Switch(Net, Rc<RefCell<Fabric>>),
}

type End = (usize, usize); // (node, port)

struct World {
    nodes: Vec<Node>,
    links: Vec<(End, End)>,
    now: i64,
    /// Frames transmitted from each endpoint.
    tx_count: BTreeMap<End, u64>,
    /// Frames seen on the wire, for inspection (bounded).
    captured: Vec<(End, Vec<u8>)>,
    capture_from: Option<End>,
}

fn mac(node: usize, port: usize) -> MacAddr {
    MacAddr([0x02, 0x10, 0, 0, node as u8, port as u8])
}

impl World {
    fn new() -> Self {
        Self { nodes: Vec::new(), links: Vec::new(), now: 1_000, tx_count: BTreeMap::new(), captured: Vec::new(), capture_from: None }
    }

    fn host(&mut self, ip: [u8; 4]) -> usize {
        let n = self.nodes.len();
        let mut s = Stack::new(StackConfig { seed: n as u64 + 7 });
        s.add_interface("eth0", mac(n, 0));
        s.configure_addr(0, Ipv4Addr(ip), 24, self.now);
        self.nodes.push(Node::Host(s));
        n
    }

    fn switch(&mut self, ports: usize) -> usize {
        let n = self.nodes.len();
        let fab = Rc::new(RefCell::new(Fabric {
            carrier: vec![true; ports],
            macs: (0..ports).map(|p| mac(n, p).0).collect(),
            ..Default::default()
        }));
        let mut net = Net::with_nics(Box::new(TestNics(fab.clone())), n as u64, self.now);
        let bm = MacAddr([0x06, 0x10, 0, 0, n as u8, 0xFF]);
        net.attach_bridge(Bridge::new(&net.port_macs(), bm, self.now), self.now);
        self.nodes.push(Node::Switch(net, fab));
        n
    }

    fn net(&mut self, n: usize) -> &mut Net {
        match &mut self.nodes[n] {
            Node::Switch(net, _) => net,
            Node::Host(_) => panic!("node {} is a host", n),
        }
    }

    fn stack(&mut self, n: usize) -> &mut Stack {
        match &mut self.nodes[n] {
            Node::Host(s) => s,
            Node::Switch(net, _) => &mut net.stack,
        }
    }

    fn bridge(&mut self, n: usize) -> &mut Bridge {
        self.net(n).bridge_mut().expect("switch has a bridge")
    }

    fn access(&mut self, sw: usize, port: usize, vid: u16) {
        let b = self.bridge(sw);
        let _ = b.create_vlan(vid);
        let _ = b.set_vlan_active(vid, true);
        b.set_port_mode(PortRef::Eth(port), PortMode::Access { vid }).unwrap();
        self.net(sw).sync_bridge_ports();
    }

    fn trunk(&mut self, sw: usize, port: usize, vids: &[u16]) {
        let b = self.bridge(sw);
        for &v in vids {
            let _ = b.create_vlan(v);
            let _ = b.set_vlan_active(v, true);
        }
        let allowed = AllowedList::Some(vids.iter().copied().chain(core::iter::once(1)).collect());
        b.set_port_mode(PortRef::Eth(port), PortMode::Trunk { native: 1, native_tag: false, allowed }).unwrap();
        self.net(sw).sync_bridge_ports();
    }

    fn link(&mut self, a: End, b: End) {
        self.links.push((a, b));
    }

    fn unlink(&mut self, a: End) {
        self.links.retain(|(x, y)| *x != a && *y != a);
        if let Node::Switch(_, fab) = &self.nodes[a.0] {
            fab.borrow_mut().carrier[a.1] = false;
        }
    }

    fn peer(&self, e: End) -> Option<End> {
        self.links.iter().find_map(|&(a, b)| if a == e { Some(b) } else if b == e { Some(a) } else { None })
    }

    fn step(&mut self) {
        let now = self.now;
        let mut out: Vec<(End, Vec<u8>)> = Vec::new();
        for (n, node) in self.nodes.iter_mut().enumerate() {
            match node {
                Node::Host(s) => {
                    while let Some((i, f)) = s.pop_tx() {
                        out.push(((n, i), f));
                    }
                }
                Node::Switch(_, fab) => {
                    for (p, f) in fab.borrow_mut().outbox.drain(..) {
                        out.push(((n, p), f));
                    }
                }
            }
        }
        for (src, f) in out {
            *self.tx_count.entry(src).or_default() += 1;
            if self.capture_from == Some(src) && self.captured.len() < 4096 {
                self.captured.push((src, f.clone()));
            }
            let Some((dn, dp)) = self.peer(src) else { continue };
            match &mut self.nodes[dn] {
                Node::Host(s) => s.handle_frame(dp, &f, now),
                Node::Switch(_, fab) => fab.borrow_mut().inbox.push_back((dp, f)),
            }
        }
        for node in self.nodes.iter_mut() {
            match node {
                Node::Host(s) => {
                    s.poll(now);
                }
                Node::Switch(net, _) => {
                    net.rx(now);
                    net.poll(now);
                }
            }
        }
        self.now += 1;
    }

    fn run(&mut self, ms: i64) {
        for _ in 0..ms {
            self.step();
        }
    }

    fn run_until(&mut self, max_ms: i64, mut cond: impl FnMut(&mut World) -> bool) -> bool {
        for _ in 0..max_ms {
            if cond(self) {
                return true;
            }
            self.step();
        }
        cond(self)
    }

    fn total_tx(&self) -> u64 {
        self.tx_count.values().sum()
    }
}

/// Send one UDP datagram from `from` to `to_ip:port` and report whether it
/// arrived at `to` within `ms`.
fn udp_reaches(w: &mut World, from: usize, to: usize, to_ip: [u8; 4], port: u16, ms: i64) -> bool {
    let rx = w.stack(to).udp_bind(SocketAddr { ip: Ipv4Addr::ZERO, port }).unwrap();
    let tx = w.stack(from).udp_bind(SocketAddr { ip: Ipv4Addr::ZERO, port: 0 }).unwrap();
    let now = w.now;
    let _ = w.stack(from).udp_send_to(tx, SocketAddr { ip: Ipv4Addr(to_ip), port }, b"probe", now);
    let got = w.run_until(ms, |w| {
        let mut buf = [0u8; 64];
        matches!(w.stack(to).udp_recv_from(rx, &mut buf), Ok(Some((_, 5))))
    });
    w.stack(to).udp_close(rx);
    w.stack(from).udp_close(tx);
    got
}

#[test]
fn tcp_transfer_between_hosts_through_the_switch() {
    let mut w = World::new();
    let h1 = w.host([10, 0, 0, 1]);
    let h2 = w.host([10, 0, 0, 2]);
    let h3 = w.host([10, 0, 0, 3]);
    let sw = w.switch(3);
    for (port, h) in [h1, h2, h3].into_iter().enumerate() {
        w.access(sw, port, 1);
        w.link((h, 0), (sw, port));
    }

    let now = w.now;
    let l = w.stack(h3).tcp_listen(SocketAddr { ip: Ipv4Addr::ZERO, port: 80 }, 4).unwrap();
    let c = w.stack(h1).tcp_connect(SocketAddr { ip: Ipv4Addr([10, 0, 0, 3]), port: 80 }, now).unwrap();
    assert!(w.run_until(5_000, |w| w.stack(h1).tcp_state(c) == Ok(TcpState::Established)), "no handshake");
    let mut srv = None;
    assert!(w.run_until(1_000, |w| {
        srv = w.stack(h3).tcp_accept(l).unwrap();
        srv.is_some()
    }));
    let srv = srv.unwrap();

    let payload: Vec<u8> = (0..200_000u32).map(|i| (i * 31 % 251) as u8).collect();
    let (mut sent, mut got) = (0usize, Vec::new());
    for _ in 0..60_000 {
        if sent < payload.len() {
            let now = w.now;
            match w.stack(h1).tcp_send(c, &payload[sent..], now) {
                Ok(n) => sent += n,
                Err(NetError::WouldBlock) => {}
                Err(e) => panic!("send failed: {:?}", e),
            }
        }
        let mut buf = [0u8; 8192];
        while let Ok(n) = w.stack(h3).tcp_recv(srv, &mut buf) {
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        if got.len() == payload.len() {
            break;
        }
        w.step();
    }
    assert_eq!(got.len(), payload.len(), "transfer incomplete");
    assert!(got == payload, "payload corrupted in transit");
    // Learning happened: h2 only saw the flooding before the switch learned.
    let fdb = w.bridge(sw).fdb_len();
    assert!(fdb >= 2, "switch learned {} MACs", fdb);
    let to_h2 = w.tx_count.get(&(sw, 1)).copied().unwrap_or(0);
    assert!(to_h2 < 20, "{} frames flooded to the uninvolved host", to_h2);
}

#[test]
fn vlans_isolate_hosts_and_svi_answers_management_arp() {
    let mut w = World::new();
    let h1 = w.host([10, 0, 10, 1]);
    let h2 = w.host([10, 0, 10, 2]); // same subnet, but VLAN 20
    let h3 = w.host([10, 0, 10, 3]);
    let sw = w.switch(3);
    w.access(sw, 0, 10);
    w.access(sw, 1, 20);
    w.access(sw, 2, 10);
    for (port, h) in [h1, h2, h3].into_iter().enumerate() {
        w.link((h, 0), (sw, port));
    }
    let now = w.now;
    assert!(w.net(sw).set_svi(10, Ipv4Addr([10, 0, 10, 254]), 24, now));

    assert!(udp_reaches(&mut w, h1, h3, [10, 0, 10, 3], 7000, 3_000), "same VLAN must work");
    assert!(!udp_reaches(&mut w, h1, h2, [10, 0, 10, 2], 7001, 4_000), "VLAN 20 must be isolated");

    // Management: the switch's SVI answers ARP from VLAN 10, and its own
    // stack can reach hosts through the bridge.
    let probe = w.stack(h1).udp_bind(SocketAddr { ip: Ipv4Addr::ZERO, port: 0 }).unwrap();
    let now = w.now;
    let _ = w.stack(h1).udp_send_to(probe, SocketAddr { ip: Ipv4Addr([10, 0, 10, 254]), port: 9 }, b"x", now);
    assert!(
        w.run_until(3_000, |w| w.stack(h1).neighbors(0).any(|(ip, _, _)| ip == Ipv4Addr([10, 0, 10, 254]))),
        "SVI did not answer ARP"
    );
    assert!(udp_reaches(&mut w, sw, h3, [10, 0, 10, 3], 7002, 3_000), "switch management stack -> host");
}

#[test]
fn trunk_between_switches_carries_tagged_vlan_traffic() {
    let mut w = World::new();
    let h1 = w.host([10, 0, 30, 1]);
    let h2 = w.host([10, 0, 30, 2]);
    let s1 = w.switch(2);
    let s2 = w.switch(2);
    w.access(s1, 0, 30);
    w.access(s2, 0, 30);
    w.trunk(s1, 1, &[30]);
    w.trunk(s2, 1, &[30]);
    w.link((h1, 0), (s1, 0));
    w.link((h2, 0), (s2, 0));
    w.link((s1, 1), (s2, 1));
    w.capture_from = Some((s1, 1));
    assert!(udp_reaches(&mut w, h1, h2, [10, 0, 30, 2], 7100, 3_000), "VLAN 30 across the trunk");
    let tagged = w.captured.iter().filter(|(_, f)| f.len() > 16 && f[12] == 0x81 && f[13] == 0x00).count();
    assert!(tagged > 0, "frames on the trunk must carry 802.1Q tags");
    let wrong = w
        .captured
        .iter()
        .filter(|(_, f)| f.len() > 16 && f[12] == 0x81 && f[13] == 0x00)
        .filter(|(_, f)| (u16::from_be_bytes([f[14], f[15]]) & 0x0FFF) != 30)
        .count();
    assert_eq!(wrong, 0, "only VLAN 30 should be tagged on this trunk");
}

#[test]
fn spanning_tree_breaks_a_loop_between_two_switches() {
    let mut w = World::new();
    let h1 = w.host([10, 0, 40, 1]);
    let h2 = w.host([10, 0, 40, 2]);
    let s1 = w.switch(3);
    let s2 = w.switch(3);
    for sw in [s1, s2] {
        for p in 0..3 {
            w.access(sw, p, 1);
        }
        let b = w.bridge(sw);
        b.set_stp_forward_delay(4).unwrap();
        b.set_stp_enabled(true);
    }
    w.link((h1, 0), (s1, 0));
    w.link((h2, 0), (s2, 0));
    // Two parallel cables: a loop.
    w.link((s1, 1), (s2, 1));
    w.link((s1, 2), (s2, 2));
    w.run(12_000); // > 2 x forward delay

    let before = w.total_tx();
    assert!(udp_reaches(&mut w, h1, h2, [10, 0, 40, 2], 7200, 3_000), "hosts must reach each other across the loop");
    // Broadcast traffic after convergence must not circulate.
    let tx = w.stack(h1).udp_bind(SocketAddr { ip: Ipv4Addr::ZERO, port: 0 }).unwrap();
    for _ in 0..5 {
        let now = w.now;
        let _ = w.stack(h1).udp_send_to(tx, SocketAddr { ip: Ipv4Addr::BROADCAST, port: 9 }, b"b", now);
        w.run(100);
    }
    w.run(3_000);
    let frames = w.total_tx() - before;
    assert!(frames < 400, "loop not broken: {} frames in ~4.5 s", frames);
}

/// Control for the test above: the same loop without spanning tree must
/// storm, proving the frame-count check can detect one.
#[test]
fn the_same_loop_without_stp_storms() {
    let mut w = World::new();
    let h1 = w.host([10, 0, 41, 1]);
    let s1 = w.switch(3);
    let s2 = w.switch(3);
    for sw in [s1, s2] {
        for p in 0..3 {
            w.access(sw, p, 1);
        }
    }
    w.link((h1, 0), (s1, 0));
    w.link((s1, 1), (s2, 1));
    w.link((s1, 2), (s2, 2));
    w.run(100);
    let before = w.total_tx();
    let tx = w.stack(h1).udp_bind(SocketAddr { ip: Ipv4Addr::ZERO, port: 0 }).unwrap();
    let now = w.now;
    let _ = w.stack(h1).udp_send_to(tx, SocketAddr { ip: Ipv4Addr::BROADCAST, port: 9 }, b"b", now);
    w.run(1_000);
    let frames = w.total_tx() - before;
    assert!(frames > 1_000, "expected a broadcast storm, saw only {} frames", frames);
}

#[test]
fn pulled_cable_is_noticed_through_carrier() {
    let mut w = World::new();
    let h1 = w.host([10, 0, 50, 1]);
    let sw = w.switch(1);
    w.access(sw, 0, 1);
    w.link((h1, 0), (sw, 0));
    w.run(300);
    assert!(w.net(sw).carrier(0));
    w.unlink((sw, 0));
    w.run(600); // carrier is sampled every 250 ms
    assert!(!w.net(sw).carrier(0));
    assert!(!w.bridge(sw).link_up(0));
}
