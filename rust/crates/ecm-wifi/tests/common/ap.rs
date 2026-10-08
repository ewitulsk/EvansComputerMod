//! A small WPA2-PSK access point written for the tests (hostapd stand-in): beacons,
//! probe responses (hidden SSID aware), open-system auth, (re)association, the
//! authenticator side of the 4-way and group-key handshakes with retransmissions,
//! CCMP data both ways and a "wired" side for bridged Ethernet frames.

#![allow(dead_code)]

use std::collections::VecDeque;

use ecm_wifi::crypto::{self, Ptk, ReplayCounters, TxPn};
use ecm_wifi::eapol::{self, KeyFrame};
use ecm_wifi::frame::{self, BeaconBody, MacAddr, Mgmt, MgmtBody, Rsn};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StaState {
    Authenticated,
    Associated,
    /// M1 sent, waiting for M2.
    WaitM2,
    /// M3 sent, waiting for M4.
    WaitM4,
    Done,
}

pub struct StaCtx {
    pub mac: MacAddr,
    pub state: StaState,
    pub aid: u16,
    pub anonce: [u8; 32],
    pub replay: u64,
    pub tptk: Option<Ptk>,
    pub ptk: Option<Ptk>,
    pub tries: u32,
    pub next_retx: u64,
    pub tx_pn: TxPn,
    pub rx: ReplayCounters,
    pub reassoc_from: Option<MacAddr>,
    pub group_pending: bool,
}

pub struct TestAp {
    pub bssid: MacAddr,
    pub ssid: Vec<u8>,
    pub channel: u8,
    pub pmk: Option<[u8; 32]>,
    pub hidden: bool,
    pub ht: bool,
    /// RSSI at which the station hears this AP (and vice versa).
    pub rssi: i32,
    pub beacon_interval_ms: u64,
    pub enabled: bool,
    next_beacon: u64,
    seq: u16,
    pub sta: Option<StaCtx>,
    pub gtk: [u8; 16],
    pub gtk_id: u8,
    pub gtk_pn: TxPn,
    pub out: VecDeque<Vec<u8>>,
    /// Ethernet frames the AP bridged from the station to the wired side.
    pub wired_rx: Vec<Vec<u8>>,
    pub mic_failures: u32,
    /// Drop this many outgoing M3 transmissions (simulated loss).
    pub drop_m3: u32,
    /// Drop this many incoming M4 receptions (simulated loss).
    pub drop_m4: u32,
    /// Corrupt the MIC of outgoing M3.
    pub corrupt_m3_mic: bool,
    /// Put this RSN IE into M3 instead of the beaconed one (downgrade attempt).
    pub m3_rsn_override: Option<Vec<u8>>,
    pub m3_sent: u32,
    pub m4_received: u32,
    pub probe_reqs: u32,
    pub reassoc_reqs: u32,
    pub deauths_rx: u32,
    pub retx_timeout_ms: u64,
    pub max_tries: u32,
    nonce_ctr: u8,
}

impl TestAp {
    pub fn new(bssid: MacAddr, ssid: &str, channel: u8, passphrase: Option<&str>) -> Self {
        let pmk = passphrase.map(|p| crypto::pmk_from_passphrase(p.as_bytes(), ssid.as_bytes()));
        let mut gtk = [0u8; 16];
        for (i, g) in gtk.iter_mut().enumerate() {
            *g = 0x60 ^ bssid[5] ^ i as u8;
        }
        TestAp {
            bssid,
            ssid: ssid.as_bytes().to_vec(),
            channel,
            pmk,
            hidden: false,
            ht: true,
            rssi: -50,
            beacon_interval_ms: 102,
            enabled: true,
            next_beacon: (bssid[5] as u64) % 50,
            seq: 0,
            sta: None,
            gtk,
            gtk_id: 1,
            gtk_pn: TxPn::new(),
            out: VecDeque::new(),
            wired_rx: Vec::new(),
            mic_failures: 0,
            drop_m3: 0,
            drop_m4: 0,
            corrupt_m3_mic: false,
            m3_rsn_override: None,
            m3_sent: 0,
            m4_received: 0,
            probe_reqs: 0,
            reassoc_reqs: 0,
            deauths_rx: 0,
            retx_timeout_ms: 100,
            max_tries: 4,
            nonce_ctr: 0,
        }
    }

    pub fn rsn_ie(&self) -> Vec<u8> {
        Rsn::wpa2_psk().to_ie()
    }

