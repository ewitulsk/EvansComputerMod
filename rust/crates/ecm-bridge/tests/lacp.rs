//! Link aggregation: LACP bring-up, hashing, failover, partner checks,
//! timeouts, static and fallback modes.

mod common;
use common::*;
use ecm_bridge::lacpdu::{self, LacpInfo, Lacpdu, ST_ACTIVITY, ST_AGGREGATION, ST_SYNC};
use ecm_bridge::{LacpMux, MacAddr, PortRef};

/// Bridge with eth0+eth1 in lag1 (L2 access vlan 1), eth2.. access ports.
fn lag_bridge(n: &mut Net, ports: usize, lacp: &str) -> usize {
    let b = n.add_bridge(ports);
    n.cli(b, &format!("interface lag 1\nno routing\n{}\nexit\ninterface eth0\nlag 1\nexit\ninterface eth1\nlag 1\nexit", lacp));
    n.l2_ports(b, &(2..ports).collect::<Vec<_>>());
    b
}

struct Pair {
    n: Net,
    a: usize,
    b: usize,
    m0: usize,
    m1: usize,
    ha: usize,
    hb: usize,
}

fn pair(lacp_a: &str, lacp_b: &str) -> Pair {
    let mut n = Net::new(31);
    let a = lag_bridge(&mut n, 3, lacp_a);
    let b = lag_bridge(&mut n, 3, lacp_b);
    let m0 = n.link(End::B(a, 0), End::B(b, 0));
    let m1 = n.link(End::B(a, 1), End::B(b, 1));
    let ha = n.add_host(1);
    let hb = n.add_host(2);
    n.link(End::B(a, 2), End::H(ha));
    n.link(End::B(b, 2), End::H(hb));
    n.capture(m0);
    n.capture(m1);
    Pair { n, a, b, m0, m1, ha, hb }
}

fn formed(n: &Net, a: usize, b: usize) -> bool {
    n.bridges[a].lag_distributing_members(1) == vec![0, 1] && n.bridges[b].lag_distributing_members(1) == vec![0, 1]
}

#[test]
fn lacp_lag_forms_between_two_bridges() {
    let mut p = pair("lacp mode active\nlacp rate fast", "lacp mode passive\nlacp rate fast");
    // Not distributing before the handshake completes.
    assert!(p.n.bridges[p.a].lag_distributing_members(1).is_empty());
    let t = p.n.run_until(10_000, |n| formed(n, 0, 1));
    assert!(t.map(|t| t <= 5_000).unwrap_or(false), "LAG took {:?} ms to form", t);
    for br in [p.a, p.b] {
        for m in 0..2 {
            let s = p.n.bridges[br].lacp_member_status(m).unwrap();
            assert_eq!(s.mux, LacpMux::CollectingDistributing);
            assert_eq!(s.partner.system, bridge_mac(1 - br));
            assert_eq!(s.partner.key, 1);
            assert!(s.selected && s.distributing);
        }
        assert!(p.n.bridges[br].lag_oper_up(1));
    }
    // LACPDUs on the wire are 124-octet frames (128 with FCS).
    let caps = p.n.links[p.m0].capture.as_ref().unwrap();
    let pdus: Vec<_> = caps.iter().filter(|(_, f)| lacpdu::parse_frame(f).is_some()).collect();
    assert!(!pdus.is_empty());
    assert!(pdus.iter().all(|(_, f)| f.len() == 124));

    // No loop: one broadcast arrives once, crossing the LAG once.
    p.n.clear_host_rx();
    p.n.send(p.ha, data(BCAST, host_mac(1)));
    p.n.run_for(2_000);
    assert_eq!(p.n.hosts[p.hb].count_from(host_mac(1)), 1);
    assert_eq!(p.n.hosts[p.ha].rx.len(), 0);
    let crossed: usize = [p.m0, p.m1].iter().map(|&l| p.n.links[l].capture.as_ref().unwrap().iter().filter(|(_, f)| src_of(f) == host_mac(1)).count()).sum();
    assert_eq!(crossed, 1);
    assert_eq!(p.n.bridges[p.b].fdb_lookup(MacAddr(host_mac(1)), 1), Some(PortRef::Lag(1)));
    let out = p.n.cli(p.a, "show lacp interfaces\nshow interface lag 1");
    assert!(out.contains("coll-dist"));
    assert!(out.contains("  Up members : eth0,eth1"), "{}", out);
}

