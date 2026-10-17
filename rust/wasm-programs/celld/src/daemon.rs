//! The daemon: the `ecm-cell` UE machine with its driver around it. It owns
//! no I/O itself; [`Io`] is the computer (the modem's framed radio, the tun,
//! netlink, the status bar, the config file) and `Sim` is the SIM's answer
//! path. The WASI program (`host.rs`) and the simulator's phone agent supply
//! them, so both run exactly this code.
//!
//! One [`Daemon::step`] (every 5 ms): radio records to the UE (beacons are
//! also kept for `cells`), frames the kernel sent on `wwan0` to the UE (ARP
//! answered here), `tick`, then the UE's outputs to the radio, the tun and
//! the event handling: on attach `wwan0` is created (or kept, the address is
//! sticky at the core) with a default route via the core's gateway and its
//! DNS server as the resolver.

use std::collections::BTreeMap;

use ecm_cell::air::{ImsInfo, Plain, SysInfo};
use ecm_cell::backhaul::IpConfig;
use ecm_cell::codec::fmt_ip;
use ecm_cell::ip::{from_kernel, to_kernel, TunFrame};
use ecm_cell::phy::{PhyCmd, PhyEvent, Profile};
use ecm_cell::simauth::{Sim, SimIdentity};
use ecm_cell::ue::{Ue, UeConfig, UeEvent, UeOut};
use ecm_cell::Now;

use crate::config::Config;
use crate::ctl::{self, CellRow, Status};

/// What the daemon needs from the computer.
pub trait Io {
    fn now(&mut self) -> Now;
    fn phy(&mut self, cmd: PhyCmd) -> Result<(), String>;
    /// The next radio record, without waiting.
    fn phy_poll(&mut self) -> Option<PhyEvent>;
    /// Create the tun interface `name` (with this address) for this daemon.
    fn tun_open(&mut self, name: &str, mac: [u8; 6], ip: [u8; 4], prefix: u8) -> Result<(), String>;
    /// Remove the tun interface (its routes go with it).
    fn tun_close(&mut self);
    /// The next frame the kernel sent on the tun, without waiting.
    fn tun_read(&mut self) -> Option<Vec<u8>>;
    fn tun_write(&mut self, frame: &[u8]);
    /// `ip route add default via <gateway>` (the kernel picks the tun: the gateway is in its subnet).
    fn default_route(&mut self, gateway: [u8; 4]) -> Result<(), String>;
    /// Make `dns` the resolver.
    fn set_dns(&mut self, dns: [u8; 4]) -> Result<(), String>;
    fn log(&mut self, line: &str);
    /// The phone's status bar: bars 0-4 and the carrier ("" when on no cell).
    fn report(&mut self, _bars: u8, _carrier: &str) {}
    /// Persist `key = value` in /etc/celld.toml.
    fn save_setting(&mut self, _key: &str, _value: &str) -> Result<(), String> {
        Ok(())
    }
    /// The IMS bearer's tun (`ims0`, docs/TELEPHONY.md): create it with this address.
    fn ims_open(&mut self, _mac: [u8; 6], _ip: [u8; 4], _prefix: u8) -> Result<(), String> {
        Err("this computer has no IMS tun".into())
    }
    fn ims_close(&mut self) {}
    fn ims_read(&mut self) -> Option<Vec<u8>> {
        None
    }
    fn ims_write(&mut self, _frame: &[u8]) {}
    /// `ip route add <pcscf>/32 dev ims0`: the IMS core goes over the IMS bearer.
    fn ims_route(&mut self, _pcscf: [u8; 4]) -> Result<(), String> {
        Ok(())
    }
    /// The phone's computer is about to sleep (the hotbar): go idle so pages wake it.
    fn sleeping(&mut self) -> bool {
        false
    }
    /// Hold the phone's computer awake for `ms` (`/dev/uictl wakelock <ms>`; 0 releases).
    fn wakelock(&mut self, _ms: u64) {}
    /// Hand the modem its paging identity (`wake tmsi N`): while the computer
    /// sleeps, a page for it wakes the phone. `None`: `wake off`.
    fn wake_filter(&mut self, _tmsi: Option<u32>) {}
}

