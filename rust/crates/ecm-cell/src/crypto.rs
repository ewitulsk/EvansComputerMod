//! Key derivation and air-interface protection.
//!
//! All derivations are HMAC-SHA256 with ASCII labels (no terminator) and
//! big-endian integers:
//!
//! ```text
//! KS_enc = KS[0..16), KS_int = KS[16..32)                   (SimAuth v1)
//! K_AS   = HMAC(KS_enc, "ECM-CELL-AS"  || cell_id u32 || epoch u32)          32 bytes, ChaCha20-Poly1305 key
//! SMC    = HMAC(KS_int, "ECM-CELL-SMC" || RAND || cell_id || epoch || ue_addr u32)[0..16)
//! SR-MAC = HMAC(KS_int, "ECM-CELL-SR"  || tmsi u32 || count u32)[0..16)
//! SITE   = HMAC(site_secret, "ECM-SITE-AUTH" || nonce[16] || site_id)        32 bytes
//! ```
//!
//! `K_AS` is bound to one cell and one key epoch: the core hands a site only
//! the `K_AS` for its cell, never KS, and every attach / handover / resume uses
//! a fresh epoch, so frame counters restart at 0 without nonce reuse.
//!
//! A protected air frame is `[0x10][epoch u32][counter u32][ChaCha20-Poly1305(items)]`
//! with nonce `dir u8 || 0 0 0 || counter u64` (dir 1 = uplink, 2 = downlink) and
//! associated data `0x10 || epoch || counter || src u32 || dst u32 || dir`
//! (radio addresses: a frame can't be replayed to another radio). Receivers
//! keep a 64-frame replay window.

use alloc::vec::Vec;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::codec::R;

type HmacSha256 = Hmac<Sha256>;

pub fn hmac_sha256_parts(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = <HmacSha256 as Mac>::new_from_slice(key).expect("hmac takes any key length");
    for p in parts {
        m.update(p);
    }
    let r = m.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&r);
    out
}

/// Constant-time equality.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut d = 0u8;
    for (x, y) in a.iter().zip(b) {
        d |= x ^ y;
    }
    d == 0
}

pub fn as_key(ks_enc: &[u8; 16], cell_id: u32, epoch: u32) -> [u8; 32] {
    hmac_sha256_parts(ks_enc, &[b"ECM-CELL-AS", &cell_id.to_be_bytes(), &epoch.to_be_bytes()])
}

pub fn smc_mac(ks_int: &[u8; 16], rand: &[u8; 16], cell_id: u32, epoch: u32, ue_addr: u32) -> [u8; 16] {
    let h = hmac_sha256_parts(
        ks_int,
        &[b"ECM-CELL-SMC", rand, &cell_id.to_be_bytes(), &epoch.to_be_bytes(), &ue_addr.to_be_bytes()],
    );
    trunc16(&h)
}

pub fn sr_mac(ks_int: &[u8; 16], tmsi: u32, count: u32) -> [u8; 16] {
    trunc16(&hmac_sha256_parts(ks_int, &[b"ECM-CELL-SR", &tmsi.to_be_bytes(), &count.to_be_bytes()]))
}

pub fn site_proof(secret: &[u8; 16], nonce: &[u8; 16], site_id: &str) -> [u8; 32] {
    hmac_sha256_parts(secret, &[b"ECM-SITE-AUTH", nonce, site_id.as_bytes()])
}

fn trunc16(h: &[u8; 32]) -> [u8; 16] {
    let mut o = [0u8; 16];
    o.copy_from_slice(&h[..16]);
    o
}

pub const PROTECTED: u8 = 0x10;
/// Bytes a protected frame adds around its items.
pub const PROTECTED_OVERHEAD: usize = 1 + 4 + 4 + 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Up = 1,
    Down = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryptoError {
    Malformed,
    /// The frame names a key epoch we don't hold.
    Epoch,
    /// Authentication tag mismatch.
    Tag,
    /// Counter already seen or too old.
    Replay,
}

/// Sliding 64-frame replay window.
#[derive(Clone, Debug, Default)]
pub struct ReplayWindow {
    /// Highest counter accepted + 1 (0 = none yet).
    top: u64,
    bits: u64,
}

