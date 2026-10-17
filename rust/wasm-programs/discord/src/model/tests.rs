use super::*;
use ecm_auth::RealmRef;

const FP: &str = "3fa19c0d00000000000000000000beef";

/// Backend `host` connected, greeted and signed in as alex (uid 1).
fn up(m: &mut Model, host: &str, name: &str) -> usize {
    let i = m.add_backend(host, DEFAULT_PORT, name, Origin::Found);
    m.on_connected(i);
    m.apply(i, &Event::Auth(RealmRef { name: name.into(), fp: FP.into(), endpoint: Some("-:7779".into()) }));
    m.apply(i, &Event::Backend { fp: FP.into(), nonce: "00".repeat(32), name: name.into() });
    assert_eq!(m.backends[i].auth, Auth::Ready);
    m.backends[i].auth = Auth::Hello;
    m.apply(i, &Event::Ready { uid: 1, name: "alex".into() });
    m.apply(i, &Event::Guild { id: 1, owner: 0, joined: true, members: 2, name: name.into() });
    m.apply(i, &Event::Channel { guild: 1, id: 1, kind: Kind::Text, name: "general".into() });
    m.apply(i, &Event::Channel { guild: 1, id: 2, kind: Kind::Voice, name: "General".into() });
    m.apply(i, &Event::Member { guild: 1, uid: 7, presence: Presence::Online, name: "Bo".into() });
    m.apply(i, &Event::Synced);
    i
}

#[test]
fn backends_are_listed_once_greeted_and_the_first_server_opens() {
    let mut m = Model::default();
    assert_eq!(m.add_backend("10.0.8.1", 7777, "Lab", Origin::Found), 0);
    assert_eq!(m.add_backend("10.0.8.1", 7777, "", Origin::Typed), 0, "same address: same backend");
    m.on_connected(0);
    assert!(m.backends[0].outbox.is_empty(), "no HELLO before the greeting and a token");
    assert_eq!(m.backends[0].auth, Auth::Greeting);
    // Requests wait for the sign-in.
    m.backends[0].guilds.insert(1, Guild { name: "x".into(), owner: 0, joined: true, members: 1 });
    m.guild = Some((0, 1));
    m.new_server = "X".into();
    assert!(m.create_server().is_err());
    m.guild = None;
    let a = up(&mut m, "10.0.8.1", "Lab");
    let b = up(&mut m, "10.0.9.1", "Village");
    assert_eq!((a, b), (0, 1));
    assert_eq!(m.backends[0].greeting.backend_fp, FP);
    assert_eq!(m.rail().len(), 2);
    assert_eq!(m.guild, Some((0, 1)), "the first server opened by itself");
    assert_eq!(m.channel, Some(1));
    assert_eq!(m.backends[0].outbox, vec!["HISTORY 1"]);
    assert_eq!(m.backends[0].my_name(), "alex");
}

#[test]
fn people_are_ids_and_names_follow_renames() {
    let mut m = Model::default();
    up(&mut m, "10.0.8.1", "Lab");
    m.apply(0, &Event::Msg { channel: 1, id: 9, time: "t".into(), uid: 7, text: "new".into(), earlier: false });
    m.apply(0, &Event::User { uid: 8, username: "cy".into(), name: "Cy".into() });
    m.apply(0, &Event::Msg { channel: 1, id: 3, time: "t".into(), uid: 8, text: "old".into(), earlier: true });
    m.apply(0, &Event::Msg { channel: 1, id: 3, time: "t".into(), uid: 8, text: "old".into(), earlier: true });
    let msgs = &m.current_channel().unwrap().messages;
    assert_eq!(msgs.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), vec!["old", "new"], "ordered by id, no duplicates");
    let b = &m.backends[0];
    assert_eq!((b.name_of(8), b.name_of(7)), ("Cy".into(), "Bo".into()));
    m.apply(0, &Event::Renamed { uid: 7, name: "Bo the Builder".into() });
    assert_eq!(m.backends[0].name_of(7), "Bo the Builder");
    // Our own messages are ours by id.
    m.apply(0, &Event::Msg { channel: 1, id: 12, time: "t".into(), uid: 1, text: "me".into(), earlier: false });
    assert!(m.current_channel().unwrap().messages.last().unwrap().mine);
    // Another channel counts unread; ours doesn't (we are looking).
    m.apply(0, &Event::Channel { guild: 1, id: 5, kind: Kind::Text, name: "news".into() });
    m.apply(0, &Event::Msg { channel: 5, id: 10, time: "t".into(), uid: 7, text: "psst".into(), earlier: false });
    assert_eq!(m.backends[0].channels[&5].unread, 1);
    m.open_channel(5);
    assert_eq!(m.backends[0].channels[&5].unread, 0);
    assert_eq!(m.page, Page::Chat);
}

