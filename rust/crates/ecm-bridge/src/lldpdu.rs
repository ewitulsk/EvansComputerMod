//! LLDPDU codec (IEEE 802.1AB-2016 clause 8).
//!
//! Mandatory TLVs (Chassis ID, Port ID, TTL) must appear first and in that
//! order. Supported optional TLVs: Port Description, System Name, System
//! Description, System Capabilities and Management Address (IPv4). Unknown
//! TLVs are skipped.

use crate::frame::{self, be16, ETHERTYPE_LLDP, LLDP_NEAREST_BRIDGE};

pub const TLV_END: u8 = 0;
pub const TLV_CHASSIS_ID: u8 = 1;
pub const TLV_PORT_ID: u8 = 2;
pub const TLV_TTL: u8 = 3;
pub const TLV_PORT_DESC: u8 = 4;
pub const TLV_SYS_NAME: u8 = 5;
pub const TLV_SYS_DESC: u8 = 6;
pub const TLV_SYS_CAPS: u8 = 7;
pub const TLV_MGMT_ADDR: u8 = 8;

pub const CHASSIS_SUBTYPE_MAC: u8 = 4;
pub const PORT_SUBTYPE_IFNAME: u8 = 5;
pub const CAP_BRIDGE: u16 = 0x0004;

/// Maximum length kept for string TLVs.
const MAX_STR: usize = 255;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lldpdu {
    /// Chassis ID TLV value including the subtype octet (2..=256 octets).
    pub chassis_id: Vec<u8>,
    /// Port ID TLV value including the subtype octet (2..=256 octets).
    pub port_id: Vec<u8>,
    pub ttl: u16,
    pub port_desc: Option<String>,
    pub sys_name: Option<String>,
    pub sys_desc: Option<String>,
    /// (capabilities, enabled capabilities)
    pub caps: Option<(u16, u16)>,
    pub mgmt_ipv4: Option<[u8; 4]>,
    /// ifIndex advertised with the management address.
    pub mgmt_ifindex: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LldpError {
    /// A TLV header or value runs past the end of the frame.
    Truncated,
    /// The first three TLVs are not Chassis ID, Port ID, TTL.
    MissingMandatory,
    /// A mandatory TLV has an invalid length.
    BadLength,
    /// A mandatory TLV appeared a second time.
    Duplicate,
}

fn push_tlv(out: &mut Vec<u8>, ty: u8, value: &[u8]) {
    let len = value.len().min(511);
    let hdr = ((ty as u16 & 0x7f) << 9) | len as u16;
    out.extend_from_slice(&hdr.to_be_bytes());
    out.extend_from_slice(value.get(..len).unwrap_or(&[]));
}

fn sanitize(b: &[u8]) -> String {
    b.iter()
        .take(MAX_STR)
        .map(|&c| if (32..127).contains(&c) { c as char } else { '.' })
        .collect()
}

/// Encode the LLDPDU (payload after the EtherType), including End TLV.
pub fn encode(p: &Lldpdu) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    push_tlv(&mut out, TLV_CHASSIS_ID, &p.chassis_id);
    push_tlv(&mut out, TLV_PORT_ID, &p.port_id);
    push_tlv(&mut out, TLV_TTL, &p.ttl.to_be_bytes());
    if let Some(s) = &p.port_desc {
        push_tlv(&mut out, TLV_PORT_DESC, s.as_bytes());
    }
    if let Some(s) = &p.sys_name {
        push_tlv(&mut out, TLV_SYS_NAME, s.as_bytes());
    }
    if let Some(s) = &p.sys_desc {
        push_tlv(&mut out, TLV_SYS_DESC, s.as_bytes());
    }
    if let Some((c, e)) = p.caps {
        let mut v = Vec::with_capacity(4);
        v.extend_from_slice(&c.to_be_bytes());
        v.extend_from_slice(&e.to_be_bytes());
        push_tlv(&mut out, TLV_SYS_CAPS, &v);
    }
    if let Some(ip) = p.mgmt_ipv4 {
        // addr-string-len, subtype 1 (IPv4), addr, if-subtype 2 (ifIndex), ifnum, OID len 0
        let mut v = vec![5, 1];
        v.extend_from_slice(&ip);
        v.push(2);
        v.extend_from_slice(&p.mgmt_ifindex.to_be_bytes());
        v.push(0);
        push_tlv(&mut out, TLV_MGMT_ADDR, &v);
    }
    push_tlv(&mut out, TLV_END, &[]);
    out
}

