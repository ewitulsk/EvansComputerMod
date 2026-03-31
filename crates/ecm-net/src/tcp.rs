//! TCP state machine and connection management.

use alloc::vec::Vec;
use super::types::{Ipv4Addr, SocketAddr, NetError};
use super::checksum::pseudo_header_checksum;
use super::ipv4::PROTO_TCP;

pub const MAX_TCP_CONNECTIONS: usize = 8;
const TCP_RX_BUF_SIZE: usize = 4096;
const TCP_TX_BUF_SIZE: usize = 4096;
pub const TCP_MSS: u16 = 1460;
const TCP_INITIAL_RTO_MS: i64 = 1000;
const TCP_MAX_RTO_MS: i64 = 60000;
const TCP_MAX_RETRIES: u8 = 5;
const TCP_TIME_WAIT_MS: i64 = 2000;

// TCP flags
pub const FIN: u8 = 0x01;
pub const SYN: u8 = 0x02;
pub const RST: u8 = 0x04;
pub const PSH: u8 = 0x08;
pub const ACK: u8 = 0x10;

/// TCP connection state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}

/// TCP header (20 bytes, no options).
pub struct TcpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq_num: u32,
    pub ack_num: u32,
    pub data_offset: u8,
    pub flags: u8,
    pub window: u16,
    pub checksum: u16,
    pub urgent_ptr: u16,
}

impl TcpHeader {
    pub const MIN_SIZE: usize = 20;

    pub fn parse(data: &[u8]) -> Option<(Self, &[u8])> {
        if data.len() < 20 {
            return None;
        }
        let data_offset = (data[12] >> 4) as usize;
        let header_len = data_offset * 4;
        if data.len() < header_len {
            return None;
        }

        let hdr = TcpHeader {
            src_port: u16::from_be_bytes([data[0], data[1]]),
            dst_port: u16::from_be_bytes([data[2], data[3]]),
            seq_num: u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
            ack_num: u32::from_be_bytes([data[8], data[9], data[10], data[11]]),
            data_offset: data_offset as u8,
            flags: data[13],
            window: u16::from_be_bytes([data[14], data[15]]),
            checksum: u16::from_be_bytes([data[16], data[17]]),
            urgent_ptr: u16::from_be_bytes([data[18], data[19]]),
        };

        Some((hdr, &data[header_len..]))
    }

    /// Serialize a TCP header + payload into buf. Computes checksum.
    /// Returns total segment length.
    pub fn serialize(
        buf: &mut [u8],
        src_port: u16,
        dst_port: u16,
        seq_num: u32,
        ack_num: u32,
        flags: u8,
        window: u16,
        payload: &[u8],
        src_ip: &Ipv4Addr,
        dst_ip: &Ipv4Addr,
    ) -> usize {
        let header_len = 20;
        let total_len = header_len + payload.len();
        if buf.len() < total_len {
            return 0;
        }

        buf[0..2].copy_from_slice(&src_port.to_be_bytes());
        buf[2..4].copy_from_slice(&dst_port.to_be_bytes());
        buf[4..8].copy_from_slice(&seq_num.to_be_bytes());
        buf[8..12].copy_from_slice(&ack_num.to_be_bytes());
        buf[12] = (5 << 4); // data offset = 5 words (20 bytes)
        buf[13] = flags;
        buf[14..16].copy_from_slice(&window.to_be_bytes());
        buf[16] = 0; // checksum placeholder
        buf[17] = 0;
        buf[18] = 0; // urgent pointer
        buf[19] = 0;

        if !payload.is_empty() {
            buf[20..total_len].copy_from_slice(payload);
        }

        // Compute checksum
        let cksum = pseudo_header_checksum(src_ip, dst_ip, PROTO_TCP, &buf[..total_len]);
        buf[16..18].copy_from_slice(&cksum.to_be_bytes());

        total_len
    }
}

/// A single TCP connection.
pub struct TcpConnection {
    pub state: TcpState,
    pub local: SocketAddr,
    pub remote: SocketAddr,
    pub active: bool,

