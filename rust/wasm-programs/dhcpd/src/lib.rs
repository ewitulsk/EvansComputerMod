//! `dhcpd` logic without host calls: the config file and the per-frame
//! server, so it is tested with plain `cargo test -p dhcpd`.
//!
//! Config (`/etc/dhcpd.conf` by default), one statement per line, `#`
//! starts a comment:
//!
//! ```text
//! pool eth0 192.168.50.10 192.168.50.100 router 192.168.50.1 dns 1.1.1.1 lease 3600
//! pool eth1 10.20.0.100 10.20.0.199 prefix 24
//! reserve 02:00:00:00:00:42 192.168.50.5
//! server eth1 10.20.0.1
//! ```
//!
//! - `pool <iface> <first> <last> [prefix <n> | netmask <mask>]
//!   [router <ip>] [dns <ip>] [lease <seconds>]`: one pool per interface.
//!   Defaults: prefix 24, lease 3600 s, no router/DNS option.
//! - `reserve <mac> <ip>`: that MAC always gets that address (it must be in
//!   a pool's subnet; it may be outside the range).
//! - `server <iface> <ip>`: the server identifier on an interface without an
//!   address of its own (otherwise the interface address is used, then the
//!   pool's router).

use std::collections::BTreeMap;

use ecm_net::dhcp::{self, Message};
use ecm_net::dhcp_server::{
    parse_client_frame, parse_mac, reply_frame, LeaseDb, PoolSpec, Reservation,
};
use ecm_net::{Ipv4Addr, MacAddr};

pub const DEFAULT_CONFIG: &str = "/etc/dhcpd.conf";
pub const DEFAULT_LEASES: &str = "/var/dhcpd.leases";
pub const DEFAULT_LEASE_SECS: u32 = 3600;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// One per interface; `PoolSpec::name` is the interface name.
    pub pools: Vec<PoolSpec>,
    pub reservations: Vec<Reservation>,
    pub servers: BTreeMap<String, Ipv4Addr>,
}

fn ip(s: &str, line: usize) -> Result<Ipv4Addr, String> {
    Ipv4Addr::parse(s).ok_or_else(|| format!("line {}: bad address '{}'", line, s))
}

