//! https for [`Client`](crate::Client): the server's certificate is checked
//! against the authorities for where it is (`ecm_tls`).
//!
//! This is the only place the client touches the TLS library, and only
//! `Client::new` refers to it: a program that makes its clients with
//! `Client::plain` links none of it.

use crate::Stream;
use ecm_tls::{Check, TlsError, TlsStream, Trust};
use std::io::{Read, Write};

/// Says goodbye (close_notify) when the exchange is over.
struct Closing(TlsStream<Box<dyn Stream>>);

impl Read for Closing {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Closing {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl Drop for Closing {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// TLS over `stream`, already connected to `peer`. The trusted in-game
/// authorities are read from `/etc/ssl/certs` the first time they are needed.
pub fn secure(trust: &mut Option<Trust>, check: Check, stream: Box<dyn Stream>, host: &str, peer: [u8; 4]) -> Result<(Box<dyn Stream>, &'static str), TlsError> {
    let trust = trust.get_or_insert_with(|| Trust::load(ecm_tls::CERTS_DIR));
    let tls = trust.connect(stream, host, peer, check)?;
    let version = tls.version();
    Ok((Box::new(Closing(tls)), version))
}
