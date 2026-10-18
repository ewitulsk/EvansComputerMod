//! celld's control socket: UDP on `127.0.0.1:7710` (the loopback, so only
//! programs on the phone itself reach it). One datagram is one request line;
//! the answer is one datagram of text lines.
//!
//! | Request | Answer |
//! |---|---|
//! | `status` | `key value` lines ([`Status`]) |
//! | `cells` | one line per cell heard in the last 10 s ([`CellRow`]) |
//! | `up` / `down` | `ok ...`: attach (again) / detach and keep the radio quiet |
//! | `roaming on` / `roaming off` | `ok ...`, saved to `/etc/celld.toml` |
//!
//! Anything else: `error <why>`. Clients: the `cell` command, Settings ->
//! Cellular. The phone's status bar gets its bars from celld through
//! `/dev/uictl cell <bars> <carrier>` instead (the shell's existing source).

use std::fmt::Write;

pub const PORT: u16 = 7710;
pub const LOOPBACK: [u8; 4] = [127, 0, 0, 1];

/// Signal bars 0-4 from the serving cell's received power, as phones show
/// them (none unless the phone is on a cell).
pub fn bars(rssi_dbm: Option<f64>, on_cell: bool) -> u8 {
    match (on_cell, rssi_dbm) {
        (true, Some(r)) if r >= -80.0 => 4,
        (true, Some(r)) if r >= -90.0 => 3,
        (true, Some(r)) if r >= -100.0 => 2,
        (true, Some(r)) if r >= -110.0 => 1,
        _ => 0,
    }
}

/// What celld reports (`status`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// `no_modem`, `no_sim`, `locked`, `down`, `searching`, `waiting`,
    /// `attaching`, `connected`, `handover`, `idle`, `detaching`, `rejected`.
    pub state: String,
    /// A sentence for people: why, and what to do.
    pub message: String,
    pub modem: String,
    pub profile: String,
    pub sim_carrier: String,
    pub imsi: String,
    /// The carrier of the serving cell ("" when on none).
    pub carrier: String,
    pub cell: Option<u32>,
    pub channel: u8,
    pub rssi: Option<f64>,
    pub sinr: Option<f64>,
    pub cqi: Option<u8>,
    pub bars: u8,
    /// `a.b.c.d/len`.
    pub ip: String,
    pub gateway: String,
    pub dns: String,
    pub handovers: u32,
    pub attaches: u32,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub roaming: bool,
    /// The phone's number (from the network, docs/TELEPHONY.md).
    pub msisdn: String,
    /// The IMS bearer's address (`a.b.c.d/len`), its P-CSCF and the carrier's SIP domain.
    pub ims_ip: String,
    pub pcscf: String,
    pub ims_domain: String,
    /// Voice bearers up (a call).
    pub bearers: u32,
    /// The paging identity (0: none).
    pub tmsi: u32,
    /// The last cause from the network (`authentication failed`, ...).
    pub cause: String,
    pub uptime_s: u64,
}

fn opt<T: ToString>(v: &Option<T>) -> String {
    v.as_ref().map(|x| x.to_string()).unwrap_or_else(|| "-".into())
}

fn text(v: &str) -> &str {
    if v.is_empty() {
        "-"
    } else {
        v
    }
}