impl Config {
    /// Parse a config file. Every bad line is reported (`line N: ...`).
    pub fn parse(text: &str) -> Result<Config, Vec<String>> {
        let mut cfg = Config::default();
        let mut errors = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let line = n + 1;
            let body = raw.split('#').next().unwrap_or("").trim();
            if body.is_empty() {
                continue;
            }
            let w: Vec<&str> = body.split_whitespace().collect();
            let r = match w[0] {
                "pool" => Self::pool(&w, line).and_then(|p| {
                    if cfg.pools.iter().any(|q| q.name == p.name) {
                        Err(format!("line {}: second pool for {}", line, p.name))
                    } else {
                        cfg.pools.push(p);
                        Ok(())
                    }
                }),
                "reserve" if w.len() == 3 => match (parse_mac(w[1]), ip(w[2], line)) {
                    (Some(mac), Ok(address)) => {
                        if cfg.reservations.iter().any(|r| r.mac == mac || r.address == address) {
                            Err(format!("line {}: duplicate reservation", line))
                        } else {
                            cfg.reservations.push(Reservation { mac, address });
                            Ok(())
                        }
                    }
                    (None, _) => Err(format!("line {}: bad MAC '{}'", line, w[1])),
                    (_, Err(e)) => Err(e),
                },
                "server" if w.len() == 3 => ip(w[2], line).map(|a| {
                    cfg.servers.insert(w[1].to_string(), a);
                }),
                other => Err(format!("line {}: unknown or incomplete statement '{}'", line, other)),
            };
            if let Err(e) = r {
                errors.push(e);
            }
        }
        for r in &cfg.reservations {
            if !cfg.pools.iter().any(|p| p.in_subnet(r.address)) {
                errors.push(format!("reservation {} is in no pool's subnet", r.address));
            }
        }
        if cfg.pools.is_empty() && errors.is_empty() {
            errors.push("no pool configured".to_string());
        }
        if errors.is_empty() {
            Ok(cfg)
        } else {
            Err(errors)
        }
    }

    fn pool(w: &[&str], line: usize) -> Result<PoolSpec, String> {
        if w.len() < 4 {
            return Err(format!("line {}: pool <iface> <first> <last> [options]", line));
        }
        let mut p = PoolSpec {
            name: w[1].to_string(),
            start: ip(w[2], line)?,
            end: ip(w[3], line)?,
            prefix: 24,
            router: None,
            dns: None,
            lease_secs: DEFAULT_LEASE_SECS,
        };
        let mut i = 4;
        while i < w.len() {
            let v = w
                .get(i + 1)
                .ok_or_else(|| format!("line {}: '{}' needs a value", line, w[i]))?;
            match w[i] {
                "router" => p.router = Some(ip(v, line)?),
                "dns" => p.dns = Some(ip(v, line)?),
                "lease" => {
                    p.lease_secs = v
                        .parse()
                        .map_err(|_| format!("line {}: bad lease '{}'", line, v))?
                }
                "prefix" => {
                    p.prefix = v
                        .trim_start_matches('/')
                        .parse()
                        .map_err(|_| format!("line {}: bad prefix '{}'", line, v))?
                }
                "netmask" => {
                    let m = u32::from_be_bytes(ip(v, line)?.0);
                    if m.leading_ones() + m.trailing_zeros() != 32 {
                        return Err(format!("line {}: bad netmask '{}'", line, v));
                    }
                    p.prefix = m.leading_ones() as u8;
                }
                o => return Err(format!("line {}: unknown pool option '{}'", line, o)),
            }
            i += 2;
        }
        if !p.is_valid() {
            return Err(format!(
                "line {}: pool {} {}-{}/{} lease {} is not a valid range in one subnet (lease >= 4 s)",
                line, p.name, p.start, p.end, p.prefix, p.lease_secs
            ));
        }
        if p.router.is_some_and(|r| !p.in_subnet(r)) {
            return Err(format!("line {}: router is outside the pool's subnet", line));
        }
        Ok(p)
    }

    /// Server identifier on `iface`: its configured address if that is in
    /// the pool's subnet, else a `server` line, else the pool's router.
    pub fn server_id(&self, iface: &str, iface_ip: Option<Ipv4Addr>) -> Option<Ipv4Addr> {
        let pool = self.pools.iter().find(|p| p.name == iface)?;
        iface_ip
            .filter(|a| pool.in_subnet(*a))
            .or_else(|| self.servers.get(iface).copied())
            .or(pool.router)
    }
}

/// One served interface.
#[derive(Clone, Debug)]
pub struct Iface {
    pub name: String,
    pub mac: MacAddr,
    pub server: Ipv4Addr,
}

/// The server: config, interfaces and leases.
pub struct Daemon {
    pub cfg: Config,
    pub db: LeaseDb,
    ip_id: u16,
}

fn kind_name(k: u8) -> &'static str {
    match k {
        dhcp::DISCOVER => "DHCPDISCOVER",
        dhcp::OFFER => "DHCPOFFER",
        dhcp::REQUEST => "DHCPREQUEST",
        dhcp::DECLINE => "DHCPDECLINE",
        dhcp::ACK => "DHCPACK",
        dhcp::NAK => "DHCPNAK",
        dhcp::RELEASE => "DHCPRELEASE",
        dhcp::INFORM => "DHCPINFORM",
        _ => "DHCP?",
    }
}

/// What a client message asked about (for the log).
fn describe_request(m: &Message, via: &str) -> String {
    match m.kind {
        dhcp::REQUEST => {
            let want = m.requested.unwrap_or(m.ciaddr);
            format!("DHCPREQUEST for {} from {} via {}", want, m.mac, via)
        }
        dhcp::RELEASE => format!("DHCPRELEASE of {} from {} via {}", m.ciaddr, m.mac, via),
        dhcp::DECLINE => format!(
            "DHCPDECLINE of {} from {} via {}",
            m.requested.unwrap_or(Ipv4Addr::ZERO),
            m.mac,
            via
        ),
        k => format!("{} from {} via {}", kind_name(k), m.mac, via),
    }
}

