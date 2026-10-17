//! End to end: phones, cell sites and a core on the simulated framed medium,
//! with an ideal backhaul (control messages and tunnels go through their sealed
//! byte encodings; a test can tamper with them on the way).

use ecm_cell::air::{Cause, Neighbour};
use ecm_cell::backhaul::{tunnel_split, Ctl};
use ecm_cell::codec::fmt_ip;
use ecm_cell::core::{ConnId, Core, CoreConfig, CoreOut, Entropy, SockAddr};
use ecm_cell::db::Db;
use ecm_cell::medium::{Medium, MediumConfig, Port, RadioId};
use ecm_cell::phy::{apply, Profile};
use ecm_cell::simauth::SoftSim;
use ecm_cell::site::{Site, SiteConfig, SiteOut};
use ecm_cell::ue::{Ue, UeConfig, UeEvent, UeOut};
use ecm_cell::Now;

const K1: [u8; 16] = [0x11; 16];
const K2: [u8; 16] = [0x22; 16];
const SITE_SECRET: [u8; 16] = [0x5e; 16];

struct Rng(u64);
impl Entropy for Rng {
    fn fill(&mut self, b: &mut [u8]) {
        for x in b {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            *x = self.0 as u8;
        }
    }
}

struct SiteNode {
    site: Site,
    radio: RadioId,
    conn: Option<ConnId>,
    addr: SockAddr,
    alive: bool,
    log: Vec<String>,
}

struct UeNode {
    ue: Ue,
    radio: RadioId,
    sim: SoftSim,
    events: Vec<UeEvent>,
    rx: Vec<Vec<u8>>,
}

struct World {
    m: Medium,
    now: u64,
    core: Option<Core>,
    db_text: String,
    core_log: Vec<String>,
    sites: Vec<SiteNode>,
    ues: Vec<UeNode>,
    lan: Vec<Vec<u8>>,
    next_conn: ConnId,
    /// Flip a bit in every uplink tunnel datagram (a box on the backhaul path).
    tamper_tunnel_up: bool,
    /// Flip a bit in the next N control messages from site 0.
    tamper_ctl_up: u32,
    /// The phones' slot clock lags the radios' by this many slots (a driver that reads
    /// the radio's clock now and then and counts forward, as celld does in game).
    ue_lag_slots: u64,
}

fn db(subs: &str) -> String {
    format!(
        "version 1\ncarrier cell-red\npool 100.65.2.0/24\ngateway 100.65.2.1\nsite site-a {s}\nsite site-b {s}\n{subs}",
        s = ecm_cell::codec::hex(&SITE_SECRET),
        subs = subs
    )
}

fn sub(imsi: &str, key: [u8; 16], extra: &str) -> String {
    format!("sub {} key={} {}\n", imsi, ecm_cell::codec::hex(&key), extra)
}

impl World {
    fn new(db_text: &str) -> World {
        let mut w = World {
            m: Medium::new(MediumConfig { exponent: 3.5, ..Default::default() }),
            now: 1000,
            core: None,
            db_text: db_text.into(),
            core_log: Vec::new(),
            sites: Vec::new(),
            ues: Vec::new(),
            lan: Vec::new(),
            next_conn: 1,
            tamper_tunnel_up: false,
            tamper_ctl_up: 0,
            ue_lag_slots: 0,
        };
        w.start_core();
        w
    }

    fn start_core(&mut self) {
        let db = Db::parse(&self.db_text).expect("db");
        self.core = Some(Core::new(db, CoreConfig::default(), Box::new(Rng(0x1234_5678 ^ self.now))));
    }

    fn stop_core(&mut self) {
        self.core = None;
        for s in &mut self.sites {
            if s.conn.take().is_some() {
                s.site.core_disconnected(Now { ms: self.now, slot: self.now });
            }
        }
    }

    fn add_site(&mut self, id: &str, cell: u32, ch: u8, x: f64, ip: [u8; 4], tweak: impl FnOnce(&mut SiteConfig)) -> usize {
        let radio = self.m.add_radio(id, [x, 0.0, 10.0]);
        let mut cfg = SiteConfig::new(id, SITE_SECRET, "cell-red", cell, 0xA000 + cell);
        cfg.channel = ch;
        tweak(&mut cfg);
        let mut site = Site::new(cfg);
        site.start(self.n());
        self.sites.push(SiteNode { site, radio, conn: None, addr: SockAddr { ip, port: 7701 }, alive: true, log: Vec::new() });
        self.sites.len() - 1
    }

    fn add_ue(&mut self, name: &str, addr: u32, x: f64, sim: SoftSim) -> usize {
        let radio = self.m.add_radio(name, [x, 0.0, 1.5]);
        let mut cfg = UeConfig::new(Profile::Lte, addr);
        cfg.channels = vec![1, 3];
        let mut ue = Ue::new(cfg);
        let mut sim = sim;
        ue.start(&mut sim, self.n());
        self.ues.push(UeNode { ue, radio, sim, events: Vec::new(), rx: Vec::new() });
        self.ues.len() - 1
    }

    fn n(&self) -> Now {
        Now { ms: self.now, slot: self.now } // LTE: 1 slot per ms
    }

