//! DNS client — A record query/response.

use super::types::Ipv4Addr;

const DNS_PORT: u16 = 53;

/// Encode a domain name into DNS wire format.
/// "example.com" → "\x07example\x03com\x00"
/// Returns bytes written.
pub fn encode_name(name: &str, buf: &mut [u8]) -> usize {
    let mut pos = 0;
    for label in name.split('.') {
        if label.is_empty() {
            continue;
        }
        let len = label.len();
        if pos + 1 + len >= buf.len() {
            return 0;
        }
        buf[pos] = len as u8;
        pos += 1;
        buf[pos..pos + len].copy_from_slice(label.as_bytes());
        pos += len;
    }
    if pos < buf.len() {
        buf[pos] = 0; // null terminator
        pos += 1;
    }
    pos
}

/// Build a DNS query for an A record.
/// Returns total packet length.
pub fn build_query(name: &str, tx_id: u16, buf: &mut [u8]) -> usize {
    if buf.len() < 512 {
        return 0;
    }

    // Header (12 bytes)
    buf[0..2].copy_from_slice(&tx_id.to_be_bytes());
    buf[2..4].copy_from_slice(&0x0100u16.to_be_bytes()); // flags: standard query, recursion desired
    buf[4..6].copy_from_slice(&1u16.to_be_bytes()); // qdcount = 1
    buf[6..8].copy_from_slice(&0u16.to_be_bytes()); // ancount = 0
    buf[8..10].copy_from_slice(&0u16.to_be_bytes()); // nscount = 0
    buf[10..12].copy_from_slice(&0u16.to_be_bytes()); // arcount = 0

    // Question
    let name_len = encode_name(name, &mut buf[12..]);
    if name_len == 0 {
        return 0;
    }
    let qtype_offset = 12 + name_len;
    buf[qtype_offset..qtype_offset + 2].copy_from_slice(&1u16.to_be_bytes()); // QTYPE = A
    buf[qtype_offset + 2..qtype_offset + 4].copy_from_slice(&1u16.to_be_bytes()); // QCLASS = IN

    qtype_offset + 4
}

/// Skip a DNS name in a response (handles compression pointers).
/// Returns the new offset after the name, or None on error.
fn skip_name(data: &[u8], mut offset: usize) -> Option<usize> {
    let mut jumped = false;
    let mut result_offset = 0;

    loop {
        if offset >= data.len() {
            return None;
        }
        let len = data[offset] as usize;
        if len == 0 {
            if !jumped {
                result_offset = offset + 1;
            }
            break;
        }
        if len & 0xC0 == 0xC0 {
            // Compression pointer
            if offset + 1 >= data.len() {
                return None;
            }
            if !jumped {
                result_offset = offset + 2;
                jumped = true;
            }
            let ptr = ((len & 0x3F) << 8) | data[offset + 1] as usize;
            offset = ptr;
        } else {
            offset += 1 + len;
            if !jumped {
                result_offset = offset;
            }
        }
    }

    Some(result_offset)
}

/// Parse a DNS response and extract the first A record IP.
pub fn parse_response(data: &[u8]) -> Option<Ipv4Addr> {
    if data.len() < 12 {
        return None;
    }

    let _flags = u16::from_be_bytes([data[2], data[3]]);
    let qdcount = u16::from_be_bytes([data[4], data[5]]) as usize;
    let ancount = u16::from_be_bytes([data[6], data[7]]) as usize;

    if ancount == 0 {
        return None;
    }

    // Skip questions
    let mut offset = 12;
    for _ in 0..qdcount {
        offset = skip_name(data, offset)?;
        offset += 4; // QTYPE + QCLASS
        if offset > data.len() {
            return None;
        }
    }

    // Parse answers
    for _ in 0..ancount {
        offset = skip_name(data, offset)?;
        if offset + 10 > data.len() {
            return None;
        }
        let rtype = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let _rclass = u16::from_be_bytes([data[offset + 2], data[offset + 3]]);
        let _ttl = u32::from_be_bytes([data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7]]);
        let rdlength = u16::from_be_bytes([data[offset + 8], data[offset + 9]]) as usize;
        offset += 10;

        if offset + rdlength > data.len() {
            return None;
        }

        if rtype == 1 && rdlength == 4 {
            // A record
            return Some(Ipv4Addr::from_bytes(&data[offset..offset + 4]));
        }

        offset += rdlength;
    }

    None
}

/// Get the DNS port.
pub fn dns_port() -> u16 {
    DNS_PORT
}
