//! L2 basics: learning, flooding, unknown unicast, ageing, MAC moves,
//! VLAN isolation, reserved addresses, SVI delivery and send_local.

mod common;
use common::*;
use ecm_bridge::{frame, MacAddr, PortRef, FDB_CAPACITY};

fn one_switch(hosts: u8) -> Net {
    let mut n = Net::new(1);
    let b = n.add_bridge(4);
    for i in 0..hosts {
        let h = n.add_host(i + 1);
        n.link(End::B(b, i as usize), End::H(h));
    }
    let ports: Vec<usize> = (0..hosts as usize).collect();
    n.l2_ports(b, &ports);
    n.run_for(100);
    n
}

#[test]
fn learning_flooding_unknown_unicast_and_ageing() {
    let mut n = one_switch(3);
    let (h0, h1, h2) = (host_mac(1), host_mac(2), host_mac(3));

    // Broadcast floods to everyone but the sender and teaches h0's port.
    n.send(0, data(BCAST, h0));
    n.run_for(20);
    assert_eq!((n.hosts[0].rx.len(), n.hosts[1].count_from(h0), n.hosts[2].count_from(h0)), (0, 1, 1));
    assert_eq!(n.bridges[0].fdb_lookup(MacAddr(h0), 1), Some(PortRef::Eth(0)));

    // Known unicast goes only to its port.
    n.clear_host_rx();
    n.send(1, data(h0, h1));
    n.run_for(20);
    assert_eq!((n.hosts[0].count_from(h1), n.hosts[2].rx.len()), (1, 0));

    // Unknown unicast floods.
    n.clear_host_rx();
    n.send(0, data(host_mac(99), h0));
    n.run_for(20);
    assert_eq!((n.hosts[1].count_from(h0), n.hosts[2].count_from(h0)), (1, 1));

    // Now h0 -> h1 is known.
    n.clear_host_rx();
    n.send(0, data(h1, h0));
    n.run_for(20);
    assert_eq!((n.hosts[1].count_from(h0), n.hosts[2].rx.len()), (1, 0));
    // Destination on the ingress segment is filtered.
    n.clear_host_rx();
    n.send(0, data(h0, h0));
    n.run_for(20);
    assert!(n.hosts.iter().all(|h| h.rx.is_empty()));

    // Ageing at the configured time (15 s).
    n.cli(0, "mac-address-table age-time 15");
    n.send(2, data(BCAST, h2));
    n.run_for(20);
    let learned = n.now;
    assert!(n.bridges[0].fdb_lookup(MacAddr(h2), 1).is_some());
    n.run_for(14_500 - (n.now - learned));
    assert!(n.bridges[0].fdb_lookup(MacAddr(h2), 1).is_some(), "aged too early");
    n.run_for(1_000);
    assert!(n.bridges[0].fdb_lookup(MacAddr(h2), 1).is_none(), "not aged at 15 s");
}

#[test]
fn mac_move_is_tracked_and_statics_win() {
    let mut n = one_switch(3);
    let h0 = host_mac(1);
    n.send(0, data(BCAST, h0));
    n.run_for(20);
    // The same MAC now appears on port 2.
    n.send(2, data(BCAST, h0));
    n.run_for(20);
    let e = n.bridges[0].fdb_entries().into_iter().find(|e| e.mac == h0).unwrap();
    assert_eq!((e.port, e.prev_port, e.move_count), (PortRef::Eth(2), Some(PortRef::Eth(0)), 1));
    let out = n.cli(0, "show mac-address-table mac-move");
    assert!(out.contains("02:aa:00:00:00:01   1       2     0     1"), "{}", out);
    // Static entry overrides learning.
    n.cli(0, "static-mac 02:aa:00:00:00:01 vlan 1 port 1");
    n.send(2, data(BCAST, h0));
    n.run_for(20);
    assert_eq!(n.bridges[0].fdb_lookup(MacAddr(h0), 1), Some(PortRef::Eth(1)));
    n.clear_host_rx();
    n.send(2, data(h0, host_mac(3)));
    n.run_for(20);
    assert_eq!((n.hosts[1].rx.len(), n.hosts[0].rx.len()), (1, 0));
}