    // Send sequence space
    pub iss: u32,
    pub snd_una: u32,
    pub snd_nxt: u32,
    pub snd_wnd: u16,

    // Receive sequence space
    pub irs: u32,
    pub rcv_nxt: u32,
    pub rcv_wnd: u16,

    // Receive buffer (circular)
    pub rx_buf: [u8; TCP_RX_BUF_SIZE],
    pub rx_head: usize,
    pub rx_count: usize,

    // Send buffer (circular)
    pub tx_buf: [u8; TCP_TX_BUF_SIZE],
    pub tx_head: usize,
    pub tx_count: usize,
    pub tx_sent: usize, // bytes sent but unacked

    // Retransmission
    pub rto_ms: i64,
    pub retransmit_deadline_ms: i64,
    pub retransmit_count: u8,

    // TIME_WAIT
    pub time_wait_deadline_ms: i64,

    // For listeners: port that spawned this connection
    pub listener_port: u16,
}

impl TcpConnection {
    pub const fn new() -> Self {
        TcpConnection {
            state: TcpState::Closed,
            local: SocketAddr { ip: Ipv4Addr::ZERO, port: 0 },
            remote: SocketAddr { ip: Ipv4Addr::ZERO, port: 0 },
            active: false,
            iss: 0,
            snd_una: 0,
            snd_nxt: 0,
            snd_wnd: 0,
            irs: 0,
            rcv_nxt: 0,
            rcv_wnd: TCP_RX_BUF_SIZE as u16,
            rx_buf: [0; TCP_RX_BUF_SIZE],
            rx_head: 0,
            rx_count: 0,
            tx_buf: [0; TCP_TX_BUF_SIZE],
            tx_head: 0,
            tx_count: 0,
            tx_sent: 0,
            rto_ms: TCP_INITIAL_RTO_MS,
            retransmit_deadline_ms: 0,
            retransmit_count: 0,
            time_wait_deadline_ms: 0,
            listener_port: 0,
        }
    }

    /// Available space in the receive buffer.
    pub fn rx_available(&self) -> usize {
        self.rx_count
    }

    /// Read data from the receive buffer.
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let read_len = buf.len().min(self.rx_count);
        for i in 0..read_len {
            buf[i] = self.rx_buf[(self.rx_head + i) % TCP_RX_BUF_SIZE];
        }
        self.rx_head = (self.rx_head + read_len) % TCP_RX_BUF_SIZE;
        self.rx_count -= read_len;
        self.rcv_wnd = (TCP_RX_BUF_SIZE - self.rx_count) as u16;
        read_len
    }

    /// Write data into the receive buffer (called when data arrives).
    pub fn rx_write(&mut self, data: &[u8]) -> usize {
        let space = TCP_RX_BUF_SIZE - self.rx_count;
        let write_len = data.len().min(space);
        let write_pos = (self.rx_head + self.rx_count) % TCP_RX_BUF_SIZE;
        for i in 0..write_len {
            self.rx_buf[(write_pos + i) % TCP_RX_BUF_SIZE] = data[i];
        }
        self.rx_count += write_len;
        self.rcv_wnd = (TCP_RX_BUF_SIZE - self.rx_count) as u16;
        write_len
    }

    /// Queue data for sending.
    pub fn write(&mut self, data: &[u8]) -> usize {
        let space = TCP_TX_BUF_SIZE - self.tx_count;
        let write_len = data.len().min(space);
        let write_pos = (self.tx_head + self.tx_count) % TCP_TX_BUF_SIZE;
        for i in 0..write_len {
            self.tx_buf[(write_pos + i) % TCP_TX_BUF_SIZE] = data[i];
        }
        self.tx_count += write_len;
        write_len
    }

    /// Get unsent data from the tx buffer for transmission.
    pub fn tx_unsent(&self, buf: &mut [u8]) -> usize {
        let unsent = self.tx_count - self.tx_sent;
        let read_len = buf.len().min(unsent).min(TCP_MSS as usize);
        let read_pos = (self.tx_head + self.tx_sent) % TCP_TX_BUF_SIZE;
        for i in 0..read_len {
            buf[i] = self.tx_buf[(read_pos + i) % TCP_TX_BUF_SIZE];
        }
        read_len
    }

    /// Acknowledge sent data (advance snd_una, free tx buffer).
    pub fn ack_data(&mut self, ack_num: u32) {
        let acked = ack_num.wrapping_sub(self.snd_una) as usize;
        if acked > 0 && acked <= self.tx_sent {
            self.tx_head = (self.tx_head + acked) % TCP_TX_BUF_SIZE;
            self.tx_count -= acked;
            self.tx_sent -= acked;
            self.snd_una = ack_num;
            self.retransmit_count = 0;
            self.rto_ms = TCP_INITIAL_RTO_MS;
        }
    }
}

