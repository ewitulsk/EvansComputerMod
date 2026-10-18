//! `ecm_auth`: accounts for ECM backends (plan: `plans/background-apps/MUSIC_AND_AUTH.md`,
//! "Accounts and the Auth server").
//!
//! Discovery answers can be forged by anyone on the network, and `tcpdump` is in the mod,
//! so the design never trusts a name or an address and never sends a password:
//!
//! - A **realm** (an `authd`, or a backend running its own auth) is its ed25519 **realm
//!   key**. Its [`fingerprint`] is what players see and what apps pin ([`store`]).
//! - The client turns the password into an ed25519 **account key** with PBKDF2 salted by
//!   the realm key ([`kdf::account_key`]); the realm stores only the public key, and the
//!   same password gives unrelated keys on every realm.
//! - A **handshake** ([`wire`]) signed by the realm key over the transcript sets up an
//!   encrypted channel; the account key signs the same transcript, so a relay can neither
//!   read the answer nor swap in its own key exchange.
//! - **Tokens** ([`token`]) are signed by the realm key, name one backend (the audience)
//!   and are bound to a session key the client proves at each backend.
//!
//! The crate is plain logic: [`server`] is sans-IO (bytes in, bytes out), [`client`]
//! runs over a small blocking [`client::Stream`] trait, and [`store`] has the text formats
//! of the pins and saved logins without touching files. `host` (feature `host`) has the
//! socket and file adapters for programs on a computer.

pub mod client;
pub mod kdf;
pub mod server;
pub mod store;
pub mod token;
pub mod wire;

#[cfg(feature = "host")]
pub mod host;

#[cfg(test)]
mod tests;

pub use client::{handshake, ServerInfo, Session, Stream, Welcome};
pub use kdf::{account_key, account_seed, DEFAULT_KDF_ITERATIONS, MAX_KDF_ITERATIONS, MIN_KDF_ITERATIONS};
pub use server::{ConnOut, Realm, ServerConn, User};
pub use store::{AccountStore, Files, Login, MemFiles, Pin, PinCheck, Pins};
pub use token::{prove, BackendGate, Claims};

pub use ecm_ssh_crypto::{Ed25519SigningKey as SigningKey, Ed25519VerifyingKey as VerifyingKey};

/// TCP port `authd` listens on by default.
pub const DEFAULT_PORT: u16 = 7600;
/// Longest username.
pub const MAX_USERNAME: usize = 16;
/// Longest display name (and realm name).
pub const MAX_DISPLAY_NAME: usize = 32;
/// How long a token is good for.
pub const TOKEN_LIFETIME_MS: u64 = 3_600_000;
/// How long a refresh token is good for (renewed each time it is used).
pub const REFRESH_LIFETIME_MS: u64 = 30 * 24 * 3_600_000;
/// Clock difference allowed between a realm and a backend when checking a token.
pub const CLOCK_SKEW_MS: u64 = 120_000;

/// Everything that can go wrong, with messages a player can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// The username doesn't exist or the key (password) is wrong. Deliberately one error.
    WrongPassword,
    /// Too many failed sign-ins for this username or address.
    Locked { wait_s: u64 },
    UsernameTaken(String),
    BadUsername(String),
    BadDisplayName(String),
    BadPassword,
    /// The account was disabled by the realm's operator.
    Disabled,
    /// The operator reset the account: the next sign-in sets a new password.
    NeedsNewPassword,
    /// The refresh token is unknown, revoked or expired: sign in with the password again.
    SignInExpired,
    /// No saved login (for this realm, or at all).
    NotSignedIn(String),
    NoSuchUser(String),
    /// A realm with the same name or address now has another key. Refused until the
    /// player removes the old pin by hand.
    RealmChanged { name: String, host: String, old_fp: String, new_fp: String },
    /// The server's handshake was not signed by the realm key it presented.
    NotTheRealm,
    /// The realm asked for a PBKDF2 iteration count outside what clients accept.
    BadKdf(u32),
    TokenMalformed,
    /// Not signed by the realm this backend trusts.
    TokenSignature,
    /// Made for another backend.
    TokenAudience,
    TokenExpired,
    /// The client couldn't prove it holds the token's session key.
    TokenProof,
    /// The other side broke the protocol.
    Protocol(String),
    Network(String),
    Timeout,
    Closed,
    /// A refusal from the server this client doesn't know (`FAIL <code> ...`).
    Server(String),
    /// Saving or reading the logins failed.
    Storage(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::WrongPassword => write!(f, "wrong username or password"),
            AuthError::Locked { wait_s } => write!(f, "too many tries: wait {wait_s} s"),
            AuthError::UsernameTaken(u) => write!(f, "the username {u} is taken"),
            AuthError::BadUsername(why) => write!(f, "bad username: {why}"),
            AuthError::BadDisplayName(why) => write!(f, "bad display name: {why}"),
            AuthError::BadPassword => write!(f, "the password can't be empty"),
            AuthError::Disabled => write!(f, "this account is disabled"),
            AuthError::NeedsNewPassword => write!(f, "this account was reset: sign up again with a new password"),
            AuthError::SignInExpired => write!(f, "your sign-in has expired: sign in again"),
            AuthError::NotSignedIn(what) => write!(f, "not signed in to {what}"),
            AuthError::NoSuchUser(u) => write!(f, "no user {u}"),
            AuthError::RealmChanged { name, host, old_fp, new_fp } => write!(
                f,
                "the identity of {name} has changed. This could be someone pretending to be it. \
                 ({host}: pinned {}, now {}; remove the old pin by hand only if you know why it changed)",
                short_fingerprint(old_fp),
                short_fingerprint(new_fp)
            ),
            AuthError::NotTheRealm => write!(f, "the server could not prove it is the realm it claims to be"),
            AuthError::BadKdf(n) => write!(f, "the server asked for {n} key rounds, which this client doesn't accept"),
            AuthError::TokenMalformed => write!(f, "not a sign-in token"),
            AuthError::TokenSignature => write!(f, "the token is not from the realm this server trusts"),
            AuthError::TokenAudience => write!(f, "the token was made for another server"),
            AuthError::TokenExpired => write!(f, "the token has expired"),
            AuthError::TokenProof => write!(f, "the token's session proof is wrong"),
            AuthError::Protocol(why) => write!(f, "protocol error: {why}"),
            AuthError::Network(why) => write!(f, "network: {why}"),
            AuthError::Timeout => write!(f, "the server didn't answer in time"),
            AuthError::Closed => write!(f, "the server closed the connection"),
            AuthError::Server(why) => write!(f, "refused: {why}"),
            AuthError::Storage(why) => write!(f, "saving the login failed: {why}"),
        }
    }
}

