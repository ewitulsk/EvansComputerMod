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
    b.app.borrow().core.window(b.window).unwrap().title.clone()
}

fn hit(b: &BrowserApp<Net>, id: &str) -> u32 {
    b.app.borrow().core.window(b.window).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no widget {id}"))
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
    let r = b.app.borrow().core.window(b.window).unwrap().rect_of("page").unwrap();
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
    assert_eq!(b.fetcher.borrow().transport.log.last().unwrap(), "100.66.0.10:80 /");
    // The address bar follows.
    assert_eq!(b.app.borrow().core.window(1).unwrap().state("url").unwrap().text, "http://100.66.0.10/");
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
    let app = b.app.borrow();
    let w = app.core.window(1).unwrap();
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
    assert!(b.fetcher.borrow().transport.log.contains(&"file /home/site/style.css".to_string()));
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
    assert_eq!(b.fetcher.borrow().transport.log.len(), 1, "no new fetch for #end");
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
    let n = b.fetcher.borrow().transport.log.len();
    click(&mut b, &mock, "reload");
    assert_eq!(b.fetcher.borrow().transport.log.len(), n + 1);
}

// ------------------------------------------------------------------ phase 2: forms, pictures, menu, downloads

/// Click the middle of element `#id` on the page.
fn click_id(b: &mut BrowserApp<Net>, mock: &MockConnection, id: &str) {
    let (x, y) = {
        let m = b.model.borrow();
        let p = m.page.as_ref().unwrap();
        let n = p.dom.by_id(id).unwrap_or_else(|| panic!("no #{id}"));
        let r = m.cache.as_ref().unwrap().rendered.list.node_rect(n).unwrap_or_else(|| panic!("#{id} not drawn"));
        ((r.x + r.w / 2.0) as i32, (r.y + r.h / 2.0 - m.scroll) as i32)
    };
    let r = b.app.borrow().core.window(b.window).unwrap().rect_of("page").unwrap();
    let h = hit(b, "page");
    mock.push(&samples::click(h, r.x + x, r.y + y));
    steps(b);
}

fn type_text(b: &mut BrowserApp<Net>, mock: &MockConnection, t: &str) {
    mock.push(&[Message::Text(proto::Text { surface: 1, text: t.into() })]);
    steps(b);
}

fn key(b: &mut BrowserApp<Net>, mock: &MockConnection, k: u16) {
    mock.push(&[Message::Key(proto::Key { surface: 1, key: k, mods: 0, down: true, repeat: false })]);
    steps(b);
}

const GUESTBOOK: &str = "<html><head><title>Guestbook</title></head><body><h1>Guestbook</h1>\
    <form method=post action=/sign><input id=name name=name placeholder=Name><textarea id=msg name=msg></textarea>\
    <label><input id=pub type=checkbox name=public value=yes> public</label>\
    <select id=mood name=mood><option>happy<option>busy</select><button id=send>Sign</button></form>\
    <form action=/search><input id=q name=q></form></body></html>";

fn guest_net() -> Net {
    Net::default()
        .page("10.0.0.1:80 /", "200 OK", "text/html", GUESTBOOK)
        .raw("10.0.0.1:80 /sign", b"HTTP/1.0 303 See Other\r\nLocation: /thanks\r\nSet-Cookie: who=Pat\r\n\r\n")
        .page("10.0.0.1:80 /thanks", "200 OK", "text/html", "<title>Thanks</title><p>Signed</p>")
        .page("10.0.0.1:80 /search?q=fiber+tap", "200 OK", "text/html", "<title>Results</title><p>1 result</p>")
}

