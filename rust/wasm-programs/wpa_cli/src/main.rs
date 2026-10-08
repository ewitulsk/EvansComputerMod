//! `wpa_cli [-i wlan0] <command>` — see `lib.rs`.

use ecm_host_abi::fs;
use ecm_host_abi::wifi::{self, ScanEntry};
use wpa_cli::{current_id, edit, format_networks, format_scan_results, format_status, is_edit_command, parse_args, usage};
use wpa_supplicant::{cmd_path, conf, status_path};

fn fail(msg: &str) -> ! {
    println!("FAIL");
    if !msg.is_empty() {
        eprintln!("wpa_cli: {}", msg);
    }
    std::process::exit(1);
}

fn daemon_status(ifname: &str) -> Option<String> {
    fs::read_file(&status_path(ifname))
}

/// Queue a command for the running daemon.
fn send_daemon(ifname: &str, cmd: &str) {
    if daemon_status(ifname).is_none() {
        eprintln!("Failed to connect to non-global ctrl_ifname: {}  error: No such file or directory", ifname);
        std::process::exit(1);
    }
    let path = cmd_path(ifname);
    let mut cur = fs::read_file(&path).unwrap_or_default();
    cur += cmd;
    cur += "\n";
    fs::write_file(&path, &cur);
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let a = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("wpa_cli: {}\n{}", e, usage());
            std::process::exit(1);
        }
    };
    let kernel = wifi::ctl("status").map(|(_, r)| String::from_utf8_lossy(&r).into_owned()).unwrap_or_default();
    match a.command.as_str() {
        "status" => match daemon_status(&a.ifname) {
            Some(d) => print!("{}", format_status(&d, &kernel)),
            None => {
                eprintln!("Failed to connect to non-global ctrl_ifname: {}  error: No such file or directory", a.ifname);
                std::process::exit(1);
            }
        },
        "scan" => match wifi::ctl("scan") {
            Some((0, _)) => println!("OK"),
            Some((wifi::EBUSY, _)) => println!("FAIL-BUSY"),
            Some((_, r)) => fail(&String::from_utf8_lossy(&r)),
            None => fail("no Wi-Fi control channel"),
        },
        "scan_results" => match wifi::ctl("scan_results") {
            Some((0, r)) => print!("{}", format_scan_results(&ScanEntry::parse_all(&String::from_utf8_lossy(&r)))),
            Some((_, r)) => fail(&String::from_utf8_lossy(&r)),
            None => fail("no Wi-Fi control channel"),
        },
        "list_networks" => {
            let c = fs::read_file(&a.conf).map(|t| conf::parse(&t)).unwrap_or(Ok(conf::Conf::default()));
            match c {
                Ok(c) => print!("{}", format_networks(&c, daemon_status(&a.ifname).as_deref().and_then(current_id))),
                Err(e) => fail(&format!("{}: {}", a.conf, e)),
            }
        }
        "reconfigure" | "disconnect" | "reconnect" | "reassociate" | "terminate" => {
            send_daemon(&a.ifname, &a.command);
            println!("OK");
        }
        cmd if is_edit_command(cmd) => {
            let mut c = match fs::read_file(&a.conf).map(|t| conf::parse(&t)) {
                Some(Ok(c)) => c,
                Some(Err(e)) => fail(&format!("{}: {}", a.conf, e)),
                None => conf::Conf::default(),
            };
            match edit(&mut c, cmd, &a.args) {
                Ok(e) => {
                    if e.save {
                        if let Some(dir) = a.conf.rsplit_once('/').map(|(d, _)| d).filter(|d| !d.is_empty()) {
                            fs::mkdir(dir);
                        }
                        if fs::write_file(&a.conf, &conf::render(&c)) < 0 {
                            fail(&format!("cannot write {}", a.conf));
                        }
                    }
                    if e.reload && daemon_status(&a.ifname).is_some() {
                        send_daemon(&a.ifname, "reconfigure");
                    }
                    println!("{}", e.reply);
                }
                Err(m) => fail(&m),
            }
        }
        "help" => print!("{}", usage()),
        other => {
            eprintln!("Unknown command '{}'\n{}", other, usage());
            std::process::exit(1);
        }
    }
}
