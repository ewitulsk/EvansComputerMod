//! `Wlan`: the station interface that presents an 802.11 connection as Ethernet.
//!
//! This is what the kernel's `wlan0` wraps. Upwards it speaks Ethernet II frames (to and
//! from `ecm-net`, including EAPOL for the packet socket); downwards it speaks raw 802.11
//! frames with rate and power for the SoftMAC host ABI (`wifi_tx_frame` /
//! `wifi_rx_frame`). It owns the [`Mlme`], the installed keys (software CCMP), the
//! per-AP minstrel rate controller, duplicate detection, link statistics and monitor mode.
//!
//! ```text
//! // RX: for each (frame, rssi, rate, channel, ts) from wifi_rx_frame:
//! match wlan.on_rx(&frame, rssi, rate, channel, now) {
//!     Some(RxOutput::Ethernet(eth)) => stack.handle_frame(wlan_if, &eth, now),
//!     Some(RxOutput::Monitor(rt)) => pcap.write(rt),   // radiotap + 802.11
//!     None => {}
//! }
//! // TX: for each Ethernet frame the stack queued on wlan0:
//! if let Some(tx) = wlan.send_ethernet(&eth, now) { wifi_tx_frame(&tx.frame, tx.rate, tx.power_dbm) }
//! // Timers and control:
//! wlan.poll(now);
//! while let Some(a) = wlan.pop_action() { /* Transmit / SetChannel / SetRxFilter / Event */ }
//! ```

use crate::crypto::{self, ReplayCounters, TxPn};
use crate::frame::{self, Header, MacAddr};
use crate::mlme::{self, Action, ConnectParams, ConnectSecurity, LinkState, Mlme, MlmeConfig, ScanRequest};
use crate::radiotap;
use crate::rate::{self, Minstrel, Rate};

/// A frame to hand to the SoftMAC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxFrame {
    /// Raw 802.11 frame without FCS.
    pub frame: Vec<u8>,
    /// Rate code (see [`crate::rate`]).
    pub rate: u8,
    pub power_dbm: i8,
}

/// What a received frame became.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RxOutput {
    /// A decrypted data frame converted to Ethernet II, for the network stack.
    Ethernet(Vec<u8>),
    /// Monitor mode: radiotap header + the raw 802.11 frame, for pcap (link type 127).
    Monitor(Vec<u8>),
}

/// Counters (as in `iw dev wlan0 station dump` / `ip -s link`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WlanStats {
    pub rx_packets: u64,
    pub rx_bytes: u64,
    pub tx_packets: u64,
    pub tx_bytes: u64,
    pub rx_dropped_replay: u64,
    pub rx_dropped_mic: u64,
    pub rx_dropped_unencrypted: u64,
    pub rx_duplicates: u64,
    pub tx_dropped_unauthorized: u64,
    pub tx_retries: u64,
    pub tx_failed: u64,
}

/// Link information for `iw dev wlan0 link`.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkInfo {
    pub bssid: MacAddr,
    pub ssid: Vec<u8>,
    pub channel: u8,
    pub freq_mhz: u16,
    pub signal_dbm: i32,
    pub tx_bitrate: Rate,
    pub rx_bitrate: Option<Rate>,
    pub beacon_interval: u16,
    pub connected_ms: u64,
    pub authorized: bool,
    pub stats: WlanStats,
}

impl LinkInfo {
    /// Render like `iw dev wlan0 link`.
    pub fn format_iw(&self) -> String {
        let mut s = String::new();
        s += &format!("Connected to {} (on wlan0)\n", frame::mac_str(&self.bssid));
        s += &format!("\tSSID: {}\n", String::from_utf8_lossy(&self.ssid));
        s += &format!("\tfreq: {}\n", self.freq_mhz);
        s += &format!("\tRX: {} bytes ({} packets)\n", self.stats.rx_bytes, self.stats.rx_packets);
        s += &format!("\tTX: {} bytes ({} packets)\n", self.stats.tx_bytes, self.stats.tx_packets);
        s += &format!("\tsignal: {} dBm\n", self.signal_dbm);
        if let Some(r) = self.rx_bitrate {
            s += &format!("\trx bitrate: {}\n", r.describe());
        }
        s += &format!("\ttx bitrate: {}\n", self.tx_bitrate.describe());
        s += &format!("\tbss flags: short-slot-time\n\tdtim period: 1\n\tbeacon int: {}\n", self.beacon_interval);
        s
    }
}

