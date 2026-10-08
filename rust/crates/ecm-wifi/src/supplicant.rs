//! WPA2-PSK supplicant logic (station side of the 4-way and group-key handshakes).
//!
//! The supplicant consumes EAPOL frames (as carried in Ethernet frames with EtherType
//! 0x888E, i.e. what a `wpa_supplicant` program reads from an `AF_PACKET` socket) and
//! produces [`SupplicantOutput`]s: EAPOL frames to send back to the authenticator, key
//! installation requests for the kernel ([`SupplicantOutput::InstallPtk`] /
//! [`SupplicantOutput::InstallGtk`]), disconnect requests and status events.
//!
//! It holds no I/O and draws randomness only through the closure passed to
//! [`Supplicant::new`], so the same code runs inside a userspace program, inside the
//! kernel, or in-process in tests.
//!
//! Hardening:
//! * the MIC of M3 / group M1 is verified before anything in them is used;
//! * EAPOL-Key replay counters must strictly increase;
//! * the RSN element in M3 must match the one in the AP's beacon (downgrade check);
//! * an identical PTK/GTK is never reinstalled (key reinstallation / KRACK defence).

use std::collections::VecDeque;

use crate::crypto::{self, Ptk};
use crate::eapol::{self, KeyFrame};
use crate::frame::{BssInfo, MacAddr, Security};
use crate::mlme::{ConnectParams, ConnectSecurity, Event};

/// One configured network (a `network={...}` block).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkConfig {
    pub ssid: Vec<u8>,
    /// Pre-shared key (PMK). `None` = open network.
    pub psk: Option<[u8; 32]>,
    /// Lock to one AP.
    pub bssid: Option<MacAddr>,
    /// Higher wins during network selection.
    pub priority: i32,
    pub disabled: bool,
}

/// Why a network configuration was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// Passphrases are 8..=63 printable ASCII characters.
    BadPassphrase,
    /// SSIDs are 1..=32 bytes.
    BadSsid,
    /// A raw PSK is 64 hex digits.
    BadPsk,
}

impl NetworkConfig {
    pub fn open(ssid: &[u8]) -> Result<Self, ConfigError> {
        check_ssid(ssid)?;
        Ok(NetworkConfig { ssid: ssid.to_vec(), psk: None, bssid: None, priority: 0, disabled: false })
    }

    /// WPA2-PSK network from a passphrase (PMK derived here, 4096 PBKDF2 rounds).
    pub fn wpa2_passphrase(ssid: &[u8], passphrase: &str) -> Result<Self, ConfigError> {
        check_ssid(ssid)?;
        let p = passphrase.as_bytes();
        if p.len() < 8 || p.len() > 63 || !p.iter().all(|&c| (0x20..=0x7e).contains(&c)) {
            return Err(ConfigError::BadPassphrase);
        }
        let psk = crypto::pmk_from_passphrase(p, ssid);
        Ok(NetworkConfig { ssid: ssid.to_vec(), psk: Some(psk), bssid: None, priority: 0, disabled: false })
    }

    /// WPA2-PSK network from a raw 256-bit PSK.
    pub fn wpa2_psk(ssid: &[u8], psk: [u8; 32]) -> Result<Self, ConfigError> {
        check_ssid(ssid)?;
        Ok(NetworkConfig { ssid: ssid.to_vec(), psk: Some(psk), bssid: None, priority: 0, disabled: false })
    }

    /// WPA2-PSK network from 64 hex digits (`psk=` without quotes).
    pub fn wpa2_psk_hex(ssid: &[u8], hex: &str) -> Result<Self, ConfigError> {
        let h = hex.as_bytes();
        if h.len() != 64 {
            return Err(ConfigError::BadPsk);
        }
        let mut psk = [0u8; 32];
        for i in 0..32 {
            let s = core::str::from_utf8(&h[2 * i..2 * i + 2]).map_err(|_| ConfigError::BadPsk)?;
            psk[i] = u8::from_str_radix(s, 16).map_err(|_| ConfigError::BadPsk)?;
        }
        Self::wpa2_psk(ssid, psk)
    }

