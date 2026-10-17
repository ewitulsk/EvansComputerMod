use super::*;
use crate::voice::Sender;

fn sent(out: &[Out], to: u32) -> Vec<String> {
    out.iter()
        .filter_map(|o| match o {
            Out::Send(id, l) if *id == to => Some(l.clone()),
            _ => None,
        })
        .collect()
}

fn events(out: &[Out], to: u32) -> Vec<Event> {
    sent(out, to).iter().filter_map(|l| Event::parse(l)).collect()
}

const T: u64 = 3_600_000; // 01:00:00

/// A backend with Discord users alex (fd 1) and bo (fd 2).
fn two() -> Backend {
    let mut b = Backend::new("Tech Village", 7778, 42);
    b.connect(1, "10.0.0.2:5000");
    b.connect(2, "10.0.0.3:5000");
    b.receive(1, b"HELLO alex\n", T);
    b.receive(2, b"HELLO bo\n", T);
    b
}

fn gid_of(b: &Backend, name: &str) -> u32 {
    b.guild_names().into_iter().find(|(_, n)| n == name).unwrap().0
}

fn channel(b: &Backend, guild: u32, kind: Kind, name: &str) -> u32 {
    b.channels_of(guild).into_iter().find(|(_, k, n)| *k == kind && n == name).unwrap().0
}

#[test]
fn hello_syncs_the_home_server_and_announces_presence() {
    let mut b = Backend::new("Tech Village", 7778, 1);
    b.connect(1, "a");
    let out = b.receive(1, b"HELLO alex\n", T);
    let ev = events(&out, 1);
    assert_eq!(ev[0], Event::Ready { nick: "alex".into(), backend: "Tech Village".into() });
    assert!(ev.contains(&Event::Guild { id: HOME, owner: NOBODY.into(), joined: true, members: 1, name: "Tech Village".into() }));
    assert!(ev.contains(&Event::Channel { guild: HOME, id: 1, kind: Kind::Text, name: "general".into() }));
    assert!(ev.contains(&Event::Channel { guild: HOME, id: 2, kind: Kind::Voice, name: "General".into() }));
    assert!(ev.contains(&Event::Member { guild: HOME, nick: "alex".into(), presence: Presence::Online }));
    assert_eq!(ev.last(), Some(&Event::Synced));
    assert!(out.contains(&Out::Save), "the new member is saved");

    // A second user: alex hears bo arrive, and the member count changed.
    b.connect(2, "b");
    let out = b.receive(2, b"HELLO alex\n", T);
    assert_eq!(events(&out, 2)[0], Event::Ready { nick: "alex2".into(), backend: "Tech Village".into() }, "a taken nick gets a suffix");
    let to_alex = events(&out, 1);
    assert!(to_alex.contains(&Event::Member { guild: HOME, nick: "alex2".into(), presence: Presence::Online }));
    assert!(to_alex.iter().any(|e| matches!(e, Event::Guild { id: HOME, members: 2, joined: true, .. })));
}

