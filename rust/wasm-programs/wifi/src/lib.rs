//! `wifi`: the simple way onto a Wi-Fi network.
//!
//! ```text
//! wifi                          what wlan0 is doing (network, signal, address)
//! wifi scan                     networks in range
//! wifi connect <ssid> [pass]    join a network and get an address (DHCP)
//! wifi disconnect               leave the network
//! wifi mode wifi|controller     switch the Wi-Fi Module between Wi-Fi and
//!                               Wireless Controller receiver
//! ```
//!
//! `connect` checks every requirement it can see, in order, and stops at the
//! first one that's missing with what to do about it: a Wi-Fi Module in a bay,
//! the module in Wi-Fi mode, wlan0 not in monitor mode, the network in range,
//! a password when the network needs one, the password accepted by the access
//! point, and an address from a DHCP server behind the access point.
//!
//! It runs the same WPA2 supplicant as `wpa_supplicant` in-process until the
//! 4-way handshake completes, then leaves the association with the kernel and
//! exits; the kernel's DHCP client keeps the address. If the link drops, run
//! it again (or run `wpa_supplicant -B ...` for a daemon that reconnects).
//!
//! Everything that decides what to print lives here so it is host-testable;
//! `main.rs` only does the I/O.

use ecm_host_abi::wifi::{hex, kv, ScanEntry, WifiEvent};
use ecm_wifi::frame::MacAddr;
use wpa_supplicant::conf::{self, ConfNetwork, Psk};
use wpa_supplicant::{parse_mac, Driver, Port, WpaState};

/// How long `connect` lets the supplicant work before giving up, ms.
pub const CONNECT_TIMEOUT_MS: u64 = 20_000;
/// How long a scan may take, ms.
pub const SCAN_TIMEOUT_MS: u64 = 5_000;
/// Below this the link works but drops easily.
pub const WEAK_SIGNAL_DBM: i32 = -80;

/// What `connect` needs from the computer besides the Wi-Fi control channel.
pub trait Host: Port {
    /// Wi-Fi Modules the computer has as peripherals: their current mode
    /// (`wifi` or `controller`).
    fn wifi_module_modes(&mut self) -> Vec<String>;
    /// Wait about `ms` milliseconds (the supplicant keeps no time of its own).
    fn sleep_ms(&mut self, ms: u64);
}

/// A requirement that isn't met: what's wrong and how to fix it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub what: String,
    pub fix: Vec<String>,
}

impl Problem {
    fn new(what: impl Into<String>, fix: &[&str]) -> Problem {
        Problem { what: what.into(), fix: fix.iter().map(|s| s.to_string()).collect() }
    }

    /// The text printed for the player.
    pub fn render(&self) -> String {
        let mut s = format!("wifi: {}\n", self.what);
        if !self.fix.is_empty() {
            s += "To fix it:\n";
            for (i, f) in self.fix.iter().enumerate() {
                s += &format!("  {}. {}\n", i + 1, f);
            }
        }
        s
    }
}

// ------------------------------------------------------------ requirements

/// The computer has a working wlan0 in managed mode. `status` is the kernel's
/// `status` reply (None: no Wi-Fi control channel at all), `modules` the modes
/// of the Wi-Fi Module peripherals.
pub fn check_interface(status: Option<&str>, modules: &[String]) -> Result<MacAddr, Problem> {
    let Some(status) = status else {
        return Err(Problem::new(
            "this computer's system has no Wi-Fi support",
            &["Update the mod: Wi-Fi needs the Radio & Wireless update of the computer OS."],
        ));
    };
    if kv(status, "present") != Some("1") {
        if modules.iter().any(|m| m == "controller") {
            return Err(Problem::new(
                "the Wi-Fi Module is in controller mode (it's receiving a Wireless Controller, not Wi-Fi)",
                &["Run: wifi mode wifi", "Then run this command again."],
            ));
        }
        return Err(Problem::new(
            "this computer has no Wi-Fi Module",
            &[
                "Hold a Module Expansion Card and right-click a side of the computer (not the screen) to fit a bay.",
                "Hold a Wi-Fi Module and right-click the same side to put it in the bay.",
                "Run this command again (wlan0 appears as soon as the module is in).",
            ],
        ));
    }
    if kv(status, "mode") == Some("monitor") {
        return Err(Problem::new(
            "wlan0 is in monitor mode (capturing packets), so it can't join a network",
            &["Run: iw dev wlan0 set type managed", "Then run this command again."],
        ));
    }
    kv(status, "mac").and_then(parse_mac).ok_or_else(|| Problem::new("the Wi-Fi Module didn't report its address", &["Take the Wi-Fi Module out and put it back, then try again."]))
}

