//! `cell` on a v1 modem engine (`lte_modem` / `mmwave_modem` peripherals,
//! server config `modem.framedRadio = false`): the modem attaches by itself
//! when it has a SIM for a carrier whose Cell Tower it hears; the kernel then
//! has carrier on `wwan0` (configure it like eth0: `ifconfig wwan0 dhcp`).
//! Cellular v2 phase 4 deletes this path with the v1 engine.

use ecm_host_abi::peripheral::{self, Error, Value};

pub const TYPES: [&str; 2] = ["lte_modem", "mmwave_modem"];

fn describe(e: Error) -> String {
    match e {
        Error::Peripheral(msg) => msg,
        Error::Unavailable => "peripherals are not available".to_string(),
        Error::Malformed(what) => format!("bad reply from the computer: {what}"),
    }
}

fn get<'a>(m: &'a Value, key: &str) -> Option<&'a Value> {
    match m {
        Value::Map(entries) => entries.iter().find(|(k, _)| matches!(k, Value::Str(s) if s == key)).map(|(_, v)| v),
        _ => None,
    }
}

fn text(m: &Value, key: &str) -> String {
    match get(m, key) {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Int(i)) => i.to_string(),
        Some(Value::Float(f)) => {
            if f.fract() == 0.0 && f.abs() < 1e12 { format!("{}", *f as i64) } else { format!("{:.1}", f) }
        }
        Some(Value::Bool(b)) => b.to_string(),
        _ => "-".to_string(),
    }
}

fn num(m: &Value, key: &str) -> Option<f64> {
    match get(m, key) {
        Some(Value::Int(i)) => Some(*i as f64),
        Some(Value::Float(f)) => Some(*f),
        _ => None,
    }
}

pub fn no_modem() -> ! {
    eprintln!("cell: this computer has no cellular modem.");
    eprintln!("Fit an LTE Modem or mmWave Modem in a module bay (right-click the computer");
    eprintln!("with a Module Expansion Card, then the modem), and give it a SIM card");
    eprintln!("(right-click the modem's bay with the SIM; an ePhone has a SIM tray).");
    std::process::exit(1);
}

fn status(name: &str) -> Result<(), Error> {
    let s = peripheral::call(name, "status", &[])?;
    println!("{}: {} modem {}", name, text(&s, "tech"), text(&s, "mac"));
    println!("  state     {} - {}", text(&s, "state"), text(&s, "message"));
    match get(&s, "imsi") {
        Some(Value::Str(imsi)) => println!("  SIM       {}, IMSI {}", text(&s, "sim_carrier"), imsi),
        _ => println!("  SIM       none"),
    }
    if matches!(get(&s, "tower"), Some(Value::Str(_))) {
        println!("  tower     {} ({}), channel {} ({} MHz)", text(&s, "carrier"), &text(&s, "tower")[..8.min(text(&s, "tower").len())],
            text(&s, "channel"), text(&s, "frequency_mhz"));
        let bars = num(&s, "bars").unwrap_or(0.0) as usize;
        println!("  signal    {} dBm, SINR {} dB  [{}{}]", text(&s, "rssi_dbm"), text(&s, "sinr_db"), "*".repeat(bars), " ".repeat(4 - bars.min(4)));
        println!("  downlink  CQI {}, {} Mbit/s", text(&s, "dl_cqi"), text(&s, "dl_mbps"));
        println!("  uplink    CQI {}, {} Mbit/s (slice {})", text(&s, "ul_cqi"), text(&s, "ul_mbps"), text(&s, "slice"));
    } else {
        println!("  scanning  channel {} ({} MHz)", text(&s, "channel"), text(&s, "frequency_mhz"));
    }
    println!("  traffic   tx {} B, rx {} B, handovers {}", text(&s, "tx_bytes"), text(&s, "rx_bytes"), text(&s, "handovers"));
    Ok(())
}

fn scan(name: &str) -> Result<(), Error> {
    let cells = peripheral::call(name, "cells", &[])?;
    let Value::List(list) = cells else { return Err(Error::Malformed("cells is not a list")) };
    if list.is_empty() {
        println!("{name}: no towers heard (yet). The modem scans one channel at a time; try again in a few seconds.");
        return Ok(());
    }
    println!("{:<24} {:<7} {:>3} {:>8} {:>7} {:>5}  TOWER", "CARRIER", "TECH", "CH", "RSSI", "SINR", "LOAD");
    for c in &list {
        println!("{:<24} {:<7} {:>3} {:>8} {:>7} {:>5}  {}", text(c, "carrier"), text(c, "tech"), text(c, "channel"),
            text(c, "rssi_dbm"), text(c, "sinr_db"), text(c, "load"), text(c, "cell"));
    }
    Ok(())
}

/// The v1 pages for `cmd` on `list` (the v1 modems).
pub fn run(cmd: &cell::Cmd, list: &[(String, String)]) {
    let r = match cmd {
        cell::Cmd::Status => list.iter().try_for_each(|(n, _)| status(n)),
        cell::Cmd::Scan => list.iter().try_for_each(|(n, _)| scan(n)),
        _ => {
            eprintln!("cell: this modem runs the v1 engine, which attaches by itself (no up/down/roaming)");
            std::process::exit(2);
        }
    };
    if let Err(e) = r {
        eprintln!("cell: {}", describe(e));
        std::process::exit(1);
    }
}
