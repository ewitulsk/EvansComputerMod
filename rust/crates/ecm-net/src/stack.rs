//! The sans-IO host stack: interfaces, routing, ARP, IPv4, ICMP, UDP, TCP, DNS.
//!
//! Nothing here performs I/O or reads a clock. Frames come in through
//! [`Stack::handle_frame`], go out through [`Stack::pop_tx`], and time is always
//! an argument. All tables are bounded; the drop policy is documented on each
//! constant.

use std::collections::VecDeque;

use crate::arp::{ArpPacket, ARP_REPLY, ARP_REQUEST};
use crate::dns;
use crate::eth::{build_frame, EthHeader, VlanTag, ETHERTYPE_ARP, ETHERTYPE_IPV4};
use crate::icmp::{IcmpPacket, ICMP_ECHO_REPLY, ICMP_ECHO_REQUEST};
use crate::ipv4::{build_packet, Ipv4Header, MAX_PAYLOAD, PROTO_ICMP, PROTO_TCP, PROTO_UDP};
use crate::tcb::{Seg, Tcb};
use crate::tcp::{self, seq_gt, TcpHeader, TcpState, ACK, FIN, RST, SYN};
use crate::types::{Ipv4Addr, MacAddr, NetError, SocketAddr};
use crate::udp::{self, UdpHeader};

/// Interface slots.
pub const MAX_INTERFACES: usize = 32;
/// Routing table entries (add beyond this → `BufferFull`).
pub const MAX_ROUTES: usize = 512;
/// Neighbour entries per interface. When full, the least recently used resolved
/// entry is evicted; if every entry is still resolving, the new packet is dropped.
pub const MAX_NEIGHBORS: usize = 32;
/// Packets queued per unresolved neighbour (drop newest when full).
pub const ARP_PENDING_PER_NEIGHBOR: usize = 4;
pub const ARP_RETRY_MS: i64 = 1000;
/// ARP requests sent before a neighbour is declared unreachable.
pub const ARP_MAX_REQUESTS: u8 = 3;
/// A learned neighbour is Reachable for this long, then Stale (re-probed on use).
pub const ARP_REACHABLE_MS: i64 = 60_000;
/// Socket slots shared by TCP (listeners + connections, incl. TIME_WAIT), UDP and ICMP.
pub const MAX_SOCKETS: usize = 128;
/// Datagrams queued per UDP socket (drop newest when full).
pub const UDP_QUEUE_LEN: usize = 32;
/// Messages queued per raw ICMP socket (drop newest when full).
pub const ICMP_QUEUE_LEN: usize = 32;
/// Frames waiting for `pop_tx` (drop newest when full, counted as `tx_dropped`).
pub const TX_QUEUE_LEN: usize = 1024;
/// Packets waiting for local (loopback) delivery (drop newest when full).
pub const LOOPBACK_QUEUE_LEN: usize = 1024;
pub const MAX_DNS_QUERIES: usize = 16;
pub const DNS_TIMEOUT_MS: i64 = 3000;
pub const DNS_TRIES: u8 = 2;
/// Upper bound on a listener's accept queue.
pub const MAX_BACKLOG: usize = 64;
const MAX_IFNAME: usize = 15;
const EPHEMERAL_START: u16 = 49152;

/// Construction parameters. `seed` drives ISNs, ephemeral ports and DNS IDs.
#[derive(Clone, Copy, Debug, Default)]
pub struct StackConfig {
    pub seed: u64,
}

/// Generation-checked socket handle. A stale handle yields `Err(BadHandle)`.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct SocketHandle {
    idx: u16,
    gen: u16,
}

impl SocketHandle {
    /// Pack into a u32 (e.g. to store in an fd table). Garbage from `from_raw` is harmless.
    pub fn to_raw(self) -> u32 {
        ((self.gen as u32) << 16) | self.idx as u32
    }
    pub fn from_raw(v: u32) -> Self {
        SocketHandle {
            idx: v as u16,
            gen: (v >> 16) as u16,
        }
    }
}

/// Handle for an in-flight DNS query.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct DnsHandle {
    idx: u16,
    gen: u16,
}

impl DnsHandle {
    pub fn to_raw(self) -> u32 {
        ((self.gen as u32) << 16) | self.idx as u32
    }
    pub fn from_raw(v: u32) -> Self {
        DnsHandle {
            idx: v as u16,
            gen: (v >> 16) as u16,
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum DnsStatus {
    Pending,
    Resolved(Ipv4Addr),
    Failed(NetError),
}

/// Per-interface counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IfStats {
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    /// Malformed frames/packets, bad checksums.
    pub rx_errors: u64,
    /// Well-formed but discarded: iface down, VLAN mismatch, unknown ethertype, fragments, ...
    pub rx_dropped: u64,
    /// ARP queue overflow / failure, TX queue full, iface down.
    pub tx_dropped: u64,
}

/// Stack-wide counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StackStats {
    pub ip_rx_bad: u64,
    pub ip_rx_fragments_dropped: u64,
    pub ip_rx_not_for_us: u64,
    pub icmp_rx_bad: u64,
    pub udp_rx_bad: u64,
    pub udp_rx_no_socket: u64,
    pub udp_rx_queue_full: u64,
    pub tcp_rx_bad: u64,
    pub tcp_rst_sent: u64,
    /// Segments sent again (RTO, fast retransmit, SYN retries).
    pub tcp_retransmits: u64,
    pub arp_failures: u64,
    pub loopback_dropped: u64,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum NeighborState {
    /// Resolving; packets are queued.
    Incomplete,
    /// Confirmed within the last 60 s.
    Reachable,
    /// Older than 60 s; next use re-probes.
    Stale,
    /// Being re-verified; still used for sending.
    Probe,
}

#[derive(Clone, Copy, Debug)]
enum Owner {
    None,
    Sock(SocketHandle),
    Dns(DnsHandle),
}

struct Neighbor {
    ip: Ipv4Addr,
    mac: MacAddr,
    state: NeighborState,
    /// Next ARP (Incomplete/Probe) or end of reachability (Reachable).
    deadline: i64,
    requests: u8,
    pending: VecDeque<(Vec<u8>, Owner)>,
    last_used: i64,
}

/// A network interface (physical port or SVI).
pub struct Interface {
    pub name: String,
    pub mac: MacAddr,
    pub ip: Ipv4Addr,
    pub prefix: u8,
    /// Carrier (set by the host via `set_link`). Defaults to up.
    pub link_up: bool,
    /// Administrative state (`ifconfig up/down`). Defaults to up.
    pub admin_up: bool,
    /// Host 802.1Q tag for this interface.
    pub vlan: Option<u16>,
    pub stats: IfStats,
    neighbors: Vec<Neighbor>,
}

impl Interface {
    pub fn is_up(&self) -> bool {
        self.link_up && self.admin_up
    }
    pub fn is_configured(&self) -> bool {
        !self.ip.is_unspecified()
    }
    pub fn netmask(&self) -> Ipv4Addr {
        Ipv4Addr::mask_from_prefix(self.prefix)
    }
}

/// Routing table entry. `gateway == 0.0.0.0` means on-link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RouteSource {
    Connected = 2,
    Static = 4,
    Dhcp = 16,
    Bgp = 186,
}

impl RouteSource {
    pub fn distance(self) -> u8 {
        match self {
            Self::Connected => 0,
            Self::Static => 1,
            Self::Dhcp => 5,
            Self::Bgp => 20,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub dst: Ipv4Addr,
    pub prefix: u8,
    pub gateway: Ipv4Addr,
    pub iface: usize,
    pub source: RouteSource,
    pub distance: u8,
    pub metric: u32,
}

struct TcpSock {
    tcb: Tcb,
    accept_q: VecDeque<SocketHandle>,
    backlog: usize,
}

struct UdpSock {
    local: SocketAddr,
    rx: VecDeque<(SocketAddr, Vec<u8>)>,
    error: Option<NetError>,
    dead: bool,
}

struct IcmpSock {
    rx: VecDeque<(Ipv4Addr, Vec<u8>)>,
    error: Option<NetError>,
    ttl: u8,
}

enum Sock {
    Tcp(Box<TcpSock>),
    Udp(UdpSock),
    Icmp(IcmpSock),
}

struct Slot {
    gen: u16,
    sock: Option<Sock>,
}

struct DnsQuery {
    name: String,
    txid: u16,
    port: u16,
    server: Ipv4Addr,
    tries: u8,
    deadline: i64,
    status: DnsStatus,
}

struct DnsSlot {
    gen: u16,
    q: Option<DnsQuery>,
}

struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }
}

fn next_gen(g: u16) -> u16 {
    match g.wrapping_add(1) {
        0 => 1,
        n => n,
    }
}

/// The host network stack. See the crate docs for the driving model.
pub struct Stack {
    ifaces: Vec<Option<Interface>>,
    routes: Vec<Route>,
    dns_server: Ipv4Addr,
    sockets: Vec<Slot>,
    dns: Vec<DnsSlot>,
    tx: VecDeque<(usize, Vec<u8>)>,
    loopback: VecDeque<Vec<u8>>,
    rng: Rng,
    ip_id: u16,
    now: i64,
    stats: StackStats,
    forwarding: bool,
    icmp_error_next: i64,
    dhcp_clients: std::collections::BTreeMap<usize, crate::dhcp::Client>,
}

impl Stack {
    pub fn new(cfg: StackConfig) -> Self {
        let mut rng = Rng(cfg.seed);
        let ip_id = rng.next_u32() as u16;
        Stack {
            ifaces: Vec::new(),
            routes: Vec::new(),
            dns_server: Ipv4Addr::ZERO,
            sockets: Vec::new(),
            dns: Vec::new(),
            tx: VecDeque::new(),
            loopback: VecDeque::new(),
            rng,
            ip_id,
            now: 0,
            stats: StackStats::default(),
            forwarding: false,
            icmp_error_next: i64::MIN,
            dhcp_clients: std::collections::BTreeMap::new(),
        }
    }

    /// Stack-wide counters (a snapshot).
    pub fn stats(&self) -> StackStats {
        let mut s = self.stats;
        for slot in &self.sockets {
            if let Some(Sock::Tcp(t)) = &slot.sock {
                s.tcp_retransmits += t.tcb.retransmits;
            }
        }
        s
    }

