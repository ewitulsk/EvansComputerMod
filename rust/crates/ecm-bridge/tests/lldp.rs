//! LLDP neighbour discovery, TTL 0, link-down purge, ageing, bounds.

mod common;
use common::*;
use ecm_bridge::lldpdu::{self, Lldpdu};
use ecm_bridge::MAX_NEIGHBORS_PER_PORT;

fn two() -> (Net, usize) {
    let mut n = Net::new(41);
    let a = n.add_bridge(2);
    let b = n.add_bridge(2);
    n.l2_ports(a, &[0, 1]);
    n.l2_ports(b, &[0, 1]);
    let l = n.link(End::B(a, 0), End::B(b, 0));
    (n, l)
}

fn pdu(chassis: u8, ttl: u16) -> Lldpdu {
    Lldpdu {
        chassis_id: vec![4, 2, 0xee, 0, 0, 0, chassis],
        port_id: vec![5, b'p', b'1'],
        ttl,
        port_desc: None,
        sys_name: Some(format!("peer{}", chassis)),
        sys_desc: None,
        caps: None,
        mgmt_ipv4: None,
        mgmt_ifindex: 0,
    }
}

#[test]
fn neighbours_discover_each_other() {
    let (mut n, _) = two();
    n.cli(1, "interface vlan 1\nip address 10.0.0.2/24\nexit");
    // The SVI address change is advertised after txdelay (2 s).
    n.run_for(2_500);
    let na = n.bridges[0].lldp_neighbors(Some(0));
    let nb = n.bridges[1].lldp_neighbors(Some(0));
    assert_eq!(na.len(), 1);
    assert_eq!(nb.len(), 1);
    assert_eq!(na[0].chassis_string(), "02:b0:00:00:00:02");
    assert_eq!(na[0].port_string(), "eth0");
    assert_eq!(na[0].sys_name.as_deref(), Some("ecm-switch"));
    assert_eq!(na[0].ttl, 121); // 30 s x 4 + 1
    assert_eq!(na[0].mgmt_ipv4, Some([10, 0, 0, 2])); // falls back to the SVI address
    let out = n.cli(0, "show lldp neighbor-info\nshow lldp neighbor-info eth0\nshow lldp statistics");
    assert!(out.contains("eth0    02:b0:00:00:00:02   eth0"), "{}", out);
    assert!(out.contains("Mgmt IPv4        : 10.0.0.2"));
    // Periodic: roughly one PDU per 30 s timer.
    let before = n.bridges[0].lldp_port_stats(0).unwrap().tx;
    n.run_for(90_000);
    let sent = n.bridges[0].lldp_port_stats(0).unwrap().tx - before;
    assert!((3..=4).contains(&sent), "sent {} PDUs in 90 s", sent);
    // Refreshing keeps the neighbour alive well past one TTL.
    n.run_for(150_000);
    assert_eq!(n.bridges[0].lldp_neighbors(Some(0)).len(), 1);
}

#[test]
fn ttl_zero_deletes_neighbour() {
    let (mut n, _) = two();
    n.run_for(500);
    assert_eq!(n.bridges[0].lldp_neighbors(Some(0)).len(), 1);
    // Disabling transmit sends a shutdown LLDPDU (TTL 0).
    n.cli(1, "interface eth0\nno lldp transmit\nexit");
    n.run_for(50);
    assert!(n.bridges[0].lldp_neighbors(Some(0)).is_empty());
    assert!(n.logs[0].iter().any(|m| m.contains("TTL 0")));
    // Re-enabling honours reinit (2 s) before transmitting again.
    n.cli(1, "interface eth0\nlldp transmit\nexit");
    n.run_for(1_000);
    assert!(n.bridges[0].lldp_neighbors(Some(0)).is_empty());
    n.run_for(1_500);
    assert_eq!(n.bridges[0].lldp_neighbors(Some(0)).len(), 1);
}

#[test]
fn link_down_purges_and_silence_ages_out() {
    let (mut n, l) = two();
    n.run_for(500);
    n.set_link(l, false);
    assert!(n.bridges[0].lldp_neighbors(None).is_empty());
    assert!(n.bridges[1].lldp_neighbors(None).is_empty());
    n.set_link(l, true);
    n.run_for(500);
    assert_eq!(n.bridges[0].lldp_neighbors(Some(0)).len(), 1);
    // B goes silent (frames lost): A ages the entry out at the TTL.
    n.set_drop(l, 0, 1000);
    let last = n.bridges[0].lldp_neighbors(Some(0))[0].last_update_ms;
    let t = n.run_until(200_000, |n| n.bridges[0].lldp_neighbors(Some(0)).is_empty()).expect("never aged");
    let aged_at = n.now - last;
    assert!((121_000..=121_100).contains(&aged_at), "aged {} ms after last update ({} after drop)", aged_at, t);
    assert_eq!(n.bridges[0].lldp_port_stats(0).unwrap().ageouts, 1);
}

