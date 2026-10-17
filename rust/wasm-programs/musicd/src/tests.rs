//! Host tests: the whole API against an in-memory library made of the
//! ecm-music fixture MP3s, with tokens minted by a test realm.

use crate::api::{App, Body, Config, ConnCtx, Reply};
use crate::auth::Gatekeeper;
use crate::conn::Conn;
use crate::disk::{Disk, MemDisk};
use ecm_auth::token::Claims;
use ecm_auth::{fingerprint, hex, prove, public_key, signing_key, SigningKey};
use ecm_music::http::{parse_request, Json, Request};
use ecm_music::transcode::{SaverHeader, SAVER_MIME};

const CBR: &[u8] = include_bytes!("../../../crates/ecm-music/tests/data/tone_cbr.mp3");
const VBR: &[u8] = include_bytes!("../../../crates/ecm-music/tests/data/tone_vbr.mp3");
const MPEG2: &[u8] = include_bytes!("../../../crates/ecm-music/tests/data/tone_mpeg2.mp3");

/// The backend's own key (and, with `--auth local`, the realm key).
const BACKEND: [u8; 32] = [7; 32];
const NOW: u64 = 1_800_000_000_000;

fn backend_fp() -> String {
    fingerprint(&public_key(&signing_key(&BACKEND)))
}

fn cfg(public_browse: bool, transcode: bool) -> Config {
    Config {
        name: "Test Radio".into(),
        port: 7800,
        library_dir: "/srv/music".into(),
        data_dir: "/var/music".into(),
        transcode,
        public_browse,
        max_kbps: 192,
        realm_name: "Test Radio".into(),
        realm_fp: backend_fp(),
        auth_endpoint: "-:7801".into(),
    }
}

fn library_disk() -> MemDisk {
    let mut d = MemDisk::default();
    d.put("/srv/music/Test Artist/tone_cbr.mp3", CBR, 100);
    d.put("/srv/music/bjork/tone_vbr.mp3", VBR, 200);
    d.put("/srv/music/old/tone_mpeg2.mp3", MPEG2, 300);
    d.put("/srv/music/readme.txt", b"not music", 1);
    d
}

fn app_with(public_browse: bool, transcode: bool, disk: MemDisk) -> App<MemDisk> {
    let own = public_key(&signing_key(&BACKEND));
    let mut app = App::new(cfg(public_browse, transcode), disk, Gatekeeper::new(own, &fingerprint(&own), Some(own)));
    app.scan_now();
    app
}

fn app() -> App<MemDisk> {
    app_with(true, true, library_disk())
}

fn req(raw: &str) -> Request {
    parse_request(raw.as_bytes()).unwrap().unwrap().0
}

/// A request with optional headers and JSON body.
fn call(app: &mut App<MemDisk>, ctx: &mut ConnCtx, method: &str, path: &str, headers: &str, body: &str) -> Reply {
    let raw = format!("{method} {path} HTTP/1.1\r\nHost: x\r\n{headers}Content-Length: {}\r\n\r\n", body.len());
    app.handle(&req(&raw), body.as_bytes(), ctx, NOW)
}

fn json(r: &Reply) -> Json {
    Json::parse(&r.text()).unwrap_or_else(|e| panic!("{e}: {}", r.text()))
}

fn head(r: &Reply) -> String {
    String::from_utf8(r.head.clone()).unwrap()
}

fn s<'a>(j: &'a Json, k: &str) -> &'a str {
    j.get(k).and_then(Json::as_str).unwrap_or_else(|| panic!("no {k} in {j}"))
}

fn n(j: &Json, k: &str) -> u64 {
    j.get(k).and_then(Json::as_u64).unwrap_or_else(|| panic!("no {k} in {j}"))
}

/// A signed-in user: the session key and a token from the realm `realm_seed`
/// made for `audience`.
struct User {
    session: SigningKey,
    token: String,
}

