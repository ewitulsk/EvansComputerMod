//! Just enough X.509 for the in-game certificate authority: ed25519
//! certificates (a root, and leaves with DNS and IP names), PKCS#8 keys, PEM,
//! trust anchors from a bare public key, and a reader for the few fields
//! `cert` shows (names and dates).
//!
//! Real-world certificates are never written here, only verified (by rustls).

use ecm_ssh_crypto as c;

// ---------------------------------------------------------------- DER writer

fn len(n: usize, out: &mut Vec<u8>) {
    if n < 0x80 {
        out.push(n as u8);
    } else if n < 0x100 {
        out.extend_from_slice(&[0x81, n as u8]);
    } else {
        out.extend_from_slice(&[0x82, (n >> 8) as u8, n as u8]);
    }
}

/// One TLV.
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 4);
    out.push(tag);
    len(content.len(), &mut out);
    out.extend_from_slice(content);
    out
}

fn seq(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

const OID_ED25519: &[u8] = &[0x06, 0x03, 0x2B, 0x65, 0x70];
const OID_CN: &[u8] = &[0x06, 0x03, 0x55, 0x04, 0x03];
const OID_BASIC_CONSTRAINTS: &[u8] = &[0x06, 0x03, 0x55, 0x1D, 0x13];
const OID_KEY_USAGE: &[u8] = &[0x06, 0x03, 0x55, 0x1D, 0x0F];
const OID_SAN: &[u8] = &[0x06, 0x03, 0x55, 0x1D, 0x11];
const OID_EKU: &[u8] = &[0x06, 0x03, 0x55, 0x1D, 0x25];
const OID_SERVER_AUTH: &[u8] = &[0x06, 0x08, 0x2B, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01];
const TRUE: &[u8] = &[0x01, 0x01, 0xFF];

fn alg() -> Vec<u8> {
    seq(&[OID_ED25519])
}

/// The contents of a Name with one common name (no outer SEQUENCE), as a
/// trust anchor's subject is given.
pub fn name_body(common_name: &str) -> Vec<u8> {
    tlv(0x31, &seq(&[OID_CN, &tlv(0x0C, common_name.as_bytes())]))
}

fn name(common_name: &str) -> Vec<u8> {
    tlv(0x30, &name_body(common_name))
}

/// The contents of an ed25519 SubjectPublicKeyInfo (no outer SEQUENCE).
pub fn spki_body(public: &[u8; 32]) -> Vec<u8> {
    let mut bits = vec![0u8];
    bits.extend_from_slice(public);
    [alg(), tlv(0x03, &bits)].concat()
}

/// PKCS#8 for an ed25519 seed (what rustls loads as a private key).
pub fn pkcs8(seed: &[u8; 32]) -> Vec<u8> {
    let inner = tlv(0x04, seed);
    seq(&[&[0x02, 0x01, 0x00], &alg(), &tlv(0x04, &inner)])
}

/// The seed inside a PKCS#8 ed25519 key written by [`pkcs8`].
pub fn seed_of_pkcs8(der: &[u8]) -> Option<[u8; 32]> {
    let want = pkcs8(&[0u8; 32]);
    if der.len() != want.len() || der[..16] != want[..16] {
        return None;
    }
    der[16..].try_into().ok()
}

/// (year, month, day, hour, minute, second) of a Unix time (UTC).
pub fn civil(unix: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = unix / 86_400;
    let rem = unix % 86_400;
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y as u64, m as u64, d as u64, rem / 3_600, rem % 3_600 / 60, rem % 60)
}

/// Unix time of a UTC date.
pub fn unix_of(y: u64, m: u64, d: u64, hh: u64, mm: u64, ss: u64) -> u64 {
    let y = y as i64 - if m <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    ((era * 146_097 + doe - 719_468) as u64) * 86_400 + hh * 3_600 + mm * 60 + ss
}

/// `2026-11-26 14:03 UTC`.
pub fn format_time(unix: u64) -> String {
    let (y, m, d, hh, mm, _) = civil(unix);
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02} UTC")
}

fn time(unix: u64) -> Vec<u8> {
    let (y, m, d, hh, mm, ss) = civil(unix);
    if (1950..2050).contains(&y) {
        tlv(0x17, format!("{:02}{m:02}{d:02}{hh:02}{mm:02}{ss:02}Z", y % 100).as_bytes())
    } else {
        tlv(0x18, format!("{y:04}{m:02}{d:02}{hh:02}{mm:02}{ss:02}Z").as_bytes())
    }
}

