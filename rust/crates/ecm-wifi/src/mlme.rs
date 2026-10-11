//! Station MLME (the mac80211 + cfg80211 "managed mode" state machine).
//!
//! Sans-IO: the driver feeds received management frames with [`Mlme::on_rx`], runs timers
//! with [`Mlme::poll`] (which returns the next deadline) and drains [`Action`]s with
//! [`Mlme::pop_action`]: frames to transmit (with rate and power), channel and RX filter
//! changes for the SoftMAC, and [`Event`]s for userspace (scan done, connected,
//! disconnected, ...).
//!
//! Covered: active and passive scanning with per-channel dwell, a BSS table, BSS
//! selection, Open System authentication, (re)association, connection monitoring with
//! beacon-loss detection, AP-initiated deauthentication / disassociation, automatic
//! reconnection and roaming to a stronger BSS of the same ESS.

use std::collections::VecDeque;

use crate::frame::{self, BssInfo, Header, MacAddr, Mgmt, MgmtBody, Rsn, Security};
use crate::rate;

/// RX filter programmed into the SoftMAC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RxFilter {
    /// Not associated: frames to our address and broadcast only.
    Idle,
    /// Scanning: additionally all beacons and probe responses.
    Scan,
    /// Associated: frames from this BSSID addressed to us or group-addressed.
    Bssid(MacAddr),
    /// Monitor: everything, including other BSSs and control frames.
    Monitor,
}

/// Why a connection attempt failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectFailure {
    /// No BSS matching the requested SSID/BSSID/security was found.
    NoBss,
    AuthTimeout,
    AuthRejected(u16),
    AssocTimeout,
    AssocRejected(u16),
    /// The AP deauthenticated us before association completed.
    Deauthenticated(u16),
}

/// Why an established connection ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisconnectReason {
    /// The AP sent a Deauthentication frame with this reason code.
    Deauth(u16),
    /// The AP sent a Disassociation frame with this reason code.
    Disassoc(u16),
    /// No frame from the AP within the beacon-loss timeout.
    BeaconLoss,
    /// We disconnected (userspace request) with this reason code.
    Local(u16),
}

impl DisconnectReason {
    /// The 802.11 reason code (beacon loss reports 4, inactivity).
    pub fn code(&self) -> u16 {
        match *self {
            DisconnectReason::Deauth(c) | DisconnectReason::Disassoc(c) | DisconnectReason::Local(c) => c,
            DisconnectReason::BeaconLoss => frame::REASON_INACTIVITY,
        }
    }
    pub fn locally_generated(&self) -> bool {
        matches!(self, DisconnectReason::Local(_) | DisconnectReason::BeaconLoss)
    }
}

/// Notifications for the owner of the interface (and through it, userspace).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    ScanDone { results: usize, aborted: bool },
    Connected {
        bssid: MacAddr,
        ssid: Vec<u8>,
        channel: u8,
        aid: u16,
        /// AP's RSN element as advertised in its beacon/probe response (whole IE).
        ap_rsn_ie: Option<Vec<u8>>,
        /// RSN element we sent in the (re)association request (whole IE).
        sta_rsn_ie: Option<Vec<u8>>,
        /// True if this was a reassociation to another AP of the same ESS.
        roamed: bool,
    },
    ConnectFailed { bssid: Option<MacAddr>, reason: ConnectFailure },
    Disconnected { bssid: MacAddr, reason: DisconnectReason },
}

/// Output of the MLME.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Transmit a raw 802.11 frame (no FCS) at `rate` (rate code) and `power_dbm`.
    Transmit { frame: Vec<u8>, rate: u8, power_dbm: i8 },
    SetChannel(u8),
    SetRxFilter(RxFilter),
    Event(Event),
}

/// Security of a connect request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectSecurity {
    Open,
    Wpa2Psk,
}

/// A cfg80211-style connect request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectParams {
    pub ssid: Vec<u8>,
    /// Lock to this BSSID (no roaming away from it).
    pub bssid: Option<MacAddr>,
    pub security: ConnectSecurity,
    /// Reconnect automatically after beacon loss or AP-initiated deauth.
    pub auto_reconnect: bool,
    /// Roam to a stronger AP of the same ESS.
    pub roaming: bool,
}

impl ConnectParams {
    pub fn new(ssid: &[u8], security: ConnectSecurity) -> Self {
        ConnectParams { ssid: ssid.to_vec(), bssid: None, security, auto_reconnect: true, roaming: true }
    }
}

