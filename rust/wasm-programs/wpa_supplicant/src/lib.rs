//! `wpa_supplicant` for ECM: network selection, the WPA2-PSK 4-way and group-key
//! handshakes (`ecm_wifi::supplicant`), key installation into the kernel's `wlan0`
//! and reconnection / roaming follow-up.
//!
//! Everything here is host-testable: the program's `main` only provides a [`Port`]
//! (the kernel's Wi-Fi control channel, an EAPOL transport and a clock).
//!
//! Files:
//! * `/etc/wpa_supplicant.conf` — `network={ ssid="..." psk="..." }` blocks (see [`conf`]);
//! * `/run/wpa_supplicant.<ifname>.status` — `key=value` status for `wpa_cli status`;
//! * `/run/wpa_supplicant.<ifname>.cmd` — commands from `wpa_cli` (`reconfigure`,
//!   `disconnect`, `reconnect`, `reassociate`), one per line, consumed by the daemon.

pub mod conf;

/// The computer's files through the ECM host. Paths are written Linux-style
/// (`/etc/wpa_supplicant.conf`); the host takes them relative to the disk root.
pub mod files {
    pub fn rel(p: &str) -> &str {
        p.trim_start_matches('/')
    }
    pub fn read(p: &str) -> Option<String> {
        ecm_host_abi::fs::read_file(rel(p))
    }
    /// Write (creating parent directories); false on failure.
    pub fn write(p: &str, data: &str) -> bool {
        if let Some((dir, _)) = rel(p).rsplit_once('/') {
            if !dir.is_empty() {
                ecm_host_abi::fs::mkdir(dir);
            }
        }
        ecm_host_abi::fs::write_file(rel(p), data) >= 0
    }
    pub fn exists(p: &str) -> bool {
        ecm_host_abi::fs::exists(rel(p))
    }
    pub fn delete(p: &str) {
        ecm_host_abi::fs::delete(rel(p));
    }
}

use ecm_host_abi::wifi::{hex, kv, ScanEntry, WifiEvent};
use ecm_wifi::frame::{self, BssInfo, MacAddr, Rsn};
use ecm_wifi::supplicant::{HandshakeState, NetworkConfig, Supplicant, SupplicantEvent, SupplicantOutput};
use ecm_wifi::ConnectSecurity;

pub const DEFAULT_CONF: &str = "/etc/wpa_supplicant.conf";
pub const RUN_DIR: &str = "/run";

pub fn status_path(ifname: &str) -> String {
    format!("{}/wpa_supplicant.{}.status", RUN_DIR, ifname)
}
pub fn cmd_path(ifname: &str) -> String {
    format!("{}/wpa_supplicant.{}.cmd", RUN_DIR, ifname)
}

/// Command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    pub ifname: String,
    pub conf: String,
    pub background: bool,
    /// `-d`: debug log lines.
    pub debug: bool,
    /// `-f <file>`: log file (default: stdout, or none with `-B`).
    pub log_file: Option<String>,
    /// `-D packet|ctl`: EAPOL transport (default: packet socket if the kernel has
    /// `AF_PACKET`, else the kernel's EAPOL tap on the control channel).
    pub driver: Option<String>,
}

pub fn usage() -> &'static str {
    "usage: wpa_supplicant [-B] [-d] -i <ifname> [-c <config>] [-f <logfile>] [-D packet|ctl]\n\
     \n  -B  run in the background (the shell returns to the prompt)\
     \n  -c  configuration file (default /etc/wpa_supplicant.conf)\
     \n  -i  interface (wlan0)\n"
}

pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut a = Args { ifname: String::new(), conf: DEFAULT_CONF.into(), background: false, debug: false, log_file: None, driver: None };
    let mut i = 0;
    while i < argv.len() {
        let s = argv[i].as_str();
        let value = |i: &mut usize, flag: &str| -> Result<String, String> {
            // -iwlan0 or -i wlan0
            if s.len() > 2 {
                return Ok(s[2..].to_string());
            }
            *i += 1;
            argv.get(*i).cloned().ok_or_else(|| format!("option requires an argument -- '{}'", flag))
        };
        match s.get(..2) {
            Some("-B") if s.len() == 2 => a.background = true,
            Some("-d") if s.chars().skip(1).all(|c| c == 'd') => a.debug = true,
            Some("-i") => a.ifname = value(&mut i, "i")?,
            Some("-c") => a.conf = value(&mut i, "c")?,
            Some("-f") => a.log_file = Some(value(&mut i, "f")?),
            Some("-D") => {
                let d = value(&mut i, "D")?;
                // Accept Linux driver names too (nl80211, wext) as "auto".
                a.driver = match d.as_str() {
                    "packet" | "ctl" => Some(d),
                    "nl80211" | "wext" => None,
                    _ => return Err(format!("unknown driver '{}'", d)),
                };
            }
            _ => return Err(format!("unknown option '{}'", s)),
        }
        i += 1;
    }
    if a.ifname.is_empty() {
        return Err("no interface given (-i wlan0)".into());
    }
    Ok(a)
}

