//! registryd: a TLD registry (docs/DNS.md "The registrar"). Registrars send EPP-lite
//! commands on TCP 700 (`ecm_dns::registry`); every change rewrites the zone file and has
//! the local `named` reload it, which NOTIFYs the secondaries.
//!
//! `registryd [-c FILE]` runs it (`registryd &`); `/etc/registryd.conf`:
//! `zone ecm /etc/named/ecm.zone`, `port 700`, `registrar <id> <secret>` (one per accredited
//! registrar), `db /etc/registry.db`. `registryd show` lists the domains.

use ecm_dns::name::{fqdn, normalize};
use ecm_dns::registry::{Registry, Session};
use ecm_dns::sys::{self, now_ms};
use ecm_dns::zone::Zone;
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, MSG_DONTWAIT, POLLERR, POLLHUP, POLLIN};
use std::io::Write;

struct Conf {
    tld: String,
    zone: String,
    port: u16,
    db: String,
    registrars: Vec<(String, String)>,
}

fn log(text: &str) {
    let t = now_ms() / 1000;
    println!("[{:02}:{:02}:{:02}] registryd: {}", (t / 3600) % 24, (t / 60) % 60, t % 60, text);
    let _ = std::io::stdout().flush();
}

fn conf(path: &str) -> Result<Conf, String> {
    let text = std::fs::read_to_string(path).map_err(|_| format!("cannot read {path}"))?;
    let mut c = Conf { tld: String::new(), zone: String::new(), port: ecm_dns::registry::PORT, db: "/etc/registry.db".into(), registrars: Vec::new() };
    for line in text.lines() {
        let w: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
        match w.as_slice() {
            [] => {}
            ["zone", t, f] => {
                c.tld = normalize(t).ok_or_else(|| format!("bad zone '{t}'"))?;
                c.zone = f.to_string();
            }
            ["port", p] => c.port = p.parse().map_err(|_| format!("bad port '{p}'"))?,
            ["db", f] => c.db = f.to_string(),
            ["registrar", id, secret] => c.registrars.push((id.to_string(), secret.to_string())),
            _ => return Err(format!("{path}: cannot read '{line}'")),
        }
    }
    if c.zone.is_empty() {
        return Err(format!("{path}: no 'zone TLD FILE' line"));
    }
    Ok(c)
}

fn load(c: &Conf) -> Registry {
    let mut r = std::fs::read_to_string(&c.db).ok().and_then(|t| Registry::parse(&t).ok()).unwrap_or_else(|| Registry::new(&c.tld));
    r.tld = c.tld.clone();
    r.registrars = c.registrars.iter().cloned().collect();
    r
}

/// Write the zone from the database (serial + 1) and have named reload it.
fn publish(c: &Conf, r: &Registry) -> Result<u32, String> {
    let text = std::fs::read_to_string(&c.zone).map_err(|_| format!("cannot read {}", c.zone))?;
    let base = Zone::parse(&text, &c.tld).map_err(|e| format!("{}: {e}", c.zone))?;
    let mut z = r.zone(&base);
    let serial = z.bump_serial(0);
    let header = format!("; {}: written by registryd from {} (do not edit by hand)\n", fqdn(&c.tld), c.db);
    if !sys::write_file(&c.zone, &(header + &z.to_text())) || !sys::write_file(&c.db, &r.render()) {
        return Err("cannot write the zone or the database".into());
    }
    match sys::control("reload", ecm_dns::conf::CONTROL_PORT) {
        Ok(_) => {}
        Err(e) => log(&format!("named reload: {e}")),
    }
    Ok(serial)
}

