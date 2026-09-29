//! DNS client wire format — A record query / response.
//!
//! Hardened: label/name length limits, compression-pointer hop limit,
//! transaction-ID and question matching.

use super::types::{Ipv4Addr, NetError};

pub const DNS_PORT: u16 = 53;
/// Max compression pointers followed while reading one name.
pub const MAX_POINTER_HOPS: usize = 16;
const MAX_NAME_LEN: usize = 255;

/// Encode a domain name into DNS wire format ("example.com" → "\x07example\x03com\x00").
/// A single trailing dot is allowed. Returns `None` for empty labels, labels > 63 bytes,
/// or names longer than 255 bytes on the wire.
pub fn encode_name_vec(name: &str) -> Option<Vec<u8>> {
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(name.len() + 2);
    for label in name.split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    if out.len() > MAX_NAME_LEN {
        return None;
    }
    Some(out)
}

/// Encode into `buf`; returns bytes written or 0 on error / insufficient space.
pub fn encode_name(name: &str, buf: &mut [u8]) -> usize {
    match encode_name_vec(name) {
        Some(v) => match buf.get_mut(..v.len()) {
            Some(b) => {
                b.copy_from_slice(&v);
                v.len()
            }
            None => 0,
        },
        None => 0,
    }
}

/// Build an A/IN query with recursion desired.
pub fn build_query(name: &str, tx_id: u16) -> Option<Vec<u8>> {
    let qname = encode_name_vec(name)?;
    let mut b = Vec::with_capacity(12 + qname.len() + 4);
    b.extend_from_slice(&tx_id.to_be_bytes());
    b.extend_from_slice(&0x0100u16.to_be_bytes()); // RD
    b.extend_from_slice(&1u16.to_be_bytes()); // qdcount
    b.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    b.extend_from_slice(&qname);
    b.extend_from_slice(&1u16.to_be_bytes()); // A
    b.extend_from_slice(&1u16.to_be_bytes()); // IN
    Some(b)
}

/// Read a (possibly compressed) name at `offset`. Returns the lowercase dotted name
/// and the offset just past the name in the original position.
pub fn read_name(data: &[u8], offset: usize) -> Option<(String, usize)> {
    let mut name = String::new();
    let mut pos = offset;
    let mut end = None;
    let mut hops = 0;
    let mut wire_len = 0usize;
    loop {
        let len = *data.get(pos)? as usize;
        if len == 0 {
            if end.is_none() {
                end = Some(pos + 1);
            }
            break;
        }
        match len & 0xC0 {
            0xC0 => {
                let lo = *data.get(pos + 1)? as usize;
                if end.is_none() {
                    end = Some(pos + 2);
                }
                hops += 1;
                if hops > MAX_POINTER_HOPS {
                    return None;
                }
                pos = ((len & 0x3F) << 8) | lo;
            }
            0x00 => {
                let label = data.get(pos + 1..pos + 1 + len)?;
                wire_len += 1 + len;
                if wire_len > MAX_NAME_LEN {
                    return None;
                }
                if !name.is_empty() {
                    name.push('.');
                }
                for &c in label {
                    name.push(c.to_ascii_lowercase() as char);
                }
                pos += 1 + len;
            }
            _ => return None, // reserved label types
        }
    }
    Some((name, end?))
}