#[test]
fn servers_are_made_joined_left_and_deleted() {
    let mut b = two();
    // alex makes a server: everyone sees it in the list; only alex is in it, with #general and General.
    let out = b.receive(1, b"GCREATE Builders Guild\n", T);
    let gid = gid_of(&b, "Builders Guild");
    assert!(events(&out, 1).contains(&Event::Guild { id: gid, owner: "alex".into(), joined: true, members: 1, name: "Builders Guild".into() }));
    assert!(events(&out, 2).contains(&Event::Guild { id: gid, owner: "alex".into(), joined: false, members: 1, name: "Builders Guild".into() }));
    assert!(events(&out, 1).iter().any(|e| matches!(e, Event::Channel { guild, kind: Kind::Text, name, .. } if *guild == gid && name == "general")));
    assert!(!events(&out, 2).iter().any(|e| matches!(e, Event::Channel { .. })), "non-members don't get its channels");

    // Control: bo can't post there before joining, nor make channels.
    let general = channel(&b, gid, Kind::Text, "general");
    let out = b.receive(2, format!("SAY {general} hi\n").as_bytes(), T);
    assert_eq!(sent(&out, 2), vec!["ERR join the server first"]);
    let out = b.receive(2, format!("CCREATE {gid} text plans\n").as_bytes(), T);
    assert_eq!(sent(&out, 2), vec!["ERR only the server's owner can make channels"]);

    // bo joins: gets the channels and members; alex sees bo arrive.
    let out = b.receive(2, format!("GJOIN {gid}\n").as_bytes(), T);
    let to_bo = events(&out, 2);
    assert!(to_bo.contains(&Event::Guild { id: gid, owner: "alex".into(), joined: true, members: 2, name: "Builders Guild".into() }));
    assert!(to_bo.iter().any(|e| matches!(e, Event::Channel { guild, .. } if *guild == gid)));
    assert!(to_bo.contains(&Event::Member { guild: gid, nick: "alex".into(), presence: Presence::Online }));
    assert!(events(&out, 1).contains(&Event::Member { guild: gid, nick: "bo".into(), presence: Presence::Online }));

    // The owner makes a text and a voice channel; members see them.
    let out = b.receive(1, format!("CCREATE {gid} text Build Plans\nCCREATE {gid} voice The Lounge\n").as_bytes(), T);
    let plans = channel(&b, gid, Kind::Text, "build-plans");
    let lounge = channel(&b, gid, Kind::Voice, "The Lounge");
    assert!(events(&out, 2).contains(&Event::Channel { guild: gid, id: plans, kind: Kind::Text, name: "build-plans".into() }));
    assert!(events(&out, 2).contains(&Event::Channel { guild: gid, id: lounge, kind: Kind::Voice, name: "The Lounge".into() }));
    let out = b.receive(1, format!("CCREATE {gid} text build-plans\n").as_bytes(), T);
    assert!(sent(&out, 1)[0].contains("already"), "{:?}", sent(&out, 1));

    // The owner can't leave; a member can.
    let out = b.receive(1, format!("GLEAVE {gid}\n").as_bytes(), T);
    assert!(sent(&out, 1)[0].contains("delete it instead"));
    let out = b.receive(2, format!("GLEAVE {gid}\n").as_bytes(), T);
    assert!(events(&out, 1).contains(&Event::Member { guild: gid, nick: "bo".into(), presence: Presence::Left }));
    assert!(events(&out, 2).contains(&Event::Guild { id: gid, owner: "alex".into(), joined: false, members: 1, name: "Builders Guild".into() }));

    // Only the owner deletes it; everyone hears.
    let out = b.receive(2, format!("GDELETE {gid}\n").as_bytes(), T);
    assert_eq!(sent(&out, 2), vec!["ERR only the server's owner can delete it"]);
    let out = b.receive(1, format!("GDELETE {gid}\n").as_bytes(), T);
    assert!(events(&out, 2).contains(&Event::GuildGone(gid)));
    assert!(b.channels_of(gid).is_empty());
    // The home server has no owner: nobody deletes it.
    let out = b.receive(1, b"GDELETE 1\n", T);
    assert!(sent(&out, 1)[0].starts_with("ERR"));
}

#[test]
fn messages_reach_members_and_are_kept() {
    let mut b = two();
    let out = b.receive(1, b"SAY 1 hello bo\n", T + 5000);
    let m = Event::Msg { channel: 1, id: 1, time: "01:00:05".into(), nick: "alex".into(), text: "hello bo".into(), earlier: false };
    assert!(events(&out, 1).contains(&m));
    assert!(events(&out, 2).contains(&m));
    // History, oldest first, capped.
    for i in 0..60 {
        b.receive(2, format!("SAY 1 m{i}\n").as_bytes(), T);
    }
    let out = b.receive(1, b"HISTORY 1\n", T);
    let hist: Vec<Event> = events(&out, 1);
    assert_eq!(hist.len(), proto::HISTORY + 1);
    assert!(matches!(&hist[0], Event::Msg { text, earlier: true, .. } if text == "m10"));
    assert_eq!(hist.last(), Some(&Event::HistoryEnd(1)));
    // Not in a voice channel, not in a channel that doesn't exist.
    assert!(sent(&b.receive(1, b"SAY 2 hi\n", T), 1)[0].contains("voice channel"));
    assert!(sent(&b.receive(1, b"SAY 99 hi\n", T), 1)[0].contains("no such channel"));
}