impl Status {
    pub fn format(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "state {}", self.state);
        let _ = writeln!(s, "message {}", text(&self.message));
        let _ = writeln!(s, "modem {}", text(&self.modem));
        let _ = writeln!(s, "profile {}", text(&self.profile));
        let _ = writeln!(s, "sim_carrier {}", text(&self.sim_carrier));
        let _ = writeln!(s, "imsi {}", text(&self.imsi));
        let _ = writeln!(s, "carrier {}", text(&self.carrier));
        let _ = writeln!(s, "cell {}", opt(&self.cell));
        let _ = writeln!(s, "channel {}", self.channel);
        let _ = writeln!(s, "rssi {}", self.rssi.map(|r| format!("{r:.1}")).unwrap_or_else(|| "-".into()));
        let _ = writeln!(s, "sinr {}", self.sinr.map(|r| format!("{r:.1}")).unwrap_or_else(|| "-".into()));
        let _ = writeln!(s, "cqi {}", opt(&self.cqi));
        let _ = writeln!(s, "bars {}", self.bars);
        let _ = writeln!(s, "ip {}", text(&self.ip));
        let _ = writeln!(s, "gateway {}", text(&self.gateway));
        let _ = writeln!(s, "dns {}", text(&self.dns));
        let _ = writeln!(s, "handovers {}", self.handovers);
        let _ = writeln!(s, "attaches {}", self.attaches);
        let _ = writeln!(s, "tx_bytes {}", self.tx_bytes);
        let _ = writeln!(s, "rx_bytes {}", self.rx_bytes);
        let _ = writeln!(s, "tx_packets {}", self.tx_packets);
        let _ = writeln!(s, "rx_packets {}", self.rx_packets);
        let _ = writeln!(s, "roaming {}", if self.roaming { "on" } else { "off" });
        let _ = writeln!(s, "msisdn {}", text(&self.msisdn));
        let _ = writeln!(s, "ims_ip {}", text(&self.ims_ip));
        let _ = writeln!(s, "pcscf {}", text(&self.pcscf));
        let _ = writeln!(s, "ims_domain {}", text(&self.ims_domain));
        let _ = writeln!(s, "bearers {}", self.bearers);
        let _ = writeln!(s, "tmsi {}", self.tmsi);
        let _ = writeln!(s, "cause {}", text(&self.cause));
        let _ = writeln!(s, "uptime {}", self.uptime_s);
        s
    }

    pub fn parse(t: &str) -> Status {
        let mut s = Status::default();
        for line in t.lines() {
            let (k, v) = line.split_once(' ').unwrap_or((line, ""));
            let v = v.trim();
            let st = || if v == "-" { String::new() } else { v.to_string() };
            match k {
                "state" => s.state = st(),
                "message" => s.message = st(),
                "modem" => s.modem = st(),
                "profile" => s.profile = st(),
                "sim_carrier" => s.sim_carrier = st(),
                "imsi" => s.imsi = st(),
                "carrier" => s.carrier = st(),
                "cell" => s.cell = v.parse().ok(),
                "channel" => s.channel = v.parse().unwrap_or(0),
                "rssi" => s.rssi = v.parse().ok(),
                "sinr" => s.sinr = v.parse().ok(),
                "cqi" => s.cqi = v.parse().ok(),
                "bars" => s.bars = v.parse().unwrap_or(0),
                "ip" => s.ip = st(),
                "gateway" => s.gateway = st(),
                "dns" => s.dns = st(),
                "handovers" => s.handovers = v.parse().unwrap_or(0),
                "attaches" => s.attaches = v.parse().unwrap_or(0),
                "tx_bytes" => s.tx_bytes = v.parse().unwrap_or(0),
                "rx_bytes" => s.rx_bytes = v.parse().unwrap_or(0),
                "tx_packets" => s.tx_packets = v.parse().unwrap_or(0),
                "rx_packets" => s.rx_packets = v.parse().unwrap_or(0),
                "roaming" => s.roaming = v == "on",
                "cause" => s.cause = st(),
                "msisdn" => s.msisdn = st(),
                "ims_ip" => s.ims_ip = st(),
                "pcscf" => s.pcscf = st(),
                "ims_domain" => s.ims_domain = st(),
                "bearers" => s.bearers = v.parse().unwrap_or(0),
                "tmsi" => s.tmsi = v.parse().unwrap_or(0),
                "uptime" => s.uptime_s = v.parse().unwrap_or(0),
                _ => {}
            }
        }
        s
    }

    /// On a cell (attached, idle on it, or handing over).
    pub fn on_cell(&self) -> bool {
        matches!(self.state.as_str(), "connected" | "idle" | "handover")
    }
}

/// One cell heard (`cells`).
#[derive(Clone, Debug, PartialEq)]
pub struct CellRow {
    pub cell: u32,
    pub carrier: String,
    pub profile: String,
    pub channel: u8,
    pub rssi: f64,
    pub sinr: f64,
    pub age_ms: u64,
    pub serving: bool,
    /// What keeps this phone off it: `other carrier`, `core down`, `barred` ("" = usable).
    pub note: String,
}

