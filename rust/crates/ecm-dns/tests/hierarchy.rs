//! Several `named` state machines on an in-memory network, laid out like the ring:
//! a hidden root primary (IANA), two root letters that are its secondaries, the `ring.`
//! registry with a secondary, a village zone on a web server, a resolver, and a fake
//! real-internet forwarder. Datagrams are delivered after 1 ms; servers can be "down".

use ecm_dns::conf::NamedConf;
use ecm_dns::name::{parse_ip, Ip};
use ecm_dns::server::{Out, Server};
use ecm_dns::wire::*;
use ecm_dns::xfr::AxfrClient;
use std::collections::{HashMap, HashSet};

const RESOLVER_PORT: u16 = 5300;
const CLIENT: Ip = [10, 9, 9, 9];

fn ip(s: &str) -> Ip {
    parse_ip(s).unwrap()
}

struct Net {
    servers: Vec<(Ip, Server)>,
    files: HashMap<Ip, HashMap<String, String>>,
    queue: Vec<(i64, Ip, u16, Ip, u16, Vec<u8>)>,
    now: i64,
    down: HashSet<Ip>,
    answers: Vec<Message>,
    logs: Vec<String>,
}

const ROOT_HINTS: &str = ". 3600000 NS a.root-servers.ring.\n. 3600000 NS b.root-servers.ring.\na.root-servers.ring. 3600000 A 100.65.0.54\nb.root-servers.ring. 3600000 A 100.67.0.54\n";

const ROOT_ZONE: &str = "$TTL 86400\n@ SOA a.root-servers.ring. nstld.nic.ring. 1 60 30 600 300\n@ NS a.root-servers.ring.\n@ NS b.root-servers.ring.\na.root-servers.ring. A 100.65.0.54\nb.root-servers.ring. A 100.67.0.54\nring. NS ns1.ring.ring.\nring. NS ns2.ring.ring.\nns1.ring.ring. A 100.68.0.55\nns2.ring.ring. A 100.73.0.55\nlame. NS ns.lame.\nns.lame. A 100.99.0.1\n";

const RING_ZONE: &str = "$TTL 3600\n@ SOA ns1.ring. hostmaster.nic.ring. 1 60 30 600 60\n@ NS ns1.ring.ring.\n@ NS ns2.ring.ring.\nns1.ring A 100.68.0.55\nns2.ring A 100.73.0.55\nv4 NS ns.v4\nns.v4 A 100.68.0.10\nchat CNAME web.v4\n";

const V4_ZONE: &str = "$TTL 300\n@ SOA ns hostmaster.nic.ring. 1 60 30 600 60\n@ NS ns\nns A 100.68.0.10\nweb A 100.68.0.10\nisp A 100.68.0.1\n";

const COM_ZONE: &str = "$TTL 300\n@ SOA ns a 1 60 30 600 60\n@ NS ns\nns A 1.1.1.1\nexample A 93.184.215.14\n";

impl Net {
    fn new() -> Net {
        Net { servers: Vec::new(), files: HashMap::new(), queue: Vec::new(), now: 0, down: HashSet::new(), answers: Vec::new(), logs: Vec::new() }
    }

    fn add(&mut self, addr: &str, conf: &str, files: &[(&str, &str)]) {
        let a = ip(addr);
        let fs: HashMap<String, String> = files.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let conf = NamedConf::parse(conf).unwrap();
        let read = |p: &str| fs.get(p).cloned();
        let s = Server::load(conf, &read, vec![a], u32::from_be_bytes(a), self.now);
        self.files.insert(a, fs);
        self.servers.push((a, s));
    }

    fn server(&mut self, a: &str) -> &mut Server {
        let a = ip(a);
        &mut self.servers.iter_mut().find(|(x, _)| *x == a).unwrap().1
    }