#[test]
fn the_chat_command_talks_in_the_home_general() {
    let mut b = two();
    b.connect(3, "10.0.0.9:4000");
    let out = b.receive(3, b"NICK base\n", T);
    assert_eq!(sent(&out, 3)[0], "WELCOME base Tech Village");
    // Discord users see the chat user online in the home server.
    assert!(events(&out, 1).contains(&Event::Member { guild: HOME, nick: "base".into(), presence: Presence::Online }));
    // chat -> Discord.
    let out = b.receive(3, b"MSG hi from the console\n", T);
    assert!(events(&out, 2).iter().any(|e| matches!(e, Event::Msg { channel: 1, nick, text, .. } if nick == "base" && text == "hi from the console")));
    assert_eq!(sent(&out, 3), vec!["MSG 01:00:00 base hi from the console"]);
    // Discord -> chat.
    let out = b.receive(1, b"SAY 1 hi base\n", T);
    assert_eq!(sent(&out, 3), vec!["MSG 01:00:00 alex hi base"]);
    // Other channels don't reach the chat command.
    b.receive(1, b"GCREATE Mine\n", T);
    let g = gid_of(&b, "Mine");
    let c = channel(&b, g, Kind::Text, "general");
    assert!(sent(&b.receive(1, format!("SAY {c} private\n").as_bytes(), T), 3).is_empty());
    // A new chat user gets #general's history; WHO lists everyone.
    b.connect(4, "x");
    let out = b.receive(4, b"NICK cy\nWHO\n", T);
    let lines = sent(&out, 4);
    assert!(lines.contains(&"HIST 01:00:00 base hi from the console".to_string()), "{lines:?}");
    assert_eq!(lines.last().unwrap(), "WHO alex base bo cy");
    // HELLO after joining the old way is refused.
    assert!(sent(&b.receive(4, b"HELLO cy\n", T), 4)[0].starts_with("ERR"));
    // Leaving: Discord users see "left" (the chat user was never a member).
    let out = b.receive(3, b"QUIT\n", T);
    assert!(events(&out, 1).contains(&Event::Member { guild: HOME, nick: "base".into(), presence: Presence::Left }));
    assert_eq!(out.last(), Some(&Out::Close(3)));
}

#[test]
fn voice_is_forwarded_to_the_others_in_the_channel() {
    let mut b = two();
    b.connect(3, "c");
    b.receive(3, b"HELLO cy\n", T);
    // alex and bo in the General voice channel (2); cy in none.
    let out = b.receive(1, b"VJOIN 2\n", T);
    let Some(Event::Voice { port, token: ta, sid: sa, .. }) = events(&out, 1).into_iter().find(|e| matches!(e, Event::Voice { .. })) else {
        panic!("no VOICE: {:?}", sent(&out, 1))
    };
    assert_eq!(port, 7778);
    assert!(events(&out, 2).contains(&Event::VoiceState { channel: 2, sid: sa, nick: "alex".into(), event: VoiceEvent::Join }));
    let out = b.receive(2, b"VJOIN 2\n", T);
    let Some(Event::Voice { token: tb, sid: sb, .. }) = events(&out, 2).into_iter().find(|e| matches!(e, Event::Voice { .. })) else { panic!() };
    assert_ne!(ta, tb);
    assert_eq!(b.voice_in(2), vec!["alex", "bo"]);

    // bo's keep-alive tells the backend where bo is; alex talks: bo gets it, cy doesn't.
    let mut bo = Sender::new(tb);
    assert!(b.udp([10, 0, 0, 3], 6000, &bo.keepalive().encode(), T).is_empty());
    let mut alex = Sender::new(ta);
    let tone: Vec<i16> = (0..crate::voice::FRAME).map(|i| ((i as f64 * 0.2).sin() * 8000.0) as i16).collect();
    let pkt = alex.push(&tone).pop().unwrap();
    let out = b.udp([10, 0, 0, 2], 6001, &pkt.encode(), T);
    let fwd: Vec<&Out> = out.iter().filter(|o| matches!(o, Out::Udp(..))).collect();
    assert_eq!(fwd.len(), 1);
    let Out::Udp(ip, port, data) = fwd[0] else { unreachable!() };
    assert_eq!((*ip, *port), ([10, 0, 0, 3], 6000));
    assert_eq!(Packet::decode(data).unwrap(), pkt.forwarded(sa));
    // Everyone in the server sees alex speaking (cy too: a member of the home server).
    assert!(events(&out, 3).contains(&Event::VoiceState { channel: 2, sid: sa, nick: "alex".into(), event: VoiceEvent::Speaking }));
    assert!(b.anyone_speaking());
    // Control: a wrong token is ignored; bo hasn't sent from anywhere new.
    let mut bad = pkt.clone();
    bad.id ^= 0xffff;
    assert!(b.udp([10, 0, 0, 9], 1, &bad.encode(), T).is_empty());
    // Silence: quiet after the hold time.
    assert!(b.tick(T + 100).is_empty());
    let out = b.tick(T + 1000);
    assert!(events(&out, 2).contains(&Event::VoiceState { channel: 2, sid: sa, nick: "alex".into(), event: VoiceEvent::Quiet }));

    // Leaving voice (and dropping the connection) is announced; packets stop going to bo.
    let out = b.receive(2, b"VLEAVE\n", T);
    assert!(events(&out, 1).contains(&Event::VoiceState { channel: 2, sid: sb, nick: "bo".into(), event: VoiceEvent::Leave }));
    let out = b.udp([10, 0, 0, 2], 6001, &pkt.encode(), T);
    assert!(!out.iter().any(|o| matches!(o, Out::Udp(..))));
    let out = b.disconnect(1, T);
    assert!(events(&out, 3).contains(&Event::VoiceState { channel: 2, sid: sa, nick: "alex".into(), event: VoiceEvent::Leave }));
    assert!(events(&out, 3).contains(&Event::Member { guild: HOME, nick: "alex".into(), presence: Presence::Offline }));
    assert!(b.voice_in(2).is_empty());
    // Not a text channel.
    assert!(sent(&b.receive(2, b"VJOIN 1\n", T), 2)[0].contains("text channel"));
}

