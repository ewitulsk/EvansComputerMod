//! `iw`: show / manipulate the Wi-Fi interface through the kernel's `wlan0`
//! control channel. Parsing and output formatting are here (host-testable).

use ecm_host_abi::wifi::{kv, ScanEntry};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// `iw dev`
    ListDevs,
    /// `iw dev wlan0 info`
    Info,
    /// `iw dev wlan0 scan [passive] [ssid <name>]`; `dump` = cached results only.
    Scan { passive: bool, ssid: Option<String>, dump: bool },
    /// `iw dev wlan0 link`
    Link,
    /// `iw dev wlan0 station dump`
    StationDump,
    /// `iw dev wlan0 set type monitor|managed`
    SetType(String),
    /// `iw dev wlan0 set channel <n>` / `set freq <mhz>`
    SetChannel(u8),
    /// `iw dev wlan0 set txpower fixed <mBm>`
    SetTxPower(i32),
    /// `iw dev wlan0 set keepalive <ms>|off` (ECM extension: Null data keep-alive interval)
    SetKeepalive(u64),
    /// `iw dev wlan0 connect <ssid> [bssid]` (open networks; WPA2 needs wpa_supplicant)
    Connect { ssid: String, bssid: Option<String> },
    /// `iw dev wlan0 disconnect`
    Disconnect,
    Help,
}

pub fn usage() -> &'static str {
    "Usage: iw dev [<devname> <command>]\n\
     \tdev                                list wireless interfaces\n\
     \tdev wlan0 info                     interface details\n\
     \tdev wlan0 scan [passive] [ssid S]  scan for access points (scan dump: cached)\n\
     \tdev wlan0 link                     current link: signal and bitrate\n\
     \tdev wlan0 station dump             counters\n\
     \tdev wlan0 set type monitor|managed\n\
     \tdev wlan0 set channel <1-13|36-165>\n\
     \tdev wlan0 set txpower fixed <mBm>\n\
     \tdev wlan0 set keepalive <ms>|off  keep-alive while idle (default 30000)\n\
     \tdev wlan0 connect <ssid> [bssid]   join an open network\n\
     \tdev wlan0 disconnect\n"
}

/// Freq (MHz) → channel number.
pub fn freq_to_channel(f: u32) -> Option<u8> {
    match f {
        2484 => Some(14),
        2412..=2472 if (f - 2407) % 5 == 0 => Some(((f - 2407) / 5) as u8),
        5160..=5885 if (f - 5000) % 5 == 0 => Some(((f - 5000) / 5) as u8),
        _ => None,
    }
}

pub fn parse(argv: &[String]) -> Result<(String, Cmd), String> {
    let a: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
    match a.as_slice() {
        [] | ["help"] | ["--help"] => Ok((String::new(), Cmd::Help)),
        ["dev"] => Ok((String::new(), Cmd::ListDevs)),
        ["dev", dev, rest @ ..] => {
            let dev = dev.to_string();
            let cmd = match rest {
                ["info"] => Cmd::Info,
                ["link"] => Cmd::Link,
                ["station", "dump"] => Cmd::StationDump,
                ["disconnect"] => Cmd::Disconnect,
                ["scan", "dump"] => Cmd::Scan { passive: false, ssid: None, dump: true },
                ["scan", opts @ ..] => {
                    let mut passive = false;
                    let mut ssid = None;
                    let mut i = 0;
                    while i < opts.len() {
                        match opts[i] {
                            "passive" => passive = true,
                            "ssid" => {
                                ssid = Some(opts.get(i + 1).ok_or("scan ssid needs a name")?.to_string());
                                i += 1;
                            }
                            o => return Err(format!("unknown scan option '{}'", o)),
                        }
                        i += 1;
                    }
                    Cmd::Scan { passive, ssid, dump: false }
                }
                ["set", "type", t] => match *t {
                    "monitor" | "managed" | "station" => Cmd::SetType(if *t == "station" { "managed".into() } else { t.to_string() }),
                    _ => return Err(format!("unsupported interface type '{}' (monitor or managed)", t)),
                },
                ["set", "channel", n, ..] => Cmd::SetChannel(n.parse().map_err(|_| format!("bad channel '{}'", n))?),
                ["set", "freq", f, ..] => {
                    let f: u32 = f.parse().map_err(|_| format!("bad frequency '{}'", f))?;
                    Cmd::SetChannel(freq_to_channel(f).ok_or_else(|| format!("no channel at {} MHz", f))?)
                }
                ["set", "txpower", "fixed", mbm] => Cmd::SetTxPower(mbm.parse().map_err(|_| format!("bad power '{}'", mbm))?),
                ["set", "txpower", "auto"] => Cmd::SetTxPower(2000),
                ["set", "keepalive", "off"] => Cmd::SetKeepalive(0),
                ["set", "keepalive", ms] => Cmd::SetKeepalive(ms.parse().map_err(|_| format!("bad interval '{}'", ms))?),
                ["connect", ssid, rest @ ..] => Cmd::Connect { ssid: ssid.to_string(), bssid: rest.first().map(|b| b.to_string()) },
                _ => return Err(format!("command failed: unknown command '{}'", rest.join(" "))),
            };
            Ok((dev, cmd))
        }
        _ => Err("command failed: try 'iw dev'".into()),
    }
}

