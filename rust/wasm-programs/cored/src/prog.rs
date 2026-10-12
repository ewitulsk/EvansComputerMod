//! The program: CLI and the daemon loop (WASI only).

use cored::config::{self, Conf, KEYS};
use cored::daemon::{init_db, Action, Card, CardWriter, Daemon};
use ecm_cell::admin::value;
use ecm_cell::codec::fmt_ip;
use ecm_cell::core::{CoreOut, SockAddr};
use ecm_cell::db::Db;
use ecm_cell::ip::{from_kernel, to_kernel, TunFrame};
use ecm_cell_net::{admin_call, read_now, readable, send_all, tcp_listen, udp_bind, AdminServer, Read, WasiEntropy};
use ecm_host_abi::peripheral::{self, Value};
use ecm_host_abi::socket::{self, SockAddrIn};
use ecm_host_abi::tun;
use std::time::Instant;

const SERVICE_LINE: &str = "cored start &";

fn fail(msg: impl AsRef<str>) -> ! {
    println!("cored: {}", msg.as_ref());
    std::process::exit(1);
}

fn usage() -> ! {
    println!("usage: cored init <carrier> <pool> [--gateway IP] [--dns IP]");
    println!("       cored start | stop | status | sites | subs | sessions | log [n] | handovers");
    println!("       cored site add <id> | site remove <id> | site-add <id> <secret>");
    println!("       cored sim new [--label L] [--quota BYTES]");
    println!("       cored add <imsi> <key> [quota] | revoke <imsi> | unrevoke <imsi> | quota <imsi> <bytes>");
    println!("       cored remove <imsi> | label <imsi> <label> | release <imsi> | reset-usage <imsi>");
    println!("       cored config | config set <key> <value>... | config keys");
    println!("       cored enable | disable");
    std::process::exit(2);
}

fn load_conf() -> Conf {
    match std::fs::read_to_string(config::PATH) {
        Ok(t) => Conf::parse(&t).unwrap_or_else(|e| fail(format!("{}: {e}", config::PATH))),
        Err(_) => Conf::default(),
    }
}

fn load_db(conf: &Conf) -> Db {
    let path = conf.get("db");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| fail(format!("no database at {path}: create one with: cored init <carrier> <pool>  (e.g. cored init cell-red 100.65.2.0/24)")));
    Db::parse(&text).unwrap_or_else(|e| fail(format!("{path}: {e}")))
}

fn write_db(conf: &Conf, text: &str) -> Result<(), String> {
    let path = conf.get("db");
    if let Some(dir) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&path, text).map_err(|e| format!("can't write {path}: {e}"))
}

// ------------------------------------------------------------ the Card Reader/Writer

struct Reader {
    wanted: String,
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
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn flag(m: &Value, key: &str) -> bool {
    matches!(get(m, key), Some(Value::Bool(true)))
}

fn perr(e: peripheral::Error) -> String {
    match e {
        peripheral::Error::Peripheral(m) => m,
        peripheral::Error::Unavailable => "peripherals are not available".into(),
        peripheral::Error::Malformed(w) => format!("bad reply from the computer: {w}"),
    }
}

impl Reader {
    fn name(&self) -> Result<String, String> {
        let list = peripheral::list().map_err(perr)?;
        let readers: Vec<String> = list.into_iter().filter(|(_, t)| t == "card_reader").map(|(n, _)| n).collect();
        if self.wanted.is_empty() {
            readers.into_iter().next().ok_or_else(|| "no Card Reader/Writer next to this computer (place one against any side)".to_string())
        } else if readers.contains(&self.wanted) {
            Ok(self.wanted.clone())
        } else {
            Err(format!("no Card Reader/Writer called '{}'", self.wanted))
        }
    }
}

impl CardWriter for Reader {
    fn card(&mut self) -> Result<Card, String> {
        let n = self.name()?;
        let st = peripheral::call(&n, "status", &[]).map_err(perr)?;
        if !flag(&st, "present") {
            return Ok(Card::Empty);
        }
        let id = peripheral::call(&n, "identity", &[]).map_err(perr)?;
        Ok(if text(&id, "type") != "sim" {
            Card::Other(text(&id, "type"))
        } else if flag(&id, "blank") {
            Card::Blank
        } else {
            Card::Sim { imsi: text(&id, "imsi"), carrier: text(&id, "carrier") }
        })
    }

