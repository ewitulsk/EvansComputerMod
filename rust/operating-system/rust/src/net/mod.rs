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

use std::collections::BTreeMap;

use ecm_bridge::{Bridge, Output};
use ecm_net::types::{Ipv4Addr, MacAddr, MAX_FRAME_SIZE};
use ecm_net::{Stack, StackConfig};

use crate::hal;

/// Frames drained from the host per `rx` call before yielding.
const RX_BUDGET: usize = 512;
/// How often carrier state is re-sampled from the host.
const CARRIER_POLL_MS: i64 = 250;

pub struct Net {
    pub stack: Stack,
    bridge: Option<Bridge>,
    phys: usize,
    carrier: Vec<bool>,
    next_carrier_poll: i64,
    /// vlan id → stack interface index of that VLAN's SVI.
    svis: BTreeMap<u16, usize>,
}

impl Net {
    pub fn new(now: i64) -> Self {
        let mut stack = Stack::new(StackConfig { seed: hal::random_u64() });
        let phys = hal::net::interface_count().min(12);
        let mut carrier = Vec::with_capacity(phys);
        for i in 0..phys {
            let mac = hal::net::interface_mac(i).unwrap_or([0x02, 0, 0, 0, 0, i as u8]);
            stack.add_interface(&format!("eth{}", i), MacAddr(mac));
            let up = hal::net::carrier(i);
            stack.set_link(i, up, now);
            carrier.push(up);
        }
        Self { stack, bridge: None, phys, carrier, next_carrier_poll: now + CARRIER_POLL_MS, svis: BTreeMap::new() }
    }

    pub fn port_count(&self) -> usize {
        self.phys
    }

    pub fn port_macs(&self) -> Vec<MacAddr> {
        (0..self.phys).map(|i| self.stack.iface(i).map(|f| f.mac).unwrap_or(MacAddr::ZERO)).collect()
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

    pub fn attach_bridge(&mut self, bridge: Bridge) {
        self.bridge = Some(bridge);
        self.sync_bridge_ports();
    }

    pub fn detach_bridge(&mut self) -> Option<Bridge> {
        let b = self.bridge.take();
        for vlan in self.svis.keys().copied().collect::<Vec<_>>() {
            self.remove_svi(vlan);
        }
        for i in 0..self.phys {
            hal::net::set_promiscuous(i, false);
        }
        b
    }

    /// Promiscuous mode follows L2 membership: bridge ports must see every
    /// frame on their segment, routed ports only their own.
    pub fn sync_bridge_ports(&mut self) {
        for i in 0..self.phys {
            let l2 = self.bridge.as_ref().is_some_and(|b| b.is_l2_port(i));
            hal::net::set_promiscuous(i, l2);
        }
    }

    pub fn set_svi(&mut self, vlan: u16, ip: Ipv4Addr, prefix: u8, now: i64) -> bool {
        let idx = match self.svis.get(&vlan) {
            Some(&i) => i,
            None => {
                let Some(bridge) = self.bridge.as_ref() else { return false };
                let Some(i) = self.stack.add_interface(&format!("vlan{}", vlan), bridge.bridge_mac()) else { return false };
                self.stack.set_link(i, true, now);
                self.svis.insert(vlan, i);
                i
            }
        };
        self.stack.configure_addr(idx, ip, prefix, now);
        true
    }

    pub fn remove_svi(&mut self, vlan: u16) {
        if let Some(i) = self.svis.remove(&vlan) {
            self.stack.remove_interface(i);
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

    // ------------------------------------------------------------ I/O

    /// Drain received frames from the host (IRQ_NETWORK).
    pub fn rx(&mut self, now: i64) {
        let mut buf = [0u8; MAX_FRAME_SIZE];
        for _ in 0..RX_BUDGET {
            let Some((port, len)) = hal::net::rx_any(&mut buf) else { break };
            if port >= self.phys {
                continue;
            }
            let frame = &buf[..len];
            match self.bridge.as_mut() {
                Some(b) if b.is_l2_port(port) => b.handle_frame(port, frame, now),
                _ => self.stack.handle_frame(port, frame, now),
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
                if let Some(vlan) = self.svi_vlan(iface) {
                    if let Some(b) = self.bridge.as_mut() {
                        b.send_local(vlan, &frame, now);
                    }
                } else if iface < self.phys {
                    let bridged = self.bridge.as_ref().is_some_and(|b| b.is_l2_port(iface));
                    if !bridged {
                        hal::net::tx(iface, &frame);
                    }
                }
            }
            if let Some(b) = self.bridge.as_mut() {
                while let Some(out) = b.pop_output() {
                    progressed = true;
                    match out {
                        Output::Tx { port, frame } => {
                            if port < self.phys {
                                hal::net::tx(port, &frame);
                            }
                        }
                        Output::Local { vlan, frame } => {
                            if let Some(&i) = self.svis.get(&vlan) {
                                self.stack.handle_frame(i, &frame, now);
                            }
                        }
                        // The bridge keeps its own log ring (`show logging`).
                        Output::Log { .. } => {}
                    }
                }
            }
            if !progressed {
                break;
            }
        }
    }

    /// Timers, carrier sampling and output. Returns the next deadline.
    pub fn poll(&mut self, now: i64) -> Option<i64> {
        if now >= self.next_carrier_poll {
            self.next_carrier_poll = now + CARRIER_POLL_MS;
            for i in 0..self.phys {
                let up = hal::net::carrier(i);
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
