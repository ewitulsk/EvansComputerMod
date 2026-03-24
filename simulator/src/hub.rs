//! In-memory Ethernet hub connecting multiple WASM instances.
//!
//! The hub acts as a Layer 2 switch: it delivers ethernet frames between
//! registered NICs based on destination MAC address. Broadcast frames
//! (dst = ff:ff:ff:ff:ff:ff) are delivered to all NICs except the sender.
//!
//! Optionally bridges to a TAP device for real internet access.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::interrupts::InterruptQueue;

/// Maximum frames queued per NIC before dropping.
const MAX_QUEUE_SIZE: usize = 64;

/// IRQ number for network frame arrival.
pub const IRQ_NETWORK: i32 = 3;

/// Broadcast MAC address.
const BROADCAST_MAC: [u8; 6] = [0xff; 6];

/// Per-NIC receive state.
struct NicMailbox {
    rx_queue: VecDeque<Vec<u8>>,
    promiscuous: bool,
    interrupt_queue: InterruptQueue,
}

/// Shared state behind the condvar.
struct HubInner {
    nics: HashMap<[u8; 6], NicMailbox>,
    /// Frames queued for transmission via the TAP device.
    /// A writer thread drains this queue.
    tap_tx_queue: VecDeque<Vec<u8>>,
    /// Whether a TAP device is attached.
    tap_attached: bool,
}

/// Thread-safe ethernet hub.
pub struct EthernetHub {
    inner: Mutex<HubInner>,
    /// Condvar notified whenever a frame is enqueued on any NIC.
    notify: Condvar,
    /// Condvar notified when a frame is queued for TAP transmission.
    tap_notify: Condvar,
}

