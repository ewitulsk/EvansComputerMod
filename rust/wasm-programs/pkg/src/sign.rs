//! ed25519 package signatures (keys and signatures as lower-case hex), on top of
//! `ecm-ssh-crypto`.

use crate::package::{hex, unhex};
use ecm_ssh_crypto as c;

/// (public key hex, secret seed hex) from a 32-byte seed.
pub fn keypair_from_seed(seed: &[u8; 32]) -> (String, String) {
    let sk = c::ed25519_private_key_from_bytes(seed);
    let pk = sk.verifying_key();
    (hex(&c::ed25519_public_key_bytes(&pk)), hex(seed))
}

/// A fresh random key pair.
pub fn generate() -> (String, String) {
    let (_, sk) = c::ed25519_generate_keypair();
    keypair_from_seed(&c::ed25519_private_key_bytes(&sk))
}

/// Public key hex of a secret seed hex.
pub fn public_of(secret_hex: &str) -> Option<String> {
    let seed: [u8; 32] = unhex(secret_hex)?.try_into().ok()?;
    Some(keypair_from_seed(&seed).0)
}

/// Signature hex. Panics on a malformed secret (callers load it with [`public_of`] first).
pub fn sign(secret_hex: &str, msg: &[u8]) -> String {
    let seed: [u8; 32] = unhex(secret_hex).and_then(|v| v.try_into().ok()).expect("bad secret key");
    hex(&c::ed25519_sign(&c::ed25519_private_key_from_bytes(&seed), msg))
}

pub fn verify(public_hex: &str, msg: &[u8], sig_hex: &str) -> bool {
    let Some(pk) = unhex(public_hex).and_then(|v| <[u8; 32]>::try_from(v).ok()) else { return false };
    let Some(sig) = unhex(sig_hex).and_then(|v| <[u8; 64]>::try_from(v).ok()) else { return false };
    match c::ed25519_public_key_from_bytes(&pk) {
        Ok(k) => c::ed25519_verify(&k, msg, &sig),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify() {
        let (pk, sk) = keypair_from_seed(&[3u8; 32]);
        assert_eq!(public_of(&sk).unwrap(), pk);
        let s = sign(&sk, b"msg");
        assert!(verify(&pk, b"msg", &s));
        assert!(!verify(&pk, b"msh", &s));
        assert!(!verify("zz", b"msg", &s));
    }
}