    fn step(&mut self) {
        self.now += 5;
        let now = self.n();
        let us = self.now * 1000;
        self.m.advance(us);
        for i in 0..self.sites.len() {
            if !self.sites[i].alive {
                continue;
            }
            while let Some(e) = self.m.poll(self.sites[i].radio, us) {
                self.sites[i].site.on_phy(e, now);
            }
            if self.sites[i].conn.is_none() && self.sites[i].site.wants_core(now) {
                if let Some(core) = self.core.as_mut() {
                    let c = self.next_conn;
                    self.next_conn += 1;
                    self.sites[i].conn = Some(c);
                    self.sites[i].site.core_connected(now);
                    core.on_connect(c, self.sites[i].addr.ip, self.now);
                }
            }
            self.sites[i].site.tick(now);
        }
        let now = Now { ms: now.ms, slot: now.slot.saturating_sub(self.ue_lag_slots) };
        for i in 0..self.ues.len() {
            let u = &mut self.ues[i];
            while let Some(e) = self.m.poll(u.radio, us) {
                if std::env::var_os("E2E_DEBUG").is_some() { if let ecm_cell::phy::PhyEvent::Status(st) = &e { eprintln!("{} ue{} status {:?}", self.now, i, st); } }
                u.ue.on_phy(e, &mut u.sim, now);
            }
            u.ue.tick(now);
        }
        if let Some(c) = self.core.as_mut() {
            c.tick(self.now);
        }
        self.route();
    }

    fn route(&mut self) {
        let now = self.n();
        let us = self.now * 1000;
        for _ in 0..50 {
            let mut busy = false;
            for i in 0..self.sites.len() {
                let outs = self.sites[i].site.take_out();
                busy |= !outs.is_empty();
                for o in outs {
                    match o {
                        SiteOut::Phy(c) => {
                            if self.sites[i].alive {
                                apply(&mut Port { medium: &mut self.m, id: self.sites[i].radio, now_us: us }, [c]).unwrap();
                            }
                        }
                        SiteOut::Send(mut b) => {
                            if i == 0 && self.tamper_ctl_up > 0 && b.len() > 14 {
                                self.tamper_ctl_up -= 1;
                                let n = b.len();
                                b[n - 1] ^= 0x01;
                            }
                            if let (Some(c), Some(core)) = (self.sites[i].conn, self.core.as_mut()) {
                                // bytes in two pieces: the core reassembles
                                let (x, y) = b.split_at(b.len() / 2);
                                core.on_bytes(c, x, self.now);
                                core.on_bytes(c, y, self.now);
                            }
                        }
                        SiteOut::CloseCore => {
                            if let Some(c) = self.sites[i].conn.take() {
                                if let Some(core) = self.core.as_mut() {
                                    core.on_disconnect(c, self.now);
                                }
                            }
                        }
                        SiteOut::Tunnel(mut d) => {
                            if self.tamper_tunnel_up {
                                let n = d.len();
                                d[n / 2] ^= 0x10;
                            }
                            if let (Some(_), Some(core)) = (self.sites[i].conn, self.core.as_mut()) {
                                core.on_tunnel(self.sites[i].addr, &d, self.now);
                            }
                        }
                        SiteOut::Log(l) => self.sites[i].log.push(format!("{:>6} {}", self.now, l)),
                    }
                }
            }
            for i in 0..self.ues.len() {
                let outs = self.ues[i].ue.take_out();
                busy |= !outs.is_empty();
                for o in outs {
                    match o {
                        UeOut::Phy(c) => { if std::env::var_os("E2E_DEBUG").is_some() { if let ecm_cell::phy::PhyCmd::Tx(t) = &c { eprintln!("{} ue{} tx len {} slot {} sub {} cqi {} tag {}", self.now, i, t.payload.len(), t.slot, t.subchannel, t.cqi, t.tag); } } apply(&mut Port { medium: &mut self.m, id: self.ues[i].radio, now_us: us }, [c]).unwrap() }
                        UeOut::Ip(p) => self.ues[i].rx.push(p),
                        UeOut::Event(e) => self.ues[i].events.push(e),
                        UeOut::Log(_) => {}
                    }
                }
            }
            if let Some(core) = self.core.as_mut() {
                let outs = core.take_out();
                busy |= !outs.is_empty();
                for o in outs {
                    match o {
                        CoreOut::Send(c, b) => {
                            if let Some(s) = self.sites.iter_mut().find(|s| s.conn == Some(c)) {
                                s.site.on_bytes(&b, now);
                            }
                        }
                        CoreOut::Close(c) => {
                            if let Some(s) = self.sites.iter_mut().find(|s| s.conn == Some(c)) {
                                s.conn = None;
                                s.site.core_disconnected(now);
                            }
                        }
                        CoreOut::Tunnel(a, d) => {
                            if let Some(s) = self.sites.iter_mut().find(|s| s.addr == a && s.conn.is_some()) {
                                s.site.on_tunnel(&d, now);
                            }
                        }
                        CoreOut::ToLan(p) => self.lan.push(p),
                        CoreOut::SaveDb(t) => self.db_text = t,
                        CoreOut::Log(l) => self.core_log.push(format!("{:>6} {}", self.now, l)),
                    }
                }
            }
            if !busy {
                return;
            }
        }
    }

    fn run(&mut self, ms: u64) {
        let end = self.now + ms;
        while self.now < end {
            self.step();
        }
    }

    fn run_until(&mut self, ms: u64, mut f: impl FnMut(&World) -> bool) -> bool {
        let end = self.now + ms;
        while self.now < end {
            if f(self) {
                return true;
            }
            self.step();
        }
        f(self)
    }

    /// A phone's app sends a packet (routed at once, as a driver would).
    fn up(&mut self, u: usize, p: Vec<u8>) -> bool {
        let n = self.n();
        let n = Now { ms: n.ms, slot: n.slot.saturating_sub(self.ue_lag_slots) };
        let ok = self.ues[u].ue.send_ip(p, n);
        self.route();
        ok
    }