fn user_for(name: &str, realm_seed: &[u8; 32], audience: &str, expires: u64) -> User {
    let session = signing_key(&[name.len() as u8 + 40; 32]);
    let realm = signing_key(realm_seed);
    let token = Claims {
        realm_fp: fingerprint(&public_key(&realm)),
        user_id: name.len() as u64,
        username: name.into(),
        display_name: name.into(),
        audience: audience.into(),
        cnf: public_key(&session),
        issued_ms: NOW - 1000,
        expires_ms: expires,
    }
    .mint(&realm);
    User { session, token }
}

fn user(name: &str) -> User {
    user_for(name, &BACKEND, &backend_fp(), NOW + 3_600_000)
}

/// Headers proving `u`'s token with a fresh nonce from the server.
fn signed(app: &mut App<MemDisk>, u: &User) -> String {
    let nonce = json(&call(app, &mut ConnCtx::new("9.9.9.9"), "GET", "/nonce", "", ""));
    let nonce = s(&nonce, "nonce").to_string();
    let proof = prove(&u.session, &ecm_auth::unhex_n(&nonce).unwrap(), &backend_fp());
    format!("Authorization: Bearer {}\r\nX-ECM-Nonce: {nonce}\r\nX-ECM-Proof: {}\r\n", u.token, hex(&proof))
}

fn song_id(app: &App<MemDisk>, title: &str) -> String {
    app.lib.songs().iter().find(|s| s.title == title).unwrap_or_else(|| panic!("no song {title}")).id.clone()
}