impl ReplayWindow {
    /// Would `c` be accepted?
    pub fn check(&self, c: u64) -> bool {
        if self.top == 0 || c >= self.top {
            return true;
        }
        let back = self.top - 1 - c;
        back < 64 && self.bits & (1 << back) == 0
    }
    pub fn accept(&mut self, c: u64) {
        if self.top == 0 || c >= self.top {
            let shift = if self.top == 0 { 64 } else { c + 1 - self.top };
            self.bits = if shift >= 64 { 0 } else { self.bits << shift };
            self.bits |= 1;
            self.top = c + 1;
        } else {
            let back = self.top - 1 - c;
            if back < 64 {
                self.bits |= 1 << back;
            }
        }
    }
}

/// One direction-pair of an air security context (`K_AS`, epoch).
#[derive(Clone)]
pub struct AirCipher {
    aead: ChaCha20Poly1305,
    pub epoch: u32,
    /// The direction this side sends in.
    tx_dir: Dir,
    tx_counter: u64,
    rx: ReplayWindow,
}

impl core::fmt::Debug for AirCipher {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "AirCipher {{ epoch: {}, tx: {:?}#{} }}", self.epoch, self.tx_dir, self.tx_counter)
    }
}

impl AirCipher {
    /// `ue` = this side is the phone (sends uplink).
    pub fn new(k_as: &[u8; 32], epoch: u32, ue: bool) -> AirCipher {
        AirCipher {
            aead: ChaCha20Poly1305::new(Key::from_slice(k_as)),
            epoch,
            tx_dir: if ue { Dir::Up } else { Dir::Down },
            tx_counter: 0,
            rx: ReplayWindow::default(),
        }
    }

