//! The engine and the app against real musicd backends (and their realms) on a
//! scripted network with a fake clock and speaker, and the window on a
//! scripted UI connection.

use super::fake::*;
use super::*;
use crate::engine::{Engine, Event, Quality, State, Track, AHEAD_MS, RATE, START_MS};
use ecm_auth::store::MemFiles;
use ecm_music::transcode::{decode_mp3_mono, SaverHeader};
use ecm_ui::{samples as ui_samples, MockConnection};

// ------------------------------------------------------------------ the world

const SEED: [u8; 32] = [7; 32];
const HOST: &str = "10.0.0.5";

fn songs() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("daft punk/long.mp3", long_song(20, "Long Road", "Daft Punk", "Discovery")),
        ("daft punk/second.mp3", long_song(5, "One More Time", "Daft Punk", "Discovery")),
        ("daft punk/slow.mp3", long_song(10, "Slow Song", "Daft Punk", "Homework")),
        ("test/tone_cbr.mp3", CBR.to_vec()),
    ]
}

fn world() -> FakeNet {
    FakeNet::new(vec![FakeServer::new(HOST, "Village Radio", SEED, &songs())])
}

fn file_of(net: &FakeNet, title: &str) -> Vec<u8> {
    let s = net.servers[0].app.lib.songs().iter().find(|s| s.title == title).unwrap().clone();
    net.servers[0].app.disk.files[&format!("/srv/music/{}", s.path)].0.clone()
}

fn track(net: &FakeNet, title: &str) -> Track {
    let s = net.servers[0].app.lib.songs().iter().find(|s| s.title == title).unwrap();
    Track { id: s.id.clone(), title: s.title.clone(), artist: s.artist.clone(), album: s.album.clone(), duration_ms: s.duration_ms, art: false, host: HOST.into(), port: 7800 }
}

/// Sign up on the backend's own realm and prove a token: the auth headers.
fn auth(net: &mut FakeNet) -> String {
    let mut acc = AccountStore::new(MemFiles::default());
    let realm = format!("{HOST}:7801");
    acc.sign_up(net.realm(&realm).unwrap(), &realm, "alice", "pw", None).unwrap();
    let fp = ecm_auth::fingerprint(&public_key_of(SEED));
    let (token, _) = acc.token(net.realm(&realm).unwrap(), &fp, &fp).unwrap();
    let now = net.now.get();
    let nonce = net.servers[0].app.gate.issue_nonce(ecm_auth::random(), now);
    let proof = acc.prove(&fp, &ecm_auth::unhex_n(&nonce).unwrap(), &fp).unwrap();
    format!("Authorization: Bearer {token}\r\nX-ECM-Nonce: {nonce}\r\nX-ECM-Proof: {}\r\n", ecm_auth::hex(&proof))
}

fn public_key_of(seed: [u8; 32]) -> [u8; 32] {
    ecm_auth::public_key(&ecm_auth::signing_key(&seed))
}

/// Run the engine for `ms` in 20 ms turns; the events, and the states seen.
fn run(e: &mut Engine, net: &mut FakeNet, spk: &mut FakeSpeaker, ms: u64) -> (Vec<Event>, Vec<State>) {
    let mut ev = Vec::new();
    let mut states = vec![e.state.clone()];
    for _ in 0..ms / 20 {
        net.advance(20);
        let now = net.now.get();
        ev.extend(e.tick(net, spk, now));
        if states.last() != Some(&e.state) {
            states.push(e.state.clone());
        }
    }
    (ev, states)
}

fn whole(net: &FakeNet, title: &str) -> usize {
    decode_mp3_mono(&file_of(net, title), RATE).len()
}

// ------------------------------------------------------------------ the engine

