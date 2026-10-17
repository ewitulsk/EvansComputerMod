//! The realm (server side), sans-IO: `authd`'s poll loop, and backends running
//! `--auth local`, feed each connection's bytes to a [`ServerConn`] and send what it
//! returns. The [`Realm`] holds the realm key, the users and the refresh tokens.
//!
//! Users file ([`Realm::save_text`]), one record per line:
//!
//! ```text
//! ecm-auth-users 1
//! kdf <iterations>
//! next <next user id>
//! U <id> <username> ok|disabled <account pub hex|-> <created ms> <display name...>
//! R <user id> <sha256(refresh token) hex> <expires ms>
//! ```
//!
//! `-` as the key is an account the operator reset: the next sign-in with that username
//! sets a new password (it is accepted as a sign-up for the existing account). Refresh
//! tokens are stored hashed, so a copied users file doesn't sign anyone in. The
//! iteration count is stored because changing it would change every account key.

use std::collections::HashMap;

use crate::wire::{self, Channel, FrameReader, ServerHello, LOGIN_LABEL, PASSWD_LABEL, SIGNUP_LABEL};
use crate::{
    display_name, fingerprint, hex, is_fingerprint, kdf, public_key, random, sign, signing_key, unhex_n, username, verify,
    AuthError, SigningKey, REFRESH_LIFETIME_MS, TOKEN_LIFETIME_MS,
};

/// Failed sign-ins a username gets before it is locked out.
pub const FREE_TRIES_USER: u32 = 5;
/// Failed sign-ins an address gets (higher: several players can share one).
pub const FREE_TRIES_ADDR: u32 = 10;
/// The first lockout; each further failure doubles it, up to [`LOCK_MAX_MS`].
pub const LOCK_BASE_MS: u64 = 30_000;
pub const LOCK_MAX_MS: u64 = 15 * 60_000;
/// Failures older than this are forgotten.
pub const FAIL_MEMORY_MS: u64 = 60 * 60_000;
/// Sign-in attempts on one connection before the server hangs up.
pub const TRIES_PER_CONNECTION: u8 = 3;
/// Refresh tokens one user keeps (signed-in devices); the oldest goes first.
pub const MAX_REFRESH_PER_USER: usize = 16;
pub const MAX_USERS: usize = 10_000;

/// One account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: u64,
    pub username: String,
    pub display_name: String,
    /// The account public key; None after `authd reset`.
    pub key: Option<[u8; 32]>,
    pub disabled: bool,
    pub created_ms: u64,
}

struct RefreshRec {
    user_id: u64,
    hash: [u8; 32],
    expires_ms: u64,
}

struct Fails {
    count: u32,
    last_ms: u64,
}

/// A realm: its key, name, KDF cost, users and refresh tokens.
pub struct Realm {
    key: SigningKey,
    name: String,
    kdf_iterations: u32,
    users: Vec<User>,
    refresh: Vec<RefreshRec>,
    fails: HashMap<String, Fails>,
    next_id: u64,
    dirty: bool,
}

impl Realm {
    /// A new realm with no users and [`kdf::DEFAULT_KDF_ITERATIONS`].
    pub fn new(seed: &[u8; 32], name: &str) -> Result<Realm, AuthError> {
        Ok(Realm {
            key: signing_key(seed),
            name: display_name(name)?,
            kdf_iterations: kdf::DEFAULT_KDF_ITERATIONS,
            users: Vec::new(),
            refresh: Vec::new(),
            fails: HashMap::new(),
            next_id: 1,
            dirty: true,
        })
    }

    /// Set the iteration count of a realm that has no users yet (tests use the minimum).
    pub fn with_kdf(mut self, iterations: u32) -> Realm {
        if self.users.is_empty() {
            self.kdf_iterations = iterations;
        }
        self
    }

