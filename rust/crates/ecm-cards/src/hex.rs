//! Lower-case hex, the way every card peripheral passes bytes.

use alloc::string::String;
use alloc::vec::Vec;

const DIGITS: &[u8; 16] = b"0123456789abcdef";

pub fn encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 15) as usize] as char);
    }
    s
}

fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// `None` for an odd length or a non-hex character.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let b = text.as_bytes();
    if b.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(b.len() / 2);
    for pair in b.chunks_exact(2) {
        out.push((nibble(pair[0])? << 4) | nibble(pair[1])?);
    }
    Some(out)
}

/// Decode exactly `N` bytes.
pub fn decode_array<const N: usize>(text: &str) -> Option<[u8; N]> {
    let v = decode(text)?;
    if v.len() != N {
        return None;
    }
    let mut out = [0u8; N];
    out.copy_from_slice(&v);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        assert_eq!(encode(&[0, 15, 16, 255]), "000f10ff");
        assert_eq!(decode("000F10ff").unwrap(), [0, 15, 16, 255]);
        assert!(decode("abc").is_none());
        assert!(decode("zz").is_none());
        assert_eq!(decode_array::<2>("0102"), Some([1, 2]));
        assert_eq!(decode_array::<3>("0102"), None);
    }
}