#[test]
fn typing_into_a_form_and_posting_it() {
    let (mut b, mock) = start(guest_net(), 640, 400, Some("10.0.0.1"));
    click_id(&mut b, &mock, "name");
    assert!(b.model.borrow().focus.is_some(), "the field has the focus");
    type_text(&mut b, &mock, "Pax");
    key(&mut b, &mock, 0x2A); // Backspace edits the field (it doesn't go back)
    type_text(&mut b, &mock, "t");
    assert_eq!(b.model.borrow().loads, 1);
    click_id(&mut b, &mock, "msg");
    type_text(&mut b, &mock, "hi");
    key(&mut b, &mock, 0x28); // Enter in a textarea: a new line
    type_text(&mut b, &mock, "there");
    click_id(&mut b, &mock, "pub");
    click_id(&mut b, &mock, "mood");
    // The select's popup menu opens (a popup surface beside the window).
    assert_eq!(b.app.borrow().core.windows.len(), 2, "a popup for the select");
    {
        let mut m = b.model.borrow_mut();
        let sel = m.page.as_ref().unwrap().dom.by_id("mood").unwrap();
        m.choose(sel, 1);
    }
    b.app.borrow_mut().core.mark_all_dirty();
    steps(&mut b);
    click_id(&mut b, &mock, "send");
    assert_eq!(title(&b), "Thanks - Browser");
    let fetcher = b.fetcher.borrow();
    let post = fetcher.transport.requests.iter().find(|r| r.starts_with("POST /sign")).expect("a POST");
    assert!(post.ends_with("name=Pat&msg=hi%0D%0Athere&public=yes&mood=busy"), "{post}");
    // The cookie the server set goes back with the next request to it.
    let thanks = fetcher.transport.requests.iter().find(|r| r.starts_with("GET /thanks")).unwrap();
    assert!(thanks.contains("Cookie: who=Pat"), "{thanks}");
}

#[test]
fn enter_in_a_field_sends_a_get_form() {
    let (mut b, mock) = start(guest_net(), 640, 400, Some("10.0.0.1"));
    click_id(&mut b, &mock, "q");
    type_text(&mut b, &mock, "fiber tap");
    key(&mut b, &mock, 0x28);
    assert_eq!(title(&b), "Results - Browser");
    assert_eq!(b.app.borrow().core.window(1).unwrap().state("url").unwrap().text, "http://10.0.0.1/search?q=fiber+tap");
}

#[test]
fn pictures_are_fetched_and_shown() {
    let mut px = Vec::new();
    for i in 0..(8 * 4) {
        px.extend_from_slice(&[i as u8 * 7, 100, 200, 255]);
    }
    let png = ecm_web::png::encode(8, 4, &px);
    let mut answer = format!("HTTP/1.0 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n", png.len()).into_bytes();
    answer.extend_from_slice(&png);
    let net = Net::default()
        .page("10.0.0.1:80 /", "200 OK", "text/html", "<title>Pic</title><img src=a.png width=32 alt=pic><img src=gone.png alt=gone>")
        .raw("10.0.0.1:80 /a.png", &answer);
    let (mut b, mock) = start(net, 640, 400, Some("10.0.0.1"));
    steps(&mut b);
    let m = b.model.borrow();
    let img = m.images.get("http://10.0.0.1/a.png").copied().flatten().expect("a.png loaded");
    assert_eq!((img.w, img.h), (8, 4));
    assert_eq!(m.images.get("http://10.0.0.1/gone.png"), Some(&None), "a missing picture is remembered as missing");
    let sent = sent_texts(&mock);
    assert!(sent.contains("ResourceCreate"), "uploaded as an image resource");
    assert!(sent.contains(&format!("Resource({})", img.resource)), "an image node shows it");
    // Shown at the size the page asks (width 32: height 16).
    let list = &m.cache.as_ref().unwrap().rendered.list;
    assert!(list.items.iter().any(|i| matches!(i, ecm_web::paint::Item::Image { w: 32, h: 16, .. })));
    assert!(list.items.iter().any(|i| matches!(i, ecm_web::paint::Item::Text { text, .. } if text == "gone")));
}

