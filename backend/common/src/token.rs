//! Opaque tokens. Only their SHA-256 is stored; lookups and comparisons go through the hash.

use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const API_KEY: &str = "ecm_k_";
pub const DEVICE: &str = "ecm_d_";
pub const SESSION: &str = "ecm_s_";

/// `n` random bytes from the operating system, as hex.
pub fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::rngs::OsRng.fill_bytes(&mut b);
    crate::hex(&b)
}

/// A new token: the prefix and 256 random bits.
pub fn generate(prefix: &str) -> String {
    format!("{prefix}{}", random_hex(32))
}

/// What is stored for a token.
pub fn hash(token: &str) -> String {
    crate::hex(&Sha256::digest(token.as_bytes()))
}

/// Compare two secrets without leaking where they differ (both are hashed first).
pub fn same(a: &str, b: &str) -> bool {
    Sha256::digest(a.as_bytes()).ct_eq(&Sha256::digest(b.as_bytes())).into()
}

/// A random string of `n` characters from `alphabet` (unbiased).
pub fn random_from(alphabet: &[u8], n: usize) -> String {
    let limit = 256 - 256 % alphabet.len();
    let mut out = String::with_capacity(n);
    let mut b = [0u8; 1];
    while out.len() < n {
        rand::rngs::OsRng.fill_bytes(&mut b);
        if (b[0] as usize) < limit {
            out.push(alphabet[b[0] as usize % alphabet.len()] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        let t = generate(API_KEY);
        assert!(t.starts_with("ecm_k_") && t.len() == 6 + 64);
        assert_ne!(t, generate(API_KEY));
        assert_eq!(hash("a"), "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb");
        assert!(same("x", "x") && !same("x", "y") && !same("x", ""));
        let c = random_from(b"ABC", 50);
        assert!(c.len() == 50 && c.bytes().all(|b| b"ABC".contains(&b)));
    }
}
