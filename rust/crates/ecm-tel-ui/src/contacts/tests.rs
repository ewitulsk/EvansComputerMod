//! Contacts on a scripted UI connection, over an in-memory address book.

use super::*;
use crate::book::PATH;
use crate::fake::{FakeTel, MemFiles};
use crate::testkit::{configure, hit, mock, push_click, push_text, sent_text};
use ecm_ui::MockConnection;

type A = ContactsApp<FakeTel, MemFiles>;

fn start(tel: FakeTel, files: MemFiles, w: i32, h: i32) -> (A, MockConnection) {
    let m = mock(w, h);
    let mut a = ContactsApp::new(Box::new(m.clone()), tel, files).unwrap();
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

fn type_into(a: &mut A, m: &MockConnection, id: &str, text: &str, t: u64) {
    click(a, m, id, t);
    push_text(m, text);
    turn(a, t + 2);
}

fn book_text(a: &A) -> String {
    String::from_utf8(a.files.files.get(PATH).cloned().unwrap_or_default()).unwrap()
}

/// Launch messages the app sent: (app id, args).
fn launches(m: &MockConnection) -> Vec<(String, Vec<String>)> {
    m.sent_messages()
        .into_iter()
        .filter_map(|x| match x {
            Message::Launch(l) => Some((l.app_id, l.args)),
            _ => None,
        })
        .collect()
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    b
}

#[test]
fn the_list_has_my_card_the_contacts_and_the_favourite() {
    let (a, m) = start(FakeTel::new("+12145550199"), sample_files(), 320, 480);
    let t = sent_text(&m);
    for want in ["My card", "(214) 555-0199", "Alex Smith", "Bo Jones", "Cy Miller", "Dana Lee", "Mia Park", "(214) 555-0146"] {
        assert!(t.contains(want), "{want} missing");
    }
    for l in ["A", "B", "C", "D", "M"] {
        hit(&a.app, &format!("az_{l}"));
    }
    assert_eq!(a.tel.requests, vec![(ecm_tel::ctl::SMS_PORT, "status".to_string())], "only status, on the texts port");
    assert_eq!(a.model.borrow().visible().len(), 5);
    assert!(a.model.borrow().book.contacts[0].favourite);
}

#[test]
fn no_sim_or_no_telephonyd() {
    let (_a, m) = start(FakeTel { down: true, ..Default::default() }, sample_files(), 320, 480);
    assert!(sent_text(&m).contains("No number (insert a SIM)"));
    let (_a, m) = start(FakeTel::new(""), MemFiles::default(), 320, 480);
    let t = sent_text(&m);
    assert!(t.contains("No number (insert a SIM)") && t.contains("No contacts yet"), "{t}");
}

#[test]
fn search_and_the_letter_index() {
    let (mut a, m) = start(FakeTel::new("+12145550199"), sample_files(), 320, 480);
    type_into(&mut a, &m, "search", "555-0144", 100);
    assert_eq!(a.model.borrow().query, "555-0144");
    let names: Vec<String> = a.model.borrow().visible().iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, ["Cy Miller"]);
    a.model.borrow_mut().query.clear();
    a.model.borrow_mut().dirty = true;
    turn(&mut a, 200);
    click(&mut a, &m, "az_M", 300);
    let names: Vec<String> = a.model.borrow().visible().iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, ["Mia Park"]);
    assert!(sent_text(&m).contains("Names under M"));
    click(&mut a, &m, "az_M", 400);
    assert_eq!(a.model.borrow().visible().len(), 5, "the same letter again: all");
}

#[test]
fn a_contact_page_calls_and_texts_through_the_other_apps() {
    let (mut a, m) = start(FakeTel::new("+12145550199"), sample_files(), 640, 400);
    click(&mut a, &m, "contacts", 100); // the first row: Alex
    assert!(matches!(&a.model.borrow().screen, Screen::Contact(_)));
    let t = sent_text(&m);
    for want in ["Alex Smith", "mobile", "work", "(214) 555-0142", "(214) 555-0100", "alex@village.ecm", "Has the spare SIM cards", "Unfavourite", "Share", "Delete"] {
        assert!(t.contains(want), "{want} missing");
    }
    click(&mut a, &m, "call_1", 200);
    click(&mut a, &m, "text_0", 300);
    assert_eq!(
        launches(&m),
        vec![(DIALER_ID.to_string(), vec!["--call".to_string(), "+12145550100".into()]), (MESSAGES_ID.to_string(), vec!["--to".to_string(), "+12145550142".into()])]
    );
    assert!(a.tel.requests.iter().all(|(p, r)| *p == ecm_tel::ctl::SMS_PORT && r == "status"), "no telephonyd calls of its own");
}