#[test]
fn scan_info_and_public_browsing() {
    let mut a = app();
    let mut ctx = ConnCtx::new("10.0.0.5");
    assert_eq!(a.lib.len(), 3, "readme.txt is not a song");
    let info = json(&call(&mut a, &mut ctx, "GET", "/info", "", ""));
    assert_eq!((s(&info, "name"), n(&info, "songs"), s(&info, "backend_fp")), ("Test Radio", 3, backend_fp().as_str()));
    assert_eq!(info.get("transcode").unwrap().as_bool(), Some(true));
    assert_eq!(info.get("public_browse").unwrap().as_bool(), Some(true));
    let auth = info.get("auth").unwrap();
    assert_eq!((s(auth, "fp"), s(auth, "endpoint"), s(auth, "name")), (backend_fp().as_str(), "-:7801", "Test Radio"));
    assert!(s(&info, "realm").starts_with(&format!("AUTH {} - ", backend_fp())));

    let r = json(&call(&mut a, &mut ctx, "GET", "/search?q=sine+to", "", ""));
    let top = r.get("top").unwrap();
    assert_eq!(s(top, "kind"), "song");
    assert_eq!(s(top.get("item").unwrap(), "title"), "Sine Tone");
    assert_eq!(n(r.get("songs").unwrap(), "total"), 1);
    let r = json(&call(&mut a, &mut ctx, "GET", "/search?q=bjork", "", ""));
    assert_eq!(s(r.get("top").unwrap(), "kind"), "artist");
    let artists = r.get("artists").unwrap().get("items").unwrap().as_array().unwrap();
    let artist_id = s(&artists[0], "id").to_string();

    let cbr = song_id(&a, "Sine Tone");
    let song = json(&call(&mut a, &mut ctx, "GET", &format!("/songs/{cbr}"), "", ""));
    assert_eq!((s(&song, "artist"), s(&song, "album"), n(&song, "track"), n(&song, "year")), ("Test Artist", "Test Album", 3, 2001));
    assert_eq!(song.get("art").unwrap().as_bool(), Some(true));
    assert_eq!(song.get("liked").unwrap().as_bool(), Some(false));
    let album = json(&call(&mut a, &mut ctx, "GET", &format!("/albums/{}", s(&song, "album_id")), "", ""));
    assert_eq!((s(&album, "name"), s(&album, "artist"), n(&album, "song_count")), ("Test Album", "Various", 1));
    assert_eq!(album.get("songs").unwrap().as_array().unwrap().len(), 1);
    let artist = json(&call(&mut a, &mut ctx, "GET", &format!("/artists/{artist_id}"), "", ""));
    assert_eq!(s(&artist, "name"), "Björk");
    assert_eq!(artist.get("songs").unwrap().as_array().unwrap().len(), 1);
    let genres = json(&call(&mut a, &mut ctx, "GET", "/genres", "", ""));
    assert_eq!(genres.get("genres").unwrap().as_array().unwrap().len(), 3);
    let rock = json(&call(&mut a, &mut ctx, "GET", "/genres/rock", "", ""));
    assert_eq!(n(&rock, "total"), 1);
    let new = json(&call(&mut a, &mut ctx, "GET", "/browse?sort=new&limit=2", "", ""));
    assert_eq!((n(&new, "total"), new.get("items").unwrap().as_array().unwrap().len()), (3, 2));
    assert_eq!(s(&new.get("items").unwrap().as_array().unwrap()[0], "title"), "Old Tag", "newest file first");
    let az = json(&call(&mut a, &mut ctx, "GET", "/browse?sort=az&kind=albums", "", ""));
    assert_eq!(n(&az, "total"), 2, "the VBR file has no album");
    assert_eq!(call(&mut a, &mut ctx, "GET", "/browse?sort=pop", "", "").status, 400);
    let albums = json(&call(&mut a, &mut ctx, "GET", "/albums", "", ""));
    assert_eq!(n(&albums, "total"), 2);

    // Cover art, by song, album or artist.
    let png = call(&mut a, &mut ctx, "GET", &format!("/art/{cbr}/64.png"), "", "");
    assert_eq!(png.status, 200);
    assert!(head(&png).contains("Content-Type: image/png"));
    let Body::Bytes(p) = &png.body else { panic!() };
    assert_eq!(&p[1..4], b"PNG");
    assert_eq!(u32::from_be_bytes([p[16], p[17], p[18], p[19]]), 64);
    assert_eq!(call(&mut a, &mut ctx, "GET", &format!("/art/{}/160.png", s(&song, "album_id")), "", "").status, 200);
    assert_eq!(call(&mut a, &mut ctx, "GET", &format!("/art/{cbr}/99.png"), "", "").status, 404);
    let vbr = song_id(&a, "Café Ünïcode ß");
    assert_eq!(call(&mut a, &mut ctx, "GET", &format!("/art/{vbr}/64.png"), "", "").status, 404, "no picture in that file");

    assert_eq!(call(&mut a, &mut ctx, "GET", "/songs/nope", "", "").status, 404);
    assert_eq!(call(&mut a, &mut ctx, "GET", "/nothing", "", "").status, 404);
    assert_eq!(call(&mut a, &mut ctx, "DELETE", "/info", "", "").status, 405);
    // Signed-in only, even with public browsing.
    for p in [format!("/stream/{cbr}"), "/likes".into(), "/recent".into()] {
        let r = call(&mut a, &mut ctx, "GET", &p, "", "");
        assert_eq!((r.status, s(&json(&r), "code")), (401, "signin_required"), "{p}");
    }
}

#[test]
fn public_browse_off_needs_a_sign_in_but_info_does_not() {
    let mut a = app_with(false, true, library_disk());
    let mut ctx = ConnCtx::new("10.0.0.5");
    assert_eq!(call(&mut a, &mut ctx, "GET", "/info", "", "").status, 200);
    assert_eq!(call(&mut a, &mut ctx, "GET", "/nonce", "", "").status, 200);
    for p in ["/search?q=x", "/browse", "/genres", "/albums", "/playlists"] {
        assert_eq!(call(&mut a, &mut ctx, "GET", p, "", "").status, 401, "{p}");
    }
    let alice = user("alice");
    let h = signed(&mut a, &alice);
    assert_eq!(call(&mut a, &mut ctx, "GET", "/search?q=sine", &h, "").status, 200);
}

