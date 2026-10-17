//! What a client keeps: the pinned realms and one saved login per realm key. Each app
//! keeps its own, in its own data folder (`/var/apps/<app>/accounts/`); a script keeps
//! them in `.auth/` next to it. Nothing is shared between apps (decided).
//!
//! Files in that folder (all text; [`Files`] reads and writes them, so this module
//! never touches the disk itself):
//!
//! - `pins`: one realm per line, `<fp> <host:port> <realm name...>`. A realm may have
//!   several lines (several addresses). Pins can exist without a login: a village
//!   computer ships with the village realm pinned.
//! - `<fp>`: the login for that realm ([`Login::to_text`]).
//! - `last`: the fingerprint signed in to last.
//!
//! Pinning rules ([`Pins::check_pin`]): a realm seen for the first time is pinned when
//! the player signs in or up (trust on first use, like SSH). A realm whose name **or**
//! address matches a pin with a different key is [`PinCheck::Changed`] and refused, and
//! its saved login is never offered to the new key, until the player removes the old pin
//! by hand ([`AccountStore::remove_pin`]). There is no "trust the new key" path.

use std::collections::BTreeMap;

use crate::client::{handshake, ServerInfo, Session, Stream};
use crate::{endpoint, hex, is_fingerprint, kdf, public_key, random, signing_key, unhex_n, AuthError, SigningKey};

pub const PINS_FILE: &str = "pins";
pub const LAST_FILE: &str = "last";

/// One pinned realm address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub fp: String,
    pub host: String,
    pub name: String,
}

/// The answer to "may I talk to this realm?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinCheck {
    /// Known key: go ahead (a saved login may be used).
    Pinned,
    /// Never seen: show name, host and fingerprint; pin after the player signs in.
    FirstContact,
    /// A pin with the same name or host has another key. Refuse.
    Changed { old_fp: String, name: String, host: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pins {
    pub pins: Vec<Pin>,
}

impl Pins {
    /// The `pins` file (bad lines are skipped; `#` starts a comment line).
    pub fn parse(text: &str) -> Pins {
        let mut pins = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let w: Vec<&str> = line.splitn(3, ' ').collect();
            if w.len() == 3 && is_fingerprint(w[0]) {
                pins.push(Pin { fp: w[0].to_string(), host: w[1].to_string(), name: w[2].trim().to_string() });
            }
        }
        Pins { pins }
    }

    pub fn to_text(&self) -> String {
        self.pins.iter().map(|p| format!("{} {} {}\n", p.fp, p.host, p.name)).collect()
    }

    /// Whether `fp`, presenting itself as `name` at `host`, may be used.
    pub fn check_pin(&self, name: &str, host: &str, fp: &str) -> PinCheck {
        let host = endpoint(host).unwrap_or_else(|_| host.to_string());
        if let Some(p) = self.pins.iter().find(|p| p.fp != fp && (p.name.eq_ignore_ascii_case(name.trim()) || p.host == host)) {
            return PinCheck::Changed { old_fp: p.fp.clone(), name: p.name.clone(), host: p.host.clone() };
        }
        if self.is_pinned(fp) {
            PinCheck::Pinned
        } else {
            PinCheck::FirstContact
        }
    }

    pub fn is_pinned(&self, fp: &str) -> bool {
        self.pins.iter().any(|p| p.fp == fp)
    }

    /// Pin `fp` at `host` (a known realm at a new address gets another line).
    pub fn add(&mut self, fp: &str, host: &str, name: &str) {
        let host = endpoint(host).unwrap_or_else(|_| host.to_string());
        match self.pins.iter_mut().find(|p| p.fp == fp && p.host == host) {
            Some(p) => p.name = name.to_string(),
            None => self.pins.push(Pin { fp: fp.to_string(), host, name: name.to_string() }),
        }
    }

    /// Forget every line of `fp`. True if there was one.
    pub fn remove_pin(&mut self, fp: &str) -> bool {
        let n = self.pins.len();
        self.pins.retain(|p| p.fp != fp);
        self.pins.len() != n
    }

    /// The realm pinned at `host`, if any.
    pub fn at_host(&self, host: &str) -> Option<&Pin> {
        let host = endpoint(host).unwrap_or_else(|_| host.to_string());
        self.pins.iter().find(|p| p.host == host)
    }
}

