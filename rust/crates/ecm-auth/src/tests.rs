//! Host tests: the KDF, sign-up and sign-in through a real [`ServerConn`] over an
//! in-memory pipe, the attacks the design stops, tokens at backends, and the formats.

use super::*;
use crate::kdf::pbkdf2_hmac_sha256;
use crate::wire::Channel;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

const T0: u64 = 1_800_000_000_000;

type Tamper = Box<dyn FnMut(&mut Vec<u8>)>;

/// A client stream that drives a server connection directly, recording every byte on
/// the wire (both directions) and optionally rewriting them (a relay or a spoof).
struct Pipe {
    realm: Rc<RefCell<Realm>>,
    conn: ServerConn,
    inbox: VecDeque<u8>,
    wire: Rc<RefCell<Vec<u8>>>,
    now: Rc<Cell<u64>>,
    closed: bool,
    c2s: Option<Tamper>,
    s2c: Option<Tamper>,
}

impl Stream for Pipe {
    fn write_all(&mut self, data: &[u8]) -> Result<(), AuthError> {
        if self.closed {
            return Err(AuthError::Closed);
        }
        let mut d = data.to_vec();
        if let Some(t) = &mut self.c2s {
            t(&mut d);
        }
        self.wire.borrow_mut().extend_from_slice(&d);
        let out = self.conn.receive(&mut self.realm.borrow_mut(), &d, self.now.get());
        let mut s = out.send;
        if let Some(t) = &mut self.s2c {
            t(&mut s);
        }
        self.wire.borrow_mut().extend_from_slice(&s);
        self.inbox.extend(s);
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

/// A realm under test, the clock and the wire capture.
struct Net {
    realm: Rc<RefCell<Realm>>,
    now: Rc<Cell<u64>>,
    wire: Rc<RefCell<Vec<u8>>>,
}

impl Net {
    fn new(seed: u8, name: &str) -> Net {
        let realm = Realm::new(&[seed; 32], name).unwrap().with_kdf(MIN_KDF_ITERATIONS);
        Net { realm: Rc::new(RefCell::new(realm)), now: Rc::new(Cell::new(T0)), wire: Rc::default() }
    }

    fn pipe(&self) -> Pipe {
        self.pipe_from("10.0.0.2:40000")
    }

    fn pipe_from(&self, peer: &str) -> Pipe {
        Pipe {
            realm: self.realm.clone(),
            conn: ServerConn::new(peer),
            inbox: VecDeque::new(),
            wire: self.wire.clone(),
            now: self.now.clone(),
            closed: false,
            c2s: None,
            s2c: None,
        }
    }

    fn fp(&self) -> String {
        self.realm.borrow().fingerprint()
    }

    fn pub_key(&self) -> [u8; 32] {
        self.realm.borrow().public_key()
    }

    fn later(&self, ms: u64) {
        self.now.set(self.now.get() + ms);
    }
}

fn store() -> AccountStore<MemFiles> {
    AccountStore::new(MemFiles::default())
}

const HOST: &str = "10.0.0.1:7600";

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

// ------------------------------------------------------------------ KDF

#[test]
fn pbkdf2_matches_the_published_sha256_vectors() {
    let cases: [(u32, &str); 3] = [
        (1, "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"),
        (2, "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"),
        (4096, "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a"),
    ];
    for (c, want) in cases {
        let mut out = [0u8; 32];
        pbkdf2_hmac_sha256(b"password", b"salt", c, &mut out);
        assert_eq!(hex(&out), want, "c = {c}");
    }
    // Longer than one block (RFC 7914 section 11: P = "passwd", S = "salt", c = 1, 64 bytes).
    let mut out = [0u8; 64];
    pbkdf2_hmac_sha256(b"passwd", b"salt", 1, &mut out);
    assert_eq!(
        hex(&out),
        "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc49ca9cccf179b645991664b39d77ef317c71b845b1e30bd509112041d3a19783"
    );
}

#[test]
fn the_realm_key_salts_the_account_key() {
    let a = [1u8; 32];
    let b = [2u8; 32];
    let ka = account_seed("hunter2", &a, "alice", MIN_KDF_ITERATIONS);
    let kb = account_seed("hunter2", &b, "alice", MIN_KDF_ITERATIONS);
    assert_ne!(ka, kb, "same password, two realms: unrelated keys");
    assert_eq!(ka, account_seed("hunter2", &a, "ALICE", MIN_KDF_ITERATIONS), "usernames compare in lower case");
    assert_ne!(ka, account_seed("hunter2", &a, "bob", MIN_KDF_ITERATIONS));
    assert_ne!(ka, account_seed("hunter3", &a, "alice", MIN_KDF_ITERATIONS));
    // The salt is exactly "ecm-auth-v1" || realm_pub || lowercase(username).
    let mut want = [0u8; 32];
    pbkdf2_hmac_sha256(b"hunter2", &[b"ecm-auth-v1".as_slice(), &a, b"alice"].concat(), MIN_KDF_ITERATIONS, &mut want);
    assert_eq!(ka, want);
}

/// `cargo test --release -p ecm-auth kdf_timing -- --nocapture` prints the native cost
/// of one sign-in's key derivation at the default count (see kdf.rs for the choice).
#[test]
fn kdf_timing() {
    let t = std::time::Instant::now();
    let _ = account_seed("correct horse battery staple", &[7u8; 32], "alice", DEFAULT_KDF_ITERATIONS);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("PBKDF2-HMAC-SHA256 x {DEFAULT_KDF_ITERATIONS}: {ms:.1} ms natively ({:.3} us per iteration)", ms * 1000.0 / DEFAULT_KDF_ITERATIONS as f64);
}

// ------------------------------------------------------------------ sign-up and sign-in

#[test]
fn sign_up_then_sign_in_on_another_device() {
    let net = Net::new(9, "Tech Village");
    let mut phone = store();
    let login = phone.sign_up(net.pipe(), HOST, "Alice", "hunter2", Some("Alice A.")).unwrap();
    assert_eq!((login.username.as_str(), login.display_name.as_str(), login.realm_fp.as_str()), ("alice", "Alice A.", net.fp().as_str()));
    assert_eq!(phone.pins().check_pin("Tech Village", HOST, &net.fp()), PinCheck::Pinned, "signing up pinned the realm");
    assert_eq!(phone.last().as_deref(), Some(net.fp().as_str()));

    // The users file has the public key only.
    let users = net.realm.borrow().save_text();
    assert!(users.contains("U 1 alice ok ") && users.contains(" Alice A.\n"), "{users}");
    assert!(!users.contains(&hex(&login.account_seed)));
    assert!(!users.contains(&login.refresh_token), "refresh tokens are stored hashed");

    let mut terminal = store();
    let again = terminal.sign_in(net.pipe(), HOST, "alice", "hunter2").unwrap();
    assert_eq!(again.user_id, login.user_id);
    assert_eq!(again.account_seed, login.account_seed, "the same password gives the same key");
    assert_ne!(again.session_seed, login.session_seed, "every sign-in has its own session key");
    assert_ne!(again.refresh_token, login.refresh_token);
    assert_eq!(net.realm.borrow().sessions("alice", net.now.get()), 2);

    // The taken name can't be signed up again.
    assert_eq!(store().sign_up(net.pipe(), HOST, "alice", "other", None).unwrap_err(), AuthError::UsernameTaken("alice".into()));

    // WHOAMI on a resumed session.
    let mut l = terminal.login(&net.fp()).unwrap();
    let mut s = terminal.resume(net.pipe(), &mut l).unwrap();
    assert_eq!(s.whoami().unwrap(), (1, "alice".into(), "Alice A.".into()));

    // A realm saved and loaded keeps everything.
    let text = net.realm.borrow().save_text();
    let loaded = Realm::load(&[9; 32], "Tech Village", &text).unwrap();
    assert_eq!(loaded.save_text(), text);
    assert_eq!(loaded.kdf_iterations(), MIN_KDF_ITERATIONS);
}

#[test]
fn wrong_passwords_are_refused_then_rate_limited() {
    let net = Net::new(9, "Tech Village");
    store().sign_up(net.pipe(), HOST, "alice", "hunter2", None).unwrap();
    let mut c = store();
    assert_eq!(c.sign_in(net.pipe(), HOST, "alice", "hunter3").unwrap_err(), AuthError::WrongPassword);
    assert_eq!(c.sign_in(net.pipe(), HOST, "nobody", "x").unwrap_err(), AuthError::WrongPassword, "an unknown user looks the same");
    assert_eq!(AuthError::WrongPassword.to_string(), "wrong username or password");
    for _ in 0..4 {
        assert_eq!(c.sign_in(net.pipe(), HOST, "alice", "guess").unwrap_err(), AuthError::WrongPassword);
    }
    // Five failures for alice: locked, even with the right password.
    let e = c.sign_in(net.pipe(), HOST, "alice", "hunter2").unwrap_err();
    assert_eq!(e, AuthError::Locked { wait_s: 30 });
    assert_eq!(e.to_string(), "too many tries: wait 30 s");
    assert!(c.login(&net.fp()).is_none(), "nothing saved after a refusal");
    net.later(31_000);
    c.sign_in(net.pipe(), HOST, "alice", "hunter2").unwrap();

    // Per address: ten failures from one address lock it for every username.
    let net = Net::new(9, "Tech Village");
    store().sign_up(net.pipe(), HOST, "bob", "pw", None).unwrap();
    for i in 0..10 {
        let _ = c.sign_in(net.pipe_from("10.0.0.66:1000"), HOST, &format!("u{i}"), "x");
    }
    assert!(matches!(c.sign_in(net.pipe_from("10.0.0.66:1001"), HOST, "bob", "pw").unwrap_err(), AuthError::Locked { .. }));
    c.sign_in(net.pipe_from("10.0.0.7:1000"), HOST, "bob", "pw").unwrap();
}

#[test]
fn a_connection_hangs_up_after_three_failed_tries() {
    let net = Net::new(9, "Tech Village");
    let (mut s, info) = handshake(net.pipe()).unwrap();
    let k = account_key("x", &info.realm_pub, "a", info.kdf_iterations);
    for _ in 0..3 {
        assert_eq!(s.login("a", &k, &[1; 32]).unwrap_err(), AuthError::WrongPassword);
    }
    assert_eq!(s.login("a", &k, &[1; 32]).unwrap_err(), AuthError::Closed);
}

// ------------------------------------------------------------------ attacks

#[test]
fn a_spoofed_server_without_the_realm_key_fails_at_the_hello() {
    let real = Net::new(9, "Tech Village");
    let spoof = Net::new(66, "Tech Village");
    let real_pub = real.pub_key();
    let mut p = spoof.pipe();
    // The spoof claims the real realm's public key but can only sign with its own.
    p.s2c = Some(Box::new(move |d: &mut Vec<u8>| {
        if d.len() > 4 + 66 && d[4] == wire::SERVER_HELLO {
            d[4 + 34..4 + 66].copy_from_slice(&real_pub);
        }
    }));
    assert_eq!(handshake(p).err().unwrap(), AuthError::NotTheRealm);
    // Through the store too: nothing is pinned or sent.
    let mut c = store();
    let mut p = spoof.pipe();
    p.s2c = Some(Box::new(move |d: &mut Vec<u8>| {
        if d.len() > 4 + 66 && d[4] == wire::SERVER_HELLO {
            d[4 + 34..4 + 66].copy_from_slice(&real_pub);
        }
    }));
    assert_eq!(c.sign_in(p, HOST, "alice", "hunter2").unwrap_err(), AuthError::NotTheRealm);
    assert!(c.pins().pins.is_empty());
    assert!(spoof.realm.borrow().users().is_empty());
}

#[test]
fn a_relay_that_swaps_in_its_own_key_exchange_fails() {
    let net = Net::new(9, "Tech Village");
    store().sign_up(net.pipe(), HOST, "alice", "hunter2", None).unwrap();
    let mut p = net.pipe();
    let (relay_ce, _) = ecm_ssh_crypto::x25519_generate_keypair();
    let relay_ce = relay_ce.to_bytes();
    p.c2s = Some(Box::new(move |d: &mut Vec<u8>| {
        if d.len() == 4 + 34 && d[4] == wire::CLIENT_HELLO {
            d[6..38].copy_from_slice(&relay_ce);
        }
    }));
    // The realm signed a transcript with the relay's `ce`: the client's check fails.
    assert_eq!(store().sign_in(p, HOST, "alice", "hunter2").unwrap_err(), AuthError::NotTheRealm);
}

#[test]
fn replayed_or_reordered_frames_end_the_connection() {
    let th = [3u8; 32];
    let dh = [4u8; 32];
    let mut c = Channel::new(&dh, &th, true).unwrap();
    let mut s = Channel::new(&dh, &th, false).unwrap();
    let f1 = c.seal(b"WHOAMI");
    let f2 = c.seal(b"LOGOUT");
    assert!(s.open(&f2[4..]).is_err(), "out of order");
    let mut s = Channel::new(&dh, &th, false).unwrap();
    assert_eq!(s.open(&f1[4..]).unwrap(), b"WHOAMI");
    assert!(s.open(&f1[4..]).is_err(), "replayed");
    // The directions have different keys: a reflected frame doesn't open either.
    let mut c2 = Channel::new(&dh, &th, true).unwrap();
    assert!(c2.open(&f1[4..]).is_err());
}

#[test]
fn a_pinned_realm_with_a_changed_key_is_refused() {
    let real = Net::new(9, "Tech Village");
    let mut c = store();
    c.sign_up(real.pipe(), HOST, "alice", "hunter2", None).unwrap();

    // Same name, another key (at another address).
    let spoof = Net::new(66, "Tech Village");
    let e = c.sign_in(spoof.pipe(), "10.0.0.9", "alice", "hunter2").unwrap_err();
    assert_eq!(e, AuthError::RealmChanged { name: "Tech Village".into(), host: HOST.into(), old_fp: real.fp(), new_fp: spoof.fp() });
    assert!(e.to_string().starts_with("the identity of Tech Village has changed. This could be someone pretending to be it."), "{e}");
    // Same address, another name and key.
    let other = Net::new(67, "Free Accounts");
    assert!(matches!(c.sign_in(other.pipe(), HOST, "alice", "hunter2").unwrap_err(), AuthError::RealmChanged { .. }));
    assert!(matches!(c.fingerprint(other.pipe(), HOST).unwrap_err(), AuthError::RealmChanged { .. }));
    // The saved login is never offered to the new key: a token request at the old
    // address, now answered by the other realm, is refused before any request is sent.
    assert!(matches!(c.token(other.pipe(), &real.fp(), &"ab".repeat(16)).unwrap_err(), AuthError::RealmChanged { .. }));
    assert_eq!(other.realm.borrow().sessions("alice", other.now.get()), 0);
    assert!(other.realm.borrow().users().is_empty());

    // Only removing the pin by hand lets the new key in (as a first contact).
    assert!(c.remove_pin(&real.fp()).unwrap());
    assert!(c.login(&real.fp()).is_none(), "the old key's login went with its pin");
    let (info, check) = c.fingerprint(other.pipe(), HOST).unwrap();
    assert_eq!((info.realm_fp, check), (other.fp(), PinCheck::FirstContact));
}

#[test]
fn pin_rules() {
    let mut pins = Pins::default();
    let (a, b) = ("a".repeat(32), "b".repeat(32));
    assert_eq!(pins.check_pin("Village", "10.0.0.1", &a), PinCheck::FirstContact);
    pins.add(&a, "10.0.0.1", "Village");
    assert_eq!(pins.check_pin("Village", "10.0.0.1:7600", &a), PinCheck::Pinned, "the default port is the same host");
    assert_eq!(pins.check_pin("Village", "10.0.0.5", &a), PinCheck::Pinned, "a known key at a new address");
    assert_eq!(
        pins.check_pin("village", "10.0.0.5", &b),
        PinCheck::Changed { old_fp: a.clone(), name: "Village".into(), host: "10.0.0.1:7600".into() },
        "same name (any case), other key"
    );
    assert!(matches!(pins.check_pin("Other", "10.0.0.1", &b), PinCheck::Changed { .. }), "same host, other key");
    assert_eq!(pins.check_pin("Other", "10.0.0.2", &b), PinCheck::FirstContact);
    let text = pins.to_text();
    assert_eq!(text, format!("{a} 10.0.0.1:7600 Village\n"));
    assert_eq!(Pins::parse(&format!("# comment\n{text}junk line\n")), pins);
    assert!(pins.remove_pin(&a) && !pins.remove_pin(&a));
}

// ------------------------------------------------------------------ tokens

/// A signed-in client store, its realm fingerprint and two backends' keys.
fn signed_in() -> (Net, AccountStore<MemFiles>, String, SigningKey, SigningKey) {
    let net = Net::new(9, "Tech Village");
    let mut c = store();
    c.sign_up(net.pipe(), HOST, "alice", "hunter2", Some("Alice")).unwrap();
    let fp = net.fp();
    (net, c, fp, signing_key(&[21; 32]), signing_key(&[22; 32]))
}

#[test]
fn tokens_work_only_at_their_backend_with_the_session_key() {
    let (net, mut c, fp, music, discord) = signed_in();
    let gate_a = BackendGate::new(net.pub_key(), public_key(&music));
    let gate_b = BackendGate::new(net.pub_key(), public_key(&discord));
    let (tok, exp) = c.token(net.pipe(), &fp, &gate_a.audience()).unwrap();
    assert!(tok.starts_with("ecmtok1."));
    assert_eq!(exp, net.now.get() + TOKEN_LIFETIME_MS);

    let nonce = gate_a.challenge();
    let proof = c.prove(&fp, &nonce, &gate_a.audience()).unwrap();
    let claims = gate_a.check(&tok, &proof, &nonce, net.now.get()).unwrap();
    assert_eq!((claims.username.as_str(), claims.display_name.as_str(), claims.user_id), ("alice", "Alice", 1));
    assert_eq!(claims.realm_fp, fp);
    assert_eq!(Claims::peek(&tok).unwrap(), claims);

    // Taken from backend A to backend B: refused (even with a proof for B's nonce).
    let nonce_b = gate_b.challenge();
    let proof_b = c.prove(&fp, &nonce_b, &gate_b.audience()).unwrap();
    assert_eq!(gate_b.check(&tok, &proof_b, &nonce_b, net.now.get()).unwrap_err(), AuthError::TokenAudience);
    // Proof for another nonce (a replayed connection), or by another key.
    assert_eq!(gate_a.check(&tok, &proof, &gate_a.challenge(), net.now.get()).unwrap_err(), AuthError::TokenProof);
    let thief = prove(&signing_key(&[5; 32]), &nonce, &gate_a.audience());
    assert_eq!(gate_a.check(&tok, &thief, &nonce, net.now.get()).unwrap_err(), AuthError::TokenProof);
    assert_eq!(gate_a.check(&tok, &[0u8; 10], &nonce, net.now.get()).unwrap_err(), AuthError::TokenProof);
    // Expired (after the hour and the skew).
    let late = net.now.get() + TOKEN_LIFETIME_MS + CLOCK_SKEW_MS + 1;
    assert_eq!(gate_a.check(&tok, &proof, &nonce, late).unwrap_err(), AuthError::TokenExpired);
    assert!(gate_a.check(&tok, &proof, &nonce, net.now.get() + TOKEN_LIFETIME_MS).is_ok());
    // A backend that trusts another realm.
    let other_realm = BackendGate::new(public_key(&signing_key(&[66; 32])), public_key(&music));
    assert_eq!(other_realm.check(&tok, &proof, &nonce, net.now.get()).unwrap_err(), AuthError::TokenSignature);
}

#[test]
fn tampered_tokens_are_refused() {
    let (net, mut c, fp, music, _) = signed_in();
    let gate = BackendGate::new(net.pub_key(), public_key(&music));
    let (tok, _) = c.token(net.pipe(), &fp, &gate.audience()).unwrap();
    let nonce = gate.challenge();
    let proof = c.prove(&fp, &nonce, &gate.audience()).unwrap();
    // Rewrite the username in the payload, keep the signature.
    let (head, sig) = tok.rsplit_once('.').unwrap();
    let payload = String::from_utf8(unb64url(head.strip_prefix("ecmtok1.").unwrap()).unwrap()).unwrap();
    let forged = format!("ecmtok1.{}.{sig}", b64url(payload.replace("user=alice", "user=admin").as_bytes()));
    assert_eq!(gate.check(&forged, &proof, &nonce, net.now.get()).unwrap_err(), AuthError::TokenSignature);
    // A flipped signature byte, and junk.
    let mut bad_sig = unb64url(sig).unwrap();
    bad_sig[0] ^= 1;
    let flipped = format!("{head}.{}", b64url(&bad_sig));
    assert_eq!(gate.check(&flipped, &proof, &nonce, net.now.get()).unwrap_err(), AuthError::TokenSignature);
    for junk in ["", "ecmtok1.", "ecmtok1.abc", "ecmtok2.a.b", &tok[..tok.len() - 3]] {
        assert!(gate.check(junk, &proof, &nonce, net.now.get()).is_err(), "{junk}");
    }
}

// ------------------------------------------------------------------ what's on the wire

#[test]
fn the_wire_never_carries_the_password_or_the_account_key() {
    let net = Net::new(9, "Tech Village");
    let password = "correct-horse-battery";
    let mut c = store();
    let login = c.sign_up(net.pipe(), HOST, "alice", password, None).unwrap();
    store().sign_in(net.pipe(), HOST, "alice", password).unwrap();
    c.token(net.pipe(), &login.realm_fp, &"cd".repeat(16)).unwrap();
    c.passwd(net.pipe(), &login.realm_fp, "new-password-here").unwrap();
    let wire = net.wire.borrow();
    assert!(wire.len() > 1000, "the capture has the traffic: {} bytes", wire.len());
    for secret in [password.as_bytes(), b"new-password-here".as_slice(), &login.account_seed, hex(&login.account_seed).as_bytes(), &login.session_seed] {
        assert!(!contains(&wire, secret), "a secret is on the wire");
    }
    // Everything after the hellos is encrypted: not even the username or the refresh token shows.
    assert!(!contains(&wire, b"alice"));
    assert!(!contains(&wire, login.refresh_token.as_bytes()));
    assert!(!contains(&wire, b"LOGIN") && !contains(&wire, b"WELCOME"));
}

// ------------------------------------------------------------------ refresh, admin, passwd

#[test]
fn refresh_works_until_revoked() {
    let (net, mut c, fp, music, _) = signed_in();
    let aud = fingerprint(&public_key(&music));
    let first = c.login(&fp).unwrap().refresh_expires_ms;
    net.later(24 * 3_600_000);
    c.token(net.pipe(), &fp, &aud).unwrap();
    assert_eq!(c.login(&fp).unwrap().refresh_expires_ms, first + 24 * 3_600_000, "using the refresh token renews it");
    assert_eq!(net.realm.borrow_mut().revoke("alice").unwrap(), 1);
    let e = c.token(net.pipe(), &fp, &aud).unwrap_err();
    assert_eq!(e, AuthError::SignInExpired);
    assert_eq!(e.to_string(), "your sign-in has expired: sign in again");
    // A password sign-in works again.
    c.sign_in(net.pipe(), HOST, "alice", "hunter2").unwrap();
    c.token(net.pipe(), &fp, &aud).unwrap();
    // A refresh token expires after 30 days without use.
    net.later(REFRESH_LIFETIME_MS + 1);
    assert_eq!(c.token(net.pipe(), &fp, &aud).unwrap_err(), AuthError::SignInExpired);
}

#[test]
fn logout_revokes_the_refresh_token_and_forgets_the_login() {
    let (net, mut c, fp, _, _) = signed_in();
    let saved = c.login(&fp).unwrap();
    c.logout(Some(net.pipe()), &fp).unwrap();
    assert!(c.login(&fp).is_none());
    assert!(c.pins().is_pinned(&fp), "the pin stays");
    assert_eq!(net.realm.borrow().sessions("alice", net.now.get()), 0);
    // The old refresh token is dead at the realm.
    let (mut s, _) = handshake(net.pipe()).unwrap();
    assert_eq!(s.refresh("alice", &saved.refresh_token, &[1; 32]).unwrap_err(), AuthError::SignInExpired);
}

#[test]
fn reset_lets_the_user_set_a_new_password() {
    let (net, mut c, fp, _, _) = signed_in();
    net.realm.borrow_mut().reset("alice").unwrap();
    assert_eq!(net.realm.borrow().sessions("alice", net.now.get()), 0, "reset signs out everywhere");
    assert_eq!(c.token(net.pipe(), &fp, &"ab".repeat(16)).unwrap_err(), AuthError::SignInExpired);
    // The next sign-in sets the password: the same account (id, display name).
    let l = store().sign_in(net.pipe(), HOST, "alice", "brand-new").unwrap();
    assert_eq!((l.user_id, l.display_name.as_str()), (1, "Alice"));
    assert_eq!(store().sign_in(net.pipe(), HOST, "alice", "hunter2").unwrap_err(), AuthError::WrongPassword);
    store().sign_in(net.pipe(), HOST, "alice", "brand-new").unwrap();
    assert!(Realm::load(&[9; 32], "Tech Village", &net.realm.borrow().save_text()).unwrap().user("alice").unwrap().key.is_some());
}

#[test]
fn passwd_changes_the_key() {
    let (net, mut c, fp, _, _) = signed_in();
    let other_device = store();
    let mut other_device = other_device;
    other_device.sign_in(net.pipe(), HOST, "alice", "hunter2").unwrap();
    c.passwd(net.pipe(), &fp, "s3cond").unwrap();
    assert_eq!(store().sign_in(net.pipe(), HOST, "alice", "hunter2").unwrap_err(), AuthError::WrongPassword);
    store().sign_in(net.pipe(), HOST, "alice", "s3cond").unwrap();
    // This device stays signed in; the other one has to sign in again.
    c.token(net.pipe(), &fp, &"ab".repeat(16)).unwrap();
    assert_eq!(other_device.token(net.pipe(), &fp, &"ab".repeat(16)).unwrap_err(), AuthError::SignInExpired);
}

#[test]
fn disabled_accounts_are_refused() {
    let (net, mut c, fp, _, _) = signed_in();
    net.realm.borrow_mut().disable("alice").unwrap();
    assert_eq!(c.token(net.pipe(), &fp, &"ab".repeat(16)).unwrap_err(), AuthError::SignInExpired);
    assert_eq!(store().sign_in(net.pipe(), HOST, "alice", "hunter2").unwrap_err(), AuthError::Disabled);
    assert_eq!(store().sign_in(net.pipe(), HOST, "alice", "nope").unwrap_err(), AuthError::WrongPassword, "no hint without the password");
    net.realm.borrow_mut().enable("alice").unwrap();
    store().sign_in(net.pipe(), HOST, "alice", "hunter2").unwrap();
    assert_eq!(net.realm.borrow_mut().disable("zed").unwrap_err(), AuthError::NoSuchUser("zed".into()));
}

#[test]
fn clients_refuse_weak_or_huge_key_derivation() {
    for n in [MIN_KDF_ITERATIONS - 1, MAX_KDF_ITERATIONS + 1] {
        let net = Net::new(9, "Tech Village");
        net.realm.replace(Realm::new(&[9; 32], "Tech Village").unwrap().with_kdf(n));
        assert_eq!(handshake(net.pipe()).err().unwrap(), AuthError::BadKdf(n));
    }
}

// ------------------------------------------------------------------ formats

#[test]
fn fingerprints_names_and_realm_lines() {
    let fp = fingerprint(&[0u8; 32]);
    assert_eq!(fp, hex(&ecm_ssh_crypto::sha256(&[0u8; 32])[..16]));
    assert_eq!(fp.len(), 32);
    assert_eq!(display_fingerprint("3fa19c0d00000000000000000000beef"), "3F:A1:9C:0D:00:00:00:00:00:00:00:00:00:00:BE:EF");
    assert_eq!(short_fingerprint("3fa19c0d00000000000000000000beef"), "3F:A1:9C:0D");
    assert_eq!(parse_fingerprint("3F:A1:9C:0D:00:00:00:00:00:00:00:00:00:00:BE:EF").as_deref(), Some("3fa19c0d00000000000000000000beef"));

    assert_eq!(username(" Alice.B_2-x ").unwrap(), "alice.b_2-x");
    assert!(username("").is_err() && username("a b").is_err() && username(&"a".repeat(17)).is_err() && username("é").is_err());
    assert_eq!(display_name("  Alice A. ").unwrap(), "Alice A.");
    assert!(display_name("").is_err() && display_name(&"x".repeat(33)).is_err() && display_name("a\nb").is_err());

    let r = RealmRef { name: "Tech Village".into(), fp: fp.clone(), endpoint: Some("10.0.8.1:7600".into()) };
    assert_eq!(r.to_line(), format!("AUTH {fp} 10.0.8.1:7600 Tech Village"));
    assert_eq!(RealmRef::parse(&r.to_line()).unwrap(), r);
    let local = RealmRef { name: "Music".into(), fp: fp.clone(), endpoint: None };
    assert_eq!(local.to_line(), format!("AUTH {fp} - Music"));
    assert_eq!(RealmRef::parse(&local.to_line()).unwrap(), local);
    assert_eq!(RealmRef::parse(&format!("AUTH {fp} 10.0.8.1 X")).unwrap().endpoint.as_deref(), Some("10.0.8.1:7600"));
    assert!(RealmRef::parse("AUTH nothex - X").is_err() && RealmRef::parse(&format!("AUTH {fp} -")).is_err());

    for data in [&b""[..], b"f", b"fo", b"foo", b"foob", b"\xff\xfe\x00"] {
        assert_eq!(unb64url(&b64url(data)).unwrap(), data);
    }
    assert_eq!(b64url(b"\xfb\xff"), "-_8");
}

#[test]
fn saved_logins_round_trip() {
    let (_net, c, fp, _, _) = signed_in();
    let l = c.login(&fp).unwrap();
    let text = l.to_text();
    assert!(text.starts_with("ecm-auth-login 1\nrealm "), "{text}");
    assert_eq!(Login::parse(&text).unwrap(), l);
    assert!(Login::parse("ecm-auth-login 1\nrealm x\n").is_err());
    assert_eq!(c.login_for(Some("10.0.0.1")).unwrap(), l);
    assert_eq!(c.login_for(None).unwrap(), l);
    assert!(c.login_for(Some("10.0.0.99")).is_err());
}