    /// A LAN host sends a packet to a phone through the core.
    fn down(&mut self, p: Vec<u8>) {
        let now = self.now;
        if let Some(c) = self.core.as_mut() {
            c.on_lan_packet(&p, now);
        }
        self.route();
    }

    fn ue_ip(&self, u: usize) -> Option<[u8; 4]> {
        self.ues[u].ue.status().ip.map(|c| c.ip)
    }

    fn attached(&self, u: usize) -> bool {
        self.ues[u].ue.status().state == "connected"
    }

    fn dump(&self) -> String {
        let mut s = String::new();
        s += &format!("lan {} packets: {:?}\n", self.lan.len(), self.lan.iter().map(|p| (p.len(), tag_of(p))).collect::<Vec<_>>());
        for i in 0..self.ues.len() {
            s += &format!("ue {} radio {:?}\n", i, self.m.stats(self.ues[i].radio));
        }
        for i in 0..self.sites.len() {
            s += &format!("site {} radio {:?}\n", i, self.m.stats(self.sites[i].radio));
        }
        s += "core:\n";
        for l in &self.core_log {
            s += &format!("  {}\n", l);
        }
        for (i, x) in self.sites.iter().enumerate() {
            s += &format!("site {} ({:?}):\n", i, x.site.status().core);
            for l in &x.log {
                s += &format!("  {}\n", l);
            }
        }
        for (i, u) in self.ues.iter().enumerate() {
            s += &format!("ue {}: {:?}\n  events {:?}\n", i, u.ue.status(), u.events);
        }
        s
    }
}

/// A UDP-ish IPv4 packet (protocol 17, no checksum checks anywhere here).
fn pkt(src: [u8; 4], dst: [u8; 4], len: usize, tag: u32) -> Vec<u8> {
    let mut p = vec![0u8; len.max(28)];
    p[0] = 0x45;
    let n = p.len() as u16;
    p[2..4].copy_from_slice(&n.to_be_bytes());
    p[8] = 64;
    p[9] = 17;
    p[12..16].copy_from_slice(&src);
    p[16..20].copy_from_slice(&dst);
    p[24..28].copy_from_slice(&tag.to_be_bytes());
    p
}

fn tag_of(p: &[u8]) -> u32 {
    u32::from_be_bytes([p[24], p[25], p[26], p[27]])
}

const SERVER: [u8; 4] = [100, 65, 0, 10];

#[test]
fn attach_and_data_both_ways() {
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    let u = w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w.run_until(5000, |w| w.attached(0)), "attach\n{}", w.dump());
    let ip = w.ue_ip(u).unwrap();
    assert_eq!(fmt_ip(ip), "100.65.2.2");
    assert!(w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Attached { cell: 1, .. })));
    // uplink: small and full-size packets
    for (i, len) in [64usize, 1500, 600].iter().enumerate() {
        
        assert!(w.up(0, pkt(ip, SERVER, *len, i as u32)));
    }
    assert!(w.run_until(2000, |w| w.lan.len() >= 3), "uplink\n{}", w.dump());
    assert_eq!(w.lan.iter().map(|p| p.len()).collect::<Vec<_>>(), vec![64, 1500, 600]);
    // downlink
    for (i, len) in [1500usize, 80].iter().enumerate() {
        w.down(pkt(SERVER, ip, *len, 100 + i as u32));
    }
    assert!(w.run_until(2000, |w| w.ues[0].rx.len() >= 2), "downlink\n{}", w.dump());
    assert_eq!(w.ues[0].rx.iter().map(|p| tag_of(p)).collect::<Vec<_>>(), vec![100, 101]);
    // a spoofed uplink source never reaches the LAN
    
    assert!(w.up(0, pkt([100, 65, 2, 99], SERVER, 64, 9)));
    w.run(500);
    assert_eq!(w.lan.len(), 3, "spoofed source dropped");
    let s = &w.core.as_ref().unwrap().sessions()[0];
    assert_eq!(s.state, "active");
    assert!(s.used >= 64 + 1500 + 600 + 1500 + 80, "usage {}", s.used);
    // detach
    let n = w.n();
    w.ues[0].ue.detach(n);
    w.route();
    assert!(w.run_until(2000, |w| w.ues[0].events.contains(&UeEvent::Detached)), "{}", w.dump());
    assert!(w.run_until(1000, |w| w.core.as_ref().unwrap().sessions().is_empty()), "{}", w.dump());
}

#[test]
fn attach_with_a_lagging_slot_clock() {
    // The phone answers on the next slot of its own (late) clock: the radio refuses those
    // slots as past, and the attach messages must go out on the next slot instead.
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.ue_lag_slots = 3;
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w.run_until(3000, |w| w.attached(0)), "attach with a lagging clock\n{}", w.dump());
}

#[test]
fn a_transmit_lead_covers_a_lagging_slot_clock() {
    // With a lead larger than the lag (celld schedules 5 ms ahead), nothing is refused as late,
    // so nothing is resent out of order: the first packet after the attach reaches the LAN.
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.ue_lag_slots = 3;
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    let u = w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    w.ues[u].ue.set_tx_lead_us(5000);
    assert!(w.run_until(3000, |w| w.attached(0)), "attach\n{}", w.dump());
    let ip = w.ue_ip(u).unwrap();
    assert!(w.up(0, pkt(ip, SERVER, 200, 1)));
    assert!(w.run_until(2000, |w| !w.lan.is_empty()), "uplink\n{}", w.dump());
}

