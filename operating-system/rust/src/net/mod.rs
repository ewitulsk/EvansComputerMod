//! Networking stack — Ethernet → ARP → IPv4 → ICMP/UDP/TCP → DNS.
//!
//! All networking state is in the `NetStack` singleton, initialized at boot.

pub mod types;
pub mod checksum;
pub mod eth;
pub mod arp;
pub mod ipv4;
pub mod icmp;
pub mod udp;
pub mod tcp;
pub mod dns;
pub mod http;

use types::*;
use eth::{EthHeader, ETHERTYPE_ARP, ETHERTYPE_IPV4};
use arp::{ArpPacket, ArpTable, ARP_REQUEST, ARP_REPLY};
use ipv4::{Ipv4Header, PROTO_ICMP, PROTO_TCP, PROTO_UDP};
use icmp::{IcmpPacket, ICMP_ECHO_REQUEST, ICMP_ECHO_REPLY};
use udp::{UdpHeader, UdpSocketTable};
use tcp::{TcpHeader, TcpConnectionTable, TcpAction, TcpState};

/// Global frame buffers.
static mut RX_BUF: [u8; MAX_FRAME_SIZE] = [0u8; MAX_FRAME_SIZE];
static mut TX_BUF: [u8; MAX_FRAME_SIZE] = [0u8; MAX_FRAME_SIZE];
/// Scratch buffer for building IP payloads (TCP/UDP segments, ICMP packets).
static mut PAYLOAD_BUF: [u8; 1500] = [0u8; 1500];

/// The networking stack singleton.
static mut NET_STACK: Option<NetStack> = None;

pub struct NetStack {
    pub mac: MacAddr,
    pub ip: Ipv4Addr,
    pub subnet_mask: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub dns_server: Ipv4Addr,
    pub configured: bool,

    pub arp_table: ArpTable,
    pub routing: ipv4::RoutingTable,
    pub udp_sockets: UdpSocketTable,
    pub tcp_connections: TcpConnectionTable,

    // ICMP ping state
    pub ping_id: u16,
    pub ping_seq: u16,
    pub ping_reply_rtt: Option<u32>,
    pub ping_sent_ms: i64,

    // IP identification counter
    ip_id: u16,

    // Current time (updated by poll)
    pub now_ms: i64,
}

impl NetStack {
    /// Initialize the networking stack. Call once at boot.
    pub fn init() {
        let mac = eth::get_local_mac();
        let stack = NetStack {
            mac,
            ip: Ipv4Addr::ZERO,
            subnet_mask: Ipv4Addr::ZERO,
            gateway: Ipv4Addr::ZERO,
            dns_server: Ipv4Addr::ZERO,
            configured: false,
            arp_table: ArpTable::new(),
            routing: ipv4::RoutingTable::new(),
            udp_sockets: UdpSocketTable::new(),
            tcp_connections: TcpConnectionTable::new(),
            ping_id: 1,
            ping_seq: 0,
            ping_reply_rtt: None,
            ping_sent_ms: 0,
            ip_id: 0,
            now_ms: current_time_ms(),
        };
        unsafe {
            NET_STACK = Some(stack);
        }
    }