fn extension(oid: &[u8], critical: bool, value: &[u8]) -> Vec<u8> {
    let v = tlv(0x04, value);
    if critical {
        seq(&[oid, TRUE, &v])
    } else {
        seq(&[oid, &v])
    }
}

/// A name a certificate is for: a DNS name or an IPv4 address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubjectName {
    Dns(String),
    Ip([u8; 4]),
}

impl SubjectName {
    /// `10.0.0.1` is an address, anything else a DNS name (lower-cased). None when it is neither.
    pub fn parse(s: &str) -> Option<SubjectName> {
        let s = s.trim().trim_end_matches('.').to_ascii_lowercase();
        if let Some(ip) = parse_ipv4(&s) {
            return Some(SubjectName::Ip(ip));
        }
        let ok = !s.is_empty()
            && s.len() <= 253
            && s.split('.').all(|l| {
                !l.is_empty()
                    && l.len() <= 63
                    && l.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                    && !l.starts_with('-')
                    && !l.ends_with('-')
            });
        ok.then_some(SubjectName::Dns(s))
    }
}

impl std::fmt::Display for SubjectName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubjectName::Dns(d) => write!(f, "{d}"),
            SubjectName::Ip(i) => write!(f, "{}.{}.{}.{}", i[0], i[1], i[2], i[3]),
        }
    }
}

pub fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut n = 0;
    for part in s.split('.') {
        if n == 4 || part.is_empty() || part.len() > 3 || !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        out[n] = part.parse().ok()?;
        n += 1;
    }
    (n == 4).then_some(out)
}

fn sign(tbs: Vec<u8>, issuer_seed: &[u8; 32]) -> Vec<u8> {
    let sig = c::ed25519_sign(&c::ed25519_private_key_from_bytes(issuer_seed), &tbs);
    let mut bits = vec![0u8];
    bits.extend_from_slice(&sig);
    seq(&[&tbs, &alg(), &tlv(0x03, &bits)])
}

fn tbs(serial: &[u8; 16], issuer: &str, subject: &str, public: &[u8; 32], not_before: u64, not_after: u64, extensions: &[Vec<u8>]) -> Vec<u8> {
    // A positive INTEGER: clear the top bit rather than grow a byte.
    let mut s = *serial;
    s[0] &= 0x7F;
    if s[0] == 0 {
        s[0] = 1;
    }
    seq(&[
        &tlv(0xA0, &[0x02, 0x01, 0x02]),
        &tlv(0x02, &s),
        &alg(),
        &name(issuer),
        &seq(&[&time(not_before), &time(not_after)]),
        &name(subject),
        &tlv(0x30, &spki_body(public)),
        &tlv(0xA3, &tlv(0x30, &extensions.concat())),
    ])
}

/// A self-signed root for the authority named `common_name`.
pub fn root_certificate(seed: &[u8; 32], common_name: &str, serial: &[u8; 16], not_before: u64, not_after: u64) -> Vec<u8> {
    let public = public_of(seed);
    let ext = [
        extension(OID_BASIC_CONSTRAINTS, true, &seq(&[TRUE])),
        extension(OID_KEY_USAGE, true, &[0x03, 0x02, 0x01, 0x06]),
    ];
    sign(tbs(serial, common_name, common_name, &public, not_before, not_after, &ext), seed)
}

/// A server certificate for `names` (the first is also the common name),
/// signed by the authority `issuer` with `issuer_seed`.
pub fn leaf_certificate(
    issuer_seed: &[u8; 32],
    issuer: &str,
    public: &[u8; 32],
    names: &[SubjectName],
    serial: &[u8; 16],
    not_before: u64,
    not_after: u64,
) -> Vec<u8> {
    let mut san = Vec::new();
    for n in names {
        match n {
            SubjectName::Dns(d) => san.extend(tlv(0x82, d.as_bytes())),
            SubjectName::Ip(i) => san.extend(tlv(0x87, i)),
        }
    }
    let ext = [
        extension(OID_BASIC_CONSTRAINTS, true, &seq(&[])),
        extension(OID_KEY_USAGE, true, &[0x03, 0x02, 0x07, 0x80]),
        extension(OID_EKU, false, &seq(&[OID_SERVER_AUTH])),
        extension(OID_SAN, false, &tlv(0x30, &san)),
    ];
    let cn = names.first().map(|n| n.to_string()).unwrap_or_default();
    sign(tbs(serial, issuer, &cn, public, not_before, not_after, &ext), issuer_seed)
}

