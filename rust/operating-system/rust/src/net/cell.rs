//! `wwan0`: the cellular modem (an LTE or mmWave module in a bay).
//!
//! The modem does the radio work on the host (search, attach, handover,
//! scheduling); to the kernel it is a plain Ethernet NIC that appears while a
//! modem module is installed and has carrier while it is attached to a tower.
//! The tower bridges its frames onto its backhaul cable, so DHCP, ARP, IP and
//! TCP run over `wwan0` exactly as over `eth0` (docs/gui/CELLULAR.md).
//!
//! Host ABI (`cell_*`, abi/host-abi.toml): presence and carrier are sampled
//! every [`POLL_MS`]; received frames raise IRQ 3 (network) like the NICs.

use ecm_net::types::{MacAddr, MAX_FRAME_SIZE};

use crate::hal;

/// The interface name userspace uses.
pub const IFNAME: &str = "wwan0";
/// How often presence, MAC and carrier are re-sampled (module hot-plug, attach/detach).
pub const POLL_MS: i64 = 250;
/// Frames drained from the modem per `rx` call.
pub const RX_BUDGET: usize = 256;

/// The modem as the kernel drives it. Host-side tests plug in a fake.
pub trait CellModem {
    /// Number of modems (0 = none; modem 0 is `wwan0`).
    fn present(&self) -> usize;
    fn mac(&self) -> Option<[u8; 6]>;
    /// Attached to a tower (carrier).
    fn link(&self) -> bool;
    fn tx(&mut self, frame: &[u8]) -> bool;
    /// Non-blocking receive of one Ethernet frame: its length.
    fn rx(&mut self, buf: &mut [u8]) -> Option<usize>;
}

/// The host's modem module (`cell_*` imports).
pub struct HostCell;

impl CellModem for HostCell {
    fn present(&self) -> usize {
        hal::cell::present()
    }
    fn mac(&self) -> Option<[u8; 6]> {
        hal::cell::mac()
    }
    fn link(&self) -> bool {
        hal::cell::link()
    }
    fn tx(&mut self, frame: &[u8]) -> bool {
        hal::cell::tx(frame)
    }
    fn rx(&mut self, buf: &mut [u8]) -> Option<usize> {
        hal::cell::rx(buf)
    }
}

/// No modem (host-side tests, hosts without the ABI).
pub struct NoCell;

impl CellModem for NoCell {
    fn present(&self) -> usize {
        0
    }
    fn mac(&self) -> Option<[u8; 6]> {
        None
    }
    fn link(&self) -> bool {
        false
    }
    fn tx(&mut self, _: &[u8]) -> bool {
        false
    }
    fn rx(&mut self, _: &mut [u8]) -> Option<usize> {
        None
    }
}

/// What changed at a [`CellDev::sample`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Sample {
    /// The modem appeared, or a different modem (new MAC) replaced it: (re)create `wwan0`.
    pub attach_iface: Option<MacAddr>,
    /// New carrier state.
    pub carrier: Option<bool>,
}

/// Kernel-side state of the modem.
pub struct CellDev {
    modem: Box<dyn CellModem>,
    /// Stack index of `wwan0` once it exists.
    pub iface: Option<usize>,
    mac: Option<[u8; 6]>,
    present: bool,
    carrier: bool,
    next_poll: i64,
    pub tx_frames: u64,
    pub tx_dropped: u64,
    pub rx_frames: u64,
}

impl CellDev {
    pub fn new(modem: Box<dyn CellModem>, now: i64) -> Self {
        Self { modem, iface: None, mac: None, present: false, carrier: false, next_poll: now, tx_frames: 0, tx_dropped: 0, rx_frames: 0 }
    }

    pub fn present(&self) -> bool {
        self.present
    }

    pub fn carrier(&self) -> bool {
        self.carrier
    }

    pub fn poll_due(&self, now: i64) -> bool {
        now >= self.next_poll
    }

    pub fn next_poll(&self) -> i64 {
        self.next_poll
    }

    /// Re-read presence, MAC and carrier.
    pub fn sample(&mut self, now: i64) -> Sample {
        self.next_poll = now + POLL_MS;
        let mut out = Sample::default();
        let present = self.modem.present() > 0;
        self.present = present;
        if present {
            let mac = self.modem.mac();
            if mac.is_some() && (self.iface.is_none() || mac != self.mac) {
                self.mac = mac;
                out.attach_iface = mac.map(MacAddr);
            }
        }
        let carrier = present && self.modem.link();
        if carrier != self.carrier || out.attach_iface.is_some() {
            self.carrier = carrier;
            out.carrier = Some(carrier);
        }
        out
    }

    /// Send a stack frame out of `wwan0` (dropped while not attached).
    pub fn send(&mut self, frame: &[u8]) {
        if self.carrier && self.modem.tx(frame) {
            self.tx_frames += 1;
        } else {
            self.tx_dropped += 1;
        }
    }

    /// Every waiting frame from the modem.
    pub fn drain(&mut self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        if !self.present {
            return out;
        }
        let mut buf = [0u8; MAX_FRAME_SIZE];
        for _ in 0..RX_BUDGET {
            match self.modem.rx(&mut buf) {
                Some(n) if n >= 14 => {
                    self.rx_frames += 1;
                    out.push(buf[..n.min(buf.len())].to_vec());
                }
                Some(_) => {}
                None => break,
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "cell_tests.rs"]
mod tests;
