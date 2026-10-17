//! Phone on a scripted UI connection over a scripted telephonyd.

use super::*;
use crate::book::Book;
use crate::fake::{FakeTel, MemFiles};
use crate::testkit::{configure, hit, mock, push_click, sent_text};
use ecm_tel::vcard::Contact;
use ecm_ui::{samples as ui_samples, MockConnection};

pub fn book_files() -> MemFiles {
    let mut f = MemFiles::default();
    let mut b = Book::default();
    let mut a = Contact::new("Alex Smith", "+12145550142");
    a.favourite = true;
    b.put(a);
    let mut bo = Contact::new("Bo Jones", "+12145550143");
    bo.tels.push(ecm_tel::vcard::Tel { label: "WORK".into(), number: "+12145550199".into() });
    b.put(bo);
    b.put(Contact::new("Cy", "+12145550144"));
    b.save(&mut f).unwrap();
    f
}

pub fn call(id: u32, state: &str, number: &str, seconds: u64) -> CallLine {
    CallLine { id, state: state.into(), number: number.into(), seconds, hd: true, codec: "ADPCM-WB".into(), outgoing: state != "incoming", ..Default::default() }
}

type A = DialerApp<FakeTel, MemFiles>;

fn start(tel: FakeTel, w: i32, h: i32, args: &[&str]) -> (A, MockConnection) {
    let m = mock(w, h);
    let mut a = DialerApp::new(Box::new(m.clone()), tel, book_files()).unwrap();
    a.start(&Args::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()));
    configure(&m, w, h);
    a.step(0, 0);
    a.step(0, 10);
    (a, m)
}

fn turn(a: &mut A, t: u64) {
    a.step(0, t);
    a.step(0, t + 1);
}

fn click(a: &mut A, m: &MockConnection, id: &str, t: u64) {
    push_click(&a.app, m, id);
    turn(a, t);
}

fn click_row(a: &mut A, m: &MockConnection, id: &str, i: usize, t: u64) {
    let h = a.app.core.window(1).unwrap().hit_id_of_row(id, i).unwrap_or_else(|| panic!("no row {i} of {id}"));
    m.push(&ui_samples::click(h, 10, 10));
    turn(a, t);
}

fn has(a: &A, id: &str) -> bool {
    a.app.core.window(1).unwrap().hit_id_of(id).is_some()
}

#[test]
fn args_parse() {
    let a = Args::parse(&["--incoming".into(), "7".into(), "--number".into(), "2145550142".into()]);
    assert_eq!(a, Args { incoming: Some(7), number: Some("2145550142".into()), call: None });
    assert_eq!(Vm::parse("vm 3 +12145550143 1200 5400 new"), Some(Vm { id: 3, from: "+12145550143".into(), time_ms: 1200, ms: 5400, new: true }));
    assert_eq!(Vm::parse("msg 3 x"), None);
}

#[test]
fn not_running() {
    let (_a, m) = start(FakeTel { down: true, ..Default::default() }, 320, 480, &[]);
    assert!(sent_text(&m).contains("telephonyd is not running"));
}

