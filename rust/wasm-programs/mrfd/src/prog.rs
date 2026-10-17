//! The program: CLI and the daemon loop (WASI only).

use std::collections::BTreeMap;
use std::time::Instant;

use ecm_cell_net::{admin_call, AdminServer};
use ecm_tel::mrf::{Mrf, MrfConfig, MrfOut, SPOOL};
use ecm_tel::txn::Addr;
use ecm_tel_net::sys::{self, HssClient, HttpServer, Log};

const PATH: &str = "/etc/mrfd.toml";
const KEYS: &[(&str, &str)] = &[
    ("hss", "127.0.0.1:3868"),
    ("hss_secret", ""),
    ("ip", ""),
    ("ims", ""),
    ("smsc", ""),
    ("rtp_ports", "31000-32000"),
    ("max_message", "60"),
    ("http_port", "8003"),
    ("admin_port", "7724"),
];
const HELP: &[(&str, &str)] = &[
    ("hss", "cored's HSS port (ip:port)"),
    ("hss_secret", "cored's hss_secret, when cored runs on another computer"),
    ("ip", "this computer's IMS address (default: the P-CSCF the HSS names)"),
    ("ims", "imsd (default ip:5060)"),
    ("smsc", "smscd, for the support line's texts (default ip:5062)"),
    ("rtp_ports", "media ports, from-to"),
    ("max_message", "the longest voicemail, seconds"),
    ("http_port", "visual voicemail (phones fetch their messages here)"),
    ("admin_port", "the admin socket (127.0.0.1)"),
];

fn fail(m: impl AsRef<str>) -> ! {
    println!("mrfd: {}", m.as_ref());
    std::process::exit(1);
}

fn conf() -> BTreeMap<String, String> {
    sys::load_conf(PATH, KEYS).unwrap_or_else(|e| fail(e))
}

fn port(c: &BTreeMap<String, String>, k: &str) -> u16 {
    c.get(k).and_then(|p| p.parse().ok()).unwrap_or_else(|| fail(format!("bad {k}")))
}

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["start"] => run(false),
        ["start", "--wait"] => run(true),
        ["stop"] | ["status"] | ["boxes"] | ["log"] | ["log", _] => match admin_call(port(&conf(), "admin_port"), &a.join(" ")) {
            Ok(b) => b.iter().for_each(|l| println!("{l}")),
            Err(e) if e == "not running" => fail("not running (start it with: mrfd start &)"),
            Err(e) => fail(e),
        },
        ["config"] => conf().iter().for_each(|(k, v)| println!("{k} = {v}")),
        ["config", "keys"] => HELP.iter().for_each(|(k, h)| println!("{k:<15} {h}")),
        ["config", "set", rest @ ..] if !rest.is_empty() && rest.len() % 2 == 0 => {
            let mut c = conf();
            for kv in rest.chunks(2) {
                if !KEYS.iter().any(|(k, _)| *k == kv[0]) {
                    fail(format!("unknown key '{}' (mrfd config keys)", kv[0]));
                }
                c.insert(kv[0].into(), kv[1].into());
            }
            sys::save_conf(PATH, "mrfd (docs/TELEPHONY.md)", KEYS, &c).unwrap_or_else(|e| fail(e));
            println!("mrfd: saved (restart mrfd to apply)");
        }
        ["enable"] | ["disable"] => {
            sys::service_line("mrfd start", "mrfd start --wait &", a[0] == "enable").unwrap_or_else(|e| fail(e));
            println!("mrfd: {}", if a[0] == "enable" { "starts at boot" } else { "no longer starts at boot" });
        }
        _ => {
            println!("usage: mrfd start [--wait] | stop | status | boxes | log [n]");
            println!("       mrfd config | config keys | config set <key> <value>... | enable | disable");
            std::process::exit(2);
        }
    }
}