/// How the daemon was started.
#[derive(Clone, Debug)]
pub struct Options {
    pub profile: Profile,
    /// The modem radio's frame address.
    pub address: u32,
    /// The modem's attachment name (for status).
    pub modem: String,
    /// The tun interface to create.
    pub tun: String,
    /// Transmit power, dBm (None = the radio's UE default).
    pub power_dbm: Option<f64>,
    pub config: Config,
}

#[derive(Clone)]
struct Heard {
    sib: SysInfo,
    rssi: f64,
    sinr: f64,
    cqi: u8,
    last_ms: u64,
}

#[derive(Clone, Debug, PartialEq)]
enum Hold {
    /// Running the UE.
    No,
    NoSim,
    /// Carrier lock: the SIM is for another carrier.
    Locked(String),
    /// `cell down` (or autoconnect off).
    Down,
}

/// How often the SIM is looked at again (taken out, swapped, put back).
const SIM_CHECK_MS: u64 = 2000;
/// See `UeConfig::tx_lead_us`.
const TX_LEAD_US: u64 = 5000;
/// The status bar is refreshed this often even when nothing changed.
const REPORT_MS: u64 = 5000;
/// Cells are listed this long after their last beacon.
const HEARD_MS: u64 = 10_000;
/// Quiet time before a sleeping phone's resumed session goes idle again (ms).
const IDLE_QUIET_MS: u64 = 1500;
/// Each renewal of celld's wake lock while asleep and connected (ms; renewed at half).
const WAKELOCK_MS: u64 = 3000;
/// How often celld renews that lock (ms of its own clock).
const WAKELOCK_RENEW_MS: u64 = 1000;

pub struct Daemon {
    ue: Ue,
    opts: Options,
    mac: [u8; 6],
    tun: Option<IpConfig>,
    identity: Option<SimIdentity>,
    hold: Hold,
    admin_down: bool,
    next_sim_check: u64,
    heard: BTreeMap<u32, Heard>,
    no_service: Option<String>,
    handovers: u32,
    attaches: u32,
    tx_bytes: u64,
    rx_bytes: u64,
    tx_packets: u64,
    rx_packets: u64,
    cause: String,
    started_ms: u64,
    reported: Option<(u8, String, u64)>,
    phy_errors: u32,
    /// The IMS bearer (VoLTE, SMS over IMS) when the attach gave one.
    ims: Option<ImsInfo>,
    ims_mac: [u8; 6],
    ims_up: bool,
    /// The wake filter the modem holds.
    wake: Option<u32>,
    /// Asked for idle because the computer goes to sleep.
    slept: bool,
    /// Packets so far and when the count last moved (going idle again after a page, above).
    last_packets: u64,
    last_traffic_ms: u64,
    next_idle_try: u64,
    /// Our wake lock while asleep and connected (above).
    locked: bool,
    lock_renew_at: u64,
}

/// A UE event as a log line (the simulator's scenarios match these).
pub fn describe_event(e: &UeEvent) -> String {
    match e {
        UeEvent::Searching => "searching".into(),
        UeEvent::NoService { heard } => {
            if heard.is_empty() {
                "no service (no cell heard)".into()
            } else {
                format!("no service (heard {})", heard.join(", "))
            }
        }
        UeEvent::Attached { cell, ip } => format!("attached cell {} ip {}/{} gw {}", cell, fmt_ip(ip.ip), ip.prefix, fmt_ip(ip.gateway)),
        UeEvent::Resumed { cell } => format!("resumed cell {}", cell),
        UeEvent::Handover { from, to, channel } => format!("handover cell {} -> {} (channel {})", from, to, channel),
        UeEvent::Released { cause } => format!("released: {}", cause),
        UeEvent::Rejected { cause } => format!("rejected: {}", cause),
        UeEvent::NetworkAuthFailed { cell } => format!("cell {} failed network authentication", cell),
        UeEvent::SimError => "SIM error".into(),
        UeEvent::Idle => "idle".into(),
        UeEvent::Detached => "detached".into(),
        UeEvent::Ims(i) => format!("ims bearer {} number {}", fmt_ip(i.ip), if i.msisdn.is_empty() { "-" } else { &i.msisdn }),
        UeEvent::Bearer { bearer, up } => format!("voice bearer {} {}", bearer.id, if *up { "up" } else { "released" }),
    }
}

