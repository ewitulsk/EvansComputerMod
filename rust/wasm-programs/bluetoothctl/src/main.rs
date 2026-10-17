//! bluetoothctl: the console's Bluetooth tool (docs/BLUETOOTH.md "Programs").
//!
//!   bluetoothctl [show]            the radio: name, address, what is connected, where sound goes
//!   bluetoothctl devices           paired devices
//!   bluetoothctl scan [SECONDS]    devices nearby (default 5 s): open an AirPods case's lid to see them
//!   bluetoothctl pair DEVICE       pair (then it connects); DEVICE is an address or part of a name
//!   bluetoothctl connect DEVICE    connect a paired device
//!   bluetoothctl disconnect        disconnect (it stays paired)
//!   bluetoothctl remove DEVICE     forget a paired device
//!   bluetoothctl power on|off
//!
//! It talks to the computer's `bluetooth` peripheral (an ePhone has one).

use ecm_host_abi::peripheral::{self, Value};
use std::time::{Duration, Instant};

const USAGE: &str = "usage: bluetoothctl [show|devices|scan [SECONDS]|pair DEVICE|connect DEVICE|disconnect|remove DEVICE|power on|off]\n  DEVICE: an address (AC:DE:48:00:11:22) or part of a name (airpods)";

fn get<'a>(m: &'a Value, key: &str) -> Option<&'a Value> {
    match m {
        Value::Map(e) => e.iter().find(|(k, _)| matches!(k, Value::Str(s) if s == key)).map(|(_, v)| v),
        _ => None,
    }
}