impl std::error::Error for AuthError {}

impl AuthError {
    /// The refusal a server sends for this error: `<code>` plus an argument where it has one.
    pub fn fail_words(&self) -> String {
        match self {
            AuthError::WrongPassword => "auth".into(),
            AuthError::Locked { wait_s } => format!("locked {wait_s}"),
            AuthError::UsernameTaken(u) => format!("taken {u}"),
            AuthError::BadUsername(why) => format!("username {why}"),
            AuthError::BadDisplayName(why) => format!("display {why}"),
            AuthError::Disabled => "disabled".into(),
            AuthError::NeedsNewPassword => "reset".into(),
            AuthError::SignInExpired => "expired".into(),
            AuthError::NotSignedIn(_) => "signin".into(),
            other => format!("other {other}"),
        }
    }

    /// The error a client reports for `FAIL <code> <rest>`.
    pub fn from_fail(code: &str, rest: &str) -> AuthError {
        match code {
            "auth" => AuthError::WrongPassword,
            "locked" => AuthError::Locked { wait_s: rest.trim().parse().unwrap_or(60) },
            "taken" => AuthError::UsernameTaken(rest.trim().to_string()),
            "username" => AuthError::BadUsername(rest.trim().to_string()),
            "display" => AuthError::BadDisplayName(rest.trim().to_string()),
            "disabled" => AuthError::Disabled,
            "reset" => AuthError::NeedsNewPassword,
            "expired" => AuthError::SignInExpired,
            "signin" => AuthError::NotSignedIn("this realm".into()),
            _ => AuthError::Server(rest.trim().to_string()),
        }
    }
}

// ------------------------------------------------------------------ fingerprints

/// A realm's (or backend's) fingerprint: lowercase hex of the first 16 bytes of
/// SHA-256 of its 32-byte ed25519 public key (32 hex characters).
pub fn fingerprint(public_key: &[u8; 32]) -> String {
    hex(&ecm_ssh_crypto::sha256(public_key)[..16])
}

/// A fingerprint as players see it: upper-case byte pairs joined by ':' (`3F:A1:9C:...`).
pub fn display_fingerprint(fp: &str) -> String {
    let b = fp.as_bytes();
    b.chunks(2).map(|c| String::from_utf8_lossy(c).to_ascii_uppercase()).collect::<Vec<_>>().join(":")
}

/// The first 4 pairs of the display form (`3F:A1:9C:0D`), for one-line labels.
pub fn short_fingerprint(fp: &str) -> String {
    display_fingerprint(&fp[..fp.len().min(8)])
}