impl Daemon {
    pub fn new(opts: Options, now_ms: u64) -> Daemon {
        let mut cfg = UeConfig::new(opts.profile, opts.address);
        cfg.channels = opts.config.channels.clone();
        cfg.power_dbm = opts.power_dbm;
        // The driver reads the radio's slot clock now and then and counts forward with its own
        // clock, so it lags the radio by a millisecond or two: schedule a little ahead (a frame
        // named for a slot the radio has passed is refused, and a resent one can overtake).
        cfg.tx_lead_us = TX_LEAD_US;
        let a = opts.address.to_be_bytes();
        Daemon {
            ue: Ue::new(cfg),
            mac: [0x02, 0xce, a[0], a[1], a[2], a[3]],
            tun: None,
            identity: None,
            hold: Hold::NoSim,
            admin_down: !opts.config.autoconnect,
            next_sim_check: 0,
            heard: BTreeMap::new(),
            no_service: None,
            handovers: 0,
            attaches: 0,
            tx_bytes: 0,
            rx_bytes: 0,
            tx_packets: 0,
            rx_packets: 0,
            cause: String::new(),
            started_ms: now_ms,
            reported: None,
            phy_errors: 0,
            ims: None,
            ims_mac: [0; 6],
            ims_up: false,
            wake: None,
            slept: false,
            last_packets: 0,
            last_traffic_ms: 0,
            next_idle_try: 0,
            locked: false,
            lock_renew_at: 0,
            opts,
        }
    }

    pub fn options(&self) -> &Options {
        &self.opts
    }