#[test]
fn starts_after_3_s_stays_30_s_ahead_and_plays_to_the_end() {
    let mut net = world();
    let mut spk = FakeSpeaker::new(net.now.clone());
    let h = auth(&mut net);
    let t = track(&net, "Long Road");
    assert!((39_000..44_000).contains(&t.duration_ms), "{}", t.duration_ms);
    // A link twice as fast as the song (32 kbps): it takes a moment to get 3 s.
    net.rate = Some(8000);
    let mut e = Engine::new();
    e.load(&mut net, &mut spk, t.clone(), Quality::Original, &h, 0, true);
    run(&mut e, &mut net, &mut spk, 800);
    assert_eq!(e.state, State::Buffering, "{} ms decoded", e.ahead_ms());
    assert!(e.ahead_ms() < START_MS && spk.played.is_empty(), "nothing plays before 3 s are decoded");
    run(&mut e, &mut net, &mut spk, 1500);
    assert_eq!(e.state, State::Playing);
    // As fast as asked: the download stops 30 s ahead.
    net.rate = None;
    run(&mut e, &mut net, &mut spk, 400);
    let total = file_of(&net, "Long Road").len() as u64;
    assert!(e.ahead_ms() >= AHEAD_MS && e.ahead_ms() < AHEAD_MS + 20_000, "{}", e.ahead_ms());
    assert!(e.downloaded() < total, "not everything at once: {} of {total}", e.downloaded());
    let before = e.downloaded();
    run(&mut e, &mut net, &mut spk, 400);
    assert_eq!(e.downloaded(), before, "nothing more while 30 s are ahead");
    // Pause and play again: what the speaker had queued is not lost.
    e.pause(&mut spk);
    let pos = e.position_ms();
    run(&mut e, &mut net, &mut spk, 2000);
    assert_eq!(e.position_ms(), pos, "paused");
    e.play();
    let (ev, _) = run(&mut e, &mut net, &mut spk, 45_000);
    assert!(ev.contains(&Event::Ended(t.id.clone())), "{ev:?}");
    assert!(ev.contains(&Event::Played(t.id.clone())));
    assert_eq!(e.state, State::Ended);
    let want = whole(&net, "Long Road");
    assert!(spk.played.len().abs_diff(want) < 50, "every sample played once: {} vs {want}", spk.played.len());
}

#[test]
fn a_slow_link_buffers_and_resumes_without_skipping() {
    let mut net = world();
    let mut spk = FakeSpeaker::new(net.now.clone());
    let h = auth(&mut net);
    let t = track(&net, "Slow Song");
    net.rate = Some(3000); // slower than the song's 4000 B/s
    let mut e = Engine::new();
    e.load(&mut net, &mut spk, t.clone(), Quality::Original, &h, 0, true);
    let (ev, states) = run(&mut e, &mut net, &mut spk, 60_000);
    let rebuffers = states.windows(2).filter(|w| w[0] == State::Playing && w[1] == State::Buffering).count();
    assert!(rebuffers >= 1, "it ran dry at least once: {states:?}");
    assert!(ev.contains(&Event::Ended(t.id.clone())));
    let want = whole(&net, "Slow Song");
    assert!(spk.played.len().abs_diff(want) < 50, "nothing skipped: {} vs {want}", spk.played.len());
}

#[test]
fn a_dropped_connection_resumes_at_the_byte_where_it_stopped() {
    let mut net = world();
    let mut spk = FakeSpeaker::new(net.now.clone());
    let h = auth(&mut net);
    let t = track(&net, "Slow Song");
    net.drop_after = Some(30_000);
    let mut e = Engine::new();
    e.load(&mut net, &mut spk, t.clone(), Quality::Original, &h, 0, true);
    let (ev, states) = run(&mut e, &mut net, &mut spk, 25_000);
    assert_eq!(net.drops, 1);
    let starts = net.range_starts(&t.id);
    assert_eq!(starts[0], 0);
    assert!((29_000..30_000).contains(&starts[1]), "the next read starts where the dropped one stopped: {starts:?}");
    assert!(ev.contains(&Event::Ended(t.id.clone())), "{states:?}");
    let want = whole(&net, "Slow Song");
    assert!(spk.played.len().abs_diff(want) < 50, "no gap, no repeat: {} vs {want}", spk.played.len());
    // A server that is down: Buffering and retries, then it comes back.
    let mut e = Engine::new();
    net.down.push(HOST.into());
    e.load(&mut net, &mut spk, t.clone(), Quality::Original, &h, 0, true);
    run(&mut e, &mut net, &mut spk, 3000);
    assert_eq!(e.state, State::Buffering);
    assert!(e.failures() >= 2);
    net.down.clear();
    run(&mut e, &mut net, &mut spk, 2000);
    assert_eq!(e.state, State::Playing);
}

