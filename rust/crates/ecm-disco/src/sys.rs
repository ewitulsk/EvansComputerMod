//! Discovery over the computer's sockets (wasm32 only): searching, announcing, `disco serve`.

use crate::*;
use ecm_host_abi::net_config;
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLIN, SOCK_DGRAM};
use std::time::Instant;

/// Every configured interface's address and prefix.
pub fn interfaces() -> Vec<(Ip, u8)> {
    (0..net_config::iface_count())
        .filter_map(|i| {
            let info = net_config::iface_info(i)?;
            let ip = parse_ip(json_field(&info, "ip")?)?;
            let prefix: u8 = json_field(&info, "prefix_len")?.parse().ok()?;
            Some((ip, prefix))
        })
        .collect()
}

pub fn gateway() -> Option<Ip> {
    default_gateway(&net_config::route_list()?)
}

fn seed() -> u32 {
    let mut b = [0u8; 4];
    ecm_host_abi::random::fill(&mut b);
    u32::from_le_bytes(b)
}

/// Search the network for services of `kind` (`*`: every type) for `wait_ms`.
pub fn search(kind: &str, wait_ms: i64) -> Result<Vec<Found>, String> {
    let fd = socket::socket(AF_INET, SOCK_DGRAM, 0);
    if fd == -13 {
        return Err("not allowed to use the network".into());
    }
    if fd < 0 {
        return Err("no network: cannot open a socket".into());
    }
    if socket::bind(fd, &SockAddrIn::any(0)) != 0 {
        socket::close(fd);
        return Err("no network: cannot bind a socket".into());
    }
    let mut s = Search::new(seed() & 0x7fff_ffff, kind);
    let q = s.query();
    for t in targets(&interfaces(), gateway()) {
        // A segment without a route (or a link that is down) just fails: the others still go.
        socket::sendto(fd, &q, 0, &SockAddrIn::from_ip_port(t, PORT));
    }
    let start = Instant::now();
    let mut buf = [0u8; 2048];
    loop {
        let left = wait_ms - start.elapsed().as_millis() as i64;
        if left <= 0 {
            break;
        }
        let mut fds = [PollFd { fd, events: POLLIN, revents: 0 }];
        if socket::poll(&mut fds, left.min(250) as i32) <= 0 || fds[0].revents & POLLIN == 0 {
            continue;
        }
        let mut from = SockAddrIn::default();
        let n = socket::recvfrom(fd, &mut buf, MSG_DONTWAIT, &mut from);
        if n > 0 {
            s.on_datagram(from.ip(), &buf[..n as usize]);
        }
    }
    socket::close(fd);
    Ok(s.results())
}

/// A server announces itself to this computer's responder (`/run/disco/<type>-<port>`).
/// Best effort: a computer without `disco serve` just doesn't answer for it.
pub fn announce(kind: &str, port: u16, name: &str) {
    let (file, line) = announcement(kind, port, name);
    let _ = std::fs::create_dir_all(RUN_DIR);
    if std::fs::write(format!("{RUN_DIR}/{file}"), &line).is_err() {
        let dir = RUN_DIR.trim_start_matches('/');
        ecm_host_abi::fs::mkdir("run");
        ecm_host_abi::fs::mkdir(dir);
        ecm_host_abi::fs::write_file(&format!("{dir}/{file}"), &line);
    }
}

/// The configuration plus the announced services, read again for every query.
pub fn load_config() -> (Config, Vec<String>) {
    let text = std::fs::read_to_string(CONF).unwrap_or_default();
    let (mut c, bad) = Config::parse(&text);
    if let Ok(rd) = std::fs::read_dir(RUN_DIR) {
        let mut names: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        names.sort();
        for p in names {
            if let Ok(t) = std::fs::read_to_string(&p) {
                c.add_announced(&t);
            }
        }
    }
    (c, bad)
}

/// `disco serve`: answer on UDP [`PORT`] until killed (`verbose`: log every query).
pub fn serve(verbose: bool, log: &mut dyn FnMut(&str)) -> Result<(), String> {
    let fd = socket::socket(AF_INET, SOCK_DGRAM, 0);
    if fd < 0 {
        return Err("cannot open a socket".into());
    }
    if socket::bind(fd, &SockAddrIn::any(PORT)) != 0 {
        socket::close(fd);
        return Err(format!("cannot listen on UDP port {PORT} (is disco serve already running?)"));
    }
    let (cfg, bad) = load_config();
    for b in &bad {
        log(&format!("{CONF}: ignored {b}"));
    }
    let relay = match cfg.relay {
        Relay::None => "no relay".to_string(),
        Relay::Gateway => "relaying to the default gateway".to_string(),
        Relay::To(ip) => format!("relaying to {}", ip_str(ip)),
    };
    log(&format!("disco: answering on UDP port {PORT} for {} services, {relay}. Ctrl+T to stop.", cfg.services.len()));
    let mut r = Responder::new(seed());
    let start = Instant::now();
    let mut buf = [0u8; 2048];
    loop {
        let mut fds = [PollFd { fd, events: POLLIN, revents: 0 }];
        if socket::poll(&mut fds, 1000) <= 0 || fds[0].revents & POLLIN == 0 {
            continue;
        }
        let mut from = SockAddrIn::default();
        let n = socket::recvfrom(fd, &mut buf, MSG_DONTWAIT, &mut from);
        if n <= 0 {
            continue;
        }
        let data = &buf[..n as usize];
        let (cfg, _) = load_config();
        let upstream = match cfg.relay {
            Relay::None => None,
            Relay::Gateway => gateway(),
            Relay::To(ip) => Some(ip),
        };
        let now = start.elapsed().as_millis() as i64;
        if let (true, Some(Msg::Query { kind, .. })) = (verbose, Msg::parse(data)) {
            log(&format!("query {} from {}", kind, from.to_string()));
        }
        for o in r.on_datagram(&cfg, upstream, (from.ip(), from.port()), data, now) {
            socket::sendto(fd, &o.msg.encode(), 0, &SockAddrIn::from_ip_port(o.to.0, o.to.1));
        }
    }
}
