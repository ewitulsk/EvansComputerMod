//! Small shared helpers.

/// Lower-case hex.
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

/// Hex to bytes (None on odd length or a bad digit).
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// Constant-time string comparison (for proofs and digests).
pub fn ct_eq_str(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 (RFC 4648, with padding).
pub fn base64_encode(b: &[u8]) -> String {
    let mut o = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        o.push(B64[(n >> 18) as usize & 63] as char);
        o.push(B64[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    o
}

/// Base64 back to bytes (whitespace ignored; None on a bad character or length).
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let v: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if v.len() % 4 != 0 {
        return None;
    }
    let val = |c: u8| -> Option<u32> { B64.iter().position(|x| *x == c).map(|p| p as u32) };
    let mut o = Vec::with_capacity(v.len() / 4 * 3);
    for q in v.chunks(4) {
        let pad = q.iter().rev().take_while(|c| **c == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut n = 0u32;
        for (i, c) in q.iter().enumerate() {
            let x = if i >= 4 - pad { 0 } else { val(*c)? };
            n = n << 6 | x;
        }
        o.push((n >> 16) as u8);
        if pad < 2 {
            o.push((n >> 8) as u8);
        }
        if pad < 1 {
            o.push(n as u8);
        }
    }
    Some(o)
}

/// Current-ish wall time in seconds from milliseconds, for CDR stamps.
pub fn secs(ms: u64) -> u64 {
    ms / 1000
}

#[cfg(test)]
mod tests {
    #[test]
    fn hex_round_trip() {
        assert_eq!(super::unhex(&super::hex(&[0, 1, 0xab])).unwrap(), vec![0, 1, 0xab]);
        assert!(super::unhex("abc").is_none());
        assert!(super::unhex("zz").is_none());
        assert!(super::ct_eq_str("ab", "ab") && !super::ct_eq_str("ab", "ac") && !super::ct_eq_str("a", "ab"));
        for t in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            let e = super::base64_encode(t.as_bytes());
            assert_eq!(super::base64_decode(&e).unwrap(), t.as_bytes());
        }
        assert_eq!(super::base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(super::base64_encode(b"fo"), "Zm8=");
        assert!(super::base64_decode("Zm9").is_none());
        assert!(super::base64_decode("Zm9!").is_none());
    }
}
