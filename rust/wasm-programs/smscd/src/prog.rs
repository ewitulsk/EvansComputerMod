//! The program: CLI and the daemon loop (WASI only).

use std::collections::BTreeMap;
use std::time::Instant;

use ecm_cell::admin::Lines;
use ecm_cell_net::{admin_call, AdminServer};
use ecm_tel::smsc::{Smsc, SmscConfig, SmscOut};
use ecm_tel::txn::Addr;
use ecm_tel_net::sys::{self, HssClient, Log};

const PATH: &str = "/etc/smscd.toml";
const QUEUE: &str = "/var/spool/smsc/queue";
const KEYS: &[(&str, &str)] = &[
    ("hss", "127.0.0.1:3868"),
    ("hss_secret", ""),
    ("ip", ""),
    ("ims", ""),
    ("address", "+10000000001"),
    ("validity", "140"),
    ("rate_per_min", "30"),
    ("admin_port", "7722"),
    ("shortcode_port", "7730"),
];
const HELP: &[(&str, &str)] = &[
    ("hss", "cored's HSS port (ip:port)"),
    ("hss_secret", "cored's hss_secret, when cored runs on another computer"),
    ("ip", "this computer's IMS address (default: the P-CSCF the HSS names)"),
    ("ims", "imsd (default ip:5060)"),
    ("address", "the SMSC's own number (RP originator)"),
    ("validity", "minutes a text waits for an off phone (140 = 7 in-game days)"),
    ("rate_per_min", "texts a sender may submit per minute"),
    ("admin_port", "the admin socket (127.0.0.1)"),
    ("shortcode_port", "the short-code socket for programs (127.0.0.1)"),
];

fn fail(m: impl AsRef<str>) -> ! {
    println!("smscd: {}", m.as_ref());
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
        ["stop"] | ["status"] | ["queue"] | ["log"] | ["log", _] => match admin_call(port(&conf(), "admin_port"), &a.join(" ")) {
            Ok(b) => b.iter().for_each(|l| println!("{l}")),
            Err(e) if e == "not running" => fail("not running (start it with: smscd start &)"),
            Err(e) => fail(e),
        },
        ["config"] => conf().iter().for_each(|(k, v)| println!("{k} = {v}")),
        ["config", "keys"] => HELP.iter().for_each(|(k, h)| println!("{k:<15} {h}")),
        ["config", "set", rest @ ..] if !rest.is_empty() && rest.len() % 2 == 0 => {
            let mut c = conf();
            for kv in rest.chunks(2) {
                if !KEYS.iter().any(|(k, _)| *k == kv[0]) {
                    fail(format!("unknown key '{}' (smscd config keys)", kv[0]));
                }
                c.insert(kv[0].into(), kv[1].into());
            }
            sys::save_conf(PATH, "smscd (docs/TELEPHONY.md)", KEYS, &c).unwrap_or_else(|e| fail(e));
            println!("smscd: saved (restart smscd to apply)");
        }
        ["enable"] | ["disable"] => {
            sys::service_line("smscd start", "smscd start --wait &", a[0] == "enable").unwrap_or_else(|e| fail(e));
            println!("smscd: {}", if a[0] == "enable" { "starts at boot" } else { "no longer starts at boot" });
        }
        _ => {
            println!("usage: smscd start [--wait] | stop | status | queue | log [n]");
            println!("       smscd config | config keys | config set <key> <value>... | enable | disable");
            std::process::exit(2);
        }
    }
}

struct Bot {
    fd: i32,
    lines: Lines,
    codes: Vec<String>,
}

