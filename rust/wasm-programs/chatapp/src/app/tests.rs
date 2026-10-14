//! The Chat app on a scripted UI connection and a scripted chat server.

use super::*;
use super::fake::FakeNet;
use crate::model::{Conn, Pane};
use ecm_ui::{samples as ui_samples, MockConnection};


pub fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("ecm-chatapp-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().replace('\\', "/")
}

fn configure(mock: &MockConnection, w: i32, h: i32, activated: bool, serial: u32) {
    mock.push(&[Message::Configure(proto::Configure {
        surface: 1,
        serial,
        w,
        h,
        states: if activated { proto::window_states::ACTIVATED } else { 0 },
    })]);
}

/// A Chat app at `w x h` (activated), with its data in a fresh folder.
pub fn start(name: &str, w: i32, h: i32) -> (ChatApp<FakeNet>, MockConnection) {
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    let mut wl = ui_samples::welcome(false);
    if let Message::Welcome(ref mut x) = wl {
        x.display.w = w;
        x.display.h = h;
        x.display.size_class = proto::SizeClass::for_width(w);
    }
    mock.push(&[wl]);
    let mut ca = ChatApp::new(Box::new(mock.clone()), FakeNet::default(), Some(tmp(name)), Model::default()).unwrap();
    configure(&mock, w, h, true, 1);
    ca.step(0);
    ca.step(0);
    (ca, mock)
}

fn hit(ca: &ChatApp<FakeNet>, id: &str) -> u32 {
    ca.app.core.window(1).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no widget {id}"))
}

pub fn click(ca: &mut ChatApp<FakeNet>, mock: &MockConnection, id: &str) {
    ca.step(0);
    let h = hit(ca, id);
    mock.push(&ui_samples::click(h, 10, 10));
    ca.step(0);
    ca.step(0);
}

pub fn type_into(ca: &mut ChatApp<FakeNet>, mock: &MockConnection, id: &str, text: &str) {
    click(ca, mock, id);
    mock.push(&[Message::Text(proto::Text { surface: 1, text: text.into() })]);
    ca.step(0);
    ca.step(0);
}

/// Connect from the form as `nick` (empty: the default) and join.
pub fn join(ca: &mut ChatApp<FakeNet>, mock: &MockConnection, nick: &str) {
    type_into(ca, mock, "server", "10.0.30.1");
    if !nick.is_empty() {
        type_into(ca, mock, "nick", nick);
    }
    click(ca, mock, "connect");
    let me = ca.model.borrow().nick.clone();
    ca.net.server_says(&[&format!("WELCOME {me} Tech Village chat"), "HIST 09:59:00 bo earlier words"]);
    ca.step(0);
    ca.net.server_says(&[&format!("WHO bo {me}")]);
    ca.step(0);
    ca.step(0);
}

fn texts(mock: &MockConnection) -> String {
    format!("{:?}", mock.sent_messages())
}