#[test]
fn traffic_balances_by_hash_across_members() {
    let mut p = pair("lacp mode active\nlacp rate fast\nhash l3-src-dst", "lacp mode active\nlacp rate fast");
    p.n.run_until(10_000, |n| formed(n, 0, 1)).expect("formed");
    // Teach both sides where the hosts are.
    p.n.send(p.hb, data(BCAST, host_mac(2)));
    p.n.send(p.ha, data(BCAST, host_mac(1)));
    p.n.run_for(100);
    let count = |n: &Net, l: usize| n.links[l].capture.as_ref().unwrap().iter().filter(|(ab, f)| *ab && src_of(f) == host_mac(1)).count();
    let (c0, c1) = (count(&p.n, p.m0), count(&p.n, p.m1));
    for i in 0..400u32 {
        let f = ipv4(host_mac(2), host_mac(1), [10, 0, (i >> 8) as u8, i as u8], [10, 1, 0, (i % 7) as u8], 1000, 2000);
        p.n.send(p.ha, f);
        if i % 20 == 0 {
            p.n.step();
        }
    }
    p.n.run_for(100);
    let (d0, d1) = (count(&p.n, p.m0) - c0, count(&p.n, p.m1) - c1);
    assert_eq!(d0 + d1, 400);
    assert_eq!(p.n.hosts[p.hb].count_from(host_mac(1)), 401);
    assert!(d0 >= 120 && d1 >= 120, "unbalanced: {} / {}", d0, d1);
    // A single flow always takes one member (no reordering).
    let (e0, e1) = (count(&p.n, p.m0), count(&p.n, p.m1));
    for _ in 0..50 {
        p.n.send(p.ha, ipv4(host_mac(2), host_mac(1), [10, 0, 0, 1], [10, 1, 0, 1], 5, 6));
    }
    p.n.run_for(50);
    let (f0, f1) = (count(&p.n, p.m0) - e0, count(&p.n, p.m1) - e1);
    assert!(f0 == 50 && f1 == 0 || f0 == 0 && f1 == 50);
}

#[test]
fn member_failure_fails_over_without_loss() {
    let mut p = pair("lacp mode active\nlacp rate fast\nhash l4-src-dst", "lacp mode active\nlacp rate fast");
    p.n.run_until(10_000, |n| formed(n, 0, 1)).expect("formed");
    p.n.send(p.hb, data(BCAST, host_mac(2)));
    p.n.run_for(100);
    p.n.set_link(p.m0, false);
    assert_eq!(p.n.bridges[p.a].lag_distributing_members(1), vec![1]);
    assert!(p.n.bridges[p.a].lag_oper_up(1));
    // FDB still points at the LAG; every frame is delivered.
    assert_eq!(p.n.bridges[p.a].fdb_lookup(MacAddr(host_mac(2)), 1), Some(PortRef::Lag(1)));
    p.n.clear_host_rx();
    for i in 0..100u16 {
        p.n.send(p.ha, ipv4(host_mac(2), host_mac(1), [10, 0, 0, 1], [10, 1, 0, 1], i, 80));
        p.n.step();
    }
    p.n.run_for(100);
    assert_eq!(p.n.hosts[p.hb].count_from(host_mac(1)), 100);
    // The member comes back and rejoins.
    p.n.set_link(p.m0, true);
    let t = p.n.run_until(10_000, |n| formed(n, 0, 1));
    assert!(t.is_some());
}

