//! The program: CLI and the daemon loop (WASI only).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use ecm_cell_net::{admin_call, AdminServer};
use ecm_tel::ims::{Ims, ImsOut};
use ecm_tel::txn::Addr;
use ecm_tel_net::sys::{self, HssClient, Log};
use imsd::{config, format_peers, parse_num, parse_peers, status_text, ADMIN_PORT, HELP, KEYS, PATH};

const SERVICE: &str = "imsd start --wait &";

fn fail(m: impl AsRef<str>) -> ! {
    println!("imsd: {}", m.as_ref());
    std::process::exit(1);
}

fn usage() -> ! {
    println!("usage: imsd start [--wait] | stop | status | log [n]");
    println!("       imsd config | config keys | config set <key> <value>...");
    println!("       imsd peer add <domain> <ip[:port]|srv> <secret> | peer remove <domain> | peers");
    println!("       imsd enable | disable");
    std::process::exit(2);
}

fn conf() -> BTreeMap<String, String> {
    sys::load_conf(PATH, KEYS).unwrap_or_else(|e| fail(e))
}

fn save(c: &BTreeMap<String, String>) {
    let mut t = String::from("# imsd (docs/TELEPHONY.md); imsd config keys explains them\n");
    for (k, d) in KEYS {
        let v = c.get(*k).cloned().unwrap_or_default();
        if v != *d {
            t.push_str(&format!("{} = \"{}\"\n", k, v));
        }
    }
    sys::write_file(PATH, t.as_bytes()).unwrap_or_else(|e| fail(e));
}

fn admin_port(c: &BTreeMap<String, String>) -> u16 {
    c.get("admin_port").and_then(|p| p.parse().ok()).unwrap_or(ADMIN_PORT)
}

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["start"] => run(false),
        ["start", "--wait"] => run(true),
        ["stop"] | ["status"] | ["log"] | ["log", _] => {
            let c = conf();
            match admin_call(admin_port(&c), &a.join(" ")) {
                Ok(b) => b.iter().for_each(|l| println!("{l}")),
                Err(e) if e == "not running" => fail("not running (start it with: imsd start &)"),
                Err(e) => fail(e),
            }
        }
        ["config"] => {
            for (k, v) in conf() {
                println!("{k} = {v}");
            }
        }
        ["config", "keys"] => {
            for (k, h) in HELP {
                println!("{k:<15} {h}");
            }
        }
        ["config", "set", rest @ ..] if !rest.is_empty() && rest.len() % 2 == 0 => {
            let mut c = conf();
            for kv in rest.chunks(2) {
                if !KEYS.iter().any(|(k, _)| *k == kv[0]) {
                    fail(format!("unknown key '{}' (imsd config keys)", kv[0]));
                }
                c.insert(kv[0].into(), kv[1].into());
            }
            config(&c, &imsd::Numbering { pcscf: Some([127, 0, 0, 1]), ..Default::default() }).unwrap_or_else(|e| fail(e));
            save(&c);
            println!("imsd: saved (restart imsd to apply)");
        }
        ["peer", "add", domain, addr, secret] => {
            let mut c = conf();
            let mut p = parse_peers(c.get("peers").map(String::as_str).unwrap_or("")).unwrap_or_else(|e| fail(e));
            p.retain(|x| x.domain != *domain);
            p.extend(parse_peers(&format!("{domain}@{addr}/{secret}")).unwrap_or_else(|e| fail(e)));
            c.insert("peers".into(), format_peers(&p));
            save(&c);
            println!("imsd: peer {domain} added (restart imsd to apply)");
        }
        ["peer", "remove", domain] => {
            let mut c = conf();
            let mut p = parse_peers(c.get("peers").map(String::as_str).unwrap_or("")).unwrap_or_else(|e| fail(e));
            let n = p.len();
            p.retain(|x| x.domain != *domain);
            if n == p.len() {
                fail(format!("no peer {domain}"));
            }
            c.insert("peers".into(), format_peers(&p));
            save(&c);
            println!("imsd: peer {domain} removed (restart imsd to apply)");
        }
        ["peers"] => {
            for p in parse_peers(conf().get("peers").map(String::as_str).unwrap_or("")).unwrap_or_default() {
                println!("{} {}", p.domain, p.addr.map(|a| a.to_string()).unwrap_or_else(|| "srv".into()));
            }
        }
        ["enable"] | ["disable"] => {
            let t = std::fs::read_to_string("/services.cfg").unwrap_or_default();
            let mut lines: Vec<String> = t.lines().filter(|l| !l.trim().starts_with("imsd start")).map(String::from).collect();
            if a[0] == "enable" {
                lines.push(SERVICE.into());
            }
            sys::write_file("/services.cfg", format!("{}\n", lines.join("\n")).as_bytes()).unwrap_or_else(|e| fail(e));
            println!("imsd: {}", if a[0] == "enable" { "starts at boot" } else { "no longer starts at boot" });
        }
        _ => usage(),
    }
}