fn security_lines(sec: &str) -> &'static str {
    match sec {
        "wpa2-psk" => "\tRSN:\t * Version: 1\n\t\t * Group cipher: CCMP\n\t\t * Pairwise ciphers: CCMP\n\t\t * Authentication suites: PSK\n",
        "open" => "",
        _ => "\tcapability: ESS Privacy\n",
    }
}

/// `iw dev wlan0 scan` output (strongest first, as the kernel sorts it).
pub fn format_scan(dev: &str, entries: &[ScanEntry], associated: Option<&str>) -> String {
    let mut s = String::new();
    for e in entries {
        let assoc = if associated == Some(e.bssid.as_str()) { " -- associated" } else { "" };
        s += &format!("BSS {}(on {}){}\n", e.bssid, dev, assoc);
        s += &format!("\tfreq: {}\n", e.freq);
        s += &format!("\tsignal: {}.00 dBm\n", e.signal_dbm);
        if e.ssid.is_empty() {
            s += "\tSSID: \n";
        } else {
            s += &format!("\tSSID: {}\n", e.ssid_str());
        }
        s += &format!("\tDS Parameter set: channel {}\n", e.channel);
        s += security_lines(&e.security);
    }
    s
}

/// `iw dev` / `iw dev wlan0 info` from the kernel's `status`.
pub fn format_info(status: &str, header: bool) -> String {
    let mac = kv(status, "mac").unwrap_or("00:00:00:00:00:00");
    let mode = kv(status, "mode").unwrap_or("managed");
    let ch = kv(status, "channel").unwrap_or("1");
    let freq = kv(status, "freq").unwrap_or("2412");
    let mut s = String::new();
    if header {
        s += "phy#0\n";
    }
    let ind = if header { "\t" } else { "" };
    s += &format!("{}Interface {}\n", ind, kv(status, "ifname").unwrap_or("wlan0"));
    s += &format!("{}\tifindex 0\n{}\twdev 0x1\n", ind, ind);
    s += &format!("{}\taddr {}\n", ind, mac);
    if let Some(ssid) = kv(status, "ssid").and_then(ecm_host_abi::wifi::unhex) {
        s += &format!("{}\tssid {}\n", ind, String::from_utf8_lossy(&ssid));
    }
    s += &format!("{}\ttype {}\n", ind, mode);
    s += &format!("{}\tchannel {} ({} MHz), width: 20 MHz\n", ind, ch, freq);
    if let Some(p) = kv(status, "tx_power_dbm") {
        s += &format!("{}\ttxpower {}.00 dBm\n", ind, p);
    }
    s
}

