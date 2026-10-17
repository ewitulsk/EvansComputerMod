//! Messages on a scripted UI connection over a scripted telephonyd.

use super::*;
use crate::fake::{FakeTel, MemFiles};
use crate::testkit::{configure, mock, push_click, push_text, sent_text};
use ecm_tel::vcard::Contact;
use ecm_ui::MockConnection;

const ME: &str = "+12145550100";
const ALEX: &str = "+12145550142";
const BO: &str = "+12145550143";
const CY: &str = "+12145550144";

/// A small PNG (stored deflate) of `w x h` in one colour.
pub fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    fn crc(d: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in d {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc(&c).to_be_bytes());
    }
    let mut raw = Vec::new();
    for _ in 0..h {
        raw.push(0);
        for _ in 0..w {
            raw.extend_from_slice(&rgb);
        }
    }
    let mut z = vec![0x78, 0x01];
    for (i, blk) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(blk.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(blk.len() as u16)).to_le_bytes());
        z.extend_from_slice(blk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn book_files() -> MemFiles {
    let mut f = MemFiles::default();
    let mut b = Book::default();
    b.put(Contact::new("Alex Smith", ALEX));
    b.put(Contact::new("Bo Jones", BO));
    b.save(&mut f).unwrap();
    f
}

fn msg(id: u64, incoming: bool, from: &str, t: u64, status: &str, text: &str) -> MsgLine {
    MsgLine { id, incoming, from: from.into(), time_ms: t, status: status.into(), text: text.into(), ..Default::default() }
}

fn tel() -> FakeTel {
    let mut t = FakeTel::new(ME);
    t.now_ms = 600_000;
    t.threads = vec![
        ThreadLine { key: ALEX.into(), unread: 2, time_ms: 590_000, preview: "are you coming?".into() },
        ThreadLine { key: format!("{ALEX},{BO},{CY}"), unread: 0, time_ms: 300_000, preview: "pizza at 8".into() },
        ThreadLine { key: "+12145550199".into(), unread: 0, time_ms: 10_000, preview: "Your code is 4411".into() },
    ];
    t.messages.insert(
        ALEX.into(),
        vec![
            msg(1, true, ALEX, 100_000, "read", "hey, still on for tonight?"),
            msg(2, false, ME, 200_000, "delivered", "yes! 8pm"),
            msg(3, false, ME, 400_000, "sent", "bringing snacks"),
            msg(4, false, ME, 500_000, "failed:the phone is off", "see you there"),
            msg(5, true, ALEX, 590_000, "received", "are you coming?"),
        ],
    );
    t
}

type A = MessagesApp<FakeTel, MemFiles>;

fn start(t: FakeTel, f: MemFiles, w: i32, h: i32) -> (A, MockConnection) {
    let m = mock(w, h);
    let mut a = MessagesApp::new(Box::new(m.clone()), t, f).unwrap();
    configure(&m, w, h);
    a.step(0, 600_000);
    a.step(0, 600_010);
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

fn type_in(a: &mut A, m: &MockConnection, id: &str, text: &str, t: u64) {
    click(a, m, id, t);
    push_text(m, text);
    turn(a, t + 5);
}

#[test]
fn not_running() {
    let mut t = FakeTel::new(ME);
    t.down = true;
    let (_a, m) = start(t, MemFiles::default(), 320, 480);
    assert!(sent_text(&m).contains("telephonyd is not running"));
}

#[test]
fn the_list_names_threads_and_counts_unread() {
    let (a, m) = start(tel(), book_files(), 320, 480);
    assert_eq!(a.model.borrow().daemon, Daemon::Running);
    let t = sent_text(&m);
    for want in ["Alex Smith", "Alex, Bo +1", "(214) 555-0199", "are you coming?", "5 min", "\"2\"", "My number (214) 555-0100", "AS"] {
        assert!(t.contains(want), "{want} missing");
    }
    assert!(a.tel.requests.iter().all(|(p, _)| *p == SMS_PORT), "texts only: {:?}", a.tel.requests);
    assert_eq!(thread_initials(&a.model.borrow().book, "+12145550199"), "#");
    assert_eq!(thread_initials(&a.model.borrow().book, &format!("{ALEX},{BO}")), "AB");
}

#[test]
fn opening_a_thread_reads_it_and_shows_each_status() {
    let (mut a, m) = start(tel(), book_files(), 640, 400);
    click(&mut a, &m, "threads", 601_000);
    assert_eq!(a.model.borrow().key.as_deref(), Some(ALEX));
    assert_eq!(a.model.borrow().screen, Screen::Thread);
    assert!(a.tel.sent("read ").contains(&format!("read {ALEX}")));
    assert!(a.tel.sent("messages ").contains(&format!("messages {ALEX} 200")));
    assert_eq!(a.model.borrow().msgs.len(), 5);
    let t = sent_text(&m);
    for want in ["hey, still on for tonight?", "Delivered", "Sent", "Failed: the phone is off", "Retry"] {
        assert!(t.contains(want), "{want} missing");
    }
    // a status event updates the bubble
    a.tel.push_event("delivered", &[("id", "3")]);
    a.tel.messages.get_mut(ALEX).unwrap()[2].status = "delivered".into();
    turn(&mut a, 601_100);
    assert_eq!(a.model.borrow().msgs[2].status, "delivered");
}

#[test]
fn reply_and_retry() {
    let (mut a, m) = start(tel(), book_files(), 320, 480);
    a.model.borrow_mut().open_thread(ALEX, false);
    turn(&mut a, 601_000);
    type_in(&mut a, &m, "draft", "on my way", 601_100);
    assert_eq!(a.model.borrow().draft, "on my way");
    click(&mut a, &m, "send", 601_200);
    assert_eq!(a.tel.sent("sms "), vec![format!("sms {ALEX} on%20my%20way")]);
    assert!(a.model.borrow().draft.is_empty(), "the draft is cleared once telephonyd has it");
    assert!(a.model.borrow().msgs.iter().any(|m| m.text == "on my way" && m.status == "sending"));
    assert!(sent_text(&m).contains("Sending"));
    // Retry the failed one: its copy goes, it is sent again
    click(&mut a, &m, "retry_4", 601_300);
    assert_eq!(a.tel.sent("delete "), vec!["delete 4".to_string()]);
    assert_eq!(a.tel.sent("sms ").last().unwrap(), &format!("sms {ALEX} see%20you%20there"));
}

#[test]
fn compose_counts_parts_and_sends_a_group_as_mms() {
    assert_eq!(counter("hi", 1, 0), "2 characters · 1 part · 158 left");
    assert_eq!(counter(&"a".repeat(170), 1, 0), "170 characters · 2 parts · 136 left");
    assert!(counter("héllo ☺", 1, 0).contains("Unicode"));
    assert_eq!(counter("hi", 2, 0), "2 characters · MMS");
    assert_eq!(send_request(&[ALEX.into()], "a b", &[]).unwrap(), format!("sms {ALEX} a%20b"));
    assert_eq!(send_request(&[ALEX.into(), BO.into()], "", &["/home/Pictures/a b.png".into()]).unwrap(), format!("mms {ALEX},{BO} - /home/Pictures/a%20b.png"));
    assert!(send_request(&[], "x", &[]).is_none());

    let (mut a, m) = start(tel(), book_files(), 320, 480);
    click(&mut a, &m, "new", 601_000);
    assert_eq!(a.model.borrow().screen, Screen::Compose);
    // a contact by name from the suggestions
    type_in(&mut a, &m, "to_field", "bo", 601_100);
    assert!(sent_text(&m).contains("Bo Jones"));
    click(&mut a, &m, "suggest", 601_200);
    assert_eq!(a.model.borrow().to, vec![BO.to_string()]);
    // and a typed number
    type_in(&mut a, &m, "to_field", "(214) 555-0144", 601_300);
    click(&mut a, &m, "add_to", 601_400);
    assert_eq!(a.model.borrow().to.len(), 2);
    type_in(&mut a, &m, "compose_text", &"x".repeat(170), 601_500);
    assert!(sent_text(&m).contains("170 characters · MMS"));
    assert!(sent_text(&m).matches("\"xxxxxxxxxx").count() >= 3, "the whole text shows wrapped above the one-line field");
    click(&mut a, &m, "send", 601_600);
    assert_eq!(a.tel.sent("mms "), vec![format!("mms {BO},{CY} {} -", "x".repeat(170))]);
    assert_eq!(a.model.borrow().key.as_deref(), Some(format!("{BO},{CY}").as_str()), "the group thread opens");
    assert!(a.tel.threads.iter().any(|t| t.key == format!("{BO},{CY}")), "the fake keyed it like telephonyd");
    assert_eq!(a.model.borrow().screen, Screen::Thread);
}

#[test]
fn one_recipient_text_only_is_an_sms_with_two_parts() {
    let (mut a, m) = start(tel(), book_files(), 320, 480);
    a.model.borrow_mut().apply_args(&["--to".into(), CY.into(), "--text".into(), "y".repeat(161)]);
    turn(&mut a, 601_000);
    assert!(sent_text(&m).contains("161 characters · 2 parts"));
    click(&mut a, &m, "send", 601_100);
    assert_eq!(a.tel.sent("sms "), vec![format!("sms {CY} {}", "y".repeat(161))]);
    assert_eq!(a.model.borrow().key.as_deref(), Some(CY));
    // nobody to send to
    a.model.borrow_mut().new_message();
    a.model.borrow_mut().text = "hi".into();
    a.model.borrow_mut().send_compose();
    assert!(matches!(a.model.borrow().message.clone(), Some((false, t)) if t.contains("who")));
}

#[test]
fn pictures_voice_notes_and_cards() {
    let mut t = tel();
    let k = format!("{ALEX},{BO},{CY}");
    let card = Contact::new("Cy Young", CY).to_vcard();
    t.messages.insert(
        k.clone(),
        vec![
            MsgLine { id: 10, incoming: true, from: BO.into(), time_ms: 100_000, status: "read".into(), mms: true, text: "look".into(), files: vec!["/home/Messages/1-10/p.png".into()] },
            MsgLine { id: 11, incoming: true, from: ALEX.into(), time_ms: 200_000, status: "read".into(), mms: true, text: String::new(), files: vec!["/home/Messages/2-11/v.wav".into(), "/home/Messages/2-11/cy.vcf".into()] },
        ],
    );
    let mut f = book_files();
    f.files.insert("/home/Messages/1-10/p.png".into(), png(64, 48, [200, 80, 40]));
    f.files.insert("/home/Messages/2-11/cy.vcf".into(), card.into_bytes());
    f.files.insert("/home/Pictures/p.png".into(), b"already there".to_vec());
    let (mut a, m) = start(t, f, 320, 480);
    a.model.borrow_mut().open_thread(&k, false);
    turn(&mut a, 601_000);
    assert_eq!(png_size(&png(64, 48, [0, 0, 0])), Some((64, 48)));
    assert_eq!(m.state.borrow().uploads.len(), 1, "the picture is uploaded once");
    assert!(matches!(a.model.borrow().parts.get("/home/Messages/1-10/p.png"), Some(Part::Picture { w: 64, h: 48, .. })));
    let s = sent_text(&m);
    for want in ["Bo Jones", "Alex Smith", "look", "Voice note", "Contact card: Cy Young", "Add to Contacts", "View"] {
        assert!(s.contains(want), "{want} missing");
    }
    turn(&mut a, 602_000);
    assert_eq!(m.state.borrow().uploads.len(), 1, "not again on the next poll");
    // the card goes into Contacts
    click(&mut a, &m, "add_cy.vcf", 602_100);
    assert!(Book::load(&mut a.files).find(CY).is_some_and(|c| c.name == "Cy Young"));
    assert!(sent_text(&m).contains("Added Cy Young to Contacts"));
    // the voice note plays in its app
    click(&mut a, &m, "play_v.wav", 602_200);
    assert!(sent_text(&m).contains("/home/Messages/2-11/v.wav"));
    // the picture full window, saved to Files next to the one already there
    click(&mut a, &m, "view_p.png", 602_300);
    assert_eq!(a.model.borrow().screen, Screen::Viewer("/home/Messages/1-10/p.png".into()));
    click(&mut a, &m, "save_picture", 602_400);
    assert_eq!(a.files.files.get("/home/Pictures/p-1.png"), Some(&png(64, 48, [200, 80, 40])));
    assert!(sent_text(&m).contains("Saved to /home/Pictures/p-1.png"));
    click(&mut a, &m, "back", 602_500);
    assert_eq!(a.model.borrow().screen, Screen::Thread);
}

#[test]
fn attachments_from_files_contacts_and_the_microphone() {
    struct Mic(bool, usize);
    impl Recorder for Mic {
        fn start(&mut self) -> Result<(), String> {
            self.0 = true;
            Ok(())
        }
        fn pump(&mut self) {
            self.1 += 1;
        }
        fn stop(&mut self) -> Vec<u8> {
            self.0 = false;
            let mut w = b"RIFF".to_vec();
            w.resize(144, 1);
            w
        }
    }
    let m = mock(320, 480);
    let mut a = MessagesApp::with_recorder(Box::new(m.clone()), tel(), book_files(), Mic(false, 0)).unwrap();
    configure(&m, 320, 480);
    a.step(0, 0);
    a.step(0, 1);
    a.model.borrow_mut().apply_args(&["--to".into(), ALEX.into()]);
    a.step(0, 10);
    a.step(0, 11);
    // a picture from the file picker
    push_click(&a.app, &m, "attach_picture");
    a.step(0, 20);
    a.step(0, 21);
    assert!(sent_text(&m).contains("PickFile"));
    let req = a.model.borrow().picking.unwrap();
    m.push(&[Message::FilePicked(proto::FilePicked { req, path: "/home/Pictures/cat.png".into() })]);
    a.step(0, 30);
    a.step(0, 31);
    assert_eq!(a.model.borrow().attach, vec!["/home/Pictures/cat.png".to_string()]);
    // a contact card
    push_click(&a.app, &m, "attach_contact");
    a.step(0, 40);
    a.step(0, 41);
    assert_eq!(a.model.borrow().screen, Screen::ShareContact);
    push_click(&a.app, &m, "share");
    a.step(0, 50);
    a.step(0, 51);
    assert_eq!(a.model.borrow().screen, Screen::Compose);
    let vcf = "/home/Messages/out/Alex_Smith.vcf";
    assert!(String::from_utf8_lossy(a.files.files.get(vcf).expect("the card is written")).contains("FN:Alex Smith"));
    assert!(sent_text(&m).contains("Contact card: Alex Smith"));
    // a voice note
    push_click(&a.app, &m, "record");
    a.step(0, 60);
    a.step(0, 61);
    assert!(a.mic.0 && a.model.borrow().recording.is_some());
    a.step(0, 3_000);
    a.step(0, 3_001);
    assert!(sent_text(&m).contains("Stop 0:02"));
    push_click(&a.app, &m, "record");
    a.step(0, 4_000);
    a.step(0, 4_001);
    assert!(!a.mic.0 && a.mic.1 > 0);
    let wav = "/home/Messages/out/voice-4000.wav";
    assert!(a.files.files.contains_key(wav));
    assert_eq!(a.model.borrow().attach.len(), 3);
    assert!(sent_text(&m).contains("3 attachments"));
    push_click(&a.app, &m, "send");
    a.step(0, 5_000);
    a.step(0, 5_001);
    assert_eq!(a.tel.sent("mms "), vec![format!("mms {ALEX} - /home/Pictures/cat.png,{vcf},{wav}")]);
    // telephonyd has it in the thread, which is now open, with its parts
    let sent = a.tel.messages.get(ALEX).unwrap().last().cloned().unwrap();
    assert!(sent.mms && sent.files == vec!["/home/Pictures/cat.png".to_string(), vcf.to_string(), wav.to_string()]);
    assert_eq!(a.model.borrow().key.as_deref(), Some(ALEX));
    assert!(a.model.borrow().msgs.iter().any(|m| m.id == sent.id));
    assert!(sent_text(&m).contains("Voice note"));
}

#[test]
fn no_microphone_says_so() {
    let (mut a, m) = start(tel(), book_files(), 320, 480);
    a.model.borrow_mut().new_message();
    turn(&mut a, 601_000);
    click(&mut a, &m, "record", 601_100);
    assert!(matches!(a.model.borrow().message.clone(), Some((false, t)) if t.contains("Attach a .wav from Files instead")));
    assert!(sent_text(&m).contains("Can't record"));
    assert!(a.model.borrow().recording.is_none());
}

#[test]
fn a_notification_opens_its_thread_with_a_reply_box() {
    let (mut a, m) = start(tel(), book_files(), 320, 480);
    m.push(&[Message::NotificationAction(proto::NotificationAction { tag: "sms:%2B12145550142".into(), action: String::new() })]);
    turn(&mut a, 601_000);
    assert_eq!(a.model.borrow().key.as_deref(), Some(ALEX));
    assert!(a.model.borrow().quick_reply);
    assert!(sent_text(&m).contains("Reply to Alex Smith"));
    assert!(a.tel.sent("read ").contains(&format!("read {ALEX}")));
    type_in(&mut a, &m, "quick_reply", "yes", 601_100);
    click(&mut a, &m, "quick_send", 601_200);
    assert_eq!(a.tel.sent("sms "), vec![format!("sms {ALEX} yes")]);
    assert!(!a.model.borrow().quick_reply);
    // the failed-message notification shows the list; a plain tag works too
    m.push(&[Message::NotificationAction(proto::NotificationAction { tag: "sms:failed".into(), action: String::new() })]);
    turn(&mut a, 601_300);
    assert_eq!(a.model.borrow().screen, Screen::List);
    m.push(&[Message::NotificationAction(proto::NotificationAction { tag: format!("sms:{BO}"), action: String::new() })]);
    turn(&mut a, 601_400);
    assert_eq!(a.model.borrow().key.as_deref(), Some(BO));
}

#[test]
fn incoming_text_while_open_is_read_and_shown() {
    let (mut a, m) = start(tel(), book_files(), 320, 480);
    a.model.borrow_mut().open_thread(ALEX, false);
    turn(&mut a, 601_000);
    let reads = a.tel.sent("read ").len();
    a.tel.messages.get_mut(ALEX).unwrap().push(msg(6, true, ALEX, 601_050, "received", "hello??"));
    a.tel.push_event("sms", &[("id", "6"), ("from", ALEX), ("thread", ALEX), ("text", "hello??")]);
    turn(&mut a, 601_100);
    turn(&mut a, 601_200);
    assert!(sent_text(&m).contains("hello??"));
    assert!(a.tel.sent("read ").len() > reads, "read again while on screen");
}

#[test]
fn delete_a_thread_after_confirming() {
    let (mut a, m) = start(tel(), book_files(), 320, 480);
    a.model.borrow_mut().open_thread(ALEX, false);
    turn(&mut a, 601_000);
    click(&mut a, &m, "delete", 601_100);
    assert!(a.tel.sent("delete-thread").is_empty(), "not before confirming");
    assert!(sent_text(&m).contains("Delete this conversation?"));
    click(&mut a, &m, "confirm_delete", 601_200);
    assert_eq!(a.tel.sent("delete-thread"), vec![format!("delete-thread {ALEX}")]);
    assert_eq!(a.model.borrow().screen, Screen::List);
}

#[test]
fn args() {
    let mut md = Model::default();
    md.apply_args(&["--thread".into(), ALEX.into(), "--text".into(), "hi".into()]);
    assert_eq!((md.screen.clone(), md.key.as_deref(), md.draft.as_str()), (Screen::Thread, Some(ALEX), "hi"));
    let mut md = Model::default();
    md.apply_args(&["--to".into(), format!("{ALEX},{BO}")]);
    assert_eq!(md.screen, Screen::Compose);
    assert_eq!(md.to, vec![ALEX.to_string(), BO.to_string()]);
    let mut mc = Model::default();
    mc.apply_args(&["--vcard".into(), "/home/.contacts/share/Bo_Jones.vcf".into()]);
    assert_eq!((mc.screen.clone(), mc.attach.clone()), (Screen::Compose, vec!["/home/.contacts/share/Bo_Jones.vcf".to_string()]));
    md.msisdn = ME.into();
    assert_eq!(md.key_for(&["(214) 555-0143".into(), ALEX.into(), ME.into()]), format!("{ALEX},{BO}"));
    assert_eq!(md.normalize("5550143"), BO);
}

#[test]
fn every_screen_at_every_size() {
    for (w, h) in [(320, 480), (640, 400), (1024, 640)] {
        let (mut a, m) = start(tel(), book_files(), w, h);
        a.model.borrow_mut().open_thread(ALEX, false);
        turn(&mut a, 601_000);
        a.model.borrow_mut().new_message();
        turn(&mut a, 601_100);
        let t = sent_text(&m);
        for want in ["Alex Smith", "Delivered", "New message", "0 characters"] {
            assert!(t.contains(want), "{w}: {want} missing");
        }
    }
    // wide: the list beside the thread
    let (mut a, m) = start(tel(), book_files(), 1024, 640);
    a.model.borrow_mut().open_thread(ALEX, false);
    turn(&mut a, 601_000);
    let t = sent_text(&m);
    assert!(t.contains("Alex, Bo +1") && t.contains("bringing snacks"));
    let w = a.app.core.window(1).unwrap();
    assert!(w.hit_id_of("threads").is_some() && w.hit_id_of("draft").is_some(), "both panes");
}

#[test]
fn send_in_messages_from_contacts_attaches_the_card() {
    let mut f = book_files();
    let p = "/home/.contacts/share/Cy_Young.vcf";
    f.files.insert(p.into(), Contact::new("Cy Young", CY).to_vcard().into_bytes());
    let (mut a, m) = start(tel(), f, 320, 480);
    a.model.borrow_mut().apply_args(&["--vcard".into(), p.into()]);
    turn(&mut a, 601_000);
    assert!(sent_text(&m).contains("Contact card: Cy Young"));
    a.model.borrow_mut().to = vec![BO.into()];
    a.model.borrow_mut().send_compose();
    turn(&mut a, 601_100);
    assert_eq!(a.tel.sent("mms "), vec![format!("mms {BO} - {p}")]);
}