fn run(wait: bool) {
    let c = conf();
    let hss_addr = Addr::parse(c.get("hss").map(String::as_str).unwrap_or("127.0.0.1:3868"), 3868).unwrap_or_else(|| fail("bad hss address"));
    let mut hss = HssClient::new(hss_addr, "imsd", c.get("hss_secret").map(String::as_str).unwrap_or(""));
    let mut log = Log::new("imsd", true);
    // the numbering and IMS setup from the HSS (waiting for cored at boot)
    let started = Instant::now();
    let num = loop {
        hss.request(1, "NUM");
        let mut got = None;
        for _ in 0..40 {
            if let Some(fd) = hss.fd() {
                sys::readable(&[fd], 50);
            } else {
                std::thread::sleep(Duration::from_millis(50));
            }
            for (id, r) in hss.pump() {
                if id == 1 {
                    got = Some(r);
                }
            }
            if got.is_some() {
                break;
            }
        }
        match got {
            Some(Ok(b)) => break parse_num(&b),
            Some(Err(e)) if !wait => fail(format!("the HSS: {e}")),
            None if !wait && started.elapsed() > Duration::from_secs(5) => fail(hss.last_error.clone()),
            _ => std::thread::sleep(Duration::from_secs(2)),
        }
    };
    let cfg = config(&c, &num).unwrap_or_else(|e| fail(e));
    let mut admin = AdminServer::bind(admin_port(&c)).unwrap_or_else(|e| fail(format!("admin socket: {e} (is imsd already running?)")));
    let sip = sys::udp_bind(cfg.sip_port).unwrap_or_else(|e| fail(e));
    log.line(&format!("imsd: {} on {} (P-CSCF for {}/{}), border {}, {} peer(s)", cfg.domain, Addr::new(cfg.ims_ip, cfg.sip_port), Addr::new(cfg.ims_pool.0, 0).host(), cfg.ims_pool.1, Addr::new(cfg.public_ip, 0).host(), cfg.peers.len()));
    let seed = u64::from_le_bytes(ecm_cell_net::random::<8>());
    let mut ims = Ims::new(cfg.clone(), seed);
    let mut relays: BTreeMap<u16, i32> = BTreeMap::new();
    let mut hss_ids: BTreeMap<u64, u64> = BTreeMap::new();
    let mut enum_cache: BTreeMap<String, (Instant, Option<ecm_tel::ims::EnumAnswer>)> = BTreeMap::new();
    let clock = Instant::now();
    let ms = || clock.elapsed().as_millis() as u64;
    let mut buf = vec![0u8; 65536];
    let mut next_hss = 100u64;
    let mut last_tick = 0u64;
    let mut stop = false;
    while !stop {
        let mut fds = vec![sip];
        let rports: Vec<u16> = relays.keys().copied().collect();
        fds.extend(rports.iter().map(|p| relays[p]));
        let hss_fd = hss.fd();
        if let Some(f) = hss_fd {
            fds.push(f);
        }
        let afds = admin.fds();
        fds.extend(&afds);
        let ready = sys::readable(&fds, 5);
        let now = ms();
        if ready[0] {
            for (from, d) in sys::udp_drain(sip, &mut buf, 64) {
                ims.on_sip(from, &d, now);
            }
        }
        for (i, p) in rports.iter().enumerate() {
            if ready[1 + i] {
                for (from, d) in sys::udp_drain(relays[p], &mut buf, 64) {
                    ims.on_relay(*p, from, &d, now);
                }
            }
        }
        for (id, r) in hss.pump() {
            if let Some(mid) = hss_ids.remove(&id) {
                ims.on_hss(mid, r, now);
            }
        }
        let aready = ready[ready.len() - afds.len()..].to_vec();
        {
            let st = ims.status();
            let (ok, err) = (hss.connected(), hss.last_error.clone());
            let cfg2 = &cfg;
            let log2 = &log;
            admin.serve(&aready, &mut |line| {
                let w: Vec<&str> = line.split_whitespace().collect();
                match w.as_slice() {
                    ["status"] => Ok(status_text(cfg2, &st, ok, &err, now)),
                    ["log"] => Ok(log2.tail(50)),
                    ["log", n] => Ok(log2.tail(n.parse().unwrap_or(50))),
                    ["stop"] => {
                        stop = true;
                        Ok("stopping".into())
                    }
                    _ => Err(format!("unknown command '{}' (status, log, stop)", line.trim())),
                }
            });
        }
        if now.saturating_sub(last_tick) >= 20 {
            last_tick = now;
            ims.tick(now);
        }
        for o in ims.take_out() {
            match o {
                ImsOut::Sip { to, data } => sys::udp_send(sip, to, &data),
                ImsOut::Relay { port, to, data } => {
                    if let Some(fd) = relays.get(&port) {
                        sys::udp_send(*fd, to, &data);
                    }
                }
                ImsOut::OpenRelay(p) => match sys::udp_bind(p) {
                    Ok(fd) => {
                        relays.insert(p, fd);
                    }
                    Err(e) => log.line(&format!("relay port {p}: {e}")),
                },
                ImsOut::CloseRelay(p) => {
                    if let Some(fd) = relays.remove(&p) {
                        ecm_host_abi::socket::close(fd);
                    }
                }
                ImsOut::Hss { id, line } => {
                    next_hss += 1;
                    hss_ids.insert(next_hss, id);
                    hss.request(next_hss, &line);
                }
                ImsOut::Enum { id, number } => {
                    let fresh = enum_cache.get(&number).filter(|(t, _)| t.elapsed() < Duration::from_secs(300)).map(|(_, a)| a.clone());
                    let a = match fresh {
                        Some(a) => a,
                        None => {
                            let a = sys::enum_lookup(&number);
                            enum_cache.insert(number.clone(), (Instant::now(), a.clone()));
                            a
                        }
                    };
                    match &a {
                        Some(x) => log.line(&format!("ENUM {number}: {} (border {})", x.domain, x.border.map(|b| b.to_string()).unwrap_or_else(|| "-".into()))),
                        None => log.line(&format!("ENUM {number}: no record")),
                    }
                    ims.on_enum(id, a, now);
                }
                ImsOut::Cdr(l) => sys::cdr("imsd", &l),
                ImsOut::Log(l) => log.line(&l),
            }
        }
    }
    log.line("imsd: stopped");
}
