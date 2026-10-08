//! DHCPv4 wire format and client, driven entirely by virtual milliseconds.
use crate::{Ipv4Addr, MacAddr};

pub const DISCOVER: u8 = 1;
pub const OFFER: u8 = 2;
pub const REQUEST: u8 = 3;
pub const DECLINE: u8 = 4;
pub const ACK: u8 = 5;
pub const NAK: u8 = 6;
pub const RELEASE: u8 = 7;
pub const INFORM: u8 = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub reply: bool,
    pub xid: u32,
    pub mac: MacAddr,
    pub kind: u8,
    pub ciaddr: Ipv4Addr,
    pub yiaddr: Ipv4Addr,
    pub relay: Ipv4Addr,
    pub server: Option<Ipv4Addr>,
    pub requested: Option<Ipv4Addr>,
    pub mask: Option<Ipv4Addr>,
    pub router: Option<Ipv4Addr>,
    pub dns: Option<Ipv4Addr>,
    pub lease_secs: u32,
    pub t1_secs: u32,
    pub t2_secs: u32,
}

/// The BOOTP broadcast flag (bit 15 of `flags`) of a raw DHCP message.
pub fn broadcast_flag(data: &[u8]) -> bool {
    data.get(10).is_some_and(|b| b & 0x80 != 0)
}

impl Message {
    pub fn new(kind: u8, xid: u32, mac: MacAddr) -> Self {
        Self {
            reply: false,
            xid,
            mac,
            kind,
            ciaddr: Ipv4Addr::ZERO,
            yiaddr: Ipv4Addr::ZERO,
            relay: Ipv4Addr::ZERO,
            server: None,
            requested: None,
            mask: None,
            router: None,
            dns: None,
            lease_secs: 0,
            t1_secs: 0,
            t2_secs: 0,
        }
    }
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 240
            || !matches!(data[0], 1 | 2)
            || data[1..3] != [1, 6]
            || data[236..240] != [99, 130, 83, 99]
        {
            return None;
        }
        let mut mac = [0; 6];
        mac.copy_from_slice(&data[28..34]);
        let mut m = Self::new(
            0,
            u32::from_be_bytes(data[4..8].try_into().ok()?),
            MacAddr(mac),
        );
        m.reply = data[0] == 2;
        m.ciaddr = Ipv4Addr::from_bytes(&data[12..16]);
        m.yiaddr = Ipv4Addr::from_bytes(&data[16..20]);
        m.relay = Ipv4Addr::from_bytes(&data[24..28]);
        let mut i = 240;
        while i < data.len() {
            let tag = data[i];
            i += 1;
            if tag == 255 {
                break;
            }
            if tag == 0 {
                continue;
            }
            let len = *data.get(i)? as usize;
            i += 1;
            let v = data.get(i..i + len)?;
            i += len;
            match (tag, len) {
                (53, 1) => m.kind = v[0],
                (54, 4) => m.server = Some(Ipv4Addr::from_bytes(v)),
                (50, 4) => m.requested = Some(Ipv4Addr::from_bytes(v)),
                (1, 4) => m.mask = Some(Ipv4Addr::from_bytes(v)),
                (3, n) if n >= 4 && n % 4 == 0 => m.router = Some(Ipv4Addr::from_bytes(&v[..4])),
                (6, n) if n >= 4 && n % 4 == 0 => m.dns = Some(Ipv4Addr::from_bytes(&v[..4])),
                (51, 4) => m.lease_secs = u32::from_be_bytes(v.try_into().ok()?),
                (58, 4) => m.t1_secs = u32::from_be_bytes(v.try_into().ok()?),
                (59, 4) => m.t2_secs = u32::from_be_bytes(v.try_into().ok()?),
                _ => {}
            }
        }
        (m.kind != 0).then_some(m)
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![0u8; 240];
        b[0] = if self.reply { 2 } else { 1 };
        b[1] = 1;
        b[2] = 6;
        b[4..8].copy_from_slice(&self.xid.to_be_bytes());
        b[10] = 0x80;
        b[12..16].copy_from_slice(&self.ciaddr.0);
        b[16..20].copy_from_slice(&self.yiaddr.0);
        b[24..28].copy_from_slice(&self.relay.0);
        b[28..34].copy_from_slice(&self.mac.0);
        b[236..240].copy_from_slice(&[99, 130, 83, 99]);
        b.extend_from_slice(&[53, 1, self.kind]);
        for (tag, ip) in [
            (54, self.server),
            (50, self.requested),
            (1, self.mask),
            (3, self.router),
            (6, self.dns),
        ] {
            if let Some(ip) = ip {
                b.extend_from_slice(&[tag, 4]);
                b.extend_from_slice(&ip.0);
            }
        }
        for (tag, v) in [
            (51, self.lease_secs),
            (58, self.t1_secs),
            (59, self.t2_secs),
        ] {
            if v > 0 {
                b.extend_from_slice(&[tag, 4]);
                b.extend_from_slice(&v.to_be_bytes());
            }
        }
        if !self.reply {
            b.extend_from_slice(&[55, 3, 1, 3, 6]);
        }
        b.push(255);
        b.resize(b.len().max(300), 0);
        b
    }
}

#[derive(Clone, Debug)]
pub struct Lease {
    pub address: Ipv4Addr,
    pub prefix: u8,
    pub router: Option<Ipv4Addr>,
    pub dns: Option<Ipv4Addr>,
    pub expires: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// No lease and nothing sent yet (RFC 2131 INIT); the next poll sends
    /// DISCOVER and moves to `Selecting`.
    Init,
    Selecting,
    Requesting,
    Bound,
    Renewing,
    Rebinding,
}