/// Steady traffic both ways while the phone moves; returns (sent, received) per direction.
fn traffic_while(w: &mut World, ms: u64, every: u64) -> ((u32, u32), (u32, u32)) {
    let ip = w.ue_ip(0).unwrap();
    let (mut up, mut down) = (0u32, 0u32);
    let lan0 = w.lan.len();
    let rx0 = w.ues[0].rx.len();
    let end = w.now + ms;
    while w.now < end {
        if w.now.is_multiple_of(every) {
            if let Some(ip) = w.ue_ip(0) {
                w.up(0, pkt(ip, SERVER, 200, up));
            }
            up += 1;
            w.down(pkt(SERVER, ip, 300, 1_000_000 + down));
            down += 1;
        }
        w.step();
    }
    w.run(500);
    ((up, (w.lan.len() - lan0) as u32), (down, (w.ues[0].rx.len() - rx0) as u32))
}

#[test]
fn handover_same_channel_keeps_ip_and_traffic() {
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_site("site-b", 2, 1, 1200.0, [100, 66, 1, 2], |_| {});
    w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w.run_until(5000, |w| w.attached(0)), "{}", w.dump());
    assert_eq!(w.ues[0].ue.status().cell, Some(1));
    let ip = w.ue_ip(0);
    let (r, now) = (w.ues[0].radio, w.now * 1000);
    w.m.move_to(r, [1100.0, 0.0, 1.5], now, 12_000_000);
    let ((up, upr), (down, downr)) = traffic_while(&mut w, 14_000, 50);
    let d = w.dump();
    assert_eq!(w.ues[0].ue.status().cell, Some(2), "{}", d);
    assert!(w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Handover { from: 1, to: 2, channel: 1 })), "{}", d);
    assert!(!w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Released { .. })), "no re-attach: {}", d);
    assert_eq!(w.ue_ip(0), ip, "address kept");
    assert!(upr * 100 >= up * 95 && downr * 100 >= down * 95, "up {}/{} down {}/{}\n{}", upr, up, downr, down, d);
    // no duplicates downlink
    let mut tags: Vec<u32> = w.ues[0].rx.iter().map(|p| tag_of(p)).collect();
    let n = tags.len();
    tags.sort();
    tags.dedup();
    assert_eq!(tags.len(), n, "duplicates delivered");
    assert!(w.core_log.iter().any(|l| l.contains("handover 310150000000001 cell 1 -> 2")), "{}", d);
}

#[test]
fn handover_across_channels() {
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |c| c.neighbours = vec![Neighbour { cell_id: 2, profile: 1, channel: 3, ul_mask: 0 }]);
    w.add_site("site-b", 2, 3, 1200.0, [100, 66, 1, 2], |c| c.neighbours = vec![Neighbour { cell_id: 1, profile: 1, channel: 1, ul_mask: 0 }]);
    w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w.run_until(5000, |w| w.attached(0)), "{}", w.dump());
    assert_eq!(w.ues[0].ue.status().cell, Some(1));
    let ip = w.ue_ip(0);
    let (r, now) = (w.ues[0].radio, w.now * 1000);
    w.m.move_to(r, [1100.0, 0.0, 1.5], now, 12_000_000);
    let ((up, upr), (down, downr)) = traffic_while(&mut w, 14_000, 50);
    let d = w.dump();
    assert!(w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Handover { from: 1, to: 2, channel: 3 })), "{}", d);
    assert!(!w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Released { .. })), "{}", d);
    assert_eq!(w.ue_ip(0), ip);
    assert!(upr * 100 >= up * 90 && downr * 100 >= down * 90, "up {}/{} down {}/{}\n{}", upr, up, downr, down, d);
}

#[test]
fn wrong_carrier_is_not_used_and_is_rejected() {
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_ue("blue", 0xB001, 100.0, SoftSim::new("cell-blue", "310150000000001", K1));
    w.run(3000);
    let d = w.dump();
    assert!(!w.attached(0));
    assert!(w.ues[0].events.iter().any(|e| matches!(e, UeEvent::NoService { heard } if heard == &vec!["cell-red".to_string()])), "{}", d);
    // a site that claims another carrier: its phones are refused by the core
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |c| c.carrier = "cell-blue".into());
    w.add_ue("blue", 0xB001, 100.0, SoftSim::new("cell-blue", "310150000000001", K1));
    assert!(w.run_until(5000, |w| w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Rejected { cause: Cause::WrongCarrier }))), "{}", w.dump());
    assert_eq!(w.ues[0].ue.status().state, "rejected", "permanent: no retries");
}

#[test]
fn forged_sim_is_rejected() {
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_ue("forged", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K2));
    assert!(w.run_until(5000, |w| w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Rejected { cause: Cause::AuthFailed }))), "{}", w.dump());
    w.run(5000);
    assert_eq!(w.ues[0].ue.status().state, "rejected");
    assert_eq!(w.core_log.iter().filter(|l| l.contains("authentication failed")).count(), 1, "{}", w.dump());
    assert!(w.core.as_ref().unwrap().sessions().is_empty());
    // unknown IMSI too
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_ue("stranger", 0xB002, 100.0, SoftSim::new("cell-red", "310150000000777", K1));
    assert!(w.run_until(5000, |w| w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Rejected { cause: Cause::UnknownSubscriber }))), "{}", w.dump());
}