fn text(m: &Value, key: &str) -> String {
    match get(m, key) {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Int(i)) => i.to_string(),
        Some(Value::Float(f)) => format!("{}", *f as i64),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn flag(m: &Value, key: &str) -> bool {
    matches!(get(m, key), Some(Value::Bool(true)))
}

fn call(method: &str, args: &[Value]) -> Result<Value, String> {
    peripheral::call("bluetooth", method, args).map_err(|e| match e {
        peripheral::Error::Peripheral(m) if m.contains("no peripheral") || m.contains("not attached") => {
            "this computer has no Bluetooth radio (an ePhone has one)".to_string()
        }
        peripheral::Error::Peripheral(m) => m,
        peripheral::Error::Unavailable => "this computer has no Bluetooth radio (an ePhone has one)".into(),
        peripheral::Error::Malformed(m) => m.to_string(),
    })
}

fn list(method: &str) -> Result<Vec<Value>, String> {
    match call(method, &[])? {
        Value::List(v) => Ok(v),
        _ => Ok(Vec::new()),
    }
}

/// An address, or the one device (paired or nearby) whose name contains `what`.
fn resolve(what: &str) -> Result<String, String> {
    if what.len() == 17 && what.matches(':').count() == 5 {
        return Ok(what.to_uppercase());
    }
    let w = what.to_lowercase();
    let mut all = list("devices")?;
    all.extend(list("scan")?);
    let mut hits: Vec<(String, String)> = all
        .iter()
        .filter(|d| text(d, "name").to_lowercase().contains(&w))
        .map(|d| (text(d, "address"), text(d, "name")))
        .collect();
    hits.dedup_by(|a, b| a.0 == b.0);
    match hits.len() {
        0 => Err(format!("no device called '{what}' in range or paired (open an AirPods case's lid, then: bluetoothctl scan)")),
        1 => Ok(hits.remove(0).0),
        _ => Err(format!("'{what}' matches several devices: {}; use an address", hits.iter().map(|h| format!("{} ({})", h.1, h.0)).collect::<Vec<_>>().join(", "))),
    }
}

/// Wait up to `secs` for status().last to become one of `done`; prints it.
fn wait_for(secs: u64, done: &[&str]) -> Result<String, String> {
    let start = Instant::now();
    loop {
        let st = call("status", &[])?;
        let last = text(&st, "last");
        // Done, or a failure ("no answer: ...", "lost the connection ...").
        let failed = !matches!(last.as_str(), "" | "pairing" | "connecting" | "paired" | "connected");
        if done.iter().any(|d| last == *d) || failed || start.elapsed() > Duration::from_secs(secs) {
            return Ok(last);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn show() -> Result<(), String> {
    let st = call("status", &[])?;
    println!("Controller {} ({})", text(&st, "address"), text(&st, "name"));
    println!("  Powered: {}", if flag(&st, "powered") { "yes" } else { "no" });
    match get(&st, "connected") {
        Some(Value::Str(a)) => {
            let bat = text(&st, "battery");
            let rssi = text(&st, "rssi_dbm");
            println!(
                "  Connected: {} ({}){}{}",
                text(&st, "connected_name"),
                a,
                if bat.is_empty() { String::new() } else { format!(", battery {bat}%") },
                if rssi.is_empty() { String::new() } else { format!(", {rssi} dBm") }
            );
        }
        _ => println!("  Connected: no"),
    }
    println!("  Audio: {}", if text(&st, "audio") == "headphones" { "headphones (the connected device)" } else { "speaker" });
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None | Some("show") => show(),
        Some("-h") | Some("--help") | Some("help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some("devices") => {
            let devs = list("devices")?;
            if devs.is_empty() {
                println!("No paired devices.");
            }
            for d in devs {
                let state = if flag(&d, "connected") { "connected" } else if flag(&d, "connecting") { "connecting" } else { "paired" };
                println!("Device {} {} ({})", text(&d, "address"), text(&d, "name"), state);
            }
            Ok(())
        }
        Some("scan") => {
            let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5).clamp(1, 30);
            println!("Scanning for {secs} s ...");
            let start = Instant::now();
            let mut shown: Vec<String> = Vec::new();
            while start.elapsed() < Duration::from_secs(secs) {
                for d in list("scan")? {
                    let a = text(&d, "address");
                    if shown.contains(&a) {
                        continue;
                    }
                    shown.push(a.clone());
                    let mut notes = Vec::new();
                    if flag(&d, "pairable") {
                        notes.push("pairing mode".to_string());
                    }
                    if flag(&d, "paired") {
                        notes.push("paired".to_string());
                    }
                    notes.push(format!("{} dBm", text(&d, "rssi_dbm")));
                    println!("[NEW] Device {} {} ({})", a, text(&d, "name"), notes.join(", "));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            if shown.is_empty() {
                println!("Nothing found. Open an AirPods case's lid next to this phone (right-click the case).");
            }
            Ok(())
        }
        Some("pair") => {
            let a = resolve(args.get(1).ok_or(USAGE)?)?;
            call("pair", &[Value::Str(a.clone())])?;
            println!("Pairing with {a} ...");
            match wait_for(6, &["connected", "refused"])?.as_str() {
                "connected" => {
                    println!("Paired and connected: sound plays on it while worn.");
                    Ok(())
                }
                "paired" => {
                    println!("Paired; it connects once it is worn (or its case is open).");
                    Ok(())
                }
                other => Err(format!("pairing failed: {other}")),
            }
        }
        Some("connect") => {
            let a = resolve(args.get(1).ok_or(USAGE)?)?;
            call("connect", &[Value::Str(a.clone())])?;
            println!("Connecting to {a} ...");
            match wait_for(6, &["connected", "refused"])?.as_str() {
                "connected" => {
                    println!("Connected.");
                    Ok(())
                }
                other => Err(format!("connection failed: {other}")),
            }
        }
        Some("disconnect") => {
            call("disconnect", &[])?;
            println!("Disconnected (still paired).");
            Ok(())
        }
        Some("remove") => {
            let a = resolve(args.get(1).ok_or(USAGE)?)?;
            call("forget", &[Value::Str(a.clone())])?;
            println!("Device {a} removed.");
            Ok(())
        }
        Some("power") => {
            let on = match args.get(1).map(String::as_str) {
                Some("on") => true,
                Some("off") => false,
                _ => return Err(USAGE.into()),
            };
            call("set_powered", &[Value::Bool(on)])?;
            println!("Bluetooth {}.", if on { "on" } else { "off" });
            Ok(())
        }
        Some(other) => Err(format!("unknown command '{other}'\n{USAGE}")),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        println!("bluetoothctl: {e}");
        std::process::exit(1);
    }
}