    fn ies(&self, for_probe: bool) -> Vec<u8> {
        let mut ies = Vec::new();
        let ssid: Vec<u8> = if self.hidden && !for_probe { Vec::new() } else { self.ssid.clone() };
        frame::push_ie(&mut ies, frame::IE_SSID, &ssid);
        frame::push_rates(&mut ies, &[0x82, 0x84, 0x8b, 0x96, 12, 18, 24, 36, 48, 72, 96, 108]);
        frame::push_ie(&mut ies, frame::IE_DS_PARAMS, &[self.channel]);
        if self.pmk.is_some() {
            ies.extend_from_slice(&self.rsn_ie());
        }
        if self.ht {
            frame::push_ie(&mut ies, frame::IE_HT_CAP, &frame::ht_cap_body());
        }
        ies
    }

    fn cap(&self) -> u16 {
        frame::CAP_ESS | if self.pmk.is_some() { frame::CAP_PRIVACY } else { 0 }
    }

    fn send_mgmt(&mut self, mut m: Mgmt) {
        m.hdr.seq = self.seq;
        self.seq = (self.seq + 1) & 0xfff;
        self.out.push_back(m.to_bytes());
    }

    pub fn poll(&mut self, now: u64) {
        if !self.enabled {
            return;
        }
        if now >= self.next_beacon {
            self.next_beacon = now + self.beacon_interval_ms;
            let bb = BeaconBody { timestamp: now * 1000, interval: 100, cap: self.cap(), ies: self.ies(false) };
            let b = self.bssid;
            self.send_mgmt(Mgmt::new(frame::BROADCAST, b, b, MgmtBody::Beacon(bb)));
        }
        // EAPOL retransmissions.
        let retx = match &self.sta {
            Some(s) if matches!(s.state, StaState::WaitM2 | StaState::WaitM4) && now >= s.next_retx => {
                Some(s.state.clone())
            }
            _ => None,
        };
        if let Some(st) = retx {
            let tries = self.sta.as_ref().unwrap().tries;
            if tries >= self.max_tries {
                self.deauth(frame::REASON_4WAY_TIMEOUT);
            } else if st == StaState::WaitM2 {
                self.send_m1(now, false);
            } else {
                self.send_m3(now);
            }
        }
    }

    /// Deauthenticate the station.
    pub fn deauth(&mut self, reason: u16) {
        if let Some(s) = self.sta.take() {
            let b = self.bssid;
            self.send_mgmt(Mgmt::new(s.mac, b, b, MgmtBody::Deauth { reason }));
        }
    }

    fn eapol_to_sta(&mut self, payload: &[u8]) {
        let Some(s) = &self.sta else { return };
        let eth = frame::ethernet(&s.mac, &self.bssid, frame::ETHERTYPE_EAPOL, payload);
        self.downlink(&eth, true);
    }

    fn send_m1(&mut self, now: u64, fresh: bool) {
        let Some(s) = self.sta.as_mut() else { return };
        if fresh {
            self.nonce_ctr = self.nonce_ctr.wrapping_add(1);
            for (i, n) in s.anonce.iter_mut().enumerate() {
                *n = (i as u8).wrapping_mul(7) ^ self.nonce_ctr ^ self.bssid[5];
            }
            s.tries = 0;
        }
        s.replay += 1;
        s.tries += 1;
        s.state = StaState::WaitM2;
        s.next_retx = now + self.retx_timeout_ms;
        let mut m1 = KeyFrame::new(2, eapol::KI_VERSION_AES_SHA1 | eapol::KI_PAIRWISE | eapol::KI_ACK);
        m1.key_len = 16;
        m1.replay_counter = s.replay;
        m1.nonce = s.anonce;
        let b = m1.to_bytes();
        self.eapol_to_sta(&b);
    }

    fn send_m3(&mut self, now: u64) {
        let rsn = self.m3_rsn_override.clone().unwrap_or_else(|| self.rsn_ie());
        let gtk_kde = eapol::gtk_kde(self.gtk_id, true, &self.gtk);
        let rsc = self.gtk_pn.current();
        let corrupt = self.corrupt_m3_mic;
        let Some(s) = self.sta.as_mut() else { return };
        let Some(tptk) = s.tptk.clone() else { return };
        s.replay += 1;
        s.tries += 1;
        s.state = StaState::WaitM4;
        s.next_retx = now + self.retx_timeout_ms;
        let mut kd = rsn;
        kd.extend_from_slice(&gtk_kde);
        let wrapped = crypto::aes_wrap(&tptk.kek, &eapol::pad_key_data(kd)).unwrap();
        let mut m3 = KeyFrame::new(
            2,
            eapol::KI_VERSION_AES_SHA1
                | eapol::KI_PAIRWISE
                | eapol::KI_INSTALL
                | eapol::KI_ACK
                | eapol::KI_MIC
                | eapol::KI_SECURE
                | eapol::KI_ENC_KEY_DATA,
        );
        m3.key_len = 16;
        m3.replay_counter = s.replay;
        m3.nonce = s.anonce;
        m3.rsc = rsc;
        m3.key_data = wrapped;
        let mut b = m3.to_signed_bytes(&tptk.kck);
        if corrupt {
            b[eapol::MIC_OFFSET] ^= 0xff;
        }
        self.m3_sent += 1;
        if self.drop_m3 > 0 {
            self.drop_m3 -= 1;
            return;
        }
        self.eapol_to_sta(&b);
    }