#[test]
fn routed_ports_do_not_bridge() {
    let mut n = Net::new(2);
    let b = n.add_bridge(3);
    for i in 0..3u8 {
        let h = n.add_host(i + 1);
        n.link(End::B(b, i as usize), End::H(h));
    }
    n.l2_ports(b, &[0, 1]);
    assert!(n.bridges[b].is_l2_port(0) && !n.bridges[b].is_l2_port(2));
    n.send(0, data(BCAST, host_mac(1)));
    n.send(2, data(BCAST, host_mac(3)));
    n.run_for(20);
    assert_eq!(n.hosts[1].rx.len(), 1);
    assert_eq!(n.hosts[2].rx.len(), 0, "routed port received bridged traffic");
    assert_eq!(n.hosts[0].rx.len(), 0, "frame from routed port was bridged");
}

#[test]
fn vlan_isolation_access_and_trunk() {
    let mut n = Net::new(3);
    let a = n.add_bridge(4);
    let b = n.add_bridge(4);
    let hosts: Vec<usize> = (1..=5).map(|i| n.add_host(i)).collect();
    n.link(End::B(a, 0), End::H(hosts[0])); // A vlan 10
    n.link(End::B(a, 1), End::H(hosts[1])); // A vlan 20
    n.link(End::B(b, 0), End::H(hosts[2])); // B vlan 10
    n.link(End::B(b, 1), End::H(hosts[3])); // B vlan 20
    let trunk = n.link(End::B(a, 3), End::B(b, 3));
    let sniff = n.link(End::B(a, 2), End::H(hosts[4])); // trunk native 1 allowed 1,10
    n.capture(trunk);
    for s in [a, b] {
        n.cli(
            s,
            "vlan 10\nno shutdown\nexit\nvlan 20\nno shutdown\nexit\ninterface eth0\nno routing\nvlan access 10\nexit\ninterface eth1\nno routing\nvlan access 20\nexit\ninterface eth3\nno routing\nvlan trunk native 1\nvlan trunk allowed 1,10,20\nexit",
        );
    }
    n.cli(a, "interface eth2\nno routing\nvlan trunk native 1\nvlan trunk allowed 1,10\nexit");
    n.run_for(50);
    let _ = sniff;

    n.send(hosts[0], data(BCAST, host_mac(1)));
    n.run_for(30);
    assert_eq!(n.hosts[hosts[2]].count_from(host_mac(1)), 1, "same VLAN across the trunk");
    assert_eq!(n.hosts[hosts[1]].rx.len() + n.hosts[hosts[3]].rx.len(), 0, "VLAN 20 saw VLAN 10 traffic");
    // Tagged with VID 10 on the trunk and toward the sniffer; untagged at the edge.
    let on_trunk: Vec<_> = n.links[trunk].capture.as_ref().unwrap().iter().filter(|(_, f)| src_of(f) == host_mac(1)).collect();
    assert_eq!(on_trunk.len(), 1);
    assert_eq!(vid_of(&on_trunk[0].1), Some(10));
    assert_eq!(n.hosts[hosts[4]].rx.iter().map(|f| vid_of(f)).collect::<Vec<_>>(), vec![Some(10)]);
    assert_eq!(vid_of(&n.hosts[hosts[2]].rx[0]), None);

    // VLAN 20 is not allowed toward the sniffer.
    n.clear_host_rx();
    n.send(hosts[1], data(BCAST, host_mac(2)));
    n.run_for(30);
    assert_eq!(n.hosts[hosts[3]].count_from(host_mac(2)), 1);
    assert_eq!(n.hosts[hosts[4]].rx.len() + n.hosts[hosts[0]].rx.len() + n.hosts[hosts[2]].rx.len(), 0);

    // Access ports drop tagged ingress; trunks drop disallowed VIDs.
    n.clear_host_rx();
    n.send(hosts[0], frame::tag(&data(BCAST, host_mac(1)), 20, 0).unwrap());
    n.send(hosts[4], frame::tag(&data(BCAST, host_mac(5)), 20, 0).unwrap());
    n.run_for(30);
    assert!(n.hosts.iter().all(|h| h.rx.is_empty()));
    // Tagged VLAN 10 from the sniffer trunk reaches both VLAN 10 hosts.
    n.send(hosts[4], frame::tag(&data(BCAST, host_mac(5)), 10, 3).unwrap());
    n.run_for(30);
    assert_eq!(n.hosts[hosts[0]].count_from(host_mac(5)), 1);
    assert_eq!(n.hosts[hosts[2]].count_from(host_mac(5)), 1);

    // A shut-down VLAN drops traffic.
    n.clear_host_rx();
    n.cli(a, "vlan 10\nshutdown\nexit");
    n.send(hosts[0], data(BCAST, host_mac(1)));
    n.run_for(30);
    assert!(n.hosts.iter().all(|h| h.rx.is_empty()));
}

