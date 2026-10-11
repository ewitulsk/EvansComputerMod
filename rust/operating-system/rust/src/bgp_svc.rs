//! Nonblocking TCP transport adapter for the pure BGP engine.
use ecm_bgp::{Action, Config, Engine, Policy, Prefix};
use ecm_net::{Ipv4Addr, RouteSource, SocketAddr, SocketHandle, Stack, TcpState};
use std::collections::{BTreeMap, VecDeque};
struct Connection {
    handle: SocketHandle,
    opened: bool,
    output: VecDeque<Vec<u8>>,
    offset: usize,
}
pub struct BgpService {
    pub engine: Engine,
    listener: Option<SocketHandle>,
    connections: BTreeMap<Ipv4Addr, Connection>,
    next: i64,
}
impl BgpService {
    pub fn new(config: Config, policy: Policy, now: i64) -> Self {
        Self {
            engine: Engine::new(config, policy, now),
            listener: None,
            connections: BTreeMap::new(),
            next: now,
        }
    }
    pub fn stop(&mut self, stack: &mut Stack) {
        if let Some(h) = self.listener.take() {
            let _ = stack.tcp_abort(h);
        }
        for c in self.connections.values() {
            let _ = stack.tcp_abort(c.handle);
        }
        self.connections.clear();
        stack.remove_protocol_routes(RouteSource::Bgp);
    }
    fn source(&self, stack: &Stack, peer: Ipv4Addr) -> Option<Ipv4Addr> {
        let configured = self
            .engine
            .peers
            .get(&peer)
            .and_then(|p| p.config.update_source.as_ref());
        if let Some(name) = configured {
            Ipv4Addr::parse(name)
                .or_else(|| {
                    stack
                        .find_iface(name)
                        .and_then(|i| stack.iface(i))
                        .map(|f| f.ip)
                })
                .filter(|ip| !ip.is_unspecified())
        } else {
            stack
                .lookup_route(peer)
                .and_then(|(i, _)| stack.iface(i))
                .map(|f| f.ip)
        }
    }
    /// Count of dynamic peers, for `show`.
    pub fn dynamic_peers(&self) -> usize {
        self.engine.dynamic_count(None)
    }
    pub fn poll(&mut self, stack: &mut Stack, now: i64) -> Option<i64> {
        if now < self.next {
            return Some(self.next);
        }
        self.next = now + 50;
        if self.listener.is_none() {
            self.listener = stack
                .tcp_listen(SocketAddr::new(Ipv4Addr::ZERO, 179), 16)
                .ok();
        }
        if let Some(h) = self.listener {
            for _ in 0..16 {
                let Ok(Some(accepted)) = stack.tcp_accept(h) else {
                    break;
                };
                let Ok(remote) = stack.tcp_peer_addr(accepted) else {
                    let _ = stack.tcp_abort(accepted);
                    continue;
                };
                let ip = remote.ip;
                // Configured neighbors, or a dynamic neighbor created from a listen
                // range (within its limits). Anything else is refused with a reset.
                if self.engine.accept(ip, now).is_err() {
                    let _ = stack.tcp_abort(accepted);
                    continue;
                }
                let local = stack
                    .tcp_local_addr(accepted)
                    .map(|a| a.ip)
                    .unwrap_or(Ipv4Addr::ZERO);
                // Resolve simultaneous opens deterministically on the connection endpoints.
                let passive = self.engine.peers.get(&ip).is_some_and(|p| p.passive());
                if let Some(old) = self.connections.get(&ip) {
                    // A passive (dynamic) peer never connects out: a new connection
                    // replaces a stale one (the remote side restarted).
                    if !passive && local < ip {
                        let _ = stack.tcp_abort(accepted);
                        continue;
                    }
                    let _ = stack.tcp_abort(old.handle);
                    self.engine.disconnected(ip, now);
                    // A dynamic peer is gone with its old connection: recreate it.
                    if self.engine.accept(ip, now).is_err() {
                        self.connections.remove(&ip);
                        let _ = stack.tcp_abort(accepted);
                        continue;
                    }
                }
                self.connections.insert(
                    ip,
                    Connection {
                        handle: accepted,
                        opened: true,
                        output: VecDeque::new(),
                        offset: 0,
                    },
                );
                self.engine.connected(ip, local, now);
            }
        }
        let peers: Vec<_> = self.connections.keys().copied().collect();
        let mut dead = Vec::new();
        let mut buf = [0; 4096];
        for ip in peers {
            let c = self.connections.get_mut(&ip).unwrap();
            if let Ok(local) = stack.tcp_local_addr(c.handle) {
                if (0..stack.iface_count())
                    .filter_map(|i| stack.iface(i))
                    .any(|f| f.ip == local.ip && !f.is_up())
                {
                    dead.push(ip);
                    continue;
                }
            }
            match stack.tcp_state(c.handle) {
                Ok(TcpState::Established) => {
                    if !c.opened {
                        c.opened = true;
                        self.engine
                            .connected(ip, stack.tcp_local_addr(c.handle).unwrap().ip, now);
                    }
                    for _ in 0..16 {
                        match stack.tcp_recv(c.handle, &mut buf) {
                            Ok(0) => {
                                dead.push(ip);
                                break;
                            }
                            Ok(n) => self.engine.receive(ip, &buf[..n], now),
                            Err(ecm_net::NetError::WouldBlock) => break,
                            Err(_) => {
                                dead.push(ip);
                                break;
                            }
                        }
                    }
                }
                Ok(TcpState::SynSent | TcpState::SynReceived) => {}
                _ => dead.push(ip),
            }
        }
        for ip in dead {
            if let Some(c) = self.connections.remove(&ip) {
                let _ = stack.tcp_abort(c.handle);
            }
            self.engine.disconnected(ip, now);
        }
        // Only originate configured networks backed by the forwarding table.
        let mut origins = Vec::new();
        for r in stack.routes() {
            let p = Prefix::new(r.dst, r.prefix);
            if (r.source == RouteSource::Connected && self.engine.config.connected)
                || (r.source == RouteSource::Static && self.engine.config.static_routes)
                || (r.source != RouteSource::Bgp && self.engine.config.networks.contains(&p))
            {
                origins.push(p);
            }
        }
        self.engine.set_origins(&origins);
        let deadline = self.engine.poll(now);
        for _ in 0..4096 {
            let Some(a) = self.engine.pop_action() else {
                break;
            };
            match a {
                Action::Connect(ip) => {
                    if self.connections.contains_key(&ip) {
                        continue;
                    }
                    let bound = self.source(stack, ip).unwrap_or(Ipv4Addr::ZERO);
                    match stack.tcp_connect_bound(
                        SocketAddr::new(bound, 0),
                        SocketAddr::new(ip, 179),
                        now,
                    ) {
                        Ok(h) => {
                            self.connections.insert(
                                ip,
                                Connection {
                                    handle: h,
                                    opened: false,
                                    output: VecDeque::new(),
                                    offset: 0,
                                },
                            );
                        }
                        Err(_) => self.engine.disconnected(ip, now),
                    }
                }
                Action::Send(ip, data) => {
                    if let Some(c) = self.connections.get_mut(&ip) {
                        if c.output.len() < 2048 {
                            c.output.push_back(data);
                        } else {
                            let _ = stack.tcp_abort(c.handle);
                        }
                    }
                }
                Action::Close(ip) => {
                    if let Some(c) = self.connections.remove(&ip) {
                        let _ = stack.tcp_abort(c.handle);
                    }
                }
                Action::RoutesChanged => {}
            }
        }
        // Reinstall only the selected BGP paths. Resolve next hops through the
        // connected/static table to avoid recursive stale BGP next hops.
        stack.remove_protocol_routes(RouteSource::Bgp);
        for r in self.engine.rib.values() {
            if r.peer.is_none() {
                continue;
            }
            if let Some((iface, _)) = stack.lookup_route(r.attributes.next_hop) {
                let _ = stack.add_protocol_route(
                    r.prefix.address,
                    r.prefix.len,
                    r.attributes.next_hop,
                    iface,
                    RouteSource::Bgp,
                    if r.external { 20 } else { 200 },
                    r.attributes.path_length() as u32,
                );
            }
        }
        for c in self.connections.values_mut() {
            if !c.opened {
                continue;
            }
            for _ in 0..64 {
                let Some(front) = c.output.front() else {
                    break;
                };
                match stack.tcp_send(c.handle, &front[c.offset..], now) {
                    Ok(n) => {
                        c.offset += n;
                        if c.offset == front.len() {
                            c.output.pop_front();
                            c.offset = 0;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        crate::net::min_opt(deadline, Some(self.next))
    }
}
