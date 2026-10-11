//! BGP open-peering CLI: peer groups, listen ranges, maximum-prefix, round trip, errors.
use ecm_bgp::{MaxPrefix, RemoteAs};
use ecm_net::Ipv4Addr;
use ecm_router::cli::{self, Session};
use ecm_router::Config;

const ISP: &str = "configure terminal
ip prefix-list TAP-IN seq 10 deny 100.64.0.0/10 le 32
ip prefix-list TAP-IN seq 20 deny 172.31.0.0/16 le 32
ip prefix-list TAP-IN seq 30 deny 0.0.0.0/0
ip prefix-list TAP-IN seq 40 permit 0.0.0.0/0 le 24
route-map TAP-IN permit 10
match ip address prefix-list TAP-IN
exit
router bgp 65003
bgp router-id 100.67.0.1
neighbor 172.31.2.1 remote-as 65002
neighbor 172.31.3.2 remote-as 65004
neighbor TAPS peer-group
neighbor TAPS remote-as external
neighbor TAPS listen ip-range 172.31.2.0/28 limit 8
neighbor TAPS listen ip-range 172.31.3.0/28 limit 8
address-family ipv4 unicast
neighbor 172.31.2.1 activate
neighbor 172.31.3.2 activate
neighbor TAPS activate
neighbor TAPS route-map TAP-IN in
neighbor TAPS maximum-prefix 20
network 100.67.0.0/24
end
";

fn ip(s: &str) -> Ipv4Addr {
    Ipv4Addr::parse(s).unwrap()
}

#[test]
fn open_peering_config_parses_and_round_trips() {
    let c = cli::load(ISP).unwrap();
    let g = &c.bgp.groups["TAPS"];
    assert_eq!(g.remote_as, Some(RemoteAs::External));
    assert!(g.active);
    assert_eq!(g.inbound.as_deref(), Some("TAP-IN"));
    assert_eq!(g.maximum_prefix, Some(MaxPrefix::new(20)));
    assert_eq!(c.bgp.listen.len(), 2);
    assert!(c.bgp.listen.iter().all(|r| r.limit == Some(8) && r.group == "TAPS"));
    let rendered = c.render();
    assert!(rendered.contains("neighbor TAPS peer-group\n"), "{}", rendered);
    assert!(rendered.contains("neighbor TAPS listen ip-range 172.31.3.0/28 limit 8\n"));
    assert!(rendered.contains("neighbor TAPS maximum-prefix 20\n"));
    let again = cli::load(&rendered).unwrap();
    assert_eq!(again.render(), rendered);
    assert_eq!(again.bgp, c.bgp);
}

#[test]
fn frr_spellings_map_to_the_same_config() {
    let frr = "configure terminal\nrouter bgp 65003\nbgp router-id 1.1.1.1\nneighbor TAPS peer-group\nneighbor TAPS remote-as external\nbgp listen range 172.31.3.0/28 peer-group TAPS\nbgp listen limit 8\naddress-family ipv4 unicast\nneighbor TAPS activate\nneighbor TAPS maximum-prefix 20 80\nend\n";
    let c = cli::load(frr).unwrap();
    assert_eq!(c.bgp.listen_limit, Some(8));
    assert_eq!(c.bgp.listen[0].group, "TAPS");
    assert_eq!(c.bgp.groups["TAPS"].maximum_prefix.unwrap().threshold, 80);
    let r = c.render();
    assert!(r.contains("bgp listen limit 8\n"));
    assert!(r.contains("neighbor TAPS listen ip-range 172.31.3.0/28\n"));
    assert!(r.contains("neighbor TAPS maximum-prefix 20 threshold 80\n"));
    assert_eq!(cli::load(&r).unwrap().render(), r);
}

#[test]
fn every_option_round_trips() {
    let text = "configure terminal\nrouter bgp 65200\nbgp router-id 10.200.0.1\nneighbor UP peer-group\nneighbor UP remote-as 65003\nneighbor UP update-source eth1\nneighbor UP listen ip-range 10.9.0.0/24 as-range 65001-65010,65100 limit 3\nneighbor 172.31.3.1 peer-group UP\nneighbor 172.31.3.9 remote-as internal\naddress-family ipv4 unicast\nneighbor UP activate\nneighbor 172.31.3.1 default-originate\nneighbor 172.31.3.9 activate\nneighbor 172.31.3.9 maximum-prefix 100 threshold 50 restart 60 warning-only\nend\n";
    let c = cli::load(text).unwrap();
    let r = c.render();
    for line in [
        "neighbor UP update-source eth1\n",
        "neighbor UP listen ip-range 10.9.0.0/24 as-range 65001-65010,65100 limit 3\n",
        "neighbor 172.31.3.1 peer-group UP\n",
        "neighbor 172.31.3.9 remote-as internal\n",
        "neighbor 172.31.3.9 maximum-prefix 100 threshold 50 restart 60 warning-only\n",
    ] {
        assert!(r.contains(line), "missing {:?} in\n{}", line, r);
    }
    assert_eq!(cli::load(&r).unwrap().bgp, c.bgp);
    let member = c.bgp.effective(&c.bgp.neighbors[&ip("172.31.3.1")]);
    assert_eq!(member.remote_as, Some(RemoteAs::Asn(65003)));
    assert!(member.active && member.default_originate);
}