    /// A realm from its users file.
    pub fn load(seed: &[u8; 32], name: &str, text: &str) -> Result<Realm, AuthError> {
        let mut r = Realm::new(seed, name)?;
        let bad = |n: usize| AuthError::Storage(format!("users file line {}", n + 1));
        for (n, line) in text.lines().enumerate() {
            let (tag, rest) = wire::word(line.trim_end());
            match tag {
                "kdf" => r.kdf_iterations = rest.trim().parse().map_err(|_| bad(n))?,
                "next" => r.next_id = rest.trim().parse().map_err(|_| bad(n))?,
                "U" => {
                    let w: Vec<&str> = rest.splitn(6, ' ').collect();
                    if w.len() != 6 {
                        return Err(bad(n));
                    }
                    r.users.push(User {
                        id: w[0].parse().map_err(|_| bad(n))?,
                        username: w[1].to_string(),
                        disabled: w[2] == "disabled",
                        key: if w[3] == "-" { None } else { Some(unhex_n(w[3]).ok_or_else(|| bad(n))?) },
                        created_ms: w[4].parse().map_err(|_| bad(n))?,
                        display_name: w[5].to_string(),
                    });
                }
                "R" => {
                    let w: Vec<&str> = rest.split(' ').collect();
                    if w.len() != 3 {
                        return Err(bad(n));
                    }
                    r.refresh.push(RefreshRec {
                        user_id: w[0].parse().map_err(|_| bad(n))?,
                        hash: unhex_n(w[1]).ok_or_else(|| bad(n))?,
                        expires_ms: w[2].parse().map_err(|_| bad(n))?,
                    });
                }
                _ => {}
            }
        }
        r.next_id = r.next_id.max(r.users.iter().map(|u| u.id + 1).max().unwrap_or(1));
        r.dirty = false;
        Ok(r)
    }

    pub fn save_text(&self) -> String {
        let mut s = format!("ecm-auth-users 1\nkdf {}\nnext {}\n", self.kdf_iterations, self.next_id);
        for u in &self.users {
            s.push_str(&format!(
                "U {} {} {} {} {} {}\n",
                u.id,
                u.username,
                if u.disabled { "disabled" } else { "ok" },
                u.key.map(|k| hex(&k)).unwrap_or_else(|| "-".into()),
                u.created_ms,
                u.display_name
            ));
        }
        for t in &self.refresh {
            s.push_str(&format!("R {} {} {}\n", t.user_id, hex(&t.hash), t.expires_ms));
        }
        s
    }

