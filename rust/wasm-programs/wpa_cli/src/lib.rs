//! `wpa_cli`: talks to the running `wpa_supplicant` (its status file and command
//! file under `/run`), edits `/etc/wpa_supplicant.conf`, and asks the kernel's
//! `wlan0` for scans. Everything that decides output lives here so it is
//! host-testable; `main.rs` only does the I/O.

use ecm_host_abi::wifi::ScanEntry;
use wpa_supplicant::conf::{self, Conf, ConfNetwork};

/// Parsed command line: `wpa_cli [-i <ifname>] [-c <conf>] <command> [args...]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    pub ifname: String,
    pub conf: String,
    pub command: String,
    pub args: Vec<String>,
}

pub fn usage() -> &'static str {
    "usage: wpa_cli [-i wlan0] [-c /etc/wpa_supplicant.conf] <command> [args]\n\
     commands:\n\
     \x20 status                       connection state\n\
     \x20 scan / scan_results          scan and list access points\n\
     \x20 list_networks                configured networks\n\
     \x20 add_network                  new (disabled) network, prints its id\n\
     \x20 set_network <id> <var> <val> ssid, psk, key_mgmt, bssid, priority\n\
     \x20 enable_network <id|all>      / disable_network / remove_network / select_network\n\
     \x20 save_config                  (changes are saved immediately)\n\
     \x20 reconfigure                  reload the configuration\n\
     \x20 disconnect / reconnect / reassociate / terminate\n"
}

pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut a = Args { ifname: "wlan0".into(), conf: wpa_supplicant::DEFAULT_CONF.into(), command: String::new(), args: Vec::new() };
    let mut i = 0;
    while i < argv.len() && a.command.is_empty() {
        let s = argv[i].as_str();
        if s == "-i" || s == "-c" || s == "-p" {
            let v = argv.get(i + 1).ok_or_else(|| format!("option requires an argument -- '{}'", &s[1..]))?.clone();
            match s {
                "-i" => a.ifname = v,
                "-c" => a.conf = v,
                _ => {} // ctrl_interface path: accepted, unused
            }
            i += 2;
            continue;
        }
        if let Some(v) = s.strip_prefix("-i").filter(|v| !v.is_empty()) {
            a.ifname = v.to_string();
        } else if s.starts_with('-') {
            return Err(format!("unknown option '{}'", s));
        } else {
            a.command = s.to_string();
            a.args = argv[i + 1..].to_vec();
        }
        i += 1;
    }
    if a.command.is_empty() {
        return Err("no command given".into());
    }
    Ok(a)
}

/// `scan_results` table.
pub fn format_scan_results(entries: &[ScanEntry]) -> String {
    let mut s = String::from("bssid / frequency / signal level / flags / ssid\n");
    for e in entries {
        let flags = match e.security.as_str() {
            "wpa2-psk" => "[WPA2-PSK-CCMP][ESS]",
            "open" => "[ESS]",
            _ => "[WEP][ESS]",
        };
        s += &format!("{}\t{}\t{}\t{}\t{}\n", e.bssid, e.freq, e.signal_dbm, flags, e.ssid_str());
    }
    s
}

/// `list_networks` table; `current` is the id the daemon is connected with.
pub fn format_networks(c: &Conf, current: Option<usize>) -> String {
    let mut s = String::from("network id / ssid / bssid / flags\n");
    for (i, n) in c.networks.iter().enumerate() {
        let bssid = n.bssid.map(|b| ecm_wifi_mac(&b)).unwrap_or_else(|| "any".into());
        let flags = if current == Some(i) { "[CURRENT]" } else { n.flags() };
        s += &format!("{}\t{}\t{}\t{}\n", i, String::from_utf8_lossy(&n.ssid), bssid, flags);
    }
    s
}

fn ecm_wifi_mac(m: &[u8; 6]) -> String {
    format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", m[0], m[1], m[2], m[3], m[4], m[5])
}

/// Normalise a `set_network` value the way people type it at our shell: quoted
/// strings stay quoted; a bare ssid/psk that isn't hex is taken as text.
pub fn normalise_value(var: &str, v: &str) -> String {
    let v = v.trim();
    if v.starts_with('"') {
        return v.to_string();
    }
    let is_hex = !v.is_empty() && v.len() % 2 == 0 && v.bytes().all(|c| c.is_ascii_hexdigit());
    match var {
        "ssid" if !is_hex => format!("\"{}\"", v),
        "psk" if !(v.len() == 64 && is_hex) => format!("\"{}\"", v),
        _ => v.to_string(),
    }
}

