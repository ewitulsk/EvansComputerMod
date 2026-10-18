use super::*;
use crate::proto::Greeting;
use crate::voice::Sender;
use ecm_auth::{signing_key, Claims, SigningKey};

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

fn realm_key() -> SigningKey {
    signing_key(&[7; 32])
}

fn backend_key() -> SigningKey {
    signing_key(&[8; 32])
}

fn setup() -> AuthSetup {
    let realm_pub = ecm_auth::public_key(&realm_key());
    AuthSetup {
        realm: RealmRef { name: "Tech Village".into(), fp: ecm_auth::fingerprint(&realm_pub), endpoint: Some("10.0.8.1:7600".into()) },
        realm_pub,
        backend_pub: ecm_auth::public_key(&backend_key()),
    }
}

fn backend() -> Backend {
    Backend::new("Tech Village", 7778, 42, setup())
}

/// A token from the realm for `audience`, bound to `session`.
fn token(uid: u64, user: &str, display: &str, audience: &str, session: &SigningKey, now: u64) -> String {
    Claims {
        realm_fp: setup().realm.fp,
        user_id: uid,
        username: user.into(),
        display_name: display.into(),
        audience: audience.into(),
        cnf: ecm_auth::public_key(session),
        issued_ms: now,
        expires_ms: now + ecm_auth::TOKEN_LIFETIME_MS,
    }
    .mint(&realm_key())
}

/// Connect `id` and sign in as user `uid` the way a client does: read the greeting,
/// prove the session key for the nonce. `chat` uses LOGIN (the chat command).
fn join(b: &mut Backend, id: u32, uid: u64, user: &str, display: &str, chat: bool) -> Vec<Out> {
    let greet = b.connect(id, &format!("10.0.0.{id}:5000"));
    let mut g = Greeting::default();
    for l in sent(&greet, id) {
        g.feed(&l);
    }
    assert!(g.complete(), "{greet:?}");
    let session = signing_key(&[uid as u8; 32]);
    let tok = token(uid, user, display, &g.backend_fp, &session, T);
    b.receive(id, format!("{}\n", g.sign_in_line(&tok, &session, chat)).as_bytes(), T)
}

/// A backend with Discord users alex (fd 1, uid 11) and bo (fd 2, uid 12).
fn two() -> Backend {
    let mut b = backend();
    join(&mut b, 1, 11, "alex", "Alex A.", false);
    join(&mut b, 2, 12, "bo", "Bo", false);
    b
}

fn gid_of(b: &Backend, name: &str) -> u32 {
    b.guild_names().into_iter().find(|(_, n)| n == name).unwrap().0
}

fn channel(b: &Backend, guild: u32, kind: Kind, name: &str) -> u32 {
    b.channels_of(guild).into_iter().find(|(_, k, n)| *k == kind && n == name).unwrap().0
}

#[test]
fn every_connection_is_greeted_with_the_realm_and_a_nonce() {
    let mut b = backend();
    let out = b.connect(1, "10.0.0.2:5000");
    let ev = events(&out, 1);
    assert_eq!(ev[0], Event::Auth(setup().realm));
    let Event::Backend { fp, nonce, name } = &ev[1] else { panic!("{ev:?}") };
    assert_eq!((fp.as_str(), name.as_str()), (b.audience().as_str(), "Tech Village"));
    assert_eq!(*fp, ecm_auth::fingerprint(&ecm_auth::public_key(&backend_key())));
    // Another connection gets another nonce.
    let Event::Backend { nonce: other, .. } = &events(&b.connect(2, "x"), 2)[1] else { panic!() };
    assert_ne!(nonce, other);
}

