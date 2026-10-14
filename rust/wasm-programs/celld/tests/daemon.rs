//! The daemon end to end on the simulated framed medium: a core and two cell
//! sites (ideal backhaul through the byte encodings, as `ecm-cell`'s e2e
//! tests), and celld's `Daemon` over a fake computer that records what it does
//! to the tun, the routes, the resolver and the status bar.

use std::collections::VecDeque;

use celld::config::Config;
use celld::ctl::{self, Status};
use celld::{Daemon, Io, Options};
use ecm_cell::core::{ConnId, Core, CoreConfig, CoreOut, Entropy, SockAddr};
use ecm_cell::db::Db;
use ecm_cell::ip::{to_kernel, PEER_MAC};
use ecm_cell::medium::{Medium, MediumConfig, Port, RadioId};
use ecm_cell::phy::{apply, PhyCmd, PhyEvent, Profile};
use ecm_cell::simauth::{Sim, SimError, SimIdentity, SimResponse, SoftSim};
use ecm_cell::site::{Site, SiteConfig, SiteOut};
use ecm_cell::Now;

const KEY: [u8; 16] = [0x11; 16];
const SECRET: [u8; 16] = [0x5e; 16];
const IMSI: &str = "310150000000001";

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

/// A SIM slot: a card or none.
struct Slot(Option<SoftSim>);
impl Sim for Slot {
    fn identity(&self) -> Result<SimIdentity, SimError> {
        self.0.as_ref().map(|s| s.identity.clone()).ok_or(SimError::NoCard)
    }
    fn respond(&mut self, rand: &[u8; 16]) -> Result<SimResponse, SimError> {
        self.0.as_mut().ok_or(SimError::NoCard)?.respond(rand)
    }
}

/// What the fake computer saw.
#[derive(Default)]
struct Box_ {
    tun: Option<(String, [u8; 6], [u8; 4], u8)>,
    opens: u32,
    closes: u32,
    /// Frames the kernel sent on the tun, waiting for the daemon.
    kernel_out: VecDeque<Vec<u8>>,
    /// Frames the daemon gave the kernel.
    kernel_in: Vec<Vec<u8>>,
    routes: Vec<[u8; 4]>,
    dns: Vec<[u8; 4]>,
    log: Vec<String>,
    reports: Vec<(u8, String)>,
    saved: Vec<(String, String)>,
}

struct PhoneIo<'a> {
    m: &'a mut Medium,
    radio: RadioId,
    now: u64,
    b: &'a mut Box_,
}

impl Io for PhoneIo<'_> {
    fn now(&mut self) -> Now {
        Now { ms: self.now, slot: self.now }
    }
    fn phy(&mut self, cmd: PhyCmd) -> Result<(), String> {
        apply(&mut Port { medium: self.m, id: self.radio, now_us: self.now * 1000 }, [cmd]).map_err(|e| e.0)
    }
    fn phy_poll(&mut self) -> Option<PhyEvent> {
        self.m.poll(self.radio, self.now * 1000)
    }
    fn tun_open(&mut self, name: &str, mac: [u8; 6], ip: [u8; 4], prefix: u8) -> Result<(), String> {
        self.b.opens += 1;
        self.b.tun = Some((name.into(), mac, ip, prefix));
        Ok(())
    }
    fn tun_close(&mut self) {
        if self.b.tun.take().is_some() {
            self.b.closes += 1;
        }
    }
    fn tun_read(&mut self) -> Option<Vec<u8>> {
        self.b.kernel_out.pop_front()
    }
    fn tun_write(&mut self, frame: &[u8]) {
        self.b.kernel_in.push(frame.to_vec());
    }
    fn default_route(&mut self, gw: [u8; 4]) -> Result<(), String> {
        self.b.routes.push(gw);
        Ok(())
    }
    fn set_dns(&mut self, dns: [u8; 4]) -> Result<(), String> {
        self.b.dns.push(dns);
        Ok(())
    }
    fn log(&mut self, line: &str) {
        self.b.log.push(format!("{:>6} {}", self.now, line));
    }
    fn report(&mut self, bars: u8, carrier: &str) {
        self.b.reports.push((bars, carrier.into()));
    }
    fn save_setting(&mut self, key: &str, value: &str) -> Result<(), String> {
        self.b.saved.push((key.into(), value.into()));
        Ok(())
    }
}

struct SiteNode {
    site: Site,
    radio: RadioId,
    conn: Option<ConnId>,
    addr: SockAddr,
}

struct World {
    m: Medium,
    now: u64,
    core: Core,
    sites: Vec<SiteNode>,
    phone: Daemon,
    phone_radio: RadioId,
    sim: Slot,
    b: Box_,
    lan: Vec<Vec<u8>>,
    next_conn: ConnId,
}