pub fn public_of(seed: &[u8; 32]) -> [u8; 32] {
    c::ed25519_public_key_bytes(&c::ed25519_private_key_from_bytes(seed).verifying_key())
}

// ---------------------------------------------------------------- DER reader

/// (tag, contents, rest).
fn take(b: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, b) = b.split_first()?;
    let (&l0, b) = b.split_first()?;
    let (n, b) = if l0 < 0x80 {
        (l0 as usize, b)
    } else {
        let k = (l0 & 0x7F) as usize;
        if k == 0 || k > 3 || b.len() < k {
            return None;
        }
        (b[..k].iter().fold(0usize, |a, &x| (a << 8) | x as usize), &b[k..])
    };
    (b.len() >= n).then(|| (tag, &b[..n], &b[n..]))
}

fn common_name(name: &[u8]) -> String {
    let mut rest = name;
    while let Some((0x31, set, r)) = take(rest) {
        rest = r;
        if let Some((0x30, atv, _)) = take(set) {
            if atv.starts_with(OID_CN) {
                if let Some((_, v, _)) = take(&atv[OID_CN.len()..]) {
                    return String::from_utf8_lossy(v).into_owned();
                }
            }
        }
    }
    String::new()
}

fn read_time(tag: u8, v: &[u8]) -> Option<u64> {
    let s = std::str::from_utf8(v).ok()?;
    let n = |a: usize, b: usize| s.get(a..b)?.parse::<u64>().ok();
    let (y, o) = match tag {
        0x17 => {
            let y = n(0, 2)?;
            (if y >= 50 { 1900 + y } else { 2000 + y }, 2)
        }
        0x18 => (n(0, 4)?, 4),
        _ => return None,
    };
    Some(unix_of(y, n(o, o + 2)?, n(o + 2, o + 4)?, n(o + 4, o + 6)?, n(o + 6, o + 8)?, n(o + 8, o + 10)?))
}

/// What `cert` shows of a certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertInfo {
    pub subject: String,
    pub issuer: String,
    pub not_before: u64,
    pub not_after: u64,
    pub names: Vec<SubjectName>,
    pub is_ca: bool,
}

/// Read the subject, issuer, dates and names of a certificate (any issuer's, not just ours).
pub fn describe(der: &[u8]) -> Option<CertInfo> {
    let (0x30, cert, _) = take(der)? else { return None };
    let (0x30, mut t, _) = take(cert)? else { return None };
    if let Some((0xA0, _, r)) = take(t) {
        t = r;
    }
    let (_, _, t) = take(t)?; // serial
    let (_, _, t) = take(t)?; // signature algorithm
    let (0x30, issuer, t) = take(t)? else { return None };
    let (0x30, validity, t) = take(t)? else { return None };
    let (tag1, v1, v) = take(validity)?;
    let (tag2, v2, _) = take(v)?;
    let (0x30, subject, t) = take(t)? else { return None };
    let (_, _, mut t) = take(t)?; // SPKI
    let mut info = CertInfo {
        subject: common_name(subject),
        issuer: common_name(issuer),
        not_before: read_time(tag1, v1)?,
        not_after: read_time(tag2, v2)?,
        names: Vec::new(),
        is_ca: false,
    };
    while let Some((tag, body, r)) = take(t) {
        t = r;
        if tag != 0xA3 {
            continue;
        }
        let (0x30, mut exts, _) = take(body)? else { return None };
        while let Some((0x30, ext, r)) = take(exts) {
            exts = r;
            let (0x06, oid, mut e) = take(ext)? else { return None };
            if let Some((0x01, _, r)) = take(e) {
                e = r;
            }
            let (0x04, value, _) = take(e)? else { return None };
            if oid == &OID_SAN[2..] {
                let (0x30, mut names, _) = take(value)? else { return None };
                while let Some((tag, v, r)) = take(names) {
                    names = r;
                    match tag {
                        0x82 => info.names.push(SubjectName::Dns(String::from_utf8_lossy(v).into_owned())),
                        0x87 if v.len() == 4 => info.names.push(SubjectName::Ip([v[0], v[1], v[2], v[3]])),
                        _ => {}
                    }
                }
            } else if oid == &OID_BASIC_CONSTRAINTS[2..] {
                if let Some((0x30, bc, _)) = take(value) {
                    info.is_ca = bc.starts_with(TRUE);
                }
            }
        }
    }
    Some(info)
}