#[test]
fn seeking_original_and_data_saver() {
    let mut net = world();
    let mut spk = FakeSpeaker::new(net.now.clone());
    let h = auth(&mut net);
    let t = track(&net, "Long Road");
    let mut e = Engine::new();
    e.load(&mut net, &mut spk, t.clone(), Quality::Original, &h, 0, true);
    run(&mut e, &mut net, &mut spk, 1000);
    let reads = net.range_starts(&t.id).len();
    e.seek(&mut net, &mut spk, 20_000);
    assert_eq!(e.position_ms(), 20_000);
    run(&mut e, &mut net, &mut spk, 2000);
    let p = e.position_ms();
    assert!((21_500..22_100).contains(&p), "{p} of {}", t.duration_ms);
    let off = net.range_starts(&t.id)[reads];
    // 32 kbps: 4000 bytes a second (proportional in a file without a Xing table).
    assert!(off.abs_diff(80_000) < 2_000, "{off}");
    // Data saver: block offsets.
    net.servers[0].make_saver();
    let cache = net.servers[0].app.cfg.cache_file(&t.id);
    let hdr = SaverHeader::parse(&net.servers[0].app.disk.files[&cache].0).unwrap();
    let mut e = Engine::new();
    e.load(&mut net, &mut spk, t.clone(), Quality::Saver, &h, 0, true);
    run(&mut e, &mut net, &mut spk, 500);
    let reads = net.range_starts(&t.id).len();
    e.seek(&mut net, &mut spk, 10_000);
    assert_eq!(e.position_ms(), hdr.ms_for_block(hdr.block_for_ms(10_000)));
    run(&mut e, &mut net, &mut spk, 1000);
    assert_eq!(net.range_starts(&t.id)[reads], hdr.offset_for_ms(10_000), "a Range read from the block boundary");
    assert_eq!(e.state, State::Playing);
    assert_eq!(e.quality(), Some(Quality::Saver));
    // Loading paused at a saved position (a cold boot): seeks once the header is in.
    let mut e = Engine::new();
    e.load(&mut net, &mut spk, t.clone(), Quality::Saver, &h, 12_000, false);
    run(&mut e, &mut net, &mut spk, 500);
    assert_eq!(e.state, State::Paused);
    assert!(e.position_ms().abs_diff(12_000) < 100, "{}", e.position_ms());
    e.play();
    run(&mut e, &mut net, &mut spk, 1000);
    assert!(e.position_ms() > 12_500);
}

#[test]
fn the_next_song_is_prefetched_and_starts_at_once() {
    let mut net = world();
    let mut spk = FakeSpeaker::new(net.now.clone());
    let h = auth(&mut net);
    let (a, b) = (track(&net, "One More Time"), track(&net, "Long Road"));
    let mut e = Engine::new();
    e.load(&mut net, &mut spk, a.clone(), Quality::Original, &h, 0, true);
    e.prefetch(&mut net, b.clone(), Quality::Original, &h);
    run(&mut e, &mut net, &mut spk, 400);
    assert!(e.prefetched_ms() >= 3000, "{}", e.prefetched_ms());
    let fetched = net.range_starts(&b.id).len();
    assert_eq!(fetched, 1, "only the start of the next song");
    let (ev, _) = run(&mut e, &mut net, &mut spk, 11_000);
    assert!(ev.contains(&Event::Ended(a.id.clone())));
    e.load(&mut net, &mut spk, b.clone(), Quality::Original, &h, 0, true);
    net.advance(20);
    let now = net.now.get();
    e.tick(&mut net, &mut spk, now);
    assert_eq!(e.state, State::Playing, "no buffering: its start was already here");
}

