//! Deterministic fuzzing: random and mutated datagrams into a server (authoritative and
//! recursive), its TCP side and the resolver; random text into the zone, named.conf and
//! hints parsers. Success = no panic, and the server still answers afterwards.

use ecm_dns::conf::NamedConf;
use ecm_dns::server::{Out, Server};
use ecm_dns::wire::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

const ZONE: &str = "$TTL 60\n@ SOA ns1 a 1 60 30 600 30\n@ NS ns1\nns1 A 10.0.0.1\nwww A 10.0.0.2\nsub NS ns.sub\nns.sub A 10.0.0.3\nc CNAME www\nt TXT \"x\"\nm MX 5 www\n";
const HINTS: &str = ". NS a.root.\na.root. A 10.0.0.9\n";

fn server() -> Server {
    let conf = NamedConf::parse("recursion yes\nroot-hints /h\nfallback 1.1.1.1\nzone lab.test primary /z\nzone other secondary 10.0.0.5\n").unwrap();
    let read = |p: &str| match p {
        "/z" => Some(ZONE.to_string()),
        "/h" => Some(HINTS.to_string()),
        _ => None,
    };
    Server::load(conf, &read, vec![[10, 0, 0, 1]], 42, 0)
}

fn seeds() -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    for (n, t) in [("www.lab.test", A), ("lab.test", AXFR), ("x.sub.lab.test", A), ("c.lab.test", A), ("example.com", MX), ("", NS)] {
        v.push(Message::query(7, n, t, true).encode());
    }
    let mut r = Message::response_to(&Message::query(9, "www.lab.test", A, false), NOERROR);
    r.answers.push(Record::new("www.lab.test", 60, RData::A([1, 2, 3, 4])));
    r.authority.push(Record::new("lab.test", 60, RData::Ns("ns1.lab.test".into())));
    v.push(r.encode());
    v
}

#[test]
fn datagrams_never_panic() {
    let mut s = server();
    let mut rng = Rng(0x9e3779b97f4a7c15);
    let seeds = seeds();
    for i in 0..60_000i64 {
        let base = &seeds[rng.below(seeds.len() as u64) as usize];
        let mut d = base.clone();
        match rng.below(4) {
            0 => {
                let n = rng.below(80) as usize;
                d = (0..n).map(|_| rng.next() as u8).collect();
            }
            _ => {
                for _ in 0..1 + rng.below(4) {
                    if d.is_empty() {
                        break;
                    }
                    let at = rng.below(d.len() as u64) as usize;
                    match rng.below(3) {
                        0 => d[at] = rng.next() as u8,
                        1 => d.truncate(at),
                        _ => d.insert(at, rng.next() as u8),
                    }
                }
            }
        }
        let from = [10, 0, 0, 1 + rng.below(9) as u8];
        match rng.below(3) {
            0 => s.on_udp(from, 1000, &d, i),
            1 => s.on_resolver_udp(from, &d, i),
            _ => {
                let _ = s.on_tcp(from, &d, i);
            }
        }
        s.tick(i);
        let _ = s.take_out();
    }
    // still answers
    s.on_udp([10, 0, 0, 7], 1000, &Message::query(1, "www.lab.test", A, false).encode(), 70_000);
    let out = s.take_out();
    let ans = out.iter().find_map(|o| match o {
        Out::Udp { data, .. } => Message::decode(data).ok(),
        _ => None,
    });
    assert_eq!(ans.unwrap().answers[0].data, RData::A([10, 0, 0, 2]));
}

#[test]
fn text_parsers_never_panic() {
    let mut rng = Rng(77);
    let alphabet: Vec<char> = "$@();\"\\ \n\t.ORIGINTTLSOANSACNAMEMXTXTzone primary secondary 0123456789/#ab-".chars().collect();
    for _ in 0..20_000 {
        let n = rng.below(120) as usize;
        let s: String = (0..n).map(|_| alphabet[rng.below(alphabet.len() as u64) as usize]).collect();
        let _ = ecm_dns::zone::Zone::parse(&s, "t");
        let _ = NamedConf::parse(&s);
        let _ = ecm_dns::resolver::parse_hints(&s);
    }
}