// ---------------------------------------------------------------- PEM

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for ch in data.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if ch.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if ch.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn unbase64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let v = B64.iter().position(|&b| b == c)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// `-----BEGIN <label>-----` block, 64 columns.
pub fn pem(label: &str, der: &[u8]) -> String {
    let b = base64(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in b.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).unwrap_or(""));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

/// Every block with `label` in `text`, in order.
pub fn pem_blocks(text: &str, label: &str) -> Vec<Vec<u8>> {
    let (begin, end) = (format!("-----BEGIN {label}-----"), format!("-----END {label}-----"));
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(&begin) {
        let after = &rest[i + begin.len()..];
        let Some(j) = after.find(&end) else { break };
        if let Some(der) = unbase64(&after[..j]) {
            out.push(der);
        }
        rest = &after[j + end.len()..];
    }
    out
}

pub const CERTIFICATE: &str = "CERTIFICATE";
pub const PRIVATE_KEY: &str = "PRIVATE KEY";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip() {
        for t in [0u64, 86_399, 951_782_400, 1_709_164_800, 1_795_000_000, 2_524_608_000, 4_102_444_800] {
            let (y, m, d, hh, mm, ss) = civil(t);
            assert_eq!(unix_of(y, m, d, hh, mm, ss), t, "{t}");
        }
        assert_eq!(civil(951_782_400), (2000, 2, 29, 0, 0, 0));
        assert_eq!(format_time(1_795_000_000), "2026-11-18 11:06 UTC");
        // UTCTime before 2050, GeneralizedTime after.
        assert_eq!(time(1_795_000_000)[0], 0x17);
        assert_eq!(time(4_102_444_800)[0], 0x18);
    }

    #[test]
    fn names() {
        assert_eq!(SubjectName::parse("Shop.ECM."), Some(SubjectName::Dns("shop.ecm".into())));
        assert_eq!(SubjectName::parse("100.65.0.21"), Some(SubjectName::Ip([100, 65, 0, 21])));
        for bad in ["", "a..b", "-a.ecm", "a_b.ecm", "1.2.3.4.5/x", "a b"] {
            assert_eq!(SubjectName::parse(bad), None, "{bad}");
        }
        assert_eq!(SubjectName::parse("1.2.3.999"), Some(SubjectName::Dns("1.2.3.999".into())));
    }

    #[test]
    fn certificates_describe_themselves() {
        let ca = [7u8; 32];
        let root = root_certificate(&ca, "Test Village CA", &[9; 16], 1_700_000_000, 4_102_444_800);
        let r = describe(&root).unwrap();
        assert_eq!((r.subject.as_str(), r.issuer.as_str(), r.is_ca), ("Test Village CA", "Test Village CA", true));
        assert_eq!((r.not_before, r.not_after), (1_700_000_000, 4_102_444_800));
        let leaf_key = [3u8; 32];
        let names = vec![SubjectName::Dns("shop.ecm".into()), SubjectName::Ip([10, 0, 7, 1])];
        let leaf = leaf_certificate(&ca, "Test Village CA", &public_of(&leaf_key), &names, &[0xFF; 16], 1_700_000_000, 1_707_776_000);
        let l = describe(&leaf).unwrap();
        assert_eq!((l.subject.as_str(), l.issuer.as_str(), l.is_ca), ("shop.ecm", "Test Village CA", false));
        assert_eq!(l.names, names);
        assert!(describe(&leaf[..leaf.len() - 5]).is_none());
        assert!(describe(b"junk").is_none());
    }

    #[test]
    fn pem_and_keys() {
        let der: Vec<u8> = (0u8..200).collect();
        let text = format!("junk\n{}{}", pem(CERTIFICATE, &der), pem(CERTIFICATE, &der[..7]));
        assert_eq!(pem_blocks(&text, CERTIFICATE), vec![der.clone(), der[..7].to_vec()]);
        assert!(pem_blocks(&text, PRIVATE_KEY).is_empty());
        assert_eq!(base64(b"hi"), "aGk=");
        assert_eq!(unbase64("aGk=").unwrap(), b"hi");
        let k = pkcs8(&[5u8; 32]);
        assert_eq!(k.len(), 48);
        assert_eq!(seed_of_pkcs8(&k), Some([5u8; 32]));
        assert_eq!(seed_of_pkcs8(&k[1..]), None);
    }
}