/// What `connect` will join, chosen from a scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub network: ScanEntry,
    pub secured: bool,
    /// Something worth knowing that doesn't stop the connection.
    pub warnings: Vec<String>,
}

fn heard_list(entries: &[ScanEntry]) -> String {
    let mut seen: Vec<&ScanEntry> = Vec::new();
    for e in entries {
        if e.ssid.is_empty() || seen.iter().any(|s| s.ssid == e.ssid) {
            continue;
        }
        seen.push(e);
    }
    seen.iter().map(|e| format!("'{}' ({} dBm)", e.ssid_str(), e.signal_dbm)).collect::<Vec<_>>().join(", ")
}

/// Pick the network called `ssid` (its strongest access point) and check the
/// password against what it needs.
pub fn choose(entries: &[ScanEntry], ssid: &str, password: Option<&str>) -> Result<Choice, Problem> {
    if entries.iter().all(|e| e.ssid.is_empty()) {
        return Err(Problem::new(
            "no Wi-Fi networks are in range",
            &[
                "Place an Access Point (a network cable next to it connects it to your wired network).",
                "Stay within range: 2.4 GHz passes through glass, wood and leaves, but stone, water and iron block it.",
                "Run 'wifi scan' to check what this computer can hear.",
            ],
        ));
    }
    let best = entries.iter().filter(|e| e.ssid == ssid.as_bytes()).max_by_key(|e| e.signal_dbm);
    let Some(best) = best else {
        let near = entries.iter().find(|e| e.ssid_str().eq_ignore_ascii_case(ssid));
        let mut fix = Vec::new();
        if let Some(n) = near {
            fix.push(format!("Network names are case-sensitive: did you mean '{}'?", n.ssid_str()));
        }
        fix.push("Check the name: right-click the Access Point and look at its SSID.".to_string());
        fix.push("Move closer or remove stone, water or iron between you and the Access Point.".to_string());
        return Err(Problem {
            what: format!("no network called '{}' is in range (this computer hears {})", ssid, heard_list(entries)),
            fix,
        });
    };
    let mut warnings = Vec::new();
    let secured = match best.security.as_str() {
        "open" => {
            if password.is_some() {
                warnings.push(format!("'{}' is an open network; the password isn't needed and was ignored.", ssid));
            }
            false
        }
        "wpa2-psk" => {
            match password {
                None => {
                    return Err(Problem {
                        what: format!("'{}' needs a password (WPA2)", ssid),
                        fix: vec![
                            format!("Run: wifi connect {} <password>", quote(ssid)),
                            "Put the password in quotes if it has spaces.".to_string(),
                            "The Access Point's owner sets the password in its settings (right-click it).".to_string(),
                        ],
                    })
                }
                Some(p) if p.len() < 8 || p.len() > 63 => {
                    return Err(Problem::new(
                        format!("that password can't be right: WPA2 passwords are 8 to 63 characters, this one is {}", p.len()),
                        &["Check the password (put it in quotes if it has spaces)."],
                    ))
                }
                Some(_) => {}
            }
            true
        }
        _ => {
            return Err(Problem::new(
                format!("'{}' uses a kind of security this computer can't join", ssid),
                &["Set the Access Point to WPA2 or Open in its settings (right-click it)."],
            ))
        }
    };
    if best.signal_dbm < WEAK_SIGNAL_DBM {
        warnings.push(format!(
            "The signal is weak ({} dBm): the connection may be slow or drop. Move closer or clear walls between you and the Access Point.",
            best.signal_dbm
        ));
    }
    Ok(Choice { network: best.clone(), secured, warnings })
}

/// Quote a word for the shell if it needs it.
pub fn quote(s: &str) -> String {
    if s.contains(' ') {
        format!("\"{}\"", s)
    } else {
        s.to_string()
    }
}

/// The network block saved for `wpa_supplicant`/`wpa_cli`.
pub fn conf_network(ssid: &str, password: Option<&str>) -> ConfNetwork {
    ConfNetwork {
        ssid: ssid.as_bytes().to_vec(),
        psk: password.map(|p| Psk::Passphrase(p.to_string())),
        open: password.is_none(),
        ..Default::default()
    }
}

/// `conf` with `n` added (replacing a network with the same SSID). A `text`
/// that doesn't parse is an error (its parse error): rewriting it would drop
/// every network it holds.
pub fn save_network(text: Option<&str>, n: ConfNetwork) -> Result<String, String> {
    let mut c = match text {
        Some(t) => conf::parse(t).map_err(|e| e.to_string())?,
        None => Default::default(),
    };
    c.networks.retain(|x| x.ssid != n.ssid);
    c.networks.push(n);
    Ok(conf::render(&c))
}

