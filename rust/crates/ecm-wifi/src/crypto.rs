//! WPA2 cryptography (IEEE 802.11-2020 §12.7 and §12.5.3).
//!
//! * PMK = PBKDF2-HMAC-SHA1(passphrase, SSID, 4096, 256 bits)
//! * PRF-n (HMAC-SHA1 based, §12.7.1.2) and the pairwise key hierarchy
//!   (PTK = PRF-384 split into KCK / KEK / TK for CCMP)
//! * EAPOL-Key MIC (HMAC-SHA1-128, key descriptor version 2)
//! * AES key wrap (RFC 3394) for the GTK in message 3 / group message 1
//! * CCMP: the exact 802.11 nonce and AAD construction around AES-CCM (M=8, L=2)
//! * Per-TID packet-number replay counters
//!
//! Everything is pure; no randomness is drawn here.

use aes::Aes128;
use ccm::aead::generic_array::GenericArray;
use ccm::aead::{AeadInPlace, KeyInit};
use ccm::consts::{U13, U8};
use ccm::Ccm;
use hmac::{Hmac, Mac as _};
use sha1::Sha1;

use crate::frame::{self, MacAddr};

type HmacSha1 = Hmac<Sha1>;
type Ccmp128 = Ccm<Aes128, U8, U13>;

/// Length of the CCMP header (PN0 PN1 rsvd keyid PN2..PN5).
pub const CCMP_HDR_LEN: usize = 8;
/// Length of the CCMP MIC.
pub const CCMP_MIC_LEN: usize = 8;

/// Derive the 256-bit PMK from an ASCII passphrase (8..=63 chars) and SSID.
pub fn pmk_from_passphrase(passphrase: &[u8], ssid: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha1>(passphrase, ssid, 4096, &mut out);
    out
}

/// HMAC-SHA1 over the concatenation of `parts`.
pub fn hmac_sha1(key: &[u8], parts: &[&[u8]]) -> [u8; 20] {
    let mut mac = <HmacSha1 as hmac::Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    for p in parts {
        mac.update(p);
    }
    mac.finalize().into_bytes().into()
}

/// The 802.11 PRF: `PRF(K, A, B, len)` = first `len` bytes of
/// `HMAC-SHA1(K, A || 0 || B || i)` for i = 0, 1, 2, ...
pub fn prf(key: &[u8], label: &[u8], data: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 20);
    let mut i: u8 = 0;
    while out.len() < len {
        out.extend_from_slice(&hmac_sha1(key, &[label, &[0u8], data, &[i]]));
        i = i.wrapping_add(1);
    }
    out.truncate(len);
    out
}

/// Pairwise transient key for CCMP (PRF-384): KCK ‖ KEK ‖ TK.
#[derive(Clone, PartialEq, Eq)]
pub struct Ptk {
    pub kck: [u8; 16],
    pub kek: [u8; 16],
    pub tk: [u8; 16],
}

impl core::fmt::Debug for Ptk {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Ptk{..}")
    }
}

impl Ptk {
    /// Split 48 bytes of PRF output.
    pub fn from_bytes(b: &[u8]) -> Ptk {
        let mut p = Ptk { kck: [0; 16], kek: [0; 16], tk: [0; 16] };
        p.kck.copy_from_slice(&b[0..16]);
        p.kek.copy_from_slice(&b[16..32]);
        p.tk.copy_from_slice(&b[32..48]);
        p
    }
}

/// PTK = PRF-384(PMK, "Pairwise key expansion",
///               Min(AA,SPA) ‖ Max(AA,SPA) ‖ Min(ANonce,SNonce) ‖ Max(ANonce,SNonce)).
pub fn derive_ptk(pmk: &[u8; 32], aa: &MacAddr, spa: &MacAddr, anonce: &[u8; 32], snonce: &[u8; 32]) -> Ptk {
    let mut data = Vec::with_capacity(76);
    let (a1, a2) = if aa <= spa { (aa, spa) } else { (spa, aa) };
    data.extend_from_slice(a1);
    data.extend_from_slice(a2);
    let (n1, n2) = if anonce <= snonce { (anonce, snonce) } else { (snonce, anonce) };
    data.extend_from_slice(n1);
    data.extend_from_slice(n2);
    Ptk::from_bytes(&prf(pmk, b"Pairwise key expansion", &data, 48))
}

/// EAPOL-Key MIC for key descriptor version 2: HMAC-SHA1 truncated to 128 bits,
/// computed over the whole EAPOL frame with the MIC field zeroed.
pub fn eapol_mic(kck: &[u8; 16], eapol_frame_mic_zeroed: &[u8]) -> [u8; 16] {
    let full = hmac_sha1(kck, &[eapol_frame_mic_zeroed]);
    let mut out = [0u8; 16];
    out.copy_from_slice(&full[..16]);
    out
}

