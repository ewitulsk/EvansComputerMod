use super::*;

fn ip(s: &str) -> Ip {
    parse_ip(s).unwrap()
}

#[test]
fn messages_round_trip_and_junk_is_ignored() {
    let q = Msg::Query { id: 42, kind: "chat".into(), hops: 1 };
    assert_eq!(Msg::parse(&q.encode()), Some(q));
    assert_eq!(String::from_utf8(Msg::Query { id: 7, kind: "*".into(), hops: 0 }.encode()).unwrap(), "ECMDISCO/1 QUERY 7 * 0\n");
    let a = Msg::Answer {
        id: 9,
        services: vec![Service::new("chat", None, 7777, "Tech Village chat"), Service::new("pkg", Some(ip("100.74.0.21")), 8080, "")],
    };
    assert_eq!(String::from_utf8(a.encode()).unwrap(), "ECMDISCO/1 ANSWER 9\nchat - 7777 Tech Village chat\npkg 100.74.0.21 8080\n");
    assert_eq!(Msg::parse(&a.encode()), Some(a));
    for junk in [&b"hello"[..], b"ECMDISCO/2 QUERY 1 chat 0", b"ECMDISCO/1 QUERY x chat 0", b"ECMDISCO/1 QUERY 1 Chat! 0", b"\xff\xfe"] {
        assert_eq!(Msg::parse(junk), None, "{:?}", String::from_utf8_lossy(junk));
    }
    // Bad service lines are skipped, the rest kept.
    let m = Msg::parse(b"ECMDISCO/1 ANSWER 1\nchat 1.2.3 7777\nchat - 0\nchat - 7777 ok\n").unwrap();
    assert_eq!(m, Msg::Answer { id: 1, services: vec![Service::new("chat", None, 7777, "ok")] });
}

#[test]
fn names_are_one_clean_line() {
    assert_eq!(clean_name("  a\tb \n c\u{7}  "), "a b c");
    assert_eq!(clean_name(&"x".repeat(100)).len(), MAX_NAME);
    assert!(valid_type("pkg") && valid_type("my-svc2") && !valid_type("") && !valid_type("UP") && !valid_type(&"a".repeat(17)));
}

#[test]
fn config_lines() {
    let (c, bad) = Config::parse(
        "# the ring's services\nservice pkg 8080 My repo\nrecord chat 100.74.0.20 7777 Tech Village chat\nrelay gateway\n\nservice chat nope\nrecord chat 1.2.3 7777\nbogus\n",
    );
    assert_eq!(
        c.services,
        vec![Service::new("pkg", None, 8080, "My repo"), Service::new("chat", Some(ip("100.74.0.20")), 7777, "Tech Village chat")]
    );
    assert_eq!(c.relay, Relay::Gateway);
    assert_eq!(bad.len(), 3);
    assert_eq!(Config::parse("relay 10.0.0.1").0.relay, Relay::To(ip("10.0.0.1")));
    let mut c = Config::default();
    let (file, line) = announcement("chat", 7777, "Tech  Village chat");
    assert_eq!(file, "chat-7777");
    c.add_announced(&line);
    c.add_announced(&line);
    assert_eq!(c.services, vec![Service::new("chat", None, 7777, "Tech Village chat")]);
}

#[test]
fn a_responder_answers_for_its_services_only_when_it_has_some() {
    let (cfg, _) = Config::parse("service chat 7777 Room\nservice pkg 8080\n");
    let mut r = Responder::new(1);
    let asker = (ip("10.0.8.100"), 40000);
    let out = r.on_datagram(&cfg, None, asker, &Msg::Query { id: 5, kind: "pkg".into(), hops: 0 }.encode(), 0);
    assert_eq!(out, vec![Out { to: asker, msg: Msg::Answer { id: 5, services: vec![Service::new("pkg", None, 8080, "")] } }]);
    let all = r.on_datagram(&cfg, None, asker, &Msg::Query { id: 6, kind: "*".into(), hops: 0 }.encode(), 0);
    assert!(matches!(&all[0].msg, Msg::Answer { services, .. } if services.len() == 2));
    // Control: a type it doesn't have gets no answer at all (nothing to relay either).
    assert!(r.on_datagram(&cfg, None, asker, &Msg::Query { id: 7, kind: "ssh".into(), hops: 0 }.encode(), 0).is_empty());
    // Its own answers coming back (it hears its own group) are ignored.
    assert!(r.on_datagram(&cfg, None, asker, &all[0].msg.encode(), 0).is_empty());
}

