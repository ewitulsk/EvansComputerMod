//! `wlan0`: the Wi-Fi station interface (the kernel's mac80211 + cfg80211).
//!
//! The Wi-Fi module in a bay is a SoftMAC radio (Java low MAC: ACK, retry,
//! CSMA/CA, FCS, airtime). This adapter sits between it and `ecm-net`:
//!
//! ```text
//!  wifi_rx_frame ──► Wlan::on_rx ──► Ethernet ──► Stack (iface wlan0)
//!                                └─► radiotap  ──► monitor ring (tcpdump -i wlan0)
//!  Stack pop_tx(wlan0) ──► Wlan::send_ethernet ──► wifi_tx_frame
//!  Wlan::poll / pop_action ──► wifi_tx_frame / wifi_set_channel / wifi_set_rx_filter / events
//!  wifi_tx_status ──► Wlan::tx_status (minstrel rate control)
//! ```
//!
//! Carrier is "associated"; `Wlan` itself keeps the 802.1X port closed to
//! everything but EAPOL until keys are installed. Userspace
//! (`iw`, `wpa_supplicant`, `wpa_cli`, `tcpdump`) drives it through one text
//! control channel, [`WifiDev::ctl`], reached by the `WIFI_CTL` IPC syscall
//! (see `kernel.rs` and `ecm_host_abi::wifi`). Requests are one line:
//!
//! | request | reply |
//! | --- | --- |
//! | `status` | `key=value` lines |
//! | `scan [ssid_hex] [passive]` | status 0, or -16 busy |
//! | `scan_results` | `bssid freq signal_dbm security channel ssid_hex` lines |
//! | `connect <ssid_hex> open\|wpa2 [bssid]` / `disconnect [reason]` | status |
//! | `link` | `iw dev wlan0 link` text |
//! | `set_type monitor\|managed`, `set_channel <n>`, `set_power <dbm>` | status |
//! | `install_ptk <bssid> <tk_hex>`, `install_gtk <id> <rsc> <gtk_hex>`, `clear_keys` | status |
//! | `events <after_seq>` | event lines `<seq> <NAME> key=value...` |
//! | `eapol_subscribe` / `eapol_unsubscribe` / `eapol_rx` / `eapol_tx <eth_hex>` | EAPOL fallback when no `AF_PACKET` |
//! | `mon_read` | one radiotap + 802.11 frame (binary), status 1, or status 0 when empty |
//! | `stats` | counters |

use std::collections::VecDeque;

use ecm_wifi::frame::{self, MacAddr};
use ecm_wifi::mlme::{ConnectFailure, DisconnectReason};
use ecm_wifi::rate::{Rate, RATES};
use ecm_wifi::{radiotap, Action, ConnectParams, ConnectSecurity, Event, LinkState, MlmeConfig, RxFilter, ScanRequest, Wlan};

use crate::hal;
pub use crate::hal::wifi::{RxMeta, TxStatus};

/// The Wi-Fi radio as the kernel sees it. `HostWifi` calls the host; tests
/// plug in an in-memory radio.
pub trait WifiRadio {
    /// Number of radios (0 = no Wi-Fi module installed).
    fn present(&self) -> usize;
    fn mac(&self) -> Option<MacAddr>;
    fn tx(&mut self, frame: &[u8], rate_kbps: u32, power_dbm: i8) -> bool;
    fn rx(&mut self, buf: &mut [u8]) -> Option<(usize, RxMeta)>;
    fn set_channel(&mut self, ch: u8) -> bool;
    fn set_rx_filter(&mut self, mode: i32, bssid: Option<MacAddr>) -> bool;
    fn tx_status(&mut self) -> Option<TxStatus>;
}

/// The host's Wi-Fi module (`wifi_*` imports).
pub struct HostWifi;