impl EthernetHub {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(HubInner {
                nics: HashMap::new(),
                tap_tx_queue: VecDeque::new(),
                tap_attached: false,
            }),
            notify: Condvar::new(),
            tap_notify: Condvar::new(),
        })
    }

    /// Register a NIC with the given MAC address.
    pub fn register_nic(&self, mac: [u8; 6], interrupt_queue: InterruptQueue) {
        let mut inner = self.inner.lock().unwrap();
        inner.nics.insert(mac, NicMailbox {
            rx_queue: VecDeque::new(),
            promiscuous: false,
            interrupt_queue,
        });
    }

    /// Unregister a NIC.
    pub fn unregister_nic(&self, mac: &[u8; 6]) {
        let mut inner = self.inner.lock().unwrap();
        inner.nics.remove(mac);
    }

    /// Mark that a TAP device is attached.
    pub fn set_tap_attached(&self, attached: bool) {
        let mut inner = self.inner.lock().unwrap();
        inner.tap_attached = attached;
    }

    /// Transmit an ethernet frame from a WASM instance.
    ///
    /// Routing:
    /// - Broadcast dst → deliver to all NICs except sender + TAP
    /// - Known unicast dst → deliver to that NIC only
    /// - Unknown unicast dst → forward to TAP (for the real network)
    /// - Promiscuous NICs receive all frames except their own
    pub fn transmit(&self, src_mac: &[u8; 6], frame: &[u8]) {
        if frame.len() < 14 {
            return;
        }

        let mut dst_mac = [0u8; 6];
        dst_mac.copy_from_slice(&frame[0..6]);

        let is_broadcast = dst_mac == BROADCAST_MAC;

        let mut inner = self.inner.lock().unwrap();

        let mut delivered_to_nic = false;

        if is_broadcast {
            // Deliver to all NICs except sender
            for (mac, mailbox) in inner.nics.iter_mut() {
                if mac != src_mac {
                    Self::enqueue(mailbox, frame);
                    delivered_to_nic = true;
                }
            }
            // Also forward broadcast to TAP
            if inner.tap_attached {
                inner.tap_tx_queue.push_back(frame.to_vec());
            }
        } else {
            // Unicast: try to deliver to destination NIC
            if let Some(mailbox) = inner.nics.get_mut(&dst_mac) {
                Self::enqueue(mailbox, frame);
                delivered_to_nic = true;
            }

            // Also deliver to promiscuous NICs (except sender and destination)
            for (mac, mailbox) in inner.nics.iter_mut() {
                if mac != src_mac && *mac != dst_mac && mailbox.promiscuous {
                    Self::enqueue(mailbox, frame);
                }
            }

            // If no NIC matched, forward to TAP (it's for the real network)
            if !delivered_to_nic && inner.tap_attached {
                inner.tap_tx_queue.push_back(frame.to_vec());
            }
        }

        // Wake any blocking receivers and TAP writer
        self.notify.notify_all();
        if inner.tap_attached {
            self.tap_notify.notify_all();
        }
    }

    /// Inject a frame received from the TAP device into the hub.
    /// Routes it to matching NICs (by destination MAC) or broadcasts.
    pub fn inject_from_tap(&self, frame: &[u8]) {
        if frame.len() < 14 {
            return;
        }

        let mut dst_mac = [0u8; 6];
        dst_mac.copy_from_slice(&frame[0..6]);

        let is_broadcast = dst_mac == BROADCAST_MAC;

        let mut inner = self.inner.lock().unwrap();

        if is_broadcast {
            // Deliver to all NICs
            for (_mac, mailbox) in inner.nics.iter_mut() {
                Self::enqueue(mailbox, frame);
            }
        } else {
            // Deliver to matching NIC
            if let Some(mailbox) = inner.nics.get_mut(&dst_mac) {
                Self::enqueue(mailbox, frame);
            }
            // Also to promiscuous NICs
            for (mac, mailbox) in inner.nics.iter_mut() {
                if *mac != dst_mac && mailbox.promiscuous {
                    Self::enqueue(mailbox, frame);
                }
            }
        }

        self.notify.notify_all();
    }

    /// Pop a frame from the TAP tx queue (called by the TAP writer thread).
    /// Blocks up to `timeout` if the queue is empty.
    pub fn pop_tap_frame(&self, timeout: Duration) -> Option<Vec<u8>> {
        let mut inner = self.inner.lock().unwrap();

        if let Some(frame) = inner.tap_tx_queue.pop_front() {
            return Some(frame);
        }

        let (mut inner, _) = self.tap_notify.wait_timeout(inner, timeout).unwrap();
        inner.tap_tx_queue.pop_front()
    }

    /// Non-blocking receive: pop the next frame for the given MAC.
    pub fn receive(&self, mac: &[u8; 6]) -> Option<Vec<u8>> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(mailbox) = inner.nics.get_mut(mac) {
            mailbox.rx_queue.pop_front()
        } else {
            None
        }
    }

    /// Blocking receive: wait up to `timeout` for a frame.
    pub fn receive_blocking(&self, mac: &[u8; 6], timeout: Duration) -> Option<Vec<u8>> {
        let mut inner = self.inner.lock().unwrap();

        // Check if there's already a frame
        if let Some(mailbox) = inner.nics.get_mut(mac) {
            if let Some(frame) = mailbox.rx_queue.pop_front() {
                return Some(frame);
            }
        }

        // Wait for notification
        let (mut inner, _) = self.notify.wait_timeout(inner, timeout).unwrap();

        // Check again
        if let Some(mailbox) = inner.nics.get_mut(mac) {
            mailbox.rx_queue.pop_front()
        } else {
            None
        }
    }

    /// Set promiscuous mode for a NIC.
    pub fn set_promiscuous(&self, mac: &[u8; 6], enabled: bool) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(mailbox) = inner.nics.get_mut(mac) {
            mailbox.promiscuous = enabled;
        }
    }

    /// Enqueue a frame on a NIC's receive queue, dropping oldest if full.
    fn enqueue(mailbox: &mut NicMailbox, frame: &[u8]) {
        if mailbox.rx_queue.len() >= MAX_QUEUE_SIZE {
            mailbox.rx_queue.pop_front(); // drop oldest
        }
        mailbox.rx_queue.push_back(frame.to_vec());

        // Fire IRQ_NETWORK interrupt
        mailbox.interrupt_queue.push(
            IRQ_NETWORK,
            format!("{{\"frame_len\":{}}}", frame.len()),
        );
    }
}