#[test]
fn data_saver_not_ready_plays_original() {
    let mut net = world();
    let mut spk = FakeSpeaker::new(net.now.clone());
    let h = auth(&mut net);
    let t = track(&net, "Long Road");
    let mut e = Engine::new();
    e.load(&mut net, &mut spk, t.clone(), Quality::Saver, &h, 0, true);
    let (ev, _) = run(&mut e, &mut net, &mut spk, 500);
    assert!(ev.contains(&Event::Fallback(t.id.clone())), "{ev:?}");
    assert_eq!((e.quality(), e.state.clone()), (Some(Quality::Original), State::Playing));
    // A token for another backend is refused: NeedAuth.
    let mut e = Engine::new();
    e.load(&mut net, &mut spk, t.clone(), Quality::Original, "Authorization: Bearer nope\r\n", 0, true);
    let (ev, _) = run(&mut e, &mut net, &mut spk, 300);
    assert!(ev.contains(&Event::NeedAuth { host: HOST.into(), port: 7800 }), "{ev:?}");
    assert_eq!(e.state, State::Buffering);
    e.set_auth(HOST, 7800, &h);
    run(&mut e, &mut net, &mut spk, 1500);
    assert_eq!(e.state, State::Playing);
}

// ------------------------------------------------------------------ the app

type A = MusicApp<FakeNet, FakeSpeaker>;

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("ecm-music-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().replace('\\', "/")
}

fn configure(mock: &MockConnection, w: i32, h: i32, serial: u32) {
    mock.push(&[Message::Configure(proto::Configure { surface: 1, serial, w, h, states: proto::window_states::ACTIVATED })]);
}

fn start_with(name: &str, w: i32, net: FakeNet, restore: Option<Vec<u8>>) -> (A, MockConnection) {
    start_sized(name, w, 400, net, restore)
}

fn start_sized(name: &str, w: i32, h: i32, net: FakeNet, restore: Option<Vec<u8>>) -> (A, MockConnection) {
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    let mut wl = ui_samples::welcome(restore.is_some());
    if let Message::Welcome(ref mut x) = wl {
        x.display.w = w;
        x.display.h = h;
        x.display.size_class = proto::SizeClass::for_width(w);
    }
    mock.push(&[wl]);
    if let Some(data) = restore {
        mock.push(&[Message::Restore(proto::Restore { data })]);
    }
    let spk = FakeSpeaker::new(net.now.clone());
    let mut a = MusicApp::new(Box::new(mock.clone()), net, spk, Some(tmp(name)), Box::new(MemFiles::default())).unwrap();
    configure(&mock, w, h, 1);
    settle(&mut a);
    (a, mock)
}

fn start(name: &str, w: i32) -> (A, MockConnection) {
    start_with(name, w, world(), None)
}

fn settle(a: &mut A) {
    for _ in 0..12 {
        a.step(0);
    }
}

/// Let `ms` pass in 20 ms turns.
fn play_for(a: &mut A, ms: u64) {
    for _ in 0..ms / 20 {
        a.net.advance(20);
        a.step(0);
    }
}

fn hit(a: &A, id: &str) -> u32 {
    a.app.core.window(1).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no widget {id}"))
}

fn click(a: &mut A, mock: &MockConnection, id: &str) {
    a.step(0);
    let h = hit(a, id);
    mock.push(&ui_samples::click(h, 10, 10));
    settle(a);
}

fn type_into(a: &mut A, mock: &MockConnection, id: &str, text: &str) {
    click(a, mock, id);
    mock.push(&[Message::Text(proto::Text { surface: 1, text: text.into() })]);
    settle(a);
}

/// Sign up as alice through the sign-in screen.
fn sign_up(a: &mut A, mock: &MockConnection) {
    click(a, mock, "signin:0");
    let screen = a.model.borrow().signin.clone().expect("the sign-in screen");
    assert!(!screen.borrow().pinned, "first contact");
    screen.borrow_mut().set_mode(ecm_apps_ui::signin::Mode::SignUp);
    assert!(screen.borrow_mut().submit("alice", "Alice", "pw", "pw"));
    settle(a);
    assert!(matches!(screen.borrow().stage, ecm_apps_ui::signin::Stage::SignedIn { .. }), "{:?}", screen.borrow().stage);
    click(a, mock, "cancel");
    assert_eq!(a.model.borrow().backends[0].user.as_deref(), Some("Alice"));
}