#[test]
fn quota_exhaustion() {
    let mut w = World::new(&db(&sub("310150000000001", K1, "quota=3000")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w.run_until(5000, |w| w.attached(0)), "{}", w.dump());
    let ip = w.ue_ip(0).unwrap();
    for i in 0..10 {
        
        w.up(0, pkt(ip, SERVER, 500, i));
        w.run(100);
    }
    let d = w.dump();
    assert!(w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Released { cause: Cause::QuotaExhausted })), "{}", d);
    assert!(w.lan.len() < 10 && w.lan.len() >= 5, "{} delivered before the cut", w.lan.len());
    assert!(w.db_text.contains("quota=3000 used=3"), "{}", w.db_text);
    // the core keeps refusing (the phone backs off; an operator top-up lets it back)
    let now = w.now;
    w.core.as_mut().unwrap().admin("reset-usage 310150000000001", now).unwrap();
    w.route();
    assert!(w.db_text.contains("used=0"));
}

#[test]
fn revoked_sim() {
    // revoked from the start
    let mut w = World::new(&db(&sub("310150000000001", K1, "revoked=1")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w.run_until(5000, |w| w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Rejected { cause: Cause::Revoked }))), "{}", w.dump());
    assert!(w.sites[0].log.iter().any(|l| l.contains("refused here: SIM revoked")), "the site refuses locally: {}", w.dump());
    // revoked while attached
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w.run_until(5000, |w| w.attached(0)), "{}", w.dump());
    let now = w.now;
    w.core.as_mut().unwrap().admin("revoke 310150000000001", now).unwrap();
    w.route();
    assert!(w.run_until(2000, |w| w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Released { cause: Cause::Revoked }))), "{}", w.dump());
    w.run(3000);
    assert_eq!(w.ues[0].ue.status().state, "rejected");
    assert!(w.db_text.contains("revoked=1"));
}

#[test]
fn core_restart_reloads_the_database() {
    let mut w = World::new(&db(&(sub("310150000000001", K1, "") + &sub("310150000000002", K2, ""))));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_ue("ue2", 0xB002, 120.0, SoftSim::new("cell-red", "310150000000002", K2));
    assert!(w.run_until(5000, |w| w.attached(0)), "{}", w.dump());
    let ip = w.ue_ip(0);
    assert!(w.db_text.contains("ip=100.65.2.2"), "address saved: {}", w.db_text);
    w.stop_core();
    w.run(2000);
    assert!(!w.attached(0));
    assert!(w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Released { cause: Cause::CoreUnavailable })), "{}", w.dump());
    w.start_core(); // from the saved file
    assert!(w.run_until(8000, |w| w.attached(0)), "{}", w.dump());
    assert_eq!(w.ue_ip(0), ip, "same address after the restart");
    let ip = ip.unwrap();
    
    assert!(w.up(0, pkt(ip, SERVER, 100, 1)));
    assert!(w.run_until(1000, |w| !w.lan.is_empty()));
}

#[test]
fn site_failure_and_reattach() {
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_site("site-b", 2, 3, 400.0, [100, 66, 1, 2], |_| {});
    w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w.run_until(5000, |w| w.attached(0) && w.ues[0].ue.status().cell == Some(1)), "{}", w.dump());
    let ip = w.ue_ip(0);
    // site A loses power: radio silent, backhaul gone
    w.sites[0].alive = false;
    w.m.set_on(w.sites[0].radio, false);
    if let Some(c) = w.sites[0].conn.take() {
        let now = w.now;
        w.core.as_mut().unwrap().on_disconnect(c, now);
    }
    assert!(w.run_until(8000, |w| w.attached(0) && w.ues[0].ue.status().cell == Some(2)), "{}", w.dump());
    assert_eq!(w.ue_ip(0), ip, "address kept across the re-attach");
    assert!(w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Released { cause: Cause::RadioLost })));
}

#[test]
fn idle_and_paging() {
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |c| c.idle_after_ms = 2000);
    w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w.run_until(5000, |w| w.attached(0)), "{}", w.dump());
    let ip = w.ue_ip(0).unwrap();
    assert!(w.run_until(5000, |w| w.ues[0].ue.status().state == "idle"), "{}", w.dump());
    assert_eq!(w.core.as_ref().unwrap().sessions()[0].state, "idle");
    w.down(pkt(SERVER, ip, 120, 77));
    assert!(w.run_until(3000, |w| !w.ues[0].rx.is_empty()), "paged and delivered\n{}", w.dump());
    assert_eq!(tag_of(&w.ues[0].rx[0]), 77);
    assert!(w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Resumed { cell: 1 })), "{}", w.dump());
    assert_eq!(w.ue_ip(0), Some(ip));
    // uplink from idle resumes too
    assert!(w.run_until(5000, |w| w.ues[0].ue.status().state == "idle"), "{}", w.dump());
    let before = w.lan.len();
    w.up(0, pkt(ip, SERVER, 64, 5));
    assert!(w.run_until(3000, |w| w.lan.len() > before), "{}", w.dump());
}

#[test]
fn many_phones_share_a_cell() {
    let mut subs = String::new();
    for i in 0..6 {
        subs += &sub(&format!("31015000000010{}", i), K1, "");
    }
    let mut w = World::new(&db(&subs));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    for i in 0..6 {
        w.add_ue(&format!("ue{}", i), 0xB100 + i, 50.0 + 30.0 * i as f64, SoftSim::new("cell-red", &format!("31015000000010{}", i), K1));
    }
    assert!(w.run_until(10_000, |w| (0..6).all(|i| w.attached(i))), "{}", w.dump());
    let ips: std::collections::BTreeSet<_> = (0..6).map(|i| w.ue_ip(i).unwrap()).collect();
    assert_eq!(ips.len(), 6);
    for i in 0..6 {
        for k in 0..5 {
            let ip = w.ue_ip(i).unwrap();
            w.up(i, pkt(ip, SERVER, 400, (i * 10 + k) as u32));
        }
    }
    assert!(w.run_until(3000, |w| w.lan.len() >= 30), "{} of 30\n{}", w.lan.len(), w.dump());
}

