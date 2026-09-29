//! RSTP convergence, loop prevention, re-election, guards.

mod common;
use common::*;
use ecm_bridge::bpdu::{self, Bpdu, BpduBody};
use ecm_bridge::{PortRef, StpPortState, StpRole};

const FD_MS: i64 = 15_000;

fn stp_bridge(n: &mut Net, ports: usize, prio: Option<u16>) -> usize {
    let b = n.add_bridge(ports);
    n.l2_ports(b, &(0..ports).collect::<Vec<_>>());
    n.cli(b, "spanning-tree");
    if let Some(p) = prio {
        n.cli(b, &format!("spanning-tree priority {}", p));
    }
    b
}

fn st(n: &Net, b: usize, p: usize) -> (StpRole, StpPortState) {
    let s = n.bridges[b].stp_port_status(PortRef::Eth(p)).expect("bridge port");
    (s.role, s.state)
}

fn discarding(n: &Net, ports: &[(usize, usize)]) -> Vec<(usize, usize)> {
    ports.iter().copied().filter(|&(b, p)| st(n, b, p).1 != StpPortState::Forwarding).collect()
}

/// Count data frames from `src` that crossed the given links.
fn crossings(n: &Net, links: &[usize], src: [u8; 6]) -> usize {
    links.iter().map(|&l| n.links[l].capture.as_ref().map(|c| c.iter().filter(|(_, f)| src_of(f) == src).count()).unwrap_or(0)).sum()
}

#[test]
fn two_bridge_ring_converges_to_one_blocked_port() {
    let mut n = Net::new(11);
    let a = stp_bridge(&mut n, 3, None);
    let b = stp_bridge(&mut n, 3, None);
    let ha = n.add_host(1);
    let hb = n.add_host(2);
    let l0 = n.link(End::B(a, 0), End::B(b, 0));
    let l1 = n.link(End::B(a, 1), End::B(b, 1));
    n.link(End::B(a, 2), End::H(ha));
    n.link(End::B(b, 2), End::H(hb));
    n.capture(l0);
    n.capture(l1);

    // Nothing forwards before the forward-delay timers run out: no storm.
    n.run_for(1_000);
    n.send(ha, data(BCAST, host_mac(1)));
    n.run_for(5_000);
    assert_eq!(n.hosts[hb].rx.len(), 0);
    assert_eq!(crossings(&n, &[l0, l1], host_mac(1)), 0);

    let inter = [(a, 0), (a, 1), (b, 0), (b, 1)];
    let t = n.run_until(2 * FD_MS + 5_000, |n| {
        discarding(n, &inter).len() == 1 && [(a, 2), (b, 2)].iter().all(|&(x, p)| st(n, x, p).1 == StpPortState::Forwarding)
    });
    let elapsed = n.now - 1_000;
    assert!(t.is_some(), "did not converge");
    assert!(elapsed <= 2 * FD_MS + 3_000, "converged too slowly: {} ms", elapsed);
    // A (lower MAC) is root; B blocks the higher port ID link.
    assert!(n.bridges[a].stp_status().is_root);
    assert_eq!(n.bridges[b].stp_status().root_port, Some(PortRef::Eth(0)));
    assert_eq!(st(&n, b, 1), (StpRole::Alternate, StpPortState::Discarding));
    assert_eq!(st(&n, a, 0), (StpRole::Designated, StpPortState::Forwarding));

    // A broadcast is delivered exactly once; the network stays quiet.
    n.clear_host_rx();
    let before_data = crossings(&n, &[l0, l1], host_mac(1));
    let before_total = n.link_total();
    n.send(ha, data(BCAST, host_mac(1)));
    n.run_for(4_000);
    assert_eq!(n.hosts[hb].count_from(host_mac(1)), 1);
    assert_eq!(n.hosts[ha].rx.len(), 0);
    // A's two designated ports each send one copy; B's alternate port drops one.
    assert_eq!(crossings(&n, &[l0, l1], host_mac(1)) - before_data, 2);
    assert!(n.link_total() - before_total < 40, "too many frames: {}", n.link_total() - before_total);
}