/// What a configuration edit produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    /// Printed to the user.
    pub reply: String,
    /// The file must be written back.
    pub save: bool,
    /// Ask the daemon to reload (changes that apply at once in wpa_supplicant).
    pub reload: bool,
}

fn ok(save: bool, reload: bool) -> Edit {
    Edit { reply: "OK".into(), save, reload }
}

fn id_arg(c: &Conf, args: &[String]) -> Result<Option<usize>, String> {
    let a = args.first().ok_or("missing network id")?;
    if a == "all" {
        return Ok(None);
    }
    let id: usize = a.parse().map_err(|_| format!("bad network id '{}'", a))?;
    if id >= c.networks.len() {
        return Err(format!("network id {} does not exist", id));
    }
    Ok(Some(id))
}

/// Apply a configuration command to `c`. `Err` prints `FAIL` plus the reason.
pub fn edit(c: &mut Conf, cmd: &str, args: &[String]) -> Result<Edit, String> {
    match cmd {
        "add_network" => {
            c.networks.push(ConfNetwork { disabled: true, ..Default::default() });
            Ok(Edit { reply: (c.networks.len() - 1).to_string(), save: true, reload: false })
        }
        "set_network" => {
            if args.len() < 3 {
                return Err("usage: set_network <id> <variable> <value>".into());
            }
            let id = id_arg(c, args)?.ok_or("set_network needs one id")?;
            let value = normalise_value(&args[1], &args[2..].join(" "));
            conf::set_var(&mut c.networks[id], &args[1], &value)?;
            Ok(ok(true, false))
        }
        "get_network" => {
            if args.len() < 2 {
                return Err("usage: get_network <id> <variable>".into());
            }
            let id = id_arg(c, args)?.ok_or("get_network needs one id")?;
            let n = &c.networks[id];
            let reply = match args[1].as_str() {
                "ssid" => format!("\"{}\"", String::from_utf8_lossy(&n.ssid)),
                "psk" => "*".into(),
                "key_mgmt" => (if n.open { "NONE" } else { "WPA-PSK" }).into(),
                "priority" => n.priority.to_string(),
                "disabled" => (n.disabled as u8).to_string(),
                v => return Err(format!("unknown variable {}", v)),
            };
            Ok(Edit { reply, save: false, reload: false })
        }
        "enable_network" | "disable_network" => {
            let dis = cmd == "disable_network";
            let ids: Vec<usize> = match id_arg(c, args)? {
                Some(i) => vec![i],
                None => (0..c.networks.len()).collect(),
            };
            for i in ids {
                let n = &c.networks[i];
                if !dis && (n.ssid.is_empty() || (!n.open && n.psk.is_none())) {
                    return Err(format!("network {} is incomplete (set ssid and psk, or key_mgmt NONE)", i));
                }
                c.networks[i].disabled = dis;
            }
            Ok(ok(true, true))
        }
        "select_network" => {
            let id = id_arg(c, args)?.ok_or("select_network needs one id")?;
            let n = &c.networks[id];
            if n.ssid.is_empty() || (!n.open && n.psk.is_none()) {
                return Err(format!("network {} is incomplete", id));
            }
            for (i, n) in c.networks.iter_mut().enumerate() {
                n.disabled = i != id;
            }
            Ok(ok(true, true))
        }
        "remove_network" => {
            match id_arg(c, args)? {
                Some(i) => {
                    c.networks.remove(i);
                }
                None => c.networks.clear(),
            }
            Ok(ok(true, true))
        }
        "save_config" => Ok(ok(true, false)),
        _ => Err(format!("unknown command '{}'", cmd)),
    }
}

pub fn is_edit_command(cmd: &str) -> bool {
    matches!(
        cmd,
        "add_network" | "set_network" | "get_network" | "enable_network" | "disable_network" | "select_network" | "remove_network" | "save_config"
    )
}

/// The daemon's network id from its status file.
pub fn current_id(status: &str) -> Option<usize> {
    if ecm_host_abi::wifi::kv(status, "wpa_state") != Some("COMPLETED") {
        return None;
    }
    ecm_host_abi::wifi::kv(status, "id").and_then(|s| s.parse().ok())
}

