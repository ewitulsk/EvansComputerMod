//! Networking: the single owner of the NICs.
//!
//! `Net` is the ONLY code that receives frames from the host. Each frame
//! goes to exactly one consumer:
//! - an L2 bridge port (switch running, port in access/trunk mode) → `Bridge`
//! - anything else → the host `Stack`
//!
//! The bridge hands frames addressed to the switch itself back as
//! `Output::Local{vlan}`; those enter the stack on that VLAN's SVI interface
//! (`vlanN`). Stack output on an SVI goes back into the bridge via
//! `send_local`, so management traffic is tagged and forwarded like any
//! other frame. Nothing else calls `rx`, so no blocking loop can steal
//! frames from the switch.

pub mod config;
pub mod ipc;
pub mod netlink;
pub mod radio0;

use std::collections::BTreeMap;

use ecm_bridge::{Bridge, Output};
use ecm_net::types::{Ipv4Addr, MacAddr, MAX_FRAME_SIZE};
use ecm_net::{Stack, StackConfig};

use crate::hal;

/// The physical NICs. In the kernel these are the host's faces (`HostNics`);
/// host-side tests plug in in-memory NICs to drive the real dispatcher.
pub trait Nics {
    fn count(&self) -> usize;
    fn mac(&self, port: usize) -> Option<[u8; 6]>;
    fn tx(&mut self, port: usize, frame: &[u8]);
    /// Non-blocking receive from any port: (port, len).
    fn rx(&mut self, buf: &mut [u8]) -> Option<(usize, usize)>;
    fn set_promiscuous(&mut self, port: usize, on: bool);
    fn carrier(&self, port: usize) -> bool;
}

/// NICs provided by the host through `hal`.
pub struct HostNics;

impl Nics for HostNics {
    fn count(&self) -> usize {
        hal::net::interface_count()
    }
    fn mac(&self, port: usize) -> Option<[u8; 6]> {
        hal::net::interface_mac(port)
    }
    fn tx(&mut self, port: usize, frame: &[u8]) {
        hal::net::tx(port, frame);
    }
    fn rx(&mut self, buf: &mut [u8]) -> Option<(usize, usize)> {
        hal::net::rx_any(buf)
    }
    fn set_promiscuous(&mut self, port: usize, on: bool) {
        hal::net::set_promiscuous(port, on);
    }
    fn carrier(&self, port: usize) -> bool {
        hal::net::carrier(port)
    }
}

/// Frames drained from the host per `rx` call before yielding.
const RX_BUDGET: usize = 512;
/// How often carrier state is re-sampled from the host.
const CARRIER_POLL_MS: i64 = 250;

pub struct Net {
    nics: Box<dyn Nics>,
    pub stack: Stack,
    bridge: Option<Bridge>,
    phys: usize,
    carrier: Vec<bool>,
    next_carrier_poll: i64,
    /// vlan id → stack interface index of that VLAN's SVI.
    svis: BTreeMap<u16, usize>,
    /// Switch log events waiting to be shown (`logging console`).
    console_log: Vec<String>,
    pub router: Option<ecm_router::Router>,
    pub bgp: Option<crate::bgp_svc::BgpService>,
    /// Tun-style interfaces carried by programs (`radio0`, see radio0.rs).
    pub tuns: radio0::Tuns,
}

impl Net {
    pub fn new(now: i64) -> Self {
        Self::with_nics(Box::new(HostNics), hal::random_u64(), now)
    }

    pub fn with_nics(nics: Box<dyn Nics>, seed: u64, now: i64) -> Self {
        let mut stack = Stack::new(StackConfig { seed });
        let phys = nics.count().min(12);
        let mut carrier = Vec::with_capacity(phys);
        for i in 0..phys {
            let mac = nics.mac(i).unwrap_or([0x02, 0, 0, 0, 0, i as u8]);
            stack.add_interface(&format!("eth{}", i), MacAddr(mac));
            let up = nics.carrier(i);
            stack.set_link(i, up, now);
            carrier.push(up);
        }
        Self {
            nics,
            stack,
            bridge: None,
            phys,
            carrier,
            next_carrier_poll: now + CARRIER_POLL_MS,
            svis: BTreeMap::new(),
            console_log: Vec::new(),
            router: None,
            bgp: None,
            tuns: radio0::Tuns::new(),
        }
    }

