//! Tun-style virtual interfaces: a program creates a kernel network
//! interface (e.g. `radio0`) whose Ethernet frames it carries itself. Used by
//! `radiod` to run IP over packet radio.
//!
//! It rides on the ordinary socket calls (no new host functions):
//!
//! 1. `socket(AF_ECM_TUN, SOCK_RAW, 0)`
//! 2. `setsockopt(fd, SOL_TUN, TUN_SETIFF, setiff_value(name, mac, ip, prefix))`
//!    creates the interface (link up, address configured if `ip` != 0).
//! 3. `send(fd, frame)` delivers an Ethernet frame to the kernel as if it
//!    arrived on the interface; `recv(fd, buf, 0)` takes the next frame the
//!    kernel transmitted on it, or returns [`TUN_EMPTY`] at once (non-blocking).
//! 4. `close(fd)` (or the program exiting) removes the interface.
//!
//! The wrappers that call host functions are `#[inline]` so native kernel
//! test builds (which link this crate without the host) never codegen them.

extern crate alloc;

use alloc::vec::Vec;

/// Socket domain for tun interfaces (ECM-specific; outside Linux's range).
pub const AF_ECM_TUN: i32 = 1024;
/// setsockopt level for tun options.
pub const SOL_TUN: i32 = 1024;
/// setsockopt: create/attach the interface (value from [`setiff_value`]).
pub const TUN_SETIFF: i32 = 1;
/// recv status when no frame is waiting.
pub const TUN_EMPTY: i32 = -2;
/// Kernel socket ids of tun sockets start here (ordinary sockets are < 64).
pub const TUN_ID_BASE: i32 = 0x1000;
/// Most frames queued towards the program before the oldest is dropped.
pub const TUN_QUEUE: usize = 64;

/// `[mac 6][ip 4][prefix 1][name...]`.
pub fn setiff_value(name: &str, mac: [u8; 6], ip: [u8; 4], prefix: u8) -> Vec<u8> {
    let mut v = Vec::with_capacity(11 + name.len());
    v.extend_from_slice(&mac);
    v.extend_from_slice(&ip);
    v.push(prefix);
    v.extend_from_slice(name.as_bytes());
    v
}

/// Inverse of [`setiff_value`]: (name, mac, ip, prefix).
pub fn parse_setiff(v: &[u8]) -> Option<(&str, [u8; 6], [u8; 4], u8)> {
    if v.len() < 12 {
        return None;
    }
    let mac = [v[0], v[1], v[2], v[3], v[4], v[5]];
    let ip = [v[6], v[7], v[8], v[9]];
    let name = core::str::from_utf8(&v[11..]).ok()?;
    Some((name, mac, ip, v[10]))
}

/// Create a tun interface; returns the socket fd or a negative error.
#[inline]
pub fn open(name: &str, mac: [u8; 6], ip: [u8; 4], prefix: u8) -> i32 {
    let fd = crate::socket::socket(AF_ECM_TUN, crate::socket::SOCK_RAW, 0);
    if fd < 0 {
        return fd;
    }
    let rc = crate::socket::setsockopt(fd, SOL_TUN, TUN_SETIFF, &setiff_value(name, mac, ip, prefix));
    if rc < 0 {
        crate::socket::close(fd);
        return rc;
    }
    fd
}

/// Hand a received frame to the kernel.
#[inline]
pub fn write(fd: i32, frame: &[u8]) -> i32 {
    crate::socket::send(fd, frame, 0)
}

/// Next frame the kernel sent on the interface: its length, [`TUN_EMPTY`],
/// or another negative error.
#[inline]
pub fn read(fd: i32, buf: &mut [u8]) -> i32 {
    crate::socket::recv(fd, buf, 0)
}