    fn collect(&mut self) {
        for i in 0..self.servers.len() {
            let (me, _) = self.servers[i];
            let outs = self.servers[i].1.take_out();
            for o in outs {
                match o {
                    Out::Udp { to, port, data } => self.queue.push((self.now + 1, me, 53, to, port, data)),
                    Out::Resolver { to, data } => self.queue.push((self.now + 1, me, RESOLVER_PORT, to, 53, data)),
                    Out::Log(l) => self.logs.push(format!("{}: {}", ecm_dns::name::ip_str(me), l)),
                    Out::Save { file, text } => {
                        self.files.get_mut(&me).unwrap().insert(file, text);
                    }
                    Out::Transfer { zone, primary } => {
                        let mut c = AxfrClient::new(&zone, 4242);
                        let result = if self.down.contains(&primary) {
                            Err("connection timed out".to_string())
                        } else {
                            let now = self.now;
                            let msgs = match self.servers.iter_mut().find(|(x, _)| *x == primary) {
                                Some((_, p)) => p.on_tcp(me, &c.query().encode(), now),
                                None => vec![],
                            };
                            let mut got = Err("no transfer".to_string());
                            for m in msgs {
                                match c.feed(&m.encode()) {
                                    Ok(Some(z)) => got = Ok(z),
                                    Ok(None) => {}
                                    Err(e) => got = Err(e),
                                }
                            }
                            got
                        };
                        let now = self.now;
                        self.servers[i].1.on_transfer(&zone, result, now);
                    }
                }
            }
        }
    }

    fn step(&mut self) {
        self.collect();
        let now = self.now;
        let due: Vec<_> = self.queue.iter().filter(|q| q.0 <= now).cloned().collect();
        self.queue.retain(|q| q.0 > now);
        for (_, from, fport, to, port, data) in due {
            if self.down.contains(&to) || self.down.contains(&from) {
                continue;
            }
            if to == CLIENT {
                self.answers.push(Message::decode(&data).unwrap());
                continue;
            }
            let Some((_, s)) = self.servers.iter_mut().find(|(x, _)| *x == to) else { continue };
            if port == RESOLVER_PORT {
                s.on_resolver_udp(from, &data, now);
            } else {
                s.on_udp(from, fport, &data, now);
            }
        }
        for (_, s) in self.servers.iter_mut() {
            s.tick(now);
        }
        self.collect();
        self.now += 1;
    }

    fn run(&mut self, ms: i64) {
        for _ in 0..ms {
            self.step();
        }
    }

    /// Ask `server` (RD) and wait for the answer.
    fn ask(&mut self, server: &str, name: &str, qtype: u16) -> Message {
        let id = (self.now as u16).wrapping_mul(7).wrapping_add(1);
        let q = Message::query(id, name, qtype, true);
        self.queue.push((self.now, CLIENT, 4000, ip(server), 53, q.encode()));
        for _ in 0..20_000 {
            self.step();
            if let Some(p) = self.answers.iter().position(|a| a.header.id == id) {
                return self.answers.remove(p);
            }
        }
        panic!("no answer for {name}; logs:\n{}", self.logs.join("\n"));
    }
}

fn ring() -> Net {
    let mut n = Net::new();
    // IANA: hidden primary of the root
    n.add("100.66.0.57", "zone . primary /etc/named/root.zone\n", &[("/etc/named/root.zone", ROOT_ZONE)]);
    // root letters a and b
    n.add("100.65.0.54", "zone . secondary 100.66.0.57\n", &[]);
    n.add("100.67.0.54", "zone . secondary 100.66.0.57\n", &[]);
    // ring registry primary and secondary
    n.add("100.68.0.55", "zone ring primary /etc/named/ring.zone\n", &[("/etc/named/ring.zone", RING_ZONE)]);
    n.add("100.73.0.55", "zone ring secondary 100.68.0.55\n", &[]);
    // village 4's web server
    n.add("100.68.0.10", "zone v4.ring primary /etc/named/v4.ring.zone\n", &[("/etc/named/v4.ring.zone", V4_ZONE)]);
    // the resolver, with the real internet behind 1.1.1.1
    n.add(
        "100.70.0.53",
        "recursion yes\nroot-hints /etc/named/root.hints\nfallback 1.1.1.1\n",
        &[("/etc/named/root.hints", ROOT_HINTS)],
    );
    n.add("1.1.1.1", "zone com primary /c.zone\n", &[("/c.zone", COM_ZONE)]);
    // secondaries pick their zones up
    n.run(200);
    n
}

