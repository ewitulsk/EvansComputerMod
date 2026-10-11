//! Sans-IO router configuration, DHCP leases and NAPT. No host calls or clocks.
pub mod cli;
pub mod dhcp;
pub mod nat;

use ecm_net::{Ipv4Addr, Stack};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NatRole {
    Inside,
    Outside,
}

#[derive(Clone, Debug, Default)]
pub struct Port {
    pub address: Option<(Ipv4Addr, u8)>,
    pub dhcp: bool,
    pub nat: Option<NatRole>,
}
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub routing: bool,
    pub ports: BTreeMap<String, Port>,
    pub routes: Vec<(Ipv4Addr, u8, Ipv4Addr, String)>,
    pub pools: BTreeMap<String, dhcp::Pool>,
    pub forwards: Vec<nat::Forward>,
    pub bgp: ecm_bgp::Config,
    pub policy: ecm_bgp::Policy,
}
impl Config {
    pub fn apply(&self, stack: &mut Stack, now: i64) {
        stack.set_forwarding(self.routing);
        for (name, p) in &self.ports {
            if let Some(i) = stack.find_iface(name) {
                if p.dhcp {
                    if !stack.dhcp_enabled(i) {
                        let _ = stack.start_dhcp(i, now);
                    }
                } else if let Some((ip, prefix)) = p.address {
                    stack.stop_dhcp(i);
                    if !stack
                        .iface(i)
                        .is_some_and(|f| f.ip == ip && f.prefix == prefix)
                    {
                        stack.configure_addr(i, ip, prefix, now);
                    }
                }
            }
        }
        stack.remove_protocol_routes(ecm_net::RouteSource::Static);
        for (dst, prefix, gw, name) in &self.routes {
            let iface = stack
                .find_iface(name)
                .or_else(|| stack.lookup_route(*gw).map(|r| r.0));
            if let Some(i) = iface {
                let _ = stack.add_route(*dst, *prefix, *gw, i);
            }
        }
    }
    pub fn render(&self) -> String {
        let mut s = String::from("configure terminal\n");
        if self.routing {
            s.push_str("ip routing\n");
        }
        for (name, p) in &self.ports {
            s.push_str(&format!("interface {}\nrouting\n", name));
            if p.dhcp {
                s.push_str("ip dhcp\n");
            }
            if let Some((ip, prefix)) = p.address {
                s.push_str(&format!("ip address {}/{}\n", ip, prefix));
            }
            if let Some(role) = p.nat {
                s.push_str(if role == NatRole::Inside {
                    "ip nat inside\n"
                } else {
                    "ip nat outside\n"
                });
            }
            s.push_str("exit\n");
        }
        for (dst, p, gw, dev) in &self.routes {
            s.push_str(&format!("ip route {}/{} {} {}\n", dst, p, gw, dev));
        }
        for f in &self.forwards {
            s.push_str(&format!(
                "ip nat inside source static {} {} {} {}\n",
                if f.protocol == 6 { "tcp" } else { "udp" },
                f.inside,
                f.inside_port,
                f.outside_port
            ));
        }
        s.push_str("dhcp-server vrf default\n");
        for (name, p) in &self.pools {
            s.push_str(&format!(
                "pool {}\nrange {} {}\ndefault-router {}\ndns-server {}\nlease {}\n",
                name, p.start, p.end, p.router, p.dns, p.lease_secs
            ));
            if p.enabled {
                s.push_str("enable\n");
            }
            s.push_str("exit\n");
        }
        s.push_str("exit\n");
        for (name, rules) in &self.policy.lists {
            for r in rules {
                s.push_str(&format!(
                    "ip prefix-list {} seq {} {} {}/{} ge {} le {}\n",
                    name,
                    r.seq,
                    if r.permit { "permit" } else { "deny" },
                    r.prefix.address,
                    r.prefix.len,
                    r.min,
                    r.max
                ));
            }
        }
        for (name, rules) in &self.policy.maps {
            for r in rules {
                s.push_str(&format!(
                    "route-map {} {} {}\n",
                    name,
                    if r.permit { "permit" } else { "deny" },
                    r.seq
                ));
                if let Some(n) = &r.prefix_list {
                    s.push_str(&format!("match ip address prefix-list {}\n", n));
                }
                if let Some(n) = r.local_pref {
                    s.push_str(&format!("set local-preference {}\n", n));
                }
                if let Some(n) = r.med {
                    s.push_str(&format!("set metric {}\n", n));
                }
                if let Some(n) = r.community {
                    s.push_str(&format!("set community {}\n", n));
                }
                s.push_str("exit\n");
            }
        }
        if self.bgp.asn != 0 {
            s.push_str(&format!(
                "router bgp {}\nbgp router-id {}\ntimers bgp {} {}\n",
                self.bgp.asn, self.bgp.router_id, self.bgp.keepalive, self.bgp.hold
            ));
            if let Some(n) = self.bgp.listen_limit {
                s.push_str(&format!("bgp listen limit {}\n", n));
            }
            // Session settings: groups first (members and ranges refer to them).
            for (name, g) in &self.bgp.groups {
                s.push_str(&format!("neighbor {} peer-group\n", name));
                if let Some(r) = g.remote_as {
                    s.push_str(&format!("neighbor {} remote-as {}\n", name, r.render()));
                }
                if let Some(source) = &g.update_source {
                    s.push_str(&format!("neighbor {} update-source {}\n", name, source));
                }
                for r in self.bgp.listen.iter().filter(|r| &r.group == name) {
                    s.push_str(&format!(
                        "neighbor {} listen ip-range {}/{}",
                        name, r.prefix.address, r.prefix.len
                    ));
                    if let Some(a) = &r.as_range {
                        s.push_str(&format!(" as-range {}", ecm_bgp::render_as_range(a)));
                    }
                    if let Some(l) = r.limit {
                        s.push_str(&format!(" limit {}", l));
                    }
                    s.push('\n');
                }
            }
            for (ip, n) in &self.bgp.neighbors {
                if let Some(r) = n.remote_as {
                    s.push_str(&format!("neighbor {} remote-as {}\n", ip, r.render()));
                }
                if let Some(g) = &n.peer_group {
                    s.push_str(&format!("neighbor {} peer-group {}\n", ip, g));
                }
                if let Some(source) = &n.update_source {
                    s.push_str(&format!("neighbor {} update-source {}\n", ip, source));
                }
            }
            s.push_str("address-family ipv4 unicast\n");
            let family = |s: &mut String, who: &str, n: &ecm_bgp::Neighbor| {
                if n.active {
                    s.push_str(&format!("neighbor {} activate\n", who));
                }
                if n.default_originate {
                    s.push_str(&format!("neighbor {} default-originate\n", who));
                }
                if let Some(map) = &n.inbound {
                    s.push_str(&format!("neighbor {} route-map {} in\n", who, map));
                }
                if let Some(map) = &n.outbound {
                    s.push_str(&format!("neighbor {} route-map {} out\n", who, map));
                }
                if let Some(m) = &n.maximum_prefix {
                    s.push_str(&format!("neighbor {} {}\n", who, m.render()));
                }
            };
            for (name, g) in &self.bgp.groups {
                family(&mut s, name, g);
            }
            for (ip, n) in &self.bgp.neighbors {
                family(&mut s, &ip.to_string(), n);
            }
            for p in &self.bgp.networks {
                s.push_str(&format!("network {}/{}\n", p.address, p.len));
            }
            if self.bgp.connected {
                s.push_str("redistribute connected\n");
            }
            if self.bgp.static_routes {
                s.push_str("redistribute static\n");
            }
        }
        s.push_str("end\n");
        s
    }
}

