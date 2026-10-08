//! `AF_PACKET`-style sockets: whole Ethernet frames, bound to one interface
//! and one EtherType (or [`ETH_P_ALL`]).
//!
//! Received frames are *copied* to every matching packet socket before the
//! stack processes them, so packet sockets never steal traffic from ARP/IP.
//! Frames sent on a packet socket go out of the bound interface exactly as
//! given (no ARP, no VLAN tag added), through the normal TX queue.
//!
//! Delivery rules (see [`Stack::handle_frame`]):
//! - frames to the interface MAC, broadcast, or any group (multicast) MAC;
//! - not frames the interface itself sent (reflected by a hub);
//! - only while the interface is administratively and physically up.
//! Outgoing frames are not looped back to packet sockets.

use std::collections::VecDeque;

use super::{next_gen, SocketHandle, Stack, TX_QUEUE_LEN};
use crate::types::{NetError, MAX_FRAME_SIZE};

/// Match every EtherType (Linux `ETH_P_ALL`).
pub const ETH_P_ALL: u16 = 0x0003;
/// Open packet sockets per stack.
pub const MAX_PACKET_SOCKETS: usize = 32;
/// Frames queued per packet socket; further frames are dropped (counted).
pub const PACKET_QUEUE_LEN: usize = 64;
/// Smallest frame accepted for sending: an untagged Ethernet header.
pub const MIN_FRAME_SIZE: usize = 14;

pub(super) struct PacketSock {
    /// Bound interface; `None` = unbound (receives nothing, cannot send).
    iface: Option<usize>,
    /// EtherType filter in host order; 0 = none, [`ETH_P_ALL`] = all.
    proto: u16,
    rx: VecDeque<(usize, Vec<u8>)>,
    dropped: u64,
    /// The bound interface was removed: every call errors.
    dead: bool,
}

pub(super) struct PacketSlot {
    gen: u16,
    sock: Option<PacketSock>,
}

/// What [`Stack::packet_recv`] returns for one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketInfo {
    /// Interface the frame arrived on.
    pub iface: usize,
    /// EtherType of the frame (after an 802.1Q tag, if any).
    pub ethertype: u16,
    /// Full frame length (may exceed what was copied).
    pub len: usize,
}

fn frame_ethertype(frame: &[u8]) -> Option<u16> {
    let t = u16::from_be_bytes([*frame.get(12)?, *frame.get(13)?]);
    if t == 0x8100 {
        Some(u16::from_be_bytes([*frame.get(16)?, *frame.get(17)?]))
    } else {
        Some(t)
    }
}

impl Stack {
    fn packet_sock(&self, h: SocketHandle) -> Result<&PacketSock, NetError> {
        match self.packet.get(h.idx as usize) {
            Some(PacketSlot {
                gen,
                sock: Some(s),
            }) if *gen == h.gen => Ok(s),
            _ => Err(NetError::BadHandle),
        }
    }

    fn packet_sock_mut(&mut self, h: SocketHandle) -> Result<&mut PacketSock, NetError> {
        match self.packet.get_mut(h.idx as usize) {
            Some(PacketSlot {
                gen,
                sock: Some(s),
            }) if *gen == h.gen => Ok(s),
            _ => Err(NetError::BadHandle),
        }
    }

    /// Open an unbound packet socket filtering on `ethertype` (host order;
    /// [`ETH_P_ALL`] for every frame, 0 for none until rebound).
    pub fn packet_open(&mut self, ethertype: u16) -> Result<SocketHandle, NetError> {
        let sock = PacketSock {
            iface: None,
            proto: ethertype,
            rx: VecDeque::new(),
            dropped: 0,
            dead: false,
        };
        if let Some(i) = self.packet.iter().position(|s| s.sock.is_none()) {
            self.packet[i].sock = Some(sock);
            return Ok(SocketHandle {
                idx: i as u16,
                gen: self.packet[i].gen,
            });
        }
        if self.packet.len() >= MAX_PACKET_SOCKETS {
            return Err(NetError::NoSockets);
        }
        self.packet.push(PacketSlot {
            gen: 1,
            sock: Some(sock),
        });
        Ok(SocketHandle {
            idx: (self.packet.len() - 1) as u16,
            gen: 1,
        })
    }

    /// Bind to interface `iface`; `ethertype` (if given) replaces the filter.
    /// Rebinding drops frames queued from the previous binding.
    pub fn packet_bind(
        &mut self,
        h: SocketHandle,
        iface: usize,
        ethertype: Option<u16>,
    ) -> Result<(), NetError> {
        if self.iface(iface).is_none() {
            return Err(NetError::InvalidInput);
        }
        let s = self.packet_sock_mut(h)?;
        if s.dead {
            return Err(NetError::ConnectionAborted);
        }
        if s.iface != Some(iface) {
            s.rx.clear();
        }
        s.iface = Some(iface);
        if let Some(t) = ethertype {
            s.proto = t;
        }
        Ok(())
    }