#[test]
fn site_with_a_bad_credential_is_refused() {
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |c| c.secret = [0; 16]);
    w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    w.run(3000);
    assert!(!w.attached(0));
    assert!(w.core_log.iter().any(|l| l.contains("refused: bad credential")), "{}", w.dump());
    assert!(w.sites[0].log.iter().any(|l| l.contains("core refused this site")), "{}", w.dump());
    let _ = Ctl::Heartbeat { seq: 0 };
}

/// The backhaul is sealed: tampered or forged tunnel datagrams and a tampered
/// control message are refused; the network recovers once the path is clean.
#[test]
fn tampered_backhaul_is_rejected() {
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w.run_until(5000, |w| w.attached(0)), "{}", w.dump());
    let ip = w.ue_ip(0).unwrap();
    // control: clean uplink works
    w.up(0, pkt(ip, SERVER, 200, 1));
    assert!(w.run_until(1000, |w| w.lan.iter().any(|p| tag_of(p) == 1)), "{}", w.dump());
    // a box on the path flips a bit in every uplink tunnel datagram: nothing gets through
    w.tamper_tunnel_up = true;
    for t in 2..7 {
        w.up(0, pkt(ip, SERVER, 200, t));
    }
    w.run(1000);
    assert!(!w.lan.iter().any(|p| (2..7).contains(&tag_of(p))), "tampered datagrams reached the LAN\n{}", w.dump());
    assert!(w.core.as_ref().unwrap().tunnel_rejects >= 5, "{}", w.dump());
    assert!(w.core_log.iter().any(|l| l.contains("failed authentication, dropped")), "{}", w.dump());
    w.tamper_tunnel_up = false;
    // a forged datagram from the site's own address: the old plaintext format, or a sealed one replayed
    let forged = tunnel_split(1, 999, &pkt(ip, SERVER, 100, 77)).remove(0);
    let before = w.core.as_ref().unwrap().tunnel_rejects;
    let t = w.now;
    w.core.as_mut().unwrap().on_tunnel(SockAddr { ip: [100, 65, 1, 2], port: 7701 }, &forged, t);
    w.route();
    assert_eq!(w.core.as_ref().unwrap().tunnel_rejects, before + 1);
    assert!(!w.lan.iter().any(|p| tag_of(p) == 77));
    // clean again: traffic flows
    w.up(0, pkt(ip, SERVER, 200, 8));
    assert!(w.run_until(1000, |w| w.lan.iter().any(|p| tag_of(p) == 8)), "{}", w.dump());
    // a tampered control message: the core closes the connection, the site registers again
    w.tamper_ctl_up = 1;
    w.run(1500); // the next heartbeat carries the flipped bit
    assert!(w.core_log.iter().any(|l| l.contains("site site-a: control message failed authentication, closing")), "{}", w.dump());
    assert!(w.run_until(15_000, |w| w.attached(0) && w.core.as_ref().unwrap().site_ids().contains("site-a")), "{}", w.dump());
    assert_eq!(w.ue_ip(0), Some(ip), "the address is sticky");
}

