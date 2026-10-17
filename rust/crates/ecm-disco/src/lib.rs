//! `ecm_disco`: finding services on the network (docs/DISCOVERY.md).
//!
//! A computer that offers services runs `disco serve` (the responder, like avahi-daemon),
//! which joins the link-local multicast group [`GROUP`] on every interface. A program
//! looking for a service sends one UDP query to that group on every interface, to its own
//! computer (127.0.0.1) and to its default gateway, and collects the answers for a moment.
//! Link-local multicast never crosses a router, so a router that runs `disco serve` with `relay gateway` passes the query on to
//! its own gateway and returns what comes back: a house PC asks its home router, which
//! asks the village ISP, which knows the ring's services from its `/etc/disco.conf`.
//!
//! Wire format, one datagram, text lines (`\n`):
//!
//! ```text
//! ECMDISCO/1 QUERY <id> <type|*> <hops>
//! ECMDISCO/1 ANSWER <id>
//! <type> <host|-> <port> <name...>        one line per service; '-' is the answer's sender
//! ```
//!
//! Everything here is plain logic; [`sys`] does the I/O on a computer.

use std::collections::HashMap;

#[cfg(target_arch = "wasm32")]
pub mod sys;

/// The UDP port responders listen on.
pub const PORT: u16 = 5354;
/// The link-local multicast group responders join (224.0.0.254, RFC 4727's experimental
/// link-local address; mDNS's 224.0.0.251 belongs to a different protocol).
pub const GROUP: Ip = [224, 0, 0, 254];
pub const MAGIC: &str = "ECMDISCO/1";
/// A query relayed this many times is not relayed again (stops loops between relays).
pub const MAX_HOPS: u8 = 3;
/// How long a client listens for answers.
pub const CLIENT_WAIT_MS: i64 = 1500;
/// How long a relay keeps passing answers back for a forwarded query (less than the client waits).
pub const RELAY_WAIT_MS: i64 = 1000;
/// The responder's configuration.
pub const CONF: &str = "/etc/disco.conf";
/// Where servers announce themselves (`<type>-<port>`, one `service` line each).
pub const RUN_DIR: &str = "/run/disco";
pub const MAX_NAME: usize = 48;
/// Answers per datagram (keeps it well under one Ethernet frame).
pub const MAX_ANSWERS: usize = 16;

/// The service types the mod's programs use.
pub mod types {
    /// Something the `chat` command talks to (a Discord backend, `discordd`).
    pub const CHAT: &str = "chat";
    /// A Discord backend (`discordd`): the Discord app connects to every one it finds.
    pub const DISCORD: &str = "discord";
    /// A package repository (`pkg serve`): `http://<host>:<port>/`.
    pub const PKG: &str = "pkg";
    /// An Auth server (`authd`). Only an address: trust comes from the pinned realm key.
    pub const AUTH: &str = "auth";
    /// A music backend (`musicd`).
    pub const MUSIC: &str = "music";
}

pub type Ip = [u8; 4];

pub fn ip_str(ip: Ip) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

pub fn parse_ip(s: &str) -> Option<Ip> {
    let parts: Vec<&str> = s.trim().split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut ip = [0u8; 4];
    for (o, p) in ip.iter_mut().zip(parts) {
        if p.is_empty() || p.len() > 3 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *o = p.parse().ok()?;
    }
    Some(ip)
}

/// A service type: 1-16 of `a-z`, `0-9`, `-` (`*` in a query means every type).
pub fn valid_type(t: &str) -> bool {
    !t.is_empty() && t.len() <= 16 && t.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A display name: printable, one line, trimmed, at most [`MAX_NAME`] characters.
pub fn clean_name(n: &str) -> String {
    let s: String = n.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_NAME).collect()
}

/// One service in an answer or a configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    pub kind: String,
    /// `None`: on the computer that sent the answer (or, in a config, this computer).
    pub host: Option<Ip>,
    pub port: u16,
    pub name: String,
}

impl Service {
    pub fn new(kind: &str, host: Option<Ip>, port: u16, name: &str) -> Service {
        Service { kind: kind.into(), host, port, name: clean_name(name) }
    }

    fn line(&self) -> String {
        let host = self.host.map(ip_str).unwrap_or_else(|| "-".into());
        let mut l = format!("{} {} {}", self.kind, host, self.port);
        if !self.name.is_empty() {
            l.push(' ');
            l.push_str(&self.name);
        }
        l
    }

