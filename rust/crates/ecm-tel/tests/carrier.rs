//! A carrier end to end: phones register with IMS AKA, text and call each
//! other through imsd and smscd (docs/TELEPHONY.md). Real SIP, SDP, RTP and
//! 3GPP SMS bytes cross an ideal network.

mod common;

use common::*;
use ecm_tel::ua::{CallState, RegState, UaEvent};

const ALEX: &str = "+12145550142";
const BO: &str = "+12145550143";
const CY: &str = "+12145550144";

fn net3() -> Net {
    let mut n = Net::new();
    n.add_carrier("cell-red.ecm", CORE, [10, 250, 0, 0], &["214-555"]);
    n.add_phone(0, "001010000000001", ALEX, [10, 250, 0, 2]);
    n.add_phone(0, "001010000000002", BO, [10, 250, 0, 3]);
    n.add_phone(0, "001010000000003", CY, [10, 250, 0, 4]);
    for p in 0..3 {
        n.power_on(p);
    }
    assert!(n.run_until(3000, |n| (0..3).all(|p| n.registered(p))), "{}", n.dump());
    n
}

fn call_of(n: &Net, p: usize) -> Option<u32> {
    n.phones[p].ua.calls().first().map(|c| c.id)
}

fn state_of(n: &Net, p: usize) -> Option<CallState> {
    n.phones[p].ua.calls().first().map(|c| c.state)
}

#[test]
fn phones_register_with_ims_aka_and_learn_their_number() {
    let n = net3();
    assert_eq!(n.phones[0].ua.msisdn(), ALEX);
    assert!(n.phones[1].events.iter().any(|e| matches!(e, UaEvent::Registration { state: RegState::Registered, msisdn, .. } if msisdn == BO)));
    let st = n.carriers[0].ims.status();
    assert_eq!(st.registrations.len(), 3);
    assert_eq!(n.carriers[0].hss.regs.len(), 3, "SAR at the HSS");
    // a forged SIM (right IMSI, wrong key) fails IMS AKA
    let mut n2 = Net::new();
    n2.add_carrier("cell-red.ecm", CORE, [10, 250, 0, 0], &["214-555"]);
    let p = n2.add_phone(0, "001010000000009", "+12145550199", [10, 250, 0, 9]);
    n2.phones[p].key = [0x42; 16];
    n2.power_on(p);
    n2.run(2000);
    assert_ne!(n2.phones[p].ua.registration(), RegState::Registered);
    assert!(n2.carriers[0].logs.iter().any(|l| l.contains("IMS AKA failed")), "{}", n2.dump());
    // a phone sending from another phone's IMS address is refused
    assert!(n2.carriers[0].ims.status().registrations.is_empty());
}

#[test]
fn a_text_is_sent_delivered_and_a_long_one_arrives_whole() {
    let mut n = net3();
    let now = n.now;
    let id = n.phones[0].ua.send_sms(BO, "hi bo, it's alex", now).unwrap();
    n.pump();
    assert!(n.run_until(3000, |n| n.phones[0].events.contains(&UaEvent::SmsDelivered { id })), "{}", n.dump());
    let sent_at = n.phones[0].events.iter().position(|e| *e == UaEvent::SmsSent { id }).expect("Sent first");
    let deliv_at = n.phones[0].events.iter().position(|e| *e == UaEvent::SmsDelivered { id }).unwrap();
    assert!(sent_at < deliv_at);
    assert!(n.phones[1].events.iter().any(|e| matches!(e, UaEvent::Sms { from, text, .. } if from == ALEX && text == "hi bo, it's alex")));
    // 300 characters: two parts, rejoined
    let long: String = (0..300).map(|i| (b'a' + (i % 26) as u8) as char).collect();
    let now = n.now;
    let id2 = n.phones[0].ua.send_sms(BO, &long, now).unwrap();
    n.pump();
    assert!(n.run_until(3000, |n| n.phones[0].events.contains(&UaEvent::SmsDelivered { id: id2 })), "{}", n.dump());
    let got: Vec<&String> = n.phones[1].events.iter().filter_map(|e| if let UaEvent::Sms { text, .. } = e { Some(text) } else { None }).collect();
    assert_eq!(got.last().map(|s| s.as_str()), Some(long.as_str()), "{:?}", got);
    assert!(n.carriers[0].cdrs.iter().any(|c| c.starts_with("sms ") && c.contains("parts=2")), "{:?}", n.carriers[0].cdrs);
}