/// Decode an LLDPDU payload.
pub fn decode(b: &[u8]) -> Result<Lldpdu, LldpError> {
    let mut i = 0usize;
    let mut idx = 0usize;
    let mut out = Lldpdu {
        chassis_id: Vec::new(),
        port_id: Vec::new(),
        ttl: 0,
        port_desc: None,
        sys_name: None,
        sys_desc: None,
        caps: None,
        mgmt_ipv4: None,
        mgmt_ifindex: 0,
    };
    loop {
        if i >= b.len() {
            // End TLV is optional in 802.1AB-2016; running out cleanly is fine
            // once the mandatory TLVs have been seen.
            if idx < 3 {
                return Err(LldpError::MissingMandatory);
            }
            break;
        }
        let hdr = be16(b, i).ok_or(LldpError::Truncated)?;
        let ty = (hdr >> 9) as u8;
        let len = (hdr & 0x1ff) as usize;
        let v = b.get(i + 2..i + 2 + len).ok_or(LldpError::Truncated)?;
        i += 2 + len;
        match (idx, ty) {
            (0, TLV_CHASSIS_ID) => {
                if !(2..=256).contains(&len) {
                    return Err(LldpError::BadLength);
                }
                out.chassis_id = v.to_vec();
            }
            (1, TLV_PORT_ID) => {
                if !(2..=256).contains(&len) {
                    return Err(LldpError::BadLength);
                }
                out.port_id = v.to_vec();
            }
            (2, TLV_TTL) => {
                out.ttl = be16(v, 0).ok_or(LldpError::BadLength)?;
            }
            (0..=2, _) => return Err(LldpError::MissingMandatory),
            (_, TLV_END) => break,
            (_, TLV_CHASSIS_ID | TLV_PORT_ID | TLV_TTL) => return Err(LldpError::Duplicate),
            (_, TLV_PORT_DESC) => out.port_desc = Some(sanitize(v)),
            (_, TLV_SYS_NAME) => out.sys_name = Some(sanitize(v)),
            (_, TLV_SYS_DESC) => out.sys_desc = Some(sanitize(v)),
            (_, TLV_SYS_CAPS) => {
                if let (Some(c), Some(e)) = (be16(v, 0), be16(v, 2)) {
                    out.caps = Some((c, e));
                }
            }
            (_, TLV_MGMT_ADDR) => {
                // v[0] = addr string len (subtype + addr), v[1] = subtype
                if v.first() == Some(&5) && v.get(1) == Some(&1) {
                    if let Some(a) = v.get(2..6) {
                        let mut ip = [0u8; 4];
                        ip.copy_from_slice(a);
                        out.mgmt_ipv4 = Some(ip);
                        if let Some(n) = v.get(7..11) {
                            out.mgmt_ifindex = u32::from_be_bytes([n[0], n[1], n[2], n[3]]);
                        }
                    }
                }
            }
            _ => {} // unknown / organizationally specific: skip
        }
        idx += 1;
    }
    Ok(out)
}

/// Build a complete LLDP frame to the nearest-bridge address.
pub fn build_frame(src: [u8; 6], p: &Lldpdu) -> Vec<u8> {
    let mut f = frame::eth_header(LLDP_NEAREST_BRIDGE, src, ETHERTYPE_LLDP);
    f.extend_from_slice(&encode(p));
    frame::pad_min(f)
}