    pub fn port_count(&self) -> usize {
        self.phys
    }

    pub fn port_macs(&self) -> Vec<MacAddr> {
        (0..self.phys)
            .map(|i| self.stack.iface(i).map(|f| f.mac).unwrap_or(MacAddr::ZERO))
            .collect()
    }

    pub fn carrier(&self, port: usize) -> bool {
        self.carrier.get(port).copied().unwrap_or(false)
    }

    // ------------------------------------------------------------ bridge

    pub fn bridge(&self) -> Option<&Bridge> {
        self.bridge.as_ref()
    }

    pub fn bridge_mut(&mut self) -> Option<&mut Bridge> {
        self.bridge.as_mut()
    }

    /// Start switching. The bridge starts with every link down, so it is
    /// told the current carrier of each port (later changes are pushed by
    /// `poll`).
    pub fn attach_bridge(&mut self, mut bridge: Bridge, now: i64) {
        for (i, &up) in self.carrier.iter().enumerate() {
            bridge.set_link(i, up, now);
        }
        self.bridge = Some(bridge);
        self.sync_bridge_ports();
    }

    pub fn detach_bridge(&mut self) -> Option<Bridge> {
        let b = self.bridge.take();
        for vlan in self.svis.keys().copied().collect::<Vec<_>>() {
            self.remove_svi(vlan);
        }
        for i in 0..self.phys {
            self.nics.set_promiscuous(i, false);
        }
        b
    }

    /// Promiscuous mode follows L2 membership: bridge ports must see every
    /// frame on their segment, routed ports only their own.
    pub fn sync_bridge_ports(&mut self) {
        for i in 0..self.phys {
            let l2 = self.bridge.as_ref().is_some_and(|b| b.is_l2_port(i));
            self.nics.set_promiscuous(i, l2);
        }
    }

    pub fn set_svi(&mut self, vlan: u16, ip: Ipv4Addr, prefix: u8, now: i64) -> bool {
        let idx = match self.svis.get(&vlan) {
            Some(&i) => i,
            None => {
                let Some(bridge) = self.bridge.as_ref() else {
                    return false;
                };
                let Some(i) = self
                    .stack
                    .add_interface(&format!("vlan{}", vlan), bridge.bridge_mac())
                else {
                    return false;
                };
                self.stack.set_link(i, true, now);
                self.svis.insert(vlan, i);
                i
            }
        };
        self.stack.configure_addr(idx, ip, prefix, now);
        // The bridge only hands up frames for VLANs it knows have an SVI.
        if let Some(b) = self.bridge.as_mut() {
            let _ = b.set_svi(vlan, Some((ip, prefix)));
        }
        true
    }

    pub fn remove_svi(&mut self, vlan: u16) {
        if let Some(i) = self.svis.remove(&vlan) {
            self.stack.remove_interface(i);
        }
        if let Some(b) = self.bridge.as_mut() {
            let _ = b.set_svi(vlan, None);
        }
    }

    pub fn svis(&self) -> Vec<(u16, Ipv4Addr, u8)> {
        self.svis
            .iter()
            .filter_map(|(&v, &i)| self.stack.iface(i).map(|f| (v, f.ip, f.prefix)))
            .collect()
    }

    fn svi_vlan(&self, iface: usize) -> Option<u16> {
        self.svis.iter().find(|(_, &i)| i == iface).map(|(&v, _)| v)
    }

    /// Socket syscalls on tun sockets (`radio0`); `None` = not a tun call.
    pub fn tun_ipc(&mut self, pid: i32, syscall: i32, args: &[u8], result: &mut [u8], now: i64) -> Option<i32> {
        self.tuns.ipc(&mut self.stack, pid, syscall, args, result, now)
    }

    // ------------------------------------------------------------ I/O