    pub fn security(&self) -> ConnectSecurity {
        if self.psk.is_some() {
            ConnectSecurity::Wpa2Psk
        } else {
            ConnectSecurity::Open
        }
    }

    /// The connect request for the MLME.
    pub fn connect_params(&self) -> ConnectParams {
        let mut p = ConnectParams::new(&self.ssid, self.security());
        p.bssid = self.bssid;
        p
    }

    fn matches(&self, b: &BssInfo) -> bool {
        if self.disabled || b.ssid != self.ssid {
            return false;
        }
        if let Some(l) = self.bssid {
            if l != b.bssid {
                return false;
            }
        }
        match self.security() {
            ConnectSecurity::Open => b.security() == Security::Open,
            ConnectSecurity::Wpa2Psk => b.security() == Security::Wpa2Psk,
        }
    }
}

fn check_ssid(ssid: &[u8]) -> Result<(), ConfigError> {
    if ssid.is_empty() || ssid.len() > 32 {
        Err(ConfigError::BadSsid)
    } else {
        Ok(())
    }
}

/// Status notifications.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SupplicantEvent {
    /// Port authorized: 4-way handshake done (or open network associated).
    Completed { bssid: MacAddr },
    /// A new GTK was installed by the group key handshake.
    GroupRekey { key_id: u8 },
    /// An EAPOL-Key frame failed MIC verification (message 3 = M3, 5 = group M1).
    MicFailure { bssid: MacAddr, message: u8 },
    /// The RSN element in M3 differs from the beacon's (possible downgrade attack).
    RsnIeMismatch { bssid: MacAddr },
    /// The handshake did not complete in time.
    HandshakeTimeout { bssid: MacAddr },
    /// Disconnected during the 4-way handshake after sending M2: the PSK is probably wrong.
    PossibleWrongKey { bssid: MacAddr },
}

/// Output of the supplicant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SupplicantOutput {
    /// Send this EAPOL frame (no Ethernet header) to `dst` with EtherType 0x888E.
    SendEapol { dst: MacAddr, frame: Vec<u8> },
    /// Install the pairwise temporal key for `bssid` (CCMP, key id 0).
    InstallPtk { bssid: MacAddr, tk: [u8; 16] },
    /// Install a group temporal key; `rsc` is the receive sequence counter to start from.
    InstallGtk { key_id: u8, gtk: Vec<u8>, rsc: u64 },
    /// Ask the kernel to deauthenticate with this reason code.
    Disconnect { reason: u16 },
    Event(SupplicantEvent),
}

/// Handshake state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandshakeState {
    Disconnected,
    /// Associated, waiting for message 1.
    WaitM1,
    /// M2 sent, waiting for message 3.
    WaitM3,
    /// Keys installed, port authorized.
    Completed,
    Failed,
}

struct Assoc {
    bssid: MacAddr,
    pmk: Option<[u8; 32]>,
    ap_rsn_ie: Option<Vec<u8>>,
    sta_rsn_ie: Vec<u8>,
    started: u64,
    anonce: Option<[u8; 32]>,
    snonce: [u8; 32],
    m1_replay: u64,
    tptk: Option<Ptk>,
    ptk: Option<Ptk>,
    last_replay: Option<u64>,
    installed_tk: Option<[u8; 16]>,
    installed_gtk: Option<(u8, Vec<u8>)>,
}

/// The WPA2-PSK supplicant.
pub struct Supplicant {
    own: MacAddr,
    networks: Vec<NetworkConfig>,
    rand: Box<dyn FnMut(&mut [u8])>,
    assoc: Option<Assoc>,
    state: HandshakeState,
    outputs: VecDeque<SupplicantOutput>,
    /// Give up on a handshake after this long (ms).
    pub handshake_timeout_ms: u64,
}

impl Supplicant {
    /// `rand` fills buffers with cryptographically secure random bytes (SNonce).
    pub fn new(own: MacAddr, rand: Box<dyn FnMut(&mut [u8])>) -> Self {
        Supplicant {
            own,
            networks: Vec::new(),
            rand,
            assoc: None,
            state: HandshakeState::Disconnected,
            outputs: VecDeque::new(),
            handshake_timeout_ms: 10_000,
        }
    }

