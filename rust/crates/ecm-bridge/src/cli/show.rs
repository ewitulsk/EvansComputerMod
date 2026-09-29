//! `show ...` and `clear ...` commands.

use super::{format_vlan_list, parse_eth, parse_mac, parse_port_ref, parse_vid, running_config, Out};
use crate::api::{FdbEntryInfo, FdbFilter};
use crate::bridge::{fmt_ip, fmt_mac, Bridge};
use crate::lacpdu::state_string;
use crate::log::{format_time, Severity};
use crate::stp::format_bid;
use crate::types::*;

pub(super) fn show(b: &Bridge, o: &mut Out, args: &[&str]) {
    let rest = args.get(1..).unwrap_or(&[]);
    match args.first().copied().unwrap_or("") {
        "mac-address-table" => show_mac(b, o, rest),
        "vlan" => show_vlan(b, o, rest),
        "logging" | "events" => show_logging(b, o, rest),
        "lldp" => show_lldp(b, o, rest),
        "spanning-tree" => show_stp(b, o),
        "lacp" => show_lacp(b, o, rest),
        "interface" | "interfaces" => match rest {
            ["lag", id] => show_interface_lag(b, o, id),
            [x] if x.starts_with("lag") && x.len() > 3 => show_interface_lag(b, o, x.get(3..).unwrap_or("")),
            ["vlan"] | ["vlan", ..] => show_interface_vlan(b, o),
            [] | ["brief"] => show_interface_brief(b, o),
            _ => o.err("Usage: show interface [brief] | show interface lag <id> | show interface vlan"),
        },
        "running-config" | "run" => {
            for l in running_config(b, &[]).lines() {
                o.line(l);
            }
        }
        _ => o.err("Unknown show command. Try 'help'."),
    }
}

// ---------------------------------------------------------------------
// MAC table
// ---------------------------------------------------------------------

#[derive(Default, Clone, Copy)]
struct Filter {
    only_dynamic: bool,
    only_static: bool,
    port: Option<PortRef>,
    vlan: Option<u16>,
    mac: Option<[u8; 6]>,
}

impl Filter {
    fn matches(&self, e: &FdbEntryInfo) -> bool {
        !(self.only_dynamic && e.is_static)
            && !(self.only_static && !e.is_static)
            && self.port.map(|p| p == e.port).unwrap_or(true)
            && self.vlan.map(|v| v == e.vlan).unwrap_or(true)
            && self.mac.map(|m| m == e.mac).unwrap_or(true)
    }
}