    /// Start a group-key handshake with a new GTK.
    pub fn rekey_group(&mut self) {
        self.gtk_id = if self.gtk_id == 1 { 2 } else { 1 };
        for g in self.gtk.iter_mut() {
            *g = g.wrapping_add(0x11);
        }
        self.gtk_pn = TxPn::new();
        let gtk_kde = eapol::gtk_kde(self.gtk_id, true, &self.gtk);
        let Some(s) = self.sta.as_mut() else { return };
        let Some(ptk) = s.ptk.clone() else { return };
        s.replay += 1;
        s.group_pending = true;
        let wrapped = crypto::aes_wrap(&ptk.kek, &eapol::pad_key_data(gtk_kde)).unwrap();
        let mut g1 = KeyFrame::new(
            2,
            eapol::KI_VERSION_AES_SHA1 | eapol::KI_ACK | eapol::KI_MIC | eapol::KI_SECURE | eapol::KI_ENC_KEY_DATA,
        );
        g1.key_len = 16;
        g1.replay_counter = s.replay;
        g1.key_data = wrapped;
        let b = g1.to_signed_bytes(&ptk.kck);
        self.eapol_to_sta(&b);
    }

    /// Send an Ethernet frame from the wired side towards the station (unicast with the
    /// TK, group-addressed with the GTK). `allow_plain` lets EAPOL out before keys exist.
    pub fn downlink(&mut self, eth: &[u8], allow_plain: bool) {
        let mut mpdu = frame::ethernet_to_data_fromds(eth, &self.bssid).unwrap();
        frame::set_seq(&mut mpdu, self.seq);
        self.seq = (self.seq + 1) & 0xfff;
        let group = frame::is_multicast(&mpdu[4..10].try_into().unwrap());
        let out = if group {
            if self.pmk.is_some() {
                let pn = self.gtk_pn.next().unwrap();
                crypto::ccmp_encrypt(&self.gtk, &mpdu, pn, self.gtk_id).unwrap()
            } else {
                mpdu
            }
        } else {
            match self.sta.as_mut().and_then(|s| s.ptk.clone().map(|p| (p, s))) {
                Some((p, s)) => {
                    let pn = s.tx_pn.next().unwrap();
                    crypto::ccmp_encrypt(&p.tk, &mpdu, pn, 0).unwrap()
                }
                None if allow_plain || self.pmk.is_none() => mpdu,
                None => return,
            }
        };
        self.out.push_back(out);
    }

    /// Receive a frame from the air.
    pub fn on_rx(&mut self, f: &[u8], now: u64) {
        if !self.enabled || f.len() < 24 {
            return;
        }
        match frame::frame_type(f) {
            frame::TYPE_MGMT => self.on_mgmt(f, now),
            frame::TYPE_DATA => self.on_data(f, now),
            _ => {}
        }
    }