    fn write(&mut self, imsi: &str, carrier: &str, key_hex: &str) -> Result<bool, String> {
        let n = self.name()?;
        let r = peripheral::call(&n, "write", &[Value::Str(imsi.into()), Value::Str(carrier.into()), Value::Str(key_hex.into())]).map_err(perr)?;
        Ok(flag(&r, "ejected"))
    }
}

// ------------------------------------------------------------ CLI

/// Run an admin command on the running core, or on the database when it isn't running.
fn command(line: &str) -> Result<Vec<String>, String> {
    let conf = load_conf();
    match admin_call(conf.port("admin_port"), line) {
        Err(e) if e == "not running" => {}
        r => return r,
    }
    let db = load_db(&conf);
    let reader = conf.get("reader");
    let mut d = Daemon::new(db, conf.clone(), Box::new(WasiEntropy), Box::new(WasiEntropy), 0);
    let (r, _) = d.handle(line, 0, &mut Reader { wanted: reader });
    for o in d.take_out(0) {
        if let CoreOut::SaveDb(t) = o {
            write_db(&conf, &t)?;
        }
    }
    r.map(|b| b.lines().map(String::from).collect())
}

fn print_or_fail(r: Result<Vec<String>, String>) {
    match r {
        Ok(b) => {
            for l in b {
                println!("{l}");
            }
        }
        Err(e) => fail(e),
    }
}

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["init", carrier, pool, rest @ ..] => init(carrier, pool, rest),
        ["start"] => run(),
        ["stop"] => match admin_call(load_conf().port("admin_port"), "stop") {
            Ok(_) => println!("cored: stopping"),
            Err(e) => fail(e),
        },
        ["status"] => {
            let conf = load_conf();
            match admin_call(conf.port("admin_port"), "status") {
                Ok(b) => {
                    println!("cored: running");
                    for l in b {
                        println!("{l}");
                    }
                }
                Err(e) if e == "not running" => {
                    println!("cored: not running (start it with: cored start &)");
                    print_or_fail(command("status"));
                }
                Err(e) => fail(e),
            }
        }
        ["sites"] | ["subs"] | ["sessions"] | ["handovers"] => print_or_fail(command(a[0])),
        ["log"] | ["log", _] => match admin_call(load_conf().port("admin_port"), &a.join(" ")) {
            Ok(b) => {
                for l in b {
                    println!("{l}");
                }
            }
            Err(e) => fail(e),
        },
        ["site", "add", id] => match command(&format!("site-new {id}")) {
            Ok(b) => {
                let secret = value(&b, "secret").unwrap_or("").to_string();
                println!("cored: site {id} added, secret {secret}");
                println!("  on the site's computer: celld-site config set site_id {id} secret {secret}");
            }
            Err(e) => fail(e),
        },
        ["site", "remove", id] => print_or_fail(command(&format!("site-remove {id}"))),
        ["sim", "new", rest @ ..] => sim_new(rest),
        [cmd, ..] if ["add", "revoke", "unrevoke", "quota", "reset-usage", "remove", "label", "release", "site-add"].contains(cmd) => print_or_fail(command(&a.join(" "))),
        ["config"] => print!("{}", load_conf().to_text()),
        ["config", "keys"] => {
            for k in KEYS {
                println!("{:<12} {} (default {})", k.name, k.help, if k.default.is_empty() { "empty" } else { k.default });
            }
        }
        ["config", "set", rest @ ..] if !rest.is_empty() && rest.len() % 2 == 0 => {
            let mut c = load_conf();
            for kv in rest.chunks(2) {
                c.set(kv[0], kv[1]).unwrap_or_else(|e| fail(e));
            }
            let _ = std::fs::create_dir_all("/etc");
            std::fs::write(config::PATH, c.to_text()).unwrap_or_else(|e| fail(format!("can't write {}: {e}", config::PATH)));
            println!("cored: saved (restart cored to apply)");
        }
        ["enable"] | ["disable"] => {
            let path = "/services.cfg";
            let t = std::fs::read_to_string(path).unwrap_or_default();
            let mut lines: Vec<String> = t.lines().filter(|l| l.trim() != SERVICE_LINE).map(String::from).collect();
            if a[0] == "enable" {
                lines.push(SERVICE_LINE.into());
            }
            std::fs::write(path, format!("{}\n", lines.join("\n"))).unwrap_or_else(|e| fail(format!("can't write services.cfg: {e}")));
            println!("cored: {}", if a[0] == "enable" { "starts at boot" } else { "no longer starts at boot" });
        }
        _ => usage(),
    }
}