#[test]
fn a_relay_passes_queries_up_and_answers_back_naming_the_upstream() {
    let (cfg, _) = Config::parse("relay gateway\n");
    let mut r = Responder::new(1);
    let pc = (ip("192.168.1.10"), 40000);
    let isp = ip("100.74.1.1");
    let out = r.on_datagram(&cfg, Some(isp), pc, &Msg::Query { id: 5, kind: "chat".into(), hops: 0 }.encode(), 100);
    assert_eq!(out.len(), 1);
    let Msg::Query { id: up_id, hops: 1, .. } = out[0].msg.clone() else { panic!("{:?}", out) };
    assert_eq!(out[0].to, (isp, PORT));
    assert_eq!(r.pending(), 1);
    // The same query again (to the group and to the gateway): not relayed twice.
    assert!(r.on_datagram(&cfg, Some(isp), pc, &Msg::Query { id: 5, kind: "chat".into(), hops: 0 }.encode(), 110).is_empty());
    // The ISP answers: '-' becomes the ISP, explicit hosts stay.
    let ans = Msg::Answer {
        id: up_id,
        services: vec![Service::new("chat", None, 7777, "isp"), Service::new("chat", Some(ip("100.74.0.20")), 7777, "Tech Village chat")],
    };
    let back = r.on_datagram(&cfg, Some(isp), (isp, PORT), &ans.encode(), 200);
    assert_eq!(
        back,
        vec![Out {
            to: pc,
            msg: Msg::Answer {
                id: 5,
                services: vec![Service::new("chat", Some(isp), 7777, "isp"), Service::new("chat", Some(ip("100.74.0.20")), 7777, "Tech Village chat")]
            }
        }]
    );
    // Controls: an answer from someone else with that id, and one after the relay stopped waiting.
    assert!(r.on_datagram(&cfg, Some(isp), (ip("6.6.6.6"), PORT), &ans.encode(), 300).is_empty());
    assert!(r.on_datagram(&cfg, Some(isp), (isp, PORT), &ans.encode(), 100 + RELAY_WAIT_MS + 1).is_empty());
    assert_eq!(r.pending(), 0);
}

#[test]
fn relays_stop_at_the_hop_limit_and_never_send_a_query_back() {
    let (cfg, _) = Config::parse("relay gateway\n");
    let mut r = Responder::new(1);
    let gw = ip("10.0.0.1");
    assert!(r.on_datagram(&cfg, Some(gw), (ip("10.0.0.5"), 1), &Msg::Query { id: 1, kind: "chat".into(), hops: MAX_HOPS }.encode(), 0).is_empty());
    assert!(r.on_datagram(&cfg, Some(gw), (gw, PORT), &Msg::Query { id: 2, kind: "chat".into(), hops: 0 }.encode(), 0).is_empty());
    assert!(r.on_datagram(&cfg, None, (ip("10.0.0.5"), 1), &Msg::Query { id: 3, kind: "chat".into(), hops: 0 }.encode(), 0).is_empty());
}