#[test]
fn secondaries_load_by_transfer_and_notify() {
    let mut n = ring();
    for s in ["100.65.0.54", "100.67.0.54"] {
        assert_eq!(n.server(s).auth.find("ring").map(|z| z.serial()), Some(1), "{}", n.logs.join("\n"));
    }
    assert!(n.logs.iter().any(|l| l.contains("transferred serial 1")));
    assert_eq!(n.server("100.73.0.55").auth.zones.len(), 1);
    // the copy was written to disk for the next start
    assert!(n.files[&ip("100.65.0.54")].contains_key("/etc/named/sec/root.zone"));
    // a change at the primary: reload, NOTIFY, the secondary transfers
    let bumped = RING_ZONE.replace("hostmaster.nic.ring. 1 ", "hostmaster.nic.ring. 2 ") + "new A 100.68.0.99\n";
    n.files.get_mut(&ip("100.68.0.55")).unwrap().insert("/etc/named/ring.zone".into(), bumped);
    let fs = n.files[&ip("100.68.0.55")].clone();
    let conf = n.server("100.68.0.55").conf.clone();
    let now = n.now;
    n.server("100.68.0.55").reload(conf, &|p: &str| fs.get(p).cloned(), now);
    n.run(50);
    assert!(n.logs.iter().any(|l| l.contains("sending NOTIFY (serial 2) to 100.73.0.55")), "{}", n.logs.join("\n"));
    assert!(n.logs.iter().any(|l| l.contains("NOTIFY from 100.68.0.55 (serial 2)")));
    assert_eq!(n.server("100.73.0.55").auth.find("ring").unwrap().serial(), 2);
    // stop the primary: the secondary still answers authoritatively
    n.down.insert(ip("100.68.0.55"));
    let r = n.ask("100.73.0.55", "new.ring", A);
    assert!(r.header.aa);
    assert_eq!(r.answers[0].data, RData::A([100, 68, 0, 99]));
}

#[test]
fn iterative_resolution_with_trace_and_cache() {
    let mut n = ring();
    let r = n.ask("100.70.0.53", "web.v4.ring", A);
    assert_eq!(r.header.rcode, NOERROR);
    assert!(r.header.ra && !r.header.aa);
    assert_eq!(r.answers[0].data, RData::A([100, 68, 0, 10]));
    assert_eq!(r.answers[0].ttl, 300);
    // second time from the cache, the TTL counted down
    n.run(2000);
    let r = n.ask("100.70.0.53", "web.v4.ring", A);
    assert_eq!(r.answers[0].ttl, 298);
    // CNAME across zones: chat.ring -> web.v4.ring
    let r = n.ask("100.70.0.53", "chat.ring", A);
    assert_eq!(r.answers.len(), 2);
    assert_eq!(r.answers[0].rtype, CNAME);
    assert_eq!(r.answers[1].data, RData::A([100, 68, 0, 10]));
    // NXDOMAIN, then from the negative cache
    let r = n.ask("100.70.0.53", "nosuch.ring", A);
    assert_eq!(r.header.rcode, NXDOMAIN);
    assert_eq!(r.authority[0].rtype, SOA);
    // NODATA
    let r = n.ask("100.70.0.53", "web.v4.ring", MX);
    assert_eq!((r.header.rcode, r.answers.len()), (NOERROR, 0));
}