impl WifiRadio for HostWifi {
    fn present(&self) -> usize {
        hal::wifi::present()
    }
    fn mac(&self) -> Option<MacAddr> {
        hal::wifi::mac()
    }
    fn tx(&mut self, frame: &[u8], rate_kbps: u32, power_dbm: i8) -> bool {
        hal::wifi::tx(frame, rate_kbps, power_dbm)
    }
    fn rx(&mut self, buf: &mut [u8]) -> Option<(usize, RxMeta)> {
        hal::wifi::rx(buf)
    }
    fn set_channel(&mut self, ch: u8) -> bool {
        hal::wifi::set_channel(ch)
    }
    fn set_rx_filter(&mut self, mode: i32, bssid: Option<MacAddr>) -> bool {
        hal::wifi::set_rx_filter(mode, bssid)
    }
    fn tx_status(&mut self) -> Option<TxStatus> {
        hal::wifi::tx_status()
    }
}

/// No radio (host tests that don't care about Wi-Fi).
pub struct NoWifi;

impl WifiRadio for NoWifi {
    fn present(&self) -> usize {
        0
    }
    fn mac(&self) -> Option<MacAddr> {
        None
    }
    fn tx(&mut self, _: &[u8], _: u32, _: i8) -> bool {
        false
    }
    fn rx(&mut self, _: &mut [u8]) -> Option<(usize, RxMeta)> {
        None
    }
    fn set_channel(&mut self, _: u8) -> bool {
        false
    }
    fn set_rx_filter(&mut self, _: i32, _: Option<MacAddr>) -> bool {
        false
    }
    fn tx_status(&mut self) -> Option<TxStatus> {
        None
    }
}

pub const RX_FILTER_NORMAL: i32 = 0;
pub const RX_FILTER_PROMISC: i32 = 1;
pub const RX_FILTER_MONITOR: i32 = 2;

/// The interface name userspace uses.
pub const IFNAME: &str = "wlan0";
/// How often `wifi_present` is re-sampled (module hot-plug).
pub const PRESENT_POLL_MS: i64 = 1000;
/// Frames drained from the radio per `rx` call.
const RX_BUDGET: usize = 256;
const MAX_EVENTS: usize = 64;
const MAX_EAPOL: usize = 16;
const MAX_MONITOR: usize = 512;
const MAX_FRAME: usize = 2400;

pub const EBUSY: i32 = -16;
pub const EINVAL: i32 = -22;
pub const ENODEV: i32 = -19;
pub const ENOTCONN: i32 = -107;

/// Rate code (500 kb/s units, HT = 0x80|MCS) for a bit rate in kb/s.
pub fn rate_code_for_kbps(kbps: u32) -> Option<u8> {
    RATES.iter().find(|r| r.kbps == kbps).map(|r| r.code)
}

pub fn kbps_for_rate_code(code: u8) -> Option<u32> {
    Rate::from_code(code).map(|r| r.kbps)
}

/// Output of [`WifiDev::rx`] for the network stack.
pub struct RxResult {
    /// Ethernet frames for the stack on `wlan0`.
    pub ethernet: Vec<Vec<u8>>,
}

/// One Wi-Fi radio presented as `wlan0`.
pub struct WifiDev {
    radio: Box<dyn WifiRadio>,
    wlan: Option<Wlan>,
    mac: MacAddr,
    /// Stack interface index of `wlan0` once added.
    pub iface: Option<usize>,
    present: bool,
    next_present_poll: i64,
    tx_power_dbm: i8,
    events: VecDeque<(u64, String)>,
    next_event_seq: u64,
    eapol_sub: bool,
    eapol_rx: VecDeque<Vec<u8>>,
    monitor_rx: VecDeque<Vec<u8>>,
    pub monitor_dropped: u64,
    /// RX filter last programmed (for `status`).
    filter: RxFilter,
    last_scan_results: usize,
}

fn now_u(now: i64) -> u64 {
    now.max(0) as u64
}

pub fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{:02x}", x));
    }
    s
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.as_bytes();
    if s.len() % 2 != 0 {
        return None;
    }
    let v = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(s.len() / 2);
    for c in s.chunks(2) {
        out.push(v(c[0])? << 4 | v(c[1])?);
    }
    Some(out)
}