#[test]
fn favourite_share_and_delete() {
    let (mut a, m) = start(FakeTel::new("+12145550199"), sample_files(), 320, 480);
    let bo = a.model.borrow().book.find("+12145550143").unwrap().uid.clone();
    a.model.borrow_mut().show(&bo);
    turn(&mut a, 100);
    click(&mut a, &m, "favourite", 200);
    assert!(a.model.borrow().book.by_uid(&bo).unwrap().favourite);
    assert!(Book::load(&mut a.files.clone()).find("+12145550143").unwrap().favourite, "saved");
    assert!(sent_text(&m).contains("Added to Favourites"));
    // share
    click(&mut a, &m, "share", 300);
    let card = String::from_utf8(a.files.files["/home/Documents/Bo Jones.vcf"].clone()).unwrap();
    assert!(card.contains("FN:Bo Jones") && card.contains("+12145550143"), "{card}");
    assert!(sent_text(&m).contains("Saved the card to /home/Documents/Bo Jones.vcf"));
    click(&mut a, &m, "send_vcard", 400);
    assert_eq!(launches(&m), vec![(MESSAGES_ID.to_string(), vec!["--vcard".to_string(), "/home/Documents/Bo Jones.vcf".into()])]);
    // delete asks first; Cancel keeps it
    click(&mut a, &m, "delete", 500);
    assert!(sent_text(&m).contains("Delete Bo Jones?"));
    click(&mut a, &m, "delete_cancel", 600);
    assert!(book_text(&a).contains("Bo Jones"));
    click(&mut a, &m, "delete", 700);
    click(&mut a, &m, "delete_yes", 800);
    assert!(!book_text(&a).contains("Bo Jones"));
    assert_eq!(a.model.borrow().screen, Screen::List);
    assert_eq!(a.model.borrow().book.contacts.len(), 4);
    assert!(sent_text(&m).contains("Deleted Bo Jones"));
}

#[test]
fn add_validates_and_saves_numbers_with_labels() {
    let (mut a, m) = start(FakeTel::new("+12145550199"), MemFiles::default(), 320, 480);
    click(&mut a, &m, "add", 100);
    assert_eq!(a.model.borrow().screen, Screen::Edit);
    assert!(sent_text(&m).contains("New contact"));
    click(&mut a, &m, "save", 200);
    assert_eq!(a.model.borrow().message, Some((false, "Give the contact a name or a number".into())));
    assert!(!a.files.files.contains_key(PATH), "nothing saved");
    type_into(&mut a, &m, "e_name", "Eve Stone", 300);
    type_into(&mut a, &m, "e_num_0", "2145550150", 400);
    click(&mut a, &m, "e_add_num", 500);
    type_into(&mut a, &m, "e_num_1", "+12145550151", 600);
    assert_eq!(a.model.borrow().draft.as_ref().unwrap().tels[1].label, "HOME");
    click(&mut a, &m, "e_label_1", 700);
    assert_eq!(a.model.borrow().draft.as_ref().unwrap().tels[1].label, "WORK");
    click(&mut a, &m, "e_add_num", 800);
    click(&mut a, &m, "e_remove_2", 900);
    type_into(&mut a, &m, "e_email", "eve@village.ecm", 1000);
    type_into(&mut a, &m, "e_note", "Runs the mine", 1100);
    click(&mut a, &m, "save", 1200);
    let b = Book::load(&mut a.files.clone());
    let eve = b.find("+12145550151").expect("saved");
    assert_eq!(eve.name, "Eve Stone");
    assert_eq!(eve.tels.iter().map(|t| (t.label.as_str(), t.number.as_str())).collect::<Vec<_>>(), [("CELL", "2145550150"), ("WORK", "+12145550151")]);
    assert_eq!(eve.emails, ["eve@village.ecm"]);
    assert_eq!(eve.note, "Runs the mine");
    assert!(matches!(&a.model.borrow().screen, Screen::Contact(u) if *u == eve.uid));
    assert!(sent_text(&m).contains("Saved Eve Stone"));
    // a number alone is enough
    a.model.borrow_mut().add("+12145550160");
    turn(&mut a, 1300);
    click(&mut a, &m, "save", 1400);
    assert!(Book::load(&mut a.files.clone()).find("+12145550160").is_some_and(|c| c.name == "(214) 555-0160"));
}

