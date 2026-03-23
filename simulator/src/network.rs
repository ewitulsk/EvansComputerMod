//! Network state for a single WASM instance.
//!
//! Stored in `HostState::custom` via `insert_custom::<NetworkState>()`.

use std::sync::Arc;
use crate::hub::EthernetHub;

/// Network state for one computer instance.
pub struct NetworkState {
    /// This computer's 6-byte MAC address.
    pub mac: [u8; 6],
    /// Shared reference to the ethernet hub.
    pub hub: Arc<EthernetHub>,
}

impl NetworkState {
    pub fn new(mac: [u8; 6], hub: Arc<EthernetHub>) -> Self {
        Self { mac, hub }
    }
}

/// Generate a locally-administered MAC address for a given instance index.
/// Uses the locally administered bit (bit 1 of first octet).
pub fn mac_for_instance(index: u8) -> [u8; 6] {
    [0x02, 0x00, 0x00, 0x00, 0x00, index]
}
