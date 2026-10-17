//! named: the DNS server (docs/DNS.md), like BIND.
//!
//! `named [-c FILE] [-q]` runs the daemon (start it with `named &` or from
//! `services.cfg`): authoritative for the zones in `/etc/named.conf`, recursive for
//! allowed clients when `recursion yes`, secondaries kept fresh by NOTIFY and AXFR.
//! `named reload | status | cache | flush [NAME] | retransfer ZONE | stop` talk to the
//! running daemon on 127.0.0.1 port 953. `named check [FILE]` checks the configuration
//! and every primary zone file without starting anything.

use ecm_dns::conf::{NamedConf, ZoneKind, CONTROL_PORT, PATH};
use ecm_dns::name::{fqdn, ip_str, Ip};
use ecm_dns::server::{Out, Server};
use ecm_dns::sys::{self, now_ms};
use ecm_dns::xfr::{frame, Deframer};
use ecm_dns::zone::Zone;
use ecm_host_abi::socket::{
    self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLERR, POLLHUP, POLLIN, SOCK_DGRAM, SOCK_STREAM, SOL_SOCKET, SO_REUSEADDR,
};
use std::io::Write;

fn log(text: &str) {
    let t = now_ms() / 1000;
    println!("[{:02}:{:02}:{:02}] named: {}", (t / 3600) % 24, (t / 60) % 60, t % 60, text);
    let _ = std::io::stdout().flush();
}

fn usage() {
    println!("Usage: named [-c FILE] [-q]        run the DNS server (named &)");
    println!("       named reload                re-read named.conf and the zone files");
    println!("       named status                zones, serials, recursion, counters");
    println!("       named cache                 what the resolver has cached");
    println!("       named flush [NAME]          forget cached answers");
    println!("       named retransfer ZONE       a secondary checks its primary now");
    println!("       named check [FILE]          check named.conf and the primary zones");
    println!("       named stop                  stop the daemon");
}

fn load_conf(path: &str) -> Result<NamedConf, String> {
    let text = std::fs::read_to_string(path).map_err(|_| format!("cannot read {path}"))?;
    NamedConf::parse(&text)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut conf_path = PATH.to_string();
    let mut quiet = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" | "help" => return usage(),
            "-c" => {
                i += 1;
                conf_path = args.get(i).cloned().unwrap_or_default();
            }
            "-q" => quiet = true,
            "reload" | "status" | "cache" | "flush" | "retransfer" | "stop" => {
                let cmd = args[i..].join(" ");
                match sys::control(&cmd, CONTROL_PORT) {
                    Ok(reply) => {
                        if !reply.is_empty() {
                            println!("{reply}");
                        }
                        if reply.starts_with("error") {
                            std::process::exit(1);
                        }
                    }
                    Err(e) => {
                        eprintln!("named: {e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            "check" => {
                let path = args.get(i + 1).cloned().unwrap_or(conf_path);
                std::process::exit(if check(&path) { 0 } else { 1 });
            }
            a => {
                eprintln!("named: unknown argument '{a}'");
                usage();
                std::process::exit(1);
            }
        }
        i += 1;
    }
    let conf = match load_conf(&conf_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("named: {e}");
            std::process::exit(1);
        }
    };
    run(conf, &conf_path, quiet);
}

fn check(path: &str) -> bool {
    let conf = match load_conf(path) {
        Ok(c) => c,
        Err(e) => {
            println!("{e}");
            return false;
        }
    };
    let mut ok = true;
    for z in &conf.zones {
        match &z.kind {
            ZoneKind::Primary { file, .. } => match std::fs::read_to_string(file) {
                Ok(t) => match Zone::parse(&t, &z.name) {
                    Ok(zone) => println!("zone {}: {} OK, serial {}, {} records", fqdn(&z.name), file, zone.serial(), zone.records.len()),
                    Err(e) => {
                        println!("zone {}: {} {}", fqdn(&z.name), file, e);
                        ok = false;
                    }
                },
                Err(_) => {
                    println!("zone {}: cannot read {}", fqdn(&z.name), file);
                    ok = false;
                }
            },
            ZoneKind::Secondary { primary, .. } => println!("zone {}: secondary of {}", fqdn(&z.name), ip_str(*primary)),
        }
    }
    if let Some(h) = &conf.root_hints {
        match std::fs::read_to_string(h).map_err(|_| "cannot read".to_string()).and_then(|t| ecm_dns::resolver::parse_hints(&t)) {
            Ok(v) => println!("root hints {}: {} servers", h, v.len()),
            Err(e) => {
                println!("root hints {h}: {e}");
                ok = false;
            }
        }
    }
    ok
}

