//! The client side of the handshake and the requests after it, over a small blocking
//! [`Stream`]: a program plugs in a socket ([`crate::host`]), tests an in-memory pipe
//! that drives a [`crate::ServerConn`].
//!
//! [`handshake`] checks that the server holds the private half of the realm key it
//! presents; whether that key is the *right* one (pinned) is the caller's decision
//! ([`crate::store`] makes it).

use crate::wire::{self, Channel, ServerHello, LOGIN_LABEL, PASSWD_LABEL, SIGNUP_LABEL};
use crate::{display_name, fingerprint, hex, kdf, public_key, sign, verify, AuthError, SigningKey};

/// How long to wait for each answer.
pub const READ_TIMEOUT_MS: u32 = 10_000;

/// A byte stream to the server.
pub trait Stream {
    fn write_all(&mut self, data: &[u8]) -> Result<(), AuthError>;
    /// Fill `buf` completely, or fail (`Timeout` after `timeout_ms` without data, `Closed`).
    fn read_exact(&mut self, buf: &mut [u8], timeout_ms: u32) -> Result<(), AuthError>;
}

impl<S: Stream + ?Sized> Stream for Box<S> {
    fn write_all(&mut self, data: &[u8]) -> Result<(), AuthError> {
        (**self).write_all(data)
    }
    fn read_exact(&mut self, buf: &mut [u8], timeout_ms: u32) -> Result<(), AuthError> {
        (**self).read_exact(buf, timeout_ms)
    }
}

impl<S: Stream + ?Sized> Stream for &mut S {
    fn write_all(&mut self, data: &[u8]) -> Result<(), AuthError> {
        (**self).write_all(data)
    }
    fn read_exact(&mut self, buf: &mut [u8], timeout_ms: u32) -> Result<(), AuthError> {
        (**self).read_exact(buf, timeout_ms)
    }
}

fn read_frame<S: Stream>(s: &mut S) -> Result<Vec<u8>, AuthError> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len, READ_TIMEOUT_MS)?;
    let n = u32::from_le_bytes(len) as usize;
    if n > wire::MAX_FRAME {
        return Err(AuthError::Protocol(format!("frame of {n} bytes")));
    }
    let mut p = vec![0u8; n];
    s.read_exact(&mut p, READ_TIMEOUT_MS)?;
    Ok(p)
}

/// Who the server proved to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    pub realm_name: String,
    pub realm_pub: [u8; 32],
    pub realm_fp: String,
    pub kdf_iterations: u32,
}

/// A successful sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Welcome {
    pub user_id: u64,
    pub username: String,
    pub display_name: String,
    pub refresh_token: String,
    pub refresh_expires_ms: u64,
}

/// An open, encrypted connection to a realm.
pub struct Session<S: Stream> {
    stream: S,
    ch: Channel,
    th: [u8; 32],
    info: ServerInfo,
}

/// Steps 1-3: send our ephemeral key, check the realm's signature over the transcript,
/// derive the channel keys.
pub fn handshake<S: Stream>(mut stream: S) -> Result<(Session<S>, ServerInfo), AuthError> {
    let (ce_pub, ce_secret) = ecm_ssh_crypto::x25519_generate_keypair();
    let ce = ce_pub.to_bytes();
    stream.write_all(&wire::frame(&wire::client_hello(&ce)))?;
    let hello = ServerHello::parse(&read_frame(&mut stream)?)?;
    let th = wire::transcript(&ce, &hello.se, &hello.realm_pub, &hello.realm_name, hello.kdf_iterations);
    // A spoof without the realm's private key fails here; so does a relay that changed `ce`.
    if !verify(&hello.realm_pub, &[&th], &hello.sig) {
        return Err(AuthError::NotTheRealm);
    }
    if !kdf::acceptable(hello.kdf_iterations) {
        return Err(AuthError::BadKdf(hello.kdf_iterations));
    }
    let realm_name = display_name(&hello.realm_name).map_err(|_| AuthError::Protocol("bad realm name".into()))?;
    let dh = ecm_ssh_crypto::x25519_diffie_hellman(&ce_secret, &ecm_ssh_crypto::X25519Public::from(hello.se));
    let ch = Channel::new(&dh, &th, true)?;
    let info = ServerInfo { realm_name, realm_pub: hello.realm_pub, realm_fp: fingerprint(&hello.realm_pub), kdf_iterations: hello.kdf_iterations };
    Ok((Session { stream, ch, th, info: info.clone() }, info))
}

