//! Multi-interface networking stack — Ethernet → ARP → IPv4 → ICMP/UDP/TCP → DNS.
//!
//! Each computer has multiple network interfaces (eth0-ethN), each with its own
//! MAC address, IP configuration, VLAN setting, and ARP table. Routing is done
//! via a proper routing table with longest-prefix match.
//!
//! This crate is `no_std` and uses host functions for sleep and time.

#![no_std]

extern crate alloc;

extern "C" {
    fn sleep_ms(milliseconds: i32);
    fn get_time_ms() -> i64;
}

pub fn host_sleep_ms(ms: u32) {
    unsafe { sleep_ms(ms as i32); }
}


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
use eth::{EthHeader, VlanTag, ETHERTYPE_ARP, ETHERTYPE_IPV4};
use arp::{ArpPacket, ArpTable, ARP_REQUEST, ARP_REPLY};
use ipv4::{Ipv4Header, PROTO_ICMP, PROTO_TCP, PROTO_UDP};
use icmp::{IcmpPacket, ICMP_ECHO_REQUEST, ICMP_ECHO_REPLY};
use udp::{UdpHeader, UdpSocketTable};
use tcp::{TcpHeader, TcpConnectionTable, TcpAction, TcpState};

/// Global frame buffers.
static mut RX_BUF: [u8; MAX_FRAME_SIZE] = [0u8; MAX_FRAME_SIZE];
static mut TX_BUF: [u8; MAX_FRAME_SIZE] = [0u8; MAX_FRAME_SIZE];
static mut PAYLOAD_BUF: [u8; 1500] = [0u8; 1500];

/// The networking stack singleton.
static mut NET_STACK: Option<NetStack> = None;

pub const MAX_INTERFACES: usize = 12;

/// A single network interface.
pub struct NetworkInterface {
    pub index: usize,
    pub mac: MacAddr,
    pub ip: Ipv4Addr,
    pub prefix_len: u8,
    pub vlan: Option<u16>,
    pub link_up: bool,
    pub hw_present: bool,
    pub arp_table: ArpTable,
    name_buf: [u8; 8],
    name_len: usize,
}

impl NetworkInterface {
    const fn empty() -> Self {
        NetworkInterface {
            index: 0,
            mac: MacAddr::ZERO,
            ip: Ipv4Addr::ZERO,
            prefix_len: 0,
            vlan: None,
            link_up: true,
            hw_present: false,
            arp_table: ArpTable::new(),
            name_buf: [0; 8],
            name_len: 0,
        }
    }

    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name_buf[..self.name_len]).unwrap_or("?")
    }

    pub fn configured(&self) -> bool {
        self.ip != Ipv4Addr::ZERO && self.prefix_len > 0
    }

    pub fn subnet_mask(&self) -> Ipv4Addr {
        Ipv4Addr::mask_from_prefix(self.prefix_len)
    }

    pub fn vlan_tag(&self) -> Option<VlanTag> {
        self.vlan.map(VlanTag::new)
    }

    fn set_name(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let len = bytes.len().min(self.name_buf.len());
        self.name_buf[..len].copy_from_slice(&bytes[..len]);
        self.name_len = len;
    }
}

/// The multi-interface networking stack.
pub struct NetStack {
    pub interfaces: [NetworkInterface; MAX_INTERFACES],
    pub iface_count: usize,

    pub routing: ipv4::RoutingTable,
    pub dns_server: Ipv4Addr,

    pub udp_sockets: UdpSocketTable,
    pub tcp_connections: TcpConnectionTable,

    pub ping_id: u16,
    pub ping_seq: u16,
    pub ping_reply_rtt: Option<u32>,
    pub ping_sent_ms: i64,

    ip_id: u16,
    pub now_ms: i64,

    /// Raw ICMP reply buffer for IPC raw sockets.
    /// Stores (source_ip, icmp_packet_data) for delivery to raw socket owners.
    pub raw_icmp_replies: [(Ipv4Addr, [u8; 128], usize); 4],
    pub raw_icmp_reply_count: usize,
}