fn db() -> String {
    let s = ecm_cell::codec::hex(&SECRET);
    format!(
        "version 1\ncarrier cell-red\npool 100.65.2.0/24\ngateway 100.65.2.1\ndns 100.65.2.1\nsite site-a {s}\nsite site-b {s}\nsub {IMSI} key={}\n",
        ecm_cell::codec::hex(&KEY)
    )
}

impl World {
    fn new(sim: Option<SoftSim>, cfg: Config) -> World {
        let mut m = Medium::new(MediumConfig { exponent: 3.5, ..Default::default() });
        let phone_radio = m.add_radio("phone", [100.0, 0.0, 1.5]);
        let core = Core::new(Db::parse(&db()).unwrap(), CoreConfig::default(), Box::new(Rng(0x1234_5678)));
        let opts = Options { profile: Profile::Lte, address: 0xB000_0001, modem: "module_1".into(), tun: "wwan0".into(), power_dbm: None, config: cfg };
        let mut w = World { m, now: 1000, core, sites: Vec::new(), phone: Daemon::new(opts, 1000), phone_radio, sim: Slot(sim), b: Box_::default(), lan: Vec::new(), next_conn: 1 };
        w.add_site("site-a", 1, 0.0, [100, 65, 0, 21]);
        w.add_site("site-b", 2, 1200.0, [100, 65, 0, 22]);
        let World { m, now, phone, sim, b, phone_radio, .. } = &mut w;
        phone.start(&mut PhoneIo { m, radio: *phone_radio, now: *now, b }, sim);
        w
    }

    fn add_site(&mut self, id: &str, cell: u32, x: f64, ip: [u8; 4]) {
        let radio = self.m.add_radio(id, [x, 0.0, 20.0]);
        let mut cfg = SiteConfig::new(id, SECRET, "cell-red", cell, 0xA000 + cell);
        cfg.channel = 1;
        let mut site = Site::new(cfg);
        site.start(Now { ms: self.now, slot: self.now });
        self.sites.push(SiteNode { site, radio, conn: None, addr: SockAddr { ip, port: 7701 } });
    }

    fn step(&mut self) {
        self.now += 5;
        let now = Now { ms: self.now, slot: self.now };
        let us = self.now * 1000;
        self.m.advance(us);
        for s in &mut self.sites {
            while let Some(e) = self.m.poll(s.radio, us) {
                s.site.on_phy(e, now);
            }
            if s.conn.is_none() && s.site.wants_core(now) {
                let c = self.next_conn;
                self.next_conn += 1;
                s.conn = Some(c);
                s.site.core_connected(now);
                self.core.on_connect(c, s.addr.ip, self.now);
            }
            s.site.tick(now);
        }
        {
            let World { m, now, phone, sim, b, phone_radio, .. } = self;
            phone.step(&mut PhoneIo { m, radio: *phone_radio, now: *now, b }, sim);
        }
        self.core.tick(self.now);
        self.route();
    }

    fn route(&mut self) {
        let now = Now { ms: self.now, slot: self.now };
        let us = self.now * 1000;
        for _ in 0..50 {
            let mut busy = false;
            for i in 0..self.sites.len() {
                let outs = self.sites[i].site.take_out();
                busy |= !outs.is_empty();
                for o in outs {
                    match o {
                        SiteOut::Phy(c) => apply(&mut Port { medium: &mut self.m, id: self.sites[i].radio, now_us: us }, [c]).unwrap(),
                        SiteOut::Send(bytes) => {
                            if let Some(c) = self.sites[i].conn {
                                self.core.on_bytes(c, &bytes, self.now);
                            }
                        }
                        SiteOut::CloseCore => {
                            if let Some(c) = self.sites[i].conn.take() {
                                self.core.on_disconnect(c, self.now);
                            }
                        }
                        SiteOut::Tunnel(d) => self.core.on_tunnel(self.sites[i].addr, &d, self.now),
                        SiteOut::Log(_) => {}
                    }
                }
            }
            let outs = self.core.take_out();
            busy |= !outs.is_empty();
            for o in outs {
                match o {
                    CoreOut::Send(c, bytes) => {
                        if let Some(s) = self.sites.iter_mut().find(|s| s.conn == Some(c)) {
                            s.site.on_bytes(&bytes, now);
                        }
                    }
                    CoreOut::Close(c) => {
                        if let Some(s) = self.sites.iter_mut().find(|s| s.conn == Some(c)) {
                            s.conn = None;
                            s.site.core_disconnected(now);
                        }
                    }
                    CoreOut::Tunnel(a, d) => {
                        if let Some(s) = self.sites.iter_mut().find(|s| s.addr == a) {
                            s.site.on_tunnel(&d, now);
                        }
                    }
                    CoreOut::ToLan(p) => self.lan.push(p),
                    _ => {}
                }
            }
            if !busy {
                return;
            }
        }
    }

