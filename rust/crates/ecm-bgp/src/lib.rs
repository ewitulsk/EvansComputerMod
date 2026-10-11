//! BGP engine: virtual time, explicit transport events, bounded RIBs.
pub mod show;
pub mod wire;
use ecm_net::Ipv4Addr;
use std::collections::{BTreeMap, VecDeque};
use wire::Message;
pub const MAX_PREFIXES: usize = 512;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Prefix {
    pub address: Ipv4Addr,
    pub len: u8,
}
impl Prefix {
    pub fn new(address: Ipv4Addr, len: u8) -> Self {
        Self {
            address: address.network_addr(len.min(32)),
            len: len.min(32),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attributes {
    pub path: Vec<u32>,
    /// (offset, length) of AS_SET segments in the flattened membership list.
    pub path_sets: Vec<(usize, usize)>,
    pub origin: u8,
    pub med: u32,
    pub local_pref: u32,
    pub next_hop: Ipv4Addr,
    pub communities: Vec<u32>,
    pub unknown_transitive: Vec<(u8, Vec<u8>)>,
}
impl Default for Attributes {
    fn default() -> Self {
        Self {
            path: Vec::new(),
            path_sets: Vec::new(),
            origin: 0,
            med: 0,
            local_pref: 100,
            next_hop: Ipv4Addr::ZERO,
            communities: Vec::new(),
            unknown_transitive: Vec::new(),
        }
    }
}
impl Attributes {
    pub fn path_length(&self) -> usize {
        self.path.len() - self.path_sets.iter().map(|(_, n)| n - 1).sum::<usize>()
    }
    fn prepend(&mut self, asn: u32) {
        self.path.insert(0, asn);
        for (start, _) in &mut self.path_sets {
            *start += 1;
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub prefix: Prefix,
    pub attributes: Attributes,
    pub peer: Option<Ipv4Addr>,
    pub external: bool,
    pub router_id: Ipv4Addr,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Connect,
    Active,
    OpenSent,
    OpenConfirm,
    Established,
}
/// How the remote AS of a neighbor or peer group is specified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteAs {
    /// `remote-as ASN`: the OPEN must carry exactly this AS.
    Asn(u32),
    /// `remote-as external` (FRR): any AS other than the local one (eBGP).
    External,
    /// `remote-as internal` (FRR): the local AS (iBGP).
    Internal,
}
impl RemoteAs {
    pub fn accepts(self, asn: u32, local: u32) -> bool {
        match self {
            RemoteAs::Asn(n) => asn == n,
            RemoteAs::External => asn != local,
            RemoteAs::Internal => asn == local,
        }
    }
    pub fn render(self) -> String {
        match self {
            RemoteAs::Asn(n) => n.to_string(),
            RemoteAs::External => "external".into(),
            RemoteAs::Internal => "internal".into(),
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "external" => Some(RemoteAs::External),
            "internal" => Some(RemoteAs::Internal),
            _ => s.parse::<u32>().ok().filter(|n| *n > 0).map(RemoteAs::Asn),
        }
    }
}
/// `neighbor X maximum-prefix MAX [threshold PCT] [restart SECS] [warning-only]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaxPrefix {
    pub max: u32,
    pub threshold: u8,
    pub restart: Option<u16>,
    pub warning_only: bool,
}
impl MaxPrefix {
    pub fn new(max: u32) -> Self {
        Self {
            max,
            threshold: 75,
            restart: None,
            warning_only: false,
        }
    }
    pub fn render(&self) -> String {
        let mut s = format!("maximum-prefix {}", self.max);
        if self.threshold != 75 {
            s.push_str(&format!(" threshold {}", self.threshold));
        }
        if let Some(r) = self.restart {
            s.push_str(&format!(" restart {}", r));
        }
        if self.warning_only {
            s.push_str(" warning-only");
        }
        s
    }
}
/// A neighbor, or a peer-group template (same fields; a group has no `peer_group`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Neighbor {
    /// None: inherited from the peer group (a neighbor without either is not started).
    pub remote_as: Option<RemoteAs>,
    pub update_source: Option<String>,
    pub active: bool,
    pub default_originate: bool,
    pub inbound: Option<String>,
    pub outbound: Option<String>,
    pub maximum_prefix: Option<MaxPrefix>,
    pub peer_group: Option<String>,
}
impl Neighbor {
    pub fn new(asn: u32) -> Self {
        Self {
            remote_as: Some(RemoteAs::Asn(asn)),
            ..Self::default()
        }
    }
    /// The neighbor's own settings over its group's (`activate` from either).
    pub fn merged(&self, group: Option<&Neighbor>) -> Neighbor {
        let Some(g) = group else {
            return self.clone();
        };
        Neighbor {
            remote_as: self.remote_as.or(g.remote_as),
            update_source: self.update_source.clone().or_else(|| g.update_source.clone()),
            active: self.active || g.active,
            default_originate: self.default_originate || g.default_originate,
            inbound: self.inbound.clone().or_else(|| g.inbound.clone()),
            outbound: self.outbound.clone().or_else(|| g.outbound.clone()),
            maximum_prefix: self.maximum_prefix.or(g.maximum_prefix),
            peer_group: self.peer_group.clone(),
        }
    }
}
/// `neighbor GROUP listen ip-range PREFIX [as-range RANGE] [limit N]` (AOS-CX dynamic peers).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListenRange {
    pub prefix: Prefix,
    pub group: String,
    /// Inclusive AS intervals the OPEN's AS must fall in (None: any).
    pub as_range: Option<Vec<(u32, u32)>>,
    /// Most dynamic peers this range may hold.
    pub limit: Option<u32>,
}
impl ListenRange {
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        ip.same_subnet_prefix(&self.prefix.address, self.prefix.len)
    }
    pub fn as_allowed(&self, asn: u32) -> bool {
        self.as_range
            .as_ref()
            .is_none_or(|r| r.iter().any(|(a, b)| (*a..=*b).contains(&asn)))
    }
}
/// Parses an AOS-CX AS range: `65001-65010`, `65001,65005,65100-65200`.
pub fn parse_as_range(s: &str) -> Option<Vec<(u32, u32)>> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.parse::<u32>().ok()?, b.parse::<u32>().ok()?),
            None => {
                let n = part.parse::<u32>().ok()?;
                (n, n)
            }
        };
        if a == 0 || a > b {
            return None;
        }
        out.push((a, b));
    }
    (!out.is_empty()).then_some(out)
}
pub fn render_as_range(r: &[(u32, u32)]) -> String {
    r.iter()
        .map(|(a, b)| if a == b { a.to_string() } else { format!("{}-{}", a, b) })
        .collect::<Vec<_>>()
        .join(",")
}
/// FRR/Cisco default for `bgp listen limit`.
pub const DEFAULT_LISTEN_LIMIT: u32 = 100;
#[derive(Clone, Debug)]
pub struct PrefixRule {
    pub seq: u32,
    pub permit: bool,
    pub prefix: Prefix,
    pub min: u8,
    pub max: u8,
}
#[derive(Clone, Debug)]
pub struct MapRule {
    pub seq: u32,
    pub permit: bool,
    pub prefix_list: Option<String>,
    pub local_pref: Option<u32>,
    pub med: Option<u32>,
    pub community: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub asn: u32,
    pub router_id: Ipv4Addr,
    pub keepalive: u16,
    pub hold: u16,
    pub neighbors: BTreeMap<Ipv4Addr, Neighbor>,
    /// Peer groups by name (`neighbor NAME peer-group`).
    pub groups: BTreeMap<String, Neighbor>,
    /// Dynamic-neighbor listen ranges.
    pub listen: Vec<ListenRange>,
    /// `bgp listen limit N`: most dynamic peers in total (None: the default, 100).
    pub listen_limit: Option<u32>,
    pub networks: Vec<Prefix>,
    pub connected: bool,
    pub static_routes: bool,
}
impl Config {
    /// A configured neighbor with its peer group's settings applied.
    pub fn effective(&self, n: &Neighbor) -> Neighbor {
        n.merged(n.peer_group.as_ref().and_then(|g| self.groups.get(g)))
    }
    /// The listen range with the longest prefix that contains `ip`.
    pub fn listen_range(&self, ip: Ipv4Addr) -> Option<&ListenRange> {
        self.listen
            .iter()
            .filter(|r| r.contains(ip) && self.groups.contains_key(&r.group))
            .max_by_key(|r| r.prefix.len)
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            asn: 0,
            router_id: Ipv4Addr::ZERO,
            keepalive: 60,
            hold: 180,
            neighbors: BTreeMap::new(),
            groups: BTreeMap::new(),
            listen: Vec::new(),
            listen_limit: None,
            networks: Vec::new(),
            connected: false,
            static_routes: false,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Policy {
    pub lists: BTreeMap<String, Vec<PrefixRule>>,
    pub maps: BTreeMap<String, Vec<MapRule>>,
}
impl Policy {
    pub fn apply(&self, name: Option<&str>, prefix: Prefix, a: &mut Attributes) -> bool {
        let Some(name) = name else {
            return true;
        };
        let Some(rules) = self.maps.get(name) else {
            return false;
        };
        let mut rules: Vec<_> = rules.iter().collect();
        rules.sort_by_key(|r| r.seq);
        for r in rules {
            let matched = r.prefix_list.as_ref().map_or(true, |name| {
                let Some(list) = self.lists.get(name) else {
                    return false;
                };
                let mut list: Vec<_> = list.iter().collect();
                list.sort_by_key(|r| r.seq);
                list.iter()
                    .find(|r| {
                        prefix
                            .address
                            .same_subnet_prefix(&r.prefix.address, r.prefix.len)
                            && r.min <= prefix.len
                            && prefix.len <= r.max
                    })
                    .is_some_and(|r| r.permit)
            });
            if matched {
                if !r.permit {
                    return false;
                }
                if let Some(n) = r.local_pref {
                    a.local_pref = n;
                }
                if let Some(n) = r.med {
                    a.med = n;
                }
                if let Some(n) = r.community {
                    a.communities.push(n);
                }
                return true;
            }
        }
        false
    }
}
/// Last NOTIFICATION exchanged with a peer, for `show bgp ... neighbors`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LastError {
    pub sent: bool,
    pub code: u8,
    pub subcode: u8,
    pub at: i64,
}
impl LastError {
    pub fn describe(&self) -> String {
        format!(
            "{} {}",
            if self.sent { "sent" } else { "received" },
            notification_text(self.code, self.subcode)
        )
    }
}
/// RFC 4271 / RFC 4486 names for NOTIFICATION codes.
pub fn notification_text(code: u8, subcode: u8) -> String {
    let sub = match (code, subcode) {
        (2, 2) => "Bad Peer AS",
        (2, 3) => "Bad BGP Identifier",
        (2, 6) => "Unacceptable Hold Time",
        (6, 1) => "Maximum Number of Prefixes Reached",
        (6, 2) => "Administrative Shutdown",
        (6, 3) => "Peer De-configured",
        (6, 4) => "Administrative Reset",
        (6, 5) => "Connection Rejected",
        (6, 7) => "Connection Collision Resolution",
        _ => "",
    };
    let main = match code {
        1 => "Message Header Error",
        2 => "OPEN Message Error",
        3 => "UPDATE Message Error",
        4 => "Hold Timer Expired",
        5 => "Finite State Machine Error",
        6 => "Cease",
        _ => "Unknown",
    };
    if sub.is_empty() {
        format!("{} ({}/{})", main, code, subcode)
    } else {
        format!("{}/{} ({}/{})", main, sub, code, subcode)
    }
}
pub struct Peer {
    /// Effective settings (neighbor over its peer group).
    pub config: Neighbor,
    /// The peer's actual AS: configured, or learned from its OPEN (0 until then).
    pub remote_as: u32,
    /// Peer group of a dynamic (listen-range) neighbor; None for configured ones.
    pub dynamic: Option<String>,
    pub state: State,
    pub router_id: Ipv4Addr,
    pub as4: bool,
    pub refresh: bool,
    pub adj_in: BTreeMap<Prefix, Attributes>,
    pub adj_out: BTreeMap<Prefix, Attributes>,
    pub retry: i64,
    pub hold_deadline: i64,
    pub keepalive_deadline: i64,
    pub negotiated_hold: u16,
    pub msgs_in: u64,
    pub msgs_out: u64,
    /// Time of the last change into or out of Established.
    pub since: i64,
    pub last_error: Option<LastError>,
    /// Set after `maximum-prefix` tore the session down: no session before
    /// this time (i64::MAX: until `clear bgp`).
    pub prefix_block: Option<i64>,
    input: Vec<u8>,
    local: Ipv4Addr,
}
impl Peer {
    fn new(config: Neighbor, remote_as: u32, dynamic: Option<String>, hold: u16, now: i64) -> Self {
        Peer {
            config,
            remote_as,
            dynamic,
            state: State::Idle,
            router_id: Ipv4Addr::ZERO,
            as4: false,
            refresh: false,
            adj_in: BTreeMap::new(),
            adj_out: BTreeMap::new(),
            retry: now,
            hold_deadline: i64::MAX,
            keepalive_deadline: i64::MAX,
            negotiated_hold: hold,
            msgs_in: 0,
            msgs_out: 0,
            since: now,
            last_error: None,
            prefix_block: None,
            input: Vec::new(),
            local: Ipv4Addr::ZERO,
        }
    }
    pub fn is_dynamic(&self) -> bool {
        self.dynamic.is_some()
    }
    /// Passive peers wait for the remote side to connect (all dynamic peers).
    pub fn passive(&self) -> bool {
        self.dynamic.is_some()
    }
    /// The local address of the session (the next hop we advertise).
    pub fn local_addr(&self) -> Ipv4Addr {
        self.local
    }
}
#[derive(Clone, Debug)]
pub enum Action {
    Connect(Ipv4Addr),
    Send(Ipv4Addr, Vec<u8>),
    Close(Ipv4Addr),
    RoutesChanged,
}
/// Why `Engine::accept` refused an incoming connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reject {
    /// Not a configured neighbor and in no listen range.
    Unknown,
    /// The neighbor or its peer group is not activated in the address family.
    Inactive,
    /// The range's `limit` or `bgp listen limit` is reached.
    Limit,
    /// `maximum-prefix` shut this address out (until its restart time or `clear bgp`).
    PrefixLimit,
}
pub struct Engine {
    pub config: Config,
    pub policy: Policy,
    pub peers: BTreeMap<Ipv4Addr, Peer>,
    pub rib: BTreeMap<Prefix, Route>,
    /// Dynamic addresses shut out by maximum-prefix, with the time they may return.
    pub blocked: BTreeMap<Ipv4Addr, i64>,
    origins: BTreeMap<Prefix, Attributes>,
    actions: VecDeque<Action>,
}
impl Engine {
    pub fn new(config: Config, policy: Policy, now: i64) -> Self {
        let mut peers = BTreeMap::new();
        for (ip, n) in &config.neighbors {
            let c = config.effective(n);
            let Some(rule) = c.remote_as else {
                continue;
            };
            let asn = match rule {
                RemoteAs::Asn(a) => a,
                RemoteAs::Internal => config.asn,
                RemoteAs::External => 0,
            };
            peers.insert(*ip, Peer::new(c, asn, None, config.hold, now));
        }
        Self {
            config,
            policy,
            peers,
            rib: BTreeMap::new(),
            blocked: BTreeMap::new(),
            origins: BTreeMap::new(),
            actions: VecDeque::new(),
        }
    }
    pub fn pop_action(&mut self) -> Option<Action> {
        self.actions.pop_front()
    }
    fn send(&mut self, ip: Ipv4Addr, m: Message) {
        let local_as = self.config.asn;
        let Some(p) = self.peers.get_mut(&ip) else {
            return;
        };
        if let Message::Notification { code, subcode } = m {
            p.last_error = Some(LastError {
                sent: true,
                code,
                subcode,
                at: p.since,
            });
        }
        let b = m.encode_for_peer(p.as4, p.remote_as == local_as);
        if b.len() <= wire::MAX_MESSAGE {
            p.msgs_out += 1;
            self.actions.push_back(Action::Send(ip, b));
        }
    }
    /// Number of dynamic peers, in total or inside `range`.
    pub fn dynamic_count(&self, range: Option<&ListenRange>) -> usize {
        self.peers
            .iter()
            .filter(|(ip, p)| p.is_dynamic() && range.is_none_or(|r| r.contains(**ip)))
            .count()
    }
    /// An incoming TCP connection from `ip`: Ok if a session may run on it.
    /// Creates a dynamic peer when `ip` is not a configured neighbor but falls in a
    /// listen range (configured neighbors take precedence over ranges).
    pub fn accept(&mut self, ip: Ipv4Addr, now: i64) -> Result<(), Reject> {
        if let Some(p) = self.peers.get(&ip) {
            if p.prefix_block.is_some_and(|t| now < t) {
                return Err(Reject::PrefixLimit);
            }
            return if p.config.active {
                Ok(())
            } else {
                Err(Reject::Inactive)
            };
        }
        if self.blocked.get(&ip).is_some_and(|t| now < *t) {
            return Err(Reject::PrefixLimit);
        }
        self.blocked.remove(&ip);
        let Some(range) = self.config.listen_range(ip).cloned() else {
            return Err(Reject::Unknown);
        };
        let group = self.config.groups[&range.group].clone();
        if !group.active {
            return Err(Reject::Inactive);
        }
        if range
            .limit
            .is_some_and(|l| self.dynamic_count(Some(&range)) >= l as usize)
            || self.dynamic_count(None)
                >= self.config.listen_limit.unwrap_or(DEFAULT_LISTEN_LIMIT) as usize
        {
            return Err(Reject::Limit);
        }
        let asn = match group.remote_as {
            Some(RemoteAs::Asn(a)) => a,
            Some(RemoteAs::Internal) => self.config.asn,
            _ => 0,
        };
        let mut c = group;
        c.peer_group = Some(range.group.clone());
        self.peers.insert(
            ip,
            Peer::new(c, asn, Some(range.group), self.config.hold, now),
        );
        Ok(())
    }
    /// Is `asn` (from the peer's OPEN) acceptable for this peer?
    fn as_acceptable(&self, ip: Ipv4Addr, asn: u32) -> bool {
        let p = &self.peers[&ip];
        let rule_ok = match p.config.remote_as {
            Some(rule) => rule.accepts(asn, self.config.asn),
            // A listen range with an as-range but no remote-as: the OPEN decides.
            None => p.is_dynamic(),
        };
        let range_ok = !p.is_dynamic()
            || self
                .config
                .listen_range(ip)
                .is_some_and(|r| r.as_allowed(asn));
        rule_ok && range_ok
    }
    pub fn connected(&mut self, ip: Ipv4Addr, local: Ipv4Addr, now: i64) {
        let Some(p) = self.peers.get_mut(&ip) else {
            return;
        };
        if !p.config.active || p.prefix_block.is_some_and(|t| now < t) {
            self.actions.push_back(Action::Close(ip));
            return;
        }
        p.prefix_block = None;
        p.local = local;
        p.state = State::OpenSent;
        p.input.clear();
        p.hold_deadline = now + 240_000;
        self.send(
            ip,
            Message::Open {
                asn: self.config.asn,
                hold: self.config.hold,
                id: self.config.router_id,
                as4: true,
                refresh: true,
            },
        );
    }
    pub fn disconnected(&mut self, ip: Ipv4Addr, now: i64) {
        let mut remove = false;
        if let Some(p) = self.peers.get_mut(&ip) {
            if p.state == State::Established {
                p.since = now;
            }
            p.state = if p.prefix_block.is_some() {
                State::Idle
            } else {
                State::Active
            };
            p.retry = now + 5000;
            p.input.clear();
            p.adj_in.clear();
            p.adj_out.clear();
            p.hold_deadline = i64::MAX;
            p.keepalive_deadline = i64::MAX;
            // Dynamic neighbors exist only while their connection does.
            if p.is_dynamic() {
                if let Some(t) = p.prefix_block {
                    self.blocked.insert(ip, t);
                }
                remove = true;
            }
        }
        if remove {
            self.peers.remove(&ip);
        }
        self.recompute();
    }
    /// `clear bgp`: reset one session (or all with None) and lift maximum-prefix blocks.
    pub fn clear(&mut self, ip: Option<Ipv4Addr>, now: i64) {
        let ips: Vec<_> = self
            .peers
            .keys()
            .copied()
            .filter(|p| ip.is_none_or(|ip| ip == *p))
            .collect();
        match ip {
            Some(ip) => {
                self.blocked.remove(&ip);
            }
            None => self.blocked.clear(),
        }
        for ip in ips {
            let p = self.peers.get_mut(&ip).unwrap();
            p.prefix_block = None;
            if !matches!(p.state, State::Idle | State::Active | State::Connect) {
                self.send(
                    ip,
                    Message::Notification {
                        code: 6,
                        subcode: 4,
                    },
                );
            }
            self.actions.push_back(Action::Close(ip));
            self.disconnected(ip, now);
            if let Some(p) = self.peers.get_mut(&ip) {
                p.retry = now;
            }
        }
    }
    pub fn poll(&mut self, now: i64) -> Option<i64> {
        let ips: Vec<_> = self.peers.keys().copied().collect();
        let mut deadline = i64::MAX;
        for ip in ips {
            let Some(p) = self.peers.get_mut(&ip) else {
                continue;
            };
            if !p.config.active {
                continue;
            }
            if let Some(t) = p.prefix_block {
                if now < t {
                    if t != i64::MAX {
                        deadline = deadline.min(t);
                    }
                    continue;
                }
                p.prefix_block = None;
                p.retry = now;
            }
            match p.state {
                State::Idle | State::Active if p.passive() => {}
                State::Idle | State::Active if now >= p.retry => {
                    p.state = State::Connect;
                    p.retry = now + 5000;
                    self.actions.push_back(Action::Connect(ip));
                }
                State::Connect if now >= p.retry => {
                    self.actions.push_back(Action::Close(ip));
                    p.state = State::Active;
                    p.retry = now + 5000;
                }
                State::OpenSent | State::OpenConfirm | State::Established
                    if now >= p.hold_deadline =>
                {
                    self.fail(ip, 4, 0, now);
                    continue;
                }
                State::Established if now >= p.keepalive_deadline => {
                    p.keepalive_deadline = now
                        + self.config.keepalive.min((p.negotiated_hold / 3).max(1)) as i64 * 1000;
                    self.send(ip, Message::Keepalive);
                }
                _ => {}
            }
            let Some(p) = self.peers.get(&ip) else {
                continue;
            };
            deadline = deadline.min(match p.state {
                State::Idle | State::Active if p.passive() => i64::MAX,
                State::Idle | State::Active | State::Connect => p.retry,
                _ => p.hold_deadline.min(p.keepalive_deadline),
            });
        }
        (deadline != i64::MAX).then_some(deadline)
    }
    fn alive(&self, ip: Ipv4Addr) -> bool {
        self.peers
            .get(&ip)
            .is_some_and(|p| !matches!(p.state, State::Idle | State::Active))
    }
    pub fn receive(&mut self, ip: Ipv4Addr, data: &[u8], now: i64) {
        let Some(p) = self.peers.get_mut(&ip) else {
            return;
        };
        if p.input.len() + data.len() > 65536 {
            self.actions.push_back(Action::Close(ip));
            self.disconnected(ip, now);
            return;
        }
        p.input.extend_from_slice(data);
        loop {
            let Some(p) = self.peers.get(&ip) else {
                break;
            };
            let decoded = Message::decode(&p.input, p.as4).or_else(|e| {
                if e.code == 3 && p.state == State::Established {
                    if let Some(recovered) = wire::withdraw_malformed_update(&p.input) {
                        return Ok(Some(recovered));
                    }
                }
                Err(e)
            });
            let m = match decoded {
                Ok(Some((m, n))) => {
                    let p = self.peers.get_mut(&ip).unwrap();
                    p.input.drain(..n);
                    p.msgs_in += 1;
                    m
                }
                Ok(None) => break,
                Err(e) => {
                    self.fail(ip, e.code, e.subcode, now);
                    break;
                }
            };
            self.message(ip, m, now);
            if !self.alive(ip) {
                break;
            }
        }
    }
    fn fail(&mut self, ip: Ipv4Addr, code: u8, subcode: u8, now: i64) {
        self.send(ip, Message::Notification { code, subcode });
        self.actions.push_back(Action::Close(ip));
        self.disconnected(ip, now);
    }
    fn message(&mut self, ip: Ipv4Addr, m: Message, now: i64) {
        let local_as = self.config.asn;
        let p = self.peers.get_mut(&ip).unwrap();
        if p.negotiated_hold > 0 {
            p.hold_deadline = now + p.negotiated_hold as i64 * 1000;
        }
        match (p.state, m) {
            (
                State::OpenSent,
                Message::Open {
                    asn,
                    hold,
                    id,
                    as4,
                    refresh,
                },
            ) => {
                if !self.as_acceptable(ip, asn) {
                    self.fail(ip, 2, 2, now);
                    return;
                }
                if id == self.config.router_id {
                    self.fail(ip, 2, 3, now);
                    return;
                }
                let hold_local = self.config.hold;
                let p = self.peers.get_mut(&ip).unwrap();
                p.remote_as = asn;
                p.router_id = id;
                p.as4 = as4;
                p.refresh = refresh;
                p.negotiated_hold = hold.min(hold_local);
                p.hold_deadline = if p.negotiated_hold == 0 {
                    i64::MAX
                } else {
                    now + p.negotiated_hold as i64 * 1000
                };
                p.state = State::OpenConfirm;
                self.send(ip, Message::Keepalive);
            }
            (State::OpenConfirm, Message::Keepalive) => {
                p.state = State::Established;
                p.since = now;
                p.keepalive_deadline = if p.negotiated_hold == 0 {
                    i64::MAX
                } else {
                    now + self.config.keepalive.min((p.negotiated_hold / 3).max(1)) as i64 * 1000
                };
                self.advertise(ip, true);
            }
            (State::Established, Message::Keepalive) => {}
            (
                State::Established,
                Message::Update {
                    withdrawn,
                    attributes: mut a,
                    nlri,
                },
            ) => {
                for prefix in withdrawn {
                    p.adj_in.remove(&prefix);
                }
                let external = p.remote_as != local_as;
                // eBGP cannot dictate our local preference; policy may set it.
                if external {
                    a.local_pref = 100;
                }
                for prefix in nlri {
                    let mut attrs = a.clone();
                    if a.path.contains(&local_as)
                        || (external && a.path.first() != Some(&p.remote_as))
                        || !self
                            .policy
                            .apply(p.config.inbound.as_deref(), prefix, &mut attrs)
                    {
                        p.adj_in.remove(&prefix);
                        continue;
                    }
                    if p.adj_in.len() < MAX_PREFIXES || p.adj_in.contains_key(&prefix) {
                        p.adj_in.insert(prefix, attrs);
                    } else {
                        self.fail(ip, 6, 1, now);
                        return;
                    }
                }
                // maximum-prefix counts accepted (post-policy) prefixes, as FRR does.
                if let Some(m) = p.config.maximum_prefix {
                    if p.adj_in.len() > m.max as usize && !m.warning_only {
                        p.prefix_block = Some(match m.restart {
                            Some(secs) => now + secs as i64 * 1000,
                            None => i64::MAX,
                        });
                        self.fail(ip, 6, 1, now);
                        return;
                    }
                }
                self.recompute();
            }
            (State::Established, Message::Refresh) if p.refresh => self.advertise(ip, true),
            (_, Message::Notification { code, subcode }) => {
                p.last_error = Some(LastError {
                    sent: false,
                    code,
                    subcode,
                    at: now,
                });
                self.actions.push_back(Action::Close(ip));
                self.disconnected(ip, now);
            }
            _ => self.fail(ip, 5, 0, now),
        }
    }
    pub fn set_origins(&mut self, prefixes: &[Prefix]) {
        let origins = prefixes
            .iter()
            .take(MAX_PREFIXES)
            .map(|p| (*p, Attributes::default()))
            .collect();
        if self.origins != origins {
            self.origins = origins;
            self.recompute();
        }
    }
    fn recompute(&mut self) {
        let mut rib = BTreeMap::new();
        for (prefix, a) in &self.origins {
            rib.insert(
                *prefix,
                Route {
                    prefix: *prefix,
                    attributes: a.clone(),
                    peer: None,
                    external: false,
                    router_id: self.config.router_id,
                },
            );
        }
        for (ip, p) in &self.peers {
            if p.state != State::Established {
                continue;
            }
            for (prefix, a) in &p.adj_in {
                let r = Route {
                    prefix: *prefix,
                    attributes: a.clone(),
                    peer: Some(*ip),
                    external: p.remote_as != self.config.asn,
                    router_id: p.router_id,
                };
                if rib.get(prefix).is_none_or(|old| better(&r, old))
                    && (rib.len() < MAX_PREFIXES || rib.contains_key(prefix))
                {
                    rib.insert(*prefix, r);
                }
            }
        }
        if rib != self.rib {
            self.rib = rib;
            self.actions.push_back(Action::RoutesChanged);
            let peers: Vec<_> = self.peers.keys().copied().collect();
            for ip in peers {
                self.advertise(ip, false);
            }
        }
    }
    fn advertise(&mut self, ip: Ipv4Addr, force: bool) {
        let p = &self.peers[&ip];
        if p.state != State::Established {
            return;
        }
        let ibgp = p.remote_as == self.config.asn;
        let mut desired = BTreeMap::new();
        for (prefix, r) in &self.rib {
            if r.peer == Some(ip)
                || r.attributes.path.contains(&p.remote_as)
                || r.attributes.communities.contains(&0xffffff02)
                || (!ibgp && r.attributes.communities.contains(&0xffffff01))
                || (ibgp && r.peer.is_some() && !r.external)
            {
                continue;
            }
            let mut a = r.attributes.clone();
            if !ibgp {
                a.prepend(self.config.asn);
                a.next_hop = p.local;
            } else if r.peer.is_none() {
                a.next_hop = p.local;
            }
            if self
                .policy
                .apply(p.config.outbound.as_deref(), *prefix, &mut a)
            {
                desired.insert(*prefix, a);
            }
        }
        if p.config.default_originate {
            let mut a = Attributes::default();
            if !ibgp {
                a.prepend(self.config.asn);
            }
            a.next_hop = p.local;
            let prefix = Prefix::new(Ipv4Addr::ZERO, 0);
            if self
                .policy
                .apply(p.config.outbound.as_deref(), prefix, &mut a)
            {
                desired.insert(prefix, a);
            }
        }
        let withdrawn: Vec<_> = p
            .adj_out
            .keys()
            .filter(|p| !desired.contains_key(p))
            .copied()
            .collect();
        let updates: Vec<_> = desired
            .iter()
            .filter(|(prefix, a)| force || p.adj_out.get(prefix) != Some(a))
            .map(|(p, a)| (*p, a.clone()))
            .collect();
        if !withdrawn.is_empty() {
            self.send(
                ip,
                Message::Update {
                    withdrawn,
                    attributes: Attributes::default(),
                    nlri: Vec::new(),
                },
            );
        }
        for (prefix, attributes) in updates {
            self.send(
                ip,
                Message::Update {
                    withdrawn: Vec::new(),
                    attributes,
                    nlri: vec![prefix],
                },
            );
        }
        self.peers.get_mut(&ip).unwrap().adj_out = desired;
    }
}
pub fn better(a: &Route, b: &Route) -> bool {
    if a.peer.is_none() != b.peer.is_none() {
        return a.peer.is_none();
    }
    if a.attributes.local_pref != b.attributes.local_pref {
        return a.attributes.local_pref > b.attributes.local_pref;
    }
    if a.attributes.path_length() != b.attributes.path_length() {
        return a.attributes.path_length() < b.attributes.path_length();
    }
    if a.attributes.origin != b.attributes.origin {
        return a.attributes.origin < b.attributes.origin;
    }
    if a.attributes.path.first() == b.attributes.path.first()
        && a.attributes.med != b.attributes.med
    {
        return a.attributes.med < b.attributes.med;
    }
    if a.external != b.external {
        return a.external;
    }
    (a.router_id, a.peer) < (b.router_id, b.peer)
}