/// `iw dev wlan0 station dump` from `link` + `stats`.
pub fn format_station(link: &str, stats: &str, dev: &str) -> String {
    let Some(first) = link.lines().next().filter(|l| l.starts_with("Connected to ")) else {
        return String::new();
    };
    let bssid = first.trim_start_matches("Connected to ").split(' ').next().unwrap_or("");
    let field = |name: &str| link.lines().find_map(|l| l.trim().strip_prefix(name).map(|v| v.trim().to_string()));
    let mut s = format!("Station {} (on {})\n", bssid, dev);
    for k in ["rx_bytes", "rx_packets", "tx_bytes", "tx_packets", "tx_retries", "tx_failed"] {
        if let Some(v) = kv(stats, k) {
            s += &format!("\t{}:\t{}\n", k.replace('_', " "), v);
        }
    }
    if let Some(sig) = field("signal:") {
        s += &format!("\tsignal:  \t{}\n", sig);
    }
    if let Some(r) = field("tx bitrate:") {
        s += &format!("\ttx bitrate:\t{}\n", r);
    }
    if let Some(r) = field("rx bitrate:") {
        s += &format!("\trx bitrate:\t{}\n", r);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<(String, Cmd), String> {
        parse(&s.split_whitespace().map(String::from).collect::<Vec<_>>())
    }

    #[test]
    fn parses_commands() {
        assert_eq!(p("dev").unwrap().1, Cmd::ListDevs);
        assert_eq!(p("dev wlan0 scan").unwrap(), ("wlan0".into(), Cmd::Scan { passive: false, ssid: None, dump: false }));
        assert_eq!(p("dev wlan0 scan passive ssid hidden-net").unwrap().1, Cmd::Scan { passive: true, ssid: Some("hidden-net".into()), dump: false });
        assert_eq!(p("dev wlan0 scan dump").unwrap().1, Cmd::Scan { passive: false, ssid: None, dump: true });
        assert_eq!(p("dev wlan0 link").unwrap().1, Cmd::Link);
        assert_eq!(p("dev wlan0 set type monitor").unwrap().1, Cmd::SetType("monitor".into()));
        assert_eq!(p("dev wlan0 set type station").unwrap().1, Cmd::SetType("managed".into()));
        assert_eq!(p("dev wlan0 set channel 11").unwrap().1, Cmd::SetChannel(11));
        assert_eq!(p("dev wlan0 set channel 6 HT20").unwrap().1, Cmd::SetChannel(6));
        assert_eq!(p("dev wlan0 set freq 5180").unwrap().1, Cmd::SetChannel(36));
        assert_eq!(p("dev wlan0 set txpower fixed 1500").unwrap().1, Cmd::SetTxPower(1500));
        assert_eq!(p("dev wlan0 connect cafe").unwrap().1, Cmd::Connect { ssid: "cafe".into(), bssid: None });
        assert_eq!(p("dev wlan0 station dump").unwrap().1, Cmd::StationDump);
        assert_eq!(p("dev wlan0 set keepalive 1000").unwrap().1, Cmd::SetKeepalive(1000));
        assert_eq!(p("dev wlan0 set keepalive off").unwrap().1, Cmd::SetKeepalive(0));
        assert!(p("dev wlan0 set keepalive soon").is_err());
        assert!(p("dev wlan0 set type ibss").is_err());
        assert!(p("dev wlan0 set channel x").is_err());
        assert!(p("dev wlan0 set freq 1234").is_err());
        assert!(p("dev wlan0 scan ssid").is_err());
        assert!(p("dev wlan0 frob").is_err());
        assert!(p("phy").is_err());
        assert_eq!(p("").unwrap().1, Cmd::Help);
    }

    #[test]
    fn formats_scan_like_iw() {
        let e = ScanEntry::parse_all("02:aa:00:00:00:01 2437 -48 wpa2-psk 6 65636d2d6c6162\n02:aa:00:00:00:02 2412 -70 open 1 -\n");
        let s = format_scan("wlan0", &e, Some("02:aa:00:00:00:01"));
        assert!(s.starts_with("BSS 02:aa:00:00:00:01(on wlan0) -- associated\n\tfreq: 2437\n\tsignal: -48.00 dBm\n\tSSID: ecm-lab\n\tDS Parameter set: channel 6\n\tRSN:"), "{s}");
        assert!(s.contains("BSS 02:aa:00:00:00:02(on wlan0)\n\tfreq: 2412\n\tsignal: -70.00 dBm\n\tSSID: \n"), "{s}");
        assert_eq!(format_scan("wlan0", &[], None), "");
    }

    #[test]
    fn formats_info_and_station() {
        let st = "present=1\nifname=wlan0\nmac=02:ec:00:00:00:42\nmode=monitor\nchannel=6\nfreq=2437\ntx_power_dbm=20\n";
        let s = format_info(st, true);
        assert!(s.starts_with("phy#0\n\tInterface wlan0\n") && s.contains("\t\taddr 02:ec:00:00:00:42\n") && s.contains("\t\ttype monitor\n") && s.contains("channel 6 (2437 MHz)"), "{s}");
        let link = "Connected to 02:aa:00:00:00:01 (on wlan0)\n\tSSID: ecm-lab\n\tfreq: 2437\n\tsignal: -48 dBm\n\ttx bitrate: 65.0 MBit/s MCS 7\n";
        let sd = format_station(link, "rx_packets=3\ntx_packets=4\ntx_retries=1\n", "wlan0");
        assert!(sd.starts_with("Station 02:aa:00:00:00:01 (on wlan0)\n") && sd.contains("tx retries:\t1") && sd.contains("signal:  \t-48 dBm") && sd.contains("tx bitrate:\t65.0 MBit/s MCS 7"), "{sd}");
        assert_eq!(format_station("Not connected.\n", "", "wlan0"), "");
    }
}
