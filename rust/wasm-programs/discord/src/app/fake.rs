//! A scripted network of Discord backends and realms, and a scripted sound card, for
//! tests and the golden-session recorder. Each fake backend is the real backend state
//! machine (`ecm_discord::server::Backend`) and each realm the real
//! `ecm_auth::Realm` behind a real `ServerConn`, so the app talks the real protocols.
//!
//! A backend added with [`FakeNet::add`] runs its own realm (`--auth local`, sign-ins on
//! port 7779); [`FakeNet::add_realm`] + [`FakeNet::add_shared`] make backends that share
//! an Auth server (`authd` at port 7600).

use super::{Audio, Net};
use ecm_auth::{signing_key, AuthError, RealmRef, ServerConn, SigningKey};
use ecm_discord::proto::Greeting;
use ecm_discord::server::{AuthSetup, Backend, Out};
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

/// The local realm's port (`discordd` PORT+2).
pub const LOCAL_REALM_PORT: u16 = 7779;

/// One realm on the fake network.
pub struct FakeRealm {
    pub host: String,
    pub port: u16,
    pub realm: Rc<RefCell<ecm_auth::Realm>>,
}

/// One backend on the fake network.
pub struct FakeBackend {
    pub host: String,
    pub port: u16,
    pub backend: Backend,
    /// The backend's own key (its fingerprint is the tokens' audience).
    pub key: SigningKey,
    /// Index of the realm it trusts.
    pub realm: usize,
    /// Lines to clients that haven't been read yet, per connection handle.
    pub pending: BTreeMap<i32, Vec<u8>>,
    /// Found by a network search.
    pub announced: bool,
    /// Datagrams the backend sent to the app's voice socket.
    pub udp_to_app: VecDeque<Vec<u8>>,
}

/// A network with any number of backends and realms.
#[derive(Default)]
pub struct FakeNet {
    pub backends: Vec<FakeBackend>,
    pub realms: Vec<FakeRealm>,
    /// Handle -> backend index.
    pub conns: BTreeMap<i32, usize>,
    /// The greeting each other-user connection got.
    pub greetings: BTreeMap<i32, Greeting>,
    next: i32,
    keys: u8,
    pub now: u64,
    /// Searches made.
    pub searches: usize,
    pub refuse: Vec<String>,
    /// The voice socket: (backend index) it talks to.
    pub voice_to: Option<usize>,
    pub voice_sent: Vec<Vec<u8>>,
    /// Lines every handle sent (tests).
    pub sent: Vec<(i32, String)>,
    /// Connections the app opened to realms (tests: "silent" sign-ins still talk to the realm).
    pub realm_connects: usize,
}

fn new_realm(seed: &[u8; 32], name: &str) -> Rc<RefCell<ecm_auth::Realm>> {
    Rc::new(RefCell::new(ecm_auth::Realm::new(seed, name).unwrap().with_kdf(ecm_auth::MIN_KDF_ITERATIONS)))
}

impl FakeNet {
    fn seed(&mut self) -> [u8; 32] {
        self.keys += 1;
        [0x40 + self.keys; 32]
    }

    /// Add a backend called `name` at `host` (port 7777) with its own accounts
    /// (`--auth local`); `announced` = found by a search.
    pub fn add(&mut self, host: &str, name: &str, announced: bool) -> usize {
        let seed = self.seed();
        let key = signing_key(&seed);
        let realm = new_realm(&seed, name);
        self.realms.push(FakeRealm { host: host.into(), port: LOCAL_REALM_PORT, realm });
        let ri = self.realms.len() - 1;
        let pubk = ecm_auth::public_key(&key);
        let setup = AuthSetup {
            realm: RealmRef { name: name.into(), fp: ecm_auth::fingerprint(&pubk), endpoint: Some(format!("-:{LOCAL_REALM_PORT}")) },
            realm_pub: pubk,
            backend_pub: pubk,
        };
        self.push_backend(host, name, announced, key, ri, setup)
    }

    /// An Auth server (`authd`) called `name` at `host:7600`.
    pub fn add_realm(&mut self, host: &str, name: &str) -> usize {
        let seed = self.seed();
        self.realms.push(FakeRealm { host: host.into(), port: ecm_auth::DEFAULT_PORT, realm: new_realm(&seed, name) });
        self.realms.len() - 1
    }

