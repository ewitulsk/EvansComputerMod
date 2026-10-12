//! The program: CLI and the daemon loop (WASI only).

use celld_site::config::{self, Conf, KEYS};
use celld_site::daemon::{Action, Daemon};
use ecm_cell::codec::fmt_ip;
use ecm_cell::phy::{self, FramedPhy};
use ecm_cell::site::SiteOut;
use ecm_cell::Now;
use ecm_cell_net::{admin_call, readable, read_now, send_all, tcp_connect, udp_bind, AdminServer, Read, WasiEntropy};
use ecm_host_abi::socket::{self, SockAddrIn};
use ecm_radio::framed::Framed;
use std::time::Instant;

/// The line `celld-site enable` adds to services.cfg: at boot the SDR may attach later than the
/// program starts (a block peripheral attaches once its chunk ticks), so the site waits for it.
const SERVICE_LINE: &str = "celld-site start --wait &";
/// The line older versions wrote (still removed by `disable`).
const OLD_SERVICE_LINE: &str = "celld-site start &";

fn fail(msg: impl AsRef<str>) -> ! {
    println!("celld-site: {}", msg.as_ref());
    std::process::exit(1);
}

fn usage() -> ! {
    println!("usage: celld-site start [--wait]             run the site (background it: celld-site start &);");
    println!("                                             --wait: wait for the framed radio instead of failing");
    println!("       celld-site stop | restart | status | log [n]");
    println!("       celld-site config                     show {}", config::PATH);
    println!("       celld-site config set <key> <value> [<key> <value>...]");
    println!("       celld-site config keys");
    println!("       celld-site enable | disable           start at boot");
    std::process::exit(2);
}

fn load() -> Result<Conf, String> {
    match std::fs::read_to_string(config::PATH) {
        Ok(t) => Conf::parse(&t).map_err(|e| format!("{}: {}", config::PATH, e)),
        Err(_) => Ok(Conf::default()),
    }
}

fn save(text: &str) -> Result<(), String> {
    let _ = std::fs::create_dir_all("/etc");
    std::fs::write(config::PATH, text).map_err(|e| format!("can't write {}: {e}", config::PATH))
}

fn admin_port() -> u16 {
    load().ok().and_then(|c| c.get("admin_port").parse().ok()).unwrap_or(config::ADMIN_PORT)
}

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["start"] => run(false),
        ["start", "--wait"] => run(true),
        ["stop"] | ["restart"] => match admin_call(admin_port(), a[0]) {
            Ok(b) => println!("celld-site: {}", b.join(" ")),
            Err(e) => fail(e),
        },
        ["status"] => match admin_call(admin_port(), "status") {
            Ok(b) => {
                for l in b {
                    println!("{l}");
                }
            }
            Err(e) if e == "not running" => {
                println!("celld-site: not running (start it with: celld-site start &)");
                std::process::exit(1);
            }
            Err(e) => fail(e),
        },
        ["log"] | ["log", _] => match admin_call(admin_port(), &a.join(" ")) {
            Ok(b) => {
                for l in b {
                    println!("{l}");
                }
            }
            Err(e) => fail(e),
        },
        ["config"] => {
            let c = load().unwrap_or_else(|e| fail(e));
            if c == Conf::default() {
                println!("celld-site: no settings yet; set them with: celld-site config set <key> <value> (celld-site config keys)");
                return;
            }
            print!("{}", c.to_text());
            let m = c.missing();
            if !m.is_empty() {
                println!("# still needed: {}", m.join(", "));
            }
        }
        ["config", "keys"] => {
            for k in KEYS {
                let d = if k.required { "required".to_string() } else if k.default.is_empty() { "default: empty".into() } else { format!("default {}", k.default) };
                println!("{:<17} {} ({d})", k.name, k.help);
            }
        }
        ["config", "set", rest @ ..] if !rest.is_empty() && rest.len() % 2 == 0 => {
            let mut c = load().unwrap_or_else(|e| fail(e));
            for kv in rest.chunks(2) {
                c.set(kv[0], kv[1]).unwrap_or_else(|e| fail(e));
            }
            save(&c.to_text()).unwrap_or_else(|e| fail(e));
            // the Cell Site app is installed on computers with this file
            let _ = ecm_host_abi::apps::ctl(&["reindex"]);
            let keys: Vec<&str> = rest.chunks(2).map(|kv| kv[0]).collect();
            println!("celld-site: saved {}", keys.join(", "));
            let m = c.missing();
            if !m.is_empty() {
                println!("celld-site: still needed: {}", m.join(", "));
            } else if admin_call(c.get("admin_port").parse().unwrap_or(config::ADMIN_PORT), "config").is_ok() {
                println!("celld-site: the site is running: celld-site restart applies the change");
            }
        }
        ["enable"] | ["disable"] => {
            let path = "/services.cfg";
            let text = std::fs::read_to_string(path).unwrap_or_default();
            let mut lines: Vec<String> =
                text.lines().filter(|l| l.trim() != SERVICE_LINE && l.trim() != OLD_SERVICE_LINE).map(String::from).collect();
            if a[0] == "enable" {
                lines.push(SERVICE_LINE.into());
            }
            std::fs::write(path, format!("{}\n", lines.join("\n"))).unwrap_or_else(|e| fail(format!("can't write services.cfg: {e}")));
            println!("celld-site: {}", if a[0] == "enable" { "starts at boot" } else { "no longer starts at boot" });
        }
        _ => usage(),
    }
}