    /// True once after something worth saving changed.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    pub fn public_key(&self) -> [u8; 32] {
        public_key(&self.key)
    }

    pub fn fingerprint(&self) -> String {
        fingerprint(&self.public_key())
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kdf_iterations(&self) -> u32 {
        self.kdf_iterations
    }

    /// The realm key (a `--auth local` backend signs with the same key it is known by).
    pub fn key(&self) -> &SigningKey {
        &self.key
    }

    pub fn users(&self) -> &[User] {
        &self.users
    }

    pub fn user(&self, name: &str) -> Option<&User> {
        let name = name.to_ascii_lowercase();
        self.users.iter().find(|u| u.username == name)
    }

    fn user_mut(&mut self, name: &str) -> Result<&mut User, AuthError> {
        let name = name.trim().to_ascii_lowercase();
        self.users.iter_mut().find(|u| u.username == name).ok_or(AuthError::NoSuchUser(name))
    }

    /// Valid (unexpired) refresh tokens of a user: their signed-in devices.
    pub fn sessions(&self, name: &str, now_ms: u64) -> usize {
        match self.user(name) {
            Some(u) => self.refresh.iter().filter(|t| t.user_id == u.id && t.expires_ms > now_ms).count(),
            None => 0,
        }
    }

    /// `authd disable`: no sign-in, refresh or token until enabled; signs the user out everywhere.
    pub fn disable(&mut self, name: &str) -> Result<(), AuthError> {
        let u = self.user_mut(name)?;
        u.disabled = true;
        let id = u.id;
        self.refresh.retain(|t| t.user_id != id);
        self.dirty = true;
        Ok(())
    }

    pub fn enable(&mut self, name: &str) -> Result<(), AuthError> {
        self.user_mut(name)?.disabled = false;
        self.dirty = true;
        Ok(())
    }

    /// `authd reset`: clear the key (the next sign-in sets a new password), sign out everywhere.
    pub fn reset(&mut self, name: &str) -> Result<(), AuthError> {
        let u = self.user_mut(name)?;
        u.key = None;
        let (id, uname) = (u.id, u.username.clone());
        self.refresh.retain(|t| t.user_id != id);
        self.fails.remove(&format!("u:{uname}"));
        self.dirty = true;
        Ok(())
    }

    /// `authd revoke`: every refresh token of the user (signed out on every device).
    pub fn revoke(&mut self, name: &str) -> Result<usize, AuthError> {
        let id = self.user_mut(name)?.id;
        let before = self.refresh.len();
        self.refresh.retain(|t| t.user_id != id);
        self.dirty = true;
        Ok(before - self.refresh.len())
    }

    // -------------------------------------------------------------- rate limiting

    fn wait_s(&self, key: &str, free: u32, now_ms: u64) -> Option<u64> {
        let f = self.fails.get(key)?;
        if f.count < free || now_ms.saturating_sub(f.last_ms) > FAIL_MEMORY_MS {
            return None;
        }
        let lock = LOCK_BASE_MS.saturating_mul(1u64 << (f.count - free).min(20)).min(LOCK_MAX_MS);
        let until = f.last_ms + lock;
        (now_ms < until).then(|| (until - now_ms).div_ceil(1000))
    }

    /// Refuse a sign-in while the username or the address is locked out.
    fn check_locks(&self, user: &str, addr: &str, now_ms: u64) -> Result<(), AuthError> {
        let u = self.wait_s(&format!("u:{user}"), FREE_TRIES_USER, now_ms);
        let a = self.wait_s(&format!("a:{addr}"), FREE_TRIES_ADDR, now_ms);
        match u.max(a) {
            Some(wait_s) => Err(AuthError::Locked { wait_s }),
            None => Ok(()),
        }
    }

    fn failed(&mut self, user: &str, addr: &str, now_ms: u64) {
        if self.fails.len() > 4096 {
            self.fails.retain(|_, f| now_ms.saturating_sub(f.last_ms) <= FAIL_MEMORY_MS);
        }
        for key in [format!("u:{user}"), format!("a:{addr}")] {
            let f = self.fails.entry(key).or_insert(Fails { count: 0, last_ms: now_ms });
            if now_ms.saturating_sub(f.last_ms) > FAIL_MEMORY_MS {
                f.count = 0;
            }
            f.count += 1;
            f.last_ms = now_ms;
        }
    }

    fn succeeded(&mut self, user: &str, addr: &str) {
        self.fails.remove(&format!("u:{user}"));
        self.fails.remove(&format!("a:{addr}"));
    }

    // -------------------------------------------------------------- tokens

    fn new_refresh(&mut self, user_id: u64, now_ms: u64) -> (String, [u8; 32], u64) {
        let token = hex(&random::<32>());
        let hash = ecm_ssh_crypto::sha256(token.as_bytes());
        let expires_ms = now_ms + REFRESH_LIFETIME_MS;
        self.refresh.retain(|t| t.expires_ms > now_ms);
        let mine: Vec<usize> = (0..self.refresh.len()).filter(|&i| self.refresh[i].user_id == user_id).collect();
        if mine.len() >= MAX_REFRESH_PER_USER {
            let oldest = *mine.iter().min_by_key(|&&i| self.refresh[i].expires_ms).unwrap();
            self.refresh.remove(oldest);
        }
        self.refresh.push(RefreshRec { user_id, hash, expires_ms });
        self.dirty = true;
        (token, hash, expires_ms)
    }

    /// A token for `audience` (a backend's fingerprint), bound to `session_pub`.
    pub fn mint(&self, user: &User, audience: &str, session_pub: &[u8; 32], now_ms: u64) -> (String, u64) {
        let exp = now_ms + TOKEN_LIFETIME_MS;
        let claims = crate::Claims {
            realm_fp: self.fingerprint(),
            user_id: user.id,
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            audience: audience.to_string(),
            cnf: *session_pub,
            issued_ms: now_ms,
            expires_ms: exp,
        };
        (claims.mint(&self.key), exp)
    }
}

// ------------------------------------------------------------------ connections

/// What to do after feeding a connection.
#[derive(Debug, Default)]
pub struct ConnOut {
    /// Bytes to send to the peer.
    pub send: Vec<u8>,
    /// Hang up (after sending).
    pub close: bool,
    /// Lines for the server's log.
    pub log: Vec<String>,
}

struct SignedIn {
    user_id: u64,
    refresh_hash: [u8; 32],
    session_pub: [u8; 32],
}

enum State {
    Hello,
    Open { ch: Channel, th: [u8; 32], user: Option<SignedIn> },
    Closed,
}

/// One client connection: feed it bytes, send what comes back.
pub struct ServerConn {
    peer: String,
    addr: String,
    frames: FrameReader,
    state: State,
    tries: u8,
}