    /// The MAC `wwan0` gets (from the radio's address).
    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }

    /// Read the SIM and, unless held (no SIM, carrier lock, `cell down`), start the UE.
    pub fn start(&mut self, io: &mut dyn Io, sim: &mut dyn Sim) {
        let now = io.now();
        self.next_sim_check = now.ms + SIM_CHECK_MS;
        match sim.identity() {
            Err(_) => {
                if self.hold != Hold::NoSim || self.identity.is_some() {
                    io.log("no SIM card");
                }
                self.identity = None;
                self.hold = Hold::NoSim;
                return;
            }
            Ok(id) => {
                if let Some(lock) = self.opts.config.carrier.clone() {
                    if lock != id.carrier {
                        if self.hold != Hold::Locked(lock.clone()) {
                            io.log(&format!("SIM is for {}, this phone is locked to {}", id.carrier, lock));
                        }
                        self.identity = Some(id);
                        self.hold = Hold::Locked(lock);
                        return;
                    }
                }
                io.log(&format!("SIM {} IMSI {}", id.carrier, id.imsi));
                self.identity = Some(id);
            }
        }
        if self.admin_down {
            self.hold = Hold::Down;
            return;
        }
        self.hold = Hold::No;
        self.cause.clear();
        self.ue.start(sim, now);
        self.drain(io);
    }

    /// One pass of the driver loop.
    pub fn step(&mut self, io: &mut dyn Io, sim: &mut dyn Sim) {
        let now = io.now();
        while let Some(e) = io.phy_poll() {
            self.peek(&e, now.ms);
            self.ue.on_phy(e, sim, now);
        }
        if let Some(ipc) = self.tun {
            for _ in 0..64 {
                let Some(frame) = io.tun_read() else { break };
                match from_kernel(&frame, ipc.ip) {
                    TunFrame::ArpReply(r) => io.tun_write(&r),
                    TunFrame::Ipv4(p) => {
                        let n = p.len() as u64;
                        if self.ue.status().state == "idle" {
                            io.log(&format!("traffic resumes the session: {}", packet_desc(p)));
                        }
                        if self.ue.send_ip(p.to_vec(), now) {
                            self.tx_bytes += n;
                            self.tx_packets += 1;
                        }
                    }
                    TunFrame::Other => {}
                }
            }
        }
        if self.ims_up {
            let ims_ip = self.ims.as_ref().map(|i| i.ip).unwrap_or_default();
            for _ in 0..64 {
                let Some(frame) = io.ims_read() else { break };
                match from_kernel(&frame, ims_ip) {
                    TunFrame::ArpReply(r) => io.ims_write(&r),
                    TunFrame::Ipv4(p) => {
                        let n = p.len() as u64;
                        if self.ue.status().state == "idle" {
                            io.log(&format!("traffic resumes the session: {}", packet_desc(p)));
                        }
                        if self.ue.send_ip(p.to_vec(), now) {
                            self.tx_bytes += n;
                            self.tx_packets += 1;
                        }
                    }
                    TunFrame::Other => {}
                }
            }
        }
        if now.ms >= self.next_sim_check {
            self.check_sim(io, sim, now);
        }
        // the hotbar: the computer sleeps soon; idle now so the network pages us. A page wakes
        // the session again (an incoming call, a text); once the network has been quiet for
        // IDLE_QUIET_MS go idle again, or the next call wouldn't be paged. A call's voice bearer
        // keeps the session up (go_idle refuses while one is set up).
        let sleeping = io.sleeping();
        let packets = self.tx_packets + self.rx_packets;
        if packets != self.last_packets {
            self.last_packets = packets;
            self.last_traffic_ms = now.ms;
        }
        let connected = self.ue.status().state == "connected";
        if sleeping && !self.slept {
            if self.ue.go_idle(now) {
                self.slept = true;
                self.next_idle_try = now.ms + IDLE_QUIET_MS;
                io.log("going idle: the phone sleeps (a page wakes it)");
            }
        } else if sleeping
            && connected
            && now.ms >= self.next_idle_try
            && now.ms.saturating_sub(self.last_traffic_ms) >= IDLE_QUIET_MS
        {
            self.next_idle_try = now.ms + IDLE_QUIET_MS;
            if self.ue.go_idle(now) {
                io.log("idle again: quiet since the session resumed");
            }
        } else if !sleeping {
            self.slept = false;
        }
        // A sleeping phone whose session is up (traffic resumed it, a page woke it, a call) can't
        // see its downlink while its computer is frozen: the modem wakes it only for a page. Hold
        // the computer awake until the session is idle again (a call's voice bearer keeps it up,
        // and telephonyd holds its own lock for the call).
        if sleeping && connected && now.ms >= self.lock_renew_at {
            if !self.locked {
                io.log("asleep with the session up: holding the computer awake until idle");
            }
            io.wakelock(WAKELOCK_MS);
            self.lock_renew_at = now.ms + WAKELOCK_RENEW_MS;
            self.locked = true;
        }
        self.ue.tick(now);
        self.drain(io);
        self.sync_wake(io);
        // Let the computer sleep again only now, with the modem's wake filter armed for the idle
        // session (released before sync_wake, it could freeze in between and miss its pages).
        if self.locked {
            let st = self.ue.status().state;
            let armed = st != "idle" || self.wake.is_some();
            if armed && !(sleeping && st == "connected") {
                io.wakelock(0);
                self.locked = false;
                self.lock_renew_at = 0;
                io.log(&format!("wake lock released ({st})"));
            }
        }
        self.report(io, now.ms);
    }

    /// The SIM taken out, swapped or put back.
    fn check_sim(&mut self, io: &mut dyn Io, sim: &mut dyn Sim, now: Now) {
        self.next_sim_check = now.ms + SIM_CHECK_MS;
        let id = sim.identity().ok();
        if id == self.identity && self.hold != Hold::NoSim {
            return;
        }
        if id.is_none() && self.hold == Hold::NoSim {
            return;
        }
        if self.hold == Hold::No {
            // The card the session was made with is gone: drop it, as a modem does.
            self.ue.detach(now);
            self.drain(io);
            self.down_tun(io);
        }
        self.start(io, sim);
    }

    fn peek(&mut self, e: &PhyEvent, now_ms: u64) {
        if let PhyEvent::Rx(f) = e {
            if f.is_beacon() {
                if let Ok(Plain::SysInfo(sib)) = Plain::decode(&f.payload) {
                    self.heard.insert(sib.cell_id, Heard { sib, rssi: f.rssi_dbm, sinr: f.sinr_db, cqi: f.cqi, last_ms: now_ms });
                }
            }
        }
        if self.heard.len() > 32 {
            self.heard.retain(|_, h| now_ms.saturating_sub(h.last_ms) < HEARD_MS);
        }
    }

    fn drain(&mut self, io: &mut dyn Io) {
        for o in self.ue.take_out() {
            match o {
                UeOut::Phy(c) => {
                    if let Err(e) = io.phy(c) {
                        self.phy_errors += 1;
                        if self.phy_errors <= 5 || self.phy_errors % 500 == 0 {
                            io.log(&format!("radio: {e} ({} errors)", self.phy_errors));
                        }
                    }
                }
                UeOut::Ip(p) => {
                    let to_ims = self.ims_up && self.ims.as_ref().is_some_and(|i| p.len() >= 20 && p[16..20] == i.ip);
                    if to_ims {
                        self.rx_bytes += p.len() as u64;
                        self.rx_packets += 1;
                        io.ims_write(&to_kernel(&p, self.ims_mac));
                    } else if self.tun.is_some() {
                        self.rx_bytes += p.len() as u64;
                        self.rx_packets += 1;
                        io.tun_write(&to_kernel(&p, self.mac));
                    }
                }
                UeOut::Event(e) => {
                    io.log(&describe_event(&e));
                    self.on_event(e, io);
                }
                UeOut::Log(l) => io.log(&l),
            }
        }
    }

    fn on_event(&mut self, e: UeEvent, io: &mut dyn Io) {
        match e {
            UeEvent::Attached { ip, .. } => {
                self.attaches += 1;
                self.cause.clear();
                self.no_service = None;
                self.bring_up(io, ip);
            }
            UeEvent::Handover { .. } => self.handovers += 1,
            UeEvent::Released { cause } | UeEvent::Rejected { cause } => {
                self.cause = cause.name().to_string();
                if cause.is_permanent() {
                    self.down_tun(io);
                }
            }
            UeEvent::NetworkAuthFailed { cell } => self.cause = format!("cell {cell} failed network authentication"),
            UeEvent::NoService { heard } => {
                self.no_service = Some(if heard.is_empty() { "no cell heard".into() } else { format!("heard {}", heard.join(", ")) });
            }
            UeEvent::Detached => self.down_tun(io),
            UeEvent::SimError => {
                self.hold = Hold::NoSim;
                self.identity = None;
                self.down_tun(io);
            }
            UeEvent::Ims(info) => self.bring_up_ims(io, info),
            UeEvent::Bearer { bearer, up } => {
                io.log(&format!("voice bearer {} {} (udp {} <-> {}:{})", bearer.id, if up { "up" } else { "released" }, bearer.ue_port, fmt_ip(bearer.remote), bearer.remote_port));
            }
            UeEvent::Searching | UeEvent::Resumed { .. } | UeEvent::Idle => {}
        }
    }

    /// The modem watches for pages while the phone is idle, not otherwise.
    fn sync_wake(&mut self, io: &mut dyn Io) {
        let st = self.ue.status();
        let want = if st.state == "idle" && st.tmsi != 0 { Some(st.tmsi) } else { None };
        if want != self.wake {
            self.wake = want;
            io.wake_filter(want);
        }
    }

    fn bring_up_ims(&mut self, io: &mut dyn Io, info: ImsInfo) {
        if self.ims_up && self.ims.as_ref().is_some_and(|i| i.ip == info.ip && i.prefix == info.prefix) {
            self.ims = Some(info);
            return;
        }
        if self.ims_up {
            io.ims_close();
            self.ims_up = false;
        }
        self.ims_mac = [0x02, 0xce, 0x1a, info.ip[1], info.ip[2], info.ip[3]];
        if let Err(e) = io.ims_open(self.ims_mac, info.ip, info.prefix) {
            io.log(&format!("ims0: can't create the interface: {e}"));
            self.ims = Some(info);
            return;
        }
        self.ims_up = true;
        let route = match io.ims_route(info.pcscf) {
            Ok(()) => String::new(),
            Err(e) => format!(" (route error {e})"),
        };
        io.log(&format!(
            "ims0 up {}/{}, P-CSCF {}, domain {}, number {}{}",
            fmt_ip(info.ip),
            info.prefix,
            fmt_ip(info.pcscf),
            info.domain,
            if info.msisdn.is_empty() { "-" } else { &info.msisdn },
            route
        ));
        self.ims = Some(info);
    }

    fn bring_up(&mut self, io: &mut dyn Io, ip: IpConfig) {
        if let Some(cur) = self.tun {
            if cur.ip == ip.ip && cur.prefix == ip.prefix {
                if cur.gateway != ip.gateway {
                    let _ = io.default_route(ip.gateway);
                }
                self.tun = Some(ip);
                return;
            }
            io.tun_close();
            self.tun = None;
        }
        let name = self.opts.tun.clone();
        if let Err(e) = io.tun_open(&name, self.mac, ip.ip, ip.prefix) {
            io.log(&format!("{name}: can't create the interface: {e}"));
            return;
        }
        self.tun = Some(ip);
        let route = match io.default_route(ip.gateway) {
            Ok(()) => String::new(),
            Err(e) => format!(" (route error {e})"),
        };
        let dns = if ip.dns != [0; 4] {
            match io.set_dns(ip.dns) {
                Ok(()) => format!(", dns {}", fmt_ip(ip.dns)),
                Err(e) => format!(", dns {} not set: {e}", fmt_ip(ip.dns)),
            }
        } else {
            String::new()
        };
        io.log(&format!("{} up {}/{} default via {}{}{}", name, fmt_ip(ip.ip), ip.prefix, fmt_ip(ip.gateway), dns, route));
    }

    fn down_tun(&mut self, io: &mut dyn Io) {
        if self.ims_up {
            io.ims_close();
            self.ims_up = false;
            io.log("ims0 down");
        }
        self.ims = None;
        if self.tun.take().is_some() {
            io.tun_close();
            io.log(&format!("{} down", self.opts.tun));
        }
    }

    fn report(&mut self, io: &mut dyn Io, now_ms: u64) {
        let s = self.status(now_ms);
        let bars = s.bars;
        let carrier = s.carrier.clone();
        let due = match &self.reported {
            None => true,
            Some((b, c, at)) => *b != bars || *c != carrier || now_ms.saturating_sub(*at) >= REPORT_MS,
        };
        if due {
            io.report(bars, &carrier);
            self.reported = Some((bars, carrier, now_ms));
        }
    }

    // ------------------------------------------------------------ status and control

    pub fn state(&self) -> String {
        let ue = self.ue.status().state;
        match &self.hold {
            Hold::NoSim => "no_sim".into(),
            Hold::Locked(_) => "locked".into(),
            Hold::Down => "down".into(),
            Hold::No if ue == "off" => "down".into(),
            Hold::No => ue.into(),
        }
    }

    pub fn status(&self, now_ms: u64) -> Status {
        let st = self.ue.status();
        let state = self.state();
        let on_cell = matches!(state.as_str(), "connected" | "idle" | "handover");
        let sim_carrier = self.identity.as_ref().map(|i| i.carrier.clone()).unwrap_or_default();
        let cell = st.cell.filter(|_| self.hold == Hold::No);
        let message = match state.as_str() {
            "no_sim" => "no SIM card: put one in the phone's SIM tray (or click it into the modem's bay)".to_string(),
            "locked" => match &self.hold {
                Hold::Locked(l) => format!("this phone is locked to {l}; the SIM is for {sim_carrier}"),
                _ => String::new(),
            },
            "down" => "switched off (cell up to attach)".to_string(),
            "searching" => match &self.no_service {
                Some(h) => format!("no service ({h}), looking for a {sim_carrier} cell"),
                None => format!("looking for a {sim_carrier} cell"),
            },
            "waiting" => format!("waiting before trying again ({})", if self.cause.is_empty() { "-" } else { &self.cause }),
            "attaching" => format!("attaching to cell {}", cell.map(|c| c.to_string()).unwrap_or_else(|| "-".into())),
            "connected" => format!("on {} cell {}", sim_carrier, cell.unwrap_or(0)),
            "handover" => format!("handing over to cell {}", cell.unwrap_or(0)),
            "idle" => format!("idle on cell {} (resumes on traffic)", cell.unwrap_or(0)),
            "detaching" => "detaching".to_string(),
            "rejected" => format!("rejected: {} (cell up to try again)", if self.cause.is_empty() { "-" } else { &self.cause }),
            _ => String::new(),
        };
        let ip = self.tun;
        Status {
            message,
            modem: self.opts.modem.clone(),
            profile: self.opts.profile.name().into(),
            imsi: self.identity.as_ref().map(|i| i.imsi.clone()).unwrap_or_default(),
            carrier: if on_cell { sim_carrier.clone() } else { String::new() },
            sim_carrier,
            cell,
            channel: st.channel,
            rssi: st.rssi_dbm,
            sinr: st.sinr_db,
            cqi: cell.and_then(|c| self.heard.get(&c)).map(|h| h.cqi),
            bars: ctl::bars(st.rssi_dbm, on_cell),
            ip: ip.map(|i| format!("{}/{}", fmt_ip(i.ip), i.prefix)).unwrap_or_default(),
            gateway: ip.map(|i| fmt_ip(i.gateway)).unwrap_or_default(),
            dns: ip.filter(|i| i.dns != [0; 4]).map(|i| fmt_ip(i.dns)).unwrap_or_default(),
            handovers: self.handovers,
            attaches: self.attaches,
            tx_bytes: self.tx_bytes,
            rx_bytes: self.rx_bytes,
            tx_packets: self.tx_packets,
            rx_packets: self.rx_packets,
            roaming: self.opts.config.roaming,
            msisdn: self.ims.as_ref().map(|i| i.msisdn.clone()).unwrap_or_default(),
            ims_ip: self.ims.as_ref().filter(|_| self.ims_up).map(|i| format!("{}/{}", fmt_ip(i.ip), i.prefix)).unwrap_or_default(),
            pcscf: self.ims.as_ref().map(|i| fmt_ip(i.pcscf)).unwrap_or_default(),
            ims_domain: self.ims.as_ref().map(|i| i.domain.clone()).unwrap_or_default(),
            bearers: st.bearers as u32,
            tmsi: st.tmsi,
            cause: self.cause.clone(),
            uptime_s: now_ms.saturating_sub(self.started_ms) / 1000,
            state,
        }
    }

    /// Cells heard in the last 10 s, strongest first.
    pub fn cells(&self, now_ms: u64) -> Vec<CellRow> {
        let st = self.ue.status();
        let mine = self.identity.as_ref().map(|i| i.carrier.as_str()).unwrap_or("");
        let mut rows: Vec<CellRow> = self
            .heard
            .values()
            .filter(|h| now_ms.saturating_sub(h.last_ms) < HEARD_MS)
            .map(|h| CellRow {
                cell: h.sib.cell_id,
                carrier: h.sib.carrier.clone(),
                profile: Profile::from_code(h.sib.profile).map(|p| p.name()).unwrap_or("?").into(),
                channel: h.sib.channel,
                rssi: h.rssi,
                sinr: h.sinr,
                age_ms: now_ms.saturating_sub(h.last_ms),
                serving: st.cell == Some(h.sib.cell_id) && self.hold == Hold::No,
                note: if h.sib.carrier != mine {
                    "other carrier".into()
                } else if !h.sib.core_up() {
                    "core down".into()
                } else if h.sib.barred() {
                    "barred".into()
                } else {
                    String::new()
                },
            })
            .collect();
        rows.sort_by(|a, b| b.rssi.partial_cmp(&a.rssi).unwrap_or(std::cmp::Ordering::Equal));
        rows
    }

    /// A control-socket request; the answer.
    pub fn control(&mut self, line: &str, io: &mut dyn Io, sim: &mut dyn Sim) -> String {
        let now = io.now();
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.as_slice() {
            [] | ["status"] => self.status(now.ms).format(),
            ["cells"] | ["scan"] => {
                // Never an empty datagram: a client can't tell that from no answer.
                let rows: String = self.cells(now.ms).iter().map(|r| r.format() + "\n").collect();
                if rows.is_empty() {
                    "none\n".into()
                } else {
                    rows
                }
            }
            ["up"] => {
                self.admin_down = false;
                let state = self.state();
                // On a cell (or on the way): nothing to do. Otherwise start again at once,
                // also out of a back-off (as switching airplane mode off does).
                if !matches!(state.as_str(), "connected" | "idle" | "handover" | "attaching" | "detaching") {
                    io.log("up (cell up)");
                    self.hold = Hold::No;
                    self.start(io, sim);
                    format!("ok {}\n", self.state())
                } else {
                    format!("ok already {}\n", state)
                }
            }
            ["down"] => {
                self.admin_down = true;
                io.log("down (cell down)");
                if self.hold == Hold::No {
                    self.ue.detach(now);
                    self.drain(io);
                }
                if self.hold == Hold::No && self.ue.status().state == "off" {
                    self.down_tun(io);
                }
                if self.hold == Hold::No {
                    self.hold = Hold::Down;
                }
                "ok detaching\n".into()
            }
            ["idle"] => {
                if self.ue.go_idle(now) {
                    self.drain(io);
                    "ok idle\n".into()
                } else {
                    format!("error not now ({})\n", self.state())
                }
            }
            ["roaming", v @ ("on" | "off")] => {
                let on = *v == "on";
                self.opts.config.roaming = on;
                match io.save_setting("roaming", if on { "true" } else { "false" }) {
                    Ok(()) => format!("ok roaming {v}\n"),
                    Err(e) => format!("ok roaming {v} (not saved: {e})\n"),
                }
            }
            _ => format!("error unknown command '{}' (status, cells, up, down, idle, roaming on|off)\n", line.trim()),
        }
    }
}

