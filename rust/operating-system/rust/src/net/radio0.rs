//! Tun-style virtual interfaces (`radio0`): the kernel side of
//! `ecm_host_abi::tun`.
//!
//! A program (normally `radiod`, the KISS-TNC bridge) opens a socket of
//! domain `AF_ECM_TUN` and names an interface with `TUN_SETIFF`. The kernel
//! adds an Ethernet interface to the stack; everything the stack transmits
//! on it is queued for the program (`recv`, non-blocking), and every frame
//! the program `send`s enters the stack as if received on it. Closing the
//! socket, or the program exiting, removes the interface.
//!
//! These sockets travel over the ordinary socket syscalls, so no host
//! function changes are needed. `Kernel::sock_ipc` offers each call to
//! [`Tuns::ipc`] first: it claims `socket()` for its domain and any call on a
//! socket id at or above `TUN_ID_BASE`; everything else goes on to
//! `net::ipc` unchanged.

use std::collections::VecDeque;

use ecm_host_abi::tun::{parse_setiff, AF_ECM_TUN, SOL_TUN, TUN_EMPTY, TUN_ID_BASE, TUN_QUEUE, TUN_SETIFF};
use ecm_net::types::{Ipv4Addr, MacAddr};
use ecm_net::Stack;

// Socket syscall ids (match net::ipc / Java SocketFd).
const SOCK_SOCKET: i32 = 0;
const SOCK_BIND: i32 = 1;
const SOCK_CONNECT: i32 = 2;
const SOCK_LISTEN: i32 = 3;
const SOCK_ACCEPT: i32 = 4;
const SOCK_SEND: i32 = 5;
const SOCK_RECV: i32 = 6;
const SOCK_CLOSE: i32 = 7;
const SOCK_SETSOCKOPT: i32 = 8;
const SOCK_SENDTO: i32 = 9;
const SOCK_RECVFROM: i32 = 10;
const SOCK_GETSOCKNAME: i32 = 12;
const SOCK_GETPEERNAME: i32 = 13;
const SOCK_SHUTDOWN: i32 = 14;
const SOCK_DESTROY_SESSION: i32 = 99;

/// Most tun interfaces at once.
const MAX_TUNS: usize = 8;

struct Tun {
    pid: i32,
    /// Stack interface index once `TUN_SETIFF` succeeded.
    iface: Option<usize>,
    /// Frames the stack transmitted, waiting for the program.
    out: VecDeque<Vec<u8>>,
    dropped: u64,
}

/// Every tun socket in the kernel.
#[derive(Default)]
pub struct Tuns {
    tuns: Vec<Option<Tun>>,
}