fn port_col(p: PortRef) -> String {
    match p {
        PortRef::Eth(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn table_header(o: &mut Out) {
    o.line("MAC Address         VLAN    Type     Port");
    o.line("-----------------------------------------");
}

fn entry_line(o: &mut Out, e: &FdbEntryInfo) {
    let kind = if e.is_static { "static" } else { "dynamic" };
    o.line(format!("{}   {:<6}  {:<7}  {}", fmt_mac(&e.mac), e.vlan, kind, port_col(e.port)));
}

/// Parse `port <p>` / `vlan <v>` / `address <m>` pairs into a filter.
fn parse_filter(b: &Bridge, o: &mut Out, mut args: &[&str], f: &mut Filter) -> bool {
    while !args.is_empty() {
        match args {
            ["port", p, r @ ..] => {
                match parse_port_ref(p, b.port_count()) {
                    Some(x) => f.port = Some(x),
                    None => {
                        o.err("Invalid port number");
                        return false;
                    }
                }
                args = r;
            }
            ["vlan", v, r @ ..] => {
                match v.parse::<u16>() {
                    Ok(x) => f.vlan = Some(x),
                    Err(_) => {
                        o.err("Invalid VLAN id");
                        return false;
                    }
                }
                args = r;
            }
            ["address", m, r @ ..] => {
                match parse_mac(m) {
                    Some(x) => f.mac = Some(x),
                    None => {
                        o.err("Invalid MAC address");
                        return false;
                    }
                }
                args = r;
            }
            _ => {
                o.err("Unknown show mac-address-table option. Try 'help'.");
                return false;
            }
        }
    }
    true
}

fn show_mac(b: &Bridge, o: &mut Out, args: &[&str]) {
    let entries = b.fdb_entries();
    let mut f = Filter::default();
    match args.first().copied() {
        None => {
            o.line(format!("MAC age-time            : {} seconds", b.age_time_secs()));
            o.line(format!("Number of MAC addresses : {}", entries.len()));
            o.line("");
            table_header(o);
            for e in &entries {
                entry_line(o, e);
            }
            return;
        }
        Some("count") => {
            let rest = args.get(1..).unwrap_or(&[]);
            let rest = if rest.first() == Some(&"dynamic") {
                f.only_dynamic = true;
                rest.get(1..).unwrap_or(&[])
            } else {
                rest
            };
            if !parse_filter(b, o, rest, &mut f) {
                return;
            }
            o.line(format!("Number of entries: {}", entries.iter().filter(|e| f.matches(e)).count()));
            return;
        }
        Some("mac-move") => {
            if !parse_filter(b, o, args.get(1..).unwrap_or(&[]), &mut f) {
                return;
            }
            o.line("MAC Address         VLAN    Curr  Prev  Moves  Last-Move");
            o.line("-----------------------------------------------------------");
            let mut any = false;
            for e in entries.iter().filter(|e| e.move_count > 0 && f.matches(e)) {
                let prev = e.prev_port.map(port_col).unwrap_or_else(|| "-".to_string());
                o.line(format!(
                    "{}   {:<6}  {:<4}  {:<4}  {:<5}  {}",
                    fmt_mac(&e.mac),
                    e.vlan,
                    port_col(e.port),
                    prev,
                    e.move_count,
                    if e.last_move_ms >= 0 { format_time(e.last_move_ms) } else { "-".into() }
                ));
                any = true;
            }
            if !any {
                o.line("(no MAC moves recorded)");
            }
            return;
        }
        Some("dynamic") => {
            f.only_dynamic = true;
            if !parse_filter(b, o, args.get(1..).unwrap_or(&[]), &mut f) {
                return;
            }
        }
        Some("static") => {
            f.only_static = true;
            if !parse_filter(b, o, args.get(1..).unwrap_or(&[]), &mut f) {
                return;
            }
        }
        Some(_) => {
            if !parse_filter(b, o, args, &mut f) {
                return;
            }
        }
    }
    table_header(o);
    let mut n = 0;
    for e in entries.iter().filter(|e| f.matches(e)) {
        entry_line(o, e);
        n += 1;
    }
    if n == 0 {
        o.line("(no matching entries)");
    }
}

// ---------------------------------------------------------------------
// VLANs
// ---------------------------------------------------------------------

fn port_list(v: &[PortRef]) -> String {
    if v.is_empty() {
        "-".to_string()
    } else {
        v.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",")
    }
}

fn vlan_header(o: &mut Out) {
    o.line("VLAN  Name             Status   Reason       Ports (untagged / tagged)");
    o.line("----  ---------------  -------  -----------  -----------------------------");
}

fn vlan_row(b: &Bridge, o: &mut Out, v: &VlanConfig) {
    let (u, t) = b.vlan_members(v.vid);
    let (status, reason) = if !v.active {
        ("down", "admin-down")
    } else if u.is_empty() && t.is_empty() {
        ("down", "no-members")
    } else if !u.iter().chain(t.iter()).any(|p| b.bridge_port_oper_up(*p)) {
        ("down", "no-member-up")
    } else {
        ("up", "ok")
    };
    let name: String = v.name.as_deref().unwrap_or("-").chars().take(15).collect();
    o.line(format!("{:<4}  {:<15}  {:<7}  {:<11}  {} / {}", v.vid, name, status, reason, port_list(&u), port_list(&t)));
}

fn show_vlan(b: &Bridge, o: &mut Out, args: &[&str]) {
    match args {
        [] => {
            vlan_header(o);
            for v in b.vlans() {
                vlan_row(b, o, v);
            }
        }
        ["summary"] => o.line(format!("Number of VLANs: {}", b.vlans().count())),
        ["port", p] => {
            let n = b.port_count();
            let i = match parse_port_ref(p, n) {
                Some(PortRef::Eth(x)) => PortRef::Eth(x),
                Some(PortRef::Lag(id)) if b.lag_config(id).is_some() => PortRef::Lag(id),
                _ => return o.err("Invalid port"),
            };
            o.line(format!("Port: {}", i));
            if let PortRef::Eth(x) = i {
                if let Some(l) = b.eth_config(x).and_then(|c| c.lag) {
                    o.line(format!("Member of: lag{} (LAG VLAN config applies)", l));
                }
            }
            match b.port_mode(i) {
                Some(PortMode::Access { vid }) => {
                    o.line("Mode: access");
                    o.line(format!("Untagged VLAN: {}", vid));
                    o.line("Tagged VLANs: -");
                }
                Some(PortMode::Trunk { native, native_tag, allowed }) => {
                    o.line("Mode: trunk");
                    o.line(format!("Native VLAN: {} ({})", native, if *native_tag { "tagged" } else { "untagged" }));
                    let a = match allowed {
                        AllowedList::All => "all".to_string(),
                        AllowedList::Some(s) if s.is_empty() => "-".to_string(),
                        AllowedList::Some(s) => format_vlan_list(s),
                    };
                    o.line(format!("Allowed VLANs: {}", a));
                }
                _ => o.line("Mode: routed (L3)"),
            }
        }
        [v] => match parse_vid(v) {
            Ok(vid) => match b.vlan(vid) {
                Some(cfg) => {
                    vlan_header(o);
                    vlan_row(b, o, cfg);
                    let (u, t) = b.vlan_members(vid);
                    o.line("");
                    o.line(format!("Untagged ports: {}", port_list(&u)));
                    o.line(format!("Tagged ports:   {}", port_list(&t)));
                    if let Some(d) = &cfg.description {
                        o.line(format!("Description:    {}", d));
                    }
                }
                None => o.err(format!("VLAN {} does not exist", vid)),
            },
            Err(_) => o.err("Usage: show vlan [<id> | summary | port <n>]"),
        },
        _ => o.err("Usage: show vlan [<id> | summary | port <n>]"),
    }
}

// ---------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------

fn show_logging(b: &Bridge, o: &mut Out, mut args: &[&str]) {
    let mut reverse = false;
    let mut min: Option<Severity> = None;
    while !args.is_empty() {
        match args {
            ["-r" | "reverse", r @ ..] => {
                reverse = true;
                args = r;
            }
            ["severity", s, r @ ..] => {
                match Severity::parse(s) {
                    Some(x) => min = Some(x),
                    None => return o.err("Unknown severity"),
                }
                args = r;
            }
            _ => return o.err("Usage: show logging [-r] [severity <lvl>]"),
        }
    }
    o.line(format!("Logging severity: {}", b.log_severity().as_str()));
    o.line(format!("Console: {}", if b.log_console() { "on" } else { "off" }));
    let remotes = b.log_remotes();
    if !remotes.is_empty() {
        o.line(format!("Collectors: {}", remotes.iter().map(fmt_ip).collect::<Vec<_>>().join(", ")));
    }
    let entries: Vec<_> = b.log_entries().filter(|e| min.map(|m| e.severity.passes(m)).unwrap_or(true)).collect();
    o.line(format!("Entries: {}", entries.len()));
    o.line("");
    let it: Box<dyn Iterator<Item = &&crate::log::LogEntry>> = if reverse { Box::new(entries.iter().rev()) } else { Box::new(entries.iter()) };
    for e in it {
        o.line(e.format());
    }
}

// ---------------------------------------------------------------------
// LLDP
// ---------------------------------------------------------------------

fn onoff(b: bool) -> &'static str {
    if b {
        "on "
    } else {
        "off"
    }
}

fn show_lldp(b: &Bridge, o: &mut Out, args: &[&str]) {
    let c = b.lldp_config();
    match args {
        ["configuration"] => {
            o.line("LLDP Global:");
            o.line(format!("  Enabled   : {}", c.enabled));
            o.line(format!("  Timer     : {} s", c.timer_secs));
            o.line(format!("  Holdtime  : {} (TTL = {} s)", c.holdtime, (c.timer_secs.saturating_mul(c.holdtime) + 1).min(65535)));
            o.line(format!("  Reinit    : {} s", c.reinit_secs));
            o.line(format!("  Tx-delay  : {} s", c.txdelay_secs));
            if let Some(ip) = c.mgmt_ipv4 {
                o.line(format!("  Mgmt IPv4 : {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]));
            }
            o.line("");
            o.line("Per-Port:");
            o.line("  Port    Tx    Rx");
            for i in 0..b.port_count() {
                if let Some(e) = b.eth_config(i) {
                    o.line(format!("  eth{:<3}  {}   {}", i, onoff(e.lldp_tx), onoff(e.lldp_rx)));
                }
            }
        }
        ["neighbor-info", rest @ ..] => {
            let filter = match rest {
                [] => None,
                [p] => match parse_eth(p, b.port_count()) {
                    Some(x) => Some(x),
                    None => return o.err("Invalid port"),
                },
                _ => return o.err("Usage: show lldp neighbor-info [<port>]"),
            };
            let ns = b.lldp_neighbors(filter);
            if ns.is_empty() {
                o.line("(no LLDP neighbors)");
                return;
            }
            if filter.is_some() {
                for n in &ns {
                    o.line(format!("Local Port       : eth{}", n.port));
                    o.line(format!("Chassis ID       : {}", n.chassis_string()));
                    o.line(format!("Port ID          : {}", n.port_string()));
                    o.line(format!("TTL              : {} s", n.ttl));
                    o.line(format!("Port Description : {}", n.port_desc.as_deref().unwrap_or("-")));
                    o.line(format!("System Name      : {}", n.sys_name.as_deref().unwrap_or("-")));
                    o.line(format!("System Desc      : {}", n.sys_desc.as_deref().unwrap_or("-")));
                    if let Some((cap, en)) = n.caps {
                        o.line(format!("Capabilities     : 0x{:04x} (enabled 0x{:04x})", cap, en));
                    }
                    if let Some(ip) = n.mgmt_ipv4 {
                        o.line(format!("Mgmt IPv4        : {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]));
                    }
                    o.line("");
                }
                return;
            }
            o.line("Port    Chassis             Remote-Port        Sys-Name          TTL");
            o.line("-----------------------------------------------------------------------");
            for n in &ns {
                let sys: String = n.sys_name.clone().unwrap_or_default().chars().take(16).collect();
                o.line(format!("eth{:<3}  {:<18}  {:<16}  {:<16}  {}", n.port, n.chassis_string(), n.port_string(), sys, n.ttl));
            }
        }
        ["statistics"] => {
            o.line("Port    Tx      Rx      Rx-Err  Discard  Aged    TooMany");
            for i in 0..b.port_count() {
                if let Some(s) = b.lldp_port_stats(i) {
                    o.line(format!("eth{:<3}  {:<6}  {:<6}  {:<6}  {:<7}  {:<6}  {}", i, s.tx, s.rx, s.rx_errors, s.rx_discards, s.ageouts, s.too_many));
                }
            }
        }
        ["tlv"] => {
            o.line("Optional TLVs:");
            for t in LldpTlv::ALL {
                o.line(format!("  {:<10}: {}", t.name(), c.tlv(t)));
            }
        }
        ["local-device"] => {
            o.line(format!("Chassis ID (MAC) : {}", fmt_mac(&b.bridge_mac().0)));
            o.line(format!("System Name      : {}", c.sys_name));
            o.line(format!("System Desc      : {}", c.sys_desc));
            o.line("Capabilities     : bridge");
            let mgmt = c.mgmt_ipv4.map(ecm_net::types::Ipv4Addr).or_else(|| b.svis().first().map(|s| s.1));
            if let Some(ip) = mgmt {
                o.line(format!("Mgmt IPv4        : {}", fmt_ip(&ip)));
            }
        }
        _ => o.err("Usage: show lldp [configuration|neighbor-info|statistics|tlv|local-device]"),
    }
}

// ---------------------------------------------------------------------
// Spanning tree
// ---------------------------------------------------------------------

fn show_stp(b: &Bridge, o: &mut Out) {
    let s = b.stp_status();
    let c = b.stp_config();
    o.line(format!("Spanning Tree: {} (RSTP, CIST)", if s.enabled { "enabled" } else { "disabled" }));
    o.line(format!("  Bridge ID  : {}", format_bid(s.bridge_id)));
    o.line(format!("  Root ID    : {}", format_bid(s.root_id)));
    if s.is_root {
        o.line("  This bridge is the root");
    }
    o.line(format!("  Root port  : {}", s.root_port.map(|p| p.to_string()).unwrap_or_else(|| "-".into())));
    o.line(format!("  Root cost  : {}", s.root_path_cost));
    o.line(format!("  Hello/FD/MaxAge: {}/{}/{}", c.hello_secs, c.forward_delay_secs, c.max_age_secs));
    o.line(format!(
        "  Topo changes: {}{}",
        s.topology_changes,
        s.last_tc_ms.map(|t| format!(" (last at {})", format_time(t))).unwrap_or_default()
    ));
    if !c.config_name.is_empty() || c.config_revision != 0 {
        o.line(format!("  Config name/revision: {}/{}", c.config_name, c.config_revision));
    }
    o.line("");
    o.line("Port    Role         State        Cost       Prio  Flags      Designated-Bridge");
    o.line("--------------------------------------------------------------------------------");
    for bp in b.bridge_ports() {
        if let Some(p) = b.stp_port_status(bp) {
            let mut flags = String::new();
            if p.err_disabled {
                flags.push_str("ERR ");
            }
            if p.root_inconsistent {
                flags.push_str("RINC ");
            }
            if p.oper_edge {
                flags.push_str("Edge ");
            }
            if !p.stp_active {
                flags.push_str("NoSTP ");
            }
            let (role, state) = if !p.stp_active {
                (if p.oper_up { "-" } else { "Disabled" }, if p.forwarding { "Forwarding" } else { "Down" })
            } else {
                (p.role.as_str(), p.state.as_str())
            };
            o.line(format!(
                "{:<6}  {:<11}  {:<11}  {:<9}  {:<4}  {:<9}  {}",
                bp.to_string(),
                role,
                state,
                p.cost,
                p.priority,
                if flags.is_empty() { "-".to_string() } else { flags.trim_end().to_string() },
                p.designated_bridge.map(format_bid).unwrap_or_else(|| "-".into())
            ));
        }
    }
}

// ---------------------------------------------------------------------
// LACP / LAG
// ---------------------------------------------------------------------

fn mode_str(m: LacpMode) -> &'static str {
    match m {
        LacpMode::Active => "active",
        LacpMode::Passive => "passive",
        LacpMode::Off => "static",
    }
}

fn rate_str(r: LacpRate) -> &'static str {
    match r {
        LacpRate::Fast => "fast",
        LacpRate::Slow => "slow",
    }
}

fn eth_list(v: &[usize]) -> String {
    if v.is_empty() {
        "-".into()
    } else {
        v.iter().map(|m| format!("eth{}", m)).collect::<Vec<_>>().join(",")
    }
}

fn show_lacp(b: &Bridge, o: &mut Out, args: &[&str]) {
    match args {
        ["configuration"] => {
            o.line(format!("System Priority : {}", crate::lacp::SYSTEM_PRIORITY));
            o.line(format!("System MAC      : {}", fmt_mac(&b.bridge_mac().0)));
            for id in b.lag_ids() {
                if let Some(c) = b.lag_config(id) {
                    o.line(format!(
                        "LAG {}: mode={} rate={} hash={} fallback={} members={}",
                        id,
                        mode_str(c.lacp_mode),
                        rate_str(c.lacp_rate),
                        c.hash.as_str(),
                        c.fallback,
                        eth_list(&b.lag_members(id))
                    ));
                }
            }
        }
        ["aggregates"] => {
            o.line("LAG   Members                 Mode      Rate   Up  Distributing");
            for id in b.lag_ids() {
                if let Some(c) = b.lag_config(id) {
                    o.line(format!(
                        "{:<4}  {:<22}  {:<8}  {:<5}  {:<2}  {}",
                        id,
                        eth_list(&b.lag_members(id)),
                        mode_str(c.lacp_mode),
                        rate_str(c.lacp_rate),
                        if b.lag_oper_up(id) { "y" } else { "n" },
                        eth_list(&b.lag_distributing_members(id))
                    ));
                }
            }
        }
        ["interfaces"] => {
            o.line("Port    LAG   Actor     Partner   PartnerSystem      Key    Sel  Mux        Tx      Rx");
            o.line("---------------------------------------------------------------------------------------");
            for p in 0..b.port_count() {
                if let Some(s) = b.lacp_member_status(p) {
                    o.line(format!(
                        "eth{:<3}  {:<4}  {}  {}  {}  {:<5}  {:<3}  {:<9}  {:<6}  {}",
                        p,
                        s.lag,
                        state_string(s.actor.state),
                        state_string(s.partner.state),
                        fmt_mac(&s.partner.system),
                        s.partner.key,
                        if s.selected { "yes" } else { "no" },
                        s.mux.as_str(),
                        s.tx_count,
                        s.rx_count
                    ));
                }
            }
        }
        _ => o.err("Usage: show lacp [interfaces|aggregates|configuration]"),
    }
}

fn show_interface_lag(b: &Bridge, o: &mut Out, id: &str) {
    let id: u16 = match id.parse() {
        Ok(v) => v,
        Err(_) => return o.err("Invalid LAG id"),
    };
    let c = match b.lag_config(id) {
        Some(c) => c,
        None => return o.err("LAG not configured"),
    };
    o.line(format!("LAG {}:", id));
    let mode = match &c.mode {
        PortMode::Routed => "routed".to_string(),
        PortMode::Access { vid } => format!("access vlan {}", vid),
        PortMode::Trunk { native, native_tag, allowed } => format!(
            "trunk native {}{} allowed {}",
            native,
            if *native_tag { " tagged-native" } else { "" },
            match allowed {
                AllowedList::All => "all".to_string(),
                AllowedList::Some(s) => format_vlan_list(s),
            }
        ),
    };
    o.line(format!("  Mode       : {}", mode));
    o.line(format!("  Admin      : {}{}", if c.admin_up { "up" } else { "down" }, if b.is_err_disabled(PortRef::Lag(id)) { " (err-disabled)" } else { "" }));
    o.line(format!("  Oper       : {}", if b.lag_oper_up(id) { "up" } else { "down" }));
    o.line(format!("  LACP Mode  : {}", mode_str(c.lacp_mode)));
    o.line(format!("  LACP Rate  : {}", rate_str(c.lacp_rate)));
    o.line(format!("  Hash       : {}", c.hash.as_str()));
    o.line(format!("  Fallback   : {}", c.fallback));
    o.line(format!("  Members    : {}", eth_list(&b.lag_members(id))));
    o.line(format!("  Up members : {}", eth_list(&b.lag_distributing_members(id))));
}

fn show_interface_vlan(b: &Bridge, o: &mut Out) {
    let svis = b.svis();
    if svis.is_empty() {
        o.line("(no VLAN interfaces)");
        return;
    }
    o.line("Interface   Address             Status");
    for (v, ip, p) in svis {
        o.line(format!("vlan{:<6}  {:<18}  {}", v, format!("{}/{}", fmt_ip(&ip), p), if b.vlan(v).map(|x| x.active).unwrap_or(false) { "up" } else { "down" }));
    }
}

fn show_interface_brief(b: &Bridge, o: &mut Out) {
    o.line("Port    Link  Admin  Mode     LAG    Forwarding");
    o.line("-----------------------------------------------");
    for i in 0..b.port_count() {
        let c = match b.eth_config(i) {
            Some(c) => c,
            None => continue,
        };
        let mode = match (&c.lag, b.is_l2_port(i), &c.mode) {
            (Some(_), true, _) => "lag-l2",
            (Some(_), false, _) => "lag-l3",
            (None, _, PortMode::Routed) => "routed",
            (None, _, PortMode::Access { .. }) => "access",
            (None, _, PortMode::Trunk { .. }) => "trunk",
        };
        let bp = match c.lag {
            Some(id) => PortRef::Lag(id),
            None => PortRef::Eth(i),
        };
        let fwd = if !b.is_l2_port(i) {
            "-"
        } else if b.is_err_disabled(bp) {
            "err-disabled"
        } else if b.bridge_port_forwarding(bp) {
            "yes"
        } else {
            "no"
        };
        o.line(format!(
            "eth{:<3}  {:<4}  {:<5}  {:<7}  {:<5}  {}",
            i,
            if b.link_up(i) { "up" } else { "down" },
            if c.admin_up { "up" } else { "down" },
            mode,
            c.lag.map(|l| format!("lag{}", l)).unwrap_or_else(|| "-".into()),
            fwd
        ));
    }
}

// ---------------------------------------------------------------------
// clear
// ---------------------------------------------------------------------

pub(super) fn clear(b: &mut Bridge, o: &mut Out, args: &[&str]) {
    match args {
        ["logging" | "events"] => {
            b.clear_log();
            o.line("Log buffer cleared.");
        }
        ["lldp", "neighbors"] => {
            b.clear_lldp_neighbors();
            o.line("LLDP neighbors cleared.");
        }
        ["lldp", "statistics"] => {
            b.clear_lldp_stats();
            o.line("LLDP statistics cleared.");
        }
        ["mac-address-table", "dynamic", rest @ ..] => {
            let mut f = FdbFilter::default();
            let what = match rest {
                [] => String::new(),
                ["vlan", v] => match v.parse::<u16>() {
                    Ok(x) => {
                        f.vlan = Some(x);
                        format!(" in VLAN {}", x)
                    }
                    Err(_) => return o.err("Invalid VLAN id"),
                },
                ["port", p] => match parse_port_ref(p, b.port_count()) {
                    Some(x) => {
                        f.port = Some(x);
                        format!(" on port {}", port_col(x))
                    }
                    None => return o.err("Invalid port number"),
                },
                ["address", m] => match parse_mac(m) {
                    Some(x) => {
                        f.mac = Some(x);
                        format!(" matching {}", fmt_mac(&x))
                    }
                    None => return o.err("Invalid MAC address"),
                },
                _ => return o.err("Usage: clear mac-address-table dynamic [vlan <id> | port <n> | address <mac>]"),
            };
            let n = b.clear_dynamic(f);
            o.line(format!("Cleared {} dynamic entries{}.", n, what));
        }
        ["lldp", ..] => o.err("Usage: clear lldp [neighbors|statistics]"),
        _ => o.err("Usage: clear mac-address-table dynamic [vlan <id> | port <n> | address <mac>] | clear logging | clear lldp ..."),
    }
}
