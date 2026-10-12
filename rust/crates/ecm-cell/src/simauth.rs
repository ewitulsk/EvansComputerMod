//! SimAuth version 1 (CELLULAR_V2.md "Key derivation"), the [`Sim`] trait the
//! UE asks for responses, and identity validation shared with the SIM card.
//!
//! ```text
//! RES = HMAC-SHA256(K, "ECM-SIM-RES" || RAND)[0..16)
//! KS  = HMAC-SHA256(K, "ECM-SIM-KS"  || RAND)   KS[0..16) encryption, KS[16..32) integrity
//! ```
//! The real SIM computes this on the server behind the phone's SIM slot or a
//! `card_reader` (`respond(challenge_hex)`) and never exposes K; the core
//! computes the same from its database copy of K.

use alloc::string::String;
use core::fmt;

use crate::crypto::hmac_sha256_parts;

pub const KEY_LEN: usize = 16;
pub const RAND_LEN: usize = 16;
pub const RES_LEN: usize = 16;
pub const KS_LEN: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SimResponse {
    pub res: [u8; RES_LEN],
    pub ks: [u8; KS_LEN],
}

impl fmt::Debug for SimResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print key material.
        write!(f, "SimResponse {{ res: {}, ks: <redacted> }}", crate::codec::hex(&self.res))
    }
}

impl SimResponse {
    /// KS[0..16): the encryption half.
    pub fn ks_enc(&self) -> [u8; 16] {
        let mut k = [0u8; 16];
        k.copy_from_slice(&self.ks[..16]);
        k
    }
    /// KS[16..32): the integrity half.
    pub fn ks_int(&self) -> [u8; 16] {
        let mut k = [0u8; 16];
        k.copy_from_slice(&self.ks[16..]);
        k
    }
}

/// SimAuth v1 for key `k` and challenge `rand`.
pub fn simauth_v1(k: &[u8; KEY_LEN], rand: &[u8; RAND_LEN]) -> SimResponse {
    let r = hmac_sha256_parts(k, &[b"ECM-SIM-RES", rand]);
    let ks = hmac_sha256_parts(k, &[b"ECM-SIM-KS", rand]);
    let mut res = [0u8; RES_LEN];
    res.copy_from_slice(&r[..RES_LEN]);
    SimResponse { res, ks }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimIdentity {
    pub carrier: String,
    pub imsi: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimError {
    /// No card in the slot / reader.
    NoCard,
    /// A blank card (never written).
    Blank,
    /// The card cannot answer (key unknown on this side, reader error).
    Failed,
}

/// What the UE needs from its SIM. In game this is the phone's SIM slot or a
/// `card_reader` peripheral (`identity()`, `respond(challenge_hex)`).
pub trait Sim {
    fn identity(&self) -> Result<SimIdentity, SimError>;
    fn respond(&mut self, rand: &[u8; RAND_LEN]) -> Result<SimResponse, SimError>;
}

/// A SIM whose key is known to this program: tests and the simulator. Real
/// phones use the smart card.
#[derive(Clone)]
pub struct SoftSim {
    pub identity: SimIdentity,
    key: [u8; KEY_LEN],
}

impl SoftSim {
    pub fn new(carrier: &str, imsi: &str, key: [u8; KEY_LEN]) -> SoftSim {
        SoftSim { identity: SimIdentity { carrier: carrier.into(), imsi: imsi.into() }, key }
    }
}

impl Sim for SoftSim {
    fn identity(&self) -> Result<SimIdentity, SimError> {
        Ok(self.identity.clone())
    }
    fn respond(&mut self, rand: &[u8; RAND_LEN]) -> Result<SimResponse, SimError> {
        Ok(simauth_v1(&self.key, rand))
    }
}

/// IMSI: 6-15 ASCII digits (as `SmartSim.write` checks).
pub fn valid_imsi(s: &str) -> bool {
    (6..=15).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_digit())
}

/// Carrier id: `[a-z0-9_-]{1,24}` (as `SmartSim.write` checks).
pub fn valid_carrier(s: &str) -> bool {
    (1..=24).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{hex, unhex};

    /// The vectors in CELLULAR_V2.md (checked by the Java `SimCardTest` too).
    #[test]
    fn simauth_v1_vectors() {
        let cases = [
            (
                "00112233445566778899aabbccddeeff",
                "000102030405060708090a0b0c0d0e0f",
                "4c455ba887affefa1448bea727baa584",
                "3a2295de88d4ea04764491a508856d342f71ed47a7415c5bffeabd934308713a",
            ),
            (
                "00112233445566778899aabbccddeeff",
                "ffffffffffffffffffffffffffffffff",
                "3e316dff329533c535f8b5106208b75c",
                "4531753cba8d31b05e36c2c9fd04a80f1baac7d541c8a580b21b2cd17f9cf09e",
            ),
            (
                "00000000000000000000000000000000",
                "00000000000000000000000000000000",
                "7fbf4baffaedda01afa5c9856ba7a9f4",
                "027c68857ae22afa9558fa8d747477918e9a90cb23146cb02617eeafabb9dd23",
            ),
        ];
        for (k, rand, res, ks) in cases {
            let r = simauth_v1(&unhex::<16>(k).unwrap(), &unhex::<16>(rand).unwrap());
            assert_eq!(hex(&r.res), res, "RES for K={} RAND={}", k, rand);
            assert_eq!(hex(&r.ks), ks, "KS for K={} RAND={}", k, rand);
            assert_eq!(hex(&r.ks_enc()), &ks[..32]);
            assert_eq!(hex(&r.ks_int()), &ks[32..]);
        }
    }

    #[test]
    fn debug_hides_ks() {
        let r = simauth_v1(&[0; 16], &[0; 16]);
        let s = alloc::format!("{:?}", r);
        assert!(!s.contains("027c6885"), "{}", s);
    }

    #[test]
    fn identity_rules() {
        assert!(valid_imsi("310150000000001"));
        assert!(!valid_imsi("31015"));
        assert!(!valid_imsi("3101500000000012"));
        assert!(!valid_imsi("31015000000000a"));
        assert!(valid_carrier("cell-red"));
        assert!(valid_carrier("a_1"));
        assert!(!valid_carrier("Cell"));
        assert!(!valid_carrier(""));
        assert!(!valid_carrier("abcdefghijklmnopqrstuvwxy"));
    }
}