/// `wpa_state` values as `wpa_cli status` shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WpaState {
    InterfaceDisabled,
    Inactive,
    Disconnected,
    Scanning,
    Associating,
    Associated,
    FourWayHandshake,
    Completed,
}

impl WpaState {
    pub fn name(&self) -> &'static str {
        match self {
            WpaState::InterfaceDisabled => "INTERFACE_DISABLED",
            WpaState::Inactive => "INACTIVE",
            WpaState::Disconnected => "DISCONNECTED",
            WpaState::Scanning => "SCANNING",
            WpaState::Associating => "ASSOCIATING",
            WpaState::Associated => "ASSOCIATED",
            WpaState::FourWayHandshake => "4WAY_HANDSHAKE",
            WpaState::Completed => "COMPLETED",
        }
    }
}

/// What the driver needs from the outside world.
pub trait Port {
    /// One request on the kernel's Wi-Fi control channel.
    fn ctl(&mut self, req: &str) -> Option<(i32, Vec<u8>)>;
    /// Send a whole Ethernet frame (EtherType 0x888E) out of the interface.
    fn eapol_send(&mut self, eth: &[u8]) -> bool;
    /// Next received EAPOL Ethernet frame, if any (non-blocking).
    fn eapol_recv(&mut self) -> Option<Vec<u8>>;
    fn now_ms(&mut self) -> u64;
    fn log(&mut self, line: &str);
}

/// Seconds between scans while no configured network is in range.
pub const RESCAN_MS: u64 = 5_000;
/// Delay before rescanning after a failed connection attempt.
pub const RETRY_MS: u64 = 2_000;

/// The supplicant daemon's state machine.
pub struct Driver {
    pub supp: Supplicant,
    pub own: MacAddr,
    pub ifname: String,
    pub state: WpaState,
    event_seq: u64,
    scan_pending: bool,
    next_scan_at: u64,
    /// User asked to stay disconnected (`wpa_cli disconnect`).
    pub user_disconnected: bool,
    pub bssid: Option<MacAddr>,
    pub ssid: Option<Vec<u8>>,
    pub freq: u32,
    pub network_id: Option<usize>,
    pub last_event: String,
    pub completed_count: u32,
    status_dirty: bool,
}

pub fn parse_mac(s: &str) -> Option<MacAddr> {
    let p: Vec<&str> = s.split(':').collect();
    if p.len() != 6 {
        return None;
    }
    let mut m = [0u8; 6];
    for (i, x) in p.iter().enumerate() {
        m[i] = u8::from_str_radix(x, 16).ok()?;
    }
    Some(m)
}

/// A scan line as the BSS description network selection works on.
pub fn bss_from_scan(e: &ScanEntry) -> Option<BssInfo> {
    let bssid = parse_mac(&e.bssid)?;
    let (cap, rsn) = match e.security.as_str() {
        "open" => (frame::CAP_ESS, None),
        "wpa2-psk" => (frame::CAP_ESS | frame::CAP_PRIVACY, Some(Rsn::wpa2_psk())),
        _ => (frame::CAP_ESS | frame::CAP_PRIVACY, None),
    };
    Some(BssInfo {
        bssid,
        ssid: e.ssid.clone(),
        channel: e.channel as u8,
        cap,
        beacon_interval: 100,
        rates: Vec::new(),
        ht_mcs: 0,
        rsn_ie: rsn.as_ref().map(|r| r.body()),
        rsn,
        rssi: e.signal_dbm,
        last_seen: 0,
    })
}

