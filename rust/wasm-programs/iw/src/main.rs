//! `iw dev wlan0 scan|link|info|set ...` — see `lib.rs`.

use ecm_host_abi::wifi::{self, hex, kv, ScanEntry, WifiEvent};
use iw::{format_info, format_scan, format_station, parse, usage, Cmd};

fn ctl_text(req: &str) -> (i32, String) {
    match wifi::ctl(req) {
        Some((s, r)) => (s, String::from_utf8_lossy(&r).into_owned()),
        None => {
            eprintln!("command failed: no Wi-Fi control channel");
            std::process::exit(1);
        }
    }
}

fn check(req: &str) {
    let (s, r) = ctl_text(req);
    if s < 0 {
        eprintln!("command failed: {} ({})", r.trim(), s);
        std::process::exit(1);
    }
}

fn sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

fn scan(dev: &str, passive: bool, ssid: Option<String>, dump: bool, status: &str) {
    if !dump {
        if kv(status, "mode") == Some("monitor") {
            eprintln!("command failed: Operation not supported (-95)");
            std::process::exit(1);
        }
        let seq: u64 = kv(status, "event_seq").and_then(|s| s.parse().ok()).unwrap_or(0);
        let mut req = String::from("scan");
        if let Some(s) = &ssid {
            req += &format!(" {}", hex(s.as_bytes()));
        }
        if passive {
            req += " passive";
        }
        // A scan already running (wpa_supplicant's) is just as good: wait for it.
        let (s, r) = ctl_text(&req);
        if s < 0 && s != wifi::EBUSY {
            eprintln!("command failed: {} ({})", r.trim(), s);
            std::process::exit(1);
        }
        let mut done = false;
        for _ in 0..200 {
            let (_, ev) = ctl_text(&format!("events {}", seq));
            if WifiEvent::parse_all(&ev).iter().any(|e| e.name == "SCAN_DONE") {
                done = true;
                break;
            }
            sleep_ms(50);
        }
        if !done {
            eprintln!("command failed: scan timed out (-110)");
            std::process::exit(1);
        }
    }
    let (_, res) = ctl_text("scan_results");
    let mut entries = ScanEntry::parse_all(&res);
    if let Some(s) = ssid {
        entries.retain(|e| e.ssid == s.as_bytes());
    }
    let status = ctl_text("status").1;
    print!("{}", format_scan(dev, &entries, kv(&status, "bssid")));
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (dev, cmd) = match parse(&argv) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{}\n{}", e, usage());
            std::process::exit(1);
        }
    };
    if cmd == Cmd::Help {
        print!("{}", usage());
        return;
    }
    let (_, status) = ctl_text("status");
    if kv(&status, "present") != Some("1") {
        if cmd == Cmd::ListDevs {
            return; // no wireless devices: iw dev prints nothing
        }
        eprintln!("command failed: No such device (-19)");
        std::process::exit(1);
    }
    if !dev.is_empty() && dev != "wlan0" {
        eprintln!("command failed: No such device (-19)");
        std::process::exit(1);
    }
    let dev = if dev.is_empty() { "wlan0".to_string() } else { dev };
    match cmd {
        Cmd::Help => {}
        Cmd::ListDevs => print!("{}", format_info(&status, true)),
        Cmd::Info => print!("{}", format_info(&status, false)),
        Cmd::Scan { passive, ssid, dump } => scan(&dev, passive, ssid, dump, &status),
        Cmd::Link => print!("{}", ctl_text("link").1),
        Cmd::StationDump => {
            let link = ctl_text("link").1;
            let stats = ctl_text("stats").1;
            print!("{}", format_station(&link, &stats, &dev));
        }
        Cmd::SetType(t) => check(&format!("set_type {}", t)),
        Cmd::SetChannel(n) => check(&format!("set_channel {}", n)),
        Cmd::SetTxPower(mbm) => check(&format!("set_power {}", (mbm / 100).clamp(0, 20))),
        Cmd::SetKeepalive(ms) => check(&format!("set_keepalive {}", ms)),
        Cmd::Connect { ssid, bssid } => {
            let mut req = format!("connect {} open", hex(ssid.as_bytes()));
            if let Some(b) = bssid {
                req += &format!(" {}", b);
            }
            check(&req);
        }
        Cmd::Disconnect => check("disconnect"),
    }
}
