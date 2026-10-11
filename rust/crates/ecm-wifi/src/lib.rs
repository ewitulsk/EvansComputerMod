//! Sans-IO IEEE 802.11 station stack for EvansComputerMod (the Rust half of Linux's
//! mac80211 / cfg80211 / wpa_supplicant split).
//!
//! * [`crypto`]   — PBKDF2 PMK, PRF, PTK, EAPOL MIC, AES key wrap, CCMP, replay counters
//! * [`frame`]    — 802.11 header / IE / management / data codec, LLC/SNAP ↔ Ethernet, FCS
//! * [`eapol`]    — EAPOL-Key frames and KDEs
//! * [`radiotap`] — radiotap header writer / parser for monitor mode and pcaps
//! * [`rate`]     — b/g/n rate table and a minstrel-style rate controller
//! * [`mlme`]     — scan / auth / assoc / connection-monitor / roaming state machine
//! * [`supplicant`] — WPA2-PSK 4-way and group-key handshakes (station side)
//! * [`iface`]    — [`Wlan`]: the connection presented as an Ethernet interface
//!
//! Nothing here performs I/O, reads a clock, or draws randomness: callers pass `now_ms`
//! and inject a random-bytes closure, so the crate builds unchanged for the host,
//! `wasm32-unknown-unknown` (kernel) and `wasm32-wasip1` (programs).

pub mod crypto;
pub mod eapol;
pub mod frame;
pub mod iface;
pub mod mlme;
pub mod radiotap;
pub mod rate;
pub mod supplicant;

pub use frame::{BssInfo, MacAddr, Security};
pub use iface::{LinkInfo, RxOutput, TxFrame, Wlan, WlanStats};
pub use mlme::{
    Action, ConnectFailure, ConnectParams, ConnectSecurity, DisconnectReason, Event, LinkState, Mlme, MlmeConfig,
    RxFilter, ScanRequest,
};
pub use supplicant::{NetworkConfig, Supplicant, SupplicantEvent, SupplicantOutput};
