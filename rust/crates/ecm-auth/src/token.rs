//! Tokens, and how a backend checks them.
//!
//! `ecmtok1.<base64url payload>.<base64url sig>`: `sig` is the realm key's ed25519
//! signature over the ASCII text `ecmtok1.<base64url payload>` (what is on the wire, so
//! there's nothing to canonicalise). The payload is UTF-8 `key=value` lines in this order:
//!
//! ```text
//! v=1
//! realm=<realm fp>
//! uid=<user id>
//! user=<username>
//! name=<display name>
//! aud=<audience: the backend's own key fingerprint>
//! cnf=<session public key, hex>
//! iat=<issued, ms since the epoch, realm clock>
//! exp=<expires, ms>
//! ```
//!
//! A backend ([`BackendGate`]) accepts a token only if the realm it trusts signed it, it
//! names this backend, it hasn't expired, and the client signs the backend's fresh
//! connect nonce with the session key ([`prove`]): a token sniffed or taken from one
//! backend is useless at another, and useless without the session key.

use crate::{b64url, fingerprint, hex, sign, unb64url, unhex_n, verify, AuthError, SigningKey, CLOCK_SKEW_MS};

pub const PREFIX: &str = "ecmtok1.";
pub const PROOF_LABEL: &[u8] = b"ecm-auth-proof1";

/// What a token says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claims {
    pub realm_fp: String,
    pub user_id: u64,
    pub username: String,
    pub display_name: String,
    pub audience: String,
    /// The session public key the client must prove.
    pub cnf: [u8; 32],
    pub issued_ms: u64,
    pub expires_ms: u64,
}

impl Claims {
    fn payload(&self) -> String {
        format!(
            "v=1\nrealm={}\nuid={}\nuser={}\nname={}\naud={}\ncnf={}\niat={}\nexp={}",
            self.realm_fp,
            self.user_id,
            self.username,
            self.display_name,
            self.audience,
            hex(&self.cnf),
            self.issued_ms,
            self.expires_ms
        )
    }

    fn parse_payload(text: &str) -> Option<Claims> {
        let mut lines = text.split('\n');
        let mut field = |key: &str| lines.next()?.strip_prefix(key)?.strip_prefix('=').map(str::to_string);
        if field("v")? != "1" {
            return None;
        }
        Some(Claims {
            realm_fp: field("realm")?,
            user_id: field("uid")?.parse().ok()?,
            username: field("user")?,
            display_name: field("name")?,
            audience: field("aud")?,
            cnf: unhex_n(&field("cnf")?)?,
            issued_ms: field("iat")?.parse().ok()?,
            expires_ms: field("exp")?.parse().ok()?,
        })
    }

    /// The signed token (by the realm key).
    pub fn mint(&self, realm_key: &SigningKey) -> String {
        let head = format!("{PREFIX}{}", b64url(self.payload().as_bytes()));
        let sig = sign(realm_key, &[head.as_bytes()]);
        format!("{head}.{}", b64url(&sig))
    }

    /// Read a token **without checking it** (for showing who a token is for).
    pub fn peek(token: &str) -> Result<Claims, AuthError> {
        split(token).map(|(_, c, _)| c)
    }
}

fn split(token: &str) -> Result<(&str, Claims, [u8; 64]), AuthError> {
    let rest = token.trim().strip_prefix(PREFIX).ok_or(AuthError::TokenMalformed)?;
    let (payload, sig) = rest.split_once('.').ok_or(AuthError::TokenMalformed)?;
    let head = &token.trim()[..PREFIX.len() + payload.len()];
    let text = String::from_utf8(unb64url(payload).ok_or(AuthError::TokenMalformed)?).map_err(|_| AuthError::TokenMalformed)?;
    let claims = Claims::parse_payload(&text).ok_or(AuthError::TokenMalformed)?;
    let sig: [u8; 64] = unb64url(sig).and_then(|s| s.try_into().ok()).ok_or(AuthError::TokenMalformed)?;
    Ok((head, claims, sig))
}

/// The client's proof for a backend: the session key's signature over
/// `"ecm-auth-proof1" || nonce || audience fp (ASCII)`.
pub fn prove(session_key: &SigningKey, nonce: &[u8; 32], audience_fp: &str) -> [u8; 64] {
    sign(session_key, &[PROOF_LABEL, nonce, audience_fp.as_bytes()])
}

/// A backend's door: the realm it trusts and its own identity.
pub struct BackendGate {
    pub realm_pub: [u8; 32],
    /// The backend's own public key; its fingerprint is the audience tokens must name.
    pub own_key: [u8; 32],
}

impl BackendGate {
    pub fn new(realm_pub: [u8; 32], own_key: [u8; 32]) -> BackendGate {
        BackendGate { realm_pub, own_key }
    }

    /// This backend's fingerprint (what clients ask a token for).
    pub fn audience(&self) -> String {
        fingerprint(&self.own_key)
    }

    /// A fresh random nonce for a new connection; the client signs it with [`prove`].
    pub fn challenge(&self) -> [u8; 32] {
        crate::random()
    }

    /// Accept `token` if the trusted realm signed it, it is for this backend, it hasn't
    /// expired at `now_ms` (with [`CLOCK_SKEW_MS`] allowed), and `proof` is the session
    /// key's signature for this connection's `nonce`.
    pub fn check(&self, token: &str, proof: &[u8], nonce: &[u8; 32], now_ms: u64) -> Result<Claims, AuthError> {
        let (head, claims, sig) = split(token)?;
        if !verify(&self.realm_pub, &[head.as_bytes()], &sig) || claims.realm_fp != fingerprint(&self.realm_pub) {
            return Err(AuthError::TokenSignature);
        }
        let audience = self.audience();
        if claims.audience != audience {
            return Err(AuthError::TokenAudience);
        }
        if now_ms > claims.expires_ms + CLOCK_SKEW_MS || claims.issued_ms > now_ms + CLOCK_SKEW_MS {
            return Err(AuthError::TokenExpired);
        }
        let proof: [u8; 64] = proof.try_into().map_err(|_| AuthError::TokenProof)?;
        if !verify(&claims.cnf, &[PROOF_LABEL, nonce, audience.as_bytes()], &proof) {
            return Err(AuthError::TokenProof);
        }
        Ok(claims)
    }
}
