use ecm_net::{
    dhcp::{Message, ACK, DECLINE, DISCOVER, INFORM, NAK, OFFER, RELEASE, REQUEST},
    Ipv4Addr, MacAddr,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Pool {
    pub start: Ipv4Addr,
    pub end: Ipv4Addr,
    pub router: Ipv4Addr,
    pub dns: Ipv4Addr,
    pub lease_secs: u32,
    pub enabled: bool,
}
impl Default for Pool {
    fn default() -> Self {
        Self {
            start: Ipv4Addr::ZERO,
            end: Ipv4Addr::ZERO,
            router: Ipv4Addr::ZERO,
            dns: Ipv4Addr::ZERO,
            lease_secs: 86400,
            enabled: false,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Lease {
    pub pool: String,
    pub mac: MacAddr,
    pub address: Ipv4Addr,
    pub expires: i64,
    pub offered: bool,
}
pub struct Server {
    pub leases: Vec<Lease>,
    pub dirty: bool,
}
impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}
impl Server {
    pub fn new() -> Self {
        Self {
            leases: Vec::new(),
            dirty: false,
        }
    }
    pub fn receive(
        &mut self,
        pools: &BTreeMap<String, Pool>,
        m: &Message,
        server: Ipv4Addr,
        now: i64,
    ) -> Option<Message> {
        if m.reply || !m.relay.is_unspecified() || server.is_unspecified() {
            return None;
        }
        if m.server.is_some_and(|s| s != server) {
            return None;
        }
        self.leases.retain(|l| now < l.expires);
        let (name, p) = pools.iter().find(|(_, p)| {
            p.enabled && p.router == server && p.start != Ipv4Addr::ZERO && p.lease_secs >= 4
        })?;
        let start = u32::from_be_bytes(p.start.0);
        let end = u32::from_be_bytes(p.end.0);
        if end < start || end - start > 65535 {
            return None;
        }
        if m.kind == RELEASE {
            self.leases
                .retain(|l| !(l.mac == m.mac && l.address == m.ciaddr && l.pool == *name));
            self.dirty = true;
            return None;
        }
        if m.kind == DECLINE {
            if let Some(l) = self
                .leases
                .iter_mut()
                .find(|l| l.mac == m.mac && Some(l.address) == m.requested && l.pool == *name)
            {
                l.mac = MacAddr::ZERO;
                l.expires = now + 600_000;
                self.dirty = true;
            }
            return None;
        }
        if !matches!(m.kind, DISCOVER | REQUEST | INFORM) {
            return None;
        }
        let existing = self
            .leases
            .iter()
            .find(|l| l.mac == m.mac && l.pool == *name)
            .map(|l| l.address);
        let requested = m
            .requested
            .or_else(|| (!m.ciaddr.is_unspecified()).then_some(m.ciaddr));
        let available = |a: Ipv4Addr| {
            let n = u32::from_be_bytes(a.0);
            start <= n
                && n <= end
                && a != server
                && a.0[3] != 0
                && a.0[3] != 255
                && !self
                    .leases
                    .iter()
                    .any(|l| l.address == a && l.pool == *name && l.mac != m.mac)
        };
        let address = if m.kind == INFORM {
            m.ciaddr
        } else if let Some(a) = requested.filter(|a| available(*a)) {
            a
        } else if m.kind == REQUEST {
            Ipv4Addr::ZERO
        } else {
            existing.filter(|a| available(*a)).or_else(|| {
                (start..=end)
                    .map(|n| Ipv4Addr(n.to_be_bytes()))
                    .find(|a| available(*a))
            })?
        };
        let mut reply = Message::new(
            if address.is_unspecified() {
                NAK
            } else if m.kind == DISCOVER {
                OFFER
            } else {
                ACK
            },
            m.xid,
            m.mac,
        );
        reply.reply = true;
        reply.server = Some(server);
        if reply.kind == NAK {
            return Some(reply);
        }
        reply.yiaddr = if m.kind == INFORM {
            Ipv4Addr::ZERO
        } else {
            address
        };
        reply.mask = Some(Ipv4Addr::mask_from_prefix(24));
        reply.router = Some(p.router);
        reply.dns = Some(p.dns);
        if m.kind != INFORM {
            reply.lease_secs = p.lease_secs;
            reply.t1_secs = p.lease_secs / 2;
            reply.t2_secs = p.lease_secs * 7 / 8;
            self.leases.retain(|l| !(l.mac == m.mac && l.pool == *name));
            self.leases.push(Lease {
                pool: name.clone(),
                mac: m.mac,
                address,
                expires: now
                    + if m.kind == DISCOVER {
                        60_000
                    } else {
                        p.lease_secs as i64 * 1000
                    },
                offered: m.kind == DISCOVER,
            });
            self.dirty = true;
        }
        Some(reply)
    }
    pub fn render(&self) -> String {
        self.leases
            .iter()
            .filter(|l| !l.offered)
            .map(|l| format!("{} {} {} {}\n", l.pool, l.mac, l.address, l.expires))
            .collect()
    }
    pub fn restore(&mut self, text: &str, now: i64) {
        for line in text.lines() {
            let f: Vec<_> = line.split_whitespace().collect();
            if f.len() != 4 {
                continue;
            }
            let bytes: Option<Vec<_>> = f[1]
                .split(':')
                .map(|s| u8::from_str_radix(s, 16).ok())
                .collect();
            if let (Some(b), Some(ip), Ok(expires)) =
                (bytes, Ipv4Addr::parse(f[2]), f[3].parse::<i64>())
            {
                if b.len() == 6 && expires > now && self.leases.len() < 65536 {
                    self.leases.push(Lease {
                        pool: f[0].to_string(),
                        mac: MacAddr(b.try_into().unwrap()),
                        address: ip,
                        expires,
                        offered: false,
                    });
                }
            }
        }
    }
}