#[test]
fn a_search_keeps_one_of_each_and_prefers_names() {
    let mut s = Search::new(77, "pkg");
    let q = Msg::parse(&s.query()).unwrap();
    assert_eq!(q, Msg::Query { id: 77, kind: "pkg".into(), hops: 0 });
    let server = ip("10.0.8.1");
    s.on_datagram(server, &Msg::Answer { id: 77, services: vec![Service::new("pkg", None, 8080, "")] }.encode());
    s.on_datagram(server, &Msg::Answer { id: 77, services: vec![Service::new("pkg", Some(server), 8080, "Lab repo")] }.encode());
    // Controls: another search's answer, another type.
    s.on_datagram(server, &Msg::Answer { id: 78, services: vec![Service::new("pkg", None, 9000, "x")] }.encode());
    s.on_datagram(server, &Msg::Answer { id: 77, services: vec![Service::new("chat", None, 7777, "x")] }.encode());
    // This computer's own copy over loopback is dropped when its address answered too.
    s.on_datagram([127, 0, 0, 1], &Msg::Answer { id: 77, services: vec![Service::new("pkg", None, 8080, "Lab repo")] }.encode());
    s.on_datagram([127, 0, 0, 1], &Msg::Answer { id: 77, services: vec![Service::new("pkg", None, 9090, "")] }.encode());
    let r = s.results();
    assert_eq!(r.len(), 2, "{r:?}");
    assert_eq!((r[0].endpoint(), r[0].label()), ("10.0.8.1:8080".to_string(), "Lab repo".to_string()));
    assert_eq!((r[1].endpoint(), r[1].label()), ("127.0.0.1:9090".to_string(), "pkg on 127.0.0.1".to_string()));
    assert!(format_results(&r).contains("pkg   10.0.8.1:8080         Lab repo"));
}

#[test]
fn queries_go_to_this_computer_every_segment_and_the_gateway() {
    let ifaces = [(ip("192.168.1.10"), 24), (ip("0.0.0.0"), 0), (ip("127.0.0.1"), 8), (ip("100.80.0.5"), 22), (ip("172.31.2.5"), 32)];
    assert_eq!(multicast_interfaces(&ifaces), vec![ip("192.168.1.10"), ip("100.80.0.5"), ip("172.31.2.5")]);
    assert_eq!(targets(Some(ip("192.168.1.1"))), vec![ip("127.0.0.1"), ip("192.168.1.1")]);
    assert_eq!(targets(None), vec![ip("127.0.0.1")]);
    assert_eq!(targets(Some(ip("0.0.0.0"))), vec![ip("127.0.0.1")]);
    assert_eq!(GROUP, [224, 0, 0, 254]);
    let routes = r#"[{"dest":"192.168.1.0","prefix_len":24,"gateway":"0.0.0.0","iface_idx":1},{"dest":"0.0.0.0","prefix_len":0,"gateway":"192.168.1.1","iface_idx":1}]"#;
    assert_eq!(default_gateway(routes), Some(ip("192.168.1.1")));
    assert_eq!(default_gateway("[]"), None);
}

/// A house PC finds the ring's chat server: it asks its home router (a relay), which asks
/// the village ISP (records only). Datagrams are carried by hand between the three.
#[test]
fn through_a_home_router_to_the_isp() {
    let pc = ip("192.168.1.10");
    let home_lan = ip("192.168.1.1");
    let home_wan = ip("100.74.1.10");
    let isp = ip("100.74.1.1");
    let (home_cfg, _) = Config::parse("relay gateway\n");
    let (isp_cfg, _) = Config::parse("record chat 100.74.0.20 7777 Tech Village chat\nrecord pkg 100.74.0.21 8080 Tech-Village-Apps\n");
    let mut home = Responder::new(3);
    let mut isp_r = Responder::new(5);

    let mut search = Search::new(1000, "chat");
    // To the home router as the gateway (its copy through the group would be deduplicated).
    let up = home.on_datagram(&home_cfg, Some(isp), (pc, 40000), &search.query(), 0);
    assert_eq!(up.len(), 1);
    let at_isp = isp_r.on_datagram(&isp_cfg, None, (home_wan, PORT), &up[0].msg.encode(), 10);
    assert_eq!(at_isp.len(), 1);
    assert_eq!(at_isp[0].to, (home_wan, PORT));
    let back = home.on_datagram(&home_cfg, Some(isp), (isp, PORT), &at_isp[0].msg.encode(), 20);
    assert_eq!(back[0].to, (pc, 40000));
    search.on_datagram(home_lan, &back[0].msg.encode());
    let r = search.results();
    assert_eq!(r.len(), 1);
    assert_eq!((r[0].endpoint(), r[0].label()), ("100.74.0.20:7777".into(), "Tech Village chat".into()));
}
