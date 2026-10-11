//! BGP-4 framing, IPv4 unicast UPDATE attributes and OPEN capabilities.
use crate::{Attributes, Prefix};
use ecm_net::Ipv4Addr;
pub const MAX_MESSAGE: usize = 4096;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Open {
        asn: u32,
        hold: u16,
        id: Ipv4Addr,
        as4: bool,
        refresh: bool,
    },
    Update {
        withdrawn: Vec<Prefix>,
        attributes: Attributes,
        nlri: Vec<Prefix>,
    },
    Notification {
        code: u8,
        subcode: u8,
    },
    Keepalive,
    Refresh,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub code: u8,
    pub subcode: u8,
}
fn error(code: u8, subcode: u8) -> Error {
    Error { code, subcode }
}
fn prefixes(data: &[u8]) -> Result<Vec<Prefix>, Error> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let bits = data[i];
        i += 1;
        if bits > 32 {
            return Err(error(3, 10));
        }
        let n = (bits as usize + 7) / 8;
        let bytes = data.get(i..i + n).ok_or(error(3, 10))?;
        i += n;
        let mut ip = [0; 4];
        ip[..n].copy_from_slice(bytes);
        result.push(Prefix::new(Ipv4Addr(ip), bits));
    }
    Ok(result)
}
fn put_prefixes(out: &mut Vec<u8>, p: &[Prefix]) {
    for p in p {
        out.push(p.len);
        out.extend_from_slice(&p.address.0[..(p.len as usize + 7) / 8]);
    }
}
fn attr(out: &mut Vec<u8>, flags: u8, kind: u8, value: &[u8]) {
    if value.len() > 255 {
        out.extend_from_slice(&[flags | 16, kind]);
        out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    } else {
        out.extend_from_slice(&[flags, kind, value.len() as u8]);
    }
    out.extend_from_slice(value);
}
fn path(value: &[u8], as4: bool) -> Result<(Vec<u32>, Vec<(usize, usize)>), Error> {
    let mut out = Vec::new();
    let mut sets = Vec::new();
    let mut i = 0;
    let width = if as4 { 4 } else { 2 };
    while i < value.len() {
        let head = value.get(i..i + 2).ok_or(error(3, 11))?;
        i += 2;
        if !matches!(head[0], 1 | 2) || head[1] == 0 {
            return Err(error(3, 11));
        }
        if head[0] == 1 {
            sets.push((out.len(), head[1] as usize));
        }
        for _ in 0..head[1] {
            let b = value.get(i..i + width).ok_or(error(3, 11))?;
            i += width;
            out.push(if as4 {
                u32::from_be_bytes(b.try_into().unwrap())
            } else {
                u16::from_be_bytes(b.try_into().unwrap()) as u32
            });
        }
    }
    Ok((out, sets))
}
fn put_path(p: &[u32], sets: &[(usize, usize)], as4: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < p.len() {
        let set = sets.iter().find(|(offset, _)| *offset == start);
        let end = if let Some((_, len)) = set {
            start + len
        } else {
            sets.iter()
                .filter(|(offset, _)| *offset > start)
                .map(|(offset, _)| *offset)
                .min()
                .unwrap_or(p.len())
                .min(start + 255)
        };
        let chunk = &p[start..end];
        out.extend_from_slice(&[if set.is_some() { 1 } else { 2 }, chunk.len() as u8]);
        for a in chunk {
            if as4 {
                out.extend_from_slice(&a.to_be_bytes());
            } else {
                out.extend_from_slice(&(if *a > 65535 { 23456 } else { *a as u16 }).to_be_bytes());
            }
        }
        start = end;
    }
    out
}
impl Message {
    pub fn encode(&self, as4: bool) -> Vec<u8> {
        self.encode_for_peer(as4, true)
    }
    pub fn encode_for_peer(&self, as4: bool, internal: bool) -> Vec<u8> {
        let mut b = vec![255; 16];
        b.extend_from_slice(&[0, 0, 0]);
        match self {
            Self::Open {
                asn,
                hold,
                id,
                as4,
                refresh,
            } => {
                b[18] = 1;
                b.push(4);
                b.extend_from_slice(
                    &(if *asn > 65535 { 23456u16 } else { *asn as u16 }).to_be_bytes(),
                );
                b.extend_from_slice(&hold.to_be_bytes());
                b.extend_from_slice(&id.0);
                let mut caps = vec![1, 4, 0, 1, 0, 1];
                if *as4 {
                    caps.extend_from_slice(&[65, 4]);
                    caps.extend_from_slice(&asn.to_be_bytes());
                }
                if *refresh {
                    caps.extend_from_slice(&[2, 0]);
                }
                b.push((caps.len() + 2) as u8);
                b.extend_from_slice(&[2, caps.len() as u8]);
                b.extend(caps);
            }
            Self::Keepalive => b[18] = 4,
            Self::Notification { code, subcode } => {
                b[18] = 3;
                b.extend_from_slice(&[*code, *subcode]);
            }
            Self::Refresh => {
                b[18] = 5;
                b.extend_from_slice(&[0, 1, 0, 1]);
            }
            Self::Update {
                withdrawn,
                attributes: a,
                nlri,
            } => {
                b[18] = 2;
                let mut w = Vec::new();
                put_prefixes(&mut w, withdrawn);
                b.extend_from_slice(&(w.len() as u16).to_be_bytes());
                b.extend(w);
                let mut v = Vec::new();
                if !nlri.is_empty() {
                    attr(&mut v, 64, 1, &[a.origin]);
                    attr(&mut v, 64, 2, &put_path(&a.path, &a.path_sets, as4));
                    attr(&mut v, 64, 3, &a.next_hop.0);
                    if !as4 && a.path.iter().any(|a| *a > 65535) {
                        attr(&mut v, 192, 17, &put_path(&a.path, &a.path_sets, true));
                    }
                    attr(&mut v, 128, 4, &a.med.to_be_bytes());
                    if internal {
                        attr(&mut v, 64, 5, &a.local_pref.to_be_bytes());
                    }
                    if !a.communities.is_empty() {
                        let c: Vec<_> =
                            a.communities.iter().flat_map(|n| n.to_be_bytes()).collect();
                        attr(&mut v, 192, 8, &c);
                    }
                    for (kind, value) in &a.unknown_transitive {
                        attr(&mut v, 224, *kind, value);
                    }
                }
                b.extend_from_slice(&(v.len() as u16).to_be_bytes());
                b.extend(v);
                put_prefixes(&mut b, nlri);
            }
        }
        let n = b.len() as u16;
        b[16..18].copy_from_slice(&n.to_be_bytes());
        b
    }
    /// None means an incomplete TCP frame. Errors contain NOTIFICATION codes.
    pub fn decode(data: &[u8], as4: bool) -> Result<Option<(Self, usize)>, Error> {
        if data.len() < 19 {
            return Ok(None);
        }
        if data[..16] != [255; 16] {
            return Err(error(1, 1));
        }
        let len = u16::from_be_bytes([data[16], data[17]]) as usize;
        if !(19..=MAX_MESSAGE).contains(&len) {
            return Err(error(1, 2));
        }
        if data.len() < len {
            return Ok(None);
        }
        let p = &data[19..len];
        let msg = match data[18] {
            1 => {
                if p.len() < 10 || p[0] != 4 || p.len() != 10 + p[9] as usize {
                    return Err(error(2, 4));
                }
                let hold = u16::from_be_bytes([p[3], p[4]]);
                if hold == 1 || hold == 2 {
                    return Err(error(2, 6));
                }
                let id = Ipv4Addr::from_bytes(&p[5..9]);
                if id.is_unspecified() || id.is_multicast() || id.is_broadcast() {
                    return Err(error(2, 3));
                }
                let mut asn = u16::from_be_bytes([p[1], p[2]]) as u32;
                let mut four = false;
                let mut refresh = false;
                let mut i = 10;
                while i < p.len() {
                    let head = p.get(i..i + 2).ok_or(error(2, 4))?;
                    i += 2;
                    let v = p.get(i..i + head[1] as usize).ok_or(error(2, 4))?;
                    i += v.len();
                    if head[0] != 2 {
                        return Err(error(2, 4));
                    }
                    let mut j = 0;
                    while j < v.len() {
                        let cap = v.get(j..j + 2).ok_or(error(2, 4))?;
                        j += 2;
                        let body = v.get(j..j + cap[1] as usize).ok_or(error(2, 4))?;
                        j += body.len();
                        match (cap[0], body.len()) {
                            (65, 4) => {
                                four = true;
                                asn = u32::from_be_bytes(body.try_into().unwrap());
                            }
                            (2, 0) => refresh = true,
                            _ => {}
                        }
                    }
                }
                Self::Open {
                    asn,
                    hold,
                    id,
                    as4: four,
                    refresh,
                }
            }
            4 if p.is_empty() => Self::Keepalive,
            3 if p.len() >= 2 => Self::Notification {
                code: p[0],
                subcode: p[1],
            },
            5 if p == [0, 1, 0, 1] => Self::Refresh,
            2 => {
                if p.len() < 4 {
                    return Err(error(3, 1));
                }
                let wl = u16::from_be_bytes([p[0], p[1]]) as usize;
                let w = p.get(2..2 + wl).ok_or(error(3, 1))?;
                let head = p.get(2 + wl..4 + wl).ok_or(error(3, 1))?;
                let al = u16::from_be_bytes(head.try_into().unwrap()) as usize;
                let values = p.get(4 + wl..4 + wl + al).ok_or(error(3, 1))?;
                let mut a = Attributes::default();
                let mut seen = [false; 256];
                let mut i = 0;
                let mut as4path = None;
                let mut mp_nlri = Vec::new();
                let mut mp_withdraw = Vec::new();
                while i < values.len() {
                    let h = values.get(i..i + 3).ok_or(error(3, 1))?;
                    let flags = h[0];
                    let kind = h[1];
                    i += 2;
                    let n = if flags & 16 != 0 {
                        let b = values.get(i..i + 2).ok_or(error(3, 1))?;
                        i += 2;
                        u16::from_be_bytes(b.try_into().unwrap()) as usize
                    } else {
                        let n = values[i] as usize;
                        i += 1;
                        n
                    };
                    let v = values.get(i..i + n).ok_or(error(3, 1))?;
                    i += n;
                    if seen[kind as usize] {
                        return Err(error(3, 1));
                    }
                    seen[kind as usize] = true;
                    let required = match kind {
                        1 | 2 | 3 | 5 => Some(64),
                        4 | 14 | 15 => Some(128),
                        8 | 17 => Some(192),
                        _ => None,
                    };
                    if flags & 15 != 0
                        || required.is_some_and(|expected| flags & 192 != expected)
                        || flags & 32 != 0 && flags & 192 != 192
                    {
                        return Err(error(3, 4));
                    }
                    match kind {
                        1 if n == 1 && v[0] <= 2 => a.origin = v[0],
                        2 => {
                            (a.path, a.path_sets) = path(v, as4)?;
                        }
                        3 if n == 4 => a.next_hop = Ipv4Addr::from_bytes(v),
                        4 if n == 4 => a.med = u32::from_be_bytes(v.try_into().unwrap()),
                        5 if n == 4 => a.local_pref = u32::from_be_bytes(v.try_into().unwrap()),
                        8 if n % 4 == 0 => {
                            a.communities = v
                                .chunks_exact(4)
                                .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
                                .collect()
                        }
                        17 => as4path = Some(path(v, true)?),
                        14 if n >= 9 && v[..4] == [0, 1, 1, 4] => {
                            a.next_hop = Ipv4Addr::from_bytes(&v[4..8]);
                            mp_nlri = prefixes(&v[9..])?;
                        }
                        15 if n >= 3 && v[..3] == [0, 1, 1] => mp_withdraw = prefixes(&v[3..])?,
                        1 | 3 | 4 | 5 | 8 | 14 | 15 => return Err(error(3, 5)),
                        _ if flags & 128 == 0 => return Err(error(3, 2)),
                        _ if flags & 64 != 0 => a.unknown_transitive.push((kind, v.to_vec())),
                        _ => {}
                    }
                }
                if !as4 {
                    if let Some((p, sets)) = as4path {
                        if p.len() <= a.path.len() {
                            let keep = a.path.len() - p.len();
                            // Never splice through a set: its membership is indivisible.
                            if a.path_sets.iter().any(|(s, n)| *s < keep && s + n > keep) {
                                return Err(error(3, 11));
                            }
                            a.path.truncate(keep);
                            a.path_sets.retain(|(s, _)| *s < keep);
                            a.path_sets
                                .extend(sets.into_iter().map(|(s, n)| (s + keep, n)));
                            a.path.extend(p);
                        }
                    }
                }
                let mut nlri = prefixes(&p[4 + wl + al..])?;
                nlri.extend(mp_nlri);
                if !nlri.is_empty() && (!seen[1] || !seen[2] || (!seen[3] && !seen[14])) {
                    return Err(error(3, 3));
                }
                if !nlri.is_empty()
                    && (a.next_hop.is_unspecified()
                        || a.next_hop.is_multicast()
                        || a.next_hop.is_broadcast())
                {
                    return Err(error(3, 8));
                }
                let mut withdrawn = prefixes(w)?;
                withdrawn.extend(mp_withdraw);
                Self::Update {
                    withdrawn,
                    attributes: a,
                    nlri,
                }
            }
            _ => return Err(error(1, 3)),
        };
        Ok(Some((msg, len)))
    }
}

