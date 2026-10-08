//! Control of the kernel's DHCP client over netlink (`RTM_ECM_DHCP`), used
//! by `dhclient` and `ifconfig <iface> dhcp`.

use alloc::vec::Vec;

use crate::netlink::*;
use crate::socket::{self, SockAddrIn, AF_NETLINK, NETLINK_ROUTE, SOCK_DGRAM};

/// What `dhclient -s` shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DhcpInfo {
    /// One of `DHCP_STATE_*` (see [`dhcp_state_name`]).
    pub state: u32,
    pub address: Option<[u8; 4]>,
    pub prefix: u8,
    pub router: Option<[u8; 4]>,
    pub dns: Option<[u8; 4]>,
    pub server: Option<[u8; 4]>,
    /// Seconds until the lease expires / T1 / T2.
    pub expires_in: u32,
    pub renew_in: u32,
    pub rebind_in: u32,
}

/// Netlink errors (negative errno) or local failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DhcpError {
    /// The interface doesn't exist.
    NoSuchInterface,
    /// RELEASE/STOP with no client running.
    NotRunning,
    /// Socket failure or an unexpected reply.
    Io,
    Errno(i32),
}

struct Nl(i32);

impl Nl {
    fn open() -> Result<Self, DhcpError> {
        let fd = socket::socket(AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE);
        if fd < 0 {
            Err(DhcpError::Io)
        } else {
            Ok(Nl(fd))
        }
    }

    /// Send one request, collect the reply (until DONE / ERROR / a single
    /// non-multipart message).
    fn request(&self, req: &[u8]) -> Vec<u8> {
        socket::sendto(self.0, req, 0, &SockAddrIn::default());
        let mut out = Vec::new();
        let mut buf = [0u8; 2048];
        loop {
            let mut from = SockAddrIn::default();
            let n = socket::recvfrom(self.0, &mut buf, 0, &mut from);
            if n <= 0 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
            if let Some(h) = NlMsgHdr::parse(&out) {
                if h.nlmsg_flags & NLM_F_MULTI == 0 || h.nlmsg_type == NLMSG_DONE {
                    break;
                }
            }
            if contains_done(&out) {
                break;
            }
        }
        out
    }
}

impl Drop for Nl {
    fn drop(&mut self) {
        socket::close(self.0);
    }
}

fn contains_done(buf: &[u8]) -> bool {
    let mut off = 0;
    while let Some(h) = buf.get(off..).and_then(NlMsgHdr::parse) {
        if h.nlmsg_type == NLMSG_DONE {
            return true;
        }
        let len = nlmsg_align(h.nlmsg_len as usize);
        if len == 0 {
            break;
        }
        off += len;
    }
    false
}

/// 1-based netlink index of interface `name`.
pub fn ifindex(name: &str) -> Option<i32> {
    let nl = Nl::open().ok()?;
    let mut req = [0u8; NLMSG_HDR_SIZE + IFINFOMSG_SIZE];
    NlMsgHdr {
        nlmsg_len: req.len() as u32,
        nlmsg_type: RTM_GETLINK,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_DUMP,
        nlmsg_seq: 1,
        nlmsg_pid: 0,
    }
    .serialize(&mut req);
    let links = nl.request(&req);
    let mut off = 0;
    while let Some(h) = links.get(off..).and_then(NlMsgHdr::parse) {
        if h.nlmsg_type == NLMSG_DONE || h.nlmsg_len < NLMSG_HDR_SIZE as u32 {
            break;
        }
        let end = (off + h.nlmsg_len as usize).min(links.len());
        if h.nlmsg_type == RTM_NEWLINK {
            let payload = &links[off + NLMSG_HDR_SIZE..end];
            if let Some(info) = IfInfoMsg::parse(payload) {
                let mut a = IFINFOMSG_SIZE;
                while let Some((kind, data, next)) = parse_attr(payload, a) {
                    if kind == IFLA_IFNAME {
                        let n = data.iter().position(|&b| b == 0).unwrap_or(data.len());
                        if &data[..n] == name.as_bytes() {
                            return Some(info.ifi_index);
                        }
                    }
                    if next <= a {
                        break;
                    }
                    a = next;
                }
            }
        }
        off += nlmsg_align(h.nlmsg_len as usize);
    }
    None
}

