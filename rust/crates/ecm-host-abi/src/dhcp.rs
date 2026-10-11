//! Control of the kernel's DHCP client over netlink (`RTM_ECM_DHCP`), used
//! by `dhclient` and `ifconfig <iface> dhcp`.

use alloc::string::String;
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
        if len < NLMSG_HDR_SIZE {
            break;
        }
        off += len;
    }
    false
}

/// An interface as the kernel reports it (link + IPv4 address dumps).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkInfo {
    /// 1-based netlink index.
    pub index: i32,
    pub name: String,
    pub mac: [u8; 6],
    /// Administratively up and carrier present.
    pub running: bool,
    pub ip: Option<[u8; 4]>,
    pub prefix: u8,
}

fn dump(nl: &Nl, kind: u16, body: usize) -> Vec<u8> {
    let mut req = alloc::vec![0u8; NLMSG_HDR_SIZE + body];
    NlMsgHdr {
        nlmsg_len: req.len() as u32,
        nlmsg_type: kind,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_DUMP,
        nlmsg_seq: 1,
        nlmsg_pid: 0,
    }
    .serialize(&mut req);
    nl.request(&req)
}

/// Each message of a dump: (type, payload).
fn messages(buf: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    let mut off = 0;
    while let Some(h) = buf.get(off..).and_then(NlMsgHdr::parse) {
        if h.nlmsg_type == NLMSG_DONE || (h.nlmsg_len as usize) < NLMSG_HDR_SIZE {
            break;
        }
        let end = (off + h.nlmsg_len as usize).min(buf.len());
        out.push((h.nlmsg_type, &buf[off + NLMSG_HDR_SIZE..end]));
        off += nlmsg_align(h.nlmsg_len as usize);
    }
    out
}

fn attrs(payload: &[u8], start: usize) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    let mut a = start;
    while let Some((kind, data, next)) = parse_attr(payload, a) {
        out.push((kind, data));
        if next <= a || out.len() > 64 {
            break;
        }
        a = next;
    }
    out
}

/// Every interface with its MAC and IPv4 address.
pub fn links() -> Vec<LinkInfo> {
    let Ok(nl) = Nl::open() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (kind, payload) in messages(&dump(&nl, RTM_GETLINK, IFINFOMSG_SIZE)) {
        if kind != RTM_NEWLINK {
            continue;
        }
        let Some(info) = IfInfoMsg::parse(payload) else { continue };
        let mut l = LinkInfo {
            index: info.ifi_index,
            running: info.ifi_flags & IFF_RUNNING != 0,
            ..LinkInfo::default()
        };
        for (k, d) in attrs(payload, IFINFOMSG_SIZE) {
            match k {
                IFLA_IFNAME => {
                    let n = d.iter().position(|&b| b == 0).unwrap_or(d.len());
                    l.name = String::from(core::str::from_utf8(&d[..n]).unwrap_or(""));
                }
                IFLA_ADDRESS if d.len() >= 6 => l.mac.copy_from_slice(&d[..6]),
                _ => {}
            }
        }
        out.push(l);
    }
    for (kind, payload) in messages(&dump(&nl, RTM_GETADDR, IFADDRMSG_SIZE)) {
        if kind != RTM_NEWADDR {
            continue;
        }
        let Some(ifa) = IfAddrMsg::parse(payload) else { continue };
        let Some(l) = out.iter_mut().find(|l| l.index as u32 == ifa.ifa_index) else {
            continue;
        };
        for (k, d) in attrs(payload, IFADDRMSG_SIZE) {
            if (k == IFA_LOCAL || k == IFA_ADDRESS) && d.len() >= 4 {
                l.ip = Some([d[0], d[1], d[2], d[3]]);
                l.prefix = ifa.ifa_prefixlen;
            }
        }
    }
    out
}

/// The interface called `name`.
pub fn link(name: &str) -> Option<LinkInfo> {
    links().into_iter().find(|l| l.name == name)
}

/// 1-based netlink index of interface `name`.
pub fn ifindex(name: &str) -> Option<i32> {
    link(name).map(|l| l.index)
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