#[test]
fn members_to_different_partners_do_not_bundle() {
    let mut n = Net::new(32);
    let a = lag_bridge(&mut n, 3, "lacp mode active\nlacp rate fast");
    // B and C each run a one-member LAG towards A, and are also linked.
    let mut peers = Vec::new();
    for _ in 0..2 {
        let x = n.add_bridge(3);
        n.cli(x, "interface lag 1\nno routing\nlacp mode active\nlacp rate fast\nexit\ninterface eth0\nlag 1\nexit");
        n.l2_ports(x, &[1, 2]);
        peers.push(x);
    }
    let (b, c) = (peers[0], peers[1]);
    n.link(End::B(a, 0), End::B(b, 0));
    n.link(End::B(a, 1), End::B(c, 0));
    n.link(End::B(b, 1), End::B(c, 1)); // would be a loop if both members bundled
    let ha = n.add_host(1);
    let hb = n.add_host(2);
    let hc = n.add_host(3);
    n.link(End::B(a, 2), End::H(ha));
    n.link(End::B(b, 2), End::H(hb));
    n.link(End::B(c, 2), End::H(hc));
    n.run_for(15_000);
    let dist = n.bridges[a].lag_distributing_members(1);
    assert_eq!(dist.len(), 1, "members to two partners bundled: {:?}", dist);
    let other = 1 - dist[0];
    let s = n.bridges[a].lacp_member_status(other).unwrap();
    assert!(!s.selected && !s.distributing);
    // Exactly one of B/C has its LAG up.
    let up: Vec<bool> = [b, c].iter().map(|&x| n.bridges[x].lag_oper_up(1)).collect();
    assert_eq!(up.iter().filter(|u| **u).count(), 1);
    // No storm: a broadcast reaches each host once.
    n.clear_host_rx();
    let total = n.link_total();
    n.send(ha, data(BCAST, host_mac(1)));
    n.run_for(2_000);
    assert_eq!(n.hosts[hb].count_from(host_mac(1)), 1);
    assert_eq!(n.hosts[hc].count_from(host_mac(1)), 1);
    assert!(n.link_total() - total < 60);
}

#[test]
fn partner_timeout_is_three_seconds_with_fast_rate() {
    let mut p = pair("lacp mode active\nlacp rate fast", "lacp mode active\nlacp rate fast");
    p.n.run_until(10_000, |n| formed(n, 0, 1)).expect("formed");
    p.n.run_for(5_000);
    // B's LACPDUs stop reaching A on member 0; carrier stays up.
    p.n.set_drop(p.m0, 0, 1000);
    let t = p.n.run_until(10_000, |n| !n.bridges[0].lag_distributing_members(1).contains(&0)).expect("timed out");
    assert!((2_000..=3_100).contains(&t), "partner timeout after {} ms", t);
    assert_eq!(p.n.bridges[p.a].lag_distributing_members(1), vec![1]);
    assert!(p.n.logs[p.a].iter().any(|m| m.contains("partner timed out")));
}

#[test]
fn slow_rate_times_out_after_ninety_seconds() {
    let mut p = pair("lacp mode active", "lacp mode active");
    p.n.run_until(10_000, |n| formed(n, 0, 1)).expect("formed");
    p.n.set_drop(p.m0, 0, 1000);
    p.n.run_for(60_000);
    assert!(p.n.bridges[p.a].lag_distributing_members(1).contains(&0), "slow rate expired early");
    let t = p.n.run_until(40_000, |n| !n.bridges[0].lag_distributing_members(1).contains(&0));
    assert!(t.is_some(), "slow rate never expired");
}

#[test]
fn passive_passive_never_forms() {
    let mut p = pair("lacp mode passive", "lacp mode passive");
    p.n.run_for(10_000);
    assert!(p.n.bridges[p.a].lag_distributing_members(1).is_empty());
    assert!(!p.n.bridges[p.a].lag_oper_up(1));
    let pdus = p.n.links[p.m0].capture.as_ref().unwrap().iter().filter(|(_, f)| lacpdu::parse_frame(f).is_some()).count();
    assert_eq!(pdus, 0);
}

