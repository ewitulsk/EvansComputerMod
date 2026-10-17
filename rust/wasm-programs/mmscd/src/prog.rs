//! The program: CLI and the daemon loop (WASI only).

use std::collections::BTreeMap;
use std::time::Instant;

use ecm_cell_net::{admin_call, AdminServer};
use ecm_tel::mmsc::{HttpResult, Mmsc, MmscConfig, MmscOut};
use ecm_tel::txn::Addr;
use ecm_tel_net::smtp;
use ecm_tel_net::sys::{self, HssClient, HttpServer, Log};

const PATH: &str = "/etc/mmscd.toml";
/// The Browser links' key (made once).
const LINK_KEY: &str = "/var/mmscd/link.key";

const KEYS: &[(&str, &str)] = &[
    ("hss", "127.0.0.1:3868"),
    ("hss_secret", ""),
    ("ip", ""),
    ("http_port", "8002"),
    ("smsc", ""),
    ("max_kb", "600"),
    ("expiry", "140"),
    ("peers", ""),
    ("public_peering", "false"),
    ("mm4_port", "25"),
    ("admin_port", "7723"),
];
const HELP: &[(&str, &str)] = &[
    ("hss", "cored's HSS port (ip:port)"),
    ("hss_secret", "cored's hss_secret, when cored runs on another computer"),
    ("ip", "this computer's address (default: the MMSC the HSS names, else its P-CSCF)"),
    ("http_port", "where phones send and fetch MMS"),
    ("smsc", "smscd, for notifications (default ip:5062)"),
    ("max_kb", "the largest MMS accepted, KB"),
    ("expiry", "minutes content waits to be fetched (140 = 7 in-game days)"),
    ("peers", "other carriers' MMSCs (MM4): domain/secret, ... (the same secrets as imsd's peers)"),
    ("public_peering", "true: also MM4 from carriers that are not listed"),
    ("mm4_port", "the SMTP port MM4 arrives on (other carriers always send to 25: change it only to test)"),
    ("admin_port", "the admin socket (127.0.0.1)"),
];

fn fail(m: impl AsRef<str>) -> ! {
    println!("mmscd: {}", m.as_ref());
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
        ["stop"] | ["status"] | ["stored"] | ["log"] | ["log", _] => match admin_call(port(&conf(), "admin_port"), &a.join(" ")) {
            Ok(b) => b.iter().for_each(|l| println!("{l}")),
            Err(e) if e == "not running" => fail("not running (start it with: mmscd start &)"),
            Err(e) => fail(e),
        },
        ["config"] => conf().iter().for_each(|(k, v)| println!("{k} = {v}")),
        ["config", "keys"] => HELP.iter().for_each(|(k, h)| println!("{k:<15} {h}")),
        ["config", "set", rest @ ..] if !rest.is_empty() && rest.len() % 2 == 0 => {
            let mut c = conf();
            for kv in rest.chunks(2) {
                if !KEYS.iter().any(|(k, _)| *k == kv[0]) {
                    fail(format!("unknown key '{}' (mmscd config keys)", kv[0]));
                }
                c.insert(kv[0].into(), kv[1].into());
            }
            sys::save_conf(PATH, "mmscd (docs/TELEPHONY.md)", KEYS, &c).unwrap_or_else(|e| fail(e));
            println!("mmscd: saved (restart mmscd to apply)");
        }
        ["enable"] | ["disable"] => {
            sys::service_line("mmscd start", "mmscd start --wait &", a[0] == "enable").unwrap_or_else(|e| fail(e));
            println!("mmscd: {}", if a[0] == "enable" { "starts at boot" } else { "no longer starts at boot" });
        }
        _ => {
            println!("usage: mmscd start [--wait] | stop | status | stored | log [n]");
            println!("       mmscd config | config keys | config set <key> <value>... | enable | disable");
            std::process::exit(2);
        }
    }
}