pub fn parse_mac(s: &str) -> Option<MacAddr> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut m = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        m[i] = u8::from_str_radix(p, 16).ok()?;
    }
    Some(m)
}

fn failure_str(f: &ConnectFailure) -> String {
    match f {
        ConnectFailure::NoBss => "no_bss".into(),
        ConnectFailure::AuthTimeout => "auth_timeout".into(),
        ConnectFailure::AuthRejected(c) => format!("auth_rejected:{}", c),
        ConnectFailure::AssocTimeout => "assoc_timeout".into(),
        ConnectFailure::AssocRejected(c) => format!("assoc_rejected:{}", c),
        ConnectFailure::Deauthenticated(c) => format!("deauth:{}", c),
    }
}

fn state_str(s: &LinkState) -> &'static str {
    match s {
        LinkState::Idle => "IDLE",
        LinkState::Searching => "SCANNING",
        LinkState::Authenticating { .. } => "AUTHENTICATING",
        LinkState::Associating { .. } => "ASSOCIATING",
        LinkState::Connected { .. } => "ASSOCIATED",
        LinkState::ReconnectWait { .. } => "DISCONNECTED",
        LinkState::Monitor => "MONITOR",
    }
}

impl WifiDev {
    pub fn new(radio: Box<dyn WifiRadio>, now: i64) -> Self {
        let mut d = WifiDev {
            radio,
            wlan: None,
            mac: [0; 6],
            iface: None,
            present: false,
            next_present_poll: now,
            tx_power_dbm: 20,
            events: VecDeque::new(),
            next_event_seq: 1,
            eapol_sub: false,
            eapol_rx: VecDeque::new(),
            monitor_rx: VecDeque::new(),
            monitor_dropped: 0,
            filter: RxFilter::Idle,
            last_scan_results: 0,
        };
        d.sample_present(now);
        d
    }

    pub fn present(&self) -> bool {
        self.present
    }
    pub fn mac(&self) -> MacAddr {
        self.mac
    }
    pub fn wlan(&self) -> Option<&Wlan> {
        self.wlan.as_ref()
    }

    /// Re-read `wifi_present`. Returns true when the presence changed.
    pub fn sample_present(&mut self, now: i64) -> bool {
        self.next_present_poll = now + PRESENT_POLL_MS;
        let p = self.radio.present() > 0;
        if p == self.present {
            return false;
        }
        self.present = p;
        if p {
            let mac = self.radio.mac().unwrap_or([0x02, 0xec, 0x57, 0x1f, 0, 1]);
            let mut cfg = MlmeConfig::new(mac);
            cfg.tx_power_dbm = self.tx_power_dbm;
            self.mac = mac;
            self.wlan = Some(Wlan::new(cfg));
            // Program the radio for the idle station.
            let ch = self.wlan.as_ref().map(|w| w.mlme().channel()).unwrap_or(1);
            self.radio.set_channel(ch);
            self.apply_filter(RxFilter::Idle);
            self.push_event("PRESENT".to_string());
        } else {
            self.wlan = None;
            self.eapol_rx.clear();
            self.monitor_rx.clear();
            self.push_event("REMOVED".to_string());
        }
        true
    }

    pub fn present_poll_due(&self, now: i64) -> bool {
        now >= self.next_present_poll
    }

    /// Carrier: associated and not in monitor mode (as Linux reports it). Until
    /// the 802.1X port is authorized `Wlan` passes only EAPOL, so the
    /// supplicant's packet socket can run the handshake over `wlan0`.
    pub fn carrier(&self) -> bool {
        self.wlan.as_ref().is_some_and(|w| w.mlme().current_bss().is_some() && !w.is_monitor())
    }

    fn push_event(&mut self, text: String) {
        let seq = self.next_event_seq;
        self.next_event_seq += 1;
        self.events.push_back((seq, text));
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
    }

    fn apply_filter(&mut self, f: RxFilter) {
        self.filter = f;
        match f {
            RxFilter::Idle | RxFilter::Scan => self.radio.set_rx_filter(RX_FILTER_NORMAL, None),
            RxFilter::Bssid(b) => self.radio.set_rx_filter(RX_FILTER_NORMAL, Some(b)),
            RxFilter::Monitor => self.radio.set_rx_filter(RX_FILTER_MONITOR, None),
        };
    }