struct PairwiseKey {
    bssid: MacAddr,
    tk: [u8; 16],
    tx_pn: TxPn,
    rx: ReplayCounters,
}

struct GroupKey {
    gtk: [u8; 16],
    rx: ReplayCounters,
}

/// The wireless station interface.
pub struct Wlan {
    mlme: Mlme,
    gen: u64,
    ptk: Option<PairwiseKey>,
    gtk: [Option<GroupKey>; 4],
    rc: Option<Minstrel>,
    rc_seed: u64,
    stats: WlanStats,
    last_rx_rate: Option<u8>,
    last_tx_rate: Option<Rate>,
    /// Last (seq, frag) received per TID for duplicate detection.
    last_seq: [Option<(u16, u8)>; 17],
}

impl Wlan {
    pub fn new(cfg: MlmeConfig) -> Self {
        let seed = u64::from_le_bytes([cfg.own_mac[0], cfg.own_mac[1], cfg.own_mac[2], cfg.own_mac[3], cfg.own_mac[4], cfg.own_mac[5], 0x5a, 0xa5]);
        Wlan {
            mlme: Mlme::new(cfg),
            gen: 0,
            ptk: None,
            gtk: [None, None, None, None],
            rc: None,
            rc_seed: seed,
            stats: WlanStats::default(),
            last_rx_rate: None,
            last_tx_rate: None,
            last_seq: [None; 17],
        }
    }

    pub fn mac(&self) -> MacAddr {
        self.mlme.own_mac()
    }
    pub fn mlme(&self) -> &Mlme {
        &self.mlme
    }
    pub fn mlme_mut(&mut self) -> &mut Mlme {
        &mut self.mlme
    }
    pub fn stats(&self) -> &WlanStats {
        &self.stats
    }
    pub fn rate_control(&self) -> Option<&Minstrel> {
        self.rc.as_ref()
    }

    /// Drop keys and per-peer state when the association changed.
    fn sync(&mut self) {
        if self.mlme.generation() != self.gen {
            self.gen = self.mlme.generation();
            self.ptk = None;
            self.gtk = [None, None, None, None];
            self.rc = None;
            self.last_seq = [None; 17];
            self.last_tx_rate = None;
        }
        if self.rc.is_none() {
            if let Some(b) = self.mlme.current_bss() {
                let rates = rate::usable_rates(b.channel, &b.rates, b.ht_mcs);
                self.rc_seed = self.rc_seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                self.rc = Some(Minstrel::new(rates, self.rc_seed));
            }
        }
    }

    // ---------------------------------------------------------------- control

    pub fn scan(&mut self, req: ScanRequest, now: u64) -> bool {
        self.mlme.scan(req, now)
    }
    pub fn connect(&mut self, p: ConnectParams, now: u64) {
        self.mlme.connect(p, now);
        self.sync();
    }
    pub fn disconnect(&mut self, reason: u16, now: u64) {
        self.mlme.disconnect(reason, now);
        self.sync();
    }
    pub fn set_monitor(&mut self, on: bool, now: u64) {
        self.mlme.set_monitor(on, now);
        self.sync();
    }
    pub fn is_monitor(&self) -> bool {
        *self.mlme.state() == LinkState::Monitor
    }

    pub fn poll(&mut self, now: u64) -> Option<u64> {
        let d = self.mlme.poll(now);
        self.sync();
        d
    }

    pub fn pop_action(&mut self) -> Option<Action> {
        self.sync();
        self.mlme.pop_action()
    }

    /// Install the pairwise key for the current AP. Returns false (and keeps the
    /// existing key and its packet numbers) if the same key is already installed,
    /// or if `bssid` is not the AP we are associated with.
    pub fn install_ptk(&mut self, bssid: MacAddr, tk: [u8; 16]) -> bool {
        self.sync();
        if self.mlme.active_bssid() != Some(bssid) {
            return false;
        }
        if let Some(k) = &self.ptk {
            if k.bssid == bssid && k.tk == tk {
                return false;
            }
        }
        self.ptk = Some(PairwiseKey { bssid, tk, tx_pn: TxPn::new(), rx: ReplayCounters::new() });
        true
    }