#[test]
fn trunk_native_tagged() {
    let mut n = Net::new(4);
    let a = n.add_bridge(2);
    let h0 = n.add_host(1);
    let h1 = n.add_host(2);
    n.link(End::B(a, 0), End::H(h0));
    n.link(End::B(a, 1), End::H(h1));
    n.cli(a, "interface eth0\nno routing\nexit\ninterface eth1\nno routing\nvlan trunk native 1 tag\nexit");
    n.run_for(20);
    n.send(h0, data(BCAST, host_mac(1)));
    n.send(h1, data(BCAST, host_mac(2))); // untagged: rejected
    n.run_for(20);
    assert_eq!(n.hosts[h1].rx.iter().map(|f| vid_of(f)).collect::<Vec<_>>(), vec![Some(1)]);
    assert!(n.hosts[h0].rx.is_empty());
}

#[test]
fn reserved_group_addresses_are_never_forwarded() {
    let mut n = one_switch(3);
    for last in 0u8..=0x0f {
        let dst = [0x01, 0x80, 0xc2, 0, 0, last];
        n.send(0, eth(dst, host_mac(1), 0x88b5, &[1; 46]));
        n.send(0, eth(dst, host_mac(1), 0x0026, &[0x42, 0x42, 0x03, 0, 0, 0, 0]));
        n.send(0, eth(dst, host_mac(1), 0x88cc, &[0; 46]));
        n.send(0, eth(dst, host_mac(1), 0x8809, &[1; 46]));
    }
    // Slow protocols are never forwarded, whatever the destination.
    n.send(0, eth(BCAST, host_mac(1), 0x8809, &[1; 46]));
    n.run_for(50);
    assert!(n.hosts.iter().all(|h| h.rx.is_empty()));
    assert!(n.hosts.iter().all(|h| h.ctrl_rx.iter().all(|f| src_of(f) != host_mac(1))), "reserved frame forwarded");
    // 01:80:c2:00:00:10 is not reserved.
    n.send(0, eth([0x01, 0x80, 0xc2, 0, 0, 0x10], host_mac(1), 0x88b5, &[1; 46]));
    n.run_for(20);
    assert_eq!(n.hosts[1].rx.len(), 1);
}

#[test]
fn fdb_is_bounded() {
    let mut n = one_switch(2);
    for i in 0..3000u32 {
        let src = [0x02, 0xcc, 0, (i >> 16) as u8, (i >> 8) as u8, i as u8];
        n.send(0, data(host_mac(2), src));
        if i % 50 == 0 {
            n.run_for(10);
        }
    }
    n.run_for(10);
    assert_eq!(n.bridges[0].fdb_len(), FDB_CAPACITY);
    let dynamic = n.bridges[0].fdb_entries().iter().filter(|e| !e.is_static).count();
    assert_eq!(dynamic, FDB_CAPACITY);
}

// ---------------------------------------------------------------------
// SVIs
// ---------------------------------------------------------------------

