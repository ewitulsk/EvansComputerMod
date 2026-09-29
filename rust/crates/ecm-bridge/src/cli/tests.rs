use super::*;
use crate::types::PortRef;
use ecm_net::types::MacAddr;

fn bridge(n: usize) -> Bridge {
    let macs: Vec<MacAddr> = (0..n).map(|i| MacAddr([2, 0, 0, 0, 1, i as u8])).collect();
    Bridge::new(&macs, MacAddr([2, 0, 0, 0, 0, 0xaa]), 0)
}

fn run(b: &mut Bridge, s: &mut CliSession, cmds: &str) -> CliResult {
    let mut all = CliResult::default();
    for l in cmds.lines() {
        let r = exec(b, s, l, 1000);
        all.output.push_str(&r.output);
        all.effects.extend(r.effects);
    }
    all
}

#[test]
fn prompts_and_contexts() {
    let mut b = bridge(4);
    let mut s = CliSession::new();
    assert_eq!(prompt(&s), "switch(config)# ");
    exec(&mut b, &mut s, "vlan 10", 0);
    assert_eq!(prompt(&s), "switch(config-vlan-10)# ");
    let r = exec(&mut b, &mut s, "exit", 0);
    assert!(r.effects.is_empty());
    assert_eq!(prompt(&s), "switch(config)# ");
    exec(&mut b, &mut s, "interface eth2", 0);
    assert_eq!(prompt(&s), "switch(config-if-eth2)# ");
    exec(&mut b, &mut s, "end", 0);
    assert_eq!(s.context(), Context::Top);
    exec(&mut b, &mut s, "interface 3", 0);
    assert_eq!(prompt(&s), "switch(config-if-eth3)# ");
    exec(&mut b, &mut s, "quit", 0);
    exec(&mut b, &mut s, "interface lag 5", 0);
    assert_eq!(prompt(&s), "switch(config-lag-5)# ");
    exec(&mut b, &mut s, "exit", 0);
    exec(&mut b, &mut s, "interface vlan 10", 0);
    assert_eq!(prompt(&s), "switch(config-if-vlan-10)# ");
    exec(&mut b, &mut s, "exit", 0);
    let r = exec(&mut b, &mut s, "exit", 0);
    assert_eq!(r.effects, vec![CliEffect::ExitCli]);
    // Invalid interface leaves the context alone.
    let r = exec(&mut b, &mut s, "interface eth9", 0);
    assert!(r.output.starts_with("% Invalid port"));
    assert_eq!(s.context(), Context::Top);
}

#[test]
fn on_emits_detach() {
    let mut b = bridge(2);
    let mut s = CliSession::new();
    let r = exec(&mut b, &mut s, "on", 0);
    assert_eq!(r.effects, vec![CliEffect::Detach]);
    assert!(exec(&mut b, &mut s, "off", 0).output.starts_with('%'));
}

#[test]
fn comments_and_blank_lines_ignored() {
    let mut b = bridge(2);
    let mut s = CliSession::new();
    assert_eq!(exec(&mut b, &mut s, "   ", 0), CliResult::default());
    assert_eq!(exec(&mut b, &mut s, "# vlan 10", 0), CliResult::default());
    assert!(b.vlan(10).is_none());
}