    fn run_until(&mut self, ms: u64, f: impl Fn(&World) -> bool) -> bool {
        let end = self.now + ms;
        while self.now < end {
            if f(self) {
                return true;
            }
            self.step();
        }
        f(self)
    }

    fn control(&mut self, line: &str) -> String {
        let World { m, now, phone, sim, b, phone_radio, .. } = self;
        phone.control(line, &mut PhoneIo { m, radio: *phone_radio, now: *now, b }, sim)
    }

    fn status(&mut self) -> Status {
        Status::parse(&self.control("status"))
    }

    fn state(&self) -> String {
        self.phone.state()
    }

    fn dump(&self) -> String {
        format!("state {}\nlog:\n  {}\n", self.phone.state(), self.b.log.join("\n  "))
    }
}

fn soft() -> Option<SoftSim> {
    Some(SoftSim::new("cell-red", IMSI, KEY))
}

/// An IPv4 packet (protocol 17) from `src` to `dst`.
fn pkt(src: [u8; 4], dst: [u8; 4], len: usize) -> Vec<u8> {
    let mut p = vec![0u8; len.max(28)];
    p[0] = 0x45;
    let n = p.len() as u16;
    p[2..4].copy_from_slice(&n.to_be_bytes());
    p[8] = 64;
    p[9] = 17;
    p[12..16].copy_from_slice(&src);
    p[16..20].copy_from_slice(&dst);
    p
}

const PHONE_IP: [u8; 4] = [100, 65, 2, 2];
const SERVER: [u8; 4] = [100, 65, 0, 10];

#[test]
fn attaches_creates_wwan0_with_a_default_route_and_carries_packets_both_ways() {
    let mut w = World::new(soft(), Config::default());
    assert!(w.run_until(10_000, |w| w.state() == "connected" && w.b.tun.is_some()), "{}", w.dump());
    let (name, mac, ip, prefix) = w.b.tun.clone().unwrap();
    assert_eq!((name.as_str(), ip, prefix), ("wwan0", PHONE_IP, 24));
    assert_eq!(mac, w.phone.mac());
    assert_eq!(w.b.routes, vec![[100, 65, 2, 1]], "default via the core's gateway");
    assert_eq!(w.b.dns, vec![[100, 65, 2, 1]], "the core's DNS server is the resolver");
    assert!(w.b.log.iter().any(|l| l.contains("wwan0 up 100.65.2.2/24 default via 100.65.2.1, dns 100.65.2.1")), "{}", w.dump());

    // ARP for the gateway: answered by celld (proxy ARP on the point-to-point link).
    let mut arp = vec![0u8; 42];
    arp[0..6].copy_from_slice(&[0xff; 6]);
    arp[6..12].copy_from_slice(&mac);
    arp[12..14].copy_from_slice(&[0x08, 0x06]);
    arp[14..22].copy_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1]);
    arp[22..28].copy_from_slice(&mac);
    arp[28..32].copy_from_slice(&PHONE_IP);
    arp[38..42].copy_from_slice(&[100, 65, 2, 1]);
    w.b.kernel_out.push_back(arp);
    w.step();
    let reply = w.b.kernel_in.iter().find(|f| f[12..14] == [0x08, 0x06]).expect("an ARP reply");
    assert_eq!(&reply[22..28], &PEER_MAC);

    // Uplink: the kernel routes a packet out of wwan0; it reaches the core's LAN.
    let mut frame = vec![0u8; 14];
    frame[0..6].copy_from_slice(&PEER_MAC);
    frame[6..12].copy_from_slice(&mac);
    frame[12..14].copy_from_slice(&[0x08, 0x00]);
    frame.extend_from_slice(&pkt(PHONE_IP, SERVER, 300));
    w.b.kernel_out.push_back(frame);
    assert!(w.run_until(2000, |w| w.lan.iter().any(|p| p.len() == 300 && p[12..16] == PHONE_IP)), "{}", w.dump());

    // Downlink: a LAN host answers; the frame arrives on wwan0 addressed to its MAC.
    let before = w.b.kernel_in.len();
    let now = w.now;
    w.core.on_lan_packet(&pkt(SERVER, PHONE_IP, 200), now);
    w.route();
    assert!(w.run_until(2000, |w| w.b.kernel_in.len() > before), "{}", w.dump());
    let down = w.b.kernel_in.last().unwrap();
    assert_eq!(down, &to_kernel(&pkt(SERVER, PHONE_IP, 200), mac));

    let s = w.status();
    assert_eq!((s.state.as_str(), s.carrier.as_str(), s.cell, s.ip.as_str()), ("connected", "cell-red", Some(1), "100.65.2.2/24"), "{s:?}");
    assert_eq!((s.sim_carrier.as_str(), s.imsi.as_str(), s.modem.as_str(), s.profile.as_str()), ("cell-red", IMSI, "module_1", "lte"));
    assert!(s.bars >= 3 && s.cqi.is_some() && s.rssi.is_some() && s.sinr.is_some(), "{s:?}");
    assert_eq!((s.tx_packets, s.rx_packets, s.tx_bytes, s.rx_bytes), (1, 1, 300, 200), "{s:?}");
    assert_eq!(s.gateway, "100.65.2.1");
    // The status bar got the bars and the carrier.
    assert!(w.b.reports.iter().any(|(b, c)| *b >= 3 && c == "cell-red"), "{:?}", w.b.reports);
    // Both cells were heard, the serving one marked.
    let cells = ctl::parse_cells(&w.control("cells"));
    assert!(cells.iter().any(|c| c.cell == 1 && c.serving && c.note.is_empty()), "{cells:?}");
}