#[test]
fn state_survives_a_restart() {
    let mut b = two();
    b.receive(1, b"GCREATE Builders\n", T);
    let g = gid_of(&b, "Builders");
    b.receive(1, format!("CCREATE {g} voice Lounge\n").as_bytes(), T);
    b.receive(2, format!("GJOIN {g}\n").as_bytes(), T);
    b.receive(1, b"SAY 1 remember me\n", T);
    let text = b.save_text();
    let c = Backend::load("Tech Village", 7778, 9, &text);
    assert_eq!(c.guild_names(), b.guild_names());
    assert_eq!(c.channels_of(g), b.channels_of(g));
    assert_eq!(c.members_of(g), vec!["alex", "bo"]);
    assert_eq!(c.messages_of(1), vec!["<alex> remember me"]);
    // New ids continue after the loaded ones.
    let mut c = c;
    c.connect(1, "a");
    c.receive(1, b"HELLO alex\n", T);
    c.receive(1, b"GCREATE Next\n", T);
    assert!(gid_of(&c, "Next") > g);
    // A renamed backend renames its home server; garbage lines are skipped.
    let d = Backend::load("Renamed", 7778, 1, &format!("junk\nguild x y\n{text}"));
    assert_eq!(d.guild_names()[0], (HOME, "Renamed".into()));
    // An empty file: just the home server.
    assert_eq!(Backend::load("V", 7778, 1, "").guild_names(), vec![(HOME, "V".into())]);
}

#[test]
fn renames_follow_membership_and_ownership() {
    let mut b = two();
    b.receive(1, b"GCREATE Mine\n", T);
    let g = gid_of(&b, "Mine");
    let out = b.receive(1, b"NICK al\n", T);
    assert!(events(&out, 2).contains(&Event::Renamed { old: "alex".into(), new: "al".into() }));
    assert_eq!(b.members_of(g), vec!["al"]);
    // Still the owner under the new name.
    assert!(sent(&b.receive(1, format!("CCREATE {g} text x\n").as_bytes(), T), 1).iter().all(|l| !l.starts_with("ERR")));
    assert!(sent(&b.receive(1, b"NICK bo\n", T), 1)[0].contains("taken"));
}

#[test]
fn full_backend_refuses_and_garbage_is_harmless() {
    let mut b = Backend::new("x", 7778, 1);
    for id in 0..MAX_CLIENTS as u32 {
        b.connect(id, "p");
    }
    let out = b.connect(100, "late");
    assert_eq!(out[1], Out::Close(100));
    let mut b = two();
    assert!(sent(&b.receive(1, b"FROB\n", T), 1)[0].starts_with("ERR unknown command"));
    assert!(b.receive(77, b"SAY 1 x\n", T).is_empty());
    assert!(b.udp([1, 2, 3, 4], 5, b"nonsense", T).is_empty());
    assert_eq!(sent(&b.receive(1, b"PING\n", T), 1), vec!["PONG"]);
}