/// Constant-time equality for MIC checks.
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

/// RFC 3394 AES key wrap with a 128-bit KEK. `data` must be a multiple of 8, ≥ 16 bytes.
pub fn aes_wrap(kek: &[u8; 16], data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 16 || data.len() % 8 != 0 {
        return None;
    }
    aes_kw::KekAes128::from(*kek).wrap_vec(data).ok()
}

/// RFC 3394 AES key unwrap with a 128-bit KEK; `None` if the integrity check fails.
pub fn aes_unwrap(kek: &[u8; 16], data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 24 || data.len() % 8 != 0 {
        return None;
    }
    aes_kw::KekAes128::from(*kek).unwrap_vec(data).ok()
}

/// AES key wrap with a 256-bit KEK (used only by the RFC 3394 vectors / future suites).
pub fn aes_wrap_256(kek: &[u8; 32], data: &[u8]) -> Option<Vec<u8>> {
    aes_kw::KekAes256::from(*kek).wrap_vec(data).ok()
}

/// AES key unwrap with a 256-bit KEK.
pub fn aes_unwrap_256(kek: &[u8; 32], data: &[u8]) -> Option<Vec<u8>> {
    aes_kw::KekAes256::from(*kek).unwrap_vec(data).ok()
}

// ---------------------------------------------------------------------------
// CCMP
// ---------------------------------------------------------------------------

/// Build the CCMP header for a 48-bit PN and key id (ExtIV always set).
pub fn ccmp_header(pn: u64, key_id: u8) -> [u8; 8] {
    [
        pn as u8,
        (pn >> 8) as u8,
        0,
        0x20 | ((key_id & 3) << 6),
        (pn >> 16) as u8,
        (pn >> 24) as u8,
        (pn >> 32) as u8,
        (pn >> 40) as u8,
    ]
}

/// Parse a CCMP header: (PN, key id). `None` if ExtIV is clear.
pub fn parse_ccmp_header(h: &[u8]) -> Option<(u64, u8)> {
    if h.len() < 8 || h[3] & 0x20 == 0 {
        return None;
    }
    let pn = h[0] as u64
        | (h[1] as u64) << 8
        | (h[4] as u64) << 16
        | (h[5] as u64) << 24
        | (h[6] as u64) << 32
        | (h[7] as u64) << 40;
    Some((pn, h[3] >> 6))
}

/// CCM nonce: Nonce Flags (priority, management bit) ‖ A2 ‖ PN5..PN0.
pub fn ccmp_nonce(hdr: &[u8], pn: u64) -> [u8; 13] {
    let fc0 = hdr[0];
    let ftype = (fc0 >> 2) & 3;
    let mut flags = 0u8;
    if ftype == frame::TYPE_MGMT {
        flags |= 0x10;
    } else if let Some(qc) = frame::qos_control(hdr) {
        flags |= (qc & 0x0f) as u8;
    }
    let mut n = [0u8; 13];
    n[0] = flags;
    n[1..7].copy_from_slice(&hdr[10..16]);
    for i in 0..6 {
        n[7 + i] = (pn >> (8 * (5 - i))) as u8;
    }
    n
}

/// CCM additional authentication data built from the MAC header (§12.5.3.3.3):
/// FC with subtype bits 4..6 (data frames), Retry, PwrMgt and MoreData masked and
/// Protected set; Order masked for QoS data; A1–A3; SC with the sequence number masked;
/// A4 if present; QC with only the TID (and A-MSDU present bit 7) kept.
pub fn ccmp_aad(hdr: &[u8]) -> Vec<u8> {
    let hlen = frame::header_len(hdr).unwrap_or(24).min(hdr.len());
    let mut aad = Vec::with_capacity(30);
    let ftype = (hdr[0] >> 2) & 3;
    let is_qos = frame::qos_control(hdr).is_some();
    let mut fc0 = hdr[0];
    if ftype == frame::TYPE_DATA {
        fc0 &= !0x70; // subtype bits b4..b6
    }
    let mut fc1 = hdr[1] & !(0x08 | 0x10 | 0x20); // Retry, PwrMgt, MoreData
    fc1 |= 0x40; // Protected
    if is_qos {
        fc1 &= !0x80; // Order
    }
    aad.push(fc0);
    aad.push(fc1);
    aad.extend_from_slice(&hdr[4..22]); // A1 A2 A3
    aad.push(hdr[22] & 0x0f); // fragment number only
    aad.push(0);
    let mut off = 24;
    if frame::has_addr4(hdr) {
        aad.extend_from_slice(&hdr[24..30]);
        off = 30;
    }
    if is_qos && hlen >= off + 2 {
        aad.push(hdr[off] & 0x8f);
        aad.push(0);
    }
    aad
}

