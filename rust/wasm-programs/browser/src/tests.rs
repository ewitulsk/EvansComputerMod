//! The Browser on a scripted UI connection and a scripted network/disk.

use crate::app::BrowserApp;
use crate::model::Nav;
use ecm_ui::{samples, MockConnection};
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use crate::fake::Net;

fn village(i: u32) -> String {
    let mut links = String::new();
    for v in 1..=3 {
        if v != i {
            links.push_str(&format!("<li><a href=\"http://100.{}.0.10/\">Tech Village {v}</a> (AS {})</li>", 64 + v, 65000 + v));
        }
    }
    format!("<html><head><title>Tech Village {i} - AS {}</title></head><body><h1>Tech Village {i}</h1><p>A village.</p><h2>Other villages</h2><ul>{links}</ul></body></html>", 65000 + i)
}

fn ring() -> Net {
    let mut n = Net::default();
    for i in 1..=3 {
        n = n.page(&format!("100.{}.0.10:80 /", 64 + i), "200 OK", "text/html", &village(i));
    }
    n.page("100.68.0.10:80 /nope.html", "404 Not Found", "text/plain", "404 Not Found")
}

fn start(net: Net, w: i32, h: i32, addr: Option<&str>) -> (BrowserApp<Net>, MockConnection) {
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    let mut wl = samples::welcome(false);
    if let Message::Welcome(ref mut x) = wl {
        x.display.w = w;
        x.display.h = h;
        x.display.size_class = proto::SizeClass::for_width(w);
    }
    mock.push(&[wl]);
    let mut b = BrowserApp::new(Box::new(mock.clone()), net, addr).unwrap();
    mock.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w, h, states: proto::window_states::ACTIVATED })]);
    for _ in 0..4 {
        b.step(0);
    }
    (b, mock)
}

fn steps(b: &mut BrowserApp<Net>) {
    for _ in 0..4 {
        b.step(0);
    }
}

/// Every text node the window sent so far, as one string.
fn sent_texts(mock: &MockConnection) -> String {
    format!("{:?}", mock.sent_messages())
}

fn title(b: &BrowserApp<Net>) -> String {
    b.app.core.window(b.window).unwrap().title.clone()
}

fn hit(b: &BrowserApp<Net>, id: &str) -> u32 {
    b.app.core.window(b.window).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no widget {id}"))
}

fn click(b: &mut BrowserApp<Net>, mock: &MockConnection, id: &str) {
    let h = hit(b, id);
    mock.push(&samples::click(h, 5, 5));
    steps(b);
}

fn type_address(b: &mut BrowserApp<Net>, mock: &MockConnection, text: &str) {
    click(b, mock, "url");
    mock.push(&[Message::Key(proto::Key { surface: 1, key: 0x04, mods: proto::mods::CTRL, down: true, repeat: false })]);
    mock.push(&[Message::Text(proto::Text { surface: 1, text: text.into() })]);
    mock.push(&[Message::Key(proto::Key { surface: 1, key: 0x28, mods: 0, down: true, repeat: false })]);
    steps(b);
}

/// Click the page where the text `t` is drawn (as the compositor would route it).
fn click_text(b: &mut BrowserApp<Net>, mock: &MockConnection, t: &str) {
    let (x, y) = {
        let m = b.model.borrow();
        let list = &m.cache.as_ref().unwrap().rendered.list;
        let pos = list.items.iter().find_map(|i| match i {
            ecm_web::paint::Item::Text { x, y, text, .. } if text.contains(t) => Some((*x + 3, *y + 3)),
            _ => None,
        });
        let (x, y) = pos.unwrap_or_else(|| panic!("no {t:?} on the page"));
        (x, y - m.scroll as i32)
    };
    let r = b.app.core.window(b.window).unwrap().rect_of("page").unwrap();
    let h = hit(b, "page");
    mock.push(&samples::click(h, r.x + x, r.y + y));
    steps(b);
}

fn page_text(b: &BrowserApp<Net>) -> String {
    let m = b.model.borrow();
    m.page.as_ref().map(|p| p.dom.text_content(0)).unwrap_or_default()
}

#[test]
fn start_page_then_typed_address_then_a_link_and_back() {
    let (mut b, mock) = start(ring(), 640, 400, None);
    assert_eq!(title(&b), "Start - Browser");
    assert!(page_text(&b).contains("Tech Village 10"));
    type_address(&mut b, &mock, "100.68.0.10");
    // Control: 100.68.0.10 isn't on this ring: an error page, no crash.
    assert!(title(&b).starts_with("Can't reach this page"), "{}", title(&b));
    assert!(page_text(&b).contains("100.68.0.10:80"));
    type_address(&mut b, &mock, "100.65.0.10");
    assert_eq!(title(&b), "Tech Village 1 - AS 65001 - Browser");
    assert!(sent_texts(&mock).contains("Tech Village 1"));
    assert_eq!(b.model.borrow().status, 200);
    click_text(&mut b, &mock, "Tech Village 2");
    assert_eq!(title(&b), "Tech Village 2 - AS 65002 - Browser");
    assert_eq!(b.fetcher.transport.log.last().unwrap(), "100.66.0.10:80 /");
    // The address bar follows.
    assert_eq!(b.app.core.window(1).unwrap().state("url").unwrap().text, "http://100.66.0.10/");
    click(&mut b, &mock, "back");
    assert_eq!(title(&b), "Tech Village 1 - AS 65001 - Browser");
    click(&mut b, &mock, "forward");
    assert_eq!(title(&b), "Tech Village 2 - AS 65002 - Browser");
    click(&mut b, &mock, "home");
    assert_eq!(title(&b), "Start - Browser");
    // The history: start, error, village 1, village 2, start.
    assert_eq!(b.model.borrow().history.len(), 5);
}

