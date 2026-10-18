//! The computer's side of celld (WASI only): the modem's framed radio
//! (`/dev/frames.<bay>`), the SIM through the `sim` peripheral, `wwan0` as a
//! `tun`, netlink for the default route and the resolver, the control socket,
//! `/dev/uictl` for the phone's status bar, and the program's main loop.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use ecm_cell::codec::{fmt_ip, hex, unhex};
use ecm_cell::phy::{FramedPhy, PhyCmd, PhyConfig, PhyEvent, Profile, Role};
use ecm_cell::simauth::{Sim, SimError, SimIdentity, SimResponse};
use ecm_cell::Now;
use ecm_host_abi::peripheral::{self, Value};
use ecm_host_abi::socket::{self, PollFd, SockAddrIn};
use ecm_host_abi::tun;
use ecm_radio::framed::{self, Framed, TxOpts};

use crate::config::{self, Config};
use crate::ctl;
use crate::daemon::{Daemon, Io, Options};

fn get<'a>(m: &'a Value, key: &str) -> Option<&'a Value> {
    match m {
        Value::Map(e) => e.iter().find(|(k, _)| matches!(k, Value::Str(s) if s == key)).map(|(_, v)| v),
        _ => None,
    }
}

fn get_str(m: &Value, key: &str) -> Option<String> {
    match get(m, key) {
        Some(Value::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

// ------------------------------------------------------------------ the SIM

/// The SIM behind a `sim` peripheral (the phone's SIM tray, or the modem's own
/// slot on a Terminal). The card answers on the server; K never comes here.
pub struct HostSim {
    pub name: String,
}

impl Sim for HostSim {
    fn identity(&self) -> Result<SimIdentity, SimError> {
        let v = peripheral::call(&self.name, "identity", &[]).map_err(|_| SimError::Failed)?;
        match (get_str(&v, "carrier"), get_str(&v, "imsi")) {
            (Some(carrier), Some(imsi)) => Ok(SimIdentity { carrier, imsi }),
            _ => Err(SimError::NoCard),
        }
    }

    fn respond(&mut self, rand: &[u8; 16]) -> Result<SimResponse, SimError> {
        let v = peripheral::call(&self.name, "respond", &[Value::Str(hex(rand))]).map_err(|_| SimError::Failed)?;
        let res = get_str(&v, "res").and_then(|h| unhex::<16>(&h)).ok_or(SimError::Failed)?;
        let ks = get_str(&v, "ks").and_then(|h| unhex::<32>(&h)).ok_or(SimError::Failed)?;
        Ok(SimResponse { res, ks })
    }
}

// ------------------------------------------------------------------ the computer

pub struct HostIo {
    framed: Framed,
    /// What the radio is set to (a retune only sends what changed: `phy` would reset it).
    applied: Option<PhyConfig>,
    t0: Instant,
    slot_us: u64,
    slot_base: u64,
    slot_at: Instant,
    tun_fd: Option<i32>,
    /// `ims0`, the IMS bearer's tun (docs/TELEPHONY.md).
    ims_fd: Option<i32>,
    phone: bool,
    log: Logger,
    /// The phone's state, read from `/dev/uictl phone` now and then.
    asleep: bool,
    asleep_at: Option<Instant>,
}

impl HostIo {
    fn sync_slot(&mut self) {
        self.slot_base = Framed::slot(&mut self.framed).unwrap_or(self.slot_base);
        self.slot_at = Instant::now();
    }
}

impl Io for HostIo {
    fn now(&mut self) -> Now {
        if self.slot_at.elapsed() >= Duration::from_millis(250) {
            self.sync_slot();
        }
        let slot = self.slot_base + self.slot_at.elapsed().as_micros() as u64 / self.slot_us.max(1);
        Now { ms: self.t0.elapsed().as_millis() as u64, slot }
    }

    fn phy(&mut self, cmd: PhyCmd) -> Result<(), String> {
        match cmd {
            PhyCmd::Configure(c) => {
                let lines: Vec<String> = match &self.applied {
                    Some(a) if a.profile == c.profile => {
                        let mut v = Vec::new();
                        if a.role != c.role {
                            v.push(format!("role {}", if c.role == Role::Bs { "bs" } else { "ue" }));
                        }
                        if a.channel != c.channel {
                            v.push(format!("channel {}", c.channel));
                        }
                        if a.power_dbm != c.power_dbm {
                            if let Some(p) = c.power_dbm {
                                v.push(format!("power {p}"));
                            }
                        }
                        if a.address != c.address {
                            v.push(format!("address {:08x}", c.address));
                        }
                        v
                    }
                    _ => framed::Config {
                        phy: Some(c.profile.name().into()),
                        role: Some(if c.role == Role::Bs { "bs".into() } else { "ue".into() }),
                        channel: Some(c.channel as u32),
                        bandwidth_hz: None,
                        power_dbm: c.power_dbm,
                        address: Some(c.address),
                        promiscuous: Some(false),
                    }
                    .lines(),
                };
                for l in lines {
                    self.framed.control(&l).map_err(|e| format!("{l}: {e}"))?;
                }
                self.applied = Some(c);
                Ok(())
            }
            PhyCmd::Tx(t) => {
                let o = TxOpts { harq: t.harq, slot: t.slot, cqi: t.cqi, subchannel: t.subchannel, tag: t.tag };
                self.framed.send(t.dst, &t.payload, &o).map_err(|e| e.to_string())
            }
            PhyCmd::Beacon { index, period_ms, payload } => self.framed.beacon(index, period_ms, &payload).map_err(|e| e.to_string()),
        }
    }

    fn phy_poll(&mut self) -> Option<PhyEvent> {
        FramedPhy::poll(&mut self.framed)
    }

    fn tun_open(&mut self, name: &str, mac: [u8; 6], ip: [u8; 4], prefix: u8) -> Result<(), String> {
        self.tun_close();
        let fd = tun::open(name, mac, ip, prefix);
        if fd < 0 {
            return Err(format!("error {fd} (is another program using {name}?)"));
        }
        self.tun_fd = Some(fd);
        Ok(())
    }

    fn tun_close(&mut self) {
        if let Some(fd) = self.tun_fd.take() {
            socket::close(fd);
        }
    }

    fn tun_read(&mut self) -> Option<Vec<u8>> {
        let fd = self.tun_fd?;
        let mut buf = vec![0u8; 2048];
        let n = tun::read(fd, &mut buf);
        if n <= 0 {
            return None;
        }
        buf.truncate(n as usize);
        Some(buf)
    }

    fn tun_write(&mut self, frame: &[u8]) {
        if let Some(fd) = self.tun_fd {
            tun::write(fd, frame);
        }
    }

    fn default_route(&mut self, gw: [u8; 4]) -> Result<(), String> {
        use ecm_host_abi::netlink::*;
        let fd = socket::socket(socket::AF_NETLINK, socket::SOCK_DGRAM, socket::NETLINK_ROUTE);
        if fd < 0 {
            return Err("no netlink".into());
        }
        // header, rtmsg (family, dst_len 0, src_len, tos, table main, proto static, scope universe, unicast, flags), RTA_GATEWAY
        let mut m = vec![0u8; NLMSG_HDR_SIZE];
        m.extend_from_slice(&[2, 0, 0, 0, RT_TABLE_MAIN, RTPROT_STATIC, RT_SCOPE_UNIVERSE, RTN_UNICAST, 0, 0, 0, 0]);
        m.extend_from_slice(&8u16.to_le_bytes());
        m.extend_from_slice(&RTA_GATEWAY.to_le_bytes());
        m.extend_from_slice(&gw);
        let len = m.len() as u32;
        NlMsgHdr { nlmsg_len: len, nlmsg_type: RTM_NEWROUTE, nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE, nlmsg_seq: 7, nlmsg_pid: 0 }
            .serialize(&mut m);
        socket::sendto(fd, &m, 0, &SockAddrIn::default());
        let mut buf = [0u8; 64];
        let mut from = SockAddrIn::default();
        let n = socket::recvfrom(fd, &mut buf, 0, &mut from);
        socket::close(fd);
        let err = if n >= 20 { i32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]) } else { -1 };
        if err == 0 {
            Ok(())
        } else {
            Err(err.to_string())
        }
    }

    fn set_dns(&mut self, dns: [u8; 4]) -> Result<(), String> {
        match ecm_host_abi::net_config::dns_set(&fmt_ip(dns)) {
            0 => Ok(()),
            e => Err(e.to_string()),
        }
    }

    fn log(&mut self, line: &str) {
        self.log.line(line);
    }

    fn ims_open(&mut self, mac: [u8; 6], ip: [u8; 4], prefix: u8) -> Result<(), String> {
        self.ims_close();
        let fd = tun::open("ims0", mac, ip, prefix);
        if fd < 0 {
            return Err(format!("error {fd}"));
        }
        self.ims_fd = Some(fd);
        Ok(())
    }

    fn ims_close(&mut self) {
        if let Some(fd) = self.ims_fd.take() {
            socket::close(fd);
        }
    }

    fn ims_read(&mut self) -> Option<Vec<u8>> {
        let fd = self.ims_fd?;
        let mut buf = vec![0u8; 2048];
        let n = tun::read(fd, &mut buf);
        if n <= 0 {
            return None;
        }
        buf.truncate(n as usize);
        Some(buf)
    }

    fn ims_write(&mut self, frame: &[u8]) {
        if let Some(fd) = self.ims_fd {
            tun::write(fd, frame);
        }
    }

    fn ims_route(&mut self, pcscf: [u8; 4]) -> Result<(), String> {
        let n = ecm_host_abi::net_config::iface_count();
        let idx = (0..n).find(|i| ecm_host_abi::net_config::iface_info(*i).is_some_and(|j| j.contains("\"name\":\"ims0\""))).ok_or("ims0 not found")?;
        match ecm_host_abi::net_config::route_add(&fmt_ip(pcscf), 32, "0.0.0.0", idx) {
            0 => Ok(()),
            e => Err(e.to_string()),
        }
    }

    fn sleeping(&mut self) -> bool {
        if !self.phone {
            return false;
        }
        if self.asleep_at.is_none_or(|t| t.elapsed() >= Duration::from_millis(250)) {
            self.asleep_at = Some(Instant::now());
            let mut st = String::new();
            if let Ok(mut f) = std::fs::OpenOptions::new().read(true).write(true).open("/dev/uictl") {
                if f.write_all(b"phone").is_ok() {
                    let _ = f.read_to_string(&mut st);
                }
            }
            self.asleep = st.lines().any(|l| l.trim() == "state asleep" || l.trim() == "sleep_pending 1");
        }
        self.asleep
    }

    fn wakelock(&mut self, ms: u64) {
        if !self.phone {
            return;
        }
        // the platform's wake lock (PLATFORM_API.md): per pid, capped at apps.wakelockMaxMs
        if let Ok(mut f) = std::fs::OpenOptions::new().read(true).write(true).open("/dev/uictl") {
            if f.write_all(format!("wakelock {ms}").as_bytes()).is_ok() {
                let mut answer = String::new();
                let _ = f.read_to_string(&mut answer);
                if answer.trim() != "ok" {
                    self.log.line(&format!("wakelock {ms}: {}", answer.trim()));
                }
            }
        }
    }

    fn wake_filter(&mut self, tmsi: Option<u32>) {
        let line = match tmsi {
            Some(t) => format!("wake tmsi {t}"),
            None => "wake off".to_string(),
        };
        if let Err(e) = self.framed.control(&line) {
            self.log.line(&format!("modem: {line}: {e}"));
        }
    }

    fn report(&mut self, bars: u8, carrier: &str) {
        if !self.phone {
            return;
        }
        let cmd = format!("cell {} {}", bars, if carrier.is_empty() { "-" } else { carrier });
        if let Ok(mut f) = std::fs::OpenOptions::new().read(true).write(true).open("/dev/uictl") {
            if f.write_all(cmd.as_bytes()).is_ok() {
                let mut answer = String::new();
                let _ = f.read_to_string(&mut answer);
            }
        }
    }

    fn save_setting(&mut self, key: &str, value: &str) -> Result<(), String> {
        let text = std::fs::read_to_string(config::PATH).unwrap_or_default();
        let _ = std::fs::create_dir_all("/etc");
        std::fs::write(config::PATH, config::set(&text, key, value)).map_err(|e| e.to_string())
    }
}

/// Log lines: all of them to `/var/log/celld.log` (kept under 64 KB), the
/// interesting ones to stdout (not the search loop, no repeats).
struct Logger {
    t0: Instant,
    last: String,
    quiet: bool,
}

const LOG: &str = "/var/log/celld.log";

impl Logger {
    fn line(&mut self, l: &str) {
        let t = self.t0.elapsed().as_millis() as f64 / 1000.0;
        let entry = format!("[{t:>9.3}s] {l}\n");
        let _ = std::fs::create_dir_all("/var/log");
        if std::fs::metadata(LOG).map(|m| m.len() > 64 * 1024).unwrap_or(false) {
            let _ = std::fs::rename(LOG, "/var/log/celld.log.1");
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(LOG) {
            let _ = f.write_all(entry.as_bytes());
        }
        let noisy = l == "searching" || l.starts_with("no service") || l.starts_with("radio:");
        if !self.quiet && l != self.last && !(noisy && self.last.starts_with("no service")) {
            println!("celld: {l}");
        }
        self.last = l.to_string();
    }
}

// ------------------------------------------------------------------ control socket client

/// Ask the running celld (`cell`, Settings). Err: celld isn't running.
pub fn query(cmd: &str, timeout_ms: i32) -> Result<String, String> {
    let fd = socket::socket(socket::AF_INET, socket::SOCK_DGRAM, socket::IPPROTO_UDP);
    if fd < 0 {
        return Err("no sockets".into());
    }
    let to = SockAddrIn::from_ip_port(ctl::LOOPBACK, ctl::PORT);
    if socket::sendto(fd, cmd.as_bytes(), 0, &to) < 0 {
        socket::close(fd);
        return Err("celld is not running".into());
    }
    let mut p = [PollFd { fd, events: socket::POLLIN, revents: 0 }];
    let ready = socket::poll(&mut p, timeout_ms);
    if ready <= 0 || p[0].revents & socket::POLLIN == 0 {
        socket::close(fd);
        return Err("celld is not running".into());
    }
    let mut buf = vec![0u8; 4096];
    let mut from = SockAddrIn::default();
    let n = socket::recvfrom(fd, &mut buf, 0, &mut from);
    socket::close(fd);
    if n < 0 {
        return Err("celld is not running".into());
    }
    Ok(String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

// ------------------------------------------------------------------ the program

pub const USAGE: &str = "usage: celld [--phone] [--modem NAME] [--sim NAME] [--quiet]
  the cellular daemon: attaches the modem (a framed_radio) with the SIM, runs wwan0.
  Settings in /etc/celld.toml; talk to it with `cell` (cell, cell scan, cell up|down).";

struct Args {
    phone: bool,
    modem: Option<String>,
    sim: Option<String>,
    quiet: bool,
}

fn parse_args(a: &[String]) -> Result<Args, String> {
    let mut out = Args { phone: false, modem: None, sim: None, quiet: false };
    let mut i = 0;
    while i < a.len() {
        match a[i].as_str() {
            "--phone" => out.phone = true,
            "--quiet" | "-q" => out.quiet = true,
            "--modem" => {
                i += 1;
                out.modem = Some(a.get(i).ok_or("--modem needs a name")?.clone());
            }
            "--sim" => {
                i += 1;
                out.sim = Some(a.get(i).ok_or("--sim needs a name")?.clone());
            }
            "-h" | "--help" => return Err(String::new()),
            o => return Err(format!("unknown argument '{o}'")),
        }
        i += 1;
    }
    Ok(out)
}

fn fail(msg: &str) -> ! {
    eprintln!("celld: {msg}");
    std::process::exit(1);
}

pub fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("celld: {e}");
            }
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    let cfg = match std::fs::read_to_string(config::PATH) {
        Ok(t) => Config::parse(&t).unwrap_or_else(|e| fail(&format!("{}: {e}", config::PATH))),
        Err(_) => Config::default(),
    };
    let list = peripheral::list().unwrap_or_default();
    let modem = args
        .modem
        .clone()
        .or_else(|| cfg.modem.clone())
        .or_else(|| list.iter().find(|(_, t)| t == "framed_radio").map(|(n, _)| n.clone()))
        .unwrap_or_else(|| {
            fail("no modem: fit an LTE Modem or mmWave Modem (a framed_radio) in a module bay");
        });
    let sim_name = args
        .sim
        .clone()
        .or_else(|| cfg.sim.clone())
        .or_else(|| list.iter().find(|(n, t)| t == "sim" && *n == format!("{modem}_sim")).map(|(n, _)| n.clone()))
        .or_else(|| list.iter().find(|(_, t)| t == "sim").map(|(n, _)| n.clone()))
        .unwrap_or_else(|| format!("{modem}_sim"));
    let mut framed = Framed::open(&modem).unwrap_or_else(|e| fail(&format!("{modem}: {e} (is it a framed_radio?)")));
    let st = framed.status().unwrap_or_else(|e| fail(&format!("{modem}: {e}")));
    let profile = st.get("phy").and_then(Profile::parse).unwrap_or(Profile::Lte);
    let address = framed.address().unwrap_or_else(|e| fail(&format!("{modem}: {e}")));

    // The control socket; a second celld finds it taken.
    let ctl_fd = socket::socket(socket::AF_INET, socket::SOCK_DGRAM, socket::IPPROTO_UDP);
    if ctl_fd < 0 || socket::bind(ctl_fd, &SockAddrIn::from_ip_port(ctl::LOOPBACK, ctl::PORT)) < 0 {
        fail(&format!("can't open the control socket 127.0.0.1:{} (is celld already running?)", ctl::PORT));
    }

    let t0 = Instant::now();
    let mut io = HostIo {
        framed,
        applied: None,
        t0,
        slot_us: profile.slot_us(),
        slot_base: 0,
        slot_at: t0,
        tun_fd: None,
        ims_fd: None,
        asleep: false,
        asleep_at: None,
        phone: args.phone,
        log: Logger { t0, last: String::new(), quiet: args.quiet },
    };
    io.sync_slot();
    let mut sim = HostSim { name: sim_name.clone() };
    io.log(&format!("modem {modem} ({} radio {:08x}), SIM {sim_name}, control 127.0.0.1:{}", profile.name(), address, ctl::PORT));
    let opts = Options { profile, address, modem, tun: "wwan0".into(), power_dbm: None, config: cfg };
    let mut d = Daemon::new(opts, 0);
    d.start(&mut io, &mut sim);
    let mut buf = vec![0u8; 512];
    loop {
        d.step(&mut io, &mut sim);
        for _ in 0..8 {
            let mut from = SockAddrIn::default();
            let n = socket::recvfrom(ctl_fd, &mut buf, socket::MSG_DONTWAIT, &mut from);
            if n <= 0 {
                break;
            }
            if from.ip()[0] != 127 {
                continue;
            }
            let line = String::from_utf8_lossy(&buf[..n as usize]).into_owned();
            let answer = d.control(&line, &mut io, &mut sim);
            socket::sendto(ctl_fd, answer.as_bytes(), 0, &from);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
