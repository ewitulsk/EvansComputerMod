//! Netlink protocol constants and structures for route management.
//!
//! Used by both the kernel (to parse/build netlink messages) and WASI programs
//! (to construct requests and parse responses). Matches Linux netlink definitions.

#![allow(dead_code)]

// --- Message types (rtnetlink) ---
pub const RTM_NEWLINK: u16 = 16;
pub const RTM_DELLINK: u16 = 17;
pub const RTM_GETLINK: u16 = 18;
pub const RTM_NEWADDR: u16 = 20;
pub const RTM_DELADDR: u16 = 21;
pub const RTM_GETADDR: u16 = 22;
pub const RTM_NEWROUTE: u16 = 24;
pub const RTM_DELROUTE: u16 = 25;
pub const RTM_GETROUTE: u16 = 26;
pub const NLMSG_DONE: u16 = 3;
pub const NLMSG_ERROR: u16 = 2;

// --- Message flags ---
pub const NLM_F_REQUEST: u16 = 0x0001;
pub const NLM_F_MULTI: u16 = 0x0002;
pub const NLM_F_ACK: u16 = 0x0004;
pub const NLM_F_DUMP: u16 = 0x0300; // NLM_F_ROOT | NLM_F_MATCH
pub const NLM_F_CREATE: u16 = 0x0400;
pub const NLM_F_EXCL: u16 = 0x0200;

// --- Interface flags ---
pub const IFF_UP: u32 = 0x1;
pub const IFF_RUNNING: u32 = 0x40;

// --- Interface info attributes ---
pub const IFLA_UNSPEC: u16 = 0;
pub const IFLA_ADDRESS: u16 = 1;   // MAC address
pub const IFLA_IFNAME: u16 = 3;    // Interface name
pub const IFLA_MTU: u16 = 4;       // MTU
pub const IFLA_LINK: u16 = 5;

// --- Address attributes ---
pub const IFA_UNSPEC: u16 = 0;
pub const IFA_ADDRESS: u16 = 1;    // Interface address
pub const IFA_LOCAL: u16 = 2;      // Local address
pub const IFA_LABEL: u16 = 3;      // Interface name

// --- Route attributes ---
pub const RTA_UNSPEC: u16 = 0;
pub const RTA_DST: u16 = 1;        // Destination address
pub const RTA_GATEWAY: u16 = 5;    // Gateway address
pub const RTA_OIF: u16 = 4;        // Output interface index

// --- Address families ---
pub const AF_INET: u8 = 2;

// --- Route types ---
pub const RTN_UNICAST: u8 = 1;

// --- Route protocols ---
pub const RTPROT_STATIC: u8 = 4;

// --- Route scope ---
pub const RT_SCOPE_UNIVERSE: u8 = 0;
pub const RT_SCOPE_LINK: u8 = 253;

// --- Route table ---
pub const RT_TABLE_MAIN: u8 = 254;

// --- Netlink message header (16 bytes) ---
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NlMsgHdr {
    pub nlmsg_len: u32,
    pub nlmsg_type: u16,
    pub nlmsg_flags: u16,
    pub nlmsg_seq: u32,
    pub nlmsg_pid: u32,
}

pub const NLMSG_HDR_SIZE: usize = 16;

impl NlMsgHdr {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < NLMSG_HDR_SIZE { return None; }
        Some(Self {
            nlmsg_len: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            nlmsg_type: u16::from_le_bytes([data[4], data[5]]),
            nlmsg_flags: u16::from_le_bytes([data[6], data[7]]),
            nlmsg_seq: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
            nlmsg_pid: u32::from_le_bytes([data[12], data[13], data[14], data[15]]),
        })
    }

    pub fn serialize(&self, buf: &mut [u8]) -> usize {
        if buf.len() < NLMSG_HDR_SIZE { return 0; }
        buf[0..4].copy_from_slice(&self.nlmsg_len.to_le_bytes());
        buf[4..6].copy_from_slice(&self.nlmsg_type.to_le_bytes());
        buf[6..8].copy_from_slice(&self.nlmsg_flags.to_le_bytes());
        buf[8..12].copy_from_slice(&self.nlmsg_seq.to_le_bytes());
        buf[12..16].copy_from_slice(&self.nlmsg_pid.to_le_bytes());
        NLMSG_HDR_SIZE
    }
}

// --- ifinfomsg (16 bytes after nlmsghdr) ---
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IfInfoMsg {
    pub ifi_family: u8,
    pub _pad: u8,
    pub ifi_type: u16,     // ARPHRD_ETHER = 1
    pub ifi_index: i32,
    pub ifi_flags: u32,
    pub ifi_change: u32,
}

pub const IFINFOMSG_SIZE: usize = 16;

