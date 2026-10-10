//! `wifi` — see `lib.rs`.

use std::thread::sleep;
use std::time::{Duration, Instant};

use ecm_host_abi::dhcp;
use ecm_host_abi::netlink::{dhcp_state_name, DHCP_STATE_BOUND};
use ecm_host_abi::peripheral::{self, Value};
use ecm_host_abi::wifi as hw;
use ::wifi::{check_interface, choose, conf_network, format_scan, format_status, join, no_lease, save_network, scan, usage, Host, Problem};
use wpa_supplicant::{cmd_path, files, status_path, Port, DEFAULT_CONF};

#[link(wasm_import_module = "env")]
extern "C" {
    fn get_time_ms() -> i64;
}

/// How long to wait for a DHCP lease after joining, seconds.
const DHCP_WAIT_SECS: u64 = 10;

/// The real computer: the kernel's Wi-Fi control channel (EAPOL through its tap)
/// and the peripheral bus.
struct Computer;

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

fn die(p: Problem) -> ! {
    eprint!("{}", p.render());
    std::process::exit(1);
}

fn status_text() -> Option<String> {
    hw::ctl("status").map(|(_, r)| String::from_utf8_lossy(&r).into_owned())
}

fn address() -> Option<String> {
    let s = dhcp::status("wlan0").ok()?;
    let a = s.address?;
    Some(format!("{}.{}.{}.{}/{}", a[0], a[1], a[2], a[3], s.prefix))
}

fn ip(a: Option<[u8; 4]>) -> String {
    a.map(|a| format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3])).unwrap_or_else(|| "-".into())
}

fn ready(c: &mut Computer) -> [u8; 6] {
    let modules = c.wifi_module_modes();
    check_interface(status_text().as_deref(), &modules).unwrap_or_else(|p| die(p))
}

/// A running `wpa_supplicant` would fight over wlan0: ask it to stop.
fn stop_daemon() {
    let sp = status_path("wlan0");
    let Some(st) = files::read(&sp) else { return };
    if st.contains("wpa_state=INTERFACE_DISABLED") {
        return;
    }
    println!("Stopping the running wpa_supplicant so wifi can manage wlan0...");
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

fn connect(ssid: &str, password: Option<&str>) {
    let mut c = Computer;
    let own = ready(&mut c);
    stop_daemon();
    println!("Looking for '{}'...", ssid);
    let found = scan(&mut c).unwrap_or_else(|p| die(p));
    let choice = choose(&found, ssid, password).unwrap_or_else(|p| die(p));
    for w in &choice.warnings {
        println!("note: {}", w);
    }
    println!(
        "Found '{}' ({} dBm, channel {}, {}). Joining...",
        ssid,
        choice.network.signal_dbm,
        choice.network.channel,
        if choice.secured { "WPA2" } else { "open" }
    );
    let rand = Box::new(|b: &mut [u8]| {
        ecm_host_abi::random::getrandom(b);
    });
    hw::ctl("eapol_subscribe");
    let joined = join(&mut c, own, &choice, password, rand);
    hw::ctl("eapol_unsubscribe");
    let joined = joined.unwrap_or_else(|p| die(p));
    // Saved for wpa_cli and for a wpa_supplicant daemon.
    let saved = save_network(files::read(DEFAULT_CONF).as_deref(), conf_network(ssid, if choice.secured { password } else { None }));
    files::write(DEFAULT_CONF, &saved);
    println!(
        "Joined '{}' ({}, channel {}{}). Getting an address...",
        ssid,
        joined.bssid,
        joined.channel,
        joined.signal_dbm.map(|s| format!(", {} dBm", s)).unwrap_or_default()
    );

    // A lease from before (another network) is stale: start over.
    let _ = dhcp::stop("wlan0");
    if let Err(e) = dhcp::start("wlan0") {
        die(Problem { what: format!("couldn't start the DHCP client on wlan0 ({:?})", e), fix: vec!["Give wlan0 an address by hand: ifconfig wlan0 <address>/<prefix>".into()] });
    }
    let t0 = Instant::now();
    loop {
        let s = dhcp::status("wlan0").unwrap_or_default();
        if s.state == DHCP_STATE_BOUND && s.address.is_some() {
            println!("Connected. Address {}/{}, router {}, DNS {}", ip(s.address), s.prefix, ip(s.router), ip(s.dns));
            return;
        }
        if t0.elapsed() >= Duration::from_secs(DHCP_WAIT_SECS) {
            die(no_lease(ssid, dhcp_state_name(s.state), DHCP_WAIT_SECS));
        }
        sleep(Duration::from_millis(200));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match a.as_slice() {
        [] | ["status"] => {
            let mut c = Computer;
            ready(&mut c);
            let st = status_text().unwrap_or_default();
            print!("{}", format_status(&st, address().as_deref()));
        }
        ["scan"] => {
            let mut c = Computer;
            ready(&mut c);
            print!("{}", format_scan(&scan(&mut c).unwrap_or_else(|p| die(p))));
        }
        ["connect", ssid] => connect(ssid, None),
        ["connect", ssid, password] => connect(ssid, Some(password)),
        ["connect", ssid, rest @ ..] if !rest.is_empty() => {
            // An unquoted password with spaces: the shell split it.
            connect(ssid, Some(&rest.join(" ")))
        }
        ["disconnect"] => {
            let mut c = Computer;
            ready(&mut c);
            stop_daemon();
            let _ = dhcp::release("wlan0");
            hw::ctl("disconnect");
            println!("wlan0: disconnected");
        }
        ["mode", m @ ("wifi" | "controller")] => {
            let Ok(list) = peripheral::list() else { die(Problem { what: "can't list this computer's peripherals".into(), fix: vec![] }) };
            let Some((name, _)) = list.iter().find(|(_, t)| t == "wifi") else {
                die(Problem {
                    what: "this computer has no Wi-Fi Module".into(),
                    fix: vec![
                        "Hold a Module Expansion Card and right-click a side of the computer (not the screen) to fit a bay.".into(),
                        "Hold a Wi-Fi Module and right-click the same side to put it in the bay.".into(),
                    ],
                })
            };
            match peripheral::call(name, "set_mode", &[Value::Str(m.to_string())]) {
                Ok(_) => println!("Wi-Fi Module ({}) is now in {} mode", name, m),
                Err(e) => die(Problem { what: format!("couldn't switch the Wi-Fi Module: {:?}", e), fix: vec![] }),
            }
        }
        ["help"] | ["-h"] | ["--help"] => print!("{}", usage()),
        _ => {
            eprint!("{}", usage());
            std::process::exit(2);
        }
    }
}