/// RFC 7606 treat-as-withdraw when the IPv4 NLRI can be delimited safely.
/// Corrupt framing/NLRI cannot be recovered and requires a session reset.
pub fn withdraw_malformed_update(data: &[u8]) -> Option<(Message, usize)> {
    if data.len() < 23 || data[18] != 2 {
        return None;
    }
    let len = u16::from_be_bytes([data[16], data[17]]) as usize;
    if len > data.len() || len < 23 {
        return None;
    }
    let p = &data[19..len];
    let wl = u16::from_be_bytes([p[0], p[1]]) as usize;
    let head = p.get(2 + wl..4 + wl)?;
    let al = u16::from_be_bytes(head.try_into().ok()?) as usize;
    let attrs = p.get(4 + wl..4 + wl + al)?;
    let mut withdrawn = prefixes(p.get(2..2 + wl)?).ok()?;
    withdrawn.extend(prefixes(p.get(4 + wl + al..)?).ok()?);
    let mut i = 0;
    while i < attrs.len() {
        let h = attrs.get(i..i + 3)?;
        let kind = h[1];
        let extended = h[0] & 16 != 0;
        i += 2;
        let n = if extended {
            let n = u16::from_be_bytes(attrs.get(i..i + 2)?.try_into().ok()?) as usize;
            i += 2;
            n
        } else {
            let n = *attrs.get(i)? as usize;
            i += 1;
            n
        };
        let value = attrs.get(i..i + n)?;
        i += n;
        if kind == 14 {
            if value.len() < 9 || value[..4] != [0, 1, 1, 4] {
                return None;
            }
            withdrawn.extend(prefixes(&value[9..]).ok()?);
        }
        if kind == 15 {
            if value.len() < 3 || value[..3] != [0, 1, 1] {
                return None;
            }
            withdrawn.extend(prefixes(&value[3..]).ok()?);
        }
    }
    Some((
        Message::Update {
            withdrawn,
            attributes: Attributes::default(),
            nlri: Vec::new(),
        },
        len,
    ))
}
