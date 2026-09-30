//! Endpoint-independent mapping, bounded at 4096 entries; no eviction of live flows.
use ecm_net::{
    checksum::{internet_checksum, pseudo_header_checksum},
    ipv4::Ipv4Header,
    Ipv4Addr,
};

pub const UDP_TIMEOUT: i64 = 300_000;
pub const TCP_ESTABLISHED_TIMEOUT: i64 = 7_440_000;
pub const TCP_TRANSITORY_TIMEOUT: i64 = 240_000;
pub const ICMP_TIMEOUT: i64 = 60_000;
pub const MAX_MAPPINGS: usize = 4096;
#[derive(Clone, Debug)]
pub struct Forward {
    pub protocol: u8,
    pub inside: Ipv4Addr,
    pub inside_port: u16,
    pub outside_port: u16,
}
#[derive(Clone, Debug)]
pub struct Mapping {
    pub protocol: u8,
    pub inside: Ipv4Addr,
    pub port: u16,
    pub outside: Ipv4Addr,
    pub external: u16,
    pub expires: i64,
    pub established: bool,
    pub closing: bool,
}
pub struct Nat {
    pub mappings: Vec<Mapping>,
    next: u16,
}
impl Default for Nat {
    fn default() -> Self {
        Self::new()
    }
}
impl Nat {
    pub fn new() -> Self {
        Self {
            mappings: Vec::new(),
            next: 49152,
        }
    }
    pub fn expire(&mut self, now: i64) {
        self.mappings.retain(|m| now < m.expires);
    }
    pub fn outbound(
        &mut self,
        p: &mut [u8],
        outside: Ipv4Addr,
        forwards: &[Forward],
        now: i64,
    ) -> Result<(), ()> {
        self.expire(now);
        let (h, seg) = Ipv4Header::parse(p).ok_or(())?;
        if h.is_fragment() || outside.is_unspecified() {
            return Err(());
        }
        validate_transport(&h, seg)?;
        if h.protocol == 1 && seg.first().is_some_and(|t| matches!(t, 3 | 11 | 12)) {
            return self.translate_error(p, outside, forwards, true);
        }
        let port = identifier(h.protocol, seg, true)?;
        if let Some(f) = forwards
            .iter()
            .find(|f| f.protocol == h.protocol && f.inside == h.src && f.inside_port == port)
        {
            return rewrite(p, true, outside, f.outside_port);
        }
        let idx = if let Some(i) = self.mappings.iter().position(|m| {
            m.protocol == h.protocol && m.inside == h.src && m.port == port && m.outside == outside
        }) {
            i
        } else {
            if self.mappings.len() >= MAX_MAPPINGS {
                return Err(());
            }
            let mut external = None;
            for _ in 0..16384 {
                let n = self.next;
                self.next = if n == 65535 { 49152 } else { n + 1 };
                if !self
                    .mappings
                    .iter()
                    .any(|m| m.protocol == h.protocol && m.external == n && m.outside == outside)
                    && !forwards
                        .iter()
                        .any(|f| f.protocol == h.protocol && f.outside_port == n)
                {
                    external = Some(n);
                    break;
                }
            }
            self.mappings.push(Mapping {
                protocol: h.protocol,
                inside: h.src,
                port,
                outside,
                external: external.ok_or(())?,
                expires: now,
                established: false,
                closing: false,
            });
            self.mappings.len() - 1
        };
        let m = &mut self.mappings[idx];
        touch(m, seg, now);
        let n = m.external;
        rewrite(p, true, outside, n)
    }
    pub fn inbound(
        &mut self,
        p: &mut [u8],
        outside: Ipv4Addr,
        forwards: &[Forward],
        now: i64,
    ) -> Result<(), ()> {
        self.expire(now);
        let (h, seg) = Ipv4Header::parse(p).ok_or(())?;
        if h.is_fragment() || h.dst != outside {
            return Err(());
        }
        validate_transport(&h, seg)?;
        if h.protocol == 1 && seg.first().is_some_and(|t| matches!(t, 3 | 11 | 12)) {
            return self.translate_error(p, outside, forwards, false);
        }
        let n = identifier(h.protocol, seg, false)?;
        if let Some(f) = forwards
            .iter()
            .find(|f| f.protocol == h.protocol && f.outside_port == n)
        {
            return rewrite(p, false, f.inside, f.inside_port);
        }
        let m = self
            .mappings
            .iter_mut()
            .find(|m| m.protocol == h.protocol && m.outside == outside && m.external == n)
            .ok_or(())?;
        touch(m, seg, now);
        let (ip, port) = (m.inside, m.port);
        rewrite(p, false, ip, port)
    }
    fn translate_error(
        &self,
        p: &mut [u8],
        outside: Ipv4Addr,
        forwards: &[Forward],
        outbound: bool,
    ) -> Result<(), ()> {
        let (h, seg) = Ipv4Header::parse(p).ok_or(())?;

        if seg.len() < 36 {
            return Err(());
        }
        let quote = &seg[8..];
        let hl = (quote[0] as usize & 15) * 4;
        if quote[0] >> 4 != 4
            || hl < 20
            || quote.len() < hl + 8
            || internet_checksum(&quote[..hl]) != 0
        {
            return Err(());
        }
        let proto = quote[9];
        let address_pos = if outbound { 16 } else { 12 };
        let ip = Ipv4Addr::from_bytes(&quote[address_pos..address_pos + 4]);
        let n = if matches!(proto, 6 | 17) {
            {
                let port_pos = hl + if outbound { 2 } else { 0 };
                u16::from_be_bytes([quote[port_pos], quote[port_pos + 1]])
            }
        } else {
            identifier(proto, &quote[hl..], true)?
        };
        let translated = self
            .mappings
            .iter()
            .find(|m| {
                m.protocol == proto
                    && m.outside == outside
                    && if outbound {
                        m.inside == ip && m.port == n
                    } else {
                        m.outside == ip && m.external == n
                    }
            })
            .map(|m| {
                if outbound {
                    (outside, m.external)
                } else {
                    (m.inside, m.port)
                }
            })
            .or_else(|| {
                forwards
                    .iter()
                    .find(|f| {
                        f.protocol == proto
                            && if outbound {
                                f.inside == ip && f.inside_port == n
                            } else {
                                ip == outside && f.outside_port == n
                            }
                    })
                    .map(|f| {
                        if outbound {
                            (outside, f.outside_port)
                        } else {
                            (f.inside, f.inside_port)
                        }
                    })
            });
        let (inside, port) = translated.ok_or(())?;
        let offset = h.header_len();
        let outer_pos = if outbound { 12 } else { 16 };
        p[outer_pos..outer_pos + 4].copy_from_slice(&inside.0);
        let q = &mut p[offset + 8..];
        let ck = match proto {
            17 => Some(hl + 6),
            6 if q.len() >= hl + 18 => Some(hl + 16),
            1 => Some(hl + 2),
            _ => None,
        };
        if let Some(ck) = ck {
            let original = u16::from_be_bytes([q[ck], q[ck + 1]]);
            if proto != 17 || original != 0 {
                let mut sum = original;
                if proto != 1 {
                    for i in [0, 2] {
                        sum = adjust(
                            sum,
                            u16::from_be_bytes([ip.0[i], ip.0[i + 1]]),
                            u16::from_be_bytes([inside.0[i], inside.0[i + 1]]),
                        );
                    }
                }
                sum = adjust(sum, n, port);
                if proto == 17 && sum == 0 {
                    sum = 65535;
                }
                q[ck..ck + 2].copy_from_slice(&sum.to_be_bytes());
            }
        }
        q[address_pos..address_pos + 4].copy_from_slice(&inside.0);
        let pos = if proto == 1 {
            hl + 4
        } else {
            hl + if outbound { 2 } else { 0 }
        };
        q[pos..pos + 2].copy_from_slice(&port.to_be_bytes());
        q[10..12].fill(0);
        let sum = internet_checksum(&q[..hl]);
        q[10..12].copy_from_slice(&sum.to_be_bytes());
        p[offset + 2..offset + 4].fill(0);
        let sum = internet_checksum(&p[offset..h.total_length as usize]);
        p[offset + 2..offset + 4].copy_from_slice(&sum.to_be_bytes());
        p[10..12].fill(0);
        let sum = internet_checksum(&p[..offset]);
        p[10..12].copy_from_slice(&sum.to_be_bytes());
        Ok(())
    }
}
fn validate_transport(h: &Ipv4Header, seg: &[u8]) -> Result<(), ()> {
    let valid = match h.protocol {
        6 => {
            ecm_net::tcp::TcpHeader::parse(seg).is_some()
                && ecm_net::tcp::verify_checksum(&h.src, &h.dst, seg)
        }
        17 => {
            ecm_net::udp::UdpHeader::parse(seg).is_some()
                && ecm_net::udp::verify_checksum(&h.src, &h.dst, seg)
        }
        1 => seg.len() >= 8 && internet_checksum(seg) == 0,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(())
    }
}
fn adjust(checksum: u16, old: u16, new: u16) -> u16 {
    let mut sum = (!checksum as u32) + (!old as u32) + (new as u32);
    while sum > 65535 {
        sum = (sum & 65535) + (sum >> 16);
    }
    !(sum as u16)
}
fn touch(m: &mut Mapping, seg: &[u8], now: i64) {
    let timeout = match m.protocol {
        17 => UDP_TIMEOUT,
        1 => ICMP_TIMEOUT,
        6 => {
            let flags = seg.get(13).copied().unwrap_or(0);
            if flags & 0x02 != 0 {
                m.closing = false;
                m.established = false;
            }
            if !m.closing && flags & 0x10 != 0 && flags & 0x02 == 0 {
                m.established = true;
            }
            if flags & 0x05 != 0 {
                m.established = false;
                m.closing = true;
            }
            if m.established {
                TCP_ESTABLISHED_TIMEOUT
            } else {
                TCP_TRANSITORY_TIMEOUT
            }
        }
        _ => 0,
    };
    m.expires = now.saturating_add(timeout);
}
fn identifier(proto: u8, seg: &[u8], source: bool) -> Result<u16, ()> {
    let pos = match proto {
        6 if seg.len() >= 20 => {
            if source {
                0
            } else {
                2
            }
        }
        17 if seg.len() >= 8 => {
            if source {
                0
            } else {
                2
            }
        }
        1 if seg.len() >= 8 && matches!(seg[0], 0 | 8) => 4,
        _ => return Err(()),
    };
    Ok(u16::from_be_bytes([seg[pos], seg[pos + 1]]))
}
fn rewrite(p: &mut [u8], source: bool, ip: Ipv4Addr, port: u16) -> Result<(), ()> {
    let (h, _) = Ipv4Header::parse(p).ok_or(())?;
    let hl = h.header_len();
    let total = h.total_length as usize;
    let pos = if source { 12 } else { 16 };
    p[pos..pos + 4].copy_from_slice(&ip.0);
    let s = Ipv4Addr::from_bytes(&p[12..16]);
    let d = Ipv4Addr::from_bytes(&p[16..20]);
    let seg = &mut p[hl..total];
    let pos = if h.protocol == 1 {
        4
    } else if source {
        0
    } else {
        2
    };
    seg[pos..pos + 2].copy_from_slice(&port.to_be_bytes());
    let ck = match h.protocol {
        6 => 16,
        17 => 6,
        1 => 2,
        _ => return Err(()),
    };
    seg[ck..ck + 2].fill(0);
    let mut sum = if h.protocol == 1 {
        internet_checksum(seg)
    } else {
        pseudo_header_checksum(&s, &d, h.protocol, seg)
    };
    if sum == 0 && h.protocol == 17 {
        sum = 65535;
    }
    seg[ck..ck + 2].copy_from_slice(&sum.to_be_bytes());
    p[10..12].fill(0);
    let sum = internet_checksum(&p[..hl]);
    p[10..12].copy_from_slice(&sum.to_be_bytes());
    Ok(())
}
