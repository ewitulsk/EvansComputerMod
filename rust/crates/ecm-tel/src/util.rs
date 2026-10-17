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
    }
}