    /// Install a GTK (16 bytes, CCMP) under `key_id` with starting RSC.
    pub fn install_gtk(&mut self, key_id: u8, gtk: &[u8], rsc: u64) -> bool {
        self.sync();
        if gtk.len() != 16 || key_id > 3 || self.mlme.active_bssid().is_none() {
            return false;
        }
        let mut g = [0u8; 16];
        g.copy_from_slice(gtk);
        if let Some(k) = &self.gtk[key_id as usize] {
            if k.gtk == g {
                return false;
            }
        }
        self.gtk[key_id as usize] = Some(GroupKey { gtk: g, rx: ReplayCounters::with_start(rsc) });
        true
    }

    pub fn clear_keys(&mut self) {
        self.ptk = None;
        self.gtk = [None, None, None, None];
    }

    pub fn has_ptk(&self) -> bool {
        self.ptk.is_some()
    }

    fn secured(&self) -> bool {
        self.mlme.connect_params().map_or(false, |p| p.security == ConnectSecurity::Wpa2Psk)
    }

    /// 802.1X controlled port open: open network, or PTK installed.
    pub fn authorized(&self) -> bool {
        self.mlme.current_bss().is_some() && (!self.secured() || self.ptk.is_some())
    }

    // ---------------------------------------------------------------- data path

    /// Ethernet frame from the stack → 802.11 frame for the SoftMAC (encrypted when a PTK
    /// is installed). Non-EAPOL traffic is dropped until the port is authorized.
    pub fn send_ethernet(&mut self, eth: &[u8], now: u64) -> Option<TxFrame> {
        self.sync();
        let bss = self.mlme.current_bss()?.clone();
        let et = frame::ethertype(eth)?;
        let eapol = et == frame::ETHERTYPE_EAPOL;
        if !eapol && !self.authorized() {
            self.stats.tx_dropped_unauthorized += 1;
            return None;
        }
        let mut mpdu = frame::ethernet_to_data_tods(eth, &bss.bssid)?;
        let seq = self.mlme.next_seq();
        frame::set_seq(&mut mpdu, seq);
        let out = match self.ptk.as_mut() {
            Some(k) => {
                let pn = k.tx_pn.next()?;
                crypto::ccmp_encrypt(&k.tk, &mpdu, pn, 0)?
            }
            None => mpdu,
        };
        let rate = if eapol {
            rate::basic_rate(bss.channel)
        } else {
            match self.rc.as_mut() {
                Some(rc) => rc.tx_rate(now),
                None => rate::basic_rate(bss.channel),
            }
        };
        if !eapol {
            self.last_tx_rate = Some(rate);
        }
        self.stats.tx_packets += 1;
        self.stats.tx_bytes += eth.len() as u64;
        Some(TxFrame { frame: out, rate: rate.code, power_dbm: self.mlme.config().tx_power_dbm })
    }

    /// Transmit status from the SoftMAC for a data frame (feeds rate control).
    pub fn tx_status(&mut self, rate_code: u8, attempts: u32, acked: bool, now: u64) {
        self.sync();
        self.stats.tx_retries += attempts.saturating_sub(1) as u64;
        if !acked {
            self.stats.tx_failed += 1;
        }
        if let (Some(rc), Some(r)) = (self.rc.as_mut(), Rate::from_code(rate_code)) {
            rc.tx_status(r, attempts, acked, now);
        }
    }

    /// Handle any received frame. Management frames go to the MLME, data frames are
    /// decrypted and converted to Ethernet; in monitor mode everything is returned with
    /// a radiotap header.
    pub fn on_rx(&mut self, f: &[u8], rssi_dbm: i32, rate: u8, channel: u8, now: u64) -> Option<RxOutput> {
        self.sync();
        if self.is_monitor() {
            let rt = radiotap::encapsulate(f, now * 1000, rate, channel, rssi_dbm.clamp(-128, 127) as i8, false);
            return Some(RxOutput::Monitor(rt));
        }
        if f.len() < 2 {
            return None;
        }
        match frame::frame_type(f) {
            frame::TYPE_MGMT => {
                self.mlme.on_rx(f, rssi_dbm, rate, channel, now);
                self.sync();
                None
            }
            frame::TYPE_DATA => {
                let bssid = self.mlme.current_bss()?.bssid;
                if Mlme::frame_bssid(f) != Some(bssid) {
                    return None;
                }
                self.mlme.note_link_rx(rssi_dbm, now);
                self.last_rx_rate = Some(rate);
                self.recv_80211(f, now).map(RxOutput::Ethernet)
            }
            _ => None,
        }
    }