fn sent(mock: &MockConnection, f: impl Fn(&Message) -> bool) -> bool {
    mock.sent_messages().iter().any(f)
}

#[test]
fn finds_the_backend_signs_up_searches_as_you_type_and_plays_in_the_background() {
    let (mut a, mock) = start("play", 640);
    assert_eq!(a.net.searches, 1, "it looked on the network as it opened");
    {
        let m = a.model.borrow();
        assert_eq!(m.backends.len(), 1);
        assert_eq!(m.backends[0].name, "Village Radio");
        assert_eq!(m.backends[0].info.as_ref().unwrap().realm_host, "10.0.0.5:7801");
        assert_eq!(m.new_songs.len(), 4, "browsing works signed out");
        assert!(m.backends[0].user.is_none());
    }
    hit(&a, "signin:0");
    // Control: playing needs an account.
    let all = a.model.borrow().new_songs.clone();
    a.model.borrow_mut().act(Action::Play { songs: all, index: 0 });
    settle(&mut a);
    assert!(a.model.borrow().notice.as_deref().unwrap_or("").contains("Sign in"), "{:?}", a.model.borrow().notice);
    assert!(a.engine.track().is_none());
    sign_up(&mut a, &mock);
    click(&mut a, &mock, "tab:search");
    type_into(&mut a, &mock, "q", "daft pu");
    let r = a.model.borrow().results.clone();
    assert_eq!(r.top.as_ref().map(|t| t.0.as_str()), Some("artist"), "{r:?}");
    assert_eq!(r.songs.len(), 3, "Daft Punk's songs");
    let i = r.songs.iter().position(|s| s.title == "One More Time").unwrap();
    click(&mut a, &mock, &format!("song:search:{i}"));
    play_for(&mut a, 600);
    assert_eq!(a.engine.state, State::Playing);
    assert_eq!(a.model.borrow().queue.len(), 3, "the list is the queue");
    assert!(sent(&mock, |m| matches!(m, Message::BackgroundMode(b) if b.mode == "audio" && b.on)), "audio background mode while playing");
    let s = a.sessions.last().unwrap().clone();
    assert_eq!((s.title.as_str(), s.artist.as_str(), s.state), ("One More Time", "Daft Punk", ecm_ui::MediaState::Playing));
    assert!(s.actions.contains(&"pause".to_string()) && s.actions.contains(&"next".to_string()));
    let n = a.sessions.len();
    play_for(&mut a, 2100);
    assert!(a.sessions.len() >= n + 2, "the card is refreshed about once a second");
    assert!(a.sessions.last().unwrap().position_ms >= 2000);
    // Hotkey 1 pauses; the audio mode stays a while (Play can still reach Music), then ends.
    let off = |m: &MockConnection| m.sent_messages().iter().any(|x| matches!(x, Message::BackgroundMode(b) if b.mode == "audio" && !b.on));
    mock.push(&[Message::Hotkey(proto::Hotkey { slot: 1, down: true })]);
    settle(&mut a);
    assert!(!a.engine.playing());
    assert!(!off(&mock), "just paused: the audio mode is kept");
    a.net.advance(PAUSE_GRACE_MS + 1000);
    settle(&mut a);
    assert!(off(&mock), "a minute paused: the audio mode ends");
    mock.push(&[Message::MediaAction(proto::MediaAction { action: "play".into(), position_ms: 0 })]);
    settle(&mut a);
    assert!(a.engine.playing());
    // Next (hotkey 3) goes to the next song in the list; after half of a 10 s song it counted as played.
    play_for(&mut a, 3000);
    assert_eq!(a.played.len(), 1, "POST /played after half the song");
    mock.push(&[Message::Hotkey(proto::Hotkey { slot: 3, down: true })]);
    settle(&mut a);
    assert_eq!(a.model.borrow().pos, Some((i + 1) % 3));
    assert_eq!(a.net.servers[0].app.store.recent("alice").len(), 1);
}