    /// Get the singleton.
    pub fn get() -> Option<&'static mut NetStack> {
        unsafe { NET_STACK.as_mut() }
    }

    /// Configure the network interface.
    pub fn configure(&mut self, ip: Ipv4Addr, mask: Ipv4Addr, gw: Ipv4Addr, dns: Ipv4Addr) {
        self.ip = ip;
        self.subnet_mask = mask;
        self.gateway = gw;
        self.dns_server = dns;
        self.configured = true;
        self.routing = ipv4::RoutingTable {
            local_ip: ip,
            subnet_mask: mask,
            gateway: gw,
        };

        // Send gratuitous ARP
        let tx = unsafe { &mut TX_BUF };
        arp::send_gratuitous_arp(tx, &self.mac, &self.ip);
    }

    // ===== Frame Reception =====

    /// Drain all pending frames from the host and process them.
    pub fn poll_rx(&mut self) {
        loop {
            let rx = unsafe { &mut RX_BUF };
            match eth::recv_frame(rx) {
                Some(len) => {
                    // Copy frame data to avoid aliasing issues with TX_BUF
                    let mut frame_copy = [0u8; MAX_FRAME_SIZE];
                    frame_copy[..len].copy_from_slice(&rx[..len]);
                    self.process_frame(&frame_copy[..len]);
                }
                None => break,
            }
        }
    }

    fn process_frame(&mut self, frame: &[u8]) {
        let (eth_hdr, payload) = match EthHeader::parse(frame) {
            Some(v) => v,
            None => return,
        };

        // Filter: only our MAC or broadcast
        if eth_hdr.dst != self.mac && eth_hdr.dst != MacAddr::BROADCAST {
            return;
        }

        match eth_hdr.ethertype {
            ETHERTYPE_ARP => self.handle_arp(payload, &eth_hdr.src),
            ETHERTYPE_IPV4 => self.handle_ipv4(payload),
            _ => {}
        }
    }

    fn handle_arp(&mut self, data: &[u8], _sender_mac: &MacAddr) {
        let pkt = match ArpPacket::parse(data) {
            Some(p) => p,
            None => return,
        };

        // Always learn from ARP packets
        self.arp_table.insert(pkt.sender_ip, pkt.sender_mac, self.now_ms);

        match pkt.operation {
            ARP_REQUEST => {
                if self.configured && pkt.target_ip == self.ip {
                    let tx = unsafe { &mut TX_BUF };
                    arp::send_arp_reply(tx, &self.mac, &self.ip, &pkt.sender_mac, &pkt.sender_ip);
                }
            }
            ARP_REPLY => {
                // Already inserted above
            }
            _ => {}
        }
    }

    fn handle_ipv4(&mut self, data: &[u8]) {
        let (ip_hdr, payload) = match Ipv4Header::parse(data) {
            Some(v) => v,
            None => return,
        };

        // Check destination
        if self.configured && ip_hdr.dst != self.ip && ip_hdr.dst != Ipv4Addr::BROADCAST {
            return;
        }

        match ip_hdr.protocol {
            PROTO_ICMP => self.handle_icmp(payload, &ip_hdr),
            PROTO_UDP => self.handle_udp(payload, &ip_hdr),
            PROTO_TCP => self.handle_tcp(payload, &ip_hdr),
            _ => {}
        }
    }

    fn handle_icmp(&mut self, data: &[u8], ip_hdr: &Ipv4Header) {
        let (pkt, payload) = match IcmpPacket::parse(data) {
            Some(v) => v,
            None => return,
        };

        match pkt.icmp_type {
            ICMP_ECHO_REQUEST => {
                // Reply with echo response
                let pbuf = unsafe { &mut PAYLOAD_BUF };
                let icmp_len = IcmpPacket::serialize_echo(
                    pbuf,
                    ICMP_ECHO_REPLY,
                    pkt.id,
                    pkt.seq,
                    payload,
                );
                if icmp_len > 0 {
                    self.send_ipv4(ip_hdr.src, PROTO_ICMP, &pbuf[..icmp_len]);
                }
            }
            ICMP_ECHO_REPLY => {
                // Check if this is a reply to our ping
                if pkt.id == self.ping_id {
                    let rtt = (self.now_ms - self.ping_sent_ms) as u32;
                    self.ping_reply_rtt = Some(rtt);
                }
            }
            _ => {}
        }
    }

    fn handle_udp(&mut self, data: &[u8], ip_hdr: &Ipv4Header) {
        let (udp_hdr, payload) = match UdpHeader::parse(data) {
            Some(v) => v,
            None => return,
        };

        let src = SocketAddr {
            ip: ip_hdr.src,
            port: udp_hdr.src_port,
        };

        self.udp_sockets.deliver(udp_hdr.dst_port, src, payload);
    }

    fn handle_tcp(&mut self, data: &[u8], ip_hdr: &Ipv4Header) {
        let (tcp_hdr, payload) = match TcpHeader::parse(data) {
            Some(v) => v,
            None => return,
        };

        let actions = self.tcp_connections.process_segment(
            self.ip,
            ip_hdr.src,
            &tcp_hdr,
            payload,
            self.now_ms,
        );

        for action in actions {
            self.execute_tcp_action(action);
        }
    }

    // ===== Frame Transmission =====

    /// Send an IPv4 packet. Resolves MAC via ARP.
    pub fn send_ipv4(&mut self, dst_ip: Ipv4Addr, protocol: u8, payload: &[u8]) -> Result<(), NetError> {
        if !self.configured {
            return Err(NetError::NotConfigured);
        }

        let next_hop = self.routing.next_hop(&dst_ip);

        // For broadcast, use broadcast MAC
        let dst_mac = if dst_ip == Ipv4Addr::BROADCAST {
            MacAddr::BROADCAST
        } else {
            match self.arp_table.lookup(&next_hop, self.now_ms) {
                Some(mac) => mac,
                None => {
                    // Send ARP request
                    let tx = unsafe { &mut TX_BUF };
                    arp::send_arp_request(tx, &self.mac, &self.ip, &next_hop);
                    return Err(NetError::WouldBlock);
                }
            }
        };

        self.send_ipv4_with_mac(dst_ip, dst_mac, protocol, payload)
    }

    fn send_ipv4_with_mac(
        &mut self,
        dst_ip: Ipv4Addr,
        dst_mac: MacAddr,
        protocol: u8,
        payload: &[u8],
    ) -> Result<(), NetError> {
        let tx = unsafe { &mut TX_BUF };
        let ip_total = Ipv4Header::SIZE + payload.len();
        let frame_len = EthHeader::SIZE + ip_total;
        if frame_len > MAX_FRAME_SIZE {
            return Err(NetError::BufferFull);
        }

        // Ethernet header
        let eth_hdr = EthHeader {
            dst: dst_mac,
            src: self.mac,
            ethertype: ETHERTYPE_IPV4,
        };
        eth_hdr.write(&mut tx[..EthHeader::SIZE]);

        // IP header
        let ip_hdr = Ipv4Header::new_outgoing(
            self.ip,
            dst_ip,
            protocol,
            payload.len(),
            self.next_ip_id(),
        );
        ip_hdr.serialize(&mut tx[EthHeader::SIZE..EthHeader::SIZE + Ipv4Header::SIZE]);

        // Payload
        let payload_start = EthHeader::SIZE + Ipv4Header::SIZE;
        tx[payload_start..payload_start + payload.len()].copy_from_slice(payload);

        eth::send_frame(&tx[..frame_len]);
        Ok(())
    }

    fn next_ip_id(&mut self) -> u16 {
        self.ip_id = self.ip_id.wrapping_add(1);
        self.ip_id
    }

    // ===== TCP Actions =====

    fn execute_tcp_action(&mut self, action: TcpAction) {
        match action {
            TcpAction::SendSegment { conn_idx, flags, seq, ack, payload_len } => {
                let c = &self.tcp_connections.connections[conn_idx];
                let local = c.local;
                let remote = c.remote;
                let window = c.rcv_wnd;

                let pbuf = unsafe { &mut PAYLOAD_BUF };

                // Get payload data if any
                let mut tcp_payload = [0u8; 1460];
                let actual_payload_len = if payload_len > 0 && flags & tcp::ACK != 0 {
                    self.tcp_connections.connections[conn_idx].tx_unsent(&mut tcp_payload)
                } else {
                    0
                };

                let seg_len = TcpHeader::serialize(
                    pbuf,
                    local.port,
                    remote.port,
                    seq,
                    ack,
                    flags,
                    window,
                    &tcp_payload[..actual_payload_len],
                    &local.ip,
                    &remote.ip,
                );

                if seg_len > 0 {
                    let _ = self.send_ipv4(remote.ip, PROTO_TCP, &pbuf[..seg_len]);
                    if actual_payload_len > 0 {
                        self.tcp_connections.connections[conn_idx].tx_sent += actual_payload_len;
                        self.tcp_connections.connections[conn_idx].snd_nxt =
                            self.tcp_connections.connections[conn_idx].snd_nxt
                                .wrapping_add(actual_payload_len as u32);
                        self.tcp_connections.connections[conn_idx].retransmit_deadline_ms =
                            self.now_ms + self.tcp_connections.connections[conn_idx].rto_ms;
                    }
                }
            }
            TcpAction::SendReset { src, dst, seq, ack } => {
                let pbuf = unsafe { &mut PAYLOAD_BUF };
                let flags = tcp::RST | if ack != 0 { tcp::ACK } else { 0 };
                let seg_len = TcpHeader::serialize(
                    pbuf, src.port, dst.port, seq, ack, flags, 0, &[], &src.ip, &dst.ip,
                );
                if seg_len > 0 {
                    let _ = self.send_ipv4(dst.ip, PROTO_TCP, &pbuf[..seg_len]);
                }
            }
        }
    }

    // ===== Timer Polling =====

    /// Must be called periodically to drive retransmissions and expiry.
    pub fn poll_timers(&mut self) {
        self.now_ms = current_time_ms();
        self.arp_table.evict_expired(self.now_ms);

        let actions = self.tcp_connections.poll_timers(self.now_ms);
        for action in actions {
            self.execute_tcp_action(action);
        }

        // Transmit any pending TCP data
        self.tcp_flush_all();
    }

    /// Try to send unsent TCP data for all established connections.
    fn tcp_flush_all(&mut self) {
        for i in 0..tcp::MAX_TCP_CONNECTIONS {
            let c = &self.tcp_connections.connections[i];
            if !c.active || c.state != TcpState::Established {
                continue;
            }
            if c.tx_count > c.tx_sent {
                let conn = &self.tcp_connections.connections[i];
                let local = conn.local;
                let remote = conn.remote;
                let seq = conn.snd_nxt;
                let ack = conn.rcv_nxt;
                let window = conn.rcv_wnd;

                let pbuf = unsafe { &mut PAYLOAD_BUF };
                let mut tcp_payload = [0u8; 1460];
                let payload_len = self.tcp_connections.connections[i].tx_unsent(&mut tcp_payload);

                if payload_len > 0 {
                    let seg_len = TcpHeader::serialize(
                        pbuf, local.port, remote.port, seq, ack,
                        tcp::ACK | tcp::PSH, window, &tcp_payload[..payload_len],
                        &local.ip, &remote.ip,
                    );
                    if seg_len > 0 {
                        let _ = self.send_ipv4(remote.ip, PROTO_TCP, &pbuf[..seg_len]);
                        self.tcp_connections.connections[i].tx_sent += payload_len;
                        self.tcp_connections.connections[i].snd_nxt =
                            self.tcp_connections.connections[i].snd_nxt.wrapping_add(payload_len as u32);
                        self.tcp_connections.connections[i].retransmit_deadline_ms =
                            self.now_ms + self.tcp_connections.connections[i].rto_ms;
                    }
                }
            }
        }
    }

    // ===== High-Level API =====

    /// Send a ping and wait for reply.
    pub fn ping(&mut self, target: Ipv4Addr, timeout_ms: u32) -> Result<u32, NetError> {
        if !self.configured {
            return Err(NetError::NotConfigured);
        }

        self.ping_seq = self.ping_seq.wrapping_add(1);
        self.ping_reply_rtt = None;
        self.now_ms = current_time_ms();
        self.ping_sent_ms = self.now_ms;

        // Build ICMP echo request
        let pbuf = unsafe { &mut PAYLOAD_BUF };
        let ping_data = [0u8; 32]; // 32 bytes of payload
        let icmp_len = IcmpPacket::serialize_echo(
            pbuf,
            ICMP_ECHO_REQUEST,
            self.ping_id,
            self.ping_seq,
            &ping_data,
        );

        // Try to send (may need ARP first)
        let mut arp_retries = 3;
        loop {
            match self.send_ipv4(target, PROTO_ICMP, &pbuf[..icmp_len]) {
                Ok(()) => break,
                Err(NetError::WouldBlock) => {
                    if arp_retries == 0 {
                        return Err(NetError::ArpTimeout);
                    }
                    arp_retries -= 1;
                    // Wait for ARP reply
                    crate::terminal::raw_sleep_ms(100);
                    self.now_ms = current_time_ms();
                    self.poll_rx();
                }
                Err(e) => return Err(e),
            }
        }

        // Wait for reply
        let deadline = self.now_ms + timeout_ms as i64;
        while self.now_ms < deadline {
            crate::terminal::raw_sleep_ms(10);
            self.now_ms = current_time_ms();
            self.poll_rx();

            if let Some(rtt) = self.ping_reply_rtt {
                return Ok(rtt);
            }
        }

        Err(NetError::TimedOut)
    }

    /// ARP-resolve an IP, blocking with retries.
    pub fn arp_resolve(&mut self, ip: Ipv4Addr, timeout_ms: u32) -> Result<MacAddr, NetError> {
        // Check cache first
        if let Some(mac) = self.arp_table.lookup(&ip, self.now_ms) {
            return Ok(mac);
        }

        let tx = unsafe { &mut TX_BUF };
        let mut attempts = 0;
        let max_attempts = 3;
        let attempt_interval = timeout_ms / max_attempts;

        while attempts < max_attempts {
            arp::send_arp_request(tx, &self.mac, &self.ip, &ip);
            attempts += 1;

            let deadline = current_time_ms() + attempt_interval as i64;
            while current_time_ms() < deadline {
                crate::terminal::raw_sleep_ms(10);
                self.now_ms = current_time_ms();
                self.poll_rx();

                if let Some(mac) = self.arp_table.lookup(&ip, self.now_ms) {
                    return Ok(mac);
                }
            }
        }

        Err(NetError::ArpTimeout)
    }

    /// Send a UDP datagram.
    pub fn udp_send(
        &mut self,
        sock_idx: usize,
        dst: SocketAddr,
        data: &[u8],
    ) -> Result<(), NetError> {
        let src_port = self.udp_sockets.sockets[sock_idx].local_port;
        let pbuf = unsafe { &mut PAYLOAD_BUF };
        let udp_len = UdpHeader::serialize(pbuf, src_port, dst.port, data, &self.ip, &dst.ip);

        // Try to send, with ARP retry
        let mut retries = 3;
        loop {
            match self.send_ipv4(dst.ip, PROTO_UDP, &pbuf[..udp_len]) {
                Ok(()) => return Ok(()),
                Err(NetError::WouldBlock) => {
                    if retries == 0 {
                        return Err(NetError::ArpTimeout);
                    }
                    retries -= 1;
                    crate::terminal::raw_sleep_ms(100);
                    self.now_ms = current_time_ms();
                    self.poll_rx();
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Resolve a hostname via DNS.
    pub fn dns_resolve(&mut self, name: &str, timeout_ms: u32) -> Result<Ipv4Addr, NetError> {
        if !self.configured {
            return Err(NetError::NotConfigured);
        }

        let (sock_idx, src_port) = self.udp_sockets.bind_ephemeral()?;

        // Build DNS query
        let mut query_buf = [0u8; 512];
        let tx_id = (self.now_ms & 0xFFFF) as u16;
        let query_len = dns::build_query(name, tx_id, &mut query_buf);
        if query_len == 0 {
            self.udp_sockets.close(sock_idx);
            return Err(NetError::InvalidPacket);
        }

        // Send query
        let dst = SocketAddr { ip: self.dns_server, port: dns::dns_port() };
        self.udp_send(sock_idx, dst, &query_buf[..query_len])?;

        // Wait for response
        let deadline = current_time_ms() + timeout_ms as i64;
        let mut recv_buf = [0u8; 512];
        loop {
            self.now_ms = current_time_ms();
            if self.now_ms >= deadline {
                break;
            }

            if let Some((_src, len)) = self.udp_sockets.recv(sock_idx, &mut recv_buf) {
                if let Some(ip) = dns::parse_response(&recv_buf[..len]) {
                    self.udp_sockets.close(sock_idx);
                    return Ok(ip);
                }
            }

            crate::terminal::raw_sleep_ms(10);
            self.poll_rx();
        }

        self.udp_sockets.close(sock_idx);
        Err(NetError::TimedOut)
    }

    // ===== TCP High-Level API =====

    /// Initiate a TCP connection (blocking).
    pub fn tcp_connect(&mut self, remote: SocketAddr, timeout_ms: u32) -> Result<usize, NetError> {
        // First ensure we have the MAC for the next hop
        let next_hop = self.routing.next_hop(&remote.ip);
        self.arp_resolve(next_hop, 1500)?;

        let idx = self.tcp_connections.connect(self.ip, remote, self.now_ms)?;

        // Send SYN
        let c = &self.tcp_connections.connections[idx];
        let action = TcpAction::SendSegment {
            conn_idx: idx,
            flags: tcp::SYN,
            seq: c.iss,
            ack: 0,
            payload_len: 0,
        };
        self.execute_tcp_action(action);

        // Wait for ESTABLISHED
        let deadline = current_time_ms() + timeout_ms as i64;
        loop {
            self.now_ms = current_time_ms();
            if self.now_ms >= deadline {
                self.tcp_connections.connections[idx].state = TcpState::Closed;
                self.tcp_connections.connections[idx].active = false;
                return Err(NetError::TimedOut);
            }

            self.poll_rx();
            self.poll_timers();

            let state = self.tcp_connections.connections[idx].state;
            match state {
                TcpState::Established => return Ok(idx),
                TcpState::Closed => return Err(NetError::ConnectionRefused),
                _ => {}
            }

            crate::terminal::raw_sleep_ms(10);
        }
    }

    /// Accept a TCP connection (blocking).
    pub fn tcp_accept(&mut self, listener_idx: usize, timeout_ms: u32) -> Result<usize, NetError> {
        let port = self.tcp_connections.connections[listener_idx].local.port;
        let deadline = current_time_ms() + timeout_ms as i64;

        loop {
            self.now_ms = current_time_ms();
            if self.now_ms >= deadline {
                return Err(NetError::TimedOut);
            }

            // Check for established connection from this listener
            if let Some(idx) = self.tcp_connections.find_established_from_listener(port) {
                // Clear the listener_port so it's not found again
                self.tcp_connections.connections[idx].listener_port = 0;
                return Ok(idx);
            }

            self.poll_rx();
            self.poll_timers();
            crate::terminal::raw_sleep_ms(10);
        }
    }

    /// Send data on a TCP connection.
    pub fn tcp_send(&mut self, idx: usize, data: &[u8]) -> Result<usize, NetError> {
        let c = &mut self.tcp_connections.connections[idx];
        if !c.active || c.state != TcpState::Established {
            return Err(NetError::NotConnected);
        }
        let written = c.write(data);
        // Flush immediately
        self.tcp_flush_all();
        Ok(written)
    }

    /// Receive data from a TCP connection (blocking).
    pub fn tcp_recv(&mut self, idx: usize, buf: &mut [u8], timeout_ms: u32) -> Result<usize, NetError> {
        let deadline = current_time_ms() + timeout_ms as i64;

        loop {
            let c = &mut self.tcp_connections.connections[idx];
            if !c.active {
                return Err(NetError::NotConnected);
            }

            // Check for data
            if c.rx_available() > 0 {
                return Ok(c.read(buf));
            }

            // Connection closed by peer
            if c.state == TcpState::CloseWait || c.state == TcpState::Closed {
                return Ok(0); // EOF
            }

            self.now_ms = current_time_ms();
            if self.now_ms >= deadline {
                return Err(NetError::TimedOut);
            }

            self.poll_rx();
            self.poll_timers();
            crate::terminal::raw_sleep_ms(10);
        }
    }

    /// Close a TCP connection.
    pub fn tcp_close(&mut self, idx: usize) {
        if let Some(action) = self.tcp_connections.close(idx, self.now_ms) {
            self.execute_tcp_action(action);
        }
    }
}

fn current_time_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