/// A saved login for one realm (the file named after its fingerprint).
///
/// ```text
/// ecm-auth-login 1
/// realm <fp>
/// name <realm name>
/// host <host:port>
/// user <username>
/// id <user id>
/// display <display name>
/// account <account key seed, hex>
/// session <session key seed, hex>
/// refresh <refresh token>
/// expires <refresh token expiry, ms>
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub realm_fp: String,
    pub realm_name: String,
    pub host: String,
    pub username: String,
    pub user_id: u64,
    pub display_name: String,
    pub account_seed: [u8; 32],
    pub session_seed: [u8; 32],
    pub refresh_token: String,
    pub refresh_expires_ms: u64,
}

impl Login {
    pub fn to_text(&self) -> String {
        format!(
            "ecm-auth-login 1\nrealm {}\nname {}\nhost {}\nuser {}\nid {}\ndisplay {}\naccount {}\nsession {}\nrefresh {}\nexpires {}\n",
            self.realm_fp,
            self.realm_name,
            self.host,
            self.username,
            self.user_id,
            self.display_name,
            hex(&self.account_seed),
            hex(&self.session_seed),
            self.refresh_token,
            self.refresh_expires_ms
        )
    }

    pub fn parse(text: &str) -> Result<Login, AuthError> {
        let mut m: BTreeMap<&str, &str> = BTreeMap::new();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once(' ') {
                m.insert(k, v.trim_end());
            }
        }
        let bad = |k: &str| AuthError::Storage(format!("saved login: bad {k}"));
        let get = |k: &str| m.get(k).copied().ok_or_else(|| bad(k));
        if get("ecm-auth-login")? != "1" {
            return Err(bad("version"));
        }
        Ok(Login {
            realm_fp: get("realm")?.to_string(),
            realm_name: get("name")?.to_string(),
            host: get("host")?.to_string(),
            username: get("user")?.to_string(),
            user_id: get("id")?.parse().map_err(|_| bad("id"))?,
            display_name: get("display")?.to_string(),
            account_seed: unhex_n(get("account")?).ok_or_else(|| bad("account"))?,
            session_seed: unhex_n(get("session")?).ok_or_else(|| bad("session"))?,
            refresh_token: get("refresh")?.to_string(),
            refresh_expires_ms: get("expires")?.parse().map_err(|_| bad("expires"))?,
        })
    }

    pub fn session_key(&self) -> SigningKey {
        signing_key(&self.session_seed)
    }

    pub fn account_key(&self) -> SigningKey {
        signing_key(&self.account_seed)
    }
}

/// The folder of one app's (or script's) accounts. Names are plain file names.
pub trait Files {
    fn read(&self, name: &str) -> Option<String>;
    fn write(&mut self, name: &str, text: &str) -> Result<(), AuthError>;
    fn remove(&mut self, name: &str);
}

impl<F: Files + ?Sized> Files for Box<F> {
    fn read(&self, name: &str) -> Option<String> {
        (**self).read(name)
    }
    fn write(&mut self, name: &str, text: &str) -> Result<(), AuthError> {
        (**self).write(name, text)
    }
    fn remove(&mut self, name: &str) {
        (**self).remove(name)
    }
}

/// Files in memory (tests, and callers that persist some other way).
#[derive(Debug, Clone, Default)]
pub struct MemFiles(pub BTreeMap<String, String>);

impl Files for MemFiles {
    fn read(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }
    fn write(&mut self, name: &str, text: &str) -> Result<(), AuthError> {
        self.0.insert(name.to_string(), text.to_string());
        Ok(())
    }
    fn remove(&mut self, name: &str) {
        self.0.remove(name);
    }
}

/// Pins plus saved logins, and the sign-in flows that apply the pinning rules.
pub struct AccountStore<F: Files> {
    pub files: F,
}

impl<F: Files> AccountStore<F> {
    pub fn new(files: F) -> Self {
        AccountStore { files }
    }

    pub fn pins(&self) -> Pins {
        Pins::parse(&self.files.read(PINS_FILE).unwrap_or_default())
    }

    pub fn save_pins(&mut self, pins: &Pins) -> Result<(), AuthError> {
        self.files.write(PINS_FILE, &pins.to_text())
    }

    /// Pin a realm without signing in (a village computer's built-in realm).
    pub fn pin(&mut self, fp: &str, host: &str, name: &str) -> Result<(), AuthError> {
        let mut pins = self.pins();
        pins.add(fp, host, name);
        self.save_pins(&pins)
    }

