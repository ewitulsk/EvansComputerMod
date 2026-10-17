//! The Discord app on a scripted UI connection, a scripted network of real backends
//! and a scripted sound card.

use super::fake::{FakeAudio, FakeNet};
use super::*;
use crate::model::Page;
use ecm_discord::proto::Kind;
use ecm_discord::voice::{PacketKind, FRAME};
use ecm_ui::{samples as ui_samples, MockConnection};

pub fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("ecm-discord-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().replace('\\', "/")
}

fn configure(mock: &MockConnection, w: i32, h: i32, activated: bool, serial: u32) {
    mock.push(&[Message::Configure(proto::Configure {
        surface: 1,
        serial,
        w,
        h,
        states: if activated { proto::window_states::ACTIVATED } else { 0 },
    })]);
}

pub type D = DiscordApp<FakeNet, FakeAudio>;

/// A Discord app at `w x h` (activated) on `net`, with its data in a fresh folder.
pub fn start(name: &str, w: i32, h: i32, net: FakeNet, chat_conf: Option<&str>) -> (D, MockConnection) {
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    let mut wl = ui_samples::welcome(false);
    if let Message::Welcome(ref mut x) = wl {
        x.display.w = w;
        x.display.h = h;
        x.display.size_class = proto::SizeClass::for_width(w);
    }
    mock.push(&[wl]);
    let mut d = DiscordApp::new(Box::new(mock.clone()), net, FakeAudio::default(), Some(tmp(name)), chat_conf).unwrap();
    configure(&mock, w, h, true, 1);
    settle(&mut d);
    (d, mock)
}

/// Run turns until nothing is left to do.
pub fn settle(d: &mut D) {
    for _ in 0..12 {
        d.step(0);
    }
}

fn hit(d: &D, id: &str) -> u32 {
    d.app.core.window(1).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no widget {id}"))
}

pub fn click(d: &mut D, mock: &MockConnection, id: &str) {
    d.step(0);
    let h = hit(d, id);
    mock.push(&ui_samples::click(h, 10, 10));
    settle(d);
}

pub fn type_into(d: &mut D, mock: &MockConnection, id: &str, text: &str) {
    click(d, mock, id);
    mock.push(&[Message::Text(proto::Text { surface: 1, text: text.into() })]);
    settle(d);
}

/// Two backends announced on the network: "Tech Village" (home of bo) and "Lab".
pub fn two_backends() -> FakeNet {
    let mut net = FakeNet::default();
    net.now = 3_600_000;
    net.add("100.74.0.20", "Tech Village", true);
    net.add("10.0.8.1", "Lab", true);
    net
}

fn rail_names(d: &D) -> Vec<String> {
    d.model.borrow().rail().into_iter().map(|(_, g)| g.name.clone()).collect()
}

#[test]
fn finds_every_backend_and_merges_their_servers() {
    let (d, _mock) = start("merge", 640, 400, two_backends(), None);
    assert_eq!(d.net.searches, 1, "it looked on the network as it opened");
    let b = d.backends();
    assert_eq!(b.len(), 2);
    assert!(b.iter().all(|(_, _, origin, up)| *origin == Origin::Found && *up), "{b:?}");
    // Each backend's home server, side by side on the rail.
    assert_eq!(rail_names(&d), vec!["Tech Village", "Lab"]);
    // The first one is open, on #general (its history asked for).
    let m = d.model.borrow();
    assert_eq!(m.current().unwrap().1.name, "Tech Village");
    assert_eq!(m.current_channel().unwrap().name, "general");
    assert!(d.net.sent.iter().any(|(_, l)| l == "HISTORY 1"));
    assert!(d.net.sent.iter().any(|(_, l)| l == "HELLO pc-0a1b"), "nickname from the MAC");
}

#[test]
fn creates_a_server_on_the_chosen_backend_with_text_and_voice_channels() {
    let (mut d, mock) = start("create", 640, 400, two_backends(), None);
    click(&mut d, &mock, "add");
    assert_eq!(d.model.borrow().page, Page::Add);
    click(&mut d, &mock, "create");
    // Control: no name.
    click(&mut d, &mock, "create_server");
    assert!(d.model.borrow().notice.as_deref().unwrap_or("").contains("needs a name"));
    type_into(&mut d, &mock, "server_name", "Builders Guild");
    // Two backends are connected: pick the Lab one.
    click(&mut d, &mock, "host:1");
    click(&mut d, &mock, "create_server");
    assert_eq!(rail_names(&d), vec!["Tech Village", "Lab", "Builders Guild"]);
    assert_eq!(d.net.backends[1].backend.guild_names().last().unwrap().1, "Builders Guild", "made on Lab");
    {
        let m = d.model.borrow();
        assert_eq!(m.current().unwrap().1.name, "Builders Guild", "the new server opens");
        assert!(m.owns_current());
        assert_eq!(m.current_channel().unwrap().name, "general");
    }
    // The owner makes a text and a voice channel.
    click(&mut d, &mock, "new_text");
    type_into(&mut d, &mock, "channel_name", "Build Plans");
    click(&mut d, &mock, "create_channel");
    click(&mut d, &mock, "new_voice");
    type_into(&mut d, &mock, "channel_name", "The Lounge");
    click(&mut d, &mock, "create_channel");
    let m = d.model.borrow();
    let (b, _) = m.current().unwrap();
    let (_, gid) = m.guild.unwrap();
    let texts: Vec<String> = b.channels_of(gid, Kind::Text).iter().map(|(_, c)| c.name.clone()).collect();
    let voices: Vec<String> = b.channels_of(gid, Kind::Voice).iter().map(|(_, c)| c.name.clone()).collect();
    assert_eq!(texts, vec!["general", "build-plans"]);
    assert_eq!(voices, vec!["General", "The Lounge"]);
}

