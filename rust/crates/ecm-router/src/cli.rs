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
const HELP: &str = "\
configure terminal; interface ethN; ip routing; ip route PREFIX GATEWAY [ethN]; dhcp-server vrf default
router bgp ASN: bgp router-id IP; timers bgp KEEPALIVE HOLD
  neighbor IP remote-as ASN|external|internal; neighbor IP update-source ethN|IP
  neighbor GROUP peer-group; neighbor GROUP remote-as ASN|external|internal; neighbor IP peer-group GROUP
  neighbor GROUP listen ip-range PREFIX [as-range RANGE] [limit 1-512]; bgp listen limit N
  address-family ipv4 unicast: neighbor IP|GROUP activate | route-map NAME in|out | default-originate
    neighbor IP|GROUP maximum-prefix MAX [threshold PCT] [restart SECS] [warning-only]; network PREFIX
ip prefix-list NAME seq N permit|deny PREFIX [ge N] [le N]; route-map NAME permit|deny SEQ
show ip route; show arp; show ip nat translations; show dhcp-server leases
show bgp ipv4 unicast [summary | neighbors [IP]]; clear bgp * | IP
show running-config; write memory; end; exit
";
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
            ["help"]|["?"]=>return (HELP.into(),false,false),
            _=>{},
        }
        if matches!(self.context, Context::Bgp | Context::Family) {
            if let Some(r) = bgp_command(&self.context, c, &v) {
                return match r {
                    Ok(()) => ok(),
                    Err(e) => (format!("% {}
", e), false, false),
                };
            }
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
            (Context::Bgp, ["address-family", "ipv4", "unicast"]) => {
                self.context = Context::Family;
                return ok();
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

/// A peer-group name: not an address, starts with a letter, `[A-Za-z0-9_-]`, at most 32.
fn group_name(s: &str) -> bool {
    Ipv4Addr::parse(s).is_none()
        && s.len() <= 32
        && s.starts_with(|c: char| c.is_ascii_alphabetic())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The neighbor (by address) or peer group (by name) a `neighbor X ...` line addresses.
fn target<'a>(c: &'a mut Config, x: &str) -> Result<&'a mut ecm_bgp::Neighbor, String> {
    if let Some(ip) = Ipv4Addr::parse(x) {
        return c.bgp.neighbors.get_mut(&ip).ok_or_else(|| {
            format!(
                "Neighbor {} is not configured (use 'neighbor {} remote-as ASN' first)",
                x, x
            )
        });
    }
    if !group_name(x) {
        return Err(format!("Invalid neighbor address or peer-group name: {}", x));
    }
    c.bgp.groups.get_mut(x).ok_or_else(|| {
        format!(
            "Peer group {} does not exist (use 'neighbor {} peer-group' first)",
            x, x
        )
    })
}

fn listen_prefix(s: &str) -> Result<ecm_bgp::Prefix, String> {
    let (a, len) = Ipv4Addr::parse_cidr(s).ok_or_else(|| format!("Invalid prefix: {}", s))?;
    if len == 0 {
        return Err("A listen range must be narrower than 0.0.0.0/0".into());
    }
    Ok(ecm_bgp::Prefix::new(a, len))
}

fn add_listen(c: &mut Config, r: ecm_bgp::ListenRange) {
    c.bgp.listen.retain(|o| o.prefix != r.prefix);
    c.bgp.listen.push(r);
}

fn parse_listen_options(
    r: &mut ecm_bgp::ListenRange,
    rest: &[&str],
) -> Result<(), String> {
    for pair in rest.chunks(2) {
        match pair {
            ["as-range", range] => {
                r.as_range = Some(
                    ecm_bgp::parse_as_range(range)
                        .ok_or_else(|| format!("Invalid AS range: {}", range))?,
                )
            }
            ["limit", n] => match n.parse::<u32>() {
                Ok(n) if (1..=512).contains(&n) => r.limit = Some(n),
                _ => return Err(format!("Invalid limit {} (1-512)", n)),
            },
            _ => return Err("Expected [as-range RANGE] [limit 1-512]".into()),
        }
    }
    Ok(())
}

fn parse_max_prefix(max: &str, rest: &[&str]) -> Result<ecm_bgp::MaxPrefix, String> {
    let mut m = match max.parse::<u32>() {
        Ok(n) if (1..=128000).contains(&n) => ecm_bgp::MaxPrefix::new(n),
        _ => return Err(format!("Invalid maximum {} (1-128000)", max)),
    };
    let mut i = 0;
    while i < rest.len() {
        match (rest[i], rest.get(i + 1)) {
            ("warning-only", _) => {
                m.warning_only = true;
                i += 1;
            }
            ("threshold", Some(t)) => {
                m.threshold = match t.parse::<u8>() {
                    Ok(t) if (1..=100).contains(&t) => t,
                    _ => return Err(format!("Invalid threshold {} (1-100)", t)),
                };
                i += 2;
            }
            ("restart", Some(t)) => {
                m.restart = match t.parse::<u16>() {
                    Ok(t) if t >= 30 => Some(t),
                    _ => return Err(format!("Invalid restart interval {} (30-65535)", t)),
                };
                i += 2;
            }
            // Cisco/FRR positional threshold: maximum-prefix MAX PERCENT.
            (t, _) if i == 0 && t.parse::<u8>().is_ok_and(|t| (1..=100).contains(&t)) => {
                m.threshold = t.parse().unwrap();
                i += 1;
            }
            _ => return Err("Expected [threshold 1-100] [restart 30-65535] [warning-only]".into()),
        }
    }
    Ok(m)
}

/// BGP neighbor, peer-group and dynamic-neighbor commands of `router bgp` and its
/// IPv4 unicast address family. None: not one of these commands.
fn bgp_command(ctx: &Context, c: &mut Config, v: &[&str]) -> Option<Result<(), String>> {
    use ecm_bgp::{ListenRange, RemoteAs};
    let r = match (ctx, v) {
        // ---- router bgp context ----
        (Context::Bgp, ["neighbor", name, "peer-group"]) => {
            if !group_name(name) {
                Err(format!("Invalid peer-group name: {}", name))
            } else {
                c.bgp.groups.entry(name.to_string()).or_default();
                Ok(())
            }
        }
        (Context::Bgp, ["neighbor", ip, "peer-group", name]) => {
            match (Ipv4Addr::parse(ip), c.bgp.groups.contains_key(*name)) {
                (None, _) => Err(format!("Invalid neighbor address: {}", ip)),
                (_, false) => Err(format!("Peer group {} does not exist", name)),
                (Some(ip), true) => {
                    c.bgp.neighbors.entry(ip).or_default().peer_group = Some(name.to_string());
                    Ok(())
                }
            }
        }
        (Context::Bgp, ["neighbor", x, "remote-as", asn]) => match RemoteAs::parse(asn) {
            None => Err(format!(
                "Invalid remote AS {} (expected 1-4294967295, external or internal)",
                asn
            )),
            Some(rule) => match Ipv4Addr::parse(x) {
                Some(ip) => {
                    c.bgp.neighbors.entry(ip).or_default().remote_as = Some(rule);
                    Ok(())
                }
                None => target(c, x).map(|n| n.remote_as = Some(rule)),
            },
        },
        (Context::Bgp, ["neighbor", x, "update-source", source]) => {
            target(c, x).map(|n| n.update_source = Some(source.to_string()))
        }
        (Context::Bgp, ["no", "neighbor", x, "update-source", ..]) => {
            target(c, x).map(|n| n.update_source = None)
        }
        (Context::Bgp, ["neighbor", x, "listen", "ip-range", prefix, rest @ ..]) => {
            if !c.bgp.groups.contains_key(*x) {
                Err(if group_name(x) {
                    format!("Peer group {} does not exist", x)
                } else {
                    "Dynamic neighbors need a peer group: neighbor GROUP listen ip-range PREFIX"
                        .into()
                })
            } else {
                listen_prefix(prefix).and_then(|prefix| {
                    let mut r = ListenRange {
                        prefix,
                        group: x.to_string(),
                        as_range: None,
                        limit: None,
                    };
                    parse_listen_options(&mut r, rest)?;
                    add_listen(c, r);
                    Ok(())
                })
            }
        }
        (Context::Bgp, ["no", "neighbor", x, "listen", "ip-range", prefix, ..]) => {
            listen_prefix(prefix).and_then(|prefix| {
                let before = c.bgp.listen.len();
                c.bgp
                    .listen
                    .retain(|r| !(r.prefix == prefix && r.group == *x));
                if c.bgp.listen.len() == before {
                    Err(format!("No listen range {} for {}", v[5], x))
                } else {
                    Ok(())
                }
            })
        }
        // FRRouting / Cisco IOS spelling of the same range.
        (Context::Bgp, ["bgp", "listen", "range", prefix, "peer-group", name]) => {
            if !c.bgp.groups.contains_key(*name) {
                Err(format!("Peer group {} does not exist", name))
            } else {
                listen_prefix(prefix).map(|prefix| {
                    add_listen(
                        c,
                        ListenRange {
                            prefix,
                            group: name.to_string(),
                            as_range: None,
                            limit: None,
                        },
                    )
                })
            }
        }
        (Context::Bgp, ["no", "bgp", "listen", "range", prefix, "peer-group", name]) => {
            listen_prefix(prefix).map(|prefix| {
                c.bgp
                    .listen
                    .retain(|r| !(r.prefix == prefix && r.group == *name))
            })
        }
        (Context::Bgp, ["bgp", "listen", "limit", n]) => match n.parse::<u32>() {
            Ok(n) if (1..=65535).contains(&n) => {
                c.bgp.listen_limit = Some(n);
                Ok(())
            }
            _ => Err(format!("Invalid listen limit {} (1-65535)", n)),
        },
        (Context::Bgp, ["no", "bgp", "listen", "limit", ..]) => {
            c.bgp.listen_limit = None;
            Ok(())
        }
        (Context::Bgp, ["no", "neighbor", x]) => {
            if let Some(ip) = Ipv4Addr::parse(x) {
                c.bgp
                    .neighbors
                    .remove(&ip)
                    .map(|_| ())
                    .ok_or_else(|| format!("Neighbor {} is not configured", x))
            } else if c.bgp.groups.remove(*x).is_some() {
                // As in FRR: deleting a group deletes its ranges and members.
                c.bgp.listen.retain(|r| r.group != *x);
                c.bgp
                    .neighbors
                    .retain(|_, n| n.peer_group.as_deref() != Some(*x));
                Ok(())
            } else {
                Err(format!("Peer group {} does not exist", x))
            }
        }
        // ---- address-family ipv4 unicast ----
        (Context::Family, ["neighbor", x, "activate"]) => target(c, x).map(|n| n.active = true),
        (Context::Family, ["no", "neighbor", x, "activate"]) => {
            target(c, x).map(|n| n.active = false)
        }
        (Context::Family, ["neighbor", x, "default-originate"]) => {
            target(c, x).map(|n| n.default_originate = true)
        }
        (Context::Family, ["no", "neighbor", x, "default-originate"]) => {
            target(c, x).map(|n| n.default_originate = false)
        }
        (Context::Family, ["neighbor", x, "route-map", name, dir]) => match *dir {
            "in" => target(c, x).map(|n| n.inbound = Some(name.to_string())),
            "out" => target(c, x).map(|n| n.outbound = Some(name.to_string())),
            _ => Err("Expected in or out".into()),
        },
        (Context::Family, ["no", "neighbor", x, "route-map", _, dir]) => match *dir {
            "in" => target(c, x).map(|n| n.inbound = None),
            "out" => target(c, x).map(|n| n.outbound = None),
            _ => Err("Expected in or out".into()),
        },
        (Context::Family, ["neighbor", x, "maximum-prefix", max, rest @ ..]) => {
            parse_max_prefix(max, rest)
                .and_then(|m| target(c, x).map(|n| n.maximum_prefix = Some(m)))
        }
        (Context::Family, ["no", "neighbor", x, "maximum-prefix", ..]) => {
            target(c, x).map(|n| n.maximum_prefix = None)
        }
        _ => return None,
    };
    Some(r)
}