fn three_ring(n: &mut Net, root_prio: Option<u16>) -> ([usize; 3], [usize; 3], [usize; 3]) {
    let a = stp_bridge(n, 3, root_prio);
    let b = stp_bridge(n, 3, None);
    let c = stp_bridge(n, 3, None);
    let lab = n.link(End::B(a, 0), End::B(b, 0));
    let lbc = n.link(End::B(b, 1), End::B(c, 1));
    let lca = n.link(End::B(c, 0), End::B(a, 1));
    let hs = [n.add_host(1), n.add_host(2), n.add_host(3)];
    for (i, h) in hs.iter().enumerate() {
        n.link(End::B([a, b, c][i], 2), End::H(*h));
    }
    for l in [lab, lbc, lca] {
        n.capture(l);
    }
    ([a, b, c], [lab, lbc, lca], hs)
}

const RING_PORTS: [(usize, usize); 6] = [(0, 0), (0, 1), (1, 0), (1, 1), (2, 0), (2, 1)];

#[test]
fn three_bridge_ring_converges_and_broadcasts_once() {
    let mut n = Net::new(12);
    let (br, links, hs) = three_ring(&mut n, None);
    let t = n.run_until(2 * FD_MS + 5_000, |n| discarding(n, &RING_PORTS).len() == 1 && (0..3).all(|b| st(n, b, 2).1 == StpPortState::Forwarding));
    assert!(t.map(|t| t <= 2 * FD_MS + 3_000).unwrap_or(false), "convergence took {:?}", t);
    assert!(n.bridges[br[0]].stp_status().is_root);
    // B wins the B-C segment (same cost, lower bridge ID): C's port blocks.
    assert_eq!(discarding(&n, &RING_PORTS), vec![(2, 1)]);
    assert_eq!(st(&n, 2, 1).0, StpRole::Alternate);
    assert_eq!(n.bridges[br[2]].stp_status().root_path_cost, 20_000);

    for (i, &h) in hs.iter().enumerate() {
        n.clear_host_rx();
        let src = host_mac(i as u8 + 1);
        let before = crossings(&n, &links, src);
        let total = n.link_total();
        n.send(h, data(BCAST, src));
        n.run_for(3_000);
        for (j, &o) in hs.iter().enumerate() {
            let want = if i == j { 0 } else { 1 };
            assert_eq!(n.hosts[o].count_from(src), want, "host {} -> host {}", i, j);
        }
        // Two tree links plus one copy onto the blocked segment (discarded
        // by C's alternate port), and nothing more: no loop.
        assert_eq!(crossings(&n, &links, src) - before, 3);
        assert!(n.link_total() - total < 60);
    }
    // Everyone agrees on the root.
    let root = n.bridges[br[0]].stp_status().bridge_id;
    assert!(br.iter().all(|&b| n.bridges[b].stp_status().root_id == root));
}

fn probe_until_delivered(n: &mut Net, from: usize, src: [u8; 6], to: usize, max_ms: i64) -> Option<i64> {
    let start = n.now;
    while n.now - start <= max_ms {
        n.clear_host_rx();
        n.send(from, data(BCAST, src));
        n.run_for(500);
        if n.hosts[to].count_from(src) > 0 {
            return Some(n.now - start);
        }
    }
    None
}

#[test]
fn root_failure_link_down_reelects_and_restores() {
    let mut n = Net::new(13);
    let (br, links, hs) = three_ring(&mut n, Some(4096));
    n.run_for(2 * FD_MS + 3_000);
    assert_eq!(discarding(&n, &RING_PORTS), vec![(2, 1)]);
    // Unplug the root.
    n.set_link(links[0], false);
    n.set_link(links[2], false);
    let t = probe_until_delivered(&mut n, hs[1], host_mac(2), hs[2], 20_000 + 2 * FD_MS);
    assert!(t.is_some(), "connectivity not restored within max-age + 2*FD");
    let s_b = n.bridges[br[1]].stp_status();
    let s_c = n.bridges[br[2]].stp_status();
    assert!(s_b.is_root, "B should be the new root");
    assert_eq!(s_c.root_id, s_b.bridge_id);
    assert_eq!(s_c.root_port, Some(PortRef::Eth(1)));
    assert!(s_c.topology_changes >= 1);
}