#[test]
fn edit_keeps_what_the_form_does_not_show_and_cancel_changes_nothing() {
    let (mut a, m) = start(FakeTel::new("+12145550199"), sample_files(), 320, 480);
    let alex = a.model.borrow().book.find("+12145550142").unwrap().clone();
    a.model.borrow_mut().show(&alex.uid);
    turn(&mut a, 100);
    click(&mut a, &m, "edit", 200);
    assert!(sent_text(&m).contains("Edit contact"));
    type_into(&mut a, &m, "e_name", "x", 300);
    click(&mut a, &m, "cancel", 400);
    assert_eq!(a.model.borrow().book.by_uid(&alex.uid).unwrap().name, "Alex Smith");
    assert_eq!(a.model.borrow().screen, Screen::Contact(alex.uid.clone()));
    click(&mut a, &m, "edit", 500);
    a.model.borrow_mut().draft.as_mut().unwrap().name = "Alex Smythe".into();
    click(&mut a, &m, "save", 600);
    let c = Book::load(&mut a.files.clone()).by_uid(&alex.uid).cloned().unwrap();
    assert_eq!(c.name, "Alex Smythe");
    assert!(c.favourite && c.tels.len() == 2 && c.note == alex.note, "the rest kept: {c:?}");
}

#[test]
fn a_photo_from_the_file_picker() {
    let mut f = sample_files();
    f.files.insert("/home/Pictures/bo.png".into(), png(64, 64));
    let (mut a, m) = start(FakeTel::new("+12145550199"), f, 320, 480);
    let bo = a.model.borrow().book.find("+12145550143").unwrap().uid.clone();
    a.model.borrow_mut().edit(&bo);
    turn(&mut a, 100);
    click(&mut a, &m, "photo", 200);
    let picks: Vec<proto::PickFile> = m.sent_messages().into_iter().filter_map(|x| if let Message::PickFile(p) = x { Some(p) } else { None }).collect();
    assert_eq!(picks.len(), 1);
    assert_eq!((picks[0].req, picks[0].types.clone()), (PICK_PHOTO, vec!["png".to_string()]));
    m.push(&[Message::FilePicked(proto::FilePicked { req: PICK_PHOTO, path: "/home/Pictures/bo.png".into() })]);
    turn(&mut a, 300);
    assert_eq!(a.model.borrow().draft.as_ref().unwrap().photo, "/home/Pictures/bo.png");
    assert!(matches!(a.model.borrow().photos.get("/home/Pictures/bo.png"), Some(Some(_))), "uploaded as an image");
    click(&mut a, &m, "save", 400);
    assert_eq!(Book::load(&mut a.files.clone()).by_uid(&bo).unwrap().photo, "/home/Pictures/bo.png");
    // a path that isn't a PNG falls back to the initials
    a.model.borrow_mut().edit(&bo);
    a.model.borrow_mut().draft.as_mut().unwrap().photo = "/home/notes.txt".into();
    turn(&mut a, 500);
    assert_eq!(a.model.borrow().photos.get("/home/notes.txt"), Some(&None));
}

const VCF: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Bo J.\r\nTEL;TYPE=CELL:+12145550143\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nFN:Zoe Hart\r\nTEL;TYPE=CELL:+12145550170\r\nEND:VCARD\r\n";