#[test]
fn server_404_page_is_shown() {
    let (b, mock) = start(ring(), 640, 400, Some("http://100.68.0.10/nope.html"));
    assert_eq!(b.model.borrow().status, 404);
    assert!(page_text(&b).contains("404 Not Found"));
    assert_eq!(title(&b), "404 Not Found - Browser");
    let _ = mock;
}

#[test]
fn phone_layout_has_a_bottom_bar() {
    let (b, _mock) = start(ring(), 240, 400, Some("100.65.0.10"));
    let w = b.app.core.window(1).unwrap();
    let url = w.rect_of("url").unwrap();
    let page = w.rect_of("page").unwrap();
    let back = w.rect_of("back").unwrap();
    assert!(url.y < page.y && page.y + page.h <= back.y, "address, page, then the buttons: {url:?} {page:?} {back:?}");
    assert!(page.w >= 236 && page.h > 300, "{page:?}");
    // The page fits the phone: nothing is drawn past the right edge.
    let m = b.model.borrow();
    let list = &m.cache.as_ref().unwrap().rendered.list;
    for i in &list.items {
        if let ecm_web::paint::Item::Text { x, text, size, .. } = i {
            let f = ecm_ui_proto::fonts::face(ecm_ui_proto::Font::Ui, *size).unwrap();
            assert!(x + f.text_width(text) <= 240, "{text:?}");
        }
    }
}

#[test]
fn scrolling_with_the_wheel_and_a_drag() {
    let mut long = String::from("<h1>Long</h1>");
    for i in 0..80 {
        long.push_str(&format!("<p>Paragraph {i}</p>"));
    }
    let net = Net::default().page("10.0.0.1:80 /", "200 OK", "text/html", &long);
    let (mut b, mock) = start(net, 640, 400, Some("10.0.0.1"));
    let h = hit(&b, "page");
    let wheel = |steps: i32| {
        Message::Pointer(proto::Pointer { surface: 1, kind: proto::PointerKind::Scroll, x: 100, y: 100, buttons: 0, button: proto::Button::None, scroll_x: 0, scroll_y: steps as i16, hit_id: h })
    };
    mock.push(&[wheel(3)]);
    steps(&mut b);
    let s1 = b.model.borrow().scroll;
    assert!(s1 > 0.0);
    // The scroll moves one group, and the scrollbar shows.
    let sent = sent_texts(&mock);
    assert!(sent.contains(&format!("Y({})", -(s1 as i32))), "the document group moved");
    // Can't scroll above the top.
    mock.push(&[wheel(-100)]);
    steps(&mut b);
    assert_eq!(b.model.borrow().scroll, 0.0);
    // A drag up scrolls down (a phone's finger), and is no click.
    let p = |kind, y| Message::Pointer(proto::Pointer { surface: 1, kind, x: 100, y, buttons: if kind == proto::PointerKind::Up { 0 } else { 1 }, button: proto::Button::Left, scroll_x: 0, scroll_y: 0, hit_id: h });
    mock.push(&[p(proto::PointerKind::Down, 300), p(proto::PointerKind::Move, 250), p(proto::PointerKind::Move, 200), p(proto::PointerKind::Up, 200)]);
    steps(&mut b);
    assert_eq!(b.model.borrow().scroll, 100.0);
    assert_eq!(b.model.borrow().loads, 1);
    // End and Home keys.
    b.model.borrow_mut().pending = None;
    let key = |k| Message::Key(proto::Key { surface: 1, key: k, mods: 0, down: true, repeat: false });
    mock.push(&[key(0x4D)]);
    steps(&mut b);
    let m = b.model.borrow().max_scroll();
    assert!(m > 100.0);
    assert_eq!(b.model.borrow().scroll, m);
}