#[test]
fn a_changed_key_is_refused_until_the_pin_is_forgotten_by_hand() {
    let (mut a, mock) = start("pins", 640);
    sign_up(&mut a, &mock);
    let old_fp = a.model.borrow().backends[0].info.as_ref().unwrap().realm_fp.clone();
    let expected = old_fp.clone();
    // Someone else answers at the same address now, with another key.
    a.net.servers[0] = FakeServer::new(HOST, "Village Radio", [9; 32], &songs());
    a.model.borrow_mut().backends[0].info = None;
    settle(&mut a);
    assert!(a.model.borrow().backends[0].user.is_none(), "the saved sign-in is not offered to the new key");
    click(&mut a, &mock, "signin:0");
    let screen = a.model.borrow().signin.clone().unwrap();
    assert!(matches!(screen.borrow().stage, ecm_apps_ui::signin::Stage::Changed { ref old_fp, .. } if *old_fp == expected), "{:?}", screen.borrow().stage);
    assert!(a.app.core.window(1).unwrap().hit_id_of("submit").is_none(), "no way to sign in or trust it here");
    click(&mut a, &mock, "cancel");
    // Settings > Accounts: forget the old key by hand; then it is a first contact again.
    click(&mut a, &mock, "tab:settings");
    click(&mut a, &mock, &format!("forget:{old_fp}"));
    assert!(a.model.borrow().pins.is_empty());
    click(&mut a, &mock, "signin:0");
    let screen = a.model.borrow().signin.clone().unwrap();
    assert_eq!(screen.borrow().stage, ecm_apps_ui::signin::Stage::Form);
    assert!(!screen.borrow().pinned);
}

#[test]
fn playlists_and_likes() {
    let (mut a, mock) = start("playlists", 640);
    sign_up(&mut a, &mock);
    click(&mut a, &mock, "tab:search");
    type_into(&mut a, &mock, "q", "long road");
    assert_eq!(a.model.borrow().results.songs[0].title, "Long Road");
    click(&mut a, &mock, "like:search:0");
    assert!(a.model.borrow().results.songs[0].liked);
    let id = a.net.servers[0].song_id("Long Road");
    assert_eq!(a.net.servers[0].app.store.liked("alice"), &[id.clone()]);
    click(&mut a, &mock, "add:search:0");
    assert!(matches!(a.model.borrow().page, Page::AddTo(_)));
    click(&mut a, &mock, "addto_new");
    type_into(&mut a, &mock, "pl_name", "Road Trip");
    click(&mut a, &mock, "create_playlist");
    assert_eq!(a.model.borrow().notice.as_deref(), Some("Added Long Road to Road Trip"));
    let pls = a.net.servers[0].app.store.playlists_of("alice").iter().map(|p| (p.name.clone(), p.songs.clone())).collect::<Vec<_>>();
    assert_eq!(pls, vec![("Road Trip".to_string(), vec![id.clone()])]);
    // Add a second song from the search, open it in the Library, reorder, make it public.
    type_into(&mut a, &mock, "q", "one more time");
    click(&mut a, &mock, "add:search:0");
    let pid = a.model.borrow().my_playlists[0].id.clone();
    click(&mut a, &mock, &format!("addto:{pid}"));
    click(&mut a, &mock, "tab:library");
    click(&mut a, &mock, &format!("playlist:{pid}"));
    assert_eq!(a.model.borrow().detail.playlist.as_ref().unwrap().songs.len(), 2);
    click(&mut a, &mock, "down:0");
    assert_eq!(a.model.borrow().detail.playlist.as_ref().unwrap().songs[1].id, id);
    click(&mut a, &mock, "public");
    assert!(a.net.servers[0].app.store.playlist(None, &pid).is_ok(), "public now");
    click(&mut a, &mock, "remove:0");
    assert_eq!(a.net.servers[0].app.store.playlist(None, &pid).unwrap().songs, vec![id]);
    click(&mut a, &mock, "play_all");
    play_for(&mut a, 400);
    assert_eq!(a.engine.track().unwrap().title, "Long Road");
}