    // ================================================================ interfaces

    /// Add an interface; returns its index (lowest free slot). `None` if 16 are in
    /// use, or the name is empty, longer than 15 bytes, or already taken.
    pub fn add_interface(&mut self, name: &str, mac: MacAddr) -> Option<usize> {
        if name.is_empty() || name.len() > MAX_IFNAME || self.find_iface(name).is_some() {
            return None;
        }
        let ifc = Interface {
            name: name.to_string(),
            mac,
            ip: Ipv4Addr::ZERO,
            prefix: 0,
            link_up: true,
            admin_up: true,
            vlan: None,
            stats: IfStats::default(),
            neighbors: Vec::new(),
        };
        if let Some(i) = self.ifaces.iter().position(Option::is_none) {
            self.ifaces[i] = Some(ifc);
            return Some(i);
        }
        if self.ifaces.len() >= MAX_INTERFACES {
            return None;
        }
        self.ifaces.push(Some(ifc));
        Some(self.ifaces.len() - 1)
    }

    /// Remove an interface: drops its routes and queued frames, and closes sockets
    /// bound to its IP (TCP → Closed with `ConnectionAborted`; UDP → every call errors).
    pub fn remove_interface(&mut self, idx: usize) {
        self.dhcp_clients.remove(&idx);
        let Some(ifc) = self.ifaces.get_mut(idx).and_then(Option::take) else {
            return;
        };
        self.routes.retain(|r| r.iface != idx);
        self.tx.retain(|(i, _)| *i != idx);
        let ip = ifc.ip;
        if ip.is_unspecified() || self.local_iface(ip).is_some() {
            return;
        }
        for i in 0..self.sockets.len() {
            match self.sockets[i].sock.as_mut() {
                Some(Sock::Tcp(t)) if t.tcb.local.ip == ip => {
                    t.tcb.fail(NetError::ConnectionAborted)
                }
                Some(Sock::Udp(u)) if u.local.ip == ip => {
                    u.dead = true;
                    u.rx.clear();
                }
                _ => continue,
            }
            self.reap(i);
        }
    }

    /// Highest interface index + 1 (slots below it may be empty).
    pub fn iface_count(&self) -> usize {
        self.ifaces
            .iter()
            .rposition(Option::is_some)
            .map_or(0, |i| i + 1)
    }

    pub fn iface(&self, idx: usize) -> Option<&Interface> {
        self.ifaces.get(idx).and_then(Option::as_ref)
    }

    fn iface_mut(&mut self, idx: usize) -> Option<&mut Interface> {
        self.ifaces.get_mut(idx).and_then(Option::as_mut)
    }

    pub fn find_iface(&self, name: &str) -> Option<usize> {
        self.ifaces
            .iter()
            .position(|i| i.as_ref().is_some_and(|i| i.name == name))
    }

    /// Carrier up/down. Going down flushes the neighbour cache; coming up sends a
    /// gratuitous ARP if configured.
    pub fn set_link(&mut self, idx: usize, up: bool, now_ms: i64) {
        self.now = now_ms;
        self.change_updown(idx, |i| i.link_up = up);
    }

    /// Administrative up/down (same side effects as `set_link`).
    pub fn set_admin_up(&mut self, idx: usize, up: bool, now_ms: i64) {
        self.now = now_ms;
        self.change_updown(idx, |i| i.admin_up = up);
    }

    fn change_updown(&mut self, idx: usize, f: impl FnOnce(&mut Interface)) {
        let Some(ifc) = self.iface_mut(idx) else {
            return;
        };
        let was = ifc.is_up();
        f(ifc);
        let is = ifc.is_up();
        if was && !is {
            ifc.neighbors.clear();
        }
        if !was && is && ifc.is_configured() {
            self.send_gratuitous_arp(idx);
        }
    }

    /// Host 802.1Q tagging: `Some(1..=4094)` tags egress and accepts only that tag;
    /// `None` sends untagged and accepts untagged or VID-0 frames. Other values are ignored.
    pub fn set_vlan(&mut self, idx: usize, vid: Option<u16>) {
        if let Some(v) = vid {
            if v == 0 || v >= 4095 {
                return;
            }
        }
        if let Some(ifc) = self.iface_mut(idx) {
            ifc.vlan = vid;
            ifc.neighbors.clear();
        }
    }

    /// Set the address: replaces the old connected route, flushes ARP on that
    /// interface and sends a gratuitous ARP. `ip == 0.0.0.0` clears; prefix > 32 is ignored.
    pub fn configure_addr(&mut self, idx: usize, ip: Ipv4Addr, prefix: u8, now_ms: i64) {
        self.now = now_ms;
        if prefix > 32 || self.iface(idx).is_none() {
            return;
        }
        if ip.is_unspecified() {
            self.clear_addr(idx);
            return;
        }
        self.remove_connected_route(idx);
        let Some(ifc) = self.iface_mut(idx) else {
            return;
        };
        ifc.ip = ip;
        ifc.prefix = prefix;
        ifc.neighbors.clear();
        let net = ip.network_addr(prefix);
        if self.routes.len() < MAX_ROUTES {
            self.routes.push(Route {
                dst: net,
                prefix,
                gateway: Ipv4Addr::ZERO,
                iface: idx,
                source: RouteSource::Connected,
                distance: 0,
                metric: 0,
            });
        }
        self.send_gratuitous_arp(idx);
    }

    fn remove_connected_route(&mut self, idx: usize) {
        self.routes
            .retain(|r| !(r.iface == idx && r.source == RouteSource::Connected));
    }

    /// Remove the address and every route through this interface; flush ARP.
    pub fn clear_addr(&mut self, idx: usize) {
        let Some(ifc) = self.iface_mut(idx) else {
            return;
        };
        ifc.ip = Ipv4Addr::ZERO;
        ifc.prefix = 0;
        ifc.neighbors.clear();
        self.routes.retain(|r| r.iface != idx);
    }

    pub fn routes(&self) -> &[Route] {
        &self.routes
    }

    /// Add or replace a route. `dst` is masked to `prefix`. Errors: `InvalidInput`
    /// (no such iface, prefix > 32, gateway is one of our addresses), `BufferFull`.
    pub fn add_route(
        &mut self,
        dst: Ipv4Addr,
        prefix: u8,
        gw: Ipv4Addr,
        iface: usize,
    ) -> Result<(), NetError> {
        self.add_protocol_route(dst, prefix, gw, iface, RouteSource::Static, 1, 0)
    }

    pub fn add_protocol_route(
        &mut self,
        dst: Ipv4Addr,
        prefix: u8,
        gw: Ipv4Addr,
        iface: usize,
        source: RouteSource,
        distance: u8,
        metric: u32,
    ) -> Result<(), NetError> {
        if prefix > 32 || self.iface(iface).is_none() || self.is_local(gw) {
            return Err(NetError::InvalidInput);
        }
        let dst = dst.network_addr(prefix);
        let r = Route {
            dst,
            prefix,
            gateway: gw,
            iface,
            source,
            distance,
            metric,
        };
        if let Some(e) = self
            .routes
            .iter_mut()
            .find(|e| e.dst == dst && e.prefix == prefix && e.source == source && e.iface == iface)
        {
            *e = r;
            return Ok(());
        }
        if self.routes.len() >= MAX_ROUTES {
            return Err(NetError::BufferFull);
        }
        self.routes.push(r);
        Ok(())
    }

    pub fn del_route(&mut self, dst: Ipv4Addr, prefix: u8) -> Result<(), NetError> {
        if prefix > 32 {
            return Err(NetError::InvalidInput);
        }
        let dst = dst.network_addr(prefix);
        let before = self.routes.len();
        self.routes
            .retain(|e| !(e.dst == dst && e.prefix == prefix && e.source == RouteSource::Static));
        if self.routes.len() == before {
            Err(NetError::NotFound)
        } else {
            Ok(())
        }
    }

    pub fn dns_server(&self) -> Ipv4Addr {
        self.dns_server
    }

    pub fn remove_protocol_routes(&mut self, source: RouteSource) {
        self.routes.retain(|r| r.source != source);
    }
    pub fn set_forwarding(&mut self, enabled: bool) {
        self.forwarding = enabled;
    }
    pub fn forwarding(&self) -> bool {
        self.forwarding
    }
    pub fn dhcp_enabled(&self, iface: usize) -> bool {
        self.dhcp_clients.contains_key(&iface)
    }
    pub fn start_dhcp(&mut self, iface: usize, now: i64) -> Result<(), NetError> {
        let mac = self.iface(iface).ok_or(NetError::InvalidInput)?.mac;
        self.clear_addr(iface);
        let xid = self.rng.next_u32();
        self.dhcp_clients
            .insert(iface, crate::dhcp::Client::new(mac, xid, now));
        Ok(())
    }
    pub fn stop_dhcp(&mut self, iface: usize) {
        self.dhcp_clients.remove(&iface);
    }

    /// Explicit-interface datagram, including DHCP bootstrap on an unconfigured NIC.
    pub fn send_udp_on_interface(
        &mut self,
        iface: usize,
        src: Ipv4Addr,
        dst: Ipv4Addr,
        src_port: u16,
        dst_port: u16,
        data: &[u8],
        now: i64,
    ) -> Result<(), NetError> {
        self.now = now;
        if !self.iface(iface).is_some_and(Interface::is_up) {
            return Err(NetError::NoRoute);
        }
        if data.len() > udp::MAX_PAYLOAD {
            return Err(NetError::MessageTooLong);
        }
        let dg = udp::build(src, dst, src_port, dst_port, data);
        let id = self.next_id();
        let pkt = build_packet(src, dst, PROTO_UDP, id, &dg).ok_or(NetError::MessageTooLong)?;
        if dst.is_broadcast() {
            self.emit(iface, MacAddr::BROADCAST, ETHERTYPE_IPV4, &pkt);
        } else {
            self.resolve_and_send(iface, dst, pkt, Owner::None);
        }
        Ok(())
    }

    fn apply_dhcp_lease(&mut self, iface: usize) {
        self.routes
            .retain(|r| !(r.iface == iface && r.source == RouteSource::Dhcp));
        let lease = self.dhcp_clients.get(&iface).and_then(|c| c.lease.clone());
        if let Some(l) = lease {
            self.configure_addr(iface, l.address, l.prefix, self.now);
            if let Some(gw) = l.router {
                let _ =
                    self.add_protocol_route(Ipv4Addr::ZERO, 0, gw, iface, RouteSource::Dhcp, 5, 0);
            }
            if let Some(dns) = l.dns {
                self.set_dns_server(dns);
            }
        } else {
            self.clear_addr(iface);
        }
    }