/// Actions the TCP layer requests the stack to perform.
pub enum TcpAction {
    /// Send a TCP segment for connection at index.
    SendSegment {
        conn_idx: usize,
        flags: u8,
        seq: u32,
        ack: u32,
        payload_len: usize,
    },
    /// Send a RST.
    SendReset {
        src: SocketAddr,
        dst: SocketAddr,
        seq: u32,
        ack: u32,
    },
}

/// TCP connection table.
pub struct TcpConnectionTable {
    pub connections: [TcpConnection; MAX_TCP_CONNECTIONS],
    next_ephemeral: u16,
}

impl TcpConnectionTable {
    pub const fn new() -> Self {
        TcpConnectionTable {
            connections: [const { TcpConnection::new() }; MAX_TCP_CONNECTIONS],
            next_ephemeral: 49152,
        }
    }

    /// Allocate a new connection slot.
    pub fn alloc(&mut self) -> Result<usize, NetError> {
        for (i, c) in self.connections.iter_mut().enumerate() {
            if !c.active {
                *c = TcpConnection::new();
                c.active = true;
                return Ok(i);
            }
        }
        Err(NetError::NoSockets)
    }

    /// Find a connection by 4-tuple.
    pub fn find(&self, local: &SocketAddr, remote: &SocketAddr) -> Option<usize> {
        for (i, c) in self.connections.iter().enumerate() {
            if c.active && c.state != TcpState::Listen
                && c.local == *local && c.remote == *remote
            {
                return Some(i);
            }
        }
        None
    }

    /// Find a listener on the given port.
    pub fn find_listener(&self, port: u16) -> Option<usize> {
        for (i, c) in self.connections.iter().enumerate() {
            if c.active && c.state == TcpState::Listen && c.local.port == port {
                return Some(i);
            }
        }
        None
    }

    /// Find a SynReceived connection spawned from a listener.
    pub fn find_syn_received(&self, listener_port: u16) -> Option<usize> {
        for (i, c) in self.connections.iter().enumerate() {
            if c.active && c.state == TcpState::SynReceived && c.listener_port == listener_port {
                return Some(i);
            }
        }
        None
    }

    /// Find an Established connection spawned from a listener.
    pub fn find_established_from_listener(&self, listener_port: u16) -> Option<usize> {
        for (i, c) in self.connections.iter().enumerate() {
            if c.active && c.state == TcpState::Established && c.listener_port == listener_port {
                return Some(i);
            }
        }
        None
    }

    fn next_ephemeral(&mut self) -> u16 {
        let p = self.next_ephemeral;
        self.next_ephemeral = if p >= 65000 { 49152 } else { p + 1 };
        p
    }

    /// Initiate a TCP connect (active open).
    pub fn connect(
        &mut self,
        local_ip: Ipv4Addr,
        remote: SocketAddr,
        now_ms: i64,
    ) -> Result<usize, NetError> {
        let idx = self.alloc()?;
        let port = self.next_ephemeral();
        let c = &mut self.connections[idx];

        // Generate ISN from timestamp
        c.iss = (now_ms as u32).wrapping_mul(0x9e37_79b9);
        c.snd_una = c.iss;
        c.snd_nxt = c.iss.wrapping_add(1);
        c.local = SocketAddr { ip: local_ip, port };
        c.remote = remote;
        c.state = TcpState::SynSent;
        c.retransmit_deadline_ms = now_ms + TCP_INITIAL_RTO_MS;

        Ok(idx)
    }