#[test]
fn only_nearest_bridge_lldp_is_consumed() {
    let mut n = Net::new(42);
    let a = n.add_bridge(2);
    n.l2_ports(a, &[0, 1]);
    let h0 = n.add_host(1);
    let h1 = n.add_host(2);
    n.link(End::B(a, 0), End::H(h0));
    n.link(End::B(a, 1), End::H(h1));
    let payload = lldpdu::encode(&pdu(1, 120));
    // Non-TPMR bridge scope: reserved, dropped, not learned.
    n.send(h0, eth([0x01, 0x80, 0xc2, 0, 0, 0x03], host_mac(1), 0x88cc, &payload));
    // Some other multicast: ordinary data, forwarded, not learned.
    n.send(h0, eth([0x01, 0x00, 0x5e, 0, 0, 1], host_mac(1), 0x88cc, &payload));
    n.run_for(100);
    assert!(n.bridges[a].lldp_neighbors(None).is_empty());
    assert_eq!(n.hosts[h1].rx.len(), 1);
    n.send(h0, lldpdu::build_frame(host_mac(1), &pdu(1, 120)));
    n.run_for(100);
    assert_eq!(n.bridges[a].lldp_neighbors(Some(0)).len(), 1);
    assert_eq!(n.hosts[h1].rx.len(), 1, "LLDP was forwarded");
    // Receive disabled: discarded and counted.
    n.cli(a, "interface eth0\nno lldp receive\nexit");
    n.send(h0, lldpdu::build_frame(host_mac(1), &pdu(2, 120)));
    n.run_for(100);
    assert_eq!(n.bridges[a].lldp_neighbors(Some(0)).len(), 1);
    assert_eq!(n.bridges[a].lldp_port_stats(0).unwrap().rx_discards, 1);
}

#[test]
fn neighbour_table_is_bounded_and_garbage_counted() {
    let mut n = Net::new(43);
    let a = n.add_bridge(1);
    n.l2_ports(a, &[0]);
    let h = n.add_host(1);
    n.link(End::B(a, 0), End::H(h));
    for c in 0..100u8 {
        n.send(h, lldpdu::build_frame(host_mac(1), &pdu(c, 120)));
        if c % 16 == 0 {
            n.step();
        }
    }
    n.run_for(50);
    assert_eq!(n.bridges[a].lldp_neighbors(Some(0)).len(), MAX_NEIGHBORS_PER_PORT);
    let s = n.bridges[a].lldp_port_stats(0).unwrap();
    assert_eq!(s.too_many, 100 - MAX_NEIGHBORS_PER_PORT as u64);
    // Malformed LLDPDU: counted as an error.
    n.send(h, eth(ecm_bridge::frame::LLDP_NEAREST_BRIDGE, host_mac(1), 0x88cc, &[0x04, 0x02, 4, 1]));
    n.run_for(50);
    assert_eq!(n.bridges[a].lldp_port_stats(0).unwrap().rx_errors, 1);
    // TTL 0 for one of them removes exactly that one.
    n.send(h, lldpdu::build_frame(host_mac(1), &pdu(5, 0)));
    n.run_for(50);
    assert_eq!(n.bridges[a].lldp_neighbors(Some(0)).len(), MAX_NEIGHBORS_PER_PORT - 1);
}

#[test]
fn local_change_triggers_transmission_after_txdelay() {
    let (mut n, _) = two();
    n.run_for(5_000);
    let before = n.bridges[0].lldp_port_stats(0).unwrap().tx;
    n.cli(0, "lldp management-ipv4-address 10.9.9.9");
    n.run_for(100);
    assert_eq!(n.bridges[0].lldp_port_stats(0).unwrap().tx, before + 1);
    assert_eq!(n.bridges[1].lldp_neighbors(Some(0))[0].mgmt_ipv4, Some([10, 9, 9, 9]));
    // A second change within txdelay (2 s) waits.
    n.cli(0, "no lldp select-tlv sys-name");
    n.run_for(500);
    assert_eq!(n.bridges[0].lldp_port_stats(0).unwrap().tx, before + 1);
    n.run_for(2_000);
    assert_eq!(n.bridges[0].lldp_port_stats(0).unwrap().tx, before + 2);
    assert_eq!(n.bridges[1].lldp_neighbors(Some(0))[0].sys_name, None);
    // Globally disabled: nothing is sent or learned.
    n.cli(0, "no lldp");
    n.cli(1, "no lldp");
    n.run_for(200_000);
    assert!(n.bridges[1].lldp_neighbors(None).is_empty());
}