/// `status` output: the daemon's file plus the interface address from the kernel.
pub fn format_status(daemon: &str, kernel: &str) -> String {
    let mut s = String::new();
    for l in daemon.lines() {
        if l.starts_with("completed=") || l.starts_with("last_event=") {
            continue;
        }
        s += l;
        s += "\n";
    }
    if ecm_host_abi::wifi::kv(&s, "address").is_none() {
        if let Some(m) = ecm_host_abi::wifi::kv(kernel, "mac") {
            s += &format!("address={}\n", m);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_args() {
        let a = parse_args(&v("-i wlan0 set_network 0 ssid ecm-lab")).unwrap();
        assert_eq!((a.ifname.as_str(), a.command.as_str()), ("wlan0", "set_network"));
        assert_eq!(a.args, v("0 ssid ecm-lab"));
        assert_eq!(parse_args(&v("-iwlan0 status")).unwrap().command, "status");
        assert_eq!(parse_args(&v("-c /tmp/x.conf list_networks")).unwrap().conf, "/tmp/x.conf");
        assert!(parse_args(&v("-i")).is_err());
        assert!(parse_args(&v("-z status")).is_err());
        assert!(parse_args(&[]).is_err());
    }

    #[test]
    fn add_set_enable_select_remove_networks() {
        let mut c = Conf::default();
        assert_eq!(edit(&mut c, "add_network", &[]).unwrap().reply, "0");
        assert!(edit(&mut c, "enable_network", &v("0")).is_err(), "incomplete network can't be enabled");
        edit(&mut c, "set_network", &v("0 ssid \"ecm-lab\"")).unwrap();
        edit(&mut c, "set_network", &v("0 psk correct horse battery")).unwrap();
        let e = edit(&mut c, "enable_network", &v("0")).unwrap();
        assert!(e.save && e.reload);
        assert_eq!(c.networks[0].ssid, b"ecm-lab");
        assert_eq!(c.networks[0].psk, Some(conf::Psk::Passphrase("correct horse battery".into())));
        assert!(!c.networks[0].disabled);
        edit(&mut c, "add_network", &[]).unwrap();
        edit(&mut c, "set_network", &v("1 ssid 636166")).unwrap();
        edit(&mut c, "set_network", &v("1 key_mgmt NONE")).unwrap();
        assert_eq!(c.networks[1].ssid, b"caf", "bare hex ssid");
        edit(&mut c, "select_network", &v("1")).unwrap();
        assert!(c.networks[0].disabled && !c.networks[1].disabled);
        assert_eq!(edit(&mut c, "get_network", &v("1 key_mgmt")).unwrap().reply, "NONE");
        assert!(edit(&mut c, "set_network", &v("0 psk short")).is_err());
        assert!(edit(&mut c, "set_network", &v("7 ssid x")).is_err());
        assert!(edit(&mut c, "frob", &[]).is_err());
        // The edited file parses back to the same thing.
        let text = conf::render(&c);
        assert_eq!(conf::parse(&text).unwrap(), c);
        let t = format_networks(&c, Some(1));
        assert_eq!(t, "network id / ssid / bssid / flags\n0\tecm-lab\tany\t[DISABLED]\n1\tcaf\tany\t[CURRENT]\n");
        edit(&mut c, "remove_network", &v("all")).unwrap();
        assert!(c.networks.is_empty());
    }

    #[test]
    fn formats_scan_results_and_status() {
        let e = ecm_host_abi::wifi::ScanEntry::parse_all("02:aa:00:00:00:01 2437 -48 wpa2-psk 6 65636d2d6c6162\n02:aa:00:00:00:02 2412 -70 open 1 636166\n");
        assert_eq!(
            format_scan_results(&e),
            "bssid / frequency / signal level / flags / ssid\n02:aa:00:00:00:01\t2437\t-48\t[WPA2-PSK-CCMP][ESS]\tecm-lab\n02:aa:00:00:00:02\t2412\t-70\t[ESS]\tcaf\n"
        );
        let daemon = "wpa_state=COMPLETED\naddress=02:ec:00:00:00:42\nbssid=02:aa:00:00:00:01\nid=0\ncompleted=1\nlast_event=x\n";
        assert_eq!(current_id(daemon), Some(0));
        assert_eq!(current_id("wpa_state=SCANNING\nid=0\n"), None);
        let s = format_status(daemon, "mac=02:ec:00:00:00:42\n");
        assert!(s.contains("wpa_state=COMPLETED") && !s.contains("last_event"));
        assert!(format_status("wpa_state=SCANNING\n", "mac=02:ec:00:00:00:42\n").contains("address=02:ec:00:00:00:42"));
    }

    #[test]
    fn normalises_unquoted_values() {
        assert_eq!(normalise_value("ssid", "ecm-lab"), "\"ecm-lab\"");
        assert_eq!(normalise_value("ssid", "636166"), "636166");
        assert_eq!(normalise_value("ssid", "\"x y\""), "\"x y\"");
        assert_eq!(normalise_value("psk", "hunter22hunter"), "\"hunter22hunter\"");
        assert_eq!(normalise_value("priority", "5"), "5");
    }
}