    fn nonce(dir: Dir, counter: u64) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[0] = dir as u8;
        n[4..].copy_from_slice(&counter.to_be_bytes());
        n
    }

    fn aad(epoch: u32, counter: u32, src: u32, dst: u32, dir: Dir) -> [u8; 18] {
        let mut a = [0u8; 18];
        a[0] = PROTECTED;
        a[1..5].copy_from_slice(&epoch.to_be_bytes());
        a[5..9].copy_from_slice(&counter.to_be_bytes());
        a[9..13].copy_from_slice(&src.to_be_bytes());
        a[13..17].copy_from_slice(&dst.to_be_bytes());
        a[17] = dir as u8;
        a
    }

    /// Protect `items` (an encoded item list) sent from radio `src` to `dst`.
    pub fn seal(&mut self, src: u32, dst: u32, items: &[u8]) -> Vec<u8> {
        let c = self.tx_counter;
        self.tx_counter += 1;
        let c32 = c as u32;
        let aad = Self::aad(self.epoch, c32, src, dst, self.tx_dir);
        let ct = self
            .aead
            .encrypt(Nonce::from_slice(&Self::nonce(self.tx_dir, c32 as u64)), Payload { msg: items, aad: &aad })
            .expect("chacha20poly1305 encrypt");
        let mut out = Vec::with_capacity(9 + ct.len());
        out.push(PROTECTED);
        out.extend_from_slice(&self.epoch.to_be_bytes());
        out.extend_from_slice(&c32.to_be_bytes());
        out.extend_from_slice(&ct);
        out
    }

    /// The epoch a protected frame names (to pick a key), without checking it.
    pub fn frame_epoch(frame: &[u8]) -> Option<u32> {
        let mut r = R::new(frame);
        if r.u8().ok()? != PROTECTED {
            return None;
        }
        r.u32().ok()
    }

    /// Verify and decrypt a protected frame from `src` to `dst`.
    pub fn open(&mut self, src: u32, dst: u32, frame: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut r = R::new(frame);
        let t = r.u8().map_err(|_| CryptoError::Malformed)?;
        let epoch = r.u32().map_err(|_| CryptoError::Malformed)?;
        let c = r.u32().map_err(|_| CryptoError::Malformed)?;
        if t != PROTECTED || r.left() < 16 {
            return Err(CryptoError::Malformed);
        }
        if epoch != self.epoch {
            return Err(CryptoError::Epoch);
        }
        if !self.rx.check(c as u64) {
            return Err(CryptoError::Replay);
        }
        let rx_dir = if self.tx_dir == Dir::Up { Dir::Down } else { Dir::Up };
        let aad = Self::aad(epoch, c, src, dst, rx_dir);
        let pt = self
            .aead
            .decrypt(Nonce::from_slice(&Self::nonce(rx_dir, c as u64)), Payload { msg: r.rest(), aad: &aad })
            .map_err(|_| CryptoError::Tag)?;
        self.rx.accept(c as u64);
        Ok(pt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::hex;

    #[test]
    fn derivations_are_stable() {
        // Pinned so cored, celld-site and celld (and any reimplementation)
        // can check themselves against these.
        let ks_enc = [0x11u8; 16];
        let ks_int = [0x22u8; 16];
        assert_eq!(
            hex(&as_key(&ks_enc, 7, 1)),
            hex(&hmac_sha256_parts(&ks_enc, &[b"ECM-CELL-AS", &[0, 0, 0, 7], &[0, 0, 0, 1]]))
        );
        let m = smc_mac(&ks_int, &[3; 16], 7, 1, 0xA0);
        assert_eq!(m.len(), 16);
        assert_ne!(smc_mac(&ks_int, &[3; 16], 7, 2, 0xA0), m, "epoch is bound");
        assert_ne!(smc_mac(&ks_int, &[3; 16], 8, 1, 0xA0), m, "cell is bound");
        assert_ne!(as_key(&ks_enc, 7, 1), as_key(&ks_enc, 7, 2));
        assert_ne!(sr_mac(&ks_int, 5, 1), sr_mac(&ks_int, 5, 2));
        assert_ne!(site_proof(&[1; 16], &[2; 16], "a"), site_proof(&[1; 16], &[2; 16], "b"));
    }

    #[test]
    fn seal_open_both_directions() {
        let k = [9u8; 32];
        let mut ue = AirCipher::new(&k, 3, true);
        let mut bs = AirCipher::new(&k, 3, false);
        let f = ue.seal(0xA, 0xB, b"hello");
        assert_eq!(f.len(), 5 + PROTECTED_OVERHEAD);
        assert_eq!(AirCipher::frame_epoch(&f), Some(3));
        assert_eq!(bs.open(0xA, 0xB, &f).unwrap(), b"hello");
        assert_eq!(bs.open(0xA, 0xB, &f), Err(CryptoError::Replay), "replay refused");
        let g = bs.seal(0xB, 0xA, b"down");
        assert_eq!(ue.open(0xB, 0xA, &g).unwrap(), b"down");
        // uplink frame looped back to the UE: wrong direction -> tag fails
        let f2 = ue.seal(0xA, 0xB, b"x");
        let mut ue2 = AirCipher::new(&k, 3, true);
        assert_eq!(ue2.open(0xA, 0xB, &f2), Err(CryptoError::Tag));
        // addresses are bound
        let f3 = ue.seal(0xA, 0xB, b"y");
        assert_eq!(bs.open(0xC, 0xB, &f3), Err(CryptoError::Tag));
        // a flipped bit
        let mut f4 = ue.seal(0xA, 0xB, b"z");
        let n = f4.len();
        f4[n - 1] ^= 1;
        assert_eq!(bs.open(0xA, 0xB, &f4), Err(CryptoError::Tag));
        // another epoch
        let mut other = AirCipher::new(&k, 4, true);
        assert_eq!(bs.open(0xA, 0xB, &other.seal(0xA, 0xB, b"q")), Err(CryptoError::Epoch));
        assert_eq!(bs.open(0xA, 0xB, &[0x10, 0, 0]), Err(CryptoError::Malformed));
    }

    #[test]
    fn replay_window_reorder() {
        let mut w = ReplayWindow::default();
        for c in [5u64, 3, 4, 60, 6] {
            assert!(w.check(c), "{}", c);
            w.accept(c);
        }
        assert!(!w.check(3) && !w.check(60) && !w.check(5));
        assert!(w.check(59));
        w.accept(200);
        assert!(!w.check(100), "older than the window");
        assert!(w.check(199));
    }
}