    fn transmit(&mut self, frame: &[u8], rate: u8, power_dbm: i8) {
        let kbps = kbps_for_rate_code(rate).unwrap_or(1000);
        self.radio.tx(frame, kbps, power_dbm);
    }

    /// Drain MLME actions into the radio and the event log.
    pub fn process_actions(&mut self) {
        loop {
            let Some(a) = self.wlan.as_mut().and_then(|w| w.pop_action()) else { break };
            match a {
                Action::Transmit { frame, rate, power_dbm } => self.transmit(&frame, rate, power_dbm),
                Action::SetChannel(ch) => {
                    self.radio.set_channel(ch);
                }
                Action::SetRxFilter(f) => self.apply_filter(f),
                Action::Event(e) => self.on_event(e),
            }
        }
    }

    fn on_event(&mut self, e: Event) {
        let text = match e {
            Event::ScanDone { results, aborted } => {
                self.last_scan_results = results;
                format!("SCAN_DONE results={} aborted={}", results, aborted as u8)
            }
            Event::Connected { bssid, ssid, channel, aid, ap_rsn_ie, sta_rsn_ie, roamed } => format!(
                "CONNECTED bssid={} ssid={} channel={} aid={} ap_rsn={} sta_rsn={} roamed={}",
                frame::mac_str(&bssid),
                hex(&ssid),
                channel,
                aid,
                ap_rsn_ie.as_deref().map(hex).unwrap_or_else(|| "-".into()),
                sta_rsn_ie.as_deref().map(hex).unwrap_or_else(|| "-".into()),
                roamed as u8
            ),
            Event::ConnectFailed { bssid, reason } => format!(
                "CONNECT_FAILED bssid={} reason={}",
                bssid.map(|b| frame::mac_str(&b)).unwrap_or_else(|| "-".into()),
                failure_str(&reason)
            ),
            Event::Disconnected { bssid, reason } => {
                let local = reason.locally_generated();
                let kind = match reason {
                    DisconnectReason::Deauth(_) => "deauth",
                    DisconnectReason::Disassoc(_) => "disassoc",
                    DisconnectReason::BeaconLoss => "beacon_loss",
                    DisconnectReason::Local(_) => "local",
                };
                format!(
                    "DISCONNECTED bssid={} reason={} kind={} local={}",
                    frame::mac_str(&bssid),
                    reason.code(),
                    kind,
                    local as u8
                )
            }
        };
        self.push_event(text);
    }

    /// Drain received frames and transmit statuses (IRQ_WIFI).
    pub fn rx(&mut self, now: i64) -> RxResult {
        let mut out = RxResult { ethernet: Vec::new() };
        if self.wlan.is_none() {
            return out;
        }
        let t = now_u(now);
        let mut buf = vec![0u8; MAX_FRAME];
        for _ in 0..RX_BUDGET {
            let Some((len, meta)) = self.radio.rx(&mut buf) else { break };
            let f = &buf[..len];
            let rssi = meta.rssi_dbm_x10.div_euclid(10);
            let code = rate_code_for_kbps(meta.rate_kbps).unwrap_or(2);
            let Some(w) = self.wlan.as_mut() else { break };
            if w.is_monitor() {
                let ts = if meta.timestamp_us > 0 { meta.timestamp_us as u64 } else { t * 1000 };
                let rt = radiotap::encapsulate(f, ts, code, meta.channel, rssi.clamp(-128, 127) as i8, false);
                self.push_monitor(rt);
                continue;
            }
            match w.on_rx(f, rssi, code, meta.channel, t) {
                Some(ecm_wifi::RxOutput::Ethernet(eth)) => {
                    if self.eapol_sub && frame::ethertype(&eth) == Some(frame::ETHERTYPE_EAPOL) && self.eapol_rx.len() < MAX_EAPOL {
                        self.eapol_rx.push_back(eth.clone());
                    }
                    out.ethernet.push(eth);
                }
                Some(ecm_wifi::RxOutput::Monitor(rt)) => self.push_monitor(rt),
                None => {}
            }
        }
        self.drain_tx_status(now);
        self.process_actions();
        out
    }