#[test]
fn trace_walks_root_tld_village() {
    let n = ring();
    let mut res = ecm_dns::resolver::Resolver::new(
        ecm_dns::resolver::Config { hints: ecm_dns::resolver::parse_hints(ROOT_HINTS).unwrap(), ..Default::default() },
        7,
    );
    // drive the resolver by hand against the network's servers
    let mut net = n;
    let q = res.resolve_fresh("web.v4.ring", A, net.now);
    let mut result = None;
    for _ in 0..5000 {
        for (to, data) in res.take_sends() {
            let now = net.now;
            let Some((_, s)) = net.servers.iter_mut().find(|(x, _)| *x == to) else { continue };
            s.on_udp([10, 0, 0, 1], 999, &data, now);
            for o in s.take_out() {
                if let Out::Udp { data, .. } = o {
                    res.on_packet(to, &data, now);
                }
            }
        }
        if let Some((id, r)) = res.take_done().pop() {
            assert_eq!(id, q);
            result = Some(r);
            break;
        }
        net.now += 1;
        res.tick(net.now);
    }
    let r = result.unwrap();
    let zones: Vec<&str> = r.steps.iter().map(|s| s.zone.as_str()).collect();
    assert_eq!(zones, vec!["", "ring", "v4.ring"]);
    assert_eq!(r.steps[0].server_name, "a.root-servers.ring");
    assert_eq!(r.answers[0].data, RData::A([100, 68, 0, 10]));
}

#[test]
fn a_dead_root_letter_and_lame_delegations() {
    let mut n = ring();
    n.down.insert(ip("100.65.0.54"));
    let r = n.ask("100.70.0.53", "isp.v4.ring", A);
    assert_eq!(r.answers[0].data, RData::A([100, 68, 0, 1]), "b.root answers");
    // the delegation to `lame.` points at nothing: SERVFAIL with a reason in the log
    let r = n.ask("100.70.0.53", "x.lame", A);
    assert_eq!(r.header.rcode, SERVFAIL);
    assert!(n.logs.iter().any(|l| l.contains("lame delegation")), "{}", n.logs.join("\n"));
    // the village web server stops: names in v4.ring fail, others still resolve
    n.down.insert(ip("100.68.0.10"));
    n.server("100.70.0.53").flush(None);
    assert_eq!(n.ask("100.70.0.53", "web.v4.ring", A).header.rcode, SERVFAIL);
    assert_eq!(n.ask("100.70.0.53", "ns1.ring.ring", A).header.rcode, NOERROR);
}

#[test]
fn real_internet_names_go_to_the_fallback() {
    let mut n = ring();
    let r = n.ask("100.70.0.53", "example.com", A);
    assert_eq!(r.header.rcode, NOERROR);
    assert_eq!(r.answers[0].data, RData::A([93, 184, 215, 14]));
    // special-use names never leave the game
    assert_eq!(n.ask("100.70.0.53", "foo.test", A).header.rcode, NXDOMAIN);
    // without the uplink: no answer, SERVFAIL
    n.down.insert(ip("1.1.1.1"));
    assert_eq!(n.ask("100.70.0.53", "other.com", A).header.rcode, SERVFAIL);
}

#[test]
fn recursion_only_for_allowed_clients() {
    let mut n = ring();
    // the web server does not recurse: REFUSED for other names, answers its own
    let r = n.ask("100.68.0.10", "example.com", A);
    assert_eq!(r.header.rcode, REFUSED);
    let r = n.ask("100.68.0.10", "web.v4.ring", A);
    assert!(r.header.aa && !r.header.ra);
    // a resolver that only allows 127.0.0.0/8
    n.add("100.71.0.53", "recursion yes\nallow 127.0.0.0/8\nroot-hints /h\n", &[("/h", ROOT_HINTS)]);
    assert_eq!(n.ask("100.71.0.53", "web.v4.ring", A).header.rcode, REFUSED);
}

#[test]
fn a_broken_zone_file_keeps_the_old_copy() {
    let mut n = ring();
    n.files.get_mut(&ip("100.68.0.10")).unwrap().insert("/etc/named/v4.ring.zone".into(), "$TTL 300\n@ SOA ns hostmaster.nic.ring. 2 60 30 600 60\nbroken line here\n".into());
    let fs = n.files[&ip("100.68.0.10")].clone();
    let conf = n.server("100.68.0.10").conf.clone();
    let now = n.now;
    n.server("100.68.0.10").reload(conf, &|p: &str| fs.get(p).cloned(), now);
    n.run(10);
    assert!(n.logs.iter().any(|l| l.contains("still serving serial 1")), "{}", n.logs.join("\n"));
    let r = n.ask("100.68.0.10", "web.v4.ring", A);
    assert_eq!(r.answers[0].data, RData::A([100, 68, 0, 10]));
}