    pub fn login(&self, fp: &str) -> Option<Login> {
        if !is_fingerprint(fp) {
            return None;
        }
        Login::parse(&self.files.read(fp)?).ok()
    }

    pub fn save_login(&mut self, login: &Login) -> Result<(), AuthError> {
        self.files.write(&login.realm_fp, &login.to_text())?;
        self.files.write(LAST_FILE, &login.realm_fp)
    }

    /// The realm signed in to last.
    pub fn last(&self) -> Option<String> {
        self.files.read(LAST_FILE).map(|s| s.trim().to_string()).filter(|s| is_fingerprint(s))
    }

    /// The saved login for `host` (through its pin), or the last one when `host` is None.
    pub fn login_for(&self, host: Option<&str>) -> Result<Login, AuthError> {
        let fp = match host {
            Some(h) => self.pins().at_host(h).map(|p| p.fp.clone()).ok_or_else(|| AuthError::NotSignedIn(h.to_string()))?,
            None => self.last().ok_or_else(|| AuthError::NotSignedIn("any realm".into()))?,
        };
        self.login(&fp).ok_or_else(|| AuthError::NotSignedIn(host.unwrap_or("any realm").to_string()))
    }

    /// Remove a pin by hand (and the login saved for that key). The only way past
    /// [`PinCheck::Changed`].
    pub fn remove_pin(&mut self, fp: &str) -> Result<bool, AuthError> {
        let mut pins = self.pins();
        let had = pins.remove_pin(fp);
        self.save_pins(&pins)?;
        if is_fingerprint(fp) {
            self.files.remove(fp);
        }
        if self.last().as_deref() == Some(fp) {
            self.files.remove(LAST_FILE);
        }
        Ok(had)
    }

    /// Handshake with the realm at `host` and apply the pins. `Changed` is an error here.
    pub fn connect<S: Stream>(&self, stream: S, host: &str) -> Result<(Session<S>, PinCheck), AuthError> {
        let host = endpoint(host)?;
        let (session, info) = handshake(stream)?;
        let check = self.pins().check_pin(&info.realm_name, &host, &info.realm_fp);
        if let PinCheck::Changed { old_fp, name, host } = &check {
            return Err(AuthError::RealmChanged { name: name.clone(), host: host.clone(), old_fp: old_fp.clone(), new_fp: info.realm_fp });
        }
        Ok((session, check))
    }

    /// The realm at `host` without signing in (refused if it changed).
    pub fn fingerprint<S: Stream>(&self, stream: S, host: &str) -> Result<(ServerInfo, PinCheck), AuthError> {
        let (session, check) = self.connect(stream, host)?;
        Ok((session.info().clone(), check))
    }

    fn signed_in(&mut self, info: &ServerInfo, host: &str, w: crate::Welcome, account_seed: [u8; 32], session_seed: [u8; 32]) -> Result<Login, AuthError> {
        let host = endpoint(host)?;
        let mut pins = self.pins();
        pins.add(&info.realm_fp, &host, &info.realm_name);
        self.save_pins(&pins)?;
        let login = Login {
            realm_fp: info.realm_fp.clone(),
            realm_name: info.realm_name.clone(),
            host,
            username: w.username,
            user_id: w.user_id,
            display_name: w.display_name,
            account_seed,
            session_seed,
            refresh_token: w.refresh_token,
            refresh_expires_ms: w.refresh_expires_ms,
        };
        self.save_login(&login)?;
        Ok(login)
    }

    /// Sign in with a password. An account the operator reset takes this password as its new one.
    pub fn sign_in<S: Stream>(&mut self, stream: S, host: &str, username: &str, password: &str) -> Result<Login, AuthError> {
        if password.is_empty() {
            return Err(AuthError::BadPassword);
        }
        let u = crate::username(username)?;
        let (mut session, _) = self.connect(stream, host)?;
        let info = session.info().clone();
        let seed = kdf::account_seed(password, &info.realm_pub, &u, info.kdf_iterations);
        let account = signing_key(&seed);
        let session_seed: [u8; 32] = random();
        let session_pub = public_key(&signing_key(&session_seed));
        let w = match session.login(&u, &account, &session_pub) {
            Err(AuthError::NeedsNewPassword) => session.signup(&u, &u, &account, &session_pub)?,
            other => other?,
        };
        self.signed_in(&info, host, w, seed, session_seed)
    }