impl NetStack {
    /// Initialize the networking stack. Discovers interfaces from the host.
    pub fn init() {
        let iface_count = eth::get_interface_count().min(MAX_INTERFACES);

        let mut interfaces = [
            NetworkInterface::empty(), NetworkInterface::empty(),
            NetworkInterface::empty(), NetworkInterface::empty(),
            NetworkInterface::empty(), NetworkInterface::empty(),
            NetworkInterface::empty(), NetworkInterface::empty(),
            NetworkInterface::empty(), NetworkInterface::empty(),
            NetworkInterface::empty(), NetworkInterface::empty(),
        ];

        for i in 0..iface_count {
            interfaces[i].index = i;
            interfaces[i].mac = eth::get_interface_mac(i);
            interfaces[i].hw_present = true;
            interfaces[i].link_up = true;
            // Generate name "ethN"
            let name = format_iface_name(i);
            interfaces[i].set_name(&name);
        }

        let stack = NetStack {
            interfaces,
            iface_count,
            routing: ipv4::RoutingTable::new(),
            dns_server: Ipv4Addr::ZERO,
            udp_sockets: UdpSocketTable::new(),
            tcp_connections: TcpConnectionTable::new(),
            ping_id: 1,
            ping_seq: 0,
            ping_reply_rtt: None,
            ping_sent_ms: 0,
            ip_id: 0,
            now_ms: current_time_ms(),
            raw_icmp_replies: [(Ipv4Addr::ZERO, [0u8; 128], 0); 4],
            raw_icmp_reply_count: 0,
        };
        unsafe { NET_STACK = Some(stack); }
    }

