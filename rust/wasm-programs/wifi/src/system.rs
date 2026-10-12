//! The `wifi` steps against the real computer (wasm32 only): the kernel's
//! Wi-Fi control channel, the peripheral bus, the DHCP client and the
//! supplicant config. Shared by the `wifi` command and the GUI Settings app;
//! both report the same progress lines and the same [`Problem`]s.

use std::thread::sleep;
use std::time::{Duration, Instant};

use ecm_host_abi::dhcp;
use ecm_host_abi::netlink::{dhcp_state_name, DHCP_STATE_BOUND};
use ecm_host_abi::peripheral::{self, Value};
use ecm_host_abi::wifi as hw;
use ecm_host_abi::wifi::ScanEntry;
use wpa_supplicant::{cmd_path, files, status_path, Port, DEFAULT_CONF};

use crate::{check_interface, choose, conf_network, join, no_lease, save_network, scan, Host, Problem};

#[link(wasm_import_module = "env")]
extern "C" {
    fn get_time_ms() -> i64;
}

/// How long to wait for a DHCP lease after joining, seconds.
pub const DHCP_WAIT_SECS: u64 = 10;

/// The real computer: the kernel's Wi-Fi control channel (EAPOL through its tap)
/// and the peripheral bus.
pub struct Computer;

impl Port for Computer {
    fn ctl(&mut self, req: &str) -> Option<(i32, Vec<u8>)> {
        hw::ctl(req)
    }
    fn eapol_send(&mut self, eth: &[u8]) -> bool {
        matches!(hw::ctl(&format!("eapol_tx {}", hw::hex(eth))), Some((0, _)))
    }
    fn eapol_recv(&mut self) -> Option<Vec<u8>> {
        match hw::ctl("eapol_rx") {
            Some((1, f)) => Some(f),
            _ => None,
        }
    }
    fn now_ms(&mut self) -> u64 {
        unsafe { get_time_ms() }.max(0) as u64
    }
    fn log(&mut self, _line: &str) {}
}

impl Host for Computer {
    fn wifi_module_modes(&mut self) -> Vec<String> {
        let Ok(list) = peripheral::list() else { return Vec::new() };
        list.iter()
            .filter(|(_, ty)| ty == "wifi")
            .filter_map(|(name, _)| match peripheral::call(name, "get_mode", &[]) {
                Ok(Value::Str(m)) => Some(m),
                _ => None,
            })
            .collect()
    }
    fn sleep_ms(&mut self, ms: u64) {
        sleep(Duration::from_millis(ms));
    }
}

/// The kernel's `status` reply for wlan0 (None: no Wi-Fi control channel).
pub fn status_text() -> Option<String> {
    hw::ctl("status").map(|(_, r)| String::from_utf8_lossy(&r).into_owned())
}

/// wlan0's DHCP address as `a.b.c.d/prefix`.
pub fn address() -> Option<String> {
    let s = dhcp::status("wlan0").ok()?;
    let a = s.address?;
    Some(format!("{}.{}.{}.{}/{}", a[0], a[1], a[2], a[3], s.prefix))
}

fn ip(a: Option<[u8; 4]>) -> String {
    a.map(|a| format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3])).unwrap_or_else(|| "-".into())
}

/// wlan0 is there and usable: its MAC, or what's wrong.
pub fn ready(c: &mut Computer) -> Result<[u8; 6], Problem> {
    let modules = c.wifi_module_modes();
    check_interface(status_text().as_deref(), &modules)
}

/// Scan for networks (checks the interface first).
pub fn scan_networks() -> Result<Vec<ScanEntry>, Problem> {
    let mut c = Computer;
    ready(&mut c)?;
    scan(&mut c)
}