/// Encrypt an MPDU in place semantics: input is the plaintext MPDU (header + body, no FCS,
/// Protected bit may be clear); output is header (Protected set) ‖ CCMP hdr ‖ ciphertext ‖ MIC.
pub fn ccmp_encrypt(tk: &[u8; 16], mpdu: &[u8], pn: u64, key_id: u8) -> Option<Vec<u8>> {
    let hlen = frame::header_len(mpdu)?;
    if mpdu.len() < hlen {
        return None;
    }
    let mut hdr = mpdu[..hlen].to_vec();
    hdr[1] |= 0x40;
    let nonce = ccmp_nonce(&hdr, pn);
    let aad = ccmp_aad(&hdr);
    let mut body = mpdu[hlen..].to_vec();
    let cipher = Ccmp128::new(GenericArray::from_slice(tk));
    let tag = cipher
        .encrypt_in_place_detached(GenericArray::from_slice(&nonce), &aad, &mut body)
        .ok()?;
    let mut out = Vec::with_capacity(hlen + 8 + body.len() + 8);
    out.extend_from_slice(&hdr);
    out.extend_from_slice(&ccmp_header(pn, key_id));
    out.extend_from_slice(&body);
    out.extend_from_slice(&tag);
    Some(out)
}

/// Result of a successful CCMP decryption.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decrypted {
    pub pn: u64,
    pub key_id: u8,
    /// Plaintext MPDU: header with Protected cleared, followed by the decrypted body.
    pub mpdu: Vec<u8>,
}

/// Read the CCMP PN and key id of a protected MPDU without decrypting it.
pub fn ccmp_peek(mpdu: &[u8]) -> Option<(u64, u8)> {
    let hlen = frame::header_len(mpdu)?;
    parse_ccmp_header(mpdu.get(hlen..hlen + 8)?)
}

/// Decrypt and authenticate a protected MPDU (no FCS). `None` on MIC failure / malformed.
pub fn ccmp_decrypt(tk: &[u8; 16], mpdu: &[u8]) -> Option<Decrypted> {
    let hlen = frame::header_len(mpdu)?;
    if mpdu.len() < hlen + CCMP_HDR_LEN + CCMP_MIC_LEN || mpdu[1] & 0x40 == 0 {
        return None;
    }
    let hdr = &mpdu[..hlen];
    let (pn, key_id) = parse_ccmp_header(&mpdu[hlen..hlen + 8])?;
    let nonce = ccmp_nonce(hdr, pn);
    let aad = ccmp_aad(hdr);
    let body_end = mpdu.len() - CCMP_MIC_LEN;
    let mut body = mpdu[hlen + 8..body_end].to_vec();
    let tag = &mpdu[body_end..];
    let cipher = Ccmp128::new(GenericArray::from_slice(tk));
    cipher
        .decrypt_in_place_detached(
            GenericArray::from_slice(&nonce),
            &aad,
            &mut body,
            GenericArray::from_slice(tag),
        )
        .ok()?;
    let mut out = Vec::with_capacity(hlen + body.len());
    out.extend_from_slice(hdr);
    out[1] &= !0x40;
    out.extend_from_slice(&body);
    Some(Decrypted { pn, key_id, mpdu: out })
}

/// Receive replay counters: one per TID (0..16) plus one for management frames (index 16).
/// A PN is accepted only if strictly greater than the last accepted PN for its counter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplayCounters {
    last: [Option<u64>; 17],
}

impl ReplayCounters {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start every counter at `rsc` (e.g. the GTK RSC from message 3, the last PN the
    /// authenticator used): a received PN must exceed it, as in mac80211.
    pub fn with_start(rsc: u64) -> Self {
        ReplayCounters { last: [Some(rsc); 17] }
    }

    /// Would `pn` be accepted on counter `idx`?
    pub fn check(&self, idx: usize, pn: u64) -> bool {
        match self.last[idx.min(16)] {
            Some(l) => pn > l,
            None => true,
        }
    }

    /// Record an accepted PN (call only after the MIC verified).
    pub fn update(&mut self, idx: usize, pn: u64) {
        self.last[idx.min(16)] = Some(pn);
    }

    /// Check and record in one step. Returns false for a replay.
    pub fn accept(&mut self, idx: usize, pn: u64) -> bool {
        if !self.check(idx, pn) {
            return false;
        }
        self.update(idx, pn);
        true
    }
}

/// Transmit packet number: 48-bit, monotonically increasing, never reused for one key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TxPn(u64);

impl TxPn {
    pub fn new() -> Self {
        TxPn(0)
    }
    /// Next PN to use (starts at 1). `None` once the 48-bit space is exhausted (rekey needed).
    pub fn next(&mut self) -> Option<u64> {
        if self.0 >= (1u64 << 48) - 1 {
            return None;
        }
        self.0 += 1;
        Some(self.0)
    }
    pub fn current(&self) -> u64 {
        self.0
    }
}
