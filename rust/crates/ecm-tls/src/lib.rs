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

pub use rustls;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, Der, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, TrustAnchor, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, ServerConfig, ServerConnection, SignatureScheme, StreamOwned};
use std::io::{self, Read, Write};
use std::sync::Arc;

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

fn classify(e: &rustls::Error, ingame: bool) -> TlsError {
    use rustls::CertificateError as C;
    match e {
        rustls::Error::InvalidCertificate(c) => match c {
            // BadSignature: an authority of that name is trusted, but another key signed this.
            C::UnknownIssuer | C::BadSignature => TlsError::UnknownAuthority(if ingame {
                format!("no authority in {CERTS_DIR} signed it (add one with: cert trust)")
            } else {
                "no public authority signed it".into()
            }),
            C::NotValidForName | C::NotValidForNameContext { .. } => TlsError::WrongName(format!("{c:?}")),
            C::Expired | C::ExpiredContext { .. } | C::NotValidYet | C::NotValidYetContext { .. } => {
                TlsError::Expired("check the date, or renew it with: cert renew".into())
            }
            other => TlsError::BadCertificate(format!("{other:?}")),
        },
        other => TlsError::Handshake(other.to_string()),
    }
}

/// Is this a public (real internet) address? Everything the game's own networks use is not.
pub fn is_public(ip: [u8; 4]) -> bool {
    !matches!(
        ip,
        [0, ..] | [10, ..] | [127, ..] | [169, 254, ..] | [192, 168, ..] | [100, 64..=127, ..] | [172, 16..=31, ..] | [224..=255, ..]
    )
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls_rustcrypto::provider())
}

/// An in-game authority a computer trusts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authority {
    pub common_name: String,
    /// The file it came from.
    pub source: String,
    subject: Vec<u8>,
    spki: Vec<u8>,
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

    /// From a root certificate (DER).
    pub fn from_certificate(der: &[u8], source: &str) -> Option<Authority> {
        let cert = CertificateDer::from(der.to_vec());
        let mut store = RootCertStore::empty();
        store.add(cert).ok()?;
        let a = store.roots.first()?;
        let cn = x509::describe(der).map(|i| i.subject).unwrap_or_default();
        Some(Authority { common_name: cn, source: source.into(), subject: a.subject.as_ref().to_vec(), spki: a.subject_public_key_info.as_ref().to_vec() })
    }

    fn anchor(&self) -> TrustAnchor<'static> {
        TrustAnchor { subject: Der::from(self.subject.clone()), subject_public_key_info: Der::from(self.spki.clone()), name_constraints: None }
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
    public_cfg: Option<Arc<ClientConfig>>,
    ingame_cfg: Option<Arc<ClientConfig>>,
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

    fn config(&mut self, ingame: bool, check: Check) -> Result<Arc<ClientConfig>, TlsError> {
        let builder = ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| TlsError::Config(e.to_string()))?;
        if check == Check::None {
            let v = Arc::new(AcceptAny(provider()));
            return Ok(Arc::new(builder.dangerous().with_custom_certificate_verifier(v).with_no_client_auth()));
        }
        let slot = if ingame { &mut self.ingame_cfg } else { &mut self.public_cfg };
        if let Some(c) = slot {
            return Ok(c.clone());
        }
        let mut roots = RootCertStore::empty();
        if ingame {
            roots.roots.extend(self.ingame.iter().map(Authority::anchor));
        } else {
            roots.roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
        let cfg = Arc::new(builder.with_root_certificates(roots).with_no_client_auth());
        *slot = Some(cfg.clone());
        Ok(cfg)
    }

    /// Open a TLS connection as a client over `stream`, which is already
    /// connected to `peer`. `server_name` is the host name (or address) asked
    /// for; the handshake is finished before this returns.
    pub fn connect<S: Read + Write>(&mut self, stream: S, server_name: &str, peer: [u8; 4], check: Check) -> Result<TlsStream<S>, TlsError> {
        let ingame = !is_public(peer);
        if ingame && check == Check::Full && self.ingame.is_empty() {
            return Err(TlsError::UnknownAuthority(format!(
                "this computer trusts no in-game authority ({CERTS_DIR} is empty; add one with: cert trust)"
            )));
        }
        let cfg = self.config(ingame, check)?;
        let name = ServerName::try_from(server_name.to_string()).map_err(|_| TlsError::Config(format!("'{server_name}' is not a host name")))?;
        let conn = ClientConnection::new(cfg, name).map_err(|e| TlsError::Config(e.to_string()))?;
        let mut s = StreamOwned::new(conn, stream);
        while s.conn.is_handshaking() {
            if let Err(e) = s.conn.complete_io(&mut s.sock) {
                return Err(handshake_error(e, ingame));
            }
        }
        Ok(TlsStream(Inner::Client(s)))
    }
}

fn handshake_error(e: io::Error, ingame: bool) -> TlsError {
    match e.get_ref().and_then(|i| i.downcast_ref::<rustls::Error>()) {
        Some(r) => classify(r, ingame),
        None if e.kind() == io::ErrorKind::UnexpectedEof => TlsError::Handshake("the other end closed the connection (is it a TLS port?)".into()),
        None => TlsError::Io(e.to_string()),
    }
}

