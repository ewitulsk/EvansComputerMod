//! Network configuration host function wrappers.
//!
//! These host functions allow WASI programs to query and configure the
//! kernel's network interfaces, routing table, and DNS settings.

extern crate alloc;
use alloc::string::String;
use alloc::vec;

extern "C" {
    fn ipc_net_iface_count() -> i32;
    fn ipc_net_iface_info(index: i32, buf_ptr: i32, buf_len: i32) -> i32;
    fn ipc_net_iface_configure(index: i32, ip_ptr: i32, ip_len: i32, prefix: i32) -> i32;
    fn ipc_net_iface_deconfigure(index: i32) -> i32;
    fn ipc_net_iface_set_link(index: i32, up: i32) -> i32;
    fn ipc_net_iface_set_vlan(index: i32, vlan_id: i32) -> i32;
    fn ipc_net_route_list(buf_ptr: i32, buf_len: i32) -> i32;
    fn ipc_net_route_add(dest_ptr: i32, dest_len: i32, prefix: i32, gw_ptr: i32, gw_len: i32, iface_idx: i32) -> i32;
    fn ipc_net_route_del(dest_ptr: i32, dest_len: i32, prefix: i32) -> i32;
    fn ipc_net_ping(ip_ptr: i32, ip_len: i32, timeout_ms: i32) -> i32;
    fn ipc_net_dns_get(buf_ptr: i32, buf_len: i32) -> i32;
    fn ipc_net_dns_set(ip_ptr: i32, ip_len: i32) -> i32;
    fn ipc_net_save_config() -> i32;
}

/// Get the number of network interfaces.
pub fn iface_count() -> i32 {
    unsafe { ipc_net_iface_count() }
}

/// Get info about interface at `index` as a JSON string.
/// Returns JSON like: {"name":"eth0","mac":"02:00:00:00:00:00","ip":"10.0.0.1","prefix":24,"link_up":true,"vlan":null}
pub fn iface_info(index: i32) -> Option<String> {
    let mut buf = vec![0u8; 512];
    let n = unsafe { ipc_net_iface_info(index, buf.as_mut_ptr() as i32, buf.len() as i32) };
    if n <= 0 {
        None
    } else {
        buf.truncate(n as usize);
        String::from_utf8(buf).ok()
    }
}

/// Configure interface with IP address and prefix length.
/// `ip` is a dotted-quad string like "10.0.0.1".
pub fn iface_configure(index: i32, ip: &str, prefix: u8) -> i32 {
    unsafe { ipc_net_iface_configure(index, ip.as_ptr() as i32, ip.len() as i32, prefix as i32) }
}

/// Remove IP configuration from interface.
pub fn iface_deconfigure(index: i32) -> i32 {
    unsafe { ipc_net_iface_deconfigure(index) }
}

/// Set interface link state. `up` = true for up, false for down.
pub fn iface_set_link(index: i32, up: bool) -> i32 {
    unsafe { ipc_net_iface_set_link(index, if up { 1 } else { 0 }) }
}

/// Set VLAN on interface. Pass -1 to disable VLAN.
pub fn iface_set_vlan(index: i32, vlan_id: i32) -> i32 {
    unsafe { ipc_net_iface_set_vlan(index, vlan_id) }
}

/// Get the routing table as a JSON array string.
/// Returns JSON like: [{"dest":"10.0.0.0","prefix":24,"gateway":"0.0.0.0","iface_idx":0}]
pub fn route_list() -> Option<String> {
    let mut buf = vec![0u8; 2048];
    let n = unsafe { ipc_net_route_list(buf.as_mut_ptr() as i32, buf.len() as i32) };
    if n <= 0 {
        None
    } else {
        buf.truncate(n as usize);
        String::from_utf8(buf).ok()
    }
}

/// Add a route. `dest` is dotted-quad, `prefix` is CIDR prefix length,
/// `gateway` is dotted-quad (use "0.0.0.0" for connected routes).
pub fn route_add(dest: &str, prefix: u8, gateway: &str, iface_idx: i32) -> i32 {
    unsafe {
        ipc_net_route_add(
            dest.as_ptr() as i32, dest.len() as i32,
            prefix as i32,
            gateway.as_ptr() as i32, gateway.len() as i32,
            iface_idx,
        )
    }
}

/// Delete a route by destination and prefix.
pub fn route_del(dest: &str, prefix: u8) -> i32 {
    unsafe { ipc_net_route_del(dest.as_ptr() as i32, dest.len() as i32, prefix as i32) }
}

/// Send an ICMP ping to `ip` (dotted-quad string) with timeout in ms.
/// Returns RTT in ms on success, -1 on failure/timeout.
pub fn ping(ip: &str, timeout_ms: i32) -> i32 {
    unsafe { ipc_net_ping(ip.as_ptr() as i32, ip.len() as i32, timeout_ms) }
}

/// Get the current DNS server address as a string.
pub fn dns_get() -> Option<String> {
    let mut buf = vec![0u8; 64];
    let n = unsafe { ipc_net_dns_get(buf.as_mut_ptr() as i32, buf.len() as i32) };
    if n <= 0 {
        None
    } else {
        buf.truncate(n as usize);
        String::from_utf8(buf).ok()
    }
}

/// Set the DNS server. `ip` is a dotted-quad string.
pub fn dns_set(ip: &str) -> i32 {
    unsafe { ipc_net_dns_set(ip.as_ptr() as i32, ip.len() as i32) }
}

/// Persist the current network configuration (interfaces, routes, DNS) to disk.
pub fn save_config() -> i32 {
    unsafe { ipc_net_save_config() }
}