#[test]
fn local_site_with_a_stylesheet_and_without() {
    let net = Net::default()
        .file("/home/site/index.html", "<html><head><title>My site</title><link rel=stylesheet href=style.css></head><body><nav class=bar><a href=a.html>A</a><a href=b.html>B</a></nav></body></html>")
        .file("/home/site/style.css", ".bar { display: flex; gap: 20px; background: #102030 }");
    let (b, _mock) = start(net.clone(), 640, 400, Some("file:///home/site/index.html"));
    assert_eq!(title(&b), "My site - Browser");
    {
        let m = b.model.borrow();
        let list = &m.cache.as_ref().unwrap().rendered.list;
        let xs: Vec<i32> = list.items.iter().filter_map(|i| match i { ecm_web::paint::Item::Text { x, text, .. } if text == "A" || text == "B" => Some(*x), _ => None }).collect();
        assert_eq!(xs.len(), 2);
        assert!(xs[1] - xs[0] >= 20, "flex gap between the links: {xs:?}");
        assert!(list.items.iter().any(|i| matches!(i, ecm_web::paint::Item::Rect { fill: 0x102030ff, .. })));
    }
    assert!(b.fetcher.transport.log.contains(&"file /home/site/style.css".to_string()));
    // Control: without the stylesheet the page still shows, unstyled.
    let mut net2 = net;
    net2.files.remove("/home/site/style.css");
    let (b, _mock) = start(net2, 640, 400, Some("/home/site/index.html"));
    assert_eq!(title(&b), "My site - Browser");
    let m = b.model.borrow();
    let list = &m.cache.as_ref().unwrap().rendered.list;
    assert!(!list.items.iter().any(|i| matches!(i, ecm_web::paint::Item::Rect { fill: 0x102030ff, .. })));
    assert!(page_text_m(&m).contains('A'));
}

fn page_text_m(m: &crate::model::Model) -> String {
    m.page.as_ref().map(|p| p.dom.text_content(0)).unwrap_or_default()
}

#[test]
fn folders_and_missing_files() {
    let net = Net::default().file("/home/notes.txt", "plain <text>").file("/home/a.html", "<h1>A</h1>");
    let (mut b, mock) = start(net, 640, 400, Some("file:///home/"));
    assert!(page_text(&b).contains("notes.txt") && page_text(&b).contains("a.html"));
    click_text(&mut b, &mock, "notes.txt");
    assert!(page_text(&b).contains("plain <text>"), "text files are shown as text");
    type_address(&mut b, &mock, "/home/missing.html");
    assert!(title(&b).starts_with("File not found"));
}

#[test]
fn anchors_scroll_within_the_page() {
    let mut html = String::from("<a href=#end>to the end</a>");
    for i in 0..60 {
        html.push_str(&format!("<p>p{i}</p>"));
    }
    html.push_str("<h2 id=end>The end</h2>");
    let net = Net::default().page("10.0.0.1:80 /", "200 OK", "text/html", &html);
    let (mut b, mock) = start(net, 640, 300, Some("10.0.0.1"));
    click_text(&mut b, &mock, "to the end");
    assert!(b.model.borrow().scroll > 500.0);
    assert_eq!(b.fetcher.transport.log.len(), 1, "no new fetch for #end");
    assert_eq!(b.model.borrow().history.len(), 2);
}

#[test]
fn suspend_and_restore() {
    let (mut b, mock) = start(ring(), 640, 400, Some("100.66.0.10"));
    b.model.borrow_mut().scroll = 0.0;
    mock.push(&[Message::WillSuspend(proto::WillSuspend { deadline_ms: 500 })]);
    steps(&mut b);
    let saved = mock.sent_messages().into_iter().find_map(|m| match m {
        Message::SaveState(s) => Some(s.data),
        _ => None,
    });
    let saved = saved.expect("SaveState");
    let text = String::from_utf8(saved.clone()).unwrap();
    assert!(text.contains("http://100.66.0.10/"), "{text}");
    // A cold start with that state opens the same page.
    let mock2 = MockConnection::new();
    mock2.state.borrow_mut().idle_timeouts = true;
    mock2.push(&[samples::welcome(true), Message::Restore(proto::Restore { data: saved })]);
    let mut b2 = BrowserApp::new(Box::new(mock2.clone()), ring(), None).unwrap();
    mock2.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w: 640, h: 400, states: proto::window_states::ACTIVATED })]);
    steps(&mut b2);
    assert_eq!(title(&b2), "Tech Village 2 - AS 65002 - Browser");
    assert_eq!(b2.model.borrow().pending, None::<Nav>);
}

#[test]
fn hotkeys_and_reload() {
    let (mut b, mock) = start(ring(), 240, 400, Some("100.65.0.10"));
    click_text(&mut b, &mock, "Tech Village 3");
    assert_eq!(title(&b), "Tech Village 3 - AS 65003 - Browser");
    mock.push(&[Message::Hotkey(proto::Hotkey { slot: 1, down: true }), Message::Hotkey(proto::Hotkey { slot: 1, down: false })]);
    steps(&mut b);
    assert_eq!(title(&b), "Tech Village 1 - AS 65001 - Browser");
    let n = b.fetcher.transport.log.len();
    click(&mut b, &mock, "reload");
    assert_eq!(b.fetcher.transport.log.len(), n + 1);
}