fn i32_at(a: &[u8], off: usize) -> Option<i32> {
    a.get(off..off + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u16_at(a: &[u8], off: usize) -> Option<usize> {
    a.get(off..off + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
}

/// `[status i32][payload]` like net::ipc.
fn reply(result: &mut [u8], status: i32, payload: &[u8]) -> i32 {
    if result.len() < 4 {
        return -1;
    }
    result[..4].copy_from_slice(&status.to_le_bytes());
    let n = payload.len().min(result.len() - 4);
    result[4..4 + n].copy_from_slice(&payload[..n]);
    (4 + n) as i32
}

impl Tuns {
    pub fn new() -> Self {
        Self::default()
    }

    fn slot(&self, id: i32) -> Option<usize> {
        let s = id.checked_sub(TUN_ID_BASE)?;
        (s >= 0 && (s as usize) < self.tuns.len() && self.tuns[s as usize].is_some()).then_some(s as usize)
    }

    /// Interface indices backed by tun sockets.
    pub fn ifaces(&self) -> impl Iterator<Item = usize> + '_ {
        self.tuns.iter().flatten().filter_map(|t| t.iface)
    }

    /// The stack transmitted `frame` on `iface`: queue it for the program if
    /// that interface is a tun. Returns whether it was consumed.
    pub fn on_tx(&mut self, iface: usize, frame: &[u8]) -> bool {
        for t in self.tuns.iter_mut().flatten() {
            if t.iface == Some(iface) {
                if t.out.len() >= TUN_QUEUE {
                    t.out.pop_front();
                    t.dropped += 1;
                }
                t.out.push_back(frame.to_vec());
                return true;
            }
        }
        false
    }

    fn close(&mut self, stack: &mut Stack, slot: usize) {
        if let Some(t) = self.tuns[slot].take() {
            if let Some(i) = t.iface {
                stack.remove_interface(i);
            }
        }
    }

    /// The process exited or was killed: remove its interfaces.
    pub fn destroy_session(&mut self, stack: &mut Stack, pid: i32) {
        for s in 0..self.tuns.len() {
            if self.tuns[s].as_ref().is_some_and(|t| t.pid == pid) {
                self.close(stack, s);
            }
        }
    }

    /// Handle a socket syscall if it belongs to a tun socket. `None` = not
    /// ours (pass it to `net::ipc`). `SOCK_DESTROY_SESSION` is handled *and*
    /// passed on (returns `None`).
    pub fn ipc(&mut self, stack: &mut Stack, pid: i32, syscall: i32, args: &[u8], result: &mut [u8], now: i64) -> Option<i32> {
        match syscall {
            SOCK_DESTROY_SESSION => {
                self.destroy_session(stack, pid);
                return None;
            }
            SOCK_SOCKET => {
                if i32_at(args, 0)? != AF_ECM_TUN {
                    return None;
                }
                let free = self.tuns.iter().position(Option::is_none);
                let slot = match free {
                    Some(s) => s,
                    None if self.tuns.len() < MAX_TUNS => {
                        self.tuns.push(None);
                        self.tuns.len() - 1
                    }
                    None => return Some(reply(result, -1, &[])),
                };
                self.tuns[slot] = Some(Tun { pid, iface: None, out: VecDeque::new(), dropped: 0 });
                return Some(reply(result, TUN_ID_BASE + slot as i32, &[]));
            }
            SOCK_BIND | SOCK_CONNECT | SOCK_LISTEN | SOCK_ACCEPT | SOCK_SEND | SOCK_RECV | SOCK_CLOSE | SOCK_SETSOCKOPT
            | SOCK_SENDTO | SOCK_RECVFROM | SOCK_GETSOCKNAME | SOCK_GETPEERNAME | SOCK_SHUTDOWN => {}
            _ => return None,
        }
        let id = i32_at(args, 0)?;
        if id < TUN_ID_BASE {
            return None;
        }
        let Some(slot) = self.slot(id) else {
            return Some(reply(result, -1, &[]));
        };
        if self.tuns[slot].as_ref().map(|t| t.pid) != Some(pid) {
            return Some(reply(result, -1, &[]));
        }
        Some(match syscall {
            SOCK_SETSOCKOPT => {
                let (Some(level), Some(name)) = (i32_at(args, 4), i32_at(args, 8)) else {
                    return Some(reply(result, -1, &[]));
                };
                if level != SOL_TUN || name != TUN_SETIFF {
                    return Some(reply(result, -1, &[]));
                }
                let Some((ifname, mac, ip, prefix)) = parse_setiff(args.get(12..).unwrap_or(&[])) else {
                    return Some(reply(result, -1, &[]));
                };
                if self.tuns[slot].as_ref().is_some_and(|t| t.iface.is_some()) || prefix > 32 {
                    return Some(reply(result, -1, &[]));
                }
                let Some(i) = stack.add_interface(ifname, MacAddr(mac)) else {
                    return Some(reply(result, -1, &[]));
                };
                stack.set_link(i, true, now);
                if ip != [0; 4] {
                    stack.configure_addr(i, Ipv4Addr(ip), prefix, now);
                }
                self.tuns[slot].as_mut().unwrap().iface = Some(i);
                reply(result, 0, &[])
            }
            SOCK_SEND | SOCK_SENDTO => {
                let frame = if syscall == SOCK_SEND {
                    u16_at(args, 4).and_then(|n| args.get(6..6 + n))
                } else {
                    u16_at(args, 4).and_then(|al| u16_at(args, 6 + al).and_then(|n| args.get(8 + al..8 + al + n)))
                };
                let (Some(frame), Some(iface)) = (frame, self.tuns[slot].as_ref().and_then(|t| t.iface)) else {
                    return Some(reply(result, -1, &[]));
                };
                if frame.len() < 14 {
                    return Some(reply(result, -1, &[]));
                }
                stack.handle_frame(iface, frame, now);
                reply(result, frame.len() as i32, &[])
            }
            SOCK_RECV | SOCK_RECVFROM => {
                let max = i32_at(args, 4).unwrap_or(0).max(0) as usize;
                let t = self.tuns[slot].as_mut().unwrap();
                if t.iface.is_none() {
                    return Some(reply(result, -1, &[]));
                }
                match t.out.pop_front() {
                    None => reply(result, TUN_EMPTY, &[]),
                    Some(f) => {
                        let n = f.len().min(max);
                        if syscall == SOCK_RECV {
                            reply(result, n as i32, &f[..n])
                        } else {
                            let mut p = vec![0u8; 16];
                            p.extend_from_slice(&f[..n]);
                            reply(result, n as i32, &p)
                        }
                    }
                }
            }
            SOCK_CLOSE => {
                self.close(stack, slot);
                reply(result, 0, &[])
            }
            _ => reply(result, -1, &[]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{Net, Nics};
    use ecm_dsp::C32;
    use ecm_host_abi::tun::setiff_value;
    use ecm_net::types::SocketAddr;
    use ecm_net::TcpState;
    use ecm_radio::link::Tnc;

    struct NoNics;
    impl Nics for NoNics {
        fn count(&self) -> usize {
            0
        }
        fn mac(&self, _: usize) -> Option<[u8; 6]> {
            None
        }
        fn tx(&mut self, _: usize, _: &[u8]) {}
        fn rx(&mut self, _: &mut [u8]) -> Option<(usize, usize)> {
            None
        }
        fn set_promiscuous(&mut self, _: usize, _: bool) {}
        fn carrier(&self, _: usize) -> bool {
            false
        }
    }

    fn call(net: &mut Net, pid: i32, sys: i32, args: &[u8], now: i64) -> (i32, Vec<u8>) {
        let mut res = vec![0u8; 4096];
        let n = net.tun_ipc(pid, sys, args, &mut res, now).expect("tun call claimed");
        let status = i32::from_le_bytes([res[0], res[1], res[2], res[3]]);
        (status, res[4..n as usize].to_vec())
    }

    fn i32s(v: &[i32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    /// Open a tun socket through the syscall encoding, create `radio0`.
    fn open_tun(net: &mut Net, pid: i32, mac: [u8; 6], ip: [u8; 4], now: i64) -> i32 {
        let (id, _) = call(net, pid, SOCK_SOCKET, &i32s(&[AF_ECM_TUN, 3, 0]), now);
        assert!(id >= TUN_ID_BASE, "socket id {id}");
        let mut a = i32s(&[id, SOL_TUN, TUN_SETIFF]);
        a.extend(setiff_value("radio0", mac, ip, 24));
        assert_eq!(call(net, pid, SOCK_SETSOCKOPT, &a, now).0, 0);
        id
    }

    fn send_args(id: i32, frame: &[u8]) -> Vec<u8> {
        let mut a = id.to_le_bytes().to_vec();
        a.extend_from_slice(&(frame.len() as u16).to_le_bytes());
        a.extend_from_slice(frame);
        a
    }

    fn drain(net: &mut Net, pid: i32, id: i32, now: i64) -> Vec<Vec<u8>> {
        net.flush(now);
        let mut v = Vec::new();
        loop {
            let (st, f) = call(net, pid, SOCK_RECV, &i32s(&[id, 2048, 0]), now);
            if st == TUN_EMPTY {
                return v;
            }
            assert!(st > 0, "recv status {st}");
            v.push(f);
        }
    }

    #[test]
    fn other_sockets_pass_through() {
        let mut net = Net::with_nics(Box::new(NoNics), 1, 0);
        let mut res = vec![0u8; 64];
        assert!(net.tun_ipc(1, SOCK_SOCKET, &i32s(&[2, 1, 0]), &mut res, 0).is_none(), "AF_INET is not ours");
        assert!(net.tun_ipc(1, SOCK_RECV, &i32s(&[3, 100, 0]), &mut res, 0).is_none());
        assert!(net.tun_ipc(1, 11, &i32s(&[TUN_ID_BASE]), &mut res, 0).is_none(), "getaddrinfo never ours");
        let id = open_tun(&mut net, 7, [2, 0, 0, 0, 0, 1], [10, 44, 0, 1], 0);
        // another process can't use it
        assert_eq!(call(&mut net, 8, SOCK_RECV, &i32s(&[id, 100, 0]), 0).0, -1);
        // a second interface with the same name is refused
        let (id2, _) = call(&mut net, 7, SOCK_SOCKET, &i32s(&[AF_ECM_TUN, 3, 0]), 0);
        let mut a = i32s(&[id2, SOL_TUN, TUN_SETIFF]);
        a.extend(setiff_value("radio0", [2, 0, 0, 0, 0, 2], [0; 4], 24));
        assert_eq!(call(&mut net, 7, SOCK_SETSOCKOPT, &a, 0).0, -1);
        let i = net.stack.find_iface("radio0").expect("radio0 exists");
        assert_eq!(net.stack.iface(i).unwrap().ip, Ipv4Addr([10, 44, 0, 1]));
        // process exit removes it
        assert!(net.tun_ipc(7, SOCK_DESTROY_SESSION, &[], &mut res, 0).is_none());
        assert!(net.stack.find_iface("radio0").is_none());
    }

    /// Two kernels, each with `radio0` from a tun socket and a TNC (AX.25 +
    /// KISS + AFSK1200 over NBFM) in between, joined by an ideal channel of
    /// IQ samples: ping gets a reply, and a TCP connection (what ssh uses)
    /// carries data both ways with its MSS clamped to one AX.25 frame.
    #[test]
    fn ping_and_tcp_over_radio0_through_the_tnc() {
        let rate = 48_000.0;
        let mut a = Net::with_nics(Box::new(NoNics), 1, 0);
        let mut b = Net::with_nics(Box::new(NoNics), 2, 0);
        let mut ta = Tnc::new(ecm_dsp::coding::ax25::Address::parse("N0CALL-1").unwrap(), rate);
        let mut tb = Tnc::new(ecm_dsp::coding::ax25::Address::parse("N0CALL-2").unwrap(), rate);
        let ida = open_tun(&mut a, 1, ta.mac(), [10, 44, 0, 1], 0);
        let idb = open_tun(&mut b, 1, tb.mac(), [10, 44, 0, 2], 0);
        let mut now = 0i64;
        let air = std::cell::Cell::new(0usize);
        let max_frame = std::cell::Cell::new(0usize);
        // One TNC's queued bursts, through an ideal channel, into the other kernel.
        let deliver = |tx: &mut Tnc, rx: &mut Tnc, net: &mut Net, id: i32, now: i64| {
            while let Some(burst) = tx.take_burst() {
                air.set(air.get() + burst.len());
                let mut iq: Vec<C32> = burst;
                iq.extend(vec![C32::new(1.0, 0.0); 2400]);
                for c in iq.chunks(4800) {
                    for eth in rx.from_air(c) {
                        max_frame.set(max_frame.get().max(eth.len()));
                        assert!(call(net, 1, SOCK_SEND, &send_args(id, &eth), now).0 > 0);
                    }
                }
            }
        };
        // Move frames: kernel -> TNC -> IQ -> other TNC -> other kernel.
        let step = |a: &mut Net, b: &mut Net, ta: &mut Tnc, tb: &mut Tnc, now: i64| {
            for f in drain(a, 1, ida, now) {
                ta.from_kernel(&f);
            }
            for f in drain(b, 1, idb, now) {
                tb.from_kernel(&f);
            }
            deliver(ta, tb, b, idb, now);
            deliver(tb, ta, a, ida, now);
            a.poll(now);
            b.poll(now);
        };

        // ping 10.44.0.2 from A (ARP first, then echo)
        let h = a.stack.icmp_open().unwrap();
        let echo = [8u8, 0, 0xf7, 0xfd, 0, 1, 0, 1]; // id 1, seq 1, valid checksum
        a.stack.icmp_send(h, Ipv4Addr([10, 44, 0, 2]), &echo, now).unwrap();
        let mut got = None;
        for _ in 0..20 {
            now += 500;
            step(&mut a, &mut b, &mut ta, &mut tb, now);
            let mut buf = [0u8; 64];
            if let Some((from, n)) = a.stack.icmp_recv(h, &mut buf).unwrap() {
                got = Some((from, buf[0], n));
                break;
            }
        }
        assert_eq!(got.map(|g| (g.0, g.1)), Some((Ipv4Addr([10, 44, 0, 2]), 0)), "echo reply over radio0 (air {} samples, stats {:?} / {:?})", air.get(), ta.link().stats, tb.link().stats);
        assert!(air.get() > 0);

        // TCP: B listens on 22, A connects and both sides send.
        let l = b.stack.tcp_listen(SocketAddr { ip: Ipv4Addr::ZERO, port: 22 }, 4).unwrap();
        let c = a.stack.tcp_connect(SocketAddr { ip: Ipv4Addr([10, 44, 0, 2]), port: 22 }, now).unwrap();
        let mut server = None;
        let payload: Vec<u8> = (0..600u32).map(|i| (i % 251) as u8).collect();
        let (mut sent_a, mut got_b, mut got_a) = (false, Vec::new(), Vec::new());
        for _ in 0..200 {
            now += 250;
            step(&mut a, &mut b, &mut ta, &mut tb, now);
            if server.is_none() {
                server = b.stack.tcp_accept(l).unwrap();
            }
            if !sent_a && a.stack.tcp_state(c) == Ok(TcpState::Established) {
                a.stack.tcp_send(c, &payload, now).unwrap();
                sent_a = true;
            }
            if let Some(s) = server {
                let mut buf = [0u8; 2048];
                if let Ok(n) = b.stack.tcp_recv(s, &mut buf) {
                    if n > 0 {
                        got_b.extend_from_slice(&buf[..n]);
                        if got_b.len() == payload.len() {
                            b.stack.tcp_send(s, b"SSH-2.0-ecm\r\n", now).unwrap();
                        }
                    }
                }
            }
            let mut buf = [0u8; 64];
            if let Ok(n) = a.stack.tcp_recv(c, &mut buf) {
                got_a.extend_from_slice(&buf[..n]);
            }
            if got_a.len() >= 13 {
                break;
            }
        }
        assert_eq!(got_b, payload, "A -> B data");
        assert_eq!(got_a, b"SSH-2.0-ecm\r\n", "B -> A data");
        assert!(ta.link().stats.mss_clamped >= 1, "SYN MSS clamped: {:?}", ta.link().stats);
        assert!(max_frame.get() <= 14 + ecm_radio::link::MAX_INFO, "IP packets fit one AX.25 frame: {}", max_frame.get());
        // closing the socket removes the interface
        assert_eq!(call(&mut a, 1, SOCK_CLOSE, &i32s(&[ida]), now).0, 0);
        assert!(a.stack.find_iface("radio0").is_none());
    }
}