#[test]
fn talks_in_a_channel_and_gets_replies_and_notifications() {
    let (mut d, mock) = start("talk", 640, 400, two_backends(), None);
    // bo is on the Tech Village backend too.
    d.net.user(0, 9, "HELLO bo\n");
    type_into(&mut d, &mock, "compose", "hello bo");
    click(&mut d, &mock, "send");
    assert!(d.net.backends[0].backend.messages_of(1).contains(&"<pc-0a1b> hello bo".to_string()));
    let got = d.net.user(0, 9, "SAY 1 hi there\n");
    assert!(got.contains("M 1 2"), "{got}");
    settle(&mut d);
    {
        let m = d.model.borrow();
        let texts: Vec<String> = m.current_channel().unwrap().messages.iter().map(|x| x.text.clone()).collect();
        assert_eq!(texts, vec!["hello bo", "hi there"]);
    }
    assert!(d.posted.is_empty(), "control: no notification while Discord is on screen");
    // Off screen: a notification.
    configure(&mock, 640, 400, false, 2);
    settle(&mut d);
    d.net.user(0, 9, "SAY 1 still there?\n");
    settle(&mut d);
    assert_eq!(d.posted, vec![("bo in #general (Tech Village)".to_string(), "still there?".to_string())]);
}

#[test]
fn joins_a_server_another_user_made_on_another_backend() {
    let (mut d, mock) = start("join", 640, 400, two_backends(), None);
    d.net.user(1, 9, "HELLO cy\nGCREATE Cy's Corner\n");
    settle(&mut d);
    click(&mut d, &mock, "add");
    click(&mut d, &mock, "join");
    let gid = d.net.backends[1].backend.guild_names().last().unwrap().0;
    click(&mut d, &mock, &format!("join:1:{gid}"));
    assert_eq!(rail_names(&d), vec!["Tech Village", "Lab", "Cy's Corner"]);
    assert_eq!(d.model.borrow().current().unwrap().1.name, "Cy's Corner");
    assert!(!d.model.borrow().owns_current());
    assert_eq!(d.net.backends[1].backend.members_of(gid), vec!["cy", "pc-0a1b"]);
}

#[test]
fn a_typed_backend_and_chat_conf_are_connected_and_failures_retry() {
    let mut net = FakeNet::default();
    net.now = 1000;
    net.add("100.68.0.20", "Village", false); // not announced: only /etc/chat.conf knows it
    net.add("10.9.9.9", "Hidden", false);
    net.refuse.push("10.9.9.9".into());
    let conf = "server 100.68.0.20\nport 7777\nnick v1-house\n";
    let (mut d, mock) = start("typed", 640, 400, net, Some(conf));
    assert_eq!(d.backends()[0], ("Village".into(), "100.68.0.20".into(), Origin::Config, true));
    assert_eq!(d.model.borrow().nick, "v1-house", "the nickname from chat.conf");
    click(&mut d, &mock, "add");
    type_into(&mut d, &mock, "address", "10.9.9.9");
    click(&mut d, &mock, "add_backend");
    assert!(matches!(&d.model.borrow().backends[1].link, Link::Down(w) if w.contains("refused")));
    // It comes up later: retried after a while.
    d.net.refuse.clear();
    d.net.now += RETRY_MS + 1;
    settle(&mut d);
    assert!(d.model.borrow().backends[1].up());
    // Saved for next time.
    let saved = std::fs::read_to_string(format!("{}/settings", d.data_dir.clone().unwrap())).unwrap();
    assert!(saved.contains("backend 10.9.9.9:7777 Hidden") && saved.contains("nick v1-house"), "{saved}");
}