    /// Drain received frames from the host (IRQ_NETWORK).
    pub fn rx(&mut self, now: i64) {
        let mut buf = [0u8; MAX_FRAME_SIZE];
        for _ in 0..RX_BUDGET {
            let Some((port, len)) = self.nics.rx(&mut buf) else {
                break;
            };
            if port >= self.phys {
                continue;
            }
            let frame = &buf[..len];
            match self.bridge.as_mut() {
                Some(b) if b.is_l2_port(port) => b.handle_frame(port, frame, now),
                _ => {
                    if let Some(r) = self.router.as_mut() {
                        if let Some(frame) = r.ingress(&mut self.stack, port, frame, now) {
                            self.stack.handle_frame(port, &frame, now);
                        }
                    } else {
                        self.stack.handle_frame(port, frame, now);
                    }
                }
            }
        }
        self.flush(now);
    }

    /// Move queued output between stack, bridge and host until quiescent.
    pub fn flush(&mut self, now: i64) {
        for _ in 0..64 {
            let mut progressed = false;
            while let Some((iface, frame)) = self.stack.pop_tx() {
                progressed = true;
                let frame = if let Some(r) = self.router.as_mut() {
                    let Some(f) = r.egress(&self.stack, iface, &frame, now) else {
                        continue;
                    };
                    f
                } else {
                    frame
                };
                if self.tuns.on_tx(iface, &frame) {
                    continue;
                }
                if let Some(vlan) = self.svi_vlan(iface) {
                    if let Some(b) = self.bridge.as_mut() {
                        b.send_local(vlan, &frame, now);
                    }
                } else if iface < self.phys {
                    let bridged = self.bridge.as_ref().is_some_and(|b| b.is_l2_port(iface));
                    if !bridged {
                        self.nics.tx(iface, &frame);
                    }
                }
            }
            if let Some(b) = self.bridge.as_mut() {
                while let Some(out) = b.pop_output() {
                    progressed = true;
                    match out {
                        Output::Tx { port, frame } => {
                            if port < self.phys {
                                self.nics.tx(port, &frame);
                            }
                        }
                        Output::Local { vlan, frame } => {
                            if let Some(&i) = self.svis.get(&vlan) {
                                self.stack.handle_frame(i, &frame, now);
                            }
                        }
                        // The bridge keeps its own ring for `show logging`;
                        // echo to the console only if `logging console`.
                        Output::Log { severity, msg } => {
                            if b.log_console() && self.console_log.len() < 64 {
                                self.console_log.push(format!("{:?}: {}", severity, msg));
                            }
                        }
                    }
                }
            }
            if !progressed {
                break;
            }
        }
    }

    /// Switch log lines queued for the console since the last call.
    pub fn take_console_log(&mut self) -> Vec<String> {
        core::mem::take(&mut self.console_log)
    }

    /// Timers, carrier sampling and output. Returns the next deadline.
    pub fn poll(&mut self, now: i64) -> Option<i64> {
        if now >= self.next_carrier_poll {
            self.next_carrier_poll = now + CARRIER_POLL_MS;
            for i in 0..self.phys {
                let up = self.nics.carrier(i);
                if up != self.carrier[i] {
                    self.carrier[i] = up;
                    self.stack.set_link(i, up, now);
                    if let Some(b) = self.bridge.as_mut() {
                        b.set_link(i, up, now);
                    }
                }
            }
        }
        let mut deadline = self.stack.poll(now);
        if let Some(b) = self.bgp.as_mut() {
            deadline = min_opt(deadline, b.poll(&mut self.stack, now));
        }
        if let Some(r) = self.router.as_mut() {
            r.nat.expire(now);
            if r.dhcp.dirty && crate::fs::write("router.leases", r.dhcp.render().as_bytes()) {
                r.dhcp.dirty = false;
            }
        }
        if let Some(b) = self.bridge.as_mut() {
            deadline = min_opt(deadline, b.poll(now));
        }
        self.flush(now);
        min_opt(deadline, Some(self.next_carrier_poll))
    }
}

pub fn min_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

#[cfg(test)]
#[path = "net_tests.rs"]
mod tests;