#[test]
fn survives_a_handover_with_the_same_address() {
    let mut w = World::new(soft(), Config::default());
    assert!(w.run_until(10_000, |w| w.state() == "connected"), "{}", w.dump());
    let opens = w.b.opens;
    let r = w.phone_radio;
    let now_us = w.now * 1000;
    w.m.move_to(r, [1100.0, 0.0, 1.5], now_us, 15_000_000);
    assert!(w.run_until(25_000, |w| w.phone.status(w.now).handovers >= 1 && w.state() == "connected"), "{}", w.dump());
    let s = w.status();
    assert_eq!((s.cell, s.ip.as_str(), s.handovers), (Some(2), "100.65.2.2/24", 1), "{s:?}\n{}", w.dump());
    assert_eq!(w.b.opens, opens, "wwan0 is kept across the handover");
    assert!(w.b.log.iter().any(|l| l.contains("handover cell 1 -> 2 (channel 1)")), "{}", w.dump());
}

#[test]
fn down_removes_wwan0_and_up_brings_the_same_address_back() {
    let mut w = World::new(soft(), Config::default());
    assert!(w.run_until(10_000, |w| w.state() == "connected"), "{}", w.dump());
    assert_eq!(w.control("down"), "ok detaching\n");
    assert!(w.run_until(3000, |w| w.b.tun.is_none()), "{}", w.dump());
    assert_eq!(w.state(), "down");
    w.run_until(2000, |_| false);
    assert_eq!(w.state(), "down", "stays down (no re-attach on its own)");
    assert!(w.b.reports.last().is_some_and(|(b, c)| *b == 0 && c.is_empty()), "{:?}", w.b.reports);
    assert!(w.control("up").starts_with("ok "));
    assert!(w.run_until(10_000, |w| w.state() == "connected" && w.b.tun.is_some()), "{}", w.dump());
    assert_eq!(w.b.tun.as_ref().unwrap().2, PHONE_IP, "the address is sticky at the core");
    assert_eq!(w.status().attaches, 2);
    assert!(w.control("up").starts_with("ok already"));
    assert!(w.control("fly").starts_with("error unknown command"));
}

#[test]
fn no_sim_waits_for_a_card_and_a_carrier_lock_refuses_another_carrier() {
    let mut w = World::new(None, Config::default());
    w.run_until(1000, |_| false);
    let s = w.status();
    assert_eq!(s.state, "no_sim");
    assert!(s.message.contains("no SIM card"), "{s:?}");
    assert_eq!(w.control("cells"), "none
", "an empty list is still an answer (never an empty datagram)");
    assert!(w.b.tun.is_none());
    // The card goes in: within the SIM check period it attaches.
    w.sim = Slot(soft());
    assert!(w.run_until(12_000, |w| w.state() == "connected"), "{}", w.dump());
    // Taken out again: the session is dropped.
    w.sim = Slot(None);
    assert!(w.run_until(5000, |w| w.state() == "no_sim" && w.b.tun.is_none()), "{}", w.dump());

    let lock = Config { carrier: Some("cell-blue".into()), ..Config::default() };
    let mut w = World::new(soft(), lock);
    w.run_until(3000, |_| false);
    let s = w.status();
    assert_eq!(s.state, "locked", "{s:?}");
    assert!(s.message.contains("locked to cell-blue"), "{s:?}");
    assert!(w.b.tun.is_none());
}

#[test]
fn autoconnect_off_waits_for_up_and_roaming_is_saved() {
    let mut w = World::new(soft(), Config { autoconnect: false, ..Config::default() });
    w.run_until(2000, |_| false);
    assert_eq!(w.state(), "down");
    assert_eq!(w.control("roaming on"), "ok roaming on\n");
    assert_eq!(w.b.saved, vec![("roaming".to_string(), "true".to_string())]);
    assert!(w.status().roaming);
    w.control("up");
    assert!(w.run_until(10_000, |w| w.state() == "connected"), "{}", w.dump());
}