#[test]
fn tokens_are_refused_for_another_backend_another_realm_without_proof_or_expired() {
    let mut a = app();
    let id = song_id(&a, "Sine Tone");
    let path = format!("/stream/{id}");
    let code = |r: &Reply| (r.status, s(&json(r), "code").to_string());
    let alice = user("alice");
    let mut ctx = ConnCtx::new("10.0.0.5");
    // A token but no proof.
    let r = call(&mut a, &mut ctx, "GET", &path, &format!("Authorization: Bearer {}\r\n", alice.token), "");
    assert_eq!(code(&r), (401, "proof_required".into()));
    // A token made for another backend.
    let other = user_for("alice", &BACKEND, &fingerprint(&public_key(&signing_key(&[8; 32]))), NOW + 3_600_000);
    let h = signed(&mut a, &other);
    assert_eq!(code(&call(&mut a, &mut ctx, "GET", &path, &h, "")), (401, "token_audience".into()));
    // Signed by a realm this backend doesn't trust.
    let spoof = user_for("alice", &[9; 32], &backend_fp(), NOW + 3_600_000);
    let h = signed(&mut a, &spoof);
    assert_eq!(code(&call(&mut a, &mut ctx, "GET", &path, &h, "")), (401, "token_signature".into()));
    // Expired (beyond the clock skew allowed).
    let old = user_for("alice", &BACKEND, &backend_fp(), NOW - 600_000);
    let h = signed(&mut a, &old);
    assert_eq!(code(&call(&mut a, &mut ctx, "GET", &path, &h, "")), (401, "token_expired".into()));
    // Someone who sniffed the token but has no session key: a proof by another key.
    let thief = User { session: signing_key(&[99; 32]), token: alice.token.clone() };
    let h = signed(&mut a, &thief);
    assert_eq!(code(&call(&mut a, &mut ctx, "GET", &path, &h, "")), (401, "token_proof".into()));
    // A nonce the server never issued.
    let fake = format!("Authorization: Bearer {}\r\nX-ECM-Nonce: {}\r\nX-ECM-Proof: {}\r\n", alice.token, "ab".repeat(32), "cd".repeat(64));
    assert_eq!(code(&call(&mut a, &mut ctx, "GET", &path, &fake, "")), (401, "nonce_unknown".into()));
    // The real thing works.
    let h = signed(&mut a, &alice);
    assert_eq!(call(&mut a, &mut ConnCtx::new("10.0.0.5"), "GET", &path, &h, "").status, 200);
    // The same proof works again from the same address (a new connection, within 10 minutes) ...
    assert_eq!(call(&mut a, &mut ConnCtx::new("10.0.0.5"), "GET", &path, &h, "").status, 200);
    // ... but not from another address: the nonce was used.
    assert_eq!(code(&call(&mut a, &mut ConnCtx::new("10.0.0.6"), "GET", &path, &h, "")), (401, "nonce_unknown".into()));

    // --auth HOST:PORT before the realm's key is known: try again later.
    let own = public_key(&signing_key(&BACKEND));
    let mut remote = App::new(cfg(true, true), library_disk(), Gatekeeper::new(own, &"ab".repeat(16), None));
    remote.scan_now();
    let h = signed(&mut remote, &alice);
    assert_eq!(code(&call(&mut remote, &mut ctx, "GET", &path, &h, "")), (503, "realm_unavailable".into()));
    // With the realm's key (pinned by fingerprint): its tokens are accepted, the backend's own are not.
    let realm_seed = [5u8; 32];
    let realm_pub = public_key(&signing_key(&realm_seed));
    let mut gate = Gatekeeper::new(own, &fingerprint(&realm_pub), None);
    assert!(!gate.set_realm_pub(own), "a key with another fingerprint is refused");
    assert!(gate.set_realm_pub(realm_pub));
    let mut remote = App::new(cfg(true, true), library_disk(), gate);
    remote.scan_now();
    let shared = user_for("bob", &realm_seed, &backend_fp(), NOW + 3_600_000);
    let h = signed(&mut remote, &shared);
    assert_eq!(call(&mut remote, &mut ConnCtx::new("1.1.1.1"), "GET", &path, &h, "").status, 200);
    let h = signed(&mut remote, &alice);
    assert_eq!(code(&call(&mut remote, &mut ConnCtx::new("1.1.1.1"), "GET", &path, &h, "")), (401, "token_signature".into()));
}