// ------------------------------------------------------------ scanning

/// Scan and return what was heard.
pub fn scan(host: &mut dyn Host) -> Result<Vec<ScanEntry>, Problem> {
    let ctl_err = || Problem::new("the Wi-Fi control channel didn't answer", &["Try again in a moment."]);
    let (_, st) = host.ctl("status").ok_or_else(ctl_err)?;
    let after: u64 = kv(&String::from_utf8_lossy(&st), "event_seq").and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut waited = 0;
    loop {
        match host.ctl("scan") {
            Some((0, _)) => break,
            // Busy: a scan is already running (ours from a moment ago, or the supplicant's).
            Some(_) if waited < SCAN_TIMEOUT_MS => {
                host.sleep_ms(100);
                waited += 100;
            }
            _ => return Err(ctl_err()),
        }
    }
    while waited < SCAN_TIMEOUT_MS {
        host.sleep_ms(100);
        waited += 100;
        if let Some((0, ev)) = host.ctl(&format!("events {}", after)) {
            if WifiEvent::parse_all(&String::from_utf8_lossy(&ev)).iter().any(|e| e.name == "SCAN_DONE") {
                break;
            }
        }
    }
    match host.ctl("scan_results") {
        Some((0, r)) => Ok(ScanEntry::parse_all(&String::from_utf8_lossy(&r))),
        _ => Err(ctl_err()),
    }
}

/// `wifi scan` output.
pub fn format_scan(entries: &[ScanEntry]) -> String {
    if entries.is_empty() {
        return "No Wi-Fi networks in range.\n".to_string();
    }
    let mut v: Vec<&ScanEntry> = entries.iter().collect();
    v.sort_by_key(|e| -e.signal_dbm);
    let mut s = format!("{:<24} {:>7} {:>4}  {}\n", "NETWORK", "SIGNAL", "CH", "SECURITY");
    for e in v {
        let name = if e.ssid.is_empty() { "(hidden)".to_string() } else { e.ssid_str() };
        let sec = match e.security.as_str() {
            "open" => "open",
            "wpa2-psk" => "WPA2 (password)",
            _ => "unsupported",
        };
        s += &format!("{:<24} {:>3} dBm {:>4}  {}\n", name, e.signal_dbm, e.channel, sec);
    }
    s
}

// ------------------------------------------------------------ joining

/// Records the supplicant's log while passing everything else through.
struct Recorder<'a> {
    host: &'a mut dyn Host,
    log: Vec<String>,
}

impl Port for Recorder<'_> {
    fn ctl(&mut self, req: &str) -> Option<(i32, Vec<u8>)> {
        self.host.ctl(req)
    }
    fn eapol_send(&mut self, eth: &[u8]) -> bool {
        self.host.eapol_send(eth)
    }
    fn eapol_recv(&mut self) -> Option<Vec<u8>> {
        self.host.eapol_recv()
    }
    fn now_ms(&mut self) -> u64 {
        self.host.now_ms()
    }
    fn log(&mut self, line: &str) {
        self.log.push(line.to_string());
    }
}

/// Where the link ended up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Joined {
    pub bssid: String,
    pub channel: u32,
    pub signal_dbm: Option<i32>,
}