#[test]
fn vlan_and_port_modes() {
    let mut b = bridge(6);
    let mut s = CliSession::new();
    let r = run(&mut b, &mut s, "vlan 10\nname engineering\nno shutdown\nexit\ninterface eth0\nvlan access 10");
    assert!(r.output.contains("% Run 'no routing' first"), "{}", r.output);
    let r = run(&mut b, &mut s, "no routing\nvlan access 10\nexit");
    assert!(r.output.contains("Port converted to L2 (access vlan 1)."));
    assert!(r.output.contains("Access VLAN set to 10."));
    assert_eq!(b.port_mode(PortRef::Eth(0)), Some(&PortMode::Access { vid: 10 }));
    assert!(b.is_l2_port(0));
    assert!(!b.is_l2_port(1));
    // Access to a missing VLAN creates it inactive with a warning.
    let r = run(&mut b, &mut s, "interface eth1\nno routing\nvlan access 30\nexit");
    assert!(r.output.contains("% Warning: VLAN 30 does not exist"));
    assert_eq!(b.vlan(30).map(|v| v.active), Some(false));
    // Trunk.
    let r = run(&mut b, &mut s, "interface eth5\nno routing\nvlan trunk allowed 10\nvlan trunk native 1\nvlan trunk allowed 10,20-22\nexit");
    assert!(r.output.contains("% Port is not in trunk mode"));
    match b.port_mode(PortRef::Eth(5)) {
        Some(PortMode::Trunk { native: 1, native_tag: false, allowed: AllowedList::Some(s) }) => {
            assert_eq!(s.iter().copied().collect::<Vec<_>>(), vec![10, 20, 21, 22])
        }
        m => panic!("{:?}", m),
    }
    let r = run(&mut b, &mut s, "interface eth5\nno vlan trunk allowed 21\nvlan trunk allowed 20-15\nvlan trunk native 100 tag\nexit");
    assert!(r.output.contains("Removed VLANs from trunk allowed list."));
    assert!(r.output.contains("% Invalid VLAN list: 20-15"));
    assert!(r.output.contains("Trunk native VLAN set to 100 (tagged)."));
    // In-use VLAN cannot be deleted; VLAN 1 never.
    let r = run(&mut b, &mut s, "no vlan 10\nno vlan 1\nno vlan 999");
    assert!(r.output.contains("% VLAN 10 is in use on port eth0"));
    assert!(r.output.contains("% Cannot modify VLAN 1"));
    assert!(r.output.contains("% VLAN 999 does not exist"));
    let r = run(&mut b, &mut s, "vlan 100-110");
    assert!(r.output.contains("Created 11 VLANs (range 100-110)."), "{}", r.output);
    let r = run(&mut b, &mut s, "vlan 1\nshutdown\nexit");
    assert!(r.output.contains("% Cannot modify VLAN 1"));
}

#[test]
fn svi_effects_and_state() {
    let mut b = bridge(2);
    let mut s = CliSession::new();
    let r = run(&mut b, &mut s, "interface vlan 20");
    assert!(r.output.contains("% VLAN 20 does not exist"));
    let r = run(&mut b, &mut s, "vlan 20\nno shutdown\nexit\ninterface vlan 20\nip address 10.0.20.1/24\nexit");
    assert_eq!(r.effects, vec![CliEffect::SviAddress { vlan: 20, ip: Ipv4Addr([10, 0, 20, 1]), prefix: 24 }]);
    assert!(b.svis() == vec![(20, Ipv4Addr([10, 0, 20, 1]), 24)]);
    assert!(b.has_svi(20));
    let r = run(&mut b, &mut s, "interface vlan 20\nip address 10.0.20.2 255.255.0.0\nip address 1.2.3.4/33\nexit");
    assert_eq!(r.effects, vec![CliEffect::SviAddress { vlan: 20, ip: Ipv4Addr([10, 0, 20, 2]), prefix: 16 }]);
    assert!(r.output.contains("% Usage: ip address"));
    let r = run(&mut b, &mut s, "no vlan 20");
    assert!(r.output.contains("has an SVI"));
    let r = run(&mut b, &mut s, "write memory");
    match r.effects.as_slice() {
        [CliEffect::SaveConfig(cfg)] => assert!(cfg.contains("interface vlan 20\n ip address 10.0.20.2/16\n")),
        e => panic!("{:?}", e),
    }
    let r = run(&mut b, &mut s, "interface vlan 20\nno ip address\nexit");
    assert_eq!(r.effects, vec![CliEffect::SviRemove { vlan: 20 }]);
    assert!(!b.has_svi(20));
    let r = run(&mut b, &mut s, "interface vlan 20\nip address 10.0.0.1/8\nexit\nno interface vlan 20");
    assert_eq!(r.effects.last(), Some(&CliEffect::SviRemove { vlan: 20 }));
    assert!(b.svis().is_empty());
}

