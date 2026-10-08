//! DHCPv4 server lease logic, shared by the kernel router service
//! (`ecm-router`) and the `dhcpd` program.
//!
//! Sans-IO: [`LeaseDb::handle`] takes one parsed client message and returns
//! the reply to send (if any). The caller picks the pool for the interface
//! the message arrived on and decides how to transmit the reply.
//!
//! Behaviour (RFC 2131 subset):
//! - DISCOVER → OFFER: the client's reservation, else the address it asked
//!   for, else its previous lease, else the first free address in the range.
//!   Offers are held for [`OFFER_HOLD_MS`]. No free address → no reply.
//! - REQUEST → ACK if the address is in the pool's subnet and free for this
//!   client (and matches its reservation, if it has one), else NAK.
//! - REQUEST naming another server → our offer to that client is withdrawn.
//! - RELEASE frees the lease; DECLINE parks the address for
//!   [`DECLINE_HOLD_MS`]; INFORM gets an ACK with options but no lease.
//! - Relayed messages (giaddr set) are ignored.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::dhcp::{Message, ACK, DECLINE, DISCOVER, INFORM, NAK, OFFER, RELEASE, REQUEST};
use crate::{Ipv4Addr, MacAddr};

/// How long an OFFER reserves its address for the client.
pub const OFFER_HOLD_MS: i64 = 60_000;
/// How long a DECLINEd address is kept out of use.
pub const DECLINE_HOLD_MS: i64 = 600_000;
/// Largest pool range (addresses).
pub const MAX_POOL_SIZE: u32 = 65_536;
/// Upper bound on stored leases (restore stops there).
pub const MAX_LEASES: usize = 65_536;

/// One address pool (one per interface in `dhcpd`, one per named pool in
/// the router).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolSpec {
    pub name: String,
    pub start: Ipv4Addr,
    pub end: Ipv4Addr,
    /// Subnet prefix handed out with the lease (option 1).
    pub prefix: u8,
    pub router: Option<Ipv4Addr>,
    pub dns: Option<Ipv4Addr>,
    pub lease_secs: u32,
}

impl PoolSpec {
    /// Range inside one subnet, at most [`MAX_POOL_SIZE`] addresses, and a
    /// lease long enough for T1 < T2 < lease.
    pub fn is_valid(&self) -> bool {
        let (s, e) = (u32::from_be_bytes(self.start.0), u32::from_be_bytes(self.end.0));
        !self.start.is_unspecified()
            && s <= e
            && e - s < MAX_POOL_SIZE
            && (1..=30).contains(&self.prefix)
            && self.start.same_subnet_prefix(&self.end, self.prefix)
            && self.lease_secs >= 4
    }

    fn in_range(&self, a: Ipv4Addr) -> bool {
        let n = u32::from_be_bytes(a.0);
        u32::from_be_bytes(self.start.0) <= n && n <= u32::from_be_bytes(self.end.0)
    }

    /// A usable host address of the pool's subnet.
    pub fn in_subnet(&self, a: Ipv4Addr) -> bool {
        a.same_subnet_prefix(&self.start, self.prefix)
            && a != self.start.network_addr(self.prefix)
            && a != self.start.broadcast_addr(self.prefix)
    }
}

/// A fixed address for one MAC (`reserve <mac> <ip>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reservation {
    pub mac: MacAddr,
    pub address: Ipv4Addr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    pub pool: String,
    /// `MacAddr::ZERO` for a declined (parked) address.
    pub mac: MacAddr,
    pub address: Ipv4Addr,
    /// Absolute expiry, ms on the caller's clock.
    pub expires: i64,
    /// Only offered (not yet ACKed); not persisted.
    pub offered: bool,
}

/// Every lease of every pool.
#[derive(Clone, Debug, Default)]
pub struct LeaseDb {
    pub leases: Vec<Lease>,
    /// Bound leases changed since the caller last persisted [`render`](Self::render).
    pub dirty: bool,
}

pub fn parse_mac(s: &str) -> Option<MacAddr> {
    let parts: Vec<u8> = s
        .split([':', '-'])
        .map(|p| (p.len() == 2).then(|| u8::from_str_radix(p, 16).ok()).flatten())
        .collect::<Option<_>>()?;
    let b: [u8; 6] = parts.try_into().ok()?;
    Some(MacAddr(b))
}

