//! The wire format: frames, the two handshake messages, and the encrypted channel.
//!
//! Every message is a **frame**: a u32 little-endian length, then that many payload
//! bytes (at most [`MAX_FRAME`]).
//!
//! 1. Client to server, plain: `ClientHello` = `0x01, version, ce (32)`, where `ce` is
//!    the client's ephemeral X25519 public key.
//! 2. Server to client, plain: `ServerHello` = `0x02, version, se (32), realm_pub (32),
//!    kdf_iterations (u32 LE), name length (u8), realm name (UTF-8), sig (64)`, where
//!    `sig` is the **realm key**'s ed25519 signature over the transcript hash
//!    `th = SHA-256("ecm-auth-hs1" || ce || se || realm_pub || u32 LE len(name) || name ||
//!    u32 LE kdf_iterations)`.
//! 3. Both sides compute `dh = X25519(ce, se)` and the direction keys
//!    `prk = HMAC-SHA256(key = th, dh)`, `k_c2s = HMAC-SHA256(prk, "ecm-auth c2s")`,
//!    `k_s2c = HMAC-SHA256(prk, "ecm-auth s2c")`. Every later frame's payload is
//!    ChaCha20-Poly1305 under its direction's key with the nonce `u64 LE counter || 4 zero
//!    bytes`; the counter starts at 0 in each direction and counts frames, so a replayed,
//!    dropped or reordered frame fails to decrypt and ends the connection.
//!
//! The encrypted frames carry one UTF-8 line each (no newline), words separated by
//! spaces, binary values in lowercase hex; a free-text field (a display name, a
//! message) is always last:
//!
//! | Client to server | |
//! |---|---|
//! | `LOGIN <username> <session_pub> <sig>` | `sig`: account key over `"ecm-auth-login1" \|\| th` |
//! | `SIGNUP <username> <account_pub> <session_pub> <sig> <display name...>` | `sig`: the new account key over `"ecm-auth-signup1" \|\| th` (proof of possession) |
//! | `REFRESH <username> <session_pub> <refresh_token>` | sign in with a saved refresh token |
//! | `TOKEN <audience fp>` | a token for one backend (after signing in) |
//! | `PASSWD <new_pub> <new_sig> <old_sig>` | `new_sig`: new key over `"ecm-auth-passwd1" \|\| th`; `old_sig`: old key over `"ecm-auth-passwd1" \|\| th \|\| new_pub` |
//! | `WHOAMI` / `LOGOUT` | who this session is / revoke this session's refresh token |
//!
//! | Server to client | |
//! |---|---|
//! | `WELCOME <user_id> <username> <refresh_token> <refresh_expires_ms> <display name...>` | signed in |
//! | `FAIL <code> [arg]` | refused ([`crate::AuthError::from_fail`]): `auth`, `locked <s>`, `taken <user>`, `username <why>`, `display <why>`, `disabled`, `reset`, `expired`, `signin`, `other <text>` |
//! | `TOKEN <expires_ms> <token>` | the token ([`crate::token`]) |
//! | `ME <user_id> <username> <display name...>` | answer to `WHOAMI` |
//! | `OK` / `BYE` | `PASSWD` done / signed out (the server closes) |
//!
//! `session_pub` is a fresh ed25519 key the client made for this sign-in; tokens are
//! bound to it (`cnf`). A relay can forward 1 and 2 but can't read anything after them,
//! and can't swap in its own `ce`: the realm's signature, and the account signature
//! inside the channel, both cover `th`.

use crate::AuthError;
use ecm_ssh_crypto::{hmac_sha256, sha256_multi};

/// Protocol version in both hellos.
pub const VERSION: u8 = 1;
/// Largest frame payload.
pub const MAX_FRAME: usize = 64 * 1024;
pub const CLIENT_HELLO: u8 = 1;
pub const SERVER_HELLO: u8 = 2;

pub const HS_LABEL: &[u8] = b"ecm-auth-hs1";
pub const LOGIN_LABEL: &[u8] = b"ecm-auth-login1";
pub const SIGNUP_LABEL: &[u8] = b"ecm-auth-signup1";
pub const PASSWD_LABEL: &[u8] = b"ecm-auth-passwd1";

/// A frame: u32 LE length, then the payload.
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + payload.len());
    v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

/// Collects bytes from a stream and cuts them into frames.
#[derive(Default)]
pub struct FrameReader {
    buf: Vec<u8>,
}

impl FrameReader {
    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// The next whole frame, if one has arrived. An oversized length is an error.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, AuthError> {
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let n = u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
        if n > MAX_FRAME {
            return Err(AuthError::Protocol(format!("frame of {n} bytes")));
        }
        if self.buf.len() < 4 + n {
            return Ok(None);
        }
        let payload = self.buf[4..4 + n].to_vec();
        self.buf.drain(..4 + n);
        Ok(Some(payload))
    }
}

