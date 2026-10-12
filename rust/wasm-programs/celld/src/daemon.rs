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

use ecm_cell::air::{Plain, SysInfo};
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
/// The status bar is refreshed this often even when nothing changed.
const REPORT_MS: u64 = 5000;
/// Cells are listed this long after their last beacon.
const HEARD_MS: u64 = 10_000;

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
    }
}

impl Daemon {
    pub fn new(opts: Options, now_ms: u64) -> Daemon {
        let mut cfg = UeConfig::new(opts.profile, opts.address);
        cfg.channels = opts.config.channels.clone();
        cfg.power_dbm = opts.power_dbm;
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
        self.ue.tick(now);
        self.drain(io);
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
                    if self.tun.is_some() {
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
            UeEvent::Searching | UeEvent::Resumed { .. } | UeEvent::Idle => {}
        }
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
            ["cells"] | ["scan"] => self.cells(now.ms).iter().map(|r| r.format() + "\n").collect(),
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
            ["roaming", v @ ("on" | "off")] => {
                let on = *v == "on";
                self.opts.config.roaming = on;
                match io.save_setting("roaming", if on { "true" } else { "false" }) {
                    Ok(()) => format!("ok roaming {v}\n"),
                    Err(e) => format!("ok roaming {v} (not saved: {e})\n"),
                }
            }
            _ => format!("error unknown command '{}' (status, cells, up, down, roaming on|off)\n", line.trim()),
        }
    }
}