#[test]
fn streaming_with_ranges_over_a_keep_alive_connection() {
    let mut a = app();
    let id = song_id(&a, "Sine Tone");
    let alice = user("alice");
    let h = signed(&mut a, &alice);
    let mut c = Conn::new("10.0.0.5:4000", NOW);
    // Two pipelined requests in one packet; the second needs no new nonce (same connection).
    let raw = format!(
        "GET /stream/{id} HTTP/1.1\r\n{h}Range: bytes=100-50099\r\n\r\nGET /stream/{id} HTTP/1.1\r\nAuthorization: Bearer {}\r\nRange: bytes=-10\r\n\r\n",
        alice.token
    );
    c.receive(&mut a, raw.as_bytes(), NOW);
    let first = c.drain_all(&a.disk);
    let text = String::from_utf8_lossy(&first[..200]).to_string();
    assert!(text.starts_with("HTTP/1.1 206 Partial Content\r\n"), "{text}");
    let head_end = first.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let head = String::from_utf8_lossy(&first[..head_end]).to_string();
    let total = CBR.len();
    let end = 50099.min(total - 1);
    assert!(head.contains(&format!("Content-Range: bytes 100-{end}/{total}\r\n")), "{head}");
    assert!(head.contains("Content-Type: audio/mpeg\r\n") && head.contains("Accept-Ranges: bytes\r\n") && head.contains("X-ECM-Duration-Ms: "));
    assert_eq!(&first[head_end..], &CBR[100..=end], "the body is the file's bytes, read a chunk at a time");
    // The pipelined second answer comes after the first is out.
    c.process(&mut a, NOW);
    let second = c.drain_all(&a.disk);
    let h2 = second.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    assert!(String::from_utf8_lossy(&second[..h2]).contains(&format!("Content-Range: bytes {}-{}/{total}", total - 10, total - 1)));
    assert_eq!(&second[h2..], &CBR[total - 10..]);
    assert_eq!(c.requests, 2);
    assert!(!c.finished(), "keep-alive");
    // A whole file, then past the end.
    c.receive(&mut a, format!("GET /stream/{id} HTTP/1.1\r\nAuthorization: Bearer {}\r\n\r\n", alice.token).as_bytes(), NOW);
    let whole = c.drain_all(&a.disk);
    let h3 = whole.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    assert!(whole.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert_eq!(&whole[h3..], CBR);
    c.receive(&mut a, format!("GET /stream/{id} HTTP/1.1\r\nAuthorization: Bearer {}\r\nRange: bytes={total}-\r\n\r\n", alice.token).as_bytes(), NOW);
    let past = String::from_utf8(c.drain_all(&a.disk)).unwrap();
    assert!(past.starts_with("HTTP/1.1 416") && past.contains(&format!("Content-Range: bytes */{total}")));
    // HEAD: the head only. Connection: close: closed after the answer.
    c.receive(&mut a, format!("HEAD /stream/{id} HTTP/1.1\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n", alice.token).as_bytes(), NOW);
    let h = c.drain_all(&a.disk);
    assert!(h.ends_with(b"\r\n\r\n") && h.starts_with(b"HTTP/1.1 200"));
    assert!(c.finished());
    // A bad request closes the connection with a 400; a huge body is refused.
    let mut bad = Conn::new("1.2.3.4:5", NOW);
    bad.receive(&mut a, b"BLAH\r\n\r\n", NOW);
    assert!(String::from_utf8(bad.drain_all(&a.disk)).unwrap().starts_with("HTTP/1.1 400"));
    assert!(bad.finished());
    let mut big = Conn::new("1.2.3.4:5", NOW);
    big.receive(&mut a, b"POST /playlists HTTP/1.1\r\nContent-Length: 999999\r\n\r\n", NOW);
    assert!(String::from_utf8(big.drain_all(&a.disk)).unwrap().starts_with("HTTP/1.1 413"));
}

#[test]
fn data_saver_is_made_in_the_background_then_streamed() {
    let mut a = app();
    let id = song_id(&a, "Café Ünïcode ß");
    let alice = user("alice");
    let h = signed(&mut a, &alice);
    let mut ctx = ConnCtx::new("10.0.0.5");
    let r = call(&mut a, &mut ctx, "GET", &format!("/stream/{id}?q=saver"), &h, "");
    assert_eq!(r.status, 503);
    assert!(head(&r).contains("Retry-After: 2"));
    assert_eq!(s(&json(&r), "code"), "preparing");
    let mut steps = 0;
    while a.busy() {
        a.step();
        steps += 1;
        assert!(steps < 1000);
    }
    assert!(steps > 3, "made a slice at a time ({steps} steps)");
    let r = call(&mut a, &mut ctx, "GET", &format!("/stream/{id}?q=saver"), &h, "");
    assert_eq!(r.status, 200);
    assert!(head(&r).contains(&format!("Content-Type: {SAVER_MIME}")));
    let Body::File { path, .. } = &r.body else { panic!() };
    let file = a.disk.read(path).unwrap();
    let sh = SaverHeader::parse(&file).unwrap();
    assert!((1900..2300).contains(&sh.duration_ms), "{}", sh.duration_ms);
    // A Range read from a block boundary.
    let off = sh.offset_for_ms(1000);
    let r = call(&mut a, &mut ctx, "GET", &format!("/stream/{id}?q=saver"), &format!("{h}Range: bytes={off}-\r\n"), "");
    assert_eq!(r.status, 206);
    assert_eq!(call(&mut a, &mut ctx, "GET", &format!("/stream/{id}?q=best"), &h, "").status, 400);
    // --transcode off: Original only.
    let mut off = app_with(true, false, library_disk());
    assert!(!off.busy());
    let h = signed(&mut off, &alice);
    let r = call(&mut off, &mut ConnCtx::new("10.0.0.5"), "GET", &format!("/stream/{id}?q=saver"), &h, "");
    assert_eq!((r.status, s(&json(&r), "code")), (404, "no_transcode"));
    assert_eq!(json(&call(&mut off, &mut ctx, "GET", "/info", "", "")).get("transcode").unwrap().as_bool(), Some(false));
}

#[test]
fn playlists_owner_rules_and_search() {
    let mut a = app();
    let s1 = song_id(&a, "Sine Tone");
    let s2 = song_id(&a, "Old Tag");
    let s3 = song_id(&a, "Café Ünïcode ß");
    let (alice, bob) = (user("alice"), user("bob"));
    let ha = signed(&mut a, &alice);
    let hb = signed(&mut a, &bob);
    let ta = format!("Authorization: Bearer {}\r\n", alice.token);
    let tb = format!("Authorization: Bearer {}\r\n", bob.token);
    let (mut ca, mut cb, mut anon) = (ConnCtx::new("10.0.0.5"), ConnCtx::new("10.0.0.6"), ConnCtx::new("10.0.0.7"));
    let r = call(&mut a, &mut ca, "POST", "/playlists", &ha, &format!(r#"{{"name":"Road Trip","songs":["{s1}","{s2}"]}}"#));
    assert_eq!(r.status, 201, "{}", r.text());
    let p = json(&r);
    let id = s(&p, "id").to_string();
    assert_eq!((s(&p, "owner"), p.get("public").unwrap().as_bool(), n(&p, "song_count")), ("alice", Some(false), 2));
    assert_eq!(p.get("mine").unwrap().as_bool(), Some(true));
    assert_eq!(s(&p.get("songs").unwrap().as_array().unwrap()[1], "id"), s2);
    // Private: nobody else sees it.
    let path = format!("/playlists/{id}");
    assert_eq!(call(&mut a, &mut cb, "GET", &path, &hb, "").status, 404);
    assert_eq!(call(&mut a, &mut anon, "GET", &path, "", "").status, 404);
    assert_eq!(call(&mut a, &mut cb, "PUT", &path, &hb, r#"{"name":"Mine"}"#).status, 404);
    assert_eq!(n(json(&call(&mut a, &mut ca, "GET", "/search?q=road", &ta, "")).get("playlists").unwrap(), "total"), 1);
    assert_eq!(n(json(&call(&mut a, &mut anon, "GET", "/search?q=road", "", "")).get("playlists").unwrap(), "total"), 0);
    assert_eq!(n(json(&call(&mut a, &mut cb, "GET", "/search?q=road", &tb, "")).get("playlists").unwrap(), "total"), 0);
    // Edit: add at the front, move, remove, rename, publish.
    let r = call(&mut a, &mut ca, "PUT", &path, &ta, &format!(r#"{{"add":["{s3}"],"at":0}}"#));
    assert_eq!(r.status, 200, "{}", r.text());
    let order = |j: &Json| j.get("songs").unwrap().as_array().unwrap().iter().map(|x| s(x, "id").to_string()).collect::<Vec<_>>();
    assert_eq!(order(&json(&r)), vec![s3.clone(), s1.clone(), s2.clone()]);
    let r = json(&call(&mut a, &mut ca, "PUT", &path, &ta, r#"{"move":{"from":0,"to":2},"name":"Summer","public":true}"#));
    assert_eq!(order(&r), vec![s1.clone(), s2.clone(), s3.clone()]);
    assert_eq!((s(&r, "name"), r.get("public").unwrap().as_bool()), ("Summer", Some(true)));
    let r = json(&call(&mut a, &mut ca, "DELETE", &format!("{path}/songs/1"), &ta, ""));
    assert_eq!(order(&r), vec![s1.clone(), s3.clone()]);
    let r = json(&call(&mut a, &mut ca, "POST", &format!("{path}/songs"), &ta, &format!(r#"{{"songs":["{s2}"]}}"#)));
    assert_eq!(order(&r), vec![s1.clone(), s3.clone(), s2.clone()]);
    let r = json(&call(&mut a, &mut ca, "PUT", &path, &ta, &format!(r#"{{"songs":["{s2}","{s2}"]}}"#)));
    assert_eq!(order(&r), vec![s2.clone(), s2.clone()]);
    // Public now: bob sees it but can't change it.
    let seen = json(&call(&mut a, &mut cb, "GET", &path, &hb, ""));
    assert_eq!(seen.get("mine").unwrap().as_bool(), Some(false));
    assert_eq!(call(&mut a, &mut anon, "GET", &path, "", "").status, 200);
    let r = call(&mut a, &mut cb, "PUT", &path, &tb, r#"{"name":"Mine"}"#);
    assert_eq!((r.status, s(&json(&r), "code")), (403, "not_owner"));
    assert_eq!(call(&mut a, &mut cb, "DELETE", &path, &tb, "").status, 403);
    assert_eq!(n(json(&call(&mut a, &mut anon, "GET", "/search?q=summer", "", "")).get("playlists").unwrap(), "total"), 1);
    // Lists.
    assert_eq!(json(&call(&mut a, &mut ca, "GET", "/playlists", &ta, "")).get("playlists").unwrap().as_array().unwrap().len(), 1);
    assert_eq!(json(&call(&mut a, &mut cb, "GET", "/playlists", &tb, "")).get("playlists").unwrap().as_array().unwrap().len(), 0);
    assert_eq!(json(&call(&mut a, &mut cb, "GET", "/playlists?owner=alice", &tb, "")).get("playlists").unwrap().as_array().unwrap().len(), 1);
    // Bad input.
    assert_eq!(s(&json(&call(&mut a, &mut ca, "PUT", &path, &ta, r#"{"add":["nope"]}"#)), "code"), "unknown_song");
    assert_eq!(call(&mut a, &mut ca, "PUT", &path, &ta, r#"{"remove":[9]}"#).status, 400);
    assert_eq!(call(&mut a, &mut ca, "PUT", &path, &ta, "{bad json").status, 400);
    assert_eq!(call(&mut a, &mut ca, "POST", "/playlists", &ta, r#"{"name":""}"#).status, 400);
    assert_eq!(call(&mut a, &mut anon, "POST", "/playlists", "", r#"{"name":"x"}"#).status, 401);
    // Kept across a restart.
    let disk = a.disk.clone();
    let own = public_key(&signing_key(&BACKEND));
    let mut again = App::new(cfg(true, true), disk, Gatekeeper::new(own, &fingerprint(&own), Some(own)));
    assert_eq!(again.lib.len(), 3, "the index was saved");
    assert_eq!(call(&mut again, &mut ConnCtx::new("1.1.1.1"), "GET", &path, "", "").status, 200);
    // Delete.
    assert_eq!(call(&mut a, &mut ca, "DELETE", &path, &ta, "").status, 204);
    assert_eq!(call(&mut a, &mut ca, "GET", &path, &ta, "").status, 404);
}

#[test]
fn likes_plays_and_recently_played() {
    let mut a = app();
    let s1 = song_id(&a, "Sine Tone");
    let s2 = song_id(&a, "Old Tag");
    let alice = user("alice");
    let h = signed(&mut a, &alice);
    let tc = format!("Authorization: Bearer {}\r\n", alice.token);
    let mut c = ConnCtx::new("10.0.0.5");
    assert_eq!(json(&call(&mut a, &mut c, "POST", &format!("/likes/{s1}"), &h, "")).get("liked").unwrap().as_bool(), Some(true));
    assert_eq!(call(&mut a, &mut c, "POST", "/likes/nope", &tc, "").status, 404);
    let likes = json(&call(&mut a, &mut c, "GET", "/likes", &tc, ""));
    assert_eq!(n(&likes, "total"), 1);
    assert_eq!(json(&call(&mut a, &mut c, "GET", &format!("/songs/{s1}"), &tc, "")).get("liked").unwrap().as_bool(), Some(true));
    assert_eq!(json(&call(&mut a, &mut ConnCtx::new("1.1.1.1"), "GET", &format!("/songs/{s1}"), "", "")).get("liked").unwrap().as_bool(), Some(false), "signed out");
    call(&mut a, &mut c, "DELETE", &format!("/likes/{s1}"), &tc, "");
    assert_eq!(n(&json(&call(&mut a, &mut c, "GET", "/likes", &tc, "")), "total"), 0);
    for id in [&s1, &s2, &s1] {
        call(&mut a, &mut c, "POST", &format!("/played/{id}"), &tc, "");
    }
    let p = json(&call(&mut a, &mut c, "POST", &format!("/played/{s2}"), &tc, ""));
    assert_eq!((n(&p, "plays"), n(&p, "user_plays")), (2, 2));
    let recent = json(&call(&mut a, &mut c, "GET", "/recent", &tc, ""));
    let ids: Vec<&str> = recent.get("songs").unwrap().as_array().unwrap().iter().map(|j| s(j, "id")).collect();
    assert_eq!(ids, vec![s2.as_str(), s1.as_str()]);
    assert_eq!(n(&json(&call(&mut a, &mut c, "GET", &format!("/songs/{s1}"), &tc, "")), "plays"), 2);
}

#[test]
fn rescans_keep_ids_and_pick_up_changes() {
    let mut a = app();
    let s1 = song_id(&a, "Sine Tone");
    assert!(!a.start_scan(), "nothing changed");
    a.disk.remove("/srv/music/old/tone_mpeg2.mp3");
    a.disk.put("/srv/music/new/again.mp3", VBR, 400);
    assert!(a.start_scan());
    while a.scanning() {
        a.step();
    }
    let r = a.last_scan.clone().unwrap();
    assert_eq!((r.added, r.removed, r.unchanged), (1, 1, 2));
    assert_eq!(song_id(&a, "Sine Tone"), s1, "same id after a rescan");
    assert_eq!(a.lib.len(), 3);
    let lines = a.status_lines().join("\n");
    assert!(lines.contains("3 songs") && lines.contains("no files above 192 kbps"), "{lines}");
}
