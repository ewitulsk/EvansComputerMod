//! Call features end to end (docs/TELEPHONY.md): voicemail on mrfd, intercept
//! announcements, the 611 line, forwarding, DND, call waiting, hold and swap,
//! three-way calls.

mod common;

use common::*;
use ecm_tel::ua::{CallState, UaEvent};

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

fn calls(n: &Net, p: usize) -> Vec<ecm_tel::ua::CallInfo> {
    n.phones[p].ua.calls()
}

fn dial(n: &mut Net, p: usize, to: &str) -> u32 {
    let now = n.now;
    let id = n.phones[p].ua.dial(to, false, now).unwrap();
    n.pump();
    id
}

fn ringing(n: &Net, p: usize) -> Option<u32> {
    calls(n, p).into_iter().find(|c| c.state == CallState::Incoming).map(|c| c.id)
}

fn answer(n: &mut Net, p: usize) {
    let id = ringing(n, p).expect("ringing");
    let now = n.now;
    n.phones[p].ua.answer(id, now).unwrap();
    n.pump();
}

fn active(n: &Net, p: usize) -> bool {
    calls(n, p).iter().any(|c| c.state == CallState::Active)
}

fn hang_up_all(n: &mut Net, p: usize) {
    for c in calls(n, p) {
        let now = n.now;
        let _ = n.phones[p].ua.hangup(c.id, now);
    }
    n.pump();
}

/// Does `p` hear something while only `talker` speaks?
fn hears(n: &mut Net, p: usize, talker: usize) -> bool {
    for x in &mut n.phones {
        x.mic_level = 0;
    }
    n.run(400);
    n.phones[talker].mic_level = 6000;
    n.run(300);
    n.phones[p].heard = 0;
    n.run(600);
    n.phones[talker].mic_level = 0;
    n.phones[p].heard > 100_000
}

#[test]
fn no_answer_goes_to_voicemail_and_bo_plays_it() {
    let mut n = net3();
    dial(&mut n, 0, BO);
    assert!(n.run_until(2000, |n| ringing(n, 1).is_some()));
    // nobody answers: after 30 s the caller is with voicemail
    assert!(n.run_until(35_000, |n| active(n, 0)), "{}", n.dump());
    assert!(ringing(&n, 1).is_none() && calls(&n, 1).is_empty(), "bo's phone stopped ringing: {}", n.dump());
    n.run(2500); // the greeting and the beep
    n.phones[0].mic_level = 5000;
    n.run(3000);
    n.phones[0].mic_level = 0;
    hang_up_all(&mut n, 0);
    n.run(500);
    let vm = n.carriers[0].mrf.mailbox(BO);
    assert_eq!(vm.len(), 1, "{}", n.dump());
    assert_eq!(vm[0].from, ALEX);
    assert!(vm[0].ms >= 2000, "{} ms", vm[0].ms);
    assert!(n.carriers[0].files.keys().any(|f| f.starts_with("/var/spool/vm/+12145550143/") && f.ends_with(".wav")));
    assert!(n.phones[1].events.contains(&UaEvent::Voicemail { waiting: 1 }), "{}", n.dump());
    // bo dials *86 and hears it
    dial(&mut n, 1, "*86");
    assert!(n.run_until(2000, |n| active(n, 1)), "{}", n.dump());
    n.phones[1].heard = 0;
    n.run(3000);
    assert!(n.phones[1].heard > 200_000, "the message plays: {}", n.phones[1].heard);
    assert!(n.run_until(8000, |n| n.carriers[0].mrf.mailbox(BO).iter().all(|m| m.heard)), "{}", n.dump());
    assert!(n.phones[1].events.contains(&UaEvent::Voicemail { waiting: 0 }));
    assert!(n.carriers[0].cdrs.iter().any(|c| c.contains("route=service")));
}

#[test]
fn a_number_nobody_has_gets_the_intercept_and_not_in_service() {
    let mut n = net3();
    dial(&mut n, 0, "+12145559999");
    assert!(n.run_until(2000, |n| calls(n, 0).first().is_some_and(|c| c.state == CallState::Alerting)), "early media: {}", n.dump());
    n.phones[0].heard = 0;
    n.run(1500);
    assert!(n.phones[0].heard > 100_000, "SIT tones: {}", n.phones[0].heard);
    assert!(n.run_until(6000, |n| calls(n, 0).is_empty()), "{}", n.dump());
    assert!(n.phones[0].events.iter().any(|e| matches!(e, UaEvent::CallState { state: CallState::Ended, reason, .. } if reason.contains("not in service"))), "{}", n.dump());
    // another carrier's number with no ENUM record: the same
    dial(&mut n, 0, "+13125550100");
    assert!(n.run_until(8000, |n| calls(n, 0).is_empty()));
    assert!(n.carriers[0].cdrs.iter().filter(|c| c.contains("result=not-in-service")).count() >= 2, "{:?}", n.carriers[0].cdrs);
}