#[test]
fn nothing_works_before_signing_in_and_bad_tokens_are_refused() {
    let mut b = backend();
    let greet = b.connect(1, "10.0.0.2:5000");
    let mut g = Greeting::default();
    sent(&greet, 1).iter().for_each(|l| {
        g.feed(l);
    });
    // The old nickname-only ways in are gone.
    assert_eq!(sent(&b.receive(1, b"HELLO alex\n", T), 1)[0], "ERR sign in with a token and its proof (HELLO <token> <proof>)");
    assert_eq!(sent(&b.receive(1, b"GCREATE Mine\n", T), 1), vec!["ERR sign in first (HELLO <token> <proof>)"]);
    assert!(sent(&b.receive(1, b"NICK base\nMSG hi\n", T), 1).iter().all(|l| l.starts_with("ERR")));
    let session = signing_key(&[1; 32]);
    // A token made for another backend (its audience) is refused.
    let other = token(11, "alex", "Alex", &"ab".repeat(16), &session, T);
    let out = b.receive(1, format!("{}\n", g.sign_in_line(&other, &session, false)).as_bytes(), T);
    assert_eq!(sent(&out, 1), vec!["ERR sign-in refused: the token was made for another server"]);
    // A stolen token without the session key: the proof fails.
    let tok = token(11, "alex", "Alex", &g.backend_fp, &session, T);
    let thief = signing_key(&[2; 32]);
    let out = b.receive(1, format!("{}\n", g.sign_in_line(&tok, &thief, false)).as_bytes(), T);
    assert!(sent(&out, 1)[0].contains("session proof"), "{out:?}");
    // The third failure hangs up.
    let out = b.receive(1, format!("{}\n", g.sign_in_line(&tok, &session, false)).as_bytes(), T + 3 * ecm_auth::TOKEN_LIFETIME_MS);
    assert!(sent(&out, 1)[0].contains("expired"));
    assert!(out.contains(&Out::Close(1)));
    assert!(b.is_empty());
    // A token signed by another realm key.
    let mut b = backend();
    let greet = b.connect(5, "p");
    let mut g = Greeting::default();
    sent(&greet, 5).iter().for_each(|l| {
        g.feed(l);
    });
    let mut c = Claims::peek(&token(11, "alex", "Alex", &g.backend_fp, &session, T)).unwrap();
    c.username = "alex".into();
    let forged = c.mint(&signing_key(&[66; 32]));
    let out = b.receive(5, format!("{}\n", g.sign_in_line(&forged, &session, false)).as_bytes(), T);
    assert!(sent(&out, 5)[0].contains("not from the realm"), "{out:?}");
}

#[test]
fn hello_syncs_the_home_server_and_announces_presence() {
    let mut b = backend();
    let out = join(&mut b, 1, 11, "alex", "Alex A.", false);
    let ev = events(&out, 1);
    assert_eq!(ev[0], Event::Ready { uid: 11, name: "Alex A.".into() });
    assert!(ev.contains(&Event::Guild { id: HOME, owner: NOBODY, joined: true, members: 1, name: "Tech Village".into() }));
    assert!(ev.contains(&Event::Channel { guild: HOME, id: 1, kind: Kind::Text, name: "general".into() }));
    assert!(ev.contains(&Event::Channel { guild: HOME, id: 2, kind: Kind::Voice, name: "General".into() }));
    assert!(ev.contains(&Event::Member { guild: HOME, uid: 11, presence: Presence::Online, name: "Alex A.".into() }));
    assert_eq!(ev.last(), Some(&Event::Synced));
    assert!(out.contains(&Out::Save), "the new member is saved");

    // A second user: alex hears bo arrive, and the member count changed.
    let out = join(&mut b, 2, 12, "bo", "Bo", false);
    assert_eq!(events(&out, 2)[0], Event::Ready { uid: 12, name: "Bo".into() });
    let to_alex = events(&out, 1);
    assert!(to_alex.contains(&Event::Member { guild: HOME, uid: 12, presence: Presence::Online, name: "Bo".into() }));
    assert!(to_alex.iter().any(|e| matches!(e, Event::Guild { id: HOME, members: 2, joined: true, .. })));
    // The same account on a second device: one member, both connections signed in.
    let out = join(&mut b, 3, 11, "alex", "Alex A.", false);
    assert_eq!(events(&out, 3)[0], Event::Ready { uid: 11, name: "Alex A.".into() });
    assert_eq!(b.members_of(HOME), vec!["Alex A.", "Bo"]);
    assert_eq!(b.online_users(), vec!["alex", "bo"]);
    assert!(sent(&b.receive(3, b"HELLO x y\n", T), 3)[0].starts_with("ERR"));
}

