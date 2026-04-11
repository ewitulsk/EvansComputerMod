//! Kernel-mode `NetTools` backend for switch-os.
//!
//! Where the WASI backend goes through `sock_*`/netlink host calls, this
//! backend talks directly to the in-process `ecm_net::NetStack` and the
//! kernel netlink message handler moved into `ecm-kernel-core`. No IPC, no
//! file descriptors — just straight method calls.

use ecm_kernel_core::netlink::handle_netlink_message;
use ecm_net::types::Ipv4Addr;
use ecm_net::NetStack;
use ecm_net_tools::NetTools;

use crate::term;

extern "C" {
    fn get_time_ms() -> i64;
    fn sleep_ms(milliseconds: i32);
}

pub struct KernelNetTools;

impl KernelNetTools {
    pub fn new() -> Self {
        Self
    }
}

impl Default for KernelNetTools {
    fn default() -> Self {
        Self::new()
    }
}

impl NetTools for KernelNetTools {
    fn out(&mut self, s: &str) {
        term::print(s);
    }

    fn err(&mut self, s: &str) {
        // Switch-os has no separate stderr sink; errors go to the same VTE.
        term::print(s);
    }

    fn now_ms(&mut self) -> i64 {
        unsafe { get_time_ms() }
    }

    fn sleep_ms(&mut self, ms: u32) {
        unsafe { sleep_ms(ms as i32) }
    }

    fn netlink_request(&mut self, req: &[u8]) -> Vec<u8> {
        handle_netlink_message(req)
    }

    fn getaddrinfo(&mut self, host: &str) -> Option<[u8; 4]> {
        let stack = NetStack::get()?;
        match stack.dns_resolve(host, 5000) {
            Ok(ip) => Some(ip.0),
            Err(_) => None,
        }
    }

    fn icmp_echo(&mut self, dst: [u8; 4], _id: u16, _seq: u16, timeout_ms: u32) -> Option<u32> {
        let stack = NetStack::get()?;
        match stack.ping(Ipv4Addr(dst), timeout_ms) {
            Ok(rtt) => Some(rtt),
            Err(_) => None,
        }
    }
}