impl CellRow {
    pub fn format(&self) -> String {
        format!(
            "cell {} carrier {} profile {} channel {} rssi {:.1} sinr {:.1} age {}{}{}",
            self.cell,
            self.carrier,
            self.profile,
            self.channel,
            self.rssi,
            self.sinr,
            self.age_ms,
            if self.serving { " serving" } else { "" },
            if self.note.is_empty() { String::new() } else { format!(" note {}", self.note.replace(' ', "_")) }
        )
    }

    pub fn parse(line: &str) -> Option<CellRow> {
        let w: Vec<&str> = line.split_whitespace().collect();
        let get = |k: &str| w.iter().position(|x| *x == k).and_then(|i| w.get(i + 1)).copied();
        Some(CellRow {
            cell: get("cell")?.parse().ok()?,
            carrier: get("carrier")?.to_string(),
            profile: get("profile").unwrap_or("lte").to_string(),
            channel: get("channel")?.parse().ok()?,
            rssi: get("rssi")?.parse().ok()?,
            sinr: get("sinr")?.parse().ok()?,
            age_ms: get("age").and_then(|a| a.parse().ok()).unwrap_or(0),
            serving: w.contains(&"serving"),
            note: get("note").map(|n| n.replace('_', " ")).unwrap_or_default(),
        })
    }
}

pub fn parse_cells(t: &str) -> Vec<CellRow> {
    t.lines().filter_map(CellRow::parse).collect()
}

/// Bytes for people.
pub fn human_bytes(n: u64) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{:.1} KB", n as f64 / 1024.0),
        n => format!("{:.1} MB", n as f64 / (1024.0 * 1024.0)),
    }
}

/// `[**  ]`
pub fn bar_text(bars: u8) -> String {
    let b = bars.min(4) as usize;
    format!("[{}{}]", "*".repeat(b), " ".repeat(4 - b))
}

/// The `cell` command's status page.
pub fn render_status(s: &Status) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "{}: {} modem, celld", text(&s.modem), if s.profile.is_empty() { "cellular".to_string() } else { s.profile.to_uppercase() });
    let _ = writeln!(o, "  state     {} - {}", s.state, text(&s.message));
    if s.imsi.is_empty() {
        let _ = writeln!(o, "  SIM       none");
    } else {
        let _ = writeln!(o, "  SIM       {}, IMSI {}", s.sim_carrier, s.imsi);
    }
    match s.cell {
        Some(c) if s.on_cell() || s.state == "attaching" => {
            let _ = writeln!(o, "  cell      {} cell {}, channel {}", text(&s.carrier), c, s.channel);
            let _ = writeln!(
                o,
                "  signal    {} dBm, SINR {} dB, CQI {}  {}",
                s.rssi.map(|r| format!("{r:.1}")).unwrap_or_else(|| "-".into()),
                s.sinr.map(|r| format!("{r:.1}")).unwrap_or_else(|| "-".into()),
                opt(&s.cqi),
                bar_text(s.bars)
            );
        }
        _ => {
            let _ = writeln!(o, "  cell      none (channel {})", s.channel);
        }
    }
    if !s.ip.is_empty() {
        let _ = writeln!(o, "  address   {} via {}, dns {}", s.ip, text(&s.gateway), text(&s.dns));
    }
    if !s.msisdn.is_empty() {
        let _ = writeln!(o, "  number    {}", s.msisdn);
    }
    if !s.ims_ip.is_empty() {
        let calls = if s.bearers > 0 { format!(", {} voice bearer(s)", s.bearers) } else { String::new() };
        let _ = writeln!(o, "  IMS       {} to P-CSCF {} ({}){}", s.ims_ip, text(&s.pcscf), text(&s.ims_domain), calls);
    }
    let _ = writeln!(
        o,
        "  traffic   tx {} ({} packets), rx {} ({} packets), handovers {}",
        human_bytes(s.tx_bytes),
        s.tx_packets,
        human_bytes(s.rx_bytes),
        s.rx_packets,
        s.handovers
    );
    let _ = writeln!(o, "  roaming   {}", if s.roaming { "on" } else { "off" });
    if !s.cause.is_empty() {
        let _ = writeln!(o, "  last      {}", s.cause);
    }
    o
}