    fn on_mgmt(&mut self, f: &[u8], now: u64) {
        let Some(m) = Mgmt::parse(f) else { return };
        let b = self.bssid;
        let for_us = m.hdr.addr1 == b || m.hdr.addr1 == frame::BROADCAST;
        if !for_us {
            return;
        }
        let sa = m.hdr.addr2;
        match &m.body {
            MgmtBody::ProbeReq { ies } => {
                let ssid = frame::find_ie(ies, frame::IE_SSID).unwrap_or(&[]);
                if (ssid.is_empty() && !self.hidden) || ssid == &self.ssid[..] {
                    self.probe_reqs += 1;
                    let bb = BeaconBody { timestamp: now * 1000, interval: 100, cap: self.cap(), ies: self.ies(true) };
                    self.send_mgmt(Mgmt::new(sa, b, b, MgmtBody::ProbeResp(bb)));
                }
            }
            MgmtBody::Auth { algo: 0, seq: 1, .. } if m.hdr.addr1 == b => {
                self.sta = Some(StaCtx {
                    mac: sa,
                    state: StaState::Authenticated,
                    aid: 0,
                    anonce: [0; 32],
                    replay: 0,
                    tptk: None,
                    ptk: None,
                    tries: 0,
                    next_retx: 0,
                    tx_pn: TxPn::new(),
                    rx: ReplayCounters::new(),
                    reassoc_from: None,
                    group_pending: false,
                });
                self.send_mgmt(Mgmt::new(sa, b, b, MgmtBody::Auth { algo: 0, seq: 2, status: 0, ies: Vec::new() }));
            }
            MgmtBody::AssocReq { ies, .. } | MgmtBody::ReassocReq { ies, .. } if m.hdr.addr1 == b => {
                let reassoc = matches!(m.body, MgmtBody::ReassocReq { .. });
                let ok = self.sta.as_ref().map_or(false, |s| s.mac == sa);
                let rsn_ok = self.pmk.is_none() || frame::find_ie(ies, frame::IE_RSN).and_then(Rsn::parse).map_or(false, |r| r.supports_psk_ccmp());
                let status = if ok && rsn_ok { 0 } else { frame::STATUS_UNSPECIFIED };
                if reassoc {
                    self.reassoc_reqs += 1;
                }
                if let (Some(s), MgmtBody::ReassocReq { current_ap, .. }) = (self.sta.as_mut(), &m.body) {
                    s.reassoc_from = Some(*current_ap);
                }
                let aid = 1;
                let body = if reassoc {
                    MgmtBody::ReassocResp { cap: self.cap(), status, aid, ies: Vec::new() }
                } else {
                    MgmtBody::AssocResp { cap: self.cap(), status, aid, ies: Vec::new() }
                };
                self.send_mgmt(Mgmt::new(sa, b, b, body));
                if status == 0 {
                    let s = self.sta.as_mut().unwrap();
                    s.aid = aid;
                    s.state = StaState::Associated;
                    s.ptk = None;
                    s.tptk = None;
                    if self.pmk.is_some() {
                        self.send_m1(now, true);
                    } else {
                        s.state = StaState::Done;
                    }
                }
            }
            MgmtBody::Deauth { .. } | MgmtBody::Disassoc { .. } if m.hdr.addr1 == b => {
                self.deauths_rx += 1;
                if self.sta.as_ref().map_or(false, |s| s.mac == sa) {
                    self.sta = None;
                }
            }
            _ => {}
        }
    }

    fn on_data(&mut self, f: &[u8], now: u64) {
        let Some((h, _)) = frame::Header::parse(f) else { return };
        if !h.to_ds() || h.from_ds() || h.addr1 != self.bssid {
            return;
        }
        let Some(s) = self.sta.as_mut() else { return };
        if s.mac != h.addr2 {
            return;
        }
        let plain = if h.protected() {
            let Some(p) = s.ptk.clone() else { return };
            let Some((pn, _)) = crypto::ccmp_peek(f) else { return };
            if !s.rx.check(frame::tid(f), pn) {
                return;
            }
            let Some(d) = crypto::ccmp_decrypt(&p.tk, f) else { return };
            s.rx.update(frame::tid(f), pn);
            d.mpdu
        } else {
            f.to_vec()
        };
        let Some(eth) = frame::data_to_ethernet(&plain) else { return };
        if frame::ethertype(&eth) == Some(frame::ETHERTYPE_EAPOL) {
            self.on_eapol(&eth[14..], now);
        } else if s.state == StaState::Done && (h.protected() || self.pmk.is_none()) {
            self.wired_rx.push(eth);
        }
    }

    fn on_eapol(&mut self, raw: &[u8], now: u64) {
        let Some(kf) = KeyFrame::parse(raw) else { return };
        let Some(pmk) = self.pmk else { return };
        let b = self.bssid;
        let Some(s) = self.sta.as_mut() else { return };
        if kf.replay_counter != s.replay {
            return;
        }
        if kf.has(eapol::KI_PAIRWISE) && !kf.has(eapol::KI_SECURE) && s.state == StaState::WaitM2 {
            // M2
            let tptk = crypto::derive_ptk(&pmk, &b, &s.mac, &s.anonce, &kf.nonce);
            if !eapol::verify_mic_raw(&tptk.kck, raw) {
                self.mic_failures += 1;
                return;
            }
            s.tptk = Some(tptk);
            s.tries = 0;
            self.send_m3(now);
        } else if kf.has(eapol::KI_PAIRWISE) && kf.has(eapol::KI_SECURE) && s.state == StaState::WaitM4 {
            if self.drop_m4 > 0 {
                self.drop_m4 -= 1;
                return;
            }
            let Some(tptk) = s.tptk.clone() else { return };
            if !eapol::verify_mic_raw(&tptk.kck, raw) {
                self.mic_failures += 1;
                return;
            }
            self.m4_received += 1;
            s.ptk = Some(tptk);
            s.state = StaState::Done;
        } else if !kf.has(eapol::KI_PAIRWISE) && s.group_pending {
            let Some(ptk) = s.ptk.clone() else { return };
            if eapol::verify_mic_raw(&ptk.kck, raw) {
                s.group_pending = false;
            } else {
                self.mic_failures += 1;
            }
        }
    }

    pub fn sta_done(&self) -> bool {
        self.sta.as_ref().map_or(false, |s| s.state == StaState::Done)
    }
}
