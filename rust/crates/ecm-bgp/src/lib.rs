//! BGP engine: virtual time, explicit transport events, bounded RIBs.
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Neighbor {
    pub remote_as: u32,
    pub update_source: Option<String>,
    pub active: bool,
    pub default_originate: bool,
    pub inbound: Option<String>,
    pub outbound: Option<String>,
}
impl Neighbor {
    pub fn new(asn: u32) -> Self {
        Self {
            remote_as: asn,
            update_source: None,
            active: false,
            default_originate: false,
            inbound: None,
            outbound: None,
        }
    }
}
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
    pub networks: Vec<Prefix>,
    pub connected: bool,
    pub static_routes: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            asn: 0,
            router_id: Ipv4Addr::ZERO,
            keepalive: 60,
            hold: 180,
            neighbors: BTreeMap::new(),
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
pub struct Peer {
    pub config: Neighbor,
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
    input: Vec<u8>,
    local: Ipv4Addr,
}
#[derive(Clone, Debug)]
pub enum Action {
    Connect(Ipv4Addr),
    Send(Ipv4Addr, Vec<u8>),
    Close(Ipv4Addr),
    RoutesChanged,
}
pub struct Engine {
    pub config: Config,
    pub policy: Policy,
    pub peers: BTreeMap<Ipv4Addr, Peer>,
    pub rib: BTreeMap<Prefix, Route>,
    origins: BTreeMap<Prefix, Attributes>,
    actions: VecDeque<Action>,
}
impl Engine {
    pub fn new(config: Config, policy: Policy, now: i64) -> Self {
        let peers = config
            .neighbors
            .iter()
            .map(|(ip, c)| {
                (
                    *ip,
                    Peer {
                        config: c.clone(),
                        state: State::Idle,
                        router_id: Ipv4Addr::ZERO,
                        as4: false,
                        refresh: false,
                        adj_in: BTreeMap::new(),
                        adj_out: BTreeMap::new(),
                        retry: now,
                        hold_deadline: i64::MAX,
                        keepalive_deadline: i64::MAX,
                        negotiated_hold: config.hold,
                        input: Vec::new(),
                        local: Ipv4Addr::ZERO,
                    },
                )
            })
            .collect();
        Self {
            config,
            policy,
            peers,
            rib: BTreeMap::new(),
            origins: BTreeMap::new(),
            actions: VecDeque::new(),
        }
    }
    pub fn pop_action(&mut self) -> Option<Action> {
        self.actions.pop_front()
    }
    fn send(&mut self, ip: Ipv4Addr, m: Message) {
        let as4 = self.peers.get(&ip).is_some_and(|p| p.as4);
        let internal = self
            .peers
            .get(&ip)
            .is_some_and(|p| p.config.remote_as == self.config.asn);
        let b = m.encode_for_peer(as4, internal);
        if b.len() <= wire::MAX_MESSAGE {
            self.actions.push_back(Action::Send(ip, b));
        }
    }
    pub fn connected(&mut self, ip: Ipv4Addr, local: Ipv4Addr, now: i64) {
        let Some(p) = self.peers.get_mut(&ip) else {
            return;
        };
        if !p.config.active {
            self.actions.push_back(Action::Close(ip));
            return;
        }
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
        if let Some(p) = self.peers.get_mut(&ip) {
            p.state = State::Active;
            p.retry = now + 5000;
            p.input.clear();
            p.adj_in.clear();
            p.adj_out.clear();
            p.hold_deadline = i64::MAX;
            p.keepalive_deadline = i64::MAX;
        }
        self.recompute();
    }
    pub fn poll(&mut self, now: i64) -> Option<i64> {
        let ips: Vec<_> = self.peers.keys().copied().collect();
        let mut deadline = i64::MAX;
        for ip in ips {
            let p = self.peers.get_mut(&ip).unwrap();
            if !p.config.active {
                continue;
            }
            match p.state {
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
                    self.send(
                        ip,
                        Message::Notification {
                            code: 4,
                            subcode: 0,
                        },
                    );
                    self.actions.push_back(Action::Close(ip));
                    self.disconnected(ip, now);
                    continue;
                }
                State::Established if now >= p.keepalive_deadline => {
                    p.keepalive_deadline = now
                        + self.config.keepalive.min((p.negotiated_hold / 3).max(1)) as i64 * 1000;
                    self.send(ip, Message::Keepalive);
                }
                _ => {}
            }
            let p = &self.peers[&ip];
            deadline = deadline.min(match p.state {
                State::Idle | State::Active | State::Connect => p.retry,
                _ => p.hold_deadline.min(p.keepalive_deadline),
            });
        }
        (deadline != i64::MAX).then_some(deadline)
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
            let p = &self.peers[&ip];
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
                    self.peers.get_mut(&ip).unwrap().input.drain(..n);
                    m
                }
                Ok(None) => break,
                Err(e) => {
                    self.send(
                        ip,
                        Message::Notification {
                            code: e.code,
                            subcode: e.subcode,
                        },
                    );
                    self.actions.push_back(Action::Close(ip));
                    self.disconnected(ip, now);
                    break;
                }
            };
            self.message(ip, m, now);
            if matches!(self.peers[&ip].state, State::Idle | State::Active) {
                break;
            }
        }
    }
    fn message(&mut self, ip: Ipv4Addr, m: Message, now: i64) {
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
                if asn != p.config.remote_as || id == self.config.router_id {
                    self.send(
                        ip,
                        Message::Notification {
                            code: 2,
                            subcode: 2,
                        },
                    );
                    self.actions.push_back(Action::Close(ip));
                    self.disconnected(ip, now);
                    return;
                }
                p.router_id = id;
                p.as4 = as4;
                p.refresh = refresh;
                p.negotiated_hold = hold.min(self.config.hold);
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
                // eBGP cannot dictate our local preference; policy may set it.
                if p.config.remote_as != self.config.asn {
                    a.local_pref = 100;
                }
                for prefix in nlri {
                    let mut attrs = a.clone();
                    if a.path.contains(&self.config.asn)
                        || (p.config.remote_as != self.config.asn
                            && a.path.first() != Some(&p.config.remote_as))
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
                        self.send(
                            ip,
                            Message::Notification {
                                code: 6,
                                subcode: 1,
                            },
                        );
                        self.actions.push_back(Action::Close(ip));
                        self.disconnected(ip, now);
                        return;
                    }
                }
                self.recompute();
            }
            (State::Established, Message::Refresh) if p.refresh => self.advertise(ip, true),
            (_, Message::Notification { .. }) => {
                self.actions.push_back(Action::Close(ip));
                self.disconnected(ip, now);
            }
            _ => {
                self.send(
                    ip,
                    Message::Notification {
                        code: 5,
                        subcode: 0,
                    },
                );
                self.actions.push_back(Action::Close(ip));
                self.disconnected(ip, now);
            }
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
                    external: p.config.remote_as != self.config.asn,
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
        let mut desired = BTreeMap::new();
        for (prefix, r) in &self.rib {
            if r.peer == Some(ip)
                || r.attributes.path.contains(&p.config.remote_as)
                || r.attributes.communities.contains(&0xffffff02)
                || (p.config.remote_as != self.config.asn
                    && r.attributes.communities.contains(&0xffffff01))
                || (p.config.remote_as == self.config.asn && r.peer.is_some() && !r.external)
            {
                continue;
            }
            let mut a = r.attributes.clone();
            if p.config.remote_as != self.config.asn {
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
            if p.config.remote_as != self.config.asn {
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
