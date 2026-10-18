//! Password to account key.
//!
//! `seed = PBKDF2-HMAC-SHA256(password, salt = "ecm-auth-v1" || realm_pub || lowercase(username),
//! iterations, 32 bytes)`, and the account key is the ed25519 key of that seed. The realm key
//! is in the salt, so the same password gives an unrelated key on every realm: a spoofed
//! realm (another key) learns nothing it can use at the real one.
//!
//! The realm announces its iteration count in the signed handshake; it is stored with the
//! realm's users because changing it would change every account key.
//!
//! ## Choosing [`DEFAULT_KDF_ITERATIONS`]
//!
//! The target is about 0.3 s per sign-in on an ePhone. One iteration is two SHA-256
//! compressions (the keyed inner and outer HMAC states are computed once and cloned).
//! Measured natively (`cargo test --release -p ecm-auth kdf_timing -- --nocapture`, a
//! desktop x86-64): 30,000 iterations took 2.4 ms and 50,000 took 5.7 ms (0.08-0.11 us
//! per iteration), but that
//! machine's `sha2` uses the CPU's SHA instructions, which WASM doesn't have; plain
//! software SHA-256 is roughly 5x slower (about 0.4 us per iteration), and the debug
//! build measured 5 us. WASM on a computer runs maybe 5-20x slower than native software
//! code, so 50,000 iterations cost about 0.1-0.4 s there: the target, at the top of
//! the 20,000-50,000 range, because a phone sign-in happens rarely (refresh tokens
//! cover the rest) while an offline guesser pays for every guess. Clients refuse fewer
//! than [`MIN_KDF_ITERATIONS`] (a weak realm would make stolen public keys easy to
//! crack) and more than [`MAX_KDF_ITERATIONS`] (a hostile realm could freeze the phone).

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::{signing_key, SigningKey};

type HmacSha256 = Hmac<Sha256>;

pub const DEFAULT_KDF_ITERATIONS: u32 = 50_000;
pub const MIN_KDF_ITERATIONS: u32 = 4_096;
pub const MAX_KDF_ITERATIONS: u32 = 250_000;

/// The salt prefix (domain and version).
pub const SALT_PREFIX: &[u8] = b"ecm-auth-v1";

/// PBKDF2-HMAC-SHA256 (RFC 8018) of any output length.
pub fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32, out: &mut [u8]) {
    // The keyed state (password through ipad and opad) is made once; each round clones it,
    // so a round costs two compressions instead of four.
    let keyed = <HmacSha256 as Mac>::new_from_slice(password).expect("HMAC takes any key length");
    for (i, block) in out.chunks_mut(32).enumerate() {
        let mut mac = keyed.clone();
        mac.update(salt);
        mac.update(&(i as u32 + 1).to_be_bytes());
        let mut u: [u8; 32] = mac.finalize().into_bytes().into();
        let mut t = u;
        for _ in 1..iterations.max(1) {
            let mut mac = keyed.clone();
            mac.update(&u);
            u = mac.finalize().into_bytes().into();
            for (a, b) in t.iter_mut().zip(u.iter()) {
                *a ^= b;
            }
        }
        block.copy_from_slice(&t[..block.len()]);
    }
}

/// The account key's seed for `password` at the realm `realm_pub` (username compared in lower case).
pub fn account_seed(password: &str, realm_pub: &[u8; 32], username: &str, iterations: u32) -> [u8; 32] {
    let salt = [SALT_PREFIX, realm_pub, username.trim().to_ascii_lowercase().as_bytes()].concat();
    let mut seed = [0u8; 32];
    pbkdf2_hmac_sha256(password.as_bytes(), &salt, iterations, &mut seed);
    seed
}

/// The account key for `password` at the realm `realm_pub`.
pub fn account_key(password: &str, realm_pub: &[u8; 32], username: &str, iterations: u32) -> SigningKey {
    signing_key(&account_seed(password, realm_pub, username, iterations))
}

/// Whether a client accepts a realm's iteration count.
pub fn acceptable(iterations: u32) -> bool {
    (MIN_KDF_ITERATIONS..=MAX_KDF_ITERATIONS).contains(&iterations)
}