    /// Start listening on a port (passive open).
    pub fn listen(&mut self, local_ip: Ipv4Addr, port: u16) -> Result<usize, NetError> {
        let idx = self.alloc()?;
        let c = &mut self.connections[idx];
        c.local = SocketAddr { ip: local_ip, port };
        c.state = TcpState::Listen;
        Ok(idx)
    }

    /// Process an incoming TCP segment.
    /// Returns a list of actions for the stack to execute.
    pub fn process_segment(
        &mut self,
        local_ip: Ipv4Addr,
        src_ip: Ipv4Addr,
        hdr: &TcpHeader,
        data: &[u8],
        now_ms: i64,
    ) -> Vec<TcpAction> {
        let mut actions = Vec::new();
        let local = SocketAddr { ip: local_ip, port: hdr.dst_port };
        let remote = SocketAddr { ip: src_ip, port: hdr.src_port };

        // Find existing connection by 4-tuple
        if let Some(idx) = self.find(&local, &remote) {
            self.process_for_connection(idx, hdr, data, now_ms, &mut actions);
            return actions;
        }

        // Check for listener
        if let Some(listener_idx) = self.find_listener(hdr.dst_port) {
            if hdr.flags & SYN != 0 && hdr.flags & ACK == 0 {
                // New connection attempt — allocate a new connection
                let listen_port = self.connections[listener_idx].local.port;
                if let Ok(new_idx) = self.alloc() {
                    let c = &mut self.connections[new_idx];
                    c.local = local;
                    c.remote = remote;
                    c.irs = hdr.seq_num;
                    c.rcv_nxt = hdr.seq_num.wrapping_add(1);
                    c.iss = (now_ms as u32).wrapping_mul(0x9e37_79b9);
                    c.snd_una = c.iss;
                    c.snd_nxt = c.iss.wrapping_add(1);
                    c.snd_wnd = hdr.window;
                    c.state = TcpState::SynReceived;
                    c.listener_port = listen_port;
                    c.retransmit_deadline_ms = now_ms + TCP_INITIAL_RTO_MS;

                    actions.push(TcpAction::SendSegment {
                        conn_idx: new_idx,
                        flags: SYN | ACK,
                        seq: c.iss,
                        ack: c.rcv_nxt,
                        payload_len: 0,
                    });
                }
            }
            return actions;
        }

        // No connection found — send RST if not a RST
        if hdr.flags & RST == 0 {
            if hdr.flags & ACK != 0 {
                actions.push(TcpAction::SendReset {
                    src: local,
                    dst: remote,
                    seq: hdr.ack_num,
                    ack: 0,
                });
            } else {
                let ack = hdr.seq_num.wrapping_add(data.len() as u32)
                    .wrapping_add(if hdr.flags & SYN != 0 { 1 } else { 0 })
                    .wrapping_add(if hdr.flags & FIN != 0 { 1 } else { 0 });
                actions.push(TcpAction::SendReset {
                    src: local,
                    dst: remote,
                    seq: 0,
                    ack,
                });
            }
        }

        actions
    }