/// `udp 10.250.0.2:5060 -> 100.65.2.1:5060` for a log line.
fn packet_desc(p: &[u8]) -> String {
    if p.len() < 20 {
        return format!("{} bytes", p.len());
    }
    let ihl = ((p[0] & 0x0f) as usize) * 4;
    let ip = |o: usize| format!("{}.{}.{}.{}", p[o], p[o + 1], p[o + 2], p[o + 3]);
    let port = |o: usize| if p.len() >= o + 2 { u16::from_be_bytes([p[o], p[o + 1]]).to_string() } else { "-".into() };
    let proto = match p[9] {
        1 => "icmp",
        6 => "tcp",
        17 => "udp",
        _ => "ip",
    };
    if p[9] == 17 && p.len() > ihl + 8 {
        // a SIP request or status line says what woke the link
        let body = &p[ihl + 8..];
        let line: String = body.iter().take_while(|&&b| b != b'\r' && b != b'\n').take(48).map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' }).collect();
        format!("{proto} {}:{} -> {}:{} {line}", ip(12), port(ihl), ip(16), port(ihl + 2))
    } else if p[9] == 6 {
        format!("{proto} {}:{} -> {}:{}", ip(12), port(ihl), ip(16), port(ihl + 2))
    } else {
        format!("{proto} {} -> {}", ip(12), ip(16))
    }
}