/// A fingerprint as typed: lower-cased, `:` and spaces dropped. None if it isn't 32 hex digits.
pub fn parse_fingerprint(s: &str) -> Option<String> {
    let fp: String = s.chars().filter(|c| *c != ':' && !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    is_fingerprint(&fp).then_some(fp)
}

/// 32 lowercase hex digits.
pub fn is_fingerprint(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// ------------------------------------------------------------------ names

/// A username: 1-16 of `a-z 0-9 . _ -`, compared in lower case. Returns the lower-case form.
pub fn username(s: &str) -> Result<String, AuthError> {
    let u = s.trim().to_ascii_lowercase();
    if u.is_empty() {
        return Err(AuthError::BadUsername("it can't be empty".into()));
    }
    if u.chars().count() > MAX_USERNAME {
        return Err(AuthError::BadUsername(format!("at most {MAX_USERNAME} characters")));
    }
    if !u.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')) {
        return Err(AuthError::BadUsername("use only a-z, 0-9, '.', '_' and '-'".into()));
    }
    Ok(u)
}

/// A display name (or realm name): 1-32 printable characters, trimmed.
pub fn display_name(s: &str) -> Result<String, AuthError> {
    let d = s.trim();
    if d.is_empty() {
        return Err(AuthError::BadDisplayName("it can't be empty".into()));
    }
    if d.chars().count() > MAX_DISPLAY_NAME {
        return Err(AuthError::BadDisplayName(format!("at most {MAX_DISPLAY_NAME} characters")));
    }
    if d.chars().any(|c| c.is_control()) {
        return Err(AuthError::BadDisplayName("only printable characters".into()));
    }
    Ok(d.to_string())
}

/// `host` or `host:port` -> `host:port` (port [`DEFAULT_PORT`] when none is given).
pub fn host_port(s: &str) -> Result<(String, u16), AuthError> {
    let s = s.trim();
    let (h, p) = match s.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok().filter(|p| *p > 0).ok_or_else(|| AuthError::Network(format!("bad port in '{s}'")))?),
        None => (s, DEFAULT_PORT),
    };
    if h.is_empty() || h.contains(char::is_whitespace) {
        return Err(AuthError::Network(format!("bad address '{s}'")));
    }
    Ok((h.to_string(), p))
}

/// The `host:port` form used as a pin's host.
pub fn endpoint(s: &str) -> Result<String, AuthError> {
    let (h, p) = host_port(s)?;
    Ok(format!("{h}:{p}"))
}

// ------------------------------------------------------------------ realm reference

/// The line a backend uses to say which realm it trusts:
/// `AUTH <fp> <host:port|-> <name...>`, where `-` means "this backend runs the realm
/// itself" (`--auth local`), at the address the backend is reached at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmRef {
    pub name: String,
    pub fp: String,
    pub endpoint: Option<String>,
}

impl RealmRef {
    pub fn to_line(&self) -> String {
        format!("AUTH {} {} {}", self.fp, self.endpoint.as_deref().unwrap_or("-"), self.name)
    }

    pub fn parse(line: &str) -> Result<RealmRef, AuthError> {
        let bad = || AuthError::Protocol(format!("bad realm line '{}'", line.trim()));
        let rest = line.trim().strip_prefix("AUTH ").ok_or_else(bad)?;
        let mut w = rest.splitn(3, ' ');
        let fp = w.next().filter(|f| is_fingerprint(f)).ok_or_else(bad)?.to_string();
        let ep = w.next().ok_or_else(bad)?;
        let name = display_name(w.next().ok_or_else(bad)?).map_err(|_| bad())?;
        let endpoint = if ep == "-" { None } else { Some(endpoint(ep).map_err(|_| bad())?) };
        Ok(RealmRef { name, fp, endpoint })
    }
}

// ------------------------------------------------------------------ encodings

pub fn hex(bytes: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(D[(b >> 4) as usize] as char);
        s.push(D[(b & 15) as usize] as char);
    }
    s
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let v = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    s.as_bytes().chunks(2).map(|p| Some(v(p[0])? << 4 | v(p[1])?)).collect()
}

/// Exactly N bytes of hex.
pub fn unhex_n<const N: usize>(s: &str) -> Option<[u8; N]> {
    unhex(s)?.try_into().ok()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// base64url without padding (RFC 4648 section 5).
pub fn b64url(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..c.len() + 1 {
            s.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    s
}

pub fn unb64url(s: &str) -> Option<Vec<u8>> {
    if s.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for c in s.as_bytes().chunks(4) {
        let mut n = 0u32;
        for (i, &b) in c.iter().enumerate() {
            let v = B64.iter().position(|&x| x == b)? as u32;
            n |= v << (18 - 6 * i);
        }
        out.extend_from_slice(&n.to_be_bytes()[1..c.len()]);
    }
    Some(out)
}

/// Random bytes from the platform (WASI `random_get` on a computer).
pub fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).expect("no random source");
    b
}

/// The ed25519 key for a 32-byte seed.
pub fn signing_key(seed: &[u8; 32]) -> SigningKey {
    ecm_ssh_crypto::ed25519_private_key_from_bytes(seed)
}

/// The 32-byte public key of a signing key.
pub fn public_key(key: &SigningKey) -> [u8; 32] {
    ecm_ssh_crypto::ed25519_public_key_bytes(&key.verifying_key())
}

/// Sign `parts` concatenated.
pub fn sign(key: &SigningKey, parts: &[&[u8]]) -> [u8; 64] {
    ecm_ssh_crypto::ed25519_sign(key, &parts.concat())
}

/// Check a signature by `public` over `parts` concatenated (false for a bad key).
pub fn verify(public: &[u8; 32], parts: &[&[u8]], sig: &[u8; 64]) -> bool {
    match ecm_ssh_crypto::ed25519_public_key_from_bytes(public) {
        Ok(vk) => ecm_ssh_crypto::ed25519_verify(&vk, &parts.concat(), sig),
        Err(_) => false,
    }
}