/// A scan request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanRequest {
    /// Directed SSID to probe for (finds hidden networks); `None` = wildcard.
    pub ssid: Option<Vec<u8>>,
    /// Channels to visit; empty = all configured channels.
    pub channels: Vec<u8>,
    /// Listen only (no probe requests).
    pub passive: bool,
}

/// Tunables (defaults mirror mac80211 where it has an equivalent).
#[derive(Clone, Debug)]
pub struct MlmeConfig {
    pub own_mac: MacAddr,
    pub tx_power_dbm: i8,
    pub channels: Vec<u8>,
    /// Dwell per channel for an active scan (ms). In the world an Access Point
    /// answers probes only on its server tick (50 ms) and beacons every 100 ms,
    /// so a shorter dwell misses networks that are right there.
    pub active_dwell_ms: u64,
    /// Dwell per channel for a passive scan (ms); must exceed one beacon interval.
    pub passive_dwell_ms: u64,
    pub auth_timeout_ms: u64,
    pub assoc_timeout_ms: u64,
    /// Transmissions of an auth/assoc request before giving up on a BSS.
    pub max_tries: u32,
    /// No frame from the AP for this long → disconnected (a probe is sent at half).
    pub beacon_loss_ms: u64,
    /// Start roam scans when the link RSSI falls below this (dBm).
    pub roam_rssi_dbm: i32,
    /// A candidate must be this much stronger than the current AP (dB).
    pub roam_hysteresis_db: i32,
    /// Minimum time between roam scans (ms).
    pub roam_scan_interval_ms: u64,
    /// Delay before an automatic reconnect (ms).
    pub reconnect_delay_ms: u64,
    /// Scan results older than this are not returned (ms).
    pub bss_expire_ms: u64,
    /// While associated and nothing has been sent for this long, send a Null data frame to
    /// the AP (ms; 0 = never). APs drop clients they haven't heard from (the in-world Access
    /// Point after 300 s, hostapd's default too), so an idle but present station keeps its
    /// association; a station that has really gone stops sending and is still dropped.
    pub keepalive_ms: u64,
}

/// 5 GHz channels a station scans: the 20 MHz UNII-1 and UNII-3 channels Access Points offer.
pub const CHANNELS_5GHZ: [u8; 9] = [36, 40, 44, 48, 149, 153, 157, 161, 165];

/// Every channel a station scans by default: 2.4 GHz 1-13, then the 5 GHz channels.
pub fn default_channels() -> Vec<u8> {
    let mut v: Vec<u8> = (1..=13).collect();
    v.extend_from_slice(&CHANNELS_5GHZ);
    v
}