#[test]
fn static_macs() {
    let mut b = bridge(4);
    let mut s = CliSession::new();
    let r = run(
        &mut b,
        &mut s,
        "static-mac 02:00:00:00:00:10 vlan 10 port 3\nstatic-mac 02-00-00-00-00-11 vlan 10 port eth2\nstatic-mac 02:00:00:00:00:12 vlan 1 port lag4\nstatic-mac zz vlan 1 port 1\nstatic-mac 02:00:00:00:00:13 vlan 1 port 9",
    );
    assert_eq!(r.output.matches("Static MAC entry added.").count(), 3, "{}", r.output);
    assert!(r.output.contains("% Invalid MAC address"));
    assert!(r.output.contains("% Port 9 out of range (0-3)"));
    assert_eq!(b.fdb_lookup(MacAddr([2, 0, 0, 0, 0, 0x10]), 10), Some(PortRef::Eth(3)));
    let r = run(&mut b, &mut s, "no static-mac 02:00:00:00:00:10 vlan 10 port 2\nno static-mac 02:00:00:00:00:10 vlan 10 port 3");
    assert!(r.output.contains("% No matching static entry"));
    assert!(r.output.contains("Static MAC entry removed."));
    let r = run(&mut b, &mut s, "show mac-address-table");
    assert!(r.output.contains("MAC age-time            : 300 seconds"));
    assert!(r.output.contains("Number of MAC addresses : 2"));
    assert!(r.output.contains("02:00:00:00:00:12   1       static   lag4"), "{}", r.output);
    let r = run(&mut b, &mut s, "show mac-address-table count\nshow mac-address-table static\nshow mac-address-table vlan 10\nshow mac-address-table port 2\nshow mac-address-table address 02:00:00:00:00:11\nshow mac-address-table mac-move\nshow mac-address-table dynamic port 1");
    assert!(r.output.contains("Number of entries: 2"));
    assert!(r.output.contains("(no MAC moves recorded)"));
    assert!(r.output.contains("(no matching entries)"));
}

#[test]
fn age_time_and_logging() {
    let mut b = bridge(2);
    let mut s = CliSession::new();
    let r = run(&mut b, &mut s, "mac-address-table age-time 14\nmac-address-table age-time 60\nmac-address-table age-time x");
    assert!(r.output.contains("% age-time out of range (15-1000000)"));
    assert!(r.output.contains("MAC age-time set to 60 seconds."));
    assert!(r.output.contains("% Invalid age-time value"));
    assert_eq!(b.age_time_secs(), 60);
    run(&mut b, &mut s, "no mac-address-table age-time");
    assert_eq!(b.age_time_secs(), 300);
    let r = run(&mut b, &mut s, "logging severity debug\nlogging console\nlogging 10.0.0.9\nlogging bogus\nvlan 42\nexit\nshow logging");
    assert!(r.output.contains("Logging severity: debug"));
    assert!(r.output.contains("Console: on"));
    assert!(r.output.contains("Collectors: 10.0.0.9"));
    assert!(r.output.contains("VLAN 42 created"));
    assert!(r.output.contains("% Unknown logging argument"));
    assert!(b.log_console());
    let r = run(&mut b, &mut s, "clear logging\nshow events -r");
    assert!(r.output.contains("Entries: 0"));
}

#[test]
fn lag_and_lacp_commands() {
    let mut b = bridge(4);
    let mut s = CliSession::new();
    let r = run(&mut b, &mut s, "interface eth0\nlag 1\nexit");
    assert!(r.output.contains("% LAG 1 does not exist"));
    let r = run(
        &mut b,
        &mut s,
        "interface lag 1\nno routing\nlacp mode active\nlacp rate fast\nhash l4-src-dst\nfallback\nlacp mode bogus\nexit\ninterface eth0\nlag 1\nexit\ninterface eth1\nlag 1\nexit",
    );
    assert!(r.output.contains("LACP mode set to active."));
    assert!(r.output.contains("% Usage: lacp mode"));
    let c = b.lag_config(1).unwrap();
    assert_eq!((c.lacp_mode, c.lacp_rate, c.hash, c.fallback), (LacpMode::Active, LacpRate::Fast, HashMode::L4, true));
    assert_eq!(b.lag_members(1), vec![0, 1]);
    assert!(b.is_l2_port(0) && b.is_l2_port(1));
    // Joining another LAG moves the port.
    run(&mut b, &mut s, "interface lag 2\nexit\ninterface eth1\nlag 2\nexit");
    assert_eq!(b.lag_members(1), vec![0]);
    assert_eq!(b.lag_members(2), vec![1]);
    assert!(!b.is_l2_port(1)); // lag 2 is routed
    let r = run(&mut b, &mut s, "show lacp aggregates\nshow lacp interfaces\nshow lacp configuration\nshow interface lag 1\nshow interface lag 9");
    assert!(r.output.contains("LAG 1:"));
    assert!(r.output.contains("  Hash       : l4-src-dst"));
    assert!(r.output.contains("% LAG not configured"));
    let r = run(&mut b, &mut s, "no interface lag 1\ninterface eth0\nno lag\nexit");
    assert!(r.output.contains("LAG 1 deleted."));
    assert!(r.output.contains("% eth0 is not a LAG member"));
    assert!(b.lag_config(1).is_none());
}