#[test]
fn static_lag_distributes_without_lacpdus() {
    let mut p = pair("", "");
    p.n.run_for(100);
    assert_eq!(p.n.bridges[p.a].lag_distributing_members(1), vec![0, 1]);
    p.n.send(p.ha, data(BCAST, host_mac(1)));
    p.n.run_for(500);
    assert_eq!(p.n.hosts[p.hb].count_from(host_mac(1)), 1);
    assert_eq!(p.n.hosts[p.ha].rx.len(), 0);
    let pdus = p.n.links[p.m0].capture.as_ref().unwrap().iter().filter(|(_, f)| f.get(12..14) == Some(&[0x88, 0x09][..])).count();
    assert_eq!(pdus, 0);
}

#[test]
fn fallback_brings_up_one_member_without_partner() {
    let mut n = Net::new(33);
    let a = lag_bridge(&mut n, 3, "lacp mode active\nlacp rate fast\nlacp fallback");
    let h0 = n.add_host(1);
    let h1 = n.add_host(2);
    let hx = n.add_host(3);
    n.link(End::B(a, 0), End::H(h0));
    n.link(End::B(a, 1), End::H(h1));
    n.link(End::B(a, 2), End::H(hx));
    n.run_for(1_000);
    assert!(n.bridges[a].lag_distributing_members(1).is_empty());
    let t = n.run_until(10_000, |n| n.bridges[0].lag_distributing_members(1) == vec![0]);
    assert!(t.is_some(), "fallback did not engage");
    n.send(hx, data(BCAST, host_mac(3)));
    n.run_for(100);
    assert_eq!(n.hosts[h0].count_from(host_mac(3)) + n.hosts[h1].count_from(host_mac(3)), 1);
    // Without fallback nothing comes up.
    n.cli(a, "interface lag 1\nno lacp fallback\nexit");
    n.run_for(100);
    assert!(n.bridges[a].lag_distributing_members(1).is_empty());
}

#[test]
fn loopback_to_own_system_never_aggregates() {
    let mut n = Net::new(34);
    let a = lag_bridge(&mut n, 2, "lacp mode active\nlacp rate fast");
    n.link(End::B(a, 0), End::B(a, 1));
    n.run_for(10_000);
    assert!(n.bridges[a].lag_distributing_members(1).is_empty());
}

#[test]
fn partner_must_acknowledge_before_distributing() {
    // A host that answers LACPDUs but never claims Sync: we attach (Sync)
    // but must not start collecting/distributing.
    let mut n = Net::new(35);
    let a = n.add_bridge(1);
    n.cli(a, "interface lag 1\nno routing\nlacp mode active\nlacp rate fast\nexit\ninterface eth0\nlag 1\nexit");
    let h = n.add_host(9);
    n.link(End::B(a, 0), End::H(h));
    for _ in 0..8 {
        let ours = n.hosts[h].ctrl_rx.iter().rev().find_map(|f| lacpdu::parse_frame(f)).map(|p| p.actor).unwrap_or_default();
        let pdu = Lacpdu {
            actor: LacpInfo { system_priority: 1, system: host_mac(9), key: 77, port_priority: 1, port: 1, state: ST_ACTIVITY | ST_AGGREGATION },
            partner: ours,
            collector_max_delay: 0,
        };
        n.send(h, lacpdu::build_frame(host_mac(9), &pdu));
        n.run_for(1_000);
    }
    let s = n.bridges[a].lacp_member_status(0).unwrap();
    assert!(s.selected);
    assert_eq!(s.mux, LacpMux::Attached);
    assert!(s.actor.state & ST_SYNC != 0);
    assert!(!s.distributing);
    assert_eq!(s.partner.key, 77); // keys need not match ours
}