    /// A backend that uses the accounts of realm `ri` (`--auth HOST:7600 --auth-key FP`).
    pub fn add_shared(&mut self, host: &str, name: &str, announced: bool, ri: usize) -> usize {
        let seed = self.seed();
        let key = signing_key(&seed);
        let (realm_pub, realm_name) = {
            let r = self.realms[ri].realm.borrow();
            (r.public_key(), r.name().to_string())
        };
        let setup = AuthSetup {
            realm: RealmRef {
                name: realm_name,
                fp: ecm_auth::fingerprint(&realm_pub),
                endpoint: Some(format!("{}:{}", self.realms[ri].host, self.realms[ri].port)),
            },
            realm_pub,
            backend_pub: ecm_auth::public_key(&key),
        };
        self.push_backend(host, name, announced, key, ri, setup)
    }

    fn push_backend(&mut self, host: &str, name: &str, announced: bool, key: SigningKey, realm: usize, setup: AuthSetup) -> usize {
        self.backends.push(FakeBackend {
            host: host.into(),
            port: 7777,
            backend: Backend::new(name, 7778, 7, setup),
            key,
            realm,
            pending: BTreeMap::new(),
            announced,
            udp_to_app: VecDeque::new(),
        });
        self.backends.len() - 1
    }

    /// Backend `bi` (with its own accounts) is set up again with a new key, same name and
    /// address: a changed realm key, as a spoof (or a reinstall) would show. Its
    /// connections drop.
    pub fn rekey(&mut self, bi: usize) {
        let (host, name, announced) = (self.backends[bi].host.clone(), self.backends[bi].backend.name.clone(), self.backends[bi].announced);
        let seed = self.seed();
        let key = signing_key(&seed);
        let ri = self.backends[bi].realm;
        self.realms[ri].realm = new_realm(&seed, &name);
        let pubk = ecm_auth::public_key(&key);
        let setup = AuthSetup {
            realm: RealmRef { name: name.clone(), fp: ecm_auth::fingerprint(&pubk), endpoint: Some(format!("-:{LOCAL_REALM_PORT}")) },
            realm_pub: pubk,
            backend_pub: pubk,
        };
        self.backends[bi] = FakeBackend {
            host,
            port: 7777,
            backend: Backend::new(&name, 7778, 7, setup),
            key,
            realm: ri,
            pending: BTreeMap::new(),
            announced,
            udp_to_app: VecDeque::new(),
        };
        self.conns.retain(|_, b| *b != bi);
    }

    /// The fingerprint backend `bi` trusts (its realm's).
    pub fn realm_fp(&self, bi: usize) -> String {
        self.backends[bi].backend.realm().fp.clone()
    }

    fn perform(&mut self, bi: usize, outs: Vec<Out>) {
        for o in outs {
            match o {
                Out::Send(id, line) => {
                    let b = &mut self.backends[bi];
                    b.pending.entry(id as i32).or_default().extend_from_slice(format!("{line}\n").as_bytes());
                }
                Out::Close(id) => {
                    self.conns.remove(&(id as i32));
                }
                Out::Udp(_, _, data) => self.backends[bi].udp_to_app.push_back(data),
                Out::Log(_) | Out::Save => {}
            }
        }
    }

    /// Another user's connection to backend `bi` (a second app, the chat command), opened
    /// if new: its greeting.
    pub fn greeting(&mut self, bi: usize, handle: i32) -> Greeting {
        if !self.conns.contains_key(&handle) {
            self.conns.insert(handle, bi);
            let outs = self.backends[bi].backend.connect(handle as u32, "10.0.0.99:4000");
            self.perform(bi, outs);
            let text = String::from_utf8(self.backends[bi].pending.remove(&handle).unwrap_or_default()).unwrap();
            let mut g = Greeting::default();
            for l in text.lines() {
                g.feed(l);
            }
            self.greetings.insert(handle, g);
        }
        self.greetings[&handle].clone()
    }

