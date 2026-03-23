//! Network host functions: raw ethernet frame send/receive.

use std::sync::atomic::Ordering;
use std::time::Duration;

use wasmtime::*;
use crate::wasm_host::HostState;
use crate::network::NetworkState;
use super::memory;

/// Host function names registered by this module.
pub const FUNCTIONS: &[&str] = &[
    "net_get_mac",
    "net_tx_frame",
    "net_rx_frame",
    "net_rx_frame_blocking",
    "net_set_promiscuous",
];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    // net_get_mac(buf_ptr) -> i32
    // Writes the 6-byte MAC address to the buffer, returns 6.
    linker.func_wrap("env", "net_get_mac", |mut caller: Caller<'_, HostState>, buf_ptr: i32| -> i32 {
        let mac = match caller.data().get_custom::<NetworkState>() {
            Some(net) => net.mac,
            None => return -1,
        };
        memory::write_bytes(&mut caller, buf_ptr, &mac);
        6
    })?;

    // net_tx_frame(buf_ptr, frame_len) -> i32
    // Reads an ethernet frame from WASM memory and transmits it via the hub.
    // Returns 0 on success, -1 on error.
    linker.func_wrap("env", "net_tx_frame", |mut caller: Caller<'_, HostState>, buf_ptr: i32, frame_len: i32| -> i32 {
        if frame_len < 14 || frame_len > 1514 {
            return -1;
        }

        let frame = match memory::read_bytes(&mut caller, buf_ptr, frame_len) {
            Some(f) => f,
            None => return -1,
        };

        let (mac, hub) = match caller.data().get_custom::<NetworkState>() {
            Some(net) => (net.mac, net.hub.clone()),
            None => return -1,
        };

        hub.transmit(&mac, &frame);
        0
    })?;

    // net_rx_frame(buf_ptr, buf_len) -> i32
    // Non-blocking receive. Returns frame length, or -1 if no frame available.
    linker.func_wrap("env", "net_rx_frame", |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32| -> i32 {
        let (mac, hub) = match caller.data().get_custom::<NetworkState>() {
            Some(net) => (net.mac, net.hub.clone()),
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
    })?;

    // net_rx_frame_blocking(buf_ptr, buf_len, timeout_ms) -> i32
    // Blocking receive with timeout. Polls in 10ms chunks respecting shutdown.
    // Returns frame length, or -1 on timeout/error.
    linker.func_wrap("env", "net_rx_frame_blocking",
        |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32, timeout_ms: i32| -> i32 {
            let timeout_ms = timeout_ms.clamp(0, 60_000);
            let shutdown = caller.data().shutdown.clone();

            let (mac, hub) = match caller.data().get_custom::<NetworkState>() {
                Some(net) => (net.mac, net.hub.clone()),
                None => return -1,
            };

            let mut remaining = timeout_ms;
            while remaining > 0 && !shutdown.load(Ordering::Relaxed) {
                let chunk = remaining.min(10);
                match hub.receive_blocking(&mac, Duration::from_millis(chunk as u64)) {
                    Some(frame) => {
                        let write_len = frame.len().min(buf_len as usize);
                        memory::write_bytes(&mut caller, buf_ptr, &frame[..write_len]);
                        return write_len as i32;
                    }
                    None => {
                        remaining -= chunk;
                    }
                }
            }

            -1
        },
    )?;

    // net_set_promiscuous(enabled) -> i32
    // Enable or disable promiscuous mode. Returns 0 on success.
    linker.func_wrap("env", "net_set_promiscuous", |caller: Caller<'_, HostState>, enabled: i32| -> i32 {
        let (mac, hub) = match caller.data().get_custom::<NetworkState>() {
            Some(net) => (net.mac, net.hub.clone()),
            None => return -1,
        };

        hub.set_promiscuous(&mac, enabled != 0);
        0
    })?;

    Ok(())
}
