//! The Cell Site app on a scripted UI connection.

use super::*;
use crate::fake::{busy_site, Scripted, SiteAdmin, SITE_CONF as CONF};
use crate::testkit::{configure, mock, push_click, push_text, sent_text};
use ecm_ui::MockConnection;


fn start<A: Admin>(admin: A, w: i32, h: i32) -> (SiteApp<A>, MockConnection) {
    let m = mock(w, h);
    let mut a = SiteApp::new(Box::new(m.clone()), admin).unwrap();
    configure(&m, w, h);
    a.step(0, 0);
    a.step(0, 10);
    (a, m)
}

fn turn<A: Admin>(a: &mut SiteApp<A>, t: u64) {
    a.step(0, t);
    a.step(0, t + 1);
}

#[test]
fn not_running_says_how_to_start_it() {
    let (a, m) = start(Scripted { down: true, ..Default::default() }, 640, 400);
    assert_eq!(a.model.borrow().daemon, Daemon::Down("not running".into()));
    let t = sent_text(&m);
    assert!(t.contains("celld-site is not running") && t.contains("celld-site start &"), "{t}");
}

#[test]
fn shows_the_running_site_and_saves_settings() {
    let (mut a, m) = start(SiteAdmin::new(CONF), 640, 400);
    {
        let st = a.model.borrow();
        assert_eq!(st.daemon, Daemon::Running);
        assert_eq!(st.status.get("site"), "site-a");
        assert!(st.loaded);
        assert_eq!(st.form["channel"], "3");
    }
    assert!(sent_text(&m).contains("Core link down"));
    a.model.borrow_mut().page = Page::Settings;
    turn(&mut a, 20);
    push_click(&a.app, &m, "f_channel");
    turn(&mut a, 30);
    push_text(&m, "5");
    turn(&mut a, 40);
    let typed = a.model.borrow().form["channel"].clone();
    assert!(typed.len() == 2 && typed.contains('5') && typed.contains('3'), "typing reaches the form: {typed}");
    a.model.borrow_mut().form.insert("channel".into(), "5".into());
    push_click(&a.app, &m, "save");
    turn(&mut a, 50);
    assert!(a.admin.file.contains("channel = 5"), "{}", a.admin.file);
    let msg = a.model.borrow().message.clone();
    assert_eq!(msg, Some((true, "Saved channel. Restart the site to apply.".into())));
    // a bad value is refused by the daemon, and said so
    a.model.borrow_mut().form.insert("channel".into(), "9".into());
    push_click(&a.app, &m, "save");
    turn(&mut a, 60);
    assert!(matches!(a.model.borrow().message.clone(), Some((false, e)) if e.contains("bad channel")));
    assert!(a.admin.file.contains("channel = 5"));
    push_click(&a.app, &m, "restart");
    turn(&mut a, 70);
    assert_eq!(a.model.borrow().message, Some((true, "Restarting the site with the saved settings".into())));
}

#[test]
fn phones_and_every_size() {
    for (w, h) in [(240, 400), (640, 400), (1024, 640)] {
        let (mut a, m) = start(busy_site(), w, h);
        a.model.borrow_mut().page = Page::Phones;
        turn(&mut a, 2000);
        let t = sent_text(&m);
        assert!(t.contains("001010000000417") && t.contains("CQI 11"), "{w}: phones missing");
        assert!(t.contains("Core link up") && t.contains("2 phones"), "{w}");
        assert!(t.contains("cell 2"), "{w}: neighbour");
    }
}