#[test]
fn the_queue_and_position_survive_a_cold_boot_paused() {
    let (mut a, mock) = start("suspend", 640);
    sign_up(&mut a, &mock);
    let songs = a.model.borrow().new_songs.clone();
    let i = songs.iter().position(|s| s.title == "Long Road").unwrap();
    a.model.borrow_mut().act(Action::Play { songs, index: i });
    play_for(&mut a, 6000);
    let pos = a.engine.position_ms();
    assert!(pos > 2000, "{pos}");
    mock.push(&[Message::WillSuspend(proto::WillSuspend { deadline_ms: 500 })]);
    settle(&mut a);
    let data = mock.sent_messages().iter().find_map(|m| if let Message::SaveState(s) = m { Some(s.data.clone()) } else { None }).expect("SaveState");
    // A new run with the same accounts folder content.
    let net = std::mem::replace(&mut a.net, FakeNet::new(Vec::new()));
    let files = std::mem::replace(&mut a.accounts.files, AnyFiles(Box::new(MemFiles::default())));
    let (mut b, _mock2) = start_with("suspend2", 640, net, Some(data));
    b.accounts.files = files;
    b.model.borrow_mut().backends[0].info = None;
    settle(&mut b);
    play_for(&mut b, 400);
    assert_eq!(b.engine.track().map(|t| t.title.clone()), Some("Long Road".into()));
    assert_eq!(b.engine.state, State::Paused, "restored paused");
    assert!(b.engine.position_ms().abs_diff(pos) < 600, "{} vs {pos}", b.engine.position_ms());
    assert_eq!(b.model.borrow().queue.len(), 4);
}

#[test]
fn a_quality_change_applies_at_the_next_song() {
    let (mut a, mock) = start("quality", 640);
    sign_up(&mut a, &mock);
    a.net.servers[0].make_saver();
    click(&mut a, &mock, "tab:settings");
    click(&mut a, &mock, "quality:original");
    let songs = a.model.borrow().new_songs.clone();
    a.model.borrow_mut().act(Action::Play { songs, index: 0 });
    play_for(&mut a, 600);
    assert_eq!(a.engine.quality(), Some(Quality::Original));
    click(&mut a, &mock, "quality:saver");
    play_for(&mut a, 200);
    assert_eq!(a.engine.quality(), Some(Quality::Original), "the current song keeps playing as it was");
    a.model.borrow_mut().act(Action::Next);
    play_for(&mut a, 600);
    assert_eq!(a.engine.quality(), Some(Quality::Saver));
    assert_eq!(a.engine.state, State::Playing);
    // Auto: Data saver on cellular, Original on Wi-Fi.
    click(&mut a, &mock, "quality:auto");
    a.net.cellular = false;
    a.model.borrow_mut().act(Action::Next);
    play_for(&mut a, 400);
    assert_eq!(a.engine.quality(), Some(Quality::Original));
    a.net.cellular = true;
    a.model.borrow_mut().act(Action::Next);
    play_for(&mut a, 400);
    assert_eq!(a.engine.quality(), Some(Quality::Saver));
    // Settings are kept.
    let saved = std::fs::read_to_string(format!("{}/settings", a.data_dir.clone().unwrap())).unwrap();
    assert!(saved.starts_with("quality auto"), "{saved}");
}

#[test]
fn compact_has_tabs_at_the_bottom_and_a_player_bar() {
    let (mut a, mock) = start("compact", 240);
    sign_up(&mut a, &mock);
    let songs = a.model.borrow().new_songs.clone();
    a.model.borrow_mut().act(Action::Play { songs, index: 0 });
    play_for(&mut a, 400);
    hit(&a, "mini_toggle");
    click(&mut a, &mock, "mini_title");
    assert_eq!(a.model.borrow().tab, Tab::NowPlaying);
    hit(&a, "seek");
    click(&mut a, &mock, "toggle");
    assert!(!a.engine.playing());
    click(&mut a, &mock, "tab:home");
    hit(&a, "tab:library");
}