#[test]
fn keypad_call_in_call_and_back() {
    let (mut a, m) = start(FakeTel::new("+12145550100"), 320, 480, &[]);
    let t = sent_text(&m);
    assert!(t.contains("My number (214) 555-0100") && t.contains("VoLTE"), "header");
    a.model.borrow_mut().tab = Tab::Favourites;
    turn(&mut a, 50);
    // Keypad: 2 1 4 5 5 5 0 1 typed, formatted, the contact found
    a.model.borrow_mut().tab = Tab::Keypad;
    turn(&mut a, 100);
    for (i, d) in "21455501".chars().enumerate() {
        click(&mut a, &m, &format!("key_{d}"), 200 + i as u64 * 10);
    }
    assert_eq!(a.model.borrow().typed, "21455501");
    let t = sent_text(&m);
    assert!(t.contains("(214) 555-01"), "formatted as typed");
    assert!(t.contains("Alex Smith"), "the matching contact");
    // control: Delete takes the last digit off
    click(&mut a, &m, "del", 400);
    assert_eq!(a.model.borrow().typed, "2145550");
    for (i, d) in "142".chars().enumerate() {
        click(&mut a, &m, &format!("key_{d}"), 410 + i as u64 * 10);
    }
    click(&mut a, &m, "call", 500);
    assert_eq!(a.tel.sent("dial"), vec!["dial 2145550142"]);
    assert!(a.model.borrow().typed.is_empty());
    assert!(sent_text(&m).contains("Calling..."), "the in-call screen while it dials");
    // answered: 0:42, HD, codec line
    a.tel.calls[0].state = "active".into();
    a.tel.calls[0].seconds = 42;
    turn(&mut a, 2000);
    let t = sent_text(&m);
    assert!(t.contains("0:42") && t.contains("\"HD\"") && t.contains("ADPCM-WB"), "{t}");
    click(&mut a, &m, "mute", 2100);
    assert_eq!(a.tel.sent("mute"), vec!["mute 1 on"]);
    assert!(a.model.borrow().calls[0].muted && sent_text(&m).contains("Muted"), "muted, as calls says");
    click(&mut a, &m, "speaker", 2200);
    assert_eq!(a.tel.sent("speaker"), vec!["speaker on"]);
    assert!(a.tel.speaker && a.model.borrow().speaker, "status says the speaker is on");
    click(&mut a, &m, "hold", 2300);
    assert_eq!(a.tel.sent("hold"), vec!["hold 1 on"]);
    assert!(sent_text(&m).contains("Resume"), "held");
    click(&mut a, &m, "hold", 2350);
    assert_eq!(a.tel.sent("hold"), vec!["hold 1 on", "hold 1 off"]);
    // tones
    click(&mut a, &m, "keypad", 2400);
    click(&mut a, &m, "dtmf_5", 2500);
    click(&mut a, &m, "dtmf_hash", 2600);
    assert_eq!(a.tel.sent("dtmf"), vec!["dtmf 1 5", "dtmf 1 #"]);
    assert_eq!(a.model.borrow().dtmf, "5#");
    click(&mut a, &m, "keypad", 2700);
    click(&mut a, &m, "end", 2800);
    assert_eq!(a.tel.sent("hangup"), vec!["hangup 1"]);
    assert!(!a.model.borrow().in_call());
    assert_eq!(a.model.borrow().tab, Tab::Keypad, "back to the tab it was on");
    assert!(has(&a, "tabs"));
}

#[test]
fn call_args_and_add_call() {
    let (mut a, m) = start(FakeTel::new("+12145550100"), 640, 400, &["--call", "+12145550143"]);
    assert_eq!(a.tel.sent("dial"), vec!["dial +12145550143"]);
    a.tel.calls[0].state = "active".into();
    turn(&mut a, 1500);
    assert!(sent_text(&m).contains("Bo Jones"));
    // Add call: the keypad over the call, a second dial
    click(&mut a, &m, "add", 1600);
    assert!(has(&a, "return") && has(&a, "key_1"));
    a.model.borrow_mut().typed = "2145550144".into();
    turn(&mut a, 1700);
    click(&mut a, &m, "call", 1800);
    assert_eq!(a.tel.sent("dial").len(), 2);
    a.tel.calls[0].state = "held".into();
    a.tel.calls[1].state = "active".into();
    turn(&mut a, 3000);
    let t = sent_text(&m);
    assert!(t.contains("On hold: Bo Jones"), "{t}");
    click(&mut a, &m, "swap", 3100);
    click(&mut a, &m, "merge", 3200);
    assert_eq!((a.tel.sent("swap").len(), a.tel.sent("merge").len()), (1, 1));
    // both end: back on the tab from before Add call
    a.tel.calls.clear();
    a.tel.push_event("call", &[("call", "2"), ("state", "ended"), ("reason", "call ended")]);
    turn(&mut a, 3300);
    assert_eq!(a.model.borrow().tab, Tab::Favourites);
    assert!(sent_text(&m).contains("Call ended"));
}

#[test]
fn incoming_from_the_argument_answered_with_the_hotkey() {
    let mut tel = FakeTel::new("+12145550100");
    tel.calls.push(call(5, "incoming", "+12145550143", 0));
    let (mut a, m) = start(tel, 320, 480, &["--incoming", "5"]);
    let t = sent_text(&m);
    assert!(t.contains("Bo Jones") && t.contains("Incoming call") && t.contains("(214) 555-0143"), "{t}");
    assert!(t.contains("HotkeyClaim") && t.contains("\"Answer\"") && t.contains("\"Decline\""), "hotkeys claimed");
    m.clear_sent();
    m.push(&[Message::Hotkey(proto::Hotkey { slot: ANSWER_SLOT, down: true }), Message::Hotkey(proto::Hotkey { slot: ANSWER_SLOT, down: false })]);
    turn(&mut a, 100);
    assert_eq!(a.tel.sent("answer"), vec!["answer 5"]);
    assert!(a.model.borrow().incoming.is_none());
    let t = sent_text(&m);
    assert!(t.contains("HotkeyClaim") && t.contains("slot: 1, label: \"\""), "released: {t}");
    assert!(t.contains("0:00"), "the in-call screen");
}