#[test]
fn menu_bookmarks_history_and_home_are_saved() {
    let dir = std::env::temp_dir().join(format!("ecm-browser-settings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dir = dir.to_string_lossy().replace('\\', "/");
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    mock.push(&[samples::welcome(false)]);
    let mut b = BrowserApp::with_settings(Box::new(mock.clone()), ring(), Some("100.65.0.10"), Some(dir.clone()), "about:home").unwrap();
    mock.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w: 640, h: 400, states: proto::window_states::ACTIVATED })]);
    steps(&mut b);
    crate::view::menu_action(&b.model, 0);
    crate::view::menu_action(&b.model, 3);
    steps(&mut b);
    assert!(b.model.borrow().status_text().starts_with("Home page:"));
    crate::view::menu_action(&b.model, 1);
    steps(&mut b);
    assert_eq!(title(&b), "Bookmarks - Browser");
    assert!(page_text(&b).contains("Tech Village 1 - AS 65001"));
    crate::view::menu_action(&b.model, 2);
    steps(&mut b);
    assert!(page_text(&b).contains("http://100.65.0.10/"));
    let saved = std::fs::read_to_string(format!("{dir}/browser.json")).unwrap();
    assert!(saved.contains("\"home\":\"http://100.65.0.10/\""), "{saved}");
    // A new Browser starts at the saved home page, its bookmark kept.
    let mock2 = MockConnection::new();
    mock2.state.borrow_mut().idle_timeouts = true;
    mock2.push(&[samples::welcome(false)]);
    let mut b2 = BrowserApp::with_settings(Box::new(mock2.clone()), ring(), None, Some(dir), "about:home").unwrap();
    mock2.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w: 640, h: 400, states: proto::window_states::ACTIVATED })]);
    steps(&mut b2);
    assert_eq!(title(&b2), "Tech Village 1 - AS 65001 - Browser");
    assert_eq!(b2.model.borrow().settings.bookmarks.len(), 1);
}

#[test]
fn other_files_are_downloads() {
    let net = Net::default().raw(
        "10.0.0.1:80 /apps/dev.ecm.x-1.0.0.ecmpkg",
        b"HTTP/1.0 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 4\r\n\r\nPK\x03\x04",
    );
    let (b, _mock) = start(net, 640, 400, Some("10.0.0.1/apps/dev.ecm.x-1.0.0.ecmpkg"));
    assert_eq!(b.fetcher.borrow().transport.saved, vec![("/home/Downloads/dev.ecm.x-1.0.0.ecmpkg".to_string(), b"PK\x03\x04".to_vec())]);
    assert_eq!(title(&b), "Downloaded - Browser");
    assert!(page_text(&b).contains("pkg install /home/Downloads/dev.ecm.x-1.0.0.ecmpkg"));
}

#[test]
fn two_inline_fields_one_after_the_other() {
    let html = "<html><head><title>Python guestbook</title></head><body><h1>Python guestbook</h1><p style='color: #c03030'>Name and message are required.</p><table><tr><th>Name</th><th>Message</th></tr></table><form method=post><input id=n name=name value='' placeholder='Your name'> <input id=m name=msg placeholder='Your message'> <button>Sign</button></form></body></html>";
    let net = Net::default().page("10.0.0.1:80 /", "200 OK", "text/html", html);
    let (mut b, mock) = start(net, 484, 270, Some("10.0.0.1"));
    click_text(&mut b, &mock, "Your name");
    type_text(&mut b, &mock, "Pat");
    click_text(&mut b, &mock, "Your message");
    let f = b.model.borrow().focus;
    let m = b.model.borrow().page.as_ref().unwrap().dom.by_id("m");
    assert_eq!(f.map(|x| x.0), m, "focus after clicking the second field");
    type_text(&mut b, &mock, "hello");
    let v = ecm_web::forms::value(&b.model.borrow().page.as_ref().unwrap().dom, m.unwrap());
    assert_eq!(v, "hello");
}

#[test]
fn a_click_right_after_typing_in_the_same_batch() {
    // Events a player sends quickly arrive together: the click must hit-test the page as typing left it.
    let net = guest_net();
    let (mut b, mock) = start(net, 640, 400, Some("10.0.0.1"));
    let (at_name, at_q) = {
        let m = b.model.borrow();
        let p = m.page.as_ref().unwrap();
        let list = &m.cache.as_ref().unwrap().rendered.list;
        let c = |id: &str| {
            let r = list.node_rect(p.dom.by_id(id).unwrap()).unwrap();
            ((r.x + 3.0) as i32, (r.y + r.h / 2.0) as i32)
        };
        (c("name"), c("q"))
    };
    let r = b.app.borrow().core.window(b.window).unwrap().rect_of("page").unwrap();
    let h = hit(&b, "page");
    let mut batch = samples::click(h, r.x + at_name.0, r.y + at_name.1);
    batch.push(Message::Text(proto::Text { surface: 1, text: "Pat".into() }));
    batch.extend(samples::click(h, r.x + at_q.0, r.y + at_q.1));
    batch.push(Message::Text(proto::Text { surface: 1, text: "tap".into() }));
    mock.push(&batch);
    steps(&mut b);
    let m = b.model.borrow();
    let dom = &m.page.as_ref().unwrap().dom;
    assert_eq!(ecm_web::forms::value(dom, dom.by_id("name").unwrap()), "Pat");
    assert_eq!(ecm_web::forms::value(dom, dom.by_id("q").unwrap()), "tap");
}

