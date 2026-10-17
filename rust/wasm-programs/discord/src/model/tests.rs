use super::*;

fn up(m: &mut Model, host: &str, name: &str) -> usize {
    let i = m.add_backend(host, DEFAULT_PORT, name, Origin::Found);
    m.on_connected(i);
    m.backends[i].outbox.clear();
    m.apply(i, &Event::Ready { nick: "alex".into(), backend: name.into() });
    m.apply(i, &Event::Guild { id: 1, owner: "-".into(), joined: true, members: 2, name: name.into() });
    m.apply(i, &Event::Channel { guild: 1, id: 1, kind: Kind::Text, name: "general".into() });
    m.apply(i, &Event::Channel { guild: 1, id: 2, kind: Kind::Voice, name: "General".into() });
    m.apply(i, &Event::Synced);
    i
}

#[test]
fn backends_are_listed_once_and_the_first_server_opens() {
    let mut m = Model { nick: "alex".into(), ..Default::default() };
    assert_eq!(m.add_backend("10.0.8.1", 7777, "Lab", Origin::Found), 0);
    assert_eq!(m.add_backend("10.0.8.1", 7777, "", Origin::Typed), 0, "same address: same backend");
    assert_eq!(m.backends[0].outbox, Vec::<String>::new());
    m.on_connected(0);
    assert_eq!(m.backends[0].outbox, vec!["HELLO alex"]);
    let a = up(&mut m, "10.0.8.1", "Lab");
    let b = up(&mut m, "10.0.9.1", "Village");
    assert_eq!((a, b), (0, 1));
    assert_eq!(m.rail().len(), 2);
    assert_eq!(m.guild, Some((0, 1)), "the first server opened by itself");
    assert_eq!(m.channel, Some(1));
    assert_eq!(m.backends[0].outbox, vec!["HISTORY 1"]);
}

#[test]
fn history_slots_in_before_new_messages_and_unread_counts() {
    let mut m = Model { nick: "alex".into(), ..Default::default() };
    up(&mut m, "10.0.8.1", "Lab");
    m.apply(0, &Event::Msg { channel: 1, id: 9, time: "t".into(), nick: "bo".into(), text: "new".into(), earlier: false });
    m.apply(0, &Event::Msg { channel: 1, id: 3, time: "t".into(), nick: "bo".into(), text: "old".into(), earlier: true });
    m.apply(0, &Event::Msg { channel: 1, id: 3, time: "t".into(), nick: "bo".into(), text: "old".into(), earlier: true });
    let texts: Vec<&str> = m.current_channel().unwrap().messages.iter().map(|x| x.text.as_str()).collect();
    assert_eq!(texts, vec!["old", "new"], "ordered by id, no duplicates");
    // Another channel counts unread; ours doesn't (we are looking).
    m.apply(0, &Event::Channel { guild: 1, id: 5, kind: Kind::Text, name: "news".into() });
    m.apply(0, &Event::Msg { channel: 5, id: 10, time: "t".into(), nick: "bo".into(), text: "psst".into(), earlier: false });
    assert_eq!(m.backends[0].channels[&5].unread, 1);
    assert_eq!(m.backends[0].channels[&1].unread, 0);
    m.open_channel(5);
    assert_eq!(m.backends[0].channels[&5].unread, 0);
    assert_eq!(m.page, Page::Chat);
}

#[test]
fn settings_round_trip_with_chat_conf() {
    let mut m = Model { nick: "alex".into(), ..Default::default() };
    up(&mut m, "10.0.8.1", "Lab");
    let i = m.add_backend("10.0.9.9", 7000, "", Origin::Typed);
    assert_eq!(m.backends[i].endpoint(), "10.0.9.9:7000");
    let text = m.settings_text();
    let mut n = Model::default();
    n.load(Some(&text), Some("server 100.68.0.20\nnick v2-pc\n"));
    assert_eq!(n.nick, "alex", "a saved nickname wins over chat.conf");
    let hosts: Vec<(String, Origin)> = n.backends.iter().map(|b| (b.endpoint(), b.origin.clone())).collect();
    assert_eq!(
        hosts,
        vec![("100.68.0.20".into(), Origin::Config), ("10.0.8.1".into(), Origin::Saved), ("10.0.9.9:7000".into(), Origin::Saved)]
    );
    assert_eq!(n.reopen, Some(("10.0.8.1".into(), 7777, 1)));
}

#[test]
fn voice_follows_the_backend() {
    let mut m = Model { nick: "alex".into(), ..Default::default() };
    up(&mut m, "10.0.8.1", "Lab");
    m.join_voice(2).unwrap();
    assert_eq!(m.backends[0].outbox.last().unwrap(), "VJOIN 2");
    assert_eq!(m.voice_wanted, Some((0, 2)));
    m.apply(0, &Event::Voice { channel: 2, port: 7778, token: 55, sid: 4 });
    assert_eq!(m.voice, Some(VoiceSession { backend: 0, channel: 2, port: 7778, token: 55, sid: 4 }));
    m.apply(0, &Event::VoiceState { channel: 2, sid: 4, nick: "alex".into(), event: VoiceEvent::Join });
    m.apply(0, &Event::VoiceState { channel: 2, sid: 7, nick: "bo".into(), event: VoiceEvent::Join });
    m.apply(0, &Event::VoiceState { channel: 2, sid: 7, nick: "bo".into(), event: VoiceEvent::Speaking });
    assert!(m.backends[0].in_voice(2).iter().any(|(_, v)| v.nick == "bo" && v.speaking));
    assert_eq!(m.voice_place(), Some(("Lab".into(), "General".into())));
    // The backend goes away: so does the call.
    m.on_down(0, "closed");
    assert!(m.voice.is_none());
    assert!(m.guild.is_none());
}

#[test]
fn errors_show_and_requests_need_a_connection() {
    let mut m = Model { nick: "alex".into(), ..Default::default() };
    up(&mut m, "10.0.8.1", "Lab");
    m.apply(0, &Event::Err("only the server's owner can make channels".into()));
    assert_eq!(m.notice.as_deref(), Some("Lab: only the server's owner can make channels"));
    m.on_down(0, "closed");
    m.new_server = "X".into();
    assert!(m.create_server().unwrap_err().contains("No Discord backend"));
    m.compose("hi");
    assert!(m.notice.is_some());
}