#[test]
fn incoming_event_declined_with_a_message() {
    let (mut a, m) = start(FakeTel::new("+12145550100"), 320, 480, &[]);
    assert!(a.model.borrow().incoming.is_none());
    a.tel.calls.push(call(9, "incoming", "+12145550177", 0));
    a.tel.push_event("incoming", &[("call", "9"), ("from", "+12145550177"), ("hidden", "0"), ("waiting", "0")]);
    turn(&mut a, 100);
    assert!(sent_text(&m).contains("(214) 555-0177"));
    click(&mut a, &m, "message_reply", 200);
    assert_eq!(a.tel.sent("decline"), vec!["decline 9"]);
    let t = format!("{:?}", m.sent_messages());
    assert!(t.contains("Launch") && t.contains("dev.ecm.messages") && t.contains("--to") && t.contains("+12145550177"), "{t}");
    assert!(has(&a, "tabs"), "back to the tabs");
}

#[test]
fn hidden_caller_and_call_waiting() {
    let mut tel = FakeTel::new("+12145550100");
    tel.calls.push(call(1, "active", "+12145550142", 30));
    tel.calls.push(CallLine { hidden: true, ..call(2, "incoming", "anonymous", 0) });
    let (mut a, m) = start(tel, 320, 480, &[]);
    let t = sent_text(&m);
    assert!(t.contains("No Caller ID") && t.contains("Call waiting") && t.contains("Hold & answer") && t.contains("End & answer"), "{t}");
    assert!(!has(&a, "message_reply"));
    click(&mut a, &m, "hold_answer", 100);
    assert_eq!(a.tel.sent("hold"), vec!["hold 1 on"]);
    assert_eq!(a.tel.sent("answer"), vec!["answer 2"]);
}

#[test]
fn recents_missed_and_call_back() {
    let mut tel = FakeTel::new("+12145550100");
    tel.log.push(LogLine { id: 1, dir: "out".into(), number: "+12145550142".into(), start_ms: 0, seconds: 75, reason: "-".into() });
    tel.log.push(LogLine { id: 2, dir: "missed".into(), number: "+12145550143".into(), start_ms: 1000, seconds: 0, reason: "no answer".into() });
    // telephonyd's clock (status now): the missed call was 50 minutes ago
    tel.now_ms = 3_001_000;
    let (mut a, m) = start(tel, 640, 400, &[]);
    assert!(sent_text(&m).contains("Recents (1)"), "the missed badge");
    assert_eq!(a.model.borrow().now_ms, 3_001_000);
    // tapping the missed-call notification opens Recents
    m.push(&[Message::NotificationAction(proto::NotificationAction { tag: "call:missed".into(), action: String::new() })]);
    turn(&mut a, 100);
    assert_eq!(a.model.borrow().tab, Tab::Recents);
    let t = sent_text(&m);
    assert!(t.contains("Missed") && t.contains("Outgoing · 1:15") && t.contains("Bo Jones") && t.contains("50 min"), "{t}");
    assert_eq!(a.model.borrow().log[0].dir, "missed", "newest first");
    click_row(&mut a, &m, "recents", 0, 200);
    assert_eq!(a.tel.sent("dial"), vec!["dial +12145550143"]);
}

#[test]
fn favourites_and_contacts_dial() {
    let (mut a, m) = start(FakeTel::new("+12145550100"), 320, 480, &[]);
    assert!(sent_text(&m).contains("Alex Smith"));
    click_row(&mut a, &m, "favourites", 0, 100);
    assert_eq!(a.tel.sent("dial"), vec!["dial +12145550142"]);
    a.tel.calls.clear();
    a.model.borrow_mut().tab = Tab::Contacts;
    turn(&mut a, 1500);
    click_row(&mut a, &m, "contacts", 1, 1600);
    assert_eq!(a.model.borrow().contact.as_deref().and_then(|u| a.model.borrow().book.by_uid(u).map(|c| c.name.clone())), Some("Bo Jones".into()));
    assert!(sent_text(&m).contains("Work"));
    click_row(&mut a, &m, "numbers", 1, 1700);
    assert_eq!(a.tel.sent("dial").last().unwrap(), "dial +12145550199");
}