    pub fn get() -> Option<&'static mut NetStack> {
        unsafe { NET_STACK.as_mut() }
    }

    /// Find interface by name. Returns index or None.
    pub fn find_iface(&self, name: &str) -> Option<usize> {
        for i in 0..self.iface_count {
            if self.interfaces[i].name_str() == name {
                return Some(i);
            }
        }
        None
    }

    /// Set link state on an interface and notify the host (for visual cable disconnect).
    pub fn set_link_state(&mut self, iface_idx: usize, up: bool) {
        if iface_idx >= self.iface_count { return; }
        self.interfaces[iface_idx].link_up = up;
        eth::set_link_state(iface_idx, up);
    }

    /// Check if any interface is configured with an IP.
    pub fn configured(&self) -> bool {
        self.interfaces[..self.iface_count].iter().any(|i| i.configured())
    }

    /// Get source IP for a destination (via routing table lookup).
    pub fn source_ip_for(&self, dst: &Ipv4Addr) -> Option<Ipv4Addr> {
        let (_, iface_idx) = self.routing.lookup(dst)?;
        let iface = &self.interfaces[iface_idx];
        if iface.configured() { Some(iface.ip) } else { None }
    }

    /// Configure an interface with IP/prefix. Auto-adds connected route.
    pub fn configure_iface(&mut self, iface_idx: usize, ip: Ipv4Addr, prefix_len: u8) {
        if iface_idx >= self.iface_count { return; }

        // Remove old connected route if interface was previously configured
        if self.interfaces[iface_idx].configured() {
            let old_net = self.interfaces[iface_idx].ip.network_addr(self.interfaces[iface_idx].prefix_len);
            self.routing.del_route(old_net, self.interfaces[iface_idx].prefix_len);
        }

        self.interfaces[iface_idx].ip = ip;
        self.interfaces[iface_idx].prefix_len = prefix_len;

        // Auto-add connected route
        let net_addr = ip.network_addr(prefix_len);
        let _ = self.routing.add_route(net_addr, prefix_len, Ipv4Addr::ZERO, iface_idx);

        // Send gratuitous ARP
        let mac = self.interfaces[iface_idx].mac;
        let vtag = self.interfaces[iface_idx].vlan_tag();
        let tx = unsafe { &mut TX_BUF };
        arp::send_arp_on(tx, iface_idx, &mac, &ip, &mac, &ip, ARP_REQUEST, &MacAddr::BROADCAST, vtag.as_ref());
    }

    /// Clear IP configuration from an interface.
    pub fn deconfigure_iface(&mut self, iface_idx: usize) {
        if iface_idx >= self.iface_count { return; }
        let iface = &self.interfaces[iface_idx];
        if iface.configured() {
            let net_addr = iface.ip.network_addr(iface.prefix_len);
            self.routing.del_route(net_addr, iface.prefix_len);
        }
        self.interfaces[iface_idx].ip = Ipv4Addr::ZERO;
        self.interfaces[iface_idx].prefix_len = 0;
    }

    // ===== Frame Reception =====

    pub fn poll_rx(&mut self) {
        loop {
            let rx = unsafe { &mut RX_BUF };
            match eth::recv_frame_any(rx) {
                Some((iface_idx, len)) => {
                    let mut frame_copy = [0u8; MAX_FRAME_SIZE];
                    frame_copy[..len].copy_from_slice(&rx[..len]);
                    self.process_frame_on(iface_idx, &frame_copy[..len]);
                }
                None => break,
            }
        }
    }

    fn process_frame_on(&mut self, iface_idx: usize, frame: &[u8]) {
        if iface_idx >= self.iface_count { return; }

        let (eth_hdr, payload) = match EthHeader::parse(frame) {
            Some(v) => v,
            None => return,
        };

        let iface = &self.interfaces[iface_idx];
        if !iface.link_up { return; }

        // MAC filter: only this interface's MAC or broadcast
        if eth_hdr.dst != iface.mac && eth_hdr.dst != MacAddr::BROADCAST {
            return;
        }

        // VLAN filter
        match (iface.vlan, &eth_hdr.vlan_tag) {
            (None, Some(_)) => return,
            (Some(_), None) => return,
            (Some(our_vid), Some(tag)) if tag.vid != our_vid => return,
            _ => {}
        }

        match eth_hdr.ethertype {
            ETHERTYPE_ARP => self.handle_arp(iface_idx, payload),
            ETHERTYPE_IPV4 => self.handle_ipv4(iface_idx, payload),
            _ => {}
        }
    }

    fn handle_arp(&mut self, iface_idx: usize, data: &[u8]) {
        let pkt = match ArpPacket::parse(data) {
            Some(p) => p,
            None => return,
        };

        // Learn from ARP on this interface's ARP table
        self.interfaces[iface_idx].arp_table.insert(pkt.sender_ip, pkt.sender_mac, self.now_ms);

        match pkt.operation {
            ARP_REQUEST => {
                // Check if any of our interfaces has this IP
                let iface = &self.interfaces[iface_idx];
                if iface.configured() && pkt.target_ip == iface.ip {
                    let mac = iface.mac;
                    let ip = iface.ip;
                    let vtag = iface.vlan_tag();
                    let tx = unsafe { &mut TX_BUF };
                    arp::send_arp_on(tx, iface_idx, &mac, &ip, &pkt.sender_mac, &pkt.sender_ip, ARP_REPLY, &pkt.sender_mac, vtag.as_ref());
                }
            }
            ARP_REPLY => {} // Already learned above
            _ => {}
        }
    }

    fn handle_ipv4(&mut self, _iface_idx: usize, data: &[u8]) {
        let (ip_hdr, payload) = match Ipv4Header::parse(data) {
            Some(v) => v,
            None => return,
        };

        // Check if destination matches ANY of our configured interface IPs
        let is_for_us = ip_hdr.dst == Ipv4Addr::BROADCAST ||
            self.interfaces[..self.iface_count].iter().any(|i| i.configured() && i.ip == ip_hdr.dst);

        if !is_for_us { return; }

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
                let pbuf = unsafe { &mut PAYLOAD_BUF };
                let icmp_len = IcmpPacket::serialize_echo(pbuf, ICMP_ECHO_REPLY, pkt.id, pkt.seq, payload);
                if icmp_len > 0 {
                    let _ = self.send_ipv4(ip_hdr.src, PROTO_ICMP, &pbuf[..icmp_len]);
                }
            }
            ICMP_ECHO_REPLY => {
                if pkt.id == self.ping_id {
                    let rtt = (self.now_ms - self.ping_sent_ms) as u32;
                    self.ping_reply_rtt = Some(rtt);
                }
                // Buffer raw ICMP reply for IPC raw sockets
                if self.raw_icmp_reply_count < self.raw_icmp_replies.len() {
                    let idx = self.raw_icmp_reply_count;
                    self.raw_icmp_replies[idx].0 = ip_hdr.src;
                    let copy_len = data.len().min(128);
                    self.raw_icmp_replies[idx].1[..copy_len].copy_from_slice(&data[..copy_len]);
                    self.raw_icmp_replies[idx].2 = copy_len;
                    self.raw_icmp_reply_count += 1;
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
        let src = SocketAddr { ip: ip_hdr.src, port: udp_hdr.src_port };
        self.udp_sockets.deliver(udp_hdr.dst_port, src, payload);
    }

    fn handle_tcp(&mut self, data: &[u8], ip_hdr: &Ipv4Header) {
        let (tcp_hdr, payload) = match TcpHeader::parse(data) {
            Some(v) => v,
            None => return,
        };

        // Use destination IP (which is our IP) as local_ip for TCP
        let actions = self.tcp_connections.process_segment(
            ip_hdr.dst,
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

    /// Send an IPv4 packet. Uses routing table to determine egress interface.
    pub fn send_ipv4(&mut self, dst_ip: Ipv4Addr, protocol: u8, payload: &[u8]) -> Result<(), NetError> {
        let (next_hop, iface_idx) = self.routing.lookup(&dst_ip).ok_or(NetError::NoRoute)?;

        let iface = &self.interfaces[iface_idx];
        if !iface.configured() || !iface.link_up {
            return Err(NetError::NotConfigured);
        }

        let src_ip = iface.ip;
        let src_mac = iface.mac;
        let vtag = iface.vlan_tag();

        // Determine actual next hop (ZERO means on-link = dst itself)
        let actual_next_hop = if next_hop == Ipv4Addr::ZERO { dst_ip } else { next_hop };

        let dst_mac = if dst_ip == Ipv4Addr::BROADCAST {
            MacAddr::BROADCAST
        } else {
            match self.interfaces[iface_idx].arp_table.lookup(&actual_next_hop, self.now_ms) {
                Some(mac) => mac,
                None => {
                    // Send ARP request on this interface
                    let tx = unsafe { &mut TX_BUF };
                    arp::send_arp_on(tx, iface_idx, &src_mac, &src_ip, &MacAddr::ZERO, &actual_next_hop, ARP_REQUEST, &MacAddr::BROADCAST, vtag.as_ref());
                    return Err(NetError::WouldBlock);
                }
            }
        };

        self.send_ipv4_on(iface_idx, src_ip, dst_ip, dst_mac, protocol, payload, vtag)
    }

    fn send_ipv4_on(
        &mut self,
        iface_idx: usize,
        src_ip: Ipv4Addr,
        dst_ip: Ipv4Addr,
        dst_mac: MacAddr,
        protocol: u8,
        payload: &[u8],
        vtag: Option<VlanTag>,
    ) -> Result<(), NetError> {
        let src_mac = self.interfaces[iface_idx].mac;
        let tx = unsafe { &mut TX_BUF };
        let eth_hdr = EthHeader {
            dst: dst_mac,
            src: src_mac,
            vlan_tag: vtag,
            ethertype: ETHERTYPE_IPV4,
        };
        let hdr_size = eth_hdr.header_size();
        let ip_total = Ipv4Header::SIZE + payload.len();
        let frame_len = hdr_size + ip_total;
        if frame_len > MAX_FRAME_SIZE {
            return Err(NetError::BufferFull);
        }

        eth_hdr.write(&mut tx[..hdr_size]);

        let ip_hdr = Ipv4Header::new_outgoing(src_ip, dst_ip, protocol, payload.len(), self.next_ip_id());
        ip_hdr.serialize(&mut tx[hdr_size..hdr_size + Ipv4Header::SIZE]);

        let payload_start = hdr_size + Ipv4Header::SIZE;
        tx[payload_start..payload_start + payload.len()].copy_from_slice(payload);

        eth::send_frame_on(iface_idx, &tx[..frame_len]);
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
                let mut tcp_payload = [0u8; 1460];
                let actual_payload_len = if payload_len > 0 && flags & tcp::ACK != 0 {
                    self.tcp_connections.connections[conn_idx].tx_unsent(&mut tcp_payload)
                } else {
                    0
                };

                let seg_len = TcpHeader::serialize(
                    pbuf, local.port, remote.port, seq, ack, flags, window,
                    &tcp_payload[..actual_payload_len], &local.ip, &remote.ip,
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

    pub fn poll_timers(&mut self) {
        self.now_ms = current_time_ms();

        // Evict expired ARP on all interfaces
        for i in 0..self.iface_count {
            self.interfaces[i].arp_table.evict_expired(self.now_ms);
        }

        let actions = self.tcp_connections.poll_timers(self.now_ms);
        for action in actions {
            self.execute_tcp_action(action);
        }
        self.tcp_flush_all();
    }

    fn tcp_flush_all(&mut self) {
        for i in 0..tcp::MAX_TCP_CONNECTIONS {
            let c = &self.tcp_connections.connections[i];
            if !c.active || c.state != TcpState::Established { continue; }
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

    /// Fast-first ARP retry schedule (ms) used by every blocking send path.
    /// Mirrors the schedule that the WASI raw-ICMP IPC handler in
    /// `net_ipc_handler.rs` uses. Centralised here so any direct in-kernel
    /// caller (e.g. switch-os via `KernelNetTools::icmp_echo`) gets the same
    /// fast behaviour as the WASI ping subprocess.
    const SEND_ARP_RETRY_DELAYS_MS: [u32; 4] = [5, 10, 20, 40];

    /// Send an IPv4 packet, retrying on `WouldBlock` (ARP not yet resolved)
    /// using the fast schedule above. Each retry sleeps for the next delay,
    /// re-syncs `now_ms`, and pumps `poll_rx` so incoming ARP replies get
    /// installed before the next send attempt. Returns the same errors as
    /// `send_ipv4`, plus `NetError::ArpTimeout` if all retries are exhausted.
    pub fn send_ipv4_with_retry(
        &mut self,
        target: Ipv4Addr,
        proto: u8,
        data: &[u8],
    ) -> Result<(), NetError> {
        let mut retry_idx = 0usize;
        loop {
            match self.send_ipv4(target, proto, data) {
                Ok(()) => return Ok(()),
                Err(NetError::WouldBlock) => {
                    if retry_idx >= Self::SEND_ARP_RETRY_DELAYS_MS.len() {
                        return Err(NetError::ArpTimeout);
                    }
                    let delay = Self::SEND_ARP_RETRY_DELAYS_MS[retry_idx];
                    retry_idx += 1;
                    host_sleep_ms(delay);
                    self.now_ms = current_time_ms();
                    self.poll_rx();
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub fn ping(&mut self, target: Ipv4Addr, timeout_ms: u32) -> Result<u32, NetError> {
        if !self.configured() {
            return Err(NetError::NotConfigured);
        }

        self.ping_seq = self.ping_seq.wrapping_add(1);
        self.ping_reply_rtt = None;
        self.now_ms = current_time_ms();
        self.ping_sent_ms = self.now_ms;

        let pbuf = unsafe { &mut PAYLOAD_BUF };
        let ping_data = [0u8; 32];
        let icmp_len = IcmpPacket::serialize_echo(pbuf, ICMP_ECHO_REQUEST, self.ping_id, self.ping_seq, &ping_data);

        // Fast-first ARP retry — was a hard-coded 3×100ms loop, now matches
        // the WASI raw-ICMP IPC path's [5, 10, 20, 40]ms schedule so an
        // in-kernel caller (switch-os) sees the same first-ping latency as
        // the WASI ping CLI.
        self.send_ipv4_with_retry(target, PROTO_ICMP, &pbuf[..icmp_len])?;

        // Re-stamp `ping_sent_ms` AFTER the ARP retry loop so the reported
        // RTT measures the actual flight time of this packet, not the
        // ARP-resolution stall that preceded it. (Pre-fix this was stamped
        // before the loop, so first-ping RTTs were polluted by ARP wait.)
        self.now_ms = current_time_ms();
        self.ping_sent_ms = self.now_ms;

        // Reply poll: was a flat host_sleep_ms(10) — minimum measurable RTT
        // 10ms even on a local cable. Use the same 1→2→4→8 ms exponential
        // backoff the WASI raw-ICMP recv path uses.
        let deadline = self.now_ms + timeout_ms as i64;
        let mut sleep_ms = 1u32;
        while self.now_ms < deadline {
            host_sleep_ms(sleep_ms);
            sleep_ms = (sleep_ms.saturating_mul(2)).min(8);
            self.now_ms = current_time_ms();
            self.poll_rx();
            if let Some(rtt) = self.ping_reply_rtt {
                return Ok(rtt);
            }
        }
        Err(NetError::TimedOut)
    }

    pub fn arp_resolve(&mut self, ip: Ipv4Addr, timeout_ms: u32) -> Result<MacAddr, NetError> {
        // Find which interface to ARP on via routing
        let (_, iface_idx) = self.routing.lookup(&ip).ok_or(NetError::NoRoute)?;

        if let Some(mac) = self.interfaces[iface_idx].arp_table.lookup(&ip, self.now_ms) {
            return Ok(mac);
        }

        let src_mac = self.interfaces[iface_idx].mac;
        let src_ip = self.interfaces[iface_idx].ip;
        let vtag = self.interfaces[iface_idx].vlan_tag();
        let mut attempts = 0u32;
        let max_attempts = 3u32;
        let attempt_interval = timeout_ms / max_attempts;

        while attempts < max_attempts {
            let tx = unsafe { &mut TX_BUF };
            arp::send_arp_on(tx, iface_idx, &src_mac, &src_ip, &MacAddr::ZERO, &ip, ARP_REQUEST, &MacAddr::BROADCAST, vtag.as_ref());
            attempts += 1;

            let deadline = current_time_ms() + attempt_interval as i64;
            while current_time_ms() < deadline {
                host_sleep_ms(10);
                self.now_ms = current_time_ms();
                self.poll_rx();
                if let Some(mac) = self.interfaces[iface_idx].arp_table.lookup(&ip, self.now_ms) {
                    return Ok(mac);
                }
            }
        }
        Err(NetError::ArpTimeout)
    }

    pub fn udp_send(&mut self, sock_idx: usize, dst: SocketAddr, data: &[u8]) -> Result<(), NetError> {
        // Determine source IP via routing
        let src_ip = self.source_ip_for(&dst.ip).ok_or(NetError::NoRoute)?;
        let src_port = self.udp_sockets.sockets[sock_idx].local_port;
        let pbuf = unsafe { &mut PAYLOAD_BUF };
        let udp_len = UdpHeader::serialize(pbuf, src_port, dst.port, data, &src_ip, &dst.ip);

        // Was a hard-coded 3×100ms ARP retry loop. Now uses the same fast
        // schedule as `ping` and the WASI raw-ICMP IPC path so DNS lookups
        // and any other in-kernel UDP send (notably switch-os via
        // `KernelNetTools::getaddrinfo → dns_resolve → udp_send`) don't
        // stall on ARP.
        self.send_ipv4_with_retry(dst.ip, PROTO_UDP, &pbuf[..udp_len])
    }

    pub fn dns_resolve(&mut self, name: &str, timeout_ms: u32) -> Result<Ipv4Addr, NetError> {
        if !self.configured() || self.dns_server == Ipv4Addr::ZERO {
            return Err(NetError::NotConfigured);
        }

        let (sock_idx, _src_port) = self.udp_sockets.bind_ephemeral()?;
        let mut query_buf = [0u8; 512];
        let tx_id = (self.now_ms & 0xFFFF) as u16;
        let query_len = dns::build_query(name, tx_id, &mut query_buf);
        if query_len == 0 {
            self.udp_sockets.close(sock_idx);
            return Err(NetError::InvalidPacket);
        }

        let dst = SocketAddr { ip: self.dns_server, port: dns::dns_port() };
        self.udp_send(sock_idx, dst, &query_buf[..query_len])?;

        // Reply poll: was a flat host_sleep_ms(10) — now uses the same
        // 1→2→4→8 ms exponential backoff as `ping` and the WASI raw-ICMP
        // recv path. Brings switch-os DNS resolution down from "10ms minimum
        // per poll" to "1ms minimum, capped at 8ms".
        let deadline = current_time_ms() + timeout_ms as i64;
        let mut recv_buf = [0u8; 512];
        let mut sleep_ms = 1u32;
        loop {
            self.now_ms = current_time_ms();
            if self.now_ms >= deadline { break; }
            if let Some((_src, len)) = self.udp_sockets.recv(sock_idx, &mut recv_buf) {
                if let Some(ip) = dns::parse_response(&recv_buf[..len]) {
                    self.udp_sockets.close(sock_idx);
                    return Ok(ip);
                }
            }
            host_sleep_ms(sleep_ms);
            sleep_ms = (sleep_ms.saturating_mul(2)).min(8);
            self.poll_rx();
        }
        self.udp_sockets.close(sock_idx);
        Err(NetError::TimedOut)
    }

    // ===== TCP High-Level API =====

    pub fn tcp_connect(&mut self, remote: SocketAddr, timeout_ms: u32) -> Result<usize, NetError> {
        let (next_hop, _) = self.routing.lookup(&remote.ip).ok_or(NetError::NoRoute)?;
        let actual_hop = if next_hop == Ipv4Addr::ZERO { remote.ip } else { next_hop };
        self.arp_resolve(actual_hop, 1500)?;

        let src_ip = self.source_ip_for(&remote.ip).ok_or(NetError::NoRoute)?;
        let idx = self.tcp_connections.connect(src_ip, remote, self.now_ms)?;

        let c = &self.tcp_connections.connections[idx];
        let action = TcpAction::SendSegment {
            conn_idx: idx, flags: tcp::SYN, seq: c.iss, ack: 0, payload_len: 0,
        };
        self.execute_tcp_action(action);

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
            match self.tcp_connections.connections[idx].state {
                TcpState::Established => return Ok(idx),
                TcpState::Closed => return Err(NetError::ConnectionRefused),
                _ => {}
            }
            host_sleep_ms(10);
        }
    }

    pub fn tcp_accept(&mut self, listener_idx: usize, timeout_ms: u32) -> Result<usize, NetError> {
        let port = self.tcp_connections.connections[listener_idx].local.port;
        let deadline = current_time_ms() + timeout_ms as i64;
        loop {
            self.now_ms = current_time_ms();
            if self.now_ms >= deadline { return Err(NetError::TimedOut); }
            if let Some(idx) = self.tcp_connections.find_established_from_listener(port) {
                self.tcp_connections.connections[idx].listener_port = 0;
                return Ok(idx);
            }
            self.poll_rx();
            self.poll_timers();
            host_sleep_ms(10);
        }
    }

    pub fn tcp_send(&mut self, idx: usize, data: &[u8]) -> Result<usize, NetError> {
        let c = &mut self.tcp_connections.connections[idx];
        if !c.active || c.state != TcpState::Established {
            return Err(NetError::NotConnected);
        }
        let written = c.write(data);
        self.tcp_flush_all();
        Ok(written)
    }

    pub fn tcp_recv(&mut self, idx: usize, buf: &mut [u8], timeout_ms: u32) -> Result<usize, NetError> {
        let deadline = current_time_ms() + timeout_ms as i64;
        loop {
            let c = &mut self.tcp_connections.connections[idx];
            if !c.active { return Err(NetError::NotConnected); }
            if c.rx_available() > 0 { return Ok(c.read(buf)); }
            if c.state == TcpState::CloseWait || c.state == TcpState::Closed { return Ok(0); }
            self.now_ms = current_time_ms();
            if self.now_ms >= deadline { return Err(NetError::TimedOut); }
            self.poll_rx();
            self.poll_timers();
            host_sleep_ms(10);
        }
    }

    pub fn tcp_close(&mut self, idx: usize) {
        if let Some(action) = self.tcp_connections.close(idx, self.now_ms) {
            self.execute_tcp_action(action);
        }
    }

    /// Immediately close a TCP connection, skipping FIN/TIME_WAIT.
    /// Used by sshd to free the connection slot so the listener can
    /// accept new connections without waiting for the close sequence.
    pub fn tcp_close_immediate(&mut self, idx: usize) {
        if idx < self.tcp_connections.connections.len() {
            let c = &mut self.tcp_connections.connections[idx];
            if c.active {
                c.state = tcp::TcpState::Closed;
                c.active = false;
            }
        }
    }
}

pub fn current_time_ms() -> i64 {
    unsafe { get_time_ms() }
}

/// Format interface name: "eth0", "eth1", etc.
fn format_iface_name(index: usize) -> alloc::string::String {
    alloc::format!("eth{}", index)
}
