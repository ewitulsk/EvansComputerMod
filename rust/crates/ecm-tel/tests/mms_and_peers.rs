//! MMS, group texts and calls between carriers end to end (docs/TELEPHONY.md):
//! M-Send.req over HTTP, WAP push through the SMSC, the fetch and the delivery
//! report; a group where a reply reaches everyone; a phone without MMS gets a
//! link; a second carrier reached by ENUM for a call, a text and a picture (MM4);
//! an unlisted peer gets 403.

mod common;

use common::*;
use ecm_tel::txn::Addr;
use ecm_tel::ua::{CallState, UaEvent};

const ALEX: &str = "+12145550142";
const BO: &str = "+12145550143";
const CY: &str = "+12145550144";
const DEE: &str = "+13125550170";

fn png(n: usize) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
    v.extend((0..n).map(|i| (i * 7) as u8));
    v
}

fn net() -> Net {
    let mut n = Net::new();
    n.add_carrier("cell-red.ecm", CORE, [10, 250, 0, 0], &["214-555"]);
    n.add_phone(0, "001010000000001", ALEX, [10, 250, 0, 2]);
    n.add_phone(0, "001010000000002", BO, [10, 250, 0, 3]);
    n.add_phone(0, "001010000000003", CY, [10, 250, 0, 4]);
    n
}

fn up(n: &mut Net) {
    for p in 0..n.phones.len() {
        n.power_on(p);
    }
    assert!(n.run_until(3000, |n| (0..n.phones.len()).all(|p| n.registered(p))), "{}", n.dump());
}

#[test]
fn a_picture_and_a_voice_note_reach_bo_and_alex_sees_delivered() {
    let mut n = net();
    up(&mut n);
    let wav = ecm_rtp::wav::write(16_000, &vec![1000i16; 16_000]);
    n.send_mms(0, &[BO], Some("our house"), vec![("image/png", "house.png", png(5000)), ("audio/wav", "note.wav", wav.clone())]);
    let mid = n.phones[0].mms_sent[0].clone().expect("Sent");
    assert!(n.run_until(3000, |n| !n.phones[1].mms.is_empty()), "{}", n.dump());
    let m = &n.phones[1].mms[0];
    assert_eq!((m.from.as_str(), m.text.as_deref()), (ALEX, Some("our house")));
    assert_eq!(m.files.len(), 2);
    assert_eq!(m.files[0].0, "image/png");
    assert_eq!(m.files[1].2, wav, "the voice note arrives intact");
    assert!(n.run_until(3000, |n| n.phones[0].events.iter().any(|e| matches!(e, UaEvent::MmsDelivered { message_id, to, status } if *message_id == mid && to == BO && status == "delivered"))), "{}", n.dump());
    assert!(n.carriers[0].cdrs.iter().any(|c| c.starts_with("mms ")));
    // too big for the network: the MMSC refuses it (Messages scales pictures down first)
    n.send_mms(0, &[BO], None, vec![("image/png", "huge.png", png(700 * 1024))]);
    assert!(n.phones[0].mms_sent[1].as_ref().unwrap_err().contains("too big"));
    // the MMSC only knows a sender by its data address: anyone else is refused
    let r = n.mms_http(0, "GET", "/mms/nonexistent", &[]);
    assert_eq!(r.status, 404);
}

#[test]
fn a_group_text_reaches_everyone_and_replies_go_to_the_group() {
    let mut n = net();
    up(&mut n);
    n.send_mms(0, &[BO, CY], Some("pizza tonight?"), vec![]);
    assert!(n.run_until(3000, |n| !n.phones[1].mms.is_empty() && !n.phones[2].mms.is_empty()), "{}", n.dump());
    let g = &n.phones[2].mms[0];
    assert_eq!(ecm_tel::mmsua::thread_key(CY, &g.from, &g.to), vec![ALEX.to_string(), BO.to_string()]);
    // cy replies to the group: to everyone else in it
    let to = ecm_tel::mmsua::thread_key(CY, &g.from, &g.to);
    let to: Vec<&str> = to.iter().map(|s| s.as_str()).collect();
    n.send_mms(2, &to, Some("yes!"), vec![]);
    assert!(n.run_until(3000, |n| n.phones[0].mms.len() == 1 && n.phones[1].mms.len() == 2), "{}", n.dump());
    let at_bo = &n.phones[1].mms[1];
    assert_eq!(at_bo.from, CY, "the bubble names its sender");
    assert_eq!(ecm_tel::mmsua::thread_key(BO, &at_bo.from, &at_bo.to), vec![ALEX.to_string(), CY.to_string()], "same thread as the first");
}