/// Render an ID TLV value (subtype + id) for display.
pub fn id_to_string(v: &[u8]) -> String {
    let (sub, rest) = match v.split_first() {
        Some((s, r)) => (*s, r),
        None => return String::new(),
    };
    let printable = !rest.is_empty() && rest.iter().all(|&c| (32..127).contains(&c));
    if (sub == 3 || sub == 4) && rest.len() == 6 {
        // MAC address subtypes (port: 3, chassis: 4)
        return rest.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(":");
    }
    if printable {
        return String::from_utf8_lossy(rest).to_string();
    }
    rest.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Lldpdu {
        Lldpdu {
            chassis_id: vec![4, 2, 0, 0, 0, 0, 1],
            port_id: vec![5, b'e', b't', b'h', b'3'],
            ttl: 120,
            port_desc: Some("Port 3".into()),
            sys_name: Some("sw1".into()),
            sys_desc: Some("desc".into()),
            caps: Some((CAP_BRIDGE, CAP_BRIDGE)),
            mgmt_ipv4: Some([10, 0, 0, 1]),
            mgmt_ifindex: 4,
        }
    }

    #[test]
    fn round_trip() {
        let p = sample();
        assert_eq!(decode(&encode(&p)), Ok(p.clone()));
        let min = Lldpdu { port_desc: None, sys_name: None, sys_desc: None, caps: None, mgmt_ipv4: None, mgmt_ifindex: 0, ..p };
        assert_eq!(decode(&encode(&min)), Ok(min));
    }

    #[test]
    fn frame_padding_and_trailing_zeros() {
        let f = build_frame([2; 6], &Lldpdu { port_desc: None, sys_name: None, sys_desc: None, caps: None, mgmt_ipv4: None, ..sample() });
        assert!(f.len() >= 60);
        // Padding after End TLV is ignored.
        assert!(decode(&f[14..]).is_ok());
    }

    #[test]
    fn ttl_zero_decodes() {
        let p = Lldpdu { ttl: 0, ..sample() };
        assert_eq!(decode(&encode(&p)).map(|x| x.ttl), Ok(0));
    }

    #[test]
    fn mandatory_order_enforced() {
        let mut out = Vec::new();
        push_tlv(&mut out, TLV_PORT_ID, &[5, b'x']);
        push_tlv(&mut out, TLV_CHASSIS_ID, &[4, 1, 2, 3, 4, 5, 6]);
        push_tlv(&mut out, TLV_TTL, &[0, 5]);
        assert_eq!(decode(&out), Err(LldpError::MissingMandatory));
        let mut out = Vec::new();
        push_tlv(&mut out, TLV_CHASSIS_ID, &[4]);
        assert_eq!(decode(&out), Err(LldpError::BadLength));
        let mut enc = encode(&sample());
        let n = enc.len();
        enc.truncate(n - 2); // drop End TLV
        push_tlv(&mut enc, TLV_TTL, &[0, 1]);
        assert_eq!(decode(&enc), Err(LldpError::Duplicate));
    }

    #[test]
    fn truncated_and_garbage() {
        let enc = encode(&sample());
        for n in 0..enc.len() - 2 {
            // Any strict prefix either lacks mandatory TLVs or truncates one.
            let r = decode(&enc[..n]);
            if n < 18 {
                assert!(r.is_err(), "prefix {} decoded", n);
            }
        }
        let mut x: u32 = 99;
        for _ in 0..3000 {
            let n = (x % 200) as usize;
            let v: Vec<u8> = (0..n).map(|_| { x ^= x << 13; x ^= x >> 17; x ^= x << 5; x as u8 }).collect();
            let _ = decode(&v);
        }
    }

    #[test]
    fn id_rendering() {
        assert_eq!(id_to_string(&[4, 2, 0, 0, 0, 0, 1]), "02:00:00:00:00:01");
        assert_eq!(id_to_string(&[5, b'e', b't', b'h', b'0']), "eth0");
        assert_eq!(id_to_string(&[7, 0, 1]), "00:01");
        assert_eq!(id_to_string(&[]), "");
    }
}