/// Parse a response to the query `tx_id` for `qname`. Returns the first A record.
///
/// Errors: `InvalidPacket` for anything malformed or not matching (wrong id, not a
/// response, wrong question), `NotFound` for NXDOMAIN or no A record, `ConnectionRefused`
/// for other RCODEs (SERVFAIL, REFUSED, ...).
pub fn parse_answer(data: &[u8], tx_id: u16, qname: &str) -> Result<Ipv4Addr, NetError> {
    let h = data.get(..12).ok_or(NetError::InvalidPacket)?;
    let id = u16::from_be_bytes([h[0], h[1]]);
    let flags = u16::from_be_bytes([h[2], h[3]]);
    let qdcount = u16::from_be_bytes([h[4], h[5]]) as usize;
    let ancount = u16::from_be_bytes([h[6], h[7]]) as usize;
    if id != tx_id || flags & 0x8000 == 0 || qdcount != 1 {
        return Err(NetError::InvalidPacket);
    }
    let want = qname.strip_suffix('.').unwrap_or(qname).to_ascii_lowercase();
    let (got, mut off) = read_name(data, 12).ok_or(NetError::InvalidPacket)?;
    let q = data.get(off..off + 4).ok_or(NetError::InvalidPacket)?;
    if got != want || q != [0, 1, 0, 1] {
        return Err(NetError::InvalidPacket);
    }
    off += 4;
    match flags & 0x000f {
        0 => {}
        3 => return Err(NetError::NotFound),
        _ => return Err(NetError::ConnectionRefused),
    }
    for _ in 0..ancount {
        let (_, o) = read_name(data, off).ok_or(NetError::InvalidPacket)?;
        let rr = data.get(o..o + 10).ok_or(NetError::InvalidPacket)?;
        let rtype = u16::from_be_bytes([rr[0], rr[1]]);
        let rclass = u16::from_be_bytes([rr[2], rr[3]]);
        let rdlen = u16::from_be_bytes([rr[8], rr[9]]) as usize;
        let rdata = data.get(o + 10..o + 10 + rdlen).ok_or(NetError::InvalidPacket)?;
        if rtype == 1 && rclass == 1 && rdlen == 4 {
            return Ok(Ipv4Addr::from_bytes(rdata));
        }
        off = o + 10 + rdlen;
    }
    Err(NetError::NotFound)
}

/// Lenient legacy parser: first A record of any response, no id/question checks.
pub fn parse_response(data: &[u8]) -> Option<Ipv4Addr> {
    let h = data.get(..12)?;
    let qdcount = u16::from_be_bytes([h[4], h[5]]) as usize;
    let ancount = u16::from_be_bytes([h[6], h[7]]) as usize;
    let mut off = 12;
    for _ in 0..qdcount {
        off = read_name(data, off)?.1 + 4;
        if off > data.len() {
            return None;
        }
    }
    for _ in 0..ancount {
        let (_, o) = read_name(data, off)?;
        let rr = data.get(o..o + 10)?;
        let rtype = u16::from_be_bytes([rr[0], rr[1]]);
        let rdlen = u16::from_be_bytes([rr[8], rr[9]]) as usize;
        let rdata = data.get(o + 10..o + 10 + rdlen)?;
        if rtype == 1 && rdlen == 4 {
            return Some(Ipv4Addr::from_bytes(rdata));
        }
        off = o + 10 + rdlen;
    }
    None
}

/// Get the DNS port.
pub fn dns_port() -> u16 {
    DNS_PORT
}