impl LeaseDb {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop expired leases and offers. Returns how many bound leases expired.
    pub fn expire(&mut self, now: i64) -> usize {
        let before = self.leases.iter().filter(|l| !l.offered).count();
        self.leases.retain(|l| now < l.expires);
        let gone = before - self.leases.iter().filter(|l| !l.offered).count();
        if gone > 0 {
            self.dirty = true;
        }
        gone
    }

    /// The live lease (bound or offered) of `mac` in `pool`.
    pub fn lease_of(&self, pool: &str, mac: MacAddr) -> Option<&Lease> {
        self.leases.iter().find(|l| l.pool == pool && l.mac == mac)
    }

    /// Bound (ACKed) leases of `pool`.
    pub fn bound<'a>(&'a self, pool: &'a str) -> impl Iterator<Item = &'a Lease> + 'a {
        self.leases.iter().filter(move |l| l.pool == pool && !l.offered && l.mac != MacAddr::ZERO)
    }

    /// Answer one client message for `pool`, as server `server` (our
    /// address on that network, sent as the server identifier).
    pub fn handle(
        &mut self,
        pool: &PoolSpec,
        reservations: &[Reservation],
        m: &Message,
        server: Ipv4Addr,
        now: i64,
    ) -> Option<Message> {
        if m.reply || !m.relay.is_unspecified() || server.is_unspecified() || !pool.is_valid() {
            return None;
        }
        self.expire(now);
        let name = pool.name.as_str();
        if let Some(s) = m.server {
            if s != server {
                // The client chose another server: withdraw our offer.
                if m.kind == REQUEST {
                    self.leases
                        .retain(|l| !(l.offered && l.mac == m.mac && l.pool == name));
                }
                return None;
            }
        }
        match m.kind {
            RELEASE => {
                let before = self.leases.len();
                self.leases
                    .retain(|l| !(l.mac == m.mac && l.address == m.ciaddr && l.pool == name));
                if self.leases.len() != before {
                    self.dirty = true;
                }
                return None;
            }
            DECLINE => {
                if let Some(l) = self.leases.iter_mut().find(|l| {
                    l.mac == m.mac && Some(l.address) == m.requested && l.pool == name
                }) {
                    l.mac = MacAddr::ZERO;
                    l.offered = false;
                    l.expires = now + DECLINE_HOLD_MS;
                    self.dirty = true;
                }
                return None;
            }
            INFORM => {
                if !pool.in_subnet(m.ciaddr) {
                    return None;
                }
                let mut r = self.reply(ACK, m, pool, server);
                r.yiaddr = Ipv4Addr::ZERO;
                return Some(r);
            }
            DISCOVER | REQUEST => {}
            _ => return None,
        }
        let reserved = reservations
            .iter()
            .find(|r| r.mac == m.mac && pool.in_subnet(r.address))
            .map(|r| r.address);
        let available = |a: Ipv4Addr| {
            pool.in_subnet(a)
                && a != server
                && (pool.in_range(a) || reserved == Some(a))
                && !reservations.iter().any(|r| r.address == a && r.mac != m.mac)
                && !self
                    .leases
                    .iter()
                    .any(|l| l.address == a && l.pool == name && l.mac != m.mac)
        };
        let existing = self.lease_of(name, m.mac).map(|l| l.address);
        let requested = m
            .requested
            .or_else(|| (!m.ciaddr.is_unspecified()).then_some(m.ciaddr));
        let address = if m.kind == DISCOVER {
            let first_free = || {
                let (s, e) = (u32::from_be_bytes(pool.start.0), u32::from_be_bytes(pool.end.0));
                (s..=e).map(|n| Ipv4Addr(n.to_be_bytes())).find(|a| available(*a))
            };
            match reserved {
                Some(r) => Some(r).filter(|a| available(*a)),
                None => requested
                    .filter(|a| available(*a))
                    .or_else(|| existing.filter(|a| available(*a)))
                    .or_else(first_free),
            }?
        } else {
            match requested {
                Some(a) if available(a) && reserved.is_none_or(|r| r == a) => a,
                _ => return Some(self.reply(NAK, m, pool, server)),
            }
        };
        let offer = m.kind == DISCOVER;
        let mut r = self.reply(if offer { OFFER } else { ACK }, m, pool, server);
        r.yiaddr = address;
        r.lease_secs = pool.lease_secs;
        r.t1_secs = pool.lease_secs / 2;
        r.t2_secs = pool.lease_secs / 8 * 7;
        if r.t2_secs <= r.t1_secs {
            r.t2_secs = r.t1_secs + 1;
        }
        let was_bound = self
            .leases
            .iter()
            .any(|l| l.mac == m.mac && l.pool == name && !l.offered);
        // An OFFER never shortens a bound lease (a bound client may DISCOVER again).
        if !(offer && was_bound) {
            self.leases.retain(|l| !(l.mac == m.mac && l.pool == name));
            self.leases.push(Lease {
                pool: pool.name.clone(),
                mac: m.mac,
                address,
                expires: now
                    + if offer {
                        OFFER_HOLD_MS
                    } else {
                        pool.lease_secs as i64 * 1000
                    },
                offered: offer,
            });
            if !offer {
                self.dirty = true;
            }
        }
        Some(r)
    }

    fn reply(&self, kind: u8, m: &Message, pool: &PoolSpec, server: Ipv4Addr) -> Message {
        let mut r = Message::new(kind, m.xid, m.mac);
        r.reply = true;
        r.server = Some(server);
        if kind != NAK {
            r.mask = Some(Ipv4Addr::mask_from_prefix(pool.prefix));
            r.router = pool.router;
            r.dns = pool.dns;
        }
        r
    }

    /// Bound leases, one per line: `<pool> <mac> <ip> <expires-ms>`
    /// (declined addresses keep MAC 00:00:00:00:00:00).
    pub fn render(&self) -> String {
        let mut out = String::new();
        for l in self.leases.iter().filter(|l| !l.offered) {
            let _ = writeln!(out, "{} {} {} {}", l.pool, l.mac, l.address, l.expires);
        }
        out
    }

    /// Load [`render`](Self::render) output, skipping malformed or expired lines.
    pub fn restore(&mut self, text: &str, now: i64) {
        for line in text.lines() {
            let f: Vec<_> = line.split_whitespace().collect();
            if f.len() != 4 || self.leases.len() >= MAX_LEASES {
                continue;
            }
            if let (Some(mac), Some(ip), Ok(expires)) =
                (parse_mac(f[1]), Ipv4Addr::parse(f[2]), f[3].parse::<i64>())
            {
                if expires > now {
                    self.leases.retain(|l| !(l.pool == f[0] && l.address == ip));
                    self.leases.push(Lease {
                        pool: f[0].to_string(),
                        mac,
                        address: ip,
                        expires,
                        offered: false,
                    });
                }
            }
        }
    }

    /// Leases grouped by pool name (for status output).
    pub fn by_pool(&self) -> BTreeMap<&str, Vec<&Lease>> {
        let mut m: BTreeMap<&str, Vec<&Lease>> = BTreeMap::new();
        for l in &self.leases {
            m.entry(l.pool.as_str()).or_default().push(l);
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dhcp::Client;

    fn ip(s: &str) -> Ipv4Addr {
        Ipv4Addr::parse(s).unwrap()
    }
    fn pool() -> PoolSpec {
        PoolSpec {
            name: "eth0".into(),
            start: ip("192.168.50.10"),
            end: ip("192.168.50.12"),
            prefix: 24,
            router: Some(ip("192.168.50.1")),
            dns: Some(ip("1.1.1.1")),
            lease_secs: 3600,
        }
    }
    const SRV: &str = "192.168.50.1";
    fn mac(n: u8) -> MacAddr {
        MacAddr([2, 0, 0, 0, 0, n])
    }
    fn msg(kind: u8, n: u8) -> Message {
        Message::new(kind, 7, mac(n))
    }
    fn dora(db: &mut LeaseDb, res: &[Reservation], n: u8, now: i64) -> Option<Ipv4Addr> {
        let offer = db.handle(&pool(), res, &msg(DISCOVER, n), ip(SRV), now)?;
        assert_eq!(offer.kind, OFFER);
        let mut req = msg(REQUEST, n);
        req.requested = Some(offer.yiaddr);
        req.server = offer.server;
        let ack = db.handle(&pool(), res, &req, ip(SRV), now)?;
        (ack.kind == ACK).then_some(ack.yiaddr)
    }

    #[test]
    fn allocates_in_order_with_options_and_exhausts() {
        let mut db = LeaseDb::new();
        let offer = db.handle(&pool(), &[], &msg(DISCOVER, 1), ip(SRV), 0).unwrap();
        assert_eq!(offer.yiaddr, ip("192.168.50.10"));
        assert_eq!(offer.mask, Some(ip("255.255.255.0")));
        assert_eq!(offer.router, Some(ip(SRV)));
        assert_eq!(offer.dns, Some(ip("1.1.1.1")));
        assert_eq!((offer.lease_secs, offer.t1_secs, offer.t2_secs), (3600, 1800, 3150));
        assert_eq!(dora(&mut db, &[], 1, 0), Some(ip("192.168.50.10")));
        assert_eq!(dora(&mut db, &[], 2, 0), Some(ip("192.168.50.11")));
        assert_eq!(dora(&mut db, &[], 3, 0), Some(ip("192.168.50.12")));
        // Exhausted: no offer for a fourth client.
        assert!(db.handle(&pool(), &[], &msg(DISCOVER, 4), ip(SRV), 0).is_none());
        // A known client asking again gets its own address back.
        assert_eq!(dora(&mut db, &[], 2, 10), Some(ip("192.168.50.11")));
        assert_eq!(db.bound("eth0").count(), 3);
    }

    #[test]
    fn reservation_wins_and_is_kept_from_others() {
        let res = [Reservation {
            mac: mac(9),
            address: ip("192.168.50.10"),
        }];
        let mut db = LeaseDb::new();
        // Another client never gets the reserved address.
        assert_eq!(dora(&mut db, &res, 1, 0), Some(ip("192.168.50.11")));
        let mut steal = msg(REQUEST, 2);
        steal.requested = Some(ip("192.168.50.10"));
        assert_eq!(db.handle(&pool(), &res, &steal, ip(SRV), 0).unwrap().kind, NAK);
        // The reserved MAC gets it even when asking for something else.
        let mut d = msg(DISCOVER, 9);
        d.requested = Some(ip("192.168.50.12"));
        assert_eq!(db.handle(&pool(), &res, &d, ip(SRV), 0).unwrap().yiaddr, ip("192.168.50.10"));
        assert_eq!(dora(&mut db, &res, 9, 0), Some(ip("192.168.50.10")));
        // Reservations outside the range (but in the subnet) work too.
        let out = [Reservation {
            mac: mac(8),
            address: ip("192.168.50.200"),
        }];
        assert_eq!(dora(&mut db, &out, 8, 0), Some(ip("192.168.50.200")));
    }

    #[test]
    fn leases_expire_and_offers_lapse() {
        let mut db = LeaseDb::new();
        assert_eq!(dora(&mut db, &[], 1, 0), Some(ip("192.168.50.10")));
        // An unanswered offer holds its address for OFFER_HOLD_MS only.
        let o = db.handle(&pool(), &[], &msg(DISCOVER, 2), ip(SRV), 0).unwrap();
        assert_eq!(o.yiaddr, ip("192.168.50.11"));
        let o3 = db.handle(&pool(), &[], &msg(DISCOVER, 3), ip(SRV), 1).unwrap();
        assert_eq!(o3.yiaddr, ip("192.168.50.12"));
        let o4 = db.handle(&pool(), &[], &msg(DISCOVER, 4), ip(SRV), OFFER_HOLD_MS + 5).unwrap();
        assert_eq!(o4.yiaddr, ip("192.168.50.11"), "lapsed offer is reused");
        // The bound lease expires after lease_secs.
        db.dirty = false;
        assert_eq!(db.expire(3_600_000), 1);
        assert!(db.dirty);
        assert!(db.lease_of("eth0", mac(1)).is_none());
    }

    #[test]
    fn nak_on_wrong_subnet_or_unknown_and_other_server_ignored() {
        let mut db = LeaseDb::new();
        let mut r = msg(REQUEST, 1);
        r.requested = Some(ip("10.0.0.5"));
        assert_eq!(db.handle(&pool(), &[], &r, ip(SRV), 0).unwrap().kind, NAK);
        let nak = db.handle(&pool(), &[], &r, ip(SRV), 0).unwrap();
        assert_eq!(nak.server, Some(ip(SRV)));
        assert!(nak.mask.is_none() && nak.lease_secs == 0);
        // Out of range but in subnet, without a reservation: NAK.
        r.requested = Some(ip("192.168.50.99"));
        assert_eq!(db.handle(&pool(), &[], &r, ip(SRV), 0).unwrap().kind, NAK);
        // No requested address at all: NAK.
        assert_eq!(db.handle(&pool(), &[], &msg(REQUEST, 1), ip(SRV), 0).unwrap().kind, NAK);
        // Choosing another server withdraws our offer, without a reply.
        let o = db.handle(&pool(), &[], &msg(DISCOVER, 2), ip(SRV), 0).unwrap();
        let mut other = msg(REQUEST, 2);
        other.requested = Some(o.yiaddr);
        other.server = Some(ip("192.168.50.2"));
        assert!(db.handle(&pool(), &[], &other, ip(SRV), 0).is_none());
        assert!(db.lease_of("eth0", mac(2)).is_none());
        // Relayed messages are not ours to answer.
        let mut relayed = msg(DISCOVER, 3);
        relayed.relay = ip("192.168.60.1");
        assert!(db.handle(&pool(), &[], &relayed, ip(SRV), 0).is_none());
    }

    #[test]
    fn release_frees_decline_parks_inform_answers() {
        let mut db = LeaseDb::new();
        assert_eq!(dora(&mut db, &[], 1, 0), Some(ip("192.168.50.10")));
        let mut rel = msg(RELEASE, 1);
        rel.ciaddr = ip("192.168.50.10");
        rel.server = Some(ip(SRV));
        assert!(db.handle(&pool(), &[], &rel, ip(SRV), 5).is_none());
        assert!(db.lease_of("eth0", mac(1)).is_none());
        // Freed: the next client gets .10 again.
        assert_eq!(dora(&mut db, &[], 2, 6), Some(ip("192.168.50.10")));
        // DECLINE parks the address: nobody is offered it for a while.
        let mut dec = msg(DECLINE, 2);
        dec.requested = Some(ip("192.168.50.10"));
        dec.server = Some(ip(SRV));
        assert!(db.handle(&pool(), &[], &dec, ip(SRV), 7).is_none());
        let o = db.handle(&pool(), &[], &msg(DISCOVER, 3), ip(SRV), 8).unwrap();
        assert_eq!(o.yiaddr, ip("192.168.50.11"));
        let o = db
            .handle(&pool(), &[], &msg(DISCOVER, 4), ip(SRV), 8 + DECLINE_HOLD_MS)
            .unwrap();
        assert_eq!(o.yiaddr, ip("192.168.50.10"));
        // INFORM: options, no address, no lease.
        let mut inf = msg(INFORM, 5);
        inf.ciaddr = ip("192.168.50.77");
        let ack = db.handle(&pool(), &[], &inf, ip(SRV), 9).unwrap();
        assert_eq!(ack.kind, ACK);
        assert!(ack.yiaddr.is_unspecified() && ack.lease_secs == 0);
        assert_eq!(ack.router, Some(ip(SRV)));
        assert!(db.lease_of("eth0", mac(5)).is_none());
    }

    #[test]
    fn persistence_roundtrip_and_real_client() {
        let mut db = LeaseDb::new();
        let mut c = Client::new(mac(1), 42, 0);
        let (_, d) = c.poll(0).unwrap();
        let offer = db.handle(&pool(), &[], &Message::parse(&d.encode()).unwrap(), ip(SRV), 0).unwrap();
        c.receive(&Message::parse(&offer.encode()).unwrap(), 0);
        let (_, r) = c.poll(0).unwrap();
        let ack = db.handle(&pool(), &[], &Message::parse(&r.encode()).unwrap(), ip(SRV), 0).unwrap();
        assert!(c.receive(&Message::parse(&ack.encode()).unwrap(), 0));
        let lease = c.lease.clone().unwrap();
        assert_eq!((lease.address, lease.prefix), (ip("192.168.50.10"), 24));
        let text = db.render();
        assert_eq!(text, "eth0 02:00:00:00:00:01 192.168.50.10 3600000\n");
        let mut back = LeaseDb::new();
        back.restore(&text, 1000);
        back.restore("garbage\neth0 zz:00 1.2.3.4 9\n", 1000);
        assert_eq!(back.leases, db.leases.iter().filter(|l| !l.offered).cloned().collect::<Vec<_>>());
        let mut expired = LeaseDb::new();
        expired.restore(&text, 4_000_000);
        assert!(expired.leases.is_empty());
        assert_eq!(parse_mac("AA-bb-0c-00-00-01"), Some(MacAddr([0xaa, 0xbb, 0x0c, 0, 0, 1])));
        assert_eq!(parse_mac("aa:bb:cc"), None);
    }
}