#[test]
fn previous_while_the_server_is_down_buffers_then_resumes() {
    let (mut a, mock) = start("prev", 640);
    sign_up(&mut a, &mock);
    let songs = a.model.borrow().new_songs.clone();
    let i = songs.iter().position(|s| s.title == "Long Road").unwrap();
    a.model.borrow_mut().act(Action::Play { songs, index: i });
    play_for(&mut a, 5000);
    assert_eq!(a.engine.state, State::Playing);
    a.net.down.push(HOST.into());
    a.net.drop_all();
    click(&mut a, &mock, "tab:now");
    click(&mut a, &mock, "prev");
    play_for(&mut a, 2000);
    assert!(a.model.borrow().now.buffering, "{:?} {}", a.engine.state, a.engine.position_ms());
    a.net.down.clear();
    play_for(&mut a, 3000);
    assert_eq!(a.engine.state, State::Playing);
}

#[test]
fn a_click_on_now_playing_with_cover_art() {
    let (mut a, mock) = start_sized("art", 480, 300, world(), None);
    sign_up(&mut a, &mock);
    let songs = a.model.borrow().new_songs.clone();
    let i = songs.iter().position(|s| s.title == "Sine Tone").unwrap();
    assert!(songs[i].art);
    a.model.borrow_mut().act(Action::Play { songs, index: i });
    play_for(&mut a, 500);
    click(&mut a, &mock, "tab:now");
    assert!(a.model.borrow().now.art != 0, "the cover is shown");
    play_for(&mut a, 1500);
    click(&mut a, &mock, "prev");
    play_for(&mut a, 300);
    assert!(a.engine.position_ms() < 1000, "{}", a.engine.position_ms());
}

#[test]
fn the_visualizer_commits_frames_at_the_speakers_pts_and_flushes_on_pause() {
    let (mut a, mock) = start("viz", 640);
    sign_up(&mut a, &mock);
    let songs = a.model.borrow().new_songs.clone();
    let i = songs.iter().position(|s| s.title == "Long Road").unwrap();
    a.model.borrow_mut().act(Action::Play { songs, index: i });
    play_for(&mut a, 1000);
    assert!(a.viz_commits.is_empty(), "control: not drawn while another tab is open");
    click(&mut a, &mock, "tab:now");
    play_for(&mut a, 2000);
    assert!(a.model.borrow().now.viz != 0, "the canvas is shown");
    let commits: Vec<u64> = mock
        .sent_messages()
        .iter()
        .filter_map(|m| if let Message::CommitAt(c) = m { Some(c.pts_us) } else { None })
        .collect();
    assert!(commits.len() >= 20, "about 15 frames a second: {}", commits.len());
    assert_eq!(commits, a.viz_commits);
    let pts = a.spk.pts.clone();
    for p in &commits {
        // Each frame is for when its audio plays: at or after the pts the speaker gave for that audio.
        assert!(pts.iter().any(|s| *p >= *s && *p - *s < 500_000), "frame {p} matches no speaker pts");
    }
    assert!(commits.windows(2).all(|w| w[1] > w[0]), "in play order");
    assert!(sent(&mock, |m| matches!(m, Message::SurfaceSetContent(c) if c.hint == 2)), "a motion canvas");
    let flushes = |m: &MockConnection| m.sent_messages().iter().filter(|x| matches!(x, Message::QueueFlush(_))).count();
    let before = flushes(&mock);
    click(&mut a, &mock, "toggle");
    assert!(!a.engine.playing());
    assert_eq!(flushes(&mock), before + 1, "pausing drops the queued frames");
    let n = a.viz_commits.len();
    play_for(&mut a, 1000);
    assert_eq!(a.viz_commits.len(), n, "nothing drawn while paused");
    click(&mut a, &mock, "toggle");
    play_for(&mut a, 600);
    a.model.borrow_mut().act(Action::Seek(20_000));
    play_for(&mut a, 40);
    assert_eq!(flushes(&mock), before + 2, "a seek drops them too");
}