/// Build a response to `query` (test/server helper): echoes the question and adds one A
/// record per address, using a compression pointer to the question name. `rcode` goes in
/// the low flag bits.
pub fn build_response(query: &[u8], addrs: &[Ipv4Addr], rcode: u8) -> Option<Vec<u8>> {
    let h = query.get(..12)?;
    let (_, qend) = read_name(query, 12)?;
    let question = query.get(12..qend + 4)?;
    let mut b = Vec::new();
    b.extend_from_slice(&h[0..2]);
    b.extend_from_slice(&(0x8180u16 | (rcode as u16 & 0xf)).to_be_bytes());
    b.extend_from_slice(&1u16.to_be_bytes());
    b.extend_from_slice(&(addrs.len() as u16).to_be_bytes());
    b.extend_from_slice(&[0, 0, 0, 0]);
    b.extend_from_slice(question);
    for a in addrs {
        b.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
        b.extend_from_slice(&a.0);
    }
    Some(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode() {
        assert_eq!(encode_name_vec("example.com").unwrap(), b"\x07example\x03com\x00");
        assert_eq!(encode_name_vec("example.com.").unwrap(), b"\x07example\x03com\x00");
        for bad in ["", ".", "a..b", ".a", &"x".repeat(64), &["abcdefghi"; 30].join(".")] {
            assert!(encode_name_vec(bad).is_none(), "{bad:?}");
        }
        assert_eq!(encode_name("a.b", &mut [0u8; 4]), 0);
        assert_eq!(encode_name("a.b", &mut [0u8; 5]), 5);
    }

    #[test]
    fn query_response_roundtrip() {
        let q = build_query("Host.Example.com", 0xbeef).unwrap();
        let r = build_response(&q, &[Ipv4Addr::new(1, 2, 3, 4), Ipv4Addr::new(5, 6, 7, 8)], 0).unwrap();
        assert_eq!(parse_answer(&r, 0xbeef, "host.example.com"), Ok(Ipv4Addr::new(1, 2, 3, 4)));
        assert_eq!(parse_answer(&r, 0xbeee, "host.example.com"), Err(NetError::InvalidPacket));
        assert_eq!(parse_answer(&r, 0xbeef, "other.example.com"), Err(NetError::InvalidPacket));
        assert_eq!(parse_response(&r), Some(Ipv4Addr::new(1, 2, 3, 4)));
        let nx = build_response(&q, &[], 3).unwrap();
        assert_eq!(parse_answer(&nx, 0xbeef, "host.example.com"), Err(NetError::NotFound));
        let empty = build_response(&q, &[], 0).unwrap();
        assert_eq!(parse_answer(&empty, 0xbeef, "host.example.com"), Err(NetError::NotFound));
        let sf = build_response(&q, &[], 2).unwrap();
        assert_eq!(parse_answer(&sf, 0xbeef, "host.example.com"), Err(NetError::ConnectionRefused));
        // the query itself is not a response
        assert_eq!(parse_answer(&q, 0xbeef, "host.example.com"), Err(NetError::InvalidPacket));
    }

    #[test]
    fn pointer_loops_are_bounded() {
        // name at 12 is a pointer to itself
        let mut d = vec![0u8; 12];
        d[5] = 1;
        d[7] = 1;
        d.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1]);
        assert!(read_name(&d, 12).is_none());
        assert!(parse_response(&d).is_none());
        // two pointers pointing at each other
        let mut d = vec![0u8; 12];
        d.extend_from_slice(&[0xC0, 14, 0xC0, 12]);
        assert!(read_name(&d, 12).is_none());
        // a chain of 16 pointers is fine, 17 is not
        let mut d = vec![0u8; 12];
        let base = 12;
        for i in 0..17u8 {
            d.extend_from_slice(&[0xC0, base + 2 * (i + 1)]);
        }
        d.extend_from_slice(&[1, b'a', 0]);
        assert!(read_name(&d, 12 + 2).is_some());
        assert!(read_name(&d, 12).is_none());
    }

    #[test]
    fn truncated_and_garbage() {
        let q = build_query("a.example", 1).unwrap();
        let r = build_response(&q, &[Ipv4Addr::new(9, 9, 9, 9)], 0).unwrap();
        for n in 0..r.len() {
            let _ = parse_answer(&r[..n], 1, "a.example");
            assert!(parse_response(&r[..n]).is_none() || n == r.len());
            let _ = read_name(&r[..n], 12);
            let _ = build_response(&r[..n], &[], 0);
        }
        assert!(read_name(&[0x40, 0], 0).is_none()); // reserved label type
        assert!(read_name(&[5, b'a'], 0).is_none());
        // name > 255 bytes via pointers
        let mut d = vec![];
        d.push(63);
        d.extend_from_slice(&[b'x'; 63]);
        d.extend_from_slice(&[0xC0, 0]);
        assert!(read_name(&d, 0).is_none());
    }
}