#[test]
fn settings_round_trip_with_chat_conf() {
    let mut m = Model::default();
    up(&mut m, "10.0.8.1", "Lab");
    let i = m.add_backend("10.0.9.9", 7000, "", Origin::Typed);
    assert_eq!(m.backends[i].endpoint(), "10.0.9.9:7000");
    let text = m.settings_text();
    assert!(!text.contains("nick"), "names come from accounts now: {text}");
    let mut n = Model::default();
    n.load(Some(&text), Some("server 100.68.0.20\nnick v2-pc\n"));
    let hosts: Vec<(String, Origin)> = n.backends.iter().map(|b| (b.endpoint(), b.origin.clone())).collect();
    assert_eq!(
        hosts,
        vec![("100.68.0.20".into(), Origin::Config), ("10.0.8.1".into(), Origin::Saved), ("10.0.9.9:7000".into(), Origin::Saved)]
    );
    assert_eq!(n.reopen, Some(("10.0.8.1".into(), 7777, 1)));
}

#[test]
fn voice_follows_the_backend() {
    let mut m = Model::default();
    up(&mut m, "10.0.8.1", "Lab");
    m.join_voice(2).unwrap();
    assert_eq!(m.backends[0].outbox.last().unwrap(), "VJOIN 2");
    assert_eq!(m.voice_wanted, Some((0, 2)));
    m.apply(0, &Event::Voice { channel: 2, port: 7778, token: 55, sid: 4 });
    assert_eq!(m.voice, Some(VoiceSession { backend: 0, channel: 2, port: 7778, token: 55, sid: 4 }));
    m.apply(0, &Event::VoiceState { channel: 2, sid: 4, uid: 1, event: VoiceEvent::Join });
    m.apply(0, &Event::VoiceState { channel: 2, sid: 7, uid: 7, event: VoiceEvent::Join });
    m.apply(0, &Event::VoiceState { channel: 2, sid: 7, uid: 7, event: VoiceEvent::Speaking });
    assert!(m.backends[0].in_voice(2).iter().any(|(_, v)| v.uid == 7 && v.speaking));
    assert_eq!(m.voice_place(), Some(("Lab".into(), "General".into())));
    // The backend goes away: so does the call.
    m.on_down(0, "closed");
    assert!(m.voice.is_none());
    assert!(m.guild.is_none());
}

#[test]
fn nicknames_are_per_backend() {
    let mut m = Model::default();
    up(&mut m, "10.0.8.1", "Lab");
    up(&mut m, "10.0.9.1", "Village");
    m.backends[0].outbox.clear();
    m.compose("/nick Al the Builder");
    assert_eq!(m.backends[0].outbox, vec!["NICK Al the Builder"]);
    assert!(m.backends[1].outbox.iter().all(|l| !l.starts_with("NICK")), "only the open server's backend");
    m.compose("/nick");
    assert_eq!(m.backends[0].outbox.last().unwrap(), "NICK");
    m.open_settings();
    assert_eq!(m.backends[1].nick_draft, "alex");
    m.backends[1].nick_draft = "x".repeat(40);
    assert!(m.save_nick(1).is_err());
}

#[test]
fn errors_show_and_a_refused_hello_signs_in_again() {
    let mut m = Model::default();
    up(&mut m, "10.0.8.1", "Lab");
    m.apply(0, &Event::Err("only the server's owner can make channels".into()));
    assert_eq!(m.notice.as_deref(), Some("Lab: only the server's owner can make channels"));
    m.backends[0].auth = Auth::Hello;
    m.apply(0, &Event::Err("sign-in refused: the token has expired".into()));
    assert_eq!(m.backends[0].auth, Auth::Ready);
    assert!(m.backends[0].refused.is_some());
    m.on_down(0, "closed");
    m.new_server = "X".into();
    assert!(m.create_server().unwrap_err().contains("No Discord backend"));
    m.compose("hi");
    assert!(m.notice.is_some());
}
