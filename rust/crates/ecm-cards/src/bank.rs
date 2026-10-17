//! The bank chip's "Keyed Ledger Response" (NFC_MIX.md "Bank TAG", "PIN hash").
//!
//! ```text
//! TAG      = HMAC-SHA256(K, "ECM-BANK-TAG" || NONCE[8] || CTR u64 BE || AMOUNT u64 BE)[0..16)
//! pin_hash = SHA-256("ECM-BANK-PIN" || UID[8] || PIN ASCII)
//! ```
//!
//! The card increments CTR *before* computing TAG; an issuer verifying with
//! [`verify_tag`] also rejects any CTR it has seen or that moved backwards.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

pub const KEY_BYTES: usize = 32;
pub const NONCE_BYTES: usize = 8;
pub const TAG_BYTES: usize = 16;
pub const TAG_LABEL: &[u8] = b"ECM-BANK-TAG";
pub const PIN_LABEL: &[u8] = b"ECM-BANK-PIN";

pub fn tag(k: &[u8; 32], nonce: &[u8; 8], ctr: u64, amount: u64) -> [u8; 16] {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(k).expect("hmac takes any key length");
    m.update(TAG_LABEL);
    m.update(nonce);
    m.update(&ctr.to_be_bytes());
    m.update(&amount.to_be_bytes());
    let full = m.finalize().into_bytes();
    let mut out = [0u8; 16];
    out.copy_from_slice(&full[..16]);
    out
}

pub fn pin_hash(uid: &[u8; 8], pin: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(PIN_LABEL);
    h.update(uid);
    h.update(pin.as_bytes());
    h.finalize().into()
}

/// Constant-time check of a card's `tag` against the issuer's copy of K.
pub fn verify_tag(k: &[u8; 32], nonce: &[u8; 8], ctr: u64, amount: u64, tag_bytes: &[u8]) -> bool {
    let want = tag(k, nonce, ctr, amount);
    if tag_bytes.len() != want.len() {
        return false;
    }
    let mut d = 0u8;
    for (a, b) in want.iter().zip(tag_bytes) {
        d |= a ^ b;
    }
    d == 0
}

/// A PIN is 4 to 8 ASCII digits.
pub fn valid_pin(pin: &str) -> bool {
    (4..=8).contains(&pin.len()) && pin.bytes().all(|c| c.is_ascii_digit())
}

/// A PAN is 8 to 19 digits.
pub fn valid_pan(pan: &str) -> bool {
    (8..=19).contains(&pan.len()) && pan.bytes().all(|c| c.is_ascii_digit())
}

/// An issuer is `[a-z0-9_-]{1,24}`.
pub fn valid_issuer(issuer: &str) -> bool {
    (1..=24).contains(&issuer.len()) && issuer.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hex;

    const K1: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    /// NFC_MIX.md "Bank TAG" vectors.
    #[test]
    fn tag_vectors() {
        for (k, nonce, ctr, amount, want) in [
            (K1, "0001020304050607", 1u64, 0u64, "aef6052d8232ad7c4db312596726e55a"),
            (K1, "0001020304050607", 2, 1500, "ce2e7e5ad8d1e0b8bd42aa2b8903e5b7"),
            (&"00".repeat(32), "0000000000000000", 0, 0, "c1e9e0459b7a23ecb6d54dc7772a25af"),
            (&"ff".repeat(32), "ffffffffffffffff", 9223372036854775807, 1099511627776, "6987930a133eede5b3b6de9ef58f0e91"),
        ] {
            let k = hex::decode_array::<32>(k).unwrap();
            let n = hex::decode_array::<8>(nonce).unwrap();
            assert_eq!(hex::encode(&tag(&k, &n, ctr, amount)), want, "TAG ctr={ctr} amount={amount}");
            assert!(verify_tag(&k, &n, ctr, amount, &hex::decode(want).unwrap()));
            assert!(!verify_tag(&k, &n, ctr + 1, amount, &hex::decode(want).unwrap()));
            assert!(!verify_tag(&k, &n, ctr, amount, &hex::decode(want).unwrap()[..15]));
        }
    }

    /// NFC_MIX.md "PIN hash" vectors.
    #[test]
    fn pin_hash_vectors() {
        for (uid, pin, want) in [
            ("0102030405060708", "1234", "282fbfbe3f85ce96542b0435cc442d28745c372aaacf04b6229e657998169fd2"),
            ("0000000000000000", "00000000", "72a0521796bf77ccf576d8f5554d61b5a5cd371fad7492f3b64514b7e2a4cb19"),
        ] {
            let u = hex::decode_array::<8>(uid).unwrap();
            assert_eq!(hex::encode(&pin_hash(&u, pin)), want, "pin_hash({uid}, {pin})");
        }
    }

    #[test]
    fn validation() {
        assert!(valid_pin("1234") && valid_pin("12345678"));
        assert!(!valid_pin("123") && !valid_pin("123456789") && !valid_pin("12a4"));
        assert!(valid_pan("12345678") && !valid_pan("1234567") && !valid_pan(&"1".repeat(20)));
        assert!(valid_issuer("cell-red_1") && !valid_issuer("Bank") && !valid_issuer("") && !valid_issuer(&"a".repeat(25)));
    }
}