fn udp(ip: Option<Ip>, port: u16) -> i32 {
    let fd = socket::socket(AF_INET, SOCK_DGRAM, 0);
    if fd < 0 {
        return fd;
    }
    let addr = match ip {
        Some(ip) => SockAddrIn::from_ip_port(ip, port),
        None => SockAddrIn::any(port),
    };
    if socket::bind(fd, &addr) != 0 {
        socket::close(fd);
        return -1;
    }
    fd
}

struct Conn {
    fd: i32,
    peer: Ip,
    d: Deframer,
    last: i64,
}

fn reader(p: &str) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

fn run(conf: NamedConf, conf_path: &str, quiet: bool) {
    let ctl = udp(Some([127, 0, 0, 1]), CONTROL_PORT);
    if ctl < 0 {
        eprintln!("named: already running (the control port {CONTROL_PORT} is taken)");
        std::process::exit(1);
    }
    let port = conf.port;
    // One socket per address (as BIND does): an answer leaves from the address it was
    // asked on, which a client checks. A computer with several interfaces (a router) would
    // otherwise answer from its outgoing interface's address and be ignored.
    let own = sys::own_addresses();
    let mut addrs: Vec<Option<Ip>> = match conf.listen {
        Some(ip) => vec![Some(ip)],
        None if own.is_empty() => vec![None],
        None => own.iter().map(|ip| Some(*ip)).chain(std::iter::once(Some([127, 0, 0, 1]))).collect(),
    };
    addrs.dedup();
    let mains: Vec<i32> = addrs.iter().map(|a| udp(*a, port)).filter(|fd| *fd >= 0).collect();
    if mains.is_empty() {
        eprintln!("named: cannot listen on UDP port {port} (is another DNS server running?)");
        std::process::exit(1);
    }
    // Which socket each client asked on, for its answer.
    let mut asked_on: std::collections::HashMap<(Ip, u16), i32> = std::collections::HashMap::new();
    let res_fd = udp(None, 0);
    let tcp = socket::socket(AF_INET, SOCK_STREAM, 0);
    socket::setsockopt(tcp, SOL_SOCKET, SO_REUSEADDR, &1i32.to_le_bytes());
    let tcp_ok = socket::bind(tcp, &SockAddrIn::any(port)) == 0 && socket::listen(tcp, 8) == 0;
    if !tcp_ok {
        log(&format!("cannot listen on TCP port {port}: no zone transfers from here"));
    }
    let mut seed = [0u8; 4];
    ecm_host_abi::random::fill(&mut seed);
    let mut srv = Server::load(conf, &reader, own, u32::from_le_bytes(seed), now_ms());
    let where_ = srv.conf.listen.map(ip_str).unwrap_or_else(|| "all addresses".into());
    log(&format!(
        "listening on {} port {}, {} zone(s), recursion {}. Ctrl+T to stop.",
        where_,
        port,
        srv.conf.zones.len(),
        if srv.conf.recursion { "yes" } else { "no" }
    ));
    let mut conns: Vec<Conn> = Vec::new();
    let mut buf = vec![0u8; 4096];
    loop {
        // Everything the server wants done.
        for o in srv.take_out() {
            match o {
                Out::Udp { to, port, data } => {
                    let fd = asked_on.get(&(to, port)).copied().unwrap_or(mains[0]);
                    socket::sendto(fd, &data, 0, &SockAddrIn::from_ip_port(to, port));
                }
                Out::Resolver { to, data } => {
                    socket::sendto(res_fd, &data, 0, &SockAddrIn::from_ip_port(to, 53));
                }
                Out::Log(l) => {
                    if !quiet || l.contains("SERVFAIL") || l.contains("transfer") || l.contains("NOTIFY") || l.contains("loaded") {
                        log(&l);
                    }
                }
                Out::Save { file, text } => {
                    if !sys::write_file(&file, &text) {
                        log(&format!("cannot write {file}"));
                    }
                }
                Out::Transfer { zone, primary } => {
                    log(&format!("zone {}: transferring from {}", fqdn(&zone), ip_str(primary)));
                    let r = sys::axfr(primary, &zone, 5000);
                    srv.on_transfer(&zone, r, now_ms());
                }
            }
        }
        let now = now_ms();
        let wait = srv.next_deadline().map_or(500, |d| (d - now).clamp(1, 500));
        let mut fds: Vec<PollFd> = mains.iter().map(|&fd| PollFd { fd, events: POLLIN, revents: 0 }).collect();
        fds.extend([
            PollFd { fd: res_fd, events: POLLIN, revents: 0 },
            PollFd { fd: ctl, events: POLLIN, revents: 0 },
        ]);
        if tcp_ok {
            fds.push(PollFd { fd: tcp, events: POLLIN, revents: 0 });
        }
        for c in &conns {
            fds.push(PollFd { fd: c.fd, events: POLLIN, revents: 0 });
        }
        if socket::poll(&mut fds, wait as i32) < 0 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let now = now_ms();
        let ready = |fd: i32| fds.iter().any(|f| f.fd == fd && f.revents & (POLLIN | POLLERR | POLLHUP) != 0);
        for &main in &mains {
            if !ready(main) {
                continue;
            }
            loop {
                let mut from = SockAddrIn::default();
                let n = socket::recvfrom(main, &mut buf, MSG_DONTWAIT, &mut from);
                if n <= 0 {
                    break;
                }
                if asked_on.len() > 4096 {
                    asked_on.clear();
                }
                asked_on.insert((from.ip(), from.port()), main);
                srv.on_udp(from.ip(), from.port(), &buf[..n as usize], now);
            }
        }
        if ready(res_fd) {
            loop {
                let mut from = SockAddrIn::default();
                let n = socket::recvfrom(res_fd, &mut buf, MSG_DONTWAIT, &mut from);
                if n <= 0 {
                    break;
                }
                if from.port() == 53 {
                    srv.on_resolver_udp(from.ip(), &buf[..n as usize], now);
                }
            }
        }
        if ready(ctl) {
            let mut from = SockAddrIn::default();
            let n = socket::recvfrom(ctl, &mut buf, MSG_DONTWAIT, &mut from);
            if n > 0 && from.ip() == [127, 0, 0, 1] {
                let cmd = String::from_utf8_lossy(&buf[..n as usize]).trim().to_string();
                let (reply, stop) = control(&mut srv, &cmd, conf_path, now);
                for chunk in sys::control_chunks(&reply) {
                    socket::sendto(ctl, &chunk, 0, &from);
                }
                if stop {
                    log("stopping");
                    return;
                }
            }
        }
        if tcp_ok && ready(tcp) {
            let mut peer = SockAddrIn::default();
            let c = socket::accept(tcp, &mut peer);
            if c >= 0 {
                if conns.len() >= 16 {
                    socket::close(c);
                } else {
                    conns.push(Conn { fd: c, peer: peer.ip(), d: Deframer::default(), last: now });
                }
            }
        }
        let mut gone = Vec::new();
        for (ci, c) in conns.iter_mut().enumerate() {
            if !ready(c.fd) {
                if now - c.last > 15_000 {
                    gone.push(ci);
                }
                continue;
            }
            let n = socket::recv(c.fd, &mut buf, MSG_DONTWAIT);
            if n == -2 {
                continue;
            }
            if n <= 0 {
                gone.push(ci);
                continue;
            }
            c.last = now;
            for msg in c.d.push(&buf[..n as usize]) {
                for answer in srv.on_tcp(c.peer, &msg, now) {
                    if !sys::send_all(c.fd, &frame(&answer.encode())) {
                        break;
                    }
                }
            }
        }
        for ci in gone.into_iter().rev() {
            let c = conns.remove(ci);
            socket::close(c.fd);
        }
        srv.tick(now_ms());
    }
}