    fn dhcp_timers(&mut self) {
        let ids: Vec<_> = self.dhcp_clients.keys().copied().collect();
        for i in ids {
            if !self.iface(i).is_some_and(Interface::is_up) {
                continue;
            }
            let c = self.dhcp_clients.get_mut(&i).unwrap();
            let had_lease = c.lease.is_some();
            let output = c.poll(self.now);
            let expired = had_lease && c.lease.is_none();
            if expired {
                self.apply_dhcp_lease(i);
            }
            if let Some((dst, msg)) = output {
                let _ =
                    self.send_udp_on_interface(i, msg.ciaddr, dst, 68, 67, &msg.encode(), self.now);
            }
        }
    }
    pub fn lookup_route(&self, dst: Ipv4Addr) -> Option<(usize, Ipv4Addr)> {
        self.route(dst, Ipv4Addr::ZERO).ok().map(|(i, n, _)| (i, n))
    }

    pub fn set_dns_server(&mut self, ip: Ipv4Addr) {
        self.dns_server = ip;
    }

    /// Neighbour cache of one interface (empty for a bad index). Incomplete entries
    /// report `MacAddr::ZERO`.
    pub fn neighbors(
        &self,
        idx: usize,
    ) -> impl Iterator<Item = (Ipv4Addr, MacAddr, NeighborState)> + '_ {
        self.iface(idx)
            .map(|i| i.neighbors.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(|n| (n.ip, n.mac, n.state))
    }