impl Driver {
    pub fn new(own: MacAddr, ifname: &str, rand: Box<dyn FnMut(&mut [u8])>, networks: Vec<NetworkConfig>) -> Driver {
        let mut supp = Supplicant::new(own, rand);
        for n in networks {
            supp.add_network(n);
        }
        Driver {
            supp,
            own,
            ifname: ifname.into(),
            state: WpaState::Disconnected,
            event_seq: 0,
            scan_pending: false,
            next_scan_at: 0,
            user_disconnected: false,
            bssid: None,
            ssid: None,
            freq: 0,
            network_id: None,
            last_event: String::new(),
            completed_count: 0,
            status_dirty: true,
        }
    }

    /// Start from the kernel's current event sequence (old events are history).
    pub fn attach(&mut self, port: &mut dyn Port) -> Result<(), String> {
        let (st, reply) = port.ctl("status").ok_or("no Wi-Fi control channel (not running on ECM?)")?;
        let text = String::from_utf8_lossy(&reply).into_owned();
        if st != 0 || kv(&text, "present") != Some("1") {
            self.set_state(WpaState::InterfaceDisabled);
            return Err(format!("{}: no Wi-Fi module installed", self.ifname));
        }
        self.event_seq = kv(&text, "event_seq").and_then(|s| s.parse().ok()).unwrap_or(0);
        if kv(&text, "mode") == Some("monitor") {
            return Err(format!("{}: interface is in monitor mode (iw dev {} set type managed)", self.ifname, self.ifname));
        }
        // Start clean: a previous association has no keys from us.
        port.ctl("disconnect");
        self.next_scan_at = 0;
        Ok(())
    }

    pub fn set_networks(&mut self, port: &mut dyn Port, nets: Vec<NetworkConfig>) {
        *self.supp.networks_mut() = nets;
        port.log("CTRL-EVENT-CONFIG-RELOADED");
        port.ctl("disconnect");
        self.supp.on_disconnected();
        self.user_disconnected = false;
        self.scan_pending = false;
        self.next_scan_at = port.now_ms();
        self.set_state(WpaState::Disconnected);
    }

    fn set_state(&mut self, s: WpaState) {
        if s != self.state {
            self.state = s;
            self.status_dirty = true;
        }
    }

    /// Returns the status file text once after each change.
    pub fn take_status(&mut self) -> Option<String> {
        if !self.status_dirty {
            return None;
        }
        self.status_dirty = false;
        Some(self.status_text())
    }

    pub fn status_text(&self) -> String {
        let mut s = format!("wpa_state={}\naddress={}\n", self.state.name(), frame::mac_str(&self.own));
        if let (Some(b), Some(ssid)) = (self.bssid, self.ssid.as_ref()) {
            s += &format!("bssid={}\nfreq={}\nssid={}\n", frame::mac_str(&b), self.freq, String::from_utf8_lossy(ssid));
            if let Some(id) = self.network_id {
                s += &format!("id={}\n", id);
            }
            let secured = self.network_id.and_then(|i| self.supp.networks().get(i)).is_some_and(|n| n.psk.is_some());
            s += &format!("mode=station\npairwise_cipher={}\ngroup_cipher={}\nkey_mgmt={}\n",
                if secured { "CCMP" } else { "NONE" },
                if secured { "CCMP" } else { "NONE" },
                if secured { "WPA2-PSK" } else { "NONE" });
        }
        s += &format!("completed={}\n", self.completed_count);
        if !self.last_event.is_empty() {
            s += &format!("last_event={}\n", self.last_event);
        }
        s
    }

    fn event(&mut self, port: &mut dyn Port, line: String) {
        port.log(&line);
        self.last_event = line;
        self.status_dirty = true;
    }

    /// A command from `wpa_cli` (`/run/wpa_supplicant.<if>.cmd`).
    pub fn command(&mut self, port: &mut dyn Port, cmd: &str) {
        match cmd.trim() {
            "disconnect" => {
                self.user_disconnected = true;
                port.ctl("disconnect");
                self.set_state(WpaState::Disconnected);
            }
            "reconnect" | "reassociate" => {
                self.user_disconnected = false;
                if cmd.trim() == "reassociate" {
                    port.ctl("disconnect");
                }
                self.next_scan_at = port.now_ms();
            }
            "scan" => {
                self.next_scan_at = port.now_ms();
                self.user_disconnected = false;
            }
            _ => {}
        }
    }

