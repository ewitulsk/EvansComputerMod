//! `wpa_supplicant -B -i wlan0 -c /etc/wpa_supplicant.conf`
//!
//! EAPOL transport: an `AF_PACKET` socket for EtherType 0x888E bound to the
//! interface when the kernel has packet sockets (`-D packet` forces it), otherwise
//! the kernel's EAPOL tap on the Wi-Fi control channel (`-D ctl`).

use std::io::Write;

use ecm_host_abi::socket::{self, SOCK_RAW, SOL_SOCKET, SO_RCVTIMEO};
use ecm_host_abi::wifi::{self, kv};
use wpa_supplicant::{cmd_path, conf, files, parse_args, parse_mac, status_path, usage, Driver, Port};

#[link(wasm_import_module = "env")]
extern "C" {
    fn get_time_ms() -> i64;
}

const AF_PACKET: i32 = 17;
const ETH_P_EAPOL: u16 = 0x888e;
const SO_BINDTODEVICE: i32 = 25;

enum Eapol {
    Packet(i32),
    Ctl,
}

struct HostPort {
    eapol: Eapol,
    quiet: bool,
    debug: bool,
    log_file: Option<String>,
}

impl HostPort {
    fn use_ctl(&mut self) {
        if let Eapol::Packet(fd) = self.eapol {
            socket::close(fd);
        }
        self.eapol = Eapol::Ctl;
        wifi::ctl("eapol_subscribe");
    }
}

impl Port for HostPort {
    fn ctl(&mut self, req: &str) -> Option<(i32, Vec<u8>)> {
        let r = wifi::ctl(req);
        if self.debug && !req.starts_with("events") {
            let st = r.as_ref().map(|x| x.0).unwrap_or(-1);
            self.log(&format!("ctl '{}' -> {}", req.split(' ').next().unwrap_or(""), st));
        }
        r
    }

    fn eapol_send(&mut self, eth: &[u8]) -> bool {
        if let Eapol::Packet(fd) = self.eapol {
            if socket::send(fd, eth, 0) == eth.len() as i32 {
                return true;
            }
            self.log("EAPOL: packet socket send failed; using the kernel EAPOL tap");
            self.use_ctl();
        }
        matches!(wifi::ctl(&format!("eapol_tx {}", wifi::hex(eth))), Some((0, _)))
    }

    fn eapol_recv(&mut self) -> Option<Vec<u8>> {
        match self.eapol {
            Eapol::Packet(fd) => {
                let mut buf = vec![0u8; 2048];
                let n = socket::recv(fd, &mut buf, 0);
                if n > 14 {
                    buf.truncate(n as usize);
                    Some(buf)
                } else {
                    None
                }
            }
            Eapol::Ctl => match wifi::ctl("eapol_rx") {
                Some((1, f)) => Some(f),
                _ => None,
            },
        }
    }

    fn now_ms(&mut self) -> u64 {
        unsafe { get_time_ms() }.max(0) as u64
    }

    fn log(&mut self, line: &str) {
        if let Some(path) = &self.log_file {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(files::rel(path)) {
                let _ = writeln!(f, "{}", line);
            }
        }
        if !self.quiet {
            println!("{}", line);
        }
    }
}

fn open_packet_socket(ifname: &str) -> Option<i32> {
    let fd = socket::socket(AF_PACKET, SOCK_RAW, ETH_P_EAPOL.to_be() as i32);
    if fd < 0 {
        return None;
    }
    if socket::setsockopt(fd, SOL_SOCKET, SO_BINDTODEVICE, ifname.as_bytes()) < 0 {
        socket::close(fd);
        return None;
    }
    // Poll, don't block: the main loop also watches kernel events.
    socket::setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &5i32.to_le_bytes());
    Some(fd)
}

fn load_networks(path: &str, port: &mut HostPort) -> Option<Vec<ecm_wifi::supplicant::NetworkConfig>> {
    let text = match files::read(path) {
        Some(t) => t,
        None => {
            eprintln!("Failed to read configuration file '{}'", path);
            return None;
        }
    };
    let c = match conf::parse(&text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}: {}", path, e);
            return None;
        }
    };
    let mut nets = Vec::new();
    for (i, n) in c.networks.iter().enumerate() {
        match n.to_network_config() {
            Ok(x) => nets.push(x),
            Err(e) => port.log(&format!("network {}: {} (skipped)", i, e)),
        }
    }
    Some(nets)
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("wpa_supplicant: {}\n{}", e, usage());
            std::process::exit(1);
        }
    };
    if args.ifname != "wlan0" {
        eprintln!("wpa_supplicant: {}: no such wireless interface", args.ifname);
        std::process::exit(1);
    }
    let mut port = HostPort { eapol: Eapol::Ctl, quiet: args.background && args.log_file.is_none(), debug: args.debug, log_file: args.log_file.clone() };
    let Some(nets) = load_networks(&args.conf, &mut port) else { std::process::exit(1) };

    let status = wifi::ctl("status").map(|(_, r)| String::from_utf8_lossy(&r).into_owned()).unwrap_or_default();
    let Some(own) = kv(&status, "mac").and_then(parse_mac) else {
        eprintln!("wpa_supplicant: {}: no Wi-Fi module installed", args.ifname);
        std::process::exit(1);
    };

    port.eapol = match args.driver.as_deref() {
        Some("ctl") => Eapol::Ctl,
        Some("packet") => match open_packet_socket(&args.ifname) {
            Some(fd) => Eapol::Packet(fd),
            None => {
                eprintln!("wpa_supplicant: no AF_PACKET support in this kernel");
                std::process::exit(1);
            }
        },
        _ => open_packet_socket(&args.ifname).map(Eapol::Packet).unwrap_or(Eapol::Ctl),
    };
    match port.eapol {
        Eapol::Ctl => {
            wifi::ctl("eapol_subscribe");
            if args.debug {
                port.log("EAPOL transport: kernel EAPOL tap");
            }
        }
        Eapol::Packet(_) => {
            if args.debug {
                port.log("EAPOL transport: AF_PACKET 0x888e");
            }
        }
    }

    let rand = Box::new(|b: &mut [u8]| {
        ecm_host_abi::random::getrandom(b);
    });
    let mut d = Driver::new(own, &args.ifname, rand, nets);
    if let Err(e) = d.attach(&mut port) {
        eprintln!("wpa_supplicant: {}", e);
        std::process::exit(1);
    }
    let sp = status_path(&args.ifname);
    let cp = cmd_path(&args.ifname);
    files::delete(&cp);
    port.log(&format!("Successfully initialized wpa_supplicant ({} networks)", d.supp.networks().len()));

    loop {
        if files::exists(&cp) {
            let cmds = files::read(&cp).unwrap_or_default();
            files::delete(&cp);
            for c in cmds.lines() {
                match c.trim() {
                    "reconfigure" => {
                        if let Some(n) = load_networks(&args.conf, &mut port) {
                            d.set_networks(&mut port, n);
                        }
                    }
                    "terminate" => {
                        wifi::ctl("disconnect");
                        wifi::ctl("eapol_unsubscribe");
                        files::write(&sp, "wpa_state=INTERFACE_DISABLED\n");
                        port.log("CTRL-EVENT-TERMINATING");
                        return;
                    }
                    other => d.command(&mut port, other),
                }
            }
        }
        d.step(&mut port);
        if let Some(s) = d.take_status() {
            files::write(&sp, &s);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