impl Daemon {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            db: LeaseDb::new(),
            ip_id: 1,
        }
    }

    /// Handle one frame received on `iface`. Returns the reply frame to
    /// send (if any) and log lines.
    pub fn on_frame(&mut self, iface: &Iface, frame: &[u8], now: i64) -> (Option<Vec<u8>>, Vec<String>) {
        let mut log = Vec::new();
        let Some(req) = parse_client_frame(frame) else {
            return (None, log);
        };
        let Some(pool) = self.cfg.pools.iter().find(|p| p.name == iface.name) else {
            return (None, log);
        };
        log.push(describe_request(&req.msg, &iface.name));
        let Some(reply) = self
            .db
            .handle(pool, &self.cfg.reservations, &req.msg, iface.server, now)
        else {
            if req.msg.kind == dhcp::DISCOVER {
                log.push(format!("no free address in pool {} for {}", iface.name, req.msg.mac));
            }
            return (None, log);
        };
        let shown = if reply.kind == dhcp::NAK {
            req.msg.requested.unwrap_or(req.msg.ciaddr)
        } else if reply.yiaddr.is_unspecified() {
            req.msg.ciaddr
        } else {
            reply.yiaddr
        };
        log.push(format!("{} on {} to {} via {}", kind_name(reply.kind), shown, req.msg.mac, iface.name));
        self.ip_id = self.ip_id.wrapping_add(1);
        (reply_frame(iface.mac, iface.server, &req, &reply, self.ip_id), log)
    }

    /// Bound leases as `iface mac ip expires_in_s` lines.
    pub fn lease_table(&self, now: i64) -> Vec<String> {
        let mut out = Vec::new();
        for p in &self.cfg.pools {
            for l in self.db.bound(&p.name) {
                out.push(format!(
                    "{} {} {} {}s",
                    l.pool,
                    l.mac,
                    l.address,
                    ((l.expires - now) / 1000).max(0)
                ));
            }
        }
        out
    }
}


// ------------------------------------------------------------ defaults

/// An interface as `dhcpd` sees it when it writes a default config.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkView {
    pub name: String,
    pub running: bool,
    pub ip: Option<Ipv4Addr>,
    pub prefix: u8,
}

/// A config `dhcpd` wrote because there was none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generated {
    /// The file's text.
    pub text: String,
    /// The interface it serves.
    pub iface: String,
    /// What to tell the player (besides the file).
    pub notes: Vec<String>,
}

/// The server address used when the chosen interface has no address yet.
pub const FALLBACK_SERVER: [u8; 4] = [192, 168, 100, 1];