#[test]
fn root_failure_silent_ages_out_and_restores() {
    let mut n = Net::new(14);
    let (br, links, hs) = three_ring(&mut n, Some(4096));
    n.run_for(2 * FD_MS + 3_000);
    // The root dies without carrier loss: its links go silent.
    n.set_drop(links[0], 1000, 1000);
    n.set_drop(links[2], 1000, 1000);
    let t = probe_until_delivered(&mut n, hs[1], host_mac(2), hs[2], 20_000 + 2 * FD_MS);
    assert!(t.map(|t| t <= 20_000 + 2 * FD_MS).unwrap_or(false), "restoration took {:?}", t);
    // Old root information must have been discarded everywhere.
    let b_id = n.bridges[br[1]].stp_status().bridge_id;
    assert_eq!(n.bridges[br[1]].stp_status().root_id, b_id);
    assert_eq!(n.bridges[br[2]].stp_status().root_id, b_id);
}

#[test]
fn link_failure_triggers_topology_change_and_flush() {
    let mut n = Net::new(15);
    let (br, links, hs) = three_ring(&mut n, None);
    n.run_for(2 * FD_MS + 3_000);
    // Teach every bridge where host B lives.
    n.send(hs[1], data(BCAST, host_mac(2)));
    n.run_for(500);
    assert_eq!(n.bridges[br[2]].fdb_lookup(ecm_bridge::MacAddr(host_mac(2)), 1), Some(PortRef::Eth(0)));
    let tc_before = n.bridges[br[2]].stp_status().topology_changes;
    // B loses its root port: C's alternate becomes designated.
    n.set_link(links[0], false);
    let t = probe_until_delivered(&mut n, hs[0], host_mac(1), hs[1], 2 * FD_MS + 5_000);
    assert!(t.is_some());
    assert!(n.bridges[br[2]].stp_status().topology_changes > tc_before);
    assert_eq!(st(&n, 2, 1), (StpRole::Designated, StpPortState::Forwarding));
    // The stale entry via A was flushed and relearned on the new path.
    n.send(hs[1], data(BCAST, host_mac(2)));
    n.run_for(500);
    assert_eq!(n.bridges[br[2]].fdb_lookup(ecm_bridge::MacAddr(host_mac(2)), 1), Some(PortRef::Eth(1)));
    n.clear_host_rx();
    n.send(hs[2], data(host_mac(2), host_mac(3)));
    n.run_for(500);
    assert_eq!(n.hosts[hs[1]].count_from(host_mac(3)), 1);
    assert_eq!(n.hosts[hs[0]].count_from(host_mac(3)), 0);
}

#[test]
fn self_loop_cable_gives_backup_port() {
    let mut n = Net::new(16);
    let a = stp_bridge(&mut n, 3, None);
    let h = n.add_host(1);
    n.link(End::B(a, 0), End::B(a, 1));
    n.link(End::B(a, 2), End::H(h));
    n.run_for(2 * FD_MS + 3_000);
    assert_eq!(st(&n, a, 0), (StpRole::Designated, StpPortState::Forwarding));
    assert_eq!(st(&n, a, 1), (StpRole::Backup, StpPortState::Discarding));
    let total = n.link_total();
    n.send(h, data(BCAST, host_mac(1)));
    n.run_for(2_000);
    assert!(n.link_total() - total < 20);
    assert_eq!(n.hosts[h].rx.len(), 0);
}