/// The radio's slot clock: read from the radio twice a second, counted forward in between.
struct SlotClock {
    slot: u64,
    at_ms: u64,
    slot_us: u64,
}

impl SlotClock {
    fn now(&self, ms: u64) -> u64 {
        self.slot + (ms.saturating_sub(self.at_ms)) * 1000 / self.slot_us.max(1)
    }
}

fn run(wait: bool) {
    let conf = load().unwrap_or_else(|e| fail(e));
    if conf == Conf::default() {
        fail(format!("no {}: configure the site first (celld-site config keys)", config::PATH));
    }
    let (_, rt) = conf.site_config().unwrap_or_else(|e| fail(e));
    let name = if rt.radio.is_empty() { "sdr".to_string() } else { rt.radio.clone() };
    let mut waited = false;
    let mut radio = loop {
        match Framed::open(&rt.radio) {
            Ok(r) => break r,
            Err(e) if !wait => fail(format!(
                "no framed radio '{name}' ({e}): place an Advanced SDR next to this computer (radio = \"\" uses the first SDR)"
            )),
            Err(e) => {
                if !waited {
                    println!("celld-site: waiting for the framed radio '{name}' ({e})");
                    waited = true;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    };
    if waited {
        println!("celld-site: the framed radio is here");
    }
    let mut admin = AdminServer::bind(rt.admin_port).unwrap_or_else(|e| fail(format!("admin socket: {e} (is celld-site already running?)")));
    let clock = Instant::now();
    let ms = || clock.elapsed().as_millis() as u64;
    let mut d = Daemon::new(conf, Box::new(WasiEntropy), ms()).unwrap_or_else(|e| fail(e));
    let mut restart = false;
    loop {
        let r = serve(&mut d, &mut admin, &mut radio, &clock, restart);
        match r {
            Ok(Action::Restart) => {
                let conf = load().and_then(|c| c.site_config().map(|_| c));
                match conf.and_then(|c| Daemon::new(c, Box::new(WasiEntropy), ms())) {
                    Ok(mut nd) => {
                        nd.log = std::mem::take(&mut d.log);
                        nd.log(ms(), "restarted with the saved settings");
                        d = nd;
                        restart = true;
                    }
                    Err(e) => {
                        d.log(ms(), &format!("restart refused: {e}"));
                        restart = true;
                    }
                }
            }
            Ok(_) => break,
            Err(e) => {
                admin.close();
                fail(e);
            }
        }
    }
    admin.close();
    println!("celld-site: stopped");
}

/// Run the site until a stop or restart. `again` = this is a restart (quieter banner).
fn serve(d: &mut Daemon, admin: &mut AdminServer, radio: &mut Framed, clock: &Instant, again: bool) -> Result<Action, String> {
    let ms = || clock.elapsed().as_millis() as u64;
    let profile = d.site.cfg.profile;
    let mut slots = SlotClock { slot: radio.slot().unwrap_or(0), at_ms: ms(), slot_us: profile.slot_us() };
    let mut last_sync = ms();
    let udp = udp_bind(d.site.cfg.tunnel_port)?;
    let mut tcp: Option<i32> = None;
    let mut next_connect = 0u64;
    let now = Now { ms: ms(), slot: slots.now(ms()) };
    d.site.start(now);
    let c = &d.site.cfg;
    let banner = format!(
        "{} cell {} ({}) on {} channel {}, core {}:{}",
        c.site_id,
        c.cell_id,
        c.carrier,
        c.profile.name(),
        c.channel,
        fmt_ip(d.rt.core),
        d.rt.core_port
    );
    if !again {
        println!("celld-site: {banner}");
    }
    d.log(ms(), &format!("started: {banner}"));
    let mut action = Action::None;
    let mut buf = vec![0u8; 4096];
    let mut stopping_until: Option<u64> = None;
    loop {
        let t = ms();
        if t.saturating_sub(last_sync) >= 500 {
            last_sync = t;
            if let Ok(st) = radio.status() {
                if let Some(s) = st.num("slot") {
                    slots = SlotClock { slot: s as u64, at_ms: t, slot_us: profile.slot_us() };
                }
                d.radio = st.0.iter().filter(|(k, _)| !k.starts_with("rx_gain")).map(|(k, v)| (k.clone(), v.clone())).collect();
            }
        }
        let now = Now { ms: t, slot: slots.now(t) };
        if stopping_until.is_none() && tcp.is_none() && d.site.wants_core(now) && t >= next_connect {
            match tcp_connect(d.rt.core, d.rt.core_port) {
                Some(fd) => {
                    tcp = Some(fd);
                    d.log(ms(), &format!("connected to the core {}:{}", fmt_ip(d.rt.core), d.rt.core_port));
                    d.site.core_connected(Now { ms: ms(), slot: slots.now(ms()) });
                }
                None => {
                    if next_connect == 0 || d.log.back().is_none_or(|l| !l.contains("can't reach the core")) {
                        d.log(ms(), &format!("can't reach the core at {}:{}; trying again", fmt_ip(d.rt.core), d.rt.core_port));
                    }
                    next_connect = ms() + 2000;
                }
            }
        }
        // sockets: tunnel, control, admin
        let mut fds = vec![udp];
        fds.extend(tcp);
        let afds = admin.fds();
        fds.extend(&afds);
        let ready = readable(&fds, 5);
        let t = ms();
        let now = Now { ms: t, slot: slots.now(t) };
        if ready[0] {
            for _ in 0..64 {
                let mut from = SockAddrIn::default();
                let n = socket::recvfrom(udp, &mut buf, socket::MSG_DONTWAIT, &mut from);
                if n <= 0 {
                    break;
                }
                d.site.on_tunnel(&buf[..n as usize], now);
            }
        }
        if let Some(fd) = tcp {
            if ready[1] {
                match read_now(fd, &mut buf) {
                    Read::Data(b) => d.site.on_bytes(&b, now),
                    Read::Nothing => {}
                    Read::Closed => {
                        socket::close(fd);
                        tcp = None;
                        next_connect = t + 1000;
                        d.site.core_disconnected(now);
                    }
                }
            }
        }
        let aready = ready[ready.len() - afds.len()..].to_vec();
        let mut wanted = Action::None;
        {
            let dd = &mut *d;
            admin.serve(&aready, &mut |line| {
                let (r, a) = dd.handle(line, ms(), &mut |text| {
                    let _ = std::fs::create_dir_all("/etc");
                    std::fs::write(config::PATH, text).map_err(|e| format!("can't write {}: {e}", config::PATH))
                });
                if a != Action::None {
                    wanted = a;
                }
                r
            });
        }
        if wanted != Action::None && stopping_until.is_none() {
            // release the phones and stop beaconing, then leave once that went out
            action = wanted;
            d.site.stop();
            d.log(ms(), if action == Action::Stop { "stopping" } else { "restarting" });
            stopping_until = Some(ms() + 300);
        }
        // the radio
        for _ in 0..64 {
            match FramedPhy::poll(radio) {
                Some(e) => d.site.on_phy(e, now),
                None => break,
            }
        }
        d.site.tick(now);
        for o in d.site.take_out() {
            match o {
                SiteOut::Phy(c) => {
                    if let Err(e) = phy::apply(radio, [c]) {
                        d.log(ms(), &format!("radio: {}", e.0));
                    }
                }
                SiteOut::Send(b) => {
                    if let Some(fd) = tcp {
                        if !send_all(fd, &b) {
                            socket::close(fd);
                            tcp = None;
                            next_connect = ms() + 1000;
                            d.site.core_disconnected(now);
                        }
                    }
                }
                SiteOut::CloseCore => {
                    if let Some(fd) = tcp.take() {
                        socket::close(fd);
                        next_connect = ms() + 1000;
                    }
                }
                SiteOut::Tunnel(dg) => {
                    socket::sendto(udp, &dg, 0, &SockAddrIn::from_ip_port(d.rt.core, d.rt.core_tunnel_port));
                }
                SiteOut::Log(l) => d.log(ms(), &l),
            }
        }
        if stopping_until.is_some_and(|u| ms() >= u) {
            break;
        }
    }
    if let Some(fd) = tcp {
        socket::close(fd);
    }
    socket::close(udp);
    if action == Action::Stop {
        let _ = radio.beacon(0, 0, &[]);
    }
    Ok(action)
}