fn run(wait: bool) {
    let c = conf();
    let g = |k: &str| c.get(k).cloned().unwrap_or_default();
    let mut hss = HssClient::new(Addr::parse(&g("hss"), 3868).unwrap_or_else(|| fail("bad hss")), "smscd", &g("hss_secret"));
    let mut log = Log::new("smscd", true);
    let num = sys::numbering(&mut hss, wait).unwrap_or_else(|e| fail(e));
    let ip = match g("ip").as_str() {
        "" => Addr::parse_ip(&sys::kv(&num, "pcscf")).unwrap_or_else(|| fail("the HSS has no IMS set up (cored ims pool=...) and smscd.toml has no ip")),
        s => Addr::parse_ip(s).unwrap_or_else(|| fail("bad ip")),
    };
    let domain = sys::kv(&num, "domain");
    let mut cfg = SmscConfig::new(&domain, &sys::kv(&num, "carrier"), ip);
    if !g("ims").is_empty() {
        cfg.ims = Addr::parse(&g("ims"), 5060).unwrap_or_else(|| fail("bad ims"));
    }
    cfg.address = g("address");
    cfg.validity_ms = g("validity").parse::<u64>().unwrap_or(140) * 60_000;
    cfg.rate_per_min = g("rate_per_min").parse().unwrap_or(30);
    let mut admin = AdminServer::bind(port(&c, "admin_port")).unwrap_or_else(|e| fail(format!("admin socket: {e} (is smscd already running?)")));
    let sip = sys::udp_bind(cfg.sip_port).unwrap_or_else(|e| fail(e));
    let bots_l = ecm_cell_net::tcp_listen(ecm_cell_net::LOOPBACK, port(&c, "shortcode_port")).unwrap_or_else(|e| fail(format!("short-code socket: {e}")));
    let seed = u64::from_le_bytes(ecm_cell_net::random::<8>());
    let mut smsc = Smsc::new(cfg.clone(), seed);
    let clock = Instant::now();
    let ms = || clock.elapsed().as_millis() as u64;
    if let Ok(t) = std::fs::read_to_string(QUEUE) {
        let n = smsc.load(&t, ms());
        if n > 0 {
            log.line(&format!("smscd: {n} stored text(s) reloaded"));
        }
    }
    log.line(&format!("smscd: SMSC of {} on {}, imsd at {}", domain, Addr::new(ip, cfg.sip_port), cfg.ims));
    let mut bots: Vec<Bot> = Vec::new();
    let mut hss_ids: BTreeMap<u64, u64> = BTreeMap::new();
    let mut next_hss = 100u64;
    let mut buf = vec![0u8; 65536];
    let mut last_tick = 0;
    let mut stop = false;
    let mut submitted = 0u64;
    while !stop {
        let mut fds = vec![sip, bots_l];
        fds.extend(bots.iter().map(|b| b.fd));
        if let Some(f) = hss.fd() {
            fds.push(f);
        }
        let afds = admin.fds();
        fds.extend(&afds);
        let ready = sys::readable(&fds, 10);
        let now = ms();
        if ready[0] {
            for (from, d) in sys::udp_drain(sip, &mut buf, 64) {
                smsc.on_sip(from, &d, now);
            }
        }
        if ready[1] {
            let mut a = ecm_host_abi::socket::SockAddrIn::default();
            let fd = ecm_host_abi::socket::accept(bots_l, &mut a);
            if fd >= 0 {
                bots.push(Bot { fd, lines: Lines::default(), codes: Vec::new() });
            }
        }
        let mut gone = Vec::new();
        for (i, b) in bots.iter_mut().enumerate() {
            if !ready[2 + i] {
                continue;
            }
            match sys::read_now(b.fd, &mut buf) {
                Some(d) if d.is_empty() => {}
                Some(d) => {
                    for l in b.lines.push(&d).unwrap_or_default() {
                        let w: Vec<&str> = l.splitn(4, ' ').collect();
                        let r = match w.as_slice() {
                            ["register", code] if (3..=6).contains(&code.len()) && code.bytes().all(|x| x.is_ascii_digit()) => {
                                b.codes.push(code.to_string());
                                log.line(&format!("short code {code} registered"));
                                "ok".to_string()
                            }
                            ["reply", code, to, text] if b.codes.iter().any(|c| c == code) => {
                                smsc.short_code_reply(code, to, &crate::dec(text), now);
                                "ok".to_string()
                            }
                            _ => "err register <code> | reply <code> <to> <text>".to_string(),
                        };
                        sys::send_all(b.fd, format!("{r}\n").as_bytes());
                    }
                }
                None => gone.push(b.fd),
            }
        }
        for fd in gone {
            ecm_host_abi::socket::close(fd);
            bots.retain(|b| b.fd != fd);
        }
        for (id, r) in hss.pump() {
            if let Some(mid) = hss_ids.remove(&id) {
                smsc.on_hss(mid, r, now);
            }
        }
        let aready = ready[ready.len() - afds.len()..].to_vec();
        {
            let q: Vec<String> = smsc.queued().iter().map(|s| format!("msg {} {} {} {}s {}", s.id, s.from, s.to, now.saturating_sub(s.submitted_ms) / 1000, s.attempts)).collect();
            let log2 = &log;
            let (hss_ok, codes) = (hss.connected(), bots.iter().flat_map(|b| b.codes.clone()).collect::<Vec<_>>());
            admin.serve(&aready, &mut |line| match line.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["status"] => Ok(format!(
                    "domain {}\nip {}\nhss {}\nqueued {}\nsubmitted {}\nshort_codes {}\n",
                    cfg.domain,
                    Addr::new(cfg.ip, cfg.sip_port),
                    if hss_ok { "up" } else { "down" },
                    q.len(),
                    submitted,
                    if codes.is_empty() { "-".into() } else { codes.join(",") }
                )),
                ["queue"] => Ok(q.join("\n")),
                ["log"] => Ok(log2.tail(50)),
                ["log", n] => Ok(log2.tail(n.parse().unwrap_or(50))),
                ["stop"] => {
                    stop = true;
                    Ok("stopping".into())
                }
                _ => Err("unknown command (status, queue, log, stop)".into()),
            });
        }
        if now.saturating_sub(last_tick) >= 50 {
            last_tick = now;
            smsc.tick(now);
        }
        for o in smsc.take_out() {
            match o {
                SmscOut::Sip { to, data } => sys::udp_send(sip, to, &data),
                SmscOut::Save(t) => {
                    if let Err(e) = sys::write_file(QUEUE, t.as_bytes()) {
                        log.line(&e);
                    }
                }
                SmscOut::Hss { id, line } => {
                    next_hss += 1;
                    hss_ids.insert(next_hss, id);
                    hss.request(next_hss, &line);
                }
                SmscOut::ShortCode { code, from, text } => {
                    let mut sent = false;
                    for b in bots.iter().filter(|b| b.codes.contains(&code)) {
                        sent |= sys::send_all(b.fd, format!("sms {} {} {}\n", code, from, crate::enc(&text)).as_bytes());
                    }
                    if !sent {
                        log.line(&format!("text to short code {code} from {from}: no program serves it"));
                    }
                }
                SmscOut::Cdr(l) => {
                    submitted += 1;
                    sys::cdr("smscd", &l);
                }
                SmscOut::Log(l) => log.line(&l),
            }
        }
    }
    log.line("smscd: stopped");
}