pub struct Router {
    pub config: Config,
    pub nat: nat::Nat,
    pub dhcp: dhcp::Server,
}
impl Router {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            nat: nat::Nat::new(),
            dhcp: dhcp::Server::new(),
        }
    }
    pub fn ingress(
        &mut self,
        stack: &mut Stack,
        iface: usize,
        frame: &[u8],
        now: i64,
    ) -> Option<Vec<u8>> {
        use ecm_net::{
            eth::{EthHeader, ETHERTYPE_IPV4},
            ipv4::Ipv4Header,
            udp::UdpHeader,
        };
        let (eth, packet) = EthHeader::parse(frame)?;
        if eth.ethertype != ETHERTYPE_IPV4 {
            return Some(frame.to_vec());
        }
        let (ip, seg) = Ipv4Header::parse(packet)?;
        let port = stack.iface(iface)?;
        let name = port.name.clone();
        let address = port.ip;
        if ip.protocol == 17 {
            if let Some((udp, data)) = UdpHeader::parse(seg) {
                if udp.src_port == 68
                    && udp.dst_port == 67
                    && ecm_net::udp::verify_checksum(&ip.src, &ip.dst, &seg[..udp.length as usize])
                {
                    if let Some(msg) = ecm_net::dhcp::Message::parse(data) {
                        if let Some(reply) =
                            self.dhcp.receive(&self.config.pools, &msg, address, now)
                        {
                            let _ = stack.send_udp_on_interface(
                                iface,
                                address,
                                Ipv4Addr::BROADCAST,
                                67,
                                68,
                                &reply.encode(),
                                now,
                            );
                            return None;
                        }
                    }
                }
            }
        }
        let role = self.config.ports.get(&name).and_then(|p| p.nat);
        let mut translated = packet.to_vec();
        if role == Some(NatRole::Inside) {
            let outside = self
                .config
                .ports
                .iter()
                .filter(|(_, p)| p.nat == Some(NatRole::Outside))
                .filter_map(|(name, _)| stack.find_iface(name).and_then(|i| stack.iface(i)))
                .find(|f| f.ip == ip.dst)
                .map(|f| f.ip);
            if let Some(outside) = outside {
                if self
                    .nat
                    .inbound(&mut translated, outside, &self.config.forwards, now)
                    .is_ok()
                {
                    self.nat
                        .outbound(&mut translated, outside, &self.config.forwards, now)
                        .ok()?;
                    if ip.src.is_loopback()
                        || (0..32)
                            .any(|i| stack.iface(i).is_some_and(|f| f.ip == ip.src))
                    {
                        return None;
                    }
                    stack.forward_translated_packet(&translated);
                    return None;
                }
            }
        }
        if role == Some(NatRole::Outside) {
            if ip.dst == address
                && self
                    .nat
                    .inbound(&mut translated, address, &self.config.forwards, now)
                    .is_err()
            {
                // Local services (BGP, SSH) still go to the host stack.
                translated = packet.to_vec();
            }
        }
        ecm_net::eth::build_frame(eth.dst, eth.src, eth.vlan_tag, ETHERTYPE_IPV4, &translated)
    }
    pub fn egress(
        &mut self,
        stack: &Stack,
        iface: usize,
        frame: &[u8],
        now: i64,
    ) -> Option<Vec<u8>> {
        use ecm_net::{
            eth::{EthHeader, ETHERTYPE_IPV4},
            ipv4::Ipv4Header,
        };
        let f = stack.iface(iface)?;
        if self.config.ports.get(&f.name).and_then(|p| p.nat) != Some(NatRole::Outside) {
            return Some(frame.to_vec());
        }
        let (eth, pkt) = EthHeader::parse(frame)?;
        if eth.ethertype != ETHERTYPE_IPV4 {
            return Some(frame.to_vec());
        }
        let (h, _) = Ipv4Header::parse(pkt)?;
        let from_inside = stack
            .lookup_route(h.src)
            .and_then(|(i, _)| stack.iface(i))
            .is_some_and(|source| {
                h.src != source.ip
                    && self.config.ports.get(&source.name).and_then(|p| p.nat)
                        == Some(NatRole::Inside)
            });
        if !from_inside {
            return Some(frame.to_vec());
        }
        let mut translated = pkt.to_vec();
        self.nat
            .outbound(&mut translated, f.ip, &self.config.forwards, now)
            .ok()?;
        ecm_net::eth::build_frame(eth.dst, eth.src, eth.vlan_tag, ETHERTYPE_IPV4, &translated)
    }
}