fn run(wait: bool) {
    let c = conf();
    let g = |k: &str| c.get(k).cloned().unwrap_or_default();
    let mut hss = HssClient::new(Addr::parse(&g("hss"), 3868).unwrap_or_else(|| fail("bad hss")), "mmscd", &g("hss_secret"));
    let mut log = Log::new("mmscd", true);
    let num = sys::numbering(&mut hss, wait).unwrap_or_else(|e| fail(e));
    let ip = match g("ip").as_str() {
        "" => Addr::parse_ip(&sys::kv(&num, "mmsc")).or_else(|| Addr::parse_ip(&sys::kv(&num, "pcscf"))).unwrap_or_else(|| fail("the HSS has no IMS set up (cored ims pool=...) and mmscd.toml has no ip")),
        s => Addr::parse_ip(s).unwrap_or_else(|| fail("bad ip")),
    };
    let domain = sys::kv(&num, "domain");
    let mut cfg = MmscConfig::new(&domain, &sys::kv(&num, "carrier"), ip);
    cfg.http_port = port(&c, "http_port");
    if !g("smsc").is_empty() {
        cfg.smsc = Addr::parse(&g("smsc"), 5062).unwrap_or_else(|| fail("bad smsc"));
    } else if let Some(p) = Addr::parse_ip(&sys::kv(&num, "pcscf")) {
        // smscd runs beside imsd
        cfg.smsc = Addr::new(p, 5062);
    }
    cfg.max_bytes = g("max_kb").parse::<usize>().unwrap_or(600) * 1024;
    cfg.expiry_ms = g("expiry").parse::<u64>().unwrap_or(140) * 60_000;
    cfg.public_peering = g("public_peering") == "true";
    for item in g("peers").split(',').map(str::trim).filter(|x| !x.is_empty()) {
        let (d, s) = item.split_once('/').unwrap_or_else(|| fail(format!("peer '{item}': domain/secret")));
        cfg.peers.push((d.to_ascii_lowercase(), s.to_string()));
    }
    // The key of the Browser links (/v/<id>?r=...): kept, so links in texts still work after a restart.
    cfg.secret = match std::fs::read_to_string(LINK_KEY).ok().map(|s| s.trim().to_string()).filter(|s| s.len() == 16) {
        Some(k) => k,
        None => {
            let k = hex8(&ecm_cell_net::random::<8>());
            let _ = sys::write_file(LINK_KEY, k.as_bytes());
            k
        }
    };
    let mut admin = AdminServer::bind(port(&c, "admin_port")).unwrap_or_else(|e| fail(format!("admin socket: {e} (is mmscd already running?)")));
    let sip = sys::udp_bind(cfg.sip_port).unwrap_or_else(|e| fail(e));
    let mut http = HttpServer::bind(cfg.http_port).unwrap_or_else(|e| fail(e));
    let mm4 = sys::tcp_listen(port(&c, "mm4_port")).unwrap_or_else(|e| fail(format!("MM4 (SMTP) port: {e}")));
    let seed = u64::from_le_bytes(ecm_cell_net::random::<8>());
    let mut mmsc = Mmsc::new(cfg.clone(), seed);
    log.line(&format!("mmscd: MMSC of {} at {}/mms, MM4 on port {}, {} peer(s)", domain, cfg.url(), port(&c, "mm4_port"), cfg.peers.len()));
    let clock = Instant::now();
    let ms = || clock.elapsed().as_millis() as u64;
    let mut smtp_conns: Vec<(i32, smtp::Server)> = Vec::new();
    let mut hss_ids: BTreeMap<u64, u64> = BTreeMap::new();
    let mut later: BTreeMap<u64, u64> = BTreeMap::new();
    let mut next_hss = 100u64;
    let mut buf = vec![0u8; 65536];
    let mut last_tick = 0;
    let mut stop = false;
    while !stop {
        let mut fds = vec![sip, mm4];
        fds.extend(smtp_conns.iter().map(|c| c.0));
        let hfds = http.fds();
        let hbase = fds.len();
        fds.extend(&hfds);
        if let Some(f) = hss.fd() {
            fds.push(f);
        }
        let afds = admin.fds();
        fds.extend(&afds);
        // connections accepted below have no entry in `ready` until the next round
        let polled = smtp_conns.len();
        let ready = sys::readable(&fds, 10);
        let now = ms();
        if ready[0] {
            for (from, d) in sys::udp_drain(sip, &mut buf, 64) {
                mmsc.on_sip(from, &d, now);
            }
        }
        if ready[1] {
            let mut a = ecm_host_abi::socket::SockAddrIn::default();
            let fd = ecm_host_abi::socket::accept(mm4, &mut a);
            if fd >= 0 {
                let s = smtp::Server::new(&domain);
                sys::send_all(fd, format!("{}\r\n", s.greeting()).as_bytes());
                smtp_conns.push((fd, s));
            }
        }
        let mut gone = Vec::new();
        for (i, (fd, s)) in smtp_conns.iter_mut().enumerate() {
            if i >= polled || !ready[2 + i] {
                continue;
            }
            match sys::read_now(*fd, &mut buf) {
                Some(d) if d.is_empty() => {}
                Some(d) => {
                    let mut step = s.push(&d);
                    loop {
                        for r in &step.replies {
                            sys::send_all(*fd, format!("{r}\r\n").as_bytes());
                        }
                        if step.close {
                            gone.push(*fd);
                            break;
                        }
                        match step.message.take() {
                            Some((_from, _rcpt, text)) => {
                                let v = mmsc.on_mm4(&text, now);
                                sys::send_all(*fd, format!("{}\r\n", smtp::Server::verdict(&v)).as_bytes());
                                step = s.push(&[]);
                            }
                            None => break,
                        }
                    }
                }
                None => gone.push(*fd),
            }
        }
        for fd in gone {
            ecm_host_abi::socket::close(fd);
            smtp_conns.retain(|c| c.0 != fd);
        }
        let hready = ready[hbase..hbase + hfds.len()].to_vec();
        for (conn, from, req) in http.poll(&hready) {
            match mmsc.http(&req.method, &req.target, from, &req.body, now) {
                HttpResult::Now(r) => http.reply(conn, r.status, &r.content_type, &r.body),
                HttpResult::Later(id) => {
                    later.insert(id, conn);
                }
            }
        }
        for (id, r) in hss.pump() {
            if let Some(mid) = hss_ids.remove(&id) {
                mmsc.on_hss(mid, r, now);
            }
        }
        let aready = ready[ready.len() - afds.len()..].to_vec();
        {
            let stored: Vec<String> = mmsc
                .stored()
                .iter()
                .map(|m| format!("mms {} {} {} {}B {}", m.id, m.from, m.to.join(","), m.size, m.recipients.iter().map(|r| format!("{}:{}", r.number, if r.reported { "delivered" } else if r.fetched { "fetched" } else if r.notified { "notified" } else { "waiting" })).collect::<Vec<_>>().join(",")))
                .collect();
            let log2 = &log;
            let hss_ok = hss.connected();
            admin.serve(&aready, &mut |line| match line.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["status"] => Ok(format!("domain {}\nurl {}/mms\nhss {}\nstored {}\npeers {}\n", cfg.domain, cfg.url(), if hss_ok { "up" } else { "down" }, stored.len(), cfg.peers.len())),
                ["stored"] => Ok(stored.join("\n")),
                ["log"] => Ok(log2.tail(50)),
                ["log", n] => Ok(log2.tail(n.parse().unwrap_or(50))),
                ["stop"] => {
                    stop = true;
                    Ok("stopping".into())
                }
                _ => Err("unknown command (status, stored, log, stop)".into()),
            });
        }
        if now.saturating_sub(last_tick) >= 100 {
            last_tick = now;
            mmsc.tick(now);
        }
        for o in mmsc.take_out() {
            match o {
                MmscOut::Sip { to, data } => sys::udp_send(sip, to, &data),
                MmscOut::Hss { id, line } => {
                    next_hss += 1;
                    hss_ids.insert(next_hss, id);
                    hss.request(next_hss, &line);
                }
                MmscOut::Enum { id, number } => {
                    let d = sys::enum_lookup(&number).map(|a| a.domain);
                    mmsc.on_enum(id, d, now);
                }
                MmscOut::Mm4 { id, domain: to, rcpt, from, message } => {
                    let ok = match sys::mx_lookup(&to) {
                        Some(host) => match sys::smtp_send(host, &format!("mms.{}", domain), &from, &rcpt, &message) {
                            Ok(()) => true,
                            Err(e) => {
                                log.line(&format!("MM4 to {to}: {e}"));
                                false
                            }
                        },
                        None => {
                            log.line(&format!("MM4 to {to}: no MX or address for it"));
                            false
                        }
                    };
                    mmsc.on_mm4_sent(id, ok, now);
                }
                MmscOut::HttpReply { req, reply } => {
                    if let Some(conn) = later.remove(&req) {
                        http.reply(conn, reply.status, &reply.content_type, &reply.body);
                    }
                }
                MmscOut::Save { path, data } => {
                    if let Err(e) = sys::write_file(&path, &data) {
                        log.line(&e);
                    }
                }
                MmscOut::Delete { path } => {
                    let _ = std::fs::remove_file(&path);
                }
                MmscOut::Cdr(l) => sys::cdr("mmscd", &l),
                MmscOut::Log(l) => log.line(&l),
            }
        }
    }
    log.line("mmscd: stopped");
}

fn hex8(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}