    fn push_monitor(&mut self, rt: Vec<u8>) {
        if self.monitor_rx.len() >= MAX_MONITOR {
            self.monitor_rx.pop_front();
            self.monitor_dropped += 1;
        }
        self.monitor_rx.push_back(rt);
    }

    /// Feed the SoftMAC's per-frame transmit status into rate control.
    pub fn drain_tx_status(&mut self, now: i64) {
        for _ in 0..RX_BUDGET {
            let Some(st) = self.radio.tx_status() else { break };
            let Some(w) = self.wlan.as_mut() else { continue };
            // Only data frames are rate-controlled (fc type bits 2..3 = 2).
            if (st.frame_control >> 2) & 3 != frame::TYPE_DATA as u16 {
                continue;
            }
            if let Some(code) = rate_code_for_kbps(st.rate_kbps) {
                w.tx_status(code, st.attempts.max(1), st.acked, now_u(now));
            }
        }
    }

    /// An Ethernet frame the stack queued on `wlan0`.
    pub fn send_ethernet(&mut self, eth: &[u8], now: i64) -> bool {
        let Some(w) = self.wlan.as_mut() else { return false };
        match w.send_ethernet(eth, now_u(now)) {
            Some(tx) => {
                self.transmit(&tx.frame, tx.rate, tx.power_dbm);
                true
            }
            None => false,
        }
    }

    /// Timers. Returns the next deadline (ms).
    pub fn poll(&mut self, now: i64) -> Option<i64> {
        let d = self.wlan.as_mut().and_then(|w| w.poll(now_u(now)));
        self.drain_tx_status(now);
        self.process_actions();
        let p = Some(self.next_present_poll);
        match d {
            Some(d) => Some((d.min(i64::MAX as u64) as i64).min(self.next_present_poll)),
            None => p,
        }
    }

    // ------------------------------------------------------------ control