    pub fn add_network(&mut self, n: NetworkConfig) -> usize {
        self.networks.push(n);
        self.networks.len() - 1
    }
    pub fn networks(&self) -> &[NetworkConfig] {
        &self.networks
    }
    pub fn networks_mut(&mut self) -> &mut Vec<NetworkConfig> {
        &mut self.networks
    }
    pub fn state(&self) -> &HandshakeState {
        &self.state
    }
    pub fn bssid(&self) -> Option<MacAddr> {
        self.assoc.as_ref().map(|a| a.bssid)
    }
    pub fn pop_output(&mut self) -> Option<SupplicantOutput> {
        self.outputs.pop_front()
    }

    /// Network selection: the enabled network with the highest priority that is
    /// present in `results`, then the strongest BSS for it.
    pub fn select_network(&self, results: &[BssInfo]) -> Option<ConnectParams> {
        let mut best: Option<(&NetworkConfig, i32)> = None;
        for n in &self.networks {
            let Some(rssi) = results.iter().filter(|b| n.matches(b)).map(|b| b.rssi).max() else { continue };
            let better = match best {
                None => true,
                Some((bn, br)) => n.priority > bn.priority || (n.priority == bn.priority && rssi > br),
            };
            if better {
                best = Some((n, rssi));
            }
        }
        best.map(|(n, _)| n.connect_params())
    }

    /// Feed an MLME event (Connected / Disconnected) to the supplicant.
    pub fn on_event(&mut self, e: &Event, now: u64) {
        match e {
            Event::Connected { bssid, ssid, ap_rsn_ie, sta_rsn_ie, .. } => {
                self.on_associated(*bssid, ssid, ap_rsn_ie.as_deref(), sta_rsn_ie.as_deref(), now)
            }
            Event::Disconnected { .. } => self.on_disconnected(),
            _ => {}
        }
    }

    /// The station associated (or reassociated) with `bssid`.
    pub fn on_associated(
        &mut self,
        bssid: MacAddr,
        ssid: &[u8],
        ap_rsn_ie: Option<&[u8]>,
        sta_rsn_ie: Option<&[u8]>,
        now: u64,
    ) {
        let net = self.networks.iter().find(|n| n.ssid == ssid && !n.disabled);
        let pmk = net.and_then(|n| n.psk);
        self.assoc = Some(Assoc {
            bssid,
            pmk,
            ap_rsn_ie: ap_rsn_ie.map(|b| b.to_vec()),
            sta_rsn_ie: sta_rsn_ie.map(|b| b.to_vec()).unwrap_or_else(crate::mlme::Mlme::sta_rsn_ie),
            started: now,
            anonce: None,
            snonce: [0; 32],
            m1_replay: 0,
            tptk: None,
            ptk: None,
            last_replay: None,
            installed_tk: None,
            installed_gtk: None,
        });
        if pmk.is_none() || sta_rsn_ie.is_none() {
            // Open network: nothing to negotiate.
            self.state = HandshakeState::Completed;
            self.outputs.push_back(SupplicantOutput::Event(SupplicantEvent::Completed { bssid }));
        } else {
            self.state = HandshakeState::WaitM1;
        }
    }

    pub fn on_disconnected(&mut self) {
        if let Some(a) = &self.assoc {
            if self.state == HandshakeState::WaitM3 {
                self.outputs.push_back(SupplicantOutput::Event(SupplicantEvent::PossibleWrongKey { bssid: a.bssid }));
            }
        }
        self.assoc = None;
        self.state = HandshakeState::Disconnected;
    }