/// A running `wpa_supplicant` would fight over wlan0: ask it to stop.
pub fn stop_daemon(progress: &mut dyn FnMut(&str)) {
    let sp = status_path("wlan0");
    let Some(st) = files::read(&sp) else { return };
    if st.contains("wpa_state=INTERFACE_DISABLED") {
        return;
    }
    progress("Stopping the running wpa_supplicant so wifi can manage wlan0...");
    files::write(&cmd_path("wlan0"), "terminate\n");
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(2) {
        if files::read(&sp).map_or(true, |s| s.contains("INTERFACE_DISABLED")) {
            return;
        }
        sleep(Duration::from_millis(50));
    }
    // Left over from before a reboot: nothing is running.
    files::delete(&sp);
    files::delete(&cmd_path("wlan0"));
}

/// Join `ssid` and get an address: every step is reported to `progress`
/// (the same lines `wifi connect` prints), notes about the config file to
/// `warn`. Returns the final "Connected. Address ..." line, or the first
/// requirement that isn't met.
pub fn connect(ssid: &str, password: Option<&str>, progress: &mut dyn FnMut(&str), warn: &mut dyn FnMut(&str)) -> Result<String, Problem> {
    let mut c = Computer;
    let own = ready(&mut c)?;
    stop_daemon(progress);
    progress(&format!("Looking for '{}'...", ssid));
    let found = scan(&mut c)?;
    let choice = choose(&found, ssid, password)?;
    for w in &choice.warnings {
        progress(&format!("note: {}", w));
    }
    progress(&format!(
        "Found '{}' ({} dBm, channel {}, {}). Joining...",
        ssid,
        choice.network.signal_dbm,
        choice.network.channel,
        if choice.secured { "WPA2" } else { "open" }
    ));
    let rand = Box::new(|b: &mut [u8]| {
        ecm_host_abi::random::getrandom(b);
    });
    hw::ctl("eapol_subscribe");
    let joined = join(&mut c, own, &choice, password, rand);
    hw::ctl("eapol_unsubscribe");
    let joined = joined?;
    // Saved for wpa_cli and for a wpa_supplicant daemon.
    let saved = save_network(files::read(DEFAULT_CONF).as_deref(), conf_network(ssid, if choice.secured { password } else { None }));
    match saved {
        Ok(text) => {
            if !files::write(DEFAULT_CONF, &text) {
                warn(&format!("wifi: couldn't write {}; '{}' wasn't saved there", DEFAULT_CONF, ssid));
            }
        }
        // Don't rewrite (and so empty) a file we can't read back.
        Err(e) => warn(&format!(
            "wifi: {} isn't a valid wpa_supplicant config ({}); left it unchanged, so '{}' wasn't saved there. Fix or delete it and connect again to save the network.",
            DEFAULT_CONF, e, ssid
        )),
    }
    progress(&format!(
        "Joined '{}' ({}, channel {}{}). Getting an address...",
        ssid,
        joined.bssid,
        joined.channel,
        joined.signal_dbm.map(|s| format!(", {} dBm", s)).unwrap_or_default()
    ));

    // A lease from before (another network) is stale: start over.
    let _ = dhcp::stop("wlan0");
    if let Err(e) = dhcp::start("wlan0") {
        return Err(Problem {
            what: format!("couldn't start the DHCP client on wlan0 ({:?})", e),
            fix: vec!["Give wlan0 an address by hand: ifconfig wlan0 <address>/<prefix>".into()],
        });
    }
    let t0 = Instant::now();
    loop {
        let s = dhcp::status("wlan0").unwrap_or_default();
        if s.state == DHCP_STATE_BOUND && s.address.is_some() {
            return Ok(format!("Connected. Address {}/{}, router {}, DNS {}", ip(s.address), s.prefix, ip(s.router), ip(s.dns)));
        }
        if t0.elapsed() >= Duration::from_secs(DHCP_WAIT_SECS) {
            return Err(no_lease(ssid, dhcp_state_name(s.state), DHCP_WAIT_SECS));
        }
        sleep(Duration::from_millis(200));
    }
}

/// Leave the network (and stop a supplicant daemon).
pub fn disconnect(progress: &mut dyn FnMut(&str)) -> Result<(), Problem> {
    let mut c = Computer;
    ready(&mut c)?;
    stop_daemon(progress);
    let _ = dhcp::release("wlan0");
    hw::ctl("disconnect");
    Ok(())
}