/// `curl -k`: any certificate, but the handshake signature must still be real.
#[derive(Debug)]
struct AcceptAny(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// A server's certificate chain and key.
#[derive(Clone)]
pub struct Identity(Arc<ServerConfig>);

impl Identity {
    /// From DER: the chain (leaf first) and a PKCS#8 key.
    pub fn new(chain: Vec<Vec<u8>>, pkcs8: Vec<u8>) -> Result<Identity, TlsError> {
        if chain.is_empty() {
            return Err(TlsError::Config("no certificate".into()));
        }
        // An ed25519 key (what `cert` makes) must be the one in the certificate.
        if let Some(seed) = x509::seed_of_pkcs8(&pkcs8) {
            let public = x509::public_of(&seed);
            if !chain[0].windows(32).any(|w| w == public) {
                return Err(TlsError::Config("certificate and key do not fit: the key belongs to another certificate".into()));
            }
        }
        let chain: Vec<CertificateDer<'static>> = chain.into_iter().map(CertificateDer::from).collect();
        let cfg = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| TlsError::Config(e.to_string()))?
            .with_no_client_auth()
            .with_single_cert(chain, PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8)))
            .map_err(|e| TlsError::Config(format!("certificate and key do not fit: {e}")))?;
        Ok(Identity(Arc::new(cfg)))
    }

    /// From PEM text: `CERTIFICATE` blocks (leaf first) and one `PRIVATE KEY` block, in either or both strings.
    pub fn from_pem(cert_pem: &str, key_pem: &str) -> Result<Identity, TlsError> {
        let chain = x509::pem_blocks(cert_pem, x509::CERTIFICATE);
        let key = x509::pem_blocks(key_pem, x509::PRIVATE_KEY).into_iter().next().ok_or_else(|| TlsError::Config("no PRIVATE KEY block".into()))?;
        Identity::new(chain, key)
    }

    /// Read `<cert file>` and `<key file>`.
    pub fn load(cert_path: &str, key_path: &str) -> Result<Identity, TlsError> {
        let read = |p: &str| std::fs::read_to_string(p).map_err(|e| TlsError::Config(format!("cannot read {p}: {e}")));
        Identity::from_pem(&read(cert_path)?, &read(key_path)?)
    }

    /// Answer a TLS client on `stream`; the handshake is finished before this returns.
    pub fn accept<S: Read + Write>(&self, stream: S) -> Result<TlsStream<S>, TlsError> {
        let conn = ServerConnection::new(self.0.clone()).map_err(|e| TlsError::Config(e.to_string()))?;
        let mut s = StreamOwned::new(conn, stream);
        while s.conn.is_handshaking() {
            if let Err(e) = s.conn.complete_io(&mut s.sock) {
                return Err(handshake_error(e, true));
            }
        }
        Ok(TlsStream(Inner::Server(s)))
    }
}

enum Inner<S: Read + Write> {
    Client(StreamOwned<ClientConnection, S>),
    Server(StreamOwned<ServerConnection, S>),
}

/// An open TLS connection: read and write plain bytes.
pub struct TlsStream<S: Read + Write>(Inner<S>);

impl<S: Read + Write> TlsStream<S> {
    /// `TLSv1.3` or `TLSv1.2`.
    pub fn version(&self) -> &'static str {
        let v = match &self.0 {
            Inner::Client(s) => s.conn.protocol_version(),
            Inner::Server(s) => s.conn.protocol_version(),
        };
        match v {
            Some(rustls::ProtocolVersion::TLSv1_3) => "TLSv1.3",
            Some(rustls::ProtocolVersion::TLSv1_2) => "TLSv1.2",
            _ => "TLS",
        }
    }

    /// The certificates the other end showed (a client sees the server's chain), DER, leaf first.
    pub fn peer_certificates(&self) -> Vec<Vec<u8>> {
        let certs = match &self.0 {
            Inner::Client(s) => s.conn.peer_certificates(),
            Inner::Server(s) => s.conn.peer_certificates(),
        };
        certs.map(|c| c.iter().map(|d| d.as_ref().to_vec()).collect()).unwrap_or_default()
    }

    /// Say goodbye (close_notify) and flush. The stream underneath is left open.
    pub fn close(&mut self) {
        match &mut self.0 {
            Inner::Client(s) => {
                s.conn.send_close_notify();
                let _ = s.conn.complete_io(&mut s.sock);
            }
            Inner::Server(s) => {
                s.conn.send_close_notify();
                let _ = s.conn.complete_io(&mut s.sock);
            }
        }
    }

    pub fn get_mut(&mut self) -> &mut S {
        match &mut self.0 {
            Inner::Client(s) => &mut s.sock,
            Inner::Server(s) => &mut s.sock,
        }
    }
}

impl<S: Read + Write> Read for TlsStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let r = match &mut self.0 {
            Inner::Client(s) => s.read(buf),
            Inner::Server(s) => s.read(buf),
        };
        // A peer that just drops the connection (no close_notify) ends the stream:
        // HTTP bodies carry their own length, so a cut-off one is still noticed.
        match r {
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(0),
            other => other,
        }
    }
}

impl<S: Read + Write> Write for TlsStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.0 {
            Inner::Client(s) => s.write(buf),
            Inner::Server(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.0 {
            Inner::Client(s) => s.flush(),
            Inner::Server(s) => s.flush(),
        }
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
