//! TLS for programs on a computer.
//!
//! rustls does the protocol and RustCrypto the arithmetic, all in the program
//! (there is no TLS in the kernel or in the host). A connection runs over any
//! `Read + Write`: a socket on a computer, a pipe in tests.
//!
//! # Whom a client trusts
//!
//! Two sets of authorities, chosen by the address the client is connected to:
//!
//! * a **public** address (the real internet, through an Internet Gateway):
//!   the real world's web roots, built in;
//! * any **other** address (10/8, 172.16/12, 192.168/16, 100.64/10, loopback,
//!   link-local: the game's networks): the in-game authorities this computer
//!   trusts, from `/etc/ssl/certs` ([`Trust::load`]).
//!
//! So an in-game authority cannot vouch for a server on the real internet, and
//! a real authority is never asked about a server in the game.
//!
//! `/etc/ssl/certs` holds `*.pem` (root certificates) and `*.anchor` files
//! (one line: `ed25519 <public key hex> <common name>`, an authority known by
//! its key alone, which is how a Tech Village computer knows the village's).

pub mod x509;

mod tls;
pub use rustls;
pub use tls::{Identity, TlsStream};

/// Where a computer keeps the in-game authorities it trusts.
pub const CERTS_DIR: &str = "/etc/ssl/certs";

/// Where `cert` puts this computer's certificate (the chain, leaf first), and servers look for it.
pub const SERVER_CERT: &str = "/etc/ssl/server.crt";
/// The key of [`SERVER_CERT`].
pub const SERVER_KEY: &str = "/etc/ssl/private/server.key";

/// Why a TLS connection failed, in words a player can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsError {
    /// The certificate is not signed by an authority this computer trusts for that address.
    UnknownAuthority(String),
    /// The certificate is for another name.
    WrongName(String),
    /// The certificate is out of date (or not valid yet).
    Expired(String),
    /// Any other certificate problem.
    BadCertificate(String),
    /// The two ends could not agree, or the other end is not speaking TLS.
    Handshake(String),
    /// The connection broke.
    Io(String),
    /// A bad certificate, key or name given to this library.
    Config(String),
}

impl std::fmt::Display for TlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TlsError::UnknownAuthority(m) => write!(f, "certificate not trusted: {m}"),
            TlsError::WrongName(m) => write!(f, "certificate is for another name: {m}"),
            TlsError::Expired(m) => write!(f, "certificate is out of date: {m}"),
            TlsError::BadCertificate(m) => write!(f, "bad certificate: {m}"),
            TlsError::Handshake(m) => write!(f, "TLS handshake failed: {m}"),
            TlsError::Io(m) => write!(f, "connection lost during the TLS handshake: {m}"),
            TlsError::Config(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for TlsError {}

/// Is this a public (real internet) address? Everything the game's own networks use is not.
pub fn is_public(ip: [u8; 4]) -> bool {
    !matches!(
        ip,
        [0, ..] | [10, ..] | [127, ..] | [169, 254, ..] | [192, 168, ..] | [100, 64..=127, ..] | [172, 16..=31, ..] | [224..=255, ..]
    )
}

/// An in-game authority a computer trusts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authority {
    pub common_name: String,
    /// The file it came from.
    pub source: String,
    pub(crate) subject: Vec<u8>,
    pub(crate) spki: Vec<u8>,
}

impl Authority {
    /// From its ed25519 public key and name alone.
    pub fn from_key(public: &[u8; 32], common_name: &str, source: &str) -> Authority {
        Authority { common_name: common_name.into(), source: source.into(), subject: x509::name_body(common_name), spki: x509::spki_body(public) }
    }

    /// The `.anchor` line for an authority known by its key.
    pub fn anchor_line(public: &[u8; 32], common_name: &str) -> String {
        format!("ed25519 {} {}\n", hex(public), common_name)
    }

    /// Parse an `.anchor` file: `ed25519 <64 hex digits> <common name>`.
    pub fn parse_anchor(text: &str, source: &str) -> Option<Authority> {
        let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'))?;
        let mut it = line.splitn(3, ' ');
        if it.next()? != "ed25519" {
            return None;
        }
        let key: [u8; 32] = unhex(it.next()?)?.try_into().ok()?;
        let cn = it.next()?.trim();
        (!cn.is_empty() && cn.len() <= 64).then(|| Authority::from_key(&key, cn, source))
    }

    /// From a root certificate (DER); None when it is not a certificate authority's.
    pub fn from_certificate(der: &[u8], source: &str) -> Option<Authority> {
        let info = x509::describe(der)?;
        info.is_ca.then(|| Authority { common_name: info.subject, source: source.into(), subject: info.subject_der, spki: info.spki_der })
    }
}

/// How strictly a client checks the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Check {
    /// The certificate must chain to a trusted authority and name the server.
    #[default]
    Full,
    /// Encrypt, but accept any certificate (`curl -k`). Never for anything that carries a secret.
    None,
}

/// What a client trusts: the built-in public roots and this computer's in-game authorities.
#[derive(Clone, Default)]
pub struct Trust {
    pub ingame: Vec<Authority>,
    /// Files in the certificate folder that could not be read as an authority.
    pub skipped: Vec<String>,
        pub(crate) public_cfg: Option<std::sync::Arc<rustls::ClientConfig>>,
        pub(crate) ingame_cfg: Option<std::sync::Arc<rustls::ClientConfig>>,
}

impl Trust {
    /// Only the public roots (and no in-game authority).
    pub fn public_only() -> Trust {
        Trust::default()
    }

    pub fn with(authorities: Vec<Authority>) -> Trust {
        Trust { ingame: authorities, ..Trust::default() }
    }

    /// Read the authorities in `dir` (`*.pem` and `*.anchor`); a missing folder is an empty set.
    pub fn load(dir: &str) -> Trust {
        let mut t = Trust::default();
        let Ok(rd) = std::fs::read_dir(dir) else { return t };
        let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        for name in names {
            let path = format!("{}/{}", dir.trim_end_matches('/'), name);
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let before = t.ingame.len();
            if name.ends_with(".anchor") {
                t.ingame.extend(Authority::parse_anchor(&text, &name));
            } else if name.ends_with(".pem") || name.ends_with(".crt") {
                for der in x509::pem_blocks(&text, x509::CERTIFICATE) {
                    t.ingame.extend(Authority::from_certificate(&der, &name));
                }
            } else {
                continue;
            }
            if t.ingame.len() == before {
                t.skipped.push(name);
            }
        }
        t
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

#[cfg(test)]
mod tests;