#[test]
fn the_611_line_texts_the_balance_after_a_key() {
    let mut n = net3();
    let id = dial(&mut n, 0, "611");
    assert!(n.run_until(2000, |n| active(n, 0)), "{}", n.dump());
    let now = n.now;
    n.phones[0].ua.send_dtmf(id, "1", now).unwrap();
    assert!(
        n.run_until(4000, |n| n.phones[0].events.iter().any(|e| matches!(e, UaEvent::Sms { from, text, .. } if from == "611" && text.contains("used")))),
        "{}",
        n.dump()
    );
    let now = n.now;
    n.phones[0].ua.send_dtmf(id, "2", now).unwrap();
    assert!(n.run_until(4000, |n| n.phones[0].events.iter().any(|e| matches!(e, UaEvent::Sms { text, .. } if text.contains("(214) 555-0142")))), "{}", n.dump());
    // texting BAL to 22225 answers too
    let now = n.now;
    n.phones[0].ua.send_sms("22225", "BAL", now).unwrap();
    assert!(n.run_until(4000, |n| n.phones[0].events.iter().any(|e| matches!(e, UaEvent::Sms { from, .. } if from == "22225"))), "{}", n.dump());
}

#[test]
fn forwarding_with_star_codes_and_dnd() {
    let mut n = net3();
    // bo forwards everything to cy: *72 + number
    dial(&mut n, 1, &format!("*72{}", CY));
    n.run(1000);
    assert_eq!(n.carriers[0].hss.subs[1].fields.get("cfu").map(|s| s.as_str()), Some(CY), "{}", n.dump());
    dial(&mut n, 0, BO);
    assert!(n.run_until(2000, |n| ringing(n, 2).is_some()), "cy rings: {}", n.dump());
    assert!(ringing(&n, 1).is_none());
    answer(&mut n, 2);
    assert!(n.run_until(2000, |n| active(n, 0)));
    assert!(hears(&mut n, 2, 0));
    hang_up_all(&mut n, 0);
    n.run(500);
    assert!(n.carriers[0].cdrs.iter().any(|c| c.contains(&format!("to={}", CY)) && c.contains(&format!("dialled={}", BO))), "{:?}", n.carriers[0].cdrs);
    // *73 turns it off; DND sends the call to voicemail at once
    dial(&mut n, 1, "*73");
    n.run(1000);
    n.carriers[0].hss.subs[1].fields.insert("dnd".into(), "1".into());
    dial(&mut n, 0, BO);
    assert!(n.run_until(3000, |n| active(n, 0)), "voicemail answers: {}", n.dump());
    assert!(ringing(&n, 1).is_none() && ringing(&n, 2).is_none());
}

#[test]
fn call_waiting_hold_swap_and_a_three_way_call() {
    let mut n = net3();
    dial(&mut n, 0, BO);
    assert!(n.run_until(2000, |n| ringing(n, 1).is_some()));
    answer(&mut n, 1);
    assert!(n.run_until(2000, |n| active(n, 0)));
    // cy calls alex, who is in a call: a waiting call (a beep, no ringtone)
    dial(&mut n, 2, ALEX);
    assert!(n.run_until(2000, |n| ringing(n, 0).is_some()), "{}", n.dump());
    assert!(n.phones[0].events.iter().any(|e| matches!(e, UaEvent::Incoming { from, waiting: true, .. } if from == CY)));
    answer(&mut n, 0); // answer & hold: bo is held
    assert!(n.run_until(2000, |n| active(n, 2)), "{}", n.dump());
    let c = calls(&n, 0);
    assert!(c.iter().any(|x| x.remote == BO && x.state == CallState::Held), "{:?}", c);
    assert!(hears(&mut n, 2, 0), "alex and cy talk");
    assert!(!hears(&mut n, 1, 0), "bo, on hold, doesn't hear alex");
    // swap back to bo
    let now = n.now;
    n.phones[0].ua.swap(now).unwrap();
    n.pump();
    n.run(500);
    assert!(hears(&mut n, 1, 0), "bo again");
    assert!(!hears(&mut n, 2, 0), "cy is held now");
    // merge: everyone hears everyone
    let now = n.now;
    n.phones[0].ua.merge(now).unwrap();
    n.pump();
    n.run(1500);
    assert!(n.carriers[0].logs.iter().any(|l| l.contains("merged into conference")), "{}", n.dump());
    assert!(hears(&mut n, 2, 1), "cy hears bo");
    assert!(hears(&mut n, 1, 2), "bo hears cy");
    assert!(hears(&mut n, 0, 2), "alex hears cy");
    assert!(hears(&mut n, 1, 0), "bo hears alex");
    hang_up_all(&mut n, 0);
    n.run(1000);
    assert!(calls(&n, 1).is_empty() && calls(&n, 2).is_empty(), "{}", n.dump());
}

#[test]
fn without_call_waiting_a_second_caller_gets_voicemail() {
    let mut n = net3();
    n.carriers[0].hss.subs[0].fields.insert("cw".into(), "0".into());
    dial(&mut n, 1, ALEX);
    assert!(n.run_until(2000, |n| ringing(n, 0).is_some()));
    answer(&mut n, 0);
    assert!(n.run_until(2000, |n| active(n, 1)));
    dial(&mut n, 2, ALEX);
    assert!(n.run_until(3000, |n| active(n, 2)), "cy reaches alex's voicemail: {}", n.dump());
    assert!(ringing(&n, 0).is_none());
}