impl IfInfoMsg {
    pub fn serialize(&self, buf: &mut [u8]) -> usize {
        if buf.len() < IFINFOMSG_SIZE { return 0; }
        buf[0] = self.ifi_family;
        buf[1] = self._pad;
        buf[2..4].copy_from_slice(&self.ifi_type.to_le_bytes());
        buf[4..8].copy_from_slice(&self.ifi_index.to_le_bytes());
        buf[8..12].copy_from_slice(&self.ifi_flags.to_le_bytes());
        buf[12..16].copy_from_slice(&self.ifi_change.to_le_bytes());
        IFINFOMSG_SIZE
    }

    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < IFINFOMSG_SIZE { return None; }
        Some(Self {
            ifi_family: data[0],
            _pad: data[1],
            ifi_type: u16::from_le_bytes([data[2], data[3]]),
            ifi_index: i32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            ifi_flags: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
            ifi_change: u32::from_le_bytes([data[12], data[13], data[14], data[15]]),
        })
    }
}

// --- ifaddrmsg (8 bytes after nlmsghdr) ---
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IfAddrMsg {
    pub ifa_family: u8,
    pub ifa_prefixlen: u8,
    pub ifa_flags: u8,
    pub ifa_scope: u8,
    pub ifa_index: u32,
}

pub const IFADDRMSG_SIZE: usize = 8;

impl IfAddrMsg {
    pub fn serialize(&self, buf: &mut [u8]) -> usize {
        if buf.len() < IFADDRMSG_SIZE { return 0; }
        buf[0] = self.ifa_family;
        buf[1] = self.ifa_prefixlen;
        buf[2] = self.ifa_flags;
        buf[3] = self.ifa_scope;
        buf[4..8].copy_from_slice(&self.ifa_index.to_le_bytes());
        IFADDRMSG_SIZE
    }

    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < IFADDRMSG_SIZE { return None; }
        Some(Self {
            ifa_family: data[0],
            ifa_prefixlen: data[1],
            ifa_flags: data[2],
            ifa_scope: data[3],
            ifa_index: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
        })
    }
}

// --- rtmsg (12 bytes after nlmsghdr) ---
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RtMsg {
    pub rtm_family: u8,
    pub rtm_dst_len: u8,
    pub rtm_src_len: u8,
    pub rtm_tos: u8,
    pub rtm_table: u8,
    pub rtm_protocol: u8,
    pub rtm_scope: u8,
    pub rtm_type: u8,
    pub rtm_flags: u32,
}

pub const RTMSG_SIZE: usize = 12;

impl RtMsg {
    pub fn serialize(&self, buf: &mut [u8]) -> usize {
        if buf.len() < RTMSG_SIZE { return 0; }
        buf[0] = self.rtm_family;
        buf[1] = self.rtm_dst_len;
        buf[2] = self.rtm_src_len;
        buf[3] = self.rtm_tos;
        buf[4] = self.rtm_table;
        buf[5] = self.rtm_protocol;
        buf[6] = self.rtm_scope;
        buf[7] = self.rtm_type;
        buf[8..12].copy_from_slice(&self.rtm_flags.to_le_bytes());
        RTMSG_SIZE
    }

    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < RTMSG_SIZE { return None; }
        Some(Self {
            rtm_family: data[0],
            rtm_dst_len: data[1],
            rtm_src_len: data[2],
            rtm_tos: data[3],
            rtm_table: data[4],
            rtm_protocol: data[5],
            rtm_scope: data[6],
            rtm_type: data[7],
            rtm_flags: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
        })
    }
}

// --- rtattr (4-byte header + variable payload) ---
pub const RTA_HDR_SIZE: usize = 4;

/// Write a netlink attribute: [rta_len: u16, rta_type: u16, payload...]
/// Returns total bytes written (4-byte aligned).
pub fn write_attr(buf: &mut [u8], off: usize, attr_type: u16, payload: &[u8]) -> usize {
    let rta_len = (RTA_HDR_SIZE + payload.len()) as u16;
    let aligned = nlmsg_align(rta_len as usize);
    if buf.len() < off + aligned { return 0; }
    buf[off..off+2].copy_from_slice(&rta_len.to_le_bytes());
    buf[off+2..off+4].copy_from_slice(&attr_type.to_le_bytes());
    buf[off+4..off+4+payload.len()].copy_from_slice(payload);
    // Zero padding
    for i in off+4+payload.len()..off+aligned {
        buf[i] = 0;
    }
    aligned
}

/// Write a u32 attribute.
pub fn write_attr_u32(buf: &mut [u8], off: usize, attr_type: u16, val: u32) -> usize {
    write_attr(buf, off, attr_type, &val.to_le_bytes())
}

/// Parse a netlink attribute from `data` at offset `off`.
/// Returns (attr_type, payload_slice, next_offset).
pub fn parse_attr(data: &[u8], off: usize) -> Option<(u16, &[u8], usize)> {
    if data.len() < off + RTA_HDR_SIZE { return None; }
    let rta_len = u16::from_le_bytes([data[off], data[off+1]]) as usize;
    let rta_type = u16::from_le_bytes([data[off+2], data[off+3]]);
    if rta_len < RTA_HDR_SIZE || data.len() < off + rta_len { return None; }
    let payload = &data[off+4..off+rta_len];
    let next = off + nlmsg_align(rta_len);
    Some((rta_type, payload, next))
}

/// Align to 4-byte boundary (netlink standard).
pub fn nlmsg_align(len: usize) -> usize {
    (len + 3) & !3
}