fn svi_net() -> Net {
    let mut n = Net::new(5);
    let a = n.add_bridge(4);
    for i in 1..=3u8 {
        let h = n.add_host(i);
        n.link(End::B(a, i as usize - 1), End::H(h));
    }
    n.cli(
        a,
        "vlan 10\nno shutdown\nexit\nvlan 20\nno shutdown\nexit\ninterface eth0\nno routing\nvlan access 10\nexit\ninterface eth1\nno routing\nvlan access 20\nexit\ninterface eth2\nno routing\nvlan trunk native 1\nexit\ninterface vlan 10\nip address 10.0.10.1/24\nexit",
    );
    n.run_for(20);
    n
}

#[test]
fn svi_receives_own_unicast_and_broadcasts_in_its_vlan() {
    let mut n = svi_net();
    let bm = bridge_mac(0);
    // ARP-like broadcast in VLAN 10: local copy (untagged) and flooded.
    n.send(0, eth(BCAST, host_mac(1), 0x0806, &[0; 28]));
    n.run_for(20);
    assert_eq!(n.locals[0].len(), 1);
    assert_eq!(n.locals[0][0].0, 10);
    assert_eq!(vid_of(&n.locals[0][0].1), None);
    assert_eq!(n.hosts[2].rx.iter().map(|f| vid_of(f)).collect::<Vec<_>>(), vec![Some(10)]);
    // Unicast to the bridge MAC: local only.
    n.clear_host_rx();
    n.send(0, data(bm, host_mac(1)));
    n.run_for(20);
    assert_eq!(n.locals[0].len(), 2);
    assert!(n.hosts.iter().all(|h| h.rx.is_empty()));
    // Tagged over the trunk: delivered untagged.
    n.send(2, frame::tag(&data(bm, host_mac(3)), 10, 0).unwrap());
    n.run_for(20);
    assert_eq!(n.locals[0].len(), 3);
    assert_eq!(vid_of(&n.locals[0][2].1), None);
    // VLAN 20 has no SVI: broadcast is not delivered locally.
    n.send(1, eth(BCAST, host_mac(2), 0x0806, &[0; 28]));
    n.run_for(20);
    assert_eq!(n.locals[0].len(), 3);
    // Frames to our MAC in a VLAN without an SVI are dropped, not forwarded.
    n.clear_host_rx();
    n.send(1, data(bm, host_mac(2)));
    n.run_for(20);
    assert!(n.hosts.iter().all(|h| h.rx.is_empty()));
    assert_eq!(n.locals[0].len(), 3);
}

#[test]
fn send_local_forwards_like_a_cpu_port() {
    let mut n = svi_net();
    let bm = bridge_mac(0);
    let now = n.now;
    // Broadcast from the stack: VLAN 10 members only, tagged on the trunk.
    n.bridges[0].send_local(10, &eth(BCAST, bm, 0x0806, &[0; 28]), now);
    n.settle();
    assert_eq!(n.hosts[0].count_from(bm), 1);
    assert_eq!(n.hosts[1].rx.len(), 0);
    assert_eq!(n.hosts[2].rx.iter().map(|f| vid_of(f)).collect::<Vec<_>>(), vec![Some(10)]);
    assert_eq!(n.bridges[0].fdb_lookup(MacAddr(bm), 10), Some(PortRef::Cpu));
    // A reply to the bridge MAC reaches the SVI.
    n.send(0, data(bm, host_mac(1)));
    n.run_for(20);
    assert_eq!(n.locals[0].len(), 1);
    // Unicast from the stack to a learned host goes only there.
    n.clear_host_rx();
    n.bridges[0].send_local(10, &data(host_mac(1), bm), n.now);
    n.settle();
    assert_eq!((n.hosts[0].rx.len(), n.hosts[2].rx.len()), (1, 0));
    // No SVI on VLAN 20: send_local is ignored.
    n.clear_host_rx();
    n.bridges[0].send_local(20, &eth(BCAST, bm, 0x0806, &[0; 28]), n.now);
    n.settle();
    assert!(n.hosts.iter().all(|h| h.rx.is_empty()));
    // Removing the SVI stops local delivery.
    n.cli(0, "no interface vlan 10");
    n.send(0, data(bm, host_mac(1)));
    n.run_for(20);
    assert_eq!(n.locals[0].len(), 1);
}
