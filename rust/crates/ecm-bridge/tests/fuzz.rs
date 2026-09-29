//! Hostile input: 100k random and mutated frames on every port, plus
//! random send_local traffic and extreme clock values. Nothing may panic
//! and every table must stay within its bound.

use ecm_bridge::bpdu::{self, Bpdu, BpduBody};
use ecm_bridge::cli::{self, CliSession};
use ecm_bridge::lacpdu::{self, LacpInfo, Lacpdu};
use ecm_bridge::lldpdu::{self, Lldpdu};
use ecm_bridge::{frame, Bridge, MacAddr, Output, FDB_CAPACITY, LOG_CAPACITY, MAX_NEIGHBORS_PER_PORT, OUTPUT_CAPACITY};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

const NPORTS: usize = 6;

fn configured_bridge(now: i64) -> Bridge {
    let macs: Vec<MacAddr> = (0..NPORTS).map(|i| MacAddr([2, 0, 0, 0, 9, i as u8])).collect();
    let mut b = Bridge::new(&macs, MacAddr([2, 0, 0, 0, 9, 0xff]), now);
    let mut s = CliSession::new();
    let cfg = "spanning-tree
spanning-tree forward-delay 4
spanning-tree hello-time 1
logging severity debug
vlan 10
no shutdown
exit
vlan 20
no shutdown
exit
interface lag 1
no routing
vlan trunk native 1
lacp mode active
lacp rate fast
lacp fallback
exit
interface eth0
lag 1
exit
interface eth1
lag 1
exit
interface eth2
no routing
vlan access 10
spanning-tree admin-edge-port
exit
interface eth3
no routing
vlan trunk native 10
vlan trunk allowed 1,10,20
exit
interface eth4
no routing
spanning-tree bpdu-guard
exit
interface eth5
no routing
vlan trunk native 20 tag
spanning-tree root-guard
exit
interface vlan 10
ip address 10.0.10.1/24
exit";
    for l in cfg.lines() {
        let r = cli::exec(&mut b, &mut s, l, now);
        assert!(!r.output.contains("% "), "{}: {}", l, r.output);
    }
    for p in 0..NPORTS {
        b.set_link(p, true, now);
    }
    b
}

fn valid_frames(r: &mut Rng) -> Vec<Vec<u8>> {
    let mac = |r: &mut Rng| {
        let mut m = [0u8; 6];
        for b in m.iter_mut() {
            *b = r.byte();
        }
        m[0] &= 0xfe;
        m
    };
    let src = mac(r);
    let body = BpduBody {
        flags: r.byte(),
        root_id: r.next(),
        root_path_cost: r.next() as u32,
        bridge_id: r.next(),
        port_id: r.next() as u16,
        message_age: r.next() as u16,
        max_age: r.next() as u16,
        hello_time: r.next() as u16,
        forward_delay: r.next() as u16,
    };
    let info = |r: &mut Rng| LacpInfo {
        system_priority: r.next() as u16,
        system: mac(r),
        key: r.next() as u16,
        port_priority: r.next() as u16,
        port: r.next() as u16,
        state: r.byte(),
    };
    let lacp = Lacpdu { actor: info(r), partner: info(r), collector_max_delay: r.next() as u16 };
    let lldp = Lldpdu {
        chassis_id: vec![4, r.byte(), r.byte(), r.byte(), r.byte(), r.byte(), r.byte()],
        port_id: vec![5, b'x', r.byte()],
        ttl: (r.next() % 4) as u16 * 40,
        port_desc: Some("p".into()),
        sys_name: Some("fuzz".into()),
        sys_desc: None,
        caps: Some((4, 4)),
        mgmt_ipv4: Some([1, 2, 3, 4]),
        mgmt_ifindex: 1,
    };
    let dst = if r.below(3) == 0 { [0xff; 6] } else { mac(r) };
    let mut data = frame::eth_header(dst, src, 0x0800);
    data.extend((0..r.below(1500)).map(|_| r.byte()));
    let mut arp = frame::eth_header([0xff; 6], src, 0x0806);
    arp.extend_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1]);
    arp.extend((0..20).map(|_| r.byte()));
    vec![
        bpdu::build_frame(src, &Bpdu::Rst(body)),
        bpdu::build_frame(src, &Bpdu::Config(body)),
        bpdu::build_frame(src, &Bpdu::Tcn),
        lacpdu::build_frame(src, &lacp),
        lldpdu::build_frame(src, &lldp),
        data.clone(),
        frame::tag(&data, (r.next() % 4096) as u16, r.byte()).unwrap_or_default(),
        arp,
        {
            // to the bridge MAC
            let mut f = frame::eth_header([2, 0, 0, 0, 9, 0xff], src, 0x0800);
            f.extend_from_slice(&[0x45; 40]);
            f
        },
    ]
}