#[test]
fn bpdu_guard_err_disables_and_stays_disabled() {
    let mut n = Net::new(17);
    let a = stp_bridge(&mut n, 2, None);
    let b = stp_bridge(&mut n, 2, None);
    let ha = n.add_host(1);
    let hb = n.add_host(2);
    n.cli(a, "interface eth0\nspanning-tree bpdu-guard\nspanning-tree admin-edge-port\nexit");
    let l = n.link(End::B(a, 0), End::B(b, 0));
    n.link(End::B(a, 1), End::H(ha));
    n.link(End::B(b, 1), End::H(hb));
    n.run_for(1_000);
    assert!(n.bridges[a].is_err_disabled(PortRef::Eth(0)));
    assert!(n.logs[a].iter().any(|m| m.contains("bpdu-guard")));
    // Stays disabled while BPDUs keep arriving, and blocks traffic.
    n.run_for(2 * FD_MS + 30_000);
    assert!(n.bridges[a].is_err_disabled(PortRef::Eth(0)));
    n.send(ha, data(BCAST, host_mac(1)));
    n.run_for(1_000);
    assert_eq!(n.hosts[hb].rx.len(), 0);
    assert!(n.cli(a, "show interface brief").contains("err-disabled"));
    // `no shutdown` clears it, but the next BPDU trips it again.
    n.cli(a, "interface eth0\nno shutdown\nexit");
    assert!(!n.bridges[a].is_err_disabled(PortRef::Eth(0)));
    n.run_for(3_000);
    assert!(n.bridges[a].is_err_disabled(PortRef::Eth(0)));
    // A link flap also clears it.
    n.set_link(l, false);
    n.set_link(l, true);
    assert!(!n.bridges[a].is_err_disabled(PortRef::Eth(0)));
    n.run_for(3_000);
    assert!(n.bridges[a].is_err_disabled(PortRef::Eth(0)));
    // Once the peer stops sending BPDUs, `no shutdown` sticks and traffic flows.
    n.cli(b, "interface eth0\nno spanning-tree\nexit");
    n.run_for(100);
    n.cli(a, "interface eth0\nno shutdown\nexit");
    n.run_for(5_000);
    assert!(!n.bridges[a].is_err_disabled(PortRef::Eth(0)));
    n.clear_host_rx();
    n.send(ha, data(BCAST, host_mac(1)));
    n.run_for(1_000);
    assert_eq!(n.hosts[hb].count_from(host_mac(1)), 1);
}

#[test]
fn bpdu_guard_trips_on_injected_bpdu() {
    let mut n = Net::new(18);
    let a = n.add_bridge(2);
    n.l2_ports(a, &[0, 1]);
    // Guard works even with spanning tree disabled globally.
    n.cli(a, "interface eth0\nspanning-tree bpdu-guard\nexit");
    let h = n.add_host(1);
    n.link(End::B(a, 0), End::H(h));
    n.run_for(100);
    let body = BpduBody { flags: 0, root_id: 1, root_path_cost: 0, bridge_id: 1, port_id: 0x8001, message_age: 0, max_age: 20 * 256, hello_time: 512, forward_delay: 15 * 256 };
    n.send(h, bpdu::build_frame(host_mac(1), &Bpdu::Config(body)));
    n.run_for(100);
    assert!(n.bridges[a].is_err_disabled(PortRef::Eth(0)));
    assert!(!n.bridges[a].bridge_port_oper_up(PortRef::Eth(0)));
}

#[test]
fn root_guard_blocks_superior_bpdus_while_they_arrive() {
    let mut n = Net::new(19);
    let a = stp_bridge(&mut n, 2, None);
    let b = stp_bridge(&mut n, 2, Some(4096)); // superior
    let ha = n.add_host(1);
    let hb = n.add_host(2);
    n.cli(a, "interface eth0\nspanning-tree root-guard\nexit");
    n.link(End::B(a, 0), End::B(b, 0));
    n.link(End::B(a, 1), End::H(ha));
    n.link(End::B(b, 1), End::H(hb));
    n.run_for(2 * FD_MS + 5_000);
    let s = n.bridges[a].stp_port_status(PortRef::Eth(0)).unwrap();
    assert!(s.root_inconsistent);
    assert_eq!((s.role, s.state), (StpRole::Alternate, StpPortState::Discarding));
    assert!(n.bridges[a].stp_status().is_root, "root guard let a superior root in");
    n.send(ha, data(BCAST, host_mac(1)));
    n.run_for(1_000);
    assert_eq!(n.hosts[hb].rx.len(), 0);
    assert!(n.cli(a, "show spanning-tree").contains("RINC"));
    // Still blocked much later (root guard is not undone by timers).
    n.run_for(60_000);
    assert!(n.bridges[a].stp_port_status(PortRef::Eth(0)).unwrap().root_inconsistent);
    // Superior BPDUs stop: the port recovers through the normal timers.
    n.cli(b, "spanning-tree priority 61440");
    let t = probe_until_delivered(&mut n, ha, host_mac(1), hb, 2 * FD_MS + 10_000);
    assert!(t.is_some());
    assert!(!n.bridges[a].stp_port_status(PortRef::Eth(0)).unwrap().root_inconsistent);
    assert!(n.bridges[a].stp_status().is_root);
}