    /// Run timers.
    pub fn poll(&mut self, now: u64) -> Option<u64> {
        let Some(a) = &self.assoc else { return None };
        match self.state {
            HandshakeState::WaitM1 | HandshakeState::WaitM3 => {
                let deadline = a.started + self.handshake_timeout_ms;
                if now >= deadline {
                    let bssid = a.bssid;
                    self.state = HandshakeState::Failed;
                    self.outputs.push_back(SupplicantOutput::Event(SupplicantEvent::HandshakeTimeout { bssid }));
                    self.outputs.push_back(SupplicantOutput::Disconnect { reason: crate::frame::REASON_4WAY_TIMEOUT });
                    None
                } else {
                    Some(deadline)
                }
            }
            _ => None,
        }
    }

    /// Handle an Ethernet frame from the packet socket (only EtherType 0x888E is used).
    pub fn on_ethernet(&mut self, eth: &[u8], now: u64) {
        if crate::frame::ethertype(eth) != Some(crate::frame::ETHERTYPE_EAPOL) {
            return;
        }
        let mut src = [0u8; 6];
        src.copy_from_slice(&eth[6..12]);
        self.on_eapol(src, &eth[14..], now);
    }

    /// Handle an EAPOL frame from `src`.
    pub fn on_eapol(&mut self, src: MacAddr, raw: &[u8], _now: u64) {
        let Some(kf) = KeyFrame::parse(raw) else { return };
        let Some(a) = &self.assoc else { return };
        if src != a.bssid || a.pmk.is_none() {
            return;
        }
        if kf.descriptor_version() != eapol::KI_VERSION_AES_SHA1 {
            return; // only CCMP / HMAC-SHA1 (descriptor version 2) is supported
        }
        if kf.has(eapol::KI_REQUEST) || !kf.has(eapol::KI_ACK) {
            return; // not an authenticator message
        }
        if let Some(last) = a.last_replay {
            if kf.replay_counter <= last {
                return; // replayed
            }
        }
        if kf.has(eapol::KI_PAIRWISE) {
            if kf.has(eapol::KI_MIC) {
                self.handle_m3(&kf, raw);
            } else {
                self.handle_m1(&kf);
            }
        } else if kf.has(eapol::KI_MIC) {
            self.handle_group_m1(&kf, raw);
        }
    }

    fn handle_m1(&mut self, kf: &KeyFrame) {
        if self.state == HandshakeState::Failed {
            return;
        }
        let own = self.own;
        let Some(a) = self.assoc.as_mut() else { return };
        // A retransmitted M1 (same ANonce) reuses the SNonce, as wpa_supplicant does.
        if a.anonce != Some(kf.nonce) || a.tptk.is_none() {
            let mut sn = [0u8; 32];
            (self.rand)(&mut sn);
            a.snonce = sn;
        }
        a.anonce = Some(kf.nonce);
        a.m1_replay = kf.replay_counter;
        let pmk = a.pmk.expect("checked");
        let tptk = crypto::derive_ptk(&pmk, &a.bssid, &own, &kf.nonce, &a.snonce);
        let mut m2 = KeyFrame::new(kf.version, eapol::KI_VERSION_AES_SHA1 | eapol::KI_PAIRWISE | eapol::KI_MIC);
        m2.replay_counter = kf.replay_counter;
        m2.nonce = a.snonce;
        m2.key_data = a.sta_rsn_ie.clone();
        let bytes = m2.to_signed_bytes(&tptk.kck);
        a.tptk = Some(tptk);
        let dst = a.bssid;
        if self.state != HandshakeState::Completed {
            self.state = HandshakeState::WaitM3;
        }
        self.outputs.push_back(SupplicantOutput::SendEapol { dst, frame: bytes });
    }