    fn process_for_connection(
        &mut self,
        idx: usize,
        hdr: &TcpHeader,
        data: &[u8],
        now_ms: i64,
        actions: &mut Vec<TcpAction>,
    ) {
        let c = &mut self.connections[idx];

        // RST handling
        if hdr.flags & RST != 0 {
            c.state = TcpState::Closed;
            c.active = false;
            return;
        }

        match c.state {
            TcpState::SynSent => {
                if hdr.flags & SYN != 0 && hdr.flags & ACK != 0 {
                    // SYN-ACK received
                    c.irs = hdr.seq_num;
                    c.rcv_nxt = hdr.seq_num.wrapping_add(1);
                    c.snd_una = hdr.ack_num;
                    c.snd_wnd = hdr.window;
                    c.state = TcpState::Established;
                    c.retransmit_count = 0;

                    actions.push(TcpAction::SendSegment {
                        conn_idx: idx,
                        flags: ACK,
                        seq: c.snd_nxt,
                        ack: c.rcv_nxt,
                        payload_len: 0,
                    });
                }
            }
            TcpState::SynReceived => {
                if hdr.flags & ACK != 0 {
                    c.snd_una = hdr.ack_num;
                    c.snd_wnd = hdr.window;
                    c.state = TcpState::Established;
                    c.retransmit_count = 0;
                }
            }
            TcpState::Established => {
                // Process ACK
                if hdr.flags & ACK != 0 {
                    c.ack_data(hdr.ack_num);
                    c.snd_wnd = hdr.window;
                }

                // Process data
                if !data.is_empty() && hdr.seq_num == c.rcv_nxt {
                    let written = c.rx_write(data);
                    c.rcv_nxt = c.rcv_nxt.wrapping_add(written as u32);

                    // Send ACK
                    actions.push(TcpAction::SendSegment {
                        conn_idx: idx,
                        flags: ACK,
                        seq: c.snd_nxt,
                        ack: c.rcv_nxt,
                        payload_len: 0,
                    });
                }

                // FIN received
                if hdr.flags & FIN != 0 {
                    c.rcv_nxt = c.rcv_nxt.wrapping_add(1);
                    c.state = TcpState::CloseWait;
                    actions.push(TcpAction::SendSegment {
                        conn_idx: idx,
                        flags: ACK,
                        seq: c.snd_nxt,
                        ack: c.rcv_nxt,
                        payload_len: 0,
                    });
                }
            }
            TcpState::FinWait1 => {
                if hdr.flags & ACK != 0 {
                    c.ack_data(hdr.ack_num);
                    c.snd_wnd = hdr.window;
                }
                if hdr.flags & FIN != 0 {
                    c.rcv_nxt = c.rcv_nxt.wrapping_add(1);
                    if hdr.flags & ACK != 0 && hdr.ack_num == c.snd_nxt {
                        // FIN+ACK: go straight to TIME_WAIT
                        c.state = TcpState::TimeWait;
                        c.time_wait_deadline_ms = now_ms + TCP_TIME_WAIT_MS;
                    } else {
                        // Simultaneous close
                        c.state = TcpState::Closing;
                    }
                    actions.push(TcpAction::SendSegment {
                        conn_idx: idx,
                        flags: ACK,
                        seq: c.snd_nxt,
                        ack: c.rcv_nxt,
                        payload_len: 0,
                    });
                } else if hdr.flags & ACK != 0 && hdr.ack_num == c.snd_nxt {
                    c.state = TcpState::FinWait2;
                }
            }
            TcpState::FinWait2 => {
                if hdr.flags & FIN != 0 {
                    c.rcv_nxt = c.rcv_nxt.wrapping_add(1);
                    c.state = TcpState::TimeWait;
                    c.time_wait_deadline_ms = now_ms + TCP_TIME_WAIT_MS;
                    actions.push(TcpAction::SendSegment {
                        conn_idx: idx,
                        flags: ACK,
                        seq: c.snd_nxt,
                        ack: c.rcv_nxt,
                        payload_len: 0,
                    });
                }
            }
            TcpState::CloseWait => {
                // Waiting for application to close
                if hdr.flags & ACK != 0 {
                    c.ack_data(hdr.ack_num);
                }
            }
            TcpState::LastAck => {
                if hdr.flags & ACK != 0 {
                    c.state = TcpState::Closed;
                    c.active = false;
                }
            }
            TcpState::Closing => {
                if hdr.flags & ACK != 0 {
                    c.state = TcpState::TimeWait;
                    c.time_wait_deadline_ms = now_ms + TCP_TIME_WAIT_MS;
                }
            }
            TcpState::TimeWait => {
                // Ignore
            }
            _ => {}
        }
    }