    fn parse_line(l: &str) -> Option<Service> {
        let mut p = l.splitn(4, ' ');
        let kind = p.next()?;
        let host = p.next()?;
        let port: u16 = p.next()?.parse().ok()?;
        let name = p.next().unwrap_or("");
        if !valid_type(kind) || port == 0 {
            return None;
        }
        let host = if host == "-" { None } else { Some(parse_ip(host)?) };
        Some(Service::new(kind, host, port, name))
    }

    pub fn matches(&self, kind: &str) -> bool {
        kind == "*" || self.kind == kind
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    Query { id: u32, kind: String, hops: u8 },
    Answer { id: u32, services: Vec<Service> },
}

impl Msg {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Msg::Query { id, kind, hops } => format!("{MAGIC} QUERY {id} {kind} {hops}\n").into_bytes(),
            Msg::Answer { id, services } => {
                let mut s = format!("{MAGIC} ANSWER {id}\n");
                for sv in services.iter().take(MAX_ANSWERS) {
                    s.push_str(&sv.line());
                    s.push('\n');
                }
                s.into_bytes()
            }
        }
    }

    /// `None` for anything that is not a well-formed ECMDISCO/1 message.
    pub fn parse(data: &[u8]) -> Option<Msg> {
        let text = std::str::from_utf8(data).ok()?;
        let mut lines = text.lines();
        let head: Vec<&str> = lines.next()?.split(' ').collect();
        if head.first() != Some(&MAGIC) {
            return None;
        }
        match head.get(1..)? {
            ["QUERY", id, kind, hops] => {
                let kind = kind.to_string();
                if kind != "*" && !valid_type(&kind) {
                    return None;
                }
                Some(Msg::Query { id: id.parse().ok()?, kind, hops: hops.parse().ok()? })
            }
            ["ANSWER", id] => Some(Msg::Answer {
                id: id.parse().ok()?,
                services: lines.filter_map(Service::parse_line).take(MAX_ANSWERS).collect(),
            }),
            _ => None,
        }
    }
}

/// Where a responder passes queries on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Relay {
    #[default]
    None,
    /// The default gateway at the time of the query.
    Gateway,
    To(Ip),
}

/// `/etc/disco.conf`:
///
/// ```text
/// service <type> <port> [name]           a service on this computer
/// record <type> <host> <port> [name]     a service elsewhere this responder answers for
/// relay gateway | relay <address>        pass queries on (routers)
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub services: Vec<Service>,
    pub relay: Relay,
}

impl Config {
    /// The configuration and a message for every line it ignored.
    pub fn parse(text: &str) -> (Config, Vec<String>) {
        let mut c = Config::default();
        let mut bad = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let l = raw.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let words: Vec<&str> = l.split_whitespace().collect();
            let ok = match words.as_slice() {
                ["service", kind, port, name @ ..] => match port.parse::<u16>() {
                    Ok(p) if valid_type(kind) && p > 0 => {
                        c.services.push(Service::new(kind, None, p, &name.join(" ")));
                        true
                    }
                    _ => false,
                },
                ["record", kind, host, port, name @ ..] => match (parse_ip(host), port.parse::<u16>()) {
                    (Some(h), Ok(p)) if valid_type(kind) && p > 0 => {
                        c.services.push(Service::new(kind, Some(h), p, &name.join(" ")));
                        true
                    }
                    _ => false,
                },
                ["relay", "gateway"] => {
                    c.relay = Relay::Gateway;
                    true
                }
                ["relay", "none"] => {
                    c.relay = Relay::None;
                    true
                }
                ["relay", ip] => match parse_ip(ip) {
                    Some(ip) => {
                        c.relay = Relay::To(ip);
                        true
                    }
                    None => false,
                },
                _ => false,
            };
            if !ok {
                bad.push(format!("line {}: {}", n + 1, l));
            }
        }
        (c, bad)
    }

    /// Add the services announced in [`RUN_DIR`] (each file holds `service` lines).
    pub fn add_announced(&mut self, text: &str) {
        let (c, _) = Config::parse(text);
        for s in c.services {
            if !self.services.contains(&s) {
                self.services.push(s);
            }
        }
    }
}