    /// Handle one control request. Returns (status, reply bytes).
    pub fn ctl(&mut self, req: &[u8], now: i64) -> (i32, Vec<u8>) {
        let t = now_u(now);
        let line = String::from_utf8_lossy(req).trim().to_string();
        let parts: Vec<&str> = line.split_whitespace().collect();
        let Some(&cmd) = parts.first() else { return (EINVAL, b"empty request".to_vec()) };
        if cmd == "status" && self.wlan.is_none() {
            return (0, b"present=0\n".to_vec());
        }
        if cmd == "events" {
            let after: u64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let mut s = String::new();
            for (seq, e) in &self.events {
                if *seq > after && s.len() < 6000 {
                    s += &format!("{} {}\n", seq, e);
                }
            }
            return (0, s.into_bytes());
        }
        if self.wlan.is_none() {
            return (ENODEV, b"no Wi-Fi module".to_vec());
        }
        let r = match (cmd, &parts[1..]) {
            ("status", _) => (0, self.status(t).into_bytes()),
            ("scan", rest) => {
                let mut req = ScanRequest::default();
                for a in rest {
                    if *a == "passive" {
                        req.passive = true;
                    } else if let Some(b) = unhex(a) {
                        req.ssid = Some(b);
                    }
                }
                let w = self.wlan.as_mut().unwrap();
                if w.scan(req, t) {
                    (0, Vec::new())
                } else {
                    (EBUSY, b"scan already running or interface in monitor mode".to_vec())
                }
            }
            ("scan_results", _) => {
                let w = self.wlan.as_ref().unwrap();
                let mut s = String::new();
                for b in w.mlme().scan_results(t) {
                    let sec = match b.security() {
                        frame::Security::Open => "open",
                        frame::Security::Wpa2Psk => "wpa2-psk",
                        frame::Security::Unsupported => "unsupported",
                    };
                    let line = format!(
                        "{} {} {} {} {} {}\n",
                        frame::mac_str(&b.bssid),
                        frame::channel_to_freq(b.channel),
                        b.rssi,
                        sec,
                        b.channel,
                        if b.ssid.is_empty() { "-".to_string() } else { hex(&b.ssid) }
                    );
                    if s.len() + line.len() > 7000 {
                        break;
                    }
                    s += &line;
                }
                (0, s.into_bytes())
            }
            ("connect", [ssid, sec, rest @ ..]) => {
                let Some(ssid) = unhex(ssid) else { return (EINVAL, b"bad ssid".to_vec()) };
                let security = match *sec {
                    "open" => ConnectSecurity::Open,
                    "wpa2" => ConnectSecurity::Wpa2Psk,
                    _ => return (EINVAL, b"security must be open or wpa2".to_vec()),
                };
                let mut p = ConnectParams::new(&ssid, security);
                if let Some(b) = rest.first() {
                    match parse_mac(b) {
                        Some(m) => p.bssid = Some(m),
                        None => return (EINVAL, b"bad bssid".to_vec()),
                    }
                }
                let w = self.wlan.as_mut().unwrap();
                if w.is_monitor() {
                    return (EBUSY, b"interface is in monitor mode".to_vec());
                }
                w.connect(p, t);
                (0, Vec::new())
            }
            ("disconnect", rest) => {
                let reason = rest.first().and_then(|s| s.parse().ok()).unwrap_or(frame::REASON_DEAUTH_LEAVING);
                self.wlan.as_mut().unwrap().disconnect(reason, t);
                (0, Vec::new())
            }
            ("link", _) => {
                let w = self.wlan.as_ref().unwrap();
                match w.link_info(t) {
                    Some(l) => (0, l.format_iw().into_bytes()),
                    None => (0, b"Not connected.\n".to_vec()),
                }
            }
            ("set_type", [kind]) => {
                let on = match *kind {
                    "monitor" => true,
                    "managed" | "station" => false,
                    _ => return (EINVAL, b"type must be monitor or managed".to_vec()),
                };
                self.wlan.as_mut().unwrap().set_monitor(on, t);
                if !on {
                    self.monitor_rx.clear();
                }
                (0, Vec::new())
            }
            ("set_channel", [ch]) => {
                let Ok(ch) = ch.parse::<u8>() else { return (EINVAL, b"bad channel".to_vec()) };
                if !(1..=14).contains(&ch) && !(32..=177).contains(&ch) {
                    return (EINVAL, b"bad channel".to_vec());
                }
                if self.wlan.as_mut().unwrap().mlme_mut().set_channel_manual(ch) {
                    (0, Vec::new())
                } else {
                    (EBUSY, b"interface is busy (connected or scanning)".to_vec())
                }
            }
            ("set_power", [dbm]) => {
                let Ok(dbm) = dbm.parse::<i8>() else { return (EINVAL, b"bad power".to_vec()) };
                let dbm = dbm.clamp(0, 20);
                self.tx_power_dbm = dbm;
                self.wlan.as_mut().unwrap().mlme_mut().config_mut().tx_power_dbm = dbm;
                (0, Vec::new())
            }
            ("install_ptk", [bssid, tk]) => {
                let (Some(b), Some(k)) = (parse_mac(bssid), unhex(tk)) else { return (EINVAL, b"bad key".to_vec()) };
                if k.len() != 16 {
                    return (EINVAL, b"TK must be 16 bytes".to_vec());
                }
                let mut tk = [0u8; 16];
                tk.copy_from_slice(&k);
                let ok = self.wlan.as_mut().unwrap().install_ptk(b, tk);
                (if ok { 0 } else { 1 }, Vec::new())
            }
            ("install_gtk", [id, rsc, gtk]) => {
                let (Ok(id), Ok(rsc), Some(g)) = (id.parse::<u8>(), rsc.parse::<u64>(), unhex(gtk)) else {
                    return (EINVAL, b"bad key".to_vec());
                };
                let ok = self.wlan.as_mut().unwrap().install_gtk(id, &g, rsc);
                (if ok { 0 } else { 1 }, Vec::new())
            }
            ("clear_keys", _) => {
                self.wlan.as_mut().unwrap().clear_keys();
                (0, Vec::new())
            }
            ("eapol_subscribe", _) => {
                self.eapol_sub = true;
                self.eapol_rx.clear();
                (0, Vec::new())
            }
            ("eapol_unsubscribe", _) => {
                self.eapol_sub = false;
                self.eapol_rx.clear();
                (0, Vec::new())
            }
            ("eapol_rx", _) => match self.eapol_rx.pop_front() {
                Some(f) => (1, f),
                None => (0, Vec::new()),
            },
            ("eapol_tx", [h]) => {
                let Some(eth) = unhex(h) else { return (EINVAL, b"bad frame".to_vec()) };
                if frame::ethertype(&eth) != Some(frame::ETHERTYPE_EAPOL) {
                    return (EINVAL, b"not an EAPOL frame".to_vec());
                }
                if self.send_ethernet(&eth, now) {
                    (0, Vec::new())
                } else {
                    (ENOTCONN, b"not associated".to_vec())
                }
            }
            ("mon_read", _) => match self.monitor_rx.pop_front() {
                Some(f) => (1, f),
                None => (0, Vec::new()),
            },
            ("stats", _) => {
                let s = self.wlan.as_ref().unwrap().stats().clone();
                (
                    0,
                    format!(
                        "rx_packets={}\nrx_bytes={}\ntx_packets={}\ntx_bytes={}\ntx_retries={}\ntx_failed={}\nrx_dropped_replay={}\nrx_dropped_mic={}\nrx_duplicates={}\nmonitor_queued={}\nmonitor_dropped={}\n",
                        s.rx_packets, s.rx_bytes, s.tx_packets, s.tx_bytes, s.tx_retries, s.tx_failed,
                        s.rx_dropped_replay, s.rx_dropped_mic, s.rx_duplicates, self.monitor_rx.len(), self.monitor_dropped
                    )
                    .into_bytes(),
                )
            }
            _ => (EINVAL, format!("unknown request: {}", line).into_bytes()),
        };
        // Control requests (scan, connect, set_type ...) queue MLME actions.
        self.process_actions();
        r
    }