// ------------------------------------------------------------------ phase 3: page scripts

const COUNTER: &str = r#"<html><head><title>Counter</title></head><body>
<p id=count>0</p><button id=plus>+1</button><button id=load>Load</button><button id=hang>Hang</button>
<p id=news>-</p><a id=away href=/away>away</a>
<form id=f action=/search><input id=q name=q><button id=go>Go</button></form>
<script>
var n = 0;
document.getElementById('plus').addEventListener('click', function () { n++; document.getElementById('count').textContent = n; });
document.getElementById('load').onclick = function () {
  fetch('data.json').then(function (r) { return r.json(); }).then(function (j) { document.getElementById('news').textContent = j.news; });
};
document.getElementById('hang').addEventListener('click', function () { while (true) {} });
document.getElementById('away').addEventListener('click', function (e) { if (n < 2) { e.preventDefault(); console.log('not yet'); } });
document.getElementById('f').addEventListener('submit', function (e) { if (!document.getElementById('q').value) { e.preventDefault(); document.getElementById('news').textContent = 'type something'; } });
document.addEventListener('keydown', function (e) { if (e.key === 'k') document.getElementById('news').textContent = 'key k'; });
</script></body></html>"#;

fn counter_net() -> Net {
    Net::default()
        .page("10.0.0.1:80 /", "200 OK", "text/html", COUNTER)
        .page("10.0.0.1:80 /data.json", "200 OK", "application/json", r#"{"news": "the mast is up"}"#)
        .page("10.0.0.1:80 /away", "200 OK", "text/html", "<title>Away</title>")
        .page("10.0.0.1:80 /search?q=tap", "200 OK", "text/html", "<title>Found</title>")
}

fn text_by_id(b: &BrowserApp<Net>, id: &str) -> String {
    let m = b.model.borrow();
    let d = &m.page.as_ref().unwrap().dom;
    d.text_content(d.by_id(id).unwrap())
}

#[test]
fn a_counter_a_fetch_and_a_runaway_script() {
    let (mut b, mock) = start(counter_net(), 640, 400, Some("10.0.0.1"));
    assert!(b.script.is_some(), "the page has scripts");
    click_id(&mut b, &mock, "plus");
    click_id(&mut b, &mock, "plus");
    assert_eq!(text_by_id(&b, "count"), "2");
    // The page shown follows the document the script changed.
    assert!(b.model.borrow().cache.as_ref().unwrap().rendered.list.items.iter().any(|i| matches!(i, ecm_web::paint::Item::Text { text, .. } if text == "2")));
    click_id(&mut b, &mock, "load");
    assert_eq!(text_by_id(&b, "news"), "the mast is up");
    assert!(b.fetcher.borrow().transport.log.contains(&"10.0.0.1:80 /data.json".to_string()));
    // while (true) {}: stopped by the budget; the page keeps working.
    let t = std::time::Instant::now();
    click_id(&mut b, &mock, "hang");
    assert!(t.elapsed() < std::time::Duration::from_secs(3));
    assert!(b.model.borrow().console.iter().any(|l| l.contains("ran too long")), "{:?}", b.model.borrow().console);
    click_id(&mut b, &mock, "plus");
    assert_eq!(text_by_id(&b, "count"), "3");
    // Keys reach the page's listeners.
    key(&mut b, &mock, 0x0E);
    assert_eq!(text_by_id(&b, "news"), "key k");
}

#[test]
fn scripts_can_stop_links_and_forms() {
    let (mut b, mock) = start(counter_net(), 640, 400, Some("10.0.0.1"));
    click_id(&mut b, &mock, "away");
    assert_eq!(title(&b), "Counter - Browser", "the click listener prevented the link");
    assert!(b.model.borrow().console.contains(&"not yet".to_string()));
    click_id(&mut b, &mock, "go");
    assert_eq!(text_by_id(&b, "news"), "type something", "the submit listener refused the empty form");
    assert_eq!(title(&b), "Counter - Browser");
    click_id(&mut b, &mock, "q");
    type_text(&mut b, &mock, "tap");
    key(&mut b, &mock, 0x28);
    assert_eq!(title(&b), "Found - Browser", "Enter: keydown, submit, then the form is sent");
    b.model.borrow_mut().pending = Some(Nav::Back);
    steps(&mut b);
    click_id(&mut b, &mock, "plus");
    click_id(&mut b, &mock, "plus");
    click_id(&mut b, &mock, "away");
    assert_eq!(title(&b), "Away - Browser", "allowed now: the default happens");
}

#[test]
fn timers_storage_and_confirm() {
    let dir = std::env::temp_dir().join(format!("ecm-browser-js-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dir = dir.to_string_lossy().replace('\\', "/");
    let html = r#"<title>Timers</title><p id=t>waiting</p><p id=v></p><p id=c></p><script>
setTimeout(function () { document.getElementById('t').textContent = 'fired'; }, 30);
var visits = +(localStorage.getItem('visits') || 0) + 1; localStorage.setItem('visits', visits);
document.getElementById('v').textContent = 'visit ' + visits;
document.getElementById('c').textContent = confirm('Really?') ? 'yes' : 'no';
</script>"#;
    let net = Net::default().page("10.0.0.1:80 /", "200 OK", "text/html", html);
    let open = |dir: &str| {
        let mock = MockConnection::new();
        mock.state.borrow_mut().idle_timeouts = true;
        mock.push(&[samples::welcome(false)]);
        let b = BrowserApp::with_settings(Box::new(mock.clone()), net.clone(), None, Some(dir.to_string()), "about:home").unwrap();
        b.model.borrow_mut().dialog_auto = Some(false);
        b.model.borrow_mut().go_input("10.0.0.1");
        mock.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w: 640, h: 400, states: proto::window_states::ACTIVATED })]);
        (b, mock)
    };
    let (mut b, _mock) = open(&dir);
    for _ in 0..4 {
        b.step(0);
    }
    assert_eq!(text_by_id(&b, "c"), "no", "confirm answered Cancel");
    assert_eq!(text_by_id(&b, "v"), "visit 1");
    assert_eq!(text_by_id(&b, "t"), "waiting");
    let t = std::time::Instant::now();
    while text_by_id(&b, "t") == "waiting" && t.elapsed() < std::time::Duration::from_secs(2) {
        b.step(5);
    }
    assert_eq!(text_by_id(&b, "t"), "fired");
    // localStorage is kept per host in the app's data folder.
    let (mut b2, _m2) = open(&dir);
    for _ in 0..4 {
        b2.step(0);
    }
    assert_eq!(text_by_id(&b2, "v"), "visit 2");
}