fn mutate(r: &mut Rng, mut f: Vec<u8>) -> Vec<u8> {
    match r.below(6) {
        0 => {} // unmodified
        1 => {
            let n = r.below(f.len() as u64 + 1) as usize;
            f.truncate(n);
        }
        2 => {
            for _ in 0..1 + r.below(8) {
                let i = r.below(f.len() as u64) as usize;
                if let Some(b) = f.get_mut(i) {
                    *b = r.byte();
                }
            }
        }
        3 => {
            let extra = r.below(2000) as usize;
            f.extend((0..extra).map(|_| r.byte()));
        }
        4 => {
            // Keep a plausible header, randomise the rest.
            for b in f.iter_mut().skip(14 + r.below(20) as usize) {
                *b = 0;
            }
            let len = f.len();
            if len > 20 {
                f[r.below(len as u64) as usize] ^= 0xff;
            }
        }
        _ => {
            let n = r.below(100) as usize;
            f = (0..n).map(|_| r.byte()).collect();
        }
    }
    f
}

fn drain(b: &mut Bridge, max_seen: &mut usize) {
    let mut n = 0;
    while let Some(o) = b.pop_output() {
        n += 1;
        if let Output::Tx { frame, .. } = o {
            assert!(frame.len() <= frame::MAX_FRAME_LEN + 4);
        }
    }
    *max_seen = (*max_seen).max(n);
}

#[test]
fn hundred_thousand_hostile_frames() {
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let mut now = 5_000;
    let mut b = configured_bridge(now);
    let mut max_out = 0;
    for i in 0..100_000u32 {
        let pool = valid_frames(&mut r);
        let pick = r.below(pool.len() as u64 + 1) as usize;
        let f = match pool.into_iter().nth(pick) {
            Some(f) => mutate(&mut r, f),
            None => (0..r.below(200)).map(|_| r.byte()).collect(),
        };
        let port = r.below(NPORTS as u64 + 2) as usize; // includes invalid ports
        b.handle_frame(port, &f, now);
        if i % 7 == 0 {
            let vlan = [1u16, 10, 20, 4095, 0][r.below(5) as usize];
            b.send_local(vlan, &f, now);
        }
        if i % 97 == 0 {
            now += r.below(3_000) as i64;
            if let Some(d) = b.poll(now) {
                assert!(d >= now);
            }
        }
        if i % 5_003 == 0 {
            let p = r.below(NPORTS as u64) as usize;
            b.set_link(p, false, now);
            b.set_link(p, true, now);
        }
        drain(&mut b, &mut max_out);
        assert!(b.fdb_len() <= FDB_CAPACITY);
    }
    assert!(b.log_entries().count() <= LOG_CAPACITY);
    for p in 0..NPORTS {
        assert!(b.lldp_neighbors(Some(p)).len() <= MAX_NEIGHBORS_PER_PORT);
    }
    assert!(max_out <= OUTPUT_CAPACITY);
    // Still a working switch afterwards: show commands and config render.
    let mut s = CliSession::new();
    for cmd in ["show spanning-tree", "show lacp interfaces", "show lldp neighbor-info", "show mac-address-table count", "show logging -r", "write memory"] {
        let _ = cli::exec(&mut b, &mut s, cmd, now);
    }
}

#[test]
fn extreme_clock_values_do_not_panic() {
    for start in [i64::MIN, i64::MIN / 2, -1, 0, i64::MAX / 2, i64::MAX - 1_000_000, i64::MAX] {
        let mut r = Rng(start as u64 | 1);
        let mut b = configured_bridge(start);
        let mut now = start;
        let mut max_out = 0;
        for i in 0..3_000 {
            let pool = valid_frames(&mut r);
            let f = pool.into_iter().nth(r.below(9) as usize).unwrap_or_default();
            b.handle_frame((i % NPORTS as u32) as usize, &f, now);
            now = now.saturating_add(r.below(5_000) as i64);
            let _ = b.poll(now);
            drain(&mut b, &mut max_out);
        }
        // Time going backwards is tolerated too.
        let _ = b.poll(start);
        let _ = b.poll(now.saturating_sub(1_000_000));
    }
}

#[test]
fn zero_port_bridge_is_inert() {
    let mut b = Bridge::new(&[], MacAddr([2, 0, 0, 0, 0, 1]), 0);
    b.handle_frame(0, &[0u8; 64], 0);
    b.set_link(3, true, 0);
    b.send_local(1, &[0xffu8; 64], 0);
    assert_eq!(b.poll(10), None);
    assert!(b.pop_output().is_none());
    assert!(!b.is_l2_port(0));
    let mut s = CliSession::new();
    for c in ["interface eth0", "show vlan", "show interface", "static-mac 02:00:00:00:00:01 vlan 1 port 0"] {
        let _ = cli::exec(&mut b, &mut s, c, 0);
    }
}