    /// One iteration of the main loop.
    pub fn step(&mut self, port: &mut dyn Port) {
        let now = port.now_ms();
        // 1. Kernel events.
        if let Some((0, ev)) = port.ctl(&format!("events {}", self.event_seq)) {
            let text = String::from_utf8_lossy(&ev).into_owned();
            for e in WifiEvent::parse_all(&text) {
                self.event_seq = self.event_seq.max(e.seq);
                self.on_kernel_event(port, &e, now);
            }
        }
        // 2. EAPOL from the AP.
        while let Some(f) = port.eapol_recv() {
            self.supp.on_ethernet(&f, now);
        }
        // 3. Timers.
        self.supp.poll(now);
        // 4. Supplicant output.
        while let Some(o) = self.supp.pop_output() {
            match o {
                SupplicantOutput::SendEapol { dst, frame: e } => {
                    let eth = frame::ethernet(&dst, &self.own, frame::ETHERTYPE_EAPOL, &e);
                    if !port.eapol_send(&eth) {
                        port.log("EAPOL: send failed");
                    }
                    if self.state == WpaState::Associated {
                        self.set_state(WpaState::FourWayHandshake);
                    }
                }
                SupplicantOutput::InstallPtk { bssid, tk } => {
                    port.ctl(&format!("install_ptk {} {}", frame::mac_str(&bssid), hex(&tk)));
                }
                SupplicantOutput::InstallGtk { key_id, gtk, rsc } => {
                    port.ctl(&format!("install_gtk {} {} {}", key_id, rsc, hex(&gtk)));
                }
                SupplicantOutput::Disconnect { reason } => {
                    port.ctl(&format!("disconnect {}", reason));
                }
                SupplicantOutput::Event(e) => self.on_supplicant_event(port, e, now),
            }
        }
        // 5. Look for a network when idle.
        let idle = matches!(self.state, WpaState::Disconnected | WpaState::Inactive);
        if idle && !self.scan_pending && !self.user_disconnected && now >= self.next_scan_at {
            if self.supp.networks().iter().all(|n| n.disabled) {
                self.set_state(WpaState::Inactive);
                self.next_scan_at = now + RESCAN_MS;
            } else {
                match port.ctl("scan") {
                    Some((0, _)) => {
                        self.scan_pending = true;
                        self.set_state(WpaState::Scanning);
                    }
                    // Busy (a user `iw scan` is running): its results will do.
                    Some((st, _)) if st == ecm_host_abi::wifi::EBUSY => {
                        self.scan_pending = true;
                        self.set_state(WpaState::Scanning);
                    }
                    _ => self.next_scan_at = now + RETRY_MS,
                }
            }
        }
    }