#[test]
fn connect_join_send_and_receive() {
    let (mut ca, mock) = start("flow", 640, 400);
    // The connect form first.
    assert!(ca.app.core.window(1).unwrap().hit_id_of("connect").is_some());
    // Control: Connect with no address says what to type and connects nowhere.
    click(&mut ca, &mock, "connect");
    assert!(ca.net.connected.is_none());
    assert!(matches!(&ca.model.borrow().conn, Conn::Failed(w) if w.contains("address")));
    join(&mut ca, &mock, "alex");
    assert_eq!(ca.net.connected, Some(("10.0.30.1".into(), 7777)));
    assert_eq!(ca.net.sent, vec!["NICK alex", "WHO"]);
    {
        let m = ca.model.borrow();
        assert_eq!(m.conn, Conn::Connected { joined: true });
        assert_eq!(m.people, vec!["alex", "bo"]);
        assert!(m.lines.iter().any(|l| l.text == "earlier words" && l.earlier));
    }
    // The settings are saved for next time.
    let saved = std::fs::read_to_string(format!("{}/settings", ca.data_dir.clone().unwrap())).unwrap();
    assert!(saved.contains("server 10.0.30.1") && saved.contains("nick alex"), "{saved}");

    // Type and press Send: one MSG line; the box empties.
    type_into(&mut ca, &mock, "compose", "hello bo");
    click(&mut ca, &mock, "send");
    assert_eq!(ca.net.sent.last().unwrap(), "MSG hello bo");
    assert_eq!(ca.app.core.window(1).unwrap().state("compose").map(|s| s.text.clone()).unwrap_or_default(), "");
    // Enter sends too.
    type_into(&mut ca, &mock, "compose", "and again");
    mock.push(&[Message::Key(proto::Key { surface: 1, key: 0x28, mods: 0, down: true, repeat: false })]);
    ca.step(0);
    ca.step(0);
    assert_eq!(ca.net.sent.last().unwrap(), "MSG and again");

    // The server echoes ours and bo answers: both show, on screen no notification.
    mock.clear_sent();
    ca.net.server_says(&["MSG 10:00:00 alex hello bo", "MSG 10:00:05 bo hi alex!"]);
    ca.step(0);
    ca.step(0);
    let m = ca.model.borrow();
    let last = m.lines.last().unwrap();
    assert_eq!((last.nick.as_str(), last.text.as_str(), last.mine), ("bo", "hi alex!", false));
    drop(m);
    assert!(texts(&mock).contains("hi alex!"), "the reply is drawn");
    assert!(ca.posted.is_empty());
    // A long conversation stays scrolled to its newest line.
    for i in 0..30 {
        ca.net.server_says(&[&format!("MSG 10:02:{:02} bo line {i}", i)]);
    }
    for _ in 0..32 {
        ca.step(0);
    }
    let scroll = ca.app.core.window(1).unwrap().state("history").unwrap().scroll;
    assert!(scroll > 300, "scrolled to the end: {scroll}");
}

#[test]
fn messages_while_not_on_screen_post_a_notification() {
    let (mut ca, mock) = start("notify", 640, 400);
    join(&mut ca, &mock, "alex");
    // Another window gets focus (a Terminal), or the phone shows another app.
    configure(&mock, 640, 400, false, 2);
    ca.step(0);
    ca.step(0);
    assert!(!ca.model.borrow().on_screen);
    mock.clear_sent();
    ca.net.server_says(&["MSG 10:01:00 bo are you there?"]);
    ca.step(0);
    ca.step(0);
    assert_eq!(ca.posted, vec![("bo in Tech Village chat".to_string(), "are you there?".to_string())]);
    let sent = mock.sent_messages();
    assert!(sent.iter().any(|m| matches!(m, Message::Notify(n) if n.tag == "chat" && n.body == "are you there?")), "{sent:?}");
    assert!(sent.iter().any(|m| matches!(m, Message::BackgroundDone(_))), "a background wake can end");
    // Our own echo never notifies; a second message counts.
    ca.net.server_says(&["MSG 10:01:01 alex yes", "MSG 10:01:02 bo great"]);
    ca.step(0);
    ca.step(0);
    assert_eq!(ca.posted.len(), 2);
    assert_eq!(ca.posted[1].1, "great (+1 more)");
    // Back on screen: unread cleared, no more notifications.
    configure(&mock, 640, 400, true, 3);
    ca.step(0);
    ca.step(0);
    assert_eq!(ca.model.borrow().unread, 0);
    ca.net.server_says(&["MSG 10:01:03 bo :)"]);
    ca.step(0);
    ca.step(0);
    assert_eq!(ca.posted.len(), 2);
}

#[test]
fn every_size_class_has_its_layout() {
    for (w, h) in [(240, 364), (640, 400), (1024, 640)] {
        let (mut ca, mock) = start(&format!("size{w}"), w, h);
        join(&mut ca, &mock, "");
        assert_eq!(ca.model.borrow().me, "pc-0a1b", "{w}: the default nickname");
        let win = ca.app.core.window(1).unwrap();
        for id in ["history", "compose", "send"] {
            assert!(win.hit_id_of(id).is_some() || win.state(id).is_some(), "{w}: {id}");
        }
        if w < 400 {
            // Compact: People is one tap away and replaces the conversation.
            assert!(win.hit_id_of("people").is_none());
            click(&mut ca, &mock, "people_toggle");
            assert_eq!(ca.model.borrow().pane, Pane::People);
            assert!(ca.app.core.window(1).unwrap().hit_id_of("people").is_some());
            click(&mut ca, &mock, "people_toggle");
            assert_eq!(ca.model.borrow().pane, Pane::Conversation);
        } else {
            assert!(win.hit_id_of("people").is_some(), "{w}: people beside the conversation");
            assert!(win.hit_id_of("rooms").is_some(), "{w}: the room list");
        }
    }
}