/// Garbage on every input (radio frames, control bytes, tunnel datagrams,
/// LAN packets) of a running network: nothing panics and the phone stays up.
#[test]
fn garbage_inputs_never_panic() {
    use ecm_cell::phy::{PhyEvent, RxFrame};
    let mut w = World::new(&db(&sub("310150000000001", K1, "")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w.run_until(5000, |w| w.attached(0)), "{}", w.dump());
    let mut x: u64 = 0x2545F4914F6CDD1D;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let ue_addr = 0xB001;
    let site_addr = 0xA001;
    for round in 0..3000 {
        let n = (rnd() % 120) as usize;
        let mut b: Vec<u8> = (0..n).map(|_| rnd() as u8).collect();
        if round % 3 == 0 && !b.is_empty() {
            b[0] = 0x10; // looks protected
        }
        if round % 5 == 0 && !b.is_empty() {
            b[0] = (rnd() % 12) as u8; // a plain type
        }
        let now = w.n();
        let f = |src: u32, dst: u32, payload: Vec<u8>| RxFrame { src, dst, cqi: 9, payload, ..Default::default() };
        w.sites[0].site.on_phy(PhyEvent::Rx(f(ue_addr, site_addr, b.clone())), now);
        w.sites[0].site.on_phy(PhyEvent::Rx(f(0xDEAD, site_addr, b.clone())), now);
        let u = &mut w.ues[0];
        u.ue.on_phy(PhyEvent::Rx(f(site_addr, ue_addr, b.clone())), &mut u.sim, now);
        u.ue.on_phy(PhyEvent::Rx(f(site_addr, ecm_cell::phy::BROADCAST, b.clone())), &mut u.sim, now);
        w.sites[0].site.on_tunnel(&b, now);
        let t = w.now;
        let core = w.core.as_mut().unwrap();
        core.on_tunnel(SockAddr { ip: [100, 65, 1, 2], port: 7701 }, &b, t);
        core.on_lan_packet(&b, t);
        if round % 100 == 0 {
            // a garbage control stream on a fresh connection is closed, not fatal
            core.on_connect(9000 + round, [6, 6, 6, 6], t);
            core.on_bytes(9000 + round, &b, t);
        }
        if round % 10 == 0 {
            w.step();
        }
    }
    w.run(2000);
    assert!(w.attached(0), "{}", w.dump());
    let ip = w.ue_ip(0).unwrap();
    let before = w.lan.len();
    w.up(0, pkt(ip, SERVER, 100, 4242));
    assert!(w.run_until(1000, |w| w.lan.len() > before), "{}", w.dump());
}

// ------------------------------------------------------------ telephony (docs/TELEPHONY.md)

const PCSCF: [u8; 4] = [100, 65, 2, 1];

fn ims_db(subs: &str) -> String {
    db(&format!("block 214-555\nims pool=10.250.0.0/24 domain=cell-red.ecm net=100.65.2.1/32\n{}", subs))
}

/// An IPv4 UDP packet with ports (the tag sits where the UDP length/checksum go).
fn udp(src: [u8; 4], dst: [u8; 4], sport: u16, dport: u16, len: usize, tag: u32) -> Vec<u8> {
    let mut p = pkt(src, dst, len, tag);
    p[20..22].copy_from_slice(&sport.to_be_bytes());
    p[22..24].copy_from_slice(&dport.to_be_bytes());
    p
}

fn ims_of(w: &World, u: usize) -> Option<ecm_cell::air::ImsInfo> {
    w.ues[u].ue.status().ims
}

#[test]
fn ims_bearer_comes_with_the_attach_and_reaches_only_the_ims_core() {
    let mut w = World::new(&ims_db(&(sub("310150000000001", K1, "msisdn=+12145550142") + &sub("310150000000002", K2, ""))));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w.run_until(5000, |w| w.attached(0) && ims_of(w, 0).is_some()), "{}", w.dump());
    let ims = ims_of(&w, 0).unwrap();
    assert_eq!((ims.ip, ims.prefix, ims.pcscf), ([10, 250, 0, 2], 24, PCSCF));
    assert_eq!((ims.domain.as_str(), ims.msisdn.as_str()), ("cell-red.ecm", "+12145550142"));
    let core = w.core.as_ref().unwrap();
    // the second subscriber got a number from the block when the core loaded the database
    assert_eq!(core.db().subs["310150000000002"].ims.msisdn, "+12145550100");
    let r = core.reach("+12145550142").unwrap();
    assert_eq!((r.imsi.as_str(), r.ims_ip, r.idle), ("310150000000001", Some([10, 250, 0, 2]), false));
    assert_eq!(core.imsi_at([10, 250, 0, 2]), Some("310150000000001"));
    // SIP to the P-CSCF goes out; the same address towards the internet is dropped
    let used0 = core.db().subs["310150000000001"].used;
    let before = w.lan.len();
    w.up(0, udp(ims.ip, PCSCF, 5060, 5060, 400, 1));
    w.up(0, udp(ims.ip, SERVER, 5060, 80, 400, 2));
    w.run(500);
    let got: Vec<u32> = w.lan[before..].iter().map(|p| tag_of(p)).collect();
    assert_eq!(got, vec![1], "{}", w.dump());
    let core = w.core.as_ref().unwrap();
    assert_eq!(core.ims_policy_drops, 1);
    assert_eq!(core.db().subs["310150000000001"].used, used0, "the IMS bearer is not counted");
    // downlink to the IMS address arrives (the P-CSCF answering)
    w.down(udp(PCSCF, ims.ip, 5060, 5060, 300, 3));
    assert!(w.run_until(2000, |w| w.ues[0].rx.iter().any(|p| tag_of(p) == 3)), "{}", w.dump());
    // a SIM without a number gets no IMS bearer
    let mut w2 = World::new(&db(&sub("310150000000001", K1, "")));
    w2.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w2.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w2.run_until(5000, |w| w.attached(0)));
    w2.run(300);
    assert!(ims_of(&w2, 0).is_none());
}

