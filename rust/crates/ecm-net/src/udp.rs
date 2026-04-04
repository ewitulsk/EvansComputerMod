//! UDP socket table and packet handling.

use super::types::{Ipv4Addr, SocketAddr, NetError};
use super::checksum::pseudo_header_checksum;
use super::ipv4::PROTO_UDP;

const MAX_UDP_SOCKETS: usize = 8;
const UDP_RX_BUF_SIZE: usize = 2048;
const MAX_RX_PACKETS: usize = 8;

/// UDP header (8 bytes).
pub struct UdpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub length: u16,
    pub checksum: u16,
}

impl UdpHeader {
    pub const SIZE: usize = 8;

    pub fn parse(data: &[u8]) -> Option<(Self, &[u8])> {
        if data.len() < 8 {
            return None;
        }
        let length = u16::from_be_bytes([data[4], data[5]]) as usize;
        if data.len() < length {
            return None;
        }
        Some((
            UdpHeader {
                src_port: u16::from_be_bytes([data[0], data[1]]),
                dst_port: u16::from_be_bytes([data[2], data[3]]),
                length: length as u16,
                checksum: u16::from_be_bytes([data[6], data[7]]),
            },
            &data[8..length],
        ))
    }

    pub fn serialize(
        buf: &mut [u8],
        src_port: u16,
        dst_port: u16,
        payload: &[u8],
        src_ip: &Ipv4Addr,
        dst_ip: &Ipv4Addr,
    ) -> usize {
        let length = (8 + payload.len()) as u16;
        buf[0..2].copy_from_slice(&src_port.to_be_bytes());
        buf[2..4].copy_from_slice(&dst_port.to_be_bytes());
        buf[4..6].copy_from_slice(&length.to_be_bytes());
        buf[6] = 0; // checksum placeholder
        buf[7] = 0;
        buf[8..8 + payload.len()].copy_from_slice(payload);

        // Compute checksum
        let total = length as usize;
        let cksum = pseudo_header_checksum(src_ip, dst_ip, PROTO_UDP, &buf[..total]);
        buf[6..8].copy_from_slice(&cksum.to_be_bytes());

        total
    }
}

/// Metadata for a received UDP packet in the ring buffer.
#[derive(Clone, Copy)]
struct UdpRxEntry {
    src: SocketAddr,
    offset: usize,
    len: usize,
    valid: bool,
}

impl UdpRxEntry {
    const EMPTY: Self = UdpRxEntry {
        src: SocketAddr { ip: Ipv4Addr::ZERO, port: 0 },
        offset: 0,
        len: 0,
        valid: false,
    };
}

/// A bound UDP socket.
pub struct UdpSocket {
    pub local_port: u16,
    pub active: bool,
    rx_buf: [u8; UDP_RX_BUF_SIZE],
    rx_packets: [UdpRxEntry; MAX_RX_PACKETS],
    rx_head: usize,
    rx_tail: usize,
    rx_write_pos: usize,
}

impl UdpSocket {
    const fn new() -> Self {
        UdpSocket {
            local_port: 0,
            active: false,
            rx_buf: [0u8; UDP_RX_BUF_SIZE],
            rx_packets: [UdpRxEntry::EMPTY; MAX_RX_PACKETS],
            rx_head: 0,
            rx_tail: 0,
            rx_write_pos: 0,
        }
    }
}

/// Table of UDP sockets.
pub struct UdpSocketTable {
    pub sockets: [UdpSocket; MAX_UDP_SOCKETS],
    next_ephemeral: u16,
}

impl UdpSocketTable {
    pub const fn new() -> Self {
        UdpSocketTable {
            sockets: [const { UdpSocket::new() }; MAX_UDP_SOCKETS],
            next_ephemeral: 49152,
        }
    }

    /// Bind a UDP socket to a port. Returns socket index.
    pub fn bind(&mut self, port: u16) -> Result<usize, NetError> {
        // Check port not already bound
        for s in &self.sockets {
            if s.active && s.local_port == port {
                return Err(NetError::AddrInUse);
            }
        }
        // Find free slot
        for (i, s) in self.sockets.iter_mut().enumerate() {
            if !s.active {
                s.active = true;
                s.local_port = port;
                s.rx_head = 0;
                s.rx_tail = 0;
                s.rx_write_pos = 0;
                return Ok(i);
            }
        }
        Err(NetError::NoSockets)
    }

    /// Bind to an ephemeral port. Returns (socket index, port).
    pub fn bind_ephemeral(&mut self) -> Result<(usize, u16), NetError> {
        let port = self.next_ephemeral;
        self.next_ephemeral = if self.next_ephemeral >= 65000 { 49152 } else { self.next_ephemeral + 1 };
        let idx = self.bind(port)?;
        Ok((idx, port))
    }

    /// Close a socket.
    pub fn close(&mut self, idx: usize) {
        if idx < MAX_UDP_SOCKETS {
            self.sockets[idx].active = false;
        }
    }

    /// Deliver a received UDP datagram to the appropriate socket.
    pub fn deliver(&mut self, dst_port: u16, src: SocketAddr, data: &[u8]) -> bool {
        for s in self.sockets.iter_mut() {
            if s.active && s.local_port == dst_port {
                // Check if we have room in the packet ring
                let next_tail = (s.rx_tail + 1) % MAX_RX_PACKETS;
                if next_tail == s.rx_head {
                    return false; // ring full
                }
                // Check if we have room in the data buffer
                if s.rx_write_pos + data.len() > UDP_RX_BUF_SIZE {
                    s.rx_write_pos = 0; // wrap around (simple approach)
                }
                let offset = s.rx_write_pos;
                s.rx_buf[offset..offset + data.len()].copy_from_slice(data);
                s.rx_packets[s.rx_tail] = UdpRxEntry {
                    src,
                    offset,
                    len: data.len(),
                    valid: true,
                };
                s.rx_write_pos += data.len();
                s.rx_tail = next_tail;
                return true;
            }
        }
        false
    }

    /// Receive a datagram from a socket. Returns (source, data) or None.
    pub fn recv(&mut self, idx: usize, out_buf: &mut [u8]) -> Option<(SocketAddr, usize)> {
        if idx >= MAX_UDP_SOCKETS || !self.sockets[idx].active {
            return None;
        }
        let s = &mut self.sockets[idx];
        if s.rx_head == s.rx_tail {
            return None; // empty
        }
        let entry = s.rx_packets[s.rx_head];
        if !entry.valid {
            return None;
        }
        let copy_len = entry.len.min(out_buf.len());
        out_buf[..copy_len].copy_from_slice(&s.rx_buf[entry.offset..entry.offset + copy_len]);
        s.rx_packets[s.rx_head].valid = false;
        s.rx_head = (s.rx_head + 1) % MAX_RX_PACKETS;
        Some((entry.src, copy_len))
    }
}