#[test]
fn servers_are_made_joined_left_and_deleted() {
    let mut b = two();
    // alex makes a server: everyone sees it in the list; only alex is in it, with #general and General.
    let out = b.receive(1, b"GCREATE Builders Guild\n", T);
    let gid = gid_of(&b, "Builders Guild");
    assert!(events(&out, 1).contains(&Event::Guild { id: gid, owner: 11, joined: true, members: 1, name: "Builders Guild".into() }));
    assert!(events(&out, 2).contains(&Event::Guild { id: gid, owner: 11, joined: false, members: 1, name: "Builders Guild".into() }));
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
    assert!(to_bo.contains(&Event::Guild { id: gid, owner: 11, joined: true, members: 2, name: "Builders Guild".into() }));
    assert!(to_bo.iter().any(|e| matches!(e, Event::Channel { guild, .. } if *guild == gid)));
    assert!(to_bo.contains(&Event::Member { guild: gid, uid: 11, presence: Presence::Online, name: "Alex A.".into() }));
    assert!(events(&out, 1).contains(&Event::Member { guild: gid, uid: 12, presence: Presence::Online, name: "Bo".into() }));

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
    assert!(events(&out, 1).contains(&Event::Member { guild: gid, uid: 12, presence: Presence::Left, name: "Bo".into() }));
    assert!(events(&out, 2).contains(&Event::Guild { id: gid, owner: 11, joined: false, members: 1, name: "Builders Guild".into() }));

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
    let m = Event::Msg { channel: 1, id: 1, time: "01:00:05".into(), uid: 11, text: "hello bo".into(), earlier: false };
    assert!(events(&out, 1).contains(&m));
    assert!(events(&out, 2).contains(&m));
    // History, oldest first, capped, with its authors first.
    for i in 0..60 {
        b.receive(2, format!("SAY 1 m{i}\n").as_bytes(), T);
    }
    let out = b.receive(1, b"HISTORY 1\n", T);
    let hist: Vec<Event> = events(&out, 1);
    assert_eq!(hist[0], Event::User { uid: 12, username: "bo".into(), name: "Bo".into() });
    assert_eq!(hist.len(), 1 + proto::HISTORY + 1);
    assert!(matches!(&hist[1], Event::Msg { text, earlier: true, uid: 12, .. } if text == "m10"));
    assert_eq!(hist.last(), Some(&Event::HistoryEnd(1)));
    // Not in a voice channel, not in a channel that doesn't exist.
    assert!(sent(&b.receive(1, b"SAY 2 hi\n", T), 1)[0].contains("voice channel"));
    assert!(sent(&b.receive(1, b"SAY 99 hi\n", T), 1)[0].contains("no such channel"));
}

#[test]
fn the_chat_command_signs_in_and_talks_in_the_home_general() {
    let mut b = two();
    let out = join(&mut b, 3, 13, "base", "Base Camp", true);
    assert_eq!(sent(&out, 3)[0], "WELCOME base Tech Village");
    // Discord users see the chat user online in the home server (a member now, by account).
    assert!(events(&out, 1).contains(&Event::Member { guild: HOME, uid: 13, presence: Presence::Online, name: "Base Camp".into() }));
    // chat -> Discord.
    let out = b.receive(3, b"MSG hi from the console\n", T);
    assert!(events(&out, 2).iter().any(|e| matches!(e, Event::Msg { channel: 1, uid: 13, text, .. } if text == "hi from the console")));
    assert_eq!(sent(&out, 3), vec!["MSG 01:00:00 base hi from the console"]);
    // Discord -> chat, by username.
    let out = b.receive(1, b"SAY 1 hi base\n", T);
    assert_eq!(sent(&out, 3), vec!["MSG 01:00:00 alex hi base"]);
    // Other channels don't reach the chat command.
    b.receive(1, b"GCREATE Mine\n", T);
    let g = gid_of(&b, "Mine");
    let c = channel(&b, g, Kind::Text, "general");
    assert!(sent(&b.receive(1, format!("SAY {c} private\n").as_bytes(), T), 3).is_empty());
    // A new chat user gets #general's history; WHO lists everyone signed in.
    let out = join(&mut b, 4, 14, "cy", "Cy", true);
    assert!(sent(&out, 4).contains(&"HIST 01:00:00 base hi from the console".to_string()), "{:?}", sent(&out, 4));
    assert_eq!(sent(&b.receive(4, b"WHO\n", T), 4), vec!["WHO alex base bo cy"]);
    // Names come from the account; signing in twice is refused.
    assert!(sent(&b.receive(4, b"NICK other\n", T), 4)[0].contains("username"));
    assert!(sent(&b.receive(4, b"LOGIN a b\n", T), 4)[0].starts_with("ERR"));
    // Leaving: Discord users see the account go offline.
    let out = b.receive(3, b"QUIT\n", T);
    assert!(events(&out, 1).contains(&Event::Member { guild: HOME, uid: 13, presence: Presence::Offline, name: "Base Camp".into() }));
    assert_eq!(out.last(), Some(&Out::Close(3)));
}