#[test]
fn stp_and_lldp_commands() {
    let mut b = bridge(3);
    let mut s = CliSession::new();
    let r = run(
        &mut b,
        &mut s,
        "spanning-tree\nspanning-tree priority 4096\nspanning-tree priority 100\nspanning-tree forward-delay 4\nspanning-tree hello-time 11\nspanning-tree max-age 6\nspanning-tree config-name my region",
    );
    assert!(r.output.contains("Spanning tree enabled."));
    assert!(r.output.contains("% Priority must be 0..61440 in steps of 4096"));
    assert!(r.output.contains("% hello-time out of range (1-10)"));
    let c = b.stp_config().clone();
    assert_eq!((c.enabled, c.priority, c.forward_delay_secs, c.max_age_secs, c.config_name.as_str()), (true, 4096, 4, 6, "my region"));
    let r = run(
        &mut b,
        &mut s,
        "interface eth1\nno routing\nspanning-tree port-priority 64\nspanning-tree port-priority 65\nspanning-tree cost 5\nspanning-tree bpdu-guard\nspanning-tree root-guard\nspanning-tree admin-edge-port\nno lldp transmit\nexit",
    );
    assert!(r.output.contains("% Port-priority must be 0..240 in steps of 16"));
    let p = b.port_stp_config(PortRef::Eth(1)).unwrap();
    assert_eq!((p.priority, p.cost, p.bpdu_guard, p.root_guard, p.admin_edge), (64, 5, true, true, true));
    assert!(!b.eth_config(1).unwrap().lldp_tx);
    let r = run(&mut b, &mut s, "lldp timer 10\nlldp timer 4\nlldp holdtime 3\nno lldp select-tlv sys-desc\nlldp management-ipv4-address 10.1.1.1\nshow lldp configuration\nshow lldp tlv\nshow lldp local-device\nshow lldp statistics\nshow lldp neighbor-info\nshow spanning-tree");
    assert!(r.output.contains("LLDP timer set to 10."));
    assert!(r.output.contains("% timer out of range (5-32768)"));
    assert!(r.output.contains("  Holdtime  : 3 (TTL = 31 s)"));
    assert!(r.output.contains("  sys-desc  : false"));
    assert!(r.output.contains("Mgmt IPv4        : 10.1.1.1"));
    assert!(r.output.contains("(no LLDP neighbors)"));
    assert!(r.output.contains("Spanning Tree: enabled"));
    assert!(r.output.contains("Bridge ID  : 1000.020000.0000aa"), "{}", r.output);
}

fn build_complex(b: &mut Bridge) -> Vec<CliEffect> {
    let mut s = CliSession::new();
    let r = run(
        b,
        &mut s,
        "mac-address-table age-time 120
logging severity notice
logging console
logging 10.9.9.9
logging 10.9.9.8
no lldp
lldp timer 12
lldp holdtime 5
lldp reinit 3
lldp txdelay 4
lldp management-ipv4-address 10.0.0.250
no lldp select-tlv port-desc
no lldp select-tlv mgmt-addr
spanning-tree
spanning-tree priority 8192
spanning-tree forward-delay 10
spanning-tree hello-time 1
spanning-tree max-age 12
spanning-tree config-name region one
spanning-tree config-revision 7
vlan 1
name mgmt
description management vlan
exit
vlan 10
name engineering
description Engineering network
no shutdown
exit
vlan 20
name guest
exit
vlan 30
no shutdown
exit
interface lag 3
no routing
vlan trunk native 10 tag
vlan trunk allowed 10,20,30
no vlan trunk allowed 20
lacp mode passive
lacp rate fast
lacp fallback
hash l2-src-dst
spanning-tree cost 1000
spanning-tree root-guard
shutdown
exit
interface lag 4
exit
interface eth0
no routing
vlan access 10
spanning-tree admin-edge-port
spanning-tree bpdu-guard
no lldp receive
exit
interface eth1
no routing
vlan trunk native 1
exit
interface eth2
lag 3
exit
interface eth3
lag 3
no spanning-tree
spanning-tree port-priority 32
spanning-tree tcn-guard
exit
interface eth4
no routing
vlan trunk native 30
vlan trunk allowed 1
no vlan trunk allowed 1
shutdown
no lldp transmit
exit
interface vlan 10
ip address 192.168.10.1/24
exit
interface vlan 30
ip address 10.30.0.1/16
exit
static-mac 02:00:00:00:00:10 vlan 10 port 0
static-mac 02:00:00:00:00:11 vlan 30 port lag3
static-mac 02:00:00:00:00:12 vlan 1 port 5",
    );
    assert!(!r.output.contains('%'), "unexpected errors:\n{}", r.output);
    r.effects
}