#[test]
fn per_port_stp_disable_forwards_immediately() {
    let mut n = Net::new(20);
    let a = stp_bridge(&mut n, 2, None);
    n.cli(a, "interface eth0\nno spanning-tree\nexit\ninterface eth1\nno spanning-tree\nexit");
    let h0 = n.add_host(1);
    let h1 = n.add_host(2);
    n.link(End::B(a, 0), End::H(h0));
    n.link(End::B(a, 1), End::H(h1));
    n.run_for(100);
    n.send(h0, data(BCAST, host_mac(1)));
    n.run_for(100);
    assert_eq!(n.hosts[h1].rx.len(), 1);
    // And sends no BPDUs.
    assert!(n.hosts[h1].ctrl_rx.iter().all(|f| bpdu::parse_frame(f).is_none()));
}

#[test]
fn admin_edge_port_forwards_immediately_and_still_sends_bpdus() {
    let mut n = Net::new(21);
    let a = stp_bridge(&mut n, 2, None);
    n.cli(a, "interface eth0\nspanning-tree admin-edge-port\nexit\ninterface eth1\nspanning-tree admin-edge-port\nexit");
    let h0 = n.add_host(1);
    let h1 = n.add_host(2);
    n.link(End::B(a, 0), End::H(h0));
    n.link(End::B(a, 1), End::H(h1));
    n.run_for(100);
    n.send(h0, data(BCAST, host_mac(1)));
    n.run_for(100);
    assert_eq!(n.hosts[h1].rx.len(), 1);
    assert!(n.hosts[h1].ctrl_rx.iter().any(|f| matches!(bpdu::parse_frame(f), Some(Bpdu::Rst(_)))));
    // No topology change from edge ports going forwarding.
    assert_eq!(n.bridges[a].stp_status().topology_changes, 0);
}

#[test]
fn legacy_stp_neighbour_gets_v0_bpdus_and_tcn() {
    let mut n = Net::new(22);
    let a = stp_bridge(&mut n, 2, None);
    let h = n.add_host(1);
    let h2 = n.add_host(2);
    n.link(End::B(a, 0), End::H(h));
    n.link(End::B(a, 1), End::H(h2));
    let root: u64 = 0x0000_0200_0000_0001; // priority 0: superior
    for _ in 0..45 {
        let body = BpduBody { flags: 0, root_id: root, root_path_cost: 0, bridge_id: root, port_id: 0x8001, message_age: 0, max_age: 20 * 256, hello_time: 512, forward_delay: 15 * 256 };
        n.send(h, bpdu::build_frame(host_mac(1), &Bpdu::Config(body)));
        n.run_for(1_000);
    }
    let s = n.bridges[a].stp_status();
    assert_eq!(s.root_id, root);
    assert_eq!(s.root_port, Some(PortRef::Eth(0)));
    assert_eq!(st(&n, a, 0), (StpRole::Root, StpPortState::Forwarding));
    // eth1 going forwarding was a topology change: announced with a TCN on
    // the root port (STP compatibility), RST BPDUs elsewhere.
    assert!(n.hosts[h].ctrl_rx.iter().any(|f| bpdu::parse_frame(f) == Some(Bpdu::Tcn)));
    assert!(n.hosts[h2].ctrl_rx.iter().any(|f| matches!(bpdu::parse_frame(f), Some(Bpdu::Rst(b)) if b.root_id == root && b.message_age == 256)));
}

#[test]
fn stale_info_with_excessive_message_age_is_discarded() {
    let mut n = Net::new(23);
    let a = stp_bridge(&mut n, 1, None);
    let h = n.add_host(1);
    n.link(End::B(a, 0), End::H(h));
    for _ in 0..5 {
        let body = BpduBody { flags: 0x0c, root_id: 1, root_path_cost: 0, bridge_id: 1, port_id: 0x8001, message_age: 20 * 256, max_age: 20 * 256, hello_time: 512, forward_delay: 15 * 256 };
        n.send(h, bpdu::build_frame(host_mac(1), &Bpdu::Rst(body)));
        n.run_for(1_000);
    }
    assert!(n.bridges[a].stp_status().is_root, "expired information was used");
}