fn run(wait: bool) {
    let c = conf();
    let g = |k: &str| c.get(k).cloned().unwrap_or_default();
    let mut hss = HssClient::new(Addr::parse(&g("hss"), 3868).unwrap_or_else(|| fail("bad hss")), "mrfd", &g("hss_secret"));
    let mut log = Log::new("mrfd", true);
    let num = sys::numbering(&mut hss, wait).unwrap_or_else(|e| fail(e));
    let ip = match g("ip").as_str() {
        "" => Addr::parse_ip(&sys::kv(&num, "pcscf")).unwrap_or_else(|| fail("the HSS has no IMS set up (cored ims pool=...) and mrfd.toml has no ip")),
        s => Addr::parse_ip(s).unwrap_or_else(|| fail("bad ip")),
    };
    let domain = sys::kv(&num, "domain");
    let mut cfg = MrfConfig::new(&domain, &sys::kv(&num, "carrier"), ip);
    if !g("ims").is_empty() {
        cfg.ims = Addr::parse(&g("ims"), 5060).unwrap_or_else(|| fail("bad ims"));
    }
    if !g("smsc").is_empty() {
        cfg.smsc = Addr::parse(&g("smsc"), 5062).unwrap_or_else(|| fail("bad smsc"));
    }
    let (lo, hi) = g("rtp_ports").split_once('-').and_then(|(a, b)| Some((a.trim().parse::<u16>().ok()?, b.trim().parse::<u16>().ok()?))).unwrap_or_else(|| fail("rtp_ports is from-to"));
    cfg.rtp_ports = (lo, hi);
    cfg.max_record_ms = g("max_message").parse::<u64>().unwrap_or(60).clamp(5, 300) * 1000;
    let mut admin = AdminServer::bind(port(&c, "admin_port")).unwrap_or_else(|e| fail(format!("admin socket: {e} (is mrfd already running?)")));
    let sip = sys::udp_bind(cfg.sip_port).unwrap_or_else(|e| fail(e));
    let mut http = HttpServer::bind(port(&c, "http_port")).unwrap_or_else(|e| fail(e));
    let seed = u64::from_le_bytes(ecm_cell_net::random::<8>());
    let mut mrf = Mrf::new(cfg.clone(), seed);
    // the voicemail boxes on disk
    let mut boxes = 0;
    if let Ok(dir) = std::fs::read_dir(SPOOL) {
        for e in dir.flatten() {
            let number = e.file_name().to_string_lossy().to_string();
            if let Ok(index) = std::fs::read_to_string(format!("{SPOOL}/{number}/index")) {
                mrf.load_box(&number, &index, &|p| std::fs::read(p).ok());
                boxes += 1;
            }
        }
    }
    log.line(&format!("mrfd: media server of {} on {}, {} voicemail box(es)", domain, Addr::new(ip, cfg.sip_port), boxes));
    let clock = Instant::now();
    let ms = || clock.elapsed().as_millis() as u64;
    let mut rtp: BTreeMap<u16, i32> = BTreeMap::new();
    let mut hss_ids: BTreeMap<u64, u64> = BTreeMap::new();
    let mut next_hss = 100u64;
    let mut buf = vec![0u8; 65536];
    let mut next_tick = ms();
    let mut stop = false;
    while !stop {
        let mut fds = vec![sip];
        let ports: Vec<u16> = rtp.keys().copied().collect();
        fds.extend(ports.iter().map(|p| rtp[p]));
        let hfds = http.fds();
        let hbase = fds.len();
        fds.extend(&hfds);
        if let Some(f) = hss.fd() {
            fds.push(f);
        }
        let afds = admin.fds();
        fds.extend(&afds);
        let wait_ms = next_tick.saturating_sub(ms()).min(5) as i32;
        let ready = sys::readable(&fds, wait_ms);
        let now = ms();
        if ready[0] {
            for (from, d) in sys::udp_drain(sip, &mut buf, 64) {
                mrf.on_sip(from, &d, now);
            }
        }
        for (i, p) in ports.iter().enumerate() {
            if ready[1 + i] {
                for (from, d) in sys::udp_drain(rtp[p], &mut buf, 64) {
                    mrf.on_rtp(*p, from, &d, now);
                }
            }
        }
        let hready = ready[hbase..hbase + hfds.len()].to_vec();
        for (conn, from, req) in http.poll(&hready) {
            let (st, ct, body) = mrf.http(&req.method, &req.target, from, now);
            http.reply(conn, st, ct, &body);
        }
        for (id, r) in hss.pump() {
            if let Some(mid) = hss_ids.remove(&id) {
                mrf.on_hss(mid, r, now);
            }
        }
        let aready = ready[ready.len() - afds.len()..].to_vec();
        {
            let log2 = &log;
            let n = mrf.sessions();
            let conf_levels = mrf.conference_levels();
            admin.serve(&aready, &mut |line| match line.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["status"] => Ok(format!("domain {}\nip {}\nsessions {}\nconference_participants {}\n", cfg.domain, Addr::new(cfg.ip, cfg.sip_port), n, conf_levels.len())),
                ["boxes"] => Ok(std::fs::read_dir(SPOOL)
                    .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect::<Vec<_>>().join("\n"))
                    .unwrap_or_default()),
                ["log"] => Ok(log2.tail(50)),
                ["log", k] => Ok(log2.tail(k.parse().unwrap_or(50))),
                ["stop"] => {
                    stop = true;
                    Ok("stopping".into())
                }
                _ => Err("unknown command (status, boxes, log, stop)".into()),
            });
        }
        // the media clock: 20 ms frames (catch up a little when late)
        let mut catch = 0;
        while ms() >= next_tick && catch < 5 {
            mrf.tick(next_tick);
            next_tick += 20;
            catch += 1;
        }
        if ms() > next_tick + 200 {
            next_tick = ms();
        }
        for o in mrf.take_out() {
            match o {
                MrfOut::Sip { to, data } => sys::udp_send(sip, to, &data),
                MrfOut::Rtp { port, to, data } => {
                    if let Some(fd) = rtp.get(&port) {
                        sys::udp_send(*fd, to, &data);
                    }
                }
                MrfOut::OpenRtp(p) => match sys::udp_bind(p) {
                    Ok(fd) => {
                        rtp.insert(p, fd);
                    }
                    Err(e) => log.line(&format!("RTP port {p}: {e}")),
                },
                MrfOut::CloseRtp(p) => {
                    if let Some(fd) = rtp.remove(&p) {
                        ecm_host_abi::socket::close(fd);
                    }
                }
                MrfOut::Save { path, data } => {
                    if let Err(e) = sys::write_file(&path, &data) {
                        log.line(&e);
                    }
                }
                MrfOut::Delete { path } => {
                    let _ = std::fs::remove_file(&path);
                }
                MrfOut::Hss { id, line } => {
                    next_hss += 1;
                    hss_ids.insert(next_hss, id);
                    hss.request(next_hss, &line);
                }
                MrfOut::Log(l) => log.line(&l),
            }
        }
    }
    log.line("mrfd: stopped");
}
