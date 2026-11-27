//! The TLS protocol: rustls with RustCrypto arithmetic (the `tls` feature).

use crate::{is_public, x509, Authority, Check, TlsError, Trust, CERTS_DIR};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, Der, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, TrustAnchor, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, ServerConfig, ServerConnection, SignatureScheme, StreamOwned};
use std::io::{self, Read, Write};
use std::sync::Arc;

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

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls_rustcrypto::provider())
}

impl Authority {
    fn anchor(&self) -> TrustAnchor<'static> {
        TrustAnchor { subject: Der::from(self.subject.clone()), subject_public_key_info: Der::from(self.spki.clone()), name_constraints: None }
    }
}

impl Trust {
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