fn parse_welcome(reply: &str) -> Result<Welcome, AuthError> {
    let bad = || AuthError::Protocol("bad WELCOME".into());
    let rest = reply.strip_prefix("WELCOME ").ok_or_else(bad)?;
    let w: Vec<&str> = rest.splitn(5, ' ').collect();
    if w.len() != 5 {
        return Err(bad());
    }
    Ok(Welcome {
        user_id: w[0].parse().map_err(|_| bad())?,
        username: w[1].to_string(),
        refresh_token: w[2].to_string(),
        refresh_expires_ms: w[3].parse().map_err(|_| bad())?,
        display_name: w[4].to_string(),
    })
}

impl<S: Stream> Session<S> {
    pub fn info(&self) -> &ServerInfo {
        &self.info
    }

    /// The transcript hash (what sign-in signatures cover).
    pub fn transcript(&self) -> &[u8; 32] {
        &self.th
    }

    /// One request, one answer; `FAIL` becomes the error it names.
    fn request(&mut self, line: &str) -> Result<String, AuthError> {
        let f = self.ch.seal(line.as_bytes());
        self.stream.write_all(&f)?;
        let p = read_frame(&mut self.stream)?;
        let reply = String::from_utf8(self.ch.open(&p)?).map_err(|_| AuthError::Protocol("not UTF-8".into()))?;
        if let Some(rest) = reply.strip_prefix("FAIL ") {
            let (code, arg) = wire::word(rest);
            return Err(AuthError::from_fail(code, arg));
        }
        Ok(reply)
    }

    /// Sign in with the account key (from [`crate::account_key`]).
    pub fn login(&mut self, username: &str, account: &SigningKey, session_pub: &[u8; 32]) -> Result<Welcome, AuthError> {
        let u = crate::username(username)?;
        let sig = sign(account, &[LOGIN_LABEL, &self.th]);
        let reply = self.request(&format!("LOGIN {u} {} {}", hex(session_pub), hex(&sig)))?;
        parse_welcome(&reply)
    }

    /// Make an account (or set the new password of an account the operator reset).
    pub fn signup(&mut self, username: &str, display: &str, account: &SigningKey, session_pub: &[u8; 32]) -> Result<Welcome, AuthError> {
        let u = crate::username(username)?;
        let d = display_name(display)?;
        let sig = sign(account, &[SIGNUP_LABEL, &self.th]);
        let reply = self.request(&format!("SIGNUP {u} {} {} {} {d}", hex(&public_key(account)), hex(session_pub), hex(&sig)))?;
        parse_welcome(&reply)
    }

    /// Sign in with a saved refresh token.
    pub fn refresh(&mut self, username: &str, refresh_token: &str, session_pub: &[u8; 32]) -> Result<Welcome, AuthError> {
        let u = crate::username(username)?;
        if refresh_token.is_empty() || refresh_token.contains(' ') {
            return Err(AuthError::SignInExpired);
        }
        let reply = self.request(&format!("REFRESH {u} {} {refresh_token}", hex(session_pub)))?;
        parse_welcome(&reply)
    }

    /// A token for the backend whose key fingerprint is `audience`: (token, expires ms).
    pub fn token(&mut self, audience: &str) -> Result<(String, u64), AuthError> {
        let aud = crate::parse_fingerprint(audience).ok_or_else(|| AuthError::Protocol(format!("'{audience}' is not a fingerprint")))?;
        let reply = self.request(&format!("TOKEN {aud}"))?;
        let rest = reply.strip_prefix("TOKEN ").ok_or_else(|| AuthError::Protocol("bad TOKEN".into()))?;
        let (exp, token) = wire::word(rest);
        Ok((token.to_string(), exp.parse().map_err(|_| AuthError::Protocol("bad TOKEN".into()))?))
    }

    /// Change the password: the old key vouches for the new one, the new one proves itself.
    pub fn passwd(&mut self, old: &SigningKey, new: &SigningKey) -> Result<(), AuthError> {
        let new_pub = public_key(new);
        let new_sig = sign(new, &[PASSWD_LABEL, &self.th]);
        let old_sig = sign(old, &[PASSWD_LABEL, &self.th, &new_pub]);
        self.request(&format!("PASSWD {} {} {}", hex(&new_pub), hex(&new_sig), hex(&old_sig)))?;
        Ok(())
    }

    /// (user id, username, display name).
    pub fn whoami(&mut self) -> Result<(u64, String, String), AuthError> {
        let reply = self.request("WHOAMI")?;
        let bad = || AuthError::Protocol("bad ME".into());
        let w: Vec<&str> = reply.strip_prefix("ME ").ok_or_else(bad)?.splitn(3, ' ').collect();
        if w.len() != 3 {
            return Err(bad());
        }
        Ok((w[0].parse().map_err(|_| bad())?, w[1].to_string(), w[2].to_string()))
    }

    /// Revoke this session's refresh token; the server hangs up.
    pub fn logout(&mut self) -> Result<(), AuthError> {
        self.request("LOGOUT")?;
        Ok(())
    }

    pub fn into_stream(self) -> S {
        self.stream
    }
}