#[test]
fn the_dialog_waits_for_the_player() {
    let (mut b, mock) = start(guest_net(), 640, 400, Some("10.0.0.1"));
    b.model.borrow_mut().dialog = Some(crate::model::Dialog { text: "Delete all entries?".into(), ask: true });
    b.app.borrow_mut().core.mark_all_dirty();
    steps(&mut b);
    assert!(sent_texts(&mock).contains("Delete all entries?"));
    click(&mut b, &mock, "dialog_cancel");
    assert_eq!(b.model.borrow().dialog_answer, Some(false));
}

// ------------------------------------------------------------------ phase 4: the registrar's pages

#[test]
fn the_registrar_forms_after_a_search() {
    let home = ecm_dns::registrar_web::home("ecm", None, &[]);
    let found = ecm_dns::registrar_web::result("check", "OK site1.ecm available", None);
    let net = Net::default()
        .page("nic.ring:80 /", "200 OK", "text/html", &home)
        .page("nic.ring:80 /search?domain=site1.ecm", "200 OK", "text/html", &found);
    let (mut b, mock) = start(net, 640, 400, Some("http://nic.ring/"));
    assert_eq!(title(&b), "Register a .ecm name - nic.ring - Browser");
    click_text(&mut b, &mock, "a name to look up");
    type_text(&mut b, &mock, "site1.ecm");
    click_text(&mut b, &mock, "Search");
    assert!(page_text(&b).contains("site1.ecm is free."), "{}", page_text(&b));
    click_text(&mut b, &mock, "Back to nic.ring");
    assert!(page_text(&b).contains("Create an account"));
    click_text(&mut b, &mock, "pick a handle");
    type_text(&mut b, &mock, "dana");
    click_text(&mut b, &mock, "pick a password");
    type_text(&mut b, &mock, "pw1234");
    key(&mut b, &mock, 0x2A);
    let m = b.model.borrow();
    let dom = &m.page.as_ref().unwrap().dom;
    let handle: Vec<_> = dom.find_all("input").into_iter().filter(|&n| dom.attr(n, "placeholder") == Some("pick a handle")).collect();
    assert_eq!(ecm_web::forms::value(dom, handle[0]), "dana", "focus {:?}", m.focus);
}