    /// Close a connection (initiate FIN).
    pub fn close(&mut self, idx: usize, now_ms: i64) -> Option<TcpAction> {
        if idx >= MAX_TCP_CONNECTIONS || !self.connections[idx].active {
            return None;
        }
        let c = &mut self.connections[idx];
        match c.state {
            TcpState::Established => {
                c.state = TcpState::FinWait1;
                let seq = c.snd_nxt;
                c.snd_nxt = c.snd_nxt.wrapping_add(1);
                c.retransmit_deadline_ms = now_ms + c.rto_ms;
                Some(TcpAction::SendSegment {
                    conn_idx: idx,
                    flags: FIN | ACK,
                    seq,
                    ack: c.rcv_nxt,
                    payload_len: 0,
                })
            }
            TcpState::CloseWait => {
                c.state = TcpState::LastAck;
                let seq = c.snd_nxt;
                c.snd_nxt = c.snd_nxt.wrapping_add(1);
                c.retransmit_deadline_ms = now_ms + c.rto_ms;
                Some(TcpAction::SendSegment {
                    conn_idx: idx,
                    flags: FIN | ACK,
                    seq,
                    ack: c.rcv_nxt,
                    payload_len: 0,
                })
            }
            TcpState::Listen | TcpState::SynSent => {
                c.state = TcpState::Closed;
                c.active = false;
                None
            }
            _ => None,
        }
    }

    /// Poll timers for retransmission and TIME_WAIT expiry.
    pub fn poll_timers(&mut self, now_ms: i64) -> Vec<TcpAction> {
        let mut actions = Vec::new();

        for i in 0..MAX_TCP_CONNECTIONS {
            let c = &mut self.connections[i];
            if !c.active {
                continue;
            }

            match c.state {
                TcpState::TimeWait => {
                    if now_ms >= c.time_wait_deadline_ms {
                        c.state = TcpState::Closed;
                        c.active = false;
                    }
                }
                TcpState::SynSent | TcpState::SynReceived => {
                    if now_ms >= c.retransmit_deadline_ms {
                        if c.retransmit_count >= TCP_MAX_RETRIES {
                            c.state = TcpState::Closed;
                            c.active = false;
                        } else {
                            c.retransmit_count += 1;
                            c.rto_ms = (c.rto_ms * 2).min(TCP_MAX_RTO_MS);
                            c.retransmit_deadline_ms = now_ms + c.rto_ms;

                            let flags = if c.state == TcpState::SynSent { SYN } else { SYN | ACK };
                            actions.push(TcpAction::SendSegment {
                                conn_idx: i,
                                flags,
                                seq: c.iss,
                                ack: c.rcv_nxt,
                                payload_len: 0,
                            });
                        }
                    }
                }
                TcpState::Established => {
                    // Retransmit unacked data
                    if c.tx_sent > 0 && now_ms >= c.retransmit_deadline_ms {
                        if c.retransmit_count >= TCP_MAX_RETRIES {
                            c.state = TcpState::Closed;
                            c.active = false;
                        } else {
                            c.retransmit_count += 1;
                            c.rto_ms = (c.rto_ms * 2).min(TCP_MAX_RTO_MS);
                            c.retransmit_deadline_ms = now_ms + c.rto_ms;
                            // Re-send from snd_una
                            c.tx_sent = 0; // reset so tx_unsent returns all data
                        }
                    }
                }
                TcpState::FinWait1 | TcpState::LastAck => {
                    if now_ms >= c.retransmit_deadline_ms {
                        if c.retransmit_count >= TCP_MAX_RETRIES {
                            c.state = TcpState::Closed;
                            c.active = false;
                        } else {
                            c.retransmit_count += 1;
                            c.rto_ms = (c.rto_ms * 2).min(TCP_MAX_RTO_MS);
                            c.retransmit_deadline_ms = now_ms + c.rto_ms;
                            let flags = FIN | ACK;
                            actions.push(TcpAction::SendSegment {
                                conn_idx: i,
                                flags,
                                seq: c.snd_nxt.wrapping_sub(1),
                                ack: c.rcv_nxt,
                                payload_len: 0,
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        actions
    }
}