/// The client's first message.
pub fn client_hello(ce: &[u8; 32]) -> Vec<u8> {
    let mut v = vec![CLIENT_HELLO, VERSION];
    v.extend_from_slice(ce);
    v
}

pub fn parse_client_hello(p: &[u8]) -> Result<[u8; 32], AuthError> {
    if p.len() != 34 || p[0] != CLIENT_HELLO {
        return Err(AuthError::Protocol("not a client hello".into()));
    }
    if p[1] != VERSION {
        return Err(AuthError::Protocol(format!("client speaks version {}", p[1])));
    }
    Ok(p[2..34].try_into().unwrap())
}

/// The server's answer, as sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerHello {
    pub se: [u8; 32],
    pub realm_pub: [u8; 32],
    pub kdf_iterations: u32,
    pub realm_name: String,
    pub sig: [u8; 64],
}

impl ServerHello {
    pub fn encode(&self) -> Vec<u8> {
        let name = self.realm_name.as_bytes();
        let mut v = vec![SERVER_HELLO, VERSION];
        v.extend_from_slice(&self.se);
        v.extend_from_slice(&self.realm_pub);
        v.extend_from_slice(&self.kdf_iterations.to_le_bytes());
        v.push(name.len().min(255) as u8);
        v.extend_from_slice(&name[..name.len().min(255)]);
        v.extend_from_slice(&self.sig);
        v
    }

    pub fn parse(p: &[u8]) -> Result<ServerHello, AuthError> {
        let bad = || AuthError::Protocol("not an auth server (bad hello)".into());
        if p.len() < 2 + 32 + 32 + 4 + 1 + 64 || p[0] != SERVER_HELLO {
            return Err(bad());
        }
        if p[1] != VERSION {
            return Err(AuthError::Protocol(format!("server speaks version {}", p[1])));
        }
        let n = p[70] as usize;
        if p.len() != 71 + n + 64 {
            return Err(bad());
        }
        Ok(ServerHello {
            se: p[2..34].try_into().unwrap(),
            realm_pub: p[34..66].try_into().unwrap(),
            kdf_iterations: u32::from_le_bytes(p[66..70].try_into().unwrap()),
            realm_name: String::from_utf8(p[71..71 + n].to_vec()).map_err(|_| bad())?,
            sig: p[71 + n..].try_into().unwrap(),
        })
    }
}

/// The transcript hash both sides sign.
pub fn transcript(ce: &[u8; 32], se: &[u8; 32], realm_pub: &[u8; 32], realm_name: &str, kdf_iterations: u32) -> [u8; 32] {
    let name = realm_name.as_bytes();
    sha256_multi(&[HS_LABEL, ce, se, realm_pub, &(name.len() as u32).to_le_bytes(), name, &kdf_iterations.to_le_bytes()])
}

/// The encrypted channel after the handshake (one per side).
pub struct Channel {
    send_key: [u8; 32],
    recv_key: [u8; 32],
    send_n: u64,
    recv_n: u64,
}

impl Channel {
    /// Keys from the DH output and the transcript hash; `client` picks the directions.
    pub fn new(dh: &[u8; 32], th: &[u8; 32], client: bool) -> Result<Channel, AuthError> {
        if dh.iter().all(|b| *b == 0) {
            // A low-order public key: no secret at all.
            return Err(AuthError::Protocol("bad key exchange".into()));
        }
        let prk = hmac_sha256(th, dh);
        let c2s = hmac_sha256(&prk, b"ecm-auth c2s");
        let s2c = hmac_sha256(&prk, b"ecm-auth s2c");
        let (send_key, recv_key) = if client { (c2s, s2c) } else { (s2c, c2s) };
        Ok(Channel { send_key, recv_key, send_n: 0, recv_n: 0 })
    }

    fn nonce(n: u64) -> [u8; 12] {
        let mut v = [0u8; 12];
        v[..8].copy_from_slice(&n.to_le_bytes());
        v
    }

    /// Encrypt one message into a whole frame.
    pub fn seal(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let ct = ecm_ssh_crypto::chacha20_poly1305_encrypt(&self.send_key, &Self::nonce(self.send_n), plaintext).expect("encrypt");
        self.send_n += 1;
        frame(&ct)
    }

    /// Decrypt the next frame's payload (in order; anything else fails).
    pub fn open(&mut self, payload: &[u8]) -> Result<Vec<u8>, AuthError> {
        let pt = ecm_ssh_crypto::chacha20_poly1305_decrypt(&self.recv_key, &Self::nonce(self.recv_n), payload)
            .map_err(|_| AuthError::Protocol("a frame failed to decrypt (tampered, replayed or out of order)".into()))?;
        self.recv_n += 1;
        Ok(pt)
    }
}

/// Split off the first word.
pub fn word(s: &str) -> (&str, &str) {
    match s.find(' ') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    }
}