#[test]
fn the_registrar_forms_in_a_small_window() {
    let home = ecm_dns::registrar_web::home("ecm", None, &[]);
    let net = Net::default().page("nic.ring:80 /", "200 OK", "text/html", &home);
    let (mut b, mock) = start(net, 484, 270, Some("http://nic.ring/"));
    let h = hit(&b, "page");
    let wheel = |steps: i32| Message::Pointer(proto::Pointer { surface: 1, kind: proto::PointerKind::Scroll, x: 100, y: 100, buttons: 0, button: proto::Button::None, scroll_x: 0, scroll_y: steps as i16, hit_id: h });
    mock.push(&[wheel(1)]);
    steps(&mut b);
    let s0 = b.model.borrow().scroll;
    click_text(&mut b, &mock, "pick a handle");
    type_text(&mut b, &mock, "dana");
    click_text(&mut b, &mock, "pick a password");
    type_text(&mut b, &mock, "pw1234");
    mock.push(&[wheel(3)]);
    steps(&mut b);
    let s1 = b.model.borrow().scroll;
    assert!(s1 > s0, "the wheel scrolls with a field focused: {s0} -> {s1}");
    let m = b.model.borrow();
    let dom = &m.page.as_ref().unwrap().dom;
    let v: Vec<String> = dom.find_all("input").into_iter().map(|n| ecm_web::forms::value(dom, n)).collect();
    assert!(v.contains(&"dana".to_string()) && v.contains(&"pw1234".to_string()), "{v:?}");
}

#[test]
fn the_registrar_account_form_sends_what_was_typed() {
    let home = ecm_dns::registrar_web::home("ecm", None, &[]);
    let found = ecm_dns::registrar_web::result("check", "OK site1.ecm available", None);
    let net = Net::default()
        .page("nic.ring:80 /", "200 OK", "text/html", &home)
        .page("nic.ring:80 /search?domain=site1.ecm", "200 OK", "text/html", &found)
        .page("nic.ring:80 /web", "200 OK", "text/html", "<title>Posted</title>");
    let (mut b, mock) = start(net, 484, 270, Some("http://nic.ring/"));
    click_text(&mut b, &mock, "a name to look up");
    type_text(&mut b, &mock, "site1.ecm");
    click_text(&mut b, &mock, "Search");
    click_text(&mut b, &mock, "Back to nic.ring");
    let h = hit(&b, "page");
    mock.push(&[Message::Pointer(proto::Pointer { surface: 1, kind: proto::PointerKind::Scroll, x: 100, y: 100, buttons: 0, button: proto::Button::None, scroll_x: 0, scroll_y: 1, hit_id: h })]);
    steps(&mut b);
    click_text(&mut b, &mock, "pick a handle");
    type_text(&mut b, &mock, "site1");
    click_text(&mut b, &mock, "pick a password");
    type_text(&mut b, &mock, "pw1234");
    click_text(&mut b, &mock, "Create account");
    let reqs = b.fetcher.borrow().transport.requests.clone();
    let last = reqs.last().unwrap();
    assert!(last.contains("op=account&handle=site1&password=pw1234"), "{reqs:#?}");
}