    /// A token from backend `bi`'s realm for user `name` (a stable id from the name), for
    /// `audience`, bound to `session`. Made by the realm itself, as if `name` signed in.
    pub fn token_for(&self, bi: usize, name: &str, audience: &str, session: &SigningKey) -> String {
        let uid = 1000 + name.bytes().map(|b| b as u64).sum::<u64>();
        let user = ecm_auth::User {
            id: uid,
            username: name.to_ascii_lowercase(),
            display_name: name.into(),
            key: None,
            disabled: false,
            created_ms: 0,
        };
        let realm = self.realms[self.backends[bi].realm].realm.borrow();
        realm.mint(&user, audience, &ecm_auth::public_key(session), self.now).0
    }

    /// Another user on backend `bi`: their lines in, what they got out. A line `HELLO
    /// <name>` signs them in as `name` (a token from the backend's realm and its proof).
    pub fn user(&mut self, bi: usize, handle: i32, lines: &str) -> String {
        let g = self.greeting(bi, handle);
        let mut out = String::new();
        for line in lines.lines() {
            let line = match line.strip_prefix("HELLO ") {
                Some(name) if !name.contains(' ') => {
                    let session = signing_key(&[handle as u8; 32]);
                    let tok = self.token_for(bi, name, &g.backend_fp, &session);
                    g.sign_in_line(&tok, &session, false)
                }
                _ => line.to_string(),
            };
            out.push_str(&self.raw(bi, handle, &line));
        }
        out
    }

    /// One line as it is, from connection `handle` to backend `bi`: what came back.
    pub fn raw(&mut self, bi: usize, handle: i32, line: &str) -> String {
        self.greeting(bi, handle);
        let now = self.now;
        let outs = self.backends[bi].backend.receive(handle as u32, format!("{line}\n").as_bytes(), now);
        self.perform(bi, outs);
        String::from_utf8(self.backends[bi].pending.remove(&handle).unwrap_or_default()).unwrap()
    }

    /// Another user's voice datagram to backend `bi` (from 10.0.0.99:6000).
    pub fn user_udp(&mut self, bi: usize, data: &[u8]) -> Vec<Vec<u8>> {
        let now = self.now;
        let outs = self.backends[bi].backend.udp([10, 0, 0, 99], 6000, data, now);
        let mut to_user = Vec::new();
        for o in outs {
            match o {
                Out::Udp([10, 0, 0, 99], 6000, d) => to_user.push(d),
                o => self.perform(bi, vec![o]),
            }
        }
        to_user
    }
}

/// A connection to a fake realm: bytes in drive its `ServerConn` at the network's time.
pub struct RealmStream {
    realm: Rc<RefCell<ecm_auth::Realm>>,
    conn: ServerConn,
    inbox: VecDeque<u8>,
    now: u64,
    closed: bool,
}

impl ecm_auth::Stream for RealmStream {
    fn write_all(&mut self, data: &[u8]) -> Result<(), AuthError> {
        if self.closed {
            return Err(AuthError::Closed);
        }
        let out = self.conn.receive(&mut self.realm.borrow_mut(), data, self.now);
        self.inbox.extend(out.send);
        self.closed |= out.close;
        Ok(())
    }

    fn read_exact(&mut self, buf: &mut [u8], _timeout_ms: u32) -> Result<(), AuthError> {
        if self.inbox.len() < buf.len() {
            return Err(if self.closed { AuthError::Closed } else { AuthError::Timeout });
        }
        for b in buf.iter_mut() {
            *b = self.inbox.pop_front().unwrap();
        }
        Ok(())
    }
}

