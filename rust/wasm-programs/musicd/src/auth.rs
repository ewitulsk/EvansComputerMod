//! Who may stream and change playlists: tokens from the trusted realm, made
//! for this backend, proven with the token's session key.
//!
//! The client asks `GET /nonce` for a fresh nonce (single use, good for
//! [`NONCE_MS`]), signs it with `ecm_auth::prove(session_key, nonce,
//! backend_fp)` and sends `Authorization: Bearer <token>`, `X-ECM-Nonce:
//! <hex>` and `X-ECM-Proof: <hex>`. A connection that proved a token keeps it
//! (the many `Range` reads of one song need no new nonce), and the same
//! token + nonce + proof from the same address is accepted again for
//! [`REUSE_MS`], so a reconnecting stream stays cheap. A token sniffed off
//! the network is useless without the session key, and a token made for
//! another backend is refused by its audience.

use ecm_auth::{fingerprint, unhex, unhex_n, AuthError, BackendGate, Claims, CLOCK_SKEW_MS};
use std::collections::HashMap;

pub const NONCE_MS: u64 = 60_000;
pub const REUSE_MS: u64 = 600_000;
const MAX_NONCES: usize = 4096;
const MAX_REUSE: usize = 4096;

/// Why a request isn't signed in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthFail {
    /// No `Authorization: Bearer` header.
    Required,
    /// A token but no `X-ECM-Nonce` / `X-ECM-Proof`.
    ProofRequired,
    /// The nonce wasn't issued here, was used already, or is too old.
    NonceUnknown,
    /// `--auth HOST:PORT` and the realm's public key isn't known yet.
    RealmUnavailable,
    Token(AuthError),
}

impl AuthFail {
    pub fn status(&self) -> u16 {
        match self {
            AuthFail::RealmUnavailable => 503,
            _ => 401,
        }
    }

    /// The `code` field of the JSON error.
    pub fn code(&self) -> &'static str {
        match self {
            AuthFail::Required => "signin_required",
            AuthFail::ProofRequired => "proof_required",
            AuthFail::NonceUnknown => "nonce_unknown",
            AuthFail::RealmUnavailable => "realm_unavailable",
            AuthFail::Token(AuthError::TokenSignature) => "token_signature",
            AuthFail::Token(AuthError::TokenAudience) => "token_audience",
            AuthFail::Token(AuthError::TokenExpired) => "token_expired",
            AuthFail::Token(AuthError::TokenProof) => "token_proof",
            AuthFail::Token(_) => "token_malformed",
        }
    }

    pub fn message(&self) -> String {
        match self {
            AuthFail::Required => "sign in first".into(),
            AuthFail::ProofRequired => "the token needs X-ECM-Nonce and X-ECM-Proof (GET /nonce)".into(),
            AuthFail::NonceUnknown => "unknown or used nonce: GET /nonce for a new one".into(),
            AuthFail::RealmUnavailable => "the accounts server hasn't been reached yet: try again soon".into(),
            AuthFail::Token(e) => e.to_string(),
        }
    }
}

pub struct Gatekeeper {
    /// This backend's public key (its fingerprint is the audience).
    pub own_pub: [u8; 32],
    pub realm_fp: String,
    realm_pub: Option<[u8; 32]>,
    nonces: HashMap<String, u64>,
    reuse: HashMap<String, (Claims, u64)>,
}

impl Gatekeeper {
    pub fn new(own_pub: [u8; 32], realm_fp: &str, realm_pub: Option<[u8; 32]>) -> Gatekeeper {
        Gatekeeper { own_pub, realm_fp: realm_fp.to_string(), realm_pub, nonces: HashMap::new(), reuse: HashMap::new() }
    }

    pub fn audience(&self) -> String {
        fingerprint(&self.own_pub)
    }

    pub fn realm_pub(&self) -> Option<[u8; 32]> {
        self.realm_pub
    }

    /// Learn the realm's public key (only one with the pinned fingerprint).
    pub fn set_realm_pub(&mut self, public: [u8; 32]) -> bool {
        if fingerprint(&public) != self.realm_fp {
            return false;
        }
        self.realm_pub = Some(public);
        true
    }

    /// A new nonce (hex) from 32 random bytes.
    pub fn issue_nonce(&mut self, random: [u8; 32], now: u64) -> String {
        if self.nonces.len() >= MAX_NONCES {
            self.nonces.retain(|_, t| now.saturating_sub(*t) < NONCE_MS);
            if self.nonces.len() >= MAX_NONCES {
                // Under a flood: the oldest go.
                let mut v: Vec<(String, u64)> = self.nonces.drain().collect();
                v.sort_by_key(|e| std::cmp::Reverse(e.1));
                v.truncate(MAX_NONCES / 2);
                self.nonces = v.into_iter().collect();
            }
        }
        let n = ecm_auth::hex(&random);
        self.nonces.insert(n.clone(), now);
        n
    }

    /// Check a token with its proof for `nonce`, from `peer_ip`.
    pub fn verify(&mut self, token: &str, nonce: &str, proof: &str, peer_ip: &str, now: u64) -> Result<Claims, AuthFail> {
        let realm_pub = self.realm_pub.ok_or(AuthFail::RealmUnavailable)?;
        let nonce = nonce.trim().to_ascii_lowercase();
        let proof = proof.trim().to_ascii_lowercase();
        let key = format!("{peer_ip}\n{token}\n{nonce}\n{proof}");
        if let Some((c, at)) = self.reuse.get(&key) {
            if now.saturating_sub(*at) < REUSE_MS && now <= c.expires_ms + CLOCK_SKEW_MS {
                return Ok(c.clone());
            }
            self.reuse.remove(&key);
        }
        let issued = self.nonces.remove(&nonce).ok_or(AuthFail::NonceUnknown)?;
        if now.saturating_sub(issued) >= NONCE_MS {
            return Err(AuthFail::NonceUnknown);
        }
        let n: [u8; 32] = unhex_n(&nonce).ok_or(AuthFail::NonceUnknown)?;
        let p = unhex(&proof).ok_or(AuthFail::Token(AuthError::TokenProof))?;
        let claims = BackendGate::new(realm_pub, self.own_pub).check(token, &p, &n, now).map_err(AuthFail::Token)?;
        if self.reuse.len() >= MAX_REUSE {
            self.reuse.retain(|_, (_, at)| now.saturating_sub(*at) < REUSE_MS);
            if self.reuse.len() >= MAX_REUSE {
                self.reuse.clear();
            }
        }
        self.reuse.insert(key, (claims.clone(), now));
        Ok(claims)
    }
}