#[test]
fn voicemail_play_and_delete() {
    let mut tel = FakeTel::new("+12145550100");
    tel.voicemail = vec!["vm 3 +12145550143 0 5400 new".into(), "vm 2 +12145550177 0 2000 heard".into()];
    let (mut a, m) = start(tel, 320, 480, &[]);
    assert!(sent_text(&m).contains("VM (1)"));
    m.push(&[Message::NotificationAction(proto::NotificationAction { tag: "vm".into(), action: String::new() })]);
    turn(&mut a, 100);
    assert_eq!(a.model.borrow().tab, Tab::Voicemail);
    click_row(&mut a, &m, "voicemail", 0, 200);
    click(&mut a, &m, "vm_play", 300);
    turn(&mut a, 310);
    assert_eq!(a.tel.sent("voicemail-fetch"), vec!["voicemail-fetch 3 /tmp/vm-3.wav"]);
    assert_eq!(a.tel.sent("voicemail-heard"), vec!["voicemail-heard 3"]);
    let t = format!("{:?}", m.sent_messages());
    assert!(t.contains("Open") && t.contains("/tmp/vm-3.wav") && t.contains("Playing the voicemail from Bo Jones"), "{t}");
    click(&mut a, &m, "vm_delete", 400);
    assert_eq!(a.tel.sent("voicemail-delete"), vec!["voicemail-delete 3"]);
}

#[test]
fn number_argument_and_errors() {
    let (mut a, m) = start(FakeTel::new("+12145550100"), 320, 480, &["--number", "(214) 555-0144"]);
    assert_eq!(a.model.borrow().tab, Tab::Keypad);
    assert!(sent_text(&m).contains("Cy"));
    // control: an empty keypad with no history refuses
    a.model.borrow_mut().typed.clear();
    click(&mut a, &m, "call", 100);
    assert!(a.tel.sent("dial").is_empty());
    assert!(sent_text(&m).contains("Type a number first"));
    let _ = hit(&a.app, "tabs");
}

#[test]
fn every_tab_at_every_size() {
    for (w, h) in [(320, 480), (640, 400), (1024, 640)] {
        let mut tel = FakeTel::new("+12145550100");
        tel.log.push(LogLine { id: 2, dir: "missed".into(), number: "+12145550143".into(), start_ms: 0, seconds: 0, reason: "-".into() });
        tel.voicemail = vec!["vm 3 +12145550143 0 5400 new".into()];
        let (mut a, m) = start(tel, w, h, &[]);
        for t in Tab::ALL {
            a.model.borrow_mut().tab = t;
            turn(&mut a, 2000);
        }
        let s = sent_text(&m);
        for want in ["Alex Smith", "Bo Jones", "Missed", "Delete", "0:06"] {
            assert!(s.contains(want), "{w}: {want} missing");
        }
    }
}

/// The big buttons are laid out inside a 320x480 phone.
#[test]
fn screens_fit_a_phone() {
    let inside = |a: &A, id: &str| {
        let r = a.app.core.window(1).unwrap().rect_of(id).unwrap_or_else(|| panic!("no {id}"));
        assert!(r.x >= 0 && r.y >= 0 && r.x + r.w <= 320 && r.y + r.h <= 480 && r.w > 20, "{id} at {r:?}");
        r
    };
    let mut tel = FakeTel::new("+12145550100");
    tel.calls.push(call(5, "incoming", "+12145550143", 0));
    let (mut a, _m) = start(tel, 320, 480, &[]);
    let (d, an) = (inside(&a, "decline"), inside(&a, "answer"));
    assert!(d.x + d.w <= an.x, "Decline left of Answer");
    inside(&a, "message_reply");
    a.tel.calls[0].state = "active".into();
    a.tel.calls[0].seconds = 42;
    a.model.borrow_mut().incoming = None;
    a.model.borrow_mut().dismissed.push(5);
    turn(&mut a, 2000);
    for id in ["mute", "keypad", "speaker", "add", "hold", "end"] {
        inside(&a, id);
    }
    a.tel.calls.clear();
    a.model.borrow_mut().tab = Tab::Keypad;
    turn(&mut a, 4000);
    for id in ["key_1", "key_hash", "del", "call"] {
        inside(&a, id);
    }
}