impl ServerConn {
    /// `peer` is the client's address (`ip:port`); failures are counted per ip.
    pub fn new(peer: &str) -> ServerConn {
        let addr = peer.rsplit_once(':').map(|(a, _)| a).unwrap_or(peer).to_string();
        ServerConn { peer: peer.to_string(), addr, frames: FrameReader::default(), state: State::Hello, tries: 0 }
    }

    /// The signed-in user's id, if any.
    pub fn user_id(&self) -> Option<u64> {
        match &self.state {
            State::Open { user: Some(u), .. } => Some(u.user_id),
            _ => None,
        }
    }

    /// Bytes received from the peer at `now_ms`.
    pub fn receive(&mut self, realm: &mut Realm, data: &[u8], now_ms: u64) -> ConnOut {
        let mut out = ConnOut::default();
        if matches!(self.state, State::Closed) {
            out.close = true;
            return out;
        }
        self.frames.push(data);
        loop {
            match self.frames.next_frame() {
                Ok(Some(f)) => self.frame(realm, &f, now_ms, &mut out),
                Ok(None) => break,
                Err(e) => {
                    out.log.push(format!("{}: {e}", self.peer));
                    out.close = true;
                }
            }
            if out.close {
                self.state = State::Closed;
                break;
            }
        }
        out
    }

    fn frame(&mut self, realm: &mut Realm, f: &[u8], now_ms: u64, out: &mut ConnOut) {
        match &mut self.state {
            State::Hello => {
                let ce = match wire::parse_client_hello(f) {
                    Ok(ce) => ce,
                    Err(e) => {
                        out.log.push(format!("{}: {e}", self.peer));
                        out.close = true;
                        return;
                    }
                };
                let (se_pub, se_secret) = ecm_ssh_crypto::x25519_generate_keypair();
                let se = se_pub.to_bytes();
                let realm_pub = realm.public_key();
                let th = wire::transcript(&ce, &se, &realm_pub, &realm.name, realm.kdf_iterations);
                let hello = ServerHello {
                    se,
                    realm_pub,
                    kdf_iterations: realm.kdf_iterations,
                    realm_name: realm.name.clone(),
                    sig: sign(&realm.key, &[&th]),
                };
                let dh = ecm_ssh_crypto::x25519_diffie_hellman(&se_secret, &ecm_ssh_crypto::X25519Public::from(ce));
                match Channel::new(&dh, &th, false) {
                    Ok(ch) => {
                        out.send.extend(wire::frame(&hello.encode()));
                        self.state = State::Open { ch, th, user: None };
                    }
                    Err(e) => {
                        out.log.push(format!("{}: {e}", self.peer));
                        out.close = true;
                    }
                }
            }
            State::Open { ch, th, user } => {
                let line = match ch.open(f).ok().and_then(|p| String::from_utf8(p).ok()) {
                    Some(l) => l,
                    None => {
                        out.log.push(format!("{}: a frame failed to decrypt; hanging up", self.peer));
                        out.close = true;
                        return;
                    }
                };
                let (cmd, rest) = wire::word(&line);
                let signing_in = user.is_none();
                let answer = match (cmd, signing_in) {
                    ("LOGIN", true) => login(realm, th, &self.addr, rest, now_ms).map(|(r, s)| {
                        *user = Some(s);
                        r
                    }),
                    ("SIGNUP", true) => signup(realm, th, &self.addr, rest, now_ms).map(|(r, s)| {
                        *user = Some(s);
                        r
                    }),
                    ("REFRESH", true) => refresh(realm, &self.addr, rest, now_ms).map(|(r, s)| {
                        *user = Some(s);
                        r
                    }),
                    (_, true) => Err(AuthError::NotSignedIn(realm.name.clone())),
                    ("TOKEN", false) => token(realm, user.as_ref().unwrap(), rest, now_ms),
                    ("PASSWD", false) => passwd(realm, th, user.as_ref().unwrap(), rest),
                    ("WHOAMI", false) => whoami(realm, user.as_ref().unwrap()),
                    ("LOGOUT", false) => {
                        let s = user.take().unwrap();
                        realm.refresh.retain(|t| t.hash != s.refresh_hash);
                        realm.dirty = true;
                        out.close = true;
                        Ok("BYE".to_string())
                    }
                    _ => Err(AuthError::Protocol(format!("unknown request {}", cmd.chars().take(16).collect::<String>()))),
                };
                let who = rest.split(' ').next().unwrap_or("").chars().take(16).collect::<String>();
                match answer {
                    Ok(reply) => {
                        if signing_in {
                            let uname = user.as_ref().and_then(|s| realm.users.iter().find(|u| u.id == s.user_id)).map(|u| u.username.clone());
                            out.log.push(format!("{} {} from {}", cmd.to_ascii_lowercase(), uname.unwrap_or_default(), self.peer));
                        } else if cmd == "PASSWD" || cmd == "LOGOUT" {
                            out.log.push(format!("{} from {}", cmd.to_ascii_lowercase(), self.peer));
                        }
                        out.send.extend(ch.seal(reply.as_bytes()));
                    }
                    Err(e) => {
                        if signing_in {
                            out.log.push(format!("failed {} {} from {}: {e}", cmd.to_ascii_lowercase(), who, self.peer));
                        }
                        out.send.extend(ch.seal(format!("FAIL {}", e.fail_words()).as_bytes()));
                        if matches!(e, AuthError::Protocol(_)) {
                            out.close = true;
                        }
                        if signing_in {
                            self.tries += 1;
                            if self.tries >= TRIES_PER_CONNECTION {
                                out.close = true;
                            }
                        }
                    }
                }
            }
            State::Closed => out.close = true,
        }
    }
}