impl Net for FakeNet {
    fn connect(&mut self, host: &str, port: u16) -> Result<i32, String> {
        if self.refuse.iter().any(|h| h == host) {
            return Err(format!("{host}:{port} refused the connection (is discordd running there?)"));
        }
        let Some(bi) = self.backends.iter().position(|b| b.host == host && b.port == port) else {
            return Err(format!("no route to {host}"));
        };
        self.next += 1;
        let h = 100 + self.next;
        self.conns.insert(h, bi);
        let outs = self.backends[bi].backend.connect(h as u32, "10.0.0.50:40000");
        self.perform(bi, outs);
        Ok(h)
    }
    fn send(&mut self, h: i32, line: &str) -> Result<(), String> {
        let Some(&bi) = self.conns.get(&h) else { return Err(String::new()) };
        self.sent.push((h, line.to_string()));
        let now = self.now;
        let outs = self.backends[bi].backend.receive(h as u32, format!("{line}\n").as_bytes(), now);
        self.perform(bi, outs);
        Ok(())
    }
    fn recv(&mut self, h: i32) -> Result<Vec<u8>, String> {
        let Some(&bi) = self.conns.get(&h) else { return Err(String::new()) };
        Ok(self.backends[bi].pending.remove(&h).unwrap_or_default())
    }
    fn close(&mut self, h: i32) {
        if let Some(bi) = self.conns.remove(&h) {
            let now = self.now;
            let outs = self.backends[bi].backend.disconnect(h as u32, now);
            self.perform(bi, outs);
        }
    }
    fn auth_stream(&mut self, host: &str) -> Result<Box<dyn ecm_auth::Stream>, String> {
        let (h, port) = ecm_auth::host_port(host).map_err(|e| e.to_string())?;
        if self.refuse.iter().any(|r| *r == h) {
            return Err(format!("{h}:{port} refused the connection"));
        }
        let Some(r) = self.realms.iter().find(|r| r.host == h && r.port == port) else {
            return Err(format!("no route to {h}:{port}"));
        };
        self.realm_connects += 1;
        Ok(Box::new(RealmStream { realm: r.realm.clone(), conn: ServerConn::new("10.0.0.50:41000"), inbox: VecDeque::new(), now: self.now, closed: false }))
    }
    fn discover(&mut self) -> Vec<ecm_disco::Found> {
        self.searches += 1;
        self.backends
            .iter()
            .filter(|b| b.announced)
            .map(|b| ecm_disco::Found {
                kind: "discord".into(),
                host: ecm_disco::parse_ip(&b.host).unwrap_or([0, 0, 0, 0]),
                port: b.port,
                name: b.backend.name.clone(),
            })
            .collect()
    }
    fn now_ms(&mut self) -> u64 {
        self.now
    }
    fn voice_open(&mut self, host: &str, _port: u16) -> Result<(), String> {
        self.voice_to = self.backends.iter().position(|b| b.host == host);
        self.voice_to.map(|_| ()).ok_or_else(|| "no route".into())
    }
    fn voice_send(&mut self, data: &[u8]) {
        self.voice_sent.push(data.to_vec());
        if let Some(bi) = self.voice_to {
            let now = self.now;
            let outs = self.backends[bi].backend.udp([10, 0, 0, 50], 50000, data, now);
            // Forwarded to the other user (10.0.0.99) is dropped here; tests read voice_sent.
            for o in outs {
                if !matches!(o, Out::Udp(..)) {
                    self.perform(bi, vec![o]);
                }
            }
        }
    }
    fn voice_recv(&mut self) -> Vec<Vec<u8>> {
        match self.voice_to {
            Some(bi) => self.backends[bi].udp_to_app.drain(..).collect(),
            None => Vec::new(),
        }
    }
    fn voice_close(&mut self) {
        self.voice_to = None;
    }
}

/// A sound card in a test: what the microphone will capture, what was played.
#[derive(Default)]
pub struct FakeAudio {
    pub open: bool,
    pub mic: VecDeque<i16>,
    pub played: Vec<i16>,
    pub queued: i64,
    pub headphones: bool,
    pub fail: Option<String>,
    /// Every wake lock request (ms; 0 = release).
    pub wakelocks: Vec<u64>,
}

impl Audio for FakeAudio {
    fn open(&mut self) -> Result<(), String> {
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        self.open = true;
        Ok(())
    }
    fn capture(&mut self) -> Vec<i16> {
        self.mic.drain(..).collect()
    }
    fn play(&mut self, pcm: &[i16]) {
        self.played.extend_from_slice(pcm);
    }
    fn queued_ms(&mut self) -> i64 {
        self.queued
    }
    fn headphones(&mut self) -> bool {
        self.headphones
    }
    fn close(&mut self) {
        self.open = false;
    }
    fn wakelock(&mut self, ms: u64) {
        self.wakelocks.push(ms);
    }
}