pub struct Client {
    pub state: State,
    pub lease: Option<Lease>,
    pub deadline: i64,
    mac: MacAddr,
    xid: u32,
    server: Option<Ipv4Addr>,
    offered: Option<Ipv4Addr>,
    retry: i64,
    renew: i64,
    rebind: i64,
}
impl Client {
    pub fn new(mac: MacAddr, xid: u32, now: i64) -> Self {
        Self {
            state: State::Init,
            lease: None,
            deadline: now,
            mac,
            xid,
            server: None,
            offered: None,
            retry: 4000,
            renew: 0,
            rebind: 0,
        }
    }
    pub fn poll(&mut self, now: i64) -> Option<(Ipv4Addr, Message)> {
        if now < self.deadline {
            return None;
        }
        if self.lease.as_ref().is_some_and(|l| now >= l.expires) {
            self.lease = None;
            self.state = State::Init;
            self.server = None;
            self.offered = None;
            self.xid = self.xid.wrapping_add(1);
            self.retry = 4000;
        }
        if self.state == State::Init {
            self.state = State::Selecting;
        }
        if self.state == State::Bound {
            self.state = State::Renewing;
            self.xid = self.xid.wrapping_add(1);
        }
        if self.state == State::Renewing && now >= self.rebind {
            self.state = State::Rebinding;
        }
        let mut m = Message::new(
            if self.state == State::Selecting {
                DISCOVER
            } else {
                REQUEST
            },
            self.xid,
            self.mac,
        );
        let mut dest = Ipv4Addr::BROADCAST;
        if let Some(l) = &self.lease {
            m.ciaddr = l.address;
            if self.state == State::Renewing {
                dest = self.server.unwrap_or(dest);
            }
            self.deadline = (now + self.retry).min(if self.state == State::Renewing {
                self.rebind
            } else {
                l.expires
            });
        } else {
            m.requested = self.offered;
            m.server = self.server;
            self.deadline = now + self.retry;
        }
        self.retry = (self.retry * 2).min(64000);
        Some((dest, m))
    }
    /// Server the client is bound to (or requesting from).
    pub fn server(&self) -> Option<Ipv4Addr> {
        self.server
    }
    /// Absolute times (ms) of T1 (renew) and T2 (rebind); 0 before a lease.
    pub fn renew_at(&self) -> i64 {
        self.renew
    }
    pub fn rebind_at(&self) -> i64 {
        self.rebind
    }
    /// A DHCPRELEASE for the current lease, addressed to its server, and
    /// forget the lease. `None` if there is no lease.
    pub fn release(&mut self) -> Option<(Ipv4Addr, Message)> {
        let lease = self.lease.take()?;
        let server = self.server.take()?;
        self.xid = self.xid.wrapping_add(1);
        let mut m = Message::new(RELEASE, self.xid, self.mac);
        m.ciaddr = lease.address;
        m.server = Some(server);
        self.state = State::Init;
        self.offered = None;
        Some((server, m))
    }
    pub fn receive(&mut self, m: &Message, now: i64) -> bool {
        if !m.reply || m.xid != self.xid || m.mac != self.mac || m.server.is_none() {
            return false;
        }
        match (self.state, m.kind) {
            (State::Selecting, OFFER) if !m.yiaddr.is_unspecified() => {
                self.server = m.server;
                self.offered = Some(m.yiaddr);
                self.state = State::Requesting;
                self.retry = 4000;
                self.deadline = now;
                false
            }
            (State::Requesting | State::Renewing | State::Rebinding, ACK)
                if (self.state == State::Rebinding || m.server == self.server)
                    && m.lease_secs >= 4 =>
            {
                let Some(mask) = m.mask else {
                    return false;
                };
                let bits = u32::from_be_bytes(mask.0);
                let prefix = bits.leading_ones() as u8;
                if Ipv4Addr::mask_from_prefix(prefix) != mask {
                    return false;
                }
                let address = if m.yiaddr.is_unspecified() {
                    self.lease
                        .as_ref()
                        .map(|l| l.address)
                        .unwrap_or(Ipv4Addr::ZERO)
                } else {
                    m.yiaddr
                };
                if address.is_unspecified() || address.is_broadcast() || address.is_multicast() {
                    return false;
                }
                let duration = m.lease_secs as i64 * 1000;
                let t1 = if m.t1_secs > 0 {
                    m.t1_secs as i64 * 1000
                } else {
                    duration / 2
                };
                let t2 = if m.t2_secs > 0 {
                    m.t2_secs as i64 * 1000
                } else {
                    duration * 7 / 8
                };
                if !(0 < t1 && t1 < t2 && t2 < duration) {
                    return false;
                }
                self.lease = Some(Lease {
                    address,
                    prefix,
                    router: m.router,
                    dns: m.dns,
                    expires: now + duration,
                });
                self.server = m.server;
                self.renew = now + t1;
                self.rebind = now + t2;
                self.state = State::Bound;
                self.deadline = self.renew;
                self.retry = 4000;
                true
            }
            (State::Requesting | State::Renewing | State::Rebinding, NAK)
                if self.state == State::Rebinding || self.server == m.server =>
            {
                self.lease = None;
                self.server = None;
                self.offered = None;
                self.state = State::Init;
                self.xid = self.xid.wrapping_add(1);
                self.retry = 4000;
                self.deadline = now;
                true
            }
            _ => false,
        }
    }
}
