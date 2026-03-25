//! Network state for a single WASM instance.
//!
//! Each instance can have multiple network interfaces, each with its own MAC.
//! Stored in `HostState::custom` via `insert_custom::<NetworkState>()`.

use std::sync::Arc;
use crate::hub::EthernetHub;

/// Network state for one computer instance (supports multiple interfaces).
pub struct NetworkState {
    /// MAC addresses for each interface, indexed by interface number.
    pub macs: Vec<[u8; 6]>,
    /// Shared reference to the ethernet hub.
    pub hub: Arc<EthernetHub>,
}

impl NetworkState {
    pub fn new(macs: Vec<[u8; 6]>, hub: Arc<EthernetHub>) -> Self {
        Self { macs, hub }
    }

    pub fn interface_count(&self) -> usize {
        self.macs.len()
    }

    pub fn mac_for(&self, index: usize) -> Option<&[u8; 6]> {
        self.macs.get(index)
    }
}

/// Generate a locally-administered MAC address for (instance, interface).
/// Layout: [0x02, instance, 0x00, 0x00, iface_hi, iface_lo]
pub fn mac_for_interface(instance_index: u8, interface_index: u16) -> [u8; 6] {
    [
        0x02,
        instance_index,
        0x00,
        0x00,
        (interface_index >> 8) as u8,
        (interface_index & 0xFF) as u8,
    ]
}