#[test]
fn voice_is_forwarded_to_the_others_in_the_channel() {
    let mut b = two();
    join(&mut b, 3, 13, "cy", "Cy", false);
    // alex and bo in the General voice channel (2); cy in none.
    let out = b.receive(1, b"VJOIN 2\n", T);
    let Some(Event::Voice { port, token: ta, sid: sa, .. }) = events(&out, 1).into_iter().find(|e| matches!(e, Event::Voice { .. })) else {
        panic!("no VOICE: {:?}", sent(&out, 1))
    };
    assert_eq!(port, 7778);
    assert!(events(&out, 2).contains(&Event::VoiceState { channel: 2, sid: sa, uid: 11, event: VoiceEvent::Join }));
    let out = b.receive(2, b"VJOIN 2\n", T);
    let Some(Event::Voice { token: tb, sid: sb, .. }) = events(&out, 2).into_iter().find(|e| matches!(e, Event::Voice { .. })) else { panic!() };
    assert_ne!(ta, tb);
    assert_eq!(b.voice_in(2), vec!["Alex A.", "Bo"]);

    // bo's keep-alive tells the backend where bo is; alex talks: bo gets it, cy doesn't.
    let bo = Sender::new(tb);
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
    assert!(events(&out, 3).contains(&Event::VoiceState { channel: 2, sid: sa, uid: 11, event: VoiceEvent::Speaking }));
    assert!(b.anyone_speaking());
    // Control: a wrong token is ignored.
    let mut bad = pkt.clone();
    bad.id ^= 0xffff;
    assert!(b.udp([10, 0, 0, 9], 1, &bad.encode(), T).is_empty());
    // Silence: quiet after the hold time.
    assert!(b.tick(T + 100).is_empty());
    let out = b.tick(T + 1000);
    assert!(events(&out, 2).contains(&Event::VoiceState { channel: 2, sid: sa, uid: 11, event: VoiceEvent::Quiet }));

    // Leaving voice (and dropping the connection) is announced; packets stop going to bo.
    let out = b.receive(2, b"VLEAVE\n", T);
    assert!(events(&out, 1).contains(&Event::VoiceState { channel: 2, sid: sb, uid: 12, event: VoiceEvent::Leave }));
    let out = b.udp([10, 0, 0, 2], 6001, &pkt.encode(), T);
    assert!(!out.iter().any(|o| matches!(o, Out::Udp(..))));
    let out = b.disconnect(1, T);
    assert!(events(&out, 3).contains(&Event::VoiceState { channel: 2, sid: sa, uid: 11, event: VoiceEvent::Leave }));
    assert!(events(&out, 3).contains(&Event::Member { guild: HOME, uid: 11, presence: Presence::Offline, name: "Alex A.".into() }));
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
    b.receive(2, b"NICK Bo the Builder\n", T);
    let text = b.save_text();
    assert!(text.starts_with("discordd 2\n") && text.contains("user 11 alex Alex A.\n") && text.contains("nick 12 Bo the Builder\n"), "{text}");
    let c = Backend::load("Tech Village", 7778, 9, setup(), &text);
    assert_eq!(c.guild_names(), b.guild_names());
    assert_eq!(c.channels_of(g), b.channels_of(g));
    assert_eq!(c.members_of(g), vec!["Alex A.", "Bo the Builder"]);
    assert_eq!(c.messages_of(1), vec!["<Alex A.> remember me"]);
    // New ids continue after the loaded ones; the owner is still the owner by id.
    let mut c = c;
    join(&mut c, 1, 11, "alex", "Alex A.", false);
    assert!(sent(&c.receive(1, format!("CCREATE {g} text x\n").as_bytes(), T), 1).iter().all(|l| !l.starts_with("ERR")));
    c.receive(1, b"GCREATE Next\n", T);
    assert!(gid_of(&c, "Next") > g);
    // A renamed backend renames its home server; garbage and old nickname-era lines are skipped.
    let d = Backend::load("Renamed", 7778, 1, setup(), &format!("junk\nguild x y\nmember 1 alex\n{text}"));
    assert_eq!(d.guild_names()[0], (HOME, "Renamed".into()));
    assert_eq!(d.members_of(HOME), vec!["Alex A.", "Bo the Builder"]);
    // An empty file: just the home server.
    assert_eq!(Backend::load("V", 7778, 1, setup(), "").guild_names(), vec![(HOME, "V".into())]);
}

#[test]
fn nicknames_and_display_names_rename_by_user_id() {
    let mut b = two();
    b.receive(1, b"GCREATE Mine\n", T);
    let g = gid_of(&b, "Mine");
    let out = b.receive(1, b"NICK Al\n", T);
    assert!(events(&out, 2).contains(&Event::Renamed { uid: 11, name: "Al".into() }));
    assert_eq!(b.members_of(g), vec!["Al"]);
    // Still the owner (by id), and two people may share a name.
    assert!(sent(&b.receive(1, format!("CCREATE {g} text x\n").as_bytes(), T), 1).iter().all(|l| !l.starts_with("ERR")));
    assert!(sent(&b.receive(2, b"NICK Al\n", T), 2).iter().all(|l| !l.starts_with("ERR")));
    // NICK alone goes back to the display name.
    let out = b.receive(1, b"NICK\n", T);
    assert!(events(&out, 2).contains(&Event::Renamed { uid: 11, name: "Alex A.".into() }));
    // A display name changed at the realm shows at the next sign-in.
    let out = join(&mut b, 5, 12, "bo", "Bo Renamed", false);
    assert!(!events(&out, 1).iter().any(|e| matches!(e, Event::Renamed { uid: 12, .. })), "bo's nickname still wins");
    b.receive(2, b"NICK\n", T);
    assert_eq!(b.name_of(12), "Bo Renamed");
}

#[test]
fn full_backend_refuses_and_garbage_is_harmless() {
    let mut b = backend();
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