    /// (bound interface, EtherType filter).
    pub fn packet_binding(&self, h: SocketHandle) -> Result<(Option<usize>, u16), NetError> {
        let s = self.packet_sock(h)?;
        Ok((s.iface, s.proto))
    }

    /// Send one whole Ethernet frame (header included, no FCS) out of the
    /// bound interface, or out of `via` if given. Unbound sockets with no
    /// `via` get `NotConnected`.
    pub fn packet_send(
        &mut self,
        h: SocketHandle,
        via: Option<usize>,
        frame: &[u8],
    ) -> Result<usize, NetError> {
        let s = self.packet_sock(h)?;
        if s.dead {
            return Err(NetError::ConnectionAborted);
        }
        let iface = via.or(s.iface).ok_or(NetError::NotConnected)?;
        if frame.len() < MIN_FRAME_SIZE {
            return Err(NetError::InvalidInput);
        }
        if frame.len() > MAX_FRAME_SIZE {
            return Err(NetError::MessageTooLong);
        }
        let full = self.tx.len() >= TX_QUEUE_LEN;
        let ifc = self.iface_mut(iface).ok_or(NetError::InvalidInput)?;
        if !ifc.is_up() {
            ifc.stats.tx_dropped += 1;
            return Err(NetError::NoRoute);
        }
        if full {
            ifc.stats.tx_dropped += 1;
            return Err(NetError::WouldBlock);
        }
        ifc.stats.tx_packets += 1;
        ifc.stats.tx_bytes += frame.len() as u64;
        self.tx.push_back((iface, frame.to_vec()));
        Ok(frame.len())
    }

    /// Dequeue one frame into `buf` (truncated to fit). `Ok(None)` = nothing
    /// queued. Unbound sockets get `NotConnected`.
    pub fn packet_recv(
        &mut self,
        h: SocketHandle,
        buf: &mut [u8],
    ) -> Result<Option<PacketInfo>, NetError> {
        let s = self.packet_sock_mut(h)?;
        if s.dead {
            return Err(NetError::ConnectionAborted);
        }
        if s.iface.is_none() {
            return Err(NetError::NotConnected);
        }
        let Some((iface, frame)) = s.rx.pop_front() else {
            return Ok(None);
        };
        let n = frame.len().min(buf.len());
        buf[..n].copy_from_slice(&frame[..n]);
        Ok(Some(PacketInfo {
            iface,
            ethertype: frame_ethertype(&frame).unwrap_or(0),
            len: frame.len(),
        }))
    }

    /// A frame is queued (or the socket is in an error state, so a receive
    /// would return at once).
    pub fn packet_can_read(&self, h: SocketHandle) -> bool {
        match self.packet_sock(h) {
            Ok(s) => s.dead || s.iface.is_none() || !s.rx.is_empty(),
            Err(_) => true,
        }
    }

    /// Frames dropped because the socket's queue was full.
    pub fn packet_dropped(&self, h: SocketHandle) -> Result<u64, NetError> {
        Ok(self.packet_sock(h)?.dropped)
    }

    pub fn packet_close(&mut self, h: SocketHandle) {
        if let Some(slot) = self.packet.get_mut(h.idx as usize) {
            if slot.gen == h.gen && slot.sock.is_some() {
                slot.sock = None;
                slot.gen = next_gen(slot.gen);
            }
        }
    }

    /// Copy a received frame to every packet socket bound to `iface` whose
    /// filter matches.
    pub(super) fn packet_deliver(&mut self, iface: usize, frame: &[u8]) {
        if self.packet.is_empty() {
            return;
        }
        let Some(ethertype) = frame_ethertype(frame) else {
            return;
        };
        for slot in &mut self.packet {
            let Some(s) = slot.sock.as_mut() else {
                continue;
            };
            if s.dead || s.iface != Some(iface) {
                continue;
            }
            if !(s.proto == ETH_P_ALL || (s.proto != 0 && s.proto == ethertype)) {
                continue;
            }
            if s.rx.len() >= PACKET_QUEUE_LEN {
                s.dropped += 1;
            } else {
                s.rx.push_back((iface, frame.to_vec()));
            }
        }
    }

    /// The interface went away: its packet sockets fail from now on.
    pub(super) fn packet_iface_removed(&mut self, iface: usize) {
        for s in self.packet.iter_mut().filter_map(|s| s.sock.as_mut()) {
            if s.iface == Some(iface) {
                s.dead = true;
                s.rx.clear();
            }
        }
    }
}