/// Run the supplicant until the network is joined (WPA2: keys installed) or
/// it's clear why not.
pub fn join(host: &mut dyn Host, own: MacAddr, choice: &Choice, password: Option<&str>, rand: Box<dyn FnMut(&mut [u8])>) -> Result<Joined, Problem> {
    let ssid = choice.network.ssid_str();
    let net = conf_network(&ssid, if choice.secured { password } else { None });
    let cfg = net.to_network_config().map_err(|e| Problem::new(format!("can't use that network: {}", e), &[]))?;
    let mut d = Driver::new(own, "wlan0", rand, vec![cfg]);
    let mut port = Recorder { host, log: Vec::new() };
    d.attach(&mut port).map_err(|e| Problem::new(e, &[]))?;
    let start = port.now_ms();
    let mut rejects = 0;
    loop {
        d.step(&mut port);
        if d.state == WpaState::Completed {
            break;
        }
        if port.log.iter().any(|l| l.contains("4-Way Handshake failed") || l.contains("MIC failure")) {
            port.ctl("disconnect");
            return Err(Problem {
                what: format!("the password for '{}' is wrong (the Access Point rejected it)", ssid),
                fix: vec![
                    "Check the password: it's case-sensitive; put it in quotes if it has spaces.".to_string(),
                    "The Access Point's owner can see whether a password is set and change it in its settings (right-click it).".to_string(),
                ],
            });
        }
        let r = port.log.iter().filter(|l| l.contains("CTRL-EVENT-ASSOC-REJECT")).count();
        if r > rejects && r >= 3 {
            port.ctl("disconnect");
            let reason = port.log.iter().rev().find(|l| l.contains("ASSOC-REJECT")).cloned().unwrap_or_default();
            return Err(Problem {
                what: format!("the Access Point for '{}' refused this computer ({})", ssid, reason.trim()),
                fix: vec![
                    "The Access Point may be full or its owner may have blocked this computer: check its Status tab (right-click it).".to_string(),
                    "Try again in a moment.".to_string(),
                ],
            });
        }
        rejects = r;
        if port.now_ms().saturating_sub(start) > CONNECT_TIMEOUT_MS {
            port.ctl("disconnect");
            return Err(Problem {
                what: format!(
                    "couldn't join '{}' in {} seconds (signal {} dBm, last step: {})",
                    ssid,
                    CONNECT_TIMEOUT_MS / 1000,
                    choice.network.signal_dbm,
                    d.state.name()
                ),
                fix: vec![
                    "Move closer to the Access Point or clear stone, water or iron between you.".to_string(),
                    "Check that the Access Point is still there and its radio is on (right-click it).".to_string(),
                ],
            });
        }
        port.host.sleep_ms(20);
    }
    let (_, st) = port.ctl("status").unwrap_or((0, Vec::new()));
    let st = String::from_utf8_lossy(&st).into_owned();
    Ok(Joined {
        bssid: kv(&st, "bssid").unwrap_or(&choice.network.bssid).to_string(),
        channel: kv(&st, "bss_channel").and_then(|c| c.parse().ok()).unwrap_or(choice.network.channel),
        signal_dbm: kv(&st, "signal").and_then(|s| s.parse().ok()),
    })
}

/// No address after joining.
pub fn no_lease(ssid: &str, state: &str, secs: u64) -> Problem {
    Problem {
        what: format!("joined '{}', but nothing gave this computer an address (no DHCP server answered in {} s; DHCP state {})", ssid, secs, state),
        fix: vec![
            "Cable the Access Point to your network (a network cable touching it; its lights come on).".to_string(),
            "Run a DHCP server on that network: on a cabled computer, set up /etc/dhcpd.conf and run 'dhcpd &' (or use a router with a DHCP pool). The Internet Gateway doesn't hand out addresses.".to_string(),
            "Or give this computer an address by hand: ifconfig wlan0 <address>/<prefix>".to_string(),
            "The DHCP client keeps trying in the background: 'dhclient -s wlan0' shows when it gets one.".to_string(),
        ],
    }
}

/// `wifi` (status) output from the kernel's `status` reply and the address.
pub fn format_status(status: &str, address: Option<&str>) -> String {
    let mut s = String::new();
    let state = kv(status, "state").unwrap_or("?");
    let authorized = kv(status, "authorized") == Some("1");
    match (kv(status, "ssid"), kv(status, "bssid")) {
        (Some(ssid), Some(bssid)) if state == "ASSOCIATED" => {
            let name = ecm_host_abi::wifi::unhex(ssid).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            let wpa2 = kv(status, "target_security") == Some("wpa2");
            if wpa2 && !authorized {
                s += &format!("wlan0: joining '{}' ({}): password handshake not finished\n", name, bssid);
            } else {
                s += &format!("wlan0: connected to '{}' ({})\n", name, bssid);
            }
            s += &format!(
                "  channel {}, signal {} dBm, {}\n",
                kv(status, "bss_channel").unwrap_or("?"),
                kv(status, "signal").unwrap_or("?"),
                if wpa2 { "WPA2" } else { "open" }
            );
        }
        _ => s += &format!("wlan0: not connected (state {})\n  Run 'wifi scan' to see networks and 'wifi connect <name> [password]' to join one.\n", state),
    }
    s += &format!("  address: {}\n", address.unwrap_or("none"));
    s
}

/// Hex-encoded SSID for kernel requests.
pub fn ssid_hex(ssid: &str) -> String {
    hex(ssid.as_bytes())
}

pub fn usage() -> &'static str {
    "usage:\n\
     \x20 wifi                          show the Wi-Fi connection\n\
     \x20 wifi scan                     list networks in range\n\
     \x20 wifi connect <name> [password] join a network (quote names/passwords with spaces)\n\
     \x20 wifi disconnect               leave the network\n\
     \x20 wifi mode wifi|controller     use the Wi-Fi Module for Wi-Fi or as a Wireless Controller receiver\n"
}

#[cfg(test)]
mod tests;