fn init(carrier: &str, pool: &str, rest: &[&str]) {
    let (mut gw, mut dns, mut force) = (None, None, false);
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "--gateway" if i + 1 < rest.len() => {
                gw = Some(rest[i + 1]);
                i += 1;
            }
            "--dns" if i + 1 < rest.len() => {
                dns = Some(rest[i + 1]);
                i += 1;
            }
            "--force" => force = true,
            _ => usage(),
        }
        i += 1;
    }
    let conf = load_conf();
    let path = conf.get("db");
    if !force && std::fs::metadata(&path).is_ok() {
        fail(format!("{path} exists (cored init ... --force replaces it, subscribers and all)"));
    }
    let text = init_db(carrier, pool, gw, dns).unwrap_or_else(|e| fail(e));
    write_db(&conf, &text).unwrap_or_else(|e| fail(e));
    let db = Db::parse(&text).unwrap_or_else(|e| fail(e.msg));
    println!("cored: carrier {} pool {}/{} gateway {} in {path}", db.carrier, fmt_ip(db.pool), db.prefix, fmt_ip(db.gateway));
}

fn sim_new(rest: &[&str]) {
    let (mut label, mut quota) = (None, None);
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "--label" if i + 1 < rest.len() => {
                label = Some(rest[i + 1]);
                i += 1;
            }
            "--quota" if i + 1 < rest.len() => {
                quota = Some(rest[i + 1].parse::<u64>().unwrap_or_else(|_| fail("--quota takes bytes (0 = unlimited)")));
                i += 1;
            }
            _ => usage(),
        }
        i += 1;
    }
    let cmd = match (label, quota) {
        (Some(l), q) => format!("sim-new {l} {}", q.unwrap_or(0)),
        (None, Some(q)) => format!("sim-new {q}"),
        (None, None) => "sim-new".into(),
    };
    match command(&cmd) {
        Ok(b) => {
            let imsi = value(&b, "imsi").unwrap_or("?");
            let l = value(&b, "label").filter(|l| *l != "-").map(|l| format!(" ({l})")).unwrap_or_default();
            println!("cored: wrote SIM IMSI {imsi}{l}");
            if value(&b, "ejected") == Some("1") {
                println!("cored: the card was ejected into an adjacent inventory");
            } else {
                println!("cored: the card is released: take it out of the reader");
            }
        }
        Err(e) => fail(e),
    }
}

// ------------------------------------------------------------ the daemon