/// The announcement a server writes for itself: its file name in [`RUN_DIR`] and its line.
pub fn announcement(kind: &str, port: u16, name: &str) -> (String, String) {
    let name = clean_name(name);
    let line = if name.is_empty() { format!("service {kind} {port}\n") } else { format!("service {kind} {port} {name}\n") };
    (format!("{kind}-{port}"), line)
}

/// A datagram to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Out {
    pub to: (Ip, u16),
    pub msg: Msg,
}

#[derive(Clone, Debug)]
struct Pending {
    upstream_id: u32,
    upstream: Ip,
    peer: (Ip, u16),
    id: u32,
    deadline: i64,
}

/// `disco serve`'s logic: answer queries from the configuration and relay them upstream.
#[derive(Debug, Default)]
pub struct Responder {
    pending: Vec<Pending>,
    next_id: u32,
}

impl Responder {
    pub fn new(seed: u32) -> Responder {
        Responder { pending: Vec::new(), next_id: seed | 1 }
    }

    /// A datagram arrived from `from`. `upstream` is where to relay to (the configuration's
    /// relay resolved: the current default gateway for [`Relay::Gateway`]).
    pub fn on_datagram(&mut self, cfg: &Config, upstream: Option<Ip>, from: (Ip, u16), data: &[u8], now: i64) -> Vec<Out> {
        self.expire(now);
        match Msg::parse(data) {
            Some(Msg::Query { id, kind, hops }) => self.on_query(cfg, upstream, from, id, &kind, hops, now),
            Some(Msg::Answer { id, services }) => self.on_answer(from, id, services),
            None => Vec::new(),
        }
    }

    fn on_query(&mut self, cfg: &Config, upstream: Option<Ip>, from: (Ip, u16), id: u32, kind: &str, hops: u8, now: i64) -> Vec<Out> {
        // The same query twice (to the group and to us as the gateway): once is enough.
        if self.pending.iter().any(|p| p.peer == from && p.id == id) {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mine: Vec<Service> = cfg.services.iter().filter(|s| s.matches(kind)).cloned().collect();
        if !mine.is_empty() {
            out.push(Out { to: from, msg: Msg::Answer { id, services: mine } });
        }
        if let Some(up) = upstream {
            // Never back to where the query came from, never past MAX_HOPS.
            if up != from.0 && hops < MAX_HOPS && up != [0, 0, 0, 0] {
                self.next_id = self.next_id.wrapping_add(2);
                let upstream_id = self.next_id;
                self.pending.push(Pending { upstream_id, upstream: up, peer: from, id, deadline: now + RELAY_WAIT_MS });
                out.push(Out { to: (up, PORT), msg: Msg::Query { id: upstream_id, kind: kind.into(), hops: hops + 1 } });
            }
        }
        out
    }

    fn on_answer(&mut self, from: (Ip, u16), id: u32, services: Vec<Service>) -> Vec<Out> {
        let Some(p) = self.pending.iter().find(|p| p.upstream_id == id && p.upstream == from.0) else {
            return Vec::new();
        };
        // '-' was the upstream responder itself: name it, the asker can't see it.
        let services: Vec<Service> = services
            .into_iter()
            .map(|mut s| {
                s.host = Some(s.host.unwrap_or(from.0));
                s
            })
            .collect();
        if services.is_empty() {
            return Vec::new();
        }
        vec![Out { to: p.peer, msg: Msg::Answer { id: p.id, services } }]
    }

    fn expire(&mut self, now: i64) {
        self.pending.retain(|p| p.deadline > now);
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}

/// A service a client found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub kind: String,
    pub host: Ip,
    pub port: u16,
    pub name: String,
}

impl Found {
    pub fn address(&self) -> String {
        ip_str(self.host)
    }

    /// `host:port`.
    pub fn endpoint(&self) -> String {
        format!("{}:{}", ip_str(self.host), self.port)
    }

    /// The name, or `kind on host` when the service has none.
    pub fn label(&self) -> String {
        if self.name.is_empty() {
            format!("{} on {}", self.kind, ip_str(self.host))
        } else {
            self.name.clone()
        }
    }
}

/// The client's side of one search: the query and what came back.
#[derive(Debug)]
pub struct Search {
    pub id: u32,
    pub kind: String,
    found: Vec<Found>,
}

impl Search {
    pub fn new(id: u32, kind: &str) -> Search {
        Search { id, kind: kind.into(), found: Vec::new() }
    }