#[test]
fn lost_connection_and_reconnect_with_saved_settings() {
    let (mut ca, mock) = start("lost", 640, 400);
    join(&mut ca, &mock, "alex");
    ca.net.closed = true;
    ca.step(0);
    ca.step(0);
    assert!(matches!(&ca.model.borrow().conn, Conn::Failed(w) if w.contains("closed")));
    assert!(ca.app.core.window(1).unwrap().hit_id_of("connect").is_some(), "Reconnect");
    click(&mut ca, &mock, "connect");
    assert!(ca.model.borrow().connected());
    assert_eq!(ca.net.sent.last().unwrap(), "NICK alex");
    // A refused connection says why.
    let dir = ca.data_dir.clone().unwrap();
    let mock2 = MockConnection::new();
    mock2.state.borrow_mut().idle_timeouts = true;
    mock2.push(&[ui_samples::welcome(false)]);
    let mut net = FakeNet::default();
    net.refuse = Some("connection to 10.0.30.1:7777 refused: the computer answered but no chat server listens there.".into());
    let mut again = ChatApp::new(Box::new(mock2.clone()), net, Some(dir), Model::default()).unwrap();
    // The next start connects by itself with the saved server.
    again.auto_connect();
    assert_eq!(again.model.borrow().server, "10.0.30.1");
    assert!(matches!(&again.model.borrow().conn, Conn::Failed(w) if w.contains("refused")));
}

#[test]
fn suspend_saves_the_conversation_and_restore_brings_it_back() {
    let (mut ca, mock) = start("save", 640, 400);
    join(&mut ca, &mock, "alex");
    ca.net.server_says(&["MSG 10:00:05 bo remember me"]);
    ca.step(0);
    let j = save_state(&ca.model.borrow());
    let mut m = Model::default();
    restore(&mut m, &j);
    assert_eq!(m.server, "10.0.30.1");
    assert!(m.lines.iter().any(|l| l.text == "remember me" && l.nick == "bo"));
}

#[test]
fn chat_conf_prefills_the_connect_form_without_connecting() {
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    mock.push(&[ui_samples::welcome(false)]);
    let conf = "# Tech Village chat: run 'chat' (see 'chat --help')\nserver 100.68.0.20\nport 7777\nnick v3-house1\n";
    let dir = tmp("prefill");
    let mut ca = ChatApp::with_chat_conf(Box::new(mock.clone()), FakeNet::default(), Some(dir.clone()), Model::default(), Some(conf)).unwrap();
    configure(&mock, 640, 400, true, 1);
    ca.auto_connect();
    ca.step(0);
    ca.step(0);
    // The form shows the village's server and says where it came from; nothing connects by itself.
    assert!(ca.net.connected.is_none());
    assert_eq!(ca.app.core.window(1).unwrap().state("server").map(|s| s.text.clone()).unwrap_or_default(), "100.68.0.20");
    assert!(texts(&mock).contains("Filled in from /etc/chat.conf"), "{}", texts(&mock));
    // Connect uses it (the nickname too) and saves it: the next start connects at once.
    click(&mut ca, &mock, "connect");
    assert_eq!(ca.net.connected, Some(("100.68.0.20".into(), 7777)));
    assert_eq!(ca.net.sent.first().map(String::as_str), Some("NICK v3-house1"));
    let mock2 = MockConnection::new();
    mock2.state.borrow_mut().idle_timeouts = true;
    mock2.push(&[ui_samples::welcome(false)]);
    let mut again = ChatApp::with_chat_conf(Box::new(mock2.clone()), FakeNet::default(), Some(dir), Model::default(), Some(conf)).unwrap();
    again.auto_connect();
    assert_eq!(again.net.connected, Some(("100.68.0.20".into(), 7777)));
    // Control: no /etc/chat.conf (a phone): an empty form with the hint where to find the address.
    let (ca3, mock3) = start("noconf", 360, 640);
    assert!(ca3.net.connected.is_none() && ca3.model.borrow().server.is_empty());
    assert!(texts(&mock3).contains("web page"), "{}", texts(&mock3));
}
