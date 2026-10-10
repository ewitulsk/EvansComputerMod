//! Operational `show bgp ipv4 unicast summary|neighbors` output, AOS-CX layout.
use crate::{Engine, Peer, RemoteAs, State};
use ecm_net::Ipv4Addr;

/// `00h:05m:12s`, or `1d:02h:03m` past a day (AOS-CX style up/down time).
pub fn duration(ms: i64) -> String {
    let s = (ms.max(0) / 1000) as u64;
    let (d, h, m, sec) = (s / 86400, s / 3600 % 24, s / 60 % 60, s % 60);
    if d > 0 {
        format!("{}d:{:02}h:{:02}m", d, h, m)
    } else {
        format!("{:02}h:{:02}m:{:02}s", h, m, sec)
    }
}

fn state(s: State) -> &'static str {
    match s {
        State::Idle => "Idle",
        State::Connect => "Connect",
        State::Active => "Active",
        State::OpenSent => "OpenSent",
        State::OpenConfirm => "OpenConfirm",
        State::Established => "Established",
    }
}

fn remote_as(p: &Peer) -> String {
    if p.remote_as != 0 {
        p.remote_as.to_string()
    } else {
        match p.config.remote_as {
            Some(RemoteAs::External) => "external".into(),
            Some(RemoteAs::Internal) => "internal".into(),
            _ => "?".into(),
        }
    }
}

impl Engine {
    /// `show bgp ipv4 unicast summary`. Dynamic neighbors are marked `*` (as FRR does).
    pub fn show_summary(&self, now: i64) -> String {
        let c = &self.config;
        let mut s = String::new();
        s.push_str("VRF : default\nBGP Summary\n-----------\n");
        s.push_str(&format!(
            " Local AS               : {:<12} BGP Router Identifier  : {}\n",
            c.asn, c.router_id
        ));
        s.push_str(&format!(
            " Peers                  : {:<12} Dynamic Peers          : {}\n",
            self.peers.len(),
            self.dynamic_count(None)
        ));
        s.push_str(&format!(
            " Cfg. Hold Time         : {:<12} Cfg. Keep Alive        : {}\n\n",
            c.hold, c.keepalive
        ));
        s.push_str(" Neighbor         Remote-AS   MsgRcvd  MsgSent  Up/Down Time  State        AdminStatus  PfxRcd\n");
        for (ip, p) in &self.peers {
            s.push_str(&format!(
                "{}{:<16} {:<11} {:<8} {:<8} {:<13} {:<12} {:<12} {}\n",
                if p.is_dynamic() { "*" } else { " " },
                ip.to_string(),
                remote_as(p),
                p.msgs_in,
                p.msgs_out,
                duration(now - p.since),
                state(p.state),
                if !p.config.active {
                    "Down"
                } else if p.prefix_block.is_some() {
                    "Idle(PfxCt)"
                } else {
                    "Up"
                },
                if p.state == State::Established {
                    p.adj_in.len().to_string()
                } else {
                    "-".into()
                }
            ));
        }
        if self.peers.values().any(|p| p.is_dynamic()) || !c.listen.is_empty() {
            s.push_str("\n* - dynamic neighbor (listen ip-range)\n");
            for r in &c.listen {
                let n = self.dynamic_count(Some(r));
                s.push_str(&format!(
                    "Listen range {}/{} peer-group {}: {} dynamic neighbor(s){}\n",
                    r.prefix.address,
                    r.prefix.len,
                    r.group,
                    n,
                    r.limit.map(|l| format!(", limit {}", l)).unwrap_or_default()
                ));
            }
        }
        s
    }

    /// `show bgp ipv4 unicast neighbors [IP]`.
    pub fn show_neighbors(&self, only: Option<Ipv4Addr>, now: i64) -> String {
        let mut s = String::from("VRF : default\n");
        let mut any = false;
        for (ip, p) in &self.peers {
            if only.is_some_and(|o| o != *ip) {
                continue;
            }
            any = true;
            let kind = match &p.dynamic {
                Some(g) => format!(
                    " (dynamic, peer-group {}, listen range {})",
                    g,
                    self.config
                        .listen_range(*ip)
                        .map(|r| format!("{}/{}", r.prefix.address, r.prefix.len))
                        .unwrap_or_else(|| "-".into())
                ),
                None => match &p.config.peer_group {
                    Some(g) => format!(" (peer-group {})", g),
                    None => String::new(),
                },
            };
            s.push_str(&format!(
                "\n  BGP Neighbor {}{}\n  -------------------------------------------------\n",
                ip, kind
            ));
            let row = |s: &mut String, a: &str, av: String, b: &str, bv: String| {
                s.push_str(&format!("  {:<19}: {:<16} {:<17}: {}\n", a, av, b, bv));
            };
            row(
                &mut s,
                "Remote AS",
                remote_as(p),
                "Local AS",
                self.config.asn.to_string(),
            );
            row(
                &mut s,
                "Remote Router ID",
                p.router_id.to_string(),
                "Local Router ID",
                self.config.router_id.to_string(),
            );
            row(
                &mut s,
                "State",
                state(p.state).into(),
                "Admin Status",
                if p.config.active { "Up" } else { "Down" }.into(),
            );
            row(
                &mut s,
                "Up/Down Time",
                duration(now - p.since),
                "Local Address",
                p.local_addr().to_string(),
            );
            row(
                &mut s,
                "Hold Time",
                p.negotiated_hold.to_string(),
                "Keep Alive",
                self.config
                    .keepalive
                    .min((p.negotiated_hold / 3).max(1))
                    .to_string(),
            );
            row(
                &mut s,
                "Messages Rcvd",
                p.msgs_in.to_string(),
                "Messages Sent",
                p.msgs_out.to_string(),
            );
            row(
                &mut s,
                "Prefixes Accepted",
                p.adj_in.len().to_string(),
                "Advertised",
                p.adj_out.len().to_string(),
            );
            row(
                &mut s,
                "Route Map In",
                p.config.inbound.clone().unwrap_or_else(|| "-".into()),
                "Route Map Out",
                p.config.outbound.clone().unwrap_or_else(|| "-".into()),
            );
            if let Some(m) = p.config.maximum_prefix {
                s.push_str(&format!(
                    "  Maximum Prefix     : {} (warning at {}%{}{})\n",
                    m.max,
                    m.threshold,
                    m.restart
                        .map(|r| format!(", restart {}s", r))
                        .unwrap_or_default(),
                    if m.warning_only { ", warning-only" } else { "" }
                ));
            }
            if let Some(t) = p.prefix_block {
                s.push_str(&format!(
                    "  Prefix limit hit   : session held down {}\n",
                    if t == i64::MAX {
                        "until 'clear bgp'".to_string()
                    } else {
                        format!("for {}", duration(t - now))
                    }
                ));
            }
            if let Some(e) = p.last_error {
                s.push_str(&format!("  Last Notification  : {}\n", e.describe()));
            }
        }
        for (ip, until) in &self.blocked {
            if only.is_none_or(|o| o == *ip) {
                any = true;
                s.push_str(&format!(
                    "\n  {} (dynamic): shut out after exceeding maximum-prefix, {}\n",
                    ip,
                    if *until == i64::MAX {
                        "until 'clear bgp'".to_string()
                    } else {
                        format!("for {}", duration(until - now))
                    }
                ));
            }
        }
        if !any {
            s.push_str(match only {
                Some(_) => "% No such neighbor\n",
                None => "No BGP neighbors.\n",
            });
        }
        s
    }
}