#[test]
fn a_phone_that_does_not_fetch_mms_gets_a_link() {
    let mut n = net();
    n.phones[1].ua.cfg.mms = false;
    up(&mut n);
    n.send_mms(0, &[BO], Some("see this"), vec![("image/png", "p.png", png(100))]);
    let link = n.run_until(3000, |n| n.phones[1].events.iter().any(|e| matches!(e, UaEvent::Sms { from, text, .. } if from == "MMS" && text.contains("/v/"))));
    assert!(link, "{}", n.dump());
    assert!(n.phones[1].mms.is_empty());
    let text = n.phones[1].events.iter().find_map(|e| if let UaEvent::Sms { text, from, .. } = e { (from == "MMS").then(|| text.clone()) } else { None }).unwrap();
    let path = text.split("8002").nth(1).unwrap().trim().to_string();
    let r = n.mms_http(1, "GET", &path, &[]);
    assert_eq!(r.status, 200);
    assert_eq!(r.content_type, "text/html");
    let page = String::from_utf8_lossy(&r.body);
    assert!(page.contains("see this") && page.contains("<img"), "{}", page);
    let bad = n.mms_http(1, "GET", &path.replace("r=", "r=00"), &[]);
    assert_eq!(bad.status, 403);
}

fn two_carriers(peered: bool) -> Net {
    let mut n = net();
    n.add_carrier("lakeline.ecm", [100, 66, 0, 9], [10, 251, 0, 0], &["312-555"]);
    n.add_phone(1, "001020000000001", DEE, [10, 251, 0, 2]);
    if peered {
        n.peer(0, 1, "s3cret");
    }
    n.enum_db.push(("+1312555".into(), "lakeline.ecm".into(), Addr::new([100, 66, 0, 9], 5060)));
    n.enum_db.push(("+1214555".into(), "cell-red.ecm".into(), Addr::new(CORE, 5060)));
    up(&mut n);
    n
}

#[test]
fn another_carrier_calls_texts_and_sends_a_picture_through_enum_and_mm4() {
    let mut n = two_carriers(true);
    let dee = 3;
    // a call from lakeline to cell-red
    let now = n.now;
    n.phones[dee].ua.dial(BO, false, now).unwrap();
    n.pump();
    assert!(n.run_until(3000, |n| n.phones[1].ua.calls().iter().any(|c| c.state == CallState::Incoming)), "{}", n.dump());
    assert!(n.phones[1].events.iter().any(|e| matches!(e, UaEvent::Incoming { from, .. } if from == DEE)), "caller ID across carriers");
    let id = n.phones[1].ua.calls()[0].id;
    let now = n.now;
    n.phones[1].ua.answer(id, now).unwrap();
    n.pump();
    assert!(n.run_until(2000, |n| n.phones[dee].ua.calls().iter().any(|c| c.state == CallState::Active)), "{}", n.dump());
    n.phones[dee].mic_level = 6000;
    n.run(400);
    n.phones[1].heard = 0;
    n.run(600);
    assert!(n.phones[1].heard > 100_000, "voice through both borders");
    n.phones[dee].mic_level = 0;
    let now = n.now;
    n.phones[1].ua.hangup(id, now).unwrap();
    n.pump();
    assert!(n.run_until(2000, |n| n.phones[dee].ua.calls().is_empty()));
    // a text
    let now = n.now;
    let sid = n.phones[dee].ua.send_sms(BO, "hello from lakeline", now).unwrap();
    n.pump();
    assert!(n.run_until(3000, |n| n.phones[1].events.iter().any(|e| matches!(e, UaEvent::Sms { from, text, .. } if from == DEE && text == "hello from lakeline"))), "{}", n.dump());
    assert!(n.run_until(3000, |n| n.phones[dee].events.contains(&UaEvent::SmsDelivered { id: sid })), "the report crosses back: {}", n.dump());
    // and a picture by MM4
    n.send_mms(dee, &[BO], Some("lake view"), vec![("image/png", "lake.png", png(3000))]);
    assert!(n.run_until(3000, |n| n.phones[1].mms.iter().any(|m| m.from == DEE)), "{}", n.dump());
    assert!(n.carriers[1].logs.iter().chain(&n.carriers[0].logs).any(|l| l.contains("MM4 from lakeline.ecm")), "{}", n.dump());
    assert!(n.run_until(3000, |n| n.phones[dee].events.iter().any(|e| matches!(e, UaEvent::MmsDelivered { status, .. } if status == "delivered"))), "{}", n.dump());
}

#[test]
fn an_unlisted_peer_gets_403() {
    let mut n = two_carriers(false);
    let dee = 3;
    // lakeline lists cell-red as a peer, but cell-red doesn't list lakeline
    n.carriers[1].ims.cfg.peers.push(ecm_tel::ims::Peer { domain: "cell-red.ecm".into(), addr: Some(Addr::new(CORE, 5060)), secret: "guess".into() });
    let now = n.now;
    n.phones[dee].ua.dial(BO, false, now).unwrap();
    n.pump();
    assert!(n.run_until(3000, |n| n.phones[dee].ua.calls().is_empty()), "{}", n.dump());
    assert!(!n.phones[1].events.iter().any(|e| matches!(e, UaEvent::Incoming { .. })));
    assert!(n.carriers[0].logs.iter().any(|l| l.contains("INVITE from unlisted peer")), "{}", n.dump());
    assert!(n.phones[dee].events.iter().any(|e| matches!(e, UaEvent::CallState { state: CallState::Ended, reason, .. } if reason == "call not allowed")), "{}", n.dump());
}