#[test]
fn running_config_round_trip() {
    let mut a = bridge(6);
    let effects = build_complex(&mut a);
    assert_eq!(effects.iter().filter(|e| matches!(e, CliEffect::SviAddress { .. })).count(), 2);
    let text = running_config(&a, &[]);
    assert!(!text.lines().any(|l| l == "exit"), "top-level exit in:\n{}", text);
    let mut b = bridge(6);
    let (applied, r) = load_config(&mut b, &text, 5000);
    assert!(applied > 40);
    assert!(!r.output.contains('%'), "replay errors:\n{}", r.output);
    assert_eq!(r.effects.iter().filter(|e| matches!(e, CliEffect::SviAddress { .. })).count(), 2);
    assert_eq!(a.config_snapshot(), b.config_snapshot());
    assert_eq!(text, running_config(&b, &[]));
    // The exec path (one session, line by line) gives the same result.
    let mut c = bridge(6);
    let mut s = CliSession::new();
    for l in text.lines() {
        let r = exec(&mut c, &mut s, l, 0);
        assert!(!r.effects.contains(&CliEffect::ExitCli));
    }
    assert_eq!(a.config_snapshot(), c.config_snapshot());
    assert_eq!(s.context(), Context::Top);
}

#[test]
fn default_config_is_minimal() {
    let b = bridge(3);
    assert_eq!(running_config(&b, &[]), "# switch.cfg - generated by 'write memory'\n");
    let text = running_config(&b, &[(5, Ipv4Addr([1, 2, 3, 4]), 8)]);
    assert!(text.contains("interface vlan 5\n ip address 1.2.3.4/8\n exit\n"));
}

#[test]
fn legacy_switch_cfg_loads() {
    // The format written by the previous kernel implementation.
    let legacy = "# switch.cfg - generated by 'write memory'
mac-address-table age-time 300
vlan 10
 name engineering
 description Engineering network
 no shutdown
exit
vlan 20
 name guest
 no shutdown
exit
interface eth0
 no routing
 vlan access 10
exit
interface eth5
 no routing
 vlan trunk native 1
 vlan trunk allowed 10,20
exit
static-mac 02:00:00:00:00:10 vlan 10 port 3
logging severity warning
spanning-tree
interface eth1
 spanning-tree bpdu-guard
exit
interface lag 1
 no routing
 lacp mode active
exit
interface eth2
 lag 1
exit
";
    let mut b = bridge(6);
    let (_, r) = load_config(&mut b, legacy, 0);
    assert!(!r.output.contains('%'), "{}", r.output);
    assert!(r.output.contains("Loaded switch.cfg"));
    assert_eq!(b.port_mode(PortRef::Eth(0)), Some(&PortMode::Access { vid: 10 }));
    assert_eq!(b.lag_members(1), vec![2]);
    assert!(b.stp_config().enabled);
    assert_eq!(b.log_severity(), Severity::Warning);
}

#[test]
fn show_vlan_outputs() {
    let mut b = bridge(6);
    let mut s = CliSession::new();
    run(&mut b, &mut s, "vlan 10\nname engineering\nno shutdown\nexit\ninterface eth3\nno routing\nvlan access 10\nexit\ninterface eth5\nno routing\nvlan trunk native 1\nexit");
    b.set_link(3, true, 0);
    let r = run(&mut b, &mut s, "show vlan\nshow vlan 10\nshow vlan summary\nshow vlan port 5\nshow vlan port 0\nshow vlan 77");
    assert!(r.output.contains("VLAN  Name             Status   Reason       Ports (untagged / tagged)"));
    assert!(r.output.contains("10    engineering      up       ok           eth3 / eth5"), "{}", r.output);
    assert!(r.output.contains("1     default          down     no-member-up  eth5 / -"), "{}", r.output);
    assert!(r.output.contains("Untagged ports: eth3"));
    assert!(r.output.contains("Number of VLANs: 2"));
    assert!(r.output.contains("Native VLAN: 1 (untagged)"));
    assert!(r.output.contains("Allowed VLANs: all"));
    assert!(r.output.contains("Mode: routed (L3)"));
    assert!(r.output.contains("% VLAN 77 does not exist"));
    let r = run(&mut b, &mut s, "show interface brief\nshow running-config\nhelp");
    assert!(r.output.contains("eth3    up    up     access"));
    assert!(r.output.contains("interface eth5"));
    assert!(r.output.contains("Switch configuration commands:"));
}

