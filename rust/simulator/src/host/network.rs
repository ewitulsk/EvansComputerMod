//! Multi-interface network host functions: raw ethernet frame send/receive.

use wasmtime::*;
use crate::wasm_host::HostState;
use crate::network::NetworkState;
use super::memory;

/// Host function names registered by this module.
pub const FUNCTIONS: &[&str] = &[
    "net_get_interface_count",
    "net_get_interface_mac",
    "net_tx_frame_on",
    "net_rx_frame_on",
    "net_rx_frame_any",
    "net_set_promiscuous_on",
    "net_set_link_state",
];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    // net_get_interface_count() -> i32
    linker.func_wrap("env", "net_get_interface_count", |caller: Caller<'_, HostState>| -> i32 {
        match caller.data().get_custom::<NetworkState>() {
            Some(net) => net.interface_count() as i32,
            None => 0,
        }
    })?;

    // net_get_interface_mac(index, buf_ptr) -> i32
    // Writes 6-byte MAC for interface at `index`. Returns 6, or -1 if out of range.
    linker.func_wrap("env", "net_get_interface_mac",
        |mut caller: Caller<'_, HostState>, index: i32, buf_ptr: i32| -> i32 {
            let mac = match caller.data().get_custom::<NetworkState>() {
                Some(net) => match net.mac_for(index as usize) {
                    Some(m) => *m,
                    None => return -1,
                },
                None => return -1,
            };
            memory::write_bytes(&mut caller, buf_ptr, &mac);
            6
        },
    )?;

    // net_tx_frame_on(index, buf_ptr, frame_len) -> i32
    // Transmit ethernet frame on specific interface. Returns 0 or -1.
    linker.func_wrap("env", "net_tx_frame_on",
        |mut caller: Caller<'_, HostState>, index: i32, buf_ptr: i32, frame_len: i32| -> i32 {
            if frame_len < 14 || frame_len > 1518 {
                return -1;
            }
            let frame = match memory::read_bytes(&mut caller, buf_ptr, frame_len) {
                Some(f) => f,
                None => return -1,
            };
            let (mac, hub) = match caller.data().get_custom::<NetworkState>() {
                Some(net) => match net.mac_for(index as usize) {
                    Some(m) => (*m, net.hub.clone()),
                    None => return -1,
                },
                None => return -1,
            };
            hub.transmit(&mac, &frame);
            0
        },
    )?;

    // net_rx_frame_on(index, buf_ptr, buf_len) -> i32
    // Non-blocking receive on specific interface. Returns frame length or -1.
    linker.func_wrap("env", "net_rx_frame_on",
        |mut caller: Caller<'_, HostState>, index: i32, buf_ptr: i32, buf_len: i32| -> i32 {
            let (mac, hub) = match caller.data().get_custom::<NetworkState>() {
                Some(net) => match net.mac_for(index as usize) {
                    Some(m) => (*m, net.hub.clone()),
                    None => return -1,
                },
                None => return -1,
            };
            match hub.receive(&mac) {
                Some(frame) => {
                    let write_len = frame.len().min(buf_len as usize);
                    memory::write_bytes(&mut caller, buf_ptr, &frame[..write_len]);
                    write_len as i32
                }
                None => -1,
            }
        },
    )?;

    // net_rx_frame_any(buf_ptr, buf_len, iface_idx_ptr) -> i32
    // Non-blocking receive from any interface. Returns frame length or -1.
    // Writes the interface index (as little-endian i32) to iface_idx_ptr.
    linker.func_wrap("env", "net_rx_frame_any",
        |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32, iface_idx_ptr: i32| -> i32 {
            let (macs, hub) = match caller.data().get_custom::<NetworkState>() {
                Some(net) => (net.macs.clone(), net.hub.clone()),
                None => return -1,
            };

            for (idx, mac) in macs.iter().enumerate() {
                if let Some(frame) = hub.receive(mac) {
                    let write_len = frame.len().min(buf_len as usize);
                    memory::write_bytes(&mut caller, buf_ptr, &frame[..write_len]);
                    memory::write_bytes(&mut caller, iface_idx_ptr, &(idx as i32).to_le_bytes());
                    return write_len as i32;
                }
            }
            -1
        },
    )?;

    // net_set_promiscuous_on(index, enabled) -> i32
    linker.func_wrap("env", "net_set_promiscuous_on",
        |caller: Caller<'_, HostState>, index: i32, enabled: i32| -> i32 {
            let (mac, hub) = match caller.data().get_custom::<NetworkState>() {
                Some(net) => match net.mac_for(index as usize) {
                    Some(m) => (*m, net.hub.clone()),
                    None => return -1,
                },
                None => return -1,
            };
            hub.set_promiscuous(&mac, enabled != 0);
            0
        },
    )?;

    // net_set_link_state(index, up) -> i32
    // Notifies the host of link state changes (no-op in simulator — no visual blocks).
    linker.func_wrap("env", "net_set_link_state",
        |_caller: Caller<'_, HostState>, _index: i32, _up: i32| -> i32 {
            0 // no-op
        },
    )?;

    Ok(())
}