fn bad(what: &str) -> AuthError {
    AuthError::Protocol(format!("bad {what}"))
}

fn welcome(realm: &mut Realm, user_id: u64, session_pub: [u8; 32], now_ms: u64, reuse: Option<(String, [u8; 32])>) -> (String, SignedIn) {
    let (token, hash, expires) = match reuse {
        Some((token, hash)) => {
            // A refresh renews the token it used.
            let exp = now_ms + REFRESH_LIFETIME_MS;
            if let Some(t) = realm.refresh.iter_mut().find(|t| t.hash == hash) {
                t.expires_ms = exp;
            }
            realm.dirty = true;
            (token, hash, exp)
        }
        None => realm.new_refresh(user_id, now_ms),
    };
    let u = realm.users.iter().find(|u| u.id == user_id).unwrap();
    let reply = format!("WELCOME {} {} {} {} {}", u.id, u.username, token, expires, u.display_name);
    (reply, SignedIn { user_id, refresh_hash: hash, session_pub })
}

fn login(realm: &mut Realm, th: &[u8; 32], addr: &str, rest: &str, now_ms: u64) -> Result<(String, SignedIn), AuthError> {
    let w: Vec<&str> = rest.split(' ').collect();
    if w.len() != 3 {
        return Err(bad("LOGIN"));
    }
    let name = username(w[0])?;
    let session_pub: [u8; 32] = unhex_n(w[1]).ok_or_else(|| bad("session key"))?;
    let sig: [u8; 64] = unhex_n(w[2]).ok_or_else(|| bad("signature"))?;
    realm.check_locks(&name, addr, now_ms)?;
    let found = realm.user(&name).cloned();
    let ok = match &found {
        Some(User { key: None, disabled: false, .. }) => return Err(AuthError::NeedsNewPassword),
        Some(User { key: Some(k), .. }) => verify(k, &[LOGIN_LABEL, th], &sig),
        _ => false,
    };
    if !ok {
        realm.failed(&name, addr, now_ms);
        return Err(AuthError::WrongPassword);
    }
    let user = found.unwrap();
    if user.disabled {
        return Err(AuthError::Disabled);
    }
    realm.succeeded(&name, addr);
    Ok(welcome(realm, user.id, session_pub, now_ms, None))
}

fn signup(realm: &mut Realm, th: &[u8; 32], addr: &str, rest: &str, now_ms: u64) -> Result<(String, SignedIn), AuthError> {
    let w: Vec<&str> = rest.splitn(5, ' ').collect();
    if w.len() != 5 {
        return Err(bad("SIGNUP"));
    }
    let name = username(w[0])?;
    let account: [u8; 32] = unhex_n(w[1]).ok_or_else(|| bad("account key"))?;
    let session_pub: [u8; 32] = unhex_n(w[2]).ok_or_else(|| bad("session key"))?;
    let sig: [u8; 64] = unhex_n(w[3]).ok_or_else(|| bad("signature"))?;
    let display = display_name(w[4])?;
    realm.check_locks(&name, addr, now_ms)?;
    if !verify(&account, &[SIGNUP_LABEL, th], &sig) {
        realm.failed(&name, addr, now_ms);
        return Err(bad("proof of the account key"));
    }
    let id = match realm.users.iter_mut().find(|u| u.username == name) {
        Some(u) if u.key.is_some() => return Err(AuthError::UsernameTaken(name)),
        Some(u) if u.disabled => return Err(AuthError::Disabled),
        // Reset by the operator: this sign-in sets the new password; the account (id,
        // display name) stays the same.
        Some(u) => {
            u.key = Some(account);
            u.id
        }
        None => {
            if realm.users.len() >= MAX_USERS {
                return Err(AuthError::Server("this realm is full".into()));
            }
            let id = realm.next_id;
            realm.next_id += 1;
            realm.users.push(User { id, username: name.clone(), display_name: display, key: Some(account), disabled: false, created_ms: now_ms });
            id
        }
    };
    realm.dirty = true;
    realm.succeeded(&name, addr);
    Ok(welcome(realm, id, session_pub, now_ms, None))
}