#[test]
fn clear_commands() {
    let mut b = bridge(3);
    let mut s = CliSession::new();
    let r = run(&mut b, &mut s, "clear mac-address-table dynamic\nclear mac-address-table dynamic vlan 3\nclear mac-address-table dynamic port 1\nclear mac-address-table dynamic address 02:00:00:00:00:01\nclear mac-address-table\nclear lldp neighbors\nclear lldp statistics\nclear lldp");
    assert!(r.output.contains("Cleared 0 dynamic entries."));
    assert!(r.output.contains("Cleared 0 dynamic entries in VLAN 3."));
    assert!(r.output.contains("Cleared 0 dynamic entries on port 1."));
    assert!(r.output.contains("LLDP statistics cleared."));
    assert!(r.output.contains("% Usage: clear lldp [neighbors|statistics]"));
}

#[test]
fn help_in_every_context() {
    let mut b = bridge(2);
    let mut s = CliSession::new();
    for ctx in ["", "vlan 5", "interface eth0", "interface lag 1", "interface vlan 5"] {
        exec(&mut b, &mut s, "end", 0);
        if !ctx.is_empty() {
            exec(&mut b, &mut s, ctx, 0);
        }
        let r = exec(&mut b, &mut s, "?", 0);
        assert!(r.output.contains("commands:"), "{}: {}", ctx, r.output);
    }
}

#[test]
fn cli_fuzz_no_panic() {
    let vocab = [
        "vlan", "no", "interface", "eth0", "eth99", "lag", "1", "0", "4095", "4094", "-", "1-3", "3-1", "access", "trunk", "native", "allowed",
        "all", "tag", "routing", "show", "clear", "mac-address-table", "dynamic", "static", "port", "address", "count", "mac-move", "static-mac",
        "02:00:00:00:00:01", "zz:zz", "age-time", "spanning-tree", "priority", "cost", "port-priority", "bpdu-guard", "lldp", "timer", "select-tlv",
        "logging", "severity", "debug", "console", "10.0.0.1", "lacp", "mode", "active", "rate", "fast", "hash", "l2", "exit", "end", "ip", "10.0.0.1/24",
        "/", "999999999999", "write", "memory", "shutdown", "name", "description", "on", "neighbor-info", "statistics", "running-config", "\u{e9}",
        ",", "1,2,,3", "fallback", "summary", "brief", "events", "-r", "?",
    ];
    let mut b = bridge(4);
    let mut s = CliSession::new();
    let mut x: u64 = 0x1234_5678;
    for i in 0..20_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let n = (x % 7) as usize;
        let mut line = String::new();
        for k in 0..n {
            let w = vocab[((x >> (k * 5)) as usize) % vocab.len()];
            line.push_str(w);
            line.push(' ');
        }
        let _ = exec(&mut b, &mut s, &line, i);
        let _ = prompt(&s);
    }
    let _ = running_config(&b, &[]);
}

#[test]
fn parsers() {
    assert_eq!(parse_mac("aa:bb:cc:dd:ee:ff"), Some([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]));
    assert_eq!(parse_mac("aa-bb-cc-dd-ee-ff"), Some([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]));
    assert_eq!(parse_mac("aabb.ccdd.eeff"), Some([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]));
    assert_eq!(parse_mac("aa:bb:cc:dd:ee"), None);
    assert_eq!(parse_mac("aa:bb:cc:dd:ee:fff"), None);
    assert_eq!(parse_mac("\u{e9}\u{e9}.\u{e9}\u{e9}.\u{e9}\u{e9}"), None);
    assert_eq!(parse_ipv4("10.0.0.1").map(|i| i.0), Some([10, 0, 0, 1]));
    assert!(parse_ipv4("10.0.0").is_none());
    assert!(parse_ipv4("10.0.0.256").is_none());
    assert!(parse_ipv4("1.2.3.4.5").is_none());
    assert!(parse_vlan_list("").is_err());
    assert!(parse_vlan_list("0").is_err());
    assert_eq!(format_vlan_list(&parse_vlan_list("5,1-3,4,9").unwrap()), "1-5,9");
    assert_eq!(parse_port_ref("lag3", 2), Some(PortRef::Lag(3)));
    assert_eq!(parse_port_ref("lag0", 2), None);
    assert_eq!(parse_port_ref("eth1", 2), Some(PortRef::Eth(1)));
    assert_eq!(parse_port_ref("2", 2), None);
}