struct Conn {
    fd: i32,
    sess: Session,
    buf: Vec<u8>,
    last: i64,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut path = "/etc/registryd.conf".to_string();
    let mut show = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-c" => {
                i += 1;
                path = args.get(i).cloned().unwrap_or_default();
            }
            "show" => show = true,
            "-h" | "--help" => {
                println!("Usage: registryd [-c FILE]      run the registry (registryd &)");
                println!("       registryd show           the registered domains");
                return;
            }
            a => {
                eprintln!("registryd: unknown argument '{a}'");
                std::process::exit(1);
            }
        }
        i += 1;
    }
    let c = match conf(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("registryd: {e}");
            std::process::exit(1);
        }
    };
    let mut reg = load(&c);
    if show {
        println!("{} domain(s) under .{}", reg.domains.len(), reg.tld);
        for d in reg.domains.values() {
            let how = if d.hosted() { format!("{} record(s), hosted", d.records.len()) } else { format!("delegated to {}", d.ns.iter().map(|(n, _)| fqdn(n)).collect::<Vec<_>>().join(" ")) };
            println!("  {} ({}, via {}): {}", d.name, d.registrant, d.registrar, how);
        }
        return;
    }
    let listener = match sys::listen(c.port) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("registryd: {e}");
            std::process::exit(1);
        }
    };
    log(&format!(
        "the .{} registry: EPP-lite on port {}, {} domain(s), {} registrar(s). Ctrl+T to stop.",
        reg.tld,
        c.port,
        reg.domains.len(),
        reg.registrars.len()
    ));
    let mut conns: Vec<Conn> = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        let mut fds = vec![PollFd { fd: listener, events: POLLIN, revents: 0 }];
        for cn in &conns {
            fds.push(PollFd { fd: cn.fd, events: POLLIN, revents: 0 });
        }
        if socket::poll(&mut fds, 1000) < 0 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let now = now_ms();
        if fds[0].revents & POLLIN != 0 {
            let mut peer = SockAddrIn::default();
            let fd = socket::accept(listener, &mut peer);
            if fd >= 0 {
                if conns.len() >= 16 {
                    socket::close(fd);
                } else {
                    sys::send_all(fd, format!("OK registryd .{} EPP-lite ready\r\n", reg.tld).as_bytes());
                    conns.push(Conn { fd, sess: Session::default(), buf: Vec::new(), last: now });
                }
            }
        }
        let mut gone = Vec::new();
        for (ci, cn) in conns.iter_mut().enumerate() {
            let ready = fds.iter().any(|f| f.fd == cn.fd && f.revents & (POLLIN | POLLERR | POLLHUP) != 0);
            if !ready {
                if now - cn.last > 60_000 {
                    gone.push(ci);
                }
                continue;
            }
            let n = socket::recv(cn.fd, &mut buf, MSG_DONTWAIT);
            if n == -2 {
                continue;
            }
            if n <= 0 {
                gone.push(ci);
                continue;
            }
            cn.last = now;
            cn.buf.extend_from_slice(&buf[..n as usize]);
            while let Some(p) = cn.buf.iter().position(|&b| b == b'\n') {
                let raw: Vec<u8> = cn.buf.drain(..=p).collect();
                let line = String::from_utf8_lossy(&raw).trim().to_string();
                if line.is_empty() {
                    continue;
                }
                let r = reg.command(&mut cn.sess, &line, (now / 1000) as u64);
                let shown = if line.to_ascii_uppercase().starts_with("LOGIN") { "LOGIN ***".to_string() } else { line.clone() };
                let who = cn.sess.registrar.clone().unwrap_or_else(|| "-".into());
                log(&format!("{who}: {shown} -> {}", r.text.lines().next().unwrap_or("")));
                let mut text = r.text.clone();
                if r.changed {
                    match publish(&c, &reg) {
                        Ok(serial) => log(&format!("zone {} serial {serial} written, named reloaded", fqdn(&reg.tld))),
                        Err(e) => {
                            log(&e);
                            text = format!("ERR 2400 {e}");
                        }
                    }
                }
                sys::send_all(cn.fd, format!("{text}\r\n").as_bytes());
                if r.close {
                    gone.push(ci);
                    break;
                }
            }
        }
        gone.sort();
        gone.dedup();
        for ci in gone.into_iter().rev() {
            let cn = conns.remove(ci);
            socket::close(cn.fd);
        }
    }
}