/// A reasonable config for this computer when there is no `/etc/dhcpd.conf`:
///
/// * the interface: `want` if given, else the first one with an address that
///   is up, else the first one with an address, else the first one that is up
///   (never loopback);
/// * the pool: `.100`-`.199` of the /24 the interface's address is in (for a
///   prefix longer than /24, every host of the subnet); the server's own
///   address is never leased, even when it falls inside the range;
/// * router: the interface's address; DNS: the same, only when a resolver runs
///   on this computer (`resolver_here`); lease: [`DEFAULT_LEASE_SECS`];
/// * an interface without an address gets the 192.168.100.0/24 pool with
///   `server <iface> 192.168.100.1`, and a note to give it that address.
pub fn default_config(links: &[LinkView], want: Option<&str>, resolver_here: bool) -> Result<Generated, String> {
    let usable = |l: &&LinkView| l.name != "lo" && !l.ip.is_some_and(|a| a.0[0] == 127);
    let chosen = match want {
        Some(w) => links.iter().filter(usable).find(|l| l.name == w).ok_or_else(|| format!("no interface {}", w))?,
        None => links
            .iter()
            .filter(usable)
            .find(|l| l.ip.is_some() && l.running)
            .or_else(|| links.iter().filter(usable).find(|l| l.ip.is_some()))
            .or_else(|| links.iter().filter(usable).find(|l| l.running))
            .ok_or_else(|| "no network interface to serve (cable the computer or fit a modem first)".to_string())?,
    };
    let mut notes = Vec::new();
    let (server, prefix, own) = match chosen.ip.filter(|a| !a.is_unspecified()) {
        Some(a) => (a, chosen.prefix, true),
        None => {
            let s = Ipv4Addr(FALLBACK_SERVER);
            notes.push(format!(
                "{} has no address: give it {} (ifconfig {} {}/24) so clients can reach this server",
                chosen.name, s, chosen.name, s
            ));
            (s, 24, false)
        }
    };
    if prefix == 0 || prefix > 30 {
        return Err(format!("{} is {}/{}: that subnet is too small to hand out addresses", chosen.name, server, prefix));
    }
    let ip = u32::from_be_bytes(server.0);
    let (first, last) = if prefix <= 24 {
        let base = ip & 0xFFFF_FF00;
        (base + 100, base + 199)
    } else {
        let mask = u32::MAX << (32 - prefix);
        let net = ip & mask;
        (net + 1, (net | !mask) - 1)
    };
    let (first, last) = (Ipv4Addr(first.to_be_bytes()), Ipv4Addr(last.to_be_bytes()));
    let mut pool = format!("pool {} {} {} prefix {} router {}", chosen.name, first, last, prefix, server);
    if resolver_here {
        pool.push_str(&format!(" dns {}", server));
    }
    pool.push_str(&format!(" lease {}", DEFAULT_LEASE_SECS));
    let mut text = String::new();
    text.push_str("# Written by dhcpd because there was no config. Edit it and restart dhcpd to change it.\n");
    text.push_str(&format!(
        "# Serves {} ({}/{}): {}-{}, this computer as the router{}.\n",
        chosen.name,
        server,
        prefix,
        first,
        last,
        if resolver_here { " and DNS server" } else { "" }
    ));
    text.push_str(&pool);
    text.push('\n');
    if !own {
        text.push_str(&format!("server {} {}\n", chosen.name, server));
    }
    // It must parse the way an edited file would.
    Config::parse(&text).map_err(|e| e.join("; "))?;
    Ok(Generated { text, iface: chosen.name.clone(), notes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecm_net::dhcp::Client;
    use ecm_net::dhcp_server::{DECLINE_HOLD_MS, OFFER_HOLD_MS};
    use ecm_net::eth::build_frame;
    use ecm_net::ipv4::{build_packet, PROTO_UDP};
    use ecm_net::udp;

    const CONF: &str = "\
# LAN
pool eth0 192.168.50.10 192.168.50.12 router 192.168.50.1 dns 1.1.1.1 lease 60
reserve 02:00:00:00:00:42 192.168.50.5   # printer
pool eth1 10.20.0.100 10.20.0.101 netmask 255.255.0.0
server eth1 10.20.0.1
";

    fn ipa(s: &str) -> Ipv4Addr {
        Ipv4Addr::parse(s).unwrap()
    }
    fn mac(n: u8) -> MacAddr {
        MacAddr([2, 0, 0, 0, 0, n])
    }
    fn eth0() -> Iface {
        Iface {
            name: "eth0".into(),
            mac: MacAddr([2, 0xaa, 0, 0, 0, 1]),
            server: ipa("192.168.50.1"),
        }
    }
    /// A client message as the kernel stack sends it (broadcast).
    fn client_frame(m: &Message) -> Vec<u8> {
        let src = if m.ciaddr.is_unspecified() { Ipv4Addr::ZERO } else { m.ciaddr };
        let dg = udp::build(src, Ipv4Addr::BROADCAST, 68, 67, &m.encode());
        let pkt = build_packet(src, Ipv4Addr::BROADCAST, PROTO_UDP, 9, &dg).unwrap();
        build_frame(MacAddr::BROADCAST, m.mac, None, 0x0800, &pkt).unwrap()
    }
    fn reply_of(frame: &[u8]) -> Message {
        // Ethernet 14 + IPv4 20 + UDP 8.
        Message::parse(&frame[42..]).unwrap()
    }
    /// Run the kernel's client against the daemon until bound (or give up).
    fn lease(d: &mut Daemon, n: u8, now: i64) -> Option<(Ipv4Addr, Vec<String>)> {
        let mut c = Client::new(mac(n), 100 + n as u32, now);
        let mut log = Vec::new();
        for _ in 0..4 {
            let (_, m) = c.poll(now)?;
            let (reply, l) = d.on_frame(&eth0(), &client_frame(&m), now);
            log.extend(l);
            if c.receive(&reply_of(&reply?), now) {
                return c.lease.as_ref().map(|l| (l.address, log));
            }
        }
        None
    }

    #[test]
    fn config_parses_pools_reservations_and_servers() {
        let cfg = Config::parse(CONF).unwrap();
        assert_eq!(cfg.pools.len(), 2);
        let p = &cfg.pools[0];
        assert_eq!((p.name.as_str(), p.prefix, p.lease_secs), ("eth0", 24, 60));
        assert_eq!((p.router, p.dns), (Some(ipa("192.168.50.1")), Some(ipa("1.1.1.1"))));
        assert_eq!(cfg.pools[1].prefix, 16);
        assert_eq!(cfg.reservations, vec![Reservation { mac: mac(0x42), address: ipa("192.168.50.5") }]);
        // Server id: interface address, then `server`, then the router.
        assert_eq!(cfg.server_id("eth0", Some(ipa("192.168.50.2"))), Some(ipa("192.168.50.2")));
        assert_eq!(cfg.server_id("eth0", Some(ipa("10.9.9.9"))), Some(ipa("192.168.50.1")));
        assert_eq!(cfg.server_id("eth1", None), Some(ipa("10.20.0.1")));
        assert_eq!(cfg.server_id("eth2", None), None);
    }

    #[test]
    fn config_errors_name_their_lines() {
        let bad = "pool eth0 192.168.50.10\npool eth1 10.0.0.9 10.0.0.1\nreserve zz 1.2.3.4\nfoo bar\npool eth2 10.0.0.1 10.0.0.5 lease 2\npool eth3 10.0.0.1 10.0.0.5 router 10.9.0.1\npool eth4 10.0.0.1 10.0.0.5 netmask 255.0.255.0\n";
        let errs = Config::parse(bad).unwrap_err();
        for n in 1..=7 {
            assert!(errs.iter().any(|e| e.starts_with(&format!("line {}:", n))), "line {} in {:?}", n, errs);
        }
        assert_eq!(Config::parse("# empty\n").unwrap_err(), vec!["no pool configured".to_string()]);
        let orphan = Config::parse("pool eth0 192.168.50.10 192.168.50.12\nreserve 02:00:00:00:00:01 10.1.1.1\n");
        assert!(orphan.unwrap_err()[0].contains("no pool's subnet"));
    }

    #[test]
    fn dora_with_the_kernel_client_and_reservation() {
        let mut d = Daemon::new(Config::parse(CONF).unwrap());
        let (a, log) = lease(&mut d, 1, 0).unwrap();
        assert_eq!(a, ipa("192.168.50.10"));
        assert_eq!(
            log,
            vec![
                "DHCPDISCOVER from 02:00:00:00:00:01 via eth0",
                "DHCPOFFER on 192.168.50.10 to 02:00:00:00:00:01 via eth0",
                "DHCPREQUEST for 192.168.50.10 from 02:00:00:00:00:01 via eth0",
                "DHCPACK on 192.168.50.10 to 02:00:00:00:00:01 via eth0",
            ]
        );
        // The reserved MAC gets its address, outside the range.
        assert_eq!(lease(&mut d, 0x42, 0).unwrap().0, ipa("192.168.50.5"));
        assert_eq!(lease(&mut d, 2, 0).unwrap().0, ipa("192.168.50.11"));
        assert_eq!(d.lease_table(30_000).len(), 3);
        assert!(d.lease_table(30_000).contains(&"eth0 02:00:00:00:00:01 192.168.50.10 30s".to_string()));
        // A frame on an interface without a pool, or not DHCP, is ignored.
        let other = Iface { name: "eth5".into(), ..eth0() };
        let m = Message::new(dhcp::DISCOVER, 1, mac(9));
        assert!(d.on_frame(&other, &client_frame(&m), 0).0.is_none());
        assert!(d.on_frame(&eth0(), &[0u8; 60], 0).0.is_none());
    }

    #[test]
    fn replies_go_broadcast_or_unicast_per_rfc() {
        let mut d = Daemon::new(Config::parse(CONF).unwrap());
        let m = Message::new(dhcp::DISCOVER, 5, mac(1));
        let (f, _) = d.on_frame(&eth0(), &client_frame(&m), 0);
        let f = f.unwrap();
        // Broadcast flag set by the kernel client: broadcast at L2 and L3.
        assert_eq!(&f[0..6], &[0xff; 6]);
        assert_eq!(&f[30..34], &[255, 255, 255, 255]);
        assert_eq!(&f[6..12], &eth0().mac.0);
        assert_eq!(&f[26..30], &[192, 168, 50, 1]);
        // A renewing client (ciaddr set) gets a unicast reply.
        let mut r = Message::new(dhcp::REQUEST, 6, mac(1));
        r.ciaddr = ipa("192.168.50.10");
        let (f, _) = d.on_frame(&eth0(), &client_frame(&r), 0);
        let f = f.unwrap();
        assert_eq!(&f[0..6], &mac(1).0);
        assert_eq!(&f[30..34], &[192, 168, 50, 10]);
        assert_eq!(reply_of(&f).kind, dhcp::ACK);
    }

    #[test]
    fn exhaustion_expiry_nak_release_and_decline() {
        let mut d = Daemon::new(Config::parse(CONF).unwrap());
        for n in 1..=3 {
            assert!(lease(&mut d, n, 0).is_some());
        }
        // Pool full: a fourth client is not offered anything.
        let m = Message::new(dhcp::DISCOVER, 7, mac(4));
        let (reply, log) = d.on_frame(&eth0(), &client_frame(&m), 0);
        assert!(reply.is_none());
        assert!(log.last().unwrap().starts_with("no free address"));
        // After the 60 s leases expire, it is.
        assert!(lease(&mut d, 4, 61_000).is_some());
        // Asking for an address of another subnet is NAKed (broadcast).
        let mut r = Message::new(dhcp::REQUEST, 8, mac(5));
        r.requested = Some(ipa("10.0.0.5"));
        let (f, log) = d.on_frame(&eth0(), &client_frame(&r), 61_000);
        let f = f.unwrap();
        assert_eq!(reply_of(&f).kind, dhcp::NAK);
        assert_eq!(&f[0..6], &[0xff; 6]);
        assert_eq!(log[1], "DHCPNAK on 10.0.0.5 to 02:00:00:00:00:05 via eth0");
        // RELEASE frees the address for the next client.
        let mut rel = Message::new(dhcp::RELEASE, 9, mac(4));
        rel.ciaddr = ipa("192.168.50.10");
        rel.server = Some(ipa("192.168.50.1"));
        let (f, log) = d.on_frame(&eth0(), &client_frame(&rel), 61_001);
        assert!(f.is_none());
        assert_eq!(log, vec!["DHCPRELEASE of 192.168.50.10 from 02:00:00:00:00:04 via eth0"]);
        assert_eq!(lease(&mut d, 6, 61_002).unwrap().0, ipa("192.168.50.10"));
        // DECLINE parks it; the next client gets another address.
        let mut dec = Message::new(dhcp::DECLINE, 10, mac(6));
        dec.requested = Some(ipa("192.168.50.10"));
        dec.server = Some(ipa("192.168.50.1"));
        d.on_frame(&eth0(), &client_frame(&dec), 61_003);
        assert_eq!(lease(&mut d, 7, 61_004).unwrap().0, ipa("192.168.50.11"));
        assert!(DECLINE_HOLD_MS > OFFER_HOLD_MS);
    }

    #[test]
    fn leases_survive_a_restart_through_the_lease_file() {
        let mut d = Daemon::new(Config::parse(CONF).unwrap());
        lease(&mut d, 1, 0).unwrap();
        lease(&mut d, 2, 0).unwrap();
        let file = d.db.render();
        let mut again = Daemon::new(Config::parse(CONF).unwrap());
        again.db.restore(&file, 10_000);
        // Client 2 keeps its address; a newcomer gets the remaining one.
        assert_eq!(lease(&mut again, 2, 10_000).unwrap().0, ipa("192.168.50.11"));
        assert_eq!(lease(&mut again, 3, 10_000).unwrap().0, ipa("192.168.50.12"));
    }

    fn link(name: &str, running: bool, ip: Option<&str>, prefix: u8) -> LinkView {
        LinkView { name: name.into(), running, ip: ip.map(ipa), prefix }
    }

    #[test]
    fn default_config_serves_the_addressed_interface() {
        let links = [link("lo", true, Some("127.0.0.1"), 8), link("eth0", true, None, 0), link("eth1", true, Some("10.0.30.1"), 24)];
        let g = default_config(&links, None, false).unwrap();
        assert_eq!(g.iface, "eth1");
        assert!(g.notes.is_empty());
        assert!(g.text.contains("\npool eth1 10.0.30.100 10.0.30.199 prefix 24 router 10.0.30.1 lease 3600\n"), "{}", g.text);
        assert!(!g.text.contains(" dns "), "no resolver here: no DNS option");
        let cfg = Config::parse(&g.text).unwrap();
        assert_eq!(cfg.server_id("eth1", Some(ipa("10.0.30.1"))), Some(ipa("10.0.30.1")));
        // With a resolver running here, DNS is this computer too.
        let g = default_config(&links, None, true).unwrap();
        assert!(g.text.contains("router 10.0.30.1 dns 10.0.30.1 lease 3600"), "{}", g.text);
    }

    #[test]
    fn default_config_never_leases_the_servers_own_address() {
        // The server's address is inside the generated range.
        let g = default_config(&[link("eth0", true, Some("192.168.7.150"), 24)], None, false).unwrap();
        assert!(g.text.contains("pool eth0 192.168.7.100 192.168.7.199"), "{}", g.text);
        let mut d = Daemon::new(Config::parse(&g.text).unwrap());
        let iface = Iface { name: "eth0".into(), mac: MacAddr([2, 0xaa, 0, 0, 0, 1]), server: ipa("192.168.7.150") };
        let mut leased = Vec::new();
        for n in 1..=101u8 {
            let mut c = Client::new(mac(n), 100 + n as u32, 0);
            for _ in 0..4 {
                let Some((_, m)) = c.poll(0) else { break };
                let Some(reply) = d.on_frame(&iface, &client_frame(&m), 0).0 else { break };
                if c.receive(&reply_of(&reply), 0) {
                    leased.push(c.lease.as_ref().unwrap().address);
                    break;
                }
            }
        }
        assert_eq!(leased.len(), 99, "100 addresses minus the server's own");
        assert!(!leased.contains(&ipa("192.168.7.150")));
    }

    #[test]
    fn default_config_small_subnets_unaddressed_and_none() {
        // A /28: every host of the subnet.
        let g = default_config(&[link("eth0", true, Some("10.1.1.17"), 28)], None, false).unwrap();
        assert!(g.text.contains("pool eth0 10.1.1.17 10.1.1.30 prefix 28"), "{}", g.text);
        // Wider than /24: the /24 the address is in.
        let g = default_config(&[link("eth0", true, Some("10.2.3.4"), 16)], None, false).unwrap();
        assert!(g.text.contains("pool eth0 10.2.3.100 10.2.3.199 prefix 16 router 10.2.3.4"), "{}", g.text);
        // No address anywhere: the first interface that is up, a fallback subnet and a note.
        let links = [link("lo", true, Some("127.0.0.1"), 8), link("eth0", false, None, 0), link("wwan0", true, None, 0)];
        let g = default_config(&links, None, false).unwrap();
        assert_eq!(g.iface, "wwan0");
        assert!(g.text.contains("pool wwan0 192.168.100.100 192.168.100.199 prefix 24 router 192.168.100.1"), "{}", g.text);
        assert!(g.text.contains("\nserver wwan0 192.168.100.1\n"));
        assert!(g.notes[0].contains("ifconfig wwan0 192.168.100.1/24"), "{:?}", g.notes);
        // A named interface wins; loopback alone is an error; /31 is too small.
        let links = [link("eth0", true, Some("10.0.0.1"), 24), link("eth1", true, Some("10.9.0.1"), 24)];
        assert_eq!(default_config(&links, Some("eth1"), false).unwrap().iface, "eth1");
        assert!(default_config(&links, Some("eth7"), false).is_err());
        assert!(default_config(&[link("lo", true, Some("127.0.0.1"), 8)], None, false).is_err());
        assert!(default_config(&[link("eth0", true, Some("10.0.0.1"), 31)], None, false).is_err());
    }
}