fn run() {
    let conf = load_conf();
    let db = load_db(&conf);
    let mut admin = AdminServer::bind(conf.port("admin_port")).unwrap_or_else(|e| fail(format!("admin socket: {e} (is cored already running?)")));
    let lis = tcp_listen([0; 4], conf.port("listen")).unwrap_or_else(|e| fail(format!("control port: {e}")));
    let udp = udp_bind(conf.port("tunnel_port")).unwrap_or_else(|e| fail(format!("tunnel port: {e}")));
    let (gw, prefix) = (db.gateway, db.prefix);
    let mac = [0x02, 0xce, 0xc0, 0x4e, gw[2], gw[3]];
    let tun_name = conf.get("tun");
    let tfd = tun::open(&tun_name, mac, gw, prefix);
    if tfd < 0 {
        fail(format!("can't create {tun_name} ({tfd}): is another cored running?"));
    }
    let clock = Instant::now();
    let ms = || clock.elapsed().as_millis() as u64;
    println!(
        "cored: carrier {} pool {}/{}, {} {}/{}; sites connect to port {} (tunnels {})",
        db.carrier,
        fmt_ip(db.pool),
        db.prefix,
        tun_name,
        fmt_ip(gw),
        prefix,
        conf.get("listen"),
        conf.get("tunnel_port")
    );
    let reader = conf.get("reader");
    let mut d = Daemon::new(db, conf.clone(), Box::new(WasiEntropy), Box::new(WasiEntropy), ms());
    d.log(ms(), &format!("started: {} subscribers, {} sites configured", d.core.db().subs.len(), d.core.db().sites.len()));
    let mut conns: Vec<i32> = Vec::new();
    let mut buf = vec![0u8; 4096];
    let mut stop = false;
    while !stop {
        let mut fds = vec![lis, udp];
        fds.extend(&conns);
        let afds = admin.fds();
        fds.extend(&afds);
        let ready = readable(&fds, 5);
        let now = ms();
        if ready[0] {
            let mut a = SockAddrIn::default();
            let fd = socket::accept(lis, &mut a);
            if fd >= 0 {
                conns.push(fd);
                d.core.on_connect(fd as u32, a.ip(), now);
            }
        }
        if ready[1] {
            for _ in 0..64 {
                let mut from = SockAddrIn::default();
                let n = socket::recvfrom(udp, &mut buf, socket::MSG_DONTWAIT, &mut from);
                if n <= 0 {
                    break;
                }
                d.core.on_tunnel(SockAddr { ip: from.ip(), port: from.port() }, &buf[..n as usize], now);
            }
        }
        let snapshot = conns.clone();
        for (i, fd) in snapshot.iter().enumerate() {
            if !ready[2 + i] {
                continue;
            }
            match read_now(*fd, &mut buf) {
                Read::Data(b) => d.core.on_bytes(*fd as u32, &b, now),
                Read::Nothing => {}
                Read::Closed => {
                    socket::close(*fd);
                    conns.retain(|c| c != fd);
                    d.core.on_disconnect(*fd as u32, now);
                }
            }
        }
        for _ in 0..64 {
            let n = tun::read(tfd, &mut buf);
            if n == tun::TUN_EMPTY {
                break;
            }
            if n < 0 {
                fail(format!("{tun_name} went away ({n})"));
            }
            match from_kernel(&buf[..n as usize], gw) {
                TunFrame::ArpReply(r) => {
                    tun::write(tfd, &r);
                }
                TunFrame::Ipv4(p) => d.core.on_lan_packet(p, now),
                TunFrame::Other => {}
            }
        }
        let aready = ready[ready.len() - afds.len()..].to_vec();
        let mut card = Reader { wanted: reader.clone() };
        {
            let dd = &mut d;
            admin.serve(&aready, &mut |line| {
                let (r, a) = dd.handle(line, ms(), &mut card);
                if a == Action::Stop {
                    stop = true;
                }
                r
            });
        }
        d.core.tick(now);
        for o in d.take_out(ms()) {
            match o {
                CoreOut::Send(c, b) => {
                    let fd = c as i32;
                    if conns.contains(&fd) && !send_all(fd, &b) {
                        socket::close(fd);
                        conns.retain(|x| *x != fd);
                        d.core.on_disconnect(c, now);
                    }
                }
                CoreOut::Close(c) => {
                    let fd = c as i32;
                    if conns.contains(&fd) {
                        socket::close(fd);
                        conns.retain(|x| *x != fd);
                    }
                }
                CoreOut::Tunnel(addr, dg) => {
                    socket::sendto(udp, &dg, 0, &SockAddrIn::from_ip_port(addr.ip, addr.port));
                }
                CoreOut::ToLan(p) => {
                    tun::write(tfd, &to_kernel(&p, mac));
                }
                CoreOut::SaveDb(t) => {
                    if let Err(e) = write_db(&conf, &t) {
                        d.log(ms(), &e);
                    }
                }
                CoreOut::Log(l) => d.log(ms(), &l),
            }
        }
    }
    for fd in conns {
        socket::close(fd);
    }
    socket::close(lis);
    socket::close(udp);
    socket::close(tfd);
    admin.close();
    println!("cored: stopped");
}
