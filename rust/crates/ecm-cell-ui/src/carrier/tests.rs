//! The Carrier Console on a scripted UI connection, over the real `cored` command handling.

use super::*;
use crate::fake::{busy_core, CoreAdmin, Scripted};
use crate::testkit::{configure, mock, push_click, push_text, sent_text};
use cored::daemon::Card;
use ecm_cell::codec::hex;
use ecm_cell::db::Db;
use ecm_ui::MockConnection;

pub fn db() -> String {
    cored::daemon::init_db("cell-red", "100.65.2.0/24", None, None).unwrap()
}

fn start<A: Admin>(admin: A, w: i32, h: i32) -> (ConsoleApp<A>, MockConnection) {
    let m = mock(w, h);
    let mut a = ConsoleApp::new(Box::new(m.clone()), admin).unwrap();
    configure(&m, w, h);
    a.step(0, 0);
    a.step(0, 10);
    (a, m)
}

fn turn<A: Admin>(a: &mut ConsoleApp<A>, t: u64) {
    a.step(0, t);
    a.step(0, t + 1);
}

#[test]
fn not_running() {
    let (_a, m) = start(Scripted { down: true, ..Default::default() }, 640, 400);
    assert!(sent_text(&m).contains("cored is not running"));
}

#[test]
fn write_a_sim_from_the_console() {
    let (mut a, m) = start(CoreAdmin::new(&db()), 640, 400);
    assert_eq!(a.model.borrow().daemon, Daemon::Running);
    assert!(sent_text(&m).contains("No subscribers yet"));
    // control: no card in the reader
    push_click(&a.app, &m, "write_sim");
    turn(&mut a, 100);
    let msg = a.model.borrow().message.clone();
    assert!(matches!(&msg, Some((false, e)) if e.contains("no card in the Card Reader/Writer")), "{msg:?}");
    assert!(a.admin.reader.written.is_empty());
    // a blank SIM in the reader, a label, Write SIM
    a.admin.reader.card = Some(Card::Blank);
    push_click(&a.app, &m, "new_label");
    turn(&mut a, 200);
    push_text(&m, "alice");
    turn(&mut a, 300);
    assert_eq!(a.model.borrow().new_label, "alice");
    push_click(&a.app, &m, "write_sim");
    turn(&mut a, 400);
    let msg = a.model.borrow().message.clone().unwrap();
    assert!(msg.0 && msg.1.starts_with("Wrote SIM 00101") && msg.1.contains("(alice): take it out of the reader"), "{msg:?}");
    let (imsi, carrier, key) = a.admin.reader.written[0].clone();
    assert_eq!(carrier, "cell-red");
    let db = Db::parse(&a.admin.db_text).unwrap();
    assert_eq!(hex(&db.subs[&imsi].key), key, "the database (as saved) holds the card's key");
    assert_eq!(db.subs[&imsi].label, "alice");
    assert!(!sent_text(&m).contains(&key), "the key is never drawn");
    assert_eq!(a.model.borrow().subs.len(), 1);
    assert!(a.model.borrow().new_label.is_empty(), "the form is cleared");
    // choose it and revoke it
    a.model.borrow_mut().selected = Some(imsi.clone());
    turn(&mut a, 500);
    push_click(&a.app, &m, "revoke");
    turn(&mut a, 600);
    assert!(Db::parse(&a.admin.db_text).unwrap().subs[&imsi].revoked);
    assert!(a.model.borrow().subs[0].revoked);
    assert!(sent_text(&m).contains("Reinstate"));
}

#[test]
fn add_a_site() {
    let (mut a, m) = start(CoreAdmin::new(&db()), 1024, 640);
    a.model.borrow_mut().page = Page::Sites;
    turn(&mut a, 100);
    push_click(&a.app, &m, "add_site");
    turn(&mut a, 200);
    assert!(matches!(a.model.borrow().message.clone(), Some((false, _))), "no id: refused");
    push_click(&a.app, &m, "new_site");
    turn(&mut a, 300);
    push_text(&m, "site-b");
    turn(&mut a, 400);
    push_click(&a.app, &m, "add_site");
    turn(&mut a, 500);
    let (id, secret) = a.model.borrow().credential.clone().expect("a credential");
    assert_eq!(id, "site-b");
    assert_eq!(secret.len(), 32);
    assert!(Db::parse(&a.admin.db_text).unwrap().sites.contains_key("site-b"));
    let t = sent_text(&m);
    assert!(t.contains("celld-site config set site_id site-b secret") && t.contains("not connected"), "{t}");
}

#[test]
fn every_page_at_every_size() {
    for (w, h) in [(240, 400), (640, 400), (1024, 640)] {
        let (mut a, m) = start(busy_core(), w, h);
        for p in Page::ALL {
            a.model.borrow_mut().page = p;
            turn(&mut a, 2000);
        }
        let t = sent_text(&m);
        for want in ["alice", "lost-phone", "site-b", "waiting for the site to connect", "handover 001010000000417", "100.65.2.0/24", "1/2 sites"] {
            assert!(t.contains(want), "{w}: {want} missing");
        }
    }
}