#[test]
fn import_a_vcf_by_open_file_and_by_the_import_button() {
    let mut f = sample_files();
    f.files.insert("/home/Downloads/friends.vcf".into(), VCF.as_bytes().to_vec());
    f.files.insert("/home/Downloads/bad.vcf".into(), b"BEGIN:VCARD\nFN:x\n".to_vec());
    let (mut a, m) = start(FakeTel::new("+12145550199"), f, 320, 480);
    m.push(&[Message::OpenFile(proto::OpenFile { path: "/home/Downloads/friends.vcf".into() })]);
    turn(&mut a, 100);
    assert_eq!(a.model.borrow().message, Some((true, "Imported 2 contacts: 1 added, 1 merged".into())));
    let b = Book::load(&mut a.files.clone());
    assert_eq!(b.contacts.len(), 6);
    assert_eq!(b.name_of("+12145550143"), "Bo J.");
    // the Import button asks the picker for a .vcf
    click(&mut a, &m, "import", 200);
    assert!(m.sent_messages().iter().any(|x| matches!(x, Message::PickFile(p) if p.req == PICK_IMPORT && p.types == ["vcf"])));
    m.push(&[Message::FilePicked(proto::FilePicked { req: PICK_IMPORT, path: "/home/Downloads/bad.vcf".into() })]);
    turn(&mut a, 300);
    assert!(matches!(&a.model.borrow().message, Some((false, e)) if e.contains("is not a vCard file")));
    m.push(&[Message::FilePicked(proto::FilePicked { req: PICK_IMPORT, path: String::new() })]);
    turn(&mut a, 400);
    assert_eq!(Book::load(&mut a.files.clone()).contacts.len(), 6, "a cancelled pick does nothing");
}

#[test]
fn program_arguments() {
    assert_eq!(Start::parse(&["--add".into(), "+12145550190".into()]), Start::Add("+12145550190".into()));
    assert_eq!(Start::parse(&["--show".into(), "2145550142".into()]), Start::Show("2145550142".into()));
    assert_eq!(Start::parse(&["/home/a.VCF".into()]), Start::Import("/home/a.VCF".into()));
    assert_eq!(Start::parse(&[]), Start::List);
    let (mut a, m) = start(FakeTel::new("+12145550199"), sample_files(), 320, 480);
    a.start(Start::Show("(214) 555-0142".into()));
    turn(&mut a, 100);
    assert!(matches!(a.model.borrow().current(), Some(c) if c.name == "Alex Smith"));
    a.start(Start::Show("+12145550190".into()));
    turn(&mut a, 200);
    assert_eq!(a.model.borrow().screen, Screen::Edit);
    assert_eq!(a.model.borrow().draft.as_ref().unwrap().tels[0].number, "+12145550190");
    assert!(sent_text(&m).contains("+12145550190"));
    a.start(Start::Add("611".into()));
    turn(&mut a, 300);
    assert_eq!(a.model.borrow().draft.as_ref().unwrap().tels[0].number, "611");
}

#[test]
fn another_app_changing_the_book_is_picked_up() {
    let (mut a, m) = start(FakeTel::new("+12145550199"), sample_files(), 320, 480);
    let mut b = Book::load(&mut a.files);
    b.put(Contact::new("Zed Quinn", "+12145550180"));
    b.save(&mut a.files).unwrap();
    turn(&mut a, 2000);
    assert!(sent_text(&m).contains("Zed Quinn"));
}

#[test]
fn every_screen_at_every_size() {
    for (w, h) in [(320, 480), (640, 400), (1024, 640)] {
        let (mut a, m) = start(FakeTel::new("+12145550199"), sample_files(), w, h);
        let alex = a.model.borrow().book.contacts[0].uid.clone();
        a.model.borrow_mut().show(&alex);
        turn(&mut a, 100);
        if w >= 800 {
            hit(&a.app, "contacts");
            hit(&a.app, "edit");
        }
        a.model.borrow_mut().edit(&alex);
        turn(&mut a, 200);
        let t = sent_text(&m);
        for want in ["Alex Smith", "Has the spare SIM cards", "Add number", "Save"] {
            assert!(t.contains(want), "{w}: {want} missing");
        }
    }
}

#[test]
fn helpers() {
    assert_eq!(label_name("CELL"), "mobile");
    assert_eq!(label_name("work"), "work");
    assert_eq!(next_label("CELL"), "HOME");
    assert_eq!(next_label("WORK"), "CELL");
    assert_eq!(next_label("OTHER"), "CELL");
    assert_eq!(vcf_name("Alex/Smith"), "Alex_Smith.vcf");
    assert_eq!(vcf_name("..."), "contact.vcf");
    assert_eq!(png_size(&png(40, 30)), Some((40, 30)));
    assert_eq!(png_size(b"GIF89a"), None);
}