    /// Data MPDU from the AP → Ethernet frame (decrypt, replay check, duplicate check,
    /// port control, LLC/SNAP removal).
    pub fn recv_80211(&mut self, f: &[u8], _now: u64) -> Option<Vec<u8>> {
        self.sync();
        let own = self.mac();
        let bssid = self.mlme.current_bss()?.bssid;
        let (h, _) = Header::parse(f)?;
        if h.ftype != frame::TYPE_DATA || h.to_ds() || !h.from_ds() || h.addr2 != bssid {
            return None;
        }
        let group = frame::is_multicast(&h.addr1);
        if !(group || h.addr1 == own) {
            return None;
        }
        if group && h.addr3 == own {
            return None; // our own broadcast relayed back by the AP
        }
        let tid = frame::tid(f);
        // Duplicate detection (retransmissions whose ACK was lost).
        if !group {
            if h.retry() && self.last_seq[tid] == Some((h.seq, h.frag)) {
                self.stats.rx_duplicates += 1;
                return None;
            }
            self.last_seq[tid] = Some((h.seq, h.frag));
        }
        let plain = if h.protected() {
            let (pn, key_id) = crypto::ccmp_peek(f)?;
            let (tk, counters) = if group {
                let k = self.gtk[key_id as usize].as_mut()?;
                (k.gtk, &mut k.rx)
            } else {
                let k = self.ptk.as_mut()?;
                (k.tk, &mut k.rx)
            };
            if !counters.check(tid, pn) {
                self.stats.rx_dropped_replay += 1;
                return None;
            }
            let Some(d) = crypto::ccmp_decrypt(&tk, f) else {
                self.stats.rx_dropped_mic += 1;
                return None;
            };
            counters.update(tid, pn);
            d.mpdu
        } else {
            f.to_vec()
        };
        let eth = frame::data_to_ethernet(&plain)?;
        if !h.protected() && self.secured() && frame::ethertype(&eth) != Some(frame::ETHERTYPE_EAPOL) {
            self.stats.rx_dropped_unencrypted += 1;
            return None;
        }
        self.stats.rx_packets += 1;
        self.stats.rx_bytes += eth.len() as u64;
        Some(eth)
    }

    /// Monitor-mode injection: a radiotap-prefixed frame → TxFrame (rate from the radiotap
    /// header, else the basic rate).
    pub fn inject(&mut self, radiotap_frame: &[u8]) -> Option<TxFrame> {
        if !self.is_monitor() {
            return None;
        }
        let (info, hlen) = radiotap::parse(radiotap_frame)?;
        let body = &radiotap_frame[hlen..];
        let body = if info.flags.map_or(false, |f| f & radiotap::F_FCS != 0) { frame::strip_fcs(body)? } else { body };
        let ch = self.mlme.channel();
        let rate = info.rate.filter(|c| Rate::from_code(*c).is_some()).unwrap_or(rate::basic_rate(ch).code);
        let power = info.tx_power_dbm.unwrap_or(self.mlme.config().tx_power_dbm);
        Some(TxFrame { frame: body.to_vec(), rate, power_dbm: power })
    }

    /// Link information while connected.
    pub fn link_info(&self, now: u64) -> Option<LinkInfo> {
        let b = self.mlme.current_bss()?;
        let tx = self
            .last_tx_rate
            .or_else(|| self.rc.as_ref().map(|r| r.best_rate()))
            .unwrap_or_else(|| rate::basic_rate(b.channel));
        Some(LinkInfo {
            bssid: b.bssid,
            ssid: b.ssid.clone(),
            channel: b.channel,
            freq_mhz: frame::channel_to_freq(b.channel),
            signal_dbm: self.mlme.link_rssi().unwrap_or(b.rssi),
            tx_bitrate: tx,
            rx_bitrate: self.last_rx_rate.and_then(Rate::from_code),
            beacon_interval: b.beacon_interval,
            connected_ms: now.saturating_sub(self.mlme.connected_since().unwrap_or(now)),
            authorized: self.authorized(),
            stats: self.stats.clone(),
        })
    }
}

/// Re-export for drivers matching on events.
pub use mlme::Event as WlanEvent;