    fn handle_m3(&mut self, kf: &KeyFrame, raw: &[u8]) {
        let Some(a) = self.assoc.as_mut() else { return };
        let bssid = a.bssid;
        if !(kf.has(eapol::KI_INSTALL) && kf.has(eapol::KI_ENC_KEY_DATA)) {
            return;
        }
        // M3 must belong to the handshake we answered.
        if a.anonce != Some(kf.nonce) {
            return;
        }
        let Some(tptk) = a.tptk.clone() else { return };
        if !eapol::verify_mic_raw(&tptk.kck, raw) {
            self.outputs.push_back(SupplicantOutput::Event(SupplicantEvent::MicFailure { bssid, message: 3 }));
            return;
        }
        a.last_replay = Some(kf.replay_counter);
        let Some(plain) = crypto::aes_unwrap(&tptk.kek, &kf.key_data) else {
            self.state = HandshakeState::Failed;
            self.outputs.push_back(SupplicantOutput::Disconnect { reason: crate::frame::REASON_UNSPECIFIED });
            return;
        };
        let kd = eapol::parse_key_data(&plain);
        if let (Some(beacon_ie), Some(m3_ie)) = (&a.ap_rsn_ie, &kd.rsn_ie) {
            if beacon_ie != m3_ie {
                self.state = HandshakeState::Failed;
                self.outputs.push_back(SupplicantOutput::Event(SupplicantEvent::RsnIeMismatch { bssid }));
                self.outputs.push_back(SupplicantOutput::Disconnect { reason: crate::frame::REASON_IE_MISMATCH });
                return;
            }
        }
        // M4
        let mut m4 =
            KeyFrame::new(kf.version, eapol::KI_VERSION_AES_SHA1 | eapol::KI_PAIRWISE | eapol::KI_MIC | eapol::KI_SECURE);
        m4.replay_counter = kf.replay_counter;
        let bytes = m4.to_signed_bytes(&tptk.kck);
        self.outputs.push_back(SupplicantOutput::SendEapol { dst: bssid, frame: bytes });
        // Install keys (never reinstall the same key: resets the PN otherwise).
        if a.installed_tk != Some(tptk.tk) {
            a.installed_tk = Some(tptk.tk);
            self.outputs.push_back(SupplicantOutput::InstallPtk { bssid, tk: tptk.tk });
        }
        if let Some(g) = kd.gtk {
            let cur = Some((g.key_id, g.gtk.clone()));
            if a.installed_gtk != cur {
                a.installed_gtk = cur;
                self.outputs.push_back(SupplicantOutput::InstallGtk { key_id: g.key_id, gtk: g.gtk, rsc: kf.rsc });
            }
        }
        a.ptk = Some(tptk);
        if self.state != HandshakeState::Completed {
            self.state = HandshakeState::Completed;
            self.outputs.push_back(SupplicantOutput::Event(SupplicantEvent::Completed { bssid }));
        }
    }

    fn handle_group_m1(&mut self, kf: &KeyFrame, raw: &[u8]) {
        if self.state != HandshakeState::Completed {
            return;
        }
        let Some(a) = self.assoc.as_mut() else { return };
        let bssid = a.bssid;
        let Some(ptk) = a.ptk.clone() else { return };
        if !kf.has(eapol::KI_SECURE) || !kf.has(eapol::KI_ENC_KEY_DATA) {
            return;
        }
        if !eapol::verify_mic_raw(&ptk.kck, raw) {
            self.outputs.push_back(SupplicantOutput::Event(SupplicantEvent::MicFailure { bssid, message: 5 }));
            return;
        }
        a.last_replay = Some(kf.replay_counter);
        let Some(plain) = crypto::aes_unwrap(&ptk.kek, &kf.key_data) else { return };
        let Some(g) = eapol::parse_key_data(&plain).gtk else { return };
        let mut m2 = KeyFrame::new(kf.version, eapol::KI_VERSION_AES_SHA1 | eapol::KI_MIC | eapol::KI_SECURE);
        m2.replay_counter = kf.replay_counter;
        let bytes = m2.to_signed_bytes(&ptk.kck);
        self.outputs.push_back(SupplicantOutput::SendEapol { dst: bssid, frame: bytes });
        let cur = Some((g.key_id, g.gtk.clone()));
        if a.installed_gtk != cur {
            a.installed_gtk = cur;
            let key_id = g.key_id;
            self.outputs.push_back(SupplicantOutput::InstallGtk { key_id, gtk: g.gtk, rsc: kf.rsc });
            self.outputs.push_back(SupplicantOutput::Event(SupplicantEvent::GroupRekey { key_id }));
        }
    }
}