/// One control command: the reply text and whether to stop.
fn control(srv: &mut Server, cmd: &str, conf_path: &str, now: i64) -> (String, bool) {
    let w: Vec<&str> = cmd.split_whitespace().collect();
    match w.as_slice() {
        ["reload"] => match load_conf(conf_path) {
            Ok(c) => {
                srv.reload(c, &reader, now);
                let mut lines = vec!["reloaded".to_string()];
                for o in srv.take_out() {
                    if let Out::Log(l) = &o {
                        lines.push(l.clone());
                        log(l);
                    } else {
                        // keep NOTIFYs and sends for the main loop
                        requeue(srv, o);
                    }
                }
                (lines.join("\n"), false)
            }
            Err(e) => (format!("error: {e}"), false),
        },
        ["status"] => (srv.status().join("\n"), false),
        ["cache"] => {
            let lines = srv.cache_dump(now);
            if lines.is_empty() {
                ("(the cache is empty)".into(), false)
            } else {
                (lines.join("\n"), false)
            }
        }
        ["flush"] => {
            srv.flush(None);
            ("cache flushed".into(), false)
        }
        ["flush", name] => match ecm_dns::name::normalize(name) {
            Some(n) => {
                srv.flush(Some(&n));
                (format!("flushed {}", fqdn(&n)), false)
            }
            None => (format!("error: bad name '{name}'"), false),
        },
        ["retransfer", zone] => {
            let z = ecm_dns::name::normalize(zone).unwrap_or_default();
            if srv.retransfer(&z, now) {
                (format!("zone {}: checking the primary", fqdn(&z)), false)
            } else {
                (format!("error: {} is not a secondary zone here", fqdn(&z)), false)
            }
        }
        ["stop"] => ("stopping".into(), true),
        _ => (format!("error: unknown command '{cmd}'"), false),
    }
}

/// Outputs that came out during a control command go back to the main loop.
fn requeue(srv: &mut Server, o: Out) {
    srv.push_out(o);
}