    fn on_kernel_event(&mut self, port: &mut dyn Port, e: &WifiEvent, now: u64) {
        match e.name.as_str() {
            "SCAN_DONE" => {
                if !self.scan_pending {
                    return;
                }
                self.scan_pending = false;
                if self.state != WpaState::Scanning || self.user_disconnected {
                    return;
                }
                let results = port
                    .ctl("scan_results")
                    .filter(|(s, _)| *s == 0)
                    .map(|(_, r)| ScanEntry::parse_all(&String::from_utf8_lossy(&r)))
                    .unwrap_or_default();
                let bss: Vec<BssInfo> = results.iter().filter_map(bss_from_scan).collect();
                match self.supp.select_network(&bss) {
                    Some(p) => {
                        let sec = if p.security == ConnectSecurity::Wpa2Psk { "wpa2" } else { "open" };
                        let mut req = format!("connect {} {}", hex(&p.ssid), sec);
                        if let Some(b) = p.bssid {
                            req += &format!(" {}", frame::mac_str(&b));
                        }
                        self.network_id = self.supp.networks().iter().position(|n| n.ssid == p.ssid && !n.disabled);
                        let ssid = String::from_utf8_lossy(&p.ssid).into_owned();
                        port.log(&format!("{}: Trying to associate with SSID '{}'", self.ifname, ssid));
                        match port.ctl(&req) {
                            Some((0, _)) => self.set_state(WpaState::Associating),
                            _ => {
                                self.set_state(WpaState::Disconnected);
                                self.next_scan_at = now + RETRY_MS;
                            }
                        }
                    }
                    None => {
                        self.set_state(WpaState::Disconnected);
                        self.next_scan_at = now + RESCAN_MS;
                    }
                }
            }
            "CONNECTED" => {
                let (Some(bssid), Some(ssid)) = (e.get("bssid").and_then(parse_mac), e.hex_field("ssid")) else { return };
                let ap_rsn = e.hex_field("ap_rsn");
                let sta_rsn = e.hex_field("sta_rsn");
                let ch: u32 = e.get("channel").and_then(|c| c.parse().ok()).unwrap_or(0);
                self.bssid = Some(bssid);
                self.freq = frame::channel_to_freq(ch as u8) as u32;
                self.network_id = self.supp.networks().iter().position(|n| n.ssid == ssid && !n.disabled);
                self.ssid = Some(ssid.clone());
                let roamed = e.get("roamed") == Some("1");
                self.event(port, format!("{}: Associated with {}{}", self.ifname, frame::mac_str(&bssid), if roamed { " (roamed)" } else { "" }));
                self.set_state(WpaState::Associated);
                self.supp.on_associated(bssid, &ssid, ap_rsn.as_deref(), sta_rsn.as_deref(), now);
                // Open network: the supplicant reports Completed at once (handled below).
            }
            "DISCONNECTED" => {
                let reason = e.get("reason").unwrap_or("0").to_string();
                let local = e.get("local") == Some("1");
                let bssid = e.get("bssid").unwrap_or("-").to_string();
                self.event(port, format!("CTRL-EVENT-DISCONNECTED bssid={} reason={}{}", bssid, reason, if local { " locally_generated=1" } else { "" }));
                self.supp.on_disconnected();
                self.bssid = None;
                self.ssid = None;
                // The kernel reconnects by itself after deauth / beacon loss; a local
                // disconnect (ours, or the handshake failing) needs a new scan.
                if self.state != WpaState::Disconnected {
                    self.set_state(WpaState::Disconnected);
                }
                self.next_scan_at = now + if local { RETRY_MS } else { RETRY_MS * 2 };
            }
            "CONNECT_FAILED" => {
                let reason = e.get("reason").unwrap_or("?").to_string();
                self.event(port, format!("CTRL-EVENT-ASSOC-REJECT reason={}", reason));
                self.supp.on_disconnected();
                self.set_state(WpaState::Disconnected);
                self.next_scan_at = now + RETRY_MS;
            }
            "REMOVED" => {
                self.supp.on_disconnected();
                self.set_state(WpaState::InterfaceDisabled);
            }
            "PRESENT" => {
                self.set_state(WpaState::Disconnected);
                self.next_scan_at = now;
            }
            _ => {}
        }
    }

    fn on_supplicant_event(&mut self, port: &mut dyn Port, e: SupplicantEvent, _now: u64) {
        match e {
            SupplicantEvent::Completed { bssid } => {
                self.completed_count += 1;
                self.set_state(WpaState::Completed);
                let id = self.network_id.map(|i| i.to_string()).unwrap_or_else(|| "?".into());
                self.event(port, format!("CTRL-EVENT-CONNECTED - Connection to {} completed [id={}]", frame::mac_str(&bssid), id));
            }
            SupplicantEvent::GroupRekey { key_id } => {
                self.event(port, format!("WPA: Group rekeying completed (key id {})", key_id));
            }
            SupplicantEvent::MicFailure { bssid, message } => {
                self.event(port, format!("WPA: MIC failure in message {} from {}", message, frame::mac_str(&bssid)));
            }
            SupplicantEvent::RsnIeMismatch { bssid } => {
                self.event(port, format!("WPA: RSN IE in 4-Way Handshake differs from Beacon/ProbeResp ({})", frame::mac_str(&bssid)));
            }
            SupplicantEvent::HandshakeTimeout { bssid } => {
                self.event(port, format!("WPA: 4-Way Handshake with {} timed out", frame::mac_str(&bssid)));
            }
            SupplicantEvent::PossibleWrongKey { bssid } => {
                self.event(port, format!("WPA: 4-Way Handshake failed - pre-shared key may be incorrect ({})", frame::mac_str(&bssid)));
            }
        }
    }

    pub fn handshake(&self) -> &HandshakeState {
        self.supp.state()
    }
}

#[cfg(test)]
extern crate self as wpa_supplicant;
#[cfg(test)]
mod tests;