#[test]
fn voice_sends_the_microphone_and_plays_others() {
    let (mut d, mock) = start("voice", 640, 400, two_backends(), None);
    d.net.user(0, 9, "HELLO bo\n");
    // Join the home server's General voice channel (2).
    click(&mut d, &mock, "voice:2");
    assert!(d.audio.open, "the microphone and speaker opened");
    assert_eq!(d.net.backends[0].backend.voice_in(2), vec!["pc-0a1b"]);
    let first = Packet::decode(&d.net.voice_sent[0]).unwrap();
    assert_eq!(first.kind, PacketKind::KeepAlive, "a keep-alive tells the backend where we are");
    // We talk: the gate opens and the backend forwards our voice.
    let tone: Vec<i16> = (0..FRAME * 3).map(|i| ((i as f64 * 0.17).sin() * 9000.0) as i16).collect();
    d.audio.mic.extend(tone.iter().copied());
    settle(&mut d);
    assert!(d.voice_sent >= 3, "{}", d.voice_sent);
    assert!(d.model.borrow().talking);
    // Hiss isn't sent.
    let before = d.voice_sent;
    d.audio.mic.extend(std::iter::repeat_n(3i16, FRAME * 30));
    settle(&mut d);
    assert!(d.voice_sent - before <= 8, "only the hang time after the words");
    // bo joins and talks: we hear bo.
    let out = d.net.user(0, 9, "VJOIN 2\n");
    let token: u32 = out.lines().find(|l| l.starts_with("VOICE")).unwrap().split(' ').nth(3).unwrap().parse().unwrap();
    let mut bo = ecm_discord::voice::Sender::new(token);
    for p in bo.push(&tone) {
        d.net.user_udp(0, &p.encode());
    }
    settle(&mut d);
    assert!(d.voice_heard >= 3);
    assert!(d.audio.played.len() >= FRAME * 2, "{}", d.audio.played.len());
    assert!(ecm_audio::peak_dbfs(&d.audio.played) > -12);
    // Muted: nothing more is sent.
    click(&mut d, &mock, "mute");
    let before = d.voice_sent;
    d.audio.mic.extend(tone.iter().copied());
    settle(&mut d);
    assert_eq!(d.voice_sent, before);
    // Hang up.
    click(&mut d, &mock, "hangup");
    assert!(!d.audio.open);
    assert_eq!(d.net.backends[0].backend.voice_in(2), vec!["bo"]);
}

#[test]
fn a_loudspeaker_is_half_duplex_and_headphones_are_not() {
    let (mut d, mock) = start("duplex", 640, 400, two_backends(), None);
    let out = d.net.user(0, 9, "HELLO bo\nVJOIN 2\n");
    let token: u32 = out.lines().find(|l| l.starts_with("VOICE")).unwrap().split(' ').nth(3).unwrap().parse().unwrap();
    click(&mut d, &mock, "voice:2");
    d.net.user_udp(0, &ecm_discord::voice::Sender::new(token).keepalive().encode());
    let tone: Vec<i16> = (0..FRAME * 4).map(|i| ((i as f64 * 0.17).sin() * 9000.0) as i16).collect();
    let mut bo = ecm_discord::voice::Sender::new(token);
    // bo talks and we play it on the loudspeaker; then our microphone hears it (no AirPods).
    for p in bo.push(&tone) {
        d.net.user_udp(0, &p.encode());
    }
    settle(&mut d);
    d.audio.mic.extend(tone.iter().copied());
    settle(&mut d);
    assert!(!d.audio.played.is_empty());
    assert_eq!(d.voice_sent, 0, "the loudspeaker's sound isn't sent back (echo)");
    // With headphones the microphone stays open while others talk.
    d.audio.headphones = true;
    d.net.now += 2000;
    settle(&mut d);
    assert!(d.model.borrow().headphones);
    for p in bo.push(&tone) {
        d.net.user_udp(0, &p.encode());
    }
    d.audio.mic.extend(tone.iter().copied());
    settle(&mut d);
    assert!(d.voice_sent >= 3, "{}", d.voice_sent);
}

#[test]
fn compact_pages_go_back_and_forth() {
    let (mut d, mock) = start("compact", 240, 400, two_backends(), None);
    assert_eq!(d.model.borrow().page, Page::Servers, "a phone starts on the server list");
    click(&mut d, &mock, "chan:1");
    assert_eq!(d.model.borrow().page, Page::Chat);
    click(&mut d, &mock, "members");
    assert_eq!(d.model.borrow().page, Page::Members);
    click(&mut d, &mock, "back");
    assert_eq!(d.model.borrow().page, Page::Servers);
    click(&mut d, &mock, "guild:1");
    assert_eq!(d.model.borrow().current().unwrap().1.name, "Lab");
}

#[test]
fn a_backend_that_goes_away_drops_its_servers() {
    let (mut d, _mock) = start("down", 640, 400, two_backends(), None);
    let h = *d.net.conns.iter().find(|(_, b)| **b == 0).unwrap().0;
    d.net.conns.remove(&h);
    settle(&mut d);
    assert_eq!(rail_names(&d), vec!["Lab"]);
    assert_eq!(d.model.borrow().current().unwrap().1.name, "Lab", "another server opens");
}