fn refresh(realm: &mut Realm, addr: &str, rest: &str, now_ms: u64) -> Result<(String, SignedIn), AuthError> {
    let w: Vec<&str> = rest.split(' ').collect();
    if w.len() != 3 {
        return Err(bad("REFRESH"));
    }
    let name = username(w[0])?;
    let session_pub: [u8; 32] = unhex_n(w[1]).ok_or_else(|| bad("session key"))?;
    let token = w[2].to_string();
    realm.check_locks(&name, addr, now_ms)?;
    let hash = ecm_ssh_crypto::sha256(token.as_bytes());
    let user = realm.user(&name).cloned();
    let valid = user.as_ref().is_some_and(|u| realm.refresh.iter().any(|t| t.user_id == u.id && t.hash == hash && t.expires_ms > now_ms));
    if !valid {
        realm.failed(&name, addr, now_ms);
        return Err(AuthError::SignInExpired);
    }
    let user = user.unwrap();
    if user.disabled {
        return Err(AuthError::Disabled);
    }
    realm.succeeded(&name, addr);
    Ok(welcome(realm, user.id, session_pub, now_ms, Some((token, hash))))
}

fn current(realm: &Realm, s: &SignedIn) -> Result<User, AuthError> {
    let u = realm.users.iter().find(|u| u.id == s.user_id).cloned().ok_or(AuthError::SignInExpired)?;
    if u.disabled {
        return Err(AuthError::Disabled);
    }
    // Revoked (or reset) while this connection was open.
    if !realm.refresh.iter().any(|t| t.hash == s.refresh_hash) {
        return Err(AuthError::SignInExpired);
    }
    Ok(u)
}

fn token(realm: &mut Realm, s: &SignedIn, rest: &str, now_ms: u64) -> Result<String, AuthError> {
    let aud = rest.trim();
    if !is_fingerprint(aud) {
        return Err(bad("audience"));
    }
    let u = current(realm, s)?;
    let (token, exp) = realm.mint(&u, aud, &s.session_pub, now_ms);
    Ok(format!("TOKEN {exp} {token}"))
}

fn passwd(realm: &mut Realm, th: &[u8; 32], s: &SignedIn, rest: &str) -> Result<String, AuthError> {
    let w: Vec<&str> = rest.split(' ').collect();
    if w.len() != 3 {
        return Err(bad("PASSWD"));
    }
    let new_pub: [u8; 32] = unhex_n(w[0]).ok_or_else(|| bad("key"))?;
    let new_sig: [u8; 64] = unhex_n(w[1]).ok_or_else(|| bad("signature"))?;
    let old_sig: [u8; 64] = unhex_n(w[2]).ok_or_else(|| bad("signature"))?;
    let u = current(realm, s)?;
    let old = u.key.ok_or(AuthError::NeedsNewPassword)?;
    if !verify(&old, &[PASSWD_LABEL, th, &new_pub], &old_sig) {
        return Err(AuthError::WrongPassword);
    }
    if !verify(&new_pub, &[PASSWD_LABEL, th], &new_sig) {
        return Err(bad("proof of the new key"));
    }
    if let Some(user) = realm.users.iter_mut().find(|x| x.id == u.id) {
        user.key = Some(new_pub);
    }
    // Other devices sign in again with the new password.
    realm.refresh.retain(|t| t.user_id != u.id || t.hash == s.refresh_hash);
    realm.dirty = true;
    Ok("OK".into())
}

fn whoami(realm: &Realm, s: &SignedIn) -> Result<String, AuthError> {
    let u = current(realm, s)?;
    Ok(format!("ME {} {} {}", u.id, u.username, u.display_name))
}