impl MlmeConfig {
    pub fn new(own_mac: MacAddr) -> Self {
        MlmeConfig {
            own_mac,
            tx_power_dbm: 20,
            channels: default_channels(),
            active_dwell_ms: 120,
            passive_dwell_ms: 110,
            auth_timeout_ms: 200,
            assoc_timeout_ms: 200,
            max_tries: 3,
            beacon_loss_ms: 2000,
            roam_rssi_dbm: -70,
            roam_hysteresis_db: 8,
            roam_scan_interval_ms: 10_000,
            reconnect_delay_ms: 1000,
            bss_expire_ms: 30_000,
            keepalive_ms: 30_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScanPurpose {
    User,
    Connect,
    Roam,
}

#[derive(Clone, Debug)]
struct ScanState {
    channels: Vec<u8>,
    idx: usize,
    ssid: Option<Vec<u8>>,
    passive: bool,
    dwell_end: u64,
    started: u64,
    purpose: ScanPurpose,
}

/// Station link state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    Idle,
    /// Waiting for a scan to pick a BSS for the pending connect request.
    Searching,
    Authenticating { bssid: MacAddr, tries: u32, deadline: u64 },
    Associating { bssid: MacAddr, tries: u32, deadline: u64 },
    Connected { bssid: MacAddr, aid: u16 },
    /// Auto-reconnect pending.
    ReconnectWait { until: u64 },
    Monitor,
}

/// The station MLME.
pub struct Mlme {
    cfg: MlmeConfig,
    state: LinkState,
    params: Option<ConnectParams>,
    bss: Vec<BssInfo>,
    scan: Option<ScanState>,
    actions: VecDeque<Action>,
    channel: u8,
    seq: u16,
    generation: u64,
    /// Remaining candidates for the current connect attempt.
    candidates: Vec<MacAddr>,
    /// BSS we are reassociating away from.
    roam_from: Option<MacAddr>,
    /// BSS the current association / attempt is for.
    target: Option<BssInfo>,
    sta_rsn_ie: Option<Vec<u8>>,
    last_heard: u64,
    link_rssi: Option<i32>,
    probe_sent: bool,
    last_roam_scan: u64,
    connected_at: u64,
    /// When this station last transmitted to its AP (keep-alive timer).
    last_tx: u64,
}

impl Mlme {
    pub fn new(cfg: MlmeConfig) -> Self {
        let ch = cfg.channels.first().copied().unwrap_or(1);
        Mlme {
            cfg,
            state: LinkState::Idle,
            params: None,
            bss: Vec::new(),
            scan: None,
            actions: VecDeque::new(),
            channel: ch,
            seq: 0,
            generation: 0,
            candidates: Vec::new(),
            roam_from: None,
            target: None,
            sta_rsn_ie: None,
            last_heard: 0,
            link_rssi: None,
            probe_sent: false,
            last_roam_scan: 0,
            connected_at: 0,
            last_tx: 0,
        }
    }

    pub fn config(&self) -> &MlmeConfig {
        &self.cfg
    }
    pub fn config_mut(&mut self) -> &mut MlmeConfig {
        &mut self.cfg
    }
    pub fn own_mac(&self) -> MacAddr {
        self.cfg.own_mac
    }
    pub fn state(&self) -> &LinkState {
        &self.state
    }
    pub fn channel(&self) -> u8 {
        self.channel
    }
    pub fn is_scanning(&self) -> bool {
        self.scan.is_some()
    }
    pub fn connect_params(&self) -> Option<&ConnectParams> {
        self.params.as_ref()
    }

    /// Incremented whenever the association changes (new attempt, roam, disconnect):
    /// keys and per-peer state bound to the previous association must be dropped.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The associated BSS, if connected.
    pub fn current_bss(&self) -> Option<&BssInfo> {
        match self.state {
            LinkState::Connected { .. } => self.target.as_ref(),
            _ => None,
        }
    }

    /// BSSID of the BSS we are associated with or authenticating/associating to.
    pub fn active_bssid(&self) -> Option<MacAddr> {
        match self.state {
            LinkState::Connected { bssid, .. }
            | LinkState::Authenticating { bssid, .. }
            | LinkState::Associating { bssid, .. } => Some(bssid),
            _ => None,
        }
    }

    pub fn link_rssi(&self) -> Option<i32> {
        self.link_rssi
    }
    pub fn connected_since(&self) -> Option<u64> {
        self.current_bss().map(|_| self.connected_at)
    }

    /// Next sequence number for a transmitted frame (shared with the data path).
    pub fn next_seq(&mut self) -> u16 {
        let s = self.seq;
        self.seq = (self.seq + 1) & 0x0fff;
        s
    }

    pub fn pop_action(&mut self) -> Option<Action> {
        self.actions.pop_front()
    }

    /// Scan results not older than `bss_expire_ms`, strongest first.
    pub fn scan_results(&self, now: u64) -> Vec<BssInfo> {
        let mut v: Vec<BssInfo> = self
            .bss
            .iter()
            .filter(|b| now.saturating_sub(b.last_seen) <= self.cfg.bss_expire_ms)
            .cloned()
            .collect();
        v.sort_by(|a, b| b.rssi.cmp(&a.rssi));
        v
    }

    fn emit(&mut self, e: Event) {
        self.actions.push_back(Action::Event(e));
    }

    fn set_channel(&mut self, ch: u8) {
        if ch != self.channel {
            self.channel = ch;
            self.actions.push_back(Action::SetChannel(ch));
        }
    }

    fn set_filter(&mut self, f: RxFilter) {
        self.actions.push_back(Action::SetRxFilter(f));
    }

    /// The station sent a frame to its AP (data path): restarts the keep-alive timer.
    pub fn note_tx(&mut self, now: u64) {
        self.last_tx = self.last_tx.max(now);
    }

    /// A Null data frame (To-DS, no body) to the associated AP: the 802.11 keep-alive.
    fn send_null_data(&mut self, bssid: MacAddr, now: u64) {
        let own = self.cfg.own_mac;
        let seq = self.next_seq();
        let mut f = Vec::with_capacity(24);
        f.extend_from_slice(&[0x48, 0x01, 0, 0]); // data / Null, To-DS; duration 0
        f.extend_from_slice(&bssid);
        f.extend_from_slice(&own);
        f.extend_from_slice(&bssid);
        f.extend_from_slice(&((seq & 0xfff) << 4).to_le_bytes());
        let rate = rate::basic_rate(self.channel).code;
        self.actions.push_back(Action::Transmit { frame: f, rate, power_dbm: self.cfg.tx_power_dbm });
        self.last_tx = now;
    }

    fn tx_mgmt(&mut self, mut m: Mgmt) {
        m.hdr.seq = self.next_seq();
        let rate = rate::basic_rate(self.channel).code;
        self.actions.push_back(Action::Transmit { frame: m.to_bytes(), rate, power_dbm: self.cfg.tx_power_dbm });
    }

    // ------------------------------------------------------------------ scanning

    /// Start a user scan. Returns false if a scan is already running or in monitor mode.
    pub fn scan(&mut self, req: ScanRequest, now: u64) -> bool {
        if self.scan.is_some() || self.state == LinkState::Monitor {
            return false;
        }
        self.start_scan(req, ScanPurpose::User, now);
        true
    }

    fn start_scan(&mut self, req: ScanRequest, purpose: ScanPurpose, now: u64) {
        let channels = if req.channels.is_empty() { self.cfg.channels.clone() } else { req.channels };
        self.scan = Some(ScanState {
            channels,
            idx: 0,
            ssid: req.ssid,
            passive: req.passive,
            dwell_end: 0,
            started: now,
            purpose,
        });
        self.set_filter(RxFilter::Scan);
        self.scan_visit(now);
    }

    fn scan_visit(&mut self, now: u64) {
        let Some(s) = self.scan.as_ref() else { return };
        let ch = s.channels[s.idx];
        let passive = s.passive;
        let ssid = s.ssid.clone();
        let dwell = if passive { self.cfg.passive_dwell_ms } else { self.cfg.active_dwell_ms };
        self.set_channel(ch);
        if !passive {
            let mut ies = Vec::new();
            frame::push_ie(&mut ies, frame::IE_SSID, ssid.as_deref().unwrap_or(&[]));
            frame::push_rates(&mut ies, &rate::our_rates(ch));
            frame::push_ie(&mut ies, frame::IE_DS_PARAMS, &[ch]);
            frame::push_ie(&mut ies, frame::IE_HT_CAP, &frame::ht_cap_body());
            let own = self.cfg.own_mac;
            self.tx_mgmt(Mgmt::new(frame::BROADCAST, own, frame::BROADCAST, MgmtBody::ProbeReq { ies }));
        }
        if let Some(s) = self.scan.as_mut() {
            s.dwell_end = now + dwell;
        }
    }

    /// Abort a running scan.
    pub fn abort_scan(&mut self, now: u64) {
        if self.scan.is_some() {
            self.finish_scan(now, true);
        }
    }

    fn finish_scan(&mut self, now: u64, aborted: bool) {
        let Some(s) = self.scan.take() else { return };
        // Back to the operating channel and filter.
        match self.state {
            LinkState::Connected { bssid, .. } => {
                let ch = self.target.as_ref().map(|b| b.channel).unwrap_or(self.channel);
                self.set_channel(ch);
                self.set_filter(RxFilter::Bssid(bssid));
                // Time off-channel does not count as beacon loss.
                self.last_heard = now;
                self.probe_sent = false;
            }
            _ => self.set_filter(RxFilter::Idle),
        }
        let results = self.bss.iter().filter(|b| b.last_seen >= s.started).count();
        self.emit(Event::ScanDone { results, aborted });
        match s.purpose {
            ScanPurpose::User => {}
            ScanPurpose::Connect => {
                if self.state == LinkState::Searching {
                    self.select_and_join(now, s.started);
                }
            }
            ScanPurpose::Roam => self.evaluate_roam(now, s.started),
        }
    }

    fn record_bss(&mut self, mut info: BssInfo) {
        if let Some(e) = self.bss.iter_mut().find(|b| b.bssid == info.bssid) {
            if info.hidden() && !e.hidden() {
                info.ssid = e.ssid.clone();
            }
            *e = info;
        } else {
            self.bss.push(info);
        }
    }

    // ------------------------------------------------------------------ connect

    fn matches(&self, b: &BssInfo, p: &ConnectParams) -> bool {
        if b.ssid != p.ssid {
            return false;
        }
        if let Some(lock) = p.bssid {
            if b.bssid != lock {
                return false;
            }
        }
        match p.security {
            ConnectSecurity::Open => b.security() == Security::Open,
            ConnectSecurity::Wpa2Psk => b.security() == Security::Wpa2Psk,
        }
    }

    /// Request a connection (like `nl80211 CONNECT`). Scans first unless a fresh scan
    /// result already matches.
    pub fn connect(&mut self, params: ConnectParams, now: u64) {
        if self.state == LinkState::Monitor {
            return;
        }
        if let LinkState::Connected { .. } = self.state {
            self.disconnect(frame::REASON_DEAUTH_LEAVING, now);
        }
        self.params = Some(params);
        self.begin_search(now);
    }

    fn begin_search(&mut self, now: u64) {
        self.generation += 1;
        self.state = LinkState::Searching;
        let fresh = self.params.as_ref().map_or(false, |p| {
            self.bss.iter().any(|b| now.saturating_sub(b.last_seen) <= 3000 && self.matches(b, p))
        });
        if fresh && self.scan.is_none() {
            self.select_and_join(now, now.saturating_sub(3000));
            return;
        }
        if self.scan.is_some() {
            // A running scan will pick a BSS when it finishes: make it a connect scan.
            if let Some(s) = self.scan.as_mut() {
                s.purpose = ScanPurpose::Connect;
            }
            return;
        }
        let ssid = self.params.as_ref().map(|p| p.ssid.clone());
        self.start_scan(ScanRequest { ssid, channels: Vec::new(), passive: false }, ScanPurpose::Connect, now);
    }

    fn select_and_join(&mut self, now: u64, since: u64) {
        let Some(p) = self.params.clone() else {
            self.state = LinkState::Idle;
            return;
        };
        let mut cands: Vec<&BssInfo> =
            self.bss.iter().filter(|b| b.last_seen >= since && self.matches(b, &p)).collect();
        cands.sort_by(|a, b| b.rssi.cmp(&a.rssi));
        self.candidates = cands.iter().rev().map(|b| b.bssid).collect(); // pop() = strongest
        self.roam_from = None;
        self.try_next_candidate(now, None);
    }

    fn try_next_candidate(&mut self, now: u64, last_failure: Option<(MacAddr, ConnectFailure)>) {
        match self.candidates.pop() {
            Some(bssid) => self.start_auth(bssid, now),
            None => {
                let (bssid, reason) = match last_failure {
                    Some((b, r)) => (Some(b), r),
                    None => (None, ConnectFailure::NoBss),
                };
                self.emit(Event::ConnectFailed { bssid, reason });
                self.target = None;
                self.after_link_loss(now);
            }
        }
    }

    fn after_link_loss(&mut self, now: u64) {
        self.generation += 1;
        self.set_filter(RxFilter::Idle);
        let auto = self.params.as_ref().map_or(false, |p| p.auto_reconnect);
        self.state = if auto {
            LinkState::ReconnectWait { until: now + self.cfg.reconnect_delay_ms }
        } else {
            self.params = None;
            LinkState::Idle
        };
    }

    fn start_auth(&mut self, bssid: MacAddr, now: u64) {
        let Some(info) = self.bss.iter().find(|b| b.bssid == bssid).cloned() else {
            self.try_next_candidate(now, None);
            return;
        };
        self.generation += 1;
        self.set_channel(info.channel);
        self.set_filter(RxFilter::Bssid(bssid));
        self.target = Some(info);
        self.state = LinkState::Authenticating { bssid, tries: 0, deadline: now };
        self.send_auth(now);
    }

    fn send_auth(&mut self, now: u64) {
        if let LinkState::Authenticating { bssid, tries, deadline } = &mut self.state {
            *tries += 1;
            *deadline = now + self.cfg.auth_timeout_ms;
            let b = *bssid;
            let own = self.cfg.own_mac;
            self.tx_mgmt(Mgmt::new(b, own, b, MgmtBody::Auth { algo: 0, seq: 1, status: 0, ies: Vec::new() }));
        }
    }

    /// RSN element we advertise for WPA2-PSK/CCMP (whole IE).
    pub fn sta_rsn_ie() -> Vec<u8> {
        Rsn::wpa2_psk().to_ie()
    }

    fn send_assoc(&mut self, now: u64) {
        let LinkState::Associating { bssid, tries, deadline } = &mut self.state else { return };
        *tries += 1;
        *deadline = now + self.cfg.assoc_timeout_ms;
        let b = *bssid;
        let Some(info) = self.target.clone() else { return };
        let secure = self.params.as_ref().map_or(false, |p| p.security == ConnectSecurity::Wpa2Psk);
        let mut ies = Vec::new();
        frame::push_ie(&mut ies, frame::IE_SSID, &info.ssid);
        frame::push_rates(&mut ies, &rate::our_rates(info.channel));
        self.sta_rsn_ie = if secure { Some(Self::sta_rsn_ie()) } else { None };
        if let Some(r) = &self.sta_rsn_ie {
            ies.extend_from_slice(r);
        }
        if info.ht_mcs != 0 {
            frame::push_ie(&mut ies, frame::IE_HT_CAP, &frame::ht_cap_body());
        }
        let mut cap = frame::CAP_ESS | frame::CAP_SHORT_SLOT;
        if secure {
            cap |= frame::CAP_PRIVACY;
        }
        let own = self.cfg.own_mac;
        let body = match self.roam_from {
            Some(old) => MgmtBody::ReassocReq { cap, listen: 10, current_ap: old, ies },
            None => MgmtBody::AssocReq { cap, listen: 10, ies },
        };
        self.tx_mgmt(Mgmt::new(b, own, b, body));
    }

    /// Leave the BSS (sends Deauthentication) and forget the connect request.
    pub fn disconnect(&mut self, reason: u16, now: u64) {
        let active = self.active_bssid();
        let was_connected = matches!(self.state, LinkState::Connected { .. });
        if let Some(b) = active {
            let own = self.cfg.own_mac;
            self.tx_mgmt(Mgmt::new(b, own, b, MgmtBody::Deauth { reason }));
        }
        self.params = None;
        self.candidates.clear();
        if self.scan.is_some() {
            self.finish_scan(now, true);
        }
        if was_connected {
            if let Some(b) = active {
                self.emit(Event::Disconnected { bssid: b, reason: DisconnectReason::Local(reason) });
            }
        }
        self.target = None;
        self.link_rssi = None;
        self.generation += 1;
        self.state = LinkState::Idle;
        self.set_filter(RxFilter::Idle);
    }

    /// Reassociate to `bssid` (same ESS) now, e.g. on `wpa_cli roam`.
    pub fn roam(&mut self, bssid: MacAddr, now: u64) -> bool {
        let LinkState::Connected { bssid: cur, .. } = self.state else { return false };
        if cur == bssid {
            return false;
        }
        let Some(p) = self.params.clone() else { return false };
        let Some(info) = self.bss.iter().find(|b| b.bssid == bssid).cloned() else { return false };
        if !self.matches(&info, &ConnectParams { bssid: None, ..p }) {
            return false;
        }
        self.candidates.clear();
        self.start_auth(bssid, now);
        self.roam_from = Some(cur);
        true
    }

    fn evaluate_roam(&mut self, now: u64, since: u64) {
        let LinkState::Connected { bssid: cur, .. } = self.state else { return };
        let Some(p) = self.params.clone() else { return };
        if p.bssid.is_some() || !p.roaming {
            return;
        }
        let cur_rssi = self.link_rssi.unwrap_or(-100);
        let best = self
            .bss
            .iter()
            .filter(|b| b.last_seen >= since && b.bssid != cur && self.matches(b, &p))
            .max_by_key(|b| b.rssi)
            .cloned();
        if let Some(b) = best {
            if b.rssi >= cur_rssi + self.cfg.roam_hysteresis_db {
                self.roam(b.bssid, now);
            }
        }
    }

    // ------------------------------------------------------------------ monitor

    /// Enter or leave monitor mode (`iw dev wlan0 set type monitor|managed`).
    pub fn set_monitor(&mut self, on: bool, now: u64) {
        if on {
            if self.state != LinkState::Monitor {
                self.disconnect(frame::REASON_DEAUTH_LEAVING, now);
                self.state = LinkState::Monitor;
                self.set_filter(RxFilter::Monitor);
            }
        } else if self.state == LinkState::Monitor {
            self.state = LinkState::Idle;
            self.set_filter(RxFilter::Idle);
        }
    }

    /// Tune the radio (monitor mode, or idle).
    pub fn set_channel_manual(&mut self, ch: u8) -> bool {
        match self.state {
            LinkState::Monitor | LinkState::Idle => {
                self.set_channel(ch);
                true
            }
            _ => false,
        }
    }

    // ------------------------------------------------------------------ rx / timers

    /// Note that a frame from the associated AP was received (data path).
    pub fn note_link_rx(&mut self, rssi_dbm: i32, now: u64) {
        if let LinkState::Connected { .. } = self.state {
            self.last_heard = now;
            self.probe_sent = false;
            self.link_rssi = Some(match self.link_rssi {
                Some(r) => (r * 3 + rssi_dbm) / 4,
                None => rssi_dbm,
            });
        }
    }

    /// Handle a received management frame (non-management frames are ignored).
    pub fn on_rx(&mut self, f: &[u8], rssi_dbm: i32, _rate: u8, channel: u8, now: u64) {
        if self.state == LinkState::Monitor {
            return;
        }
        let Some(m) = Mgmt::parse(f) else { return };
        let own = self.cfg.own_mac;
        let to_us = m.hdr.addr1 == own;
        let group = frame::is_multicast(&m.hdr.addr1);
        match &m.body {
            MgmtBody::Beacon(bb) | MgmtBody::ProbeResp(bb) => {
                if !(to_us || group) {
                    return;
                }
                let info = BssInfo::from_beacon(m.hdr.addr3, bb, channel, rssi_dbm, now);
                if info.channel != channel {
                    return; // heard on an adjacent channel; ignore like mac80211
                }
                let from_current = self.active_bssid() == Some(info.bssid);
                self.record_bss(info);
                if from_current {
                    if let Some(i) = self.bss.iter().find(|b| b.bssid == m.hdr.addr3).cloned() {
                        // Keep the target's view (incl. a learned hidden SSID) fresh.
                        if let Some(t) = self.target.as_mut() {
                            t.rssi = i.rssi;
                            t.last_seen = now;
                        }
                    }
                    self.note_link_rx(rssi_dbm, now);
                }
            }
            MgmtBody::Auth { algo, seq, status, .. } => {
                let LinkState::Authenticating { bssid, .. } = self.state else { return };
                if !to_us || m.hdr.addr2 != bssid || *algo != 0 || *seq != 2 {
                    return;
                }
                if *status != frame::STATUS_SUCCESS {
                    self.try_next_candidate(now, Some((bssid, ConnectFailure::AuthRejected(*status))));
                    return;
                }
                self.state = LinkState::Associating { bssid, tries: 0, deadline: now };
                self.send_assoc(now);
            }
            MgmtBody::AssocResp { status, aid, ies, .. } | MgmtBody::ReassocResp { status, aid, ies, .. } => {
                let _ = ies;
                let LinkState::Associating { bssid, .. } = self.state else { return };
                if !to_us || m.hdr.addr2 != bssid {
                    return;
                }
                if *status != frame::STATUS_SUCCESS {
                    self.try_next_candidate(now, Some((bssid, ConnectFailure::AssocRejected(*status))));
                    return;
                }
                let roamed = self.roam_from.take().is_some();
                self.state = LinkState::Connected { bssid, aid: *aid };
                self.candidates.clear();
                self.last_heard = now;
                self.probe_sent = false;
                self.connected_at = now;
                self.last_tx = now;
                self.link_rssi = Some(rssi_dbm);
                self.last_roam_scan = now;
                let t = self.target.clone();
                let (ssid, channel, ap_rsn_ie) = match &t {
                    Some(t) => (
                        t.ssid.clone(),
                        t.channel,
                        t.rsn_ie.as_ref().map(|b| {
                            let mut v = vec![frame::IE_RSN, b.len() as u8];
                            v.extend_from_slice(b);
                            v
                        }),
                    ),
                    None => (Vec::new(), channel, None),
                };
                let sta_rsn_ie = self.sta_rsn_ie.clone();
                self.emit(Event::Connected { bssid, ssid, channel, aid: *aid, ap_rsn_ie, sta_rsn_ie, roamed });
            }
            MgmtBody::Deauth { reason } | MgmtBody::Disassoc { reason } => {
                if !(to_us || group) {
                    return;
                }
                let deauth = matches!(m.body, MgmtBody::Deauth { .. });
                match self.state {
                    LinkState::Connected { bssid, .. } if m.hdr.addr2 == bssid || m.hdr.addr3 == bssid => {
                        let r = if deauth { DisconnectReason::Deauth(*reason) } else { DisconnectReason::Disassoc(*reason) };
                        self.emit(Event::Disconnected { bssid, reason: r });
                        self.target = None;
                        self.link_rssi = None;
                        if self.scan.is_some() {
                            self.scan = None;
                            self.emit(Event::ScanDone { results: 0, aborted: true });
                        }
                        self.after_link_loss(now);
                    }
                    LinkState::Authenticating { bssid, .. } | LinkState::Associating { bssid, .. }
                        if m.hdr.addr2 == bssid =>
                    {
                        self.try_next_candidate(now, Some((bssid, ConnectFailure::Deauthenticated(*reason))));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Run timers. Returns the next deadline (ms), if any.
    pub fn poll(&mut self, now: u64) -> Option<u64> {
        // Scan dwell.
        if let Some(s) = &self.scan {
            if now >= s.dwell_end {
                if s.idx + 1 < s.channels.len() {
                    if let Some(s) = self.scan.as_mut() {
                        s.idx += 1;
                    }
                    self.scan_visit(now);
                } else {
                    self.finish_scan(now, false);
                }
            }
        }
        match self.state.clone() {
            LinkState::Authenticating { bssid, tries, deadline } if now >= deadline => {
                if tries >= self.cfg.max_tries {
                    self.try_next_candidate(now, Some((bssid, ConnectFailure::AuthTimeout)));
                } else {
                    self.send_auth(now);
                }
            }
            LinkState::Associating { bssid, tries, deadline } if now >= deadline => {
                if tries >= self.cfg.max_tries {
                    self.try_next_candidate(now, Some((bssid, ConnectFailure::AssocTimeout)));
                } else {
                    self.send_assoc(now);
                }
            }
            LinkState::Connected { bssid, .. } if self.scan.is_none() => {
                let silent = now.saturating_sub(self.last_heard);
                if silent >= self.cfg.beacon_loss_ms {
                    self.emit(Event::Disconnected { bssid, reason: DisconnectReason::BeaconLoss });
                    self.target = None;
                    self.link_rssi = None;
                    self.after_link_loss(now);
                } else if silent >= self.cfg.beacon_loss_ms / 2 && !self.probe_sent {
                    // Connection monitor: a unicast probe request to the AP.
                    self.probe_sent = true;
                    let mut ies = Vec::new();
                    let ssid = self.target.as_ref().map(|t| t.ssid.clone()).unwrap_or_default();
                    frame::push_ie(&mut ies, frame::IE_SSID, &ssid);
                    frame::push_rates(&mut ies, &rate::our_rates(self.channel));
                    let own = self.cfg.own_mac;
                    self.tx_mgmt(Mgmt::new(bssid, own, bssid, MgmtBody::ProbeReq { ies }));
                } else if self.cfg.keepalive_ms > 0 && now >= self.last_tx + self.cfg.keepalive_ms {
                    self.send_null_data(bssid, now);
                } else if let Some(p) = &self.params {
                    let weak = self.link_rssi.map_or(false, |r| r < self.cfg.roam_rssi_dbm);
                    if p.roaming
                        && p.bssid.is_none()
                        && weak
                        && now >= self.last_roam_scan + self.cfg.roam_scan_interval_ms
                    {
                        self.last_roam_scan = now;
                        let ssid = Some(p.ssid.clone());
                        self.start_scan(ScanRequest { ssid, channels: Vec::new(), passive: false }, ScanPurpose::Roam, now);
                    }
                }
            }
            LinkState::ReconnectWait { until } if now >= until => {
                if self.params.is_some() {
                    self.begin_search(now);
                } else {
                    self.state = LinkState::Idle;
                }
            }
            _ => {}
        }
        self.next_deadline()
    }

    fn next_deadline(&self) -> Option<u64> {
        let mut d: Option<u64> = self.scan.as_ref().map(|s| s.dwell_end);
        let mut take = |t: u64| d = Some(d.map_or(t, |x| x.min(t)));
        match self.state {
            LinkState::Authenticating { deadline, .. } | LinkState::Associating { deadline, .. } => take(deadline),
            LinkState::Connected { .. } => {
                let half = self.last_heard + self.cfg.beacon_loss_ms / 2;
                take(if self.probe_sent { self.last_heard + self.cfg.beacon_loss_ms } else { half });
                if self.cfg.keepalive_ms > 0 {
                    take(self.last_tx + self.cfg.keepalive_ms);
                }
                // Only when poll() would actually start a roam scan; otherwise (roaming off or a locked
                // BSSID) the scan never runs, last_roam_scan never advances and this deadline would
                // stay in the past, spinning the caller.
                let may_roam = self.params.as_ref().map_or(false, |p| p.roaming && p.bssid.is_none());
                if may_roam && self.link_rssi.map_or(false, |r| r < self.cfg.roam_rssi_dbm) {
                    take(self.last_roam_scan + self.cfg.roam_scan_interval_ms);
                }
            }
            LinkState::ReconnectWait { until } => take(until),
            _ => {}
        }
        d
    }

    /// Parse a frame's header (helper for drivers that want the BSSID before routing).
    pub fn frame_bssid(f: &[u8]) -> Option<MacAddr> {
        Header::parse(f).and_then(|(h, _)| h.bssid())
    }
}
