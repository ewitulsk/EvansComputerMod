//! Hamming(7,4): single-error-correcting block code.
//!
//! Codeword bit layout (bit 0 = position 1): `p1 p2 d1 p3 d2 d3 d4`, so the
//! syndrome directly gives the 1-based error position.

/// Encode the low 4 bits of `nibble` into a 7-bit codeword.
pub fn encode(nibble: u8) -> u8 {
    let d = |i: u8| (nibble >> i) & 1;
    let (d1, d2, d3, d4) = (d(0), d(1), d(2), d(3));
    let p1 = d1 ^ d2 ^ d4;
    let p2 = d1 ^ d3 ^ d4;
    let p3 = d2 ^ d3 ^ d4;
    p1 | (p2 << 1) | (d1 << 2) | (p3 << 3) | (d2 << 4) | (d3 << 5) | (d4 << 6)
}

/// Decode a 7-bit codeword, correcting up to one bit error.
/// Returns `(nibble, corrected)`.
pub fn decode(code: u8) -> (u8, bool) {
    let b = |pos: u8| (code >> (pos - 1)) & 1;
    let s1 = b(1) ^ b(3) ^ b(5) ^ b(7);
    let s2 = b(2) ^ b(3) ^ b(6) ^ b(7);
    let s3 = b(4) ^ b(5) ^ b(6) ^ b(7);
    let syn = s1 | (s2 << 1) | (s3 << 2);
    let c = if syn != 0 { code ^ (1 << (syn - 1)) } else { code } & 0x7F;
    let g = |pos: u8| (c >> (pos - 1)) & 1;
    (g(3) | (g(5) << 1) | (g(6) << 2) | (g(7) << 3), syn != 0)
}

/// Encode bytes: two codewords per byte (low nibble first).
pub fn encode_bytes(data: &[u8]) -> Vec<u8> {
    data.iter().flat_map(|&b| [encode(b & 0xF), encode(b >> 4)]).collect()
}

/// Decode codewords produced by [`encode_bytes`]. Returns the data and the
/// number of corrected codewords.
pub fn decode_bytes(codes: &[u8]) -> (Vec<u8>, usize) {
    let mut fixed = 0;
    let out = codes
        .chunks(2)
        .map(|c| {
            let (lo, a) = decode(c[0]);
            let (hi, b) = if c.len() > 1 { decode(c[1]) } else { (0, false) };
            fixed += a as usize + b as usize;
            lo | (hi << 4)
        })
        .collect();
    (out, fixed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrects_every_single_bit_error() {
        for n in 0..16u8 {
            let c = encode(n);
            assert_eq!(decode(c), (n, false));
            for bit in 0..7 {
                assert_eq!(decode(c ^ (1 << bit)), (n, true));
            }
        }
        let (d, f) = decode_bytes(&encode_bytes(b"radio"));
        assert_eq!((d.as_slice(), f), (&b"radio"[..], 0));
        let mut cw = encode_bytes(b"xyz");
        cw[1] ^= 0x10;
        cw[4] ^= 0x01;
        let (d, f) = decode_bytes(&cw);
        assert_eq!((d.as_slice(), f), (&b"xyz"[..], 2));
    }
}
