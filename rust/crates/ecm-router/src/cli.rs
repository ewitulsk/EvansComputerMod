//! AOS-CX contexts. NAT and WAN DHCP are documented Aruba-style extensions.
use crate::{nat::Forward, Config, NatRole};
use ecm_net::Ipv4Addr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Context {
    Exec,
    Config,
    Interface(String),
    Dhcp,
    Pool(String),
    Bgp,
    Family,
    RouteMap(String, u32),
}
pub struct Session {
    pub context: Context,
}
impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}
impl Session {
    pub fn new() -> Self {
        Self {
            context: Context::Exec,
        }
    }
    pub fn prompt(&self) -> String {
        format!(
            "router{}# ",
            match &self.context {
                Context::Exec => "",
                Context::Config => "(config)",
                Context::Interface(_) => "(config-if)",
                Context::Dhcp => "(config-dhcp)",
                Context::Pool(_) => "(config-dhcp-pool)",
                Context::Bgp => "(config-bgp)",
                Context::Family => "(config-bgp-ipv4-uc)",
                Context::RouteMap(_, _) => "(config-route-map)",
            }
        )
    }
    /// Returns (output, leave CLI, save config). Invalid lines never change config.
    pub fn exec(&mut self, c: &mut Config, line: &str) -> (String, bool, bool) {
        let v: Vec<_> = line.split_whitespace().collect();
        if v.is_empty() || v[0].starts_with('#') {
            return (String::new(), false, false);
        }
        let ok = || (String::new(), false, false);
        match v.as_slice() {
            ["configure","terminal"]|["conf","t"]=>{self.context=Context::Config;return ok();},
            ["end"]=>{self.context=Context::Exec;return ok();},
            ["exit"]=>{
                self.context=match &self.context {Context::Exec=>return (String::new(),true,false),Context::Config=>Context::Exec,
                    Context::Interface(_)|Context::Dhcp|Context::Bgp|Context::RouteMap(_,_)=>Context::Config,Context::Pool(_)=>Context::Dhcp,Context::Family=>Context::Bgp};return ok();
            },
            ["write","memory"]|["copy","running-config","startup-config"]=>return ("Configuration saved.\n".into(),false,true),
            ["show","running-config"]=>return (c.render(),false,false),
            ["help"]|["?"]=>return ("configure terminal; interface ethN; ip routing; ip route PREFIX GATEWAY [ethN]; dhcp-server vrf default; show ip route; show arp; show ip nat translations; write memory; exit\n".into(),false,false),
            _=>{},
        }
        match (&self.context, v.as_slice()) {
            (Context::Config, ["router", "bgp", asn]) => {
                if let Ok(asn) = asn.parse::<u32>() {
                    if asn > 0 {
                        c.bgp.asn = asn;
                        self.context = Context::Bgp;
                        return ok();
                    }
                }
            }
            (Context::Bgp, ["bgp", "router-id", ip]) => {
                if let Some(ip) = Ipv4Addr::parse(ip) {
                    if !ip.is_unspecified() && !ip.is_multicast() {
                        c.bgp.router_id = ip;
                        return ok();
                    }
                }
            }
            (Context::Bgp, ["timers", "bgp", keep, hold]) => {
                if let (Ok(k), Ok(h)) = (keep.parse::<u16>(), hold.parse::<u16>()) {
                    if k > 0 && (h == 0 || h >= 3) {
                        c.bgp.keepalive = k;
                        c.bgp.hold = h;
                        return ok();
                    }
                }
            }
            (Context::Bgp, ["neighbor", ip, "remote-as", asn]) => {
                if let (Some(ip), Ok(asn)) = (Ipv4Addr::parse(ip), asn.parse::<u32>()) {
                    if asn > 0 {
                        c.bgp
                            .neighbors
                            .entry(ip)
                            .or_insert_with(|| ecm_bgp::Neighbor::new(asn))
                            .remote_as = asn;
                        return ok();
                    }
                }
            }
            (Context::Bgp, ["neighbor", ip, "update-source", source]) => {
                if let Some(n) = Ipv4Addr::parse(ip).and_then(|ip| c.bgp.neighbors.get_mut(&ip)) {
                    n.update_source = Some(source.to_string());
                    return ok();
                }
            }
            (Context::Bgp, ["address-family", "ipv4", "unicast"]) => {
                self.context = Context::Family;
                return ok();
            }
            (Context::Family, ["neighbor", ip, "activate"]) => {
                if let Some(n) = Ipv4Addr::parse(ip).and_then(|ip| c.bgp.neighbors.get_mut(&ip)) {
                    n.active = true;
                    return ok();
                }
            }
            (Context::Family, ["neighbor", ip, "default-originate"]) => {
                if let Some(n) = Ipv4Addr::parse(ip).and_then(|ip| c.bgp.neighbors.get_mut(&ip)) {
                    n.default_originate = true;
                    return ok();
                }
            }
            (Context::Family, ["neighbor", ip, "route-map", name, direction]) => {
                if let Some(n) = Ipv4Addr::parse(ip).and_then(|ip| c.bgp.neighbors.get_mut(&ip)) {
                    match *direction {
                        "in" => n.inbound = Some(name.to_string()),
                        "out" => n.outbound = Some(name.to_string()),
                        _ => return ("% Expected in or out\n".into(), false, false),
                    };
                    return ok();
                }
            }
            (Context::Family, ["network", prefix]) => {
                if let Some((a, p)) = Ipv4Addr::parse_cidr(prefix) {
                    let p = ecm_bgp::Prefix::new(a, p);
                    if !c.bgp.networks.contains(&p) {
                        c.bgp.networks.push(p);
                    }
                    return ok();
                }
            }
            (Context::Family, ["redistribute", source]) => {
                match *source {
                    "connected" => c.bgp.connected = true,
                    "static" => c.bgp.static_routes = true,
                    _ => return ("% Expected connected or static\n".into(), false, false),
                };
                return ok();
            }
            (Context::Config, ["ip", "prefix-list", name, "seq", seq, mode, prefix, rest @ ..]) => {
                if let (Ok(seq), Some((a, p))) = (seq.parse::<u32>(), Ipv4Addr::parse_cidr(prefix))
                {
                    if matches!(*mode, "permit" | "deny") {
                        let (mut min, mut max) = (p, p);
                        let mut valid = true;
                        for pair in rest.chunks(2) {
                            if pair.len() != 2 {
                                valid = false;
                                break;
                            }
                            match (pair[0], pair[1].parse::<u8>()) {
                                ("ge", Ok(n)) => {
                                    min = n;
                                    max = 32;
                                }
                                ("le", Ok(n)) => max = n,
                                _ => valid = false,
                            }
                        }
                        if valid && p <= min && min <= max && max <= 32 {
                            let list = c.policy.lists.entry(name.to_string()).or_default();
                            list.retain(|r| r.seq != seq);
                            list.push(ecm_bgp::PrefixRule {
                                seq,
                                permit: *mode == "permit",
                                prefix: ecm_bgp::Prefix::new(a, p),
                                min,
                                max,
                            });
                            return ok();
                        }
                    }
                }
            }
            (Context::Config, ["route-map", name, mode, seq])
                if matches!(*mode, "permit" | "deny") =>
            {
                if let Ok(seq) = seq.parse::<u32>() {
                    let rules = c.policy.maps.entry(name.to_string()).or_default();
                    if let Some(r) = rules.iter_mut().find(|r| r.seq == seq) {
                        r.permit = *mode == "permit";
                    } else {
                        rules.push(ecm_bgp::MapRule {
                            seq,
                            permit: *mode == "permit",
                            prefix_list: None,
                            local_pref: None,
                            med: None,
                            community: None,
                        });
                    }
                    self.context = Context::RouteMap(name.to_string(), seq);
                    return ok();
                }
            }
            (Context::RouteMap(name, seq), ["match", "ip", "address", "prefix-list", list]) => {
                c.policy
                    .maps
                    .get_mut(name)
                    .unwrap()
                    .iter_mut()
                    .find(|r| r.seq == *seq)
                    .unwrap()
                    .prefix_list = Some(list.to_string());
                return ok();
            }
            (Context::RouteMap(name, seq), ["set", field, value]) => {
                let value = if *field == "community" {
                    let b: Vec<_> = value.split(':').collect();
                    if b.len() == 2 {
                        b[0].parse::<u16>()
                            .ok()
                            .zip(b[1].parse::<u16>().ok())
                            .map(|(a, b)| ((a as u32) << 16) | b as u32)
                    } else {
                        value.parse().ok()
                    }
                } else {
                    value.parse().ok()
                };
                if let Some(value) = value {
                    let r = c
                        .policy
                        .maps
                        .get_mut(name)
                        .unwrap()
                        .iter_mut()
                        .find(|r| r.seq == *seq)
                        .unwrap();
                    match *field {
                        "local-preference" => r.local_pref = Some(value),
                        "metric" => r.med = Some(value),
                        "community" => r.community = Some(value),
                        _ => return ("% Unsupported set\n".into(), false, false),
                    };
                    return ok();
                }
            }
            (Context::Config, ["ip", "routing"]) => {
                c.routing = true;
                return ok();
            }
            (Context::Config, ["no", "ip", "routing"]) => {
                c.routing = false;
                return ok();
            }
            (Context::Config, ["interface", name])
                if name.starts_with("eth") || name.starts_with("vlan") =>
            {
                c.ports.entry(name.to_string()).or_default();
                self.context = Context::Interface(name.to_string());
                return ok();
            }
            (Context::Config, ["ip", "route", prefix, gw, ..]) if v.len() == 4 || v.len() == 5 => {
                // Optional interface is resolved from the next hop by the kernel.
                let dev = v.get(4).copied().unwrap_or("");
                if let (Some((d, p)), Some(g)) = (Ipv4Addr::parse_cidr(prefix), Ipv4Addr::parse(gw))
                {
                    c.routes.retain(|r| !(r.0 == d.network_addr(p) && r.1 == p));
                    c.routes.push((d.network_addr(p), p, g, dev.to_string()));
                    return ok();
                }
            }
            (
                Context::Config,
                ["ip", "nat", "inside", "source", "static", proto, ip, local, external],
            ) => {
                if let (Some(ip), Ok(local), Ok(external)) = (
                    Ipv4Addr::parse(ip),
                    local.parse::<u16>(),
                    external.parse::<u16>(),
                ) {
                    let protocol = match *proto {
                        "tcp" => 6,
                        "udp" => 17,
                        _ => 0,
                    };
                    if protocol != 0 && local != 0 && external != 0 {
                        c.forwards
                            .retain(|f| !(f.protocol == protocol && f.outside_port == external));
                        c.forwards.push(Forward {
                            protocol,
                            inside: ip,
                            inside_port: local,
                            outside_port: external,
                        });
                        return ok();
                    }
                }
            }
            (Context::Config, ["dhcp-server", "vrf", "default"]) => {
                self.context = Context::Dhcp;
                return ok();
            }
            (Context::Dhcp, ["pool", name]) => {
                c.pools.entry(name.to_string()).or_default();
                self.context = Context::Pool(name.to_string());
                return ok();
            }
            (Context::Interface(_), ["routing"]) => return ok(),
            (Context::Interface(name), ["ip", "address", cidr]) => {
                if let Some(addr) = Ipv4Addr::parse_cidr(cidr) {
                    let p = c.ports.get_mut(name).unwrap();
                    p.address = Some(addr);
                    p.dhcp = false;
                    return ok();
                }
            }
            (Context::Interface(name), ["ip", "dhcp"]) => {
                let p = c.ports.get_mut(name).unwrap();
                p.dhcp = true;
                p.address = None;
                return ok();
            }
            (Context::Interface(name), ["ip", "nat", role]) => {
                if let Some(role) = match *role {
                    "inside" => Some(NatRole::Inside),
                    "outside" => Some(NatRole::Outside),
                    _ => None,
                } {
                    c.ports.get_mut(name).unwrap().nat = Some(role);
                    return ok();
                }
            }
            (Context::Pool(name), ["range", start, end]) => {
                if let (Some(a), Some(b)) = (Ipv4Addr::parse(start), Ipv4Addr::parse(end)) {
                    if a.0 <= b.0 && a.same_subnet_prefix(&b, 24) {
                        let p = c.pools.get_mut(name).unwrap();
                        p.start = a;
                        p.end = b;
                        return ok();
                    }
                }
            }
            (Context::Pool(name), ["default-router", ip])
            | (Context::Pool(name), ["dns-server", ip]) => {
                if let Some(ip) = Ipv4Addr::parse(ip) {
                    let p = c.pools.get_mut(name).unwrap();
                    if v[0] == "default-router" {
                        p.router = ip;
                    } else {
                        p.dns = ip;
                    }
                    return ok();
                }
            }
            (Context::Pool(name), ["lease", secs]) => {
                if let Ok(n) = secs.parse::<u32>() {
                    if (4..=31536000).contains(&n) {
                        c.pools.get_mut(name).unwrap().lease_secs = n;
                        return ok();
                    }
                }
            }
            (Context::Pool(name), ["enable"]) => {
                c.pools.get_mut(name).unwrap().enabled = true;
                return ok();
            }
            (Context::Pool(name), ["disable"]) => {
                c.pools.get_mut(name).unwrap().enabled = false;
                return ok();
            }
            _ => {}
        }
        (
            format!("% Invalid command in this context: {}\n", line),
            false,
            false,
        )
    }
}
pub fn load(text: &str) -> Result<Config, String> {
    let mut c = Config::default();
    let mut s = Session::new();
    for line in text.lines() {
        let (o, _, _) = s.exec(&mut c, line);
        if o.starts_with('%') {
            return Err(o);
        }
    }
    Ok(c)
}