/// The `cell scan` table.
pub fn render_cells(rows: &[CellRow]) -> String {
    if rows.is_empty() {
        return "no cells heard (yet). celld listens on one channel at a time while it searches; try again in a few seconds.\n".into();
    }
    let mut o = format!("{:<16} {:>6} {:<7} {:>3} {:>7} {:>6}  {}\n", "CARRIER", "CELL", "PROFILE", "CH", "RSSI", "SINR", "");
    for r in rows {
        let _ = writeln!(
            o,
            "{:<16} {:>6} {:<7} {:>3} {:>7.1} {:>6.1}  {}{}",
            r.carrier,
            r.cell,
            r.profile,
            r.channel,
            r.rssi,
            r.sinr,
            if r.serving { "serving" } else { "" },
            if r.note.is_empty() { String::new() } else { format!("({})", r.note) }
        );
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_and_renders() {
        let s = Status {
            state: "connected".into(),
            message: "on cell-red".into(),
            modem: "module_1".into(),
            profile: "lte".into(),
            sim_carrier: "cell-red".into(),
            imsi: "310150000000001".into(),
            carrier: "cell-red".into(),
            cell: Some(2),
            channel: 3,
            rssi: Some(-71.26),
            sinr: Some(23.5),
            cqi: Some(15),
            bars: 4,
            ip: "100.65.2.2/24".into(),
            gateway: "100.65.2.1".into(),
            dns: String::new(),
            handovers: 1,
            attaches: 1,
            tx_bytes: 2048,
            rx_bytes: 100,
            tx_packets: 3,
            rx_packets: 2,
            roaming: false,
            msisdn: "+12145550142".into(),
            ims_ip: "10.250.0.2/16".into(),
            pcscf: "100.65.2.1".into(),
            ims_domain: "cell-red.ecm".into(),
            bearers: 1,
            tmsi: 77,
            cause: String::new(),
            uptime_s: 12,
        };
        let mut back = Status::parse(&s.format());
        assert_eq!(back.rssi, Some(-71.3), "one decimal on the wire");
        back.rssi = s.rssi;
        assert_eq!(back, s);
        let r = render_status(&s);
        assert!(r.contains("cell      cell-red cell 2, channel 3"), "{r}");
        assert!(r.contains("CQI 15  [****]"), "{r}");
        assert!(r.contains("address   100.65.2.2/24 via 100.65.2.1, dns -"), "{r}");
        assert!(r.contains("tx 2.0 KB (3 packets)"), "{r}");
        let none = render_status(&Status { state: "no_sim".into(), ..Default::default() });
        assert!(none.contains("SIM       none") && none.contains("cell      none"), "{none}");
    }

    #[test]
    fn cells_round_trip() {
        let rows = vec![
            CellRow { cell: 1, carrier: "cell-red".into(), profile: "lte".into(), channel: 1, rssi: -60.0, sinr: 30.0, age_ms: 40, serving: true, note: String::new() },
            CellRow { cell: 7, carrier: "cell-blue".into(), profile: "lte".into(), channel: 3, rssi: -90.5, sinr: 2.0, age_ms: 900, serving: false, note: "other carrier".into() },
        ];
        let text: String = rows.iter().map(|r| r.format() + "\n").collect();
        assert_eq!(parse_cells(&text), rows);
        let t = render_cells(&rows);
        assert!(t.contains("serving") && t.contains("(other carrier)"), "{t}");
        assert!(render_cells(&[]).starts_with("no cells heard"));
    }

    #[test]
    fn bars_follow_the_serving_signal() {
        assert_eq!(bars(Some(-60.0), true), 4);
        assert_eq!(bars(Some(-85.0), true), 3);
        assert_eq!(bars(Some(-95.0), true), 2);
        assert_eq!(bars(Some(-105.0), true), 1);
        assert_eq!(bars(Some(-112.0), true), 0);
        assert_eq!(bars(Some(-60.0), false), 0, "no bars off a cell");
        assert_eq!(bars(None, true), 0);
    }
}