    pub fn query(&self) -> Vec<u8> {
        Msg::Query { id: self.id, kind: self.kind.clone(), hops: 0 }.encode()
    }

    /// A datagram from `from`: keep the answers to this search (one per type, host and
    /// port; a named answer replaces an unnamed one).
    pub fn on_datagram(&mut self, from: Ip, data: &[u8]) {
        let Some(Msg::Answer { id, services }) = Msg::parse(data) else { return };
        if id != self.id {
            return;
        }
        for s in services {
            if !s.matches(&self.kind) {
                continue;
            }
            let f = Found { kind: s.kind, host: s.host.unwrap_or(from), port: s.port, name: s.name };
            match self.found.iter_mut().find(|x| x.kind == f.kind && x.host == f.host && x.port == f.port) {
                Some(x) => {
                    if x.name.is_empty() {
                        x.name = f.name;
                    }
                }
                None => self.found.push(f),
            }
        }
    }

    /// What was found, loopback last (the same service is usually also found by its address), by name.
    pub fn results(&self) -> Vec<Found> {
        let mut v = self.found.clone();
        // This computer's own service, also found by one of its addresses: keep that one.
        let lan: Vec<(String, u16, String)> =
            v.iter().filter(|f| f.host[0] != 127).map(|f| (f.kind.clone(), f.port, f.name.clone())).collect();
        v.retain(|f| f.host[0] != 127 || !lan.contains(&(f.kind.clone(), f.port, f.name.clone())));
        v.sort_by(|a, b| (a.host[0] == 127, a.label().to_lowercase(), a.host).cmp(&(b.host[0] == 127, b.label().to_lowercase(), b.host)));
        v
    }
}

/// Where a client sends its query by unicast: this computer and the default gateway (the
/// group goes out of every interface in [`multicast_interfaces`]).
pub fn targets(gateway: Option<Ip>) -> Vec<Ip> {
    let mut t = vec![[127, 0, 0, 1]];
    if let Some(g) = gateway.filter(|g| *g != [0, 0, 0, 0] && g[0] != 127) {
        t.push(g);
    }
    t
}

/// The interfaces (by address) the group is sent out of and joined on: every configured, non-loopback one.
pub fn multicast_interfaces(ifaces: &[(Ip, u8)]) -> Vec<Ip> {
    let mut v: Vec<Ip> = Vec::new();
    for &(ip, _) in ifaces {
        if ip != [0, 0, 0, 0] && ip[0] != 127 && !v.contains(&ip) {
            v.push(ip);
        }
    }
    v
}

/// A flat JSON object's string or number field (what the network config API returns).
pub fn json_field<'a>(obj: &'a str, key: &str) -> Option<&'a str> {
    let at = obj.find(&format!("\"{}\":", key))? + key.len() + 3;
    let rest = obj[at..].trim_start();
    if let Some(s) = rest.strip_prefix('"') {
        Some(&s[..s.find('"')?])
    } else {
        let end = rest.find(|c: char| c == ',' || c == '}').unwrap_or(rest.len());
        Some(rest[..end].trim())
    }
}

/// The default route's gateway from the route list JSON (`[{"dest":..,"prefix_len":..,"gateway":..},...]`).
pub fn default_gateway(routes_json: &str) -> Option<Ip> {
    routes_json.split('{').skip(1).find_map(|o| {
        let dest = parse_ip(json_field(o, "dest")?)?;
        let len: u8 = json_field(o, "prefix_len")?.parse().ok()?;
        let gw = parse_ip(json_field(o, "gateway")?)?;
        (dest == [0, 0, 0, 0] && len == 0 && gw != [0, 0, 0, 0]).then_some(gw)
    })
}

/// Results grouped for printing: `disco`'s output.
pub fn format_results(found: &[Found]) -> String {
    if found.is_empty() {
        return String::new();
    }
    let mut by: HashMap<&str, Vec<&Found>> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    for f in found {
        if !by.contains_key(f.kind.as_str()) {
            order.push(&f.kind);
        }
        by.entry(&f.kind).or_default().push(f);
    }
    order.sort();
    let mut s = String::new();
    for k in order {
        for f in &by[k] {
            s.push_str(&format!("{:<5} {:<21} {}\n", f.kind, f.endpoint(), f.label()));
        }
    }
    s
}

#[cfg(test)]
mod tests;