fn run(lines: &[&str]) -> (Config, Vec<String>) {
    let mut c = Config::default();
    let mut s = Session::new();
    let out = lines.iter().map(|l| s.exec(&mut c, l).0).collect();
    (c, out)
}

#[test]
fn bad_syntax_is_rejected_without_changing_config() {
    let base = ["configure terminal", "router bgp 65003"];
    for (bad, why) in [
        ("neighbor GHOST listen ip-range 172.31.3.0/28", "Peer group GHOST does not exist"),
        ("neighbor 1.2.3.4 listen ip-range 172.31.3.0/28", "need a peer group"),
        ("neighbor TAPS listen ip-range 172.31.3.0/99", "Invalid prefix"),
        ("neighbor TAPS listen ip-range 0.0.0.0/0", "narrower"),
        ("neighbor TAPS listen ip-range 172.31.3.0/28 limit 0", "Invalid limit"),
        ("neighbor TAPS listen ip-range 172.31.3.0/28 limit 513", "Invalid limit"),
        ("neighbor TAPS listen ip-range 172.31.3.0/28 as-range 9-1", "Invalid AS range"),
        ("neighbor TAPS listen ip-range 172.31.3.0/28 bogus 1", "Expected"),
        ("neighbor TAPS remote-as banana", "Invalid remote AS"),
        ("neighbor TAPS remote-as 0", "Invalid remote AS"),
        ("neighbor 1.2.3.4 peer-group GHOST", "does not exist"),
        ("neighbor 9bad peer-group", "Invalid peer-group name"),
        ("bgp listen limit 0", "Invalid listen limit"),
        ("bgp listen range 172.31.3.0/28 peer-group GHOST", "does not exist"),
        ("no neighbor GHOST", "does not exist"),
    ] {
        let mut lines = base.to_vec();
        lines.push("neighbor TAPS peer-group");
        lines.push(bad);
        let (c, out) = run(&lines);
        let last = out.last().unwrap();
        assert!(last.starts_with('%') && last.contains(why), "{:?} -> {:?}", bad, last);
        assert!(c.bgp.listen.is_empty(), "{:?} must not add a range", bad);
        assert_eq!(c.bgp.groups["TAPS"].remote_as, None);
    }
    for (bad, why) in [
        ("neighbor 1.2.3.4 activate", "is not configured"),
        ("neighbor GHOST activate", "does not exist"),
        ("neighbor TAPS maximum-prefix 0", "Invalid maximum"),
        ("neighbor TAPS maximum-prefix 10 threshold 101", "Invalid threshold"),
        ("neighbor TAPS maximum-prefix 10 restart 5", "Invalid restart"),
        ("neighbor TAPS maximum-prefix 10 sometimes", "Expected"),
        ("neighbor TAPS route-map X sideways", "Expected in or out"),
    ] {
        let (c, out) = run(&[
            "configure terminal",
            "router bgp 65003",
            "neighbor TAPS peer-group",
            "address-family ipv4 unicast",
            bad,
        ]);
        let last = out.last().unwrap();
        assert!(last.starts_with('%') && last.contains(why), "{:?} -> {:?}", bad, last);
        let g = &c.bgp.groups["TAPS"];
        assert!(!g.active && g.maximum_prefix.is_none() && g.inbound.is_none());
    }
    // Commands of the wrong context are invalid.
    let (_, out) = run(&["configure terminal", "neighbor TAPS peer-group"]);
    assert!(out[1].starts_with("% Invalid command"));
    assert!(cli::load("configure terminal\nrouter bgp 1\nneighbor X listen ip-range 10.0.0.0/8\n").is_err());
}

#[test]
fn no_forms_remove_settings() {
    let (c, out) = run(&[
        "configure terminal",
        "router bgp 65003",
        "neighbor TAPS peer-group",
        "neighbor TAPS remote-as external",
        "neighbor TAPS listen ip-range 172.31.3.0/28",
        "neighbor TAPS listen ip-range 172.31.2.0/28",
        "neighbor 10.0.0.2 peer-group TAPS",
        "bgp listen limit 4",
        "address-family ipv4 unicast",
        "neighbor TAPS activate",
        "neighbor TAPS maximum-prefix 5",
        "no neighbor TAPS maximum-prefix",
        "no neighbor TAPS activate",
        "exit",
        "no neighbor TAPS listen ip-range 172.31.2.0/28",
        "no bgp listen limit",
    ]);
    assert!(out.iter().all(|o| o.is_empty()), "{:?}", out);
    assert_eq!(c.bgp.listen.len(), 1);
    assert_eq!(c.bgp.listen_limit, None);
    assert!(!c.bgp.groups["TAPS"].active);
    assert!(c.bgp.groups["TAPS"].maximum_prefix.is_none());
    let (c, _) = run(&[
        "configure terminal",
        "router bgp 65003",
        "neighbor TAPS peer-group",
        "neighbor TAPS listen ip-range 172.31.3.0/28",
        "neighbor 10.0.0.2 peer-group TAPS",
        "no neighbor TAPS",
    ]);
    assert!(c.bgp.groups.is_empty() && c.bgp.listen.is_empty() && c.bgp.neighbors.is_empty());
}