#[test]
fn a_voice_bearer_goes_first_and_moves_with_a_handover() {
    let mut w = World::new(&ims_db(&sub("310150000000001", K1, "msisdn=+12145550142")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_site("site-b", 2, 1, 1200.0, [100, 66, 1, 2], |_| {});
    w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w.run_until(5000, |w| w.attached(0) && ims_of(w, 0).is_some()), "{}", w.dump());
    let ims = ims_of(&w, 0).unwrap().ip;
    let media = [100, 65, 2, 1];
    let (bid, sent) = w.core.as_mut().unwrap().bearer_setup(ims, 40000, media, 30000).unwrap();
    assert!(sent);
    w.route();
    assert!(w.run_until(1000, |w| w.ues[0].ue.status().bearers == 1), "{}", w.dump());
    assert!(w.core.as_ref().unwrap().sessions()[0].bearers[0].1, "admitted by the site");
    assert!(w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Bearer { up: true, .. })));
    // a download is queued, then a voice frame: the voice frame overtakes it
    let ip = w.ue_ip(0).unwrap();
    for i in 0..40 {
        w.down(pkt(SERVER, ip, 1400, 100 + i));
    }
    w.down(udp(media, ims, 30000, 40000, 120, 7));
    assert!(w.run_until(3000, |w| w.ues[0].rx.iter().any(|p| tag_of(p) == 7)), "{}", w.dump());
    let pos = w.ues[0].rx.iter().position(|p| tag_of(p) == 7).unwrap();
    assert!(pos < 10, "voice delivered after {} bulk packets", pos);
    w.run(3000);
    // the call keeps going across a handover: a frame each way every 20 ms
    let rx0 = w.ues[0].rx.len();
    let lan0 = w.lan.len();
    let (r, now) = (w.ues[0].radio, w.now * 1000);
    w.m.move_to(r, [1100.0, 0.0, 1.5], now, 12_000_000);
    let mut n = 0u32;
    let end = w.now + 14_000;
    while w.now < end {
        if w.now % 20 == 0 {
            w.down(udp(media, ims, 30000, 40000, 100, 10_000 + n));
            w.up(0, udp(ims, media, 40000, 30000, 100, 20_000 + n));
            n += 1;
        }
        w.step();
    }
    w.run(500);
    let d = w.dump();
    assert_eq!(w.ues[0].ue.status().cell, Some(2), "{}", d);
    assert_eq!(w.ues[0].ue.status().bearers, 1, "the bearer is still up: {}", d);
    assert!(w.core.as_ref().unwrap().sessions()[0].bearers[0].1, "admitted by the target: {}", d);
    // longest run of lost frames each way (20 ms each): under 100 ms per handover
    let worst = |got: Vec<u32>, base: u32| {
        let set: std::collections::BTreeSet<u32> = got.into_iter().collect();
        let (mut run, mut worst) = (0, 0);
        for k in 0..n {
            if set.contains(&(base + k)) {
                run = 0;
            } else {
                run += 1;
                worst = worst.max(run);
            }
        }
        worst
    };
    let down_worst = worst(w.ues[0].rx[rx0..].iter().map(|p| tag_of(p)).collect(), 10_000);
    let up_worst = worst(w.lan[lan0..].iter().map(|p| tag_of(p)).collect(), 20_000);
    assert!(down_worst * 20 < 100 && up_worst * 20 < 100, "lost {} ms down, {} ms up\n{}", down_worst * 20, up_worst * 20, d);
    // hang up
    w.core.as_mut().unwrap().bearer_release(ims, bid).unwrap();
    w.route();
    assert!(w.run_until(1000, |w| w.ues[0].ue.status().bearers == 0), "{}", w.dump());
}

#[test]
fn a_full_cell_runs_extra_calls_best_effort() {
    let mut subs = String::new();
    for i in 0..3 {
        subs += &sub(&format!("31015000000020{}", i), K1, &format!("msisdn=+1214555020{}", i));
    }
    let mut w = World::new(&ims_db(&subs));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |c| c.max_voice = 2);
    for i in 0..3 {
        w.add_ue(&format!("ue{}", i), 0xB200 + i, 60.0 + 20.0 * i as f64, SoftSim::new("cell-red", &format!("31015000000020{}", i), K1));
    }
    assert!(w.run_until(10_000, |w| (0..3).all(|i| ims_of(w, i).is_some())), "{}", w.dump());
    for i in 0..3 {
        let ims = ims_of(&w, i).unwrap().ip;
        w.core.as_mut().unwrap().bearer_setup(ims, 40000, PCSCF, 30000 + i as u16).unwrap();
        w.route();
        w.run(200);
    }
    let s = w.core.as_ref().unwrap().sessions();
    let admitted = s.iter().filter(|x| x.bearers.first().is_some_and(|b| b.1)).count();
    assert_eq!(admitted, 2, "{:?}", s);
    assert!(w.core_log.iter().any(|l| l.contains("best effort")), "{}", w.dump());
}

#[test]
fn a_phone_going_to_sleep_asks_for_idle_and_is_paged_for_a_call() {
    let mut w = World::new(&ims_db(&sub("310150000000001", K1, "msisdn=+12145550142")));
    w.add_site("site-a", 1, 1, 0.0, [100, 65, 1, 2], |_| {});
    w.add_ue("ue1", 0xB001, 100.0, SoftSim::new("cell-red", "310150000000001", K1));
    assert!(w.run_until(5000, |w| w.attached(0) && ims_of(w, 0).is_some()), "{}", w.dump());
    let ims = ims_of(&w, 0).unwrap().ip;
    let tmsi = w.ues[0].ue.status().tmsi;
    assert_ne!(tmsi, 0);
    let n = w.n();
    assert!(w.ues[0].ue.go_idle(n));
    w.route();
    assert!(w.run_until(2000, |w| w.ues[0].ue.status().state == "idle"), "{}", w.dump());
    let r = w.core.as_ref().unwrap().reach("+12145550142").unwrap();
    assert!(r.idle && r.ims_ip == Some(ims), "{:?}", r);
    assert_eq!(w.ues[0].ue.status().tmsi, tmsi, "same TMSI while idle (the modem wakes on it)");
    // an INVITE for it pages the phone, which resumes and gets it
    w.down(udp(PCSCF, ims, 5060, 5060, 600, 9));
    assert!(w.run_until(3000, |w| w.ues[0].rx.iter().any(|p| tag_of(p) == 9)), "{}", w.dump());
    assert!(w.ues[0].events.iter().any(|e| matches!(e, UeEvent::Resumed { .. })));
    assert_eq!(ims_of(&w, 0).map(|i| i.ip), Some(ims), "same IMS address after the resume");
    // while a call's bearer is up the phone stays connected
    w.core.as_mut().unwrap().bearer_setup(ims, 40000, PCSCF, 30000).unwrap();
    w.route();
    assert!(w.run_until(1000, |w| w.ues[0].ue.status().bearers == 1), "{}", w.dump());
    let n = w.n();
    assert!(!w.ues[0].ue.go_idle(n));
}
