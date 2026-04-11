//! `NetTools` backend for WASI programs. Delegates every operation to the
//! host ABI (`ecm-host-abi::socket` / `ecm-host-abi::netlink`) and writes
//! output through standard stdout/stderr so the existing shell continues to
//! see identical behavior after programs are refactored onto the trait.

use crate::{icmp, NetTools};
use ecm_host_abi::netlink::{NlMsgHdr, NLMSG_DONE, NLMSG_HDR_SIZE};
use ecm_host_abi::socket::{
    self, SockAddrIn, AF_INET, AF_NETLINK, IPPROTO_ICMP, NETLINK_ROUTE, SOCK_DGRAM, SOCK_RAW,
};
use std::io::Write as _;

#[link(wasm_import_module = "env")]
extern "C" {
    fn get_time_ms() -> i64;
}

/// WASI-side `NetTools` implementation. Lazily opens a netlink socket and a
/// raw ICMP socket on first use and keeps them around for the lifetime of
/// the program.
pub struct WasiNetTools {
    nl_fd: i32,
    icmp_fd: i32,
}

impl WasiNetTools {
    pub fn new() -> Self {
        Self {
            nl_fd: -1,
            icmp_fd: -1,
        }
    }

    fn ensure_nl(&mut self) -> i32 {
        if self.nl_fd < 0 {
            self.nl_fd = socket::socket(AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE);
        }
        self.nl_fd
    }

    fn ensure_icmp(&mut self) -> i32 {
        if self.icmp_fd < 0 {
            self.icmp_fd = socket::socket(AF_INET, SOCK_RAW, IPPROTO_ICMP);
        }
        self.icmp_fd
    }
}

impl Default for WasiNetTools {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WasiNetTools {
    fn drop(&mut self) {
        if self.nl_fd >= 0 {
            socket::close(self.nl_fd);
        }
        if self.icmp_fd >= 0 {
            socket::close(self.icmp_fd);
        }
    }
}

impl NetTools for WasiNetTools {
    fn out(&mut self, s: &str) {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        let _ = lock.write_all(s.as_bytes());
        let _ = lock.flush();
    }

    fn err(&mut self, s: &str) {
        let stderr = std::io::stderr();
        let mut lock = stderr.lock();
        let _ = lock.write_all(s.as_bytes());
        let _ = lock.flush();
    }

    fn now_ms(&mut self) -> i64 {
        unsafe { get_time_ms() }
    }

    fn sleep_ms(&mut self, ms: u32) {
        std::thread::sleep(std::time::Duration::from_millis(ms as u64));
    }

    fn netlink_request(&mut self, req: &[u8]) -> Vec<u8> {
        let fd = self.ensure_nl();
        if fd < 0 {
            return Vec::new();
        }
        let nl_addr = SockAddrIn::default();
        socket::sendto(fd, req, 0, &nl_addr);

        let mut response = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let mut from = SockAddrIn::default();
            let n = socket::recvfrom(fd, &mut buf, 0, &mut from);
            if n <= 0 {
                break;
            }
            response.extend_from_slice(&buf[..n as usize]);

            // Stop once we've seen NLMSG_DONE or NLMSG_ERROR in the response.
            let mut check = 0;
            let mut done = false;
            while check + NLMSG_HDR_SIZE <= response.len() {
                if let Some(h) = NlMsgHdr::parse(&response[check..]) {
                    if h.nlmsg_type == NLMSG_DONE || h.nlmsg_type == 2 /* NLMSG_ERROR */ {
                        done = true;
                        break;
                    }
                    let step = ecm_host_abi::netlink::nlmsg_align(h.nlmsg_len as usize);
                    if step == 0 {
                        break;
                    }
                    check += step;
                } else {
                    break;
                }
            }
            if done {
                break;
            }
        }
        response
    }

    fn getaddrinfo(&mut self, host: &str) -> Option<[u8; 4]> {
        let mut addr = SockAddrIn::default();
        if socket::getaddrinfo(host, &mut addr) == 0 {
            Some(addr.sin_addr)
        } else {
            None
        }
    }

    fn icmp_echo(&mut self, dst: [u8; 4], id: u16, seq: u16, _timeout_ms: u32) -> Option<u32> {
        let fd = self.ensure_icmp();
        if fd < 0 {
            return None;
        }
        let pkt = icmp::build_echo_packet(id, seq);
        let target = SockAddrIn::from_ip_port(dst, 0);
        let start = unsafe { get_time_ms() };
        let n = socket::sendto(fd, &pkt, 0, &target);
        if n < 0 {
            return None;
        }
        let mut reply = [0u8; 128];
        let mut from = SockAddrIn::default();
        let n = socket::recvfrom(fd, &mut reply, 0, &mut from);
        if n > 0 {
            let elapsed = unsafe { get_time_ms() } - start;
            Some(elapsed.max(0) as u32)
        } else {
            None
        }
    }
}