    fn status(&self, t: u64) -> String {
        let w = self.wlan.as_ref().unwrap();
        let m = w.mlme();
        let mut s = String::new();
        s += "present=1\n";
        s += &format!("ifname={}\n", IFNAME);
        s += &format!("mac={}\n", frame::mac_str(&self.mac));
        s += &format!("mode={}\n", if w.is_monitor() { "monitor" } else { "managed" });
        s += &format!("state={}\n", state_str(m.state()));
        s += &format!("scanning={}\n", m.is_scanning() as u8);
        s += &format!("channel={}\n", m.channel());
        s += &format!("freq={}\n", frame::channel_to_freq(m.channel()));
        s += &format!("tx_power_dbm={}\n", m.config().tx_power_dbm);
        s += &format!("authorized={}\n", w.authorized() as u8);
        s += &format!("ptk={}\n", w.has_ptk() as u8);
        s += &format!("scan_results={}\n", m.scan_results(t).len());
        if let Some(b) = m.current_bss() {
            s += &format!("bssid={}\n", frame::mac_str(&b.bssid));
            s += &format!("ssid={}\n", hex(&b.ssid));
            s += &format!("bss_channel={}\n", b.channel);
            if let Some(r) = m.link_rssi() {
                s += &format!("signal={}\n", r);
            }
        }
        if let Some(p) = m.connect_params() {
            s += &format!("target_ssid={}\n", hex(&p.ssid));
            s += &format!(
                "target_security={}\n",
                if p.security == ConnectSecurity::Wpa2Psk { "wpa2" } else { "open" }
            );
        }
        s += &format!("event_seq={}\n", self.next_event_seq - 1);
        s
    }
}

#[cfg(test)]
#[path = "wifi_tests.rs"]
mod tests;