    /// Make an account.
    pub fn sign_up<S: Stream>(&mut self, stream: S, host: &str, username: &str, password: &str, display: Option<&str>) -> Result<Login, AuthError> {
        if password.is_empty() {
            return Err(AuthError::BadPassword);
        }
        let u = crate::username(username)?;
        let display = crate::display_name(display.unwrap_or(&u))?;
        let (mut session, _) = self.connect(stream, host)?;
        let info = session.info().clone();
        let seed = kdf::account_seed(password, &info.realm_pub, &u, info.kdf_iterations);
        let session_seed: [u8; 32] = random();
        let session_pub = public_key(&signing_key(&session_seed));
        let w = session.signup(&u, &display, &signing_key(&seed), &session_pub)?;
        self.signed_in(&info, host, w, seed, session_seed)
    }

    /// Open a signed-in session with a saved login (refresh token), checking that the
    /// realm still has the login's key.
    pub fn resume<S: Stream>(&mut self, stream: S, login: &mut Login) -> Result<Session<S>, AuthError> {
        let (mut session, _) = self.connect(stream, &login.host)?;
        if session.info().realm_fp != login.realm_fp {
            // Not pinned under this host any more, but never offer the login to another key.
            return Err(AuthError::RealmChanged {
                name: login.realm_name.clone(),
                host: login.host.clone(),
                old_fp: login.realm_fp.clone(),
                new_fp: session.info().realm_fp.clone(),
            });
        }
        let w = session.refresh(&login.username, &login.refresh_token, &public_key(&login.session_key()))?;
        login.refresh_token = w.refresh_token;
        login.refresh_expires_ms = w.refresh_expires_ms;
        login.display_name = w.display_name;
        self.save_login(login)?;
        Ok(session)
    }

    /// A token for the backend `audience` from the saved login of realm `fp`: (token, expires ms).
    pub fn token<S: Stream>(&mut self, stream: S, fp: &str, audience: &str) -> Result<(String, u64), AuthError> {
        let mut login = self.login(fp).ok_or_else(|| AuthError::NotSignedIn(fp.to_string()))?;
        let mut session = self.resume(stream, &mut login)?;
        session.token(audience)
    }

    /// The proof for a backend's connect nonce, with realm `fp`'s session key.
    pub fn prove(&self, fp: &str, nonce: &[u8; 32], audience: &str) -> Result<[u8; 64], AuthError> {
        let login = self.login(fp).ok_or_else(|| AuthError::NotSignedIn(fp.to_string()))?;
        let aud = crate::parse_fingerprint(audience).ok_or_else(|| AuthError::Protocol(format!("'{audience}' is not a fingerprint")))?;
        Ok(crate::prove(&login.session_key(), nonce, &aud))
    }

    /// Sign out of realm `fp`: revoke the refresh token at the realm (best effort) and
    /// forget the login. The pin stays.
    pub fn logout<S: Stream>(&mut self, stream: Option<S>, fp: &str) -> Result<(), AuthError> {
        let mut login = self.login(fp).ok_or_else(|| AuthError::NotSignedIn(fp.to_string()))?;
        let result = match stream {
            Some(s) => self.resume(s, &mut login).and_then(|mut session| session.logout()),
            None => Ok(()),
        };
        self.files.remove(fp);
        if self.last().as_deref() == Some(fp) {
            self.files.remove(LAST_FILE);
        }
        match result {
            Ok(()) | Err(AuthError::SignInExpired) | Err(AuthError::Closed) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Change the password of the saved login for `fp`.
    pub fn passwd<S: Stream>(&mut self, stream: S, fp: &str, new_password: &str) -> Result<(), AuthError> {
        if new_password.is_empty() {
            return Err(AuthError::BadPassword);
        }
        let mut login = self.login(fp).ok_or_else(|| AuthError::NotSignedIn(fp.to_string()))?;
        let mut session = self.resume(stream, &mut login)?;
        let info = session.info().clone();
        let seed = kdf::account_seed(new_password, &info.realm_pub, &login.username, info.kdf_iterations);
        session.passwd(&login.account_key(), &signing_key(&seed))?;
        login.account_seed = seed;
        self.save_login(&login)
    }
}