    /// All TCP sockets: (handle-or-None for orphans, local, remote, state).
    pub fn tcp_list(
        &self,
    ) -> impl Iterator<Item = (Option<SocketHandle>, SocketAddr, SocketAddr, TcpState)> + '_ {
        self.sockets
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match &s.sock {
                Some(Sock::Tcp(t)) => {
                    let h = SocketHandle {
                        idx: i as u16,
                        gen: s.gen,
                    };
                    Some((
                        t.tcb.user_open.then_some(h),
                        t.tcb.local,
                        t.tcb.remote,
                        t.tcb.state,
                    ))
                }
                _ => None,
            })
    }

    /// All UDP sockets' local addresses.
    pub fn udp_list(&self) -> impl Iterator<Item = (SocketHandle, SocketAddr)> + '_ {
        self.sockets
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match &s.sock {
                Some(Sock::Udp(u)) => Some((
                    SocketHandle {
                        idx: i as u16,
                        gen: s.gen,
                    },
                    u.local,
                )),
                _ => None,
            })
    }

    // ===================================================================== I/O

    /// Process one received frame from interface `iface`. Never panics. Frames not
    /// addressed to the interface MAC or broadcast are ignored without counting.
    pub fn handle_frame(&mut self, iface: usize, frame: &[u8], now_ms: i64) {
        self.now = now_ms;
        let Some(ifc) = self.iface_mut(iface) else {
            return;
        };
        let Some((eh, payload)) = EthHeader::parse(frame) else {
            if ifc.is_up() {
                ifc.stats.rx_errors += 1;
            }
            return;
        };
        if !(eh.dst == ifc.mac || eh.dst.is_broadcast()) || eh.src == ifc.mac {
            return; // not for us (promiscuous NIC / hub), or our own frame reflected
        }
        if !ifc.is_up() {
            ifc.stats.rx_dropped += 1;
            return;
        }
        let vlan_ok = match (ifc.vlan, eh.vlan_tag) {
            (Some(v), Some(t)) => t.vid == v,
            (None, None) => true,
            (None, Some(t)) => t.vid == 0,
            (Some(_), None) => false,
        };
        if !vlan_ok {
            ifc.stats.rx_dropped += 1;
            return;
        }
        ifc.stats.rx_packets += 1;
        ifc.stats.rx_bytes += frame.len() as u64;
        match eh.ethertype {
            ETHERTYPE_ARP => self.rx_arp(iface, payload),
            ETHERTYPE_IPV4 => self.rx_ip(Some(iface), payload),
            _ => ifc.stats.rx_dropped += 1,
        }
        self.drain_loopback();
    }

    /// Run timers (TCP, ARP, DNS) and pending loopback delivery. Returns the next
    /// absolute deadline in ms, or `None` if nothing is scheduled. Cheap and
    /// idempotent: call it after every batch of API calls to learn the new deadline.
    pub fn poll(&mut self, now_ms: i64) -> Option<i64> {
        self.now = now_ms;
        self.drain_loopback();
        self.neighbor_timers();
        self.tcp_timers();
        self.dns_timers();
        self.dhcp_timers();
        self.drain_loopback();
        self.next_deadline()
    }

    /// Next frame to transmit: (interface index, full ethernet frame).
    pub fn pop_tx(&mut self) -> Option<(usize, Vec<u8>)> {
        self.tx.pop_front()
    }

    fn next_deadline(&self) -> Option<i64> {
        let mut best: Option<i64> = None;
        let mut take = |d: i64| best = Some(best.map_or(d, |b| b.min(d)));
        if !self.loopback.is_empty() {
            take(self.now);
        }
        for ifc in self.ifaces.iter().flatten() {
            for n in &ifc.neighbors {
                if matches!(n.state, NeighborState::Incomplete | NeighborState::Probe) {
                    take(n.deadline);
                }
            }
        }
        for s in &self.sockets {
            if let Some(Sock::Tcp(t)) = &s.sock {
                if let Some(d) = t.tcb.next_deadline() {
                    take(d);
                }
            }
        }
        for d in &self.dns {
            if let Some(q) = &d.q {
                if q.status == DnsStatus::Pending {
                    take(q.deadline);
                }
            }
        }
        for (i, c) in &self.dhcp_clients {
            if self.iface(*i).is_some_and(Interface::is_up) {
                take(c.deadline);
            }
        }
        best
    }

    // ================================================================= sockets

    fn alloc(&mut self, sock: Sock) -> Result<SocketHandle, NetError> {
        if let Some(i) = self.sockets.iter().position(|s| s.sock.is_none()) {
            self.sockets[i].sock = Some(sock);
            return Ok(SocketHandle {
                idx: i as u16,
                gen: self.sockets[i].gen,
            });
        }
        if self.sockets.len() >= MAX_SOCKETS {
            return Err(NetError::NoSockets);
        }
        self.sockets.push(Slot {
            gen: 1,
            sock: Some(sock),
        });
        Ok(SocketHandle {
            idx: (self.sockets.len() - 1) as u16,
            gen: 1,
        })
    }

    fn free(&mut self, idx: usize) {
        if let Some(s) = self.sockets.get_mut(idx) {
            if let Some(Sock::Tcp(t)) = &s.sock {
                self.stats.tcp_retransmits += t.tcb.retransmits;
            }
            s.sock = None;
            s.gen = next_gen(s.gen);
        }
    }

    fn slot(&self, h: SocketHandle) -> Option<&Sock> {
        let s = self.sockets.get(h.idx as usize)?;
        if s.gen != h.gen {
            return None;
        }
        s.sock.as_ref()
    }

    fn slot_mut(&mut self, h: SocketHandle) -> Option<&mut Sock> {
        let s = self.sockets.get_mut(h.idx as usize)?;
        if s.gen != h.gen {
            return None;
        }
        s.sock.as_mut()
    }

    fn tcp_ref(&self, h: SocketHandle) -> Result<&TcpSock, NetError> {
        match self.slot(h) {
            Some(Sock::Tcp(t)) if t.tcb.user_open => Ok(t),
            _ => Err(NetError::BadHandle),
        }
    }

    fn tcp_mut(&mut self, h: SocketHandle) -> Result<&mut TcpSock, NetError> {
        match self.slot_mut(h) {
            Some(Sock::Tcp(t)) if t.tcb.user_open => Ok(t),
            _ => Err(NetError::BadHandle),
        }
    }

    /// Free a TCP slot whose connection is finished and not user-owned.
    fn reap(&mut self, idx: usize) {
        let dead = match self.sockets.get(idx).and_then(|s| s.sock.as_ref()) {
            Some(Sock::Tcp(t)) => t.tcb.state == TcpState::Closed && !t.tcb.user_open,
            _ => false,
        };
        if dead {
            self.free(idx);
        }
    }

    fn rand_port(&mut self) -> u16 {
        EPHEMERAL_START + (self.rng.next_u32() % (65536 - EPHEMERAL_START as u32)) as u16
    }

    fn tcp_port_used(&self, port: u16) -> bool {
        self.sockets
            .iter()
            .any(|s| matches!(&s.sock, Some(Sock::Tcp(t)) if t.tcb.local.port == port))
    }

    fn udp_port_used(&self, ip: Ipv4Addr, port: u16) -> bool {
        self.sockets.iter().any(|s| {
            matches!(&s.sock, Some(Sock::Udp(u)) if u.local.port == port
                && (u.local.ip == ip || u.local.ip.is_unspecified() || ip.is_unspecified()))
        }) || self
            .dns
            .iter()
            .any(|d| d.q.as_ref().is_some_and(|q| q.port == port))
    }

    fn pick_port(&mut self, used: impl Fn(&Self, u16) -> bool) -> Result<u16, NetError> {
        for _ in 0..64 {
            let p = self.rand_port();
            if !used(self, p) {
                return Ok(p);
            }
        }
        (EPHEMERAL_START..=u16::MAX)
            .find(|&p| !used(self, p))
            .ok_or(NetError::AddrInUse)
    }

    fn gen_isn(&mut self) -> u32 {
        self.rng
            .next_u32()
            .wrapping_add((self.now as u64).wrapping_mul(250) as u32)
    }

    /// Source address for a socket bound to `bound` sending to `dst`.
    fn select_src(&self, bound: Ipv4Addr, dst: Ipv4Addr) -> Result<Ipv4Addr, NetError> {
        if !bound.is_unspecified() {
            return Ok(bound);
        }
        if self.is_local(dst) {
            return Ok(if dst.is_loopback() {
                Ipv4Addr::LOCALHOST
            } else {
                dst
            });
        }
        let (i, _, _) = self.route(dst, Ipv4Addr::ZERO)?;
        self.iface(i).map(|i| i.ip).ok_or(NetError::NotConfigured)
    }

    fn valid_local_bind(&self, ip: Ipv4Addr) -> bool {
        ip.is_unspecified() || self.is_local(ip)
    }

    // ===================================================================== TCP

    /// Listen on `local` (ip 0.0.0.0 = all; port 0 = ephemeral). `backlog` is clamped to 1..=64.
    pub fn tcp_listen(
        &mut self,
        local: SocketAddr,
        backlog: usize,
    ) -> Result<SocketHandle, NetError> {
        if !self.valid_local_bind(local.ip) {
            return Err(NetError::InvalidInput);
        }
        let port = if local.port == 0 {
            self.pick_port(|s, p| s.tcp_port_used(p))?
        } else {
            let clash = self.sockets.iter().any(|s| {
                matches!(&s.sock, Some(Sock::Tcp(t)) if t.tcb.state == TcpState::Listen
                    && t.tcb.local.port == local.port
                    && (t.tcb.local.ip == local.ip || t.tcb.local.ip.is_unspecified() || local.ip.is_unspecified()))
            });
            if clash {
                return Err(NetError::AddrInUse);
            }
            local.port
        };
        let tcb = Tcb::new_listen(SocketAddr::new(local.ip, port));
        self.alloc(Sock::Tcp(Box::new(TcpSock {
            tcb,
            accept_q: VecDeque::new(),
            backlog: backlog.clamp(1, MAX_BACKLOG),
        })))
    }

    /// Accept an established connection. `Ok(None)` if none is ready yet.
    pub fn tcp_accept(&mut self, listener: SocketHandle) -> Result<Option<SocketHandle>, NetError> {
        let q: Vec<SocketHandle> = {
            let l = self.tcp_ref(listener)?;
            if l.tcb.state != TcpState::Listen {
                return Err(NetError::InvalidInput);
            }
            l.accept_q.iter().copied().collect()
        };
        let mut pick = None;
        let mut live = Vec::with_capacity(q.len());
        for ch in q {
            // children that died before accept are dropped from the queue
            if let Some(Sock::Tcp(c)) = self.slot(ch) {
                if pick.is_none() && c.tcb.state != TcpState::SynReceived {
                    pick = Some(ch);
                } else {
                    live.push(ch);
                }
            }
        }
        if let Ok(l) = self.tcp_mut(listener) {
            l.accept_q = live.into();
        }
        if let Some(ch) = pick {
            if let Some(Sock::Tcp(c)) = self.slot_mut(ch) {
                c.tcb.user_open = true;
            }
        }
        Ok(pick)
    }

    /// Start connecting (returns immediately in SynSent). Errors: `InvalidInput`,
    /// `NoRoute`, `NotConfigured`, `NoSockets`.
    pub fn tcp_connect(
        &mut self,
        remote: SocketAddr,
        now_ms: i64,
    ) -> Result<SocketHandle, NetError> {
        self.tcp_connect_bound(SocketAddr::new(Ipv4Addr::ZERO, 0), remote, now_ms)
    }

    pub fn tcp_connect_bound(
        &mut self,
        bound: SocketAddr,
        remote: SocketAddr,
        now_ms: i64,
    ) -> Result<SocketHandle, NetError> {
        self.now = now_ms;
        if remote.port == 0
            || remote.ip.is_unspecified()
            || remote.ip.is_broadcast()
            || remote.ip.is_multicast()
        {
            return Err(NetError::InvalidInput);
        }
        if !bound.ip.is_unspecified() && !self.is_local(bound.ip) {
            return Err(NetError::InvalidInput);
        }
        let local_ip = self.select_src(bound.ip, remote.ip)?;
        if !self.is_local(remote.ip) {
            if let Ok((_, _, true)) = self.route(remote.ip, Ipv4Addr::ZERO) {
                return Err(NetError::InvalidInput); // subnet broadcast
            }
        }
        if self.sockets.len() >= MAX_SOCKETS && self.sockets.iter().all(|s| s.sock.is_some()) {
            return Err(NetError::NoSockets);
        }
        let port = if bound.port == 0 {
            self.pick_port(|s, p| s.tcp_port_used(p))?
        } else if self.tcp_port_used(bound.port) {
            return Err(NetError::AddrInUse);
        } else {
            bound.port
        };
        let local = SocketAddr::new(local_ip, port);
        let iss = self.gen_isn();
        let mut out = Vec::new();
        let tcb = Tcb::new_connect(local, remote, iss, now_ms, &mut out);
        let h = self.alloc(Sock::Tcp(Box::new(TcpSock {
            tcb,
            accept_q: VecDeque::new(),
            backlog: 0,
        })))?;
        self.tcp_transmit(h, local, remote, out);
        self.drain_loopback();
        Ok(h)
    }

    pub fn tcp_state(&self, h: SocketHandle) -> Result<TcpState, NetError> {
        Ok(self.tcp_ref(h)?.tcb.state)
    }

    /// Queue data. Returns bytes accepted; `WouldBlock` if the send buffer is full.
    /// Allowed in SynSent/SynReceived (queued), Established and CloseWait.
    pub fn tcp_send(
        &mut self,
        h: SocketHandle,
        data: &[u8],
        now_ms: i64,
    ) -> Result<usize, NetError> {
        self.now = now_ms;
        let t = self.tcp_mut(h)?;
        match t.tcb.state {
            TcpState::SynSent
            | TcpState::SynReceived
            | TcpState::Established
            | TcpState::CloseWait => {}
            TcpState::Closed => return Err(t.tcb.error.unwrap_or(NetError::NotConnected)),
            _ => return Err(NetError::NotConnected),
        }
        if data.is_empty() {
            return Ok(0);
        }
        let mut out = Vec::new();
        let n = t.tcb.write(data, now_ms, &mut out);
        let (l, r) = (t.tcb.local, t.tcb.remote);
        self.tcp_transmit(h, l, r, out);
        self.drain_loopback();
        if n == 0 {
            Err(NetError::WouldBlock)
        } else {
            Ok(n)
        }
    }

    /// Read. `Ok(0)` = EOF (peer sent FIN) or empty `buf`; `WouldBlock` = no data yet;
    /// other errors report why the connection died (`ConnectionReset`, `TimedOut`, ...).
    pub fn tcp_recv(&mut self, h: SocketHandle, buf: &mut [u8]) -> Result<usize, NetError> {
        let t = self.tcp_mut(h)?;
        if t.tcb.state == TcpState::Listen {
            return Err(NetError::NotConnected);
        }
        if buf.is_empty() {
            return Ok(0);
        }
        if t.tcb.rx_available() > 0 {
            let mut out = Vec::new();
            let n = t.tcb.read(buf, &mut out);
            let (l, r) = (t.tcb.local, t.tcb.remote);
            self.tcp_transmit(h, l, r, out);
            self.drain_loopback();
            return Ok(n);
        }
        if t.tcb.fin_received {
            return Ok(0);
        }
        if let Some(e) = t.tcb.error {
            return Err(e);
        }
        if t.tcb.state == TcpState::Closed {
            return Err(NetError::NotConnected);
        }
        Err(NetError::WouldBlock)
    }

    /// Half-close: FIN is sent after all queued data.
    pub fn tcp_shutdown_write(&mut self, h: SocketHandle, now_ms: i64) -> Result<(), NetError> {
        self.now = now_ms;
        let t = self.tcp_mut(h)?;
        match t.tcb.state {
            TcpState::Established | TcpState::CloseWait => {}
            TcpState::FinWait1
            | TcpState::FinWait2
            | TcpState::Closing
            | TcpState::LastAck
            | TcpState::TimeWait => return Ok(()),
            TcpState::Closed => return Err(t.tcb.error.unwrap_or(NetError::NotConnected)),
            _ => return Err(NetError::NotConnected),
        }
        let mut out = Vec::new();
        t.tcb.shutdown_write(now_ms, &mut out);
        let (l, r) = (t.tcb.local, t.tcb.remote);
        self.tcp_transmit(h, l, r, out);
        self.drain_loopback();
        Ok(())
    }

    /// Graceful close; the handle is invalid afterwards. Queued data is still
    /// delivered before FIN. Closing a listener resets its un-accepted connections.
    pub fn tcp_close(&mut self, h: SocketHandle, now_ms: i64) {
        self.now = now_ms;
        let Ok(t) = self.tcp_mut(h) else { return };
        if t.tcb.state == TcpState::Listen {
            self.close_listener(h);
            return;
        }
        let mut out = Vec::new();
        if t.tcb.close(now_ms, &mut out) {
            t.tcb.abort(&mut out);
        }
        let (l, r) = (t.tcb.local, t.tcb.remote);
        self.tcp_transmit(h, l, r, out);
        self.reap(h.idx as usize);
        self.drain_loopback();
    }

    /// Abort with RST; the handle is invalid afterwards.
    pub fn tcp_abort(&mut self, h: SocketHandle) {
        let Ok(t) = self.tcp_mut(h) else { return };
        if t.tcb.state == TcpState::Listen {
            self.close_listener(h);
            return;
        }
        let mut out = Vec::new();
        t.tcb.user_open = false;
        t.tcb.abort(&mut out);
        let (l, r) = (t.tcb.local, t.tcb.remote);
        self.tcp_transmit(h, l, r, out);
        self.reap(h.idx as usize);
        self.drain_loopback();
    }

    fn close_listener(&mut self, h: SocketHandle) {
        let q: Vec<SocketHandle> = match self.slot_mut(h) {
            Some(Sock::Tcp(t)) => t.accept_q.drain(..).collect(),
            _ => return,
        };
        for ch in q {
            if let Some(Sock::Tcp(c)) = self.slot_mut(ch) {
                if !c.tcb.user_open {
                    let mut out = Vec::new();
                    c.tcb.abort(&mut out);
                    let (l, r) = (c.tcb.local, c.tcb.remote);
                    self.tcp_transmit(ch, l, r, out);
                    self.reap(ch.idx as usize);
                }
            }
        }
        self.free(h.idx as usize);
        self.drain_loopback();
    }

    pub fn tcp_local_addr(&self, h: SocketHandle) -> Result<SocketAddr, NetError> {
        Ok(self.tcp_ref(h)?.tcb.local)
    }

    pub fn tcp_peer_addr(&self, h: SocketHandle) -> Result<SocketAddr, NetError> {
        let t = self.tcp_ref(h)?;
        if t.tcb.state == TcpState::Listen {
            return Err(NetError::NotConnected);
        }
        Ok(t.tcb.remote)
    }

    /// True if `tcp_recv` (or `tcp_accept` for a listener) would not return `WouldBlock`.
    /// Also true for a stale handle, so pollers wake up and see the error.
    pub fn tcp_can_read(&self, h: SocketHandle) -> bool {
        let Ok(t) = self.tcp_ref(h) else { return true };
        if t.tcb.state == TcpState::Listen {
            return t.accept_q.iter().any(|ch| matches!(self.slot(*ch), Some(Sock::Tcp(c)) if c.tcb.state != TcpState::SynReceived));
        }
        t.tcb.rx_available() > 0
            || t.tcb.fin_received
            || t.tcb.error.is_some()
            || t.tcb.state == TcpState::Closed
    }

    /// True if `tcp_send` would not return `WouldBlock` (including when it would error).
    pub fn tcp_can_write(&self, h: SocketHandle) -> bool {
        let Ok(t) = self.tcp_ref(h) else { return true };
        match t.tcb.state {
            TcpState::Established | TcpState::CloseWait => t.tcb.tx_space() > 0,
            TcpState::SynSent | TcpState::SynReceived | TcpState::Listen => false,
            _ => true,
        }
    }

    /// Bytes queued for sending but not yet acknowledged.
    pub fn tcp_tx_pending(&self, h: SocketHandle) -> Result<usize, NetError> {
        Ok(self.tcp_ref(h)?.tcb.tx_pending())
    }

    fn tcp_transmit(
        &mut self,
        h: SocketHandle,
        local: SocketAddr,
        remote: SocketAddr,
        segs: Vec<Seg>,
    ) {
        for s in segs {
            if s.flags & RST != 0 {
                self.stats.tcp_rst_sent += 1;
            }
            let b = tcp::build_segment(
                local.ip,
                remote.ip,
                local.port,
                remote.port,
                s.seq,
                s.ack,
                s.flags,
                s.window,
                s.mss,
                &s.payload,
            );
            let _ = self.send_ip(local.ip, remote.ip, PROTO_TCP, &b, Owner::Sock(h));
        }
    }

    fn tcp_timers(&mut self) {
        let now = self.now;
        for i in 0..self.sockets.len() {
            let gen = self.sockets[i].gen;
            let Some(Sock::Tcp(t)) = self.sockets[i].sock.as_mut() else {
                continue;
            };
            if !t.tcb.next_deadline().is_some_and(|d| d <= now) {
                continue;
            }
            let mut out = Vec::new();
            t.tcb.on_timer(now, &mut out);
            let (l, r) = (t.tcb.local, t.tcb.remote);
            self.tcp_transmit(SocketHandle { idx: i as u16, gen }, l, r, out);
            self.reap(i);
        }
    }

    fn rx_tcp(&mut self, ih: &Ipv4Header, seg: &[u8], ingress: Option<usize>) {
        let now = self.now;
        if !tcp::verify_checksum(&ih.src, &ih.dst, seg) {
            return self.count_bad(ingress, |s| s.tcp_rx_bad += 1);
        }
        let Some((th, payload)) = TcpHeader::parse(seg) else {
            return self.count_bad(ingress, |s| s.tcp_rx_bad += 1);
        };
        if self.local_unicast(ih.dst).is_none()
            || ih.src.is_broadcast()
            || ih.src.is_multicast()
            || ih.src.is_unspecified()
        {
            return;
        }
        let local = SocketAddr::new(ih.dst, th.dst_port);
        let remote = SocketAddr::new(ih.src, th.src_port);

        let mut conn = self.sockets.iter().position(|s| {
            matches!(&s.sock, Some(Sock::Tcp(t)) if t.tcb.state != TcpState::Listen && t.tcb.local == local && t.tcb.remote == remote)
        });
        // A new SYN for a TIME_WAIT 4-tuple with a higher sequence number reopens it.
        if let Some(i) = conn {
            if let Some(Sock::Tcp(t)) = &self.sockets[i].sock {
                if t.tcb.state == TcpState::TimeWait
                    && th.has(SYN)
                    && !th.has(ACK)
                    && seq_gt(th.seq_num, t.tcb.rcv_nxt())
                    && !t.tcb.user_open
                {
                    self.free(i);
                    conn = None;
                }
            }
        }
        if let Some(i) = conn {
            let gen = self.sockets[i].gen;
            let Some(Sock::Tcp(t)) = self.sockets[i].sock.as_mut() else {
                return;
            };
            let mut out = Vec::new();
            t.tcb.on_segment(&th, payload, now, &mut out);
            self.tcp_transmit(SocketHandle { idx: i as u16, gen }, local, remote, out);
            self.reap(i);
            return;
        }

        let listener = self
            .sockets
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match &s.sock {
                Some(Sock::Tcp(t))
                    if t.tcb.state == TcpState::Listen && t.tcb.local.port == local.port =>
                {
                    if t.tcb.local.ip == local.ip {
                        Some((0, i))
                    } else if t.tcb.local.ip.is_unspecified() {
                        Some((1, i))
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .min()
            .map(|(_, i)| i);
        match listener {
            Some(li) => self.listen_input(li, local, remote, &th),
            None if !th.has(RST) => self.send_rst_for(local, remote, &th, payload.len()),
            None => {}
        }
    }

    fn listen_input(&mut self, li: usize, local: SocketAddr, remote: SocketAddr, th: &TcpHeader) {
        if th.has(RST) {
            return;
        }
        if th.has(ACK) {
            return self.send_rst_for(local, remote, th, 0);
        }
        if !th.has(SYN) || th.has(FIN) {
            return;
        }
        let lh = SocketHandle {
            idx: li as u16,
            gen: self.sockets[li].gen,
        };
        // prune dead children, enforce backlog
        let q: Vec<SocketHandle> = match self.slot(lh) {
            Some(Sock::Tcp(t)) => t.accept_q.iter().copied().collect(),
            _ => return,
        };
        let live: VecDeque<SocketHandle> = q
            .into_iter()
            .filter(|c| matches!(self.slot(*c), Some(Sock::Tcp(_))))
            .collect();
        let full = match self.slot_mut(lh) {
            Some(Sock::Tcp(t)) => {
                t.accept_q = live;
                t.accept_q.len() >= t.backlog
            }
            _ => return,
        };
        if full {
            return; // drop the SYN; the peer retries
        }
        let iss = self.gen_isn();
        let mut out = Vec::new();
        let tcb = Tcb::new_passive(local, remote, iss, th, self.now, &mut out);
        let Ok(ch) = self.alloc(Sock::Tcp(Box::new(TcpSock {
            tcb,
            accept_q: VecDeque::new(),
            backlog: 0,
        }))) else {
            return;
        };
        if let Some(Sock::Tcp(t)) = self.slot_mut(lh) {
            t.accept_q.push_back(ch);
        }
        self.tcp_transmit(ch, local, remote, out);
    }

    fn send_rst_for(&mut self, local: SocketAddr, remote: SocketAddr, th: &TcpHeader, plen: usize) {
        let (seq, ack, flags) = if th.has(ACK) {
            (th.ack_num, 0, RST)
        } else {
            let len = plen as u32 + th.has(SYN) as u32 + th.has(FIN) as u32;
            (0, th.seq_num.wrapping_add(len), RST | ACK)
        };
        self.stats.tcp_rst_sent += 1;
        let b = tcp::build_segment(
            local.ip,
            remote.ip,
            local.port,
            remote.port,
            seq,
            ack,
            flags,
            0,
            None,
            &[],
        );
        let _ = self.send_ip(local.ip, remote.ip, PROTO_TCP, &b, Owner::None);
    }

    // ===================================================================== UDP

    /// Bind (ip 0.0.0.0 = all local addresses; port 0 = ephemeral).
    pub fn udp_bind(&mut self, local: SocketAddr) -> Result<SocketHandle, NetError> {
        if !self.valid_local_bind(local.ip) {
            return Err(NetError::InvalidInput);
        }
        let port = if local.port == 0 {
            self.pick_port(|s, p| s.udp_port_used(Ipv4Addr::ZERO, p))?
        } else if self.udp_port_used(local.ip, local.port) {
            return Err(NetError::AddrInUse);
        } else {
            local.port
        };
        self.alloc(Sock::Udp(UdpSock {
            local: SocketAddr::new(local.ip, port),
            rx: VecDeque::new(),
            error: None,
            dead: false,
        }))
    }

    /// Send one datagram. Queues behind ARP resolution. `MessageTooLong` above 1472 bytes.
    pub fn udp_send_to(
        &mut self,
        h: SocketHandle,
        dst: SocketAddr,
        data: &[u8],
        now_ms: i64,
    ) -> Result<usize, NetError> {
        self.now = now_ms;
        let local = match self.slot(h) {
            Some(Sock::Udp(u)) if u.dead => return Err(NetError::ConnectionAborted),
            Some(Sock::Udp(u)) => u.local,
            _ => return Err(NetError::BadHandle),
        };
        if data.len() > udp::MAX_PAYLOAD {
            return Err(NetError::MessageTooLong);
        }
        if dst.port == 0 || dst.ip.is_unspecified() {
            return Err(NetError::InvalidInput);
        }
        let src = self.select_src(local.ip, dst.ip)?;
        let dg = udp::build(src, dst.ip, local.port, dst.port, data);
        self.send_ip(src, dst.ip, PROTO_UDP, &dg, Owner::Sock(h))?;
        self.drain_loopback();
        Ok(data.len())
    }

    /// Receive one datagram (truncated to `buf`). `Ok(None)` if the queue is empty.
    /// A pending asynchronous error (e.g. `HostUnreachable`) is returned once.
    pub fn udp_recv_from(
        &mut self,
        h: SocketHandle,
        buf: &mut [u8],
    ) -> Result<Option<(SocketAddr, usize)>, NetError> {
        let Some(Sock::Udp(u)) = self.slot_mut(h) else {
            return Err(NetError::BadHandle);
        };
        if let Some((from, d)) = u.rx.pop_front() {
            let n = d.len().min(buf.len());
            buf[..n].copy_from_slice(&d[..n]);
            return Ok(Some((from, n)));
        }
        if let Some(e) = u.error.take() {
            return Err(e);
        }
        if u.dead {
            return Err(NetError::ConnectionAborted);
        }
        Ok(None)
    }

    pub fn udp_can_read(&self, h: SocketHandle) -> bool {
        match self.slot(h) {
            Some(Sock::Udp(u)) => !u.rx.is_empty() || u.error.is_some() || u.dead,
            _ => true,
        }
    }

    pub fn udp_local_addr(&self, h: SocketHandle) -> Result<SocketAddr, NetError> {
        match self.slot(h) {
            Some(Sock::Udp(u)) => Ok(u.local),
            _ => Err(NetError::BadHandle),
        }
    }

    pub fn udp_close(&mut self, h: SocketHandle) {
        if let Some(Sock::Udp(_)) = self.slot(h) {
            self.free(h.idx as usize);
        }
    }

    fn rx_udp(&mut self, ih: &Ipv4Header, seg: &[u8], ingress: Option<usize>, packet: &[u8]) {
        let Some((uh, data)) = UdpHeader::parse(seg) else {
            return self.count_bad(ingress, |s| s.udp_rx_bad += 1);
        };
        let dg = seg.get(..uh.length as usize).unwrap_or(seg);
        if !udp::verify_checksum(&ih.src, &ih.dst, dg) {
            return self.count_bad(ingress, |s| s.udp_rx_bad += 1);
        }
        let from = SocketAddr::new(ih.src, uh.src_port);
        if uh.src_port == 67 && uh.dst_port == 68 {
            if let (Some(i), Some(m)) = (ingress, crate::dhcp::Message::parse(data)) {
                if let Some(c) = self.dhcp_clients.get_mut(&i) {
                    if c.receive(&m, self.now) {
                        self.apply_dhcp_lease(i);
                    }
                    return;
                }
            }
        }
        let unicast = self.local_unicast(ih.dst).is_some();

        // DNS client queries
        if unicast {
            for d in self.dns.iter_mut() {
                let Some(q) = d.q.as_mut() else { continue };
                if q.status != DnsStatus::Pending || q.port != uh.dst_port {
                    continue;
                }
                if from.ip == q.server && from.port == dns::DNS_PORT {
                    match dns::parse_answer(data, q.txid, &q.name) {
                        Ok(ip) => q.status = DnsStatus::Resolved(ip),
                        Err(NetError::InvalidPacket) => {}
                        Err(e) => q.status = DnsStatus::Failed(e),
                    }
                }
                return;
            }
        }

        let mut best: Option<(u8, usize)> = None;
        for (i, s) in self.sockets.iter().enumerate() {
            if let Some(Sock::Udp(u)) = &s.sock {
                if u.dead || u.local.port != uh.dst_port {
                    continue;
                }
                let rank = if u.local.ip == ih.dst {
                    0
                } else if u.local.ip.is_unspecified() {
                    1
                } else {
                    continue;
                };
                if best.is_none_or(|(r, _)| rank < r) {
                    best = Some((rank, i));
                }
            }
        }
        match best {
            Some((_, i)) => {
                if let Some(Sock::Udp(u)) = self.sockets[i].sock.as_mut() {
                    if u.rx.len() >= UDP_QUEUE_LEN {
                        self.stats.udp_rx_queue_full += 1;
                    } else {
                        u.rx.push_back((from, data.to_vec()));
                    }
                }
            }
            None => {
                self.stats.udp_rx_no_socket += 1;
                if unicast {
                    self.icmp_error(packet, 3, 3);
                }
            }
        }
    }

    // ==================================================================== ICMP

    /// Open a raw ICMP socket. Every open socket receives a copy of each ICMP message
    /// addressed to us except echo requests (which the stack answers itself).
    pub fn icmp_open(&mut self) -> Result<SocketHandle, NetError> {
        self.alloc(Sock::Icmp(IcmpSock {
            rx: VecDeque::new(),
            error: None,
            ttl: 64,
        }))
    }
    pub fn icmp_set_ttl(&mut self, h: SocketHandle, ttl: u8) -> Result<(), NetError> {
        if ttl == 0 {
            return Err(NetError::InvalidInput);
        }
        let Some(Sock::Icmp(s)) = self.slot_mut(h) else {
            return Err(NetError::BadHandle);
        };
        s.ttl = ttl;
        Ok(())
    }

    /// Send a complete ICMP message (the caller fills in type/code/checksum).
    pub fn icmp_send(
        &mut self,
        h: SocketHandle,
        dst: Ipv4Addr,
        icmp_packet: &[u8],
        now_ms: i64,
    ) -> Result<usize, NetError> {
        self.now = now_ms;
        if !matches!(self.slot(h), Some(Sock::Icmp(_))) {
            return Err(NetError::BadHandle);
        }
        if icmp_packet.len() < IcmpPacket::HEADER_SIZE || dst.is_unspecified() {
            return Err(NetError::InvalidInput);
        }
        if icmp_packet.len() > MAX_PAYLOAD {
            return Err(NetError::MessageTooLong);
        }
        let src = self.select_src(Ipv4Addr::ZERO, dst)?;
        self.send_ip(src, dst, PROTO_ICMP, icmp_packet, Owner::Sock(h))?;
        self.drain_loopback();
        Ok(icmp_packet.len())
    }

    /// Receive one ICMP message (whole message, truncated to `buf`) and its source.
    pub fn icmp_recv(
        &mut self,
        h: SocketHandle,
        buf: &mut [u8],
    ) -> Result<Option<(Ipv4Addr, usize)>, NetError> {
        let Some(Sock::Icmp(s)) = self.slot_mut(h) else {
            return Err(NetError::BadHandle);
        };
        if let Some((from, d)) = s.rx.pop_front() {
            let n = d.len().min(buf.len());
            buf[..n].copy_from_slice(&d[..n]);
            return Ok(Some((from, n)));
        }
        if let Some(e) = s.error.take() {
            return Err(e);
        }
        Ok(None)
    }

    pub fn icmp_can_read(&self, h: SocketHandle) -> bool {
        match self.slot(h) {
            Some(Sock::Icmp(s)) => !s.rx.is_empty() || s.error.is_some(),
            _ => true,
        }
    }

    pub fn icmp_close(&mut self, h: SocketHandle) {
        if let Some(Sock::Icmp(_)) = self.slot(h) {
            self.free(h.idx as usize);
        }
    }

    fn rx_icmp(&mut self, ih: &Ipv4Header, payload: &[u8], ingress: Option<usize>) {
        let Some((p, body)) = IcmpPacket::parse(payload) else {
            return self.count_bad(ingress, |s| s.icmp_rx_bad += 1);
        };
        if p.icmp_type == ICMP_ECHO_REQUEST {
            if p.code == 0 && self.local_unicast(ih.dst).is_some() {
                let reply = IcmpPacket::build_echo(ICMP_ECHO_REPLY, p.id, p.seq, body);
                let _ = self.send_ip(ih.dst, ih.src, PROTO_ICMP, &reply, Owner::None);
            }
            return;
        }
        for s in self.sockets.iter_mut() {
            if let Some(Sock::Icmp(i)) = s.sock.as_mut() {
                if i.rx.len() < ICMP_QUEUE_LEN {
                    i.rx.push_back((ih.src, payload.to_vec()));
                }
            }
        }
    }

    // ===================================================================== DNS

    /// Start resolving `name` (A record) via the configured server. IPv4 literals and
    /// "localhost" resolve immediately. Errors: `InvalidInput` (bad name),
    /// `NotConfigured` (no DNS server), `NoSockets` (16 queries in flight).
    pub fn dns_query(&mut self, name: &str, now_ms: i64) -> Result<DnsHandle, NetError> {
        self.now = now_ms;
        let immediate = if let Some(ip) = Ipv4Addr::parse(name) {
            Some(ip)
        } else if name.eq_ignore_ascii_case("localhost") {
            Some(Ipv4Addr::LOCALHOST)
        } else {
            None
        };
        if immediate.is_none() {
            dns::encode_name_vec(name).ok_or(NetError::InvalidInput)?;
            if self.dns_server.is_unspecified() {
                return Err(NetError::NotConfigured);
            }
        }
        let idx = match self.dns.iter().position(|d| d.q.is_none()) {
            Some(i) => i,
            None if self.dns.len() < MAX_DNS_QUERIES => {
                self.dns.push(DnsSlot { gen: 1, q: None });
                self.dns.len() - 1
            }
            None => return Err(NetError::NoSockets),
        };
        let port = if immediate.is_some() {
            0
        } else {
            self.pick_port(|s, p| s.udp_port_used(Ipv4Addr::ZERO, p))?
        };
        let txid = self.rng.next_u32() as u16;
        let q = DnsQuery {
            name: name.to_string(),
            txid,
            port,
            server: self.dns_server,
            tries: 1,
            deadline: now_ms + DNS_TIMEOUT_MS,
            status: immediate.map_or(DnsStatus::Pending, DnsStatus::Resolved),
        };
        self.dns[idx].q = Some(q);
        let h = DnsHandle {
            idx: idx as u16,
            gen: self.dns[idx].gen,
        };
        if immediate.is_none() {
            self.dns_send(idx);
            self.drain_loopback();
        }
        Ok(h)
    }

    /// Poll a query. Non-`Pending` results free the handle (a second poll → `Failed(BadHandle)`).
    pub fn dns_poll(&mut self, q: DnsHandle) -> DnsStatus {
        let Some(slot) = self.dns.get_mut(q.idx as usize) else {
            return DnsStatus::Failed(NetError::BadHandle);
        };
        if slot.gen != q.gen {
            return DnsStatus::Failed(NetError::BadHandle);
        }
        let Some(query) = slot.q.as_ref() else {
            return DnsStatus::Failed(NetError::BadHandle);
        };
        let st = query.status;
        if st != DnsStatus::Pending {
            slot.q = None;
            slot.gen = next_gen(slot.gen);
        }
        st
    }

    /// Abandon a query (e.g. the requesting process exited).
    pub fn dns_cancel(&mut self, q: DnsHandle) {
        if let Some(slot) = self.dns.get_mut(q.idx as usize) {
            if slot.gen == q.gen && slot.q.is_some() {
                slot.q = None;
                slot.gen = next_gen(slot.gen);
            }
        }
    }

    fn dns_send(&mut self, idx: usize) {
        let Some(slot) = self.dns.get(idx) else {
            return;
        };
        let Some(q) = slot.q.as_ref() else { return };
        let h = DnsHandle {
            idx: idx as u16,
            gen: slot.gen,
        };
        let (server, port) = (q.server, q.port);
        let res = dns::build_query(&q.name, q.txid)
            .ok_or(NetError::InvalidInput)
            .and_then(|pkt| {
                let src = self.select_src(Ipv4Addr::ZERO, server)?;
                let dg = udp::build(src, server, port, dns::DNS_PORT, &pkt);
                self.send_ip(src, server, PROTO_UDP, &dg, Owner::Dns(h))
            });
        if let Err(e) = res {
            self.dns_fail(h, e);
        }
    }

    fn dns_fail(&mut self, h: DnsHandle, e: NetError) {
        if let Some(slot) = self.dns.get_mut(h.idx as usize) {
            if slot.gen == h.gen {
                if let Some(q) = slot.q.as_mut() {
                    if q.status == DnsStatus::Pending {
                        q.status = DnsStatus::Failed(e);
                    }
                }
            }
        }
    }

    fn dns_timers(&mut self) {
        let now = self.now;
        for i in 0..self.dns.len() {
            let resend = {
                let Some(q) = self.dns[i].q.as_mut() else {
                    continue;
                };
                if q.status != DnsStatus::Pending || now < q.deadline {
                    continue;
                }
                if q.tries >= DNS_TRIES {
                    q.status = DnsStatus::Failed(NetError::TimedOut);
                    false
                } else {
                    q.tries += 1;
                    q.deadline = now + DNS_TIMEOUT_MS;
                    true
                }
            };
            if resend {
                let txid = self.rng.next_u32() as u16;
                if let Some(q) = self.dns[i].q.as_mut() {
                    q.txid = txid;
                }
                self.dns_send(i);
            }
        }
    }

    // ============================================================ IPv4 / ARP

    fn next_id(&mut self) -> u16 {
        self.ip_id = self.ip_id.wrapping_add(1);
        self.ip_id
    }

    /// Interface that owns `ip` (configured).
    fn local_iface(&self, ip: Ipv4Addr) -> Option<usize> {
        if ip.is_unspecified() {
            return None;
        }
        self.ifaces
            .iter()
            .position(|i| i.as_ref().is_some_and(|i| i.ip == ip))
    }

    /// One of our unicast addresses (including 127/8)?
    fn local_unicast(&self, ip: Ipv4Addr) -> Option<()> {
        (ip.is_loopback() || self.local_iface(ip).is_some()).then_some(())
    }

    fn is_local(&self, ip: Ipv4Addr) -> bool {
        self.local_unicast(ip).is_some()
    }

    fn usable(&self, idx: usize) -> bool {
        self.iface(idx)
            .is_some_and(|i| i.is_up() && i.is_configured())
    }

    /// Route lookup → (iface, next hop, is_broadcast).
    fn route(
        &self,
        dst: Ipv4Addr,
        src_hint: Ipv4Addr,
    ) -> Result<(usize, Ipv4Addr, bool), NetError> {
        if dst.is_broadcast() {
            let i = self
                .local_iface(src_hint)
                .filter(|&i| self.usable(i))
                .or_else(|| (0..self.ifaces.len()).find(|&i| self.usable(i)))
                .ok_or(NetError::NoRoute)?;
            return Ok((i, dst, true));
        }
        if dst.is_multicast() || dst.is_unspecified() {
            return Err(NetError::NoRoute);
        }
        let r = self
            .routes
            .iter()
            .filter(|r| self.usable(r.iface) && r.dst.same_subnet_prefix(&dst, r.prefix))
            .fold(None::<&Route>, |b, r| match b {
                Some(b)
                    if b.prefix > r.prefix
                        || (b.prefix == r.prefix
                            && (b.distance, b.metric) <= (r.distance, r.metric)) =>
                {
                    Some(b)
                }
                _ => Some(r),
            })
            .ok_or(NetError::NoRoute)?;
        if r.gateway.is_unspecified() {
            let ifc = self.iface(r.iface).ok_or(NetError::NoRoute)?;
            let bcast = ifc.prefix < 31 && dst == ifc.ip.broadcast_addr(ifc.prefix);
            Ok((r.iface, dst, bcast))
        } else {
            Ok((r.iface, r.gateway, false))
        }
    }

    /// Build and send an IPv4 packet (loopback, broadcast or via ARP).
    fn packet_ttl(&self, pkt: &mut [u8], owner: Owner) {
        if let Owner::Sock(h) = owner {
            if let Some(Sock::Icmp(s)) = self.slot(h) {
                pkt[8] = s.ttl;
                pkt[10..12].fill(0);
                let sum = crate::checksum::internet_checksum(&pkt[..20]);
                pkt[10..12].copy_from_slice(&sum.to_be_bytes());
            }
        }
    }
    fn send_ip(
        &mut self,
        src: Ipv4Addr,
        dst: Ipv4Addr,
        proto: u8,
        payload: &[u8],
        owner: Owner,
    ) -> Result<(), NetError> {
        if payload.len() > MAX_PAYLOAD {
            return Err(NetError::MessageTooLong);
        }
        if self.is_local(dst) {
            let src = if src.is_unspecified() {
                if dst.is_loopback() {
                    Ipv4Addr::LOCALHOST
                } else {
                    dst
                }
            } else {
                src
            };
            let id = self.next_id();
            let mut pkt =
                build_packet(src, dst, proto, id, payload).ok_or(NetError::MessageTooLong)?;
            self.packet_ttl(&mut pkt, owner);
            if self.loopback.len() >= LOOPBACK_QUEUE_LEN {
                self.stats.loopback_dropped += 1;
            } else {
                self.loopback.push_back(pkt);
            }
            return Ok(());
        }
        if src.is_loopback() {
            return Err(NetError::NoRoute);
        }
        let (iface, nh, bcast) = self.route(dst, src)?;
        let src = if src.is_unspecified() {
            self.iface(iface).map_or(Ipv4Addr::ZERO, |i| i.ip)
        } else {
            src
        };
        let id = self.next_id();
        let mut pkt = build_packet(src, dst, proto, id, payload).ok_or(NetError::MessageTooLong)?;
        self.packet_ttl(&mut pkt, owner);
        if bcast {
            self.emit(iface, MacAddr::BROADCAST, ETHERTYPE_IPV4, &pkt);
        } else {
            self.resolve_and_send(iface, nh, pkt, owner);
        }
        Ok(())
    }

    fn resolve_and_send(&mut self, iface: usize, nh: Ipv4Addr, pkt: Vec<u8>, owner: Owner) {
        let now = self.now;
        let Some(ifc) = self.iface_mut(iface) else {
            return;
        };
        let (mac, send_req) = if let Some(n) = ifc.neighbors.iter_mut().find(|n| n.ip == nh) {
            n.last_used = now;
            match n.state {
                NeighborState::Incomplete => {
                    if n.pending.len() < ARP_PENDING_PER_NEIGHBOR {
                        n.pending.push_back((pkt, owner));
                    } else {
                        ifc.stats.tx_dropped += 1;
                    }
                    return;
                }
                NeighborState::Reachable if now < n.deadline => (n.mac, false),
                NeighborState::Reachable | NeighborState::Stale => {
                    n.state = NeighborState::Probe;
                    n.requests = 1;
                    n.deadline = now + ARP_RETRY_MS;
                    (n.mac, true)
                }
                NeighborState::Probe => (n.mac, false),
            }
        } else {
            if ifc.neighbors.len() >= MAX_NEIGHBORS {
                let victim = ifc
                    .neighbors
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| n.state != NeighborState::Incomplete)
                    .min_by_key(|(_, n)| n.last_used)
                    .map(|(i, _)| i);
                match victim {
                    Some(v) => {
                        ifc.neighbors.swap_remove(v);
                    }
                    None => {
                        ifc.stats.tx_dropped += 1;
                        return;
                    }
                }
            }
            let mut pending = VecDeque::new();
            pending.push_back((pkt, owner));
            ifc.neighbors.push(Neighbor {
                ip: nh,
                mac: MacAddr::ZERO,
                state: NeighborState::Incomplete,
                deadline: now + ARP_RETRY_MS,
                requests: 1,
                pending,
                last_used: now,
            });
            self.send_arp_request(iface, nh);
            return;
        };
        if send_req {
            self.send_arp_request(iface, nh);
        }
        self.emit(iface, mac, ETHERTYPE_IPV4, &pkt);
    }

    fn send_arp_request(&mut self, iface: usize, target: Ipv4Addr) {
        let Some(ifc) = self.iface(iface) else { return };
        let p = ArpPacket {
            operation: ARP_REQUEST,
            sender_mac: ifc.mac,
            sender_ip: ifc.ip,
            target_mac: MacAddr::ZERO,
            target_ip: target,
        };
        self.emit(iface, MacAddr::BROADCAST, ETHERTYPE_ARP, &p.to_bytes());
    }

    fn send_gratuitous_arp(&mut self, iface: usize) {
        let Some(ifc) = self.iface(iface) else { return };
        if ifc.is_configured() {
            let ip = ifc.ip;
            self.send_arp_request(iface, ip);
        }
    }

    fn emit(&mut self, iface: usize, dst: MacAddr, ethertype: u16, payload: &[u8]) {
        let Some(ifc) = self.ifaces.get_mut(iface).and_then(Option::as_mut) else {
            return;
        };
        if !ifc.is_up() || self.tx.len() >= TX_QUEUE_LEN {
            ifc.stats.tx_dropped += 1;
            return;
        }
        let Some(frame) = build_frame(dst, ifc.mac, ifc.vlan.map(VlanTag::new), ethertype, payload)
        else {
            ifc.stats.tx_dropped += 1;
            return;
        };
        ifc.stats.tx_packets += 1;
        ifc.stats.tx_bytes += frame.len() as u64;
        self.tx.push_back((iface, frame));
    }

    fn rx_arp(&mut self, iface: usize, payload: &[u8]) {
        let Some(ifc) = self.iface_mut(iface) else {
            return;
        };
        let Some(p) = ArpPacket::parse(payload) else {
            ifc.stats.rx_errors += 1;
            return;
        };
        let (my_ip, my_mac) = (ifc.ip, ifc.mac);
        if my_ip.is_unspecified()
            || !p.sender_mac.is_unicast()
            || p.sender_ip.is_unspecified()
            || p.sender_ip.is_broadcast()
            || p.sender_ip.is_multicast()
            || p.sender_ip.is_loopback()
            || p.sender_ip == my_ip
        {
            return;
        }
        let for_us = p.target_ip == my_ip;
        match p.operation {
            ARP_REQUEST => {
                // Learn (create) only from requests for our IP; others (incl. gratuitous) only refresh.
                self.learn(iface, p.sender_ip, p.sender_mac, for_us);
                if for_us {
                    let r = ArpPacket {
                        operation: ARP_REPLY,
                        sender_mac: my_mac,
                        sender_ip: my_ip,
                        target_mac: p.sender_mac,
                        target_ip: p.sender_ip,
                    };
                    self.emit(iface, p.sender_mac, ETHERTYPE_ARP, &r.to_bytes());
                }
            }
            ARP_REPLY => self.learn(iface, p.sender_ip, p.sender_mac, for_us),
            _ => {}
        }
    }

    fn learn(&mut self, iface: usize, ip: Ipv4Addr, mac: MacAddr, create: bool) {
        let now = self.now;
        let Some(ifc) = self.iface_mut(iface) else {
            return;
        };
        let pending = if let Some(n) = ifc.neighbors.iter_mut().find(|n| n.ip == ip) {
            n.mac = mac;
            n.state = NeighborState::Reachable;
            n.deadline = now + ARP_REACHABLE_MS;
            n.requests = 0;
            std::mem::take(&mut n.pending)
        } else if create {
            if ifc.neighbors.len() >= MAX_NEIGHBORS {
                let victim = ifc
                    .neighbors
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| n.state != NeighborState::Incomplete)
                    .min_by_key(|(_, n)| n.last_used)
                    .map(|(i, _)| i);
                match victim {
                    Some(v) => {
                        ifc.neighbors.swap_remove(v);
                    }
                    None => return,
                }
            }
            ifc.neighbors.push(Neighbor {
                ip,
                mac,
                state: NeighborState::Reachable,
                deadline: now + ARP_REACHABLE_MS,
                requests: 0,
                pending: VecDeque::new(),
                last_used: now,
            });
            VecDeque::new()
        } else {
            return;
        };
        for (pkt, _) in pending {
            self.emit(iface, mac, ETHERTYPE_IPV4, &pkt);
        }
    }

    fn neighbor_timers(&mut self) {
        let now = self.now;
        for i in 0..self.ifaces.len() {
            let mut requests = Vec::new();
            let mut failed: Vec<(Vec<u8>, Owner)> = Vec::new();
            let mut nfail = 0u64;
            {
                let Some(ifc) = self.iface_mut(i) else {
                    continue;
                };
                ifc.neighbors.retain_mut(|n| match n.state {
                    NeighborState::Incomplete | NeighborState::Probe if now >= n.deadline => {
                        if n.requests >= ARP_MAX_REQUESTS {
                            failed.extend(n.pending.drain(..));
                            nfail += 1;
                            false
                        } else {
                            n.requests += 1;
                            n.deadline = now + ARP_RETRY_MS;
                            requests.push(n.ip);
                            true
                        }
                    }
                    NeighborState::Reachable if now >= n.deadline => {
                        n.state = NeighborState::Stale;
                        true
                    }
                    _ => true,
                });
                ifc.stats.tx_dropped += failed.len() as u64;
            }
            self.stats.arp_failures += nfail;
            for ip in requests {
                self.send_arp_request(i, ip);
            }
            for (pkt, owner) in failed {
                if self.forwarding
                    && Ipv4Header::parse(&pkt).is_some_and(|(h, _)| !self.is_local(h.src))
                {
                    self.icmp_error(&pkt, 3, 1);
                }
                self.notify_unreachable(owner);
            }
        }
    }

    fn notify_unreachable(&mut self, owner: Owner) {
        match owner {
            Owner::None => {}
            Owner::Dns(h) => self.dns_fail(h, NetError::HostUnreachable),
            Owner::Sock(h) => {
                match self.slot_mut(h) {
                    Some(Sock::Tcp(t)) => t.tcb.on_unreachable(),
                    Some(Sock::Udp(u)) => u.error = Some(NetError::HostUnreachable),
                    Some(Sock::Icmp(s)) => s.error = Some(NetError::HostUnreachable),
                    None => {}
                }
                self.reap(h.idx as usize);
            }
        }
    }

    fn count_bad(&mut self, ingress: Option<usize>, f: impl FnOnce(&mut StackStats)) {
        f(&mut self.stats);
        if let Some(ifc) = ingress.and_then(|i| self.iface_mut(i)) {
            ifc.stats.rx_errors += 1;
        }
    }

    fn non_forwardable(&self, ip: Ipv4Addr) -> bool {
        ip.is_unspecified()
            || ip.is_loopback()
            || ip.is_multicast()
            || ip.is_broadcast()
            || ip.0[0] == 0
            || ip.0[0] >= 240
            || ip.0[..2] == [169, 254]
            || self
                .ifaces
                .iter()
                .flatten()
                .any(|i| i.is_configured() && i.prefix < 31 && ip == i.ip.broadcast_addr(i.prefix))
    }

    fn icmp_error(&mut self, pkt: &[u8], kind: u8, code: u8) {
        let Some((h, payload)) = Ipv4Header::parse(pkt) else {
            return;
        };
        // RFC 1812: never answer an error with an error, or answer broadcasts,
        // non-initial fragments, or invalid sources. Global 10/s bound.
        if self.now < self.icmp_error_next
            || self.non_forwardable(h.src)
            || self.non_forwardable(h.dst)
            || h.flags_fragment & 0x1fff != 0
            || (h.protocol == PROTO_ICMP
                && payload
                    .first()
                    .is_some_and(|t| matches!(t, 3 | 4 | 5 | 11 | 12)))
        {
            return;
        }
        let Ok(src) = self.select_src(Ipv4Addr::ZERO, h.src) else {
            return;
        };
        let mut error = vec![kind, code, 0, 0, 0, 0, 0, 0];
        error.extend_from_slice(&pkt[..(h.header_len() + 8).min(h.total_length as usize)]);
        let sum = crate::checksum::internet_checksum(&error);
        error[2..4].copy_from_slice(&sum.to_be_bytes());
        self.icmp_error_next = self.now.saturating_add(100);
        let _ = self.send_ip(src, h.src, PROTO_ICMP, &error, Owner::None);
    }

    /// Forward a packet already validated and translated by the router's NAT.
    /// Hairpin SNAT uses our own outside address, which normal receive correctly
    /// rejects as a spoofed source. This entry point never delivers local traffic.
    pub fn forward_translated_packet(&mut self, pkt: &[u8]) {
        if let Ok((h, _)) = Ipv4Header::parse_checked(pkt) {
            if self.forwarding && self.local_iface(h.dst).is_none() {
                self.forward_packet(pkt, &h);
            }
        }
    }

    fn forward_packet(&mut self, pkt: &[u8], h: &Ipv4Header) {
        if self.non_forwardable(h.src) || self.non_forwardable(h.dst) {
            return;
        }
        if h.ttl <= 1 {
            self.icmp_error(pkt, 11, 0);
            return;
        }
        let Ok((iface, hop, broadcast)) = self.route(h.dst, Ipv4Addr::ZERO) else {
            self.icmp_error(pkt, 3, 0);
            return;
        };
        if broadcast {
            return;
        }
        if h.total_length as usize > 1500 {
            self.icmp_error(pkt, 3, 4);
            return;
        }
        let mut forwarded = pkt[..h.total_length as usize].to_vec();
        forwarded[8] -= 1;
        forwarded[10..12].fill(0);
        let sum = crate::checksum::internet_checksum(&forwarded[..h.header_len()]);
        forwarded[10..12].copy_from_slice(&sum.to_be_bytes());
        self.resolve_and_send(iface, hop, forwarded, Owner::None);
    }

    fn rx_ip(&mut self, ingress: Option<usize>, pkt: &[u8]) {
        let (ih, payload) = match Ipv4Header::parse_checked(pkt) {
            Ok(v) => v,
            Err(_) => return self.count_bad(ingress, |s| s.ip_rx_bad += 1),
        };
        if ih.is_fragment() {
            self.stats.ip_rx_fragments_dropped += 1;
            if let Some(ifc) = ingress.and_then(|i| self.iface_mut(i)) {
                ifc.stats.rx_dropped += 1;
            }
            return;
        }
        if let Some(i) = ingress {
            let Some(ifc) = self.iface(i) else { return };
            let dst_ok = self.local_iface(ih.dst).is_some()
                || ih.dst.is_broadcast()
                || (ifc.is_configured()
                    && ifc.prefix < 31
                    && ih.dst == ifc.ip.broadcast_addr(ifc.prefix));
            let martian =
                ih.src.is_loopback() || ih.dst.is_loopback() || self.local_iface(ih.src).is_some();
            if !dst_ok && !martian && self.forwarding {
                self.forward_packet(pkt, &ih);
                return;
            }
            if !dst_ok || martian {
                self.stats.ip_rx_not_for_us += 1;
                if let Some(ifc) = self.iface_mut(i) {
                    ifc.stats.rx_dropped += 1;
                }
                return;
            }
        }
        match ih.protocol {
            PROTO_ICMP => self.rx_icmp(&ih, payload, ingress),
            PROTO_UDP => self.rx_udp(&ih, payload, ingress, pkt),
            PROTO_TCP => self.rx_tcp(&ih, payload, ingress),
            _ => {
                if let Some(ifc) = ingress.and_then(|i| self.iface_mut(i)) {
                    ifc.stats.rx_dropped += 1;
                }
            }
        }
    }

    /// Deliver queued loopback packets (bounded work per call; leftovers make
    /// `poll` report an immediate deadline).
    fn drain_loopback(&mut self) {
        for _ in 0..(4 * LOOPBACK_QUEUE_LEN) {
            let Some(p) = self.loopback.pop_front() else {
                return;
            };
            self.rx_ip(None, &p);
        }
    }
}