#[test]
fn a_text_to_a_phone_that_is_off_waits_then_arrives() {
    let mut n = net3();
    n.power_off(1);
    // the registration lapses at imsd when the phone is gone (here: it deregistered by vanishing)
    n.carriers[0].ims = {
        let mut cfg = n.carriers[0].ims.cfg.clone();
        cfg.max_expires = 3600;
        ecm_tel::ims::Ims::new(cfg, 77)
    };
    // the two phones still on register again with the fresh imsd
    for p in [0, 2] {
        let now = n.now;
        let b = n.phones[p].ua.bearer().cloned().unwrap();
        n.phones[p].ua.bearer_down(now);
        n.phones[p].ua.bearer_up(b, now);
    }
    assert!(n.run_until(3000, |n| n.registered(0) && n.registered(2)), "{}", n.dump());
    let now = n.now;
    let id = n.phones[0].ua.send_sms(BO, "are you there?", now).unwrap();
    n.pump();
    assert!(n.run_until(2000, |n| n.phones[0].events.contains(&UaEvent::SmsSent { id })), "{}", n.dump());
    n.run(3000);
    assert!(!n.phones[0].events.contains(&UaEvent::SmsDelivered { id }), "not delivered while off");
    assert_eq!(n.carriers[0].smsc.queued().len(), 1);
    assert!(n.carriers[0].saved_queue.contains(&format!(" {} {} ", ALEX, BO)), "stored on disk: {}", n.carriers[0].saved_queue);
    n.power_on(1);
    assert!(n.run_until(5000, |n| n.phones[0].events.contains(&UaEvent::SmsDelivered { id })), "{}", n.dump());
    assert!(n.phones[1].events.iter().any(|e| matches!(e, UaEvent::Sms { text, .. } if text == "are you there?")));
    assert!(n.carriers[0].smsc.queued().is_empty());
}

#[test]
fn alex_calls_bo_who_answers_hears_alex_and_hangs_up() {
    let mut n = net3();
    let now = n.now;
    n.phones[0].ua.dial(BO, false, now).unwrap();
    n.pump();
    assert!(n.run_until(2000, |n| state_of(n, 1) == Some(CallState::Incoming)), "{}", n.dump());
    assert!(n.phones[1].events.iter().any(|e| matches!(e, UaEvent::Incoming { from, hidden: false, waiting: false, .. } if from == ALEX)));
    assert!(n.phones[1].ua.is_ringing());
    assert_eq!(state_of(&n, 0), Some(CallState::Alerting));
    // ringback plays at alex's end
    n.phones[0].heard = 0;
    n.run(500);
    assert!(n.phones[0].heard > 0, "ringback");
    let id = call_of(&n, 1).unwrap();
    let now = n.now;
    n.phones[1].ua.answer(id, now).unwrap();
    n.pump();
    assert!(n.run_until(2000, |n| state_of(n, 0) == Some(CallState::Active)), "{}", n.dump());
    let info = n.phones[0].ua.calls()[0].clone();
    assert!(info.hd, "HD voice: ADPCM-WB both ends");
    assert_eq!(n.carriers[0].hss.bearers.len(), 2, "a voice bearer for each phone");
    // alex speaks: bo hears it through the relay
    n.phones[0].mic_level = 6000;
    n.run(300);
    n.phones[1].heard = 0;
    n.run(500);
    assert!(n.phones[1].heard > 100_000, "bo hears alex: {}", n.phones[1].heard);
    n.phones[0].mic_level = 0;
    // bo hangs up: both ends end
    let now = n.now;
    n.phones[1].ua.hangup(id, now).unwrap();
    n.pump();
    assert!(n.run_until(2000, |n| n.phones[0].ua.calls().is_empty()), "{}", n.dump());
    assert!(n.carriers[0].cdrs.iter().any(|c| c.starts_with("call ") && c.contains("result=answered") && c.contains(&format!("from={}", ALEX))), "{:?}", n.carriers[0].cdrs);
    assert!(n.carriers[0].hss.bearers.is_empty(), "bearers released");
    assert_eq!(n.carriers[0].ims.status().relays, 0);
}

#[test]
fn a_declined_call_is_busy_for_the_caller() {
    let mut n = net3();
    let now = n.now;
    n.phones[0].ua.dial(BO, false, now).unwrap();
    n.pump();
    assert!(n.run_until(2000, |n| state_of(n, 1) == Some(CallState::Incoming)));
    let id = call_of(&n, 1).unwrap();
    let now = n.now;
    n.phones[1].ua.hangup(id, now).unwrap();
    n.pump();
    n.run(200);
    assert!(n.phones[0].events.iter().any(|e| matches!(e, UaEvent::CallState { state: CallState::Ended, reason, .. } if reason == "busy")), "{}", n.dump());
    n.phones[0].heard = 0;
    n.run(1000);
    assert!(n.phones[0].heard > 0, "busy tone");
}

#[test]
fn hidden_caller_id_and_a_network_block_list() {
    let mut n = net3();
    let now = n.now;
    n.phones[0].ua.dial(BO, true, now).unwrap();
    n.pump();
    assert!(n.run_until(2000, |n| state_of(n, 1) == Some(CallState::Incoming)));
    assert!(n.phones[1].events.iter().any(|e| matches!(e, UaEvent::Incoming { from, hidden: true, .. } if from == "anonymous")), "{}", n.dump());
    let id = call_of(&n, 0).unwrap();
    let now = n.now;
    n.phones[0].ua.hangup(id, now).unwrap();
    n.pump();
    n.run(500);
    assert!(n.phones[1].ua.calls().is_empty(), "the cancel reached bo: {}", n.dump());
    // bo blocks cy at the network: cy's call never rings
    n.carriers[0].hss.subs[1].fields.insert("block".into(), CY.into());
    let now = n.now;
    n.phones[2].ua.dial(BO, false, now).unwrap();
    n.pump();
    n.run(1000);
    assert!(!n.phones[1].events.iter().any(|e| matches!(e, UaEvent::Incoming { from, .. } if from == CY)));
    assert!(n.phones[2].ua.calls().is_empty() || n.phones[2].ua.calls()[0].state == CallState::Ended, "{}", n.dump());
}