fn dhcp_request(index: i32, op: u32) -> Vec<u8> {
    let mut req = alloc::vec![0u8; 64];
    let mut off = NLMSG_HDR_SIZE;
    off += IfInfoMsg {
        ifi_family: 0,
        _pad: 0,
        ifi_type: 0,
        ifi_index: index,
        ifi_flags: 0,
        ifi_change: 0,
    }
    .serialize(&mut req[off..]);
    off += write_attr_u32(&mut req, off, IFLA_ECM_DHCP_OP, op);
    NlMsgHdr {
        nlmsg_len: off as u32,
        nlmsg_type: RTM_ECM_DHCP,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_ACK,
        nlmsg_seq: 1,
        nlmsg_pid: 0,
    }
    .serialize(&mut req);
    req.truncate(off);
    req
}

fn control(ifname: &str, op: u32) -> Result<(), DhcpError> {
    let idx = ifindex(ifname).ok_or(DhcpError::NoSuchInterface)?;
    let nl = Nl::open()?;
    let r = nl.request(&dhcp_request(idx, op));
    let h = NlMsgHdr::parse(&r).ok_or(DhcpError::Io)?;
    if h.nlmsg_type != NLMSG_ERROR || r.len() < NLMSG_HDR_SIZE + 4 {
        return Err(DhcpError::Io);
    }
    let e = i32::from_le_bytes([r[16], r[17], r[18], r[19]]);
    match e {
        0 => Ok(()),
        -19 => Err(DhcpError::NoSuchInterface),
        -3 => Err(DhcpError::NotRunning),
        e => Err(DhcpError::Errno(e)),
    }
}

/// Start the kernel DHCP client on `ifname` (kept if already running).
pub fn start(ifname: &str) -> Result<(), DhcpError> {
    control(ifname, DHCP_OP_START)
}

/// Send DHCPRELEASE, stop the client and drop the leased address.
pub fn release(ifname: &str) -> Result<(), DhcpError> {
    control(ifname, DHCP_OP_RELEASE)
}

/// Stop the client, keeping the address until it expires.
pub fn stop(ifname: &str) -> Result<(), DhcpError> {
    control(ifname, DHCP_OP_STOP)
}

/// The client's state and lease on `ifname`.
pub fn status(ifname: &str) -> Result<DhcpInfo, DhcpError> {
    let idx = ifindex(ifname).ok_or(DhcpError::NoSuchInterface)?;
    let nl = Nl::open()?;
    let r = nl.request(&dhcp_request(idx, DHCP_OP_STATUS));
    parse_status(&r)
}

/// Decode an `RTM_ECM_DHCP` status reply.
pub fn parse_status(r: &[u8]) -> Result<DhcpInfo, DhcpError> {
    let h = NlMsgHdr::parse(r).ok_or(DhcpError::Io)?;
    if h.nlmsg_type == NLMSG_ERROR && r.len() >= NLMSG_HDR_SIZE + 4 {
        return Err(match i32::from_le_bytes([r[16], r[17], r[18], r[19]]) {
            -19 => DhcpError::NoSuchInterface,
            e => DhcpError::Errno(e),
        });
    }
    if h.nlmsg_type != RTM_ECM_DHCP {
        return Err(DhcpError::Io);
    }
    let end = (h.nlmsg_len as usize).min(r.len());
    let payload = r.get(NLMSG_HDR_SIZE..end).ok_or(DhcpError::Io)?;
    let mut info = DhcpInfo::default();
    let mut a = IFINFOMSG_SIZE;
    while let Some((kind, d, next)) = parse_attr(payload, a) {
        let ip = (d.len() >= 4).then(|| [d[0], d[1], d[2], d[3]]);
        let v = ip.map(u32::from_le_bytes).unwrap_or(0);
        match kind {
            DHCPA_STATE => info.state = v,
            DHCPA_ADDRESS => info.address = ip,
            DHCPA_PREFIX => info.prefix = v as u8,
            DHCPA_ROUTER => info.router = ip,
            DHCPA_DNS => info.dns = ip,
            DHCPA_SERVER => info.server = ip,
            DHCPA_EXPIRES_IN => info.expires_in = v,
            DHCPA_RENEW_IN => info.renew_in = v,
            DHCPA_REBIND_IN => info.rebind_in = v,
            _ => {}
        }
        if next <= a {
            break;
        }
        a = next;
    }
    Ok(info)
}
